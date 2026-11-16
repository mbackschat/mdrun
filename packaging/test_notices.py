import hashlib
import importlib.util
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest import mock


spec = importlib.util.spec_from_file_location("notices", Path(__file__).with_name("notices.py"))
notices = importlib.util.module_from_spec(spec)
spec.loader.exec_module(notices)


class NoticeFixture:
    def __init__(self, root, files=None, name="fixture", version="1.2.3", license_file=None):
        self.root = root
        self.lock = root / "Cargo.lock"
        self.metadata = root / "metadata.json"
        self.archive = root / "registry/cache/example" / f"{name}-{version}.crate"
        self.archive.parent.mkdir(parents=True)
        self.package = {"name": name, "version": version, "source": notices.REGISTRY,
                        "manifest_path": str(root / "registry/src/example" / f"{name}-{version}" / "Cargo.toml"),
                        "license": "MIT", "license_file": license_file}
        if files is None:
            files = {"LICENSE-MIT": b"Copyright upstream\npermission\r\n",
                     "nested/component/NOTICE": b"Distinct nested attribution\n",
                     "src/lib.rs": b"implementation must not be copied"}
        files = dict(files)
        manifest = f'[package]\nname = "{name}"\nversion = "{version}"\nlicense = "MIT"\n'
        if license_file:
            manifest += f'license-file = "{license_file}"\n'
        files["Cargo.toml"] = manifest.encode()
        with tarfile.open(self.archive, "w:gz") as tar:
            for path, data in files.items():
                member = tarfile.TarInfo(f"{name}-{version}/{path}")
                member.size = len(data)
                tar.addfile(member, io.BytesIO(data))
        checksum = hashlib.sha256(self.archive.read_bytes()).hexdigest()
        self.lock.write_text(f'version = 4\n[[package]]\nname = "{name}"\nversion = "{version}"\nsource = "{notices.REGISTRY}"\nchecksum = "{checksum}"\n')
        self.metadata.write_text(json.dumps({"packages": [self.package]}))

    def save(self):
        inventory, bundle = notices.collect(self.lock, self.metadata)
        (self.root / "packaging").mkdir()
        (self.root / notices.INVENTORY).write_text(json.dumps(inventory))
        (self.root / notices.BUNDLE).write_bytes(bundle)
        return inventory, bundle

    def recipe(self, headers):
        path = self.root / "notice-extracts.json"
        path.write_text(json.dumps({"version": 1, "packages": [{
            "name": self.package["name"], "version": self.package["version"], "source": notices.REGISTRY,
            "checksum": hashlib.sha256(self.archive.read_bytes()).hexdigest(), "headers": headers}]}))
        return path


