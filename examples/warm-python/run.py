"""Record, replay and check the Python warm demo with an already installed mdrun."""

import argparse
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mdrun", type=Path, help="existing executable (default: PATH, then target/release/mdrun)")
    parser.add_argument("--output", type=Path, help="new directory outside the checkout (default: persistent temporary directory)")
    args = parser.parse_args()
    if sys.version_info < (3, 12):
        parser.error("Python 3.12 or later is required")
    source = Path(__file__).resolve().parent
    binary = (args.mdrun or Path(shutil.which("mdrun") or source.parents[1] / "target/release/mdrun")).expanduser().resolve()
    if not binary.is_file() or not os.access(binary, os.X_OK):
        parser.error("mdrun executable not found; install it or pass --mdrun /path/to/mdrun")
    output = (args.output.expanduser().absolute() if args.output else
              Path(tempfile.mkdtemp(prefix="mdrun-warm-demo-")).resolve() / "demo")
    print(f"Using mdrun: {binary}\nResults: {output}", flush=True)

    def run(label: str, command: list[str], expected: int = 0) -> None:
        print(f"\n{label}", flush=True)
        log = output / (label.replace(" ", "-").lower() + ".log")
        with log.open("w", encoding="utf-8") as stream:
            try:
                result = subprocess.run(command, stdout=stream, stderr=subprocess.STDOUT, timeout=60)
            except subprocess.TimeoutExpired as error:
                raise RuntimeError(f"{label} exceeded 60 seconds; see {log}") from error
        print(log.read_text(encoding="utf-8"), end="", flush=True)
        if result.returncode != expected:
            raise RuntimeError(f"{label} exited with status {result.returncode}; expected {expected}; see {log}")

    try:
        subprocess.run([sys.executable, str(source / "prepare.py"), "--output", str(output)],
                       check=True, timeout=60)
        output = output.resolve()
        host = [str(binary), "--workdir", str(output)]
        binding = ["--binding", str(output / "binding.json")]
        checker = [sys.executable, str(source / "check.py"), "--directory", str(output)]
        warm, cold, batch = (str(output / name) for name in ("warm.json", "cold.json", "batch.json"))
        run("Warm recording", host + ["session"] + binding + [
            "--require-warm", "--report", warm, "--", "sh", str(output / "author.sh"), str(binary)])
        run("Warm check", checker + ["--report", warm, "--mode", "fast"])
        run("Cold replay", host + ["session"] + binding + [
            "--mode", "cold", "--report", cold, "--", "sh", "-c",
            '"$1" verify first.md && "$1" verify second.md', "sh", str(binary)])
        run("Cold check", checker + ["--report", cold, "--mode", "cold"])
        run("Reject cold as warm (expected failure)", checker + ["--report", cold, "--mode", "fast"], expected=1)
        run("Two document warm batch", host + ["verify-many"] + binding + [
            "--require-warm", "--jobs", "2", "--report", batch,
            str(output / "first.md"), str(output / "second.md")])
        run("Batch check", checker + ["--report", batch, "--mode", "batch"])
    except (OSError, RuntimeError, subprocess.SubprocessError) as error:
        parser.exit(1, f"Demo failed: {error}\nResults: {output}\n")
    print(f"\nDemo passed: warm recording, cold replay, warm batch and negative control.\n"
          f"Documents: {output / 'first.md'} and {output / 'second.md'}\n"
          f"Reports and logs: {output}")


if __name__ == "__main__":
    main()
