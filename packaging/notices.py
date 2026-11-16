"""Preserve locked registry-crate notices without installing a license tool.

Generate from full `cargo metadata --locked --offline --format-version 1` output.
The cached .crate archives must match Cargo.lock before any notice is read.
`check` needs only the checked inventory, bundle and lock; --metadata additionally
reproduces them from the cached upstream archives. This does not audit Rust's
standard library, compiler runtimes, musl, or separately distributed SDKs.
Source headers use explicitly reviewed, version/archive/file/range-pinned recipes;
this helper does not infer license obligations from arbitrary source comments.
"""

import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import tarfile
import tomllib


BUNDLE = "THIRD_PARTY_NOTICES.txt"
INVENTORY = "packaging/notices.json"
EXTRACTS = Path(__file__).with_name("notice-extracts.json")
REGISTRY = "registry+https://github.com/rust-lang/crates.io-index"
NOTICE_NAME = re.compile(r"^(license|licence|copying|copyright|notice)(?:$|[._-])", re.I)
# r-efi 6.0.0 README's License section points to AUTHORS; it contains the full
# MIT grant and copyright notices, rather than a conventionally named license.
EXTRA_FILES = {("r-efi", "6.0.0"): ("AUTHORS",)}
CHOICES = {("r-efi", "6.0.0"): "MIT"}
HEADER = b"""mdrun: third-party registry-crate notices

This bundle conservatively includes every registry package in Cargo.lock,
including build-time and other-target packages. It is not a linked-code list.
Upstream license alternatives are preserved; OR alternatives are not cumulative.
An explicit redistribution choice is identified where needed.
Only upstream notice texts are reproduced, without their implementation source.
Reviewed in-source notices retain their exact ranges and source identities.
Rust toolchain, standard-library/runtime and musl attribution are a separate
release audit. Separately distributed SDK dependencies are not covered here.

"""


def digest(data):
    return hashlib.sha256(data).hexdigest()


def identity(package):
    return package["name"], package["version"], package["source"]


def locked_packages(lock_path):
    packages = {}
    for package in tomllib.loads(lock_path.read_text())["package"]:
        if "source" not in package:
            continue
        if package["source"] != REGISTRY:
            raise ValueError(f"unreviewed dependency source: {package['source']}")
        key = identity(package)
        if key in packages or not re.fullmatch(r"[a-f0-9]{64}", package.get("checksum", "")):
            raise ValueError(f"invalid locked package identity/checksum: {key}")
        packages[key] = package
    return packages


def source_header_recipes(locked):
    data = json.loads(EXTRACTS.read_text())
    if type(data) is not dict or set(data) != {"version", "packages"} or type(data["version"]) is not int or data["version"] != 1:
        raise ValueError("unsupported source-header recipe format")
    if type(data["packages"]) is not list:
        raise ValueError("invalid source-header recipe packages")
    recipes = {}
    for package in data["packages"]:
        required = {"name", "version", "source", "checksum", "headers"}
        if type(package) is not dict or set(package) not in (required, required | {"limitations"}):
            raise ValueError("invalid source-header package recipe")
        if any(type(package[name]) is not str or not package[name] for name in ("name", "version", "source")):
            raise ValueError("invalid source-header package identity")
        key = identity(package)
        if key in recipes or package["source"] != REGISTRY or type(package["checksum"]) is not str or not re.fullmatch(r"[a-f0-9]{64}", package["checksum"]):
            raise ValueError("invalid source-header package identity/checksum")
        headers = package["headers"]
        if type(headers) is not list or not headers:
            raise ValueError("empty source-header recipe")
        ranges = set()
        for header in headers:
            if type(header) is not dict or set(header) != {"path", "source_sha256", "first_line", "last_line", "sha256"}:
                raise ValueError("invalid source-header range recipe")
            path = header["path"]
            if type(path) is not str or not path or PurePosixPath(path).is_absolute() or ".." in PurePosixPath(path).parts or PurePosixPath(path).as_posix() != path:
                raise ValueError("invalid source-header path")
            if any(type(header[name]) is not int for name in ("first_line", "last_line")) or not 1 <= header["first_line"] <= header["last_line"]:
                raise ValueError("invalid source-header line range")
            if any(type(header[name]) is not str or not re.fullmatch(r"[a-f0-9]{64}", header[name]) for name in ("source_sha256", "sha256")):
                raise ValueError("invalid source-header checksum")
            span = (path, header["first_line"], header["last_line"])
            if span in ranges:
                raise ValueError("duplicate source-header range")
            ranges.add(span)
        limits = package.get("limitations", [])
        if type(limits) is not list or any(type(limit) is not str or not limit.strip() for limit in limits):
            raise ValueError("invalid source-header limitation")
        recipes[key] = package
    names = {key[0] for key in recipes}
    result = {}
    for key, package in locked.items():
        if key[0] in names:
            if key not in recipes or recipes[key]["checksum"] != package["checksum"]:
                raise ValueError(f"source-header recipe needs review for locked package: {key}")
            result[key] = {"headers": sorted(recipes[key]["headers"], key=lambda h: (h["path"], h["first_line"])),
                           "limitations": recipes[key].get("limitations", [])}
    return result


