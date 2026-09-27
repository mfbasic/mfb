# bug-735: every GPU shape but a filled polygon or a rectangle still flips a coverage step against the oracle near black

Last updated: 2026-09-25
Effort: huge (>3d)
Severity: MEDIUM
Class: Correctness (GPU/oracle agreement)

Status: Open — split from bug-687 by owner decision (2026-09-25)
Regression Test: none yet. The reproduction below builds one scene per kind; each
becomes a `tests/canvas/scenes/*_on_black.mfb` row in `rt_canvas_metal.rs` and
`scripts/test-canvas-gpu-rows.sh` when its kind is fixed.

bug-687 fixed filled polygons. Each vertex now travels at double-float precision and
the polygon's distance, crossing test, inverse transform and coverage rounding are
double-float in both shaders. Every other shape still computes its signed distance in
fp32 from inputs narrowed to 16.16 fixed point, against the software oracle's f64. Near
black, one coverage step is several sRGB levels, and a distance landing within ~1e-5 px
of a quantisation boundary rounds to the neighbouring step. So these kinds put pixels
past `Tolerance::GPU_DEFAULT` (max channel delta 2), by 3-9 levels, on both Metal and
Vulkan:

- circles, ellipses, lines, arcs and polylines;
- a polygon's stroke when the half-width is off the 16.16 grid;
- any item in a group drawn at an offset off the 16.16 grid.

**The single correct behavior a fix produces:** every shape kind, stroked or filled,
grouped or not, agrees with the software oracle within `GPU_DEFAULT` over a black
background on Metal and Vulkan. That means 0 pixels past delta 2 on each row below,
with the comparison instrument and the quantisation unchanged.

References:

- `bugs/completed/bug-687-metal-coverage-flips-past-gpu-tolerance-near-black.md` (once
  archived): the mechanism, the C model (V1-V5), the V5 design this would extend, and
  its "Phase 3" measurement, which is the table below.
