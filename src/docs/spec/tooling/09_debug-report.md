# Debug Report

`mfb build --debug` and `mfb test --debug` produce a program that carries a
**debug report**: after everything the program itself printed, the very last
thing the process does before exiting is write one block of machine-readable
lines to stderr. The report exists to measure a program from the outside — it
needs no source change. Builds without `--debug` omit the report helpers and data.[[src/cli/build/options.rs:parse_build_options]]

## Enabling

`--debug` is accepted at most once by `build` and by `test`
(`mfb build accepts at most one --debug option`). It is an axis of its own: it
does not change the optimization level, the target, or the build mode, and
`--app --debug` is a valid combination. The flag reaches native codegen as
`NirModule::debug` (a `DebugOptions` value), never as a process global, so the
nested builds of source dependencies are unaffected. A `--debug` build written as
a `--nir` dump says so with a `"debug": true` line; a normal dump is unchanged.
[[src/codegen/debug/mod.rs:DebugOptions]]

The report is emitted only for a module with an entry point. A package, or any
build without `--debug`, gets no report code and no report data.
[[src/codegen/debug/mod.rs:report_enabled]]

## When it prints

The report is written by `_mfb_debug_shutdown`, called as the **last call** of
`_mfb_shutdown` — after its `shutdown_done` label, so both of `_mfb_shutdown`'s
paths reach it: the full teardown, and the early return a second entry takes
(for example SIGINT/SIGTERM arriving during normal cleanup). A once-guard, set
before the first byte is written, makes any later arrival print nothing, so a
process prints at most one block.
[[src/codegen/os/process/process_lifecycle.rs:lower_shutdown]]
[[src/codegen/debug/shutdown.rs:lower_debug_shutdown]]

Every exit that reaches `_mfb_shutdown` therefore prints it: a normal return from
the entry function, `EXIT PROGRAM n`, an untrapped error (after the error text),
the SIGINT/SIGTERM handler of a Unix console program, and the finish of an app
build's worker. The report never changes the exit status, stdout, or anything on
stderr before the block.

An exit that does **not** reach `_mfb_shutdown` prints no report. That includes a
crash (a fatal signal or access violation), Ctrl-C on a Windows console program
(no console control handler is installed), and a signal delivered to an app-mode
build (app builds install no console signal handlers).
[[src/codegen/engine/builder/lower_module.rs:lower_module_for_platform]]

The report helper runs after the main arena has been destroyed, so it never
allocates and never reads the arena register; each line it writes is assembled
from constants or in its own stack frame.

## Format (version 1)

Every line is `<key> <value>` followed by a newline, written to file descriptor 2.
Keys are dot-separated; a value is a decimal integer or a single token with no
spaces. The block is bracketed by `begin` and `end` lines whose value is the
format version:

```
mfb.debug.begin 1
mfb.debug.target macos-aarch64
mfb.debug.build console
mfb.debug.end 1
```

Each line is submitted in a single `write`; the helper does not retry short
writes or errors. Atomicity depends on the destination and operating system.
[[src/codegen/debug/write.rs:emit_write_window]] A consumer takes the **last** `mfb.debug.begin` block
in stderr.

`mfb dbg` depends on that shape: it takes each line at the `write` call, by the
thread inside `_mfb_debug_shutdown` and to descriptor 2, and skips the call, so
under the debugger the block does not reach stderr (`./mfb spec tooling
debugger`, *show report*).

## Sections

Sections print in the order of the compiler's feature registry,
`DEBUG_FEATURES`; each owns the keys that start with its name.
[[src/codegen/debug/mod.rs:DEBUG_FEATURES]]

