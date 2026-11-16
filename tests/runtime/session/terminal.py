"""Qualify actual-host terminal behavior with a controlling PTY and independent cold execution."""

import argparse
import ctypes
import errno
import fcntl
import hashlib
import json
import os
from pathlib import Path
import platform
import pty
import select
import shlex
import shutil
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import termios
import time


SCRIPT = Path(__file__).resolve()
INPUT = b"terminal input\n"
OUTPUT = b"OUT\x00\xff\nERR\xfe\x00\nINPUT=" + INPUT
CASES = ["independent", "cold", "fast", "no_grant", "no_restore", "independent_interrupt",
         "cold_interrupt", "fast_interrupt", "background_session", "background_group"]


def publish(directory, name, value):
    path = directory / name
    temporary = path.with_suffix(".tmp")
    temporary.write_text(json.dumps(value, sort_keys=True) + "\n")
    temporary.replace(path)


def invocation(args, role):
    return [sys.executable, str(SCRIPT), "--role", role, "--case", args.case,
            "--directory", str(args.directory), "--mdrun", str(args.mdrun)]


def descriptors():
    # Darwin exposes FMARK/FWASWRITTEN bookkeeping without changing descriptor access/behavior.
    # https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/fcntl.h
    written = 0x00011000 if sys.platform == "darwin" else 0
    return [{"device": os.fstat(fd).st_dev, "inode": os.fstat(fd).st_ino,
             "terminal_device": os.fstat(fd).st_rdev, "flags": fcntl.fcntl(fd, fcntl.F_GETFL) & ~written}
            for fd in range(3)]


def foreground(group):
    # A supervisor restores foreground ownership while it is itself in the background.
    # https://man7.org/linux/man-pages/man3/tcsetpgrp.3.html
    previous = signal.pthread_sigmask(signal.SIG_BLOCK, {signal.SIGTTOU})
    try:
        os.tcsetpgrp(0, group)
    finally:
        signal.pthread_sigmask(signal.SIG_SETMASK, previous)


def bounded(args, seconds):
    def expired(*_):
        publish(args.directory, args.role + ".expired.json", {"pid": os.getpid()})
        raise TimeoutError("fixture self-expiry")
    signal.signal(signal.SIGALRM, expired)
    signal.alarm(seconds)


def process_info(pid):
    if sys.platform == "darwin":
        # Public proc_bsdinfo supplies the birth identity needed across the group-freeze handshake.
        # https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/proc_info.h
        class BsdInfo(ctypes.Structure):
            _fields_ = [(name, ctypes.c_uint32) for name in
                        ("flags", "status", "xstatus", "pid", "parent", "uid", "gid", "ruid", "rgid", "svuid", "svgid", "reserved")]
            _fields_ += [("comm", ctypes.c_char * 16), ("name", ctypes.c_char * 32)]
            _fields_ += [(name, ctypes.c_uint32) for name in ("nfiles", "group", "jobc", "tdev", "tgroup")]
            _fields_ += [("nice", ctypes.c_int32), ("start_seconds", ctypes.c_uint64), ("start_microseconds", ctypes.c_uint64)]
        info = BsdInfo()
        library = ctypes.CDLL("/usr/lib/libproc.dylib", use_errno=True)
        inspect = library.proc_pidinfo
        inspect.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_uint64, ctypes.c_void_p, ctypes.c_int]
        inspect.restype = ctypes.c_int
        size = inspect(pid, 3, 0, ctypes.byref(info), ctypes.sizeof(info))
        if size == 0 and ctypes.get_errno() == errno.ESRCH:
            return None
        if size != ctypes.sizeof(info):
            raise OSError(ctypes.get_errno(), "cannot inspect owned fixture process")
        return {"pid": pid, "parent": info.parent, "group": info.group,
                "identity": [info.start_seconds, info.start_microseconds],
                "state": "Z" if info.status == 5 else "T" if info.status == 4 else "live"}
    try:
        fields = Path(f"/proc/{pid}/stat").read_bytes().rsplit(b")", 1)[1].split()
    except OSError as error:
        if error.errno in (errno.ENOENT, errno.ESRCH):
            return None
        raise
    return {"pid": pid, "parent": int(fields[1]), "group": int(fields[2]),
            "identity": [int(fields[19])], "state": fields[0].decode("ascii")}


