# plan-175: Canvas geometry identity split (a geometry key, and a paint identity that damage also reads)

Last updated: 2026-10-12
Effort: huge (>3d) over five phases; each phase is large (3h–1d) and lands alone

A canvas item has ONE 62-bit content hash. It is the key of the geometry cache and it
is the damage identity (`.ai/canvas-threading.md` §14). It folds the item's whole
`Paint`, so a scene that changes only colour misses the cache for every item on every
frame: each record is built again and inserted again, and the cache compacts away the
records of the frame before. bug-765 measured that on 100,000 dots whose fill
alternates, and the owner's direction of 2026-10-12 is:

> the geometry hash is based on geometry only, and there is a second identity for paint
> and anything else that damage needs.

This plan is that split. It is the output of bug-765's spike. Every load-bearing claim
is backed by a measurement or a throwaway experiment, with its command; the spike's
code was never committed (§9 says how to rebuild it).

**Read §2.3 before deciding to start.** The split removes what it was aimed at, and it
is NOT what stands between this scene and 30 frames a second. The measurements name
three larger costs. The plan covers the split and says, in §8, what each of the other
three needs.

References:

- `bugs/bug-765-canvas-100k-color-changing-dots-miss-30-fps.md` — the bug, its Cause
  note, and what its first deliverable (frames drawn in passes) landed.
- `.ai/canvas-threading.md` §3 (scene ring, hashes revision), §10 (the gate, passes),
  §14 (geometry cache and item hashes), §16 (`AutoPaint`), §17 (`Picture`).
- `src/docs/spec/app/06_canvas.md` — the contract; §4.7 lists the text a split changes.
- `tools/canvas-bench/README.md` — `dots N`, the row this plan is judged by.
- `planning/plan-173-canvas-atlas-tile.md` — adds a `DrawItem` variant. Whichever of the
  two lands second classifies the other's fields; §4.1's guard makes forgetting a build
  error.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| bug-765's first deliverable is on the branch: 100,000 dots are GPU frames | `tools/canvas-bench/bench.sh target/release/mfb dots 100000 5` → `gpuFrames` equals `frames` | MET since `61c75308e` (`frames=69 gpuFrames=69`) |
| The geometry verifier is green at the base | `cargo test --release --test rt_canvas_geo_native` → 4 passed | MET at `ab9061ff8` |

## 1. Goal

- An item's GEOMETRY identity `G` is a 62-bit hash of what decides the SHAPE of its
  geometry record: its own fields, its transform, and the parts of its paint that are
  geometry (§4.1). `G` keys the geometry cache.
- An item's APPEARANCE identity `A` is a 62-bit hash of everything else it draws with.
  `A` keys nothing by itself.
- The damage identity is `D = mix(G, A)`. It changes exactly when today's hash changes,
  so the damage diff, `MFB_CANVAS_DAMAGE` and every draw hash keep their meaning.
- A frame that changes only paint builds no geometry record. `generations` does not
  move and the cache does not compact; the cached record's appearance slots are
  rewritten in place (§4.3).
- No pixel changes, on any backend. `MFB_CANVAS_GEO_VERIFY=1` reports 0 mismatches on
  every suite that sets it.

### Non-goals

- No change to the public API, to `present`'s deep copy and ownership, to the content
  frame skip, to the scene ring, or to image and font lifetimes.
