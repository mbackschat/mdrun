use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mdrun"))
        .args(args)
        .env_clear()
        .output()
        .expect("run standalone mdrun")
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
fn help_explains_the_current_bootstrap_boundary() {
    for args in [&[][..], &["--help"], &["-h"], &["help"]] {
        let output = run(args);
        assert!(output.status.success());
        let help = String::from_utf8(output.stdout).unwrap();
        assert!(help.contains("Create, run, and verify executable Markdown documents."));
        assert!(help.contains("bootstrap"));
        assert!(help.contains("docs/PLAN.md"));
        assert!(output.stderr.is_empty());
    }
}

#[test]
fn unsupported_commands_and_extra_arguments_fail_explicitly() {
    for args in [
        &["verify", "demo.md"][..],
        &["--unknown"],
        &["--version", "ignored"],
        &["--help", "ignored"],
    ] {
        let output = run(args);
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8(output.stderr).unwrap().contains("mdrun:"));
    }
}
