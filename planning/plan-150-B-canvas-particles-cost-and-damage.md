# plan-150-B: Particle systems — geometry records, damage, and the cost model

Last updated: 2026-10-11 (re-derived against the completed plan-172 and the bug-686
geometry cache; see "Re-derivation record")
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
system sustains at 60 fps and the count at which a frame declines to software; and,
under `MFB_CANVAS_DAMAGE`, a particle system that moves repaints only where it moved,
rather than forcing a full redraw every frame.

References:

- `planning/plan-150-A-canvas-particles-types-and-expansion.md` — the type set, the
  analytic model, and the expansion this letter measures. Its Open Decision 1 (where the
  per-particle affine lives) is the choice this letter's numbers settle.
- `.ai/canvas-threading.md` §14 — the geometry cache as it is now: keyed by a 62-bit
  item hash, nothing evicted inside a frame, unused slots dropped at
  `canvas::geoBeginFrame`. §4 — damage is off unless `MFB_CANVAS_DAMAGE` is set.
- `src/codegen/builtins/canvas/helper_damage.rs` — the damage diff, and the sites that
  read the per-entry offsets and matrices.
- `planning/plan-152-canvas-program-side-cost-spike.md` item C6 — asks for this
  measurement at 10k and 50k particles.
- `benchmark/RANKING.md` §1.1 — why `min` and not `median` is the column to rank on.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-150-A complete and archived | `ls planning/plan-150-A-*` → no matches, and `ls planning/completed/plan-150-A-*` → present | NOT MET (2026-10-11) |
| The particle golden renders | `cargo test --test rt_canvas_golden particles` → pass | NOT MET (2026-10-11) |

If plan-150-A is not complete, this letter cannot start, full stop.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again before
> you decide to stop. If you stop, report the status of *all* prerequisites.

### Re-derivation record (2026-10-11)

This letter was written against a 256-entry LRU geometry cache and a 4,096-quad frame
cap. Neither exists at `af940da94`:

- **The cache has no fixed capacity and evicts nothing mid-frame** (bug-686).
  `grep -rn 'GEO_CAPACITY\|__CANVAS_GEO_LIVE' src` → 0. The old goal — "a particle
  system does not evict unrelated scene geometry" — is met by construction. What is
  left to measure is the cost of building N records a frame and dropping them the next.
- **The frame cap is 65,536 blocks per backend** (`VULKAN_MAX_FRAME_ITEMS`,
  `METAL_MAX_FRAME_ITEMS`), not 4,096, so the measured counts go higher.
- **Damage is off by default.** An ordinary frame is a full redraw; the damage diff runs
  only under `MFB_CANVAS_DAMAGE`. Phase 3 is about that mode only.
- **plan-172-G gave every draw entry a matrix**, which is a second way to place a
  particle and the leading candidate for Phase 2.
- **The hit walk and the `AutoPaint` resolve are new per-frame costs** a particle scene
  must be shown not to pay.

## 1. Goal

- The per-frame cost of a particle system is **measured**, at four counts, and
  recorded in this plan as a table with the command that produced it.
- The share of that cost spent building and dropping per-particle geometry records is
  known, and reduced if it is material.
- Under `MFB_CANVAS_DAMAGE`, a moving particle system produces a damage rectangle
  bounded by the particles, not a full-frame redraw, whenever the rest of the scene is
  static.
- A particle scene costs the worker nothing when the mouse is polled: the hit walk
  skips a system (plan-150-A §6), and this letter measures that it does.

### Non-goals (explicit constraints)

- **`VULKAN_MAX_FRAME_ITEMS` / `METAL_MAX_FRAME_ITEMS` are not raised by this letter.**
  Measuring what the current 65,536 sustains is the whole point; changing a cap before
  the measurement exists would size a transport against a guess. If the measurement
  argues for a change, that is a separate letter with the number in hand.
- **No change to the analytic model.** Letter A's formulas are fixed; this letter is
  about what expansion costs, not what it computes.
- **No truncation.** A system that exceeds a frame cap declines the frame to software,
  as every other over-cap scene does. Clamping the particle count to fit would draw a
  *different scene* and report success — the exact lie the `*Renderable` predicates
  exist to prevent (`helper_render.rs:__canvas_metalRenderable`,
  `__canvas_vulkanRenderable`).
- **No change a particle-free scene can see.** Whatever this letter does to the
  geometry cache or the draw list must leave a particle-free scene's behaviour
  byte-identical.
