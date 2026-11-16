import sys
from application import read_file

raise SystemExit(read_file(sys.argv[1:], sys.stdout.buffer))
