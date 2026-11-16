use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt, symlink};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{Value, json};

use super::broker::{CustodyAccess, ScopedCustody};
use super::client::{ClientConfig, NativeRequest, Reply};
use super::config::{Binding, Connect, WorkerConfig};
use super::context::{NativeContext, SESSION_ENV};
use super::custody::{self, Custody, CustodyConfig, CustodyEvent, PayloadStatus, SpawnSpec};
use super::wire::{self, Frame, Role, Terminal};
use super::{Mode, Options};

const TICK: Duration = Duration::from_millis(5);

#[derive(Clone)]
struct Identity {
    session_id: String,
    artifact_id: String,
    token: String,
}

struct PrivateDirectory {
    path: PathBuf,
    retained: bool,
}

impl PrivateDirectory {
    fn new() -> Result<Self, String> {
        let name = format!("mdrun-{}", uuid::Uuid::new_v4().simple());
        let mut path = std::env::temp_dir().join(&name);
        if path.to_str().is_none() || path.join("s").as_os_str().as_bytes().len() >= 104 {
            // The private JSON path must be UTF-8; macOS also limits sun_path to 104 bytes.
            // https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/un.h
            path = Path::new("/tmp").join(name);
        }
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .map_err(|_| "cannot create private session directory".to_owned())?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
            .map_err(|_| "cannot protect private session directory".to_owned())?;
        Ok(Self {
            path,
            retained: false,
        })
    }

    fn write(&self, name: &str, value: &impl Serialize) -> Result<PathBuf, String> {
        let path = self.path.join(name);
        let mut file = self.file(name)?;
        serde_json::to_writer(&mut file, value)
            .map_err(|_| "cannot encode private session configuration".to_owned())?;
        file.flush()
            .map_err(|_| "cannot flush private session configuration".to_owned())?;
        Ok(path)
    }

    fn file(&self, name: &str) -> Result<File, String> {
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(self.path.join(name))
            .map_err(|_| "cannot create private session file".to_owned())?;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|_| "cannot protect private session file".to_owned())?;
        Ok(file)
    }
}

impl Drop for PrivateDirectory {
    fn drop(&mut self) {
        if !self.retained {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

trait Socket: Read + Write + Send + 'static {
    fn duplicate(&self) -> std::io::Result<Self>
    where
        Self: Sized;
    fn close(&self);
}

impl Socket for TcpStream {
    fn duplicate(&self) -> std::io::Result<Self> {
        self.try_clone()
    }
    fn close(&self) {
        let _ = self.shutdown(Shutdown::Both);
    }
}

impl Socket for UnixStream {
    fn duplicate(&self) -> std::io::Result<Self> {
        self.try_clone()
    }
    fn close(&self) {
        let _ = self.shutdown(Shutdown::Both);
    }
}

struct Reader<S: Socket> {
    socket: S,
    frames: Receiver<Result<Option<Frame>, String>>,
}

impl<S: Socket> Reader<S> {
    fn new(socket: S) -> Result<Self, String> {
        let mut input = socket
            .duplicate()
            .map_err(|_| "cannot duplicate session socket".to_owned())?;
        let (sender, frames) = mpsc::sync_channel(1);
        thread::spawn(move || {
            loop {
                let frame = wire::read_frame(&mut input);
                let done = !matches!(frame, Ok(Some(_)));
                if sender.send(frame).is_err() || done {
                    break;
                }
            }
        });
        Ok(Self { socket, frames })
    }
}

impl<S: Socket> Drop for Reader<S> {
    fn drop(&mut self) {
        self.socket.close();
    }
}

struct Writer<S: Socket, T> {
    socket: S,
    outgoing: SyncSender<T>,
    delivered: Receiver<Result<(), String>>,
    inflight: usize,
}

impl<S: Socket, T: Serialize + Send + 'static> Writer<S, T> {
    fn new(socket: S) -> Result<Self, String> {
        let mut output = socket
            .duplicate()
            .map_err(|_| "cannot duplicate session socket".to_owned())?;
        let (outgoing, messages) = mpsc::sync_channel::<T>(1);
        let (completed, delivered) = mpsc::sync_channel(1);
        thread::spawn(move || {
            while let Ok(message) = messages.recv() {
                let result = wire::write_message(&mut output, &message);
                let failed = result.is_err();
                if completed.send(result).is_err() || failed {
                    break;
                }
            }
        });
        Ok(Self {
            socket,
            outgoing,
            delivered,
            inflight: 0,
        })
    }

    fn flush(&mut self, pending: &mut Option<T>) -> Result<(), String> {
        while let Ok(result) = self.delivered.try_recv() {
            self.inflight = self.inflight.saturating_sub(1);
            result?;
        }
        if let Some(message) = pending.take() {
            match self.outgoing.try_send(message) {
                Ok(()) => self.inflight += 1,
                Err(TrySendError::Full(message)) => *pending = Some(message),
                Err(TrySendError::Disconnected(_)) => {
                    return Err("session writer stopped".to_owned());
                }
            }
        }
        Ok(())
    }
}

impl<S: Socket, T> Drop for Writer<S, T> {
    fn drop(&mut self) {
        self.socket.close();
    }
}

struct Worker {
    data: Reader<TcpStream>,
    writer: Writer<TcpStream, Frame>,
    control: Reader<TcpStream>,
    pending: Option<Frame>,
}

impl Worker {
    fn shutdown(&mut self, timeout: Duration) -> Result<(), String> {
        self.control
            .socket
            .set_write_timeout(Some(timeout))
            .map_err(|_| "cannot bound worker shutdown write".to_owned())?;
        wire::write_frame(&mut self.control.socket, &Frame::Shutdown)
    }

    fn close(&self) {
        self.data.socket.close();
        self.writer.socket.close();
        self.control.socket.close();
    }
}

struct Connection {
    request: NativeRequest,
    descriptors: Vec<OwnedFd>,
    socket: UnixStream,
}

struct Handshake {
    pending: HashMap<u64, UnixStream>,
    next_id: u64,
    sender: SyncSender<(u64, Result<Connection, String>)>,
    completed: Receiver<(u64, Result<Connection, String>)>,
}

impl Handshake {
    fn new() -> Self {
        let (sender, completed) = mpsc::sync_channel(2);
        Self {
            pending: HashMap::new(),
            next_id: 0,
            sender,
            completed,
        }
    }

