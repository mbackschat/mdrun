package io.github.mbackschat.mdrun.consumer;

import io.github.mbackschat.mdrun.sdk.Application;
import io.github.mbackschat.mdrun.sdk.Classification;
import io.github.mbackschat.mdrun.sdk.Invocation;
import io.github.mbackschat.mdrun.sdk.Worker;
import java.io.IOException;
import java.nio.file.Path;
import java.util.List;

public final class WorkerMain {
    private WorkerMain() {}

    public static void main(String[] argv) {
        try {
            Worker.run(Path.of(argv[0]), Reader::new);
        } catch (Throwable failure) {
            System.exit(70);
        }
    }

    private static final class Reader implements Application {
        @Override
        public Classification classify(List<String> argv) {
            return argv.size() == 2 && argv.getFirst().equals("read_file")
                    ? new Classification.Eligible()
                    : new Classification.Refused("unsupported_command");
        }

        @Override
        public int invoke(Invocation request) throws IOException {
            return ReadFile.run(request.argv(), request.stdout());
        }

        @Override
        public void reset(Invocation request) {
            // ReadFile closes its one file before returning and owns no detached work.
        }
    }
}
