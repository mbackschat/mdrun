# Node worker SDK — version 1

Use this SDK when a Node command appears repeatedly in your executable Markdown and you want those calls to share application initialization. You write a small adapter around your application's callable command logic; mdrun keeps its dedicated Node worker alive for eligible calls within a managed session.

When ordinary `mdrun verify` executes a Markdown block containing `node /absolute/path/to/cli.mjs hello`, that call starts a new process, loads its modules and initializes the application. Repeated calls repeat that setup. Here `cli.mjs` stands for your existing application entry point, not a file supplied by mdrun. A warm worker can retain the modules and reusable application objects you deliberately initialize once. It still executes every command, reads changing inputs and produces fresh output; it does not cache results or make arbitrary scripts reusable.

The application supplies the normal CLI, the adapter and a binding JSON file identifying both launchers. The SDK supplies the worker transport, request streams and invocation lifecycle. The standalone Rust host launches the worker and controls routing and process custody; it requires Node only when you use a Node worker.

## Prepare the SDK

Use the [versioned SDK download and preparation guide](../../docs/sdk-distribution.md) to pin a host/SDK version and archive digest for local and CI builds. The prepared directory supplies `index.mjs`, a dependency-free ESM module; import it explicitly in your worker bootstrap. The archive contains no application adapter, so you create that file. Keep the normal cold entry point SDK-free.

The API follows the frozen [version-1 session contract](../../docs/plans/session-protocol.md) for 0.1.0. The distribution guide covers available GitHub release downloads and verified local assets.

## Implement an adapter

Save this complete, stateless example as `hello-worker.mjs` beside the prepared `index.mjs`. You create `hello-worker.mjs`; it contains both the bootstrap and the example command logic. In a real application, the adapter calls your existing command implementation and initializes reusable application objects before calling `runWorker`.

```js
import { runWorker } from './index.mjs';

try {
    await runWorker({
        configPath: process.argv[2],
        classify(argv) {
            return argv.length === 1 && argv[0] === 'hello'
                ? true
                : 'unsupported_command';
        },
        async invoke(request) {
            await request.stdout.write(Buffer.from('hello\n'));
            return 0;
        },
        async reset(request) {},
    });
} catch {
    process.exitCode = 70;
}
```

The host appends one private startup configuration path to `worker_argv`; with this bootstrap it becomes `process.argv[2]`. Pass it directly to `runWorker` and keep its contents and credentials out of application output. Do not run the bootstrap as an ordinary CLI or invent the private configuration yourself.

For each accepted `hello` call, the example writes the UTF-8 bytes `hello\n` to request stdout and returns status `0`. The handler runs again for the next call in the same worker. Its empty reset is justified only because this example is synchronous apart from the awaited SDK write and owns no mutable state or background work. Real adapters must stop and check their own work before permitting reuse.

## Connect it to a session

