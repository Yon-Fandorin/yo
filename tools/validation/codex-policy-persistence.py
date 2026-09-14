#!/usr/bin/env python3
"""Exercise installed Codex policy writes/reloads using loopback-only inference fixtures."""

import argparse
import fcntl
import gzip
import http.server
import json
import os
from pathlib import Path
import queue
import shlex
import shutil
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time


class NativeClient:
    def __init__(self, argv, environment, cwd):
        self.process = subprocess.Popen(
            argv, cwd=cwd, env=environment, stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True,
        )
        self.messages = queue.Queue()
        self.next_id = 1
        self.events = []
        self.responses = []
        self.thread = threading.Thread(target=self._read, daemon=True)
        self.thread.start()

    def _read(self):
        try:
            for line in self.process.stdout:
                self.messages.put(json.loads(line))
        finally:
            self.messages.put(None)

    def send(self, message):
        self.process.stdin.write(json.dumps(message) + "\n")
        self.process.stdin.flush()

    def receive(self, timeout=30):
        message = self.messages.get(timeout=timeout)
        if message is None:
            raise RuntimeError("native app-server closed before the expected event")
        self.events.append(message)
        return message

    def call(self, method, params):
        request_id = self.next_id
        self.next_id += 1
        self.send({"id": request_id, "method": method, "params": params})
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            message = self.receive(max(0.01, deadline - time.monotonic()))
            if message.get("id") == request_id and "method" not in message:
                if "error" in message:
                    raise RuntimeError(f"{method}: {message['error']}")
                return message["result"]
        raise RuntimeError(f"{method} did not return")

    def answer(self, request, decision):
        self.responses.append(decision)
        self.send({"id": request["id"], "result": {"decision": decision}})

    def close(self):
        if self.process.poll() is None:
            self.process.stdin.close()
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.terminate()
                try:
                    self.process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    self.process.kill()
                    self.process.wait(timeout=5)
        self.process.stdout.close()
        self.thread.join(timeout=5)


class Fixture:
    def __init__(self, workspace):
        self.workspace = workspace
        self.command = None
        self.request_count = 0
        self.pending_call = False
        self.failure = None
        fixture = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_POST(self):
                try:
                    length = int(self.headers.get("Content-Length", "0"))
                    if not 0 < length <= 2_000_000:
                        raise RuntimeError("unexpected fixture request size")
                    raw = self.rfile.read(length)
                    if self.headers.get("Content-Encoding") == "gzip":
                        raw = gzip.decompress(raw)
                    body = json.loads(raw)
                    fixture.request_count += 1
                    if fixture.request_count > 8:
                        raise RuntimeError("local fixture request ceiling exceeded")
                    if fixture.pending_call:
                        fixture.pending_call = False
                        item = {"type": "message", "id": "message_fixture", "role": "assistant",
                                "status": "completed", "phase": "final_answer",
                                "content": [{"type": "output_text", "text": "YO_POLICY_FIXTURE_DONE", "annotations": []}]}
                    else:
                        tools = [nested for tool in body.get("tools", []) for nested in (tool.get("tools", []) if tool.get("type") == "namespace" else [tool])]
                        tool = next((tool for tool in tools if tool.get("name") in ("shell", "exec_command")), None)
                        if tool is None:
                            raise RuntimeError(f"native shell tool absent: path={self.path}, keys={list(body)}, model={body.get('model')}, tools={tools}")
                        properties = tool.get("parameters", {}).get("properties", {})
                        args = {"workdir": str(fixture.workspace), "sandbox_permissions": "require_escalated",
                                "justification": "Execute the exact disposable policy probe only",
                                "prefix_rule": fixture.command}
                        if tool["name"] == "shell":
                            args["command"] = fixture.command
                            args["timeout_ms"] = 10000
                        else:
                            args["cmd"] = shlex.join(fixture.command)
                            args["max_output_tokens"] = 256
                            args["yield_time_ms"] = 1000
                        if properties:
                            args = {key: value for key, value in args.items() if key in properties}
                        fixture.pending_call = True
                        item = {"type": "function_call", "id": f"fc_{fixture.request_count}",
                                "call_id": f"call_{fixture.request_count}", "name": tool["name"],
                                "arguments": json.dumps(args), "status": "completed"}
                    response = {"id": f"resp_fixture_{fixture.request_count}", "object": "response",
                                "status": "completed", "model": body.get("model"), "output": [item],
                                "usage": {"input_tokens": 1, "output_tokens": 1, "total_tokens": 2}}
                    events = [{"type": "response.created", "response": {**response, "status": "in_progress", "output": []}},
                              {"type": "response.output_item.done", "output_index": 0, "item": item},
                              {"type": "response.completed", "response": response}]
                    data = "".join("data: " + json.dumps(event) + "\n\n" for event in events).encode()
                    self.send_response(200)
                    self.send_header("Content-Type", "text/event-stream")
                    self.send_header("Content-Length", str(len(data)))
                    self.end_headers()
                    self.wfile.write(data)
                except Exception as error:
                    fixture.failure = str(error)
                    self.send_error(500)

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)


