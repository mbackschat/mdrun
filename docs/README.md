# Documentation

Document CLI, read-only discovery, cooperative managed execution, native packages and SDK assets have current macOS/Linux pre-release qualification. The public session and SDK version-1 contract is frozen for the 0.1.0 release candidate; publication is held. The [implementation plan](PLAN.md) identifies exact-revision evidence and remaining public-download/Homebrew gates. Current documents below govern work; the research index maps the extracted conclusions to their maintained owners.

| Document | Owns |
| --- | --- |
| [User guides](guides/README.md) | Offline-compatible document workflows, session setup and command reference |
| [Recorded greeting](examples/greeting.md) | Executed README quick start with replayable output |
| [Warm Python demo](examples/warm-python.md) | Runnable adapter, changed-input recording, cold comparison and parallel document replay |
| [Product contract](PRODUCT.md) | Current behavior, accepted goals and architectural boundaries |
| [Architecture](architecture.md) | Native document/session components, runtime adapter boundary, custody, batch output and evidence limits |
| [Implementation plan](PLAN.md) | Milestone ordering, open decisions and acceptance gates |
| [Active plans](plans/README.md) | Detailed compatibility and consumer qualification |
| [Version-1 session contract](plans/session-protocol.md) | Frozen public candidate contract, cooperative ownership boundary and qualification invariants |
| [Node SDK](../sdk/node/README.md) | Version-1 cooperative worker API, examples and resource limits |
| [JVM SDK](../sdk/jvm/README.md) | Version-1 Java 21 worker API, preparation and application reset contract |
| [JVM character output example](../sdk/jvm/examples/README.md) | Request-owned Appendable sinks, charset parity and explicit lifetime limits |
| [Python SDK](../sdk/python/README.md) | Version-1 cooperative Python application API, binary streams and cancellation limits |
| [Deferred work](DEFERRED.md) | Unscheduled scope and reopening conditions |
| [Distribution](distribution.md) | Release targets, artifacts, Homebrew and Linux package qualification |
| [Performance](performance.md) | Measured complete-session cold/warm comparisons, artifact identities and reproduction |
| [SDK release consumption](sdk-distribution.md) | Version- and checksum-pinned GitHub SDK artifacts for local development and CI |
| [Dependency choices](dependencies.md) | Reasons for compiled libraries and qualification policy |
| [Security](security.md) | Dependency quarantine, security-fix exceptions and GitHub scanning |
| [Research](research/README.md) | Extracted conclusions and source provenance |

Contributor instructions live in [CLAUDE.md](../CLAUDE.md); user entry points live in [README.md](../README.md). Avoid duplicating detailed contracts across navigation documents.
