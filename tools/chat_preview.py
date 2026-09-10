#!/usr/bin/env python3
"""Developer-only, backend-free chat visual feedback loop (Unix terminals)."""
import argparse
import json
import os
from pathlib import Path
import select
import shutil
import subprocess
import sys
import tempfile
import termios
import time
import tty

REPOSITORY = Path(__file__).resolve().parents[1]
DEFAULT_DIRECTORY = REPOSITORY / "target" / "chat-preview"
SCENARIOS = ("welcome", "conversation", "working", "markdown", "tables", "diff", "tools", "error", "interrupted", "draft", "history", "commands", "long-tools", "multi-turn", "approval", "interview", "syntax", "charts", "images", "media-errors")
WIDTHS = (20, 40, 88)
PALETTES = ("color", "light", "mono", "light-indexed", "plain")


def build(directory):
    """Publish a complete generation only after the real renderer test passes."""
    directory.mkdir(parents=True, exist_ok=True)
    generation = Path(tempfile.mkdtemp(prefix="generation-", dir=directory))
    command = ["cargo", "test", "--locked", "-p", "yo-tui", "chat_preview"]
    env = dict(os.environ, YO_TUI_PREVIEW_DIR=str(generation))
    with (generation / "build.log").open("wb") as log:
        result = subprocess.run(command, cwd=REPOSITORY, env=env,
                                stdin=subprocess.DEVNULL, stdout=log, stderr=log)
    if result.returncode:
        raise RuntimeError(f"Build failed; previous preview retained. Log: {generation / 'build.log'}")
    for scenario in SCENARIOS:
        for width in WIDTHS:
            for palette in PALETTES:
                for extension in ("ansi", "html"):
                    if not (generation / f"{scenario}-{palette}-{width}.{extension}").is_file():
                        raise RuntimeError(f"Incomplete preview: {generation}")
    manifest = {"generation": generation.name, "built_at": time.strftime("%H:%M:%S")}
    pending = generation / "publish.json"
    pending.write_text(json.dumps(manifest), encoding="utf-8")
    os.replace(pending, directory / "current.json")
    return manifest


def current(directory):
    manifest = json.loads((directory / "current.json").read_text(encoding="utf-8"))
    name = manifest["generation"]
    if not isinstance(name, str) or Path(name).name != name or not name.startswith("generation-"):
        raise ValueError("Invalid preview generation")
    return manifest


def snapshot(directory, manifest, scenario, palette, width):
    destination = Path(tempfile.mkdtemp(prefix="snapshot-", dir=directory))
    stem = f"{scenario}-{palette}-{width}"
    for extension in ("ansi", "html"):
        shutil.copyfile(directory / manifest["generation"] / f"{stem}.{extension}",
                        destination / f"frame.{extension}")
    metadata = dict(manifest, scenario=scenario, palette=palette, width=width)
    (destination / "frame.json").write_text(json.dumps(metadata), encoding="utf-8")
    return destination


def paint(directory, manifest, scenario, palette, width, notice):
    size = os.get_terminal_size(sys.stdout.fileno())
    if size.columns < width or size.lines < 28:
        output = (f"Preview needs {width}x28; terminal is {size.columns}x{size.lines}.\r\n"
                  "Press w for a smaller width, resize the pane, or q to exit.")
        payload = output.encode()
    else:
        stem = f"{scenario}-{palette}-{width}"
        payload = (directory / manifest["generation"] / f"{stem}.ansi").read_bytes()
        status = f"yo preview | {scenario} | {width} cols | {palette} | built {manifest['built_at']}"
        keys = "[/] scene  1–9 jump  w width  c theme  b build  r reload  s snapshot  q exit"
        # Viewer chrome is outside the 22-row product frame. No embedded shell or model.
        payload += (f"\x1b[0m\x1b[25;1H{status[:size.columns]}\r\n"
                    f"{keys[:size.columns]}\r\n{notice[:size.columns]}").encode()
    sys.stdout.buffer.write(b"\x1b[0m\x1b[2J\x1b[H" + payload)
    sys.stdout.buffer.flush()


def view(directory):
    if not sys.stdin.isatty() or not sys.stdout.isatty():
        raise RuntimeError("Interactive preview requires a TTY; use --build-only for export.")
    manifest = current(directory)
    scenario, palette, width = "conversation", "color", 88
    notice = "Static real-renderer fixtures; no model calls. External builds auto-reload."
    fd = sys.stdin.fileno()
    saved = termios.tcgetattr(fd)
    try:
        tty.setraw(fd)
        sys.stdout.write("\x1b[?1049h\x1b[?25l")
        sys.stdout.flush()
        previous = None
        while True:
            try:
                manifest = current(directory)
            except (OSError, ValueError) as error:
                notice = f"Reload failed; keeping previous frame: {error}"
            identity = (manifest["generation"], scenario, palette, width,
                        os.get_terminal_size(sys.stdout.fileno()), notice)
            if identity != previous:
                paint(directory, manifest, scenario, palette, width, notice)
                previous = identity
            if not select.select([fd], [], [], 0.25)[0]:
                continue
            key = os.read(fd, 1)
            if key in (b"q", b"\x03", b"\x04", b""):
                break
            if key in (b"1", b"2", b"3", b"4", b"5", b"6", b"7", b"8", b"9"):
                scenario = SCENARIOS[int(key) - 1]
            elif key in (b"[", b"]"):
                step = 1 if key == b"]" else -1
                scenario = SCENARIOS[(SCENARIOS.index(scenario) + step) % len(SCENARIOS)]
            elif key == b"w":
                width = {88: 40, 40: 20, 20: 88}[width]
            elif key == b"c":
                palette = PALETTES[(PALETTES.index(palette) + 1) % len(PALETTES)]
            elif key == b"r":
                previous = None
            elif key == b"s":
                path = snapshot(directory, manifest, scenario, palette, width)
                notice = f"Saved: {path}"
            elif key == b"b":
                paint(directory, manifest, scenario, palette, width, "Building; output goes to build.log...")
                try:
                    manifest = build(directory)
                    notice = "Build passed. New frame loaded."
                except (OSError, RuntimeError) as error:
                    notice = str(error)
    finally:
        try:
            sys.stdout.write("\x1b[0m\x1b[?25h\x1b[?1049l")
            sys.stdout.flush()
        finally:
            termios.tcsetattr(fd, termios.TCSADRAIN, saved)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--build-only", action="store_true", help="Build and atomically refresh any open viewer")
    parser.add_argument("--directory", type=Path, default=DEFAULT_DIRECTORY)
    args = parser.parse_args()
    directory = args.directory.resolve()
    if args.build_only or not (directory / "current.json").exists():
        manifest = build(directory)
        print(json.dumps(dict(manifest, directory=str(directory))))
    if not args.build_only:
        view(directory)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, RuntimeError) as error:
        print(f"chat-preview: {error}", file=sys.stderr)
        sys.exit(1)
