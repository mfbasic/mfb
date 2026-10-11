# pointer-probe

Puts the REAL pointer somewhere, for a test of a real window. One driver per window
system, because none of them shares a way to do it:

| File | Platform | How it moves the pointer |
|---|---|---|
| `pointer-probe.c` | macOS | `CGWarpMouseCursorPosition` + a posted `CGEvent` |
| `mutter-pointer.py` | GNOME on Wayland | `org.gnome.Mutter.RemoteDesktop` on the session bus |
| `windows-pointer.ps1` | Windows | `SetCursorPos` + `mouse_event` |

## Why it exists

A canvas window's pointer handlers publish the pointer to the graphics state
(plan-172-J; `mfb spec app canvas`, "The pointer, as the graphics thread sees it").
Every other test of that scripts the graphics thread's words directly
(`MFB_CANVAS_POINTER`) or calls the platform's callback headless
(`MFB_CANVAS_POINTER_X` / `_Y`, `scripts/test-canvas-pointer.sh`). Those prove the
words, their reader and the callback — and nothing about whether the window system
DELIVERS a pointer event to the callback, with the arguments it expects. Nothing
already in the tree could move a pointer: `osascript`'s System Events sends keys and
sets window attributes, and a headless run has no window to be over.

It is not idle caution. The first real windows found three things no headless run
could:

- on GNOME Wayland, GDK's held-button state bits for the right and middle buttons are
  in the opposite order to its button numbers (`linux_gtk/pointer.rs`,
  `POINTER_HELD_SYMBOL`);
- on GTK, a canvas program's window had no mouse controllers at all, so
  `canvas::pollMouse` reported nothing (bug-1043);
- on GTK, the right and middle buttons were never reported (bug-1044).

What a run reads back is the `pointer=x,y,buttons,inside` field of a `--debug` build's
`MFB_CANVAS_STATS` line, so the program under test must keep presenting: the pointer
signals no redraw of its own.

## macOS: `pointer-probe.c`

    clang -O1 -o pointer-probe tools/pointer-probe/pointer-probe.c -framework ApplicationServices

    pointer-probe window <pid>     "x y width height" of <pid>'s frontmost window
    pointer-probe move <x> <y>     move the pointer there, posting the mouse-moved event
    pointer-probe down <x> <y>     press the left button there
    pointer-probe drag <x> <y>     move there with the left button held
    pointer-probe up <x> <y>       release it there
    pointer-probe where            the pointer's "x y"
    pointer-probe trusted          1 if this process may post events, else 0

All coordinates are global display points, origin top-left — what
`CGWindowListCopyWindowInfo` reports and `CGEventCreateMouseEvent` takes.

Used by `scripts/test-macapp.sh`, case 6c, which builds it into its work directory.

- **It needs Accessibility permission** for the process that runs it (the terminal, or
  whatever launched the shell). `pointer-probe trusted` says whether it has it; without
  it the pointer still moves (the warp needs no permission) but no event is posted, and
  a view's `mouseMoved:` does not fire.
- **A window opened from a shell comes up behind the frontmost app.** The script's leg
  brings its window forward (System Events, `set frontmost`) before moving the pointer;
  without that the pointer is over somebody else's window and nothing arrives.

## GNOME on Wayland: `mutter-pointer.py`

Reads commands from stdin — `home`, `rel DX DY`, `down|up [left|middle|right]`,
`sleep SECONDS` — and plays them through a Mutter remote-desktop session, which lasts
as long as the script. Needs PyGObject (`python3-gi`), and must run in the session's
environment:

    export XDG_RUNTIME_DIR=/run/user/$(id -u) WAYLAND_DISPLAY=wayland-0 \
           DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/$(id -u)/bus
    printf 'home\nrel 840 540\nsleep 1\ndown right\nsleep 1\nup right\n' | python3 mutter-pointer.py

Start the program under test in the same environment, as an ordinary background
process, and give it several seconds to map its window before driving.

- **Wayland tells a client neither where the pointer is nor where its window is.**
  `home` drives the pointer into the screen's top-left corner (a relative move the
  compositor clamps), so every later `rel` is from a known point. For the window: put
  the program in fullscreen (`app::setFullscreen(TRUE)`), where screen and surface
  coincide, or sweep the pointer and read the window's origin off the first `pointer=`
  that reports inside. A window that is not fullscreen does not always open in the same
  place — a run that sees only `pointer=0,0,0,0` missed it.
- **"Session creation inhibited"** means the session's screen shield is up (the box sat
  idle). Lift it first:
  `gdbus call --session --dest org.gnome.ScreenSaver --object-path /org/gnome/ScreenSaver --method org.gnome.ScreenSaver.SetActive false`

Box 2226 (`.ai/remote_systems.md`) has such a session.

## Windows: `windows-pointer.ps1`

Starts the program, finds its window's client origin, and plays a fixed sequence: in,
left press, drag, release, right click, middle click, out. It must run inside the
logged-in desktop session, which from ssh is a scheduled task created with `/it`
(`.ai/remote_systems.md`, "A visible window from ssh") running a `.cmd` like:

    set MFB_CANVAS_GPU=0
    set MFB_CANVAS_STATS=C:\work\stats.txt
    powershell -ExecutionPolicy Bypass -File C:\work\windows-pointer.ps1 -Exe C:\work\app.exe > C:\work\ps.out 2>&1

- **`MFB_CANVAS_GPU=0` is not optional today.** With the GPU renderer a real window on
  box 206 writes no stats line at all (bug-1045), so there is nothing to read back.
- Each step is held 2.5 s, because a `--debug` software frame of the whole window is
  slow and a shorter hold can fall between two stats lines.

## What all three do to the machine

They move the real pointer and click. The macOS leg puts the pointer back and only
presses inside the window it opened — but while it runs, the machine is not yours, which
is why that leg is behind `MFB_MACAPP_GUI=1` like every other leg that drives the window
server. The other two run on test boxes.
