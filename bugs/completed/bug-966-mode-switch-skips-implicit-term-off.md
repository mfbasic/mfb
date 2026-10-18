# bug-966: `app::setMode` does not perform the documented implicit `term::off` — TUI state survives a mode switch, and on macOS all later output is invisible

Last updated: 2026-10-05
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Fixed
Coordinator note: measured by the review agent only (Linux runs in podman, or a macOS window screenshot); the coordinator did not re-run it.
Regression Test: (proposed) macOS headless: `setMode(Console); term::on(); setMode(Canvas); setMode(Console); io::print(toString(term::isOn()))` prints `FALSE`; GUI: the same then `io::print("X")` shows `X` in the transcript

## Cause (coordinator, bug-staging-24)

- Rule: every `setMode` that changes the mode tears the surface down "with an implicit `term::off` so no raw/grid state survives a mode switch" (`src/docs/spec/app/05_presentation-mode.md`, "The surface-reconcile seam"); a same-mode `setMode` is not a change and keeps the TUI (this doc's Correct behavior).
- Where: the one place every backend passes through, `src/codegen/builtins/app/func_set_mode.rs:lower_set_mode`, stores the mode and calls `emit_app_mode_reconcile`; no backend's seam runs term-off.
- Fix: `lower_set_mode`, when the new mode differs from the current one, runs `term::off`'s own lowering (final present, restore, `active = 0`) before the reconcile, on every backend at once; no per-backend copy. The `types.rs` doc on `emit_app_mode_reconcile` then states what the seam really does.

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

## Fix

- **What changed.** `src/codegen/builtins/app/func_set_mode.rs:lower_set_mode` now, in a
  program with `term::` state (`ctx.term_state_offset.is_some()`), loads the mode
  already in the slot, compares it with the requested one, and on a change calls
  `_mfb_rt_term_term_off` — `term::off`'s own helper (final present, input restore,
  `active = 0`) — *before* storing the new mode and running the per-backend reconcile.
  One place, every backend; no backend reconcile changed. It runs while the slot still
  holds the old mode, so an active TUI (only possible in `Console`) passes
  `term::off`'s `ErrWrongMode` gate; from `Canvas`/`None` the TUI is already off and the
  helper's static `ErrWrongMode` result is discarded (setMode tags OK afterwards, as the
  shutdown auto-restore already does with the same helper). Same-mode `setMode` skips
  the call (the Non-goal). The `_mfb_rt_term_term_off` symbol always exists when term
  state exists (`builder::mod` / `plan::symbols` force it in for the exit restore).
  The `types.rs` doc on `emit_app_mode_reconcile` now says the seam does NOT run
  `term::off`; the spec "surface-reconcile seam" section lists the three steps; the
  `app::setMode` man DESC states the TUI is left on a switch and kept on a same-mode call.
- **Tests (`tests/cli/cli_app_canvas_mode.rs`).**
  `macos_mode_change_turns_the_tui_off_and_same_mode_keeps_it` (runtime, macOS headless:
  same-mode keeps TUI, Canvas→Console and None→Console turn it off) and
  `every_app_target_set_mode_calls_term_off_on_a_mode_change` (ncode: linux-aarch64,
  linux-x86_64, windows-x86_64, macos-aarch64 setMode calls the helper behind a
  mode-compare `b.eq`; a term-free program does not). RED with main's compiler
  (`MFB_TEST_EXE=/tmp/main-dbg-target/release/mfb cargo test --release --test
  cli_app_canvas_mode -- mode_change term_off`: both FAILED — exit 3, "linux-aarch64:
  setMode must call the term::off helper"); GREEN with the fix (13/13).
- **Cross-backend runtime proof** (probe = the macOS test's program):
  GTK box 2226 (glibc AppImage): main compiler exit 3 under `DISPLAY=:77` (Xvfb) and
  `MFB_HEADLESS=1`; fixed exit 0 in both. Windows box 206 (`MFB_HEADLESS=1`, result
  written to a file because a Windows app's `main AS Integer` exit code is not the
  process exit code): main `code=3`, fixed `code=0`. macOS GUI run of this doc's repro
  logs `isOn=FALSE` (was `TRUE`); the window screenshot could not be taken (the
  session's screen capture returns a black frame), so the visible transcript was not
  photographed.
- **Checks run.** `cargo test --release --test cli_app_canvas_mode` 13 passed;
  `cargo test --release --bin mfb -- builtins::app:: builtins::tests:: app::hook::`
  43 passed; `-- spec` 46 passed; `-- registry` 107 passed; `--test architecture_guards`
  8 passed; `--test no_type_strings` 7 passed; `MFB=… scripts/man-run-examples.sh app`
  6/6 and `term` 48/48 built; `scripts/spec-census.sh --citations app` 0 missing,
  `--render app` 0 leaks, `--links app` 0 unresolved.
- **Goldens expected to change** (`scripts/artifact-gate.sh <mfb> all`: 4 diffs, all
  here): `tests/syntax/app/app-mouse-surface/golden/app_mouse_surface.{macos-aarch64,
  linux-aarch64,linux-x86_64,windows-x86_64}.app.ncodesum` — the only fixture whose app
  build has both `app::setMode` and `term::` state. Localized on linux-x86_64: main's
  `-ncode` hashes to the committed golden, and the only function that differs from the
  fixed compiler's plan is `_mfb_rt_app_app_setMode`.
