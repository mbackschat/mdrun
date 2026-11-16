use super::broker::{CustodyAccess, CustodyBroker};
use super::config::{Binding, cleanup_grace};
use super::context::{NativeContext, SESSION_ENV};
use super::custody::{Custody, CustodyConfig, CustodyEvent, SpawnSpec};
use super::host::{AuthorStdio, PreparedSession, SessionOutcome, run_prepared};
use super::report::{ReportFile, conflicts};
use super::{Mode, Options as SessionOptions};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::num::NonZeroUsize;
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const TICK: Duration = Duration::from_millis(5);

#[derive(Debug, clap::Args)]
pub struct Options {
    /// Prepared application binding shared by the batch.
    #[arg(long)]
    pub binding: PathBuf,
    /// Maximum number of documents including startup and cleanup.
    #[arg(long, default_value = "1")]
    pub jobs: NonZeroUsize,
    /// Permit independent documents to share the binding's resource exclusions.
    #[arg(long)]
    pub share_resources: bool,
    /// Prefer eligible warm calls, or use only the packaged launcher.
    #[arg(long, value_enum, default_value = "fast")]
    pub mode: Mode,
    /// Write an aggregate report outside recorded document output.
    #[arg(long)]
    pub report: Option<PathBuf>,
    /// Reject calls that cannot execute warm.
    #[arg(long)]
    pub require_warm: bool,
}

struct Workspace {
    path: PathBuf,
    retained: bool,
}

