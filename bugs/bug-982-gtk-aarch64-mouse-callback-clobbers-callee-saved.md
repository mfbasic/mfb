# bug-982: Linux aarch64 GTK mouse callbacks clobber callee-saved `x19`–`x22`, so any mouse event SIGSEGVs an app that enabled the mouse

Last updated: 2026-10-05
Effort: small (<1h)
Severity: HIGH
Class: Memory-safety

Status: Open
Regression Test: (proposed) a GTK app-mode row in `scripts/test-appimage.sh` that clicks, moves and scrolls under Xvfb + xdotool with `term::enableMouse(TRUE)` on, and asserts exit 0

On linux-aarch64 `--app` builds, an app that calls `term::enableMouse(TRUE)` dies with
SIGSEGV (exit 139) on the first mouse event: a click, a motion, or a scroll. This includes
`Mode.Console` default programs. Without a mouse action, the same program exits 0.

**Correct behavior:** a GTK signal callback is called through the C ABI, so it must preserve
the AAPCS64 callee-saved registers (`x19`–`x28`). The mouse handler must save every
callee-saved register it writes and restore it before it returns. A mouse event is then
queued for `term::pollMouse` and the program keeps running.

## Root cause

`emit_mouse_handler` (`src/target/linux_gtk/mouse.rs`) builds the four GTK mouse callbacks
(press, release, motion, scroll). Its prologue saves only `lr` (plus the controller and the
d0/d1 coordinates). The body then writes `abi::LOCAL[0]` (`x19`) and `abi::LOCAL[1]` (`x20`)
in the gate, in `emit_translate_and_convert` and in `emit_modifiers`. The SGR formatting
writes `SCRATCH[10..12]`, which realize to `x20`–`x22`. None of them are restored. GLib's
closure marshaller holds a pointer in `x19` across the `blr` into our callback, and faults
dereferencing it after the return (`x19 = 0x1`, the `_mfb_rt_mouse_mode` value the handler
loaded).

`grep -n 'LOCAL\[0\]' src/target/linux_gtk/mouse.rs` shows the writes. The prologue at the
top of `emit_mouse_handler` stores only `link_register`, `c_arg(0)` and `FP_SCRATCH[0..1]`.

x86-64 may be unaffected, because callbacks there are bracketed by `wrap_x86_instructions`.
This is a guess: box 2228 was down, so x86-64 GTK was not run.

## Failing Reproduction

Measured by the coordinator on 2026-10-05, with main at c6dd68acc
(`/tmp/main-dbg-target/release/mfb`), on box 2226 (Debian 12 aarch64 glibc, GTK).

```
mfb init p962mc   # src/main.mfb:
IMPORT term
IMPORT io
IMPORT os
IMPORT fs
FUNC main() AS Nothing
  os::sleep(500)
  term::on()
  term::enableMouse(TRUE)
  io::print("mouse")
  term::sync()
  MUT i AS Integer = 0
  WHILE i < 40
    LET ev AS term::MouseEvent = term::pollMouse()
    IF ev.kind <> term::MouseKind.None THEN
      fs::appendText("/tmp/p962m.log", "event row=" & toString(ev.row) & " col=" & toString(ev.column) & "\n")
    END IF
    os::sleep(100)
    i = i + 1
  END WHILE
  term::off()
  fs::appendText("/tmp/p962m.log", "done\n")
END FUNC

mfb build -q --app --target linux-aarch64 p962mc
# ship build/p962mc-glibc.AppImage to 2226, then there:
Xvfb :93 -screen 0 1024x768x24 &
export DISPLAY=:93 GSK_RENDERER=cairo GDK_BACKEND=x11
timeout 20 ./p962mc-glibc.AppImage & sleep 1.2; xdotool click 1; wait $!; echo rc=$?
```

Observed, one run per action:

| action | rc |
|---|---|
| none | 0 |
| `xdotool click 1` | 139 |
| `xdotool mousemove 120 90; xdotool mousemove 300 200` | 139 |
| `xdotool click 4` (scroll) | 139 |

gdb on 2226 (from the agent that found it, bs16-c): the fault is in thread 1 (the GTK main
thread) at `ldr x0, [x19]`, right after `blr x26` (the GLib closure marshal's call into our
callback), with `x19 = 0x1` and `x20 = 0xaaaaaabff300` (the term area our handler loaded).

Expected: rc 0, with one `event` line logged per mouse action, and a final `done`.

## Related

- Found while proving bug-962 (`term::` in an `app::setMode` program has no drawing area).
- bug-985 (reproduced): the window built by
  `emit_reconcile_build_helper` never attaches the mouse controllers, so a `setMode` program
  gets no mouse events at all. It can only be reproduced once bug-962 has landed, and it must
  be fixed after this bug, because attaching the controllers first would turn "no events"
  into this crash.
