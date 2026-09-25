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
| plan-154-B complete | `ls planning/plan-154-B-*` → no match (archived) | NOT MET (2026-09-24) |
| A Vulkan box answers | `scripts/test-canvas-vulkan.sh target/release/mfb --box 2228` on HEAD → reports `vulkanReady=TRUE` | UNMEASURED (2026-09-24) |

If either row is not met, this plan cannot start, full stop.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**

## 1. Goal

- `__canvas_vulkanRenderable` no longer declines PolyLine.
- `mfb_canvas.frag` draws it.
- The regenerated SPIR-V is committed.
- The Vulkan oracle comparison passes on box 2228.

### Non-goals

- **Vulkan's caps are unchanged.** `VULKAN_MAX_FRAME_EDGES` = 16,384
  (`src/codegen/runtime/canvas/mod.rs`). Raising it, and adding a band index,
  are bug-688's work.
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
`maxParticles`, not a measurement). That is already past the 16,384 cap, and
the comment on `VULKAN_MAX_FRAME_EDGES` records that wind's coastline alone
exceeds it. So wind frames on Vulkan stay in software until bug-688 lands.
That is correct, and it is outside this sub-plan.

### Measured populations

| What | Count | Command |
|---|---|---|
| `GEO_KIND_POLYGON` sites in vulkan.rs | UNMEASURED — first task | `grep -n "GEO_KIND_POLYGON" src/codegen/runtime/canvas/vulkan.rs` |

## 3. Design Overview

This is the same as plan-154-B, but without a band index:

1. At each `GEO_KIND_POLYGON` edge-upload site, also accept
   `GEO_KIND_POLYLINE`.
2. Add a fragment branch that loops over the item's edges, tracks the minimum
   and `(i, t)`, and applies the butt end caps and the fade.
3. Regenerate the SPIR-V.
4. Remove the decline in `__canvas_vulkanRenderable`.

**Risk.** The shader and the committed SPIR-V drifting apart. The regenerate
script is the only producer, and the test exercises the committed binary.

**Gate class:** new GPU behavior. The gate is the box oracle comparison.

## Phases

> **NOTE — keep the checkboxes current as you go.** **An unticked box means NOT DONE.**

### Phase 1 — Vulkan draws PolyLine

- [ ] Measure the `GEO_KIND_POLYGON` sites, and record the count in §2.
- [ ] `vulkan.rs`: upload PolyLine edges.
- [ ] `mfb_canvas.frag`: add the PolyLine branch, then run
      `scripts/regen-spirv.sh` and commit the `.spv`.
- [ ] `helper_render.rs`: stop declining in `__canvas_vulkanRenderable`.
- [ ] Add the polyline scene to `scripts/test-canvas-vulkan.sh`'s comparison
      set.
- [ ] Spec, `06_canvas.md`: remove the remaining fallback sentence for
      PolyLine.

Acceptance: the polyline scene matches the oracle on the Vulkan GPU on box
2228.
  Check: `scripts/test-canvas-vulkan.sh target/release/mfb --box 2228` → polyline row within `GPU_DEFAULT`, `vulkanReady=TRUE` (est. 10 min; the build and the AppImage ship to the box dominate, and only a real Vulkan ICD can show the shader is right — box 2228 is the script's default glibc x86_64 box with one).
Commit: —

## Validation Plan

- Tests: the `test-canvas-vulkan.sh` polyline row.
- Runtime proof: the box's stats line and the frame diff.
- Doc sync: `spec app canvas`.
- Final gate: once, at the end of plan-154-D.

## Open Decisions

- None beyond plan-154-A's.

## Corrections

(none yet)

## Summary

A shader branch plus an upload arm. Wind-scale scenes stay in software on
Vulkan until bug-688, by design.
