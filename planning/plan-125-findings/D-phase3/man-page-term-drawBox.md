### 1. `rowB` omits its range behavior
UNIT:      man-page:term/drawBox  
CLAIM:     “Row of the opposite corner; may be less or greater than `rowA`.”  
VERDICT:   incomplete  
EVIDENCE:  `src/codegen/term/core/term.rs:emit_draw_box` normalizes `rowA`/`rowB` in either order and clamps each resulting edge/corner span; negative coordinates are accepted. The claim also omits the accepted equal case.  
SUGGESTED: Row of the opposite corner. It may be less than, equal to, or greater than `rowA`; rows are zero-based (`0` is the top), and off-surface values, including negative ones, are clamped as part of the box span.

### 2. `columnB` omits its range behavior
UNIT:      man-page:term/drawBox  
CLAIM:     “Column of the opposite corner; may be less or greater than `columnA`.”  
VERDICT:   incomplete  
EVIDENCE:  `src/codegen/term/core/term.rs:emit_draw_box` normalizes `columnA`/`columnB` in either order and clamps each resulting edge/corner span; negative coordinates are accepted. The claim also omits the accepted equal case.  
SUGGESTED: Column of the opposite corner. It may be less than, equal to, or greater than `columnA`; columns are zero-based (`0` is the left), and off-surface values, including negative ones, are clamped as part of the box span.

### 3. Degenerate-box glyph result is omitted
UNIT:      man-page:term/drawBox  
CLAIM:     “A one-cell-wide or one-cell-tall box collapses to a line or a single cell, with the corners drawn last.”  
VERDICT:   incomplete  
EVIDENCE:  The scratch probe at `/tmp/plan-125-scratch/D-phase3/man-page-term-drawBox/geometry/src/main.mfb`, built and run with the specified release binary, printed `╝` for `drawBox(Double, 5, 5, 5, 5)` and `┐`, `│`, `┘` for `drawBox(Light, 8, 8, 10, 8)`. `src/codegen/term/core/term.rs:emit_draw_box` stamps corners in TL, TR, BL, BR order, explaining those final cells.  
SUGGESTED: A one-cell-wide box has `┐`/`┘`-style end cells (or the matching style), and a one-cell-tall box has `└`/`┘`-style end cells, because corners overwrite the edges in order. If both dimensions are one cell, the final bottom-right corner glyph remains.