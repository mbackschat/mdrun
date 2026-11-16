# Differential compatibility checks

This development-only Python harness runs a finite corpus against independent workspaces for pinned Showboat and mdrun. It compares each command's status, stdout, stderr, all regular file bytes/paths, directories and HTTP form/multipart values/uploads. Raw streams, files and request bodies are retained as base64 JSON for inspection. It does not require or copy the discarded feasibility prototype. Python and Go are qualification tools, not mdrun runtime dependencies.

## Prepare the oracle

Use Go 1.27.4 to build a clean Showboat v0.6.1 checkout, pinned to `d531261b8faf0c388b02c7891d50f1f47c3e2b52`. The preparation script checks both commit/tag and cleanliness, disables CGO and records the build flags/toolchain and resulting SHA-256 beside the binary. The upstream source remains in a disposable checkout outside this repository.

```sh
git clone --branch v0.6.1 --depth 1 https://github.com/simonw/showboat.git /tmp/mdrun-showboat-source
python3 tests/compatibility/prepare.py --source /tmp/mdrun-showboat-source --output /tmp/mdrun-showboat-oracle
```

Output and result directories must be new, so qualification cannot silently replace earlier evidence. `prepare.py` uses the configured Go module cache/proxy; ordinary mdrun tests do not download Go or Showboat.

## Run

```sh
python3 -m unittest discover -s tests/compatibility -p 'test_*.py'
python3 tests/compatibility/oracle.py --suite upstream --oracle /tmp/mdrun-showboat-oracle/showboat --candidate /tmp/mdrun-showboat-oracle/showboat --results /tmp/mdrun-oracle-selfcheck
cargo build --locked
python3 tests/compatibility/oracle.py --suite behavior --oracle /tmp/mdrun-showboat-oracle/showboat --candidate target/debug/mdrun --results /tmp/mdrun-differential
```

Use `--case SUBSTRING` for focused work. A missing case match is an error. A nonzero result means a difference or a harness failure; inspect the saved observations. An oracle-to-itself pass checks fixture independence and nondeterminism handling; it is not product compatibility evidence.

## Suite boundary

The default `behavior` suite retains command execution, documents, status, output, filesystem effects and remote payloads. Parser/help/error policy belongs to clap and independent Rust tests. The `upstream` suite retains the full upstream-quirk corpus for optional Showboat selfchecks, with exact program names in stdout/stderr. As of 2026-11-16, the suites contain 75 and 118 cases respectively; the summary records which suite ran.

| Cases excluded from `behavior` | Reason and replacement oracle |
| --- | --- |
| `cli-*` | Upstream help/version branding, argument grammar, arity and parser diagnostics are covered by independent Rust CLI assertions. |
| `init-extra`, `note-extras`, `exec-extra` | Upstream ignores extra positionals; clap owns acceptance and errors. |
| `note-literal-option`, `note-global-in-text` | Upstream strips global flags from text or accepts unknown flag-shaped positionals; clap owns option/positional separation. |
| `workdir-repeated`, `verify-repeated-output`, `extract-repeated` | Upstream's last-value behavior for repeated options is parser policy. |
| `verify-output-missing-value`, `extract-missing-value` | Missing option values use clap diagnostics and status. |
| `verify-output-before-file` | Upstream treats this valid clap option position as a filename; Rust CLI tests own supported option placement. |
| `verify-unknown-extra` | Upstream silently ignores unknown options; clap owns rejection. |
| `text-crlf`, `text-extra-blanks` | mdrun preserves these source bytes instead of reproducing upstream rewrites; independent Rust source-preservation assertions remain required. |

Excluded source differences are not normalized. Other document bytes, command output whitespace, statuses, file effects and HTTP values remain exact.

## Explicit projections and controls

The negative controls mutate status, either output stream, file effects and captured HTTP fields, and distinguish whitespace/UUID payloads from metadata. Generated timestamps and UUID-v4 headers are validated and mapped only for explicitly identified new documents; existing legacy values remain exact and generated identity must stay stable through later steps. Generated image names are mapped with their document and upload references. Multipart boundary strings and form-encoding spelling are excluded from semantic comparison, while raw requests remain available.

In `behavior`, only successful declared `extract` steps project the emitted program name. The script executes against inert `showboat`/`mdrun` shims in another temporary workspace, recording arguments in a separate capture log. The projection permits only emitted `init`, `note`, `exec` and `image` command forms and compares every argument byte. It accepts the optional `--` immediately after an `mdrun` subcommand; later literal `--` arguments remain exact. Equivalent shell quoting is accepted, while malformed quoting, unsupported commands, printed capture records and unexpected file effects fail validation. Literal product names inside quoted multiline prose are never replaced. The reconstruction fixture additionally replays the original emitted script using the actual binary in a separate workspace and compares its document/image effects.

For a declared non-success HTTP fixture, `behavior` maps only an exact `showboat:`/`mdrun:` warning for that fixture's `server returned N` status. Other warning text, extra diagnostics and incorrect status numbers remain visible. Help/version output and parser failures are excluded cases, not normalized output. Raw extraction scripts and warnings remain in the evidence records, including validation failures.

The environment is whitelisted and each case uses a fresh temporary root. Loopback capture needs permission to bind a local socket. Commands run with a deadline; all started process groups are reclaimed when the case finishes, including timeout and failure paths. These fixtures run trusted test code; the harness is not a sandbox or arbitrary-command isolation service.

The [compatibility plan](../../docs/plans/compatibility.md) owns the source inventory, contract decisions and milestone status. Extend the corpus for a discovered behavior or regression, rather than treating its current case count as a completeness argument.
