import importlib.util
from pathlib import Path
import tarfile
import tempfile
import tomllib
import unittest


spec = importlib.util.spec_from_file_location(
    "distribution", Path(__file__).with_name("distribution.py")
)
distribution = importlib.util.module_from_spec(spec)
spec.loader.exec_module(distribution)
ROOT = Path(__file__).resolve().parents[1]


class DistributionTests(unittest.TestCase):
    def test_archive_contains_the_exact_binary_and_required_documents(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = distribution.TARGETS[0]
            binary = root / "target" / target / "release" / "mdrun"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"independent-binary-fixture\x00\xff")
            binary.chmod(0o755)
            for name in ("README.md", "CHANGELOG.md", "LICENSE", "THIRD_PARTY_NOTICES.txt"):
                (root / name).write_text(name)
            man = root / "docs/man"
            man.mkdir(parents=True)
            pages = {"mdrun.1": b".TH mdrun 1\nroot manual\n",
                     "mdrun-session.1": b".TH mdrun-session 1\nsession manual\n",
                     "mdrun-future.1": b".TH mdrun-future 1\nanother manual\n"}
            for name, content in pages.items():
                (man / name).write_bytes(content)
            (man / "not-a-manpage.txt").write_text("not packaged")
            archive = distribution.archive(root, "1.2.3", target, root / "dist")
            prefix = f"mdrun-v1.2.3-{target}"
            with tarfile.open(archive) as result:
                self.assertEqual(
                    set(result.getnames()),
                    {f"{prefix}/{name}" for name in ("mdrun", "README.md", "CHANGELOG.md", "LICENSE", "THIRD_PARTY_NOTICES.txt")}
                    | {f"{prefix}/share/man/man1/{name}" for name in pages},
                )
                self.assertEqual(result.extractfile(f"{prefix}/mdrun").read(), binary.read_bytes())
                self.assertEqual(result.getmember(f"{prefix}/mdrun").mode & 0o111, 0o111)
                self.assertEqual(result.extractfile(f"{prefix}/THIRD_PARTY_NOTICES.txt").read(), b"THIRD_PARTY_NOTICES.txt")
                for name, content in pages.items():
                    self.assertEqual(result.extractfile(f"{prefix}/share/man/man1/{name}").read(), content)

    def test_archive_refuses_missing_root_manpage_even_with_subcommand_pages(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = distribution.TARGETS[0]
            binary = root / "target" / target / "release" / "mdrun"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"binary")
            for name in ("README.md", "CHANGELOG.md", "LICENSE", "THIRD_PARTY_NOTICES.txt"):
                (root / name).write_text(name)
            man = root / "docs/man"
            man.mkdir(parents=True)
            (man / "mdrun-session.1").write_text("subcommand manual")
            with self.assertRaisesRegex(ValueError, "mdrun.1"):
                distribution.archive(root, "1.2.3", target, root / "dist")
            self.assertFalse((root / "dist").exists())

    def test_linux_assets_cover_every_generated_manpage_in_the_man1_directory(self):
        metadata = tomllib.loads((ROOT / "Cargo.toml").read_text())["package"]["metadata"]
        pages = {page.name: page.relative_to(ROOT).as_posix() for page in (ROOT / "docs/man").glob("*.1")}
        self.assertIn("mdrun.1", pages)
        self.assertIn("mdrun-session.1", pages)
        for package, assets in (("deb", metadata["deb"]["assets"]), ("rpm", metadata["generate-rpm"]["assets"])):
            installed = {}
            for asset in assets:
                source, destination, mode = (asset["source"], asset["dest"], asset["mode"]) if isinstance(asset, dict) else asset
                if destination.lstrip("/").startswith("usr/share/man/man1/"):
                    self.assertEqual(mode, "644", package)
                    self.assertTrue(destination.endswith("/"), package)
                    for page in ROOT.glob(source):
                        self.assertTrue(page.is_file(), package)
                        installed[(Path(destination) / page.name).as_posix().lstrip("/")] = page.relative_to(ROOT).as_posix()
            self.assertEqual(installed, {"usr/share/man/man1/" + name: source for name, source in pages.items()}, package)

    def test_archive_refuses_missing_crate_notices(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = distribution.TARGETS[0]
            binary = root / "target" / target / "release" / "mdrun"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"binary")
            for name in ("README.md", "CHANGELOG.md", "LICENSE"):
                (root / name).write_text(name)
            with self.assertRaisesRegex(ValueError, "THIRD_PARTY_NOTICES"):
                distribution.archive(root, "1.2.3", target, root / "dist")

    def test_formula_uses_each_target_checksum_and_has_no_placeholders(self):
        sums = "\n".join(
            f"{str(index) * 64}  mdrun-v1.2.3-{target}.tar.gz"
            for index, target in enumerate(distribution.TARGETS, 1)
        )
        formula = distribution.formula("1.2.3", sums)
        self.assertNotIn("{{", formula)
        self.assertIn('version "1.2.3"', formula)
        for index, target in enumerate(distribution.TARGETS, 1):
            self.assertIn(f"mdrun-v1.2.3-{target}.tar.gz", formula)
            self.assertIn(f'sha256 "{str(index) * 64}"', formula)

    def test_formula_refuses_missing_ambiguous_or_invalid_checksums(self):
        rows = [f"{'a' * 64}  mdrun-v1.2.3-{target}.tar.gz" for target in distribution.TARGETS]
        for content in ("", "\n".join(rows[:-1]), "\n".join(rows + [rows[0]]), "\n".join(rows).replace("a" * 64, "invalid", 1)):
            with self.subTest(content=content), self.assertRaises(ValueError):
                distribution.formula("1.2.3", content)

    def test_versions_and_targets_cannot_change_output_paths(self):
        for version in ("../escape", "v1.2.3", "1.2", "01.2.3", "1.2.3\n"):
            with self.subTest(version=version), self.assertRaises(ValueError):
                distribution.validate_version(version)
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(ValueError):
                distribution.archive(Path(directory), "1.2.3", "../escape", Path(directory))


if __name__ == "__main__":
    unittest.main()
