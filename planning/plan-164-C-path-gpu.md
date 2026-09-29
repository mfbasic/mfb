# plan-164-C: Metal and Vulkan draw `canvas::Path` per pixel

Last updated: 2026-09-28
Effort: large (3h–1d)
Depends on: plan-164-B

This letter moves `Path` drawing onto the GPU. The rational-quadratic pieces are
uploaded to a frame **curve region** with a per-item **band index**, and both shaders
evaluate the fill test and the distance per pixel, line for line with the software
renderer. The GPU is compared against the software renderer within a **curve
tolerance**, measured here and recorded as a constant. The user accepted a looser
tolerance for curves in exchange for GPU work (2026-09-28). It also closes plan-164
with the docs, the spec and the final gate.

**The single behavioral outcome:** a frame containing `Path` items draws on Metal and
Vulkan (no software fallback), within `Tolerance::GPU_CURVE` of the software renderer.
Every non-`Path` scene is unchanged against `GPU_DEFAULT`.

References:

- plan-164-B §3 (the piece format, the fill test, both distances, sub-records, the
  dash table);
- `.ai/canvas-threading.md` :517–574 (per-item payloads are frame regions, caps are
  frame sums) and :1003–1040 (item-block growth, if needed);
- `.ai/testing-gates.md` :235 and :309;
- `tests/common/canvas_image.rs` (`Tolerance`, `GPU_DEFAULT` near :139).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-164-B complete and archived | `ls planning/completed/plan-164-B-*` → match | NOT MET (2026-09-28) |

> **NOTE — the Status column is a snapshot; the Command column is the truth.**

## 1. Goal

- **A curve region**: a frame-wide region holding the pieces (7 words each), the
  per-item band tables, the per-subpath tables and the dash arc-length tables. It is
  filled by new Metal and Vulkan emitters that mirror the edge and band emitters.
  There is a frame-sum cap constant `CANVAS_CURVE_PIECES` generated into
  `helper_render.rs`, whose value is recorded here after measuring the region budget.
- **The band index over pieces.** Each piece's y-range is exact (it is y-monotone, so
  the range is `[min(y0,y2), max(y0,y2)]`). The existing `emit_band_index` is
  parameterised by the element type.
