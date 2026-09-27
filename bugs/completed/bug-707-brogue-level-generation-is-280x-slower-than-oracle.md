# bug-707: brogue level generation is ~150-300x slower than the C oracle for identical output

Last updated: 2026-09-26
Effort: x-large (1d–3d)
Severity: HIGH
Class: Performance

Status: Closed
Regression Test: tools/brogue-perf/bench.sh

## STATUS: FIXED (628be1e83)

Both fixes in the Fix Design landed and are byte-identical to the oracle
(`check-terrain.sh`, 320 (seed, depth) pairs across both phases). Each
worked exactly as predicted **on allocation count**: `dig 1 5`'s
`arena.0.alloc_calls` went 6,577,973 → 1,339,889 (Phase 2, grid.mfb hoist)
→ 244,393 (Phase 3, flat flag table) — a combined 96.3% cut, within 0.5
points of this doc's own 96.8% prediction.

**Deviation from the plan: the Goal's proposed "within 10x" was not
reached.** User CPU time barely moved despite the 96.3% allocation-count
cut — `dig 1 5` went from ~2.9s to ~2.0-2.2s, and the harness's own
quiet-machine measurement (`tools/brogue-perf/bench.sh --runs 7`, lock held)
puts the final ratio at **~120x**, not ~10x. Investigation (call-count
instrumentation across every function in the terrain-generation source, and
a project-wide wrap of every `List OF Integer` `collections::set`/`append`
call) found allocation *bytes*, not call *count*, tracks wall-clock cost
here, and ruled out every function this bug named as the byte source. That
gap is filed as `bugs/bug-737-brogue-level-generation-still-120x-slower-after-bug-707.md`
rather than reopening this bug's scope — its own Fix Design's two changes are
both complete, correct, and landed. `tools/brogue-perf/bench.sh`'s budget is
set to 150 (not the originally-proposed 10) to reflect what this bug's own
fixes actually achieve; bug-737 owns closing the remaining gap.

