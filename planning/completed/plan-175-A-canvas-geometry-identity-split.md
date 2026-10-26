# plan-175-A: Canvas geometry identity split (a geometry key, and a paint identity that damage also reads), and the group and picture walks

Last updated: 2026-10-13
Overall Effort: huge (>3d) — plan-175 is three letters: A (this file), then
`plan-175-B-canvas-draw-node-table.md` (large), then
`plan-175-C-canvas-group-node-instancing.md` (medium), landed in that order.
Effort: huge (>3d) over seven phases; each phase is large (3h–1d) and lands alone
Depends on: nothing (its Phase 2 waits on §8.1, decision 3)
Dependency graph (Correction 3): A Phase 1 → D → A Phases 2–7 → B → C. `plan-175-D`
(`planning/completed/plan-175-D-canvas-worker-present-copy-compare.md`, landed at
17b7b529e) is §8.1's worker copy and compare, which decision 3 put before Phase 2 and no
letter covered.

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

**Added 2026-10-13: the group and picture walks (§2.4, §4.8, Phases 5 and 6).** The
canvas benchmark (`benchmark/canvas`) found that at 10,000 items the graphics thread's
scene walk costs more than the renderer for `Group` nodes and for `Picture`s. Both walks
run through the code the split rewrites (§4.6: a group's children are hashed and resolved
in `__canvas_appendDraw`; a picture is confirmed by `__canvas_pictureCached` every frame),
so they are done here, after the split's Phases 1 and 2, rather than twice. The
`ParticleSystem` expansion, ~~the benchmark's largest walk~~ the largest walk once the
`AutoPaint` walk's quadratic cost is fixed (Correction 2), touches none of this plan's
code and is not in it (§8.5).

**Updated 2026-10-13: plan-173 landed first** (`canvas::Tile` and viewport culling,
merged at `bf951f32f`). It changed four things this plan stands on, each corrected in
place below and listed in Corrections 1:
- a hashed kind more: `Tile` is in `HASHED` and, like `Picture`, is never carried and
  never resolved on its hash alone (`emit_hit` skips `TILE_KIND_BITS`). §4.1 and §4.6
  classify it.
- `__canvas_pictureCached` confirms tile records too. A tilemap of 10,000 `Tile`s of one
  atlas is Phase 6's case as much as 10,000 pictures of one image is.
- the draw-list builders cull. `__canvas_pushDraw` tests every block of every node
  against the viewport (`__canvas_boundsMeetViewport`), and the native flat list does
  the same (`emit_block_visible`). That is a new per-node cost in the group walk, which
  §2.4's baseline predates.
- `benchmark/canvas` has `tile` rows, which its committed baseline does not.

References:

- `bugs/bug-765-canvas-100k-color-changing-dots-miss-30-fps.md` — the bug, its Cause
  note, and what its first deliverable (frames drawn in passes) landed.
- `.ai/canvas-threading.md` §3 (scene ring, hashes revision), §10 (the gate, passes),
  §14 (geometry cache and item hashes), §16 (`AutoPaint`), §17 (`Picture`).
- `src/docs/spec/app/06_canvas.md` — the contract; §4.7 lists the text a split changes.
- `tools/canvas-bench/README.md` — `dots N`, the row this plan is judged by.
- `benchmark/canvas/` — the standing per-kind benchmark; its `group fill 10000` and
  `picture fill 10000` rows are what Phases 5 and 6 are judged by (§2.4).
- `planning/completed/plan-173-canvas-atlas-tile.md` — added `canvas::Tile` and viewport
  culling, and landed first. So this plan classifies `Tile`'s fields (§4.1); the guard
  makes forgetting a build error.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| bug-765's first deliverable is on the branch: 100,000 dots are GPU frames | `tools/canvas-bench/bench.sh target/release/mfb dots 100000 5` → `gpuFrames` equals `frames` | MET 2026-10-25 at `2f281c5e7` (`frames=119 gpuFrames=119`, Metal) |
| The geometry verifier is green at the base | `cargo test --release --test rt_canvas_geo_native` → 4 passed | MET 2026-10-25 at `2f281c5e7` (`4 passed`) |

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
- A frame that draws a `Group` node hashes the group's items once per `setGroup` of that
  group, not once per node per frame, and copies no item list per node (§4.8).
- A frame that draws many `Picture`s of one image confirms that image once, not once per
  picture (§4.8).

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
  eleven kinds (`HASHED`: Rectangle, Circle, Line, Arc, Polygon, Picture, Ellipse,
  PolyLine, PolyCurve, CurvePolygon, Tile) it folds `id`, the kind's own fields, the
  WHOLE `paint` record and the transform, in slot order, into a 64-bit multiply-xorshift
  that is shifted to 62 bits. A `Tile` has no `id`, `paint` or transform: it folds its
  image handle and four `vector::Float2`s. `Text`, `Group`, `AutoPaint` and
  `ParticleSystem` (`NOT_HASHED`) answer -1 and
  are hashed in MFBASIC by `helper_geometry.rs:__canvas_hashItem` with the two-lane
  `__canvas_hashStep`; its paint walk is `__canvas_hashPaint`.
- The worker computes the hashes in `canvas::present`:
  `func_carried_hashes.rs:lower_carried_hashes` keeps the installed hash of every item
  whose payload bytes did not change, and `func_item_hash.rs:lower_scene_hashes` hashes
  the rest. They are published beside the scene (`canvas::publishHashes`) under the
  scene's revision.
- The graphics thread resolves every scene index by that hash
  (`func_geo_cache.rs:lower_scene_resolve`): `emit_find` in `__CANVAS_GEO_TABLE`; a hit
  is trusted on the hash alone (`emit_hit`, pictures and tiles excepted:
  `PICTURE_KIND_BITS`, `TILE_KIND_BITS`); a miss of a natively
  built kind is built in place (`emit_fill_record`, `emit_add_slot`,
  `generations` + 1), anything else is left to `__canvas_appendDraw`.
- The same hash, folded with the entry's group offset and matrix
  (`helper_render.rs:__canvas_noteEntry`, natively `emit_hash_step_zero`), is the draw
  hash `helper_damage.rs:__canvas_damageFor` compares frame to frame.

### 2.2 What today's record carries that is appearance, not geometry

A record is `__CANVAS_GEO_HEADER` = 69 floats (`HEADER_SLOTS` in
`runtime/canvas/mod.rs`) and a tail. Read from
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
| 67, 68 | rim, x and y (bug-1083) | geometry |
| 69–72 (tile tail) | a `Tile`'s `srcPosition` and `srcSize` | geometry |
| 35, 36 (tile) | the atlas's pixel block address, as a picture's | content identity (§4.6) |
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

### 2.4 Measured: the group and picture walks (added 2026-10-13)

`benchmark/canvas` (macOS, Metal, a `--debug` build in a real window, display sync on;
`benchmark/canvas/README.md` defines the columns). From its committed baseline,
`benchmark/canvas/baseline/canvas.log` (run `20261013-190605` at `8b43850b0`, Apple M2
Max, 120 Hz), per frame, `walk` = `phaseOffsetsMs` + `phaseDrawsMs` averaged over 119
frames, the rest medians:

| Row | walk ms | work ms | gpu ms | encode ms | draws | passes |
|---|---:|---:|---:|---:|---:|---:|
| `group fill 10000` (10,000 `Group` nodes naming one one-rectangle group) | 14.1 | 10.5 | 6.9 | 2.2 | 10,001 | 1 |
| `picture fill 10000` (10,000 `Picture`s of one 32×32 image in a circle) | 12.0 | 21.3 | 18.8 | 0.05 | 2 | 2 |
| `rect fill 10000`, for scale | 0.4 | 7.2 | 6.0 | 0.03 | 1 | 1 |

That baseline predates plan-173. Its group walk had no viewport test, and it has no
`tile` rows. Phases 5 and 6 therefore measure their own "before" at their base, and the
figures above are context, not the bar.

What a group node costs, read from `helper_render.rs:__canvas_appendDraw`'s `Group`
arm: every node, every frame, resolves its name (`canvas::groupResolve`), copies the
group's item list (`canvas::groupItems`), and hashes each child again in MFBASIC
(`__canvas_hashItem`), although a group's items change only at `setGroup`, which bumps
its revision (`canvas::groupRevision`). Each node is also its own draw-list entry, so
10,000 nodes are 10,001 draw calls a frame, and encoding them is 2.2 ms.

What a picture costs: every picture is confirmed against its image's current pixel
block every frame (`__canvas_pictureCached`, `__canvas_geoStamp`), and its draw hash
folds the block address in (`__canvas_noteEntry`), although all 10,000 name one image.

