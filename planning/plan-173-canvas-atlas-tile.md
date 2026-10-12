# plan-173: Canvas atlas tiles and viewport culling

Last updated: 2026-10-11
Effort: large (3h–1d)

Add `canvas::Tile`, a `DrawItem` that samples a rectangle of a `canvas::Image` and
draws it into a destination rectangle. Source coordinates and sizes are in image pixels;
destination position and size are in canvas coordinates. Samples outside the image are
transparent, with no error and no remapping of the in-bounds portion. Tiles have no
paint, shape, or per-item transform. They can be stored in a `Group`, whose transform
moves, scales, or rotates the map as a whole.

The renderers also skip any item whose fully transformed bounds do not intersect the
viewport. This culling preserves scene membership and order, so moving a group back into
view makes its items visible again.

References:

- `planning/new_canvas.md` — canvas item and group surface; update with `Tile`.
- `planning/plan-172-A-canvas-shapes-premises.md` through
  `planning/plan-172-M-canvas-closeout.md` — preceding canvas surface and affine work.
- `.ai/canvas-threading.md` §§3, 6, 10, 13–14 — scene publication, image lifetime,
  group expansion, and item identity.
- `.ai/codegen-invariants.md`, `.ai/arch-abi.md`, `.ai/testing-gates.md`,
  `.ai/specifications.md`, `.ai/spec-content.md`, `.ai/man-content.md`.
- `src/docs/spec/app/06_canvas.md` — canvas contract.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| Plan 172 is complete and archived | `ls planning/plan-172-[A-M]-*` → no matches | NOT MET |
| Canvas group affine is in the registry and renderers | `rg -n 'name: "Group"|transform AS canvas::Transform' src/codegen/builtins/canvas/mod.rs` → group transform is present | NOT MET |

If plan 172 is not complete and archived, this plan cannot start, full stop. This plan
is written against the canvas surface after plan 172.

## 1. Goal

- `canvas::Tile` is a `DrawItem` with `image`, `srcPosition`, `srcSize`, `position`, and
  `size`. It has no paint, shape, or per-item transform.
- The requested source rectangle maps directly to the destination rectangle using the
  canvas's nearest-pixel sampling. Samples outside the image contribute transparent
  pixels; the source rectangle is neither clamped nor rejected.
- A tile works in a named `Group`; group transforms apply to it like other items.
- Software, Metal, and Vulkan skip raster/draw work for any item whose transformed
  bounds miss the viewport. A later group move can bring it back into view.
- `mfb man canvas` and `mfb spec app canvas` describe the new item and culling behavior.

### Non-goals

- No atlas loader, tilemap data structure, automatic grid slicing, or repeated texture
  fill.
- No tile-specific paint, shape, hit testing, or transform. The group is the transform
  boundary for maps.
- No changes to `Image` ownership, closing, mutation, or scene publication rules.
- No change to `Picture` semantics.

## 2. Current State

`canvas::Picture` currently samples an entire image into its destination rectangle:
`__canvas_pictureHeader` writes image dimensions and shadow address into the geometry
header (`src/codegen/builtins/canvas/helper_geometry.rs`). The software rasterizer
recognizes `__CANVAS_GEO_PICTURE` and reads the shadow (`src/codegen/builtins/canvas/helper_items.rs`).
The Metal and Vulkan paths each build a frame picture table and sample the image in
their renderers (`src/target/macos_aarch64/app/metal.rs` and
`src/codegen/runtime/canvas/vulkan.rs`).

The software renderer skips items whose bounds miss a partial damage rectangle via
`__canvas_boundsMeetOffset` (`src/codegen/builtins/canvas/helper_damage.rs`), and its
per-item pixel loop clamps bounds to the surface (`__canvas_drawGeometry` in
`src/codegen/builtins/canvas/helper_items.rs`). Its full-frame walk still visits each
item. Metal and Vulkan redraw the full frame and build their draw list from the expanded
scene (`__canvas_renderMetal` and `__canvas_renderVulkan` in
`src/codegen/builtins/canvas/helper_render.rs`); they do not currently filter draw-list
items by viewport intersection.

Plan 172 supplies group affine transforms and the picture shape work this plan follows.
The current renderer uses a shared 67-float geometry header
(`__CANVAS_GEO_HEADER` in `helper_geometry.rs`); slots 55–66 already encode gradient
state. Tile source coordinates must be carried without widening or reinterpreting the
header in a way that changes other item kinds. Use a tile-specific geometry tail or
another explicit, cache- and backend-visible representation.

### Measured populations

