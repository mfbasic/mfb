# plan-175-B: The draw payload moves into a per-pass node table (both backends, no pixel change)

Last updated: 2026-10-13
Effort: large (3h–1d)
Depends on: plan-175-A

Every draw call today carries its group's offset and matrix as a 32-byte per-draw
payload (`MfbDraw` on Metal, a push constant on Vulkan). That is the one thing that
stops two `canvas::Group` nodes naming the same group from being one instanced draw:
their blocks are already recorded once and shared, but each node needs its own draw
call to carry its own offset. This plan moves that payload into a NODE TABLE: a region
of the frame buffer the emitter fills once per pass, one 32-byte node per draw-list
row, read by both shader stages through a node index. A draw then carries only a
16-byte RUN (`base`, `count`, `node`, `repeat`), and the shaders already know how to
draw `repeat` consecutive nodes in one call. plan-175-C uses that to merge group nodes;
this plan changes no pixel and merges nothing.

The single outcome: every canvas frame on Metal (macOS), Vulkan (Linux, box 2226) and
Vulkan (Windows, box 206) is byte-identical to the frame before the change, with each
draw's offset and matrix read from the node table and no per-draw `setFragmentBytes:`.

References:

- `planning/plan-175-A-canvas-geometry-identity-split.md` — decision 5 (this letter),
  §2.4 (the group row's 10,001 draws and 2.2 ms of encode).
- `.ai/canvas-threading.md` §10, "A group is one instanced draw per node, with an offset
  bound to BOTH stages" (the payload and why both stages need it), "The per-item
  parameter block travels in a buffer" (instance index semantics), "A frame that does
  not fit the frame buffer is drawn in passes" (bug-765).
- `.ai/arch-abi.md` (macOS AArch64 objc_msgSend staging), `.ai/compiler.md`.
- `scripts/regen-spirv.sh` — the SPIR-V is checked in and regenerated on a Linux box.
- `benchmark/canvas/README.md` — `.sums` holds each row's last-frame pixel hash.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-175-A is complete and archived | `ls planning/plan-175-A-* planning/completed/plan-175-A-*` → only the `completed/` path | NOT MET (2026-10-13) |

Everything below is written against the world where that holds.

## 1. Goal

- Each draw-list row is also a NODE: its words 2, 3, 5, 6 and 8–11 (offset words,
  residual words, inverse linear part) are its node payload, unchanged in meaning.
- Each row's word 13 is its REPEAT: 1 for every row this plan writes. A row with repeat
  `R >= 1` is a draw of `count * R` instances; plan-175-C writes `R > 1` and rows of
  repeat 0 (node-only rows, never drawn).
- Per pass, each emitter writes every row's node payload into the frame buffer's node
  region, row `i` at node `i` (32 bytes, the `MfbDraw` layout).
- Per draw, the emitter sends a 16-byte run `{ base, count, node, repeat }` to the
  vertex stage only. The vertex stage computes `local = instance - base`, the item
  index `base + local % count` and the node `node + local / count`, and hands the node
  to the fragment stage as a flat varying. Both stages read offset and inverse from the
  node table where they read the payload today.
- No pixel changes on any backend.

### Non-goals (explicit constraints)

- No change to the public API, the scene ring, the geometry cache, damage, item
  blocks (`ITEM_BLOCK_SIZE`), the software renderer, or any draw hash.
- No merging of draws: every row keeps repeat 1 (plan-175-C).
- No change to what a row's words 0, 1, 4, 7 and 12 mean (base, count, blend mode,
  `hasMatrix`, the pass word).
- The draw-list row stays 16 words (`CANVAS_DRAW_ENTRY_WORDS`); the seam's eight
  arguments stay eight (`func_metal_draw.rs`: the bank is full).

## 2. Current State

### The payload, from the walk to the shader

