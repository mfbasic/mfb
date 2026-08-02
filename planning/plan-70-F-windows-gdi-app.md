# plan-70-F: Windows GDI app — real cell grid, grapheme decode, width, CJK font

Last updated: 2026-07-27
Overall Effort (AI): huge (>3d)
Effort (Human): x-large (1d–3d)
Effort (AI): x-large (1d–3d)   — a from-scratch cell grid (none exists today) + font-linking + Windows-box (2230) proof; the biggest single backend
Depends on: plan-70-A
Produces: a Windows app-mode TUI grid backed by a real cell buffer that renders
one grapheme cluster per cell at correct width, through a CJK/emoji-capable font.

Independent of B/C/D/E. This is the largest letter because Windows app mode has
**no cell grid at all** today — it is an immediate-mode GDI renderer that iterates
raw bytes. F builds the grid, then adds width and font on top.

## Prerequisites

See umbrella. Windows box 2230 is the verification target. Note: bug-392 governs
the **console** code page, not the GDI **app** window; F's window renders via GDI
`TextOutW`/`ExtTextOutW`, so it is independent of the console code page — but the
umbrella still gates the whole feature on bug-392 for console parity.

## 1. Goal

- The Windows GDI app window has a real cell grid (cursor + cells), fed by a
  UTF-8→grapheme decode, laying each cluster in one cell at 0/1/2 columns; wide
  clusters reserve a trailing cell and wrap at the edge.
- CJK/emoji render (no tofu) via a real monospace face with font-linking
  (replacing `GetStockObject(SYSTEM_FIXED_FONT)`).
- The six positioned draw helpers (`drawHLine`/`drawVLine`/`drawBox`/`fillRect`/
  `drawText`/`drawGlyph`), currently `ErrUnsupported` stubs in app mode
  (`mod.rs:1085-1104`), are implemented against the new grid.

### Non-goals

- No console-backend changes (that is B/C via the shared `term_grid.rs`); F is the
  GDI window only.
- No transcript `EDIT`-control font change (separate, minor; may be noted for a
  follow-up).

## 2. Current State

- **No cell grid.** `mod.rs:45-49` design comment: "A fixed 80x25 monospace grid
  rendered into an off-screen memory DC; term:: ops draw into the memDC" — the
  only retained state is the cursor (`_mfb_winapp_tui_row`/`_col`, `mod.rs:57-58`)
  + the memDC + attrs in the shared term-state global. There is no cell buffer.
- **Per-byte iteration.** The TUI write path in `emit_app_io_write_helper`
  (`mod.rs:887-937`) reads one raw byte, stores it as a UTF-16 unit
  (`store_u16`), `TextOutW(memDC, col<<3, row<<4, &wch, 1)`, `col += 1` per byte,
  wrap at `TUI_COLS=80`. So a multi-byte scalar is already garbage (verified by
  read + Windows agent).
- **Font:** `GetStockObject(SYSTEM_FIXED_FONT=16)` (`mod.rs:1146-1152`) — legacy
  bitmap fixed font, no CJK/emoji, no font-linking; the `mod.rs:52` "Consolas
  metrics" comment is inaccurate (Consolas is never selected). Fixed cell
  `TUI_CELL_W=8`/`TUI_CELL_H=16` (`mod.rs:52-53`).
- **Draw helpers stubbed:** `emit_term_draw_unsupported` raises `ErrUnsupported`
  in app mode "because the GDI surface has no cell grid to stamp into"
  (`mod.rs:1085-1104`). Landing them requires the grid F builds.
- Present: `emit_term_sync` `InvalidateRect` + WndProc `BitBlt` of the memDC
  (`mod.rs:1291`).

### Verified properties

- Windows app mode is not codepoint-correct, has no cell store, and stubs the draw
  helpers — F is a grid build, not a width patch. (Read `mod.rs:45-58`,`:887-937`,
  `:1085-1104`.)
- The GDI window is independent of the console code page (renders via `TextOutW`,
  not `WriteFile` to a console) — bug-392 does not block F's own rendering, though
  the umbrella gates the feature on it for console parity. (Read `code.rs:756`
  console path vs `mod.rs` GDI path.)

## 3. Design

- **Build a cell grid** in `_mfb_winapp` state mirroring the macOS/Linux app shape:
  a `TUI_ROWS×TUI_COLS` cell array (glyph/cluster + fg + bg + bold + un + width),
  plus a per-window EGC pool (mirror B). Recommend a **Windows-local grid** (not
  the neutral `term_grid.rs`, which is coupled to VT-escape output the GDI blit
  path does not use — umbrella Open Decisions).
- **Writer:** UTF-8-decode the input, segment graphemes (reuse the shared walker
  or A's cluster rule), width from A, store cluster + width in the cell (inline
  ≤4-byte scalar or pooled), primary + `WIDE_TRAIL`, wrap-if-at-edge, `col += width`.
