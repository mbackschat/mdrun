# Distribution and releases

Status: final artifact qualification for the frozen 0.1.0 release candidate is in progress, 2026-11-16. The owner selected public GitHub hosting, MIT licensing, Homebrew and Linux packages, and requested a stop before publication. No product version has been released. The version-1 cooperative session/SDK contract has passed its freeze review and native lifecycle qualification; installed-package checks and the held publication gates remain separate.

## Targets and artifacts

| Target | Runner | Artifacts |
| --- | --- | --- |
| `aarch64-apple-darwin` | macOS 15 / Apple Silicon | Native `.tar.gz`; Homebrew archive URL |
| `x86_64-unknown-linux-musl` | Ubuntu 24.04 / x86_64 | Static `.tar.gz`, `.deb`, `.rpm`; Homebrew archive URL |
| `aarch64-unknown-linux-musl` | Ubuntu 24.04 ARM64 | Static `.tar.gz`, `.deb`, `.rpm`; Homebrew archive URL |

The initial supported platform baseline is Apple Silicon with macOS 15 or later, and x86_64/ARM64 Linux with Debian 13 or Fedora 43 packages. Ubuntu 24.04 is the native Linux qualification/build host. Older operating systems are outside the qualified matrix; broader full-product qualification remains M6 work. Intel macOS and Windows have no release artifact in this matrix. Linux packages declare no separately installed application runtime: their packaged binary must pass the static-linkage gate. The macOS executable uses system libraries.