- `mfb spec app canvas` ("A polygon's coverage on either GPU is computed in
  double-float…").
- `.ai/testing-gates.md`, "The GPU comparison is against the ORACLE".

## Failing Reproduction

The probe generator below builds 140 items of one kind on a 60-px grid, so nothing
overlaps. Positions and sizes are off the pixel and 16.16 grids, and the fill is
`rgb(250, 250, 120)` over the default opaque-black surface. Save it as `gen.py`:

```python
import sys
kind=sys.argv[1]
hdr='''IMPORT app
IMPORT canvas
IMPORT color
IMPORT collections
IMPORT math

FUNC ring(n AS Integer, cx AS Float, cy AS Float, r AS Float) AS List OF canvas::Point
  MUT points AS List OF canvas::Point = []
  MUT i AS Integer = 0
  WHILE i < n
    LET a AS Float = toFloat(i) * 6.283185307179586 / toFloat(n)
    LET rr AS Float = r + 3.0 * math::sin(a * 5.0)
    points = collections::append(points, canvas::Point[x := cx + rr * math::cos(a), y := cy + rr * math::sin(a)])
    i = i + 1
  END WHILE
  RETURN points
END FUNC

SUB main()
  app::setMode(app::Mode.Canvas)
  MUT scene AS List OF canvas::DrawItem = []
  MUT k AS Integer = 0
  WHILE k < 140
    LET fx AS Float = toFloat(k) * 7.3719
    LET x AS Float = 32.0 + toFloat((k MOD 14) * 60) + math::sin(fx) * 0.37
    LET y AS Float = 32.0 + toFloat((k / 14) * 60) + math::cos(fx) * 0.41
    LET r AS Float = 8.0 + toFloat(k MOD 13) * 1.513
    LET c AS color::Color = color::rgb(250, 250, 120)
'''
body={
'circle':'    LET d AS canvas::DrawItem = canvas::Circle[x := x, y := y, radius := r, paint := canvas::fill(c)]',
'ellipse':'    LET d AS canvas::DrawItem = canvas::Ellipse[x := x, y := y, radiusX := r, radiusY := r * 0.61, angle := fx, paint := canvas::fill(c)]',
'line':'    LET d AS canvas::DrawItem = canvas::Line[x1 := x - r, y1 := y - r * 0.7, x2 := x + r, y2 := y + r * 0.9, cap := canvas::CapStyle.Round, paint := canvas::stroke(c, 2.0)]',
'arc':'    LET d AS canvas::DrawItem = canvas::Arc[x := x, y := y, radius := r, startAngle := 0.3, endAngle := 4.1, cap := canvas::CapStyle.Butt, paint := canvas::stroke(c, 2.0)]',
'rect':'    LET d AS canvas::DrawItem = canvas::Rectangle[x := x - r, y := y - r * 0.6, w := r * 1.9, h := r * 1.3, paint := canvas::fill(c)]',
'polygon':'    LET d AS canvas::DrawItem = canvas::Polygon[points := ring(40, x, y, r), paint := canvas::fill(c)]',
'strokehalf':'    LET d AS canvas::DrawItem = canvas::Polygon[points := ring(40, x, y, r), paint := canvas::stroke(c, 1.0 + toFloat(k MOD 7) * 0.3137)]',
'polyline':'    LET d AS canvas::DrawItem = canvas::PolyLine[points := ring(40, x, y, r), colors := [], cap := canvas::CapStyle.Round, paint := canvas::stroke(c, 2.0)]',
}.get(kind,'')
tail='''
    scene = collections::append(scene, d)
    k = k + 1
  END WHILE
'''
if kind=='group':
    body='    LET d AS canvas::DrawItem = canvas::Polygon[points := ring(40, 0.0, 0.0, r), paint := canvas::fill(c)]\n    canvas::setGroup("g" & toString(k), [d])\n    LET gd AS canvas::DrawItem = canvas::Group[name := "g" & toString(k), dx := x, dy := y]'
    tail='''
    scene = collections::append(scene, gd)
    k = k + 1
  END WHILE
'''
print(hdr+body+tail+'  canvas::present(scene)\nEND SUB')
```

Then for each kind (`circle ellipse line arc rect polygon strokehalf polyline group`):

```
python3 gen.py circle > probe/src/main.mfb      # probe/project.json: a plain executable named "probe"
mfb build -app --debug probe
for m in 0 1; do
  MFB_MACAPP_HEADLESS=1 MFB_CANVAS_SYNC=1 MFB_CANVAS_GPU=$m MFB_CANVAS_DUMP=frame-$m \
    probe/build/probe.app/Contents/MacOS/probe
done
python3 scripts/rgba_compare.py frame-0 frame-1
```

On Linux, `mfb build --app --debug --target linux-aarch64` and run the extracted
AppImage with `MFB_GTKAPP_HEADLESS=1` on box 2226, as `scripts/test-canvas-gpu-rows.sh`
does. Measured 2026-09-25; each cell is pixels past delta 2 / worst channel delta:

| kind | Metal before bug-687 (`8158a9524`) | Metal after bug-687 | Vulkan 2226 (lavapipe) after |
| --- | --- | --- | --- |
| rectangle (contrast: passes) | 0 / 2 | 0 / 2 | 0 / 2 |
| polygon, filled (contrast: fixed by bug-687) | 3 / 4 | 0 / 1 | 0 / 1 |
| polygon, stroke width 1.0-2.9 (half-width off 16.16) | 4 / 6 | 1 / 3 | 2 / 3 |
| polygon in a group at an off-grid offset | 3 / 4 | 3 / 4 | 4 / 4 |
| polyline | 2 / 6 | 2 / 6 | 4 / 6 |
| circle | 2 / 6 | 2 / 6 | 3 / 6 |
| ellipse | 8 / 9 | 8 / 9 | 12 / 9 |
| line | 1 / 3 | 1 / 3 | 1 / 3 |
| arc | 3 / 9 | 3 / 9 | 3 / 9 |

- Observed: e.g. ellipse, Metal, 8 pixels past delta 2, worst 9.
- Expected: 0 pixels past delta 2 for every kind.
- Contrast: rectangles pass (worst 2, only just) and filled polygons pass since bug-687.
  Both are guarded by `tests/canvas/scenes/rectangles_on_black.mfb` and
  `small_polygons_on_black.mfb`.

## Root Cause

bug-687's mechanism, for every kind it did not convert.

1. **Precision.** Each kind's shader distance (`rectDistance`, `length(p - c) - r`,
   `segmentDistance` / `segmentDistanceButt`, `ellipseDistance`, the arc band and caps,
   `polylineScan`) runs in fp32 in `METAL_SHADER_BODY`
   (`src/target/macos_aarch64/app/metal.rs`) and in
   `src/codegen/runtime/canvas/shaders/mfb_canvas.frag`. The oracle's twins in
   `src/codegen/builtins/canvas/helper_shapes.rs` and `helper_draw.rs` run in f64. fp32
   at a few hundred pixels resolves ~3-6e-5 px, which is enough to land on the other side
   of a coverage boundary.
