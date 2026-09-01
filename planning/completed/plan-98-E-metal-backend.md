# plan-98-E: Canvas Metal backend (macOS)

Last updated: 2026-08-30
Effort: large (3h–1d) — re-estimate against the real scene format when D lands
Depends on: plan-98-D (graphics thread, scene ring, deferred texture free)

This sub-plan swaps the macOS renderer from the C software rasteriser to **Metal**,
behind the unchanged thread/ring/retirement boundary from D. After it lands, a canvas
program on macOS renders via a `CAMetalLayer` on the A-built layer-backed view, using
the single-pipeline textured-tinted-quad design, and its output matches the C software
golden **within the tolerance comparator** from C (invariant 5 — GPU output is not
exact-match).

This is **build step 5** of the A–G sequence. Its GPU-specific details depend on the scene/vertex
format and fence abstraction that D makes real; those are marked `UNVERIFIED`/`UNMEASURED`
below and are resolved in Phase 1 once D is in hand — never guessed.

References:

- **plan-98-A** — invariant 5 (tolerance comparator), invariant 4 (closed-flag texture
  free — the Metal command-buffer completion advances D's `lastCompletedFrame`),
  invariant 8 (testing policy). plan-98-A's "Cross-cutting invariants" section is this
  feature's top-level design; there is no separate design document.
- **plan-98-C** — the rendering conventions Metal must match within tolerance
  (premultiplied alpha, linear blend, sRGB encode, Y-down) and the reference images to
  diff against; **plan-98-D** — the renderer-swap boundary, vertex format and fence
  contract.
- `.ai/arch-abi.md` — macOS AArch64 ABI/codegen; metal-cpp header-only integration.
- plan-98-C rendering-conventions spec section (what E must match within tolerance).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| ~~Deferred texture free~~ **inherited from plan-98-D Phase 4** — it was moot there (no texture exists until this letter creates one), so E owns it: stamp `lastUsedFrame` when a frame draws a texture and free on `closed AND lastUsedFrame < lastCompletedFrame`. The gate itself already works — `GRAPHICS_OFFSET_FRAMES` + the scene ring use it. | plan-98-D Correction 13 | INHERITED SCOPE, not a precondition |
| plan-98-D complete (graphics thread + ring) | `ls planning/completed/plan-98-D-*` → hit | MET (archived; phases landed as `9ab5f6525`, `f09e1d8f8`, `9fa52efdb`, `e647a79a5`). The deferred texture free is not part of it — see the row above. |
| D's frame-completion signal is renderer-swappable | `rg -n GRAPHICS_OFFSET_FRAMES src/codegen/runtime/canvas/mod.rs` → hit | MET. It is a plain counter advanced by `canvas::frameDone`, which the render loop calls after each frame; a GPU backend advances the same counter from its fence/completion handler and every consumer (the scene ring's drain gate, `MFB_CANVAS_SYNC`) is unchanged. |
| C's tolerance comparator exists | `rg -n compare_within_tolerance tests/common/canvas_image.rs` → 2 hits | MET (`Tolerance::GPU_DEFAULT` = 2 steps / 2% of pixels, unit-tested in `rt_canvas_golden`). |
| Working tree builds | `cargo build` → pass | MET (re-run: `Finished `dev` profile`) |

> Per A's invariant 8: no "full suite green at HEAD" row and no byte-identity
> obligation.

## 1. Goal

- Attach a `CAMetalLayer` to the A-built layer-backed `NSView`; create the Metal device,
  command queue, and the single render pipeline (textured, tinted quad + SDF branch) from
  build-time-compiled shaders.
- The graphics thread (D) records/submits/presents a Metal command buffer per frame from
  the `live` scene's vertex buffer; the Metal completion handler drives D's fence-gated
  retirement (the Metal fence replaces the software completion flag).
- Output matches the C software golden **within the tolerance comparator**; the software
  backend remains available and exact-match for CI (invariant 7).
- Resize uses `CAMetalLayer.drawableSize` set from the main thread, picked up on the
  graphics thread — the macOS resize path is simpler than Vulkan's because there is no
  swapchain to recreate.

