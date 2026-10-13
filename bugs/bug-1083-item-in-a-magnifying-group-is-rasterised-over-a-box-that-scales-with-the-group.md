# bug-1083: an item inside a magnifying `canvas::Group` is rasterised over a box that scales with the group, so 49 small polygons cost 40 ms a frame

Last updated: 2026-10-12
Effort: medium (a geometry-record change)
Severity: MEDIUM
Class: Other (Performance)

Status: Open (built, measured and reverted in bug-staging-23 round 3, 2026-10-12: the Cause note's expression is not a bound for the coverage rule's fallback gradient, and one scene gains two lit pixels the base's box cut off; see "Built, round 3". The build is commits `963750802`, `82b2f6bc3`, `ff1dfecd3`, `b109fd4bc` on bug-991's `094ae5056` and `94b259391`)
Regression Test: written and reverted with the build (commit `963750802`): `an_items_box_inside_a_scaling_group_is_its_mapped_extent_and_a_rim_of_pixels` in `tests/canvas/rt_canvas_damage.rs` and `an_unevenly_transformed_item_in_a_shrinking_group_keeps_every_covered_pixel` in `tests/canvas/rt_canvas_metal.rs`; `tools/canvas-bench/bench.sh <mfb> magnified-polygons` (`group`)

Found during `bug-staging-23` (2026-10-12). Added to that phase by the owner on 2026-10-12.

## Cause (coordinator, 2026-10-12, bug-staging-23)

OWNER RULING 2026-10-12 (third note; supersedes "leaves the phase"): keep bug-991's built fix
(`5752c7e5b`, reverted by `6d3161619`) and fix bug-1083 NOW, in this phase, so the fix's one
changed case is repaired. bug-991 and bug-1083 are ONE fix with one cause and go to one agent.

