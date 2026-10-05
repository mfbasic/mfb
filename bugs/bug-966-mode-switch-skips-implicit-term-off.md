# bug-966: `app::setMode` does not perform the documented implicit `term::off` — TUI state survives a mode switch, and on macOS all later output is invisible

Last updated: 2026-10-05
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Open
Coordinator note: measured by the review agent only (Linux runs in podman, or a macOS window screenshot); the coordinator did not re-run it.
Regression Test: (proposed) macOS headless: `setMode(Console); term::on(); setMode(Canvas); setMode(Console); io::print(toString(term::isOn()))` prints `FALSE`; GUI: the same then `io::print("X")` shows `X` in the transcript

The presentation-mode spec says the reconcile tears the surface down "with an implicit
`term::off` so no raw/grid state survives a mode switch". No backend's reconcile seam
does that: the term-state `active` slot stays `1` across `Console → Canvas/None →
Console`.

Observable on macOS (GUI): after `term::on`, a round trip through `Canvas` back to
`Console` leaves the window showing the (empty) transcript, while `io::print` keeps
routing into the off-screen TermView grid (TUI still "active") and `term::drawText` +
`term::sync` present into a view that is not installed. Nothing the program writes
afterwards is visible. `term::isOn()` reports `TRUE` throughout (also headless).

The single correct behavior: every `setMode` that changes the mode first runs
`term::off` semantics (final present, restore, `active = 0`), so after returning to
`Console` the program is in the transcript with TUI off and its `io::print` shows.

References:

- `src/docs/spec/app/05_presentation-mode.md` "The surface-reconcile seam".
- `src/codegen/engine/types/types.rs` doc on `emit_app_mode_reconcile`: "reconcile the
  window surface (implicit `term::off`, then build/teardown)".
- Found by hardening batch B17.

## Failing Reproduction

```
IMPORT app
IMPORT term
IMPORT io
IMPORT os
IMPORT fs

FUNC main() AS Nothing
  app::setMode(app::Mode.Console)
  term::on()
  term::drawText(0, 0, "GRID-FRAME-1")
  term::sync()
  app::setMode(app::Mode.Canvas)
  app::setMode(app::Mode.Console)
  io::print("PRINT-AFTER-RETURN")
  term::drawText(2, 0, "GRID-AFTER-RETURN")
  term::sync()
  fs::appendText("/tmp/hardening/B17/ms6.log", "isOn=" & toString(term::isOn()) & "\n")
  os::sleep(4000)
END FUNC
```

```
mfb build --app ms6; ms6/build/p1.app/Contents/MacOS/p1 &  # screenshot at 3 s
```

- Observed: `ms6.log` = `isOn=TRUE`; window screenshot (`/tmp/hardening/B17/ms6_c.png`)
  is a blank white transcript — neither line visible.
- Expected: `isOn=FALSE`; `PRINT-AFTER-RETURN` visible in the transcript (the
  `drawText` is then a no-op behind the inactive gate).

Headless (`MFB_MACAPP_HEADLESS=1`, `/tmp/hardening/B17/ms1`): after 15
`Console(term::on) → Canvas → None` rounds, final `term::isOn()` = `TRUE`.
GTK: the same sequence crashes instead (see the B17 "gtk-term-broken-after-setmode"
report), so this could not be isolated there.

## Root Cause

`src/codegen/builtins/app/func_set_mode.rs:lower_set_mode` stores the mode word and
calls `CodegenPlatform::emit_app_mode_reconcile`; the macOS seam
(`src/target/macos_aarch64/app/mod.rs:emit_reconcile_seam`) only reloads the mode and
calls the marshal helper; the main-thread reconcile
(`src/target/macos_aarch64/app/bootstrap.rs`, reconcile `none_path` / console /
canvas paths) swaps content views but never calls `_mfb_rt_term_term_off` or clears
`TERM_STATE_ACTIVE_OFFSET`. `rg -n "implicit .term::off"` finds the claim only in the
spec and the `types.rs` doc comment.

## Goal

- After any mode change `term::isOn()` (in `Console`) is `FALSE`, raw input mode is
  restored, and the transcript is the installed content view.

### Non-goals (must NOT change)

- `setMode` to the mode already in effect (should not tear down an active TUI —
  owner decision whether that counts as a "switch").

## Blast Radius

- macOS reconcile — read in code, measured.
- GTK `emit_reconcile_idle_helper` — contains no `ST_TERM_ACTIVE` / term-off
  reference (grep); runtime masked by the GTK crash report.
- Windows `WM_APP_RECONCILE` arm — not checked.