**Where the graphics thread's time goes**, by `sample` (self time, one-millisecond
samples, 8 s of a 400-frame run of each row; scratch build of `benchmark/canvas/mfb`
with `--debug` at `09eaa48f3`; command in §9). Sampling slows the run (the walk read
25.9 and 25.1 ms a frame under it), so read the SHARES, not the times:

| Share of the busy graphics thread | `group fill 10000` | `picture fill 10000` |
|---|---:|---:|
| `--debug` perf timers around every arena alloc and free (`mach_absolute_time`, `gettimeofday`, `mfb_rt_perf_*`) | 38.6% | 27.1% |
| arena free scrub (`mfb_arena_fill_random`, every build) | 20.8% | 18.1% |
| arena alloc, free and drops | 4.1% | 2.2% |
| canvas walk code (MFBASIC and native helpers) | 18.8% | 41.0% |
| renderer, Metal, objc | 11.0% | 8.8% |
| the rest (stats line, its clock, other) | 6.7% | 2.8% |

The walk code's largest self times: for groups `canvas::groupItems`, `__canvas_hashStep`,
`__canvas_sceneOffsets`, `__canvas_pushOneDraw`, `canvas::itemHash`,
`__canvas_appendDraw`, `__canvas_sceneDrawsWalk`; for pictures `__canvas_geoStamp`,
`__canvas_hashStep`, `__canvas_pictureCached`, `__canvas_blockCost`,
`__canvas_sceneOffsets`, `__canvas_passNext`, `__canvas_flatScene`, `__canvas_hashFloat`.

What the measurements say:

1. **Most of both walks is allocation.** Two thirds of the busy group thread and
   nearly half of the picture thread is in the arena or around it. The `--debug` perf
   section times every `_mfb_arena_alloc` and `_mfb_arena_free`, so a `--debug` walk
   overstates allocation-heavy code; the free scrub is in every build. The per-node and
   per-picture lists the walk builds and drops are the cost to remove, and fewer
   allocations is how a phase here shows its gain.
2. **A group node repeats per frame what only `setGroup` changes**: the item list copy
   and the children's hashes. The draw-list entry per node is a separate cost (encode
   2.2 ms), taken on by plan-175-B and plan-175-C (decision 5).
3. **The picture row's second pass is real, not a miscount.** A picture in a circle
   writes 1,297 bytes of frame buffer a frame and its texels are copied once a pass
   (`profTexels` 1,024 a pass): 9,000 pictures draw in one pass and 10,000 in two
   (`KIND=picture COUNT=9000|10000 FRAMES=4` with `MFB_CANVAS_STATS`, last frame's
   `profPasses` and `profBytes`). So `__canvas_blockCost` and `__canvas_passNext` in
   that profile are §8.4's pass layout, not a picture cost.

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
- every item field is `Geometry` except `id` (§7, decision 1) and the `image` of a
  `Picture` or a `Tile` (content identity, §4.6).

`fold_field` takes the lane from the table. A field the table does not name fails the
build, as an unknown field TYPE does today, so a new `Paint` field or a new item kind
cannot be silently folded into the wrong lane or into neither. `Tile` (plan-173) is
the first kind the table must name that was not designed with it. Its `srcPosition`,
`srcSize`, `position` and `size` are `Geometry` and its `image` is content identity. It
has no `paint` and no `id`, so its `A` lane is empty: one constant for every tile, and
a tile is never refreshed.

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
  neither otherwise. `Text`, `Picture`, `Tile`, `AutoPaint` and `ParticleSystem` are
  never carried, as now (the kind list in `lower_carried_hashes`; `Text`, `Picture`
  and `Tile` fold a resource id that answers 0 once closed).
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
- **`Tile`** (plan-173): the same rule as a picture. `emit_hit` leaves it to the
  MFBASIC walk (`TILE_KIND_BITS`), and `__canvas_tileGeometry` confirms slots 35/36 with
  `__canvas_pictureCached` before trusting its record. It is one record under the
  item's own key, which becomes `G`, and is never refreshed (§4.1: its `A` is empty).
  A tile in a `Group` cannot see a `setBytes` at all: `canvas::setGroup` moves the
  atlas with the item list (plan-173 Correction 8). So only a directly presented tile
  needs the confirmation.
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

### 4.8 The group and picture walks

Both walks stay on the graphics thread and in the code §4.6 already changes; neither
changes a record, a key's meaning, or a pixel.

- **A group's children are hashed once per revision.** The graphics thread keeps, per
  group slot, the `(G, A)` pairs of its items and the revision they were computed at.
  `__canvas_appendDraw`'s `Group` arm reads the pairs when `canvas::groupRevision(slot)`
  still matches, and computes and stores them when it does not. A child's draw hash
  still folds the node's composed offset and matrix (`__canvas_noteEntry`), so damage is
  unchanged. The slot table is the graphics thread's own (`.ai/canvas-threading.md` §2:
  arena state is per thread), ~~and a group freed by the retirement drain of §13 there
  clears its slot's entry~~ keyed by the revision and the items block, and never
  cleared: a slot's revision only grows (Phase 5, the moot task).
- **No item list is copied per node.** The arm walks the group's items in place
  ~~through an index (`canvas::groupItemAt(slot, k)` and a count)~~ through the slot's
  items block, taken once per node (`canvas::groupItemsBlock`, `groupBlockCount`,
  `groupBlockItem`; Correction 7: an index by slot would read two lists in one node),
  instead of `canvas::groupItems` building a list per node per frame. The items are the
  retired-then-drained buffer §13 protects, so reading them in place is as safe within
  a frame as the copy is.
- ~~**A picture's image is confirmed once per frame.** `__canvas_pictureCached`'s answer
  for an image's pixel-block address is remembered for the frame, keyed by that address;
  the 2nd to 10,000th picture of one image reuse it.~~ **A cached picture or tile is laid
  out without being copied** (Correction 8: the confirmation is per record and is not
  the cost; the copy and the lists were). `canvas::itemImageShadow` reads the image's
  block from the item, and `__canvas_imageCached` resolves every cached record before
  the `MATCH` would copy the variant. `__canvas_noteEntry` still folds
  each picture's block address into its draw hash, so a `setBytes` still damages every
  picture of the image. The same path serves every `Tile` of the atlas; a tilemap is
  the case where one image is named most often.