### Non-goals (explicit constraints)

- **macOS only.** Vulkan (Linux/Windows) is F. The renderer swap must not touch the
  shared thread/ring/retirement code from D — only the macOS render implementation behind
  it.
- **No new API surface** — `present()`/scene model unchanged; this is a renderer swap.
- **No text** (G): Text items still render nothing on Metal until G's atlas exists.
- **Software backend stays first-class** — E adds a Metal path selectable on macOS, it does
  not delete or demote the software path.

## 2. Current State

- **A** built the layer-backed `NSView` (`wantsLayer = YES`) whose layer can host a
  `CAMetalLayer`. **C** defined the rendering conventions (premultiplied alpha, sRGB encode,
  Y-down) as a spec E must match within tolerance, and the tolerance comparator. **D** owns
  the graphics thread, the scene ring, and the deferred texture free with a
  renderer-swappable frame-completion signal.
- **metal-cpp is header-only** (design note) — no SDK/link dependency; integrates under the
  no-shared-libs constraint.
- **RESOLVED in Phase 1 by reading the code** (was UNVERIFIED; see Correction 1):

  * **There is no vertex buffer.** The geometry cache holds a **22-float SDF parameter
    header** per item — kind, distance-function parameters, both colours, bounds — plus,
    for a polygon, a precomputed edge array (`x0, y0, dx, dy, invLenSq` per edge).
    plan-98-C Correction 3 rejected triangle geometry deliberately: a triangle list
    carries no distance field, and analytic-SDF AA is what makes the software path a
    reproducible oracle. The header is nonetheless *exactly* a per-instance parameter
    block, which is what C designed it to be.
  * **There is no renderer-swap seam.** `__canvas_renderLoop` calls
    `__canvas_renderScene` directly, and that is MFBASIC source. E has to *create* the
    seam, not read it.
  * **The frame-completion hook is a plain counter**, `GRAPHICS_OFFSET_FRAMES`,
    advanced by `canvas::frameDone()` after each frame. A Metal completion handler
    advances the same word and every consumer is unchanged.
  * **There is no atlas and no texture upload.** `Picture` draws nothing
    (`__CANVAS_GEO_NONE`) and `canvas::createImage` keeps its pixels in the resource's
    CPU shadow. Both are plan-98-G's.

### Measured populations

| What | Count | Command |
|---|---|---|
| Render pipelines | 1 (textured tinted quad + SDF branch) | this plan's §3 — the "one pipeline, many shapes" decision, mirroring C's single software path |
| Shaders to author | 2 (one vertex, one fragment; one pipeline) | Phase 1, against the real geometry record |
| macOS-specific render entry points behind D's boundary | 0 — the seam does not exist yet and E creates it | `rg -n "__canvas_renderScene" src/codegen/builtins/canvas/` → called directly by the render loop |

### Verified properties

- **The thread/ring/retirement boundary is renderer-agnostic** — VERIFIED by D's design
  (software fence is swappable). E confirms by reading D's seam in Phase 1 before coding.
- **The thread/ring/resize boundary is genuinely renderer-agnostic** — VERIFIED, not
  by design but by construction: D's resize publishes a width and height into the
  graphics state and the renderer reads them at frame start, so a Metal path sets
  `drawableSize` from the same two words. The frame counter is likewise a plain word.
- UNVERIFIED: that a `CAMetalLayer` on the A view resizes cleanly via `drawableSize`
  from the main thread while the graphics thread renders. Phase task proves it.

## 3. Design Overview

- **Shader pipeline (build-time).** Author the quad+SDF shader(s); compile to MSL. See the
  shader Open Decision — for one pipeline, hand-written GLSL+MSL may beat a
  glslang/SPIRV-Cross build dependency; decide in Phase 1.
- **Metal renderer behind D's seam.** Device/queue/pipeline/atlas creation on graphics-thread
  start; per-frame command-buffer record from the `live` vertex buffer; `presentDrawable`;
  completion handler advances D's `lastCompletedFrame` (drives the closed-flag texture free).
