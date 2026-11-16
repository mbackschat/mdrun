# Dependency and repository security

Review a dependency change before compiling it: Rust build scripts and procedural macros can execute during tooling as well as during a normal build. This page gives maintainers the adoption checks, the narrow exception for an urgent security fix, and the repository controls that detect known vulnerabilities and leaked credentials.

The owner adopted the policy on 2026-11-16. Routine Rust dependencies must be stable, non-yanked releases published at least 14 days before adoption, including every resolved transitive dependency. Security alerts and security-fix pull requests remain immediate. Every security update requires review; none is automatically merged.

## Dependency quarantine

For a new or updated Rust dependency, run the standard-library-only [policy guard](../scripts/dependency_policy.py) before compiling any dependency code:

```sh
python3 scripts/dependency_policy.py
```

The guard checks every registry version in [Cargo.lock](../Cargo.lock) against crates.io publication dates and yank status. It rejects prereleases and unreviewed dependency sources, and fails if registry evidence cannot be obtained. CI, release builds and Rust CodeQL extraction run it before dependency build code can execute. A version proposed by an update tool still needs this complete-lockfile check: dependency resolution and security fixes can introduce versions beyond the update being proposed.

[Dependabot](../.github/dependabot.yml) checks Cargo, GitHub Actions and the JVM SDK's Maven manifest weekly. Cargo updates cover direct and transitive dependencies. Cargo and Maven version updates use a 14-day cooldown; GitHub documents that [cooldown applies to version updates, not security updates](https://docs.github.com/en/code-security/reference/supply-chain-security/dependabot-options-reference#cooldown). The cooldown does not replace the guard. A JVM update also needs an explicitly reviewed matching [SDK lock](../sdk/jvm/dependencies.lock.json); a POM-only update fails preparation before compilation.

### Review an urgent security-fix exception

If a verified security fix cannot wait for quarantine, use an exact crate-and-version exception in [dependency-exceptions.json](../.github/dependency-exceptions.json):

1. Review the advisory's affected versions and the upstream fix.
2. Record the advisory URL, rationale, reviewed upstream change URL, relevant test evidence and expiry date.
3. Run the applicable checks before merging.
4. Remove the exception when it expires or is superseded.

An exception waives only the age threshold. Prerelease and yank checks still apply. Do not exempt security pull requests as a class or ignore a vulnerability to pass a check. The [dependency inventory](dependencies.md) records the adopted libraries and their evidence; [distribution](distribution.md#regenerate-crate-notices-after-dependency-changes) covers the associated notice update.

## Automated checks

Use these controls together: each detects a different class of problem and has a narrower boundary than a general safety guarantee.

| Control | Coverage and boundary |
| --- | --- |
| [CodeQL](../.github/workflows/codeql.yml) | Analyzes Rust, Python, JavaScript and Java on pull requests, pushes to `main`, a weekly schedule and manual dispatch. Rust uses the supported `none` build mode; [extraction still executes build scripts and procedural macros](https://docs.github.com/en/code-security/reference/code-scanning/codeql/build-options-for-compiled-languages#building-rust), so the quarantine guard runs first. Java uses source analysis without a compilation step. |
| [Dependency review](../.github/workflows/dependency-review.yml) | Rejects pull requests introducing known vulnerabilities of any severity in runtime, development or unknown scopes. GitHub's [dependency review](https://docs.github.com/en/code-security/concepts/supply-chain-security/dependency-review) includes lockfile transitives; it detects known vulnerable dependencies, not arbitrary malicious packages. |
| Dependabot alerts and security updates | Alerts on known vulnerabilities in the repository dependency graph and proposes fixes immediately, including outside the weekly version-update schedule. |
| Secret scanning and push protection | Detects supported secret patterns and blocks matching pushes; rotate a leaked credential when an alert identifies it. |

Workflow files configure checks; passing hosted runs establish that they actually execute. Require the dependency-review check in repository branch protection if it must block merges. No automatic merge rule is configured here.

## Repository settings

Repository administrators enable the account-side controls under **Settings → Advanced Security**. Keep the dependency graph, Dependabot alerts, Dependabot security updates, secret scanning and push protection enabled. Use the committed CodeQL workflow as advanced setup; do not configure a duplicate default CodeQL setup.

For automation, use GitHub's [repository API](https://docs.github.com/en/rest/repos/repos):

| Control | Request |
| --- | --- |
| Dependency alerts and graph | `PUT /repos/{owner}/{repo}/vulnerability-alerts` |
| Security-fix pull requests | `PUT /repos/{owner}/{repo}/automated-security-fixes` |
| Secret scanning | `PATCH /repos/{owner}/{repo}` with `security_and_analysis.secret_scanning.status` set to `enabled` |
| Secret-scanning push protection | `PATCH /repos/{owner}/{repo}` with `security_and_analysis.secret_scanning_push_protection.status` set to `enabled` |

Read settings back after changes. Committed configuration files alone do not enable these repository controls.
