# Self-explaining CLI and user guide

Status: documentation/help implementation and focused local controls are present, 2026-11-16; complete semantic-reference review and cross-platform installed-manual qualification remain open. The owner requested an engaging, instructive README and self-explaining help/man pages, using Showboat and LOCmetry as references. This extends the documentation gate in [execution qualification](execution.md); it does not postpone the existing runtime, scheduling or distribution requirements.

## Reading paths

| Surface | Reader's task | Required content |
| --- | --- | --- |
| [README](../../README.md) | Understand the value and complete a first useful workflow | Clear purpose, truthful installation/status, copyable quick start, actual recorded result, task-oriented command table, verification/update workflow, warm-session value and prerequisites, links to focused reference and SDKs, Showboat credit |
| `mdrun -h` | Find a command quickly | Compact capability and command summaries; no full manual dump |
| `mdrun --help` | Choose and execute a common task | A small working author/verify example, relevant defaults and conditional routes to command help or guides |
| `mdrun COMMAND --help` | Use one command correctly | Syntax, material defaults, input/output/path rules, examples and observable results; explain interpreter `-c`, captured failures, read-only verification and session terminator where relevant |
| Embedded task guides | Work offline without locating a checkout | Small, separately selectable document, session and reference topics; one maintained Markdown source per guide |
| `mdrun(1)` and command man pages | Consult installed command reference | Generated command/option reference plus exit statuses, environment, files and focused examples; install with Homebrew/Linux packages and include in archives |
| JSON inventory/session reports | Automate without scraping presentation | Document existing field meanings, actual route/worker counts and failure interpretation; preserve existing data contracts |

Keep the root help short enough for unrelated tasks. Route to deeper material conditionally, naming the relevant command or guide rather than requiring an index detour. Explain what a successful command establishes and how to inspect its result. Preserve the accepted cooperative process-group boundary and distinguish implemented experimental sessions from qualified release claims.

## Implementation and acceptance

- [x] Rewrite the README around user tasks, replacing the current project-status-first structure while retaining honest release status and attribution.
- [x] Add concise short help and actionable long command help through clap. Parsing, validation, terminators and diagnostics remain library-owned.
- [x] Embed focused Markdown guides behind a clap-parsed guide command; keep examples and reference facts in their maintained source files.
- [x] Generate man pages from the same clap command definitions with a qualified development-only generator, avoiding a second option inventory or a handwritten CLI parser. Include semantic reference sections and exclude internal custody commands.
- [ ] Package/install the man pages with archives, Homebrew and both Linux package formats; verify their presence and rendering.
- [x] Execute the displayed quick-start/help recipes against independent expected bytes and statuses, including changed-output failure and original-document preservation. Recorded output must come from real execution.
- [x] Guard public command/option coverage, guide-source agreement and generated-man freshness. These checks establish coverage and source agreement, not correctness of every reference statement.
- [ ] Complete a semantic audit of reference exits, context/routing, reports, paths and lifecycle limits against implementation and independent behavioral controls. The focused cold-reason guard covers host reason names only.
- [ ] Complete independent representative first-use, automation and warm-session review after the batch/report repairs; retain observed usability gaps and limitations. Shorter prose alone does not establish measured agent efficiency.

Shell completions and machine-readable schemas may be evaluated separately if the maintained help/guide surfaces leave a concrete discovery gap; they are not prerequisites for this requested checkpoint. No graphical documentation system or reproduction of LOCmetry's research harness is required.

## Local qualification

The README and long-help recipes have independent literal output/status assertions. The [retained greeting](../examples/greeting.md) was captured by mdrun, not hand-authored. Bootstrap guards cover exact guide/source agreement, short/long help distinction, changed-output detection, updated-copy output with input preservation, and multiline command-source stdin. The pre-implementation guide/help controls failed before the new command and examples existed. Source agreement and replay agreement do not establish every guide fact.

The initial local run rendered all public-command manual pages with mandoc and passed freshness, public-option coverage and hidden-command exclusion; missing pages and an intentionally stale page failed. Both new development dependencies passed the full registry quarantine before compilation, with existing locked versions retained. Independent review corrected cold sessions' zero-worker wording and distinguished ordinary document failure from custody-wide cancellation. Subsequent source review found semantic gaps in cold reason naming, signal/forwarding statuses, resource concurrency, report fields and batch output buffering; the owning guides/help now describe those behaviors. The integrated cold-reason guard must pass with the remaining applicable checks before accepting the repair. No agent-efficiency measurement is claimed.

Archive guards preserve every generated page's bytes and reject a missing root page. Homebrew and Linux assets/install oracles include the pages; the packaging suite passes locally. Actual cross-target installed checks are still required before the packaging task above is complete. Generated pages include the experimental batch command; that command's lifecycle qualification remains separately governed by [scheduling](scheduling.md).

## Maintain the help and manuals

Edit command syntax/options in clap, extended examples in [CLI help](../../src/cli/help.rs), and semantic reference text in the [guides](../guides/README.md). The guide command embeds those Markdown files directly. The development-only [manual generator](../../src/manual.rs) reads the public clap tree, excludes internal commands and adds the reference guide's exit, input/output, file and environment semantics. Its finite Markdown adapter rejects unsupported formatting instead of silently omitting it.

After a command or reference change, explicitly regenerate the checked pages, then run ordinary freshness tests. Substitute the absolute checkout path in the first command:

```sh
MDRUN_MAN_OUTPUT=/absolute/path/to/mdrun/docs/man cargo test --locked --bin mdrun manual::regenerate -- --ignored
cargo test --locked --bin mdrun manual::
```

Where mandoc is installed, render every page with `MDRUN_MANDOC=/absolute/path/to/mandoc cargo test --locked --bin mdrun manual::render_with_mandoc -- --ignored`. Generation uses an explicit review date, not a nondeterministic build timestamp. Normal tests never rewrite pages. Package assets consume checked pages; installed users need neither Cargo nor the generator.

## Sources and adapted lessons

[Showboat's pinned README](https://github.com/simonw/showboat/blob/d531261b8faf0c388b02c7891d50f1f47c3e2b52/README.md) leads with executable demonstrations, explains captured failures and replay, and shows how Markdown is produced. Adapt that practical progression while keeping mdrun's actual command behavior and installation routes explicit; do not reproduce the whole help text in the README.

Read-only LOCmetry inspection used revision `d96eb6369484c1e8f60e65d1b65a41b64485629b`. Its [help design](https://github.com/mbackschat/locmetry/blob/d96eb6369484c1e8f60e65d1b65a41b64485629b/docs/help-design.md) and [agent lessons](https://github.com/mbackschat/locmetry/blob/d96eb6369484c1e8f60e65d1b65a41b64485629b/docs/agent-lessons.md) support task-specific detail, conditional reference routing, observable verification, meaningful errors and one source for embedded guides. Its measured results belong to its tested tasks/models; this project adopts the design lessons without inheriting those performance claims or its implementation complexity.
