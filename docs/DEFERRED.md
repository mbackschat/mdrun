# Deferred scope

| Item | Reopening condition |
| --- | --- |
| Native Windows qualification | Owner activates it; qualify native shell/context behavior and descendant ownership on Windows before claiming support |
| Multiple accelerated bindings in one session | A concrete consumer requires them after single-binding lifecycle and identity guarantees are established |
| Machine-global daemon or shared worker pool | Measured session-scoped startup cost justifies the added discovery, ownership and isolation contract |
| Runtimes beyond JVM, Node and Python | A cooperative application and parity/lifecycle qualification justify an additional SDK |

Bundled runtimes, runtime installation, a package manager, a plugin marketplace and transparent acceleration of arbitrary unqualified CLIs are outside the accepted initial product. They are not implicit backlog commitments.
