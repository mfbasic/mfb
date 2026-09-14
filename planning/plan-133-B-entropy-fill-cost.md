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
Commit: —

### Phase 2 — the A/B, recorded

- [ ] Worktree `/tmp/p133-b-fill` at the base; throwaway early return in
      `lower_arena_fill_random`; release build (never committed; the worktree is removed
      afterwards).
- [ ] macOS: `benchmark/mfb` suite normal vs fill-off, `--run 3` each, back to back;
      per-row medians compared; geomean and the arena-heavy rows plan-130-C named
      (bignum.modmul, bignum.modexp, crypto.churn, arena.transient, arena.mixed,
      arena.growshrink, scalarbench.listchurn, mapchurn.churn, datetime.civil,
      datetime.iso). ~20 min, because the suite has no row filter.
- [ ] 2223: the same pair, cross-built for linux-aarch64 (~25 min on the box).
- [ ] 2223: browser `Main_Page` load wall time, normal vs fill-off, 3 runs each (~5 min).
- [ ] Record all of it in `planning/todo.md` § Memory § 1 item 2, with the counters from one
      `--debug` browser run (fill bytes as a share of `alloc_bytes`).

Acceptance: `planning/todo.md` § 1 item 2 carries both geomeans, the ten named rows on both
hosts, the browser medians, and the counters; `git -C /tmp/p133-b-fill status` shows the
throwaway patch was never committed, and the worktree is removed.
Commit: —

### Phase 3 — docs

- [ ] `src/docs/spec/tooling/09_debug-report.md`: the four fill counters.
- [ ] `src/docs/spec/memory/04_arenas.md` § Entropy Fill: one sentence pointing at the
      counters.

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

## Summary

Four counters and one recorded experiment. The only care needed is that the fill-off patch
never lands.
