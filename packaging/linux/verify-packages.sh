#!/bin/sh
set -eu
directory=${1:?usage: verify-packages.sh ABSOLUTE_DIRECTORY}
case "$directory" in /*) ;; *) echo "use an absolute package directory" >&2; exit 1 ;; esac
script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)

found_deb=0
found_rpm=0
for package in "$directory"/*.deb "$directory"/*.rpm; do
    [ -f "$package" ] || continue
    name=$(basename "$package")
    case "$name" in
        *amd64.deb | *x86_64.rpm) platform=linux/amd64 ;;
        *arm64.deb | *aarch64.rpm) platform=linux/arm64 ;;
        *) echo "unknown architecture: $name" >&2; exit 1 ;;
    esac
    case "$name" in
        *.deb)
            found_deb=1
            docker run --rm --platform "$platform" -v "$directory:/packages:ro" -v "$script_directory/../smoke.sh:/package-smoke.sh:ro" -v "$script_directory/../../docs/man:/expected-man:ro" debian:13-slim \
                sh -euc 'dpkg -i "/packages/$1"; env -i /usr/bin/mdrun --version; sh /package-smoke.sh /usr/bin/mdrun; test -s /usr/share/doc/mdrun/THIRD_PARTY_NOTICES.txt; for page in /expected-man/*.1; do name=${page##*/}; test -s "/usr/share/man/man1/$name" || test -s "/usr/share/man/man1/$name.gz"; done; dpkg-query -L mdrun | grep /usr/share/doc/mdrun/LICENSE; dpkg -r mdrun; test ! -e /usr/bin/mdrun' sh "$name"
            ;;
        *.rpm)
            found_rpm=1
            docker run --rm --platform "$platform" -v "$directory:/packages:ro" -v "$script_directory/../smoke.sh:/package-smoke.sh:ro" -v "$script_directory/../../docs/man:/expected-man:ro" fedora:43 \
                sh -euc 'rpm -i "/packages/$1"; env -i /usr/bin/mdrun --version; sh /package-smoke.sh /usr/bin/mdrun; test -s /usr/share/licenses/mdrun/THIRD_PARTY_NOTICES.txt; test -s /usr/share/licenses/mdrun/LICENSE; for page in /expected-man/*.1; do name=${page##*/}; test -s "/usr/share/man/man1/$name" || test -s "/usr/share/man/man1/$name.gz"; done; rpm -e mdrun; test ! -e /usr/bin/mdrun' sh "$name"
            ;;
    esac
done
[ "$found_deb" = 1 ] && [ "$found_rpm" = 1 ] || { echo "both .deb and .rpm packages are required" >&2; exit 1; }
