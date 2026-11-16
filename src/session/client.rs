use super::context::NativeContext;
use super::custody::{self, PayloadStatus};
use super::wire::{self, Frame, OutputStream, Role, Terminal};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::fmt;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClientConfig {
    pub protocol: u32,
    pub session_id: String,
    pub artifact_id: String,
    pub token: String,
    pub binding_name: String,
    pub socket: PathBuf,
    pub binding_directory: PathBuf,
    pub baseline: NativeContext,
    pub deadline_unix_ms: u64,
}

impl fmt::Debug for ClientConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClientConfig")
            .finish_non_exhaustive()
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeRequest {
    pub request_id: String,
    pub argv: Vec<OsString>,
    pub context: NativeContext,
    pub terminal: Terminal,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Reply {
    Worker {
        frame: Frame,
    },
    ColdDone {
        request_id: String,
        #[serde(deserialize_with = "wire::required_option")]
        exit_code: Option<u8>,
        #[serde(deserialize_with = "wire::required_option")]
        signal: Option<u32>,
    },
    Error {
        reason: String,
    },
}

pub fn load(path: &Path) -> Result<ClientConfig, String> {
    let file = File::open(path).map_err(|_| "cannot open private session configuration")?;
    let metadata = file
        .metadata()
        .map_err(|_| "cannot inspect session configuration")?;
    if !metadata.is_file()
        || metadata.mode() & 0o077 != 0
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.len() > wire::MAX_FRAME_BYTES as u64
    {
        return Err("session configuration is not a bounded private file".to_owned());
    }
    let config: ClientConfig = serde_json::from_reader(io::BufReader::new(file))
        .map_err(|_| "invalid private session configuration".to_owned())?;
    validate_config(&config)?;
    Ok(config)
}

fn validate_config(config: &ClientConfig) -> Result<(), String> {
    wire::validate_frame(&Frame::Hello {
        protocol: config.protocol,
        session_id: config.session_id.clone(),
        artifact_id: config.artifact_id.clone(),
        token: config.token.clone(),
        role: Role::Client,
    })?;
    if config.binding_name.is_empty()
        || matches!(config.binding_name.as_str(), "." | "..")
        || config.binding_name.contains(['/', '\0'])
        || !config.socket.is_absolute()
        || !config.binding_directory.is_absolute()
        || !config.baseline.cwd.is_absolute()
        || config.baseline.umask > 0o777
    {
        return Err("invalid native session binding configuration".to_owned());
    }
    Ok(())
}

struct Deadline(Instant);

impl Deadline {
    fn new(unix_ms: u64) -> Result<Self, String> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "cannot observe session deadline")?
            .as_millis();
        let remaining = u128::from(unix_ms)
            .checked_sub(now)
            .filter(|remaining| *remaining > 0)
            .ok_or("session deadline expired")?;
        let duration = Duration::from_millis(remaining.min(u128::from(u64::MAX)) as u64);
        Instant::now()
            .checked_add(duration)
            .map(Self)
            .ok_or_else(|| "invalid session deadline".to_owned())
    }

    fn poll(&self, descriptors: &mut [libc::pollfd]) -> Result<(), String> {
        loop {
            let remaining = self
                .0
                .checked_duration_since(Instant::now())
                .ok_or("session deadline expired")?;
            let timeout = remaining.as_millis().clamp(1, 50) as i32;
            if unsafe { libc::poll(descriptors.as_mut_ptr(), descriptors.len() as _, timeout) } >= 0
            {
                return Ok(());
            }
            if io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
                return Err("cannot poll native session descriptors".to_owned());
            }
        }
    }
}

struct PeerClose {
    #[cfg(target_os = "macos")]
    queue: OwnedFd,
    #[cfg(target_os = "linux")]
    socket: i32,
}

