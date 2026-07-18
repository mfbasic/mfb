# App Mode Runtime

The GUI application runtime selected by `mfb build --app`: how a windowed MFBASIC
program boots, where its per-process state lives, how `io::` is redirected to the
window, and how the `term::` TUI is rendered on a drawing surface. This is the
runtime contract a GUI MFBASIC program observes — distinct from the console-mode
program-startup sequence (`./mfb spec memory program-startup`) and from the
Mach-O/ELF container bytes (`./mfb spec linker`).

App mode is dispatched through shared codegen hooks, with the target OS selecting
the toolkit: AppKit on macOS, GTK4 on Linux. [[src/target/shared/code/types.rs:AppEntrySpec]]

Each toolkit has its own single output shape. macOS emits a `build/<name>.app`
bundle; Linux emits a `build/<name>.AppImage` — the AppImage type-2 runtime with
an uncompressed SquashFS image of an AppDir concatenated at the runtime's exact
length, which a user downloads and double-clicks. `--app-debug` retains the
intermediate `build/<name>.AppDir` beside it, itself directly runnable via its
`AppRun`. Neither shape emits a console `.out`. See
`./mfb spec architecture artifacts`.

## Reading order

- `macos-runtime` — the AppKit `_main` bootstrap (NSApplication/NSWindow/menu/
  delegate), the transcript view, the worker pthread shim, and the
  associated-object per-process state scheme; plus the `MFB_MACAPP_HEADLESS`
  test path.
- `linux-runtime` — the GTK4 bootstrap, the `_mfb_gtkapp_state` global, the
  drawing-area term surface, and the documented divergences.
- `console-io` — how `io::write`/`flush`/`input`/`isTerminal`/`terminalSize` are
  re-implemented over a window (the input pipe dup2'd onto fd 0, line vs raw key
  handling).
- `term-backend` — the GUI `term::` grid/cell model, the drawing surface, and the
  content-view swap on `term::on`/`off`.

## See Also

* ./mfb spec memory program-startup — the console-mode entry/teardown sequence
* ./mfb spec architecture commands — the `--app` build flag and `buildMode`
* ./mfb spec linker static-and-dynamic-output — app-mode entry-bootstrap import differences
* ./mfb spec threading os-integration — the worker pthread the window drives