    fn accept(
        &mut self,
        listener: &UnixListener,
        identity: &Identity,
        deadline: Instant,
    ) -> Result<(), String> {
        loop {
            let (mut socket, _) = match listener.accept() {
                Ok(connection) => connection,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
                Err(_) => return Err("cannot accept native session client".to_owned()),
            };
            if self.pending.len() >= 2 {
                socket.close();
                return Err("overlapping native session clients".to_owned());
            }
            let timeout = remaining(deadline)?;
            socket
                .set_nonblocking(false)
                .map_err(|_| "cannot configure native socket".to_owned())?;
            socket
                .set_read_timeout(Some(timeout))
                .map_err(|_| "cannot set native read deadline".to_owned())?;
            socket
                .set_write_timeout(Some(timeout))
                .map_err(|_| "cannot set native write deadline".to_owned())?;
            let id = self.next_id;
            self.next_id += 1;
            self.pending.insert(
                id,
                socket
                    .try_clone()
                    .map_err(|_| "cannot duplicate native socket".to_owned())?,
            );
            let identity = identity.clone();
            let sender = self.sender.clone();
            thread::spawn(move || {
                let result = (|| {
                    let hello = wire::read_frame(&mut socket)?
                        .ok_or("client closed before authentication")?;
                    authenticate(&hello, &identity, Role::Client)?;
                    let request = wire::read_message::<_, NativeRequest>(&mut socket)?
                        .ok_or("client closed before native request")?;
                    validate_request(&request)?;
                    let descriptors = custody::receive_fds(&socket, 3)
                        .map_err(|_| "cannot receive original client descriptors")?;
                    Ok(Connection {
                        request,
                        descriptors,
                        socket,
                    })
                })()
                .map_err(|_: String| "invalid native client handshake".to_owned());
                let _ = sender.send((id, result));
            });
        }
    }
}

impl Drop for Handshake {
    fn drop(&mut self) {
        for socket in self.pending.values() {
            socket.close();
        }
    }
}

#[derive(Clone, Copy)]
enum Phase {
    Classifying,
    Accepting,
    Running,
    WarmAcknowledgement(u8),
    WorkerRelease,
    ColdRunning,
    ColdAcknowledgement,
    ClientRelease,
}

struct Invocation {
    request: NativeRequest,
    descriptors: Vec<OwnedFd>,
    input: Reader<UnixStream>,
    output: Writer<UnixStream, Reply>,
    pending: Option<Reply>,
    phase: Phase,
    sequence: u64,
    credit: Option<u32>,
    eof: bool,
    cold_group: Option<String>,
    cold_status: Option<PayloadStatus>,
    cold_clean: bool,
}

impl Invocation {
    fn new(connection: Connection) -> Result<Self, String> {
        let input = Reader::new(
            connection
                .socket
                .try_clone()
                .map_err(|_| "cannot duplicate native socket".to_owned())?,
        )?;
        let output = Writer::new(connection.socket)?;
        Ok(Self {
            request: connection.request,
            descriptors: connection.descriptors,
            input,
            output,
            pending: None,
            phase: Phase::Classifying,
            sequence: 0,
            credit: None,
            eof: false,
            cold_group: None,
            cold_status: None,
            cold_clean: false,
        })
    }

    fn send(&mut self, frame: Frame) -> Result<(), String> {
        if self.pending.is_some() {
            return Err("native output queue overflow".to_owned());
        }
        self.pending = Some(Reply::Worker { frame });
        Ok(())
    }

    fn id(&self) -> &str {
        &self.request.request_id
    }
}

#[derive(Default)]
struct GroupState {
    status: Option<PayloadStatus>,
    clean: bool,
}

struct State {
    author: GroupState,
    worker: GroupState,
    infrastructure: Option<String>,
    stopped: bool,
    uncertain: bool,
    shutdown: bool,
    warm_calls: u64,
    cold_calls: u64,
    cold_reasons: BTreeMap<String, u64>,
    application_statuses: Vec<Value>,
    failure_observer: Option<Arc<AtomicBool>>,
}

impl State {
    fn new(failure_observer: Option<Arc<AtomicBool>>) -> Self {
        Self {
            author: GroupState::default(),
            worker: GroupState::default(),
            infrastructure: None,
            stopped: false,
            uncertain: false,
            shutdown: false,
            warm_calls: 0,
            cold_calls: 0,
            cold_reasons: BTreeMap::new(),
            application_statuses: Vec::new(),
            failure_observer,
        }
    }

    fn fail(&mut self, reason: impl Into<String>) {
        self.observe_failure();
        self.infrastructure.get_or_insert_with(|| reason.into());
    }

    fn observe_failure(&self) {
        if let Some(observer) = &self.failure_observer {
            observer.store(true, Ordering::Release);
        }
    }

