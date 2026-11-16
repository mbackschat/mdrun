"""Prepare the experimental JVM SDK with a Java 21 JDK and a hash-locked dependency."""

import argparse
from datetime import date, datetime, time, timedelta, timezone
import hashlib
from io import BytesIO
import json
from pathlib import Path
import re
import subprocess
import tempfile
from urllib.request import HTTPRedirectHandler, Request, build_opener
import xml.etree.ElementTree as ET
import zipfile


ROOT = Path(__file__).resolve().parents[1]
SDK_ROOT = ROOT / "sdk/jvm"
PACKAGE = "io/github/mbackschat/mdrun/sdk"
MAVEN_NS = {"m": "http://maven.apache.org/POM/4.0.0"}
MAX_DOWNLOAD_BYTES = 16 * 1024 * 1024


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def validate_dependency(sdk_root, now):
    """Check the sole reviewed runtime dependency and its POM before executing Java."""
    lock = json.loads((sdk_root / "dependencies.lock.json").read_text())
    if lock.get("schema_version") != 1 or lock.get("minimum_age_days") != 14 or len(lock.get("dependencies", [])) != 1:
        raise ValueError("unsupported JVM dependency lock or age policy")
    dependency = lock["dependencies"][0]
    if (dependency.get("group_id"), dependency.get("artifact_id")) != ("com.fasterxml.jackson.core", "jackson-core"):
        raise ValueError("unreviewed JVM dependency")
    version = dependency.get("version", "")
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", version):
        raise ValueError("JVM dependency must be an exact stable version")
    if dependency.get("runtime_dependencies") != []:
        raise ValueError("unreviewed transitive JVM dependencies")
    if not re.fullmatch(r"[0-9a-f]{64}", dependency.get("sha256", "")):
        raise ValueError("missing exact dependency SHA-256")
    filename = f"jackson-core-{version}.jar"
    artifact_url = f"https://repo.maven.apache.org/maven2/com/fasterxml/jackson/core/jackson-core/{version}/{filename}"
    if dependency.get("artifact_url") != artifact_url or dependency.get("checksum_url") != artifact_url + ".sha256":
        raise ValueError("dependency URL must identify the exact canonical Maven Central artifact")
    published = date.fromisoformat(dependency["published"])
    # The tagged release notes give a date, not a time; age from its latest possible instant.
    latest_publication = datetime.combine(published + timedelta(days=1), time.min, timezone.utc)
    if now - latest_publication < timedelta(days=14):
        raise ValueError(f"jackson-core@{version}: release has not aged 14 days")
    pom = ET.parse(sdk_root / "pom.xml").getroot()
    entries = pom.findall("m:dependencies/m:dependency", MAVEN_NS)
    expected = [dependency["group_id"], dependency["artifact_id"], version]
    actual = [[entry.findtext("m:" + field, namespaces=MAVEN_NS) for field in ("groupId", "artifactId", "version")]
              for entry in entries]
    if actual != [expected] or pom.findtext("m:properties/m:maven.compiler.release", namespaces=MAVEN_NS) != "21":
        raise ValueError("pom.xml must expose exactly the locked dependency and Java 21 release")
    return dependency


def validate_dependency_jar(data, dependency):
    if sha256(data) != dependency["sha256"]:
        raise ValueError("Jackson core SHA-256 does not match dependencies.lock.json")
    required = {"META-INF/LICENSE", "META-INF/NOTICE", "META-INF/FastDoubleParser-LICENSE",
                "META-INF/FastDoubleParser-ThirdParty-LICENSE", "META-INF/Schubfach-LICENSE"}
    if set(dependency.get("bundled_notices", [])) != required:
        raise ValueError("dependency lock must retain all five bundled notices")
    try:
        with zipfile.ZipFile(BytesIO(data)) as archive:
            if not required.issubset(archive.namelist()):
                raise ValueError("Jackson core is missing a required bundled notice")
    except zipfile.BadZipFile as error:
        raise ValueError("Jackson core is not a valid jar") from error


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, request, response, code, message, headers, new_url):
        return None


def download_dependency(url):
    request = Request(url, headers={"User-Agent": "mdrun JVM preparation (github.com/mbackschat/mdrun)"})
    with build_opener(NoRedirect()).open(request, timeout=30) as response:
        data = response.read(MAX_DOWNLOAD_BYTES + 1)
    if len(data) > MAX_DOWNLOAD_BYTES:
        raise ValueError("Jackson core download exceeds the preparation byte limit")
    return data