impl Workspace {
    fn new() -> Result<Self, String> {
        let name = format!("mdrun-batch-{}", uuid::Uuid::new_v4().simple());
        let mut path = std::env::temp_dir().join(&name);
        if path.to_str().is_none() {
            path = Path::new("/tmp").join(name);
        }
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .map_err(|_| "cannot create batch workspace")?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
            .map_err(|_| "cannot protect batch workspace")?;
        Ok(Self {
            path,
            retained: false,
        })
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        if !self.retained {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

enum Phase {
    Queued,
    Running,
    Finished(Result<SessionOutcome, String>),
}

struct Document {
    input: PathBuf,
    absolute: PathBuf,
    stdout: PathBuf,
    stderr: PathBuf,
    phase: Phase,
}

fn command(executable: &Path, file: &Path) -> Vec<OsString> {
    vec![
        executable.as_os_str().to_owned(),
        "verify".into(),
        file.as_os_str().to_owned(),
    ]
}

fn session_options(options: &Options) -> SessionOptions {
    SessionOptions {
        binding: options.binding.clone(),
        mode: options.mode,
        report: None,
        require_warm: options.require_warm,
    }
}

fn batch_budget(
    count: usize,
    jobs: usize,
    session: Duration,
    cleanup: Duration,
) -> Result<Duration, String> {
    let waves = count.div_ceil(jobs);
    let wave = session
        .checked_add(cleanup)
        .ok_or("batch duration is not representable")?;
    wave.checked_mul(u32::try_from(waves).map_err(|_| "too many document waves")?)
        .and_then(|value| value.checked_add(cleanup))
        .ok_or_else(|| "batch duration is not representable".to_owned())
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Publication {
    label: String,
    stdout: Option<PathBuf>,
    stderr: Option<PathBuf>,
    diagnostics: Vec<String>,
}

fn publication(document: &Document) -> Result<Option<Publication>, String> {
    let Phase::Finished(result) = &document.phase else {
        return Ok(None);
    };
    let output = |path: &Path| -> Result<bool, String> {
        match fs::metadata(path) {
            Ok(metadata) => Ok(metadata.len() > 0),
            Err(error) if error.kind() == io::ErrorKind::NotFound && result.is_err() => Ok(false),
            Err(_) => Err("cannot inspect document spool".to_owned()),
        }
    };
    let stdout = output(&document.stdout)?.then(|| document.stdout.clone());
    let stderr = output(&document.stderr)?.then(|| document.stderr.clone());
    let diagnostics = match result {
        Ok(outcome) => outcome.diagnostics.clone(),
        Err(error) => vec![format!("mdrun: document failed: {error}")],
    };
    Ok(
        (stdout.is_some() || stderr.is_some() || !diagnostics.is_empty()).then(|| Publication {
            label: format!("{:?}", document.input.as_os_str()),
            stdout,
            stderr,
            diagnostics,
        }),
    )
}

// Spool writers use custody; final diagnostics use a bounded direct child with
// its own expiry. Parent writes or unjoinable threads cannot bound blocked output.
fn publish(publication: Publication) -> io::Result<()> {
    if let Some(source) = publication.stdout {
        let mut out = io::stdout().lock();
        writeln!(out, "{}:", publication.label)?;
        io::copy(&mut File::open(source)?, &mut out)?;
        out.flush()?;
    }
    if publication.stderr.is_some() || !publication.diagnostics.is_empty() {
        let mut err = io::stderr().lock();
        writeln!(err, "{}:", publication.label)?;
        if let Some(source) = publication.stderr {
            io::copy(&mut File::open(source)?, &mut err)?;
        }
        for diagnostic in publication.diagnostics {
            writeln!(err, "{diagnostic}")?;
        }
        err.flush()?;
    }
    Ok(())
}

pub fn run_publisher(config: &Path, diagnostics_only: bool) -> i32 {
    if diagnostics_only {
        // This entry point starts no children. The local timer also bounds a
        // blocked diagnostic after abrupt parent loss, without resource custody.
        unsafe {
            let mut set = std::mem::zeroed();
            libc::sigemptyset(&mut set);
            libc::sigaddset(&mut set, libc::SIGALRM);
            if libc::signal(libc::SIGALRM, libc::SIG_DFL) == libc::SIG_ERR
                || libc::pthread_sigmask(libc::SIG_UNBLOCK, &set, std::ptr::null_mut()) != 0
            {
                return 1;
            }
            libc::alarm(1);
        }
    }
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let publication: Publication = serde_json::from_reader(File::open(config)?)?;
        if diagnostics_only && (publication.stdout.is_some() || publication.stderr.is_some()) {
            return Err("diagnostic publisher cannot copy spools".into());
        }
        publish(publication)?;
        Ok(())
    })();
    if diagnostics_only {
        unsafe {
            libc::alarm(0);
        }
    }
    i32::from(result.is_err())
}

fn publish_diagnostics(
    executable: &Path,
    workspace: &Path,
    stderr: OwnedFd,
    diagnostics: Vec<String>,
) {
    let prepared = (|| -> io::Result<PathBuf> {
        let path = workspace.join("diagnostics.json");
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)?;
        serde_json::to_writer(
            &mut file,
            &Publication {
                label: "mdrun".to_owned(),
                stdout: None,
                stderr: None,
                diagnostics,
            },
        )?;
        file.flush()?;
        Ok(path)
    })();
    let Ok(path) = prepared else {
        return;
    };
    let Ok(mut child) = Command::new(executable)
        .args(["__batch-publish", "--diagnostics-only", "--config"])
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(stderr))
        .spawn()
    else {
        return;
    };
    let deadline = Instant::now() + Duration::from_millis(500);
    while Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) => thread::sleep(TICK),
            Err(_) => break,
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

struct Publishing {
    index: usize,
    id: String,
    exited: bool,
    clean: bool,
}

struct Publisher {
    custody: CustodyAccess,
    stdio: [OwnedFd; 3],
    active: Option<Publishing>,
}

