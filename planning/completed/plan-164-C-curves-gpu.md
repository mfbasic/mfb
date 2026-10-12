# plan-164-C: Metal and Vulkan draw `canvas::PolyCurve` and `canvas::CurvePolygon` per pixel

Last updated: 2026-10-12
Effort: large (3h–1d)
Depends on: plan-164-B

This letter moves curve drawing onto the GPU. The rational-quadratic pieces are
uploaded to a frame **curve region** with a per-item **band index**, and both shaders
evaluate the fill test and the distance per pixel, line for line with the software
renderer. The GPU is compared against the software renderer within a **curve
tolerance**, measured here and recorded as a constant. The user accepted a looser
tolerance for curves in exchange for GPU work (2026-09-28). It also closes plan-164
with the docs, the spec and the final gate.

**The single behavioral outcome:** a frame containing `PolyCurve` or `CurvePolygon`
items — bare, under an `AutoPaint`, or as a `Picture`'s shape — draws on Metal and
Vulkan (no software fallback), within `Tolerance::GPU_CURVE` of the software renderer.
Every scene without a curve is unchanged against `GPU_DEFAULT`.

References:

- plan-164-B §3 (the piece format, the fill test, both distances, the per-contour
  table, the dash table, the hit region, the picture's flattened clip);
- `.ai/canvas-threading.md` — §10 "A group is one instanced draw per node" (the draw
  entry, its offset and its matrix, bound to both stages) and "The per-item parameter
  block travels in a buffer, on both backends" (per-item payloads are frame regions,
  caps are frame sums); "Growing `ITEM_BLOCK_SIZE` moves six things, and three are
  silent"; "A layout constant shared by MFBASIC source and the emitter has no compiler
  between them";
- `.ai/testing-gates.md` — "Canvas reference images: exact for software, tolerance for
  GPU" and "The GPU comparison is against the ORACLE, not a stored picture";
- `tests/common/canvas_image.rs` (`Tolerance`, `Tolerance::GPU_DEFAULT`).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-164-B complete and archived | `ls planning/completed/plan-164-B-*` → match | MET (2026-10-11): `planning/completed/plan-164-B-curves-software.md`, landed in `8d3fdd3a3` |

> **NOTE — the Status column is a snapshot; the Command column is the truth.**

### What the current tree gives this letter (measured 2026-10-11, `af940da94`)

- **The query point reaches an item through two inverses**: the draw entry's (the
  enclosing groups' offset and matrix, plan-172-G/H) and the item's own (`position` +
  `transform`, in `ITEM_OFFSET_TRANSFORM`). Both exist and are shared by every kind; the
  curve arms evaluate at the local point they produce.
- **The item block is 368 bytes**
  (`grep -n 'const ITEM_BLOCK_SIZE' src/codegen/runtime/canvas/mod.rs`), and its regions
  run to `ITEM_OFFSET_GRADIENT_FOCAL` at 352. "Which words are free for the two curve
  kinds" is read from that layout.
- **The Metal renderer lives in `src/target/macos_aarch64/app/metal.rs`**
  (`METAL_SHADER_BODY`, `emit_edge_buffer`, `emit_band_index`,
  `the_draw_frame_slots_do_not_overlap`); the Vulkan one in
  `src/codegen/runtime/canvas/vulkan.rs` (`emit_edge_upload`, `emit_band_index`,
  `the_shared_buffer_holds_every_region`). Both `emit_band_index` already take a
  `BandSet`.
- **One item is one record and one block**, for both shapes (plan-164-B §2). There is
  nothing per-item for the GPU walk to learn.
- **There is one Windows box**, 192.168.1.206, and it has the only real GPU
  (`.ai/remote_systems.md`).

## 1. Goal

> **Read with the Corrections.** What was built differs from this section in four
> places, each recorded there with its evidence: there is no curve region, emitter pair
> or `CANVAS_CURVE_PIECES` (Correction 1); the shader text is one template (3); there is
> one distance function (4); and box 2228 was down, so 2226 stood in (5).

- **A curve region**: a frame-wide region holding the pieces (7 words each), the
  per-item band tables, the per-contour tables and the dash arc-length tables. It is
  filled by new Metal and Vulkan emitters that mirror the edge and band emitters
  (`emit_edge_buffer` / `emit_edge_upload`). There is a frame-sum cap constant
  `CANVAS_CURVE_PIECES` generated into `helper_render.rs`, the way
  `__CANVAS_METAL_MAX_FRAME_EDGES` is, whose value is recorded here after measuring the
  region budget.
