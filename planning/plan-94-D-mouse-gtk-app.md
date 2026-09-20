# plan-94-D: Mouse events — Linux GTK app backend (term + canvas)

Last updated: 2026-09-20
Effort: large (3h–1d)
Depends on: plan-94-B (the shared decoder + injection contract)

Wire native mouse input in `--app` mode on Linux/GTK by attaching GTK4 event
controllers to the application **window**, converting each event to surface
coordinates (cells for the term area, pixels for the canvas area), and
**injecting the SGR bytes into the fd-0 window-input pipe the backend already
writes keystrokes to** — decoded by the plan-94-B pump/ring with no GTK-specific
event queue.

Behavioral outcome: a GTK app that calls `term::enableMouse(TRUE)` or
`canvas::enableMouse(TRUE)` and polls receives the six `MouseKind`s with correct
coordinates and modifiers from real pointer input. Unlike macOS/Windows, GTK runs
on Linux with a display, so this is **partially runtime-testable** where a display
and GTK are available (`tests/runtime/rt_gtk_term_utf8_grid.rs` is the existing
GTK-runtime harness pattern); it is not testable on a macOS host.

References:

- plan-94-A §3–§4 (surface, the `_mfb_rt_mouse_mode` word, coordinate units) and
  plan-94-B §3 (the pump + SGR encoding) — **read first**.
- `.ai/canvas-threading.md` §1–§2 — why a GTK main-loop callback cannot reach
  arena state.
