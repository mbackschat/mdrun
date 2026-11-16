use serde_json::{Value, json};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "mdrun-discovery-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_mdrun"));
        command
            .current_dir(&self.0)
            .env_clear()
            .env("PATH", "/usr/bin:/bin");
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }

    fn inventory(&self, args: &[&str]) -> Value {
        let output = self.run(args);
        assert_success(&output);
        assert!(output.stderr.is_empty());
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn write(&self, name: &str, bytes: &[u8]) {
        fs::write(self.0.join(name), bytes).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "status {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn inventory_uses_replay_indices_and_includes_unpaired_legacy_commands() {
    let fixture = Fixture::new();
    let source = b"# Legacy\n\n*2020-01-01 by Showboat 0.6.1*\n<!-- showboat-id: old -->\n\n## Setup\nProse\n\n```sh\nprintf 'first\\n'\n```\n\n```output\nstale\n```\n\n```bash {image}\nmissing.png\n```\n\n![Picture](missing.png)\n\n## Later\n\n```sh\nprintf 'second\\n' > replayed\n```\n";
    fixture.write("legacy.md", source);

    let inventory = fixture.inventory(&["list", "legacy.md", "--json"]);
    assert_eq!(
        inventory,
        json!([
            {
                "file": "legacy.md",
                "block": 2,
                "line": 9,
                "heading": "Setup",
                "interpreter": "sh",
                "summary": "printf 'first\\n'"
            },
            {
                "file": "legacy.md",
                "block": 7,
                "line": 25,
                "heading": "Later",
                "interpreter": "sh",
                "summary": "printf 'second\\n' > replayed"
            }
        ])
    );
    assert!(!fixture.0.join("replayed").exists());
    let plain = fixture.run(&["list", "legacy.md"]);
    assert_success(&plain);
    let plain = String::from_utf8(plain.stdout).unwrap();
    for text in ["legacy.md", "Setup", "Later", "sh", "printf 'first\\n'"] {
        assert!(plain.contains(text), "missing {text:?} in {plain:?}");
    }
    assert!(!plain.contains("missing.png"));
    assert_eq!(fs::read(fixture.0.join("legacy.md")).unwrap(), source);

    let verified = fixture.run(&["verify", "legacy.md"]);
    assert_eq!(verified.status.code(), Some(1));
    assert_eq!(
        verified.stdout,
        b"block 2:\n  expected: stale\n  actual:   first\n"
    );
    assert_eq!(inventory[0]["block"], 2);
    assert_eq!(fs::read(fixture.0.join("replayed")).unwrap(), b"second\n");
    assert_eq!(fs::read(fixture.0.join("legacy.md")).unwrap(), source);
}

#[test]
fn filtering_is_case_sensitive_literal_text_across_file_heading_and_complete_command() {
    let fixture = Fixture::new();
    fixture.write(
        "filename-match.md",
        b"# Title\n\n```sh\nprintf first\nprintf 'Needle[.*]'\n```\n\n## Heading-match\n\n```sh\n# Pretend heading\nprintf second\n```\n",
    );
    fixture.write("other.md", b"```python\nprint('third')\n```\n");

    let full = fixture.inventory(&["list", "filename-match.md", "other.md", "--json"]);
    assert_eq!(full.as_array().unwrap().len(), 3);
    assert_eq!(full[0]["heading"], "Title");
    assert_eq!(full[1]["heading"], "Heading-match");
    assert_eq!(full[2]["heading"], Value::Null);
    let command = fixture.inventory(&[
        "list",
        "filename-match.md",
        "other.md",
        "--json",
        "--filter",
        "Needle[.*]",
    ]);
    assert_eq!(command, json!([full[0]]));
    assert_eq!(command[0]["summary"], "printf first");
    let heading = fixture.inventory(&[
        "list",
        "filename-match.md",
        "other.md",
        "--filter",
        "Heading-match",
        "--json",
    ]);
    assert_eq!(heading, json!([full[1]]));
    let file = fixture.inventory(&[
        "list",
        "filename-match.md",
        "other.md",
        "--filter",
        "filename-match",
        "--json",
    ]);
    assert_eq!(file, json!([full[0], full[1]]));
    for text in ["needle[.*]", "Needle.*", "does not occur"] {
        assert_eq!(
            fixture.inventory(&[
                "list",
                "filename-match.md",
                "other.md",
                "--filter",
                text,
                "--json"
            ]),
            json!([])
        );
    }
}

#[test]
fn preview_preserves_full_fenced_source_bytes_without_evaluation_or_stdin() {
    let fixture = Fixture::new();
    fixture.write("worker", b"#!/bin/sh\ntouch worker-started\n");
    fs::set_permissions(fixture.0.join("worker"), fs::Permissions::from_mode(0o755)).unwrap();
    let span = b"````./worker\r\nprintf '%s' \"$(touch expanded) $UNSET `touch backticks`\"\r\n```\r\n\xff\0\r\n````\r\n";
    let source = [b"# Raw\r\n\r\n".as_slice(), span, b"\r\nuntouched\r\n"].concat();
    fixture.write("raw.md", &source);

    let mut command = fixture.command();
    command
        .args(["preview", "raw.md", "1"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    let input = child.stdin.take().unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let completed_without_stdin = loop {
        if child.try_wait().unwrap().is_some() {
            break true;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            break false;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    drop(input);
    let output = child.wait_with_output().unwrap();
    assert!(
        completed_without_stdin,
        "preview consumed or waited for stdin"
    );
    assert_success(&output);
    assert_eq!(output.stdout, span);
    assert!(output.stderr.is_empty());
    for name in ["worker-started", "expanded", "backticks"] {
        assert!(!fixture.0.join(name).exists());
    }
    assert_eq!(fs::read(fixture.0.join("raw.md")).unwrap(), source);
}

#[test]
fn discovery_does_not_start_interpreters_and_rejects_non_executable_block_locations() {
    let fixture = Fixture::new();
    fixture.write("worker", b"#!/bin/sh\ntouch worker-started\n");
    fs::set_permissions(fixture.0.join("worker"), fs::Permissions::from_mode(0o755)).unwrap();
    let source = b"# Safe\n\n```./worker\ntouch command-ran; echo $(touch expanded)\n```\n\n```output\n# Not a heading\n```\n\n```bash {image}\nmissing.png\n```\n\n![Picture](missing.png)\n";
    fixture.write("safe.md", source);
    let entries = fixture.inventory(&["list", "safe.md", "--json"]);
    assert_eq!(entries.as_array().unwrap().len(), 1);
    assert_eq!(entries[0]["block"], 1);
    assert_eq!(entries[0]["heading"], "Safe");
    assert_success(&fixture.run(&["preview", "safe.md", "1"]));

    for index in ["0", "2", "3", "4", "99"] {
        let output = fixture.run(&["preview", "safe.md", index]);
        assert_eq!(output.status.code(), Some(1), "index {index}");
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("block"));
    }
    for args in [
        vec!["list", "missing.md", "--json"],
        vec!["preview", "missing.md", "1"],
    ] {
        let output = fixture.run(&args);
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("missing.md"));
    }
    for args in [
        vec!["list"],
        vec!["preview", "safe.md"],
        vec!["preview", "safe.md", "invalid"],
        vec!["preview", "safe.md", "1", "extra"],
    ] {
        let output = fixture.run(&args);
        assert_eq!(output.status.code(), Some(2), "args {args:?}");
        assert!(output.stdout.is_empty());
    }
    for name in ["worker-started", "command-ran", "expanded"] {
        assert!(!fixture.0.join(name).exists());
    }
    assert_eq!(fs::read(fixture.0.join("safe.md")).unwrap(), source);
}

#[test]
fn json_inventory_escapes_metadata_and_reports_every_file_before_emitting() {
    let fixture = Fixture::new();
    let filename = "quote\"\\\t-ü.md";
    fixture.write(
        filename,
        b"## A \"quoted\" \\ heading\n\n```shell \"name\"\necho \"quoted\"\t\\suffix\n```\n",
    );
    let entries = fixture.inventory(&["list", filename, "--json"]);
    assert_eq!(
        entries,
        json!([{
            "file": filename,
            "block": 1,
            "line": 3,
            "heading": "A \"quoted\" \\ heading",
            "interpreter": "shell \"name\"",
            "summary": "echo \"quoted\"\t\\suffix"
        }])
    );
    let failed = fixture.run(&["list", filename, "absent.md", "--json"]);
    assert_eq!(failed.status.code(), Some(1));
    assert!(failed.stdout.is_empty());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("absent.md"));
}

#[test]
fn invalid_utf8_metadata_fails_clearly_while_preview_keeps_native_bytes() {
    let fixture = Fixture::new();
    fixture.write("valid.md", b"```sh\n:\n```\n");
    for (name, source) in [
        (
            "heading.md",
            b"# Invalid \xff\n\n```sh\n:\n```\n".as_slice(),
        ),
        ("language.md", b"```sh\xff\n:\n```\n".as_slice()),
        ("summary.md", b"```sh\necho \xff\n```\n".as_slice()),
    ] {
        fixture.write(name, source);
        let output = fixture.run(&["list", "valid.md", name, "--json"]);
        assert_eq!(output.status.code(), Some(1), "file {name}");
        assert!(output.stdout.is_empty(), "partial inventory for {name}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("UTF-8"),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let native_name = OsString::from_vec(b"native-\xff.md".to_vec());
    let output = fixture
        .command()
        .args([OsStr::new("list"), &native_name, OsStr::new("--json")])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("UTF-8"));
    let output = fixture.run(&["preview", "summary.md", "0"]);
    assert_success(&output);
    assert_eq!(output.stdout, b"```sh\necho \xff\n```\n");
}

#[cfg(target_os = "linux")]
#[test]
fn preview_accepts_native_non_utf8_filenames_without_changing_source() {
    let fixture = Fixture::new();
    let native_name = OsString::from_vec(b"native-\xff.md".to_vec());
    let source = b"```sh\necho \xfe\n```";
    fs::write(fixture.0.join(&native_name), source).unwrap();
    let output = fixture
        .command()
        .args([OsStr::new("preview"), &native_name, OsStr::new("0")])
        .output()
        .unwrap();
    assert_success(&output);
    assert_eq!(output.stdout, source);
}
