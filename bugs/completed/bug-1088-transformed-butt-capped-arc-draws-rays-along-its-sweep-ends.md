# bug-1088: a butt-capped `canvas::Arc` under a magnifying transform draws dotted rays from its centre along the ends of its sweep

Last updated: 2026-10-12
Effort: unknown
Severity: MEDIUM
Class: Correctness

Status: Fixed (bs24-agent-a, bug-staging-24)
Regression Test: `tests/canvas/rt_canvas_rasteriser.rs::a_magnified_butt_arc_draws_its_band_and_antialiased_ends_only`; `rt_canvas_metal.rs::magnified_arcs_match_the_software_oracle_on_metal`; Vulkan row `magarcs`

## Cause (coordinator, bug-staging-24)

- Rule: "Each cap is defined on the signed distance the coverage rule already uses, so a cap edge is antialiased exactly as any other edge is" (`mfb spec app canvas`, `cap`); a butt LINE follows it (`max(d − half, −t·|v|, (t−1)·|v|)`, `helper_shapes.rs:__canvas_segmentDistanceButt`).
- Where: a butt ARC breaks it: `helper_draw.rs` (the `__CANVAS_GEO_ARC` arm) answers `1000000.0` for any point outside the sweep, a jump rather than a distance, and both shaders mirror it (`metal.rs`, `mfb_canvas.frag`: "out of sweep it jumps to 1e6"). Under a transform the central-difference gradient across that jump is what lights the rays.
- Fix: a butt arc's distance is the band intersected with the sweep wedge as signed half-plane distances through the two sweep-end radii (`max` of the two for a sweep ≤ 180°, `min` for a reflex one), `max`'d with `|ρ − r| − half`, exactly as the butt line takes its end planes; the same expression in the software oracle and both shaders. That makes an arc 1-Lipschitz, so the "a transformed arc takes no shortcut" exception and the 1e6 sentinel are deleted. Round and Square arcs union their end shapes with this band as they do now. Expected pixel change: a butt arc's ends become antialiased (they are hard cuts today); goldens that hold a butt arc move.

Found during `bug-staging-23` (2026-10-12) by bs23-agent-d, re-run by the coordinator with main's compiler (`7011bb534`), and placed in a later phase.

**The single correct behavior a fix produces:** the scene below draws the quarter-ring and
nothing else, on software and on the GPU.

## Reproduction

`bugs/repro/bug-1088/butt_arc_rays.mfb`: an arc of radius 0.5 under a transform that scales
by 100, so on the surface a quarter-ring of radius 50 about (300.3, 300.7).

```
mfb build --app --debug <dir>
MFB_MACAPP_HEADLESS=1 MFB_CANVAS_SYNC=1 MFB_CANVAS_GPU=0 MFB_CANVAS_DUMP=sw.rgba <binary>
lit pixels: 1190, x 161..455, y 301..456
```

A ring of radius 50 with a stroke that ends near 55 cannot light x = 161 or x = 455: those
are the rays, out to the edge of the item's box. The agent measured 110 lit pixels more than
57 px from the centre, the same on software and Metal (Metal not re-run by the coordinator).

## Cause, by reading and NOT confirmed

An arc's distance is not continuous across its sweep ends (the shader's own comment: "never
for an arc, whose distance jumps"), so under a transform the central-difference gradient is
wrong along those two lines and the coverage `d / g` lights pixels far from the stroke.
bug-991's fix has to leave an arc's box alone for this reason.

## Fix (bs24-agent-a, bug-staging-24)

The Cause note's design, unchanged.

