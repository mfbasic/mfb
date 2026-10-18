# bug-985: a GTK window built by `app::setMode` never attaches the mouse controllers, so `term::pollMouse` gets no events

Last updated: 2026-10-06
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Fixed
Regression Test: (proposed) the bug-982 GTK app-mode row in `scripts/test-appimage.sh`, run again with a program that calls `app::setMode(app::Mode.Console)` first, asserting one `event` line per mouse action

## Cause (coordinator, bug-staging-24)

- Rule: one window, however it is built: the window `setMode`'s reconcile builds is the window `activate` builds (the same rule bug-962 applied to the term drawing area through `emit_term_area_setup`, which runs in both start modes).
- Where: `src/target/linux_gtk/` `emit_reconcile_build_helper` does not attach the press/release/motion/scroll controllers the `activate` path attaches.
- Fix: move the controller attach into the setup both paths already share (or a shared helper both call), so there is one attach; delete the `activate`-only copy. After bug-1015.

**bug-982 is fixed** (bugs/completed). Its proof found that `term::pollMouse` only drains the
ring: the decoder runs on the stdin read path, so the repro must read stdin (e.g. one
`io::readKey(0)` per loop, as bug-758's does) or it logs no `event` line even when the
controllers are attached. The original note follows.

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

## Fix

- **State at this branch before the fix.** bug-1043 had already given the
  reconcile-built window its mouse controllers — by adding a SECOND copy of the attach
  to `emit_reconcile_build_helper`, while `emit_activate_handler` kept its own inlined
  window build (window, scrolled, text view, key controller, mouse controllers,
  close-request, fullscreen tracking). The behaviour this doc measured was therefore
  already masked by that copy plus bug-1015; the rule ("one window, however it is
  built") was still broken: two builds that had drifted twice.
- **What changed.** `src/target/linux_gtk/bootstrap.rs:emit_activate_handler` no longer
  builds a window: for a `Console`-default program it runs `emit_term_area_setup`,
  calls the shared `_mfb_gtkapp_reconcile_build` (`emit_reconcile_build_helper`) and
  presents; the inlined copy is deleted (and with it the now-unused `uses_mouse` /
  `uses_window` parameters and the `x19` save). `src/target/linux_gtk/mod.rs` emits the
  build helper for every GTK app (it was `None`-default only). The build helper is the
  one place the window gets its children, controllers and signals.
- **Test.** `linux_gtk::canvas_reconcile_tests::the_startup_window_is_the_shared_window_build`
  (unit; activate must call the shared build once and attach nothing itself, for every
  `uses_mouse`×`uses_window`): RED before the edit (`cargo test --release --bin mfb --
  the_startup_window_is_the_shared_window_build` → "the Console-default startup window
  must come from the shared build"), GREEN after. Runtime regression row:
  `scripts/test-appimage.sh --gui`'s pollMouse row now runs for both the startup window
  and an `app::setMode(app::Mode.Console)` window — both `events=1` on 2226
  (`MFB_APPIMAGE_DISPLAY=:77 … --box 2226 --libc glibc --gui`; the 4 other FAILs are
  the 2226 box-environment ones listed in bug-965, identical with main).
  Manual xdotool (2226, X11): startup window click/move/scroll 2/5/2 events, setMode
  window 2/5/2; key + mouse program `keys=abcd`, 2 events.
- **Checks run.** `cargo test --release --bin mfb -- linux_gtk:: linux_common::` 58
  passed; `--test cli_linux_app_mode` 11 passed; `spec-census --citations/--render/
  --links app` clean (spec `02_linux-runtime.md` "activate" section rewritten: activate
  calls the one window build; also corrects its stale "frame 32" to the real 96).
- **Goldens expected to change** (`scripts/artifact-gate.sh <mfb> all`, new with this
  bug): every linux app `.app.ncodesum` — here
  `tests/syntax/app/macos-app-mode-term/golden/macos_app_mode_term.{linux-aarch64,linux-x86_64}.app.ncodesum`
  (Console-default: localized on linux-aarch64, main's `-ncode` = the committed golden;
  `_mfb_gtkapp_reconcile_build` added, `_mfb_gtkapp_activate` changed, nothing else),
  plus the app-window-surface / app-mouse-surface linux rows already moved by
  965/1015 (their `_mfb_gtkapp_activate` does not change: they are `None`-default).
