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

## Fix, round 2 (coordinator review, bug-staging-24)

Finding: two half-planes through the centre also meet along the OPPOSITE ray when the
sweep is near 0, so a zero-sweep butt arc (r 100, stroke 20, at 0.5) lit 21 px at its
start and 21 behind its centre, and a 0.002 rad one 17 behind it (round-1 build; main 0).

- **The wedge is the exact distance to the cone of the two end radii** (oracle arc arm,
  both shaders line for line): a point is inside a sweep up to a half-turn when
  `startPlane <= 0`, `endPlane <= 0` AND it is ahead of the centre along one radius
  (`d·u_s >= 0` or `d·u_e >= 0`); inside, `max(startPlane, endPlane)`; outside, the
  distance to the nearer end RAY. A reflex sweep is the complement of its gap's cone,
  built the same way. Same zero set as the review's "intersection AND the half-plane in
  front of the centre along the bisector", but NOT the review's `-d·m` term: measured,
  `-d·(s + e)/2` flips sign with the sweep vectors' rounding at a half-turn
  (`|s + e| ~ 1e-7`; a normalised `m` is noise there), and a 0..π butt semicircle came out
  at half coverage, 6,300 px off main. The exact cone distance is 1-Lipschitz and needs
  no bisector.
- **A whole turn or more is the ring alone.** `__canvas_arcTurn` (oracle) and
  `emit_item_arc_flags` (both emitters, `arc.y`) answer 2 for `endAngle − startAngle >=
  2π`, 1 past π, else 0. Round 1 had a REGRESSION here that this fixes: two equal sweep
  vectors cannot tell "everything" from "nothing", and a 2π butt arc had a half-covered
  seam along its start radius (32 ring pixels short; main and round 2: 0).
- **A butt arc with `endAngle <= startAngle` draws nothing** (empty header in
  `__canvas_arcHeader`), as a zero-length butt line does (spec: "A zero-sweep arc"). Round
  and Square draw their two ends (a zero-sweep Round arc lights 349 px, as main).
- Spec (`cap`, the zero-sweep paragraph) rewritten; SPIR-V regenerated on 2226.
- Commit `0df67cc15`.

**Contract changes beyond the bug doc, for the coordinator to rule on** (both are
consequences of the cone, and main's behaviour in both is itself wrong):
- `endAngle < startAngle`, butt: main drew the cone OPPOSITE the sweep (a 0.5 rad wedge
  behind the centre, 1,057 px for 0.5 → 0.0); round 2 draws nothing (Round/Square: the two
  ends).
- sweep > 2π: main drew the union of the two half-planes of the wrapped vectors, which
  is discontinuous at 2π (the whole ring at exactly 2π, about half of it just past); round
  2 draws the whole ring. `stroked_shapes_on_black` and `transformed_shapes_on_black` (whose
  arcs sweep `3.8 + 0.01·fx`, past 2π for most `fx`) move: 1,255 px of the first differ from
  main, every one on those round-capped arcs; both still match the oracle on every GPU.

RED → GREEN (`cargo test --release --test rt_canvas_rasteriser butt_arc`, 4 passed):
- `a_zero_or_tiny_sweep_butt_arc_draws_nothing_behind_its_centre`: the same scene with the
  round-1 compiler (`/tmp/b1085r2/bin/mfb`) lit zero 21 front / 21 behind, tiny 24 / 17;
  round 2: zero 0 / 0, tiny 24 / 0. The coordinator's `arc0` probe and `count.py`, round 2,
  software and Metal: zero 0 / 0, tiny 24 / 0.
- `a_butt_arc_swept_a_whole_turn_is_the_whole_ring`: round 1 32 ring pixels short, now 0.
- `a_butt_arc_whose_end_is_before_its_start_draws_nothing`: main 1,057 px, now 0.

Checks: `bash scripts/test-fast.sh --test rt_canvas_rasteriser --test rt_canvas_metal
--test rt_canvas_golden --test rt_canvas_hit --test rt_canvas_curves` (logs
`/var/folders/2n/9d_8f0dd7tj6bwwx3tmxrgn80000gn/T/mfb-test-fast.3ygve6`): fmt, check, build,
skips ok; tests FAILED only on the two `endcaps` reference tests (rasteriser 111 passed,
metal 74, hit 14, curves 40). `cargo test --release --bin mfb -- vulkan:: metal:: canvas::`
99 passed. Vulkan: linux-aarch64 (2226) `magarcs,arcsblack,dashes,squarecaps,
strokedblack,xformblack,joins,groupaffine,curves,polylinesblack` all match;
windows-x86_64 (206) `magarcs,arcsblack,dashes,squarecaps,strokedblack,xformblack,
groupaffine` all match (a copy of the compiler from before the `endAngle <= startAngle`
header rule; no row scene has such an arc, and the shaders are the same). spec-census
`--citations/--render/--links app`: 0 / 0 / 0. `scripts/artifact-gate.sh <mfb> all`: the
same 10 diffs as round 1.

Goldens expected to change: as round 1 -- `tests/golden/canvas/endcaps.png` (the same 28
pixels, first at (206, 372)), `app-mouse-surface` ir/nir/nplan and four ncode,
`app-window-surface` macos ncode, `resources/canvas-append-unconsumed-valid` and
`resources/loop-body-resource-reference-valid` `.ir`. No other pixel reference moves
(`rt_canvas_golden` 26 passed).

## Owner rulings (2026-10-17, bug-staging-24 review)

- A butt arc whose `endAngle` is before (or equal to) its `startAngle` draws nothing, as a
  zero-length butt line does (option (a); the HTML-canvas wrap-around was not chosen).
- A sweep of a whole turn or more draws the whole ring.