impl PeerClose {
    fn new(stream: &UnixStream) -> Result<Self, String> {
        #[cfg(target_os = "macos")]
        {
            let queue = unsafe { libc::kqueue() };
            if queue < 0 {
                return Err("cannot observe native session host liveness".to_owned());
            }
            let queue = unsafe { OwnedFd::from_raw_fd(queue) };
            if custody::set_cloexec(queue.as_raw_fd()).is_err() {
                return Err("cannot configure native host liveness observation".to_owned());
            }
            let event = libc::kevent {
                ident: stream.as_raw_fd() as _,
                filter: libc::EVFILT_READ,
                flags: libc::EV_ADD | libc::EV_ENABLE,
                fflags: 0,
                data: 0,
                udata: std::ptr::null_mut(),
            };
            if unsafe {
                libc::kevent(
                    queue.as_raw_fd(),
                    &event,
                    1,
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null(),
                )
            } < 0
            {
                return Err("cannot observe native session host liveness".to_owned());
            }
            Ok(Self { queue })
        }
        #[cfg(target_os = "linux")]
        {
            Ok(Self {
                socket: stream.as_raw_fd(),
            })
        }
    }

    fn closed(&self) -> Result<bool, String> {
        // poll may report only unread data after close; EV_EOF/POLLRDHUP expose closure
        // independently of backpressured output and queued control bytes.
        // https://man.freebsd.org/cgi/man.cgi?query=kqueue&sektion=2
        #[cfg(target_os = "macos")]
        {
            let mut event: libc::kevent = unsafe { std::mem::zeroed() };
            let timeout = libc::timespec {
                tv_sec: 0,
                tv_nsec: 0,
            };
            let count = unsafe {
                libc::kevent(
                    self.queue.as_raw_fd(),
                    std::ptr::null(),
                    0,
                    &mut event,
                    1,
                    &timeout,
                )
            };
            if count < 0 && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
                return Err("native host liveness observation failed".to_owned());
            }
            if count == 1 && event.flags & libc::EV_ERROR != 0 {
                return Err("native host liveness observation failed".to_owned());
            }
            Ok(count == 1 && event.flags & libc::EV_EOF != 0)
        }
        #[cfg(target_os = "linux")]
        {
            let mut descriptor = descriptor(self.socket, libc::POLLRDHUP);
            let count = unsafe { libc::poll(&mut descriptor, 1, 0) };
            if count < 0 && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
                return Err("native host liveness observation failed".to_owned());
            }
            Ok(count == 1 && descriptor.revents & libc::POLLRDHUP != 0)
        }
    }
}

fn descriptor(fd: i32, events: i16) -> libc::pollfd {
    libc::pollfd {
        fd,
        events,
        revents: 0,
    }
}

fn connect(path: &Path, deadline: &Deadline) -> Result<UnixStream, String> {
    let bytes = path.as_os_str().as_bytes();
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if bytes.contains(&0) || bytes.len() >= address.sun_path.len() {
        return Err("invalid native session socket path".to_owned());
    }
    address.sun_family = libc::AF_UNIX as _;
    for (target, byte) in address.sun_path.iter_mut().zip(bytes) {
        *target = *byte as _;
    }
    #[cfg(target_os = "macos")]
    {
        address.sun_len = std::mem::size_of::<libc::sockaddr_un>() as _;
    }
    let raw = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    if raw < 0 {
        return Err("cannot create native session socket".to_owned());
    }
    let owned = unsafe { OwnedFd::from_raw_fd(raw) };
    if custody::set_cloexec(raw).is_err()
        || unsafe { libc::fcntl(raw, libc::F_SETFL, libc::O_NONBLOCK) } < 0
    {
        return Err("cannot configure native session socket".to_owned());
    }
    #[cfg(target_os = "macos")]
    {
        let enabled: libc::c_int = 1;
        if unsafe {
            libc::setsockopt(
                raw,
                libc::SOL_SOCKET,
                libc::SO_NOSIGPIPE,
                (&enabled as *const libc::c_int).cast(),
                std::mem::size_of_val(&enabled) as _,
            )
        } < 0
        {
            return Err("cannot configure native session socket".to_owned());
        }
    }
    let connected = unsafe {
        libc::connect(
            raw,
            (&address as *const libc::sockaddr_un).cast(),
            std::mem::size_of_val(&address) as _,
        )
    };
    if connected < 0 {
        if io::Error::last_os_error().raw_os_error() != Some(libc::EINPROGRESS) {
            return Err("cannot connect to native session host".to_owned());
        }
        loop {
            let mut descriptors = [descriptor(raw, libc::POLLOUT)];
            deadline.poll(&mut descriptors)?;
            if descriptors[0].revents != 0 {
                break;
            }
        }
        let mut error: libc::c_int = 0;
        let mut length = std::mem::size_of_val(&error) as libc::socklen_t;
        if unsafe {
            libc::getsockopt(
                raw,
                libc::SOL_SOCKET,
                libc::SO_ERROR,
                (&mut error as *mut libc::c_int).cast(),
                &mut length,
            )
        } < 0
            || error != 0
        {
            return Err("cannot connect to native session host".to_owned());
        }
    }
    Ok(UnixStream::from(owned))
}

