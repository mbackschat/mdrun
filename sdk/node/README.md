# Experimental Node worker SDK

This dependency-free ESM peer implements the proposed [session protocol](../../docs/plans/session-protocol.md). It is an experimental vertical slice, not a qualified public API or an automatic Node state reset facility. Fake-host and Rust-host interoperability tests pass with Node v26.10.0 on macOS and Linux; [execution qualification](../../docs/plans/execution.md) records the revisions and remaining application/lifecycle gates.

The application bootstrap loads its handler and establishes its baseline resources before calling `runWorker`. The host appends one private startup configuration path to `worker_argv`; the bootstrap passes that path directly and does not need another command-line parser. For an ESM entry point whose only argument is that path:

```js
import { runWorker } from './sdk/node/index.mjs';
import { classify, invoke, reset } from './application-adapter.mjs';

try {
    await runWorker({ configPath: process.argv[2], classify, invoke, reset });
} catch {
    process.exitCode = 70;
}
```

| Callback | Contract |
| --- | --- |
| `classify(argv)` | Receive a frozen array of strings; return `true` or a stable nonempty refusal reason. It must perform no application effects, output or input reads. Refusal permits the host to choose cold execution. |
| `invoke(request)` | Return an integer exit status from 0 through 255, synchronously or as a promise. Nonzero status is ordinary application failure and permits subsequent reuse after successful reset and release. Exceptions poison the worker. |
| `reset(request)` | Mandatory; resolve only after application-owned work has ended and state/resource checks pass. Reject when safe reuse cannot be established. It runs after the handler returns and accepted output has flushed, before `complete`. |

Each invocation receives a fresh frozen `request` containing `requestId`, frozen `argv`, `signal`, `stdin`, `stdout` and `stderr`. `stdin.read(maxBytes)` returns a promise for a nonempty `Buffer` or `null` at EOF; `maxBytes` must be an integer from 1 through 65536. Only an actual read emits credit. EOF is retained only for that invocation, one read may be pending, and the handler must await or otherwise finish it before returning. A handler that never reads creates no input credit.

`stdout.write(bytes)` and `stderr.write(bytes)` accept `Uint8Array`, including `Buffer`, and return promises. Await each write before making another write to either stream; overlapping writes poison the worker instead of creating an unbounded queue. A single write can exceed a frame because the SDK emits chunks of at most 65536 bytes with one shared sequence across both streams. Keep the supplied bytes unchanged until the promise settles. The SDK flushes its one accepted operation before reset even if the handler omitted `await`; this does not authorize detached application work. Request streams close to new operations when the handler returns. Writes through an old request poison reuse even when the application catches the rejected promise.

Application output must use request streams. `console`, process descriptors, imported stream singletons and native writes are not redirected; worker process stdout/stderr are private startup/diagnostic channels. The adapter must qualify the real command path and reject commands that depend on process-global descriptors, terminal behavior or unsupported native encoding.

The SDK retains its reservation through `complete` until `release`, then sends `released`. No retry or cold fallback happens inside this peer. Authentication and readiness bind both connections to the private configuration identity. Strict frame schemas, UTF-8, frame lengths, canonical padded Base64, credit and transition checks reject malformed peers. Tokens and arbitrary callback error messages are excluded from SDK fault diagnostics.

Control closure or active shutdown aborts the request signal, rejects pending SDK input and breaks blocked socket output. `runWorker` rejects on poisoning and resolves on orderly idle shutdown. A retained request stream used after that resolution still records worker process failure with exit status 70, preserving an existing nonzero status, even when the application catches the rejected operation. A resolved promise cannot report this later violation, so the host must observe the worker's actual exit status. Adapters must let the process finish cooperatively and must not reset its failure status or force successful exit. The SDK installs process exception/rejection listeners only while running, restores them afterward, and rejects simultaneous workers in one process. Importing the module changes no process hooks. Bootstrap failures and faults must cause worker failure; do not catch them and continue serving work. The host independently owns process custody and termination deadlines, including handlers that block the event loop or ignore cancellation.

Reset is an explicit application attestation. The SDK does not enumerate or restore timers, event listeners, children, native handles, module caches, global variables or unresolved promises, and successful reset does not establish their absence. The adapter must stop and check its own work, listeners, timers and children; await application promises; reread mutable inputs; and qualify failure-then-success behavior against independent packaged execution. Uncaught exceptions and unhandled rejections poison the peer while it remains active; operations through retained request streams also record process failure after orderly shutdown. Detached work that never touches SDK streams can escape these guards, so it must be covered by application checks and host custody rather than claimed as generically reset.

Run the independent fake-host suite with `node --test --test-timeout=10000 tests/runtime/node/worker.test.mjs`. It exercises lazy credit, 2.5 MB binary streaming with fresh EOF, stdout/stderr ordering, ordinary nonzero reuse, refusal, output flushing, reservation retention, malformed frames/input, reset/escaped callback failures, caught late stdin/stdout/stderr after orderly shutdown, late output while active and control loss during blocked input/output. It does not prove cold descriptor fidelity, bounded resident memory under a sustained slow consumer, arbitrary Node resource cleanup, descendant custody or macOS/Linux product qualification.

The separate [actual-host streaming suite](../../tests/runtime/session/streaming.py) checks 128 MiB output with sustained reader pauses, resumed progress, an eager-buffering negative control and a broken output reader. Run it with an optimized `--mdrun` executable and an absolute `--node` path. Its hard bound covers accepted queued payload; RSS observations are supplementary. [Execution qualification](../../docs/plans/execution.md) records local and hosted results separately from application-specific cleanup guarantees.

The native-host [cold/warm parity guard](../../tests/integration/session_node.rs) uses an SDK-free Node entry point and the same seven-call native author as JVM and Python. It asserts literal bytes/statuses and actual routing across ordinary failure, binary output, changed-file reads and a 512 KiB echo; cold starts no worker and required-warm reuses exactly one. Run it with `MDRUN_NODE=/absolute/node cargo test --locked --test integration session::node_parity:: -- --ignored`. Local qualification passes; hosted qualification and application-owned resource examples remain separate gates.
