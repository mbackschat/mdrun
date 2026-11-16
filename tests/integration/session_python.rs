use super::{Fixture, PARITY_CALLS, parity_mode_fixture, run_parity_author};
use serde_json::{Value, json};
use std::ffi::OsStr;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const OPEN_INPUT_AUTHOR: &str = "MDRUN_PYTHON_OPEN_INPUT_AUTHOR";

struct OwnedClient(Child);

impl Drop for OwnedClient {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

fn binding() -> Value {
    let python = PathBuf::from(std::env::var_os("MDRUN_PYTHON").expect(
        "set MDRUN_PYTHON to the prepared absolute Python executable for runtime qualification",
    ));
    assert!(python.is_absolute() && python.is_file());
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    json!({
        "version":1,"name":"tool","artifact_id":"python-sdk-fixture-v1",
        "worker_argv":[python,root.join("tests/runtime/python/fixture_worker.py")],
        "cold_argv":[python,root.join("tests/runtime/python/cold_app.py")],
        "context":{"environment":{"LC_ALL":"C","LANG":"C","PYTHONCOERCECLOCALE":"0","PYTHONUTF8":"1","PYTHONDONTWRITEBYTECODE":"1"}},
        "startup_timeout_ms":5000,"session_timeout_ms":20000,"cleanup_timeout_ms":2000
    })
}

fn fixture() -> Fixture {
    let fixture = Fixture::new();
    fixture.write("binding.json", serde_json::to_vec(&binding()).unwrap());
    fixture
}

#[test]
#[ignore = "requires a prepared Python runtime selected by MDRUN_PYTHON"]
fn packaged_cold_matches_required_warm_native_author() {
    let binding = binding();
    let root = Fixture::new();
    let cold = parity_mode_fixture(&root, "cold", &binding);
    let fast = parity_mode_fixture(&root, "fast", &binding);
    let expected_statuses = [0, 17, 0, 0, 0, 0, 0];
    let echo_input = fs::read(cold.0.join("echo.input")).unwrap();
    let expected_stdout: [&[u8]; 7] = [
        b"ok",
        b"",
        &[0, 255, 1, 2],
        b"before application input changes\n",
        b"after application input changes\0\xff\n",
        &echo_input,
        b"ok",
    ];
    for (fixture, options, warm_calls, cold_calls, worker_starts) in [
        (&cold, &["--mode", "cold"][..], 0, 7, 0),
        (&fast, &["--mode", "fast", "--require-warm"][..], 7, 0, 1),
    ] {
        let output = run_parity_author(fixture, options);
        assert!(
            output.status.success(),
            "native author failed: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        for (index, call) in PARITY_CALLS.iter().enumerate() {
            assert_eq!(
                fs::read(fixture.0.join(format!("{call}.stdout"))).unwrap(),
                expected_stdout[index],
                "stdout for {call}"
            );
            let expected_stderr: &[u8] = if index == 2 { &[254, 0] } else { b"" };
            assert_eq!(
                fs::read(fixture.0.join(format!("{call}.stderr"))).unwrap(),
                expected_stderr,
                "stderr for {call}"
            );
            assert_eq!(
                fs::read_to_string(fixture.0.join(format!("{call}.status"))).unwrap(),
                format!("{}\n", expected_statuses[index]),
                "status for {call}"
            );
        }
        let report = fixture.report();
        assert_eq!(report["artifact_id"], binding["artifact_id"]);
        assert_eq!(report["warm_calls"], warm_calls);
        assert_eq!(report["cold_calls"], cold_calls);
        assert_eq!(report["worker_starts"], worker_starts);
        if cold_calls != 0 {
            assert_eq!(report["cold_reasons"]["explicit_cold"], cold_calls);
        } else {
            assert_eq!(report["cold_reasons"], json!({}));
        }
        let statuses = report["application_statuses"].as_array().unwrap();
        assert_eq!(statuses.len(), expected_statuses.len());
        for (actual, expected) in statuses.iter().zip(expected_statuses) {
            assert_eq!(actual["exit_code"], expected);
            assert_eq!(actual["signal"], Value::Null);
            assert_eq!(
                actual["route"],
                if cold_calls == 0 { "warm" } else { "cold" }
            );
        }
        assert_eq!(report["author_status"]["exit_code"], 0);
        assert_eq!(report["outcome"], "success");
        assert_eq!(report["infrastructure_failure"], Value::Null);
        assert_eq!(report["cleanup"]["outcome"], "clean");
    }
    for call in PARITY_CALLS {
        for suffix in ["stdout", "stderr", "status"] {
            let file = format!("{call}.{suffix}");
            assert_eq!(
                fs::read(cold.0.join(&file)).unwrap(),
                fs::read(fast.0.join(&file)).unwrap(),
                "packaged cold/warm parity for {file}"
            );
        }
    }
}

#[test]
#[ignore = "requires a prepared Python runtime selected by MDRUN_PYTHON"]
fn refusal_runs_cold_once_and_require_warm_prevents_the_effect() {
    for required in [false, true] {
        let fixture = fixture();
        let options: &[&str] = if required { &["--require-warm"] } else { &[] };
        let output = fixture.run(options, &["tool", "refuse", "effect"]);
        assert_eq!(output.status.code(), Some(if required { 1 } else { 23 }));
        assert_eq!(
            output.stdout,
            if required {
                &b""[..]
            } else {
                &b"cold refusal\n"[..]
            }
        );
        if required {
            assert!(!fixture.0.join("effect").exists());
        } else {
            assert_eq!(
                fs::read(fixture.0.join("effect")).unwrap(),
                b"cold refusal executed\n"
            );
        }
        let report = fixture.report();
        assert_eq!(report["warm_calls"], 0);
        assert_eq!(report["cold_calls"], if required { 0 } else { 1 });
        assert_eq!(report["worker_starts"], 1);
        if !required {
            assert_eq!(report["cold_reasons"]["unsupported_command"], 1);
            assert_eq!(report["infrastructure_failure"], Value::Null);
        }
        assert_eq!(report["cleanup"]["outcome"], "clean");
    }
}

#[test]
#[ignore = "requires a prepared Python runtime selected by MDRUN_PYTHON"]
fn caught_invalidation_and_retained_operations_fail_the_session() {
    for command in [
        "invalidate_caught",
        "late_stdout",
        "late_input",
        "late_invalidate",
        "late_atexit",
    ] {
        let fixture = fixture();
        let output = fixture.run(&["--require-warm"], &["tool", command]);
        assert!(!output.status.success(), "{command}");
        assert!(output.stdout.is_empty(), "{command}");
        let report = fixture.report();
        assert_eq!(report["warm_calls"], 1, "{command}");
        assert_eq!(report["cold_calls"], 0, "{command}");
        assert_eq!(report["worker_starts"], 1, "{command}");
        assert_eq!(report["outcome"], "failure", "{command}");
        assert_eq!(report["cleanup"]["outcome"], "clean", "{command}");
        assert!(
            report["infrastructure_failure"]
                .as_str()
                .is_some_and(|reason| !reason.is_empty())
        );
        if command == "invalidate_caught" {
            assert_eq!(report["application_statuses"], json!([]));
        } else {
            assert_eq!(report["author_status"]["exit_code"], 0);
            let statuses = report["application_statuses"].as_array().unwrap();
            assert_eq!(statuses.len(), 1);
            assert_eq!(statuses[0]["exit_code"], 0);
            assert_eq!(statuses[0]["signal"], Value::Null);
            assert_eq!(statuses[0]["route"], "warm");
        }
    }
}

#[test]
#[ignore = "requires a prepared Python runtime selected by MDRUN_PYTHON"]
fn unused_open_stdin_finishes_without_waiting_for_eof() {
    let fixture = fixture();
    let mut command = Command::new(env!("CARGO_BIN_EXE_mdrun"));
    command
        .args(["session", "--binding"])
        .arg(fixture.0.join("binding.json"))
        .arg("--report")
        .arg(fixture.0.join("report.json"))
        .args(["--require-warm", "--"])
        .arg(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "session::python::open_input_author_fixture",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .current_dir(&fixture.0);
    let output =
        fixture.capture_with_environment(&mut command, &[(OPEN_INPUT_AUTHOR, OsStr::new("1"))]);
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read(fixture.0.join("ignored.stdout")).unwrap(), b"ok");
    assert!(
        fs::read(fixture.0.join("ignored.stderr"))
            .unwrap()
            .is_empty()
    );
    let report = fixture.report();
    assert_eq!(report["warm_calls"], 1);
    assert_eq!(report["cold_calls"], 0);
    assert_eq!(report["worker_starts"], 1);
    assert_eq!(report["infrastructure_failure"], Value::Null);
    assert_eq!(report["cleanup"]["outcome"], "clean");
}

#[test]
#[ignore = "native author helper; no-op without its managed-session marker"]
fn open_input_author_fixture() {
    if std::env::var_os(OPEN_INPUT_AUTHOR).as_deref() != Some(OsStr::new("1")) {
        return;
    }
    let mut child = OwnedClient(
        Command::new("tool")
            .arg("ignore")
            .stdin(Stdio::piped())
            .stdout(fs::File::create("ignored.stdout").unwrap())
            .stderr(fs::File::create("ignored.stderr").unwrap())
            .spawn()
            .unwrap(),
    );
    let mut input = child.0.stdin.take().unwrap();
    input.write_all(b"input remains open and unused").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            panic!("ignored-input invocation waited for open stdin EOF");
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(status.success());
    drop(input);
}
