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

    def observation(self, stdout=b"", stderr=b""):
        return [{"status": 0, "stdout": stdout, "stderr": stderr, "requests": [],
                 "files": {}, "directories": []}]

    def test_behavior_suite_separates_parser_policy_and_source_preservation(self):
        from cases import cases
        upstream = {case.name for case in cases("upstream")}
        behavior = {case.name for case in cases("behavior")}
        self.assertEqual(len(upstream), 118)
        self.assertEqual(len(behavior), 75)
        self.assertEqual(upstream - behavior, {name for name in upstream if name.startswith("cli-")} | {
            "init-extra", "note-extras", "exec-extra", "note-literal-option", "note-global-in-text",
            "workdir-repeated", "verify-repeated-output", "verify-output-missing-value",
            "verify-output-before-file", "verify-unknown-extra", "extract-missing-value",
            "extract-repeated", "text-crlf", "text-extra-blanks",
        })
        self.assertTrue({"exec-binary", "exec-nonzero", "verify-copy", "workdir-middle",
                         "remote-workflow-503", "extract-reconstruction"} <= behavior)

    def test_extract_projection_preserves_quoted_program_names_and_rejects_invalid_quoting(self):
        case = Case("extract-projection", {}, [Step(["extract", "doc.md"])])
        left = self.observation(b"showboat init doc.md Example\nshowboat note doc.md 'literal showboat\nshowboat remains quoted'\n")
        right = self.observation(b"mdrun init doc.md Example\nmdrun note doc.md 'literal showboat\nshowboat remains quoted'\n")
        self.assertEqual(compare(case, left, right), [])
        self.assertTrue(compare(case, left, right, suite="upstream"))
        separated = self.observation(b"mdrun init -- doc.md Example\nmdrun note -- doc.md 'literal showboat\nshowboat remains quoted'\n")
        self.assertEqual(compare(case, left, separated), [])
        literal = self.observation(b"showboat note doc.md --\n")
        self.assertEqual(compare(case, literal, self.observation(b"mdrun note -- doc.md --\n")), [])
        self.assertTrue(compare(case, literal, self.observation(b"mdrun note -- doc.md different\n")))
        with self.assertRaises(ValueError):
            compare(case, literal, self.observation(b"showboat note -- doc.md --\n"))
        changed = copy.deepcopy(right)
        changed[0]["stdout"] = changed[0]["stdout"].replace(b"literal showboat", b"literal mdrun")
        self.assertTrue(compare(case, left, changed))
        changed[0]["stdout"] = b"mdrun note doc.md 'unclosed\n"
        with self.assertRaises(ValueError):
            compare(case, left, changed)
        changed[0]["stdout"] = b"mdrun pop doc.md\n"
        with self.assertRaises(ValueError):
            compare(case, left, changed)
        changed[0]["stdout"] = b"printf '%s\\n' '[\"aW5pdA==\",\"ZG9jLm1k\",\"RXhhbXBsZQ==\"]'\n"
        with self.assertRaises(ValueError):
            compare(case, left, changed)
        case.steps = [Step(["exec", "doc.md", "sh", ":"])]
        self.assertTrue(compare(case, left, right))

    def test_remote_warning_projection_requires_the_declared_status_and_exact_warning(self):
        case = Case("remote-projection", {}, [Step(["note", "doc.md", "text"])], remote_status=503)
        left = self.observation(stderr=b"showboat: remote POST warning: server returned 503\n")
        right = self.observation(stderr=b"mdrun: remote POST warning: server returned 503\n")
        self.assertEqual(compare(case, left, right), [])
        self.assertTrue(compare(case, left, right, suite="upstream"))
        for warning in [b"mdrun: remote POST warning: server returned 504\n",
                        b"mdrun: remote POST warning: connection refused\n",
                        b"mdrun: remote POST warning: server returned 503\nextra\n"]:
            self.assertTrue(compare(case, left, self.observation(stderr=warning)))
        case.remote_status = None
        self.assertTrue(compare(case, left, right))

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
