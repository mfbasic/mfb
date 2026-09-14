# plan-133-C: memory over time, per arena, in the `--debug` report

Last updated: 2026-09-13
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
| plan-133-B complete | `ls planning/completed/plan-133-B-*` → one match | MET (2026-09-13: `planning/completed/plan-133-B-entropy-fill-cost.md`) |
| Boxes 2223, 2227, 2229, 2230 reachable | `for p in 2223 2227 2229; do ssh -o ConnectTimeout=8 -o BatchMode=yes -p $p test@127.0.0.1 true && echo $p ok; done; ssh -o ConnectTimeout=8 -o BatchMode=yes -p 2230 test@127.0.0.1 ver && echo 2230 ok` → four `ok` (2230 is Windows: `true` does not exist there, `ver` does) | MET (2026-09-13, re-checked before C: 2223, 2227, 2229 `true` ok; 2230 `ver` ok) |

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
| Browser worker grows per `Main_Page` load | 189,654 | `arena.1.grow`, 2223, main `db8e34157`, 2026-09-13 (plan-133-A § Measured populations; 192,273 before plan-134) |
| Browser main-arena grows per load | 5,125 | `arena.0.grow`, same run (15,236 before plan-134) |
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

- [x] `src/codegen/debug/clock.rs::emit_debug_monotonic_nanos` (both arms). — The libc arm
      (`clock_gettime(platform.clock_monotonic())` into a vreg) is exercised by every macOS
      probe: `t_ns` strictly increases, e.g. `/tmp/plan-133-c/recgen4k` 9,000 → 831,090,000 ns.
      The Windows arm (QPC/QPF with the overflow-safe fold) compiles
      (`cargo check --all-targets` → `EXIT=0`, no warnings). Its runtime proof is Phase 2's
      2230 run.
- [x] Series block in the registry slot; sampling and compaction in the grow path, debug only.
      — Implemented as the `_mfb_debug_arena_sample` helper, which the grow path reaches with
      an internal `bl` after the plan-133-B fill counters; the relocation is added only when
      `debug_arena` (design and slot layout in Corrections). `t0` is stored at the first
      registration. The `SLOT_SIZE == 8448` compile-time assert holds. Probes: 14 and 20 grows
      kept every sample (`grow20k`, `grow200k`); 5,003 grows halved to 159 samples (`big5k`);
      2,000 and 4,000 grows to 252 (`recgen2k`, `recgen4k`). In every probe the last sample's
      `mapped_bytes` equals `arena.0.mapped_bytes`.
      `cargo test --bin mfb codegen::debug` → `4 passed`, after the fixture fix in Corrections.
- [x] Report lines for the series. — After each arena's counters: `arena.<n>.series.count <k>`
      (kept + pending), then `arena.<n>.series.<i>.t_ns`, `.mapped_bytes`, `.live_bytes`,
      `.peak_rss_bytes`, oldest first. They appear in every probe report above.
- [x] `tests/runtime/rt_debug_arena.rs`: — `cargo test --release --test rt_debug_arena series`
      → `the_series_rises_in_three_bursts ... ok`, `the_series_stays_bounded_and_ordered ...
      ok`, `2 passed; 0 failed` (82.16 s). The bounded test runs 4,000 grows, not 50,000,
      with a grows floor and a last-sample-is-latest-grow assertion added (Corrections). The
      bursts program binds each read to a `LET` (bug-626).
  - `the_series_rises_in_three_bursts`: allocate and retain 8 MiB, `os::sleep(200)`, three
    times; assert `count ≥ 3`, `t_ns` strictly increasing, and at least two gaps
    ≥ 150,000,000 ns.
  - `the_series_stays_bounded_and_ordered`: 50,000 grows; assert `count ≤ 256`,
    `mapped_bytes` strictly increasing, first `t_ns` ≤ last.
