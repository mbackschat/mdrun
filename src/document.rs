use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlockKind {
    Title {
        title: Vec<u8>,
        timestamp: Vec<u8>,
        version: Vec<u8>,
        document_id: Vec<u8>,
    },
    Commentary {
        text: Vec<u8>,
    },
    Code {
        lang: Vec<u8>,
        code: Vec<u8>,
        is_image: bool,
    },
    Output {
        content: Vec<u8>,
    },
    Image {
        alt: Vec<u8>,
        filename: Vec<u8>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub kind: BlockKind,
    pub span: Range<usize>,
}

#[derive(Clone, Debug)]
pub struct Document {
    pub source: Vec<u8>,
    pub blocks: Vec<Block>,
    open_fence: Option<Vec<u8>>,
}

impl Document {
    pub fn parse(source: &[u8]) -> Result<Self, String> {
        let lines = scan_lines(source)?;
        let mut blocks = Vec::new();
        let mut open_fence = None;
        let mut i = 0;
        while i < lines.len() {
            let start = lines[i].start;
            if blocks.is_empty() && lines[i].text.starts_with(b"# ") {
                let title = lines[i].text[2..].to_vec();
                i += 1;
                skip_separator(&lines, &mut i);
                let mut timestamp = Vec::new();
                let mut version = Vec::new();
                if let Some(line) = lines.get(i)
                    && line.text.starts_with(b"*")
                    && line.text.ends_with(b"*")
                {
                    let dateline = trim_stars(line.text);
                    if let Some((offset, marker)) = [b" by Showboat ".as_slice(), b" by mdrun "]
                        .into_iter()
                        .find_map(|marker| {
                            find_bytes(dateline, marker).map(|offset| (offset, marker))
                        })
                    {
                        timestamp.extend_from_slice(&dateline[..offset]);
                        version.extend_from_slice(&dateline[offset + marker.len()..]);
                    } else {
                        timestamp.extend_from_slice(dateline);
                    }
                    i += 1;
                }
                let mut document_id = Vec::new();
                if let Some(line) = lines.get(i) {
                    for prefix in [b"<!-- showboat-id: ".as_slice(), b"<!-- mdrun-id: "] {
                        if let Some(rest) = line.text.strip_prefix(prefix)
                            && let Some(id) = rest.strip_suffix(b" -->")
                        {
                            document_id.extend_from_slice(id);
                            i += 1;
                            break;
                        }
                    }
                }
                blocks.push(Block {
                    kind: BlockKind::Title {
                        title,
                        timestamp,
                        version,
                        document_id,
                    },
                    span: start..lines[i - 1].end,
                });
                skip_separator(&lines, &mut i);
                continue;
            }

            if lines[i].text.starts_with(b"```") {
                let width = lines[i]
                    .text
                    .iter()
                    .take_while(|&&byte| byte == b'`')
                    .count();
                let closing = vec![b'`'; width];
                let info = &lines[i].text[width..];
                i += 1;
                let content_start = i;
                while i < lines.len() && lines[i].text != closing {
                    i += 1;
                }
                let content_end = i;
                if i < lines.len() {
                    i += 1;
                } else {
                    open_fence = Some(closing);
                }
                let content = &lines[content_start..content_end];
                let kind = if info == b"output" {
                    let mut output = Vec::new();
                    for line in content {
                        output.extend_from_slice(line.text);
                        output.push(b'\n');
                    }
                    BlockKind::Output { content: output }
                } else {
                    let (lang, is_image) = if let Some(lang) = info.strip_suffix(b" {image}") {
                        (lang, true)
                    } else {
                        (info, false)
                    };
                    BlockKind::Code {
                        lang: lang.to_vec(),
                        code: join_lines(content),
                        is_image,
                    }
                };
                blocks.push(Block {
                    kind,
                    span: start..lines[i - 1].end,
                });
                skip_separator(&lines, &mut i);
                continue;
            }

            if let Some((alt, filename)) = parse_image_ref(lines[i].text) {
                blocks.push(Block {
                    kind: BlockKind::Image {
                        alt: alt.to_vec(),
                        filename: filename.to_vec(),
                    },
                    span: start..lines[i].end,
                });
                i += 1;
                skip_separator(&lines, &mut i);
                continue;
            }

            let first = i;
            while i < lines.len()
                && !lines[i].text.starts_with(b"```")
                && parse_image_ref(lines[i].text).is_none()
            {
                i += 1;
            }
            let mut end = i;
            while end > first && lines[end - 1].text.is_empty() {
                end -= 1;
            }
            if end > first {
                blocks.push(Block {
                    kind: BlockKind::Commentary {
                        text: join_lines(&lines[first..end]),
                    },
                    span: start..lines[end - 1].end,
                });
            }
        }
        Ok(Self {
            source: source.to_vec(),
            blocks,
            open_fence,
        })
    }

    pub fn append(&self, rendered: &[u8]) -> Vec<u8> {
        let mut result = self.source.clone();
        if let Some(fence) = &self.open_fence {
            if !result.ends_with(b"\n") {
                result.push(b'\n');
            }
            result.extend_from_slice(fence);
            result.push(b'\n');
        }
        if !result.is_empty() {
            let mut end = result.len();
            let mut newlines = 0;
            while newlines < 2 && end > 0 && result[end - 1] == b'\n' {
                end -= 1;
                if end > 0 && result[end - 1] == b'\r' {
                    end -= 1;
                }
                newlines += 1;
            }
            result.extend(std::iter::repeat_n(b'\n', 2 - newlines));
        }
        result.extend_from_slice(rendered);
        result
    }

    pub fn pop(&self) -> Result<Vec<u8>, String> {
        let Some(last) = self.blocks.last() else {
            return Err("document is empty".to_owned());
        };
        if self.blocks.len() == 1 && matches!(last.kind, BlockKind::Title { .. }) {
            return Err("nothing to pop: document only contains a title".to_owned());
        }
        let removed = if matches!(
            last.kind,
            BlockKind::Output { .. } | BlockKind::Image { .. }
        ) {
            2
        } else {
            1
        };
        let retained = self.blocks.len().saturating_sub(removed);
        let end = if retained == 0 {
            0
        } else {
            self.blocks[retained - 1].span.end
        };
        Ok(self.source[..end].to_vec())
    }

    pub fn replace_outputs(&self, replacements: &[(usize, Vec<u8>)]) -> Result<Vec<u8>, String> {
        let mut ordered = std::collections::BTreeMap::new();
        for (index, content) in replacements {
            let Some(Block {
                kind: BlockKind::Output { .. },
                ..
            }) = self.blocks.get(*index)
            else {
                return Err(format!("block {index} is not an output block"));
            };
            if ordered.insert(*index, content).is_some() {
                return Err(format!("duplicate output replacement for block {index}"));
            }
        }
        let mut result = Vec::new();
        let mut copied = 0;
        for (index, content) in ordered {
            let block = &self.blocks[index];
            if matches!(&block.kind, BlockKind::Output {content: existing} if existing == content) {
                continue;
            }
            result.extend_from_slice(&self.source[copied..block.span.start]);
            result.extend_from_slice(&render_output(content));
            copied = block.span.end;
        }
        result.extend_from_slice(&self.source[copied..]);
        Ok(result)
    }

    pub fn extract(&self, target: &[u8]) -> Vec<Vec<u8>> {
        let target = shell_quote(target);
        self.blocks
            .iter()
            .filter_map(|block| {
                let command = match &block.kind {
                    BlockKind::Title { title, .. } => [
                        b"mdrun init -- ",
                        target.as_slice(),
                        b" ",
                        &shell_quote(title),
                    ]
                    .concat(),
                    BlockKind::Commentary { text } => [
                        b"mdrun note -- ",
                        target.as_slice(),
                        b" ",
                        &shell_quote(text),
                    ]
                    .concat(),
                    BlockKind::Code {
                        code,
                        is_image: true,
                        ..
                    } => [
                        b"mdrun image -- ",
                        target.as_slice(),
                        b" ",
                        &shell_quote(code),
                    ]
                    .concat(),
                    BlockKind::Code {
                        lang,
                        code,
                        is_image: false,
                    } => [
                        b"mdrun exec -- ",
                        target.as_slice(),
                        b" ",
                        &shell_quote(lang),
                        b" ",
                        &shell_quote(code),
                    ]
                    .concat(),
                    BlockKind::Output { .. } | BlockKind::Image { .. } => return None,
                };
                Some(command)
            })
            .collect()
    }

    pub fn document_id(&self) -> Option<&[u8]> {
        self.blocks.iter().find_map(|block| match &block.kind {
            BlockKind::Title { document_id, .. } if !document_id.is_empty() => {
                Some(document_id.as_slice())
            }
            _ => None,
        })
    }
}

struct Line<'a> {
    text: &'a [u8],
    start: usize,
    end: usize,
}

