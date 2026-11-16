"""Measure forwarded tiny calls and unpaused streaming with the same frozen Node fixture."""

import argparse
from contextlib import contextmanager
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import shutil
import statistics
import subprocess
import sys
import tempfile
import time

from compare import digest, validate_report

ROOT = Path(__file__).resolve().parents[2]
TOTAL = 128 * 1024 * 1024
EXPECTED_HASH = "06233646f01770d62d9318d0228528a8097936a5d25d96484a45526f48ded01c"


@contextmanager
def workspace():
    directory = tempfile.mkdtemp(prefix="mdrun-wakeup-measure-")
    try:
        yield directory
    except BaseException:
        print(f"Failed measurement workspace retained: {directory}", file=sys.stderr)
        raise
    else:
        shutil.rmtree(directory)


def validate_observations(value, calls):
    latencies = value.get("latency_ms", [])
    times = [*latencies, value.get("stream_ms", 0)]
    if (len(latencies) != calls or value.get("stream_bytes") != TOTAL
            or value.get("stream_sha256") != EXPECTED_HASH
            or any(not isinstance(item, (float, int)) or not math.isfinite(item) or item <= 0
                   for item in times)):
        raise ValueError("call count, stream bytes/hash or timing oracle failed")


def author(directory, calls):
    observations = {"latency_ms": []}
    for _ in range(calls):
        started = time.perf_counter_ns()
        result = subprocess.run(["tool", "ignore"], stdin=subprocess.DEVNULL,
                                capture_output=True, timeout=5)
        elapsed = (time.perf_counter_ns() - started) / 1_000_000
        if result.returncode or result.stdout != b"ok" or result.stderr:
            raise ValueError("tiny invocation output/status changed")
        observations["latency_ms"].append(elapsed)
    output = directory / "stream.bin"
    with output.open("wb") as stream:
        started = time.perf_counter_ns()
        result = subprocess.run(["tool", "streaming", str(directory / "progress.json")],
                                stdin=subprocess.DEVNULL, stdout=stream,
                                stderr=subprocess.PIPE, timeout=25)
        observations["stream_ms"] = (time.perf_counter_ns() - started) / 1_000_000
    if result.returncode or result.stderr:
        raise ValueError("stream invocation failed")
    observations["stream_bytes"] = output.stat().st_size
    with output.open("rb") as stream:
        observations["stream_sha256"] = hashlib.file_digest(stream, "sha256").hexdigest()
    validate_observations(observations, calls)
    (directory / "observations.json").write_text(json.dumps(observations))


