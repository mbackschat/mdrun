"""Check literal demo output, successful application calls and actual execution routes."""

import argparse
import json
from pathlib import Path
import re


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def check_session(report: dict, artifact: str, mode: str, calls: int) -> None:
    require(report["version"] == 1 and report["artifact_id"] == artifact, "unexpected report identity")
    require((report["outcome"], report["infrastructure_failure"], report["cleanup"]["outcome"]) ==
            ("success", None, "clean"), "session failed or cleanup was not clean")
    require(report["author_status"] == {"exit_code": 0, "signal": None}, "author failed")
    expected = (calls, 0, 1) if mode == "fast" else (0, calls, 0)
    require((report["warm_calls"], report["cold_calls"], report["worker_starts"]) == expected,
            f"expected warm/cold/workers={expected}")
    require(report["cold_reasons"] == ({} if mode == "fast" else {"explicit_cold": calls}),
            "unexpected cold route reason")
    statuses = report["application_statuses"]
    require(len(statuses) == calls and all(
        (status["exit_code"], status["signal"], status["route"]) ==
        (0, None, "warm" if mode == "fast" else "cold") for status in statuses),
        "application results disagree with the oracle")


def check(directory: Path, report_path: Path, mode: str) -> None:
    expected = {"first.md": ["lines=1 words=3\n", "lines=1 words=5\n"],
                "second.md": ["lines=1 words=2\n"]}
    for name, captures in expected.items():
        document = (directory / name).read_text(encoding="utf-8")
        require(re.findall(r"```output\n(.*?)```", document, re.DOTALL) == captures,
                f"{name} output differs from the literal oracle")
    require((directory / "first.txt").read_bytes() == b"red green blue gold silver\n" and
            (directory / "second.txt").read_bytes() == b"one two\n", "unexpected final input contents")
    artifact = json.loads((directory / "binding.json").read_text(encoding="utf-8"))["artifact_id"]
    report = json.loads(report_path.read_text(encoding="utf-8"))
    if mode == "batch":
        require((report["version"], report["binding"]["artifact_id"], report["outcome"],
                 report["infrastructure_failure"], report["cleanup"]["outcome"]) ==
                (1, artifact, "success", None, "clean"), "batch failed or cleanup was not clean")
        require((report["concurrency"]["requested"], report["concurrency"]["effective"]) == (2, 2),
                "expected two batch slots")
        documents = report["documents"]
        require(len(documents) == 2, "expected two batch documents")
        for document, calls in zip(documents, (2, 1)):
            require(document["state"] == "completed" and document["status"] == 0, "document failed")
            check_session(document["session"], artifact, "fast", calls)
        print("Verified literal output: warm=3 cold=0 workers=2 cleanup=clean")
    else:
        check_session(report, artifact, mode, 3)
        print("Verified literal output: " +
              ("warm=3 cold=0 workers=1" if mode == "fast" else "warm=0 cold=3 workers=0") + " cleanup=clean")


parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--directory", type=Path, required=True)
parser.add_argument("--report", type=Path, required=True)
parser.add_argument("--mode", choices=("fast", "cold", "batch"), required=True)
args = parser.parse_args()
try:
    check(args.directory, args.report, args.mode)
except (OSError, ValueError, KeyError, TypeError) as error:
    parser.exit(1, f"Demo check failed: {error}\n")