fn scan_lines(source: &[u8]) -> Result<Vec<Line<'_>>, String> {
    let mut lines = Vec::new();
    let mut start = 0;
    for raw in source.split_inclusive(|&byte| byte == b'\n') {
        let terminated = raw.ends_with(b"\n");
        if raw.len() > 65_536 || (raw.len() == 65_536 && !terminated) {
            return Err("bufio.Scanner: token too long".to_owned());
        }
        let text = raw.strip_suffix(b"\n").unwrap_or(raw);
        let text = text.strip_suffix(b"\r").unwrap_or(text);
        lines.push(Line {
            text,
            start,
            end: start + raw.len(),
        });
        start += raw.len();
    }
    Ok(lines)
}

fn skip_separator(lines: &[Line<'_>], i: &mut usize) {
    if lines.get(*i).is_some_and(|line| line.text.is_empty()) {
        *i += 1;
    }
}

fn trim_stars(text: &[u8]) -> &[u8] {
    let start = text
        .iter()
        .position(|&byte| byte != b'*')
        .unwrap_or(text.len());
    let end = text
        .iter()
        .rposition(|&byte| byte != b'*')
        .map_or(start, |index| index + 1);
    &text[start..end]
}

fn find_bytes(text: &[u8], needle: &[u8]) -> Option<usize> {
    text.windows(needle.len())
        .position(|window| window == needle)
}

fn join_lines(lines: &[Line<'_>]) -> Vec<u8> {
    let mut result = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            result.push(b'\n');
        }
        result.extend_from_slice(line.text);
    }
    result
}

