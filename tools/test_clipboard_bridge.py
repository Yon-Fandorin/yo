import contextlib
import io
import os
from pathlib import Path
import signal
import socket
import stat
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

import clipboard_bridge as bridge


class ClipboardBridgeTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name).resolve()
        self.socket_path = self.directory / "private" / "clipboard.sock"

    def command(self, source):
        script = self.directory / "xclip"
        script.write_text(f"#!{sys.executable}\n" + source)
        script.chmod(0o700)
        return [str(script)]

    def capture(self, source):
        command = self.command(source)
        with patch.object(bridge, "clipboard_command", return_value=command):
            return bridge.capture_png()

    def start_server(self, source):
        self.command(source)
        environment = dict(os.environ, DISPLAY=":test", PATH=str(self.directory))
        environment.pop("WAYLAND_DISPLAY", None)
        process = subprocess.Popen(
            [sys.executable, bridge.__file__, "--socket", str(self.socket_path)],
            env=environment, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
        )

        def cleanup():
            if process.poll() is None:
                process.kill()
            process.communicate(timeout=3)

        self.addCleanup(cleanup)
        deadline = time.monotonic() + 3
        while not self.socket_path.exists() and time.monotonic() < deadline:
            if process.poll() is not None:
                self.fail(process.communicate()[1].decode())
            time.sleep(0.01)
        self.assertTrue(self.socket_path.exists())
        return process

    def connect(self):
        connection = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.addCleanup(connection.close)
        connection.settimeout(3)
        connection.connect(str(self.socket_path))
        return connection

    def test_private_parent_and_socket_are_created(self):
        listener, identity = bridge.bind_socket(self.socket_path)
        try:
            self.assertEqual(stat.S_IMODE(self.socket_path.parent.stat().st_mode), 0o700)
            self.assertEqual(stat.S_IMODE(self.socket_path.stat().st_mode), 0o600)
            self.assertEqual(self.socket_path.stat().st_uid, os.geteuid())
        finally:
            listener.close()
            bridge.remove_owned_socket(self.socket_path, identity)
        self.assertFalse(self.socket_path.exists())

    def test_relative_and_public_parent_rejected_without_chmod(self):
        with self.assertRaisesRegex(bridge.BridgeError, "absolute"):
            bridge.bind_socket("relative.sock")
        self.socket_path.parent.mkdir(mode=0o755)
        with self.assertRaisesRegex(bridge.BridgeError, "0700"):
            bridge.bind_socket(self.socket_path)
        self.assertEqual(stat.S_IMODE(self.socket_path.parent.stat().st_mode), 0o755)
        self.assertFalse(self.socket_path.exists())

    def test_foreign_owner_and_symlink_parent_rejected(self):
        self.socket_path.parent.mkdir(mode=0o700)
        with patch.object(bridge.os, "geteuid", return_value=os.geteuid() + 1):
            with self.assertRaisesRegex(bridge.BridgeError, "current-user-owned"):
                bridge.bind_socket(self.socket_path)
        alias = self.directory / "alias"
        alias.symlink_to(self.socket_path.parent, target_is_directory=True)
        with self.assertRaisesRegex(bridge.BridgeError, "symlinks"):
            bridge.bind_socket(alias / "clipboard.sock")

    def test_intermediate_symlink_rejected_before_creating_parent(self):
        target = self.directory / "target"
        target.mkdir(mode=0o700)
        alias = self.directory / "alias"
        alias.symlink_to(target, target_is_directory=True)
        with self.assertRaisesRegex(bridge.BridgeError, "symlinks"):
            bridge.bind_socket(alias / "private" / "clipboard.sock")
        self.assertFalse((target / "private").exists())
        self.assertTrue(alias.is_symlink())

    def test_writable_ancestor_rejected_before_creating_parent(self):
        ancestor = self.directory / "shared"
        ancestor.mkdir(mode=0o700)
        for mode in [0o702, 0o720, 0o777]:
            ancestor.chmod(mode)
            with self.assertRaisesRegex(bridge.BridgeError, "writable by other users"):
                bridge.bind_socket(ancestor / "private" / "clipboard.sock")
            self.assertFalse((ancestor / "private").exists())
            self.assertEqual(stat.S_IMODE(ancestor.stat().st_mode), mode)

    def test_lexical_parent_traversal_rejected_before_mkdir(self):
        path = self.directory / "missing" / ".." / "private" / "clipboard.sock"
        with self.assertRaisesRegex(bridge.BridgeError, "path components"):
            bridge.bind_socket(path)
        self.assertFalse((self.directory / "missing").exists())
        self.assertFalse(self.socket_path.parent.exists())

    def test_existing_file_and_dangling_symlink_are_preserved(self):
        self.socket_path.parent.mkdir(mode=0o700)
        self.socket_path.write_bytes(b"existing")
        with self.assertRaisesRegex(bridge.BridgeError, "already exists"):
            bridge.bind_socket(self.socket_path)
        self.assertEqual(self.socket_path.read_bytes(), b"existing")
        self.socket_path.unlink()
        self.socket_path.symlink_to("missing")
        with self.assertRaisesRegex(bridge.BridgeError, "already exists"):
            bridge.bind_socket(self.socket_path)
        self.assertTrue(self.socket_path.is_symlink())

    def test_cleanup_preserves_replacement_socket(self):
        listener, identity = bridge.bind_socket(self.socket_path)
        try:
            with self.assertRaisesRegex(bridge.BridgeError, "already exists"):
                bridge.bind_socket(self.socket_path)
            self.socket_path.unlink()
            replacement, replacement_identity = bridge.bind_socket(self.socket_path)
            try:
                bridge.remove_owned_socket(self.socket_path, identity)
                self.assertTrue(self.socket_path.exists())
            finally:
                replacement.close()
                bridge.remove_owned_socket(self.socket_path, replacement_identity)
        finally:
            listener.close()

    def test_exact_limit_succeeds_and_first_excess_byte_reaps_child(self):
        source = f"import os\nos.write(1, {bridge.PNG_SIGNATURE!r} + b'x' * ({bridge.MAX_PNG_BYTES} - 8))\n"
        self.assertEqual(len(self.capture(source)), bridge.MAX_PNG_BYTES)
        pid_path = self.directory / "pid"
        source = (
            "import os,time\n"
            f"open({str(pid_path)!r}, 'w').write(str(os.getpid()))\n"
            f"data = {bridge.PNG_SIGNATURE!r} + b'x' * ({bridge.MAX_PNG_BYTES} - 7)\n"
            "while data:\n    data = data[os.write(1, data):]\n"
            "time.sleep(30)\n"
        )
        with self.assertRaisesRegex(bridge.BridgeError, "exceeds 4 MiB"):
            self.capture(source)
        with self.assertRaises(ProcessLookupError):
            os.kill(int(pid_path.read_text()), 0)

    def test_hanging_capture_is_bounded_and_reaped(self):
        pid_path = self.directory / "pid"
        source = f"import os,time\nopen({str(pid_path)!r}, 'w').write(str(os.getpid()))\ntime.sleep(30)\n"
        started = time.monotonic()
        with patch.object(bridge, "CAPTURE_TIMEOUT", 0.2):
            with self.assertRaisesRegex(bridge.BridgeError, "timed out"):
                self.capture(source)
        self.assertLess(time.monotonic() - started, 1.5)
        with self.assertRaises(ProcessLookupError):
            os.kill(int(pid_path.read_text()), 0)

    def test_non_png_failed_command_and_missing_tool_do_not_expose_output(self):
        for source in ["print('clipboard-secret')\n", "import sys\nsys.stderr.write('clipboard-secret')\nsys.exit(1)\n"]:
            with self.assertRaisesRegex(bridge.BridgeError, "No clipboard PNG") as error:
                self.capture(source)
            self.assertNotIn("clipboard-secret", str(error.exception))
        with patch.object(bridge.sys, "platform", "darwin"), patch.object(bridge.shutil, "which", return_value=None):
            with self.assertRaisesRegex(bridge.BridgeError, "Install pngpaste"):
                bridge.clipboard_command()

    def test_native_command_selection_is_fixed(self):
        cases = [
            ("darwin", {}, ["pngpaste", "-"]),
            ("linux", {"WAYLAND_DISPLAY": "wayland-0"}, ["wl-paste", "--type", "image/png"]),
            ("linux", {"DISPLAY": ":0"}, ["xclip", "-selection", "clipboard", "-t", "image/png", "-o"]),
        ]
        for platform, environment, expected in cases:
            with patch.object(bridge.sys, "platform", platform), patch.dict(os.environ, environment, clear=True), patch.object(bridge.shutil, "which", side_effect=lambda command: "/fake/" + command):
                self.assertEqual(bridge.clipboard_command(), ["/fake/" + expected[0], *expected[1:]])

    @unittest.skipUnless(sys.platform.startswith("linux"), "fake Linux display for process integration")
    def test_capture_waits_for_empty_half_closed_request_and_sigterm_cleans_socket(self):
        marker = self.directory / "captured"
        payload = bridge.PNG_SIGNATURE + b"test-payload"
        process = self.start_server(
            f"import os\nopen({str(marker)!r}, 'w').write('captured')\nos.write(1, {payload!r})\n"
        )
        connection = self.connect()
        time.sleep(0.1)
        self.assertFalse(marker.exists())
        connection.shutdown(socket.SHUT_WR)
        output = bytearray()
        while block := connection.recv(65536):
            output.extend(block)
        self.assertEqual(output, payload)
        self.assertTrue(marker.exists())
        process.send_signal(signal.SIGTERM)
        _, diagnostics = process.communicate(timeout=3)
        self.assertEqual(process.returncode, 0)
        self.assertNotIn(b"test-payload", diagnostics)
        self.assertFalse(self.socket_path.exists())

    def test_request_body_and_timeout_never_read_clipboard(self):
        for body in [b"forbidden", None]:
            server, client = socket.socketpair()
            with server, client, patch.object(bridge, "capture_png") as capture, patch.object(bridge, "SOCKET_TIMEOUT", 0.02), contextlib.redirect_stderr(io.StringIO()):
                if body is not None:
                    client.sendall(body)
                    client.shutdown(socket.SHUT_WR)
                bridge.serve_client(server)
                capture.assert_not_called()

    def test_missing_image_yields_empty_eof_and_actionable_diagnostic(self):
        server, client = socket.socketpair()
        diagnostics = io.StringIO()
        with server, client:
            client.shutdown(socket.SHUT_WR)
            with patch.object(bridge, "capture_png", side_effect=bridge.BridgeError("No clipboard PNG available; copy a screenshot and retry.")), contextlib.redirect_stderr(diagnostics):
                bridge.serve_client(server)
            server.close()
            self.assertEqual(client.recv(1), b"")
        self.assertIn("copy a screenshot", diagnostics.getvalue())


if __name__ == "__main__":
    unittest.main()
