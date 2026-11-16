"""Qualify a hash-pinned Python SDK wheel installed by uv in an application virtual environment."""

import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import shlex
import shutil
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
RELEASE_ROOT = "https://github.com/mbackschat/mdrun/releases/download"


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


shared = load("wheel_session_qualification", ROOT / "packaging/qualify_sdk.py")
archive_consumer = load("wheel_version_validation", ROOT / "scripts/prepare_sdk.py")


def requirement(version, digest, *, wheel=None):
    archive_consumer.validate_version(version)
    if not isinstance(digest, str) or not archive_consumer.DIGEST.fullmatch(digest):
        raise ValueError("wheel digest must be an exact lowercase SHA-256")
    filename = f"mdrun_sdk-{version}-py3-none-any.whl"
    if wheel is None:
        source = f"{RELEASE_ROOT}/v{version}/{filename}"
    else:
        wheel = Path(wheel).absolute()
        if wheel.name != filename:
            raise ValueError("wheel filename must match its pinned SDK version")
        source = wheel.as_uri()
    return f"mdrun-sdk @ {source} --hash=sha256:{digest}\n"


def install(version, digest, python, uv, output, *, wheel=None):
    """Use uv's standard installer; application and system interpreters remain separate."""
    pinned = requirement(version, digest, wheel=wheel)
    output = Path(output).absolute()
    output.mkdir(parents=True)
    requirements = output / "requirements.txt"
    requirements.write_text(pinned)
    virtualenv = output / ".venv"
    shared.run([uv, "venv", "--no-config", "--no-cache", "--no-python-downloads", "--python", python,
                str(virtualenv)], output)
    interpreter = virtualenv / "bin/python"
    shared.run([uv, "pip", "install", "--no-config", "--no-cache", "--no-python-downloads", "--no-index", "--no-deps",
                "--require-hashes", "--link-mode", "copy", "--python", str(interpreter),
                "--requirement", str(requirements)], output)
    return interpreter


def qualify(version, digest, python, uv, mdrun, output, *, wheel=None):
    output = Path(output).absolute()
    started = time.monotonic_ns()
    interpreter = install(version, digest, python, uv, output, wheel=wheel)
    if shared.run([mdrun, "--version"], output) != f"mdrun {version}\n".encode():
        raise ValueError("native host and Python SDK wheel versions must match")
    probe = ("import json, mdrun_sdk; from importlib.metadata import version; "
             "print(json.dumps({'version': version('mdrun-sdk'), 'module': mdrun_sdk.__file__}))")
    installed = json.loads(shared.run([str(interpreter), "-I", "-c", probe], output))
    if installed["version"] != version or not Path(installed["module"]).resolve().is_relative_to((output / ".venv").resolve()):
        raise ValueError("Python SDK import or installed distribution version differs from the isolated wheel")
    source = ROOT / "packaging/fixtures/python"
    for name in ("application.py", "worker.py"):
        shutil.copyfile(source / name, output / name)
    cold_directory = output / "cold"
    cold_directory.mkdir()
    for name in ("application.py", "cold.py"):
        shutil.copyfile(source / name, cold_directory / name)
    if shared.run([python, "-S", "-c", "from importlib.util import find_spec; print(find_spec('mdrun_sdk'))"],
                  cold_directory) != b"None\n":
        raise ValueError("cold application interpreter can import the cooperative SDK")
    environment = {"LC_ALL": "C", "LANG": "C", "PYTHONCOERCECLOCALE": "0", "PYTHONUTF8": "1",
                   "PYTHONDONTWRITEBYTECODE": "1"}
    binding = {"version": 1, "name": "tool", "worker_argv": [str(interpreter), str(output / "worker.py")],
               "cold_argv": [python, "-S", str(cold_directory / "cold.py")], "warm_ignore_env": ["SHLVL", "_"],
               "context": {"environment": environment}, "startup_timeout_ms": 10000,
               "session_timeout_ms": 30000, "cleanup_timeout_ms": 3000}
    identities = {path.relative_to(output).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
                  for path in sorted(output.rglob("*")) if path.is_file()}
    binding["artifact_id"] = shared.artifact_identity(identities, binding)
    binding_path = output / "binding.json"
    binding_path.write_text(json.dumps(binding, indent=2, sort_keys=True) + "\n")
    preparation_ms = (time.monotonic_ns() - started) / 1_000_000
    payload = b"independent uv-installed SDK consumer oracle\n"
    input_path = output / "input.txt"
    input_path.write_bytes(payload)
    if shared.run([*binding["cold_argv"], "read_file", str(input_path)], output) != payload:
        raise ValueError("SDK-free cold output disagrees with its independent literal oracle")
    document = output / "calls.md"
    command = "tool read_file " + shlex.quote(str(input_path))
    document.write_text("# uv-installed SDK consumer\n\n" + (
        "```sh\n" + command + "\n```\n\n```output\n" + payload.decode() + "```\n\n") * 2)
    sessions = {}
    for mode in ("cold", "fast"):
        report_path = output / (mode + ".json")
        options = ["--require-warm"] if mode == "fast" else []
        shared.run([mdrun, "verify", str(document), "--binding", str(binding_path), "--mode", mode,
                    "--report", str(report_path), *options], output)
        report = json.loads(report_path.read_text())
        shared.check_report(report, mode, 2)
        sessions[mode] = report
    result = {"schema_version": 1, "version": version, "wheel_sha256": digest,
              "source": "release" if wheel is None else "local", "installed": installed,
              "uv": shared.run([uv, "--version"], output).decode().strip(),
              "artifact_id": binding["artifact_id"], "preparation_ms": preparation_ms, "sessions": sessions}
    (output / "qualification.json").write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--wheel", type=Path)
    source.add_argument("--release", action="store_true", help="install the exact GitHub release wheel URL")
    parser.add_argument("--version", required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--python", type=Path, required=True)
    parser.add_argument("--uv", type=Path, required=True)
    parser.add_argument("--mdrun", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        result = qualify(args.version, args.sha256, str(args.python.resolve()), str(args.uv.resolve()),
                         str(args.mdrun.resolve()), args.output, wheel=args.wheel)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        detail = error.stderr.decode(errors="replace") if isinstance(error, subprocess.CalledProcessError) else str(error)
        parser.exit(1, f"Python wheel consumer qualification failed: {detail}\n")
    print(json.dumps({"version": result["version"], "wheel_sha256": result["wheel_sha256"],
                      "source": result["source"], "artifact_id": result["artifact_id"]}, sort_keys=True))


if __name__ == "__main__":
    main()
