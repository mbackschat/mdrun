from pathlib import Path
import sys
from application import read_file
from mdrun_sdk import Application, Eligible, Refused, run_worker


class Reader(Application):
    def classify(self, argv):
        return Eligible() if len(argv) == 2 and argv[0] == "read_file" else Refused("unsupported_command")

    def invoke(self, request):
        return read_file(request.argv, request.stdout)

    def reset(self, request):
        # read_file closes its one file before returning and owns no detached work.
        pass


try:
    run_worker(Path(sys.argv[1]), Reader)
except BaseException:
    raise SystemExit(70)
