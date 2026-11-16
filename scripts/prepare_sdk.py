"""Verify and prepare a version- and SHA-256-pinned mdrun GitHub SDK archive."""

import argparse
import gzip
import hashlib
from io import BytesIO
import json
import os
from pathlib import Path, PurePosixPath
import re
import tarfile
import tempfile
from urllib.request import Request, urlopen

RUNTIMES = ("jvm", "node", "python")
MAX_ARCHIVE_BYTES = 32 * 1024 * 1024
MAX_UNPACKED_BYTES = 64 * 1024 * 1024
MAX_FILE_BYTES = 16 * 1024 * 1024
DIGEST = re.compile(r"[0-9a-f]{64}")


def validate_version(version):
    if not isinstance(version, str) or not re.fullmatch(r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)", version):
        raise ValueError("expected an exact stable version such as 0.1.0")
    return version


def archive_name(runtime, version):
    validate_version(version)
    if runtime not in RUNTIMES:
        raise ValueError("unsupported SDK runtime")
    return f"mdrun-sdk-{runtime}-v{version}.tar.gz"


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate SDK manifest field")
        result[key] = value
    return result


def json_object(data):
    result = json.loads(data, object_pairs_hook=unique_object)
    if not isinstance(result, dict):
        raise ValueError("expected an SDK manifest object")
    return result


def expected_files(runtime, contents):
    common = {"README.md", "LICENSE", "VERSION", "manifest.json"}
    if runtime == "node":
        return common | {"index.mjs"}
    if runtime == "python":
        return common | {"mdrun_sdk/__init__.py"}
    try:
        lock = json_object(contents["dependencies.lock.json"])
        dependency, = lock["dependencies"]
        if (lock["schema_version"], lock["minimum_age_days"], dependency["group_id"], dependency["artifact_id"],
                dependency["runtime_dependencies"]) != (1, 14, "com.fasterxml.jackson.core", "jackson-core", []):
            raise ValueError("unsupported JVM dependency lock")
        version = validate_version(dependency["version"])
        notices = {"META-INF/LICENSE", "META-INF/NOTICE", "META-INF/FastDoubleParser-LICENSE",
                   "META-INF/FastDoubleParser-ThirdParty-LICENSE", "META-INF/Schubfach-LICENSE"}
        if set(dependency["bundled_notices"]) != notices:
            raise ValueError("incomplete JVM dependency notices")
        jar = f"jackson-core-{version}.jar"
        if not DIGEST.fullmatch(dependency["sha256"]) or sha256(contents[jar]) != dependency["sha256"]:
            raise ValueError("JVM dependency SHA-256 mismatch")
        build = json_object(contents["build-manifest.json"])
        sdk_entry = {"path": "mdrun-sdk.jar", "sha256": sha256(contents["mdrun-sdk.jar"])}
        if (build["schema_version"], build["java_release"], build["sdk"], build["classpath"]) != (
                1, 21, sdk_entry, [sdk_entry, {"path": jar, "sha256": dependency["sha256"]}]):
            raise ValueError("JVM build manifest mismatch")
        return common | {"mdrun-sdk.jar", jar, "pom.xml", "dependencies.lock.json", "build-manifest.json"} | {
            "licenses/jackson-core/" + PurePosixPath(name).name for name in notices}
    except (KeyError, TypeError) as error:
        raise ValueError("invalid JVM SDK archive metadata") from error


