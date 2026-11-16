# mdrun

[![CI](https://github.com/mbackschat/mdrun/actions/workflows/ci.yml/badge.svg)](https://github.com/mbackschat/mdrun/actions/workflows/ci.yml)
[![Release builds](https://github.com/mbackschat/mdrun/actions/workflows/release.yml/badge.svg)](https://github.com/mbackschat/mdrun/actions/workflows/release.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-2024-orange.svg)](Cargo.toml)
[![Status: bootstrap](https://img.shields.io/badge/status-bootstrap-yellow.svg)](docs/PLAN.md)

**Create, run, and verify executable Markdown documents.**

Inspired by [Showboat](https://github.com/simonw/showboat), created by Simon Willison. Showboat established the workflow this project builds on: executable Markdown combining prose, commands and captured output, with replay to verify the results. Credit for that original idea and command model belongs to Showboat and its contributors.

Current status: project bootstrap. The executable supports help and version output only; document commands and warm execution are planned. The preserved feasibility prototype is separate from the product and does not establish production compatibility or lifecycle guarantees.

## Why a new implementation?

| Goal | Motivation |
| --- | --- |
| Small native binary | Rust lets us pursue a compact host. The 2026-11-15 execution prototype measured 372,192 bytes on macOS arm64 and 487,320 bytes on Linux arm64, versus roughly 2.5 MB for the equivalent Go prototype. These are [prototype measurements](docs/research/showboat-like-tool/docs/07-RESULTS.md), not a comparison with Showboat or a size promise for the finished tool. |
| Standalone installation | Distribute a native executable that starts without Rust, Java, Node, Python or a separate Showboat installation. Target commands still need their own runtimes and tools; macOS uses system libraries, and static Linux release binaries are the packaging target. |
| Warm JVM, Node and Python execution | Reuse qualified runtime/application initialization across commands to reduce repeated startup cost, while preserving independent cold execution for parity and release checks. JVM and Node come first, then Python; [all three have feasibility prototypes](docs/research/showboat-like-tool/README.md). |
| One reusable execution engine | Move session supervision, forwarding, streaming and cleanup into mdrun so consumer repositories can retain thin build/policy/application adapters instead of their own warm-execution harnesses. |
| Familiar documents and commands | Preserve pinned Showboat behavior and existing recordings while giving the new implementation its own truthful identity. |

[Showboat also provides compiled native binaries](https://github.com/simonw/showboat/blob/d531261b8faf0c388b02c7891d50f1f47c3e2b52/README.md#installation). mdrun's motivation combines compact distribution with built-in cooperative warm execution. It is an independent implementation and is not affiliated with or endorsed by Showboat. “Standalone” describes runtime installation requirements; it does not rule out compiled Rust library dependencies where justified.

## Distribution

There is no product release yet. Release automation is prepared for macOS Apple Silicon and Linux x86_64/arm64, with native archives, static Linux binaries, `.deb`/`.rpm` packages and a Homebrew formula for the existing `mbackschat/tap` channel. See [distribution and release instructions](docs/distribution.md) for dry-run builds and publication steps. Installation commands will become user-facing once a qualified release and tap formula are published.

## Develop

Install Rust/Cargo with rustfmt and Clippy; prefer Homebrew on macOS. The bootstrap has no third-party crate dependencies and needs no target language runtime. Rust edition 2024 follows the supplied Rust prototype; a minimum supported Rust version has not been qualified.

```sh
cargo build --locked
cargo run --locked -- --help
cargo test --locked
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo build --release --locked
```

The executable is `target/release/mdrun`. Keep [Cargo.lock](Cargo.lock) in version control. Integration tests are modules of one binary at [tests/integration](tests/integration). The [CI workflow](.github/workflows/ci.yml) checks the bootstrap on macOS and Linux; configured jobs are not evidence of hosted qualification.

## Project map

| Start here | Purpose |
| --- | --- |
| [Contributor instructions](CLAUDE.md) | Coding-agent workflow; `AGENTS.md` is a relative symlink to this file |
| [Documentation](docs/README.md) | Current documents and their ownership |
| [Product contract](docs/PRODUCT.md) | Supported behavior and accepted product goals, clearly separated |
| [Implementation plan](docs/PLAN.md) | Ordered milestones and completion gates |
| [Compatibility qualification](docs/plans/compatibility.md) | Proposed upstream baseline and finite acceptance inventory |
| [Research provenance](docs/research/README.md) | Complete imported prototype, evidence, checksums and Git history |
| [Distribution](docs/distribution.md) | macOS/Linux CI, Homebrew and Linux release packages |
| [Deferred work](docs/DEFERRED.md) | Explicitly deferred scope and reopening conditions |

LOCmetry informed repository organization and, at the owner's request, distribution conventions. Dependencies, runtime design and product features are not inherited from it. CLI parsing support such as `clap` will be assessed against pinned Showboat behavior during compatibility qualification; no dependency is selected solely because another project uses it.

Licensed under the [MIT License](LICENSE), selected by the owner for mdrun. Cargo registry publication is disabled (`publish = false`); GitHub releases and Homebrew/Linux distribution are independent of that setting. Preserve upstream license notices if upstream code is incorporated later.
