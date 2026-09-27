#!/usr/bin/env python3
"""Drive the console key probe over a pty (tools/key-probe).

    python3 drive_pty.py <probe-exe> <out-file> [--tui] [--kitty]

Runs the probe with the pty as its controlling terminal and types the probe's
key sequence as a terminal would send it:

- default: the xterm encoding a legacy terminal sends (no releases);
- `--kitty` (implies `--tui`): the pty answers `term::on`'s keyboard-protocol
  query as a kitty terminal and then sends every key in the kitty form
  `term::on` asked for — presses and releases, modifiers, associated text.

With `--tui` and no `--kitty` the pty answers the query as a legacy terminal
(device attributes only). Prints the probe's output file. Standard library only.
"""
import os
import pty
import select
import sys
import time

# (name, legacy xterm bytes, kitty press, kitty release)
KEYS = [
    ("a", b"a", b"\x1b[97;;97u", b"\x1b[97;1:3u"),
    ("Left", b"\x1b[D", b"\x1b[D", b"\x1b[1;1:3D"),
    ("Right", b"\x1b[C", b"\x1b[C", b"\x1b[1;1:3C"),
    ("Up", b"\x1b[A", b"\x1b[A", b"\x1b[1;1:3A"),
    ("Down", b"\x1b[B", b"\x1b[B", b"\x1b[1;1:3B"),
    ("Backspace", b"\x7f", b"\x1b[127u", b"\x1b[127;1:3u"),
    ("Delete", b"\x1b[3~", b"\x1b[3~", b"\x1b[3;1:3~"),
    ("Escape", b"\x1b", b"\x1b[27u", b"\x1b[27;1:3u"),
    ("F5", b"\x1b[15~", b"\x1b[15~", b"\x1b[15;1:3~"),
    ("Shift+Left", b"\x1b[1;2D", b"\x1b[1;2D", b"\x1b[1;2:3D"),
    ("Home", b"\x1b[H", b"\x1b[H", b"\x1b[1;1:3H"),
    ("End", b"\x1b[F", b"\x1b[F", b"\x1b[1;1:3F"),
    ("é", "é".encode(), b"\x1b[233;;233u", b"\x1b[233;1:3u"),
    ("q", b"q", b"\x1b[113;;113u", b"\x1b[113;1:3u"),
]


def main() -> int:
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    kitty = "--kitty" in sys.argv
    tui = kitty or "--tui" in sys.argv
    if len(args) != 2:
        print(__doc__, file=sys.stderr)
        return 2
    exe, out = args
    argv = [exe, out] + (["tui"] if tui else [])
    pid, master = pty.fork()
    if pid == 0:
        os.execv(exe, argv)
    seen = b""

    def drain(seconds: float) -> None:
        nonlocal seen
        end = time.time() + seconds
        while time.time() < end:
            ready, _, _ = select.select([master], [], [], 0.01)
            if ready:
                try:
                    seen += os.read(master, 4096)
                except OSError:
                    return

    if tui:
        deadline = time.time() + 10
        while b"\x1b[?u" not in seen and time.time() < deadline:
            drain(0.02)
        os.write(master, b"\x1b[?0u\x1b[?62;c" if kitty else b"\x1b[?62;c")
    drain(0.5)
    def send(data: bytes) -> None:
        # The probe exits on `q`'s press, so its release may find the pty closed.
        try:
            os.write(master, data)
        except OSError:
            pass

    for _name, legacy, press, release in KEYS:
        if kitty:
            send(press)
            drain(0.05)
            send(release)
        else:
            send(legacy)
        # Past the 25 ms input delay, so a lone Escape is decided on its own.
        drain(0.1)
    deadline = time.time() + 10
    status = None
    while status is None and time.time() < deadline:
        drain(0.05)
        done, raw = os.waitpid(pid, os.WNOHANG)
        if done:
            status = raw
    if status is None:
        os.kill(pid, 9)
        print("probe did not exit", file=sys.stderr)
        return 1
    with open(out, encoding="utf-8") as f:
        sys.stdout.write(f.read())
    return 0


if __name__ == "__main__":
    sys.exit(main())
