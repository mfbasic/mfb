# bug-1085: a banded shape or path clip (32 or more edges) inside a `canvas::Group` whose matrix shrinks disagrees with the software oracle on Metal

Last updated: 2026-10-12
Effort: medium
Severity: MEDIUM
Class: Correctness

Status: Fixed (bs24-agent-a, bug-staging-24)
Regression Test: `tests/canvas/rt_canvas_metal.rs::banded_shapes_in_shrinking_groups_match_the_software_oracle_on_metal`; Vulkan row `bandgroups` in `scripts/test-canvas-gpu-rows.sh`

## Cause (coordinator, bug-staging-24)

- Rule: the band index is exact only while every one of the five samples lies within the band's reach of the centre: `r = half + 0.5 + 1.5·F`, `F` the Frobenius norm of the COMPOSED inverse the shader samples with (`emit_band_index`'s own doc, and `metal.rs:shapeCoverage`'s comment: "like the shape's it does not cover a group's matrix, which the band index never sees").
- Where: `emit_band_index` (`src/target/macos_aarch64/app/metal.rs`, `src/codegen/runtime/canvas/vulkan.rs`) builds `F` from the item's inverse only; a block is built once a pass and may be drawn under several group matrices, so the builder cannot know the composed `F`.
- Fix: the band header records the `F` its reach was built for; the shader's band lookup (`edgeDistance`'s band path, the path clip's too, Metal and GLSL line for line) takes the band only when the pixel's composed norm (`norm`, already computed in `shapeCoverage`) is within it, and otherwise walks every edge, which is exact (the band-count-0 path that already exists). Deletes the "does not cover a group's matrix" caveat from both comments and from `mfb spec app canvas`. SPIR-V regenerates (`MFB_SPIRV_PORT=2226 scripts/regen-spirv.sh`).

Found during `bug-staging-23` (2026-10-12) and placed in a later phase.

**The single correct behavior a fix produces:** the scene below draws on the GPU within
`Tolerance::GPU_DEFAULT` of the software frame.

## Reproduction (main `3745e626a`, Metal)

`bugs/repro/bug-1085/group_band.mfb`: a 256-edge ring of radius 3000, as a polygon and as a
path clip, inside groups scaled 0.05.

```
mfb build --app --debug <dir>
MFB_MACAPP_HEADLESS=1 MFB_CANVAS_SYNC=1 MFB_CANVAS_GPU=0 MFB_CANVAS_DUMP=sw.rgba  <binary>
MFB_MACAPP_HEADLESS=1 MFB_CANVAS_SYNC=1 MFB_CANVAS_GPU=1 MFB_CANVAS_DUMP=gpu.rgba <binary>
python3 scripts/rgba_compare.py sw.rgba gpu.rgba 900
worst=109 differing=0.1193% first-beyond-tolerance=(161, 155, '251c07ff', '211806ff')
```

Found by bs23-agent-b while proving bug-995; re-run by the coordinator with the same line.
A 40-edge star at the same scale passes, and so does a 0.5-scale group (the agent's runs).

## Cause, by reading and NOT confirmed

`emit_band_index` (`metal.rs`, `vulkan.rs`) builds a band's reach from the ITEM's inverse
only, while under a group matrix the five samples sit `gi * 0.5` apart in the group's space:
a shrinking group puts neighbours outside the band the centre chose. Vulkan not run.

## Fix (bs24-agent-a, bug-staging-24)

The Cause note's design, unchanged.

- **The band header records the norm its reach was built for.** Each backend's
  `emit_band_index` (`metal.rs`, `vulkan.rs`) writes `F`, the Frobenius norm of the item's
  own inverse (0 untransformed), in 16.16 rounded down, into the header's fifth word:
  `ITEM_BAND_NORM` (`capsLo.w`, the residual of `arcCaps.w`, which a band header has none
  of and only an arc reads) for the item's edges, `ITEM_CLIP_BAND_NORM` (`gradFocal.w`,
  which nothing else writes; a clip can be on an arc) for a path clip's. The `none` path
  writes 0 there with the rest of the header. The item block's size and layout are
  otherwise unchanged.
- **The shader's band lookups take the band only when the reach covers the pixel.**
  `polygonBand` and `clipBandAt` (MSL and GLSL, line for line) take the pixel's composed
  norm `norm` (computed in `shapeCoverage` before the lookup now, 0 for a pixel that
  takes one sample) and return "every edge" unless `0.75·norm <= F`: the builder adds
  `2F` where the five-sample argument needs `1.5·N`. Every edge is exact (the band-count-0
  path). The PolyLine gradient's centre-only lookup in `main` passes 0.
- The "does not cover a group's matrix, which the band index never sees" caveat is gone
  from `metal.rs:shapeCoverage`; both `emit_band_index` docs and `mfb spec app canvas`
  state the group-matrix rule. SPIR-V regenerated on box 2226 (`frag.spv` only; `.vert`
  came back unchanged).

Performance consequence (expected, by design): an UNTRANSFORMED banded polygon or path
clip drawn under a group with a matrix (any turn or scale) now walks every edge per pixel,
since its reach (`F = 0`) covers no neighbour sample. Pixels are unchanged; time per pixel
grows with edge count for those items only. Translate-only groups are unaffected (no
matrix, one sample).

Commands run (all with `CARGO_TARGET_DIR=/tmp/bs24-agent-a`):

- RED, Metal: `cargo test --release --test rt_canvas_metal banded_shapes_in_shrinking`
  failed before the fix: `449 of 576000 pixels differ (max channel delta 109)`.
- RED, Vulkan: `scripts/test-canvas-gpu-rows.sh /tmp/main-dbg-target/release/mfb --target
  linux-aarch64 --rows bandgroups` → `FAIL ... worst=109 differing=0.1396%`.
- GREEN: `cargo test --release --test rt_canvas_metal` 73 passed;
  `rt_canvas_golden` 28 passed; `rt_canvas_curves` 40 passed.
- `scripts/test-canvas-gpu-rows.sh <mfb> --target windows-x86_64 --rows
  bandgroups,xformclips,groupaffine,groupsblack,polygonsblack,pathclips,polylinesblack,curves,xformblack,huge`
  all pass (`bandgroups` worst=1 differing=0.0950%); `--target linux-aarch64` (all rows).
- `cargo test --release --bin mfb -- vulkan:: metal:: canvas::` 99 passed.
- `scripts/spec-census.sh --citations app` / `--render app` / `--links app`: 0 missing,
  0 leaks, 0 unresolved.
- `scripts/artifact-gate.sh <mfb> all`: 5 diffs, all expected (below).

Goldens expected to change (sha256 `.app.ncodesum`; not regenerated here):

- `tests/syntax/app/app-mouse-surface` — `macos-aarch64`, `linux-aarch64`,
  `linux-x86_64`, `windows-x86_64` `.app.ncode`: the band builder's new norm stores in
  both backends' draw code, plus the MSL string (macOS) or the fragment SPIR-V blob and
  its size immediate (Linux/Windows). Localized on `linux-x86_64` by diffing main's and
  this build's `-ncode` dumps: 20 changed lines, all the SPIR-V symbol, its size and the
  `emit_band_index` stores (`rsp+364` and `rsp+492`: the item block at `rsp+128`, plus 236 and 364).
- `tests/syntax/app/app-window-surface` — `macos-aarch64` `.app.ncode`: the
  `_mfb_macapp_metal_shader` string only (main's dump reproduces the committed sha; the
  one differing symbol is the shader).
