"""Prepare the source-checkout Python demo in a new directory outside the repository."""

import argparse
import hashlib
import json
from pathlib import Path
import shutil
import sys


def prepare(output: Path) -> None:
    if sys.version_info < (3, 12):
        raise ValueError("Python 3.12 or later is required")
    source = Path(__file__).resolve().parent
    root = source.parents[1]
    if output.exists() or output.is_symlink():
        raise ValueError("--output must name a new directory")
    output = output.resolve()
    if output == root or root in output.parents:
        raise ValueError("--output must be outside the source checkout")
    output.mkdir()
    for name in ("application.py", "worker.py", "author.sh"):
        shutil.copyfile(source / name, output / name)
    sdk = output / "sdk" / "mdrun_sdk"
    sdk.mkdir(parents=True)
    shutil.copyfile(root / "sdk/python/mdrun_sdk/__init__.py", sdk / "__init__.py")
    python = str(Path(sys.executable).resolve())
    binding = {
        "version": 1,
        "name": "wordcount",
        "worker_argv": [python, "-I", "-S", "-B", str(output / "worker.py")],
        "cold_argv": [python, "-I", "-S", "-B", str(output / "application.py")],
        # Eligible calls depend only on UTF-8 stdin, not shell bookkeeping variables.
        "warm_ignore_env": ["SHLVL", "_"],
        "context": {"environment": {"LC_ALL": "C", "LANG": "C", "PWD": str(output), "OLDPWD": None}},
    }
    identity = {
        "binding": binding,
        "files": {path.relative_to(output).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
                  for path in sorted(output.rglob("*")) if path.is_file()},
        "python_sha256": hashlib.sha256(Path(python).read_bytes()).hexdigest(),
        "python_version": sys.version,
    }
    binding["artifact_id"] = "sha256:" + hashlib.sha256(
        json.dumps(identity, sort_keys=True).encode("utf-8")).hexdigest()
    (output / "binding.json").write_text(json.dumps(binding, indent=2) + "\n", encoding="utf-8")
    print(f"Prepared {output}")


parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--output", type=Path, required=True, help="new directory; its parent must exist")
args = parser.parse_args()
try:
    prepare(args.output.expanduser())
except (OSError, ValueError) as error:
    parser.exit(1, f"Preparation failed: {error}\n")
