#!/usr/bin/env python3
"""Diagnose a managed TUI's pre-acceptance failure using loopback and a fake key.

The fixture rejects TLS before any HTTP request. It never forwards traffic and
does not reproduce a provider's HTTP status. Evidence is collected before all
temporary state is removed, including on a failed check.
"""

import argparse
import errno
import fcntl
import json
import os
from pathlib import Path
import platform
import re
import select
import signal
import socket
import struct
import subprocess
import tempfile
import termios
import time


DEADLINE_SECONDS = 15
CAPTURE_LIMIT = 4 * 1024 * 1024


def child_terminal(slave):
    os.setsid()
    fcntl.ioctl(slave, termios.TIOCSCTTY, 0)


def tag_count(value, tag):
    if isinstance(value, dict):
        return int(value.get("type") == tag) + sum(
            tag_count(item, tag) for item in value.values() if isinstance(item, (dict, list))
        )
    if isinstance(value, list):
        return sum(tag_count(item, tag) for item in value)
    return 0


def collect_state(root, report):
    journals = sorted((root / "sessions").glob("*.jsonl"))
    records = [json.loads(line) for path in journals for line in path.read_text().splitlines()]
    report["session_files"] = len(journals)
    report["journal_records"] = len(records)
    report["accepted_request_records"] = tag_count(records, "backend_request_accepted")
    report["turn_finished_records"] = tag_count(records, "turn_finished")
    connections = root / "config" / "connections.yaml"
    # Copy only the typed failure line; never capture credentials.yaml.
    report["binding_failure_kinds"] = re.findall(
        r"(?m)^\s*last_failure:\n\s*kind: ([a-z_]+)\s*$", connections.read_text()
    ) if connections.exists() else []