- **Oracle** (`helper_draw.rs`, `__canvas_geoDistance`'s arc arm): the sweep is two signed
  half-plane distances through the end radii, `startPlane = sy·dx − sx·dy` (positive before
  the start) and `endPlane = ex·dy − ey·dx` (positive past the end), combined by `max` (a
  sweep up to a half-turn) or `min` (reflex), and the arc's distance is
  `max(|ρ − r| − half, sweep)`. Round and Square still `min` their discs / boxes with it.
  The 1e6 sentinel is gone, and with it `__canvas_arcInSweep` (its only caller), whose
  rationale (no `atan2`, intersection vs union, the Y-down sign) moved to the arm.
- **Shaders** (`metal.rs` / `mfb_canvas.frag`, `dfShapeDistance` kind 3, line for line):
  the same two planes in double-float from `arcCaps − centre` (the radius times the
  oracle's Taylor `(cos, sin)`), divided by the radius. The arc's
  `reach = item.misc.x == 3 ? 1.0e30 : …` exception in `shapeCoverage` is deleted: the
  arc is 1-Lipschitz and takes the centre-sample shortcut like every kind. `farBound`'s
  arc bound is unchanged and stays a lower bound (the sweep only raises the distance).
  SPIR-V regenerated on box 2226 (`frag.spv` only).
- `mfb spec app canvas` (cap, the double-float section, the box paragraph) states the
  butt arc as a band cut by two half-planes; it no longer calls the arc non-Lipschitz.
  The arc's box is unchanged (still the square about its whole ring, rim 0); its comment
  in `helper_geometry.rs` no longer gives the old jump as its reason.

Consequences beyond the pixels (all from the one distance, by design):

- `canvas::hit` measures with the same distance (`__canvas_hitDistance`), so a thin
  arc's 3 px hit slack now reaches past its butt ends as a butt line's does, rather than
  stopping dead on the end radius. `rt_canvas_hit` (14 tests) passes unchanged.
- A dashed arc now takes the transformed centre-sample shortcut too (it was excluded by
  kind). Dashed circles, ellipses and rounded rectangles already took it; `dashes` and
  `magarcs` (a dashed magnified arc) match the oracle on Metal, lavapipe and box 206.

Commands run (`CARGO_TARGET_DIR=/tmp/bs24-agent-a`):

- RED: `cargo test --release --test rt_canvas_rasteriser a_magnified_butt_arc` failed
  before the fix: `145 pixels lit away from the arc's band ... first [(301, 301, 187), …]`.
- GREEN: `cargo test --release --test rt_canvas_rasteriser` 108 passed (the
  `an_arc_swept_to_pi_reaches_its_end_cap` row included); `rt_canvas_metal` 74 passed;
  `rt_canvas_hit` 14 passed; `cargo test --release --bin mfb -- vulkan:: metal:: canvas::`
  99 passed.
- On the scene's quarter-ring (main vs this build, software): 1190 lit pixels with 145
  more than 6 px off the ring, against 1055 lit and 0 off; Metal: 1055 and 0.
- `scripts/test-canvas-gpu-rows.sh <mfb> --target linux-aarch64 --rows
  magarcs,arcsblack,dashes,squarecaps,strokedblack,xformblack,joins,groupaffine,curves,polylinesblack`
  all match; `--target windows-x86_64 --rows
  magarcs,arcsblack,dashes,squarecaps,strokedblack,xformblack,groupaffine` all match.
- `scripts/spec-census.sh --citations/--render/--links app`: 0 missing, 0 leaks, 0
  unresolved.

Goldens expected to change (not regenerated):

- `tests/golden/canvas/endcaps.png` (`rt_canvas_golden::endcaps_match_their_reference_exactly`
  and `the_gpu_draws_the_endcap_scene_the_reference_shows` fail on it): the butt arc's END
  radius (angle 0.6π, oblique) is now antialiased. Localized by rendering the scene with
  main and with this build: exactly 28 pixels differ, every one on that radius
  (angle 1.880..1.889 rad, ρ 76..104, the arc about (230, 300)); the start radius (angle 0,
  along a pixel row, where the plane is ±0.5 at every pixel centre) and the round arc do not
  move. `smiley`, `blendmodes` (arcs ending on axis-aligned radii) do not move:
  `rt_canvas_golden` 26 passed, 2 failed (the two above).
- `artifact-gate all`: 10 diffs. `app-mouse-surface` `.ir`, `.macos-aarch64.app.nir`,
  `.app.nplan` and all four `.app.ncode`; `app-window-surface` `.macos-aarch64.app.ncode`;
  `resources/canvas-append-unconsumed-valid` and `resources/loop-body-resource-reference-valid`
  `.ir`. Cause: the canvas helper bodies (the arc arm, `__canvas_arcInSweep` deleted) and the
  shader string / SPIR-V. Localized on `canvas-append-unconsumed-valid.ir`: with
  `"line": N` normalised the remaining diff is the arc arm's locals (`startPlane`,
  `endPlane`, `sweep`, `arcD`), the deleted `#canvas_arcInSweep`, and the source-line
  constants in helper `fail` sites (`builtins/canvas.mfb` 9767 → 9770), all from the
  helper source's line count changing.
