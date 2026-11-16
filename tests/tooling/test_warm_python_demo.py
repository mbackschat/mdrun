from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
APPLICATION = ROOT / "examples/warm-python/application.py"


class WarmPythonDemoTests(unittest.TestCase):
    def test_cli_counts_piped_input_without_files(self):
        cases = [
            (b"red green blue\n", b"lines=1 words=3\n"),
            (b"red green blue gold silver\n", b"lines=1 words=5\n"),
            (b"", b"lines=0 words=0\n"),
            ((" " * 65535 + "é bleu\n").encode(), b"lines=1 words=2\n"),
        ]
        with tempfile.TemporaryDirectory(prefix="mdrun-stdin-example-") as directory:
            for data, expected in cases:
                with self.subTest(size=len(data)):
                    result = subprocess.run([sys.executable, str(APPLICATION)], input=data,
                                            capture_output=True, cwd=directory, timeout=5)
                    self.assertEqual((result.returncode, result.stdout, result.stderr),
                                     (0, expected, b""))
            self.assertEqual(list(Path(directory).iterdir()), [])

    def test_invalid_utf8_is_an_application_failure(self):
        result = subprocess.run([sys.executable, str(APPLICATION)], input=b"\xff",
                                capture_output=True, timeout=5)
        self.assertEqual(result.returncode, 1)
        self.assertEqual(result.stdout, b"")
        self.assertTrue(result.stderr.startswith(b"wordcount: "))


if __name__ == "__main__":
    unittest.main()
