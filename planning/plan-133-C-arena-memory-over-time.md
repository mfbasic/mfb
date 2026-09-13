# plan-133-C: memory over time, per arena, in the `--debug` report

Last updated: 2026-09-12
Effort: large (3h–1d)
Depends on: plan-133-B

`planning/todo.md` § Memory § 1 item 1 asks for RSS over time per thread. The plan-130
report gives only end-of-run values (`peak_live_bytes`, `peak_rss_bytes`). This letter
adds a bounded time series per arena — one arena per thread — sampled at the moments
memory grows, with a timestamp, the arena's mapped and live bytes, and the process's
peak RSS so far.

Behavioral outcome: a `--debug` report contains, for every registered arena,
`arena.<n>.series.count <k>` (k ≤ 256) and `k` samples in time order; for a program that
grows memory in three spaced bursts the samples show three separated rises, on all five
targets.

References: plan-133-A § Prerequisites; `src/codegen/debug/arena.rs` (registry, slot
layout, `ARENA_COUNTERS`); `src/codegen/debug/process.rs` (`emit_peak_rss_bytes` via the
platform); `src/codegen/builtins/datetime/func_monotonic_nanos.rs` (per-platform
monotonic clock); `src/codegen/builtins/net/gen_ping.rs` (`emit_monotonic_nanos`, the
`platform.clock_monotonic()` form); `src/docs/spec/tooling/09_debug-report.md`.

## Prerequisites

See plan-133-A § Prerequisites. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-133-B complete | `ls planning/completed/plan-133-B-*` → one match | NOT MET |
| Boxes 2223, 2227, 2229, 2230 reachable | `for p in 2223 2227 2229 2230; do ssh -o ConnectTimeout=8 -p $p test@127.0.0.1 true && echo $p ok; done` → four `ok` | UNMEASURED |

## 1. Goal

- Per arena: `series.count`, and per sample `i`: `series.<i>.t_ns` (nanoseconds since the
  main arena registered), `series.<i>.mapped_bytes`, `series.<i>.live_bytes`,
  `series.<i>.peak_rss_bytes`.
- Samples are strictly increasing in `t_ns` and in `mapped_bytes`.
- The first sample is the arena's first grow and the last is its most recent grow.
- A non-`--debug` build is unchanged.

### Non-goals

- No sampler thread and no timer.
- No current (non-peak) RSS: no portable cheap call gives it (§2).
- No change to the existing 18 counters or their meaning.

## 2. Current State

- Registry slots: `ARENA_DEBUG_SLOTS = 1024`, slot fields at `COUNTER_*` offsets 16…152
  (`src/codegen/debug/arena.rs`). The grow path already counts `maps`/`mapped_bytes`/`grow`
  in `lower_arena_alloc` (`src/codegen/memory/arena/arena.rs`).
- Monotonic clock per platform:
  - macOS and Linux: libc `clock_gettime(platform.clock_monotonic(), &ts)`
    (`gen_ping.rs::emit_monotonic_nanos`; `clock_monotonic()` is Darwin 6 / Linux 1).
    Linux entries already import `clock_gettime` (`linux_common/plan.rs`, the entry
    import list).
  - Windows: `clock_monotonic()` is `unreachable!` (`win_x86_64/code.rs`);
    `datetime::monotonicNanos` uses `QueryPerformanceCounter` +
    `QueryPerformanceFrequency` with an overflow-safe fold (`func_monotonic_nanos.rs`).
- perf's `emit_read_monotonic_nanos` (`src/codegen/builtins/perf/perf.rs`) hard-codes the
  Darwin clock id and is macOS-only.
- Peak RSS: `CodegenPlatform::emit_peak_rss_bytes` exists on all five targets (plan-130-D).
- Current RSS: Linux exposes it only through `/proc/self/statm` (file I/O);
  `getrusage` reports only the peak.

### Measured populations

| What | Value | Command |
|---|---|---|
| Browser worker grows per `Main_Page` load | 192,273 | `arena.1.grow` in `planning/todo.md` § 2 |
| Browser main-arena grows per load | 15,236 | `arena.0.grow`, same |
| Existing per-slot counter words | 18 | `grep -c '^    ("' src/codegen/debug/arena.rs` over `ARENA_COUNTERS` (18 entries) |

### Verified properties

- UNVERIFIED — that the libc `clock_gettime` call is arena-free and usable from inside
  `lower_arena_alloc`'s grow path without disturbing the vregs live there (it is an external
  call; the grow path already makes a `bl` to `_mfb_arena_fill_random`, so the spill model
  already covers a call there). Phase 1's tests prove it.
- UNVERIFIED — the cost of one clock read plus one `getrusage` per *kept* sample. With the
  decimation below, kept samples per arena are at most 256 plus one per doubling; Phase 1
  measures the browser `Main_Page` wall time with and without the series on 2223.

## 3. Design Overview

**Record on grow, keep a bounded log-spaced series.**

- Each slot gains a series block: `{count, stride, until, samples[256] × 32 B}`
  (8,216 B per slot). `ARENA_DEBUG_SLOTS` × 8,216 B is added to the debug-only registry
  mapping, which normal builds never create.