- `helper_render.rs:__canvas_pushOneDraw` appends one 16-word row: `instBase`,
  `instCount`, the 16.16 offsets, the blend mode, the two residual words, `hasMatrix`,
  the inverse's four float32 bit patterns, and four zero words (12–15).
  `helper_render.rs:__canvas_passDraw` copies a row into a pass with words 2–15
  unchanged; `__canvas_passNext` writes the pass word into word 12 of a pass's first
  row (`CANVAS_DRAW_PASS_WORD` = 12, `runtime/canvas/mod.rs`).
  `func_geo_cache.rs:lower_scene_draws_flat` writes the same rows natively for a frame
  with no group, and `helper_geometry.rs:__canvas_drawsVerify` checks it against the
  walk under `MFB_CANVAS_GEO_VERIFY`.
- Rows are written only for blocks in the viewport (plan-173). `__canvas_pushDraw`
  ends a run where `__canvas_boundsMeetViewport` changes its answer from one block to
  the next, and drops an offscreen run; `lower_scene_draws_flat` does the same through
  `emit_block_visible`. An offscreen block keeps its place in `__CANVAS_DRAW_BLOCKS` and
  its instance base, so culling changes how many rows there are, never a row's `base`.
  Row `i` is still the `i`-th row written, which is what "row `i` at node `i`" needs.
- Metal (`app/metal.rs:emit_draw_list_pass`), per row: copies words 2, 3, 5, 6, 8–11
  into the 32 bytes at `OFF_DRAW_PAIR`, binds the row's pipeline
  (`setRenderPipelineState:`), sends the 32 bytes with `setVertexBytes:` at index 1 and
  `setFragmentBytes:` at index 3, and draws `count` instances from `baseInstance: base`.
  Each of the four sends is preceded by a `load_selector` (`sel_registerName`).
- The MSL (`app/metal.rs`, `METAL_SHADER_SOURCE`): `struct MfbDraw { int4 offset; int4
  inverse; }`, read through `drawGrouped(draw)` and `drawInverse(draw)` and as
  `draw.offset` in `mfbVertex` and `mfbFragment`. `[[instance_id]]` includes
  `baseInstance` (plan-116-A Correction C5, measured). `VOut` carries `uint item
  [[flat]]`. Fragment `buffer(2)` is unused since bug-670; vertex uses `buffer(0)`
  (items) and `buffer(1)` (the payload).
- Vulkan (`runtime/canvas/vulkan.rs:emit_draw_list_pass`, the one draw loop for Linux
  readback and the Windows present), per row: `vkCmdPushConstants` of the 32 bytes
  (`PUSH_OFFSET_BYTES`), stages `SHADER_STAGE_VERTEX_AND_FRAGMENT`, then
  `vkCmdDraw(cmd, 4, count, 0, base)`. The GLSL (`shaders/mfb_canvas.vert`, `.frag`)
  declares `layout(push_constant) uniform Draw { ivec4 offset; ivec4 inverse; } draw`
  in both stages. `binding = 0` (the edge buffer: edges, residuals, bands, gradient
  stops, dashes, glyphs and the picture table, each region found by a specialization
  constant) is `SHADER_STAGE_FRAGMENT` only; only the fragment stage is given
  specialization info (`vulkan.rs`, `if stage == SHADER_STAGE_FRAGMENT`).
- The frame buffer regions are sized per PASS (`runtime/canvas/mod.rs`:
  `METAL_BUFFER_BYTES`, `VULKAN_BUFFER_BYTES`), and `__canvas_framePasses` /
  `__canvas_passNext` split a frame wherever a region would overflow
  (`__canvas_blockCost`). Neither counts draw-list rows today: rows carry their payload
  per draw, so their number is unbounded.

### Measured populations

