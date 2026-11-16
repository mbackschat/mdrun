# Experimental JVM worker SDK

This Java 21 peer implements the proposed [session protocol](../../docs/plans/session-protocol.md) for one cooperative application in a dedicated JVM process. Its API remains experimental; the [execution checklist](../../docs/plans/execution.md) owns qualification and the remaining application, lifecycle and platform gates. The native Rust host remains standalone: Java and Jackson are dependencies of the separately launched JVM worker, not requirements for starting the host or using its ordinary document commands.

## Prepare the SDK

Run the [preparation helper](../../scripts/prepare_jvm.py) from the repository root with a Java 21 JDK, an explicitly prepared Jackson core jar and a new or empty output directory:

```sh
python3 scripts/prepare_jvm.py \
  --java-home "$JAVA_HOME" \
  --jackson-core /prepared/jackson-core-2.22.4.jar \
  --output /prepared/mdrun-jvm
```

The helper uses `javac --release 21 -proc:none` and the JDK's `jar` tool; Maven is not needed. It compiles only the SDK package under `src/main/java`, excluding qualification fixtures, and packages classes with fixed archive timestamps, the project license and the SDK POM. It writes `mdrun-sdk.jar`, an unchanged `jackson-core-2.22.4.jar` and `build-manifest.json`; the manifest records relative classpath paths and SHA-256 identities. Keep both jars on the worker's classpath. Preparation does not package an application adapter, construct its host binding or publish a release or Maven artifact.

The [POM](pom.xml) exposes the exact Jackson dependency to standard consumers and security scanners. The [dependency lock](dependencies.lock.json) records Jackson core `2.22.4`, its 2026-10-17 release date, source URLs, Apache license, bundled notices and reviewed artifact hash. The helper checks POM agreement, a conservative 14 full days after the publication date, the exact jar hash and all five bundled notices before executing any Java tool. Jackson core has no external runtime dependencies in this reviewed configuration; databind and annotations are not added.

## Implement an adapter

The host appends one private configuration-file path to the binding's `worker_argv`. Pass that argument directly to `Worker.run`; it contains the host endpoint and credentials, and must stay out of application output. This minimal adapter accepts only `hello`, writes through its invocation's borrowed output stream and owns no mutable state or background work:

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

`Worker.run(Path, ApplicationFactory)` initializes the application after all three persistent SDK threads have started. The factory and application callbacks run on the same persistent execution thread, allowing the factory to establish a stable resource baseline. One worker lifetime is allowed per JVM; use a dedicated process and let bootstrap failure end it unsuccessfully.

| Application method | Contract |
| --- | --- |
| `Classification classify(List<String> argv)` | Receive immutable arguments; return `Eligible` or `Refused` with a stable nonempty reason. Perform no application effects, output or input reads. Refusal permits the host to select cold execution. |
| `int invoke(Invocation invocation)` | Return an application exit status from 0 through 255. Nonzero status is an ordinary application failure and allows reuse after successful reset and release. An exception or invalid status poisons the worker. |
| `void reset(Invocation invocation)` | Attest that application-owned work has stopped and state/resource checks permit another invocation. Throw when safe reuse cannot be established. Reset runs after the handler returns and accepted output finishes, before `complete`. |
| `void close()` | Release application resources when serving ends. The default implementation is empty; orderly worker return waits for the actual implementation. A close failure fails the worker. |

The example's empty reset is justified only by its stateless, synchronous behavior. A real adapter must stop and inspect its own threads, executors, children and other resources, reread mutable inputs and qualify failure-then-success behavior against independently launched packaged cold execution. Do not copy an empty reset into an adapter that owns work requiring cleanup.

## Borrowed streams and completion

The SDK constructs each immutable `Invocation`; adapters receive it rather than construct value objects. Its five accessors remain `requestId()`, `argv()`, `stdin()`, `stdout()` and `stderr()`, with immutable arguments and borrowed binary streams. Positive-length reads and writes are valid only while `invoke` runs. Input is lazy: an actual read issues credit for at most 65536 bytes, one read may be outstanding, and EOF lasts only for that invocation. A handler that ignores stdin issues no input credit. Finish all application reads before returning; outstanding input invalidates completion.

