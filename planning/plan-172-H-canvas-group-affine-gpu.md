# plan-172-H: The group affine on Metal and Vulkan

Last updated: 2026-10-10
Effort: large (3h–1d)
Depends on: plan-172-G

172-G made a transformed group draw on the software renderer and told both GPU backends
to decline such a frame. This letter teaches both backends the per-draw matrix and
removes the decline.

Behavioral outcome: a scene with rotated, scaled and nested groups renders on Metal and
on Vulkan within `Tolerance::GPU_DEFAULT` of the software oracle, and a translate-only
group scene renders byte-identically on each GPU backend to what it rendered before this
letter.

References:

- `planning/plan-172-G-canvas-group-affine-software.md` §3 — the 16-word draw entry and
  the composed inverse.
- `.ai/canvas-threading.md` §10 — "The offset goes to the fragment stage as well as the
  vertex stage, and that is the fact most likely to be got wrong"; "The instance index
  includes the base on both languages".
- `.ai/arch-abi.md` — macOS AArch64 and Windows traps for the two emitters.
- `src/target/macos_aarch64/app/metal.rs` — `emit_draw_list_pass`, the MSL source
  (`METAL_SHADER_BODY`), `mfbVertex`, `mfbFragment`, `inverseMap`, `dfShapePoint`.
- `src/codegen/runtime/canvas/vulkan.rs:emit_draw_list_pass`,
  `src/codegen/runtime/canvas/shaders/mfb_canvas.vert`, `mfb_canvas.frag`.
- `scripts/regen-spirv.sh`, `scripts/test-canvas-vulkan.sh`,
  `scripts/test-canvas-gpu-rows.sh`, `.ai/remote_systems.md`.

## Prerequisites

The plan-wide gate is in `plan-172-A` §Prerequisites. This letter adds:

| Must be true | Command | Status |
|---|---|---|
| plan-172-G complete and archived | `ls planning/plan-172-G-*` → no matches | NOT MET |
| The software golden exists | `ls tests/golden/canvas/group_affine.png` → present | NOT MET |
| The draw entry is 16 words | `grep -c 'CANVAS_DRAW_ENTRY_WORDS: usize = 16' src/codegen/runtime/canvas/mod.rs` → 1 | NOT MET |
| Box 2226 reaches a Vulkan device | `scripts/test-canvas-vulkan.sh target/release/mfb --box 2226` on an unchanged tree → pass | UNVERIFIED — run first |

If plan-172-G is not complete, this letter cannot start, full stop.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again before
> you decide to stop. If you stop, report the status of *all* prerequisites.

## 1. Goal

- Metal and Vulkan draw a transformed-group scene; neither predicate declines it.
- Each matches `tests/golden/canvas/group_affine.png` within `Tolerance::GPU_DEFAULT`.
- A translate-only group scene produces the same GPU frame, byte for byte, before and
  after this letter, on each backend.
- The walk's decline flag from 172-G is deleted.

### Non-goals (explicit constraints)

- **`ITEM_BLOCK_SIZE` stays 368.** The matrix is per-draw, not per-item.
- **No change to how items, edges, glyphs or pictures are transported.**
- **No Vulkan direct-present work.**
- **The software renderer is not touched**; it is the oracle this letter is measured
  against.

## 2. Current State

Each backend walks the draw list once and issues one instanced draw per entry. It binds
four 32-bit words — the 16.16 offset and its two residuals — as a 16-byte per-draw
payload:

- **Metal:** `setVertexBytes:length:atIndex:` (16 bytes, index 1) and
  `setFragmentBytes:length:atIndex:` (16 bytes, index 3). `mfbVertex` adds the offset to
  the quad corner; `mfbFragment` subtracts it from the pixel to get the shape-space
  point, in double-float for the residual.
- **Vulkan:** one push-constant range, both stages, offset 0, `PUSH_OFFSET_BYTES = 16`,
  declared in both shaders as `uniform Draw { ivec4 offset; } draw;`.

Each fragment shader already has `hasTransform()` and `inverseMap()` for the item's own
inverse, read from the item block at `ITEM_OFFSET_TRANSFORM`.

### Measured populations