| Section | Keys | Meaning |
| --- | --- | --- |
| `mfb.debug` | `mfb.debug.target` | the target the program was compiled for (`macos-aarch64`, `linux-aarch64`, `linux-x86_64`, `linux-riscv64`, `windows-x86_64`) |
| | `mfb.debug.build` | `console` or `app` |
| `perf` (macOS only) | `perf.<span>.count`, `.avg`, `.median`, `.min`, `.max`, `.sum` | for each timed span — `program` (from entry to the report), `mfb_alloc` and `mfb_free` (every arena allocation and free call) — the number of samples and their average, median, minimum, maximum, and total duration in nanoseconds; a span that never ran prints nothing |
| | `perf.mismatch`, `perf.overflow` | printed only when non-zero: a span end with no open start, and samples dropped because the timing region filled |
| `arena` | `arena.count` | the number of arenas registered during the run |
| | `arena.registry_overflow` | arenas that could not register because all 1024 slots were taken; their events are not counted |
| | `arena.<n>.kind` | `main` (the entry thread), `worker` (a `thread::start` thread), `thread_block` (a thread's control-block arena, registered just before its `worker`), or `graphics` (a canvas program's graphics thread), numbered in registration order |
| | `arena.<n>.maps`, `.mapped_bytes`, `.unmaps`, `.unmapped_bytes` | blocks the arena mapped from the OS and returned to it, and their bytes |
| | `arena.<n>.alloc_calls`, `.alloc_bytes` | allocation calls and the bytes requested, after size and alignment rounding |
| | `arena.<n>.hit_quick_bin`, `.hit_carve`, `.hit_large_bin`, `.hit_walk`, `.grow` | which path served each allocation: a small-size bin, a split of the current carve chunk, a large-bin size class (a large request, or a small one taking a large chunk as its carve chunk), a free-list walk, or a newly mapped block; the five always add up to `alloc_calls` |
| | `arena.<n>.free_calls`, `.free_bytes` | free calls and their bytes, including frees that were skipped |
| | `arena.<n>.double_free_skips` | frees skipped because the chunk was already free |
| | `arena.<n>.live_bytes`, `.peak_live_bytes` | bytes allocated and not yet freed when the report ran, and the most ever held at once |
| | `arena.<n>.flushes`, `.insert_free_calls` | free-list coalescing passes, and chunks inserted into the coalescing free list |
| | `arena.<n>.fill_grow_calls`, `.fill_grow_bytes` | zero fills of a newly mapped block and their bytes: each fill covers the block's usable region (its size minus the 32-byte header), and `fill_grow_calls` always equals `grow` |
| | `arena.<n>.fill_free_calls`, `.fill_free_bytes` | zero scrubs of a freed chunk and their bytes: each covers the chunk past its 16-byte free-list node, so a 16-byte chunk is not scrubbed and not counted, and skipped double frees add no scrub bytes |
| | `arena.<n>.series.count` | the number of memory-over-time samples that follow, at most 256 |
| | `arena.<n>.series.<i>.t_ns`, `.mapped_bytes`, `.live_bytes`, `.peak_rss_bytes` | one sample per line group, oldest first: nanoseconds since the first arena registered (monotonic clock), the arena's mapped and live bytes just after a grow, and the process's peak RSS read at that moment the same way as `process.peak_rss_bytes` |
| `process` | `process.peak_rss_bytes` | the most physical memory the process has held at once, in bytes, as the operating system counts it when the report runs: `ru_maxrss` from `getrusage` on macOS and Linux (Linux's KiB scaled to bytes), `PeakWorkingSetSize` from `K32GetProcessMemoryInfo` on Windows; `0` if the call fails |
| `thread` (a program that starts threads) | `thread.table_capacity` | slots in the thread table's array: 0 before the first `thread::start`, then 64, doubling each time it fills |
| | `thread.table_high` | slots ever handed out — the most control blocks alive at once |
| | `thread.table_live` | control blocks not yet freed when the report ran |

The `arena` section is emitted for every target. Every arena registers in a
shared table when it is created, and each allocator helper adds to its own
arena's counters there. The helpers do this only in a `--debug` build, so a normal
build's allocator is unchanged. The report reads the table without stopping
other threads, so it is a **snapshot**: a thread still running at exit may have
updated some counters and not others. A finished thread's counters are final.
`live_bytes` stops at zero when a subtraction would take it below: a free of memory
allocated before the arena registered was never added, so the value can
undercount but never wraps.
[[src/codegen/debug/arena/mod.rs:ArenaFeature]]

Each arena also records a **series**: memory over time, sampled when the arena grows
(maps a new block), not by a timer. An arena that never grows after its first block
has a single sample.
- Every grow reads the clock and the peak RSS and writes a sample after the kept ones.
- The sample is kept when the arena's `grow` count has reached the next kept grow.
  That starts at the first grow, with a stride of 1.
- When 256 samples are kept, the series halves: it keeps every other sample from the
  first, plus the newest, so 129 remain. The stride then doubles.
- A grow that is not kept stays as a provisional sample until the next grow replaces
  it, and the report prints it last and counts it in `series.count`.

So the series stays within 256 samples, the first is the arena's first sampled grow,
and the last is always its most recent grow. The samples are spaced roughly evenly in
grow count, not in time.
[[src/codegen/debug/arena/sample.rs:lower_sample]]

A `thread_block` arena is destroyed when its control block is freed, so a finished
thread's block arena shows `unmaps` equal to `maps` and `unmapped_bytes` equal to
`mapped_bytes`; its `live_bytes` counts the values freed with the arena rather than
one by one (the seed and the result). The `thread` section reads the thread table
unlocked at exit, like the arena table. [[src/codegen/debug/thread.rs:ThreadFeature]]

The `perf` section maps its own timing region at program entry (never the arena)
and prints each line with a single `write`; it is emitted only for a `macos-aarch64`
`--debug` build. [[src/codegen/debug/perf.rs:PerfFeature]]

## Environment variables a build reads

A compiled program reads four environment variables in every build:

| Variable | Program | Effect |
| --- | --- | --- |
| `MFB_HEADLESS` | app | an app-mode program skips its window and event loop; a canvas program also defaults to the software renderer |
| `MFB_CANVAS_GPU` | canvas | the canvas renderer choice |
| `MFB_CANVAS_SYNC` | canvas | any non-empty value makes `present` wait for its frame; unset, `present` never waits |
| `MFB_MOUSE_INJECT` | `term::enableMouse` and `canvas::enableMouse`, POSIX targets | raw SGR bytes fed through the mouse decoder |

`MFB_CANVAS_SYNC` and `MFB_MOUSE_INJECT` are test switches. They are read by every
build because `tools/canvas-bench` measures release builds with them (`sweep --release`
and `hit`), and a `--debug` build's instrumentation would change the timings.
[[src/codegen/builtins/canvas/helper_render/render_loop.rs:ENSURE_GRAPHICS]]
[[src/codegen/term/core/term/mouse.rs:emit_mouse_inject]]

Every other `MFB_` variable a program reads is read only by a `--debug` build: the
variable name and the code that reads it are both absent from a normal build's binary,
so a normal build ignores them. This section is the canonical list.

**Reporting hooks** make the program write diagnostic output while it runs.

| Variable | Program | What the program writes |
| --- | --- | --- |
| `MFB_CANVAS_STATS` | canvas | appends one line of renderer counters (`generations=`, `frames=`, `skipped=`, `damage=`, ...) to the named file for every rendered or skipped frame; its timing fields are under "The canvas frame profile" below |
| `MFB_CANVAS_DUMP` | canvas | writes each presented frame's raw RGBA8 bytes to the named file, overwriting it |
| `MFB_CANVAS_FACE_DUMP` | canvas | appends one line per loaded font to the named file: `face` and then every word of the font's face index, the parse-once form of its metrics, `cmap` and advances [[src/codegen/builtins/canvas/helper_font.rs:FACE_DUMP_DEBUG]] |
| `MFB_CANVAS_FACE_CHECK` | canvas | on each `canvas::measureText`, holds the measured font's face index to its font bytes for every Unicode scalar, every glyph id and the four metrics, and appends `faceCheck glyphMismatches=… advanceMismatches=… metricMismatches=… mapped=… glyphIds=… runs=… kern=…` to the named file, where `kern=` lists `left:right:x1:x2` for every pair of the first 64 glyph ids the font kerns. It costs seconds per call on a large font |
| `MFB_CANVAS_SHAPE_DUMP` | canvas | appends one line per string `canvas::measureText` shapes: `shape` and then the shaped run's words, six per glyph (font index, glyph id, advance in font units, scalar index of its first codepoint, 1 when a grapheme boundary follows the glyph, and how many codepoints it stands for) [[src/codegen/builtins/canvas/helper_font.rs:FACE_DUMP_DEBUG]] |
| `MFB_TRANSCRIPT_DUMP` | app (macOS, Linux GTK, Windows), windowed run | when the program finishes, writes the transcript window's text to stdout and exits with the program's exit code, instead of appending `Program exited with code N` and keeping the window open |

`MFB_TRANSCRIPT_DUMP` output is UTF-8 on macOS and Linux GTK and raw UTF-16 on
Windows (the UI thread reads the transcript back with `WM_GETTEXT`). A headless run
has no transcript window, so the variable does nothing there. On macOS the finish
helper runs the dump on the main queue with `dispatch_sync_f`; on Linux GTK it
schedules an idle callback with `g_idle_add` that writes the buffer to fd 1 and calls
`_exit`.
[[src/target/macos_aarch64/app/bootstrap.rs:emit_transcript_dump_helper]]
[[src/target/linux_gtk/bootstrap.rs:emit_transcript_dump_idle_helper]]
[[src/target/win_x86_64/app/mod.rs:app_mode_data_objects]]

**Test switches** let a harness drive a program deterministically.

| Variable | Program | Effect |
| --- | --- | --- |
| `MFB_CANVAS_DAMAGE` | canvas | turns damage tracking on; unset, damage tracking is off |
| `MFB_CANVAS_DISPLAY_HZ` | canvas | overrides the display refresh rate the runtime reports; unset, no override |
| `MFB_CANVAS_VSYNC` | canvas app, windowed run on macOS | `0` turns the window's Metal layer's display sync off; unset or any other value, the layer's default (on) |
| `MFB_CANVAS_FRAME_HOLD_MS` | canvas | holds the graphics thread inside a frame; unset, no hold |
| `MFB_CANVAS_GLYPH_BUDGET` | canvas | sets the glyph cache byte budget; unset, the budget is 1048576 bytes |
| `MFB_CANVAS_POINTER` | canvas | a pointer script, `ms,x,y,buttons,inside;…`, published by the graphics thread as each entry falls due |
| `MFB_CANVAS_AUTO_WINDOW_MS` | canvas | overrides how long an `AutoPaint` enter or exit paint is held, in milliseconds; unset, the project's `options.canvasAutoPaintMS` |
| `MFB_CANVAS_GEO_VERIFY` | canvas | `1` rebuilds every geometry record with the MFBASIC builders and counts those that differ from the native builder's, on the stats line |
| `MFB_CANVAS_RESIZE_W` / `_H` | canvas app, headless run on Linux GTK and Windows | publishes a surface size in pixels |
| `MFB_CANVAS_POINTER_X` / `_Y` | canvas app, headless run on Linux GTK and Windows | delivers a pointer position to the platform callback |
| `MFB_WINAPP_INPUT` | Windows app, windowed run | posts each character of the value to the input control, then Enter |
| `MFB_WINAPP_TERM_RESIZE_W` / `_H` | Windows `term::` app, headless run | gives the window a client size in pixels |

In a normal build, therefore, damage tracking is always off, the glyph cache budget
is always 1048576 bytes, the window layer's display sync is always on, and there is no
display-rate override or frame hold. The `MFB_CANVAS_RESIZE_*` and
`MFB_CANVAS_POINTER_X`/`_Y` variables are never read on macOS.

The canvas variables live in the canvas package's source, which carries a normal and
a `--debug` body for each reading helper (`RegistryHelper::debug_split`); the build
injects the body matching its flag.
[[src/codegen/registry/descriptors.rs:debug_split]]
[[src/codegen/builtins/canvas/helper_render/render_loop.rs:TEST_SWITCHES_DEBUG]]
[[src/codegen/builtins/canvas/helper_damage.rs:DAMAGE_SWITCH_DEBUG]]
[[src/codegen/builtins/canvas/helper_glyph_cache.rs:GLYPH_BUDGET_DEBUG]]
The Linux GTK bootstrap emits its variable names and the `atoi`/`usleep` imports only
for `--debug`.
[[src/target/linux_gtk/bootstrap.rs:emit_main_bootstrap]]
[[src/target/linux_gtk/mod.rs:debug_app_mode_imports]]
A macOS canvas program imports `clock_gettime` for `_main` only for `--debug`, for the
frame profile below.
[[src/target/macos_aarch64/plan.rs:debug_app_mode_imports]]
The Windows entry emits its names and readers only for `--debug`.
[[src/target/win_x86_64/app/mod.rs:emit_main]]

## The canvas frame profile

A `--debug` build's `MFB_CANVAS_STATS` line carries four phase totals for the graphics
thread's frame, in milliseconds: `phaseOffsetsMs=`, `phaseDrawsMs=`, `phaseDamageMs=`
and `phaseRenderMs=`. The last covers the whole call into whichever renderer drew the
frame. [[src/codegen/builtins/canvas/helper_render/render_loop.rs:PHASE_TIMERS_DEBUG]]

On macOS the Metal renderer splits that call. It keeps twelve 64-bit words in the data
object `_mfb_rt_canvas_profile`, and the stats line prints each one after
`phaseRenderMs=`. Every word is cumulative over the run, so one frame's share is the
difference between two lines.
[[src/codegen/runtime/canvas/mod.rs:CANVAS_PROFILE_SYMBOL]]
[[src/target/macos_aarch64/app/metal.rs:emit_profile_mark]]

| Field | Unit | What it totals |
| --- | --- | --- |
| `profAcquireUs=` | microseconds | taking the window's drawable, including the main-thread hop that sizes its layer |
| `profFillUs=` | microseconds | writing the frame buffer: item blocks, edges, band index, gradient stops, dash words, glyph and picture texels |
| `profEncodeUs=` | microseconds | recording commands: the render pass and its bindings, one draw per draw-list entry, the copy into the drawable |
| `profWaitUs=` | microseconds | `commit` to `waitUntilCompleted` returning |
| `profGpuUs=` | microseconds | the command buffer's `GPUEndTime` minus its `GPUStartTime` |
| `profReadbackUs=` | microseconds | `getBytes:` into the CPU surface and the BGRA to RGBA reorder |
| `profPresentUs=` | microseconds | `[drawable present]` |
| `profPasses=` | count | command buffers submitted and waited for |
| `profBytes=` | bytes | frame-buffer bytes written, as each region's cursor at the end of a fill times its element size |
| `profTexels=` | count | glyph and picture texels copied into the frame buffer |
| `profDraws=` | count | draw calls, one per draw-list entry |
| `profVsyncOff=` | flag | 1 once `MFB_CANVAS_VSYNC=0` has turned the window layer's display sync off, else 0; not cumulative |

The time fields are measured with `clock_gettime(CLOCK_MONOTONIC)`: one reading when
the render target is ready, then seven more, each adding the nanoseconds since the
reading before it to one word (`profEncodeUs=` takes two of them, either side of the
fill). The stats line divides by 1000. `profGpuUs=` is the device's own interval, not one of those
points, and a negative interval adds nothing.
[[src/target/macos_aarch64/app/metal.rs:emit_profile_gpu_time]]

Which fields a frame adds to depends on where it goes. A frame presented to the window
adds to `profAcquireUs=` and `profPresentUs=` and not to `profReadbackUs=`. A frame
read back adds to `profReadbackUs=` and not to `profPresentUs=`; a headless run reads
every frame back. A frame drawn in several passes adds each pass's fill, encode and
wait, and only its last pass reads back or presents. A pass that ends early (no
drawable, or a command buffer that did not complete) keeps what it had added.

`profBytes=` leaves out the picture table's records.
[[src/target/macos_aarch64/app/metal.rs:emit_profile_counts]]

**Display sync.** With the layer's default, a windowed frame's wait for the display's
next refresh is inside `profAcquireUs=`: `nextDrawable` blocks until the compositor
releases a drawable. `MFB_CANVAS_VSYNC=0` makes the main thread's layer apply send
`setDisplaySyncEnabled:NO` when it first configures the layer, and set `profVsyncOff=`
to 1. It does not take the refresh out of a frame. In a window the compositor still
paces the drawables, and the wait moves from `profAcquireUs=` into `profGpuUs=` and
`profWaitUs=`: measured on one moving rectangle at 120 Hz, 120 frames, the per-frame
medians went from acquire 7.4 ms, wait 0.8 ms, GPU 0.23 ms to acquire 0.05 ms, wait
8.3 ms, GPU 8.0 ms, and the frame time from 8 to 16 ms. So with display sync on,
`profAcquireUs=` is the refresh wait and the other times are the frame's own work; with
it off, the GPU times are not. A headless run configures no layer, so the variable does
nothing there and `profVsyncOff=` stays 0.
[[src/target/macos_aarch64/app/metal.rs:emit_vsync_switch]]

A rendered frame's line is written before that frame's `phaseRenderMs=` is added, so
on any one line the `prof…` fields include the frame being reported and
`phaseRenderMs=` does not.
[[src/codegen/builtins/canvas/helper_render/present.rs:DIRECT_PRESENT_DEBUG]]

A software frame writes nothing to the block. On Linux and Windows no renderer writes
it, so a `--debug` build there prints every `prof…` field as 0.

A normal build has no block, no clock reads in the renderer and no `GPUStartTime` or
`GPUEndTime` selector: `emit_metal_draw` emits the same instructions it would without
the profile, and `canvas::profileWord`, which reads the block, is called only from the
`--debug` phase timers.
[[src/target/macos_aarch64/app/metal.rs:emit_metal_draw]]
[[src/codegen/runtime/canvas/mod.rs:emit_profile_word]]
[[src/target/macos_aarch64/app/metal.rs:metal_profile_data_objects]]

Nor does a normal build's layer apply read `MFB_CANVAS_VSYNC` or send
`setDisplaySyncEnabled:`; both are in `emit_metal_layer_apply` only when it is built for
`--debug`.
[[src/target/macos_aarch64/app/metal.rs:emit_metal_layer_apply]]

## Symbol tables

`--debug` has a second, independent effect: the linker writes every symbol the
encoder defined — each function, runtime helper and data object — into the
executable's native symbol table, so debuggers, profilers, `nm` and `atos` show
names instead of raw addresses. Each target backend stamps the flag onto the
encoded image (`EncodedImage::debug_symbols`), and every output shape of that
target (console, `.app`, AppDir) shares the one writer that reads it.
[[src/arch/image.rs:EncodedImage]] [[src/os/debug_symbols.rs:debug_symbols]]

The table holds **names only**: no DWARF, no line tables, no variable locations,
no dSYM and no PDB. The names are the native symbols verbatim (`_mfb_fn_main`,
`_mfb_rt_io_io_print`, `_mfb_str_0`; see `./mfb spec linker symbols-and-relocations`).
Linker-generated import stubs get no name. A build without `--debug` writes none of
these names, on any target: its output is byte-identical to a compiler without the
feature.

| Format | Table | Status |
| --- | --- | --- |
| Mach-O (`macos-aarch64`) | local `LC_SYMTAB` entries, `LC_FUNCTION_STARTS`, and a content-derived `LC_UUID` (see `./mfb spec linker macos-aarch64`) | written |
| ELF (`linux-*`) | `.symtab` / `.strtab` of local `STT_FUNC` / `STT_OBJECT` entries, in a section header table the build gains for it (see `./mfb spec linker static-and-dynamic-output`) | written |
| PE (`windows-x86_64`) | a COFF symbol table and string table past the last section (see `./mfb spec linker windows-x86_64`) | written; Microsoft's debuggers and profilers read only a PDB, which is not written, so they still show no names |

`mfb test --debug` builds its test programs through the same backend, so they carry
the table too.

## See Also

* ./mfb spec tooling cli-reference — the `build` and `test` flags
* ./mfb spec linker symbols-and-relocations — the shared debug symbol model
* ./mfb man debug report — the developer guide to this report; ./mfb man debug covers `mfb dbg`, and ./mfb man debug-external breakpoints, backtraces, profiling and address lookup with each platform's tools
* ./mfb spec memory program-startup — `_mfb_shutdown` and the exit sequence
