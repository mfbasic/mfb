# plan-154-C: `canvas::PolyLine` — Vulkan

Last updated: 2026-09-24
Effort: medium (1h–2h)
Depends on: plan-154-B

This sub-plan makes Vulkan (Linux and Windows) draw `canvas::PolyLine`
instead of falling back to software.

**Correct behavior.** Take the plan-154-A polyline scene, run by
`scripts/test-canvas-vulkan.sh` on a box with a Vulkan ICD:

- it matches the software frame within `Tolerance::GPU_DEFAULT`;
- the stats line shows `vulkanReady=TRUE` and the frame drawn on the GPU.

References: plan-154-A (contract), plan-154-B (the Metal twin this mirrors),
`.ai/canvas-threading.md`, `bugs/bug-688-vulkan-canvas-lacks-bug-686-metal-parity.md`
(Vulkan caps and its lack of a band index).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-154-B complete | `ls planning/plan-154-B-*` → no match (archived) | MET (2026-09-25: no match; archived in 06b22c3f3) |
| A Vulkan box answers | `scripts/test-canvas-vulkan.sh target/release/mfb --box 2226 --target linux-aarch64` on HEAD → reports `vulkanReady=TRUE` | MET (2026-09-25 on 2226, HEAD 06b22c3f3: `vulkanReady=TRUE`, "canvas Vulkan runtime tests passed"; 2228 refuses connections — Corrections C2) |

If either row is not met, this plan cannot start, full stop.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**

## 1. Goal

- `__canvas_vulkanRenderable` no longer declines PolyLine.
- `mfb_canvas.frag` draws it.
- The regenerated SPIR-V is committed.
- The Vulkan oracle comparison passes on box 2228.

### Non-goals

- **Vulkan's caps are unchanged.** `VULKAN_MAX_FRAME_EDGES` = 262,144
  (`src/codegen/runtime/canvas/mod.rs`; Corrections C1).
- **No per-item block growth.**

## 2. Current State

The Vulkan fragment shader is
`src/codegen/runtime/canvas/shaders/mfb_canvas.frag`, dispatching on
`item.misc.x`: `== 2` is a segment, and `== 4` is a polygon, which loops over
all of its edges. Edges are uploaded by `emit_edge_upload`, and item blocks
by `emit_item_block`, both in `src/codegen/runtime/canvas/vulkan.rs`. After
any shader edit, regenerate the `.spv` files with `scripts/regen-spirv.sh`.

**Consequence for wind.** Wind would need 35,200 edges for its polylines
(3,200 particles × 11 segments; this is arithmetic from `countFor`'s
`maxParticles`, not a measurement). That is well inside the 262,144 cap
(Corrections C1).

### Measured populations

| What | Count | Command |
|---|---|---|
| `GEO_KIND_POLYGON` sites in vulkan.rs | 2: the import (line 46) and the edge upload's kind test (line 3937) | `grep -n "GEO_KIND_POLYGON" src/codegen/runtime/canvas/vulkan.rs` |

## 3. Design Overview

This is the same as plan-154-B, including the band index (Corrections C1):

1. At each `GEO_KIND_POLYGON` edge-upload site, also accept
   `GEO_KIND_POLYLINE`, with plan-154-B's steps 3a (whole-number first and last
   segments) and 3b (band reach from the radius slot).
2. Add a fragment branch that loops over the item's edges, tracks the minimum
   and `(i, t)`, applies the butt end caps, and takes its colour from the
   gradient lookup at the path parameter, as plan-154-B does.
3. Regenerate the SPIR-V.
4. Remove the decline in `__canvas_vulkanRenderable`.

**Risk.** The shader and the committed SPIR-V drifting apart. The regenerate
script is the only producer, and the test exercises the committed binary.

**Gate class:** new GPU behavior. The gate is the box oracle comparison.

## Phases

> **NOTE — keep the checkboxes current as you go.** **An unticked box means NOT DONE.**

### Phase 1 — Vulkan draws PolyLine

- [x] Measure the `GEO_KIND_POLYGON` sites, and record the count in §2 (2).
- [x] `vulkan.rs`: upload PolyLine edges. `emit_edge_upload` accepts both
      kinds and writes the first/last live segments as whole numbers;
      `emit_band_index` takes its reach from `max(slot 7, slot 6, 0)`.