def measure(binary, node, samples, calls):
    if os.environ.get("MDRUN_SESSION"):
        raise ValueError("measure outside a managed session")
    fixture = ROOT / "tests/runtime/node/fixture-worker.mjs"
    sdk = ROOT / "sdk/node/index.mjs"
    tracked = [binary, node, Path(sys.executable).resolve(), fixture, sdk, Path(__file__).resolve()]
    identities = {str(path): digest(path) for path in tracked}
    environment = {"PATH": "/usr/bin:/bin", "LC_ALL": "C", "LANG": "C",
                   "PYTHONCOERCECLOCALE": "0", "PYTHONUTF8": "1", "PYTHONDONTWRITEBYTECODE": "1"}
    if sys.platform == "darwin":
        probe = subprocess.run([str(node), "-p", "process.env.__CF_USER_TEXT_ENCODING ?? ''"],
                               env=environment, capture_output=True, check=True, timeout=5)
        environment["__CF_USER_TEXT_ENCODING"] = os.fsdecode(probe.stdout.removesuffix(b"\n"))
    version = subprocess.run([str(node), "--version"], capture_output=True, text=True,
                             check=True, timeout=5).stdout.strip()
    observations = []
    with workspace() as temporary:
        prepared = Path(temporary).resolve()
        copied_fixture = prepared / "tests/runtime/node/fixture-worker.mjs"
        copied_sdk = prepared / "sdk/node/index.mjs"
        copied_fixture.parent.mkdir(parents=True)
        copied_sdk.parent.mkdir(parents=True)
        shutil.copyfile(fixture, copied_fixture)
        shutil.copyfile(sdk, copied_sdk)
        artifact = "sha256:" + hashlib.sha256(json.dumps(
            {"fixture": identities[str(fixture)], "sdk": identities[str(sdk)],
             "node": identities[str(node)]}, sort_keys=True).encode()).hexdigest()
        binding = {"version": 1, "name": "tool", "artifact_id": artifact,
                   "worker_argv": [str(node), str(copied_fixture)], "cold_argv": ["/bin/false"],
                   "startup_timeout_ms": 5000, "session_timeout_ms": 30000,
                   "cleanup_timeout_ms": 2000, "context": {"environment": environment}}
        binding_path = prepared / "binding.json"
        binding_path.write_text(json.dumps(binding))
        for sample in range(samples):
            if {str(path): digest(path) for path in tracked} != identities:
                raise ValueError("measured source/runtime changed between samples")
            directory = prepared / str(sample)
            directory.mkdir()
            report = directory / "report.json"
            command = [str(binary), "session", "--binding", str(binding_path), "--require-warm",
                       "--report", str(report), "--", sys.executable, str(Path(__file__).resolve()),
                       "--author", str(directory), "--calls", str(calls)]
            started = time.perf_counter_ns()
            result = subprocess.run(command, cwd=directory, env={**environment, "PWD": str(directory)},
                                    stdin=subprocess.DEVNULL, capture_output=True, timeout=40)
            elapsed = (time.perf_counter_ns() - started) / 1_000_000
            if result.returncode or result.stdout or result.stderr:
                raise ValueError(f"session failed: {result.returncode}: {result.stderr.decode(errors='replace')}")
            status = json.loads(report.read_bytes())
            validate_report(status, "fast", calls + 1, artifact)
            observation = json.loads((directory / "observations.json").read_bytes())
            validate_observations(observation, calls)
            if {str(path): digest(path) for path in tracked} != identities:
                raise ValueError("measured source/runtime changed during sample")
            if digest(copied_fixture) != identities[str(fixture)] or digest(copied_sdk) != identities[str(sdk)]:
                raise ValueError("prepared fixture or SDK changed")
            observations.append({**observation, "session_ms": elapsed,
                                 "warm_calls": status["warm_calls"], "cold_calls": status["cold_calls"],
                                 "worker_starts": status["worker_starts"], "cleanup": status["cleanup"]})
            print(f"sample {sample + 1}/{samples} qualified", file=sys.stderr, flush=True)
    latencies = sorted(value for observation in observations for value in observation["latency_ms"])
    return {"version": 1, "platform": platform.platform(), "node_version": version,
            "files_sha256": identities, "host_sha256": identities[str(binary)],
            "artifact_id": artifact, "calls_per_session": calls, "samples": observations,
            "latency_ms": {"median": statistics.median(latencies),
                           "p95": latencies[math.ceil(len(latencies) * .95) - 1]},
            "stream_mib_per_second": statistics.median(
                TOTAL / 1048576 / (sample["stream_ms"] / 1000) for sample in observations),
            "scope": "Forwarded tiny-call latency includes client process startup and the complete protocol round trip. Unpaused 128 MiB streaming includes the fixture's per-frame progress-file writes and output to a regular file. Worker/session startup is outside per-call timing; complete session durations are retained. No general application speedup is claimed."}


def positive(value):
    result = int(value)
    if result < 1:
        raise argparse.ArgumentTypeError("must be positive")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mdrun", type=Path)
    parser.add_argument("--node", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--samples", type=positive, default=3)
    parser.add_argument("--calls", type=positive, default=20)
    parser.add_argument("--author", type=Path, help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.author:
        author(args.author, args.calls)
        return
    if args.mdrun is None or args.node is None or args.output is None:
        parser.error("--mdrun, --node and --output are required")
    if args.output.exists():
        parser.error("output must not exist")
    result = measure(args.mdrun.resolve(strict=True), args.node.resolve(strict=True), args.samples, args.calls)
    with args.output.open("x") as output:
        json.dump(result, output, indent=2)
        output.write("\n")


if __name__ == "__main__":
    main()
