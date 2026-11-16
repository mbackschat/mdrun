# JVM worker SDK — version 1

Use this SDK when an executable Markdown document calls a JVM application repeatedly and you want those calls to share a running JVM and application initialization. You write a small adapter around a callable application; mdrun sends eligible commands to it within a managed session.

When ordinary `mdrun verify` executes a Markdown block containing `java -jar /absolute/path/to/mytool.jar hello`, that call starts a JVM and loads and initializes the application. Repeated calls repeat that setup. `mytool.jar` here stands for your packaged application, not a file supplied by mdrun. A warm worker keeps its JVM, loaded classes and reusable application objects alive between calls. Each invocation still executes the command and reads changing inputs; results are not cached. The application must provide a safe boundary between requests.

You supply the packaged CLI, the adapter and a binding JSON file identifying both launchers. The SDK supplies worker transport, borrowed request streams and a callback lifecycle. The native Rust host manages routing and process custody. Java and Jackson belong to the separately launched worker; starting the host and using ordinary document commands requires neither.

## Prepare the SDK

For consumer builds, use the [versioned SDK download and preparation guide](../../docs/sdk-distribution.md). The JVM archive already contains the prepared SDK and locked Jackson jars, POM, dependency provenance, licenses and hashes; a source checkout, Maven and a compiler are unnecessary to prepare that archive. A Java 21 runtime remains necessary to run a worker, and compiling your own adapter requires a Java 21 JDK. Pin the exact host/SDK version and archive digest in both local and CI builds.

No product version has been published and publication is held. The distribution guide covers the future release-download route and preparation of verified local assets. The API follows the frozen [version-1 session contract](../../docs/plans/session-protocol.md) for the 0.1.0 release candidate.

## Implement an adapter

Save this complete example as `ExampleWorker.java` in your application project and compile it against the prepared SDK jar. You create this file; its `Greeting` class is the entire example application. In a real adapter, call your existing application logic and initialize reusable objects in the application factory or constructor.

The host appends one private configuration-file path to the binding's `worker_argv`. With this bootstrap it becomes `args[0]`; pass it directly to `Worker.run`. It contains the host endpoint and credentials, so keep it out of application output. Do not launch this bootstrap as a normal CLI or construct the private configuration yourself.

```java
import io.github.mbackschat.mdrun.sdk.Application;
import io.github.mbackschat.mdrun.sdk.Classification;
import io.github.mbackschat.mdrun.sdk.Invocation;
import io.github.mbackschat.mdrun.sdk.Worker;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Path;
import java.util.List;

public final class ExampleWorker {
    public static void main(String[] args) {
        try {
            Worker.run(Path.of(args[0]), Greeting::new);
        } catch (Throwable failure) {
            System.exit(70);
        }
    }

    private static final class Greeting implements Application {
        @Override
        public Classification classify(List<String> argv) {
            return argv.equals(List.of("hello"))
                    ? new Classification.Eligible()
                    : new Classification.Refused("unsupported_command");
        }

        @Override
        public int invoke(Invocation invocation) throws IOException {
            invocation.stdout().write("hello\n".getBytes(StandardCharsets.UTF_8));
            return 0;
        }

        @Override
        public void reset(Invocation invocation) {}
    }
}
```

Each accepted `hello` call writes the UTF-8 bytes `hello\n` to invocation stdout and returns status `0`. Another accepted call runs the handler again on the same application instance.

The example's empty reset is justified only by its stateless, synchronous behavior. A real adapter must stop and inspect its own threads, executors, children and other resources, reread mutable inputs and qualify failure-then-success behavior against independently launched packaged cold execution. Do not copy an empty reset into an adapter that owns work requiring cleanup.

For callable applications taking `Appendable`, the separate [application output example](examples/README.md) preserves process-stream charsets while enforcing invocation lifetime before character encoding. Close its admission inside `invoke`, before returning. It is a small application helper outside the SDK package; global stream routing and application reset/resource checks remain the adapter's responsibility.

## Connect it to a session

