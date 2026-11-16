"""Attribution must describe the compiler used, not merely a crate lockfile."""

import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


spec = importlib.util.spec_from_file_location("toolchain_notices", Path(__file__).with_name("toolchain.py"))
toolchain = importlib.util.module_from_spec(spec)
spec.loader.exec_module(toolchain)


class ToolchainNoticesTests(unittest.TestCase):
    def test_compiler_version_and_source_commit_both_must_match(self):
        pinned = {"release": "1.98.1", "commit": "a" * 40}
        good = "release: 1.98.1\ncommit-hash: " + "a" * 40 + "\n"
        toolchain.check_identity(good, pinned)
        for wrong in (good.replace("1.98.1", "1.98.0"), good.replace("a" * 40, "b" * 40), "release: 1.98.1\n"):
            with self.subTest(wrong=wrong), self.assertRaisesRegex(ValueError, "compiler identity"):
                toolchain.check_identity(wrong, pinned)

    def test_changed_missing_or_empty_upstream_notice_fails_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "notice"
            source.write_bytes(b"unmodified upstream notice\n")
            row = {"path": "notice", "sha256": toolchain.digest(source.read_bytes())}
            self.assertEqual(toolchain.read_verified(root, row), source.read_bytes())
            source.write_bytes(b"changed notice\n")
            with self.assertRaisesRegex(ValueError, "notice checksum"):
                toolchain.read_verified(root, row)
            source.write_bytes(b"")
            with self.assertRaises(ValueError):
                toolchain.read_verified(root, row)
            source.unlink()
            with self.assertRaises(OSError):
                toolchain.read_verified(root, row)

    def test_archive_and_linux_packages_require_runtime_notices(self):
        root = Path(__file__).resolve().parents[1]
        import tomllib
        metadata = tomllib.loads((root / "Cargo.toml").read_text())["package"]["metadata"]
        for assets in (metadata["deb"]["assets"], metadata["generate-rpm"]["assets"]):
            self.assertIn("TOOLCHAIN_NOTICES.txt", [a["source"] if isinstance(a, dict) else a[0] for a in assets])
        inventory = json.loads((root / "packaging/toolchain.json").read_text())
        self.assertIn("COPYRIGHT-library.html", [r["path"] for r in inventory["rust_files"]])
        self.assertIn("COPYRIGHT.html", [r["path"] for r in inventory["rust_files"]])
        self.assertTrue(inventory["musl"]["archive_sha256"])


if __name__ == "__main__":
    unittest.main()
