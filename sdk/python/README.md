# Experimental Python worker SDK

This dependency-free package implements the proposed [session protocol](../../docs/plans/session-protocol.md) for one cooperative application in a dedicated Python process. The API is experimental. The native Rust host remains standalone; Python is required only for a separately launched Python worker. This foundation does not publish a package or establish arbitrary Python state/resource cleanup.

## Implement an adapter

Make `mdrun_sdk` from this directory available to the prepared worker's Python import path. The host appends one private configuration-file path to `worker_argv`. Use `argparse` for bootstrap arguments and pass the path to `run_worker`; keep its contents and credentials out of application output.

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

The empty reset is justified only by this stateless, synchronous example. Real adapters must stop and check application-owned work and resources, reread mutable inputs and qualify failure-then-success behavior against independently launched packaged execution. Classify handlers needing unqualified process-global streams, terminal behavior or native-extension state as cold before invocation.

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

The lifecycle controls require successful reset and reuse after joined work, and failure before completion for retained or expired work. Separate controls distinguish an application return status from `SystemExit(0)` and `os._exit(0)`: the former faults the callback; the latter loses the worker without a completion message. After either accepted invocation, the native host must fail the session without cold retry, even though the independently launched cold program exits successfully. One durable warm marker and no cold marker guard this distinction. All 20 local peer tests and seven native-host entries pass; hosted qualification of these additions remains pending.

Run the independent standard-library wire-peer suite with `PYTHONDONTWRITEBYTECODE=1 python3 tests/runtime/python/qualify.py`. It exercises lazy bounded input, binary streams, nonzero reuse, refusal, stable factory thread baselines, acceptance/release gates, malformed peers, callback/reset/close failure, late operations during and after serving, and control loss during blocked SDK I/O. Each peer fixture owns its worker `Popen` and a ten-second external watchdog. The separately enabled native-host controls in [session_python.rs](../../tests/integration/session_python.rs) compare seven independent cold/warm calls, literal binary/status expectations, changed files, refusal effects and require-warm, invalidation after success and unused open stdin. Select an absolute interpreter with `MDRUN_PYTHON` and run `cargo test --locked --test integration session::python:: -- --ignored`.

The shared [streaming qualifier](../../tests/runtime/session/streaming.py) accepts `--python /absolute/python3` alongside `--mdrun /absolute/mdrun`. It reuses a 64 KiB pattern chunk for 128 MiB, publishing atomic progress only after completed SDK writes. The unchanged oracle requires a paused-reader plateau, resumed production after a bounded drain, at most 32 MiB of accepted bytes beyond consumption and an exact incremental count/hash. An eager scratch-spool negative control must violate that oracle; a broken reader must fail with clean host custody. CI configures the same three cases against the release host. Results record artifact identities; RSS is supplementary, and these byte bounds establish neither performance nor release readiness. macOS/Linux CI (historical run) passes all three streaming cases at `051cc3d`; [execution qualification](../../docs/plans/execution.md) records the runtime versions and measured bounds. Application resource cleanup remains a separate gate.
