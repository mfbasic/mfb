# plan-133-B: what the entropy fill costs

Last updated: 2026-09-13
Effort: medium (1h–2h)
Depends on: plan-133-A

Every freshly mapped arena block and every freed chunk is filled with pseudo-random
bytes (`src/docs/spec/memory/04_arenas.md` § Entropy Fill; always on, in every build).
`planning/todo.md` § Memory § 1 item 2 asks what that costs, so every later A/B knows
the fill's share of its numbers. This letter adds fill counters to the `--debug` report
and records one throwaway A/B of wall time with the fill off. The fill itself does not
change.

Behavioral outcome: every `--debug` report carries per-arena `fill_grow_calls`,
`fill_grow_bytes`, `fill_free_calls`, `fill_free_bytes`; and `planning/todo.md` § 1
item 2 records the benchmark suite's geomean and per-row ratios for fill on vs off, on
macOS and on box 2223.

References: plan-133-A § Prerequisites; `src/codegen/memory/arena/arena.rs`
(`lower_arena_alloc` grow path, `lower_arena_free` scrub);
`src/codegen/builtins/math/gen_rng_pcg64.rs` (`lower_arena_fill_random`);
`src/codegen/debug/arena.rs` (`ARENA_COUNTERS`); plan-130-C § Corrections "Phase 1 —
lookup-cost measurement" (the A/B method this letter reuses).

## Prerequisites

See plan-133-A § Prerequisites. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-133-A complete | `ls planning/completed/plan-133-A-*` → one match | MET (2026-09-13: `planning/completed/plan-133-A-browser-memory-diagnosis-and-soak-test.md`) |

## 1. Goal

- Four new per-arena counters in `ARENA_COUNTERS`, reported after the existing 18.
- For a program with known allocations, `fill_grow_bytes` equals the sum of the fresh
  blocks' usable bytes, and `fill_free_bytes` equals Σ(size − 16) over frees of chunks
  larger than 16 B.
- A recorded A/B: `benchmark/mfb` suite, fill on vs off, geomean and the arena-heavy
  rows, on macOS and on 2223.

### Non-goals

- The fill is not removed, weakened, or made optional in any landed build. The fill-off
  build is a throwaway patch that is never committed.
- No change to what is filled or when.

## 2. Current State

- Fill call sites, both `bl _mfb_arena_fill_random`:
  - `lower_arena_alloc` grow path, right after the new block's header is written:
    `c_arg(0) = ubase`, `c_arg(1) = usable`.
  - `lower_arena_free` at `arena_free_scrub`: skipped when `size == 16`, otherwise
    `c_arg(0) = ptr + 16`, `c_arg(1) = size − 16`.

  Measured: `git grep -nw "ARENA_FILL_RANDOM_SYMBOL" -- src ':!src/docs'` →
  `arena.rs` two `branch_link`s plus their relocation lines, and
  `gen_rng_pcg64.rs` (the definition).
- The helper: `lower_arena_fill_random` (`gen_rng_pcg64.rs`) streams a per-arena PCG64
  over `[ptr, ptr+len)`.
- Counters: 18 per-slot words today (`ARENA_COUNTERS`, `src/codegen/debug/arena.rs`),
  incremented through the plan-130-C counting emitter, only in `--debug` builds.
- A/B precedent: plan-130-C measured its registry lookup by building the whole
  `benchmark/mfb` suite (no row filter) with and without a throwaway patch, running
  `--run 3` back to back, and comparing per-`section.row` medians (485 rows); on 2223
  and on macOS.

### Measured populations