- **No change to what a particle looks like.** If Phase 2 moves the per-particle affine
  from the item into the draw entry, the particle goldens must not move: letter A
  defines a shape particle as its group-node equivalent so that this is possible.

## 2. Current State

Every entry `__canvas_appendDraw` produces is resolved through
`__canvas_geometryFor(item, hash)`, which is the geometry cache
(`helper_geometry.rs`, kept by the native members of `func_geo_cache.rs`): an
open-addressing table of `(hash, slot)`, a hit trusted on the hash alone. Slots are
stamped with the frame that used them; `canvas::geoBeginFrame`, at the top of
`__canvas_sceneOffsets`, drops what the previous frame did not use and compacts the
float arena in place (`__CANVAS_GEO_COMPACTIONS`, `__CANVAS_GEO_GENERATIONS`).

Letter A's expansion gives every live particle a distinct hash — distinct `position`,
distinct `transform`, distinct tint — so a 1,000-particle system presents 1,000 new keys
**every frame**, and every one of the previous frame's 1,000 records is unused and
dropped. The cache does no caching for them: it is a per-frame arena with a table
insert, a table removal and a compaction attached.

The item hash is not free either: `__canvas_hashItem` folds the whole item, and letter
A calls it once per live particle.

The damage diff (`helper_damage.rs`) runs only under `MFB_CANVAS_DAMAGE`. It pairs the
expanded hash list with the expanded offset list by index and answers a full redraw
when the two are different lengths. A particle system's live count changes as particles
are born and die, so its entry count changes between frames — which is exactly the
condition that forces a full redraw.

### Measured populations

