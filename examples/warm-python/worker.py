"""Explicit cooperative adapter for the example's stdin word-counting command."""

import argparse
from pathlib import Path
import sys

DIRECTORY = Path(__file__).resolve().parent
sys.path.insert(0, str(DIRECTORY))
sys.path.insert(0, str(DIRECTORY / "sdk"))

from application import count_stdin
from mdrun_sdk import Application, Eligible, Invocation, Refused, run_worker


class WordCount(Application):
    def classify(self, argv: tuple[str, ...]) -> Eligible | Refused:
        return Eligible() if not argv else Refused("unsupported_arguments")

    def invoke(self, request: Invocation) -> int:
        return count_stdin(request.stdin, request.stdout, request.stderr)

    def reset(self, request: Invocation) -> None:
        # count_stdin finishes its reads/writes and retains no requests, state or work.
        pass


parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("config", type=Path)
args = parser.parse_args()
try:
    run_worker(args.config, WordCount)
except BaseException:
    raise SystemExit(70)