def process_rows(deadline=None):
    timeout = 2 if deadline is None else min(2, deadline - time.monotonic())
    if timeout <= 0:
        raise TimeoutError("fixture teardown exceeded its absolute deadline")
    output = subprocess.check_output(["/bin/ps", "-axo", "pid=,ppid=,pgid=,stat="], text=True, timeout=timeout)
    return [{"pid": int(pid), "parent": int(parent), "group": int(group), "state": state[0]}
            for pid, parent, group, state in (line.split() for line in output.splitlines())]


def wait_for(condition, seconds, description):
    deadline = time.monotonic() + seconds
    while not condition():
        if time.monotonic() >= deadline:
            raise TimeoutError(description)
        time.sleep(0.005)


def same_process(reference):
    current = process_info(reference["pid"])
    assert current is not None and current["identity"] == reference["identity"], "fixture process identity changed"
    return current


def pinned_child(parent, reference, deadline=None):
    current_parent = same_process(parent)
    assert current_parent["state"] == "T", "fixture parent must remain stopped while child identifiers are used"
    row = next((row for row in process_rows(deadline) if row["pid"] == reference["pid"]), None)
    assert row is not None and row["parent"] == parent["pid"], "fixture child lost its stopped parent pin"
    if row["state"].startswith("Z"):
        assert same_process(parent)["state"] == "T"
        return row
    current = process_info(reference["pid"])
    if current is None:
        # Darwin's native identity query can return ESRCH after a pinned child becomes a zombie.
        # The still-stopped parent plus an explicit current Z row proves dead without signaling it.
        row = next((row for row in process_rows(deadline) if row["pid"] == reference["pid"]), None)
        assert row is not None and row["parent"] == parent["pid"] and row["state"].startswith("Z")
        assert same_process(parent)["state"] == "T"
        return row
    assert current["parent"] == parent["pid"], "fixture child lost its stopped parent pin"
    if "identity" in reference:
        assert current["identity"] == reference["identity"], "live fixture child identity changed"
    assert same_process(parent)["state"] == "T"
    return current


def terminate_pinned_tree(parent, reference, visited, deadline):
    assert len(visited) < 32, "fixture teardown tree exceeded its bounded inventory"
    assert time.monotonic() < deadline, "fixture teardown exceeded its absolute deadline"
    current = pinned_child(parent, reference, deadline)
    reference = current
    visited.append(current["pid"])
    if current["state"] == "Z":
        return
    os.kill(current["pid"], signal.SIGSTOP)
    wait_for(lambda: pinned_child(parent, reference, deadline)["state"] in ("T", "Z"), min(0.5, deadline - time.monotonic()), "fixture child did not stop")
    current = pinned_child(parent, reference, deadline)
    if current["state"] != "Z":
        children = [row for row in process_rows(deadline) if row["parent"] == current["pid"]]
        for child in children:
            terminate_pinned_tree(current, child, visited, deadline)
        pinned_child(parent, reference, deadline)
        os.kill(current["pid"], signal.SIGKILL)
        wait_for(lambda: pinned_child(parent, reference, deadline)["state"] == "Z", min(0.5, deadline - time.monotonic()), "fixture child did not exit")


