# plan-94-C: Mouse events — macOS app backend (term + canvas)

Last updated: 2026-09-20
Effort: large (3h–1d)
Depends on: plan-94-B (the shared decoder + injection contract)

Wire native mouse input in `--app` mode on macOS by adding mouse-event IMPs to the
two synthesized views — `TermView` (cells, for `term::pollMouse`) and
`MFBCanvasView` (pixels, for `canvas::pollMouse`) — converting each event to
surface coordinates and **injecting the SGR bytes into the same window input pipe
both views already use for keystrokes** — so the plan-94-B decoder/ring decodes
them with zero macOS-specific event-queue code.

Behavioral outcome: a macOS app that calls `term::enableMouse(TRUE)` (in `Console`
mode) or `canvas::enableMouse(TRUE)` (in `Canvas` mode) and polls receives
`Down`/`Up`/`Drag`/`Move`/`ScrollUp`/`ScrollDown` with correct coordinates and
modifiers from real trackpad/mouse input. (App mode is not headless-testable; the
gate is compile + assembly inspection + the `.app.ncode` golden diff, exactly like
the `didResize` macOS work.)

References:

- plan-94-A §3–§4 (surface, the `_mfb_rt_mouse_mode` word, coordinate units) and
  plan-94-B §3 (the pump + SGR encoding) — **read first**.
- `.ai/canvas-threading.md` §1–§2 — the main/worker/graphics split and why a
  UI-thread IMP cannot read arena state. This is the reason the enable flag is a
  process-global word and not a `TVSTATE` byte.
- `.ai/arch-abi.md` — macOS AArch64 ABI traps (the ObjC calling convention, NSPoint
  in float registers, no `x19` arena base inside a callback).
- `src/target/macos_aarch64/app/term_view.rs` — the `TermView` class synthesis and
  the `setFrameSize:` IMP body.
- `src/target/macos_aarch64/app/mod.rs` — the `TV_*` offsets
  (`TV_CELL_W_OFFSET = 40`, `TV_CELL_H_OFFSET = 48`, `:517`), the selector and
  type-encoding tables, the `class_addMethod` registration beside
  `emit_term_set_frame_size_helper()` (`:795`), `KEY_DOWN_SYMBOL` (`:229`),
  `PIPE_ASSOC_KEY` (`:230`), `CANVAS_KEY_DOWN_SYMBOL` (`:460`),
  `CANVAS_ACCEPTS_FR_SYMBOL` (`:456`).
- `src/target/macos_aarch64/app/app_io.rs:551` (`emit_app_term_helper`).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-94-B complete | CLI mouse rt test passes; the decoder consumes injected bytes from the input pipe | NOT MET |
| The keystroke→pipe write is reusable from a new IMP | read `KEY_DOWN_SYMBOL`'s body and `PIPE_ASSOC_KEY`; confirm the write is a callable sequence, not inlined one-off | UNVERIFIED (Phase 1 first task) |

> If plan-94-B is not complete, this sub-plan cannot start, full stop.

## 1. Goal

- `TermView` overrides `mouseDown:`/`mouseUp:`/`mouseDragged:`/`mouseMoved:`/
  `rightMouseDown:`/`rightMouseUp:`/`rightMouseDragged:`/`otherMouse*:`/
  `scrollWheel:` — the set needed for the six `MouseKind`s and three
  `MouseButton`s.
- `MFBCanvasView` overrides the same set.
- Each `TermView` IMP converts the event location to a 0-based cell `(row, col)`
  using the cached `TV_CELL_W`/`TV_CELL_H`; each `MFBCanvasView` IMP converts to
  0-based pixels. Both format an SGR report and write the bytes to the window
  input pipe.
- Emission is gated on the process-global `_mfb_rt_mouse_mode` word (plan-94-A
  §4.4b): `0` ⇒ write nothing, `1` ⇒ TermView emits, `2` ⇒ CanvasView emits. So
  motion never floods the pipe when the program has not asked for it.

### Non-goals

- No change to CLI, GTK, or Windows.
- No new event queue: events flow as bytes through the existing pipe into the
  plan-94-B decoder/ring. (Keeps the ring worker-local — plan-94-A §4.3.)
- No change to `setFrameSize:`/`drawRect:`/the grid/the canvas blit.
- No hit testing, no drag thresholds, no double-click synthesis. A `Down` is a
  `Down`; the program decides what a gesture is.

## 2. Current State