- [x] Measure the series' cost: browser `Main_Page` load wall time, `--debug` with vs
      without the series (a throwaway flag in a worktree), 3 runs each, on 2223; record
      the medians in Corrections (~10 min, because it is the only workload with 192 k grows).
      `~/p133c/ab-series-2223.sh` → series on: median `load_ms=8022` (10615, 7883, 8022);
      series off: median `load_ms=7914` (7983, 7854, 7914); +1.4 %.

Acceptance: `cargo test --release --test rt_debug_arena series` → 2 passed (~3 min).
Commit: 2375c9fef, 6c0926c14 (task 5's record)

### Phase 2 — the other four targets

- [x] linux-aarch64 on 2223 (native): run `the_series_rises_in_three_bursts`'s program,
      cross-built; record the series lines. `p133c/bursts-glibc.out` → stdout `3`, exit 0,
      `series.count 5`, rises at 18.0 / 247.6 / 515.2 ms (Corrections).
- [x] linux-x86_64 on 2227 (musl; emulated x86 — needed because x86_64 is its own
      backend): same. `/tmp/p133c-bursts-musl.out` → `3`, `series.count 5`, rises at
      281.2 / 835.4 / 1,573.8 ms.
- [x] linux-riscv64 on 2229: same. The musl binary (the glibc one: `sh: … not found`) →
      `3`, exit 0, `series.count 5`, rises at 65.1 / 332.8 / 646.2 ms.
- [x] windows-x86_64 on 2230 (QPC arm): same, via a CRLF `.cmd` wrapper. `run-bursts.cmd`
      → `exit=0`, stdout `3`, `series.count 8`, 8 MiB rises at 12.8 / 842.3 / 2,787.2 ms.

Acceptance: each box's recorded series shows three rises separated by ≥ 150 ms, in this
file's Corrections (one ~1 min run per box).
Commit: e12c1b45f

### Phase 3 — docs, todo, and the family's full suite

- [x] `src/docs/spec/tooling/09_debug-report.md`: the series keys, the decimation rule, the
      256 bound. Two table rows plus a series paragraph citing
      `[[src/codegen/debug/arena.rs:lower_sample]]`. `cargo test --release --bin mfb
      docs::spec` → `8 passed` (incl. `spec_citations_resolve`).
- [x] `planning/todo.md` § Memory § 1 item 1: done, with the report keys and the
      browser's worker series from one 2223 run pasted as the first data. It has the
      series-on A/B run's `arena.1` series: 188 samples, 8 rows selected, last `mapped_bytes`
      818.9 MiB equal to `arena.1.mapped_bytes 858677248`, and `unmaps 0`.
- [x] Full suite once for plan-133 (on the tree merged with main `caf191edd`): `EXIT=0`,
      180 test binaries, 5,630 passed, 0 failed, 11 ignored; gate `artifact-gate [all]:
      1441 tests, 1607 build(s), 2021 golden(s) checked, 0 diff(s)`; `acceptance tests
      passed (1464 test(s) ran)`, `ACCEPT=0`, with no baseline failures.
      `cargo fmt --all --check` in both workspaces showed no diffs.
      `cargo test --no-fail-fast -- --skip artifact_gate_all > /tmp/p133.log 2>&1; echo EXIT=$?`
      (~25 min, the only run that sees every rt test after three letters of debug-path
      changes), `scripts/artifact-gate.sh target/release/mfb all` (~20 min, proves normal
      builds are unchanged), `scripts/test-accept.sh target/release/mfb /tmp/p133-accept`.

Acceptance: `EXIT=0`; gate `0 diff(s)`; acceptance passes except the recorded baseline.
Commit: 6c0926c14 (the suite record's commit is named in the archive commit)

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

- **2026-09-13 — the box-reachability command was wrong for Windows box 2230.** It ran
  `ssh … -p 2230 test@127.0.0.1 true`, and `true` is not a Windows command. The row read
  NOT MET, but ssh had connected and authenticated. The failure was the remote shell:
  stderr was `'true' is not recognized as an internal or external command, operable program
  or batch file.` Re-measured with `ssh -o ConnectTimeout=8 -o BatchMode=yes -p 2230
  test@127.0.0.1 ver`, which printed `Microsoft Windows [Version 10.0.26100.9445]` and exited 0.
  The Prerequisites row now runs `ver` on 2230 and `true` on the three Linux boxes. Status:
  MET. It is still to be re-checked before C starts.
- **2026-09-13 — § 2: there is no `CodegenPlatform::emit_peak_rss_bytes`.**
  `grep -rn "fn emit_peak_rss_bytes" src/codegen src/target src/os` finds nothing. The peak-RSS
  read is inline in `src/codegen/debug/process.rs::lower_report`, the report-only function.
  On macOS and Linux it calls `getrusage(RUSAGE_SELF, sp + RESULT_BUFFER_OFFSET)` and reads
  `ru_maxrss`, shifted left by 10 on Linux (KiB). On Windows it calls
  `K32GetProcessMemoryInfo`. Its buffer is a frame local from
  `finalize_vreg_body_with_locals(…, RESULT_BUFFER_OFFSET + RUSAGE_SIZE.max(PMC_SIZE))`, and
  the OS imports come from `NativePlanPlatform::peak_rss_imports`. Consequence for Phase 1:
  the per-sample RSS read must be factored out of `lower_report` into an emitter the grow
  path can call, not called through an existing seam.
- **2026-09-13 — § 2: the Linux `clock_gettime` entry import is confirmed.**
  `linux_common/plan.rs:115` pushes `libc_import("clock_gettime", "_main")`.
- **2026-09-13 — § 3: the grow path has no frame buffer for the two new calls.**
  `lower_arena_alloc` ends in `finalize_vreg_helper`, which builds a frame only for spilled
  vregs; `arena.rs` uses no `stack_pointer()` slot. `clock_gettime` needs a 16-byte
  `timespec` and `getrusage` a `struct rusage`, so the allocator must reserve a local area
  with `finalize_vreg_body_with_locals` (it rounds `local_size` up to 16 and lays spill slots
  after it). The alternative is to keep both buffers in the debug registry region, off the
  stack.
- **2026-09-13 — § 1 and § 3 disagree about the last sample.** § 1 requires "the last is its
  most recent grow". § 3 samples a grow only when `grow_count ≥ until`, so once the stride
  exceeds 1, the most recent grow is usually not kept, and the last sample is the most recent
  *kept* grow. Resolution, keeping § 1's goal: every grow writes a provisional sample (clock,
  `mapped_bytes`, `live_bytes`, peak RSS) into the entry after the kept ones. `count` advances
  only when the decimation rule keeps that entry, and the report prints the provisional entry
  after the kept ones when the latest grow was not kept. The cost is one clock read and one
  `getrusage` per grow (189,654 grows in a browser `Main_Page` worker) instead of per kept
  sample. Phase 1's existing task, which measures the series' wall-time cost on 2223, decides
  whether that is affordable. If it is not, the goal is changed here, with the measurement.
- **2026-09-13 — the sample cannot go right after the `grow` counter.** In the grow path the
  mapped block's address stays in the physical return register until the header is written
  (`abi::store_u64(…, abi::return_register(), …)`), and an external call there would destroy
  it. The sample is taken after `ubase` is derived, beside the plan-133-B fill counters.
  `ubase`, `usable`, `size` and `eff_align` are vregs, which the allocator already spills
  across the fill call.
- **2026-09-13 — Phase 1 design: sample in a debug helper, not in the allocator's frame.**
  `lower_arena_alloc` takes no `platform_imports`, so it cannot make external calls without
  new plumbing, and its frame has no local buffer. The grow path instead calls a debug-only
  helper, `_mfb_debug_arena_sample(slot)`, with an internal `bl`, the same way it calls
  `_mfb_arena_fill_random`; the relocation is added only when `debug_arena`. The helper owns
  its `timespec`/QPC words and its `rusage` or `PROCESS_MEMORY_COUNTERS` buffer through
  `finalize_vreg_body_with_locals`, and makes the external calls, the way
  `_mfb_debug_arena_register` already calls the platform mutex. The allocator's frame and
  normal-build code do not change.
  - `datetime::gen_shared::emit_libc_clock_nanos` cannot be reused: it writes the physical
    `RESULT_VALUE_REGISTER`, and the vreg finalizer rejects physical registers (plan-34-D).
    The new `src/codegen/debug/clock.rs::emit_debug_monotonic_nanos` writes a vreg.
  - Slot layout: the 22 counters end at 192; then `series.count` (192), `stride` (200),
    `until` (208), `pending` (216), and 257 × 32-byte entries `{t_ns, mapped_bytes,
    live_bytes, peak_rss_bytes}` from 224, which is 256 kept entries plus one provisional.
    `SLOT_SIZE` = 8,448. The region header grows to `{count, overflow, t0}` (24 B).
  - Imports: macOS (`macos_aarch64/plan.rs`) and Linux (`linux_common/plan.rs:115`) import
    `clock_gettime` for the entry unconditionally. Every `--debug` build already imports the
    peak-RSS calls through `ProcessFeature::os_imports`. Only Windows lacks
    `QueryPerformanceCounter`/`Frequency` for the entry, so `NativePlanPlatform` gains
    `debug_clock_imports` with an empty default that Windows overrides.
- **2026-09-13 — Phase 1: "50,000 grows" cannot be produced cheaply, and the obvious ways to
  try are traps.** The plan took the number from the browser worker's 189,654 grows. Measured
  with `--debug` probes on the macOS host:
  - `lower_arena_alloc` maps one 4,096 B default block when `size + align + 32` fits,
    otherwise exactly that request rounded up to 4 KiB pages. It never doubles.
  - A program that retains values in a growing list gets few grows: 20,000 strings made 14
    (`/tmp/plan-133-c/grow20k`) and 200,000 made 20 (`grow200k`). The list reallocates in
    doubling steps, each mapped exactly (4,096 → 8,192 → 16,384 B …), and the small retained
    values refill the freed space. A list built by `append` of `""` and then filled with
    `collections::set` behaved the same: 18 and 20 grows at 50,000 and 100,000 entries.
  - Appending a builtin call's result directly does give about one grow per append, but only
    through bug-626's copy of the whole list on every append. At N=5,000,
    `append(keep, fs::readText(…))` allocated 63,063,560,064 B, mapped 63,023,300,608 B and
    peaked at 63 GB RSS (44.5 s). A 50,000 run was killed before it exhausted memory. A test
    must not rely on that bug, or its coverage vanishes when the bug is fixed.
  - With 5,003 grows the series was kept to 159 samples, and the last sample's `mapped_bytes`
    (63,523,377,152) equalled the arena's, so halving and the latest-grow entry both work.
  - A generator that works: recursion. Each level binds `strings::repeat("x", 4200 + (n MOD
    97))` and recurses before returning, so every level keeps a string slightly larger than a
    block alive, and each string needs a fresh map. Measured
    (`/tmp/plan-133-c/recgen{2k,4k}`): depth 2,000 gave `grow 2000`, `mapped_bytes 16384000`
    (8,192 B per level), `alloc_bytes 8528128`, 0.53 s, 34.7 MB RSS; depth 4,000 gave
    `grow 4000`, `mapped_bytes 32768000`, `alloc_bytes 17057536`, 1.00 s, 68.3 MB RSS, which
    is linear. Both reported `series.count 252`, with the last sample's `mapped_bytes` equal to
    the arena's.
  - Two other list-filling shapes were also quadratic in time, root cause not found (filed as
    `bugs/bug-627-set-loop-over-a-string-list-is-quadratic-in-time.md`; the append copy is
    `bugs/bug-626-append-of-a-builtin-call-result-copies-the-whole-list.md`):
    building a list with `append` of `""` then `collections::set` of bound strings took 2.6 s
    at 50,000 and 9.1 s at 100,000 entries; `strings::split` then the same `set` loop took
    9.7 s at 100,000 and 35.5 s at 200,000. `try_inplace_set_assign` has no exclusion for a
    `List OF String` that would explain it.
  - **The `codegen::debug` unit-test fixture gained `_clock_gettime`.** After the series landed,
    `a_debug_module_gets_the_report_helper_and_each_section` and
    `debug_helpers_reference_no_arena_symbol` both panicked at `src/codegen/debug/tests.rs:47`
    with `called Result::unwrap() on an Err value: "runtime helper requires _clock_gettime
    import"`. The tests' `imports()` is a hand-built list of the macOS imports a real plan
    gives the debug helpers. plan-130-C added the mutex pair to it for the same reason: a
    helper needed a new import.
    1. When and why written: plan-130-C/D, to mirror a real plan's imports.
    2. What it protects: that the debug helpers lower against that set and reference no
       allocator symbol.
    3. Who depends on it: only this file.
    4. Why it was wrong: a real macOS plan imports `_clock_gettime` for every program entry
       (`macos_aarch64/plan.rs` `entry_imports`, for the arena-fill seed), and the helpers now
       call it.

    The fixture now lists it, and no assertion changed. `cargo test --bin mfb codegen::debug`
    → `4 passed`.
  - **Resolution:** `the_series_stays_bounded_and_ordered` uses the recursion generator at
    depth 4,000, not 50,000 grows. That depth goes through several halvings in about a
    second. 50,000 levels would map about 400 MB, and the stack cost of that depth has not
    been measured. The check is not weaker: the test also asserts that at least 4,000 grows
    happened (so the halving really ran) and that the last sample's `mapped_bytes` equals the
    arena's (§ 1's latest-grow goal).