- No retained GPU item buffer: every frame still uploads every drawn block (§8.3).
- No change to what a `Picture` or a `Text` must have confirmed beyond its hash
  (`__canvas_pictureCached`; a `Text` record's glyph indices).
- No sharing of geometry between an `AutoPaint`'s state records (§4.6).
- Reaching 30 fps at 100,000 dots is NOT promised by this plan. §2.3 gives what it buys.

## 2. Current State

### 2.1 One hash, two jobs

- `func_item_hash.rs:emit_item_hash` is one walker generated from the type model. For
  ten kinds (`HASHED`: Rectangle, Circle, Line, Arc, Polygon, Picture, Ellipse,
  PolyLine, PolyCurve, CurvePolygon) it folds `id`, the kind's own fields, the WHOLE
  `paint` record and the transform, in slot order, into a 64-bit multiply-xorshift that
  is shifted to 62 bits. `Text`, `Group` and `AutoPaint` (`NOT_HASHED`) answer -1 and
  are hashed in MFBASIC by `helper_geometry.rs:__canvas_hashItem` with the two-lane
  `__canvas_hashStep`; its paint walk is `__canvas_hashPaint`.
- The worker computes the hashes in `canvas::present`:
  `func_carried_hashes.rs:lower_carried_hashes` keeps the installed hash of every item
  whose payload bytes did not change, and `func_item_hash.rs:lower_scene_hashes` hashes
  the rest. They are published beside the scene (`canvas::publishHashes`) under the
  scene's revision.
- The graphics thread resolves every scene index by that hash
  (`func_geo_cache.rs:lower_scene_resolve`): `emit_find` in `__CANVAS_GEO_TABLE`; a hit
  is trusted on the hash alone (`emit_hit`, pictures excepted); a miss of a natively
  built kind is built in place (`emit_fill_record`, `emit_add_slot`,
  `generations` + 1), anything else is left to `__canvas_appendDraw`.
- The same hash, folded with the entry's group offset and matrix
  (`helper_render.rs:__canvas_noteEntry`, natively `emit_hash_step_zero`), is the draw
  hash `helper_damage.rs:__canvas_damageFor` compares frame to frame.

### 2.2 What today's record carries that is appearance, not geometry

A record is `__CANVAS_GEO_HEADER` = 67 floats and a tail. Read from
`helper_geometry.rs` (`__canvas_paintHeader`, `__canvas_gradientHeader`,
`__canvas_strokeAsFill`, `__canvas_withDashes`, `__canvas_withJoins`) and its native
twin `func_geo_build.rs:emit_paint_header`:

| Slots | Holds | Class |
|---|---|---|
| 0, 1 | kind, record length | geometry; the length includes the gradient's stop COUNT |
| 2–6, 16–25, 27–40, 47–53 | box, radius, bounds, aux, clip, inverse transform, cap, arc ends, join count and reach, dash count and phase, clip mode | geometry (some FROM paint: stroke width, cap, join, miter limit, dashes, clip) |
| 7 | strokeHalf | geometry from paint: `strokeWidth / 2`, or -1 when the stroke's alpha is 0 or its width is not positive |
| 8–11, 12–15 | fill RGBA, stroke RGBA (alpha faded by `transparency`) | APPEARANCE |
| 26 | blend mode | APPEARANCE, and it decides the draw list: instance count and run splits |
| 41 | gradient stop count | structure: it sizes the tail and decides "is filled" |
| 42–46, 55–66 | gradient kind, axis, spread, apply, radial inverse, focal | APPEARANCE |
| 54 | a picture's transparency | APPEARANCE |
| 35, 36 (picture) | the pixel block's address | content identity, neither class (§4.6) |
| tail: edges, join records, dash floats, clip edges, curve pieces, glyph triples | | geometry |
| tail: gradient stops (5 floats each), a `PolyLine`'s per-point colours | | APPEARANCE of a fixed COUNT |

Three facts decide the design, and each was checked:

- **A zero stroke alpha is geometry.** `__canvas_strokeHalf` answers -1 for it, which
  moves the bounds, drops the joins and dashes, and makes a `Line`, `Arc`, `PolyLine` or
  `PolyCurve` an empty record. A fill's alpha never changes what is built. So `G` folds
  PREDICATES of the paint (is the stroke drawn; how many stops; does the gradient apply
  to the stroke), not only whole fields.
- **Appearance is read from the record by every consumer.** The software renderer
  (`helper_items.rs:__canvas_drawGeometry`: slots 8–15, 26, 41–46, 54–66 and the stops),
  both emitters' `emit_item_block` (the block's fill at byte 32, stroke at 48, blend at
  104, gradient words at 108, 184–207 and 320–367 of `ITEM_BLOCK_SIZE` = 368) and their
  stop uploads, `__canvas_blockInstances` and `__canvas_drawsJoin` (slots 7, 11, 26,
  41), and the pass sum. Moving appearance OUT of the record changes all of them.
  Rewriting it IN the record changes none.
- **Equal items share one record today.** 1,000 identical circles and one more of the
  same geometry and another paint are 1,001 blocks over 2 records (spike experiment 3,
  §9: `entries=2 blocks=1001`). Under a geometry key the 1,001st would land on the
  first 1,000's slot, so an in-place rewrite is sound only with the rule of §4.3.

`helper_hit.rs:__canvas_hitCurrent` reads bounds, transform, clip, radius, strokeHalf,
joins and the text box: no colour, no fill alpha, no blend. The hit path needs `G` only.

### 2.3 Measured: where a frame of dots goes, and what the split removes

Scene: `tools/canvas-bench/gen/dots.sh`, two prebuilt scenes of N fixed circles that
differ in fill, presented in turn, 900 by 640, headless (the readback path). Columns:

- **worker** — microseconds inside `canvas::present` per call in a RELEASE build (deep
  copy, content compare, carried hashes, hashing, publish). The `same` row presents one
  scene every time, so it is copy + compare and nothing else.
- **offsets / draws / render** — the `--debug` stats line's `phaseOffsetsMs`
  (snapshot, resolve, geometry builds, compaction), `phaseDrawsMs` (draw list) and
  `phaseRenderMs` (the gate, pass layout, upload, GPU, readback), per frame.