def retained_teardown(args, custodian, groups):
    # The directly owned wrapper freezes its own group, then only it is resumed by its driver.
    # A stopped custodian cannot reap anchors and invalidate their individual PID identities.
    os.kill(0, signal.SIGSTOP)
    deadline = time.monotonic() + 3
    wait_for(lambda: same_process(custodian)["state"] == "T", 0.5, "custodian did not stop")
    assert same_process(custodian)["group"] == os.getpgrp(), "custodian left the live wrapper group"
    report = json.loads((args.directory / "report.json").read_text())
    state = json.loads(Path(report["cleanup"]["status_path"]).read_text())
    publish(args.directory, "retained-custody.json", state)
    anchors = {group["anchor_pid"] for group in state["groups"]}
    children = [row for row in process_rows(deadline) if row["parent"] == custodian["pid"]]
    assert {child["pid"] for child in children}.issubset(anchors), "custodian has an unaccounted child"
    visited = []
    for child in children:
        terminate_pinned_tree(custodian, child, visited, deadline)
    assert not live_members(groups, exclude={os.getpid()}, deadline=deadline), "payload group survived pinned teardown"
    publish(args.directory, "teardown.json", {"custodian": custodian, "terminated_children": visited})
    os.kill(0, signal.SIGKILL)
    raise AssertionError("owned group sweep returned")


def payload(args):
    if args.case.endswith("interrupt"):
        def interrupted(*_):
            publish(args.directory, "interrupted.json", {"signal": signal.SIGINT})
            os.write(1, b"INTERRUPTED\n")
            raise SystemExit(0)
        signal.signal(signal.SIGINT, interrupted)
    publish(args.directory, "payload.json", {
        "pid": os.getpid(), "group": os.getpgrp(), "session": os.getsid(0),
        "foreground": os.tcgetpgrp(0), "isatty": [os.isatty(fd) for fd in range(3)],
        "descriptors": descriptors(),
    })
    if args.case.endswith("interrupt"):
        while True:
            signal.pause()
    os.write(1, b"OUT\x00\xff\n")
    os.write(2, b"ERR\xfe\x00\n")
    data = os.read(0, 100)
    publish(args.directory, "input.json", {"bytes": data.hex()})
    os.write(1, b"INPUT=" + data)
    return 17


def author(args):
    publish(args.directory, "author.json", {"pid": os.getpid(), "group": os.getpgrp()})
    if args.case == "no_restore":
        child = subprocess.Popen(invocation(args, "payload"), process_group=0)
        foreground(child.pid)
    else:
        child = subprocess.Popen(["tool"], stdin=None, stdout=None, stderr=None)
    try:
        status = child.wait(timeout=7)
    finally:
        if child.poll() is None:
            child.kill()
            child.wait()
    if args.case.endswith("interrupt"):
        os.write(1, b"AUTHOR_MASKED\n")
        return 0
    return status


def background(args):
    publish(args.directory, "background.json", {
        "pid": os.getpid(), "group": os.getpgrp(), "parent_group": os.getpgid(os.getppid()),
        "session": os.getsid(0),
    })
    sequence = 0
    try:
        while not (args.directory / "finished").exists():
            publish(args.directory, "heartbeat.json", {"sequence": sequence})
            sequence += 1
            time.sleep(0.01)
    finally:
        publish(args.directory, "background.closed.json", {"pid": os.getpid()})
    return 0


def exact(connection, size):
    data = bytearray()
    while len(data) < size:
        chunk = connection.recv(size - len(data))
        if not chunk:
            raise EOFError("ready-only peer lost its owner")
        data.extend(chunk)
    return bytes(data)


def send(connection, frame):
    data = json.dumps(frame, separators=(",", ":")).encode()
    connection.sendall(struct.pack(">I", len(data)) + data)


def peer(args):
    application_frames = 0
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(70))
    publish(args.directory, "peer.started.json", {"group": os.getpgrp()})
    publish(args.directory, "peer.json", {"application_frames": 0})
    config = json.loads(Path(args.config).read_text())
    identity = {key: config[key] for key in ("protocol", "session_id", "artifact_id", "token")}
    sockets = []
    try:
        for role in ("worker_data", "worker_control"):
            connection = socket.create_connection((config["connect"]["host"], config["connect"]["port"]), timeout=3)
            sockets.append(connection)
            send(connection, {"kind": "hello", **identity, "role": role})
        send(sockets[0], {"kind": "ready", **{key: config[key] for key in ("protocol", "session_id", "artifact_id")},
                          "capabilities": ["pull_stdin", "sequenced_output", "release_ack"]})
        readable, _, _ = select.select(sockets, [], [], 6)
        if sockets[0] in readable:
            header = sockets[0].recv(4)
            if header:
                application_frames += 1
                raise AssertionError("terminal preflight dispatched application work to the ready-only peer")
        if sockets[1] not in readable:
            return 70
        size = struct.unpack(">I", exact(sockets[1], 4))[0]
        assert 0 < size <= 1048576
        assert json.loads(exact(sockets[1], size)) == {"kind": "shutdown"}
        return 0
    finally:
        publish(args.directory, "peer.json", {"application_frames": application_frames})
        for connection in sockets:
            connection.close()


