"""Build deterministic, versioned runtime SDK archives for GitHub Releases."""

import argparse
import base64
import csv
from datetime import datetime, timezone
import gzip
import hashlib
import importlib.util
from io import BytesIO, StringIO
import json
from pathlib import Path
import re
import tarfile
import tomllib
from urllib.parse import urljoin
import xml.etree.ElementTree as ET
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


consumer = load("mdrun_sdk_consumer", ROOT / "scripts/prepare_sdk.py")
jvm_builder = load("mdrun_jvm_preparation", ROOT / "scripts/prepare_jvm.py")


def regular_file(path):
    if not path.is_file() or path.is_symlink():
        raise ValueError(f"SDK input must be a regular file: {path}")
    return path.read_bytes()


def version(root):
    value = consumer.validate_version(tomllib.loads((root / "Cargo.toml").read_text())["package"]["version"])
    pom = ET.fromstring(regular_file(root / "sdk/jvm/pom.xml"))
    if pom.findtext("m:version", namespaces=jvm_builder.MAVEN_NS) != value:
        raise ValueError("host and JVM SDK version must match")
    return value


def jvm_files(root, prepared):
    dependency = jvm_builder.validate_dependency(root / "sdk/jvm", datetime.now(timezone.utc))
    dependency_name = f"jackson-core-{dependency['version']}.jar"
    if prepared is None:
        raise ValueError("--jvm-prepared is required for the JVM SDK")
    files = {name: regular_file(prepared / name) for name in ("mdrun-sdk.jar", dependency_name, "build-manifest.json")}
    files.update({name: regular_file(root / "sdk/jvm" / name) for name in ("pom.xml", "dependencies.lock.json")})
    jvm_builder.validate_dependency_jar(files[dependency_name], dependency)
    try:
        build = consumer.json_object(files["build-manifest.json"])
        sdk_entry = {"path": "mdrun-sdk.jar", "sha256": consumer.sha256(files["mdrun-sdk.jar"])}
        expected = {"schema_version": 1, "java_release": 21, "sdk": sdk_entry,
                    "classpath": [sdk_entry, {"path": dependency_name, "sha256": dependency["sha256"]}],
                    "dependency": {"coordinates": "com.fasterxml.jackson.core:jackson-core:" + dependency["version"],
                                   "published": dependency["published"],
                                   "canonical_artifact_verified": dependency["canonical_artifact_verified"]}}
        if build != expected:
            raise ValueError("JVM preparation manifest does not match the locked artifacts")
        with zipfile.ZipFile(BytesIO(files["mdrun-sdk.jar"])) as jar:
            names = jar.namelist()
            metadata = {"META-INF/LICENSE", "META-INF/maven/io.github.mbackschat/mdrun-sdk/pom.xml"}
            classes = {name for name in names if name.startswith(jvm_builder.PACKAGE + "/") and name.endswith(".class")}
            if not classes or len(set(names)) != len(names) or set(names) != classes | metadata:
                raise ValueError("prepared SDK jar contains files outside the SDK package")
            if jar.read("META-INF/LICENSE") != regular_file(root / "LICENSE") or jar.read(
                    "META-INF/maven/io.github.mbackschat/mdrun-sdk/pom.xml") != files["pom.xml"]:
                raise ValueError("prepared SDK jar license or POM mismatch")
        with zipfile.ZipFile(BytesIO(files[dependency_name])) as jar:
            for name in dependency["bundled_notices"]:
                files["licenses/jackson-core/" + Path(name).name] = jar.read(name)
    except (zipfile.BadZipFile, KeyError) as error:
        raise ValueError("invalid JVM preparation manifest or SDK jar") from error
    return files


def archive_readme(root, runtime, release):
    source = regular_file(root / "sdk" / runtime / "README.md").decode("utf-8")
    base = f"https://github.com/mbackschat/mdrun/blob/v{release}/sdk/{runtime}/README.md"
    # An extracted SDK has no repository siblings; retain the links against its immutable source tag.
    return re.sub(r"(\[[^\]]*\]\()([^)]+)(\))",
                  lambda match: match[1] + urljoin(base, match[2]) + match[3], source).encode("utf-8")


