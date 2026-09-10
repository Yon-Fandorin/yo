import json
import os
from pathlib import Path
import pty
import select
import struct
import subprocess
import sys
import tempfile
import termios
import time
import unittest
from unittest.mock import patch
import fcntl

import chat_preview as preview


class PreviewTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)

    def export(self, command, **kwargs):
        directory = Path(kwargs["env"]["YO_TUI_PREVIEW_DIR"])
        for scenario in preview.SCENARIOS:
            for width in preview.WIDTHS:
                for palette in preview.PALETTES:
                    for extension in ("ansi", "html"):
                        (directory / f"{scenario}-{palette}-{width}.{extension}").write_text(scenario)
        return subprocess.CompletedProcess(command, 0)

    def build(self):
        with patch.object(preview.subprocess, "run", side_effect=self.export):
            return preview.build(self.directory)

    def test_failed_build_keeps_published_generation(self):
        first = self.build()
        with patch.object(preview.subprocess, "run", return_value=subprocess.CompletedProcess([], 1)):
            with self.assertRaisesRegex(RuntimeError, "previous preview retained"):
                preview.build(self.directory)
        self.assertEqual(preview.current(self.directory), first)

    def test_incomplete_success_does_not_publish(self):
        first = self.build()
        with patch.object(preview.subprocess, "run", return_value=subprocess.CompletedProcess([], 0)):
            with self.assertRaisesRegex(RuntimeError, "Incomplete"):
                preview.build(self.directory)
        self.assertEqual(preview.current(self.directory), first)

    def test_snapshot_remains_bound_to_original_generation(self):
        first = self.build()
        saved = preview.snapshot(self.directory, first, "working", "plain", 40)
        second = self.build()
        self.assertNotEqual(first["generation"], second["generation"])
        self.assertEqual(json.loads((saved / "frame.json").read_text())["generation"], first["generation"])
        self.assertEqual((saved / "frame.ansi").read_text(), "working")

    def test_real_pty_switch_reload_and_restore(self):
        self.build()
        master, slave = pty.openpty()
        try:
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))
            original = termios.tcgetattr(slave)
            process = subprocess.Popen(
                [sys.executable, str(Path(preview.__file__)), "--directory", str(self.directory)],
                stdin=slave, stdout=slave, stderr=slave,
            )
            try:
                def until(expected):
                    output = b""
                    deadline = time.monotonic() + 5
                    while expected not in output and time.monotonic() < deadline:
                        if select.select([master], [], [], 0.1)[0]:
                            output += os.read(master, 65536)
                    self.assertIn(expected, output)
                    return output

                until(b"conversation | 88")
                os.write(master, b"3")
                until(b"working | 88")
                self.build()
                until(b"working | 88")
                os.write(master, b"q")
                until(b"\x1b[?1049l")
                self.assertEqual(process.wait(timeout=5), 0)
                self.assertEqual(termios.tcgetattr(slave), original)
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait()
        finally:
            os.close(master)
            os.close(slave)


if __name__ == "__main__":
    unittest.main()