def job(args):
    assert os.read(args.gate, 1) == b"G"
    if args.case in ("independent", "independent_interrupt", "no_grant"):
        command = invocation(args, "payload")
    elif args.case == "no_restore":
        command = invocation(args, "author")
    else:
        mode = "fast" if args.case.startswith("fast") else "cold"
        command = [str(args.mdrun), "session", "--mode", mode, "--binding", str(args.directory / "binding.json"),
                   "--report", str(args.directory / "report.json"), "--"]
        if args.case == "background_group":
            child = shlex.join(invocation(args, "background"))
            command.extend(["/bin/sh", "-m", "-c", child + " & while [ ! -f background.json ]; do /bin/sleep 0.01; done; exit 0"])
        else:
            command.extend(invocation(args, "author"))
    if args.case == "no_grant":
        os.close(args.gate)
        os.execv(command[0], command)
    child = subprocess.Popen(command)
    # Keep the outer ownership authority alive when the terminal interrupts its foreground child group.
    signal.signal(signal.SIGINT, signal.SIG_IGN)
    try:
        status = child.wait(timeout=8)
    finally:
        if child.poll() is None:
            child.kill()
            child.wait()
    signal.alarm(0)
    groups = known_groups(args.directory)
    report_path = args.directory / "report.json"
    custodian = None
    if report_path.exists():
        report = json.loads(report_path.read_text())
        if report["cleanup"]["outcome"] != "clean":
            custodian = process_info(report["cleanup"]["custodian_pid"])
            assert custodian is not None and custodian["group"] == os.getpgrp()
    publish(args.directory, "job.json", {"status": status, "group": os.getpgrp(), "groups": sorted(groups)})
    acknowledgement = os.read(args.gate, 1)
    if acknowledgement == b"R":
        os.close(args.gate)
        return status
    assert acknowledgement == b"S" and custodian is not None
    try:
        retained_teardown(args, custodian, groups)
    except BaseException as error:
        publish(args.directory, "teardown.failure.json", {
            "error": repr(error), "custodian": custodian, "groups": sorted(groups),
        })
        assert os.read(args.gate, 1) == b"F", "driver did not observe fixture teardown failure"
        os.close(args.gate)
        if not live_members(groups, exclude={os.getpid()}):
            publish(args.directory, "teardown.json", {"failure_observed": True, "payload_groups_empty": True})
            os.kill(0, signal.SIGKILL)
        return 76


def known_groups(directory):
    groups = set()
    for name in ("payload.json", "author.json", "peer.started.json", "background.json"):
        path = directory / name
        if path.exists():
            groups.add(json.loads(path.read_text())["group"])
    report_path = directory / "report.json"
    if report_path.exists():
        status_path = json.loads(report_path.read_text())["cleanup"]["status_path"]
        if status_path:
            deadline = time.monotonic() + 0.5
            while True:
                try:
                    state = json.loads(Path(status_path).read_text())
                    break
                except json.JSONDecodeError:
                    if time.monotonic() >= deadline:
                        raise
                    time.sleep(0.005)
            groups.update(group["anchor_pid"] for group in state["groups"])
            publish(directory, "retained-custody.json", state)
    return groups


def live_members(groups, exclude=(), deadline=None):
    return [row["pid"] for row in process_rows(deadline)
            if row["group"] in groups and row["pid"] not in exclude and not row["state"].startswith("Z")]


