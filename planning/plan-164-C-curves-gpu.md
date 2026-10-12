# plan-164-C: Metal and Vulkan draw `canvas::PolyCurve` and `canvas::CurvePolygon` per pixel

Last updated: 2026-10-11
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

- [ ] Decide the item-block words (read the words free for kinds 11 and 12 in the
      current layout and record them here). Grow the block only if necessary.
- [ ] Metal `emit_curve_buffer` and Vulkan `emit_curve_upload` (mirroring the
      edge/band emitters). The frame-sum predicate counts pieces against
      `CANVAS_CURVE_PIECES`. Remove plan-164-B's whole-frame software fallback for the
      two kinds from `__canvas_metalRenderable` and `__canvas_vulkanRenderable`.
- [ ] Guard tests: extend `the_draw_frame_slots_do_not_overlap` (Metal),
      `the_metal_shader_region_bases_match_the_buffer_layout` (Metal),
      `the_shared_buffer_holds_every_region` and
      `the_pipeline_specializes_every_region_base` (Vulkan) for the curve region.

Acceptance: pieces and band tables reach the GPU buffers intact.
  Check: `cargo test --bin mfb canvas` → pass (est. 3 min, guess), plus a debug
  readback test that dumps the uploaded piece words for a 4-piece circle and compares
  them with the software tail (est. 3 min, guess).
Commit: —

### Phase 2 — Shaders, tolerance measurement

- [ ] The shader functions of §1 in MSL and GLSL, line for line. Regenerate both `.spv`
      on box 2228 (`scripts/regen-spirv.sh`).
- [ ] Measure: the plan-164-B test scenes as `rt_canvas_metal.rs` rows and as
      `scripts/test-canvas-vulkan.sh` runs on box 2228. Include, beyond B's fill and
      stroke scenes: each shape under its own rotation; each inside a turned and scaled
      group; an `AutoPaint` over a `CurvePolygon` in its hover state (the scripted
      pointer); and a `Picture` on a `CurvePolygon`, stroked, in both `PictureClip`
      modes. Record the worst per-channel delta and pixel fraction per scene. Set
      `Tolerance::GPU_CURVE`, then apply it to the curve rows only.
- [ ] Record what a curved picture costs: the flattened edge count of a circle and of a
      rounded-star `CurvePolygon` at three sizes, against the frame edge caps.
- [ ] Add a curve scene to `tests/canvas/scenes/`, so `scripts/test-canvas-gpu-rows.sh`
      carries it to every target, and run
      `scripts/test-canvas-gpu-rows.sh <mfb-exe> --target windows-x86_64` against the
      Windows box: the row draws on the GPU and is within `GPU_CURVE`.

Acceptance: both GPUs draw both shapes within the measured `GPU_CURVE`, and every
row without a curve still passes `GPU_DEFAULT`.
  Check: `cargo test --test rt_canvas_metal` → pass with `gpuFrames≠0` on the curve
  rows (est. 6 min, guess); the Vulkan runs on 2228 are within `GPU_CURVE` (est. 15
  min, guess; 2228 is the Linux Vulkan lane, `.ai/remote_systems.md`).
Commit: —

### Phase 3 — Docs, spec, final gate

- [ ] `src/docs/spec/app/06_canvas.md`, a new "Curves" section, to
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
- [ ] The same file's existing sections, reviewed as a whole now that the feature is
      complete: "The item unions" (ten shapes, and the tag list plan-164-A wrote),
      "Hit-testing" → "The region of an item", "AutoPaint", and "A picture is drawn
      inside a shape" (the flattened clip, the true-curve outline, the exact extent).
- [ ] `.ai/canvas-threading.md`: the curve region and its frame cap; and in §17, that a
      picture on a `CurvePolygon` clips its image to a flattened outline.
- [ ] Man prose check on the `plan-164-A` types and functions (`.ai/man-content.md`),
      including what a user must be told that the types do not say: scaling a group
      that holds a curve coarsens it where scaling the shape's own `transform` does
      not; a `PolyCurve` is never closed, so a loop wants a `CurvePolygon`; and a
      `Picture` on a `PolyCurve` draws nothing.

Acceptance: man and spec render and cite, and the whole tree passes.
  Check: `mfb man canvas types` shows `PolyCurve`, `CurvePolygon` and `BezierCurve`;
  `bash scripts/man-census.sh --memory-scope` → 0 unclassified;
  `bash scripts/man-run-examples.sh canvas --run` → pass;
  `bash scripts/spec-census.sh --citations` and `--links` → 0 dangling;
  `cargo test --bin mfb spec` → pass (est. 8 min, guess).
Commit: —

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

(none yet)

## Summary

The risk is fp32 precision in the per-piece quadratic root and the rational search on
the GPU. It is measured into `GPU_CURVE`, with a hard ceiling that forces a precision
fix rather than a looser test. Two things are worth watching beside it: the pieces are
read at a point that has been through two inverses, and a turned, scaled group is where
a shader arm that skipped one shows; and a curved picture is the one scene that puts a
curve's cost in two regions at once.