def archive(root, runtime, output, *, jvm_prepared=None):
    root, output = Path(root), Path(output)
    release = version(root)
    filename = consumer.archive_name(runtime, release)
    files = {"README.md": archive_readme(root, runtime, release),
             "LICENSE": regular_file(root / "LICENSE"), "VERSION": (release + "\n").encode()}
    if runtime == "jvm":
        files.update(jvm_files(root, Path(jvm_prepared) if jvm_prepared is not None else None))
    else:
        name = "index.mjs" if runtime == "node" else "mdrun_sdk/__init__.py"
        files[name] = regular_file(root / "sdk" / runtime / name)
    manifest = {"schema_version": 1, "runtime": runtime, "version": release,
                "files": {name: consumer.sha256(data) for name, data in sorted(files.items())}}
    files["manifest.json"] = (json.dumps(manifest, indent=2, sort_keys=True) + "\n").encode()
    payload = BytesIO()
    with gzip.GzipFile(fileobj=payload, mode="wb", filename="", mtime=0, compresslevel=9) as compressed:
        with tarfile.open(fileobj=compressed, mode="w", format=tarfile.USTAR_FORMAT) as result:
            for name, data in sorted(files.items()):
                entry = tarfile.TarInfo(filename.removesuffix(".tar.gz") + "/" + name)
                entry.mode, entry.size, entry.mtime = 0o644, len(data), 0
                result.addfile(entry, BytesIO(data))
    data = payload.getvalue()
    consumer.validate_archive(data, runtime, release)
    output.mkdir(parents=True, exist_ok=True)
    destination = output / filename
    with destination.open("xb") as result:
        result.write(data)
    return destination


def wheel(root, output):
    """Build the dependency-free purelib wheel defined by the PyPA wheel specification."""
    root, output = Path(root), Path(output)
    release = version(root)
    info = f"mdrun_sdk-{release}.dist-info/"
    metadata = ("Metadata-Version: 2.4\nName: mdrun-sdk\n"
                f"Version: {release}\n"
                "Summary: Cooperative Python worker SDK for experimental mdrun sessions\n"
                "Requires-Python: >=3.12\nLicense-Expression: MIT\nLicense-File: LICENSE\n"
                "Project-URL: Source, https://github.com/mbackschat/mdrun\n"
                "Description-Content-Type: text/markdown\n\n").encode() + archive_readme(root, "python", release)
    files = {"mdrun_sdk/__init__.py": regular_file(root / "sdk/python/mdrun_sdk/__init__.py"),
             info + "METADATA": metadata,
             info + "WHEEL": b"Wheel-Version: 1.0\nGenerator: mdrun-sdk-builder\nRoot-Is-Purelib: true\nTag: py3-none-any\n",
             info + "licenses/LICENSE": regular_file(root / "LICENSE")}
    record = StringIO(newline="")
    rows = csv.writer(record, lineterminator="\n")
    for name, data in sorted(files.items()):
        digest = base64.urlsafe_b64encode(hashlib.sha256(data).digest()).rstrip(b"=").decode()
        rows.writerow((name, "sha256=" + digest, str(len(data))))
    rows.writerow((info + "RECORD", "", ""))
    files[info + "RECORD"] = record.getvalue().encode()
    payload = BytesIO()
    with zipfile.ZipFile(payload, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as result:
        for name in sorted(files, key=lambda name: (name.startswith(info), name == info + "RECORD", name)):
            entry = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
            entry.create_system, entry.external_attr, entry.compress_type = 3, 0o100644 << 16, zipfile.ZIP_DEFLATED
            result.writestr(entry, files[name])
    output.mkdir(parents=True, exist_ok=True)
    destination = output / f"mdrun_sdk-{release}-py3-none-any.whl"
    with destination.open("xb") as result:
        result.write(payload.getvalue())
    return destination


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    package = commands.add_parser("archive")
    package.add_argument("runtime", choices=consumer.RUNTIMES)
    package.add_argument("output", type=Path)
    package.add_argument("--jvm-prepared", type=Path)
    python_wheel = commands.add_parser("wheel", help="build the standard pure-Python SDK wheel")
    python_wheel.add_argument("output", type=Path)
    args = parser.parse_args()
    try:
        print(wheel(ROOT, args.output) if args.command == "wheel" else
              archive(ROOT, args.runtime, args.output, jvm_prepared=args.jvm_prepared))
    except (OSError, ValueError, ET.ParseError) as error:
        parser.exit(1, f"SDK packaging failed: {error}\n")


if __name__ == "__main__":
    main()
