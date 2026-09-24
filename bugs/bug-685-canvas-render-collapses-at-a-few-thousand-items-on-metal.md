# bug-685: the canvas render pipeline collapses to under 1 fps at a few thousand items, with the GPU selected and idle

Last updated: 2026-09-23
Effort: large (3h–1d)
Severity: HIGH
Class: Correctness

Status: Open
Regression Test: `tests/canvas/` — a frame-rate/geometry-work budget test; see Phases

On Metal, with `gpuSelected=TRUE`, a scene of 2,500 small polygons renders at
**0.47 fps**. The same scene at 2,000 polygons renders at **7.3 fps**. A 25%
increase in scene size costs a 15× drop in frame rate, and by 5,000 items the
figure is 0.2 fps — three frames in fifteen seconds.

Two things make this a pipeline defect rather than a hardware limit:

- **The GPU is not the bottleneck.** A few thousand flat-shaded polygons is
  trivial work for Metal; the time is going somewhere on the CPU before
  anything is submitted. Throughput actually runs *backwards*: 1,000 items
  sustains 19,000 item-updates/sec, while 5,000 items manages 1,000/sec.
- **A completely static scene is no faster than a moving one.** Presenting an
  identical 5,000-item scene every frame renders at the same 0.2 fps as one
  whose every coordinate changes. The geometry cache — whose entire purpose is
  to make the unchanged case free — delivers nothing at these sizes.

**The single correct behavior a fix produces:** frame rate degrades smoothly
and proportionally with scene size, and an unchanged scene costs
asymptotically nothing to re-present. A few thousand simple items is an
interactive frame rate, not a slideshow.

The user-visible symptom that prompted this: `examples/wind` presents ~5,750
line segments and repaints roughly once every 5–30 seconds, so its keyboard
controls appear dead — the keys are read and the view state changes, but the
repaint never lands.

References:

- `src/codegen/builtins/canvas/helper_geometry.rs:108`
  `__CANVAS_GEO_CAPACITY = 256` — the cache capacity, against scenes of
  thousands.
- `src/codegen/builtins/canvas/helper_geometry.rs:944` `__canvas_geoEvict`,
  `:968` `__canvas_geometryFor` — the per-item probe/evict/build path.
- `bugs/completed/bug-682-*` (arena growth) and `bugs/completed/bug-683-*`
  (present leak) — both fixed; this is the remaining cost on the same path and
  is **not** explained by either.
- `planning/plan-150-*` — `canvas::ParticleSystem` exists to avoid presenting N
  changing items. This bug is why that matters, and also why plan-150's
  expansion must not route N particles through this same per-item path.
- `mfb man canvas` — "the runtime caches each item's geometry on a content
  hash: re-presenting an item that did not change is free." At these scene
  sizes it is not free; it is the dominant cost.

## Failing Reproduction

A canvas program presenting N polygons of P points each, every frame, on Metal.
`MFB_CANVAS_GPU=1`, `MFB_CANVAS_STATS` for the frame counter, 15 seconds per
run. `moving` gives every item new coordinates each frame; `static` presents an
identical scene every time.

```sh
MODE=moving ITEMS=2500 POINTS=8 MFB_CANVAS_GPU=1 \
  MFB_CANVAS_STATS=/tmp/stats.txt ./canvasleak.app/Contents/MacOS/canvasleak
wc -l /tmp/stats.txt        # one line per rendered frame, over 15 s
```

| mode | items | points | frames in 15 s | fps | generations | gen / item / frame |
| --- | --- | --- | --- | --- | --- | --- |
| moving | 95 | 8 | 708 | 47 | 67,260 | 1.0 |
| moving | 1,000 | 8 | 287 | 19 | 574,000 | 2.0 |
| moving | 2,000 | 8 | 114 | 7.6 | 456,000 | 2.0 |
| **static** | **2,000** | **8** | **109** | **7.3** | 436,000 | 2.0 |
| moving | 2,500 | 8 | **7** | **0.47** | 35,000 | 2.0 |
| moving | 3,000 | 8 | 6 | 0.4 | 36,000 | 2.0 |
| moving | 5,000 | 8 | 3 | 0.2 | 30,000 | 2.0 |
| **static** | **5,000** | **8** | **3** | **0.2** | 30,000 | 2.0 |
| moving | 3,000 | 3 | 67 | 4.5 | 402,000 | 2.0 |
| moving | 5,000 | 3 | 3 | 0.2 | 30,000 | 2.0 |
| moving | 1,200 | 16 | 11 | 0.73 | 26,400 | 2.0 |