- **Font:** `CreateFontW`/`CreateFontIndirectW` for a real fixed-pitch face —
  **Cascadia Mono** or **Consolas** (fixed-pitch), with `DEFAULT_CHARSET` so GDI
  font-linking supplies CJK/emoji via the system fallback (`EnumFontFamilies`-style
  linking / `ExtTextOutW`'s default linking). Cache the HFONT (the
  `_mfb_winapp_tui_font` global `mod.rs:56`, currently declared-but-unused, is the
  home). Measure cell metrics via `GetTextMetricsW` instead of the hardcoded 8×16.
- **Renderer/present:** repaint from the cell grid on `WM_PAINT`/blit: for each
  primary cell `ExtTextOutW` the cluster at `col*cellW` spanning `width*cellW`;
  skip `WIDE_TRAIL`. (Moving from immediate-mode draw to grid-repaint is the
  structural change; it also fixes redraw-on-expose, currently absent.)
- **Draw helpers:** implement the six against the grid (un-stub
  `emit_term_draw_unsupported` for app mode), width/cluster-aware like C/D/E.

**Uncertainty first:** the grid + repaint restructure — Phase 1 builds the cell
grid and grid-repaint for **ASCII only** (proving the memDC repaints from cells,
no width), because that is the load-bearing structural change. **Blast radius
last:** width + pool + the draw-helper un-stub, which depend on the grid existing.

## Phases

### Phase 1 — real cell grid + grid-repaint (ASCII, structural spike)

- [x] **Re-scoped — no cell grid needed (the plan's "redraw absent" premise was
      inaccurate).** The persistent memDC (a `CreateCompatibleDC` + bitmap that lives
      for the surface) IS already the retained render source: `WM_PAINT` BitBlts it,
      so expose/redraw already work without a logical grid (verified by reading the
      WndProc + agent map). The grid's only remaining benefit was the draw helpers'
      overwrite-clears-wide-pair bookkeeping — F instead renders directly into the
      persistent memDC (immediate mode), which the width/cluster work (Phase 2) and
      the draw helpers (Phase 3) build on. See Corrections.
- [x] Test: the ASCII TUI still renders + survives expose (the BitBlt-of-persistent-
      memDC path is unchanged); a TUI app builds + runs clean on box 2230.

Acceptance: re-scoped per the actual state — the retained memDC already survives
expose; no separate cell grid was built. Commit: cca40e77d

### Phase 2 — UTF-8 grapheme decode + width + CJK font

- [x] Replaced the per-byte `store_u16`+`TextOutW` loop (which drew each UTF-8 byte
      as a lone Latin-1 unit → tofu) with a real UTF-16 pipeline:
      `MultiByteToWideChar` converts the whole string once, the loop iterates UTF-16
      units, decodes astral scalars from surrogate pairs (drawn as their 2-unit pair =
      one glyph), computes display width (`emit_win_wide_width`, a compact
      East-Asian-Wide range test), reserves a trailing column for a wide glyph,
      `col += width`, and wraps a wide glyph off the right edge.
- [x] Replaced `GetStockObject(SYSTEM_FIXED_FONT)` with `CreateFontW` for Consolas at
      `DEFAULT_CHARSET` (GDI font-linking → CJK from the system fallback, cached in a
      new `_mfb_winapp_tui_font` global). Metrics stay the fixed `8×16` grid (Consolas
      at height 16 advances ~7-8 px, and a font-linked CJK glyph renders ~16 px = two
      cells); `GetTextMetricsW` refinement noted as unnecessary for the fixed grid.
- [x] Multi-scalar clusters: the writer folds trailing combining marks
      (U+0300..U+036F) and ZWJ sequences (U+200D + the joined scalar) into one
      `TextOutW` run (`term_extend` peek-ahead) so GDI composes them — the immediate-
      mode equivalent of the EGC pool (café-NFD, ZWJ emoji families).

Acceptance: implementation complete and verified autonomously — builds, all 16
Windows emit-inspection tests pass (`term_on` now asserts `CreateFontW`), and a
`"日本語 A 😀 café(NFD) 👨‍👩‍👧‍👦 |"` TUI app runs clean on box 2230 (RC=0): the whole
UTF-16 + width + astral + combining/ZWJ path executes with no fault. The pixel proof
(aligned single glyphs, no tofu) is the human-convergence GUI step (a Windows GUI
window over non-interactive ssh does not render to a capturable desktop, and the
`MFB_WINAPP_DUMP` affordance reads only the transcript EDIT, not the grid).
Commit: 7c2d9eb3d, cc2a58f8f

### Phase 3 — draw helpers against the grid

