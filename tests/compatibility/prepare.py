"""Build the exact Showboat oracle from a clean, separately fetched checkout."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

COMMIT = "d531261b8faf0c388b02c7891d50f1f47c3e2b52"
GO_VERSION = "go1.27.4"
FLAGS = ["-trimpath", "-buildvcs=false", "-ldflags=-s -w -X main.version=0.6.1"]


def checked_source(source):
    def git(*args):
        return subprocess.check_output(["git", "-C", str(source), *args], text=True).strip()
    if git("rev-parse", "HEAD") != COMMIT or git("rev-parse", "v0.6.1^{commit}") != COMMIT:
        raise ValueError("source must be the pinned Showboat v0.6.1 commit")
    if git("status", "--porcelain", "--untracked-files=all"):
        raise ValueError("oracle checkout must be clean, including untracked files")


def verify_identity(binary):
    identity = json.loads(binary.with_suffix(".identity.json").read_text())
    if identity["commit"] != COMMIT or identity["go_version"].split()[2] != GO_VERSION:
        raise ValueError("unexpected oracle source/toolchain identity")
    if identity["sha256"] != hashlib.sha256(binary.read_bytes()).hexdigest():
        raise ValueError("oracle binary differs from its prepared identity")
    if subprocess.check_output([str(binary), "--version"]) != b"0.6.1\n":
        raise ValueError("oracle must report Showboat 0.6.1")
    return identity


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    source, output = args.source.resolve(), args.output.resolve()
    checked_source(source)
    version = subprocess.check_output(["go", "version"], text=True).strip()
    if version.split()[2] != GO_VERSION:
        parser.error(f"oracle build requires {GO_VERSION}, found {version}")
    output.mkdir(parents=True, exist_ok=False)
    binary = output / "showboat"
    env = {**os.environ, "CGO_ENABLED": "0", "GOTOOLCHAIN": "local"}
    subprocess.run(["go", "build", *FLAGS, "-o", str(binary), "."], cwd=source, env=env, check=True)
    identity = {"commit": COMMIT, "version": "0.6.1", "go_version": version,
                "flags": FLAGS, "CGO_ENABLED": "0",
                "sha256": hashlib.sha256(binary.read_bytes()).hexdigest()}
    binary.with_suffix(".identity.json").write_text(json.dumps(identity, indent=2) + "\n")
    verify_identity(binary)
    print(json.dumps(identity, indent=2))


if __name__ == "__main__":
    main()
