use std::{env, process::ExitCode};

const HELP: &str = "mdrun — Create, run, and verify executable Markdown documents.

Usage: mdrun [--help | --version]

  --help, -h, help  Show this help
  --version         Show the mdrun version

This is the project bootstrap. Document commands and warm execution are
planned in docs/PLAN.md and are not implemented in this build.
";

fn main() -> ExitCode {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.is_empty()
        || (args.len() == 1 && matches!(args[0].to_str(), Some("--help" | "-h" | "help")))
    {
        print!("{HELP}");
        ExitCode::SUCCESS
    } else if args.len() == 1 && args[0] == "--version" {
        println!("mdrun {}", env!("CARGO_PKG_VERSION"));
        ExitCode::SUCCESS
    } else {
        eprintln!("mdrun: unsupported arguments in this bootstrap; use --help");
        ExitCode::from(2)
    }
}
