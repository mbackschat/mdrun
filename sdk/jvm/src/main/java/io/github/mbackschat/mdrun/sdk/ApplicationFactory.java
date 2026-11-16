package io.github.mbackschat.mdrun.sdk;

/** Initializes an adapter after the worker's persistent SDK threads exist. */
@FunctionalInterface
public interface ApplicationFactory {
    Application create() throws Exception;
}
