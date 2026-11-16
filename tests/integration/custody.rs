use serde_json::{Value, json};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::net::UnixStream;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

mod scopes;

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("mdrun-custody-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        if self.0.join("cleanup.failure").exists() {
            eprintln!(
                "custody fixture retained its cleanup evidence: {}",
                self.0.display()
            );
        } else {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

fn wait_until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !condition() {
        assert!(Instant::now() < deadline, "custody oracle deadline expired");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Live,
    Stopped,
    Zombie,
}

#[derive(Clone, Debug)]
struct Process {
    pid: i32,
    parent: i32,
    group: i32,
    identity: String,
    state: State,
}

fn observe(pid: i32) -> io::Result<Option<Process>> {
    #[cfg(target_os = "macos")]
    {
        let mut info = unsafe { std::mem::zeroed::<libc::proc_bsdinfo>() };
        let size = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                0,
                (&mut info as *mut libc::proc_bsdinfo).cast(),
                size_of_val(&info) as i32,
            )
        };
        if size == 0 && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
            return Ok(None);
        }
        if size as usize != size_of_val(&info) {
            return Err(io::Error::other("cannot inspect custody fixture process"));
        }
        Ok(Some(Process {
            pid,
            parent: info.pbi_ppid as i32,
            group: info.pbi_pgid as i32,
            identity: format!("{}:{}", info.pbi_start_tvsec, info.pbi_start_tvusec),
            state: match info.pbi_status {
                libc::SSTOP => State::Stopped,
                libc::SZOMB => State::Zombie,
                _ => State::Live,
            },
        }))
    }
    #[cfg(target_os = "linux")]
    {
        let stat = match fs::read(format!("/proc/{pid}/stat")) {
            Ok(stat) => stat,
            Err(error)
                if matches!(error.raw_os_error(), Some(libc::ENOENT) | Some(libc::ESRCH)) =>
            {
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        parse_linux_stat(pid, &stat).map(Some)
    }
}

fn parse_linux_stat(pid: i32, stat: &[u8]) -> io::Result<Process> {
    let invalid = || io::Error::other("invalid custody fixture process status");
    let end = stat
        .iter()
        .rposition(|byte| *byte == b')')
        .ok_or_else(invalid)?;
    let fields: Vec<_> = stat[end + 1..]
        .split(|byte| byte.is_ascii_whitespace())
        .filter(|field| !field.is_empty())
        .collect();
    let number = |index: usize| -> io::Result<i32> {
        std::str::from_utf8(fields.get(index).ok_or_else(invalid)?)
            .map_err(|_| invalid())?
            .parse()
            .map_err(|_| invalid())
    };
    Ok(Process {
        pid,
        parent: number(1)?,
        group: number(2)?,
        identity: std::str::from_utf8(fields.get(19).ok_or_else(invalid)?)
            .map_err(|_| invalid())?
            .to_owned(),
        state: match *fields.first().ok_or_else(invalid)? {
            b"T" | b"t" => State::Stopped,
            b"Z" | b"X" | b"x" => State::Zombie,
            _ => State::Live,
        },
    })
}

struct OwnedLeaf(Child);

impl Drop for OwnedLeaf {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

impl Process {
    fn record(pid: i32) -> Self {
        observe(pid).unwrap().expect("fixture process must be live")
    }

    fn alive(&self) -> io::Result<bool> {
        Ok(observe(self.pid)?.is_some_and(|current| {
            current.identity == self.identity && current.state != State::Zombie
        }))
    }
}

fn poll_until<T>(
    deadline: Instant,
    description: &str,
    mut poll: impl FnMut() -> io::Result<Option<T>>,
) -> io::Result<T> {
    loop {
        if let Some(value) = poll()? {
            return Ok(value);
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                description.to_owned(),
            ));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn signal_owned(child: &mut Child, signal: i32) -> io::Result<()> {
    if child.try_wait()?.is_some() {
        return Err(io::Error::other("owned fixture child already exited"));
    }
    if unsafe { libc::kill(child.id() as i32, signal) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn ordinary_child_disposition() -> io::Result<()> {
    // A stopped parent pins exited child PIDs only when exit leaves zombies;
    // SIG_IGN/SA_NOCLDWAIT instead permit automatic reaping (wait(2)).
    let mut action = unsafe { std::mem::zeroed::<libc::sigaction>() };
    if unsafe { libc::sigaction(libc::SIGCHLD, std::ptr::null(), &mut action) } < 0 {
        return Err(io::Error::last_os_error());
    }
    if action.sa_sigaction != libc::SIG_DFL || action.sa_flags & libc::SA_NOCLDWAIT != 0 {
        return Err(io::Error::other(
            "custody fixtures require zombie-preserving SIGCHLD defaults",
        ));
    }
    Ok(())
}

struct FrozenParent<'a> {
    child: &'a mut Child,
    process: Process,
}

impl<'a> FrozenParent<'a> {
    fn new(child: &'a mut Child, deadline: Instant) -> io::Result<Self> {
        Self::with_observer(child, deadline, observe)
    }

    fn with_observer(
        child: &'a mut Child,
        deadline: Instant,
        mut observe_stopped: impl FnMut(i32) -> io::Result<Option<Process>>,
    ) -> io::Result<Self> {
        let before = observe(child.id() as i32)?
            .ok_or_else(|| io::Error::other("custodian exited before fixture freeze"))?;
        let mut parent = Self {
            child,
            process: before,
        };
        signal_owned(parent.child, libc::SIGSTOP)?;
        parent.process = poll_until(deadline, "custodian did not stop", || {
            let current = observe_stopped(parent.process.pid)?
                .ok_or_else(|| io::Error::other("custodian disappeared while stopping"))?;
            if current.identity != parent.process.identity {
                return Err(io::Error::other("custodian identity changed"));
            }
            Ok((current.state == State::Stopped).then_some(current))
        })?;
        Ok(parent)
    }
}

impl Drop for FrozenParent<'_> {
    fn drop(&mut self) {
        let _ = signal_owned(self.child, libc::SIGCONT);
    }
}

#[derive(Clone)]
struct Row {
    pid: i32,
    parent: i32,
    zombie: bool,
}

fn rows(directory: &Path, deadline: Instant) -> io::Result<Vec<Row>> {
    let output = File::create(directory.join("processes.txt"))?;
    let mut child = OwnedLeaf(
        Command::new("/bin/ps")
            .args(["-axo", "pid=,ppid=,stat="])
            .stdout(output)
            .stderr(Stdio::null())
            .spawn()?,
    );
    let status = poll_until(deadline, "process observation did not finish", || {
        child.0.try_wait()
    })?;
    if !status.success() {
        return Err(io::Error::other(
            "cannot enumerate custody fixture children",
        ));
    }
    fs::read_to_string(directory.join("processes.txt"))?
        .lines()
        .map(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            if fields.len() != 3 {
                return Err(io::Error::other("invalid custody fixture child row"));
            }
            Ok(Row {
                pid: fields[0].parse().map_err(io::Error::other)?,
                parent: fields[1].parse().map_err(io::Error::other)?,
                zombie: fields[2].starts_with('Z'),
            })
        })
        .collect()
}

fn stopped(parent: &Process) -> io::Result<()> {
    let current = observe(parent.pid)?
        .ok_or_else(|| io::Error::other("frozen fixture parent disappeared"))?;
    if current.identity != parent.identity || current.state != State::Stopped {
        return Err(io::Error::other(
            "fixture parent is not the same stopped process",
        ));
    }
    Ok(())
}

fn pinned_child(
    directory: &Path,
    parent: &Process,
    pid: i32,
    identity: Option<&str>,
    deadline: Instant,
) -> io::Result<Option<Process>> {
    stopped(parent)?;
    let row = rows(directory, deadline)?
        .into_iter()
        .find(|row| row.pid == pid)
        .ok_or_else(|| io::Error::other("fixture child row disappeared"))?;
    if row.parent != parent.pid {
        return Err(io::Error::other(
            "fixture target is not a child of its frozen parent",
        ));
    }
    if row.zombie {
        stopped(parent)?;
        return Ok(None);
    }
    let current = observe(pid)?;
    if let Some(current) = current {
        if current.parent != parent.pid
            || identity.is_some_and(|identity| identity != current.identity)
        {
            return Err(io::Error::other("live fixture child identity changed"));
        }
        stopped(parent)?;
        return Ok(Some(current));
    }
    let dead = rows(directory, deadline)?
        .into_iter()
        .any(|row| row.pid == pid && row.parent == parent.pid && row.zombie);
    stopped(parent)?;
    if dead {
        Ok(None)
    } else {
        Err(io::Error::other("fixture child death is unknown"))
    }
}

fn signal_pinned(
    directory: &Path,
    parent: &Process,
    child: &Process,
    signal: i32,
    deadline: Instant,
) -> io::Result<()> {
    // The verified parent stays stopped across observation and signal, so it
    // cannot reap this child and permit PID reuse between those operations.
    if pinned_child(
        directory,
        parent,
        child.pid,
        Some(&child.identity),
        deadline,
    )?
    .is_some()
        && unsafe { libc::kill(child.pid, signal) } < 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn terminate_children(
    directory: &Path,
    parent: &Process,
    deadline: Instant,
    remaining: &mut usize,
) -> io::Result<()> {
    stopped(parent)?;
    for row in rows(directory, deadline)?
        .into_iter()
        .filter(|row| row.parent == parent.pid)
    {
        *remaining = remaining.checked_sub(1).ok_or_else(|| {
            io::Error::other("fixture process tree exceeds its bounded inventory")
        })?;
        let Some(child) = pinned_child(directory, parent, row.pid, None, deadline)? else {
            continue;
        };
        signal_pinned(directory, parent, &child, libc::SIGSTOP, deadline)?;
        let child = poll_until(deadline, "fixture child did not stop", || {
            Ok(
                match pinned_child(
                    directory,
                    parent,
                    child.pid,
                    Some(&child.identity),
                    deadline,
                )? {
                    None => Some(None),
                    Some(current) if current.state == State::Stopped => Some(Some(current)),
                    _ => None,
                },
            )
        })?;
        let Some(child) = child else { continue };
        terminate_children(directory, &child, deadline, remaining)?;
        signal_pinned(directory, parent, &child, libc::SIGKILL, deadline)?;
        poll_until(deadline, "fixture child did not exit", || {
            Ok(pinned_child(
                directory,
                parent,
                child.pid,
                Some(&child.identity),
                deadline,
            )?
            .is_none()
            .then_some(()))
        })?;
    }
    Ok(())
}

#[test]
fn selected_listener_endpoint_stays_reserved_until_custodian_handoff() {
    let directory = Directory::new();
    let reservation = super::ports::Reservation::new();
    let address = reservation.address();
    assert!(
        excluded(address),
        "selected endpoint became available before handoff"
    );
    let mut controller = Controller::start(&directory.0, reservation, 30000);
    assert!(excluded(address));
    controller.finish();
    assert!(!excluded(address));
}
fn excluded(address: SocketAddr) -> bool {
    match TcpListener::bind(address) {
        Ok(_) => false,
        Err(error) => {
            assert_eq!(error.kind(), std::io::ErrorKind::AddrInUse);
            true
        }
    }
}
fn os(value: impl AsRef<[u8]>) -> Value {
    json!({"Unix":value.as_ref()})
}
fn spec(command: &str) -> Value {
    json!({"argv":[os(b"/bin/sh"),os(b"-c"),os(command)],"cwd":os(b"/"),"environment":[],"umask":18,"caller_group":unsafe { libc::getpgrp() }})
}

struct Controller {
    stream: UnixStream,
    child: Child,
    directory: PathBuf,
    released: bool,
    payloads: Vec<Process>,
}
impl Controller {
    fn kill_anchor(&mut self, process: &Process) -> std::io::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(3);
        let parent = FrozenParent::new(&mut self.child, deadline)?;
        signal_pinned(
            &self.directory,
            &parent.process,
            process,
            libc::SIGKILL,
            deadline,
        )?;
        poll_until(deadline, "anchor did not exit", || {
            Ok(pinned_child(
                &self.directory,
                &parent.process,
                process.pid,
                Some(&process.identity),
                deadline,
            )?
            .is_none()
            .then_some(()))
        })?;
        Ok(())
    }

    fn start(
        directory: &Path,
        reservation: super::ports::Reservation,
        session_timeout_ms: u64,
    ) -> Self {
        reservation.handoff(|address| Self::start_at(directory, address, session_timeout_ms))
    }

    fn start_at(directory: &Path, address: SocketAddr, session_timeout_ms: u64) -> Self {
        ordinary_child_disposition().unwrap();
        let path = directory.join("custody.json");
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&path)
            .unwrap();
        serde_json::to_writer(&mut file, &json!({"resources":[address.to_string()],"session_timeout_ms":session_timeout_ms,"cleanup_timeout_ms":2000})).unwrap();
        file.flush().unwrap();
        let (stream, bootstrap) = UnixStream::pair().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(8)))
            .unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_mdrun"))
            .args(["__custodian", "--config"])
            .arg(path)
            .stdin(Stdio::from(OwnedFd::from(bootstrap)))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut controller = Self {
            stream,
            child,
            directory: directory.to_owned(),
            released: false,
            payloads: Vec::new(),
        };
        assert_eq!(controller.receive(), json!({"kind":"ready"}));
        controller
    }
    fn send(&mut self, message: Value, fds: &[i32]) {
        let data = serde_json::to_vec(&json!({"message":message,"descriptors":fds.len()})).unwrap();
        self.stream
            .write_all(&(data.len() as u32).to_be_bytes())
            .unwrap();
        self.stream.write_all(&data).unwrap();
        if fds.is_empty() {
            return;
        }
        let bytes = size_of_val(fds);
        let space = unsafe { libc::CMSG_SPACE(bytes as u32) } as usize;
        let mut ancillary = vec![0_usize; space.div_ceil(size_of::<usize>())];
        let mut marker = b'F';
        let mut iov = libc::iovec {
            iov_base: (&mut marker as *mut u8).cast(),
            iov_len: 1,
        };
        let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
        message.msg_iov = &mut iov;
        message.msg_iovlen = 1;
        message.msg_control = ancillary.as_mut_ptr().cast();
        message.msg_controllen = space as _;
        unsafe {
            let header = libc::CMSG_FIRSTHDR(&message);
            (*header).cmsg_level = libc::SOL_SOCKET;
            (*header).cmsg_type = libc::SCM_RIGHTS;
            (*header).cmsg_len = libc::CMSG_LEN(bytes as u32) as _;
            std::ptr::copy_nonoverlapping(fds.as_ptr(), libc::CMSG_DATA(header).cast(), fds.len());
            assert_eq!(libc::sendmsg(self.stream.as_raw_fd(), &message, 0), 1);
        }
    }
    fn receive(&mut self) -> Value {
        let mut prefix = [0; 4];
        self.stream.read_exact(&mut prefix).unwrap();
        let size = u32::from_be_bytes(prefix) as usize;
        assert!((1..=1048576).contains(&size));
        let mut bytes = vec![0; size];
        self.stream.read_exact(&mut bytes).unwrap();
        let packet: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(packet["descriptors"], 0);
        let message = packet["message"].clone();
        if message["kind"] == "stopped" && message["clean"] == true {
            self.released = true;
        }
        message
    }
    fn launch(&mut self, command: &str, output: &File) -> Value {
        self.launch_spec(spec(command), output)
    }
    fn launch_spec(&mut self, launch: Value, output: &File) -> Value {
        let input = File::open("/dev/null").unwrap();
        self.send(
            json!({"kind":"spawn","id":"payload","spec":launch}),
            &[input.as_raw_fd(), output.as_raw_fd(), output.as_raw_fd()],
        );
        let started = self.receive();
        assert_eq!(started["kind"], "started", "{started}");
        if let Some(process) = observe(started["pid"].as_i64().unwrap() as i32).unwrap() {
            self.payloads.push(process);
        }
        started
    }
    fn finish(&mut self) {
        self.send(json!({"kind":"stop"}), &[]);
        assert_eq!(self.receive(), json!({"kind":"stopped","clean":true}));
        assert!(self.child.wait().unwrap().success());
    }

    fn cleanup(&mut self) -> io::Result<()> {
        fs::write(self.directory.join("finished"), b"")?;
        let deadline = Instant::now() + Duration::from_secs(3);
        if self.released {
            poll_until(deadline, "released custodian did not exit", || {
                self.child.try_wait()
            })?;
            return Ok(());
        }
        if self.child.try_wait()?.is_some() {
            return Err(io::Error::other(
                "fixture parent exited without released custody",
            ));
        }
        let parent = FrozenParent::new(&mut self.child, deadline)?;
        let mut remaining = 32;
        terminate_children(&self.directory, &parent.process, deadline, &mut remaining)?;
        for payload in &self.payloads {
            poll_until(
                deadline,
                "cooperative fixture payload survived cleanup",
                || Ok((!payload.alive()?).then_some(())),
            )?;
        }
        drop(parent);
        if self.child.try_wait()?.is_none() {
            self.child.kill()?;
        }
        poll_until(deadline, "fixture custodian did not exit", || {
            self.child.try_wait()
        })?;
        Ok(())
    }
}
impl Drop for Controller {
    fn drop(&mut self) {
        if let Err(error) = self.cleanup() {
            let _ = fs::write(self.directory.join("cleanup.failure"), error.to_string());
            eprintln!("custody fixture cleanup failed: {error}");
            if !std::thread::panicking() {
                panic!("custody fixture cleanup failed: {error}");
            }
        }
    }
}