- On every grow (after the existing `grow` increment): if `grow_count ≥ until`, take a
  sample — `t_ns` (clock), `mapped_bytes`, `live_bytes` (already in the slot), and
  `peak_rss_bytes` (`emit_peak_rss_bytes`) — then `until += stride`.
- When `count` reaches 256: keep every other sample (in place, 128 left), double `stride`.
  This keeps samples evenly spaced in grow count across the whole run with a fixed bound.
- `t0`: the registry header stores the clock reading taken when the main arena registers.
- New `emit_debug_monotonic_nanos(dst)` in `src/codegen/debug/clock.rs`:
  `clock_gettime(platform.clock_monotonic())` off Windows; QPC/QPF with
  `func_monotonic_nanos.rs`'s fold on Windows. Imports are added only in debug builds.
- Report: after the 18 counters of each slot, `series.count` then the samples.

**Risk:** the allocator grow path again (register lifetimes around two new external calls
on five backends), and the in-place compaction (an off-by-one corrupts the series, not the
program). Both are covered by exact tests.

**Rejected:**
- *A sampler thread*: adds a thread and a timer per target, perturbs the program it
  measures, and still cannot read per-thread memory.
- *Sampling on every alloc*: 109 M events per browser load.
- *A file sink* (`MFB_DEBUG_SERIES=<path>`): useful for very long runs but a second output
  contract; the bounded series fits the existing report.

## Phases

### Phase 1 — clock, series block, sampling on macOS

- [ ] `src/codegen/debug/clock.rs::emit_debug_monotonic_nanos` (both arms).
- [ ] Series block in the registry slot; sampling and compaction in the grow path, debug only.
- [ ] Report lines for the series.
- [ ] `tests/runtime/rt_debug_arena.rs`:
  - `the_series_rises_in_three_bursts`: allocate and retain 8 MiB, `os::sleep(200)`, three
    times; assert `count ≥ 3`, `t_ns` strictly increasing, and at least two gaps
    ≥ 150,000,000 ns.
  - `the_series_stays_bounded_and_ordered`: 50,000 grows; assert `count ≤ 256`,
    `mapped_bytes` strictly increasing, first `t_ns` ≤ last.
- [ ] Measure the series' cost: browser `Main_Page` load wall time, `--debug` with vs
      without the series (a throwaway flag in a worktree), 3 runs each, on 2223; record
      the medians in Corrections (~10 min, because it is the only workload with 192 k grows).

Acceptance: `cargo test --release --test rt_debug_arena series` → 2 passed (~3 min).
Commit: —

### Phase 2 — the other four targets

- [ ] linux-aarch64 on 2223 (native): run `the_series_rises_in_three_bursts`'s program,
      cross-built; record the series lines.
- [ ] linux-x86_64 on 2227 (musl; emulated x86 — needed because x86_64 is its own
      backend): same.
- [ ] linux-riscv64 on 2229: same.
- [ ] windows-x86_64 on 2230 (QPC arm): same, via a CRLF `.cmd` wrapper.

Acceptance: each box's recorded series shows three rises separated by ≥ 150 ms, in this
file's Corrections (one ~1 min run per box).
Commit: —

### Phase 3 — docs, todo, and the family's full suite

- [ ] `src/docs/spec/tooling/09_debug-report.md`: the series keys, the decimation rule, the
      256 bound.
- [ ] `planning/todo.md` § Memory § 1 item 1: done, with the report keys and the
      browser's worker series from one 2223 run pasted as the first data.
- [ ] Full suite once for plan-133:
      `cargo test --no-fail-fast -- --skip artifact_gate_all > /tmp/p133.log 2>&1; echo EXIT=$?`
      (~25 min, the only run that sees every rt test after three letters of debug-path
      changes), `scripts/artifact-gate.sh target/release/mfb all` (~20 min, proves normal
      builds are unchanged), `scripts/test-accept.sh target/release/mfb /tmp/p133-accept`.

Acceptance: `EXIT=0`; gate `0 diff(s)`; acceptance passes except the recorded baseline.
Commit: —

## Validation Plan

- Tests: the two `rt_debug_arena.rs` series cases; the full suite once in Phase 3.
- Runtime proof: Phase 2's four box runs, plus the macOS host test.
- Doc sync: `09_debug-report.md`, `planning/todo.md` § 1 item 1.

## Open Decisions

- **Process RSS per sample** — recommended: peak-so-far (`getrusage` /
  `K32GetProcessMemoryInfo` peak), which exists on every target. Alternative: current RSS
  (`/proc/self/statm` on Linux, `mach_task_basic_info` on macOS, `WorkingSetSize` on
  Windows) — three new mechanisms and file I/O from the grow path.
- **Series capacity** — recommended 256 samples per arena; alternative: a
  `--debug`-time knob, not worth a flag yet.

## Corrections

## Summary

A bounded, grow-triggered series answers "memory over time per thread" without a sampler.
The risk is the grow path's register lifetimes around two new calls on five backends,
covered by exact-count tests and one run per box.
