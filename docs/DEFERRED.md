# Deferred scope

| Item | Reopening condition |
| --- | --- |
| Native Windows qualification | Owner activates it; qualify native shell/context behavior and descendant ownership on Windows before claiming support |
| Custody of ordinary shell-created process groups and daemonized descendants | A concrete consumer requires it and a qualified platform mechanism can retain ownership without silently changing terminal behavior; current managed commands must keep descendants in assigned groups |
| Multiple accelerated bindings in one session | A concrete consumer requires them after single-binding lifecycle and identity guarantees are established |
| Machine-global daemon or shared worker pool | Measured session-scoped startup cost justifies the added discovery, ownership and isolation contract |
| Runtimes beyond JVM, Node and Python | A cooperative application and parity/lifecycle qualification justify an additional SDK |
| Explicit command names | Authors need stable references beyond file/block positions; prefer document-local names qualified by file, while retaining unnamed blocks |
| Tag metadata | A concrete grouping need exceeds file/directory selection; begin with document-level tags, without implicit execution order or dependencies |
| Interactive picker | Human discovery needs exceed searchable text/JSON inventory and preview; deterministic CLI operation remains available |
| Block-level execution or verification | A demonstrated authoring need justifies partial replay; require explicit setup expectations and clearly label partial verification, without inferred prefix execution or a dependency engine |
| Product warm/cold comparison command | The M6 qualification harness proves useful enough to expose as a product interface with repeatable workloads and explicit fixture reset |

Bundled runtimes, runtime installation, a package manager, a plugin marketplace and transparent acceleration of arbitrary unqualified CLIs are outside the accepted initial product. They are not implicit backlog commitments.