- **Viewport culling is untouched, and it is part of the group walk's cost.** Since
  plan-173, `__canvas_pushDraw` calls `__canvas_boundsMeetViewport` for every block of
  every node (two `__canvas_drawBound` corner hulls under the node's matrix). The cost
  stays per node, because the test needs the node's own placement. Phase 5's "before"
  includes it, and Phase 5 must not cache a visibility answer across nodes or frames.
- **Fewer allocations is the measure.** Each change above removes a per-node or
  per-picture list. Phase 5 and 6 record `perf.mfb_alloc.count` and
  `perf.mfb_free.count` from the `--debug` report (`mfb spec tooling debug-report`, the
  perf section) for their row before and after, alongside the walk time — and the
  graphics arena's `arena.<n>.alloc_calls` from the same report, because
  `perf.mfb_alloc.count` does not count that arena (Phase 6: unchanged at 349,096
  while the graphics arena's calls fell 6,810,176 → 2,040,176; Correction 8).

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
- Phase 5 adds a per-slot table of child hashes to the graphics thread's group state, and
  a native `canvas::groupItemAt` beside `canvas::groupItems`; Phase 6 adds a per-frame
  map from pixel-block address to confirmation. None is program-visible.

## Phases

### Phase 1 — Two lanes, one damage identity (no behaviour change)

The classification table and its build-time guard; the native walk and the MFBASIC
walk each produce `(G, A)`; the worker publishes both and carries both; every consumer
is handed `D = mix(G, A)` and behaves as now. The cache is still keyed by `D`.

- [x] `func_item_hash.rs`: the field classification (`func_item_lanes.rs`), `fold_field`
      by lane (`fold_item_field`, `fold_paint`), the three geometry predicates (the
      stroke alpha's zero-ness, the stop count, `strokeApply`, each folded into `G` as
      its own word — `__canvas_strokeHalf`'s inputs, Correction 4's notes), `D =
      finish(fold(fold(SEED_DRAW, G), A))`; `canvas::itemGeoHash` answers `G`.
- [x] `helper_geometry/cache.rs`: `__canvas_hashPaintGeometry` /
      `__canvas_hashPaintAppearance` (and `__canvas_hashPaint`, both in one lane, for the
      kinds hashed whole); `__canvas_geoHash` (G), `__canvas_drawHashOf` (D from G),
      `__canvas_mixHash`; `Text` is `mix(textGeo, appearance)` with no `id`;
      `__canvas_pictureHash` is `D` and `__canvas_pictureGeoHash` (new hook, both chunks)
      `G`, neither with the `id`. A `Group` node, an `AutoPaint` and a `ParticleSystem`
      fold everything in one lane, so their `G` is their `D`. The `Tile` arm is the
      native walk's (`Tile` has no appearance field: its `A` is the empty lane's seed).
- [x] `func_carried_hashes.rs` (carries both words), `func_item_hash.rs:
      lower_scene_hashes` and `helper_render/scene.rs:__canvas_hashScene` /
      `__canvas_hashLayers` (2n words: every `D`, then every `G`),
      `func_geo_cache/resolve.rs` (the index count is half the list). `func_present*.rs`
      and `lower_scene_snapshot` needed no change: they pass the list whole.
- [x] Unit tests: `builtins/tests/canvas_item_lanes.rs` — every `Paint` field and every
      field of the eleven kinds, the five of `Tile` included, has its lane, and an
      unclassified `Paint` field fails (`cargo test --release --bin mfb -- canvas_item`
      → 10 passed, the 7 guard tests of `canvas_item_hash_guards.rs` among them). The
      value tests of `G` are Phase 2's (Correction 4). Decision 1's obligation:
      `rt_canvas_hit.rs:over_id_answers_a_renamed_item` (passes) and
      `rt_canvas_damage.rs:an_id_only_change_damages_nothing` (passes; at the base the
      rename rebuilt, `generations=3`, and repainted, `partial=1`).
- [x] The collision argument of §4.5 in the module doc (`func_item_hash.rs`) and
      `.ai/canvas-threading.md` §14; the spec's "The item unions" paragraph on the two
      identities.

Acceptance: frames are byte-identical to the base on Metal, box 2226 and box 206, and
`generations` is unchanged for every bench row.
  Check: `cargo test --release --test rt_canvas_geo_native --test rt_canvas_damage
  --test rt_canvas_metal --test rt_canvas_hit --test rt_canvas_autopaint` → pass.
  Check: `scripts/test-canvas-gpu-rows.sh <mfb> --target windows-x86_64 --keep <dir>`
  and `--target linux-aarch64 --keep <dir>` against the base compiler's `--keep`: every
  `*-gpu.rgba` equal by `cmp`.
  Check: `tools/canvas-bench/bench.sh <mfb> dots 60000 5` → `builds/frame=60000`.

**MET 2026-10-25**, against the base compiler at `2f281c5e7` (a detached worktree):
- the five suites → autopaint 23, damage 12, geo_native 4, hit 15, metal 74 passed;
- `scripts/test-canvas-gpu-rows.sh <mfb> --target windows-x86_64 --keep` and
  `--target linux-aarch64 --keep`, base and Phase 1 → box 206 and box 2226 each 124
  kept frames (62 render rows, software and GPU) equal by `cmp`, `generations=` equal
  on all 66 rows, 256 `ok:` and no `FAIL` either way;
- Metal: the same 62 render rows drawn headless on this Mac's GPU with both compilers
  (`MFB_CANVAS_GPU=1 MFB_CANVAS_GEO_VERIFY=1 MFB_CANVAS_SYNC=1`, a one-off probe of the
  script's row list) → 62 GPU frames equal by `cmp`, every row's `gpuFrames`,
  `generations` and `geoVerifyMismatches=0` equal. (`benchmark/canvas`, which would
  have been the Metal check, needs a live display, which this session has not: its first
  row stops with `gpuFrames=0 of frames=120`.)
- `tools/canvas-bench/bench.sh target/release/mfb dots 60000 5` → `builds/frame=60000`.
Commit: 6d98d7e3c

### Phase 2 — The cache keyed by `G`; native refresh for solid paints

- [x] `func_geo_cache/`: the slot's damage identity as the parallel list
      `__CANVAS_GEO_DRAWS` (Correction 5), appended by `emit_add_slot` and moved by the
      compaction; `lower_scene_resolve` branches 2 to 5 of §4.3 (`resolve.rs`); refresh
      through `emit_paint_appearance`, split out of `emit_paint_header`
      (`func_geo_build/mod.rs:emit_refresh_record`); `canvas::geoInsert(hash, draw, ...)`
      and the new `canvas::geoRefresh`. `emit_rebuild` reads keys only, so it needs no
      change. Dots under the verifier: `generations=2000 refreshes=240000
      geoVerifyMismatches=0 geoResolvedChecked=242000` over 121 frames of 2,000 dots.
- [x] `helper_geometry/cache.rs`: `__canvas_geoLookup` / `__canvas_geoStore` (branches 2
      to 5 in MFBASIC; branch 3 rebuilds in place through `canvas::geoRefresh`),
      `__canvas_geometryOf(item, geo, draw)`; `__canvas_geometryFor(item, key)` is it
      with `geo = draw = key`, for the callers whose key is their damage identity. The
      picture and tile records use the same protocol (`__canvas_pictureCachedOf`,
      Correction 5); `__canvas_appendDraw` carries `geo` beside `hash`.
- [x] `refreshes=` on the stats line (decision 2), after `generations=`; the debug-report
      spec's `MFB_CANVAS_STATS` row names it.
- [x] `tests/canvas/rt_canvas_geo_native.rs`:
      `a_paint_only_change_builds_nothing_and_refreshes_every_record` (scene
      `canvas_geo_native_repaint.mfb`: every kind, A, B, A),
      `two_items_of_one_geometry_and_two_paints_draw_right_in_either_order` (software and
      Metal), `a_frame_after_a_refresh_is_the_fresh_runs_frame` (software and Metal,
      byte for byte); the stroke alpha 0 to 1 case is the next task's frame 7. All pass
      (`cargo test --release --test rt_canvas_geo_native` → 8 passed).
- [x] Moved from Phase 1 (Correction 4), the same file:
      `appearance_fields_refresh_and_geometry_predicates_build` (scene
      `canvas_geo_native_lanes.mfb`): fill, stroke colour, blend, transparency and a
      ramp's colours each build 0; stroke alpha 0 → 255, one stop → two, `FillOnly` →
      `FillAndStroke` on a `Line` and no colours → three on a `PolyLine` each build 1.
      Passes.
- [x] `tests/canvas/rt_canvas_damage.rs`: `a_paint_only_change_damages_the_items_box`
      (partial repaint, damage the rectangle's box, `generations` unchanged, the new
      colour drawn). Passes (`rt_canvas_damage` → 14 passed).
- [x] `scripts/test-canvas-gpu-rows.sh`: a `repaint` row (`canvas_geo_native_repaint`,
      kind `geo`: one GPU run under `MFB_CANVAS_SYNC`, every record verified).
- [x] Found doing this: the alpha fade divided at transparency 0, where it is the
      identity (`emit_fade_alpha` now skips it; the dots row's offsets 8.74 → 7.76 ms on
      this Mac, verifier clean).
- [x] Found doing this: `rt_canvas_autopaint:an_autopaint_frame_costs_linear_in_its_
      instances` failed at the base too (26.0x, bound 25) because it measured whole
      milliseconds over five frames; fixed to sixty (8def521d9: base 21.7x, here 21.9x).

Acceptance: the dots scene builds no geometry after its first two frames, verifies
clean, and no pixel moves.
  Check: `tools/canvas-bench/bench.sh <mfb> dots 100000 5` → `builds/frame=0`.
  Check: `MFB_CANVAS_GEO_VERIFY=1` rows of `scripts/test-canvas-gpu-rows.sh` on box 206
  and box 2226 → `geoVerifyMismatches=0` on every row.
  Check: every `tests/canvas` target passes; the `--keep` frames equal the base's.
  Check, by measurement: ~~`phaseOffsetsMs` per frame at 100,000 on box 206 is within
  25% of the spike's 11.0 ms (the refresh's own cost is what the 25% allows)~~ — a
  prediction the measurements refuse (Correction 6): on box 206 the paint row's
  `phaseOffsetsMs` at most twice the base compiler's all-hit row and at least three
  times under the base compiler's paint row; on this Mac within 10% of the touch-only
  floor.

**MET 2026-10-25** (compiler of this phase frozen as one binary for every check):
- `dots 100000 5` on box 206 → `builds/frame=0 refreshes/frame=100000`; this Mac the same.
- `scripts/test-canvas-gpu-rows.sh <mfb> --target windows-x86_64 --keep` and
  `--target linux-aarch64 --keep` → 258 `ok:` and no `FAIL` on each box, the new
  `repaint` row `geoVerifyMismatches=0` (806 refreshes on 206), and all 124 kept
  frames of each box equal by `cmp` to the base's; Metal (the 62 render rows, headless)
  → 62 GPU frames equal by `cmp` to the base's, every row `geoVerifyMismatches=0`.
- every `tests/canvas` target → 24 of 24 `test result: ok`.
- Correction 6's bar: box 206 paint row 16.45 ms ≤ 18.0 (twice the base all-hit 9.00)
  and 3.3 times under the base paint row's 54.93; this Mac 7.76 ms ≤ 8.0.
Commit: f7c605784

### Phase 3 — Refresh for gradients, pictures and text (only where measured)

- [x] A bench row per case: `tools/canvas-bench/bench.sh <mfb> dots N --kind
      gradient|picture|text`, each with a `--control` row (one paint, a moving marker:
      every record the last frame's). Rebuild in place (Phase 2), 10,000 items, this Mac
      (load 21), `phaseOffsetsMs` paint / control, and the frame (offsets + draws +
      render) of the paint row:
      | case | paint | control | rebuild | of the frame |
      |---|---:|---:|---:|---:|
      | two-stop ramp's colours | 94.28 | 0.89 | 93.4 | 82% of 113.7 |
      | picture tint | 41.04 | 8.62 | 32.4 | 66% of 48.9 |
      | text colour | 124.00 | 0.46 | 123.5 | 85% of 145.4 |
- [x] Refresh for each case whose row shows the rebuild above 10% of its frame — all
      three are:
      - **the ramp and the text: implemented.** `__canvas_geoRepaint` builds only the
        kind's header (`__canvas_circleItem`, `__canvas_ellipseHeader`,
        `__canvas_rectItem`; for a `Text` the builder's own `__canvas_paintHeader` call
        and `__canvas_autoTextBox`) and the ramp's stops (`__canvas_gradientTail`), and
        the native `canvas::geoRepaint` copies the header's appearance slots (8–15, 26,
        42–46, 55–66) and the stops over the record — no edges, dashes or glyph run.
        `__canvas_geometryOf` tries it before rebuilding; other kinds rebuild whole.
        After (load 13): ramp 31.84 against control 0.37; text 34.49 against 0.34 —
        3.0 and 3.6 times less than the rebuild. Under the verifier, 1,000 items each
        (`dots.sh` with `KIND=...`, `MFB_CANVAS_GEO_VERIFY=1`): gradient 97,000
        refreshes, text 94,000, `geoVerifyMismatches=0` both.
      - **the picture tint: not implemented, cost recorded.** 42.42 against a control
        of 10.05 after (41.04 / 8.62 before): a picture's record is a header with no
        tail (`__canvas_picturePlain`, `__canvas_pictureStamp`), so its rebuild in place
        already is header-only, and what a refresh would save is the MFBASIC picture
        walk around it (`__canvas_pictureAppend`, two cache probes a picture), which is
        Phase 6's subject. Verifier clean (111,000 refreshes, 1,000 pictures).

