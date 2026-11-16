"""Compile and exercise the JVM SDK against an independent bounded wire peer."""

import base64
import contextlib
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import tempfile
import unittest

from prepare import prepare as prepare_fixture

def receive(connection):
    def exact(size):
        result = bytearray()
        while len(result) < size:
            part = connection.recv(size - len(result))
            if not part:
                raise EOFError("worker closed the connection")
            result.extend(part)
        return result

    size = struct.unpack(">I", exact(4))[0]
    if not 0 < size <= 1048576:
        raise AssertionError(f"invalid frame size {size}")
    return json.loads(exact(size).decode("utf-8"))


def send(connection, frame):
    data = json.dumps(frame, separators=(",", ":")).encode()
    connection.sendall(struct.pack(">I", len(data)) + data)


class Host(contextlib.AbstractContextManager):
    def __init__(self, java, classpath):
        self.directory = tempfile.TemporaryDirectory(prefix="mdrun-jvm-peer-")
        self.listener = socket.socket()
        self.listener.bind(("127.0.0.1", 0))
        self.listener.listen(2)
        self.listener.settimeout(5)
        self.identity = {"protocol": 1, "session_id": "jvm-test", "artifact_id": "fixture-v1", "token": "private-test-token"}
        self.peers = {}
        self.connections = []
        self.diagnostics = open(Path(self.directory.name) / "diagnostics", "w+b")
        config = Path(self.directory.name) / "worker.json"
        config.write_text(json.dumps({**self.identity, "connect": {"host": "127.0.0.1", "port": self.listener.getsockname()[1]}}))
        config.chmod(0o600)
        self.process = subprocess.Popen([java, "-cp", classpath, "io.github.mbackschat.mdrun.fixture.Fixture", str(config)],
                                        stdin=subprocess.DEVNULL, stdout=self.diagnostics, stderr=self.diagnostics)
        try:
            for _ in range(2):
                connection, _ = self.listener.accept()
                self.connections.append(connection)
                connection.settimeout(3)
                hello = receive(connection)
                role = hello.pop("role")
                assert hello == {"kind": "hello", **self.identity}, hello
                assert role in ("worker_data", "worker_control") and role not in self.peers, role
                self.peers[role] = connection
            self.data = self.peers["worker_data"]
            self.control = self.peers["worker_control"]
            assert receive(self.data) == {"kind": "ready", "protocol": 1, "session_id": "jvm-test", "artifact_id": "fixture-v1",
                                          "capabilities": ["pull_stdin", "sequenced_output", "release_ack"]}
        except BaseException:
            self.__exit__(None, None, None)
            raise

    def __exit__(self, *_):
        for connection in self.connections:
            connection.close()
        self.listener.close()
        if self.process.poll() is None:
            self.process.kill()
        self.process.wait(timeout=3)
        self.diagnostics.close()
        self.directory.cleanup()

    def classify(self, command, request="one", *arguments):
        send(self.data, {"kind": "classify", "request_id": request, "argv": [command, *arguments]})
        return receive(self.data)

    def invoke(self, command, request="one", *arguments):
        assert self.classify(command, request, *arguments) == {"kind": "eligible", "request_id": request}
        send(self.data, {"kind": "invoke", "request_id": request})
        assert receive(self.data) == {"kind": "accepted", "request_id": request}

    def release(self, request="one"):
        send(self.data, {"kind": "release", "request_id": request})
        assert receive(self.data) == {"kind": "released", "request_id": request}

    def shutdown(self):
        send(self.control, {"kind": "shutdown"})
        return self.process.wait(timeout=3)


