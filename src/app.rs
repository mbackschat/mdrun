use crate::cli::Command as Action;
use crate::discovery;
use crate::document::{self, BlockKind, Document};
use crate::remote;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

pub fn execute(action: &Action, workdir: Option<&Path>) -> Result<i32, String> {
    let remote_url = std::env::var("SHOWBOAT_REMOTE_URL").ok();
    let remote = remote_url.as_deref();
    match action {
        Action::Init { file: path, title } => {
            if path.exists() {
                return Err(format!("file already exists: {}", path.display()));
            }
            let timestamp = OffsetDateTime::now_utc()
                .replace_nanosecond(0)
                .map_err(|error| error.to_string())?
                .format(&Rfc3339)
                .map_err(|error| error.to_string())?;
            let id = uuid::Uuid::new_v4().to_string();
            write(
                path,
                &document::render_title(
                    title.as_bytes(),
                    timestamp.as_bytes(),
                    env!("CARGO_PKG_VERSION").as_bytes(),
                    id.as_bytes(),
                ),
            )?;
            remote::post_form(
                remote,
                &[
                    ("uuid", id.as_bytes()),
                    ("command", b"init"),
                    ("title", title.as_bytes()),
                ],
            );
        }
        Action::Note { file: path, text } => {
            let text = text_arg(text.as_deref())?;
            let doc = read(path)?;
            let rendered = document::render_note(&text);
            write(path, &doc.append(&rendered))?;
            post(&doc, remote, b"note", &[("markdown", &rendered)]);
        }
        Action::Exec {
            file: path,
            lang,
            code,
        } => {
            let code = text_arg(code.as_deref())?;
            require_file(path)?;
            let (output, status) = run_code(lang.as_bytes(), &code, workdir)?;
            let doc = read(path)?;
            let rendered = [
                document::render_code(lang.as_bytes(), &code, false),
                b"\n".to_vec(),
                document::render_output(&output),
            ]
            .concat();
            write(path, &doc.append(&rendered))?;
            post(
                &doc,
                remote,
                b"exec",
                &[
                    ("language", lang.as_bytes()),
                    ("input", &code),
                    ("output", &output),
                ],
            );
            stdout(&output)?;
            return Ok(status);
        }
        Action::Image { file: path, input } => {
            let input = text_arg(input.as_deref())?;
            require_file(path)?;
            let (source, supplied_alt) = image_input(&input);
            let source_path = Path::new(OsStr::from_bytes(source));
            let metadata = fs::metadata(source_path)
                .map_err(|_| format!("image file not found: {}", source_path.display()))?;
            if metadata.is_dir() {
                return Err(format!(
                    "image path is a directory: {}",
                    source_path.display()
                ));
            }
            let extension = source_path
                .extension()
                .map(|value| value.as_bytes().to_ascii_lowercase())
                .unwrap_or_default();
            if ![b"png".as_slice(), b"jpg", b"jpeg", b"gif", b"svg"].contains(&extension.as_slice())
            {
                let dot = if extension.is_empty() { "" } else { "." };
                return Err(format!(
                    "unrecognized image format: {dot}{}",
                    String::from_utf8_lossy(&extension)
                ));
            }
            let day = OffsetDateTime::now_utc().date();
            let filename = format!(
                "{}-{day}.{}",
                &uuid::Uuid::new_v4().to_string()[..8],
                String::from_utf8_lossy(&extension)
            );
            let destination = path.parent().unwrap_or(Path::new(".")).join(&filename);
            let bytes = fs::read(source_path).map_err(|error| {
                format!(
                    "opening image: open {}: {}",
                    source_path.display(),
                    io_message(&error)
                )
            })?;
            fs::write(&destination, bytes).map_err(|error| {
                format!(
                    "creating destination: open {}: {}",
                    destination.display(),
                    io_message(&error)
                )
            })?;
            let alt = if supplied_alt.is_empty() {
                Path::new(&filename).file_stem().unwrap().as_bytes()
            } else {
                supplied_alt
            };
            let doc = read(path)?;
            let rendered = [
                document::render_code(b"bash", &input, true),
                b"\n".to_vec(),
                document::render_image(alt, filename.as_bytes()),
            ]
            .concat();
            write(path, &doc.append(&rendered))?;
            if let Some(id) = doc.document_id() {
                remote::post_image(remote, id, &filename, alt, &destination);
            }
        }
        Action::Pop { file: path } => {
            let doc = read(path)?;
            write(path, &doc.pop()?)?;
            post(&doc, remote, b"pop", &[]);
        }
        Action::Verify { file: path, output } => {
            let doc = read(path)?;
            let mut replacements = Vec::new();
            let mut differences = Vec::new();
            for (index, block) in doc.blocks.iter().enumerate() {
                if let BlockKind::Code {
                    lang,
                    code,
                    is_image: false,
                } = &block.kind
                {
                    let (actual, _) = run_code(lang, code, workdir).map_err(|error| {
                        format!(
                            "executing block {index}: {}",
                            error.strip_prefix("running code: ").unwrap_or(&error)
                        )
                    })?;
                    if let Some(next) = doc.blocks.get(index + 1)
                        && let BlockKind::Output { content } = &next.kind
                        && *content != actual
                    {
                        differences
                            .extend_from_slice(format!("block {index}:\n  expected: ").as_bytes());
                        differences.extend_from_slice(trim_newlines(content));
                        differences.extend_from_slice(b"\n  actual:   ");
                        differences.extend_from_slice(trim_newlines(&actual));
                        differences.push(b'\n');
                        replacements.push((index + 1, actual));
                    }
                }
            }
            if let Some(output) = output {
                write(Path::new(output), &doc.replace_outputs(&replacements)?)
                    .map_err(|error| format!("writing output file: {error}"))?;
            }
            stdout(&differences)?;
            return Ok(i32::from(!replacements.is_empty()));
        }
        Action::Extract {
            file: path,
            filename,
        } => {
            let doc = read(path)?;
            let target = filename.as_deref().unwrap_or(path.as_os_str());
            for command in doc.extract(target.as_bytes()) {
                stdout(&command)?;
                stdout(b"\n")?;
            }
        }
        Action::List {
            files,
            filter,
            json,
        } => {
            let mut entries = Vec::new();
            for file in files {
                let filename = file
                    .to_str()
                    .ok_or_else(|| format!("file path is not valid UTF-8: {}", file.display()))?;
                entries.extend(discovery::inventory(
                    filename,
                    &read(file)?,
                    filter.as_deref(),
                )?);
            }
            stdout(&discovery::render(&entries, *json)?)?;
        }
        Action::Preview { file, block } => {
            let doc = read(file)?;
            stdout(discovery::preview(&doc, *block)?)?;
        }
    }
    Ok(0)
}

