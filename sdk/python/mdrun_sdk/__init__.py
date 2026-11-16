"""Experimental, dependency-free peer for one cooperative Python worker process."""

from dataclasses import dataclass, field
from enum import Enum, auto
import base64
import json
import os
from pathlib import Path
import re
import select
import socket
import struct
import sys
import threading
from typing import Callable


_FRAME_LIMIT = 1_048_576
_CHUNK_LIMIT = 65_536
_MAX_SEQUENCE = 9_007_199_254_740_991
_START_LOCK = threading.Lock()
_STARTED = False


class WorkerError(RuntimeError):
    """Report a stable worker infrastructure failure, excluding application details."""


@dataclass(frozen=True, slots=True)
class Eligible:
    """Permit the classified arguments to enter cooperative execution."""


@dataclass(frozen=True, slots=True)
class Refused:
    """Supply a stable nonempty reason for pre-execution cold routing."""

    reason: str


class Application:
    """Own application eligibility, callable behavior and resource cleanup checks."""

    def classify(self, argv: tuple[str, ...]) -> Eligible | Refused:
        """Classify immutable arguments without application effects, I/O or input reads."""
        raise NotImplementedError

    def invoke(self, invocation: "Invocation") -> int:
        """Return an ordinary application status from 0 through 255."""
        raise NotImplementedError

    def reset(self, invocation: "Invocation") -> None:
        """Attest application-owned work has ended and state/resources permit reuse."""
        raise NotImplementedError

    def close(self) -> None:
        """Release the application after serving ends, on the execution thread."""


@dataclass(frozen=True, slots=True)
class Invocation:
    """Provide immutable arguments, borrowed binary streams and cooperative cancellation."""

    request_id: str
    argv: tuple[str, ...]
    stdin: "_Input"
    stdout: "_Output"
    stderr: "_Output"
    _session: "_Session" = field(repr=False)

    @property
    def cancelled(self) -> bool:
        return self._session.cancelled.is_set()

    def wait_cancelled(self, timeout: float | None = None) -> bool:
        return self._session.cancelled.wait(timeout)

    def invalidate(self) -> None:
        """Permanently fault this worker; catching the exception cannot restore health."""
        raise self._session.violation("application_invalidated")


class _WireError(Exception):
    pass


class _ConnectionLost(Exception):
    pass


class _LocalClosure(Exception):
    pass


def _require(condition):
    if not condition:
        raise _WireError


def _fields(value, names):
    _require(type(value) is dict and value.keys() == set(names))


# Lone surrogates cannot be encoded as UTF-8 on the wire; a per-character Python loop dominated stdin frame parsing.
_SURROGATE = re.compile("[\ud800-\udfff]")


def _string(value, nonempty=False):
    _require(type(value) is str and (not nonempty or bool(value)))
    _require(value.isascii() or _SURROGATE.search(value) is None)
    return value


def _integer(value, minimum, maximum):
    _require(type(value) is int and minimum <= value <= maximum)
    return value


def _parse(body):
    def pairs(items):
        result = {}
        for key, value in items:
            _require(key not in result)
            result[key] = value
        return result

    def integer(text):
        _require(len(text) <= 32)
        return int(text)

    def forbidden(_):
        raise _WireError

    try:
        value = json.loads(body.decode("utf-8"), object_pairs_hook=pairs,
                           parse_int=integer, parse_float=forbidden, parse_constant=forbidden)
        pending = [(value, 0)]
        while pending:
            item, depth = pending.pop()
            _require(depth <= 16)
            if type(item) is str:
                _string(item)
            elif type(item) is dict:
                pending.extend((key, depth + 1) for key in item)
                pending.extend((child, depth + 1) for child in item.values())
            elif type(item) is list:
                pending.extend((child, depth + 1) for child in item)
        _require(type(value) is dict)
        return value
    except (UnicodeError, ValueError, RecursionError):
        raise _WireError from None


def _encode(frame):
    body = json.dumps(frame, ensure_ascii=False, allow_nan=False, separators=(",", ":")).encode("utf-8")
    _require(0 < len(body) <= _FRAME_LIMIT)
    return struct.pack(">I", len(body)) + body


