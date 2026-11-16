package io.github.mbackschat.mdrun.sdk;

import java.io.InputStream;
import java.io.OutputStream;
import java.util.List;
import java.util.Objects;

/** Provides SDK-owned immutable arguments and borrowed, binary streams for one invocation.
 *
 * Positive-length reads and writes are valid only while the handler runs. Writes are
 * synchronous and share one output sequence; overlapping writes to either output
 * stream poison the worker. Input is lazy, with at most one outstanding read.
 * Stream close and flush are no-ops, including after completion, and zero-length
 * operations emit no frames. Closing input does not cancel outstanding credit.
 */
public final class Invocation {
    private final String requestId;
    private final List<String> argv;
    private final InputStream stdin;
    private final OutputStream stdout;
    private final OutputStream stderr;
    private final Runnable invalidator;

    Invocation(String requestId, List<String> argv, InputStream stdin, OutputStream stdout,
            OutputStream stderr, Runnable invalidator) {
        this.requestId = Objects.requireNonNull(requestId);
        this.argv = List.copyOf(argv);
        this.stdin = Objects.requireNonNull(stdin);
        this.stdout = Objects.requireNonNull(stdout);
        this.stderr = Objects.requireNonNull(stderr);
        this.invalidator = Objects.requireNonNull(invalidator);
    }

    public String requestId() {
        return requestId;
    }

    public List<String> argv() {
        return argv;
    }

    public InputStream stdin() {
        return stdin;
    }

    public OutputStream stdout() {
        return stdout;
    }

    public OutputStream stderr() {
        return stderr;
    }

    /** Permanently faults the worker without emitting application output.
     *
     * While serving, this records a fixed infrastructure fault and throws an
     * unchecked exception. Catching it cannot restore health. A retained invocation
     * can also invalidate a later request or an idle worker. After Worker.run has
     * finished, invalidation immediately halts the dedicated JVM with status 70.
     * This method does not attest resource cleanup or promise immediate handler termination.
     */
    public void invalidate() {
        invalidator.run();
    }
}
