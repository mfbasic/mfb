### 1. Endpoint inclusivity is omitted
UNIT:      man-page:term/fillRect
CLAIM:     "The two points `(rowA, columnA)` and `(rowB, columnB)` are **opposite corners**."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/term/core/term.rs:emit_fill_rect` normalizes then iterates `row` through `yhi` inclusively; `emit_stamp_run` likewise stamps through its upper endpoint. The rendered page never states that both supplied corner cells are included.
SUGGESTED: "The two points `(rowA, columnA)` and `(rowB, columnB)` are opposite corners, and the filled region includes both corner cells."

### 2. `rowB` omits its coordinate and clamping contract
UNIT:      man-page:term/fillRect
CLAIM:     "Row of the opposite corner; may be less or greater than `rowA`."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/term/core/term.rs:emit_fill_rect` accepts either order, clamps the normalized row range to `0..rows - 1`, and skips an entirely off-surface range. The edge probe called `fillRect(Filled, -5, -5, -1, -1)` and `fillRect(Light, -1, -1, 0, 0)`; it exited 0 and emitted 0 `█` glyphs and 1 `░` glyph, proving negative endpoints clamp rather than raise.
SUGGESTED: "Row of the opposite corner (zero-based, counting from 0 at the top); may be less or greater than `rowA`. Clamped to the surface."

### 3. `columnB` omits its coordinate and clamping contract
UNIT:      man-page:term/fillRect
CLAIM:     "Column of the opposite corner; may be less or greater than `columnA`."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/term/core/term.rs:emit_fill_rect` passes normalized columns to `emit_stamp_run`, which clamps them to `0..columns - 1` and skips an empty off-surface span. The same negative-coordinate edge probe exited 0, with only the partially overlapping `░` cell emitted.
SUGGESTED: "Column of the opposite corner (zero-based, counting from 0 at the left); may be less or greater than `columnA`. Clamped to the surface."