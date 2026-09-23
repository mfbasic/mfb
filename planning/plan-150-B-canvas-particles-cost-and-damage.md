# plan-150-B: Particle systems — geometry cache, damage, and the cost model

Last updated: 2026-09-22
Effort: medium (1h–2h)
Depends on: plan-150-A

Letter A expands a `ParticleSystem` into N ordinary draw entries and renders them. It
does so through the *existing* geometry cache and the *existing* damage diff, neither
of which was designed for N entries that all differ and all change every frame.

This letter measures what that actually costs and fixes what the measurement finds.
Its deliverable is a **cost model a user can plan against**: how many particles are
reachable before the frame declines to software, what the expansion costs per frame,
and what a moving system does to the damage diff.

Behavioral outcome: `mfb man canvas` can state, from measurement, the particle count a
system sustains at 60 fps and the count at which a frame declines to software; and a
particle system that moves repaints only where it moved, rather than forcing a full
redraw every frame.

References:

- `planning/plan-150-A-canvas-particles-types-and-expansion.md` — the type set, the
  analytic model, and the expansion this letter measures.
- `src/codegen/builtins/canvas/helper_geometry.rs:108` — `__CANVAS_GEO_CAPACITY = 256`,
  the cache this letter stresses.
- `src/codegen/builtins/canvas/helper_damage.rs` — the damage diff, and the two sites
  that read the per-entry offsets.
- `benchmark/RANKING.md` §1.1 — why `min` and not `median` is the column to rank on.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-150-A complete and archived | `ls planning/plan-150-A-* → no matches` | NOT MET |
| The particle golden renders | `cargo test --test rt_canvas_golden particles` → pass | NOT MET |

If plan-150-A is not complete, this letter cannot start, full stop.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again before
> you decide to stop. If you stop, report the status of *all* prerequisites.

## 1. Goal

- The per-frame cost of a particle system is **measured**, at three counts, and
  recorded in this plan as a table with the command that produced it.
- A particle system does not evict unrelated scene geometry from the 256-entry cache.
- A moving particle system produces a damage rectangle bounded by the particles, not a
  full-frame redraw, whenever the rest of the scene is static.

### Non-goals (explicit constraints)

- **`CANVAS_MAX_FRAME_ITEMS` is not raised by this letter.** Measuring what the current
  4096 sustains is the whole point; changing the constant before the measurement exists
  would size a transport against a guess. If the measurement argues for a change, that
  is a separate letter with the number in hand.
- **No change to the analytic model.** Letter A's formulas are fixed; this letter is
  about what expansion costs, not what it computes.
- **No truncation.** A system that exceeds the frame cap declines the frame to software,
  as every other over-cap scene does. Clamping the particle count to fit would draw a
  *different scene* and report success — the exact lie the `*Renderable` predicates
  exist to prevent (`helper_render.rs:120`).
- **No new caching layer for anything but particles.** Whatever this letter does to the
  geometry cache must leave a particle-free scene's behaviour byte-identical.

## 2. Current State

Every entry `__canvas_appendDraw` produces is resolved through `__canvas_geometryFor(item, hash)`,
which is the geometry cache: 256 entries (`helper_geometry.rs:108
LET __CANVAS_GEO_CAPACITY AS Integer = 256`), keyed by item hash, evicted
least-recently-*used* via `__CANVAS_GEO_REV`.

Letter A's expansion gives every live particle a distinct hash — distinct transform,
distinct tint — so a 1,000-particle system presents 1,000 distinct keys to a 256-entry
cache **every frame**. The cache's own comment already anticipates the shape of this
problem for glyphs: *"a scene larger than `__CANVAS_GEO_CAPACITY` produces it on every
frame"* (`helper_glyph_cache.rs:325`).

The damage diff (`helper_damage.rs`) pairs the expanded hash list with the expanded
offset list by index and answers a full redraw when the two are different lengths.
A particle system's live count changes as particles are born and die, so its entry
count changes between frames — which is exactly the condition that forces a full
redraw.

### Measured populations

| What | Count | Command |
|---|---|---|
| Geometry cache capacity | 256 | `grep -n 'GEO_CAPACITY' src/codegen/builtins/canvas/helper_geometry.rs` |
| Geometry header size | 47 floats | `grep -n 'GEO_HEADER' src/codegen/runtime/canvas/mod.rs` → `__CANVAS_GEO_HEADER = 47` |
| Frame quad cap | 4096 | `src/codegen/runtime/canvas/mod.rs:681` |
| Heavy Float3 pipeline throughput | 200,000 iterations in 28.492 ms min (Apple M2 Max) | `grep -A2 '^vector:' benchmark/baseline/mfb-O1.log` → `math : 28.764, 28.733, 28.492, 28.905`; the loop is `FOR k = 0 TO 199999` with 8 `vector::` ops (`benchmark/mfb/src/vector.mfb`) |
| Per-frame cost of a particle system | **UNMEASURED** | Phase 1 measures it; the effort estimate above assumes it is not pathological |

