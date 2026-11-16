#!/bin/sh
set -eu

mdrun=$1

"$mdrun" init first.md 'Fresh inputs in one Python worker'
"$mdrun" exec first.md sh 'printf "red green blue\n" > first.txt && wordcount first.txt'
"$mdrun" exec first.md sh 'printf "red green blue gold silver\n" > first.txt && wordcount first.txt'

"$mdrun" init second.md 'An independent document'
"$mdrun" exec second.md sh 'printf "one two\n" > second.txt && wordcount second.txt'