| What | Count | Command |
|---|---|---|
| Draw rows a frame, `group fill 10000` | 10,001 | `grep group_fill_10000_draws benchmark/canvas/baseline/canvas.sums` |
| Encode median, `group fill 10000` | 2,221 µs | `awk '/^group fill 10000:/{s=1} s&&/encode/{print;exit}' benchmark/canvas/baseline/canvas.log` |
| Encode median, `group fill 100` (101 rows) | 33 µs | same, `group fill 100` |
| Objective-C sends per Metal draw row | 4 (`setRenderPipelineState:`, `setVertexBytes:`, `setFragmentBytes:`, `drawPrimitives:…`) | read `app/metal.rs:emit_draw_list_pass` |
| `draw.` payload reads in the MSL | 12 | `grep -c -E 'draw\.(offset\|inverse)\|\(draw\)\|, draw\)' src/target/macos_aarch64/app/metal.rs` |
| `draw.` payload reads in the GLSL | 5 vertex, 6 fragment | `grep -c -E 'draw\.(offset\|inverse)' src/codegen/runtime/canvas/shaders/mfb_canvas.{vert,frag}` |
| Places that write or read a row's layout | 6: `__canvas_pushOneDraw`, `__canvas_passDraw`, `__canvas_drawsText`, `lower_scene_draws_flat`, Metal's and Vulkan's `emit_draw_list_pass` | `git grep -n -E '__CANVAS_DRAWS\|CANVAS_DRAW_ENTRY' -- src` read |

### Verified properties

- **Rows never need the payload for anything but the shaders.** Read: the pass walk
  copies words 2–15 blindly (`__canvas_passDraw`); `__canvas_drawsText` prints words
  0–4; damage reads `__CANVAS_DRAW_HASHES`, not the rows. So filling a node table from
  the rows changes nothing upstream of the emitters.
- **The software renderer reads no row.** `git grep -n __CANVAS_DRAWS -- src` finds
  only `helper_render.rs`, `helper_surface.rs` (stats text) and `helper_geometry.rs`
  (the verifier). The oracle every GPU test compares against is untouched, so pixel
  equality against it stays a real check.
- **Culling decides rows, not nodes.** Neither viewport test reads or writes a row's
  words 2–15 (`__canvas_boundsMeetViewport` reads the geometry record and the entry's
  offset and matrix, which the walk already holds; `emit_block_visible` runs at no
  offset). So the node table needs no culling logic of its own, and a frame with every
  item offscreen has zero rows and zero nodes. Read: `helper_render.rs:__canvas_pushDraw`,
  `func_geo_cache.rs:emit_block_visible`.
- **Words 13–15 of a row are free.** `__canvas_pushOneDraw` writes 0 there, and
  `runtime/canvas/mod.rs`'s comment on `CANVAS_DRAW_ENTRY_WORDS` records "word 12 is
  the pass word; words 13–15 are reserved"; of the six readers above only the pass walk
  touches word 12 and none reads 13–15.

## 3. Design Overview

Three layers, landed in this order:

1. **Rows get a repeat word** (MFBASIC and the native flat list): word 13 = 1. Nothing
   reads it yet.
2. **The pass walk bounds rows per pass** by the node region's capacity,
   `MAX_FRAME_NODES` = 65,536 nodes (2,097,152 bytes, 32 a node) on each backend, the
   same as `MAX_FRAME_ITEMS`. A frame of more than 65,536 rows is drawn in passes, as a
   frame of too many blocks is today.
3. **The emitters and shaders switch to the node table** — Metal and Vulkan in one
   phase, because a row's meaning to a shader must not differ between the backends a
   test compares.

Where the correctness risk concentrates: layer 3. An off-by-one in `local % count` or
`local / count`, a node region at the wrong offset, or a stage that still reads the old
payload is a wrong picture, not a fault. The gate is pixels: every canvas suite that
compares against the software oracle, `benchmark/canvas`'s `.sums` (a last-frame hash
per row, 83 rows), and the GPU rows on box 2226 and box 206.

