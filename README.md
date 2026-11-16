# mdrun

[![CI](https://github.com/mbackschat/mdrun/actions/workflows/ci.yml/badge.svg)](https://github.com/mbackschat/mdrun/actions/workflows/ci.yml)
[![Release builds](https://github.com/mbackschat/mdrun/actions/workflows/release.yml/badge.svg)](https://github.com/mbackschat/mdrun/actions/workflows/release.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-2024-orange.svg)](Cargo.toml)
[![GitHub release](https://img.shields.io/github/v/release/mbackschat/mdrun)](https://github.com/mbackschat/mdrun/releases)

**Executable Markdown with warm JVM, Node and Python sessions.**

Turn a CLI walkthrough, bug reproduction or tutorial into a document you can check again. Write commands and expected output in Markdown; mdrun replays them and shows what changed. **Write it by hand, or run a command and let mdrun capture the result.**

Keep a JVM, Node or Python worker running across calls, so your examples spend less time loading the same application. [Try the warm demo](#reuse-application-startup). mdrun itself is a standalone Rust executable; warm applications connect through an SDK adapter.

Inspired by **[Showboat](https://github.com/simonw/showboat), created by Simon Willison**. Credit for the original executable-demonstration idea and command model belongs to Showboat and its contributors.

## Start with a Markdown file

Save this as `demo.md` in your editor:

````markdown
```sh
printf "hello from mdrun\n"
```

```output
hello from mdrun
```
````

With [mdrun installed](#install-and-get-help), run:

```sh
mdrun verify demo.md
```

mdrun executes the `sh` block and compares its output with the adjacent `output` block. A match exits successfully and quietly. Change the command to print a different greeting while leaving the expected output unchanged: verification shows a diff and exits with status `1`.

That's a runnable document. Add prose to explain the steps, commit it alongside your code, and verify it when the application changes. Hand-written files need no generated metadata. See the [document guide](docs/guides/documents.md#write-the-markdown-yourself) for the format and comparison rules.

## Record your first demonstration

Prefer to capture the actual output automatically? In a fresh directory, these four commands create and check the same example:

```sh
mdrun init demo.md 'A repeatable greeting'
mdrun note demo.md 'This command prints a greeting.'
mdrun exec demo.md sh 'printf "hello from mdrun\n"'
mdrun verify demo.md
```

`init` creates the title, timestamp and document ID, `note` adds the explanatory paragraph, and `exec` records the command and its actual output. `verify` checks the result without changing the document. Here is the **complete Markdown file** from the [actual recording](docs/examples/greeting.md):

````markdown
# A repeatable greeting

*2026-11-16T09:16:24Z by mdrun 0.1.0*
<!-- mdrun-id: 959d798a-69fa-43d7-bbe6-de849a836799 -->

This command prints a greeting.

```sh
printf "hello from mdrun\n"
```

```output
hello from mdrun
```
````

Your timestamp and document ID will differ. Open the file on GitHub or in your editor: it reads like a walkthrough and doubles as a repeatable check.

## Reuse application startup

A walkthrough may call the same application dozens of times. With a warm session, those calls share an application process: load it once, then run each request with fresh arguments and input. Your Markdown still uses ordinary CLI commands.

| Runtime | Initialization you can reuse | Integration guide |
| --- | --- | --- |
| JVM | JVM startup, class loading and application initialization | [Java 21 SDK](sdk/jvm/README.md) |
| Node | Module loading and application initialization | [Node SDK](sdk/node/README.md) |
| Python | Imports and application initialization | [Python SDK](sdk/python/README.md) |

### Try it: a Python program that stays running

#### Without warm reuse: a fresh Python process per call

You can already record and verify Python commands with mdrun. Save the following as `ordinary.md` beside the demo's [application.py](examples/warm-python/application.py), which counts lines and words from stdin:

````markdown
```sh
echo "red green blue" | python3 application.py
```

```output
lines=1 words=3
```

```sh
echo "red green blue gold silver" | python3 application.py
```

```output
lines=1 words=5
```
````

Here `echo` supplies the text through a pipe; Python reads it from stdin. From that directory, run:

```sh
mdrun verify ordinary.md
```

mdrun executes both blocks and checks the recorded counts. **Each `python3 application.py` call starts a fresh Python process.** Verification works, but a larger application's imports, model loading or service initialization would be repeated at every call.

<details>
<summary>Record this file with mdrun instead of writing it by hand</summary>

To create a new `ordinary.md` and capture the output:

```sh
mdrun init ordinary.md 'Python without worker reuse'
mdrun exec ordinary.md sh 'echo "red green blue" | python3 application.py'
mdrun exec ordinary.md sh 'echo "red green blue gold silver" | python3 application.py'
mdrun verify ordinary.md
```

</details>

#### With warm reuse: one Python worker for repeated calls

Keep recording and verifying Markdown with mdrun, but route the application calls through a warm session. The supplied [worker.py](examples/warm-python/worker.py) adapter imports the same `count_stdin` function and serves repeated calls in one Python process.

Replace `python3 application.py` with `wordcount`, the command mdrun creates for this worker. Save the warm version as `first.md`:

````markdown
```sh
echo "red green blue" | wordcount
```

```output
lines=1 words=3
```

```sh
echo "red green blue gold silver" | wordcount
```

```output
lines=1 words=5
```
````

With the demo's prepared `binding.json` beside that document, verify it through the worker:

```sh
mdrun verify first.md --binding binding.json --require-warm
```

Both versions use mdrun to run Markdown and check the same results. The warm version keeps Python alive between the calls. `--require-warm` makes an unexpected fresh-process fallback fail. The one-command runner below prepares the binding, application and documents for you.

#### How does `wordcount` run Python without restarting it?

**The shell needs a way to send a request to that running worker. That's what `wordcount` does.** The name is chosen in the demo's configuration, `binding.json`, which tells mdrun:

| Setting | Meaning in this demo |
| --- | --- |
| `name: "wordcount"` | Make the application callable as `wordcount` inside the session. |
| `worker_argv` | Start Python with `worker.py` to serve repeated calls. |
| `cold_argv` | Start Python with `application.py` when a fresh process is wanted. |

When the session starts, mdrun creates a **symbolic link**—a filesystem shortcut—to its own executable. With illustrative paths, it looks like this:

```text
<session-directory>/bin/wordcount → /path/to/mdrun
```

There is no `wordcount` script to write or install. mdrun adds that temporary `bin` directory to the session's `PATH`, the list of directories the shell searches for commands. Your normal terminal's `PATH` stays unchanged.

In `echo "red green blue" | wordcount`, the shell pipes the text into a small mdrun client. The client connects that input to the session; the Python adapter passes the request's stdin to `count_stdin`, then sends the counts back to the shell. The client exits; **the Python worker stays alive for the next call**, with a fresh input stream.

The demo's [setup script](examples/warm-python/prepare.py) supplies the configuration automatically. For your own application, you provide the Python code, worker adapter and configuration; mdrun creates the command link and manages the processes.

**Run the complete demo with one command.** Return to the repository root, with Python 3.12+ and mdrun installed or [built locally](#install-and-get-help):

```sh
python3 examples/warm-python/run.py
```

The runner records this example plus a second document with one application call, then checks these results:

| Run | Application calls | Python startup |
| --- | --- | --- |
| Warm recording | 3 calls | One shared worker |
| Cold replay | Same 3 calls, same output | A fresh process per call |
| Parallel replay | 3 calls across 2 documents | One worker per document |

It prints the location of the Markdown, reports and logs. No Python packages or SDK downloads are needed; `--help` lists runner options.

#### Record the Markdown automatically

You can write the Markdown above by hand. To capture the actual results instead, the demo runner opens one managed session around [author.sh](examples/warm-python/author.sh). These are the script's first three authoring commands; `$mdrun` is the executable path supplied by the runner:

```sh
"$mdrun" init first.md 'Fresh inputs in one Python worker'
"$mdrun" exec first.md sh 'echo "red green blue" | wordcount'
"$mdrun" exec first.md sh 'echo "red green blue gold silver" | wordcount'
```

`init` creates the document with its title and metadata. Each `exec` then runs the pipeline and appends both its shell code and captured output—the Markdown shown above. The first records `words=3`; the second records `words=5`. Both use the worker owned by the surrounding session, so the script does not need to start Python or repeat `--binding` for each call.

The runner subsequently verifies those documents with fresh Python processes and with warm workers. This checks that reuse preserves the recorded results. The [complete demo walkthrough](docs/examples/warm-python.md) includes preparation, individual session commands, report interpretation and the full adapter.

#### Connect your own application

The Python program and the adapter have different jobs. [application.py](examples/warm-python/application.py) owns the word-counting logic and works as a normal CLI. [worker.py](examples/warm-python/worker.py) connects that same logic to mdrun. Its invocation method is:

```python
    def invoke(self, request: Invocation) -> int:
        return count_stdin(request.stdin, request.stdout, request.stderr)
```

Those streams belong to one call. They carry the text from its pipeline and return its result, even though the Python process continues running afterward. The ordinary CLI calls the same function with Python's standard streams. That shared function is what lets both execution modes do the same work.

To integrate your application:

1. **Expose the command logic as a callable function or object.** Initialize reusable resources once in the worker, then pass each call's arguments and streams to the application. Keep its normal CLI entry point for fresh-process execution.
2. **Write the adapter's three callbacks.** `classify` accepts only calls safe to reuse, before doing any application work. `invoke` runs the application through the supplied streams. `reset` finishes background work and clears request-specific state before the next call. This demo's reset is empty because its counting function finishes all I/O and retains no state or background work.
3. **Prepare the binding and check both modes.** Choose the command name and the worker/cold launch commands described above, and identify the prepared application/SDK build with `artifact_id`. Run your examples with `--require-warm` and with `--mode cold`; compare output, exit statuses and any file effects, and check the reports for worker reuse and clean cleanup.

The [Python SDK guide](sdk/python/README.md) supplies the complete adapter API and bootstrap. The [session guide](docs/guides/sessions.md) provides the full binding format, execution commands and ownership limits. mdrun supplies forwarding and process management; your adapter establishes which application state can safely survive between calls.

#### When does reuse help?

Reuse is most useful when a demonstration repeatedly pays for application startup: loading classes or modules, parsing schemas, or initializing reusable application objects. Work dominated by computation or I/O may benefit less. The tiny word counter demonstrates the mechanism; it is not a speed benchmark.

With the current optimized host, a twenty-call synthetic document ran **about five times faster for JVM and Node**. These are complete-session medians from five alternating cold/warm pairs on macOS arm64, measured on 2026-11-16 with Rust 1.98.1:

| Runtime | Fresh processes | One warm worker | Speedup |
| --- | ---: | ---: | ---: |
| JVM | 1,979 ms | 401 ms | 4.9× |
| Node | 1,852 ms | 358 ms | 5.2× |
| Python | 1,568 ms | 382 ms | 4.1× |

Every call read different input, and every sample passed output, route and cleanup checks. Times include worker startup, execution, shutdown and report publication; application/SDK preparation is separate. These results describe this workload, not a promised speedup for every application. The [performance report](docs/performance.md#current-host-comparison) gives all sample pairs, runtime versions, exact artifact identities and reproduction instructions; it also retains the older measurements that this table replaces.

### Use the same approach with Node or Java/JVM

The comparison is the same for Node and Java/JVM: ordinary `mdrun verify` starts the application for each call; verification with a prepared warm binding reuses its worker. The commands below illustrate stdin-reading applications **you provide**; the runnable word-count demo above is Python.

| Runtime | Command in ordinary mdrun Markdown | Warm mdrun session: what you implement |
| --- | --- | --- |
| Node | `echo "red green blue" \| node cli.mjs` | A `worker.mjs` that imports your application once and serves calls through the Node SDK's `runWorker`. Its `invoke(request)` callback runs your function with each request's arguments and streams. |
| Java/JVM | `echo "red green blue" \| java -jar app.jar` | A worker main class using the JVM SDK's `Worker.run`. It constructs your application once; its `invoke(invocation)` method handles successive calls in the same JVM. |

For Node, this reuses module loading and application initialization. For Java/JVM, it reuses JVM startup, class loading and initialized application objects. The adapter finishes each call's work and resets request-specific state before accepting the next one.

In either case, your binding names the CLI command and provides two launch commands: `worker_argv` for the reusable worker and `cold_argv` for the ordinary CLI. mdrun creates the same temporary command link shown above. Your Markdown calls that name; the runtime-specific adapter does the application work.

Start with the [Node adapter example](sdk/node/README.md) or the [Java 21 adapter example](sdk/jvm/README.md#implement-an-adapter). You supply the adapter and binding around your application; changing a launch command alone cannot make an arbitrary script or JAR safely reusable.

### Verify independent documents in parallel

Have a folder of demonstrations? `verify-many --jobs 2` checks two independent documents at a time, each with its own warm worker. The demo runner includes this step. See the [batch guide](docs/guides/sessions.md#independent-documents) for scheduling and reports.

For your own application, the SDK adapter defines which calls can reuse a worker and how to clean up between them. Follow the [integration and ownership contract](docs/guides/sessions.md#context-and-ownership-limits); reuse requires application cooperation.

## Keep the evidence current

Run `mdrun verify demo.md` after changing the application. Matching output returns `0`; a changed capture produces a diff and returns `1`. Review updated output in a separate file before replacing your recording:

```sh
mdrun verify demo.md --output candidate.md
```

This runs the commands and writes updated captures to `candidate.md`, leaving `demo.md` unchanged. Selecting the input path explicitly with `--output demo.md` updates it in place. Verification executes real commands and does not undo their side effects. Prefer deterministic examples: timestamps, random identifiers and unstable ordering can make otherwise correct runs differ.

`exec` captures combined stdout/stderr even when the command fails, and returns its exit status. That lets you document expected errors. `verify` checks recorded output, not application exit status; a matching error demonstration can verify successfully. Output comparison uses Markdown's text representation, so it is not an arbitrary binary-output oracle. See [document behavior](docs/guides/documents.md) for line-ending and final-newline details.

<details>
<summary>Record a multiline command from stdin</summary>

For multiline source, omit the code argument and pipe it into `exec`:

```sh
cat <<'CODE' | mdrun exec demo.md sh
printf 'first\n'
printf 'second\n'
CODE
```

This appends the following Markdown:

````markdown
```sh
printf 'first\n'
printf 'second\n'

```

```output
first
second
```
````

The pipe supplies command source. Executed blocks receive EOF on stdin unless their code supplies input, for example with a file redirection. `note` and `image` also accept their omitted argument from stdin.

</details>

## Find a command without running it

Search either version of `demo.md` without executing anything:

```sh
mdrun list demo.md --filter hello
mdrun list demo.md --filter hello --json
```

Use the block number reported by `list` with `preview`. For the hand-written file at the top of this page, it is `0`:

```sh
mdrun preview demo.md 0
```

This prints the original fenced source:

````markdown
```sh
printf "hello from mdrun\n"
```
````

Block numbers are positions in the current document, not permanent identifiers; the CLI-recorded version has additional metadata and a different number. JSON inventory supports scripts and coding agents. Neither command starts an application worker. See [discovery and extraction](docs/guides/documents.md#find-inspect-and-recreate).

## Install and get help

Choose an available version from [GitHub Releases](https://github.com/mbackschat/mdrun/releases). Prebuilt archives target macOS Apple Silicon and Linux x86_64/arm64; Linux also has `.deb` and `.rpm` packages. Check the release's `SHA256SUMS`, extract the matching archive and put its `mdrun` executable in a directory on your PATH, or install the matching Linux package. See [distribution](docs/distribution.md) for the supported platform matrix and Homebrew publication workflow.

To build from source, use Rust 1.98.1:

```sh
git clone https://github.com/mbackschat/mdrun.git
cd mdrun
cargo build --release --locked
```

Put `target/release/mdrun` in a directory on your PATH, and run `mdrun --help` to explore the CLI. The executable starts without Rust, Go, Java, Node, Python or an external Showboat installation. Commands you execute still need their own interpreters and application runtimes.

Document editing, replay, discovery, cooperative managed execution, native packages and SDK assets have passed macOS/Linux qualification at identified candidate revisions. The public session and SDK version-1 contract is frozen for 0.1.0; the [implementation plan](docs/PLAN.md) records exact-revision evidence and outstanding release gates. Application adapters still need their own parity and resource qualification.

For a cooperative Python application, choose the standard Python 3.12+ SDK wheel from the same release. Install it with `uv pip install` into the application's virtual environment using an exact release URL and SHA-256 requirement; the same pinned file works locally and in CI. The [Python SDK installation guide](docs/sdk-distribution.md#install-the-python-sdk-with-uv) covers release URLs and local wheels. This library is separate from the standalone host, and PyPI publication remains deferred.

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
