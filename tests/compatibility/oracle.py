"""Compare executable Markdown CLIs in disposable, equivalent workspaces."""

import argparse
import base64
import copy
from contextlib import ExitStack
from dataclasses import dataclass, field
from datetime import datetime
from email.parser import BytesParser
from email.policy import default
import hashlib
from http.server import BaseHTTPRequestHandler, HTTPServer
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import tempfile
import threading
from urllib.parse import unquote_to_bytes
import uuid


@dataclass
class Step:
    args: list[str]
    stdin: bytes = b""


@dataclass
class Case:
    name: str
    files: dict[str, bytes]
    steps: list[Step]
    remote_status: int | None = None
    generated_documents: list[str] = field(default_factory=list)
    generated_images: bool = False
    directories: list[str] = field(default_factory=list)
    reconstruct_document: str | None = None
    reconstruct_files: dict[str, bytes] = field(default_factory=dict)


def request_fields(content_type, body):
    if content_type.startswith("application/x-www-form-urlencoded"):
        fields = {}
        for pair in body.split(b"&"):
            if not pair:
                continue
            key, _, value = pair.partition(b"=")
            name = unquote_to_bytes(key.replace(b"+", b" ")).decode("ascii")
            fields.setdefault(name, []).append(unquote_to_bytes(value.replace(b"+", b" ")))
        return fields, []
    if content_type.startswith("multipart/form-data"):
        message = BytesParser(policy=default).parsebytes(
            b"Content-Type: " + content_type.encode() + b"\r\n\r\n" + body
        )
        fields, uploads = {}, []
        for part in message.iter_parts():
            name = part.get_param("name", header="content-disposition")
            data = part.get_payload(decode=True)
            if part.get_filename() is None:
                fields.setdefault(name, []).append(data)
            else:
                uploads.append({"name": name, "filename": part.get_filename(),
                                "content_type": part.get_content_type(), "body": data})
        return fields, uploads
    raise ValueError(f"unexpected request content type: {content_type}")


def kill_group(pid):
    try:
        os.killpg(pid, signal.SIGKILL)
    except ProcessLookupError:
        pass


def observe(command, case, timeout=10, bindings=None):
    requests = []

    class Capture(BaseHTTPRequestHandler):
        def do_POST(self):
            body = self.rfile.read(int(self.headers.get("Content-Length", "0")))
            content_type = self.headers.get("Content-Type", "")
            fields, uploads = request_fields(content_type, body)
            requests.append({"method": self.command, "path": self.path,
                             "content_type": content_type.split(";", 1)[0],
                             "fields": fields, "uploads": uploads,
                             "raw_content_type": content_type, "raw_body": body})
            self.send_response(case.remote_status)
            self.end_headers()

        def log_message(self, *_args):
            pass

    server = HTTPServer(("127.0.0.1", 0), Capture) if case.remote_status else None
    thread = None
    if server:
        thread = threading.Thread(target=server.serve_forever, kwargs={"poll_interval": 0.01})
        thread.start()
    try:
        with tempfile.TemporaryDirectory(prefix="mdrun-oracle-") as directory, ExitStack() as cleanup:
            root = Path(directory)
            for name in case.directories:
                (root / name).mkdir(parents=True, exist_ok=True)
            for name, content in case.files.items():
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(content)
            # Whitelist context so developer credentials, shell startup and remote settings cannot leak in.
            env = {"PATH": "/usr/bin:/bin", "HOME": directory, "TMPDIR": directory,
                   "LANG": "C", "LC_ALL": "C", "TZ": "UTC", "PWD": directory}
            if bindings:
                bindir = cleanup.enter_context(tempfile.TemporaryDirectory(prefix="mdrun-oracle-bin-"))
                for name, executable in bindings.items():
                    (Path(bindir) / name).symlink_to(executable)
                env["PATH"] = bindir + ":" + env["PATH"]
            if server:
                env["SHOWBOAT_REMOTE_URL"] = f"http://127.0.0.1:{server.server_port}/capture"
            observations = []
            for step in case.steps:
                request_start = len(requests)
                process = subprocess.Popen([*command, *step.args], cwd=root, env=env,
                                           stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                           stderr=subprocess.PIPE, start_new_session=True)
                cleanup.callback(kill_group, process.pid)
                try:
                    stdout, stderr = process.communicate(step.stdin, timeout=timeout)
                except subprocess.TimeoutExpired as exc:
                    kill_group(process.pid)
                    process.communicate()
                    raise TimeoutError(f"{case.name}: {step.args!r} exceeded {timeout}s") from exc
                files = {str(path.relative_to(root)): path.read_bytes()
                         for path in sorted(root.rglob("*")) if path.is_file()}
                directories = [str(path.relative_to(root)) for path in sorted(root.rglob("*")) if path.is_dir()]
                observations.append({"status": process.returncode, "stdout": stdout,
                                     "stderr": stderr, "files": files, "directories": directories,
                                     "requests": copy.deepcopy(requests[request_start:])})
                if case.reconstruct_document and step.args[0] == "extract" and process.returncode == 0:
                    replay = Case(case.name + "-reconstruction", case.reconstruct_files, [Step([], stdout)])
                    observations[-1]["reconstruction"] = observe(
                        ["/bin/sh"], replay, timeout,
                        bindings={"showboat": command[0], "mdrun": command[0]},
                    )
            return observations
    finally:
        if server:
            server.shutdown()
            thread.join()
            server.server_close()


