import hashlib
from io import BytesIO
import importlib.util
import json
import os
from datetime import datetime, timezone
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock
from urllib.error import HTTPError
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

    def test_download_cache_avoids_network_and_revalidates_every_hit(self):
        dependency = self.lock["dependencies"][0]
        cache = self.root / "cache"
        data = self.jar.read_bytes()
        with mock.patch.object(preparation, "download_dependency", return_value=data) as download:
            self.assertEqual(preparation.load_download(dependency, cache), data)
            self.assertEqual(preparation.load_download(dependency, cache), data)
            download.assert_called_once()
            entries = list(cache.iterdir())
            self.assertEqual(len(entries), 1)
            entries[0].write_bytes(b"corrupt")
            with self.assertRaisesRegex(ValueError, "SHA-256"):
                preparation.load_download(dependency, cache)
            download.assert_called_once()

    def test_invalid_download_is_never_cached(self):
        cache = self.root / "cache"
        with mock.patch.object(preparation, "download_dependency", return_value=b"corrupt"):
            with self.assertRaisesRegex(ValueError, "SHA-256"):
                preparation.load_download(self.lock["dependencies"][0], cache)
        self.assertFalse(cache.exists() and list(cache.iterdir()))

    def test_failed_cache_publication_leaves_no_partial_entry(self):
        cache = self.root / "cache"
        with mock.patch.object(preparation, "download_dependency", return_value=self.jar.read_bytes()), mock.patch.object(Path, "replace", side_effect=OSError("disk full")):
            with self.assertRaisesRegex(OSError, "disk full"):
                preparation.load_download(self.lock["dependencies"][0], cache)
        self.assertEqual(list(cache.iterdir()), [])

    def test_cache_is_used_by_preparation_before_compilation(self):
        dependency = self.lock["dependencies"][0]
        cache = self.root / "cache"
        cache.mkdir()
        (cache / (dependency["sha256"] + ".jar")).write_bytes(b"corrupt")
        with mock.patch.object(preparation, "download_dependency") as download, mock.patch.object(subprocess, "run") as run:
            with self.assertRaisesRegex(ValueError, "SHA-256"):
                preparation.prepare(self.java, None, self.output, now=self.now, download=True, cache_dir=cache)
        download.assert_not_called()
        run.assert_not_called()

    def test_rate_limit_waits_then_returns_exact_bytes(self):
        url = self.lock["dependencies"][0]["artifact_url"]
        error = HTTPError(url, 429, "rate limited", {"Retry-After": "7"}, BytesIO())
        opener = mock.Mock()
        opener.open.side_effect = [error, BytesIO(b"jar bytes")]
        with mock.patch.object(preparation, "build_opener", return_value=opener), mock.patch.object(preparation, "sleep", create=True) as sleep:
            self.assertEqual(preparation.download_dependency(url), b"jar bytes")
        self.assertEqual(opener.open.call_count, 2)
        sleep.assert_called_once_with(7)
        self.assertTrue(error.fp.closed)

    def test_repeated_transient_errors_have_finite_attempts_and_backoff(self):
        url = self.lock["dependencies"][0]["artifact_url"]
        for status in (429, 502, 503, 504):
            with self.subTest(status=status):
                opener = mock.Mock()
                opener.open.side_effect = [HTTPError(url, status, "unavailable", {}, BytesIO()) for _ in range(3)]
                with mock.patch.object(preparation, "build_opener", return_value=opener), mock.patch.object(preparation, "sleep") as sleep:
                    with self.assertRaises(HTTPError):
                        preparation.download_dependency(url)
                self.assertEqual(opener.open.call_count, 3)
                self.assertEqual(sleep.call_args_list, [mock.call(2), mock.call(4)])

    def test_permanent_errors_and_long_retry_after_do_not_retry(self):
        url = self.lock["dependencies"][0]["artifact_url"]
        for status, headers in ((403, {}), (404, {}), (302, {}), (429, {"Retry-After": "120"}),
                                (429, {"Retry-After": "9" * 400})):
            with self.subTest(status=status, headers=headers):
                opener = mock.Mock()
                opener.open.side_effect = HTTPError(url, status, "refused", headers, BytesIO())
                with mock.patch.object(preparation, "build_opener", return_value=opener), mock.patch.object(preparation, "sleep") as sleep:
                    with self.assertRaises(HTTPError):
                        preparation.download_dependency(url)
                opener.open.assert_called_once()
                sleep.assert_not_called()

    def test_retry_after_accepts_dates_and_ignores_invalid_values(self):
        now = datetime(2026, 11, 16, 12, 0, 0, tzinfo=timezone.utc)
        self.assertEqual(preparation.retry_delay("Mon, 16 Nov 2026 12:00:09 GMT", 2, now), 9)
        for value in (None, "bad", "-1", "NaN"):
            with self.subTest(value=value):
                self.assertEqual(preparation.retry_delay(value, 4, now), 4)
        self.assertEqual(preparation.retry_delay("0", 2, now), 2)

    def test_oversized_download_is_not_retried(self):
        opener = mock.Mock()
        opener.open.return_value = BytesIO(b"12345")
        with mock.patch.object(preparation, "build_opener", return_value=opener), mock.patch.object(preparation, "MAX_DOWNLOAD_BYTES", 4), mock.patch.object(preparation, "sleep") as sleep:
            with self.assertRaisesRegex(ValueError, "byte limit"):
                preparation.download_dependency(self.lock["dependencies"][0]["artifact_url"])
        opener.open.assert_called_once()
        sleep.assert_not_called()

    @unittest.skipUnless(os.environ.get("MDRUN_JAVA_HOME"), "set MDRUN_JAVA_HOME to exercise a real Java 21 toolchain")
    def test_real_jdk_cache_miss_and_hit_produce_identical_preparations(self):
        cache = self.root / "cache"
        java = Path(os.environ["MDRUN_JAVA_HOME"])
        with mock.patch.object(preparation, "download_dependency", return_value=self.jar.read_bytes()) as download:
            first = preparation.prepare(java, None, self.output, now=self.now, download=True, cache_dir=cache)
            second = preparation.prepare(java, None, self.root / "again", now=self.now, download=True, cache_dir=cache)
        download.assert_called_once()
        self.assertEqual(first, second)

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
