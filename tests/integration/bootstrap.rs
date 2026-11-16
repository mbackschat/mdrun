use std::ffi::OsStr;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "mdrun-parser-{}-{}",
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

    fn run_native(&self, args: &[&OsStr]) -> Output {
        self.command().args(args).output().unwrap()
    }

    fn stdin(&self, args: &[&str], input: &[u8]) -> Output {
        let mut child = self
            .command()
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(input).unwrap();
        child.wait_with_output().unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mdrun"))
        .args(args)
        .env_clear()
        .output()
        .expect("run standalone mdrun")
}

#[test]
fn absent_or_empty_executable_argument_uses_cli_diagnostics_without_session_forwarding() {
    use std::ffi::CString;
    use std::os::unix::process::CommandExt;

    for absent in [true, false] {
        for session_marker in [false, true] {
            let binary = CString::new(env!("CARGO_BIN_EXE_mdrun")).unwrap();
            let empty = CString::new("").unwrap();
            let marker = CString::new("MDRUN_SESSION=/nonexistent-md-run-session").unwrap();
            let mut command = Command::new(env!("CARGO_BIN_EXE_mdrun"));
            command.env_clear();
            unsafe {
                command.pre_exec(move || {
                    let argv = [
                        if absent {
                            std::ptr::null()
                        } else {
                            empty.as_ptr()
                        },
                        std::ptr::null(),
                    ];
                    let environment = [
                        if session_marker {
                            marker.as_ptr()
                        } else {
                            std::ptr::null()
                        },
                        std::ptr::null(),
                    ];
                    libc::execve(binary.as_ptr(), argv.as_ptr(), environment.as_ptr());
                    Err(std::io::Error::last_os_error())
                });
            }
            let output = command.output().unwrap();
            let diagnostic = String::from_utf8(output.stderr).unwrap();
            assert_eq!(
                output.status.code(),
                Some(2),
                "absent={absent}, session_marker={session_marker}: {diagnostic}"
            );
            assert!(output.stdout.is_empty());
            assert!(diagnostic.contains("Usage:"), "{diagnostic}");
            assert!(!diagnostic.contains("panicked"), "{diagnostic}");
        }
    }
}

#[test]
fn version_identifies_mdrun_without_a_target_runtime() {
    let output = run(&["--version"]);
    assert!(output.status.success());
    assert_eq!(
        output.stdout,
        format!("mdrun {}\n", env!("CARGO_PKG_VERSION")).as_bytes()
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn help_exposes_document_commands_and_global_options() {
    for args in [&["--help"][..], &["-h"], &["help"]] {
        let output = run(args);
        assert!(output.status.success());
        let help = String::from_utf8(output.stdout).unwrap();
        assert!(help.contains("Create, run, and verify executable Markdown documents."));
        assert!(help.contains("Usage: mdrun"));
        assert!(help.contains("Commands:"));
        for command in ["init", "note", "exec", "image", "pop", "verify", "extract"] {
            assert!(
                help.lines()
                    .any(|line| line.split_whitespace().next() == Some(command))
            );
        }
        assert!(help.contains("--workdir <DIR>"));
        assert!(output.stderr.is_empty());
    }
}

#[test]
fn offline_guides_match_the_maintained_sources_without_creating_files() {
    let fixture = Fixture::new();
    for (topic, expected) in [
        (None, include_str!("../../docs/guides/README.md")),
        (
            Some("documents"),
            include_str!("../../docs/guides/documents.md"),
        ),
        (
            Some("sessions"),
            include_str!("../../docs/guides/sessions.md"),
        ),
        (
            Some("reference"),
            include_str!("../../docs/guides/reference.md"),
        ),
    ] {
        let mut args = vec!["guide"];
        args.extend(topic);
        let output = fixture.run(&args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, expected.as_bytes());
        assert!(output.stderr.is_empty());
    }
    let invalid = fixture.run(&["guide", "unknown-topic"]);
    assert_eq!(invalid.status.code(), Some(2));
    assert!(invalid.stdout.is_empty());
    assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), 0);
}

