"""Fixed remote clipboard operation, embedded by the Rust SSH reader."""

import os
import selectors
import signal
import subprocess
import sys
import time


def interrupted(signum, frame):
    raise InterruptedError("clipboard capture interrupted")


def capture(command, maximum):
    process = None
    try:
        process = subprocess.Popen(command, stdin=subprocess.DEVNULL,
                                   stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                                   start_new_session=True)
        data = bytearray()
        deadline = time.monotonic() + 2
        os.set_blocking(process.stdout.fileno(), False)
        with selectors.DefaultSelector() as selector:
            selector.register(process.stdout, selectors.EVENT_READ)
            while True:
                remaining = deadline - time.monotonic()
                if remaining <= 0 or not selector.select(remaining):
                    raise TimeoutError("clipboard capture deadline")
                block = os.read(process.stdout.fileno(), min(65536, maximum + 1 - len(data)))
                if not block:
                    break
                data.extend(block)
                if len(data) > maximum:
                    raise ValueError("clipboard source limit")
        status = process.wait(timeout=max(0, deadline - time.monotonic()))
        if status or not data:
            raise ValueError("clipboard reader failed")
        return data
    finally:
        if process is not None:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait(timeout=1)
            process.stdout.close()


def main():
    readers = {
        "macos": ["pngpaste", "-"],
        "wayland": ["wl-paste", "--no-newline", "--type", "image/png"],
        "x11": ["xclip", "-selection", "clipboard", "-t", "image/png", "-o"],
    }
    for signum in (signal.SIGALRM, signal.SIGHUP, signal.SIGTERM, signal.SIGINT):
        signal.signal(signum, interrupted)
    signal.alarm(3)
    try:
        reader, maximum = sys.argv[1:]
        data = capture(readers[reader], int(maximum))
        sys.stdout.buffer.write(data)
        sys.stdout.buffer.flush()
        return 0
    except (OSError, ValueError, KeyError, subprocess.SubprocessError):
        # Never expose clipboard bytes or raw remote diagnostics on the terminal.
        return 1
    finally:
        signal.alarm(0)


if __name__ == "__main__":
    sys.exit(main())
