# bug-1118: macOS `--app` `term::drawGlyph` drops a wide glyph in the last column instead of drawing its first cell

Last updated: 2026-10-26
Effort: small (≤1h)
Severity: LOW
Class: Runtime (app backend parity)

Status: Open
Regression Test: —

`mfb man term drawGlyph` says a wide glyph placed in the last column has only
its first cell on the surface. The console and the Linux GTK app backend do
that. The macOS app backend drops the glyph entirely.

**The correct behavior a fix produces:** on macOS `--app`,
`term::drawGlyph(0, cols - 1, 23383)` (`字`) shows the glyph's first cell in the
last column, matching the console and GTK.

References:

- Found by plan-125-D Phase 6, finding `term-drawGlyph` 1 (source only; no window
  was run).
- `src/target/macos_aarch64/app/term_view.rs:app_stamp_cluster` ("A wide
  cluster needs col+1 … else it is dropped"), at the `"dgl_s"` call site;
  compare `src/target/linux_gtk/term_draw.rs:emit_term_glyph_helper`.

## Failing Reproduction

An `--app` build in `app::Mode.Console` that calls
`term::drawGlyph(0, term::terminalSize().columns - 1, 23383)` then `term::sync()`.

- Observed (by reading): nothing is drawn in the last column.
- Expected: the first cell of `字`.
