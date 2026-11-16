import hashlib
import importlib.util
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import os
import subprocess
import sys
import tempfile
import threading
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


sdk = load("wheel_builder", ROOT / "packaging/sdk.py")
wheel_consumer = load("wheel_consumer", ROOT / "packaging/qualify_python_wheel.py")


class PythonWheelConsumptionTests(unittest.TestCase):
    def test_requirement_pins_exact_release_url_version_and_sha256_without_an_index(self):
        requirement = wheel_consumer.requirement("0.1.0", "a" * 64)
        self.assertEqual(requirement, "mdrun-sdk @ https://github.com/mbackschat/mdrun/releases/download/v0.1.0/"
                         "mdrun_sdk-0.1.0-py3-none-any.whl --hash=sha256:" + "a" * 64 + "\n")
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "mdrun_sdk-0.1.0-py3-none-any.whl"
            self.assertEqual(wheel_consumer.requirement("0.1.0", "b" * 64, wheel=path),
                             "mdrun-sdk @ " + path.as_uri() + " --hash=sha256:" + "b" * 64 + "\n")
        for version, digest in (("../escape", "a" * 64), ("0.1.0", "wrong"), ("0.1.0", "A" * 64)):
            with self.subTest(version=version, digest=digest), self.assertRaises(ValueError):
                wheel_consumer.requirement(version, digest)

    def test_installation_uses_uv_pip_hash_checking_in_an_explicit_isolated_venv(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            wheel = root / "mdrun_sdk-0.1.0-py3-none-any.whl"
            wheel.write_bytes(b"opaque wheel bytes")
            with mock.patch.object(wheel_consumer.shared, "run", return_value=b"") as run:
                wheel_consumer.install("0.1.0", "a" * 64, "/python", "/uv", root / "app", wheel=wheel)
            commands = [call.args[0] for call in run.call_args_list]
            self.assertEqual(commands[0][:2], ["/uv", "venv"])
            self.assertIn("--no-python-downloads", commands[0])
            self.assertEqual(commands[1][:3], ["/uv", "pip", "install"])
            for argument in ("--require-hashes", "--no-index", "--no-deps", "--no-config", "--no-cache"):
                self.assertIn(argument, commands[1])
            self.assertIn(str(root / "app/.venv/bin/python"), commands[1])
            self.assertNotIn("--system", commands[1])
            self.assertIn("--hash=sha256:" + "a" * 64, (root / "app/requirements.txt").read_text())

    @unittest.skipUnless(os.environ.get("MDRUN_UV"), "set MDRUN_UV to qualify a real uv installer")
    def test_real_uv_installs_local_wheel_and_rejects_wrong_hash(self):
        with tempfile.TemporaryDirectory(prefix="mdrun-wheel-consumer-test-") as temporary:
            root = Path(temporary)
            wheel = sdk.wheel(ROOT, root / "dist")
            digest = hashlib.sha256(wheel.read_bytes()).hexdigest()
            interpreter = wheel_consumer.install("0.1.0", digest, sys.executable, os.environ["MDRUN_UV"],
                                                 root / "accepted", wheel=wheel)
            observed = subprocess.check_output([str(interpreter), "-I", "-c",
                "import mdrun_sdk; from importlib.metadata import version; print(version('mdrun-sdk'))"], cwd=root)
            self.assertEqual(observed, b"0.1.0\n")
            with self.assertRaises(subprocess.CalledProcessError):
                wheel_consumer.install("0.1.0", "0" * 64, sys.executable, os.environ["MDRUN_UV"],
                                       root / "refused", wheel=wheel)
            observed = subprocess.check_output([str(root / "refused/.venv/bin/python"), "-I", "-c",
                "from importlib.util import find_spec; print(find_spec('mdrun_sdk'))"], cwd=root)
            self.assertEqual(observed, b"None\n")

    @unittest.skipUnless(os.environ.get("MDRUN_UV"), "set MDRUN_UV to qualify a real uv URL install")
    def test_real_uv_installs_hash_pinned_release_url_shape_and_rejects_wrong_hash(self):
        with tempfile.TemporaryDirectory(prefix="mdrun-wheel-url-test-") as temporary:
            root = Path(temporary)
            wheel = sdk.wheel(ROOT, root / "dist")
            payload = wheel.read_bytes()
            digest = hashlib.sha256(payload).hexdigest()
            expected_path = "/mbackschat/mdrun/releases/download/v0.1.0/" + wheel.name
            observed = []

            class ReleaseAsset(BaseHTTPRequestHandler):
                def do_HEAD(self):
                    self.serve_asset(False)

                def do_GET(self):
                    self.serve_asset(True)

                def serve_asset(self, include_body):
                    observed.append(self.path)
                    if self.path != expected_path:
                        self.send_error(404)
                        return
                    self.send_response(200)
                    self.send_header("Content-Length", str(len(payload)))
                    self.end_headers()
                    if include_body:
                        self.wfile.write(payload)

                def log_message(self, format, *args):
                    pass

            server = ThreadingHTTPServer(("127.0.0.1", 0), ReleaseAsset)
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            try:
                origin = f"http://127.0.0.1:{server.server_port}/mbackschat/mdrun/releases/download"
                with mock.patch.object(wheel_consumer, "RELEASE_ROOT", origin):
                    interpreter = wheel_consumer.install("0.1.0", digest, sys.executable, os.environ["MDRUN_UV"],
                                                         root / "accepted-url")
                    version = subprocess.check_output([str(interpreter), "-I", "-c",
                        "import mdrun_sdk; from importlib.metadata import version; print(version('mdrun-sdk'))"], cwd=root)
                    self.assertEqual(version, b"0.1.0\n")
                    with self.assertRaises(subprocess.CalledProcessError):
                        wheel_consumer.install("0.1.0", "0" * 64, sys.executable, os.environ["MDRUN_UV"],
                                               root / "refused-url")
                self.assertGreaterEqual(len(observed), 2)
                self.assertEqual(set(observed), {expected_path})
            finally:
                server.shutdown()
                server.server_close()
                thread.join(timeout=5)
                self.assertFalse(thread.is_alive())


if __name__ == "__main__":
    unittest.main()
