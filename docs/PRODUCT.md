# Product contract

mdrun turns Markdown examples into repeatable checks and lets cooperative applications reuse startup across calls. This document defines the behavior users and integrations can rely on. For a working first example, use the [README](../README.md); for component design, use the [architecture guide](architecture.md).

The 0.1.0 candidate's document CLI, discovery, managed execution, native packages and SDK artifacts have macOS/Linux pre-release qualification. The public version-1 session and SDK contract is frozen; publication remains held. The [implementation plan](PLAN.md) owns exact-revision evidence and remaining publication gates. Qualification of mdrun does not establish safe reuse or cold/warm parity for an arbitrary application.

## Implemented behavior

Ordinary execution starts a process for each command. A managed session can instead forward eligible calls to an application worker that remains alive. The host is a standalone Rust executable; invoked commands and workers still need their own interpreters and runtimes.

| Task | Commands and behavior |
| --- | --- |
| Author a demonstration | `init`, `note`, `exec`, `image` and `pop` create and edit Markdown, capture combined stdout/stderr, include copied images and undo the last entry. Text, code and image inputs can come from stdin. |
| Check or update results | `verify` replays commands and compares captures. Ordinary verification leaves the input document unchanged; `--output` writes updated captures, including in place when the input path is explicitly selected. |
| Recreate authoring steps | `extract` emits shell commands that reconstruct a document. |
| Inspect commands | `list` inventories executable blocks; `preview` returns original fenced source without execution. Their precise contracts are below. |
| Reuse application startup | Native sessions and the Node, JVM and Python SDKs implement the [version-1 execution contract](plans/session-protocol.md). Application adapters establish eligibility and cleanup. |
| Read help offline | `guide` embeds maintained task guides; generated man pages and CLI help share the public clap definitions. |
| Deliver recordings remotely | Configured remote delivery uses form/multipart requests. The [reference](guides/reference.md) owns delivery and failure behavior. |

New documents identify mdrun. Editing preserves untouched legacy source and metadata. Working-directory selection, command exit statuses, image alt text and extraction quoting are part of the document contract, detailed below and in the [document guide](guides/documents.md).

Clap owns parsing, help and version output. Informational requests exit 0; invalid or missing arguments exit 2; document/verification failures exit 1. `exec` returns the child status, with 255 for signal termination on qualified Unix behavior. No installed Showboat or language runtime is required to start the host; its compiled Rust dependencies supply the host functionality.

## Required document behavior

Retain Showboat’s seven document commands, familiar options, document format and execution behavior against a pinned release. On 2026-11-16 the owner explicitly replaced exact CLI mimicry with normal library-based parsing: use clap for argument handling, help, version and diagnostics; do not reproduce incidental parser quirks or write a custom option parser. The [compatibility plan](plans/compatibility.md) owns the inventory and baseline qualification. Preserve unmanaged Markdown and legacy provenance; newly authored documents identify mdrun truthfully. Do not rewrite a corpus merely to change branding.

Required commands are `init`, `note`, `exec`, `image`, `pop`, `verify` and `extract`. Required behavior includes stdin inputs, command failures, interpreter selection, working directories, captured output, image copying/alt text, exact replay and diffs, updated-copy output, extraction/quoting, timestamps/IDs, help/version/options and remote streaming. Recording and verification must operate without an installed Showboat executable; upstream is a test oracle only.

## Discovery behavior

`list FILE... [--filter TEXT] [--json]` lists replay-recognized executable blocks in file/block order, including existing unnamed Showboat commands and commands without recorded output. It excludes image and output blocks. Entries identify the file, zero-based parsed-block index, one-based opening-fence line, nearest preceding title/ATX heading, interpreter and first command line. The filter is a case-sensitive literal substring across filename, heading and complete command text. Positions locate blocks in the current document and are not persistent IDs. Setext headings are not interpreted.

JSON is an array of objects with `file`, `block`, `line`, `heading`, `interpreter` and `summary`; heading is null when absent. Inventory requires UTF-8 paths, headings, interpreters and command text and reports invalid text without emitting a partial inventory. Plain output escapes control characters so each entry occupies one row. `preview FILE BLOCK` emits the complete original fenced source span, preserving arbitrary bytes and line endings, and rejects missing or non-executable blocks. It uses replay's parser and performs no shell evaluation, variable expansion, stdin consumption or worker startup.

Selection initially operates on whole documents. Names, tag metadata, interactive pickers and partial block execution are [deferred](DEFERRED.md); discovery does not require rewriting the existing corpus or introducing dependencies between blocks.

## Required execution behavior

The host is generic Rust; JVM, Node and Python SDKs implement the external versioned protocol, and application adapters qualify callable entry points and reset/resource behavior. Each session has one accelerated binding. Independent application fixtures exercise the shared boundary; application-specific build, eligibility and reset policies remain outside the host.

Fast execution is the ordinary development route for qualified cooperative bindings; unqualified commands execute normally. Explicit cold execution exercises the actual packaged launcher with original context and descriptors and starts no warm machinery. Session-local native forwarding preserves existing shell blocks, pipes, redirects and executable names. Runtime/application initialization may be reused; command results may not be cached implicitly.

| Boundary | Required guarantee |
| --- | --- |
| Binding | Prepared worker argv, absolute cold launcher and immutable artifact identity supplied externally; fixed startup context captured by the host or explicitly configured |
| Admission | Side-effect-free classification before acceptance; no stdin consumption during classification; no retry after acceptance |
| Context | Unsupported cwd/PWD, environment, umask, terminal, encoding or startup-option differences select cold before execution; bindings may explicitly declare qualified `SHLVL`/`_` irrelevance for warm comparison only |
| Streams | Lazy input, streamed binary output, bounded backpressure and descriptor fidelity; frame bounds are not total-input bounds |
| Sequential reuse | Invocation ownership extends through output flush and client acknowledgement, including cold calls |
| Failures | Application status is distinct from infrastructure/cleanup status; masked transport failures still fail the session |
| Ownership | Cancellation, owner loss, worker failure and descendant exit within assigned process groups are observed; uncertain cleanup invalidates reuse and success |
| Resource exclusion | Generic external resource identities remain excluded through observed cleanup of assigned groups, including parent loss |
| Sessions | At most one sequential worker per document; explicit cold starts none; an outer authoring session may span multiple CLI processes |
| Scheduling | Bounded parallel independent documents; document failure stops admission and drains healthy launched peers; cancellation or custody-wide failure stops all scopes |
| Reporting | Versioned machine-readable application/infrastructure results, route counts/reasons, worker starts, identities and preparation/replay timing; no credentials or report data in captured command output |

## Platforms and distribution

Target macOS and Linux first. A small standalone native host is required; target runtimes remain external. More than one host process may implement supervision/forwarding. Qualify exact OS/architecture versions and a representative binary-size budget before release. Windows qualification is [deferred](DEFERRED.md).

Managed scripts and applications must keep descendants in their assigned process group and stop/await background jobs for successful orderly completion. Shell monitor mode (`set -m`), independently created process groups and daemonization are unsupported by the portable backend. These can be ordinary application features; the limitation does not depend on hostile intent. The host cleans and observes assigned groups, cannot reliably detect every group departure, and does not contain arbitrary descendant trees. Outside this cooperation boundary, a descendant may survive a successful report and released exclusion. The owner accepted this boundary on 2026-11-16. Host/OS failure remains outside the guarantee. No global daemon, bundled language runtimes, package manager or plugin marketplace is required.
