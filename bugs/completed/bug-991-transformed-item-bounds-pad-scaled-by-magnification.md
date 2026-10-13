# bug-991: A magnified transformed item's bounds pad scales with the transform, so a 27 px polygon covers 369 px

Last updated: 2026-10-12 (bug-staging-23 round 2)
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Other (Performance)

Status: FIXED (bug-staging-23, 2026-10-12)
Regression Test: written and reverted with the fix: `a_transformed_items_box_is_its_mapped_extent_and_a_rim_of_pixels` in `tests/canvas/rt_canvas_damage.rs` (commit `4d9923d44`, reverted by `e84c14262`; restored as `094ae5056` and reverted again by `95fe00093` in round 3); `tools/canvas-bench/bench.sh <mfb> magnified-polygons`

## Resolution (bug-staging-23, 2026-10-12): FIXED, one fix for bug-991 and bug-1083

OWNER RULING 2026-10-12: "Land 991 with 1083 - approved", accepting the two pixel changes
below. The six commits bs23-agent-f built and reverted were restored on staging by the
coordinator (cherry-picks of `094ae5056 94b259391 963750802 82b2f6bc3 ff1dfecd3 b109fd4bc`).

What landed: `__canvas_boundsHeader` takes each builder's extent, pad and rim; under an item
transform the rim is half the inverse's norm in shape units. The geometry record says how
much of its box is rim (header slots 67 and 68, carried to the vertex stages in two unused
item-block words), and the three group sites (`__canvas_drawBound`, `mfb_canvas.vert`,
Metal's `mfbVertex`) scale that rim by the group inverse's norm. `ITEM_BLOCK_SIZE` did not grow.

Accepted pixel changes (every other compared frame is byte-identical to base: 108 of 112 on
each of software, Metal, box 2226 and box 206, the agent's runs):

1. A shape minified along one axis only. Base's box cut lit pixels and its renderers
   disagreed; the fix lights the pixels the coverage rule gives, the same on every renderer.
   `bugs/repro/bug-991/uneven_minify.mfb`: 12 (software) and 11 (Metal) lit at base, 14 and 14
   with the fix.
2. The zero-gradient fallback column under a huge magnification is cut by the tighter box.
   `bugs/repro/bug-991/fallback_column.mfb`: 191 lit at base, 73 with the fix, software and
   Metal alike. What remains of the column is bug-1090.

Coordinator's re-runs on staging (`MFB_CANVAS_DUMP`, lit pixels counted, main's compiler at
`471aaff16` against staging's): `halo_in_shrinking_group.mfb` 26/26 against 26/26;
`anisotropic_halo.mfb` 26/26 against 26/26; `uneven_minify.mfb` 12/11 against 14/14;
`fallback_column.mfb` 191/191 against 73/73 (software/Metal each).
`bash tools/canvas-bench/bench.sh <mfb> magnified-polygons`, Metal, two runs each, load
average 8 to 21 (indicative): `flat` 37.9, 35.9 -> 9.3, 7.5; `group` 37.8, 35.5 -> 10.9, 7.9;
`circle-xf` 12.5, 10.9 -> 4.4, 4.4; `x100` 6.9, 6.6 -> 7.2, 6.9; `pix` 4.2, 3.8 -> 3.5, 3.9.
All 22 `tests/canvas` targets pass on staging with the fix (`/tmp/bs23/merge-check3-summary.txt`).

Not met, and why it is not chased: software `flat` is 2.9 to 4.1 times software `pix`
against the doc's Goal of 3; the Goal assumed three distance samples a transformed pixel and
the oracle takes five. Secondary 1 (the `0.5 * max(F, 1)` early-out radius) was measured and
left alone. An arc keeps its whole box (its distance jumps at the sweep ends, bug-1088).

## Round 3 (bug-staging-23 agent F, 2026-10-12)

The third Cause note was built: round 2's item-transform half restored (`git revert` of
its two reverts: `094ae5056`, `94b259391`) and bug-1083's group half on top. It was
measured and then reverted, so no rendering code differs from `a6749e280` at the
branch's head. What the group half is, its proofs, its Golden impact, the commits and
the command that brings the build back are in bug-1083's document, "Built, round 3".
This section is what concerns THIS bug's half. Base for "before" is `a6749e280`.

**The case that stopped round 2 is repaired.** `halo_in_shrinking_group.mfb` at group
scale 0.1 draws its 26 pixels again, byte-identical to the base on software and Metal
(24 with this half alone), and 46 at 0.25.

**A new changed case, and it is this half's own.** The second Cause note says every
pixel stays identical because the rim `0.5 * F` covers every pixel that can take
coverage. That is true, and it is the reason a pixel changes: the OLD rim, one unit,
did not cover them where `F > 2`, that is, where the transform shrinks along some axis.
Under a uniform shrink the item's own extent hides it. Under an uneven one the base
draws its box and not its coverage rule. `bugs/repro/bug-991/uneven_minify.mfb` (a
circle of radius 0.5 under `Transform[a := 3, d := 0.03, tx := 300, ty := 200.3]`, no
group):

```
mfb build --app --debug <dir>
MFB_MACAPP_HEADLESS=1 MFB_CANVAS_SYNC=1 MFB_CANVAS_GPU=0|1 MFB_CANVAS_DUMP=f.rgba <binary>
                          software            Metal
before (a6749e280)        12 lit, x 294..305  11 lit, x 294..304
with this half            14 lit, x 293..306  14 lit, x 293..306
```

The base's box there is `(0.5 + 1.0) * 3 + 1 = 5.5` px either side of x = 300, which
software's truncation makes 294..305 and Metal's pixel-centre rule 294..304: the base's
lit set IS its box, on each renderer, and the two renderers disagree by a pixel of value
34 (past `GPU_DEFAULT`: run as an extra row of a scratch copy of
`scripts/test-canvas-gpu-rows.sh`, the scene fails on box 2226 and on box 206 at the
base, and passes on box 2226 with the fix). The coverage rule with
no box lights 14 pixels. The same composition reached through a group
(`halo_in_shrinking_group.mfb` at 0.03) goes from 12 to 14 the same way, and with both
halves the two spellings draw the same row. Round 2's 102 dumps held no transform that
shrinks along one axis only (its twentieth-scale items were uniform), which is why it
read "byte-identical".

So the Non-goal "every rendered pixel, on every backend" and the Goal's bound cannot
both hold there. The pixels the old box cut off are the artefact family of
`anisotropic_halo.mfb`, which the second note kept.

**And a case where `0.5 * F` is NOT a bound: the fallback gradient.** The second note's
rule is "a pixel takes coverage only where `d < 0.5 * g`, and `g <= F`". There is a
second way to take coverage: a pixel whose gradient comes out under 1e-6 is given
`g = 1` (`__canvas_drawGeometry`, `shapeCoverage`), and then it is lit wherever
`d < 0.5` of the item's UNITS, whatever `F` is. That is what the `max(F, 1)` in
`shapeCoverage`'s skip radius allows for (secondary 1). Both central differences vanish
exactly only at a point midway between two parts of the shape, which is inside its
extent; but they fall UNDER the threshold along the whole line of such points, for a
distance of about `1e-6 / F` units either side, once the transform magnifies more than
about a thousand times. `bugs/repro/bug-991/fallback_column.mfb` (one polygon of two
squares 0.9 units apart, magnified 14,000 times, so both are far off the surface):

```
                          software                 Metal
before (a6749e280)        191 lit, x 450, y 205..395   the same 191
with this half            73 lit, x 450, y 264..336    the same 73
```

The base draws the whole faint column the fallback makes (its box is a unit, 14,000 px,
either side); the new box is the squares' extent, 7 px, and a rim of 28 (the `1/256` in
the widening of `F` is 27 px at this scale) and cuts it. An untransformed item inside a
group that magnifies as much is cut the same way by bug-1083's half
(`bugs/repro/bug-1083/fallback_column_in_group.mfb`: 191 lit before, 19 after). The rim that IS a
bound for this rule is `0.5 * max(F, 1)`, and that is the old unit again under any
magnification, which is the cost this bug exists to remove. So the box cannot be both
tight under magnification and a bound, unless the fallback stops lighting pixels that
far from the shape, and that is a change of the coverage rule (secondary 1's subject),
not of a box. This is the third note's stop condition; nothing was patched around it.

**The software half of the Goal**, `bash tools/canvas-bench/bench.sh <mfb>
magnified-polygons 3 --software`, two runs each (load average 14 to 60, indicative):
`flat` 20,634 and 19,800 ms a frame before, 164.0 and 196.0 after; `pix` 57.5 and 47.5
in the same two runs. That is 2.9 and 4.1 times `pix`. The doc's "within 3" assumed
three samples a transformed pixel; the oracle takes five (round 2, finding 2), which
predicts about 5.7. Reported, not chased.

**Metal**, five runs each (load 10 to 30): `flat` 31.5, 33.6, 31.5, 31.3, 31.5 before and
13.9, 8.3, 7.7, 14.3, 8.5 after; `x100` 7.1, 7.5, 8.4, 7.7, 7.3 and 6.6, 7.2, 9.9, 6.6,
6.2; `circle-xf` 12.1, 12.5, 14.5, 11.8, 14.0 and 9.8, 4.7, 4.3, 7.0, 4.4; `pix` 4.0,
4.9, 6.1, 4.2, 4.9 and 6.2, 4.0, 3.9, 3.9, 4.3. `flat` is within noise of `x100`.

**Secondary 1** (the `0.5 * max(F, 1)` skip radius), measured as the doc says and not
changed: with the pad fixed, `flat` (median 8.5 ms, where the radius is 85 px and no
pixel of a quad skips) against `x100` (median 6.6, where `F` is 0.83, the radius is
0.85 px and the inside of every disc skips). The radius is worth at most the difference
between those two, about 2 ms of 8 at this size, inside the run-to-run spread. On box
206 (Intel UHD 630; `... --target windows-x86_64 --host test@192.168.1.206
magnified-polygons`, four runs) it is not inside the spread: `flat` 34.2, 30.2, 30.1,
30.4 against `x100` 21.3 four times and `pix` 12.5, 12.5, 12.4, 12.4 (before: `flat`
363.4 to 364.7, `x100` 18.1 to 18.8). So on that GPU the five-sample path over a whole
quad is 9 to 13 ms of a 30 ms frame, and the radius is worth measuring there once the
fallback it guards is settled. No variant was built.

## Stopped, round 2 (bug-staging-23 agent D, 2026-10-12)

The SECOND Cause note (below) was implemented as written, measured, and then reverted.
No rendering code differs from before this round at the branch's head. The build is
kept in history so the numbers here can be reproduced: `4d9923d44` (the RED test),
`5752c7e5b` (the fix, the spec sentence, the `metal.rs` comment), reverted by
`6d3161619` and `e84c14262`. Base for "before" is `896a4876d`.

It is stopped on two things the note does not settle, and one more is recorded because
the next note has to know it.

**1. A changed pixel: a transformed item inside a group whose matrix shrinks.** The note
leaves the three group sites alone (`__canvas_drawBound`, the two GPU vertex stages) and
says every pixel stays identical. Those sites map the item's FINISHED box, so they
inherit whatever rim the item's box carries, and under a group the gradient bound is the
COMPOSED inverse's, which a shrinking group makes larger than the item's own `F`. The
old rim (one unit of the item's space) covered that by accident for most scenes; `0.5 F`
of the item's own inverse does not. `bugs/repro/bug-991/halo_in_shrinking_group.mfb` (a
circle of radius 0.5 under `Transform[a := 100, d := 1]`, inside a `canvas::Group` whose
transform scales by 0.1):

```
mfb build --app --debug <dir>
MFB_MACAPP_HEADLESS=1 MFB_CANVAS_SYNC=1 MFB_CANVAS_GPU=0|1 MFB_CANVAS_DUMP=f.rgba <binary>
                          software            Metal
before (896a4876d)        26 lit, x 287..312  26 lit, x 287..312
with the fix (5752c7e5b)  24 lit, x 288..311  24 lit, x 288..311
group scale 0.03 instead: before 12 and 12 lit; with the fix 10 (software) and 8 (Metal)
```

With the group scaling by 0.25 the frames are identical (12 lit pixels, both
compilers, both renderers). Only an UNEVEN item transform under a shrinking group
matrix does it: under a uniform item transform the group stage's own pixel is enough.
The lost pixels are the uneven-transform halo (finding 1 of the first stop), which the
note chose to keep. Keeping it under a group needs the group sites to grow the box by
`0.5 F` of the composed inverse, which needs the header to say how much of its box is
rim: the record change the note assigns to bug-1083. So the pad fix and bug-1083 have
to land together, or the owner has to rule that this halo may shrink under a group.

**2. The software half of the Goal is not met, and cannot be by the pad.** `bash
tools/canvas-bench/bench.sh <mfb> magnified-polygons` (Metal, 40 frames) and
`... magnified-polygons 3 --software`, each compiler twice, ms a frame (load average 9
to 14 for Metal, 13 to 68 for software, from other agents' work: indicative):

| mode | Metal before | Metal with the fix | software before | software with the fix |
|---|---|---|---|---|
| `flat` | 241.8, 36.6 | 7.5, 9.1 | 33,459, 29,968 | 210.0, 165.5 |
| `group` | 41.1, 38.1 | 36.9, 47.2 | 28,246, 14,640 | 29,385, 25,469 |
| `x10` | 15.7, 10.4 | 8.5, 9.2 | 1,214, 1,276 | 312.0, 169.0 |
| `x100` | 7.6, 6.4 | 7.2, 8.4 | 562.0, 225.5 | 223.5, 266.5 |
| `pix` | 4.9, 5.3 | 4.2, 5.1 | 70.0, 35.5 | 46.5, 45.0 |
| `circle-xf` | 20.4, 15.5 | 4.6, 4.5 | 5,588, 4,917 | 72.5, 113.0 |
| `circle-pix` | 10.5, 3.5 | 4.0, 12.3 | 16.0, 13.5 | 34.0, 78.5 |

Metal `flat` meets the Goal (within noise of `x100`), and `circle-xf` is within noise of
`circle-pix`. `group` does not move, as the note expects. Software `flat` is 3.7 and 4.5
times software `pix` (165.5 / 45.0, 210.0 / 46.5), and the Goal says within 3. The
Goal's 3 is "the three-evaluation transformed rule"; the oracle takes FIVE distance
samples a transformed pixel (the centre and four neighbours, `__canvas_drawGeometry`),
over a box that is now 31 px square against `pix`'s 29, which predicts 5.7 times. So
the pad is not what misses it, and secondary 1 (the `0.5 * max(F, 1)` skip radius) is
not what misses it either: the software renderer has no skip. The Goal's number needs
the owner's ruling. Nothing was changed for secondary 1.

**3. An arc cannot take the rim at all.** The note's rule is "a pixel takes coverage only
where `d < 0.5 g`, and `g <= F`". That holds for a distance that is 1-Lipschitz. An
arc's is not (out of its sweep it answers 1e6; `shapeCoverage` says so and never skips
for one; the spec says so), and with butt caps it is visible today:
`bugs/repro/bug-1088/butt_arc_rays.mfb` (a butt-capped arc of radius 0.5 under a
transform that scales by 100) draws two dotted rays from its centre along the ends of
its sweep, out to the edge of its box, on software and on Metal alike (at `896a4876d`:
1,190 lit pixels, 110 of them more than 57 px from the centre of an arc whose stroke
ends at 55, x from 161 to 455). Any change of a transformed arc's box moves where those
rays end. The build therefore had `__canvas_arcHeader` hand its box over whole with no
rim, as the glyph run, the text box and the picture do, and a transformed arc keeps
today's box and today's pixels. The rays themselves are a separate bug, not fixed here
and not yet filed.

### What the build was

- `__canvas_boundsHeader(h, minX, minY, maxX, maxY, padX, padY, rim)`: the extent, how far
  the builder's pad reaches past it on each axis (with the rim, computed exactly as
  today), and how much of that pad is rim. Untransformed it writes `minX - padX` and so
  on, the operations each builder performed inline (an ellipse's pad differs by axis and
  is `sqrt(...) + (reach + 1.0)`, which is why the mapper is handed the pad and not the
  reach: `(reach + 1.0)` and `reach`, `1.0` separately give different floats there).
  Transformed with a rim it maps the extent grown by `(pad - rim) + 0.5 * frob`,
  `frob = F * (1 + 1/256) + 1/256` as the shaders compute it, then adds its pixel.
  Transformed with rim 0 it maps the box as it is.
