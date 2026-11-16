# Implementation plan

Status: document CLI, discovery, managed sessions and all three SDKs are implemented. The public version-1 contract is frozen for the 0.1.0 release candidate after independent review of the qualified cooperative execution boundary. The [execution checklist](plans/execution.md) owns final candidate packaging checks and the publication gates held at the owner's request.

Publication boundary, 2026-11-16: the owner requested a stop before release. Complete implementation and non-publishing qualification, then hand back the candidate for review. Do not create a release tag, publish GitHub assets or publish a Homebrew formula until the owner resumes that work. Actual public-download and Homebrew publication checks remain explicit release gates.

## External review remediation

The independent review of the pre-rewrite source and unfinished working tree on 2026-11-16 found the session/batch slice unready. The following remediation outcomes now have discriminating guards and source/runtime qualification; the historical F2 cause remains unknown. Installed-package checks and publication retain separate gates.

| Finding | Required outcome |
| --- | --- |
| F1: batch infrastructure failures lose diagnostics and retain stale reports | Replace stale success before execution; preserve every input and admitted result in failure reports; diagnose custody startup failure and remove workspaces that never acquired custody |
| F2: observed Linux sibling-scope termination | Qualify both invariants with process/group observations: ordinary scope cleanup preserves healthy peers; uncertain cleanup stops all admitted scopes while retaining failed custody. Keep the historical cause unknown unless its missing preceding trace is recovered; repetition alone cannot establish it |
| F3: custody test port handoff race | Prevent overlapping fixture reservations through readiness and distinguish fixture admission failures from product scope failures |
| F4: JVM pending interrupts fault ordinary output | Preserve expected output/status and interrupt semantics with a focused peer regression |
| F5: help and guide contract drift | Verify reason names, exit statuses, scheduling/admission, output labels and report fields against source and executable controls |
| F6: single-session report aliasing and lost diagnostics | Reject input aliases before execution; publish reports atomically and preserve cleanup diagnostics if reporting fails |
| F7: Node protocol discrepancies | Qualify request identity, JSON/Unicode validation, fault reasons and inert empty I/O against the shared contract |
| F8: weak peer-suite oracles | Prove refused calls never invoke applications and active uncredited input fails; align applicable lifecycle controls across SDKs |
| F9: batch output limits | Document disk spool growth and input-ordered publication delays; keep these explicit in the public contract |

The review also identified maintenance risks: duplicated broker/custodian scope transitions, a large batch orchestration closure, unused standalone custody configuration in scoped execution, and unchecked empty process argv. The last two are corrected; retain independent transition validation around the remaining maintenance surfaces. Architecture/SDK guides, versioned SDK packaging, pinned preparation, release qualification gates and toolchain attribution are implemented. Final hosted artifact checks remain pending; registry publication alone is [deferred](DEFERRED.md).

Remediation on 2026-11-16 addresses F1 and F3–F9: failure reporting now runs outside batch orchestration, stale reports are invalidated before admission, shared atomic report publication rejects existing and dangling aliases, failed startup reaps its custodian, and bounded diagnostics cannot hold the parent on an unread pipe. SDK regressions cover interrupts, identity/framing, refusal effects and active uncredited input. The [architecture](architecture.md) and owning guides explain the implemented boundaries and limitations. Standalone custody configuration is now constructed only in standalone execution.

The earlier local remediation checkpoint passed 82 ordinary Rust unit tests, 80 ordinary integration tests, all 29 runtime-gated native integration tests, eight broker controls, Clippy, manual generation/freshness/rendering and package-asset guards. SDK peer suites then passed 51 Node, 19 JVM and 21 Python tests; independent refusal-effect mutations were rejected. The subsequent JVM lifecycle guard brings its peer suite to twenty tests. These counts describe dated checkpoints, not a substitute for exact-revision hosted qualification.