- **geo** — the spike's upper bound: the item hash made to skip `paint`
  (`MFB_SPIKE_GEOHASH`), so every item hits. Correctness is knowingly broken: the
  picture keeps its first colours. It is what a geometry key costs when a paint change
  needs NO work at all; the real design adds the rewrite of §4.3 on top.

Box 206 (Windows 11, Intel UHD 630, Vulkan). The box ran nothing else; two runs each,
both shown where they differ by more than 1%
(`/tmp/bs23-agent-e/spike-run.sh <spike mfb> N <variant> 5 test@192.168.1.206`):

| N | variant | release pps / fps | worker µs | offsets ms | draws ms | render ms | builds/frame | compactions/frame |
|---:|---|---|---:|---:|---:|---:|---:|---:|
| 10,000 | paint | 20 / 21 | 8,730 / 8,711 | 5.33 | 0.41 | 44.6 | 10,000 | 1.00 |
| 10,000 | geo | 22 / 23 | 7,352 | 1.08 | 0.37 | 44.8 | 0 | 0 |
| 10,000 | same | 60 / 0 | 7,973 | | | | | |
| 50,000 | paint | 7 / 8 | 43,843 | 27.0 | 2.21 | 109.9 | 50,000 | 1.00 |
| 50,000 | geo | 8 / 9 | 36,734 | 5.40 | 2.35 | 110.9 | 0 | 0 |
| 50,000 | same | 25 / 0 | 40,443 | | | | | |
| 60,000 | paint | 6 / 7 | 52,755 | 31.9 | 2.59 | 125.0 | 60,000 | 1.00 |
| 60,000 | geo | 7 / 8 | 44,402 | 6.40 | 2.83 | 125.1 | 0 | 0 |
| 60,000 | same | 21 / 0 | 48,867 | | | | | |
| 100,000 | paint | 4 / 4 | 88,837 / 89,297 | 55.9 | 4.12 | 213.6 | 100,000 | 1.00 |
| 100,000 | geo | 4 / 5 | 74,957 / 74,394 | 11.0 | 4.75 | 217.4 | 0 | 0 |
| 100,000 | same | 13 / 0 | 82,681 / 81,393 | | | | | |

The render phase on box 206, split at 60,000 by two more spike switches (the frame
drawn with an empty draw list, then with an empty block list too; two runs each):