**Byte-identity is the gate for PIXELS, not for code.** The canvas goldens under
`tests/syntax/app/` and every canvas program's `.ncode` are EXPECTED to change (the
renderer and the walk change). A frame that differs by a pixel is a bug to root-cause
on one fixture and fix.

Rejected alternatives:

- **Node payloads as a ninth seam argument.** The bank is eight wide and full
  (`func_metal_draw.rs`); a ninth goes on the stack on one ABI and not another, which
  is the asymmetry the seam was built to avoid.
- **A separate node list beside the rows.** It needs that ninth argument, and a pass
  would have to rebase two lists that must agree. Rows already carry the payload.
- **The node table in the item buffer.** On Vulkan that is `binding = 1`, a block of
  `Item` structs; a second array in it needs an aliased declaration. The edge buffer
  already holds six regions found by specialization constants, and `binding = 0` only
  needs the vertex stage added.

## 4. Detailed Design

### 4.1 Rows and the pass walk (MFBASIC, both backends)

- `__canvas_pushOneDraw`: word 13 = 1 (it appends the same 16 words; the 13th is now
  1). `lower_scene_draws_flat` writes 1 there too, so `__canvas_drawsVerify` still
  compares equal lists.
- `__canvas_passNext`: counts rows in the pass. A row is added only if the pass's rows
  plus `max(repeat, 1)` stay within `__CANVAS_PASS_MAX_NODES`; otherwise the pass ends
  (`more`), exactly as a block that does not fit ends it. `__canvas_framePasses` adds
  the frame's row count to its "several passes" test against the same capacity.
  `__canvas_metalRenderable` / `__canvas_vulkanRenderable` pass each backend's
  `MAX_FRAME_NODES` (generated into the source like the other caps, `RENDER_CAPS`).
- `__canvas_drawsText` appends `:repeat` (word 13) to each row, so a test sees it.

### 4.2 Metal

- `runtime/canvas/mod.rs`: `METAL_MAX_FRAME_NODES` = 65,536, `METAL_NODE_BYTES` =
  32 × that, `METAL_NODE_OFFSET` = the node region's byte offset, after the picture
  table; `METAL_BUFFER_BYTES` grows by `METAL_NODE_BYTES`.
- `emit_draw_list_pass`, before the row loop: write each row's eight payload words as
  32-bit words to `contents + METAL_NODE_OFFSET + 32 * row`; then
  `setVertexBuffer:offset:atIndex:` and `setFragmentBuffer:offset:atIndex:` the frame
  buffer at `METAL_NODE_OFFSET`, index 2 on both stages. Per row: pipeline as now;
  `setVertexBytes:` of the 16-byte run at index 1; no `setFragmentBytes:`; draw
  `count * repeat` instances from `base`. A row of repeat 0 draws nothing.
- The selector lookups move out of the row loop: each `load_selector` result is parked
  on the frame once per pass and reused per row.
- MSL: `struct MfbRun { uint base; uint count; uint node; uint repeat; };`;
  `mfbVertex` takes `constant MfbRun &run [[buffer(1)]]` and `constant MfbDraw *nodes
  [[buffer(2)]]`, computes `local`, `index` and `node` as in §1, and reads
  `nodes[node]` where it read `draw`; `VOut` gains `uint node [[flat]]`. `mfbFragment`
  takes `constant MfbDraw *nodes [[buffer(2)]]` and reads `nodes[in.node]`.
  `drawGrouped` / `drawInverse` take the node.

### 4.3 Vulkan

- `runtime/canvas/mod.rs`: `VULKAN_MAX_FRAME_NODES` = 65,536 and
  `VULKAN_NODE_BASE_WORDS`, the node region's word offset in the edge buffer, after
  the picture table; `VULKAN_BUFFER_BYTES` grows by 2,097,152.
