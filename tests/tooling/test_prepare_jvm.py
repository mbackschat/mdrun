import hashlib
import importlib.util
import json
import os
from datetime import datetime, timezone
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock
import zipfile


ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("prepare_jvm", ROOT / "scripts/prepare_jvm.py")
preparation = importlib.util.module_from_spec(spec)
spec.loader.exec_module(preparation)


class PrepareJvmTests(unittest.TestCase):
    now = datetime(2026, 11, 16, tzinfo=timezone.utc)

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="mdrun-prepare-jvm-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.sdk = self.root / "sdk"
        self.sdk.mkdir()
        self.pom = self.sdk / "pom.xml"
        self.pom.write_bytes((ROOT / "sdk/jvm/pom.xml").read_bytes())
        self.lock = json.loads((ROOT / "sdk/jvm/dependencies.lock.json").read_text())
        self.jar = self.root / "dependency.jar"
        with zipfile.ZipFile(self.jar, "w") as archive:
            for name in self.lock["dependencies"][0]["bundled_notices"]:
                archive.writestr(name, "test notice")
        self.lock["dependencies"][0]["sha256"] = hashlib.sha256(self.jar.read_bytes()).hexdigest()
        self.write_lock()
        self.java = self.root / "jdk"
        (self.java / "bin").mkdir(parents=True)
        (self.java / "release").write_text('JAVA_VERSION="21.0.1"\n')
        for tool in ("javac", "jar"):
            executable = self.java / "bin" / tool
            executable.write_text("#!/bin/sh\nexit 99\n")
            executable.chmod(0o755)
        self.source = self.sdk / "src/main/java/io/github/mbackschat/mdrun/sdk/Example.java"
        self.source.parent.mkdir(parents=True)
        self.source.write_text("package io.github.mbackschat.mdrun.sdk; public final class Example {}\n")
        self.output = self.root / "prepared"
        self.patch = mock.patch.object(preparation, "SDK_ROOT", self.sdk)
        self.patch.start()
        self.addCleanup(self.patch.stop)

    def write_lock(self):
        (self.sdk / "dependencies.lock.json").write_text(json.dumps(self.lock))

    def prepare(self, **kwargs):
        return preparation.prepare(self.java, self.jar, self.output, now=self.now, **kwargs)

    def test_corrupt_dependency_is_rejected_before_any_java_process(self):
        self.jar.write_bytes(self.jar.read_bytes() + b"corrupt")
        with mock.patch.object(subprocess, "run") as run:
            with self.assertRaisesRegex(ValueError, "SHA-256"):
                self.prepare()
        run.assert_not_called()
        self.assertFalse(self.output.exists())

    def test_publication_date_uses_conservative_fourteen_full_days(self):
        with mock.patch.object(subprocess, "run") as run:
            with self.assertRaisesRegex(ValueError, "14 days"):
                preparation.prepare(self.java, self.jar, self.output,
                                    now=datetime(2026, 10, 31, 23, 59, 59, tzinfo=timezone.utc))
        run.assert_not_called()
        preparation.validate_dependency(self.sdk, datetime(2026, 11, 1, tzinfo=timezone.utc))

    def test_pom_lock_version_mismatch_is_rejected_before_java_process(self):
        self.pom.write_text(self.pom.read_text().replace("<version>2.22.4</version>", "<version>2.22.3</version>"))
        with mock.patch.object(subprocess, "run") as run:
            with self.assertRaisesRegex(ValueError, "pom.xml"):
                self.prepare()
        run.assert_not_called()

    def test_prerelease_and_noncanonical_download_are_rejected(self):
        for change in ({"version": "2.22.4-rc1"}, {"artifact_url": "https://example.invalid/core.jar"}):
            with self.subTest(change=change):
                original = self.lock["dependencies"][0].copy()
                self.lock["dependencies"][0].update(change)
                self.write_lock()
                with mock.patch.object(subprocess, "run") as run:
                    with self.assertRaises(ValueError):
                        self.prepare()
                run.assert_not_called()
                self.lock["dependencies"][0] = original

    def test_wrong_java_major_is_rejected_without_running_compiler(self):
        (self.java / "release").write_text('JAVA_VERSION="25.0.1"\n')
        with mock.patch.object(subprocess, "run") as run:
            with self.assertRaisesRegex(ValueError, "Java 21"):
                self.prepare()
        run.assert_not_called()

    def test_missing_required_notice_is_rejected_before_java_process(self):
        with zipfile.ZipFile(self.jar, "w") as archive:
            archive.writestr("META-INF/LICENSE", "only one notice")
        self.lock["dependencies"][0]["sha256"] = hashlib.sha256(self.jar.read_bytes()).hexdigest()
        self.write_lock()
        with mock.patch.object(subprocess, "run") as run:
            with self.assertRaisesRegex(ValueError, "notice"):
                self.prepare()
        run.assert_not_called()

    def test_download_is_hash_checked_before_java_process(self):
        with mock.patch.object(preparation, "download_dependency", return_value=b"unreviewed bytes") as download:
            with mock.patch.object(subprocess, "run") as run:
                with self.assertRaisesRegex(ValueError, "SHA-256"):
                    preparation.prepare(self.java, None, self.output, now=self.now, download=True)
        download.assert_called_once_with(self.lock["dependencies"][0]["artifact_url"])
        run.assert_not_called()

    @unittest.skipUnless(os.environ.get("MDRUN_JAVA_HOME"), "set MDRUN_JAVA_HOME to exercise a real Java 21 toolchain")
    def test_real_jdk_produces_repeatable_sdk_only_jar_and_relative_manifest(self):
        self.java = Path(os.environ["MDRUN_JAVA_HOME"])
        fixture = self.sdk / "src/test/java/Fixture.java"
        fixture.parent.mkdir(parents=True)
        fixture.write_text("this intentionally invalid fixture must never be compiled\n")
        first = self.prepare()
        jar = self.output / first["sdk"]["path"]
        with zipfile.ZipFile(jar) as archive:
            names = archive.namelist()
        self.assertIn("io/github/mbackschat/mdrun/sdk/Example.class", names)
        self.assertFalse(any("Fixture" in name or name.endswith(".java") for name in names))
        self.assertFalse(any(name.startswith("com/fasterxml/") for name in names))
        self.assertIn("META-INF/maven/io.github.mbackschat/mdrun-sdk/pom.xml", names)
        self.assertEqual(json.loads((self.output / "build-manifest.json").read_text()), first)
        self.assertTrue(all(not Path(entry["path"]).is_absolute() for entry in first["classpath"]))
        self.assertEqual((self.output / first["classpath"][1]["path"]).read_bytes(), self.jar.read_bytes())
        self.output = self.root / "prepared-again"
        second = self.prepare()
        self.assertEqual(first, second)
        self.assertEqual(jar.read_bytes(), (self.output / second["sdk"]["path"]).read_bytes())


if __name__ == "__main__":
    unittest.main()
