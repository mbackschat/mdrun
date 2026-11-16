# Proposed session and SDK contract

## Scope and proposed commands

Start with one explicit executable binding, one sequential worker and one application adapter per session. Rust owns forwarding, admission, routing, reporting and process/resource custody; SDKs own wire transport and invocation streams; applications own callable entry points, eligibility and reset/resource checks. Preparation supplies immutable artifacts and does not run inside an invocation. There is no global daemon, automatic PATH enrollment, result cache or general process-state reset engine.

The developing public commands are `mdrun session --binding binding.json --mode fast --report report.json -- command args...` for a managed author command and `mdrun verify document.md --binding binding.json --mode fast --report report.json` for a document session. `--mode cold` executes the configured packaged launcher and starts no worker, SDK or warm stream relay. The native PATH binding and host control/reporting channel remain available for admission, process custody, route accounting and completion; this control machinery is distinct from warm execution. The cold child inherits the caller's original descriptors directly, and its application bytes never traverse the control channel. `--require-warm` turns a cold routing decision into a non-executing failure. The seven existing document commands retain their contracts. Repeated document CLI processes inside an outer author session inherit that session; explicitly starting another managed session inside it fails before payload launch.

Clap parses public and hidden subcommands, options and terminators. A native forwarding executable may recognize its configured `argv[0]` identity and forward every remaining native argument unchanged; it does not interpret application flags. Internal supervision modes must not introduce a handwritten option parser.

## Binding manifest

The following is a complete illustrative manifest. Paths are absolute, argv arrays are nonempty, and numeric durations are positive integer milliseconds. The host captures the session context; consumers do not construct or serialize complete environment maps. These examples specify JSON types; field names and protocol numbers are proposals.

```json
{
  "version": 1,
  "name": "example-tool",
  "artifact_id": "sha256:prepared-content-identity",
  "worker_argv": ["/usr/bin/java", "-jar", "/prepared/example-worker.jar"],
  "cold_argv": ["/prepared/bin/example-tool"],
  "cold_if_env_nonempty": ["JAVA_TOOL_OPTIONS"],
  "resources": [{"kind": "tcp_listener", "host": "127.0.0.1", "port": 49152}],
  "startup_timeout_ms": 30000,
  "session_timeout_ms": 300000,
  "cleanup_timeout_ms": 6000
}
```

`artifact_id` is a nonempty, preparation-supplied content identity for the selected application, SDK and launch configuration; the example value is a placeholder, not a real digest. The preparer owns its derivation and the immutability of the prepared files for the entire session. The host binds every connection and report to that identity and rejects mismatching readiness; echoing an identity is not independent proof that mutable files still match it. Qualification records the actual artifacts and independently checks warm/cold equivalence. No build-tool or application-specific discovery belongs in the host.

Before concurrency, PATH injection or payload launch, the host captures actual session cwd, the complete inherited native environment and umask once. It resolves any explicit execution working directory first while retaining PWD as a separate environment value. That fixed baseline supplies both worker startup and author/document execution; every invocation is checked against it. Native environment bytes remain host-owned and are not written into the prepared binding or reports. Fixed context means stable for the session, not necessarily supplied as a complete snapshot by the consumer.

An optional `context` object supports only explicit overrides: `cwd` is an absolute path, `environment` is a map from variable names to strings or null, and `umask` is an integer 0–511. Omitted fields inherit the captured values; null `context`, `cwd` and `umask` also mean no override. String environment entries replace values, null entries remove variables, and all unmentioned variables remain inherited. Overrides apply once to the session execution context and its worker, not independently to each invocation. Setting cwd does not silently overwrite PWD. Reject conflicting cwd selections rather than inventing precedence between a manifest override and an explicit execution working-directory option. Ordinary callers need no `context` object.

`cold_if_env_nonempty` is an optional array of variable names, defaulting to empty, whose nonempty caller values require packaged execution. It is application/launcher policy, not a runtime-specific host branch. If the fixed baseline contains such a value, the host does not start a worker: matching calls are cold, while changed contexts also remain cold under the normal context rule. `--require-warm` fails before payload launch in this case. This avoids a second worker environment and prevents launcher diagnostics, startup failures or options from being silently absorbed into reusable state. Removing startup variables only from a worker would need an additional qualified contract and is unnecessary for the initial slice.

`warm_ignore_env` is an optional array containing only distinct `"SHLVL"` and/or `"_"` names, defaulting to empty. For example, `"warm_ignore_env": ["SHLVL", "_"]` declares that neither variable affects any warm-eligible behavior under this binding. It ignores their values and presence only when comparing invocation environment with the fixed baseline; it does not alter either environment or the original cold launch. The declaration covers deliberate changes, empty values, additions and removals as well as shell bookkeeping. Native encoding, startup-option, cwd/PWD, umask, terminal and all undeclared environment checks still apply. Invalid or duplicate names fail manifest validation before payload launch.

