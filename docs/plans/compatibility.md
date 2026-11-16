# Showboat compatibility qualification

Status: M1 harness passed local and hosted macOS/Linux qualification on 2026-11-16; M2 product compatibility remains open. The owner accepted normal clap parsing/help/errors instead of exact CLI mimicry on the same date. [PRODUCT](../PRODUCT.md) owns the accepted requirement; [research provenance](../research/README.md) records its origin.

## Baseline

Credit [Showboat and Simon Willison](https://github.com/simonw/showboat) for the original executable-document workflow and CLI model. Upstream's pinned README is a discovery source, not a complete behavioral oracle. Capture the upstream license/notice obligations before reusing any source.

Upstream uses Apache-2.0. Its source and license were inspected in a temporary checkout; no upstream implementation, tests or full checkout is vendored here. Keep required attribution/license notices if implementation later incorporates upstream material. Go is used only to build the test oracle; the production host is Rust and needs no Go installation.

## Source-grounded behavior

| Source at the verified pin | Observable contract and executable case families |
| --- | --- |
| [main.go](https://github.com/simonw/showboat/blob/d531261b8faf0c388b02c7891d50f1f47c3e2b52/main.go) | `cli-*`, `workdir-*`: global `--version`/`--workdir` scan every argument position; last workdir wins; no general `--` terminator or equals syntax; help is a command token; no-args/unknown command exit 1; command arity is minimum-only and some surplus arguments are ignored |
| [init.go](https://github.com/simonw/showboat/blob/d531261b8faf0c388b02c7891d50f1f47c3e2b52/cmd/init.go), [build.go](https://github.com/simonw/showboat/blob/d531261b8faf0c388b02c7891d50f1f47c3e2b52/cmd/build.go) | `init-*`, `note-*`, `exec-*`: init rejects existing paths; UTC RFC3339 and UUID v4; first text argument or all stdin; failures still recorded; missing document checked before execution |
| [runner.go](https://github.com/simonw/showboat/blob/d531261b8faf0c388b02c7891d50f1f47c3e2b52/exec/runner.go) | `exec-*`, `workdir-*`: interpreter receives `-c` and complete code, stdout/stderr share capture, command stdin is EOF, workdir affects child execution rather than document paths; signal death becomes CLI status 255 on these Unix targets |
| [image.go](https://github.com/simonw/showboat/blob/d531261b8faf0c388b02c7891d50f1f47c3e2b52/exec/image.go) | `image-*`: PNG/JPG/JPEG/GIF/SVG extension checks, eight UUID hex characters plus UTC date in copied filename; bytes copied without image decoding; explicit/derived alt; image input resolves in CLI cwd even with workdir |
| [pop.go](https://github.com/simonw/showboat/blob/d531261b8faf0c388b02c7891d50f1f47c3e2b52/cmd/pop.go) | `pop-*`: remove final entry and command/output pair, protect title, reject empty document, retain copied image files |
| [verify.go](https://github.com/simonw/showboat/blob/d531261b8faf0c388b02c7891d50f1f47c3e2b52/cmd/verify.go) | `verify-*`: compare captured bytes with adjacent output; ignore application nonzero status when output agrees; execute unpaired commands but skip image commands; report zero-based parsed-block indexes; `--output` writes a copy even on differences; explicitly choosing the input path updates it |
| [extract.go](https://github.com/simonw/showboat/blob/d531261b8faf0c388b02c7891d50f1f47c3e2b52/cmd/extract.go) | `extract-*`: emitted init/note/exec/image sequence, quoting, filename override, ignored/missing/repeated options, reconstruction in a separate workspace with images and multiline quoted prose |
| [parser.go](https://github.com/simonw/showboat/blob/d531261b8faf0c388b02c7891d50f1f47c3e2b52/markdown/parser.go), [writer.go](https://github.com/simonw/showboat/blob/d531261b8faf0c388b02c7891d50f1f47c3e2b52/markdown/writer.go) | `text-*`, `verify-*`, `exec-*`: beginning-only title/provenance, fences/long fences/image marker, LF reconstruction, output fence sizing, final newline behavior, CRLF/BOM/blank lines, unclosed fences and scanner long-line errors |
| [remote.go](https://github.com/simonw/showboat/blob/d531261b8faf0c388b02c7891d50f1f47c3e2b52/cmd/remote.go) | `remote-*`: init/note/exec/pop forms, image multipart, generated and existing IDs, no legacy-ID request when absent, raw invalid-UTF-8 output bytes, server-error warning without command failure; ten-second client timeout is source-observed and remains a focused qualification case |

The source tests were reviewed alongside these implementation files. [cases.py](../../tests/compatibility/cases.py) is the finite executable inventory; add cases for concrete uncovered behavior rather than inferring completeness from the count.

## Acceptance inventory

Each row must become executable cases against the pinned upstream binary and mdrun in independent equivalent fixtures. Compare stdout, stderr, status and file/HTTP effects; retain raw failures. Test a broken variant or negative control for every important oracle. Expand this inventory after inspecting the pinned source and tests rather than treating it as exhaustive.

| Surface | Required cases and oracle |
| --- | --- |
| CLI grammar | Independent clap contract tests: seven commands, normal help/version, familiar options, accepted positions, arity/unknown/extra argument errors, native paths and `--`. Historical Showboat quirks stay in the upstream inventory but are not product parity gates. |
| `init` | Title, existing/missing file paths, timestamp and UUID invariants, metadata structure, workdir effects |
| `note` | Argument and stdin input, empty input, multiline Markdown, Unicode and preserved surrounding bytes |
| `exec` | Argument/stdin code, interpreter lookup, cwd, stdout/stderr capture semantics, nonzero/signal outcomes, recorded failures, quoting and invalid text representation |
| `image` | Path and Markdown image forms, alt text, copying/naming, collisions, missing files and bytes preserved |
| `pop` | Each entry type, empty documents, command/output pairing and preservation of earlier content |
| `verify` | Exact replay/diffs, multiple blocks, failed commands, unmanaged prose/fences, no source mutation, updated-copy destination and legacy metadata preservation |
| `extract` | Command sequence, shell quoting, title/prose/code/images, filename override and reconstruction in an independent workspace |
| Text fidelity | LF/CRLF, final newline, embedded fences, Unicode/BOM, binary transport versus textual recording and malformed managed documents |
| Remote streaming | `SHOWBOAT_REMOTE_URL`, UUID association, form/multipart fields, images, warnings and unreachable/non-success responses using a local HTTP capture fixture |
| Isolation | No installed Showboat needed by mdrun; different fixtures for oracle/product runs; deterministic inputs and explicit environment |

## Intentional differences

New mdrun documents and version output identify mdrun. Legacy bylines and `showboat-id` metadata survive reads and updated-copy replay unchanged. Timestamp/UUID comparisons establish format, identity association and round-trip behavior rather than requiring random/time values to match. Help identifies mdrun and follows clap. The familiar commands/options remain; parser diagnostics, no-argument behavior, error status 2, rejected extra arguments and normal `--` handling deliberately follow clap rather than Showboat’s ad hoc grammar.

Normalize only explicitly listed nondeterministic or branding fields, separately from the raw observations. Do not normalize output whitespace, ordering, exit codes, warnings, file bytes or failed-command behavior without a documented product decision. Additional fast/cold/session syntax must not change the meaning of legacy commands. Parser-only upstream cases are explicitly separated from behavioral parity. Help/version have independent product assertions; extraction and remote-warning identity projection must be narrowly scoped, with no global product-name replacement.

## Dependency decision

Use clap for all CLI parsing. The owner explicitly relaxed incidental parser compatibility and prohibited custom option parsing; the ordinary consumer forms remain supported. Adopt stable clap 4.6.7 (published 2026-09-14), age-qualified with its complete resolved graph. Source-preserving document edits, HTTP/TLS, timestamps/UUIDs and process support are assessed independently when implemented; no dependency list is inherited from LOCmetry.

## Completion

M1 completes with the pin, source/executable identities, finite expanded matrix, narrow normalization rules and an oracle harness that detects deliberately divergent results. M2 completes when the production commands pass every required case on the qualified platforms, with original-file preservation and standalone operation independently checked. Preserve failures and resolutions as evidence, and replace planned status only after those checks run.

Local M1 evidence on 2026-11-16: eight harness guards pass, including invalid UTF-8 HTTP bytes and a disabled-cleanup negative control; 117 cases agree across independent upstream runs, with the added extraction-reconstruction case passing separately. The bootstrap negative control produces differences in status, both output streams and document effects. Two clean builds have identical hashes. A fresh MEDIUM review found the form-decoding and child-cleanup defects, both fixed with guards, and prompted fixed-legacy-ID and extraction-reconstruction cases. Hosted macOS/Linux oracle self-comparison passed at `607cadc`; it is distinct from M2 product qualification.

Local M2 evidence on 2026-11-16: all 75 required behavioral cases agree against the identified oracle; all 118 historical upstream cases pass self-comparison, and 11 harness guards pass. The [harness contract](../../tests/compatibility/README.md) explicitly lists the parser-policy and source-preservation exclusions. The qualified macOS arm64 candidate has SHA-256 `7263e207c21851fa08b38e456d41c1fe219cec4bc060e9762391ec18d4810ef9`. All 20 unit and 14 integration tests pass, including ordinary clap syntax, native argument bytes, source preservation, shell reconstruction, refused HTTP connections and an unanswered request reaching the actual ten-second timeout without losing local recording or command status. Formatting, Clippy and release build pass. A HIGH review found missing positional terminators and unquoted interpreter paths in extraction; both are fixed with failing-then-passing reconstruction guards. The binary starts with an empty environment and links only macOS system libraries; hosted product qualification remains pending.
