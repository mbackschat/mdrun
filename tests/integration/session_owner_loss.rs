use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs;
use std::io;
use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const FIXTURE: &str = "session_owner_loss::payload_fixture";

#[derive(Debug, Deserialize, Serialize)]
pub(super) struct Process {
    pub(super) pid: i32,
    pub(super) parent: i32,
    pub(super) group: i32,
    identity: String,
}

pub(super) fn observe(pid: i32) -> io::Result<Option<Process>> {
    #[cfg(target_os = "macos")]
    {
        let mut info = unsafe { std::mem::zeroed::<libc::proc_bsdinfo>() };
        let bytes = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                0,
                (&mut info as *mut libc::proc_bsdinfo).cast(),
                size_of_val(&info) as i32,
            )
        };
        if bytes == 0 && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
            return Ok(None);
        }
        if bytes as usize != size_of_val(&info) {
            return Err(io::Error::other("cannot observe fixture process"));
        }
        if info.pbi_status == libc::SZOMB {
            return Ok(None);
        }
        Ok(Some(Process {
            pid,
            parent: info.pbi_ppid as i32,
            group: info.pbi_pgid as i32,
            identity: format!("{}:{}", info.pbi_start_tvsec, info.pbi_start_tvusec),
        }))
    }
    #[cfg(target_os = "linux")]
    {
        let stat = match fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(stat) => stat,
            Err(error) if process_disappeared(&error) => return Ok(None),
            Err(error) => return Err(error),
        };
        let (_, fields) = stat
            .rsplit_once(')')
            .ok_or_else(|| io::Error::other("invalid fixture process status"))?;
        let fields: Vec<_> = fields.split_whitespace().collect();
        if fields.len() < 20 {
            return Err(io::Error::other("incomplete fixture process status"));
        }
        if matches!(fields[0], "Z" | "X" | "x") {
            return Ok(None);
        }
        let number = |field: &str| {
            field
                .parse()
                .map_err(|_| io::Error::other("invalid fixture process identifier"))
        };
        Ok(Some(Process {
            pid,
            parent: number(fields[1])?,
            group: number(fields[2])?,
            identity: fields[19].to_owned(),
        }))
    }
}

fn process_disappeared(error: &io::Error) -> bool {
    // An open procfs stat may lose its task before reading, which reports ESRCH instead of ENOENT.
    // https://docs.kernel.org/filesystems/proc.html#process-specific-subdirectories
    matches!(error.raw_os_error(), Some(libc::ENOENT) | Some(libc::ESRCH))
}

#[test]
fn independent_procfs_observer_accepts_only_disappearance_errors() {
    for errno in [libc::ENOENT, libc::ESRCH] {
        assert!(
            process_disappeared(&io::Error::from_raw_os_error(errno)),
            "errno {errno}"
        );
    }
    for errno in [
        libc::EACCES,
        libc::EPERM,
        libc::EIO,
        libc::EBADF,
        libc::EINVAL,
        libc::EINTR,
    ] {
        assert!(
            !process_disappeared(&io::Error::from_raw_os_error(errno)),
            "errno {errno}"
        );
    }
    for kind in [
        io::ErrorKind::NotFound,
        io::ErrorKind::PermissionDenied,
        io::ErrorKind::InvalidData,
        io::ErrorKind::Other,
    ] {
        assert!(!process_disappeared(&io::Error::new(
            kind,
            "observation failed"
        )));
    }
}

impl Process {
    pub(super) fn alive(&self) -> bool {
        observe(self.pid)
            .unwrap()
            .is_some_and(|current| current.identity == self.identity)
    }
}