The first `cold_argv` element is the absolute original launcher, never a name resolved through session PATH. Remaining elements are fixed launcher arguments preceding the caller's original arguments. Launcher override variables remain visible to that launcher. `resources` can be empty. Version 1 needs only explicitly configured exclusive loopback TCP listeners; adding other resource authorities requires a concrete consumer and a reviewed extension. Reject malformed manifests, unknown versions, duplicate resource endpoints and invalid executable bindings before launching user payloads.

## Startup, authentication and transport

The host creates a private directory with mode 0700, a private configuration file with mode 0600 and a native PATH binding for exactly `name`. It generates an unpredictable session identifier and authentication token. Tokens travel only in private startup configuration and authenticated connections; they must not appear in argv, documents, reports or diagnostics. The worker command is `worker_argv` followed by one configuration-file path; there is no shell expansion or placeholder language.

The startup file contains `protocol` (integer 1), `session_id` and `artifact_id` (strings), `token` (string), and a `connect` object with `host` (literal `127.0.0.1`) and `port` (integer 1–65535). The worker connects separate data and control sockets to the host and authenticates each with a `hello` frame containing these identity fields and `role` equal to `worker_data` or `worker_control`. Native forwarders authenticate with `role` equal to `client`. The host checks role uniqueness where required and rejects stale identities. The control connection remains observable when application input or output is blocked.

Frames are a four-byte unsigned big-endian byte length followed by that many UTF-8 JSON bytes. A frame is an object with a string `kind`; the maximum encoded frame is 1 MiB. Binary chunks use canonical padded Base64 and decode to at most 65536 bytes. Frame and chunk limits never impose a total invocation input/output limit. Reject malformed JSON, invalid encodings, unknown frame kinds, invalid field types, out-of-range integers and invalid state transitions. Implementations validate mandatory fields and reject unknown fields for protocol 1, so extensions require explicit protocol review.

When a warm worker is needed, it initializes the adapter, establishes its baseline resource checks and then sends `ready` on the data channel with `protocol`, `session_id`, `artifact_id` and `capabilities: ["pull_stdin", "sequenced_output", "release_ack"]`. These are mandatory version-1 capabilities, not an extensible plugin negotiation system. The host launches the author/document payload only after required worker readiness and process/resource custody are established. Explicit cold and a baseline already known to require cold have no worker readiness step. Startup stdout/stderr are private diagnostics, never protocol transport or captured application output. Readiness timeout is an infrastructure failure.

## Invocation and routing

Each native client authenticates on a private Unix socket, submits one native request and transfers duplicates of its three original standard descriptors with `SCM_RIGHTS`. It remains connected until release. The private request contains `request_id` (nonempty unique string), native `argv`, a context containing native cwd/environment bytes, umask (integer 0–511) and caller process group, plus `terminal` (object with Boolean `stdin`, `stdout`, `stderr`). Native strings and paths use lossless OS-string serialization; this host/client protocol is private to one mdrun executable revision, separate from the UTF-8 runtime SDK protocol. It contains no stdin bytes. Authentication identity is connection-scoped and is not repeated in every data frame. Every invocation frame includes `request_id`; output sequence numbers are integers in 0–9007199254740991.

The host receives native context before constructing any UTF-8 worker message. Values that cannot be represented faithfully select cold with reason `native_encoding`; explicit cold selects reason `explicit_cold` and needs no SDK connection. All routes use the same native request and reserve the same single admission slot. Overlap fails and poisons the session rather than silently queueing unqualified concurrent application access.

The host compares actual cwd, PWD, environment and umask against the baseline; any terminal descriptor, unsupported difference or nonempty `cold_if_env_nonempty` variable selects cold before application execution. Only mdrun's own environment variable and exact injected PATH entry are removed for comparison and cold execution. An explicit qualified `warm_ignore_env` declaration changes only environment comparison as specified above; all cold values remain intact. The client reads umask before starting concurrency and must not mutate shared descriptor flags to perform context inspection. SDK/application classification additionally checks native encoding and startup-option behavior that the generic host cannot infer.

For a matching request, the host sends `classify` to the worker with `request_id` and `argv`. The adapter must classify without application effects, application output or stdin reads. It responds with either `refused` containing a stable string `reason`, or `eligible`. Refusal clears the worker's classification state before responding, while the host retains the client reservation for cold execution. Eligibility retains the worker reservation; it cannot accept another request. On eligibility the host sends `invoke` with the same `request_id`; the worker sends `accepted` before calling the application. Host and SDK reject argument or identity changes between classification and invocation.

