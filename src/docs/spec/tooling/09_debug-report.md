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
[[src/codegen/engine/builder/mod.rs:lower_module_for_platform]]

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
| | `arena.<n>.fill_grow_calls`, `.fill_grow_bytes` | entropy fills of a newly mapped block and their bytes: each fill covers the block's usable region (its size minus the 32-byte header), and `fill_grow_calls` always equals `grow` |
| | `arena.<n>.fill_free_calls`, `.fill_free_bytes` | entropy scrubs of a freed chunk and their bytes: each covers the chunk past its 16-byte free-list node, so a 16-byte chunk is not scrubbed and not counted, and skipped double frees add no scrub bytes |
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
[[src/codegen/debug/arena.rs:ArenaFeature]]

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
[[src/codegen/debug/arena.rs:lower_sample]]

A `thread_block` arena is destroyed when its control block is freed, so a finished
thread's block arena shows `unmaps` equal to `maps` and `unmapped_bytes` equal to
`mapped_bytes`; its `live_bytes` counts the values freed with the arena rather than
one by one (the seed and the result). The `thread` section reads the thread table
unlocked at exit, like the arena table. [[src/codegen/debug/thread.rs:ThreadFeature]]

The `perf` section maps its own timing region at program entry (never the arena)
and prints each line with a single `write`; it is emitted only for a `macos-aarch64`
`--debug` build. [[src/codegen/debug/perf.rs:PerfFeature]]

## Reporting hooks

Three environment variables make a program write diagnostic output while it runs.
They are reporting hooks, not program behavior, so only a `--debug` build contains
the code that reads them; a normal build ignores them.

| Variable | Build | What the program writes |
| --- | --- | --- |
| `MFB_CANVAS_STATS` | canvas program | appends one line of renderer counters (`generations=`, `frames=`, `skipped=`, `damage=`, …) to the named file for every rendered or skipped frame |
| `MFB_CANVAS_DUMP` | canvas program | writes each presented frame's raw RGBA8 bytes to the named file, overwriting it |
| `MFB_WINAPP_DUMP` | Windows app | when set, the UI thread reads the window transcript back at exit and writes it to stdout as raw UTF-16 |

The canvas hooks live in the canvas package's source, which carries a normal and a
`--debug` body for `__canvas_presentSurface` and the render loop
(`RegistryHelper::debug_split`); the build injects the body matching its flag.
[[src/codegen/registry/mod.rs:debug_split]]
The Windows hook's variable name, buffer and reader are emitted only when the app
entry is built with `--debug`. [[src/target/win_x86_64/app/mod.rs:app_mode_data_objects]]

The switches tests use to drive a program — `MFB_MACAPP_HEADLESS`,
`MFB_WINAPP_HEADLESS`, `MFB_GTKAPP_HEADLESS`, `MFB_CANVAS_SYNC`, `MFB_CANVAS_GPU`,
`MFB_CANVAS_RESIZE_W`/`_H`, `MFB_WINAPP_INPUT` — are not reporting hooks and work in
every build.

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
* ./mfb spec memory program-startup — `_mfb_shutdown` and the exit sequence