class _Channel:
    def __init__(self, port):
        self.socket = socket.create_connection(("127.0.0.1", port))
        self.socket.setblocking(False)
        self.closed = threading.Event()
        self.write_lock = threading.Lock()

    def _wait(self, reading):
        if self.closed.is_set():
            raise _LocalClosure
        select.select([self.socket] if reading else [], [] if reading else [self.socket], [], .1)

    def _read_exact(self, size):
        result = bytearray(size)
        offset = 0
        try:
            while offset < size:
                if self.closed.is_set():
                    raise _LocalClosure
                try:
                    count = self.socket.recv_into(memoryview(result)[offset:])
                except BlockingIOError:
                    self._wait(True)
                    continue
                if count == 0:
                    raise _ConnectionLost
                offset += count
        except (OSError, ValueError):
            raise _LocalClosure if self.closed.is_set() else _ConnectionLost from None
        return result

    def read(self):
        size = struct.unpack(">I", self._read_exact(4))[0]
        _require(0 < size <= _FRAME_LIMIT)
        frame = _parse(self._read_exact(size))
        _string(frame.get("kind"))
        return frame

    def send(self, frame):
        data = memoryview(_encode(frame))
        with self.write_lock:
            try:
                while data:
                    if self.closed.is_set():
                        raise _LocalClosure
                    try:
                        count = self.socket.send(data)
                    except BlockingIOError:
                        self._wait(False)
                        continue
                    if count == 0:
                        raise _ConnectionLost
                    data = data[count:]
            except (OSError, ValueError):
                raise _LocalClosure if self.closed.is_set() else _ConnectionLost from None

    def abort(self, frame):
        # A fault is best effort: cancellation must not wait for a blocked writer.
        if self.write_lock.acquire(blocking=False):
            try:
                data = memoryview(_encode(frame))
                while data and not self.closed.is_set():
                    count = self.socket.send(data)
                    if not count:
                        break
                    data = data[count:]
            except (OSError, ValueError, _WireError):
                pass
            finally:
                self.write_lock.release()
        self.close()

    def close(self):
        self.closed.set()
        try:
            self.socket.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        self.socket.close()


class _Phase(Enum):
    CLASSIFYING = auto()
    ELIGIBLE = auto()
    RUNNING = auto()
    RESETTING = auto()
    COMPLETE = auto()
    RELEASED = auto()
    FAULTED = auto()


@dataclass(slots=True)
class _PendingRead:
    limit: int
    done: bool = False
    data: bytes | None = None


class _Request:
    def __init__(self, session, request_id, argv):
        self.request_id = request_id
        self.argv = argv
        self.phase = _Phase.CLASSIFYING
        self.input_lock = threading.Lock()
        self.output_lock = threading.Lock()
        self.pending_read = None
        self.input_busy = False
        self.output_busy = False
        self.eof = False
        self.sequence = 0
        self.invocation = Invocation(request_id, argv, _Input(session, self),
                                     _Output(session, self, "stdout"), _Output(session, self, "stderr"), session)


class _Input:
    def __init__(self, session, request):
        self.session, self.request = session, request

    def read(self, size: int) -> bytes:
        if type(size) is not int or size < 0:
            raise self.session.violation("invalid_input_credit")
        if size == 0:
            return b""
        request, session = self.request, self.session
        if not request.input_lock.acquire(blocking=False):
            raise session.violation("concurrent_input")
        pending = None
        try:
            with session.condition:
                if not session.live(request):
                    raise session.violation("late_input")
                if request.eof:
                    return b""
                pending = _PendingRead(min(size, _CHUNK_LIMIT))
                request.pending_read = pending
                request.input_busy = True
            try:
                session.data.send({"kind": "stdin_read", "request_id": request.request_id, "max_bytes": pending.limit})
            except BaseException:
                raise session.violation("input_failed") from None
            with session.condition:
                session.condition.wait_for(lambda: pending.done or session.stopped)
                if session.stopped:
                    raise OSError("worker_stopped")
                return b"" if pending.data is None else pending.data
        finally:
            with session.condition:
                if pending is not None:
                    request.input_busy = False
                session.condition.notify_all()
            request.input_lock.release()

    def readinto(self, buffer) -> int:
        try:
            view = memoryview(buffer).cast("B")
            if view.readonly:
                raise TypeError
        except (TypeError, ValueError):
            raise self.session.violation("invalid_input_buffer") from None
        try:
            data = self.read(len(view))
            view[:len(data)] = data
            return len(data)
        finally:
            view.release()

    def close(self) -> None:
        pass


