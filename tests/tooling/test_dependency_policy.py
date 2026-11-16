import importlib.util
from datetime import datetime, timedelta, timezone
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("dependency_policy", Path(__file__).resolve().parents[2] / "scripts/dependency_policy.py")
policy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(policy)


class DependencyPolicyTests(unittest.TestCase):
    now = datetime(2026, 11, 16, tzinfo=timezone.utc)

    def release(self, age=14, version="1.2.3", yanked=False):
        return {"num": version, "yanked": yanked, "created_at": (self.now - timedelta(days=age)).isoformat()}

    def test_fourteen_days_is_an_exact_boundary(self):
        self.assertEqual(policy.check_release("sample", self.release(), self.now, []), [])
        self.assertTrue(policy.check_release("sample", self.release(age=13.99), self.now, []))

    def test_stable_build_metadata_is_allowed_but_prerelease_and_yanks_are_not(self):
        self.assertEqual(policy.check_release("sample", self.release(version="1.0.0+build-1"), self.now, []), [])
        self.assertTrue(policy.check_release("sample", self.release(version="1.0.0-rc.1"), self.now, []))
        self.assertTrue(policy.check_release("sample", self.release(yanked=True), self.now, []))

    def test_exception_requires_exact_version_advisory_review_and_expiry(self):
        exception = {"name": "sample", "version": "1.2.3", "advisory": "https://rustsec.org/advisories/RUSTSEC-2026-0000.html",
                     "rationale": "Verified upstream fix applies to the enabled feature.",
                     "reviewed_change": "https://github.com/example/sample/commit/1234",
                     "tests": "Focused regression and full suite passed.", "expires": "2026-11-20"}
        self.assertEqual(policy.check_release("sample", self.release(age=1), self.now, [exception]), [])
        for field in exception:
            incomplete = {key: value for key, value in exception.items() if key != field}
            self.assertTrue(policy.check_release("sample", self.release(age=1), self.now, [incomplete]), field)
        self.assertTrue(policy.check_release("other", self.release(age=1), self.now, [exception]))
        self.assertTrue(policy.check_release("sample", self.release(age=1, version="1.2.4"), self.now, [exception]))
        self.assertTrue(policy.check_release("sample", self.release(age=1), self.now + timedelta(days=5), [exception]))
        self.assertTrue(policy.check_release("sample", self.release(age=1, yanked=True), self.now, [exception]))

    def test_every_locked_registry_package_is_checked_including_transitives(self):
        packages = [{"name": "mdrun", "version": "0.1.0"},
                    {"name": "direct", "version": "1.2.3", "source": policy.REGISTRY},
                    {"name": "transitive", "version": "1.2.3", "source": policy.REGISTRY}]
        metadata = {"direct": {"versions": [self.release()]}, "transitive": {"versions": [self.release(age=1)]}}
        problems = policy.check_packages(packages, metadata, self.now, [])
        self.assertEqual(len(problems), 1)
        self.assertIn("transitive", problems[0])
        with self.assertRaises(ValueError):
            policy.check_packages(packages, {}, self.now, [])
        with self.assertRaises(ValueError):
            policy.check_packages([{**packages[1], "source": "git+https://example.invalid/pkg"}], {}, self.now, [])


if __name__ == "__main__":
    unittest.main()
