# Research and provenance

The [showboat-like-tool snapshot](showboat-like-tool/README.md) contains all 95 tracked files from source commit `9dc0888eb90d626522b1268c47a9a53463ae03ac`, imported byte for byte on 2026-11-16. This includes hidden configuration, Go/Rust hosts, JVM/Node/Python workers, tests, scripts, design documents and raw evidence. The source working tree was clean and had no untracked non-ignored files at import.

The [Git bundle](showboat-like-tool.bundle) preserves the complete reachable source history and refs, independently of the original folder. [SHA-256 checksums](showboat-like-tool.sha256) cover the snapshot and bundle. Generated `out/`, Rust build output, Python caches and `.DS_Store` were omitted; they are disposable and the tracked source can regenerate build outputs. Source `.git` configuration, reflogs and hooks are not part of the bundle or needed for the project.

| Material | Use |
| --- | --- |
| [Decisions/handoff](showboat-like-tool/docs/09-NEXT-SESSION.md) | Accepted name, Rust direction, compatibility goal and runtime order |
| [Design](showboat-like-tool/docs/02-DESIGN.md) | Generic host/SDK/application boundary and invariants |
| [Prototype contract](showboat-like-tool/docs/06-PROTOTYPE.md) | Exact experimental behavior and evidence limits |
| [JVM/Node results](showboat-like-tool/docs/07-RESULTS.md) | Historical footprint/timing results and raw evidence links |
| [Python extension](showboat-like-tool/docs/08-PYTHON.md) | Third runtime through the unchanged prototype host |
| [Sources](showboat-like-tool/docs/05-SOURCES.md) | Original upstream pins and reference provenance |

## Integrity and recovery

From the mdrun repository root:

```sh
shasum -a 256 -c docs/research/showboat-like-tool.sha256
git bundle verify docs/research/showboat-like-tool.bundle
```

Linux can use `sha256sum --check docs/research/showboat-like-tool.sha256`. To reproduce the prototype, clone the bundle into a new disposable directory, then follow that clone's README. For example, from the mdrun root, `git clone docs/research/showboat-like-tool.bundle /tmp/mdrun-prototype-reproduction` works when the destination does not exist. The original Downloads folder is not needed. The harness may overwrite its own evidence files; never run it in the immutable snapshot.

LOCmetry at `d96eb6369484c1e8f60e65d1b65a41b64485629b` supplied repository organization and distribution conventions only. Its dependencies and product architecture were not copied. mdrun's product direction comes from the supplied research and owner instructions.

## Known prototype limitations

The prototype's experimental binary protocol buffers stdin in a bounded request frame; this cannot satisfy the production requirement for lazy and arbitrarily large streams. Its POSIX supervision does not qualify abrupt owner loss or all production cleanup requirements. Its build/check harness can overwrite historical evidence names. These existing design limitations are preserved for reference and must not become production assumptions. The active plans require independent packaged-cold parity and ownership/stream qualification before reuse.
