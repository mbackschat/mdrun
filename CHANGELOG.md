# Changelog

## Unreleased

- Handle Linux procfs records for already-dead processes with detached process-group IDs without falsely retaining custody; retain the first cleanup-uncertainty reason in private status snapshots.
- Add versioned JVM, Node and Python SDK archives and a standard Python wheel for checksum-pinned GitHub release consumption, including isolated `uv` installation.
- Report ordinary CLI usage errors when native launchers omit or empty the executable argument, including inside a managed session.
- Add experimental `verify-many` with bounded document admission, shared resource custody, ordered output and aggregate reports; platform/lifecycle qualification remains open.
- Preserve batch failure reports and bounded diagnostics, reject report aliases before execution, and publish session reports atomically.
- Preserve JVM invocation interrupt semantics and strengthen Node framing, request identity and inert empty-output handling.
- Add focused offline guides, actionable command help and generated manual pages, with an executed first-use recording.
- Document version 1 session/batch report fields, exact cold reasons, signal/forwarding statuses and batch output/concurrency limits.
- Add experimental native managed sessions, private executable forwarding, independent cold routing, custody/reporting with mdrun package identity, and a cooperative Node SDK; runtime and consumer qualification remains in progress.
- Add an experimental Java 21 SDK with callable application hooks, lazy binary streams, reset/release gating, sticky invocation invalidation, hash-locked preparation and an application character-output example; JVM and consumer qualification remains in progress.
- Add an experimental dependency-free Python SDK with explicit application hooks, lazy binary streams, cooperative cancellation and sticky failure; native-host parity and protocol controls qualify the initial boundary.
- Allow experimental bindings to declare `SHLVL`/`_` irrelevant to warm-eligible behavior, enabling qualified shell workflows while retaining strict defaults and original cold environments.
- Clean up rejected terminal launches before payload execution without leaving unnecessary custody behind; ambiguous launch/control failures still retain exclusion.
- Restore nested terminal foreground ownership in dependency order after interruption and publish complete custody status snapshots for concurrent readers.
- Add read-only command inventory, literal filtering, JSON output and byte-preserving block preview.
- Implement the seven Showboat-style document commands with clap parsing, cold execution, image copying, replay, extraction and remote delivery.
- Preserve untouched document bytes and legacy provenance; new recordings identify mdrun.
- Establish the mdrun project bootstrap with standalone help/version output and a staged implementation and migration plan.
