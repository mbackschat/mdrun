# Dependency and repository security

The owner adopted this policy on 2026-11-16: routine Rust dependencies must be stable, non-yanked releases published at least 14 days before adoption, including every resolved transitive dependency. Security alerts and security-fix pull requests remain immediate. Security updates require review and are never automatically merged.

## Dependency quarantine

Run `python3 scripts/dependency_policy.py` before compiling new or updated dependencies. The standard-library-only [guard](../scripts/dependency_policy.py) checks every registry version in [Cargo.lock](../Cargo.lock) against crates.io publication dates and yank status, rejects prereleases and unreviewed dependency sources, and fails if registry evidence cannot be obtained. CI, release builds and Rust CodeQL extraction run it before dependency build code can execute.

[Dependabot](../.github/dependabot.yml) checks Cargo and GitHub Actions weekly. Cargo updates include direct and transitive dependencies and use a 14-day cooldown. GitHub documents that [cooldown applies to version updates, not security updates](https://docs.github.com/en/code-security/reference/supply-chain-security/dependabot-options-reference#cooldown). It does not replace auditing the complete resolved lockfile: dependency resolution and security updates can introduce versions beyond the update being proposed.

For a verified security fix that cannot wait for quarantine, record an exception for the exact crate and version in [dependency-exceptions.json](../.github/dependency-exceptions.json), including the advisory URL, rationale, reviewed upstream change URL, relevant test evidence and expiry date. Review affected versions and the upstream fix before accepting the exception; use the applicable checks before merging. Exceptions waive only the age threshold, never prerelease or yank checks. Do not add a blanket exemption for security pull requests or ignore a vulnerability merely to pass a check. Remove expired or superseded exceptions.

## Automated checks

| Control | Coverage and boundary |
| --- | --- |
| [CodeQL](../.github/workflows/codeql.yml) | Analyzes Rust and repository Python on pull requests, pushes to `main`, a weekly schedule and manual dispatch. Rust uses the supported `none` build mode; [extraction still executes build scripts and procedural macros](https://docs.github.com/en/code-security/reference/code-scanning/codeql/build-options-for-compiled-languages#building-rust), so the quarantine guard runs first. |
| [Dependency review](../.github/workflows/dependency-review.yml) | Rejects pull requests introducing known vulnerabilities of any severity in runtime, development or unknown scopes. GitHub's [dependency review](https://docs.github.com/en/code-security/concepts/supply-chain-security/dependency-review) includes lockfile transitives; it detects known vulnerable dependencies, not arbitrary malicious packages. |
| Dependabot alerts and security updates | Alerts on known vulnerabilities in the repository dependency graph and proposes fixes immediately, including outside the weekly version-update schedule. |
| Secret scanning and push protection | Detects supported secret patterns and blocks matching pushes; rotate a leaked credential when an alert identifies it. |

The workflow files configure checks; passing hosted runs establish that they actually execute. Require the dependency-review check in repository branch protection if it must block merges. No automatic merge rule is configured here.

## Repository settings

Repository administrators manage these controls under **Settings → Advanced Security**. Keep the dependency graph, Dependabot alerts, Dependabot security updates, secret scanning and push protection enabled. Use the committed CodeQL workflow as advanced setup; do not configure a duplicate default CodeQL setup.

For automation, GitHub's [repository API](https://docs.github.com/en/rest/repos/repos) enables dependency alerts and the graph with `PUT /repos/{owner}/{repo}/vulnerability-alerts`, and security-fix pull requests with `PUT /repos/{owner}/{repo}/automated-security-fixes`. `PATCH /repos/{owner}/{repo}` accepts `security_and_analysis.secret_scanning.status` and `security_and_analysis.secret_scanning_push_protection.status` as `enabled`. Read the settings back after changes; configuration files alone do not enable those repository controls.
