# Execution implementation and qualification

## M3: host and SDK vertical slice

The reviewed [proposed session contract](session-protocol.md) supplies the initial CLI, binding, wire and custody design. It remains unqualified: implement and test its proof obligations before freezing SDK APIs. Host-owned context capture keeps generic environment work out of consumer harnesses. An ambiguous disconnect after dispatch must fail without retry even when the acceptance response was lost.

The experimental native host and [Node SDK](../../sdk/node/README.md) now have local macOS controls for sequential reuse, refusal followed by independent cold execution, changed-environment routing, explicit cold, bound document verification, multi-megabyte binary streams, unused open stdin, lost acceptance without duplicated effects, and idle workers beyond the startup timeout. Private native transport preserves OS-string cwd/argv/environment bytes and transfers original descriptors to custody. Public and hidden CLI options use clap; forwarded application flags are uninterpreted. These results advance the checklist without completing M3 or freezing the SDK.

Independent HIGH review found and guarded native cwd serialization loss, cancellation masked by exit zero or observed before launch, final-release admission ordering, and late retained SDK operations after orderly shutdown. Native guards also exposed macOS socket descriptors returning a different `tcgetpgrp` error from pipes; terminal classification now uses `isatty`. The Node fixture explicitly supplies the selected runtime's observed macOS text-encoding environment value; the host does not ignore that variable or other context differences. Ordinary shell-wrapper context changes still need consumer parity evidence.

Local checkpoint on 2026-11-16, macOS 27.2.1 arm64, Rust 1.98.0 and Node 26.10.0: 58 Rust unit tests and 40 ordinary integration tests pass; six additional Rust-host/Node integration controls pass when explicitly enabled; the independent Node fake-host suite passes 33 tests. Formatting, Clippy and release compilation pass, and the release executable passes all 75 retained Showboat behavioral differential cases. Its SHA-256 is `76677ea4cc8067e240e1f1496162e56421703cc4dab6e9920c4a1c60a2007261`. These counts include test fixture entry points and do not establish every outstanding proof obligation.

A subsequent actual-host cold-session control passes locally: kill only the host while a payload descendant survives, observe the declared TCP listener still excluded, then require descendant exit and certified group cleanup before release. A direct-owner negative control detects the early-release behavior this design prevents. Its two focused test entries include one fixture entry point; bounded self-expiry and direct `Child` handles avoid teardown signals to remembered PIDs/groups. Sampling establishes observed ordering, not continuous-time proof; hosted qualification remains required.

- [ ] Define a versioned binding manifest: executable name, prepared worker argv, absolute packaged-cold launcher, protocol/capabilities, immutable artifact identity, fixed startup cwd/environment and external resource identities. Validate before payload startup; build preparation remains external.
- [ ] Specify readiness, authentication, side-effect-free classification, refusal, acceptance, invocation, completion acknowledgement and shutdown. Reject incompatible versions, stale identities, malformed frames and overlapping calls. Frame limits bound individual messages, not total streams.
- [ ] Implement a private session directory and native forwarding entry point for one explicit PATH binding. Protect credentials; keep them out of documents, logs and reports. Do not enroll unrelated commands or alter shell startup files.
- [ ] Provide one sequential worker per document and an outer managed author command spanning multiple CLI processes. Keep plain Showboat invocations and shell blocks usable. Explicit cold starts no warm machinery and cannot rediscover the forwarding client.
- [ ] Transport bytes with bounded backpressure and pull-based stdin. Preserve stdout/stderr descriptor behavior, EOF, flush and client acknowledgement before releasing admission. Test unread/open stdin and multi-megabyte input/output; never buffer a whole request as the stdin contract.
- [ ] Compare context before acceptance: cwd/PWD, environment, umask, terminal descriptors, native encoding and runtime startup options. Unsupported changes select the original launcher with original context/descriptors. Document only narrowly justified wrapper normalization.
- [ ] Distinguish application failure from infrastructure/cleanup failure. Refused calls may take cold; accepted calls never retry. Use a durable-effects negative control and make poisoned sessions fail even when a shell masks an individual status.
- [ ] Establish POSIX ownership before payload launch. Handle cancellation, deadlines, worker/control/owner loss and descendant exit. Observe cleanup rather than inferring it from closed pipes or the direct child's exit. Retain external resource exclusion through cleanup and parent loss; document cooperative-process escape and host-failure limits.
- [ ] Publish versioned reports with application/infrastructure/cleanup results, route reasons/counts, starts, artifact/tool identities and preparation/replay/total durations, separate from captured output.

## M4/M5: SDKs and scheduling

- [ ] Node SDK: load an explicit handler once and await sequential invocations. Keep loaded code immutable; reread mutable input/configuration. Qualify timers, listeners, unresolved work, children, failures and late output. Module reload or a VM context is not a general reset strategy.
- [ ] Python SDK: load an explicit factory with fresh request-owned arguments/streams. Ordinary failures return status; uncaught exceptions and invalid completion poison reuse. Qualify threads, process exits and native-extension limits; do not promise automatic reuse of arbitrary scripts.
- [ ] Run shared conformance through every SDK without runtime-specific host branches. Compare independently launched packaged cold, assert actual routing/reuse, and check changed inputs plus failure-then-success sequences.
- [ ] Bound parallel independent document sessions. Stop admission on failure, drain launched peers and aggregate reports; keep worker calls sequential and preserve shared-resource exclusions.
- [ ] Provide a minimal application-adapter example and SDK guide. Keep protocol, relay, supervision, scheduling and report production out of consumer code.

## M6: complete-product qualification

- [ ] Run document differential acceptance and runtime/lifecycle conformance on macOS/Linux; pin supported OS/architecture/runtime versions and MSRV from actual runs.
- [ ] Run standalone archive and Homebrew/Linux package install/remove checks. Audit dependency licenses, record binary/linkage sizes, choose a measured size budget and include required notices.
- [ ] Measure complete repeated sessions against independently packaged cold with equal artifacts/context/workload/reset fixtures. Include native clients, startup/shutdown and cleanup; report preparation separately, routing and remaining cold classes. Do not inherit prototype speedup claims.
- [ ] Document commands/options, binding/report schemas, SDKs, failure behavior, examples, installation and limitations. Keep help, README and agent entry points aligned.
- [ ] Publish a qualified release and Homebrew formula after distribution gates pass; verify downloads and installation. Public GitHub, MIT, Homebrew and Linux distribution are authorized.

## M7: consumer readiness

- [ ] Provide a concrete minimal adapter and migration instructions in mdrun. Preserve only build/configuration, application invocation/eligibility/reset and repository policy in the target architecture.