    fn event(&mut self, event: CustodyEvent, active: Option<&mut Invocation>) {
        match event {
            CustodyEvent::Exited { id, status } => {
                if id == "author" {
                    if status_code(&status) != 0 {
                        self.observe_failure();
                    }
                    self.author.status = Some(status);
                } else if id == "worker" {
                    if self.shutdown && status_code(&status) != 0 {
                        self.fail("worker orderly shutdown failed");
                    }
                    self.worker.status = Some(status);
                    if !self.shutdown {
                        self.fail("worker exited before orderly shutdown");
                    }
                } else if let Some(active) = active {
                    if active.cold_group.as_deref() == Some(&id) {
                        active.cold_status = Some(status);
                    } else {
                        self.fail("unknown payload exit");
                    }
                } else {
                    self.fail("unknown payload exit");
                }
            }
            CustodyEvent::Clean { id, forced } => {
                if forced {
                    self.fail("owned payload required forced cleanup");
                }
                if id == "author" {
                    self.author.clean = true;
                } else if id == "worker" {
                    self.worker.clean = true;
                } else if let Some(active) = active {
                    if active.cold_group.as_deref() == Some(&id) {
                        active.cold_clean = true;
                    } else {
                        self.fail("unknown payload cleanup");
                    }
                } else {
                    self.fail("unknown payload cleanup");
                }
            }
            CustodyEvent::Uncertain { reason, .. } => {
                self.uncertain = true;
                self.fail(format!("payload custody is uncertain: {reason}"));
            }
            CustodyEvent::Fault { reason } => {
                self.fail(format!("native custody infrastructure failure: {reason}"))
            }
            CustodyEvent::Stopped { clean } => {
                self.stopped = clean;
                if !clean {
                    self.uncertain = true;
                    self.fail("native custody stopped without clean ownership");
                }
            }
            CustodyEvent::Ready | CustodyEvent::Started { .. } => {}
            CustodyEvent::ScopeRegistered { .. }
            | CustodyEvent::ScopeRejected { .. }
            | CustodyEvent::ScopeEvent { .. }
            | CustodyEvent::ScopeStopped { .. } => {
                self.fail("unexpected scoped custody event in standalone session");
            }
        }
    }
}

pub struct PreparedSession {
    options: Options,
    binding: Binding,
    command: Vec<OsString>,
    baseline: NativeContext,
    baseline_cold: Option<&'static str>,
    started: Instant,
    deadline: Instant,
    deadline_unix_ms: u64,
    executable: PathBuf,
    failure_observer: Option<Arc<AtomicBool>>,
}

pub struct AuthorStdio(pub [OwnedFd; 3]);

impl AuthorStdio {
    fn inherited() -> Result<Self, String> {
        let duplicate = |descriptor| {
            unsafe { BorrowedFd::borrow_raw(descriptor) }
                .try_clone_to_owned()
                .map_err(|_| "cannot duplicate author descriptor".to_owned())
        };
        Ok(Self([duplicate(0)?, duplicate(1)?, duplicate(2)?]))
    }
}

pub struct SessionOutcome {
    pub status: i32,
    pub report: Value,
    pub diagnostics: Vec<String>,
}

impl SessionOutcome {
    fn publish(&self, report: Option<&mut super::report::ReportFile>) -> Result<(), String> {
        for diagnostic in &self.diagnostics {
            eprintln!("{diagnostic}");
        }
        if let Some(report) = report {
            report.publish(&self.report)?;
        }
        Ok(())
    }
}

impl PreparedSession {
    pub fn from_context(
        options: Options,
        binding: Binding,
        command: Vec<OsString>,
        mut baseline: NativeContext,
        workdir: Option<&Path>,
        deadline: Instant,
    ) -> Result<Self, String> {
        if baseline
            .environment_value(OsStr::new(SESSION_ENV))
            .is_some()
        {
            return Err("nested managed sessions are unsupported".to_owned());
        }
        let started = Instant::now();
        if command.is_empty() {
            return Err("session requires an author command".to_owned());
        }
        baseline.apply_binding(&binding, workdir)?;
        baseline.cwd = fs::canonicalize(&baseline.cwd)
            .map_err(|_| "cannot resolve session working directory".to_owned())?;
        if !baseline.cwd.is_dir() {
            return Err("session working directory is not a directory".to_owned());
        }
        let baseline_cold = if options.mode == Mode::Cold {
            Some("explicit_cold")
        } else {
            baseline.cold_reason(
                &baseline,
                &binding,
                &[],
                Terminal {
                    stdin: false,
                    stdout: false,
                    stderr: false,
                },
            )
        };
        if options.require_warm && baseline_cold.is_some() {
            return Err(
                "warm execution is required but the baseline requires cold execution".to_owned(),
            );
        }
        let deadline_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "system clock precedes epoch")?
            .as_millis()
            .checked_add(u128::from(remaining(deadline)?.as_millis() as u64))
            .and_then(|value| u64::try_from(value).ok())
            .ok_or("session deadline is not representable")?;
        let executable = std::env::current_exe()
            .map_err(|_| "cannot resolve native host executable".to_owned())?;
        Ok(Self {
            options,
            binding,
            command,
            baseline,
            baseline_cold,
            started,
            deadline,
            deadline_unix_ms,
            executable,
            failure_observer: None,
        })
    }

    pub fn observe_failure(mut self, observer: Arc<AtomicBool>) -> Self {
        self.failure_observer = Some(observer);
        self
    }
}

pub fn run(options: &Options, command: &[OsString], workdir: Option<&Path>) -> Result<i32, String> {
    if std::env::var_os(SESSION_ENV).is_some() {
        return Err("nested managed sessions are unsupported".to_owned());
    }
    let started = Instant::now();
    let binding = Binding::from_file(&options.binding)?;
    let baseline = NativeContext::capture()?;
    let mut report_file = options
        .report
        .as_ref()
        .map(|report| {
            let report = baseline.cwd.join(report);
            if super::report::conflicts(&report, &[baseline.cwd.join(&options.binding)])? {
                return Err("execution report conflicts with the binding".to_owned());
            }
            super::report::ReportFile::prepare(report)
        })
        .transpose()?;
    let stdio = AuthorStdio::inherited()?;
    let deadline = started
        .checked_add(Duration::from_millis(binding.session_timeout_ms))
        .ok_or("session deadline is not representable")?;
    let mut prepared = PreparedSession::from_context(
        options.clone(),
        binding,
        command.to_vec(),
        baseline,
        workdir,
        deadline,
    )?;
    prepared.started = started;
    if let Some(report) = &report_file {
        report.invalidate_previous()?;
    }
    let outcome = run_prepared(prepared, stdio, None)?;
    outcome.publish(report_file.as_mut())?;
    Ok(outcome.status)
}

