package io.github.mbackschat.mdrun.fixture;

import io.github.mbackschat.mdrun.sdk.Application;
import io.github.mbackschat.mdrun.sdk.Classification;
import io.github.mbackschat.mdrun.sdk.Invocation;
import io.github.mbackschat.mdrun.sdk.Worker;
import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.nio.file.StandardOpenOption;
import java.util.List;
import java.util.Set;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.locks.LockSupport;

public final class Fixture implements Application {
    private final Set<Thread> baseline = Thread.getAllStackTraces().keySet();
    private static Invocation retained;
    private static Invocation previous;
    private static String lateOperation;

    public static void main(String[] args) {
        try {
            Worker.run(Path.of(args[0]), Fixture::new);
            if (retained != null) {
                if (lateOperation.equals("late_hook") || lateOperation.equals("late_invalidate_hook")) {
                    CountDownLatch blocked = new CountDownLatch(1);
                    Runtime.getRuntime().addShutdownHook(new Thread(() -> {
                        blocked.countDown();
                        for (;;) LockSupport.park();
                    }));
                    Runtime.getRuntime().addShutdownHook(new Thread(() -> {
                        try { blocked.await(); } catch (InterruptedException ignored) { }
                        lateIo();
                    }));
                } else {
                    Thread late = Thread.ofPlatform().start(Fixture::lateIo);
                    late.join();
                }
            }
        } catch (Exception failure) {
            System.exit(70);
        }
    }

    private static void lateIo() {
        try {
            if (lateOperation.equals("late_invalidate") || lateOperation.equals("late_invalidate_hook")) {
                invalidateCaught(retained);
                return;
            }
            if (lateOperation.equals("late_noop")) {
                retained.stdin().close();
                retained.stdout().close();
                retained.stderr().flush();
                retained.stdout().write(new byte[0]);
                if (retained.stdin().read(new byte[0]) != 0) throw new AssertionError("zero-length input");
                return;
            }
            if (lateOperation.equals("late_input")) retained.stdin().read();
            else if (lateOperation.equals("late_stderr")) retained.stderr().write(1);
            else retained.stdout().write(1);
        } catch (IOException ignored) { }
    }

    private static void invalidateCaught(Invocation request) {
        try {
            request.invalidate();
        } catch (RuntimeException ignored) { }
    }

    @Override
    public Classification classify(List<String> argv) {
        if (!argv.isEmpty() && argv.getFirst().equals("environment")) {
            return new Classification.Refused("observes_shell_environment");
        }
        return !argv.isEmpty() && argv.getFirst().equals("refuse")
            ? new Classification.Refused("unsupported_command") : new Classification.Eligible();
    }

    @Override
    public int invoke(Invocation request) throws Exception {
        try {
            request.argv().add("mutated");
            throw new IllegalStateException("request argv must be immutable");
        } catch (UnsupportedOperationException expected) { }
        return switch (request.argv().isEmpty() ? "" : request.argv().getFirst()) {
            case "invalidate_caught", "invalidate_twice" -> {
                invalidateCaught(request);
                if (request.argv().getFirst().equals("invalidate_twice")) invalidateCaught(request);
                yield 0;
            }
            case "retain_invalidation" -> {
                previous = request;
                yield 0;
            }
            case "invalidate_previous" -> {
                invalidateCaught(previous);
                yield 0;
            }
            case "invalidate_gated" -> {
                Path gate = Path.of(request.argv().get(1));
                Thread.ofPlatform().daemon().name("fixture-invalidation").start(() -> {
                    while (!Files.exists(gate)) LockSupport.parkNanos(1_000_000);
                    invalidateCaught(request);
                });
                yield 0;
            }
            case "refuse" -> {
                Files.writeString(Path.of(request.argv().get(1)), "invoked\n", StandardOpenOption.CREATE, StandardOpenOption.APPEND);
                yield 0;
            }
            case "gated_no_input" -> {
                new CountDownLatch(1).await();
                yield 0;
            }
            case "throw" -> throw new IllegalStateException("private application detail");
            case "blocked_input" -> {
                request.stdin().read();
                yield 0;
            }
            case "blocked_output", "interrupted_blocked_output" -> {
                if (request.argv().getFirst().equals("interrupted_blocked_output")) Thread.currentThread().interrupt();
                byte[] bytes = new byte[65536];
                for (;;) request.stdout().write(bytes);
            }
            case "streaming" -> {
                Path progress = Path.of(request.argv().get(1));
                byte[] pattern = new byte[65536];
                for (int index = 0; index < pattern.length; index++) pattern[index] = (byte) (index % 251);
                publishProgress(progress, 0);
                for (long completed = 0; completed < 128L * 1024 * 1024;) {
                    request.stdout().write(pattern);
                    completed += pattern.length;
                    publishProgress(progress, completed);
                }
                yield 0;
            }
            case "borrowed_close" -> {
                request.stdin().close();
                request.stdout().close();
                request.stderr().close();
                request.stdout().write(new byte[0]);
                if (request.stdin().read(new byte[0]) != 0) throw new AssertionError("zero-length input");
                request.stdout().write(new byte[] {'o', 'k'});
                request.stdout().flush();
                yield 0;
            }
            case "late_stdout", "late_stderr", "late_input", "late_hook", "late_noop", "late_invalidate", "late_invalidate_hook" -> {
                retained = request;
                lateOperation = request.argv().getFirst();
                yield 0;
            }
            default -> Example.run(request.argv(), request.stdin(), request.stdout(), request.stderr());
        };
    }

    private static void publishProgress(Path progress, long completed) throws IOException {
        Path pending = progress.resolveSibling(progress.getFileName() + ".pending");
        Files.writeString(pending, "{\"pid\":" + ProcessHandle.current().pid()
                + ",\"completed_bytes\":" + completed + "}");
        Files.move(pending, progress, StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING);
    }

    @Override
    public void reset(Invocation request) {
        if (Thread.currentThread().isInterrupted()) throw new AssertionError("callback interrupt leaked into reset");
        if (!request.argv().isEmpty() && request.argv().getFirst().equals("resetfail")) {
            throw new IllegalStateException("resource remained live");
        }
        for (Thread thread : Thread.getAllStackTraces().keySet()) {
            if (thread.isAlive() && thread.getName().startsWith("mdrun-") && !baseline.contains(thread)) {
                throw new IllegalStateException("SDK thread appeared after application baseline");
            }
        }
    }
}
