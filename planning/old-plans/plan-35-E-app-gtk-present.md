# plan-35-E: App (GTK) — present-driven redraw + close tearing + resize

Last updated: 2026-07-11
Effort: medium (1h–2h)
Depends on: plan-35-A

Mirror plan-35-D for the GTK backend. GTK already routes `io::write` into the
`_mfb_gtkapp_state` parallel grid arrays while TUI is active; this sub-plan (1)
coalesces the per-write `g_idle_add(redraw)` into a present triggered by
`term::sync()` / `term::off`, (2) closes the documented **tearing** caveat by
presenting a marshaled snapshot on the GTK main loop, and (3) adds grid
resize on window resize (clearing the Phase-6 SCAFFOLD gap the spec notes).
Rendered content is unchanged.

Shared design of record: `planning/plan-35-shadow-grid-unify.md` (§2.3, §4.5).
Encodes D1 (mandatory `sync`; `off` implies a final present). Independent of
console B/C and macOS D (different files), so E can land in parallel after A.

## 1. Goal

- `term::sync()` (and `term::off`'s final present) drives the single
  `gtk_widget_queue_draw`; `_mfb_gtkapp_term_write` no longer `g_idle_add`s a
  redraw per write.
- The present marshals a consistent grid snapshot to the main loop so a draw
  cannot show a torn frame (the worker/draw race in `term_draw.rs:513-528`).
- On window resize, rows/cols recompute, the active extent updates, and a full
  redraw is forced (removing the SCAFFOLD `io::terminalSize`/resize gap noted in
  `04_term-backend.md` §Linux).
- `mfb build -app` `examples/life` (Linux) renders identically to today, repaints
  once per present, no tearing, reflows on resize.

### Non-goals

- No change to console (B/C) or macOS (D).
- No change to what is drawn — glyph/color/caret rendering stays as today.

## 2. Current State

`src/target/linux_gtk/term_draw.rs` + `.../mod.rs`: grid is three parallel static
arrays (chars u8, fg u32, bg u32) at stride `TERM_MAX_COLS=160 × TERM_MAX_ROWS=48`
in the process-wide `_mfb_gtkapp_state`; flags packed into fg/bg (COLOR_SET bit
24, bold 25, underline 26). `_mfb_gtkapp_term_write` (`term_draw.rs:510`) mutates
the arrays and `g_idle_add(_mfb_gtkapp_term_redraw_idle)` per write; the idle
calls `gtk_widget_queue_draw`. **Tearing caveat** (`term_draw.rs:513-528`): the
worker mutates the grid and only *requests* a redraw, so a concurrent draw can
show a torn frame. `io::flush` app-mode is a SCAFFOLD no-op
(`app_io.rs:485`) explicitly meant to later drain the pending main-thread update.
Resize/`io::terminalSize` is a documented SCAFFOLD gap (rows/cols derived once at
activate; spec §Linux).

## 3. Design

- **Present.** Add a `term::sync` app arm (`emit_app_term_helper`, GTK
  `app_io.rs:9` table) that schedules one `gtk_widget_queue_draw` via the
  existing redraw-idle symbol; make `emit_app_io_flush_helper` (`app_io.rs:485`)
  drive the same present. Remove the per-write `g_idle_add(redraw)` from
  `_mfb_gtkapp_term_write` so redraw is present-driven.
- **Close tearing.** On present, marshal a snapshot to the main loop so the draw
  reads a consistent grid: either double-buffer the arrays (present copies the
  worker-written arrays into a draw-owned back copy under the main loop before
  `queue_draw`), or serialize the write→present→draw handoff through the idle so
  the draw never reads a half-written frame. Prefer the snapshot copy — it maps
  cleanly onto the shared back/front model.
- **Resize.** On the drawing area's `resize`/`size-allocate`, recompute
  `cols`/`rows` from the new allocation and cell metrics, update the active
  extent in `_mfb_gtkapp_state`, and force a full redraw. `term::terminalSize`
  then reflects the new extent (closes the SCAFFOLD gap).
- **`off` final present.** Schedule the final `queue_draw` before hiding the
  drawing area / swapping back to the transcript.

Risk: the GTK threading model — all GTK calls on the main loop; the snapshot copy
must happen on the main loop, not the worker. The COLOR_SET/bold/underline
bit-packing must be preserved through the snapshot.

## Phases

### Phase 1 — present-driven redraw + close tearing

- [ ] `term::sync` app arm → one `queue_draw` via the redraw-idle
      (`app_io.rs`/`term_draw.rs`); make `emit_app_io_flush_helper`
      (`app_io.rs:485`) drive it.
- [ ] Remove the per-write `g_idle_add(redraw)` from `_mfb_gtkapp_term_write`
      (`term_draw.rs:510`).
- [ ] Present marshals a main-loop snapshot copy of the grid arrays before
      `queue_draw`, closing the tearing caveat (`term_draw.rs:513-528`).

Acceptance: a `-app` program draws + `term::sync()` shows the frame with no
tearing under rapid updates; draw-without-`sync` shows nothing new. Commit: —

### Phase 2 — resize + `off` present

- [ ] `size-allocate` hook recomputes cols/rows + active extent, forces full
      redraw (`term_draw.rs`/`mod.rs`); `term::terminalSize` reflects it.
- [ ] `off` schedules the final `queue_draw` before the surface swap.
- [ ] Tests: manual `-app` run of `examples/life` (linux) — resize reflows, no
      tearing; smoke that `term::terminalSize` tracks a programmatic resize.

Acceptance: resizing the GTK window reflows `life`; `term::terminalSize` returns
the new extent; no torn frames. Commit: —

## Validation Plan

- Runtime proof: `mfb build -app` `examples/life` on Linux — identical rendering,
  one repaint per present, no tearing, resize reflows.
- Tests: `func_term_*` under `-app` (linux) green; resize smoke.
- Doc sync: update `04_term-backend.md` §Linux (present-driven redraw, tearing
  closed, resize implemented — remove the SCAFFOLD notes).

## Summary

GTK moves to present-driven redraw, ends the worker/draw tearing race via a
main-loop snapshot, and gains resize reflow. Content unchanged; risk is GTK
main-loop threading of the snapshot. Console and macOS untouched.