def enable_loopback():
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as descriptor:
        request = struct.pack("16sH14x", b"lo", 0)
        current = fcntl.ioctl(descriptor, 0x8913, request)
        flags = struct.unpack("16sH14x", current)[1]
        fcntl.ioctl(descriptor, 0x8914, struct.pack("16sH14x", b"lo", flags | 1))


def fixture_argv(codex, address):
    argv = [codex]
    config = {
        "model_provider": '"yo_policy_fixture"', "model": '"gpt-5.5"',
        "model_providers.yo_policy_fixture": '{name="Loopback fixture",base_url=' + json.dumps(address) + ',wire_api="responses",requires_openai_auth=false,request_max_retries=0,stream_max_retries=0,supports_websockets=false}',
        "model_reasoning_effort": '"low"', "approval_policy": '"on-request"',
        "approvals_reviewer": '"user"', "web_search": '"disabled"', "mcp_servers": '{}',
        "project_doc_max_bytes": '0', "developer_instructions": '""',
    }
    for key, value in config.items():
        argv.extend(["-c", key + "=" + value])
    for feature in ["apps", "plugins", "remote_plugin", "multi_agent", "multi_agent_v2", "hooks",
                    "shell_snapshot", "code_mode", "code_mode_host", "code_mode_only",
                    "browser_use", "computer_use", "image_generation", "memories"]:
        argv.extend(["--disable", feature])
    argv.extend(["--enable", "shell_tool", "--enable", "unified_exec", "app-server", "--strict-config", "--listen", "stdio://"])

    return argv


