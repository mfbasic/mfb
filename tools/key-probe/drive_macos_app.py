#!/usr/bin/env python3
"""Drive the --app key probe on macOS (tools/key-probe).

    python3 drive_macos_app.py <keyprobe.app> <out-file> [transcript|tui|canvas]

Launches the app's executable (`KEYPROBE_OUT`/`KEYPROBE_SURFACE` in its
environment) and posts the probe's
key sequence to it with `CGEventPostToPid` (Quartz, through ctypes): real
`NSEvent`s through the window server, which works with the session locked,
where System Events keystrokes do not. The posting process needs Accessibility
trust (`AXIsProcessTrusted`). Prints the probe's output file. Standard library
only.
"""
import ctypes
import ctypes.util
import os
import subprocess
import sys
import time

CG = ctypes.CDLL(ctypes.util.find_library("ApplicationServices"))
CG.CGEventCreateKeyboardEvent.restype = ctypes.c_void_p
CG.CGEventCreateKeyboardEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint16, ctypes.c_bool]
CG.CGEventSetFlags.argtypes = [ctypes.c_void_p, ctypes.c_uint64]
CG.CGEventKeyboardSetUnicodeString.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_void_p]
CG.CGEventPostToPid.argtypes = [ctypes.c_int, ctypes.c_void_p]
CG.CFRelease.argtypes = [ctypes.c_void_p]
CG.AXIsProcessTrusted.restype = ctypes.c_bool

SHIFT = 0x20000  # kCGEventFlagMaskShift

# (name, virtual key code, flags, unicode text or None)
KEYS = [
    ("a", 0, 0, None),
    ("Left", 123, 0, None),
    ("Right", 124, 0, None),
    ("Up", 126, 0, None),
    ("Down", 125, 0, None),
    ("Backspace", 51, 0, None),
    ("Delete", 117, 0, None),
    ("Escape", 53, 0, None),
    ("F5", 96, 0, None),
    ("Shift+Left", 123, SHIFT, None),
    ("Home", 115, 0, None),
    ("End", 119, 0, None),
    ("é", 0, 0, "é"),
    ("q", 12, 0, None),
]


def post(pid: int, code: int, down: bool, flags: int, text) -> None:
    event = CG.CGEventCreateKeyboardEvent(None, code, down)
    CG.CGEventSetFlags(event, flags)
    if text is not None:
        units = text.encode("utf-16-le")
        buf = ctypes.create_string_buffer(units)
        CG.CGEventKeyboardSetUnicodeString(event, len(units) // 2, buf)
    CG.CGEventPostToPid(pid, event)
    CG.CFRelease(event)


def main() -> int:
    if len(sys.argv) < 3:
        print(__doc__, file=sys.stderr)
        return 2
    app, out = sys.argv[1], sys.argv[2]
    surface = sys.argv[3] if len(sys.argv) > 3 else "transcript"
    if not CG.AXIsProcessTrusted():
        print("this process is not trusted for Accessibility; keys would not arrive",
              file=sys.stderr)
        return 1
    name = os.path.splitext(os.path.basename(app.rstrip("/")))[0]
    exe = os.path.join(app, "Contents", "MacOS", name)
    if os.path.exists(out):
        os.remove(out)
    env = dict(os.environ, KEYPROBE_OUT=out, KEYPROBE_SURFACE=surface)
    proc = subprocess.Popen([exe], env=env)
    deadline = time.time() + 10
    while not os.path.exists(out) and time.time() < deadline:
        time.sleep(0.05)
    time.sleep(1.0)
    for _name, code, flags, text in KEYS:
        post(proc.pid, code, True, flags, text)
        time.sleep(0.05)
        post(proc.pid, code, False, flags, text)
        time.sleep(0.15)
    try:
        proc.wait(timeout=10)
    except subprocess.TimeoutExpired:
        proc.kill()
        print("probe did not exit", file=sys.stderr)
    with open(out, encoding="utf-8") as f:
        sys.stdout.write(f.read())
    return 0


if __name__ == "__main__":
    sys.exit(main())