#[test]
fn short_help_stays_compact_while_long_help_supplies_a_working_recipe() {
    let short = run(&["-h"]);
    let long = run(&["--help"]);
    assert!(short.status.success() && long.status.success());
    let short = String::from_utf8(short.stdout).unwrap();
    let long = String::from_utf8(long.stdout).unwrap();
    assert!(short.len() < long.len());
    assert!(!short.contains("A repeatable greeting"));
    assert!(long.contains("mdrun init demo.md 'A repeatable greeting'"));
    assert!(long.contains("mdrun verify demo.md"));
    assert!(!long.contains("__custodian"));
    assert!(!long.contains("__anchor"));
}

#[test]
fn root_help_recipe_records_literal_output_and_replays_without_edits() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.0.join("bin")).unwrap();
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_mdrun"), fixture.0.join("bin/mdrun")).unwrap();
    let help = String::from_utf8(run(&["--help"]).stdout).unwrap();
    let lines: Vec<_> = help
        .lines()
        .filter_map(|line| line.strip_prefix("    mdrun "))
        .map(|line| format!("mdrun {line}"))
        .collect();
    assert_eq!(
        lines.len(),
        4,
        "the quick start must remain one finite recipe"
    );
    let output = Command::new("/bin/sh")
        .args(["-eu", "-c", &lines.join("\n")])
        .current_dir(&fixture.0)
        .env_clear()
        .env(
            "PATH",
            format!("{}:/usr/bin:/bin", fixture.0.join("bin").display()),
        )
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"hello from mdrun\n");
    let recorded = fs::read(fixture.0.join("demo.md")).unwrap();
    assert!(String::from_utf8_lossy(&recorded).contains("```output\nhello from mdrun\n```"));
    assert!(fixture.run(&["verify", "demo.md"]).status.success());
    assert_eq!(fs::read(fixture.0.join("demo.md")).unwrap(), recorded);
}

#[test]
fn readme_recipe_and_recording_match_then_expose_changed_output_without_editing_input() {
    let fixture = Fixture::new();
    let readme = include_str!("../../README.md");
    let recording = readme
        .split_once("## Record your first demonstration\n")
        .expect("README must retain the first-recording walkthrough")
        .1
        .split("\n## ")
        .next()
        .unwrap();
    let recipe = recording
        .split_once("```sh\n")
        .unwrap()
        .1
        .split_once("```")
        .unwrap()
        .0;
    assert_eq!(recipe.lines().count(), 4);
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_mdrun"), fixture.0.join("mdrun")).unwrap();
    let output = Command::new("/bin/sh")
        .args(["-eu", "-c", recipe])
        .current_dir(&fixture.0)
        .env_clear()
        .env("PATH", format!("{}:/usr/bin:/bin", fixture.0.display()))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"hello from mdrun\n");
    let excerpt = recording
        .split_once("````markdown\n")
        .unwrap()
        .1
        .split_once("````")
        .unwrap()
        .0;
    let recorded = fs::read_to_string(fixture.0.join("demo.md")).unwrap();
    let retained = include_str!("../../docs/examples/greeting.md");
    assert_eq!(excerpt, retained, "README must show the complete recording");
    let (recorded_title, recorded_rest) = recorded.split_once("\n\n").unwrap();
    let (retained_title, retained_rest) = retained.split_once("\n\n").unwrap();
    assert_eq!(recorded_title, retained_title);
    // Each init generates its own timestamp and ID; compare all authored content.
    assert_eq!(
        recorded_rest.split_once("\n\n").unwrap().1,
        retained_rest.split_once("\n\n").unwrap().1
    );
    fs::write(fixture.0.join("retained.md"), retained).unwrap();
    assert!(fixture.run(&["verify", "retained.md"]).status.success());

    let changed = recorded.replace(
        "printf \"hello from mdrun\\n\"",
        "printf \"changed greeting\\n\"",
    );
    assert_ne!(changed, recorded);
    fs::write(fixture.0.join("demo.md"), &changed).unwrap();
    let mismatch = fixture.run(&["verify", "demo.md"]);
    assert_eq!(mismatch.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&mismatch.stdout).contains("changed greeting"));
    assert_eq!(
        fs::read_to_string(fixture.0.join("demo.md")).unwrap(),
        changed
    );
    let updated = fixture.run(&["verify", "demo.md", "--output", "candidate.md"]);
    assert_eq!(updated.status.code(), Some(1));
    assert_eq!(
        fs::read_to_string(fixture.0.join("demo.md")).unwrap(),
        changed
    );
    let candidate = fs::read_to_string(fixture.0.join("candidate.md")).unwrap();
    assert!(candidate.contains("```output\nchanged greeting\n```"));
    assert!(fixture.run(&["verify", "candidate.md"]).status.success());

    let multiline = readme
        .split_once("```sh\ncat <<'CODE'")
        .unwrap()
        .1
        .split_once("```")
        .unwrap()
        .0;
    let output = Command::new("/bin/sh")
        .args(["-eu", "-c", &format!("cat <<'CODE'{multiline}")])
        .current_dir(&fixture.0)
        .env_clear()
        .env("PATH", format!("{}:/usr/bin:/bin", fixture.0.display()))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"first\nsecond\n");
}