- Nine builders pass `(extent, pad, pad, __CANVAS_BOUNDS_RIM)` (a new module constant,
  1.0); the arc, `__canvas_glyphRunHeader`, `__canvas_textBoxHeader` and the picture
  pass their box with `0.0, 0.0, 0.0`. `helper_curves.rs` fits the rim form (its pad is
  `max(strokePad, capReach + 1.0)`).
- 57 non-comment lines added or removed in `helper_geometry.rs`, and one line changed in
  each of `helper_curves.rs`, `helper_hit.rs` and `helper_picture.rs`
  (`git show 5752c7e5b`).
- Golden impact, had it landed: every function that calls `__canvas_boundsHeader` moves
  in the `app_mouse_surface` sentinel (`__canvas_boundsHeader` itself, `__canvas_rectHeader`,
  `__canvas_circleHeader`, `__canvas_ellipseHeader`, `__canvas_segmentHeader`,
  `__canvas_arcHeader`, `__canvas_polygonHeader`, `__canvas_polylineHeader`,
  `__canvas_textHeader`, `__canvas_glyphRunHeader`, `__canvas_textBoxHeader`,
  `__canvas_pictureHeader`, the curve header builder), plus the new module constant's
  slot; no pixel golden.

### What it proved, with the fix in place

- RED then GREEN: `cargo test --test rt_canvas_damage a_transformed_items_box` failed at
  `896a4876d` ("a filled square, magnified 100 times: the box 58,58,285,285 is more than
  5 px past the item's hull (160.0, 160.0, 240.0, 240.0)") and passed with the fix (the
  seven boxes: 157,157,86,86 / 447,147,106,106 / 90,490,120,20 / 467,467,66,66 /
  690,90,120,120 / 686,357,128,86 / 277,537,46,46). The test reads each item's box as
  the damage rectangle of a present that changes only that item (`MFB_CANVAS_DAMAGE=1`,
  the `damage=` stats field), so it needed no new hook.
