use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    directory: PathBuf,
    listener: UnixListener,
}

impl Fixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "mdrun-client-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        let directory = fs::canonicalize(directory).unwrap();
        symlink(env!("CARGO_BIN_EXE_mdrun"), directory.join("bound-tool")).unwrap();
        let listener = UnixListener::bind(directory.join("host.sock")).unwrap();
        listener.set_nonblocking(true).unwrap();
        let environment: Vec<(OsString, OsString)> = vec![("PATH".into(), "/usr/bin:/bin".into())];
        let config = json!({
            "protocol":1,"session_id":"fake-session","artifact_id":"fake-artifact",
            "token":"private-client-token","binding_name":"bound-tool",
            "socket":directory.join("host.sock"),"binding_directory":directory,
            "baseline":{"cwd":directory.as_os_str(),"environment":environment,"umask":18,"caller_group":0},
            "deadline_unix_ms":SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64 + 10000
        });
        fs::write(
            directory.join("client.json"),
            serde_json::to_vec(&config).unwrap(),
        )
        .unwrap();
        fs::set_permissions(
            directory.join("client.json"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        Self {
            directory,
            listener,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(self.directory.join("bound-tool"));
        command
            .current_dir(&self.directory)
            .env_clear()
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", self.directory.display()),
            )
            .env("MDRUN_SESSION", self.directory.join("client.json"))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }

    fn accept(&self) -> (UnixStream, Value, Vec<OwnedFd>) {
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut stream = loop {
            match self.listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Err(error) => panic!("native alias did not authenticate to fake host: {error}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        assert_eq!(
            read(&mut stream),
            json!({"kind":"hello","protocol":1,"session_id":"fake-session","artifact_id":"fake-artifact","token":"private-client-token","role":"client"})
        );
        let request = read(&mut stream);
        let descriptors = receive_descriptors(&stream);
        (stream, request, descriptors)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.directory).unwrap();
    }
}

