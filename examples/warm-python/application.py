"""An ordinary SDK-free word-count CLI, also callable by the warm adapter."""

import argparse
from pathlib import Path
import sys


def count_file(path: Path, stdout, stderr) -> int:
    try:
        text = path.read_text(encoding="utf-8")
    except (OSError, UnicodeError) as error:
        stderr.write(f"wordcount: {error}\n".encode("utf-8"))
        return 1
    stdout.write(f"lines={len(text.splitlines())} words={len(text.split())}\n".encode("ascii"))
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description="Count lines and words in a UTF-8 file.")
    parser.add_argument("file", type=Path)
    args = parser.parse_args()
    return count_file(args.file, sys.stdout.buffer, sys.stderr.buffer)


if __name__ == "__main__":
    raise SystemExit(main())
