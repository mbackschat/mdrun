# Implementation plan

Status: active, 2026-11-16. Repository setup, local checks, hosted macOS/Linux CI and the three-target packaging dry run passed. The production milestones below remain open. Follow [PRODUCT](PRODUCT.md) for accepted scope and [extracted research](research/README.md) for provenance.

## Ordered milestones

| Milestone | Scope | Completion evidence |
| --- | --- | --- |
| M1 — Freeze compatibility | Select and verify the Showboat pin; enumerate syntax, errors, files, metadata and remote behavior; assess CLI parsing dependency | [Compatibility matrix](plans/compatibility.md) expanded into executable differential cases against an exact upstream build; intentional differences isolated |
| M2 — Document CLI | Implement all pinned commands and options, source-preserving edits, cold execution, images, replay/diffs, extraction and remote streaming | Every M1 case passes; originals remain unchanged on verify/copy; no external Showboat requirement in product operation |
| M2a — Discovery | Read-only command inventory, plain-text filtering, JSON and single-block preview using the replay parser | Existing unnamed documents are discoverable; preview shows complete source and starts no command or worker; block locators match replay |
| M4 — JVM and Node execution | Native forwarding, managed session scopes, runtime SDKs, bounded scheduling, resource custody and cold routing | Independent packaged-cold parity, actual warm-route assertions, no-retry effects guard, lazy/large-stream tests, state/resource and owner-loss controls on macOS/Linux |
| M5 — Python extension | Implement a cooperative Python SDK/application boundary | Same host contract and parity/lifecycle suite, with explicit Python state/resource limits; no runtime-specific host branch |
| M6 — Product qualification | Pin platform matrix, qualify distribution, audit dependencies/licenses and measure complete-session performance | Clean standalone installs, recorded binary/linkage sizes, repeatable warm/cold ratios with route counts and preparation/replay separated |

## Immediate work: M1

- Confirm the proposed v0.6.1/commit baseline against upstream source and the consumer pin, then record the exact executable/toolchain identity.
- Enumerate all command forms, option positions, stdin/exit/error semantics and filesystem/HTTP effects; implement the oracle harness before product command code.
- Evaluate `clap` or another parsing approach against this inventory. Choose the smallest solution that preserves required behavior; do not inherit LOCmetry's dependency graph or assume parser defaults match upstream.
- Establish timestamp/UUID/branding normalization narrowly; preserve legacy metadata exactly. Select dependencies for demonstrated needs only.
- Turn each matrix row into discriminating tests, including negative controls, and keep production integration tests in one binary.

## Decisions before their gate

| Decision | Gate | Evidence to use |
| --- | --- | --- |
| Production protocol and public fast/cold/session configuration | M3 | Lazy-stream, ownership, consumer and unrelated-application controls; `spike/1` is experimental |
| Upstream attribution obligations | Before reusing externally licensed code | Owner selected MIT for mdrun on 2026-11-16; retain Showboat credit and any license notices required by subsequent source reuse |
| OS/architecture versions, MSRV and binary-size budget | M6 | Native qualification and representative full-product measurements |
| Release readiness and tap publication | M6 | Owner selected public GitHub, Homebrew and Linux packages; exercise the [distribution gates](distribution.md) before publishing a product version |

## Working discipline

Use red/green implementation and finite oracles. Review protocol/lifecycle and final compatibility claims adversarially before accepting their milestones. Keep qualification results tied to revisions and artifacts; a historical pass or configured workflow does not satisfy a new gate. Save new evidence separately from source and time comparisons alone on a near-idle host.

## Concrete lessons from related implementations

Source and feature inspection on 2026-11-16 used Runme `18a321dfdaa8f4e1eba33b28e7bf50460d9ad597` and speedrunner `9abaf508c3e039c89d1bb43542189a7dcec086c2`. No code from those projects was executed or copied. Keep lessons that improve a concrete user workflow, help fulfill an existing requirement, or prevent a plausible failure in the chosen design. The owner accepted the recommendations on 2026-11-16: the small inventory/preview becomes M2a, existing session/reporting requirements remain, and optional extensions stay deferred. CLI spelling is undecided. [PRODUCT](PRODUCT.md) owns accepted behavior.

