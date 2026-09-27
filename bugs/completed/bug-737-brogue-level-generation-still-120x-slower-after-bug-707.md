# bug-737: brogue level generation is still ~120x slower than the oracle after bug-707's fixes

Last updated: 2026-09-26
Effort: x-large (1d–3d)
Severity: HIGH
Class: Performance

Status: FIXED
Regression Test: tests/runtime/rt_operand_snapshot.rs (`a_global_operand_before_a_call_that_cannot_store_it_is_not_copied`, `a_call_that_reaches_the_store_through_a_helper_still_snapshots`); tools/brogue-perf/bench.sh (budget 150 -> 25)

## STATUS: FIXED (84ff87d37)

Root cause found and fixed in the compiler, not the example. bug-496's operand
snapshot (`src/codegen/engine/value/operand_snapshot.rs`) copied any operand
rooted in a module-level global into a statement temporary whenever a LATER
operand called any user function. `src/level.mfb`'s `layerAt` is
`collections::get(pmapLayers, layerIndex(x, y, layer))`: `layerIndex` is pure,
yet every read copied the 9,164-entry (73 KB) layer list; `cellFlagsAt`,
`volumeAt` and `machineNumberAt` copied their 18 KB lists the same way
(`get(pmapFlags, gi(x, y))`). The fix (`later_operands_can_free`) asks, for a
global root, whether a later operand can reach a `StoreGlobal` of THAT global
(the `store_reach.rs` walk bug-665/666 already use); resource-holding globals
and local roots keep the broad rule.

Measured, `dig 1 5`, `--debug` arena report: alloc_bytes 6,087,626,176 ->
32,033,136, hit_large_bin 98,645 -> 2,348, alloc_calls 244,393 -> 148,095;
user time 2.3 s -> 0.20 s. Harness (`bench.sh --runs 7`, heavy lock held):
**120.1x -> 17.6x** (port 1.284 s, oracle 0.073 s). `check-terrain.sh 8 1`:
all 320 pairs match.

The 10x goal is NOT met. The remainder is not memory: a sampling profile puts
~77% of `dig 1 20` in `cellularAutomataRound` and four one-expression leaf
helpers (`gi`, `coordinatesAreInMap`, `nbDx`, `nbDy`), called rather than
inlined, with every temporary spilled. Filed as **bug-740**. `bench.sh`'s
budget is tightened from 150 to 25 (17.6 plus noise headroom), so a return of
the copying fails the gate.

Deviation: one existing test assertion corrected (the `GS & other(1)` line of
`concat_operand_survives_a_call_that_reassigns_the_global` no longer
snapshots — `other` stores a different global); the four-question proof is in
the commit message of 84ff87d37.

bug-707 landed both of its identified fixes for `examples/brogue`'s `dig`
performance — hoisting `nbDx`/`nbDy`/`cDx`/`cDy` to module constants
(`src/grid.mfb`) and flattening the per-cell terrain-flag lookup
(`tileFlagsCatalog`/`tileMechFlagsCatalog` in `src/tables.mfb`, read by
`terrainFlags`/`terrainMechFlags` in `src/level.mfb`) — and both landed
byte-identical to the oracle (`check-terrain.sh`, 320 (seed, depth) pairs).
Both fixes worked exactly as bug-707 predicted **on allocation call count**:
one `dig 1 5` dropped from 6,577,973 arena allocations to 1,339,889 after the
first fix, then to 244,393 after the second — a combined 96.3% cut, closely
matching the two fixes' own predicted share of the original count (79.6% +
17.2% = 96.8%).

**But user CPU time barely moved**: ~2.9s (bug-707's original measurement) to
~2.0-2.2s measured today across all three states (before either fix, after
the first, after both) — nowhere near a proportional drop, and nowhere near
bug-707's proposed 10x-of-oracle goal. On a quiet machine (`/tmp/mfb-heavy.lock`
held, median of 7 runs per `(seed, depth)` pair, `tools/brogue-perf/bench.sh`),
the final ratio after both fixes is **~120x** (`port total user=10.812s
oracle total user=0.090s ratio=120.1x`) — better than the noisier ~170-190x
seen on a busier machine, but still an order of magnitude past the proposed
10x goal. This document is the "report the remaining gap" bug-707's owner
asked for when its identified fixes don't reach the goal: this is that gap,
with what has and hasn't been ruled out.

