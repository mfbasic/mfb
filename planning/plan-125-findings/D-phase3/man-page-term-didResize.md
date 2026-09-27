### 1. App resize is overstated
UNIT:      man-page:term/didResize
CLAIM:     "`term::didResize` returns `TRUE` when the terminal (CLI) or window (`--app`) has changed size since the last time `term::didResize` was called, and `FALSE` otherwise."
VERDICT:   misleading
EVIDENCE:  `src/target/linux_gtk/term_draw.rs:emit_term_resize_helper` sets the flag only when the computed columns or rows differ; `src/target/macos_aarch64/app/term_view.rs:setFrameSize` likewise reaches the flag store only on a genuine grid-dimension change. A pixel-only window resize that leaves the cell extent unchanged returns FALSE.
SUGGESTED: "`term::didResize` returns `TRUE` when the terminal or app window's cell dimensions have changed since the last call, and `FALSE` otherwise."

### 2. First example cannot detect a CLI resize
UNIT:      man-page:term/didResize
CLAIM:     "Reflow a layout only when the terminal changes size:"
VERDICT:   example-broken
EVIDENCE:  The copied example compiled and exited 0 at `/tmp/plan-125-scratch/D-phase3/man-page-term-didResize/example-one`, but only emitted terminal control sequences. Its first and only `term::didResize()` call occurs before `term::sync()`. `src/codegen/term/grid/term_grid.rs:emit_grid_resize` is invoked by `term::sync` (`src/codegen/term/core/term.rs`), so the example never detects a CLI resize or reflows anything.
SUGGESTED: "In a frame loop, present a frame with `term::sync()`, then check `term::didResize()` on the following frame and rebuild the layout when it returns TRUE:"

### 3. Second example cannot reach its advertised resize branch in CLI mode
UNIT:      man-page:term/didResize
CLAIM:     "Re-read the new extent after a resize:"
VERDICT:   example-broken
EVIDENCE:  The copied example compiled and exited 0 at `/tmp/plan-125-scratch/D-phase3/man-page-term-didResize/example-two`, with no `resized to …` output. Like the first example, it checks `term::didResize()` immediately after `term::on()` and never calls `term::sync()` first; `term::sync` is where the CLI implementation re-reads terminal dimensions (`src/codegen/term/grid/term_grid.rs:emit_grid_resize`).
SUGGESTED: "After a frame has detected a resize, read the new extent:"