| What | ms per frame |
|---|---:|
| the whole render phase | 125.1 / 118.5 |
| with no draw entries (upload, clear, copy out, swizzle) | 36.2 / 36.3 |
| with no blocks either (clear, copy out, swizzle, the gate's sum) | 11.5 / 11.5 |
| so: the GPU drawing 60,000 circles | 82 to 89 |
| so: writing 60,000 item blocks into the frame buffer | 24.7 |
| the gate's sum over 60,000 blocks (`spikePlanMs`) | 4.0 |
| pass layout + gate at 100,000 (two passes) | 17.7 |

This Mac (Apple Silicon, Metal). **Indicative only**: other agents were building and
the load average was between 14 and 90 for every row, shown beside each
(`/tmp/bs23-agent-e/spike-run.sh <spike mfb> N <variant> 4`):

| N | variant | release pps / fps | worker µs | offsets ms | render ms | of it gate + layout | load |
|---:|---|---|---:|---:|---:|---:|---:|
| 10,000 | paint | 60 / 60 | 3,299 | 1.96 | 5.4 | 0.23 | 22 |
| 10,000 | geo | 60 / 60 | 2,643 | 0.67 | 6.6 | 0.44 | 20 |
| 10,000 | same | 60 / 0 | 2,112 | | | | 17 |
| 50,000 | paint | 38 / 39 | 17,537 | 12.07 | 15.4 | 1.28 | 38 |
| 50,000 | geo | 57 / 57 | 16,120 | 2.58 | 12.9 | 1.22 | 33 |
| 50,000 | same | 60 / 0 | 11,206 | | | | 31 |
| 60,000 | paint | 23 / 23 | 34,273 | 15.61 | 19.6 | 1.61 | 30 |
| 60,000 | geo | 45 / 45 | 20,619 | 3.01 | 13.9 | 1.41 | 38 |
| 60,000 | same | 60 / 0 | 17,167 | | | | 30 |
| 60,000 | paint, no draw entries | | | 14.75 | 8.6 | 1.73 | 50 |
| 100,000 | paint | 18 / 19, 14 / 15, 9 / 10 | 36,711 / 45,061 / 46,728 | 21.9 / 29.6 / 30.1 | 29.7 / 42.4 / 45.3 | 8.8 / 13.2 / 13.3 | 14 / 57 / 61 |
| 100,000 | geo | 27 / 28, 24 / 24, 9 / 9 | 33,002 / 35,777 / 86,647 | 7.2 / 5.8 / 10.1 | 40.6 / 33.5 / 45.9 | 12.9 / 11.2 / 17.4 | 19 / 51 / 90 |
| 100,000 | same | 33 / 0 | 32,030 | | | | 71 |

What the tables say:

1. **The split removes 80% of the offsets phase and nothing else.** On box 206 at
   100,000: 55.9 ms to 11.0 ms a frame, 45 ms saved, and that is geometry builds,
   inserts and the per-frame compaction (`geoCompactions` 1.00 a frame to 0;
   `entries` 200,000 to 100,000). What is left of the phase is the snapshot and one
   table probe per item.
2. **It is 12% of the work in that frame.** The same frame spends 89 ms in the worker
   (its own thread) and 214 ms in the render phase. Release goes from 4 fps to 4 or 5.
3. **The worker's `present` alone is over the budget**: 89 ms on box 206 and 33 to 47 ms
   on the Mac at 100,000, in release. 93% of it is the deep copy and the content compare
   (the `same` row); hashing is the remaining 6 ms on box 206. No hash design changes it.
4. **On the Intel GPU the draw itself is 1.4 µs a circle**: 82 to 89 ms for 60,000, with
   the buffer already written. 100,000 dots cannot reach 30 fps on that GPU by any
   change to the CPU side.
5. **Writing the item blocks is 0.41 µs a block on box 206** (24.7 ms for 60,000), every
   frame, for every drawn block, changed or not.
6. **The gate and pass layout are 9 to 13 ms a frame at 100,000 on the Mac and 17.7 ms
   on box 206** (the gate's sum alone is 4.0 ms at 60,000 there). The layout is
   MFBASIC, added by bug-765's first deliverable, and paid only by frames that do not
   fit one buffer.

So against 33.3 ms at 100,000 dots, after the split, on box 206: worker 75 to 89 ms,
offsets 11 ms, draws 4.7 ms, render 217 ms. On the Mac (load 19 to 51): worker 33 to
36 ms, offsets 6 to 7 ms, render 34 to 41 ms debug, and 24 to 28 fps in release
against 14 to 19 before.

## 3. Design Overview

**Two lanes from one walk, one classification.** Every field of every item kind, and
every field of `Paint`, is declared GEOMETRY or APPEARANCE in one table. The native
walk folds each field into the `G` lane or the `A` lane as that table says, in one pass
over the item; the MFBASIC walk for `Text`, `Group` and `AutoPaint` reads the same
classification. The three paint predicates that are geometry are folded into `G` as
their own words. `D = mix(G, A)`.

**The cache is keyed by `G`, and each slot remembers the `A` it holds.** Resolving an
item: find `G`; if the slot's `A` is the item's, it is today's hit. If it differs and no
entry of this frame has used the slot, the record's appearance slots are rewritten in
place from the item and the slot takes the new `A`: a REFRESH, with no allocation, no
insert, no compaction. If it differs and the slot is already in use this frame (the
shared-record case of §2.2), the item is resolved under the key `mix(G, A)`, which is
exactly today's behaviour for it.

**Nothing downstream moves.** The record keeps its layout. The renderers, the
emitters, the draw list, the pass sum and the verifier read what they read today. That
is the reason for rewriting in place rather than moving appearance out of the record:
§2.2 counts six consumers of the appearance slots across three renderers, and a design
that changes none of them cannot draw a different pixel through them.

**The correctness risk is one sentence: a refresh must leave the record bit-identical
to the one a fresh build makes.** `MFB_CANVAS_GEO_VERIFY` already compares every
resolved record, hits included, against a reference built from the frame's item
(`__canvas_geoVerifyFrame`, `geoResolvedChecked=`). Spike experiment 2 (§9) shows it
sees a stale paint: 120,000 mismatches in 120 frames of 2,000 dots under the
geometry-only hash, 0 under today's. So the verifier is the oracle for every phase, on
every suite and box that sets it.

Rejected alternatives:

- **Appearance in a per-frame side list, read by index.** Sound without any sharing
  rule, but every consumer of §2.2 changes (two shaders' item-block emitters, the
  software renderer, the draw-list rules, the gate), on three renderers that must agree
  to the pixel. Rejected for blast radius, not for cost.
- **A geometry "donor" index: on a miss, copy the donor's record and repaint the
  copy.** It keeps today's keying, but it still writes and inserts a record per changed
  item per frame and still compacts. For a circle the build is about 27 stores and one
  divide (`func_geo_build.rs:fill_circle`); the cost measured in §2.3 is the churn, and
  this keeps the churn.
- **Keep both frames' records alive (a larger cache).** It would make THIS benchmark
  hit, because it alternates two scenes, and do nothing for a colour that changes
  continuously. It also doubles the cache for every scene.

## 4. Detailed Design

### 4.1 The classification, and the guard

One table in `func_item_hash.rs`, beside `HASHED`:

- every field of `Paint` is `Geometry`, `Appearance` or `Split`;
- `Split` fields are `stroke` (its alpha's zero-ness is geometry, the rest appearance),
  `gradient` (the stop COUNT and `strokeApply` are geometry: they size the tail and
  change strokeHalf; everything else is appearance) and a `PolyLine`'s `colors` (its
  count is geometry, `__canvas_polylineHalf`; its values are appearance);
- every item field is `Geometry` except `id` (§7, decision 1) and a `Picture`'s `image`
  (content identity, §4.6).

`fold_field` takes the lane from the table. A field the table does not name fails the
build, as an unknown field TYPE does today, so a new `Paint` field or a new item kind
(plan-173's `Tile`) cannot be silently folded into the wrong lane or into neither.

The geometry predicates are computed ONCE, by the function the header builders call:
`__canvas_strokeHalf` and its native twin are what decide "is the stroke drawn", and
`G` folds that function's answer class, not a second copy of its rule. A second copy is
the failure this plan must not introduce: a `G` that calls a stroke drawn where the
builder calls it absent refreshes a record whose bounds are wrong.

### 4.2 What is published, and what carries

- The worker publishes `G` and `A` for each item: one list of 2N words under the one
  hashes revision (`CANVAS_SCENE_HASHES_REVISION_OFFSET`), so the sequence lock of §3
  covers both with no new word. `canvas::snapshotHashes` hands the frame both.
- `canvas::carriedHashes` carries BOTH when an item's payload bytes are unchanged, and
  neither otherwise. `Text`, `Picture` and `AutoPaint` are never carried, as now (their
  hash folds a resource id that answers 0 once closed).
- `__CANVAS_DRAW_HASHES` holds `D` folded with the entry's offset and matrix, as now.
  `__canvas_damageFor`, `__canvas_rememberScene` and `__canvas_autoResolve` are
  untouched. Spike experiment 1 (§9) is why `D` must fold `A`: with the geometry-only
  hash as the damage identity, `MFB_CANVAS_DAMAGE=1` skipped 104 of 105 frames of a
  scene whose every colour changed.

### 4.3 Resolve: hit, refresh, or today's path

`__CANVAS_GEO_SLOTS` is four words a slot (hash, offset, count, lastUsed). It becomes
five: the `A` the record holds. `lower_scene_resolve`, per index:

1. `emit_find(G)`.
2. Found, slot `A` equal: `emit_hit`, as today.
3. Found, slot `A` differs, `lastUsed` is not this frame: REFRESH. Rewrite the
   appearance slots from the item, store the new `A`, stamp the slot, hit.
4. Found, slot `A` differs, `lastUsed` IS this frame: resolve under `mix(G, A)`
   instead — find, or build and insert there. Two items of one geometry and two paints
   in one frame cost what they cost today.
5. Not found: build and insert under `G`, as today under the one hash.

Refresh writes exactly the APPEARANCE rows of §2.2's table: for a solid paint nine
floats (8–15, 26). It never changes the record's length, kind, bounds or strokeHalf,
because every paint fact that could is in `G` by §4.1; a paint change that moves one of
them changes `G` and takes branch 5.

The draw list is laid out after the resolve (`canvas::sceneDrawsFlat`,
`__canvas_sceneDrawsWalk`) and reads slots 7, 11, 26 and 41 from the records each
frame, so a refreshed blend mode or fill alpha is seen by `__canvas_blockInstances` and
`__canvas_drawsJoin` in the same frame.

Phase 2 implements refresh natively for the kinds and paints `canvas::geoBuild` builds
natively (`Kind::ALL` with fewer than two stops and no transform:
`emit_paint_supported`), by calling `emit_paint_header`'s appearance half. Every other
case takes branch 3 as a REBUILD IN PLACE through the MFBASIC builder (same length by
construction, so the slot and its offset are kept): correct, and no faster than today
until Phase 3 gives gradients, pictures and text their own refresh.

### 4.4 The worker's cache (hit-testing)

`helper_hit.rs:__canvas_hitGeometry` keys the worker's thread-local cache with
`__canvas_hashItem(item)`. It reads no appearance slot (§2.2), so it is keyed by `G`
and never refreshes: a paint-only frame costs the hit path no build at all.

### 4.5 Collisions

- **`G` is trusted alone**, as today's hash is. It is 62 bits: a false hit needs two
  different geometries with one `G` among the keys alive in the cache. With k distinct
  keys that is about k² / 2^63: 1.1e-9 for 100,000. Today every paint change mints a
  new key (100,000 a frame in the dots scene, each a chance against 200,000 entries);
  under the split a paint change mints none, so the exposure FALLS to geometry changes.
- **`A` is never trusted alone.** It is compared only against the `A` of the slot `G`
  already chose. A wrong "equal" needs one item's old and new paint to collide in 62
  bits: 2^-62 per changed item, and its effect is one item keeping its last paint.
- **`D`** decides damage only. A wrong "unchanged" is 2^-62 per changed entry and shows
  only under `MFB_CANVAS_DAMAGE`.
- The native walk's mix (`emit_item_hash`: multiply-xorshift, 62 bits kept) has no
  written collision argument today; only the MFBASIC two-lane `__canvas_hashStep` has
  (`helper_geometry.rs:GEO_HASH`). Phase 1 writes the argument above into the module
  doc and `.ai/canvas-threading.md` §14, for both walks and both lanes.

### 4.6 Pictures, text, groups, `AutoPaint`

- **`Picture`**: never resolved on a hash alone, before or after.
  `__canvas_pictureCached` still confirms slots 35/36 against the image's current pixel
  block, and `emit_hit` still leaves every picture to the MFBASIC walk. The image id
  folds into `G` (a different image is a different record: its size is geometry). The
  tint and transparency are `A`. Records under `__canvas_pictureKey` step `G`.
- **`Text`**: the font, size, string and position are `G`; the fill colour and blend
  are `A`. A `Text` record's glyph indices stay volatile (`canvas::geoForget`). Until
  Phase 3 a colour change rebuilds the run, as now.
- **`Group`**: a node's hash keys nothing. Its children are hashed on the graphics
  thread in `__canvas_appendDraw` and `__canvas_memoGroup`; both take `(G, A)` from the
  one MFBASIC walk and resolve through `__canvas_geometryFor`, which gains branch 3 and
  4 as §4.3's MFBASIC path.
- **`AutoPaint`**: its state records are keyed `__canvas_autoGeoHash(hash, shown)`. They
  keep one full record per shown paint under a key stepped from `D`, exactly as now,
  and are never refreshed (their slot's `A` is the paint the key already names).
  Sharing their geometry is a separate, smaller plan.

### 4.7 Documentation the split changes

`.ai/canvas-threading.md` §14 (rewritten: two identities, the slot's `A`, refresh, the
shared-slot rule, the collision argument), §16 and §17 (one sentence each). In
`src/docs/spec/app/06_canvas.md`: "Retained, not immediate" (the cache bullet), "The
item unions" (what is hashed natively), "Named groups" (the draw hash), "Hit-testing"
(the worker's cache key), "AutoPaint → One frame" (`__canvas_autoGeoHash`), "A picture
is drawn inside a shape" (the keys). There is no spec heading for `carriedHashes`, the
cache table or damage identity today; Phase 5 adds one ("Item identity").

## Compatibility / Format Impact

- No public API, no program-visible behaviour, no pixel.
- The published hashes list doubles (16 bytes an item). At 100,000 items that is 0.8 MB
  more in the scene region per publish.
- `__CANVAS_GEO_SLOTS` grows from four to five words a slot; `generations=` stops
  counting paint-only changes (which is its documented meaning: geometry builds).
- A new stats field is wanted, `refreshes=` (§7, decision 2). It is the only way a test
  can tell a refresh from a hit.
- Every canvas program's code changes, so the canvas goldens under `tests/syntax/app/`
  move in the phases that touch the walk and the resolve.

## Phases

### Phase 1 — Two lanes, one damage identity (no behaviour change)

The classification table and its build-time guard; the native walk and the MFBASIC
walk each produce `(G, A)`; the worker publishes both and carries both; every consumer
is handed `D = mix(G, A)` and behaves as now. The cache is still keyed by `D`.

- [ ] `func_item_hash.rs`: the field classification, `fold_field` by lane, the three
      geometry predicates from the builders' own functions, `mix`.
- [ ] `helper_geometry.rs`: `__canvas_hashItem` and `__canvas_hashPaint` return the
      pair for `Text`, `Group`, `AutoPaint`; `__canvas_pictureHash` likewise.
- [ ] `func_carried_hashes.rs`, `func_scene_hashes.rs`, `func_present.rs`,
      `func_present_layers.rs`, `func_geo_cache.rs:lower_scene_snapshot`: 2N words under
      the one revision.
- [ ] Unit tests: every `Paint` field and every item field is classified; two paints
      that differ only in an appearance field have one `G`; each geometry predicate
      (stroke alpha 0 against 1, one stop against two, `FillOnly` against
      `FillAndStroke`, empty against non-empty `colors`) changes `G`.
- [ ] The collision argument of §4.5 in the module doc and `.ai/canvas-threading.md` §14.

Acceptance: frames are byte-identical to the base on Metal, box 2226 and box 206, and
`generations` is unchanged for every bench row.
  Check: `cargo test --release --test rt_canvas_geo_native --test rt_canvas_damage
  --test rt_canvas_metal --test rt_canvas_hit --test rt_canvas_autopaint` → pass.
  Check: `scripts/test-canvas-gpu-rows.sh <mfb> --target windows-x86_64 --keep <dir>`
  and `--target linux-aarch64 --keep <dir>` against the base compiler's `--keep`: every
  `*-gpu.rgba` equal by `cmp`.
  Check: `tools/canvas-bench/bench.sh <mfb> dots 60000 5` → `builds/frame=60000`.
Commit: —

### Phase 2 — The cache keyed by `G`; native refresh for solid paints

- [ ] `func_geo_cache.rs`: the fifth slot word; `lower_scene_resolve` branches 2 to 5
      of §4.3; refresh through `emit_paint_header`'s appearance half; compaction and
      `emit_rebuild` carry the fifth word.
- [ ] `helper_geometry.rs:__canvas_geometryFor`: branches 3 (rebuild in place) and 4.
- [ ] `refreshes=` on the stats line, if decision 2 is yes.
- [ ] `tests/canvas/rt_canvas_geo_native.rs`: a paint-only change builds nothing and
      refreshes every item; a stroke alpha going 0 to 1 builds; two items of one
      geometry and two paints in one frame, alternating which is first, draw right and
      verify clean; a frame after a refresh is byte-identical to a fresh run that
      starts on that scene.
- [ ] `tests/canvas/rt_canvas_damage.rs`: a paint-only change under `MFB_CANVAS_DAMAGE`
      damages the item's bounds (the regression of spike experiment 1).
- [ ] `scripts/test-canvas-gpu-rows.sh`: a `repaint` row (the dots scene, two frames,
      `MFB_CANVAS_GEO_VERIFY=1`) so the refresh is verified on x86-64 and Linux.

Acceptance: the dots scene builds no geometry after its first two frames, verifies
clean, and no pixel moves.
  Check: `tools/canvas-bench/bench.sh <mfb> dots 100000 5` → `builds/frame=0`.
  Check: `MFB_CANVAS_GEO_VERIFY=1` rows of `scripts/test-canvas-gpu-rows.sh` on box 206
  and box 2226 → `geoVerifyMismatches=0` on every row.
  Check: every `tests/canvas` target passes; the `--keep` frames equal the base's.
  Check, by measurement: `phaseOffsetsMs` per frame at 100,000 on box 206 is within
  25% of the spike's 11.0 ms (the refresh's own cost is what the 25% allows).
Commit: —

### Phase 3 — Refresh for gradients, pictures and text (only where measured)

- [ ] A bench row per case (`dots` with a two-stop gradient whose colours alternate; a
      tilemap whose tint alternates; a page of text whose colour alternates), first, to
      show the rebuild-in-place cost of Phase 2 for each.
- [ ] Refresh for each case whose row shows the rebuild above 10% of its frame: the
      gradient header words and stops tail (same count), a picture's tint and
      transparency, a text run's colour.

Acceptance: each implemented case builds nothing on a paint-only change and verifies
clean; a case not implemented is recorded here with its measured cost.
  Check: the new bench rows; `cargo test --release --test rt_canvas_font --test
  rt_canvas_picture --test rt_canvas_golden`.
Commit: —

### Phase 4 — The worker's hit cache keyed by `G`

- [ ] `helper_hit.rs:__canvas_hitGeometry`, `__canvas_hitAppend`: key by `G`.
- [ ] `tools/canvas-bench/bench.sh <mfb> hit`: a paint-changing row.

Acceptance: a paint-only frame builds nothing on the worker; `overID` is unchanged.
  Check: `cargo test --release --test rt_canvas_hit --test rt_canvas_pointer`.
Commit: —

### Phase 5 — Spec, threading notes, closeout

- [ ] The text of §4.7.
- [ ] `scripts/spec-census.sh --render` and `--citations`.
- [ ] Re-run §2.3's rows on box 206 and this Mac alone (load under 4) and record them
      in bug-765.

Acceptance: the spec describes the implementation at HEAD.
Commit: —

## Validation Plan

- Tests: the unit and runtime tests named per phase. `MFB_CANVAS_GEO_VERIFY=1` on
  every runtime suite that sets it today, plus the new `repaint` row on both boxes.
- Coverage check: the classification guard is itself tested by a `Paint` field
  deliberately left out of the table in a unit test (the build of the walk must fail).
- Runtime proof: `dots 100000` on Metal and box 206 before and after each of Phases 1
  and 2, `--keep` frames compared by `cmp`.
- Doc sync: Phase 5.
- Final gate (once, after the last phase): `cargo test`, `scripts/artifact-gate.sh
  <mfb> all`, the GPU rows on box 206 and box 2226.

## Open Decisions

1. **Is an item's `id` part of `G`?** Today it is folded, so two shapes that differ
   only in `id` have two records. Nothing in a record depends on `id` (the hit walk
   maps entries to ids by scene index: `__canvas_hitAppend`). **Recommendation: leave
   `id` out of `G` and out of `A`, and fold it into `D` only**, so renaming an item
   costs no geometry and still counts as a change for damage.
2. **A `refreshes=` stats field.** Without it a test cannot tell "refreshed" from "hit".
   **Recommendation: yes**, cumulative like `generations=`.
3. **The order of the remaining work.** §8 lists three costs larger than the one this
   plan removes. **Recommendation: do §8.1 (the worker) before this plan's Phase 2**,
   because it is the largest cost on the Mac and no part of this plan touches it; and
   treat §8.2 (the Intel draw) as its own bug, because the 30 fps goal on box 206 is
   out of reach until it moves.
4. **Should bug-765's goal keep "every supported real GPU"?** At 1.4 µs a circle the
   UHD 630 needs 140 ms to draw 100,000. **Recommendation: keep the goal for Metal, and
   set box 206's from what §8.2 can reach** once that is measured.

## 8. What this plan does NOT do, and what each needs

1. **The worker's deep copy and content compare** — 82 ms of an 89 ms `present` at
   100,000 on box 206, 32 ms on the Mac. `gen_present.rs:emit_publish` copies the scene
   and then walks both copies (`emit_values_content_equal_branch`). Candidates, none
   measured: compare the caller's scene against the published one BEFORE copying (an
   unchanged scene then copies nothing, a changed one stops at its first difference,
   which in this benchmark is item 0); copy with the element's bytes first, as the
   compare already does. Its own bug: the ownership contract is not to be weakened.
2. **The GPU's own time on Intel** — 82 to 89 ms for 60,000 circles on the UHD 630.
   Nothing here touches it. It needs the shader's cost per fragment and per instance
   attributed on that driver (bug-688's Cause note on control-flow shape applies), and
   possibly a cheaper path for an item that is a plain filled circle or rectangle.
3. **Writing every item block every frame** — 24.7 ms for 60,000 on box 206. After this
   plan a paint-only frame still rewrites all of every block. A buffer that persists
   across frames and is rewritten only where `D` changed needs a frame buffer per
   in-flight frame or a wait, and pass boundaries that do not move between frames.
4. **Pass layout in MFBASIC** — 9 to 13 ms at 100,000 on the Mac, 17.7 ms on box 206. `__canvas_passNext` appends one
   block at a time to a global list. A native member that lays a pass out from the
   block and instance lists (the way `canvas::sceneDrawsFlat` lays out the draw list)
   would remove most of it; `__canvas_blockCost` stays the one definition by being what
   that member is generated from, or verified against.

## 9. The spike, so a claim here can be re-run

Scratch worktree at `ab9061ff8` (`git worktree add --detach`), its own
`CARGO_TARGET_DIR`, three throwaway edits, never committed:

- `func_item_hash.rs:emit_item_hash`: skip the field named `paint` when
  `MFB_SPIKE_GEOHASH` is set while the program is BUILT.
- `helper_render.rs`: nanoseconds spent in the gate and in `__canvas_passNext`,
  printed as `spikePlanMs=` beside the phase timers.
- `helper_render.rs`: `MFB_SPIKE_NODRAW` hands the renderer an empty draw list, and
  `MFB_SPIKE_NOBLOCKS` (Vulkan) an empty block list as well.

The scene is `tools/canvas-bench/gen/dots.sh` with the time inside `canvas::present`
summed and printed as `workerPresentUs=`.

| # | Experiment | Command (macOS, headless, `MFB_CANVAS_GPU=1`) | Result |
|---|---|---|---|
| 1 | the geometry-only hash as the damage identity | 2,000 dots, built with `MFB_SPIKE_GEOHASH=1`, run with `MFB_CANVAS_DAMAGE=1` for 2 s | `frames=1 skipped=104`: every paint-only frame skipped. Today's hash: `frames=100 skipped=0` |
| 2 | the verifier against a stale paint | the same build, run with `MFB_CANVAS_GEO_VERIFY=1` | `geoVerifyMismatches=120000` over 120 frames. Today's hash: 0 over 102 frames |
| 3 | equal items share a record | 1,000 identical circles and one more of the same geometry and another paint, one frame | `blocks=1001 entries=2 generations=2` |
| 4 | the alternating scene never hits and compacts every frame | §2.3's `paint` rows | `geoCompactions` 1.00 a frame, `entries` = 2N |

## Corrections

None yet. When a phase finds a claim of this plan wrong, the claim is struck in place
and the correction is added here, dated, with the command that showed it.

## Summary

The split is small in mechanism (one classification, a fifth word in a cache slot, nine
floats rewritten in place) and its risk is concentrated in one property the existing
verifier already checks: a refreshed record equals a freshly built one. The larger
risk is expectation. The measurements say this plan removes 45 ms of a 360 ms frame on
box 206 and takes the Mac from about 15 to about 25 frames a second at 100,000 dots;
the frame rate bug-765 asks for is behind the worker's copy and compare, the per-frame
block upload, and on Intel the GPU itself. Decision 3 asks the owner to order them.
