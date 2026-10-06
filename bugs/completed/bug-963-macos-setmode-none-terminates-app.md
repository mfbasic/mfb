# bug-963: macOS `--app` — switching from a windowed mode to `app::Mode.None` makes AppKit terminate the process (exit 0) moments later, killing the running program

Last updated: 2026-10-05
Effort: small (<1h)
Severity: HIGH
Class: Correctness

Status: Fixed (bs16-agent-c)
Regression Test: `scripts/test-macapp.sh` case 3d2 (GUI, `MFB_MACAPP_GUI=1`): Console→None and Canvas→None, then sleep 1.5 s, print, `RETURN 7`; expects `HIDDEN_ALIVE|code=7`. Unit: `target::macos_aarch64::app::canvas_reconcile_tests::a_program_hiding_its_window_does_not_terminate_the_app`.

On macOS, `app::setMode(app::Mode.None)` after the window has been shown (from
`Console` or `Canvas`) orders the window out. With the delegate answering
`applicationShouldTerminateAfterLastWindowClosed:` YES, AppKit schedules its
"terminate after last window closed" check when the last visible window goes away,
and `-[NSApplication terminate:]` ends the process with status 0 a few hundred ms
later — in the middle of the program. Everything the program does afterwards is lost:
no further output, no file writes, no lexical cleanup, no `_mfb_shutdown`, and the
exit status is 0 regardless of what the program would have returned.

A program that *starts* in `None` and never shows a window is unaffected, and a
program that switches to `None` and returns immediately races to completion, which is
why the existing headless tests do not see this.

The single correct behavior: `Mode.None` hides the window and the program keeps
running until it ends itself (as on GTK, measured below); only a **user** closing the
window (or Cmd-Q) quits the app.

References:

- `src/docs/spec/app/05_presentation-mode.md` — `None` is "windowless"; "a program
  that intends to manage its own surface ... brings a window up deliberately" (and,
  by the model, down again).
- `src/docs/spec/app/01_macos-runtime.md` App delegate — `applicationShouldTerminateAfterLastWindowClosed:` "returns YES so closing the window quits the app".
- Found by hardening batch B17.

## Failing Reproduction

```
IMPORT app
IMPORT io
IMPORT os
IMPORT fs

FUNC main() AS Nothing
  LET path AS String = "/tmp/hardening/B17/ms3.log"
  fs::appendText(path, "start mode=" & toString(app::getMode()) & "\n")
  io::print("printed in none (initial)")
  fs::appendText(path, "after print 1\n")
  app::setMode(app::Mode.Console)
  io::print("printed in console")
  fs::appendText(path, "after print 2\n")
  app::setMode(app::Mode.None)
  fs::appendText(path, "switched to none\n")
  os::sleep(1000)
  io::print("printed in none (after console)")
  fs::appendText(path, "end\n")
END FUNC
```

```
mfb build --app ms3 && ms3/build/p1.app/Contents/MacOS/p1; echo $?; cat ms3.log
```

- Observed: exit `0`; stdout only `printed in none (initial)`; log ends at
  `switched to none` (no `end`). Same with `Canvas` → `None` (`/tmp/hardening/B17/ms4`,
  both variants die before `slept`).
- Expected: `printed in none (after console)` on stdout, log ends with `end`.
- lldb (`breakpoint set -n terminate:`) on the main thread:
  `-[NSApplication terminate:]` ← `_terminateFromSender:askIfShouldTerminate:saveWindows:`
  ← `__79-[NSApplication(NSWindowCache) _scheduleCheckForTerminateAfterLastWindowClosed]_block_invoke`
  ← run-loop timer ← `-[NSApplication run]`.

| Environment | Result |
| --- | --- |
| macOS 15 (Darwin 24.6) aarch64, GUI | terminated ✗ |
| macOS headless (`MFB_MACAPP_HEADLESS=1`) | runs to end ✓ |
| linux-aarch64 GTK4/Xvfb, same program | runs to end ✓ |

