# bug-765: Canvas renders 100,000 color-changing dots at 3 PPS / 4 FPS instead of 30

Last updated: 2026-10-12
Effort: huge (>3d)
Severity: HIGH
Class: Other (Performance)

Status: Open
Regression Test: planned `tools/canvas-bench/bench.sh <mfb> dots 100000`

## Cause (coordinator, 2026-10-12, bug-staging-23)

- Two causes, both measured below, and neither has an existing rule to restore. (1) 100,000 items is past `METAL_MAX_FRAME_ITEMS`/`VULKAN_MAX_FRAME_ITEMS` (65,536), which size the item buffer, so the honesty gate correctly declines the frame: drawing it needs a larger cap or a multi-pass frame. (2) `func_item_hash.rs:emit_item_hash` is by design both the geometry-cache key and the damage identity (`.ai/canvas-threading.md` 14), so a fill-only change rebuilds every item's geometry (`generations` = items a frame): fixing it needs a new split of geometry identity from appearance. Waiting on the owner.

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

- [ ] Extend `tools/canvas-bench` with `dots N`, generating two prebuilt fixed-circle
      scenes whose fills differ and alternating them for a measured 10-second window.
- [ ] Make `dots 100000` assert 30.0 PPS and 30.0 FPS in release mode and confirm it
      fails for the documented reason on macOS Metal.
- [ ] Record worker publish, scene copy/compare, hashing, geometry generations,
      graphics-thread resolve, backend encode/upload, GPU, and present times.
- [ ] Run 10k, 50k, and 100k rows to determine scaling, plus identical-scene and
      position-changing contrast rows.
- [ ] Complete the Windows real-GPU matrix on box 206; record Linux as hardware-blocked
      until a real-GPU host exists.
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