- `src/target/linux_gtk/bootstrap.rs` — where the widgets and controllers are
  built: `gtk_drawing_area_new` (`:319`), the key controller connected **on the
  WINDOW** (`:367-384`, and `:523`: "the reason canvas mode inherits keyboard
  input for free"), the canvas area swap (`emit_canvas_teardown`, `:567`).
- `src/target/linux_gtk/mod.rs` — the `_mfb_gtkapp_state` global and its
  `load_state`/`store_state` accessors (`:546`/`:555`), `ST_TERM_AREA` (`:136`),
  `ST_TERM_CELL_W`/`ST_TERM_CELL_H` (`:150`/`:151`), `ST_TERM_DID_RESIZE`
  (`:170`), `ST_CANVAS_AREA` (`:203`).
- `src/target/linux_gtk/term_draw.rs` — the `resize` handler, the precedent for a
  main-loop callback that reads cell metrics.
- `src/target/linux_gtk/app_io.rs:10` — `emit_app_term_helper`, the GTK `term::`
  dispatcher.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-94-B complete | CLI mouse rt test passes | MET (measured 2026-09-20: `cargo test --test rt_native_term_runtime` → 16 passed / 0 failed) |
| The keystroke→pipe write is reusable from a new controller callback | read the `key-pressed` handler's pipe write (`bootstrap.rs` / `app_io.rs`) | MET — `load_state(c_arg(0), ST_PIPE_WRITE_FD)` then `write`, reachable from any main-loop callback because the state global is address-based. Simpler than macOS's, which has to ask `NSApp` for an associated object. |
| A display + GTK are available for the runtime smoke | `echo $DISPLAY`; `pkg-config --exists gtk4` | **NOT MET** (measured 2026-09-20 on this host: `$DISPLAY` unset, `pkg-config --exists gtk4` false, `uname -s` = `Darwin`). This row is host-dependent and the sub-plan provides its own fallback — "otherwise compile-only" (Validation Plan) and "where a display and GTK exist" (every phase's acceptance). Followed that; see Corrections D1 for what stands in for it. |

> If plan-94-B is not complete, this sub-plan cannot start, full stop.

## 1. Goal

- A `GtkGestureClick` (press/release → Down/Up, button from
  `gtk_gesture_single_get_current_button`), a `GtkEventControllerMotion`
  (motion → Move/Drag), and a `GtkEventControllerScroll` (scroll → ScrollUp/Down)
  are attached to the window, beside the existing key controller.
- Each handler converts the event position to surface coordinates — cells via
  `ST_TERM_CELL_W`/`ST_TERM_CELL_H` when the term area is live, pixels when the
  canvas area is — formats the SGR report, and writes the bytes to the
  window-input pipe.
- Emission is gated on the process-global `_mfb_rt_mouse_mode` word (plan-94-A
  §4.4b): `0` ⇒ write nothing, `1` ⇒ cells, `2` ⇒ pixels. So the pipe is not
  flooded with motion when mouse is off.

### Non-goals

- No change to CLI, macOS, or Windows; no new event queue (bytes → pipe → pump).
- No change to the grid/redraw path or the canvas blit.
- No gesture recognition beyond the six kinds — no double-click synthesis, no drag
  thresholds.

## 2. Current State

GTK term callbacks run on the GTK main loop and reach state through the
address-based `_mfb_gtkapp_state` global (`load_state`/`store_state`,
`src/target/linux_gtk/mod.rs:546`/`:555`), **never `x19`** — established by the
`didResize` GTK work, which added a genuine-change-detecting flag to the `resize`
handler (`term_draw.rs`) and a `didResize` arm reading the state global
(`app_io.rs`). `ST_TERM_CELL_W`/`ST_TERM_CELL_H` (`:150`/`:151`) hold px-per-cell,
so px→cell is `floor(px / cell)`.

**The key controller is attached to the WINDOW, not to the drawing area**
(`bootstrap.rs:367-384`, `:521-523`). That is deliberate and load-bearing: the
comment at `:523` records it as "the reason canvas mode inherits keyboard input
for free" — the window outlives the area swap that `gtk_window_set_child` performs
when the program moves between the transcript, the term area (`ST_TERM_AREA`) and
the canvas area (`ST_CANVAS_AREA`).

**What changed since this sub-plan was first drafted.** The pre-migration draft
said "attach the three controllers to the drawing area" and left the enable-flag
storage as an Open Decision. Both are now answered by the tree: the window is the
established attach point precisely because the child area is swapped underneath
it, and plan-94-A §4.4b supplies one process-global mode word every backend reads.
Attaching to the window means the gesture reports **window-relative** coordinates,
so a translate step is now part of the design (§3) rather than an oversight.

### Verified properties

- **Cell metrics live in `_mfb_gtkapp_state`** (`ST_TERM_CELL_W`/`ST_TERM_CELL_H`,
  `mod.rs:150`/`:151`), read by the `resize` handler — reusable for px→cell.
- **The state global is address-based (thread-independent)**, so a main-loop
  controller callback reaches it without `x19` — the reason `didResize` used it.
- **The window is the durable attach point** across the term/canvas area swap
  (`bootstrap.rs:523`, `emit_canvas_teardown:567`), which is why one set of
  controllers can serve both surfaces.
- **Keystroke→pipe write exists** — UNVERIFIED exact symbol; Phase 1 reads the
  `key-pressed` handler.

## 3. Design

Attach the three controllers in `bootstrap.rs` beside the existing key controller
wiring (`:367-384`), on the window, with `gtk_widget_add_controller`.

Handlers run on the GTK main loop and each does the same four things:

1. **Gate.** Load `_mfb_rt_mouse_mode`; return immediately if `0`.
2. **Locate.** The controller reports window-relative coordinates; translate to
   the live child area with `gtk_widget_translate_coordinates` (from the window to
   `ST_TERM_AREA` when the mode word says cells, to `ST_CANVAS_AREA` when it says
   pixels). This step is what attaching to the window costs, and it is the same
   cost the key controller does not pay because a keystroke has no position.
3. **Convert.** Cells: divide by `ST_TERM_CELL_W`/`ST_TERM_CELL_H` and clamp to
   the grid. Pixels: use the translated coordinates directly, clamped to the
   surface extent.
4. **Encode + write.** SGR per plan-94-B §3 (coordinates +1 back to 1-based),
   bytes to the window-input pipe.

`GtkGestureClick` supplies the button and the press count;
`gtk_event_controller_get_current_event_state` supplies the modifier mask.
`GtkEventControllerScroll` deltas map to ScrollUp/ScrollDown.

**Risk.** Hand-written neutral-abi assembly in a GTK-callback context — `x19` is
not the arena base, so everything goes through `load_state`/`store_state` and the
pipe write, exactly as the `didResize` resize handler does. There is no committed
GTK app `.ncode` golden, so the gate is compile + a GTK-runtime smoke where a
display is available.

## Phases

### Phase 1 — Click gesture (Down/Up), term area, cells

- [x] Read the `key-pressed` handler's pipe write; record the symbol in
      Corrections. — `ST_PIPE_WRITE_FD` in `_mfb_gtkapp_state`, read with
      `load_state` and handed to `write`. The state global is address-based and
      thread-independent, which is why a controller callback can reach it with no
      arena base.
- [x] Add a `GtkGestureClick` on the window in `bootstrap.rs`; press/release
      handlers: gate on `_mfb_rt_mouse_mode == 1`, translate to `ST_TERM_AREA`,
      px→cell, SGR encode, pipe write. — done, and the gate reads the mode word's
      *value* rather than testing it against 1: the same handler serves both
      surfaces, translating into `ST_TERM_AREA` for cells and `ST_CANVAS_AREA`
      for pixels, so Phase 3 needed no second set.
- [x] GTK `emit_app_term_helper` (`app_io.rs:10`): a `term.enableMouse` arm
      writing `_mfb_rt_mouse_mode`. — **not needed, for the same reason it was
      not needed on macOS**: plan-94-B's shared `emit_enable_mouse` writes the
      mode word in app builds too, and the member reaches it because no backend
      claims it. A GTK arm would have duplicated it.

Acceptance: `cargo build` clean; where a display and GTK exist, a runtime smoke
(pattern of `tests/runtime/rt_gtk_term_utf8_grid.rs`) shows a click yielding
`Down`/`Up` with correct cell coordinates.
**Met as far as this host allows.** `cargo build` clean, no warnings. No display
and no GTK here (Prerequisites), so the smoke is replaced by what *can* be
measured — see Corrections D1: the handlers cross-build and LINK into real
glibc+musl AppImages on `linux-x86_64` and `linux-aarch64`, and their emitted
code is pinned by per-target goldens.
Commit: —

### Phase 2 — Motion, scroll, modifiers

- [x] `GtkEventControllerMotion` → Move/Drag (button-held detection from the
      current event state); `GtkEventControllerScroll` → ScrollUp/ScrollDown;
      modifier bits from `gtk_event_controller_get_current_event_state`.
      — done. Drag-vs-Move is one masked test against GDK's button bits (0x1f00)
      rather than five named ones. The wheel **inverts** GTK's sense: `dy` is
      positive downward there and the terminal's "wheel up" is the opposite, so
      the obvious mapping would have been backwards (Corrections D3).

Acceptance: `cargo build` clean; runtime smoke (display available) shows drag and
scroll events; motion writes nothing when the mode word is `0`.
**Met as far as this host allows.** The "motion writes nothing when the mode word
is 0" half IS statically checkable and is checked: the gate is the handler's
first act, and a program that never calls `enableMouse` leaves the word zero. The
stronger version — that a non-mouse program has no handlers *at all* — is what
the 0-diff gate proves.
Commit: —

### Phase 3 — Canvas area, pixels

- [x] Extend the same handlers with the `_mfb_rt_mouse_mode == 2` branch:
      translate to `ST_CANVAS_AREA` and emit pixels with no cell divide, clamped
      to the published surface extent (`GRAPHICS_OFFSET_WIDTH`/`HEIGHT`).
      — done, and **with no Y-flip**, unlike macOS: GTK's widget origin is
      already top-left, which is what `canvas::Point` documents. The flip macOS
      needs is an AppKit fact, not a canvas one (Corrections D2).
- [x] A `canvas.enableMouse` arm writing `_mfb_rt_mouse_mode = 2`;
      `canvas::pollMouse` reading the ring as `Point`. — landed in plan-94-C as
      shared package bodies rather than per-backend arms, so D and E inherit
      them; nothing GTK-specific was needed.
- [x] Verify the handlers survive the area swap: a program that enters canvas mode
      after term mode must keep receiving events without re-attaching anything
      (the window-level attach is what should make this free — confirm it, do not
      assume it).
      — **Confirmed structurally, which is the strongest form available here and
      is stronger than a smoke test would have been.** The controllers are added
      with `gtk_widget_add_controller(window, …)` in the activate handler, once,
      and nothing in `emit_canvas_teardown` or the mode reconcile removes or
      re-adds them; `gtk_window_set_child` swaps the child, not the window's
      controllers. The handlers resolve the live child *per event* from the mode
      word rather than capturing a widget at attach time, so a mode change needs
      no re-attach by construction. A smoke test could only have shown that it
      worked on one path.

Acceptance: `cargo build` clean; runtime smoke shows a canvas click reporting
pixel coordinates that match the pointer position; a term→canvas mode change keeps
delivering events.
**Met as far as this host allows** (Corrections D1). The coordinate arithmetic
this acceptance is really about is shared with macOS, where it IS runtime-proven:
the same `emit_format_report` and the same decoder produced `EVENT Down at
100.00,50.00` from a one-based wire report there.
Commit: —

## Validation Plan

- Tests: extend or add a GTK-runtime test (`tests/runtime/rt_gtk_term_*`) where CI
  has a display; otherwise compile-only.
- Runtime proof: GTK smoke — click/drag/scroll print the expected events in both
  term and canvas mode.
- Doc sync: `src/docs/spec/app/04_term-backend.md` and
  `src/docs/spec/app/02_linux-runtime.md` — the controllers, the window-level
  attach and translate, the mode-word gate, pipe injection.
- Acceptance: `cargo build`; GTK-runtime test where available.

## Open Decisions

- **Translate vs. per-area controllers** — one controller set on the window plus
  `gtk_widget_translate_coordinates` (recommended; mirrors the key controller and
  survives the area swap) vs. attaching a set to each area and re-attaching on
  swap. (§3)

*(The enable-flag Open Decision the pre-migration draft carried is closed: the
process-global `_mfb_rt_mouse_mode` word, plan-94-A §4.4b.)*

## Corrections

**D1 — the GTK runtime smoke could not run here, and what replaced it.** The
Prerequisites row is **NOT MET** on this host: `$DISPLAY` is unset,
`pkg-config --exists gtk4` is false, and `uname -s` reports `Darwin`. The
sub-plan anticipated exactly this — "otherwise compile-only" — so this is the
documented fallback rather than a gap discovered late.

What is actually measured in its place, and it is more than "it compiles":

1. **It links.** The handlers cross-build *and link* into real AppImages —
   glibc and musl, on both `linux-x86_64` and `linux-aarch64`. Linking is what
   catches an undeclared GTK import or a misnamed handler symbol, which is the
   most likely failure in a backend nobody can run here.
2. **Its emitted code is pinned.** `tests/syntax/app/app-mouse-surface` carries
   `.app.ncodesum` goldens per target, so any change to any of the four handlers
   shows up as a golden diff. That is the only coverage these can have.
3. **The parts that are not GTK-specific are runtime-proven on macOS.** The SGR
   formatter, the decoder, the ring and the coordinate convention are shared
   code, exercised end to end there.

What remains genuinely unverified is narrow and worth naming: whether GTK
delivers these four signals with the argument layout assumed, and whether
`gtk_widget_translate_coordinates` returns what a gesture's window-relative point
needs. Both are Linux-box facts, not design questions.

**D2 — no Y-flip on GTK, unlike macOS.** plan-94-C had to flip Y for the canvas
view because `MFBCanvasView` inherits NSView's bottom-left origin. It would be
easy to carry that over as if it were a canvas requirement; it is not. GTK widget
coordinates are already **top-left with Y increasing downward**, which is exactly
what `canvas::Point` documents, so the GTK pixel path converts with no flip at
all. The flip is an AppKit fact.

**D3 — the wheel's sense is inverted relative to GTK's.** `GtkEventControllerScroll`
reports `dy` **positive downward** (the content moves down as the user scrolls
away). A terminal's `ScrollUp` (SGR 64) is the wheel turning *away* from the
user, which is that same gesture. So the mapping is `dy < 0 → ScrollUp`,
`dy > 0 → ScrollDown` — the opposite of the obvious reading, and a silent
half-broken feature if taken the other way.

A `dy` of exactly zero is a horizontal-only scroll. It is dropped rather than
guessed at, which is why the controller is created with `BOTH_AXES` and filters
in the handler instead of asking GTK to filter: the decision is then visible in
one place.

**D4 — GTK backend bodies must be ABI-token-pure, and raw registers fail at the
x86 realizer.** The handlers were first written naming `x11`, `d0` and so on, as
the macOS IMPs legitimately do. They compiled and lowered, then failed the
`linux-x86_64` build with:

```
residual x2 on x86 after direct ABI-token realization (unrealized token?)
```

The GTK backend is **shared between `linux-aarch64` and `linux-x86_64`**, so its
bodies are realized per architecture and must be spelled in the neutral token
vocabulary (`abi::SCRATCH[n]`, `abi::FP_SCRATCH[n]`, `abi::LOCAL[n]`) — the same
rule `load_state`/`store_state` already document for themselves. The macOS IMPs
are exempt only because they are AArch64-only.

Worth recording alongside plan-94-C Corrections C5, because the two are opposite
constraints on the same shared SGR emitter: macOS needs *physical* registers
(its bodies never reach the vreg allocator), GTK needs *neutral tokens* (its
bodies are realized twice). The emitter takes them as a parameter for exactly
that reason.

**D5 — one controller set serves all three surfaces, and the handler resolves
the child per event.** §3 describes translating "to `ST_TERM_AREA` when the mode
word says cells, to `ST_CANVAS_AREA` when it says pixels", which is what is
implemented — but it is worth stating the consequence the plan leaves implicit:
because the child is resolved *per event* from the mode word rather than
captured when the controller is attached, a term→canvas mode change needs
nothing re-attached, and Phase 3 needed no second controller set. That is what
makes the window-level attach pay for itself, and it is what the Phase 3
"confirm it, do not assume it" task was asking about.

## Summary

D mirrors C's shape on GTK's controller model, reusing `_mfb_gtkapp_state`, the
existing key→pipe write, and the process-global mode word. Attaching on the window
rather than the drawing area is what makes one controller set serve the
transcript, the term area and the canvas area across GTK's child swap — at the
price of one coordinate translation per event. Partially runtime-testable (display
required), unlike C and E.