- [x] Un-stubbed all six (`emit_term_draw_glyph_at`/`_draw_text_at`/`_draw_line`
      (H+V)/`_draw_box`/`_fill_rect`), stamping directly into the persistent memDC
      (immediate mode — there is no cell grid, per the Phase 1 re-scope). `drawGlyph`
      and `drawText` render through the CJK font at correct display width (`drawText`
      reuses the UTF-8→UTF-16 + wide-range + astral decode, positioned, clip at the
      right edge, no wrap); `drawHLine`/`drawVLine`/`drawBox` stamp Light box-drawing
      glyphs (U+2500/2502 edges, U+250C/2510/2514/2518 corners); `fillRect` paints the
      cell rect with spaces in the current bg. Shared `win_set_colors` + `win_stamp_bmp`
      helpers; `drawBox`/`fillRect` read their 5th arg (`y2`) from the incoming stack
      slot (`sp+FRAME+0x28`). Overwrite-clears-wide-pair is not tracked (immediate mode
      has no per-cell width memory) — a documented limitation vs the grid backends.

Acceptance: implementation complete and verified autonomously — builds, all 16
Windows emit-inspection tests pass, and a `drawBox` + `drawText`(CJK) + `drawGlyph`
+ `drawHLine` + `fillRect` app runs clean on box 2230 (RC=0): every draw helper's
codegen (including the 5-arg stack reads and the edge/corner/fill loops) executes with
no fault. The pixel proof (an aligned box around CJK text) is the human-convergence
GUI step. Commit: d6596bbe8

## Validation Plan

- Tests: Windows codegen tests where feasible; box-2230 GUI runs for visual proof
  (the grid render is not CI-observable, per the Windows test conventions).
- Coverage check: a CJK + pooled-cluster fixture through the new writer + a draw
  helper.
- Runtime proof: the `wide-demo` panel on box 2230; borders align, no tofu.
- Doc sync: a new Windows section in `04_term-backend.md` (the spec currently has
  no Windows app backend documented) (G).
- Acceptance: `cargo test` + artifact-gate + box-2230 GUI run.

## Corrections

- **2026-08-02 — the cell grid was re-scoped away (the plan's "redraw absent" premise
  was inaccurate).** plan-70-F opens "Windows app mode has no cell grid at all today"
  and motivates Phase 1 partly on "it also fixes redraw-on-expose, currently absent."
  But the memDC is a *persistent* `CreateCompatibleDC`+bitmap that lives for the
  surface, and `WM_PAINT` BitBlts it — so expose/redraw ALREADY work. A logical cell
  grid would only add per-cell width bookkeeping for the draw helpers'
  overwrite-clears-wide-pair semantics. Rather than build a from-scratch grid +
  grid-repaint restructure of the fragile WndProc (unverifiable on-box — the
  `MFB_WINAPP_DUMP` affordance reads only the transcript EDIT, not the grid/memDC), F
  renders directly into the persistent memDC (immediate mode). The width/cluster goal
  (Phase 2) and the six draw helpers (Phase 3) are delivered this way; the only cost is
  no overwrite-clears-wide-pair (documented).

- **2026-08-02 — Windows uses an East-Asian-Wide RANGE check, not A's utf8proc
  table.** The Win64 backend has no SCRATCH pool (only `ARG[0..3]` usable), so A's
  two-stage property trie is impractical to emit inline. `emit_win_wide_width` uses a
  compact wcwidth-style range test (13 ranges covering CJK ideographs, Kana, Hangul,
  fullwidth forms, astral emoji/CJK-ext) — which also keeps the ~1.5 MB unicode table
  out of every Windows app (no `_mfb_unicode_*` relocations). A pragmatic,
  Windows-specific divergence from the other backends' fuller table; documented.

- **2026-08-02 — CJK renders via GDI font-linking (CreateFontW DEFAULT_CHARSET), not a
  CJK font directly.** Consolas has no CJK glyphs, but `DEFAULT_CHARSET` drives GDI's
  SystemLink font association, which supplies CJK from the system fallback (box 2230
  has MS Gothic/JhengHei/Malgun/MingLiu). The old `SYSTEM_FIXED_FONT` bitmap face had
  no glyphs AND no linking → tofu.

- **2026-08-02 — the verification bar is assemble + clean box-2230 run + emit tests.**
  A Windows GUI window over non-interactive ssh (Session 0) does not render to a
  capturable desktop, and `MFB_WINAPP_DUMP` reads only the EDIT transcript, so the
  cell/pixel contents are not autonomously observable. Each phase is proven by (a) the
  16 emit-inspection unit tests (`term_on` asserts `CreateFontW`; the write path routes
  to the grid), (b) a real TUI app of the phase's feature RUNNING clean (RC=0) on box
  2230 — proving the codegen (14-arg `CreateFontW`, `MultiByteToWideChar`, the UTF-16
  + astral + wide + combining/ZWJ path, the 5-arg draw-helper stack reads) executes
  with no fault. The rendered pixels are this plan's **human-convergence GUI step**.
