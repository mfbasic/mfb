# bug-765: Canvas renders 100,000 color-changing dots at 3 PPS / 4 FPS instead of 30

Last updated: 2026-10-12
Effort: huge (>3d)
Severity: HIGH
Class: Other (Performance)

Status: Open (deliverable 1 landed 2026-10-12; the 30/30 goal is not met)
Regression Test: `tools/canvas-bench/bench.sh <mfb> dots 100000` (exits 1 today: under 30/30); the cap itself by `cargo test --test rt_canvas_metal -- a_frame_past a_frame_whose a_group_drawn_again` and the `items*` / `pass*` rows of `scripts/test-canvas-gpu-rows.sh`

## Cause (coordinator, 2026-10-12, bug-staging-23)

- OWNER RULING 2026-10-12 (plan-175-A's open decision 4): the 30 PPS / 30 FPS goal stays for Metal. Box 206's goal is NOT 30: it is set from what the Intel draw cost can reach, once that is measured (about 1.4 us a circle today, so about 140 ms for 100,000).
- Two causes, both measured below. This bug now has TWO deliverables, by the owner's rulings of 2026-10-12.
- (1) THE CAP, fixed here. Rule: the honesty gate declines "a scene the backend truly cannot draw", but `helper_render.rs:__canvas_metalRenderable` / `__canvas_vulkanRenderable` decline a scene for being larger than ONE frame buffer (`METAL_MAX_FRAME_ITEMS`/`VULKAN_MAX_FRAME_ITEMS` = 65,536, and the edge, gradient-stop, dash-word and texel regions beside it), which the backend can draw in more than one pass. Place: `__canvas_renderMetal` / `__canvas_renderVulkan` hand the whole draw list to one `metalPresentScene`/`metalDrawScene` (`vulkanPresentScene`/`vulkanDrawScene`) call that fills the buffer once. Fix: a frame is drawn as consecutive runs of the draw list, each run the longest that fits EVERY region, in order, into the same target (the first pass clears, later ones load), each pass finished before the single-buffered frame buffer is refilled, presented or read back once after the last. The constants stay as the size of one pass; no scene size is a limit. Only an item that cannot fit a region BY ITSELF is still declined. No pipeline rebuild (the region bases are compiled into the shader; bug-996).
- (2) THE HASH, NOT fixed here: `func_item_hash.rs:emit_item_hash` is by design both the geometry-cache key and the damage identity (`.ai/canvas-threading.md` 14), so a fill-only change rebuilds every item's geometry (`generations` = items a frame). Owner: geometry keyed on geometry only, with appearance (paint and anything else that is not geometry) a second identity that damage also reads. This bug is the SPIKE for that: its deliverable is a verified, workable plan document, proven by throwaway measurements, not a landed change.
- OWNER RULING 2026-10-12: no hard-coded scene limit is wanted; the mechanism is the coordinator's choice from the code ("memory based" was an idea, not a requirement).

## Landed 2026-10-12 (bug-staging-23): frames drawn in passes; the hash split is plan-175-A

**Deliverable 1, the cap** (`61c75308e`, `ab9061ff8`; the bench row `12c9e8cf2`; RED
`477d7c21e`). No scene size is a limit on a GPU backend any more. A frame whose blocks
do not fit one frame buffer is drawn as consecutive runs of its draw list, each the
longest that fits every region, into one target (the first pass clears, later ones
load), each pass submitted and waited for before the buffer is refilled, presented or
read back once. Only a block that fits no pass by itself is still declined. The
mechanism and its rules are in `mfb spec app canvas` ("No scene size is a limit...") and
`.ai/canvas-threading.md` §10.

What was run, and what it showed:

- RED at the base (`84201af37`): `cargo test --release --test rt_canvas_metal --
  a_frame_past_one_buffers` → 3 failed, `gpuFrames=0` at 65,537 items, at 132,000
  gradient stops and at 9,216,000 texels; `tools/canvas-bench/bench.sh <base mfb> dots
  100000 5` → `renderer=Software ... gpuFrames=0`.
- Metal: all 22 `tests/canvas` targets pass (`cargo test --release --no-fail-fast` over
  every `--test rt_canvas_*`: 443 passed, 0 failed, 4 ignored, at `61c75308e`; the six
  largest again after `ab9061ff8`: 275 passed). The pass tests are
  65,535 / 65,536 / 65,537 / 140,000 items with translucent squares over their
  neighbours and over the lap before, 1,025 rings (two passes on EDGES), 513 curve rings
  (two passes on edge slots, two a piece), 44,000 three-stop gradients (two passes on
  STOPS), four 1,920 by 1,200 images (two passes on TEXELS), and a group drawn in both
  passes of a frame with a run cut by the boundary and Multiply fill-and-stroke items
  after it. Each must be a GPU frame within `Tolerance::GPU_DEFAULT` (`GPU_CURVE` for
  the curve rings) of the software oracle.
- Vulkan readback, `scripts/test-canvas-gpu-rows.sh <mfb> --target ... --rows
  items65535,items65536,items65537,items140000,passedges,passstops,passtexels,passgroups`:
  0 failures on box 2226 (lavapipe) and on box 206 (Intel UHD 630). Worst channel delta
  1 on every row but `passgroups` (2 on both); largest differing population 1.57%
  (`passgroups` on box 2226) against `GPU_DEFAULT`'s 2%.
- Vulkan in a real window on box 206 (swapchain present; `schtasks /it`, `--debug`,
  `MFB_CANVAS_GPU=1`, 100,000 dots, two runs): `frames=33 gpuFrames=33 directFrames=33`
  and `frames=34 gpuFrames=34 directFrames=34`. The base compiler in the same window:
  `frames=20 gpuFrames=0`, `renderer=Software`.
- Metal in a real window (`bench.sh <mfb> dots 100000 6 --window`): `frames=91
  gpuFrames=91 directFrames=91`.
- A frame under the cap is byte-identical to the base on the same device. Metal: each
  scene file of `tests/canvas/scenes` built with both compilers and dumped with
  `MFB_CANVAS_GPU=1` (`MFB_CANVAS_DUMP`, `cmp`): 52 identical, 2 not run (they need a
  font file or a pointer script), and 1 different, `many_texels`, which is a software
  frame at the base and two GPU passes now. The boxes: every row of
  `scripts/test-canvas-gpu-rows.sh` run with both compilers and `--keep`, the GPU
  frames compared with `cmp`. Box 206: 60 rows pass with the new compiler, 52 frames
  identical, 4 different (`items65537`, `items140000`, `passedges`, `passgroups`:
  software frames at the base). Box 2226: every row passes, 53 identical, 3 different
  (`items65537`, `items140000`, `passgroups`, likewise).
- A frame under the cap is not slower. Box 206, `bench.sh <mfb> --target windows-x86_64
  --host test@192.168.1.206 sweep`, base then new, twice: `phaseRenderMs` 30.54 / 30.56
  / 30.48 / 30.58 at 1,000 items, 73.52 / 73.66 / 73.79 / 73.48 at 5,000, 108.19 /
  108.39 / 108.24 / 108.49 at 10,000. Metal at 60,000 dots (one pass), base and new in
  turn three times, load average 13 to 24: `phaseRenderMs` 14.95 / 13.91, 13.76 /
  14.09, 14.09 / 13.69. The Metal `sweep` rows were taken at a load average of 80 to
  98 and decide nothing; the coordinator's re-measurement is one command each.
- A bug of the first cut, found by the box 206 rows and fixed in `ab9061ff8`: the pass
  word was read through a second scratch register before the arguments were parked, and
  on x86-64 that register is an argument register, so every frame with a `Text`
  faulted on Windows.

**The measured state after it, 100,000 dots** (`bench.sh <mfb> dots 100000`):

| Where | release pps / fps | `--debug` frames / gpuFrames / directFrames | offsets ms | render ms |
|---|---|---|---:|---:|
| Metal, headless (load 36) | 15 / 16 | 69 / 69 / 0 | 28.6 | 38.3 |
| Metal, real window (load 18) | 19 / 19 | 91 / 91 / 91 | 27.0 | 32.0 |
| box 206, headless, two runs | 4 / 4, 4 / 4 | 29 / 29 / 0 both | 59.5 / 59.4 | 209.1 / 210.8 |
| box 206, real window, two runs | 4 / 5, 4 / 5 | 33 / 33 / 33, 34 / 34 / 34 | 66 / 66 | 172 / 170 |

30 / 30 is NOT met, on either machine. Every frame is a GPU frame.

**Deliverable 2, the hash** is `planning/plan-175-A-canvas-geometry-identity-split.md`: a
geometry key `G`, an appearance identity `A` held per cache slot, damage on
`mix(G, A)`, and a paint-only change rewritten in place in the cached record. Its §2.3
is the attribution this document's Phase 1 asks for, measured at 10k / 50k / 60k / 100k
dots on box 206 and on Metal. In short, at 100,000 on box 206 per frame: the worker's
`present` 89 ms in release (93% of it the deep copy and the content compare), geometry
builds and compaction 45 ms of a 56 ms offsets phase, the gate and pass layout 17.7 ms,
writing the item blocks 0.41 µs a block (24.7 ms at 60,000), the GPU drawing them 1.4
µs a circle (82 to 89 ms at 60,000). The split removes the 45 ms and nothing else: its
upper bound (a throwaway geometry-only hash) is 4 fps to 4 or 5 on box 206, and 14 to
19 fps to 24 to 28 on Metal.

So this bug stays open for three costs the plan lists and does not remove (its §8): the
worker's copy and compare, the per-frame block upload, and on the Intel GPU the draw
itself, which no CPU-side change can bring under 33 ms at this count.

### Golden impact (not regenerated)

`scripts/artifact-gate.sh <mfb> all` reports the same 13 diffs with the new compiler as
with the base one (`diff` of the two sorted DIFF lists is empty). Of those 13, this
change alters the eight that hold the canvas source: `tests/syntax/app/app-mouse-surface`
(`.ir`, `.nir`, `.nplan` and the four `.app.ncode` sums) and
`tests/syntax/resources/loop-body-resource-reference-valid` (`.ir`). Found by building
every fixture that has an app-mode `ncode` golden with both compilers and comparing the
two outputs: only `app-mouse-surface` differs, on all four targets. The cause in each is
`helper_render.rs` (the gate and the pass walk replace the two predicates), three new
words in the graphics state before the pointer words
(`GRAPHICS_OFFSET_VULKAN_LOAD_PASS`, `_PRESENT_LOAD_PASS`, `_SC_HELD`), and the pass
branches in `emit_metal_draw` / `emit_vulkan_draw_scene` / `emit_submit_and_present`.

Two prebuilt scenes of 100,000 fixed `canvas::Circle` items, differing only in fill
color, sustain a median 3 presents/s and 4 frames/s on macOS Metal. Scene construction
is outside the timed region. The shortfall makes a dense dot raster visibly step
instead of zoom smoothly.

**The single correct behavior a fix produces:** alternating the colors of 100,000 fixed
dots through the existing `canvas::present` / `canvas::waitFrame` API sustains at least
30.0 PPS and 30.0 FPS in a release app on a supported hardware GPU, without changing
Canvas pixels, ownership, frame-skip, or threading semantics.

References:

- `bugs/completed/bug-686-metal-declines-ordinary-scenes-and-falls-back-to-software.md`
  — optimized the shared Canvas path around roughly 10,000 moving items and explicitly
  descoped higher moving-item counts; its `worker` measurement and G2 goal do not meet
  this bug's 100,000-item budget.
- `bugs/bug-688-vulkan-canvas-lacks-bug-686-metal-parity.md` — Vulkan parity and the
  real-hardware Windows performance matrix.
- `tools/canvas-bench/README.md` — the existing cross-platform Canvas performance
  instrument this bug must extend rather than creating a second benchmark framework.
- Discovered while changing `examples/fractals` from a 25,000-dot 3D cloud to a fixed
  2D dot raster.

## Re-verified 2026-10-12: still fails, and the frame is drawn in SOFTWARE (HEAD `555130a25`)

The program below no longer compiles (plan-172: `Circle` takes `id`, `position` and
`transform`; `canvas::Size` holds Floats). Today's syntax: `bugs/repro/bug-765/dots.mfb`.

`mfb build --app`, real window, macOS AArch64, three runs:

```
count=100000 frames=27 pps=5.00 fps=5.00
count=100000 frames=51 pps=5.00 fps=6.00
count=100000 frames=51 pps=5.00 fps=5.00
```

5 PPS / 5 FPS against the 3 / 4 first recorded; still far under 30.

**New: Metal declines this scene, which this document never considered.** The same
program built `--debug` and run headless with `MFB_CANVAS_GPU=1 MFB_CANVAS_STATS`, last
stats line:

| dots | `frames` | `gpuFrames` | `generations` | `phaseOffsetsMs` | `phaseRenderMs` | program's line |
| --- | --- | --- | --- | --- | --- | --- |
| 100,000 (400×250) | 49 | **0** | 4,900,000 | 1,127 | 8,721 | `pps=5.00 fps=5.00` |
| 60,000 (300×200) | 376 | 376 | 22,560,000 | 4,487 | 5,087 | `pps=38.00 fps=38.00` |

- 100,000 items is past `METAL_MAX_FRAME_ITEMS` (65,536,
  `src/codegen/runtime/canvas/mod.rs`; `VULKAN_MAX_FRAME_ITEMS` is the same), so the
  predicate declines every frame and the software renderer draws it: 178 ms of render a
  frame (8,721 / 49) in this debug build. At 60,000 items, under the cap, every frame is a
  GPU frame and the same loop holds 38 PPS / 38 FPS, debug build included.
- So the first cause of the 5 FPS is the frame cap, not the publish, hash or upload slope
  the hypotheses below rank. A fix has to raise the cap (the item buffer is sized from
  it), or draw a frame over the cap in more than one pass, before any of Phase 2 or 3
  can show in this reproduction.
- **Hypothesis 2 is confirmed:** `generations` is 100,000 a frame (4,900,000 / 49) and
  60,000 a frame (22,560,000 / 376), so a fill-only change rebuilds every item's geometry.
- What the GPU path costs at 60,000: 11.9 ms of offsets and 13.5 ms of render a frame
  (debug build). Scaled linearly to 100,000 that is 42 ms, over the 33.3 ms budget. That
  figure is a guess from one point, not a measurement: nothing over the cap can be run on
  the GPU today.
- The Goal's "no run takes a fallback renderer" (Phase 3) already rules the software
  frame out; Phase 1 should record `gpuFrames` for every row so a declined frame is seen.

Not re-run: Windows (box 206). `planning/plan-173-canvas-atlas-tile.md` (atlas tiles and
culling) is the only canvas plan filed since; it does not name this bug.

## Failing Reproduction

Create `/tmp/mfb-canvas-100k/project.json`:

```json
{
  "name": "canvas100k",
  "version": "0.1.0",
  "mfb": "1.0",
  "kind": "executable",
  "mode": "app",
  "sources": [{"root": "src", "role": "main", "include": ["**/*.mfb"]}],
  "entry": "main",
  "targets": ["native"]
}
```

Create `/tmp/mfb-canvas-100k/src/main.mfb`:

```basic
IMPORT app
IMPORT canvas
IMPORT collections
IMPORT color
IMPORT datetime
IMPORT io
IMPORT math

LET COLUMNS AS Integer = 400
LET ROWS AS Integer = 250
LET COUNT AS Integer = COLUMNS * ROWS

FUNC main AS Integer
  app::setMode(app::Mode.Canvas)
  app::setTitle("Canvas 100k dot probe")
  LET size AS canvas::Size = canvas::getSize()
  LET cellW AS Float = toFloat(size.width) / toFloat(COLUMNS)
  LET cellH AS Float = toFloat(size.height) / toFloat(ROWS)
  LET radius AS Float = math::min(cellW, cellH) * 0.4
  LET empty AS canvas::DrawItem = canvas::Circle[x := 0.0, y := 0.0, radius := radius, paint := canvas::fill(color::rgb(0, 0, 0))]
  MUT sceneA AS List OF canvas::DrawItem = collections::repeat([], empty, COUNT)
  MUT sceneB AS List OF canvas::DrawItem = collections::repeat([], empty, COUNT)
  FOR i = 0 TO COUNT - 1
    LET column AS Integer = i MOD COLUMNS
    LET row AS Integer = i / COLUMNS
    LET a AS canvas::DrawItem = canvas::Circle[x := (toFloat(column) + 0.5) * cellW, y := (toFloat(row) + 0.5) * cellH, radius := radius, paint := canvas::fill(color::rgb(64, 128, 191))]
    LET b AS canvas::DrawItem = canvas::Circle[x := (toFloat(column) + 0.5) * cellW, y := (toFloat(row) + 0.5) * cellH, radius := radius, paint := canvas::fill(color::rgb(192, 128, 63))]
    sceneA = collections::set(sceneA, i, a)
    sceneB = collections::set(sceneB, i, b)
  NEXT
  LET started AS Integer = datetime::monotonicNanos()
  MUT frame AS Integer = 0
  DO WHILE datetime::monotonicNanos() - started < 10000000000
    IF frame MOD 2 = 0 THEN
      canvas::present(sceneA)
    ELSE
      canvas::present(sceneB)
    END IF
    canvas::waitFrame()
    frame = frame + 1
  LOOP
  LET stats AS canvas::FrameStats = canvas::getFrameStats()
  io::print("count=" & toString(COUNT) & " frames=" & toString(frame) & " pps=" & toString(stats.pps) & " fps=" & toString(stats.fps))
  RETURN 0
END FUNC
```

Build and run three times:

```sh
target/debug/mfb build /tmp/mfb-canvas-100k
/tmp/mfb-canvas-100k/build/canvas100k.app/Contents/MacOS/canvas100k
/tmp/mfb-canvas-100k/build/canvas100k.app/Contents/MacOS/canvas100k
/tmp/mfb-canvas-100k/build/canvas100k.app/Contents/MacOS/canvas100k
```

- Observed on macOS AArch64 / Metal, 2026-09-28:
  - `count=100000 frames=25 pps=2.00 fps=3.00`
  - `count=100000 frames=32 pps=3.00 fps=4.00`
  - `count=100000 frames=38 pps=3.00 fps=4.00`
  - median: 3 PPS / 4 FPS.
- Expected: every steady-state run reports at least 30.0 PPS and 30.0 FPS.
- Contrast: the two scenes are completely built before `started`, proving the timed
  shortfall is not the program's loop that constructs the 100,000 records.

| Environment | Architecture / renderer / build | Result |
| --- | --- | --- |
| Local macOS | AArch64, Metal, release app, ordinary window | 3 PPS / 4 FPS median — fails ✗ |
| Windows | x86-64, Vulkan on real GPU (box 206) | not measured yet |
| Linux | Vulkan | no reachable real-GPU machine; lavapipe numbers are not acceptance evidence |

## Root Cause

The total contribution of each phase is not measured yet; Phase 1 must add the
100,000-dot row and collect the existing worker/graphics/render counters before a design
is chosen. The known mechanisms and ordered hypotheses are:

1. **Whole-scene publication is linear and mandatory for every changed frame.**
   `src/codegen/builtins/canvas/gen_present.rs:emit_publish` deep-copies the incoming
   flat scene before its exact comparison. Alternating the two prebuilt scenes proves
   this and later Canvas work remain inside the timed region. Confirm its share with a
   worker-only benchmark row and phase timers.
2. **A color change invalidates the same structural hash that keys geometry.**
   `src/codegen/builtins/canvas/func_item_hash.rs:emit_item_hash` folds every field of
   `Circle`, including its nested `Paint`; its module contract says that hash is both
   the geometry-cache key and the damage identity. `canvas::sceneHashes` therefore
   recomputes 100,000 item hashes when all fills change. Confirm whether the color-only
   scene also records 100,000 geometry generations per frame through the existing
   `generations` counter. If so, split geometry identity from appearance/damage identity
   without weakening either comparison.
3. **The renderer still has a per-item upload/draw slope above bug-686's 10,000-item
   target.** `src/codegen/builtins/canvas/func_geo_cache.rs:lower_scene_resolve` walks
   items and resolves cache entries, while
   `src/codegen/builtins/canvas/func_metal_present.rs:lower_metal_present_scene` and
   `src/codegen/builtins/canvas/func_metal_draw.rs:lower_vulkan_present_scene` hand the
   resulting draw list to their backends. Measure worker, resolve, encode/upload, GPU,
   and present independently; do not assume the GPU is the bottleneck.

The earlier bug-686 worker measurement was 2.9–3.6 microseconds per item for 10,000
items (`tools/canvas-bench/README.md`, `worker`). Extrapolated to 100,000 items that is
290–360 ms per present, consistent with this reproduction's 250–333 ms frame interval;
the 30 PPS budget is 33.3 ms total, so an incremental improvement cannot meet the goal.

## Goal

- A release-build 400×250 fixed-circle scene that alternates only fill colors sustains
  at least 30.0 PPS and 30.0 FPS for a 10-second steady-state window on macOS Metal and
  every supported Vulkan backend with a real hardware GPU.
- The worker, graphics-thread CPU, GPU render, and present contributions are each
  measured, and their sum fits the 33.3 ms frame budget rather than hiding work outside
  the reported interval.
- Output remains pixel-equivalent to the software oracle within the existing GPU
  tolerance.

### Non-goals (must NOT change)

- Do not reduce the item count, coalesce the dots in the benchmark, update only a
  subset per frame, or measure an unchanged scene that takes the frame-skip path.
- Do not replace the example's dots with one pre-rendered image and call that
  100,000-item Canvas support.
- Do not weaken `present`'s deep-copy ownership contract, exact unchanged-scene skip,
  scene-ring thread safety, damage correctness, or group/resource lifetime rules.
- Do not weaken or re-baseline pixel-oracle tolerances to hide rendering differences.
- Do not use Linux lavapipe FPS as hardware-GPU acceptance evidence.
- Do not require a new public API unless profiling proves the existing `present` API
  cannot meet the target and the user explicitly approves that API change.

## Blast Radius

The audit starts from:

```sh
rg -n "emit_publish|__canvas_hashScene|emit_item_hash|lower_scene_resolve|lower_metal_present_scene|lower_vulkan_present_scene" src/codegen/builtins/canvas src/codegen/runtime/canvas src/target
```

- `canvas::present` / `gen_present.rs:emit_publish` — fixed by this bug; the measured
  flat-scene path.
- `canvas::presentLayers` / the same `emit_publish` generator — fixed in this bug if
  the publication change is shared; it has the same whole-scene mechanism.
- `func_item_hash.rs:emit_item_hash` and
  `func_geo_cache.rs:lower_scene_resolve` — fixed in this bug if profiling confirms
  appearance-only changes rebuild geometry.
- macOS Metal direct presentation through
  `func_metal_present.rs:lower_metal_present_scene` — measured failing path, fixed by
  this bug.
- Windows/Linux Vulkan through
  `func_metal_draw.rs:lower_vulkan_present_scene` and
  `runtime/canvas/vulkan.rs:emit_vulkan_draw_scene` — same shared publication and
  geometry path; in scope, with FPS acceptance only on real GPU hardware.
- Software rasterizer and GPU readback/dump paths — correctness must remain covered,
  but 30 FPS is not required because they are diagnostic/oracle fallbacks rather than
  the hardware presentation path.
- Identical-scene frame skip — unaffected behavior and a required regression guard;
  it must remain a no-render path.

## Fix Design

Do not select the implementation before Phase 1 attributes the frame. The likely shape
is a combination of:

1. separate geometry identity from appearance/damage identity so a fill-only update
   reuses fixed circle geometry;
2. move color changes through a compact per-item appearance upload rather than
   regenerating structural geometry records;
3. remove redundant full-scene copies/hashes from the synchronous present path while
   preserving the scene-ring adoption and ownership contract; and
4. batch the 100,000 homogeneous circle draws in the backend.

Rejected shortcuts: fewer dots, partial/interlaced updates, a picture tile, relaxed
pixel tolerances, asynchronous queueing that inflates PPS while FPS stays below 30, or
an unchanged scene that exercises the skip.

Any fix touching the shared scene/hash/geometry path must update the embedded Canvas
spec in `src/docs/spec/app/06_canvas.md`; backend-only mechanics need matching cited
spec detail if they change the contributor contract.

## Phases

### Phase 1 — failing benchmark + phase audit (no behavior change)

- [x] Extend `tools/canvas-bench` with `dots N`, generating two prebuilt fixed-circle
      scenes whose fills differ and alternating them for a measured 10-second window.
- [x] Make `dots 100000` assert 30.0 PPS and 30.0 FPS in release mode and confirm it
      fails for the documented reason on macOS Metal.
- [x] Record worker publish, scene copy/compare, hashing, geometry generations,
      graphics-thread resolve, backend encode/upload, GPU, and present times.
      (plan-175-A §2.3; the present itself is not separated from the GPU's time.)
- [x] Run 10k, 50k, and 100k rows to determine scaling, plus identical-scene and
      position-changing contrast rows. (plan-175-A §2.3; `dots N --same`, `--move`.)
- [x] Complete the Windows real-GPU matrix on box 206; record Linux as hardware-blocked
      until a real-GPU host exists. (Above and plan-175-A §2.3. Linux: box 2226 is
      lavapipe, correctness only.)
- [ ] Finish the blast-radius verdict for `presentLayers`, groups, text, pictures,
      damage-only rendering, readback, and both GPU backends.

Acceptance: the committed 100k benchmark fails below 30/30 for the documented phase(s),
the pixel oracle passes, and every shared consumer has a written verdict.
Commit: —

### Phase 2 — remove worker publication and identity bottlenecks

- [ ] Implement the measured publication/copy/hash changes without changing public
      ownership or frame-skip semantics.
- [ ] Split geometry and appearance identity if Phase 1 confirms fill-only geometry
      churn; preserve exact damage identity.
- [ ] Add unit/runtime coverage for changed-color, changed-geometry, identical-scene,
      group, resource-close, and publish-race cases.

Acceptance: the Phase 1 worker/hash/geometry thresholds fit their assigned share of the
33.3 ms budget; every correctness contrast remains green.
Commit: —

### Phase 3 — backend batching and 30/30 target

- [ ] Optimize the Metal and Vulkan draw/upload paths indicated by Phase 1, sharing
      representation where their contracts match.
- [ ] Prove the color-only 100k scene remains on the hardware GPU and is pixel-equivalent
      to the software oracle.
- [ ] Reach at least 30.0 PPS and 30.0 FPS in three consecutive 10-second release runs
      on macOS Metal and Windows Vulkan real hardware.

Acceptance: all six hardware runs meet 30/30, no run takes a fallback renderer, and the
oracle comparison stays within the existing tolerance.
Commit: —

### Phase 4 — spec sync and full validation

- [ ] Update `src/docs/spec/app/06_canvas.md` citations and implementation contract for
      any changed publication, identity, caching, or backend behavior.
- [ ] Run the full Rust suite, full artifact gate, acceptance suite, macOS app gate,
      Canvas Metal suite, and the Vulkan GPU rows on supported real hardware.
- [ ] Re-run 10k/50k/100k, unchanged, geometry-changing, and color-changing benchmark
      rows; archive raw results with the bug.

Acceptance: all suites are green, golden changes are limited to expected Canvas codegen,
the original reproduction meets 30/30, and the spec describes the implementation at
HEAD.
Commit: —

## Validation Plan

- Regression test: `tools/canvas-bench/bench.sh <mfb> dots 100000`, with 10k and 50k
  scaling rows and unchanged/geometry-changing contrasts.
- Runtime proof: three consecutive 10-second release runs at 30.0 PPS and 30.0 FPS on
  local macOS Metal and Windows Vulkan box 206, plus the existing software-vs-GPU pixel
  comparison.
- Doc sync: `src/docs/spec/app/06_canvas.md` if publication, hash/cache identity, or
  backend representation changes.
- Full suite: `cargo test --release --no-fail-fast`; serially
  `scripts/artifact-gate.sh <mfb> all` and `scripts/test-accept.sh <mfb> <actual-dir>`;
  `scripts/test-macapp.sh`; `cargo test --test rt_canvas_metal`; and
  `scripts/test-canvas-gpu-rows.sh` on each hardware-backed target.

## Open Decisions

- **Public API** — keep `canvas::present` unchanged (recommended) unless Phase 1 proves
  its ownership contract makes 30/30 impossible; any retained/dynamic-scene API requires
  explicit user approval.
- **Threshold gate** — require three consecutive release runs (recommended) rather than
  one best run; retain raw phase counters so a noisy host cannot turn a regression green.
- **Linux acceptance** — block only the Linux FPS row until real GPU hardware exists;
  keep Linux Vulkan correctness/oracle rows mandatory on lavapipe.

## Summary

The risk is not drawing a circle correctly; it is changing the representation and
publication path without breaking Canvas's thread ownership, exact damage, cache,
resource, and frame-skip contracts. The current evidence localizes the failure to the
timed `present`/render path even with both 100,000-item scenes prebuilt. Phase 1 must
separate copying, hashing, geometry churn, backend upload, and GPU cost before the fix
chooses which layers to change.