References:

- `bugs/completed/bug-707-brogue-level-generation-is-280x-slower-than-oracle.md`
  — the fix that landed and its own measurements; this bug picks up exactly
  where its Phase 3 acceptance criterion ("harness ratio at or below the
  Goal's proposed 10x... or the ratio the owner actually decides on") was not
  met.
- `tools/brogue-perf/bench.sh` — the perf harness bug-707 added; this bug's
  numbers come from it (`--runs 7`, matrix seeds 1/2/3 at depths 5/10/15/20).
- `.ai/collections.md` — "self-update" (`plan-142-H`, `plan-147`): a
  `global = op(global, ...)` or a handed-over local's `acc = helper(acc, ...)`
  self-update is documented as mutating in place, not copying, once ownership
  resolves to one holder. This bug's measurements are consistent with that
  optimization mostly working (see Root Cause) but not with it eliminating
  enough copying to close the gap.

## Failing Reproduction

Built today in the bug-707 fix's own worktree tip, oracle built per
`examples/brogue/README.md`'s recipe (`Architect.c` excluded, `#include`d by
`check/terrain_dump.c`), macOS aarch64:

```
$ ./target/release/mfb build examples/brogue
$ mkdir /tmp/mfb-heavy.lock   # quiet the machine for a real number
$ tools/brogue-perf/bench.sh --runs 7 --budget 10
...
port total user=10.812s  oracle total user=0.090s  ratio=120.1x  budget=10x  (seed 1 depth 5 alloc_calls=244393)
FAIL: ratio 120.1x exceeds budget 10x
$ rmdir /tmp/mfb-heavy.lock
```

- Observed: ratio **~120x** on a quiet machine (median of 7 runs); as high as
  ~170-190x on a busy shared machine (machine-noise, not the bug — see the
  harness's own median-of-N methodology). `dig 1 5` alone: user CPU
  ~2.0-2.2s port vs. ~0.01-0.02s oracle.
- Expected (bug-707's proposed goal, not yet met): within ~10x.
- Contrast case: `check-terrain.sh` output is unaffected — this is purely a
  performance gap, not a correctness regression from bug-707's fix.

| State | dig 1 5 user CPU | arena.0.alloc_calls | arena.0.alloc_bytes |
| --- | --- | --- | --- |
| Before bug-707 | ~2.9-3.0s (bug-707's own numbers, busier machine) | 6,577,973 | 6,761,923,648 |
| After bug-707 Phase 2 (grid.mfb hoist) | ~2.2-2.25s | 1,339,889 | 6,175,258,240 |
| After bug-707 Phase 3 (flat flag table) | ~2.0-2.15s | 244,393 | 6,087,626,176 |

The pattern across all three rows is the same: **allocation *count* fell by
96.3% total, allocation *bytes* fell by only 10%, and wall-clock time tracked
the bytes, not the count.** Whatever is allocating the remaining ~6GB per
`dig` was already there before bug-707's fixes and is unaffected by them.

## Root Cause

**Found (see STATUS above): the bug-496 operand snapshot of a global read
through a pure index helper.** The rest of this section is the original
investigation, kept for its measurements; its "working hypothesis"
(self-update chains) was wrong — the copies were reads, not updates, which is
why the `set`/`append` wrap below saw no excess.

**Originally: not yet found — this section records what has been ruled out, measured
today, and why the remaining hypotheses point at the compiler/runtime rather
than at any specific example function.**

Ruled out by call-count instrumentation (a throwaway instrumented copy of
every top-level `FUNC`/`SUB` in `src/grid.mfb`, `src/rooms.mfb`,
`src/terrain.mfb`, `src/level.mfb`, `src/dijkstra.mfb` — one counter per
function, `cmp`-confirmed still byte-identical to the oracle after
instrumentation, never committed):

- The two functions bug-707 fixed (`nbDx`/`nbDy`, `tileInfo` via
  `terrainFlags`/`terrainMechFlags`) are exactly as cheap as predicted now:
  `tileInfo`'s remaining (unfixed, out-of-bug-707's-non-goals) call count is
  36,795 for `dig 1 5` — at ~80 bytes/call that is ~3MB, not a meaningful
  share of the ~6.1GB measured.
- No function in the five instrumented files is called anywhere near the
  98,645/144,689 split the arena's `hit_large_bin`/`hit_quick_bin` counters
  report for `dig 1 5` (`arena.<n>.hit_large_bin`/`.hit_quick_bin`,
  `src/docs/spec/tooling/09_debug-report.md`; large means ≥2048 bytes,
  `ARENA_QUICK_BIN_MAX` in `src/codegen/error/constants/error_constants.rs`).
  The highest non-utility call counts measured for `dig 1 5` were
  `directionOfDoorSite` (146,503 calls — a pure read function, no
  allocation) and `tileInfo` (36,795, see above). The grid-shaped functions
  bug-707's doc suspected as generically hot — `cellularAutomataRound` (70
  calls), `fillContiguousRegion` (84), `filledGrid` (151), `spawnMapDF` (12),
  `fillSpawnMap` (12), `levelIsDisconnectedWithBlockingMap` (1) — all have
  call counts too low to individually account for a six-figure allocation
  total, even generously assuming one large allocation per call.
- Wrapping every `collections::set`/`collections::append` call site on
  `List OF Integer` project-wide (the dominant element type; the handful of
  `List OF Boolean`/`List OF Pos` sites were left unwrapped as clearly
  low-volume) with a counting shim, rebuilt and `cmp`-confirmed still
  byte-identical: **633,609 total set/append calls against only 244,393
  total arena allocations** for the same `dig 1 5`. This is strong evidence
  the runtime's self-update/copy-elision (`.ai/collections.md`'s
  `plan-142-H`/`plan-147`) is **not** simply failing to fire — most calls are
  genuinely free. The question this bug leaves open is which ~38% of those
  633,609 calls *do* allocate, and why that fraction — spread thin enough
  that no single function's call count explains it — adds up to bytes that
  do not shrink even as call count collapses 96.3%.

