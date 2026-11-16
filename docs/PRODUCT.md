# Product contract

**mdrun — Create, run, and verify executable Markdown documents.**

Status: document CLI and discovery qualified on macOS/Linux, 2026-11-16. Implemented behavior is described below; required execution behavior remains an accepted implementation goal. The useful source conclusions are recorded in [research provenance](research/README.md) and the [execution plan](plans/execution.md).

## Implemented behavior

The Rust executable implements `init`, `note`, `exec`, `image`, `pop`, `verify` and `extract` with ordinary cold process execution. It supports stdin text/code/image inputs, merged stdout/stderr recording and command exit status, working-directory selection, copied images, replay/diffs and updated output, extraction and remote form/multipart delivery. New documents use mdrun provenance; untouched legacy source and metadata are retained. Explicitly choosing the input as `verify --output` updates that file; ordinary verification is read-only.

Clap owns all parsing, help and version output. Normal informational requests exit 0; invalid or missing arguments exit 2; document/verification failures exit 1. `exec` returns the child status (255 for signal termination on qualified Unix behavior). `list` and `preview` provide the discovery behavior below. No worker/session feature is implemented yet. Compiled Rust libraries do not require an installed language runtime or Showboat for host operation; invoked programs still need their own interpreters.

## Required document behavior

Retain Showboat’s seven document commands, familiar options, document format and execution behavior against a pinned release. On 2026-11-16 the owner explicitly replaced exact CLI mimicry with normal library-based parsing: use clap for argument handling, help, version and diagnostics; do not reproduce incidental parser quirks or write a custom option parser. The [compatibility plan](plans/compatibility.md) owns the inventory and baseline qualification. Preserve unmanaged Markdown and legacy provenance; newly authored documents identify mdrun truthfully. Do not rewrite a corpus merely to change branding.

Required commands are `init`, `note`, `exec`, `image`, `pop`, `verify` and `extract`. Required behavior includes stdin inputs, command failures, interpreter selection, working directories, captured output, image copying/alt text, exact replay and diffs, updated-copy output, extraction/quoting, timestamps/IDs, help/version/options and remote streaming. Recording and verification must operate without an installed Showboat executable; upstream is a test oracle only.

## Discovery behavior

`list FILE... [--filter TEXT] [--json]` lists replay-recognized executable blocks in file/block order, including existing unnamed Showboat commands and commands without recorded output. It excludes image and output blocks. Entries identify the file, zero-based parsed-block index, one-based opening-fence line, nearest preceding title/ATX heading, interpreter and first command line. The filter is a case-sensitive literal substring across filename, heading and complete command text. Positions locate blocks in the current document and are not persistent IDs. Setext headings are not interpreted.

JSON is an array of objects with `file`, `block`, `line`, `heading`, `interpreter` and `summary`; heading is null when absent. Inventory requires UTF-8 paths, headings, interpreters and command text and reports invalid text without emitting a partial inventory. Plain output escapes control characters so each entry occupies one row. `preview FILE BLOCK` emits the complete original fenced source span, preserving arbitrary bytes and line endings, and rejects missing or non-executable blocks. It uses replay's parser and performs no shell evaluation, variable expansion, stdin consumption or worker startup.

Selection initially operates on whole documents. Names, tag metadata, interactive pickers and partial block execution are [deferred](DEFERRED.md); discovery does not require rewriting the existing corpus or introducing dependencies between blocks.

## Required execution behavior

Fast execution is the ordinary development route for qualified cooperative bindings; unqualified commands execute normally. Explicit cold execution exercises the actual packaged launcher with original context and descriptors and starts no warm machinery. Session-local native forwarding preserves existing shell blocks, pipes, redirects and executable names. Runtime/application initialization may be reused; command results may not be cached implicitly.

| Boundary | Required guarantee |
| --- | --- |
| Binding | Prepared worker argv, absolute cold launcher and immutable artifact identity supplied externally; fixed startup context captured by the host or explicitly configured |
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
