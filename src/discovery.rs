use crate::document::{BlockKind, Document};
use serde_json::json;

pub struct Entry {
    file: String,
    block: usize,
    line: usize,
    heading: Option<String>,
    interpreter: String,
    summary: String,
}

pub fn inventory(
    file: &str,
    document: &Document,
    filter: Option<&str>,
) -> Result<Vec<Entry>, String> {
    let mut entries = Vec::new();
    let mut heading = None;
    let mut line = 1;
    let mut counted = 0;
    for (block, parsed) in document.blocks.iter().enumerate() {
        line += document.source[counted..parsed.span.start]
            .iter()
            .filter(|&&byte| byte == b'\n')
            .count();
        counted = parsed.span.start;
        match &parsed.kind {
            BlockKind::Title { title, .. } => {
                heading = Some(text(title, "title", file)?.to_owned());
            }
            BlockKind::Commentary { text: prose } => {
                if let Some(title) = prose
                    .split(|&byte| byte == b'\n')
                    .rev()
                    .find_map(atx_heading)
                {
                    heading = Some(text(title, "heading", file)?.to_owned());
                }
            }
            BlockKind::Code {
                lang,
                code,
                is_image: false,
            } => {
                let interpreter = text(lang, "interpreter", file)?;
                let command = text(code, "command", file)?;
                if filter.is_some_and(|filter| {
                    !file.contains(filter)
                        && !heading
                            .as_deref()
                            .is_some_and(|title| title.contains(filter))
                        && !command.contains(filter)
                }) {
                    continue;
                }
                entries.push(Entry {
                    file: file.to_owned(),
                    block,
                    line,
                    heading: heading.clone(),
                    interpreter: interpreter.to_owned(),
                    summary: command.split('\n').next().unwrap_or("").to_owned(),
                });
            }
            _ => {}
        }
    }
    Ok(entries)
}

pub fn render(entries: &[Entry], as_json: bool) -> Result<Vec<u8>, String> {
    if as_json {
        let values: Vec<_> = entries
            .iter()
            .map(|entry| {
                json!({
                    "file": entry.file,
                    "block": entry.block,
                    "line": entry.line,
                    "heading": entry.heading,
                    "interpreter": entry.interpreter,
                    "summary": entry.summary
                })
            })
            .collect();
        let mut bytes = serde_json::to_vec(&values).map_err(|error| error.to_string())?;
        bytes.push(b'\n');
        return Ok(bytes);
    }
    let mut rendered = String::new();
    for entry in entries {
        rendered.push_str(&format!(
            "{}:{}\tblock {}\t{}\t{}\t{}\n",
            readable(&entry.file),
            entry.line,
            entry.block,
            readable(entry.heading.as_deref().unwrap_or("")),
            readable(&entry.interpreter),
            readable(&entry.summary)
        ));
    }
    Ok(rendered.into_bytes())
}

pub fn preview(document: &Document, block: usize) -> Result<&[u8], String> {
    let parsed = document
        .blocks
        .get(block)
        .ok_or_else(|| format!("block {block} does not exist in the document"))?;
    if !matches!(
        parsed.kind,
        BlockKind::Code {
            is_image: false,
            ..
        }
    ) {
        return Err(format!("block {block} is not executable"));
    }
    Ok(&document.source[parsed.span.clone()])
}

fn text<'a>(bytes: &'a [u8], field: &str, file: &str) -> Result<&'a str, String> {
    std::str::from_utf8(bytes).map_err(|_| format!("{file}: {field} is not valid UTF-8"))
}

fn readable(text: &str) -> String {
    let mut escaped = String::new();
    for character in text.chars() {
        if character.is_control() {
            escaped.extend(character.escape_debug());
        } else {
            escaped.push(character);
        }
    }
    escaped
}

fn atx_heading(line: &[u8]) -> Option<&[u8]> {
    let indentation = line.iter().take_while(|&&byte| byte == b' ').count();
    if indentation > 3 {
        return None;
    }
    let line = &line[indentation..];
    let width = line.iter().take_while(|&&byte| byte == b'#').count();
    if !(1..=6).contains(&width) {
        return None;
    }
    let rest = &line[width..];
    if rest.first().is_some_and(|byte| !b" \t".contains(byte)) {
        return None;
    }
    let title = rest.trim_ascii();
    let closing = title.iter().rev().take_while(|&&byte| byte == b'#').count();
    let prefix = &title[..title.len() - closing];
    if closing > 0 && (prefix.is_empty() || prefix.ends_with(b" ") || prefix.ends_with(b"\t")) {
        Some(prefix.trim_ascii_end())
    } else {
        Some(title)
    }
}