def source_header(tar, prefix, recipe):
    member = tar.getmember(prefix + "/" + recipe["path"])
    if not member.isfile():
        raise ValueError(f"source-header file is not regular: {member.name}")
    data = tar.extractfile(member).read()
    if digest(data) != recipe["source_sha256"]:
        raise ValueError(f"source-header file checksum mismatch: {member.name}")
    lines = data.splitlines(keepends=True)
    if recipe["last_line"] > len(lines):
        raise ValueError(f"source-header range exceeds file: {member.name}")
    selected = b"".join(lines[recipe["first_line"] - 1:recipe["last_line"]])
    if not selected.strip() or digest(selected) != recipe["sha256"]:
        raise ValueError(f"source-header range checksum mismatch: {member.name}")
    selected.decode("utf-8")
    return selected


def collect(lock_path, metadata_path):
    locked = locked_packages(lock_path)
    metadata = json.loads(metadata_path.read_text())
    packages = [p for p in metadata["packages"] if p.get("source")]
    if len(packages) != len(locked) or {identity(p) for p in packages} != set(locked):
        raise ValueError("metadata package identities do not match Cargo.lock")
    recipes = source_header_recipes(locked)
    rows = []
    output = bytearray(HEADER)
    for package in sorted(packages, key=identity):
        key = identity(package)
        pinned = locked[key]
        name, version, source = key
        recipe = recipes.get(key, {"headers": [], "limitations": []})
        root = Path(package["manifest_path"]).parent
        archive = root.parent.parent.parent / "cache" / root.parent.name / f"{name}-{version}.crate"
        if digest(archive.read_bytes()) != pinned["checksum"]:
            raise ValueError(f"crate archive checksum mismatch: {name} {version}")
        files = []
        declared = package.get("license_file")
        if declared:
            declared = Path(declared)
            declared = declared.relative_to(root).as_posix() if declared.is_absolute() else declared.as_posix()
        required = set(EXTRA_FILES.get((name, version), ()))
        if declared:
            required.add(declared)
        license_expression = package.get("license")
        if not license_expression and not declared:
            raise ValueError(f"missing upstream license declaration: {name} {version}")
        output.extend(f"{'=' * 72}\n{name} {version}\nSource: {source}\nCrate SHA-256: {pinned['checksum']}\nLicense declaration: {license_expression or declared}\n".encode())
        choice = CHOICES.get((name, version))
        if choice:
            output.extend(f"Redistribution choice: {choice}\n".encode())
        for limit in recipe["limitations"]:
            output.extend(f"Notice scope limit: {limit}\n".encode())
        with tarfile.open(archive, "r:gz") as tar:
            members = tar.getmembers()
            if len({m.name for m in members}) != len(members):
                raise ValueError(f"duplicate crate archive paths: {name} {version}")
            manifest = tomllib.loads(tar.extractfile(f"{name}-{version}/Cargo.toml").read().decode("utf-8"))["package"]
            if (manifest["name"], manifest["version"], manifest.get("license"), manifest.get("license-file")) != (name, version, license_expression, declared):
                raise ValueError(f"metadata license declaration differs from locked archive: {name} {version}")
            for member in sorted(members, key=lambda m: m.name):
                parts = PurePosixPath(member.name).parts
                if not parts or parts[0] != f"{name}-{version}" or ".." in parts:
                    raise ValueError(f"invalid crate archive path: {member.name}")
                relative = PurePosixPath(*parts[1:]).as_posix()
                if not (NOTICE_NAME.match(PurePosixPath(relative).name) or relative in required):
                    continue
                if not member.isfile():
                    raise ValueError(f"notice is not a regular file: {member.name}")
                data = tar.extractfile(member).read()
                if not data.strip():
                    raise ValueError(f"empty notice: {member.name}")
                data.decode("utf-8")
                files.append({"path": relative, "sha256": digest(data)})
                output.extend(f"\n--- {relative} ---\n".encode())
                output.extend(data)
                output.extend(b"\n\n")
            for header in recipe["headers"]:
                data = source_header(tar, f"{name}-{version}", header)
                output.extend(f"\n--- {header['path']} (source notice, lines {header['first_line']}-{header['last_line']}) ---\nSource file SHA-256: {header['source_sha256']}\nNotice SHA-256: {header['sha256']}\n\n".encode())
                output.extend(data)
                output.extend(b"\n\n")
        if not files or required - {f["path"] for f in files}:
            raise ValueError(f"missing upstream notice files: {name} {version}")
        row = {"name": name, "version": version, "source": source,
               "checksum": pinned["checksum"], "license": license_expression,
               "files": files}
        if choice:
            row["license_choice"] = choice
        if recipe["headers"]:
            row["source_headers"] = recipe["headers"]
        if recipe["limitations"]:
            row["notice_limits"] = recipe["limitations"]
        rows.append(row)
    output.extend(b"End of registry-crate notices.\n")
    bundle = bytes(output)
    return {"version": 1, "bundle_sha256": digest(bundle), "packages": rows}, bundle


