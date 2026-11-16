package io.github.mbackschat.mdrun.bridgefixture;

import io.github.mbackschat.mdrun.sdk.Application;
import io.github.mbackschat.mdrun.sdk.Classification;
import io.github.mbackschat.mdrun.sdk.Invocation;
import io.github.mbackschat.mdrun.sdk.Worker;
import io.github.mbackschat.mdrun.example.InvocationOutput;
import java.io.IOException;
import java.nio.charset.Charset;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicReference;
import java.util.concurrent.locks.LockSupport;

public final class BridgeWorker implements Application {
    private final Charset stdoutCharset = System.out.charset();
    private final Charset stderrCharset = System.err.charset();
    private Appendable previous;
    private static Appendable retained;
    private static String lateMode;

    public static void main(String[] args) {
        try {
            Worker.run(Path.of(args[0]), BridgeWorker::new);
            if (retained != null) {
                if (lateMode.equals("hook")) {
                    CountDownLatch started = new CountDownLatch(1);
                    Runtime.getRuntime().addShutdownHook(new Thread(() -> {
                        started.countDown();
                        for (;;) LockSupport.park();
                    }));
                    Runtime.getRuntime().addShutdownHook(new Thread(() -> {
                        try { started.await(); } catch (InterruptedException ignored) { }
                        appendLate();
                    }));
                } else {
                    appendLate();
                }
            }
        } catch (Exception failure) {
            System.exit(70);
        }
    }

    private static void appendLate() {
        try {
            switch (lateMode) {
                case "sequence" -> retained.append("\uD83D");
                case "range" -> retained.append("x\uD83Dy", 1, 2);
                default -> retained.append('\uD83D');
            }
        } catch (IOException | RuntimeException ignored) { }
    }

    @Override public Classification classify(List<String> argv) { return new Classification.Eligible(); }

    @Override public int invoke(Invocation request) throws Exception {
        String command = request.argv().getFirst();
        InvocationOutput output = new InvocationOutput(request, stdoutCharset, stderrCharset);
        try {
            switch (command) {
                case "closed_high" -> {
                    output.close();
                    try { output.stdout().append('\uD83D'); } catch (IOException | RuntimeException ignored) { }
                    return 0;
                }
                case "empty_closed" -> {
                    output.close();
                    output.close();
                    output.stdout().append("").append("abc", 1, 1);
                    output.stderr().append("", 0, 0);
                    try {
                        output.stdout().append("abc", -1, 1);
                        throw new AssertionError("accepted invalid range after close");
                    } catch (IndexOutOfBoundsException expected) { }
                    return 0;
                }
                case "capture" -> { previous = output.stdout(); return 0; }
                case "append_previous" -> {
                    try { previous.append('\uD83D'); } catch (IOException | RuntimeException ignored) { }
                    return 0;
                }
                case "late_character", "late_sequence", "late_range", "late_hook" -> {
                    retained = output.stdout();
                    lateMode = command.substring("late_".length());
                    return 0;
                }
                case "mutating_sequence" -> {
                    try {
                        output.stdout().append(new CharSequence() {
                            @Override public int length() { return 8193; }
                            @Override public char charAt(int index) {
                                if (index >= 5000) throw new IndexOutOfBoundsException("private sequence content");
                                return 'm';
                            }
                            @Override public CharSequence subSequence(int start, int end) { return "m"; }
                            @Override public String toString() { return "m"; }
                        });
                    } catch (IOException | RuntimeException ignored) { }
                    return 0;
                }
                case "concurrent", "concurrent_streams" -> {
                    return concurrent(output, Path.of(request.argv().get(1)), command.equals("concurrent_streams"));
                }
                default -> {
                    int status = Scenario.run(command, output.stdout(), output.stderr());
                    if (command.equals("large") && Scenario.conversions != 0) request.invalidate();
                    return status;
                }
            }
        } finally {
            output.close();
        }
    }

    private static int concurrent(InvocationOutput output, Path gate, boolean otherStream) throws Exception {
        CountDownLatch entered = new CountDownLatch(1);
        CountDownLatch closing = new CountDownLatch(1);
        AtomicReference<Throwable> failure = new AtomicReference<>();
        CharSequence text = new CharSequence() {
            private void hold() {
                entered.countDown();
                while (!Files.exists(gate)) LockSupport.parkNanos(1_000_000);
            }
            @Override public int length() { return Scenario.CONCURRENT.length(); }
            @Override public char charAt(int index) { if (index == 0) hold(); return Scenario.CONCURRENT.charAt(index); }
            @Override public CharSequence subSequence(int start, int end) { return Scenario.CONCURRENT.subSequence(start, end); }
            @Override public String toString() { hold(); return Scenario.CONCURRENT; }
        };
        Thread writer = Thread.ofPlatform().daemon().start(() -> {
            try { output.stdout().append(text); } catch (Throwable error) { failure.set(error); }
        });
        if (!entered.await(3, TimeUnit.SECONDS)) throw new AssertionError("append did not enter");
        Thread closer = Thread.ofPlatform().daemon().start(() -> {
            closing.countDown();
            try {
                if (otherStream) output.stderr().append("after");
                else output.close();
                Files.writeString(Path.of(gate + ".finished"), "finished");
            } catch (Throwable error) { failure.set(error); }
        });
        if (!closing.await(3, TimeUnit.SECONDS)) throw new AssertionError("close did not start");
        Files.writeString(Path.of(gate + ".ready"), "ready");
        writer.join(5000);
        closer.join(5000);
        if (writer.isAlive() || closer.isAlive() || failure.get() != null) throw new AssertionError("operation did not finish", failure.get());
        return 0;
    }

    @Override public void reset(Invocation request) {}
}