def validate_jdk(java_home):
    release = (java_home / "release").read_text()
    if not re.search(r'^JAVA_VERSION="21(?:[.+-][^"\n]*)?"$', release, re.MULTILINE):
        raise ValueError("--java-home must name a Java 21 JDK")
    for name in ("javac", "jar"):
        if not (java_home / "bin" / name).is_file():
            raise ValueError(f"Java 21 JDK is missing bin/{name}")


def prepare(java_home, jackson_core, output, *, now=None, download=False):
    """Build an SDK-only jar and return its relative, deterministic classpath manifest."""
    now = now or datetime.now(timezone.utc)
    dependency = validate_dependency(SDK_ROOT, now)
    if (jackson_core is None) == (not download):
        raise ValueError("provide exactly one of --jackson-core or --download")
    data = download_dependency(dependency["artifact_url"]) if download else Path(jackson_core).read_bytes()
    validate_dependency_jar(data, dependency)
    java_home, output = Path(java_home).resolve(), Path(output).resolve()
    validate_jdk(java_home)
    sources = sorted((SDK_ROOT / "src/main/java" / PACKAGE).rglob("*.java"))
    if not sources:
        raise ValueError("no JVM SDK production sources found")
    if output.exists() and (not output.is_dir() or next(output.iterdir(), None) is not None):
        raise ValueError("--output must name a new or empty directory")
    output.mkdir(parents=True, exist_ok=True)
    dependency_name = f"jackson-core-{dependency['version']}.jar"
    with tempfile.TemporaryDirectory(prefix=".mdrun-jvm-", dir=output) as temporary:
        stage = Path(temporary)
        dependency_path = stage / dependency_name
        dependency_path.write_bytes(data)
        classes = stage / "classes"
        classes.mkdir()
        subprocess.run([str(java_home / "bin/javac"), "--release", "21", "-proc:none", "-encoding", "UTF-8", "-g:none",
                        "-classpath", str(dependency_path), "-d", str(classes), *(str(source) for source in sources)],
                       check=True, capture_output=True, text=True, timeout=60)
        compiled = sorted(classes.rglob("*.class"))
        if not compiled or any(not path.relative_to(classes).as_posix().startswith(PACKAGE + "/") for path in compiled):
            raise ValueError("compiler output must contain only JVM SDK package classes")
        metadata = classes / "META-INF/maven/io.github.mbackschat/mdrun-sdk"
        metadata.mkdir(parents=True)
        (metadata / "pom.xml").write_bytes((SDK_ROOT / "pom.xml").read_bytes())
        (classes / "META-INF/LICENSE").write_bytes((ROOT / "LICENSE").read_bytes())
        sdk_path = stage / "mdrun-sdk.jar"
        command = [str(java_home / "bin/jar"), "--create", "--file", str(sdk_path), "--no-manifest",
                   "--date=1980-01-01T00:00:02Z"]
        for entry in sorted(path for path in classes.rglob("*") if path.is_file()):
            command.extend(["-C", str(classes), entry.relative_to(classes).as_posix()])
        subprocess.run(command, check=True, capture_output=True, text=True, timeout=60)
        sdk = {"path": sdk_path.name, "sha256": sha256(sdk_path.read_bytes())}
        manifest = {"schema_version": 1, "java_release": 21, "sdk": sdk,
                    "classpath": [sdk, {"path": dependency_name, "sha256": dependency["sha256"]}],
                    "dependency": {"coordinates": "com.fasterxml.jackson.core:jackson-core:" + dependency["version"],
                                   "published": dependency["published"],
                                   "canonical_artifact_verified": dependency["canonical_artifact_verified"]}}
        (stage / "build-manifest.json").write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
        for filename in (sdk_path.name, dependency_name, "build-manifest.json"):
            (stage / filename).replace(output / filename)
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--java-home", type=Path, required=True, help="Java 21 JDK root")
    dependency = parser.add_mutually_exclusive_group(required=True)
    dependency.add_argument("--jackson-core", type=Path, help="prepared Jackson core jar matching the lock")
    dependency.add_argument("--download", action="store_true", help="fetch the exact locked artifact from Maven Central")
    parser.add_argument("--output", type=Path, required=True, help="new or empty preparation directory")
    args = parser.parse_args()
    try:
        manifest = prepare(args.java_home, args.jackson_core, args.output, download=args.download)
    except (OSError, ValueError, KeyError, ET.ParseError, subprocess.SubprocessError) as error:
        detail = error.stderr if isinstance(error, subprocess.CalledProcessError) else str(error)
        parser.exit(1, f"JVM preparation failed: {detail}\n")
    print(json.dumps(manifest, sort_keys=True))


if __name__ == "__main__":
    main()