- `rt_canvas_geo_native` 4 passed (0 mismatches); `rt_canvas_golden` 26, `rt_canvas_rasteriser`
  106, `rt_canvas_curves` 40, `rt_canvas_damage` 10, `rt_canvas_hit` 13, `rt_canvas_metal`
  62, `rt_canvas_picture` 16, `rt_canvas_font` 33, `rt_canvas_autopaint` 22,
  `rt_canvas_picture_fit` 9, `rt_canvas_pointer` 2, `rt_canvas_graphics_thread` 18,
  `rt_canvas_group_ownership` 11: all passed (`cargo test --test <name>` each).
- Frames byte-identical before and after, `cmp` of every frame: Metal and software on
  this machine, 102 of 102 (the 48 render rows of `scripts/test-canvas-gpu-rows.sh`,
  `anisotropic_halo.mfb` with its 26 lit pixels on both, `butt_arc_rays.mfb`, and a
  scratch scene of every kind under magnifying, turned, sheared, uneven and
  twentieth-scale transforms with miter joins, square caps, dashes, three text runs and
  two pictures); box 206, 96 of 96 (`--target windows-x86_64 --keep`); box 2226, 96 of
  96 (`--target linux-aarch64 --keep`). Every row passed on both boxes.
- NOT identical: finding 1's scene, which none of those rows contains.

