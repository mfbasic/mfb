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
| plan-172-G complete and archived | `ls planning/plan-172-G-*` → no matches | MET (2026-10-10) |
| The software golden exists | `ls tests/golden/canvas/group_affine.png` → present | MET (2026-10-10) |
| The draw entry is 16 words | `grep -c 'CANVAS_DRAW_ENTRY_WORDS: usize = 16' src/codegen/runtime/canvas/mod.rs` → 1 | MET (2026-10-10) |
| Box 2226 reaches a Vulkan device | `scripts/test-canvas-vulkan.sh target/release/mfb --box 2226` on an unchanged tree → pass | MET (2026-10-10; run first with `--target linux-aarch64`: "canvas Vulkan runtime tests passed") |

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

- [x] On this letter's starting commit, dump the `groups.png` scene under
      `MFB_CANVAS_GPU=1` on Metal to `/tmp/plan-172-H/metal-before.rgba`.
- [x] Do the same on Vulkan with `scripts/test-canvas-vulkan.sh target/release/mfb --box
      2226`, keeping the dump as `/tmp/plan-172-H/vulkan-before.rgba`.
- [x] Record both file hashes in Corrections.

Acceptance: two baseline dumps exist with recorded hashes.
  Check: `shasum /tmp/plan-172-H/*-before.rgba` → two lines (est. 10 min, of which the
  box 2226 run is most; it is the native aarch64 Vulkan box).
  Observed (2026-10-10): `dfb2bbdd…` Metal (twice), `f087dc9f…` Vulkan on box 2226
  (twice), `f087dc9f…` Vulkan on box 2227, and `8e4e16ac…` for the `groupsblack` row on
  box 206 — full hashes and file names in Corrections.
Commit: c4e96512a

### Phase 2 — Metal

- [x] `metal.rs:emit_draw_list_pass`: copy entry words 2, 3, 5, 6 and 8–11 into the
      32-byte pair buffer; send 32 bytes to both stages.
- [x] MSL: the per-draw struct gains the four float-bit words; `mfbVertex` maps the
      corner when the linear part is non-zero; `mfbFragment` and `dfShapePoint` compose
      the query point.
- [x] `helper_render.rs:__canvas_metalRenderable`: stop reading the decline flag.
- [x] A Metal parity case for `group_affine_on_black.mfb` in
      `tests/canvas/rt_canvas_metal.rs`.
- [x] Re-dump the `groups.png` scene on Metal and compare with the baseline.

Acceptance: Metal matches `group_affine.png` within `Tolerance::GPU_DEFAULT`, and the
translate-only dump is byte-identical to `metal-before.rgba`.
  Check: `cargo test --test rt_canvas_metal group` → pass, and `python3
  tools/canvas-bench/cmp.py /tmp/plan-172-H/metal-before.rgba <after>` → 0 differing
  pixels (est. 6 min).
  Observed (2026-10-10): `cargo test --test rt_canvas_metal group` → 4 passed;
  `the_gpu_draws_the_group_affine_scene_the_reference_shows` passed (Corrections: it is
  in `rt_canvas_golden.rs`). The Metal dump after the change, and again on the final
  build, is `dfb2bbdd…` — the same file as before, so the comparison is `shasum`
  equality rather than `cmp.py`.
Commit: c4e96512a

### Phase 3 — Vulkan (largest blast radius)

- [x] `mfb_canvas.vert` and `mfb_canvas.frag`: `Draw` gains `ivec4 inverse`; the same
      vertex mapping and query-point composition as the MSL.
- [x] `bash scripts/regen-spirv.sh`; commit both `.spv` blobs with the shader sources.
- [x] `vulkan.rs`: `PUSH_OFFSET_BYTES = 32`; the pipeline layout's range size; the eight
      words in `emit_draw_list_pass`.
- [x] `helper_render.rs:__canvas_vulkanRenderable`: stop reading the decline flag;
      delete the flag and its setter in the walk.
- [x] Add `group_affine_on_black.mfb` to `scripts/test-canvas-gpu-rows.sh`.
- [x] Run the Vulkan row on box 2226 (native aarch64, lavapipe) and on box 2228
      (x86_64 glibc; needed because the emitter differs per ISA). — box 2226 done; box
      2228 is down, and the x86_64 emitter ran on box 2227 and on box 206 instead
      (Corrections).
