# Complete-session measurements

To decide whether reusing application initialization helps a workload, compare complete sessions against an independent cold launcher and verify what actually ran. The measurements below answer two narrower questions: how much a finite synthetic document benefited from warm execution, and how a coordinator change affected tiny calls and streaming. They do not establish a general application speedup.

## Current host comparison

The original complete-session table below predates the event-driven host coordinator. On 2026-11-16, the existing benchmark was repeated with the current optimized host, using the same three prepared SDK archive versions and the unchanged changing-file workload. No heavier fixture, artificial startup delay or target speedup was introduced.

Each session verifies twenty commands. Each command writes and reads a different roughly 1 KiB payload; every sample also changes the payload. Five cold/required-warm pairs per runtime alternate execution order. Runtimes were measured sequentially on macOS 27.2.1 arm64, with no concurrent project builds or agent qualification jobs. The host was built explicitly with Rust 1.98.1.

| Runtime | Fresh processes | One warm worker | Speedup |
| --- | ---: | ---: | ---: |
| OpenJDK 21.0.13.0.3 | 1,978.901 ms | 401.288 ms | 4.9× |
| Node 26.10.0 | 1,852.121 ms | 357.727 ms | 5.2× |
| CPython 3.15.2 | 1,568.330 ms | 382.156 ms | 4.1× |

Speedup is the cold median divided by the warm median, rounded to one decimal place. Both medians use all five observations; no pair was discarded. Whole-process wall time includes native forwarding, worker startup, document replay, shutdown, cleanup and report publication. SDK/application preparation happened before these measurements and is not included.

For every accepted sample, the harness checked literal document output, unchanged document/binding/prepared artifacts, successful application statuses, correct route counts and clean cleanup. Warm reports have twenty warm calls, zero cold calls and one worker; cold reports have twenty explicitly cold calls and no worker. This is fresh-process cold execution: artifact checks read files before timing, so it is not a cold-filesystem-cache experiment. Installed runtime libraries and OS scheduling remain environmental inputs outside the file snapshots.

### All five sample pairs

| Runtime | Pair | Cold ms | Warm ms |
| --- | ---: | ---: | ---: |
| JVM | 1 | 1978.901 | 396.247 |
| JVM | 2 | 1792.913 | 378.534 |
| JVM | 3 | 1874.692 | 423.188 |
| JVM | 4 | 2276.089 | 401.288 |
| JVM | 5 | 2153.006 | 498.277 |
| Node | 1 | 1852.121 | 353.432 |
| Node | 2 | 1832.376 | 357.727 |
| Node | 3 | 1854.267 | 346.275 |
| Node | 4 | 1851.264 | 360.836 |
| Node | 5 | 1853.487 | 392.776 |
| Python | 1 | 1606.121 | 382.156 |
| Python | 2 | 1561.593 | 346.598 |
| Python | 3 | 1606.159 | 382.534 |
| Python | 4 | 1564.656 | 405.103 |
| Python | 5 | 1568.330 | 374.928 |

### Current measurement identities

The measured executable is `71572eb97d6966ad016715d87215a3f2bd5813f5070d86141d9ba16b9441c830` (SHA-256), 4,218,448 bytes. It was built before history consolidation from the working tree based on pre-squash revision `93e2900a694cc23c92d91f55a1650cd632f96ae1` with rewritten embedded guides; production Rust source and Cargo dependencies were unchanged from that commit. This identifies a local measurement, not a published release or new hosted qualification. The unchanged [comparison harness](../tests/performance/compare.py) has SHA-256 `31ff6753a336ada50419fea9c5d06ca9572086fe7be05a2463a543df1bb2d4f9`.

| Runtime | Binding SHA-256 | Prepared application identity |
| --- | --- | --- |
| JVM | `2a2660649631ee5be77415d8c2286b42d8cfe3a4b00b7130a00c8a7e8adc2203` | `sha256:c5cbac6794d802f1c169fc0233762b45b8467f3fa15726a46ee0e518bdffde56` |
| Node | `c3bc61cd3fabb547c7301c816c3652eb9ae43c21587435de6031014c00a50226` | `sha256:935ea5a8bd6722ad72c96fd69f8db213b2e69946ec302662b2fd7af16c56aa44` |
| Python | `e0f2fb4b2f283aac56ee58a0663cc9ef91a1bfadee6ddfb88cad83a6d457a789` | `sha256:2733f95d3efe1c3d477ce12dd58233ff373be472b7debc225f44e0cf38a51cfe` |

