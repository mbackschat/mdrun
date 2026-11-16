package io.github.mbackschat.mdrun.fixture;

import io.github.mbackschat.mdrun.sdk.Application;
import io.github.mbackschat.mdrun.sdk.Classification;
import io.github.mbackschat.mdrun.sdk.Invocation;
import io.github.mbackschat.mdrun.sdk.Worker;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.util.List;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;

/** Attests explicitly owned handles before bootstrap cleanup can hide retained work. */
public final class Lifecycle implements Application {
    private final Path directory;
    private final AtomicBoolean stopThread = new AtomicBoolean();
    private Thread ownedThread;
    private Process child;

    private Lifecycle(Path directory) {
        this.directory = directory;
    }

    public static void main(String[] args) throws Exception {
        if (args[0].equals("child")) {
            Path directory = Path.of(args[1]);
            AtomicBoolean stopping = new AtomicBoolean();
            Thread progress = heartbeat(directory, "child", stopping);
            System.in.read();
            stopping.set(true);
            progress.join(1000);
            if (progress.isAlive()) throw new IllegalStateException("child heartbeat did not stop");
            publish(directory, "child.closed.json", "{\"clean\":true}");
            return;
        }
        Path config = Path.of(args[0]);
        try {
            Worker.run(config, () -> new Lifecycle(config.getParent()));
        } catch (Exception failure) {
            System.exit(70);
        }
    }

    private static void publish(Path directory, String name, String value) throws Exception {
        Path destination = directory.resolve(name);
        Path pending = directory.resolve(name + ".pending");
        Files.writeString(pending, value);
        Files.move(pending, destination, StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING);
    }

    private static Thread heartbeat(Path directory, String actor, AtomicBoolean stopping) {
        return Thread.ofPlatform().name("fixture-owned-" + actor).start(() -> {
            long deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(8);
            try {
                for (long count = 1; !stopping.get(); count++) {
                    if (System.nanoTime() >= deadline) {
                        Files.writeString(directory.resolve(actor + ".expired"), "fixture deadline expired");
                        Runtime.getRuntime().halt(72);
                    }
                    publish(directory, actor + ".progress", Long.toString(count));
                    Thread.sleep(10);
                }
            } catch (Exception failure) {
                Runtime.getRuntime().halt(73);
            }
        });
    }

    private void awaitGate(String name) throws Exception {
        long deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(3);
        while (!Files.exists(directory.resolve(name))) {
            if (System.nanoTime() >= deadline) {
                Files.writeString(directory.resolve("gate.expired"), name);
                throw new IllegalStateException("fixture gate expired");
            }
            Thread.sleep(5);
        }
    }

    @Override
    public Classification classify(List<String> argv) {
        return new Classification.Eligible();
    }

    @Override
    public int invoke(Invocation request) throws Exception {
        String kind = request.argv().getFirst();
        if (kind.equals("ignore")) return 0;
        if (kind.equals("clean") || kind.equals("thread")) {
            ownedThread = heartbeat(directory, "thread", stopThread);
        }
        if (kind.equals("clean") || kind.equals("child")) {
            child = new ProcessBuilder(Path.of(System.getProperty("java.home"), "bin", "java").toString(),
                    "-cp", System.getProperty("java.class.path"), Lifecycle.class.getName(), "child", directory.toString())
                    .redirectOutput(ProcessBuilder.Redirect.DISCARD)
                    .redirectError(ProcessBuilder.Redirect.DISCARD).start();
        }
        publish(directory, "owned.ready.json", "{\"kinds\":"
                + (kind.equals("clean") ? "[\"thread\",\"child\"]" : "[\"" + kind + "\"]") + "}");
        awaitGate("invoke.continue");
        if (kind.equals("clean")) {
            closeOwned();
            publish(directory, "invocation.closed.json", "{\"clean\":true}");
        }
        return 0;
    }

    @Override
    public void reset(Invocation request) throws Exception {
        if (ownedThread != null || child != null) {
            publish(directory, "reset.failed.json", "{\"actor\":\"" + request.argv().getFirst() + "\"}");
            throw new IllegalStateException("application-owned work remained live");
        }
    }

    private void closeOwned() throws Exception {
        if (ownedThread != null) {
            stopThread.set(true);
            ownedThread.join(1000);
            if (ownedThread.isAlive()) throw new IllegalStateException("owned thread did not stop");
            ownedThread = null;
        }
        if (child != null) {
            child.getOutputStream().close();
            if (!child.waitFor(1500, TimeUnit.MILLISECONDS)) {
                child.destroyForcibly();
                child.waitFor();
                throw new IllegalStateException("owned child required escalation");
            }
            int status = child.exitValue();
            publish(directory, "child.observed.json", "{\"exit_code\":" + status + "}");
            if (status != 0) throw new IllegalStateException("owned child failed");
            child = null;
        }
    }

    @Override
    public void close() throws Exception {
        try {
            if (ownedThread != null || child != null) {
                publish(directory, "cleanup.pending.json", "{}");
                awaitGate("cleanup.continue");
            }
        } finally {
            publish(directory, "cleanup.started.json", "{}");
            try {
                closeOwned();
                publish(directory, "cleanup.closed.json", "{\"clean\":true}");
            } catch (Exception failure) {
                Files.writeString(directory.resolve("cleanup.failed"), failure.toString());
                throw failure;
            }
        }
    }
}