- **2026-09-13 — Phase 1 task 5: the series costs 1.4 % of a browser load, so sampling every
  grow stays.** Two `--debug` linux-aarch64 browsers were built from `2375c9fef` with
  `/tmp/plan-133-b/build-browser.sh`:
  - **Series on:** the worktree's compiler.
  - **Series off:** a throwaway worktree (`/tmp/p133-c-series`) patched by
    `/tmp/plan-133-c/patch_series_off.py`, which removes the grow path's
    `_mfb_debug_arena_sample` call and its relocation.

  The patch was never committed: `git log -S "plan-133-C THROWAWAY" 2375c9fef..HEAD` is empty,
  and the worktree was removed. The plan asked for "a throwaway flag"; a patch is the same
  measurement without adding a flag.

  On 2223, `~/p133c/ab-series-2223.sh` ran 3 `Main_Page` loads per browser back to back with
  `tools/browser-load-timer` (load 0.13 → 0.42):
  - Series on: 10,615 / 7,883 / 8,022 ms, median **8,022**. Run 1 is the cold first load.
  - Series off: 7,983 / 7,854 / 7,914 ms, median **7,914**.

  Both reports show the same workload. On: `arena.1.kind worker`, `grow 189654`,
  `series.count 188`, `arena.0.grow 5125`, `series.count 163`. Off: the same grow counts with
  `series.count 0`. So the clock read and `getrusage` on each of 194,779 grows cost about
  108 ms, or +1.4 %. That is small enough to keep the § 1 goal (the last sample is the most
  recent grow), which Corrections above had made conditional on this measurement.