2. **Input narrowing.** The item block carries every shape parameter as 16.16
   (`emit_item_block`): centres, radii, ellipse axes, line endpoints, arc sweep
   endpoints, the stroke half-width `misc.z`, and the clip. The per-draw group offset
   `drawOffset` is 16.16 too. The oracle uses the f64 values. bug-687's C model showed
   that for polygons this narrowing alone (V4) left pixels past tolerance even with exact
   arithmetic. That is why V5 carries residual words, and the same applies to every one
   of these fields.
3. **Kind-specific extras.**
   - The arc takes `cos`/`sin` of 16.16 sweep angles on the GPU, and GPU
     transcendentals do not match the oracle's deterministic Taylor pair.
   - The ellipse's 24-step bisection compares fp32 values on every step.
   - The polyline's butt planes and path parameter are fp32.

Why rectangles pass is not established. A guess: their distance near an edge is one
subtraction of axis-aligned values, so the fp32 error is smaller. Worst 2 is the
tolerance's edge, so it is a margin, not a guarantee.

## Goal

- 0 pixels past `GPU_DEFAULT` on the reproduction for every kind in the table, on Metal
  (local), Vulkan 2226 and Vulkan 2230, with each kind's scene added as an oracle row.

### Non-goals (must NOT change)

- `tests/common/canvas_image.rs`, `Tolerance::GPU_DEFAULT` and the quantisation
  `int(clamp(0.5 - d, 0, 1) * 255 + 0.5)`. Widening any of them is the tempting wrong
  fix.
- The software rasteriser's output and every canvas golden.
- bug-687's polygon path. Its rows must stay GREEN.

## Blast Radius

Found by reading every `geoDistance` arm in both shaders (`grep -n "item.misc.x ==" `
in `metal.rs` and `mfb_canvas.frag`):

- kinds 1 (circle), 2 (line), 3 (arc), 7 (ellipse), 10 (polyline): fp32 from 16.16.
  Fixed by this bug.
- kind 4 (polygon): the stroke half-width (`misc.z`) is 16.16. Fixed by this bug.
- every kind under `Group`: `drawOffset` is 16.16 per draw. Fixed by this bug.
- kind 0 (rectangle) and 9 (picture, a rectangle to the distance field): fp32 from
  16.16, passes the probe at worst 2. Latent, same hazard. Convert them with the rest,
  or record why they are exempt.
- kind 6 (glyph): coverage is looked up from the CPU's bitmap, no distance. Unaffected.
- `clipCoverage`: fp32 rectangle SDF from a 16.16 clip. Latent, same hazard as the
  rectangle. Not probed.

## Fix Design (cost outline — decide before implementing)

Extend V5 to every kind:

1. **Residual words for every distance-feeding field**: centres, radii, axes, endpoints,
   arc cap points, half-width, clip. This grows `ITEM_BLOCK_SIZE`, which moves six things
   (`.ai/canvas-threading.md`, "Growing `ITEM_BLOCK_SIZE`"). Alternatively put them in a
   parallel per-item region, as bug-687 did for edges.
2. **The group offset at double-float precision**: a second word per axis in the
   per-draw bytes (`METAL_DRAW_OFFSET_*` and Vulkan's push constant).
3. **A double-float twin of each SDF**, with the same "decide in fp32, double-float only
   where fp32 cannot decide" pre-pass bug-687's `edgeSlack` uses, so the cost stays
   bounded:
   - the ellipse needs a double-float bisection (24 steps);
   - the arc needs the sweep endpoints and the sweep test computed from CPU-supplied
     cos/sin pairs, as the ellipse already does, not GPU trig.
4. **One call site per double-float helper**: lavapipe compiles the whole shader at
   pipeline creation (bug-687 measured a hang past 16 minutes otherwise).

Risk: the ellipse and the arc. The alternatives are bug-687's option 1 (a
quantum-aware comparison) or option 3 (a coarser shared quantisation) for the kinds V5
does not reach. Both need a new owner decision.

## Decisions (recorded 2026-09-26)

- **Design: the doc's Fix Design, V5 extended to every kind.** No owner decision was
  needed: the Phase 1 model reached `GPU_DEFAULT` for every kind, so options 1 and 3 were
  not reopened.