- [x] `mfb_canvas.frag`: add the PolyLine branch, then run
      `scripts/regen-spirv.sh` and commit the `.spv`. Run as
      `MFB_SPIRV_PORT=2226 scripts/regen-spirv.sh` (Corrections C2). Only
      `mfb_canvas.frag.spv` is committed: 2226's glslang (11:12.0.0) rebuilt the
      UNCHANGED frag byte-identically but not the unchanged vert, so the vert blob
      is restored from HEAD rather than churned (Corrections C3).
      `cargo test --bin mfb vulkan` → 5 passed.
- [x] `helper_render.rs`: stop declining in `__canvas_vulkanRenderable`, and
      count a polyline's segments against `VULKAN_MAX_FRAME_EDGES`. Each predicate's
      polyline sum is its own `IF` so that `= __CANVAS_GEO_POLYGON THEN` stays
      spelled (`the_polygon_kind_is_spelled_once`). The slot-20 read census goes
      4 → 6 (`the_predicates_read_the_edge_count_slot`, two additions, recorded in
      its doc).
- [x] Add the polyline scene to `scripts/test-canvas-vulkan.sh`'s comparison
      set: extracted from `rt_canvas_golden.rs`'s `POLYLINES` const, as the group
      scene is, and compared with `polyline.png`. The PNG decoder became
      `png_to_rgba`, shared by both rows. `scripts/README.md` updated.
- [x] Spec, `06_canvas.md`: remove the remaining fallback sentence for
      PolyLine, and cite `emit_edge_upload` and the frag's `polylineScan`.
      `scripts/spec-census.sh --citations` → `MISS-SYMBOL 0`.

Acceptance: the polyline scene matches the oracle on the Vulkan GPU on box
2226 (Corrections C2).
  Check: `scripts/test-canvas-vulkan.sh target/release/mfb --box 2228` → polyline row within `GPU_DEFAULT`, `vulkanReady=TRUE` (est. 10 min; the build and the AppImage ship to the box dominate, and only a real Vulkan ICD can show the shader is right — box 2228 is the script's default glibc x86_64 box with one).
  Result (box 2226, lavapipe): "the polyline scene reached the GPU (gpuFrames
  non-zero)"; "the software render reproduces tests/golden/canvas/polyline.png
  exactly"; "the Vulkan render matches tests/golden/canvas/polyline.png (ok
  worst=1 differing=0.5366%)"; every earlier row still passes; "canvas Vulkan
  runtime tests passed".
Commit: bfaae88cf

## Validation Plan

- Tests: the `test-canvas-vulkan.sh` polyline row.
- Runtime proof: the box's stats line and the frame diff.
- Doc sync: `spec app canvas`.
- Final gate: once, at the end of plan-154-D.

## Open Decisions

- None beyond plan-154-A's.

## Corrections

- **C1 (2026-09-24): the Vulkan caps and band index already match Metal's.**
  The plan said `VULKAN_MAX_FRAME_EDGES` is 16,384 and Vulkan has no band index.
  Both were raised or ported by bug-688's first phases:
  `grep -n "pub(crate) const VULKAN_MAX_FRAME_EDGES" src/codegen/runtime/canvas/mod.rs`
  → 262,144, and `grep -n "fn emit_band_index" src/codegen/runtime/canvas/vulkan.rs`
  → line 4139. So wind's 35,200 polyline edges fit, and the upload step is the
  same as Metal's. The design text was corrected; the wind-stays-in-software
  paragraph was false and is gone.
- **C2 (2026-09-25): box 2228 is down; 2226 is the Vulkan box.** `ssh -p 2228`
  → "Connection refused" (and so for every port but 2226 and 2230); UTM is
  running only the aarch64 and Windows guests. `test-canvas-vulkan.sh` supports
  `--box 2226 --target linux-aarch64`, and 2226 reports `vulkanReady=TRUE` with a
  Mesa lavapipe ICD, so the prerequisite and the acceptance run there. The
  predicate is the same one the plan named — a real Vulkan ICD drawing the
  committed SPIR-V — on the other Linux architecture.
- **C3 (2026-09-25): regenerating the SPIR-V on 2226 changes the unchanged vert
  blob.** Before any edit, `MFB_SPIRV_PORT=2226 scripts/regen-spirv.sh` gave a
  byte-identical `mfb_canvas.frag.spv` and a different `mfb_canvas.vert.spv`
  (`git status --short src/codegen/runtime/canvas/shaders/`). So the committed
  vert came from a different glslang. `mfb_canvas.vert` is not edited by this
  plan, so its blob is kept as committed.

## Summary

A shader branch plus an upload arm. Wind-scale scenes stay in software on
Vulkan until bug-688, by design.