`gpuSelected=TRUE` and `skipped=0` on every row.

- Observed: a cliff — 2,000→2,500 items at 8 points drops 7.6 fps to 0.47 fps.
  The cliff moves with points-per-item (3,000 items is fine at 3 points and
  collapsed at 8; 1,200 items is already collapsed at 16), so the trigger is
  some function of total geometry volume rather than item count alone, but it
  is **not** a clean linear threshold: 2,000×8 is fine while 1,200×16 is not,
  despite less total geometry.
- Expected: smooth, proportional degradation. 2,500 items should be a little
  slower than 2,000, not 15× slower.

The `static` rows are the load-bearing ones. Presenting the *same* scene every
frame should hit the geometry cache on every item and cost nothing, yet it is
exactly as slow as the moving case, and `generations` shows every item building
new geometry on every frame either way.

| Environment | | Result |
| --- | --- | --- |
| macos-aarch64, Metal | `MFB_CANVAS_GPU=1`, `gpuSelected=TRUE` | fails ✗ |
| macos-aarch64, software | default | fails ✗ (`examples/wind`, ~1 frame/5–30 s) |
| Linux / Windows | not measured | the geometry path is shared MFBASIC helper source, so expect ✗ |

## Root Cause

Not yet pinned to a single line; two contributing defects are established by
the data, and the second needs the investigation Phase 1 describes.

**1. The geometry cache thrashes to a 100% miss rate on any scene larger than
itself — confirmed.** `__CANVAS_GEO_CAPACITY` is **256**
(`helper_geometry.rs:108`). A frame walks every item in scene order and probes
the cache for each; with 2,000+ items the entries a frame installs are all
evicted before that frame comes round again, so the next frame misses on every
item. The `generations` column is the direct evidence: a steady **2 new
geometry entries per item per frame** in every run at 1,000 items and above —
including the `static` runs, where the scene is byte-identical frame to frame
and every probe should hit.

So above 256 items the cache is not merely ineffective, it is pure overhead:
each item pays a probe that scans up to 256 slots, an eviction (which does
`collections::removeAt` on four index lists), and a fresh geometry build — and
is then evicted before it can ever be read back. The 95-item run, which fits in
the cache, is the only one showing a different generation ratio.

**2. Something is superlinear past roughly 2,000 items — not yet pinned.** The
thrashing above is O(items × cache) per frame, which is bad but smooth; it does
not explain a 15× drop for a 25% size increase. Candidates, in the order Phase 1
should eliminate them:

- the eviction path's `collections::removeAt` repacking four lists per miss,
  interacting with list growth as the arena reaches a size class;
- the per-frame arena reclamation added by bug-682's fix becoming O(arena) at a
  frequency that scales with misses;
- a GPU-side buffer threshold causing a per-frame fallback or re-upload (the
  cliff moving with points-per-item is consistent with a vertex-buffer limit);
- memory pressure: the collapsed runs sit at 215–856 MB RSS, but 3,000×3 is
  healthy at 277 MB while 1,200×16 is collapsed at 215 MB, which argues against
  RSS being the trigger on its own.

The stats line already carries `generations`, `frames`, `skipped` and the GPU
flags; it does not carry where the wall-clock went. Phase 1 adds that.

## Goal

- A scene of 5,000 simple items renders at an interactive frame rate on Metal —
  concretely, no worse than proportional to the 1,000-item figure.
- Re-presenting an unchanged scene of any size costs no geometry rebuilds:
  `generations` stays at 0 after the first frame.
- No cliff: frame rate as a function of scene size has no step change.

### Non-goals (must NOT change)

- **What a scene draws.** This is scheduling and caching; every golden
  reference image must be byte-identical afterwards.
- **The live-offset invariant** (`helper_geometry.rs:114`) and the retirement
  drain gate that bug-682 and bug-683 established. A fix that reclaims or
  compacts more eagerly must not hand the renderer a freed offset.
- **Do not "fix" this by documenting an item-count ceiling**, or by telling
  programs to batch into images. Those are workarounds a program may choose;
  they are not a fix for a retained-scene API whose man page invites per-frame
  presents.
