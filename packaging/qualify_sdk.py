"""Qualify pinned SDK preparation and real-host cold/warm consumers outside the checkout."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import time
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def run(argv, cwd, *, env=None):
    environment = {"PATH": "/usr/bin:/bin"} if env is None else env.copy()
    environment["PWD"] = str(Path(cwd).resolve())
    return subprocess.run(argv, cwd=cwd, env=environment, check=True, capture_output=True, timeout=60).stdout


def jvm_application(java_home, directory):
    manifest = json.loads((directory / "build-manifest.json").read_text())
    sdk_classpath = os.pathsep.join(str(directory / entry["path"]) for entry in manifest["classpath"])
    for kind, sources in (("cold", ["ReadFile.java", "ColdMain.java"]),
                          ("worker", ["ReadFile.java", "WorkerMain.java"])):
        classes = directory / (kind + "-classes")
        classes.mkdir()
        command = [str(java_home / "bin/javac"), "--release", "21", "-proc:none", "-Xlint:all", "-Werror",
                   "-d", str(classes)]
        if kind == "worker":
            command.extend(["-cp", sdk_classpath])
        run([*command, *(str(directory / name) for name in sources)], directory)
        with zipfile.ZipFile(directory / (kind + ".jar"), "w", compression=zipfile.ZIP_DEFLATED) as archive:
            for path in sorted(classes.rglob("*.class")):
                entry = zipfile.ZipInfo(path.relative_to(classes).as_posix())
                archive.writestr(entry, path.read_bytes())
        shutil.rmtree(classes)
    java = str(java_home / "bin/java")
    return ([java, "-cp", os.pathsep.join([str(directory / "worker.jar"), sdk_classpath]),
             "io.github.mbackschat.mdrun.consumer.WorkerMain"],
            [java, "-cp", str(directory / "cold.jar"), "io.github.mbackschat.mdrun.consumer.ColdMain"])


def artifact_identity(artifacts, binding):
    identity = {"artifacts": artifacts, "binding": binding}
    return "sha256:" + hashlib.sha256(json.dumps(identity, sort_keys=True).encode()).hexdigest()


def check_report(report, mode, calls):
    if (report["outcome"], report["infrastructure_failure"], report["cleanup"]["outcome"],
            report["warm_calls"], report["cold_calls"], report["worker_starts"]) != (
            "success", None, "clean", calls if mode == "fast" else 0,
            calls if mode == "cold" else 0, 1 if mode == "fast" else 0):
        raise ValueError(f"unexpected {mode} consumer routes or session outcome: {report}")
    statuses = report["application_statuses"]
    if len(statuses) != calls or any((status["exit_code"], status["signal"], status["route"]) != (
            0, None, "warm" if mode == "fast" else "cold") for status in statuses):
        raise ValueError("consumer application statuses disagree with their oracle")


def qualify(archives, version, java_home, node, python, mdrun, output):
    archives, java_home, output = Path(archives).resolve(), Path(java_home).resolve(), Path(output).absolute()
    if output.exists():
        raise ValueError("--output workspace must not exist")
    output.mkdir(parents=True)
    if run([mdrun, "--version"], output) != f"mdrun {version}\n".encode():
        raise ValueError("native host and SDK release versions must match")
    results = {"schema_version": 1, "version": version, "archives": {}, "sessions": {}}
    environment = {"LC_ALL": "C", "LANG": "C", "PYTHONCOERCECLOCALE": "0",
                   "PYTHONUTF8": "1", "PYTHONDONTWRITEBYTECODE": "1"}
    if sys.platform == "darwin":
        # The selected macOS Node runtime may establish this variable during startup.
        environment["__CF_USER_TEXT_ENCODING"] = run(
            [node, "-p", "process.env.__CF_USER_TEXT_ENCODING ?? ''"], output).decode().rstrip("\n")
    helper = output / "prepare_sdk.py"
    shutil.copyfile(ROOT / "scripts/prepare_sdk.py", helper)
    for runtime in ("jvm", "node", "python"):
        started = time.monotonic_ns()
        filename = f"mdrun-sdk-{runtime}-v{version}.tar.gz"
        archive = archives / filename
        digest = hashlib.sha256(archive.read_bytes()).hexdigest()
        results["archives"][runtime] = {"filename": filename, "sha256": digest}
        directory = output / runtime
        # Subsequent preparation, compilation and execution use only copied consumer artifacts.
        run([python, str(helper), "--runtime", runtime, "--version", version, "--sha256", digest,
             "--archive", str(archive), "--output", str(directory)], output)
        fixture = ROOT / "packaging/fixtures" / runtime
        for path in sorted(fixture.iterdir()):
            if path.is_file():
                shutil.copyfile(path, directory / path.name)
        if runtime == "jvm":
            worker, cold = jvm_application(java_home, directory)
        else:
            executable = node if runtime == "node" else python
            suffix = ".mjs" if runtime == "node" else ".py"
            cold_directory = directory / "cold"
            cold_directory.mkdir()
            (directory / ("cold" + suffix)).replace(cold_directory / ("cold" + suffix))
            shutil.copyfile(directory / ("application" + suffix), cold_directory / ("application" + suffix))
            worker = [executable, str(directory / ("worker" + suffix))]
            cold = [executable, *(["-S"] if runtime == "python" else []), str(cold_directory / ("cold" + suffix))]
            if runtime == "python" and run(
                    [python, "-S", "-c", "from importlib.util import find_spec; print(find_spec('mdrun_sdk'))"],
                    cold_directory) != b"None\n":
                raise ValueError("cold application interpreter can import the cooperative Python SDK")
        identities = {path.relative_to(directory).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
                      for path in sorted(directory.rglob("*")) if path.is_file()}
        binding = {"version": 1, "name": "tool", "worker_argv": worker, "cold_argv": cold,
                   # These read_file-only applications never inspect either shell bookkeeping variable.
                   "warm_ignore_env": ["SHLVL", "_"],
                   "context": {"environment": environment},
                   "startup_timeout_ms": 10000, "session_timeout_ms": 30000, "cleanup_timeout_ms": 3000}
        binding["artifact_id"] = artifact_identity(identities, binding)
        binding_path = directory / "binding.json"
        binding_path.write_text(json.dumps(binding, indent=2, sort_keys=True) + "\n")
        preparation_ms = (time.monotonic_ns() - started) / 1_000_000
        results["archives"][runtime]["preparation_ms"] = preparation_ms
        results["archives"][runtime]["artifact_id"] = binding["artifact_id"]
        payload = b"independent pinned SDK consumer oracle\n"
        input_path = directory / "input.txt"
        input_path.write_bytes(payload)
        # Independently launched cold has no SDK on its module/class path and no host involvement.
        if run([*cold, "read_file", str(input_path)], directory) != payload:
            raise ValueError("independently launched cold output disagrees with its literal oracle")
        document = directory / "calls.md"
        command = "tool read_file " + shlex.quote(str(input_path))
        document.write_text("# Pinned SDK consumer\n\n" + (
            "```sh\n" + command + "\n```\n\n```output\n" + payload.decode() + "```\n\n") * 2)
        sessions = {}
        for mode in ("cold", "fast"):
            report_path = directory / (mode + ".json")
            options = ["--require-warm"] if mode == "fast" else []
            run([mdrun, "verify", str(document), "--binding", str(binding_path), "--mode", mode,
                 "--report", str(report_path), *options], directory)
            report = json.loads(report_path.read_text())
            check_report(report, mode, 2)
            sessions[mode] = report
        results["sessions"][runtime] = sessions
    (output / "qualification.json").write_text(json.dumps(results, indent=2, sort_keys=True) + "\n")
    return results


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archives", type=Path, required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--java-home", type=Path, required=True)
    parser.add_argument("--node", type=Path, required=True)
    parser.add_argument("--python", type=Path, required=True)
    parser.add_argument("--mdrun", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True, help="new persistent consumer workspace")
    args = parser.parse_args()
    try:
        result = qualify(args.archives, args.version, args.java_home, str(args.node.resolve()), str(args.python.resolve()),
                         str(args.mdrun.resolve()), args.output)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        detail = error.stderr.decode(errors="replace") if isinstance(error, subprocess.CalledProcessError) else str(error)
        parser.exit(1, f"SDK consumer qualification failed: {detail}\n")
    print(json.dumps({"version": result["version"], "archives": result["archives"],
                      "runtimes": sorted(result["sessions"])}, sort_keys=True))


if __name__ == "__main__":
    main()
