# Rust dependency choices

These libraries address demonstrated document and session needs; they were not inherited from LOCmetry. All are compiled into the native host. The complete resolved graph, including target-specific dependencies, is locked and subject to the [security quarantine](security.md) before compilation. Runtime SDK/application dependencies are qualified separately; the developing Node SDK uses only built-in modules.

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

Release qualification measures the final host footprint and audits transitive licenses. “Standalone” means no external Rust/Go/Python/Node/JVM installation is needed for the host, not that it contains no compiled libraries. Showboat is built with Go only for differential qualification; Python scripts are development/test tooling.
