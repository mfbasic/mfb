# plan-94-E: Mouse events — Windows app backend (term + canvas)

Last updated: 2026-09-20
Effort: medium (1h–2h)
Depends on: plan-94-B (the shared decoder + injection contract)

Wire native mouse input in `--app` mode on Windows by handling the mouse `WM_*`
messages in the app `WndProc`, converting each to surface coordinates (cells for
the TUI grid, pixels for a canvas surface), and **injecting the SGR bytes into the
worker input pipe the backend already uses for keystrokes** — decoded by the
plan-94-B pump/ring with no Windows-specific event queue. Smallest of the three
backends because the Windows app grid is fixed-size (constant cell metrics, no
reflow) and the `WndProc` already exists.

Behavioral outcome: a Windows app that calls `term::enableMouse(TRUE)` or
`canvas::enableMouse(TRUE)` and polls receives the six `MouseKind`s with correct
coordinates and modifiers. Not runtime-testable on a non-Windows host; the gate is
compile + assembly inspection + the `.app.ncode`/`.ncodesum` golden diff.

References:

- plan-94-A §3–§4 (surface, the `_mfb_rt_mouse_mode` word, coordinate units) and
  plan-94-B §3 (the pump + SGR encoding) — **read first**.
- `.ai/arch-abi.md` — Win64 ABI traps (shadow space, the `mfb_arg`/`c_arg`
  distinction that has already bitten this file, Windows PE/console specifics).
- `src/target/win_x86_64/app/mod.rs` — `emit_wndproc` (`:1046`), the fixed grid
  constants `TUI_COLS = 80` / `TUI_ROWS = 25` / `TUI_CELL_W = 8` /
  `TUI_CELL_H = 16` (`:64-67`), the transcript EDIT child and its subclass
  (`EDITPROC_SYMBOL:57`, `EDIT_HWND_SYM:101`, `EDIT_STYLE:175`, created at `:526`),
  the pipe write end (`:106-107`), `WM_CHAR` (`:205`), the canvas paint path
  (`CANVAS_HWND_SYM:151`, `wnd_canvas_paint:1158`), and `emit_app_term_helper`
  (`:2633`).
- `src/codegen/runtime/canvas/mod.rs:122` — `GRAPHICS_OFFSET_WIDTH`/`HEIGHT`, the
  published surface extent for clamping.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-94-B complete | CLI mouse rt test passes | NOT MET |
| Windows keystroke→worker input path located | `editproc` writes each `WM_CHAR` to the pipe (`app/mod.rs:106-107`, `:610`) | MET (read at HEAD) |
| Cell metrics available as constants | `TUI_CELL_W = 8`, `TUI_CELL_H = 16` (`app/mod.rs:66-67`) | MET |
| A mouse-enabled flag reachable from `WndProc` | `_mfb_rt_mouse_mode`, plan-94-A §4.4b | MET (by design, once A lands) |
| **Which window receives mouse messages in each mode** | read the EDIT show/hide logic (`SW_HIDE` at `:650`, `:1499`, `:1533`) | UNVERIFIED (Phase 1 first task) |

> If plan-94-B is not complete, this sub-plan cannot start, full stop.

## 1. Goal

- The app `WndProc` handles `WM_LBUTTONDOWN`/`UP`, `WM_RBUTTONDOWN`/`UP`,
  `WM_MBUTTONDOWN`/`UP`, `WM_MOUSEMOVE` and `WM_MOUSEWHEEL`, mapping each to the
  matching `MouseKind`/`MouseButton`.
- Each handler extracts client px from `lParam` (`GET_X_LPARAM`/`GET_Y_LPARAM`),
  converts to surface coordinates — divide by `TUI_CELL_W`/`TUI_CELL_H` for cells,
  use directly for pixels — reads button/modifier state from `wParam`, formats the
  SGR report, and writes the bytes to the worker input pipe.
- Emission is gated on the process-global `_mfb_rt_mouse_mode` word (plan-94-A
  §4.4b): `0` ⇒ write nothing, `1` ⇒ cells, `2` ⇒ pixels.

### Non-goals

- No change to CLI, macOS, or GTK; no new event queue (bytes → pipe → pump).
- No window resize handling — the Windows app grid is fixed, which is why
  `term::didResize` reads `FALSE` there; out of scope, as the `didResize` work
  recorded.
- No mouse handling in the *transcript* (line-mode) surface. Mouse is a TUI and
  canvas facility; a transcript is a text log.