class JvmProtocol(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if prepared := os.environ.get("MDRUN_JVM_FIXTURE"):
            manifest = json.loads(Path(prepared).read_text())
        else:
            cls.output = tempfile.TemporaryDirectory(prefix="mdrun-jvm-build-")
            cls.addClassCleanup(cls.output.cleanup)
            manifest = prepare_fixture(Path(os.environ["MDRUN_JAVA_HOME"]),
                                       Path(os.environ["MDRUN_JACKSON_CORE"]), Path(cls.output.name))
        argv = manifest["binding"]["worker_argv"]
        cls.java, cls.classpath = argv[0], argv[2]

    def host(self):
        return Host(self.java, self.classpath)

    def test_ignores_input_and_reuses_after_nonzero_status(self):
        with self.host() as host:
            for request, command, status in [("one", "ignore", 0), ("two", "fail", 17), ("three", "ignore", 0)]:
                host.invoke(command, request)
                if command == "ignore":
                    self.assertEqual(receive(host.data), {"kind": "output", "request_id": request, "sequence": 0,
                                                         "stream": "stdout", "base64": "b2s="})
                self.assertEqual(receive(host.data), {"kind": "complete", "request_id": request, "exit_code": status})
                host.release(request)
            self.assertEqual(host.shutdown(), 0)

    def test_refusal_has_no_invocation_and_clears_classification(self):
        with self.host() as host:
            self.assertEqual(host.classify("refuse"), {"kind": "refused", "request_id": "one", "reason": "unsupported_command"})
            host.invoke("fail", "two")
            self.assertEqual(receive(host.data), {"kind": "complete", "request_id": "two", "exit_code": 17})
            host.release("two")
            self.assertEqual(host.shutdown(), 0)

    def assert_invalidation_fault(self, host, request):
        self.assertEqual(receive(host.data), {"kind": "fault", "request_id": request,
                                            "reason": "application_invalidated"})
        self.assertEqual(host.process.wait(timeout=3), 70)
        with self.assertRaises(EOFError):
            receive(host.data)
        host.diagnostics.seek(0)
        self.assertEqual(host.diagnostics.read(), b"")

    def test_invalidation_is_sticky_even_when_caught_and_repeated(self):
        for command in ("invalidate_caught", "invalidate_twice"):
            with self.subTest(command=command), self.host() as host:
                host.invoke(command)
                self.assert_invalidation_fault(host, "one")

    def test_invalidation_through_an_old_request_faults_the_later_invocation(self):
        with self.host() as host:
            host.invoke("retain_invalidation", "first")
            self.assertEqual(receive(host.data), {"kind": "complete", "request_id": "first", "exit_code": 0})
            host.release("first")
            host.invoke("invalidate_previous", "second")
            self.assert_invalidation_fault(host, "second")

    def test_invalidation_after_completion_or_release_faults_the_worker(self):
        for released in (False, True):
            with self.subTest(released=released), self.host() as host:
                gate = Path(host.directory.name) / "invalidate-now"
                host.invoke("invalidate_gated", "one", str(gate))
                self.assertEqual(receive(host.data), {"kind": "complete", "request_id": "one", "exit_code": 0})
                if released:
                    host.release()
                gate.touch()
                self.assert_invalidation_fault(host, None if released else "one")

    def test_invalidation_after_orderly_return_halts_even_when_caught_or_shutdown_hooks_stall(self):
        for command in ("late_invalidate", "late_invalidate_hook"):
            with self.subTest(command=command), self.host() as host:
                host.invoke(command)
                self.assertEqual(receive(host.data), {"kind": "complete", "request_id": "one", "exit_code": 0})
                host.release()
                self.assertEqual(host.shutdown(), 70)
                with self.assertRaises(EOFError):
                    receive(host.data)
                host.diagnostics.seek(0)
                self.assertEqual(host.diagnostics.read(), b"")

    def test_binary_streams_exceed_frame_size_and_keep_fresh_eof(self):
        data = bytes(range(256)) * 12289
        with self.host() as host:
            for request in ("one", "two"):
                host.invoke("echo", request)
                offset, sequence = 0, 0
                output = bytearray()
                while True:
                    frame = receive(host.data)
                    self.assertEqual(frame["request_id"], request)
                    if frame["kind"] == "stdin_read":
                        self.assertTrue(1 <= frame["max_bytes"] <= 65536)
                        chunk = data[offset:offset + frame["max_bytes"]]
                        offset += len(chunk)
                        send(host.data, {"kind": "stdin", "request_id": request, "base64": base64.b64encode(chunk).decode()}
                             if chunk else {"kind": "stdin_eof", "request_id": request})
                    elif frame["kind"] == "output":
                        self.assertEqual(frame["sequence"], sequence)
                        self.assertEqual(frame["stream"], "stdout")
                        sequence += 1
                        output.extend(base64.b64decode(frame["base64"], validate=True))
                    else:
                        self.assertEqual(frame, {"kind": "complete", "request_id": request, "exit_code": 0})
                        break
                self.assertEqual(output, data)
                host.release(request)
            self.assertEqual(host.shutdown(), 0)

    def test_caught_late_io_after_shutdown_fails_the_process(self):
        for command in ("late_stdout", "late_stderr", "late_input", "late_hook"):
            with self.subTest(command=command), self.host() as host:
                host.invoke(command)
                self.assertEqual(receive(host.data), {"kind": "complete", "request_id": "one", "exit_code": 0})
                host.release()
                self.assertEqual(host.shutdown(), 70)

    def test_borrowed_close_flush_and_zero_length_operations_do_not_consume_credit(self):
        with self.host() as host:
            host.invoke("borrowed_close")
            self.assertEqual(receive(host.data), {"kind": "output", "request_id": "one", "sequence": 0,
                                                 "stream": "stdout", "base64": "b2s="})
            self.assertEqual(receive(host.data), {"kind": "complete", "request_id": "one", "exit_code": 0})
            host.release()
            host.invoke("late_noop", "two")
            self.assertEqual(receive(host.data), {"kind": "complete", "request_id": "two", "exit_code": 0})
            host.release("two")
            self.assertEqual(host.shutdown(), 0)

    def test_exceptions_and_failed_reset_poison_reuse(self):
        for command in ("throw", "resetfail"):
            with self.subTest(command=command), self.host() as host:
                host.invoke(command)
                self.assertEqual(receive(host.data)["kind"], "fault")
                self.assertEqual(host.process.wait(timeout=3), 70)

    def test_control_loss_interrupts_blocked_input_and_output(self):
        for command in ("blocked_input", "blocked_output"):
            with self.subTest(command=command), self.host() as host:
                host.invoke(command)
                if command == "blocked_input":
                    self.assertEqual(receive(host.data)["kind"], "stdin_read")
                host.control.shutdown(socket.SHUT_RDWR)
                host.control.close()
                self.assertEqual(host.process.wait(timeout=3), 70)

    def test_stdout_and_stderr_share_one_binary_sequence(self):
        with self.host() as host:
            host.invoke("binary")
            for sequence, stream, data in [(0, "stdout", b"\x00\xff\x01"), (1, "stderr", b"\xfe\x00"),
                                           (2, "stdout", b"\x02")]:
                self.assertEqual(receive(host.data), {"kind": "output", "request_id": "one", "sequence": sequence,
                                                     "stream": stream, "base64": base64.b64encode(data).decode()})
            self.assertEqual(receive(host.data), {"kind": "complete", "request_id": "one", "exit_code": 0})
            host.release()
            self.assertEqual(host.shutdown(), 0)

    def test_malformed_frames_fail_before_application_admission(self):
        frames = {
            "duplicate_key": b'{"kind":"classify","kind":"classify","request_id":"one","argv":[]}',
            "unknown_field": b'{"kind":"classify","request_id":"one","argv":[],"extra":true}',
            "wrong_argv_type": b'{"kind":"classify","request_id":"one","argv":[1]}',
            "empty_identity": b'{"kind":"classify","request_id":"","argv":[]}',
            "invalid_utf8": b'{"kind":"classify","request_id":"one","argv":["\xff"]}',
            "unpaired_surrogate": b'{"kind":"classify","request_id":"one","argv":["\\ud800"]}',
            "utf16": '{"kind":"classify","request_id":"one","argv":[]}'.encode("utf-16"),
            "trailing_json": b'{"kind":"classify","request_id":"one","argv":[]}{}',
            "unknown_kind": b'{"kind":"future_message","request_id":"one"}',
        }
        for name, data in frames.items():
            with self.subTest(case=name), self.host() as host:
                host.data.sendall(struct.pack(">I", len(data)) + data)
                self.assertEqual(receive(host.data)["kind"], "fault")
                self.assertEqual(host.process.wait(timeout=3), 70)
        for size in (0, 1048577):
            with self.subTest(size=size), self.host() as host:
                host.data.sendall(struct.pack(">I", size))
                self.assertEqual(receive(host.data)["kind"], "fault")
                self.assertEqual(host.process.wait(timeout=3), 70)

    def test_input_requires_matching_credit_and_canonical_base64(self):
        for name, fields in [
            ("wrong_request", {"request_id": "another", "base64": "YQ=="}),
            ("noncanonical", {"base64": "YR=="}),
            ("unpadded", {"base64": "YQ"}),
            ("empty", {"base64": ""}),
            ("over_credit", {"base64": base64.b64encode(b"ab").decode()}),
        ]:
            with self.subTest(case=name), self.host() as host:
                host.invoke("blocked_input")
                self.assertEqual(receive(host.data), {"kind": "stdin_read", "request_id": "one", "max_bytes": 1})
                send(host.data, {"kind": "stdin", "request_id": "one", **fields})
                self.assertEqual(receive(host.data)["kind"], "fault")
                self.assertEqual(host.process.wait(timeout=3), 70)
        with self.host() as host:
            send(host.data, {"kind": "stdin_eof", "request_id": "one"})
            self.assertEqual(receive(host.data)["kind"], "fault")
            self.assertEqual(host.process.wait(timeout=3), 70)

    def test_release_and_unique_identity_gate_the_next_request(self):
        for action in ("invoke_without_classify", "early_release", "withheld_release", "reused_identity"):
            with self.subTest(action=action), self.host() as host:
                if action == "early_release":
                    self.assertEqual(host.classify("ignore")["kind"], "eligible")
                    send(host.data, {"kind": "release", "request_id": "one"})
                elif action == "invoke_without_classify":
                    send(host.data, {"kind": "invoke", "request_id": "one"})
                else:
                    host.invoke("fail")
                    self.assertEqual(receive(host.data), {"kind": "complete", "request_id": "one", "exit_code": 17})
                    if action == "reused_identity":
                        host.release()
                    send(host.data, {"kind": "classify", "request_id": "one" if action == "reused_identity" else "two",
                                     "argv": ["ignore"]})
                self.assertEqual(receive(host.data)["kind"], "fault")
                self.assertEqual(host.process.wait(timeout=3), 70)


if __name__ == "__main__":
    unittest.main()
