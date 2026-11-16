use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "mdrun-cli-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_mdrun"))
            .args(args)
            .current_dir(&self.0)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .output()
            .unwrap()
    }

    fn read(&self, name: &str) -> Vec<u8> {
        fs::read(self.0.join(name)).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn extraction_reconstructs_hyphen_values_and_spaced_interpreter_paths() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new();
    fs::create_dir(fixture.0.join("bin")).unwrap();
    symlink(env!("CARGO_BIN_EXE_mdrun"), fixture.0.join("bin/mdrun")).unwrap();
    symlink("/bin/sh", fixture.0.join("shell tool")).unwrap();
    assert!(
        fixture
            .run(&["init", "original.md", "--", "-dash"])
            .status
            .success()
    );
    assert!(
        fixture
            .run(&["note", "original.md", "--", "- markdown bullet"])
            .status
            .success()
    );
    assert!(
        fixture
            .run(&["exec", "original.md", "./shell tool", "printf 'ready\\n'"])
            .status
            .success()
    );
    let extracted = fixture.run(&["extract", "original.md", "--filename", "rebuilt.md"]);
    assert!(extracted.status.success());
    let replay = Command::new("/bin/sh")
        .arg("-c")
        .arg(std::ffi::OsStr::from_bytes(&extracted.stdout))
        .current_dir(&fixture.0)
        .env_clear()
        .env(
            "PATH",
            format!("{}:/usr/bin:/bin", fixture.0.join("bin").display()),
        )
        .output()
        .unwrap();
    assert!(
        replay.status.success(),
        "{}",
        String::from_utf8_lossy(&replay.stderr)
    );
    let original = String::from_utf8(fixture.read("original.md")).unwrap();
    let rebuilt = String::from_utf8(fixture.read("rebuilt.md")).unwrap();
    assert!(rebuilt.starts_with("# -dash\n"));
    assert_eq!(
        original.split_once("-->\n").unwrap().1,
        rebuilt.split_once("-->\n").unwrap().1
    );
}

#[test]
fn author_capture_verify_extract_and_pop_preserve_the_document_contract() {
    let fixture = Fixture::new();
    assert!(fixture.run(&["init", "demo.md", "Demo"]).status.success());
    let header = fixture.read("demo.md");
    assert!(header.starts_with(b"# Demo\n\n*"));
    assert!(String::from_utf8_lossy(&header).contains(" by mdrun 0.1.0*\n<!-- mdrun-id: "));
    assert!(
        fixture
            .run(&["note", "demo.md", "A note with 'quotes'"])
            .status
            .success()
    );
    let before_exec = fixture.read("demo.md");
    let output = fixture.run(&[
        "exec",
        "demo.md",
        "sh",
        "printf 'out\\n'; printf 'err\\n' >&2; exit 7",
    ]);
    assert_eq!(output.status.code(), Some(7));
    assert_eq!(output.stdout, b"out\nerr\n");
    assert!(output.stderr.is_empty());
    let recording = fixture.read("demo.md");
    assert!(recording.ends_with(b"```output\nout\nerr\n```\n"));
    assert!(fixture.run(&["verify", "demo.md"]).status.success());
    assert_eq!(fixture.read("demo.md"), recording);
    let extract = fixture.run(&["extract", "demo.md", "--filename", "new file.md"]);
    assert!(extract.status.success());
    assert!(
        extract
            .stdout
            .starts_with(b"mdrun init -- 'new file.md' Demo\n")
    );
    assert!(fixture.run(&["pop", "demo.md"]).status.success());
    assert_eq!(fixture.read("demo.md"), before_exec);
    assert!(fixture.run(&["pop", "demo.md"]).status.success());
    assert_eq!(fixture.read("demo.md"), header);
    assert_eq!(fixture.run(&["pop", "demo.md"]).status.code(), Some(1));
}

#[test]
fn verify_copy_changes_only_output_and_retains_legacy_provenance() {
    let fixture = Fixture::new();
    let source = b"# Demo\n\n*2020-01-01 by Showboat 0.6.1*\n<!-- showboat-id: old -->\n\n```sh\nprintf 'new\\n'\n```\n\n```output\nold\n```\n";
    fs::write(fixture.0.join("demo.md"), source).unwrap();
    let output = fixture.run(&["verify", "demo.md", "--output", "updated.md"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        output.stdout,
        b"block 1:\n  expected: old\n  actual:   new\n"
    );
    assert_eq!(fixture.read("demo.md"), source);
    let expected = String::from_utf8_lossy(source)
        .replace("\nold\n", "\nnew\n")
        .into_bytes();
    assert_eq!(fixture.read("updated.md"), expected);
}

#[test]
fn workdir_changes_execution_but_not_document_paths_and_missing_documents_do_not_run() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.0.join("work")).unwrap();
    fs::write(fixture.0.join("work/marker"), b"chosen cwd\n").unwrap();
    assert!(fixture.run(&["init", "demo.md", "Demo"]).status.success());
    let output = fixture.run(&["--workdir", "work", "exec", "demo.md", "sh", "cat marker"]);
    assert_eq!(output.stdout, b"chosen cwd\n");
    assert!(output.status.success());
    assert!(!fixture.0.join("work/demo.md").exists());
    assert_eq!(
        fixture
            .run(&["exec", "absent.md", "sh", "touch unwanted"])
            .status
            .code(),
        Some(1)
    );
    assert!(!fixture.0.join("unwanted").exists());
}
