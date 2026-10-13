# plan-150-B: Particle systems — geometry records, damage, and the cost model

Last updated: 2026-10-13 (executed; see Measured and Corrections. Re-derived 2026-10-11
against the completed plan-172 and the bug-686 geometry cache; see "Re-derivation record")
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

- `planning/completed/plan-150-A-canvas-particles-types-and-expansion.md` — the type set, the
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
| plan-150-A complete and archived | `ls planning/plan-150-A-*` → no matches, and `ls planning/completed/plan-150-A-*` → present | MET (2026-10-13: no matches; `planning/completed/plan-150-A-canvas-particles-types-and-expansion.md`) |
| The particle golden renders | `cargo test --test rt_canvas_golden particles` → pass | MET (2026-10-13: "1 passed; 0 failed") |

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

- [x] Add a standalone program under `tools/particle-bench/` with a README (a
      benchmark, so `tools/`, not `scripts/`), presenting one system at 100 / 1,000 /
      10,000 / 50,000 particles with a `Circle` template, `time` advanced 120 frames.
      — `src/main.mfb`, `bench.sh` (`sweep`, `row`), `README.md`.
- [x] Record, per count: frames rendered, frames skipped, wall time per frame, and
      whether the frame took the GPU or declined to software — from
      `canvas::getFrameStats` and the `MFB_CANVAS_STATS` line, which is how a headless
      test can see graphics-thread state at all.
- [x] Split the per-frame cost three ways with scratch patches (not landed): the model
      alone (expansion that appends nothing); the model plus the item build and
      `__canvas_hashItem`; and the whole path. The difference between the last two is
      the geometry records. — in a detached checkout, `/tmp/p150/scratch`, patched by
      `/tmp/p150/scratch_patch.py model|item|whole`; a fourth variant
      (`scratch_patch2.py build`) built each record natively and threw it away, which
      split the records' share again. "The split", below.
- [x] Measure the same counts with ~~a `Picture` template and~~ a `Text` template, and with
      a `Polygon` emitter, whose spawn is a rejection loop (plan-150-A §5). — a `Picture`
      cannot be a template (`item` is an `AutoShape`, plan-150-A Corrections 3; pinned by
      `tests/syntax/canvas/canvas_particle_template_invalid`). In its place, a template
      the plan did not name and the numbers needed: a rectangle that spins, which is a
      TRANSFORMED particle and takes a different geometry builder.
- [x] Measure the same counts with the program also calling `canvas::pollMouse` every
      frame, to prove the worker's hit walk does not scale with the particle count.
- [x] Record all of it in this plan as a table, each row with its command, ranked on
      `min` per `benchmark/RANKING.md` §1.1. — "Measured", below.

Acceptance: the table exists in this file, with commands, and answers four questions
in numbers: the particle count reachable at 60 fps on the GPU; the count at which the
frame declines to software; the share of per-frame cost attributable to the geometry
records; and that polling the mouse adds no per-particle cost.
  Check: the table is present and every row names the command that produced it
  (est. 30 min, guess — longer than 10 because it is four counts × three cost splits ×
  120 frames, and a scoped single run would not separate record cost from expansion
  cost, which is the whole question).
  Result (2026-10-13): the tables under "Measured" answer the four questions — 60 fps
  to about 7,000 plain particles (about 8,000 after Phase 2); no count at which a frame
  declines; the records 28% of the expansion for a plain particle and 65% for a turned
  one; polling the mouse free.
Commit: 8a3a0557b

## Measured

All on an Apple M2 Max, macOS, headless, 2026-10-13, every row 120 presents under
`MFB_CANVAS_SYNC=1` (so the time around a present is a whole frame), ranked on the least
frame (`benchmark/RANKING.md` §1.1). The command for a row is

    bash tools/particle-bench/bench.sh target/debug/mfb row COUNT [NAME=value ...]

