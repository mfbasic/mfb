# bug-985: a GTK window built by `app::setMode` never attaches the mouse controllers, so `term::pollMouse` gets no events

Last updated: 2026-10-06
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: (proposed) the bug-982 GTK app-mode row in `scripts/test-appimage.sh`, run again with a program that calls `app::setMode(app::Mode.Console)` first, asserting one `event` line per mouse action

**Fix bug-982 first.** Attaching the controllers before bug-982 is fixed turns "no events"
into bug-982's SIGSEGV.

On Linux GTK `--app` builds, a program that calls `app::setMode(app::Mode.Console)` and then
`term::enableMouse(TRUE)` never receives a mouse event. `term::pollMouse` returns
`MouseKind.None` throughout, even while the pointer moves and clicks inside the window.
A Console-default program (no `setMode`) has the controllers attached by `activate`, and
receives events (on aarch64 it then crashes; see bug-982).

**Correct behavior:** whichever way the window is built (at `activate`, or by the
`setMode` reconcile), the press/release/motion/scroll controllers are attached, so
`term::enableMouse(TRUE)` delivers events the same way in both.

## Where to look

`emit_reconcile_build_helper` (`src/target/linux_gtk/`) builds the window for a `setMode`
program and does not attach the mouse controllers that the `activate` path attaches. This
was found by bs16-agent-c while fixing bug-962, which moved the term drawing-area setup into
`emit_term_area_setup` (run in both start modes) but deliberately left the mouse controllers
alone.

## Failing Reproduction

Measured by the coordinator on 2026-10-06 on box 2226 (Debian 12 aarch64 glibc, GTK4,
Xvfb + xdotool). The compiler is bug-staging-16 with bug-962 fixed
(`/tmp/bs16-agent-c-target/release/mfb`); before bug-962, a `setMode` program could not use
`term::` at all.

The program is bug-982's repro with `IMPORT app` added, `app::setMode(app::Mode.Console)` as
the first statement of `main`, and the log path changed to `/tmp/bs16rev-pm.log`:

```
mfb build -q --app --target linux-aarch64 pm
# on 2226:
Xvfb :93 -screen 0 1024x768x24 &
export DISPLAY=:93 GSK_RENDERER=cairo GDK_BACKEND=x11
timeout 20 ./pm-glibc.AppImage & sleep 1.5
xdotool mousemove 200 200; xdotool click 1      # or: mousemove 120 90; mousemove 300 200
wait $!; echo rc=$?; grep -c event /tmp/bs16rev-pm.log
```

Observed:

| action | rc | `event` lines | `done` |
|---|---|---|---|
| click | 0 | 0 | 1 |
| move | 0 | 0 | 1 |

Control: bug-982's Console-default program with the same `xdotool` moves crashes on the first
event (rc 139), which shows events do reach a window at those coordinates on this display.

Expected: rc 0, at least one `event` line per action (after bug-982 is fixed).