#[test]
fn command_help_uses_each_commands_arguments_and_options() {
    for command in ["init", "note", "exec", "image", "pop", "verify", "extract"] {
        let output = run(&[command, "--help"]);
        assert!(output.status.success());
        let help = String::from_utf8(output.stdout).unwrap();
        assert!(help.contains(&format!("Usage: mdrun {command}")));
        assert!(help.contains("<FILE>"));
        assert!(help.contains("--workdir <DIR>"));
        if command == "verify" {
            assert!(help.contains("--output <FILE>"));
        }
        if command == "extract" {
            assert!(help.contains("--filename <NAME>"));
        }
        assert!(output.stderr.is_empty());
        assert_eq!(run(&["help", command]).stdout, help.as_bytes());
    }
}

#[test]
fn missing_unknown_and_extra_arguments_use_standard_parser_errors_without_effects() {
    let cases: &[&[&str]] = &[
        &[],
        &["unknown"],
        &["--unknown"],
        &["init"],
        &["init", "doc.md"],
        &["exec", "doc.md"],
        &["init", "doc.md", "Title", "extra"],
        &["note", "doc.md", "Text", "extra"],
        &["exec", "doc.md", "sh", ":", "extra"],
        &["image", "doc.md", "source.png", "extra"],
        &["pop", "doc.md", "extra"],
        &["verify", "doc.md", "extra"],
        &["extract", "doc.md", "extra"],
        &["note", "doc.md", "--unknown"],
        &["--workdir"],
        &["verify", "doc.md", "--output"],
        &["extract", "doc.md", "--filename"],
        &["session"],
        &["session", "--binding", "binding.json"],
        &[
            "session",
            "--binding",
            "binding.json",
            "--unknown",
            "--",
            "tool",
        ],
        &[
            "session",
            "--binding",
            "binding.json",
            "--mode",
            "invalid",
            "--",
            "tool",
        ],
        &["verify", "doc.md", "--mode", "cold"],
        &["verify", "doc.md", "--report", "report.json"],
        &["verify", "doc.md", "--require-warm"],
        &["__custodian"],
        &["__anchor", "unexpected"],
    ];
    for args in cases {
        let fixture = Fixture::new();
        let output = fixture.run(args);
        assert_eq!(output.status.code(), Some(2), "args: {args:?}");
        assert!(output.stdout.is_empty(), "args: {args:?}");
        let error = String::from_utf8(output.stderr).unwrap();
        if args.is_empty() {
            assert!(error.contains("Usage: mdrun"), "stderr: {error}");
        } else {
            assert!(
                error.starts_with("error:"),
                "args: {args:?}, stderr: {error}"
            );
            assert!(error.contains("--help"), "args: {args:?}, stderr: {error}");
        }
        assert!(fs::read_dir(&fixture.0).unwrap().next().is_none());
    }
}

#[test]
fn end_of_options_preserves_literal_hyphen_arguments() {
    let fixture = Fixture::new();
    let init = fixture.run(&["init", "--", "-doc.md", "-Title"]);
    assert!(init.status.success(), "{:?}", init.stderr);
    let note = fixture.run(&["note", "--", "-doc.md", "--literal-note"]);
    assert!(note.status.success(), "{:?}", note.stderr);
    let source = fs::read(fixture.0.join("-doc.md")).unwrap();
    assert!(source.starts_with(b"# -Title\n"));
    assert!(source.ends_with(b"\n--literal-note\n"));
}

