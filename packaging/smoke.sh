#!/bin/sh
set -eu
binary=${1:?supply the absolute installed mdrun executable}
directory=$(mktemp -d)
trap 'rm -rf "$directory"' EXIT
trap 'exit 1' HUP INT TERM

run() {
    /usr/bin/env -i PATH="" "$binary" "$@"
}

printf 'package smoke\n' > "$directory/input.txt"
run init "$directory/demo.md" "Package qualification"
run note "$directory/demo.md" "Recorded by the installed executable."
run --workdir "$directory" exec "$directory/demo.md" /bin/sh '/bin/cat input.txt'
if ! grep -q '^package smoke$' "$directory/demo.md"; then
    echo 'recorded command output missing' >&2
    exit 1
fi
grep -q '^Recorded by the installed executable\.$' "$directory/demo.md"
run --workdir "$directory" verify "$directory/demo.md"
cp "$directory/demo.md" "$directory/original.md"
printf 'changed input\n' > "$directory/input.txt"
if run --workdir "$directory" verify "$directory/demo.md" > "$directory/diff.txt"; then
    echo 'verification accepted changed output' >&2
    exit 1
else
    test "$?" -eq 1
fi
cmp "$directory/original.md" "$directory/demo.md"
grep -q 'changed input' "$directory/diff.txt"
printf 'package document smoke passed\n'