pub(super) fn wait_until(description: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !condition() {
        assert!(Instant::now() < deadline, "deadline expired: {description}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

pub(super) fn write_json(path: &Path, value: &impl Serialize) {
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, serde_json::to_vec(value).unwrap()).unwrap();
    fs::rename(temporary, path).unwrap();
}

pub(super) fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn read_process(directory: &Path, name: &str) -> Process {
    serde_json::from_slice(&fs::read(directory.join(name)).unwrap()).unwrap()
}

pub(super) fn excluded(address: SocketAddr) -> bool {
    match TcpListener::bind(address) {
        Ok(_) => false,
        Err(error) => {
            assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
            true
        }
    }
}

pub(super) struct Run {
    pub(super) directory: PathBuf,
    pub(super) child: Child,
    pub(super) private: Option<PathBuf>,
}

impl Run {
    pub(super) fn spawn(directory: PathBuf, command: &mut Command) -> Self {
        let stdout = fs::File::create(directory.join("stdout")).unwrap();
        let stderr = fs::File::create(directory.join("stderr")).unwrap();
        let child = command
            .env("MDRUN_OWNER_LOSS_DIRECTORY", &directory)
            .stdin(Stdio::null())
            .stdout(stdout)
            .stderr(stderr)
            .spawn()
            .unwrap_or_else(|error| {
                let _ = fs::remove_dir_all(&directory);
                panic!("cannot start custody fixture: {error}");
            });
        Self {
            directory,
            child,
            private: None,
        }
    }

    pub(super) fn kill_owner(&mut self) {
        self.child.kill().unwrap();
        self.child.wait().unwrap();
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        let _ = fs::write(self.directory.join("finished"), b"");
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
        if let Some(private) = &self.private {
            let status_path = private.join("custody.status.json");
            let deadline = Instant::now() + Duration::from_secs(2);
            let released = loop {
                let released = fs::read(&status_path)
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                    .is_some_and(|state| state["status"] == "released");
                if released || Instant::now() >= deadline {
                    break released;
                }
                std::thread::sleep(Duration::from_millis(5));
            };
            if released {
                let _ = fs::remove_dir_all(private);
            } else {
                eprintln!(
                    "owner-loss fixture retained custody status: {}",
                    status_path.display()
                );
            }
        }
        let _ = fs::remove_dir_all(&self.directory);
    }
}

#[test]
fn failed_fixture_startup_does_not_leave_its_owned_directory() {
    let mut leftovers = Vec::new();
    for executable_is_directory in [false, true] {
        let directory =
            std::env::temp_dir().join(format!("mdrun-failed-start-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&directory).unwrap();
        let executable = if executable_is_directory {
            directory.clone()
        } else {
            directory.join("missing")
        };
        let mut command = Command::new(executable);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| Run::spawn(
                directory.clone(),
                &mut command
            )))
            .is_err()
        );
        leftovers.push(directory.exists());
        if directory.exists() {
            fs::remove_dir_all(directory).unwrap();
        }
    }
    assert_eq!(leftovers, [false, false]);
}

fn fixture_command(role: &str, directory: &Path) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .args(["--exact", FIXTURE, "--nocapture", "--test-threads=1"])
        .env("MDRUN_OWNER_LOSS_ROLE", role)
        .env("MDRUN_OWNER_LOSS_DIRECTORY", directory);
    command
}

