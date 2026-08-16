# plan-98-E: Canvas Metal backend (macOS)

Last updated: 2026-08-15
Effort: large (3h–1d) — re-estimate against the real scene format when D lands
Depends on: plan-98-D (graphics thread, scene ring, fence-gated retirement)

This sub-plan swaps the macOS renderer from the C software rasteriser to **Metal**,
behind the unchanged thread/ring/retirement boundary from D. After it lands, a canvas
program on macOS renders via a `CAMetalLayer` on the A-built layer-backed view, using
the single-pipeline textured-tinted-quad design, and its output matches the C software
golden **within the tolerance comparator** from C (invariant 5 — GPU output is not
byte-identical).

This is design-doc **build step 5**. Its GPU-specific details depend on the scene/vertex
format and fence abstraction that D makes real; those are marked `UNVERIFIED`/`UNMEASURED`
below and are resolved in Phase 1 once D is in hand — never guessed.

References:

- The design summary — "Platform Surfaces" (macOS/Metal), "Rendering Notes" (one
  pipeline, premultiplied alpha, sRGB, SDF), "Shaders".
- plan-98-A invariant 5 (tolerance comparator), invariant 4 (fence-gated retirement — the
  Metal fence replaces D's software completion flag).
- `.ai/arch-abi.md` — macOS AArch64 ABI/codegen; metal-cpp header-only integration.
- plan-98-C rendering-conventions spec section (what E must match within tolerance).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-98-D complete (graphics thread + ring + fence-gated retire) | `ls planning/completed/plan-98-D-*` → hit | NOT MET |
| D's fence abstraction is renderer-swappable | plan-98-D Phase 4 acceptance met | NOT MET |
| C's tolerance comparator exists | plan-98-C Phase 2 acceptance met | NOT MET |
| Full suite green at HEAD | `cargo test` → pass | UNVERIFIED |

## 1. Goal

- Attach a `CAMetalLayer` to the A-built layer-backed `NSView`; create the Metal device,
  command queue, and the single render pipeline (textured, tinted quad + SDF branch) from
  build-time-compiled shaders.
- The graphics thread (D) records/submits/presents a Metal command buffer per frame from
  the `live` scene's vertex buffer; the Metal completion handler drives D's fence-gated
  retirement (the Metal fence replaces the software completion flag).
- Output matches the C software golden **within the tolerance comparator**; the software
  backend remains available and byte-exact for CI (invariant 7).
- Resize uses `CAMetalLayer.drawableSize` set from the main thread, picked up on the
  graphics thread (design "Resize handshake — macOS is simpler").

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
  the graphics thread, the scene ring, and fence-gated retirement with a renderer-swappable
  fence.
- **metal-cpp is header-only** (design note) — no SDK/link dependency; integrates under the
  no-shared-libs constraint.
- UNVERIFIED until Phase 1 (post-D): the exact vertex-buffer layout D hands the renderer, the
  fence/completion-handler hook shape, and how the atlas (white pixel + images) is uploaded.
  These are read from D's real code, not assumed.

### Measured populations

| What | Count | Command |
|---|---|---|
| Render pipelines | 1 (textured tinted quad + SDF branch) | design "one pipeline, many shapes" |
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
  completion handler → D's fence-gated retirement.
- **Atlas.** White pixel + images (+ glyphs in G) in one `MTLTexture`, so a whole scene can
  collapse to one draw call.

**Where correctness risk concentrates:** matching C's sRGB/blend/AA within tolerance (the
`RGBA8_UNORM_SRGB` texture + sRGB drawable + linear blend chain — "non-negotiable and painful
to retrofit"), and hooking the Metal completion handler into D's retirement without changing
D's ordering. Land the pipeline first (prove one tinted quad matches within tolerance), the
completion-handler/retirement wiring last.

**Gate:** tolerance-comparator match to the C golden (not byte-identity — invariant 5). A
mismatch beyond tolerance is a blend/sRGB/coordinate bug to root-cause against the software
reference, never a re-baseline of the software oracle.

**Rejected alternatives:** MoltenVK-over-Vulkan on macOS — rejected (design): native Metal is
simpler here and avoids a translation layer; Vulkan is Linux/Windows only.

## Compatibility / Format Impact

- **Changes:** a Metal render path on macOS selectable at runtime; `CAMetalLayer` attached to
  the A view; build-time shader artifacts.
- **Unchanged:** API, scene model, thread/ring/retirement code, the software backend and its
  byte-exact goldens.

## Phases

### Phase 1 — Read D's renderer seam; pipeline + one-quad tolerance match

- [ ] Read D's renderer-swap seam and vertex/fence contract; record the real layout
      (resolves the `UNVERIFIED` rows above) in Corrections.
- [ ] Decide and implement the shader path (Open Decision); build-time compile to MSL.
- [ ] Create `CAMetalLayer`, device, queue, single pipeline; render one textured tinted quad;
      assert it matches the C software golden within tolerance headless (where macOS headless
      Metal is available; else on a window-server CI lane).

Acceptance: one quad renders via Metal and matches the software reference within the C
tolerance; the software backend still passes byte-exact.
Commit: —

### Phase 2 — Full scene render + resize via drawableSize

- [ ] Render the full primitive set (Rect/Line/Polygon/RoundedRect/Image) from the `live`
      vertex buffer; atlas upload (white pixel + images).
- [ ] Resize via `CAMetalLayer.drawableSize` from main, picked up on the graphics thread.
- [ ] Tests: the multi-primitive C golden scene matches within tolerance on Metal; resize
      repaints at the new size with zero worker involvement.

Acceptance: the full software golden scene matches within tolerance on Metal; resize is
correct and worker-free.
Commit: —

### Phase 3 — Completion-handler → fence-gated retirement (largest blast radius last)

- [ ] Hook the Metal command-buffer completion handler into D's fence-gated retirement,
      replacing the software completion flag; a referenced image is retired/freed only at
      Metal completion.
- [ ] Tests: the design race (publish → `image::destroy` → mid-record) on the Metal path frees
      exactly once at completion; the D race matrix passes with the Metal fence.

Acceptance: retirement is driven by real Metal completion; no use-after-free across
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
- Acceptance: full `cargo test`; Metal tolerance goldens pass; software byte-exact goldens
  unchanged; non-canvas byte-identity corpus unchanged; fmt.

## Open Decisions

- **Shader toolchain: hand-written GLSL+MSL vs glslang→SPIRV-Cross build step.** For a single
  pipeline (1–2 tiny shaders) hand-writing both may be cheaper than a build-time toolchain
  dependency — which is exactly the kind of thing the no-dependency constraint is ambiguous
  about. Recommended: **hand-write** if it stays 1–2 shaders; adopt the toolchain only if the
  shader count grows. Decide in Phase 1 against the real pipeline. (§Design)
- **Headless Metal in CI** — recommended: run Metal goldens on a window-server CI lane; keep
  the software byte-exact goldens as the always-headless gate. (§Phase 1)

## Corrections

<Filled in during execution — especially D's real vertex/fence contract.>

## Summary

E is a renderer swap, not new architecture: Metal behind D's unchanged thread/ring/retirement
boundary, gated by tolerance-match to the software oracle. Risk is the sRGB/blend chain and the
completion-handler retirement hook. Its GPU specifics are resolved against D's real code in
Phase 1, not guessed here.
