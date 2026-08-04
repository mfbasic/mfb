#!/usr/bin/env python3
"""Usage: python snap-term.py <app> [name]

Launches <app> in a pty, streams it into a real terminal emulator
(xterm.js in headless Chromium), waits 1 second, then writes:
  <name>.png  - screenshot of the rendered terminal
  <name>.bin  - the raw byte stream the app emitted to the pty
`name` defaults to "pty_screenshot".
"""

import base64
import os
import signal
import sys
import time

# Re-exec under the repo-local venv if it exists and we're not already in it,
# so `./scripts/snap-term.py` works regardless of the invoking interpreter.
# Detect membership via sys.prefix, not the executable path: a venv's python3
# is a symlink back to the base interpreter, so realpath(executable) matches
# the base and can't tell us whether the venv's site-packages are active.
_VENV_DIR = os.path.join(
    os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
    ".venv-term",
)
_VENV_PY = os.path.join(_VENV_DIR, "bin", "python3")
if os.path.exists(_VENV_PY) and os.path.realpath(sys.prefix) != os.path.realpath(_VENV_DIR):
    os.execv(_VENV_PY, [_VENV_PY, os.path.abspath(__file__), *sys.argv[1:]])

from playwright.sync_api import sync_playwright

COLS, ROWS = 80, 24
WAIT_SECONDS = 1.0

HTML = """
<!doctype html><html><head>
<link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/xterm@5.3.0/css/xterm.css">
<script src="https://cdn.jsdelivr.net/npm/xterm@5.3.0/lib/xterm.min.js"></script>
<script src="https://cdn.jsdelivr.net/npm/xterm-addon-unicode11@0.6.0/lib/xterm-addon-unicode11.min.js"></script>
<style>body{{margin:0;background:#000}}</style>
</head><body><div id="t"></div>
<script>
  const term = new Terminal({{cols: {cols}, rows: {rows}, allowProposedApi: true}});
  // xterm.js defaults to the Unicode 6 width table, where emoji are width 1;
  // the unicode11 addon uses Unicode 11 widths so emoji occupy 2 cells like a
  // real terminal, keeping box borders aligned. Activate BEFORE any writes.
  const U11 = window.Unicode11Addon.Unicode11Addon || window.Unicode11Addon;
  term.loadAddon(new U11());
  term.unicode.activeVersion = '11';
  term.open(document.getElementById('t'));
  window.writeToTerm = (b64) =>
    term.write(Uint8Array.from(atob(b64), c => c.charCodeAt(0)));
</script></body></html>
""".format(cols=COLS, rows=ROWS)


def spawn(command):
    """Spawn `command` in a pty. Returns (read_fn, kill_fn)."""
    if sys.platform == "win32":
        from winpty import PtyProcess

        proc = PtyProcess.spawn(command, dimensions=(ROWS, COLS))

        def read(timeout):
            end = time.time() + timeout
            while time.time() < end:
                try:
                    data = proc.read(65536)
                except EOFError:
                    return
                if data:
                    yield data.encode("utf-8", "replace")
                else:
                    time.sleep(0.01)

        def kill():
            proc.terminate()

        return read, kill

    import fcntl
    import pty
    import select
    import struct
    import termios

    pid, fd = pty.fork()
    if pid == 0:
        os.environ["TERM"] = "xterm-256color"
        os.execvp(command[0], command)

    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))

    def read(timeout):
        end = time.time() + timeout
        while time.time() < end:
            r, _, _ = select.select([fd], [], [], max(0, end - time.time()))
            if not r:
                return
            try:
                yield os.read(fd, 65536)
            except OSError:
                return

    def kill():
        try:
            os.kill(pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        os.close(fd)

    return read, kill


def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__)

    command = [sys.argv[1]]
    out_base = sys.argv[2] if len(sys.argv) > 2 else "pty_screenshot"
    png_file = out_base + ".png"
    bin_file = out_base + ".bin"

    read, kill = spawn(command)
    raw = bytearray()

    try:
        with sync_playwright() as p:
            browser = p.chromium.launch()
            page = browser.new_page(viewport={"width": 660, "height": 420})
            page.set_content(HTML)

            for chunk in read(WAIT_SECONDS):
                raw.extend(chunk)
                page.evaluate(
                    "b64 => window.writeToTerm(b64)",
                    base64.b64encode(chunk).decode(),
                )

            page.locator("#t").screenshot(path=png_file)
            browser.close()
    finally:
        kill()

    with open(bin_file, "wb") as f:
        f.write(raw)

    print(f"Saved {png_file} and {bin_file}")


if __name__ == "__main__":
    main()
    