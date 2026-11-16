package io.github.mbackschat.mdrun.fixture;

import java.util.List;

public final class ColdMain {
    private ColdMain() {}

    public static void main(String[] args) throws Exception {
        System.exit(Example.run(List.of(args), System.in, System.out, System.err));
    }
}