class _Output:
    def __init__(self, session, request, stream):
        self.session, self.request, self.stream = session, request, stream

    def write(self, data) -> int:
        try:
            view = memoryview(data).cast("B")
        except (TypeError, ValueError):
            raise self.session.violation("invalid_output") from None
        try:
            if len(view) == 0:
                return 0
            request, session = self.request, self.session
            if not request.output_lock.acquire(blocking=False):
                raise session.violation("concurrent_output")
            started = False
            try:
                with session.condition:
                    if not session.live(request):
                        raise session.violation("late_output")
                    request.output_busy = True
                    started = True
                for offset in range(0, len(view), _CHUNK_LIMIT):
                    with session.condition:
                        if session.stopped:
                            raise OSError("worker_stopped")
                        if request.sequence > _MAX_SEQUENCE:
                            raise session.violation("sequence_limit")
                        sequence = request.sequence
                        request.sequence += 1
                    encoded = base64.b64encode(view[offset:offset + _CHUNK_LIMIT]).decode("ascii")
                    try:
                        session.data.send({"kind": "output", "request_id": request.request_id,
                                           "sequence": sequence, "stream": self.stream, "base64": encoded})
                    except BaseException:
                        raise session.violation("output_failed") from None
                return len(view)
            finally:
                with session.condition:
                    if started:
                        request.output_busy = False
                    session.condition.notify_all()
                request.output_lock.release()
        finally:
            view.release()

    def flush(self) -> None:
        pass

    def close(self) -> None:
        pass


