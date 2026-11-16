# Prepare a versioned runtime SDK

Prepare an SDK when you are adapting a JVM, Node or Python CLI for repeated warm calls from executable Markdown. Ordinary execution starts the application's process each time. A cooperative worker keeps reusable initialization alive for a managed session, while your adapter invokes the application afresh for each command.

The SDK is the library for that adapter. It does not supply your application, build the adapter or turn an arbitrary script into a reusable worker. You prepare the application and its normal cold launcher, write the adapter, then create a binding JSON file that points to both launchers. The standalone `mdrun` host is installed separately and manages routing and process custody.

Choose the same exact host and SDK version for local and CI builds, and pin the downloaded asset hashes. For Python, install the wheel into the application's virtual environment. For Node and JVM, prepare a runtime archive; Python also supports the archive route. After preparation, follow your runtime's adapter guide and [verify the cold and warm routes](guides/sessions.md#verify-and-inspect-the-route).

Version [0.1.0](https://github.com/mbackschat/mdrun/releases/tag/v0.1.0) provides the JVM, Node and Python SDK archives, Python wheel and preparation helper used below. Its [exact-tag workflow](https://github.com/mbackschat/mdrun/actions/runs/43955233548) passes SDK archive/wheel consumption through the packaged host. Pinned public `prepare_sdk.py --download` preparation passes for all three archives; each archive consumer and the public `uv --release` wheel consumer passes two cold calls with zero workers and two required-warm calls with one worker and clean cleanup through the published host. [Distribution](distribution.md#published-010-checkpoint) records the asset identities. Verified local archives, wheels and CI artifacts can also use the explicit local-file routes. The SDK APIs and managed sessions follow the frozen [version-1 contract](plans/session-protocol.md); Maven Central, npm and PyPI publication remain [deferred](DEFERRED.md). Unrelated packages named `mdrun` are not these SDKs.

## Install the Python SDK with uv

Install the Python wheel into your application's virtual environment with uv. It is a library your adapter imports, so use `uv pip install` or a uv project dependency. Install the standalone `mdrun` host separately. The Python archive remains available for explicit prepared-module workflows.

Choose an exact published release version and pin its wheel URL and reviewed SHA-256 from `SHA256SUMS` in your application's requirements file. In the example below, you supply the absolute Python interpreter path and `MDRUN_PYTHON_SDK_SHA256`; uv creates `.venv`, and `printf` creates `mdrun-sdk.requirements.txt`. The example selects the published version 0.1.0:

```sh
uv venv --python /absolute/python3 .venv
printf 'mdrun-sdk @ https://github.com/mbackschat/mdrun/releases/download/v0.1.0/mdrun_sdk-0.1.0-py3-none-any.whl --hash=sha256:%s\n' \
  "$MDRUN_PYTHON_SDK_SHA256" > mdrun-sdk.requirements.txt
uv pip install --python .venv/bin/python --no-index --no-deps --require-hashes \
  --requirement mdrun-sdk.requirements.txt
```

`MDRUN_PYTHON_SDK_SHA256` must contain the reviewed lowercase wheel digest, pinned in the application's build configuration. Commit the resulting requirements file and use the same `uv pip install` command locally and in CI. For a previously downloaded wheel, change the requirement's URL to `file:///absolute/downloads/mdrun_sdk-0.1.0-py3-none-any.whl` and retain the same digest. `--no-index` avoids registry resolution and `--require-hashes` makes a missing or wrong digest fail installation. These are uv's [documented direct-file/URL package](https://docs.astral.sh/uv/pip/packages/) and [hash-checking](https://docs.astral.sh/uv/reference/settings/#require-hashes) interfaces; the project does not implement a Python package installer.

After installation, run the worker with `.venv/bin/python`, without a global import-path change. `from mdrun_sdk import run_worker` imports the SDK, and `importlib.metadata.version("mdrun-sdk")` reports the installed release version. The wheel requires Python 3.12 or later; qualification records the actual interpreter versions independently from that declared minimum. It has no native extension or runtime dependencies. Maven Central, npm and PyPI publication remain deferred; this wheel is a GitHub release asset.

## Pin and prepare an archive download

Download `prepare_sdk.py` from an exact versioned release; the helper can download the matching runtime archive for you. Record reviewed `SHA256SUMS` digests for the helper and archive in your application's build configuration and use those pinned values locally and in CI. A digest copied from an untrusted download location does not independently authenticate that location. Verify the helper before running it; for example, on macOS:

```sh
shasum -a 256 --check preparer.sha256
python3 prepare_sdk.py --runtime node --version 0.1.0 \
  --sha256 "$MDRUN_NODE_SDK_SHA256" --download --output /prepared/mdrun-node
```

You create `preparer.sha256` with the reviewed helper digest and filename in standard checksum format, and supply `MDRUN_NODE_SDK_SHA256` as the exact lowercase archive SHA-256 pinned by your application. `/prepared/mdrun-node` is an illustrative destination you choose; it must not already exist. Select an available release version and its matching digests. The helper downloads only `https://github.com/mbackschat/mdrun/releases/download/vVERSION/mdrun-sdk-RUNTIME-vVERSION.tar.gz`. GitHub's HTTPS asset redirect is supported, and the downloaded bytes must match the supplied digest before any output directory is created.

For an archive already obtained through a verified release download or CI artifact, replace `--download` with `--archive /downloads/mdrun-sdk-node-v0.1.0.tar.gz`, using your actual downloaded file path. Use `--runtime jvm` or `--runtime python` with the matching versioned archive and digest. The destination must not exist. Preparation verifies runtime/version, the complete file inventory and every per-file hash, rejects links/traversal/duplicate entries, applies compressed/unpacked byte limits and publishes one prepared directory. It does not compile or invoke application code, fetch JVM dependencies, create a host binding or modify global module/class paths.

A successful preparation leaves one verified SDK directory. Connect it only to the worker bootstrap:

| Runtime | How your adapter uses the prepared SDK |
| --- | --- |
| Node | Import the supplied `index.mjs` by explicit file path. |
| Python | Add the directory containing `mdrun_sdk` to the worker's import path. A wheel installation instead uses the application's virtual environment. |
| JVM | Resolve both classpath entries from `build-manifest.json` against the prepared directory and put them on the worker's classpath, alongside your compiled adapter. |

Preparation does not produce a running application. Implement classification, invocation and reset using the [JVM](../sdk/jvm/README.md), [Node](../sdk/node/README.md) or [Python](../sdk/python/README.md) guide. Keep the independent cold launcher on its ordinary application entry point and dependencies, without the SDK. Then create a [binding](guides/sessions.md#what-you-prepare) that points to both prepared launchers and compare repeated cold and required-warm calls.

## Release files

| Asset | Contents and requirements |
| --- | --- |
| `mdrun-sdk-jvm-vVERSION.tar.gz` | Prepared Java 21 SDK jar, unchanged hash-locked Jackson core jar, relative classpath/build manifest, POM, dependency lock/provenance, extracted dependency licenses/notices, README, project license, version and per-file SHA-256 manifest |
| `mdrun-sdk-node-vVERSION.tar.gz` | Maintained `index.mjs`, README, project license, version and per-file SHA-256 manifest; no external runtime dependencies |
| `mdrun-sdk-python-vVERSION.tar.gz` | Maintained `mdrun_sdk/__init__.py`, README, project license, version and per-file SHA-256 manifest; standard library only |
| `mdrun_sdk-VERSION-py3-none-any.whl` | Standard pure-Python distribution `mdrun-sdk`, import package `mdrun_sdk`, Python 3.12+, SDK code, core metadata, README description, MIT license and SHA-256 `RECORD`; no external dependencies or CLI entrypoint |
| `prepare_sdk.py` | Standalone Python 3 preparation helper, usable outside a source checkout |
| `SHA256SUMS` | Digests for the three compressed archives, Python wheel and preparation helper, alongside the native release assets |

Each archive has one `mdrun-sdk-RUNTIME-vVERSION` root. Runtime archives exclude tests, examples, application adapters, caches and source checkout paths. JVM SDK classes contain only the SDK package and the checked POM/project license; Jackson stays a separate unchanged jar with all five bundled notices retained. The [JVM lock](../sdk/jvm/dependencies.lock.json) identifies the canonical artifact, SHA-256, publication date and provenance. Registry metadata does not resolve or download dependencies during consumer preparation.

The host, archive `VERSION` files and JVM POM use the exact [Cargo version](../Cargo.toml). The release workflow packages these SDK assets alongside each corresponding host version.

## Reproduce and qualify SDK assets

This section is for maintainers building distribution assets from the repository. Application integrators can use a verified wheel or archive above without reproducing the SDK build. The example paths are preparation destinations and installed tools you supply, not files created by ordinary document commands.

The [builder](../packaging/sdk.py) reads the version from Cargo, rejects JVM POM drift and uses an explicit production-file allowlist. It writes sorted ordinary tar members with fixed modes, ownership and timestamps, and gzip with no filename and a zero timestamp. Identical source files, prepared jars and Python/zlib toolchain reproduce identical compressed bytes; JVM preparation additionally depends on the selected Java 21 compiler. No source tree metadata or output directory is encoded in an archive. The same dependency-free builder emits the standard wheel using fixed ZIP timestamps/modes, a complete SHA-256/size `RECORD`, core metadata, the SDK package and license. Building the wheel requires only the Python standard library; neither a build backend nor package-index access is involved. Its format follows the [PyPA wheel specification](https://packaging.python.org/en/latest/specifications/binary-distribution-format/).

```sh
python3 scripts/prepare_jvm.py --java-home "$JAVA_HOME" --download --output /prepared/jvm-sdk
python3 packaging/sdk.py archive jvm dist --jvm-prepared /prepared/jvm-sdk
python3 packaging/sdk.py archive node dist
python3 packaging/sdk.py archive python dist
python3 packaging/sdk.py wheel dist
python3 packaging/qualify_sdk.py --archives dist --version 0.1.0 \
  --java-home "$JAVA_HOME" --node /absolute/node --python /absolute/python3 \
  --mdrun /absolute/mdrun --output /prepared/sdk-consumer
```

JVM source preparation checks the exact dependency hash, conservative 14-day age, POM agreement and bundled notices before executing Java tools. Packaging independently rechecks prepared jar identities, SDK-only contents, embedded metadata and dependency notices. The [offline guards](../packaging/test_sdk.py) independently check archive contents, hashes, reproducibility and malformed/tampered preparation refusal; a mocked download guard checks the exact versioned GitHub URL and rejection of corrupted downloaded bytes.

The [consumer qualifier](../packaging/qualify_sdk.py) copies the standalone helper and synthetic `read_file` applications into a new persistent workspace, prepares the version/digest-pinned archives, compiles the JVM consumer there, and independently launches each SDK-free cold application. It then verifies two literal file-output calls through the actual native host in cold and required-warm modes: zero cold workers, exactly one reused warm worker, exact route/status counts and clean cleanup are mandatory. It uses a controlled environment for both modes; the read-only applications do not observe the two explicitly ignored shell bookkeeping variables. Prepared bindings and `qualification.json` remain available for separate full-session measurements. The [release workflow](../.github/workflows/release.yml) runs this qualifier against its same-run native archive before publishing SDK assets. A configured workflow and local pass do not establish a hosted or published release; record revision/artifact-specific results in the [execution checklist](plans/execution.md).

The separate [wheel consumer qualifier](../packaging/qualify_python_wheel.py) uses `uv venv` and `uv pip install --require-hashes` against a pinned local wheel or the exact GitHub release URL, then asserts the installed version and import location inside the application's virtual environment. Its cold interpreter disables site packages and must independently fail to find `mdrun_sdk`; cold and required-warm actual-host sessions retain the same literal output, status, route, reuse and cleanup oracles. To exercise a prepared wheel, supply `--wheel /downloads/mdrun_sdk-0.1.0-py3-none-any.whl --version 0.1.0 --sha256 "$MDRUN_PYTHON_SDK_SHA256" --python /absolute/python3 --uv /absolute/uv --mdrun /absolute/mdrun --output /prepared/wheel-consumer`. For a published asset, replace `--wheel PATH` with `--release` to qualify the real GitHub URL. The pre-publication local HTTP fixture tests uv's URL transport with the same release path and rejects a wrong digest; that fixture alone does not establish public asset availability. The release workflow pins uv `0.13.6`, [released on 2026-10-26](https://github.com/astral-sh/uv/releases/tag/0.13.6), and requires both installation/refusal guards and actual-host wheel consumption before publication.
