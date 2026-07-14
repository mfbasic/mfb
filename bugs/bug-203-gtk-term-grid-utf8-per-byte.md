# bug-203: GTK term grid stores/renders one UTF-8 byte per cell → non-ASCII glyphs become tofu

Last updated: 2026-07-14
Effort: medium (2h–4h)
Severity: MEDIUM
Class: correctness (platform: linux-gtk app mode)

Status: Open
Regression Test: tests/ (a -app term::write of a multi-byte glyph renders one column)

The GTK term grid stores and renders one UTF-8 byte per cell, so any non-ASCII
glyph (e.g. box-drawing U+25xx, ubiquitous in TUIs) is written as separate cells
of lone continuation bytes and drawn as invalid-UTF-8 tofu, and the cursor
advances by the byte count instead of one column.

## Failing Reproduction

A `mfb build -app` program on Linux/GTK runs `term::write("─")` (U+2500, 3 UTF-8
bytes). Observed: `term_write` stores 0xE2/0x94/0x80 into three consecutive cells
and advances `col` by 3; `term_draw` loads each cell as a 1-byte NUL-terminated
string and calls `cairo_show_text` on an invalid fragment → three tofu boxes.
Expected: one cell holds the glyph, cursor advances one column, one glyph drawn.

## Root Cause

`src/target/linux_gtk/term_draw.rs:674-690` (`term_write` per-byte cell store) and
`:100-104`/`:130-131` (`term_draw` per-cell single-byte `show_text`). The grid
cell is one byte wide with no UTF-8 decoding. (Distinct from bug-117's tearing
race.)

## Non-goals

- Do not change ASCII rendering behavior.
- Do not touch the macOS term view (separate implementation).

## Blast Radius

- linux_gtk `term_draw.rs` grid store + draw only.

## Fix Design

Decode UTF-8 code points in `term_write` (store the full multi-byte glyph per
cell, advance one column) and render the whole cell string in `term_draw`. If
full support is deferred, at minimum document the ASCII-only limitation at the
grid API.
