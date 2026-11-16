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

## Repeat with prepared release fixtures

First prepare the three synthetic applications using the [SDK consumer qualifier](sdk-distribution.md#reproduce-and-qualify-sdk-assets). Then measure one runtime at a time, on an otherwise idle host:

```sh
python3 tests/performance/compare.py \
  --mdrun /absolute/mdrun \
  --binding /prepared/sdk-consumer/jvm/binding.json \
  --calls 20 --samples 5 --output /results/jvm.json
```

Use the corresponding Node or Python binding to measure those runtimes. Supply `--preparation-ms` from that preparation's `qualification.json` when available; omission records null rather than inventing a duration. The output must not exist. A failed process, output mismatch, unexpected route, artifact change or uncertain cleanup invalidates the result and retains the scratch workspace for diagnosis. This is development qualification tooling, not an additional product command.
