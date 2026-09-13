# plan-133-B: what the entropy fill costs

Last updated: 2026-09-12
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
| plan-133-A complete | `ls planning/completed/plan-133-A-*` → one match | NOT MET |

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
| Fill call sites in the allocator | 2 | `git grep -n "branch_link(ARENA_FILL_RANDOM_SYMBOL)" -- src/codegen/memory/arena/arena.rs` → 2 |
| Browser worker frees per `Main_Page` load | 67,918,823 | `arena.1.free_calls`, `planning/todo.md` § 2 |
| Browser worker grows per `Main_Page` load | 192,273 | `arena.1.grow`, same |

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

- [ ] `src/codegen/debug/arena.rs`: `COUNTER_FILL_GROW_CALLS`, `_GROW_BYTES`,
      `_FREE_CALLS`, `_FREE_BYTES`; append to `ARENA_COUNTERS`; grow the slot.
- [ ] `arena.rs`: increments before the grow-path fill and the free-path scrub.
- [ ] `tests/runtime/rt_debug_arena.rs::fill_counters_match_known_allocations`: one 1 MiB
      `List OF Byte` → `fill_grow_calls == grow` and `fill_grow_bytes == mapped_bytes −
      32 × grow`; a loop freeing N 64-byte records → `fill_free_calls == N` and
      `fill_free_bytes == 48 × N`.

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

## Summary

Four counters and one recorded experiment. The only care needed is that the fill-off patch
never lands.