- **Shaders** (`METAL_SHADER_BODY`, `mfb_canvas.frag`, line for line):
  - `pieceCross` (the quadratic root, using the same stable form as software);
  - `pieceDistancePoly` (the closed-form cubic);
  - `pieceDistanceRational` (the fixed search with plan-164-B's recorded step counts);
  - a `PATH` arm in `sampleDistance`;
  - the stroke along stroke pieces, with caps and joins (plan-163-B's join region) and
    dashes (the arc-length table).
- **Sub-records** draw as consecutive instanced items. Each is its own item block, so
  no per-item branching on paint count is needed.
- **Item block.** It takes the piece base and count and the band header. Phase 1
  decides whether this needs growth by reading the item-block words free for kind 11:
  a `PATH` does not use `arc.*` or `gradient.*`'s ellipse words the way an arc does.
  Grow the block only if the words do not fit, following `.ai/canvas-threading.md:1003–1040`
  item by item.
- **`Tolerance::GPU_CURVE`** in `tests/common/canvas_image.rs`. Its per-channel delta
  and pixel fraction are the **measured** worst case over the Phase 2 scenes on Metal
  and on Vulkan, rounded up, and recorded here with the command that produced them.
  It must not exceed a per-channel delta of 8 or 4% of pixels. If the measurement is
  above that, the fix is more precision (the double-float path the shapes use), not a
  looser constant.

### Non-goals

- A GPU compute pass or GPU-side reduction. The reduction stays on the CPU, once per
  content change (plan-164-A).
- Any change to non-`Path` shader paths.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as
> the work. Use `- [~]` for partial work, with what remains. Mark a moot task
> `- [x] ~~text~~ — moot: <evidence>`. Fill `Commit:` when a phase lands. **An unticked
> box means NOT DONE.**

### Phase 1 — Curve region, emitters, band index

- [ ] Decide the item-block words (read the kind-11 free words and record them here).
      Grow the block only if necessary.
- [ ] Metal `emit_curve_buffer` and Vulkan `emit_curve_upload` (mirroring the
      edge/band emitters). The frame-sum predicate counts pieces against
      `CANVAS_CURVE_PIECES`. Remove plan-164-B's whole-frame software fallback for
      `Path`.
- [ ] Guard tests: extend `the_draw_frame_slots_do_not_overlap` and
      `the_shared_buffer_holds_every_region` for the curve region.

Acceptance: pieces and band tables reach the GPU buffers intact.
  Check: `cargo test --bin mfb canvas` → pass (est. 3 min, guess), plus a debug
  readback test that dumps the uploaded piece words for a 4-piece circle and compares
  them with the software tail (est. 3 min, guess).
Commit: —

### Phase 2 — Shaders, tolerance measurement

- [ ] The shader functions of §1 in MSL and GLSL, line for line. Regenerate both `.spv`
      on box 2228 (`scripts/regen-spirv.sh`).
- [ ] Measure: the plan-164-B test scenes as `rt_canvas_metal.rs` rows and as
      `scripts/test-canvas-vulkan.sh` runs on box 2228. Record the worst per-channel
      delta and pixel fraction per scene. Set `Tolerance::GPU_CURVE`, then apply it to
      the `Path` rows only.
- [ ] Windows GPU: `scripts/test-canvas-gpu-rows.sh <circle scene> --target
      windows-x86_64` on box 206 (the only real GPU) is within `GPU_CURVE`.

Acceptance: both GPUs draw `Path` within the measured `GPU_CURVE`, and every
non-`Path` row still passes `GPU_DEFAULT`.
  Check: `cargo test --test rt_canvas_metal` → pass with `gpuFrames≠0` on the `Path`
  rows (est. 6 min, guess); the Vulkan runs on 2228 are within `GPU_CURVE` (est. 15
  min, guess; 2228 is the Linux Vulkan lane, `.ai/remote_systems.md:112–137`).
Commit: —

### Phase 3 — Docs, spec, final gate

- [ ] `06_canvas.md`, a new "Paths and curves" section:
      - the types and validity rules;
      - the reduction and `ε`;
      - the piece format;
      - the fill test, both distances and their step counts;
      - real-corner joins;
      - sub-records and draw order;
      - the curve region and `GPU_CURVE`.
      Cite every symbol (`[[path:Symbol]]`).
- [ ] `.ai/canvas-threading.md`: the curve region, sub-records (one item → several
      records) and the frame cap.
- [ ] Man prose check on the `plan-164-A` types and functions (`.ai/man-content.md`).

Acceptance: man and spec render and cite, and the whole tree passes.
  Check: `mfb man canvas types` shows `Path`, `Subpath` and `BezierCurve`;
  `bash scripts/man-census.sh --memory-scope` → 0 unclassified;
  `bash scripts/man-run-examples.sh canvas --run` → pass;
  `bash scripts/spec-census.sh --citations` and `--links` → 0 dangling;
  `cargo test --bin mfb spec` → pass (est. 8 min, guess).
Commit: —

## Validation Plan

- Tests: Phase 2's `Path` rows on Metal, Vulkan (2228) and Windows (206).
- Coverage check: the shader paths run only on GPU lanes. Confirm `gpuFrames≠0` on the
  `Path` rows.
- Runtime proof: plan-164-B's runtime program under `MFB_CANVAS_GPU=1` on macOS,
  screenshotted, plus a zoom animation of a cubic spiral showing smooth curves at 20×.
- Final gate (run ONCE, for all of plan-164): `cargo test --no-fail-fast` (est. 40
  min, from plan-159-D's recorded run) and `bash scripts/artifact-gate.sh
  target/release/mfb all` → 0 diffs outside `tests/syntax/app/app-mouse-surface` (est.
  20 min).

## Open Decisions

None. `GPU_CURVE` is measured, bounded above by §1's ceiling.

## Corrections

(none yet)

## Summary

The risk is fp32 precision in the per-piece quadratic root and the rational search on
the GPU. It is measured into `GPU_CURVE`, with a hard ceiling that forces a precision
fix rather than a looser test.