#[test]
fn failed_stop_observation_resumes_the_directly_owned_child() {
    ordinary_child_disposition().unwrap();
    let mut child = OwnedLeaf(
        Command::new("/bin/sleep")
            .arg("15")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(1);
    let mut injected = false;
    let result = FrozenParent::with_observer(&mut child.0, deadline, |pid| {
        poll_until(
            deadline,
            "owned child did not stop before injection",
            || Ok(observe(pid)?.filter(|process| process.state == State::Stopped)),
        )?;
        injected = true;
        Err(io::Error::other("injected stop observation failure"))
    });
    assert!(result.is_err());
    drop(result);
    assert!(injected);
    poll_until(
        deadline,
        "owned child remained stopped after observation failure",
        || Ok(observe(child.0.id() as i32)?.filter(|process| process.state == State::Live)),
    )
    .unwrap();
    assert!(child.0.try_wait().unwrap().is_none());
}

#[test]
fn cleanup_authority_rejects_an_unrelated_owned_child() {
    let directory = Directory::new();
    let reservation = super::ports::Reservation::new();
    let mut controller = Controller::start(&directory.0, reservation, 30000);
    let mut sibling = OwnedLeaf(
        Command::new("/bin/sleep")
            .arg("15")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let process = Process::record(sibling.0.id() as i32);
    assert!(controller.kill_anchor(&process).is_err());
    assert!(sibling.0.try_wait().unwrap().is_none());
    controller.finish();
}

#[test]
fn exact_merged_descriptors_and_direct_status_precede_observed_release() {
    let directory = Directory::new();
    let reservation = super::ports::Reservation::new();
    let address = reservation.address();
    let mut controller = Controller::start(&directory.0, reservation, 30000);
    let output = File::create(directory.0.join("output")).unwrap();
    controller.launch("printf stdout; printf stderr >&2", &output);
    assert_eq!(
        controller.receive(),
        json!({"kind":"exited","id":"payload","status":{"code":0,"signal":null}})
    );
    assert_eq!(
        controller.receive(),
        json!({"kind":"clean","id":"payload","forced":false})
    );
    assert_eq!(
        fs::read(directory.0.join("output")).unwrap(),
        b"stdoutstderr"
    );
    assert!(excluded(address));
    controller.finish();
    assert!(!excluded(address));
}

#[test]
fn unexpected_anchor_loss_retains_exclusion_and_reports_uncertainty() {
    use std::os::unix::ffi::OsStrExt;
    let directory = Directory::new();
    let reservation = super::ports::Reservation::new();
    let address = reservation.address();
    let mut controller = Controller::start(&directory.0, reservation, 30000);
    let output = File::create(directory.0.join("output")).unwrap();
    let executable = std::env::current_exe().unwrap();
    let mut launch = spec("");
    launch["argv"] = json!([
        os(executable.as_os_str().as_bytes()),
        os("--exact"),
        os("custody::cooperative_payload_fixture"),
        os("--nocapture"),
        os("--test-threads=1")
    ]);
    launch["environment"] = json!([[
        os("MDRUN_CUSTODY_PAYLOAD_DIRECTORY"),
        os(directory.0.as_os_str().as_bytes())
    ]]);
    let started = controller.launch_spec(launch, &output);
    wait_until(|| directory.0.join("ready").exists());
    let payload = Process::record(started["pid"].as_i64().unwrap() as i32);
    let anchor = Process::record(started["group"].as_i64().unwrap() as i32);
    assert_eq!(payload.parent, anchor.pid);
    assert_eq!(payload.group, anchor.group);
    controller.kill_anchor(&anchor).unwrap();
    assert_eq!(controller.receive()["kind"], "uncertain");
    assert!(payload.alive().unwrap());
    assert!(excluded(address));
    drop(controller);
    assert!(!payload.alive().unwrap());
    assert!(!excluded(address));
    assert!(!directory.0.join("expired").exists());
}

#[test]
fn cooperative_payload_fixture() {
    let Some(directory) = std::env::var_os("MDRUN_CUSTODY_PAYLOAD_DIRECTORY") else {
        return;
    };
    let directory = PathBuf::from(directory);
    assert_ne!(
        unsafe { libc::signal(libc::SIGTERM, libc::SIG_IGN) },
        libc::SIG_ERR
    );
    fs::write(directory.join("ready"), b"").unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while !directory.join("finished").exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    if Instant::now() >= deadline {
        fs::write(directory.join("expired"), b"").unwrap();
    }
}

#[test]
fn watchdog_failure_is_distinct_from_successful_cleanup() {
    let directory = Directory::new();
    let reservation = super::ports::Reservation::new();
    let address = reservation.address();
    let mut controller = Controller::start(&directory.0, reservation, 200);
    assert_eq!(
        controller.receive(),
        json!({"kind":"fault","reason":"session deadline expired"})
    );
    assert_eq!(controller.receive(), json!({"kind":"stopped","clean":true}));
    assert!(!controller.child.wait().unwrap().success());
    assert!(!excluded(address));
}

#[test]
fn invalid_group_control_cannot_be_reported_as_success() {
    let directory = Directory::new();
    let reservation = super::ports::Reservation::new();
    let address = reservation.address();
    let mut controller = Controller::start(&directory.0, reservation, 30000);
    controller.send(json!({"kind":"stop_group","id":"unknown"}), &[]);
    assert_eq!(
        controller.receive(),
        json!({"kind":"fault","reason":"unknown owned group"})
    );
    assert_eq!(controller.receive(), json!({"kind":"stopped","clean":true}));
    assert!(!controller.child.wait().unwrap().success());
    assert!(!excluded(address));
}

#[test]
fn interruption_before_the_payload_grant_executes_nothing() {
    ordinary_child_disposition().unwrap();
    let directory = Directory::new();
    let effect = directory.0.join("effect");
    let (stream, bootstrap) = UnixStream::pair().unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(8)))
        .unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_mdrun"))
        .arg("__anchor")
        .stdin(Stdio::from(OwnedFd::from(bootstrap)))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut controller = Controller {
        stream,
        child,
        directory: directory.0.clone(),
        released: false,
        payloads: Vec::new(),
    };
    let descriptor = File::open("/dev/null").unwrap();
    controller.send(json!({"kind":"launch","spec":spec(&format!("printf executed > '{}'",effect.display())),"resources":0,"cleanup_timeout_ms":2000}), &[descriptor.as_raw_fd();3]);
    assert_eq!(controller.receive(), json!({"kind":"ready"}));
    signal_owned(&mut controller.child, libc::SIGINT).unwrap();
    controller.send(json!({"kind":"grant"}), &[]);
    let mut events = Vec::new();
    loop {
        let event = controller.receive();
        let sweep = event["kind"] == "sweep";
        events.push(event);
        if sweep {
            break;
        }
    }
    controller.send(json!({"kind":"sweep_ack"}), &[]);
    assert_eq!(
        controller.child.wait().unwrap().signal(),
        Some(libc::SIGKILL)
    );
    controller.released = true;
    assert!(
        !effect.exists(),
        "interrupted launch performed its durable effect"
    );
    assert!(
        events.iter().any(|event| event["kind"] == "interrupted"),
        "{events:?}"
    );
    assert!(
        !events.iter().any(|event| event["kind"] == "started"),
        "{events:?}"
    );
}