#[test]
fn command_options_can_precede_file_and_workdir_can_follow_positionals() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.0.join("work")).unwrap();
    fs::write(fixture.0.join("work/marker"), b"selected directory\n").unwrap();
    assert!(fixture.run(&["init", "doc.md", "Title"]).status.success());
    let executed = fixture.run(&["exec", "doc.md", "sh", "cat marker", "--workdir", "work"]);
    assert!(executed.status.success(), "{:?}", executed.stderr);
    assert_eq!(executed.stdout, b"selected directory\n");
    let verified = fixture.run(&[
        "verify",
        "--output",
        "copy.md",
        "doc.md",
        "--workdir",
        "work",
    ]);
    assert!(verified.status.success(), "{:?}", verified.stderr);
    assert_eq!(
        fs::read(fixture.0.join("copy.md")).unwrap(),
        fs::read(fixture.0.join("doc.md")).unwrap()
    );
    let extracted = fixture.run(&["extract", "--filename", "new.md", "doc.md"]);
    assert!(extracted.status.success(), "{:?}", extracted.stderr);
    assert!(
        extracted
            .stdout
            .starts_with(b"mdrun init -- new.md Title\n")
    );
}

#[test]
fn omitted_note_code_and_image_arguments_read_stdin() {
    let fixture = Fixture::new();
    assert!(fixture.run(&["init", "doc.md", "Title"]).status.success());
    let note = fixture.stdin(&["note", "doc.md"], b"From stdin");
    assert!(note.status.success(), "{:?}", note.stderr);
    let executed = fixture.stdin(&["exec", "doc.md", "sh"], b"printf 'stdin code\\n'");
    assert!(executed.status.success(), "{:?}", executed.stderr);
    assert_eq!(executed.stdout, b"stdin code\n");
    fs::write(fixture.0.join("source.png"), b"\x89PNG\r\n\x1a\n").unwrap();
    let image = fixture.stdin(&["image", "doc.md"], b"source.png");
    assert!(image.status.success(), "{:?}", image.stderr);
    let source = fs::read(fixture.0.join("doc.md")).unwrap();
    assert!(
        source
            .windows(b"\nFrom stdin\n".len())
            .any(|part| part == b"\nFrom stdin\n")
    );
    assert!(
        source
            .windows(b"```bash {image}\nsource.png\n```\n".len())
            .any(|part| part == b"```bash {image}\nsource.png\n```\n")
    );
}

#[cfg(unix)]
#[test]
fn native_title_and_note_arguments_keep_non_utf8_bytes() {
    use std::os::unix::ffi::OsStrExt;

    let fixture = Fixture::new();
    let filename = OsStr::new("doc.md");
    let init = fixture.run_native(&[
        OsStr::new("init"),
        filename,
        OsStr::from_bytes(b"Title \xff"),
    ]);
    assert!(init.status.success(), "{:?}", init.stderr);
    let note = fixture.run_native(&[
        OsStr::new("note"),
        filename,
        OsStr::from_bytes(b"Note \xfe"),
    ]);
    assert!(note.status.success(), "{:?}", note.stderr);
    let source = fs::read(fixture.0.join(filename)).unwrap();
    assert!(source.starts_with(b"# Title \xff\n"));
    assert!(source.ends_with(b"\nNote \xfe\n"));
}

#[test]
fn session_guide_documents_host_cold_reason_names() {
    use std::collections::BTreeSet;

    let context = include_str!("../../src/session/context.rs");
    let mut reasons: BTreeSet<_> = context
        .split("return Some(\"")
        .skip(1)
        .map(|literal| literal.split_once('"').unwrap().0)
        .collect();
    let host = include_str!("../../src/session/host.rs");
    assert!(host.contains("Some(\"explicit_cold\")"));
    reasons.insert("explicit_cold");
    let guide = include_str!("../../docs/guides/sessions.md");
    let section = guide
        .split_once("## Cold route reasons\n")
        .expect("the session guide must name report reason keys")
        .1
        .split("\n## ")
        .next()
        .unwrap();
    let documented: BTreeSet<_> = section
        .lines()
        .filter_map(|line| line.strip_prefix("| `"))
        .map(|row| row.split_once('`').unwrap().0)
        .collect();
    assert_eq!(documented, reasons, "document actual cold_reasons keys");
    assert!(!guide.contains("environment_mismatch"));
}
