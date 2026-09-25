# bug-670: on Metal, every glyph of a text run draws with the run's last glyph's bitmap

Last updated: 2026-09-21
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness

Status: Closed
Regression Test: tests/canvas/rt_canvas_metal.rs
(`every_glyph_of_a_text_run_draws_its_own_bitmap`)

On the Metal canvas renderer, a `canvas::Text` item with more than one distinct
glyph draws as diagonal hatching. Only the run's last glyph comes out right:
"LEVEL 01" renders as noise and a clean "1", and "BUGS" as mostly "S". Found
when the `examples/bugs/canvas` score bar was drawn on the GPU.

## STATUS: FIXED (738a30f9a; goldens in e920ede36)

Shipped as designed. The copy loop is limited to SCRATCH[0..8]; a first cut
used 10–12, which realise to the draw's callee-saved loop registers, and the
second text item vanished. Validated: canvas suites 15 binaries / 183 tests,
`cargo test --bin mfb` 4285, artifact-gate all 2088 goldens / 0 diffs, full
suite 215 binaries / 5926 passed / 0 failed, and `examples/bugs/canvas` reads
correctly on the GPU in a real window.

**Correct behavior:** each glyph samples its own cached bitmap, so a GPU text
run matches the software oracle within `Tolerance::GPU_DEFAULT` in the rows it
occupies.

References:

- plan-116-H (a text item became one instanced draw).
- `.ai/canvas-threading.md`; `mfb spec app canvas`.

## Failing Reproduction

`cargo test --release --test rt_canvas_metal every_glyph` at 9e728044e plus the
session's app commits:

- Observed: `2166 of 54000 pixels differ (max channel delta 235)` in the text band.
- Expected: within `GPU_DEFAULT`.

The same frame dumped headless (`MFB_CANVAS_GPU=1 MFB_MACAPP_HEADLESS=1
MFB_CANVAS_DUMP=…`) shows the hatching, so it is not Retina- or window-specific.

## Root Cause

`src/target/macos_aarch64/app/metal.rs:emit_glyph_publish` still binds each
glyph's bitmap with `setFragmentBytes:length:atIndex:2` inside the per-glyph loop.
Since plan-116-H the loop only *publishes* blocks, and the draw list issues the
whole run afterwards as one instanced call. At that point index 2 holds the last
glyph's bytes, and every instance indexes them with its own `misc.w`, which gives
the hatching. The Vulkan twin (`runtime/canvas/vulkan.rs:emit_glyph_publish`) was
never affected: it copies each bitmap into a frame-wide glyph region and stores
the offset in the glyph's block (`ITEM_ARC_EDGE_BASE`).

It escaped the tests for two reasons. The GPU text tests draw with a fixture font
whose glyphs share one shape, so the wrong bitmap looked right. And the other
Metal tests compare whole 900x640 frames within a 2% pixel budget, which a line
of 22 px text fits inside.

## Goal

- The regression test passes, and all of `rt_canvas_metal`, `rt_canvas_font` and
  `rt_canvas_golden` still pass.
- `examples/bugs/canvas` shows a readable score bar on the GPU.

### Non-goals

- No change to the software renderer or its goldens (the oracle).
- Vulkan is unchanged; it already has the right design.

## Blast Radius

- `metal.rs:emit_glyph_publish` and the MSL glyph arm: fixed here.
- `helper_render.rs:__canvas_metalRenderable`: its per-glyph 4 KiB cap existed
  only because of `setFragmentBytes:`. It becomes a per-frame cap, as Vulkan's is.
- Vulkan (`vulkan.rs`): unaffected, as above.

## Fix Design

Mirror Vulkan inside Metal's existing frame buffer:

- A fourth region after the gradient stops: `METAL_GLYPH_BASE_WORDS`, holding
  `METAL_MAX_FRAME_GLYPH_SAMPLES` samples at one per 32-bit word.
- A per-frame glyph cursor (`OFF_GLYPH_CURSOR`), reset with the other cursors.
- Each glyph's bitmap is copied to `contents + (GLYPH_BASE + cursor) * 4`, and
  the cursor is stored in the block's `arc.z` (`ITEM_ARC_EDGE_BASE`) before
  publishing.
- The shader reads `edges[METAL_GLYPH_BASE + item.arc.z + iy * w + ix]`. The
  `[[buffer(2)]]` binding and the per-glyph `setFragmentBytes:` go away.

Rejected: issuing one draw per glyph again. That brings back the double-draw
plan-116-H removed, and every glyph pays a command-buffer copy.

## Phases

### Phase 1 — failing test

- [x] `every_glyph_of_a_text_run_draws_its_own_bitmap`: red as above.

Commit: 738a30f9a

### Phase 2 — the fix

- [x] Region constants, predicate, emitter, shader, and the region-layout unit test.

Commit: 738a30f9a

### Phase 3 — validation

- [x] `rt_canvas_metal`, `rt_canvas_font`, `rt_canvas_golden`, and `cargo test --bin mfb`.
- [x] Full artifact gate; regenerate canvas/app goldens the helper change moves.
- [x] `examples/bugs/canvas` screenshot on the GPU.

Commit: e920ede36