Hosted [macOS/Linux CI](https://github.com/mbackschat/mdrun/actions/runs/43853169936) and [CodeQL](https://github.com/mbackschat/mdrun/actions/runs/43853169945) pass for source revision `9c28a91` on 2026-11-16. Both platform checks and both Showboat differential jobs completed successfully, including the configured runtime, streaming and native broker controls. This qualifies the reviewed source checkpoint; it does not resolve the historical F2 cause, freeze the session API, publish SDK artifacts or complete the remaining release gates.

F2's current-invariant qualification is complete at `7ffcf4d`. Linux investigation passed 120 concurrent scope-isolation attempts with distinct observed process groups; the updated custody suite also passed 20 parallel repetitions. A controlled first-anchor suspension reproduced sibling SIGTERM only after cleanup uncertainty triggered the specified batch-wide shutdown. The strengthened guard observes a live sibling, distinct process groups, sibling termination and retained failed-scope custody; removing the global-stop transition makes it fail. The clean-stop guard separately requires healthy sibling survival. Both guards pass in the hosted runtime matrix below. This proves a legitimate alternative mechanism, not the cause of the earlier observation: its preceding event trace is unavailable. Independent review found no cross-group signal path in the audited anchors. The historical incident remains unexplained, not retrospectively fixed or proven signal misrouting.

The final lifecycle additions explicitly qualify JVM-owned threads and children: counters prove live work before failed reset and while teardown is gated; successful cleanup permits reuse. Disabling reset makes both negative controls fail. Seven custody scope tests and twenty JVM peer tests pass locally and in the hosted runtime matrix, with independent source review. The empty-argv concern has a native regression covering missing/empty executable names with and without a session marker, and normal clap usage errors replace the reproduced panic/forwarding failure.

All three runtime-check jobs in [non-publishing run 43862729035](https://github.com/mbackschat/mdrun/actions/runs/43862729035) pass at `7ffcf4da8be1ef04e1528cdec72f2fac7306b25f`: macOS 15 arm64, Ubuntu 24.04 x86_64 and Ubuntu 24.04 arm64. They cover the native host, broker/batch, terminals, three SDKs and streaming controls. This is runtime-job evidence, not a claim that the whole packaging run passed. The final packaging/wheel changes at `6addb27` still need their non-publishing hosted qualification; runtime behavior is unchanged apart from the embedded inventory guide.

Pre-release work also includes [complete-session measurements](performance.md), the completed semantic and representative [usability review](plans/usability.md), local standalone macOS archive smoke with thirteen rendered manuals, and versioned SDK archives plus a Python wheel with isolated `uv`/actual-host qualification. Final cross-target package checks and hosted wheel consumption remain open. Public downloads and Homebrew publication are held by the owner's stop-before-release instruction.

## Qualification after history cleanup

Earlier qualification results remain as historical narrative tied to their original revision and artifact identities. The associated GitHub Actions records were removed; those results do not qualify the rewritten revision. Fresh macOS/Linux CI, runtime controls and applicable distribution checks must pass on an identified rewritten revision before current hosted qualification is claimed. Historical completion labels below describe the earlier checkpoints.

## Ordered milestones

| Milestone | Scope | Completion evidence |
| --- | --- | --- |
| M1 — Freeze compatibility (complete) | Verified v0.6.1 source/build, finite syntax/files/metadata/HTTP inventory | [Compatibility evidence](plans/compatibility.md#completion): upstream self-comparison, discriminating negative controls, reviewed fixes guarded; cross-platform harness CI passed |
| M2 — Document CLI (complete) | Implement all pinned commands and options, source-preserving edits, cold execution, images, replay/diffs, extraction and remote streaming | Hosted qualification (historical run) at `62c60bb`: macOS/Linux behavioral differential and independent clap/preservation/HTTP controls pass; originals stay unchanged unless explicitly selected as output; no external Showboat requirement in product operation |
| M2a — Discovery (complete) | Read-only command inventory, plain-text filtering, JSON and single-block preview using the replay parser | Hosted qualification (historical run) at `cb75f36`: existing unnamed documents are discoverable; preview shows complete source and starts no command or worker; block locators match replay; native-byte filename preview passes on Linux |
| M3 — Session and SDK vertical slice (frozen candidate) | Specify and exercise binding/context identity, classification/admission, streams, failure states, credentials, reporting and cooperative ownership | Runtime matrix at `7ffcf4d` and independent contract/lifecycle review support the version-1 freeze for 0.1.0; publication remains held |
| M4 — JVM and Node execution (implemented and qualified) | Native forwarding, managed session scopes, runtime SDKs, bounded scheduling, resource custody and cold routing | Packaged-cold parity, route/reuse assertions, no-retry, streaming, owned-resource, shared-exclusion and owner-loss controls pass on all three native runners at `7ffcf4d` |
| M5 — Python extension (implemented and qualified) | Implement a cooperative Python SDK/application boundary | The same runtime matrix passes parity/lifecycle controls with explicit thread, process-exit and native-extension limits; no runtime-specific host branch |
| M6 — Product qualification | Pin platform matrix, qualify distribution, audit dependencies/licenses and measure complete-session performance | Clean standalone installs, recorded binary/linkage sizes, repeatable warm/cold ratios with route counts and preparation/replay separated |

## Document CLI and discovery checkpoint

- [x] Use clap exclusively for the seven commands/options, standard help/version/errors and `--`; independently test ordinary consumer syntax, extra/invalid arguments and no execution on usage errors.
- [x] Implement the source-spanned document parser and init/note/pop/extract, including provenance, binary/text representation and quoting; test canonical bytes and untouched noncanonical source separately.
- [x] Implement cold exec, merged capture, failures/signals, workdir and stdin behavior; verify replay/diffs and output-copy/update semantics.
- [x] Implement image validation/copying/alt/naming and HTTP form/multipart delivery with bounded timeout and nonfatal warnings, including connection failure/timeout cases.
- [x] Define narrow identity projections for extraction and remote warnings; test normal help/version independently without scrubbing arbitrary payloads. Keep preserved-source differences visible and narrowly justified.
- [x] Pass the required behavioral differential corpus on macOS/Linux plus standalone, extraction-reconstruction and source-preservation controls; document behavior and dependency/license choices.
- [x] Implement M2a inventory, plain-text filtering, JSON and read-only single-block preview using the same recognition contract. Names/tags/TUI/partial execution remain deferred.
- [x] Complete hosted M2a qualification.
- [x] Implement, qualify and freeze the session/SDK vertical slice for the 0.1.0 candidate within the [cooperative protocol and custody boundary](plans/session-protocol.md); retain explicit artifact/publication gates in the [execution checklist](plans/execution.md).

## Decisions before their gate

| Decision | Gate | Evidence to use |
| --- | --- | --- |
| Version-1 API freeze review (complete) | 0.1.0 release candidate | Independent review accepted the documented public CLI, binding, wire, reports and SDK APIs on 2026-11-16, backed by three-platform runtime controls at `7ffcf4d`; private native protocols remain revision-local |
| Upstream attribution obligations | Before reusing externally licensed code | Owner selected MIT for mdrun on 2026-11-16; retain Showboat credit and any license notices required by subsequent source reuse |
| OS/architecture versions, MSRV and binary-size budget | M6 | Native qualification and representative full-product measurements |
| Release readiness and tap publication | M6, after the owner resumes publication | Complete the [distribution gates](distribution.md); the current instruction stops before tags, GitHub release assets or Homebrew publication |

## Working discipline

Use red/green implementation and finite oracles. Review protocol/lifecycle and final compatibility claims adversarially before accepting their milestones. Keep qualification results tied to revisions and artifacts; a historical pass or configured workflow does not satisfy a new gate. Save new evidence separately from source and time comparisons alone on a near-idle host.

The document CLI, discovery and cooperative session/SDK implementation have reached their qualified candidate checkpoints, and the version-1 public contract is frozen for 0.1.0. Final artifact checks remain distinct from the completed lifecycle controls; publication awaits the owner's instruction to resume. The [execution checklist](plans/execution.md) owns those gates. [Deferred work](DEFERRED.md) stays outside active milestones until its reopening condition is met.

## Concrete lessons from related implementations

Source and feature inspection on 2026-11-16 used Runme `18a321dfdaa8f4e1eba33b28e7bf50460d9ad597` and speedrunner `9abaf508c3e039c89d1bb43542189a7dcec086c2`. No code from those projects was executed or copied. Keep lessons that improve a concrete user workflow, help fulfill an existing requirement, or prevent a plausible failure in the chosen design. The owner accepted the recommendations on 2026-11-16: the small inventory/preview becomes M2a (`list` and `preview`), existing session/reporting requirements remain, and optional extensions stay deferred. [PRODUCT](PRODUCT.md) owns accepted behavior.

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
