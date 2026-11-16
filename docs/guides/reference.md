# Command reference notes

Use this reference when a script needs to interpret a result or a command needs precise path, stream or environment behavior. For a walkthrough, start with `mdrun guide documents` or `mdrun guide sessions`. Use `mdrun -h` to find a command, `mdrun --help` for common workflows, and `mdrun COMMAND --help` for syntax and examples. Help and guides run locally without starting an application worker. Installed packages also provide `man mdrun` and command-specific manual pages.

## Exit status

An exit status answers a different question for recording, verification and managed execution. Check which command produced it before using it as a success condition.

| Command or outcome | Status |
| --- | --- |
| Successful help, version, guide, document editing, inventory or preview | `0` |
| `verify`: recorded output matches | `0` |
| `verify`: changed recorded output or execution/document error | `1` |
| Invalid CLI usage | `2` from clap |
| `exec` | Executed program's exit status; `255` for signal termination on supported Unix platforms |
| `session` | Author's exit status, or `128 + signal` for a signalled author, when session health/cleanup succeeds; `1` for infrastructure or uncertain cleanup failure |
| Forwarded binding executable | Application status, including native signal termination for a signalled cold process; `125` on forwarding/control failure |
| `verify-many` | `0` only when all requested documents and batch cleanup succeed; otherwise `1` |

`exec` records output even for a nonzero program status, so it can capture an expected failure. `verify` checks output, not application exit status, and does not undo side effects. A matching error demonstration can therefore verify successfully. An executed program may itself return `1`, `2` or `125`; the number alone does not identify a failure in mdrun.

For managed execution, inspect the report as well as the command result. Reports distinguish application results from infrastructure and cleanup failures; use `mdrun guide sessions` for version 1 field meanings and cold-reason keys.

## Input and output

There are two separate inputs when recording a command. If you omit the text/code/image argument, `note`, `exec` and `image` read that argument from stdin. This supplies source to record. Executed document blocks instead read EOF from `/dev/null` on stdin unless their own code supplies input through a file, pipeline or redirection.

`exec` merges the program's stdout/stderr into one capture and prints that capture on stdout. Verification diffs, inventory, previews, extraction and guide text also use stdout; mdrun diagnostics use stderr.

A session invocation preserves separate stdout/stderr unless the caller merges them. The application must write through the adapter's request-owned streams so bytes reach the right invocation. Session and batch reports go to `--report FILE` and contain metadata, not captured application bytes or credentials. For automation, use `list --json` and versioned reports; human-readable output is not a machine schema.

## Paths and files

- Resolve document, image, output and report paths from the caller's current directory. `--workdir DIR` changes where programs execute; it does not change the basis of those path arguments.
- Use `init` for a new destination; it refuses an existing file. `note`, `exec`, `image` and `pop` edit the selected document.
- Choose `verify --output` deliberately. Ordinary verification leaves the input unchanged, but an output copy may replace an existing destination. Selecting the input path itself explicitly permits an in-place update.
- Find copied images beside the document. Removing an image's Markdown entry leaves the copied file in place.
- Supply the binding path explicitly. mdrun does not search ancestor directories for a binding.
- Keep report paths distinct from the binding, verification inputs and selected output, including through symlinks or hard links. Complete reports replace their destination through a temporary file and rename; a publication error fails the command.
- Use the reported status path to investigate uncertain cleanup. Managed sessions create private temporary state with protected credentials; successful cleanup removes it, while uncertain cleanup can retain it.

## Environment

| Variable | Meaning |
| --- | --- |
| `PATH` | Finds document interpreters and ordinary programs. A managed session prepends a temporary directory containing only its explicitly bound forwarding command, a symbolic link to mdrun. |
| `SHOWBOAT_REMOTE_URL` | Optional compatible remote receiver. Editing commands POST document updates; unset/empty disables delivery. Delivery failure warns without discarding the local edit. |
| `MDRUN_SESSION` | Internal managed-session context, set by mdrun for its children. Do not set it manually or copy private configuration into documents. |

The invoked application receives its execution environment. Before choosing warm execution, mdrun compares the invocation context with the session baseline; runtime startup variables may require cold execution according to the binding. The session guide describes those checks and their report reasons.

The native mdrun host starts without an installed Rust, Go, JVM, Node or Python runtime. Commands and adapters still need the interpreters and application tools they invoke.

## Remote delivery

To send edits to a compatible remote receiver, set `SHOWBOAT_REMOTE_URL`. The variable retains its Showboat-compatible name. `init`, `note`, `exec` and `pop` send URL-encoded form data; `image` sends multipart data. The document UUID identifies the stream. Network errors produce warnings and preserve the local edit, so successful local editing does not prove remote delivery.

## Compatibility

Existing Showboat documents keep their provenance; new documents identify mdrun. Source-preserving edits retain unmanaged spacing and line endings, so using mdrun does not require rewriting an existing corpus. Clap owns normal help, errors, option handling and the `--` terminator; incidental Showboat parser quirks are not compatibility requirements.