| What | Count | Command |
|---|---|---|
| Geometry header size | 67 floats | `grep -n 'LET __CANVAS_GEO_HEADER' src/codegen/builtins/canvas/helper_geometry.rs` → 67 (2026-10-11) |
| Frame block cap, per backend | 65,536 | `grep -n 'const VULKAN_MAX_FRAME_ITEMS\|const METAL_MAX_FRAME_ITEMS' src/codegen/runtime/canvas/mod.rs` (2026-10-11) |
| Item block size | 368 B | `grep -n 'const ITEM_BLOCK_SIZE' src/codegen/runtime/canvas/mod.rs` (2026-10-11) |
| Draw-entry width | 16 words | `grep -n 'const CANVAS_DRAW_ENTRY_WORDS' src/codegen/runtime/canvas/mod.rs` (2026-10-11) |
| Heavy Float3 pipeline throughput | 200,000 iterations in 19.808 ms min (Apple M2 Max, the baseline's host) | `grep -A2 '^vector:' benchmark/baseline/mfb-O1.log` → `math : 19.841, 19.851, 19.808, 19.957` (2026-10-11); the loop is `FOR k = 0 TO 199999` (`benchmark/mfb/src/vector.mfb`) |
| Per-frame cost of a particle system | **UNMEASURED** | Phase 1 measures it; the effort estimate above assumes it is not pathological |

The throughput row is the *only* basis for expecting the expansion to be affordable —
about 99 ns per heavy vector iteration (19.808 ms / 200,000). It is a bound on the
arithmetic, **not** a measurement of the expansion, which also builds a `DrawItem` per
particle, hashes it, and builds a 67-float record for it. Phase 1 exists because that
difference is unknown and it sets everything else.

### Verified properties

- **The cache is keyed on the full item hash.** VERIFIED by reading
  `.ai/canvas-threading.md` §14 and `__canvas_hashItem` in `helper_geometry.rs`: two
  particles differing only in `position` are different keys.
- **A particle system cannot evict another item's geometry.** VERIFIED by the same
  section: "Nothing is evicted inside a frame", and a slot is dropped only when the
  previous frame did not use it. A static item is used every frame.
- **UNVERIFIED — whether building N records a frame is expensive here.** A miss builds
  a 67-float header; the cost may be dominated by the table insert and the next frame's
  compaction, by `__canvas_hashItem`, or by the `DrawItem` built per particle. Phase 1
  measures before Phase 2 fixes.
- **UNVERIFIED — whether the damage diff can bound a particle system at all.** A system
  whose live count changes each frame changes the entry count, which the diff treats as
  a full redraw. Phase 3 establishes whether a per-system bounding rectangle is
  reachable without special-casing the diff.

## 3. Design Overview

Three pieces, and the first one decides the other two:

1. **Measure** (Phase 1) — the expansion's per-frame cost at 100 / 1,000 / 10,000 /
   50,000 particles, software and GPU, split into the model, the item build and hash,
   and the geometry records.
2. **Stop building a record per particle** (Phase 2) — only if Phase 1 says it matters,
   and in the shape Phase 1's numbers indicate. Two candidates:
   - **The draw-entry matrix.** A particle whose tint is the identity needs no item of
     its own: it is the template's one record drawn under a per-particle matrix, the
     mechanism plan-172-G built for groups (`__canvas_composeGroup`, `__canvas_noteEntry`).
     One record for the whole system, and one draw entry — one draw call — per particle.
   - **A record that is not cached.** The particle's record is built into the arena and
     used for the frame, but never entered in `__CANVAS_GEO_TABLE`, so there is no
     insert, no removal and nothing for the next frame to find stale.

   They are not exclusive: the first serves an untinted system, the second a tinted one.
3. **Bound the damage** (Phase 3) — give a particle system a single damage rectangle
   covering its live particles, so that under `MFB_CANVAS_DAMAGE` a moving system
   repaints its own area rather than the frame.

**Where design uncertainty concentrates: all of it is in Phase 1.** This letter is
deliberately measurement-first; its Phases 2 and 3 are written as conditional on what
Phase 1 finds, and either may legitimately become a no-op with the measurement as its
evidence.

**Byte-identity is a side-condition here, not the gate.** A particle-free scene must
produce byte-identical `.ncode` and identical goldens — that is how "we did not disturb
the existing cache" is proved. The particle path's gate is runtime behaviour and
measured numbers.

### Rejected alternatives

- **Raising a frame cap pre-emptively.** Rejected: see Non-goals. The buffer is the cap
  times `ITEM_BLOCK_SIZE` — 65,536 × 368 B, about 23 MiB per backend today — so the
  constant is a memory decision, and it should be made with a measured particle count
  beside it.
- **A second, particle-only geometry cache.** Rejected: every entry is used exactly
  once. That is not a cache, it is a per-frame arena with a table bolted on, which is
  what the particles have today.
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

- [ ] Add a standalone program under `tools/particle-bench/` with a README (a
      benchmark, so `tools/`, not `scripts/`), presenting one system at 100 / 1,000 /
      10,000 / 50,000 particles with a `Circle` template, `time` advanced 120 frames.
- [ ] Record, per count: frames rendered, frames skipped, wall time per frame, and
      whether the frame took the GPU or declined to software — from
      `canvas::getFrameStats` and the `MFB_CANVAS_STATS` line, which is how a headless
      test can see graphics-thread state at all.
- [ ] Split the per-frame cost three ways with scratch patches (not landed): the model
      alone (expansion that appends nothing); the model plus the item build and
      `__canvas_hashItem`; and the whole path. The difference between the last two is
      the geometry records.
- [ ] Measure the same counts with a `Picture` template and a `Text` template, and with
      a `Polygon` emitter, whose spawn is a rejection loop (plan-150-A §5).
- [ ] Measure the same counts with the program also calling `canvas::pollMouse` every
      frame, to prove the worker's hit walk does not scale with the particle count.
- [ ] Record all of it in this plan as a table, each row with its command, ranked on
      `min` per `benchmark/RANKING.md` §1.1.

Acceptance: the table exists in this file, with commands, and answers four questions
in numbers: the particle count reachable at 60 fps on the GPU; the count at which the
frame declines to software; the share of per-frame cost attributable to the geometry
records; and that polling the mouse adds no per-particle cost.
  Check: the table is present and every row names the command that produced it
  (est. 30 min, guess — longer than 10 because it is four counts × three cost splits ×
  120 frames, and a scoped single run would not separate record cost from expansion
  cost, which is the whole question).
Commit: —

### Phase 2 — Stop building a record per particle (conditional on Phase 1)

Land only if Phase 1 attributes material cost to the geometry records. If it does not,
tick this phase moot with the measurement as evidence.

- [ ] Choose between §3's two candidates from Phase 1's split, and record the choice and
      the numbers behind it here before writing code.
- [ ] If the draw-entry matrix: an untinted particle is appended as the template's own
      record under `__canvas_composeGroup(gdx, gdy, mat, P, M)`. The software walk, the
      GPU walk and the damage hash already read a per-entry matrix; confirm each with a
      test rather than by reading.
- [ ] If the uncached record: route particle entries around the table insert, and
      confirm the record stays valid for the whole frame — the frame-stamp rule
      (`.ai/canvas-threading.md` §14) is what protects an offset today, and a record
      outside the table must be protected by something that is stated.
- [ ] A `Text` template's glyphs: confirm the glyph cache's pin (`.ai/canvas-threading.md`
      §14, "Glyph eviction pins only what the frame in progress uses") still covers a
      particle's run under the chosen route.
- [ ] Tests: the particle golden and letter A's group-node equivalence case must pass
      **unedited**; a scene mixing a 1,000-particle system with 10 ordinary items draws
      the 10 identically with and without the system.

Acceptance: Phase 1's benchmark, re-run, shows the record share of the per-frame cost
moved in the direction the choice predicted, and no particle golden moved.
  Check: `cargo test --test rt_canvas_particles --test rt_canvas_golden` → pass, goldens
  unmodified (est. 8 min, guess), plus the re-run benchmark row recorded here.
Commit: —

### Phase 3 — Bound a particle system's damage (conditional on Phase 1)

- [ ] Determine, from Phase 1's stats under `MFB_CANVAS_DAMAGE=1`, whether a particle
      scene reports a full redraw every frame. If it does not, tick this phase moot with
      the evidence.
- [ ] Give the expansion a per-system bounding rectangle over its live particles, fed
      into the damage pass the way an ordinary item's bounds are (`helper_damage.rs`).
      The bounds must go through the entry's offset and matrix, as every bound there
      does since plan-172-G.
- [ ] Tests: a scene with a static background and one particle system confined to a
      quarter of the surface; assert the reported damage rectangle does not cover the
      full surface.

Acceptance: under `MFB_CANVAS_DAMAGE=1`, a particle system occupying a quarter of the
surface, over a static background, reports partial damage rather than a full redraw.
  Check: `cargo test --test rt_canvas_particles damage` → pass (est. 5 min, guess).
Commit: —

### Phase 4 — Write the cost model down

- [ ] Fold Phase 1's numbers into `mfb man canvas` prose for `ParticleSystem`: the
      sustainable count, the decline threshold, the fact that a `Text` template costs
      one quad per glyph per particle, and what a `Polygon` emitter costs over a
      `Circle` one. Developer vocabulary only (`.ai/man-content.md`).
- [ ] Record in this plan whether the measured numbers argue for a later letter raising
      a frame cap, and if so with what number.
- [ ] Record plan-150-A's Open Decision 1 as settled, with the measurement that settled
      it.
- [ ] Give plan-152's item C6 the numbers it asks for.

Acceptance: `mfb man canvas types` prose for `ParticleSystem` states a measured
particle count, not an adjective.
  Check: `cargo run -- man canvas types | grep -A5 ParticleSystem` → contains a numeral
  (est. 2 min, guess).
Commit: —

## Validation Plan

- **Tests:** mixed-scene and damage cases in `tests/canvas/rt_canvas_particles.rs`;
  the existing particle golden and the group-node equivalence case must not move.
- **Coverage check:** `scripts/coverage-check.sh` — any new branch in
  `helper_geometry.rs`, `helper_render.rs` or `helper_particles.rs` must be in the
  denominator.
- **Runtime proof:** the Phase 1 benchmark program, re-run after Phases 2 and 3, showing
  the per-frame cost moved in the direction the fix predicted.
- **Doc sync:** `mfb man canvas` (Phase 4); if Phase 2 changes how a particle reaches
  the cache or the draw list, `.ai/canvas-threading.md` §14 and
  `src/docs/spec/app/06_canvas.md`'s particle section say so, with the reason.
- **Final gate (run ONCE, after the last phase):** `scripts/test-accept.sh <mfb-exe> <actual-output-dir>` (both arguments are required; the script exits 2 without them).

## Open Decisions

- **Draw-entry matrix, uncached record, or both** — decided by Phase 1's split, not
  here. The matrix is the larger win where it applies (one record, no per-particle item
  or hash) and costs a draw call per particle; the uncached record keeps one draw call
  and still builds N records.
- **Whether the damage bound is per-system or per-particle** — per-system recommended.
  Per-particle is tighter but produces N rectangles the diff must union, and the diff's
  cost is already proportional to entry count.

## Corrections

<Filled in DURING execution.>

## Summary

This letter is a measurement with two conditional repairs attached. The risk is not
that the fixes are hard — it is the temptation to skip Phase 1 and optimize the record
path because building a thousand records a frame *sounds* expensive. A miss builds 67
floats; that may well be cheaper than the table work around it, or than the `DrawItem`
built and hashed per particle, and only the measurement can say.

The frame caps are left alone on purpose. They are a memory decision that deserves a
measured particle count standing next to it.
