# Reuse a qualified application

A command-line application normally starts a new process for every call. In a walkthrough with many calls, that can mean starting the JVM, loading Node modules or importing Python code over and over. A managed session lets eligible calls share an initialized application process, called a worker. Each call still runs against its current inputs; results are not cached.

Reusing a process also reuses its state. The application developer must supply an adapter that selects safe calls and checks that the worker is ready for the next one. mdrun manages the session around that adapter; it cannot make an arbitrary script safe to reuse. The [runnable Python example](../examples/warm-python.md) includes the application, adapter and setup so you can try the complete workflow before preparing your own.

Managed sessions follow the frozen version-1 contract for the 0.1.0 release candidate. Release qualification is tracked in [PLAN](../PLAN.md); publication is held.

## What you prepare

Start with the application's ordinary command-line entry point. It remains the cold launcher: the way to run one call in a fresh process. Alongside it, the application developer writes an adapter using the [JVM](../../sdk/jvm/README.md), [Node](../../sdk/node/README.md) or [Python](../../sdk/python/README.md) SDK. The SDK carries requests and streams between mdrun and the worker; the adapter connects those requests to application code.

The adapter has three responsibilities:

- Classify the arguments before execution, without side effects, application output or input reads, and refuse calls that are not safe to reuse.
- Invoke the application with the streams owned by that request, so output reaches the calling shell command.
- Check that application state and owned work permit reuse after the call; the SDK cannot reset arbitrary application state.

Build and test this application/adapter pair outside mdrun. Qualification means checking that eligible warm calls agree with the independent cold launcher in output, status and relevant file effects, and that cleanup permits safe reuse. A binding is the JSON file you supply to select those prepared executables. Its minimal shape is:

```json
{
  "version": 1,
  "name": "mytool",
  "artifact_id": "qualified-build-identity",
  "worker_argv": ["/absolute/path/to/prepared-worker"],
  "cold_argv": ["/absolute/path/to/packaged-mytool"]
}
```

The paths and artifact identity above are placeholders for files you prepare. `name` chooses the shell command that mdrun will forward inside the session. `worker_argv` launches the reusable adapter; `cold_argv` launches the ordinary packaged application independently. mdrun appends a private startup configuration path to `worker_argv`. For an interpreted adapter, put an absolute interpreter path first and the adapter script in the remaining arguments.