fn text_arg(text: Option<&OsStr>) -> Result<Vec<u8>, String> {
    if let Some(text) = text {
        return Ok(text.as_bytes().to_vec());
    }
    let mut text = Vec::new();
    io::stdin()
        .read_to_end(&mut text)
        .map_err(|error| format!("reading stdin: {error}"))?;
    Ok(text)
}

fn require_file(path: &Path) -> Result<(), String> {
    fs::metadata(path)
        .map(|_| ())
        .map_err(|_| format!("file not found: {}", path.display()))
}

fn read(path: &Path) -> Result<Document, String> {
    let source = fs::read(path).map_err(|error| {
        format!(
            "opening file: open {}: {}",
            path.display(),
            io_message(&error)
        )
    })?;
    Document::parse(&source).map_err(|error| format!("parsing file: {error}"))
}

fn write(path: &Path, content: &[u8]) -> Result<(), String> {
    fs::write(path, content).map_err(|error| {
        format!(
            "creating file: open {}: {}",
            path.display(),
            io_message(&error)
        )
    })
}

fn stdout(bytes: &[u8]) -> Result<(), String> {
    io::stdout()
        .lock()
        .write_all(bytes)
        .map_err(|error| error.to_string())
}

fn io_message(error: &io::Error) -> String {
    match error.kind() {
        io::ErrorKind::NotFound => "no such file or directory".to_owned(),
        io::ErrorKind::PermissionDenied => "permission denied".to_owned(),
        io::ErrorKind::IsADirectory => "is a directory".to_owned(),
        _ => error.to_string(),
    }
}

fn post(doc: &Document, remote: Option<&str>, command: &[u8], fields: &[(&str, &[u8])]) {
    if let Some(id) = doc.document_id() {
        let mut form = vec![("uuid", id), ("command", command)];
        form.extend_from_slice(fields);
        remote::post_form(remote, &form);
    }
}

fn image_input(input: &[u8]) -> (&[u8], &[u8]) {
    let input = input.trim_ascii();
    let unescaped = input
        .strip_prefix(b"\\![")
        .map(|_| &input[1..])
        .unwrap_or(input);
    if let Some(rest) = unescaped
        .strip_prefix(b"![")
        .and_then(|value| value.strip_suffix(b")"))
        && let Some(index) = rest.windows(2).position(|value| value == b"](")
    {
        (&rest[index + 2..], &rest[..index])
    } else {
        (input, b"")
    }
}

fn trim_newlines(value: &[u8]) -> &[u8] {
    let end = value
        .iter()
        .rposition(|byte| *byte != b'\n')
        .map_or(0, |index| index + 1);
    &value[..end]
}

fn interpreter(lang: &[u8]) -> Result<PathBuf, String> {
    let name = OsStr::from_bytes(lang);
    if lang.contains(&b'/') {
        return Ok(PathBuf::from(name));
    }
    if let Some(path) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&path) {
            let candidate = directory.join(name);
            if fs::metadata(&candidate)
                .is_ok_and(|meta| !meta.is_dir() && meta.permissions().mode() & 0o111 != 0)
            {
                return Ok(candidate);
            }
        }
    }
    Err(format!(
        "exec: {:?}: executable file not found in $PATH",
        String::from_utf8_lossy(lang)
    ))
}

fn run_code(lang: &[u8], code: &[u8], workdir: Option<&Path>) -> Result<(Vec<u8>, i32), String> {
    let context = |error: String| {
        format!(
            "running code: executing {}: {error}",
            String::from_utf8_lossy(lang)
        )
    };
    let program = interpreter(lang).map_err(context)?;
    let (mut reader, writer) = io::pipe().map_err(|error| context(error.to_string()))?;
    let writer_error = writer
        .try_clone()
        .map_err(|error| context(error.to_string()))?;
    let mut command = Command::new(&program);
    command
        .arg("-c")
        .arg(OsString::from_vec(code.to_vec()))
        .stdin(Stdio::null())
        .stdout(writer)
        .stderr(writer_error);
    if let Some(directory) = workdir.filter(|dir| !dir.as_os_str().is_empty()) {
        fs::metadata(directory).map_err(|error| {
            context(format!(
                "chdir {}: {}",
                directory.display(),
                io_message(&error)
            ))
        })?;
        command.current_dir(directory);
    }
    let mut child = command.spawn().map_err(|error| {
        context(format!(
            "fork/exec {}: {}",
            program.display(),
            io_message(&error)
        ))
    })?;
    drop(command);
    let mut output = Vec::new();
    reader
        .read_to_end(&mut output)
        .map_err(|error| context(error.to_string()))?;
    let status = child.wait().map_err(|error| context(error.to_string()))?;
    Ok((output, status.code().unwrap_or(255)))
}
