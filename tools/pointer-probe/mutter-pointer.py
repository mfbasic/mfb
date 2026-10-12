#!/usr/bin/env python3
"""mutter-pointer -- move the real pointer of a GNOME (Mutter) Wayland session.

Reads commands, one per line, from stdin:

    home             drive the pointer into the screen's top-left corner
    rel DX DY        move it by (DX, DY) logical pixels
    down [BUTTON]    press a button: left (default), middle or right
    up [BUTTON]      release it
    sleep SECONDS    wait

Wayland gives a client no way to place the pointer or to learn where a window is, and
this needs neither: `home` is a relative move far past the corner, which the
compositor clamps, so every later `rel` is from a known point.

It talks to `org.gnome.Mutter.RemoteDesktop` on the session bus, so it must run inside
the session's environment (`XDG_RUNTIME_DIR`, `DBUS_SESSION_BUS_ADDRESS`), and the
remote-desktop session it opens lives exactly as long as this process.

Needs PyGObject (`python3-gi`). See README.md beside this file.
"""
import sys
import time

from gi.repository import Gio, GLib

NAME = "org.gnome.Mutter.RemoteDesktop"
# Linux input event codes (linux/input-event-codes.h).
BUTTONS = {"left": 0x110, "right": 0x111, "middle": 0x112}


def main():
    bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
    desktop = Gio.DBusProxy.new_sync(bus, 0, None, NAME, "/org/gnome/Mutter/RemoteDesktop", NAME, None)
    (path,) = desktop.call_sync("CreateSession", None, 0, -1, None).unpack()
    session = Gio.DBusProxy.new_sync(bus, 0, None, NAME, path, NAME + ".Session", None)
    session.call_sync("Start", None, 0, -1, None)

    def call(method, signature, *args):
        session.call_sync(method, GLib.Variant(signature, args), 0, -1, None)

    for line in sys.stdin:
        words = line.split()
        if not words:
            continue
        command, args = words[0], words[1:]
        if command == "home":
            call("NotifyPointerMotionRelative", "(dd)", -20000.0, -20000.0)
        elif command == "rel":
            call("NotifyPointerMotionRelative", "(dd)", float(args[0]), float(args[1]))
        elif command in ("down", "up"):
            button = BUTTONS[args[0] if args else "left"]
            call("NotifyPointerButton", "(ib)", button, command == "down")
        elif command == "sleep":
            time.sleep(float(args[0]))
        else:
            sys.exit(f"mutter-pointer: unknown command {command!r}")
    session.call_sync("Stop", None, 0, -1, None)


if __name__ == "__main__":
    main()