**Working hypothesis, unconfirmed:** the per-`dig` allocation volume is not
concentrated in one or two hot call sites the way bug-707's was; it is
distributed across many ordinary-looking self-update chains
(`grid = collections::set(grid, ...)` inside loops throughout
`rooms.mfb`/`terrain.mfb`), each contributing a small, seemingly-correct
number of real copies (list growth reallocations, or a copy at a self-update
site the compiler's last-use analysis does not resolve to in-place across a
deeper call chain than the isolated cases already covered by
`.ai/collections.md`). Confirming this needs either the compiler's own
instrumentation (a per-callsite allocation attribution the current `--debug`
report does not provide — only aggregate arena counters) or a source-level
reading of the last-use/handover analysis (`.ai/collections.md`, the
`self_update.rs` codegen) against the actual call shapes in
`rooms.mfb`/`terrain.mfb`, which goes beyond what example-level measurement
can settle and is why this is filed rather than fixed here.

## Goal

- Either: identify the actual allocation source(s) behind the ~6GB/`dig` and
  reduce them enough to approach bug-707's original 10x goal, **or** —if the
  cost turns out to be inherent to this runtime's list-copy semantics at this
  call volume — establish a firm, evidence-backed floor for what ratio is
  achievable without a language/runtime change, and update
  `tools/brogue-perf/bench.sh`'s budget to that floor with the evidence cited.
- Output stays byte-identical to the oracle throughout (same non-goal as
  bug-707).

### Non-goals (must NOT change)

- Anything bug-707 already fixed or scoped out (see its Blast Radius) is not
  reopened by this bug unless investigation here shows a defect in that fix
  specifically (none is suspected — both fixes are confirmed working as
  designed, see Root Cause).
- `tileInfo`'s signature/return type and its other call sites — bug-707's
  non-goal, inherited here.

## Blast Radius

