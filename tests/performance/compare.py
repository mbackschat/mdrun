"""Measure complete cold and reused sessions with the read_file qualification application."""

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import shlex
import shutil
import statistics
import subprocess
import sys
import tempfile
import time


def validate_report(report, mode, calls, artifact_id):
    warm = mode == "fast"
    expected = {"artifact_id": artifact_id, "outcome": "success", "infrastructure_failure": None,
                "warm_calls": calls if warm else 0, "cold_calls": 0 if warm else calls,
                "worker_starts": 1 if warm else 0,
                "cold_reasons": {} if warm else {"explicit_cold": calls}}
    for field, value in expected.items():
        if report.get(field) != value:
            raise ValueError(f"unqualified {mode} result: {field}={report.get(field)!r}, expected {value!r}")
    if report.get("cleanup", {}).get("outcome") != "clean":
        raise ValueError("uncertain cleanup invalidates timing")
    if report.get("author_status") != {"exit_code": 0, "signal": None}:
        raise ValueError("unsuccessful author invalidates timing")
    statuses = report.get("application_statuses", [])
    if len(statuses) != calls or any(
            status.get("route") != ("warm" if warm else "cold")
            or status.get("exit_code") != 0 or status.get("signal") is not None for status in statuses):
        raise ValueError("application effects or routes do not match the workload")


def document(binding, input_path, calls, sample):
    blocks = ["# Repeated file reads\n"]
    for ordinal in range(calls):
        payload = f"sample={sample} call={ordinal} " + "0123456789abcdef" * 64
        blocks.append(f"```sh\nprintf '%s\\n' {shlex.quote(payload)} > {shlex.quote(str(input_path))}\n"
                      f"{shlex.quote(binding)} read_file {shlex.quote(str(input_path))}\n```\n\n"
                      f"```output\n{payload}\n```\n")
    return "\n".join(blocks).encode()


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def artifact_snapshot(binary, binding_path, binding):
    paths = {binary, Path(binding["worker_argv"][0]), Path(binding["cold_argv"][0])}
    paths.update(path for path in binding_path.parent.rglob("*") if path.is_file())
    return {str(path): digest(path) for path in sorted(paths)}


def compare(binary, binding_path, calls, samples, preparation_ms):
    if os.environ.get("MDRUN_SESSION"):
        raise ValueError("run qualification outside a managed session")
    binding = json.loads(binding_path.read_bytes())
    original_binding = digest(binding_path)
    artifacts = artifact_snapshot(binary, binding_path, binding)
    workspace = Path(tempfile.mkdtemp(prefix="mdrun-compare-")).resolve()
    passed = False
    try:
        environment = {"PATH": "/usr/bin:/bin", "PWD": str(workspace)}
        observations = []
        for sample in range(samples):
            source = document(binding["name"], workspace / "input.txt", calls, sample)
            path = workspace / "workload.md"
            path.write_bytes(source)
            for mode in (("cold", "fast") if sample % 2 == 0 else ("fast", "cold")):
                if artifact_snapshot(binary, binding_path, binding) != artifacts:
                    raise ValueError("prepared artifacts changed between samples")
                (workspace / "input.txt").unlink(missing_ok=True)
                report_path = workspace / "report.json"
                report_path.unlink(missing_ok=True)
                command = [str(binary), "verify", str(path), "--binding", str(binding_path),
                           "--mode", mode, "--report", str(report_path)]
                if mode == "fast":
                    command.append("--require-warm")
                deadline = (binding.get("session_timeout_ms", 30000)
                            + 2 * binding.get("cleanup_timeout_ms", 2000)) / 1000 + 5
                with (workspace / "stdout").open("wb") as stdout, (workspace / "stderr").open("wb") as stderr:
                    started = time.perf_counter_ns()
                    result = subprocess.run(command, cwd=workspace, env=environment, stdin=subprocess.DEVNULL,
                                            stdout=stdout, stderr=stderr, timeout=deadline)
                    elapsed_ms = (time.perf_counter_ns() - started) / 1_000_000
                if result.returncode:
                    raise ValueError(f"{mode} verification failed: {(workspace / 'stderr').read_text(errors='replace')}")
                if path.read_bytes() != source or digest(binding_path) != original_binding:
                    raise ValueError("qualification changed a recording or prepared binding")
                if artifact_snapshot(binary, binding_path, binding) != artifacts:
                    raise ValueError("prepared artifacts changed during execution")
                report = json.loads(report_path.read_bytes())
                validate_report(report, mode, calls, binding["artifact_id"])
                if (workspace / "stdout").read_bytes() or (workspace / "stderr").read_bytes():
                    raise ValueError("unexpected verification output invalidates timing")
                observations.append({"sample": sample, "mode": mode, "wall_ms": elapsed_ms,
                                     "document_sha256": hashlib.sha256(source).hexdigest(),
                                     "warm_calls": report["warm_calls"], "cold_calls": report["cold_calls"],
                                     "worker_starts": report["worker_starts"],
                                     "cold_reasons": report["cold_reasons"],
                                     "durations_ms": report["durations_ms"]})
        cold = statistics.median(row["wall_ms"] for row in observations if row["mode"] == "cold")
        warm = statistics.median(row["wall_ms"] for row in observations if row["mode"] == "fast")
        passed = True
        return {"version": 1, "platform": platform.platform(), "machine": platform.machine(),
                "host_sha256": artifacts[str(binary)], "binding_sha256": original_binding,
                "prepared_files_sha256": artifacts,
                "artifact_id": binding["artifact_id"], "calls_per_session": calls, "samples": samples,
                "preparation_ms": preparation_ms, "observations": observations,
                "median_ms": {"cold": cold, "fast": warm}, "cold_over_fast_ratio": cold / warm,
                "scope": "Synthetic changing-file workload; complete verify processes include startup and cleanup. Preparation is separate; no general application speedup is claimed."}
    finally:
        if passed:
            shutil.rmtree(workspace)
        else:
            print(f"Failed qualification workspace retained: {workspace}", file=sys.stderr)


def positive(value):
    result = int(value)
    if result < 1:
        raise argparse.ArgumentTypeError("must be positive")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mdrun", required=True, type=Path)
    parser.add_argument("--binding", required=True, type=Path)
    parser.add_argument("--calls", type=positive, default=20)
    parser.add_argument("--samples", type=positive, default=5)
    parser.add_argument("--preparation-ms", type=float)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    if args.output.exists():
        parser.error("output must not exist")
    if args.preparation_ms is not None and (not math.isfinite(args.preparation_ms) or args.preparation_ms < 0):
        parser.error("preparation duration must be finite and nonnegative")
    try:
        result = compare(args.mdrun.resolve(strict=True), args.binding.resolve(strict=True),
                         args.calls, args.samples, args.preparation_ms)
        with args.output.open("x") as output:
            json.dump(result, output, indent=2)
            output.write("\n")
    except (OSError, ValueError, subprocess.TimeoutExpired) as error:
        parser.error(str(error))


if __name__ == "__main__":
    main()