Explicit cold mode, local preflight or explicit worker refusal allows the host to send `cold` to the client with a stable `reason`. Native custody launches `cold_argv` plus the original native argv, using the caller's cwd, umask, transferred descriptors and environment apart from mdrun injection. Moving this launch into custody establishes ownership before the payload starts. The native client remains until the host supplies observed child status and cleanup evidence, then acknowledges and awaits release; it does not relay cold stdin/stdout/stderr. It does not retry or rediscover the PATH binding. `--require-warm` instead returns a failure without invoking either route. Context matching must not accidentally convert an explicitly cold request into a warm one.

Sending `invoke` creates an ambiguous-execution boundary: the worker may execute and lose its `accepted` response. An absent response does not prove refusal. After sending `invoke`, timeout, disconnect or malformed responses fail and poison the session without cold fallback or another invocation. The same no-retry rule applies after issuing a cold execution grant. A validated pre-execution refusal is the only worker response permitting cold execution. A separate classify step does not authorize retries after execution becomes possible.

## Streams, completion and release

| Frame | Direction | Required fields beyond `kind` and `request_id` | Meaning |
| --- | --- | --- | --- |
| `stdin_read` | Worker → host → client | `max_bytes`: integer 1–65536 | Credit issued only by an actual application input read; at most one credit is outstanding |
| `stdin` | Client → host → worker | `base64`: string | Nonempty bytes bounded by the outstanding credit; consumes that credit |
| `stdin_eof` | Client → host → worker | None | Consumes outstanding credit and establishes permanent invocation EOF |
| `output` | Worker → host → client | `sequence`: integer; `stream`: `stdout` or `stderr`; `base64`: string | Ordered binary output; sequence starts at zero and increments across both streams |
| `complete` | Worker → host → client | `exit_code`: integer 0–255 | Application returned, SDK output flushed, reset/resource checks passed; not yet permission to reuse |
| `client_done` | Client → host | `exit_code`: integer 0–255 or null; `signal`: positive integer or null | Output delivered; exactly one status field is non-null; warm status matches `complete` |
| `release` | Host → worker | None | Client acknowledged and host cleanup obligations permit the worker to finish this reservation |
| `released` | Worker → host; host → client | None | Worker reservation cleared, then client permitted to return |
| `fault` | Worker → host | `reason`: stable string | Infrastructure or invalid cleanup; never an ordinary application status; `request_id` is null only for faults outside an active reservation |

The native client reads stdin only after valid credit and never reads more than credited. An invocation that ignores stdin must complete while its input pipe remains open and unread. Do not start a speculative reader or wait for input EOF at completion. EOF persists within one invocation only. A pending read must remain cancellable without an indefinitely blocked join; changing shared open-file-description flags is not an acceptable shortcut. Completion with outstanding input credit is invalid.

SDK writes and host forwarding have bounded queues/backpressure. The client processes output frames in sequence and completes writes to its original stdout/stderr descriptors before `client_done`; this preserves `2>&1` order while leaving separate streams separate. A broken output descriptor is an infrastructure failure. Bounded framing must not become whole-stream buffering. The document recorder may retain a full output for Markdown comparison; that is separate from the native forwarding transport contract.

The worker does not reuse application state after `complete` until `release`. It responds `released`; only then does the host clear admission and tell the client `released`. Output after completion or after release poisons the session and cannot be attributed to the next call. Cold clients send `client_done` only after child status and required cleanup evidence are available; there is no worker release exchange on a cold route. The host still retains admission through the acknowledgement and its cleanup checks.

On the separate control channel, `shutdown` requests orderly SDK shutdown; closure also means owner loss. SDKs stop accepting work, interrupt cooperative invocation activity and report terminal health if possible. Host custody enforces termination deadlines independently of SDK cooperation. No cancellation, transport failure or cleanup failure is converted into an application success code. SDK exceptions that bypass the ordinary application-status contract poison reuse.

## Ownership and external resources

Acquire resource exclusions and establish native custody before worker or author/cold payload execution. Native guardians retain liveness/control channels, own process groups and duplicate exclusion handles as needed so parent loss does not release resources ahead of surviving payloads. Observe payload status separately from output EOF. A live group anchor may signal its own group; controllers must not later signal a remembered numeric group after its owner disappeared. Application checks cover invocation-created threads, children and late output in addition to host session cleanup.

