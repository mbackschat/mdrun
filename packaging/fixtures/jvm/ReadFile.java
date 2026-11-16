package io.github.mbackschat.mdrun.consumer;

import java.io.IOException;
import java.io.OutputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;

public final class ReadFile {
    private ReadFile() {}

    public static int run(List<String> argv, OutputStream output) throws IOException {
        if (argv.size() != 2 || !argv.getFirst().equals("read_file")) return 2;
        try (var input = Files.newInputStream(Path.of(argv.get(1)))) {
            byte[] chunk = new byte[32768];
            int length;
            while ((length = input.read(chunk)) != -1) output.write(chunk, 0, length);
        }
        return 0;
    }
}
