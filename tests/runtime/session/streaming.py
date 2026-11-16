"""Qualify actual-host streaming with paused consumers and an eager-buffer negative control."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import selectors
import subprocess
import sys
import tempfile
import threading
import time


ROOT = Path(__file__).resolve().parents[3]
CHUNK = 65536
TOTAL = 128 * 1024 * 1024
QUEUE_BOUND = 32 * 1024 * 1024
TRANCHE = 4 * 1024 * 1024
PLATEAU_SECONDS = 0.5
CASE_SECONDS = 30
PATTERN = bytes(index % 251 for index in range(CHUNK))


class OracleFailure(AssertionError):
    def __init__(self, reason, produced, consumed):
        super().__init__(f"{reason}: produced={produced}, consumed={consumed}")
        self.details = {"reason": reason, "produced_bytes": produced, "consumed_bytes": consumed}


class Progress:
    def __init__(self, path):
        self.path = path
        self.pid = None
        self.produced = 0
        self.consumed = 0
        self.maximum_queued = 0

    def read(self):
        if not self.path.exists():
            return self.produced
        value = json.loads(self.path.read_text())
        assert set(value) == {"pid", "completed_bytes"}, value
        pid, completed = value["pid"], value["completed_bytes"]
        assert type(pid) is int and pid > 0, value
        assert type(completed) is int and self.produced <= completed <= TOTAL and completed % CHUNK == 0, value
        assert self.pid is None or self.pid == pid, value
        self.pid, self.produced = pid, completed
        self.maximum_queued = max(self.maximum_queued, self.produced - self.consumed)
        return completed

    def assert_queue_bound(self):
        if self.produced - self.consumed > QUEUE_BOUND:
            raise OracleFailure("accepted_queue_bound", self.produced, self.consumed)


def remaining(deadline):
    value = deadline - time.monotonic()
    if value <= 0:
        raise TimeoutError("streaming case exceeded its external deadline")
    return value


def plateau(progress, child, report, deadline):
    limit = min(deadline, time.monotonic() + 8)
    previous, stable_since = None, None
    while time.monotonic() < limit:
        produced = progress.read()
        progress.assert_queue_bound()
        if child.poll() is not None or report.exists() or produced == TOTAL:
            raise OracleFailure("completed_while_consumer_paused", produced, progress.consumed)
        if progress.pid is not None:
            os.kill(progress.pid, 0)
        now = time.monotonic()
        if produced > 0 and produced == previous:
            if now - stable_since >= PLATEAU_SECONDS:
                return {"produced_bytes": produced, "consumed_bytes": progress.consumed,
                        "stable_seconds": now - stable_since, "producer_pid": progress.pid}
        else:
            previous, stable_since = produced, now
        time.sleep(0.025)
    raise OracleFailure("no_stable_plateau", progress.produced, progress.consumed)


def memory_sample(child, producer):
    """Observe RSS without making allocator or JIT behavior a pass/fail oracle."""
    try:
        inventory = subprocess.run(["ps", "-axo", "pid=,ppid=,rss=,stat="], capture_output=True,
                                   text=True, check=True, timeout=2)
        rows = {}
        for line in inventory.stdout.splitlines():
            pid, parent, rss, state = line.split()
            rows[int(pid)] = {"pid": int(pid), "parent": int(parent), "rss_kib": int(rss), "state": state}
        selected = {child.pid}
        while added := {pid for pid, row in rows.items() if row["parent"] in selected} - selected:
            selected.update(added)
        known = sorted(selected | ({producer} if producer else set()))
        commands = subprocess.run(["ps", "-p", ",".join(map(str, known)), "-o", "pid=,command="],
                                  capture_output=True, text=True, timeout=2)
        clients = []
        for line in commands.stdout.splitlines():
            pid, command = line.strip().split(maxsplit=1)
            if Path(command.split(maxsplit=1)[0]).name == "tool":
                clients.append(int(pid))
        return {"host": rows.get(child.pid), "worker": rows.get(producer),
                "clients": [rows[pid] for pid in clients if pid in rows],
                "owned_processes": [rows[pid] for pid in known if pid in rows]}
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        return {"unavailable": type(error).__name__}


def consume(child, progress, digest, deadline, target=None):
    with selectors.DefaultSelector() as selector:
        selector.register(child.stdout, selectors.EVENT_READ)
        while target is None or progress.consumed < target:
            if not selector.select(min(0.2, remaining(deadline))):
                continue
            size = CHUNK if target is None else min(CHUNK, target - progress.consumed)
            data = os.read(child.stdout.fileno(), size)
            if not data:
                assert target is None, "EOF before the bounded tranche was delivered"
                return
            digest.update(data)
            progress.consumed += len(data)
            assert progress.consumed <= TOTAL, "producer exceeded the fixed payload"
            progress.read()
            progress.assert_queue_bound()


def expected_hash():
    digest = hashlib.sha256()
    for _ in range(TOTAL // CHUNK):
        digest.update(PATTERN)
    return digest.hexdigest()


def validate_report(report, status):
    assert status == 0, report
    assert (report["warm_calls"], report["cold_calls"], report["worker_starts"]) == (1, 0, 1), report
    assert report["author_status"]["exit_code"] == 0, report
    assert len(report["application_statuses"]) == 1, report
    assert report["application_statuses"][0]["exit_code"] == 0, report
    assert report["application_statuses"][0]["route"] == "warm", report
    assert report["outcome"] == "success" and report["infrastructure_failure"] is None, report
    assert report["cleanup"]["outcome"] == "clean", report


def stop_child(child):
    if child.poll() is None:
        child.terminate()
        try:
            child.wait(timeout=3)
        except subprocess.TimeoutExpired:
            child.kill()
            child.wait(timeout=3)


def run_case(mdrun, binding, environment, case, suite_deadline):
    started = time.monotonic()
    eager, broken = case == "eager", case == "broken"
    result = {"case": case, "payload_bytes": TOTAL,
              "accepted_queue_bound_bytes": QUEUE_BOUND, "plateaus": [], "rss_samples": []}
    with tempfile.TemporaryDirectory(prefix="mdrun-streaming-") as temporary:
        directory = Path(temporary).resolve()
        progress = Progress(directory / "progress.json")
        report = directory / "report.json"
        config = directory / "binding.json"
        config.write_text(json.dumps(binding))
        deadline = min(time.monotonic() + CASE_SECONDS, suite_deadline)
        spool = None
        drain_thread = None
        drained = {"bytes": 0, "hash": hashlib.sha256(), "error": None}
        with (directory / "diagnostics").open("wb") as diagnostics:
            child = subprocess.Popen([str(mdrun), "session", "--binding", str(config), "--mode", "fast",
                                      "--require-warm", "--report", str(report), "--", "tool", "streaming", str(progress.path)],
                                     cwd=directory, env=environment, stdin=subprocess.DEVNULL,
                                     stdout=subprocess.PIPE, stderr=diagnostics, bufsize=0)
            try:
                if eager:
                    spool = (directory / "eager-spool.bin").open("wb")

                    def hidden_drain():
                        try:
                            while data := child.stdout.read(CHUNK):
                                assert drained["bytes"] + len(data) <= TOTAL
                                spool.write(data)
                                drained["bytes"] += len(data)
                                drained["hash"].update(data)
                        except BaseException as error:
                            drained["error"] = error

                    drain_thread = threading.Thread(target=hidden_drain, name="eager-control-drain", daemon=True)
                    drain_thread.start()
                    try:
                        plateau(progress, child, report, deadline)
                    except OracleFailure as rejection:
                        result["oracle_rejection"] = rejection.details
                    else:
                        raise AssertionError("eager buffering escaped the paused-consumer oracle")
                    drain_thread.join(remaining(deadline))
                    assert not drain_thread.is_alive(), (
                        f"eager control drain exceeded deadline: produced={progress.read()}, "
                        f"hidden_drained={drained['bytes']}, rejection={result.get('oracle_rejection')}"
                    )
                    assert drained["error"] is None, drained["error"]
                    assert drained["bytes"] == TOTAL and drained["hash"].hexdigest() == expected_hash()
                    spool.flush()
                    assert spool.tell() == TOTAL
                    progress.read()
                    try:
                        progress.assert_queue_bound()
                    except OracleFailure as rejection:
                        result["final_queue_rejection"] = rejection.details
                    else:
                        raise AssertionError("whole-stream spool escaped the accepted-byte bound")
                    result["hidden_spool_bytes"] = drained["bytes"]
                else:
                    digest = hashlib.sha256()
                    first = plateau(progress, child, report, deadline)
                    result["plateaus"].append(first)
                    result["rss_samples"].append(memory_sample(child, progress.pid))
                    if broken:
                        child.stdout.close()
                    else:
                        consume(child, progress, digest, deadline, TRANCHE)
                        resumed_deadline = min(deadline, time.monotonic() + 3)
                        while progress.read() <= first["produced_bytes"]:
                            progress.assert_queue_bound()
                            remaining(resumed_deadline)
                            time.sleep(0.025)
                        result["resumed_produced_bytes"] = progress.produced
                        second = plateau(progress, child, report, deadline)
                        result["plateaus"].append(second)
                        result["rss_samples"].append(memory_sample(child, progress.pid))
                        consume(child, progress, digest, deadline)
                        assert progress.consumed == TOTAL and digest.hexdigest() == expected_hash()
                        result["stdout_bytes"] = progress.consumed
                        result["stdout_sha256"] = digest.hexdigest()
                status = child.wait(timeout=remaining(deadline))
                observed = json.loads(report.read_text())
                progress.read()
                if broken:
                    assert status != 0, observed
                    assert observed["outcome"] == "failure" and observed["infrastructure_failure"], observed
                    assert observed["cleanup"]["outcome"] == "clean", observed
                    assert (observed["warm_calls"], observed["cold_calls"], observed["worker_starts"]) == (1, 0, 1), observed
                    assert observed["application_statuses"] == [], observed
                    assert progress.produced < TOTAL and progress.consumed == 0
                else:
                    validate_report(observed, status)
                    assert progress.produced == TOTAL
                result["report"] = observed
                result["producer_pid"] = progress.pid
                result["maximum_produced_minus_consumed"] = progress.maximum_queued
            finally:
                failure = sys.exc_info()[1]
                stop_child(child)
                child.stdout.close()
                if drain_thread is not None:
                    drain_thread.join(timeout=3)
                    assert not drain_thread.is_alive(), "eager control drain did not stop"
                if spool is not None:
                    spool.close()
                result["diagnostics"] = (directory / "diagnostics").read_text(errors="replace")
                if failure is not None:
                    result["failure"] = {"type": type(failure).__name__, "message": str(failure)}
                    result["last_produced_bytes"] = progress.produced
                    result["last_consumed_bytes"] = progress.consumed
                    result["hidden_drained_bytes"] = drained["bytes"]
                    if report.exists():
                        result["report"] = json.loads(report.read_text())
                    print(json.dumps({"case_failure": result}), file=sys.stderr, flush=True)
        result["elapsed_seconds"] = time.monotonic() - started
        result["owned_spool_cleaned_on_return"] = True
    return result


def runtime_binding(args):
    environment = {"PATH": "/usr/bin:/bin"}
    if args.node:
        fixture = ROOT / "tests/runtime/node/fixture-worker.mjs"
        identity = {"sdk": hashlib.sha256((ROOT / "sdk/node/index.mjs").read_bytes()).hexdigest(),
                    "fixture": hashlib.sha256(fixture.read_bytes()).hexdigest(), "node": str(args.node)}
        binding = {"version": 1, "name": "tool", "worker_argv": [str(args.node), str(fixture)],
                   "cold_argv": ["/bin/false"]}
        if sys.platform == "darwin":
            probe = subprocess.run([str(args.node), "-p", "process.env.__CF_USER_TEXT_ENCODING ?? ''"],
                                   env=environment, capture_output=True, check=True, timeout=5)
            environment["__CF_USER_TEXT_ENCODING"] = os.fsdecode(probe.stdout.removesuffix(b"\n"))
        runtime = "node"
    else:
        manifest = json.loads(args.jvm_fixture.read_text())
        assert manifest["schema_version"] == 1
        binding = dict(manifest["binding"])
        identity = {"prepared_artifact_id": binding["artifact_id"], "artifacts": manifest["artifacts"]}
        runtime = "jvm"
    binding.update({"startup_timeout_ms": 5000, "session_timeout_ms": CASE_SECONDS * 1000,
                    "cleanup_timeout_ms": 2000})
    binding["artifact_id"] = "sha256:" + hashlib.sha256(json.dumps(identity, sort_keys=True).encode()).hexdigest()
    return runtime, binding, environment


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mdrun", type=Path, required=True)
    runtime = parser.add_mutually_exclusive_group(required=True)
    runtime.add_argument("--node", type=Path)
    runtime.add_argument("--jvm-fixture", type=Path)
    parser.add_argument("--case", choices=("all", "bounded", "eager", "broken"), default="all",
                        help="all checks bounded success, eager rejection and broken-reader failure; eager alone exposes the expected failing oracle")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    for path in (args.mdrun, args.node, args.jvm_fixture):
        if path is not None:
            assert path.is_absolute() and path.is_file(), path
    started = time.monotonic()
    runtime, binding, environment = runtime_binding(args)
    result = {"runtime": runtime, "host_sha256": hashlib.sha256(args.mdrun.read_bytes()).hexdigest(),
              "binding_artifact_id": binding["artifact_id"], "cases": [],
              "limits": "RSS is supplementary; the accepted-byte bound and a separate finite-queue source audit establish bounded transport."}
    for case in (["bounded", "eager", "broken"] if args.case == "all" else [args.case]):
        result["cases"].append(run_case(args.mdrun, binding, environment, case, started + 60))
    result["elapsed_seconds"] = time.monotonic() - started
    assert result["elapsed_seconds"] <= 60, result["elapsed_seconds"]
    encoded = json.dumps(result, indent=2) + "\n"
    if args.output:
        args.output.write_text(encoded)
    print(encoded, end="")
    return 1 if args.case == "eager" else 0


if __name__ == "__main__":
    raise SystemExit(main())