pub fn run_prepared(
    prepared: PreparedSession,
    stdio: AuthorStdio,
    scoped: Option<ScopedCustody>,
) -> Result<SessionOutcome, String> {
    let PreparedSession {
        options,
        binding,
        command,
        baseline,
        baseline_cold,
        started,
        deadline,
        deadline_unix_ms,
        executable,
        failure_observer,
    } = prepared;
    let mut directory = PrivateDirectory::new()?;
    let bin = directory.path.join("bin");
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&bin)
        .map_err(|_| "cannot create private binding directory".to_owned())?;
    fs::set_permissions(&bin, fs::Permissions::from_mode(0o700))
        .map_err(|_| "cannot protect private binding directory".to_owned())?;
    symlink(&executable, bin.join(&binding.name))
        .map_err(|_| "cannot create native PATH binding".to_owned())?;
    let socket_path = directory.path.join("s");
    let listener = UnixListener::bind(&socket_path)
        .map_err(|_| "cannot create native client listener".to_owned())?;
    fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))
        .map_err(|_| "cannot protect native client listener".to_owned())?;
    listener
        .set_nonblocking(true)
        .map_err(|_| "cannot configure native listener".to_owned())?;
    let identity = Identity {
        session_id: uuid::Uuid::new_v4().to_string(),
        artifact_id: binding.artifact_id.clone(),
        token: format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        ),
    };
    let client_config = ClientConfig {
        protocol: 1,
        session_id: identity.session_id.clone(),
        artifact_id: identity.artifact_id.clone(),
        token: identity.token.clone(),
        binding_name: binding.name.clone(),
        socket: socket_path,
        binding_directory: bin.clone(),
        baseline: baseline.clone(),
        deadline_unix_ms,
    };
    let client_path = directory.write("client.json", &client_config)?;
    let custody_path = directory.path.join("custody.json");
    let mut author_context = baseline.clone();
    author_context.inject(&bin, &client_path)?;
    let mut author_argv = command.to_vec();
    author_argv[0] = resolve(&author_argv[0], &author_context)?.into_os_string();
    let mut custody = match scoped {
        Some(custody) => CustodyAccess::Scoped(custody),
        None => CustodyAccess::Standalone(
            Custody::start(
                &executable,
                &custody_path,
                &CustodyConfig {
                    resources: binding
                        .resources
                        .iter()
                        .map(|resource| resource.socket_addr())
                        .collect::<Result<_, _>>()?,
                    session_timeout_ms: binding.session_timeout_ms,
                    cleanup_timeout_ms: binding.cleanup_timeout_ms,
                },
            )
            .map_err(|error| format!("cannot establish native custody: {error}"))?,
        ),
    };
    directory.retained = true;
    let custody_status_path = custody.status_path().to_owned();
    let mut state = State::new(failure_observer);
    let mut worker = None;
    let mut starts = 0;
    let mut replay_started = None;
    let execution = (|| -> Result<(), String> {
        if baseline_cold.is_none() {
            let worker_listener = TcpListener::bind(("127.0.0.1", 0))
                .map_err(|_| "cannot create worker listener".to_owned())?;
            worker_listener
                .set_nonblocking(true)
                .map_err(|_| "cannot configure worker listener".to_owned())?;
            let worker_config = WorkerConfig {
                protocol: 1,
                session_id: identity.session_id.clone(),
                artifact_id: identity.artifact_id.clone(),
                token: identity.token.clone(),
                connect: Connect {
                    host: "127.0.0.1".to_owned(),
                    port: worker_listener
                        .local_addr()
                        .map_err(|_| "cannot inspect worker listener".to_owned())?
                        .port(),
                },
            };
            worker_config.validate()?;
            let worker_path = directory.write("worker.json", &worker_config)?;
            let diagnostics = directory.file("worker.log")?;
            let null = File::open("/dev/null").map_err(|_| "cannot open worker null input")?;
            let mut argv: Vec<OsString> = binding.worker_argv.iter().map(Into::into).collect();
            argv.push(worker_path.into_os_string());
            custody
                .spawn(
                    "worker",
                    &spec(argv, &baseline),
                    [null.as_fd(), diagnostics.as_fd(), diagnostics.as_fd()],
                )
                .map_err(|_| "cannot launch worker under native custody")?;
            starts += 1;
            let startup = Instant::now()
                .checked_add(Duration::from_millis(binding.startup_timeout_ms))
                .ok_or("startup deadline is not representable")?
                .min(deadline);
            worker = Some(start_worker(
                &worker_listener,
                &identity,
                startup,
                deadline,
                &mut custody,
                &mut state,
            )?);
        }
        replay_started = Some(Instant::now());
        custody
            .spawn(
                "author",
                &spec(author_argv, &author_context),
                [stdio.0[0].as_fd(), stdio.0[1].as_fd(), stdio.0[2].as_fd()],
            )
            .map_err(|_| "cannot launch author under native custody")?;
        coordinate(
            Scope {
                options: &options,
                binding: &binding,
                baseline: &baseline,
                baseline_cold,
                listener: &listener,
                identity: &identity,
                deadline,
            },
            &mut custody,
            &mut state,
            &mut worker,
        )
    })();
    if let Err(reason) = execution {
        state.fail(reason);
    }
    if let Some(worker) = &worker {
        worker.close();
    }
    let _ = custody.stop();
    let cleanup_deadline = Instant::now()
        .checked_add(
            Duration::from_millis(binding.cleanup_timeout_ms).saturating_mul(2)
                + Duration::from_millis(500),
        )
        .ok_or("cleanup deadline is not representable")?;
    while !state.stopped && Instant::now() < cleanup_deadline {
        match custody.poll_event(TICK) {
            Ok(Some(event)) => state.event(event, None),
            Ok(None) => {}
            Err(_) => {
                state.uncertain = true;
                state.fail("native custody control was lost during cleanup");
                break;
            }
        }
    }
    if !state.stopped {
        state.uncertain = true;
        state.fail("cleanup did not establish released custody");
    }
    directory.retained = !state.stopped;
    let author_status = state.author.status.as_ref().map(status_json);
    let result = state.author.status.as_ref().map(status_code).unwrap_or(1);
    let success = state.infrastructure.is_none() && state.stopped && result == 0;
    let report = json!({
        "version":1,"session_id":identity.session_id,"artifact_id":binding.artifact_id,"binding":{"name":binding.name},
        "tool":{"name":env!("CARGO_PKG_NAME"),"version":env!("CARGO_PKG_VERSION")},
        "outcome":if success {"success"} else {"failure"}, "author_status":author_status,
        "application_statuses":state.application_statuses,"infrastructure_failure":state.infrastructure,
        "cleanup":{"outcome":if state.stopped {"clean"} else {"uncertain"},"custodian_pid":custody.pid(),
            "status_path":if directory.retained {Some(&custody_status_path)} else {None}},
        "warm_calls":state.warm_calls,"cold_calls":state.cold_calls,"cold_reasons":state.cold_reasons,"worker_starts":starts,
        "durations_ms":{"preparation":null,"replay":replay_started.map(|started| started.elapsed().as_millis()).unwrap_or(0),"total":started.elapsed().as_millis()}
    });
    let mut diagnostics = Vec::new();
    if let Some(reason) = &state.infrastructure {
        diagnostics.push(format!("mdrun: session infrastructure failure: {reason}"));
    }
    if directory.retained {
        diagnostics.push(format!(
            "mdrun: custody retained; status: {}",
            custody_status_path.display()
        ));
    }
    let status = if state.infrastructure.is_some() || !state.stopped {
        1
    } else {
        result
    };
    Ok(SessionOutcome {
        status,
        report,
        diagnostics,
    })
}

