# Reuse a qualified application

Managed sessions are experimental while runtime and release qualification continues. They reuse application initialization across repeated calls; they do not cache command results or automatically make an arbitrary script safe to reuse.

## What you prepare

A cooperative application supplies an SDK adapter with three responsibilities: classify eligible arguments without side effects, invoke the application using request-owned streams, and attest that application state and owned work permit reuse. JVM, Node and Python SDKs implement the common transport/lifecycle contract. Build preparation stays outside mdrun.

The binding JSON identifies that prepared worker and its independent packaged-cold launcher. The minimal shape is:

```json
{
  "version": 1,
  "name": "mytool",
  "artifact_id": "qualified-build-identity",
  "worker_argv": ["/absolute/path/to/prepared-worker"],
  "cold_argv": ["/absolute/path/to/packaged-mytool"]
}
```

These are placeholders for existing prepared executables, not files mdrun creates. The host appends a private startup configuration path to `worker_argv`. Interpreted adapters put an absolute interpreter first and the adapter script in the remaining arguments. `artifact_id` identifies the immutable artifacts you qualified; mdrun does not build or hash the application for you.

## One authoring session

```sh
mdrun session --binding binding.json --report session.json -- sh author.sh
```

Within `author.sh`, repeated `mytool` commands resolve through a session-local PATH binding. Ordinary `mdrun init`, `note` and `exec` calls can share this outer session. The binding does not change your shell startup files or enroll unrelated programs. The `--` separates mdrun options from the author's command and arguments. Nested managed sessions are rejected: inside an outer session, use ordinary `mdrun verify demo.md` without another `--binding`.

Each session has at most one sequential worker. An application call is eligible for warm execution only when classification and context checks agree. A refusal or unsupported context can route to the independent cold launcher before acceptance. An accepted invocation is never retried cold, including after a disconnect.

## Verify and inspect the route

```sh
mdrun verify demo.md --binding binding.json --require-warm --report warm.json
mdrun verify demo.md --binding binding.json --mode cold --report cold.json
```

`--require-warm` fails calls that cannot use the warm route. A forwarded call connected to a terminal on stdin, stdout or stderr selects cold, so an interactive call can fail this assertion even when its arguments are otherwise eligible. Explicit cold starts no worker and uses the packaged launcher. Neither option proves output parity by itself: compare actual bytes, statuses and relevant file effects for your workload.

Inspect `outcome`, `infrastructure_failure` and `cleanup.outcome` before interpreting `warm_calls`, `cold_calls`, `cold_reasons` and `worker_starts`. Application statuses are separate from session health. A shell that masks a failed client cannot turn a poisoned session into success. Preparation duration can be unavailable; it is not silently counted as zero preparation work.

## Independent documents

```sh
mdrun verify-many --binding binding.json --jobs 4 --report batch.json -- first.md second.md
```

Each admitted document has its own session with at most one sequential worker. A scheduling slot includes startup, replay and cleanup. The first observed document failure, including a recorded-output mismatch, stops admission; already-started healthy documents finish within their deadlines. A batch custody failure can stop all sessions. The aggregate report includes every requested document, including those not started.

All documents use the same binding. If that binding declares any resource exclusion, effective concurrency is `1` unless you explicitly select `--share-resources` for documents that may safely share those resources. With sharing enabled, or with no declared resources, effective concurrency is the smaller of `--jobs` and the document count. The batch retains its external exclusions across gaps between documents; sharing does not establish that arbitrary command side effects are independent. Batch stdin reads EOF from `/dev/null`. Use code-level input redirection where needed.

Document stdout and stderr are spooled to separate temporary files and published in input order after each document's session finishes. A slow earlier document delays visible output from later completed documents. Labels use quoted, escaped native filenames, such as `"first.md":`; treat them as human presentation rather than a parseable interface. Each stream stays grouped by document, but stdout/stderr publication does not reconstruct their original timing. Spools have no byte quota and remain until batch workspace cleanup, so output can consume substantial temporary disk space. `--jobs` bounds active sessions, not spool size.

## Cold route reasons

The `cold_reasons` object counts the reasons actually selected by the host. Host-generated keys are listed below; an application's side-effect-free classifier can also supply its own nonempty refusal reason. `--require-warm` turns a would-be cold route into session failure before executing that call, so the rejected route is not counted as a cold call.

| Key | Meaning |
| --- | --- |
| `explicit_cold` | The session selected `--mode cold`. |
| `terminal` | At least one forwarded standard descriptor is a terminal. |
| `native_encoding` | An argument, cwd or environment entry cannot be represented by the warm UTF-8 transport. |
| `startup_options` | A binding-declared `cold_if_env_nonempty` variable has a nonempty value. |
| `cwd` | The invocation's physical working directory differs from the session baseline. |
| `umask` | The invocation's umask differs from the baseline. |
| `environment` | Environment names, values or presence differ after removing the private session injection and any explicitly qualified ignored names. |

## Version 1 reports