Create a [binding](../../docs/guides/sessions.md#what-you-prepare) whose `worker_argv` launches your absolute Java executable, with a classpath containing the compiled adapter and both prepared jars, followed by `ExampleWorker`. Set `cold_argv` to your independent packaged CLI, with its normal dependencies. Its `hello` command must produce the same bytes and status without the SDK. mdrun creates neither application launcher and supplies the private configuration argument only at worker startup.

Follow [managed-session verification](../../docs/guides/sessions.md#verify-and-inspect-the-route) to run repeated document commands through the binding. Eligible calls with a compatible context reuse the worker. A refusal can select the cold launcher before acceptance; `--require-warm` instead fails a call that cannot use the warm route. Compare cold and warm bytes, statuses and relevant file effects, then inspect routing, worker starts and cleanup in the report.

## Callback contract

`Worker.run(Path, ApplicationFactory)` initializes the application after all three persistent SDK threads have started. The factory and application callbacks run on the same persistent execution thread, allowing the factory to establish a stable resource baseline. The SDK clears interrupt status when the factory, classification, invocation or reset callback ends so it cannot leak into SDK waits or the next callback; cancellation remains recorded independently in worker state. One worker lifetime is allowed per JVM; use a dedicated process and let bootstrap failure end it unsuccessfully.

| Application method | Contract |
| --- | --- |
| `Classification classify(List<String> argv)` | Receive immutable arguments; return `Eligible` or `Refused` with a stable nonempty reason. Perform no application effects, output or input reads. Refusal permits the host to select cold execution. |
| `int invoke(Invocation invocation)` | Return an application exit status from 0 through 255. Nonzero status is an ordinary application failure and allows reuse after successful reset and release. An exception or invalid status poisons the worker. |
| `void reset(Invocation invocation)` | Attest that application-owned work has stopped and state/resource checks permit another invocation. Throw when safe reuse cannot be established. Reset runs after the handler returns and accepted output finishes, before `complete`. |
| `void close()` | Release application resources when serving ends. The default implementation is empty; orderly worker return waits for the actual implementation. A close failure fails the worker. |

## Borrowed streams and completion

The SDK constructs each immutable `Invocation`; adapters receive it rather than construct value objects. Its five accessors remain `requestId()`, `argv()`, `stdin()`, `stdout()` and `stderr()`, with immutable arguments and borrowed binary streams. Positive-length reads and writes are valid only while `invoke` runs. Input is lazy: an actual read issues credit for at most 65536 bytes, one read may be outstanding, and EOF lasts only for that invocation. A handler that ignores stdin issues no input credit. Finish all application reads before returning; outstanding input invalidates completion.

Writes are synchronous and apply socket backpressure. A pending application thread interrupt does not discard output or change an ordinary return status; output preserves the flag for application code until the callback ends. Worker cancellation still closes the channels and interrupts running application code. A single write may exceed one frame; the SDK chunks it and uses one sequence across stdout and stderr. Serialize writes across both streams: overlapping writes poison the worker. Keep supplied byte arrays unchanged until their write returns. Stream `close()` and output `flush()` are no-ops, and valid zero-length operations emit no frames, including after completion. Closing input does not cancel an outstanding read.

Application output must use the invocation streams. The SDK does not redirect `System.in`, `System.out`, `System.err`, cached process streams or native descriptors; worker process stdout/stderr are private startup and diagnostic channels. Classify commands requiring those global streams, terminal behavior or unsupported native encoding as cold unless their actual behavior has been qualified.

After `invoke` returns, the SDK closes the request to new positive-length I/O, finishes an already accepted output operation, runs reset and sends `complete`. It retains the reservation until the host sends `release`, then acknowledges `released`. No retry or cold fallback occurs inside the SDK. A late positive-length stream operation poisons the worker even if application code catches its `IOException`. After `Worker.run` has returned, such an operation immediately halts the dedicated JVM with status 70; the host must observe the real worker process status as well as the earlier application completion.

`invocation.invalidate()` records an irreversible worker fault without emitting application output. It accepts no arguments and uses the fixed SDK reason `application_invalidated`; while serving it throws an unchecked exception, and catching that exception cannot restore success or reuse. Calls through retained invocations fault the entire worker, including after completion/release or while a later request is running. After `Worker.run` has finished, invalidation immediately halts the dedicated JVM with status 70, including from a shutdown hook. Repeating invalidation cannot clear the fault. This capability lets an adapter report an application-detected violation even when a buffered character operation produces no bytes; it does not itself flush application buffers, observe wrappers that never call it, guarantee immediate handler termination or establish resource cleanup. Application cleanup checks and native host custody remain required.

## Shutdown and limits

An idle host shutdown allows `Worker.run` to return after application close and SDK thread shutdown. Control loss, active shutdown and protocol or callback faults stop admission, break blocked SDK I/O and interrupt the execution thread when the fault arrives from another thread. The SDK temporarily installs a default uncaught-exception handler while serving and restores the prior handler afterward. Do not catch bootstrap failure and continue serving, force successful exit or attempt a second worker lifetime.

The native host owns process custody and termination deadlines. SDK interruption cannot stop application code that ignores interrupts or blocks indefinitely; worker shutdown waits for application callbacks and close, while the host independently enforces its deadline. Reset is an explicit application attestation. The SDK does not discover or restore arbitrary static state, thread locals, executors, threads, children, native handles or shutdown hooks, and detached work that never touches SDK streams can escape its stream guards. Application checks and host custody must cover those limits; this SDK makes no general Java cleanup guarantee.

## Build the SDK from source

Source contributors can reproduce the prepared jar as follows.

Run the [preparation helper](../../scripts/prepare_jvm.py) from the repository root with a Java 21 JDK, an explicitly prepared Jackson core jar and a new or empty output directory:

```sh
python3 scripts/prepare_jvm.py \
  --java-home "$JAVA_HOME" \
  --jackson-core /prepared/jackson-core-2.22.4.jar \
  --output /prepared/mdrun-jvm
```

The helper uses `javac --release 21 -proc:none` and the JDK's `jar` tool; Maven is not needed. It compiles only the SDK package under `src/main/java`, excluding qualification fixtures, and packages classes with fixed archive timestamps, the project license and the SDK POM. It writes `mdrun-sdk.jar`, an unchanged `jackson-core-2.22.4.jar` and `build-manifest.json`; the manifest records relative classpath paths and SHA-256 identities. Keep both jars on the worker's classpath. Preparation does not package an application adapter, construct its host binding or publish a release or Maven artifact.

The [POM](pom.xml) exposes the exact Jackson dependency to standard consumers and security scanners. The [dependency lock](dependencies.lock.json) records Jackson core `2.22.4`, its 2026-10-17 release date, source URLs, Apache license, bundled notices and reviewed artifact hash. The helper checks POM agreement, a conservative 14 full days after the publication date, the exact jar hash and all five bundled notices before executing any Java tool. Jackson core has no external runtime dependencies in this reviewed configuration; databind and annotations are not added.

Use `--download` instead of `--jackson-core` during preparation to request the exact locked Maven Central URL. Downloads reject redirects and must match the locked hash before compilation. The [dependency evidence](../../docs/dependencies.md#jvm-sdk-development-dependency) records canonical-source verification and its historical qualification limits; SDK distribution and application qualification remain separate gates.

## Qualification and evidence

Each application still requires its own parity and resource qualification. The [execution checklist](../../docs/plans/execution.md) records runtime evidence and artifact checks for the frozen 0.1.0 candidate; publication remains held.

The [independent wire-peer suite](../../tests/runtime/jvm/qualify.py) exercises lazy input, active input without credit, nonzero reuse, refusal with an independent no-effects marker, pending-interrupt factory readiness, classification outcomes, output/status parity and reuse, multi-frame binary streams, fresh EOF, callback/reset failures, caught late I/O after shutdown, control loss during blocked input/output and sticky explicit invalidation while active, through retained requests and after orderly return with stalled shutdown hooks. The [preparation guards](../../tests/tooling/test_prepare_jvm.py) cover dependency age/hash/POM failures before compilation and an optional real-JDK SDK-only reproducibility check. These controls do not establish arbitrary application cleanup, cold descriptor parity, sustained slow-reader memory bounds, consumer migration or hosted macOS/Linux product qualification; record actual results in the execution checklist.

The separate [actual-host streaming suite](../../tests/runtime/session/streaming.py) checks 128 MiB output with sustained reader pauses, resumed progress, an eager-buffering negative control and a broken output reader. Run it with an optimized `--mdrun` executable and a prepared `--jvm-fixture` manifest from [fixture preparation](../../tests/runtime/jvm/prepare.py). Its hard bound covers accepted queued payload; RSS observations are supplementary. The execution checklist records local and hosted results separately from application-specific cleanup guarantees.