fn start_worker(
    listener: &TcpListener,
    identity: &Identity,
    deadline: Instant,
    session_deadline: Instant,
    custody: &mut CustodyAccess,
    state: &mut State,
) -> Result<Worker, String> {
    let mut data = None;
    let mut control = None;
    while data.is_none() || control.is_none() {
        remaining(deadline)?;
        if let Some(event) = custody
            .poll_event(TICK)
            .map_err(|_| "worker startup custody control failed")?
        {
            state.event(event, None);
        }
        if state.infrastructure.is_some() {
            return Err("worker exited before readiness".to_owned());
        }
        match listener.accept() {
            Ok((mut socket, _)) => {
                socket
                    .set_nonblocking(false)
                    .map_err(|_| "cannot configure worker socket")?;
                let timeout = remaining(deadline)?;
                socket
                    .set_read_timeout(Some(timeout))
                    .map_err(|_| "cannot configure worker read deadline")?;
                socket
                    .set_write_timeout(Some(timeout))
                    .map_err(|_| "cannot configure worker write deadline")?;
                let hello =
                    wire::read_frame(&mut socket)?.ok_or("worker closed before authentication")?;
                match &hello {
                    Frame::Hello {
                        role: Role::WorkerData,
                        ..
                    } if data.is_none() => {
                        authenticate(&hello, identity, Role::WorkerData)?;
                        data = Some(socket);
                    }
                    Frame::Hello {
                        role: Role::WorkerControl,
                        ..
                    } if control.is_none() => {
                        authenticate(&hello, identity, Role::WorkerControl)?;
                        control = Some(socket);
                    }
                    _ => return Err("duplicate or invalid worker role".to_owned()),
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => return Err("cannot accept worker connection".to_owned()),
        }
    }
    let data = data.unwrap();
    let control = control.unwrap();
    let timeout = remaining(session_deadline)?;
    for socket in [&data, &control] {
        socket
            .set_read_timeout(Some(timeout))
            .map_err(|_| "cannot set worker session deadline")?;
        socket
            .set_write_timeout(Some(timeout))
            .map_err(|_| "cannot set worker session deadline")?;
    }
    let worker = Worker {
        writer: Writer::new(
            data.try_clone()
                .map_err(|_| "cannot duplicate worker socket")?,
        )?,
        data: Reader::new(data)?,
        control: Reader::new(control)?,
        pending: None,
    };
    loop {
        remaining(deadline)?;
        if let Ok(message) = worker.control.frames.try_recv() {
            return Err(control_error(message));
        }
        match worker.data.frames.try_recv() {
            Ok(Ok(Some(Frame::Ready {
                protocol: 1,
                session_id,
                artifact_id,
                ..
            }))) if session_id == identity.session_id && artifact_id == identity.artifact_id => {
                let timeout = remaining(session_deadline)?;
                worker
                    .data
                    .socket
                    .set_read_timeout(Some(timeout))
                    .map_err(|_| "cannot set worker session deadline")?;
                worker
                    .data
                    .socket
                    .set_write_timeout(Some(timeout))
                    .map_err(|_| "cannot set worker session deadline")?;
                worker
                    .control
                    .socket
                    .set_read_timeout(Some(timeout))
                    .map_err(|_| "cannot set worker control deadline")?;
                worker
                    .control
                    .socket
                    .set_write_timeout(Some(timeout))
                    .map_err(|_| "cannot set worker control deadline")?;
                return Ok(worker);
            }
            Ok(_) => return Err("worker sent invalid readiness".to_owned()),
            Err(TryRecvError::Disconnected) => {
                return Err("worker data reader stopped before readiness".to_owned());
            }
            Err(TryRecvError::Empty) => {}
        }
        if let Some(event) = custody
            .poll_event(TICK)
            .map_err(|_| "worker startup custody control failed")?
        {
            state.event(event, None);
        }
        if state.infrastructure.is_some() {
            return Err("worker exited before readiness".to_owned());
        }
    }
}

struct Scope<'a> {
    options: &'a Options,
    binding: &'a Binding,
    baseline: &'a NativeContext,
    baseline_cold: Option<&'a str>,
    listener: &'a UnixListener,
    identity: &'a Identity,
    deadline: Instant,
}

fn coordinate(
    scope: Scope<'_>,
    custody: &mut CustodyAccess,
    state: &mut State,
    worker: &mut Option<Worker>,
) -> Result<(), String> {
    let Scope {
        options,
        binding,
        baseline,
        baseline_cold,
        listener,
        identity,
        deadline,
    } = scope;
    let mut handshakes = Handshake::new();
    let mut active: Option<Invocation> = None;
    let mut deferred: Option<Connection> = None;
    let mut seen = HashSet::new();
    let mut shutdown_deadline = None;
    loop {
        remaining(deadline)?;
        if shutdown_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return Err("worker orderly shutdown deadline expired".to_owned());
        }
        if let Some(event) = custody
            .poll_event(TICK)
            .map_err(|_| "native custody control failed")?
        {
            state.event(event, active.as_mut());
        }
        if let Some(reason) = &state.infrastructure {
            return Err(reason.clone());
        }
        finish_release(&mut active)?;
        if let Some(worker) = worker.as_mut() {
            worker.writer.flush(&mut worker.pending)?;
            match worker.control.frames.try_recv() {
                Ok(message) if !state.shutdown => return Err(control_error(message)),
                Ok(Ok(None)) | Err(TryRecvError::Disconnected) if state.shutdown => {}
                Ok(message) => return Err(control_error(message)),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    return Err("worker control reader stopped".to_owned());
                }
            }
            if active
                .as_ref()
                .is_none_or(|active| active.pending.is_none())
            {
                match worker.data.frames.try_recv() {
                    Ok(Ok(Some(Frame::Refused { request_id, reason }))) => {
                        let invocation = active
                            .as_mut()
                            .ok_or("worker refused outside an invocation")?;
                        if !matches!(invocation.phase, Phase::Classifying)
                            || invocation.id() != request_id
                            || reason.contains(&identity.token)
                        {
                            return Err("worker refusal violates invocation state".to_owned());
                        }
                        route_cold(options, binding, &reason, invocation, custody, state)?;
                    }
                    Ok(Ok(Some(frame))) => worker_frame(frame, active.as_mut(), worker, state)?,
                    Ok(Ok(None)) | Err(TryRecvError::Disconnected) if state.shutdown => {}
                    Ok(Ok(None)) | Err(TryRecvError::Disconnected) => {
                        return Err("worker data channel closed".to_owned());
                    }
                    Ok(Err(_)) => return Err("worker data channel failed".to_owned()),
                    Err(TryRecvError::Empty) => {}
                }
            }
        }
        if !state.shutdown {
            handshakes.accept(listener, identity, deadline)?;
            loop {
                let connection = if active.is_none() && deferred.is_some() {
                    deferred.take().unwrap()
                } else {
                    let Ok((id, result)) = handshakes.completed.try_recv() else {
                        break;
                    };
                    handshakes.pending.remove(&id);
                    result?
                };
                let Some(connection) = receive_admission(&active, &mut deferred, connection)?
                else {
                    continue;
                };
                if !seen.insert(connection.request.request_id.clone()) {
                    return Err("duplicate invocation identity".to_owned());
                }
                let mut invocation = Invocation::new(connection)?;
                let reason = baseline_cold.map(str::to_owned).or_else(|| {
                    invocation
                        .request
                        .context
                        .cold_reason(
                            baseline,
                            binding,
                            &invocation.request.argv,
                            invocation.request.terminal,
                        )
                        .map(str::to_owned)
                });
                if let Some(reason) = reason {
                    route_cold(options, binding, &reason, &mut invocation, custody, state)?;
                } else {
                    let argv = invocation
                        .request
                        .argv
                        .iter()
                        .map(|argument| {
                            argument
                                .to_str()
                                .map(str::to_owned)
                                .ok_or("unrepresentable warm argument".to_owned())
                        })
                        .collect::<Result<_, _>>()?;
                    let worker = worker.as_mut().ok_or("warm worker is unavailable")?;
                    if worker.pending.is_some() {
                        return Err("worker request queue overflow".to_owned());
                    }
                    worker.pending = Some(Frame::Classify {
                        request_id: invocation.id().to_owned(),
                        argv,
                    });
                }
                active = Some(invocation);
            }
        }
        let mut release = false;
        if let Some(invocation) = active.as_mut() {
            if matches!(invocation.phase, Phase::ColdRunning) && invocation.cold_clean {
                let status = invocation
                    .cold_status
                    .as_ref()
                    .ok_or("cold cleanup completed without an observed application status")?;
                let (exit_code, signal) = native_status(status)?;
                if invocation.pending.is_none() {
                    invocation.pending = Some(Reply::ColdDone {
                        request_id: invocation.id().to_owned(),
                        exit_code,
                        signal,
                    });
                    invocation.phase = Phase::ColdAcknowledgement;
                    state.application_statuses.push(json!({"request_id":invocation.id(),"route":"cold","exit_code":exit_code,"signal":signal}));
                }
            }
            invocation.output.flush(&mut invocation.pending)?;
            if worker
                .as_ref()
                .is_none_or(|worker| worker.pending.is_none())
                && invocation.pending.is_none()
            {
                match invocation.input.frames.try_recv() {
                    Ok(Ok(Some(frame))) => client_frame(frame, invocation, worker.as_mut())?,
                    Ok(Ok(None)) | Err(TryRecvError::Disconnected)
                        if matches!(invocation.phase, Phase::ClientRelease) => {}
                    Ok(Ok(None)) | Err(TryRecvError::Disconnected) => {
                        return Err("native client disconnected before release".to_owned());
                    }
                    Ok(Err(_)) => return Err("native client transport failed".to_owned()),
                    Err(TryRecvError::Empty) => {}
                }
            }
            release = matches!(invocation.phase, Phase::ClientRelease)
                && invocation.pending.is_none()
                && invocation.output.inflight == 0;
        }
        if release {
            active = None;
        }
        if state.author.clean
            && state.author.status.is_some()
            && active.is_none()
            && deferred.is_none()
            && handshakes.pending.is_empty()
        {
            if !state.shutdown {
                state.shutdown = true;
                if let Some(worker) = worker.as_mut() {
                    let budget =
                        Duration::from_millis(binding.cleanup_timeout_ms).min(remaining(deadline)?);
                    shutdown_deadline = Instant::now().checked_add(budget);
                    worker.shutdown(budget)?;
                }
            }
            if worker.is_none() || (state.worker.clean && state.worker.status.is_some()) {
                return Ok(());
            }
        }
    }
}

