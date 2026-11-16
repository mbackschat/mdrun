"""One synchronous file-reading application shared by the cold and warm entrypoints."""

from pathlib import Path


def read_file(argv, output):
    if len(argv) != 2 or argv[0] != "read_file":
        return 2
    with Path(argv[1]).open("rb") as source:
        while chunk := source.read(32768):
            output.write(chunk)
    return 0
