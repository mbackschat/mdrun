# mdrun

[![CI](https://github.com/mbackschat/mdrun/actions/workflows/ci.yml/badge.svg)](https://github.com/mbackschat/mdrun/actions/workflows/ci.yml)
[![Release builds](https://github.com/mbackschat/mdrun/actions/workflows/release.yml/badge.svg)](https://github.com/mbackschat/mdrun/actions/workflows/release.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-2024-orange.svg)](Cargo.toml)
[![Status: release candidate](https://img.shields.io/badge/status-release%20candidate-yellow.svg)](docs/PLAN.md)

**Executable Markdown with warm JVM, Node and Python sessions.**

Record command-line demonstrations as Markdown, then replay them to check that the output still matches. For JVM, Node and Python applications, mdrun can keep one application worker alive across repeated calls—reusing startup and initialization while each command runs with fresh arguments and input.

The host is a standalone Rust executable. Warm execution is opt-in: your application supplies a small SDK adapter, and mdrun handles forwarding, streams, supervision and cleanup. This is especially useful when a demonstration or regression suite spends more time starting the application than doing its work.

It is an independent Rust implementation inspired by **[Showboat](https://github.com/simonw/showboat), created by Simon Willison**. Credit for the original executable-demonstration idea and command model belongs to Showboat and its contributors.

## Reuse application startup

A managed session keeps one sequential worker available for eligible calls. Initialization happens once per worker; command results are computed anew. Your scripts keep calling the application by its CLI name.

| Runtime | Initialization you can reuse | Integration guide |
| --- | --- | --- |
| JVM | JVM startup, class loading and application initialization | [Java 21 SDK](sdk/jvm/README.md) |
| Node | Module loading and application initialization | [Node SDK](sdk/node/README.md) |
| Python | Imports and application initialization | [Python SDK](sdk/python/README.md) |

Your adapter classifies supported calls, invokes the application through request-owned streams, and checks that state and owned work permit reuse. mdrun supplies the generic execution harness. A binding connects that worker to its ordinary cold CLI, which starts a fresh process per call. See the [session guide](docs/guides/sessions.md) for the contract and [measured workloads](docs/performance.md) for performance evidence.

### Try it: changing input, one Python worker

This runnable example records three calls to a small word-count CLI. Between the first two calls it changes the input file. The counts must change, while the worker is reused. Python is the shortest setup; the JVM and Node SDKs use the same session model.

**Prerequisites:** macOS or Linux, Rust 1.98.1 and Python 3.12+ available as `python3`. No published release, Python package installation or external SDK download is needed for this source-checkout demo.

```sh
git clone https://github.com/mbackschat/mdrun.git
cd mdrun
cargo build --release --locked

MDRUN_DEMO_BIN="$PWD/target/release/mdrun"
MDRUN_DEMO_DIR=/tmp/mdrun-warm-demo
python3 examples/warm-python/prepare.py --output "$MDRUN_DEMO_DIR"

"$MDRUN_DEMO_BIN" --workdir "$MDRUN_DEMO_DIR" session \
  --binding "$MDRUN_DEMO_DIR/binding.json" --require-warm \
  --report "$MDRUN_DEMO_DIR/warm.json" \
  -- sh "$MDRUN_DEMO_DIR/author.sh" "$MDRUN_DEMO_BIN"

python3 examples/warm-python/check.py --directory "$MDRUN_DEMO_DIR" \
  --report "$MDRUN_DEMO_DIR/warm.json" --mode fast
```

Choose a new directory for each run; preparation refuses to overwrite an existing one. The author script creates two Markdown documents in that directory. Actual output from the session and checker:

```text
lines=1 words=3
lines=1 words=5
lines=1 words=2
Verified literal output: warm=3 cold=0 workers=1 cleanup=clean
```

The [author script](examples/warm-python/author.sh) uses ordinary `mdrun exec` calls containing `wordcount first.txt` and `wordcount second.txt`. The session-local binding forwards those calls to one Python worker. `--require-warm` prevents an unexpected cold fallback from passing unnoticed. The checker verifies literal document output, application statuses, route counts and clean cleanup.

Open `first.md` to see the commands and changed captures; open `warm.json` to inspect the routes. The [walkthrough](docs/examples/warm-python.md) explains the [ordinary CLI](examples/warm-python/application.py), [SDK adapter](examples/warm-python/worker.py), generated binding and setup helper. Follow its [cold comparison](docs/examples/warm-python.md#compare-with-fresh-processes) to verify the same output with three fresh processes and no reused worker. This example qualifies reuse and fresh input; the separate [performance measurements](docs/performance.md) cover execution time.

### Replay both documents in parallel

Keep the variables from the preceding demo and run:

```sh
"$MDRUN_DEMO_BIN" --workdir "$MDRUN_DEMO_DIR" verify-many \
  --binding "$MDRUN_DEMO_DIR/binding.json" --require-warm --jobs 2 \
  --report "$MDRUN_DEMO_DIR/batch.json" \
  "$MDRUN_DEMO_DIR/first.md" "$MDRUN_DEMO_DIR/second.md"

python3 examples/warm-python/check.py --directory "$MDRUN_DEMO_DIR" \
  --report "$MDRUN_DEMO_DIR/batch.json" --mode batch
```

Successful replay is quiet; the checker prints:

```text
Verified literal output: warm=3 cold=0 workers=2 cleanup=clean
```

Each document gets its own session and worker; the first document's two calls reuse one worker. These documents use separate input files, so they can run independently. `--jobs 2` bounds concurrent document sessions. An observed failure stops new admission while healthy admitted documents drain; the [batch guide](docs/guides/sessions.md#independent-documents) explains resource sharing and aggregate reports.

Warm integration requires explicit application cleanup. Managed descendants must remain in their assigned process groups; shell job control and daemonization are outside the supported boundary. Unsupported calls can use the cold launcher before acceptance, unless `--require-warm` forbids it. Accepted calls are never retried. See [context and ownership limits](docs/guides/sessions.md#context-and-ownership-limits).

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

**0.1.0 release candidate:** there is no published product release yet, and publication is held at the owner's request. Document editing, replay, discovery, cooperative managed execution, native packages and SDK assets have passed macOS/Linux pre-release qualification. The public session and SDK version-1 contract is frozen; the [implementation plan](docs/PLAN.md) records the qualified revision and remaining publication gates. Application adapters still need their own parity and resource qualification.

The warm demo above includes the source build. Put `target/release/mdrun` in a directory on your PATH to use the shorter commands below, and run `mdrun --help` to explore the CLI. The executable starts without Rust, Go, Java, Node, Python or an external Showboat installation. Commands you execute still need their own interpreters and application runtimes.

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

## Find a command without running it

Using the warm demo's variables, search its first document and inspect a matching command:

```sh
"$MDRUN_DEMO_BIN" list "$MDRUN_DEMO_DIR/first.md" --filter wordcount
"$MDRUN_DEMO_BIN" list "$MDRUN_DEMO_DIR/first.md" --filter wordcount --json
"$MDRUN_DEMO_BIN" preview "$MDRUN_DEMO_DIR/first.md" 3
```

The inventory reports two executable blocks, numbered `1` and `3`. Previewing `3` prints this original source without starting the application:

````markdown
```sh
printf "red green blue gold silver\n" > first.txt && wordcount first.txt
```
````

Use the block value reported by `list`; these are positions in the current document, not permanent identifiers. Filtering searches the filename, heading and full command text with a case-sensitive literal match. JSON output supports coding agents and scripts, while preview lets a person inspect the command before executing it. Neither starts a worker or expands shell expressions. See [discovery and extraction](docs/guides/documents.md#find-inspect-and-recreate). Names, tags, interactive pickers and selective execution remain [deferred](docs/DEFERRED.md).

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

## Why another implementation?

| Goal | Why it matters |
| --- | --- |
| Compact native distribution | A single native executable with [measured target sizes](docs/distribution.md#candidate-artifact-sizes); standard package formats keep installation straightforward |
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
