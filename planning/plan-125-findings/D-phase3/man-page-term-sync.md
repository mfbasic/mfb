### 1. Cursor is left where the last drawing call left it, not only where `term::moveTo` put it
UNIT:      man-page:term/sync
CLAIM:     "Each frame ends with the terminal's cursor left where term::moveTo put it, and shown or hidden according to term::showCursor/term::hideCursor."
VERDICT:   misleading
EVIDENCE:  Probe `/tmp/plan-125-scratch/D-phase3/man-page-term-sync/ex1` (the page's own first example) run under a pty via `script`: the frame ends `ESC[0m ESC[2;1H ESC[?25h` — cursor at row 1, column 0, because `io::print("hello")` advanced it past `moveTo(0, 0)` and wrapped on the newline. Probe `diff/src/main.mfb`: after `moveTo(0,4)` + `io::write("p")` the frame ends `ESC[1;6H`; after `moveTo(2,3)` with no write it ends `ESC[3;4H`. Source: `emit_grid_present` (`src/codegen/term/grid/term_grid.rs`) restores from `H_CUR_ROW`/`H_CUR_COL`, which `emit_grid_write` persists after every print/write (`H_CUR_ROW`/`H_CUR_COL` stores at the `gw_persist_skip` tail).
SUGGESTED: "Each frame ends with the terminal's cursor at the surface's current position — where `term::moveTo` put it, or wherever the last `io::print`/`io::write` left it — and shown or hidden according to `term::showCursor`/`term::hideCursor`."

### 2. The resize repaint happens in the same call, not "the next frame"
UNIT:      man-page:term/sync
CLAIM:     "if it changed, it resizes the surface, keeps the content that still fits in the top-left, clamps the cursor into the new bounds, and repaints the next frame in full."
VERDICT:   misleading
EVIDENCE:  Probe `resize/src/main.mfb` run via `runpty.py` with a `TIOCSWINSZ` 24x80 → 10x40 change between two `term::sync` calls: the second `sync` itself emitted the full 10-row repaint (`ESC[1;1H … hello …` through `ESC[10;1H`), preserved "hello" at the top-left, and clamped the cursor from `moveTo(22, 70)` to `ESC[10;40H`. Source: `emit_grid_present` calls `emit_grid_resize` first (sets `H_DIRTY = 1`) and then runs the present loop in the same helper, so the frame being presented is the one drawn in full. "The next frame" reads as the following `sync`.
SUGGESTED: "if it changed, it resizes the surface, keeps the content that still fits in the top-left, clamps the cursor into the new bounds, and presents this frame in full. `term::didResize` reports the change afterwards, and `term::terminalSize` gives the new extent."

### 3. The drawing-call list is presented as exhaustive but omits the `draw*`/`fillRect` family
UNIT:      man-page:term/sync
CLAIM:     "While TUI mode is on, every drawing call — io::print and io::write, and term::clear, term::moveTo, the colour and attribute setters, term::showCursor and term::hideCursor — mutates an in-memory cell grid and the current-attribute state, and touches the terminal not at all."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/term/` registers `drawHLine`, `drawVLine`, `drawBox`, `drawGlyph`, `drawText`, `fillRect`; their emitters in `src/codegen/term/core/term.rs` are documented "Emits no ANSI … shown on the next `term::sync`" (comments at `emit_draw_line`, lines ~1062, 1175, 1354, 1577, 1724). A reader of this sentence would conclude `term::drawText` is not buffered.
SUGGESTED: "While TUI mode is on, every drawing call — io::print and io::write, term::clear, term::moveTo, the term::draw* and term::fillRect calls, the colour and attribute setters, and term::showCursor/term::hideCursor — mutates the surface and the current-attribute state, and touches the terminal not at all."

### 4. See-also omits the two calls the resize paragraph depends on
UNIT:      man-page:term/sync
CLAIM:     "See also • io::print • io::write • term::clear • term::hideCursor • term::moveTo • term::off • term::on • term::showCursor"
VERDICT:   incomplete
EVIDENCE:  The page says a resize "is handled here" and `mfb man term didResize` states "In a terminal it is term::sync that re-reads the size … a program that never calls term::sync never sees a terminal resize at all", yet neither `term::didResize` nor `term::terminalSize` is linked from this page. Probe `resize` printed `didResize=TRUE` immediately after the resizing `sync`. See-also is derived, so the fix is on the descriptor's cross-reference list, not the prose.
SUGGESTED: Add `term::didResize` and `term::terminalSize` to the see-also set for `sync`.

Everything else on the page held up. Both examples compile (`mfb build -q .`) and run under a pty: example 1 paints "hello" then restores the screen; example 2 loops, presents "press q to quit", and exits on `q`. Verified by probe: `sync` before `term::on` and after `term::off` emits nothing (`gate` probe printed "before on ok"/"after off ok" with no escape bytes around them); `term::off` performs a final present (`emit_off` branches to `_mfb_rt_term_term_sync`, and the `diff` probe's "MARK4" cells appeared only in that final present); only changed cells are sent (frames 2 to 4 of the `diff` probe emitted just the modified cells plus the cursor-restore tail); the first frame after `term::on` is full (`emit_grid_alloc` stores `H_DIRTY = 1`); the ioctl-failure and allocation-failure paths keep the old surface (`emit_grid_resize` branches to `skip` on both); app mode schedules one coalesced redraw on macOS (`emit_app_term_sync`, `setNeedsDisplay:` on the main thread) and Linux GTK (`g_idle_add` of the redraw idle). The Windows `--app` arm was not read. No banned memory vocabulary appears on the page, and no sentence teaches a borrow/ownership model.
