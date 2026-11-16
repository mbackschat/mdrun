# Showboat compatibility qualification

Status: planned M1/M2 work. This inventory is not a passing compatibility claim. [PRODUCT](../PRODUCT.md) owns the accepted requirement; [research provenance](../research/README.md) records its origin.

## Baseline

Credit [Showboat and Simon Willison](https://github.com/simonw/showboat) for the original executable-document workflow and CLI model. Upstream's pinned README is a discovery source, not a complete behavioral oracle. Capture the upstream license/notice obligations before reusing any source.

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

Normalize only explicitly listed nondeterministic or branding fields, separately from the raw observations. Do not normalize output whitespace, ordering, exit codes, warnings, file bytes or failed-command behavior without a documented product decision. Additional fast/cold/session syntax must not change the meaning of legacy commands.

## Dependency decision

Evaluate CLI parsing support against the inventory before choosing it. `clap` is a candidate for options and discoverable help; its default grammar, diagnostics and exit codes are not assumed compatible. Source-preserving Markdown editing, HTTP/TLS, timestamps/UUIDs and process support may justify independent dependencies. Record why each is necessary; no full dependency list is inherited from LOCmetry.

## Completion

M1 completes with the pin, source/executable identities, finite expanded matrix, narrow normalization rules and an oracle harness that detects deliberately divergent results. M2 completes when the production commands pass every required case on the qualified platforms, with original-file preservation and standalone operation independently checked. Preserve failures and resolutions as evidence, and replace planned status only after those checks run.