class NoticeTests(unittest.TestCase):
    def test_curated_source_header_ranges_preserve_notices_without_implementation(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            c_header = b"/* Copyright Vendor C\n * Permission to redistribute with this notice.\n * No warranty. */\n"
            rust_header = b"// Copyright Vendor Rust\n// Permission to redistribute with this notice.\n// No warranty.\n"
            sources = {"src/vendor.c": c_header + b"int implementation_must_not_be_copied;\n",
                       "src/vendor.rs": rust_header + b"fn implementation_must_not_be_copied() {}\n"}
            fixture = NoticeFixture(root, {"LICENSE": b"package notice", **sources})
            headers = [{"path": path, "source_sha256": hashlib.sha256(source).hexdigest(),
                        "first_line": 1, "last_line": 3, "sha256": hashlib.sha256(header).hexdigest()}
                       for (path, source), header in zip(sources.items(), (c_header, rust_header))]
            recipe = fixture.recipe(headers)
            data = json.loads(recipe.read_text())
            data["packages"][0]["limitations"] = ["A referenced upstream author file is unavailable."]
            recipe.write_text(json.dumps(data))
            with mock.patch.object(notices, "EXTRACTS", recipe):
                inventory, bundle = fixture.save()
                self.assertIn(c_header, bundle)
                self.assertIn(rust_header, bundle)
                self.assertNotIn(b"implementation_must_not_be_copied", bundle)
                self.assertEqual(inventory["packages"][0]["source_headers"], headers)
                self.assertEqual(inventory["packages"][0]["notice_limits"], data["packages"][0]["limitations"])
                self.assertIn(b"Notice scope limit: A referenced upstream author file is unavailable.", bundle)
                notices.check(root)
                notices.check(root, fixture.metadata)

    def test_curated_source_identity_and_range_cannot_drift_into_implementation(self):
        header = b"// Copyright Vendor\n// Permission and disclaimer.\n"
        source = header + b"fn implementation_must_not_be_copied() {}\n"
        for change, message in [
            ({"source_sha256": "0" * 64}, "file checksum"),
            ({"sha256": "0" * 64}, "range checksum"),
            ({"last_line": 3}, "range checksum"),
            ({"last_line": 100}, "exceeds file"),
            ({"path": "../vendor.rs"}, "path"),
            ({"first_line": True}, "line range"),
        ]:
            with self.subTest(change=change), tempfile.TemporaryDirectory() as directory:
                fixture = NoticeFixture(Path(directory), {"LICENSE": b"package notice", "src/vendor.rs": source})
                entry = {"path": "src/vendor.rs", "source_sha256": hashlib.sha256(source).hexdigest(),
                         "first_line": 1, "last_line": 2, "sha256": hashlib.sha256(header).hexdigest(), **change}
                recipe = fixture.recipe([entry])
                with mock.patch.object(notices, "EXTRACTS", recipe):
                    with self.assertRaisesRegex(ValueError, message):
                        notices.collect(fixture.lock, fixture.metadata)

    def test_curated_source_recipe_stays_pinned_to_the_reviewed_archive_and_version(self):
        header = b"// Copyright Vendor\n// Permission and disclaimer.\n"
        for change in ({"checksum": "0" * 64}, {"version": "1.2.2"}):
            with self.subTest(change=change), tempfile.TemporaryDirectory() as directory:
                fixture = NoticeFixture(Path(directory), {"LICENSE": b"package notice", "src/vendor.rs": header})
                entry = {"path": "src/vendor.rs", "source_sha256": hashlib.sha256(header).hexdigest(),
                         "first_line": 1, "last_line": 2, "sha256": hashlib.sha256(header).hexdigest()}
                recipe = fixture.recipe([entry])
                data = json.loads(recipe.read_text())
                data["packages"][0].update(change)
                recipe.write_text(json.dumps(data))
                with mock.patch.object(notices, "EXTRACTS", recipe):
                    with self.assertRaisesRegex(ValueError, "needs review"):
                        notices.collect(fixture.lock, fixture.metadata)

    def test_source_header_inventory_is_checked_even_without_cached_archives(self):
        with tempfile.TemporaryDirectory() as directory:
            header = b"// Copyright Vendor\n// Permission and disclaimer.\n"
            fixture = NoticeFixture(Path(directory), {"LICENSE": b"package notice", "src/vendor.rs": header})
            entry = {"path": "src/vendor.rs", "source_sha256": hashlib.sha256(header).hexdigest(),
                     "first_line": 1, "last_line": 2, "sha256": hashlib.sha256(header).hexdigest()}
            recipe = fixture.recipe([entry])
            with mock.patch.object(notices, "EXTRACTS", recipe):
                inventory, _ = fixture.save()
                inventory["packages"][0]["source_headers"][0]["last_line"] = 1
                (fixture.root / notices.INVENTORY).write_text(json.dumps(inventory))
                with self.assertRaisesRegex(ValueError, "source-header inventory"):
                    notices.check(fixture.root)

    def test_locked_notice_bytes_include_nested_attribution_but_not_source(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = NoticeFixture(Path(directory))
            inventory, bundle = fixture.save()
            row = inventory["packages"][0]
            self.assertEqual([f["path"] for f in row["files"]], ["LICENSE-MIT", "nested/component/NOTICE"])
            self.assertIn(b"Copyright upstream\npermission\r\n", bundle)
            self.assertIn(b"Distinct nested attribution\n", bundle)
            self.assertNotIn(b"implementation must not be copied", bundle)
            self.assertEqual(row["files"][1]["sha256"], hashlib.sha256(b"Distinct nested attribution\n").hexdigest())
            notices.check(fixture.root)
            notices.check(fixture.root, fixture.metadata)

    def test_metadata_must_cover_exactly_the_locked_identities(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = NoticeFixture(Path(directory))
            for packages in ([], [fixture.package, fixture.package], [dict(fixture.package, version="2.0.0")]):
                with self.subTest(packages=packages):
                    fixture.metadata.write_text(json.dumps({"packages": packages}))
                    with self.assertRaisesRegex(ValueError, "identities"):
                        notices.collect(fixture.lock, fixture.metadata)

    def test_separate_copyright_notice_is_preserved(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = NoticeFixture(Path(directory), {"LICENSE-MIT": b"grant", "COPYRIGHT": b"Distinct upstream attribution"})
            _, bundle = notices.collect(fixture.lock, fixture.metadata)
            self.assertIn(b"Distinct upstream attribution", bundle)

    def test_changed_upstream_archive_is_rejected_before_reading_notices(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = NoticeFixture(Path(directory))
            fixture.archive.write_bytes(fixture.archive.read_bytes() + b"modified")
            with self.assertRaisesRegex(ValueError, "archive checksum"):
                notices.collect(fixture.lock, fixture.metadata)

    def test_license_declaration_is_checked_against_the_locked_archive(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = NoticeFixture(Path(directory))
            fixture.package["license"] = "different"
            fixture.metadata.write_text(json.dumps({"packages": [fixture.package]}))
            with self.assertRaisesRegex(ValueError, "license declaration"):
                notices.collect(fixture.lock, fixture.metadata)

    def test_missing_empty_or_missing_declared_notice_is_rejected(self):
        cases = [({"src/lib.rs": b"code"}, None), ({"LICENSE": b"\n"}, None),
                 ({"LICENSE": b"license"}, "COPYING")]
        for files, declared in cases:
            with self.subTest(files=files), tempfile.TemporaryDirectory() as directory:
                fixture = NoticeFixture(Path(directory), files, license_file=declared)
                with self.assertRaisesRegex(ValueError, "notice"):
                    notices.collect(fixture.lock, fixture.metadata)

    def test_reviewed_authors_exception_carries_the_full_upstream_file(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = NoticeFixture(Path(directory), {"AUTHORS": b"MIT grant and copyright holders\n"}, "r-efi", "6.0.0")
            inventory, bundle = notices.collect(fixture.lock, fixture.metadata)
            self.assertEqual(inventory["packages"][0]["license_choice"], "MIT")
            self.assertIn(b"MIT grant and copyright holders\n", bundle)

    def test_checked_bundle_and_inventory_cannot_drift_from_the_lock_or_sources(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = NoticeFixture(Path(directory))
            inventory, bundle = fixture.save()
            (fixture.root / notices.BUNDLE).write_bytes(bundle + b"altered")
            with self.assertRaisesRegex(ValueError, "bundle checksum"):
                notices.check(fixture.root)
            (fixture.root / notices.BUNDLE).write_bytes(bundle)
            inventory["packages"][0]["files"][0]["sha256"] = "0" * 64
            (fixture.root / notices.INVENTORY).write_text(json.dumps(inventory))
            with self.assertRaisesRegex(ValueError, "upstream archives"):
                notices.check(fixture.root, fixture.metadata)
            fixture.lock.write_text(fixture.lock.read_text().replace('version = "1.2.3"', 'version = "1.2.4"'))
            with self.assertRaisesRegex(ValueError, "identities"):
                notices.check(fixture.root)


if __name__ == "__main__":
    unittest.main()
