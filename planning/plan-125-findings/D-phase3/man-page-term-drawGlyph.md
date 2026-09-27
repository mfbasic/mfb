### 1. `codepoint` is not restricted to Unicode scalars
UNIT:      man-page:term/drawGlyph
CLAIM:     "The Unicode scalar to stamp. Control code points (< 0x20) are skipped."
VERDICT:   misleading
EVIDENCE:  `src/codegen/term/core/term.rs:emit_draw_glyph` only skips values `< 32`, then calls `emit_encode_utf8`; it performs no Unicode-scalar validation. Probe `drawGlyph(0, 0, 1114112)` built and exited `0`; its captured frame contains `f4 90 80 80`, the UTF-8 encoding of out-of-range U+110000.
SUGGESTED: `Integer to stamp. Values below U+0020, including negative values, are skipped; other values are encoded as supplied, so pass a valid printable Unicode scalar.`

### 2. The description promises scalar-only behavior the binary does not provide
UNIT:      man-page:term/drawGlyph
CLAIM:     "`term::drawGlyph` stamps a single Unicode scalar — given by its `codepoint` — into the cell at `row`/`column`, using the colours and attributes currently in effect."
VERDICT:   wrong
EVIDENCE:  The out-of-range-codepoint probe above emitted bytes for U+110000 without raising. `src/codegen/term/core/term.rs:emit_encode_utf8` explicitly assumes a valid scalar rather than validating one.
SUGGESTED: `` `term::drawGlyph` stamps the integer in `codepoint` at `row`/`column`, using the colours and attributes currently in effect; pass a valid Unicode scalar. ``

### 3. Wide glyphs affect more than the named cell
UNIT:      man-page:term/drawGlyph
CLAIM:     "`term::drawGlyph` stamps a single Unicode scalar — given by its `codepoint` — into the cell at `row`/`column`, using the colours and attributes currently in effect."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/term/core/term.rs:emit_draw_glyph` calculates display width, stamps the glyph cell, and for width `2` stamps a `WIDE_TRAIL` sentinel at `column + 1`; that second stamp is clipped at the right edge. This is a developer-visible cell-layout sharp edge omitted from the page.
SUGGESTED: `` `term::drawGlyph` stamps one scalar at `row`/`column` using the current colours and attributes. A double-width scalar also occupies the following column when it is on the surface; at the right edge, only its first cell is available. ``