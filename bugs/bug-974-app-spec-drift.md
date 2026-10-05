# bug-974: `mfb spec app` (macOS runtime, Linux runtime, console-io, term-backend, presentation-mode, canvas) — stale layouts, counts, a dangling citation and a wrong enum-naming rule

Last updated: 2026-10-05
Effort: small (<1h) – medium
Severity: LOW
Class: Other (SPEC — documentation drift; code is the reference)

Status: Open
Regression Test: n/a (doc); `scripts/` citation sweep should catch item 1

Each item: spec text (file:line) → what the code / a run shows → evidence command.

### presentation-mode (`src/docs/spec/app/05_presentation-mode.md`)

1. **Dangling citation** `[[src/codegen/engine/builder/mod.rs:lower_module]]` (static
   initial mode, l.73). No `fn lower_module` exists (`grep -rn "fn lower_module\b" src`
   → nothing; only `lower_module_for_platform`, `lower_module_mir_for_platform`,
   `lower_module_resource_drain`). The decision is in `lower_module_for_platform`
   (`builder/mod.rs`, `module_uses_call(module, "app.setMode")`).
2. **Wrong naming rule** (l.30–33): "The enum is referenced bare, like every other
   built-in type: `Mode.None`, not `app::Mode.None`." The compiler rejects bare `Mode`
   (`LET m AS Mode = app::getMode()` → `2-201-0015 SYMBOL_UNKNOWN_TYPE: Type `Mode` is
   not a built-in or top-level project type`; `Mode.Console` → `2-201-0011`), and
   `mfb man app` says "Name the enum package-qualified, as in app::Mode.None; a bare Mode
   is not a known type." (`/tmp/hardening/B17/mode1`).

### canvas (`06_canvas.md`)

3. l.3 `app::setMode(Mode.Canvas)` — bare `Mode` does not compile (item 2); and l.875
   "from a `Mode.Term` build" — there is no `Mode.Term` member (members: Console, None,
   Canvas per `mfb man app`).

### macos-runtime (`01_macos-runtime.md`)

4. l.32 "`_main` stack frame (FRAME_SIZE = 32)" and its 4-row diagram:
   `src/target/macos_aarch64/app/mod.rs` has `FRAME_SIZE: usize = 96` with
   `OFF_ATTR = 32` (`[sp+32..96]` = the program thread's `pthread_attr_t`, bug-803) —
   the same topic later says both spawn sites pass an attr.
5. l.93 delegate "synthesized with two methods": `bootstrap.rs` also adds
   `mfbReconcile:` (`if with_reconcile`), `mfbBlit:` and a second canvas method
   (`if uses_canvas`), and the window-sync / fullscreen-notification IMPs
   (`if uses_window`, documented in `07_window.md`).
6. Associated-object key table omits keys the code defines and uses:
   `CANVAS_VIEW_ASSOC_KEY` (`_mfb_macapp_canvas_view_key`), `RECONCILE_TV_KEY`,
   `RECONCILE_TV_CLASS_KEY` (`grep -n "_key\"" src/target/macos_aarch64/app/mod.rs`).
7. l.216 "App mode keeps no writable data segment for runtime state" — contradicted by
   `07_window.md` (four writable `_mfb_rt_app_*` objects, `_mfb_macapp_delegate`
   global) and by the canvas scene ring (`CANVAS_SCENE_SYMBOL`, "a writable data
   symbol", `.ai/canvas-threading.md` §3).
8. "Only a `--debug` build keeps the window open": a `--debug` program whose last mode
   is `Canvas` or `None` takes `headless_exit` because `ASSOC_KEY` was cleared by the
   reconcile, so its window closes too (measured: `/tmp/hardening/B17/ms7`, `--debug`,
   ends in `Canvas` → process exits 0 on its own with the debug report). The keep-open
   condition is "`--debug` **and** the transcript is attached".

### term-backend (`04_term-backend.md`)

9. l.452 "Twelve methods are added" — the table below lists 14, and
   `bootstrap.rs` registers 14 (drawRect, isFlipped, mfbWriteString, mfbClear,
   mfbPresent, mfbDrawLine, mfbDrawBox, mfbFillRect, mfbDrawGlyph, mfbDrawText,
   acceptsFirstResponder, keyDown, keyUp via `emit_install_key_up`, setFrameSize).
10. `term_init` steps: l.554 "`state = calloc(1, 96)`" — code is
    `calloc(1, TV_STATE_SIZE)` = 344 (the same topic's table says 344); step 2
    "`[NSFont userFixedPitchFontOfSize:N]`" — `emit_term_init_helper` uses
    `fontWithName:@"Menlo-Regular"` first, user fixed-pitch only as fallback (the
    `drawRect:` section says so).
11. l.415 console header layout "out buffer `rows*cols * 72 + 64`" — `OUTBUF_PER_CELL`
    = 136 (`src/codegen/term/grid/term_grid.rs`), and the block omits the EGC pool
    region; a later paragraph corrects it, the layout block was not updated.
12. l.807 (Linux) "`io::terminalSize` and interactive resize are implemented" — there
    is no `io::terminalSize` (03_console-io.md l.27: "Not part of `io::`"); it is
    `term::terminalSize`.
13. Linux "Renderer + ops": `cairo_select_font_face("monospace", …)` /
    `cairo_show_text` — superseded by Pango per the same topic's plan-70-E paragraph
    (`term_draw.rs` calls `pango_layout_get_pixel_extents`, `pango_cairo_*`).

### linux-runtime (`02_linux-runtime.md`)

14. `_mfb_gtkapp_state` layout table + diagram: code inserts `ST_TERM_DID_RESIZE` at
    1216 (so `ST_TERM_CHARS` = 1224, `ST_TERM_FG` = 31944, …), adds
    `ST_TERM_POOL`/`ST_TERM_SNAP_POOL` (32 B/cell each), `ST_HELD`,
    `ST_CANVAS_AREA`/`_SURFACE`/`_PIXELS`, `ST_DISPLAY_HOOKED`. `STATE_SIZE` is
    677 104, not 185 536: `nm -S lx1/build/p1-glibc.AppDir/usr/bin/p1` (a `--debug
    --target linux-aarch64` build) → `00000000000a54f0 d _mfb_gtkapp_state`
    (0xa54f0 = 677104). "13 u64 slots" (l.230) → 14.
15. l.271 term_init "measures the monospace cell from Cairo `font_extents.height` …
    `text_extents("M")`" — code measures with `pango_layout_get_pixel_extents`
    (`term_draw.rs`).
16. l.280 `_mfb_gtkapp_term_write` "… then `g_idle_add(redraw_idle)`" — the writer
    schedules nothing (`grep -n g_idle_add src/target/linux_gtk/term_draw.rs` → none);
    redraw is present-driven, as `04_term-backend.md` states.
17. l.306 "unlike the `io::terminalSize` divergence below" — no such divergence exists
    below (the divergence list has fd fallback, finish, printError, resize).

### console-io (`03_console-io.md`)

18. The per-call table omits `io::setBuffered` / `io::isBuffered`. In app mode the
    buffer is inert (`mfb man io setBuffered`: "In app mode the buffer is inert and this
    call does nothing"); measured headless: `isBuffered()` is `FALSE` after
    `setBuffered(TRUE)` (`/tmp/hardening/B17/bf`). Add the row.

## Goal

- Each item corrected to the code (or, for 2/3, the man page) in the same topic.

### Non-goals

- No code change.
