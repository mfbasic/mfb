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
| plan-98-D complete (graphics thread + ring + deferred texture free) | `ls planning/completed/plan-98-D-*` → hit | NOT MET |
| D's frame-completion signal is renderer-swappable | plan-98-D Phase 4 acceptance met | NOT MET |
| C's tolerance comparator exists | plan-98-C Phase 2 acceptance met | NOT MET |
| Working tree builds | `cargo build` → pass | UNVERIFIED (run before starting) |

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
- UNVERIFIED until Phase 1 (post-D): the exact vertex-buffer layout D hands the renderer, the
  frame-completion hook shape (`lastCompletedFrame`), and how the atlas (white pixel + images) is uploaded.
  These are read from D's real code, not assumed.

### Measured populations

| What | Count | Command |
|---|---|---|
| Render pipelines | 1 (textured tinted quad + SDF branch) | this plan's §3 — the "one pipeline, many shapes" decision, mirroring C's single software path |
| Shaders to author | UNMEASURED (1–2) | resolve in Phase 1 — see shader Open Decision |
| macOS-specific render entry points behind D's boundary | UNVERIFIED | read D's renderer-swap seam (Phase 1) |

### Verified properties

- **The thread/ring/retirement boundary is renderer-agnostic** — VERIFIED by D's design
  (software fence is swappable). E confirms by reading D's seam in Phase 1 before coding.
- UNVERIFIED: that a `CAMetalLayer` on the A view resizes cleanly via `drawableSize` from the
  main thread while the graphics thread renders. Phase task proves it.

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

- [ ] Read D's renderer-swap seam and vertex/fence contract; record the real layout
      (resolves the `UNVERIFIED` rows above) in Corrections.
- [ ] Decide and implement the shader path (Open Decision); build-time compile to MSL.
- [ ] Create `CAMetalLayer`, device, queue, single pipeline; render one textured tinted quad;
      assert it matches the C software golden within tolerance headless (where macOS headless
      Metal is available; else on a window-server CI lane).

Acceptance: one quad renders via Metal and matches the software reference within the C
tolerance; the software backend still passes exact-match.
Commit: —

### Phase 2 — Full scene render + resize via drawableSize

- [ ] Render the full primitive set (Rect/Line/Polygon/Circle/Arc/RoundedRect/Image) from
      the `live` vertex buffer; atlas upload (white pixel + images).
- [ ] Dynamic-texture upload for `canvas::setBytes` (D's dirty flag): upload a dirty
      texture at frame start. To upload while a prior frame may still be sampling it, use a
      **per-texture ring** (one `MTLTexture` per frame-in-flight) or a blit/barrier — the
      GPU realisation of D's software staging. Coalesce multiple `setBytes` to one upload.
- [ ] Resize via `CAMetalLayer.drawableSize` from main, picked up on the graphics thread.
- [ ] Tests: the multi-primitive C golden scene (incl. the smiley Circle/Arc scene) matches
      within tolerance on Metal; a `setBytes` on an in-scene image shows updated pixels next
      frame with no tearing; resize repaints at the new size with zero worker involvement.

Acceptance: the full software golden scene matches within tolerance on Metal; `setBytes`
content updates appear next frame without tearing; resize is correct and worker-free.
Commit: —

### Phase 3 — Completion-handler → frame-completion counter (largest blast radius last)

- [ ] Hook the Metal command-buffer completion handler to advance D's `lastCompletedFrame`,
      replacing the software completion signal; a closed image's texture is freed only when
      `closed AND lastUsedFrame < lastCompletedFrame` (real Metal completion).
- [ ] Tests: the design race (publish → `canvas::destroyImage` → mid-record) on the Metal path
      frees the texture exactly once after Metal completion; the D race matrix passes on Metal.

Acceptance: the texture free is driven by real Metal completion; no use-after-free across
present/destroy/resize on Metal; D's race matrix green on the Metal path.
Commit: —

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