fn send_initial(stream: &mut UnixStream, bytes: &[u8], deadline: &Deadline) -> Result<(), String> {
    let mut offset = 0;
    while offset < bytes.len() {
        let mut descriptors = [descriptor(stream.as_raw_fd(), libc::POLLOUT)];
        deadline.poll(&mut descriptors)?;
        if descriptors[0].revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 {
            return Err("native session host disconnected".to_owned());
        }
        if descriptors[0].revents & libc::POLLOUT == 0 {
            continue;
        }
        match stream.write(&bytes[offset..]) {
            Ok(0) => return Err("native session host disconnected".to_owned()),
            Ok(count) => offset += count,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                ) => {}
            Err(_) => return Err("cannot send native session request".to_owned()),
        }
    }
    Ok(())
}

#[derive(Default)]
struct Decoder {
    bytes: Vec<u8>,
    expected: Option<usize>,
}

impl Decoder {
    fn receive(&mut self, stream: &mut UnixStream) -> Result<Option<Reply>, String> {
        let target = self.expected.unwrap_or(4);
        let mut buffer = [0_u8; 8192];
        let limit = buffer.len().min(target - self.bytes.len());
        match stream.read(&mut buffer[..limit]) {
            Ok(0) => return Err("native session host disconnected".to_owned()),
            Ok(count) => self.bytes.extend_from_slice(&buffer[..count]),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                ) =>
            {
                return Ok(None);
            }
            Err(_) => return Err("cannot receive native session reply".to_owned()),
        }
        if self.bytes.len() != target {
            return Ok(None);
        }
        if self.expected.is_none() {
            let length = u32::from_be_bytes(self.bytes[..4].try_into().unwrap()) as usize;
            if !(1..=wire::MAX_FRAME_BYTES).contains(&length) {
                return Err("invalid native reply frame length".to_owned());
            }
            self.bytes.clear();
            self.expected = Some(length);
            return Ok(None);
        }
        let reply = serde_json::from_slice(&self.bytes)
            .map_err(|_| "invalid native session reply".to_owned())?;
        self.bytes.clear();
        self.expected = None;
        Ok(Some(reply))
    }
}

enum Phase {
    Routing,
    Warm,
    Cold,
    Acknowledging(PayloadStatus),
    AwaitRelease(PayloadStatus),
}

enum Input {
    Idle,
    Reading(usize),
    Sending { eof: bool },
    Eof,
}

struct Output {
    fd: i32,
    bytes: Vec<u8>,
    offset: usize,
}

struct Outbound {
    bytes: Vec<u8>,
    offset: usize,
}

fn outbound(frame: &Frame) -> Result<Outbound, String> {
    let mut bytes = Vec::new();
    wire::write_frame(&mut bytes, frame)?;
    Ok(Outbound { bytes, offset: 0 })
}

fn status_frame(
    id: &str,
    code: Option<u8>,
    signal: Option<u32>,
) -> Result<(Frame, PayloadStatus), String> {
    let frame = Frame::ClientDone {
        request_id: id.to_owned(),
        exit_code: code,
        signal,
    };
    wire::validate_frame(&frame)?;
    if signal.is_some_and(|signal| signal > i32::MAX as u32) {
        return Err("invalid native application signal".to_owned());
    }
    Ok((
        frame,
        PayloadStatus {
            code: code.map(i32::from),
            signal: signal.map(|signal| signal as i32),
        },
    ))
}