def validate_archive(data, runtime, version):
    """Return verified ordinary file bytes without trusting tar extraction semantics."""
    prefix = archive_name(runtime, version).removesuffix(".tar.gz") + "/"
    if len(data) > MAX_ARCHIVE_BYTES:
        raise ValueError("SDK archive exceeds the download byte limit")
    try:
        with gzip.GzipFile(fileobj=BytesIO(data), mode="rb") as compressed:
            unpacked = compressed.read(MAX_UNPACKED_BYTES + 1)
        if len(unpacked) > MAX_UNPACKED_BYTES:
            raise ValueError("SDK archive exceeds the unpacked byte limit")
        contents = {}
        with tarfile.open(fileobj=BytesIO(unpacked), mode="r:") as archive:
            for member in archive:
                if len(contents) >= 32 or not member.isfile() or member.pax_headers or member.linkname:
                    raise ValueError("SDK archive must contain only bounded ordinary files")
                if not member.name.startswith(prefix):
                    raise ValueError("SDK archive version or root mismatch")
                name = member.name[len(prefix):]
                path = PurePosixPath(name)
                if not name or path.is_absolute() or name != path.as_posix() or ".." in path.parts or "\\" in name:
                    raise ValueError("unsafe SDK archive path")
                if name in contents or member.size > MAX_FILE_BYTES or member.mode != 0o644:
                    raise ValueError("duplicate, oversized or noncanonical SDK archive file")
                contents[name] = archive.extractfile(member).read()
        manifest = json_object(contents["manifest.json"])
        if set(manifest) != {"schema_version", "runtime", "version", "files"} or (
                manifest["schema_version"], manifest["runtime"], manifest["version"]) != (1, runtime, version):
            raise ValueError("SDK hash manifest identity mismatch")
        files = manifest["files"]
        if not isinstance(files, dict) or set(files) != set(contents) - {"manifest.json"}:
            raise ValueError("SDK hash manifest file inventory mismatch")
        for name, digest in files.items():
            if not isinstance(digest, str) or not DIGEST.fullmatch(digest) or sha256(contents[name]) != digest:
                raise ValueError("SDK file SHA-256 mismatch")
        if contents["VERSION"] != (version + "\n").encode() or set(contents) != expected_files(runtime, contents):
            raise ValueError("SDK archive contains an unexpected runtime file inventory or version")
        return contents
    except (OSError, EOFError, tarfile.TarError, KeyError, UnicodeError, json.JSONDecodeError) as error:
        raise ValueError("invalid SDK archive or manifest") from error


def download_archive(runtime, version):
    filename = archive_name(runtime, version)
    url = f"https://github.com/mbackschat/mdrun/releases/download/v{version}/{filename}"
    request = Request(url, headers={"User-Agent": "mdrun SDK preparation (github.com/mbackschat/mdrun)"})
    with urlopen(request, timeout=30) as response:
        if not response.url.startswith("https://"):
            raise ValueError("SDK download redirected outside HTTPS")
        data = response.read(MAX_ARCHIVE_BYTES + 1)
    if len(data) > MAX_ARCHIVE_BYTES:
        raise ValueError("SDK archive exceeds the download byte limit")
    return data


def prepare(runtime, version, digest, output, *, archive=None, download=False):
    """Prepare verified SDK files into one new directory without a source checkout."""
    archive_name(runtime, version)
    if not isinstance(digest, str) or not DIGEST.fullmatch(digest):
        raise ValueError("--sha256 must be an exact lowercase SHA-256 digest")
    if (archive is None) == (not download):
        raise ValueError("provide exactly one of --archive or --download")
    if download:
        data = download_archive(runtime, version)
    else:
        with Path(archive).open("rb") as source:
            data = source.read(MAX_ARCHIVE_BYTES + 1)
    if sha256(data) != digest:
        raise ValueError("SDK archive SHA-256 does not match the pinned digest")
    contents = validate_archive(data, runtime, version)
    output = Path(output).absolute()
    if output.exists() or output.is_symlink():
        raise ValueError("--output must not exist")
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".mdrun-sdk-", dir=output.parent) as temporary:
        stage = Path(temporary) / "sdk"
        stage.mkdir()
        for name, payload in contents.items():
            path = stage / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(payload)
            path.chmod(0o644)
        # Reserve the absent destination; rename never replaces an existing consumer directory.
        output.mkdir()
        try:
            os.replace(stage, output)
        except OSError:
            output.rmdir()
            raise
    return {"runtime": runtime, "version": version, "sha256": digest, "output": str(output)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runtime", choices=RUNTIMES, required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--sha256", required=True)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--archive", type=Path, help="previously downloaded SDK archive")
    source.add_argument("--download", action="store_true", help="download the exact version from GitHub Releases")
    parser.add_argument("--output", type=Path, required=True, help="new preparation directory")
    args = parser.parse_args()
    try:
        result = prepare(args.runtime, args.version, args.sha256, args.output, archive=args.archive, download=args.download)
    except (OSError, ValueError) as error:
        parser.exit(1, f"SDK preparation failed: {error}\n")
    print(json.dumps(result, sort_keys=True))


if __name__ == "__main__":
    main()
