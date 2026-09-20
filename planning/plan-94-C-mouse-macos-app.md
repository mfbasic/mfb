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
| plan-94-B complete | CLI mouse rt test passes; the decoder consumes injected bytes from the input pipe | MET (measured 2026-09-20: every B box ticked at `7eed59fb2`; `cargo test --test rt_native_term_runtime` → 16 passed / 0 failed, including all seven mouse cases) |
| The keystroke→pipe write is reusable from a new IMP | read `KEY_DOWN_SYMBOL`'s body and `PIPE_ASSOC_KEY`; confirm the write is a callable sequence, not inlined one-off | MET — **but it is an inlined one-off, not a callable sequence.** See Corrections C1; the IMPs reproduce the three-step sequence rather than calling it. |

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

- [x] Read `KEY_DOWN_SYMBOL`'s body and `PIPE_ASSOC_KEY`; record in Corrections
      whether the pipe write is reusable as-is or needs factoring out, and the
      exact symbol/fd. — **inlined, not callable** (Corrections C1). The fd is an
      associated object on `NSApp` under `PIPE_ASSOC_KEY`
      (`_mfb_macapp_pipe_key`); the write is `_write` with the fd, a buffer and a
      length, open-coded inside `kd_commit`.
- [x] Add a `mouseDown:` IMP on `TermView`: gate on `_mfb_rt_mouse_mode == 1`,
      px→cell via `TV_CELL_W`/`TV_CELL_H`, format `\x1b[<0;x;yM`, write to the
      pipe. Register it beside `setFrameSize:` (selector + type-encoding tables +
      `class_addMethod`).
      — done, and **generalised**: one parameterised body
      (`app/mouse_view.rs::emit_mouse_imp`) produces all eleven per view from a
      table, so Phase 2's set arrived with Phase 1's rather than as nine more
      hand-written bodies.
- [x] macOS `emit_app_term_helper` (`app_io.rs:551`): a `term.enableMouse` arm
      writing `_mfb_rt_mouse_mode`, if the shared path does not already cover it.
      — **not needed; the shared path already covers it.** plan-94-B's
      `emit_enable_mouse` writes the mode word in both console and app builds
      (only the terminal escapes are suppressed in app mode), and the member is
      not in any app dispatch, so it reaches that shared body. Adding a macOS arm
      would have duplicated it.

Acceptance: `cargo build` clean; the emitted `mouseDown:` IMP disassembles to the
expected gate + px→cell + pipe-write; `.app.ncode` for a macOS app fixture shows
exactly the new IMP (regenerate + confirm the diff is only the addition).
**Met.** `cargo build` clean, no warnings. `macapp.mouse.term.mouseDown`
disassembles to exactly the predicted shape: `ldr` of `_mfb_rt_mouse_mode` →
`cmp_imm 1` → `b.ne done`; `locationInWindow` into d0/d1;
`convertPoint:fromView:` with a nil source; the cell divide; then
`_objc_getAssociatedObject` on `_mfb_macapp_pipe_key` and `_write`. No app
fixture diffed at all (see Corrections C3 for the two rounds it took).
Commit: —

### Phase 2 — Full TermView event set

- [x] `mouseUp:` (Up), `mouseDragged:`/`rightMouseDragged:` (Drag), `mouseMoved:`
      (Move — install the tracking area), `rightMouseDown:`/`rightMouseUp:` and
      `otherMouse*:` (Right/Middle), `scrollWheel:` (ScrollUp/Down from `deltaY`).
      — all eleven per view. The tracking area is installed once at view
      construction, with `NSTrackingInVisibleRect` so a window resize needs no
      re-install; without it `mouseMoved:` is registered and never called, which
      would have left drags working and plain motion silently dead.
- [x] Modifiers from `modifierFlags` into the SGR bits. — done, including on the
      wheel (a shift-scroll is a real gesture).
- [x] Y-flip + clamp verified against the `drawRect:` coordinate origin.
      — **verified, and the answer is not what the plan assumed**: the flip is
      needed on one view and wrong on the other (Corrections C2). Off-surface
      coordinates are *rejected* rather than clamped for cells (Corrections C4).