pub fn run(config: &ClientConfig, argv: Vec<OsString>) -> Result<PayloadStatus, String> {
    validate_config(config)?;
    let deadline = Deadline::new(config.deadline_unix_ms)?;
    let mut context = NativeContext::capture()?;
    context.remove_injection(&config.binding_directory, &config.baseline)?;
    let request = NativeRequest {
        request_id: uuid::Uuid::new_v4().to_string(),
        argv,
        context,
        terminal: Terminal {
            stdin: unsafe { libc::isatty(0) } == 1,
            stdout: unsafe { libc::isatty(1) } == 1,
            stderr: unsafe { libc::isatty(2) } == 1,
        },
    };
    let mut stream = connect(&config.socket, &deadline)?;
    let mut hello = Vec::new();
    wire::write_frame(
        &mut hello,
        &Frame::Hello {
            protocol: config.protocol,
            session_id: config.session_id.clone(),
            artifact_id: config.artifact_id.clone(),
            token: config.token.clone(),
            role: Role::Client,
        },
    )?;
    let mut native = Vec::new();
    wire::write_message(&mut native, &request)?;
    send_initial(&mut stream, &hello, &deadline)?;
    send_initial(&mut stream, &native, &deadline)?;
    loop {
        let mut descriptors = [descriptor(stream.as_raw_fd(), libc::POLLOUT)];
        deadline.poll(&mut descriptors)?;
        if descriptors[0].revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 {
            return Err("native session host disconnected".to_owned());
        }
        if descriptors[0].revents & libc::POLLOUT == 0 {
            continue;
        }
        let descriptors = unsafe {
            [
                BorrowedFd::borrow_raw(0),
                BorrowedFd::borrow_raw(1),
                BorrowedFd::borrow_raw(2),
            ]
        };
        match custody::send_fds(&stream, &descriptors) {
            Ok(()) => break,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(_) => return Err("cannot hand native descriptors to session custody".to_owned()),
        }
    }
    forward(&mut stream, &request.request_id, &deadline)
}

