"""An SDK-free stdin word counter, also callable by the warm adapter."""

import argparse
import sys


def count_stdin(stdin, stdout, stderr) -> int:
    try:
        data = bytearray()
        while chunk := stdin.read(65536):
            data.extend(chunk)
        text = data.decode("utf-8")
    except (OSError, UnicodeError) as error:
        stderr.write(f"wordcount: {error}\n".encode("utf-8"))
        return 1
    stdout.write(f"lines={len(text.splitlines())} words={len(text.split())}\n".encode("ascii"))
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description="Count lines and words from UTF-8 stdin.")
    parser.parse_args()
    return count_stdin(sys.stdin.buffer, sys.stdout.buffer, sys.stderr.buffer)


if __name__ == "__main__":
    raise SystemExit(main())