| What | Count | Command |
|---|---:|---|
| Picture runtime test cases | 15 test functions | `rg -c '^fn ' tests/canvas/rt_canvas_picture.rs` → 15 |
| Canvas backend render paths | 3 | `rg -n 'FUNC __canvas_renderScene|FUNC __canvas_renderMetal|FUNC __canvas_renderVulkan' src/codegen/builtins/canvas/helper_render.rs` → 3 matches |

### Verified properties

- `__canvas_boundsMeetOffset` compares an item's geometry bounds after group offsets
  against a damage rectangle; it is not a viewport test. Verified by reading the
  function in `src/codegen/builtins/canvas/helper_damage.rs`.
- `__canvas_drawGeometry` clamps the software pixel-loop bounds against `width` and
  `height`. This prevents off-surface pixel visits but still enters the function for a
  fully offscreen item. Verified by reading the bound setup in
  `src/codegen/builtins/canvas/helper_items.rs`.
- Metal and Vulkan receive the expanded scene's draw blocks and draw list; their
  renderer entry points do not receive a viewport-filtered list. Verified by reading
  `__canvas_renderMetal`, `__canvas_renderVulkan`, and their `metalDrawScene` /
  `vulkanDrawScene` calls in `helper_render.rs`.

Adding a `DrawItem` variant is not guarded by exhaustive compiler matches: several
consumers have wildcard arms. The implementation must trace the variant through the
registry, item hashing, header/tail generation, software dispatch, GPU renderability and
sampling, and native geometry tests. `.ai/codegen-invariants.md` documents this
wildcard-arm trap; the existing `Picture` and `PolyLine` paths are the precedents.

## 3. Design Overview

Keep the source rectangle as per-item data so it participates in geometry cache
confirmation and draw hashing. Keep image lifetime tied directly to the `Tile`'s
`RES canvas::Image`, following `Picture`. Use a distinct geometry kind and an explicit
source-rectangle payload; the renderer maps destination-local coordinates linearly to
the requested source rectangle, tests each source sample against image dimensions, and
returns transparent outside the image.

Add viewport culling after world bounds are known and before backend draw submission.
The culling test uses the viewport rectangle and the item's bounds after its own
placement/transform and every enclosing group transform. Preserve all scene items in
publication, hashing, and damage history; culling only affects work for the current
frame. The correctness risk is false culling at transformed edges, which can make
visible pixels disappear; tests must cover translated, rotated, and scaled groups near
viewport edges.

This is a behavior-changing plan, so byte identity is not a correctness gate. The Tile
fixture is expected to differ from its pre-Tile rendering, and culling is expected to
leave visible output unchanged while reducing submitted/rasterized offscreen work.

Rejected alternatives:

- Add source fields to `Picture`: rejected because atlas selection would change the
  existing whole-image item and make its source behavior conditional.
- Clamp the source rectangle to image bounds: rejected because it remaps the remaining
  source pixels across the full destination instead of preserving transparent out-of-
  bounds samples.
- Cull by destination position before applying group transforms: rejected because a
  transformed map can move an otherwise offscreen item into view.

## 4. Detailed Design

### Tile geometry and sampling

- Register `Tile` as a paint-free `DrawItem` variant. Fields are `image AS RES
  canvas::Image`, `srcPosition AS vector::Float2`, `srcSize AS vector::Float2`,
  `position AS vector::Float2`, and `size AS vector::Float2`.
- A nonpositive destination extent or source extent draws nothing, matching zero-area
  geometry. Fractional source positions/sizes are allowed; nearest-pixel selection uses
  the existing sampler convention.
- The sample mapping is `srcPosition + destinationLocal * srcSize / size`. A mapped
  coordinate outside `[0, imageWidth) × [0, imageHeight)` yields transparent RGBA.
  In-bounds samples retain the existing image alpha behavior.
- Include source fields and image identity in the geometry confirmation/hash so changing
  an atlas cell cannot reuse another tile's cached geometry or damage result.
- Reuse the image resource read/closed behavior of `Picture`: a closed or unavailable
  image produces an empty draw.

### Grouping and culling

- Tile bounds are its destination rectangle. Compose those bounds through its item
  placement and every enclosing group affine before testing viewport intersection.
- Add one shared bounds/viewport predicate used by software, Metal, and Vulkan draw
  submission. An empty intersection skips that item's renderer work; it does not remove
  the item from the scene or group.
- Keep partial-damage culling separate: damage decides which visible items need redraw
  into a kept software surface; viewport culling decides whether an item can affect the
  current frame at all.
- For affine transforms, use the axis-aligned hull of all four transformed rectangle
  corners. Boundary contact follows the renderer's existing half-open surface
  convention.

