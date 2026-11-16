# A runnable Python warm session

Ordinary `mdrun verify` can run and check a Python application at every step of a Markdown demonstration. Each direct Python invocation starts a new interpreter and imports the application again. A warm mdrun session keeps the application alive across those calls while still checking fresh results.

This walkthrough compares **mdrun without worker reuse** with **mdrun using a warm worker**. The included word counter is deliberately small: it demonstrates execution and reuse; [performance measurements](../performance.md) cover timing separately.

## Start with mdrun without worker reuse

Save this as `ordinary.md` beside the supplied [application.py](../../examples/warm-python/application.py), which counts UTF-8 text from stdin. The pipe sends each `echo` result directly to Python. These commands and captures were recorded with mdrun:

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

From that directory, replay the document:

```sh
mdrun verify ordinary.md
```

mdrun executes both shell blocks and checks their captured output. The first Python process reads three words and exits; a new Python process reads five words on the next call. For an application with expensive initialization, every call repeats that setup.

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

## Keep Python alive between calls

The included [worker.py](../../examples/warm-python/worker.py) imports the application's `count_stdin` function and serves repeated requests through the Python SDK. This is the application adapter: it connects existing counting logic to mdrun and checks that each call has finished before reuse.

Both the application and adapter are supplied in this demo. In your Markdown, replace `python3 application.py` with `wordcount`, the command mdrun creates to send requests to the worker. The shell's `echo` sends the text through a pipe; the worker reads it from that request's stdin. Verification still runs through mdrun; adding `--binding binding.json --require-warm` selects the prepared worker instead of starting Python for each application call.

## Authoring calls and the resulting Markdown

This is the command/output portion of `first.md` (the generated title and provenance are omitted). The input changes between calls, so the result must change even though the worker stays alive:

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

Save this warm document as `first.md`. With the prepared application and `binding.json` in the same directory, run:

```sh
mdrun verify first.md --binding binding.json --require-warm
```

