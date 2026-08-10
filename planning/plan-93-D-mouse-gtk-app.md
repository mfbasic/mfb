# plan-93-D: Mouse events — Linux GTK app backend

Last updated: 2026-08-09
Effort: large (3h–1d)
Depends on: plan-93-B (the shared decoder + injection contract)

Wire native mouse input in `--app` mode on Linux/GTK by attaching GTK4 event
controllers to the drawing area, converting each event to cell coordinates, and
**injecting the SGR bytes into the fd-0 window-input pipe the backend already
writes keystrokes to** — decoded by the plan-93-B pump/ring with no GTK-specific
event queue.

Behavioral outcome: a GTK app that `enableMouse(TRUE)` and polls receives the six
`MouseKind`s with correct cell coords/modifiers from real pointer input. Unlike
macOS/Windows, GTK runs on Linux with a display, so this is **partially
runtime-testable** where a display + GTK are available (see `tests/rt_gtk_term_utf8_grid.rs`
for the existing GTK-runtime harness pattern); it is not testable on this macOS
host.

References:

- plan-93-A §3–§4, plan-93-B §3 (pump + injection — read first).
- `src/target/linux_gtk/bootstrap.rs` (where `GtkDrawingArea` signals/controllers
  are connected — the `resize` signal wiring at `:180` and the key controller are
  the mirror), `src/target/linux_gtk/mod.rs` (the `_mfb_gtkapp_state` global with
  `ST_TERM_CELL_W`/`ST_TERM_CELL_H` and the `store_state`/`load_state` accessors),
  `src/target/linux_gtk/term_draw.rs` (the `resize` handler, precedent for a
  main-loop callback that reads cell metrics), `src/target/linux_gtk/app_io.rs`
  (the key-press handler that writes keystrokes to the window-input pipe —
  `IO_*`/pipe symbols).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-93-B complete | CLI mouse rt test passes | NOT MET |
| GTK keystroke→pipe write located | grep `linux_gtk/app_io.rs`/`bootstrap.rs` (Phase 1) | UNVERIFIED |

> If plan-93-B is not complete, this sub-plan cannot start, full stop.

## 1. Goal

- A `GtkGestureClick` (press/release → Down/Up, button from
  `gtk_gesture_single_get_current_button`), a `GtkEventControllerMotion`
  (motion → Move/Drag), and a `GtkEventControllerScroll` (scroll → ScrollUp/Down)
  are attached to the term drawing area.
- Each handler converts px→cell via `ST_TERM_CELL_W`/`ST_TERM_CELL_H` (the same
  metrics the `resize` handler uses), formats the SGR report, and writes the bytes
  to the window-input pipe.
- Controllers/emission are inert until `enableMouse(TRUE)` (a `_mfb_gtkapp_state`
  mouse-enabled flag, set by the GTK `enableMouse` arm, gates the handlers) so the
  pipe isn't flooded with motion when mouse is off.

### Non-goals

- No change to CLI, macOS, or Windows; no new event queue (bytes → pipe → pump).
- No change to the grid/redraw path.

## 2. Current State

GTK term callbacks run on the GTK main loop and reach state through the
address-based `_mfb_gtkapp_state` global (`load_state`/`store_state`), never x19 —
established by the `didResize` GTK work, which added a genuine-change-detecting flag
to the `resize` handler (`term_draw.rs`) and a `didResize` arm reading the state
global (`app_io.rs`). `ST_TERM_CELL_W`/`ST_TERM_CELL_H` hold px-per-cell
(`mod.rs`), so px→cell is `floor(px/cell)`. The key-press handler already writes
keystroke bytes into the fd-0 window-input pipe the worker reads as stdin — mouse
mirrors that write.

### Verified properties

- **Cell metrics live in `_mfb_gtkapp_state`** (`ST_TERM_CELL_W`/`H`), read by the
  `resize` handler — reusable for px→cell. Verified during the didResize GTK work.
- **The state global is address-based (thread-independent)** — so a main-loop
  controller callback reaches it without x19 (the reason `didResize` used it).
- **Keystroke→pipe write exists** — UNVERIFIED exact symbol; Phase 1 greps
  `app_io.rs` for the pipe write the key handler uses.

## 3. Design

Attach the three controllers in `bootstrap.rs` beside the existing `resize`/key
wiring. Handlers (main loop): read event coords (widget-relative px), divide by
cell metrics, clamp, encode SGR (plan-93-B §3), write to the pipe. `GtkGestureClick`
gives button + n-press; modifiers via `gtk_event_controller_get_current_event_state`.
Scroll deltas → ScrollUp/Down. A `_mfb_gtkapp_state` mouse-enabled flag (set by the
GTK `enableMouse` arm) gates emission.

Risk: hand-written neutral-abi assembly in GTK-callback context (x19 not the arena
base — use `store_state`/`load_state` and the pipe write, exactly as the didResize
resize handler does). No committed GTK app `.ncode` golden exists, so the gate is
compile + a GTK-runtime smoke test where a display is available.

## Phases

### Phase 1 — Click gesture (Down/Up) + enable flag + pipe write

- [ ] Locate the keystroke→pipe write symbol; record in Corrections.
- [ ] Add a `GtkGestureClick` on the drawing area in `bootstrap.rs`; press/release
      handlers in `term_draw.rs`/`app_io.rs`: px→cell, SGR encode, pipe write.
- [ ] Add a `_mfb_gtkapp_state` mouse-enabled flag; GTK `enableMouse` arm sets it;
      handlers early-return when clear.

Acceptance: `cargo build` clean; where a display+GTK exist, a runtime smoke
(pattern of `rt_gtk_term_utf8_grid.rs`) shows a click yields a `Down`/`Up` event
with correct coords.
Commit: —

### Phase 2 — Motion + scroll + modifiers

- [ ] `GtkEventControllerMotion` → Move/Drag (button-held detection);
      `GtkEventControllerScroll` → ScrollUp/Down; modifier bits.

Acceptance: `cargo build` clean; runtime smoke (display available) shows drag +
scroll events; motion does not fire when mouse mode is off.
Commit: —

## Validation Plan

- Tests: extend/add a GTK-runtime test (`tests/rt_gtk_term_*`) where CI has a
  display; otherwise compile-only.
- Runtime proof: GTK smoke — click/drag/scroll print expected events.
- Doc sync: term-backend spec Linux/GTK section — controllers + px→cell + pipe
  injection.
- Acceptance: `cargo build`; GTK-runtime test where available.

## Open Decisions

- **Motion gating** — enable-flag early-return in the handler (recommended) vs.
  add/remove the motion controller on enable/disable. (§3)

## Corrections

<Filled in during execution — pipe symbol, widget coordinate origin.>

## Summary

D mirrors C's shape on GTK's controller model, reusing `_mfb_gtkapp_state` +
the existing key→pipe write. Partially runtime-testable (display required), unlike
C/E.