## Phases

### Phase 1 — Tile API and software oracle

Add the registry type, geometry representation, software nearest-pixel sampling, and
focused runtime coverage for atlas selection, fractional scaling, transparent samples
outside image bounds, and invalid/closed images.

- [ ] Add `Tile` to `src/codegen/builtins/canvas/mod.rs` and `DrawItem` documentation.
- [ ] Add tile source geometry and hashing in `helper_geometry.rs` and software sampling
      in `helper_items.rs`.
- [ ] Add `tests/canvas/rt_canvas_tile.rs` with inside, partial out-of-bounds, wholly
      out-of-bounds, scaled, zero-area, and closed-image cases.
- [ ] Add the matching `tests/golden/canvas/tile.png` fixture and route it through the
      existing canvas runtime harness.

Acceptance: software output selects the requested atlas pixels and renders out-of-image
samples transparent without errors.
  Check: `cargo test --test rt_canvas_tile` → pass (est. under 10 minutes).
Commit: —

### Phase 2 — Metal and Vulkan sampling

Carry the tile source rectangle through both GPU backends and match the software oracle.

- [ ] Extend picture-table/upload preparation to preserve tile source bounds in
      `src/target/macos_aarch64/app/metal.rs` and `src/codegen/runtime/canvas/vulkan.rs`.
- [ ] Update Metal and Vulkan shader sampling to apply the same mapping and transparent
      out-of-bounds rule.
- [ ] Add a named `tile` row to `scripts/test-canvas-gpu-rows.sh` so Vulkan runs the
      atlas case against the software oracle on the remote Vulkan host.
- [ ] Extend `tests/canvas/rt_canvas_metal.rs` and the Vulkan GPU-row coverage for atlas
      regions, including partial out-of-bounds samples.

Acceptance: Metal and Vulkan tile output match the software oracle within the existing
GPU tolerance, including transparent samples outside the image.
  Check: `cargo test --test rt_canvas_metal tile` → Metal tile row passes (est. under
  10 minutes on the macOS Metal host).
  Check: `scripts/test-canvas-gpu-rows.sh target/release/mfb --target linux-aarch64
  --box 2226 --rows tile` → Vulkan tile row matches the software oracle (est. under
  10 minutes; box 2226 is the native AArch64 Vulkan host).
Commit: —

### Phase 3 — Viewport culling

Skip renderer work for fully offscreen items across software, Metal, and Vulkan while
keeping the scene intact for later group movement.

- [ ] Add the shared transformed-bounds/viewport intersection helper in
      `helper_damage.rs` or its geometry companion.
- [ ] Apply it to the software full-frame walk and both GPU draw-list builders after
      nested group transforms are composed.
- [ ] Add runtime cases for items beyond each viewport edge and a map group moved,
      scaled, and rotated across an edge; assert visible pixels are preserved and an
      offscreen-to-onscreen group move draws the items.

Acceptance: fully offscreen items are not submitted to raster/draw work, and transformed
items that intersect the viewport remain visible.
  Check: `cargo test --test rt_canvas_tile --test rt_canvas_golden group` → pass
  (est. under 10 minutes).
Commit: —

## Validation Plan

- Tests: `tests/canvas/rt_canvas_tile.rs`, `tests/canvas/rt_canvas_metal.rs`, and
  transformed-group canvas coverage; preserve existing picture, image ownership, and
  group behavior.
- Coverage check: ensure the new Tile kind is included by the native geometry test and
  by software, Metal, and Vulkan runtime suites; ensure culling tests exercise both
  fully offscreen and partially visible transformed bounds.
- Runtime proof: render an atlas image with multiple tiles inside a transformed map
  group, then move the group through the viewport; verify the selected cells and
  transparent out-of-bounds samples in each renderer.
- Doc sync: update `planning/new_canvas.md`, registry prose in
  `src/codegen/builtins/canvas/mod.rs`, `src/docs/spec/app/06_canvas.md`, and generated
  `mfb man canvas` output.
- Final gate (once, after the last phase): `cargo test` and
  `scripts/artifact-gate.sh target/release/mfb all` → pass; run the canvas GPU row
  script on macOS Metal and Vulkan host 2226 (Linux AArch64).

## Decisions

- The item is named `Tile`; its source rectangle is `srcPosition`/`srcSize` in image
  pixels.
- Out-of-bounds source samples are transparent and do not raise or clamp/remap.
- Tiles have no paint, shape, per-item transform, or hit testing. Group transforms are
  the way to move/scale/rotate a map.
- Viewport culling applies to all canvas items and all renderers, including items inside
  transformed groups.
