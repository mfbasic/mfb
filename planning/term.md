1. (done) **`fill` / `fillRect(x1,y1,x2,y2)`** — paint a rectangular region with the current background (optionally a glyph). Right now `clear` is whole-screen only; there's no way to paint a panel background or erase a sub-region. This is the biggest everyday gap — every panel/dialog needs it.
2. **Junctions (`┼ ├ ┤ ┬ ┴`)** — the meatiest *line-drawing* gap. Today, when a `drawVLine` crosses a `drawHLine`, the later call just overwrites the cell (you get `│`, not `┼`). Tables, split panes, and adjoining boxes need auto-joining intersections. Options: a `drawGrid(...)`, or make the line drawers *merge* at crossings (what notcurses does), or expose a `LineStyle` junction set.
3. (done) **`drawText(x, y, text)`** — draw a string at an absolute position. Achievable today with `moveTo` + `io::write`, but every TUI has the one-shot form (`mvaddstr`), and it avoids disturbing the shared cursor.
4. (done) **`drawGlyph`/`putCell(x, y, codepoint)`** — stamp a single arbitrary glyph at a coordinate (not just box glyphs). Minor — `moveTo`+`write` covers it — but clean for sprite-ish drawing.
5. (done) **A filled-box option** (`drawBox` with fill, or `fillBox`) — border + interior in one call. Follows from #1.
6. **More text attributes** — you have only bold + underline. Missing the common set: **reverse/inverse**, **italic**, **dim**, **strikethrough**, **blink**. Reverse and italic especially.
7. **Decoded key input** — the biggest *interactivity* gap. `io::readChar`/`readByte` give raw bytes, so arrow keys, function keys, Home/End/PgUp arrive as undecoded escape sequences. A TUI needs key *events* (`Up`, `F5`, `Enter`, modifiers). notcurses/ncurses decode these.
8. **Mouse events** — clicks/drag/scroll. Absent entirely.
9. **Windows / planes** — subsurfaces with their own coordinate space, clipping, and z-order (ncurses `WINDOW`/`panel`, notcurses `plane`). This is the defining abstraction of both libraries and your largest *architectural* gap; `term::` is a single flat surface today. Optional if you deliberately want flat-surface-only.
10. **Wide-character cell width** — you're one-cell-per-scalar, so CJK/emoji (double-width) will misalign. notcurses handles this.
11. **`term::didResize()`** — returns true after the term was resized, should be cached so it stays true after a resize until the `didResize()` is called. Should support both CLI and `--app` modes.

**Windows / planes** —

term::openPane(x1, y1, x2, y2, z) AS Integer
term::closePane(id AS Integer) AS Nothing
term::setPane(id AS Integer) AS Nothing

a id of 0 is always the default full terminal pane

openPane makes a new subsurfaces with their own coordinate space, clipping, and z-order
closePane terminate an existing subsurface
setPane sets what subsurface is in use, all term::draw*, term::fillRect and term::clear go to a specific pane. attributes/colors/etc are global