- **The band index over pieces.** Each piece's y-range is exact (it is y-monotone, so
  the range is `[min(y0,y2), max(y0,y2)]`). Both `emit_band_index` functions already
  take a `BandSet`; the piece set is a new member of it.
- **Shaders** (`METAL_SHADER_BODY`, `mfb_canvas.frag`, line for line):
  - `pieceCross` (the quadratic root, using the same stable form as software);
  - `pieceDistancePoly` (the closed-form cubic);
  - `pieceDistanceRational` (the fixed search with plan-164-B's recorded step counts);
  - a `POLYCURVE` arm and a `CURVEPOLYGON` arm in `sampleDistance`, sharing the three
    functions above; only the second runs `pieceCross` and signs the distance;
  - the stroke along the pieces, with caps on a `POLYCURVE`, joins (plan-163-B's join
    region) and dashes (the arc-length table).

  All of it is evaluated at the **local** point: the fragment stage has already put the
  pixel through the draw entry's inverse and the item's own. A new arm that reads the
  raw pixel position is the failure `the_metal_fragment_clips_and_samples_in_shape_space`
  exists to catch; extend that test to the piece functions.
- **Item block.** It takes the piece base and count and the band header. Phase 1
  decides whether this needs growth by reading the item-block words free for the two
  curve kinds in the 368-byte layout: neither uses `ITEM_OFFSET_ARC`,
  `ITEM_OFFSET_ARC_CAPS` or `ITEM_OFFSET_ELLIPSE`'s words the way those kinds do. Grow
  the block only if the words do not fit, following `.ai/canvas-threading.md`'s "Growing
  `ITEM_BLOCK_SIZE` moves six things, and three are silent" item by item.
- **Nothing new for an `AutoPaint` or a `Picture`.** An `AutoPaint` over a curve is a
  curve record under another paint: a block like any other. A `Picture` on a
  `CurvePolygon` is two blocks that both already have a GPU path — the image, whose clip
  is the flattened outline in the **edge** region (plan-164-B §3), and the outline, a
  stroke-only curve record in the **curve** region. The work is proving it, and
  measuring what the flattened edges cost against `__CANVAS_METAL_MAX_FRAME_EDGES` and
  its Vulkan twin.
- **`Tolerance::GPU_CURVE`** in `tests/common/canvas_image.rs`. Its per-channel delta
  and pixel fraction are the **measured** worst case over the Phase 2 scenes on Metal
  and on Vulkan, rounded up, and recorded here with the command that produced them.
  It must not exceed a per-channel delta of 8 or 4% of pixels (`GPU_DEFAULT` is 2 and
  2%). If the measurement is above that, the fix is more precision (the double-float
  path the shapes use), not a looser constant.

### Non-goals

- A GPU compute pass or GPU-side reduction. The reduction stays on the CPU, once per
  content change (plan-164-A).
- Any change to shader paths that draw no curve.
- Any change to the draw entry. A curve shape is placed like every other item; the
  16-word entry and its 32-byte per-draw payload are untouched.
- GPU hit-testing. The pointer is answered on the worker, in software (plan-164-B).
- A curved clip in the shaders. A picture's clip stays straight edges; only its
  outline is a true curve.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as
> the work. Use `- [~]` for partial work, with what remains. Mark a moot task
> `- [x] ~~text~~ — moot: <evidence>`. Fill `Commit:` when a phase lands. **An unticked
> box means NOT DONE.**

### Phase 1 — Curve region, emitters, band index

- [x] Decide the item-block words (read the words free for kinds 11 and 12 in the
      current layout and record them here). Grow the block only if necessary.
      — **No growth, and no new word**: the two kinds use the words a polygon uses.
      `misc.x` kind, `misc.w` piece count (header slot 20), `arc.z` first edge slot,
      `arc.y` the fill rule (`emit_item_polygon_rule` now writes it for kind 12 too),
      `arc.w` the cap, `arcCaps` the band header, `strokeStyle` the join base/count and
      dash base/count. `git diff 8d3fdd3a3 -- src/codegen/runtime/canvas/mod.rs | grep -c
      ITEM_BLOCK_SIZE` → 0. See Correction 1 for why nothing more was needed.
- [x] Metal `emit_curve_buffer` and Vulkan `emit_curve_upload` (mirroring the
      edge/band emitters). The frame-sum predicate counts pieces against
      `CANVAS_CURVE_PIECES`. Remove plan-164-B's whole-frame software fallback for the
      two kinds from `__canvas_metalRenderable` and `__canvas_vulkanRenderable`.
      — a piece branch in the existing `emit_edge_buffer` / `emit_edge_upload`, not two
      new emitters, and `2 × pieces` against the existing edge cap, not a new constant
      (Correction 1). The fallback is gone from both predicates:
      `grep -c 'neither GPU backend draws a curve' src/codegen/builtins/canvas/helper_render.rs`
      → 0. `curve_pieces_are_counted_two_slots_each_against_the_edge_region`
      (`rt_canvas_metal.rs`): 512 rings of 256 pieces (262,144 slots, the cap) draw on
      Metal; 513 are declined and the frames are byte-identical.
- [x] ~~Guard tests: extend `the_draw_frame_slots_do_not_overlap` (Metal),
      `the_metal_shader_region_bases_match_the_buffer_layout` (Metal),
      `the_shared_buffer_holds_every_region` and
      `the_pipeline_specializes_every_region_base` (Vulkan) for the curve region.~~
      — moot: there is no curve region, frame slot or region base to guard
      (Correction 1): `git diff 8d3fdd3a3 -- src/target/macos_aarch64/app/metal.rs
      src/codegen/runtime/canvas | grep -cE '^\+.*(const OFF_|_BASE_WORDS: usize)'` → 0.
      The four tests pass unchanged. What the new code CAN get wrong is guarded instead:
      `the_metal_fragment_clips_and_samples_in_shape_space` extended to the curve
      functions (see Phase 2), `the_fragment_shader_holds_the_generated_curve_functions`,
      `the_fragment_shader_draws_curves_from_the_shape_space_point`,
      `the_curve_shader_literals_are_the_oracles_constants`, and the census
      `the_predicates_read_the_edge_count_slot` (6 → 8, two added reads).

Acceptance: pieces and band tables reach the GPU buffers intact.
  Check: `cargo test --bin mfb canvas` → pass (est. 3 min, guess), plus a debug
  readback test that dumps the uploaded piece words for a 4-piece circle and compares
  them with the software tail (est. 3 min, guess).
  Result (2026-10-12): `cargo test --release --bin mfb canvas` → `133 passed; 0 failed`;
  `cargo test --release --bin mfb metal` → `10 passed`. The readback is
  `the_pieces_reach_the_gpu_buffer_with_their_residuals` (Correction 2): circles of four
  arcs and a cubic a few thousandths of a unit across under a 30,000-fold transform,
  worst delta 1 on Metal, lavapipe and Intel; with the emitter's residual words zeroed,
  worst delta 127 over 3,759 pixels.
Commit: ad1daaa64

### Phase 2 — Shaders, tolerance measurement

- [x] The shader functions of §1 in MSL and GLSL, line for line. Regenerate both `.spv`
      on box 2228 (`scripts/regen-spirv.sh`).
      — one template, `shaders/curves.tpl`, expanded into both languages
      (`curve_shader.rs`, `tools/canvas-curve-shader`), so "line for line" is a test and
      not a review (Correction 3). The functions are `pieceCrossing`, `pieceNearest`
      (one, for polynomial and rational pieces alike — Correction 4), `pieceCut`,
      `pieceArc`/`pieceAtArc` and `curveWalk`.
      `MFB_SPIRV_PORT=2226 bash scripts/regen-spirv.sh` → `vert -> 9144 bytes`, `frag ->
      298680 bytes`; box 2228 is down (Correction 5). `git status --short
      src/codegen/runtime/canvas/shaders/` shows `mfb_canvas.vert.spv` unchanged, so 2226's
      glslang (11:12.0.0) writes the blob 2228's did.
      `the_metal_fragment_clips_and_samples_in_shape_space` extended: no curve function
      names the surface position, a piece's points are taken relative to the shape-space
      point in double-float, and both `curveWalk` call sites pass that point.
- [x] Measure: the plan-164-B test scenes as `rt_canvas_metal.rs` rows and as
      `scripts/test-canvas-vulkan.sh` runs on box 2228. Include, beyond B's fill and
      stroke scenes: each shape under its own rotation; each inside a turned and scaled
      group; an `AutoPaint` over a `CurvePolygon` in its hover state (the scripted
      pointer); and a `Picture` on a `CurvePolygon`, stroked, in both `PictureClip`
      modes. Record the worst per-channel delta and pixel fraction per scene. Set
      `Tolerance::GPU_CURVE`, then apply it to the curve rows only.
      — six scenes, each a `rt_canvas_metal.rs` row and a `test-canvas-gpu-rows.sh` row.
      Worst per-channel delta / differing fraction against the software renderer:

      | Scene (`tests/canvas/scenes/`) | Metal | lavapipe, box 2226 | Intel UHD 630, Windows |
      |---|---|---|---|
      | `curves.mfb` | 1 / 0.058% | 1 / 0.145% | 1 / 0.233% |
      | `curves.mfb`, pointer on the pill | 1 / 0.061% | 1 / 0.141% | 1 / 0.163% |
      | `curve_strokes_on_black.mfb` | 1 / 0.623% | 1 / 0.876% | 1 / 0.890% |
      | `curves_placed_on_black.mfb` | 4 / 0.073% | 4 / 0.189% | 4 / 0.230% |
      | `curve_pictures_on_black.mfb` | 2 / 0.035% | 2 / 0.130% | 2 / 0.166% |
      | `curve_residuals_on_black.mfb` | 1 / 0.036% | 1 / 0.116% | 1 / 0.135% |

      Metal: `python3 target/p164/gpucmp.py target/release/mfb <scene>` (a scratch
      comparator: both renders, then every pixel). Vulkan: the `ok worst=… differing=…`
      line of `bash scripts/test-canvas-gpu-rows.sh target/p164/mfb-c2 --target
      linux-aarch64 --box 2226` and `… --target windows-x86_64`.
      The one pixel past 2 is the same pixel on all three: (203, 531) of the placed
      scene, a round dash end of the turned spiral, `071c26` in software and `051822` /
      `061822` on the GPUs — coverage 4 where the oracle has 5.
      **`Tolerance::GPU_CURVE` = 4 levels a channel, 2% of pixels** (the ceiling was 8
      and 4%). Applied to the curve rows only: the seven `assert_metal_draws_within(…,
      Tolerance::GPU_CURVE, …)` rows and the piece-cap row in `rt_canvas_metal.rs`, and
      the six `curve` rows of the script (`rgba_compare.py … curve`). Every other row is
      still `GPU_DEFAULT`.
- [x] Record what a curved picture costs: the flattened edge count of a circle and of a
      rounded-star `CurvePolygon` at three sizes, against the frame edge caps.
      — `python3 target/p164/probe_cost.py` (the real reducer's pieces, flattened by
      `__canvas_cvFlatten`'s rule at 0.05):

      | Shape | Pieces | Clip edges (1 slot each) | Outline (2 slots a piece) |
      |---|---|---|---|
      | circle of four arcs, r = 50 | 4 | 128 | 8 |
      | r = 150 | 4 | 256 | 8 |
      | r = 400 | 4 | 504 | 8 |
      | rounded star of `curves.mfb`, ×0.5 | 14 | 182 | 28 |
      | ×1 | 14 | 304 | 28 |
      | ×3 | 14 | 564 | 28 |

      against 262,144 slots a frame on both backends
      (`METAL_MAX_FRAME_EDGES`, `VULKAN_MAX_FRAME_EDGES`): about 1,900 pictures in
      50-pixel circles, or 440 full-size stars. The clip dominates, and it grows as the
      square root of the size.
- [x] Add a curve scene to `tests/canvas/scenes/`, so `scripts/test-canvas-gpu-rows.sh`
      carries it to every target, and run
      `scripts/test-canvas-gpu-rows.sh <mfb-exe> --target windows-x86_64` against the
      Windows box: the row draws on the GPU and is within `GPU_CURVE`.
      — four scene files added beside `curves.mfb` (letter B's), six rows in the script
      (`curves`, `curveshover`, `curvestrokes`, `curvesplaced`, `curvepictures`,
      `curveresiduals`) plus `vertexrow` (Correction 6). `bash
      scripts/test-canvas-gpu-rows.sh target/p164/mfb-c2 --target windows-x86_64` →
      `bug-688 rows passed on windows-x86_64`: 46 of 46 render rows `ok`, each
      curve row `drawn on Vulkan (gpuFrames=1)` on the Intel UHD 630 and within
      `GPU_CURVE` (the table above).

Acceptance: both GPUs draw both shapes within the measured `GPU_CURVE`, and every
row without a curve still passes `GPU_DEFAULT`.
  Check: `cargo test --test rt_canvas_metal` → pass with `gpuFrames≠0` on the curve
  rows (est. 6 min, guess); the Vulkan runs on 2228 are within `GPU_CURVE` (est. 15
  min, guess; 2228 is the Linux Vulkan lane, `.ai/remote_systems.md`).
  Result (2026-10-12): `MFB_TEST_EXE=target/p164/mfb-c2 cargo test --release --test
  rt_canvas_metal` → `60 passed; 0 failed` (52 before; each new row asserts
  `gpuFrames` is not 0 before it compares). `bash scripts/test-canvas-gpu-rows.sh
  target/p164/mfb-c2 --target linux-aarch64 --box 2226` → `bug-688 rows passed on
  linux-aarch64`, 46 of 46 render rows `ok` — every row without a curve under
  `GPU_DEFAULT`, the six curve rows under `GPU_CURVE` (Correction 5 for the box).
Commit: ad1daaa64

### Phase 3 — Docs, spec, final gate

- [x] `src/docs/spec/app/06_canvas.md`, a new "Curves" section, to
      `.ai/spec-content.md`'s standard:
      - the three types, the validity and joining rules, and that curve points are
        local to `position`;
      - the reduction and `ε`, including that `ε` is in the shape's own space and a
        group's scale multiplies it;
      - the piece format and the two geometry kinds;
      - the fill test, both distances and their step counts;
      - real-corner joins, caps, and where dashes restart;
      - the curve region and `GPU_CURVE`.
      Cite every symbol (`[[path:Symbol]]`).
      — `## Curves`, between *Rendering conventions* and *Named groups*, in eight parts:
      the types with validity and joining; *The reduction*; *The record*; *The fill*;
      *The distance* (one, Correction 4); *The stroke*; the named-type gate; *On the
      GPUs* (the two slots a piece, the template, fp32 and `GPU_CURVE`, a curved
      picture's cost); *The two functions*. The material letters A and B had put under
      *The item unions* moved there. 19 citations in the section
      (`[[` between the two headings).
- [x] The same file's existing sections, reviewed as a whole now that the feature is
      complete: "The item unions" (ten shapes, and the tag list plan-164-A wrote),
      "Hit-testing" → "The region of an item", "AutoPaint", and "A picture is drawn
      inside a shape" (the flattened clip, the true-curve outline, the exact extent).
      — *The item unions* keeps the ten shapes, the tags and one paragraph pointing at
      *Curves*; its "Neither GPU backend draws a curve shape yet" is gone. *Rendering
      conventions* said every shape's GPU coverage is double-float: now "every shape but
      the two curve shapes", with the pointer. *The region of an item* and *A picture is
      drawn inside a shape* read correctly as letter B left them (the curve paragraphs
      cite `CURVE_DRAW` and `CURVE_BUILD`, both still there). *AutoPaint* names no shape
      and needed nothing.
- [x] `.ai/canvas-threading.md`: the curve region and its frame cap; and in §17, that a
      picture on a `CurvePolygon` clips its image to a flattened outline.
      — §10 "The per-item parameter block…": pieces ride the edge region as two runs
      (why the chords come first, the factor of two, the fp32 weight, the template and
      its regeneration, why curve rows have their own tolerance), and the shared-vertex
      rule Correction 6 taught. §16: the NAMED gate and why a record with all-default
      fields needs it. §17: the flattened clip, the true-curve outline, the two kinds of
      slot, the extent.
- [x] Man prose check on the `plan-164-A` types and functions (`.ai/man-content.md`),
      including what a user must be told that the types do not say: scaling a group
      that holds a curve coarsens it where scaling the shape's own `transform` does
      not; a `PolyCurve` is never closed, so a loop wants a `CurvePolygon`; and a
      `Picture` on a `PolyCurve` draws nothing.
      — read as rendered (`mfb man canvas types`); all three are said, in letter A's and
      B's prose: "Enlarging a canvas::Group the path is in does not redraw the curves at
      the new size … scale the shape's own transform to zoom in"; "It is never closed: a
      path that ends where it began has two caps there, not a corner — use
      canvas::CurvePolygon for a closed outline"; and under `Picture`, a
      `canvas::PolyCurve` shape "draws nothing". No plan number, internal name or memory
      word in the three entries; nothing to change.

Acceptance: man and spec render and cite, and the whole tree passes.
  Check: `mfb man canvas types` shows `PolyCurve`, `CurvePolygon` and `BezierCurve`;
  `bash scripts/man-census.sh --memory-scope` → 0 unclassified;
  `bash scripts/man-run-examples.sh canvas --run` → pass;
  `bash scripts/spec-census.sh --citations` and `--links` → 0 dangling;
  `cargo test --bin mfb spec` → pass (est. 8 min, guess).
  Result (2026-10-12): `mfb man canvas types | grep -c 'canvas::BezierCurve\|canvas::PolyCurve\|canvas::CurvePolygon'`
  → entries for all three (lines 912, 945, 995 of the page);
  `bash scripts/man-census.sh --memory-scope` → `unclassified memory-vocabulary hits: 0`;
  `bash scripts/man-run-examples.sh canvas --run` → `examples: 37 built: 37 ran: 37
  not run: 0 failed: 0`; `bash scripts/spec-census.sh --citations` → `MISS-PATH 0
  MISS-LINE 0 MISS-SYMBOL 0`; `--links` → `links=1145 unresolved=0`;
  `cargo test --release --bin mfb spec` → `46 passed; 0 failed`.
  Runtime proof (Validation Plan): letter B's program under `MFB_CANVAS_GPU=1`
  (`python3 target/p164/rundemo.py target/release/mfb gpu`) → `gpuSelected=TRUE
  gpuFrames=1 geoVerifyMismatches=0 auto=3`, `overID` as in software (`[] [star] [star]
  [star] [] [] [] [] [button] [photo::frame]`), the hovered pill in its hover paint. The
  fourteen-cubic spiral at 20× through its OWN transform is smooth on Metal and within
  delta 2 of software; the same spiral at 20× through a group's transform differs from
  it in 1,131 pixels (worst 190) — the documented contrast, the fit's 0.05 px grown to
  1 px.
Commit: ad1daaa64

## Validation Plan

- Tests: Phase 2's curve rows on Metal, Vulkan (2228) and Windows (192.168.1.206).
- Coverage check: the shader paths run only on GPU lanes. Confirm `gpuFrames≠0` on the
  curve rows.
- Runtime proof: plan-164-B's runtime program under `MFB_CANVAS_GPU=1` on macOS,
  screenshotted, plus a zoom animation of a cubic spiral `PolyCurve` showing smooth
  curves at 20× — zoomed through the **shape's own `transform`**, which rebuilds the
  reduction at each scale. The same spiral zoomed through a group's transform is the
  documented contrast, not a failure.
- Final gate (run ONCE, for all of plan-164): `cargo test --no-fail-fast` (est. 40
  min, from plan-159-D's recorded run) and `bash scripts/artifact-gate.sh
  target/release/mfb all` → 0 diffs beyond the set plan-164-A Phase 1 recorded and
  re-baselined for the tag renumbering, and `tests/syntax/app/app-mouse-surface`
  (est. 20 min, guess). A diff outside that set is a bug to localize, not a fixture to
  regenerate.

## Open Decisions

None. `GPU_CURVE` is measured, bounded above by §1's ceiling.

## Corrections

1. **There is no curve region, no curve emitter and no `CANVAS_CURVE_PIECES`** (§1,
   Phase 1). The plan had a new frame region with its own band tables, emitters and cap.
   A piece's chord `x0, y0, x2, y2` is, to the band index, an edge — a y-monotone piece's
   two ends are its whole y-range — so the pieces ride the EDGE region as two runs of
   `count` slots (chords, then `x1, y1, bits(w1), 0`), `emit_band_index` and the item
   block are untouched, and the cap is the edge cap at two slots a piece. The piece
   upload is a branch of `emit_edge_buffer` / `emit_edge_upload`. Evidence it is the
   same contract: 512 rings of 256 pieces (262,144 slots) draw, 513 are declined
   (`curve_pieces_are_counted_two_slots_each_against_the_edge_region`). §1's
   "per-contour tables" do not exist either (plan-164-B Correction: one join record and
   one dash row a piece), and the dash rows are in the dash-word region a polygon's
   dashes use.
2. **The "debug readback" of Phase 1's acceptance is a picture, not a dump.** Nothing
   carries a GPU buffer's words back to a test — the frame's pixels are the only thing
   that comes back, on any of the three backends. A dump would have needed a new native
   path in each emitter to prove what the pixels can prove if the scene is chosen so
   every word matters: `curve_residuals_on_black.mfb` draws shapes ~0.004 units across
   under a 30,000-fold transform, where a coordinate's 16.16 word alone is good to half
   a pixel. Measured sensitivity: with the residual store zeroed in `emit_edge_buffer`,
   worst delta 127 over 3,759 pixels; as shipped, 1. It is stronger than the plan's
   check in one way that matters — it runs on Metal, lavapipe and Intel, so it reads
   back the x86-64 and AArch64 Vulkan uploads as well.
3. **One shader text, not two kept line for line.** `shaders/curves.tpl` is expanded to
   MSL when Metal's source is built and into `mfb_canvas.frag` by
   `tools/canvas-curve-shader/generate.py`;
   `the_fragment_shader_holds_the_generated_curve_functions` fails on a stale copy.
4. **One distance function.** §1 lists `pieceDistancePoly` (a closed-form cubic) and
   `pieceDistanceRational` (a fixed search). plan-164-B replaced both in software with
   root isolation in one function (its Corrections); the shaders have its twin,
   `pieceNearest`, with the same twelve steps.
5. **Box 2228 is down; 2226 stood in.** `ssh -p 2228 test@127.0.0.1 true` → `Connection
   refused`, and the VM would not start from this session (`utmctl` → OSStatus -1743).
   SPIR-V was regenerated with `MFB_SPIRV_PORT=2226` (the vertex blob came out
   byte-identical, so the two boxes' glslang agree), and the Linux Vulkan lane is 2226
   (Debian AArch64, lavapipe) through `scripts/test-canvas-gpu-rows.sh --target
   linux-aarch64 --box 2226`, the script that carries scene files, rather than
   `test-canvas-vulkan.sh`, whose scenes are inline — that one was run on 2226 as well,
   for the shader change's sake (`bash scripts/test-canvas-vulkan.sh target/p164/mfb-c2
   --box 2226 --target linux-aarch64` → `canvas Vulkan runtime tests passed`). The x86-64 Vulkan upload path is
   covered by the Windows box. **Not run: any Linux x86-64 Vulkan lane.**
6. **A polygon bug, found and fixed: a vertex on a pixel-centre row inverted the row.**
   `dfCrossing` (both shaders) rebuilt an edge's end as `ay + vy`, and double-float
   addition does not always return `by`, so the two edges sharing a vertex exactly on a
   pixel row could both count it. Found through the curved picture's flattened clip
   (239 pixels of one row). It is not this plan's: `python3 target/p164/gpucmp.py
   /tmp/p164-base/target/release/mfb tests/canvas/scenes/vertex_on_a_pixel_row.mfb`
   (the compiler of `d2025cd10`, plain polygons) → worst 255, 384 pixels past delta 2;
   with the fix, worst 1, none. `DfEdge` carries `by`. Regression:
   `a_polygon_vertex_on_a_pixel_row_is_counted_once_on_metal` and the `vertexrow` row,
   both under `GPU_DEFAULT`.
7. **The weight is uploaded as fp32 bits, not 16.16.** A weight is not a coordinate; as
   a 16.16 word one past 32,767 would wrap. The shader's arithmetic on it is fp32
   either way.
8. **`emit_item_polygon_rule` writes the rule for kind 12 too.** No task named it; the
   first NonZero ring drawn on Metal was EvenOdd (4,835 pixels past delta 2 in the
   stroke scene) until it did.
9. **Two tests changed, each on evidence.** letter B's
   `canvas_curve_frames_fall_back_to_software_on_the_gpu` pinned the fallback this
   letter removes by instruction; it is now `canvas_curve_frames_are_drawn_on_the_gpu`
   (`gpuFrames` 1, 2, 3, 4). `the_predicates_read_the_edge_count_slot` counts reads of
   header slot 20 in the predicates and went 6 → 8: one added piece-count read in each,
   the same kind of addition its own history records for `PolyLine`.
10. **`GPU_CURVE`'s population bound is `GPU_DEFAULT`'s 2%, not the measured 0.89%
    rounded up.** The fraction of pixels that differ at all is a property of how much
    antialiased edge a scene has, and it is no larger for curves than for the other
    shapes' rows; only the per-channel bound needed loosening, to the measured 4.
11. **A build error met on the way is an open bug, not a new one.** A scratch scene with
    a 1,200-element list literal of `at(0.0 - x, …)` fails to build (`AArch64 branch 'b'
    displacement … exceeds ±128 MiB`), on the baseline compiler too. That is bug-1035
    (emitted code quadratic in a function's fallible statements). Nothing here depends
    on it.

12. **The final gate moved a third fixture, and it was the plan's prediction that was
    wrong.** `bash scripts/artifact-gate.sh target/p164/mfb-c3 all` → `1686 tests, 1865
    build(s), 2350 golden(s) checked, 1 diff(s)`:
    `syntax/app/app-window-surface/app_window_surface.macos-aarch64.app.ncode`, a
    program with no canvas in it. Localized by building it with letter B's compiler and
    this one (`mfb build --app --ncode`) and diffing the two `.ncode` files: ONE line
    differs, the data symbol `_mfb_macapp_metal_shader` — the Metal shader's source text,
    which a macOS window program carries whether or not it draws — 59,920 bytes before
    and 73,360 after. No instruction moved; its `.ir`, `.nir`, `.nplan` and the three
    other targets' checksums are unchanged. The shader text is what this letter changes,
    so the Validation Plan's "0 diffs beyond … `app-mouse-surface`" should have named
    this fixture too. Only that one checksum was re-baselined. (The two canvas fixtures
    moved as predicted and were re-baselined before the gate ran, which is why they
    show as passing in it.)

13. **The final full suite, and what it found.** `MFB_TEST_EXE=target/p164/mfb-c3 cargo
    test --no-fail-fast` (debug profile, about four hours: one compiler unit test,
    `the_whole_corpus_survives_the_top_of_the_dial`, is 401 s in release and was 385 s on
    the commit before this plan, so it is not this plan's) → 344 test binaries `ok`, 7
    failed:
    - **Two censuses, real, and letter A's omission**: three new record types had no
      row in `FIELD_KIND_TABLE` (`self_update.rs`) or in
      `tests/runtime/inplace_self_update/field_kinds.tsv`. Both now have
      `canvas.BezierCurve`, `PolyCurve` and `CurvePolygon`, classed as `Polygon` and
      `PolyLine` are (each holds lists: size-varies). The rows are measured, not
      assumed: `MFB_SELF_UPDATE_FILTER='BezierCurve|PolyCurve|CurvePolygon' cargo test
      --release --test rt_inplace_self_update every_field_kind_meets_its_expectation --
      --ignored` → `1 passed`. The table was NOT regenerated: its generator prints
      `rebuild` where 35 checked-in rows say `arm`, which is open bug-1014, so the three
      rows were added by hand — the generator's own fields, and the committed `PolyLine`
      row's expectations — and the generator gained only their three literals. The
      whole grid run unfiltered fails four pairs, all `http::Route` (S4, S5, S9, S10);
      that is bug-1014's "one row of the drift is measurably wrong", and it fails the
      same way with the compiler of the commit before this plan
      (`MFB_SELF_UPDATE_FILTER='http::Route' MFB_TEST_EXE=/tmp/p164-base/target/release/mfb
      …` → `4 of 15 case/site pair(s) failed`).
    - **Four `cli_repo_*` binaries, my harness**: `mfb-repo missing at
      target/p164/mfb-repo` — the suite looks for the repository server beside the
      compiler, and the frozen copy had none. With it there, 29 of 29 pass.
    - **`rt_canvas_pointer`, load**: `the_graphics_thread_reads_each_scripted_pointer_state`
      read a scripted pointer state late while the whole suite ran beside it; alone, 2
      of 2 pass, twice.
    Re-run of all seven: `cargo test --no-fail-fast --test cli_repo_auth --test
    cli_repo_governance --test cli_repo_install --test cli_repo_publish --test
    rt_canvas_pointer --test inplace_self_update_census` → six `ok` (4, 9, 10, 6, 2, 2
    passed); `cargo test --bin mfb field_kind_census` → `1 passed`.

## Summary

The risk is fp32 precision in the per-piece quadratic root and the rational search on
the GPU. It is measured into `GPU_CURVE`, with a hard ceiling that forces a precision
fix rather than a looser test. Two things are worth watching beside it: the pieces are
read at a point that has been through two inverses, and a turned, scaled group is where
a shader arm that skipped one shows; and a curved picture is the one scene that puts a
curve's cost in two regions at once.
