use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

#[path = "session_context.rs"]
mod context;

#[path = "session_batch.rs"]
mod batch;

#[path = "session_node.rs"]
mod node_parity;

#[path = "session_python.rs"]
mod python;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("mdrun-session-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn write(&self, file: &str, bytes: impl AsRef<[u8]>) -> PathBuf {
        let path = self.0.join(file);
        fs::write(&path, bytes).unwrap();
        path
    }

    fn binding(&self, worker: Value, cold: Value) {
        self.write(
            "binding.json",
            serde_json::to_vec(&json!({
                "version":1,"name":"tool","artifact_id":"test-artifact",
                "worker_argv":worker,"cold_argv":cold,
                "startup_timeout_ms":5000,"session_timeout_ms":20000,"cleanup_timeout_ms":2000
            }))
            .unwrap(),
        );
    }

    fn run(&self, extra: &[&str], command: &[&str]) -> Output {
        self.run_in(&self.0, extra, command)
    }

    fn run_in(&self, directory: &Path, extra: &[&str], command: &[&str]) -> Output {
        let mut process = Command::new(env!("CARGO_BIN_EXE_mdrun"));
        process
            .args(["session", "--binding"])
            .arg(self.0.join("binding.json"))
            .arg("--report")
            .arg(self.0.join("report.json"))
            .args(extra)
            .arg("--")
            .args(command)
            .current_dir(directory);
        self.capture(process)
    }

    fn capture(&self, mut command: Command) -> Output {
        self.capture_with_environment(&mut command, &[])
    }

    fn capture_with_environment(
        &self,
        command: &mut Command,
        environment: &[(&str, &std::ffi::OsStr)],
    ) -> Output {
        let stdout = fs::File::create(self.0.join("stdout")).unwrap();
        let stderr = fs::File::create(self.0.join("stderr")).unwrap();
        command.env_clear().env("PATH", "/usr/bin:/bin");
        if cfg!(target_os = "macos")
            && let Some(node) = std::env::var_os("MDRUN_NODE")
        {
            // macOS Node rewrites this at startup; qualify a baseline observed from the selected runtime.
            static ENCODING: OnceLock<std::ffi::OsString> = OnceLock::new();
            let encoding = ENCODING.get_or_init(|| {
                use std::os::unix::ffi::OsStringExt;
                let output = Command::new(node)
                    .env_clear()
                    .env("PATH", "/usr/bin:/bin")
                    .args(["-p", "process.env.__CF_USER_TEXT_ENCODING ?? ''"])
                    .output()
                    .unwrap();
                assert!(output.status.success());
                std::ffi::OsString::from_vec(output.stdout.strip_suffix(b"\n").unwrap().to_vec())
            });
            command.env("__CF_USER_TEXT_ENCODING", encoding);
        }
        let mut child = command
            .envs(environment.iter().copied())
            .stdin(Stdio::null())
            .stdout(stdout)
            .stderr(stderr)
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!(
                    "session exceeded its external deadline: {}",
                    String::from_utf8_lossy(&fs::read(self.0.join("stderr")).unwrap())
                );
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        Output {
            status,
            stdout: fs::read(self.0.join("stdout")).unwrap(),
            stderr: fs::read(self.0.join("stderr")).unwrap(),
        }
    }

    fn report(&self) -> Value {
        serde_json::from_slice(&fs::read(self.0.join("report.json")).unwrap()).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn node() -> PathBuf {
    PathBuf::from(std::env::var_os("MDRUN_NODE").expect(
        "set MDRUN_NODE to the prepared absolute Node executable for runtime qualification",
    ))
}

#[test]
fn explicit_cold_starts_no_worker_and_preserves_the_application_status() {
    let fixture = Fixture::new();
    fixture.binding(
        json!(["/not/a/prepared/worker"]),
        json!(["/bin/sh", "-c", "printf 'cold bytes'; exit 7"]),
    );
    let output = fixture.run(
        &["--mode", "cold"],
        &["tool", "--ordinary-application-flag"],
    );
    assert_eq!(
        output.status.code(),
        Some(7),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"cold bytes");
    let report = fixture.report();
    assert_eq!(report["version"], 1);
    assert_eq!(
        report["tool"],
        json!({"name":"mdrun", "version":env!("CARGO_PKG_VERSION")})
    );
    assert_eq!(report["binding"], json!({"name":"tool"}));
    assert_eq!(report["artifact_id"], "test-artifact");
    assert_eq!(report["worker_starts"], 0);
    assert_eq!(report["warm_calls"], 0);
    assert_eq!(report["cold_calls"], 1);
    assert_eq!(report["cold_reasons"]["explicit_cold"], 1);
    assert_eq!(report["application_statuses"][0]["exit_code"], 7);
    assert_eq!(report["infrastructure_failure"], Value::Null);
    assert_eq!(report["cleanup"]["outcome"], "clean");
}

#[test]
fn invalid_binding_and_require_warm_cold_conflict_start_no_payload() {
    let fixture = Fixture::new();
    fixture.binding(json!(["relative-worker"]), json!(["/bin/sh"]));
    let output = fixture.run(&[], &["/bin/sh", "-c", "touch forbidden"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("absolute"));
    assert!(!fixture.0.join("forbidden").exists());
    fixture.binding(json!(["/not/a/prepared/worker"]), json!(["/bin/sh"]));
    let output = fixture.run(
        &["--mode", "cold", "--require-warm"],
        &["/bin/sh", "-c", "touch forbidden"],
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("warm execution is required"));
    assert!(!fixture.0.join("forbidden").exists());
}

#[test]
fn nested_managed_session_fails_before_starting_its_payload() {
    let fixture = Fixture::new();
    fixture.binding(json!(["/not/a/prepared/worker"]), json!(["/bin/true"]));
    let output = fixture.run(
        &["--mode", "cold"],
        &[
            env!("CARGO_BIN_EXE_mdrun"),
            "session",
            "--binding",
            "binding.json",
            "--mode",
            "cold",
            "--",
            "/bin/sh",
            "-c",
            "touch forbidden",
        ],
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("nested managed sessions"));
    assert!(!fixture.0.join("forbidden").exists());
}

#[test]
fn bound_verify_keeps_document_paths_at_the_call_site_and_executes_in_workdir() {
    let fixture = Fixture::new();
    fixture.binding(
        json!(["/not/a/prepared/worker"]),
        json!(["/bin/cat", "marker"]),
    );
    fs::create_dir(fixture.0.join("work")).unwrap();
    fixture.write("work/marker", b"selected workdir\n");
    let document =
        b"# Existing recording\n\n```sh\ntool\n```\n\n```output\nselected workdir\n```\n";
    fixture.write("document.md", document);
    let mut command = Command::new(env!("CARGO_BIN_EXE_mdrun"));
    command.current_dir(&fixture.0).args([
        "verify",
        "document.md",
        "--binding",
        "binding.json",
        "--mode",
        "cold",
        "--report",
        "report.json",
        "--workdir",
        "work",
        "--output",
        "copy.md",
    ]);
    let output = fixture.capture(command);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    assert_eq!(fs::read(fixture.0.join("document.md")).unwrap(), document);
    assert_eq!(fs::read(fixture.0.join("copy.md")).unwrap(), document);
    assert!(!fixture.0.join("work/copy.md").exists());
    assert_eq!(fixture.report()["cold_calls"], 1);
    assert_eq!(fixture.report()["worker_starts"], 0);
}

#[test]
fn long_temporary_directory_does_not_change_the_application_environment() {
    use std::os::unix::ffi::OsStrExt;
    let fixture = Fixture::new();
    fixture.binding(
        json!(["/not/a/prepared/worker"]),
        json!(["/usr/bin/printenv", "TMPDIR"]),
    );
    let directory = fixture.0.join("x".repeat(100));
    fs::create_dir(&directory).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_mdrun"));
    command.current_dir(&fixture.0).args([
        "session",
        "--binding",
        "binding.json",
        "--mode",
        "cold",
        "--",
        "tool",
    ]);
    let output =
        fixture.capture_with_environment(&mut command, &[("TMPDIR", directory.as_os_str())]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut expected = directory.as_os_str().as_bytes().to_vec();
    expected.push(b'\n');
    assert_eq!(output.stdout, expected);
}

#[test]
#[cfg(target_os = "linux")]
fn native_directory_encoding_selects_cold_and_preserves_the_actual_directory() {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    let fixture = Fixture::new();
    fixture.binding(json!(["/not/a/prepared/worker"]), json!(["/bin/pwd"]));
    let directory = fixture
        .0
        .join(std::ffi::OsString::from_vec(b"directory-\xff".to_vec()));
    fs::create_dir(&directory).unwrap();
    let output = fixture.run_in(&directory, &[], &["tool"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut expected = fs::canonicalize(&directory)
        .unwrap()
        .as_os_str()
        .as_bytes()
        .to_vec();
    expected.push(b'\n');
    assert_eq!(output.stdout, expected);
    assert_eq!(fixture.report()["cold_reasons"]["native_encoding"], 1);
    assert_eq!(fixture.report()["worker_starts"], 0);
}

#[test]
#[ignore = "requires a prepared Node runtime selected by MDRUN_NODE"]
fn managed_author_reuses_one_worker_after_an_application_failure() {
    let fixture = Fixture::new();
    let node = node();
    let worker =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/runtime/node/fixture-worker.mjs");
    fixture.binding(
        json!([node, worker]),
        json!(["/bin/sh", "-c", "printf 'unexpected cold'; exit 99"]),
    );
    let driver = fixture.write(
        "author.mjs",
        r#"import {spawnSync} from 'node:child_process';
for (const [command,status] of [['ignore',0],['fail',17],['ignore',0]]) {
  const result=spawnSync('tool',[command]);
  process.stdout.write(result.stdout); process.stderr.write(result.stderr);
  if(result.status!==status) process.exit(88);
}
"#,
    );
    let output = fixture.run(
        &["--require-warm"],
        &[node.to_str().unwrap(), driver.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"okok");
    let report = fixture.report();
    assert_eq!(report["worker_starts"], 1);
    assert_eq!(report["warm_calls"], 3);
    assert_eq!(report["cold_calls"], 0);
    assert_eq!(report["application_statuses"][1]["exit_code"], 17);
    assert_eq!(report["outcome"], "success");
    assert_eq!(report["cleanup"]["outcome"], "clean");
}

#[test]
#[ignore = "requires a prepared Node runtime selected by MDRUN_NODE"]
fn changed_environment_routes_independent_cold_and_require_warm_prevents_it() {
    let fixture = Fixture::new();
    let node = node();
    let worker =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/runtime/node/fixture-worker.mjs");
    fixture.binding(
        json!([node, worker]),
        json!(["/bin/sh", "-c", "printf '%s' \"$CHANGED\""]),
    );
    let driver = fixture.write("author.mjs", r#"import {spawnSync} from 'node:child_process';
const result=spawnSync('tool',['ignore'],{env:{...process.env,CHANGED:'fresh value'}});
process.stdout.write(result.stdout);process.stderr.write(result.stderr);process.exit(result.status??99);
"#);
    let command = [node.to_str().unwrap(), driver.to_str().unwrap()];
    let output = fixture.run(&[], &command);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"fresh value");
    assert_eq!(fixture.report()["cold_reasons"]["environment"], 1);
    let output = fixture.run(&["--require-warm"], &command);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(fixture.report()["cold_calls"], 0);
}

#[test]
#[ignore = "requires a prepared Node runtime selected by MDRUN_NODE"]
fn worker_refusal_runs_cold_once_then_reuses_the_same_worker() {
    let fixture = Fixture::new();
    let node = node();
    let worker =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/runtime/node/fixture-worker.mjs");
    fixture.binding(
        json!([node, worker]),
        json!([
            "/bin/sh",
            "-c",
            "printf cold; printf 'effect\\n' >> effects"
        ]),
    );
    let driver = fixture.write(
        "author.mjs",
        r#"import {spawnSync} from 'node:child_process';
for (const command of ['refuse','ignore']) {
  const result=spawnSync('tool',[command]);
  process.stdout.write(result.stdout);process.stderr.write(result.stderr);
  if(result.status!==0) process.exit(88);
}
"#,
    );
    let output = fixture.run(&[], &[node.to_str().unwrap(), driver.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"coldok");
    assert_eq!(fs::read(fixture.0.join("effects")).unwrap(), b"effect\n");
    let report = fixture.report();
    assert_eq!(report["cold_calls"], 1);
    assert_eq!(report["cold_reasons"]["unsupported_command"], 1);
    assert_eq!(report["warm_calls"], 1);
    assert_eq!(report["worker_starts"], 1);
}

#[test]
#[ignore = "requires a prepared Node runtime selected by MDRUN_NODE"]
fn late_sdk_output_after_shutdown_fails_the_session_despite_successful_calls() {
    let fixture = Fixture::new();
    let node = node();
    let worker =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/runtime/node/fixture-worker.mjs");
    fixture.binding(json!([node, worker]), json!(["/bin/sh", "-c", "exit 99"]));
    let driver = fixture.write(
        "author.mjs",
        r#"import {spawnSync} from 'node:child_process';
const result=spawnSync('tool',['late_shutdown_stdout']);
process.exit(result.status??99);
"#,
    );
    let output = fixture.run(
        &["--require-warm"],
        &[node.to_str().unwrap(), driver.to_str().unwrap()],
    );
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let report = fixture.report();
    assert_eq!(report["author_status"]["exit_code"], 0);
    assert_eq!(report["application_statuses"][0]["exit_code"], 0);
    assert_eq!(report["warm_calls"], 1);
    assert_eq!(
        report["infrastructure_failure"],
        "worker orderly shutdown failed"
    );
    assert_eq!(report["cleanup"]["outcome"], "clean");
}

#[test]
#[ignore = "requires a prepared Node runtime selected by MDRUN_NODE"]
fn lost_acceptance_never_retries_effects_even_when_the_author_masks_failure() {
    let fixture = Fixture::new();
    let node = node();
    let worker = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/runtime/node/lost-acceptance-worker.mjs");
    fixture.binding(
        json!([node, worker]),
        json!(["/bin/sh", "-c", "printf 'cold\\n' >> \"$1\"", "--"]),
    );
    let effect = fixture.0.join("effects");
    let driver = fixture.write(
        "author.mjs",
        r#"import {spawnSync} from 'node:child_process';
spawnSync('tool',[process.argv[2]]);
process.exit(0);
"#,
    );
    let output = fixture.run(
        &[],
        &[
            node.to_str().unwrap(),
            driver.to_str().unwrap(),
            effect.to_str().unwrap(),
        ],
    );
    assert!(!output.status.success());
    assert_eq!(fs::read(effect).unwrap(), b"warm\n");
    let report = fixture.report();
    assert_eq!(report["cold_calls"], 0);
    assert_eq!(report["warm_calls"], 1);
    assert!(report["infrastructure_failure"].is_string());
    assert_eq!(report["cleanup"]["outcome"], "clean");
}

#[test]
#[ignore = "requires a prepared Node runtime selected by MDRUN_NODE"]
fn ready_worker_remains_usable_after_the_startup_deadline() {
    let fixture = Fixture::new();
    let node = node();
    let worker =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/runtime/node/fixture-worker.mjs");
    fixture.binding(json!([node, worker]), json!(["/bin/sh", "-c", "exit 99"]));
    let mut binding: Value =
        serde_json::from_slice(&fs::read(fixture.0.join("binding.json")).unwrap()).unwrap();
    binding["startup_timeout_ms"] = json!(2000);
    fixture.write("binding.json", serde_json::to_vec(&binding).unwrap());
    let driver = fixture.write("author.mjs", r#"import {spawnSync} from 'node:child_process';
await new Promise(resolve=>setTimeout(resolve,2500));
const result=spawnSync('tool',['ignore']);
process.stdout.write(result.stdout);process.stderr.write(result.stderr);process.exit(result.status??99);
"#);
    let output = fixture.run(
        &["--require-warm"],
        &[node.to_str().unwrap(), driver.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"ok");
    assert_eq!(fixture.report()["warm_calls"], 1);
}

#[test]
#[ignore = "requires a prepared Node runtime selected by MDRUN_NODE"]
fn warm_streams_preserve_large_binary_data_and_do_not_wait_for_unused_stdin() {
    let fixture = Fixture::new();
    let node = node();
    let worker =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/runtime/node/fixture-worker.mjs");
    fixture.binding(json!([node, worker]), json!(["/bin/sh", "-c", "exit 99"]));
    let driver = fixture.write(
        "author.mjs",
        r#"import assert from 'node:assert/strict';
import {spawn,spawnSync} from 'node:child_process';
const input=Buffer.alloc(3*1024*1024+17);
for(let i=0;i<input.length;i++) input[i]=i%256;
const echo=spawnSync('tool',['echo'],{input,maxBuffer:8*1024*1024});
assert.equal(echo.status,0);assert.deepEqual(echo.stdout,input);assert.equal(echo.stderr.length,0);
const binary=spawnSync('tool',['binary']);
assert.equal(binary.status,0);assert.deepEqual(binary.stdout,Buffer.from([0,255,1,2]));
assert.deepEqual(binary.stderr,Buffer.from([254,0]));
const ignored=spawn('tool',['ignore'],{stdio:['pipe','pipe','pipe']});
ignored.stdin.on('error',()=>{});
ignored.stdin.write('input remains open');
const chunks=[];ignored.stdout.on('data',bytes=>chunks.push(bytes));
await new Promise((resolve,reject)=>{
  ignored.once('error',reject);
  ignored.once('exit',(code,signal)=>{
    ignored.stdin.destroy();
    try {assert.equal(code,0);assert.equal(signal,null);resolve();} catch(error){reject(error);}
  });
});
assert.equal(Buffer.concat(chunks).toString(),'ok');
"#,
    );
    let output = fixture.run(
        &["--require-warm"],
        &[node.to_str().unwrap(), driver.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fixture.report()["warm_calls"], 3);
    assert_eq!(fixture.report()["cold_calls"], 0);
}

const PARITY_AUTHOR_MARKER: &str = "MDRUN_PARITY_AUTHOR_FIXTURE";

fn jvm_shell_binding(fixture: &Fixture, ignore: bool) {
    let prepared = PathBuf::from(std::env::var_os("MDRUN_JVM_FIXTURE").unwrap());
    let manifest: Value = serde_json::from_slice(&fs::read(prepared).unwrap()).unwrap();
    let mut binding = manifest["binding"].clone();
    binding["artifact_id"] = json!(format!("shell-qualification-{}", uuid::Uuid::new_v4()));
    binding["context"] = json!({"environment": {
        "PWD": fixture.0.to_str().unwrap(), "SHLVL": "4", "_": "original-baseline"
    }});
    if ignore {
        binding["warm_ignore_env"] = json!(["SHLVL", "_"]);
    }
    fixture.write("binding.json", serde_json::to_vec(&binding).unwrap());
}

const JVM_SHELL_SCRIPT: &str = r#"set -e
tool ignore; printf '\n'
printf 'piped input\n' | tool echo
/bin/sh -c 'tool ignore; printf "\n"'
if tool fail; then exit 99; else test "$?" -eq 17; fi
tool ignore; printf '\n'
"#;
const JVM_SHELL_OUTPUT: &[u8] = b"ok\npiped input\nok\nok\n";

#[test]
#[ignore = "requires independently prepared JVM fixtures selected by MDRUN_JVM_FIXTURE"]
fn jvm_shell_equivalence_is_opt_in_and_matches_independent_cold() {
    for (ignore, mode, warm, cold, starts) in [
        (false, "fast", 0, 5, 1),
        (true, "fast", 5, 0, 1),
        (true, "cold", 0, 5, 0),
    ] {
        let fixture = Fixture::new();
        jvm_shell_binding(&fixture, ignore);
        let options = if ignore && mode == "fast" {
            vec!["--mode", mode, "--require-warm"]
        } else {
            vec!["--mode", mode]
        };
        let output = fixture.run(&options, &["/bin/bash", "-c", JVM_SHELL_SCRIPT]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, JVM_SHELL_OUTPUT);
        assert!(output.stderr.is_empty());
        let report = fixture.report();
        assert_eq!(report["warm_calls"], warm);
        assert_eq!(report["cold_calls"], cold);
        assert_eq!(report["worker_starts"], starts);
        if !ignore {
            assert_eq!(report["cold_reasons"]["environment"], 5);
        }
        assert_eq!(report["application_statuses"][3]["exit_code"], 17);
        assert_eq!(report["infrastructure_failure"], Value::Null);
        assert_eq!(report["cleanup"]["outcome"], "clean");
    }
}

#[test]
#[ignore = "requires independently prepared JVM fixtures selected by MDRUN_JVM_FIXTURE"]
fn jvm_shell_equivalence_verifies_an_unchanged_bash_recording() {
    let fixture = Fixture::new();
    jvm_shell_binding(&fixture, true);
    let document = format!(
        "# Existing recording\n\n```bash\n{JVM_SHELL_SCRIPT}```\n\n```output\n{}```\n",
        String::from_utf8_lossy(JVM_SHELL_OUTPUT)
    );
    fixture.write("document.md", &document);
    let mut command = Command::new(env!("CARGO_BIN_EXE_mdrun"));
    command.current_dir(&fixture.0).args([
        "verify",
        "document.md",
        "--binding",
        "binding.json",
        "--require-warm",
        "--report",
        "report.json",
    ]);
    let output = fixture.capture(command);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    assert_eq!(
        fs::read_to_string(fixture.0.join("document.md")).unwrap(),
        document
    );
    assert_eq!(fixture.report()["warm_calls"], 5);
    assert_eq!(fixture.report()["cold_calls"], 0);
    assert_eq!(fixture.report()["worker_starts"], 1);
}

#[test]
#[ignore = "requires independently prepared JVM fixtures selected by MDRUN_JVM_FIXTURE"]
fn jvm_shell_equivalence_keeps_observing_handlers_cold_with_original_values() {
    for (ignore, reason) in [(false, "environment"), (true, "observes_shell_environment")] {
        let fixture = Fixture::new();
        jvm_shell_binding(&fixture, ignore);
        let output = fixture.run(
            &[],
            &[
                "/usr/bin/env",
                "SHLVL=77",
                "_=explicit-value",
                "tool",
                "environment",
            ],
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"77\nexplicit-value\n");
        assert!(output.stderr.is_empty());
        let report = fixture.report();
        assert_eq!(report["warm_calls"], 0);
        assert_eq!(report["cold_calls"], 1);
        assert_eq!(report["cold_reasons"][reason], 1);
        assert_eq!(report["cleanup"]["outcome"], "clean");

        let required = fixture.run(
            &["--require-warm"],
            &[
                "/usr/bin/env",
                "SHLVL=77",
                "_=explicit-value",
                "tool",
                "environment",
            ],
        );
        assert!(!required.status.success());
        assert!(required.stdout.is_empty());
        assert_eq!(fixture.report()["warm_calls"], 0);
        assert_eq!(fixture.report()["cold_calls"], 0);
    }
}

const PARITY_CALLS: [&str; 7] = [
    "00-ignore",
    "01-fail",
    "02-binary",
    "03-read-file",
    "04-read-file-changed",
    "05-echo",
    "06-ignore",
];

fn parity_mode_fixture(root: &Fixture, mode: &str, binding: &Value) -> Fixture {
    let fixture = Fixture(root.0.join(mode));
    fs::create_dir(&fixture.0).unwrap();
    fixture.write("binding.json", serde_json::to_vec(binding).unwrap());
    let input: Vec<u8> = (0..512 * 1024 + 17)
        .map(|index| (index % 256) as u8)
        .collect();
    fixture.write("echo.input", input);
    fixture
}

fn run_parity_author(fixture: &Fixture, options: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_mdrun"));
    command
        .args(["session", "--binding"])
        .arg(fixture.0.join("binding.json"))
        .arg("--report")
        .arg(fixture.0.join("report.json"))
        .args(options)
        .arg("--")
        .arg(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "session::parity_author_fixture",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .current_dir(&fixture.0);
    fixture.capture_with_environment(
        &mut command,
        &[(PARITY_AUTHOR_MARKER, std::ffi::OsStr::new("1"))],
    )
}

fn jvm_manifest() -> Value {
    let prepared = PathBuf::from(
        std::env::var_os("MDRUN_JVM_FIXTURE")
            .expect("set MDRUN_JVM_FIXTURE to the prepared absolute fixture-manifest.json"),
    );
    assert!(prepared.is_absolute());
    let manifest: Value = serde_json::from_slice(&fs::read(prepared).unwrap()).unwrap();
    assert_eq!(manifest["schema_version"], 1);
    manifest
}

#[test]
#[ignore = "requires independently prepared JVM fixtures selected by MDRUN_JVM_FIXTURE"]
fn jvm_packaged_cold_matches_required_warm_native_author() {
    let manifest = jvm_manifest();
    let binding = &manifest["binding"];
    assert_eq!(binding["name"], "tool");
    let cold_argv = binding["cold_argv"].as_array().unwrap();
    assert_eq!(cold_argv.len(), 4);
    assert_eq!(cold_argv[1], "-cp");
    let cold_classpath = cold_argv[2].as_str().unwrap();
    let cold_paths: Vec<_> = std::env::split_paths(cold_classpath).collect();
    assert_eq!(
        cold_paths.len(),
        1,
        "packaged cold must load no SDK or worker jars"
    );
    assert!(cold_paths[0].is_absolute());
    assert_eq!(cold_paths[0].file_name().unwrap(), "cold.jar");
    assert_eq!(cold_argv[3], "io.github.mbackschat.mdrun.fixture.ColdMain");

    let root = Fixture::new();
    let cold = parity_mode_fixture(&root, "cold", binding);
    let fast = parity_mode_fixture(&root, "fast", binding);
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
        }
        let statuses = report["application_statuses"].as_array().unwrap();
        assert_eq!(statuses.len(), expected_statuses.len());
        for (actual, expected) in statuses.iter().zip(expected_statuses) {
            assert_eq!(actual["exit_code"], expected);
            assert_eq!(actual["signal"], Value::Null);
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
#[ignore = "requires independently prepared JVM fixtures selected by MDRUN_JVM_FIXTURE"]
fn jvm_late_output_after_shutdown_fails_the_session_despite_successful_call() {
    let manifest = jvm_manifest();
    let fixture = Fixture::new();
    fixture.write(
        "binding.json",
        serde_json::to_vec(&manifest["binding"]).unwrap(),
    );
    let output = fixture.run(&["--require-warm"], &["tool", "late_stdout"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let report = fixture.report();
    assert_eq!(report["author_status"]["exit_code"], 0);
    assert_eq!(report["application_statuses"][0]["exit_code"], 0);
    assert_eq!(report["warm_calls"], 1);
    assert_eq!(report["cold_calls"], 0);
    assert_eq!(report["worker_starts"], 1);
    assert_eq!(
        report["infrastructure_failure"],
        "worker orderly shutdown failed"
    );
    assert_eq!(report["cleanup"]["outcome"], "clean");
}

#[test]
#[ignore = "requires independently prepared JVM fixtures selected by MDRUN_JVM_FIXTURE"]
fn jvm_adapter_invalidation_fails_the_session_even_when_caught() {
    let manifest = jvm_manifest();
    for command in [
        "invalidate_caught",
        "late_invalidate",
        "late_invalidate_hook",
    ] {
        let fixture = Fixture::new();
        fixture.write(
            "binding.json",
            serde_json::to_vec(&manifest["binding"]).unwrap(),
        );
        let output = fixture.run(&["--require-warm"], &["tool", command]);
        assert!(!output.status.success(), "{command}");
        assert!(output.stdout.is_empty(), "{command}");
        let report = fixture.report();
        assert_eq!(report["warm_calls"], 1, "{command}");
        assert_eq!(report["cold_calls"], 0, "{command}");
        assert_eq!(report["worker_starts"], 1, "{command}");
        assert_eq!(report["outcome"], "failure", "{command}");
        assert_eq!(report["cleanup"]["outcome"], "clean", "{command}");
        if command == "invalidate_caught" {
            assert_eq!(report["application_statuses"], json!([]));
            assert!(
                report["infrastructure_failure"]
                    .as_str()
                    .is_some_and(|reason| !reason.is_empty())
            );
        } else {
            assert_eq!(report["author_status"]["exit_code"], 0);
            assert_eq!(report["application_statuses"][0]["exit_code"], 0);
            assert_eq!(
                report["infrastructure_failure"],
                "worker orderly shutdown failed"
            );
        }
    }
}

#[test]
#[ignore = "native author helper; no-op without its managed-session marker"]
fn parity_author_fixture() {
    if std::env::var_os(PARITY_AUTHOR_MARKER).as_deref() != Some(std::ffi::OsStr::new("1")) {
        return;
    }
    for (index, (arguments, expected_status)) in [
        (vec!["ignore"], 0),
        (vec!["fail"], 17),
        (vec!["binary"], 0),
        (vec!["read_file", "mutable.input"], 0),
        (vec!["read_file", "mutable.input"], 0),
        (vec!["echo"], 0),
        (vec!["ignore"], 0),
    ]
    .into_iter()
    .enumerate()
    {
        if index == 3 {
            fs::write("mutable.input", b"before application input changes\n").unwrap();
        } else if index == 4 {
            fs::write("mutable.input", b"after application input changes\0\xff\n").unwrap();
        }
        let call = PARITY_CALLS[index];
        let input = if index == 5 {
            Stdio::from(fs::File::open("echo.input").unwrap())
        } else {
            Stdio::null()
        };
        let status = Command::new("tool")
            .args(arguments)
            .stdin(input)
            .stdout(fs::File::create(format!("{call}.stdout")).unwrap())
            .stderr(fs::File::create(format!("{call}.stderr")).unwrap())
            .status()
            .unwrap();
        fs::write(
            format!("{call}.status"),
            format!(
                "{}\n",
                status.code().expect("application terminated by signal")
            ),
        )
        .unwrap();
        assert_eq!(status.code(), Some(expected_status), "status for {call}");
    }
}
