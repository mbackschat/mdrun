# Command reference notes

Use `mdrun -h` to find a command, `mdrun --help` for common workflows, and `mdrun COMMAND --help` for its syntax and examples. Help and guides run locally without starting an application worker. Installed packages also provide `man mdrun` and command-specific manual pages.

## Exit status

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

An executed program may itself return `1`, `2` or `125`; interpret the status in the context of the command you ran. `exec` records output even for nonzero status. `verify` checks output, not application exit status, and does not undo side effects. Managed-session reports distinguish application results from infrastructure and cleanup failures; use `mdrun guide sessions` for version 1 field meanings and cold-reason keys.

## Input and output

`note`, `exec` and `image` read their omitted text/code/image argument from stdin. Executed document blocks read EOF from `/dev/null` on stdin unless their own code supplies input. `exec` merges stdout/stderr into one capture and prints that capture on stdout. Verification diffs, inventory, previews, extraction and guide text use stdout; mdrun diagnostics use stderr.

Session invocation streams preserve separate stdout/stderr unless the caller merges them. Application output must use the adapter's request-owned streams. Session and batch reports go to `--report FILE` and contain metadata, not captured application bytes or credentials. Human output is not a machine schema: use `list --json` and the versioned reports for automation.

## Paths and files

- Document/image/output/report paths resolve from the caller's current directory. `--workdir DIR` affects executed programs, not those path arguments.
- `init` refuses an existing destination. `note`, `exec`, `image` and `pop` edit the selected document.
- `verify` leaves its input unchanged unless that same path is explicitly selected with `--output`. An output copy may replace an existing destination, so choose its path deliberately.
- Images are copied beside the document. Removing their Markdown entry does not remove the copied file.
- Binding files are supplied explicitly; mdrun does not search ancestor directories for a binding.
- Report paths must not alias the binding, a verification input or its selected output, including through symlinks or hard links. Complete reports replace their destination through a temporary file and rename; a publication error fails the command.
- Managed sessions create private temporary state with protected credentials. Successful cleanup removes it; uncertain cleanup can retain state and report its status path.

## Environment

| Variable | Meaning |
| --- | --- |
| `PATH` | Finds document interpreters and ordinary programs. Managed sessions prepend only their explicit forwarded executable binding. |
| `SHOWBOAT_REMOTE_URL` | Optional compatible remote receiver. Editing commands POST document updates; unset/empty disables delivery. Delivery failure warns without discarding the local edit. |
| `MDRUN_SESSION` | Internal managed-session context, set by mdrun for its children. Do not set it manually or copy private configuration into documents. |

The invoked application receives its execution environment. Warm admission compares context; runtime startup variables may require cold execution according to the binding. The host needs no installed Rust, Go, JVM, Node or Python runtime to start, but commands/adapters still need their own tools.

## Remote delivery

Remote updates retain the compatible `SHOWBOAT_REMOTE_URL` name. `init`, `note`, `exec` and `pop` send URL-encoded form data; `image` sends multipart data. The document UUID identifies the stream. Network errors are warnings rather than transaction rollback; local document success does not prove remote delivery.

## Compatibility

Existing Showboat document provenance remains intact. New documents identify mdrun. Source-preserving edits intentionally retain unmanaged spacing and line endings. Clap owns normal help, errors, option handling and `--`; incidental Showboat parser quirks are not compatibility requirements.
