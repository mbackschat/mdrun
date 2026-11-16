use serde_json::{Value, json};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

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
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn wait_until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !condition() {
        assert!(Instant::now() < deadline, "custody oracle deadline expired");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn process_identity(pid: i32) -> Option<String> {
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
        if size as usize != size_of_val(&info) || info.pbi_status == libc::SZOMB {
            return None;
        }
        Some(format!(
            "{}:{}",
            info.pbi_start_tvsec, info.pbi_start_tvusec
        ))
    }
    #[cfg(target_os = "linux")]
    {
        let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let (_, fields) = stat.rsplit_once(')')?;
        let fields: Vec<_> = fields.split_whitespace().collect();
        if matches!(*fields.first()?, "Z" | "X" | "x") {
            return None;
        }
        Some(fields.get(19)?.to_string())
    }
}

struct Process {
    pid: i32,
    identity: String,
}

struct OwnerFixture {
    child: Child,
    cleanup: Vec<Process>,
}

impl Drop for OwnerFixture {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
        for process in &self.cleanup {
            process.kill();
            let deadline = Instant::now() + Duration::from_secs(2);
            while process_identity(process.pid).is_some() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
}
impl Process {
    fn record(pid: i32) -> Self {
        Self {
            pid,
            identity: process_identity(pid).expect("fixture process must be live"),
        }
    }
    fn kill(&self) {
        if process_identity(self.pid).as_ref() == Some(&self.identity) {
            unsafe {
                libc::kill(self.pid, libc::SIGKILL);
            }
        }
    }
}

fn available_address() -> SocketAddr {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
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
    owned: Vec<Process>,
}
impl Controller {
    fn start(directory: &Path, address: SocketAddr, session_timeout_ms: u64) -> Self {
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
        let owned = vec![Process::record(child.id() as i32)];
        let mut controller = Self {
            stream,
            child,
            owned,
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
        packet["message"].clone()
    }
    fn launch(&mut self, command: &str, output: &File) -> Value {
        let input = File::open("/dev/null").unwrap();
        self.send(
            json!({"kind":"spawn","id":"payload","spec":spec(command)}),
            &[input.as_raw_fd(), output.as_raw_fd(), output.as_raw_fd()],
        );
        let started = self.receive();
        assert_eq!(started["kind"], "started", "{started}");
        let anchor = started["group"].as_i64().unwrap() as i32;
        if let Some(identity) = process_identity(anchor) {
            self.owned.push(Process {
                pid: anchor,
                identity,
            });
        }
        if let Some(identity) = process_identity(started["pid"].as_i64().unwrap() as i32) {
            self.owned.push(Process {
                pid: started["pid"].as_i64().unwrap() as i32,
                identity,
            });
        }
        started
    }
    fn finish(&mut self) {
        self.send(json!({"kind":"stop"}), &[]);
        assert_eq!(self.receive(), json!({"kind":"stopped","clean":true}));
        assert!(self.child.wait().unwrap().success());
        self.owned.clear();
    }
}
impl Drop for Controller {
    fn drop(&mut self) {
        for process in self.owned.iter().rev() {
            process.kill();
            let deadline = Instant::now() + Duration::from_secs(2);
            while process_identity(process.pid).is_some() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        let _ = self.child.wait();
    }
}

#[test]
fn exact_merged_descriptors_and_direct_status_precede_observed_release() {
    let directory = Directory::new();
    let address = available_address();
    let mut controller = Controller::start(&directory.0, address, 30000);
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
    let directory = Directory::new();
    let address = available_address();
    let mut controller = Controller::start(&directory.0, address, 30000);
    let output = File::create(directory.0.join("output")).unwrap();
    let started = controller.launch("trap '' TERM; exec /bin/sleep 1000", &output);
    controller.owned[1].kill();
    assert_eq!(controller.receive()["kind"], "uncertain");
    assert!(process_identity(started["pid"].as_i64().unwrap() as i32).is_some());
    assert!(excluded(address));
}

#[test]
fn watchdog_failure_is_distinct_from_successful_cleanup() {
    let directory = Directory::new();
    let address = available_address();
    let mut controller = Controller::start(&directory.0, address, 200);
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
    let address = available_address();
    let mut controller = Controller::start(&directory.0, address, 30000);
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
fn trapped_interrupt_is_a_failure_even_when_the_payload_exits_zero() {
    let directory = Directory::new();
    let address = available_address();
    let mut controller = Controller::start(&directory.0, address, 30000);
    let output_path = directory.0.join("output");
    let output = File::create(&output_path).unwrap();
    let started = controller.launch(
        "trap 'exit 0' INT; printf ready; while :; do /bin/sleep 1; done",
        &output,
    );
    wait_until(|| fs::read(&output_path).unwrap() == b"ready");
    assert_eq!(
        unsafe { libc::kill(-(started["group"].as_i64().unwrap() as i32), libc::SIGINT) },
        0
    );
    let mut events = Vec::new();
    loop {
        let event = controller.receive();
        let stopped = event["kind"] == "stopped";
        let clean = event["kind"] == "clean";
        events.push(event);
        if clean {
            controller.send(json!({"kind":"stop"}), &[]);
        }
        if stopped {
            break;
        }
    }
    let status = controller.child.wait().unwrap();
    assert!(
        events.iter().any(|event| *event
            == json!({"kind":"exited","id":"payload","status":{"code":0,"signal":null}})),
        "{events:?}"
    );
    assert!(
        events
            .iter()
            .any(|event| *event == json!({"kind":"fault","reason":"payload group interrupted"})),
        "{events:?}"
    );
    assert!(!status.success());
    assert!(!excluded(address));
}

#[test]
fn interruption_before_the_payload_grant_executes_nothing() {
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
    let owned = vec![Process::record(child.id() as i32)];
    let mut controller = Controller {
        stream,
        child,
        owned,
    };
    let descriptor = File::open("/dev/null").unwrap();
    controller.send(json!({"kind":"launch","spec":spec(&format!("printf executed > '{}'",effect.display())),"resources":0,"cleanup_timeout_ms":2000}), &[descriptor.as_raw_fd();3]);
    assert_eq!(controller.receive(), json!({"kind":"ready"}));
    assert_eq!(
        unsafe { libc::kill(controller.child.id() as i32, libc::SIGINT) },
        0
    );
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
    controller.child.wait().unwrap();
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
    let address = available_address();
    let mut controller = Controller::start(&directory.0, address, 30000);
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
    let anchor = state["groups"][0]["anchor_pid"].as_i64().unwrap() as i32;
    if let Some(identity) = process_identity(anchor) {
        controller.owned.push(Process {
            pid: anchor,
            identity,
        });
    }
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
fn owner_fixture() {
    let Some(directory) = std::env::var_os("MDRUN_CUSTODY_OWNER_FIXTURE") else {
        return;
    };
    let directory = PathBuf::from(directory);
    let address: SocketAddr = std::env::var("MDRUN_CUSTODY_FIXTURE_ADDRESS")
        .unwrap()
        .parse()
        .unwrap();
    let mut controller = Controller::start(&directory, address, 30000);
    let output = File::create(directory.join("output")).unwrap();
    let leaf = directory.join("leaf");
    let command = format!(
        "(trap '' TERM; exec /bin/sleep 1000) & echo $! > '{}'; exit 0",
        leaf.display()
    );
    let started = controller.launch(&command, &output);
    assert_eq!(controller.receive()["kind"], "exited");
    let leaf: i32 = fs::read_to_string(leaf).unwrap().trim().parse().unwrap();
    let ready = json!({"leaf":leaf,"custodian":controller.child.id(),"group":started["group"]});
    fs::write(
        directory.join("ready.tmp"),
        serde_json::to_vec(&ready).unwrap(),
    )
    .unwrap();
    fs::rename(directory.join("ready.tmp"), directory.join("ready.json")).unwrap();
    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}

#[test]
fn owner_sigkill_keeps_exclusion_until_reparented_descendant_is_cleaned() {
    let directory = Directory::new();
    let address = available_address();
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "custody::owner_fixture", "--nocapture"])
        .env("MDRUN_CUSTODY_OWNER_FIXTURE", &directory.0)
        .env("MDRUN_CUSTODY_FIXTURE_ADDRESS", address.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut owner = OwnerFixture {
        child,
        cleanup: Vec::new(),
    };
    wait_until(|| directory.0.join("ready.json").exists());
    let ready: Value =
        serde_json::from_slice(&fs::read(directory.0.join("ready.json")).unwrap()).unwrap();
    let leaf = ready["leaf"].as_i64().unwrap() as i32;
    owner.cleanup = [
        leaf,
        ready["group"].as_i64().unwrap() as i32,
        ready["custodian"].as_i64().unwrap() as i32,
    ]
    .into_iter()
    .map(Process::record)
    .collect();
    assert!(process_identity(leaf).is_some());
    owner.child.kill().unwrap();
    owner.child.wait().unwrap();
    assert!(excluded(address));
    wait_until(|| !excluded(address));
    assert!(process_identity(leaf).is_none());
    let state: Value =
        serde_json::from_slice(&fs::read(directory.0.join("custody.status.json")).unwrap())
            .unwrap();
    assert_eq!(state["status"], "released");
}