- **2026-09-13 — Phase 3: main was merged before the full suite, so one run serves Phase 3
  and the finish step.** `git rev-list --count HEAD..main` was 1: `caf191edd` (plan-136
  planning docs and a bug doc, no source). `git merge --no-edit main` was clean, and the
  suite, gate and acceptance ran on the merged tree once, instead of before and again after
  the merge.
- **2026-09-13 — finish: main moved again during that run, so it was merged and everything
  re-ran.** After `8d8894477`, `git rev-list --count HEAD..main` was 8 (`af9b1a591` …
  `b22a82f26`). Those commits change `.mfb` source in `packages/{json_schema,jwt,mustache,yaml}`,
  so the first green run no longer covered the tree being landed. `git merge --no-edit main`
  was clean. The release build, full suite, artifact gate and test-accept then ran again on the
  merged tree, one after another:
  - Build: `BUILD=0`.
  - Suite: `EXIT=0`, 180 binaries, 5,630 passed, 0 failed, 11 ignored.
  - Gate: `artifact-gate [all]: 1441 tests, 1607 build(s), 2021 golden(s) checked, 0 diff(s)`.
  - Acceptance: `acceptance tests passed (1464 test(s) ran)`, `ACCEPT=0`.
  - Formatting: `cargo fmt --all --check` showed 0 diffs in both workspaces.
