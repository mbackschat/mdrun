pub(crate) mod batch;
pub(crate) mod broker;
pub(crate) mod client;
mod config;
mod context;
pub(crate) mod custody;
pub(crate) mod host;
mod platform;
pub(crate) mod report;
mod wakeup;
mod wire;

use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Mode {
    Fast,
    Cold,
}

#[derive(Clone, Debug, clap::Args)]
pub struct Options {
    /// Prepared application binding manifest.
    #[arg(long)]
    pub binding: PathBuf,
    /// Prefer eligible warm calls, or use only the packaged launcher.
    #[arg(long, value_enum, default_value = "fast")]
    pub mode: Mode,
    /// Write a versioned execution report outside application output.
    #[arg(long)]
    pub report: Option<PathBuf>,
    /// Fail before executing calls that would use the cold launcher.
    #[arg(long)]
    pub require_warm: bool,
}