fn forward(
    stream: &mut UnixStream,
    id: &str,
    deadline: &Deadline,
) -> Result<PayloadStatus, String> {
    let peer = PeerClose::new(stream)?;
    let mut phase = Phase::Routing;
    let mut input = Input::Idle;
    let mut output: Option<Output> = None;
    let mut sending: Option<Outbound> = None;
    let mut decoder = Decoder::default();
    let mut sequence = 0_u64;
    loop {
        let mut descriptors = [
            descriptor(
                stream.as_raw_fd(),
                if output.is_none() { libc::POLLIN } else { 0 }
                    | if sending.is_some() { libc::POLLOUT } else { 0 },
            ),
            descriptor(
                if matches!(input, Input::Reading(_)) {
                    0
                } else {
                    -1
                },
                libc::POLLIN,
            ),
            descriptor(
                output.as_ref().map_or(-1, |output| output.fd),
                libc::POLLOUT,
            ),
        ];
        deadline.poll(&mut descriptors)?;
        if output.is_some() && peer.closed()? {
            return Err("native session host disconnected".to_owned());
        }
        let socket_events = descriptors[0].revents;
        if socket_events & (libc::POLLERR | libc::POLLNVAL) != 0 {
            return Err("native session host disconnected".to_owned());
        }
        if socket_events & libc::POLLIN != 0 && output.is_none() {
            if let Some(reply) = decoder.receive(stream)? {
                match reply {
                    Reply::Error { .. } => {
                        return Err("native session host rejected the invocation".to_owned());
                    }
                    Reply::ColdDone {
                        request_id,
                        exit_code,
                        signal,
                    } => {
                        if request_id != id || !matches!(phase, Phase::Cold) || sending.is_some() {
                            return Err("unexpected native cold completion".to_owned());
                        }
                        let (frame, status) = status_frame(id, exit_code, signal)?;
                        phase = Phase::Acknowledging(status);
                        sending = Some(outbound(&frame)?);
                    }
                    Reply::Worker { frame } => {
                        wire::validate_frame(&frame)?;
                        if frame.request_id() != Some(id) {
                            return Err("native reply request identity mismatch".to_owned());
                        }
                        match frame {
                            Frame::Accepted { .. } if matches!(phase, Phase::Routing) => {
                                phase = Phase::Warm
                            }
                            Frame::Cold { .. } if matches!(phase, Phase::Routing) => {
                                phase = Phase::Cold
                            }
                            Frame::StdinRead { max_bytes, .. }
                                if matches!(phase, Phase::Warm) && matches!(input, Input::Idle) =>
                            {
                                input = Input::Reading(max_bytes as usize)
                            }
                            Frame::Output {
                                sequence: actual,
                                stream,
                                base64,
                                ..
                            } if matches!(phase, Phase::Warm) && actual == sequence => {
                                let bytes = wire::decode_chunk(&base64)?;
                                sequence += 1;
                                output = Some(Output {
                                    fd: match stream {
                                        OutputStream::Stdout => 1,
                                        OutputStream::Stderr => 2,
                                    },
                                    bytes,
                                    offset: 0,
                                });
                            }
                            Frame::Complete { exit_code, .. }
                                if matches!(phase, Phase::Warm)
                                    && matches!(input, Input::Idle | Input::Eof)
                                    && sending.is_none() =>
                            {
                                let (frame, status) = status_frame(id, Some(exit_code), None)?;
                                phase = Phase::Acknowledging(status);
                                sending = Some(outbound(&frame)?);
                            }
                            Frame::Released { .. } => {
                                if let Phase::AwaitRelease(status) = phase {
                                    return Ok(status);
                                }
                                return Err(
                                    "native release arrived before completion acknowledgement"
                                        .to_owned(),
                                );
                            }
                            _ => {
                                return Err("invalid native invocation state transition".to_owned());
                            }
                        }
                    }
                }
            }
        } else if socket_events & libc::POLLHUP != 0 {
            return Err("native session host disconnected".to_owned());
        }
        if descriptors[1].revents & (libc::POLLERR | libc::POLLNVAL) != 0 {
            return Err("native stdin descriptor failed".to_owned());
        }
        if descriptors[1].revents & (libc::POLLIN | libc::POLLHUP) != 0
            && let Input::Reading(limit) = input
        {
            let mut bytes = vec![0; limit];
            let count = unsafe { libc::read(0, bytes.as_mut_ptr().cast(), bytes.len()) };
            if count < 0 {
                if !matches!(
                    io::Error::last_os_error().kind(),
                    io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                ) {
                    return Err("cannot read native stdin".to_owned());
                }
            } else {
                let eof = count == 0;
                let frame = if eof {
                    Frame::StdinEof {
                        request_id: id.to_owned(),
                    }
                } else {
                    Frame::Stdin {
                        request_id: id.to_owned(),
                        base64: wire::encode_chunk(&bytes[..count as usize])?,
                    }
                };
                input = Input::Sending { eof };
                sending = Some(outbound(&frame)?);
            }
        }
        if descriptors[2].revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
            return Err("native output descriptor failed".to_owned());
        }
        if descriptors[2].revents & libc::POLLOUT != 0
            && let Some(pending) = &mut output
        {
            // POSIX guarantees pipe writes no larger than PIPE_BUF are atomic; 512 is its portable minimum.
            // https://pubs.opengroup.org/onlinepubs/9799919799/functions/write.html
            let end = pending.bytes.len().min(pending.offset + 512);
            let written = unsafe {
                libc::write(
                    pending.fd,
                    pending.bytes[pending.offset..end].as_ptr().cast(),
                    end - pending.offset,
                )
            };
            if written < 0 {
                if !matches!(
                    io::Error::last_os_error().kind(),
                    io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                ) {
                    return Err("cannot deliver native output".to_owned());
                }
            } else if written == 0 {
                return Err("cannot deliver native output".to_owned());
            } else {
                pending.offset += written as usize;
                if pending.offset == pending.bytes.len() {
                    output = None;
                }
            }
        }
        if socket_events & libc::POLLOUT != 0
            && let Some(pending) = &mut sending
        {
            match stream.write(&pending.bytes[pending.offset..]) {
                Ok(0) => return Err("native session host disconnected".to_owned()),
                Ok(count) => pending.offset += count,
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                    ) => {}
                Err(_) => return Err("cannot forward native invocation state".to_owned()),
            }
            if pending.offset == pending.bytes.len() {
                sending = None;
                input = match input {
                    Input::Sending { eof: true } => Input::Eof,
                    Input::Sending { eof: false } => Input::Idle,
                    other => other,
                };
                if let Phase::Acknowledging(status) = phase {
                    phase = Phase::AwaitRelease(status);
                }
            }
        }
    }
}
