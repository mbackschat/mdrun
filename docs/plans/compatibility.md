# Showboat compatibility qualification

Status: M1 harness established locally on 2026-11-16; M2 product compatibility remains open. [PRODUCT](../PRODUCT.md) owns the accepted requirement; [research provenance](../research/README.md) records its origin.

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
| CLI grammar | Every command, aliases/help, global/command option positions, repeated/missing/unknown options, arity errors, paths with spaces, `--` handling and exact status/stream behavior |
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

New mdrun documents and version output identify mdrun. Legacy bylines and `showboat-id` metadata survive reads and updated-copy replay unchanged. Timestamp/UUID comparisons establish format, identity association and round-trip behavior rather than requiring random/time values to match. Help may identify mdrun and document its additional execution features; required upstream syntax and errors must still be inventoried and tested.

Normalize only explicitly listed nondeterministic or branding fields, separately from the raw observations. Do not normalize output whitespace, ordering, exit codes, warnings, file bytes or failed-command behavior without a documented product decision. Additional fast/cold/session syntax must not change the meaning of legacy commands. The harness currently retains version/help/extract branding differences visibly until M2 implements their exact approved representations; no global product-name replacement is allowed.

## Dependency decision

Use a small explicit parser over native arguments for the legacy surface. The upstream scan/arity/ignored-argument rules are simple but differ from conventional parser defaults; wrapping `clap` to undo those defaults would add work and dependencies without simplifying this contract. New subcommands may use separately specified strict parsing. Source-preserving document edits, HTTP/TLS, timestamps/UUIDs and process support are assessed independently when implemented; no dependency list is inherited from LOCmetry.

## Completion

M1 completes with the pin, source/executable identities, finite expanded matrix, narrow normalization rules and an oracle harness that detects deliberately divergent results. M2 completes when the production commands pass every required case on the qualified platforms, with original-file preservation and standalone operation independently checked. Preserve failures and resolutions as evidence, and replace planned status only after those checks run.

Local M1 evidence on 2026-11-16: eight harness guards pass, including invalid UTF-8 HTTP bytes and a disabled-cleanup negative control; 117 cases agree across independent upstream runs, with the added extraction-reconstruction case passing separately. The bootstrap negative control produces differences in status, both output streams and document effects. Two clean builds have identical hashes. A fresh MEDIUM review found the form-decoding and child-cleanup defects, both fixed with guards, and prompted fixed-legacy-ID and extraction-reconstruction cases. macOS/Linux CI is configured for oracle self-comparison; it is distinct from M2 product qualification.
