# plan-70-C: Console draw helpers — cluster/width-aware stamping

Last updated: 2026-07-27
Overall Effort (AI): huge (>3d)
Effort (Human): medium (1h–2h)
Effort (AI): small (<1h)   — mirror B's cell-model contract into the stamp path; precedent-heavy
Depends on: plan-70-A, plan-70-B
Produces:
- Width/cluster-aware `emit_draw_text` and `emit_draw_glyph` (and the shared
  `emit_stamp_cell`) so positioned stamping obeys the same cell model as the
  writer/present.

The box/line/fill helpers stamp only single-width box-drawing glyphs, so most of
C is `drawText`/`drawGlyph`; the line/box/fill drawers need only to respect
`WIDE_TRAIL` when they overwrite cells (so a box edge drawn over a wide glyph
clears the orphaned trailing cell).

## Prerequisites

See umbrella. C additionally requires B's cell-model consts (`width` offset,
`WIDE_TRAIL`, pool tagging) to exist — that is the Depends-on edge, not scope C
absorbs.

## 1. Goal

- `term::drawText(x, y, "日本語")` stamps three wide clusters occupying six
  columns from `x`, clipping at the right edge without splitting a wide glyph, and
  writing `WIDE_TRAIL` for each wide cluster's second column.
- `term::drawGlyph(x, y, cp)` stamps one cluster with its correct width.
- A line/box/fill stamp that lands on a wide glyph's primary or trailing cell
  clears the **other** half (no orphaned `WIDE_TRAIL` rendering as a stray column).

### Non-goals

- No app-backend changes (those are D/E/F, which have their own draw helpers).
- No new `term::` call surface; behavior of existing calls only.

## 2. Current State

- `emit_draw_text` (`src/target/shared/code/term.rs:1486`): iterates scalars,
  control chars skipped but still `col += 1` (`:1620-1622`), printable stamped via
  `emit_stamp_cell` (`:1613`) then `col += 1` (`:1616`); clips at the row edge.
- `emit_draw_glyph` (`:1406`): stamps one scalar at a cell.
- `emit_stamp_cell` (`:867`): writes glyph + current attributes into one cell.
- All three assume one scalar = one cell = one column (verified by read,
  `:1613-1622`).
- These stamp into the same back-buffer cells B redefined; they must write the
  `width` byte and `WIDE_TRAIL` trailing cell exactly as B's writer does, or the
  presenter (B) will mis-account a stamped wide glyph.

### Verified properties

- The draw helpers stamp into B's cell layout (same `back` buffer, same
  `CELL_SIZE`), so they inherit B's width/sentinel contract; they do **not** go
  through `emit_grid_write`. (Read `emit_stamp_cell` — it addresses cells
  directly.) Therefore C is required for stamped wide glyphs to align, and cannot
  be skipped by relying on B.

## 3. Design

- Factor B's "write a cluster: compute width, store inline/pooled, write primary +
  `WIDE_TRAIL`, advance by width, wrap-if-at-edge" into a shared cell-stamp
  primitive both the writer (B) and the draw helpers (C) call, so there is one
  definition of the cluster→cells mapping. If a shared helper is impractical
  across the writer/stamp register conventions, C re-implements the identical
  contract and a test asserts parity.
- `emit_draw_text`: segment into graphemes, compute width, clip a wide cluster
  that would exceed the right edge (drop it rather than split), stamp primary +
  trailing, advance by width. Control chars: keep skipping, advance by their width
  (0 for a control per A's table — verify; today they advance 1).
- `emit_draw_glyph`: single cluster, width-aware, no advance (positioned).
- `emit_stamp_cell` / line-box-fill: when stamping a cell, if the target or its
  neighbor is a `WIDE_TRAIL`/primary of an existing wide glyph, clear the paired
  cell to blank so no half-glyph remains.

## Phases

### Phase 1 — drawText / drawGlyph width + cluster aware

- [ ] Update `emit_draw_text`, `emit_draw_glyph` to segment graphemes, compute
      width (A), stamp primary + `WIDE_TRAIL`, advance/clip by width.
- [ ] Tests: a fixture drawing `"日本語"` at a known cell and a following ASCII
      marker; assert alignment via the grid/escape capture harness.

Acceptance: `drawText` of CJK/emoji aligns the following column; a wide glyph at
the right edge is dropped, not split. Commit: —

### Phase 2 — line/box/fill respect wide neighbors

- [ ] `emit_stamp_cell` (and the line/box/fill run stampers) clear the paired half
      when overwriting one cell of a wide glyph.
- [ ] Tests: draw a box edge across a row containing a wide glyph; assert no
      orphaned trailing column renders.

Acceptance: overwriting half a wide glyph leaves no stray column; `cargo test` +
artifact-gate green (goldens in G). Commit: —

## Validation Plan

- Tests: draw-helper function/fixture tests for wide `drawText`/`drawGlyph` and
  the overwrite-clears-pair case.
- Coverage check: a fixture that stamps a pooled cluster via `drawText` (not only
  the writer path).
- Runtime proof: the `wide-demo` panel (G) uses `drawBox` + `drawText` with CJK
  content; borders align.
- Doc sync: `mfb man term drawText`/`drawGlyph` note wide-glyph column behavior (G).
- Acceptance: `cargo test` + artifact-gate.

## Corrections

<Filled in during execution.>