#[test]
fn payload_fixture() {
    let Ok(role) = std::env::var("MDRUN_OWNER_LOSS_ROLE") else {
        return;
    };
    assert!(matches!(
        role.as_str(),
        "direct_owner" | "cold" | "descendant"
    ));
    let directory = PathBuf::from(std::env::var_os("MDRUN_OWNER_LOSS_DIRECTORY").unwrap());
    let _listener = if role == "direct_owner" {
        Some(TcpListener::bind(std::env::var("MDRUN_OWNER_LOSS_ADDRESS").unwrap()).unwrap())
    } else {
        None
    };
    let mut descendant = None;
    if role == "descendant" {
        assert_ne!(
            unsafe { libc::signal(libc::SIGTERM, libc::SIG_IGN) },
            libc::SIG_ERR
        );
        write_json(
            &directory.join("descendant.json"),
            &observe(std::process::id() as i32).unwrap().unwrap(),
        );
    } else {
        descendant = Some(
            fixture_command("descendant", &directory)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        wait_until("descendant readiness", || {
            directory.join("descendant.json").exists()
        });
        write_json(
            &directory.join("payload.json"),
            &observe(std::process::id() as i32).unwrap().unwrap(),
        );
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    while !directory.join("finished").exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    if let Some(mut descendant) = descendant {
        assert!(descendant.wait().unwrap().success());
    }
}

#[test]
fn host_sigkill_retains_declared_listener_until_cold_descendant_cleanup() {
    let mut reservation = super::ports::Reservation::new();
    let address = reservation.address();
    let control_directory =
        std::env::temp_dir().join(format!("mdrun-owner-loss-control-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&control_directory).unwrap();
    let mut command = fixture_command("direct_owner", &control_directory);
    command.env("MDRUN_OWNER_LOSS_ADDRESS", address.to_string());
    reservation.release_listener();
    let mut control = Run::spawn(control_directory, &mut command);
    wait_until("direct-owner readiness", || {
        control.directory.join("payload.json").exists()
    });
    drop(reservation);
    let uncontrolled = read_process(&control.directory, "descendant.json");
    assert!(excluded(address));
    control.kill_owner();
    assert!(uncontrolled.alive());
    assert!(
        !excluded(address),
        "negative control unexpectedly retained its listener"
    );
    assert!(
        uncontrolled.alive(),
        "negative-control descendant exited during the probe"
    );
    fs::write(control.directory.join("finished"), b"").unwrap();
    wait_until("negative-control descendant exit", || !uncontrolled.alive());
    drop(control);

    let mut reservation = super::ports::Reservation::new();
    let address = reservation.address();

    let directory =
        std::env::temp_dir().join(format!("mdrun-owner-loss-managed-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&directory).unwrap();
    let executable = std::env::current_exe().unwrap();
    write_json(
        &directory.join("binding.json"),
        &json!({
            "version": 1,
            "name": "tool",
            "artifact_id": "native-owner-loss-fixture",
            "worker_argv": ["/mdrun-worker-must-not-start"],
            "cold_argv": [executable, "--exact", FIXTURE, "--nocapture", "--test-threads=1"],
            "resources": [{"kind": "tcp_listener", "host": "127.0.0.1", "port": address.port()}],
            "startup_timeout_ms": 2000,
            "session_timeout_ms": 15000,
            "cleanup_timeout_ms": 2000
        }),
    );
    let mut command = Command::new(env!("CARGO_BIN_EXE_mdrun"));
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("MDRUN_OWNER_LOSS_ROLE", "cold")
        .args(["session", "--mode", "cold", "--binding"])
        .arg(directory.join("binding.json"))
        .arg("--report")
        .arg(directory.join("report.json"))
        .args([
            "--",
            "/bin/sh",
            "-c",
            "printf '%s\\n' \"$MDRUN_SESSION\" > client-path; tool & client=$!; wait \"$client\"",
        ])
        .current_dir(&directory);
    reservation.release_listener();
    let mut managed = Run::spawn(directory, &mut command);
    wait_until("author session configuration", || {
        fs::read_to_string(managed.directory.join("client-path"))
            .ok()
            .is_some_and(|path| {
                Path::new(path.trim()).is_absolute() && Path::new(path.trim()).is_file()
            })
    });
    drop(reservation);
    let client_path = PathBuf::from(
        fs::read_to_string(managed.directory.join("client-path"))
            .unwrap()
            .trim(),
    );
    managed.private = Some(client_path.parent().unwrap().to_path_buf());
    let status_path = client_path.parent().unwrap().join("custody.status.json");
    wait_until("cold payload readiness", || {
        managed.directory.join("payload.json").exists()
    });
    let payload = read_process(&managed.directory, "payload.json");
    let descendant = read_process(&managed.directory, "descendant.json");
    assert_eq!(descendant.parent, payload.pid);
    assert_eq!(descendant.group, payload.group);
    assert!(payload.alive());
    assert!(descendant.alive());
    assert!(excluded(address));
    wait_until("author and cold custody registration", || {
        fs::read(&status_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .is_some_and(|status| {
                status["groups"]
                    .as_array()
                    .is_some_and(|groups| groups.len() == 2)
            })
    });
    let status = read_json(&status_path);
    let groups = status["groups"].as_array().unwrap();
    assert_eq!(
        groups.len(),
        2,
        "expected author and cold custody: {status}"
    );
    assert!(groups.iter().any(|group| group["id"] == "author"));
    assert!(
        groups
            .iter()
            .any(|group| group["anchor_pid"] == descendant.group)
    );
    assert!(!groups.iter().any(|group| group["id"] == "worker"));

    managed.kill_owner();
    assert!(
        descendant.alive(),
        "descendant did not survive actual host death"
    );
    assert!(
        excluded(address),
        "host death released the declared listener early"
    );
    let mut surviving_probes = 0;
    wait_until("owned descendant cleanup before listener release", || {
        let alive_before = descendant.alive();
        let held = excluded(address);
        let alive_after = descendant.alive();
        if alive_before && alive_after {
            surviving_probes += 1;
            assert!(
                held,
                "listener became available while the owned descendant survived"
            );
        }
        if !held {
            assert!(!alive_after, "listener release preceded descendant cleanup");
            assert!(!payload.alive(), "cold payload survived listener release");
        }
        !held && !alive_before
    });
    assert!(
        surviving_probes > 0,
        "no post-owner-loss custody observation"
    );
    wait_until("custodian released state", || {
        fs::read(&status_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .is_some_and(|status| status["status"] == "released")
    });
    let status = read_json(&status_path);
    assert!(
        status["groups"]
            .as_array()
            .unwrap()
            .iter()
            .all(|group| group["clean"] == true && group["uncertain"] == false),
        "{status}"
    );
    assert!(
        !managed.directory.join("report.json").exists(),
        "killed host unexpectedly completed reporting"
    );
}