| What | Count | Command |
|---|---|---|
| `drawOffset` references in the Metal emitter and MSL | 8 | `grep -c drawOffset src/target/macos_aarch64/app/metal.rs` → 8 |
| `draw.offset` references in GLSL | 3 fragment, 1 vertex | `grep -c 'draw\.offset' src/codegen/runtime/canvas/shaders/mfb_canvas.frag` → 3; `.vert` → 1 |
| Per-draw payload today | 16 bytes | `grep -n 'const PUSH_OFFSET_BYTES' src/codegen/runtime/canvas/vulkan.rs` → `= 16` |
| Guaranteed push-constant range | 128 bytes | Vulkan specification minimum `maxPushConstantsSize`; cited in `.ai/canvas-threading.md` §10 |

### Verified properties

- **32 bytes of push constants is within the guaranteed range.** VERIFIED:
  `.ai/canvas-threading.md` §10 records 128 bytes as the only guaranteed value.
- **Both stages need the matrix.** VERIFIED by the same section's reasoning for the
  offset: the renderer is distance-field based, so the vertex stage moves the quad and
  the fragment stage moves the query point. A matrix bound to the vertex stage alone
  puts the shape in the right place with the wrong contents.
- **UNVERIFIED — that a double-float translation followed by a single-float 2×2 keeps
  the bug-735 precision for a translate-only entry.** It is not asked to: when
  `hasMatrix` is 0 the shaders take today's branch untouched. Phase 1's byte-identity
  check is what proves that branch really is untouched.

## 3. Design Overview

**The payload grows to 32 bytes:** the four translation words as today, then the four
float-bit words of `A⁻¹`'s linear part. `hasMatrix` is not sent; a linear part of all
zero bits means "no matrix", which is the same all-zero-is-identity convention every
`canvas::Transform` already has.

**Vertex stage.** With a matrix, the quad corner is mapped through the forward linear
part — the inverse of the 2×2 it was given, computed in the shader — and then offset.
The resulting parallelogram is padded by one pixel so no covered pixel falls outside it.

**Fragment stage.** The shape-space point is `A⁻¹ · (pixel − translation)`, then the
item's own `inverseMap()` if it has one. Distance rescaling, the gradient ramp, glyph
coverage and the clip all evaluate at that point, as they do for an item's own
transform. The composed point enters through one function per language so every
consumer sees it.

**Gate class, stated per branch:**

- `hasMatrix = 0`: **byte-identical GPU frames** before and after, on each backend. This
  is a legitimate byte-identity gate — the branch is meant to be untouched. A diff
  there is root-caused on one scene; it is not evidence the design is wrong.
- `hasMatrix = 1`: within `Tolerance::GPU_DEFAULT` of the software golden.

**Where the risk is:** the Metal emitter is hand-written AArch64 and the Vulkan emitter
exists for four targets. The byte counts in `setVertexBytes:` / `setFragmentBytes:` and
`vkCmdPushConstants`, and the Vulkan pipeline layout's range size, must all move
together; a layout range left at 16 with a 32-byte push is a validation error on some
drivers and silence on others.

### Rejected alternatives

- **Sending `hasMatrix` as a ninth word.** Rejected: it makes the payload 36 bytes for
  information the linear part already carries.
- **A second pipeline for transformed groups.** Rejected: the pipeline-per-blend-mode
  set would double, for a branch a shader takes on one comparison.
- **Leaving Vulkan on the software fallback.** Rejected: a declined frame is correct but
  it is not the feature, and Linux and Windows users would get a silent slow path.

## Compatibility / Format Impact

- **Changed:** the per-draw payload, 16 → 32 bytes; the Vulkan pipeline layout's
  push-constant range; both fragment shaders, both vertex shaders, both `.spv` blobs.
- **Unchanged:** `ITEM_BLOCK_SIZE`, the item, edge and glyph regions, the frame caps,
  the draw entry (already 16 words).

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as
> the work it describes. Use `- [~]` for partially done plus one line on what remains.
> Mark a task moot with `- [x] ~~text~~ — moot: <evidence>`. Fill each `Commit:` line
> the moment the phase lands. **An unticked box means NOT DONE.**

### Phase 1 — Capture the baselines

- [ ] On this letter's starting commit, dump the `groups.png` scene under
      `MFB_CANVAS_GPU=1` on Metal to `/tmp/plan-172-H/metal-before.rgba`.
- [ ] Do the same on Vulkan with `scripts/test-canvas-vulkan.sh target/release/mfb --box
      2226`, keeping the dump as `/tmp/plan-172-H/vulkan-before.rgba`.