Acceptance: `cargo build` clean; each IMP disassembles to the correct SGR encode;
regenerated `.app.ncode`/`.ncodesum` goldens diff only by the new IMPs. Manual
interactive smoke: build the app, click, confirm the program reports the event.
CI cannot drive a window, so that step is developer-run and documented as such.
**Met**, and the smoke step turned out not to need a human. The disassembly
distinguishes the two variants exactly as designed: the term IMP contains
`fdiv_d` (the cell divide) and **no** `fsub_d`, the canvas IMP contains `fsub_d`
(the Y-flip) and **no** `fdiv_d`, and both end in `fcvtms_x_from_d`. Goldens: 0
diffs tree-wide. And `MFB_MOUSE_INJECT` (plan-94-B) drives the decoder without a
mouse, so the end-to-end path IS machine-checked — see Phase 3.
Commit: —

### Phase 3 — `MFBCanvasView` (pixels)

- [x] The same override set on `MFBCanvasView`, gated on
      `_mfb_rt_mouse_mode == 2`, emitting flipped view coordinates as pixels with
      no cell divide, clamped to the published surface extent
      (`GRAPHICS_OFFSET_WIDTH`/`HEIGHT`).
- [x] `canvas::enableMouse` arm writing `_mfb_rt_mouse_mode = 2`;
      `canvas::pollMouse` reading the ring as `Point`. — both real bodies now.
      `pollMouse` builds the inlined-`Point` layout plan-94-A Corrections C5
      established, converts the ring's integer coordinates to `Float`, and
      **transposes the pair**: the decoder stores wire-`y` in `coord_a` because
      `term::MouseEvent` is row-first, while `canvas::Point` is x-first.
- [x] Install the tracking area on the canvas view for `mouseMoved:`.
- [x] A `syntax/app` fixture exercising `canvas::enableMouse`+`pollMouse` so the
      macOS `.app.ncode` golden covers the canvas IMPs.
      — `tests/syntax/app/app-mouse-surface`, exercising **both** surfaces and
      carrying `.app.ncodesum` goldens for all four targets, so it serves
      plan-94-D and plan-94-E too rather than each adding its own.

Acceptance: `cargo build` clean; the canvas IMPs disassemble to the expected
pixel path (no divide); goldens diff only by the new IMPs; manual smoke shows a
click reporting pixel coordinates that match where the pointer was.
**Met, and the smoke is machine-checked rather than manual.** Running the built
`.app` with `MFB_MOUSE_INJECT='\033[<0;101;51M\033[<0;101;51m\033[<64;5;5M'`
prints `EVENT Down at 100.00,50.00` / `EVENT Up at 100.00,50.00` / `EVENT
ScrollUp at 4.00,4.00` / `DRAINED` — one-based wire coordinates arriving as
zero-based `Float`s in the right fields, which is simultaneously proof of the
inlined-`Point` layout, the transposition, the integer→float conversion, the
ring and the `Mode.Canvas` gate. The canvas IMPs contain no `fdiv_d`. Gate: 0
diffs across 2066 goldens.
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

**C1 — the keystroke pipe write is inlined, not a callable sequence.** The
Prerequisites row asked which it was; the answer is inlined. `KEY_DOWN_SYMBOL`'s
`kd_commit` arm open-codes three steps — `objc_msgSend(NSApp, sharedApplication)`,
`objc_getAssociatedObject(app, PIPE_ASSOC_KEY)` for the write fd, then `_write`
— interleaved with line-buffer management that has nothing to do with the write
(`term_view.rs`, the `kd_commit_write` loop).

Not factored out, and deliberately not: `kd_commit`'s version carries a
resume-after-partial-write loop it needs because a typed line can exceed
`PIPE_BUF` (bug-241). A mouse report is at most 21 bytes and so can never split,
so the IMPs emit the plain three-step sequence (`emit_pipe_write`) instead of
inheriting a loop that would never run a second iteration. The fd and the key are
the same ones.