Reports describe execution and cleanup separately from captured application output. Check `version` before interpreting fields; the managed-session API and report contract remain experimental. Report files contain no captured application bytes or session credentials. After preflight succeeds, mdrun invalidates any previous report before managed execution; complete JSON publishes through a temporary file and rename. Preflight rejection can leave an older report in place, and setup or publication failure can leave no new report. Check the command result and report freshness; a missing, unreadable or stale report cannot establish success.

The single-session report used by `session`, `verify --binding` and each completed batch document has these fields:

| Field | Type and meaning |
| --- | --- |
| `version` | Integer `1`. |
| `tool` | Object with string `name` and `version` identifying mdrun. |
| `session_id`, `artifact_id`, `binding.name` | Strings identifying this session and the explicitly supplied prepared application. The artifact identity is supplied by the binding, not a host-computed artifact hash. |
| `outcome` | String `success` only when the author exits zero, no infrastructure failure is recorded and custody cleanup is clean; otherwise `failure`. |
| `author_status` | Object with `exit_code` (integer or null) and `signal` (integer or null), or null when no author status was observed. A signalled author has a null exit code; the session CLI returns `128 + signal` when session health permits. |
| `application_statuses` | Array of observed forwarded application results: string `request_id`, string `route` (`warm` or `cold`), and nullable integer `exit_code` and `signal`. Warm results use an exit code; cold results preserve either a native exit code or signal. An unfinished invocation can lack an entry. |
| `infrastructure_failure` | First recorded infrastructure failure as a string, or null. An ordinary nonzero application status alone does not poison the session. |
| `cleanup` | Object with `outcome` (`clean` or `uncertain`), integer `custodian_pid`, and nullable string `status_path` identifying retained custody evidence. A clean document scope in a batch does not release the batch's exclusion. |
| `warm_calls`, `cold_calls`, `worker_starts` | Integer counts of warm invocation requests issued, cold routes selected and worker launches acknowledged. Attempts that later fail can be counted; these are not successful-completion counts. Cold-only sessions start no worker. |
| `cold_reasons` | Object mapping each selected cold reason string to its integer count. |
| `durations_ms` | Object with nullable `preparation` (currently null), integer `replay` from just before author launch through cleanup, and integer `total` including host setup/startup. These are elapsed wall times, not additive phase measurements or a performance qualification. External build preparation is unmeasured. |

The aggregate `verify-many` report has its own `version: 1`, `tool`, `outcome`, `infrastructure_failure` and `cleanup`, plus the following fields. Overall success requires every requested document to succeed and batch custody to finish cleanly. A null batch `infrastructure_failure` does not imply document success: inspect `documents` too.

| Field | Type and meaning |
| --- | --- |
| `binding` | Object with string `name` and `artifact_id`, shared by all documents. |
| `cleanup` | Object with `outcome` (`clean`, `uncertain` or `not_started`), nullable integer `custodian_pid` and nullable string `status_path`. `not_started` means batch custody was not established; it does not certify cleanup of a launched batch. |
| `concurrency` | Object with integer `requested` and `effective` limits and boolean `share_resources`. |
| `budget_ms` | Object with integer `document_session`, `document_cleanup` and `batch` budgets. Cleanup includes the host's acknowledgement grace; these are limits, not measured durations. |
| `durations_ms.total` | Integer elapsed batch wall time. |
| `documents` | Array in requested input order, including documents that were not admitted. |
| Document `index`, `file` | Zero-based integer input ordinal and string input path. `file` uses lossy UTF-8 display; when that loses native bytes, an additional `file_native_bytes` array of byte integers preserves identity. |
| Document `state: completed` | Contains integer `status` and a `session` report. Completed means a session report was produced; the status/report can still indicate failure. |
| Document `state: failed` | Contains string `error`; no valid session report was produced for that admitted document. |
| Document `state: not_started` | Contains string `reason` equal to `admission_stopped`; the document was not admitted. |

## Context and ownership limits

The host captures cwd/PWD, environment, umask and other invocation context. Unsupported differences choose cold before execution. Optional binding policies can declare specifically qualified environment irrelevance; they are not a general way to ignore mismatches. `SHLVL` and `_` remain strict by default; shell wrappers can therefore select the `environment` cold reason. Declare their irrelevance only after qualifying the application, using the [binding contract](../plans/session-protocol.md#binding-manifest). Defaults are 30 seconds for worker startup, 300 seconds per session and 6 seconds for cleanup; binding JSON can set positive millisecond budgets.

Managed scripts and applications must keep descendants in their assigned process group and stop/await background work for successful orderly completion. Shell monitor mode (`set -m`), independently created groups and daemonization are unsupported. The host cannot reliably detect every departure; a descendant outside this boundary may survive a successful report and released exclusion.

Within the supported boundary, observed cleanup uncertainty fails the session and retains resource custody. Keep the reported status path for diagnosis. Do not infer cleanup from a missing output stream, an exited direct child or an elapsed timeout. SDK reset is application attestation, not a generic reset of modules, globals, threads or native extensions.