- Rule (as bug-991's second note): a pixel takes coverage only where `d < 0.5 * g`, and `g` is bounded by the norm of the COMPOSED inverse (item, then groups). The rim a box needs is therefore known only where the last matrix is known.
- Place: three sites carry a FINISHED header box (slots 16-19, builder rim folded in) through a group's forward part: `helper_items.rs:__canvas_drawBound`, `mfb_canvas.vert` and Metal's `mfbVertex`. They cannot replace the rim, they pad differently from each other (a pixel after the map in software; `length(gi)` before it on the GPUs), and so a magnifying group scales the rim (bug-1083's cost) while a shrinking one under-covers an unevenly transformed item (bug-991's changed pixels: 26 -> 24 at group scale 0.1; 12 -> 10 software, 8 Metal at 0.03).
- Fix: the geometry record carries its rim-free box (the item's extent and reach under its own transform, hull taken, nothing added) beside the padded one. The three group sites take ONE expression from it: map the rim-free box grown by `(padded - rimfree) * |gi|` (the header's own rim, scaled by the group inverse's norm, which bounds the composed norm by the product), then add the pixel. Ungrouped, slots 16-19 are read as today, so nothing ungrouped moves. A header whose pad is not a rim (glyph run, text box, picture, arc) stores rim-free = padded, which is today's box under a group.
- Proof the note is right before anything lands: the two scenes that stopped bug-991 (`bugs/repro/bug-991/halo_in_shrinking_group.mfb` at group scale 0.1 and 0.03) keep their existing lit pixels on software AND Metal with both halves in place, and `magnified-polygons` `group` falls to `flat`'s cost.

Split out of bug-991 on 2026-10-12. bug-991 fixes the box of an item under its OWN transform.
An untransformed item inside a `canvas::Group` whose matrix magnifies still draws over a box
far larger than the item: its header's box carries a 1.0 antialiasing rim in the group's
space, and the three sites that carry a finished header through the group's matrix scale that
rim with it.

**The single correct behavior a fix produces:** the area rasterised for an item inside a group
is the group-mapped extent of the item plus its reach, plus the rim the mapped coverage needs
(half the composed inverse's norm, see bug-991's second Cause note) and one pixel; it does not
grow with the group's magnification. Every pixel stays identical.

## Reproduction (main `3745e626a`, Metal, M2 Max, load average 28, so indicative)

```
bash tools/canvas-bench/bench.sh <mfb> magnified-polygons
mode=group      | ms/frame=40.3     (the same items untransformed, in a Group scaled 170)
mode=pix        | ms/frame=4.5      (the same picture in pixel coordinates)
```

`bugs/repro/bug-991/magnified.mfb group` is the same scene as a program.

## Cause, by reading (bug-991's audit, bs23-agent-b)

- `helper_items.rs:__canvas_drawBound` maps the header's box (slots 16-19) through the groups'
  forward linear part and pads by a pixel; `mfb_canvas.vert` and Metal's `mfbVertex` grow the
  box by `length(gi)` and map it. All three start from a box that already contains the
  builder's rim.
- The header does not say how wide its rim is, and the builders' rims differ
  (`__canvas_glyphRunHeader`, `__canvas_textBoxHeader`, the picture's 2.0), so the mappers
  cannot take it off without naming kinds. A fix needs the record to carry the rim-free box or
  the rim (a header slot, hashed and built natively too), and one expression at the three sites.

## Built, round 3 (bug-staging-23 agent F, 2026-10-12)

Built on bug-991's item-transform half (`5752c7e5b`, restored as `094ae5056` and
`94b259391`), measured, and then reverted: no rendering code differs from `a6749e280`
at the branch's head. The build is kept in history: `963750802` (the two RED tests),
`82b2f6bc3` (the record, the three sites, both emitters, the vertex SPIR-V), `ff1dfecd3`
(the spec and `.ai/canvas-threading.md`), `b109fd4bc` (rustfmt); reverted by
`c99311605`, `7dc7d3c43`, `24eabb5d4`, `eaea4b2a7`, `13515f76c`, `95fe00093`. To have it
back: `git cherry-pick 094ae5056 94b259391 963750802 82b2f6bc3 ff1dfecd3 b109fd4bc`.

Base for every "before" is `a6749e280`; main's debugger build (`3745e626a`) is NOT that
base, it lacks rounds 1 and 2.

### What the build is

- **The record carries the rim, not the rim-free box**: header slots 67 (x sides) and 68
  (y sides), `__CANVAS_GEO_RIM`, in the units of the box; `__CANVAS_GEO_HEADER` is 69.
  The rim-free box is `box -/+ rim`, so the two say the same thing; the rim is two floats
  and not four, it is what each site multiplies, and untransformed it is the builder's
  constant with no subtraction. `__canvas_boundsHeader` writes it: the builder's rim
  untransformed; `(|a| + |c|) * 0.5 * F + 1` and `(|b| + |d|) * 0.5 * F + 1` under a
  transform (what the mapped rim and the pixel add to the hull); 0 for a box handed over
  whole (glyph run, text box, picture, arc). `func_geo_build.rs:put_bounds` writes the
  constant for the five kinds it builds.
- **One expression at the three sites** (`__canvas_drawBound`, `mfb_canvas.vert`,
  `mfbVertex`): lower `(bound + rim) - rim * N`, upper `(bound - rim) + rim * N`, `N` the
  Frobenius norm of the groups' inverse. The derivation is `__canvas_drawBound`'s
  comment: an untransformed item needs `0.5 * N` and has `1.0 * N`; a transformed one
  needs `(|a| + |c|) * 0.5 * F * N` and has that, widened, plus `N`.
- **The pixel is NOT shared, on purpose.** Software pads the mapped hull by 1 px; the
  vertex stages grow the box by `N` before the map, as before, because their quad is the
  parallelogram and has no hull to pad. With rim 0 each site is therefore exactly what it
  was, which is what keeps a butt-capped arc's rays (bug-1088) where each device drew
  them: a single shared pixel would have moved them on one device.
- **The item block did not grow.** The rim rides two words nothing wrote, the fourth
  words of `xform1` and `gradInv1` (`ITEM_RIM_WORDS`), 16.16 like the quad. The other
  ways looked at: a new `ivec4` for the rim-free box (368 to 384 bytes, both shader
  structs, the fragment SPIR-V, Metal's frame) was rejected because only the vertex stage
  reads the value and Intel's compiler is sensitive to the fragment stage; a per-draw
  value cannot work, since one instanced draw covers many items and one block is drawn
  by entries with and without a matrix. So `mfb_canvas.frag.spv` is byte-identical and
  only `mfb_canvas.vert.spv` was regenerated (box 2226, glslang 11:12.0.0).
- Layout constants and functions changed: `__CANVAS_GEO_HEADER` / `GEO_HEADER` /
  `HEADER_SLOTS` 67 to 69; new `__CANVAS_GEO_RIM`, `HEADER_RIM`, `SLOT_RIM`,
  `BOUNDS_RIM`, `ITEM_RIM_WORDS`, `__CANVAS_GM_INORM`; `__canvas_boundsHeader`,
  `__canvas_enterGroupMatrix`, `__canvas_drawBound`, `put_bounds`, both
  `emit_item_block`s (one loop of two stores each, after the quad's), `mfbVertex`,
  `mfb_canvas.vert`'s `main`. `ITEM_BLOCK_SIZE`, every region base, the draw list and
  the frame submission are untouched.

### What holds

- RED then GREEN: `cargo test --test rt_canvas_damage an_items_box_inside` ("the box
  58,58,285,285 is more than 6.5 px past the item's hull (160.0, 160.0, 240.0, 240.0)",
  then ok); `cargo test --test rt_canvas_metal an_unevenly_transformed` (24 of 26 lit at
  group scale 0.1, then ok).
- `bugs/repro/bug-991/halo_in_shrinking_group.mfb` at group scale 0.1: 26 lit pixels,
  byte-identical to the base on software and Metal. At 0.25: 46, identical.
- Frames byte-identical to the base's on the same device, `cmp` of every frame: this
  machine, software and Metal, 108 of 112 (the 48 render rows of
  `scripts/test-canvas-gpu-rows.sh`, `anisotropic_halo.mfb`, `butt_arc_rays.mfb`, round
  2's scene of every kind under its own transform, and a scene of every kind inside
  groups: magnifying 170 times, turned, sheared, uneven, shrunk to a twentieth, a group
  in a group, items with their own transforms in shrinking and magnifying groups, three
  arcs, dashes, miters, square caps, text and pictures;
  `bugs/repro/bug-1083/every_kind_in_groups.mfb`); box 2226 (lavapipe), 108 of 112, and
  box 206 (Intel UHD 630), 108 of 112, the same rows and scenes run through a scratch
  copy of the rows script with `--keep`. The four that differ are the two scenes below,
  on each renderer. On both boxes the group scene fails its GPU-against-software
  comparison at the base and with the fix alike, with identical frames: its butt-capped
  arc's rays (bug-1088) are drawn by software and not by Vulkan under a turned group.
- `rt_canvas_geo_native` 4 passed (0 mismatches); all 22 `tests/canvas` targets pass
  (`rt_canvas_golden` 26, `rt_canvas_rasteriser` 106, `rt_canvas_curves` 40,
  `rt_canvas_damage` 11, `rt_canvas_hit` 13, `rt_canvas_metal` 63); `cargo test --bin mfb
  -- canvas metal vulkan` 154 passed.
- Cost, `bash tools/canvas-bench/bench.sh <mfb> magnified-polygons` (Metal, 40 frames,
  five runs each, load average 10 to 30, ms a frame): `group` 32.0, 31.6, 31.1, 31.1,
  31.0 before and 14.8, 8.5, 7.7, 8.6, 8.4 after; `flat` 31.5, 33.6, 31.5, 31.3, 31.5
  and 13.9, 8.3, 7.7, 14.3, 8.5; `x100` 7.1, 7.5, 8.4, 7.7, 7.3 and 6.6, 7.2, 9.9, 6.6,
  6.2. `... 3 --software` (two runs each, load 14 to 60): `group` 16,355 and 15,751
  before, 175.5 and 199.0 after; `flat` 20,634 and 19,800 before, 164.0 and 196.0 after;
  `pix` 75.0 and 55.0, 57.5 and 47.5.

- Box 206 (Intel UHD 630, Vulkan), `bash tools/canvas-bench/bench.sh <mfb> --target
  windows-x86_64 --host test@192.168.1.206 magnified-polygons`, four runs each, ms a
  frame (the Mac's load average 4 to 5): `group` 357.7, 358.8, 360.2, 358.8 before and
  36.9, 37.1, 36.9, 36.8 after; `flat` 363.4, 364.5, 364.7, 364.7 and 34.2, 30.2, 30.1,
  30.4; `circle-xf` 65.4, 66.2, 66.0, 66.1 and 13.4, 13.1, 12.9, 12.9; `x10` 33.2, 32.1,
  32.0, 32.0 and 31.9, 32.2, 32.2, 31.9; `pix` 12.1, 12.5, 12.4, 12.4 and 12.5, 12.5,
  12.4, 12.4. Two rows move the wrong way and are not explained: `x100` 18.6, 18.2,
  18.8, 18.1 before and 21.3 four times after; `circle-pix` 9.2, 9.5, 9.5, 9.6 and
  10.2, 10.1, 11.3, 10.2. A third compiler, the base with ONLY the new vertex shader,
  reads 16.9 and 17.8 for `x100` and 9.4 and 9.5 for `circle-pix`, so it is not the
  shader; `x100`'s quad is smaller than before and `circle-pix`'s block differs only in
  the two rim words, which no shader reads without a group. Guess, not measured: where
  the executable's code falls (each mode is its own program).
- Box 206, the untransformed path, base and fix twice each: `shapes` (10,000 items, render
  phase) rectangles 57.98, 58.02, 58.06, 58.00 before and 58.42, 58.32, 58.49, 58.48
  after; circles 53.69, 53.71, 53.94, 53.93 and 53.97, 54.01, 53.97, 54.06; polygons
  88.52, 89.25, 89.23, 89.26 and 89.51, 89.47, 89.57, 89.49. `sweep`: 105.2 and 105.4
  fps at 95 items before, 103.4 and 102.6 after; every row from 1,000 items up within
  0.8 fps. `clips`: no row moves outside the spread of its own two runs (the unclipped
  untransformed row reads 33.2 and 23.6 before, 27.4 and 24.7 after). The cold pipeline
  compile (`bugs/repro/bug-996/coldstart.mfb`, a new exe name, `MFB_CANVAS_GPU=1`):
  first present 65,567 ms with the fix, 64,974 ms at the base.

### What does not: two scenes gain lit pixels

The base's box was never a bound where the composed transform SHRINKS along one axis
and not the other, and there the base draws its box, not its coverage rule:

| scene | base, software | base, Metal | with the fix, both |
|---|---|---|---|
| `halo_in_shrinking_group.mfb` with the group at 0.03 | 12 lit, x 294..305 | 12 lit | 14 lit, x 293..306 (the two new pixels at 22) |
| `bugs/repro/bug-991/uneven_minify.mfb` (the same composition as ONE transform, `a := 3, d := 0.03`, no group) | 12 lit, x 294..305 | 11 lit, x 294..304 | 14 lit, x 293..306 |

`MFB_MACAPP_HEADLESS=1 MFB_CANVAS_SYNC=1 MFB_CANVAS_GPU=0|1 MFB_CANVAS_DUMP=f.rgba` on a
`--app --debug` build of each. The base's 12 is its box exactly: the header's box is the
extent and one unit, `(0.5 + 1.0) * 3 = 4.5` px and the pixel, x 294.5..305.5, which
software truncates to 294..305 and Metal's centre rule to 294..304; so the base's two
renderers disagree there past `GPU_DEFAULT` (box 2226 and box 206 fail that row at the
base and pass it with the fix). The coverage rule itself, evaluated with no box, lights
14 (a model of `clamp(0.5 - d / g)` over the row gives x 293..306), and the fix's two
spellings of the composition now draw the same 14. The second scene has no group in it:
it is bug-991's half alone that changes it, because `0.5 * F` is larger than the old
unit wherever `F > 2`.

So "every pixel identical to the base" cannot hold together with "the box is a bound"
there. And there is a case in which the rim is not a bound at all, on either half: a
pixel whose gradient falls under 1e-6 is given `g = 1` and is lit up to half a UNIT from
the shape, so under a magnification past about a thousand a tight box cuts a faint
column the base draws (`bugs/repro/bug-991/fallback_column.mfb`, an item magnified
14,000 times by its own transform: 191 lit pixels before and 73 after;
`bugs/repro/bug-1083/fallback_column_in_group.mfb`, the same item with no transform in
a group that magnifies 14,000 times: 191 before and 19 after; software and Metal alike;
bug-991's document, "Round 3", has the derivation). Both need the owner's
ruling, and the second is the Cause note's own stop condition.

### Golden impact

Measured by building the fixtures with both compilers (`scripts/artifact-gate.sh <mfb>
all` reports the same 13 diffs at the base and with the fix, because staging already
carries unregenerated diffs in the same fixtures):

- `tests/syntax/app/app-mouse-surface`: all seven goldens (`.ir`, the macOS `.nir`,
  `.nplan` and `.ncode`, and the Linux and Windows `.ncode` sums). In the `.ir`: the
  bindings `__CANVAS_BOUNDS_RIM`, `__CANVAS_GEO_RIM`, `__CANVAS_GM_INORM` (new) and
  `__CANVAS_GEO_HEADER` (69); `__canvas_boundsHeader`, the ten header builders that
  fixture compiles, `__canvas_drawBound`, `__canvas_enterGroupMatrix`; and six functions whose
  only change is a source line number (`__canvas_groupSignature`, the `loadFont` family,
  `loadImage`).
- `tests/syntax/app/app-window-surface`: the macOS `.ncode` sum only (the Metal emitter
  and the MSL text are in every macOS app).
- No pixel golden.

