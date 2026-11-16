mod app;
mod cli;
mod discovery;
mod document;
mod remote;
mod session;

use clap::Parser;
use std::ffi::OsStr;
use std::path::Path;
use std::process::ExitCode;

fn run() -> i32 {
    let arguments: Vec<_> = std::env::args_os().collect();
    // The session symlink forwards application arguments verbatim; clap owns mdrun's grammar.
    if Path::new(&arguments[0]).file_name() != Some(OsStr::new("mdrun"))
        && let Some(configuration) = std::env::var_os("MDRUN_SESSION")
    {
        let forwarded = (|| {
            let configuration = session::client::load(Path::new(&configuration))?;
            if Path::new(&arguments[0]).file_name() != Some(OsStr::new(&configuration.binding_name))
            {
                return Ok(None);
            }
            session::client::run(&configuration, arguments[1..].to_vec()).map(Some)
        })();
        match forwarded {
            Ok(Some(status)) => return native_status(status),
            Ok(None) => {}
            Err(_) => {
                // The outer host owns infrastructure diagnostics, outside recorded application streams.
                return 125;
            }
        }
    }
    let cli = cli::Cli::parse_from(arguments);
    match app::execute(&cli.command, cli.workdir.as_deref()) {
        Ok(status) => status,
        Err(message) => {
            eprintln!("error: {message}");
            1
        }
    }
}

fn native_status(status: session::custody::PayloadStatus) -> i32 {
    if let Some(code) = status.code {
        return code;
    }
    if let Some(signal) = status.signal {
        // Preserve native wait status for a signalled cold process, after release acknowledgement.
        // https://pubs.opengroup.org/onlinepubs/9799919799/functions/raise.html
        unsafe {
            let mut set = std::mem::zeroed();
            libc::sigemptyset(&mut set);
            libc::sigaddset(&mut set, signal);
            libc::sigprocmask(libc::SIG_UNBLOCK, &set, std::ptr::null_mut());
            libc::signal(signal, libc::SIG_DFL);
            libc::raise(signal);
        }
    }
    125
}

fn main() -> ExitCode {
    ExitCode::from(run() as u8)
}
