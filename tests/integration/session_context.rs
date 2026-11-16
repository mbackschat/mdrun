use super::{Fixture, node};
use serde_json::{Value, json};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

enum ExpectedOutput {
    Exact(Vec<u8>),
    Line(Vec<u8>),
}

struct Probe {
    label: &'static str,
    argv: Vec<OsString>,
    cwd: PathBuf,
    environment: Vec<(OsString, OsString)>,
    umask: Option<libc::mode_t>,
    stdout: ExpectedOutput,
}

impl Probe {
    fn run(&self, managed: bool, effect: &Path) -> Output {
        let mut command = Command::new(if managed { "tool" } else { "/usr/bin/env" });
        command.args(&self.argv).current_dir(&self.cwd);
        command.envs(self.environment.iter().map(|(key, value)| (key, value)));
        if self.umask.is_some() {
            command.arg(effect);
        }
        if !managed {
            command
                .env_remove("MDRUN_SESSION")
                .env("PATH", "/usr/bin:/bin");
        }
        if let Some(mask) = self.umask {
            unsafe {
                command.pre_exec(move || {
                    libc::umask(mask);
                    Ok(())
                });
            }
        }
        command.stdin(Stdio::null()).output().unwrap()
    }
}

fn arguments(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

#[test]
#[ignore = "native author entry point for the runtime context matrix"]
fn native_author() {
    if std::env::var_os("MDRUN_CONTEXT_AUTHOR").is_none() {
        return;
    }
    let root = std::env::current_dir().unwrap();
    let changed = fs::canonicalize(root.join("changed")).unwrap();
    let warm = || {
        let output = Command::new("tool")
            .arg("ignore")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"ok");
        assert!(output.stderr.is_empty());
    };
    warm();
    let native_argument = OsString::from_vec(b"argument-\xff-\xfe".to_vec());
    let native_value = OsString::from_vec(b"value-\xfd-\xff".to_vec());
    let native_key = OsString::from_vec(b"MDRUN_KEY_\xfe".to_vec());
    let mut cwd_output = changed.as_os_str().as_bytes().to_vec();
    cwd_output.push(b'\n');
    let mut argv_output = native_argument.as_bytes().to_vec();
    argv_output.push(b'\n');
    let mut env_output = native_value.as_bytes().to_vec();
    env_output.push(b'\n');
    let probes = [
        Probe {
            label: "cwd",
            argv: arguments(&["/bin/pwd", "-P"]),
            cwd: changed,
            environment: vec![],
            umask: None,
            stdout: ExpectedOutput::Exact(cwd_output),
        },
        Probe {
            label: "pwd",
            argv: arguments(&["/usr/bin/printenv", "PWD"]),
            cwd: root.clone(),
            environment: vec![("PWD".into(), "/deliberately/logical/changed".into())],
            umask: None,
            stdout: ExpectedOutput::Exact(b"/deliberately/logical/changed\n".to_vec()),
        },
        Probe {
            label: "umask",
            argv: arguments(&["/usr/bin/touch"]),
            cwd: root.clone(),
            environment: vec![],
            umask: Some(0o077),
            stdout: ExpectedOutput::Exact(vec![]),
        },
        Probe {
            label: "startup",
            argv: arguments(&["/usr/bin/printenv", "NODE_OPTIONS"]),
            cwd: root.clone(),
            environment: vec![("NODE_OPTIONS".into(), "--no-warnings".into())],
            umask: None,
            stdout: ExpectedOutput::Exact(b"--no-warnings\n".to_vec()),
        },
        Probe {
            label: "native_argv",
            argv: vec!["/usr/bin/printf".into(), "%s\n".into(), native_argument],
            cwd: root.clone(),
            environment: vec![],
            umask: None,
            stdout: ExpectedOutput::Exact(argv_output),
        },
        Probe {
            label: "native_value",
            argv: arguments(&["/usr/bin/printenv", "MDRUN_NATIVE"]),
            cwd: root.clone(),
            environment: vec![("MDRUN_NATIVE".into(), native_value)],
            umask: None,
            stdout: ExpectedOutput::Exact(env_output),
        },
        Probe {
            label: "native_key",
            argv: arguments(&["/usr/bin/env"]),
            cwd: root.clone(),
            environment: vec![(native_key, "native key preserved".into())],
            umask: None,
            stdout: ExpectedOutput::Line(b"MDRUN_KEY_\xfe=native key preserved".to_vec()),
        },
    ];
    for probe in probes {
        let direct_effect = root.join(format!("{}-direct", probe.label));
        let managed_effect = root.join(format!("{}-managed", probe.label));
        let direct = probe.run(false, &direct_effect);
        let managed = probe.run(true, &managed_effect);
        assert_eq!(
            direct.status.code(),
            Some(0),
            "{} independent launcher",
            probe.label
        );
        assert_eq!(managed.status, direct.status, "{} status", probe.label);
        assert_eq!(
            managed.stdout, direct.stdout,
            "{} managed bytes",
            probe.label
        );
        match probe.stdout {
            ExpectedOutput::Exact(expected) => {
                assert_eq!(direct.stdout, expected, "{} independent bytes", probe.label)
            }
            ExpectedOutput::Line(expected) => assert!(
                direct
                    .stdout
                    .split(|byte| *byte == b'\n')
                    .any(|line| line == expected),
                "{} raw environment key",
                probe.label
            ),
        }
        assert!(
            direct.stderr.is_empty(),
            "{}: {:?}",
            probe.label,
            direct.stderr
        );
        assert!(
            managed.stderr.is_empty(),
            "{}: {:?}",
            probe.label,
            managed.stderr
        );
        if probe.umask.is_some() {
            for effect in [direct_effect, managed_effect] {
                assert!(fs::read(&effect).unwrap().is_empty());
                assert_eq!(
                    fs::metadata(&effect).unwrap().permissions().mode() & 0o777,
                    0o600
                );
            }
        }
    }
    warm();
}

