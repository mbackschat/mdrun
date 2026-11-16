package io.github.mbackschat.mdrun.sdk;

import java.util.List;

/** Supplies the application policy and cooperative invocation lifecycle for one worker. */
public interface Application {
    /** Classifies immutable arguments without application effects, output, or input reads. */
    Classification classify(List<String> argv) throws Exception;

    /** Runs one invocation and returns an ordinary application status from 0 through 255. */
    int invoke(Invocation invocation) throws Exception;

    /** Attests that application-owned work and resources permit another invocation. */
    void reset(Invocation invocation) throws Exception;

    /** Releases the adapter after serving ends; successful worker shutdown waits for this method. */
    default void close() throws Exception {}
}
