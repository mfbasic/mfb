# bug-686: The canvas is not usable at scale — caps send ordinary scenes to software, and the GPU path is bounded by MFBASIC per-item work, not the GPU

Last updated: 2026-09-23
Effort: huge (>3d)
Severity: HIGH
Class: Correctness / Performance

Status: Open
Regression Test: `examples/gpu` sweep and its 60 fps rows (Phase 0, gating every later phase);
`tests/canvas/rt_canvas_metal.rs` oracle rows for large polygons and over-cap frames

## The goal

1. **The GPU draws, not the software rasteriser.** A `Mode.Canvas` program with a GPU gets
   Metal frames for ordinary scenes. Software is the fallback for a capability
   the backend really lacks, never for "too many" or "too detailed".
2. **60 fps with a medium to large number of polygons, from small to large.** This is a
   modern GPU. `examples/wind`, unmodified, runs at 60 fps. So does a scene of **5 million
   triangles**.

**Metal only.** This bug covers the macOS Metal backend. Vulkan is out of scope: it is not
measured, not changed deliberately, and not a gate. If a shared change breaks it (for
example raising `CANVAS_MAX_FRAME_ITEMS`, which both backends read), that is accepted and
tracked separately.

Today neither holds. `examples/wind` draws **one frame in 20 seconds**. With every cap
removed it draws **8.9 fps**, and 90% of each frame is MFBASIC running on the graphics
thread; Metal takes 10 ms of the 111 ms. The measurements are below. Each gives the
command behind it; the probes are listed in [Reproducing the numbers](#reproducing-the-numbers).

## Failing Reproduction

All runs are on the macOS aarch64 dev host (Apple G14X GPU, `AGXMetalG14X` in `sample`).
They are headless 900×640 with `MFB_CANVAS_GPU=1`. *Rendered* is the line count of the
stats file, which gets one line per rendered frame.

**Sections A-E use `--debug` builds**, because `MFB_CANVAS_STATS` exists only there, and
**a debug build is about 35% slower on this path.** Measured on the same 1,000 × 8 moving
scene with `MFB_CANVAS_SYNC=1`, so each present waits for its own frame: 116 frames in 5 s
as `--debug`, 157 frames as a release build. Read the per-item graphics-thread costs below
as roughly 1.35× a release build's: ~30 µs rather than ~40 µs per item at 1,000 items. The
comparisons between rows, and every conclusion, are unaffected. Section F is a release
build.

### A. `examples/wind`: 1 frame in 20 s

`wind_rate.sh` builds the example unmodified and waits for the first map frame, which
comes after the ~30 s NOMADS fetch screen on a cold cache. It then counts frames over the
next 20 s:

| build | frames / 20 s | fps | on Metal | blocks per frame |
| --- | --- | --- | --- | --- |
| HEAD `42cea38df` | **1** | 0.05 | no (software) | 5,663 |
| spike C (all caps raised) | 178 | **8.9** | yes (`gpuFrames` = frames) | 5,741 |

- Expected: 60 fps on Metal.
- Observed: keys are read and the view state changes, but the repaint takes tens of
  seconds, so the controls look dead.

### B. The cliff: scenes past a cap are drawn in software

`run_stress.sh <mfb> <tag> ITEMS POINTS MOVING 5` runs for 5 s. ITEMS polygons of POINTS
points, radius 8, are tiled over the surface. Moving means every polygon shifts each
frame. Static means only one small rect moves, since an unchanged scene is never
re-rendered:

| items × points | edges | HEAD fps | HEAD renderer | spike C fps (caps raised, Metal) |
| --- | --- | --- | --- | --- |
| 95 × 8 | 760 | 137 | Metal | 137 |
| 1,000 × 8 | 8,000 | 23 | Metal | 24 |
| 1,000 × 8 static | 8,000 | 24 | Metal | 24 |
| 2,000 × 8 | 16,000 | 12 | Metal | 12 |
| 2,500 × 8 | 20,000 | **1.4** | software (frame edges > 16,384) | 9.4 |
| 5,000 × 3 | 15,000 | **1.6** | software (quads > 4,096) | 7.0 |
| 10,000 × 4 | 40,000 | — | software | 3.4 |
| 10 × 300 | 3,000 | **18** | software (polygon > 256 edges) | 118 (spike B) |
| 100 × 300 | 30,000 | **2** | software | 14 |
| 10 × 4,000 | 40,000 | **1.8** | software | 15 |

Provenance: the 1,000 static, 2,000 and 10 × 300 rows in the last column come from spike B
plus the phase timers. Its frame caps are HEAD's, and those scenes are under them. Every other
row in that column is spike C.

Two separate failures show up here:

- **The cliff**: the rows marked software drop 5-10× the moment a cap is crossed.
- **The slope**: rows that stay on Metal still fall from 137 fps to 12 fps between 95 and
  2,000 items. Raising the caps fixes the cliff and does nothing about the slope.

### C. Where a Metal frame's time goes

Spike C instruments `__canvas_renderFrame` with `datetime::monotonicNanos` around each
phase and reports cumulative ms on the stats line (spike diff, Appendix). Per rendered
frame:

| scene | `__canvas_sceneOffsets` | `__canvas_sceneDraws` | Metal render + readback + present | fps |
| --- | --- | --- | --- | --- |
| 1,000 × 8, moving | 20.3 ms | 19.3 ms | 3.9 ms | 23 |
| 3,000 × 3, moving | 38.7 ms | 36.4 ms | 4.7 ms | 21 |
| 10,000 × 4, moving | 149 ms | 142 ms | 9.6 ms | 3.4 |
| `examples/wind` | 51 ms | 50 ms | 10.5 ms | 8.9 |

The GPU is not the bottleneck. At 10,000 items Metal needs 9.6 ms, while the two MFBASIC
scene walks in front of it need 291 ms. That is **~29 µs per item per frame** on the
graphics thread (debug build; about 22 µs in release by the ratio above), and `generations` shows **2.0 geometry rebuilds per item per frame**:
both walks miss the 256-entry cache (`__CANVAS_GEO_CAPACITY`), including on a static scene.

### D. A bigger cache is not the fix

Spike D is spike C plus `__CANVAS_GEO_CAPACITY` raised from 256 to 65,536. The
`run_stress.sh` settings are the same, with 8 s runs:

| scene | geometry rebuilds after warm-up | walks per frame | fps |
| --- | --- | --- | --- |
| 1,000 × 8 static | ≈0 (1,550 total over 276 frames) | 12.3 + 12.5 ms | 34 |
| 10,000 × 4 static | ≈0 (10,029 total) | 280 + 276 ms | 1.9 |
| 10,000 × 4 moving | 1 per item per frame | 1,137 + 1,132 ms | 0.9 |

With a 100% cache hit rate the MFBASIC walk still costs **~12 µs per item per walk**. The
cost is the walk itself: `collections::getOr` per element, arena `List OF Float`, per-item
hashing. Regeneration is only part of it. For a moving scene the bigger cache is *slower*,
because it grows and compacts.

### E. Large polygons: Metal time grows with edges × covered pixels

In spike C, one polygon of radius 300 covers about 280k pixels:
`R=300.0 CX=450.0 CY=320.0 run_stress.sh ./mfb-C Cbig 1 N 1 5`. The Metal render phase per
frame is:

| edges | 1,000 | 4,000 | 16,000 | 64,000 |
| --- | --- | --- | --- | --- |
| render ms/frame | 9.4 | 17.9 | 65.7 | 252 |

The fragment shader's `edgeDistance` in `METAL_SHADER_SOURCE`
(`src/target/macos_aarch64/app/metal.rs`) loops over **every edge of the polygon for every
covered pixel**. One quad per polygon with an SDF fill is exact and simple. But it is
O(edges × pixels), so one detailed coastline polygon can take the whole frame budget by
itself, with the CPU pipeline fixed or not.

### F. The worker's side: `canvas::present` costs 3-4 µs per item, and nothing fixes it

The frame rate is set by the slower of two threads: the worker, which builds the list and
presents it, and the graphics thread. Fixing only the graphics thread moves the limit to
the worker. `mb` is a release-build microbenchmark on the worker thread, 10,000 items,
third of three repetitions (the first is cold):

| worker operation, 10,000 items | total | per item |
| --- | --- | --- |
| build a `List OF DrawItem` of `Rectangle`s (`collections::append`) | 7.9 ms | 0.8 µs |
| build 4-point `Polygon`s sharing one points list | 10.4 ms | 1.0 µs |
| build 40,000 `Point`s | 3.9 ms | 0.1 µs per point |
| `FOR EACH` + `MATCH` over the list | 1.0 ms | 0.1 µs |
| `canvas::present` of the rectangles | 29.0 ms | **2.9 µs** |
| `canvas::present` of the polygons | 35.7 ms | **3.6 µs** |

`canvas::present` (`__canvas_present`, `src/codegen/builtins/canvas/func_present.rs`) does
three things. `canvas::publishScene` is native and deep-copies the list. The other two are
MFBASIC walks over every item on every present: `__canvas_groupSignature`, and
`__canvas_hashScene`, which calls `__canvas_hashItem` per item. A plain `FOR EACH` +
`MATCH` over the same list costs 0.1 µs per item, so most of the 3-4 µs is the per-item
hashing and deep copy. The split between them has **not** been measured.

In the `--debug` stress probe, wall time inside `present` was 10.4 ms at 1,000 items and
72-75 ms at 10,000, moving or static. That is the same ~7 µs per item order, inflated by
the debug build and by any ring waits.

What this means for 5 million triangles: the API draws a triangle as one
`canvas::Polygon` item, so **5 million triangles is 5 million `DrawItem`s**. At ~1 µs to
build one and ~3.6 µs to present it, that is ~23 s per frame (estimate by multiplication),
or the same again as a one-time `canvas::setGroup` load. An immediate-mode
`List OF DrawItem` cannot reach the 5M-triangle goal at any renderer speed. That needs
retained geometry (Phase 5).

### G. The shader fix for large polygons: a per-band edge index, bit-identical, 36-108× faster

`band/band.swift` is a standalone Metal program built with `swiftc -O`. It runs two compute
kernels over a 900×640 grid:

- `full`: today's `edgeDistance` loop and coverage quantisation copied verbatim from
  `METAL_SHADER_SOURCE`, with the same 16.16 fixed-point edges, pixel-centre sampling,
  `clamp(0.5 - d)` for a fill and `clamp(0.5 - (|d| - half))` for a stroke, `* 255 + 0.5`.
- `banded`: the same loop body run over one band's edge list.

**The idea.** Split the polygon's y-range into horizontal bands of H pixels. Put each edge
in every band its `[ymin - r, ymax + r]` touches, where r is the distance past which an
edge cannot change coverage: 0.5 for a fill, `half + 0.5` for a stroke, plus a 1 px guard.
A pixel then loops only over its own band. The result is **exactly** what the full loop
computes:

- the even-odd crossing test only counts edges whose y-span contains `p.y`, and those are
  all in the band;
- any edge left out is more than r away, so its distance clamps to the same coverage.

The band table (`start, count` per band) and the index (edge numbers) are built on the
CPU, once per geometry, and cached with it.

Measured GPU time per frame, median of 5-15 runs, `gpuEndTime - gpuStartTime`. *Differing
px* compares every pixel of the banded output against the full loop:

| polygon | edges | full loop | band 1 px | band 2 px | band 4 px | differing px (all heights) | CPU band build (2 px) |
| --- | --- | --- | --- | --- | --- | --- | --- |
| circle r=300 | 1,000 | 2.79 ms | 0.04 (66×) | 0.05 (60×) | 0.06 (48×) | **0** | 0.04 ms |
| circle r=300 | 16,000 | 44.6 ms | 0.46 (98×) | 0.53 (84×) | 0.72 (62×) | **0** | 0.30 ms |
| circle r=300 | 64,000 | 180 ms | 1.81 (99×) | 2.05 (88×) | 2.75 (65×) | **0** | 0.99 ms |
| coastline-like (5 harmonics) | 64,000 | 170 ms | 1.57 (108×) | 1.99 (85×) | 2.77 (61×) | **0** | 1.30 ms |
| same, stroke half-width 3 | 64,000 | 173 ms | 3.64 (48×) | 3.83 (45×) | 4.71 (37×) | **0** | 1.79 ms |
| wind's largest real coastline ring | 613 | 1.13 ms | 0.03 (36×) | 0.03 (34×) | 0.04 (30×) | **0** | 0.04 ms |
| same, subdivided (same shape) | 64,365 | 100 ms | 1.43 (70×) | 1.71 (59×) | 2.36 (43×) | **0** | 1.11 ms |
| **worst case**: 4,000 spikes from the centre | 4,000 | 11.2 ms | 3.76 (3×) | 3.80 (3×) | 3.81 (3×) | **0** | 0.76 ms |
| **worst case**: 64,000 spikes | 64,000 | 174 ms | — | — | 60.4 (3×) | **0** | 7.5 ms (4 px) |
| **worst case**: self-intersecting star {4001/1999} | 4,001 | 11.1 ms | 7.49 (1.5×) | — | 7.63 (1.5×) | **0** | 0.66 ms (4 px) |

What the numbers say:

- **Correctness is free.** Every pixel of every shape is bit-identical, including strokes
  and a self-intersecting even-odd polygon. The Metal-vs-software tolerance and the
  oracle tests do not change.
- **Realistic outlines drop 36-108×.** A 64,000-edge coastline goes from 170 ms to 2 ms,
  so a detailed polygon fits in a 60 fps frame.
- **Shapes whose edges each span most rows barely improve (1.5-3×).** In the spikes and
  the dense star, every band holds most edges. A 64k-edge spiky polygon still costs
  60 ms. These are pathological for map and chart geometry, but they exist.
- **Band height.** 1 px is fastest but has the largest index: 257k entries (1 MB) for the
  64k circle, 650k for the stroke, and 3.1M (12 MB) at 4 px for the 64k spikes. 2 px is
  within ~15% of 1 px at ~60% of the index size.
- **The CPU build is cheap and one-off.** About 1-2 ms for 64k edges in native code, paid
  when the geometry changes, not per frame.

The compute kernel is not the real fragment pipeline. Its full-loop numbers (180 ms at 64k
edges) are below section E's in-renderer 252 ms, which also includes the frame's fixed
cost. The *ratios* should carry over, because the loop dominates both; that is an
estimate, and Phase 3's acceptance measures it in the real renderer.

**Rejected: triangulated fill.** Hardware triangles need MSAA or a separate edge pass for
antialiasing, and neither reproduces the software rasteriser's analytic coverage, so the
oracle would need a looser tolerance or a changed rasteriser. Triangulating
self-intersecting even-odd polygons also needs a robust tessellator written natively.
The band index keeps today's shader maths and is bit-identical.

**Not spiked: the worst case.** A 2-D cell grid would help the spike and star shapes: each
cell stores the parity at one corner plus the edges touching the cell, and a pixel
combines them. It changes *which* crossings are counted, so bit-identity with the full loop
would have to be proven again, especially for vertices exactly on cell boundaries. Only
worth doing if a real program needs those shapes (Open Decisions).

### H. Pictures and text: a per-frame texel limit that tilemaps, backgrounds and text screens exceed

A `Picture` item's pixels, and every glyph of a `Text` run, go through the frame buffer's
glyph region, which holds `METAL_MAX_FRAME_GLYPH_SAMPLES` = 1,048,576 words.

- `__canvas_metalRenderable` adds `width × height` **for every `Picture` item**
  (`__canvas_pictureSamples`) and `w × h` for every glyph occurrence
  (`__canvas_runSamples`). It declines the whole frame past the limit.
- `emit_picture_buffer` (`metal.rs`) copies the image's texels into the region **for
  every item**, word by word, every frame. Nothing is reused, even when a thousand items
  name the same image.

Probe: `run_pics.sh <mfb> <tag> TILES TILE_PX BG_W BG_H`. It draws TILES pictures of two
alternating 32×32 images, shifting every frame, plus an optional background picture
scaled to the surface. It does a 5 s moving run, then a one-frame software vs Metal
compare. At HEAD:

| scene | texels counted | fps | renderer |
| --- | --- | --- | --- |
| 500 tiles (2 distinct images) | 512,000 | 78 | Metal |
| 1,024 tiles | 1,048,576 | 43 | Metal |
| **1,025 tiles** | 1,049,600 | **~5** | **software** |
| 2,000 tiles | 2,048,000 | 2.6 | software |
| 1024×1024 background + 1 tile | 1,049,600 | 11 | software |

So an ordinary tilemap of more than 1,024 32-pixel tiles, or a background image of about
1 megapixel plus anything else, sends the frame to software. The 1,025th tile costs 90% of
the frame rate.

Text hits the frame caps too (`run_text.sh <mfb> <tag> LINES COLS SIZE`: LINES `Text`
items of COLS characters in Arial, plus one moving rect). Each glyph is its own quad:

| scene | HEAD | spike C (frame caps raised) | spike F (caps + texel fix) |
| --- | --- | --- | --- |
| 80 × 20 characters, 16 px | 257 fps, Metal | — | 257 fps, Metal |
| 120 × 36 characters, 16 px (4,320 glyphs) | **8 fps, software** (quads > 4,096) | 208 fps, Metal | 202 fps, Metal |
| 200 × 60 characters, 12 px (12,000 glyphs) | software (quads > 4,096; inferred, not run) | **6 fps, software** (texels > 1M) | 181 fps, Metal |

A terminal-sized screen of text falls back on the quad cap. Past that, it falls back on
the texel cap.

**The fix, spiked (`spike-F.diff`):**

1. **Upload each distinct image once per frame.** A 256-entry `(block address, base)`
   table sits in a new fifth region of the frame buffer
   (`METAL_PICTURE_TABLE_OFFSET`), with a per-frame count in the draw frame's free slot
   at `sp + 624` (`OFF_PIC_COUNT`), reset with the other cursors. `emit_picture_buffer`
   looks the block up first. On a hit it points the item at the existing texels and
   skips the copy; on a miss it copies and records. The predicate counts each distinct
   image once, keyed on the header's shadow-address slots (35/36).
2. **Raise the texel limit to 8M words** (`1 << 23`, 32 MiB), so a 1080p or 1440p
   background fits.

Measured, with every output bit-identical to software (`cmp.py` gives 0 differing pixels
for pictures, and the same ≤Δ1 antialiasing noise as HEAD for text):

| scene | HEAD | E1 (dedupe) | E2 (dedupe + 8M) | F (E2 + spike C's frame caps) |
| --- | --- | --- | --- | --- |
| 1,025 tiles | ~5 fps, software | 45 fps, Metal | — | 44 fps, Metal |
| 2,000 tiles | 2.6 fps, software | 25 fps, Metal | — | — |
| 4,000 tiles | software | 14 fps, Metal | — | — |
| 1024×1024 background + 1 tile | 11 fps, software | 11 fps, software | 229 fps, Metal | — |
| 1920×1080 background + 1 tile | software | — | 207 fps, Metal | — |
| 4,000 tiles + 1920×1080 background | software | — | — | 13 fps, Metal |

What remains after the fix:

- **Tile counts still hit the per-item pipeline.** 2,000 tiles at 25 fps is root cause (4)
  (MFBASIC per item), not pictures: 1,000 polygons run at the same rate. Phases 1-2 fix it.
- **Per-frame copying of a large image costs about 1.8 ms per 2 Mpx.** 1 tile alone runs at
  328 fps, and 207 fps with a 1080p background, so the background adds 4.83 − 3.05 ms.
  Three full-screen parallax layers would cost ~5 ms a frame (estimate). Keeping images
  resident on the GPU, uploaded on `createImage`/`setBytes` rather than every frame,
  removes that cost. Not spiked; see Open Decisions.
- **Glyphs are still copied per occurrence.** With the 8M limit that is 200 × 60 text at
  181 fps, so it is not a limit today. Deduping by glyph cache entry is the same change
  as for pictures, if text-heavy screens ever need it.
- **The layout test gains a region.** `the_metal_shader_region_bases_match_the_buffer_layout`
  asserts the buffer is "exactly its four regions". Spike F fails exactly that assertion
  (51,466,240 vs 51,462,144 bytes, the 4 KiB table) and nothing else. The real fix
  extends the chain by one link.

**Gradient stops** (`MAX_FRAME_GRADIENT_STOPS` = 4,096 per frame) are the last per-frame
cap in `__canvas_metalRenderable`. 2,049 two-stop gradient items would fall back the same
way. That is from reading the code, not measured. It is raised with the others in
Phase 1's first step.

## Root Cause

**(1) The per-polygon cap exists twice, and the second copy is in the native emitter.**
`__canvas_metalRenderable` (`src/codegen/builtins/canvas/helper_render.rs`) declines a
frame containing any polygon with more than `__CANVAS_METAL_MAX_EDGES` (256) edges. The
Metal emitter `emit_edge_buffer` (`src/target/macos_aarch64/app/metal.rs`) checks the
same limit again against the Rust `MAX_EDGES` (`src/codegen/runtime/canvas/mod.rs`,
256). For a polygon over the limit it writes an edge count of **0**, so the polygon draws
nothing:

```
asm.push(abi::compare_immediate(count, &MAX_EDGES.to_string()));
asm.push(abi::branch_gt(&empty));   // "draw no edges and leave the base at zero"
```

Nothing else holds the 256 limit. The `setFragmentBytes:` 4 KB limit the old comments
cite has been gone since plan-116-A: the edges already come from the frame buffer at
`METAL_EDGE_BASE`. Measured with `run_poly.sh`, one wavy-ring polygon, software vs Metal
frame:

| build | N=256 | N=300 | N=1,000 | N=4,000 |
| --- | --- | --- | --- | --- |
| HEAD | 62 px differ, max Δ2 | declined, 0 px (both software) | — | — |
| spike A (MFBASIC cap lifted only) | 62 px, Δ2 | **127,822 px, Δ255: polygon absent** | same | same |
| spike B (both copies lifted) | 62 px, Δ2 | 48 px, Δ1 | 65 px, Δ1 | 62 px, Δ1 |

Spike B's differences at 4,000 edges are the same antialiasing noise as HEAD's 256-edge
row. **The cap is obsolete, and removing both copies is the whole fix for it.**
(Correction to the earlier version of this report: its "363 bytes at N=300" table could
not be reproduced. At HEAD N=300 is declined and differs by 0. With only the MFBASIC cap
lifted it differs by 127,822 pixels.)

**(2) Frame caps that ordinary scenes exceed.** `CANVAS_MAX_FRAME_ITEMS` = 4,096 quads and
`METAL_MAX_FRAME_EDGES` = 16,384 edges decline the whole frame, in `mod.rs` and in
MFBASIC as `__CANVAS_MAX_FRAME_ITEMS` / `__CANVAS_METAL_MAX_FRAME_EDGES`. `examples/wind`
publishes about 5,700 blocks a frame, so **only the per-polygon cap is not enough for
wind**: shrinking every coastline ring below 256 points would still leave it in software.
The earlier claim that doing so gave `gpuFrames=114` is contradicted by the block count.
Spike B, which lifts only the per-polygon cap, still gave `gpuFrames=5` of 8 frames in a
40 s wind run.

The MFBASIC and Rust copies cannot drift silently:
`the_two_gpu_edge_budgets_match_the_emitters`
(`src/codegen/builtins/canvas/helper_render/tests/mod.rs`) asserts they are equal. They
are still two definitions, joined by a test rather than by construction.

**(3) Raising the frame caps moves buffer regions whose shader bases are literals.**
`METAL_EDGE_BASE`, `METAL_GRADIENT_BASE` and `METAL_GLYPH_BASE` are hand-written numbers
inside the `concat!` shader source. Spike C recomputed them for 65,536 items and 262,144
edges (3,407,872 / 4,456,448 / 4,476,928). With those values
`the_metal_shader_region_bases_match_the_buffer_layout` passes, and the oracle agrees at
every size tried (`run_oracle.sh ./mfb-C C` gives max Δ1 at 2,000×8, 5,000×3, 10,000×4,
100×300 and 10×4,000, the same noise as HEAD's accepted 2,000×8). The buffer becomes
22,102,016 bytes (~21 MiB), up from 5,193,728 bytes (~5 MiB) at HEAD.

**(4) The graphics thread's CPU pipeline is MFBASIC, and it caps a Metal frame at roughly
400-550 items at 60 fps.** That is an estimate: the 16.7 ms budget divided by the
1,000-item row in C, ~40 µs per item in the debug build and ~30 µs in release. `__canvas_renderLoop`, `__canvas_sceneOffsets`,
`__canvas_sceneDraws`, `__canvas_geometryFor` and the batching are `RegistryHelper`
MFBASIC compiled onto the graphics thread over arena `List OF Float`. Measured in C and D:

- Both walks rebuild every item's geometry: 2.0 per item per frame. The comment in
  `__canvas_renderFrame` says the second walk "hits the cache"; it does not once a scene
  has more than 256 items, which is every scene in this report.
- `sceneDraws` costs as much as `sceneOffsets` (19 vs 20 ms, and 50 vs 51 ms on wind). The
  earlier "168 ms vs 37 ms" split was not reproduced.
- With perfect cache hits (D) the walks still cost ~12 µs per item each.

**(5) The polygon fill is O(edges × covered pixels) on the GPU** (section E). `edgeDistance`
loops over every edge of the polygon for every covered pixel, but only edges within r of
the pixel, or crossing its row, can affect its coverage. A 64k-edge polygon takes 252 ms of
GPU time by itself. Section G shows the per-band edge index fixes this for realistic
shapes and gives bit-identical pixels.

**(6) `canvas::present` is per-item MFBASIC on the worker** (section F). Hashing and the
group signature walk every item on every present, at 3-4 µs per item in a release build.
Once the graphics thread is native, this is the frame-rate limit for any scene rebuilt
each frame. By itself it caps a moving scene at ~4,000 items at 60 fps (estimate: 16.7 ms
÷ ~4.5 µs of build + present per polygon), before the program's own work.

**(7) Pictures and glyphs are counted and copied once per occurrence, under a 1M-texel
frame limit** (section H). `__canvas_pictureSamples` and `emit_picture_buffer` treat every
`Picture` item as a new image, and the glyph region is sized for 1,048,576 texels. More than
1,024 32-pixel tiles, or one ~1 Mpx background plus anything else, sends the frame to
software.

**(8) The API has no way to draw geometry without re-sending it every frame** (section F).
`canvas::setGroup` stores items, but `__canvas_sceneOffsets` expands every group into
per-item work on every frame, so a group is a naming convenience, not retained GPU
geometry. There is no mesh or triangle-list primitive. Every triangle is a `Polygon`
`DrawItem` that is built, hashed, published, walked, cached and batched per frame.

**(9) Found while fixing (4): a `presentLayers` scene draws nothing on Metal.**
`__canvas_sceneDraws`, which builds the block list the Metal emitter draws, walked
`canvas::installedItems()` only. A scene installed with `canvas::presentLayers` keeps
every item in its layers, so it published no blocks at all: Metal drew an empty frame and
reported `gpuFrames=1`. Measured on a two-layer scene: 0 lit pixels on Metal against
software's 40,332 (`a_layered_scene_draws_its_layers_on_metal`). Fixed with (4), because
the draw list is now laid out from the offsets `__canvas_sceneOffsets` resolved over
items and layers alike.

**(10) Found while measuring Phase 4: the Metal frame's fixed cost grows with the surface,
about 10 ms per megapixel.** A one-rectangle scene on a 2880×1800 surface (a spike build
with `DEFAULT_SURFACE_WIDTH/HEIGHT` raised) renders at 20 fps, against ~300 fps at
900×640. Timed inside `__canvas_renderMetal` (`--debug`, per frame):
`canvas::newSurface` allocating and filling 20 MB, 3.3 ms; `canvas::metalDrawScene`
(render, `getBytes` readback, per-pixel BGRA→RGBA swizzle), 5.7 ms; `__CANVAS_KEPT =
buffer`, a whole-frame copy, 6.1 ms; the headless present, 0.3 ms. A real window adds
the CPU blit to the layer on top. The `__CANVAS_KEPT` copy exists only for damage mode
and is now skipped otherwise; the rest is the readback architecture Phase 4 replaces.

## Goal

- **(G1)** No ordinary scene falls back to software. The frame caps admit tens of thousands
  of quads. The shader region bases derive from the layout constants rather than being
  literals, and the MFBASIC caps come from the Rust constants. The per-polygon edge cap is
  gone (Phase 3), so a polygon of any size draws on Metal within `Tolerance::GPU_DEFAULT`
  of software.
- **(G1b)** A tilemap of thousands of tiles drawn from a handful of images, a full-screen
  background of up to 2560×1440, and a terminal-sized screen of text all draw on Metal
  (section H).
- **(G2)** `canvas::present` costs ≤0.5 µs per item on the worker (today 2.9-3.6 µs).
- **(G3)** `examples/wind`, **unmodified**, renders every frame on Metal
  (`gpuFrames = frames`) at **60 fps** at its default window size.
- **(G4)** A scene of **5,000,000 triangles** renders at **60 fps** on Metal when static and
  while panning or zooming through a transform.
- **(G5)** Medium-to-large scenes of polygons from 3 to 64k edges hold 60 fps. The
  `examples/gpu` README names the exact sweep rows before Phase 1 starts, and every one of
  them must pass.

### Non-goals (must NOT change)

- **The honesty gate.** Declining a scene the backend truly cannot draw is correct. A
  polygon is never clamped or truncated to fit.
- **Software stays the oracle.** Every GPU change is accepted on "Metal agrees with software
  within `Tolerance::GPU_DEFAULT`" (`examples/gpu --compare`, `rt_canvas_metal.rs`), not on
  a golden file's byte-identity. Goldens are the regression net for reference scenes. A
  golden that moves by a low bit is investigated, not treated as failure by itself.
- **Do not simplify the input.** Regenerating `examples/wind`'s coastline under a cap was
  already rejected. The example is the acceptance scene exactly as it is.
- The software rasteriser does **not** need to become fast or native for this goal. It is
  the fallback and the oracle; it is not on the 60 fps path.

## Blast Radius

- `helper_render.rs`: `__canvas_metalRenderable` and the `LET __CANVAS_*MAX*` caps.
- `runtime/canvas/mod.rs`: `MAX_EDGES`, `CANVAS_MAX_FRAME_ITEMS`, `METAL_MAX_FRAME_EDGES`,
  `METAL_BUFFER_BYTES`, `METAL_*_BASE_WORDS`.
- `metal.rs`: `emit_edge_buffer` (the second per-polygon cap, and where the band table and
  index are written), `edgeDistance` in `METAL_SHADER_SOURCE`, and the shader-base literals.
- `metal.rs` `emit_picture_buffer`, the draw frame's slot map (`OFF_*`, `DRAW_FRAME`), and
  `helper_render.rs` `__canvas_pictureSamples`: the per-frame picture table (section H).
  `the_metal_shader_region_bases_match_the_buffer_layout` gains a fifth region.
- **Vulkan, knowingly:** `CANVAS_MAX_FRAME_ITEMS` is shared, so the Vulkan backend
  (`vulkan.rs`, its checked-in SPIR-V, `__canvas_vulkanRenderable`) may break. Out of scope
  (The goal).
- Tests that pin today's declines and must be updated **with proof**, per the
  never-edit-a-test rule: `an_unsupported_scene_falls_back_to_the_software_renderer` (its
  premise, "300 edges do not fit `setFragmentBytes:`", is disproved by spike B),
  `a_frame_whose_polygons_together_overflow_the_edge_region_falls_back` (its 40,000-edge
  scene becomes drawable; the decline row moves to a scene past the new budget), and
  `the_two_gpu_edge_budgets_match_the_emitters` (its per-item assertion goes with the cap).
- `__canvas_sceneOffsets`, `__canvas_sceneDraws`, `__canvas_geometryFor`, the five
  `__CANVAS_GEO_*` globals, `emit_graphics_trampoline`: the graphics-thread pipeline.
- `__canvas_present`, `__canvas_hashScene`, `__canvas_hashItem`,
  `__canvas_groupSignature`, `canvas::publishScene`: the worker-side present. The damage
  diff pairs these hashes with offsets by index (`__canvas_damageFor`), so a native hash
  must produce the same values in the same order.
- The canvas public API, if Phase 5 adds a retained-geometry primitive, which means
  `mfb man canvas` and `mfb spec app canvas` too.
- `planning/plan-150-*` `canvas::ParticleSystem`: it expands to ordinary draw entries, so it
  inherits every per-item cost here.
- `.ai/canvas-threading.md` and the plan-116-A notes, which describe the 256 cap as
  transport-forced.

## Fix Design

There are five independent ceilings, and each has to be removed for the goal to hold:

| ceiling | measured limit today | removed by |
| --- | --- | --- |
| caps → software | 4,096 quads, 16,384 edges per frame; 256 edges per polygon | Phase 1 step one (frame caps; proven in spike C), Phase 3 (per-polygon cap) |
| graphics-thread CPU | ~22-30 µs/item/frame (release/debug) | Phase 1 (native walk, cache, batching) |
| worker `present` | 2.9-3.6 µs/item/frame (release) | Phase 2 (native hashing) |
| GPU polygon fill | O(edges × pixels): 64k edges = 252 ms | Phase 3 (per-band edge index; section G) |
| the API itself | ~4.5 µs/item built + presented, per frame | Phase 5 (retained geometry) |

**Why there is no standalone "remove the caps" phase.** An earlier draft had two cheap
phases ahead of the rewrite: delete the 256-edge per-polygon cap, and raise the frame
caps. Neither stands alone once the rewrite is committed to.

- The **per-polygon cap** is deleted in Phase 3, together with the band index. Deleting it
  alone is correct (spike B), but it moves a large polygon from software to a Metal loop
  that costs edges × pixels (section E: 252 ms at 64k edges). The band index is what makes
  lifting it pay off. If wind's coastline is wanted on Metal before the rewrite lands,
  deleting both copies of the cap is a safe, separate few-line change.
- The **frame caps and shader bases** are not thrown away. The native pipeline writes
  the same frame buffer with the same regions, and its acceptance scenes (10,000 items,
  wind's ~5,700 blocks) are over today's caps. The spike had to raise them just to take
  section C's measurements. So they are Phase 1's first step, a prerequisite for
  measuring the rest of it.

**Phase 1, the native graphics-thread pipeline.** A `List` is already a flat block: a
header plus a data region at `header + capacity * ENTRY` with a fixed stride. The graphics
thread can walk the published `List OF DrawItem`, and a `Polygon`'s nested points list, in
native code through `emit_collection_data_pointer`, based on **capacity, never count**
(`.ai/collections.md`). No second representation or serialisation is needed. On top of
that:

- a native open-addressed `hash → offset` geometry cache over a process-global `Vec<f32>`,
  replacing the five `__CANVAS_GEO_*` globals and the 256-entry capacity;
- native `headerFor`/`tailFor`;
- native run-building;
- one walk per frame instead of two. An item whose hash did not change keeps its geometry
  and instance slot and is not re-uploaded.

**Phase 2, native present.** `__canvas_hashItem` and `__canvas_groupSignature` become
native walks over the same flat layout as Phase 1, producing bit-identical hashes, because
the damage diff and the geometry cache key on them. Better still, compute each item's hash
once, during `publishScene`'s deep copy, which already touches every byte. Phase 1's cache
can then key on the published hash instead of rehashing on the graphics thread.

**Phase 3, a per-band edge index for polygons** (section G). When a polygon's geometry is
generated and cached (native, from Phase 1), also build its band table and edge index:
2 px bands, each edge placed in every band its `[ymin - r, ymax + r]` meets, r = 0.5 for a
fill and `half + 0.5` for a stroke, plus a 1 px guard. For a transformed item r is in
shape space, so it is scaled by the transform's largest singular value, and the band lookup
uses the inverse-mapped point. `emit_edge_buffer` writes the table and index into the frame
buffer's edge region beside the edges. The item block carries the table's base, band top,
band height and band count. `edgeDistance` loops over one band instead of every edge,
with the body unchanged. The index is budgeted inside the edge region: when a polygon's
index would exceed a multiple of its edge count (the spike shape), use a taller band for
that polygon rather than declining. The per-polygon cap (`__CANVAS_METAL_MAX_EDGES`,
`MAX_EDGES` and its branch in `emit_edge_buffer`) is deleted, because nothing depends on it.

**Phase 5, retained geometry.** A canvas value whose vertex data is uploaded once into a
GPU buffer and drawn by reference, with a transform per frame. It could be a new
`DrawItem` variant or a `RES` handle, which is decided in the phase. A static or
camera-panned scene then costs O(1) CPU per frame, however many triangles it has. That is
what the 5M-triangle goal needs.

### What each phase buys: expected 60 fps capacity

These are **estimates**. Each one divides the 16.7 ms frame budget by the slowest stage's
measured or targeted per-item cost. The worker and the graphics thread run in parallel,
so the slower one sets the rate. Metal render + readback is 3.3 ms fixed + ~0.6 µs per item
(section C: 95 → 10,000 items). How much of that is CPU encoding has not been measured.

| after | slowest stage per moving item | moving items at 60 fps | `examples/wind` (~5,700 items) |
| --- | --- | --- | --- |
| HEAD | graphics thread, ~22-30 µs + caps | ~500 (on Metal), cliff past caps | 0.05 fps (software) |
| Phase 1 | worker: ~1 µs build + ~3.6 µs present | **~3,500** | ~35-40 fps at best, before its own simulation |
| Phase 2 | Metal, ~0.6 µs + 3.3 ms; worker ~1.5 µs | **~10,000** | 60 fps if its simulation fits |
| Phase 3 | same, plus realistic polygons of any edge count (a 64k-edge outline costs ~2 ms of GPU) | ~10,000 | 60 fps with the full-detail coastline |
| Phase 5 | only changed items cost per frame | 5,000,000 retained triangles; ~10,000 changing items per frame | coastline as retained geometry |

Past ~10,000 moving items, the per-item Metal submission and the program's own
MFBASIC build (~1 µs per item) are the next limits. Both are outside this bug.

## Phases

### Phase 0: the instrument (no behavior change)

- [ ] Promote the spike probes to `examples/gpu`: the items × points × moving/static sweep,
      the single-polygon edge sweep, the worker-side microbenchmark, a `--compare` oracle
      mode, and a README naming the 60 fps rows G5 is judged on.
- [ ] Add the phase timers (`sceneOffsets`, `sceneDraws`, render) to the `--debug`-only
      `MFB_CANVAS_STATS` line, as the spike did. Report worker `present` time as well.
      State in the README that `--debug` inflates this path by ~35%, so the fps rows are
      judged on a release build with `MFB_CANVAS_SYNC=1`.

Acceptance: one command reproduces sections B-F of this report on HEAD.
Commit: —

### Phase 1: the graphics-thread pipeline goes native

- [ ] **First step: nothing ordinary falls back.** Derive `METAL_*_BASE` from the layout
      constants by substituting into the shader source at build time. Hand the MFBASIC
      predicates the Rust constants instead of their own `LET` copies. Raise
      `CANVAS_MAX_FRAME_ITEMS` to 65,536 and the Metal frame-edge budget to 262,144, as
      sized in spike C. Raise `METAL_MAX_FRAME_GLYPH_SAMPLES` to `1 << 23` and
      `MAX_FRAME_GRADIENT_STOPS` to match the item cap. Update
      `a_frame_whose_polygons_together_overflow_the_edge_region_falls_back` with spike C as
      proof: its 40,000-edge scene becomes drawable, and the decline row moves past the
      new budget.
- [ ] **Pictures upload once per distinct image per frame** (section H, `spike-F.diff`): the
      per-frame `(block, base)` table in a fifth buffer region, the lookup in
      `emit_picture_buffer`, and the predicate counting distinct images. Add a
      `rt_canvas_metal.rs` oracle row with 2,000 tiles from two images and one with a
      1920×1080 background, both drawn by Metal. Extend the region-chain assertion by one
      link.
- [ ] Native walk of the published scene (capacity-based data pointer).
- [ ] Native geometry cache and generation; one walk per frame.
- [ ] Native batching; unchanged items keep geometry and instance slot.

Acceptance:
- After the first two steps: `examples/wind` unmodified gives `gpuFrames = frames`
  (spike C: 178/178). No row of section B or section H falls to software (spike F: all
  on Metal, pictures bit-identical to software).
- At the end: graphics-thread CPU per item falls from ~22 µs (release) to ≤0.3 µs at
  10,000 items. `generations` is 0 after warm-up on a static scene and proportional to
  changed items on a moving one. `--compare` is clean.

Commit: —

### Phase 2: native present

- [ ] `__canvas_hashItem` / `__canvas_hashScene` / `__canvas_groupSignature` become native,
      bit-identical to today's hashes. A unit test pins hash equality over every
      `DrawItem` variant.
- [ ] If measurement supports it, hash during `publishScene`'s deep copy, and let the
      graphics-thread cache key on the published hash.

Acceptance: `canvas::present` costs ≤0.5 µs per item in the `mb` microbenchmark (today 2.9
rects / 3.6 polygons). Damage and partial-redraw tests pass unchanged
(`tests/canvas/rt_canvas_damage.rs`). `examples/wind` reaches 60 fps, or the report names
the stage that stops it, measured.
Commit: —

### Phase 3: a per-band edge index for polygons, and the per-polygon cap goes

- [ ] Build the band table and edge index natively with the polygon's cached geometry
      (section G's algorithm, 2 px bands, r = 0.5 fill / `half + 0.5` stroke plus a 1 px
      guard, scaled for transforms). Taller bands when the index would blow its budget.
- [ ] `emit_edge_buffer` writes the table and index into the edge region. The item block
      carries base, band top, band height and band count. `edgeDistance` loops over one
      band. Derive any new region base from the layout constants, as in Phase 1.
- [ ] Delete `__CANVAS_METAL_MAX_EDGES`, `MAX_EDGES` and its branch in `emit_edge_buffer`.
      Update `an_unsupported_scene_falls_back_to_the_software_renderer` (its premise,
      "300 edges do not fit `setFragmentBytes:`", is disproved by spike B) and the
      per-item assertion in `the_two_gpu_edge_budgets_match_the_emitters`.
- [ ] Update `.ai/canvas-threading.md` and the stale "4 KB `setFragmentBytes:`" comments.

Acceptance:
- `rt_canvas_metal.rs` gains oracle rows at 300, 1,000, 4,000 and 64,000 edges, plus a
  stroked polygon, a transformed one and a self-intersecting one. All are drawn by Metal
  and meet the same tolerance as today's accepted polygons. Section G predicts the same
  pixels as the full loop.
- In the real renderer, section E's 64,000-edge polygon renders in ≤16 ms, down from
  252 ms. Section G's kernel measured 2 ms.
- The worst-case spike polygon still draws correctly on Metal, even though it stays slow.

Commit: —

### Phase 4: present without readback (measure first)

- [ ] Measure render + readback + present at a full-screen Retina surface. At 900×640 it is
      3.3 ms with 95 items (C), and the large-surface cost has **not** been measured. If it
      threatens the 16.7 ms budget, present the Metal texture through a `CAMetalLayer`
      drawable instead of reading it back into a CPU surface. The oracle tests keep a
      readback path.

Acceptance: a measured number at full-screen Retina, and 60 fps held there.
Commit: —

### Phase 5: retained geometry

- [ ] A retained-geometry primitive (mesh or triangle list) uploaded once, drawn by
      reference with a per-frame transform, and updatable in place. It is built from a flat
      list of coordinates, not from one `DrawItem` per triangle; section F puts the
      per-item route at ~23 s for 5M triangles.
- [ ] `mfb man canvas` / `mfb spec app canvas` pages for it.

Acceptance: `examples/gpu` draws 5,000,000 triangles at 60 fps, static and while panning.
`--compare` agrees with a software render of the same mesh.
Commit: —

## Validation Plan

- Correctness: `examples/gpu --compare` and `rt_canvas_metal.rs` (Metal vs software within
  `Tolerance::GPU_DEFAULT`) on every phase; `cargo test --test 'rt_canvas_*'`.
- Performance: the `examples/gpu` 60 fps rows and `examples/wind` unmodified, judged on
  release builds (a `--debug` build is ~35% slower on this path) and attributed with the
  Phase 0 timers, with frames counted only after the first map frame (wind's cold-cache
  fetch screen lasts ~30 s).
- Doc sync: `.ai/canvas-threading.md`, `mfb spec app canvas`, and for Phase 5 `mfb man canvas`.

## Open Decisions

- **Worst-case polygons** (many edges that each span most rows: section G's spikes and
  dense star). The band index gives them only 1.5-3×. Recommended: accept that until a real
  program needs such shapes. Then spike a 2-D cell grid with corner parity, and prove it
  bit-identical again, with vertices on cell boundaries as the test cases.
- **GPU-resident images.** Copying a large image into the frame buffer every frame costs
  ~1.8 ms per 2 Mpx (section H). Recommended: accept it for now, since the dedupe spike
  covers every measured scene. Add resident images, uploaded on
  `createImage`/`setBytes`, only when a real program draws several full-screen layers.
- **Band height**: fixed 2 px, or chosen per polygon from its edge density. Recommended:
  2 px, taller only when the index budget forces it (section G's index sizes).
- **Buffer sizing** after Phase 1's first step: fixed ~21 MiB (spike C) or sized from the scene.
  Recommended: fixed. A resize path would sit in the hottest code in the renderer.

## Reproducing the numbers

The probes are in `/tmp/bug686-spike/`. They are one-off for this bug and deliberately
**not** in `scripts/`; Phase 0 promotes them to `examples/gpu`.

- `run_poly.sh <mfb> N <tag>`: one N-point wavy ring plus a box, rendered in software and
  on Metal (`MFB_CANVAS_SYNC=1`, `MFB_CANVAS_DUMP`), with differing pixels and max channel
  delta.
- `run_stress.sh <mfb> <tag> ITEMS POINTS MOVING SECS`, with optional `R=`/`CX=`/`CY=`
  environment variables for one big polygon: frames rendered, geometry rebuilds, phase
  timers, and worker build and present time.
- `run_oracle.sh <mfb> <tag> ITEMS POINTS`: one frame of the stress scene, software vs
  Metal.
- `wind_rate.sh <mfb> <tag>`: `examples/wind` frames in a 20 s window after its first map
  frame.
- `run_pics.sh <mfb> <tag> TILES TILE_PX BG_W BG_H` and `run_text.sh <mfb> <tag> LINES COLS
  SIZE`: section H's picture and text probes (5 s moving run plus a one-frame software vs
  Metal compare).
- `band/band.swift`: section G's shader spike. Build it with
  `swiftc -O band.swift -o band`, and run `./band circle|wiggly|coast|worst|all`.
- `mb/`: the release-build worker microbenchmark from section F (build, walk and
  `canvas::present` of 10,000 items, three repetitions). Build it with `mfb build -app` and
  run it with `MFB_MACAPP_HEADLESS=1 MFB_CANVAS_GPU=0`.

Spike compilers are built from HEAD `42cea38df` with `/tmp/bug686-spike/spike-C-D.diff`
applied in stages:

- **A**: `__CANVAS_METAL_MAX_EDGES` = 16,384.
- **B**: A plus `MAX_EDGES` = 16,384.
- **C**: B plus `CANVAS_MAX_FRAME_ITEMS` = 65,536, the frame and per-item edge caps at
  262,144, the three recomputed `METAL_*_BASE` literals, `datetime` imported into the
  canvas helpers, and the four phase timers on the stats line.
- **D**: C plus `__CANVAS_GEO_CAPACITY` = 65,536.
- **E1**: HEAD plus the per-frame picture table in `emit_picture_buffer` and the
  distinct-image count in `__canvas_metalRenderable`.
- **E2**: E1 plus `METAL_MAX_FRAME_GLYPH_SAMPLES` = `1 << 23`.
- **F**: E2 plus spike C's frame caps and shader bases, without C's timers
  (`/tmp/bug686-spike/spike-F.diff`).

## Summary

The canvas fails the goal in six places, and the rewrite has to remove all of them.

1. **Caps.** The 4,096-quad and 16,384-edge frame caps and the 256-edge per-polygon cap
   send ordinary scenes, wind included, to software at 1-2 fps. The frame caps are raised
   as Phase 1's first step (proven in spike C). The per-polygon cap goes with the edge
   path in Phase 3 (proven removable in spike B).
2. **Graphics-thread CPU.** Even on Metal, wind gets 8.9 fps. The graphics thread spends
   ~22-30 µs per item per frame in MFBASIC walks that rebuild every item twice, while Metal
   needs 10 ms. A bigger cache alone does not help, because the walk itself costs ~12 µs
   per item. Phase 1.
3. **Worker `present`.** 2.9-3.6 µs per item of MFBASIC hashing, on every present. With the
   graphics thread fixed, this caps a moving scene at ~3,500 items. Phase 2 brings the
   estimate to ~10,000.
4. **GPU polygon fill.** It scales as edges × pixels, so one detailed polygon blows the
   budget. A per-band edge index is bit-identical and 36-108× faster on realistic shapes
   (section G). Phase 3.
5. **Pictures and text.** Per-occurrence texel counting and copying under a 1M limit send
   tilemaps past 1,024 tiles, backgrounds of ~1 Mpx and dense text screens to software.
   A per-frame dedupe table plus an 8M limit puts them all on Metal, bit-identical
   (section H). Phase 1's first steps.
6. **API.** One `DrawItem` per triangle cannot reach 5M triangles at any renderer speed.
   Phase 5.

The first version of this report had four errors, corrected here:

- It left root cause (1) "unresolved", though the emitter check answers it.
- Its 363-byte divergence table does not reproduce.
- Its claim that under-256 rings put wind on Metal conflicts with wind's ~5,700 blocks
  against a 4,096 cap.
- Its 168 / 37 ms phase split does not match the measured ~equal split.

Vulkan is out of scope; this bug is Metal only.

The second version had two gaps, also corrected:

- It had no phase for the worker-side `present`.
- Its graphics-thread per-item costs came from `--debug` builds without saying they are
  ~35% slow.
