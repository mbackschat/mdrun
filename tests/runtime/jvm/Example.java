package io.github.mbackschat.mdrun.fixture;

import java.io.InputStream;
import java.io.OutputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;

/** Supplies the same application behavior to independent cold and cooperative warm launchers. */
public final class Example {
    private Example() {}

    public static int run(List<String> argv, InputStream stdin, OutputStream stdout, OutputStream stderr) throws Exception {
        return switch (argv.isEmpty() ? "" : argv.getFirst()) {
            case "ignore" -> {
                stdout.write(new byte[] {'o', 'k'});
                yield 0;
            }
            case "echo" -> {
                byte[] bytes = new byte[32768];
                for (int count; (count = stdin.read(bytes)) != -1;) stdout.write(bytes, 0, count);
                if (stdin.read(bytes) != -1) throw new IllegalStateException("EOF changed");
                yield 0;
            }
            case "binary" -> {
                stdout.write(new byte[] {0, (byte) 255, 1});
                stderr.write(new byte[] {(byte) 254, 0});
                stdout.write(new byte[] {2});
                yield 0;
            }
            case "read_file" -> {
                Files.copy(Path.of(argv.get(1)), stdout);
                yield 0;
            }
            case "fail" -> 17;
            default -> 0;
        };
    }
}