- **2026-09-13 — Phase 2: the four box runs.** The program is
  `/tmp/plan-133-c/bursts/src/main.mfb`: three times, `LET burst = fs::readBytes` of an 8 MiB
  file, append it to a retained list, `os::sleep(200)`; print the list's length. The Windows
  copy (`bursts-win`) reads `C:/Users/test/p133c/burst.bin`. Built with
  `target/release/mfb build --debug --target <t>` at `2375c9fef`. Every run printed `3` and
  exited 0. The series (`t_ns` ms : `mapped_bytes`):
  - **2223** linux-aarch64 glibc: 0.025 : 4,096 · 0.048 : 8,396,800 · 18.0 : 16,789,504 ·
    247.6 : 33,570,816 · 515.2 : 58,740,736; `grow 5`, `series.count 5`.
  - **2227** linux-x86_64 musl (emulated): 2.8 : 4,096 · 5.9 : 8,396,800 · 281.2 : 16,789,504
    · 835.4 : 33,570,816 · 1,573.8 : 58,740,736; `grow 5`.
  - **2229** linux-riscv64 musl: 2.2 : 4,096 · 3.7 : 8,396,800 · 65.1 : 16,789,504 · 332.8 :
    33,570,816 · 646.2 : 58,740,736; `grow 5`. **The plan's box did not run the glibc binary:**
    `/tmp/p133c-bursts-glibc.out` gave `sh: …: not found`, exit 127, so the box has no glibc
    loader. The musl binary is the one recorded.
  - **2230** windows-x86_64 (QPC): 8.4 : 4,096 · 9.9 : 73,728 · 12.8 : 8,466,432 · 183.3 :
    16,859,136 · 722.6 : 16,928,768 · 842.3 : 33,710,080 · 2,658.0 : 33,779,712 · 2,787.2 :
    58,949,632; `grow 8`, `series.count 8`. The extra grows are small blocks around each burst.
    `live_bytes` falls between them (8,519,952 at 722.6 ms), so these are allocations made
    while the list was copied.

  On each box, the three 8 MiB bursts raise `mapped_bytes` at least 150 ms apart:
  - 2223: 18.0 → 247.6 → 515.2 ms.
  - 2227: 281.2 → 835.4 → 1,573.8 ms.
  - 2229: 65.1 → 332.8 → 646.2 ms.
  - 2230: 12.8 → 842.3 → 2,787.2 ms.

  The first sample's `mapped_bytes` of 4,096 is the entry's first block, and `t_ns` rises
  strictly on every box.

## Summary

A bounded, grow-triggered series answers "memory over time per thread" without a sampler.
The risk is the grow path's register lifetimes around two new calls on five backends,
covered by exact-count tests and one run per box.
