# mdrun

[![CI](https://github.com/mbackschat/mdrun/actions/workflows/ci.yml/badge.svg)](https://github.com/mbackschat/mdrun/actions/workflows/ci.yml)
[![Release builds](https://github.com/mbackschat/mdrun/actions/workflows/release.yml/badge.svg)](https://github.com/mbackschat/mdrun/actions/workflows/release.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-2024-orange.svg)](Cargo.toml)
[![Status: in development](https://img.shields.io/badge/status-in%20development-yellow.svg)](docs/PLAN.md)

**Create, run, and verify executable Markdown documents.**

Inspired by [Showboat](https://github.com/simonw/showboat), created by Simon Willison. Showboat established the workflow this project builds on: executable Markdown combining prose, commands and captured output, with replay to verify the results. Credit for that original idea and command model belongs to Showboat and its contributors.

Current status: the seven document commands and read-only command discovery pass macOS/Linux qualification with cold execution. Native managed sessions and a cooperative Node SDK are under implementation and qualification; their API remains experimental. JVM/Python SDKs and consumer migration readiness remain planned. See the [execution checklist](docs/plans/execution.md) for outstanding gates.

## Why a new implementation?

| Goal | Motivation |
| --- | --- |
| Small native binary | Rust lets us pursue a compact native host. Finished-product size and comparisons will be measured against identified binaries; historical prototype measurements are not product promises. |
| Standalone installation | Distribute a native executable that starts without Rust, Java, Node, Python or a separate Showboat installation. Target commands still need their own runtimes and tools; macOS uses system libraries, and static Linux release binaries are the packaging target. |
| Warm JVM, Node and Python execution | Reuse qualified runtime/application initialization across commands to reduce repeated startup cost, while preserving independent cold execution for parity and release checks. JVM and Node come first, then Python; the [execution plan](docs/plans/execution.md) owns their implementation and qualification. |
| One reusable execution engine | Move session supervision, forwarding, streaming and cleanup into mdrun so consumer repositories can retain thin build/policy/application adapters instead of their own warm-execution harnesses. |
| Familiar documents and commands | Preserve pinned Showboat behavior and existing recordings while giving the new implementation its own truthful identity. |

[Showboat also provides compiled native binaries](https://github.com/simonw/showboat/blob/d531261b8faf0c388b02c7891d50f1f47c3e2b52/README.md#installation). mdrun's motivation combines compact distribution with built-in cooperative warm execution. It is an independent implementation and is not affiliated with or endorsed by Showboat. “Standalone” describes runtime installation requirements; it does not rule out compiled Rust library dependencies where justified.

## Document workflow

```sh
mdrun init demo.md 'A runnable example'
mdrun note demo.md 'Run a shell command and record its output.'
mdrun exec demo.md sh 'printf "hello\n"'
mdrun verify demo.md
mdrun extract demo.md --filename rebuilt.md
```

`note`, `exec` and `image` accept stdin when their text/code/image argument is omitted. `exec` records combined stdout/stderr even when the command fails and returns its exit status. `verify --output candidate.md` writes updated captures to a separate candidate; selecting the input path explicitly updates it in place. `pop` removes the final note or command/output pair. Use `--workdir DIR` for the executed program's directory. See `mdrun help COMMAND` for command options; use `--` before literal positional values beginning with a hyphen.

Legacy Showboat provenance remains intact; new documents identify mdrun. `SHOWBOAT_REMOTE_URL` supports compatible section/image HTTP delivery; delivery failures warn without discarding the local recording. Tests compare meaningful document/execution behavior with pinned Showboat, while clap's normal help and validation have independent tests.

## Find and preview commands

```sh
mdrun list demo.md
mdrun list demo.md another.md --filter hello --json
mdrun preview demo.md 2
```

`list` includes every executable block recognized by replay, including unnamed commands without recorded output. It excludes output and image blocks. `--filter` matches a case-sensitive literal substring in the filename, nearest preceding title/ATX heading or complete command text. Plain rows show file/line, block index, heading, interpreter and the command's first line. JSON is an array with `file`, `block`, `line`, `heading` (null when absent), `interpreter` and `summary`; it requires UTF-8 inventory text and emits nothing if any input fails.

`preview` writes the selected block's complete fenced source verbatim, including original line endings and arbitrary bytes. Its zero-based block index is the same index reported by `verify`, not a permanent ID or executable-only ordinal. Both commands inspect documents without executing code, expanding shell syntax or starting workers. Names, tags and selective block execution remain deferred.

## Distribution

There is no product release yet. Release automation is prepared for macOS Apple Silicon and Linux x86_64/arm64, with native archives, static Linux binaries, `.deb`/`.rpm` packages and a Homebrew formula for the existing `mbackschat/tap` channel. See [distribution and release instructions](docs/distribution.md) for dry-run builds and publication steps. Installation commands will become user-facing once a qualified release and tap formula are published.

## Develop

Install Rust/Cargo with rustfmt and Clippy; prefer Homebrew on macOS. The host needs no target language runtime to start. Its focused compiled Rust dependencies provide standard CLI parsing, timestamps/UUIDs and HTTP/TLS; see [dependency choices](docs/dependencies.md). Rust edition 2024 follows the supplied Rust prototype; a minimum supported Rust version has not been qualified.

```sh
cargo build --locked
cargo run --locked -- --help
cargo test --locked
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo build --release --locked
```

The executable is `target/release/mdrun`. Keep [Cargo.lock](Cargo.lock) in version control. Integration tests are modules of one binary at [tests/integration](tests/integration). The [CI workflow](.github/workflows/ci.yml) checks the tool on macOS and Linux; configured jobs are not evidence of hosted qualification.

## Project map

| Start here | Purpose |
| --- | --- |
| [Contributor instructions](CLAUDE.md) | Coding-agent workflow; `AGENTS.md` is a relative symlink to this file |
| [Documentation](docs/README.md) | Current documents and their ownership |
| [Product contract](docs/PRODUCT.md) | Supported behavior and accepted product goals, clearly separated |
| [Implementation plan](docs/PLAN.md) | Ordered milestones and completion gates |
| [Compatibility qualification](docs/plans/compatibility.md) | Pinned upstream behavior and differential acceptance |
| [Research provenance](docs/research/README.md) | Extracted conclusions and source provenance |
| [Distribution](docs/distribution.md) | macOS/Linux CI, Homebrew and Linux release packages |
| [Deferred work](docs/DEFERRED.md) | Explicitly deferred scope and reopening conditions |

LOCmetry informed repository organization and, at the owner's request, distribution conventions. Dependencies, runtime design and product features are not inherited from it. CLI options use clap, as requested. Familiar Showboat commands and document behavior are retained; incidental parsing quirks and exact help/error wording are not compatibility goals.

Licensed under the [MIT License](LICENSE), selected by the owner for mdrun. Cargo registry publication is disabled (`publish = false`); GitHub releases and Homebrew/Linux distribution are independent of that setting. Preserve upstream license notices if upstream code is incorporated later.
