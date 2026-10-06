# bug-942: debug report `perf` durations come from macOS `CLOCK_MONOTONIC`, which has microsecond resolution, so `mfb_alloc`/`mfb_free` read as 0 ns

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Correctness

Status: Open
Regression Test: `src/codegen/builtins/tests/perf.rs` (clock id assertion) plus a `--debug` run asserting that some `perf.mfb_alloc` sample is not a multiple of 1000

**Owner decision (2026-10-05):** use a nanosecond-resolution monotonic clock on macOS (`CLOCK_UPTIME_RAW` or
`mach_absolute_time` scaled by `mach_timebase_info`) for both `datetime::monotonicNanos` and the
debug-report `perf` spans. Readings resolve below a microsecond, and allocation/free medians are
non-zero. The clock must keep `monotonicNanos`'s monotonic and suspend semantics as documented.
Check whether `CLOCK_UPTIME_RAW` (pauses during sleep) or `CLOCK_MONOTONIC_RAW` (counts sleep)
matches the current contract, and keep that behavior. Linux/Windows are unchanged unless they have
the same resolution gap (measure).

The debug report's `perf` section reports, for each timed span (`program`,
`mfb_alloc`, `mfb_free`), "the number of samples and their average, median,
minimum, maximum, and total duration in nanoseconds". On macOS (the only target
that emits `perf`), the helpers read `clock_gettime` with Darwin clock id `6`,
`CLOCK_MONOTONIC`. On Darwin that clock advances in whole microseconds. Every
duration in the section is a multiple of 1000 ns. An arena allocation or free takes
well under 1 µs, so nearly every sample is `0`: `perf.mfb_alloc.median 0`,
`.min 0`, and `.sum` only counts the samples that happened to cross a microsecond
tick.

The section exists to measure allocator cost, and at this resolution it cannot.
**Correct behavior (proposed):** read a nanosecond clock, `CLOCK_UPTIME_RAW` (8) or
`CLOCK_MONOTONIC_RAW` (4), or `mach_absolute_time` scaled by the timebase, so
sub-microsecond spans are measured. If µs resolution is kept on purpose, the spec
has to state it (UNCLEAR).

The same clock stamps the arena memory series (`arena.<n>.series.<i>.t_ns`,
`src/codegen/debug/clock.rs`, "Darwin 6"). Those values are also whole
microseconds, which matters less there.

References:

- `src/docs/spec/tooling/09_debug-report.md` *Sections*, `perf` row: "… and total
  duration in nanoseconds".
- Found during hardening batch B14.

## Failing Reproduction

Any `--debug` build on macOS AArch64. For example, a program that starts two worker
threads and computes `fib(15)` (`/tmp/hardening/B14/dr/src/main.mfb`):

```
mfb build -q --debug -O0
./build/dr.out 2>&1 >/dev/null | grep -E '^perf\.'
./build/dr.out 2>&1 >/dev/null | grep -E '^perf\.|t_ns' | awk '$2 % 1000 != 0'
```

- Observed (binary /tmp/hardening/mfb-head, HEAD 28a15a153):
  ```
  perf.mfb_alloc.count 9717
  perf.mfb_alloc.avg 35
  perf.mfb_alloc.median 0
  perf.mfb_alloc.min 0
  perf.mfb_alloc.max 10000
  perf.mfb_alloc.sum 349000
  perf.mfb_free.count 9574
  perf.mfb_free.avg 31
  perf.mfb_free.median 0
  perf.mfb_free.min 0
  perf.mfb_free.max 1000
  perf.mfb_free.sum 303000
  perf.program.sum 1125000
  ```
  The second command lists only `.count` and `.avg` lines (the derived ones). Every
  measured duration and every `t_ns` is a multiple of 1000. This held on 3 of 3
  runs, and in every other `--debug` report taken in this batch.
- Expected: nanosecond-resolution samples. A median allocation of tens of ns, not
  `0`.

## Root Cause

`src/codegen/builtins/perf/perf.rs`: `const CLOCK_MONOTONIC_DARWIN: &str = "6";`
feeds the inline `clock_gettime` read behind `perf.start`/`perf.end`.
`src/codegen/debug/clock.rs:emit_debug_monotonic_nanos` uses the same id for the
series. Darwin's `CLOCK_MONOTONIC` has microsecond granularity. The raw/uptime
clocks are nanosecond.

## Goal

- On macOS, `perf.*` sample durations have sub-microsecond resolution.
- Or the spec states the granularity explicitly.

### Non-goals (must NOT change)

- The report format (keys, ordering, version 1).
- Linux/Windows (they emit no `perf` section). Linux `CLOCK_MONOTONIC` is already
  nanosecond.

## Blast Radius

- `perf.rs` `perf.start`/`perf.end`: fixed by this bug.
- `debug/clock.rs` (series `t_ns`): same clock, same granularity. Fix together.
- `datetime::monotonicNanos` (the "Matches `datetime.rs`" comment) shares it.
  Measured: 5 consecutive readings on macOS (`11764206813118000`,
  `…231000`, `…238000`, `…246000`, `…253000`) are all multiples of 1000. `mfb man
  datetime monotonicNanos` says it returns elapsed time "in whole nanoseconds" and
  states no resolution. That is out of this batch's topics. It is the same root
  cause, and changing it is a user-visible decision (UNCLEAR: change the clock, or
  document µs resolution on macOS).
