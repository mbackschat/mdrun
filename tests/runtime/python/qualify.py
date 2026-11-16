"""Exercise the experimental Python SDK against an independent bounded wire peer."""

import base64
import contextlib
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time
import unittest


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
    assert 0 < size <= 1048576, size
    return json.loads(exact(size).decode("utf-8"))


def send(connection, frame):
    body = json.dumps(frame, separators=(",", ":")).encode()
    connection.sendall(struct.pack(">I", len(body)) + body)


class Host(contextlib.AbstractContextManager):
    def __init__(self, mode=None):
        self.directory = tempfile.TemporaryDirectory(prefix="mdrun-python-peer-")
        self.listener = socket.socket()
        self.listener.bind(("127.0.0.1", 0))
        self.listener.listen(2)
        self.listener.settimeout(2)
        self.identity = {"protocol": 1, "session_id": "python-test", "artifact_id": "fixture-v1", "token": "private-test-token"}
        self.connections = []
        self.diagnostics = open(Path(self.directory.name) / "diagnostics", "w+b")
        config = Path(self.directory.name) / "worker.json"
        config.write_text(json.dumps({**self.identity, "connect": {"host": "127.0.0.1", "port": self.listener.getsockname()[1]}}))
        config.chmod(0o600)
        environment = dict(os.environ)
        environment["PYTHONDONTWRITEBYTECODE"] = "1"
        if mode:
            environment["MDRUN_PYTHON_FIXTURE"] = mode
        else:
            environment.pop("MDRUN_PYTHON_FIXTURE", None)
        self.process = subprocess.Popen([sys.executable, str(Path(__file__).with_name("fixture_worker.py")), str(config)],
                                        env=environment, stdin=subprocess.DEVNULL,
                                        stdout=self.diagnostics, stderr=self.diagnostics)
        self.watchdog = threading.Timer(10, self.process.kill)
        self.watchdog.start()
        try:
            peers = {}
            for _ in range(2):
                connection, _ = self.listener.accept()
                self.connections.append(connection)
                connection.settimeout(2)
                hello = receive(connection)
                role = hello.pop("role")
                assert hello == {"kind": "hello", **self.identity}, hello
                assert role in ("worker_data", "worker_control") and role not in peers, role
                peers[role] = connection
            self.data, self.control = peers["worker_data"], peers["worker_control"]
        except BaseException:
            self.__exit__(None, None, None)
            raise

    def ready(self):
        try:
            assert receive(self.data) == {"kind": "ready", "protocol": 1, "session_id": "python-test", "artifact_id": "fixture-v1",
                                          "capabilities": ["pull_stdin", "sequenced_output", "release_ack"]}
            return self
        except BaseException:
            self.__exit__(None, None, None)
            raise

    def __exit__(self, *_):
        for connection in self.connections:
            connection.close()
        self.listener.close()
        if self.process.poll() is None:
            self.process.kill()
        self.process.wait(timeout=2)
        self.watchdog.cancel()
        self.watchdog.join(timeout=2)
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
        return self.process.wait(timeout=2)


