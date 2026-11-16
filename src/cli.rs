use clap::{Parser, Subcommand};
use std::ffi::OsString;
use std::path::PathBuf;

/// Create, run, and verify executable Markdown documents.
///
/// Documents combine Markdown notes, executable code, and recorded output.
/// Omit note text, command code, or image input to read it from stdin.
#[derive(Debug, Parser)]
#[command(
    version,
    about = "Create, run, and verify executable Markdown documents."
)]
pub struct Cli {
    /// Execute code in DIR while resolving document paths from the current directory.
    #[arg(long, global = true, value_name = "DIR")]
    pub workdir: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create a document with a title, timestamp, and unique identifier.
    Init {
        /// Document to create; the file must not already exist.
        file: PathBuf,
        /// Title for the new document.
        title: OsString,
    },

    /// Append a Markdown note to a document.
    Note {
        /// Existing document to update.
        file: PathBuf,
        /// Markdown text; read from stdin when omitted.
        text: Option<OsString>,
    },

    /// Execute code and append its combined stdout and stderr.
    Exec {
        /// Existing document to update.
        file: PathBuf,
        /// Interpreter executable, invoked with -c and the code.
        lang: OsString,
        /// Code to execute; read from stdin when omitted.
        code: Option<OsString>,
    },

    /// Copy an image and append its Markdown reference.
    Image {
        /// Existing document to update.
        file: PathBuf,
        /// Image path or ![alt](path) reference; read from stdin when omitted.
        input: Option<OsString>,
    },

    /// Remove the most recent note or code/output pair.
    Pop {
        /// Existing document to update.
        file: PathBuf,
    },

    /// Replay executable blocks and report changed output.
    Verify {
        /// Document to replay.
        file: PathBuf,
        /// Write a copy with updated output to FILE.
        #[arg(long, value_name = "FILE")]
        output: Option<PathBuf>,
        /// Verify through a prepared cooperative application binding.
        #[arg(long)]
        binding: Option<PathBuf>,
        /// Select warm eligibility or the independent packaged launcher.
        #[arg(long, value_enum, requires = "binding")]
        mode: Option<crate::session::Mode>,
        /// Write a versioned session report outside recorded output.
        #[arg(long, requires = "binding")]
        report: Option<PathBuf>,
        /// Reject calls that cannot execute warm.
        #[arg(long, requires = "binding")]
        require_warm: bool,
    },

    /// Print shell commands that recreate a document.
    Extract {
        /// Document to extract.
        file: PathBuf,
        /// Use NAME as the document path in the extracted commands.
        #[arg(long, value_name = "NAME")]
        filename: Option<OsString>,
    },

    /// List executable blocks without running commands.
    List {
        /// Documents to inspect, in inventory order.
        #[arg(required = true, num_args = 1.., value_name = "FILE")]
        files: Vec<PathBuf>,
        /// Case-sensitive literal substring of file, heading, or complete command text.
        #[arg(long, value_name = "TEXT")]
        filter: Option<String>,
        /// Emit a JSON array instead of readable inventory rows.
        #[arg(long)]
        json: bool,
    },

    /// Print one executable block's complete fenced source without running it.
    Preview {
        /// Document to inspect.
        file: PathBuf,
        /// Zero-based parsed-block index, identical to verify's block positions.
        block: usize,
    },

    /// Run an author command with one managed application binding.
    Session {
        #[command(flatten)]
        options: crate::session::Options,
        /// Command and arguments to execute within the managed session.
        #[arg(last = true, required = true)]
        command: Vec<OsString>,
    },

    #[command(name = "__custodian", hide = true)]
    Custodian {
        #[arg(long)]
        config: PathBuf,
    },

    #[command(name = "__anchor", hide = true)]
    Anchor,
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStringExt;
    use std::path::Path;

    #[test]
    fn session_terminator_preserves_application_flags_and_native_arguments() {
        let application_args = [
            OsString::from("tool"),
            OsString::from("--help"),
            OsString::from("--binding=application.json"),
            OsString::from("--mode"),
            OsString::from("application-mode"),
            OsString::from_vec(b"native-\xff".to_vec()),
        ];
        let cli = Cli::try_parse_from(
            ["mdrun", "session", "--binding", "binding.json", "--"]
                .map(OsString::from)
                .into_iter()
                .chain(application_args.iter().cloned()),
        )
        .unwrap();
        let Command::Session { options, command } = cli.command else {
            panic!("expected session command");
        };
        assert_eq!(options.binding, PathBuf::from("binding.json"));
        assert_eq!(command, application_args);
    }

    #[test]
    fn native_paths_interpreters_and_code_keep_their_original_bytes() {
        let file = OsString::from_vec(b"doc-\xff.md".to_vec());
        let lang = OsString::from_vec(b"interpreter-\xfe".to_vec());
        let code = OsString::from_vec(b"code \xfd".to_vec());
        let workdir = OsString::from_vec(b"work-\xfc".to_vec());
        let cli = Cli::try_parse_from([
            OsString::from("mdrun"),
            OsString::from("exec"),
            file.clone(),
            lang.clone(),
            code.clone(),
            OsString::from("--workdir"),
            workdir.clone(),
        ])
        .unwrap();
        assert_eq!(cli.workdir.unwrap(), PathBuf::from(workdir));
        let Command::Exec {
            file: actual_file,
            lang: actual_lang,
            code: actual_code,
            ..
        } = cli.command
        else {
            panic!("expected exec command");
        };
        assert_eq!(actual_file.as_path(), Path::new(&file));
        assert_eq!(actual_lang, lang);
        assert_eq!(actual_code, Some(code));
    }

    #[test]
    fn native_output_and_extraction_paths_keep_their_original_bytes() {
        let destination = OsString::from_vec(b"destination-\xff.md".to_vec());
        let verify = Cli::try_parse_from([
            OsString::from("mdrun"),
            OsString::from("verify"),
            OsString::from("--output"),
            destination.clone(),
            OsString::from("doc.md"),
        ])
        .unwrap();
        assert!(
            matches!(verify.command, Command::Verify { output: Some(output), .. } if output.as_path() == Path::new(&destination))
        );
        let extract = Cli::try_parse_from([
            OsString::from("mdrun"),
            OsString::from("extract"),
            OsString::from("doc.md"),
            OsString::from("--filename"),
            destination.clone(),
        ])
        .unwrap();
        assert!(
            matches!(extract.command, Command::Extract { filename: Some(filename), .. } if filename == destination)
        );
    }
}