HEADER = re.compile(rb"\A(# [^\n]*\n\n)\*([^\n]+) by (Showboat 0\.6\.1|mdrun [^\n *]+)\*\n<!-- (?:showboat|mdrun)-id: ([^\n]+) -->\n")
IMAGE = re.compile(r"[0-9a-f]{8}-(\d{4}-\d{2}-\d{2})\.(png|jpg|jpeg|gif|svg)\Z")


def canonical(case, observations):
    result = copy.deepcopy(observations)
    identities, images = {}, {}
    for snapshot in result:
        if "reconstruction" in snapshot:
            replay = Case(case.name + "-reconstruction", case.reconstruct_files, [],
                          generated_documents=[case.reconstruct_document], generated_images=True)
            snapshot["reconstruction"] = canonical(replay, snapshot["reconstruction"])
        for name in case.generated_documents:
            if name not in snapshot["files"]:
                continue
            data = snapshot["files"][name]
            match = HEADER.match(data)
            if not match:
                raise ValueError(f"{case.name}: malformed generated document header in {name}")
            timestamp, identity = match[2].decode(), match[4].decode()
            datetime.strptime(timestamp, "%Y-%m-%dT%H:%M:%SZ")
            parsed = uuid.UUID(identity)
            if str(parsed) != identity or parsed.version != 4 or parsed.variant != uuid.RFC_4122:
                raise ValueError(f"{case.name}: expected canonical UUID v4 in {name}")
            if name in identities and identities[name] != (timestamp, identity):
                raise ValueError(f"{case.name}: generated metadata changed after init in {name}")
            identities[name] = (timestamp, identity)
            snapshot["files"][name] = match[1] + b"*<timestamp> by <author>*\n<!-- <id>: " + name.encode() + b" -->\n" + data[match.end():]

        if case.generated_images:
            for name in snapshot["files"]:
                if name in case.files or name in images:
                    continue
                match = IMAGE.fullmatch(Path(name).name)
                if match:
                    datetime.strptime(match[1], "%Y-%m-%d")
                    images[name] = str(Path(name).with_name(f"<image-{len(images)}>" + Path(name).suffix))
            snapshot["files"] = {images.get(name, name): data for name, data in snapshot["files"].items()}
            for name, data in snapshot["files"].items():
                if name.endswith(".md"):
                    for old, new in images.items():
                        old_base, new_base = Path(old).name.encode(), Path(new).name.encode()
                        data = data.replace(b"](" + old_base + b")\n", b"](" + new_base + b")\n")
                        data = data.replace(b"![" + Path(old).stem.encode() + b"](", b"![" + Path(new).stem.encode() + b"](")
                    snapshot["files"][name] = data

        for request in snapshot["requests"]:
            # Multipart boundaries and form escaping are transport encodings; values and uploads stay exact.
            request.pop("raw_body", None)
            request.pop("raw_content_type", None)
            fields = request.get("fields", {})
            for name, (_, identity) in identities.items():
                if fields.get("uuid") == [identity.encode()]:
                    fields["uuid"] = [f"<id:{name}>".encode()]
            for old, new in images.items():
                for key, old_value, new_value in [
                    ("filename", Path(old).name, Path(new).name),
                    ("alt", Path(old).stem, Path(new).stem),
                ]:
                    if fields.get(key) == [old_value.encode()]:
                        fields[key] = [new_value.encode()]
                for upload in request.get("uploads", []):
                    if upload["filename"] in (old, Path(old).name):
                        upload["filename"] = new if upload["filename"] == old else Path(new).name
    return result


def compare(case, left, right):
    expected, actual = canonical(case, left), canonical(case, right)
    if len(expected) != len(actual):
        return ["step count"]
    return [f"step {index + 1}: {key}" for index, (a, b) in enumerate(zip(expected, actual))
            for key in sorted(a.keys() | b.keys()) if a.get(key) != b.get(key)]


def json_value(value):
    if isinstance(value, bytes):
        return {"base64": base64.b64encode(value).decode()}
    if isinstance(value, dict):
        return {key: json_value(item) for key, item in value.items()}
    if isinstance(value, list):
        return [json_value(item) for item in value]
    return value


def main():
    from cases import cases
    from prepare import verify_identity
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--oracle", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--results", type=Path, required=True)
    parser.add_argument("--case", help="case-name substring filter")
    args = parser.parse_args()
    commands = {key: [str(value.resolve())] for key, value in
                [("oracle", args.oracle), ("candidate", args.candidate)]}
    identity = verify_identity(Path(commands["oracle"][0]))
    selected = [case for case in cases() if not args.case or args.case in case.name]
    if not selected:
        parser.error("case filter matched nothing")
    args.results.mkdir(parents=True, exist_ok=False)
    failures = []
    for case in selected:
        observations = {key: observe(command, case) for key, command in commands.items()}
        differences = compare(case, observations["oracle"], observations["candidate"])
        if differences:
            failures.append({"case": case.name, "differences": differences})
        record = {"case": case.name, "steps": [step.args for step in case.steps],
                  "differences": differences, **observations}
        (args.results / f"{case.name}.json").write_text(json.dumps(json_value(record), indent=2) + "\n")
    summary = {"cases": len(selected), "failures": failures, "oracle_identity": identity,
               "binaries": {key: {"path": command[0], "sha256": hashlib.sha256(Path(command[0]).read_bytes()).hexdigest()}
                            for key, command in commands.items()}}
    (args.results / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(f"{len(selected)} cases, {len(failures)} differences; evidence: {args.results}")
    return bool(failures)


if __name__ == "__main__":
    raise SystemExit(main())
