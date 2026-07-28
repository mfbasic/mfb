# plan-70-E: Linux GTK app — width layout + Cairo-toy→Pango font migration

Last updated: 2026-07-27
Overall Effort (AI): huge (>3d)
Effort (Human): large (3h–1d)
Effort (AI): large (3h–1d)   — new Pango wiring (novel API surface) + Linux-box visual proof; converges with Human on the verify loop
Depends on: plan-70-A
Produces: a Linux app-mode TUI grid that renders one grapheme cluster per cell at
correct width **and** falls back across fonts for CJK/emoji (via Pango instead of
the Cairo toy API).

Independent of B/C/D (separate storage: parallel static arrays in
`_mfb_gtkapp_state`; Cairo/Pango rendering). Mirrors B's cell-model contract.

## Prerequisites

See umbrella. A Linux box (with GTK4) is the verification target.

## 1. Goal

- The GTK TUI grid lays each grapheme cluster in one cell at 0/1/2 columns; wide
  clusters reserve a trailing cell and wrap at the edge.
- CJK/emoji render (no tofu) because the grid draws through **Pango** (which
  cascades fonts), replacing the Cairo toy `cairo_select_font_face("monospace")`.
- Combining/ZWJ clusters render as one glyph.

### Non-goals

- No transcript-view changes (it already uses Pango via GtkTextView).
- No change to the `_mfb_gtkapp_state` cursor/attr slots beyond adding width/pool
  fields.

## 2. Current State

- Grid = three parallel `u32[160*48]` arrays `ST_TERM_CHARS`/`ST_TERM_FG`/
  `ST_TERM_BG` (one codepoint per cell; `04_term-backend.md:424-448`), plus
  snapshot copies; flags packed into fg/bg high bits (COLOR_SET bit 24, bold 25,
  underline 26; `mod.rs:COLOR_SET`) — **bits 27+ free** for a width/tag.
- Writer `_mfb_gtkapp_term_write`: `emit_utf8_decode_at` (`term_draw.rs:21`)
  decodes one UTF-8 scalar → u32 codepoint, one cell, `col += 1`.
- Renderer `_mfb_gtkapp_term_draw` (`term_draw.rs:75+`): per cell,
  `cairo_show_text` of the codepoint's UTF-8 (5-byte buf) using
  `emit_term_select_font` → **`cairo_select_font_face("monospace")`** at
  `TERM_FONT_SIZE=16` (`term_draw.rs:275-289`, `mod.rs:155,228`), cell metrics
  from `cairo_text_extents("M")` / `cairo_font_extents` (`:417-437`). The Cairo
  toy API has **no font cascade** → CJK/emoji tofu (verified by read + font audit).
- Resize `_mfb_gtkapp_term_resize` reflows cols/rows; `_mfb_gtkapp_term_scroll`
  shifts the three arrays.

### Verified properties

- The grid is one-codepoint-per-cell, `col += 1` (read `term_draw.rs` write
  helper). Width + cluster support needs new storage + advance.
- The TUI grid uses the Cairo **toy** API, not Pango — the transcript uses Pango
  (`bootstrap.rs:186-197`). So the fallback machinery exists in-process but is not
  wired to the grid. Migrating the grid draw to
  `pango_cairo_show_layout` (or `pango_layout` + `pango_cairo_create_layout`) is
  the font half of E. (Verified by read + audit.)

## 3. Design

- **Font:** replace `emit_term_select_font`'s Cairo-toy calls with a Pango layout:
  create a `PangoLayout` (via `pango_cairo_create_layout(cr)`), set a
  `PangoFontDescription` for "monospace 16" once (cache it), and per cell
  `pango_layout_set_text` + `pango_cairo_show_layout`. Pango performs the font
  cascade, so CJK/emoji render. Cell metrics from Pango
  (`pango_layout_get_pixel_extents` of "M", or the font metrics). This is new
  external-symbol wiring (`pango_cairo_*`, `pango_font_description_*`,
  `pango_layout_*` imports in `mod.rs`).
- **Cell width + cluster:** use a free bit range in the fg/bg word (bits 27+) or
  add a parallel `u32[160*48] ST_TERM_WIDTH` array for the width (0/1/2) and a
  `WIDE_TRAIL` tag; a multi-scalar cluster goes to a per-state EGC pool (mirror B)
  with the char word holding a tagged offset, or (simpler for parallel-array
  storage) a parallel `ST_TERM_EGC` offset array. Recommend a parallel width array
  + EGC-offset array to keep the fixed-stride static-storage design.
- **Writer:** segment graphemes (reuse the shared walker or decode + A's cluster
  rule), width from A, store cluster + width, primary + `WIDE_TRAIL`,
  wrap-if-at-edge, `col += width`. Snapshot copy carries the new arrays.
- **Renderer:** skip `WIDE_TRAIL`; draw the cluster via Pango spanning `width`
  cells.

**Uncertainty first:** Pango wiring — Phase 1 spikes the font migration alone
(ASCII still one cell) to prove `pango_cairo_show_layout` renders a CJK glyph in
the grid at all, before width layout. **Blast radius last:** the snapshot/resize/
scroll paths carrying the new parallel arrays.

## Phases

### Phase 1 — Pango migration for the grid (font fallback, no width yet)

- [ ] Add `pango_cairo_*`/`pango_layout_*`/`pango_font_description_*` imports;
      replace `emit_term_select_font` + `cairo_show_text` with a cached
      `PangoLayout` draw; derive cell metrics from Pango.
- [ ] Test: a Linux-box GUI run drawing `"日本語"` — glyphs render (no tofu),
      even if still one-cell-per-codepoint (misaligned is acceptable this phase).

Acceptance: CJK/emoji render as real glyphs in the GTK TUI grid on a Linux box
(fallback works). Commit: —

### Phase 2 — width layout + single-scalar wide

- [ ] Add the width array + `WIDE_TRAIL` tag; writer computes width (A), writes
      primary + trailing, `col += width`, wrap-if-at-edge; snapshot carries it.
- [ ] Renderer skips `WIDE_TRAIL`, draws the cluster spanning `width` cells.

Acceptance: `"日本語 |"` aligns the `|` (6 columns) on a Linux box. Commit: —

### Phase 3 — EGC pool for multi-scalar clusters + draw helpers

- [ ] EGC-offset array + per-state pool; writer segments full clusters; renderer
      draws pooled clusters.
- [ ] The GTK draw helpers (`drawText`/`drawGlyph`/`drawBox`/`fillRect`/line)
      width/cluster-aware; overwrite clears the wide pair.
- [ ] Resize + scroll carry width/EGC arrays.

Acceptance: NFD `"café"`, ZWJ family, and a `drawBox` around CJK render/align on a
Linux box; resize reflows without corrupting wide cells. Commit: —

## Validation Plan

- Tests: GTK app grid tests where feasible + a Linux-box GUI smoke run.
- Coverage check: a CJK + pooled-cluster fixture through writer and a draw helper.
- Runtime proof: the `wide-demo` panel on a Linux box; borders align, no tofu.
- Doc sync: `04_term-backend.md` Linux section (Cairo→Pango) (G).
- Acceptance: `cargo test` + artifact-gate + Linux GUI run.

## Corrections

<Filled in during execution.>