- [x] Re-dump the `groups.png` scene on Vulkan and compare with the baseline.
- [x] `src/docs/spec/app/06_canvas.md` "Named groups": remove the interim decline; state
      the 32-byte payload and that it binds to both stages, cited to both emitters.
- [x] `.ai/canvas-threading.md` §10: the "(itemBase, itemCount, dx, dy, …)" entry line
      and the three binding numbers become the 16-word entry and the 32-byte payload.

Acceptance: Vulkan matches the golden within `Tolerance::GPU_DEFAULT` on both boxes,
and the translate-only dump is byte-identical to `vulkan-before.rgba`.
  Check: `scripts/test-canvas-vulkan.sh target/release/mfb --box 2226` → pass, the same
  with `--box 2228` → pass, and `cmp.py` on the two dumps → 0 differing pixels (est.
  25 min; box 2228 is emulated x86_64 and is the only place the x86_64 emitter runs).
  Observed (2026-10-10): box 2226 → "canvas Vulkan runtime tests passed" and the
  `groupaffine` row "Vulkan matches the software oracle (ok worst=1 differing=0.1502%)";
  box 2228 is down, so the x86_64 emitter ran on box 2227 (same result, same figures)
  and on box 206's real GPU ("ok worst=1 differing=0.9651%") — Corrections. The
  translate-only dump is byte-identical before and after on all three.
  Local round on the final tree: all 25 canvas-embedding targets green (`rt_canvas_golden` 25, `rt_canvas_metal` 49, `rt_canvas_rasteriser` 106, `rt_canvas_font` 33), `cargo test --bin mfb canvas` 115, `spec` 46, `vulkan` 9, `metal` 10 passed. `scripts/artifact-gate.sh target/release/mfb all` → `2322 golden(s) checked, 0 diff(s)`.
Commit: c4e96512a

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

- **Three phases, one commit**, for the reason 172-G's were: the IR-embedding goldens
  move at each. The observations above were taken in order — baselines, Metal, Vulkan.
- **The baselines, in full.** The scene is the `GROUPS` const of
  `tests/canvas/rt_canvas_golden.rs` (the `groups.png` scene), built with the fixture
  font and run with `MFB_CANVAS_GPU=1`; each was on the GPU (`gpuFrames=1`).

  | Backend | Where | Before | After |
  |---|---|---|---|
  | Metal | this Mac | `dfb2bbdde9551d9cf52231678c9b01f600dd6cb7` (twice) | the same, after the Metal change and on the final build |
  | Vulkan, aarch64 | box 2226, Mesa lavapipe | `f087dc9f5ad4fd81bb83ae14cff29aaa16c73d22` (twice) | the same |
  | Vulkan, x86_64 Linux | box 2227, Mesa lavapipe (`--libc musl --icd auto`) | `f087dc9f5ad4fd81bb83ae14cff29aaa16c73d22` | the same |
  | Vulkan, x86_64 Windows | box 206, Intel UHD 630 | `8e4e16acfe983604c2b20d8dcd489fb9ff16dbd0` (the `groupsblack` row) | the same |

  The dumps are kept by a new `--keep <dir>` option on `scripts/test-canvas-vulkan.sh`
  and `scripts/test-canvas-gpu-rows.sh`, which is how a later change can repeat this.