The prepared SDK archive hashes are the three recorded in the historical comparison below. Runtime versions are shown in the current table. Prepared paths are machine-local; use the [SDK consumer preparation](sdk-distribution.md#reproduce-and-qualify-sdk-assets) and [comparison command](#repeat-complete-sessions-with-prepared-release-fixtures) to reproduce the workload with newly identified artifacts.

### Toolchain correction before the final run

An initial rerun used the local default Rust 1.98.0 compiler. Before measuring again, the build was explicitly pinned to the release qualification toolchain, Rust 1.98.1. The README uses all five pairs from that final pinned run. The preliminary medians are retained here for disclosure:

| Runtime | Rust 1.98.0 cold median | Rust 1.98.0 warm median | Cold / warm |
| --- | ---: | ---: | ---: |
| JVM | 3314.870 ms | 811.065 ms | 4.09× |
| Node | 2669.577 ms | 637.445 ms | 4.19× |
| Python | 2357.368 ms | 671.630 ms | 3.51× |

The preliminary host SHA-256 was `09a682ca6671b57b81b206aae5f2a5c1ae1c9c52bb6cc3ecd9dc49832a15ffbd`. These separate runs do not isolate a compiler effect; timing differences must not be attributed to the compiler patch alone. Neither run used a modified workload or omitted slower observations.

## Complete-session comparison

This historical baseline is retained for provenance. It predates the host coordinator improvement and is superseded by the [current-host measurements](#current-host-comparison) for README claims.

On 2026-11-16, the optimized Rust 1.98.1 host ran five alternating cold/required-warm pairs per runtime on macOS 27.2.1 arm64. Each session verified twenty Markdown commands that rewrote and read a roughly 1 KiB file; every call and sample used different contents. The independent cold launcher contains no SDK.

| Runtime | Cold median | Warm median | Cold / warm | Separate consumer preparation |
| --- | ---: | ---: | ---: | ---: |
| Homebrew OpenJDK 21.0.13.0.3 | 3,705 ms | 2,028 ms | 1.83× | 1,376 ms |
| Node 26.10.0 | 3,172 ms | 1,876 ms | 1.69× | 98 ms |
| CPython 3.15.2 | 2,580 ms | 1,926 ms | 1.34× | 136 ms |

Session timings include native forwarding, worker startup, replay, shutdown, cleanup and report publication. Preparation is measured separately by the [SDK qualifier](../packaging/qualify_sdk.py): it verifies/extracts the SDK archive and sets up the consumer; JVM preparation also compiles the synthetic application. It excludes downloading the already available archives. Cold and warm sessions reuse the same prepared application, runtime, context and workload.

### Conditions for accepting a sample

Timing alone is insufficient. Each sample must meet these independent expectations:

| Route | Required report |
| --- | --- |
| Required warm | Twenty warm calls, zero cold calls and one worker |
| Explicit cold | Twenty explicitly cold calls and zero workers |
| Both | Every application status zero; clean cleanup |

The harness also checks literal recorded output, unchanged input documents and binding identity. Equal warm/cold output alone cannot establish correctness.

The [measurement harness](../tests/performance/compare.py) records raw per-session times, host and prepared-file digests, document hashes, actual route counts and report durations in JSON. Before and after every timed process, outside the measured interval, it checks the host, prepared directory and launch executables. Installed runtime libraries and operating-system scheduling remain environmental inputs. This is a local comparison, not a controlled hardware benchmark or a promised ratio. The workload intentionally has little application work; computation- or I/O-dominated applications may benefit less or not at all.

### Executable and prepared-artifact identities

The measured host SHA-256 is `80846ff7dad8cc5a6062846c96a2259c1ebd6ca349ac90c615095b1eaab05c4f`, built from production source through `1876707`. It is 4,201,856 bytes and links macOS `libSystem` and `libiconv`. Later embedded-guide changes require a new binary identity; these figures do not silently qualify a different executable.

| Prepared SDK archive | SHA-256 |
| --- | --- |
| JVM `0.1.0` | `6945c831754fc1bbc08af581e8fba8fb2a1813481646153dc5d88da1e520c8b3` |
| Node `0.1.0` | `6ea9461ac24f900ba17a9901fd40aec6ec660d0d2190b94064174fefcb09fbb9` |
| Python `0.1.0` | `b2c90e6f42b913d2a63a16638c2c743f5390ddc583281437b981015e5db200af` |

A discarded JVM run overlapped an independent usability workload. The recorded JVM timings come from a subsequent run after that workload finished. No concurrent project compilation or qualification workload ran during the recorded samples.

## Coordinator wakeup measurements

This separate comparison isolates a host-coordinator change while holding SDK, fixture, runtime and harness bytes constant. On 2026-11-16, it measured the host after simplification commit `087c81c` against the event-driven coordinator committed as `1cee479`. Both optimized executables use Rust 1.98.1 on macOS arm64 and Node 26.10.0. No concurrent project build or qualification suite ran during either measurement.

Each host ran three sessions, each with twenty tiny calls returning literal `ok`, followed by an unpaused 128 MiB stream. The [wakeup harness](../tests/performance/wakeup.py) accepts timings only after checking the fixed stream length/SHA-256, every call warm and successful, exactly one worker, clean cleanup and unchanged artifact identities.

| Measurement | Fixed-tick host | Event-driven host |
| --- | ---: | ---: |
| Median forwarded tiny-call latency | 83.04 ms | 12.27 ms |
| 95th-percentile forwarded tiny-call latency | 137.57 ms | 16.34 ms |
| Median stream throughput | 6.10 MiB/s | 22.78 MiB/s |

Tiny-call timing includes native client process startup and the complete forwarding/protocol round trip, but excludes session/worker startup. Stream timing includes the fixture's per-frame progress-file writes and delivery into a regular file; it is not a raw socket-bandwidth benchmark. Every sample retains complete session time separately.

The local median improvements are approximately 6.77× for tiny-call latency and 3.73× for this stream workload. They do not predict arbitrary application speedups. The broker and custodian retain their separate lifecycle polling.

| Measured artifact | SHA-256 |
| --- | --- |
| Before host, production source `087c81c` | `cc5fd1e1be9db85902f3c99ebc19876f4e95fc5ddda36465fd4349e395a90adf` |
| After host, production source `1cee479` | `a14253152651029e381c5170af6b4a260247478273d30caa2b436eed673a6f58` |
| Common SDK/fixture/runtime artifact identity | `2668c340f9ec9bd3c9144bf9a820dd4921e9a758c21a12a661b947d94e60d835` |

### Repeat the coordinator comparison

Run one host at a time on an otherwise idle system. Save each output separately and compare only runs with identical non-host identities:

```sh
python3 tests/performance/wakeup.py \
  --mdrun /absolute/mdrun --node /absolute/node \
  --samples 3 --calls 20 --output /results/wakeup.json
```

The harness retains failed workspaces for diagnosis. Finite deadlines reject stalled sessions instead of recording partial results. Output, routes and cleanup qualify every sample independently of its speed; performance is recorded rather than enforced through fragile wall-clock CI thresholds.

## Repeat complete sessions with prepared release fixtures

Prepare the three synthetic applications with the [SDK consumer qualifier](sdk-distribution.md#reproduce-and-qualify-sdk-assets), then measure one runtime at a time on an otherwise idle host. The binding path below identifies that qualifier's prepared JVM application; use the corresponding Node or Python binding for those runtimes. The absolute executable and result paths are supplied by the person running the measurement.

```sh
python3 tests/performance/compare.py \
  --mdrun /absolute/mdrun \
  --binding /prepared/sdk-consumer/jvm/binding.json \
  --calls 20 --samples 5 --output /results/jvm.json
```

Supply `--preparation-ms` from that preparation's `qualification.json` when available. Omission records null rather than inventing a duration. The output must not exist. A failed process, output mismatch, unexpected route, artifact change or uncertain cleanup invalidates the result and retains the scratch workspace for diagnosis. This is development qualification tooling, not an additional product command.