The throughput row is the *only* basis for expecting the expansion to be affordable —
~142 ns per heavy vector iteration. It is a bound on the arithmetic, **not** a
measurement of the expansion, which also allocates a `DrawItem` per particle and hashes
it. Phase 1 exists because that difference is unknown and it sets everything else.

### Verified properties

- **The cache is keyed on the full item hash.** VERIFIED by reading
  `__canvas_geometryFor` and `__canvas_hashItem` in `helper_geometry.rs`: the hash folds
  every header slot, so two particles differing only in transform are different keys.
- **UNVERIFIED — whether cache thrash is actually expensive here.** A miss rebuilds a
  47-float header, which is cheap; the cost may be dominated by eviction bookkeeping
  (`__CANVAS_GEO_HASHES`/`OFFSETS`/`COUNTS`/`LASTUSED` are four parallel lists walked on
  eviction, `helper_geometry.rs:959`). Phase 1 measures before Phase 2 fixes.
- **UNVERIFIED — whether the damage diff can bound a particle system at all.** A system
  whose live count changes each frame changes the entry count, which the diff treats as
  a full redraw. Phase 3 establishes whether a per-system bounding rectangle is
  reachable without special-casing the diff.

## 3. Design Overview

Three pieces, and the first one decides the other two:

1. **Measure** (Phase 1) — the expansion's per-frame cost at 100 / 1,000 / 4,000
   particles, software and GPU, with and without the geometry cache in the path.
2. **Fix the cache** (Phase 2) — only if Phase 1 says it matters, and in the shape
   Phase 1's numbers indicate. The leading candidate is to **bypass the cache for
   particle entries**: a particle's geometry is built once, used once, and never
   recurs, which is the exact opposite of what a cache is for. Bypassing means the 256
   entries keep serving the scene's *real* geometry instead of being churned.
3. **Bound the damage** (Phase 3) — give a particle system a single damage rectangle
   covering its live particles, so a moving system repaints its own area rather than
   the frame.

**Where design uncertainty concentrates: all of it is in Phase 1.** This letter is
deliberately measurement-first; its Phases 2 and 3 are written as conditional on what
Phase 1 finds, and either may legitimately become a no-op with the measurement as its
evidence.

**Byte-identity is a side-condition here, not the gate.** A particle-free scene must
produce byte-identical `.ncode` and identical goldens — that is how "we did not disturb
the existing cache" is proved. The particle path's gate is runtime behaviour and
measured numbers.

### Rejected alternatives

- **Raising `CANVAS_MAX_FRAME_ITEMS` pre-emptively.** Rejected: see Non-goals. The
  buffer is `CANVAS_MAX_FRAME_ITEMS * ITEM_BLOCK_SIZE`, so the constant is a memory
  decision, and it should be made with a measured particle count beside it.
- **Growing the geometry cache to hold a whole system.** Rejected: a 4,000-particle
  system would need a 4,000-entry cache of 47-float headers, and every entry is used
  exactly once. That is not a cache, it is a per-frame arena with eviction overhead
  bolted on.
- **Special-casing the damage diff for particles.** Held in reserve for Phase 3, not
  chosen up front: if a per-system bounding rectangle can be expressed as an ordinary
  damage contribution, the diff stays general.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as
> the work it describes. Mark a task moot with `- [x] ~~text~~ — moot: <evidence>`.
> **An unticked box means NOT DONE.**

### Phase 1 — Measure the expansion (uncertainty first)

Nothing is optimized before this exists. The numbers set both remaining phases, and
either may end as "no change needed, here is the measurement".

- [ ] Add a particle case to the canvas benchmark path, or a standalone program under
      `benchmark/mfb/src/`, presenting one system at 100 / 1,000 / 4,000 particles with
      a `Circle` template, `time` advanced 120 frames.
- [ ] Record, per count: frames rendered, frames skipped, wall time per frame, and
      whether the frame took the GPU or declined to software — from the
      `MFB_CANVAS_STATS` line, which is how a headless test can see graphics-thread
      state at all.
- [ ] Measure the same three counts with the geometry cache bypassed for particle
      entries (a scratch patch, not landed), to separate cache cost from expansion cost.
- [ ] Record all of it in this plan as a table, each row with its command, ranked on
      `min` per `benchmark/RANKING.md` §1.1.

Acceptance: the table exists in this file, with commands, and answers three questions
in numbers: the particle count reachable at 60 fps on the GPU; the count at which the
frame declines to software; and the share of per-frame cost attributable to the
geometry cache.
  Check: the table is present and every row names the command that produced it
  (est. 20 min — longer than 10 because it is three counts × two cache configurations ×
  120 frames, and a scoped single run would not separate cache cost from expansion cost,
  which is the whole question).