def driver(args):
    initial = {"group": os.getpgrp(), "foreground": os.tcgetpgrp(0), "descriptors": descriptors()}
    assert initial["group"] == initial["foreground"]
    publish(args.directory, "driver.json", initial)
    with socket.socket() as reservation:
        reservation.bind(("127.0.0.1", 0))
        port = reservation.getsockname()[1]
    publish(args.directory, "binding.json", {
        "version": 1, "name": "tool", "artifact_id": "controlling-terminal-fixture",
        "worker_argv": invocation(args, "peer"), "cold_argv": invocation(args, "payload"),
        "resources": [{"kind": "tcp_listener", "host": "127.0.0.1", "port": port}],
        "startup_timeout_ms": 2000, "session_timeout_ms": 5000, "cleanup_timeout_ms": 1000,
    })
    read, write = os.pipe()
    child = subprocess.Popen([*invocation(args, "job"), "--gate", str(read)], pass_fds=[read], process_group=0)
    os.close(read)
    result = {"expected_foreground": initial["group"] if args.case == "background_session" else child.pid}
    try:
        if args.case not in ("no_grant", "background_session"):
            foreground(child.pid)
        os.write(write, b"G")
        if args.case == "no_grant":
            deadline = time.monotonic() + 3
            while time.monotonic() < deadline:
                _, status = os.waitpid(child.pid, os.WUNTRACED | os.WNOHANG)
                if os.WIFSTOPPED(status):
                    result["stopped_signal"] = os.WSTOPSIG(status)
                    break
                time.sleep(0.005)
            child.kill()
            result["status"] = child.wait(timeout=1)
        else:
            deadline = time.monotonic() + 8
            while not (args.directory / "job.json").exists():
                assert child.poll() is None, "live wrapper exited before publishing host status"
                if time.monotonic() >= deadline:
                    raise TimeoutError("host did not finish before its external watchdog")
                time.sleep(0.005)
            result["status"] = json.loads((args.directory / "job.json").read_text())["status"]
        result["returned_foreground"] = os.tcgetpgrp(0)
        result["returned_descriptors"] = descriptors()
    finally:
        foreground(initial["group"])
        result["driver_restored"] = os.tcgetpgrp(0) == initial["group"]
        if args.case == "background_group" and (args.directory / "heartbeat.json").exists():
            before = json.loads((args.directory / "heartbeat.json").read_text())["sequence"]
            limit = time.monotonic() + 0.3
            after = before
            while after <= before and time.monotonic() < limit:
                after = json.loads((args.directory / "heartbeat.json").read_text())["sequence"]
                time.sleep(0.005)
            result["background_advanced_after_session"] = after > before
            result["listener_available_before_fixture_cleanup"] = available(port)
        (args.directory / "finished").write_bytes(b"")
        if child.poll() is None:
            groups = known_groups(args.directory)
            report = args.directory / "report.json"
            retained = report.exists() and json.loads(report.read_text())["cleanup"]["outcome"] != "clean"
            if not retained:
                wait_for(lambda: not live_members(groups, exclude={child.pid}), 1, "live payload prevented fixture exit")
            os.write(write, b"S" if retained else b"R")
            if retained:
                def wrapper_stopped():
                    pid, status = os.waitpid(child.pid, os.WUNTRACED | os.WNOHANG)
                    if pid and (os.WIFEXITED(status) or os.WIFSIGNALED(status)):
                        child.returncode = os.waitstatus_to_exitcode(status)
                        raise AssertionError("fixture wrapper exited before freezing its group")
                    return bool(pid and os.WIFSTOPPED(status) and os.WSTOPSIG(status) == signal.SIGSTOP)
                wait_for(wrapper_stopped, 1, "wrapper did not freeze its own group")
                child.send_signal(signal.SIGCONT)
                def teardown_finished():
                    failure = args.directory / "teardown.failure.json"
                    if failure.exists() and "teardown_failure" not in result:
                        result["teardown_failure"] = json.loads(failure.read_text())
                        publish(args.directory, "result.json", result)
                        os.write(write, b"F")
                    return child.poll() is not None
                wait_for(teardown_finished, 4, "retained fixture teardown did not finish")
            teardown_status = child.wait(timeout=4)
            assert teardown_status == (-signal.SIGKILL if retained else result["status"]), "fixture wrapper exited without the required cleanup acknowledgement"
            result["fixture_teardown"] = "owned_group_sweep" if retained else "ordinary_exit"
            if retained:
                wait_for(lambda: not live_members({child.pid}), 0.5, "owned fixture group remained live after self-sweep")
                private = Path(json.loads(report.read_text())["cleanup"]["status_path"]).parent
                shutil.rmtree(private)
        os.close(write)
        publish(args.directory, "result.json", result)
    return 0


