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
| plan-94-B complete | CLI mouse rt test passes | NOT MET |
| The keystroke→pipe write is reusable from a new controller callback | read the `key-pressed` handler's pipe write (`bootstrap.rs` / `app_io.rs`) | UNVERIFIED (Phase 1 first task) |
| A display + GTK are available for the runtime smoke | `echo $DISPLAY`; `pkg-config --exists gtk4` | UNVERIFIED (host-dependent) |

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

- [ ] Read the `key-pressed` handler's pipe write; record the symbol in
      Corrections.
- [ ] Add a `GtkGestureClick` on the window in `bootstrap.rs`; press/release
      handlers: gate on `_mfb_rt_mouse_mode == 1`, translate to `ST_TERM_AREA`,
      px→cell, SGR encode, pipe write.
- [ ] GTK `emit_app_term_helper` (`app_io.rs:10`): a `term.enableMouse` arm
      writing `_mfb_rt_mouse_mode`.

Acceptance: `cargo build` clean; where a display and GTK exist, a runtime smoke
(pattern of `tests/runtime/rt_gtk_term_utf8_grid.rs`) shows a click yielding
`Down`/`Up` with correct cell coordinates.
Commit: —

### Phase 2 — Motion, scroll, modifiers

- [ ] `GtkEventControllerMotion` → Move/Drag (button-held detection from the
      current event state); `GtkEventControllerScroll` → ScrollUp/ScrollDown;
      modifier bits from `gtk_event_controller_get_current_event_state`.

Acceptance: `cargo build` clean; runtime smoke (display available) shows drag and
scroll events; motion writes nothing when the mode word is `0`.
Commit: —

### Phase 3 — Canvas area, pixels

- [ ] Extend the same handlers with the `_mfb_rt_mouse_mode == 2` branch:
      translate to `ST_CANVAS_AREA` and emit pixels with no cell divide, clamped
      to the published surface extent (`GRAPHICS_OFFSET_WIDTH`/`HEIGHT`,
      `src/codegen/runtime/canvas/mod.rs:122`).
- [ ] A `canvas.enableMouse` arm writing `_mfb_rt_mouse_mode = 2`;
      `canvas::pollMouse` reading the ring as `Point`.
- [ ] Verify the handlers survive the area swap: a program that enters canvas mode
      after term mode must keep receiving events without re-attaching anything
      (the window-level attach is what should make this free — confirm it, do not
      assume it).

Acceptance: `cargo build` clean; runtime smoke shows a canvas click reporting
pixel coordinates that match the pointer position; a term→canvas mode change keeps
delivering events.
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

<Filled in during execution — pipe symbol, the widget coordinate origin, and
whether `gtk_widget_translate_coordinates` is the right call for a gesture's
reported point.>

## Summary

D mirrors C's shape on GTK's controller model, reusing `_mfb_gtkapp_state`, the
existing key→pipe write, and the process-global mode word. Attaching on the window
rather than the drawing area is what makes one controller set serve the
transcript, the term area and the canvas area across GTK's child swap — at the
price of one coordinate translation per event. Partially runtime-testable (display
required), unlike C and E.