class PythonProtocol(unittest.TestCase):
    def host(self):
        return Host().ready()

    def assert_fault(self, host, reason=None):
        try:
            frame = receive(host.data)
            self.assertEqual(frame["kind"], "fault")
            if reason:
                self.assertEqual(frame["reason"], reason)
        except (EOFError, ConnectionResetError):
            # Even readiness delivery can still hold the write lock when its
            # peer responds. A terminal fault must never wait for that lock.
            pass
        self.assertEqual(host.process.wait(timeout=2), 70)
        host.diagnostics.seek(0)
        self.assertEqual(host.diagnostics.read(), b"")

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

    def test_refusal_clears_classification_without_invocation(self):
        with self.host() as host:
            self.assertEqual(host.classify("refuse"), {"kind": "refused", "request_id": "one", "reason": "unsupported_command"})
            host.invoke("fail", "two")
            self.assertEqual(receive(host.data), {"kind": "complete", "request_id": "two", "exit_code": 17})
            host.release("two")
            self.assertEqual(host.shutdown(), 0)

    def test_binary_streams_exceed_frame_size_with_fresh_eof_and_bounded_credit(self):
        payload = bytes(range(256)) * 12289
        with self.host() as host:
            for request in ("one", "two"):
                host.invoke("echo", request)
                offset, sequence = 0, 0
                output = bytearray()
                while True:
                    frame = receive(host.data)
                    self.assertEqual(frame["request_id"], request)
                    if frame["kind"] == "stdin_read":
                        self.assertEqual(frame["max_bytes"], 65536)
                        chunk = payload[offset:offset + frame["max_bytes"]]
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
                self.assertEqual(output, payload)
                host.release(request)
            self.assertEqual(host.shutdown(), 0)

    def test_large_single_write_and_merged_sequence(self):
        with self.host() as host:
            host.invoke("large_write")
            output, sequence = bytearray(), 0
            while (frame := receive(host.data))["kind"] == "output":
                self.assertEqual(frame["sequence"], sequence)
                self.assertLessEqual(len(base64.b64decode(frame["base64"])), 65536)
                sequence += 1
                output.extend(base64.b64decode(frame["base64"], validate=True))
            self.assertEqual(output, bytes(range(256)) * 12289)
            self.assertEqual(frame, {"kind": "complete", "request_id": "one", "exit_code": 0})
            host.release()
            host.invoke("binary", "two")
            for sequence, stream, value in [(0, "stdout", b"\x00\xff\x01"), (1, "stderr", b"\xfe\x00"), (2, "stdout", b"\x02")]:
                self.assertEqual(receive(host.data), {"kind": "output", "request_id": "two", "sequence": sequence,
                                                     "stream": stream, "base64": base64.b64encode(value).decode()})
            self.assertEqual(receive(host.data), {"kind": "complete", "request_id": "two", "exit_code": 0})
            host.release("two")
            self.assertEqual(host.shutdown(), 0)

    def test_callback_reset_and_uncaught_failures_are_sticky(self):
        for command, reason in [("throw", "invocation_failed"), ("resetfail", "reset_failed"),
                                ("bad_status", "invalid_exit_code"), ("uncaught", "uncaught_exception"),
                                ("unraisable", "unraisable_exception"),
                                ("invalidate_caught", "application_invalidated")]:
            with self.subTest(command=command), self.host() as host:
                host.invoke(command)
                self.assert_fault(host, reason)

    def test_factory_and_close_failures_terminate(self):
        with Host("initialization_failure") as host:
            self.assert_fault(host, "initialization_failed")
        with Host("close_failure").ready() as host:
            self.assertEqual(host.shutdown(), 70)

    def test_orderly_shutdown_restores_hooks_and_refuses_a_second_lifetime(self):
        with self.host() as host:
            self.assertEqual(host.shutdown(), 0)

    def test_borrowed_noops_and_retained_operations_after_return(self):
        with self.host() as host:
            host.invoke("borrowed_close")
            self.assertEqual(receive(host.data)["base64"], "b2s=")
            self.assertEqual(receive(host.data)["kind"], "complete")
            host.release()
            host.invoke("late_noop", "two")
            self.assertEqual(receive(host.data)["kind"], "complete")
            host.release("two")
            self.assertEqual(host.shutdown(), 0)
        for command in ("late_stdout", "late_stderr", "late_input", "late_invalidate", "late_atexit"):
            with self.subTest(command=command), self.host() as host:
                host.invoke(command)
                self.assertEqual(receive(host.data)["kind"], "complete")
                host.release()
                self.assertEqual(host.shutdown(), 70)

    def test_old_operations_fault_the_current_or_released_reservation(self):
        with self.host() as host:
            host.invoke("retain", "one")
            self.assertEqual(receive(host.data)["kind"], "complete")
            host.release()
            host.invoke("old_write", "two")
            self.assert_fault(host, "late_output")
        for released in (False, True):
            with self.subTest(released=released), self.host() as host:
                gate = Path(host.directory.name) / "late"
                host.invoke("gated_late", "one", str(gate))
                self.assertEqual(receive(host.data)["kind"], "complete")
                if released:
                    host.release()
                gate.touch()
                self.assert_fault(host, "late_output")

    def test_control_loss_breaks_blocked_input_and_output(self):
        for command in ("blocked_input", "blocked_output"):
            with self.subTest(command=command), self.host() as host:
                host.invoke(command)
                self.assertEqual(receive(host.data)["kind"], "stdin_read" if command == "blocked_input" else "output")
                host.control.shutdown(socket.SHUT_RDWR)
                host.control.close()
                self.assertEqual(host.process.wait(timeout=2), 70)

    def test_data_loss_and_active_or_invalid_shutdown_fail_the_worker(self):
        with self.host() as host:
            host.data.shutdown(socket.SHUT_RDWR)
            host.data.close()
            self.assertEqual(host.process.wait(timeout=2), 70)
        with self.host() as host:
            host.invoke("blocked_input")
            self.assertEqual(receive(host.data)["kind"], "stdin_read")
            send(host.control, {"kind": "shutdown"})
            self.assert_fault(host, "shutdown_during_invocation")
        with self.host() as host:
            send(host.control, {"kind": "shutdown", "extra": True})
            self.assert_fault(host, "invalid_control")

    def test_overlapping_io_and_outstanding_credit_prevent_completion(self):
        for command, reason in [("concurrent_input", "concurrent_input"), ("concurrent_output", "concurrent_output"),
                                ("outstanding_input", "outstanding_input")]:
            with self.subTest(command=command), self.host() as host:
                gate = Path(host.directory.name) / "continue"
                host.invoke(command, "one", str(gate))
                self.assertEqual(receive(host.data)["kind"], "output" if command == "concurrent_output" else "stdin_read")
                gate.touch()
                try:
                    frame = receive(host.data)
                    while frame["kind"] == "output":
                        frame = receive(host.data)
                    self.assertEqual(frame["kind"], "fault")
                    self.assertEqual(frame["reason"], reason)
                except (EOFError, ConnectionResetError):
                    # A blocked frame cannot be interrupted by another frame;
                    # socket loss and real process failure remain authoritative.
                    self.assertEqual(command, "concurrent_output")
                self.assertEqual(host.process.wait(timeout=2), 70)

    def test_release_unique_identity_and_acceptance_gate_admission(self):
        for action in ("invoke_without_classify", "early_release", "withheld_release", "reused_identity"):
            with self.subTest(action=action), self.host() as host:
                if action == "invoke_without_classify":
                    send(host.data, {"kind": "invoke", "request_id": "one"})
                elif action == "early_release":
                    self.assertEqual(host.classify("ignore")["kind"], "eligible")
                    send(host.data, {"kind": "release", "request_id": "one"})
                else:
                    host.invoke("fail")
                    self.assertEqual(receive(host.data)["kind"], "complete")
                    if action == "reused_identity":
                        host.release()
                    send(host.data, {"kind": "classify", "request_id": "one" if action == "reused_identity" else "two", "argv": ["ignore"]})
                self.assert_fault(host, "protocol_error")

    def test_strict_frame_and_scalar_validation(self):
        bodies = [
            b'{"kind":"classify","kind":"classify","request_id":"one","argv":[]}',
            b'{"kind":"classify","request_id":"one","argv":[],"extra":true}',
            b'{"kind":"classify","request_id":"one","argv":[1]}',
            b'{"kind":"classify","request_id":"","argv":[]}',
            b'{"kind":"classify","request_id":"one","argv":["\xff"]}',
            b'{"kind":"classify","request_id":"one","argv":["\\ud800"]}',
            b'{"kind":"classify","request_id":"one","argv":[]}{}',
            b'{"kind":"future_message","request_id":"one"}',
            b'{"kind":"classify","request_id":"one","argv":[NaN]}',
        ]
        for body in bodies:
            with self.subTest(body=body), self.host() as host:
                host.data.sendall(struct.pack(">I", len(body)) + body)
                self.assert_fault(host, "protocol_error")
        for size in (0, 1048577):
            with self.subTest(size=size), self.host() as host:
                host.data.sendall(struct.pack(">I", size))
                self.assert_fault(host, "protocol_error")

    def test_parsed_malformed_payload_stays_a_fault_when_shutdown_wins(self):
        frames = [
            ({"kind": "classify", "request_id": "one", "argv": []}, 0),
            ({"kind": "classify", "request_id": "one", "argv": "wrong"}, 70),
            ({"kind": "classify", "request_id": "one", "argv": [True]}, 70),
            ({"kind": "stdin", "request_id": "one", "base64": True}, 70),
            ({"kind": "stdin", "request_id": "one", "base64": "YR=="}, 70),
            ({"kind": "stdin", "request_id": "one", "base64": base64.b64encode(bytes(65537)).decode()}, 70),
        ]
        for frame, expected in frames:
            with self.subTest(frame_kind=frame["kind"], expected=expected), Host("receive_after_shutdown").ready() as host:
                send(host.data, frame)
                parsed = Path(host.directory.name) / "parsed"
                deadline = time.monotonic() + 2
                while not parsed.exists() and time.monotonic() < deadline:
                    time.sleep(.005)
                self.assertTrue(parsed.exists())
                self.assertEqual(host.shutdown(), expected)

    def test_input_requires_credit_identity_and_canonical_base64(self):
        for fields in ({"request_id": "other", "base64": "YQ=="}, {"base64": "YR=="}, {"base64": "YQ"},
                       {"base64": ""}, {"base64": "YWI="}, {"base64": True}):
            with self.subTest(fields=fields), self.host() as host:
                host.invoke("blocked_input")
                self.assertEqual(receive(host.data), {"kind": "stdin_read", "request_id": "one", "max_bytes": 1})
                send(host.data, {"kind": "stdin", "request_id": "one", **fields})
                self.assert_fault(host, "protocol_error")
        with self.host() as host:
            send(host.data, {"kind": "stdin_eof", "request_id": "one"})
            self.assert_fault(host, "protocol_error")


if __name__ == "__main__":
    unittest.main()
