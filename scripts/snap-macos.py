#!/usr/bin/env python3
"""Usage: python snap-macos.py <app> [name]

Launches the macOS application <app> (an app name for `open -a`, or a path to a
.app bundle), waits 1 second for it to draw, then captures a screenshot of its
window and writes it to <name>.png. If we launched the app, we quit it again
afterwards. `name` defaults to "macos_screenshot".

The window is captured by its CoreGraphics window id (via `screencapture -l`),
so only that window is grabbed - not whatever else is on screen behind it -
even if it is partially occluded.

This REQUIRES Screen Recording permission for the terminal (or other process)
running the script: System Settings > Privacy & Security > Screen Recording.
Without it macOS silently strips every window from all screencapture output and
you get only the desktop wallpaper - there is no workaround, so the script
fails loudly rather than saving a misleading wallpaper-only image.
"""

import os
import signal
import subprocess
import sys
import time

# Run under a repo-local venv so the script just works on a fresh checkout
# without touching the system (PEP 668) Python. On first run we create the venv
# and install the deps; every run then re-execs into it. Membership is detected
# via sys.prefix, not the executable path: a venv's python3 is a symlink back to
# the base interpreter, so realpath(executable) matches the base and can't tell
# us whether the venv's site-packages are active.
_VENV_DIR = os.path.join(
    os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
    ".venv-macos",
)
_VENV_PY = os.path.join(_VENV_DIR, "bin", "python3")


def _provision_venv():
    """Create .venv-macos and install everything the script needs."""
    print("snap-macos: first run - creating .venv-macos ...", file=sys.stderr)
    subprocess.check_call([sys.executable, "-m", "venv", _VENV_DIR])
    pip = [_VENV_PY, "-m", "pip", "install", "--quiet"]
    subprocess.check_call(pip + ["--upgrade", "pip"])
    # Quartz gives us the live on-screen window list (owner, pid, geometry).
    subprocess.check_call(pip + ["pyobjc-framework-Quartz"])


if sys.platform != "darwin":
    sys.exit("snap-macos: this script only runs on macOS")

if os.path.realpath(sys.prefix) != os.path.realpath(_VENV_DIR):
    if not os.path.exists(_VENV_PY):
        _provision_venv()
    os.execv(_VENV_PY, [_VENV_PY, os.path.abspath(__file__), *sys.argv[1:]])

import Quartz

WAIT_SECONDS = 1.0
# How long to wait for a cold app to launch and put a window on screen before
# giving up. Separate from WAIT_SECONDS, which is only the post-draw settle.
LAUNCH_TIMEOUT = 15.0


def find_window(app_arg):
    """Return (window_id, owner_pid, bounds) for the app's window, else Nones.

    Matches against each window's owner name in the *live* CoreGraphics window
    list - re-queried on every call, so it sees apps launched after this process
    started (unlike NSWorkspace's run-loop-updated cache). The list is ordered
    front-to-back, so the first matching layer-0, non-zero-size window is the one
    on top for that app.
    """
    want = os.path.splitext(os.path.basename(app_arg.rstrip("/")))[0].lower()
    windows = Quartz.CGWindowListCopyWindowInfo(
        Quartz.kCGWindowListOptionOnScreenOnly
        | Quartz.kCGWindowListExcludeDesktopElements,
        Quartz.kCGNullWindowID,
    )
    for win in windows:
        if win.get("kCGWindowLayer", 0) != 0:
            continue
        owner = (win.get("kCGWindowOwnerName") or "").lower()
        if not owner or (want != owner and want not in owner and owner not in want):
            continue
        bounds = win.get("kCGWindowBounds", {})
        if bounds.get("Width", 0) < 1 or bounds.get("Height", 0) < 1:
            continue
        return win["kCGWindowNumber"], win.get("kCGWindowOwnerPID"), bounds
    return None, None, None


def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__)

    app = sys.argv[1]
    out_base = sys.argv[2] if len(sys.argv) > 2 else "macos_screenshot"
    png_file = out_base + ".png"

    # If the app already has a window up, leave it running at the end; only quit
    # apps we launched ourselves so we never close a window the user was using.
    was_running = find_window(app)[0] is not None

    # A path to a bundle is launched with `open <path>`; a bare app name with
    # `open -a <name>` (which only accepts registered names, not paths).
    if os.path.exists(app):
        subprocess.check_call(["open", app])
    else:
        subprocess.check_call(["open", "-a", app])

    # Wait for the app to put a window on screen.
    end = time.time() + LAUNCH_TIMEOUT
    win_id = pid = None
    while time.time() < end:
        win_id, pid, _ = find_window(app)
        if win_id is not None:
            break
        time.sleep(0.1)

    if win_id is None:
        sys.exit(f"snap-macos: no window found for {app!r} within "
                 f"{LAUNCH_TIMEOUT:.0f}s (did it fail to launch?)")

    # Let the window finish drawing, then re-read it in case it moved/resized.
    time.sleep(WAIT_SECONDS)
    fresh_id, _, _ = find_window(app)
    if fresh_id is not None:
        win_id = fresh_id

    # -l<id> captures just that window. -o omits the drop shadow, -x mutes the
    # shutter sound. This fails without Screen Recording permission - and there
    # is no fallback: every other screencapture mode silently returns only the
    # wallpaper in that case, so we surface the real problem instead.
    win_shot = ["screencapture", "-x", "-o", f"-l{win_id}", png_file]
    rc = subprocess.call(win_shot)

    # Quit the app if we launched it, whether or not the capture succeeded, so
    # we never leave a stray window on screen.
    if not was_running and pid:
        try:
            os.kill(pid, signal.SIGTERM)
        except ProcessLookupError:
            pass

    if rc != 0:
        sys.exit(
            "snap-macos: could not capture the window. Grant Screen Recording "
            "permission to the app running this script (System Settings > "
            "Privacy & Security > Screen Recording), then try again."
        )

    print(f"Saved {png_file}")


if __name__ == "__main__":
    main()