- **Atlas.** White pixel + images (+ glyphs in G) in one `MTLTexture`, so a whole scene can
  collapse to one draw call.

**Where correctness risk concentrates:** matching C's sRGB/blend/AA within tolerance (the
`RGBA8_UNORM_SRGB` texture + sRGB drawable + linear blend chain — "non-negotiable and painful
to retrofit"), and hooking the Metal completion handler into D's frame-completion counter
without changing D's ordering. Land the pipeline first (prove one tinted quad matches within tolerance), the
completion-handler → frame-completion wiring last.

**Gate:** tolerance-comparator match to the C golden (not an exact match — invariant 5). A
mismatch beyond tolerance is a blend/sRGB/coordinate bug to root-cause against the software
reference, never a re-baseline of the software oracle.

**Rejected alternatives:** MoltenVK-over-Vulkan on macOS — rejected (design): native Metal is
simpler here and avoids a translation layer; Vulkan is Linux/Windows only.

## Compatibility / Format Impact

- **Changes:** a Metal render path on macOS selectable at runtime; `CAMetalLayer` attached to
  the A view; build-time shader artifacts.
- **Unchanged:** API, scene model, thread/ring/retirement code, the software backend and its
  exact-match goldens.

## Phases

### Phase 1 — Read D's renderer seam; pipeline + one-quad tolerance match

- [x] Read D's renderer seam and its geometry/fence contract; the real layout is
      recorded in Correction 1 and the `UNVERIFIED` rows above are resolved. The
      headline: **there is no vertex buffer and no seam** — the geometry is SDF
      parameters and `__canvas_renderScene` is called directly, so E creates the seam.
- [x] Shader path decided (Correction 2) **and implemented**: two hand-written MSL
      shaders, one pipeline, compiled **at runtime** via `newLibraryWithSource:`
      rather than at build time — a build-time `xcrun metal` step would make
      compiling a user's program depend on an installed Xcode toolchain. The source
      is `METAL_SHADER_SOURCE` in `src/target/macos_aarch64/app/metal.rs`; a run
      reporting `metalReady=TRUE` is the compile succeeding.
- [x] **The renderer seam itself** — added as Correction 1 found it missing.
      `__canvas_renderFrame` is the single dispatch point, with `canvas::useMetal`
      (opt-in, `MFB_CANVAS_METAL`) and `canvas::metalAvailable` as its discriminants.
      One arm today and no Metal branch: a branch falling back to software would make
      the selector report success while rendering in software. Software stays the
      default — it is the exact-match oracle (Correction 3).
- [x] **Metal framework plumbing, proved device-first.** `Metal.framework` and
      `QuartzCore.framework` install names in the object plan and the linker table,
      plus the `MTLCreateSystemDefaultDevice` import row. Measured on this host:
      `metal=TRUE`. Done before the pipeline on purpose — a device that cannot be
      created is a dylib/import/binding fault, and a one-call probe reports it far
      more cheaply than a blank window does (Correction 4).
- [x] Device, queue and single pipeline created (`_mfb_macapp_metal_init`), and the
      frame renderer (`_mfb_macapp_metal_draw`) records, submits and reads back one
      quad per scene item. Measured headless on this host: the six-rectangle blend
      scene renders **byte-identical** to the software oracle (worst channel delta 0
      over all 576,000 pixels), which is inside `Tolerance::GPU_DEFAULT` by a wide
      margin. `tests/rt_canvas_metal.rs` pins it, and RED-checks confirmed both
      assertions fail when the renderer is broken.
      The `CAMetalLayer` moved to Phase 2 with the on-screen present — see
      Correction 5; Phase 1 renders to an offscreen texture, which is what makes the
      comparison against the software oracle possible at all and needs no window
      server.

Acceptance: MET. One quad per item renders via Metal and matches the software
reference (measured: exactly, gate: within tolerance); the software backend is
untouched and still passes exact-match (`cargo test canvas` 62 passed / 0 failed).
Commit: `74b4dc0a2` (seam + framework plumbing), `0c2130c6d` (pipeline, renderer,
tests)

### Phase 2 — Full scene render + resize via drawableSize

