#!/usr/bin/env python3
"""Verify interrupted interview disk resume with isolated Codex and local fixtures."""

import gzip
import http.server
import importlib.util
import json
import os
from pathlib import Path
import queue
import socket
import subprocess
import tempfile
import threading
import time

MODULE = Path(__file__).resolve().with_name("codex-policy-persistence.py")
spec = importlib.util.spec_from_file_location("native_probe", MODULE)
shared = importlib.util.module_from_spec(spec)
spec.loader.exec_module(shared)

QUESTIONS = [
    {
        "id": "first", "header": "First",
        "question": "Choose a nonsecret fixture option.",
        "options": [
            {"label": "A", "description": "First option"},
            {"label": "B", "description": "Second option"},
        ],
    },
    {
        "id": "second", "header": "Second",
        "question": "Choose a nonsecret fixture note.",
        "options": [
            {"label": "Yes", "description": "Include the note"},
            {"label": "No", "description": "Skip the note"},
        ],
    },
]


def assert_no_replayed_questions(client, thread_id, turn_id):
    def check(event):
        if event.get("method") == "error" or "error" in event:
            raise RuntimeError("native protocol error during interview resume")
        params = event.get("params", {})
        if event.get("method") == "item/tool/requestUserInput" and (
            params.get("threadId") == thread_id or params.get("turnId") == turn_id
        ):
            raise AssertionError("native disk resume replayed a pending question RPC")

    for event in client.events:
        check(event)
    deadline = time.monotonic() + 3
    while time.monotonic() + 1 <= deadline:
        try:
            event = client.receive(1)
        except queue.Empty:
            if client.process.poll() is not None:
                raise RuntimeError("native app-server exited during the resume quiet period")
            return
        check(event)
    raise RuntimeError("native resume did not reach a bounded quiet period")


def run(codex):
    shared.enable_loopback()
    assert {name for _, name in socket.if_nameindex()} == {"lo"}
    version = subprocess.check_output([codex, "--version"], text=True).strip()
    if version != "codex-cli 0.154.0":
        raise RuntimeError(f"this validation requires exact Codex 0.154.0: {version}")
    requests = []
    failures = []

    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_POST(self):
            try:
                self.connection.settimeout(10)
                length = int(self.headers.get("Content-Length", "0"))
                if not 0 < length <= 2_000_000:
                    raise RuntimeError("unexpected fixture request size")
                raw = self.rfile.read(length)
                if self.headers.get("Content-Encoding") == "gzip":
                    raw = gzip.decompress(raw)
                body = json.loads(raw)
                requests.append(body.get("model"))
                assert len(requests) == 1, "no fixture retry or extra Turn allowed"
                tools = [
                    nested
                    for tool in body.get("tools", [])
                    for nested in (
                        tool.get("tools", []) if tool.get("type") == "namespace" else [tool]
                    )
                ]
                assert any(tool.get("name") == "request_user_input" for tool in tools)
                item = {
                    "type": "function_call", "id": "fc_fixture",
                    "call_id": "call_fixture", "name": "request_user_input",
                    "arguments": json.dumps({"questions": QUESTIONS}), "status": "completed",
                }
                response = {
                    "id": "resp_fixture", "object": "response", "status": "completed",
                    "model": body.get("model"), "output": [item],
                    "usage": {"input_tokens": 1, "output_tokens": 1, "total_tokens": 2},
                }
                events = [
                    {
                        "type": "response.created",
                        "response": {**response, "status": "in_progress", "output": []},
                    },
                    {"type": "response.output_item.done", "output_index": 0, "item": item},
                    {"type": "response.completed", "response": response},
                ]
                data = "".join("data: " + json.dumps(event) + "\n\n" for event in events).encode()
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)
            except Exception as error:
                failures.append(str(error))
                self.send_error(500)

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    worker = threading.Thread(target=server.serve_forever, daemon=True)
    worker.start()
    client = None
    try:
        with tempfile.TemporaryDirectory(prefix="yo-codex-interview-") as directory:
            root = Path(directory)
            workspace = root / "workspace"
            workspace.mkdir()
            native_home = root / "native"
            native_home.mkdir(mode=0o700)
            environment = os.environ.copy()
            environment["CODEX_HOME"] = str(native_home)
            argv = shared.fixture_argv(codex, f"http://127.0.0.1:{server.server_port}/v1")
            index = argv.index("app-server")
            argv[index:index] = ["--enable", "default_mode_request_user_input"]

            def start():
                native = shared.NativeClient(argv, environment, workspace)
                try:
                    native.call("initialize", {
                        "clientInfo": {"name": "yo_interview_resume_validation", "version": "1"},
                        "capabilities": {"experimentalApi": True},
                    })
                    native.send({"method": "initialized", "params": {}})
                    return native
                except BaseException:
                    native.close()
                    raise

            client = start()
            thread_id = client.call("thread/start", {
                "cwd": str(workspace), "model": "gpt-5.5", "modelProvider": "yo_policy_fixture",
                "sandbox": "read-only", "approvalPolicy": "never",
            })["thread"]["id"]
            turn_id = client.call("turn/start", {
                "threadId": thread_id,
                "input": [{"type": "text", "text": "Ask exactly the two nonsecret fixture questions."}],
            })["turn"]["id"]
            pending = None
            deadline = time.monotonic() + 30
            while time.monotonic() < deadline:
                event = client.receive(max(0.01, deadline - time.monotonic()))
                if event.get("method") == "item/tool/requestUserInput":
                    pending = event
                    break
                if event.get("method") == "turn/completed":
                    raise RuntimeError(json.dumps({
                        "event": event, "fixture_failures": failures, "local_requests": len(requests),
                    }))
            assert pending and pending["params"]["threadId"] == thread_id
            assert pending["params"]["turnId"] == turn_id
            received = pending["params"]["questions"]
            assert len(received) == len(QUESTIONS)
            for actual, expected in zip(received, QUESTIONS):
                assert all(actual.get(key) == value for key, value in expected.items())
                assert not actual.get("isSecret", False)
            client.close()
            client = None

            client = start()
            resumed = client.call("thread/resume", {"threadId": thread_id})["thread"]
            old = next(turn for turn in resumed["turns"] if turn["id"] == turn_id)
            assert old["status"] == "interrupted", old
            assert_no_replayed_questions(client, thread_id, turn_id)
            assert len(requests) == 1 and not failures, failures
            client.close()
            client = None
        return {
            "version": version, "pending_nonsecret_batch_questions": len(QUESTIONS),
            "native_process_restart": "passed", "recovered_original_turn_status": "interrupted",
            "pending_rpc_replayed_after_disk_resume": False,
            "local_fixture_requests": len(requests), "external_model_requests": 0,
            "normal_user_state": "unchanged", "owned_temporary_state": "removed",
        }
    finally:
        if client is not None:
            client.close()
        server.shutdown()
        server.server_close()
        worker.join(timeout=5)


if __name__ == "__main__":
    shared.main(check=run, description=__doc__)
