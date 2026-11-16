"""Build native release archives and a Homebrew formula from checked digests."""

import argparse
from pathlib import Path
import re
import tarfile
import tomllib

TARGETS = (
    "aarch64-apple-darwin",
    "x86_64-unknown-linux-musl",
    "aarch64-unknown-linux-musl",
)


def validate_version(version):
    if not re.fullmatch(r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)", version):
        raise ValueError("expected a stable version such as 0.1.0")
    return version


def archive(root, version, target, output):
    validate_version(version)
    if target not in TARGETS:
        raise ValueError(f"unsupported release target: {target}")
    files = {
        "mdrun": root / "target" / target / "release" / "mdrun",
        **{name: root / name for name in ("README.md", "CHANGELOG.md", "LICENSE", "THIRD_PARTY_NOTICES.txt")},
    }
    for source in files.values():
        if not source.is_file():
            raise ValueError(f"missing release input: {source}")
    man = root / "docs/man"
    if not (man / "mdrun.1").is_file():
        raise ValueError(f"missing release input: {man / 'mdrun.1'}")
    for page in sorted(man.glob("*.1")):
        if not page.is_file():
            raise ValueError(f"invalid manpage input: {page}")
        files[f"share/man/man1/{page.name}"] = page
    output.mkdir(parents=True, exist_ok=True)
    name = f"mdrun-v{version}-{target}"
    destination = output / f"{name}.tar.gz"
    with tarfile.open(destination, "x:gz") as result:
        for filename, source in files.items():
            result.add(source, arcname=f"{name}/{filename}")
    return destination


def formula(version, checksums):
    validate_version(version)
    values = {}
    for target in TARGETS:
        filename = f"mdrun-v{version}-{target}.tar.gz"
        matches = [line.split()[0] for line in checksums.splitlines()
                   if len(line.split()) == 2 and line.split()[1] == filename]
        if len(matches) != 1 or not re.fullmatch(r"[a-fA-F0-9]{64}", matches[0]):
            raise ValueError(f"expected exactly one SHA-256 digest for {filename}")
        values[target.replace("-", "_")] = matches[0].lower()
    template = Path(__file__).with_name("homebrew") / "mdrun.rb"
    text = template.read_text().replace("{{version}}", version)
    for target, digest in values.items():
        text = text.replace("{{sha256_" + target + "}}", digest)
    if "{{" in text:
        raise ValueError("unresolved Homebrew template placeholder")
    return text


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("version")
    package = commands.add_parser("archive")
    package.add_argument("target", choices=TARGETS)
    package.add_argument("output", type=Path)
    render = commands.add_parser("formula")
    render.add_argument("version")
    render.add_argument("checksums", type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    version = tomllib.loads((root / "Cargo.toml").read_text())["package"]["version"]
    try:
        if args.command == "version":
            print(validate_version(version))
        elif args.command == "archive":
            print(archive(root, version, args.target, args.output))
        else:
            print(formula(args.version, args.checksums.read_text()), end="")
    except (OSError, ValueError) as error:
        parser.error(str(error))


if __name__ == "__main__":
    main()