## Root Cause

`src/target/macos_aarch64/app/bootstrap.rs` reconcile `none_path` sends
`orderOut:` to the window; `emit_should_terminate_helper` unconditionally returns YES.
AppKit treats "no visible windows" after an `orderOut:` as the last window closing and
terminates. Hypothesis for the fix shape: make the delegate answer NO while the
presentation mode is `None` (or answer NO and terminate explicitly from a real
`windowWillClose:`), so only a user close quits.

## Goal

- After `setMode(None)` from a windowed mode the program keeps running and exits with
  its own status.
- Closing the window by the user still quits.

### Non-goals (must NOT change)

- Headless behavior; GTK and Windows backends.

## Blast Radius

- macOS reconcile `none_path` + `SHOULD_TERMINATE_SYMBOL` — fixed by this bug.
- Windows: not run (no host); `win_x86_64/app` reconcile hides via its own path —
  unverified.

## Resolution

The hypothesis's first shape: answer NO while the program, not the user, has hidden the
window. The reconcile (`emit_reconcile_helper`) now records the mode on NSApp under
`_mfb_macapp_program_hidden_key` before it dispatches: `None` sets it (to NSApp, ASSIGN),
and `Console` and `Canvas` clear it. For a None-start program, `emit_should_terminate_helper`
reads the key from the `sender` (NSApp): set → NO, clear → YES. A Console-default program
cannot hide its window, so its IMP stays the constant YES with unchanged bytes (the unit test
checks that it has no relocations). Headless, GTK and Windows are untouched. Spec:
`mfb spec app macos-runtime`, "App delegate". The man page already described the intended
behaviour (`mfb man app setMode`: entering None "tears the window down and routes io::print
to standard output").

### Proof (this Mac, macOS 15 / Darwin 24.6 aarch64, GUI session)

- Doc repro (`ms3`, `RETURN 7` added), base compiler, 2 runs: rc 0. stdout is only
  `printed in none (initial)`, and the log stops at `switched to none`. Fixed: rc 7,
  `printed in none (after console)` on stdout, and the log ends with `end`, 2/2.
- `scripts/test-macapp.sh` case 3d2, run alone (`MFB_MACAPP_GUI=1`). Base: both variants
  `got 'code=0'` (FAIL). Fixed: both `ok: ... (HIDDEN_ALIVE|code=7)`.
- A user close still quits. Each app was started with `open -W` and then
  `osascript -e 'tell application "System Events" to tell process "<p>" to click button 1
  of window 1'` was run at 2.5–3 s:
  - Console window shown, never hidden: the process ended 3 s after launch, and the
    "not closed" marker it would have written after its 10 s sleep is absent.
  - Console → None → Console, then close: also ended at 3 s, with no "not closed" marker.
- Debugger (`mfb dbg` on a `--debug -O0` build, `bp _mfb_macapp_should_terminate`, `run`,
  `bt 4`, `finish`): the stop is under
  `-[NSApplication(NSWindowCache) _checkForTerminateAfterLastWindowClosed:saveWindows:]`
  ← `_scheduleCheckForTerminateAfterLastWindowClosed]_block_invoke` (the doc's lldb chain).
  `finish` reports `x0 = 0` (NO), and the program then runs to `end`.
- `scripts/test-macapp.sh <mfb>` (default, headless): "macOS app mode runtime tests passed",
  12 ok, GUI cases skipped.

## Golden impact

None-start macOS app programs change: the reconcile gains the key store (about 10
instructions plus relocations) and `_mfb_macapp_should_terminate` grows its key read. The data
gains the 1-byte `_mfb_macapp_program_hidden_key`. `scripts/artifact-gate.sh <mfb> all`
shows exactly one new diff beyond the previous set:
`syntax/app/app-window-surface/app_window_surface.macos-aarch64.app.ncode`.
`syntax/app/app-mouse-surface`'s macOS golden (a setMode program) was already in the branch
base's diff set and takes this change too.
