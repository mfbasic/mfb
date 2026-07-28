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

- [ ] Add a `TUI_ROWS×TUI_COLS` cell array + cursor to `_mfb_winapp` state; the
      write path stores cells instead of drawing immediately; `WM_PAINT`/blit
      repaints from cells via `ExtTextOutW`.
- [ ] Test: a Windows-box 2230 run — ASCII TUI (e.g. the `browser` header) still
      renders and now survives an expose/redraw.

Acceptance: ASCII TUI renders from the cell grid and repaints on resize/expose on
box 2230. Commit: —

### Phase 2 — UTF-8 grapheme decode + width + CJK font

- [ ] Replace per-byte iteration with UTF-8 decode + grapheme segmentation; store
      cluster + width (A); primary + `WIDE_TRAIL`; `col += width`; wrap-if-at-edge.
- [ ] Replace `SYSTEM_FIXED_FONT` with `CreateFontW` for a fixed-pitch CJK-capable
      face + font-linking; metrics from `GetTextMetricsW`.
- [ ] EGC pool for multi-scalar clusters.

Acceptance: `"👍 日本語 A café(NFD)"` renders as aligned single glyphs (no tofu, no
byte-split) on box 2230. Commit: —

### Phase 3 — draw helpers against the grid

- [ ] Un-stub the six draw helpers for app mode; implement width/cluster-aware
      stamping into the grid (overwrite clears the wide pair).

Acceptance: `term::drawBox` + `drawText` with CJK content render/align on box
2230; `cargo test` + artifact-gate green (goldens in G). Commit: —

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

<Filled in during execution.>
