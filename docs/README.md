# Documentation

Start with the [README walkthrough](../README.md) to turn a shell command into a Markdown example you can replay. Use the guides below when you want to record longer demonstrations, reuse an application's startup or integrate a project with mdrun.

## Write and check demonstrations

| Task | Read |
| --- | --- |
| Write Markdown by hand, capture output or update a recording | [Document guide](guides/documents.md) |
| Inspect a complete recorded example | [Greeting](examples/greeting.md) |
| Find command syntax, exit statuses, paths and environment behavior | [Command reference](guides/reference.md) |
| Read guides without a network connection | [Offline guides](guides/README.md) |

## Reuse application startup

An ordinary CLI invocation starts a new process every time. A managed session can keep an application worker alive across calls. Your application supplies an adapter; mdrun handles command forwarding and process management.

| Task | Read |
| --- | --- |
| Compare ordinary mdrun verification with warm Python reuse | [Runnable Python demo](examples/warm-python.md) |
| Connect an application, run a session or verify documents in parallel | [Session guide](guides/sessions.md) |
| Write a Python worker adapter | [Python SDK](../sdk/python/README.md) |
| Write a Node worker adapter | [Node SDK](../sdk/node/README.md) |
| Write a Java 21 worker adapter | [JVM SDK](../sdk/jvm/README.md) |
| Adapt a JVM application that writes through `Appendable` | [JVM character output example](../sdk/jvm/examples/README.md) |
| Pin SDK artifacts for local development and CI | [SDK distribution](sdk-distribution.md) |
| Understand where reuse saves time and reproduce the measurements | [Performance](performance.md) |

## Understand, build and maintain mdrun

| Task | Read |
| --- | --- |
| Understand the document engine, workers, process ownership and batch execution | [Architecture](architecture.md) |
| Check supported behavior and design boundaries | [Product contract](PRODUCT.md) |
| Consult the version-1 session protocol and ownership guarantees | [Session contract](plans/session-protocol.md) |
| Build and qualify native packages, SDK artifacts and Homebrew delivery | [Distribution](distribution.md) |
| Understand dependency choices | [Dependencies](dependencies.md) |
| Apply dependency quarantine and security-update rules | [Security](security.md) |
| Contribute changes as a coding agent | [Contributor instructions](../CLAUDE.md) |

## Follow project status and evidence

Version [0.1.0](https://github.com/mbackschat/mdrun/releases/tag/v0.1.0) is published. Document commands, discovery, cooperative execution, native packages and SDK assets pass its exact-tag macOS/Linux workflow; public-download, Homebrew and SDK consumers also pass within the recorded platform limits. The public session and SDK version-1 contract is frozen; the plan identifies the release revision and evidence.

| Question | Read |
| --- | --- |
| What is complete, and what remains? | [Implementation plan](PLAN.md) |
| Where are the detailed implementation and qualification records? | [Active plans](plans/README.md) |
| Which features are deliberately deferred? | [Deferred work](DEFERRED.md) |
| Which source material informed the design? | [Research provenance](research/README.md) |