fn finish_release(active: &mut Option<Invocation>) -> Result<(), String> {
    if let Some(invocation) = active.as_mut() {
        invocation.output.flush(&mut invocation.pending)?;
        if matches!(invocation.phase, Phase::ClientRelease)
            && invocation.pending.is_none()
            && invocation.output.inflight == 0
        {
            *active = None;
        }
    }
    Ok(())
}

fn receive_admission(
    active: &Option<Invocation>,
    deferred: &mut Option<Connection>,
    connection: Connection,
) -> Result<Option<Connection>, String> {
    if active
        .as_ref()
        .is_some_and(|invocation| matches!(invocation.phase, Phase::ClientRelease))
    {
        if deferred.is_some() {
            return Err("overlapping application invocations poison the session".to_owned());
        }
        *deferred = Some(connection);
        return Ok(None);
    }
    if active.is_some() {
        return Err("overlapping application invocations poison the session".to_owned());
    }
    Ok(Some(connection))
}

fn worker_frame(
    frame: Frame,
    active: Option<&mut Invocation>,
    worker: &mut Worker,
    state: &mut State,
) -> Result<(), String> {
    if matches!(frame, Frame::Fault { .. }) {
        return Err("worker reported an infrastructure fault".to_owned());
    }
    let active = active.ok_or("worker emitted data outside an invocation")?;
    if frame.request_id() != Some(active.id()) {
        return Err("worker invocation identity mismatch".to_owned());
    }
    match frame {
        Frame::Eligible { request_id } if matches!(active.phase, Phase::Classifying) => {
            if worker.pending.is_some() {
                return Err("worker request queue overflow".to_owned());
            }
            worker.pending = Some(Frame::Invoke { request_id });
            active.phase = Phase::Accepting;
            state.warm_calls += 1;
            Ok(())
        }
        frame @ Frame::Accepted { .. } if matches!(active.phase, Phase::Accepting) => {
            active.phase = Phase::Running;
            active.send(frame)
        }
        frame @ Frame::Output { sequence, .. } if matches!(active.phase, Phase::Running) => {
            if sequence != active.sequence {
                return Err("worker output sequence mismatch".to_owned());
            }
            active.sequence = active
                .sequence
                .checked_add(1)
                .ok_or("worker output sequence overflow")?;
            active.send(frame)
        }
        frame @ Frame::StdinRead { max_bytes, .. } if matches!(active.phase, Phase::Running) => {
            if active.credit.is_some() || active.eof {
                return Err("worker issued invalid stdin credit".to_owned());
            }
            active.credit = Some(max_bytes);
            active.send(frame)
        }
        frame @ Frame::Complete { exit_code, .. } if matches!(active.phase, Phase::Running) => {
            if active.credit.is_some() {
                return Err("worker completed with outstanding stdin credit".to_owned());
            }
            active.phase = Phase::WarmAcknowledgement(exit_code);
            state.application_statuses.push(json!({"request_id":active.id(),"route":"warm","exit_code":exit_code,"signal":null}));
            active.send(frame)
        }
        frame @ Frame::Released { .. } if matches!(active.phase, Phase::WorkerRelease) => {
            active.phase = Phase::ClientRelease;
            active.send(frame)
        }
        _ => Err("worker frame violates invocation state".to_owned()),
    }
}