fn read(stream: &mut UnixStream) -> Value {
    let mut prefix = [0; 4];
    stream.read_exact(&mut prefix).unwrap();
    let length = u32::from_be_bytes(prefix) as usize;
    assert!((1..=1024 * 1024).contains(&length));
    let mut bytes = vec![0; length];
    stream.read_exact(&mut bytes).unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn write(stream: &mut UnixStream, value: Value) {
    let bytes = serde_json::to_vec(&value).unwrap();
    stream
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .unwrap();
    stream.write_all(&bytes).unwrap();
}

fn worker(stream: &mut UnixStream, frame: Value) {
    write(stream, json!({"kind":"worker","frame":frame}));
}

fn receive_descriptors(stream: &UnixStream) -> Vec<OwnedFd> {
    let mut ancillary = [0_usize; 32];
    let mut marker = [0_u8];
    let mut iov = libc::iovec {
        iov_base: marker.as_mut_ptr().cast(),
        iov_len: 1,
    };
    let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
    message.msg_iov = &mut iov;
    message.msg_iovlen = 1;
    message.msg_control = ancillary.as_mut_ptr().cast();
    message.msg_controllen = std::mem::size_of_val(&ancillary) as _;
    assert_eq!(
        unsafe { libc::recvmsg(stream.as_raw_fd(), &mut message, 0) },
        1
    );
    assert_eq!(marker, [b'F']);
    let header = unsafe { libc::CMSG_FIRSTHDR(&message) };
    assert!(!header.is_null());
    unsafe {
        assert_eq!((*header).cmsg_level, libc::SOL_SOCKET);
        assert_eq!((*header).cmsg_type, libc::SCM_RIGHTS);
        assert_eq!(
            (*header).cmsg_len as usize,
            libc::CMSG_LEN(3 * std::mem::size_of::<i32>() as u32) as usize
        );
        let values = libc::CMSG_DATA(header).cast::<i32>();
        (0..3)
            .map(|index| OwnedFd::from_raw_fd(*values.add(index)))
            .collect()
    }
}

fn finished(mut child: Child) -> Output {
    let deadline = Instant::now() + Duration::from_secs(3);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            child.kill().unwrap();
            panic!("native client did not stop within deadline");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    child.wait_with_output().unwrap()
}

fn complete(stream: &mut UnixStream, id: &Value, code: u8) {
    worker(
        stream,
        json!({"kind":"complete","request_id":id,"exit_code":code}),
    );
    assert_eq!(
        read(stream),
        json!({"kind":"client_done","request_id":id,"exit_code":code,"signal":null})
    );
    worker(stream, json!({"kind":"released","request_id":id}));
}

fn pipe() -> (File, File) {
    let mut descriptors = [-1; 2];
    assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
    unsafe {
        (
            File::from_raw_fd(descriptors[0]),
            File::from_raw_fd(descriptors[1]),
        )
    }
}

#[test]
fn ignores_open_stdin_preserves_native_arguments_and_orders_merged_binary_output() {
    let fixture = Fixture::new();
    let (mut independent_input, mut input) = pipe();
    input.write_all(b"unread sentinel").unwrap();
    let output_file = File::create(fixture.directory.join("merged")).unwrap();
    let mut command = fixture.command();
    command
        .args([
            OsString::from("--help"),
            OsString::new(),
            OsString::from_vec(b"native-\xff".to_vec()),
        ])
        .stdin(independent_input.try_clone().unwrap())
        .stdout(output_file.try_clone().unwrap())
        .stderr(output_file);
    let child = command.spawn().unwrap();
    let (mut host, request, descriptors) = fixture.accept();
    drop(descriptors);
    assert_eq!(
        request["argv"],
        json!([
            OsString::from("--help"),
            OsString::new(),
            OsString::from_vec(b"native-\xff".to_vec())
        ])
    );
    assert_eq!(
        request["context"]["cwd"],
        json!(fixture.directory.as_os_str())
    );
    let id = &request["request_id"];
    worker(&mut host, json!({"kind":"accepted","request_id":id}));
    for (sequence, stream, bytes) in [
        (0, "stdout", b"\0\xffA".as_slice()),
        (1, "stderr", b"B\xfe\n"),
    ] {
        worker(
            &mut host,
            json!({"kind":"output","request_id":id,"sequence":sequence,"stream":stream,"base64":STANDARD.encode(bytes)}),
        );
    }
    complete(&mut host, id, 7);
    let output = finished(child);
    assert_eq!(output.status.code(), Some(7));
    assert_eq!(
        fs::read(fixture.directory.join("merged")).unwrap(),
        b"\0\xffAB\xfe\n"
    );
    let mut unread = [0; 15];
    independent_input.read_exact(&mut unread).unwrap();
    assert_eq!(&unread, b"unread sentinel");
    drop(input);
}

#[test]
fn input_credit_streams_beyond_the_frame_limit_and_eof_needs_credit() {
    let fixture = Fixture::new();
    let mut child = fixture.command().stdin(Stdio::piped()).spawn().unwrap();
    let mut input = child.stdin.take().unwrap();
    let expected: Vec<_> = (0..20 * 65536).map(|index| (index % 251) as u8).collect();
    let source = expected.clone();
    let writer = std::thread::spawn(move || input.write_all(&source));
    let (mut host, request, descriptors) = fixture.accept();
    drop(descriptors);
    let id = &request["request_id"];
    worker(&mut host, json!({"kind":"accepted","request_id":id}));
    let mut received = Vec::new();
    loop {
        worker(
            &mut host,
            json!({"kind":"stdin_read","request_id":id,"max_bytes":65536}),
        );
        let frame = read(&mut host);
        assert_eq!(frame["request_id"], *id);
        if frame["kind"] == "stdin_eof" {
            break;
        }
        assert_eq!(frame["kind"], "stdin");
        let bytes = STANDARD.decode(frame["base64"].as_str().unwrap()).unwrap();
        assert!(!bytes.is_empty() && bytes.len() <= 65536);
        received.extend(bytes);
    }
    complete(&mut host, id, 0);
    assert!(finished(child).status.success());
    writer.join().unwrap().unwrap();
    assert_eq!(received, expected);
}

#[test]
fn disconnect_cancels_a_pending_input_read_without_consuming_input() {
    let fixture = Fixture::new();
    let (mut independent_input, mut writer) = pipe();
    let child = fixture
        .command()
        .stdin(independent_input.try_clone().unwrap())
        .spawn()
        .unwrap();
    let (mut host, request, descriptors) = fixture.accept();
    drop(descriptors);
    let id = &request["request_id"];
    worker(&mut host, json!({"kind":"accepted","request_id":id}));
    worker(
        &mut host,
        json!({"kind":"stdin_read","request_id":id,"max_bytes":64}),
    );
    drop(host);
    let output = finished(child);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        output.stderr.is_empty(),
        "infrastructure diagnostics belong to the outer host"
    );
    writer.write_all(b"still unread").unwrap();
    let mut bytes = [0; 12];
    independent_input.read_exact(&mut bytes).unwrap();
    assert_eq!(&bytes, b"still unread");
}