### What a third note has to settle

1. Whether the pad fix lands with bug-1083's record change (the header carries its rim,
   the three group sites replace it by `0.5 F` of the composed inverse), or the halo of
   an uneven item transform may shrink under a shrinking group.
2. The software Goal: 3 times `pix` is not reachable with five samples a pixel.
3. That an arc keeps its box until its sweep discontinuity is fixed.

## Stopped (bs23-agent-b, 2026-10-12, bug-staging-23)

No rendering code was changed for this bug. The audit below found two things the Cause
note assumes that the code does not do, and one it does not mention. Each is enough to
stop on by the brief's rules (a changed pixel; a mapper that would have to know how wide a
header's rim is).

**1. The removed pixels do not all have coverage 0.** Under a transform the coverage is
`clamp(0.5 - d / g, 0, 1)`, `d` the distance in the item's own space and `g` the length of
its central differences at half a surface pixel (`__canvas_drawGeometry`,
`shapeCoverage`). `d / g` is the surface distance only where the map is a similarity. All
that bounds it is `g <= F` (`F` the inverse's Frobenius norm), so coverage can be nonzero
as far as `0.5 * F` shape units from the outline, and through an uneven transform that is
more than a surface pixel. Measured at main, `bugs/repro/bug-991/anisotropic_halo.mfb` (a
`Circle` of radius 0.05 under `Transform[a := 100, d := 1, tx := 300, ty := 200.3]`: on
the surface an ellipse from x = 295 to 305):

```
mfb build --app --debug <dir>
MFB_MACAPP_HEADLESS=1 MFB_CANVAS_SYNC=1 MFB_CANVAS_GPU=0 MFB_CANVAS_DUMP=sw.rgba <binary>
lit pixels (red channel non-zero): 26, x 287..312, y 200
  (287, 200) 38   (288, 200) 53   (289, 200) 64  ...  (311, 200) 53   (312, 200) 38
```

`MFB_CANVAS_GPU=1` (Metal) draws the same 26 pixels with the same values. The mapped hull
plus one pixel is x 294..306, so the Cause note's box drops 13 lit pixels, 7 on the left
and 6 on the right. Today they are drawn because the shape-unit rim is 100 px wide there.
They are an artefact of the first-order estimate (a pixel 7 px past the end of the
ellipse is not "within half a pixel" of it), and the same on-surface ellipse written as a
radius-5 circle under `Transform[a := 1, d := 0.01]` already has them cut by today's
2 px rim, so the picture today depends on how the transform is factored. But "every
rendered pixel stays identical" is not true of the fix as assigned, and which pixels a
thin shape under an uneven transform shows is a contract question the doc does not
settle. A rim that IS pixel-exact follows from the same bound: map
`extent +- reach +- 0.5 * F` and add the pixel after. Under a uniform scale `S` that is
0.71 px plus the pixel, whatever `S` is, so it meets the Goal; under an uneven transform
it keeps the halo. It was not built or measured here.

**2. Not every caller of the mapper passes a 1.0 rim.** `__canvas_boundsHeader` has
twelve callers (`grep -n "__canvas_boundsHeader(" src/codegen/builtins/canvas/helper_*.rs`). Nine pass `extent +- reach +- 1.0`. Three do not:

| caller | box it passes | what "take 1.0 off, map, add a pixel" does |
|---|---|---|
| `helper_geometry.rs:__canvas_glyphRunHeader` | x: the pens' range `+- 2 * fontSize`; y: ascent and descent `+- 2.0`. A metrics box, not ink plus a rim | x keeps `2 * fontSize - 1`, which is negative for a font under half a unit (text drawn in unit space and magnified, this bug's own case): the box inverts. The run's box is the damage rectangle (`helper_damage.rs`, through `__canvas_drawBound`) and the quad each glyph's own quad is narrowed to on the GPUs (`metal.rs`, "The per-glyph quad narrows the item's quad") |
| `helper_hit.rs:__canvas_textBoxHeader` | the layout box exactly, no rim (worker-only record) | shrinks it by a unit a side. Nothing reads slots 16-19 of a text box today (`__canvas_hitCurrent` skips the bounds reject for one and tests the `TEXT_BOX` slots), so it is wrong and unobserved |
| `helper_picture.rs:__canvas_pictureHeader` | the destination `+- 2.0` | correct, by accident of the numbers: a `grown` picture's rectangle edge really is one unit outside the destination (radius slot 1, box whole), so `+- 1.0` is its extent; a picture that is not `grown` keeps one surplus unit |