**C2 — the Y-flip is needed on ONE view, and the plan called for both.** §3 says
to "flip Y (NSView is bottom-left origin; both the grid and `canvas::Point` are
top-left)" for both views. Measured:

- **`TermView` overrides `isFlipped` to return YES**
  (`emit_term_view_is_flipped`, `term_view.rs:257`). Its coordinate system is
  therefore *already* top-left, and `convertPoint:fromView:nil` hands back
  top-left coordinates. Flipping again would have put every event on the wrong
  half of the window — a bug that looks like working code, because the corner
  cases still land on real cells.
- **`MFBCanvasView` does not.** It overrides exactly `acceptsFirstResponder`,
  `keyDown:` and (conditionally) `setFrameSize:` (`bootstrap.rs`, the canvas
  class-synthesis block). So it inherits NSView's bottom-left origin and the flip
  IS required to reach the top-left origin `canvas::Point` documents.

Implemented as `y = surfaceHeight - y` on the canvas view only, from the
published extent rather than the view's bounds: the published extent is what the
program's `DrawItem`s and hit tests are written against, and `setFrameSize:`
keeps the two in step (plan-98-D Phase 3). Verified in the disassembly — the
canvas IMP carries `fsub_d` and the term IMP does not.

**C3 — gating a backend feature means gating its DATA too, not just its code.**
Two rounds of golden failures, both the same mistake in different clothes, and
both caught by `artifact-gate.sh` rather than by reasoning:

1. `_OBJC_CLASS_$_NSTrackingArea` was added to the fixed `app_mode_imports()`
   list. An unused import is still recorded in the native plan, so all three
   pre-existing macOS app fixtures diffed.
2. The nineteen mouse selector C-strings were added to the always-emitted
   `app_mode_data_objects()` table — nineteen dead strings in every app binary
   that never touches the mouse, which is bug-326-A21 exactly.

Both are now `uses_mouse`-gated, which meant threading that flag through
`NativePlanPlatform::app_mode_imports` and `CodegenPlatform::app_mode_data_objects`
(and adding `module_uses_mouse` in the plan layer, keyed on the member-symbol
suffix for the reason plan-94-A Corrections C2 gives). Re-measured: **0 diffs
across 2066 goldens**, with every pre-existing app fixture byte-identical.

Generalises to D and E: the rule is not "emit the handler conditionally" but
**"emit everything the handler names conditionally"** — imports, selector
strings, class bindings, all of it.

**C4 — off-surface coordinates are rejected, not clamped.** §1 says "clamp to the
grid". Clamping is wrong for the cell surface: a click on the window chrome, or a
drag that leaves the window, would be reported as a click on the nearest edge
cell — a phantom event at a real coordinate, which a program cannot tell from a
genuine one. The IMPs therefore test `0 <= v < limit` and return without emitting
when it fails, so the program simply sees nothing, which is the truth.

The test is **signed**, and that matters: a coordinate outside the window is
genuinely negative, and an unsigned compare would reject it only by accident of
wrapping to a huge value.

**C5 — the shared SGR formatter must name physical registers.** The
report-building emitter was first written against `Vregs`, like the console-side
mouse code. It compiled, produced a correct-looking `.ncode` dump, and then failed
at assembly: `error: unknown AArch64 register '%v0'`.

The backend bodies are hand-written `Asm` functions that never reach the vreg
allocator, so a `%v0` travels to the assembler verbatim. The formatter now takes
an explicit `SgrScratch` register set the caller chooses (x9–x15 on the macOS
IMPs, where nothing of theirs is live). Worth recording because the `.ncode` dump
looked entirely healthy — the failure is at the *next* stage, so inspecting the
dump alone would not have found it.

## Summary

C is per-backend hand-written assembly with the same headless-test limitation as
the `didResize` macOS work. The injection design keeps each IMP small — gate,
convert, format, write — with no new queue, and the two views differ only in
whether they divide by the cell metrics. Gate is assembly + golden diff + manual
smoke.