class _Session:
    def __init__(self, config, factory):
        self.config, self.factory = config, factory
        self.condition = threading.Condition()
        self.cancelled = threading.Event()
        self.initialized = threading.Event()
        self.barrier = threading.Barrier(3)
        self.current = self.task = self.application = None
        self.data = self.control = None
        self.request_ids = set()
        self.ready = self.stopped = self.returned = False
        self.failure = None
        self.threads = [self.thread("mdrun-data", self.data_loop), self.thread("mdrun-control", self.control_loop),
                        self.thread("mdrun-execution", self.execution_loop)]

    def thread(self, name, body):
        def run():
            try:
                self.barrier.wait()
            except threading.BrokenBarrierError:
                return
            body()
        return threading.Thread(target=run, name=name, daemon=True)

    def stop(self, reason):
        with self.condition:
            if self.returned and reason is not None:
                os._exit(70)
            if reason is not None and self.failure is None:
                self.failure = reason
            first = not self.stopped
            self.stopped = True
            active = self.current
            if active is not None:
                active.phase = _Phase.FAULTED
            self.cancelled.set()
            self.initialized.set()
            self.condition.notify_all()
        self.barrier.abort()
        if first:
            if reason is not None and self.data is not None:
                self.data.abort({"kind": "fault", "request_id": active.request_id if active else None, "reason": reason})
            self.close_channels()

    def violation(self, reason):
        self.stop(reason)
        return OSError(reason)

    def live(self, request):
        return not self.stopped and self.current is request and request.phase is _Phase.RUNNING

    def close_channels(self):
        if self.control is not None:
            self.control.close()
        if self.data is not None:
            self.data.close()

    def hello(self, role):
        return {"kind": "hello", **{name: self.config[name] for name in ("protocol", "session_id", "artifact_id", "token")}, "role": role}

    def run(self):
        previous_thread_hook, previous_unraisable_hook = threading.excepthook, sys.unraisablehook
        started = []
        try:
            self.data = _Channel(self.config["connect"]["port"])
            self.data.send(self.hello("worker_data"))
            self.control = _Channel(self.config["connect"]["port"])
            self.control.send(self.hello("worker_control"))
            threading.excepthook = lambda _: self.stop("uncaught_exception")
            sys.unraisablehook = lambda _: self.stop("unraisable_exception")
            for thread in self.threads:
                thread.start()
                started.append(thread)
            self.initialized.wait()
            with self.condition:
                if not self.stopped:
                    self.ready = True
            if not self.stopped:
                self.data.send({"kind": "ready", **{name: self.config[name] for name in ("protocol", "session_id", "artifact_id")},
                                "capabilities": ["pull_stdin", "sequenced_output", "release_ack"]})
            with self.condition:
                self.condition.wait_for(lambda: self.stopped)
        except BaseException:
            self.stop("worker_failed")
        finally:
            if not self.stopped:
                self.stop("worker_failed")
            self.close_channels()
            for thread in started:
                thread.join()
            threading.excepthook, sys.unraisablehook = previous_thread_hook, previous_unraisable_hook
            with self.condition:
                self.returned = True
        if self.failure is not None:
            raise WorkerError(self.failure)

    def execution_loop(self):
        try:
            if self.stopped:
                return
            application = self.factory()
            _require(all(callable(getattr(application, name, None)) for name in ("classify", "invoke", "reset", "close")))
            self.application = application
            self.initialized.set()
            while True:
                with self.condition:
                    self.condition.wait_for(lambda: self.task is not None or self.stopped)
                    if self.stopped:
                        return
                    operation, request = self.task
                    self.task = None
                if operation == "classify":
                    self.classify(request)
                else:
                    self.invoke(request)
        except BaseException:
            if not self.stopped:
                self.stop("initialization_failed" if self.application is None else "execution_failed")
        finally:
            self.initialized.set()
            if self.application is not None:
                try:
                    self.application.close()
                except BaseException:
                    self.stop("close_failed")

    def classify(self, request):
        try:
            result = self.application.classify(request.argv)
        except BaseException:
            self.stop("classification_failed")
            return
        try:
            with self.condition:
                if self.stopped:
                    return
                _require(self.current is request and request.phase is _Phase.CLASSIFYING)
                if type(result) is Eligible:
                    request.phase = _Phase.ELIGIBLE
                    response = {"kind": "eligible", "request_id": request.request_id}
                else:
                    _require(type(result) is Refused)
                    reason = _string(result.reason, nonempty=True)
                    request.phase = _Phase.RELEASED
                    self.current = None
                    response = {"kind": "refused", "request_id": request.request_id, "reason": reason}
            self.data.send(response)
        except BaseException:
            self.stop("invalid_classification")

    def invoke(self, request):
        try:
            status = self.application.invoke(request.invocation)
        except BaseException:
            self.stop("invocation_failed")
            return
        try:
            with self.condition:
                if self.stopped:
                    return
                request.phase = _Phase.RESETTING
                if type(status) is not int or not 0 <= status <= 255:
                    self.stop("invalid_exit_code")
                    return
                if request.input_busy or request.pending_read is not None:
                    self.stop("outstanding_input")
                    return
                self.condition.wait_for(lambda: not request.output_busy or self.stopped)
                if self.stopped:
                    return
            self.application.reset(request.invocation)
        except BaseException:
            self.stop("reset_failed")
            return
        try:
            with self.condition:
                if self.stopped:
                    return
                request.phase = _Phase.COMPLETE
            self.data.send({"kind": "complete", "request_id": request.request_id, "exit_code": status})
        except BaseException:
            self.stop("output_failed")

    def matching(self, frame, phase):
        request = self.current
        _require(request is not None and request.phase is phase and frame["request_id"] == request.request_id)
        return request

    def receive(self, frame):
        kind = frame["kind"]
        names = {"classify": ("kind", "request_id", "argv"), "invoke": ("kind", "request_id"),
                 "stdin": ("kind", "request_id", "base64"), "stdin_eof": ("kind", "request_id"),
                 "release": ("kind", "request_id")}
        _require(kind in names)
        _fields(frame, names[kind])
        _string(frame["request_id"], nonempty=True)
        argv, data = None, None
        if kind == "classify":
            _require(type(frame["argv"]) is list)
            argv = tuple(_string(argument) for argument in frame["argv"])
        elif kind == "stdin":
            text = _string(frame["base64"], nonempty=True)
            _require(len(text) <= 87384)
            try:
                data = base64.b64decode(text, validate=True)
            except (ValueError, UnicodeError):
                raise _WireError from None
            _require(0 < len(data) <= _CHUNK_LIMIT and base64.b64encode(data).decode("ascii") == text)
        with self.condition:
            if self.stopped:
                return
            _require(self.ready)
            if kind == "classify":
                _require(self.current is None and self.task is None and frame["request_id"] not in self.request_ids)
                self.request_ids.add(frame["request_id"])
                self.current = _Request(self, frame["request_id"], argv)
                self.task = ("classify", self.current)
                self.condition.notify_all()
            elif kind == "invoke":
                request = self.matching(frame, _Phase.ELIGIBLE)
                _require(self.task is None)
                request.phase = _Phase.RUNNING
            elif kind in ("stdin", "stdin_eof"):
                request = self.matching(frame, _Phase.RUNNING)
                pending = request.pending_read
                _require(pending is not None and not pending.done)
                if kind == "stdin":
                    _require(len(data) <= pending.limit)
                request.pending_read = None
                request.eof = data is None
                pending.data, pending.done = data, True
                self.condition.notify_all()
            elif kind == "release":
                request = self.matching(frame, _Phase.COMPLETE)
                request.phase = _Phase.RELEASED
                self.current = None
        if kind == "invoke":
            self.data.send({"kind": "accepted", "request_id": request.request_id})
            with self.condition:
                if not self.stopped:
                    self.task = ("invoke", request)
                    self.condition.notify_all()
        elif kind == "release":
            self.data.send({"kind": "released", "request_id": request.request_id})

    def data_loop(self):
        try:
            while not self.stopped:
                self.receive(self.data.read())
        except _LocalClosure:
            if not self.stopped:
                self.stop("data_lost")
        except _ConnectionLost:
            self.stop("data_lost")
        except BaseException:
            self.stop("protocol_error")

    def control_loop(self):
        try:
            frame = self.control.read()
            _fields(frame, ("kind",))
            _require(frame["kind"] == "shutdown")
            with self.condition:
                if self.current is not None:
                    self.stop("shutdown_during_invocation")
                    return
            self.stop(None)
        except _LocalClosure:
            if not self.stopped:
                self.stop("owner_lost")
        except _ConnectionLost:
            self.stop("owner_lost")
        except BaseException:
            self.stop("invalid_control")


