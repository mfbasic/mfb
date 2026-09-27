#!/usr/bin/env python3
"""bug-708 regression probe: examples/life quits on a bare 'q' with no Return.

The example's header comment used to claim the terminal "stays in cooked/line
mode" and a key needs a trailing Return to take effect. That was always false —
`term::on()` puts the tty into single-key mode before the game loop starts —
and this probe is the executable proof: build examples/life, spawn it on a
pty, send a single `q` byte with no `\n`/`\r`, and assert the process exits 0
well within a Return-sized wait. This has nothing to do with whether the
header comment's wording matches; it is the underlying behavior the comment
must describe accurately.

Usage: python3 tools/pty-probe/life_quit_no_return.py
Exits 0 and prints "PASS" on success; exits 1 and prints a diagnostic on
failure.
"""
import os
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pty_probe import spawn  # noqa: E402

REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
MFB = os.path.join(REPO_ROOT, "target", "release", "mfb")
BINARY = os.path.join(REPO_ROOT, "examples", "life", "build", "life.out")


def main() -> int:
    subprocess.run([MFB, "build", os.path.join(REPO_ROOT, "examples", "life")],
                    check=True, cwd=REPO_ROOT)

    session = spawn(BINARY, rows=24, cols=80)
    try:
        # Let the app finish its first frame before sending the key.
        deadline = time.monotonic() + 1.0
        while time.monotonic() < deadline:
            session.read_available(0.05)

        session.send(b"q")  # deliberately no \n / \r

        exited_at = None
        wait_until = time.monotonic() + 3.0
        while time.monotonic() < wait_until:
            pid, status = os.waitpid(session.pid, os.WNOHANG)
            if pid != 0:
                exited_at = time.monotonic()
                break
            session.read_available(0.02)

        if exited_at is None:
            print("FAIL: process did not exit within 3s after a bare 'q' "
                  "(no Return) — if this is the intended behavior, the header "
                  "comment's claim would be true; today it should exit almost "
                  "immediately, proving the comment false before the fix and "
                  "confirming unchanged behavior after it.")
            return 1

        if not os.WIFEXITED(status) or os.WEXITSTATUS(status) != 0:
            print(f"FAIL: process exited abnormally (status={status})")
            return 1

        print("PASS: examples/life exited 0 on a bare 'q' with no Return, "
              f"in {exited_at - (wait_until - 3.0):.3f}s")
        return 0
    finally:
        # Session may already be reaped by the waitpid above; close() tolerates that.
        try:
            os.kill(session.pid, 0)
            session.close()
        except ProcessLookupError:
            try:
                os.close(session.fd)
            except OSError:
                pass


if __name__ == "__main__":
    sys.exit(main())
