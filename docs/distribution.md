# Distribution and releases

Status: release infrastructure, 2026-11-16. The owner selected public GitHub hosting, MIT licensing, Homebrew and Linux packages. No product version has been released. The executable remains a bootstrap; a successful packaging dry run does not establish document compatibility or production runtime guarantees.

## Targets and artifacts

| Target | Runner | Artifacts |
| --- | --- | --- |
| `aarch64-apple-darwin` | macOS 15 / Apple Silicon | Native `.tar.gz`; Homebrew archive URL |
| `x86_64-unknown-linux-musl` | Ubuntu 24.04 / x86_64 | Static `.tar.gz`, `.deb`, `.rpm`; Homebrew archive URL |
| `aarch64-unknown-linux-musl` | Ubuntu 24.04 ARM64 | Static `.tar.gz`, `.deb`, `.rpm`; Homebrew archive URL |

These are initial build targets, following the macOS/Linux portion of LOCmetry's distribution. Minimum supported OS versions and full-product qualification remain M6 work. Intel macOS and Windows have no release artifact in this matrix. Linux packages declare no separately installed application runtime: their packaged binary must pass the static-linkage gate. The macOS executable uses system libraries.

[CI](../.github/workflows/ci.yml) checks formatting, Clippy, Rust tests, release compilation, packaging helpers, contributor symlink and research integrity. [Release builds](../.github/workflows/release.yml) run on version tags, relevant pull requests and manual dispatch. Pull requests and manual dispatch build/test artifacts without publishing. Native builds run on each target architecture, and Linux packages are installed, smoke-tested and removed inside Debian and Fedora containers.

## Before the first release

- Complete the applicable [product qualification](PLAN.md) and retain passing compatibility/runtime/platform evidence. Do not label this bootstrap as a working Showboat replacement.
- Replace bootstrap-only package smoke checks with a small real document author/verify scenario when M2 implements it.
- Audit newly added compiled dependencies and include any required third-party notices in archives, Linux packages and Homebrew. The bootstrap has no third-party crates; credit for Showboat's idea is already in the README.
- Confirm the release tag matches `Cargo.toml` and `Cargo.lock`, record the version's changes in the changelog, and verify the current commit's CI and packaging dry run.
- Check GitHub author email is `mbackschat@mail.de` before pushing. A version tag is the explicit publication trigger; manual dispatch never publishes.

## Build and package locally

The helper scripts require Python 3.11+ for development only. Build a supported native target installed in the Rust toolchain, then create its archive, for example on Apple Silicon:

```sh
cargo build --release --locked --target aarch64-apple-darwin
python3 packaging/distribution.py archive aarch64-apple-darwin dist
python3 -m unittest discover -s packaging -p 'test_*.py'
```

Archives include the binary, README, changelog and MIT license. The archive helper refuses missing inputs, unsupported targets, invalid versions and an existing destination. Cargo's Linux package metadata lives in [Cargo.toml](../Cargo.toml); the release workflow pins the package-building tools and checks actual installation. Rust is a build dependency; Python, Docker and packaging tools are development/CI dependencies, not dependencies of the installed host.

## Publish and update Homebrew

Once an approved version tag is pushed, all release builds must pass before GitHub publication. The workflow publishes three native archives, both Linux package formats for both architectures, `SHA256SUMS` and a rendered `mdrun.rb` formula. It does not modify another repository or require tap credentials in this repository.

The [Homebrew template](../packaging/homebrew/mdrun.rb) uses prebuilt archives for stable installs and Rust only for `--HEAD` builds. The [distribution helper](../packaging/distribution.py) requires exactly one valid digest for every native archive before rendering it. For a qualified release, copy its generated `mdrun.rb` into `Formula/mdrun.rb` in `mbackschat/homebrew-tap`, review the version/URLs/digests, and run `brew audit --strict mbackschat/tap/mdrun`, `brew install mbackschat/tap/mdrun` and `brew test mbackschat/tap/mdrun` on supported systems before publishing the tap update.

After the formula is published, the intended install command is `brew install mbackschat/tap/mdrun`. Linux users may alternatively download the matching archive or install the `.deb`/`.rpm` release package. No Homebrew formula or product release has been published by the repository bootstrap itself.

## Name and registry scope

The owner explicitly retained `mdrun` on 2026-11-16 after checking that another Rust Markdown tool uses the name. The crates.io name is occupied; Cargo registry publication remains disabled. GitHub ownership and the personal Homebrew tap provide scoped distribution names. PyPI and npm also have unrelated `mdrun` packages; installation instructions must point to this repository/tap rather than those registries. This decision does not claim exclusive ownership of the generic command name.
