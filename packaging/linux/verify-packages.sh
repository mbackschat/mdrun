#!/bin/sh
set -eu
directory=${1:?usage: verify-packages.sh ABSOLUTE_DIRECTORY}
case "$directory" in /*) ;; *) echo "use an absolute package directory" >&2; exit 1 ;; esac

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
            docker run --rm --platform "$platform" -v "$directory:/packages:ro" debian:13-slim \
                sh -euc 'dpkg -i "/packages/$1"; env -i /usr/bin/mdrun --version; env -i /usr/bin/mdrun --help; dpkg-query -L mdrun | grep /usr/share/doc/mdrun/LICENSE; dpkg -r mdrun; test ! -e /usr/bin/mdrun' sh "$name"
            ;;
        *.rpm)
            found_rpm=1
            docker run --rm --platform "$platform" -v "$directory:/packages:ro" fedora:43 \
                sh -euc 'rpm -i "/packages/$1"; env -i /usr/bin/mdrun --version; env -i /usr/bin/mdrun --help; test -s /usr/share/licenses/mdrun/LICENSE; rpm -e mdrun; test ! -e /usr/bin/mdrun' sh "$name"
            ;;
    esac
done
[ "$found_deb" = 1 ] && [ "$found_rpm" = 1 ] || { echo "both .deb and .rpm packages are required" >&2; exit 1; }
