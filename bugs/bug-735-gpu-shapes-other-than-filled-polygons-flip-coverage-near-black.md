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

## Phases

### Phase 1: failing rows and the model

- [ ] One `*_on_black.mfb` scene per kind from the generator; RED on Metal, 2226, 2230.
- [ ] Extend bug-687's C model to each kind (V1 today vs V5 extended) before touching a
      shader; stop if a kind's model does not reach `GPU_DEFAULT`.

### Phase 2: the fix, kind by kind

- [ ] Layout for residuals and the group offset; the double-float SDFs; SPIR-V; cost
      measured as bug-687 did.

### Phase 3: validation

- [ ] Every row GREEN on Metal, 2226, 2230; every existing GPU row still passes; the
      `app-mouse-surface` sentinel regenerated with proof; full suite.

Commit: —
