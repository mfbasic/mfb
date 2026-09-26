#!/usr/bin/env python3
"""Shared pty-driving helper for console-example regression probes.

Stdlib only (pty/os/select/fcntl/termios/struct/signal/time/re) — no third-party
screen-emulation dependency. Several bug probes (bug-703, bug-708) need to spawn
a `term::`-based example on a real pty (a plain pipe will not do: `term::on()`
puts the tty into raw/single-key mode, which requires an actual pty), feed it
keystrokes on a schedule, and read back what it drew. This module gives them one
shared, reviewed implementation instead of each rolling its own `pty.fork()` +
`select.select()` loop.

Screen model: `render` is a small, width-1-per-codepoint terminal emulator —
just enough to place every printed character at its (row, col) by tracking
`CSI row;col H` cursor moves plus plain-text cursor advance/wrap/`\r`/`\n`.
It is not a full VT100 emulator (it does not special-case wide/combining
glyphs, tab stops, scroll regions, or most non-CUP CSI codes — those are
consumed and otherwise ignored), but it is enough to answer "where is the head
glyph now" or "what does the status line say now" across frames, which is all
these probes need. `render` is always called on the *entire* byte history
captured so far (each `Session` accumulates it in `.raw`), not incrementally,
so a chunk boundary landing mid-escape-sequence or mid-UTF-8-codepoint never
corrupts the model — only the last, possibly-incomplete tail is ever wrong,
and it is completed on the next call once more bytes arrive.
"""
from __future__ import annotations

import fcntl
import os
import pty
import re
import select
import signal
import struct
import termios
import time
from dataclasses import dataclass, field
from typing import Callable, Optional


_CUP_RE = re.compile(rb"\x1b\[(\d+);(\d+)H([^\x1b]*)")

_CSI_FINAL = set(range(0x40, 0x7F))  # '@'..'~'


@dataclass
class Session:
    pid: int
    fd: int
    raw: bytes = field(default_factory=bytes)

    def read_available(self, timeout: float = 0.05) -> bytes:
        """Read whatever is available within `timeout` seconds (possibly empty)."""
        r, _, _ = select.select([self.fd], [], [], timeout)
        if self.fd not in r:
            return b""
        try:
            chunk = os.read(self.fd, 65536)
        except OSError:
            return b""
        self.raw += chunk
        return chunk

    def send(self, data: bytes) -> None:
        os.write(self.fd, data)

    def close(self) -> None:
        try:
            os.kill(self.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        try:
            os.waitpid(self.pid, 0)
        except ChildProcessError:
            pass
        try:
            os.close(self.fd)
        except OSError:
            pass


def spawn(binary: str, args: Optional[list] = None, rows: int = 40, cols: int = 100) -> Session:
    """Fork `binary` onto a fresh pty sized rows x cols and return the Session."""
    pid, fd = pty.fork()
    if pid == 0:
        os.execv(binary, [binary] + (args or []))
        os._exit(1)  # pragma: no cover - only reached if execv fails
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
    return Session(pid=pid, fd=fd)


def extract_cups(raw: bytes):
    """Yield (row, col, text) for every `CSI row;col H` seen in `raw`, in order.

    `text` is the literal bytes written immediately after that cursor move, up
    to the next escape byte (so it may be empty if two escapes are adjacent).
    """
    for m in _CUP_RE.finditer(raw):
        yield int(m.group(1)), int(m.group(2)), m.group(3)


def find_last(raw: bytes, needle: bytes):
    """Return the (row, col) of the last CUP-preceded occurrence of `needle`
    whose written text starts with `needle`, or None if never seen."""
    last = None
    for row, col, text in extract_cups(raw):
        if text.startswith(needle):
            last = (row, col)
    return last


def render(raw: bytes, cols: int = 100):
    """Replay `raw` from a blank terminal and return the resulting grid as a
    dict `{(row, col): codepoint}`, 0-indexed. See the module docstring for
    what this is (and is not) modeling.
    """
    grid: dict = {}
    row = 0
    col = 0
    i = 0
    n = len(raw)
    while i < n:
        b = raw[i]
        if b == 0x1B:  # ESC
            if i + 1 < n and raw[i + 1:i + 2] == b"[":
                # CSI: ESC [ params... final
                j = i + 2
                while j < n and raw[j] not in _CSI_FINAL:
                    j += 1
                if j < n:
                    final = raw[j]
                    params = raw[i + 2:j].decode("ascii", "ignore")
                    if final in (0x48, 0x66):  # 'H' or 'f' -> CUP
                        parts = params.split(";") if params else []
                        try:
                            r = int(parts[0]) if len(parts) > 0 and parts[0] else 1
                        except ValueError:
                            r = 1
                        try:
                            c = int(parts[1]) if len(parts) > 1 and parts[1] else 1
                        except ValueError:
                            c = 1
                        row, col = r - 1, c - 1
                    i = j + 1
                    continue
                else:
                    break  # incomplete trailing escape sequence; stop here
            else:
                # Non-CSI escape (e.g. a lone ESC, or OSC) — skip just the ESC
                # byte; harmless for the sequences these examples emit.
                i += 1
                continue
        elif b in (0x0D,):  # \r
            col = 0
            i += 1
            continue
        elif b in (0x0A,):  # \n
            row += 1
            col = 0
            i += 1
            continue
        elif b < 0x20:  # other control bytes: ignore
            i += 1
            continue
        else:
            # Decode one UTF-8 codepoint starting at i.
            length = 1
            if b >= 0xF0:
                length = 4
            elif b >= 0xE0:
                length = 3
            elif b >= 0xC0:
                length = 2
            if i + length > n:
                break  # incomplete trailing codepoint; stop here
            try:
                ch = raw[i:i + length].decode("utf-8")
            except UnicodeDecodeError:
                ch = "�"
            grid[(row, col)] = ch
            col += 1
            if col >= cols:
                col = 0
                row += 1
            i += length
            continue
        i += 1
    return grid


def find_text(grid: dict, row: int, col_start: int, length: int) -> str:
    """Read `length` cells starting at (row, col_start) out of a `render` grid."""
    return "".join(grid.get((row, col_start + k), " ") for k in range(length))


def find_glyph(grid: dict, ch: str):
    """Return the (row, col) of `ch` in `grid` (there should be at most one
    instance of the glyphs these probes look for), or None."""
    for pos, cell in grid.items():
        if cell == ch:
            return pos
    return None


def run_until(session: Session, deadline: float, on_tick: Optional[Callable[[float, bytes], None]] = None,
              poll: float = 0.02) -> None:
    """Pump `session.read_available` until `deadline` (a time.monotonic() value),
    calling `on_tick(now, new_bytes)` for each non-empty read."""
    while True:
        now = time.monotonic()
        if now > deadline:
            return
        chunk = session.read_available(poll)
        if chunk and on_tick is not None:
            on_tick(now, chunk)
