"""Generate release attribution from the exact audited Rust toolchain and musl sources."""

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import tarfile


ROOT = Path(__file__).resolve().parents[1]


def digest(data):
    return hashlib.sha256(data).hexdigest()


def check_identity(version, pinned):
    fields = dict(line.split(": ", 1) for line in version.splitlines() if ": " in line)
    if fields.get("release") != pinned["release"] or fields.get("commit-hash") != pinned["commit"]:
        raise ValueError("compiler identity differs from audited release and source commit")


def read_verified(root, row):
    data = (root / row["path"]).read_bytes()
    if not data.strip() or digest(data) != row["sha256"]:
        raise ValueError(f"notice checksum mismatch: {row['path']}")
    return data


def check_musl_link_map(text, library):
    bundled = (library / "self-contained/libc.a").resolve()
    linked = {Path(path).resolve() for path in re.findall(r"(?<!\S)(/[^\s()]+/libc\.a)(?=[(\s]|$)", text)}
    if linked != {bundled}:
        raise ValueError("linked musl must come exclusively from the audited Rust sysroot")


def musl_notices(archive):
    """Retain complete notice comments, including architecture-specific source notices."""
    chunks = []
    with tarfile.open(archive, "r:gz") as source:
        for member in sorted(source.getmembers(), key=lambda m: m.name):
            if not member.isfile():
                continue
            data = source.extractfile(member).read()
            if member.name == "musl-1.2.5/COPYRIGHT":
                notices = [data]
            else:
                notices = [comment for comment in re.findall(rb"/\*.*?\*/", data, re.S)
                           if re.search(rb"copyright|permission|licen[cs]e|public domain", comment, re.I)]
            if notices:
                chunks.append(f"\n--- {member.name} (source SHA-256 {digest(data)}) ---\n".encode())
                chunks.extend(notice + b"\n" for notice in notices)
    if not chunks:
        raise ValueError("musl archive contains no attribution")
    return b"".join(chunks)


def generate(root, rustc, musl_archive=None):
    inventory = json.loads((root / "packaging/toolchain.json").read_text())
    version = subprocess.check_output([rustc, "-Vv"], text=True)
    check_identity(version, inventory["rust"])
    sysroot = Path(subprocess.check_output([rustc, "--print", "sysroot"], text=True).strip())
    if musl_archive is not None:
        if digest(musl_archive.read_bytes()) != inventory["musl"]["archive_sha256"]:
            raise ValueError("musl archive checksum mismatch")
        expected = read_verified(root, inventory["musl"]["notice"])
        if musl_notices(musl_archive) != expected:
            raise ValueError("musl notices differ from original upstream archive")
    header = ("Release toolchain and runtime attribution\n"
              f"Rust {inventory['rust']['release']}, source {inventory['rust']['commit']}\n"
              "The complete upstream HTML documents and license texts below are preserved verbatim.\n"
              "This conservative inventory includes toolchain and other-target components; it does not claim all are linked.\n"
              "Rust's standard library, compiler builtins and LLVM runtime notices apply as incorporated.\n"
              "Linux static releases additionally incorporate musl 1.2.5 with the patches identified in packaging/toolchain.json.\n\n")
    output = bytearray(header.encode())
    for row in inventory["rust_files"]:
        output.extend(f"\n--- Rust distribution: {row['path']} ---\n".encode())
        output.extend(read_verified(sysroot / "share/doc/rust", row))
        output.extend(b"\n")
    for row in inventory["additional_files"] + [inventory["musl"]["notice"]]:
        output.extend(f"\n--- {row['path']} ---\n".encode())
        output.extend(read_verified(root, row))
        output.extend(b"\n")
    return bytes(output)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("generate", "check", "record", "check-link"))
    parser.add_argument("--rustc", default="rustc")
    parser.add_argument("--musl-archive", type=Path)
    parser.add_argument("--target")
    parser.add_argument("--source-revision")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--link-map", type=Path)
    args = parser.parse_args()
    try:
        output = generate(ROOT, args.rustc, args.musl_archive)
        destination = ROOT / "TOOLCHAIN_NOTICES.txt"
        if args.command == "check-link":
            if args.target not in ("x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl") or args.link_map is None:
                raise ValueError("check-link requires a Linux release --target and --link-map")
            library = Path(subprocess.check_output([args.rustc, "--print", "target-libdir", "--target", args.target], text=True).strip())
            check_musl_link_map(args.link_map.read_text(), library)
        elif args.command == "record":
            if args.target not in ("aarch64-apple-darwin", "x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl") or not re.fullmatch(r"[a-f0-9]{40}", args.source_revision or "") or args.output is None:
                raise ValueError("record requires a release --target, full --source-revision and --output")
            if destination.read_bytes() != output:
                raise ValueError("release toolchain attribution is stale")
            library = Path(subprocess.check_output([args.rustc, "--print", "target-libdir", "--target", args.target], text=True).strip())
            libraries = sorted(p for p in library.rglob("*") if p.is_file() and p.suffix in (".rlib", ".a", ".o"))
            if not libraries:
                raise ValueError("target standard-library/runtime artifacts are missing")
            record = {"source_revision": args.source_revision, "target": args.target,
                      "compiler": subprocess.check_output([args.rustc, "-Vv"], text=True),
                      "binary_size_bytes": (ROOT / "target" / args.target / "release/mdrun").stat().st_size,
                      "binary_sha256": digest((ROOT / "target" / args.target / "release/mdrun").read_bytes()),
                      "attribution_sha256": digest(output),
                      "toolchain_inventory_sha256": digest((ROOT / "packaging/toolchain.json").read_bytes()),
                      "runtime_artifacts": [{"path": p.relative_to(library).as_posix(), "sha256": digest(p.read_bytes())} for p in libraries]}
            if args.target.endswith("-linux-musl"):
                record["c_build_tools"] = {
                    "scope": "C/assembly compilation only; the linked musl runtime comes from the audited Rust sysroot",
                    "compiler": subprocess.check_output(["musl-gcc", "--version"], text=True),
                    "packages": subprocess.check_output(["dpkg-query", "-W", "musl-tools", "musl-dev"], text=True),
                }
            args.output.write_text(json.dumps(record, indent=2) + "\n")
        elif args.command == "generate":
            destination.write_bytes(output)
        elif destination.read_bytes() != output:
            raise ValueError("release toolchain attribution is stale")
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError, tarfile.TarError) as error:
        parser.error(str(error))


if __name__ == "__main__":
    main()
