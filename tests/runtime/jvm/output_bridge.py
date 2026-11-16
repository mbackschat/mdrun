"""Qualify the application Appendable helper against independent JDK cold output."""

import base64
import contextlib
import hashlib
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import unittest

from prepare import prepare as prepare_fixture
from qualify import receive, send


ROOT = Path(__file__).resolve().parents[3]
HELPER = ROOT / "sdk/jvm/examples/src/main/java/io/github/mbackschat/mdrun/example/InvocationOutput.java"
SOURCES = Path(__file__).with_suffix("")
PACKAGE = "io.github.mbackschat.mdrun.bridgefixture."


def digest(path):
    value = hashlib.sha256()
    with path.open("rb") as source:
        while chunk := source.read(65536):
            value.update(chunk)
    return value.hexdigest()


class Peer(contextlib.AbstractContextManager):
    def __init__(self, java, classpath, options):
        self.directory = tempfile.TemporaryDirectory(prefix="mdrun-output-peer-")
        self.root = Path(self.directory.name)
        self.listener = socket.socket()
        self.connections = []
        self.diagnostics = (self.root / "diagnostics").open("w+b")
        self.process = None
        try:
            self.listener.bind(("127.0.0.1", 0))
            self.listener.listen(2)
            self.listener.settimeout(5)
            identity = {"protocol": 1, "session_id": "output-bridge", "artifact_id": "test-artifact", "token": "private-test-token"}
            config = self.root / "worker.json"
            config.write_text(json.dumps({**identity, "connect": {"host": "127.0.0.1", "port": self.listener.getsockname()[1]}}))
            config.chmod(0o600)
            self.process = subprocess.Popen([str(java), *options, "-cp", classpath, PACKAGE + "BridgeWorker", str(config)],
                                            cwd=self.root, stdin=subprocess.DEVNULL, stdout=self.diagnostics, stderr=self.diagnostics)
            peers = {}
            for _ in range(2):
                connection, _ = self.listener.accept()
                self.connections.append(connection)
                connection.settimeout(3)
                hello = receive(connection)
                role = hello.pop("role")
                assert hello == {"kind": "hello", **identity}
                assert role in ("worker_data", "worker_control") and role not in peers
                peers[role] = connection
            self.data, self.control = peers["worker_data"], peers["worker_control"]
            assert receive(self.data) == {"kind": "ready", "protocol": 1, "session_id": "output-bridge", "artifact_id": "test-artifact",
                                          "capabilities": ["pull_stdin", "sequenced_output", "release_ack"]}
        except BaseException:
            self.__exit__(None, None, None)
            raise

    def __exit__(self, *_):
        for connection in self.connections:
            connection.close()
        self.listener.close()
        if self.process is not None:
            if self.process.poll() is None:
                self.process.kill()
            self.process.wait(timeout=3)
        self.diagnostics.close()
        self.directory.cleanup()

    def invoke(self, command, request="one", *arguments):
        send(self.data, {"kind": "classify", "request_id": request, "argv": [command, *arguments]})
        assert receive(self.data) == {"kind": "eligible", "request_id": request}
        send(self.data, {"kind": "invoke", "request_id": request})
        assert receive(self.data) == {"kind": "accepted", "request_id": request}

    def result(self, request="one"):
        paths = {stream: self.root / f"{request}.{stream}" for stream in ("stdout", "stderr")}
        with paths["stdout"].open("wb") as stdout, paths["stderr"].open("wb") as stderr:
            sequence = 0
            while True:
                frame = receive(self.data)
                if frame["kind"] != "output":
                    assert frame["kind"] in ("complete", "fault"), frame
                    return frame, paths
                assert set(frame) == {"kind", "request_id", "sequence", "stream", "base64"}, frame
                assert frame["request_id"] == request and frame["sequence"] == sequence, frame
                assert frame["stream"] in paths, frame
                chunk = base64.b64decode(frame["base64"], validate=True)
                assert 0 < len(chunk) <= 65536
                (stdout if frame["stream"] == "stdout" else stderr).write(chunk)
                sequence += 1

    def release(self, request="one"):
        send(self.data, {"kind": "release", "request_id": request})
        assert receive(self.data) == {"kind": "released", "request_id": request}

    def shutdown(self):
        send(self.control, {"kind": "shutdown"})
        return self.process.wait(timeout=3)