#[test]
#[ignore = "requires a prepared Node runtime selected by MDRUN_NODE"]
fn unsupported_context_preserves_native_cold_behavior_between_warm_calls() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.0.join("changed")).unwrap();
    let worker =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/runtime/node/fixture-worker.mjs");
    fixture.binding(json!([node(), worker]), json!(["/usr/bin/env"]));
    let path = fixture.0.join("binding.json");
    let mut binding: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    binding["cold_if_env_nonempty"] = json!(["NODE_OPTIONS"]);
    binding["context"] = json!({"umask":18,"environment":{"PWD":"/deliberately/logical/baseline"}});
    fs::write(path, serde_json::to_vec(&binding).unwrap()).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_mdrun"));
    command
        .args([
            "session",
            "--binding",
            "binding.json",
            "--report",
            "report.json",
            "--",
        ])
        .arg(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "session::context::native_author",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .current_dir(&fixture.0);
    let output = fixture
        .capture_with_environment(&mut command, &[("MDRUN_CONTEXT_AUTHOR", OsStr::new("1"))]);
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let report = fixture.report();
    assert_eq!(report["warm_calls"], 2);
    assert_eq!(report["cold_calls"], 7);
    assert_eq!(report["worker_starts"], 1);
    assert_eq!(
        report["cold_reasons"],
        json!({"cwd":1,"environment":1,"umask":1,"startup_options":1,"native_encoding":3})
    );
    let statuses = report["application_statuses"].as_array().unwrap();
    assert_eq!(statuses.len(), 9);
    assert!(
        statuses
            .iter()
            .all(|status| status["exit_code"] == 0 && status["signal"].is_null())
    );
    assert!(report["infrastructure_failure"].is_null());
    assert_eq!(report["cleanup"]["outcome"], "clean");
}
