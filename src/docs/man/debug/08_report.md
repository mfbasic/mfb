# report

Every line of the debug report a --debug build prints at exit

## Description

A program built with `--debug` writes one block of lines to stderr as the
very last thing it does: after everything it printed itself, and after any
error message. Each line is a key and a value:

```
mfb.debug.begin 1
mfb.debug.target macos-aarch64
mfb.debug.build console
...
mfb.debug.end 1
```

The report prints when the program ends normally, when it ends with
`os::exit`, when an error nobody trapped ends it (the error message first,
then the report), and when you stop a macOS or Linux console program with
Ctrl-C. It does not print when the program crashes outright, when you press
Ctrl-C in a Windows console, or when an app-mode program receives a signal.

The report changes nothing else: the exit status, standard output, and
everything on stderr before the block are what they would be without
`--debug`. To keep the report apart from the program's own output, send stderr
to a file (the examples below do).

Run under `mfb dbg`, the report does not print: the debugger keeps it, so it
does not run into the session, and its `show report` command prints it. The
program's last line says so:

```
--[EX] Program exit (0) - type "show report" to see the performance and memory report
```

`show report` works once the program has ended, and until you `restart` it or
`quit`. Before the end it answers `No performance and memory report yet: the
program writes it as it exits.`, and for a program built without `--debug`
that there is none. Two cases leave the report to the program, which then
prints it on stderr as usual: you stepped into the report with `step`, or the
program stopped while writing it (at a breakpoint of yours, or on Ctrl-C).
When it stopped part way, `show report` prints the lines kept until then and
says the report is incomplete.

Numbers are whole numbers. Times are in nanoseconds, sizes in bytes. Each
section of keys below appears in the order it prints.

### `mfb.debug` — the frame

| Key | Meaning |
| --- | --- |
| `mfb.debug.begin` | the report starts; the value is the format version, `1` |
| `mfb.debug.target` | the platform the program was built for: `macos-aarch64`, `linux-aarch64`, `linux-x86_64`, `linux-riscv64` or `windows-x86_64` |
| `mfb.debug.build` | `console`, or `app` for a program built with `--app` |
| `mfb.debug.end` | the report is complete; the value repeats the version |

If a program's stderr holds more than one report (say, from a script that ran
it twice), the last `mfb.debug.begin` block is the most recent run.

### `perf` — timings (macOS only)

Each `perf.<span>` key group times one kind of event: how many times it
happened (`count`) and the `avg`, `median`, `min`, `max` and total (`sum`) of
its durations.

| Span | What it times |
| --- | --- |
| `perf.program` | the whole run, from the program's start to the report; `count` is `1` |
| `perf.mfb_alloc` | each time the runtime reserved memory for a value |
| `perf.mfb_free` | each time the runtime took a value's memory back |

A span that never happened prints no lines. Two more keys print only when they
are not zero: `perf.mismatch` (an end with no matching start) and
`perf.overflow` (timings dropped because the timing log filled up).

On the Mac these examples ran on, every time was a whole multiple of 1000:
the timer counts microseconds there. A `0` means "under a microsecond".

### Memory, per thread (`arena.…`)

The values a thread creates live in its own **pool** of memory. Each pool the
program used gets a numbered group of keys, `arena.<n>.…`, numbered in the
order the pools were created.

