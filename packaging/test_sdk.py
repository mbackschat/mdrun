import base64
import csv
from email.parser import BytesParser
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import re
import shutil
import tarfile
import tempfile
import unittest
from unittest import mock
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


sdk = load("sdk_distribution", ROOT / "packaging/sdk.py")
consumer = load("sdk_consumer", ROOT / "scripts/prepare_sdk.py")
qualification = load("sdk_qualification", ROOT / "packaging/qualify_sdk.py")


class SdkArchiveTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="mdrun-sdk-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        for name in ("Cargo.toml", "LICENSE", "sdk/node/README.md", "sdk/node/index.mjs",
                     "sdk/python/README.md", "sdk/python/mdrun_sdk/__init__.py", "sdk/jvm/README.md",
                     "sdk/jvm/pom.xml", "sdk/jvm/dependencies.lock.json"):
            target = self.root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, target)
        self.output = self.root / "dist"

    def archive(self, runtime="node", **kwargs):
        return sdk.archive(self.root, runtime, self.output, **kwargs)

    def test_runtime_archives_contain_exact_production_files_and_independent_digests(self):
        for runtime, files in (("node", {"index.mjs"}), ("python", {"mdrun_sdk/__init__.py"})):
            # Extra source-tree files, examples and caches are not distributable inputs.
            (self.root / "sdk" / runtime / "private-adapter.txt").write_text("excluded")
            archive = self.archive(runtime)
            prefix = f"mdrun-sdk-{runtime}-v0.1.0"
            with tarfile.open(archive) as result:
                expected = {"README.md", "LICENSE", "VERSION", "manifest.json"} | files
                self.assertEqual(set(result.getnames()), {f"{prefix}/{name}" for name in expected})
                manifest = json.load(result.extractfile(f"{prefix}/manifest.json"))
                self.assertEqual(manifest["version"], "0.1.0")
                self.assertEqual(manifest["runtime"], runtime)
                readme = result.extractfile(f"{prefix}/README.md").read()
                self.assertIn(b"https://github.com/mbackschat/mdrun/blob/v0.1.0/docs/sdk-distribution.md", readme)
                self.assertNotIn(b"](../../", readme)
                self.assertEqual(set(manifest["files"]), expected - {"manifest.json"})
                for name, digest in manifest["files"].items():
                    data = result.extractfile(f"{prefix}/{name}").read()
                    self.assertEqual(digest, hashlib.sha256(data).hexdigest())
                    self.assertEqual(result.getmember(f"{prefix}/{name}").mtime, 0)
                    self.assertEqual(result.getmember(f"{prefix}/{name}").uid, 0)
                    self.assertEqual(result.getmember(f"{prefix}/{name}").mode, 0o644)

    def test_archive_bytes_are_repeatable_despite_source_mtime_and_output_path(self):
        first = self.archive()
        (self.root / "sdk/node/index.mjs").touch()
        second = sdk.archive(self.root, "node", self.root / "again")
        self.assertEqual(first.read_bytes(), second.read_bytes())
        self.assertEqual(first.read_bytes()[4:8], bytes(4))

    def test_python_wheel_is_standard_purelib_with_exact_metadata_license_and_record(self):
        (self.root / "sdk/python/private-adapter.py").write_text("excluded")
        wheel = sdk.wheel(self.root, self.output)
        self.assertEqual(wheel.name, "mdrun_sdk-0.1.0-py3-none-any.whl")
        info = "mdrun_sdk-0.1.0.dist-info/"
        with zipfile.ZipFile(wheel) as result:
            self.assertEqual(set(result.namelist()), {"mdrun_sdk/__init__.py", info + "METADATA", info + "WHEEL",
                                                     info + "RECORD", info + "licenses/LICENSE"})
            metadata = BytesParser().parsebytes(result.read(info + "METADATA"))
            for key, value in {"Metadata-Version": "2.4", "Name": "mdrun-sdk", "Version": "0.1.0",
                               "Requires-Python": ">=3.12", "License-Expression": "MIT", "License-File": "LICENSE",
                               "Description-Content-Type": "text/markdown"}.items():
                self.assertEqual(metadata[key], value)
            self.assertIsNone(metadata["Requires-Dist"])
            description = metadata.get_payload(decode=True).decode("utf-8")
            maintained = (self.root / "sdk/python/README.md").read_text()
            # Release descriptions resolve repository links to the source tag; preserve all other README content.
            def without_link_targets(text):
                return re.sub(r"(\[[^\]]+\])\([^)]+\)", r"\1", text)

            self.assertEqual(without_link_targets(description), without_link_targets(maintained))
            wheel_metadata = BytesParser().parsebytes(result.read(info + "WHEEL"))
            self.assertEqual(wheel_metadata["Wheel-Version"], "1.0")
            self.assertEqual(wheel_metadata["Root-Is-Purelib"], "true")
            self.assertEqual(wheel_metadata["Tag"], "py3-none-any")
            self.assertEqual(result.read("mdrun_sdk/__init__.py"), (self.root / "sdk/python/mdrun_sdk/__init__.py").read_bytes())
            self.assertEqual(result.read(info + "licenses/LICENSE"), (self.root / "LICENSE").read_bytes())
            rows = list(csv.reader(io.StringIO(result.read(info + "RECORD").decode())))
            self.assertEqual({row[0] for row in rows}, set(result.namelist()))
            self.assertEqual(len(rows), len(result.namelist()))
            for name, digest, size in rows:
                if name == info + "RECORD":
                    self.assertEqual((digest, size), ("", ""))
                else:
                    payload = result.read(name)
                    expected = "sha256=" + base64.urlsafe_b64encode(hashlib.sha256(payload).digest()).rstrip(b"=").decode()
                    self.assertEqual(digest, expected)
                    self.assertEqual(int(size), len(payload))
            for entry in result.infolist():
                self.assertEqual(entry.date_time, (1980, 1, 1, 0, 0, 0))
                self.assertEqual(entry.external_attr >> 16, 0o100644)

    def test_python_wheel_reproduces_and_refuses_version_drift_or_existing_output(self):
        wheel = sdk.wheel(self.root, self.output)
        (self.root / "sdk/python/mdrun_sdk/__init__.py").touch()
        self.assertEqual(wheel.read_bytes(), sdk.wheel(self.root, self.root / "repeat").read_bytes())
        with self.assertRaises(FileExistsError):
            sdk.wheel(self.root, self.output)
        pom = self.root / "sdk/jvm/pom.xml"
        pom.write_text(pom.read_text().replace("<version>0.1.0</version>", "<version>0.1.1</version>"))
        with self.assertRaisesRegex(ValueError, "version"):
            sdk.wheel(self.root, self.root / "bad-version")
        self.assertFalse((self.root / "bad-version").exists())

    def test_version_drift_and_symlinked_runtime_are_rejected_before_output(self):
        pom = self.root / "sdk/jvm/pom.xml"
        pom.write_text(pom.read_text().replace("<version>0.1.0</version>", "<version>0.1.1</version>"))
        with self.assertRaisesRegex(ValueError, "version"):
            self.archive()
        self.assertFalse(self.output.exists())
        shutil.copyfile(ROOT / "sdk/jvm/pom.xml", pom)
        runtime = self.root / "sdk/node/index.mjs"
        runtime.unlink()
        runtime.symlink_to(ROOT / "sdk/node/index.mjs")
        with self.assertRaisesRegex(ValueError, "regular"):
            self.archive()

    def prepared_jvm(self):
        prepared = self.root / "prepared"
        prepared.mkdir()
        lock = json.loads((self.root / "sdk/jvm/dependencies.lock.json").read_text())
        dependency = lock["dependencies"][0]
        jackson = prepared / f"jackson-core-{dependency['version']}.jar"
        with zipfile.ZipFile(jackson, "w") as result:
            for name in dependency["bundled_notices"]:
                result.writestr(name, "synthetic dependency notice\n")
        dependency["sha256"] = hashlib.sha256(jackson.read_bytes()).hexdigest()
        (self.root / "sdk/jvm/dependencies.lock.json").write_text(json.dumps(lock))
        jar = prepared / "mdrun-sdk.jar"
        with zipfile.ZipFile(jar, "w") as result:
            result.writestr("io/github/mbackschat/mdrun/sdk/Worker.class", b"synthetic class")
            result.writestr("META-INF/LICENSE", (self.root / "LICENSE").read_bytes())
            result.writestr("META-INF/maven/io.github.mbackschat/mdrun-sdk/pom.xml", (self.root / "sdk/jvm/pom.xml").read_bytes())
        sdk_entry = {"path": jar.name, "sha256": hashlib.sha256(jar.read_bytes()).hexdigest()}
        manifest = {"schema_version": 1, "java_release": 21, "sdk": sdk_entry,
                    "classpath": [sdk_entry, {"path": jackson.name, "sha256": dependency["sha256"]}],
                    "dependency": {"coordinates": "com.fasterxml.jackson.core:jackson-core:" + dependency["version"],
                                   "published": dependency["published"],
                                   "canonical_artifact_verified": dependency["canonical_artifact_verified"]}}
        (prepared / "build-manifest.json").write_text(json.dumps(manifest))
        return prepared

    def test_jvm_archive_retains_prepared_jars_pom_dependency_provenance_and_all_notices(self):
        prepared = self.prepared_jvm()
        archive = self.archive("jvm", jvm_prepared=prepared)
        contents = consumer.validate_archive(archive.read_bytes(), "jvm", "0.1.0")
        expected = {"README.md", "LICENSE", "VERSION", "manifest.json", "mdrun-sdk.jar", "jackson-core-2.22.4.jar",
                    "pom.xml", "dependencies.lock.json", "build-manifest.json"}
        lock = json.loads((self.root / "sdk/jvm/dependencies.lock.json").read_text())
        expected |= {"licenses/jackson-core/" + Path(name).name for name in lock["dependencies"][0]["bundled_notices"]}
        self.assertEqual(set(contents), expected)
        for name in ("mdrun-sdk.jar", "jackson-core-2.22.4.jar", "build-manifest.json"):
            self.assertEqual(contents[name], (prepared / name).read_bytes())
        self.assertEqual(contents["pom.xml"], (self.root / "sdk/jvm/pom.xml").read_bytes())

    def test_jvm_preparation_is_rechecked_for_hashes_metadata_and_sdk_only_classes(self):
        prepared = self.prepared_jvm()
        manifest_path = prepared / "build-manifest.json"
        original = manifest_path.read_bytes()
        manifest = json.loads(original)
        manifest["classpath"][1]["path"] = "../escape.jar"
        manifest_path.write_text(json.dumps(manifest))
        with self.assertRaisesRegex(ValueError, "manifest"):
            self.archive("jvm", jvm_prepared=prepared)
        manifest_path.write_bytes(original)
        jar = prepared / "mdrun-sdk.jar"
        with zipfile.ZipFile(jar, "a") as result:
            result.writestr("private/Adapter.class", b"excluded")
        manifest = json.loads(original)
        manifest["sdk"]["sha256"] = hashlib.sha256(jar.read_bytes()).hexdigest()
        manifest["classpath"][0] = manifest["sdk"]
        manifest_path.write_text(json.dumps(manifest))
        with self.assertRaisesRegex(ValueError, "SDK"):
            self.archive("jvm", jvm_prepared=prepared)
        self.assertFalse(self.output.exists())

    def test_standalone_preparation_checks_pinned_version_and_digest_before_writing(self):
        archive = self.archive()
        digest = hashlib.sha256(archive.read_bytes()).hexdigest()
        output = self.root / "consumer"
        for version, checksum in (("0.1.1", digest), ("0.1.0", "0" * 64)):
            with self.subTest(version=version, checksum=checksum), self.assertRaises(ValueError):
                consumer.prepare("node", version, checksum, output, archive=archive)
            self.assertFalse(output.exists())
        consumer.prepare("node", "0.1.0", digest, output, archive=archive)
        self.assertEqual((output / "index.mjs").read_bytes(), (self.root / "sdk/node/index.mjs").read_bytes())
        with self.assertRaisesRegex(ValueError, "exist"):
            consumer.prepare("node", "0.1.0", digest, output, archive=archive)

    def malicious_archive(self, change):
        original = self.archive()
        members = []
        with tarfile.open(original) as result:
            for member in result:
                members.append((member, result.extractfile(member).read()))
        change(members)
        stream = io.BytesIO()
        with tarfile.open(fileobj=stream, mode="w:gz") as result:
            for member, data in members:
                member.size = len(data)
                result.addfile(member, io.BytesIO(data))
        return stream.getvalue()

    def test_consumer_rejects_tampered_missing_extra_duplicate_links_and_traversal_entries(self):
        mutations = {
            "tampered": lambda members: members.__setitem__(0, (members[0][0], b"altered")),
            "missing": lambda members: members.pop(0),
            "extra": lambda members: members.append((tarfile.TarInfo("mdrun-sdk-node-v0.1.0/private.txt"), b"extra")),
            "duplicate": lambda members: members.append(members[0]),
            "link": lambda members: setattr(members[0][0], "type", tarfile.SYMTYPE),
            "escape": lambda members: setattr(members[0][0], "name", "../escape"),
        }
        for name, change in mutations.items():
            # Rebuild the original archive for every independent mutation.
            self.output = self.root / name
            with self.subTest(name=name), self.assertRaises(ValueError):
                consumer.validate_archive(self.malicious_archive(change), "node", "0.1.0")

    def test_download_pins_the_canonical_version_url_and_verifies_before_preparation(self):
        archive = self.archive()
        payload = archive.read_bytes()
        digest = hashlib.sha256(payload).hexdigest()
        response = mock.MagicMock()
        response.__enter__.return_value = response
        response.url = "https://release-assets.githubusercontent.com/synthetic-release-asset"
        response.read.return_value = payload
        output = self.root / "downloaded"
        with mock.patch.object(consumer, "urlopen", return_value=response) as open_url:
            consumer.prepare("node", "0.1.0", digest, output, download=True)
        request = open_url.call_args.args[0]
        self.assertEqual(request.full_url, "https://github.com/mbackschat/mdrun/releases/download/v0.1.0/mdrun-sdk-node-v0.1.0.tar.gz")
        response.read.assert_called_once_with(consumer.MAX_ARCHIVE_BYTES + 1)
        response.read.return_value = payload + b"corrupt"
        with mock.patch.object(consumer, "urlopen", return_value=response):
            with self.assertRaisesRegex(ValueError, "SHA-256"):
                consumer.prepare("node", "0.1.0", digest, self.root / "corrupt-download", download=True)
        self.assertFalse((self.root / "corrupt-download").exists())

    def test_consumer_identity_covers_artifacts_launch_context_and_ignored_environment(self):
        artifacts = {"index.mjs": "a" * 64}
        binding = {"worker_argv": ["/node", "/worker.mjs"], "cold_argv": ["/node", "/cold.mjs"],
                   "context": {"environment": {"LANG": "C"}}, "warm_ignore_env": ["SHLVL", "_"]}
        expected = "sha256:" + hashlib.sha256(json.dumps(
            {"artifacts": artifacts, "binding": binding}, sort_keys=True).encode()).hexdigest()
        self.assertEqual(qualification.artifact_identity(artifacts, binding), expected)
        for change in ({"worker_argv": ["/other-node", "/worker.mjs"]}, {"warm_ignore_env": []},
                       {"context": {"environment": {"LANG": "other"}}}):
            with self.subTest(change=change):
                self.assertNotEqual(qualification.artifact_identity(artifacts, binding | change), expected)
        self.assertNotEqual(qualification.artifact_identity({"index.mjs": "b" * 64}, binding), expected)

    def test_real_host_oracle_rejects_wrong_routes_cleanup_and_application_status(self):
        report = {"outcome": "success", "infrastructure_failure": None, "cleanup": {"outcome": "clean"},
                  "warm_calls": 2, "cold_calls": 0, "worker_starts": 1,
                  "application_statuses": [{"exit_code": 0, "signal": None, "route": "warm"}] * 2}
        qualification.check_report(report, "fast", 2)
        for change in ({"worker_starts": 2}, {"cold_calls": 1}, {"warm_calls": 1},
                       {"cleanup": {"outcome": "uncertain"}}, {"infrastructure_failure": "fault"},
                       {"application_statuses": [{"exit_code": 0, "signal": None, "route": "cold"}] * 2},
                       {"application_statuses": [{"exit_code": 1, "signal": None, "route": "warm"}] * 2}):
            with self.subTest(change=change), self.assertRaises(ValueError):
                qualification.check_report(report | change, "fast", 2)

    def test_consumer_limits_size_and_rejects_noncanonical_version_or_digest(self):
        for version in ("../escape", "v0.1.0", "0.1", "00.1.0", "0.1.0\n"):
            with self.subTest(version=version), self.assertRaises(ValueError):
                consumer.prepare("node", version, "a" * 64, self.root / "unused", archive=self.root / "none")
        with self.assertRaises(ValueError):
            consumer.prepare("node", "0.1.0", "not-a-digest", self.root / "unused", archive=self.root / "none")
        with self.assertRaisesRegex(ValueError, "limit"):
            consumer.validate_archive(b"x" * (consumer.MAX_ARCHIVE_BYTES + 1), "node", "0.1.0")


if __name__ == "__main__":
    unittest.main()