- **Box 2228 is down** (`ssh: connect to host 127.0.0.1 port 2228: Connection refused`,
  probed twice). The plan wanted it for two things, both met another way:
  - *the x86_64 emitter* ran on box 2227 (Alpine x86_64 musl, lavapipe provisioned by
    `--icd auto`): the full Vulkan test passed and the `groupaffine` and `groupsblack`
    rows passed. ISA is what 2228 was wanted for; the libc differs.
  - *SPIR-V* was compiled with `MFB_SPIRV_PORT=2226 scripts/regen-spirv.sh` (glslang
    `11:12.0.0`, not 2228's): `vert` 9144 bytes, `frag` 256440 bytes. Both shaders
    changed, so both blobs would have changed on any box. That the translate-only frame
    is byte-identical anyway says the compiler difference reaches no pixel.
- **The Windows row was run in this letter, not left to 172-M** (owner's prompt, during
  the letter): `scripts/test-canvas-gpu-rows.sh … --target windows-x86_64 --rows
  groupaffine,groupsblack,xformblack` on box 206 — all three drawn on Vulkan and within
  tolerance. That is the Win64 ABI path of the Vulkan emitter and the only real GPU.
- **`hasMatrix` is not sent, as designed**: the shaders test the four inverse words for
  all-zero. An entry with no matrix has zeros there from `__canvas_pushOneDraw` and from
  the native `canvas::sceneDrawsFlat`.
- **The quad is grown by the inverse's Frobenius norm, not "one pixel".** One pixel on
  the surface is `|M|` in the group's space at most, so the box is grown by that before
  it is mapped. A fixed pixel of padding after the map would not be a parallelogram's
  four corners any more.
- **A rectangle clip is sampled under a matrix** (`clipRectDistance` in both shaders),
  as 172-G's Corrections record for the software renderer. The golden's clipped band is
  the case.
- **The item-transform path is restructured, not just extended**: `shapeCoverage` now
  distinguishes `itemMapped` (the item's own inverse applies) from `mapped` (the
  transformed path is taken). With no matrix the two are equal and every expression is
  the one it was — which the four byte-identical baselines hold, and
  `rt_canvas_metal`'s 49 and the `xformblack` row confirm for transformed items.
- **The Metal parity case is in `tests/canvas/rt_canvas_golden.rs`**
  (`the_gpu_draws_the_group_affine_scene_the_reference_shows`), beside the other
  GPU-against-reference cases, because that file has the fixture-font helper the scene
  needs. `rt_canvas_metal.rs` has the small one: 172-G's
  `a_group_with_a_matrix_falls_back_to_the_software_renderer` became
  `a_group_with_a_matrix_draws_on_metal_and_matches_the_software_oracle` — the decline
  it pinned is what this letter removes.
- **Three Metal unit tests that pin shader text were updated** to the new spellings
  (`constant MfbDraw &draw [[buffer(N)]]`, `dfShapePoint(in.pos.xy, draw.offset)`, the
  32-byte frame slot). What each protects is unchanged: one binding per stage at the
  index the emitter sends to, the offset subtracted in shape space, no overlapping
  frame slots.
- **The fixture font moved to `scripts/remote-common.sh`** (`rc_fixture_font`), so the
  rows script can ship it beside a scene that draws text — to a Linux box and to
  Windows. `test-canvas-vulkan.sh` calls the same function.
- **No `CANVAS_DRAW_ENTRY_INVERSE` constant.** 172-G's Corrections said this letter would
  add it with its readers. Both emitters copy entry words by a table of
  `(word, byte offset)` pairs in which the translation words are already the literals
  2, 3, 5 and 6; the inverse words are 8–11 in the same table. A constant for four of
  eight entries of one table would not have made either emitter clearer.
- **One more golden moved than predicted**:
  `app-window-surface`'s macOS `.ncodesum`. That fixture is a window app that never
  draws on a canvas, but every macOS app binary carries the Metal shader source
  (`strings … | grep -c 'MfbDraw\|mfbFragment'` → 7 on a fresh build of it), and the
  shader text changed.
- **Runtime proof.** `examples/canvas-demo` is animated from wall-clock time, so a GPU
  dump and a software dump of it are different moments and cannot be compared. What
  was done: a copy of the demo with the spring scene's middle parallax group turned
  6 degrees ran headless on Metal for 14 s — `gpuFrames=634 frames=634`, so no frame
  was declined — and the dumped frame shows the layer's trunks leaning. The comparable
  proof is the reference scene, on four GPU configurations.
- **`rt_canvas_font` passed (33) in this letter's round at load average 34–50**, which
  is higher than when `bugs/bug-1024-*` was seen. Load alone is not its trigger.

## Summary

Two hand-written emitters and four shader stages change in step. The safety is that the
untransformed branch has a byte-identity gate on each backend, captured before any
edit, and the transformed branch has an oracle that 172-G already landed.
