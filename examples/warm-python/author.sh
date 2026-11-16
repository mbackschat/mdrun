#!/bin/sh
set -eu

mdrun=$1

"$mdrun" init first.md 'Fresh inputs in one Python worker'
"$mdrun" exec first.md sh 'echo "red green blue" | wordcount'
"$mdrun" exec first.md sh 'echo "red green blue gold silver" | wordcount'

"$mdrun" init second.md 'An independent document'
"$mdrun" exec second.md sh 'echo "one two" | wordcount'
