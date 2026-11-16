package io.github.mbackschat.mdrun.consumer;

import java.util.List;

public final class ColdMain {
    private ColdMain() {}

    public static void main(String[] argv) throws Exception {
        System.exit(ReadFile.run(List.of(argv), System.out));
    }
}