impl Publisher {
    fn start(
        &mut self,
        index: usize,
        document: &Document,
        executable: &Path,
        baseline: &NativeContext,
        workspace: &Path,
    ) -> Result<bool, String> {
        let Some(publication) = publication(document)? else {
            return Ok(false);
        };
        let path = workspace.join(format!("{index}.publication.json"));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .map_err(|_| "cannot prepare batch publication")?;
        serde_json::to_writer(&mut file, &publication)
            .map_err(|_| "cannot prepare batch publication")?;
        file.flush()
            .map_err(|_| "cannot prepare batch publication")?;
        let id = format!("publication-{index}");
        self.active = Some(Publishing {
            index,
            id: id.clone(),
            exited: false,
            clean: false,
        });
        self.custody
            .spawn(
                &id,
                &SpawnSpec {
                    argv: vec![
                        executable.as_os_str().into(),
                        "__batch-publish".into(),
                        "--config".into(),
                        path.into_os_string(),
                    ],
                    cwd: baseline.cwd.clone(),
                    environment: baseline.environment.clone(),
                    umask: baseline.umask,
                    caller_group: baseline.caller_group,
                },
                [
                    self.stdio[0].as_fd(),
                    self.stdio[1].as_fd(),
                    self.stdio[2].as_fd(),
                ],
            )
            .map_err(|_| "cannot start batch publication")?;
        Ok(true)
    }

    fn poll(&mut self) -> Result<Option<usize>, String> {
        let mut failed = false;
        while let Some(event) = self
            .custody
            .poll_event(Duration::ZERO)
            .map_err(|_| "batch publication custody was lost")?
        {
            match event {
                CustodyEvent::Exited { id, status } => {
                    let active = self.active.as_mut().ok_or("unexpected publisher exit")?;
                    if active.id != id || active.exited {
                        return Err("unexpected publisher exit".to_owned());
                    }
                    active.exited = true;
                    failed |= status.code != Some(0) || status.signal.is_some();
                }
                CustodyEvent::Clean { id, forced } => {
                    let active = self.active.as_mut().ok_or("unexpected publisher cleanup")?;
                    if active.id != id || active.clean {
                        return Err("unexpected publisher cleanup".to_owned());
                    }
                    active.clean = true;
                    failed |= forced;
                }
                _ => failed = true,
            }
        }
        let completed = self
            .active
            .as_ref()
            .filter(|active| active.exited && active.clean)
            .map(|active| active.index);
        if completed.is_some() {
            self.active = None;
        }
        if failed {
            Err("batch output publication failed".to_owned())
        } else {
            Ok(completed)
        }
    }
}

fn input_entry(index: usize, input: &Path) -> serde_json::Value {
    let mut entry = json!({"index":index,"file":input.to_string_lossy()});
    if input.to_str().is_none() {
        entry["file_native_bytes"] = json!(input.as_os_str().as_bytes());
    }
    entry
}

