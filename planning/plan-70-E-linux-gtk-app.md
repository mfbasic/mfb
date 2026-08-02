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

- [x] Added `libpango-1.0`/`libpangocairo-1.0` imports (`pango_cairo_create_layout`,
      `pango_cairo_show_layout`, `pango_layout_set_text`,
      `pango_layout_set_font_description`, `pango_layout_get_pixel_extents`,
      `pango_font_description_from_string`/`_set_weight`/`_free`, `g_object_unref`)
      and a `"monospace 16"` font-description data string. `term_draw`'s render loop
      now builds one `PangoLayout` + `PangoFontDescription` per frame (reused per
      cell: `set_text` + `move_to(col*cellW, row*cellH)` + `pango_cairo_show_layout`);
      bold re-weights the description (700/400) and reapplies it. `term_init` measures
      the cell from the SAME Pango font (`get_pixel_extents` of `"M"` → logical
      `{width,height}`). Removed the dead Cairo toy font path
      (`emit_term_select_font`, `cairo_select_font_face`/`_set_font_size`/
      `_show_text`/`_font_extents`/`_text_extents`, `TERM_FONT_SIZE`, `STR_MONOSPACE`).
- [x] Test: the Pango codegen assembles for **both** `linux-x86_64` and
      `linux-aarch64` (glibc + musl — 4 AppImages) and the app RUNS on a real GTK
      box (2226, aarch64, under Xvfb) with a **clean log** — every `libpango*` symbol
      binds and executes (a missing/misspelled one aborts at load), the window is
      created, and the GTK main loop + draw callback run without a crash or
      GTK-CRITICAL.

Acceptance: implementation complete and verified by the autonomous means available
here (cross-arch/cross-libc assemble; clean launch on a real GTK box binding + running
the Pango draw). The pixel-level "CJK/emoji render as real glyphs, no tofu" is the
plan's **human-convergence GUI step** — a bare headless Xvfb cannot render the GTK4
drawing area to its framebuffer (GTK4's GL renderer needs a compositor/GL session, and
even `GSK_RENDERER=cairo` stays black; the black paint + a coloured background never
reach the root either, so it is a GTK4/Xvfb display limit, NOT the Pango codegen). This
is the SAME pre-existing limitation `tests/rt_gtk_term_utf8_grid.rs` documents ("needs
a manual `-app` run on a GTK desktop"). Commit: a643ea509

### Phase 2 — width layout + single-scalar wide

- [x] Width rides in the fg word's free bits 27-28 (`WIDTH_SHIFT`) — NO separate
      array, so the snapshot memcpy + resize/scroll array shifts carry it for free.
      The writer decodes the scalar from the packed UTF-8 bytes and looks up A's
      charwidth (`emit_gtk_charwidth`, two-stage trie, width 0→1), gated on
      `uses_term` (threaded into `emit_term_write_helper`) so a non-term app never
      embeds the table. A width-2 glyph writes `fg |= width<<27`, reserves the next
      cell as a `GTK_WIDE_TRAIL` (0xFFFFFFFF) sentinel in the CHAR array, `col +=
      width`, and wraps off the right edge (spilling glyph+width across the wide-edge
      scroll).
- [x] The renderer skips a `GTK_WIDE_TRAIL` cell (its Pango primary already spans the
      column; the trail's own bg still fills). The width bits sit above the fg colour
      (masked to the low 24 bits) and the COLOR_SET/bold/underline flags, so they are
      transparent to the existing draw.

Acceptance: implementation complete and verified autonomously — `emit_gtk_charwidth`'s
scalar-decode + trie assembles for **both** `linux-x86_64` and `linux-aarch64`
(glibc+musl) and a `term::` app writing `"日本語ABC|"` runs clean on a real GTK box
(2226, Xvfb): the table embeds, the trie + WIDE_TRAIL path execute, no crash. GTK
integration tests pass. The pixel alignment (`|` at column 6) is the human-convergence
GUI step (same GTK4/Xvfb display limit as Phase 1). Commit: dae372982

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

- **2026-08-02 (Phase 1) — the cell metric must migrate to Pango too, or glyphs
  overflow.** The old init measured the cell from the Cairo toy font at
  `set_font_size(16)` (16 user units ≈ px), but Pango `"monospace 16"` is 16 POINTS
  (≈ 21 px at 96 dpi) — so drawing through Pango while sizing from Cairo would make
  every glyph overflow its cell. Fixed by measuring the cell from the same Pango font
  (`pango_layout_get_pixel_extents` of `"M"`, logical `{width,height}`), so geometry
  and rendering share one font.

- **2026-08-02 (Phase 1) — Pango draws from the top-left, not a baseline.** The Cairo
  `show_text` path did `move_to(col*cellW, (row+1)*cellH - 4)` (a baseline).
  `pango_cairo_show_layout` places the layout's TOP-LEFT at the current point, so the
  `move_to` became `(col*cellW, row*cellH)` (cell top). The fg colour is still set via
  `cairo_set_source_rgb` before the draw — `show_layout` inherits the cairo source.

- **2026-08-02 (Phase 1) — the x86-64 backend rejects a `-1` immediate.**
  `pango_layout_set_text(layout, buf, length)` wants `length = -1` for a
  NUL-terminated string, but `move_immediate(x2, "-1")` fails the x86-64 selector
  ("invalid immediate '-1'"). Emitted `move 0` + `bitwise_not` instead (0xFFFF… whose
  low 32 bits are `-1`).

- **2026-08-02 (Phase 1) — the GTK pixel proof is unreachable headless (pre-existing).**
  A bare Xvfb never composites the GTK4 drawing area to its root framebuffer: the
  window is created and the draw callback runs (clean log), but the captured root is
  100% black — even the unchanged black `cairo_paint` and a coloured background do not
  appear, and `GSK_RENDERER=cairo` does not help. This matches the limitation
  `tests/rt_gtk_term_utf8_grid.rs` already documents (a `term::` GTK app "needs a
  manual `-app` run on a GTK desktop"; the headless VM "has no reachable X server").
  So Phase 1's autonomous proof is assemble + clean-launch; the glyph pixels are the
  human step. (The window DID appear at 1×1 off-screen under no-WM Xvfb; `xdotool`
  resized it to 900×300 viewable, confirming the app + main loop are live.)
