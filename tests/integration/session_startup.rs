use serde_json::{Value, json};
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let directory =
            std::env::temp_dir().join(format!("mdrun-startup-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&directory).unwrap();
        Self(directory)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn send(stream: &mut TcpStream, frame: Value) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(&frame).unwrap();
    stream.write_all(&(bytes.len() as u32).to_be_bytes())?;
    stream.write_all(&bytes)
}

fn receive(stream: &mut TcpStream) -> Option<Value> {
    let mut prefix = [0; 4];
    if stream.read(&mut prefix[..1]).unwrap() == 0 {
        return None;
    }
    stream.read_exact(&mut prefix[1..]).unwrap();
    let size = u32::from_be_bytes(prefix) as usize;
    assert!((1..=1048576).contains(&size));
    let mut bytes = vec![0; size];
    stream.read_exact(&mut bytes).unwrap();
    Some(serde_json::from_slice(&bytes).unwrap())
}

#[test]
fn peer() {
    let Some(path) = std::env::var_os("MDRUN_STARTUP_TEST_CONFIG") else {
        return;
    };
    let config: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    let mode = std::env::var("MDRUN_STARTUP_TEST_MODE").unwrap();
    let connect = || {
        let socket = TcpStream::connect((
            config["connect"]["host"].as_str().unwrap(),
            config["connect"]["port"].as_u64().unwrap() as u16,
        ))
        .unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        socket
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        socket
    };
    let hello = |role: &str| {
        json!({"kind":"hello", "protocol":1, "session_id":config["session_id"],
            "artifact_id":config["artifact_id"], "token":config["token"], "role":role})
    };
    let mut data = connect();
    let mut control = connect();
    let mut greeting = hello("worker_data");
    match mode.as_str() {
        "token" | "session_id" | "artifact_id" => greeting[&mode] = json!("incorrect-identity"),
        "protocol" => greeting["protocol"] = json!(2),
        "role" => greeting["role"] = json!("client"),
        _ => {}
    }
    if send(&mut data, greeting).is_err() {
        return;
    }
    if send(
        &mut control,
        hello(if mode == "duplicate_role" {
            "worker_data"
        } else {
            "worker_control"
        }),
    )
    .is_err()
    {
        return;
    }
    let mut ready = json!({"kind":"ready", "protocol":1, "session_id":config["session_id"],
        "artifact_id":config["artifact_id"], "capabilities":["pull_stdin","sequenced_output","release_ack"]});
    match mode.as_str() {
        "ready_session" => ready["session_id"] = json!("incorrect-session"),
        "ready_artifact" => ready["artifact_id"] = json!("incorrect-artifact"),
        "ready_protocol" => ready["protocol"] = json!(2),
        "missing_capability" => ready["capabilities"] = json!(["pull_stdin", "sequenced_output"]),
        "duplicate_capability" => {
            ready["capabilities"] = json!(["pull_stdin", "pull_stdin", "release_ack"])
        }
        "unknown_capability" => {
            ready["capabilities"] = json!(["pull_stdin", "sequenced_output", "unknown"])
        }
        "valid" | "token" | "session_id" | "artifact_id" | "protocol" | "role"
        | "duplicate_role" => {}
        _ => panic!("unknown startup fixture mode"),
    }
    if send(&mut data, ready).is_err() {
        return;
    }
    let terminal = receive(&mut control);
    if mode == "valid" {
        assert_eq!(terminal, Some(json!({"kind":"shutdown"})));
    } else {
        assert!(terminal.is_none());
    }
}

fn check_startup(mode: &str) {
    let fixture = Fixture::new();
    let binary = std::env::current_exe().unwrap();
    let quoted_binary = binary.to_str().unwrap().replace('\'', "'\"'\"'");
    let launcher = fixture.0.join("worker.sh");
    fs::write(&launcher, format!(
        "MDRUN_STARTUP_TEST_CONFIG=\"$1\" MDRUN_STARTUP_TEST_MODE={mode} exec '{quoted_binary}' --exact session_startup::peer --nocapture\n"
    )).unwrap();
    let binding = fixture.0.join("binding.json");
    fs::write(&binding, serde_json::to_vec(&json!({
        "version":1, "name":"tool", "artifact_id":"startup-test", "worker_argv":["/bin/sh",launcher],
        "cold_argv":["/bin/false"], "startup_timeout_ms":3000, "session_timeout_ms":8000, "cleanup_timeout_ms":1000
    })).unwrap()).unwrap();
    let stdout = fs::File::create(fixture.0.join("stdout")).unwrap();
    let stderr = fs::File::create(fixture.0.join("stderr")).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_mdrun"))
        .args(["session", "--binding"])
        .arg(binding)
        .args([
            "--report",
            "report.json",
            "--",
            "/bin/sh",
            "-c",
            "printf authorized > author-effect",
        ])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .current_dir(&fixture.0)
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr)
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(12);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("startup fixture exceeded external deadline: {mode}");
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let report: Value =
        serde_json::from_slice(&fs::read(fixture.0.join("report.json")).unwrap()).unwrap();
    assert_eq!(status.success(), mode == "valid", "{mode}: {report}");
    assert_eq!(
        fixture.0.join("author-effect").exists(),
        mode == "valid",
        "{mode}: author admission"
    );
    assert!(fs::read(fixture.0.join("stdout")).unwrap().is_empty());
    assert_eq!(report["cleanup"]["outcome"], "clean", "{mode}: {report}");
    assert_eq!(report["worker_starts"], 1, "{mode}: {report}");
    assert_eq!(report["warm_calls"], 0);
    assert_eq!(report["cold_calls"], 0);
    assert_eq!(report["application_statuses"], json!([]));
    if mode == "valid" {
        assert_eq!(
            report["author_status"],
            json!({"exit_code":0,"signal":null})
        );
        assert!(report["infrastructure_failure"].is_null());
        assert_eq!(
            fs::read(fixture.0.join("author-effect")).unwrap(),
            b"authorized"
        );
    } else {
        assert!(report["author_status"].is_null(), "{mode}: {report}");
        assert!(
            !report["infrastructure_failure"]
                .as_str()
                .unwrap()
                .is_empty()
        );
        if matches!(mode, "role" | "duplicate_role") {
            assert_eq!(
                report["infrastructure_failure"], "duplicate or invalid worker role",
                "{mode}: {report}"
            );
        }
    }
}

#[test]
fn only_an_authenticated_ready_worker_can_admit_the_author() {
    for mode in [
        "valid",
        "token",
        "session_id",
        "artifact_id",
        "protocol",
        "role",
        "duplicate_role",
        "ready_session",
        "ready_artifact",
        "ready_protocol",
        "missing_capability",
        "duplicate_capability",
        "unknown_capability",
    ] {
        check_startup(mode);
    }
}
