# Distribution and releases

Choose an available version from [GitHub Releases](https://github.com/mbackschat/mdrun/releases). This page describes the consumer packages and maintainer publication gates. Non-publishing qualification passes for the frozen 0.1.0 candidate at `bbe19d9`, dated 2026-11-16: native archives, Linux package installations and SDK consumption are qualified. Public downloads and actual Homebrew installation require separate checks against published assets.

The owner selected public GitHub hosting, MIT licensing, Homebrew and Linux packages, and authorized publication on 2026-11-16 after the earlier pre-release hold. An authorized version tag must pass its complete release workflow before GitHub assets are published; Homebrew publication and installation retain the checks below.

| Reader task | Start here |
| --- | --- |
| Check whether an intended package fits your system | [Targets and artifacts](#targets-and-artifacts) |
| Pin the SDK used by your application | [SDK consumption guide](sdk-distribution.md); select a published version and its reviewed digests |
| Inspect the qualified candidate | [Artifact sizes and identities](#candidate-artifact-sizes) |
| Reproduce a package as a maintainer | [Local build and packaging](#build-and-package-locally) |
| Publish an authorized release | [Release gates](#before-the-first-release), then [publication and Homebrew](#publish-and-update-homebrew) |

## Targets and artifacts

Choose a native artifact by operating system and CPU architecture. The host itself needs no separately installed application runtime: Linux packages pass the static-linkage gate, while the macOS executable uses system libraries. Interpreters and cooperative workers used by a document still need their own runtimes.

| Target | Runner | Artifacts |
| --- | --- | --- |
| `aarch64-apple-darwin` | macOS 15 / Apple Silicon | Native `.tar.gz`; Homebrew archive URL |
| `x86_64-unknown-linux-musl` | Ubuntu 24.04 / x86_64 | Static `.tar.gz`, `.deb`, `.rpm`; Homebrew archive URL |
| `aarch64-unknown-linux-musl` | Ubuntu 24.04 ARM64 | Static `.tar.gz`, `.deb`, `.rpm`; Homebrew archive URL |

The initial supported platform baseline is Apple Silicon with macOS 15 or later, and x86_64/ARM64 Linux with Debian 13 or Fedora 43 packages. Ubuntu 24.04 is the native Linux qualification/build host. Older operating systems are outside the qualified matrix. Intel macOS and Windows have no release artifact in this matrix.

### Runtime combinations exercised

The earlier successful runtime jobs at [`f643259`](https://github.com/mbackschat/mdrun/actions/runs/43874321121) on 2026-11-16 observed the following versions. These are concrete tested combinations, not a claim that arbitrary later runtimes or application adapters inherit qualification. Current candidate source and artifact evidence is recorded below and in the [execution checklist](plans/execution.md).

| Native qualification host | JVM | Node | Python |
| --- | --- | --- | --- |
| macOS 15.8.2, arm64 | Temurin 21.0.13+9 | 26.10.0 | CPython 3.15.2 |
| Ubuntu 24.04.5, x86_64 | Temurin 21.0.13+9 | 26.10.0 | CPython 3.12.3 |
| Ubuntu 24.04.5, arm64 | Temurin 21.0.13+9 | 26.10.0 | CPython 3.12.3 |

### How maintainers qualify these artifacts

[CI](../.github/workflows/ci.yml) invokes the reusable [source qualification workflow](../.github/workflows/qualification.yml) on all three native runners. It covers formatting, Clippy, Rust tests, native broker/batch and Node/JVM/Python session controls, bounded streaming, controlling terminals and the pinned Showboat differential. Every checkout uses the triggering `github.sha`.

The [release workflow](../.github/workflows/release.yml) calls the same source qualification workflow. Publication depends on its success in that same run; a historical green branch check cannot qualify a new tag. Version tags, relevant pull requests and manual dispatch trigger release builds, but pull requests and manual dispatch only build/test artifacts. Native builds run on each target architecture. Linux packages are installed, smoke-tested and removed inside Debian and Fedora containers.

## Candidate artifact sizes

All three native builds, SDK consumption and asset assembly in [non-publishing run 43896063039](https://github.com/mbackschat/mdrun/actions/runs/43896063039) pass for source `bbe19d93a4d73ed96cd5d62f901fe05ec285f10b` with Rust 1.98.1. All three native runtime and Showboat differential suites also pass for that exact revision in [source CI 43896061768](https://github.com/mbackschat/mdrun/actions/runs/43896061768). The assembled `release-assets` Actions artifact contains seventeen files. All fifteen `SHA256SUMS` entries, three build identities, attribution hashes and formula contents were independently verified. Each native archive preserves the license, notices and all thirteen maintained man pages. These remain CI artifacts awaiting publication; later documentation edits do not change their recorded source identity.

| Target | Stripped executable bytes | Compressed native archive bytes | Unpacked regular-file bytes |
| --- | ---: | ---: | ---: |
| macOS arm64 | 4,201,904 | 2,537,750 | 22,603,197 |
| Linux x86_64 | 4,998,064 | 2,788,530 | 23,399,357 |
| Linux arm64 | 4,329,864 | 2,639,037 | 22,731,157 |

The artifact workflow's overall conclusion is `cancelled`: after the successful artifact and runtime jobs, its duplicate macOS differential was still waiting for a runner and was cancelled. That exact-revision differential passed in source CI. This does not bypass the complete workflow required for a future release tag.

For release review, use **8 MiB per stripped executable** as the initial size budget. It provides headroom above the measured largest target; revisit it explicitly when product requirements justify growth. The budget covers the host, so record SDK/archive sizes and notice payloads separately.

Unpacked totals sum ordinary file contents and exclude filesystem allocation overhead. The conservative toolchain notice bundle accounts for most of that payload. Independent inspection finds only `libSystem` and `libiconv` dependencies on macOS; both Linux ELF executables have neither an interpreter segment nor dynamic `DT_NEEDED` entries.

The candidate Python wheel is 12,771 bytes, SHA-256 `d7d5d9c47a5f790f0923ff2317c46eb4b3bc76697e30a1aa0b85794375653c12`. Hosted `uv` 0.13.6 installs version 0.1.0 into an isolated application environment and passes all four installation/refusal controls. The JVM, Node and Python archive consumers and the wheel consumer each produce two successful cold calls with zero workers and two successful required-warm calls with one reused worker and clean cleanup through the same run's packaged host. Actual versioned public-download URLs remain a post-publication check.

## Before the first release

Publication is authorized. Complete these gates and retain exact-revision evidence; authorization and historical candidate checks do not establish a successful release.

1. Complete the applicable [product qualification](PLAN.md) and retain passing compatibility/runtime/platform evidence before claiming supported warm execution or migration readiness.
2. Pass the document smoke scenario against extracted archives, installed Linux packages and Homebrew on supported targets. Local helper success and configured CI are not installation evidence.
3. Pass the checked crate and [toolchain attribution](../packaging/toolchain.json) gates. Toolchain attribution verifies the exact compiler source identity and upstream runtime notices before packaging; crate notices alone do not satisfy it. Retain credit for Showboat's idea in the README.
4. Confirm the release tag matches `Cargo.toml` and `Cargo.lock`, record the version's changes in the changelog, and verify the current commit's CI and packaging dry run.
5. Review per-target build records against the executable size budget; retain archive/notice sizes separately.
6. Check that the GitHub author email is `mbackschat@mail.de` before pushing. A version tag is the explicit publication trigger; manual dispatch never publishes.

## Build and package locally

This workflow is for maintainers producing candidate packages. Consumers do not need Rust, Python, Docker or package-building tools to run the installed host. The helper scripts require Python 3.11+ for development only.

### Create a native archive

Install a supported native target in the Rust toolchain, build it, and generate its toolchain attribution before creating the archive. For example, on Apple Silicon:

```sh
rustup toolchain install 1.98.1 --profile minimal
rustup run 1.98.1 cargo build --release --locked --target aarch64-apple-darwin
rustup run 1.98.1 python3 packaging/toolchain.py generate
python3 packaging/distribution.py archive aarch64-apple-darwin dist
python3 -m unittest discover -s packaging -p 'test_*.py'
```

Archives include the binary, README, changelog, MIT license, [third-party crate notices](../THIRD_PARTY_NOTICES.txt), generated `TOOLCHAIN_NOTICES.txt` and every generated [man page](man/) under `share/man/man1/`. The archive helper requires `mdrun.1` and refuses missing inputs, unsupported targets, invalid versions and an existing destination. Cargo's Linux package metadata lives in [Cargo.toml](../Cargo.toml); the release workflow pins the package-building tools and checks actual installation.

### Check installed documentation

Homebrew installs the root and command pages through `man1`; Linux packages install them under `/usr/share/man/man1/`. Use `man mdrun` or, for example, `man mdrun-session` after installation. For an extracted archive, add its `share/man` directory to your man-page search path or open a page directly with `man /absolute/archive/share/man/man1/mdrun.1`. Linux installation checks require every generated page, accepting the package builder's compressed `.1.gz` form as well as `.1`.

Installation checks account for minimal-image policies without weakening the required assets:

| Fixture | Required adjustment and boundary |
| --- | --- |
| Debian slim | Its default dpkg policy discards almost all manuals and documentation, including custom notice files. The qualifier overrides the filter for mdrun's paths. User systems that deliberately exclude documentation must adjust their own policy to install those files. |
| Fedora | Its minimal image omits `cmp`, which the smoke test uses to verify document preservation. The fixture installs `diffutils`; it is a test prerequisite, not an mdrun package dependency. |

Actual Debian and Fedora installation, standalone smoke, notices/manuals and removal pass for both Linux architectures in the candidate run. Failed install checks retain built native artifacts for diagnosis; publication still requires the complete job to succeed.

### Regenerate crate notices after dependency changes

The [notice helper](../packaging/notices.py) preserves upstream license/notice texts from checksum-verified crate archives, including nested component notices. Its checked bundle conservatively covers the complete lockfile, including build-only and other-target dependencies; it does not claim every listed crate is linked. CI checks the inventory against Cargo.lock and the bundle hash.

When dependencies change, generate fresh full Cargo metadata, then run `python3 packaging/notices.py generate --metadata /path/to/metadata.json` and review the selected notice files and license declarations. Run `check --metadata /path/to/metadata.json` to additionally reproduce the bundle from cached upstream archives. Toolchain/runtime attribution has its own inputs and gate below.

### Verify the exact toolchain and linked runtime

The project declares Rust 1.98 as its minimum supported Rust version and qualifies builds with the exact Rust 1.98.1 patch release (`48a229ceaefd4985c50990b14116b6d856af0985`); this is an accepted build baseline, not a claim that every older compiler fails. The patch fixes the upstream [vtable miscompilation](https://doc.rust-lang.org/releases.html#version-1981-2026-09-03). CI and release builds pin it rather than following `stable`.

[Toolchain attribution](../packaging/toolchain.py) verifies `rustc -Vv` and hashes of the official installed Rust copyright documents/license texts against the [audited inventory](../packaging/toolchain.json). The generated `TOOLCHAIN_NOTICES.txt` stays outside Git and is required in each native archive and installed package; stable Homebrew installs retain it too.

The bundle preserves complete upstream standard-library and toolchain HTML documents verbatim, their license texts, the additional [compiler-builtins license](https://github.com/rust-lang/rust/blob/48a229ceaefd4985c50990b14116b6d856af0985/library/compiler-builtins/LICENSE.txt), and musl copyright/source notice blocks, including architecture-specific Arm and math notices. It conservatively includes build-only and other-target entries without asserting that all are linked. This follows Rust's [distribution copyright contract](https://github.com/rust-lang/rust/blob/48a229ceaefd4985c50990b14116b6d856af0985/COPYRIGHT).

Linux attribution uses musl 1.2.5 from the original checksum-pinned release archive. The pinned Rust [build recipe](https://github.com/rust-lang/rust/blob/48a229ceaefd4985c50990b14116b6d856af0985/src/ci/docker/scripts/musl-toolchain.sh) uses that version with its recorded musl-cross-make revision and security patches. Reproduce retained notices with `python3 packaging/toolchain.py check --musl-archive /path/to/musl-1.2.5.tar.gz` under Rust 1.98.1. A compiler, upstream-notice or musl-source change requires explicit inventory review; mismatches stop packaging.

Each published `mdrun-build-TARGET.json` must tie the source revision, compiler identity, executable digest, attribution digest and installed target standard-library/runtime input hashes to the released asset. These hashes identify supplied inputs conservatively; they do not establish linker reachability.

Linux builds install Ubuntu `musl-tools` and select `musl-gcc` for target C/assembly compilation, including the TLS library. The final link explicitly uses Rust's self-contained runtime; a linker-map guard rejects libc archives outside the audited Rust sysroot. Build records retain the C compiler/package versions separately from linked runtime hashes. Installing the Rust target alone does not install that C compiler, as the initial release dry run demonstrated on both Linux architectures.

### Verify behavior through the installed executable

The shared [package smoke scenario](../packaging/smoke.sh) runs the installed executable with an empty environment and PATH, using explicit system interpreter/command paths for the recorded program. It creates a document, adds a note, records literal output and verifies it; changed input must then fail verification without editing the document. An independent guard rejects a version-only stub.

The release workflow passes this scenario against each candidate archive and installed Linux package. Homebrew's test implements the same author/replay/mismatch checks; record actual Homebrew installation separately after the release assets are available.

## Publish and update Homebrew

An authorized version tag triggers publication; all release builds must pass before GitHub publication. The workflow publishes three native archives, both Linux package formats for both architectures, Node/JVM/Python SDK archives, a standard Python wheel, the standalone `prepare_sdk.py` helper, per-target build identity records, `SHA256SUMS` and a rendered `mdrun.rb` formula. It does not modify another repository or require tap credentials in this repository.

SDK consumption is a separate release gate. Qualification installs each archive by explicit version and SHA-256 into an isolated directory outside the checkout, then runs an independent cold application and actual cold/required-warm sessions through the packaged host. The Python wheel additionally passes `uv` installation and hash-refusal controls, plus actual cold/required-warm sessions from an isolated application environment. The [SDK consumption guide](sdk-distribution.md) explains pinned local and CI preparation. SDK registry publication remains deferred.

The [Homebrew template](../packaging/homebrew/mdrun.rb) uses prebuilt archives for stable installs and Rust only for `--HEAD` builds. The [distribution helper](../packaging/distribution.py) requires exactly one valid digest for every native archive before rendering it. For an authorized, qualified release:

1. Copy the generated `mdrun.rb` into `Formula/mdrun.rb` in `mbackschat/homebrew-tap`.
2. Review its version, URLs and digests.
3. Run `brew audit --strict mbackschat/tap/mdrun`, `brew install mbackschat/tap/mdrun` and `brew test mbackschat/tap/mdrun` on supported systems.
4. Publish the tap update only after those checks pass.

For a published formula, install with `brew install mbackschat/tap/mdrun`. Linux users may alternatively download the matching archive or install the `.deb`/`.rpm` package from an available [GitHub release](https://github.com/mbackschat/mdrun/releases). Verify the selected release's checksums before installation.

## Name and registry scope

The owner explicitly retained `mdrun` on 2026-11-16 after checking that another Rust Markdown tool uses the name. The crates.io name is occupied; Cargo registry publication remains disabled. GitHub ownership and the personal Homebrew tap provide scoped distribution names. PyPI and npm also have unrelated `mdrun` packages; installation instructions must point to this repository/tap rather than those registries. This decision does not claim exclusive ownership of the generic command name.
