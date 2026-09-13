# Debug Report

`mfb build --debug` and `mfb test --debug` produce a program that carries a
**debug report**: after everything the program itself printed, the very last
thing the process does before exiting is write one block of machine-readable
lines to stderr. The report exists to measure a program from the outside — it
needs no source change, and a build without `--debug` is byte-identical to one
made before the flag existed.[[src/cli/build/options.rs:parse_build_options]]

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

Each line is written with a single `write`, so another thread's stderr output
cannot land inside a line. A consumer takes the **last** `mfb.debug.begin` block
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

The `perf` section maps its own timing region at program entry (never the arena)
and prints each line with a single `write`; it is emitted only for a `macos-aarch64`
`--debug` build. [[src/codegen/debug/perf.rs:PerfFeature]]

## See Also

* ./mfb spec tooling cli-reference — the `build` and `test` flags
* ./mfb spec memory program-startup — `_mfb_shutdown` and the exit sequence