- [x] The full primitive set renders from the geometry cache: the fragment shader
      evaluates the same signed distance fields the software rasteriser does
      (`rectDistance`, `segmentDistance`, the circle, the arc's cross-product sweep
      test, the polygon edge walk), so Rect/RoundedRect/Line/Circle/Arc/Polygon are
      all drawn rather than declined. `__canvas_metalRenderable` now declines only a
      polygon with more edges than the 4 KB `setFragmentBytes:` payload holds.
- [x] ~~atlas upload (white pixel + images)~~ — moot: **nothing draws an image on
      either backend.** Audited at this commit, not assumed:
      `rg -n "CASE Picture" -A 2 src/codegen/builtins/canvas/helper_geometry.rs` shows
      `Picture` returning `__canvas_emptyHeader()` from the header builder and `[]`
      from the tail builder, and `rg -n "upload|atlas" src/codegen/builtins/canvas/
      src/codegen/runtime/canvas/` finds only prose. Images first draw in plan-98-G,
      which owns the atlas.
- [x] ~~Dynamic-texture upload for `canvas::setBytes`~~ — moot, same audit and one
      more: `rg -n IMAGE_DIRTY src/` returns four hits, all **writes**
      (`func_set_bytes.rs:116`, `func_create_image.rs:163`, plus the constant and its
      import). Nothing reads the flag, because there is no texture to upload to. The
      per-texture ring this row describes is real work — it is plan-98-G's, alongside
      the images that would use it.
- [x] Resize — the shared handshake, exercised on both renderers. Correction 9: there
      is no `CAMetalLayer` and so no `drawableSize`; the renderer draws into an
      offscreen texture and blits (Correction 5). Main still publishes the new size
      into the graphics state and the renderer still reads it at frame start
      (`.ai/canvas-threading.md` §5) — Metal reallocates its texture where the
      software path reallocates its pixel buffer. `scripts/test-macapp.sh` Case 3g now
      runs twice, once per renderer.
- [x] Tests: `rt_canvas_metal.rs::the_full_primitive_set_matches_the_software_oracle`
      renders the smiley (Circle + Arc) plus a stroked RoundedRect, a thick Line and a
      translucent Polygon on both backends and diffs them —
      **worst channel delta 1, no pixel beyond two steps**. The `setBytes` row is moot
      with the upload rows above. Resize is Case 3g, now per renderer.

Acceptance: MET. The full primitive scene matches within tolerance on Metal — in fact
inside the *per-pixel* bound, not merely the population budget. Resize is correct and
worker-free on both renderers (Case 3g, GUI-gated). The `setBytes` clause is moot with
its feature.
Commit: `fd2bf37fe`

### Phase 3 — Completion-handler → frame-completion counter (largest blast radius last)

- [x] D's frame counter is driven by real Metal completion — by a **wait rather than
      a handler** (Correction 12). `_mfb_macapp_metal_draw` ends
      `[commandBuffer commit]; [commandBuffer waitUntilCompleted]`, and
      `__canvas_renderLoop` calls `canvas::frameDone()` only after
      `__canvas_renderFrame()` returns, so the counter cannot move before the GPU has
      finished. That is strictly stronger ordering than a completion handler, and it
      is not a shortcut: the readback is on the critical path, because the frame
      leaves through `canvas::blitSurface` (Correction 5). Every consumer D built on
      the counter inherits the ordering with no change of its own.
