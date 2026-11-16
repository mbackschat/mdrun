"""Offline guards for the differential oracle, independent of Showboat."""

import copy
from contextlib import nullcontext
from pathlib import Path
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

from oracle import Case, Step, compare, observe, request_fields


class OracleTests(unittest.TestCase):
    def fixture(self, source):
        return Case("probe", {"driver.py": source.encode()}, [Step(["driver.py"])])

    def test_runs_in_independent_fixtures_and_compares_all_observables(self):
        case = self.fixture(
            "from pathlib import Path\nimport sys\n"
            "p = Path('counter'); p.write_text(str(int(p.read_text()) + 1) if p.exists() else '1')\n"
            "print('out'); print('err', file=sys.stderr); sys.exit(7)\n"
        )
        left = observe([sys.executable], case)
        right = observe([sys.executable], case)
        self.assertEqual(left[0]["files"]["counter"], b"1")
        self.assertEqual(left[0]["status"], 7)
        self.assertEqual(compare(case, left, right), [])
        for field, replacement in [
            ("status", 0), ("stdout", b"wrong"), ("stderr", b"wrong"),
            ("files", {}), ("requests", [{"method": "POST", "path": "/wrong"}]),
        ]:
            with self.subTest(field=field):
                broken = copy.deepcopy(right)
                broken[0][field] = replacement
                self.assertTrue(compare(case, left, broken))

    def test_http_capture_compares_fields(self):
        case = self.fixture(
            "import os, urllib.request\n"
            "urllib.request.urlopen(os.environ['SHOWBOAT_REMOTE_URL'], b'uuid=fixed&command=note&markdown=hello%0A').read()\n"
        )
        case.remote_status = 200
        left = observe([sys.executable], case)
        right = observe([sys.executable], case)
        self.assertEqual(left[0]["status"], 0, left[0]["stderr"])
        self.assertEqual(left[0]["requests"][0]["fields"]["markdown"], [b"hello\n"])
        self.assertEqual(compare(case, left, right), [])
        right[0]["requests"][0]["fields"]["markdown"] = [b"changed\n"]
        self.assertTrue(compare(case, left, right))

    def test_remote_binary_values_remain_distinguishable(self):
        first, _ = request_fields("application/x-www-form-urlencoded", b"output=%FF%00")
        second, _ = request_fields("application/x-www-form-urlencoded", b"output=%FE%00")
        self.assertEqual(first["output"], [b"\xff\x00"])
        self.assertNotEqual(first, second)

    def test_multipart_preserves_bytes_and_repeated_fields(self):
        body = (b'--boundary\r\nContent-Disposition: form-data; name="command"\r\n\r\nimage\r\n'
                b'--boundary\r\nContent-Disposition: form-data; name="image"; filename="a.png"\r\n'
                b'Content-Type: application/octet-stream\r\n\r\n\xff\x00\r\n--boundary--\r\n')
        fields, uploads = request_fields("multipart/form-data; boundary=boundary", body)
        self.assertEqual(fields, {"command": [b"image"]})
        self.assertEqual(uploads, [{"name": "image", "filename": "a.png",
                                   "content_type": "application/octet-stream", "body": b"\xff\x00"}])

    def test_normalizes_only_generated_header_and_preserves_legacy_ids(self):
        case = Case("init", {}, [Step(["init", "doc.md", "Title"])], generated_documents=["doc.md"])
        def snapshot(timestamp, doc_id):
            return [{"status": 0, "stdout": b"", "stderr": b"", "requests": [], "files": {
                "doc.md": f"# Title\n\n*{timestamp} by Showboat 0.6.1*\n<!-- showboat-id: {doc_id} -->\n".encode(),
            }}]
        first = snapshot("2026-11-16T11:00:00Z", "11111111-1111-4111-8111-111111111111")
        second = snapshot("2026-11-16T11:00:01Z", "22222222-2222-4222-8222-222222222222")
        self.assertEqual(compare(case, first, second), [])
        case.generated_documents = []
        self.assertTrue(compare(case, first, second))
        case.generated_documents = ["doc.md"]
        second[0]["files"]["doc.md"] = second[0]["files"]["doc.md"].replace(b"4222", b"1222")
        with self.assertRaises(ValueError):
            compare(case, first, second)

    def test_payload_whitespace_and_uuid_literals_are_never_scrubbed(self):
        case = self.fixture("print('hello')\n")
        left = observe([sys.executable], case)
        for value in [b"hello \n", b"11111111-1111-4111-8111-111111111111\n"]:
            right = copy.deepcopy(left)
            right[0]["stdout"] = value
            self.assertTrue(compare(case, left, right))

    def test_timeout_is_an_error_not_a_matching_process_result(self):
        case = self.fixture("import time\ntime.sleep(60)\n")
        with self.assertRaises(TimeoutError):
            observe([sys.executable], case, timeout=0.05)

    def test_successful_case_reclaims_detached_stdio_children(self):
        for enabled in [False, True]:
            with self.subTest(cleanup=enabled), tempfile.TemporaryDirectory(prefix="mdrun-oracle-child-guard-") as directory:
                marker = Path(directory) / "escaped"
                child = f"import time; from pathlib import Path; time.sleep(0.3); Path({str(marker)!r}).touch()"
                case = self.fixture(
                    "import subprocess, sys\n"
                    f"subprocess.Popen([sys.executable, '-c', {child!r}], cwd='/', stdin=subprocess.DEVNULL, "
                    "stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)\n"
                )
                guard = nullcontext() if enabled else patch("oracle.kill_group", lambda _pid: None)
                with guard:
                    self.assertEqual(observe([sys.executable], case)[0]["status"], 0)
                time.sleep(0.5)
                self.assertEqual(marker.exists(), not enabled)


if __name__ == "__main__":
    unittest.main()
