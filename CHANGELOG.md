# Changelog

## Unreleased

- Add experimental native managed sessions, private executable forwarding, independent cold routing, custody/reporting with mdrun package identity, and a cooperative Node SDK; runtime and consumer qualification remains in progress.
- Add an experimental Java 21 SDK with callable application hooks, lazy binary streams, reset/release gating and hash-locked preparation; JVM and consumer qualification remains in progress.
- Allow experimental bindings to declare `SHLVL`/`_` irrelevant to warm-eligible behavior, enabling qualified shell workflows while retaining strict defaults and original cold environments.
- Clean up rejected terminal launches before payload execution without leaving unnecessary custody behind; ambiguous launch/control failures still retain exclusion.
- Restore nested terminal foreground ownership in dependency order after interruption and publish complete custody status snapshots for concurrent readers.
- Add read-only command inventory, literal filtering, JSON output and byte-preserving block preview.
- Implement the seven Showboat-style document commands with clap parsing, cold execution, image copying, replay, extraction and remote delivery.
- Preserve untouched document bytes and legacy provenance; new recordings identify mdrun.
- Establish the mdrun project bootstrap with standalone help/version output and a staged implementation and migration plan.
