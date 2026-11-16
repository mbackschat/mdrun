# mdrun

[![CI](https://github.com/mbackschat/mdrun/actions/workflows/ci.yml/badge.svg)](https://github.com/mbackschat/mdrun/actions/workflows/ci.yml)
[![Release builds](https://github.com/mbackschat/mdrun/actions/workflows/release.yml/badge.svg)](https://github.com/mbackschat/mdrun/actions/workflows/release.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-2024-orange.svg)](Cargo.toml)
[![Status: release candidate](https://img.shields.io/badge/status-release%20candidate-yellow.svg)](docs/PLAN.md)

**Turn a command-line demonstration into a document you can check again.**

mdrun combines Markdown explanations, executable commands and captured output. Record a bug reproduction, demonstrate a feature, or let a coding agent leave behind evidence you can replay. When the output changes, `mdrun verify` shows the difference.

It is an independent Rust implementation inspired by **[Showboat](https://github.com/simonw/showboat), created by Simon Willison**. Credit for the original executable-demonstration idea and command model belongs to Showboat and its contributors.

## Record your first demonstration

With `mdrun` on your PATH, run these four commands in an empty working directory:

```sh
mdrun init demo.md 'A repeatable greeting'
mdrun note demo.md 'This command prints a greeting.'
mdrun exec demo.md sh 'printf "hello from mdrun\n"'
mdrun verify demo.md
```

`exec` prints `hello from mdrun` and records it. `verify` runs the command again and exits successfully without changing the document when the captured output matches. Below is the command/output portion of the [actual recording](docs/examples/greeting.md):

````markdown
```sh
printf "hello from mdrun\n"
```

```output
hello from mdrun
```
````

The complete file also has your title, timestamp and note. It is ordinary Markdown, readable in GitHub and your editor. `sh` is the executable interpreter; mdrun invokes it with `-c` and your code. Other interpreters must accept that convention.

## Install and get help

**0.1.0 release candidate:** there is no published product release yet, and publication is held at the owner's request. Document editing, replay, discovery and cooperative managed execution have macOS/Linux qualification. The public session and SDK version-1 contract is frozen; final artifact checks remain tracked in the [implementation plan](docs/PLAN.md). Application adapters still need their own parity and resource qualification.

To try the current source, install Rust/Cargo and build the pinned dependencies:

```sh
git clone https://github.com/mbackschat/mdrun.git
cd mdrun
cargo build --release --locked
./target/release/mdrun --help
```

Put `target/release/mdrun` in a directory on your PATH to use the examples above. The executable starts without Rust, Go, Java, Node, Python or an external Showboat installation. Commands you execute still need their own interpreters and application runtimes.

Release automation targets macOS Apple Silicon and Linux x86_64/arm64, with archives, static Linux executables, `.deb`/`.rpm` packages and Homebrew through `mbackschat/tap`. Published installation commands will be added after qualification; see [distribution](docs/distribution.md).

For a cooperative Python application, the release assets also include a standard Python 3.12+ SDK wheel. Install it with `uv pip install` into the application's virtual environment using an exact release URL and SHA-256 requirement; the same pinned file works locally and in CI. The [Python SDK installation guide](docs/sdk-distribution.md#install-the-python-sdk-with-uv) covers release URLs and local wheels. This library is separate from the standalone host, and PyPI publication remains deferred. The commands become usable when the selected release assets are published.

| Need | Command |
| --- | --- |
| Find a command quickly | `mdrun -h` |
| See a working starting point | `mdrun --help` |
| Get syntax and examples for one command | `mdrun verify --help` |
| Read an offline workflow guide | `mdrun guide documents` |
| Understand warm execution | `mdrun guide sessions` |
| Check exit statuses, paths and environment | `mdrun guide reference` |
| Read the installed manual | `man mdrun` or `man mdrun-verify` (packaged installs) |

Help and guides need no network or application worker. The [online guides](docs/guides/README.md) are the same maintained text embedded in the executable.

## Keep the evidence current

Run `mdrun verify demo.md` after changing the application. Matching output returns `0`; a changed capture produces a diff and returns `1`. Review updated output in a separate file before replacing your recording:

```sh
mdrun verify demo.md --output candidate.md
```

This runs the commands and writes updated captures to `candidate.md`, leaving `demo.md` unchanged. Selecting the input path explicitly with `--output demo.md` updates it in place. Verification executes real commands and does not undo their side effects. Prefer deterministic examples: timestamps, random identifiers and unstable ordering can make otherwise correct runs differ.

`exec` captures combined stdout/stderr even when the command fails, and returns its exit status. That lets you document expected errors. `verify` checks recorded output, not application exit status; a matching error demonstration can verify successfully. Output comparison uses Markdown's text representation, so it is not an arbitrary binary-output oracle. See [document behavior](docs/guides/documents.md) for line-ending and final-newline details.

For multiline source, omit the code argument and pipe it into `exec`:

```sh
cat <<'CODE' | mdrun exec demo.md sh
printf 'first\n'
printf 'second\n'
CODE
```

The pipe supplies command source. Executed blocks receive EOF on stdin unless their code supplies input, for example with a file redirection. `note` and `image` also accept their omitted argument from stdin.

## Find the command you need

```sh
mdrun list demo.md
mdrun list demo.md --filter hello --json
mdrun preview demo.md 2
```

The first two commands inventory recognized executable blocks without running them. The last prints the complete source of block `2` in the initial greeting document; use the block value reported by `list` for other documents. Block numbers are replay positions, not permanent identifiers. Filtering is a case-sensitive literal search across the filename, heading and command text.

`list --json` supports automation; `preview` preserves the selected fenced source. Neither executes code, expands shell expressions or starts workers. Names, tags, interactive pickers and selective block execution remain [deferred](docs/DEFERRED.md).

## Commands at a glance

| Task | Command | Effect |
| --- | --- | --- |
| Start a demonstration | `init FILE TITLE` | Create a document; refuse an existing file |
| Explain a step | `note FILE [TEXT]` | Append Markdown |
| Run and capture | `exec FILE LANG [CODE]` | Append code and combined output |
| Include an image | `image FILE [INPUT]` | Copy an image beside the document and append its reference |
| Undo the last entry | `pop FILE` | Remove the final note or code/output pair |
| Check the recording | `verify FILE` | Replay and compare recorded output |
| Recreate the authoring commands | `extract FILE` | Print a shell script; inspect before running |
| Discover commands | `list FILE...` | Show an inventory, optionally filtered or JSON |
| Inspect one command | `preview FILE BLOCK` | Print complete fenced source without execution |
| Reuse an application | `session --binding FILE -- COMMAND...` | Run an author script with a prepared cooperative binding |
| Verify independent documents | `verify-many --binding FILE FILE...` | Run bounded sessions for independent documents |
| Read offline | `guide [TOPIC]` | Show task guides and reference notes |

Global `--workdir DIR` changes where code executes; document, image and output paths still resolve from your calling directory. Use `--` before positional values starting with a hyphen. Clap supplies ordinary option parsing, help and usage errors.

## Reuse application startup

Repeated JVM startup or expensive Node/Python initialization can dominate a demonstration. mdrun's managed sessions let a **cooperative application adapter** reuse that initialization while keeping ordinary CLI calls in the author script:

```sh
mdrun session --binding binding.json --report session.json -- sh author.sh
```

The binding identifies a prepared worker and an independent cold launcher. An adapter classifies eligible calls, invokes the application with request-owned streams, and checks that its state and owned work permit reuse. mdrun owns supervision, forwarding, streaming, scheduling and cleanup. A consumer keeps its build preparation and application-specific adapter instead of another generic warm-execution harness.

This is opt-in application integration, not automatic acceleration of every CLI. Eligible calls can share one sequential worker per session; unsupported contexts select the cold launcher before acceptance. Accepted calls are never retried. `--require-warm` makes an unexpected cold route fail, and reports show route counts and reasons. See the [session guide](docs/guides/sessions.md), [JVM SDK](sdk/jvm/README.md), [Node SDK](sdk/node/README.md) and [Python SDK](sdk/python/README.md).

Managed scripts and applications must keep descendants in their assigned process group and stop/await background work for successful orderly completion. Shell monitor mode (`set -m`), independent process groups and daemonization are unsupported. The host cannot detect every departure; a descendant outside this boundary may survive a successful report and released exclusion. The [ownership contract](docs/plans/session-protocol.md#ownership-and-external-resources) defines that limit.

## Why another implementation?

| Goal | Why it matters |
| --- | --- |
| Compact native distribution | Pursue a small executable and straightforward installs; final sizes and comparisons remain a qualification task, not a claim that Rust alone guarantees a smaller binary |
| Standalone host | No language runtime or external Showboat installation is needed to start mdrun; compiled Rust libraries supply standard CLI parsing, HTTP/TLS and other focused functionality |
| Built-in cooperative warm execution | Amortize qualified application startup across JVM, Node and Python calls while retaining an independent cold path for parity checks |
| Minimal consumer harness | Put generic session supervision in one tool; leave build identity, eligibility and application reset with the consumer |
| Familiar documents | Preserve Showboat recordings and the seven familiar document commands, while retaining unmanaged spacing and line endings during edits |

[Showboat also ships native executables](https://github.com/simonw/showboat/blob/d531261b8faf0c388b02c7891d50f1f47c3e2b52/README.md#installation). mdrun's motivation combines compact distribution with cooperative warm sessions. It is not affiliated with or endorsed by Showboat. New documents identify mdrun; existing provenance is preserved. The compatible `SHOWBOAT_REMOTE_URL` receiver setting remains available, with nonfatal delivery warnings.

## Contribute and follow progress

```sh
cargo test --locked
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
```

The [macOS/Linux CI](.github/workflows/ci.yml) includes independent tests and a pinned Showboat differential oracle. Runtime and consumer qualification use explicit prepared fixtures. New dependencies follow a [14-day release quarantine](docs/security.md); verified security fixes have a documented exception process.

| Read | Purpose |
| --- | --- |
| [Contributor instructions](CLAUDE.md) | Coding-agent conventions; `AGENTS.md` is a relative symlink |
| [Documentation index](docs/README.md) | Guides, contracts, SDKs and release instructions |
| [Product contract](docs/PRODUCT.md) | Supported behavior and accepted goals |
| [Implementation plan](docs/PLAN.md) | Outstanding work and qualification evidence |
| [Compatibility](docs/plans/compatibility.md) | Pinned Showboat behavior and intentional differences |
| [Changelog](CHANGELOG.md) | User-visible changes |

LOCmetry informed repository organization, self-explaining help and distribution conventions. Its dependencies and application design are not inherited. mdrun is licensed under the [MIT License](LICENSE); distributions preserve [third-party crate notices](THIRD_PARTY_NOTICES.txt).