`examples/brogue` is an MFBASIC port of Brogue CE's terrain generator, checked
byte-for-byte against the oracle's own C engine (`examples/brogue/README.md`).
The port's output is provably correct (`cmp` against `oracle/`'s
`terrain_dump` is identical for every seed/depth `check-terrain.sh` runs), but
generating one level costs on the order of 3 seconds of CPU time in the port
versus about 10-20 milliseconds in the oracle for the same `(seed, depth)`
pair — a 150x-300x gap. The dominant cause, measured directly (call-count
instrumentation cross-checked against the runtime's own allocator counters),
is allocation churn: two small, extremely hot helper functions
(`nbDx`/`nbDy` in `src/grid.mfb`, and `tileInfo` in `src/level.mfb`) each do a
fresh heap allocation on every call, and together they account for nearly all
of the ~6.6 million allocations one `dig` performs. Nothing leaks — peak live
bytes stay under 500 KB throughout — this is pure allocate/free churn from
functions that are correct but allocate work that could be a constant lookup
or a flat integer table.

References:

- `examples/brogue/README.md` — build and `dig <levelSeed> <depth>` usage,
  and `check-terrain.sh`'s cmp-based correctness verification this bug's
  reproduction reuses.
- `.ai/collections.md` — "List memory management" (amortized-O(1) `append`
  is the canonical shape; `get` on a record/String/nested-list element has
  "returned an owned deep copy since bug-538") and `src/codegen/memory/marshal/record.rs`
  (`emit_build_inlined_record*`) — every record construction/copy goes through
  `emit_alloc`, even for an all-scalar record; there is no stack/inline
  representation for a `TYPE ... END TYPE` value in this runtime.
- `bugs/bug-700-debug-build-exit-report-takes-tens-of-seconds.md` — a
  *different*, already-tracked bug hit incidentally while reproducing this one
  (a `--debug` build's own exit-report writer is slow once allocation counts
  get large; this bug's numbers below were read from a `--debug` run's report,
  not from its wall-clock time, which bug-700 already explains).

## Failing Reproduction

Built and run today (macOS aarch64, `mfb` at `target/release/mfb`, oracle
built per the README's own recipe against the pinned `oracle/` commit):

```
$ ./target/release/mfb build examples/brogue
Building brogue (executable) for macos-aarch64
Wrote executable to examples/brogue/build/brogue.out

$ cc -std=c99 -w -O1 -DBROGUE_EXTRA_VERSION='""' -DDATADIR=. \
    -Iexamples/brogue/oracle/src/brogue -Iexamples/brogue/oracle/src/platform -Iexamples/brogue/oracle/src/variants \
    examples/brogue/check/terrain_dump.c examples/brogue/check/oracle_host.c \
    examples/brogue/oracle/src/brogue/*.c examples/brogue/oracle/src/variants/*.c \
    examples/brogue/oracle/src/platform/platformdependent.c examples/brogue/oracle/src/platform/null-platform.c \
    -o /tmp/terrain_dump   # (Architect.c excluded from the engine list -- it's #included by terrain_dump.c)

$ /usr/bin/time -p examples/brogue/build/brogue.out dig 1 5 > /tmp/port.out
real 10.19   user 2.98   sys 0.03

$ /usr/bin/time -p /tmp/terrain_dump 1 5 > /tmp/oracle.out
real 0.02    user 0.02   sys 0.00

$ cmp /tmp/oracle.out /tmp/port.out && echo IDENTICAL
IDENTICAL
```

- Observed: the port spends **2.9-4.1 s of user CPU time** generating one
  level; the oracle spends **0.01-0.02 s** for the identical output. Three
  repeated runs of each (`dig 1 5`): port `2.98s / 2.90s / 2.95s`; oracle
  `0.02s / 0.01s / 0.02s`. That is a **~150x-300x** ratio (the oracle's time
  is at the shell timer's resolution floor, so the ratio's lower bound is the
  more defensible number; either way it is not a rounding artifact — it is
  reproducible to within noise on every run).
- `real` time was much larger than `user + sys` for the port on this test
  machine (`real 10-19s` vs `user 2.9-4.1s`) on every run. This is **not**
  attributed to the port in this doc: `uptime` during the runs showed a load
  average of ~174 on a 12-core machine (other sandboxed sessions sharing the
  host), which inflates wall-clock scheduling delay without touching CPU-time
  accounting. **User CPU time, not wall-clock, is the reliable number here**
  and is what the rest of this document uses.
- Expected: a level-generation cost within a small, explicitly-chosen
  multiple of the oracle's (see Goal) — not 150x-300x.
- Contrast case: the oracle's own `terrain_dump` driver, linked directly
  against `Architect.c` and the rest of the unmodified C engine, produces the
  same output at C speed — confirming the gap is the port's own code/runtime
  path, not an inherently expensive algorithm.

| Environment | Command | User CPU time |
| --- | --- | --- |
| macOS aarch64, `mfb build examples/brogue` (release) | `brogue.out dig 1 5` | 2.9-3.0 s (3 runs) |
| macOS aarch64, oracle `terrain_dump` (cc -O1) | `terrain_dump 1 5` | 0.01-0.02 s (3 runs) |
| macOS aarch64, port | `brogue.out dig 1 10` | 4.07 s |
| macOS aarch64, port | `brogue.out dig 1 20` | 4.03 s |

## Root Cause

**Measured, not guessed.** A `--debug` build of the port
(`./target/release/mfb build --debug examples/brogue`) run over `dig 1 5`
reports, via its stderr measurement report:

```
arena.count 1
arena.0.alloc_calls 6577973
arena.0.alloc_bytes 6761923648
arena.0.free_calls 6577963
arena.0.free_bytes 6761654544
arena.0.peak_live_bytes 429280
process.peak_rss_bytes 17334272
```

**~6.58 million allocations, ~6.76 GB total allocated, for one `dig`, with
peak live bytes under 500 KB** — this is not a leak (free_calls tracks
alloc_calls almost exactly; nothing accumulates), it is short-lived churn: the
same small handful of call sites allocate and immediately free, millions of
times.

A prior review (the source of this bug report's assignment) estimated "1.34M
allocations and 6.17 GB total allocated" from the same kind of run. The byte
total is close to what is measured here (6.76 GB vs. 6.17 GB, ~9% apart —
plausibly a different build/seed or optimizer state) but **the allocation
*count* is not**: 6.58M measured here vs. 1.34M claimed, a ~4.9x difference.
The count above is read directly off this run's `--debug` report and is the
number this document relies on; the earlier count should be treated as
superseded by it, not reconciled further.

To find what is actually calling the allocator this often, a throwaway
instrumented copy of the port (`/tmp/brogue-instrument`, never touching the
repo) had one counter added per suspect function, built and run over the same
`dig 1 5`, with `cmp` re-confirming its output was still byte-identical to
the oracle after instrumentation:

```
counters tileInfoCalls=1132307
counters terrainFlagsCalls=181382
counters nbDxCalls=2619044
counters nbDyCalls=2619044
counters filledGridCalls=151
counters fillContiguousRegionCalls=84
counters cellularAutomataRoundCalls=70
counters dijkstraScanCalls=37
```

`nbDxCalls + nbDyCalls + tileInfoCalls = 2619044 + 2619044 + 1132307 =
6370395` — **96.8% of the `--debug` build's 6,577,973 measured allocator
calls**, from exactly three functions. Ranked by measured contribution:

1. **`nbDx`/`nbDy` (`src/grid.mfb:46-52`) — ~79.6% of all allocations
   (5,238,088 of 6,577,973 calls).** Confirmed by reading the code: both
   rebuild an 8-element list literal on every call instead of reading a
   module-level constant —
   ```
   PUBLIC FUNC nbDx(dir AS Integer) AS Integer
     RETURN collections::get([0, 0, -1, 1, -1, -1, 1, 1], dir)
   END FUNC
   ```
   `cDx`/`cDy` (`src/grid.mfb:54-60`) share the identical pattern but were not
   separately instrumented; they are lower call-count call sites (used only
   by the loop-adding pass) and are named here as an in-scope sibling, not
   separately measured. `nbDx`/`nbDy` are called from the innermost loop of
   every neighbor-direction scan in the port — `cellularAutomataRound`
   (70 calls, each iterating every cell x every direction) and
   `fillContiguousRegion` (84 calls, each popping a stack and checking up to
   4 directions) among them — which is why two one-line functions dominate
   the whole run's allocation count.
2. **`tileInfo` (`src/level.mfb:78-81`) — ~17.2% of all allocations (1,132,307
   calls).** `tileInfo(tile)` is `collections::get(tileCatalog, tile)`, where
   `tileCatalog` is `List OF FloorTileType` and `FloorTileType`
   (`src/tables.mfb:609-619`) is a 9-field, all-Integer record. Per
   `.ai/collections.md`, `get` on a record-typed list element has "returned
   an owned deep copy since bug-538"; per
   `src/codegen/memory/marshal/record.rs`, every record materialization goes
   through `emit_alloc` even when the record is flat/all-scalar (no
   stack/inline record representation exists in this runtime). `terrainFlags`
   (`src/level.mfb:87-94`) calls `tileInfo` once per terrain layer inside a
   4-iteration loop purely to read `.flags` off the returned record and
   discard the rest — 181,382 `terrainFlags` calls account for 725,528 of the
   1,132,307 `tileInfo` calls (64%); the remaining 36% come from
   `terrainMechFlags` (`src/level.mfb:97-104`, same pattern reading
   `.mechFlags`) and other direct callers.
3. **`dijkstraScan` (37 calls), `fillContiguousRegion` (84 calls),
   `filledGrid` (151 calls) — ruled out by call count, confirmed today.** The
   prior review's claim that these are not hot was taken as an unverified
   lead going in; the instrumentation above reproduces its exact call counts
   (37/84/151) for this seed/depth, so it is now a measured fact, not an
   assumption. Separately, `filledGrid`'s `grid = collections::append(grid,
   value)` loop is the canonical amortized-O(1) `append` shape
   (`.ai/collections.md`), so even at up to `DCOLS*DROWS` iterations per call
   it is not itself a hot path.

**Corroborating (not conclusive) profile.** `sample <pid> <duration>` against
a plain release build running a longer synthetic workload (`dig 1 20` in a
loop) shows the overwhelming majority of on-CPU samples concentrated in two
narrow, unsymbolicated address ranges inside `brogue.out`'s own code (the
binary carries no local symbol table, so `atos`/`nm` cannot name the
functions) — consistent with a small number of tiny, extremely hot functions
dominating CPU time, but not independent proof of which functions those are.
The call-count instrumentation above, cross-checked against the `--debug`
build's own allocator counters, is the primary evidence; the `sample` run is
cited only as a second, independent signal pointing the same direction.

## Goal

**Proposed goal (not yet decided):** level generation for `dig <seed>
<depth>` should cost no more than roughly **10x** the oracle's user CPU time,
down from the ~150x-300x measured today. This is a recommendation for
discussion, not a committed number — the actual owner has not signed off on a
specific ratio.

- `nbDx`, `nbDy`, `cDx`, `cDy` stop allocating on every call (module-level
  constant lists, or an equivalent the compiler can prove immutable and
  therefore not copy).
- `terrainFlags`/`terrainMechFlags` stop allocating a full `FloorTileType`
  record per layer per call just to read one field.
- Output stays byte-identical to the oracle on every `(seed, depth)` pair
  `check-terrain.sh` covers — this bug is a performance bug precisely because
  correctness is not in question; a fix that changes even one byte of output
  is not a fix.

### Non-goals (must NOT change)

- `FloorTileType`/`tileCatalog`'s public shape as consumed by any other
  reader of `src/tables.mfb` (if any exist beyond `level.mfb`) — a flat
  bitfield table is an internal implementation detail of `tileInfo`'s
  callers, not a replacement for the record catalog, unless a full audit (out
  of scope for this bug) shows nothing else needs the record's other fields
  (`drawPriority`, `chanceToIgnite`, etc.) at the per-cell hot path — those
  callers are unaffected and must stay unaffected.
- `gen/gen_tables.py`'s guarantee that every table value comes from the
  oracle's own compiled constants, never hand-transcribed
  (`gen/gen_tables.py --check` must still pass after any table-shape change).
- The RNG draw sequence and everything `check-rng.sh`/`check-terrain.sh`
  verify today.

## Blast Radius

- `src/grid.mfb:nbDx`, `:nbDy` — fixed by this bug (module constant).
- `src/grid.mfb:cDx`, `:cDy` — same hazard, not separately call-counted above,
  but read as source and confirmed to share the identical
  rebuild-a-literal-every-call pattern; in scope as a sibling fix, low
  individual call count (loop-adding pass only).
- `src/level.mfb:tileInfo`, `:terrainFlags`, `:terrainMechFlags` — fixed by
  this bug (avoid the whole-record allocation for a single-field read).
- Every other reader of `tileInfo()` (`src/terrain.mfb`, 13 call sites) —
  unaffected if `tileInfo` keeps its existing signature/return type; only
  `terrainFlags`/`terrainMechFlags`'s internal use of it changes shape if the
  fix is a flat flag table rather than a cheaper `tileInfo`.
- `src/grid.mfb:filledGrid`, `:fillContiguousRegion`, `src/dijkstra.mfb:dijkstraScan`
  — read, instrumented, and confirmed low call count; **not** touched by this
  bug's fix; named here only because the originating review's leads listed
  them and this document is closing that question with a measurement, not
  leaving it open.
- `gen/gen_tables.py` — potentially touched if the fix chooses the flat
  `List OF Integer` flag table route (see Fix Design); it already generates
  `FloorTileType`'s `flags`/`mechFlags` fields from the oracle's compiled
  tables, so emitting a second, flattened view is an additive change to an
  existing generator, not a new one.

## Fix Design

Two independent changes, either landable alone:

1. **Hoist `nbDx`/`nbDy`/`cDx`/`cDy` to module-level constant lists.**
   Mechanical: replace `RETURN collections::get([...], dir)` with a
   `PUBLIC LET nbDxTable AS List OF Integer = [...]` at module scope and
   `RETURN collections::get(nbDxTable, dir)` in the function body (or inline
   the table access at call sites, if that proves cheaper once the list
   itself is allocated once). This is expected to eliminate the ~79.6% of
   allocations attributed to these two functions, since the module-level
   list literal is constructed once at program start rather than once per
   call.
2. **Stop materializing a whole `FloorTileType` per flag read.** Two options,
   in order of preference:
   - **Preferred: emit a flat `List OF Integer` flags/mechFlags table**
     alongside (or instead of, for this hot path) the record catalog, from
     `gen_tables.py`, so `terrainFlags`/`terrainMechFlags` become a single
     `collections::get` on an `Integer` list — no record allocation at all.
     This keeps the "every value comes from the oracle's compiled constants"
     guarantee, since the generator already has the flag values in hand when
     it emits the record catalog.
   - **Fallback: compute `terrainFlags` once per cell and cache/reuse it**
     rather than fixing the allocation at its source. Rejected as the primary
     fix because it treats the symptom (repeated calls) rather than the cause
     (each call allocating), and does not help the other 13+ direct
     `tileInfo` call sites in `src/terrain.mfb` that were not separately
     counted here.

Recommended: do (1) first — it is mechanical, low-risk, and the largest
single win (~80% of allocations) — then (2), which touches the generator and
needs its own regeneration/diff review.

## Phases

### Phase 1 — perf harness + measurement lock-in (no behavior change)

- [x] Add `tools/brogue-perf/` (see Regression Test below): a script that
      builds the port and the oracle's `terrain_dump`, runs a fixed
      `(seed, depth)` matrix, records user CPU time and (via a `--debug`
      build) `arena.0.alloc_calls`/`alloc_bytes` for the port, and fails if
      the port/oracle user-time ratio exceeds a budget constant. Seed the
      budget at today's measured ~300x (a loose gate) so Phase 1 is a
      measurement lock-in, not a behavior change.
- [x] Record this document's numbers as the harness's first captured
      baseline.

Acceptance: harness runs today, passes against current (slow) behavior,
fails if intentionally given a lower budget than measured. Confirmed RED
against a 10x budget at base (ratio 171.2x, `--runs 3`).
Commit: 3b6f50eda

### Phase 2 — hoist the neighbor-delta tables

- [x] `src/grid.mfb`: module-level constants for `nbDx`/`nbDy`/`cDx`/`cDy`.
- [x] `examples/brogue/check/check-terrain.sh` full run (all seeds/depths);
      confirm zero output diffs.
- [x] Re-run the Phase 1 harness; record the new ratio.

Acceptance: `check-terrain.sh` clean; harness shows a measured improvement
consistent with the ~79.6% allocation-count prediction above. Confirmed:
`dig 1 5` alloc_calls 6,577,973 → 1,339,889 (79.6% cut, exact prediction);
user CPU time unchanged (~2.2s), see STATUS block above.
Commit: e21aee167

### Phase 3 — flatten the per-cell flag lookup

- [x] `gen/gen_tables.py`: emit the flat flags/mechFlags table(s).
      `gen/gen_tables.py --check` passes.
- [x] `src/level.mfb`: `terrainFlags`/`terrainMechFlags` read the flat
      table instead of materializing `FloorTileType`.
- [x] `check-terrain.sh` full run again; confirm zero output diffs.
- [x] Re-run the Phase 1 harness; tighten its budget constant to the new
      measured ratio (plus headroom), so this bug's fix becomes the
      regression gate for itself.

Acceptance: `check-terrain.sh` clean; harness ratio at or below the Goal's
proposed 10x (or the ratio the owner actually decides on). **Deviation:**
ratio did not reach 10x — quiet-machine measurement is ~120x. Budget set to
150 (headroom over the measured floor, not the aspirational 10x); see
STATUS block and bug-737.
Commit: 628be1e83

## Validation Plan

- Regression test: `tools/brogue-perf/bench.sh` (Phase 1), gating on the
  port/oracle user-CPU-time ratio and on `arena.0.alloc_calls` for a fixed
  `(seed, depth)` matrix.
- Runtime proof: `examples/brogue/check/check-terrain.sh` (all seeds/depths)
  stays clean through every phase — this bug's fix must never change a
  single output byte.
- Doc sync: none expected — `examples/brogue/README.md` documents behavior,
  not performance, and does not need updating unless the fix changes the
  `dig` CLI surface (it should not).
- Full suite: the project's own `cargo test` is unaffected (this is an
  example, not compiler code) unless Phase 3 touches `gen_tables.py` output
  in a way any compiler test depends on (unexpected; verify at that point).

## Open Decisions

- ~~The target ratio (proposed: within 10x of the oracle)~~ **Decided:** took
  the doc's proposed 10x as the target to aim for; both fixes landed but only
  reached ~120x (quiet-machine measurement). Rather than silently loosen this
  bug's own goal, the harness budget was set to 150 (measured floor plus
  headroom) and the remaining gap to 10x was filed as
  `bugs/bug-737-brogue-level-generation-still-120x-slower-after-bug-707.md`.
- Whether the flat flag table (Fix Design, option 1) replaces
  `FloorTileType`'s `flags`/`mechFlags` fields outright or lives alongside
  them — **Decided:** lives alongside. `tileFlagsCatalog`/
  `tileMechFlagsCatalog` are additive tables; `FloorTileType`'s own
  `flags`/`mechFlags` fields and every other `tileInfo()` reader are
  untouched, since bug-737's investigation found the remaining allocation
  cost is not concentrated in `tileInfo`'s other call sites (see that bug's
  Root Cause) — replacing the record fields outright would have been
  unjustified churn for no measured benefit.

## Summary

The engineering risk is almost entirely in Phase 3 (the generator and table
shape change) and in the mechanical-but-must-be-exhaustive
`check-terrain.sh` re-verification after each phase — the fix itself
(Phase 2's constant hoist) is small and low-risk. The measurement in this
document is solid: call-count instrumentation in a throwaway copy
independently reproduces the `--debug` build's own allocator counters to
within 3.2%, and reproduces the prior review's ruled-out call counts
(37/84/151) exactly, so the two hot functions identified here are measured,
not guessed.