- `vulkan.rs`: `binding = 0` becomes `SHADER_STAGE_VERTEX_AND_FRAGMENT`; the vertex
  stage gets specialization info too, with constant 5 = `NODE_BASE` (both stages); the
  push-constant range becomes 16 bytes, `SHADER_STAGE_VERTEX` (`PUSH_OFFSET_BYTES`
  renamed `PUSH_RUN_BYTES`). `emit_draw_list_pass` writes the node region before its
  row loop and pushes the run per row; `vkCmdDraw(cmd, 4, count * repeat, 0, base)`.
- GLSL: the vertex stage declares the edge buffer and `layout(constant_id = 5) const
  int NODE_BASE`, the push constant `Run { int base; int count; int node; int repeat; }`,
  computes `local`, the item and the node, reads the node's eight words, and writes
  `layout(location = 1) flat out int vNode`. The fragment stage drops its push
  constant, reads `vNode`, and reads the node from the edge buffer. The stale vertex
  comment that says Metal's `[[instance_id]]` does not include the base instance is
  corrected (the MSL's own comment and plan-116-A C5 say it does).
- `scripts/regen-spirv.sh` regenerates both `.spv` files.

## Compatibility / Format Impact

- No public API, no program-visible behaviour, no pixel.
- Each backend's frame buffer grows by 2,097,152 bytes (the node region).
- A frame of more than 65,536 draw-list rows is now drawn in passes. Today such a frame
  draws in one pass if its blocks fit; it draws the same pixels in two.
- Every canvas program's code changes (walk and renderer), so the canvas goldens under
  `tests/syntax/app/` move; the SPIR-V blobs change.

## Phases

### Phase 1 — The repeat word, and rows bounded per pass

MFBASIC and the native flat list only; no emitter reads word 13 yet, so no frame can
change.

- [ ] `helper_render.rs:__canvas_pushOneDraw`: word 13 = 1.
- [ ] `func_geo_cache.rs:lower_scene_draws_flat`: word 13 = 1.
- [ ] `helper_render.rs`: `__CANVAS_PASS_MAX_NODES`; `__canvas_passNext` and
      `__canvas_framePasses` count rows (§4.1); the two renderable gates pass
      `METAL_MAX_FRAME_NODES` / `VULKAN_MAX_FRAME_NODES`, added to `runtime/canvas/mod.rs`
      and to `RENDER_CAPS`.
- [ ] `helper_surface.rs:__canvas_drawsText`: `:repeat` per row;
      `tests/canvas/rt_canvas_rasteriser/groups.rs`'s `draws=` assertions read five fields.
- [ ] Tests: a unit test in `helper_render/tests/mod.rs` that `__canvas_pushOneDraw`
      and `lower_scene_draws_flat` both write word 13; a runtime test of 65,537 group
      nodes naming one one-rectangle group that draws in two passes and matches the
      software frame.

Acceptance: rows carry repeat 1; a frame past 65,536 rows draws in passes and matches
the oracle; no other frame changes.
  Check: `cargo test --release --test rt_canvas_rasteriser --test rt_canvas_geo_native`
  → pass (est. 6 min: both suites build canvas programs; the flat-list verifier is in
  `rt_canvas_geo_native`).
  Check: `./benchmark/canvas/run.sh --only 'group|rect fill'` then `compare.py` → `.sums`
  unchanged (est. 2 min).
Commit: —

### Phase 2 — Both emitters and both shaders read the node table

- [ ] `runtime/canvas/mod.rs`: the Metal and Vulkan node-region constants (§4.2, §4.3);
      `METAL_BUFFER_BYTES` and `VULKAN_BUFFER_BYTES` grow.
- [ ] `app/metal.rs`: the MSL of §4.2; `emit_draw_list_pass` writes the node region,
      binds it at index 2 on both stages, sends the 16-byte run, sends no fragment
      bytes, draws `count * repeat`; selector lookups hoisted out of the row loop.
- [ ] `runtime/canvas/vulkan.rs`: binding 0's stages, the vertex stage's specialization
      info, the 16-byte vertex-only push range, `emit_draw_list_pass` as §4.3.
- [ ] `shaders/mfb_canvas.vert`, `shaders/mfb_canvas.frag`: §4.3; the stale comment
      fixed. `scripts/regen-spirv.sh` (box 2228) regenerates the `.spv` files.
- [ ] Tests: `the_metal_shader_binds_the_draw_offset_where_the_emitter_sends_it`
      (app/metal.rs) becomes the node table's binding test (index 2 both stages, run at
      vertex index 1, no fragment bytes); the Vulkan unit tests that pin the push range
      and binding 0's stages are updated to 16 bytes / vertex-and-fragment;
      `the_embedded_spirv_is_well_formed` passes on the new blobs.