- **Layout: grow the item block** (208 → 272) by four residual `ivec4`s rather than a
  parallel region: every item has these fields, so a per-item index would cost as much
  as the words.
- **Rectangle, picture and `clipCoverage`: converted, not exempted.** The probe's
  rectangles pass at worst 2 only because their edges land away from a step. Placed
  0.5-4.5e-6 px from a step, 16.16 alone moves 3,623 rectangle pixels and 4,684 clip
  pixels past `GPU_DEFAULT` (worst 12) on Metal, 2226 and 2230 (Phase 1). A picture is a
  rectangle to the distance field (the same arm in both shaders), so it is covered by
  the rectangle row.
- **Arc sweep:** the direction vectors are the cap endpoints less the centre, not
  CPU-supplied `cos`/`sin`. They are a positive multiple of the oracle's Taylor pair, so
  the sweep's sign agrees wherever it is decided by more than ~1e-13 px, and the header
  needed no new slot. `reflex` is decided by the emitter in f64 (`arc.y`).

## Phases

### Phase 1: failing rows and the model

- [x] One `*_on_black.mfb` scene per kind from the generator; RED on Metal, 2226, 2230.
- [x] Extend bug-687's C model to each kind (V1 today vs V5 extended) before touching a
      shader; stop if a kind's model does not reach `GPU_DEFAULT`. **It reaches it for
      every kind.**

#### Phase 1 results (2026-09-26, worktree `worktree-B-735` at `0cd2db799`)

**Rows.** Thirteen scenes in `tests/canvas/scenes/`, each a `rt_canvas_metal.rs` test and a
`scripts/test-canvas-gpu-rows.sh` row. The first eight are the generator's kinds (above);
`stroked_shapes` adds the variants the generator does not draw (butt lines, round arcs,
filled-and-stroked circles, ellipses and rounded rectangles, butt polylines, every width
off the 16.16 grid); `transformed_shapes` is the same under per-item transforms with
every other item clipped; the two `*_at_a_step` scenes put every rectangle or clip edge
0.5-4.5e-6 px from a coverage step; `blended_hairline_stroke` is the bug found below.
RED at `0cd2db799` (Metal: `cargo test --release --test rt_canvas_metal -- over_black`,
10 failed; Vulkan: `scripts/test-canvas-gpu-rows.sh <mfb> --target linux-aarch64` and
`--target windows-x86_64 --rows ...`). Pixels past delta 2 / worst:

| row | Metal | Vulkan 2226 (lavapipe) | Vulkan 2230 (lavapipe) |
| --- | --- | --- | --- |
| `circles_on_black` (circlesblack) | 2 / 6 | worst 6 | worst 6 |
| `ellipses_on_black` (ellipsesblack) | 8 / 9 | worst 9 | worst 9 |
| `lines_on_black` (linesblack) | 1 / 3 | worst 3 | worst 3 |
| `arcs_on_black` (arcsblack) | 3 / 9 | worst 9 | worst 9 |
| `stroked_polygons_on_black` (strokedpolysblack) | 1 / 3 | worst 3 | worst 3 |
| `polylines_on_black` (polylinesblack) | 2 / 6 | worst 6 | worst 6 (second run; first: lavapipe crash, below) |
| `grouped_polygons_on_black` (groupsblack) | 4 / 4 | worst 4 | worst 4 (second run; first: lavapipe crash) |
| `stroked_shapes_on_black` (strokedblack) | 5 / 12 | worst 12 | worst 12 |
| `transformed_shapes_on_black` (xformblack) | 5 / 9 | worst 4 | lavapipe crash (below) |
| `rectangles_at_a_step_on_black` (rectstepblack) | 3,623 / 12 | worst 12 | worst 12 |
| `clips_at_a_step_on_black` (clipstepblack) | 4,684 / 12 | worst 12 | worst 12 |
| `blended_hairline_stroke` (hairline) | 20,632 / 101 | worst 101 | worst 101 |

Every Vulkan run above drew on the GPU (`gpuFrames=1`, `geoVerifyMismatches=0`) unless it
says crash.