def available(port):
    with socket.socket() as probe:
        try:
            probe.bind(("127.0.0.1", port))
            return True
        except OSError as error:
            if error.errno != errno.EADDRINUSE:
                raise
            return False


def capture(args, case):
    with tempfile.TemporaryDirectory(prefix="mdrun-terminal-") as temporary:
        case_args = argparse.Namespace(**vars(args))
        case_args.case, case_args.directory = case, Path(temporary)
        master, slave = pty.openpty()
        state = termios.tcgetattr(slave)
        state[1] &= ~termios.OPOST
        state[3] &= ~termios.ECHO
        termios.tcsetattr(slave, termios.TCSANOW, state)

        def controlling_terminal():
            os.setsid()
            fcntl.ioctl(0, termios.TIOCSCTTY, 0)

        child = subprocess.Popen(invocation(case_args, "driver"), stdin=slave, stdout=slave, stderr=slave,
                                 cwd=temporary, preexec_fn=controlling_terminal)
        os.close(slave)
        output = bytearray()
        sent = False
        excluded_during_payload = None
        deadline = time.monotonic() + 12
        try:
            while True:
                if not sent and case != "no_grant" and (case_args.directory / "payload.json").exists():
                    if case not in ("independent", "independent_interrupt", "no_restore"):
                        binding = json.loads((case_args.directory / "binding.json").read_text())
                        excluded_during_payload = not available(binding["resources"][0]["port"])
                    os.write(master, b"\x03" if case.endswith("interrupt") else INPUT)
                    sent = True
                if select.select([master], [], [], 0.02)[0]:
                    try:
                        data = os.read(master, 65536)
                    except OSError as error:
                        if error.errno != errno.EIO:
                            raise
                        data = b""
                    output.extend(data)
                    if not data:
                        child.wait(timeout=1)
                        break
                elif child.poll() is not None:
                    break
                if time.monotonic() >= deadline:
                    raise TimeoutError("external controlling-terminal watchdog expired")
            child.wait(timeout=1)
            result = {"driver_status": child.returncode, "pty_hex": output.hex(),
                      "excluded_during_payload": excluded_during_payload}
            binding = json.loads((case_args.directory / "binding.json").read_text())
            result["listener_available_after_session"] = available(binding["resources"][0]["port"])
            result["files"] = {path.name: json.loads(path.read_text()) for path in case_args.directory.glob("*.json")}
            return result
        finally:
            (case_args.directory / "finished").write_bytes(b"")
            if child.poll() is None:
                child.kill()
                child.wait(timeout=1)
            if (case_args.directory / "background.json").exists():
                limit = time.monotonic() + 1
                while not (case_args.directory / "background.closed.json").exists() and time.monotonic() < limit:
                    time.sleep(0.005)
                assert (case_args.directory / "background.closed.json").exists(), "background fixture did not stop cooperatively"
            os.close(master)