**`TermView`** is synthesized for every macOS `--app` build (the `setFrameSize:`
IMP is emitted unconditionally — proven by the `didResize` work, whose two-line
`setFrameSize` change shifted the `macos-app-mode-io`/`plumbing` `.app.ncode`
goldens). `TVSTATE` caches `TV_CELL_W`@40 / `TV_CELL_H`@48
(`src/target/macos_aarch64/app/mod.rs:517`), so px→cell is `floor(px / cell)` —
the exact computation `setFrameSize:` already does for rows/cols. IMPs are
registered with `class_addMethod` against a selector + type-encoding table;
adding a mouse IMP mirrors the `setFrameSize:` registration at `:795`.

**`MFBCanvasView`** is synthesized for a canvas build and already adds exactly two
overrides for input: `acceptsFirstResponder` (`:456`) and `keyDown:` (`:460`),
which "writes the key's UTF-8 straight to the window input pipe, with no echo and
no line buffering" — the same pipe `TermView`'s `keyDown:` (`:229`) writes to,
reached through `PIPE_ASSOC_KEY` (`:230`). Mouse mirrors that write on both views.

**What changed since this sub-plan was first drafted.** `MFBCanvasView` did not
exist; the pre-migration draft covered `TermView` only and left the flag storage
as an Open Decision between a `TVSTATE` byte and an `NSTrackingArea`. Both of
those are now settled: `.ai/canvas-threading.md` §2 makes it explicit that a
UI-thread callback has no arena state, and plan-94-A §4.4b puts one process-global
word where every callback on every backend can read it — so `TVSTATE` grows no
mouse byte and the two views share one gate.

### Verified properties

- **`TV_CELL_W`/`TV_CELL_H` are the live cell metrics**, read by `setFrameSize:`
  to compute rows/cols — reusable for px→cell
  (`src/target/macos_aarch64/app/mod.rs:517`, and the `setFrameSize:` body in
  `term_view.rs`).
- **Both views already write bytes to the window input pipe from a UI-thread IMP**
  (`KEY_DOWN_SYMBOL:229`, `CANVAS_KEY_DOWN_SYMBOL:460`), so the injection this
  sub-plan needs is a pattern that already works in this exact context — not a new
  capability. What is UNVERIFIED is only whether the write is factored into a
  callable sequence a new IMP can reuse; Phase 1 reads it.
- **`canvas::Point` is top-left origin, Y increasing downward**
  (`src/codegen/builtins/canvas/mod.rs:213`), whereas NSView is bottom-left — so
  the Y flip is required on both views, and for canvas it is required to match the
  documented coordinate system, not merely the grid.

## 3. Design

Per IMP: read `locationInWindow` (an `NSPoint` — two doubles, returned in the
float registers under the ObjC calling convention), convert to view coordinates
(`convertPoint:fromView:nil`), flip Y (NSView is bottom-left origin; both the grid
and `canvas::Point` are top-left), then:

- **TermView**: divide by `TV_CELL_W`/`TV_CELL_H` → `(col, row)`, clamp to the
  grid.
- **MFBCanvasView**: take the flipped view coordinates directly as pixels, clamp
  to the surface extent.

Encode the SGR report (button/motion/modifier bits per plan-94-B §3, coordinates
+1 back to 1-based) and write the bytes to the window input pipe. `scrollWheel:`
maps `deltaY` sign to `ScrollUp`/`ScrollDown`. Modifiers come from
`[NSEvent modifierFlags]`.

**The gate.** Each IMP's first act is to load `_mfb_rt_mouse_mode` and return
immediately unless it holds this view's value (`1` for TermView, `2` for
CanvasView). IMPs are always registered; the word decides whether they do
anything. That is simpler than installing and removing an `NSTrackingArea` on
enable/disable, and it matches the `didResize` precedent of a state read inside an
always-present IMP.

`mouseMoved:` is the one override that needs more than the gate: a view receives
it only with a tracking area installed (or `acceptsMouseMovedEvents` on the
window). Phase 2 installs a tracking area once, at view construction, and lets the
mode word gate the emission — so the cost of an un-enabled program is a callback
that loads one word and returns.

**Risk.** This is hand-written AArch64 in ObjC-callback context: no `x19` arena
base, `NSPoint` arguments in float registers, a `convertPoint:` `objc_msgSend`
with a struct argument. `.ai/arch-abi.md` is the reference for all three. Bounded
by assembly inspection and the `.app.ncode` golden diff.

## Phases

### Phase 1 — Locate the pipe write + one TermView IMP (mouseDown → Down)