def check(root, metadata_path=None):
    inventory = json.loads((root / INVENTORY).read_text())
    bundle = (root / BUNDLE).read_bytes()
    if inventory.get("version") != 1:
        raise ValueError("unsupported notice inventory version")
    locked = locked_packages(root / "Cargo.lock")
    rows = inventory["packages"]
    if len(rows) != len(locked) or {identity(p) for p in rows} != set(locked):
        raise ValueError("notice inventory identities do not match Cargo.lock")
    recipes = source_header_recipes(locked)
    for row in rows:
        if row["checksum"] != locked[identity(row)]["checksum"] or not row["files"]:
            raise ValueError(f"stale or empty notice inventory: {identity(row)}")
        recipe = recipes.get(identity(row), {"headers": [], "limitations": []})
        if row.get("source_headers", []) != recipe["headers"] or row.get("notice_limits", []) != recipe["limitations"]:
            raise ValueError(f"stale source-header inventory: {identity(row)}")
    if digest(bundle) != inventory["bundle_sha256"]:
        raise ValueError("notice bundle checksum mismatch")
    if metadata_path is not None:
        expected, text = collect(root / "Cargo.lock", metadata_path)
        if inventory != expected or bundle != text:
            raise ValueError("checked notices differ from locked upstream archives")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("generate", "check"))
    parser.add_argument("--metadata", type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    try:
        if args.command == "generate":
            if args.metadata is None:
                parser.error("generate requires --metadata")
            inventory, bundle = collect(root / "Cargo.lock", args.metadata)
            (root / BUNDLE).write_bytes(bundle)
            (root / INVENTORY).write_text(json.dumps(inventory, indent=2) + "\n")
        else:
            check(root, args.metadata)
    except (OSError, ValueError, KeyError, tarfile.TarError) as error:
        parser.error(str(error))


if __name__ == "__main__":
    main()
