import base64
import ctypes
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest


CAPTURE = Path(__file__).resolve().parents[1] / "crates/yo-cli/src/execution/image/clipboard/ssh_capture.py"
PNG = base64.b64decode(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg=="
)


@unittest.skipUnless(os.name == "posix", "remote clipboard supervisor requires Unix")
class SshClipboardCaptureTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.subreaper = False
        if sys.platform.startswith("linux"):
            # Adopt only this suite's orphaned fixture descendants so tests can
            # reap them explicitly even when the container's PID 1 does not.
            cls.libc = ctypes.CDLL(None, use_errno=True)
            previous = ctypes.c_int()
            if cls.libc.prctl(37, ctypes.byref(previous), 0, 0, 0) == 0:
                cls.previous_subreaper = previous.value
                cls.subreaper = cls.libc.prctl(36, 1, 0, 0, 0) == 0

    @classmethod
    def tearDownClass(cls):
        if cls.subreaper:
            cls.libc.prctl(36, cls.previous_subreaper, 0, 0, 0)

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="yo-ssh-clipboard-test-")
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name).resolve()
        self.pids = self.directory / "owned-pids.json"
        self.called = self.directory / "reader-called"

    def start(self, source, maximum=None, reader="macos"):
        command = self.directory / "pngpaste"
        command.write_text(
            f"#!{sys.executable}\n"
            "import json,os,signal,sys,time\n"
            f"open({str(self.called)!r}, 'w').write('called')\n"
            "assert sys.argv[1:] == ['-']\n"
            + source
        )
        command.chmod(0o700)
        process = subprocess.Popen(
            [sys.executable, str(CAPTURE), reader, str(len(PNG) if maximum is None else maximum)],
            env=dict(os.environ, PATH=str(self.directory)), stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True,
        )
        self.addCleanup(self.cleanup_process, process)
        return process

    def owned_pids(self):
        try:
            return json.loads(self.pids.read_text())
        except FileNotFoundError:
            return {}

    def reap_descendant(self, pid):
        deadline = time.monotonic() + 0.7
        while time.monotonic() < deadline:
            try:
                waited, _ = os.waitpid(pid, os.WNOHANG)
            except ChildProcessError:
                return
            if waited == pid:
                return
            time.sleep(0.01)
        self.fail("Owned fixture descendant was not terminated and reaped")

    def cleanup_process(self, process):
        owned = self.owned_pids()
        if owned.get("reader"):
            try:
                os.killpg(owned["reader"], signal.SIGKILL)
            except ProcessLookupError:
                pass
        if process.poll() is None:
            process.terminate()
        try:
            process.communicate(timeout=0.7)
        except subprocess.TimeoutExpired:
            process.kill()
            process.communicate(timeout=0.7)
        if self.subreaper and owned.get("descendant"):
            self.reap_descendant(owned["descendant"])

    def wait_for_reader(self, process):
        deadline = time.monotonic() + 1
        while time.monotonic() < deadline:
            if self.pids.exists():
                return self.owned_pids()
            if process.poll() is not None:
                self.fail("Supervisor exited before the owned reader was ready")
            time.sleep(0.005)
        self.fail("Owned reader did not start within one second")

    def hanging_reader(self, descendant=False, close_stdout=False):
        source = "descendant = None\n"
        if descendant:
            source += (
                "descendant = os.fork()\n"
                "if descendant == 0:\n"
                "    time.sleep(30)\n"
                "    os._exit(0)\n"
            )
        temporary = str(self.pids) + ".new"
        source += (
            f"with open({temporary!r}, 'w') as output:\n"
            "    json.dump({'reader':os.getpid(), 'descendant':descendant}, output)\n"
            f"os.replace({temporary!r}, {str(self.pids)!r})\n"
            f"os.write(1, {PNG[:16]!r})\n"
        )
        if close_stdout:
            source += "os.close(1)\n"
        return source + "time.sleep(30)\n"

    def assert_failed_cleanly(self, process, timeout=3.5):
        output, diagnostics = process.communicate(timeout=timeout)
        self.assertEqual(process.returncode, 1)
        self.assertEqual(output, b"", "Partial clipboard bytes escaped before successful capture")
        self.assertEqual(diagnostics, b"", "Remote diagnostics escaped the capture boundary")

    def assert_owned_children_gone(self, owned):
        if self.subreaper and owned.get("descendant"):
            self.reap_descendant(owned["descendant"])
        for pid in owned.values():
            if pid is not None:
                with self.assertRaises(ProcessLookupError):
                    os.kill(pid, 0)

    def test_exact_limit_returns_the_complete_png(self):
        process = self.start(f"os.write(1, {PNG!r})\n")
        output, diagnostics = process.communicate(timeout=2)
        self.assertEqual(process.returncode, 0)
        self.assertEqual(output, PNG)
        self.assertEqual(diagnostics, b"")

    def test_first_excess_byte_rejects_all_output(self):
        process = self.start(f"os.write(1, {PNG + b'x'!r})\n")
        self.assert_failed_cleanly(process, timeout=2)

    def test_nonzero_exit_after_partial_png_publishes_nothing(self):
        process = self.start(
            f"os.write(1, {PNG[:16]!r})\n"
            "os.write(2, b'private-reader-diagnostic')\n"
            "sys.exit(7)\n"
        )
        self.assert_failed_cleanly(process, timeout=2)

    def test_empty_clipboard_publishes_nothing(self):
        self.assert_failed_cleanly(self.start("sys.exit(0)\n"), timeout=2)

    def test_timeout_kills_reader_and_descendant_holding_output_pipe(self):
        if not self.subreaper:
            self.skipTest("Descendant cleanup requires Linux subreaper support")
        started = time.monotonic()
        process = self.start(self.hanging_reader(descendant=True, close_stdout=True))
        owned = self.wait_for_reader(process)
        self.assert_failed_cleanly(process)
        self.assert_owned_children_gone(owned)
        self.assertLess(time.monotonic() - started, 4)

    def test_closed_output_pipe_does_not_hide_a_hanging_reader(self):
        started = time.monotonic()
        process = self.start(self.hanging_reader(close_stdout=True))
        owned = self.wait_for_reader(process)
        self.assert_failed_cleanly(process)
        self.assert_owned_children_gone(owned)
        self.assertLess(time.monotonic() - started, 4)

    def test_sigterm_during_capture_kills_owned_reader_and_descendant(self):
        self.check_interruption(signal.SIGTERM)

    def test_sighup_during_capture_kills_owned_reader_and_descendant(self):
        self.check_interruption(signal.SIGHUP)

    def check_interruption(self, signum):
        if not self.subreaper:
            self.skipTest("Descendant cleanup requires Linux subreaper support")
        process = self.start(self.hanging_reader(descendant=True))
        owned = self.wait_for_reader(process)
        process.send_signal(signum)
        self.assert_failed_cleanly(process, timeout=2)
        self.assert_owned_children_gone(owned)

    def test_arbitrary_reader_is_rejected_without_execution(self):
        payload = "/bin/sh -c 'touch " + str(self.directory / "unexpected") + "'"
        process = self.start("sys.exit(0)\n", reader=payload)
        self.assert_failed_cleanly(process, timeout=2)
        self.assertFalse(self.called.exists())
        self.assertFalse((self.directory / "unexpected").exists())


if __name__ == "__main__":
    unittest.main()
