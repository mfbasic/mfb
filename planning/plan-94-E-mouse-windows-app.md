# plan-94-E: Mouse events — Windows app backend

Last updated: 2026-08-09
Effort: medium (1h–2h)
Depends on: plan-94-B (the shared decoder + injection contract)

Wire native mouse input in `--app` mode on Windows by handling the mouse
`WM_*` messages in the app `WndProc`, converting each to cell coordinates, and
**injecting the SGR bytes into the worker input pipe the backend already uses for
keystrokes** — decoded by the plan-94-B pump/ring with no Windows-specific event
queue. Smallest of the three backends because the Windows app grid is fixed-size
(constant cell metrics, no reflow) and the `WndProc` already exists.

Behavioral outcome: a Windows app that `enableMouse(TRUE)` and polls receives the
six `MouseKind`s with correct cell coords/modifiers. Not runtime-testable on this
host (no Windows, no window); gate is compile + assembly inspection + the
`.app.ncode`/`.ncodesum` golden diff.

References:

- plan-94-A §3–§4, plan-94-B §3 (pump + injection — read first).
- `src/target/win_x86_64/app/mod.rs` (`WndProc` at `:801`, currently handling only
  `WM_PAINT`/`WM_DESTROY`; the `emit_app_term_helper` at `:1772`; the fixed grid
  constants `TUI_COLS`/`TUI_ROWS` and cell metrics), and how the app feeds
  keystrokes to the worker input path.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-94-B complete | CLI mouse rt test passes | NOT MET |
| Windows keystroke→worker input path located | grep `win_x86_64/app/mod.rs` (Phase 1) | UNVERIFIED |
| Windows cell metrics (px/cell) available as constants | read the fixed-grid setup | UNVERIFIED |

> If plan-94-B is not complete, this sub-plan cannot start, full stop.

## 1. Goal

- The app `WndProc` handles `WM_LBUTTONDOWN`/`UP`, `WM_RBUTTONDOWN`/`UP`,
  `WM_MBUTTONDOWN`/`UP`, `WM_MOUSEMOVE`, and `WM_MOUSEWHEEL`, mapping each to the
  matching `MouseKind`/`MouseButton`.
- Each handler extracts client px from `lParam` (`GET_X_LPARAM`/`GET_Y_LPARAM`),
  divides by the fixed cell metrics → 0-based `(row,col)`, reads button/modifier
  state from `wParam`, formats the SGR report, and writes the bytes to the worker
  input pipe.
- Emission gated on a mouse-enabled flag set by the Windows `enableMouse` arm, so
  motion doesn't flood the pipe when off.

### Non-goals

- No change to CLI, macOS, or GTK; no new event queue (bytes → pipe → pump).
- No window resize handling (the Windows app grid is fixed; out of scope, as noted
  in the didResize work).

## 2. Current State

The Windows app `WndProc` (`app/mod.rs:801`) handles only `WM_PAINT` (BitBlt the
grid) and `WM_DESTROY`; everything else falls to `DefWindowProcW`. There is no
mouse handling today. The grid is fixed-size `TUI_COLS`×`TUI_ROWS` (no reflow —
why the didResize Windows app path reads false), so cell metrics are constants →
px→cell is a constant divide. The Windows `emit_app_term_helper` (`app/mod.rs:1772`)
handles most `term::` calls itself and returns `None` for getters that fall through
to the shared backend — the `enableMouse` arm goes here (to set the flag);
`pollMouse` falls through to the shared plan-94-B ring reader.

### Verified properties

- **`WndProc` exists and is the single message choke point** (`app/mod.rs:801`) —
  adding `WM_*` mouse arms mirrors the existing `WM_PAINT` arm. Verified during the
  didResize Windows investigation.
- **Grid is fixed-size** (constant cell metrics) — verified (didResize Windows app
  reads false because the grid never reflows). So px→cell needs no `TVSTATE`.
- **Keystroke→worker input path + exact cell-metric constants** — UNVERIFIED;
  Phase 1 locates both.

## 3. Design

Add `WM_*` mouse arms to `WndProc`. Extract `x = LOWORD(lParam)`,
`y = HIWORD(lParam)` (client px), divide by the constant cell width/height, clamp,
encode SGR (plan-94-B §3), write to the worker input pipe. `wParam` carries
`MK_SHIFT`/`MK_CONTROL` (→ modifier bits) and pressed-button flags (for drag vs
move). `WM_MOUSEWHEEL` → ScrollUp/Down from the signed `HIWORD(wParam)` delta.
A mouse-enabled flag (set by the `enableMouse` arm) gates emission; `WM_MOUSEMOVE`
early-returns when off. Windows uses the pinned arena register the same as other
Windows term code (`ARENA_STATE_REGISTER` realized to the x86-64 pinned reg), but
`WndProc` is an OS callback — so, like macOS `setFrameSize:`, the flag must live
where the callback can reach it (a global or a fixed data slot, NOT x19-relative);
Phase 1 determines the reachable storage.

Risk: hand-written x86-64 in a Win32 callback; not runtime-testable here. Gate is
compile + disassembly + `.app.ncode`/`.ncodesum` golden diff (the
`macos-app-mode-*` fixtures carry a `windows-x86_64.app.ncodesum`; a Windows-app
mouse fixture golden covers this).

## Phases

### Phase 1 — One button (WM_LBUTTONDOWN/UP → Down/Up) + enable flag + pipe write

- [ ] Locate the keystroke→worker input write and the fixed cell-metric constants;
      record in Corrections. Determine callback-reachable storage for the flag.
- [ ] Add `WM_LBUTTONDOWN`/`WM_LBUTTONUP` arms to `WndProc`: px→cell, SGR encode,
      pipe write. Add the mouse-enabled flag + the Windows `enableMouse` arm.

Acceptance: `cargo build` clean; the new `WndProc` arms disassemble to the expected
px→cell + pipe write; a Windows-app mouse fixture `.app.ncode`/`.ncodesum` diffs
only by the new arms (regenerate + confirm).
Commit: —

### Phase 2 — Full event set + modifiers

- [ ] `WM_RBUTTON*`/`WM_MBUTTON*` (Right/Middle), `WM_MOUSEMOVE` (Move/Drag from
      `wParam` button flags, gated on the enable flag), `WM_MOUSEWHEEL`
      (ScrollUp/Down), modifier bits from `wParam`.

Acceptance: `cargo build` clean; each arm disassembles to the correct SGR encode;
regenerated goldens diff only by the new arms.
Commit: —

## Validation Plan

- Tests: no headless runtime; add/extend a `syntax/app` fixture with
  `enableMouse`+`pollMouse` so the Windows app `.app.ncodesum` golden covers the
  `WndProc` arms.
- Runtime proof: manual on a Windows host (documented; not in CI).
- Doc sync: term-backend spec Windows section — the `WM_*` arms + px→cell + pipe
  injection.
- Acceptance: `cargo build`; `scripts/test-accept.sh <exe> /tmp/out '*app*'`;
  regenerate + confirm the Windows-app golden diffs are only the new arms.

## Open Decisions

- **Flag storage reachable from `WndProc`** — a process-global data slot vs. a
  window `GWLP_USERDATA`/property. Recommend the simplest reachable global,
  determined in Phase 1. (§3)

## Corrections

<Filled in during execution — input-pipe symbol, cell-metric constants, flag
storage.>

## Summary

E is the smallest backend: a fixed grid (constant px→cell), an existing `WndProc`
to extend, and the injection design (format bytes → pipe) with no new queue. Not
runtime-testable here; gate is compile + disassembly + golden diff.