fn client_frame(
    frame: Frame,
    active: &mut Invocation,
    worker: Option<&mut Worker>,
) -> Result<(), String> {
    if frame.request_id() != Some(active.id()) {
        return Err("native client invocation identity mismatch".to_owned());
    }
    match frame {
        frame @ Frame::Stdin { .. } if matches!(active.phase, Phase::Running) => {
            let Frame::Stdin { ref base64, .. } = frame else {
                unreachable!()
            };
            let credit = active
                .credit
                .take()
                .ok_or("native input arrived without credit")?;
            if active.eof || wire::decode_chunk(base64)?.len() > credit as usize {
                return Err("native input exceeds worker credit".to_owned());
            }
            worker.ok_or("worker disappeared during input")?.pending = Some(frame);
            Ok(())
        }
        frame @ Frame::StdinEof { .. } if matches!(active.phase, Phase::Running) => {
            if active.credit.take().is_none() || active.eof {
                return Err("native EOF arrived without credit".to_owned());
            }
            active.eof = true;
            worker.ok_or("worker disappeared during input")?.pending = Some(frame);
            Ok(())
        }
        Frame::ClientDone {
            exit_code, signal, ..
        } => match active.phase {
            Phase::WarmAcknowledgement(expected)
                if exit_code == Some(expected) && signal.is_none() =>
            {
                worker.ok_or("worker disappeared before release")?.pending = Some(Frame::Release {
                    request_id: active.id().to_owned(),
                });
                active.phase = Phase::WorkerRelease;
                Ok(())
            }
            Phase::ColdAcknowledgement
                if active.cold_status.as_ref().map(native_status).transpose()?
                    == Some((exit_code, signal)) =>
            {
                active.phase = Phase::ClientRelease;
                active.send(Frame::Released {
                    request_id: active.id().to_owned(),
                })
            }
            _ => Err("native completion acknowledgement is invalid".to_owned()),
        },
        _ => Err("native client frame violates invocation state".to_owned()),
    }
}

fn route_cold(
    options: &Options,
    binding: &Binding,
    reason: &str,
    active: &mut Invocation,
    custody: &mut CustodyAccess,
    state: &mut State,
) -> Result<(), String> {
    if options.require_warm {
        return Err(format!(
            "warm execution is required but invocation requires cold execution ({reason})"
        ));
    }
    active.send(Frame::Cold {
        request_id: active.id().to_owned(),
        reason: reason.to_owned(),
    })?;
    let mut argv: Vec<OsString> = binding.cold_argv.iter().map(Into::into).collect();
    argv.extend(active.request.argv.iter().cloned());
    let id = format!("cold-{}", uuid::Uuid::new_v4().simple());
    active.cold_group = Some(id.clone());
    active.phase = Phase::ColdRunning;
    state.cold_calls += 1;
    *state.cold_reasons.entry(reason.to_owned()).or_default() += 1;
    let descriptors = &active.descriptors;
    let launch = custody.spawn(
        &id,
        &spec(argv, &active.request.context),
        [
            descriptors[0].as_fd(),
            descriptors[1].as_fd(),
            descriptors[2].as_fd(),
        ],
    );
    if launch.is_err() {
        while let Ok(Some(event)) = custody.poll_event(Duration::ZERO) {
            state.event(event, Some(active));
        }
        return Err("cannot grant cold execution under native custody".to_owned());
    }
    Ok(())
}

fn spec(argv: Vec<OsString>, context: &NativeContext) -> SpawnSpec {
    SpawnSpec {
        argv,
        cwd: context.cwd.clone(),
        environment: context.environment.clone(),
        umask: context.umask,
        caller_group: context.caller_group,
    }
}

fn authenticate(frame: &Frame, identity: &Identity, expected: Role) -> Result<(), String> {
    match frame {
        Frame::Hello {
            protocol: 1,
            session_id,
            artifact_id,
            token,
            role,
        } if *role == expected
            && session_id == &identity.session_id
            && artifact_id == &identity.artifact_id
            && token == &identity.token =>
        {
            Ok(())
        }
        _ => Err("session authentication failed".to_owned()),
    }
}

