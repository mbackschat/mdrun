"""Explicit cooperative adapter for the example's single file-counting command."""

import argparse
from pathlib import Path
import sys

DIRECTORY = Path(__file__).resolve().parent
sys.path.insert(0, str(DIRECTORY))
sys.path.insert(0, str(DIRECTORY / "sdk"))

from application import count_file
from mdrun_sdk import Application, Eligible, Invocation, Refused, run_worker


class WordCount(Application):
    def classify(self, argv: tuple[str, ...]) -> Eligible | Refused:
        return Eligible() if len(argv) == 1 and not argv[0].startswith("-") else Refused("unsupported_arguments")

    def invoke(self, request: Invocation) -> int:
        return count_file(Path(request.argv[0]), request.stdout, request.stderr)

    def reset(self, request: Invocation) -> None:
        # count_file closes its input and retains no requests, mutable state or work.
        pass


parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("config", type=Path)
args = parser.parse_args()
try:
    run_worker(args.config, WordCount)
except BaseException:
    raise SystemExit(70)