| Key | Meaning |
| --- | --- |
| `arena.count` | how many pools the program used |
| `arena.registry_overflow` | pools that could not be counted because 1024 were already in use; normally `0` |
| `arena.<n>.kind` | `main` (the program's own thread), `worker` (a thread started with `thread::start`), `thread_block` (the bookkeeping for one such thread, listed just before its `worker`), or `graphics` (a canvas program's drawing thread) |
| `arena.<n>.maps`, `.mapped_bytes` | blocks of memory the pool got from the operating system, and their total size |
| `arena.<n>.unmaps`, `.unmapped_bytes` | blocks it gave back, and their total size |
| `arena.<n>.alloc_calls`, `.alloc_bytes` | requests for memory for a value, and the bytes requested (rounded up to the pool's sizes) |
| `arena.<n>.free_calls`, `.free_bytes` | values whose memory came back to the pool, and their bytes |
| `arena.<n>.live_bytes` | bytes still held by values when the report ran |
| `arena.<n>.peak_live_bytes` | the most bytes values held at any one moment |
| `arena.<n>.hit_quick_bin`, `.hit_carve`, `.hit_large_bin`, `.hit_walk`, `.grow` | how each request was served, fastest first: a ready slot of the right small size, a cut from the current block, a ready slot from the large sizes, a search of the returned slots, or a new block from the operating system. The five add up to `alloc_calls`. |
| `arena.<n>.flushes`, `.insert_free_calls` | passes that merged neighboring returned slots, and slots put back for merging |
| `arena.<n>.double_free_skips` | returns of memory that was already returned, skipped; normally `0` |
| `arena.<n>.fill_grow_calls`, `.fill_grow_bytes` | new blocks filled before use, and the bytes filled; always `0`, because new blocks arrive from the operating system already zeroed |
| `arena.<n>.fill_free_calls`, `.fill_free_bytes` | returned slots overwritten with zeros, and the bytes overwritten |
| `arena.<n>.zero_checks`, `.zero_violations` | with `MFB_ARENA_CHECK_ZERO` set to any non-empty value when the program starts: requests checked (every one), and those handed memory with a non-zero byte past its first 16 bytes; normally `0` and `0` |
| `arena.<n>.series.count` | how many memory-over-time samples follow (at most 256) |
| `arena.<n>.series.<i>.t_ns` | when sample `i` was taken, in nanoseconds since the first pool was created |
| `arena.<n>.series.<i>.mapped_bytes`, `.live_bytes` | the pool's block total and its held bytes at that moment |
| `arena.<n>.series.<i>.peak_rss_bytes` | the process's peak memory at that moment (as `process.peak_rss_bytes`) |

What the numbers tell you:

- **`live_bytes` above 0 at exit** means values were still held when the
  report ran. A small, steady number is normal. One that grows with the size
  of the input means the program is holding on to more and more as it runs.
- **`peak_live_bytes`** is the most memory your values needed at once. Compare
  it with `mapped_bytes` to see how much of what the pool took from the
  system it really used.
- **The series** shows memory over time. A sample is taken each time the pool
  needs a new block, not on a timer. If `live_bytes` keeps rising from sample
  to sample, memory is growing. If it rises and falls back, it is churning.
- **`alloc_calls`** counts how often the program made a new value that needed
  memory. A hot loop that builds strings shows up here.
- **`double_free_skips`** and **`registry_overflow`** above 0 are worth
  reporting as a bug.

The report reads these numbers while other threads may still be running, so a
thread still busy at exit can show a mix of older and newer values. A finished
thread's numbers are final.

### `process` — the whole process

| Key | Meaning |
| --- | --- |
| `process.peak_rss_bytes` | the most physical memory the process held at once, as the operating system counts it; `0` if the system would not say |

### `thread` — the thread table (programs that use `thread`)

| Key | Meaning |
| --- | --- |
| `thread.table_capacity` | room in the program's table of threads: `0` before the first `thread::start`, then 64, doubling when it fills |
| `thread.table_high` | the most threads alive at once |
| `thread.table_live` | threads not yet cleaned up when the report ran; normally `0` |

## Examples

Run the example with three arguments, `2 1000 1`: two rounds of 1000 numbers,
each round on its own worker thread, so every section of the report appears.
The program's own lines go to the terminal and the report goes to
`report.txt`.

macOS:

```sh macos
mfb build --debug examples/debug
examples/debug/build/debug.out 2 1000 1 2> report.txt
cat report.txt
```

Linux:

```sh linux
mfb build --debug examples/debug
examples/debug/build/debug-glibc.out 2 1000 1 2> report.txt
cat report.txt
```

Windows (PowerShell). `Start-Process` keeps the report's lines exactly as the
program wrote them:

```powershell windows
mfb build --debug examples\debug
Start-Process -FilePath examples\debug\build\debug.exe -ArgumentList 2, 1000, 1 `
  -Wait -NoNewWindow -RedirectStandardError report.txt
Get-Content report.txt
```

On macOS the report reads as below. The groups for pools 1 to 4 (`arena.1`
to `arena.4`: two `thread_block` and two `worker` pools) are left out here. Linux and Windows
print the same sections without the `perf` lines.

```
mfb.debug.begin 1
mfb.debug.target macos-aarch64
mfb.debug.build console
perf.mfb_alloc.count 71
perf.mfb_alloc.avg 507
perf.mfb_alloc.median 0
perf.mfb_alloc.min 0
perf.mfb_alloc.max 11000
perf.mfb_alloc.sum 36000
perf.mfb_free.count 61
perf.mfb_free.avg 131
perf.mfb_free.median 0
perf.mfb_free.min 0
perf.mfb_free.max 2000
perf.mfb_free.sum 8000
perf.program.count 1
perf.program.avg 2961000
perf.program.median 2961000
perf.program.min 2961000
perf.program.max 2961000
perf.program.sum 2961000
arena.count 5
arena.registry_overflow 0
arena.0.kind main
arena.0.maps 2
arena.0.mapped_bytes 12288
arena.0.unmaps 0
arena.0.unmapped_bytes 0
arena.0.alloc_calls 61
arena.0.alloc_bytes 11600
arena.0.free_calls 61
arena.0.free_bytes 11600
arena.0.live_bytes 0
arena.0.peak_live_bytes 4624
arena.0.hit_quick_bin 36
arena.0.hit_carve 22
arena.0.hit_large_bin 1
arena.0.hit_walk 0
arena.0.grow 2
arena.0.flushes 0
arena.0.insert_free_calls 0
arena.0.double_free_skips 0
arena.0.fill_grow_calls 0
arena.0.fill_grow_bytes 0
arena.0.fill_free_calls 42
arena.0.fill_free_bytes 10624
arena.0.zero_checks 0
arena.0.zero_violations 0
arena.0.series.count 2
arena.0.series.0.t_ns 27000
arena.0.series.0.mapped_bytes 4096
arena.0.series.0.live_bytes 176
arena.0.series.0.peak_rss_bytes 1163264
arena.0.series.1.t_ns 39000
arena.0.series.1.mapped_bytes 12288
arena.0.series.1.live_bytes 4560
arena.0.series.1.peak_rss_bytes 1196032
process.peak_rss_bytes 1523712
thread.table_capacity 64
thread.table_high 1
thread.table_live 0
mfb.debug.end 1
```

Reading it: the whole run took 2961000 ns (3 ms). The main thread's pool
took two blocks, 12 KiB in all, its values never needed more than 4624 bytes
at once, and nothing was still held at exit (`live_bytes 0`). Its 61 requests
were served from ready slots (36), the current block (22), a larger slot (1)
and the 2 `grow`s that fetched the blocks. The program ran one worker at a
time, so `thread.table_high` is 1.

To pull out a few figures:

```sh macos
mfb build --debug examples/debug
examples/debug/build/debug.out 2 1000 1 2> report.txt > /dev/null
grep -E '^(arena\.[0-9]+\.(kind|peak_live_bytes|live_bytes)|process\.peak_rss_bytes|perf\.program\.sum) ' report.txt
```

```sh linux
mfb build --debug examples/debug
examples/debug/build/debug-glibc.out 2 1000 1 2> report.txt > /dev/null
grep -E '^(arena\.[0-9]+\.(kind|peak_live_bytes|live_bytes)|process\.peak_rss_bytes) ' report.txt
```

```powershell windows
mfb build --debug examples\debug
Start-Process -FilePath examples\debug\build\debug.exe -ArgumentList 2, 1000, 1 `
  -Wait -NoNewWindow -RedirectStandardError report.txt -RedirectStandardOutput out.txt
Select-String -Path report.txt -Pattern '^(arena\.\d+\.(kind|peak_live_bytes|live_bytes)|process\.peak_rss_bytes) '
```

To read the report in `mfb dbg` (macOS, and Linux on AArch64), run the program
to its end and ask for it:

```sh macos
mfb build --debug examples/debug
printf 'show report\nrun\nshow report\nq\n' | mfb dbg examples/debug/build/debug.out -- 2 1000 1
```

The lines of the report between its third and its last three are left out
here:

```
Launched examples/debug/build/debug.out as process 7247; paused before its first instruction.
DBG> show report
No performance and memory report yet: the program writes it as it exits.
DBG> run
round 1: 75 primes
round 2: 73 primes
total: 148 primes
--[EX] Program exit (0) - type "show report" to see the performance and memory report
DBG> show report
mfb.debug.begin 1
mfb.debug.target macos-aarch64
mfb.debug.build console
...
thread.table_high 1
thread.table_live 0
mfb.debug.end 1
DBG> q
Process 7247 had already exited.
```

## See also

- `mfb man debug`
- `mfb man debug breakpoints` — the line a program's end prints in `mfb dbg`
- `mfb man debug pools` — the pools these figures describe, live in `mfb dbg`
- `mfb man debug-external profiler` — where the time goes, function by function
- `mfb man variable` — how values are copied and when they go away
- `mfb spec tooling debug-report` — the report's exact format
