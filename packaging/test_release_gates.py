"""Guard publication against bypassing qualification of the tagged source."""

from pathlib import Path
import re
import unittest


ROOT = Path(__file__).resolve().parents[1]


class ReleaseGateTests(unittest.TestCase):
    def test_publication_requires_same_run_qualification(self):
        release = (ROOT / ".github/workflows/release.yml").read_text()
        qualification = re.search(r"^  qualification:\n(.*?)(?=^  \w|\Z)", release, re.M | re.S)
        self.assertIsNotNone(qualification, "release must invoke qualification, not inspect historical CI")
        self.assertIn("uses: ./.github/workflows/qualification.yml", qualification[1])
        publish = release.split("\n  publish:\n", 1)[1]
        self.assertRegex(publish, r"needs: \[[^\]]*qualification[^\]]*\]")
        self.assertNotIn("always()", publish)
        self.assertNotIn("continue-on-error", release)

    def test_shared_qualification_covers_runtime_and_differential_on_exact_source(self):
        workflow = (ROOT / ".github/workflows/qualification.yml").read_text()
        self.assertIn("workflow_call:", workflow)
        self.assertGreaterEqual(workflow.count("uses: actions/checkout@"), 2)
        self.assertEqual(workflow.count("uses: actions/checkout@"), workflow.count("ref: ${{ github.sha }}"))
        for control in ("session::broker::tests::", "session::batch::", "session::python::", "session::jvm_", "warm_owner_loss::", "tests/compatibility/oracle.py", "tests/runtime/session/streaming.py"):
            self.assertIn(control, workflow)
        self.assertNotIn("continue-on-error", workflow)
        for platform in ("macos-15", "ubuntu-24.04", "ubuntu-24.04-arm"):
            self.assertIn(platform, workflow)


if __name__ == "__main__":
    unittest.main()