with the knobs in the row's second column, and `sweep 120` runs the first five blocks.
`gpu` and `software` are the normal build with and without `MFB_CANVAS_GPU=1`; the
other columns are the `--debug` build's stats line. In every row `frames=120
skipped=0`.

### particles/frame, before Phase 2 (the compiler at `24721092c`)

| particles/frame | knobs | gpu min µs | software min µs | gpuFrames | blocks | offsets / draws / render, ms per frame |
|---|---|---|---|---|---|---|
| 100 | — | 1,758 | 1,280 | 120 | 100 | 0.28 / 0.00 / 2.26 |
| 1,000 | — | 3,741 | 6,638 | 120 | 1,000 | 2.60 / 0.03 / 3.06 |
| 6,000 | — | 14,402 | 37,085 | 120 | 6,000 | 15.56 / 0.36 / 4.03 |
| 7,000 | — | 16,443 | 42,837 | 120 | 7,000 | 18.23 / 0.39 / 4.44 |
| 8,000 | — | 18,682 | 48,689 | 120 | 8,000 | 20.80 / 0.49 / 4.74 |
| 10,000 | — | 23,114 | 62,213 | 120 | 10,000 | 26.55 / 0.66 / 5.88 |
| 50,000 | — | 111,950 | 292,277 | 120 | 50,000 | 135.13 / 5.09 / 15.88 |
| 70,000 | — | 155,197 | 389,077 | 120 | 70,000 | 181.06 / 7.26 / 24.48 |
| 140,000 | — | 306,032 | 759,726 | 120 | 140,000 | 362.11 / 17.48 / 36.68 |
| 10,000 | `TINT=white MOTION=still` | 19,012 | 56,774 | 120 | 10,000 | 20.81 / 0.57 / 5.37 |
| 100 | `TEMPLATE=rect MOTION=spinning` | 2,139 | 11,811 | 120 | 100 | 0.60 / 0.00 / 2.66 |
| 1,000 | `TEMPLATE=rect MOTION=spinning` | 6,326 | 112,204 | 120 | 1,000 | 5.52 / 0.03 / 4.30 |
| 3,000 | `TEMPLATE=rect MOTION=spinning` | 14,807 | 336,534 | 120 | 3,000 | 17.08 / 0.16 / 7.56 |
| 4,000 | `TEMPLATE=rect MOTION=spinning` | 18,948 | 444,768 | 120 | 4,000 | 22.67 / 0.18 / 6.86 |
| 10,000 | `TEMPLATE=rect MOTION=spinning` | 43,903 | 1,112,797 | 120 | 10,000 | 56.60 / 0.57 / 15.17 |
| 50,000 | `TEMPLATE=rect MOTION=spinning` | 211,523 | 5,551,855 | 120 | 50,000 | 287.60 / 5.33 / 40.94 |
| 100 | `TEMPLATE=text` (Arial, `AB`) | 40,654 | 40,275 | 120 | 100 | 40.19 / 0.03 / 2.83 |
| 1,000 | `TEMPLATE=text` | 389,317 | 395,840 | 120 | 1,000 | 397.62 / 0.20 / 5.13 |
| 10,000 | `TEMPLATE=text` | 3,875,595 | 3,918,791 | 120 | 10,000 | 3938.18 / 2.59 / 21.39 |
| 1,000 | `TEMPLATE=text FONT=`the 304-byte fixture | 7,893 | 6,244 | 120 | 1,000 | 10.55 / 0.17 / 3.89 |
| 100 | `EMITTER=polygon` | 1,757 | 1,286 | 120 | 100 | 0.32 / 0.00 / 2.77 |
| 1,000 | `EMITTER=polygon` | 3,997 | 6,861 | 120 | 1,000 | 3.02 / 0.03 / 2.90 |
| 10,000 | `EMITTER=polygon` | 25,179 | 60,417 | 120 | 10,000 | 29.65 / 0.37 / 5.65 |
| 50,000 | `EMITTER=polygon` | 117,813 | 282,870 | 120 | 50,000 | 150.69 / 3.83 / 15.85 |
| 100 | `POLL=1` | 1,757 | 1,231 | 120 | 100 | 0.27 / 0.00 / 2.67 |
| 1,000 | `POLL=1` | 3,650 | 6,651 | 120 | 1,000 | 2.52 / 0.03 / 3.04 |
| 10,000 | `POLL=1` | 21,533 | 59,625 | 120 | 10,000 | 24.98 / 0.38 / 5.17 |
| 50,000 | `POLL=1` | 102,647 | 276,929 | 120 | 50,000 | 127.18 / 3.59 / 13.65 |

The four questions:

1. **60 fps on the GPU** (16,667 µs): 7,000 plain particles (16,443 µs); 8,000 is over
   (18,682). For a particle that is turned, between 3,000 (14,807) and 4,000 (18,948).
2. **The count at which a frame declines to software: there is none.** 70,000 and
   140,000 particles are past the 65,536 blocks a pass holds and every one of their 120
   frames was a GPU frame (`gpuFrames=120`): a frame whose blocks do not fit is drawn in
   passes (bug-765). This letter's premise that a system "exceeds a frame cap [and]
   declines the frame to software" is false as of that fix — Corrections 1.
3. **The geometry records' share** — "The split".
4. **Polling the mouse adds no per-particle cost**: every `POLL=1` row is at or under
   its row without (21,533 against 23,114 µs at 10,000).

A `Polygon` emitter costs a star-shaped one about 9% at 10,000 (25,179 against 23,114)
and 5% at 50,000: its rejection loop, about half of whose candidates miss.

A `Text` template is 389 µs a particle with a system font and 7.9 µs with a 304-byte
one: the cost is the font FILE's size, copied on every text record built. That is not
this plan's to fix and is not a property of particles: bug-1091.

### The split (10,000 particles, the `--debug` phase timer, ms per frame)

`offsets` is the scene walk; with the expansion patched to stop early, what is left is
that much of it.

| The expansion… | plain circle | spinning rectangle |
|---|---|---|
| runs the model for every slot and appends nothing | 7.32 | 6.39 |
| …and builds each particle's item and hashes it | 18.98 | 19.88 |
| …and builds each record natively, then discards it (`geoBuild`) | 20.85 | (declined: transformed) |
| the whole path | 26.23 | 56.33 |

| Share of the expansion | plain circle | spinning rectangle |
|---|---|---|
| the model | 7.32 ms, 28% | 6.39 ms, 11% |
| the item built and hashed | 11.66 ms, 44% | 13.49 ms, 24% |
| the geometry record | 7.25 ms, 28% | 36.45 ms, 65% |
| — of which the record's own build | 1.87 ms | — |
| — of which the cache around it (lookup, slot, index, the next frame's drop) | 5.38 ms | — |

The whole frame at those counts is `offsets + draws + render`: 31.7 ms and 69.1 ms in
the `--debug` build, so the expansion is 83% and 82% of a frame and everything Metal
does is the rest.

### What a draw entry per particle would cost (10,000 draws of one block each)

A probe, not landed (`/tmp/p150/gdraws`): 10,000 `Group` nodes each drawing a one-circle
group under its own matrix, which is the draw list "the template under a per-particle
matrix" would produce. Per frame, `--debug` phase timers: draw list 5.7 ms and render
19 ms, against 0.6 and 4.9 for the same 10,000 circles as blocks of one draw. So a draw
entry costs about 1.9 µs on Metal where a block of an instanced draw costs about 0.5.

### Phase 2 — Stop building a record per particle (conditional on Phase 1)

Land only if Phase 1 attributes material cost to the geometry records. If it does not,
tick this phase moot with the measurement as evidence.

- [x] Choose between §3's two candidates from Phase 1's split, and record the choice and
      the numbers behind it here before writing code. — **the uncached record, for a
      particle the native builder takes; the draw-entry matrix is not landed.**
      *The matrix* saves a plain particle its item, hash and record — 18.9 ms at 10,000
      — and costs a draw entry each: 19 ms more render and 5 ms more draw list,
      measured. It nets nothing on the GPU for the common particle, it turns every
      system into the many-draws frame that is the renderer's worst case, and as
      specified it serves only a system with no tint. It would pay for a turned
      particle (36.45 ms of record against 24), and that case is left as it is, with
      the number, rather than given a second expansion path.
      *The uncached record* cannot touch the model or the item, and of the record's
      7.25 ms it leaves the 1.87 ms build. Its ceiling is the other 5.38 ms: 20% of the
      expansion, 17% of the frame.
- [x] ~~If the draw-entry matrix: an untinted particle is appended as the template's own
      record under `__canvas_composeGroup(gdx, gdy, mat, P, M)`. The software walk, the
      GPU walk and the damage hash already read a per-entry matrix; confirm each with a
      test rather than by reading.~~ — moot: not chosen, on the measurement above
      (1.9 µs a draw entry on Metal against the 1.9 µs a plain particle's item, hash and
      record cost together).
- [x] If the uncached record: route particle entries around the table insert, and
      confirm the record stays valid for the whole frame — the frame-stamp rule
      (`.ai/canvas-threading.md` §14) is what protects an offset today, and a record
      outside the table must be protected by something that is stated. —
      `__canvas_particleRecord` appends the native builder's record to
      `__CANVAS_GEO_DATA` and uses its offset: no lookup, no slot, no index entry.
      What protects it is the rule that protects every offset, now written at the
      site and in `.ai/canvas-threading.md` §14: no float of that list moves or is
      dropped inside a frame; `canvas::geoBeginFrame` is the one place that does, and
      there a float no slot used is stale and goes with the rest
      (`func_geo_cache.rs:lower_geo_begin_frame`: `stale = count − used`). A particle
      the native builder declines — turned, scaled, a gradient, text — takes the cache
      as before. `a_plain_particles_record_is_not_cached`: `entries=10` beside 1,000
      particles, `blocks=1010`, and `floats=` flat from the sixth frame to the twelfth.
- [x] A `Text` template's glyphs: confirm the glyph cache's pin (`.ai/canvas-threading.md`
      §14, "Glyph eviction pins only what the frame in progress uses") still covers a
      particle's run under the chosen route. — it is not on the route: the native
      builder declines text, so a text particle is a cached, stamped slot exactly as
      before and its glyphs are pinned by it.
      `a_text_particles_record_is_still_cached_and_its_glyphs_drawn`: 40 text particles,
      `entries=40`, all drawn.
- [x] Tests: the particle golden and letter A's group-node equivalence case must pass
      **unedited**; a scene mixing a 1,000-particle system with 10 ordinary items draws
      the 10 identically with and without the system. — `git diff 0a7733866 --
      tests/golden/canvas/particles.png` is empty and no line of the equivalence tests
      changed; `ordinary_items_draw_the_same_beside_a_system` compares every pixel of
      the items' half.

Acceptance: Phase 1's benchmark, re-run, shows the record share of the per-frame cost
moved in the direction the choice predicted, and no particle golden moved.
  Check: `cargo test --test rt_canvas_particles --test rt_canvas_golden` → pass, goldens
  unmodified (est. 8 min, guess), plus the re-run benchmark row recorded here.
  Result (2026-10-13): particles "29 passed", golden "28 passed", and
  `--test rt_canvas_damage` "11 passed", 0 failed, goldens unmodified. Re-run rows, the
  same commands:

  | particles/frame | gpu min µs before → after | offsets ms before → after |
  |---|---|---|
  | 10,000 | 23,114 → 20,557 (−11%) | 26.55 → 22.32 |
  | 50,000 | 111,950 → 94,774 (−15%) | 135.13 → 110.67 |

  And the rows the manual quotes, on the compiler with Phase 2 in it:

  | particles/frame | knobs | gpu min µs | software min µs | gpuFrames |
  |---|---|---|---|---|
  | 100 | — | 1,708 | 1,300 | 120 |
  | 1,000 | — | 3,522 | 6,829 | 120 |
  | 8,000 | — | 16,844 | 49,591 | 120 |
  | 9,000 | — | 18,381 | 55,270 | 120 |
  | 10,000 | — | 20,557 | 61,170 | 120 |
  | 50,000 | — | 94,774 | 278,299 | 120 |
  | 140,000 | — | 275,136 | 745,625 | 120 |
  | 10,000 | `TEMPLATE=rect MOTION=spinning` | 43,837 | 1,121,349 | 120 |

  60 fps (16,667 µs) is now just short of 8,000 plain particles (16,844 µs; 7,000 was
  16,443 before the change). A turned particle is unchanged, as predicted: its record
  is not one the native builder makes.

  The record's share of a plain particle's expansion: 7.25 of 26.23 ms (28%) before,
  22.32 − 18.98 = 3.34 of 22.32 (15%) after — the direction the choice predicted, and
  4.2 of the 5.38 ms ceiling.
Commit: 37c783297

### Phase 3 — Bound a particle system's damage (conditional on Phase 1)

- [x] Determine, from Phase 1's stats under `MFB_CANVAS_DAMAGE=1`, whether a particle
      scene reports a full redraw every frame. If it does not, tick this phase moot with
      the evidence. — it does, when the live count changes, and only then. A probe of
      30 presents (a static background and one system; `/tmp/p150/dmg`, run with
      `MFB_CANVAS_DAMAGE=1 MFB_CANVAS_STATS=…`, counting `damage=0,0,900,640` lines):
      a `Continuous` stream filling its pool → **28 of 30 frames full**; a `Burst` whose
      60 particles all stay alive → 1 of 30 (the first), the other 29 partial. So the
      phase is not moot.
- [x] Give the expansion a per-system bounding rectangle over its live particles, fed
      into the damage pass the way an ordinary item's bounds are (`helper_damage.rs`).
      The bounds must go through the entry's offset and matrix, as every bound there
      does since plan-172-G. — a frame that expanded a system is diffed by UNIT
      (`helper_particles.rs:DAMAGE`): each system one unit, its hash folded from its
      entries' and its rectangle the union of `__canvas_drawBound` over them; every
      other entry a unit by itself. A frame with no system takes the entry diff
      untouched (Corrections 3).
- [x] Tests: a scene with a static background and one particle system confined to a
      quarter of the surface; assert the reported damage rectangle does not cover the
      full surface. — `damage_of_a_system_is_bounded_by_its_particles`: `partial=29` of
      30, every rectangle inside the quarter, and the picture thirty partial redraws
      leave `compare_exact`s a full redraw's.
      `damage_of_an_item_beside_a_system_is_the_items`: an ordinary item moving beside
      a still system damages only its own two places.

Acceptance: under `MFB_CANVAS_DAMAGE=1`, a particle system occupying a quarter of the
surface, over a static background, reports partial damage rather than a full redraw.
  Check: `cargo test --test rt_canvas_particles damage` → pass (est. 5 min, guess).
  Result (2026-10-13): "2 passed; 0 failed" in 101.31 s; and the suite that holds the
  entry diff, `cargo test --test rt_canvas_damage` → "11 passed; 0 failed", unedited.
Commit: 37c783297 (with Phase 2: Corrections 4)

### Phase 4 — Write the cost model down

- [x] Fold Phase 1's numbers into `mfb man canvas` prose for `ParticleSystem`: the
      sustainable count, the decline threshold, the fact that a `Text` template costs
      one quad per glyph per particle, and what a `Polygon` emitter costs over a
      `Circle` one. Developer vocabulary only (`.ai/man-content.md`). — in
      `ParticleSystem`'s description: the times at 1,000 / 10,000 / 50,000, 60 fps to
      about 8,000 (3,000 turned or scaled), that there is NO threshold (Corrections 1),
      a `Polygon` emitter's tenth, and a `Text` shape's real cost, which is the font
      file's size and not its glyphs (Corrections 5).
- [x] Record in this plan whether the measured numbers argue for a later letter raising
      a frame cap, and if so with what number. — they do not: Corrections 1. The cap is
      the size of a pass, a frame past it is drawn in passes (`gpuFrames=120` of 120 at
      140,000), and at 65,536 particles the frame is already about 125 ms of expansion —
      the buffer is not what limits a system.
- [x] Record plan-150-A's Open Decision 1 as settled, with the measurement that settled
      it. — settled as recommended, composed into the item: a draw entry per particle
      costs about 1.9 µs on Metal (10,000 single-block draws: 19 ms render and 5.7 ms of
      draw list, against 4.9 and 0.6 as blocks of one draw), which is what the entry
      would have saved a plain particle. Written into
      `planning/completed/plan-150-A-…md`'s Open Decisions.
- [x] Give plan-152's item C6 the numbers it asks for. — a line under C6 in
      `planning/plan-152-canvas-program-side-cost-spike.md`: 10,000 at 20.6 ms and
      50,000 at 94.8 ms a frame on Metal. The box is that plan's to tick.

Acceptance: `mfb man canvas types` prose for `ParticleSystem` states a measured
particle count, not an adjective.
  Check: `cargo run -- man canvas types | grep -A5 ParticleSystem` → contains a numeral
  (est. 2 min, guess).
  Result (2026-10-13): `target/debug/mfb man canvas types | grep -A5 ParticleSystem |
  grep -c '[0-9]'` → 3, and the rendered page reads "3.5 milliseconds a frame at 1,000
  particles, 21 at 10,000 and 95 at 50,000 … 60 frames a second to about 8,000".
Commit: a4ccd3bd9

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

1. **No frame declines to software for its particle count** (2026-10-13). This letter
   was written to find "the count at which the frame declines to software", and its
   Non-goals say a system that exceeds a frame cap declines the frame. Since bug-765 a
   frame whose blocks do not fit one pass is drawn in several: `bench.sh … row 70000`
   and `row 140000` report `gpuFrames=120` of 120. The 65,536 is the size of a pass.
   The question the cost model answers is therefore the frame TIME at a count, and the
   manual says so. It also settles "whether the measured numbers argue for a later
   letter raising a frame cap": they do not — the cap bounds a pass, not a frame, and
   at the counts where a second pass begins the frame is already 150 ms of expansion.
2. **`item` is an `AutoShape`, so there is no `Picture` template to measure**
   (plan-150-A Corrections 3). The row is replaced by one the plan lacked: a rectangle
   that spins. It mattered — a turned particle's record is 65% of its expansion against
   a plain one's 28%, because a shape with a transform in effect is not built natively.
3. **The damage diff was special-cased, by unit** (2026-10-13). §3 held "special-casing
   the damage diff for particles" in reserve, hoping "a per-system bounding rectangle
   can be expressed as an ordinary damage contribution". It cannot be, as the diff
   stood: the diff pairs ENTRIES by index and a system's entry count is its live count.
   So a frame that expanded a system is diffed by unit — a system one unit, every other
   entry a unit by itself — and a frame with no system takes the entry diff, untouched.
   Measured before: 28 of 30 frames full for a stream filling its pool; after: 1.
4. **Phase 3 was built before Phase 2**, while Phase 1's long rows ran: the two are
   independent (damage is a `--debug` mode; the record path is not), and they landed in
   one commit because both edit `helper_particles.rs`.
5. **A `Text` particle costs the font file's size, not its glyph count** (2026-10-13).
   §Phase 4 expected to tell users "a `Text` template costs one quad per glyph per
   particle". It does cost that on the GPU, and it is not what a user will feel: 389 µs
   a particle with Arial against 7.9 µs with a 304-byte font, because every text record
   built copies the font's bytes. Filed as bug-1091 — it is every rebuilt `Text` item's
   cost, not a particle's — and the manual gives the measured number and the reason.
6. **What "the item built and hashed" costs was not on this letter's list, and is the
   largest part** — 44% of a plain particle's expansion. Neither candidate of §3 reaches
   it without a draw entry per particle. Recorded for whoever comes next: the per-
   particle `WITH` of the template record and its paint is where a further gain is.
7. **The final gate and the coverage check** (2026-10-13). `scripts/test-accept.sh` runs
   once for the whole split plan, after letter C (plan-150-A Corrections 12).
   `scripts/coverage-check.sh` needs the whole instrumented suite; the scoped answer to
   its question is that this letter's new branches are all MFBASIC source inside
   `helper_particles.rs`'s raw strings (and two hook calls each in `helper_damage.rs`
   and `helper_render.rs`'s), which are not Rust lines and are in no denominator; the
   Rust that registers them is run by every registry test. What covers the MFBASIC is
   `rt_canvas_particles` — the damage, uncached-record and mixed-scene cases.

## Summary

This letter is a measurement with two conditional repairs attached. The risk is not
that the fixes are hard — it is the temptation to skip Phase 1 and optimize the record
path because building a thousand records a frame *sounds* expensive. A miss builds 67
floats; that may well be cheaper than the table work around it, or than the `DrawItem`
built and hashed per particle, and only the measurement can say.

The frame caps are left alone on purpose. They are a memory decision that deserves a
measured particle count standing next to it.