`artifact_id` identifies the immutable application, SDK and launch configuration you tested. You derive that identity and keep the prepared files unchanged throughout the session; mdrun does not build or hash them for you. See the [binding contract](../plans/session-protocol.md#binding-manifest) for optional context, resource and timeout settings, and the [SDK distribution guide](../sdk-distribution.md) to prepare a versioned SDK dependency.

## One authoring session

Once `binding.json` points to your prepared application, put the commands you want to record in an author script, here named `author.sh`. Start one session around that script:

```sh
mdrun session --binding binding.json --report session.json -- sh author.sh
```

`author.sh` is your shell script, and `session.json` is the report mdrun writes. The `--` separates mdrun options from the script's command and arguments. Ordinary `mdrun init`, `note` and `exec` calls in that script share the outer session, so repeated `mytool` calls can use the same worker. The [Python example](../examples/warm-python.md#authoring-calls-and-the-resulting-markdown) shows both authoring calls and the Markdown they produce; you can also write that Markdown directly.

A shell command reaches the adapter through a temporary command that mdrun creates. At startup, mdrun makes a symbolic link named `mytool` to its own executable and puts that link's directory first on the session's PATH. Calling `mytool` starts a small native forwarding client: mdrun recognizes the configured name and sends the arguments to the session. The session then selects the worker or the cold launcher. mdrun supplies this forwarding entry point; you supply the application adapter separately.

The PATH change belongs to this session and its children; it does not edit shell startup files or enroll unrelated programs. Nested managed sessions are rejected. Inside an outer session, use ordinary `mdrun verify demo.md` without another `--binding`.

Each session has at most one worker, and application calls run sequentially. Warm execution requires both eligible arguments and a supported invocation context. A refusal or unsupported context can select the cold launcher before execution. Once mdrun asks the worker or cold launcher to execute a call, it never retries that call, including after a disconnect: losing a response does not prove that the call had no effects.

## Verify and inspect the route

After recording `demo.md`, verify it using the same prepared binding. These two commands ask different questions about execution:

```sh
mdrun verify demo.md --binding binding.json --require-warm --report warm.json
mdrun verify demo.md --binding binding.json --mode cold --report cold.json
```

The first requires every forwarded call to use the worker. `--require-warm` fails before executing a call that would need the cold route. The second explicitly uses the packaged launcher and starts no worker. Both still compare document output, but neither option alone proves full warm/cold equivalence: compare actual bytes, statuses and relevant file effects for your workload.

A forwarded call attached to a terminal on stdin, stdout or stderr selects cold. An interactive call can therefore fail `--require-warm` even when its arguments are otherwise eligible. The [cold route reasons](#cold-route-reasons) explain other causes.

Successful verification tells you that recorded output matched. The reports tell you how calls ran and whether cleanup succeeded. Inspect `outcome`, `infrastructure_failure` and `cleanup.outcome` before interpreting `warm_calls`, `cold_calls`, `cold_reasons` and `worker_starts`. Application statuses remain separate from session health: a shell that masks a failed forwarding client cannot turn a poisoned session into success. Unmeasured preparation time is reported as unavailable, not zero.

## Independent documents

When documents use independent inputs and side effects, check several at once:

```sh
mdrun verify-many --binding binding.json --jobs 4 --report batch.json -- first.md second.md
```

Here `first.md` and `second.md` are documents you supply, and `--jobs 4` sets the maximum number of active document sessions. Each started document gets its own session and at most one sequential worker. A slot remains occupied through startup, replay and cleanup. The first observed document failure, including a recorded-output mismatch, stops mdrun from starting further documents; already-started healthy documents finish within their deadlines. A failure of batch resource custody can stop all sessions. The aggregate report includes every requested document, including those not started.

All documents use the same binding. A binding can declare an exclusive resource, such as a loopback TCP address and port reserved for the application's local service. If any such resource exclusion is declared, effective concurrency is `1` unless you explicitly select `--share-resources` for documents that may safely share those resources. With sharing enabled, or with no declared resources, effective concurrency is the smaller of `--jobs` and the document count. Sharing is your assertion that the declared resources can be shared; it does not establish independence of arbitrary command side effects. Batch stdin reads EOF from `/dev/null`; supply input in the code through a file, pipeline or redirection.

The batch keeps its external resource exclusions across gaps between documents. Resource custody means retaining those exclusions until cleanup has been observed; a finished document does not release the batch's exclusion for another batch to claim.

For readable output, mdrun writes each document's stdout and stderr to separate temporary spool files, then publishes them in input order after that document's session finishes. A slow earlier document delays visible output from later completed documents. Labels use quoted, escaped native filenames, such as `"first.md":`; use the JSON report for automation rather than parsing these labels. Each stream stays grouped by document, but publication does not reconstruct stdout/stderr timing.

Spools have no byte quota and remain until batch workspace cleanup. Large output can therefore consume substantial temporary disk space even after a document finishes. `--jobs` bounds active sessions, not spool size.

## Cold route reasons

If a call uses a fresh process unexpectedly, inspect `cold_reasons` in its session report. This object counts the reasons actually selected by the host. Host-generated keys are listed below; the application's side-effect-free classifier can also supply its own nonempty refusal reason. `--require-warm` rejects a would-be cold route before executing the call, so that rejected route is not counted as a cold call.

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

Use reports when a script or CI check needs to distinguish matching output, worker reuse and clean completion. They contain execution and cleanup metadata, separate from captured application output, and include no captured application bytes or session credentials. Check `version` before interpreting fields; the version-1 schemas below are frozen for this release candidate.

Check the command result and report freshness together. After preflight succeeds, mdrun invalidates any previous report before managed execution, then publishes complete JSON through a temporary file and rename. Preflight rejection can leave an older report in place, and setup or publication failure can leave no new report. A missing, unreadable or stale report cannot establish success.

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

An application can observe more than its arguments. mdrun captures the working directory (`cwd` and the separate `PWD` environment value), environment, file-creation mask (`umask`) and other invocation context. Unsupported differences select cold before execution. The worker and cold launcher must remain equivalent within the context the binding allows.

Optional binding policies can declare that specifically tested environment differences do not affect warm-eligible behavior. They are not a general way to ignore mismatches. `SHLVL` and `_` remain strict by default, so shell wrappers can select the `environment` cold reason. Declare their irrelevance only after testing the application, using the [binding contract](../plans/session-protocol.md#binding-manifest). Defaults are 30 seconds for worker startup, 300 seconds per session and 6 seconds for cleanup; binding JSON can set positive millisecond budgets.

Managed scripts and applications must keep child processes in their assigned process group, the operating-system group mdrun uses to stop and observe their work. They must also stop and await background work for successful orderly completion. Shell monitor mode (`set -m`), independently created groups and daemonization are unsupported. mdrun cannot reliably detect every departure; a descendant outside this boundary may survive a successful report and released exclusion.

Within the supported boundary, observed cleanup uncertainty fails the session and retains resource custody. Keep the reported status path for diagnosis. A missing output stream, an exited direct child or an elapsed timeout does not prove cleanup. Likewise, SDK reset is the application's attestation that it is safe to reuse, not a generic reset of modules, globals, threads or native extensions. Follow the [preparation and SDK links](#what-you-prepare) for your application and qualify that lifecycle alongside warm/cold behavior.