Writes are synchronous and apply socket backpressure. A single write may exceed one frame; the SDK chunks it and uses one sequence across stdout and stderr. Serialize writes across both streams: overlapping writes poison the worker. Keep supplied byte arrays unchanged until their write returns. Stream `close()` and output `flush()` are no-ops, and valid zero-length operations emit no frames, including after completion. Closing input does not cancel an outstanding read.

Application output must use the invocation streams. The SDK does not redirect `System.in`, `System.out`, `System.err`, cached process streams or native descriptors; worker process stdout/stderr are private startup and diagnostic channels. Classify commands requiring those global streams, terminal behavior or unsupported native encoding as cold unless their actual behavior has been qualified.

After `invoke` returns, the SDK closes the request to new positive-length I/O, finishes an already accepted output operation, runs reset and sends `complete`. It retains the reservation until the host sends `release`, then acknowledges `released`. No retry or cold fallback occurs inside the SDK. A late positive-length stream operation poisons the worker even if application code catches its `IOException`. After `Worker.run` has returned, such an operation immediately halts the dedicated JVM with status 70; the host must observe the real worker process status as well as the earlier application completion.

`invocation.invalidate()` records an irreversible worker fault without emitting application output. It accepts no arguments and uses the fixed SDK reason `application_invalidated`; while serving it throws an unchecked exception, and catching that exception cannot restore success or reuse. Calls through retained invocations fault the entire worker, including after completion/release or while a later request is running. After `Worker.run` has finished, invalidation immediately halts the dedicated JVM with status 70, including from a shutdown hook. Repeating invalidation cannot clear the fault. This capability lets an adapter report an application-detected violation even when a buffered character operation produces no bytes; it does not itself flush application buffers, observe wrappers that never call it, guarantee immediate handler termination or establish resource cleanup. Application cleanup checks and native host custody remain required.

## Shutdown and limits

An idle host shutdown allows `Worker.run` to return after application close and SDK thread shutdown. Control loss, active shutdown and protocol or callback faults stop admission, break blocked SDK I/O and interrupt the execution thread when the fault arrives from another thread. The SDK temporarily installs a default uncaught-exception handler while serving and restores the prior handler afterward. Do not catch bootstrap failure and continue serving, force successful exit or attempt a second worker lifetime.

The native host owns process custody and termination deadlines. SDK interruption cannot stop application code that ignores interrupts or blocks indefinitely; worker shutdown waits for application callbacks and close, while the host independently enforces its deadline. Reset is an explicit application attestation. The SDK does not discover or restore arbitrary static state, thread locals, executors, threads, children, native handles or shutdown hooks, and detached work that never touches SDK streams can escape its stream guards. Application checks and host custody must cover those limits; this SDK makes no general Java cleanup guarantee.

The [independent wire-peer suite](../../tests/runtime/jvm/qualify.py) exercises lazy input, nonzero reuse, refusal, multi-frame binary streams, fresh EOF, callback/reset failures, caught late I/O after shutdown, control loss during blocked input/output and sticky explicit invalidation while active, through retained requests and after orderly return with stalled shutdown hooks. The [preparation guards](../../tests/tooling/test_prepare_jvm.py) cover dependency age/hash/POM failures before compilation and an optional real-JDK SDK-only reproducibility check. These controls do not establish arbitrary application cleanup, cold descriptor parity, sustained slow-reader memory bounds, consumer migration or hosted macOS/Linux product qualification; record actual results in the execution checklist.

The separate [actual-host streaming suite](../../tests/runtime/session/streaming.py) checks 128 MiB output with sustained reader pauses, resumed progress, an eager-buffering negative control and a broken output reader. Run it with an optimized `--mdrun` executable and a prepared `--jvm-fixture` manifest from [fixture preparation](../../tests/runtime/jvm/prepare.py). Its hard bound covers accepted queued payload; RSS observations are supplementary. The execution checklist records local and hosted results separately from application-specific cleanup guarantees.
