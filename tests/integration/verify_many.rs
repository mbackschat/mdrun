use serde_json::{Value, json};
use std::cell::Cell;
use std::fs;
use std::io::{Read, Write};
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStrExt;
#[cfg(target_os = "linux")]
use std::os::unix::ffi::OsStringExt;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

struct Fixture(PathBuf, Cell<bool>);

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("mdrun-many-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let fixture = Self(root, Cell::new(false));
        fixture.write("payload.sh", b"id=$1\n: > \"$id.started\"\nwhile test ! -f \"$id.go\"; do sleep 0.01; done\nif test \"$id\" = fail; then printf unexpected; else printf '%s\\n' \"$id\"; fi\nif test \"$id\" = nonzero; then exit 17; fi\n");
        fixture.write("binding.json", serde_json::to_vec(&json!({"version":1,"name":"tool","artifact_id":"batch-fixture","worker_argv":["/unused"],"cold_argv":["/bin/sh",fixture.0.join("payload.sh")],"session_timeout_ms":10000,"cleanup_timeout_ms":1000})).unwrap());
        fixture
    }

    fn write(&self, name: &str, bytes: impl AsRef<[u8]>) {
        fs::write(self.0.join(name), bytes).unwrap();
    }

    fn document(&self, name: &str, payload: &str, expected: &str) {
        self.write(
            name,
            format!("```sh\ntool {payload}\n```\n\n```output\n{expected}\n```\n"),
        );
    }

    fn command(&self, jobs: &str, files: &[&str]) -> Command {
        self.command_extra(jobs, files, &[])
    }

    fn command_extra(&self, jobs: &str, files: &[&str], extra: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_mdrun"));
        command
            .current_dir(&self.0)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .args([
                "verify-many",
                "--binding",
                "binding.json",
                "--mode",
                "cold",
                "--jobs",
                jobs,
                "--report",
                "batch.json",
            ])
            .args(extra)
            .arg("--")
            .args(files);
        command
    }

    fn launch(&self, jobs: &str, files: &[&str]) -> OwnedBatch {
        let child = self
            .command(jobs, files)
            .stdin(Stdio::null())
            .stdout(fs::File::create(self.0.join("stdout")).unwrap())
            .stderr(fs::File::create(self.0.join("stderr")).unwrap())
            .spawn()
            .unwrap();
        OwnedBatch(child)
    }

    fn wait_file(&self, name: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !self.0.join(name).exists() {
            assert!(
                Instant::now() < deadline,
                "missing {name}: {}",
                String::from_utf8_lossy(&fs::read(self.0.join("stderr")).unwrap_or_default())
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn wait_stdout(&self, text: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !fs::read_to_string(self.0.join("stdout"))
            .unwrap_or_default()
            .contains(text)
        {
            assert!(Instant::now() < deadline, "missing batch output {text}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn report(&self) -> Value {
        serde_json::from_slice(&fs::read(self.0.join("batch.json")).unwrap()).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if !self.1.get() {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

struct OwnedBatch(Child);
impl OwnedBatch {
    fn finish(&mut self) -> std::process::ExitStatus {
        self.finish_within(Duration::from_secs(15))
    }

    fn finish_within(&mut self, duration: Duration) -> std::process::ExitStatus {
        let deadline = Instant::now() + duration;
        loop {
            if let Some(status) = self.0.try_wait().unwrap() {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "batch exceeded its external deadline"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

fn short_budget(fixture: &Fixture) {
    let mut binding: Value =
        serde_json::from_slice(&fs::read(fixture.0.join("binding.json")).unwrap()).unwrap();
    binding["session_timeout_ms"] = json!(500);
    binding["cleanup_timeout_ms"] = json!(100);
    fixture.write("binding.json", serde_json::to_vec(&binding).unwrap());
}

#[test]
fn custody_start_failure_replaces_stale_report_and_leaves_no_workspace() {
    let fixture = Fixture::new();
    let listener = crate::ports::listener();
    let address = listener.local_addr().unwrap();
    let mut binding: Value =
        serde_json::from_slice(&fs::read(fixture.0.join("binding.json")).unwrap()).unwrap();
    binding["resources"] =
        json!([{"kind":"tcp_listener","host":"127.0.0.1","port":address.port()}]);
    fixture.write("binding.json", serde_json::to_vec(&binding).unwrap());
    fixture.document("a.md", "a", "a");
    fixture.document("b.md", "b", "b");
    fixture.write("batch.json", br#"{"outcome":"success","stale":true}"#);
    let temporary = fixture.0.join("temporary");
    fs::create_dir(&temporary).unwrap();
    let mut batch = OwnedBatch(
        fixture
            .command("2", &["a.md", "b.md"])
            .env("TMPDIR", &temporary)
            .stdin(Stdio::null())
            .stdout(fs::File::create(fixture.0.join("stdout")).unwrap())
            .stderr(fs::File::create(fixture.0.join("stderr")).unwrap())
            .spawn()
            .unwrap(),
    );
    assert_eq!(batch.finish_within(Duration::from_secs(5)).code(), Some(1));
    let report = fixture.report();
    assert_eq!(report["outcome"], "failure");
    assert!(
        report["infrastructure_failure"]
            .as_str()
            .unwrap()
            .contains("custody")
    );
    assert_eq!(report["documents"].as_array().unwrap().len(), 2);
    for document in report["documents"].as_array().unwrap() {
        assert_eq!(document["state"], "not_started");
    }
    assert!(
        fs::read_to_string(fixture.0.join("stderr"))
            .unwrap()
            .contains("cannot establish batch custody")
    );
    assert_eq!(fs::read_dir(&temporary).unwrap().count(), 0);
    assert!(!fixture.0.join("a.started").exists());
    assert!(!fixture.0.join("b.started").exists());
}

#[test]
fn bound_verify_rejects_report_aliases_before_execution() {
    use std::os::unix::fs::symlink;
    for alias in ["a.md", "hard.md", "symbolic.md", "binding.json"] {
        let fixture = Fixture::new();
        let source = b"```sh\n: > effect\n```\n";
        fixture.write("a.md", source);
        fs::hard_link(fixture.0.join("a.md"), fixture.0.join("hard.md")).unwrap();
        symlink("a.md", fixture.0.join("symbolic.md")).unwrap();
        let binding = fs::read(fixture.0.join("binding.json")).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_mdrun"))
            .current_dir(&fixture.0)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .args([
                "verify",
                "--binding",
                "binding.json",
                "--mode",
                "cold",
                "--report",
                alias,
                "a.md",
            ])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "alias {alias}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("report conflicts"),
            "alias {alias}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(fs::read(fixture.0.join("a.md")).unwrap(), source);
        assert_eq!(fs::read(fixture.0.join("binding.json")).unwrap(), binding);
        assert!(!fixture.0.join("effect").exists());
    }
}

#[test]
fn bound_verify_rejects_dangling_output_symlink_to_report() {
    let fixture = Fixture::new();
    fixture.write("a.md", b"```sh\n: > effect\n```\n");
    std::os::unix::fs::symlink("report.json", fixture.0.join("output.md")).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_mdrun"))
        .current_dir(&fixture.0)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .args([
            "verify",
            "--binding",
            "binding.json",
            "--mode",
            "cold",
            "--output",
            "output.md",
            "--report",
            "report.json",
            "a.md",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("report conflicts"));
    assert!(!fixture.0.join("effect").exists());
    assert!(!fixture.0.join("report.json").exists());
}

#[test]
fn session_report_failure_preserves_infrastructure_diagnostics() {
    let fixture = Fixture::new();
    fixture.write("a.md", b"```sh\n: > effect\n```\n");
    let mut binding: Value =
        serde_json::from_slice(&fs::read(fixture.0.join("binding.json")).unwrap()).unwrap();
    binding["worker_argv"] = json!(["/bin/sh", "-c", "mkdir report.json; exit 1"]);
    fixture.write("binding.json", serde_json::to_vec(&binding).unwrap());
    let output = Command::new(env!("CARGO_BIN_EXE_mdrun"))
        .current_dir(&fixture.0)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .args([
            "verify",
            "--binding",
            "binding.json",
            "--report",
            "report.json",
            "a.md",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("session infrastructure failure"),
        "{stderr}"
    );
    assert!(
        stderr.contains("cannot publish execution report"),
        "{stderr}"
    );
    assert!(!fixture.0.join("effect").exists());
}

#[test]
fn batch_report_publication_failure_has_a_bounded_diagnostic() {
    let fixture = Fixture::new();
    fixture.write("a.md", b"```sh\nmkdir batch.json\n```\n");
    let mut batch = fixture.launch("1", &["a.md"]);
    assert_eq!(batch.finish_within(Duration::from_secs(5)).code(), Some(1));
    let stderr = fs::read_to_string(fixture.0.join("stderr")).unwrap();
    assert!(
        stderr.contains("cannot publish execution report"),
        "{stderr}"
    );
}

#[test]
fn diagnostic_publisher_expires_without_an_owner_monitor() {
    use std::os::unix::process::ExitStatusExt;
    let fixture = Fixture::new();
    fixture.write(
        "diagnostic.json",
        serde_json::to_vec(
            &json!({"label":"mdrun","stdout":null,"stderr":null,"diagnostics":["failure"]}),
        )
        .unwrap(),
    );
    let (_unread, mut writer) = UnixStream::pair().unwrap();
    writer.set_nonblocking(true).unwrap();
    while let Ok(size) = writer.write(&[0u8; 65536]) {
        assert!(size > 0);
    }
    writer.set_nonblocking(false).unwrap();
    let mut publisher = OwnedBatch(
        Command::new(env!("CARGO_BIN_EXE_mdrun"))
            .args(["__batch-publish", "--diagnostics-only", "--config"])
            .arg(fixture.0.join("diagnostic.json"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(OwnedFd::from(writer)))
            .spawn()
            .unwrap(),
    );
    let status = publisher.finish_within(Duration::from_secs(3));
    assert_eq!(status.signal(), Some(libc::SIGALRM));
}

#[test]
fn publication_unread_stdout_cannot_hold_batch_past_its_deadline() {
    let fixture = Fixture::new();
    short_budget(&fixture);
    fixture.write("payload.sh", b"printf completed\n");
    fixture.document("large.md", "large", "expected");
    let (_unread, mut writer) = UnixStream::pair().unwrap();
    writer.set_nonblocking(true).unwrap();
    let mut filled = 0;
    loop {
        match writer.write(&[0u8; 65536]) {
            Ok(size) => {
                assert!(size > 0);
                filled += size;
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => panic!("cannot saturate fixture output: {error}"),
        }
    }
    assert!(filled > 0);
    writer.set_nonblocking(false).unwrap();
    let mut batch = OwnedBatch(
        fixture
            .command("1", &["large.md"])
            .stdin(Stdio::null())
            .stdout(Stdio::from(OwnedFd::from(writer)))
            .stderr(fs::File::create(fixture.0.join("stderr")).unwrap())
            .spawn()
            .unwrap(),
    );
    assert!(!batch.finish_within(Duration::from_secs(6)).success());
    let report = fixture.report();
    assert_eq!(report["documents"][0]["status"], 1);
    let session = &report["documents"][0]["session"];
    assert!(session["infrastructure_failure"].is_null(), "{report}");
    assert_eq!(session["cold_calls"], 1);
    assert_eq!(session["worker_starts"], 0);
    assert_eq!(session["author_status"]["exit_code"], 1);
    assert_eq!(session["application_statuses"][0]["exit_code"], 0);
    assert_eq!(report["cleanup"]["outcome"], "clean");
    assert_eq!(report["outcome"], "failure");
    assert!(report["infrastructure_failure"].is_string());
}

#[test]
fn publication_report_failure_cannot_write_to_an_already_saturated_stderr() {
    let fixture = Fixture::new();
    short_budget(&fixture);
    fixture.write("a.md", b"```sh\nset -eu\nset -- .mdrun-report-*\ntest \"$#\" -eq 1\nln \"$1\" report-attempt.json\nmkdir batch.json\n```\n");
    let (_unread, mut writer) = UnixStream::pair().unwrap();
    writer.set_nonblocking(true).unwrap();
    let chunk = [0u8; 65536];
    let mut filled = 0;
    loop {
        match writer.write(&chunk) {
            Ok(length) => filled += length,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
            result => panic!("cannot saturate diagnostic socket: {result:?}"),
        }
    }
    assert!(filled > 0);
    writer.set_nonblocking(false).unwrap();
    let mut batch = OwnedBatch(
        fixture
            .command("1", &["a.md"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(OwnedFd::from(writer)))
            .spawn()
            .unwrap(),
    );
    assert!(!batch.finish_within(Duration::from_secs(6)).success());
    assert!(fixture.0.join("batch.json").is_dir());
    let attempted: Value =
        serde_json::from_slice(&fs::read(fixture.0.join("report-attempt.json")).unwrap()).unwrap();
    assert_eq!(attempted["outcome"], "success");
    assert_eq!(attempted["cleanup"]["outcome"], "clean");
    assert_eq!(attempted["documents"][0]["status"], 0);
    assert_eq!(
        attempted["documents"][0]["session"]["author_status"]["exit_code"],
        0
    );
}
impl Drop for OwnedBatch {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

#[test]
fn two_slots_overlap_and_hold_queued_documents_until_cleanup() {
    let fixture = Fixture::new();
    for name in ["a", "b", "c"] {
        fixture.document(&format!("{name}.md"), name, name);
    }
    let mut batch = fixture.launch("2", &["a.md", "b.md", "c.md"]);
    fixture.wait_file("a.started");
    fixture.wait_file("b.started");
    assert!(!fixture.0.join("c.started").exists());
    fixture.write("a.go", b"");
    fixture.wait_file("c.started");
    assert!(batch.0.try_wait().unwrap().is_none());
    fixture.write("b.go", b"");
    fixture.write("c.go", b"");
    assert!(batch.finish().success());
    let report = fixture.report();
    assert_eq!(report["outcome"], "success");
    assert_eq!(
        report["concurrency"],
        json!({"requested":2,"effective":2,"share_resources":false})
    );
    assert_eq!(report["cleanup"]["outcome"], "clean");
    let documents = report["documents"].as_array().unwrap();
    assert_eq!(documents.len(), 3);
    for (index, entry) in documents.iter().enumerate() {
        assert_eq!(entry["index"], index);
        assert_eq!(entry["state"], "completed");
        assert_eq!(entry["session"]["cold_calls"], 1);
        assert_eq!(entry["session"]["worker_starts"], 0);
    }
    assert!(fs::read(fixture.0.join("stdout")).unwrap().is_empty());
}

#[test]
fn first_failure_stops_new_admission_and_drains_an_already_started_peer() {
    let fixture = Fixture::new();
    fixture.document("fail.md", "fail", "expected");
    fixture.document("healthy.md", "healthy", "healthy");
    fixture.document("queued.md", "queued", "queued");
    let mut batch = fixture.launch("2", &["fail.md", "healthy.md", "queued.md"]);
    fixture.wait_file("fail.started");
    fixture.wait_file("healthy.started");
    fixture.write("fail.go", b"");
    fixture.wait_stdout("unexpected");
    assert!(!fixture.0.join("queued.started").exists());
    fixture.write("healthy.go", b"");
    assert!(!batch.finish().success());
    assert!(!fixture.0.join("queued.started").exists());
    let report = fixture.report();
    assert_eq!(
        report["documents"][0]["session"]["author_status"]["exit_code"],
        1
    );
    assert_eq!(
        report["documents"][1]["session"]["author_status"]["exit_code"],
        0
    );
    assert_eq!(report["documents"][2]["state"], "not_started");
    assert_eq!(report["cleanup"]["outcome"], "clean");
    let output = fs::read_to_string(fixture.0.join("stdout")).unwrap();
    assert!(
        output.contains("fail.md") && output.contains("expected") && output.contains("unexpected")
    );
}

#[test]
fn invalid_batch_inputs_and_report_aliases_have_no_payload_effects() {
    let fixture = Fixture::new();
    fixture.document("a.md", "a", "a");
    let original = fs::read(fixture.0.join("a.md")).unwrap();
    fs::hard_link(fixture.0.join("a.md"), fixture.0.join("alias.json")).unwrap();
    let mut binding: Value =
        serde_json::from_slice(&fs::read(fixture.0.join("binding.json")).unwrap()).unwrap();
    binding["worker_argv"] = json!(["/bin/sh", "-c", "touch worker-started; exit 1"]);
    fixture.write("binding.json", serde_json::to_vec(&binding).unwrap());
    for arguments in [
        vec!["--binding", "binding.json", "--jobs", "0", "a.md"],
        vec!["--binding", "binding.json", "--output", "copy.md", "a.md"],
        vec![
            "--binding",
            "binding.json",
            "--mode",
            "cold",
            "a.md",
            "absent.md",
        ],
        vec!["--binding", "binding.json", "--report", "a.md", "a.md"],
        vec![
            "--binding",
            "binding.json",
            "--report",
            "alias.json",
            "a.md",
        ],
        vec![
            "--binding",
            "binding.json",
            "--report",
            "absent/report.json",
            "a.md",
        ],
    ] {
        let Output { status, .. } = Command::new(env!("CARGO_BIN_EXE_mdrun"))
            .current_dir(&fixture.0)
            .args(["verify-many"])
            .args(arguments)
            .output()
            .unwrap();
        assert!(!status.success());
        assert!(!fixture.0.join("a.started").exists());
        assert!(!fixture.0.join("worker-started").exists());
        assert_eq!(fs::read(fixture.0.join("a.md")).unwrap(), original);
        assert_eq!(fs::read(fixture.0.join("alias.json")).unwrap(), original);
    }
}

#[test]
fn declared_resources_serialize_unless_sharing_is_explicit_and_keep_one_batch_lease() {
    for share in [false, true] {
        let fixture = Fixture::new();
        for id in ["a", "b"] {
            fixture.document(&format!("{id}.md"), id, id);
        }
        let mut reservation = super::ports::Reservation::new();
        let address = reservation.address();
        let mut binding: Value =
            serde_json::from_slice(&fs::read(fixture.0.join("binding.json")).unwrap()).unwrap();
        binding["resources"] =
            json!([{"kind":"tcp_listener","host":"127.0.0.1","port":address.port()}]);
        fixture.write("binding.json", serde_json::to_vec(&binding).unwrap());
        let mut process = fixture.command_extra(
            "2",
            &["a.md", "b.md"],
            if share { &["--share-resources"] } else { &[] },
        );
        reservation.release_listener();
        let mut batch = OwnedBatch(
            process
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        fixture.wait_file("a.started");
        drop(reservation);
        assert_eq!(
            crate::ports::bind(address).unwrap_err().kind(),
            std::io::ErrorKind::AddrInUse
        );
        if share {
            fixture.wait_file("b.started");
        } else {
            assert!(!fixture.0.join("b.started").exists());
        }
        fixture.write("a.go", b"");
        fixture.wait_file("b.started");
        assert_eq!(
            crate::ports::bind(address).unwrap_err().kind(),
            std::io::ErrorKind::AddrInUse
        );
        fixture.write("b.go", b"");
        assert!(batch.finish().success());
        let report = fixture.report();
        assert_eq!(report["concurrency"]["requested"], 2);
        assert_eq!(
            report["concurrency"]["effective"],
            if share { 2 } else { 1 }
        );
        assert_eq!(report["concurrency"]["share_resources"], share);
        assert_eq!(report["cleanup"]["outcome"], "clean");
        for document in report["documents"].as_array().unwrap() {
            assert_eq!(
                document["session"]["cleanup"]["custodian_pid"],
                report["cleanup"]["custodian_pid"]
            );
        }
        assert!(crate::ports::bind(address).is_ok());
    }
}

#[test]
fn batch_stdin_is_not_shared_or_consumed_and_recorded_nonzero_remains_success() {
    let fixture = Fixture::new();
    fixture.document("a.md", "nonzero", "nonzero");
    fixture.document("b.md", "b", "b");
    fixture.write("nonzero.go", b"");
    fixture.write("b.go", b"");
    let (mut retained, mut writer) = UnixStream::pair().unwrap();
    writer.write_all(b"unread input").unwrap();
    let mut command = fixture.command("1", &["a.md", "b.md"]);
    let mut batch = OwnedBatch(
        command
            .stdin(Stdio::from(OwnedFd::from(retained.try_clone().unwrap())))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    assert!(batch.finish().success());
    retained
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    let mut unread = [0; 12];
    retained.read_exact(&mut unread).unwrap();
    assert_eq!(&unread, b"unread input");
    let report = fixture.report();
    assert_eq!(
        report["documents"][0]["session"]["application_statuses"][0]["exit_code"],
        17
    );
    assert_eq!(
        report["documents"][0]["session"]["author_status"]["exit_code"],
        0
    );
    assert_eq!(report["documents"][1]["state"], "completed");
}

#[test]
fn complete_diagnostic_groups_follow_input_order_and_reports_contain_no_command_bytes() {
    let fixture = Fixture::new();
    for id in ["a", "b"] {
        fixture.write(&format!("{id}.md"), format!("```sh\ntool {id}\n```\n\n```output\n{id}\n```\n\n```missing-{id}\nprivate command bytes\n```\n"));
    }
    let mut batch = fixture.launch("2", &["a.md", "b.md"]);
    fixture.wait_file("a.started");
    fixture.wait_file("b.started");
    fixture.write("b.go", b"");
    fixture.write("a.go", b"");
    assert!(!batch.finish().success());
    let stderr = fs::read_to_string(fixture.0.join("stderr")).unwrap();
    assert!(
        stderr.find("a.md").unwrap() < stderr.find("b.md").unwrap(),
        "{stderr}"
    );
    assert!(
        stderr.contains("missing-a") && stderr.contains("missing-b"),
        "{stderr}"
    );
    let report = fixture.report();
    assert_eq!(report["documents"].as_array().unwrap().len(), 2);
    assert!(!report.to_string().contains("private command bytes"));
}

#[test]
fn workdir_changes_execution_without_relocating_inputs_or_aggregate_report() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.0.join("work")).unwrap();
    fixture.write("work/marker", b"chosen workdir\n");
    fixture.write(
        "a.md",
        b"```sh\ncat marker\n```\n\n```output\nchosen workdir\n```\n",
    );
    let mut batch = OwnedBatch(
        fixture
            .command_extra("1", &["a.md"], &["--workdir", "work"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    assert!(batch.finish().success());
    assert!(fixture.0.join("batch.json").is_file());
    assert!(!fixture.0.join("work/batch.json").exists());
}

#[test]
fn preparation_failure_after_one_document_preserves_missing_report_and_queued_entries() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.0.join("work")).unwrap();
    fixture.write("a.md", b"```sh\nrmdir \"$PWD\"\n```\n");
    fixture.write("b.md", b"```sh\ntouch forbidden\n```\n");
    fixture.write("c.md", b"```sh\ntouch forbidden\n```\n");
    let mut batch = OwnedBatch(
        fixture
            .command_extra("1", &["a.md", "b.md", "c.md"], &["--workdir", "work"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    assert!(!batch.finish().success());
    let report = fixture.report();
    assert_eq!(report["documents"][0]["state"], "completed");
    assert_eq!(report["documents"][1]["state"], "failed");
    assert!(report["documents"][1].get("session").is_none());
    assert_eq!(report["documents"][2]["state"], "not_started");
    assert_eq!(report["cleanup"]["outcome"], "clean");
    assert!(!fixture.0.join("forbidden").exists());
}

#[test]
fn nested_batch_is_rejected_before_reading_session_credentials_or_starting_payloads() {
    let fixture = Fixture::new();
    fixture.document("a.md", "a", "a");
    let output = fixture
        .command("1", &["a.md"])
        .env("MDRUN_SESSION", "not-a-readable-configuration")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("nested managed sessions"));
    assert!(!fixture.0.join("a.started").exists());
}

#[test]
#[cfg(target_os = "linux")]
fn native_document_names_keep_execution_bytes_and_unambiguous_report_attribution() {
    let fixture = Fixture::new();
    let names = [
        b"collision-\xff.md".as_slice(),
        b"collision-\xfe.md".as_slice(),
    ]
    .map(|bytes| std::ffi::OsString::from_vec(bytes.to_vec()));
    let originals = ["a", "b"].map(|id| {
        fixture.write(&format!("{id}.go"), b"");
        format!("```sh\ntool {id}\n```\n\n```output\n{id}\n```\n")
    });
    for (name, content) in names.iter().zip(&originals) {
        fs::write(fixture.0.join(name), content).unwrap();
    }
    let mut batch = OwnedBatch(
        fixture
            .command("2", &[])
            .args(&names)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    assert!(batch.finish().success());
    let report = fixture.report();
    assert_eq!(
        report["documents"][0]["file"],
        report["documents"][1]["file"]
    );
    for (index, (name, content)) in names.iter().zip(&originals).enumerate() {
        let document = &report["documents"][index];
        assert_eq!(document["index"], index);
        assert_eq!(document["file_native_bytes"], json!(name.as_bytes()));
        assert_eq!(document["status"], 0);
        assert_eq!(document["session"]["cold_calls"], 1);
        assert_eq!(fs::read(fixture.0.join(name)).unwrap(), content.as_bytes());
    }
}

#[test]
fn unicode_spaces_and_leading_dash_filenames_are_literal_document_arguments() {
    let fixture = Fixture::new();
    let names = ["-δ document.md", "second é.md"];
    for (name, id) in names.iter().zip(["a", "b"]) {
        fixture.document(name, id, id);
        fixture.write(&format!("{id}.go"), b"");
    }
    let mut batch = fixture.launch("2", &names);
    assert!(batch.finish().success());
    let report = fixture.report();
    for (index, name) in names.iter().enumerate() {
        assert_eq!(report["documents"][index]["file"], *name);
        assert_eq!(report["documents"][index]["status"], 0);
        assert_eq!(report["documents"][index]["session"]["cold_calls"], 1);
        assert!(
            report["documents"][index]
                .get("file_native_bytes")
                .is_none()
        );
    }
}

struct OwnerLossCleanup<'a> {
    fixture: &'a Fixture,
    batch: OwnedBatch,
    address: std::net::SocketAddr,
    actors: Vec<super::session_owner_loss::Process>,
}

impl OwnerLossCleanup<'_> {
    fn status(&self) -> Option<Value> {
        fs::read_dir(&self.fixture.0).ok()?.find_map(|entry| {
            let path = entry.ok()?.path();
            if !path.file_name()?.as_bytes().starts_with(b"mdrun-batch-") {
                return None;
            }
            serde_json::from_slice(&fs::read(path.join("custody.status.json")).ok()?).ok()
        })
    }

    fn diagnostics(&self) -> String {
        let bind = crate::ports::bind(self.address)
            .map(|_| "available".to_owned())
            .unwrap_or_else(|error| format!("{error:?}"));
        let actors: Vec<_> = self
            .actors
            .iter()
            .map(|actor| (actor, super::session_owner_loss::observe(actor.pid)))
            .collect();
        let status = self.status();
        let anchors: Vec<_> = status
            .as_ref()
            .and_then(|state| state["groups"].as_array())
            .into_iter()
            .flatten()
            .filter_map(|group| group["anchor_pid"].as_i64())
            .map(|pid| (pid, super::session_owner_loss::observe(pid as i32)))
            .collect();
        format!(
            "status: {status:?}; listener {}: {bind}; actors: {actors:?}; anchors: {anchors:?}; fixture stderr: {:?}",
            self.address,
            fs::read_to_string(self.fixture.0.join("stderr"))
        )
    }

    fn wait_released(&self) {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            if self
                .status()
                .is_some_and(|state| state["status"] == "released")
                && !crate::ports::excluded(self.address)
            {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "deadline expired: whole-batch custody released; {}",
                self.diagnostics()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for OwnerLossCleanup<'_> {
    fn drop(&mut self) {
        self.fixture.write("owner.finish", b"");
        if self.batch.0.try_wait().ok().flatten().is_none() {
            let _ = self.batch.0.kill();
        }
        let _ = self.batch.0.wait();
        let deadline = Instant::now() + Duration::from_secs(4);
        loop {
            if self
                .status()
                .is_some_and(|status| status["status"] == "released")
                && !crate::ports::excluded(self.address)
                && self.actors.iter().all(|actor| !actor.alive())
            {
                break;
            }
            if Instant::now() >= deadline {
                self.fixture.1.set(true);
                eprintln!(
                    "batch owner-loss custody evidence retained: {}; {}",
                    self.fixture.0.display(),
                    self.diagnostics()
                );
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

#[test]
fn owner_loss_keeps_shared_exclusion_until_both_surviving_payload_groups_are_gone() {
    use super::session_owner_loss::{observe, wait_until};
    use crate::ports::excluded;
    let fixture = Fixture::new();
    fixture.write("payload.sh", b"id=$1\ntrap '' TERM HUP INT\nprintf '%s\\n' \"$$\" > \"$id.pid\"\n: > \"$id.started\"\nn=0\nwhile test ! -f owner.finish; do\n  if test \"$n\" -ge 1000; then : > \"$id.expired\"; exit 1; fi\n  n=$((n + 1))\n  sleep 0.01\ndone\n");
    for id in ["a", "b"] {
        fixture.document(&format!("{id}.md"), id, id);
    }
    let mut reservation = super::ports::Reservation::new();
    let address = reservation.address();
    let mut binding: Value =
        serde_json::from_slice(&fs::read(fixture.0.join("binding.json")).unwrap()).unwrap();
    binding["resources"] =
        json!([{"kind":"tcp_listener","host":"127.0.0.1","port":address.port()}]);
    fixture.write("binding.json", serde_json::to_vec(&binding).unwrap());
    reservation.release_listener();
    let batch = OwnedBatch(
        fixture
            .command_extra("2", &["a.md", "b.md"], &["--share-resources"])
            .env("TMPDIR", &fixture.0)
            .stdin(Stdio::null())
            .stdout(fs::File::create(fixture.0.join("stdout")).unwrap())
            .stderr(fs::File::create(fixture.0.join("stderr")).unwrap())
            .spawn()
            .unwrap(),
    );
    let mut cleanup = OwnerLossCleanup {
        fixture: &fixture,
        batch,
        address,
        actors: Vec::new(),
    };
    // Release the allocation gate before cleanup probes if readiness panics.
    let handoff_reservation = reservation;
    fixture.wait_file("a.started");
    fixture.wait_file("b.started");
    drop(handoff_reservation);
    cleanup.actors = ["a", "b"]
        .map(|id| {
            let pid = fs::read_to_string(fixture.0.join(format!("{id}.pid")))
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            observe(pid).unwrap().expect("ready payload must be live")
        })
        .into();
    assert_ne!(cleanup.actors[0].group, cleanup.actors[1].group);
    assert!(cleanup.actors.iter().all(|actor| actor.alive()));
    assert!(excluded(address));
    cleanup.batch.0.kill().unwrap();
    cleanup.batch.0.wait().unwrap();
    assert!(
        cleanup.actors.iter().any(|actor| actor.alive()),
        "no post-owner-loss survival window observed"
    );
    wait_until("both batch payload identities gone", || {
        let alive = cleanup.actors.iter().any(|actor| actor.alive());
        if alive {
            assert!(
                excluded(address),
                "listener released before surviving payload cleanup"
            );
        }
        !alive
    });
    cleanup.wait_released();
    assert!(!fixture.0.join("a.expired").exists());
    assert!(!fixture.0.join("b.expired").exists());
}