def check(case, result):
    files = result["files"]
    assert result["driver_status"] == 0, result
    assert "teardown.failure.json" not in files, result
    assert not any("expired" in name for name in files), result
    state = files["result.json"]
    assert state["driver_restored"], result
    if case == "no_grant":
        assert state["stopped_signal"] == signal.SIGTTIN, result
        assert "input.json" not in files, result
        assert files["payload.json"]["foreground"] != files["payload.json"]["group"], result
        return
    if case == "no_restore":
        assert state["status"] == 17, result
        assert files["input.json"]["bytes"] == INPUT.hex(), result
        assert result["pty_hex"] == OUTPUT.hex(), result
        assert files["payload.json"]["foreground"] == files["payload.json"]["group"], result
        assert state["returned_foreground"] != state["expected_foreground"], result
        return
    assert state["returned_foreground"] == state["expected_foreground"], result
    assert state["returned_descriptors"] == files["driver.json"]["descriptors"], result
    if case == "background_session":
        assert "author.json" not in files and "payload.json" not in files, result
        assert state["status"] != 0, result
    elif case == "background_group":
        background = files["background.json"]
        assert background["group"] != background["parent_group"], result
        assert not (state["background_advanced_after_session"] and state["listener_available_before_fixture_cleanup"]), "ordinary shell job survived released custody: " + json.dumps(result)
        return
    else:
        payload = files["payload.json"]
        assert payload["isatty"] == [True, True, True], result
        assert payload["foreground"] == payload["group"], result
        assert payload["descriptors"] == files["driver.json"]["descriptors"], result
        if case.endswith("interrupt"):
            assert files["interrupted.json"]["signal"] == signal.SIGINT, result
            assert b"INTERRUPTED\n" in bytes.fromhex(result["pty_hex"]), result
            if case != "independent_interrupt":
                assert b"AUTHOR_MASKED\n" in bytes.fromhex(result["pty_hex"]), result
                assert state["status"] != 0, result
        else:
            assert state["status"] == 17, result
            assert files["input.json"]["bytes"] == INPUT.hex(), result
            assert result["pty_hex"] == OUTPUT.hex(), result
    if case in ("independent", "independent_interrupt"):
        return
    report = files["report.json"]
    assert report["cleanup"]["outcome"] == "clean", result
    assert result["listener_available_after_session"], result
    assert report["warm_calls"] == 0, result
    if case != "background_session":
        fast = case.startswith("fast")
        assert result["excluded_during_payload"], result
        assert report["cold_calls"] == 1, result
        assert report["worker_starts"] == (1 if fast else 0), result
        assert report["cold_reasons"] == {"terminal" if fast else "explicit_cold": 1}, result
        if fast:
            assert files["peer.json"]["application_frames"] == 0, result
    if case == "background_session" or case.endswith("interrupt"):
        assert report["infrastructure_failure"] is not None, result
        if case.endswith("interrupt"):
            assert report["author_status"]["exit_code"] == 0, result
    else:
        assert report["infrastructure_failure"] is None, result


def main():
    # Pinned-child teardown requires zombie-preserving SIGCHLD disposition across fixture execs.
    # https://man7.org/linux/man-pages/man2/wait.2.html
    signal.signal(signal.SIGCHLD, signal.SIG_DFL)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--role", choices=["qualify", "driver", "job", "author", "payload", "peer", "background"], default="qualify")
    parser.add_argument("--mdrun", type=Path, required=True)
    parser.add_argument("--case", choices=CASES)
    parser.add_argument("--directory", type=Path)
    parser.add_argument("--results", type=Path)
    parser.add_argument("--gate", type=int)
    parser.add_argument("config", nargs="?")
    args = parser.parse_args()
    args.mdrun = args.mdrun.resolve()
    if args.role != "qualify":
        bounded(args, 10 if args.role == "driver" else 8)
        return globals()[args.role](args)
    if args.results is None:
        parser.error("--results is required for qualification")
    args.results.mkdir(parents=True)
    cases = [args.case] if args.case else CASES[:-1]
    with tempfile.TemporaryDirectory(prefix="mdrun-terminal-candidate-") as temporary:
        candidate = Path(temporary) / "mdrun"
        shutil.copy2(args.mdrun, candidate)
        args.mdrun = candidate
        publish(args.results, "identity.json", {"candidate_sha256": hashlib.sha256(candidate.read_bytes()).hexdigest(),
                                                "python": sys.version, "platform": platform.platform(), "cases": cases})
        for case in cases:
            result = capture(args, case)
            publish(args.results, case + ".json", result)
            check(case, result)
            print(case + ": passed", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