- [ ] Record both file hashes in Corrections.

Acceptance: two baseline dumps exist with recorded hashes.
  Check: `shasum /tmp/plan-172-H/*-before.rgba` → two lines (est. 10 min, of which the
  box 2226 run is most; it is the native aarch64 Vulkan box).
Commit: —

### Phase 2 — Metal

- [ ] `metal.rs:emit_draw_list_pass`: copy entry words 2, 3, 5, 6 and 8–11 into the
      32-byte pair buffer; send 32 bytes to both stages.
- [ ] MSL: the per-draw struct gains the four float-bit words; `mfbVertex` maps the
      corner when the linear part is non-zero; `mfbFragment` and `dfShapePoint` compose
      the query point.
- [ ] `helper_render.rs:__canvas_metalRenderable`: stop reading the decline flag.
- [ ] A Metal parity case for `group_affine_on_black.mfb` in
      `tests/canvas/rt_canvas_metal.rs`.
- [ ] Re-dump the `groups.png` scene on Metal and compare with the baseline.

Acceptance: Metal matches `group_affine.png` within `Tolerance::GPU_DEFAULT`, and the
translate-only dump is byte-identical to `metal-before.rgba`.
  Check: `cargo test --test rt_canvas_metal group` → pass, and `python3
  tools/canvas-bench/cmp.py /tmp/plan-172-H/metal-before.rgba <after>` → 0 differing
  pixels (est. 6 min).
Commit: —

### Phase 3 — Vulkan (largest blast radius)

- [ ] `mfb_canvas.vert` and `mfb_canvas.frag`: `Draw` gains `ivec4 inverse`; the same
      vertex mapping and query-point composition as the MSL.
- [ ] `bash scripts/regen-spirv.sh`; commit both `.spv` blobs with the shader sources.
- [ ] `vulkan.rs`: `PUSH_OFFSET_BYTES = 32`; the pipeline layout's range size; the eight
      words in `emit_draw_list_pass`.
- [ ] `helper_render.rs:__canvas_vulkanRenderable`: stop reading the decline flag;
      delete the flag and its setter in the walk.
- [ ] Add `group_affine_on_black.mfb` to `scripts/test-canvas-gpu-rows.sh`.
- [ ] Run the Vulkan row on box 2226 (native aarch64, lavapipe) and on box 2228
      (x86_64 glibc; needed because the emitter differs per ISA).
- [ ] Re-dump the `groups.png` scene on Vulkan and compare with the baseline.
- [ ] `src/docs/spec/app/06_canvas.md` "Named groups": remove the interim decline; state
      the 32-byte payload and that it binds to both stages, cited to both emitters.
- [ ] `.ai/canvas-threading.md` §10: the "(itemBase, itemCount, dx, dy, …)" entry line
      and the three binding numbers become the 16-word entry and the 32-byte payload.

Acceptance: Vulkan matches the golden within `Tolerance::GPU_DEFAULT` on both boxes,
and the translate-only dump is byte-identical to `vulkan-before.rgba`.
  Check: `scripts/test-canvas-vulkan.sh target/release/mfb --box 2226` → pass, the same
  with `--box 2228` → pass, and `cmp.py` on the two dumps → 0 differing pixels (est.
  25 min; box 2228 is emulated x86_64 and is the only place the x86_64 emitter runs).
Commit: —

## Validation Plan

- Tests: one Metal parity case; one Vulkan row on two boxes; two byte-identity
  comparisons of translate-only frames.
- Coverage check: the stats line must report `metal=` or `vulkan=` as the renderer for
  the transformed-group scene. A leftover decline would pass every pixel comparison,
  because software is what the comparison is against.
- Runtime proof: `examples/canvas-demo` under `MFB_CANVAS_GPU=1` with one of its groups
  given a rotation, compared against its software dump.
- Doc sync: `src/docs/spec/app/06_canvas.md`; `.ai/canvas-threading.md` §10.
- Final gate: none in this letter; 172-M. The Windows Vulkan row is run there, on
  `test@192.168.1.206`.

## Open Decisions

None.

## Corrections

<Filled in DURING execution.>

## Summary

Two hand-written emitters and four shader stages change in step. The safety is that the
untransformed branch has a byte-identity gate on each backend, captured before any
edit, and the transformed branch has an oracle that 172-G already landed.