Cancellation, timeout, owner/control loss and unexpected worker exit stop admission. Terminate owned work, escalate within a bounded deadline, observe descendant cleanup and report the actual outcome. Unknown membership, surviving resources or lost custody invalidate success and reuse. Keep exclusion while cleanup remains uncertain; do not claim cleanup merely because a deadline elapsed or the direct child exited. The exact macOS/Linux observation mechanism and terminal-preserving cold custody require executable qualification before this contract freezes.

The supported model is cooperative development execution, not containment of hostile processes. Deliberate process-group/session escape, external resource creation outside declared policy, simultaneous loss of all custodians and host/OS failure remain explicit limits. These limits do not excuse ordinary parent loss, reparented descendants inside owned groups or masked failures. A cleanup uncertainty needs an actionable report and retained custody, not a claim of automatic recovery.

## Native feasibility evidence

Scratch probes on 2026-11-16 (macOS 27.2.1 arm64, Apple clang 21.0.0) support an owner → custodian → group anchor → payload process tree. A direct-child-only negative control released a loopback listener while a reparented grandchild survived. With separate custody, owner `SIGKILL` left the listener excluded; a live anchor signalled its own group, and the custodian released the listener only after native observation found no live members. These probes establish feasibility, not production mdrun behavior or Linux qualification. Retain only these actionable conclusions; the scratch implementation is not imported into the product.

The custodian must remain outside payload groups and retain the listener when forced cleanup kills an anchor. Only the owner holds its liveness pipe's write end; inherited writers would hide owner loss. Payloads inherit neither exclusion nor liveness descriptors. A live anchor uses [`kill(0, signal)`](https://man7.org/linux/man-pages/man2/kill.2.html); a controller must not signal a remembered numeric group after its anchor has disappeared. Unexpected anchor death or uncertain observation retains custody and reports failure. Holding an exited anchor unreaped while checking membership is a further PID-pinning mechanism to qualify, not something established by the probe.

## Reports and SDK boundary

The host writes a versioned report to the selected file, outside captured application output. Required fields are `version`, `session_id`, `artifact_id`, binding/tool identity, overall outcome, application statuses, infrastructure failure, cleanup outcome, warm/cold call counts, stable cold reason counts, worker starts, and preparation/replay/total durations. Preparation duration is supplied by the preparer or explicitly unavailable, never invented. A session fails if infrastructure or cleanup failed even when the author shell exits zero after `tool ... || true`. Credentials and arbitrary environment values are excluded. Human diagnostics use the outer session channel, not an intercepted application's recorded stderr.

Each SDK exposes an application classification function over immutable argv, an invocation function receiving fresh request-owned argv/input/output/error streams, and reset/resource validation before completion. Classification and reset failure are distinct from ordinary application nonzero status. The JVM adapter can call a fresh CLI command tree while retaining qualified runtime initialization; Node can load an explicit handler once; Python can load an explicit factory. Applications reread mutable inputs/configuration and must not implicitly cache command results. Runtime-specific state checks belong in SDK/application code, never host route branches.

## Proof obligations before freeze

| Invariant | Required discriminating control |
| --- | --- |
| Input is genuinely lazy | Keep stdin open with unread bytes; an ignoring command completes and an independent read proves no consumption |
| Bounded streaming is not bounded total input | Send multi-megabyte binary input/output with slow readers and frame-sized limits; compare byte hashes and bounded transport memory |
| Descriptor fidelity | Compare separate and merged stdout/stderr, redirects, EOF, flush, broken readers and terminal-selected cold behavior against independent packaged cold |
| No retry under ambiguity | Perform one durable effect, drop the acceptance response, and assert exactly one effect plus session infrastructure failure |
| Context routes faithfully | Vary cwd/PWD independently, environment, umask, argument/native encoding and startup options; assert actual route and independent cold output/status/file effects |
| Reservation lasts through completion | Stall output/client acknowledgement and attempt overlap; assert no second application invocation or worker reuse |
| Late work poisons reuse | Emit after completion, leak a thread/child and fail reset; assert session failure even if the author masks the client status |
| Ownership outlives parent loss | Kill runner/owner/worker during active work with a surviving descendant; prove exclusion remains until observed cleanup |
| Cold is independent | Explicit cold starts zero workers/SDKs/warm stream relays, invokes the selected absolute packaged artifact with inherited descriptors, and still records native control/cleanup outcomes; refusal executes it at most once |

Record artifact identities, revision, OS/runtime versions and evidence limits for each result. Shared SDK conformance is necessary but does not replace application-specific state/resource parity. Passing one Linux or macOS run does not establish the other platform. Review unresolved custody, terminal and runtime-reset findings before freezing public APIs or claiming M3 completion.
