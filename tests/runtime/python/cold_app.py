"""SDK-free packaged application oracle for native cold/warm qualification."""

import argparse
import os
from pathlib import Path
import sys


def main():
    parser = argparse.ArgumentParser()
    commands = parser.add_subparsers(dest="command", required=True)
    for command in ("ignore", "fail", "binary", "echo"):
        commands.add_parser(command)
    commands.add_parser("read_file").add_argument("path", type=Path)
    commands.add_parser("refuse").add_argument("effect", type=Path)
    for command in ("system_exit_zero", "abrupt_exit_zero"):
        commands.add_parser(command).add_argument("effect", type=Path)
    arguments = parser.parse_args()
    if arguments.command == "ignore":
        sys.stdout.buffer.write(b"ok")
    elif arguments.command == "fail":
        return 17
    elif arguments.command == "binary":
        sys.stdout.buffer.write(b"\x00\xff\x01")
        sys.stdout.buffer.flush()
        sys.stderr.buffer.write(b"\xfe\x00")
        sys.stderr.buffer.flush()
        sys.stdout.buffer.write(b"\x02")
    elif arguments.command == "echo":
        while chunk := sys.stdin.buffer.read(32768):
            sys.stdout.buffer.write(chunk)
    elif arguments.command == "read_file":
        with arguments.path.open("rb") as source:
            while chunk := source.read(32768):
                sys.stdout.buffer.write(chunk)
    elif arguments.command == "refuse":
        with arguments.effect.open("ab") as effect:
            effect.write(b"cold refusal executed\n")
        sys.stdout.buffer.write(b"cold refusal\n")
        return 23
    elif arguments.command in ("system_exit_zero", "abrupt_exit_zero"):
        with arguments.effect.open("ab") as effect:
            effect.write(b"cold\n")
        if arguments.command == "system_exit_zero":
            raise SystemExit(0)
        os._exit(0)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