Acceptance: every canvas frame is byte-identical on Metal, box 2226 and box 206.
  Check: `cargo test --release --test rt_canvas_metal --test rt_canvas_golden --test
  rt_canvas_rasteriser --test rt_canvas_graphics_thread` → pass (est. 10 min: these are
  the suites that compare Metal frames with the oracle, groups and matrices included).
  Check: `./benchmark/canvas/run.sh` then `compare.py` → `checksums: 415 compared, 0
  changed` (est. 5 min, attended; every DrawItem kind in a real window).
  Check: `scripts/test-canvas-vulkan.sh` on box 2226 → every row ok (Linux Vulkan); and
  `scripts/test-canvas-gpu-rows.sh <mfb> --target windows-x86_64 --keep <dir>` on box 206
  against the base compiler's `--keep` → every `*-gpu.rgba` equal by `cmp` (the only
  real-GPU Vulkan box).
Commit: —

### Phase 3 — Docs

- [ ] `.ai/canvas-threading.md` §10: the node table, the run, why both stages read the
      node by index, the per-pass row cap; the "offset bound to BOTH stages" text
      rewritten for nodes.
- [ ] `mfb spec app canvas` (`src/docs/spec/app/06_canvas.md`) where it describes the
      group draw and the frame buffer regions.

Acceptance: the docs describe HEAD.
  Check: `scripts/spec-census.sh --render` and `--citations` → no failures (est. 1 min).
Commit: —

## Validation Plan

- Tests: Phase 1's unit and pass tests; Phase 2's binding tests; every suite that
  compares a GPU frame with the software oracle.
- Coverage check: `benchmark/canvas` covers all fifteen DrawItem kinds and groups at 1,
  100 and 10,000 nodes in a real window, which no cargo suite does (they are headless,
  the readback path); its `.sums` is the pixel proof for the direct-present path.
- Runtime proof: `benchmark/canvas` `.sums` unchanged; GPU rows on box 2226 and 206.
- Doc sync: Phase 3.
- Final gate (once, after Phase 3): `cargo test --release`, `scripts/artifact-gate.sh
  target/debug/mfb all` (canvas goldens expected to move; every other golden must
  not), `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Decisions

- plan-175-B is its own letter, after plan-175-A — decided by the owner, 2026-10-13.
  It settles that the split's walk changes land first and this plan starts from them.
- No merging here — the author's call: it keeps this letter's gate pure pixel identity,
  and plan-175-C's merge is then an MFBASIC change on a format both emitters already
  read.

## Corrections

1. 2026-10-13, before any phase: plan-173 (merged at `bf951f32f`) made both draw-list
   builders cull offscreen blocks. §2 now says rows are written only for visible runs,
   and "Verified properties" records why the node table is unaffected. No goal, task or
   check changed; `benchmark/canvas`'s `group` rows are all in view, so their row counts
   (10,001, 101) still hold.

## Summary

The risk is all in Phase 2: two shaders and two emitters change how they find a draw's
offset in one step, and a mistake draws the wrong picture rather than failing. Pixels
are the gate, on every backend, and the benchmark covers the direct-present path the
cargo suites cannot. What it buys alone is one Objective-C send and four selector
lookups fewer per Metal draw; what it enables is plan-175-C.
