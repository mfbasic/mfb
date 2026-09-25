# plan-154-B: `canvas::PolyLine` — Metal

Last updated: 2026-09-24
Effort: medium (1h–2h)
Depends on: plan-154-A

This sub-plan makes Metal draw `canvas::PolyLine` itself instead of falling
back to software. A macOS frame containing polylines then stays on the GPU,
with coverage matching the plan-154-A oracle within `Tolerance::GPU_DEFAULT`.

**Correct behavior.** Take the plan-154-A golden scene, rendered with
`MFB_CANVAS_GPU=1`:

- it matches the software frame within `Tolerance::GPU_DEFAULT` (max channel
  delta 2, at most 2% of pixels differing; `tests/common/canvas_image.rs`);
- the stats line shows the frame drawn on the GPU (`gpuFrames` equals `frames`).

References: plan-154-A (contract and geometry layout), `.ai/canvas-threading.md`
§10 (per-item GPU block, per-frame regions), `mfb spec app canvas` ("What Metal
declines").

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-154-A complete | `ls planning/plan-154-A-*` → no match (archived) | MET (2026-09-25: no match; archived to `planning/completed/` in ef4dbe28e) |

If plan-154-A is not complete, this plan cannot start, full stop.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**

## 1. Goal

- `__canvas_metalRenderable` no longer declines PolyLine.
- The Metal shader draws it.
- The GPU-vs-oracle test for the polyline scene passes.

### Non-goals

- **The item block does not grow.** `ITEM_BLOCK_SIZE` stays 208 bytes (Corrections B1); the
  PolyLine header must fit the words the polygon kind already uses. Growing
  the block moves six things, three of them silently (`.ai/canvas-threading.md`).
- **Metal frame caps are unchanged.** PolyLine edges count against
  `METAL_MAX_FRAME_EDGES` = 262,144 (`src/codegen/runtime/canvas/mod.rs`).
- **Vulkan is untouched** (plan-154-C).

## 2. Current State

The Metal shader source is `METAL_SHADER_BODY` in
`src/target/macos_aarch64/app/metal.rs`. It dispatches on the geometry kind
by `item.misc.x`: `== 2` is a segment, and the polygon kind uses
`edgeDistance` over a band index.

- Polygon edges are uploaded by `emit_edge_buffer`.
- The band index is built by `emit_band_index`.
- Per-item blocks are written by `emit_item_block`.
- Draws are issued by `emit_draw_list_pass`.
- `GEO_KIND_POLYGON` is branched on in `metal.rs` in two places: the kind
  constant, and the edge upload.
- PolyLine's tail uses the polygon edge layout (verified in plan-154-A §2), so
  the edge upload can carry it as is.

### Measured populations

| What | Count | Command |
|---|---|---|
| `GEO_KIND_POLYGON` sites in metal.rs | 2: the import (line 68) and `emit_edge_buffer`'s kind test (line 4343), plus the literal `item.misc.x != 4` in the shader's `polygonBand` | `grep -n "GEO_KIND_POLYGON" src/target/macos_aarch64/app/metal.rs` |

## 3. Design Overview

1. **Upload.** Upload PolyLine edges through the polygon edge region. At each
   `GEO_KIND_POLYGON` upload site, extend the test to `GEO_KIND_POLYGON` or
   `GEO_KIND_POLYLINE`.
2. **Band index.** Build a band index for it too. A long path benefits from
   it just as a polygon does.
3. **Shader.** Add a `misc.x == GEO_KIND_POLYLINE` branch. It takes the
   minimum unsigned segment distance over the banded edges and tracks `(i, t)`,
   and applies butt caps at the first and last live segments. The colour is the
   existing gradient lookup, evaluated at the path parameter
   `(i + t) / segments` instead of an axis parameter: the per-point colours
   are already the record's gradient stops, and `emit_gradient_buffer` uploads
   any record's stops (plan-154-A Corrections A1). It is the same arithmetic as
   the oracle in plan-154-A, in fp32.
3a. **First and last live segments as whole numbers.** `emit_item_block` writes
   the gradient axis slots, which carry them, as 16.16, and 16.16 overflows past
   32,767 segments. For a polyline the emitter writes those two words as plain
   integers.
3b. **Band reach.** `emit_band_index` measures its reach from slot 7, the stroke
   half-width, which a polyline sets to -1: its half-width is in the radius
   slot 6. The reach uses `max(slot 7, slot 6, 0)`. Slot 6 is 0 for every polygon
   (the polygon header never writes it), so a polygon's reach is unchanged.
4. **Stop declining.** Remove PolyLine from `__canvas_metalRenderable`'s
   decline.

**Risk.** fp32 against the f64 oracle at near-black edge pixels is bug-687's
class. If the tolerance fails only on dark antialiased pixels, check it
against bug-687's analysis before changing anything, and never loosen
`GPU_DEFAULT`.

**Gate class:** new GPU behavior. The gate is the oracle comparison. The
existing Metal oracle tests must pass unchanged.

## Phases

> **NOTE — keep the checkboxes current as you go.** **An unticked box means NOT DONE.**

### Phase 1 — Metal draws PolyLine

- [x] Measure the `GEO_KIND_POLYGON` sites, and record the count in §2
      (2 sites plus the shader's literal `4` in `polygonBand`).
- [x] `metal.rs`: upload the edges and build the band index for
      `GEO_KIND_POLYLINE`. `emit_edge_buffer` accepts both kinds and writes a
      polyline's first/last segments as whole numbers (step 3a).
      `emit_band_index` takes its reach from `max(slot 7, slot 6, 0)` (step 3b).
- [x] Add the shader branch: `polylineScan`, a `misc.x == 10` arm in
      `geoDistance`, `polygonBand` for kind 10, and `gradientColour` split into
      axis → `gradientAt(t)` so the polyline looks up its path parameter.
- [x] `helper_render.rs`: stop declining in `__canvas_metalRenderable`, and
      count a polyline's segments against `METAL_MAX_FRAME_EDGES`.
- [x] `tests/canvas/rt_canvas_metal.rs`: replace plan-154-A's fallback test
      with a GPU-vs-oracle comparison of the polyline scene, asserting
      `gpuFrames == frames`. Add PolyLine to
      `the_full_primitive_set_matches_the_software_oracle`.
      `polylines_match_the_software_oracle_on_metal` → ok. Its scene gained a
      41-point path, so a polyline takes the band index (`BAND_MIN_EDGES` = 32).
- [x] (added) `tests/canvas/rt_canvas_golden.rs`: the plan-154-A golden scene
      itself, on the GPU against the committed reference —
      `the_gpu_draws_the_polyline_scene_the_reference_shows` → ok. §"Correct
      behavior" names that scene, which the Metal suite's scene is not.
- [x] Spec, `06_canvas.md`: remove the Metal half of the fallback sentence,
      and cite the shader branch (`METAL_SHADER_BODY`, `emit_edge_buffer`,
      `HEADER_POLYLINE_FIRST`). `scripts/spec-census.sh --citations` →
      `MISS-SYMBOL 0`.

Acceptance: the polyline scene matches the oracle within `GPU_DEFAULT` on the
GPU, and every existing Metal test passes.
  Check: `cargo test --test rt_canvas_metal` → pass (est. 6 min; needs a Mac with Metal, the dev host).
  Result: `cargo test --test rt_canvas_metal --test rt_canvas_golden` → 20 and
  22 passed. The band-reach change was predicted neutral for polygons, and every
  polygon GPU test is unchanged (`forty_thousand_polygon_edges_draw_on_metal`,
  `a_sixty_four_thousand_edge_polygon_draws_on_metal`, the group and primitive
  scenes). `cargo test --bin mfb canvas` → 97 passed.
Commit: e2862f3e9

## Validation Plan

- Tests: `rt_canvas_metal` (the new comparison, plus the full-primitive row).
- Runtime proof: the stats line from the test, showing the frame on the GPU.
- Doc sync: `spec app canvas`.
- Final gate: once, at the end of plan-154-D.

## Open Decisions

- None beyond plan-154-A's.

## Corrections

- **B1 (2026-09-24): `ITEM_BLOCK_SIZE` is 208, not 192.** plan-116-F added the
  gradient axis `ivec4` (`grep -n "pub(crate) const ITEM_BLOCK_SIZE" src/codegen/runtime/canvas/mod.rs`
  → 208). The non-goal is unchanged: the block does not grow.
- **B2 (2026-09-24): colours need no new region.** plan-154-A's colour list
  rides the gradient stops (plan-154-A Corrections A1), so step 3 reads the stop
  region the gradient path already uploads. Steps 3a and 3b were added:
  both are measured facts about `emit_item_block` and `emit_band_index` that a
  polyline hits and a polygon does not.

## Summary

This mirrors the polygon path, and the risk is fp32 tolerance on dark edges.
The item block layout does not change.