#[test]
fn acknowledges_only_delivered_output_and_cancels_blocked_output_on_disconnect() {
    for disconnect in [false, true] {
        let fixture = Fixture::new();
        let (mut stdout, mut sink) = pipe();
        let flags = unsafe { libc::fcntl(sink.as_raw_fd(), libc::F_GETFL) };
        assert_eq!(
            unsafe { libc::fcntl(sink.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) },
            0
        );
        let mut prefilled = 0;
        loop {
            match sink.write(&[0; 512]) {
                Ok(count) => prefilled += count,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                result => panic!("cannot prefill output pipe: {result:?}"),
            }
        }
        assert_eq!(
            unsafe { libc::fcntl(sink.as_raw_fd(), libc::F_SETFL, flags) },
            0
        );
        let child = fixture.command().stdout(sink).spawn().unwrap();
        let (mut host, request, descriptors) = fixture.accept();
        drop(descriptors);
        let id = &request["request_id"];
        worker(&mut host, json!({"kind":"accepted","request_id":id}));
        let bytes = vec![42_u8; 65536];
        worker(
            &mut host,
            json!({"kind":"output","request_id":id,"sequence":0,"stream":"stdout","base64":STANDARD.encode(&bytes)}),
        );
        worker(
            &mut host,
            json!({"kind":"complete","request_id":id,"exit_code":0}),
        );
        host.set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        let mut probe = [0];
        assert!(
            matches!(host.read(&mut probe), Err(error) if matches!(error.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock))
        );
        if disconnect {
            drop(host);
            assert!(!finished(child).status.success());
        } else {
            stdout.read_exact(&mut vec![0; prefilled]).unwrap();
            let mut delivered = vec![0; bytes.len()];
            stdout.read_exact(&mut delivered).unwrap();
            assert_eq!(delivered, bytes);
            host.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            assert_eq!(
                read(&mut host),
                json!({"kind":"client_done","request_id":id,"exit_code":0,"signal":null})
            );
            worker(&mut host, json!({"kind":"released","request_id":id}));
            assert!(finished(child).status.success());
        }
    }
}

