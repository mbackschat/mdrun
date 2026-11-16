# Rust dependency choices

These libraries address demonstrated document and session needs; they were not inherited from LOCmetry. The runtime libraries below are compiled into the native host. The complete resolved graph, including development and target-specific dependencies, is locked and subject to the [security quarantine](security.md) before compilation. Runtime SDK/application dependencies are qualified separately; the developing Node and Python SDKs use only their runtimes’ standard libraries.

| Direct library | Adopted version and publication date | Purpose |
| --- | --- | --- |
| clap | 4.6.7, 2026-09-14 | Standard argument parsing, help, version and diagnostics; handwritten CLI parsing is explicitly excluded |
| base64 | 0.23.1, 2026-08-04 | Canonical binary chunk encoding in the bounded SDK wire protocol |
| libc | 0.2.191, 2026-10-16 | POSIX descriptor transfer, signals, process custody and native group observation |
| serde | 1.0.229, 2026-07-18 | Typed binding/protocol validation and lossless native context serialization |
| serde_json | 1.0.154, 2026-10-11 | Correct JSON inventory encoding, including escaped text; avoids a custom JSON serializer |
| time | 0.3.55, 2026-08-01 | UTC timestamp/date construction and RFC3339 output without an external date executable |
| uuid | 1.29.0, 2026-10-23 | Random v4 document identities and copied-image names |
| ureq | 3.5.0, 2026-10-21 | Native HTTP/TLS delivery; rustls enabled, unrelated compression/cookie/JSON features excluded |
| url | 2.5.8, 2026-01-05 | Correct relative redirect resolution for replayable POST bodies; ureq's default 307/308 handling differs from the required behavior |

Publication dates were checked against crates.io on 2026-11-16. The manifest pins these choices and Cargo.lock pins transitives. The initial age audit held back eight transitive releases. Selected `rustls-webpki 0.103.16` includes the fix for [RUSTSEC-2026-0382](https://rustsec.org/advisories/RUSTSEC-2026-0382), and `webpki-roots 1.0.11` includes the fix for [RUSTSEC-2026-0381](https://rustsec.org/advisories/RUSTSEC-2026-0381); the reviewed newer releases did not justify a security exception. No age exception is currently active.

Session work promoted already-locked `base64`, `libc` and `serde` versions to direct dependencies and enabled Serde's existing derive package. The complete graph was rechecked before compilation with zero age, prerelease or yanked-version violations; no registry package version changed in that promotion.

The checked [crate notice bundle](../THIRD_PARTY_NOTICES.txt) preserves upstream notices for the complete locked registry graph, including nested components and packages for other targets. [Distribution](distribution.md) owns its regeneration checks and the remaining exact-toolchain/musl attribution audit. Release qualification measures the final host footprint. “Standalone” means no external Rust/Go/Python/Node/JVM installation is needed for the host, not that it contains no compiled libraries. Showboat is built with Go only for differential qualification; Python scripts are development/test tooling.

## Manual generation

Development-only `clap_mangen 0.3.3` (published 2026-08-12) generates checked manual pages from the same clap command definitions used by the executable; its sole added transitive library is `roff 1.1.1` (2026-03-28). Both were checked against the live registry on 2026-11-16 before compilation, with no yanked/prerelease/age-policy violations across the final graph. Existing locked versions were preserved. The generator and its transitive library are test-only and are not linked into the installed host. Ordinary tests check page freshness; explicit generation and rendering commands are documented in the [usability plan](plans/usability.md).

## JVM SDK development dependency

The developing [JVM SDK](../sdk/jvm/README.md) adopts Jackson core `2.22.4` for streaming JSON parsing/generation, avoiding a handwritten parser and unnecessary databind/annotations dependencies. The [POM](../sdk/jvm/pom.xml) exposes this exact runtime dependency, and the [JVM dependency lock](../sdk/jvm/dependencies.lock.json) records its version, date, source URLs, hash, notices and absence of external runtime dependencies. This dependency belongs to the separately launched Java 21 worker; the Rust host remains standalone and starts without Java or Jackson. This is development adoption, not an SDK release or Maven publication.

The recorded publication date is 2026-10-17. The [preparation helper](../scripts/prepare_jvm.py) checks POM agreement, a conservative 14 full days after publication, the locked jar hash and all five bundled notices before invoking Java 21 tools. Prepared SDK and dependency jars remain separate, with relative classpath paths and hashes in the build manifest. Preparation preserves the dependency jar unchanged, including its Apache and bundled third-party notices.

Historical macOS/Linux qualification independently downloaded the locked artifact from canonical Maven Central HTTPS and matched its SHA-256 before compilation. The lock records `canonical_artifact_verified: true`; this establishes repository-source verification, without claiming a publisher PGP signature. The [history-cleanup qualification note](PLAN.md#qualification-after-history-cleanup) distinguishes that retained narrative from fresh hosted checks.

The dated upstream advisory review found no listed affected range including `2.22.4`; this is not assurance against unknown issues. The preceding `2.22.3` release fixes [error-token growth](https://github.com/FasterXML/jackson-core/security/advisories/GHSA-7hhh-6rmp-j9qf) and [number-regex denial of service](https://github.com/FasterXML/jackson-core/security/advisories/GHSA-p6pp-m3f8-5c89). SDK framing independently enforces its byte limit, decodes UTF-8 strictly before parsing and checks canonical bounded Base64 rather than relying on parser convenience methods for these guarantees.