Acceptance: each implemented case builds nothing on a paint-only change and verifies
clean; a case not implemented is recorded here with its measured cost.
  Check: the new bench rows; `cargo test --release --test rt_canvas_font --test
  rt_canvas_picture --test rt_canvas_golden`.

**MET 2026-10-25**: `builds/frame=0` on every paint row above; the verifier runs above;
`rt_canvas_font` 61, `rt_canvas_golden` 28, `rt_canvas_picture` 16 passed;
`rt_canvas_geo_native` 8 passed (its repaint scene's ramped circle, ellipse and turned
rectangle take the new path).
Commit: e0142e34a

### Phase 4 — The worker's hit cache keyed by `G`

- [x] `helper_hit.rs:__canvas_hitGeometry`, `__canvas_hitAppend`: key by `G`
      (`__canvas_geoHash`; an `AutoPaint` by its shape's `G`). A hit record reads no
      appearance slot (§2.2), so it is never refreshed: its key is its whole identity.
- [x] `tools/canvas-bench/bench.sh <mfb> hit`: the `1p` row (ids, every item recoloured
      a frame; `gen/hit.sh PAINT=1`).

Acceptance: a paint-only frame builds nothing on the worker; `overID` is unchanged.
  Check: `cargo test --release --test rt_canvas_hit --test rt_canvas_pointer`.

**MET 2026-10-25**: `rt_canvas_hit` 16 passed, `rt_canvas_pointer` 2 passed.
`tools/canvas-bench/bench.sh <mfb> hit` (this Mac, load 15 to 20), poll-first median at
10,000 items: the `1p` row 5,471 µs against the `1` row's 5,313 (one item moved, so one
rebuilt), where the base compiler took 17,463 against 11,758 — a paint-only frame costs
the worker what one changed item does, so it builds nothing; `overID` `[i484]` on every
row, 60 events each.
Commit: 6c96fa40e

### Phase 5 — The group walk: children hashed once per revision, no list per node

Depends on Phase 2 (the walk hashes `(G, A)` and resolves through §4.3).

- [x] `helper_render/gpu_groups.rs:__canvas_appendDraw`, `Group` arm: the per-slot
      child-hash table of §4.8 (`__canvas_groupHashes`: every child's `D`, then every
      `G`), read while the slot's revision AND items block both match; children walked
      in place with no list copied. `gpu_draw_list.rs:__canvas_memoGroup` reads the same
      table and block, and `__canvas_drawGroup` copies out only the nested `Group`
      children (`canvas::groupBlockIsGroup`, a tag test). Correction 7.
- [x] ~~`canvas::groupItemAt(slot, k)`~~ `canvas::groupItemsBlock(slot)`,
      `groupBlockCount(block)`, `groupBlockItem(block, k, none)` and
      `groupBlockIsGroup(block, k)` (native, `func_group_block.rs`, beside
      `canvas::groupItems`) — Correction 7.
- [x] ~~the table's entry cleared where §13's drain frees a group buffer~~ — moot: a
      slot's `CANVAS_GROUP_REVISION` has one writer, `gen_group.rs`'s `setGroup`
      lowering (`load`, `+ 1`, `store`; `grep -rn CANVAS_GROUP_REVISION src/` → that
      store, the `groupRevision` read and a layout test), and `removeGroup` zeroes only
      the name (`store_u64(&zero, &found, CANVAS_GROUP_NAME)`), so a slot refilled after
      a drain carries a revision no entry was filed under. The entry is also keyed by the
      block address, so even a revision collision would need the same block.
- [x] `tests/canvas/rt_canvas_damage.rs` and `rt_canvas_graphics_thread.rs`: a
      `setGroup` that changes one child's paint and another's geometry redraws both,
      under `MFB_CANVAS_GEO_VERIFY=1` and under `MFB_CANVAS_DAMAGE=1`; a group replaced
      while a frame holds it (R13 to R15 of `.ai/canvas-threading.md` §8) still draws
      what it resolved.
      `rt_canvas_damage_groups.rs:a_set_group_changing_one_childs_paint_and_anothers_geometry_redraws_both`
      (damage on and off, byte-equal, `geoVerifyMismatches=0`, and on Metal) and
      `a_group_replaced_while_a_frame_holds_it_draws_what_the_frame_resolved` (the held
      frame draws the old block, the next the new) → both pass. With the table's key
      cut to "any entry" (revision and block ignored), the first FAILS
      (`panicked at tests/canvas/rt_canvas_damage.rs:1195`, inside its `check`, before the split).

Acceptance: no pixel moves; the group row allocates less and walks faster.
  Check: `cargo test --release --test rt_canvas_geo_native --test rt_canvas_damage
  --test rt_canvas_group_ownership --test rt_canvas_graphics_thread` → pass.
  Check: `./benchmark/canvas/run.sh --only '^group '` then `compare.py` → every `group`
  row's `.sums` unchanged; `group fill 10000`'s walk average below the baseline's 14.1 ms
  and below the same row measured at this phase's base (`compare.py --base <base run>`).
  Check: `perf.mfb_alloc.count` from the `--debug` report of a 120-frame
  `KIND=group COUNT=10000` run of `benchmark/canvas/mfb`,
  before and after, recorded here.

**MET 2026-10-25**, against the base compiler at `e0142e34a` (a detached worktree):
- the four suites → damage 14 (now 15), geo_native 8, graphics_thread 20 (now 21),
  group_ownership 19 passed, before the two new tests; the new tests pass on their own;
- the `group fill 10000` row, headless on Metal (Correction 7), five interleaved runs
  each: walk 15.017, 13.504, 13.769, 13.547, 13.547 ms a frame at the base (median
  13.55) against 12.752, 12.658, 12.735, 12.821, 15.650 (median 12.75), so below both
  the base and the baseline's 14.1; the frame hash `d9a59c34efd34ce3` on every run of
  both compilers; 120 GPU frames each;
- `perf.mfb_alloc.count` 349,089 → 344,092 (`perf.mfb_free.count` 348,911 → 353,908).
  The graphics arena's allocation calls are unchanged (14,438,551 → 14,438,686): the
  list copy per node is gone, but each child is still copied out of the block once per
  node (`canvas::groupBlockItem`), which is one allocation where the list copy was
  one. The bytes it scrubs on free fall 5.87 GB → 4.24 GB over the run.
Commit: 44925e237

### Phase 6 — The picture walk: one confirmation per image per frame

Depends on Phase 2 (pictures and tiles are keyed by `G`, §4.6).