The successful runtime jobs at [`7ffcf4d`](https://github.com/mbackschat/mdrun/actions/runs/43862729035) on 2026-11-16 observed the following versions. These are concrete tested combinations, not a claim that arbitrary later runtimes or application adapters inherit qualification. Installed-package and final candidate checks are tracked separately below and in the [execution checklist](plans/execution.md).

| Native qualification host | JVM | Node | Python |
| --- | --- | --- | --- |
| macOS 15.8.2, arm64 | Temurin 21.0.13+9 | 26.10.0 | CPython 3.15.2 |
| Ubuntu 24.04.5, x86_64 | Temurin 21.0.13+9 | 26.10.0 | CPython 3.12.3 |
| Ubuntu 24.04.5, arm64 | Temurin 21.0.13+9 | 26.10.0 | CPython 3.12.3 |

[CI](../.github/workflows/ci.yml) invokes the reusable [source qualification workflow](../.github/workflows/qualification.yml), covering formatting, Clippy, Rust tests, native broker/batch and Node/JVM/Python session controls, bounded streaming, controlling terminals and the pinned Showboat differential on all three native runners. Every checkout uses the triggering `github.sha`. The release workflow calls this same workflow and publication explicitly depends on its success in the same run; a historical green branch check cannot qualify a new tag. [Release builds](../.github/workflows/release.yml) run on version tags, relevant pull requests and manual dispatch. Pull requests and manual dispatch build/test artifacts without publishing. Native builds run on each target architecture, and Linux packages are installed, smoke-tested and removed inside Debian and Fedora containers.

## Before the first release

- Complete the applicable [product qualification](PLAN.md) and retain passing compatibility/runtime/platform evidence before claiming supported warm execution or migration readiness.
- Pass the document smoke scenario against extracted archives, installed Linux packages and Homebrew on supported targets. Local helper success and configured CI are not installation evidence.
- Pass the checked crate and [toolchain attribution](../packaging/toolchain.json) gates. The latter verifies the exact compiler source identity and upstream runtime notices before packaging; crate notices alone do not satisfy it. Credit for Showboat's idea remains in the README.
- Confirm the release tag matches `Cargo.toml` and `Cargo.lock`, record the version's changes in the changelog, and verify the current commit's CI and packaging dry run.
- Check GitHub author email is `mbackschat@mail.de` before pushing. A version tag is the explicit publication trigger; manual dispatch never publishes.

## Build and package locally

The helper scripts require Python 3.11+ for development only. Build a supported native target installed in the Rust toolchain, then create its archive, for example on Apple Silicon:

```sh
rustup toolchain install 1.98.1 --profile minimal
rustup run 1.98.1 cargo build --release --locked --target aarch64-apple-darwin
rustup run 1.98.1 python3 packaging/toolchain.py generate
python3 packaging/distribution.py archive aarch64-apple-darwin dist
python3 -m unittest discover -s packaging -p 'test_*.py'
```

Archives include the binary, README, changelog, MIT license, [third-party crate notices](../THIRD_PARTY_NOTICES.txt), generated `TOOLCHAIN_NOTICES.txt` and every generated [man page](man/) under `share/man/man1/`. The archive helper requires `mdrun.1` and refuses missing inputs, unsupported targets, invalid versions and an existing destination. Cargo's Linux package metadata lives in [Cargo.toml](../Cargo.toml); the release workflow pins the package-building tools and checks actual installation. Rust is a build dependency; Python, Docker and packaging tools are development/CI dependencies, not dependencies of the installed host.

Homebrew installs the root and command pages through `man1`; Linux packages install them under `/usr/share/man/man1/`. Use `man mdrun` or, for example, `man mdrun-session` after installation. For an extracted archive, add its `share/man` directory to your man-page search path or open a page directly with `man /absolute/archive/share/man/man1/mdrun.1`. Linux installation checks require every generated page, accepting the package builder's compressed `.1.gz` form as well as `.1`.

The disposable Debian slim installation fixture explicitly includes mdrun's documentation when installing the package: that image's default dpkg policy discards almost all manuals and documentation, including custom notice files. The qualifier overrides that filter for mdrun's paths instead of weakening the asset checks. User systems that deliberately exclude documentation must adjust their own policy to install those files. Built native artifacts are retained after a failed install check for diagnosis; publication still requires the complete job to succeed.

The [notice helper](../packaging/notices.py) preserves upstream license/notice texts from checksum-verified crate archives, including nested component notices. Its checked bundle conservatively covers the complete lockfile, including build-only and other-target dependencies; it does not claim every listed crate is linked. CI checks the inventory against Cargo.lock and the bundle hash. When dependencies change, generate fresh full Cargo metadata, regenerate the bundle with `python3 packaging/notices.py generate --metadata /path/to/metadata.json`, and review the selected notice files and license declarations. `check --metadata /path/to/metadata.json` additionally reproduces the bundle from the cached upstream archives. Toolchain/runtime attribution is generated separately as described below.

The project declares Rust 1.98 as its minimum supported Rust version and qualifies builds with the exact Rust 1.98.1 patch release (`48a229ceaefd4985c50990b14116b6d856af0985`); this is an accepted build baseline, not a claim that every older compiler fails. The patch fixes the upstream [vtable miscompilation](https://doc.rust-lang.org/releases.html#version-1981-2026-09-03). CI and release builds pin it rather than following `stable`.

[Toolchain attribution](../packaging/toolchain.py) verifies `rustc -Vv` and hashes of the official installed Rust copyright documents/license texts against the [audited inventory](../packaging/toolchain.json). It preserves the complete upstream standard-library and toolchain HTML documents verbatim, their license texts, the additional [compiler-builtins license](https://github.com/rust-lang/rust/blob/48a229ceaefd4985c50990b14116b6d856af0985/library/compiler-builtins/LICENSE.txt), and musl copyright/source notice blocks, including architecture-specific Arm and math notices. The conservative bundle includes build-only and other-target entries; it does not assert that all are linked. This follows Rust's [distribution copyright contract](https://github.com/rust-lang/rust/blob/48a229ceaefd4985c50990b14116b6d856af0985/COPYRIGHT). Generated `TOOLCHAIN_NOTICES.txt` stays outside Git and is required in each native archive and installed package; stable Homebrew installs retain it too.

Linux attribution uses musl 1.2.5 from the original checksum-pinned release archive. The pinned Rust [build recipe](https://github.com/rust-lang/rust/blob/48a229ceaefd4985c50990b14116b6d856af0985/src/ci/docker/scripts/musl-toolchain.sh) uses that version with its recorded musl-cross-make revision and security patches. Reproduce its retained notices with `python3 packaging/toolchain.py check --musl-archive /path/to/musl-1.2.5.tar.gz` under Rust 1.98.1. Changing the compiler, upstream notices or musl source requires explicit inventory review; mismatches stop packaging. Each published `mdrun-build-TARGET.json` ties the source revision, compiler identity, executable digest, attribution digest and installed target standard-library/runtime input hashes to the released asset. These hashes identify supplied inputs conservatively, not a linker reachability analysis.

Linux builds install Ubuntu `musl-tools` and select `musl-gcc` for target C/assembly compilation, including the TLS library. The final link explicitly uses Rust's self-contained runtime; a linker-map guard rejects libc archives outside the audited Rust sysroot. Build records retain the C compiler/package versions separately from linked runtime hashes. Installing the Rust target alone does not install that C compiler, as the initial release dry run demonstrated on both Linux architectures.

The shared [package smoke scenario](../packaging/smoke.sh) runs the installed executable with an empty environment and PATH, using explicit system interpreter/command paths for the recorded program. It creates a document, adds a note, records literal output and verifies it; changed input must then fail verification without editing the document. A version-only stub is rejected by the independent guard. The release workflow applies this scenario to each extracted archive and installed Linux package; Homebrew's test performs the same author/replay/mismatch checks. The local guard passes against a separately built host; cross-target installation of this extension remains pending.

## Publish and update Homebrew

Once an approved version tag is pushed, all release builds must pass before GitHub publication. The workflow publishes three native archives, both Linux package formats for both architectures, Node/JVM/Python SDK archives, a standard Python wheel, the standalone `prepare_sdk.py` helper, per-target build identity records, `SHA256SUMS` and a rendered `mdrun.rb` formula. SDK qualification installs each archive by an explicit version and SHA-256 into an isolated directory outside the checkout, then runs an independent cold application and actual cold/required-warm sessions through the released host. The Python wheel additionally passes `uv` installation and hash-refusal controls plus actual cold/required-warm sessions from an isolated application environment. The [SDK consumption guide](sdk-distribution.md) explains pinned local and CI preparation. SDK registry publication remains deferred. It does not modify another repository or require tap credentials in this repository.

The [Homebrew template](../packaging/homebrew/mdrun.rb) uses prebuilt archives for stable installs and Rust only for `--HEAD` builds. The [distribution helper](../packaging/distribution.py) requires exactly one valid digest for every native archive before rendering it. For a qualified release, copy its generated `mdrun.rb` into `Formula/mdrun.rb` in `mbackschat/homebrew-tap`, review the version/URLs/digests, and run `brew audit --strict mbackschat/tap/mdrun`, `brew install mbackschat/tap/mdrun` and `brew test mbackschat/tap/mdrun` on supported systems before publishing the tap update.

After the formula is published, the intended install command is `brew install mbackschat/tap/mdrun`. Linux users may alternatively download the matching archive or install the `.deb`/`.rpm` release package. No Homebrew formula or product release has been published yet.

## Name and registry scope

The owner explicitly retained `mdrun` on 2026-11-16 after checking that another Rust Markdown tool uses the name. The crates.io name is occupied; Cargo registry publication remains disabled. GitHub ownership and the personal Homebrew tap provide scoped distribution names. PyPI and npm also have unrelated `mdrun` packages; installation instructions must point to this repository/tap rather than those registries. This decision does not claim exclusive ownership of the generic command name.
