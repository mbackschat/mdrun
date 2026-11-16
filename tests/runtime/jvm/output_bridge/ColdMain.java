package io.github.mbackschat.mdrun.bridgefixture;

public final class ColdMain {
    private ColdMain() {}

    public static void main(String[] args) throws Exception {
        System.exit(Scenario.run(args[0], System.out, System.err));
    }
}
