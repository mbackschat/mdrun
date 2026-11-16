use super::{platform, wire};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::collections::VecDeque;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::net::{SocketAddr, TcpListener};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const MAX_DESCRIPTORS: usize = 128;
const CONTROL_TIMEOUT: Duration = Duration::from_secs(5);
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustodyConfig {
    pub resources: Vec<SocketAddr>,
    pub session_timeout_ms: u64,
    pub cleanup_timeout_ms: u64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpawnSpec {
    pub argv: Vec<OsString>,
    #[serde(with = "super::context::native_path")]
    pub cwd: PathBuf,
    pub environment: Vec<(OsString, OsString)>,
    pub umask: u32,
    pub caller_group: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PayloadStatus {
    pub code: Option<i32>,
    pub signal: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CustodyEvent {
    Ready,
    Started { id: String, pid: u32, group: i32 },
    Exited { id: String, status: PayloadStatus },
    Clean { id: String, forced: bool },
    Uncertain { id: Option<String>, reason: String },
    Fault { reason: String },
    Stopped { clean: bool },
    ScopeRegistered { scope: String },
    ScopeRejected { scope: String, reason: String },
    ScopeEvent { scope: String, event: GroupEvent },
    ScopeStopped { scope: String, clean: bool },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GroupEvent {
    Started { id: String, pid: u32, group: i32 },
    Exited { id: String, status: PayloadStatus },
    Clean { id: String, forced: bool },
    Uncertain { id: Option<String>, reason: String },
    Fault { reason: String },
    Rejected { id: String, reason: String },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Packet<T> {
    message: T,
    descriptors: usize,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum OwnerRequest {
    Spawn {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scope: Option<String>,
        id: String,
        spec: SpawnSpec,
    },
    StopGroup {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scope: Option<String>,
        id: String,
    },
    RegisterScope {
        scope: String,
        session_timeout_ms: u64,
        cleanup_timeout_ms: u64,
    },
    StopScope {
        scope: String,
    },
    Stop,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum AnchorRequest {
    Launch {
        spec: SpawnSpec,
        resources: usize,
        cleanup_timeout_ms: u64,
    },
    Grant,
    Stop,
    SweepAck,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum AnchorEvent {
    Ready,
    Started { pid: u32 },
    LaunchFailed,
    Interrupted,
    Exited { status: PayloadStatus },
    Sweep { forced: bool },
    Uncertain { reason: String },
}

pub struct Custody {
    control: UnixStream,
    child: Child,
    pending: VecDeque<CustodyEvent>,
    status_path: PathBuf,
}

impl Custody {
    pub fn start(
        executable: &Path,
        private_config_path: &Path,
        config: &CustodyConfig,
    ) -> io::Result<Self> {
        validate_config(config)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(private_config_path)?;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        file.write_all(&serde_json::to_vec(config).map_err(invalid_data)?)?;
        let (mut control, child_control) = UnixStream::pair()?;
        configure_control(&control)?;
        let mut child = Command::new(executable)
            .arg("__custodian")
            .arg("--config")
            .arg(private_config_path)
            .stdin(Stdio::from(OwnedFd::from(child_control)))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let startup = (|| -> io::Result<()> {
            let ready: CustodyEvent = receive_packet(&mut control)?
                .ok_or_else(|| io::Error::other("custodian exited during startup"))?
                .0;
            if ready != CustodyEvent::Ready {
                return Err(io::Error::other(
                    "custodian could not establish resource custody",
                ));
            }
            Ok(())
        })();
        if let Err(error) = startup {
            // No owner request can precede Ready, so this child has no payload groups.
            // Reap failed startup before callers discard its private status directory.
            drop(control);
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        Ok(Self {
            control,
            child,
            pending: VecDeque::new(),
            status_path: private_config_path.with_extension("status.json"),
        })
    }

    pub fn spawn(
        &mut self,
        id: &str,
        spec: &SpawnSpec,
        stdio: [BorrowedFd<'_>; 3],
    ) -> io::Result<CustodyEvent> {
        validate_spawn(id, spec)?;
        send_packet(
            &mut self.control,
            &OwnerRequest::Spawn {
                scope: None,
                id: id.to_owned(),
                spec: spec.clone(),
            },
            &stdio,
        )?;
        loop {
            let event: CustodyEvent = receive_packet(&mut self.control)?
                .ok_or_else(|| io::Error::other("custodian lost during launch"))?
                .0;
            match &event {
                CustodyEvent::Started { id: started, .. } if started == id => return Ok(event),
                CustodyEvent::Uncertain { .. }
                | CustodyEvent::Fault { .. }
                | CustodyEvent::Stopped { .. } => {
                    self.pending.push_back(event);
                    return Err(io::Error::other("payload launch custody is uncertain"));
                }
                _ => self.pending.push_back(event),
            }
        }
    }

    pub fn poll_event(&mut self, timeout: Duration) -> io::Result<Option<CustodyEvent>> {
        if let Some(event) = self.pending.pop_front() {
            return Ok(Some(event));
        }
        if !readable(&self.control, timeout)? {
            return Ok(None);
        }
        let (event, descriptors) = receive_packet(&mut self.control)?
            .ok_or_else(|| io::Error::other("custodian control closed"))?;
        if !descriptors.is_empty() {
            return Err(io::Error::other("unexpected custodian descriptors"));
        }
        if matches!(event, CustodyEvent::Stopped { .. }) {
            self.child.wait()?;
        }
        Ok(Some(event))
    }

    pub fn stop(&mut self) -> io::Result<()> {
        send_packet(&mut self.control, &OwnerRequest::Stop, &[])
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    pub fn status_path(&self) -> &Path {
        &self.status_path
    }

    pub(super) fn request(
        &mut self,
        request: &OwnerRequest,
        descriptors: &[BorrowedFd<'_>],
    ) -> io::Result<()> {
        send_packet(&mut self.control, request, descriptors)
    }
}

fn validate_config(config: &CustodyConfig) -> io::Result<()> {
    if config.session_timeout_ms == 0
        || config.cleanup_timeout_ms == 0
        || config.resources.len() > MAX_DESCRIPTORS - 3
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid custody bounds",
        ));
    }
    for (index, endpoint) in config.resources.iter().enumerate() {
        if !endpoint.ip().is_loopback()
            || endpoint.port() == 0
            || config.resources[..index].contains(endpoint)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid custody resource endpoint",
            ));
        }
    }
    Ok(())
}

fn validate_spawn(id: &str, spec: &SpawnSpec) -> io::Result<()> {
    if id.is_empty()
        || id.len() > 128
        || spec.argv.is_empty()
        || spec.argv[0].is_empty()
        || !spec.cwd.is_absolute()
        || spec.umask > 0o777
        || spec.caller_group <= 0
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid custody payload",
        ));
    }
    Ok(())
}

fn invalid_data(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

fn configure_control(control: &UnixStream) -> io::Result<()> {
    control.set_read_timeout(Some(CONTROL_TIMEOUT))?;
    control.set_write_timeout(Some(CONTROL_TIMEOUT))
}

fn send_packet<T: Serialize>(
    control: &mut UnixStream,
    message: &T,
    descriptors: &[BorrowedFd<'_>],
) -> io::Result<()> {
    wire::write_message(
        control,
        &Packet {
            message,
            descriptors: descriptors.len(),
        },
    )
    .map_err(invalid_data)?;
    if !descriptors.is_empty() {
        send_fds(control, descriptors)?;
    }
    Ok(())
}

fn receive_packet<T: DeserializeOwned>(
    control: &mut UnixStream,
) -> io::Result<Option<(T, Vec<OwnedFd>)>> {
    let Some(packet) = wire::read_message::<_, Packet<T>>(control).map_err(invalid_data)? else {
        return Ok(None);
    };
    let descriptors = if packet.descriptors == 0 {
        Vec::new()
    } else {
        receive_fds(control, packet.descriptors)?
    };
    Ok(Some((packet.message, descriptors)))
}

/// Transfers descriptor references without changing their shared open-file-description flags.
/// The caller frames its metadata separately and the receiver consumes exactly one ancillary marker.
pub fn send_fds(stream: &UnixStream, descriptors: &[BorrowedFd<'_>]) -> io::Result<()> {
    if descriptors.is_empty() || descriptors.len() > MAX_DESCRIPTORS {
        return Err(invalid_data("invalid descriptor count"));
    }
    let descriptor_bytes = descriptors.len() * size_of::<libc::c_int>();
    let space = unsafe { libc::CMSG_SPACE(descriptor_bytes as u32) } as usize;
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
        (*header).cmsg_len = libc::CMSG_LEN(descriptor_bytes as u32) as _;
        let values = libc::CMSG_DATA(header).cast::<libc::c_int>();
        for (index, descriptor) in descriptors.iter().enumerate() {
            values.add(index).write(descriptor.as_raw_fd());
        }
    }
    loop {
        #[cfg(target_os = "linux")]
        let flags = libc::MSG_NOSIGNAL;
        #[cfg(target_os = "macos")]
        let flags = 0;
        let sent = unsafe { libc::sendmsg(stream.as_raw_fd(), &message, flags) };
        if sent == 1 {
            return Ok(());
        }
        if sent < 0 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
            continue;
        }
        return Err(io::Error::last_os_error());
    }
}

/// Receives the expected references and marks each close-on-exec before returning it.
/// On macOS the caller must not concurrently spawn processes during this operation,
/// because recvmsg has no MSG_CMSG_CLOEXEC equivalent.
pub fn receive_fds(stream: &UnixStream, expected: usize) -> io::Result<Vec<OwnedFd>> {
    if expected == 0 || expected > MAX_DESCRIPTORS {
        return Err(invalid_data("invalid descriptor count"));
    }
    let space =
        unsafe { libc::CMSG_SPACE((MAX_DESCRIPTORS * size_of::<libc::c_int>()) as u32) } as usize;
    let mut ancillary = vec![0_usize; space.div_ceil(size_of::<usize>())];
    let mut marker = 0_u8;
    let mut iov = libc::iovec {
        iov_base: (&mut marker as *mut u8).cast(),
        iov_len: 1,
    };
    let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
    message.msg_iov = &mut iov;
    message.msg_iovlen = 1;
    message.msg_control = ancillary.as_mut_ptr().cast();
    message.msg_controllen = space as _;
    let received = loop {
        #[cfg(target_os = "linux")]
        let flags = libc::MSG_CMSG_CLOEXEC;
        #[cfg(target_os = "macos")]
        let flags = 0;
        let received = unsafe { libc::recvmsg(stream.as_raw_fd(), &mut message, flags) };
        if received < 0 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
            continue;
        }
        break received;
    };
    if received < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut descriptors = Vec::new();
    unsafe {
        let mut header = libc::CMSG_FIRSTHDR(&message);
        while !header.is_null() {
            if (*header).cmsg_level == libc::SOL_SOCKET && (*header).cmsg_type == libc::SCM_RIGHTS {
                let length = (*header).cmsg_len as usize;
                let base = libc::CMSG_LEN(0) as usize;
                if length < base || !(length - base).is_multiple_of(size_of::<libc::c_int>()) {
                    return Err(invalid_data("invalid ancillary descriptor length"));
                }
                let count = (length - base) / size_of::<libc::c_int>();
                let values = libc::CMSG_DATA(header).cast::<libc::c_int>();
                for index in 0..count {
                    descriptors.push(OwnedFd::from_raw_fd(values.add(index).read()));
                }
            }
            header = libc::CMSG_NXTHDR(&message, header);
        }
    }
    if received != 1
        || marker != b'F'
        || message.msg_flags & libc::MSG_CTRUNC != 0
        || descriptors.len() != expected
    {
        return Err(invalid_data("invalid ancillary descriptor handoff"));
    }
    for descriptor in &descriptors {
        set_cloexec(descriptor.as_raw_fd())?;
    }
    Ok(descriptors)
}

pub(super) fn set_cloexec(fd: i32) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn readable(stream: &UnixStream, timeout: Duration) -> io::Result<bool> {
    let mut entry = libc::pollfd {
        fd: stream.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    let result = unsafe {
        libc::poll(
            &mut entry,
            1,
            timeout.as_millis().min(i32::MAX as u128) as i32,
        )
    };
    if result < 0 {
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::Interrupted {
            return Ok(false);
        }
        return Err(error);
    }
    Ok(result != 0)
}

fn bootstrap_control() -> io::Result<UnixStream> {
    let stream = unsafe { UnixStream::from_raw_fd(libc::STDIN_FILENO) };
    set_cloexec(stream.as_raw_fd())?;
    configure_control(&stream)?;
    Ok(stream)
}

extern "C" fn interrupted(_signal: libc::c_int) {
    INTERRUPTED.store(true, Ordering::Relaxed);
}

fn install_handlers() -> io::Result<()> {
    for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP, libc::SIGQUIT] {
        let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
        action.sa_sigaction = interrupted as *const () as usize;
        unsafe {
            libc::sigemptyset(&mut action.sa_mask);
        }
        if unsafe { libc::sigaction(signal, &action, std::ptr::null_mut()) } < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

struct TerminalRestore {
    descriptor: OwnedFd,
    previous: i32,
    owned: i32,
}

impl TerminalRestore {
    fn capture(stdio: &[OwnedFd], caller_group: i32, owned: i32) -> io::Result<Option<Self>> {
        for descriptor in stdio {
            if unsafe { libc::isatty(descriptor.as_raw_fd()) } == 0 {
                if io::Error::last_os_error().raw_os_error() == Some(libc::EBADF) {
                    return Err(io::Error::last_os_error());
                }
                continue;
            }
            let foreground = unsafe { libc::tcgetpgrp(descriptor.as_raw_fd()) };
            if foreground < 0 {
                if io::Error::last_os_error().raw_os_error() == Some(libc::ENOTTY) {
                    continue;
                }
                return Err(io::Error::last_os_error());
            }
            if foreground != caller_group {
                return Err(io::Error::other(
                    "background terminal launch is not qualified",
                ));
            }
            return Ok(Some(Self {
                descriptor: descriptor.try_clone()?,
                previous: foreground,
                owned,
            }));
        }
        Ok(None)
    }

    fn grant(&self) -> io::Result<()> {
        foreground(self.descriptor.as_raw_fd(), self.owned)
    }

    fn restore(&self) -> io::Result<()> {
        let current = unsafe { libc::tcgetpgrp(self.descriptor.as_raw_fd()) };
        if current < 0 {
            return Err(io::Error::last_os_error());
        }
        // The initiating shell may already have reclaimed the terminal after owner loss.
        if current == self.owned {
            foreground(self.descriptor.as_raw_fd(), self.previous)?;
        }
        Ok(())
    }
}

fn foreground(descriptor: i32, group: i32) -> io::Result<()> {
    let mut blocked: libc::sigset_t = unsafe { std::mem::zeroed() };
    let mut original: libc::sigset_t = unsafe { std::mem::zeroed() };
    unsafe {
        libc::sigemptyset(&mut blocked);
        libc::sigaddset(&mut blocked, libc::SIGTTOU);
    }
    if unsafe { libc::sigprocmask(libc::SIG_BLOCK, &blocked, &mut original) } < 0 {
        return Err(io::Error::last_os_error());
    }
    let result = unsafe { libc::tcsetpgrp(descriptor, group) };
    let error = io::Error::last_os_error();
    let restored = unsafe { libc::sigprocmask(libc::SIG_SETMASK, &original, std::ptr::null_mut()) };
    if restored < 0 {
        return Err(io::Error::last_os_error());
    }
    if result < 0 {
        return Err(error);
    }
    Ok(())
}

struct Group {
    key: GroupKey,
    child: Child,
    control: UnixStream,
    terminal: Option<TerminalRestore>,
    sweep: Sweep,
    forced: bool,
    done: bool,
    uncertainty_reason: Option<String>,
    control_closed: bool,
    stopping: Option<Instant>,
    cleanup_timeout: Duration,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct GroupKey {
    pub scope: Option<String>,
    pub id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ScopePhase {
    Running,
    Stopping,
    Stopped,
}

struct Scope {
    id: String,
    phase: ScopePhase,
    started: Instant,
    session_timeout: Duration,
    cleanup_timeout: Duration,
    failed: bool,
}

/// Flattens a group event for an unscoped consumer, which has no admission phase and treats rejection as a fault.
impl From<GroupEvent> for CustodyEvent {
    fn from(event: GroupEvent) -> Self {
        match event {
            GroupEvent::Started { id, pid, group } => CustodyEvent::Started { id, pid, group },
            GroupEvent::Exited { id, status } => CustodyEvent::Exited { id, status },
            GroupEvent::Clean { id, forced } => CustodyEvent::Clean { id, forced },
            GroupEvent::Uncertain { id, reason } => CustodyEvent::Uncertain { id, reason },
            GroupEvent::Fault { reason } | GroupEvent::Rejected { reason, .. } => {
                CustodyEvent::Fault { reason }
            }
        }
    }
}

fn emit_group(owner: &mut UnixStream, owner_alive: &mut bool, key: &GroupKey, event: GroupEvent) {
    let event = match &key.scope {
        Some(scope) => CustodyEvent::ScopeEvent {
            scope: scope.clone(),
            event,
        },
        None => event.into(),
    };
    emit(owner, owner_alive, &event);
}

fn fail_scope(scopes: &mut [Scope], key: &GroupKey, admission_failed: &mut bool) {
    if let Some(scope) = key
        .scope
        .as_ref()
        .and_then(|id| scopes.iter_mut().find(|scope| &scope.id == id))
    {
        scope.failed = true;
        scope.phase = ScopePhase::Stopping;
    } else {
        *admission_failed = true;
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Sweep {
    NotRequested,
    Pending,
    Acknowledged,
}

enum LaunchOutcome {
    Granted,
    TerminalRejected(io::Error),
}

fn emit(owner: &mut UnixStream, owner_alive: &mut bool, event: &CustodyEvent) {
    if *owner_alive && send_packet(owner, event, &[]).is_err() {
        *owner_alive = false;
    }
}

fn uncertain(group: &mut Group, owner: &mut UnixStream, owner_alive: &mut bool, reason: &str) {
    if group.uncertainty_reason.is_none() {
        group.uncertainty_reason = Some(reason.to_owned());
        emit_group(
            owner,
            owner_alive,
            &group.key,
            GroupEvent::Uncertain {
                id: Some(group.key.id.clone()),
                reason: reason.to_owned(),
            },
        );
    }
}

fn stop_group(group: &mut Group) {
    if !group.done && group.stopping.is_none() {
        group.stopping = Some(Instant::now());
        let _ = send_packet(&mut group.control, &AnchorRequest::Stop, &[]);
    }
}

fn save_state(path: &Path, groups: &[Group], scopes: &[Scope], status: &str) -> io::Result<()> {
    let groups: Vec<_> = groups.iter().map(|group| {
        let mut state = serde_json::json!({"id":group.key.id,"anchor_pid":group.child.id(),"clean":group.done,"uncertain":group.uncertainty_reason.is_some(),"sweep":group.sweep});
        if let Some(scope) = &group.key.scope { state["scope"] = serde_json::json!(scope); }
        if let Some(reason) = &group.uncertainty_reason { state["uncertainty_reason"] = serde_json::json!(reason); }
        state
    }).collect();
    let scopes: Vec<_> = scopes
        .iter()
        .map(|scope| serde_json::json!({"id":scope.id,"phase":scope.phase,"failed":scope.failed}))
        .collect();
    let mut state =
        serde_json::json!({"custodian_pid":std::process::id(),"status":status,"groups":groups});
    if !scopes.is_empty() {
        state["scopes"] = serde_json::json!(scopes);
    }
    let temporary = path.with_extension("status.tmp");
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&temporary)?;
    let result = (|| {
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        file.write_all(&serde_json::to_vec(&state).map_err(invalid_data)?)?;
        fs::rename(&temporary, path.with_extension("status.json"))
    })();
    drop(file);
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

pub fn run_custodian(private_config_path: &Path) -> io::Result<i32> {
    install_handlers()?;
    let mut owner = bootstrap_control()?;
    let metadata = fs::metadata(private_config_path)?;
    if metadata.mode() & 0o077 != 0 || metadata.uid() != unsafe { libc::geteuid() } {
        return Err(io::Error::other("custody configuration is not private"));
    }
    let config: CustodyConfig =
        serde_json::from_reader(File::open(private_config_path)?).map_err(invalid_data)?;
    validate_config(&config)?;
    let mut listeners = Vec::new();
    for endpoint in &config.resources {
        match TcpListener::bind(endpoint) {
            Ok(listener) => listeners.push(listener),
            Err(_) => {
                send_packet(
                    &mut owner,
                    &CustodyEvent::Uncertain {
                        id: None,
                        reason: "resource exclusion unavailable".to_owned(),
                    },
                    &[],
                )?;
                return Ok(1);
            }
        }
    }
    let executable = std::env::current_exe()?;
    let mut groups: Vec<Group> = Vec::new();
    let mut scopes: Vec<Scope> = Vec::new();
    let mut owner_alive = true;
    let mut stopping = false;
    let mut admission_failed = false;
    let mut last_state = None;
    let started = Instant::now();
    save_state(private_config_path, &groups, &scopes, "ready")?;
    send_packet(&mut owner, &CustodyEvent::Ready, &[])?;
    loop {
        let owner_readable = owner_alive && readable(&owner, Duration::from_millis(10))?;
        for scope in &mut scopes {
            if scope.phase == ScopePhase::Running
                && scope.started.elapsed() >= scope.session_timeout
            {
                scope.failed = true;
                scope.phase = ScopePhase::Stopping;
                emit(
                    &mut owner,
                    &mut owner_alive,
                    &CustodyEvent::ScopeEvent {
                        scope: scope.id.clone(),
                        event: GroupEvent::Fault {
                            reason: "scope deadline expired".to_owned(),
                        },
                    },
                );
            }
        }
        if owner_readable {
            match receive_packet::<OwnerRequest>(&mut owner) {
                Ok(Some((
                    OwnerRequest::RegisterScope {
                        scope,
                        session_timeout_ms,
                        cleanup_timeout_ms,
                    },
                    descriptors,
                ))) if !stopping && !admission_failed => {
                    if !descriptors.is_empty()
                        || scope.is_empty()
                        || scope.len() > 128
                        || session_timeout_ms == 0
                        || cleanup_timeout_ms == 0
                        || scopes.iter().any(|registered| registered.id == scope)
                        || (scopes.is_empty() && !groups.is_empty())
                    {
                        admission_failed = true;
                        emit(
                            &mut owner,
                            &mut owner_alive,
                            &CustodyEvent::Fault {
                                reason: "invalid custody scope registration".to_owned(),
                            },
                        );
                    } else if scopes.iter().any(|scope| scope.failed) {
                        emit(
                            &mut owner,
                            &mut owner_alive,
                            &CustodyEvent::ScopeRejected {
                                scope,
                                reason: "scope admission closed".to_owned(),
                            },
                        );
                    } else {
                        scopes.push(Scope {
                            id: scope.clone(),
                            phase: ScopePhase::Running,
                            started: Instant::now(),
                            session_timeout: Duration::from_millis(session_timeout_ms),
                            cleanup_timeout: Duration::from_millis(cleanup_timeout_ms),
                            failed: false,
                        });
                        emit(
                            &mut owner,
                            &mut owner_alive,
                            &CustodyEvent::ScopeRegistered { scope },
                        );
                    }
                }
                Ok(Some((OwnerRequest::StopScope { scope }, descriptors)))
                    if descriptors.is_empty() =>
                {
                    if let Some(scope) = scopes.iter_mut().find(|registered| registered.id == scope)
                    {
                        if scope.phase == ScopePhase::Running {
                            scope.phase = ScopePhase::Stopping;
                        }
                    } else {
                        admission_failed = true;
                        emit(
                            &mut owner,
                            &mut owner_alive,
                            &CustodyEvent::Fault {
                                reason: "unknown custody scope".to_owned(),
                            },
                        );
                    }
                }
                Ok(Some((OwnerRequest::Spawn { scope, id, spec }, descriptors)))
                    if !stopping && !admission_failed =>
                {
                    let key = GroupKey { scope, id };
                    let registered = key
                        .scope
                        .as_ref()
                        .and_then(|id| scopes.iter().find(|scope| &scope.id == id));
                    let invalid_scope = if key.scope.is_some() {
                        registered.is_none()
                    } else {
                        !scopes.is_empty()
                    };
                    let cleanup_timeout = registered
                        .map_or(Duration::from_millis(config.cleanup_timeout_ms), |scope| {
                            scope.cleanup_timeout
                        });
                    if validate_spawn(&key.id, &spec).is_err()
                        || descriptors.len() != 3
                        || invalid_scope
                    {
                        admission_failed = true;
                        emit(
                            &mut owner,
                            &mut owner_alive,
                            &CustodyEvent::Uncertain {
                                id: Some(key.id.clone()),
                                reason: "invalid custody launch request".to_owned(),
                            },
                        );
                    } else if registered.is_some_and(|scope| scope.phase != ScopePhase::Running) {
                        emit_group(
                            &mut owner,
                            &mut owner_alive,
                            &key,
                            GroupEvent::Rejected {
                                id: key.id.clone(),
                                reason: "scope admission closed".to_owned(),
                            },
                        );
                    } else if groups.iter().any(|group| group.key == key) {
                        admission_failed = true;
                        emit(
                            &mut owner,
                            &mut owner_alive,
                            &CustodyEvent::Fault {
                                reason: "duplicate owned group".to_owned(),
                            },
                        );
                    } else {
                        let (control, child_control) = UnixStream::pair()?;
                        configure_control(&control)?;
                        let child = Command::new(&executable)
                            .arg("__anchor")
                            .stdin(Stdio::from(OwnedFd::from(child_control)))
                            .stdout(Stdio::null())
                            .stderr(Stdio::null())
                            .spawn()?;
                        let mut group = Group {
                            key,
                            child,
                            control,
                            terminal: None,
                            sweep: Sweep::NotRequested,
                            forced: false,
                            done: false,
                            uncertainty_reason: None,
                            control_closed: false,
                            stopping: None,
                            cleanup_timeout,
                        };
                        let launch = (|| -> io::Result<LaunchOutcome> {
                            let mut handoff: Vec<_> = descriptors.iter().map(AsFd::as_fd).collect();
                            handoff.extend(listeners.iter().map(AsFd::as_fd));
                            send_packet(
                                &mut group.control,
                                &AnchorRequest::Launch {
                                    spec: spec.clone(),
                                    resources: listeners.len(),
                                    cleanup_timeout_ms: cleanup_timeout.as_millis() as u64,
                                },
                                &handoff,
                            )?;
                            if !matches!(receive_packet::<AnchorEvent>(&mut group.control)?,
                                Some((AnchorEvent::Ready, descriptors)) if descriptors.is_empty())
                            {
                                return Err(io::Error::other(
                                    "anchor did not establish group custody",
                                ));
                            }
                            let terminal = (|| -> io::Result<()> {
                                group.terminal = TerminalRestore::capture(
                                    &descriptors,
                                    spec.caller_group,
                                    group.child.id() as i32,
                                )?;
                                if let Some(terminal) = &group.terminal {
                                    if groups.iter().any(|parent| {
                                        !parent.done
                                            && parent.child.id() as i32 == terminal.previous
                                            && parent.sweep == Sweep::Acknowledged
                                    }) {
                                        return Err(io::Error::other(
                                            "previous terminal group is already leaving custody",
                                        ));
                                    }
                                    terminal.grant()?;
                                }
                                Ok(())
                            })();
                            if let Err(error) = terminal {
                                return Ok(LaunchOutcome::TerminalRejected(error));
                            }
                            send_packet(&mut group.control, &AnchorRequest::Grant, &[])?;
                            Ok(LaunchOutcome::Granted)
                        })();
                        match launch {
                            Ok(LaunchOutcome::Granted) => {}
                            Ok(LaunchOutcome::TerminalRejected(error)) => {
                                fail_scope(&mut scopes, &group.key, &mut admission_failed);
                                emit_group(
                                    &mut owner,
                                    &mut owner_alive,
                                    &group.key,
                                    GroupEvent::Fault {
                                        reason: format!(
                                            "terminal custody rejected before payload launch: {error}"
                                        ),
                                    },
                                );
                            }
                            Err(_) => {
                                uncertain(
                                    &mut group,
                                    &mut owner,
                                    &mut owner_alive,
                                    "anchor launch control failed",
                                );
                                admission_failed = true;
                            }
                        }
                        groups.push(group);
                    }
                }
                Ok(Some((OwnerRequest::StopGroup { scope, id }, descriptors)))
                    if descriptors.is_empty() =>
                {
                    let key = GroupKey { scope, id };
                    let registered = key
                        .scope
                        .as_ref()
                        .and_then(|id| scopes.iter().find(|scope| &scope.id == id));
                    let invalid_scope = if key.scope.is_some() {
                        registered.is_none()
                    } else {
                        !scopes.is_empty()
                    };
                    let invalid_scope = invalid_scope || key.id.is_empty() || key.id.len() > 128;
                    if !invalid_scope
                        && registered.is_some_and(|scope| scope.phase != ScopePhase::Running)
                    {
                        emit_group(
                            &mut owner,
                            &mut owner_alive,
                            &key,
                            GroupEvent::Rejected {
                                id: key.id.clone(),
                                reason: "scope admission closed".to_owned(),
                            },
                        );
                    } else if !invalid_scope
                        && let Some(group) = groups.iter_mut().find(|group| group.key == key)
                    {
                        stop_group(group);
                    } else {
                        admission_failed = true;
                        stopping = true;
                        emit(
                            &mut owner,
                            &mut owner_alive,
                            &CustodyEvent::Fault {
                                reason: "unknown owned group".to_owned(),
                            },
                        );
                    }
                }
                Ok(Some((OwnerRequest::Stop, descriptors))) if descriptors.is_empty() => {
                    stopping = true
                }
                Ok(None) => owner_alive = false,
                Err(_) => {
                    admission_failed = true;
                    emit(
                        &mut owner,
                        &mut owner_alive,
                        &CustodyEvent::Fault {
                            reason: "invalid custody control packet".to_owned(),
                        },
                    );
                    owner_alive = false;
                }
                _ => {
                    admission_failed = true;
                    stopping = true;
                    emit(
                        &mut owner,
                        &mut owner_alive,
                        &CustodyEvent::Fault {
                            reason: "invalid custody control transition".to_owned(),
                        },
                    );
                }
            }
        }
        if !stopping
            && (INTERRUPTED.load(Ordering::Relaxed)
                || started.elapsed() >= Duration::from_millis(config.session_timeout_ms))
        {
            admission_failed = true;
            emit(
                &mut owner,
                &mut owner_alive,
                &CustodyEvent::Fault {
                    reason: if INTERRUPTED.load(Ordering::Relaxed) {
                        "custodian interrupted"
                    } else {
                        "session deadline expired"
                    }
                    .to_owned(),
                },
            );
        }
        if !owner_alive || admission_failed {
            stopping = true;
        }
        for scope in &mut scopes {
            if stopping && scope.phase == ScopePhase::Running {
                scope.phase = ScopePhase::Stopping;
            }
        }
        // A terminal child must restore its predecessor before that predecessor's anchor can exit.
        let terminal_parents: Vec<_> = groups
            .iter()
            .filter(|group| !group.done)
            .filter_map(|group| group.terminal.as_ref())
            .filter_map(|terminal| {
                groups
                    .iter()
                    .find(|parent| parent.child.id() as i32 == terminal.previous)
            })
            .map(|parent| parent.key.clone())
            .collect();
        for group in &mut groups {
            if stopping
                || group.key.scope.as_ref().is_some_and(|id| {
                    scopes
                        .iter()
                        .any(|scope| &scope.id == id && scope.phase != ScopePhase::Running)
                })
            {
                stop_group(group);
            }
            if group.done {
                continue;
            }
            if !group.control_closed && readable(&group.control, Duration::ZERO)? {
                match receive_packet::<AnchorEvent>(&mut group.control) {
                    Ok(Some((AnchorEvent::Started { pid }, descriptors)))
                        if descriptors.is_empty() =>
                    {
                        emit_group(
                            &mut owner,
                            &mut owner_alive,
                            &group.key,
                            GroupEvent::Started {
                                id: group.key.id.clone(),
                                pid,
                                group: group.child.id() as i32,
                            },
                        )
                    }
                    Ok(Some((AnchorEvent::Exited { status }, descriptors)))
                        if descriptors.is_empty() =>
                    {
                        emit_group(
                            &mut owner,
                            &mut owner_alive,
                            &group.key,
                            GroupEvent::Exited {
                                id: group.key.id.clone(),
                                status,
                            },
                        );
                        group.stopping.get_or_insert_with(Instant::now);
                    }
                    Ok(Some((AnchorEvent::LaunchFailed, descriptors)))
                        if descriptors.is_empty() =>
                    {
                        fail_scope(&mut scopes, &group.key, &mut admission_failed);
                        group.stopping.get_or_insert_with(Instant::now);
                        emit_group(
                            &mut owner,
                            &mut owner_alive,
                            &group.key,
                            GroupEvent::Fault {
                                reason: "payload could not be started".to_owned(),
                            },
                        );
                    }
                    Ok(Some((AnchorEvent::Interrupted, descriptors))) if descriptors.is_empty() => {
                        fail_scope(&mut scopes, &group.key, &mut admission_failed);
                        group.stopping.get_or_insert_with(Instant::now);
                        emit_group(
                            &mut owner,
                            &mut owner_alive,
                            &group.key,
                            GroupEvent::Fault {
                                reason: "payload group interrupted".to_owned(),
                            },
                        );
                    }
                    Ok(Some((AnchorEvent::Sweep { forced }, descriptors)))
                        if descriptors.is_empty() && group.sweep == Sweep::NotRequested =>
                    {
                        group.sweep = Sweep::Pending;
                        group.forced = forced;
                    }
                    Ok(None) => group.control_closed = true,
                    Ok(Some((AnchorEvent::Uncertain { reason }, descriptors)))
                        if descriptors.is_empty() =>
                    {
                        group.control_closed = true;
                        uncertain(group, &mut owner, &mut owner_alive, &reason);
                    }
                    Err(error) => {
                        group.control_closed = true;
                        uncertain(
                            group,
                            &mut owner,
                            &mut owner_alive,
                            &format!(
                                "anchor control receive failed ({:?}, errno {:?})",
                                error.kind(),
                                error.raw_os_error()
                            ),
                        );
                    }
                    _ => {
                        group.control_closed = true;
                        uncertain(
                            group,
                            &mut owner,
                            &mut owner_alive,
                            "anchor control or observation failed",
                        );
                    }
                }
            }
            let restoring_child = terminal_parents.contains(&group.key);
            if group.sweep == Sweep::Pending
                && !restoring_child
                && group.uncertainty_reason.is_none()
            {
                if send_packet(&mut group.control, &AnchorRequest::SweepAck, &[]).is_err() {
                    uncertain(
                        group,
                        &mut owner,
                        &mut owner_alive,
                        "anchor sweep acknowledgement failed",
                    );
                } else {
                    group.sweep = Sweep::Acknowledged;
                    group.stopping = Some(Instant::now());
                }
            }
            let mut status = unsafe { std::mem::zeroed::<libc::siginfo_t>() };
            let waited = unsafe {
                libc::waitid(
                    libc::P_PID,
                    group.child.id() as libc::id_t,
                    &mut status,
                    libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                )
            };
            if waited < 0 {
                uncertain(
                    group,
                    &mut owner,
                    &mut owner_alive,
                    "anchor exit observation failed",
                );
            } else if unsafe { status.si_pid() } != 0 && group.control_closed {
                if group.sweep != Sweep::Acknowledged
                    || status.si_code != libc::CLD_KILLED
                    || unsafe { status.si_status() } != libc::SIGKILL
                {
                    uncertain(
                        group,
                        &mut owner,
                        &mut owner_alive,
                        "anchor exited without a certified sweep",
                    );
                } else if group.uncertainty_reason.is_none() {
                    match platform::live_group_members(group.child.id() as i32) {
                        Ok(members) if members.is_empty() => {
                            if group
                                .terminal
                                .as_ref()
                                .is_some_and(|terminal| terminal.restore().is_err())
                            {
                                uncertain(
                                    group,
                                    &mut owner,
                                    &mut owner_alive,
                                    "terminal foreground restoration is uncertain",
                                );
                            } else {
                                group.child.wait()?;
                                group.done = true;
                                emit_group(
                                    &mut owner,
                                    &mut owner_alive,
                                    &group.key,
                                    GroupEvent::Clean {
                                        id: group.key.id.clone(),
                                        forced: group.forced,
                                    },
                                );
                            }
                        }
                        Ok(_) => {}
                        Err(error) => uncertain(
                            group,
                            &mut owner,
                            &mut owner_alive,
                            &format!("owned group membership cannot be observed: {error}"),
                        ),
                    }
                }
            }
            if group
                .stopping
                .is_some_and(|time| time.elapsed() >= group.cleanup_timeout)
                && !group.done
                && !(group.sweep == Sweep::Pending && restoring_child)
            {
                uncertain(
                    group,
                    &mut owner,
                    &mut owner_alive,
                    "cleanup deadline expired without a clean group",
                );
            }
            if group.uncertainty_reason.is_some() {
                fail_scope(&mut scopes, &group.key, &mut admission_failed);
                admission_failed = true;
            }
        }
        for scope in &mut scopes {
            if scope.phase == ScopePhase::Stopping
                && groups
                    .iter()
                    .filter(|group| group.key.scope.as_ref() == Some(&scope.id))
                    .all(|group| group.done)
            {
                scope.phase = ScopePhase::Stopped;
                emit(
                    &mut owner,
                    &mut owner_alive,
                    &CustodyEvent::ScopeStopped {
                        scope: scope.id.clone(),
                        clean: true,
                    },
                );
            }
        }
        let state = (
            stopping,
            scopes
                .iter()
                .map(|scope| (scope.phase, scope.failed))
                .collect::<Vec<_>>(),
            groups
                .iter()
                .map(|group| (group.done, group.uncertainty_reason.is_some(), group.sweep))
                .collect::<Vec<_>>(),
        );
        if last_state.as_ref() != Some(&state) {
            save_state(
                private_config_path,
                &groups,
                &scopes,
                if groups
                    .iter()
                    .any(|group| group.uncertainty_reason.is_some())
                {
                    "custody_retained"
                } else if stopping {
                    "stopping"
                } else {
                    "active"
                },
            )?;
            last_state = Some(state);
        }
        if stopping && groups.iter().all(|group| group.done) {
            drop(listeners);
            save_state(private_config_path, &groups, &scopes, "released")?;
            emit(
                &mut owner,
                &mut owner_alive,
                &CustodyEvent::Stopped { clean: true },
            );
            return Ok(
                if admission_failed || scopes.iter().any(|scope| scope.failed) {
                    1
                } else {
                    0
                },
            );
        }
        if !owner_alive {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

fn retain_anchor(control: &mut UnixStream) -> ! {
    retain_anchor_with_reason(control, "anchor control or self-sweep failed".to_owned())
}

fn retain_anchor_with_reason(control: &mut UnixStream, reason: String) -> ! {
    let _ = send_packet(control, &AnchorEvent::Uncertain { reason }, &[]);
    // Without the independent observer, self-SIGKILL would release the last lease early.
    unsafe {
        libc::kill(0, libc::SIGTERM);
    }
    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// Delivers an anchor event, or retains the anchor because its owner can no longer account for the group.
fn send_or_retain(control: &mut UnixStream, event: &AnchorEvent) {
    if send_packet(control, event, &[]).is_err() {
        retain_anchor(control);
    }
}

fn report_interruption(
    control: &mut UnixStream,
    reported: &mut bool,
    own_termination: bool,
) -> bool {
    let interrupted = INTERRUPTED.swap(false, Ordering::AcqRel);
    if interrupted && !*reported && !own_termination {
        send_or_retain(control, &AnchorEvent::Interrupted);
        *reported = true;
    }
    *reported
}

fn sweep_anchor(control: &mut UnixStream, forced: bool, interruption: &mut bool) -> ! {
    send_or_retain(control, &AnchorEvent::Sweep { forced });
    loop {
        // Nested terminal cleanup may defer acknowledgment beyond the control socket's read timeout.
        match readable(control, Duration::from_millis(10)) {
            Ok(false) => continue,
            Ok(true) => {}
            Err(_) => retain_anchor(control),
        }
        match receive_packet::<AnchorRequest>(control) {
            Ok(Some((AnchorRequest::SweepAck, descriptors))) if descriptors.is_empty() => break,
            Ok(Some((AnchorRequest::Stop, descriptors))) if descriptors.is_empty() => {}
            _ => retain_anchor(control),
        }
    }
    report_interruption(control, interruption, forced);
    unsafe {
        libc::kill(0, libc::SIGKILL);
    }
    retain_anchor(control);
}

pub fn run_anchor() -> io::Result<i32> {
    install_handlers()?;
    let mut control = bootstrap_control()?;
    if unsafe { libc::setpgid(0, 0) } < 0 {
        return Err(io::Error::last_os_error());
    }
    let Some((
        AnchorRequest::Launch {
            spec,
            resources,
            cleanup_timeout_ms,
        },
        descriptors,
    )) = receive_packet(&mut control)?
    else {
        return Err(invalid_data("missing anchor launch"));
    };
    if descriptors.len() != resources + 3 || cleanup_timeout_ms == 0 {
        return Err(invalid_data("invalid anchor launch descriptors"));
    }
    validate_spawn("anchor", &spec)?;
    let mut descriptors = descriptors.into_iter();
    let stdin = descriptors.next().unwrap();
    let stdout = descriptors.next().unwrap();
    let stderr = descriptors.next().unwrap();
    let _leases: Vec<_> = descriptors.collect();
    send_packet(&mut control, &AnchorEvent::Ready, &[])?;
    let mut interruption = false;
    match receive_packet::<AnchorRequest>(&mut control) {
        Ok(Some((AnchorRequest::Grant, descriptors))) if descriptors.is_empty() => {}
        Ok(Some((AnchorRequest::Stop, descriptors))) if descriptors.is_empty() => {
            sweep_anchor(&mut control, false, &mut interruption);
        }
        _ => retain_anchor(&mut control),
    }
    if report_interruption(&mut control, &mut interruption, false) {
        sweep_anchor(&mut control, false, &mut interruption);
    }
    let mut command = Command::new(&spec.argv[0]);
    command
        .args(&spec.argv[1..])
        .current_dir(&spec.cwd)
        .env_clear()
        .envs(spec.environment)
        .stdin(Stdio::from(stdin))
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    unsafe {
        command.pre_exec(move || {
            libc::umask(spec.umask as libc::mode_t);
            Ok(())
        });
    }
    let mut payload = match command.spawn() {
        Ok(payload) => payload,
        Err(_) => {
            send_or_retain(&mut control, &AnchorEvent::LaunchFailed);
            sweep_anchor(&mut control, false, &mut interruption);
        }
    };
    drop(command);
    send_or_retain(&mut control, &AnchorEvent::Started { pid: payload.id() });
    let mut stopped = None;
    let mut exited = false;
    let mut term = false;
    loop {
        if !exited {
            match payload.try_wait() {
                Ok(Some(status)) => {
                    exited = true;
                    stopped.get_or_insert_with(Instant::now);
                    send_or_retain(
                        &mut control,
                        &AnchorEvent::Exited {
                            status: PayloadStatus {
                                code: status.code(),
                                signal: status.signal(),
                            },
                        },
                    );
                }
                Ok(None) => {}
                Err(error) => retain_anchor_with_reason(
                    &mut control,
                    format!("payload exit observation failed: {error}"),
                ),
            }
        }
        if report_interruption(&mut control, &mut interruption, term) {
            stopped.get_or_insert_with(Instant::now);
        }
        if let Some(started) = stopped {
            let members = match platform::live_group_members(unsafe { libc::getpgrp() }) {
                Ok(members) => members,
                Err(error) => retain_anchor_with_reason(
                    &mut control,
                    format!("owned group membership cannot be observed: {error}"),
                ),
            };
            let live_payload = members
                .into_iter()
                .any(|pid| pid != std::process::id() as i32);
            let budget = Duration::from_millis(cleanup_timeout_ms);
            if (!live_payload && exited)
                || (live_payload && started.elapsed() >= budget.mul_f64(0.6))
            {
                sweep_anchor(&mut control, term || live_payload, &mut interruption);
            }
            if !term && started.elapsed() >= budget.mul_f64(0.3) {
                term = true;
                if unsafe { libc::kill(0, libc::SIGTERM) } < 0 {
                    retain_anchor(&mut control);
                }
            }
        }
        if readable(&control, Duration::from_millis(10))? {
            match receive_packet::<AnchorRequest>(&mut control) {
                Ok(Some((AnchorRequest::Stop, descriptors))) if descriptors.is_empty() => {
                    stopped.get_or_insert_with(Instant::now);
                }
                _ => retain_anchor(&mut control),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchor_uncertainty_preserves_the_observation_reason_on_the_wire() {
        let frame = serde_json::json!({
            "kind": "uncertain",
            "reason": "owned group membership cannot be observed: Permission denied (os error 13)"
        });
        let event: AnchorEvent = serde_json::from_value(frame.clone()).unwrap();
        assert_eq!(serde_json::to_value(event).unwrap(), frame);
    }

    #[test]
    fn custody_status_readers_retain_one_complete_snapshot_during_update() {
        use std::io::Read;
        let directory =
            std::env::temp_dir().join(format!("mdrun-custody-snapshot-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&directory).unwrap();
        let outcome = (|| -> io::Result<(serde_json::Value, serde_json::Value)> {
            let path = directory.join("custody.json");
            save_state(&path, &[], &[], "ready")?;
            let mut reader = File::open(path.with_extension("status.json"))?;
            save_state(&path, &[], &[], "released")?;
            let mut original = String::new();
            reader.read_to_string(&mut original)?;
            let original = serde_json::from_str(&original).map_err(invalid_data)?;
            let current = serde_json::from_reader(File::open(path.with_extension("status.json"))?)
                .map_err(invalid_data)?;
            Ok((original, current))
        })();
        fs::remove_dir_all(&directory).unwrap();
        let (original, current) = outcome.unwrap();
        assert_eq!(original["status"], "ready");
        assert_eq!(current["status"], "released");
        assert_eq!(
            original
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["custodian_pid", "groups", "status"]
        );
    }

    #[test]
    fn pregrant_anchor_fixture() {
        if std::env::var_os("MDRUN_PREGRANT_ANCHOR_FIXTURE").is_some() {
            run_anchor().unwrap();
            panic!("anchor returned without its certified sweep");
        }
    }

    #[test]
    fn stop_before_payload_grant_certifies_an_empty_group_without_execution() {
        exercise_pregrant(Some(AnchorRequest::Stop), true, Duration::ZERO);
    }

    #[test]
    fn pending_sweep_waits_for_its_acknowledgement_without_authorizing_payload() {
        exercise_pregrant(Some(AnchorRequest::Stop), true, Duration::from_secs(6));
    }

    #[test]
    fn invalid_or_closed_pregrant_control_retains_uncertainty() {
        exercise_pregrant(Some(AnchorRequest::SweepAck), false, Duration::ZERO);
        exercise_pregrant(None, false, Duration::ZERO);
    }

    fn exercise_pregrant(request: Option<AnchorRequest>, expect_sweep: bool, ack_delay: Duration) {
        let marker =
            std::env::temp_dir().join(format!("mdrun-pregrant-effect-{}", uuid::Uuid::new_v4()));
        let (mut control, bootstrap) = UnixStream::pair().unwrap();
        configure_control(&control).unwrap();
        let lease = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = lease.local_addr().unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "session::custody::tests::pregrant_anchor_fixture",
                "--nocapture",
            ])
            .env("MDRUN_PREGRANT_ANCHOR_FIXTURE", "1")
            .stdin(Stdio::from(OwnedFd::from(bootstrap)))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let outcome = (|| -> io::Result<()> {
            let null = File::open("/dev/null")?;
            send_packet(
                &mut control,
                &AnchorRequest::Launch {
                    spec: SpawnSpec {
                        argv: vec!["/usr/bin/touch".into(), marker.clone().into_os_string()],
                        cwd: std::env::temp_dir(),
                        environment: Vec::new(),
                        umask: 0o022,
                        caller_group: unsafe { libc::getpgrp() },
                    },
                    resources: 1,
                    cleanup_timeout_ms: 1000,
                },
                &[null.as_fd(), null.as_fd(), null.as_fd(), lease.as_fd()],
            )?;
            drop(lease);
            if !matches!(receive_packet(&mut control)?, Some((AnchorEvent::Ready, _))) {
                return Err(io::Error::other("anchor did not establish launch custody"));
            }
            if let Some(request) = request {
                send_packet(&mut control, &request, &[])?;
            } else {
                control.shutdown(std::net::Shutdown::Write)?;
            }
            let event = receive_packet::<AnchorEvent>(&mut control)?;
            if !expect_sweep {
                if matches!(event, Some((AnchorEvent::Uncertain { .. }, descriptors)) if descriptors.is_empty())
                {
                    let until = Instant::now() + Duration::from_millis(100);
                    loop {
                        if child.try_wait()?.is_some() {
                            return Err(io::Error::other("uncertain anchor exited"));
                        }
                        match TcpListener::bind(address) {
                            Err(error) if error.kind() == io::ErrorKind::AddrInUse => {}
                            _ => {
                                return Err(io::Error::other(
                                    "uncertain anchor released its lease",
                                ));
                            }
                        }
                        if Instant::now() >= until {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    return Ok(());
                }
                return Err(io::Error::other(
                    "invalid pre-grant control released custody",
                ));
            }
            if !matches!(event, Some((AnchorEvent::Sweep { forced: false }, _))) {
                return Err(io::Error::other(format!(
                    "pre-grant stop did not request an empty-group sweep: {}",
                    serde_json::to_string(&event.map(|(event, _)| event)).unwrap()
                )));
            }
            std::thread::sleep(ack_delay);
            send_packet(&mut control, &AnchorRequest::SweepAck, &[])?;
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                if let Some(status) = child.try_wait()? {
                    if status.signal() == Some(libc::SIGKILL) {
                        TcpListener::bind(address)?;
                        return Ok(());
                    }
                    return Err(io::Error::other("anchor exited without its self-sweep"));
                }
                if Instant::now() >= deadline {
                    return Err(io::Error::other("anchor did not complete its sweep"));
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        })();
        if child.try_wait().unwrap().is_none() {
            child.kill().unwrap();
        }
        child.wait().unwrap();
        let executed = marker.exists();
        if executed {
            fs::remove_file(&marker).unwrap();
        }
        assert!(!executed, "aborted launch executed its payload");
        outcome.unwrap();
    }

    #[test]
    fn socket_descriptors_do_not_require_terminal_foreground_custody() {
        let (socket, _peer) = UnixStream::pair().unwrap();
        let descriptor = OwnedFd::from(socket);
        let captured =
            TerminalRestore::capture(&[descriptor], unsafe { libc::getpgrp() }, unsafe {
                libc::getpid()
            });
        assert!(captured.unwrap().is_none());
    }

    #[test]
    fn private_files_remain_owner_readable_under_a_restrictive_payload_umask() {
        let directory = std::env::temp_dir().join(format!(
            "mdrun-custody-permissions-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir(&directory).unwrap();
        let mut failures = Vec::new();
        for kind in ["config", "status"] {
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "session::custody::tests::private_file_fixture",
                    "--nocapture",
                ])
                .env("MDRUN_CUSTODY_FILE_FIXTURE", directory.join(kind))
                .env("MDRUN_CUSTODY_FILE_KIND", kind);
            unsafe {
                command.pre_exec(|| {
                    libc::umask(0o777);
                    Ok(())
                });
            }
            let output = command.output().unwrap();
            if !output.status.success() {
                failures.push(String::from_utf8_lossy(&output.stderr).into_owned());
            }
        }
        fs::remove_dir_all(directory).unwrap();
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn private_file_fixture() {
        let Some(path) = std::env::var_os("MDRUN_CUSTODY_FILE_FIXTURE") else {
            return;
        };
        let path = PathBuf::from(path);
        let observed = if std::env::var("MDRUN_CUSTODY_FILE_KIND").unwrap() == "config" {
            let config = CustodyConfig {
                resources: Vec::new(),
                session_timeout_ms: 1000,
                cleanup_timeout_ms: 1000,
            };
            assert!(
                Custody::start(
                    Path::new("/mdrun-missing-permissions-fixture"),
                    &path,
                    &config
                )
                .is_err()
            );
            path
        } else {
            save_state(&path, &[], &[], "fixture").unwrap();
            path.with_extension("status.json")
        };
        assert_eq!(fs::metadata(observed).unwrap().mode() & 0o777, 0o600);
    }

    #[test]
    fn handed_off_descriptor_retains_flags_and_is_closed_on_exec() {
        let (sender, receiver) = UnixStream::pair().unwrap();
        let file = File::open("/dev/null").unwrap();
        let original_flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
        send_fds(&sender, &[file.as_fd()]).unwrap();
        let received = receive_fds(&receiver, 1).unwrap();
        drop(file);
        assert_eq!(
            descriptor_behavior_flags(unsafe {
                libc::fcntl(received[0].as_raw_fd(), libc::F_GETFL)
            }),
            descriptor_behavior_flags(original_flags)
        );
        assert_ne!(
            unsafe { libc::fcntl(received[0].as_raw_fd(), libc::F_GETFD) } & libc::FD_CLOEXEC,
            0
        );
    }

    fn descriptor_behavior_flags(flags: i32) -> i32 {
        // Darwin's FMARK/FWASWRITTEN are kernel bookkeeping, not descriptor behavior.
        // https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/fcntl.h
        #[cfg(target_os = "macos")]
        return flags & !0x0001_1000;
        #[cfg(not(target_os = "macos"))]
        flags
    }

    #[test]
    fn descriptor_flag_projection_preserves_access_append_sync_and_nonblocking() {
        for flag in [
            libc::O_WRONLY,
            libc::O_RDWR,
            libc::O_APPEND,
            libc::O_SYNC,
            libc::O_NONBLOCK,
        ] {
            assert_ne!(
                descriptor_behavior_flags(0),
                descriptor_behavior_flags(flag)
            );
        }
        #[cfg(target_os = "macos")]
        assert_eq!(
            descriptor_behavior_flags(libc::O_RDWR | 0x0001_1000),
            descriptor_behavior_flags(libc::O_RDWR)
        );
        #[cfg(not(target_os = "macos"))]
        assert_eq!(descriptor_behavior_flags(0x0001_1000), 0x0001_1000);
    }
}
