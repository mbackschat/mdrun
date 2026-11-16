mod app;
mod cli;
mod discovery;
mod document;
mod remote;

use clap::Parser;
use std::process::ExitCode;

fn run() -> i32 {
    let cli = cli::Cli::parse();
    match app::execute(&cli.command, cli.workdir.as_deref()) {
        Ok(status) => status,
        Err(message) => {
            eprintln!("error: {message}");
            1
        }
    }
}

fn main() -> ExitCode {
    ExitCode::from(run() as u8)
}