fn validate_request(request: &NativeRequest) -> Result<(), String> {
    if request.request_id.is_empty()
        || request.context.umask > 0o777
        || !request.context.cwd.is_absolute()
        || request.context.caller_group <= 0
        || request
            .argv
            .iter()
            .any(|argument| argument.as_bytes().contains(&0))
        || request.context.environment.iter().any(|(key, value)| {
            key.is_empty()
                || key.as_bytes().contains(&b'=')
                || key.as_bytes().contains(&0)
                || value.as_bytes().contains(&0)
        })
    {
        return Err("invalid native request context".to_owned());
    }
    Ok(())
}

fn control_error(_message: Result<Option<Frame>, String>) -> String {
    "worker control channel fault or unexpected closure".to_owned()
}

fn remaining(deadline: Instant) -> Result<Duration, String> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or("session deadline expired".to_owned())
}

fn resolve(program: &OsStr, context: &NativeContext) -> Result<PathBuf, String> {
    let executable = |path: &Path| {
        fs::metadata(path)
            .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
    };
    if program.as_bytes().contains(&b'/') {
        let path = context.cwd.join(program);
        return if executable(&path) {
            Ok(path)
        } else {
            Err("author executable is unavailable".to_owned())
        };
    }
    if let Some(path) = context.environment_value(OsStr::new("PATH")) {
        for directory in std::env::split_paths(path) {
            let candidate = context.cwd.join(directory).join(program);
            if executable(&candidate) {
                return Ok(candidate);
            }
        }
    }
    Err("author executable was not found on session PATH".to_owned())
}

fn native_status(status: &PayloadStatus) -> Result<(Option<u8>, Option<u32>), String> {
    match (status.code, status.signal) {
        (Some(code), None) => Ok((
            Some(u8::try_from(code).map_err(|_| "invalid native exit status")?),
            None,
        )),
        (None, Some(signal)) if signal > 0 => Ok((None, Some(signal as u32))),
        _ => Err("native status is unavailable".to_owned()),
    }
}

fn status_json(status: &PayloadStatus) -> Value {
    json!({"exit_code":status.code,"signal":status.signal})
}
fn status_code(status: &PayloadStatus) -> i32 {
    status
        .code
        .unwrap_or_else(|| 128 + status.signal.unwrap_or(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> NativeRequest {
        NativeRequest {
            request_id: uuid::Uuid::new_v4().to_string(),
            argv: Vec::new(),
            context: NativeContext {
                cwd: PathBuf::from("/"),
                environment: Vec::new(),
                umask: 0o022,
                caller_group: 1,
            },
            terminal: Terminal {
                stdin: false,
                stdout: false,
                stderr: false,
            },
        }
    }

    fn connection() -> Connection {
        let (socket, _peer) = UnixStream::pair().unwrap();
        Connection {
            request: request(),
            descriptors: Vec::new(),
            socket,
        }
    }

    fn releasing() -> (Option<Invocation>, SyncSender<Result<(), String>>) {
        let (socket, _peer) = UnixStream::pair().unwrap();
        let (_input, frames) = mpsc::sync_channel(1);
        let (outgoing, _messages) = mpsc::sync_channel(1);
        let (completed, delivered) = mpsc::sync_channel(1);
        let invocation = Invocation {
            request: request(),
            descriptors: Vec::new(),
            input: Reader {
                socket: socket.try_clone().unwrap(),
                frames,
            },
            output: Writer {
                socket,
                outgoing,
                delivered,
                inflight: 1,
            },
            pending: None,
            phase: Phase::ClientRelease,
            sequence: 0,
            credit: None,
            eof: false,
            cold_group: None,
            cold_status: None,
            cold_clean: false,
        };
        (Some(invocation), completed)
    }

    #[test]
    fn next_authenticated_client_waits_for_final_release_delivery() {
        let (mut active, delivered) = releasing();
        let mut deferred = None;
        let next = connection();
        let next_id = next.request.request_id.clone();
        assert!(
            receive_admission(&active, &mut deferred, next)
                .unwrap()
                .is_none()
        );
        finish_release(&mut active).unwrap();
        assert!(active.is_some());
        assert_eq!(deferred.as_ref().unwrap().request.request_id, next_id);
        delivered.send(Ok(())).unwrap();
        finish_release(&mut active).unwrap();
        assert!(active.is_none());
        let next = deferred.take().unwrap();
        assert_eq!(
            receive_admission(&active, &mut deferred, next)
                .unwrap()
                .unwrap()
                .request
                .request_id,
            next_id
        );
    }

    #[test]
    fn failed_final_release_delivery_blocks_deferred_execution() {
        let (mut active, delivered) = releasing();
        let mut deferred = None;
        assert!(
            receive_admission(&active, &mut deferred, connection())
                .unwrap()
                .is_none()
        );
        delivered
            .send(Err("broken release delivery".to_owned()))
            .unwrap();
        assert!(finish_release(&mut active).is_err());
        assert!(active.is_some());
        assert!(deferred.is_some());
    }

    #[test]
    fn overlapping_active_and_multiple_deferred_clients_are_rejected() {
        let (mut active, _delivered) = releasing();
        let mut deferred = None;
        assert!(
            receive_admission(&active, &mut deferred, connection())
                .unwrap()
                .is_none()
        );
        assert!(receive_admission(&active, &mut deferred, connection()).is_err());
        active.as_mut().unwrap().phase = Phase::Running;
        deferred = None;
        assert!(receive_admission(&active, &mut deferred, connection()).is_err());
        assert!(deferred.is_none());
    }

    #[test]
    fn document_failure_closes_admission_before_its_cleanup_is_complete() {
        let failed = Arc::new(AtomicBool::new(false));
        let mut state = State::new(Some(failed.clone()));
        state.event(
            CustodyEvent::Exited {
                id: "author".to_owned(),
                status: PayloadStatus {
                    code: Some(1),
                    signal: None,
                },
            },
            None,
        );
        assert!(failed.load(Ordering::Acquire));
        assert!(!state.author.clean && !state.stopped);
        assert!(state.infrastructure.is_none());
        failed.store(false, Ordering::Release);
        let mut state = State::new(Some(failed.clone()));
        state.fail("infrastructure failed");
        assert!(failed.load(Ordering::Acquire));
        assert!(!state.stopped);
    }
}
