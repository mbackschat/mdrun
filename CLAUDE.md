# Repository guidelines

mdrun is a Rust CLI for executable Markdown with cooperative warm execution. Edit this canonical file and preserve `AGENTS.md` as the relative symlink `CLAUDE.md`. Global instructions remain applicable; this file owns project-specific conventions.

## Start here

- Read [PLAN](docs/PLAN.md), then only the contract, plan section and source relevant to the task.
- [PRODUCT](docs/PRODUCT.md) distinguishes implemented behavior from accepted goals; [README](README.md) owns user entry points.
- [Documentation index](docs/README.md) owns navigation. [Research provenance](docs/research/README.md) identifies the extracted feasibility conclusions and their limitations.
- [Distribution](docs/distribution.md) owns release targets, packaging and Homebrew publication; the MIT license and retained `mdrun` name are owner decisions.

The extracted requirements guide product direction; historical prototype guarantees are not current implementation claims. Do not reopen the accepted name, Rust choice, full compatibility goal or runtime order without a concrete blocker.

## Structure and boundaries

| Location | Owns |
| --- | --- |
| [src](src) | Production Rust executable; introduce modules when a milestone needs them |
| [tests/integration](tests/integration) | Modules of one integration-test binary; no top-level `tests/*.rs` |
| [docs/plans](docs/plans/README.md) | Active implementation and migration acceptance plans |
| [docs/research](docs/research/README.md) | Source provenance and extracted conclusions |
| [.github/workflows](.github/workflows) | Current product CI |

Reference checkouts remain read-only until an explicit migration task authorizes their edits. Test drives may use isolated copies or prepared artifacts outside that checkout; never write build output or modify its harness there. Production tests must not require either the original Downloads folder or another project checkout.

## Development and verification

```sh
cargo test --locked
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo build --release --locked
```

Use red/green TDD for features and fixes: define the observable invariant and oracle, show the focused guard failing, then implement and run the applicable checks. Group integration tests under the single binary and filter focused work with `cargo test --locked --test integration MODULE::`. Keep ordinary tests offline and self-contained; explicit differential/runtime qualification may require separately prepared tools.

Run Cargo commands sharing a target directory sequentially. Save long-running output once and inspect its failures, counts and timings; repeat only after relevant changes or unresolved failures. Dependency or cross-cutting changes require the full suite, formatting and Clippy. Documentation-only changes need relevant link/consistency checks and `git diff --check`, not unrelated builds. Before handback inspect `git status --short` and `git diff --check`; distinguish configured checks, actual local passes and hosted qualification.

Use rustfmt, four-space indentation, `snake_case` functions/modules and `PascalCase` types. Prefer finite enums and explicit state transitions over speculative frameworks. Add dependencies only for demonstrated product needs; assess CLI libraries against the pinned argument/exit/help contract rather than adopting their defaults as the contract. Retain [Cargo.lock](Cargo.lock). The host must start without Java, Node, Python or an external Showboat installation.

Never retry an accepted invocation. Keep command failure separate from infrastructure or cleanup failure; shell masking must not make a poisoned session successful. Preserve byte fidelity in transport separately from Markdown representation. Do not promote `spike/1` to the production protocol without the stream, context and ownership qualification in the plans.

## Documentation and evidence

Use one paragraph per physical line and regular Markdown links. Keep indexes as compact tables or task lists. Update the owner of a changed guarantee and affected entry points in the same change; update PLAN when scope, decisions, acceptance or milestone status changes. Keep proposed contracts distinct from supported behavior.

Archive completed detailed plans under `docs/plans/archive/` when needed, preserving evidence and updating links; transfer remaining work to PLAN or [DEFERRED](docs/DEFERRED.md). Do not create duplicate live copies. Add user-visible changes under `Unreleased` in [CHANGELOG](CHANGELOG.md); internal tooling changes do not require an entry.

Retain useful research as concise maintained contracts and source references; do not import entire prototype repositories, raw evidence collections or Git bundles. Record new qualification results with their revision, artifact identities and limitations. Do not transfer synthetic prototype speedups into product performance claims.

Temporary files belong to their invoking command; use scoped temporary directories and clean up only those directories. Keep generated output and local research out of Git. Do not copy LOCmetry-specific dependencies, hooks or analysis rules into this project without a demonstrated need. The owner explicitly requested Homebrew and Linux distribution following LOCmetry's conventions.

Before asking the owner to choose, double-check the recommendation against source and evidence, supply the rationale, and proceed on routine reversible decisions already within scope. Report encountered code smells and material evidence limits without extending the task silently. Follow global Git and commit-message rules; do not create worktrees or push without explicit instructions.

Commit autonomously when a coherent checkpoint passes the checks applicable to its changed behavior. Group related edits into reviewable units rather than committing every small step. Follow the global `F:` exception and honor explicit instructions not to commit.

For recorded user guides, capture actual command output; never invent or hand-edit it. After changing commands, fixtures or recorded behavior, verify affected recordings and independent expected-result assertions. Replay agreement alone does not establish correctness.

The owner authorizes HIGH/MEDIUM subagents where useful for independent checks and reviews. Use the global CLI-specific tier mappings and fresh agents when overriding model/effort; do not duplicate those mappings here. Keep tasks bounded and report relevant findings, including any unresolved issue, before accepting the reviewed milestone.
