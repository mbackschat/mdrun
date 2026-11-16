"""Check all locked Rust dependencies before executing their build code."""

import argparse
from concurrent.futures import ThreadPoolExecutor
from datetime import date, datetime, timedelta, timezone
import json
from pathlib import Path
import tomllib
from urllib.request import Request, urlopen

REGISTRY = "registry+https://github.com/rust-lang/crates.io-index"


def check_release(name, release, now, exceptions):
    version = release["num"]
    prefix = f"{name}@{version}: "
    if release["yanked"]:
        return [prefix + "yanked release"]
    if "-" in version.split("+", 1)[0]:
        return [prefix + "prerelease"]
    published = datetime.fromisoformat(release["created_at"].replace("Z", "+00:00"))
    if now - published >= timedelta(days=14):
        return []
    for exception in exceptions:
        if exception.get("name") != name or exception.get("version") != version:
            continue
        required = ["advisory", "rationale", "reviewed_change", "tests", "expires"]
        if not all(isinstance(exception.get(key), str) and exception[key].strip() for key in required):
            continue
        if not all(exception[key].startswith("https://") for key in ["advisory", "reviewed_change"]):
            continue
        if date.fromisoformat(exception["expires"]) >= now.date():
            return []
    return [prefix + f"published {published.isoformat()}, younger than 14 days"]


def check_packages(packages, metadata, now, exceptions):
    problems = []
    for package in packages:
        if "source" not in package:
            continue
        if package["source"] != REGISTRY:
            raise ValueError(f"unreviewed dependency source: {package['name']} {package['source']}")
        name, version = package["name"], package["version"]
        if name not in metadata:
            raise ValueError(f"missing registry metadata: {name}")
        release = next((entry for entry in metadata[name]["versions"] if entry["num"] == version), None)
        if release is None:
            raise ValueError(f"missing registry version: {name}@{version}")
        problems.extend(check_release(name, release, now, exceptions))
    return problems


def fetch_metadata(name):
    request = Request("https://crates.io/api/v1/crates/" + name,
                      headers={"User-Agent": "mdrun dependency policy (github.com/mbackschat/mdrun)"})
    with urlopen(request, timeout=30) as response:
        return name, json.load(response)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lockfile", type=Path, default=Path("Cargo.lock"))
    parser.add_argument("--exceptions", type=Path, default=Path(".github/dependency-exceptions.json"))
    args = parser.parse_args()
    packages = tomllib.loads(args.lockfile.read_text())["package"]
    exceptions = json.loads(args.exceptions.read_text())["exceptions"]
    names = sorted({package["name"] for package in packages if "source" in package})
    with ThreadPoolExecutor(max_workers=3) as pool:
        metadata = dict(pool.map(fetch_metadata, names))
    now = datetime.now(timezone.utc)
    problems = check_packages(packages, metadata, now, exceptions)
    for problem in problems:
        print(problem)
    print(f"Checked {len(names)} registry packages at {now.isoformat()}: {len(problems)} policy violations")
    return bool(problems)


if __name__ == "__main__":
    raise SystemExit(main())
