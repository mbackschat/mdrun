# Differential compatibility checks

This development-only Python harness runs a finite corpus against independent workspaces for pinned Showboat and mdrun. It compares each command's status, stdout, stderr, all regular file bytes/paths, directories and HTTP form/multipart values/uploads. Raw streams, files and request bodies are retained as base64 JSON for inspection. It does not require or copy the discarded feasibility prototype. Python and Go are qualification tools, not mdrun runtime dependencies.

## Prepare the oracle

Use Go 1.27.4 to build a clean Showboat v0.6.1 checkout, pinned to `d531261b8faf0c388b02c7891d50f1f47c3e2b52`. The preparation script checks both commit/tag and cleanliness, disables CGO and records the build flags/toolchain and resulting SHA-256 beside the binary. The upstream source remains in a disposable checkout outside this repository.

```sh
git clone --branch v0.6.1 --depth 1 https://github.com/simonw/showboat.git /tmp/mdrun-showboat-source
python3 tests/compatibility/prepare.py --source /tmp/mdrun-showboat-source --output /tmp/mdrun-showboat-oracle
```

Output and result directories must be new, so qualification cannot silently replace earlier evidence. `prepare.py` uses the configured Go module cache/proxy; ordinary mdrun tests do not download Go or Showboat.

## Run

```sh
python3 -m unittest discover -s tests/compatibility -p 'test_*.py'
python3 tests/compatibility/oracle.py --oracle /tmp/mdrun-showboat-oracle/showboat --candidate /tmp/mdrun-showboat-oracle/showboat --results /tmp/mdrun-oracle-selfcheck
cargo build --locked
python3 tests/compatibility/oracle.py --oracle /tmp/mdrun-showboat-oracle/showboat --candidate target/debug/mdrun --results /tmp/mdrun-differential
```

Use `--case SUBSTRING` for focused work. A missing case match is an error. A nonzero result means a difference or a harness failure; inspect the saved observations. The bootstrap is expected to fail product comparison. An oracle-to-itself pass checks fixture independence and nondeterminism handling; it is not product compatibility evidence.

The negative controls mutate status, either output stream, file effects and captured HTTP fields, and distinguish whitespace/UUID payloads from metadata. Generated timestamps and UUID-v4 headers are validated and mapped only for explicitly identified new documents; existing legacy values remain exact and generated identity must stay stable through later steps. Generated image names are mapped with their document and upload references. Multipart boundary strings and form-encoding spelling are excluded from semantic comparison, while raw requests remain available. Version/help/extract branding differences remain visible until the product defines and tests their exact intentional projection; never globally scrub product names or UUIDs from command output.

The environment is whitelisted and each case uses a fresh temporary root. Loopback capture needs permission to bind a local socket. Commands run with a deadline and their process group is killed on timeout. These fixtures run trusted test code; the harness is not a sandbox or arbitrary-command isolation service.

The [compatibility plan](../../docs/plans/compatibility.md) owns the source inventory, contract decisions and milestone status. Extend the corpus for a discovered behavior or regression, rather than treating its current case count as a completeness argument.
