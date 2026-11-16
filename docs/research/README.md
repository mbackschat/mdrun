# Research retained for implementation

The supplied `showboat-like-tool` research informed mdrun's requirements. Only the useful conclusions were extracted into maintained documents; its prototype source, raw measurements, copied plans, hidden configuration and Git history are not part of this repository. The owner requested removal of the bulk import, including from Git history, on 2026-11-16. The original Downloads folder is no longer needed.

| Retained knowledge | Owner |
| --- | --- |
| Rust host, full Showboat CLI compatibility, truthful identity, runtime order and standalone distribution | [Product contract](../PRODUCT.md) |
| Host/SDK/application responsibilities, stream and lifecycle requirements, qualification tasks | [Execution plan](../plans/execution.md) |
| Pinned Showboat behavior and differential verification | [Compatibility plan](../plans/compatibility.md) |
| Useful Runme/speedrunner features and focused engineering lessons | [Implementation plan](../PLAN.md#concrete-lessons-from-related-implementations) |

No prototype size or speedup is a product claim. Measurements will be made against the implemented tool and identified application artifacts. Keeping the historical benchmark corpus is unnecessary for those new measurements.

[Showboat](https://github.com/simonw/showboat/tree/d531261b8faf0c388b02c7891d50f1f47c3e2b52), by Simon Willison and contributors, supplies the original idea and compatibility reference. LOCmetry revision `d96eb6369484c1e8f60e65d1b65a41b64485629b` supplied repository organization and distribution conventions only; its dependencies and product architecture were not copied.