The [demo runner below](#run-the-demo) prepares those files automatically. Both checks use mdrun and compare the same literal counts; the warm check additionally requires worker reuse.

Writing this Markdown by hand is fully supported. The optional author script captures actual output for you:

Inside `author.sh`, `$mdrun` is the executable passed to the script. These two calls run inside the managed session:

```sh
"$mdrun" exec first.md sh 'echo "red green blue" | wordcount'
"$mdrun" exec first.md sh 'echo "red green blue gold silver" | wordcount'
```

## How the shell command reaches Python

| Piece | Supplied by | Role |
| --- | --- | --- |
| [application.py](../../examples/warm-python/application.py) | This demo | Contains the `count_stdin` function and a normal Python CLI entry point. |
| [worker.py](../../examples/warm-python/worker.py) | This demo | Uses the Python SDK to receive requests and call `count_stdin` without restarting Python. |
| `binding.json` | The demo's [prepare.py](../../examples/warm-python/prepare.py) | Sets `name` to `wordcount`, `worker_argv` to the Python worker launch command and `cold_argv` to the ordinary Python CLI launch command. |
| Temporary `wordcount` command | mdrun | Receives shell arguments and forwards them to the session. |

In a pipeline such as `echo "red green blue" | python3 application.py`, Python starts anew and reads the piped text. Warm execution keeps `worker.py` running and supplies a separate input stream for each call. The binding chooses the name `wordcount` for the command that connects the pipeline to that worker.

At session startup, mdrun uses the operating system's symbolic-link operation to create a filesystem shortcut to its own executable. The layout is illustrated here; actual session and executable paths vary:

```text
<session-directory>/bin/wordcount → /path/to/mdrun
```

This link has no script body. mdrun puts its directory first on the session's `PATH`, so the shell finds it when a Markdown block calls `wordcount`. The parent terminal's environment is unchanged, and you do not install a global `wordcount` executable.

Calling the link starts mdrun under the name `wordcount`. mdrun recognizes that name and connects the shell command's streams to the session. In warm mode, the Python adapter calls `count_stdin(request.stdin, request.stdout, request.stderr)`. The function reads that request's input to EOF and writes the counts. The client returns the output and status to the shell and exits; the Python worker remains available for the next request. In cold mode, mdrun starts `application.py` in a fresh Python process with the pipeline connected to its stdin.

For your own application, you write the adapter and binding around your application code. mdrun creates the temporary command and manages the worker; it does not generate application logic or make an arbitrary Python script reusable automatically.

## Run the demo

Prerequisites: macOS or Linux, Python 3.12+ available as `python3`, and a source checkout with an installed or already built native host. See [build instructions](../../README.md#install-and-get-help). No third-party Python packages or SDK downloads are required.

Return to the repository root, then run:

```sh
python3 examples/warm-python/run.py
```

The runner finds `mdrun` on PATH, then falls back to `target/release/mdrun`. Use `--mdrun /path/to/mdrun` to select an executable, or `--output /tmp/new-demo-directory` to choose a new directory outside the checkout. It refuses existing output directories. By default it creates a temporary directory and leaves it available for inspection; the printed path contains the documents, inputs, reports and per-step logs. Delete it when you no longer need the results. The runner does not build or install software.

The runner records three calls in one warm session, checks the results, replays them cold, then replays the two documents in parallel. Each step has a 60-second timeout and a nonzero unexpected status stops the demo. CI uses the same runner.

## Compare with fresh processes

The checker prints these lines during the warm, cold and batch checks respectively:

```text
Verified literal output: warm=3 cold=0 workers=1 cleanup=clean
Verified literal output: warm=0 cold=3 workers=0 cleanup=clean
Verified literal output: warm=3 cold=0 workers=2 cleanup=clean
```

The cold replay starts a fresh Python process for each call and must produce the same recorded results. The runner then deliberately submits that cold report to the warm checker: the labelled expected failure proves that correct output alone cannot pass as worker reuse. The final demo status must still be successful.

In the parallel replay, each document gets its own session and worker. The first document's two calls share a worker; the second document has its own piped input and worker. Open `warm.json`, `cold.json` and `batch.json` to inspect the evidence.

## Explore the implementation

| File | Responsibility |
| --- | --- |
| [run.py](../../examples/warm-python/run.py) | Run preparation, warm recording, cold replay, parallel replay and independent checks; retain results and logs. |
| [application.py](../../examples/warm-python/application.py) | Ordinary argparse CLI and synchronous stdin-counting function; imports no SDK. |
| [worker.py](../../examples/warm-python/worker.py) | Explicit SDK adapter: accept calls without arguments, invoke the shared function with borrowed input/output streams, and attest reset. |
| [prepare.py](../../examples/warm-python/prepare.py) | Copy the application, adapter, author script and local SDK into an explicitly named new directory outside the checkout; write an absolute binding and content identity. |
| [author.sh](../../examples/warm-python/author.sh) | Use ordinary `init` and `exec` commands to record three calls across two documents. |
| [check.py](../../examples/warm-python/check.py) | Check literal recorded output, application statuses, actual route counts, worker counts and clean cleanup; also require two slots for the batch example. |

The author records two pipelines in `first.md`: one sends three words, the next sends five. It records a two-word pipeline in `second.md`. Each call has its own input stream, so the two documents can replay independently in two batch slots. One author session serves all three calls with one worker; batch replay gives each document its own session and worker. The checker rejects a successful cold run when asked to prove warm execution.

## Run individual steps or hand-written Markdown

The supplied runner is enough to try the feature. Expand the following recipe only to experiment with individual session commands or a hand-written document.

<details>
<summary>Prepare a binding, record, and verify manually</summary>

From the checkout, with the release executable already built, choose a new directory:

```sh
MDRUN_DEMO_BIN="$PWD/target/release/mdrun"
MDRUN_DEMO_DIR=/tmp/mdrun-warm-manual
python3 examples/warm-python/prepare.py --output "$MDRUN_DEMO_DIR"

"$MDRUN_DEMO_BIN" --workdir "$MDRUN_DEMO_DIR" session \
  --binding "$MDRUN_DEMO_DIR/binding.json" --require-warm \
  --report "$MDRUN_DEMO_DIR/warm.json" \
  -- sh "$MDRUN_DEMO_DIR/author.sh" "$MDRUN_DEMO_BIN"

python3 examples/warm-python/check.py --directory "$MDRUN_DEMO_DIR" \
  --report "$MDRUN_DEMO_DIR/warm.json" --mode fast
```

This records the Markdown shown above and a second document with a two-word input. The [author script](../../examples/warm-python/author.sh) contains all three recording calls.

You can write this Markdown directly in your editor. `sh` fences contain ordinary shell commands; `output` fences contain the expected results. The timestamp and document ID are optional for verification. The authoring commands run the code and fill in its actual output, which is convenient when recording longer demonstrations.

Save the fragment as `manual.md` in the prepared demo directory, then verify it with the same warm binding:

```sh
"$MDRUN_DEMO_BIN" --workdir "$MDRUN_DEMO_DIR" verify "$MDRUN_DEMO_DIR/manual.md" \
  --binding "$MDRUN_DEMO_DIR/binding.json" --require-warm \
  --report "$MDRUN_DEMO_DIR/manual.json"
```

Both shell blocks run, their `wordcount` calls share one worker, and each captured result must match its adjacent `output` block. Without an `output` block, verification still executes the command but has no recorded output to compare. See [document syntax and verification](../guides/documents.md#write-the-markdown-yourself).

Replay the recorded documents using fresh processes:

```sh
"$MDRUN_DEMO_BIN" --workdir "$MDRUN_DEMO_DIR" session \
  --binding "$MDRUN_DEMO_DIR/binding.json" --mode cold \
  --report "$MDRUN_DEMO_DIR/cold.json" \
  -- sh -c '"$1" verify first.md && "$1" verify second.md' sh "$MDRUN_DEMO_BIN"

python3 examples/warm-python/check.py --directory "$MDRUN_DEMO_DIR" \
  --report "$MDRUN_DEMO_DIR/cold.json" --mode cold
```

Successful verification is quiet. The actual checker output is:

```text
Verified literal output: warm=0 cold=3 workers=0 cleanup=clean
```

The recorded output still matches, but the report now proves three cold calls and no SDK worker. Asking the checker to validate this same report with `--mode fast` fails. CI runs this negative control as well as the warm, cold and batch demonstrations.

For the parallel invocation and its checks, see [run.py](../../examples/warm-python/run.py).

</details>

## How the adapter and binding work

The runner uses the prepared directory as the global `--workdir`. The binding sets `PWD` to that same directory, because changing the physical working directory does not itself update the inherited `PWD` value. It removes `OLDPWD` once from both execution contexts because a shell can discard that inherited value during startup, and fixes `LANG` and `LC_ALL` to `C`. The worker and ordinary launcher use Python's isolated mode with site initialization and bytecode writes disabled. The binding ignores only `SHLVL` and `_` during environment comparison: this application's eligible function reads only request stdin and does not observe either variable. Other context checks still apply; these are example-specific declarations, not defaults to copy into arbitrary applications.

The adapter accepts calls without arguments. Other argument shapes, including help, are refused before invocation and can use the ordinary cold launcher unless `--require-warm` rejects that route. Input-read and UTF-8 decoding errors return an ordinary failure status. The counting function reads stdin in chunks until EOF, decodes the collected bytes and writes the result through the supplied output stream. It finishes its I/O and retains no request, mutable application state, threads or children, which justifies its empty reset callback. Each subsequent call receives a fresh input stream. Applications with owned work need their own cleanup and parity qualification; the SDK does not reset arbitrary Python process state.

Preparation hashes the copied sources, SDK, selected interpreter file and launch configuration into the binding identity. Keep those prepared artifacts unchanged during execution. The host checks the supplied identity rather than rehashing files, and the example does not copy or pin the interpreter's standard library. This is a checkout demo, not an installer or a general application qualification harness. Use the [SDK distribution guide](../sdk-distribution.md) for a versioned application dependency and the [session guide](../guides/sessions.md) for the full binding and report contract.
