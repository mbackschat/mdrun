import importlib.util
from pathlib import Path
import tarfile
import tempfile
import unittest


spec = importlib.util.spec_from_file_location(
    "distribution", Path(__file__).with_name("distribution.py")
)
distribution = importlib.util.module_from_spec(spec)
spec.loader.exec_module(distribution)


class DistributionTests(unittest.TestCase):
    def test_archive_contains_the_exact_binary_and_required_documents(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = distribution.TARGETS[0]
            binary = root / "target" / target / "release" / "mdrun"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"independent-binary-fixture\x00\xff")
            binary.chmod(0o755)
            for name in ("README.md", "CHANGELOG.md", "LICENSE"):
                (root / name).write_text(name)
            archive = distribution.archive(root, "1.2.3", target, root / "dist")
            prefix = f"mdrun-v1.2.3-{target}"
            with tarfile.open(archive) as result:
                self.assertEqual(
                    set(result.getnames()),
                    {f"{prefix}/{name}" for name in ("mdrun", "README.md", "CHANGELOG.md", "LICENSE")},
                )
                self.assertEqual(result.extractfile(f"{prefix}/mdrun").read(), binary.read_bytes())
                self.assertEqual(result.getmember(f"{prefix}/mdrun").mode & 0o111, 0o111)

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