#[test]
fn cold_handoff_keeps_descriptor_flags_and_reports_native_status_without_relay() {
    let fixture = Fixture::new();
    let (input, _writer) = pipe();
    let flags = unsafe { libc::fcntl(input.as_raw_fd(), libc::F_GETFL) };
    let child = fixture
        .command()
        .stdin(input.try_clone().unwrap())
        .spawn()
        .unwrap();
    let (mut host, request, descriptors) = fixture.accept();
    assert_eq!(
        unsafe { libc::fcntl(descriptors[0].as_raw_fd(), libc::F_GETFL) },
        unsafe { libc::fcntl(input.as_raw_fd(), libc::F_GETFL) }
    );
    let material_flags =
        libc::O_ACCMODE | libc::O_APPEND | libc::O_NONBLOCK | libc::O_ASYNC | libc::O_SYNC;
    assert_eq!(
        unsafe { libc::fcntl(input.as_raw_fd(), libc::F_GETFL) } & material_flags,
        flags & material_flags
    );
    let environment: Vec<(OsString, OsString)> =
        serde_json::from_value(request["context"]["environment"].clone()).unwrap();
    assert_eq!(environment, vec![("PATH".into(), "/usr/bin:/bin".into())]);
    let id = &request["request_id"];
    worker(
        &mut host,
        json!({"kind":"cold","request_id":id,"reason":"explicit_cold"}),
    );
    let mut native_stdout = File::from(descriptors.into_iter().nth(1).unwrap());
    native_stdout.write_all(b"native bytes\0\xff").unwrap();
    drop(native_stdout);
    write(
        &mut host,
        json!({"kind":"cold_done","request_id":id,"exit_code":23,"signal":null}),
    );
    assert_eq!(
        read(&mut host),
        json!({"kind":"client_done","request_id":id,"exit_code":23,"signal":null})
    );
    worker(&mut host, json!({"kind":"released","request_id":id}));
    let output = finished(child);
    assert_eq!(output.status.code(), Some(23));
    assert_eq!(output.stdout, b"native bytes\0\xff");
}

#[test]
fn rejects_overlapping_credit_wrong_sequence_late_output_and_invalid_cold_status() {
    for case in [
        "overlap",
        "sequence",
        "late",
        "status",
        "preaccept",
        "zero_credit",
        "oversized_credit",
        "wrong_id",
        "complete_pending",
        "early_release",
    ] {
        let fixture = Fixture::new();
        let mut child = fixture.command().stdin(Stdio::piped()).spawn().unwrap();
        let _open_input = child.stdin.take().unwrap();
        let (mut host, request, descriptors) = fixture.accept();
        drop(descriptors);
        let id = &request["request_id"];
        if case == "status" {
            worker(
                &mut host,
                json!({"kind":"cold","request_id":id,"reason":"explicit_cold"}),
            );
            write(
                &mut host,
                json!({"kind":"cold_done","request_id":id,"exit_code":0,"signal":9}),
            );
        } else {
            if case != "preaccept" {
                worker(&mut host, json!({"kind":"accepted","request_id":id}));
            }
            match case {
                "overlap" => {
                    for _ in 0..2 {
                        worker(
                            &mut host,
                            json!({"kind":"stdin_read","request_id":id,"max_bytes":1}),
                        );
                    }
                }
                "zero_credit" | "oversized_credit" => worker(
                    &mut host,
                    json!({"kind":"stdin_read","request_id":id,"max_bytes":if case == "zero_credit" {0} else {65537}}),
                ),
                "wrong_id" => worker(
                    &mut host,
                    json!({"kind":"stdin_read","request_id":"another-request","max_bytes":1}),
                ),
                "complete_pending" => {
                    worker(
                        &mut host,
                        json!({"kind":"stdin_read","request_id":id,"max_bytes":1}),
                    );
                    worker(
                        &mut host,
                        json!({"kind":"complete","request_id":id,"exit_code":0}),
                    );
                }
                "early_release" => worker(&mut host, json!({"kind":"released","request_id":id})),
                "late" => {
                    worker(
                        &mut host,
                        json!({"kind":"complete","request_id":id,"exit_code":0}),
                    );
                    assert_eq!(read(&mut host)["kind"], "client_done");
                    worker(
                        &mut host,
                        json!({"kind":"output","request_id":id,"sequence":0,"stream":"stdout","base64":"eA=="}),
                    );
                }
                _ => worker(
                    &mut host,
                    json!({"kind":"output","request_id":id,"sequence":1,"stream":"stdout","base64":"eA=="}),
                ),
            }
        }
        assert!(!finished(child).status.success(), "case {case}");
    }
}
