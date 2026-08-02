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

- [x] `emit_draw_text` cluster-walks (peek-ahead over the free-function UAX #29
      break machinery, mirroring `emit_grid_write`), stores pooled multi-scalar
      clusters into the on-grid cell's pool slot, computes width, stamps primary +
      `WIDE_TRAIL`, and CLIPS a wide glyph off the right edge (drops it, stops the
      run — never splits). `emit_draw_glyph` computes the single glyph's width and
      stamps a `WIDE_TRAIL` neighbor when wide. Both gained a `relocations` param
      (threaded from the dispatch); `emit_stamp_cell` gained a `width` param + a
      `tag` (for the paired-clear); `emit_stamp_run` stamps width 1.
- [x] Tests: `tests/rt-behavior/term/func_term_draw_wide_valid` (escape-stream
      golden) draws `日本語|` (col 6), café NFD `café|` (col 4, pooled), the ZWJ
      family `👨‍👩‍👧‍👦|` (col 2, pooled), a positioned wide `drawGlyph`, and a
      paired-clear case.

Acceptance: verified on the macOS host — `drawText` of CJK/emoji/clusters aligns
the following column; `drawGlyph` of `😀` now correctly occupies 2 columns (the
`func_term_drawGlyph_valid` golden updated — the old one encoded the pre-plan-70
1-column bug); 0 NUL bytes. `cargo test` 3748 passed. Commit: —

### Phase 2 — line/box/fill respect wide neighbors

- [x] `emit_clear_wide_pair` (called from both `emit_stamp_cell` and the
      `emit_stamp_run` line/box/fill loop) blanks the paired half of any wide glyph
      a stamp overwrites: a `WIDE_TRAIL` clears the primary to its left; a wide
      primary clears the trail to its right (blank glyph + width 1, attributes
      kept). Uses `ctx.idx`/`ctx.lo` — dead scratch at every call site.
- [x] Tests: covered by `func_term_draw_wide_valid` (draw `日本`, overwrite the
      trailing half of `日` → its orphaned primary is blanked, leaving `_X本`).

Acceptance: verified — overwriting half a wide glyph leaves no stray column
(` X本`); all 35 term acceptance fixtures green; `cargo test` 3748 passed. Commit: —

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

- **2026-08-02 — the draw helpers needed the same free-function + `relocations`
  plumbing as B.** `emit_draw_text`/`emit_draw_glyph` are free functions (raw
  `abi::` into `instructions`) with no `relocations`; the property-table lookup
  needs it, so both grew a `relocations` param threaded from the term dispatch.
  They reuse B's free primitives (`emit_utf8_codepoint_by_len`,
  `emit_unicode_property_ptr_free`, `emit_read_boundclass_icb_charwidth_free`,
  `emit_grapheme_break_branch_free`, `emit_grapheme_state_update_free`).
- **2026-08-02 — `emit_stamp_cell` MUST store the width byte even for narrow
  glyphs.** Before C it wrote glyph/fg/bg/bold/un but not `C_WIDTH`, so a narrow
  glyph stamped over a cell that had been a wide primary (stored width 2) left the
  stale width 2 → the presenter advanced 2 for a 1-column glyph. C adds a `width`
  param stored into every stamp (and `emit_stamp_run` stamps width 1).
- **2026-08-02 — the `emit_stamp_cell` paired-clear tag must be unique per call,
  not derived from `skip`.** `drawText` calls `emit_stamp_cell` twice (primary +
  trail) with the same `skip` (`advance`), so deriving the paired-clear label tag
  from `skip` produced a duplicate label (`AArch64: duplicate label …`). Fixed by
  adding an explicit `tag` param, unique per call site.
- **2026-08-02 — `drawGlyph`/`drawText` draw codegen had NO byte-identity
  coverage.** The `byte-identity/term` fixture used only `term::on/moveTo/set*/
  clear/sync`, never a draw op, so C's (complex) cluster-walk/pool/paired-clear
  codegen was cross-target-unverified. Added a wide CJK run, an NFD cluster, a ZWJ
  family, a positioned `drawGlyph`, and a paired-clear overwrite to that fixture,
  regenerating its `.ast`/`.ir` + 5-target `.ncodesum`. (Runtime is covered by the
  rt-behavior term fixtures.)
- **2026-08-02 — the `func_term_drawGlyph_valid` golden updated (proven stale).**
  Its `😀` is East Asian Wide (charwidth 2); the old golden rendered it as ONE
  column — the exact pre-plan-70 bug. C makes it 2 columns (primary + `WIDE_TRAIL`),
  so the row-fill has one fewer trailing space. Old golden proven wrong (encoded
  the bug); regenerated.
