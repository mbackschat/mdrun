use clap::{Command, CommandFactory};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

const REFERENCE: &str = include_str!("../docs/guides/reference.md");

fn commands() -> Vec<(String, Command)> {
    fn collect(command: &Command, name: String, result: &mut Vec<(String, Command)>) {
        result.push((name.clone(), command.clone()));
        for child in command
            .get_subcommands()
            .filter(|child| !child.is_hide_set())
        {
            collect(child, format!("{name}-{}", child.get_name()), result);
        }
    }
    let mut command = crate::cli::Cli::command().disable_help_subcommand(true);
    command.build();
    let mut result = Vec::new();
    collect(&command, command.get_name().to_owned(), &mut result);
    result
}

fn escaped(text: &str) -> String {
    text.replace('\\', "\\e").replace('-', "\\-")
}

fn inline(text: &str) -> Result<String, String> {
    if !text.matches('`').count().is_multiple_of(2) || text.contains(['[', ']']) {
        return Err(format!("unsupported reference inline markup: {text}"));
    }
    let mut result = String::from("\\&");
    for (index, part) in text.split('`').enumerate() {
        if index != 0 {
            result.push_str(if index % 2 == 1 { "\\fB" } else { "\\fR" });
        }
        result.push_str(&escaped(part));
    }
    Ok(result)
}

// This adapter accepts only the reference's maintained prose, bullets and two-column
// tables. New Markdown constructs must fail freshness checks instead of disappearing.
fn reference_sections(source: &str) -> Result<String, String> {
    let mut output = String::new();
    let mut active = false;
    let mut table_row = 0;
    for line in source.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            active = true;
            table_row = 0;
            let heading = match heading {
                "Paths and files" => "FILES".to_owned(),
                other => other.to_uppercase(),
            };
            if !heading.chars().all(|c| c.is_ascii_uppercase() || c == ' ') {
                return Err("unsupported reference heading".to_owned());
            }
            writeln!(output, ".SH {heading}").unwrap();
        } else if !active || line.is_empty() {
            continue;
        } else if line.starts_with('|') {
            let cells: Vec<_> = line.split('|').collect();
            if cells.len() != 4 || !cells[0].is_empty() || !cells[3].is_empty() {
                return Err("reference tables must have two columns".to_owned());
            }
            table_row += 1;
            if table_row == 1 {
                continue;
            }
            if table_row == 2 {
                if cells[1..3].iter().any(|cell| cell.trim() != "---") {
                    return Err("unsupported reference table separator".to_owned());
                }
                continue;
            }
            writeln!(
                output,
                ".TP\n{}\n{}",
                inline(cells[1].trim())?,
                inline(cells[2].trim())?
            )
            .unwrap();
        } else {
            table_row = 0;
            if let Some(item) = line.strip_prefix("- ") {
                writeln!(output, ".IP \\(bu 2\n{}", inline(item)?).unwrap();
            } else {
                if line.starts_with(['#', ' ', '>'])
                    || line.starts_with("```")
                    || line.contains("**")
                {
                    return Err(format!("unsupported reference block: {line}"));
                }
                writeln!(output, ".PP\n{}", inline(line)?).unwrap();
            }
        }
    }
    if !active {
        return Err("reference sections are missing".to_owned());
    }
    Ok(output)
}

fn pages() -> BTreeMap<String, Vec<u8>> {
    let reference = reference_sections(REFERENCE).expect("supported reference Markdown");
    commands()
        .into_iter()
        .map(|(name, command)| {
            let mut bytes = Vec::new();
            clap_mangen::Man::new(command)
                .title(&name)
                .date("2026-11-16")
                .source(concat!("mdrun ", env!("CARGO_PKG_VERSION")))
                .manual("mdrun manual")
                .render(&mut bytes)
                .expect("render clap reference");
            bytes.extend_from_slice(reference.as_bytes());
            let source = String::from_utf8(bytes).expect("UTF-8 generated manual");
            let normalized = source
                .lines()
                .map(str::trim_end)
                .collect::<Vec<_>>()
                .join("\n")
                + "\n";
            (format!("{name}.1"), normalized.into_bytes())
        })
        .collect()
}

