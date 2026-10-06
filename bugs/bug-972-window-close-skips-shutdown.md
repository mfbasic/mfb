# bug-972: closing an `--app` window (or Cmd-Q) ends a running program with exit status 0 and no teardown — buffered file output is lost, no `--debug` report

Last updated: 2026-10-05
Effort: medium (1h–2h)
Severity: LOW
Class: Other (UNCLEAR — needs owner decision)

Status: Open
Regression Test: (proposed, after the decision) a GUI test that closes the window mid-program and asserts the chosen contract (status, buffered-file drain, debug report)

**Owner decision (2026-10-05):** deliver the close request to the program. Add
`app::shouldClose() AS Boolean`, which returns `TRUE` once the user has tried to close the app
(window close button, Cmd-Q / app-menu Quit, Alt-F4 / WM close, taskbar close) and `FALSE` before
that. A close request no longer terminates the process. The window stays open, and the program
decides when to exit (normally by returning from `main`, which runs `_mfb_shutdown`: flush, RES
cleanup, debug report, normal exit status). The behavior is identical on AppKit (macOS),
Windows, and GTK (Linux). Each backend intercepts its close path
(`applicationShouldTerminate:`/`windowShouldClose:`, `WM_CLOSE`, GTK `delete-event`) and sets one
shared flag. Spec (`mfb spec app` macOS/Linux/window topics, debug-report exits list) and
`mfb man app` document it.

Points the implementer must state and bring to the owner if unclear: whether repeated close
attempts change anything (e.g. a second attempt still only sets the flag), console mode (no
window: `shouldClose` stays `FALSE`, and signals keep today's `_mfb_shutdown` + `128+signo`), and
how the flag reaches a worker thread if the program polls it there.

On macOS, a user closing the window while the program is still running goes
`applicationShouldTerminateAfterLastWindowClosed:` → `-[NSApplication terminate:]` →
libc `exit(0)`. Measured consequences:

* exit status **0** although the program did not finish (indistinguishable from
  success to a parent process / script);
* no `_mfb_shutdown`: a `fs::setBuffered(f, TRUE)` file loses its unflushed bytes
  (file is 0 bytes), RES cleanup does not run;
* a `--debug` build prints no debug report.

The console analogue (SIGINT/SIGTERM) runs `_mfb_shutdown` and exits `128+signo`
(measured 130/143). Neither the macOS runtime spec ("closing the window quits the app")
nor the debug-report spec's lists of exits that do / do not reach `_mfb_shutdown`
mention the window-close path, so the intended contract is unstated.

Owner decision needed: should a user close (a) be documented as an abrupt exit 0 with
no cleanup, (b) run `_mfb_shutdown` and exit with a distinct status, or (c) be
delivered to the program? At minimum the spec should list it.

References:

- `src/docs/spec/app/01_macos-runtime.md` App delegate / Program-finish path.
- `src/docs/spec/tooling/09_debug-report.md` "Every exit that reaches `_mfb_shutdown`
  ..." / "An exit that does not reach `_mfb_shutdown` ..." (window close absent from both).
- `mfb spec memory program-startup` "Signal Dispositions".

## Failing Reproduction

```
IMPORT io
IMPORT fs
IMPORT os

FUNC main() AS Nothing
  RES f AS fs::File = fs::open("/tmp/hardening/B17/wc1.out", "write")
  fs::setBuffered(f, TRUE)
  fs::writeAll(f, "buffered-data\n")
  io::print("waiting")
  MUT i AS Integer = 0
  WHILE i < 1000
    os::sleep(10)
    i = i + 1
  END WHILE
  io::print("end")
END FUNC
```

```
mfb build --app wc1; wc1/build/p1.app/Contents/MacOS/p1 & P=$!; sleep 2
osascript -e "tell application \"System Events\" to tell (first process whose unix id is $P) to click (first button of window 1 whose subrole is \"AXCloseButton\")"
wait $P; echo $?; ls -la /tmp/hardening/B17/wc1.out
```

- Observed: exit `0`; `wc1.out` is 0 bytes. Canvas `--debug` program
  (`/tmp/hardening/B17/ca1`, N=100000) closed the same way: exit 0, stderr empty (no
  `mfb.debug.begin` block).
- Expected: per owner decision (see above).

Note: the console build of the same program killed with SIGTERM also leaves
`wc1.out` empty (exit 143) — `_mfb_shutdown` does not drain per-handle `fs` buffers
either; the man page's "drained ... when the handle is closed" covers scope exit, not
signals. Recorded here as context, not as a separate bug.

## Root Cause

`src/target/macos_aarch64/app/bootstrap.rs:emit_should_terminate_helper` returns YES
and nothing hooks `applicationWillTerminate:` / `windowWillClose:` to run teardown.
GTK `_mfb_gtkapp_window_closed` quits the GApplication → `main` returns 0 →
`exit(0)` (spec `02_linux-runtime.md` step 5) — same shape, not run here.

## Goal

- The window-close / Cmd-Q exit contract is specified and implemented consistently
  on all three backends.

## Blast Radius

- macOS delegate, GTK `close-request`, Windows `WM_CLOSE` — all quit paths.
