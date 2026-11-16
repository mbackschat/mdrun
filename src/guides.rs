use std::io::{self, Write};

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum Topic {
    /// Create, inspect, correct and verify executable Markdown.
    Documents,
    /// Reuse a qualified application and interpret routing and cleanup.
    Sessions,
    /// Exit statuses, streams, paths, environment and remote delivery.
    Reference,
}

pub fn text(topic: Option<Topic>) -> &'static str {
    match topic {
        None => include_str!("../docs/guides/README.md"),
        Some(Topic::Documents) => include_str!("../docs/guides/documents.md"),
        Some(Topic::Sessions) => include_str!("../docs/guides/sessions.md"),
        Some(Topic::Reference) => include_str!("../docs/guides/reference.md"),
    }
}

pub fn print(topic: Option<Topic>) -> io::Result<()> {
    io::stdout().lock().write_all(text(topic).as_bytes())
}
