# A runnable Python warm session

The [README](../../README.md) runs this source-checkout example with Python 3.12 or later and an already built native host. It requires no downloads or third-party Python dependencies. The application counts words and lines in a UTF-8 file; repeated calls reuse the Python interpreter and imported adapter while rereading the file on every invocation. This deliberately small example demonstrates fresh results and worker reuse, without a speedup claim.

| File | Responsibility |
| --- | --- |
| [application.py](../../examples/warm-python/application.py) | Ordinary argparse CLI and synchronous file-counting function; imports no SDK. |
| [worker.py](../../examples/warm-python/worker.py) | Explicit SDK adapter: classify a single filename, invoke the shared function with borrowed streams, and attest reset. |
| [prepare.py](../../examples/warm-python/prepare.py) | Copy the application, adapter, author script and local SDK into an explicitly named new directory outside the checkout; write an absolute binding and content identity. |
| [author.sh](../../examples/warm-python/author.sh) | Use ordinary `init` and `exec` commands to record three calls across two documents. |
| [check.py](../../examples/warm-python/check.py) | Check literal recorded output, final inputs, application statuses, actual route counts, worker counts and clean cleanup; also require two slots for the batch example. |

The author records `first.md` with three words, then overwrites `first.txt` with five words and records another call. It records `second.md` using its own two-word `second.txt`. Replaying both documents with two batch slots therefore needs no shared mutable input. One author session serves all three calls with one worker; batch replay gives each document its own session and worker. The checker rejects a successful cold run when asked to prove warm execution.

## Compare with fresh processes

After the README demo, keep its `MDRUN_DEMO_BIN` and `MDRUN_DEMO_DIR` variables and run these commands from the source checkout. They replay both documents with the independent cold launcher, which starts a new Python process for every application call:

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

## How the adapter and binding work

Use the prepared directory as the global `--workdir` shown in the README. The binding sets `PWD` to that same directory, because changing the physical working directory does not itself update the inherited `PWD` value. It removes `OLDPWD` once from both execution contexts because a shell can discard that inherited value during startup, and fixes `LANG` and `LC_ALL` to `C`. The worker and ordinary launcher use Python's isolated mode with site initialization and bytecode writes disabled. The binding ignores only `SHLVL` and `_` during environment comparison: this application's eligible function reads its filename and file contents and does not observe either variable. Other context checks still apply; these are example-specific declarations, not defaults to copy into arbitrary applications.

The adapter accepts one filename without a leading dash. Other argument shapes, including help, are refused before invocation and can use the ordinary cold launcher unless `--require-warm` rejects that route. File and decoding errors return an ordinary failure status. The counting function closes its file and retains no request, mutable application state, threads or children, which justifies its empty reset callback. Applications with owned work need their own cleanup and parity qualification; the SDK does not reset arbitrary Python process state.

Preparation hashes the copied sources, SDK, selected interpreter file and launch configuration into the binding identity. Keep those prepared artifacts unchanged during execution. The host checks the supplied identity rather than rehashing files, and the example does not copy or pin the interpreter's standard library. This is a checkout demo, not an installer or a general application qualification harness. Use the [SDK distribution guide](../sdk-distribution.md) for a versioned application dependency and the [session guide](../guides/sessions.md) for the full binding and report contract.
