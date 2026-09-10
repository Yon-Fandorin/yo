#!/usr/bin/env python3
"""Serve local clipboard PNGs through an explicitly launched Unix socket."""

import argparse
import os
from pathlib import Path
import selectors
import shutil
import signal
import socket
import stat
import subprocess
import sys
import time


MAX_PNG_BYTES = 4 * 1024 * 1024
CAPTURE_TIMEOUT = 2.0
SOCKET_TIMEOUT = 2.0
PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"


class BridgeError(Exception):
    pass


def clipboard_command():
    if sys.platform == "darwin":
        command = ["pngpaste", "-"]
    elif sys.platform.startswith("linux") and os.environ.get("WAYLAND_DISPLAY"):
        command = ["wl-paste", "--type", "image/png"]
    elif sys.platform.startswith("linux") and os.environ.get("DISPLAY"):
        command = ["xclip", "-selection", "clipboard", "-t", "image/png", "-o"]
    else:
        raise BridgeError("Run this helper in your local macOS or Linux graphical session.")
    executable = shutil.which(command[0])
    if executable is None:
        raise BridgeError(f"Install {command[0]} on the local clipboard computer, then retry.")
    return [executable, *command[1:]]


def capture_png():
    command = clipboard_command()
    deadline = time.monotonic() + CAPTURE_TIMEOUT
    process = subprocess.Popen(
        command, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL, start_new_session=True,
    )
    data = bytearray()
    try:
        with selectors.DefaultSelector() as selector:
            os.set_blocking(process.stdout.fileno(), False)
            selector.register(process.stdout, selectors.EVENT_READ)
            while True:
                remaining = deadline - time.monotonic()
                if remaining <= 0 or not selector.select(remaining):
                    raise BridgeError("Clipboard capture timed out; copy the image again and retry.")
                block = os.read(process.stdout.fileno(), min(65536, MAX_PNG_BYTES + 1 - len(data)))
                if not block:
                    break
                data.extend(block)
                if len(data) > MAX_PNG_BYTES:
                    raise BridgeError("Clipboard PNG exceeds 4 MiB; copy a smaller image.")
        try:
            status = process.wait(timeout=max(0, deadline - time.monotonic()))
        except subprocess.TimeoutExpired:
            raise BridgeError("Clipboard capture timed out; copy the image again and retry.") from None
        if status != 0 or not data.startswith(PNG_SIGNATURE):
            raise BridgeError("No clipboard PNG available; copy a screenshot or image and retry.")
        return bytes(data)
    finally:
        # Also terminate descendants that inherited the capture pipe.
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        process.wait(timeout=1)
        process.stdout.close()


def serve_client(connection):
    connection.settimeout(SOCKET_TIMEOUT)
    try:
        # An empty, half-closed request is the sole capture trigger.
        if connection.recv(1):
            raise BridgeError("Clipboard requests must contain no body and close their write side.")
        connection.sendall(capture_png())
    except BridgeError as error:
        print(f"clipboard bridge: {error}", file=sys.stderr)
    except (OSError, subprocess.SubprocessError):
        print("clipboard bridge: clipboard command or connection failed; reconnect and retry.", file=sys.stderr)


def bind_socket(path):
    path = Path(path)
    if not path.is_absolute():
        raise BridgeError("--socket must be an absolute path.")
    if ".." in path.parts:
        raise BridgeError("--socket must not contain '..' path components.")
    parent = path.parent
    validate_ancestors(parent)
    try:
        parent.mkdir(mode=0o700)
    except FileExistsError:
        pass
    info = parent.lstat()
    if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.geteuid() or stat.S_IMODE(info.st_mode) != 0o700:
        raise BridgeError("The socket parent must be a current-user-owned directory with mode 0700.")
    if os.path.lexists(path):
        raise BridgeError("Socket path already exists; choose a new path or stop its owner first.")
    listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    identity = None
    previous_umask = os.umask(0o077)
    try:
        listener.bind(str(path))
        info = path.lstat()
        identity = (info.st_dev, info.st_ino)
        path.chmod(0o600)
        listener.listen(4)
        listener.settimeout(0.25)
        return listener, identity
    except BaseException:
        listener.close()
        if identity is not None:
            remove_owned_socket(path, identity)
        raise
    finally:
        os.umask(previous_umask)


def validate_ancestors(parent):
    # Check before mkdir: a private leaf cannot protect lookup through an
    # ancestor another user can replace. Root-owned sticky /tmp is safe because
    # it protects the current user's child from replacement by other users.
    # The filesystem root's observed owner may be mapped inside a user namespace.
    root_owner = Path("/").stat().st_uid
    current_owner = os.geteuid()
    for ancestor in [*reversed(parent.parents), parent]:
        try:
            info = ancestor.lstat()
        except FileNotFoundError:
            if ancestor == parent:
                return
            raise BridgeError("The socket parent must have existing trusted ancestors.") from None
        if not stat.S_ISDIR(info.st_mode):
            raise BridgeError("Socket ancestors must be directories without symlinks.")
        if info.st_uid not in (root_owner, current_owner):
            raise BridgeError("Socket ancestors must be root- or current-user-owned directories.")
        if info.st_mode & 0o022 and not (info.st_uid == root_owner and info.st_mode & stat.S_ISVTX):
            raise BridgeError("Socket ancestors must not be writable by other users, except root-owned sticky directories.")


def remove_owned_socket(path, identity):
    path = Path(path)
    try:
        info = path.lstat()
        if stat.S_ISSOCK(info.st_mode) and (info.st_dev, info.st_ino) == identity:
            path.unlink()
    except FileNotFoundError:
        pass


def serve(path):
    listener, identity = bind_socket(path)
    try:
        print("clipboard bridge: listening; clipboard is read only when requested.", file=sys.stderr)
        while True:
            try:
                connection, _ = listener.accept()
            except socket.timeout:
                continue
            with connection:
                serve_client(connection)
    finally:
        listener.close()
        remove_owned_socket(path, identity)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--socket", required=True, help="Absolute socket path in a private 0700 directory")
    args = parser.parse_args()

    def stop(_signum, _frame):
        raise KeyboardInterrupt

    signal.signal(signal.SIGTERM, stop)
    try:
        serve(args.socket)
    except KeyboardInterrupt:
        return 0
    except BridgeError as error:
        print(f"clipboard bridge: {error}", file=sys.stderr)
        return 1
    except OSError:
        print("clipboard bridge: cannot open the socket; check its directory, permissions, and path length.", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