class OutputBridge(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temporary = tempfile.TemporaryDirectory(prefix="mdrun-output-build-")
        cls.addClassCleanup(cls.temporary.cleanup)
        cls.root = Path(cls.temporary.name)
        if prepared := os.environ.get("MDRUN_JVM_FIXTURE"):
            manifest = json.loads(Path(prepared).read_text())
        else:
            manifest = prepare_fixture(Path(os.environ["MDRUN_JAVA_HOME"]), Path(os.environ["MDRUN_JACKSON_CORE"]), cls.root / "sdk")
        cls.java = Path(manifest["binding"]["worker_argv"][0])
        jars = [path for path in manifest["binding"]["worker_argv"][2].split(os.pathsep) if Path(path).name != "worker.jar"]
        cls.cold, cls.worker = cls.root / "cold", cls.root / "worker"
        cls.cold.mkdir(); cls.worker.mkdir()
        compile_options = [str(cls.java.with_name("javac")), "--release", "21", "-proc:none", "-encoding", "UTF-8", "-Xlint:all", "-Werror"]
        subprocess.run([*compile_options, "-d", str(cls.cold), str(SOURCES / "Scenario.java"), str(SOURCES / "ColdMain.java")], check=True, timeout=60)
        sources = [SOURCES / "Scenario.java", SOURCES / "BridgeWorker.java"]
        if HELPER.exists():
            sources.append(HELPER)
        subprocess.run([*compile_options, "-cp", os.pathsep.join(jars), "-d", str(cls.worker), *map(str, sources)], check=True, timeout=60)
        cls.classpath = os.pathsep.join([str(cls.worker), *jars])

    def peer(self, out="UTF-8", err="UTF-8"):
        return Peer(self.java, self.classpath, self.options(out, err))

    @staticmethod
    def options(out, err):
        return ["-Dfile.encoding=US-ASCII", "-Dstdout.encoding=" + out, "-Dstderr.encoding=" + err]

    def cold_result(self, name, out="UTF-8", err="UTF-8"):
        stdout, stderr = self.root / "cold.stdout", self.root / "cold.stderr"
        with stdout.open("wb") as output, stderr.open("wb") as error:
            status = subprocess.run([str(self.java), *self.options(out, err), "-cp", str(self.cold), PACKAGE + "ColdMain", name],
                                    cwd=self.root, stdin=subprocess.DEVNULL, stdout=output, stderr=error, timeout=10).returncode
        return status, {"stdout": stdout, "stderr": stderr}

    def assert_cold_parity(self, status, paths, command, out="UTF-8", err="UTF-8"):
        cold_status, cold_paths = self.cold_result(command, out, err)
        self.assertEqual(status, cold_status)
        for stream in paths:
            self.assertEqual(paths[stream].stat().st_size, cold_paths[stream].stat().st_size, stream)
            self.assertEqual(digest(paths[stream]), digest(cold_paths[stream]), stream)

    def test_raw_cold_parity_preserves_charsets_surrogates_ranges_and_nonzero_reuse(self):
        for out, err in [("UTF-8", "UTF-8"), ("US-ASCII", "ISO-8859-1"), ("UTF-16", "UTF-8")]:
            with self.subTest(stdout=out, stderr=err), self.peer(out, err) as host:
                for index, command in enumerate(("operations", "orphan", "low", "status17")):
                    request = str(index)
                    host.invoke(command, request)
                    frame, paths = host.result(request)
                    self.assertEqual(frame, {"kind": "complete", "request_id": request, "exit_code": 17 if command == "status17" else 0})
                    self.assert_cold_parity(frame["exit_code"], paths, command, out, err)
                    host.release(request)
                self.assertEqual(host.shutdown(), 0)

    def test_large_virtual_sequence_uses_bounded_character_slices(self):
        with self.peer() as host:
            host.invoke("large")
            frame, paths = host.result()
            self.assertEqual(frame, {"kind": "complete", "request_id": "one", "exit_code": 0})
            self.assert_cold_parity(0, paths, "large")
            self.assertGreater(paths["stdout"].stat().st_size, 1024 * 1024)
            host.release()
            self.assertEqual(host.shutdown(), 0)

    def test_closed_nonempty_encoder_operation_invalidates_even_when_caught(self):
        with self.peer() as host:
            host.invoke("closed_high")
            frame, paths = host.result()
            self.assertEqual(frame, {"kind": "fault", "request_id": "one", "reason": "application_invalidated"})
            self.assertTrue(all(path.stat().st_size == 0 for path in paths.values()))
            self.assertEqual(host.process.wait(timeout=3), 70)

    def test_empty_cleanup_and_invalid_ranges_remain_harmless_after_close(self):
        with self.peer() as host:
            host.invoke("empty_closed")
            frame, paths = host.result()
            self.assertEqual(frame, {"kind": "complete", "request_id": "one", "exit_code": 0})
            self.assertTrue(all(path.stat().st_size == 0 for path in paths.values()))
            host.release()
            self.assertEqual(host.shutdown(), 0)

    def test_retained_append_from_previous_request_faults_later_call(self):
        with self.peer() as host:
            host.invoke("capture", "first")
            self.assertEqual(host.result("first")[0], {"kind": "complete", "request_id": "first", "exit_code": 0})
            host.release("first")
            host.invoke("append_previous", "second")
            frame, paths = host.result("second")
            self.assertEqual(frame, {"kind": "fault", "request_id": "second", "reason": "application_invalidated"})
            self.assertTrue(all(path.stat().st_size == 0 for path in paths.values()))
            self.assertEqual(host.process.wait(timeout=3), 70)

    def test_retained_zero_byte_encoding_after_return_and_stalled_shutdown_hook_fails(self):
        for command in ("late_character", "late_sequence", "late_range", "late_hook"):
            with self.subTest(command=command), self.peer() as host:
                host.invoke(command)
                self.assertEqual(host.result()[0], {"kind": "complete", "request_id": "one", "exit_code": 0})
                host.release()
                self.assertEqual(host.shutdown(), 70)

    def test_observed_sequence_mutation_poison_cannot_be_caught_into_success(self):
        with self.peer() as host:
            host.invoke("mutating_sequence")
            frame, paths = host.result()
            self.assertEqual(frame, {"kind": "fault", "request_id": "one", "reason": "application_invalidated"})
            emitted = paths["stdout"].stat().st_size
            self.assertLessEqual(emitted, 5000)
            self.assertEqual(paths["stdout"].read_bytes(), b"m" * emitted)
            self.assertEqual(paths["stderr"].stat().st_size, 0)
            self.assertEqual(host.process.wait(timeout=3), 70)

    def test_other_stream_and_close_wait_for_accepted_append(self):
        for command in ("concurrent", "concurrent_streams"):
            with self.subTest(operation=command), self.peer() as host:
                gate = host.root / "continue"
                host.invoke(command, "one", str(gate))
                deadline = time.monotonic() + 3
                while not Path(str(gate) + ".ready").exists():
                    self.assertLess(time.monotonic(), deadline, "operations did not reach their gates")
                    time.sleep(0.01)
                time.sleep(0.1)
                self.assertFalse(Path(str(gate) + ".finished").exists(), "operation passed an unfinished accepted append")
                gate.touch()
                frame, paths = host.result()
                self.assertEqual(frame, {"kind": "complete", "request_id": "one", "exit_code": 0})
                self.assert_cold_parity(0, paths, command)
                self.assertTrue(Path(str(gate) + ".finished").exists())
                host.release()
                self.assertEqual(host.shutdown(), 0)


if __name__ == "__main__":
    unittest.main()
