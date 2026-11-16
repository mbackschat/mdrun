package io.github.mbackschat.mdrun.sdk;

import java.io.InputStream;
import java.io.OutputStream;
import java.util.List;
import java.util.Objects;

/** Holds immutable arguments and borrowed, binary streams for one invocation.
 *
 * Positive-length reads and writes are valid only while the handler runs. Writes are
 * synchronous and share one output sequence; overlapping writes to either output
 * stream poison the worker. Input is lazy, with at most one outstanding read.
 * Stream close and flush are no-ops, including after completion, and zero-length
 * operations emit no frames. Closing input does not cancel outstanding credit.
 */
public record Invocation(
        String requestId,
        List<String> argv,
        InputStream stdin,
        OutputStream stdout,
        OutputStream stderr) {
    public Invocation {
        Objects.requireNonNull(requestId);
        argv = List.copyOf(argv);
        Objects.requireNonNull(stdin);
        Objects.requireNonNull(stdout);
        Objects.requireNonNull(stderr);
    }
}
