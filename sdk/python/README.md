# Python worker SDK — version 1

Use this SDK to call a Python application repeatedly from executable Markdown while retaining its imports and reusable initialization in one worker. You write an adapter around callable application code; mdrun sends eligible commands to that adapter within a managed session.

When ordinary `mdrun verify` executes a Markdown block containing `python /absolute/path/to/cli.py hello`, that call starts a new interpreter, imports modules and initializes the application. Repeated calls repeat that setup. `cli.py` here means your existing program, not a supplied mdrun file. A warm worker starts the interpreter once and keeps the application instance returned by your factory. Every invocation still runs the command and reads current inputs; output is not cached. This requires an application that can safely finish one request and begin another.

You supply the normal CLI, the adapter and a binding JSON file connecting their launchers. The SDK supplies the worker transport, invocation streams and callback lifecycle. The standalone Rust host manages the session and process custody; Python is required only for the separately launched Python worker.

## Prepare the SDK

Use the [versioned SDK distribution guide](../../docs/sdk-distribution.md#install-the-python-sdk-with-uv) to install the standard `mdrun_sdk-VERSION-py3-none-any.whl` GitHub release asset into your application virtual environment with `uv pip install`. Pin its exact versioned URL and SHA-256 in the same requirements file locally and in CI; a verified local wheel works too. The distribution is named `mdrun-sdk` and provides `mdrun_sdk` for Python 3.12 or later, with no external dependencies or CLI entrypoint. Start the worker with that virtual environment's Python. Install the standalone Rust host separately. No product version has been published and publication is held; the distribution guide also covers verified local wheels. PyPI publication remains deferred.

The [archive preparation route](../../docs/sdk-distribution.md#pin-and-prepare-an-archive-download) remains available: add the prepared directory containing `mdrun_sdk/__init__.py` to the worker's import path. Both assets contain the maintained SDK and license, with no application adapter or tests. Keep the independent cold entrypoint SDK-free.

## Implement an adapter

Save the following as `hello_worker.py` in your application project. You create this file; its `Greeting` class contains the complete example command. A real adapter would call your existing application implementation and initialize reusable resources in its constructor or factory.

The worker's interpreter must be able to import `mdrun_sdk`, through your virtual environment or the prepared archive directory. The host appends one private configuration-file path to `worker_argv`. The bootstrap uses `argparse` to accept that path and passes it to `run_worker`; keep its contents and credentials out of application output. Do not launch this file as a normal command or create that private configuration yourself.

```python
import argparse
from pathlib import Path
from mdrun_sdk import Application, Eligible, Refused, Invocation, run_worker


class Greeting(Application):
    def classify(self, argv: tuple[str, ...]) -> Eligible | Refused:
        return Eligible() if argv == ("hello",) else Refused("unsupported_command")

    def invoke(self, request: Invocation) -> int:
        request.stdout.write(b"hello\n")
        return 0

    def reset(self, request: Invocation) -> None:
        pass


parser = argparse.ArgumentParser()
parser.add_argument("config", type=Path)
arguments = parser.parse_args()
try:
    run_worker(arguments.config, Greeting)
except BaseException:
    raise SystemExit(70)
```

For each accepted `hello` call, the example writes the bytes `hello\n` to request stdout and returns status `0`; subsequent calls run the handler again in the same worker. The empty reset is justified only by this stateless, synchronous example. Real adapters must stop and check application-owned work and resources, reread mutable inputs and qualify failure-then-success behavior against independently launched packaged execution. Classify handlers needing unqualified process-global streams, terminal behavior or native-extension state as cold before invocation.

## Connect it to a session

Create a [binding](../../docs/guides/sessions.md#what-you-prepare) whose `worker_argv` contains the absolute Python interpreter from your prepared environment followed by the absolute path to `hello_worker.py`. Set `cold_argv` to your independent normal CLI, with its usual interpreter and dependencies. Its `hello` command must produce the same bytes and status without importing the SDK. mdrun does not create either application launcher; it appends the private worker configuration path at startup.

Follow [managed-session verification](../../docs/guides/sessions.md#verify-and-inspect-the-route) to route repeated document commands through the binding. Eligible calls with a compatible context reuse the worker; a refusal can select the cold launcher before acceptance, while `--require-warm` fails a call that cannot go warm. Compare cold and warm bytes, statuses and relevant file effects, then check routing, worker starts and cleanup in the report. To try a complete supplied application first, use the [runnable Python walkthrough](../../docs/examples/warm-python.md).

## Callback contract

`run_worker(config_path, factory)` permits one worker lifetime per process, including failed startup. Its three persistent SDK threads have started before the factory runs. The factory, classification, invocation, reset and close callbacks all use the same persistent execution thread, allowing the factory to establish a stable resource baseline. Imports do not install process hooks.

| Callback | Contract |
| --- | --- |
| `classify(argv)` | Receive an immutable tuple of strings. Return `Eligible()` or `Refused(reason)` with a stable nonempty reason, without application effects, input reads or application output. |
| `invoke(request)` | Return an integer status from 0 through 255. A nonzero status is ordinary application failure and permits reuse after successful reset and release. Exceptions, Boolean statuses and invalid statuses fault the worker. |
| `reset(request)` | Mandatory application attestation that owned work has ended and state/resource checks permit reuse. It runs after the callback returns and accepted output drains, before `complete`. Failure poisons the worker. |
| `close()` | Release application resources after serving ends. The inherited implementation is empty. Orderly return waits for the actual implementation; failure faults the worker. |

## Borrowed binary streams

The SDK supplies a frozen `Invocation` with `request_id`, tuple `argv`, `stdin`, `stdout` and `stderr`. Application output must use these supplied streams. The SDK does not redirect `sys.stdin`, `sys.stdout`, `sys.stderr`, cached streams or native descriptors; process stdout/stderr remain private worker diagnostics.

`stdin.read(size)` requires an explicit nonnegative integer size. A positive read returns at most `min(size, 65536)` bytes, or `b""` at EOF; it does not read the entire input. `readinto(writable_buffer)` similarly fills at most one bounded chunk. Only an actual positive read issues input credit, at most one read may be outstanding, and EOF lasts only for that invocation. Ignoring stdin completes while the caller's pipe remains open and unread. Outstanding input when the handler returns invalidates completion.

`stdout.write(buffer)` and `stderr.write(buffer)` accept contiguous binary buffers and synchronously return the byte count. A large write is chunked into at most 65536-byte output frames, with one sequence shared across both streams. Writes apply socket backpressure, without an unbounded queue or whole-buffer conversion. Keep mutable buffers unchanged until their write returns. Concurrent writes to either output stream fault the worker; serialize them in the application.

Positive-length stream operations are admitted only during `invoke`. Once it returns, new operations are rejected, an already accepted output operation finishes, then reset and `complete` follow. The reservation stays held until host `release` and worker `released`; the SDK never retries or starts a cold fallback. Stream `close()`, output `flush()` and valid zero-length operations are inert, including after completion; closing input does not cancel an outstanding read.

A late positive-length operation or `request.invalidate()` irreversibly faults the worker even when application code catches its `OSError`. Invalidation uses the fixed reason `application_invalidated` and emits no application bytes. Retained requests can fault a later invocation or an idle worker. After `run_worker` has returned, these operations immediately terminate the dedicated process with [`os._exit(70)`](https://docs.python.org/3/library/os.html#os._exit), including from `atexit` callbacks; Python cleanup handlers are bypassed because raising a caught exception cannot revoke an earlier result. Native host custody and application cleanup obligations still apply.

## Cancellation and limits

`request.cancelled` is a read-only Boolean property; `request.wait_cancelled(timeout)` waits for terminal worker cancellation and returns a Boolean. Applications cannot clear cancellation through this API. Control loss, active shutdown, malformed protocol, callback failures and uncaught application-thread/unraisable exceptions stop admission and record sticky failure. The SDK breaks its socket and condition waits, wakes borrowed input and blocked output, and temporarily installs `threading.excepthook` and `sys.unraisablehook` while serving; it restores the previous hooks afterward.

Python's [threading API](https://docs.python.org/3/library/threading.html) provides no operation for forcibly interrupting arbitrary callback code. Callbacks and application work must cooperate with cancellation; worker return waits for the execution callback and close. The native host independently enforces its deadline. Do not catch bootstrap failure and continue serving, force successful process exit or start another worker lifetime.

Reset is an explicit application attestation. The SDK does not restore arbitrary modules, globals, context variables, threads, children, signal handlers, native extensions or external resources. Detached work that never calls the SDK can escape its stream guards. Applications must own and inspect that work; this foundation provides no automatic script reset or general Python cleanup guarantee.

The [owned-thread example](examples/owned_thread.py) shows this boundary for one known cooperative thread and stop event. `owned_joined` stops and joins its work before reset; `owned_live` deliberately leaves work running so reset rejects reuse. Expiry is also a failure, even after the thread exits. `close` stops and joins only the application's stored handle, without enumerating or joining unrelated threads. Reset requires that this handle has been cleared by explicit joining; a dead but unjoined handle cannot establish successful cleanup. Real adapters must choose cancellation and deadlines appropriate to their work.

## Qualification and evidence

The dependency-free package follows the frozen [version-1 session contract](../../docs/plans/session-protocol.md) for the 0.1.0 release candidate. The [execution checklist](../../docs/plans/execution.md) records runtime evidence and artifact checks. Registry publication remains deferred. Each application still requires its own parity and resource qualification; the SDK does not establish arbitrary Python state/resource cleanup.

The lifecycle controls require successful reset and reuse after joined work, and failure before completion for retained or expired work. Separate controls distinguish an application return status from `SystemExit(0)` and `os._exit(0)`: the former faults the callback; the latter loses the worker without a completion message. After either accepted invocation, the native host must fail the session without cold retry, even though the independently launched cold program exits successfully. One durable warm marker and no cold marker guard this distinction. All 20 local peer tests and seven native-host entries pass; macOS/Linux CI (historical run) also passes them at `6804d3d`.

Run the independent standard-library wire-peer suite with `PYTHONDONTWRITEBYTECODE=1 python3 tests/runtime/python/qualify.py`. It exercises lazy bounded input, binary streams, nonzero reuse, refusal with an independent no-effects marker, active input without credit, stable factory thread baselines, acceptance/release gates, malformed peers, callback/reset/close failure, late operations during and after serving, and control loss during blocked SDK I/O. Each peer fixture owns its worker `Popen` and a ten-second external watchdog. The separately enabled native-host controls in [session_python.rs](../../tests/integration/session_python.rs) compare seven independent cold/warm calls, literal binary/status expectations, changed files, refusal effects and require-warm, invalidation after success and unused open stdin. Select an absolute interpreter with `MDRUN_PYTHON` and run `cargo test --locked --test integration session::python:: -- --ignored`.

The shared [streaming qualifier](../../tests/runtime/session/streaming.py) accepts `--python /absolute/python3` alongside `--mdrun /absolute/mdrun`. It reuses a 64 KiB pattern chunk for 128 MiB, publishing atomic progress only after completed SDK writes. The unchanged oracle requires a paused-reader plateau, resumed production after a bounded drain, at most 32 MiB of accepted bytes beyond consumption and an exact incremental count/hash. An eager scratch-spool negative control must violate that oracle; a broken reader must fail with clean host custody. CI configures the same three cases against the release host. Results record artifact identities; RSS is supplementary, and these byte bounds establish neither performance nor release readiness. macOS/Linux CI (historical run) passes all three streaming cases at `051cc3d`; [execution qualification](../../docs/plans/execution.md) records the runtime versions and measured bounds. Application resource cleanup remains a separate gate.