fn parse_image_ref(text: &[u8]) -> Option<(&[u8], &[u8])> {
    let rest = text.strip_prefix(b"![")?;
    let close_bracket = find_bytes(rest, b"](")?;
    let alt = &rest[..close_bracket];
    let filename = &rest[close_bracket + 2..];
    let close_paren = filename.iter().position(|&byte| byte == b')')?;
    (close_paren != 0).then_some((alt, &filename[..close_paren]))
}

pub fn render_title(title: &[u8], timestamp: &[u8], version: &[u8], id: &[u8]) -> Vec<u8> {
    let mut result = [b"# ".as_slice(), title, b"\n\n*", timestamp].concat();
    if !version.is_empty() {
        result.extend_from_slice(b" by mdrun ");
        result.extend_from_slice(version);
    }
    result.extend_from_slice(b"*\n");
    if !id.is_empty() {
        result.extend_from_slice(b"<!-- mdrun-id: ");
        result.extend_from_slice(id);
        result.extend_from_slice(b" -->\n");
    }
    result
}

pub fn render_note(text: &[u8]) -> Vec<u8> {
    [text, b"\n"].concat()
}

pub fn render_code(lang: &[u8], code: &[u8], is_image: bool) -> Vec<u8> {
    let marker = if is_image {
        b" {image}".as_slice()
    } else {
        b""
    };
    [b"```", lang, marker, b"\n", code, b"\n```\n"].concat()
}

pub fn render_output(content: &[u8]) -> Vec<u8> {
    let longest = content
        .split(|&byte| byte == b'\n')
        .map(|line| line.iter().take_while(|&&byte| byte == b'`').count())
        .max()
        .unwrap_or(0);
    let fence = vec![b'`'; 3.max(longest + 1)];
    [
        fence.as_slice(),
        b"output\n",
        content,
        fence.as_slice(),
        b"\n",
    ]
    .concat()
}

pub fn render_image(alt: &[u8], filename: &[u8]) -> Vec<u8> {
    [b"![", alt, b"](", filename, b")\n"].concat()
}