def run_worker(config_path: str | os.PathLike, factory: Callable[[], Application]) -> None:
    """Serve one dedicated process lifetime, initializing after all SDK threads start.

    The factory and callbacks use one execution thread. Application callbacks must
    return cooperatively on cancellation; native host custody owns their deadline.
    Retained positive-length stream operations and invalidation after return halt
    the process with status 70, including when their exceptions would be caught.
    """
    global _STARTED
    with _START_LOCK:
        if _STARTED:
            raise WorkerError("worker_already_started")
        _STARTED = True
    if not callable(factory):
        raise WorkerError("invalid_adapter")
    try:
        with Path(config_path).open("rb") as source:
            body = source.read(_FRAME_LIMIT + 1)
        _require(0 < len(body) <= _FRAME_LIMIT)
        config = _parse(body)
        _fields(config, ("protocol", "session_id", "artifact_id", "token", "connect"))
        _integer(config["protocol"], 1, 1)
        for name in ("session_id", "artifact_id", "token"):
            _string(config[name], nonempty=True)
        _fields(config["connect"], ("host", "port"))
        _require(config["connect"]["host"] == "127.0.0.1")
        _integer(config["connect"]["port"], 1, 65535)
    except BaseException:
        raise WorkerError("invalid_config") from None
    _Session(config, factory).run()
