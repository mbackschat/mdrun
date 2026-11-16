# Independent document scheduling

Status: implemented and qualified within the cooperative ownership boundary; the public contract below is frozen for 0.1.0 and does not replace single-document `verify`. The native scope/broker interfaces described below remain revision-local implementation details. The [current execution checkpoint](execution.md#current-candidate-checkpoint) records the three-platform runtime evidence and separate publication gates; the [history-cleanup qualification note](../PLAN.md#qualification-after-history-cleanup) applies to earlier results.

## Public contract

```sh
mdrun verify-many --binding binding.json --jobs 6 \
  --share-resources --report batch.json -- first.md second.md
```

`verify-many` accepts one or more document paths, one prepared binding and a positive integer `--jobs` (default `1`). Clap owns argument parsing and usage errors. Existing `--mode`, `--require-warm` and global `--workdir` retain their meanings; document paths resolve from the caller's directory. Each admitted document gets a separate sequential worker and session identity. Exclude updated-document `--output` from this command and reject entry from an existing managed session. This command verifies whole independent documents, without inferring dependencies or selecting blocks.

Every document uses the same binding and resource declarations. `--share-resources` declares that the selected independent documents may execute concurrently while sharing the binding's external exclusions. Without it, any declared resource makes effective concurrency `1`. With sharing enabled, or with no resources, effective concurrency is the smaller of `--jobs` and the document count. The batch holds its declared exclusions for its entire lifetime, including gaps between documents. Report both requested and effective concurrency and the explicit sharing policy. Selecting concurrent documents remains the caller's corpus policy; a shared lease does not establish that arbitrary command side effects are independent.

A scheduling slot covers startup, replay and observed cleanup. The first observed document failure, including an output mismatch, stops new admission; already-started healthy documents finish within their deadlines. Preserve existing verify/session treatment of recorded application nonzero statuses. Cancellation or custody-wide failure stops launched scopes. Do not retry failed documents automatically. Missing reports, uncertain cleanup and documents skipped after admission stops cannot yield batch success.

Diagnostics remain attributable to their document and are presented without interleaving command bytes within each stream. Separate per-document stdout/stderr spool files have no byte quota, persist through batch cleanup and are published in input order; a slow earlier document delays later visible output and concurrent sessions can accumulate temporary disk usage. Publication labels use quoted, escaped native filenames and do not preserve stdout/stderr timing. An optional versioned aggregate report contains overall outcome, wall duration, concurrency/sharing policy and an input-ordered entry for every requested document: completed with its session report, failed without a valid report, or not started because admission stopped. The [session guide](../guides/sessions.md#version-1-reports) documents field meanings. Preserve existing single-session report semantics; keep credentials and captured application bytes out of aggregate metadata.

## Shared custody requirement

Use one authoritative native custodian for the entire batch. Document scopes own their worker, author and cold-process groups; scope completion requires observed cleanup of all its groups but does not release the batch listener. One uncertain scope prevents new admission and retains exclusion. Batch-owner/control loss stops all scopes through the same native custody authority. Release exclusions only after every admitted scope has certified cleanup. Removing resources from child configurations, a scheduler-only listener, or nesting today's independent sessions does not satisfy this contract.

The [native custody contract](session-protocol.md#ownership-and-external-resources) and its cooperative-process limits still apply. Sharing a listener does not weaken per-document invocation admission, context comparison, output delivery or no-retry guarantees.

## Native scope primitive

The first implementation stage added internal custody scopes before the scheduler, host broker and `verify-many` entry point; those layers now use this primitive. Standalone configuration, events and status snapshot fields retain their shapes. The first successful `register_scope` selects scoped mode before any standalone payload launch; mixing modes is a control fault. The configuration's `session_timeout_ms` remains the whole custodian lifetime limit. Each registration supplies positive session and cleanup limits, starts its own session clock, and reserves its scope ID for the rest of the custodian lifetime. Group identity is the typed pair `(scope, id)`; sibling scopes may reuse local names such as `author`.

| Internal request or event | Meaning |
| --- | --- |
| `register_scope {scope, session_timeout_ms, cleanup_timeout_ms}` → `scope_registered {scope}` | Admit a fresh document scope under the already-held listener; never bind a second listener |
| `spawn {scope, id, spec}` / `stop_group {scope, id}` | Address a group within an admitted running scope; descriptors and launch validation retain the standalone contract |
| `scope_event {scope, event}` | Attribute `started`, `exited`, `clean`, `fault`, `uncertain` and `rejected` events to one scope |
| `stop_scope {scope}` → `scope_stopped {scope, clean:true}` | Close scope admission, stop its groups and report exactly one cleanup completion after all groups are certified clean; retain the batch exclusion |
| `scope_event` with `rejected {id, reason:"scope admission closed"}` | Reject a valid late group request for a known stopping/stopped scope without aborting a healthy sibling |
| `scope_rejected {scope, reason:"scope admission closed"}` | Decline fresh registration after a scoped fault; it creates no scope or reservation |
| Global `stop` or owner-control loss | Stop every admitted scope and release the listener only after all owned groups certify cleanup |

A scope deadline or known local payload-launch failure is sticky execution failure: it stops that scope and closes new registration while existing healthy scopes can continue. Cleanup completion does not erase the failure, and the custodian eventually exits nonzero. Duplicate registration, unknown scopes or groups in running scopes, mixed modes and malformed control remain global faults. Group uncertainty stops the batch and retains custody; it cannot produce a successful scope or batch cleanup event. Terminal restoration dependencies remain global and use typed group keys, so cleanup in one scope cannot reap a predecessor still needed by another.

Scope deadlines are checked by the custodian event loop before admitting the next received request; the existing whole-custodian deadline remains in that loop. Synchronous native anchor handoffs and control writes can delay observation; these are not independently scheduled real-time timers. The existing terminal dependency and post-acknowledgement cleanup clocks remain intact. The native scope controls exercise attribution, same-name groups, late-request rejection, sibling continuation, exclusion between scopes, global owner loss and uncertain cleanup; they do not qualify document scheduling or consumer parallelism. Seven new scope entries and ten retained custody entries pass locally; the complete Rust suite, formatting and Clippy pass. Independent review corrected output-readiness races and preserved exact standalone snapshot fields, with red/green guards for that shape and malformed-control exit status. macOS/Linux CI (historical run) and CodeQL (historical run) pass at `b47fe73`.

## Broker and host boundary

The internal broker owns one native custody connection and direct child. Document handles send through a bounded request channel and receive through bounded per-scope event channels. Stop and coordinator-loss flags remain observable independently of request-queue capacity. The broker records native scope and batch cleanup before attempting nonblocking consumer delivery: losing a coordinator stops its scope, while overflow or impossible attribution stops the batch. A native registration rejection remains an unregistered terminal result; it neither aborts an admitted healthy sibling nor requires a scope cleanup acknowledgement. Event delivery cannot substitute for native cleanup certification.

`PreparedSession::from_context` accepts an already-captured native context and absolute document deadline. The caller captures cwd, environment, umask and process group before creating coordinator threads; preparation and execution do not mutate parent cwd, environment or umask. `run_prepared` takes owned author descriptors and either standalone or scoped custody, then returns a `SessionOutcome` containing status, report and diagnostics. The existing standalone wrapper preserves inherited stdio and report/diagnostic publication; the worker startup, invocation coordinator and cold routing state machines use the finite `CustodyAccess` boundary. The batch scheduler supplies per-document spool files and publishes complete diagnostic groups through a custody-owned subprocess. Scope registration derives its positive native timeout from the remaining precomputed document budget.

Native broker qualification is explicitly enabled against a separately built binary; ordinary unit tests do not infer a target directory or execute a libtest binary as the product. The controls exercise two scoped native author coordinators, descriptor separation, interleaved events, coordinator disappearance, shared exclusion and cleanup despite a full consumer queue. Deterministic guards cover registration races, invalid event order and stop flags while the ordinary request queue is full. These controls do not qualify concurrent runtime workers or consumer document scheduling.

```sh
cargo build --locked --bin mdrun
MDRUN_NATIVE_TEST_BIN=/absolute/path/to/mdrun \
  cargo test --locked --bin mdrun session::broker::tests:: -- --include-ignored
```

## Implementation and acceptance tasks

| Task | Required result |
| --- | --- |
| Add the clap command and batch entry point | Preserve single-document verify behavior and its parser guards; validate jobs, paths, binding and output/report combinations before executing documents |
| Separate host coordination from standalone startup | Reuse per-document host logic with captured context, scoped custody and explicit per-document descriptors; capture baseline context before coordinator threads start |
| Add native custody scopes and a broker | Keep one owner of the custodian control connection and direct child; register scopes, attribute groups/events, enforce scope deadlines, stop/complete scopes and certify batch release |
| Implement bounded scheduling | Hold slots through cleanup, stop admission on first observed failure, drain healthy started peers, cancel all scopes on global failure and never retry |
| Aggregate reports and diagnostics | Represent every input document, preserve failures without reports, record actual concurrency/sharing and avoid interleaved output |
| Qualify concurrency and admission | Demonstrate overlapping independent scopes within the configured limit, queued work left unstarted after failure, and a running healthy peer drained afterward |
| Qualify shared exclusion and owner loss | A competing batch cannot acquire the listener, including between scopes; multiple surviving descendants remain excluded after owner loss; one uncertain scope prevents release |
| Qualify shared-resource document scheduling | Use explicit resource-sharing policy and equal artifacts/workloads; assert concurrent documents, one sequential worker per document, route counts and cleanup |

Shared-resource custody qualification is complete at `7ffcf4d`: exclusion, concurrent-scope cleanup, owner loss and actual warm-worker scheduling controls pass on macOS and both Linux architectures in the [current execution checkpoint](execution.md#current-candidate-checkpoint). These results cover explicit sharing and cooperative assigned groups; final artifact checks and publication remain separate gates.