pub fn shell_quote(text: &[u8]) -> Vec<u8> {
    if !text.is_empty()
        && !text
            .iter()
            .any(|byte| b" \t\n\"'\\|&;()<>$`!{}[]*?#~".contains(byte))
    {
        return text.to_vec();
    }
    let mut result = vec![b'\''];
    for &byte in text {
        if byte == b'\'' {
            result.extend_from_slice(b"'\\''");
        } else {
            result.push(byte);
        }
    }
    result.push(b'\'');
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEGACY: &[u8] =
        b"# Demo\n\n*2026-01-02T03:04:05Z by Showboat 0.6.1*\n<!-- showboat-id: old-id -->\n";

    #[test]
    fn canonical_block_order_and_legacy_metadata_match_replay_indices() {
        let source = [LEGACY, b"\nProse\n\n```sh\nprintf hi\n```\n\n```output\nhi\n```\n\n```bash {image}\npic.png\n```\n\n![A](kept.png) ignored\n"].concat();
        let document = Document::parse(&source).unwrap();
        assert_eq!(document.source, source);
        assert_eq!(document.blocks.len(), 6);
        assert_eq!(document.document_id(), Some(b"old-id".as_slice()));
        assert_eq!(
            document.blocks[0].kind,
            BlockKind::Title {
                title: b"Demo".to_vec(),
                timestamp: b"2026-01-02T03:04:05Z".to_vec(),
                version: b"0.6.1".to_vec(),
                document_id: b"old-id".to_vec(),
            }
        );
        assert_eq!(document.blocks[0].span, 0..LEGACY.len());
        assert_eq!(
            document.blocks[1].kind,
            BlockKind::Commentary {
                text: b"Prose".to_vec()
            }
        );
        assert_eq!(
            document.blocks[2].kind,
            BlockKind::Code {
                lang: b"sh".to_vec(),
                code: b"printf hi".to_vec(),
                is_image: false
            }
        );
        assert_eq!(
            document.blocks[3].kind,
            BlockKind::Output {
                content: b"hi\n".to_vec()
            }
        );
        assert_eq!(
            document.blocks[4].kind,
            BlockKind::Code {
                lang: b"bash".to_vec(),
                code: b"pic.png".to_vec(),
                is_image: true
            }
        );
        assert_eq!(
            document.blocks[5].kind,
            BlockKind::Image {
                alt: b"A".to_vec(),
                filename: b"kept.png".to_vec()
            }
        );
    }

    #[test]
    fn title_is_recognized_only_at_start_and_invalid_images_remain_prose() {
        let document =
            Document::parse(b"\n# Heading\n![empty]()\n![broken]\n\n```sh\n:\n```\n# Later\n")
                .unwrap();
        assert_eq!(document.blocks.len(), 3);
        assert_eq!(
            document.blocks[0].kind,
            BlockKind::Commentary {
                text: b"\n# Heading\n![empty]()\n![broken]".to_vec()
            }
        );
        assert_eq!(
            document.blocks[2].kind,
            BlockKind::Commentary {
                text: b"# Later".to_vec()
            }
        );
        let bom = Document::parse(b"\xef\xbb\xbf# Heading\n").unwrap();
        assert_eq!(
            bom.blocks[0].kind,
            BlockKind::Commentary {
                text: b"\xef\xbb\xbf# Heading".to_vec()
            }
        );
    }

    #[test]
    fn output_fence_requires_exact_width_and_keeps_binary_bytes() {
        let document = Document::parse(b"````output\n```\n`````\n\xff\0\n````\n").unwrap();
        assert_eq!(document.blocks.len(), 1);
        assert_eq!(
            document.blocks[0].kind,
            BlockKind::Output {
                content: b"```\n`````\n\xff\0\n".to_vec()
            }
        );
        assert_eq!(document.blocks[0].span, 0..29);
    }

    #[test]
    fn image_marker_and_output_info_are_exact() {
        let document =
            Document::parse(b"```sh {image} \na\n```\n\n```output {image}\nb\n```\n").unwrap();
        assert_eq!(
            document.blocks[0].kind,
            BlockKind::Code {
                lang: b"sh {image} ".to_vec(),
                code: b"a".to_vec(),
                is_image: false
            }
        );
        assert_eq!(
            document.blocks[1].kind,
            BlockKind::Code {
                lang: b"output".to_vec(),
                code: b"b".to_vec(),
                is_image: true
            }
        );
    }

    #[test]
    fn unclosed_fences_have_showboat_semantics_without_fabricating_source() {
        let code = Document::parse(b"```sh\none\n\ntwo").unwrap();
        assert_eq!(
            code.blocks[0].kind,
            BlockKind::Code {
                lang: b"sh".to_vec(),
                code: b"one\n\ntwo".to_vec(),
                is_image: false
            }
        );
        assert_eq!(code.blocks[0].span, 0..14);
        assert_eq!(
            code.append(&render_note(b"Note")),
            b"```sh\none\n\ntwo\n```\n\nNote\n"
        );
        let output = Document::parse(b"```output\nno newline").unwrap();
        assert_eq!(
            output.blocks[0].kind,
            BlockKind::Output {
                content: b"no newline\n".to_vec()
            }
        );
    }

    #[test]
    fn scanner_limit_matches_go_without_rejecting_non_utf8() {
        assert!(Document::parse(&vec![b'x'; 65_535]).is_ok());
        assert_eq!(
            Document::parse(&vec![b'x'; 65_536]).unwrap_err(),
            "bufio.Scanner: token too long"
        );
        let mut terminated = vec![b'x'; 65_535];
        terminated.push(b'\n');
        assert!(Document::parse(&terminated).is_ok());
        let mut too_long = vec![b'x'; 65_536];
        too_long.push(b'\n');
        assert!(Document::parse(&too_long).is_err());
    }

    #[test]
    fn append_retains_legacy_crlf_extra_blanks_and_prose_bytes() {
        let source = b"# Old\r\n\r\n*when by Showboat v0.6.1*\r\n<!-- showboat-id: keep -->\r\n\r\n\r\nText  \r\n\r\n\r\n";
        let document = Document::parse(source).unwrap();
        assert_eq!(
            document.append(&render_note(b"Added")),
            [source.as_slice(), b"Added\n"].concat()
        );
        assert_eq!(document.document_id(), Some(b"keep".as_slice()));
        assert_eq!(
            document.blocks[1].kind,
            BlockKind::Commentary {
                text: b"\nText  ".to_vec()
            }
        );
        let no_newline = Document::parse(b"text").unwrap();
        assert_eq!(
            no_newline.append(&render_note(b"Added")),
            b"text\n\nAdded\n"
        );
    }

    #[test]
    fn canonical_note_and_pop_match_expected_header_bytes() {
        let document = Document::parse(LEGACY).unwrap();
        let with_note = document.append(&render_note(b"Some text"));
        assert_eq!(with_note, [LEGACY, b"\nSome text\n"].concat());
        assert_eq!(Document::parse(&with_note).unwrap().pop().unwrap(), LEGACY);
        assert_eq!(
            document.pop().unwrap_err(),
            "nothing to pop: document only contains a title"
        );
        assert_eq!(
            Document::parse(b"\n\n").unwrap().pop().unwrap_err(),
            "document is empty"
        );
    }

    #[test]
    fn pop_removes_output_and_previous_block_even_when_not_code() {
        for entry in [
            b"\n```sh\n:\n```\n\n```output\nold\n```\n".as_slice(),
            b"\n```bash {image}\npic\n```\n\n![A](kept.png)\n",
            b"\nUnpaired prose\n\n```output\nold\n```\n",
        ] {
            let source = [LEGACY, entry].concat();
            assert_eq!(Document::parse(&source).unwrap().pop().unwrap(), LEGACY);
        }
        assert_eq!(
            Document::parse(b"```output\nold\n```\n")
                .unwrap()
                .pop()
                .unwrap(),
            b""
        );
    }

    #[test]
    fn replacing_outputs_retains_all_unmodified_source_and_legacy_metadata() {
        let source = b"# Old\r\n\r\n*when by Showboat 0.6.1*\r\n<!-- showboat-id: keep -->\r\n\r\n```sh\r\nprintf new\r\n```\r\n\r\n\r\n```output\r\nold\r\n```\r\n\r\nTail  \r\n\r\n";
        let document = Document::parse(source).unwrap();
        assert_eq!(document.replace_outputs(&[(2, b"new\n\xff\n".to_vec())]).unwrap(), b"# Old\r\n\r\n*when by Showboat 0.6.1*\r\n<!-- showboat-id: keep -->\r\n\r\n```sh\r\nprintf new\r\n```\r\n\r\n\r\n```output\nnew\n\xff\n```\n\r\nTail  \r\n\r\n");
        assert_eq!(document.replace_outputs(&[]).unwrap(), source);
        assert_eq!(
            document.replace_outputs(&[(2, b"old\n".to_vec())]).unwrap(),
            source
        );
        assert!(document.replace_outputs(&[(1, Vec::new())]).is_err());
        assert!(document.replace_outputs(&[(20, Vec::new())]).is_err());
        assert!(
            document
                .replace_outputs(&[(2, Vec::new()), (2, Vec::new())])
                .is_err()
        );
    }

    #[test]
    fn rendering_matches_upstream_fences_and_no_newline_behavior() {
        assert_eq!(render_note(b"a\n"), b"a\n\n");
        assert_eq!(
            render_code(b"sh", b"echo hi", false),
            b"```sh\necho hi\n```\n"
        );
        assert_eq!(
            render_code(b"bash", b"pic.png", true),
            b"```bash {image}\npic.png\n```\n"
        );
        assert_eq!(render_image(b"Alt", b"pic.png"), b"![Alt](pic.png)\n");
        assert_eq!(render_output(b"hello"), b"```output\nhello```\n");
        assert_eq!(
            render_output(b"\xff\n````text\n"),
            b"`````output\n\xff\n````text\n`````\n"
        );
        assert_eq!(render_output(b" ```\n"), b"```output\n ```\n```\n");
        assert_eq!(render_output(b""), b"```output\n```\n");
    }

    #[test]
    fn new_titles_use_mdrun_metadata_that_can_be_read_again() {
        let title = render_title(b"New", b"2026-01-02T03:04:05Z", b"0.1.0", b"new-id");
        assert_eq!(
            title,
            b"# New\n\n*2026-01-02T03:04:05Z by mdrun 0.1.0*\n<!-- mdrun-id: new-id -->\n"
        );
        let document = Document::parse(&title).unwrap();
        assert_eq!(document.document_id(), Some(b"new-id".as_slice()));
        assert!(
            matches!(&document.blocks[0].kind, BlockKind::Title {version, ..} if version == b"0.1.0")
        );
        assert_eq!(
            render_title(b"New", b"when", b"", b""),
            b"# New\n\n*when*\n"
        );
    }

    #[test]
    fn extraction_quotes_commands_and_preserves_binary_text() {
        let source = [LEGACY, b"\nText with 'quotes', $dollars\nand \xff.\n\n```sh\nprintf 'hi\\n'\n```\n\n```output\nhi\n```\n\n```bash {image}\n![A](pic.png)\n```\n\n![A](copy.png)\n"].concat();
        let document = Document::parse(&source).unwrap();
        assert_eq!(document.extract(b"a path's file.md"), vec![
            b"mdrun init -- 'a path'\\''s file.md' Demo".to_vec(),
            b"mdrun note -- 'a path'\\''s file.md' 'Text with '\\''quotes'\\'', $dollars\nand \xff.'".to_vec(),
            b"mdrun exec -- 'a path'\\''s file.md' sh 'printf '\\''hi\\n'\\'''".to_vec(),
            b"mdrun image -- 'a path'\\''s file.md' '![A](pic.png)'".to_vec(),
        ]);
        assert_eq!(shell_quote(b""), b"''");
        assert_eq!(shell_quote(b"plain:\r\xff"), b"plain:\r\xff");
        assert_eq!(shell_quote(b"a'b"), b"'a'\\''b'");
        for special in b" \t\n\"'\\|&;()<>$`!{}[]*?#~" {
            assert_eq!(shell_quote(&[*special]).first(), Some(&b'\''));
        }
    }

    #[test]
    fn extraction_protects_hyphen_positionals_and_quotes_interpreter_paths() {
        let source = b"# -dash\n\n*when*\n\n- note\n\n```/a path/interpreter\n-code\n```\n\n```bash {image}\n-image.png\n```\n\n![A](copy.png)\n";
        let document = Document::parse(source).unwrap();
        assert_eq!(
            document.extract(b"-record.md"),
            vec![
                b"mdrun init -- -record.md -dash".to_vec(),
                b"mdrun note -- -record.md '- note'".to_vec(),
                b"mdrun exec -- -record.md '/a path/interpreter' -code".to_vec(),
                b"mdrun image -- -record.md -image.png".to_vec(),
            ]
        );
    }
}