- **Do not simply raise `__CANVAS_GEO_CAPACITY` to a large constant** without
  addressing the per-miss cost — that trades thrashing for a larger linear scan
  and more unreclaimed arena, and only moves the cliff.

## Blast Radius

- `helper_geometry.rs` `__canvas_geometryFor` / `__canvas_geoEvict` — the
  per-item path, **the bug**.
- Every `Mode.Canvas` program with a scene of more than a few hundred items,
  static or animated. `examples/wind` is blocked on it.
- `planning/plan-150-*` `canvas::ParticleSystem` — **interacts directly**. If
  graphics-thread expansion emits ordinary draw entries that probe this cache,
  a particle system of N particles inherits this cliff and plan-150 does not
  deliver what it promises. Plan-150-B (cost) should be re-read against this.
- `canvas::Group` — a named sub-scene is the existing escape hatch for "many
  items that do not change"; whether a Group's items bypass the per-item probe
  should be measured, because if they do, it is a partial answer for static
  content (but not for animation).
- The software rasteriser — same helper source, same path, **same hazard**;
  `examples/wind` demonstrates it there too.

## Fix Design

Not settled — Phase 1 is a measurement phase precisely because defect 2 is
unpinned. The shape the data already supports:

- **Make the cache scale with the scene, or stop paying for it when it cannot
  help.** A per-frame miss rate near 100% should degrade to "build geometry,
  draw it, skip the cache", not "probe 256 slots, evict, build, install, be
  evicted". Detecting that cheaply (e.g. a frame's miss ratio) turns the
  pathological case into merely the uncached case.
- **Size the cache to the scene** at present time, when the item count is
  known, rather than fixing it at 256 for every program.

Both keep the fast path for small scenes exactly as it is. Which of them — or
whether defect 2 turns out to dominate and require something else entirely —
is what Phase 1 decides.

## Phases

### Phase 1 — instrument and pin (no behavior change)

- [ ] Extend the `MFB_CANVAS_STATS` line with a wall-clock breakdown per frame:
      time in geometry probe, in eviction, in build, in arena reclamation, and
      in GPU submit.
- [ ] Re-run the table above and attribute the cliff to one of the candidates
      in Root Cause; write the verdict into this file.
- [ ] Measure whether `canvas::Group` items bypass the per-item probe.
- [ ] Add the failing test: a scene of N items whose frame rate must stay within
      a proportional band of the N/5 figure, and an unchanged-scene test
      asserting `generations` is 0 after warm-up.

Acceptance: the tests fail for a documented, attributed reason.
Commit: —

### Phase 2 — the cache

- [ ] Stop the 100% -miss thrash (bypass or resize, per Phase 1's verdict).

Acceptance: `generations` drops to 0 for an unchanged scene of any size; the
`static` rows reach an interactive frame rate.
Commit: —

### Phase 3 — the cliff

- [ ] Fix whatever Phase 1 attributed the superlinear collapse to.

Acceptance: no step change in the frame-rate/scene-size curve; 5,000 items is
within a proportional band of 1,000.
Commit: —

### Phase 4 — validation

- [ ] Canvas goldens byte-identical.
- [ ] Re-run the full table; both the cliff and the static/moving gap are gone.
- [ ] `examples/wind` at ~5,750 items is interactive, and its q/e/wasd controls
      visibly respond.
Commit: —

## Validation Plan

- Regression tests: the frame-rate-proportionality and unchanged-scene-
  `generations` tests from Phase 1.
- Runtime proof: the reproduction table, re-measured, with no cliff.
- Doc sync: `mfb man canvas`'s "re-presenting an item that did not change is
  free" becomes true at every scene size, or is qualified honestly.
- Full suite: `cargo test`, canvas goldens in particular.

## Open Decisions

- Bypass-on-thrash vs. size-the-cache-to-the-scene — recommended: decide from
  Phase 1's attribution rather than now. (§Fix Design)

## Summary

The established part is defect 1, and it is squarely a design bug: a 256-entry
cache in front of scenes of thousands turns a hit-path optimisation into a
per-item tax, which is why a *static* 5,000-item scene is as slow as a moving
one. The unestablished part is the cliff, which is sharper than thrashing alone
can explain and is what makes the difference between "slow" and "unusable".
Phase 1 exists because guessing between the four candidates would be cheaper to
write and worse to be wrong about.