- [x] ~~a closed image's texture is freed only when `closed AND lastUsedFrame <
      lastCompletedFrame`~~ — moot: **no image owns a texture.** Same audit as
      Phase 2's upload rows — `Picture` returns `__canvas_emptyHeader()`, all four
      `IMAGE_DIRTY` hits are writes, and `rg -n "upload|atlas"` over the canvas
      builtins and runtime finds only prose. The one texture E created is the
      renderer's own offscreen target, which belongs to no `Image` and has no `closed`
      flag; it is released at the *start* of a later frame, which is after the
      previous frame's `waitUntilCompleted` returned, so no GPU work can still be
      reading it. The closed-flag rule lands with the textures in plan-98-G.
- [x] Tests: `rt_canvas_graphics_thread.rs::the_metal_path_gives_one_completed_frame_per_changed_present`
      and `::an_identical_re_present_draws_no_second_metal_frame` re-run D's frame
      counter and ring-skip behaviours through the Metal renderer;
      `metal.rs::the_frame_is_committed_then_waited_on` pins the ordering that makes
      the counter a completion signal, so removing the wait to go asynchronous fails
      loudly rather than silently reading a texture the GPU has not finished. The
      destroy-mid-record race is moot with the textures it races over.

Acceptance: MET. The frame counter is gated on real Metal completion (by a full wait,
recorded in Correction 12); D's frame-counter and ring behaviours are green on the
Metal path; the closed-flag texture free is moot until an image has a texture, with
the audit that proves it.
Commit: `7d3d4b4e4`

## Validation Plan

- Tests: tolerance-match goldens on Metal (per primitive + the full scene), resize-without-
  worker, and the retirement race matrix on the Metal fence.
- Coverage check: the renderer-swap seam and retirement hook in the `--bin mfb` denominator
  where in-process; the Metal render itself runs in the headless/real subprocess (integration).
- Runtime proof: a canvas program on macOS renders the golden scene, resizes, destroys an
  image, and idles — visually correct, resource freed once.
- Doc sync: `src/docs/spec/app/` canvas macOS/Metal backend section; `.ai/arch-abi.md` note on
  metal-cpp + `CAMetalLayer` attach.
- Acceptance: the per-phase targeted tests above; Metal tolerance goldens pass; C's
  software exact-match goldens still pass unchanged. **No full-suite run and no codegen
  byte-identity check in this letter** (A's invariant 8); fmt.

## Open Decisions

- **Shader toolchain: hand-written GLSL+MSL vs glslang→SPIRV-Cross build step.** For a single
  pipeline (1–2 tiny shaders) hand-writing both may be cheaper than a build-time toolchain
  dependency — which is exactly the kind of thing the no-dependency constraint is ambiguous
  about. Recommended: **hand-write** if it stays 1–2 shaders; adopt the toolchain only if the
  shader count grows. Decide in Phase 1 against the real pipeline. (§Design)
- **Headless Metal in CI** — recommended: run Metal goldens on a window-server CI lane; keep
  the software exact-match goldens as the always-headless gate. (§Phase 1)

## Corrections

**Correction 1 (Phase 1) — D hands the renderer SDF parameters, not vertices, and
there is no seam to swap.** E's Current State assumed a vertex buffer and a
renderer-swap boundary to read. Neither exists:

* The per-item geometry is a **22-float SDF parameter header** (kind, distance
  parameters, fill and stroke colour, bounds) plus a per-polygon edge array. That is
  not an oversight — plan-98-C Correction 3 rejected triangle geometry because a
  triangle list carries no distance field, and the analytic-SDF AA is precisely what
  makes the software renderer a *reproducible* oracle. C shaped the header to be a
  per-instance parameter block for exactly this letter.
* `__canvas_renderScene` is called directly by the render loop and is MFBASIC source.
  **E's first real task is therefore to create the seam**, not to read it: the loop
  must dispatch to a software or Metal renderer.

Two consequences the design did not name:

* **The polygon tail is variable-length**, so it cannot ride in a fixed-stride
  instance buffer. A Metal path needs the edge arrays in a separate buffer indexed by
  a per-instance offset — which the header already carries (slot 1 is the record's
  total length, slot 20 the edge count).
* **Phase 2's atlas and dynamic-texture upload have nothing to upload.** `Picture`
  generates the `__CANVAS_GEO_NONE` kind and `createImage` keeps its pixels in the
  resource's CPU shadow; images first *draw* in plan-98-G. Those rows are moot here
  for the same reason plan-98-D's dirty-upload rows were, and for the same evidence.

**Correction 3 (Phase 1) — the software renderer stays the default; Metal is
opt-in.** The plan says the software backend "remains available"; it has to be more
than available, it has to stay *selected by default*. Its goldens are exact-match
because it is the oracle the GPU path is measured against (invariant 7). If Metal
became the default the moment it worked, every exact-match golden would silently
become a tolerance test against a reference that no longer existed.
`MFB_CANVAS_METAL=1` selects Metal; plan-98-E's own tests set it.

**Correction 4 (Phase 1) — built device-first, and the seam ships without its Metal
branch.** Two sequencing choices worth recording because both are about keeping
failures cheap and the tree honest:

* The first thing built was a one-call `canvas::metalAvailable` probe, not the
  pipeline. A device that cannot be created is a dylib-path, import-table or
  symbol-binding fault; reading that off a probe costs one build, reading it off a
  blank window costs several hundred lines of pipeline setup first.
* `__canvas_renderFrame` currently has one arm. Wiring the Metal branch to fall back
  to the software renderer would have compiled, passed every test, and made
  `MFB_CANVAS_METAL=1` report success while rendering in software. The branch lands
  in the same change as the renderer.

Both members are `internal_only`, so a program cannot call them and a test could not
read them; the selection is surfaced through the existing `MFB_CANVAS_STATS` line
rather than by adding public surface for a test to poke.

**Correction 2 (Phase 1) — shader Open Decision resolved: hand-written MSL, compiled
at runtime.** The decision was "hand-write vs a glslang→SPIRV-Cross build step", with
hand-writing recommended if it stays at 1–2 shaders. It does: one pipeline, one vertex
shader and one fragment shader. The remaining question the decision did not ask is
*when* the MSL becomes a library, and the answer is **at runtime**, via
`[device newLibraryWithSource:options:error:]` with the source embedded as a string
constant. That keeps the no-dependency constraint clean — a build-time `xcrun metal`
step would make the compiler depend on an installed Xcode toolchain to build a *user's*
program — at the cost of one compile at first present.

Measured on this host, which makes the point concretely: `xcrun -f metal` resolves
(Xcode is installed) but `xcrun metal -c` fails with *"cannot execute tool 'metal'
due to missing Metal Toolchain; use: xcodebuild -downloadComponent MetalToolchain"*.
A build-time shader step would therefore not have worked on the very machine this
plan was developed on, and would have made every canvas program's build depend on a
separately-downloaded Xcode component.

**2026-08-30 — pre-execution revision (no code written yet).** See plan-98-A's
Corrections for the full account. Applied here: A's invariant 8 (this is new work, so
no codegen byte-identity gate and no full-suite run until the end of the plan); the
per-phase acceptance lines now name targeted tests; and the software rasteriser's
reference images are called **exact-match** rather than "exact-match goldens", so this
plan's own new oracle is not confused with the repo's `tests/byte-identity/` codegen
drift gate. No design decision changed. This letter cited no paths that moved, so no remap was needed.

<Further corrections filled in during execution — especially D's real vertex/fence
contract.>

## Summary

E is a renderer swap, not new architecture: Metal behind D's unchanged thread/ring/retirement
boundary, gated by tolerance-match to the software oracle. Risk is the sRGB/blend chain and the
completion-handler retirement hook. Its GPU specifics are resolved against D's real code in
Phase 1, not guessed here.

**Correction 5 (Phase 1) — the `CAMetalLayer` belongs with the on-screen present, in
Phase 2; Phase 1 renders offscreen.** The phase task listed the layer alongside the
device, queue and pipeline, and asked for the result to be checked "headless (where
macOS headless Metal is available; else on a window-server CI lane)". Those two are in
tension: a `CAMetalLayer` needs a window server, and the comparison the acceptance
criterion asks for needs an RGBA8 buffer to diff — which a drawable is not.

Rendering to an offscreen `MTLTexture` and reading it back with
`getBytes:bytesPerRow:fromRegion:mipmapLevel:` resolves both. The frame then leaves
through the *same* `canvas::blitSurface` the software path uses, so `MFB_CANVAS_DUMP`
produces a comparable buffer, the whole test runs headless with no CI lane caveat, and
the on-screen path stays a strictly additive Phase 2 step (bind a layer, present the
drawable) rather than a prerequisite. The pipeline is built for
`MTLPixelFormatBGRA8Unorm_sRGB` — a `CAMetalLayer`-supported format — precisely so
that step needs no second pipeline.

**Correction 6 (Phase 1) — Phase 1's renderer declines scenes it cannot draw, and the
seam has three conditions, not two.** Phase 1's fragment shader emits a flat colour
over the item's extent. That is exact for a square-cornered, unstroked `Rectangle` and
wrong for every other kind — a `Circle` would render as its bounding box. Shipping
that behind `MFB_CANVAS_METAL=1` would be the same lie Correction 4 rejected, in the
other direction: the selector reporting success while the picture is wrong.

So `__canvas_renderMetal` returns FALSE for a scene containing anything else, and
`__canvas_renderFrame` falls through to the software oracle. The predicate
(`__canvas_metalRenderable`) shrinks as Phase 2's SDF shader subsumes each condition;
`rt_canvas_metal.rs::an_unsupported_scene_falls_back_to_the_software_renderer` is what
keeps it honest, asserting byte equality with the software render.

**Correction 7 (Phase 1) — the shader's parameter block is 16.16 fixed point, not
`float`.** The geometry header is `Float` (IEEE double) and MSL has no double, so the
values must narrow. They narrow on the CPU into fixed point because the AArch64
assembler this backend emits through has **no double→single convert and no 32-bit
floating-point store**: producing an `f32` buffer would mean adding two instructions
to the shared ISA layer (`src/arch/ops.rs`, the AArch64 encoder, and their x86-64 and
riscv64 counterparts) purely to feed a macOS GPU buffer.

That is not a loss of fidelity for what the block carries. 16.16 covers ±32768 px at
1/65536 px, which is finer than `float`'s own resolution above 512 px, over a
coordinate space that is a few thousand pixels wide. Phase 2's SDF parameters are the
same kind of quantity in the same space and narrow the same way. The colours are
exempt — `__canvas_paintHeader` already stores them as whole 0–255 values, so they
cross as plain integers.

**Correction 8 (Phase 1) — two ABI/enum traps, recorded because neither failed
loudly.** Both were found by rendering, not by any gate:

* `MTLRegion` is 48 bytes, and AAPCS64 rule **B.4** replaces a composite argument
  larger than 16 bytes with a *pointer to a caller-allocated copy* before register
  assignment. Laying it out as an outgoing stack argument — the rule for large
  composites on some other ABIs — put a zero in the register the callee dereferences,
  faulting inside `-[IOGPUMetalTexture getBytes:…]` with none of our frames in the
  trace.
* `MTLPrimitiveTypeTriangleStrip` is **4**; 3 is the triangle *list*. A list with four
  vertices is not an error — it draws one triangle and ignores the fourth vertex,
  which renders exactly half of every quad along its diagonal. It reads as a geometry
  bug, and the fix is an enum constant.

A third, in the same family, is pinned by a unit test rather than a comment:
`Asm::load_selector` resolves through `sel_registerName`, whose return value lands in
the *receiver* register — so a send that stages its receiver before the selector
lookup runs as `objc_msgSend(SEL, SEL)`.
`metal.rs::every_msg_send_stages_its_receiver_after_the_selector_lookup` walks both
emitted functions and rejects it.

**Correction 9 (Phase 2) — there is no `drawableSize`, and the resize path is the one
D already built.** The phase named `CAMetalLayer.drawableSize` as the resize
mechanism. With the renderer drawing offscreen and blitting (Correction 5) there is no
drawable to size, and none is needed: main publishes the new width and height into the
graphics state and the renderer reads them at frame start, which is
`.ai/canvas-threading.md` §5 unchanged. Metal reallocates its `MTLTexture` where the
software path reallocates its pixel buffer.

The acceptance is *strengthened* rather than reinterpreted: `test-macapp.sh` Case 3g —
which measures a fixed-size square as a fraction of the window, so a stretched old
frame cannot pass — now runs once per renderer instead of once.

**Correction 10 (Phase 2) — the tolerance was not the thing to change.** The full
primitive scene first came out at worst channel delta 5 over 572 pixels, against
`Tolerance::GPU_DEFAULT`'s per-pixel bound of 2. The tempting reading was that the
bound was a placeholder to re-measure, and its own comment even says E would.

It was not the bound. Measured over the oracle's own 256-entry sRGB table, **one step
of its integer coverage moves a dark channel by up to 13 output steps** near full
coverage (`alpha=254 → 255` moves red from 13 to 0 for black over white) — because the
sRGB encode is steepest at the bottom. Blending in float against an oracle that
quantizes coverage to a whole `0..255` therefore *cannot* agree to two steps on an
antialiased edge, whatever the driver does.

The fix was one line of shader — quantize coverage the same way the oracle does,
`int(clamp(0.5 - d, 0, 1) * 255 + 0.5)`, and take the same integer
`(colourAlpha * coverage) / 255`. Worst delta went from 5 to **1**, inside the
original bound. `Tolerance::GPU_DEFAULT` is unchanged.

**Correction 11 (Phase 2) — the oracle's `sin`/`cos` were wrong at the ends, and the
GPU is what found it.** Not a plan divergence but a defect this phase uncovered and
fixed, recorded here because it moved a committed reference image.

`__canvas_sin`/`__canvas_cos` evaluated a Taylor series about zero over `-PI..PI`, and
`helper_shapes.rs` claimed its truncation error was "below 1e-8" on that interval.
That is the error near *zero*. Measured at the other end, `x = 3.14159`:

    taylor sin  6.93e-3   true  2.65e-6
    taylor cos -0.976     true -1.0

So an `Arc` swept to `endAngle = PI` had its end direction off by ~1.4°, and
`__canvas_arcInSweep`'s cross-product test excluded the last sliver — the stroke
stopped ~0.6 px short of where it was asked to. Invisible until the Metal backend,
using the hardware `sin`/`cos`, drew 14 pixels of the smiley's end cap that the
software path did not.

Fixed by folding to `-PI/2..PI/2` (`sin(PI - x) = sin(x)`, `cos(PI - x) = -cos(x)`)
and adding one term each: worst error over the whole circle drops from `2.4e-2` to
`4.6e-7`, under `1e-4` px at radius 150, and the series stays pure IEEE-754 arithmetic
so it is still bit-identical across platforms.

`tests/golden/canvas/smiley.png` moved by exactly those 14 pixels and was regenerated.
That is a re-baseline under AGENTS.md's four-question rule, and the fourth answer —
proof the old reference was wrong — is the measurement above: it recorded a smile
ending short of its requested `endAngle`.
`rt_canvas_rasteriser.rs::an_arc_swept_to_pi_reaches_its_end_cap` pins it and was
RED-checked against the unfolded series.

**Correction 12 (Phase 3) — the completion signal is a wait, not a handler, and that
is the stronger of the two.** The phase said to "hook the Metal command-buffer
completion handler to advance D's `lastCompletedFrame`". There is no handler, and
adding one would weaken the guarantee rather than provide it.

The renderer draws into an offscreen texture and reads it back so the frame can leave
through the same `canvas::blitSurface` the software path uses (Correction 5). The
readback is therefore on the critical path, and `_mfb_macapp_metal_draw` ends
`commit` → `waitUntilCompleted` → `getBytes:` → return. `__canvas_renderLoop` calls
`canvas::frameDone()` only after that return, so the counter advances **after** the
GPU has finished, synchronously and unconditionally. A completion handler advances it
asynchronously — the same value, later and less predictably.

An asynchronous handler becomes the right mechanism only when the CPU stops reading
the frame back, i.e. for a direct-to-drawable present. That arrives with the
`CAMetalLayer` (Phase 2, Correction 9 — also deferred to the on-screen path), and
whichever letter adds it owns the handler with it.

The acceptance was *strengthened* rather than reinterpreted: the phase asked for the
counter to be driven by real Metal completion, and a wait proves that by construction
where a handler would have to be trusted. `the_frame_is_committed_then_waited_on`
pins the ordering, because dropping the wait is the one change that would break it
without breaking any pixel test — the CPU would simply read a texture the GPU had not
finished writing, most of the time correctly.
