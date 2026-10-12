# bug-1015: on GTK, `term::pollMouse` sees no mouse events unless the program also reads stdin

Last updated: 2026-10-11
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (platform parity)

Status: Open
Regression Test: (proposed) a GTK app-mode row in `scripts/test-appimage.sh`: `term::enableMouse(TRUE)` + a `term::pollMouse` loop with NO stdin read, driven by xdotool click/move/scroll under Xvfb, asserting at least one event per action

**Owner decision (2026-10-10): GTK mouse events come from the window, like macOS and Windows.**
`term::pollMouse` must return the events the window received, whether or not the program ever
reads stdin.

On Linux GTK `--app` builds the mouse handlers write an SGR mouse report into the terminal input
stream, and the decoder that turns reports into `MouseEvent`s runs only on the stdin read path
(`io::readKey` and friends). A program that polls the mouse without reading stdin never decodes
anything, so `term::pollMouse` returns `MouseKind.None` forever.

**Correct behavior:** the GTK window's press/release/motion/scroll events reach the mouse event
queue directly (or `pollMouse` drains and decodes pending input itself), so the same program
behaves the same on GTK, macOS and Windows. Check how the macOS and Windows backends feed the
queue and make GTK match.

## Failing Reproduction

Measured by the coordinator on 2026-10-10 on box 2226 (Debian 12 aarch64 glibc, GTK4, Xvfb +
xdotool), with bs18-agent-i's compiler (bug-982 fixed, so the app no longer crashes). The program
is bug-982's repro: `term::on`, `term::enableMouse(TRUE)`, then 40 × (`term::pollMouse`, log an
`event` line when the kind is not `None`, `os::sleep(100)`), then `done`. It never reads stdin.

| action | `event` lines | `done` |
|---|---|---|
| `xdotool click 1` | 0 | 1 |
| `xdotool mousemove …` | 0 | 1 |
| `xdotool click 4` (scroll) | 0 | 1 |

bs18-agent-i measured the same program with one `io::readKey(0)` added per loop: 2, 2 and 3 events.
(The coordinator has not re-run that variant.)

Expected: at least one `event` line per action.

## Windows does the same (measured 2026-10-11, plan-172-M)

The owner decision above takes Windows as the model GTK should match. In a real canvas
window on box 206 (Windows 11) it behaves as GTK does: `canvas::pollMouse` reports
nothing until the program also reads the keyboard.

`bugs/repro/bug-1015/overid.mfb` presents a small scene, calls `canvas::enableMouse(TRUE)`
and polls `canvas::pollMouse` for 14 s, appending every event to a file; with
`READ_KEYS=1` it also calls `io::readKey(0)` once a loop. `overid.ps1` starts it, puts
the REAL cursor on four client points with `SetCursorPos` and presses and releases the
left button at each. Cross-built `--app --target windows-x86_64` at `8bb3e9a60` and run
inside the logged-in desktop session (a scheduled task with `/it`):

| `READ_KEYS` | what the program recorded |
|---|---|
| unset | `started 384x261`, then `moves 0` — no move, no press, no release in 14 s |
| `1` | twelve events, a `move`, a `down` and an `up` at each of the four points |

The window received every message both times: the pointer words the graphics thread
reads (plan-172-J) are written by the same `WndProc` and were moving. The cause is the
one above — `src/target/win_x86_64/app/mouse.rs` writes SGR reports to the worker's
input pipe, and the only decoder is in the `io` read family
(`src/codegen/builtins/io/gen_read_family.rs`, the pump around
`mouse_decode::emit_drain_pending`); `canvas::pollMouse` only takes from the ring
(`src/codegen/io/mouse/ring.rs:emit_dequeue`). So the fix that makes `pollMouse` drain
and decode pending input itself fixes every platform at once, and "make GTK match
Windows" is not a fix.

`mfb man canvas pollMouse` and `enableMouse` say nothing about needing a key read.

## Related

- bug-982 (the crash these handlers used to cause; fixed). bug-985 (a `setMode`-built window
  attaches no mouse controllers at all): fix this one first or together, since both end in
  "pollMouse sees nothing".
- macOS: bs18-agent-i could not measure synthetic mouse delivery on the host at all (no IMP
  reached `write`, `screencapture` returned black), so macOS delivery is unverified, not known good.