So the mapper cannot take a fixed 1.0 off without knowing which header it has, and it may
not name kinds. The header would have to say how wide its rim is (a slot), or the
builders would have to hand the mapper the rim separately (the doc's original Fix Design
step 1, which the Cause note rules out: "the nine header builders are not touched").
Either is the coordinator's call.

**3. The GPU group stage adds its own pad on top.** `mfb_canvas.vert` and Metal's
`mfbVertex` grow the header's box by `length(gi)` (the group inverse's Frobenius norm, in
the group's space) before the forward map. That is the at-least-one-pixel pad
`__canvas_drawBound` adds after its map; the header's own 1.0 rim is still inside the
box both carry. The software and GPU sites are therefore not the same expression today,
and a fix has three group sites (`__canvas_drawBound`, the two vertex stages), not one.

### Audit: each builder's reach without the 1.0

"Full" means the box without the 1.0 contains everything the item can draw at coverage
above zero in its own space, before the antialiasing rim.

| site | box before the 1.0 | verdict |
|---|---|---|
| `helper_joins.rs:__canvas_strokePad` (polygon, rectangle, stroked text outlines, `PolyCurve`, `CurvePolygon`) | `half`; `half * sqrt(2)` for a dashed Square paint (`__canvas_dashCapReach`); `max(that, half * max(miterLimit, 1))` for a Miter join | full: a miter point is at most `half * miterLimit` from its vertex, a bevel or round corner within `half`, a dash end within the cap reach |
| `__canvas_rectHeader` | the rectangle `+- strokePad` | full |
| `__canvas_circleHeader` | `radius + dashCapReach(half)` | full |
| `__canvas_ellipseHeader` | the hull `hx`, `hy` `+ dashCapReach(half)` | full |
| `__canvas_segmentHeader` (Line) | the two ends `+- capReach(half, cap)` (`half * sqrt(2)` for Square) | full |
| `__canvas_arcHeader` | `radius + capReach(half, cap)` about the centre | full (the whole circle's box, conservative) |
| `__canvas_polygonHeader` | the points' box `+- strokePad` | full |
| `__canvas_polylineHeader` | the points' box `+- max(capReach, joinReach)` | full |
| `__canvas_textHeader` (stroked run, outlines) | the outlines' box `+- strokePad` | full |
| `helper_curves.rs` `PolyCurve` / `CurvePolygon` header | the pieces' extent `+- strokePad`, and for an open path `max(strokePad, capReach + 1.0)` | full; a plain 1.0 rim |
| `__canvas_glyphRunHeader` | see 2 | NOT a 1.0 rim |
| `__canvas_pictureHeader` | see 2 | 1.0 after the grown edge; conservative otherwise |
| `helper_hit.rs:__canvas_textBoxHeader` | see 2 | no rim at all |
| `__canvas_clipHeader` (slots 22-25) | maps first, then a pixel | already the rule; untouched |
| `func_geo_build.rs` (native, identity transform only) | `+ 1.0` in surface pixels | untouched; an untransformed header stays as it is |

Consumers of slots 16-19, all through `__canvas_drawBound` except the GPU quad:
`__canvas_drawGeometry`'s pixel loop (truncates each bound to a pixel index),
`helper_damage.rs` (the changed-item rectangles and the remembered scene; both sides use
the same function, so smaller bounds shrink damage consistently), `helper_hit.rs`
(`__canvas_hitCurrent` rejects outside the bounds `+- 3.0`; a smaller box narrows the
reject and never the hit, which the item's own distance decides), and the item block's
`quad` on both GPUs.

### What was added

- `tools/canvas-bench/bench.sh <mfb> magnified-polygons [FRAMES] [--software]`
  (`gen/magnified.sh`, this bug's repro with the mode written in). At this commit, load
  average 12 to 17 (indicative):

| mode | Metal ms/frame (40 frames) | software ms/frame (3 frames) |
|---|---|---|
| `flat` | 30.1 | 17,576 |
| `group` | 29.9 | 18,261 |
| `x10` | 9.1 | 556 |
| `x100` | 5.8 | 164 |
| `pix` | 3.8 | 34 |
| `circle-xf` | 12.7 | 5,070 |
| `circle-pix` | 3.0 | 14 |

  These are headless render-phase times, not the real-window `frameMs` of the table
  above; the ratios are the same.
- `bugs/repro/bug-991/anisotropic_halo.mfb`, the scene of finding 1.

Not done, because they need the fix: the bounds assertion test (it fails today by
construction and cannot be committed red), byte-identity against pre-transformed
coordinates, and secondary 1 (the `0.5 * max(F, 1)` early-out radius), which the doc says
to measure after the pad is fixed.

## Cause (coordinator, 2026-10-12, bug-staging-23)

OWNER RULING 2026-10-12 (third note; supersedes "leaves the phase"): keep bug-991's built fix
(`5752c7e5b`, reverted by `6d3161619`) and fix bug-1083 NOW, in this phase, so the fix's one
changed case is repaired. bug-991 and bug-1083 are ONE fix with one cause and go to one agent.

- Rule (as bug-991's second note): a pixel takes coverage only where `d < 0.5 * g`, and `g` is bounded by the norm of the COMPOSED inverse (item, then groups). The rim a box needs is therefore known only where the last matrix is known.
- Place: three sites carry a FINISHED header box (slots 16-19, builder rim folded in) through a group's forward part: `helper_items.rs:__canvas_drawBound`, `mfb_canvas.vert` and Metal's `mfbVertex`. They cannot replace the rim, they pad differently from each other (a pixel after the map in software; `length(gi)` before it on the GPUs), and so a magnifying group scales the rim (bug-1083's cost) while a shrinking one under-covers an unevenly transformed item (bug-991's changed pixels: 26 -> 24 at group scale 0.1; 12 -> 10 software, 8 Metal at 0.03).
- Fix: the geometry record carries its rim-free box (the item's extent and reach under its own transform, hull taken, nothing added) beside the padded one. The three group sites take ONE expression from it: map the rim-free box grown by `(padded - rimfree) * |gi|` (the header's own rim, scaled by the group inverse's norm, which bounds the composed norm by the product), then add the pixel. Ungrouped, slots 16-19 are read as today, so nothing ungrouped moves. A header whose pad is not a rim (glyph run, text box, picture, arc) stores rim-free = padded, which is today's box under a group.
- Proof the note is right before anything lands: the two scenes that stopped bug-991 (`bugs/repro/bug-991/halo_in_shrinking_group.mfb` at group scale 0.1 and 0.03) keep their existing lit pixels on software AND Metal with both halves in place, and `magnified-polygons` `group` falls to `flat`'s cost.

(The second note, below, is the item-transform half and still holds, except its last bullet.)

SECOND note. The first (take the 1.0 rim off at the mapping sites, map, add a pixel; no
builder touched) was WRONG on two counts, both shown by bs23-agent-b's audit above: the
removed pixels are not all coverage 0 (finding 1), and three callers do not pass a 1.0 rim
(finding 2). This note replaces it.

- Rule: under a transform a pixel takes coverage only where `d < 0.5 * g`, and `g <= F` (the inverse's Frobenius norm, the bound `shapeCoverage`'s early-out already uses). So the rim a mapped box needs is `0.5 * F` SHAPE units around `extent +- reach`, and then the pixel `__canvas_boundsHeader` already adds after mapping. That rim depends on the transform, so only the mapper can know it; a builder's literal `+ 1.0` is the rim of the untransformed case only.
- Place: `helper_geometry.rs:__canvas_boundsHeader` is handed a box with the builder's rim already folded in, so it cannot replace it. Fix: every caller hands it the box and its rim separately (what each already computes; no caller's behaviour is decided by its kind). Untransformed, the mapper writes exactly today's values by today's operations (`min - (reach + rim)`: slots 16-19 and `canvas::geoBuild` stay bit-identical, `MFB_CANVAS_GEO_VERIFY` 0 mismatches). Transformed, it maps `extent +- reach +- 0.5 * F` and adds its pixel. A caller whose pad is not a rim (`__canvas_glyphRunHeader`, `__canvas_textBoxHeader`, the picture's 2.0) passes its box as it is with rim 0.
- Every pixel stays identical on every backend, the uneven-transform halo of `bugs/repro/bug-991/anisotropic_halo.mfb` included (it is inside `0.5 * F`; whether that halo should exist is not this bug). Under a uniform scale S the box is the mapped extent plus 0.71 px plus the pixel, whatever S is.
- NOT in this bug any more: the scaled-`canvas::Group` cost. Its three sites (`__canvas_drawBound`, the two GPU vertex stages) map a FINISHED header box and would need the header to carry its rim, a record change. It is bug-1083, a later phase.

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
