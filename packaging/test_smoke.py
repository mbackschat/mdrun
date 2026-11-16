import os
from pathlib import Path
import subprocess
import tempfile
import unittest


SMOKE = Path(__file__).with_name("smoke.sh")


class InstalledCommandTests(unittest.TestCase):
    def test_a_version_only_stub_cannot_pass_document_qualification(self):
        with tempfile.TemporaryDirectory() as temporary:
            stub = Path(temporary) / "mdrun"
            stub.write_text("#!/bin/sh\nprintf 'mdrun 0.1.0\\n'\n")
            stub.chmod(0o755)
            result = subprocess.run(["/bin/sh", str(SMOKE), str(stub)], capture_output=True, timeout=10)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(b"recorded command output missing", result.stderr)

    @unittest.skipUnless(os.environ.get("MDRUN_PACKAGE_BINARY"), "requires a separately built package binary")
    def test_standalone_binary_records_replays_and_detects_changed_output(self):
        result = subprocess.run(["/bin/sh", str(SMOKE), os.environ["MDRUN_PACKAGE_BINARY"]],
                                capture_output=True, timeout=20)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn(b"package document smoke passed", result.stdout)


if __name__ == "__main__":
    unittest.main()
