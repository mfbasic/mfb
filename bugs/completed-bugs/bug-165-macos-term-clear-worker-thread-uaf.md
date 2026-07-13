# bug-165 — macOS app-mode `term::clear` mutates the grid on the worker thread, racing `setFrameSize:` realloc/free (UAF)

Last updated: 2026-07-12
Severity: MEDIUM — use-after-free / heap corruption during window resize in a GUI TUI app.
Class: Memory-safety.
Status: FIXED
Resolution: `emit_app_clear` now marshals the grid clear onto the main thread via
a new `mfbClear:` selector (`[tv performSelectorOnMainThread:@selector(mfbClear:)
withObject:nil waitUntilDone:YES]`), serializing it with `setFrameSize:`/
`drawRect:` like `mfbWriteString:`. The IMP is the existing `TERM_CLEAR_SYMBOL`
helper (reads only `self`); the selector/method are registered in the bootstrap.
macOS app-mode `.ncode` goldens regenerated.

## Finding

`src/target/macos_aarch64/app/app_io.rs:1093` (call site) invokes
`TERM_CLEAR_SYMBOL` via `call_internal` on the **worker thread**, and the helper
`emit_term_clear_helper` (`src/target/macos_aarch64/app/term_view.rs:709`) loads
`TV_CELLS_OFFSET` and `bzero`s the grid buffer directly. Every *other* cell
mutation (`mfbWriteString:`) is marshaled to the main thread via
`performSelectorOnMainThread:waitUntilDone:YES`, serializing it with
`drawRect:`/`setFrameSize:` (term_view.rs:829 comment). `term::clear` is the only
cell-buffer mutation performed directly on the worker. Concurrently
`emit_term_set_frame_size_helper` (`term_view.rs:1257`) runs on the main thread,
`calloc`s a new grid (:1353), publishes it, and `_free`s the old buffer (:1404).
The worker can hold the stale `cells` pointer and `bzero` it after the main
thread frees it → use-after-free / heap corruption (and torn frames vs a
concurrent `drawRect:` read). The helper comment at term_view.rs:708 asserting it
is "safe from the worker thread" predates the plan-35-D `setFrameSize:`
realloc/free.

## Trigger

A GUI app-mode program in active TUI mode calls `term::clear()` in a redraw loop
while the user live-resizes the window. (`term::scroll` is unaffected — only
reached from the already-marshaled `mfbWriteString:`; state-struct mutators race
only the never-freed TVSTATE struct, benign.)

## Fix

Marshal `term::clear`'s grid mutation to the main thread like writes (add a
`mfbClear:`-style selector invoked via `performSelectorOnMainThread:waitUntilDone:YES`)
instead of calling `TERM_CLEAR_SYMBOL` directly from the worker.
