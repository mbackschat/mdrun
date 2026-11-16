# Complete-session measurements

These measurements qualify a finite synthetic workload, not a general application speedup. On 2026-11-16, the optimized Rust 1.98.1 host ran five alternating cold/required-warm pairs per runtime on macOS 27.2.1 arm64. Each session verified twenty Markdown commands that rewrote and read a roughly 1 KiB file; every call and sample used different contents. Timings include native forwarding, worker startup, replay, shutdown, cleanup and report publication. The independent cold launcher contains no SDK.

| Runtime | Cold median | Warm median | Cold / warm | Separate consumer preparation |
| --- | ---: | ---: | ---: | ---: |
| Homebrew OpenJDK 21.0.13.0.3 | 3,705 ms | 2,028 ms | 1.83× | 1,376 ms |
| Node 26.10.0 | 3,172 ms | 1,876 ms | 1.69× | 98 ms |
| CPython 3.15.2 | 2,580 ms | 1,926 ms | 1.34× | 136 ms |

Preparation is the separate SDK archive verification/extraction and consumer setup measured by the [SDK qualifier](../packaging/qualify_sdk.py); JVM preparation also compiles the synthetic application. It excludes downloading the already available archives. Cold and warm reuse the same prepared application, runtime, context and workload. Required-warm reports must contain twenty warm calls, zero cold calls and one worker; cold reports must contain twenty explicitly cold calls and zero workers. Every application status must be zero and cleanup clean. Literal recorded output, unchanged input documents and binding identity are independently checked; equal warm/cold output alone is insufficient.

The measured host SHA-256 is `80846ff7dad8cc5a6062846c96a2259c1ebd6ca349ac90c615095b1eaab05c4f`, built from the production source through `1876707`. It is 4,201,856 bytes and links macOS `libSystem` and `libiconv`. Later embedded-guide changes require a new binary identity; these figures do not silently qualify a different executable. A discarded JVM run overlapped an independent usability workload; the recorded JVM timings come from a subsequent run after that workload finished. No concurrent project compilation or qualification workload ran during the recorded samples.

| Prepared SDK archive | SHA-256 |
| --- | --- |
| JVM `0.1.0` | `6945c831754fc1bbc08af581e8fba8fb2a1813481646153dc5d88da1e520c8b3` |
| Node `0.1.0` | `6ea9461ac24f900ba17a9901fd40aec6ec660d0d2190b94064174fefcb09fbb9` |
| Python `0.1.0` | `b2c90e6f42b913d2a63a16638c2c743f5390ddc583281437b981015e5db200af` |

The [measurement harness](../tests/performance/compare.py) records raw per-session times, host and prepared-file digests, document hashes, actual route counts and report durations in its JSON output. It checks the host, prepared directory and launch executables before and after every timed process, outside the measured interval. Installed runtime libraries and operating-system scheduling remain environmental inputs; this is a local comparison, not a controlled hardware benchmark or a promised ratio. The workload intentionally has little application work, so applications dominated by computation or I/O may benefit less or not at all.

## Coordinator wakeup measurements

On 2026-11-16, a separate local comparison measured the host after simplification commit `087c81c` against the event-driven coordinator. Both optimized executables use Rust 1.98.1 on macOS arm64, Node 26.10.0 and the same SDK/fixture/runtime/harness bytes. No concurrent project build or qualification suite ran during either measurement. Each host ran three sessions, each with twenty tiny calls returning literal `ok`, followed by an unpaused 128 MiB stream. The [measurement harness](../tests/performance/wakeup.py) requires the fixed stream length/SHA-256, every call warm and successful, exactly one worker, clean cleanup and unchanged artifact identities before accepting timings.

| Measurement | Fixed-tick host | Event-driven host |
| --- | ---: | ---: |
| Median forwarded tiny-call latency | 83.04 ms | 12.27 ms |
| 95th-percentile forwarded tiny-call latency | 137.57 ms | 16.34 ms |
| Median stream throughput | 6.10 MiB/s | 22.78 MiB/s |

Tiny-call timing includes native client process startup and the complete forwarding/protocol round trip, but excludes session/worker startup. Stream timing includes the fixture's per-frame progress-file writes and delivery into a regular file; it is not a raw socket-bandwidth benchmark. Every sample retains complete session time separately. The local median improvements are approximately 6.77× for tiny-call latency and 3.73× for this stream workload; they do not predict arbitrary application speedups. The broker and custodian retain their separate lifecycle polling.

| Measured artifact | SHA-256 |
| --- | --- |
| Before host, production source `087c81c` | `cc5fd1e1be9db85902f3c99ebc19876f4e95fc5ddda36465fd4349e395a90adf` |
| After host, event-driven implementation | `a14253152651029e381c5170af6b4a260247478273d30caa2b436eed673a6f58` |
| Common SDK/fixture/runtime artifact identity | `2668c340f9ec9bd3c9144bf9a820dd4921e9a758c21a12a661b947d94e60d835` |

Repeat one host at a time on an otherwise idle system; save each output separately and compare only runs with identical non-host identities:

```sh
python3 tests/performance/wakeup.py \
  --mdrun /absolute/mdrun --node /absolute/node \
  --samples 3 --calls 20 --output /results/wakeup.json
```

The harness retains failed workspaces for diagnosis. Its finite deadlines reject stalled sessions instead of recording a partial result. Output, routes and cleanup qualify each sample independently of its speed; performance is recorded rather than enforced through fragile wall-clock CI thresholds.

## Repeat complete sessions with prepared release fixtures

First prepare the three synthetic applications using the [SDK consumer qualifier](sdk-distribution.md#reproduce-and-qualify-sdk-assets). Then measure one runtime at a time, on an otherwise idle host:

```sh
python3 tests/performance/compare.py \
  --mdrun /absolute/mdrun \
  --binding /prepared/sdk-consumer/jvm/binding.json \
  --calls 20 --samples 5 --output /results/jvm.json
```

Use the corresponding Node or Python binding to measure those runtimes. Supply `--preparation-ms` from that preparation's `qualification.json` when available; omission records null rather than inventing a duration. The output must not exist. A failed process, output mismatch, unexpected route, artifact change or uncertain cleanup invalidates the result and retains the scratch workspace for diagnosis. This is development qualification tooling, not an additional product command.