**The C model** is `/tmp/b735/model.c` (`cc -O2 -ffp-contract=off -o model model.c -lm;
./model KIND sw.rgba gpu.rgba`). It rebuilds each generator scene's items, including the
MFBASIC compiler's FMA contractions (`fuse_scalar_fma`: a single-use `a*b` feeding `+`/`-`
is one rounding), and evaluates the oracle's f64 distance for each kind, today's fp32
shaders from 16.16 (V1), and the V5 design exactly as the shaders now run it, fp32
pre-passes included. Checked before believed: the oracle model reproduces the software
dump **bit for bit on all twelve scenes** it covers, and V1 predicts **every** Metal pixel
past tolerance (e.g. rectstep 3,623 of 3,623, ellipse 8 of 8, strokehalf 1 of 1 where the
device's own encode adds the step). Coverage mismatches / pixels past 2 / worst:

| scene | V1 (today) | V5 (design) |
| --- | --- | --- |
| circle | 49 / 2 / 6 | 0 / 0 / 0 |
| ellipse | 157 / 8 / 9 | 0 / 0 / 0 |
| line | 22 / 1 / 3 | 0 / 0 / 0 |
| arc | 38 / 4 / 9 | 0 / 0 / 0 |
| rect (generator) | 1 / 0 / 2 | 0 / 0 / 0 |
| polygon (control) | 0 / 0 / 0 | 0 / 0 / 0 |
| strokehalf | 23 / 0 / 2 | 0 / 0 / 0 |
| polyline | 83 / 2 / 6 | 0 / 0 / 0 |
| group | 32 / 4 / 4 | 0 / 0 / 0 |
| mixed (`stroked_shapes`) | 104 / 5 / 12 | 0 / 0 / 0 |
| rectstep | 3,623 / 3,623 / 12 | 0 / 0 / 0 |
| clipstep | 4,684 / 4,684 / 12 | 0 / 0 / 0 |

The ellipse row is the one the doc flagged as a risk: a double-float bisection can still
take a different branch than the oracle's f64 one, but only where a step's test value
lies within ~1e-12 of zero, and there the two final points differ by ~1e-10 px. None of
the model's 15,014 double-float ellipse pixels diverged.

**A bug found on the way: the draw list and the emitters split a blended item
differently.** `__canvas_blockInstances` counts a blended item that fills and strokes as
two records on the header's f64 `half > 0`; both emitters split on the half-width's 16.16
word `> 0`. A stroke thinner than 2^-16 px narrows to word 0, so the draw list counted two
records and the emitter published one: every later draw named the wrong block (a
circle drawn after it came out under the Multiply pipeline), and the oracle's hairline
was dropped. Pre-existing at `0cd2db799` on both backends (row `hairline` above). It is
the stroke half-width's narrowing, so it is fixed here: the split tests word and
residual, and `emit_item_stroke_sign` gives a positive half-width under 2^-48 px a
residual of 1, so "word or residual > 0" is exactly `half > 0`.

**Not a bug of this change: an intermittent lavapipe crash on 2230.** The base run on 2230
had four GPU runs exit `0xC0000005` inside `vulkan_lvp.dll` at one offset
(`0x1feb331`; `Get-WinEvent ... 'Application Error'`), all within eight minutes; two of the
rows were re-run at once and drew (RED, as the table says).
Measured further (2026-09-26, `mfb` at `0cd2db799`, box 2230 alone): a one-rectangle scene
built for `windows-x86_64` and run 40 times with `MFB_CANVAS_GPU=1` exited `0xC0000005` 6
times, and `small_polygons_on_black` 1 time in 20. With Windows Error Reporting LocalDumps
enabled (`.ai/remote_systems.md`), all seven minidumps show the same fault: a read at a
page boundary (`0x…4000`, `0x…d0000`, …) at `vulkan_lvp.dll+0x1feb331`, on a lavapipe
worker thread whose stack holds only `vulkan_lvp.dll` frames (LLVM-range offsets), while
the MFB graphics thread waits inside one Vulkan call (`crashloop.exe+0x457523`). No MFB
frame and no JIT'd shader code is on the faulting thread. The site is fixed, the scene
does not matter, and whether it faults changes run to run, which is a driver over-read
past an allocation that sometimes ends at a page end: Mesa 26.2 lavapipe
(`C:\mfbvk\mesa\x64`, dated 2026-08-11) under the box's x86-64 emulation. It is not
caused by this bug's change (it reproduces at `0cd2db799` with a rectangle) and is not
an MFB defect; every row that crashed drew and passed on re-run. Rows on 2230 can
therefore fail spuriously with `gpu rc=-1073741819`, one run in ~7; retrying in the
harness would change the test instrument and is left to the owner. The dump
configuration was removed from the box afterwards.

### Phase 2: the fix, kind by kind

- [x] Layout for residuals and the group offset; the double-float SDFs; SPIR-V; cost
      measured as bug-687 did.

#### Phase 2 results (2026-09-26)

**What changed.**

- **Layout.** `ITEM_BLOCK_SIZE` 208 → 272: four residual `ivec4`s (`ITEM_OFFSET_SHAPE_LO`,
  `_CAPS_LO`, `_CLIP_LO`, `_MISC_LO`) carry `round((v·65536 − word)·2^31)` for every field
  a distance reads, listed once in `ITEM_RESIDUAL_FIELDS` and emitted by the shared
  `emit_item_residual_field`, so the two backends cannot disagree. `arc.x` is written 0
  (a glyph or picture then puts its height there) and `arc.y` is an arc's `reflex` flag,
  decided in f64 (`emit_item_arc_flags`). The six things "Growing `ITEM_BLOCK_SIZE`"
  lists moved: both `ItemBlock`s (reflection on 2226: `topLevelArrayStride 272` in both
  stages, members at 0..256), Metal's frame (its slots are now `OFF_TEXTURE + n` with
  `OFF_TEXTURE = OFF_ITEM + ITEM_BLOCK_SIZE`), the region bases (generated), and the
  SPIR-V.