## 2. Current State

The app `WndProc` (`emit_wndproc`, `src/target/win_x86_64/app/mod.rs:1046`)
handles `WM_PAINT` (BitBlt the grid, or the canvas frame at `wnd_canvas_paint`,
`:1158`), `WM_DESTROY`, `WM_SIZE` and the three private `WM_APP*` messages;
everything else falls to `DefWindowProcW`. There is no mouse handling today.

The grid is fixed-size `TUI_COLS × TUI_ROWS` = 80 × 25 at `TUI_CELL_W × TUI_CELL_H`
= 8 × 16 px (`:64-67`), so px→cell is a **constant divide** with no `TVSTATE`
lookup and no reflow.

Keystrokes reach the worker through a **transcript EDIT child that fills the
window** (`:526`), subclassed by `editproc` (`:57`, `:610`): `editproc` writes each
`WM_CHAR` byte to the pipe whose read end is dup2'd onto fd 0, then chains to the
stock EDIT proc.

**The complication that needs verifying first.** That EDIT child fills the client
area, so while it is visible it — not the main `WndProc` — receives mouse
messages. `term::on` hides the EDIT so the grid shows through (`:63`, and the
`SW_HIDE` calls at `:650`, `:1499`, `:1533`), which is exactly the state in which
`term::enableMouse` can be in effect, so the main `WndProc` should be the right
place. Canvas mode likewise replaces the transcript. **Phase 1 proves this rather
than assuming it**; if some mode leaves the EDIT visible, `editproc` is the
already-subclassed fallback choke point and the same handler body moves there.

The Windows `emit_app_term_helper` (`:2633`) handles most `term::` calls itself
and returns `None` for the ones that fall through to the shared backend — the
`enableMouse` arm goes here (to write the mode word); `pollMouse` falls through to
the shared plan-94-B ring reader.

**What changed since this sub-plan was first drafted.** The canvas paint path and
`WM_CHAR` handling did not exist, so the draft listed the keystroke path and the
cell metrics as UNVERIFIED — both are now read and recorded above. The draft's
Open Decision on flag storage ("a process-global data slot vs. `GWLP_USERDATA`") is
closed by plan-94-A §4.4b. What the draft did *not* anticipate, and what is now
the one genuinely open question, is the EDIT control's claim on mouse messages.

### Verified properties

- **`WndProc` exists and is the message choke point for the main window**
  (`emit_wndproc:1046`), and adding `WM_*` mouse arms mirrors the existing
  `WM_PAINT` arm.
- **The grid is fixed-size with constant cell metrics** (`:64-67`), so px→cell
  needs no cached state.
- **A UI-thread callback already writes to the worker pipe** — `editproc` does it
  per `WM_CHAR` (`:106-107`, `:610`) — so the injection is a pattern that already
  works in this context.
- **Win64 argument-register confusion is a live hazard in this file**: a past bug
  (plan-66-J-4) read garbage because `return_register()` is `rax` on Win64, not
  `ARG[0] = rcx`. New handlers must use `abi::c_arg(_)`/`abi::mfb_arg(_)`
  deliberately.

## 3. Design

Add `WM_*` mouse arms to `WndProc`. Each arm:

1. **Gate.** Load `_mfb_rt_mouse_mode`; chain to `DefWindowProcW` if `0`.
2. **Extract.** `x = LOWORD(lParam)`, `y = HIWORD(lParam)` (client px, **signed** —
   sign-extend, because a drag can report negative coordinates outside the client
   area).
3. **Convert.** Cells (`mode == 1`): divide by `TUI_CELL_W`/`TUI_CELL_H`, clamp to
   `TUI_COLS`/`TUI_ROWS`. Pixels (`mode == 2`): use directly, clamp to
   `GRAPHICS_OFFSET_WIDTH`/`HEIGHT`.
4. **Encode + write.** SGR per plan-94-B §3 (coordinates +1 back to 1-based),
   bytes to the worker input pipe.

`wParam` carries `MK_SHIFT`/`MK_CONTROL` (→ modifier bits) and the pressed-button
flags (which distinguish `Drag` from `Move` on `WM_MOUSEMOVE`). `WM_MOUSEWHEEL` →
ScrollUp/ScrollDown from the signed `HIWORD(wParam)` delta. `WM_MOUSEMOVE`
early-returns when the mode word is `0`, so an un-enabled program pays one load
per motion message.

