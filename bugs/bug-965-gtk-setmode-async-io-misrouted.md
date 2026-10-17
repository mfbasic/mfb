# bug-965: GTK `app::setMode` returns before the window is reconciled, so the next `io::print` goes to the wrong sink (stdout instead of the new transcript, or the hidden transcript instead of stdout)

Last updated: 2026-10-05
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Open
Coordinator note: measured by the review agent only (Linux runs in podman, or a macOS window screenshot); the coordinator did not re-run it.
Regression Test: (proposed) GTK/Xvfb run of `setMode(Console); io::print("A")` asserts `A` is in the transcript buffer and not on stdout; and `setMode(None); io::print("C")` asserts `C` is on stdout

## Cause (coordinator, bug-staging-24)

- Rule: when `app::setMode(m)` returns, the surface has been reconciled and `io::` writes follow `m`'s routing (`05_presentation-mode.md` "The surface-reconcile seam" and the "Mode-gated I/O" table); macOS meets it by marshalling the reconcile with `waitUntilDone:YES`.
- Where: `src/target/linux_gtk/bootstrap.rs:emit_reconcile_seam` posts the reconcile with `g_idle_add` and returns at once (`07_window.md`: "fire-and-forget, like the mode reconcile").
- Fix: the GTK seam waits for the posted reconcile to finish (the idle signals completion, the worker blocks until it does), the same contract as macOS; headless stays state-only. The spec's "fire-and-forget, like the mode reconcile" goes for the mode reconcile. Check the Windows `WM_APP_RECONCILE` arm and say in the doc whether it is synchronous (fix it the same way if not).

On the Linux GTK backend the mode reconcile is posted with `g_idle_add` and
`app::setMode` returns immediately. The io write helper routes on `ST_TEXT_BUFFER`
at the moment of the write, so output written right after the call follows the
**previous** mode's routing:

* `None` → `Console`, then `io::print("A")`: `A` is written to **stdout** (the
  transcript does not exist yet) — a double-clicked AppImage has no visible stdout, so
  the line is lost from the window.
* `Console` → `None`, then `io::print("C")`: `C` is appended to the **now-hidden
  transcript** and never appears on stdout.

The macOS backend marshals the reconcile synchronously (`waitUntilDone:YES`) and routes
both lines correctly with the same program.

The single correct behavior: after `app::setMode(m)` returns, `io::` writes follow `m`'s
routing (presentation-mode "Mode-gated I/O" table).

References:

- `src/docs/spec/app/05_presentation-mode.md` "The surface-reconcile seam" and the
  "Mode-gated I/O" table (`Console` → transcript view; `None` → stdout).
- `src/docs/spec/app/07_window.md` Linux: "`g_idle_add(...)` — fire-and-forget, like
  the mode reconcile".
- Found by hardening batch B17.

## Failing Reproduction

```
IMPORT app
IMPORT io
IMPORT os

FUNC main() AS Nothing
  app::setMode(app::Mode.Console)
  io::print("A immediately after setMode")
  os::sleep(500)
  io::print("B after 500ms")
END FUNC
```

```
mfb build --app --target linux-aarch64 ms5
# podman + Xvfb: APPIMAGE_EXTRACT_AND_RUN=1 ./p1-glibc.AppImage > o.txt; cat o.txt
```

- Observed stdout: `A immediately after setMode` (B went to the transcript).
- Expected stdout: empty; both lines in the window transcript.

Second program (`Console`, sleep 500, print `B`, `setMode(None)`, print `C`, sleep 500,
print `D`): observed stdout is only `D`; expected `C` and `D`.

macOS `--app` of the first program: stdout empty (both in the transcript) ✓.

## Root Cause

`src/target/linux_gtk/bootstrap.rs:emit_reconcile_seam` posts the reconcile idle and
returns; `emit_reconcile_idle_helper` is what sets/clears `ST_TEXT_BUFFER`
(`src/target/linux_gtk/app_io.rs:emit_app_io_write` routes on it). Nothing orders the
worker's next write after the idle.

## Goal

- `setMode` does not return until the reconcile has run (or the worker-side routing
  state is updated synchronously before return), so the next write is routed by the
  new mode.

### Non-goals (must NOT change)

- Headless behavior (no main loop; reconcile is state-only).

## Blast Radius

- The window sync (`_mfb_gtkapp_window_sync`) is also fire-and-forget; `getTitle`/
  `getFullscreen` read process-global state so they are unaffected (not measured).
- `canvas::present` immediately after `setMode(Canvas)` on GTK: not observed to fail
  (ca1 on GTK rendered and counted frames).
