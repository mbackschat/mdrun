# Execution implementation and qualification

## M3: host and SDK vertical slice

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
