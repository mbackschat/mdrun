"""Cooperative and deliberately invalid adapters for independent SDK qualification."""

import argparse
import atexit
import gc
import json
import os
from pathlib import Path
import sys
import threading
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "sdk" / "python"))
sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "sdk" / "python" / "examples"))

from mdrun_sdk import Application, Eligible, Refused, WorkerError, run_worker
import mdrun_sdk
from owned_thread import OwnedThreadApplication


retained = None
late_operation = None


def attempt_late():
    try:
        if late_operation == "late_input":
            retained.stdin.read(1)
        elif late_operation == "late_stderr":
            retained.stderr.write(b"x")
        elif late_operation == "late_invalidate":
            retained.invalidate()
        elif late_operation == "late_noop":
            retained.stdin.close()
            retained.stdout.close()
            retained.stderr.flush()
            assert retained.stdin.read(0) == b""
            assert retained.stdout.write(b"") == 0
        else:
            retained.stdout.write(b"x")
    except BaseException:
        pass


class Fixture(Application):
    def __init__(self):
        self.baseline = frozenset(threading.enumerate())
        assert {thread.name for thread in self.baseline if thread.name.startswith("mdrun-")} == {
            "mdrun-data", "mdrun-control", "mdrun-execution"
        }
        self.execution = threading.current_thread()
        self.owned = OwnedThreadApplication()
        self.owned_markers = []
        if os.environ.get("MDRUN_PYTHON_FIXTURE") == "initialization_failure":
            raise RuntimeError("private initialization detail")

    def classify(self, argv):
        assert threading.current_thread() is self.execution
        assert isinstance(argv, tuple)
        if argv and argv[0] == "refuse":
            return Refused("unsupported_command")
        return Eligible()

    def invoke(self, request):
        global retained, late_operation
        assert threading.current_thread() is self.execution
        assert isinstance(request.argv, tuple)
        command = request.argv[0]
        if command == "ignore":
            request.stdout.write(b"ok")
        elif command == "fail":
            return 17
        elif command == "echo":
            while chunk := request.stdin.read(100_000):
                request.stdout.write(chunk)
        elif command == "read_file":
            with Path(request.argv[1]).open("rb") as source:
                while chunk := source.read(32768):
                    request.stdout.write(chunk)
        elif command == "binary":
            request.stdout.write(b"\x00\xff\x01")
            request.stderr.write(b"\xfe\x00")
            request.stdout.write(b"\x02")
        elif command == "large_write":
            request.stdout.write(bytes(range(256)) * 12289)
        elif command == "throw":
            raise RuntimeError("private callback detail")
        elif command == "bad_status":
            return True
        elif command in ("system_exit_zero", "abrupt_exit_zero"):
            with Path(request.argv[1]).open("ab") as effect:
                effect.write(b"warm\n")
            if command == "system_exit_zero":
                raise SystemExit(0)
            os._exit(0)
        elif command in ("owned_joined", "owned_live", "owned_expired"):
            marker = Path(request.argv[1])
            self.owned_markers.append(marker)
            self.owned.invoke(request)
            marker.write_bytes(b"started\n")
            if command == "owned_expired":
                # Waiting without signalling proves reset rejects expiry even after the actor dies.
                self.owned.thread.join(6)
                assert not self.owned.thread.is_alive()
                with marker.open("ab") as evidence:
                    evidence.write(b"expired\n")
        elif command == "blocked_input":
            request.stdin.read(1)
        elif command == "blocked_output":
            while True:
                request.stdout.write(bytes(65536))
        elif command == "streaming":
            progress = Path(request.argv[1])
            pending = progress.with_name(progress.name + ".pending")
            pattern = bytes(index % 251 for index in range(65536))

            def publish(completed):
                pending.write_text(json.dumps({"pid": os.getpid(), "completed_bytes": completed}))
                pending.replace(progress)

            completed = 0
            publish(completed)
            while completed < 128 * 1024 * 1024:
                written = request.stdout.write(pattern)
                assert written == len(pattern)
                completed += written
                publish(completed)
        elif command == "borrowed_close":
            request.stdin.close()
            request.stdout.close()
            request.stderr.close()
            assert request.stdin.read(0) == b""
            assert request.stdout.write(b"") == 0
            request.stdout.flush()
            request.stdout.write(b"ok")
        elif command.startswith("late_"):
            retained, late_operation = request, command
        elif command == "retain":
            retained = request
        elif command == "old_write":
            try:
                retained.stdout.write(b"late")
            except OSError:
                pass
        elif command == "invalidate_caught":
            for _ in range(2):
                try:
                    request.invalidate()
                except OSError:
                    pass
        elif command == "uncaught":
            def escape():
                raise RuntimeError("private thread detail")
            thread = threading.Thread(target=escape)
            thread.start()
            thread.join()
        elif command == "unraisable":
            class BrokenFinalizer:
                def __del__(self):
                    raise RuntimeError("private finalizer detail")
            value = BrokenFinalizer()
            del value
            gc.collect()
        elif command == "gated_late":
            gate = Path(request.argv[1])
            def late():
                deadline = time.monotonic() + 5
                while not gate.exists() and time.monotonic() < deadline:
                    time.sleep(.005)
                if gate.exists():
                    try:
                        request.stdout.write(b"late")
                    except OSError:
                        pass
            threading.Thread(target=late, daemon=True).start()
        elif command in ("concurrent_input", "concurrent_output", "outstanding_input"):
            gate = Path(request.argv[1])
            def operation():
                try:
                    if command == "concurrent_output":
                        while True:
                            request.stdout.write(bytes(65536))
                    else:
                        request.stdin.read(1)
                except OSError:
                    pass
            threading.Thread(target=operation, daemon=True).start()
            deadline = time.monotonic() + 5
            while not gate.exists() and time.monotonic() < deadline:
                if request.cancelled:
                    return 0
                time.sleep(.005)
            assert gate.exists()
            if command == "concurrent_input":
                request.stdin.read(1)
            elif command == "concurrent_output":
                request.stderr.write(b"overlap")
        return 0

    def reset(self, request):
        assert threading.current_thread() is self.execution
        assert all(thread in self.baseline for thread in threading.enumerate()
                   if thread.name.startswith("mdrun-"))
        if request.argv[0] in ("owned_joined", "owned_live", "owned_expired"):
            marker = Path(request.argv[1])
            try:
                self.owned.reset(request)
            except BaseException:
                with marker.open("ab") as evidence:
                    evidence.write(b"reset rejected\n")
                raise
            with marker.open("ab") as evidence:
                evidence.write(b"reset clean\n")
        if request.argv[0] == "resetfail":
            raise RuntimeError("private reset detail")

    def close(self):
        assert threading.current_thread() is self.execution
        self.owned.close()
        for marker in self.owned_markers:
            with marker.open("ab") as evidence:
                evidence.write(b"closed\n")
        if os.environ.get("MDRUN_PYTHON_FIXTURE") == "close_failure":
            raise RuntimeError("private close detail")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("config", type=Path)
    arguments = parser.parse_args()
    if os.environ.get("MDRUN_PYTHON_FIXTURE") == "receive_after_shutdown":
        receive = mdrun_sdk._Session.receive
        def receive_after_shutdown(session, frame):
            arguments.config.with_name("parsed").touch()
            assert session.cancelled.wait(2)
            receive(session, frame)
        mdrun_sdk._Session.receive = receive_after_shutdown
    previous_hooks = threading.excepthook, sys.unraisablehook
    try:
        run_worker(arguments.config, Fixture)
        assert (threading.excepthook, sys.unraisablehook) == previous_hooks
        try:
            run_worker(arguments.config, Fixture)
        except WorkerError as failure:
            assert str(failure) == "worker_already_started"
        else:
            raise AssertionError("second worker lifetime was accepted")
        if retained is not None:
            if late_operation == "late_atexit":
                atexit.register(attempt_late)
            else:
                attempt_late()
    except BaseException:
        if (threading.excepthook, sys.unraisablehook) != previous_hooks:
            return 71
        return 70
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
