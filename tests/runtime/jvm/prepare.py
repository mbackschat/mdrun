"""Prepare independent cold and cooperative JVM fixtures outside the consumer checkout."""

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location("mdrun_prepare_jvm", ROOT / "scripts/prepare_jvm.py")
sdk_builder = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(sdk_builder)
PACKAGE = "io/github/mbackschat/mdrun/fixture/"


def prepare(java_home, jackson_core, output, *, download=False, cache_dir=None):
    java_home, output = Path(java_home).resolve(), Path(output).resolve()
    manifest = sdk_builder.prepare(java_home, jackson_core, output, download=download, cache_dir=cache_dir)
    classpath = os.pathsep.join(str(output / entry["path"]) for entry in manifest["classpath"])
    with tempfile.TemporaryDirectory(prefix="fixture-", dir=output) as temporary:
        classes = Path(temporary)
        subprocess.run([str(java_home / "bin/javac"), "--release", "21", "-proc:none", "-Xlint:all", "-Werror",
                        "-cp", classpath, "-d", str(classes),
                        *map(str, sorted(Path(__file__).parent.glob("*.java")))], check=True, timeout=60)
        for archive_name, entries in [("cold.jar", ["Example.class", "ColdMain.class"]),
                                      ("worker.jar", ["Example.class", "Fixture.class", "Lifecycle.class"])]:
            with zipfile.ZipFile(output / archive_name, "w", compression=zipfile.ZIP_DEFLATED) as archive:
                for name in entries:
                    entry = zipfile.ZipInfo(PACKAGE + name)
                    archive.writestr(entry, (classes / PACKAGE / name).read_bytes())
    worker_classpath = os.pathsep.join([str(output / "worker.jar"), classpath])
    java = str(java_home / "bin/java")
    files = ["cold.jar", "worker.jar", *(entry["path"] for entry in manifest["classpath"])]
    identities = {name: hashlib.sha256((output / name).read_bytes()).hexdigest() for name in files}
    binding = {"version": 1, "name": "tool",
               "worker_argv": [java, "-cp", worker_classpath, "io.github.mbackschat.mdrun.fixture.Fixture"],
               "cold_argv": [java, "-cp", str(output / "cold.jar"), "io.github.mbackschat.mdrun.fixture.ColdMain"],
               "startup_timeout_ms": 5000, "session_timeout_ms": 20000, "cleanup_timeout_ms": 2000}
    identity = {"artifacts": identities, "launch": binding, "jdk_release": (java_home / "release").read_text()}
    binding["artifact_id"] = "sha256:" + hashlib.sha256(json.dumps(identity, sort_keys=True).encode()).hexdigest()
    result = {"schema_version": 1, "binding": binding, "artifacts": identities}
    (output / "fixture-manifest.json").write_text(json.dumps(result, indent=2) + "\n")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--java-home", type=Path, required=True)
    dependency = parser.add_mutually_exclusive_group(required=True)
    dependency.add_argument("--jackson-core", type=Path)
    dependency.add_argument("--download", action="store_true")
    parser.add_argument("--cache-dir", type=Path, help="reuse verified dependency bytes with --download")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(prepare(args.java_home, args.jackson_core,
                             args.output, download=args.download, cache_dir=args.cache_dir), sort_keys=True))


if __name__ == "__main__":
    main()