| What | Value | Command |
|---|---|---|
| Fill call sites in the allocator | 2 | `git grep -n "branch_link(ARENA_FILL_RANDOM_SYMBOL)" -- src/codegen/memory/arena/arena.rs` → 2 (re-check at Phase 1: plan-134 touched the allocator's neighbours) |
| Browser worker frees per `Main_Page` load | 70,952,128 (5,135,888,784 B freed) | `arena.1.free_calls` / `free_bytes`, 2223, main `db8e34157`, 2026-09-13 (plan-133-A § Measured populations) |
| Browser worker grows per `Main_Page` load | 189,654 | `arena.1.grow`, same run |
| Browser worker bytes requested per `Main_Page` load | 5,887,193,872 (×1.47 the 4,006,566,864 before plan-134) | `arena.1.alloc_bytes`, same run vs 2026-09-12 |

plan-134's copies raised the bytes the worker allocates and frees by 41–47% per page, so the
free-path scrub — which fills `size − 16` bytes of every freed chunk over 16 B — has more to
fill than when this plan was written. The A/B measures the current compiler.

### Verified properties

- UNVERIFIED — what share of wall time the fill is. That is this letter's measurement.
- UNVERIFIED — that making `_mfb_arena_fill_random` return immediately leaves every other
  path unchanged. It does: the helper has no outputs, and the allocator never reads filled
  bytes as metadata (`arena_free` writes the `FreeNode` words before scrubbing past them —
  `04_arenas.md` § `arena_free`). Phase 2's patch is exactly that one-line early return.

## 3. Design Overview

- **Counters:** four new `COUNTER_FILL_*` offsets appended to the slot and to
  `ARENA_COUNTERS`. They are incremented by the existing counting emitter immediately
  before each of the two `bl`s, debug only. Bytes are the `c_arg(1)` value the call
  receives.
- **A/B:** a worktree off the current base with a throwaway patch making
  `lower_arena_fill_random` return at entry. Build the `benchmark/mfb` suite normally,
  with and without the patch, `--run 3`, back to back. Compare per-row medians the way
  plan-130-C did. Do it once on macOS (16 KiB pages) and once on 2223 (native aarch64,
  4 KiB pages), since the fill's cost scales with block and page size.
- The browser `Main_Page` load, wall time fill on vs off on 2223, 3 runs each, as the
  app-sized data point.

**Risk:** low. The counters sit beside calls that already exist, and the A/B code is
never landed.

**Rejected:** timing each fill call with the clock. That adds a clock read per free
(67.9 M in one browser load), which would dominate the thing it measures.

## Phases

### Phase 1 — fill counters

- [x] `src/codegen/debug/arena.rs`: `COUNTER_FILL_GROW_CALLS`, `_GROW_BYTES`,
      `_FREE_CALLS`, `_FREE_BYTES`; append to `ARENA_COUNTERS`; grow the slot. — offsets
      160/168/176/184; `ARENA_COUNTERS` 18 → 22; the compile-time assert is now
      `COUNTER_FILL_FREE_BYTES + 8 == SLOT_SIZE && SLOT_SIZE == 192`; `cargo build --release
      --bin mfb` → `EXIT=0`, no warnings. No other `src/` pin on 18 counters or a 160 B slot
      (grep of `ARENA_COUNTERS`, `counters[18]`, slot 160).
- [x] `arena.rs`: increments before the grow-path fill and the free-path scrub. — Both
      only under `if let Some(slot) = &dbg_slot`, so a non-`--debug` build emits the same
      instruction sequence. The scrub length goes through a fresh vreg (`size − 16`) after the
      `size == 16` skip, so both free paths (quick bin, large bin) are counted and no physical
      argument register is touched. Observed with `--debug` builds of an 8-`Integer` record
      loop (`/tmp/plan-133-b/rec8_1000`, `rec8_2000`): `fill_free_calls` 1,000 / 2,000,
      `fill_free_bytes` 48,000 / 96,000, `free_bytes` 64,016 / 128,016 (the constant 16 B free
      is correctly not scrubbed); `grow` 1 = `fill_grow_calls` 1, `fill_grow_bytes` 4,064 =
      `mapped_bytes` 4,096 − 32.
- [x] `tests/runtime/rt_debug_arena.rs::fill_counters_match_known_allocations`: one 1 MiB
      `List OF Byte` → `fill_grow_calls == grow` and `fill_grow_bytes == mapped_bytes −
      32 × grow`; a loop freeing N 64-byte records → `fill_free_calls == N` and
      `fill_free_bytes == 48 × N`. — Implemented with an 8-`Integer` record (64 B;
      Corrections) at N=1000 vs 2000, asserting the deltas (Δ`free_calls` 1000,
      Δ`free_bytes` 64,000, Δ`fill_free_calls` 1000, Δ`fill_free_bytes` 48,000) so the
      program's fixed setup frees cancel. `COUNTERS` in the same file grew to 22, so the
      registration test checks the new keys too. `cargo test --release --test
      rt_debug_arena fill_counters` → `fill_counters_match_known_allocations ... ok`,
      `1 passed; 0 failed` (61.28 s). `scripts/artifact-gate.sh target/release/mfb
      collections` → `1 tests, 6 build(s), 7 golden(s) checked, 0 diff(s)`, EXIT=0.

Acceptance: `cargo test --release --test rt_debug_arena fill_counters` → 1 passed (~2 min);
`scripts/artifact-gate.sh target/release/mfb collections` → `0 diff(s)` (~1 min, a covered
arena-heavy builtin, proving normal builds did not change).
Commit: b31e6abf8

### Phase 2 — the A/B, recorded

- [x] Added task: a page-load timer for the browser A/B, `tools/browser-load-timer/`
      (README, `load.exp`, `run.sh`; also used by plan-133-C Phase 1). The "load, wait 40 s,
      `q`" driver plan-133-A describes exists on neither machine (Corrections). — Verified
      under both Tcl versions: the host (expect 5.45, Tcl 8.5.9) gave `run 1 load_ms=12991
      exit=0` for `BASIC`; 2223 (expect 5.45.4, Tcl 8.6.18) gave `run 1 load_ms=7451
      exit=0` for `Main_Page`. The first version timed out (`load_ms=timeout`) while the page
      had loaded; the fix is in Corrections.
- [x] Worktree `/tmp/p133-b-fill` at the base; throwaway early return in
      `lower_arena_fill_random`; release build (never committed; the worktree is removed
      afterwards). — `git worktree add --detach /tmp/p133-b-fill b31e6abf8`. One line changed:
      the entry's `abi::branch_eq("arena_fill_done")` became an unconditional
      `abi::branch("arena_fill_done")`, so the fill loop and the PCG state write-back are
      skipped and the helper returns at once (the label is followed directly by `return_`).
      `cargo build --release --bin mfb` → `EXIT=0` (1m 31s). Never committed: `git -C
      /tmp/p133-b-fill status --short` → ` M src/codegen/builtins/math/gen_rng_pcg64.rs` only;
      `git log --oneline -S "plan-133-B THROWAWAY" b31e6abf8..HEAD` → no commits; `git grep -n
      "plan-133-B THROWAWAY" HEAD -- src` → no match. Removed with `git worktree remove
      --force /tmp/p133-b-fill`: absent from `git worktree list`, and `ls` reports "No such
      file or directory".
- [x] macOS: `benchmark/mfb` suite normal vs fill-off, `--run 3` each, back to back; — Plain
      `mfb build` of each suite copy (not `run.sh`'s `-O1..3`; the same flags on both sides).
      A re-run on a quiet host (`ab-macos.sh` 18:50:47 → 18:51:10, 1-minute load 4.08 → 4.21,
      checksums identical) gave geomean x0.702 over 485 rows and the ten named rows in
      `planning/todo.md` § 1 item 2. The first pair ran under a peer build and was discarded
      (Corrections).
      per-row medians compared; geomean and the arena-heavy rows plan-130-C named
      (bignum.modmul, bignum.modexp, crypto.churn, arena.transient, arena.mixed,
      arena.growshrink, scalarbench.listchurn, mapchurn.churn, datetime.civil,
      datetime.iso). ~20 min, because the suite has no row filter.
- [x] 2223: the same pair, cross-built for linux-aarch64 (~25 min on the box). — Cross-built
      on the host with each compiler (`benchmark-glibc.out`, 9,412,608 B each). Run
      `--run 3` normal then fill-off (21:43:57 → 21:44:20), and again in swapped order
      (21:47:15 → 21:47:38); all exits 0, checksums identical, load ≤ 0.45. Geomean normal →
      fill-off x0.701 (first order) and x0.710 (swapped); same-build position noise x0.999 and
      x1.012. Recorded in `planning/todo.md` § 1 item 2 with the ten named rows.
- [x] 2223: browser `Main_Page` load wall time, normal vs fill-off, 3 runs each (~5 min). —
      `tools/browser-load-timer/run.sh … Main_Page 3` for each linux-aarch64 browser
      (21:50:31 → 21:51:21, load ≤ 0.49): normal 7,300 / 7,451 / 7,366 ms, median 7,366;
      fill-off 6,192 / 6,077 / 6,097 ms, median 6,097; x0.828. Every exit 0.
- [x] Record all of it in `planning/todo.md` § Memory § 1 item 2, with the counters from one
      `--debug` browser run (fill bytes as a share of `alloc_bytes`). — Recorded: the 2223 suite
      geomean in both run orders (x0.701, x0.710) and the same-build noise floor (x0.999,
      x1.012); the macOS geomean x0.702; the ten named rows on both hosts; the browser medians
      (7,366 → 6,097 ms, x0.828); and the counters from a `--debug` `Main_Page` load on 2223
      (`load_ms=7832 exit=0`, report kept with `MFB_TIMER_STDERR`). Fill bytes are 85.4% of
      `alloc_bytes` over both arenas (main 126.9%, worker 82.4%), and the counter identities
      hold exactly.

Acceptance: `planning/todo.md` § 1 item 2 carries both geomeans, the ten named rows on both
hosts, the browser medians, and the counters; `git -C /tmp/p133-b-fill status` shows the
throwaway patch was never committed, and the worktree is removed.
Commit: 1f997e49a, ac00bef77

### Phase 3 — docs

- [x] `src/docs/spec/tooling/09_debug-report.md`: the four fill counters. — Two rows after
      `flushes`/`insert_free_calls`: `fill_grow_calls`/`.fill_grow_bytes` (a fresh block's
      usable region, size − 32; `fill_grow_calls` equals `grow`) and
      `fill_free_calls`/`.fill_free_bytes` (a freed chunk past its 16-byte node; 16-byte
      chunks are not counted; `free_bytes − fill_free_bytes == 16 × free_calls`). Both
      identities were measured on the 2223 browser run (Phase 2).
- [x] `src/docs/spec/memory/04_arenas.md` § Entropy Fill: one sentence pointing at the
      counters. — Names the four `arena.<n>.fill_*` keys and points to
      `tooling/09_debug-report.md`. `cargo test --bin mfb docs::spec` → `EXIT=0`,
      `test result: ok. 8 passed; 0 failed`, including `spec_citations_resolve ... ok` and
      `spec_links_resolve ... ok`. No man page lists the arena counters (grep of
      `src/docs` for `double_free_skips`/`arena.<n>` finds only these two spec files).

Acceptance: `cargo test --bin mfb docs::spec` → green (~1 min; includes
`spec_citations_resolve`).
Commit: —

## Validation Plan

- Tests: `fill_counters_match_known_allocations`.
- Runtime proof: the recorded A/B on macOS and 2223.
- Doc sync: `09_debug-report.md`, `04_arenas.md`, `planning/todo.md` § 1 item 2.
- Full suite: once, at the end of plan-133-C.

## Open Decisions

- None.

## Corrections

- **2026-09-13 — Phase 1 re-check of the fill call sites: still 2.**
  `git grep -n "branch_link(ARENA_FILL_RANDOM_SYMBOL)" -- src/codegen/memory/arena/arena.rs`
  → `arena.rs:668` (grow path) and `arena.rs:1404` (free-path scrub), before this letter's
  edits. plan-134 did not add a third.
- **2026-09-13 — "a 64-byte record" means 8 `Integer` fields, not 6.** Measured on the
  pre-change release binary: a loop binding a 6-`Integer` record makes exactly one allocation
  and one free per iteration, each 48 B (`/tmp/plan-133-b/rec1000` vs `rec2000`:
  `free_calls` 1,001 → 2,001, `free_bytes` 48,016 → 96,016, `live_bytes` 0). A 48 B chunk is
  scrubbed over 32 B, so `fill_free_bytes == 48 × N` needs a 64 B chunk: 8 `Integer` fields.
  The test compares N=1000 with N=2000, so the program's fixed setup frees cancel out.
- **2026-09-13 — Phase 2: a `--run 3` suite run takes 10–18 s, not ~20–25 min.** Measured:
  on 2223 the normal build ran 21:43:57 → 21:44:10 and fill-off → 21:44:20 (485 timed rows
  each, `runs: 3`, checksums identical); on macOS 18:45:36 → 18:45:54 and → 18:46:05. The
  estimates must have counted builds. Consequence: repeating a pair (for example with the
  order swapped) costs seconds.
- **2026-09-13 — Phase 2: the browser driver was gone, and a naive rebuild of it never sees a
  load finish.** `drive-browser.exp` (plan-133-A § Measured populations) exists on neither
  machine: `find ~ -maxdepth 5 -name '*.exp'` on 2223 found only the 2026-09-12 all-examples
  strace harness `~/arena-maps/harness/drive.exp`, which uses fixed pauses; nothing on the
  host under `/tmp` or in git. A timer that waited for the footer text `Files: n/m` or the
  padlock timed out (`run 1 load_ms=timeout exit=0`) although the page loaded. A capture
  (`MFB_TIMER_LOG`) showed why: the browser redraws only changed cells, so after the load
  it sends `ESC[40;118H`, colour escapes, `3`, `ESC[40;120H`, `3`, and never re-sends the
  label. The padlock bytes `F0 9F 94 92/93` never appear at all. Separately, the host's
  `expect` 5.45 runs Tcl 8.5.9, which has no `\U` escape, while 2223 runs Tcl 8.6.18. The
  driver now matches a cursor move to row 40 followed by a non-zero digit, in plain ASCII, and
  on the host it measured `load_ms=12991 exit=0` for `BASIC`.
- **2026-09-13 — Phase 2: the first macOS pair ran on a loaded host and is re-run.** The
  load average was 8.96 → 12.93 on 12 CPUs: a peer session's `rustc` at ~400% CPU, plus
  three UTM QEMU VMs that use ~2.7 cores permanently. The pair's spread showed it
  (`mapchurn.churn` x0.528 next to `set (State-Dynamic).add` x1.644, geomean x0.615). Kept as
  `/tmp/plan-133-b/ab/mac-*-loaded.*`. It is re-run once no `rustc`/`cargo` is running and the
  1-minute load average is under 5.

## Summary

Four counters and one recorded experiment. The only care needed is that the fill-off patch
never lands.