- Whatever the eventual root cause is likely touches the compiler's
  self-update/last-use analysis (`.ai/collections.md`,
  `src/codegen/collection/assign/self_update.rs`) or the arena allocator's
  cost model for list growth — both are shared by every MFBASIC program that
  builds lists in loops, not just this example. A fix here may be a compiler
  change with a much broader blast radius than `examples/brogue`; scope that
  properly once the root cause is confirmed, rather than assuming it is
  example-local.
- `examples/brogue/src/{grid,rooms,terrain,level}.mfb` — read and
  instrumented (throwaway, never committed) for this bug's measurements; no
  source change proposed here.

## Fix Design

Not proposed — the root cause is not yet confirmed (see Root Cause). The next
step is compiler-side profiling (a per-callsite allocation attribution, or a
manual audit of `self_update.rs`'s handling of `rooms.mfb`'s deeper call
chains against the simpler cases `.ai/collections.md` documents as working),
which needs compiler-contributor context this bug hands off rather than
guesses at from example-only measurement.

## Phases

### Phase 1 — confirm or refute the self-update hypothesis

- [x] Either instrument the compiler's arena allocator to attribute bytes to
      call sites (even coarsely, e.g. by call depth or return address), or
      manually trace `self_update.rs`'s last-use analysis against 2-3 of
      `rooms.mfb`/`terrain.mfb`'s actual self-update chains (e.g.
      `attachRooms`'s `grid = insertRoomAt(grid, ...)`,
      `levelIsDisconnectedWithBlockingMap`'s `zoneMap`/`passable` builds) to
      determine whether each compiles to the in-place path or a copy.

Acceptance: a named call site (or a named class of call site) confirmed,
with a measurement, to be where the remaining ~6GB/`dig` comes from.

Done: cumulative alloc_bytes per stage (stopping `dumpDig` after each dump)
located the bytes at the `dumpMap` stages and the later stages that read the
map; `--ncode` showed `operand_snapshot` slots in exactly `layerAt`,
`cellFlagsAt`, `volumeAt`, `machineNumberAt`; hoisting the index into a `LET`
in a throwaway copy cut alloc_bytes 6.09 GB -> 32 MB with byte-identical
output, confirming the mechanism before the compiler fix.
Commit: 84ff87d37 (tests RED at base: 5 + 1 snapshot slots; GREEN: 0)

### Phase 2 — fix or floor

- [x] If a fixable site/class is found: fix it, re-run `check-terrain.sh`
      and `tools/brogue-perf/bench.sh`.
- [x] If the cost is inherent at this call volume: update
      `tools/brogue-perf/bench.sh`'s `BUDGET_RATIO` default to the confirmed
      floor, with the measurement that justifies it cited in this doc and in
      the script's own comment.

Acceptance: `check-terrain.sh` clean; `bench.sh` passes against its (possibly
revised) budget with the fix, or the budget documents why it cannot go lower.
Commit: 84ff87d37 (fix); e257b8afa (budget 25, bug-740 filed)

## Validation Plan

- Regression test: `tools/brogue-perf/bench.sh` (already exists).
- Runtime proof: `examples/brogue/check/check-terrain.sh`, all seeds/depths.
- Full suite: unaffected unless Phase 1 implicates compiler code, in which
  case the project's full `cargo test` gates the fix.

## Open Decisions

- Whether the eventual fix belongs in the compiler (self-update analysis) or
  is out of reach without a language change (e.g. an in-place/mutable list
  view builtin) — decide once Phase 1 confirms the mechanism.

## Summary

bug-707's two fixes are both correctly implemented and landed — allocation
*count* dropped 96.3% exactly as that bug's instrumentation predicted, and
`check-terrain.sh` stays clean. What bug-707 did not anticipate is that
*count* was not the wall-clock cost driver: allocation *bytes* (and user CPU
time) barely moved, because a call-count instrumentation sweep covering every
function in the example's terrain-generation source, plus a full project-wide
wrap of `List OF Integer` `set`/`append` calls, rules out any single hot
function or a simple "copy-elision isn't firing" story. The remaining
~120x gap is real, measured, and not yet root-caused; closing it needs
compiler-side tooling or expertise this bug hands off rather than guesses at.
