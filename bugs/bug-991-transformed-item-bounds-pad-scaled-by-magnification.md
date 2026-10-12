# bug-991: A magnified transformed item's bounds pad scales with the transform, so a 27 px polygon covers 369 px

Last updated: 2026-10-12
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Other (Performance)

Status: Open
Regression Test: planned `tests/canvas/rt_canvas_rasteriser.rs` (transformed-bounds case) + `tools/canvas-bench/bench.sh <mfb> magnified-polygons` (new scene)

## Cause (coordinator, 2026-10-12, bug-staging-23)

- Rule: `mfb spec app canvas` "Rendering conventions": coverage is `clamp(0.5 - d, 0, 1)` with `d` measured AFTER the transform, so the antialiasing rim is one SURFACE pixel. Every header's box (slots 16-19) is `extent +- reach +- 1.0`, with that 1.0 in the space the box is written in.
- Place: the two functions that carry a box into another space scale the rim with it: `helper_geometry.rs:__canvas_boundsHeader` (the item's own transform; it then adds a second pixel) and `helper_items.rs:__canvas_drawBound` (a group matrix), plus the GPU vertex stages that do the group mapping (`.ai/canvas-threading.md` 10).
- Fix: whoever maps a box takes the 1.0 rim off first, maps `extent +- reach`, and adds one pixel in the destination space. That is one rule at the mapping sites; the nine header builders are not touched, an untransformed ungrouped header stays bit-identical (so does `canvas::geoBuild`), and no per-builder "divide the pad by the scale" exists.
- Audit, not assume: `__canvas_glyphRunHeader` (`+- advance`, `+- 2.0`) and `__canvas_pictureHeader` (`+- 2.0`), whose pads are not a plain 1.0 rim, and the plan-164 `PolyCurve`/`CurvePolygon` headers (`helper_curves.rs`).

When a canvas item is drawn in a small "unit" coordinate space and placed on the surface
by a large-magnification `Paint.transform`, every renderer rasterises a much larger
screen rectangle than the item covers. On macOS Metal, 49 small 24-point polygons
(~27 px across on screen) with radius 0.08 under scale 170 render at 31–47 ms a frame
(21–29 fps). The same on-screen polygons with pre-transformed pixel coordinates render
in about 2–8 ms. The software renderer is slower still: the same polygon scene takes 17.5 s a frame
against ~30 ms untransformed, and a transformed circle scene takes 4.5 s against 7 ms. The pictures are identical; only the cost
differs. `examples/canvas-demo` hit this: the cathedral rose window draws 49 unit-space
panes scaled by the window radius, plus the same panes again as floor pools, and it ran
at 14 fps.

The cause is that the "+1" antialiasing margin every header builder adds to its bounds
is added in **shape units**, *before* `__canvas_boundsHeader` forward-maps the box.
Under scale S that margin is S surface pixels on each side. At S = 170 a 0.16-unit
polygon's quad is `(0.16 + 2) · 170 + 2 ≈ 369` px square, when it should be about 31 px.

**The single correct behavior a fix produces:** a transformed item's header bounds
(slots 16–19, the software loop's box and the GPU quad) are the forward-mapped hull of
the shape plus its *transformed* stroke/cap/join reach, with the antialiasing margin
added once **in surface pixels**. So the rasterised area of an item does not grow with
magnification beyond the item's own on-screen extent plus a fixed pixel margin. Every
rendered pixel stays byte-identical, because the pixels this removes all have coverage 0.

References:

- `src/docs/spec/app/06_canvas.md` "Rendering conventions": "`Paint.transform` maps the
  item's coordinates onto the surface"; coverage is `clamp(0.5 - d, 0, 1)` with `d`
  measured **after** the transform. So the ramp is ≤ 0.5 *surface* px wide whatever the
  scale, and a margin in shape units has no basis.
- `.ai/canvas-threading.md` §10 (renderer branch, item blocks, band index) and §14 (the
  geometry cache).
- `src/target/macos_aarch64/app/metal.rs` doc comment above `emit_glyph_publish` (the
  "quad is the header's bounds" note). It says the quad carries "the `strokeHalf + 1`
  pad the software rasteriser gives itself so its coverage ramp has pixels to run over".
  That holds in surface pixels only when there is no transform.
- `bugs/bug-765-canvas-100k-color-changing-dots-miss-30-fps.md`: the sibling canvas
  performance bug (item count, not item area).
- Found while building `examples/canvas-demo` (`examples/canvas-demo/src/cathedral.mfb`:
  `winT` = `Transform[a := rose.r, d := rose.r, …]` over unit-space `rosePanes`, and
  `onFloor` for the pools).

## Re-verified 2026-10-12: still reproduces, and a scaled `Group` costs the same (HEAD `555130a25`)

**The API this document names has moved.** Since plan-172-C the transform is the item's own
`transform` field, not `Paint.transform`, and the program below no longer compiles. The same
scene in today's syntax is `bugs/repro/bug-991/magnified.mfb` (arguments: mode, frame
count). `mfb build --app`, 240 frames in a real window on Metal; software is
`MFB_CANVAS_GPU=0 MFB_MACAPP_HEADLESS=1`, 3 frames for the slow rows and 24 for the fast:

| Mode | Metal frameMs | Software frameMs |
| --- | --- | --- |
| `flat` (radius 0.08, item transform ×170) | 35.4, 37.2 ✗ | 17,543 ✗ |
| `group` (same items untransformed, inside a `canvas::Group` whose transform is ×170) | 35.3 ✗ | 13,793 ✗ |
| `x10` | 8.8 | not run |
| `x100` | 7.9 | not run |
| `pix` | 5.0, 6.1 ✓ | 25.1 ✓ |
| `circle-xf` | 7.8 | 3,981 ✗ |
| `circle-pix` | 2.0 ✓ | 5.4 ✓ |

Machine load average was 9.6 (`uptime`) during the Metal rows.

- `__canvas_strokePad` still ends in `+ 1.0`, `__canvas_boundsHeader` still adds its own
  `± 1.0` after mapping, and `func_geo_build.rs` still takes only an item whose transform
  is all zero. The cause below stands as written.
- **`group` is new.** Group matrices are plan-172-G/H, after this bug was filed. The row
  above is a measurement of the cost only. The likely cause, by reading and not confirmed
  by a patched build: an untransformed child's header carries the shape-unit `+ 1.0`, and
  the group's forward matrix then scales that box (`.ai/canvas-threading.md` §10, "the
  vertex stage carries the item's box through the FORWARD linear part"). Phase 1's audit
  must give the group path a verdict, and Phase 2's fix in `__canvas_boundsHeader` alone
  will not reach it, because that function only sees the item's own transform.
- Not audited: the header builders plan-164 added for `PolyCurve` and `CurvePolygon`.

## Failing Reproduction

`/tmp/cdperf/project.json`:

```json
{ "name": "cdperf", "version": "0.1.0", "mfb": "1.0", "kind": "executable", "mode": "app",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main", "targets": ["native"] }
```

`/tmp/cdperf/src/main.mfb` (gradient/blend modes trimmed; they do not change the result):

```basic
IMPORT app
IMPORT canvas
IMPORT collections
IMPORT color
IMPORT io
IMPORT math
IMPORT os

FUNC tr(a AS Float, tx AS Float, ty AS Float) AS canvas::Transform
  RETURN canvas::Transform[a := a, b := 0.0, c := 0.0, d := a, tx := tx, ty := ty]
END FUNC

' A small 24-point disc near (ux, uy) in unit space.
FUNC wedge(ux AS Float, uy AS Float) AS List OF canvas::Point
  MUT out AS List OF canvas::Point = []
  FOR i = 0 TO 23
    LET a AS Float = toFloat(i) * 6.2831853 / 24.0
    out = collections::append(out, canvas::Point[x := ux + math::cos(a) * 0.08, y := uy + math::sin(a) * 0.08])
  NEXT
  RETURN out
END FUNC

FUNC main AS Integer
  app::setMode(app::Mode.Canvas)
  app::setTitle("cdperf")
  LET mode AS String = collections::get(os::args(), 0)
  LET R AS Float = 170.0
  MUT frame AS Integer = 0
  DO WHILE frame < 360
    MUT scene AS List OF canvas::DrawItem = []
    FOR i = 0 TO 48
      LET ux AS Float = math::cos(toFloat(i) * 0.4) * (toFloat(i) / 60.0)
      LET uy AS Float = math::sin(toFloat(i) * 0.4) * (toFloat(i) / 60.0)
      LET k AS Integer = (frame + i) MOD 200   ' colour changes every frame, so frames draw
      LET p AS canvas::Paint = WITH canvas::fill(color::rgba(200, 80, 80, 55 + k)) { transform := tr(R, 450.0, 300.0) }
      MUT d AS canvas::DrawItem = canvas::Polygon[contours := [wedge(ux, uy)], rule := canvas::FillRule.NonZero, paint := p]
      IF mode = "x10" THEN d = canvas::Polygon[contours := [collections::transform(wedge(ux, uy), LAMBDA(q AS canvas::Point) -> canvas::Point[x := q.x * 10.0, y := q.y * 10.0])], rule := canvas::FillRule.NonZero, paint := WITH p { transform := tr(R / 10.0, 450.0, 300.0) }]
      IF mode = "x100" THEN d = canvas::Polygon[contours := [collections::transform(wedge(ux, uy), LAMBDA(q AS canvas::Point) -> canvas::Point[x := q.x * 100.0, y := q.y * 100.0])], rule := canvas::FillRule.NonZero, paint := WITH p { transform := tr(R / 100.0, 450.0, 300.0) }]
      IF mode = "pix" THEN d = canvas::Polygon[contours := [collections::transform(wedge(ux, uy), LAMBDA(q AS canvas::Point) -> canvas::Point[x := 450.0 + q.x * R, y := 300.0 + q.y * R])], rule := canvas::FillRule.NonZero, paint := canvas::fill(color::rgba(200, 80, 80, 55 + k))]
      IF mode = "circle-pix" THEN d = canvas::Circle[x := 450.0 + ux * R, y := 300.0 + uy * R, radius := 0.08 * R, paint := canvas::fill(color::rgba(200, 80, 80, 55 + k))]
      IF mode = "circle-xf" THEN d = canvas::Circle[x := ux, y := uy, radius := 0.08, paint := p]
      scene = collections::append(scene, d)
    NEXT
    canvas::present(scene)
    canvas::waitFrame()
    IF frame MOD 120 = 119 THEN
      LET s AS canvas::FrameStats = canvas::getFrameStats()
      io::printError(mode & " fps=" & toString(s.fps) & " frameMs=" & toString(s.frameSeconds * 1000.0))
    END IF
    frame = frame + 1
  LOOP
  RETURN 0
END FUNC
```

```
target/release/mfb build /tmp/cdperf
/tmp/cdperf/build/cdperf.app/Contents/MacOS/cdperf <mode>                  # GPU (real window)
MFB_CANVAS_GPU=0 /tmp/cdperf/build/cdperf.app/Contents/MacOS/cdperf <mode> # software
```

All five polygon modes draw the same 49 discs, ~27 px across, at the same place on the
surface. Only the coordinate space and the transform differ.

- Observed (Metal, M-series, `flat` = the default mode): `flat fps=25.00 frameMs=31.27`,
  `fps=21.00 frameMs=46.97`, `fps=29.00 frameMs=33.53` (this session's run of the
  command above; the finder's runs gave 39–50 ms).
- Expected: within noise of `pix` (≈ 2–8 ms on the finder's runs).

Observed matrix. Metal rows are from the finder's runs, except `flat`, which this
session re-ran. Software rows are this session's runs of a 24-frame copy
(`/tmp/cdperf-sw`, same source with `frame < 24` and a stats line every 8 frames):

| Mode | Geometry | Transform scale | Metal frameMs | Software frameMs |
| --- | --- | --- | --- | --- |
| `flat` | radius 0.08 | 170 | 31–50 ✗ | 17,533 (436 s wall for 24 frames) ✗ |
| `x10` | radius 0.8 | 17 | ≈ 8.5 | 553–578 ✗ |
| `x100` | radius 8 | 1.7 | ≈ 8.4 | not run |
| `pix` | pixels | none | ≈ 2–8 ✓ | 28–36 ✓ |
| `circle-pix` | pixels | none | not slow ✓ | 6.7–7.5 ✓ |
| `circle-xf` | radius 0.08 | 170 | not slow (see Root Cause) | 4,101–4,469 ✗ |

Software `flat` (`MFB_CANVAS_GPU=0 … cdperf flat`, 24 frames) printed
`flat fps=1.00 frameMs=17533.40` and took 436 s wall, about 600× software `pix`.

Gradient fills (Radial, focal, `Repeat`) and `BlendMode.Add` do not change the picture
much (finder's runs). The magnification is what decides it.

## Root Cause

**Confirmed by code reading, and consistent with every row of the matrix; not yet
confirmed by a patched build.**

Every MFBASIC header builder pads its shape-space bounds by a reach that ends in a
literal `+ 1.0`, meant as one pixel of antialiasing room:

- `src/codegen/builtins/canvas/helper_joins.rs:__canvas_strokePad`: `reach + 1.0` (or
  `half * miterLimit + 1.0`). Used by `__canvas_polygonHeader`, `__canvas_rectHeader`
  and `__canvas_textHeader`.
- `src/codegen/builtins/canvas/helper_geometry.rs:__canvas_circleHeader`
  (`radius + dashCapReach + 1.0`), `__canvas_ellipseHeader`, `__canvas_segmentHeader`,
  `__canvas_arcHeader` and `__canvas_polylineHeader`, each `… + 1.0`.

These pass the padded box to
`src/codegen/builtins/canvas/helper_geometry.rs:__canvas_boundsHeader`. When the item has
a transform (header slot 33), that function forward-maps the four corners and then adds
**another** `±1.0` *surface* pixel to the hull. So under a forward scale S the
shape-space `+1.0` becomes S surface pixels on every side, and the real one-pixel margin
is added again on top. `__canvas_clipHeader` already does it right: it maps first and
then widens by a pixel.

`__canvas_boundsHeader` builds a transformed item's header. The native builder
`src/codegen/builtins/canvas/func_geo_build.rs` (module doc) declines any non-identity
transform and hands it to these MFBASIC builders. So all three renderers read the
inflated slots 16–19:

- **Software**: `src/codegen/builtins/canvas/helper_items.rs:__canvas_drawGeometry` loops
  `firstX..lastX × y..lastY` over slots 16–19. A transformed item takes three distance
  evaluations per pixel (centre plus central differences), with no early-out by
  distance.
- **Metal**: the quad is the header's bounds (`src/target/macos_aarch64/app/metal.rs`,
  doc above `emit_glyph_publish`; `MfbItem.quad`). Every pixel of the quad runs
  `shapeCoverage`.
- **Vulkan**: the same header and the same quad (not measured; no reachable hardware GPU).

Arithmetic from the code (computed, not measured). The repro's polygon is 0.16 units
across. Its quad under scale 170 is `(0.16 + 2·1.0)·170 + 2 ≈ 369` px square, about
136,000 px. The `pix` polygon's quad is `0.16·170 + 2·1 ≈ 29` px square, about 850 px.
That is **≈ 160× the pixels per item**, and with 49 items clustered within ~140 px of the
centre almost every quad overlaps almost every other. The software ratios fit:

- `flat` / `pix` = 17,533 / 30 ≈ 580×, and `circle-xf` / `circle-pix` = 4,469 / 7.5 ≈ 600×, against 160× area × 3 evaluations
  ≈ 480×.
- `x10` / `pix` ≈ 553 / 30 ≈ 18×, against a 63 px quad (`(0.16·10 + 2)·17 + 2`) vs
  29 px, ≈ 4.7× area × 3 ≈ 14×.

**Why a transformed circle is not slow on Metal but a transformed polygon is.** Both get
the 369 px quad. In `METAL_SHADER_BODY` `shapeCoverage`, a transformed item's
early-out radius is `reach = half + 0.5 · max(F, 1)` *shape* units (`F` = the inverse
matrix's Frobenius norm, `1.0` the fallback gradient). At scale 170 `F ≈ 0.008`, so
`reach = 0.5` units = **85 surface pixels**. Every quad pixel within 85 px of the shape
evaluates all five samples in double-float. A circle sample is one closed-form distance.
A polygon sample is `edgeDistance` over all 24 edges (24 < `BAND_MIN_EDGES` = 32, so there
is no band index) plus its double-float re-check. So the polygon's per-pixel cost is
roughly 24× the circle's across most of an inflated quad. That is what turns an
invisible over-draw into 30–50 ms.

Secondary, same root (shape-unit constants meant as pixels), latent:

1. **The `0.5 · max(F, 1)` early-out radius.** The `1` term is the fallback gradient
   `g = 1.0` that `shapeCoverage` substitutes when `g ≤ 1e-6`. It forces the
   non-skip zone to at least 0.5 shape units, which is S/2 px under magnification.
   Once the pad is fixed the quad is smaller than this zone, so every quad pixel still
   takes the five-sample path. That path is the transformed rule's inherent cost, but
   the bound is looser than it needs to be. *To confirm it matters:* after the Phase 2
   pad fix, re-time `flat` against `x100` (`F ≈ 0.83`, so `max(F,1) = 1` there too).
   If `flat` is still clearly slower than `pix`, measure a variant with the skip radius
   computed from `F` alone, and check byte-identity with the bug-735 oracle rows.
   Software is the oracle, so any change must keep frames byte-identical.
2. **The band index reach** (`src/target/macos_aarch64/app/metal.rs:emit_band_index`,
   `src/codegen/runtime/canvas/vulkan.rs:emit_band_index`): under a transform
   `r = half + 0.5 + 1 (guard) + 2F` in shape units. At scale 170 that is ≥ 1.5 units
   = 255 px, so a ≥ 32-edge magnified polygon smaller than that has every edge in every
   band and the index does nothing. It is not reached by this repro (24 edges). *To
   confirm:* repeat the repro with 64-point wedges and compare `flat` vs `pix`. It is
   correct, only slow, so it is out of scope unless measured.

## Goal

- A transformed item's header bounds are the forward-mapped hull of
  `shape ± (stroke/cap/join reach in shape units)`, widened by a fixed surface-pixel
  margin. There is no shape-unit `+1.0` under a transform.
- On the repro, Metal `flat` frameMs is within noise of `x100` (≈ 8.4 ms on the finder's
  machine), and software `flat` is within 3× of software `pix` (the three-evaluation
  transformed rule).
- `examples/canvas-demo` cathedral scene: measured fps recorded before and after.

### Non-goals (must NOT change)

- Every rendered pixel, on every backend. The removed pixels all have coverage 0. The
  software goldens are exact and must not move. If one does, that is a bug-hunt trigger,
  not a re-baseline.
- The untransformed path. Slots 16–19 for an item with no transform must stay
  bit-identical, and so must the native `canvas::geoBuild` records (the
  `MFB_CANVAS_GEO_VERIFY` byte-identity contract).
- The spec's coverage rule, transform rule and caps.
- The tempting wrong fix: "divide the `+1.0` by the scale" inside each builder. It is
  wrong under non-uniform scale or shear, it is repeated in nine places, and it still
  double-counts `__canvas_boundsHeader`'s own `±1.0`. Make the margin a surface-space
  concern in one place instead.

## Blast Radius

Searched with `grep -n "+ 1.0\|strokePad\|boundsHeader" src/codegen/builtins/canvas/helper_geometry.rs src/codegen/builtins/canvas/helper_joins.rs`
and `grep -n "boundsHeader\|Transform" src/codegen/builtins/canvas/func_geo_build.rs`:

- `helper_geometry.rs:__canvas_polygonHeader` (via `__canvas_strokePad`): observed,
  fixed by this bug.
- `helper_geometry.rs:__canvas_circleHeader`: observed in software (`circle-xf`), fixed
  by this bug.
- `__canvas_rectHeader`, `__canvas_ellipseHeader`, `__canvas_segmentHeader`,
  `__canvas_arcHeader`, `__canvas_polylineHeader`, `__canvas_textHeader` (strokePad):
  same pattern by code reading, not measured. Fixed by the same change.
- `__canvas_glyphRunHeader` (`± advance`, `± 2.0` in run units): the same shape-unit
  margin. Whether a transformed text run's per-glyph quad uses it needs checking
  during Phase 1.
- `__canvas_pictureHeader`: built on the rectangle's header, so it inherits the rect's
  verdict.
- `__canvas_clipHeader`: unaffected. It maps first and then widens by one surface pixel,
  which is the correct pattern.
- `func_geo_build.rs` (native builder): unaffected. It handles identity transforms only,
  and its `stroke_pad`/`cap_pad` `+1.0` is a surface pixel there.
- Damage mode (`helper_damage.rs`) unions item bounds. Smaller transformed bounds shrink
  damage rects too, which is correct and is the intended shift.
- Band reach and the shader early-out radius: secondary, see Root Cause.

## Fix Design

Move the antialiasing margin out of the shape-space pad when the item is transformed.
The simplest shape that keeps the untransformed records bit-identical:

1. Every builder computes `reach` (stroke/cap/join, shape units) and the `+1.0` margin
   separately, and passes both to `__canvas_boundsHeader` (or passes the shape-space
   box plus the margin).
2. `__canvas_boundsHeader`, untransformed, writes `box ± (reach + 1.0)` exactly as today,
   so the same operations happen in the same order and the bits are unchanged.
   Transformed, it forward-maps `box ± reach` and adds its existing `±1.0` surface pixel.
   That surface pixel is already the only margin a *minified* item (S < 1) relies on
   today, so it is sufficient by the existing argument.

Correctness risk: a builder whose reach was hidden inside the `+1.0` would cut a stroke
edge. Each builder's reach must cover the full stroke/cap/join extent in shape units. The
stroke scales with the shape (spec: "A stroke is transformed with the shape it
outlines"), so mapping `box ± reach` is exact. The Phase 1 test must include a stroked,
miter-joined, square-capped transformed polygon and a rotated/sheared one.

Rejected alternatives: per-builder division by the scale (above); computing the bounds
on the GPU from the edges (changes the published record shape for nothing).

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add a headless test that renders a scene of transformed items under scale ≥ 50
      (fill polygon, stroked miter polygon, square-capped line, circle, rotated
      rectangle, sheared polygon, text run). Assert two things: the header bounds
      width ≤ forward-mapped shape extent + 2·reach·S + 2 px (via whatever debug hook
      exposes slots 16–19; add one under `--debug` if none does), and that the frame is
      byte-identical to the same scene in pre-transformed coordinates where the
      coordinates are exactly representable. The first assertion must fail today.
- [ ] Add a `magnified-polygons` scene to `tools/canvas-bench` (the repro above), so the
      before/after numbers come from the existing instrument.
- [ ] Settle `__canvas_glyphRunHeader` and `__canvas_pictureHeader` in the audit.

Acceptance: the bounds assertion fails for the documented reason; the audit has a verdict
per site.
Commit: —

### Phase 2 — the fix

- [ ] Split margin from reach in every builder listed in Blast Radius, and make
      `__canvas_boundsHeader` apply the margin in surface space under a transform.
- [ ] Correct the `emit_glyph_publish`-area doc comment in `metal.rs` (the `+1` pad is
      a surface pixel) and the `__canvas_boundsHeader` comment.

Acceptance: the Phase 1 test passes; `flat` and `circle-xf` timings meet Goal; no golden
moves.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] Run the full suite. No golden or acceptance artifact is expected to move. Any that
      does must be localised on one fixture before anything else (AGENTS.md).
- [ ] `MFB_CANVAS_GEO_VERIFY=1` `--debug` run of `tests/canvas/rt_canvas_geo_native.rs`:
      0 mismatches (untransformed records unchanged).
- [ ] Re-run the repro on Metal and software, and the cathedral scene in
      `examples/canvas-demo`; record the numbers here.
- [ ] Re-measure Root Cause secondary 1 and decide it.

Acceptance: full suite green with no expected-output delta; the reproduction meets Goal.
Commit: —

## Validation Plan

- Regression test(s): the Phase 1 bounds/byte-identity test in `tests/canvas/`; the
  `magnified-polygons` canvas-bench scene.
- Runtime proof: `/tmp/cdperf` `flat` vs `x100` vs `pix` on Metal in a real window, and
  `MFB_CANVAS_GPU=0` software; the cathedral rose window's fps.
- Doc sync: `src/docs/spec/app/06_canvas.md` "Rendering conventions" / `Paint.transform`
  if it states where the bounds margin lives (today it does not; add one sentence: the
  margin is one surface pixel, the reach is transformed with the shape).
  `.ai/canvas-threading.md` needs no change.
- Full suite: the project's full `cargo test` run (never one module, per AGENTS.md).

## Open Decisions

- Fix the secondary early-out radius (`0.5·max(F, 1)`) in the same bug, or leave it for
  measurement after Phase 2? Recommended: measure after Phase 2, and fix here only if
  `flat` still misses Goal.
- Band reach under magnification: leave it latent (recommended), or file it separately
  if a ≥ 32-edge magnified polygon is measured slow.

## Summary

The bug is one misplaced constant repeated across the header builders: the antialiasing
margin is added in shape units, so magnification multiplies it. The fix itself is small.
The engineering risk is the audit: every builder's real stroke/cap/join reach must
survive having the `+1.0` cushion removed under a transform, and the untransformed and
native records must stay bit-identical. Pixels, goldens, the spec's coverage rule and the
GPU caps are left untouched.