fn absolute_environment(name: &str) -> PathBuf {
    let path = PathBuf::from(
        std::env::var_os(name).unwrap_or_else(|| panic!("set {name} to an absolute path")),
    );
    assert!(path.is_absolute(), "{name} must be absolute");
    path
}

fn rendered_text(bytes: Vec<u8>) -> String {
    let mut text = String::new();
    for character in String::from_utf8(bytes).unwrap().chars() {
        if character == '\u{8}' {
            text.pop();
        } else {
            text.push(character);
        }
    }
    text
}

#[test]
fn checked_manuals_match_clap_and_reference() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/man");
    let expected = pages();
    let actual: BTreeMap<_, _> = fs::read_dir(&directory)
        .expect("generate docs/man first")
        .map(|entry| {
            let path = entry.unwrap().path();
            (
                path.file_name().unwrap().to_str().unwrap().to_owned(),
                fs::read(path).unwrap(),
            )
        })
        .collect();
    assert_eq!(
        actual.keys().collect::<Vec<_>>(),
        expected.keys().collect::<Vec<_>>(),
        "manual inventory differs from public commands"
    );
    for (name, bytes) in expected {
        assert!(actual[&name] == bytes, "regenerate {name}: content differs");
    }
}

#[test]
fn manuals_cover_public_options_without_internal_commands() {
    let pages = pages();
    for (name, command) in commands() {
        let text = String::from_utf8(pages[&format!("{name}.1")].clone()).unwrap();
        assert!(!text.contains("__custodian") && !text.contains("__anchor"));
        for argument in command
            .get_arguments()
            .filter(|argument| !argument.is_hide_set())
        {
            if let Some(long) = argument.get_long() {
                assert!(
                    text.contains(&escaped(&format!("--{long}"))),
                    "{name} omits --{long}"
                );
            }
        }
        for section in ["EXIT STATUS", "ENVIRONMENT", "FILES"] {
            assert!(
                text.contains(&format!(".SH {section}")),
                "{name} omits {section}"
            );
        }
        assert!(text.contains("SHOWBOAT_REMOTE_URL"));
        assert!(text.contains("does not undo side effects"));
    }
    assert!(!pages.contains_key("mdrun-help.1"));
}

#[test]
fn reference_conversion_preserves_text_and_rejects_unhandled_formats() {
    assert_eq!(rendered_text(b"P\x08PA\x08AT\x08TH\x08H".to_vec()), "PATH");
    let output = reference_sections("## Files\n\n- `--output` reads C:\\tmp\n\n.dot\n\n| Name | Meaning |\n| --- | --- |\n| `PATH` | Lookup |\n").unwrap();
    assert!(output.contains("\\fB\\-\\-output\\fR reads C:\\etmp"));
    assert!(output.contains("\\&.dot"));
    assert!(output.contains(".TP\n\\&\\fBPATH\\fR\n\\&Lookup"));
    assert!(!output.contains('|'));
    for unsupported in [
        "```sh",
        "[link](target)",
        "**bold**",
        "| a | b | c |",
        "`unclosed",
    ] {
        assert!(reference_sections(&format!("## Files\n{unsupported}\n")).is_err());
    }
}

#[test]
#[ignore = "writes manuals only to an explicitly selected directory"]
fn regenerate() {
    let directory = absolute_environment("MDRUN_MAN_OUTPUT");
    fs::create_dir_all(&directory).unwrap();
    for (name, bytes) in pages() {
        fs::write(directory.join(name), bytes).unwrap();
    }
}

#[test]
#[ignore = "requires a separately prepared mandoc executable"]
fn render_with_mandoc() {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mandoc = absolute_environment("MDRUN_MANDOC");
    for (name, bytes) in pages() {
        let mut child = Command::new(&mandoc)
            .args(["-T", "utf8", "-W", "error"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut input = child.stdin.take().unwrap();
        let writer = std::thread::spawn(move || input.write_all(&bytes));
        let result = child.wait_with_output().unwrap();
        writer.join().unwrap().unwrap();
        assert!(
            result.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let rendered = rendered_text(result.stdout);
        assert!(
            rendered.contains("SHOWBOAT_REMOTE_URL"),
            "{name} lost reference text"
        );
        assert!(!rendered.contains("__custodian") && !rendered.contains("__anchor"));
    }
}
