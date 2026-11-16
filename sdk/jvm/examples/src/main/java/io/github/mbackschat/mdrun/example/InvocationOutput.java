package io.github.mbackschat.mdrun.example;

import io.github.mbackschat.mdrun.sdk.Invocation;
import java.io.PrintStream;
import java.nio.charset.Charset;
import java.util.Objects;

/** Adapts one invocation to opaque Appendable sinks with process PrintStream encoding.
 *
 * Create a fresh instance with the original stdout/stderr charsets and close it
 * inside invoke before returning. Both sinks and close share one admission lock;
 * accepted operations finish synchronously, and later nonempty appends invalidate
 * the worker before encoding. Callers must leave character sequences unchanged
 * until append returns. The helper owns neither global streams nor application resources.
 */
public final class InvocationOutput implements AutoCloseable {
    private final Invocation invocation;
    private final Object lock = new Object();
    private final char[] characters = new char[4096];
    private final PrintStream out;
    private final PrintStream err;
    private final Appendable stdout;
    private final Appendable stderr;
    private boolean closed;

    public InvocationOutput(Invocation invocation, Charset stdoutCharset, Charset stderrCharset) {
        this.invocation = Objects.requireNonNull(invocation);
        out = new PrintStream(invocation.stdout(), true, Objects.requireNonNull(stdoutCharset));
        err = new PrintStream(invocation.stderr(), true, Objects.requireNonNull(stderrCharset));
        stdout = new Sink(out);
        stderr = new Sink(err);
    }

    public Appendable stdout() {
        return stdout;
    }

    public Appendable stderr() {
        return stderr;
    }

    private void emit(PrintStream destination, Runnable operation) {
        synchronized (lock) {
            if (closed) invocation.invalidate();
            try {
                operation.run();
            } catch (RuntimeException | Error failure) {
                invocation.invalidate();
                throw failure;
            }
            if (destination.checkError()) invocation.invalidate();
        }
    }

    /** Closes admission and flushes accepted output; repeating close is harmless. */
    @Override
    public void close() {
        synchronized (lock) {
            if (closed) return;
            closed = true;
            // Encoder close changes orphan-surrogate output; output_bridge.py compares independent System.exit behavior.
            out.flush();
            err.flush();
            if (out.checkError() || err.checkError()) invocation.invalidate();
        }
    }

    private final class Sink implements Appendable {
        private final PrintStream destination;

        Sink(PrintStream destination) {
            this.destination = destination;
        }

        @Override
        public Appendable append(CharSequence sequence) {
            CharSequence value = sequence == null ? "null" : sequence;
            return append(value, 0, value.length());
        }

        @Override
        public Appendable append(CharSequence sequence, int start, int end) {
            CharSequence value = sequence == null ? "null" : sequence;
            Objects.checkFromToIndex(start, end, value.length());
            if (start == end) return this;
            emit(destination, () -> {
                for (int offset = start; offset < end;) {
                    int count = Math.min(characters.length, end - offset);
                    for (int index = 0; index < count; index++) characters[index] = value.charAt(offset + index);
                    destination.print(new String(characters, 0, count));
                    offset += count;
                }
            });
            return this;
        }

        @Override
        public Appendable append(char character) {
            emit(destination, () -> destination.print(character));
            return this;
        }
    }
}