Commit: —

### Phase 2 — Keep particles out of the geometry cache (conditional on Phase 1)

Land only if Phase 1 attributes material cost to the cache. If it does not, tick this
phase moot with the measurement as evidence.

- [ ] Route particle entries around `__canvas_geometryFor`'s cache insert — built into
      `__CANVAS_GEO_DATA` and registered in `__CANVAS_GEO_LIVE` as today, but never
      added to `__CANVAS_GEO_HASHES`/`OFFSETS`/`COUNTS`/`LASTUSED`.
- [ ] Confirm `__CANVAS_GEO_LIVE` still pins every particle offset for the whole frame
      — the glyph-eviction hazard `helper_render.rs:558` documents applies unchanged,
      and a particle whose glyph indices were renumbered mid-frame is the same silent
      corruption.
- [ ] Tests: a scene mixing a 1,000-particle system with 10 ordinary cached items;
      assert via `canvas::groupStats`/scene-hash reporting that the ordinary items stay
      resident across frames.

Acceptance: with a 1,000-particle system present, a 10-item static sub-scene keeps its
cache entries across 60 frames — i.e. the particle system no longer evicts the scene's
real geometry.
  Check: `cargo test --test rt_canvas_particles cache` → pass (est. 5 min).
Commit: —

### Phase 3 — Bound a particle system's damage (conditional on Phase 1)

- [ ] Determine, from Phase 1's stats, whether a particle scene currently reports full
      redraws every frame. If it does not, tick this phase moot with the evidence.
- [ ] Give the expansion a per-system bounding rectangle over its live particles, fed
      into the damage pass the way an ordinary item's bounds are
      (`helper_damage.rs:243`).
- [ ] Tests: a scene with a static background and one particle system confined to a
      quarter of the surface; assert the reported damage rectangle does not cover the
      full surface.

Acceptance: a particle system occupying a quarter of the surface, over a static
background, reports partial damage rather than a full redraw — visible as a non-zero
`partial` count in `MFB_CANVAS_STATS`.
  Check: `cargo test --test rt_canvas_particles damage` → pass (est. 5 min).
Commit: —

### Phase 4 — Write the cost model down

- [ ] Fold Phase 1's numbers into `mfb man canvas` prose for `ParticleSystem`: the
      sustainable count, the decline threshold, and the fact that a `Text` template
      costs one quad per glyph per particle.
- [ ] Record in this plan whether the measured numbers argue for a later letter raising
      `CANVAS_MAX_FRAME_ITEMS`, and if so with what number.

Acceptance: `mfb man canvas types` prose for `ParticleSystem` states a measured
particle count, not an adjective.
  Check: `cargo run -- man canvas types | grep -A5 ParticleSystem` → contains a numeral
  (est. 2 min).
Commit: —

## Validation Plan

- **Tests:** cache-residency and damage cases in `tests/canvas/rt_canvas_particles.rs`;
  the existing particle golden must not move.
- **Coverage check:** `scripts/coverage-check.sh` — any new branch in
  `helper_geometry.rs` must be in the denominator.
- **Runtime proof:** the Phase 1 benchmark program, re-run after Phases 2 and 3, showing
  the per-frame cost moved in the direction the fix predicted.
- **Doc sync:** `mfb man canvas` (Phase 4); if Phase 2 changes cache behaviour, a note
  in `helper_geometry.rs`'s own doc comment stating that particle entries are
  deliberately not cached and why.
- **Final gate (run ONCE, after the last phase):** `scripts/test-accept.sh <mfb-exe> <actual-output-dir>` (both arguments are required; the script exits 2 without them).

## Open Decisions

- **Whether particles bypass the cache or get a dedicated arena** — bypass recommended
  (§3). A dedicated per-frame arena is cleaner in principle but adds a second lifetime
  to reason about next to `__CANVAS_GEO_LIVE`, which is already subtle.
- **Whether the damage bound is per-system or per-particle** — per-system recommended.
  Per-particle is tighter but produces N rectangles the diff must union, and the diff's
  cost is already proportional to entry count.

## Corrections

<Filled in DURING execution.>

## Summary

This letter is a measurement with two conditional repairs attached. The risk is not
that the fixes are hard — it is the temptation to skip Phase 1 and optimize the cache
because thrashing *sounds* expensive. The cache miss rebuilds 47 floats; that may well
be cheaper than the eviction bookkeeping around it, or than the `DrawItem` allocation
per particle, and only the measurement can say.

`CANVAS_MAX_FRAME_ITEMS` is left alone on purpose. It is a memory decision that
deserves a measured particle count standing next to it.