#[test]
fn a_failed_payload_spawn_releases_custody_after_observing_the_empty_group() {
    let directory = Directory::new();
    let reservation = super::ports::Reservation::new();
    let address = reservation.address();
    let mut controller = Controller::start(&directory.0, reservation, 30000);
    let descriptor = File::open("/dev/null").unwrap();
    let mut launch = spec("");
    launch["argv"] = json!([os(format!("/mdrun-missing-{}", uuid::Uuid::new_v4()))]);
    controller.send(
        json!({"kind":"spawn","id":"payload","spec":launch}),
        &[descriptor.as_raw_fd(); 3],
    );
    let event = controller.receive();
    let state_path = directory.0.join("custody.status.json");
    wait_until(|| {
        fs::read(&state_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .is_some_and(|state| {
                state["groups"]
                    .as_array()
                    .is_some_and(|groups| !groups.is_empty())
            })
    });
    let state: Value = serde_json::from_slice(&fs::read(state_path).unwrap()).unwrap();
    assert!(!state["groups"].as_array().unwrap().is_empty());
    assert_eq!(
        event,
        json!({"kind":"fault","reason":"payload could not be started"})
    );
    assert_eq!(
        controller.receive(),
        json!({"kind":"clean","id":"payload","forced":false})
    );
    assert_eq!(controller.receive(), json!({"kind":"stopped","clean":true}));
    assert!(!controller.child.wait().unwrap().success());
    assert!(!excluded(address));
}

#[test]
fn linux_fixture_status_retains_parent_group_birth_and_native_comm_bytes() {
    let observed = parse_linux_stat(
        42,
        b"42 (native ) name-\xff) T 41 99 77 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 12345\n",
    )
    .unwrap();
    assert_eq!(
        (observed.pid, observed.parent, observed.group),
        (42, 41, 99)
    );
    assert_eq!(observed.identity, "12345");
    assert_eq!(observed.state, State::Stopped);
    for truncated in [
        b"42 no terminator".as_slice(),
        b"42 (name) T 41",
        b"42 (name) T not-a-parent 99",
    ] {
        assert!(parse_linux_stat(42, truncated).is_err());
    }
}
