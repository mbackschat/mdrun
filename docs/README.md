# Documentation

Document CLI and read-only discovery are implemented; earlier macOS/Linux qualification does not qualify the rewritten revision, as explained in the [qualification note](PLAN.md#qualification-after-history-cleanup). Managed sessions and JVM, Node and Python SDKs are experimental; runtime and release qualification continues. Current documents below govern work; the research index maps the extracted conclusions to their maintained owners.

| Document | Owns |
| --- | --- |
| [User guides](guides/README.md) | Offline-compatible document workflows, session setup and command reference |
| [Recorded greeting](examples/greeting.md) | Executed README quick start with replayable output |
| [Product contract](PRODUCT.md) | Current behavior, accepted goals and architectural boundaries |
| [Architecture](architecture.md) | Native document/session components, runtime adapter boundary, custody, batch output and evidence limits |
| [Implementation plan](PLAN.md) | Milestone ordering, open decisions and acceptance gates |
| [Active plans](plans/README.md) | Detailed compatibility and consumer qualification |
| [Proposed session protocol](plans/session-protocol.md) | Developing native/SDK contract and proof obligations before API freeze |
| [Node SDK](../sdk/node/README.md) | Experimental cooperative worker API, examples and resource limits |
| [JVM SDK](../sdk/jvm/README.md) | Experimental Java 21 worker API, preparation and application reset contract |
| [JVM character output example](../sdk/jvm/examples/README.md) | Request-owned Appendable sinks, charset parity and explicit lifetime limits |
| [Python SDK](../sdk/python/README.md) | Experimental cooperative Python application API, binary streams and cancellation limits |
| [Deferred work](DEFERRED.md) | Unscheduled scope and reopening conditions |
| [Distribution](distribution.md) | Release targets, artifacts, Homebrew and Linux package qualification |
| [Dependency choices](dependencies.md) | Reasons for compiled libraries and qualification policy |
| [Security](security.md) | Dependency quarantine, security-fix exceptions and GitHub scanning |
| [Research](research/README.md) | Extracted conclusions and source provenance |

Contributor instructions live in [CLAUDE.md](../CLAUDE.md); user entry points live in [README.md](../README.md). Avoid duplicating detailed contracts across navigation documents.