Note that `WM_MOUSEWHEEL` is delivered to the **focused** window in screen
coordinates, unlike the button and move messages which go to the window under the
cursor in client coordinates — so the wheel arm needs a `ScreenToClient` before
step 3. This is a Win32 asymmetry, not a design choice.

**Risk.** Hand-written x86-64 in a Win32 callback, not runtime-testable on a
non-Windows host. Gate is compile + disassembly + `.app.ncode`/`.ncodesum` golden
diff.

## Phases

### Phase 1 — Prove the message route, then one button (WM_LBUTTONDOWN/UP → Down/Up)

- [ ] **Determine which window receives mouse messages in TUI mode and in canvas
      mode** — read the EDIT show/hide logic (`SW_HIDE` at `:650`, `:1499`,
      `:1533`) and the canvas mode entry. Record the answer in Corrections. If the
      EDIT is visible in a mode that needs mouse, move the handler body to
      `editproc` and note it.
- [ ] Add `WM_LBUTTONDOWN`/`WM_LBUTTONUP` arms: gate, extract, px→cell, SGR
      encode, pipe write.
- [ ] Windows `emit_app_term_helper` (`:2633`): a `term.enableMouse` arm writing
      `_mfb_rt_mouse_mode`.

Acceptance: `cargo build` clean; the new `WndProc` arms disassemble to the
expected gate + px→cell + pipe write; a Windows-app mouse fixture
`.app.ncode`/`.ncodesum` diffs only by the new arms (regenerate + confirm).
Commit: —

### Phase 2 — Full event set + modifiers

- [ ] `WM_RBUTTON*`/`WM_MBUTTON*` (Right/Middle), `WM_MOUSEMOVE` (Move/Drag from
      the `wParam` button flags, gated on the mode word), `WM_MOUSEWHEEL`
      (ScrollUp/ScrollDown, with the `ScreenToClient` conversion), modifier bits
      from `wParam`.

Acceptance: `cargo build` clean; each arm disassembles to the correct SGR encode;
regenerated goldens diff only by the new arms.
Commit: —

### Phase 3 — Canvas surface, pixels

- [ ] Extend each arm with the `mode == 2` branch: no cell divide, clamp to
      `GRAPHICS_OFFSET_WIDTH`/`HEIGHT`.
- [ ] A `canvas.enableMouse` arm writing `_mfb_rt_mouse_mode = 2`;
      `canvas::pollMouse` reading the ring as `Point`.
- [ ] A `syntax/app` fixture exercising `canvas::enableMouse`+`pollMouse` so the
      Windows `.app.ncodesum` golden covers the canvas branch.

Acceptance: `cargo build` clean; the pixel branch disassembles with no divide;
goldens diff only by the new arms.
Commit: —

## Validation Plan

- Tests: no headless runtime. Add or extend `syntax/app` fixtures with
  `term::enableMouse`+`pollMouse` and `canvas::enableMouse`+`pollMouse` so the
  Windows `.app.ncodesum` golden covers the `WndProc` arms.
- Runtime proof: manual on a Windows host (documented; not in CI). See
  `.ai/remote_systems.md` for the available machines.
- Doc sync: `src/docs/spec/app/04_term-backend.md` Windows section — the `WM_*`
  arms, px→cell and px→pixel, the mode-word gate, pipe injection, and the wheel's
  screen-coordinate asymmetry.
- Acceptance: `cargo build`; `scripts/test-accept.sh <exe> /tmp/out '*app*'`;
  regenerate + confirm the Windows-app golden diffs are only the new arms.

## Open Decisions

- **Which proc hosts the handlers** — the main `WndProc` (expected, since
  `term::on` hides the EDIT) vs. the already-subclassed `editproc` if any
  mouse-relevant mode leaves the EDIT visible. Phase 1 decides on evidence. (§2)

*(The flag-storage Open Decision the pre-migration draft carried is closed: the
process-global `_mfb_rt_mouse_mode` word, plan-94-A §4.4b.)*

## Corrections

<Filled in during execution — the message-route finding, and anything the Win64
calling convention costs that §3 does not predict.>

## Summary

E is the smallest backend: a fixed grid (constant px→cell), an existing `WndProc`
to extend, an existing UI-thread pipe write to copy, and the injection design
(format bytes → pipe) with no new queue. Its one real unknown is a Win32 fact
rather than a design choice — whether the transcript EDIT child intercepts mouse
messages in any mode that needs them — and Phase 1 settles it before a line of
encoding is written. Not runtime-testable here; gate is compile + disassembly +
golden diff.