| Useful feature | Benefit to mdrun | Recommendation and scope | Implemented precedent |
| --- | --- | --- | --- |
| Searchable command inventory and preview | People and coding agents can discover executable blocks, see their source file and inspect code without running it. Machine-readable inventory avoids each consumer parsing Markdown itself. | Accepted as M2a: list recognized blocks, plain-text search, JSON and preview one block. Include existing unnamed Showboat blocks without requiring a corpus rewrite. Keep inventory/preview read-only. | Runme [list with search and JSON output](https://github.com/runmedev/runme/blob/18a321dfdaa8f4e1eba33b28e7bf50460d9ad597/cmd/list.go), [print with raw output](https://github.com/runmedev/runme/blob/18a321dfdaa8f4e1eba33b28e7bf50460d9ad597/cmd/print.go) |
| Selected block or group execution | Shorten authoring/debugging cycles by rerunning a chosen part of a long document. | Whole-document selection first. Defer block selection, names and tags until a demonstrated need; earlier blocks may prepare fixtures. Any future partial verification must be labeled partial, with explicit setup expectations and no inferred dependency engine. | Runme [run selection and tags](https://github.com/runmedev/runme/blob/18a321dfdaa8f4e1eba33b28e7bf50460d9ad597/cmd/run.go) |
| Understandable acceleration status and reasons | Explain whether a tool is warm, starting or unavailable, and why a particular call used cold execution; expose a require-warm option for workflows that depend on acceleration. | Route reasons/counts and warm assertions are already required. Adapt the user-facing explanations and startup feedback without introducing a separate global status service. | speedrunner [status](https://github.com/anthonyGosme/speedrunner/blob/9abaf508c3e039c89d1bb43542189a7dcec086c2/docs/GUIDE.md#other-commands), [fallback explanation and require-fast](https://github.com/anthonyGosme/speedrunner/blob/9abaf508c3e039c89d1bb43542189a7dcec086c2/docs/GUIDE.md#why-did-a-call-run-natively) |
| Local warm/cold comparison | Let a consumer measure whether acceleration helps its own workload, with output agreement and actual routing visible alongside timings. | Useful optional interface over the existing M6 measurement work. Start with the qualification harness; add a product command only if it provides value beyond that. Unlike speedrunner's fixed version probes, document replay needs an explicitly selected repeatable workload and fixture reset. | speedrunner [demo implementation](https://github.com/anthonyGosme/speedrunner/blob/9abaf508c3e039c89d1bb43542189a7dcec086c2/client/src/main.rs#L162-L267) |

Runme's notebook/editor UI and interactive picker are useful for human-led runbooks, but are not needed for this CLI's initial migration. speedrunner's shell typing hooks, speculative warm-up and long-lived per-user daemons solve a different usage pattern; session startup can prepare our known binding without those features. Do not add a new dependency or subsystem merely to reproduce another tool's interface.

Retain the following engineering lessons because they directly affect existing requirements. They refine how those requirements are implemented and checked, without creating separate milestones.

| Material lesson | Why it matters here and proportionate application | Source |
| --- | --- | --- |
| Preserve recordings with minimal edits | Existing Showboat documents must migrate unchanged. Use targeted document edits and a small set of byte-preservation fixtures; do not introduce a general Markdown formatter or alter shell behavior. | Runme [round-trip tests](https://github.com/runmedev/runme/blob/18a321dfdaa8f4e1eba33b28e7bf50460d9ad597/document/editor/editor_test.go#L431-L495) |

Prefer simple prevention before custom handling. For example, speedrunner restores SIGPIPE before its raw native exec, whereas Rust 1.98's standard process-launch implementation [already restores the default disposition](https://github.com/rust-lang/rust/blob/1.98.0/library/std/src/sys/process/unix/unix.rs#L360-L378). This does not prove every inherited-signal case equivalent, but it removes the claimed need for a special reset on ordinary child launches. Resolve lower-level questions against the actual implementation and applicable parity cases; unconfirmed upstream defect suspicions do not add project scope.
