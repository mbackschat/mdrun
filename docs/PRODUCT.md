# Product contract

**mdrun — Create, run, and verify executable Markdown documents.**

Status: document CLI implementation, 2026-11-16; qualification in progress. The current executable contract is described in the next section. Remaining sections are accepted requirements for implementation, not claims of supported behavior. The useful source conclusions are recorded in [research provenance](research/README.md) and the [execution plan](plans/execution.md).

## Implemented behavior

The Rust executable implements `init`, `note`, `exec`, `image`, `pop`, `verify` and `extract` with ordinary cold process execution. It supports stdin text/code/image inputs, merged stdout/stderr recording and command exit status, working-directory selection, copied images, replay/diffs and updated output, extraction and remote form/multipart delivery. New documents use mdrun provenance; untouched legacy source and metadata are retained. Explicitly choosing the input as `verify --output` updates that file; ordinary verification is read-only.

Clap owns all parsing, help and version output. Normal informational requests exit 0; invalid or missing arguments exit 2; document/verification failures exit 1. `exec` returns the child status (255 for signal termination on qualified Unix behavior). No worker/session/discovery feature is implemented yet. Compiled Rust libraries do not require an installed language runtime or Showboat for host operation; invoked programs still need their own interpreters.

## Required document behavior

Retain Showboat’s seven document commands, familiar options, document format and execution behavior against a pinned release. On 2026-11-16 the owner explicitly replaced exact CLI mimicry with normal library-based parsing: use clap for argument handling, help, version and diagnostics; do not reproduce incidental parser quirks or write a custom option parser. The [compatibility plan](plans/compatibility.md) owns the inventory and baseline qualification. Preserve unmanaged Markdown and legacy provenance; newly authored documents identify mdrun truthfully. Do not rewrite a corpus merely to change branding.

Required commands are `init`, `note`, `exec`, `image`, `pop`, `verify` and `extract`. Required behavior includes stdin inputs, command failures, interpreter selection, working directories, captured output, image copying/alt text, exact replay and diffs, updated-copy output, extraction/quoting, timestamps/IDs, help/version/options and remote streaming. Recording and verification must operate without an installed Showboat executable; upstream is a test oracle only.

## Required discovery behavior

Accepted on 2026-11-16 for implementation after M2: list recognized executable blocks, filter by plain text across file/heading/command text, emit a machine-readable JSON inventory, and preview one block's complete source without executing it. Reuse replay's block recognition and include existing unnamed Showboat blocks. Entries identify the file, block position, available heading, interpreter and command summary; positions locate blocks in the current document and are not persistent IDs. Preview performs no shell evaluation, variable expansion or worker startup. CLI spelling remains undecided.

Selection initially operates on whole documents. Names, tag metadata, interactive pickers and partial block execution are [deferred](DEFERRED.md); discovery does not require rewriting the existing corpus or introducing dependencies between blocks.

## Required execution behavior

Fast execution is the ordinary development route for qualified cooperative bindings; unqualified commands execute normally. Explicit cold execution exercises the actual packaged launcher with original context and descriptors and starts no warm machinery. Session-local native forwarding preserves existing shell blocks, pipes, redirects and executable names. Runtime/application initialization may be reused; command results may not be cached implicitly.

| Boundary | Required guarantee |
| --- | --- |
| Binding | Prepared worker argv, absolute cold launcher, context and immutable artifact identity supplied externally |
| Admission | Side-effect-free classification before acceptance; no stdin consumption during classification; no retry after acceptance |
| Context | Unsupported cwd/PWD, environment, umask, terminal, encoding or startup-option differences select cold before execution |
| Streams | Lazy input, streamed binary output, bounded backpressure and descriptor fidelity; frame bounds are not total-input bounds |
| Sequential reuse | Invocation ownership extends through output flush and client acknowledgement, including cold calls |
| Failures | Application status is distinct from infrastructure/cleanup status; masked transport failures still fail the session |
| Ownership | Cancellation, owner loss, worker failure and descendant exit are observed; uncertain cleanup invalidates reuse and success |
| Resource exclusion | Generic external resource identities remain excluded through observed cleanup, including parent loss |
| Sessions | Sequential worker per document; an outer authoring session may span multiple CLI processes |
| Scheduling | Bounded parallel independent documents; failure stops admission and drains launched peers |
| Reporting | Versioned machine-readable application/infrastructure results, route counts/reasons, worker starts, identities and preparation/replay timing; no credentials or report data in captured command output |

## Platforms and distribution

Target macOS and Linux first. A small standalone native host is required; target runtimes remain external. More than one host process may implement supervision/forwarding. Qualify exact OS/architecture versions and a representative binary-size budget before release. Windows qualification is [deferred](DEFERRED.md).

The process model covers cooperative development commands, not a security sandbox. Deliberately escaped ownership and host/OS failure need explicit documented limits. No global daemon, bundled language runtimes, package manager or plugin marketplace is required.