- **Group offset.** `__canvas_pushOneDraw` writes the offsets' residual words as draw
  entry words 5 and 6. Metal binds 16 bytes (`int4 drawOffset`) at both stages' indices,
  Vulkan pushes a 16-byte `ivec4 offset`; the fragment stage subtracts word plus residual
  in double-float. A frame with no group (`canvas::sceneDrawsFlat`) has offset 0 and
  residual 0, unchanged.
- **Shaders.** One path for every kind but the glyph: `shapeCoverage` (bug-687's
  `polygonCoverage`, generalised) takes one sample per pixel, five under a transform, each
  from `sampleDistance`: the polygon's edge walk (unchanged), a PolyLine's two-pass walk
  (`polylineDistance`, bug-687's passes over the band, with endpoint residuals), or an
  fp32 pre-pass (`farBound`) and then `dfShapeDistance`, the oracle's `__canvas_geoDistance`
  in double-float for the rectangle, picture, circle, line (round or butt), arc and
  ellipse. The clip is `clipCoverage` in double-float behind the same kind of pre-pass. The
  stroke strokes on word plus residual. The fp32 `geoDistance`, `shapeDistanceAndScale`,
  `ellipseDistance`, `arcInSweep` and the unused `premultiplied` are gone from both
  shaders; no GPU trigonometry is left.
- **Transformed shortcut.** Measured first without it: 6 rotated ellipses went from 3.0
  to 7.1 ms a frame, every quad pixel taking five 24-step double-float bisections. The
  centre sample of a transformed item now also skips when past
  `half + 0.5·max(F·(1 + 2^-8) + 2^-8, 1)`, `F` the inverse matrix's Frobenius norm, which
  bounds the central-difference gradient of a 1-Lipschitz distance; never for an arc,
  whose out-of-sweep 1e6 is not Lipschitz.
- **The split** (the bug above), in both emitters.

**Why the pre-passes are exact, not a tolerance.** Each returns a value `v` that, when
`|v| > reach`, has the distance's sign and at most its magnitude, so the coverage is
decided by the sign. Rectangle, circle and line use `edgeSlack`'s bound on their fp32
distance; the arc the ring and cap distances with the sweep ignored (the oracle answers
1e6 out of sweep, never less); the ellipse `rmin·|s − 1|` with `s` the point's scale
relative to the ellipse, a true lower bound on the distance to it (map by
`diag(1/rx, 1/ry)`: no vector stretches by more than `1/rmin`), plus 2^-14 of the
magnitudes for the 16.16 inputs. **Proved on the device:** every scene was rendered on
Metal with every pre-pass and shortcut disabled (reach 1e30, clip early-outs off) and with
the final build, then `cmp`'d. **Byte-identical for all 23**: the thirteen bug-735 rows,
`rectangles_on_black`, `small_polygons_on_black`, `star_301_97_on_black`,
`large_polygons`, `large_polygons_on_black`, `huge_polygon`, `many_gradients`, `layered`,
`five_thousand_quads`, and two cost scenes (large shapes; transformed ellipses).

**Cost.** `/tmp/b735/cost/cost.sh` (bug-687's method: each scene presented 200 times with a
moving 2-px rectangle, `MFB_CANVAS_SYNC=1`, `--debug`, `phaseRenderMs / frames`, which
covers the GPU draw plus readback and encode; minimum of 3 alternating runs, ms/frame).
**The machine was shared with five other lanes' builds and tests**, and the control moved
between runs, so the two runs are both given:

| scene (Metal) | run A before → after | run B before → after |
| --- | --- | --- |
| one rectangle (control) | 2.31 → 2.33 | 3.60 → 3.35 |
| circles | 2.71 → 2.47 | 3.73 → 3.62 |
| ellipses | 2.52 → 4.08 | 4.17 → 4.79 |
| arcs | 2.33 → 2.92 | 4.55 → 4.54 |
| polylines | 2.68 → 3.75 | 4.55 → 4.88 |
| stroked shapes | 2.44 → 3.37 | 4.36 → 3.60 |
| rectangles | 2.33 → 2.84 | 4.89 → 3.44 |
| 5,000 quads | 2.92 → 3.35 | 3.70 → 5.04 |
| {301/97} star | 6.84 → 7.63 | 8.04 → 8.21 |
| 64,000-edge polygon | 19.93 → 23.64 | 21.68 → 23.41 |
| 4 large shapes | 2.96 → 2.88 | 4.15 → 4.28 |
| 6 transformed ellipses | 3.02 → 7.13 (no shortcut) | 4.76 → 5.32 (shortcut) |

Run A's "after" is the build before the transformed shortcut; run B's is the final build.
The large polygon's +8-19% is not its algorithm, which is unchanged: the fragment
function is one shader for every kind, so the added double-float code costs registers
(occupancy) on every pixel. On lavapipe the numbers would describe a CPU, not a GPU
(bug-688), and were not taken; the full 2226 row set took 6 min 42 s.

### Phase 3: validation

- [x] Every row GREEN on Metal, 2226, 2230; every existing GPU row still passes; the
      `app-mouse-surface` sentinel regenerated with proof; full suite.

#### Phase 3 results

**Real Vulkan hardware** (2026-09-26, the real-GPU Windows box: Intel UHD Graphics 630,
Intel's Vulkan 1.3.215 driver, `windows-x86_64 --app --debug` builds; each row run as
`test-canvas-gpu-rows.sh` runs it -- `MFB_WINAPP_HEADLESS=1 MFB_CANVAS_SYNC=1
MFB_CANVAS_GEO_VERIFY=1`, software then `MFB_CANVAS_GPU=1`, `scripts/rgba_compare.py`;
every GPU run `vulkanReady=TRUE gpuFrames=1 geoVerifyMismatches=0`):

| row | real GPU at `0cd2db799` | real GPU, this change |
| --- | --- | --- |
| circlesblack | FAIL worst 6 | ok worst 1 |
| ellipsesblack | FAIL worst 9 | ok worst 1 |
| linesblack | FAIL worst 3 | ok worst 1 |
| arcsblack | FAIL worst 9 | ok worst 1 |
| strokedpolysblack | ok worst 2 | ok worst 1 |
| polylinesblack | FAIL worst 6 | ok worst 1 |
| groupsblack | FAIL worst 4 | ok worst 1 |
| strokedblack | FAIL worst 12 | ok worst 1 |
| xformblack | FAIL worst 9 | ok worst 1 |
| rectstepblack | FAIL worst 12 | ok worst 0 |
| clipstepblack | FAIL worst 12 | ok worst 0 |
| hairline | FAIL worst 101 | ok worst 1 |

The fourteen pre-existing render rows (quads through rectsblack) all pass on the real GPU
with this change, worst 0-1. A real GPU's fp32 flips exactly as Metal's and lavapipe's
did; the double-float path removes them there too.

**Vulkan lavapipe.** 2226: every row of `scripts/test-canvas-gpu-rows.sh --target
linux-aarch64` passes with the final build (exit 0, 29 checks, the race row 8 frames).
2230: every row passes (render rows worst 0-1; `smallpolysblack`, `matrix`, `xformblack`
passed on a re-run after the first run hit bug-741's driver fault, which also reproduces
at `0cd2db799` with a one-rectangle scene and never on the real Intel driver).

**Metal:** `cargo test --release --test rt_canvas_metal`, 38 passed, on the tip merged with
`main` (`33d498964`).

<<SUITE>>

Commit: <<COMMITS>>
