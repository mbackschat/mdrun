# Implementation plan

Status: active, 2026-11-16. Repository setup is complete once its local bootstrap checks and import-integrity checks pass. The production milestones below remain open. Follow [PRODUCT](PRODUCT.md) for accepted scope and [the preserved handoff](research/showboat-like-tool/docs/09-NEXT-SESSION.md) for provenance.

## Ordered milestones

| Milestone | Scope | Completion evidence |
| --- | --- | --- |
| M1 — Freeze compatibility | Select and verify the Showboat pin; enumerate syntax, errors, files, metadata and remote behavior; assess CLI parsing dependency | [Compatibility matrix](plans/compatibility.md) expanded into executable differential cases against an exact upstream build; intentional differences isolated |
| M2 — Document CLI | Implement all pinned commands and options, source-preserving edits, cold execution, images, replay/diffs, extraction and remote streaming | Every M1 case passes; originals remain unchanged on verify/copy; no external Showboat requirement in product operation |
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

Use red/green implementation and finite oracles. Review protocol/lifecycle and final compatibility claims adversarially before accepting their milestones. Keep qualification results tied to revisions and artifacts; a historical pass or configured workflow does not satisfy a new gate. Save new evidence outside the immutable snapshot and time comparisons alone on a near-idle host.

Before freezing the SDK, cover frozen load-time state, randomness, rotated/open files, controlling terminals, inherited descriptors and environment retention in negative controls. These are concrete prior-art limits documented by [speedrunner](https://github.com/anthonyGosme/speedrunner/blob/9abaf508c3e039c89d1bb43542189a7dcec086c2/docs/GUIDE.md#known-limits). Its native warm forwarding is relevant precedent, but it [excludes JVM acceleration](https://github.com/anthonyGosme/speedrunner/blob/9abaf508c3e039c89d1bb43542189a7dcec086c2/docs/GUIDE.md#java-and-the-jvm). Do not claim novelty for native distribution or persistent runtimes individually, or superior size/performance without measurement.