pub fn run(options: &Options, files: &[PathBuf], workdir: Option<&Path>) -> Result<i32, String> {
    if files.is_empty() {
        return Err("batch requires at least one document".to_owned());
    }
    if std::env::var_os(SESSION_ENV).is_some() {
        return Err("nested managed sessions are unsupported".to_owned());
    }
    let started = Instant::now();
    let baseline = NativeContext::capture()?;
    let binding = Binding::from_file(&options.binding)?;
    let absolute: Vec<_> = files.iter().map(|file| baseline.cwd.join(file)).collect();
    for file in &absolute {
        let source =
            fs::read(file).map_err(|_| format!("cannot read document: {}", file.display()))?;
        crate::document::Document::parse(&source)
            .map_err(|error| format!("cannot parse document {}: {error}", file.display()))?;
    }
    let absolute: Vec<_> = absolute
        .iter()
        .map(fs::canonicalize)
        .collect::<Result<_, _>>()
        .map_err(|_| "cannot resolve document")?;
    let report_path = options.report.as_ref().map(|path| baseline.cwd.join(path));
    if let Some(report) = &report_path {
        let mut protected = absolute.clone();
        protected.push(baseline.cwd.join(&options.binding));
        if conflicts(report, &protected)? {
            return Err("batch report conflicts with an input document".to_owned());
        }
        if report.is_dir() {
            return Err("batch report is a directory".to_owned());
        }
    }
    let jobs = if !binding.resources.is_empty() && !options.share_resources {
        1
    } else {
        options.jobs.get().min(files.len())
    };
    let session = Duration::from_millis(binding.session_timeout_ms);
    let cleanup = cleanup_grace(binding.cleanup_timeout_ms)
        .ok_or("batch cleanup budget is not representable")?;
    let budget = batch_budget(files.len(), jobs, session, cleanup)?;
    let executable = std::env::current_exe().map_err(|_| "cannot locate native executable")?;
    // Validate the fixed execution context before any custody or coordinator thread starts.
    PreparedSession::from_context(
        session_options(options),
        binding.clone(),
        command(&executable, &absolute[0]),
        baseline.clone(),
        workdir,
        Instant::now()
            .checked_add(session)
            .ok_or("document deadline is not representable")?,
    )?;
    let mut report_file = report_path.map(ReportFile::prepare).transpose()?;
    let deadline = Instant::now()
        .checked_add(budget)
        .ok_or("batch deadline is not representable")?;
    let execution_deadline = deadline
        .checked_sub(cleanup)
        .ok_or("batch deadline is not representable")?;
    let mut workspace = Workspace::new()?;
    let custody_path = workspace.path.join("custody.json");
    let publication_stdio = [
        File::open("/dev/null")
            .map_err(|_| "cannot open batch null input")?
            .into(),
        io::stdout()
            .as_fd()
            .try_clone_to_owned()
            .map_err(|_| "cannot capture batch stdout")?,
        io::stderr()
            .as_fd()
            .try_clone_to_owned()
            .map_err(|_| "cannot capture batch stderr")?,
    ];
    let diagnostic_stderr = publication_stdio[2]
        .try_clone()
        .map_err(|_| "cannot capture batch diagnostics")?;
    let mut documents: Vec<_> = files
        .iter()
        .zip(absolute)
        .enumerate()
        .map(|(index, (input, absolute))| Document {
            input: input.clone(),
            absolute,
            stdout: workspace.path.join(format!("{index}.stdout")),
            stderr: workspace.path.join(format!("{index}.stderr")),
            phase: Phase::Queued,
        })
        .collect();
    let mut custodian_pid = None;
    let mut batch_error = None;
    let mut clean = false;
    let mut success = false;
    if let Some(report) = &report_file {
        report.invalidate_previous()?;
    }
    // Once custody starts, even an error diagnostic can block inherited pipes.
    // Managed failures use the report and bounded final writer, never parent stdio.
    let managed = (|| -> Result<(), String> {
        let custody = Custody::start(
            &executable,
            &custody_path,
            &CustodyConfig {
                resources: binding
                    .resources
                    .iter()
                    .map(|resource| resource.socket_addr())
                    .collect::<Result<_, _>>()?,
                session_timeout_ms: u64::try_from(budget.as_millis())
                    .map_err(|_| "batch duration is not representable")?,
                cleanup_timeout_ms: binding.cleanup_timeout_ms,
            },
        )
        .map_err(|error| format!("cannot establish batch custody: {error}"))?;
        workspace.retained = true;
        custodian_pid = Some(custody.pid());
        let broker =
            CustodyBroker::start(custody, jobs).map_err(|_| "cannot start custody broker")?;
        let mut publisher = Publisher {
            custody: CustodyAccess::Scoped(
                broker
                    .register(
                        "publication".to_owned(),
                        execution_deadline,
                        Duration::from_millis(binding.cleanup_timeout_ms),
                    )
                    .map_err(|_| "cannot reserve publication custody")?,
            ),
            stdio: publication_stdio,
            active: None,
        };
        let failed = Arc::new(AtomicBool::new(false));
        let (completed, incoming) = mpsc::sync_channel(jobs);
        let mut active: Vec<(usize, JoinHandle<Result<SessionOutcome, String>>)> = Vec::new();
        let mut next = 0;
        let mut published = 0;
        loop {
            let mut index = 0;
            while index < active.len() {
                if active[index].1.is_finished() {
                    let (document, handle) = active.swap_remove(index);
                    let result = handle
                        .join()
                        .unwrap_or_else(|_| Err("document coordinator panicked".to_owned()));
                    if !result.as_ref().is_ok_and(|outcome| outcome.status == 0) {
                        failed.store(true, Ordering::Release);
                    }
                    documents[document].phase = Phase::Finished(result);
                } else {
                    index += 1;
                }
            }
            let state = broker.status();
            match publisher.poll() {
                Ok(Some(index)) => published = index + 1,
                Ok(None) => {}
                Err(error) => {
                    failed.store(true, Ordering::Release);
                    broker.stop();
                    batch_error.get_or_insert(error);
                }
            }
            if state.failed {
                failed.store(true, Ordering::Release);
            }
            if state.uncertain || Instant::now() >= execution_deadline {
                failed.store(true, Ordering::Release);
                broker.stop();
                batch_error.get_or_insert_with(|| {
                    if state.uncertain {
                        "batch custody is uncertain".to_owned()
                    } else {
                        "batch deadline expired".to_owned()
                    }
                });
            }
            while !failed.load(Ordering::Acquire) && next < documents.len() && active.len() < jobs {
                // Reserving this ordinal admits its slot; later failure drains that scope.
                let index = next;
                next += 1;
                let document_deadline = Instant::now()
                    .checked_add(session)
                    .unwrap_or(execution_deadline)
                    .min(execution_deadline);
                let admission = (|| -> Result<_, String> {
                    let prepared = PreparedSession::from_context(
                        session_options(options),
                        binding.clone(),
                        command(&executable, &documents[index].absolute),
                        baseline.clone(),
                        workdir,
                        document_deadline,
                    )?
                    .observe_failure(failed.clone());
                    let scope = broker
                        .register(
                            format!("document-{index}"),
                            document_deadline,
                            Duration::from_millis(binding.cleanup_timeout_ms),
                        )
                        .map_err(|_| "cannot register document custody")?;
                    let stdio = AuthorStdio([
                        File::open("/dev/null")
                            .map_err(|_| "cannot open batch null input")?
                            .into(),
                        File::create(&documents[index].stdout)
                            .map_err(|_| "cannot create document output spool")?
                            .into(),
                        File::create(&documents[index].stderr)
                            .map_err(|_| "cannot create document diagnostics spool")?
                            .into(),
                    ]);
                    Ok((prepared, scope, stdio))
                })();
                match admission {
                    Ok((prepared, scope, stdio)) => {
                        let notification = completed.clone();
                        let observer = failed.clone();
                        match thread::Builder::new()
                            .name(format!("mdrun-document-{index}"))
                            .spawn(move || {
                                let result = run_prepared(prepared, stdio, Some(scope));
                                if !result.as_ref().is_ok_and(|outcome| outcome.status == 0) {
                                    observer.store(true, Ordering::Release);
                                }
                                let _ = notification.try_send(());
                                result
                            }) {
                            Ok(handle) => {
                                documents[index].phase = Phase::Running;
                                active.push((index, handle));
                            }
                            Err(_) => {
                                failed.store(true, Ordering::Release);
                                documents[index].phase = Phase::Finished(Err(
                                    "cannot start document coordinator".to_owned(),
                                ));
                            }
                        }
                    }
                    Err(error) => {
                        failed.store(true, Ordering::Release);
                        documents[index].phase = Phase::Finished(Err(error));
                    }
                }
            }
            while batch_error.is_none()
                && publisher.active.is_none()
                && published < documents.len()
                && matches!(documents[published].phase, Phase::Finished(_))
            {
                match publisher.start(
                    published,
                    &documents[published],
                    &executable,
                    &baseline,
                    &workspace.path,
                ) {
                    Ok(true) => break,
                    Ok(false) => published += 1,
                    Err(error) => {
                        failed.store(true, Ordering::Release);
                        broker.stop();
                        batch_error.get_or_insert(error);
                    }
                }
            }
            if active.is_empty()
                && (failed.load(Ordering::Acquire) || next == documents.len())
                && (publisher.active.is_none() || state.finished)
                && (published == next || batch_error.is_some())
            {
                break;
            }
            let _ = incoming.recv_timeout(TICK);
        }
        let cleanup_deadline = Instant::now()
            .checked_add(cleanup)
            .ok_or("batch cleanup deadline is not representable")?;
        let status = broker.finish(cleanup_deadline).unwrap_or_else(|_| {
            batch_error.get_or_insert_with(|| {
                "batch cleanup did not establish released custody".to_owned()
            });
            super::broker::BrokerStatus {
                failed: true,
                uncertain: true,
                ..Default::default()
            }
        });
        clean = status.stopped && !status.uncertain;
        workspace.retained = !clean;
        success =
            !failed.load(Ordering::Acquire) && !status.failed && clean && batch_error.is_none();
        Ok(())
    })();
    if let Err(error) = managed {
        batch_error = Some(error);
    }
    let entries: Vec<_> = documents
        .iter()
        .enumerate()
        .map(|(index, document)| {
            let mut entry = input_entry(index, &document.input);
            match &document.phase {
                Phase::Finished(Ok(outcome)) => {
                    entry["state"] = json!("completed");
                    entry["status"] = json!(outcome.status);
                    entry["session"] = outcome.report.clone();
                }
                Phase::Finished(Err(error)) => {
                    entry["state"] = json!("failed");
                    entry["error"] = json!(error);
                }
                Phase::Queued => {
                    entry["state"] = json!("not_started");
                    entry["reason"] = json!("admission_stopped");
                }
                Phase::Running => unreachable!("all coordinators were joined"),
            }
            entry
        })
        .collect();
    let report = json!({"version":1,"tool":{"name":env!("CARGO_PKG_NAME"),"version":env!("CARGO_PKG_VERSION")},
    "binding":{"name":binding.name,"artifact_id":binding.artifact_id},"outcome":if success {"success"} else {"failure"},
    "concurrency":{"requested":options.jobs.get(),"effective":jobs,"share_resources":options.share_resources},
    "budget_ms":{"document_session":session.as_millis(),"document_cleanup":cleanup.as_millis(),"batch":budget.as_millis()},
    "durations_ms":{"total":started.elapsed().as_millis()},"infrastructure_failure":batch_error,
    "cleanup":{"outcome":if custodian_pid.is_none() {"not_started"} else if clean {"clean"} else {"uncertain"},"custodian_pid":custodian_pid,"status_path":if !workspace.retained {None} else {Some(custody_path.with_extension("status.json"))}},"documents":entries});
    let publication = report_file
        .as_mut()
        .map(|file| file.publish(&report))
        .transpose();
    let report_failed = publication.is_err();
    let mut diagnostics = Vec::new();
    if let Some(error) = batch_error {
        diagnostics.push(error);
    }
    if let Err(error) = publication {
        diagnostics.push(error);
    }
    if !diagnostics.is_empty() {
        publish_diagnostics(&executable, &workspace.path, diagnostic_stderr, diagnostics);
    }
    Ok(i32::from(!success || report_failed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_lifetime_covers_each_wave_and_checked_cleanup() {
        assert_eq!(
            batch_budget(5, 2, Duration::from_secs(10), Duration::from_secs(3)).unwrap(),
            Duration::from_secs(42)
        );
        assert!(cleanup_grace(u64::MAX).is_none());
        assert!(batch_budget(2, 1, Duration::MAX, Duration::from_secs(1)).is_err());
    }

    #[test]
    fn native_input_attribution_preserves_distinct_bytes_despite_equal_display_names() {
        use std::os::unix::ffi::OsStringExt;
        let inputs = [b"doc-\xff.md".as_slice(), b"doc-\xfe.md".as_slice()];
        let entries =
            inputs.map(|bytes| input_entry(0, Path::new(&OsString::from_vec(bytes.to_vec()))));
        assert_eq!(entries[0]["file"], entries[1]["file"]);
        assert_eq!(
            entries[0]["file_native_bytes"],
            json!([100, 111, 99, 45, 255, 46, 109, 100])
        );
        assert_eq!(
            entries[1]["file_native_bytes"],
            json!([100, 111, 99, 45, 254, 46, 109, 100])
        );
    }
}