- [ ] Read `KEY_DOWN_SYMBOL`'s body and `PIPE_ASSOC_KEY`; record in Corrections
      whether the pipe write is reusable as-is or needs factoring out, and the
      exact symbol/fd.
- [ ] Add a `mouseDown:` IMP on `TermView`: gate on `_mfb_rt_mouse_mode == 1`,
      px→cell via `TV_CELL_W`/`TV_CELL_H`, format `\x1b[<0;x;yM`, write to the
      pipe. Register it beside `setFrameSize:` (selector + type-encoding tables +
      `class_addMethod`, `src/target/macos_aarch64/app/mod.rs:795`).
- [ ] macOS `emit_app_term_helper` (`app_io.rs:551`): a `term.enableMouse` arm
      writing `_mfb_rt_mouse_mode`, if the shared path does not already cover it.

Acceptance: `cargo build` clean; the emitted `mouseDown:` IMP disassembles to the
expected gate + px→cell + pipe-write; `.app.ncode` for a macOS app fixture shows
exactly the new IMP (regenerate + confirm the diff is only the addition).
Commit: —

### Phase 2 — Full TermView event set

- [ ] `mouseUp:` (Up), `mouseDragged:`/`rightMouseDragged:` (Drag), `mouseMoved:`
      (Move — install the tracking area), `rightMouseDown:`/`rightMouseUp:` and
      `otherMouse*:` (Right/Middle), `scrollWheel:` (ScrollUp/Down from `deltaY`).
- [ ] Modifiers from `modifierFlags` into the SGR bits.
- [ ] Y-flip + clamp verified against the `drawRect:` coordinate origin.

Acceptance: `cargo build` clean; each IMP disassembles to the correct SGR encode;
regenerated `.app.ncode`/`.ncodesum` goldens diff only by the new IMPs. Manual
interactive smoke: build the app, click, confirm the program reports the event.
CI cannot drive a window, so that step is developer-run and documented as such.
Commit: —

### Phase 3 — `MFBCanvasView` (pixels)

- [ ] The same override set on `MFBCanvasView`, gated on
      `_mfb_rt_mouse_mode == 2`, emitting flipped view coordinates as pixels with
      no cell divide, clamped to the published surface extent
      (`GRAPHICS_OFFSET_WIDTH`/`HEIGHT`, `src/codegen/runtime/canvas/mod.rs:122`).
- [ ] `canvas::enableMouse` arm writing `_mfb_rt_mouse_mode = 2`;
      `canvas::pollMouse` reading the ring as `Point`.
- [ ] Install the tracking area on the canvas view for `mouseMoved:`.
- [ ] A `syntax/app` fixture exercising `canvas::enableMouse`+`pollMouse` so the
      macOS `.app.ncode` golden covers the canvas IMPs.

Acceptance: `cargo build` clean; the canvas IMPs disassemble to the expected
pixel path (no divide); goldens diff only by the new IMPs; manual smoke shows a
click reporting pixel coordinates that match where the pointer was.
Commit: —

## Validation Plan

- Tests: none runtime-headless. Add `syntax/app` fixtures exercising
  `term::enableMouse`+`pollMouse` and `canvas::enableMouse`+`pollMouse` so both
  views' IMPs are covered by the macOS `.app.ncode` golden.
- Runtime proof: manual — build the app, click/drag/scroll, confirm the program
  prints the events (documented as a manual step; not in CI).
- Doc sync: `src/docs/spec/app/04_term-backend.md` macOS section — the mouse IMPs,
  px→cell and px→pixel, the mode-word gate and pipe injection.
- Acceptance: `cargo build`; `scripts/test-accept.sh <exe> /tmp/out '*app*'`;
  regenerate affected `.app.ncode`/`.ncodesum` and confirm diffs are only the IMPs.

## Open Decisions

- **`mouseMoved:` delivery** — a tracking area installed once at construction
  (recommended; the mode word gates emission) vs. `acceptsMouseMovedEvents` on the
  window vs. installing/removing the area on enable/disable. (§3)

*(The flag-storage Open Decision the pre-migration draft carried is closed: the
process-global `_mfb_rt_mouse_mode` word, plan-94-A §4.4b.)*

## Corrections

<Filled in during execution — esp. the pipe-write symbol, the Y-flip origin, and
anything the ObjC NSPoint calling convention costs that §3 does not predict.>

## Summary

C is per-backend hand-written assembly with the same headless-test limitation as
the `didResize` macOS work. The injection design keeps each IMP small — gate,
convert, format, write — with no new queue, and the two views differ only in
whether they divide by the cell metrics. Gate is assembly + golden diff + manual
smoke.