def run(binary, root, report):
    for name in ["config", "home", "state", "sessions", "workspace"]:
        (root / name).mkdir()
    # Listen only on loopback. No DNS, proxy, CA or ordinary user-state changes.
    listener = socket.socket()
    listener.bind(("127.0.0.1", 0))
    listener.listen(4)
    listener.setblocking(False)
    master, slave = os.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 0, 0))
    original_termios = termios.tcgetattr(slave)
    child = None
    stdout = bytearray()
    stderr = bytearray()
    report.update(phase="import", submissions=0, connection_attempts=0, fixture_http_requests=0)
    try:
        port = listener.getsockname()[1]
        definition = root / "connection.yaml"
        definition.write_text(f"""provider: offline
account: failure
base_url: https://127.0.0.1:{port}/v1
profile:
  api_dialect: openai-chat-completions
  tokenizer_profile: utf8-bytes/v1
  input_token_limit: 256000
  max_output_tokens: 65536
  reasoning_parameters: {{}}
  optional_request_parameters: {{}}
  tool_capability_policy: no-tools/v1
  replay_profile: semantic-only/v1
models:
  - model: fixture
""")
        key = root / "fake-key"
        key.write_text("offline-fixture-not-a-real-credential\n")
        key.chmod(0o600)
        # HOME is isolated only in child processes, including macOS host identity.
        environment = {
            "PATH": os.defpath,
            "HOME": str(root / "home"),
            "XDG_CONFIG_HOME": str(root / "config"),
            "XDG_STATE_HOME": str(root / "state"),
            "YO_CONFIG": str(root / "config" / "config.yaml"),
            "YO_SESSION_REPOSITORY": str(root / "sessions"),
            "YO_SESSION_CAPACITY_BYTES": "16777216",
            "TERM": "xterm-256color",
            "LANG": "en_US.UTF-8",
        }
        imported = subprocess.run(
            [str(binary), "connect", "--from", str(definition),
             "--credential-file", str(key), "--yes"],
            cwd=root / "workspace", env=environment, stdin=subprocess.DEVNULL,
            capture_output=True, timeout=DEADLINE_SECONDS,
        )
        key.unlink()
        if imported.returncode:
            raise RuntimeError("isolated import failed: " + imported.stderr.decode(errors="replace"))
        report["phase"] = "ready"
        child = subprocess.Popen(
            [str(binary), "--" + report["mode"], "--model", "offline:failure:fixture"],
            cwd=root / "workspace", env=environment,
            stdin=slave, stdout=slave, stderr=subprocess.PIPE,
            preexec_fn=lambda: child_terminal(slave),
        )
        streams = {master: stdout, child.stderr.fileno(): stderr}
        deadline = time.monotonic() + DEADLINE_SECONDS
        while streams:
            if time.monotonic() >= deadline:
                raise RuntimeError("TUI readiness/exit exceeded the bounded deadline")
            readable, _, _ = select.select([*streams, listener], [], [], 0.05)
            for descriptor in readable:
                if descriptor is listener:
                    connection, _ = listener.accept()
                    report["connection_attempts"] += 1
                    # Closing the socket rejects the TLS handshake; no HTTP exists.
                    connection.close()
                    continue
                try:
                    data = os.read(descriptor, 65536)
                except OSError as error:
                    if error.errno != errno.EIO:
                        raise
                    data = b""
                if not data:
                    del streams[descriptor]
                else:
                    streams[descriptor].extend(data)
                    if len(streams[descriptor]) > CAPTURE_LIMIT:
                        raise RuntimeError("terminal capture exceeded the bounded limit")
            mode = termios.tcgetattr(slave)
            raw = not mode[3] & (termios.ICANON | termios.ECHO)
            if report["phase"] == "ready" and raw and b"Ask anything" in stdout:
                os.write(master, b"offline failure probe\r")
                report["submissions"] += 1
                report["phase"] = "submitted"
            if child.poll() is not None:
                # Keeping the slave open lets us compare termios after child exit.
                # Drain available output, then stop rather than waiting for PTY EOF.
                if not readable:
                    break
        report["exit_code"] = child.wait(timeout=1)
        report["phase"] = "exited"
    finally:
        if child is not None:
            if child.poll() is None:
                os.killpg(child.pid, signal.SIGKILL)
                child.wait(timeout=5)
            report["exit_code"] = child.returncode
            child.stderr.close()
        report["termios_restored"] = termios.tcgetattr(slave) == original_termios
        report["alternate_screen_enter"] = stdout.count(b"\x1b[?1049h")
        report["alternate_screen_leave"] = stdout.count(b"\x1b[?1049l")
        report["stderr_bytes"] = len(stderr)
        report["stderr"] = stderr[:4096].decode(errors="replace").strip()
        try:
            collect_state(root, report)
        except Exception as error:
            report["state_capture_error"] = str(error)
        finally:
            os.close(master)
            os.close(slave)
            listener.close()

    expected = {
        "phase": "exited", "exit_code": 1, "submissions": 1, "connection_attempts": 1,
        "fixture_http_requests": 0, "accepted_request_records": 0,
        "turn_finished_records": 0, "session_files": 1,
        "termios_restored": True,
        "alternate_screen_enter": int(report["mode"] == "fullscreen"),
        "alternate_screen_leave": int(report["mode"] == "fullscreen"),
    }
    for field, value in expected.items():
        if report.get(field) != value:
            raise RuntimeError(f"{field}: expected {value!r}, got {report.get(field)!r}")
    if "Transport: model-connector HTTP request failed" not in report["stderr"]:
        raise RuntimeError("missing typed connector failure on stderr")
    if report["binding_failure_kinds"] != ["transport"]:
        raise RuntimeError("binding did not retain the typed transport failure")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path, help="built Yo binary (Linux or macOS)")
    parser.add_argument("--mode", choices=["inline", "fullscreen"], default="fullscreen")
    arguments = parser.parse_args()
    binary = arguments.binary.resolve(strict=True)
    report = {"os": platform.system(), "arch": platform.machine(),
              "mode": arguments.mode, "passed": False}
    with tempfile.TemporaryDirectory(prefix="yo-managed-start-failure-") as directory:
        root = Path(directory).resolve()
        try:
            run(binary, root, report)
            report["passed"] = True
        except Exception as error:
            report["error"] = str(error)
    report["temporary_state_removed"] = not root.exists()
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0 if report["passed"] and report["temporary_state_removed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