Create a [binding](../../docs/guides/sessions.md#what-you-prepare) with `worker_argv` containing your absolute Node executable and the absolute path to `hello-worker.mjs`. Set `cold_argv` to your independently runnable CLI, whose `hello` command must produce the same bytes and status. These are application-prepared paths; mdrun creates neither launcher. The host supplies the private configuration argument later.

Follow [managed-session verification](../../docs/guides/sessions.md#verify-and-inspect-the-route) to run repeated document commands through the binding. An eligible, context-compatible call uses the existing worker; a refused call can use the cold launcher before acceptance. `--require-warm` instead fails a call that cannot use the warm route. Compare cold and warm bytes, statuses and relevant file effects, then inspect the report for routing, reuse and clean cleanup. This adapter only accepts `hello`; adapt its three callbacks to qualify your real command path.

## Callback contract

| Callback | Contract |
| --- | --- |
| `classify(argv)` | Receive a frozen array of strings; return `true` or a stable nonempty refusal reason. It must perform no application effects, output or input reads. Refusal permits the host to choose cold execution. |
| `invoke(request)` | Return an integer exit status from 0 through 255, synchronously or as a promise. Nonzero status is ordinary application failure and permits subsequent reuse after successful reset and release. Exceptions poison the worker. |
| `reset(request)` | Mandatory; resolve only after application-owned work has ended and state/resource checks pass. Reject when safe reuse cannot be established. It runs after the handler returns and accepted output has flushed, before `complete`. |

## Borrowed streams

Each invocation receives a fresh frozen `request` containing `requestId`, frozen `argv`, `signal`, `stdin`, `stdout` and `stderr`. `stdin.read(maxBytes)` returns a promise for a nonempty `Buffer` or `null` at EOF; `maxBytes` must be an integer from 1 through 65536. Only an actual read emits credit. EOF is retained only for that invocation, one read may be pending, and the handler must await or otherwise finish it before returning. A handler that never reads creates no input credit.

`stdout.write(bytes)` and `stderr.write(bytes)` accept `Uint8Array`, including `Buffer`, and return promises. Await each write before making another write to either stream; overlapping writes poison the worker instead of creating an unbounded queue. A single write can exceed a frame because the SDK emits chunks of at most 65536 bytes with one shared sequence across both streams. Keep the supplied bytes unchanged until the promise settles. The SDK flushes its one accepted operation before reset even if the handler omitted `await`; this does not authorize detached application work. Request streams close to new positive-length operations when the handler returns. Valid zero-length writes emit no frames and remain inert during reset, through retained requests and after orderly shutdown. Positive-length writes through an old request poison reuse even when the application catches the rejected promise.

Application output must use request streams. `console`, process descriptors, imported stream singletons and native writes are not redirected; worker process stdout/stderr are private startup/diagnostic channels. The adapter must qualify the real command path and reject commands that depend on process-global descriptors, terminal behavior or unsupported native encoding.

## Completion and protocol checks

The SDK retains its reservation through `complete` until `release`, then sends `released`. No retry or cold fallback happens inside this peer. Authentication and readiness bind both connections to the private configuration identity. Strict frame schemas, UTF-8 with Unicode scalar strings, frame lengths, canonical padded Base64, credit and transition checks reject malformed peers. JSON objects reject duplicate field names, including equivalent escaped names. Request IDs remain unique for the worker lifetime, including refused and released requests. Fault reasons are stable SDK diagnostics; the protocol does not define one exhaustive reason vocabulary shared by all runtimes. Tokens and arbitrary callback error messages are excluded from SDK fault diagnostics.

## Shutdown and cleanup limits

Control closure or active shutdown aborts the request signal, rejects pending SDK input and breaks blocked socket output. `runWorker` rejects on poisoning and resolves on orderly idle shutdown. A retained request stream used after that resolution still records worker process failure with exit status 70, preserving an existing nonzero status, even when the application catches the rejected operation. A resolved promise cannot report this later violation, so the host must observe the worker's actual exit status. Adapters must let the process finish cooperatively and must not reset its failure status or force successful exit. The SDK installs process exception/rejection listeners only while running, restores them afterward, and rejects simultaneous workers in one process. Importing the module changes no process hooks. Bootstrap failures and faults must cause worker failure; do not catch them and continue serving work. The host independently owns process custody and termination deadlines, including handlers that block the event loop or ignore cancellation.

Reset is an explicit application attestation. The SDK does not enumerate or restore timers, event listeners, children, native handles, module caches, global variables or unresolved promises, and successful reset does not establish their absence. The adapter must stop and check its own work, listeners, timers and children; await application promises; reread mutable inputs; and qualify failure-then-success behavior against independent packaged execution. Uncaught exceptions and unhandled rejections poison the peer while it remains active; operations through retained request streams also record process failure after orderly shutdown. Detached work that never touches SDK streams can escape these guards, so it must be covered by application checks and host custody rather than claimed as generically reset.

## Qualification and evidence

Peer and native-host interoperability tests pass with Node v26.10.0 on macOS and Linux; [execution qualification](../../docs/plans/execution.md) records revisions and artifact checks. Each application still needs its own parity and resource qualification. The SDK is not an automatic Node state reset facility.

Run the independent fake-host suite with `node --test --test-timeout=10000 tests/runtime/node/worker.test.mjs`. It exercises lazy credit, 2.5 MB binary streaming with fresh EOF, stdout/stderr ordering, ordinary nonzero reuse, refusal with an independent no-effects marker, unique request IDs, duplicate JSON keys and invalid Unicode, inert zero-length output, output flushing, reservation retention, malformed frames/input, reset/escaped callback failures, caught late stdin/stdout/stderr after orderly shutdown, late output while active, active input without credit, active shutdown and control loss during blocked input/output. It does not prove cold descriptor fidelity, bounded resident memory under a sustained slow consumer, arbitrary Node resource cleanup, descendant custody or macOS/Linux product qualification.

The separate [actual-host streaming suite](../../tests/runtime/session/streaming.py) checks 128 MiB output with sustained reader pauses, resumed progress, an eager-buffering negative control and a broken output reader. Run it with an optimized `--mdrun` executable and an absolute `--node` path. Its hard bound covers accepted queued payload; RSS observations are supplementary. [Execution qualification](../../docs/plans/execution.md) records local and hosted results separately from application-specific cleanup guarantees.

The native-host [cold/warm parity guard](../../tests/integration/session_node.rs) uses an SDK-free Node entry point and the same seven-call native author as JVM and Python. It asserts literal bytes/statuses and actual routing across ordinary failure, binary output, changed-file reads and a 512 KiB echo; cold starts no worker and required-warm reuses exactly one. Run it with `MDRUN_NODE=/absolute/node cargo test --locked --test integration session::node_parity:: -- --ignored`. Local and macOS/Linux qualification (historical run) pass at `051cc3d`.

The [owned-resource fixture](../../tests/runtime/node/lifecycle-worker.mjs) demonstrates explicit application ownership of a timer, listener and child process. Its clean path awaits their termination before reset and reuse. Each negative control leaves one resource observably live, requires reset failure before bootstrap cleanup, then stops and awaits only its recorded handles. Disabling reset makes all three negative controls fail. These four cases bring the local peer suite to 37 passing tests; macOS/Linux CI (historical run) also passes all 37 controls at `6804d3d`. This example establishes a cooperative adapter pattern, not automatic detection of arbitrary Node resources.