def run(codex):
    enable_loopback()
    assert {name for _, name in socket.if_nameindex()} == {"lo"}
    version = subprocess.check_output([codex, "--version"], text=True).strip()
    if version != "codex-cli 0.154.0":
        raise RuntimeError(f"this validation requires the reviewed exact 0.154.0 executable: {version}")
    with tempfile.TemporaryDirectory(prefix="yo-codex-policy-") as directory:
        root = Path(directory)
        workspace = root / "workspace"
        workspace.mkdir()
        native_home = root / "native"
        native_home.mkdir(mode=0o700)
        marker = root / "policy-marker.txt"
        declined = root / "declined-marker.txt"
        command = [sys.executable, "-c", f"from pathlib import Path; p=Path({str(marker)!r}); p.open('a').write('YO_POLICY_MARKER\\n')"]
        fixture = Fixture(workspace)
        native = None
        try:
            address = f"http://127.0.0.1:{fixture.server.server_port}/v1"
            environment = os.environ.copy()
            # This child-only Codex configuration root isolates native rule writes;
            # the caller's HOME and normal auth/config bytes are never changed.
            environment["CODEX_HOME"] = str(native_home)
            argv = fixture_argv(codex, address)

            def start():
                client = NativeClient(argv, environment, workspace)
                try:
                    client.call("initialize", {"clientInfo": {"name": "yo_policy_validation", "version": "1"},
                                               "capabilities": {"experimentalApi": True}})
                    client.send({"method": "initialized", "params": {}})
                    return client
                except BaseException:
                    client.close()
                    raise

            def turn(client, thread_id, decision):
                fixture.pending_call = False
                result = client.call("turn/start", {"threadId": thread_id, "input": [{"type": "text", "text": "Execute the exact fixture tool once."}]})
                turn_id = result["turn"]["id"]
                approvals = []
                deadline = time.monotonic() + 45
                while time.monotonic() < deadline:
                    event = client.receive(max(0.01, deadline - time.monotonic()))
                    if event.get("method") == "item/commandExecution/requestApproval":
                        assert event["params"]["threadId"] == thread_id and event["params"]["turnId"] == turn_id
                        approvals.append(event["params"])
                        if decision == "persist":
                            prefix = event["params"].get("proposedExecpolicyAmendment")
                            assert prefix == command, ("unexpected proposed scope", prefix)
                            response = {"acceptWithExecpolicyAmendment": {"execpolicy_amendment": prefix}}
                        elif decision == "cancel":
                            offered = event["params"].get("availableDecisions")
                            assert offered is not None and "cancel" in offered, "native host did not offer cancel"
                            response = "cancel"
                        else:
                            raise RuntimeError("persisted exact rule unexpectedly asked again")
                        offered = event["params"].get("availableDecisions")
                        if offered is not None:
                            assert response in offered, "native host did not offer the required decision"
                        client.answer(event, response)
                    if event.get("method") == "turn/completed" and event["params"]["turn"]["id"] == turn_id:
                        return approvals, event["params"]["turn"]["status"]
                raise RuntimeError("native policy Turn did not complete")

            native = start()
            thread_id = native.call("thread/start", {"cwd": str(workspace), "model": "gpt-5.5",
                                                    "modelProvider": "yo_policy_fixture", "sandbox": "workspace-write",
                                                    "approvalPolicy": "on-request"})["thread"]["id"]
            fixture.command = command
            approvals, status = turn(native, thread_id, "persist")
            if len(approvals) != 1 or status != "completed":
                raise RuntimeError(json.dumps({"status": status, "approvals": approvals,
                    "fixture_failure": fixture.failure, "local_requests": fixture.request_count,
                    "recent_native_events": native.events[-8:]})[:10000])
            assert marker.read_text() == "YO_POLICY_MARKER\n"
            policy = native_home / "rules/default.rules"
            original_policy = policy.read_bytes()
            assert b'prefix_rule(' in original_policy and b'decision="allow"' in original_policy
            native.close()
            native = start()
            resumed = native.call("thread/resume", {"threadId": thread_id})
            assert resumed["thread"]["id"] == thread_id
            approvals, status = turn(native, thread_id, "none")
            assert not approvals and status == "completed"
            assert marker.read_text() == "YO_POLICY_MARKER\nYO_POLICY_MARKER\n"
            assert policy.read_bytes() == original_policy
            fixture.command = [sys.executable, "-c", f"from pathlib import Path; Path({str(declined)!r}).write_text('SHOULD_NOT_EXIST')"]
            approvals, status = turn(native, thread_id, "cancel")
            assert len(approvals) == 1 and status == "interrupted" and not declined.exists()
            assert native.responses == ["cancel"]
            assert policy.read_bytes() == original_policy
            assert not fixture.failure, fixture.failure
            assert fixture.request_count == 5, fixture.request_count
            return {"version": version, "persistent_command_allow": "passed", "process_restart_reload": "passed",
                    "one_shot_non_grant_no_rule_write": "passed", "actual_non_grant_decision": native.responses[-1],
                    "non_granted_turn_status": status,
                    "local_fixture_requests": fixture.request_count, "external_model_requests": 0,
                    "normal_user_state": "unchanged", "owned_temporary_state": "removed"}
        finally:
            if native is not None:
                native.close()
            fixture.close()


def main(check=run, description=__doc__):
    parser = argparse.ArgumentParser(description=description)
    parser.add_argument("codex", help="absolute installed Codex 0.154.0 executable")
    parser.add_argument("--isolated-netns", help=argparse.SUPPRESS)
    args = parser.parse_args()
    codex = str(Path(args.codex).resolve(strict=True))
    if args.isolated_netns:
        assert os.readlink('/proc/self/ns/net') != args.isolated_netns, "network isolation was not established"
        def terminate(_signal, _frame):
            raise RuntimeError("isolated validation was interrupted; cleaning owned state")
        signal.signal(signal.SIGTERM, terminate)
        print(json.dumps(check(codex), ensure_ascii=False, indent=2))
    else:
        unshare = shutil.which('unshare')
        if unshare is None:
            parser.error("Linux unshare is required; do not run without network isolation")
        child = subprocess.Popen([unshare, '--user', '--map-root-user', '--net', sys.executable,
                                  str(Path(sys.argv[0]).resolve(strict=True)), codex, '--isolated-netns', os.readlink('/proc/self/ns/net')], start_new_session=True)
        try:
            status = child.wait(timeout=180)
            if status:
                raise subprocess.CalledProcessError(status, child.args)
        except BaseException:
            if child.poll() is None:
                os.killpg(child.pid, signal.SIGTERM)
                try:
                    child.wait(timeout=15)
                except subprocess.TimeoutExpired:
                    os.killpg(child.pid, signal.SIGKILL)
                    child.wait(timeout=5)
            raise


if __name__ == '__main__':
    main()