- [x] ~~`helper_picture.rs`: `__canvas_pictureCached`'s answer remembered per frame by
      pixel-block address (§4.8), reset at the frame's start. Both `__canvas_pictureCached`
      callers use it: the picture paths and `__canvas_tileGeometry`.~~ — moot: there is no
      per-image answer to share, and the confirmation is not the cost. Each record stamps
      the block itself (`__canvas_pictureStamp`), so the check is per record, three reads.
      `sample` of `picture fill 10000` at this phase's base (Correction 8):
      `__canvas_pictureCachedOf` 41 and `__canvas_geoStamp` 15 of the graphics thread's
      ~2,630 busy samples, against 880 in the `--debug` timers around each allocation.
      Replaced by the task below.
- [x] (added, Correction 8) The walk lays out a cached tile or picture with no
      allocation of its own: `canvas::itemImageShadow(item)` (native,
      `func_item_image.rs`) reads the image's pixel block from the item in place, and
      `__canvas_appendDraw` resolves a cached one through `__canvas_imageCached` (plain
      record, then a shaped picture's two) before the `MATCH` that would copy the
      variant; the shaped hit fills `__CANVAS_PIC_*` instead of making a list
      (`__canvas_pictureShapedHit`), and `__canvas_pictureAppend` no longer makes an
      empty list per call or probes the plain record twice.
- [x] `tests/canvas/rt_canvas_picture.rs`: many pictures of one image, then a `setBytes`
      on it, redraw every one of them with the new pixels (R9 and R10 of
      `.ai/canvas-threading.md` §8 still hold); a closed image draws nothing in every
      picture that named it (R11). `set_bytes_repaints_every_picture_of_the_image` (and
      R10), `set_bytes_twice_leaves_the_last_on_every_picture` (R9),
      `a_closed_image_draws_nothing_in_every_picture` (R11; 100 pictures, half plain,
      half in a circle) → pass. With the hit's block confirmation removed, the first
      FAILS.
- [x] `tests/canvas/rt_canvas_tile.rs`: many directly presented tiles of one atlas,
      then a `setBytes` on it, redraw every one of them with the new pixels (the
      many-tile twin of `set_bytes_on_a_presented_tile_image_repaints`).
      `set_bytes_repaints_every_tile_of_the_atlas` (200 tiles, `GEO_VERIFY`) → passes;
      FAILS with the confirmation removed.
- [x] Measure `tile fill 10000` at this phase's base and after, because the committed
      `benchmark/canvas` baseline has no `tile` row (plan-173 added it after the baseline
      run). Below.

Acceptance: no pixel moves; the picture row allocates less and walks faster.
  Check: `cargo test --release --test rt_canvas_picture --test rt_canvas_damage
  --test rt_canvas_geo_native` → pass.
  Check: `./benchmark/canvas/run.sh --only '^picture '` then `compare.py` → every
  `picture` row's `.sums` unchanged; `picture fill 10000`'s walk average below the
  baseline's 12.0 ms.
  Check: `./benchmark/canvas/run.sh --only '^tile '` at the base and after, compared
  with `compare.py --base <base run>` → every `tile` row's `.sums` unchanged, and
  `tile fill 10000`'s walk average lower after than at the base.
  Check: `perf.mfb_alloc.count` from the `--debug` report of a 120-frame
  `KIND=picture COUNT=10000` run of `benchmark/canvas/mfb`, and of the same with
  `KIND=tile`, before and after, recorded here.

**MET 2026-10-25**, against the base compiler at `44925e237` (a detached worktree), with
the headless probe of Correction 7:
- `cargo test --release --test rt_canvas_picture --test rt_canvas_damage --test
  rt_canvas_geo_native` (and `rt_canvas_tile`, `rt_canvas_graphics_thread`,
  `rt_canvas_damage_groups`, `rt_canvas_particles`, `rt_canvas_autopaint`) → picture 19,
  damage 11, geo_native 8, tile 7, graphics_thread 21, damage_groups 4, particles 29,
  autopaint 23 passed;
- `picture fill 10000`, three interleaved runs each: walk 12.949, 12.607, 13.513 ms a
  frame at the base (median 12.95, the baseline's 12.0 being a windowed number) against
  7.692, 7.470, 7.043 (median 7.47); frame hash `318834a94296f27b` on every run;
- `tile fill 10000`: 6.530, 5.487, 5.350 (median 5.49) against 3.821, 3.718, 3.667
  (median 3.72); frame hash `88bef16356cbf73c` on every run;
- allocations: `perf.mfb_alloc.count` is unchanged (picture 349,096 → 349,098, tile
  349,097 → 349,097): it does not count the graphics thread's arena. That arena's
  `alloc_calls` from the same report falls 6,810,176 → 2,040,176 (picture) and
  3,853,459 → 1,473,459 (tile), and the bytes it scrubs on free 2.76 → 1.75 GB and
  1.28 → 0.98 GB. What is left is about one allocation an item a frame, the frame's
  copy of the scene (`__canvas_flatScene`) and its element copy in pass 2.
Commit: 7cdedfffb

### Phase 7 — Spec, threading notes, closeout

- [x] (added, Correction 9) Regenerate the canvas change-sentinel goldens after the
      plan's last code change (letters B and C move them again): `cargo test --release
      --bin mfb` → `replay_shard_05/08/23` fail on `syntax/app/app-mouse-surface`,
      `syntax/resources/canvas-append-unconsumed-valid` and
      `syntax/resources/loop-body-resource-reference-valid` (`.ir` differs: the canvas
      helper text's lines and globals moved). Classify against main with
      `scripts/artifact-gate.sh <mfb> all` from a detached main-tip worktree, then
      regenerate every flagged fixture, and run `scripts/test-accept.sh` in full.
      Main's tip `ba92007cd` (merged into this branch at `e0d3b08b0`, no file changed on
      both sides) → `2387 golden(s) checked, 0 diff(s)`. The merged branch → 10 diffs in
      four fixtures, all canvas programs: `app-mouse-surface`'s `.ir`, macOS `.nir`,
      `.nplan` and four `.app.ncode` sums (the walk helpers and both emitters);
      `app-window-surface`'s macOS `.app.ncode` sum only (the Metal emitter — its `.ir`
      did not move); and the two `syntax/resources` canvas fixtures' `.ir`. Those ten
      regenerated with the gate's own build flags; then the gate → `0 diff(s)` over
      2,387; `scripts/test-accept.sh <mfb> target/accept-actual` → `acceptance tests
      passed (1757 test(s) ran)`; `replay_shard_05/08/23` → 3 passed.
- [x] The text of §4.7. `.ai/canvas-threading.md` §14 (Phases 1–2), §16 (an
      `AutoPaint`'s one identity) and §17 (the keys; a cached picture or tile laid out
      without a copy); `06_canvas.md`: "Retained, not immediate" (Phase 2), "The item
      unions" and a new "Item identity" heading (the cache's branches, the damage diff,
      the hit cache), "Named groups" (the in-place block and per-slot identities),
      "Hit-testing" (keyed by `G`), "One frame" (`__canvas_autoGeoHash` is both
      identities), "A picture is drawn inside a shape" (the keys, `itemImageShadow`).
- [x] `scripts/spec-census.sh --render` and `--citations`. → render: 0 leaks in every
      package; citations: 2,846 unique, `MISS-PATH 0`, `MISS-LINE 0`, `MISS-SYMBOL 0`,
      after repointing two citations this plan did not write but the run found stale
      (`10_debugger.md` `Breakpoints::set` → `instruction_boundary`, the refusal the
      sentence cites; `12_monomorphization.md` `Monomorphizer::new` ×2 →
      `overload_names`, the map the sentences describe — the literal text left those
      files in earlier refactors, `git log -S`).
- [x] Re-run §2.3's rows on box 206 and this Mac alone (load under 4) and record them
      in bug-765. Recorded there ("Re-measured after plan-175-A and plan-175-D"): at
      100,000 dots, box 206 4 / 4 → 11 / 12 pps / fps (offsets 55.9 → 16.2 ms, render
      213.6 → 63.9 ms, worker 88.8 → 57.5 ms), this Mac 34 / 35 (load 3.83 at the start,
      7.69 at the end: the rows' own builds), `builds/frame` 0 on every paint row.
- [x] Re-run `benchmark/canvas` in full; if the group or picture rows moved by 1.25× or
      more, re-baseline it in its own commit with `compare.py`'s output. Run attended,
      after the fix that lets `run.sh` run at all since bug-1087 (`a9edb077b`, Correction
      10), with plan-175-C landed, so one re-baseline serves A and C; against main's tip
      in the same session: `group fill 10000` work 5,260 → 2,719 µs, `tile fill 10000`
      3,178 → 2,208, every pixel hash equal. Re-baselined
      (`benchmark/canvas/baseline/README.md` holds `compare.py`'s output and the bisected
      `gpu` power-state note).
- [x] `.ai/canvas-threading.md` §13 (the child-hash table and its drain) and §17 (one
      confirmation per image per frame). §13's "There is no refcount" paragraph (Phase
      5: the block read in place, the table keyed by revision and block, no clearing
      because a revision only grows); §17 per Correction 8: the confirmation stays per
      record, and what Phase 6 removed is the copy (`canvas::itemImageShadow`).
- [x] The arena free scrub's share of the busy graphics thread in `group fill 10000`
      and `picture fill 10000`, re-profiled as in §9, recorded here against §2.4's
      20.8% and 18.1% (decision 6). At `7cdedfffb`, windowed, `FRAMES=2000`, `sample
      <pid> 8` (self time on the thread holding `__canvas_sceneOffsets`, grouped as
      §2.4 groups it, by a one-off parser of the sample file). The scrub is
      `mfb_arena_fill_zero` now, a zero fill since `4bb55ea8c`, which predates this
      plan; §2.4's profile was of `mfb_arena_fill_random`.

      | Share of the busy graphics thread | `group fill 10000` | `picture fill 10000` |
      |---|---:|---:|
      | `--debug` perf timers | 53.1% | 7.6% |
      | arena free scrub (`mfb_arena_fill_zero`) | 1.0% | 0.6% |
      | arena alloc, free and drops | 5.1% | 1.7% |
      | canvas walk code | 25.9% | 61.9% |
      | renderer, Metal, objc | 8.3% | 8.5% |
      | the rest | 6.5% | 19.6% |

      Busy samples: 5,194 of 6,880 (group) and 3,621 of 6,940 (picture). The scrub has
      stopped being a cost: the zero fill made it about a twentieth of what §2.4
      measured, and the walks allocate less. In the group row the timers' 53% are the
      per-child copy (`canvas::groupBlockItem`, Phase 5's record) under the `--debug`
      build's per-allocation timing, which a release build does not pay.

Acceptance: the spec describes the implementation at HEAD.
Commit: 2cfee6f74 (docs), 339f8e8fc (rows, scrub), f22e9c033 (goldens), 29f049ff9 (benchmark)

## Validation Plan

- Tests: the unit and runtime tests named per phase. `MFB_CANVAS_GEO_VERIFY=1` on
  every runtime suite that sets it today, plus the new `repaint` row on both boxes.
- Coverage check: the classification guard is itself tested by a `Paint` field
  deliberately left out of the table in a unit test (the build of the walk must fail).
- Runtime proof: `dots 100000` on Metal and box 206 before and after each of Phases 1
  and 2, `--keep` frames compared by `cmp`.
- Walk proof: `benchmark/canvas`'s `group` and `picture` rows before and after Phases 5
  and 6 (`compare.py`; `.sums` unchanged is the pixel proof, the walk average the gain).
- Doc sync: Phase 5.
- Final gate (once, after the last phase): `cargo test`, `scripts/artifact-gate.sh
  <mfb> all`, the GPU rows on box 206 and box 2226.

## Decisions

Every decision below is decided; none is open.

1. **Is an item's `id` part of `G`?** Today it is folded, so two shapes that differ
   only in `id` have two records. Nothing in a record depends on `id` (the hit walk
   maps entries to ids by scene index: `__canvas_hitAppend`). **Recommendation: leave
   `id` out of `G` and out of `A`, and fold it into `D` only**, so renaming an item
   costs no geometry and still counts as a change for damage.
   **DECIDED by the owner, 2026-10-12:** `id` is only meant for mouse feedback. It is in
   neither `G` nor `A`, and NOT in `D` either: a changed `id` affects neither geometry nor
   paint, so it redraws nothing. This overrides the recommendation's last clause. What a
   phase of this plan must still prove: a scene that differs only in an `id` is still
   PUBLISHED (the content compare sees it), so `canvas::pollMouse`'s `overID` answers the
   new name, with no pixel redrawn for it.
2. **A `refreshes=` stats field.** Without it a test cannot tell "refreshed" from "hit".
   **Recommendation: yes**, cumulative like `generations=`.
   **DECIDED by the owner, 2026-10-12:** add it.
3. **The order of the remaining work.** §8 lists three costs larger than the one this
   plan removes. **Recommendation: do §8.1 (the worker) before this plan's Phase 2**,
   because it is the largest cost on the Mac and no part of this plan touches it; and
   treat §8.2 (the Intel draw) as its own bug, because the 30 fps goal on box 206 is
   out of reach until it moves.
   **DECIDED by the owner, 2026-10-12:** yes, as recommended: the worker's copy and compare (§8.1) comes before this plan's Phase 2.
4. **Should bug-765's goal keep "every supported real GPU"?** At 1.4 µs a circle the
   UHD 630 needs 140 ms to draw 100,000. **Recommendation: keep the goal for Metal, and
   set box 206's from what §8.2 can reach** once that is measured.
   **DECIDED by the owner, 2026-10-12: as recommended.** The goal stays for Metal; box
   206's is set from what the Intel draw cost can reach.
5. **One draw call per `Group` node** — 10,000 nodes are 10,001 draw-list entries and
   2.2 ms of encode a frame (§2.4). **DECIDED by the owner, 2026-10-13:** it is its own
   plan letters, after this one: `plan-175-B` moves each draw's offset and matrix out
   of the per-draw payload into a per-pass node table on both backends (no pixel
   changes), and `plan-175-C` merges adjacent identical group-node entries into one
   instanced draw. This file is renamed `plan-175-A` for it.
6. **The arena's free scrub on the canvas walk** — 18 to 21% of the busy graphics
   thread in §2.4's rows. **DECIDED by the owner, 2026-10-13: no change to the scrub.**
   It belongs where it is: `_mfb_arena_free` scrubs every freed chunk with
   `mfb_arena_fill_random`, on the CPU, on the freeing thread, in every build
   (`mfb spec memory arenas`, "always on, in debug and release"). On the graphics
   thread the memory freed is the MFBASIC canvas walk's own: every caller `sample`
   resolves is MFBASIC walk code or the drops it triggers (`__canvas_appendDraw`,
   `__canvas_sceneDrawsWalk`, `__canvas_sceneOffsets`, `__canvas_pictureQuick`,
   `mfb_rt_drop_owned_*`); the native Metal renderer makes no arena call
   (`grep -c arena src/target/macos_aarch64/app/metal.rs` → 0), and GPU memory is
   Metal's buffers, never the arena. Phases 5 and 6 cut the scrub by freeing less;
   Phase 7 records the share that is left.

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
   that member is generated from, or verified against. `picture fill 10000` (two passes)
   pays it in `benchmark/canvas` (§2.4).
5. **The `ParticleSystem` expansion** — 20.7 ms of walk a frame against 2.6 ms of render
   at 10,000 particles (`benchmark/canvas`, `particles fill 10000`). Every frame,
   `helper_particles.rs:__canvas_particleAppend` takes each live particle through its
   state, tint, shape (`__canvas_particleItem`) and hash (`__canvas_hashItem`) in
   MFBASIC, and builds its geometry with no cache (`__canvas_particleRecord`). No part
   of this plan touches that path except the two-lane hash of Phase 1, so it is its own
   plan and can go before this one.

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

§2.4's profile (2026-10-13) needs no edit to the compiler: copy `benchmark/canvas/mfb`
to a scratch directory, `mfb build -app --debug` it, run it in a window with
`KIND=group` (or `picture`) `COUNT=10000 FRAMES=400 MFB_CANVAS_SYNC=1
MFB_TRANSCRIPT_DUMP=1 MFB_CANVAS_STATS=<file>`, and after 6 s `sample <pid> 8 -file
<out>`. Self time per function on the thread whose tree holds `__canvas_sceneOffsets`
is each node's count less its direct children's; the six shares group those functions by
name (the timers: `mach_absolute_time`, `gettimeofday` and its commpage helpers,
`_mach_boottime_usec`, `mfb_rt_perf_*`; the scrub: `mfb_arena_fill_random`).

| # | Experiment | Command (macOS, headless, `MFB_CANVAS_GPU=1`) | Result |
|---|---|---|---|
| 1 | the geometry-only hash as the damage identity | 2,000 dots, built with `MFB_SPIKE_GEOHASH=1`, run with `MFB_CANVAS_DAMAGE=1` for 2 s | `frames=1 skipped=104`: every paint-only frame skipped. Today's hash: `frames=100 skipped=0` |
| 2 | the verifier against a stale paint | the same build, run with `MFB_CANVAS_GEO_VERIFY=1` | `geoVerifyMismatches=120000` over 120 frames. Today's hash: 0 over 102 frames |
| 3 | equal items share a record | 1,000 identical circles and one more of the same geometry and another paint, one frame | `blocks=1001 entries=2 generations=2` |
| 4 | the alternating scene never hits and compacts every frame | §2.3's `paint` rows | `geoCompactions` 1.00 a frame, `entries` = 2N |

## Corrections

When a phase finds a claim of this plan wrong, the claim is struck in place and the
correction is added here, dated, with the command that showed it.

1. 2026-10-13, before any phase: plan-173 landed first (merged at `bf951f32f`). These
   claims were corrected in place:
   - §2.1: `HASHED` is eleven kinds with `Tile`, and `NOT_HASHED` includes
     `ParticleSystem` (`rg -n '^const (NOT_HASHED|HASHED)' -A14
     src/codegen/builtins/canvas/func_item_hash.rs`).
   - §2.1: `emit_hit` skips tiles as well as pictures (`rg -n TILE_KIND_BITS
     src/codegen/builtins/canvas/func_geo_cache.rs`).
   - §2.2: the header is 69 floats, not 67 (`rg -n 'HEADER_SLOTS: usize'
     src/codegen/runtime/canvas/mod.rs` → 69). The table gains the rim slots and the
     tile tail.
   - §4.2: the never-carried list is `lower_carried_hashes`' five kinds.
   - §4.1, §4.6, §4.8 and Phases 1, 3 and 6: `Tile` classified and carried through.
   - §4.8: the group walk's per-block viewport test is recorded.
2. 2026-10-14: the particle expansion was not the benchmark's largest walk, as the
   2026-10-13 addition said (20.7 ms a frame at 10,000 particles). The baseline had a larger
   one that went unread: `autopaint fill 10000` walked 277 ms a frame
   (`awk '/^autopaint fill 10000:/{s=1} s&&/walk/{print;exit}'
   benchmark/canvas/baseline/canvas.log` → median `277000.0` µs). It grew with the square of
   the instance count (7.7, 20.1, 59.3 and 184.3 ms at 1,000, 2,000, 4,000 and 8,000
   instances): a scan of the instance table per instance (`helper_autopaint.rs:
   __canvas_autoRow`, and `__canvas_autoVariant` per group use), and a copy of the walk's
   entry list per instance (`__canvas_autoAppend` handed its parameter straight on). It is
   a bug, not this plan's scope, fixed on its own (9a0723cc6, test
   `an_autopaint_frame_costs_linear_in_its_instances`). Corrected in place in the
   2026-10-13 paragraph at the top; this plan's group and picture walks and §8.5 are
   otherwise unaffected.
3. 2026-10-25, at the Prerequisites gate: decision 3 orders §8.1 (the worker's copy and
   compare) before Phase 2, but no letter covered it — a log search of commit messages
   since 2026-10-12 for "copy and compare" finds only this plan's own commits, and
   `emit_publish` still copies before it compares (`grep -n copy_flat_block
   src/codegen/builtins/canvas/gen_present.rs`). It is a missing prerequisite, landed as
   the new letter `plan-175-D`, with the edge D → Phase 2 in the header. Its §2 measures
   what §8.1 left unmeasured: on this Mac at 100,000 dots the worker spends its present in
   `carriedHashes`, `publishScene`, `sceneHashes` and (for an unchanged scene)
   `__canvas_groupSignature` (`sample`, plan-175-D §2).
4. 2026-10-25, Phase 1: three corrections to §4.1 and §4.2, none to the design's effect.
   - **The published pair is `(D, G)`, not `(G, A)`.** Every use of `A` in §4.3 compares
     it with the `A` of the slot `G` already chose, and for one `G`, `D = mix(G, A)`
     equal is `A` equal (up to the 2^-62 of §4.5). So the list holds every item's `D`
     at `i` and its `G` at `n + i` (2n words, one revision): every reader that wants
     "the item's hash" keeps reading index `i` and gets `D`, with no change, and the
     slot's fifth word (Phase 2) holds `D`. `A` itself is never stored.
   - **An item field's lane is a rule, not a table row.** §4.1 already says every item
     field is geometry except `id` and a resource; `func_item_lanes.rs:item_field` is
     that rule, plus a `PolyLine`'s colours. A table whose missing row failed the build
     would have refused the synthetic kinds of `canvas_item_hash_guards.rs` (fields `n`,
     `bytes`, `holder`, ...) before the refusals those tests pin. The rule is total and
     defaults to `G`, the lane where a mistake can only rebuild a record. The class of
     every canvas field is pinned by `builtins/tests/canvas_item_lanes.rs` instead.
     `Paint` keeps the strict table: a field `PAINT_FIELDS` does not name fails the
     build.
   - **`G`'s value tests move to Phase 2.** In Phase 1 the cache is keyed by `D`, and
     no test can call an internal member, so nothing at run time observes `G`. "Two
     paints differing only in appearance have one `G`" and "each geometry predicate
     changes `G`" are asserted in Phase 2 through `generations=` and `refreshes=`, where
     they are the behaviour (Phase 2's test task lists each).
5. 2026-10-25, Phase 2: four corrections to §4.3, none to its effect.
   - **The slot's damage identity is a parallel list, not a fifth slot word.** Slots
     are indexed `slot << 2` / `slot * 4` in the native table, the compaction, the
     resolve and five MFBASIC readers (`__canvas_geoStamp`, `__canvas_geometryOf`, the
     picture lookups, the hit path, the stats line's `entries=`). `__CANVAS_GEO_DRAWS`
     holds one word a slot in slot order, appended by `emit_add_slot` and moved by the
     compaction with its slot; nothing that reads a slot changes.
   - **Branch 4's second key is `D` itself**, not a new `mix(G, A)`: `D` already is that
     mix, and it never equals a `G` but by a 62-bit collision.
   - **The native refresh is `emit_paint_header`'s appearance half, as §4.3 says — a
     first cut that rebuilt the whole record in place was measured and refused.** The
     first cut wrote `emit_fill_record` over the slot's floats, on the theory that
     §2.3's cost was the churn and not the build. On box 206 at 100,000 dots it gave
     `builds/frame=0` and `phaseOffsetsMs=22.4` against the 13.75 the acceptance allows
     (`tools/canvas-bench/bench.sh <mfb> --target windows-x86_64 --host
     test@192.168.1.206 dots 100000 5`); dropping its needless push onto the verifier's
     built-list (the verifier re-checks every resolved index anyway) gave 18.8. So the
     refresh writes only `emit_paint_appearance` (slots 8–15, 26, 42–46, split out of
     `emit_paint_header`), a `Line`'s stroke-as-fill copy, and nothing for an empty
     record; a `PolyLine` alone, whose tail holds colours, is rebuilt whole in place.
     It decides "built natively" by `emit_record_size`'s support checks without the
     sizing: the kind, the transform, the stop count and a rectangle's corners are all
     geometry, so a record found by `G` was built the way the item would be now. The
     MFBASIC path rebuilds in place (`canvas::geoRefresh` writes the builders' fresh
     record over the slot's floats, length checked), which is §4.3's "rebuild in place".
6. 2026-10-25, Phase 2's last check: "within 25% of the spike's 11.0 ms" is a prediction
   the measurements refuse, and it is replaced by a bar they ground. The spike's `geo`
   variant read no item on a paint change; a correct refresh must read every changed
   item to learn its new paint, and that alone exceeds 25%. Measured (`dots 100000 4`,
   this Mac, `phaseOffsetsMs`):
   - every item a hit (`--control`: both scenes one paint, a marker moving): 5.08;
   - the refresh path doing nothing but its slot bookkeeping (a throwaway build): 6.42
     (+26%); touching each item's paint and nothing else: 7.27 (+43%);
   - the shipped refresh: 7.76 (+53%), after the fade fast path (8.74 before it);
     without its colour writes at all (throwaway): 8.55 of the 8.74.
   On box 206 (`--target windows-x86_64 --host test@192.168.1.206 dots 100000 5`),
   against the base compiler at `2f281c5e7`: paint 54.93 → 16.45 ms, all-hit control
   9.00 → 10.03 ms (the 2n-word hash list and the draw-identity check), release pps/fps
   8/9 → 12/13. The spike's 11.0 was taken at `ab9061ff8` and is not this code's floor.
   The check becomes: on box 206 at 100,000 dots, the paint row's `phaseOffsetsMs` is at
   most twice the base compiler's all-hit row (18.0 ms) and at least three times under
   the base compiler's paint row (18.3 ms); and on this Mac within 10% of the
   touch-only floor above (8.0 ms). Measured: 16.45, 3.3 times under, and 7.76.
   - **Pictures and tiles go through the same protocol** (`__canvas_pictureCachedOf`,
     replacing `__canvas_pictureCached`): keyed by `G` (stepped by
     `__canvas_pictureKey`), holding `D` (stepped the same way), with the pixel block
     confirmed before a hit is trusted, and a stale-pixel or recoloured record rebuilt
     in place. The keys handed to `__canvas_noteEntry` stay the `D` ones, so a tint
     change still damages. A picture on an `AutoPaint` keeps `G = D` (§4.6). This is
     §4.6's design landing with Phase 2 rather than waiting for Phase 3, because the
     picture walk calls the same lookup.
7. 2026-10-25, Phase 5: two corrections to §4.8 and one to the Phase 5 checks.
   - **`canvas::groupItemAt(slot, k)` would read two lists in one node.** Read by slot,
     each `k` re-reads the slot's items pointer, so a `setGroup` landing between child
     2 and child 3 of one node hands the node the old list's head and the new list's
     tail, filed under whichever revision was read. The walk instead takes the slot's
     block ONCE per node (`canvas::groupItemsBlock`, after `canvas::groupRevision`) and
     reads children from that block (`canvas::groupBlockCount`,
     `canvas::groupBlockItem`); the cache entry is keyed by the revision AND the block,
     so an entry filed while a replacement raced the two reads is never reused against
     the other block. The block is safe to hold for the frame for §13's reason — a
     displaced block is retired, not freed, until a frame completes.
   - **The first cut was slower than the copy it replaced** (28.6 against 15.0 ms
     walk): a `__canvas_noItem()` default built per node, and `__canvas_drawGroup`
     copying out every child only to skip the non-groups. One shared default
     (`__CANVAS_BLOCK_NONE`) and a tag test (`canvas::groupBlockIsGroup`) fixed both.
   - **`benchmark/canvas/run.sh` cannot run here**: its rows are windowed and
     display-synced ("attended, every window unobstructed", `baseline/README.md`), and
     ~~this session has no display~~ (corrected in Phase 7: windows do open from this
     session, but `run.sh --only '^rect fill 1$'` stops with `gpuFrames=118 of
     frames=120` — the first two frames are drawn before the window's Metal pipeline,
     built on its own thread, is ready — with the base compiler `2f281c5e7` exactly as
     with this branch, at load 4 to 12). The check is made with a headless probe of the same
     program instead (`benchmark/canvas/mfb`, `--debug`, `MFB_HEADLESS=1
     MFB_CANVAS_GPU=1 MFB_CANVAS_SYNC=1 KIND=group COUNT=10000 SIZE=small`, Metal),
     base compiler and Phase 5 alike: the walk is `phaseOffsetsMs + phaseDrawsMs` a
     frame over frames 3–120, the frame hash is the sha256 of a 2-frame run's dump (as
     `run.sh` makes its `.sums`), and the allocation counts come from the 120-frame
     run's `--debug` report. The baseline's 14.1 ms is a windowed number and is not
     comparable to the probe's; the base-compiler comparison is the one that holds.
8. 2026-10-25, Phase 6: §4.8's "a picture's image is confirmed once per frame" rested on
   the confirmation being the picture walk's cost, and it is not. `sample <pid> 6` of a
   headless 1,500-frame `KIND=picture COUNT=10000` run of `benchmark/canvas/mfb` built at
   this phase's base: of the graphics thread's 4,694 samples, 2,063 wait (GPU, redraw);
   of the ~2,630 busy, `mfb_rt_perf_perf_start`/`_end` (the `--debug` timers around every
   arena alloc and free) take 880, `__canvas_pictureKey` and its `__canvas_hashStep` 243,
   `__canvas_pictureCachedOf` 41 and `__canvas_geoStamp` 15. And there is no per-image
   answer to remember: each record stamps the block address itself
   (`__canvas_pictureStamp`), so confirming one says nothing about the next. What the
   walk did per cached picture per frame was allocate 5.7 times (`arena.<graphics>.
   alloc_calls` 6,810,176 over 120 frames, 10,000 pictures), against 0.2 for a native row
   (`rect fill 10000`: 23,012). Phase 6 removes the allocations instead: the variant copy
   that only read `image` (`canvas::itemImageShadow` reads it in place), the per-picture
   result list, and the empty list per call. Measured: 6.81M → 2.04M allocations and
   12.95 → 7.47 ms for pictures, 3.85M → 1.47M and 5.49 → 3.72 ms for tiles. Removing the
   duplicate plain probe alone measured no change (12.385 → 12.496 ms). The plan's
   per-frame memo task is marked moot with this evidence.
   - **The R11 row needs a changed scene.** A `destroyImage` signals no redraw (it is not
     one of §4's triggers) and a re-present of the same items is skipped, so
     `destroyImage` then `present(items)` draws no frame at this phase's base, at HEAD
     and at the plan's base `2f281c5e7` alike (one stats line; the picture's pixel still
     red). That is the frame-skip contract, not a defect; the 100-picture R11 test makes
     its frame as `set_bytes_then_destroy_then_a_frame_draws_nothing` does, with one
     more item.
9. 2026-10-25, Phase 6's check: `cargo test --release --bin mfb` had not been run since
   Phase 1, and six of its tests had gone red under this plan. Each is answered from
   evidence.
   - `helper_picture::tests::canvas_picture_only_the_plain_record_is_cached_under_the_item_hash`
     counted `__canvas_pictureInsert(hash,`, a function Phase 2 (`f7c605784`) replaced
     by the lookup that plans the key (`__canvas_pictureCachedOf(g, …)`) and
     `__canvas_pictureStore` (`git grep pictureInsert f7c605784 -- src` → only the test).
     Its invariant is restated in those names: only the plain probe and the Tile store
     under the item's own identity, and every other lookup is under a stepped key.
   - `builtins_codegen::canvas::the_skip_reports_false_and_the_publish_reports_true` read
     "the first `ret` after the publish label". plan-175-D moved the scene copy after
     that label, and `copy_flat_block`'s out-of-memory raise (`payload_copy.rs`) is now
     that first `ret`. The publish exit is the last `ret` before the `_retire_oom` cold
     path, and it reads `1`.
   - `…::retired_scene_blocks_are_freed_only_after_a_frame_completes` required ONE free
     ahead of the publish label: bug-683's skip freed the copy it compared. plan-175-D's
     skip makes no copy (`rt_canvas_present_leak.rs:an_unchanged_re_present_copies_nothing`),
     so the count is 0. The gate's own counts (4 behind it, 0 on the publish path before
     it, 1 on the cold path) are unchanged and pass.
   - `declared_errors_all::every_canvas_member_declares_exactly_the_errors_its_lowering_raises`:
     Phase 5's `canvas::groupBlockItem` declared `ErrOutOfMemory`, which its lowering
     never raises — the descriptor was wrong, now `[]`.
   - `raw_result_block_ownership::every_block_returning_runtime_helper_is_classified`:
     `canvas.groupBlockItem` returns a `DrawItem` that `lower_list_get_or` copies on the
     calling thread, in the caller's arena, so it is registered in
     `CALLER_ARENA_BLOCK_RESULTS`.
   - `build_replay::replay_shard_05/08/23`: three canvas programs' `.ir` goldens, the
     change sentinels `.ai/testing-gates.md` names for the canvas runtime. Regenerated
     once, after the last code change (Phase 7's added task).
10. 2026-10-25, Phase 7: `benchmark/canvas/run.sh` was not blocked by this session's
    display (Correction 7 and its Phase 7 correction). It had been broken since
    bug-1087 (`19da7eff3`, 2026-10-17) for every compiler: the window's pipeline is
    built off the critical path, so the first frames are software, and `reduce.py`
    refused any row with a software frame. Fixed in `a9edb077b`: the start-up prefix
    and the first Metal frame are dropped, and every frame after must be Metal's and
    direct. The windowed run against main's tip then showed the flat 10,000-item rows'
    `gpu` 1.3–2.5× higher. Bisected to plan-175-D (`17b7b529e`; plan base, D, A Phase 1
    and A Phase 2 alternated, two rounds each), whose change reaches no GPU input.
    Headless, frames back to back, D and its base give the same GPU and fill times
    within noise. So it is the power state of a lighter frame that still makes its
    vsync, recorded in the baseline README rather than treated as a defect.

## Summary

Added 2026-10-13: Phases 5 and 6 take the group and picture walks that `benchmark/canvas`
measured at 14.1 and 12.0 ms a frame for 10,000 items, after the split, because they run
through the code it rewrites. Both are mostly allocation (§2.4), so fewer lists per node
and per picture is the change, and the benchmark's `.sums` is the pixel proof.

The split is small in mechanism (one classification, a fifth word in a cache slot, nine
floats rewritten in place) and its risk is concentrated in one property the existing
verifier already checks: a refreshed record equals a freshly built one. The larger
risk is expectation. The measurements say this plan removes 45 ms of a 360 ms frame on
box 206 and takes the Mac from about 15 to about 25 frames a second at 100,000 dots;
the frame rate bug-765 asks for is behind the worker's copy and compare, the per-frame
block upload, and on Intel the GPU itself. Decision 3 asks the owner to order them.
