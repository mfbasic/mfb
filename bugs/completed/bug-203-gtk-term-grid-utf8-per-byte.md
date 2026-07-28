# bug-203: GTK term grid stores/renders one UTF-8 byte per cell → non-ASCII glyphs become tofu

Last updated: 2026-07-16
Effort: medium (2h–4h)
Severity: MEDIUM
Class: correctness (platform: linux-gtk app mode)

Status: FIXED (2026-07-16). Full support implemented — the ASCII-only
documentation fallback the Fix Design allowed was not needed.

A char cell is now a `u32` holding ONE code point's UTF-8 bytes packed
little-endian (lead byte low, zero-padded), matching fg/bg's width:

- `term_write` decodes a code point per iteration (`emit_utf8_decode_at`): the
  lead byte gives the length (<0xC0 → 1, <0xE0 → 2, <0xF0 → 3, else 4), the
  length is clamped to the bytes that remain so a truncated tail cannot read past
  the text, and the bytes are packed with fixed shifts (unrolled — the neutral
  abi has no variable shift). One cell per glyph, cursor advances ONE column, and
  `i` advances by the code point's length. A byte that is not a lead byte (a
  stray continuation) decodes as length 1, so malformed input still advances and
  renders per-byte rather than hanging or over-reading.
- `term_draw` loads the whole cell and `str_u32`s it into the glyph buffer with a
  NUL at +4, so `cairo_show_text` gets one complete NUL-terminated glyph
  (buffer 2B → 5B).
- The length lives in x19, added to `term_write`'s save/restore (frame 96 → 112):
  `tw_clamp` can call the scroll helper between the decode and the `i += len`
  advance, so a caller-saved scratch would not survive it.
- A blank cell is 0, not `' '`: the blanking `memset`s write whole bytes, and
  `' '` over u32 cells would pack FOUR spaces per cell. The draw skips 0 and 32
  alike (both render nothing), and compares the whole cell, so a multi-byte glyph
  whose lead byte is 0x20 can never be mistaken for a space.
- Strides updated in lockstep: scroll `memmove` shift 0 → 2, the scroll blank and
  `term::clear`/init `memset`s, and the snapshot `memcpy`. `ST_TERM_FG` /
  `ST_TERM_SNAP_FG` offsets follow.

Verified: emitted `.ncode` reviewed instruction-by-instruction (length ladder
192/224/240, the remaining-bytes clamp, the pack shifts, `lsl 2` + `str_u32` cell
store, `i += len` advance, `ldr_u32` + `str_u32` in the draw). The state object
is 185536 bytes — exactly the predicted 139456 + 2×160×48×3 for the live and
snapshot char grids, which pins the offsets against the memset/memcpy sizes.
Full acceptance 949/949. Three regression tests added, all three fail with the
fix reverted.

**Honest verification gap:** the glyph actually PAINTING one column is not
covered by an automated test, and was not observed. Rendering happens in a Cairo
draw callback needing a real display; the GTK VM available here has no reachable
X server (Xwrapper `allowed_users=console`, and installing Xvfb needs root), and
under the headless `gtk4-broadwayd` backend a `term::` app segfaults during the
program body — PRE-EXISTING, reproduced identically on a pre-change baseline
binary, so not caused by this fix. Confirming the visual result needs a manual
`mfb build -app` run on a GTK desktop.

Regression Test: tests/gtk_term_utf8_grid.rs (code-plan level; see the gap above)

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
