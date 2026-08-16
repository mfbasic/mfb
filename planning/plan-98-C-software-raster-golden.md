# plan-98-C: Canvas software rasteriser + golden-image harness

Last updated: 2026-08-15
Effort: large (3h–1d)
Depends on: plan-98-B (scene model, arena, hashing, geometry cache stub)

This sub-plan makes the geometry cache's generation real for a software backend and
adds the golden-image test harness. After it lands, `canvas::present(items)` in
`Mode.Canvas` renders the scene to an in-memory RGBA buffer that is blitted to the
platform surface built in A, and a golden-image test compares that buffer to a stored
reference **byte-identically** (the software backend is deterministic). This is the
permanent correctness oracle and the headless CI path — canvas mode can now render a
real picture with no GPU.

This is design-doc **build step 3**, and it establishes cross-cutting invariant 5
(GPU goldens tolerance-based; software golden byte-identical) for E/F to consume.

References:

- The design summary — "Software rasteriser + golden-image harness" (build step 3),
  "Rendering Notes" (premultiplied alpha, sRGB, Y-down), "Curves via analytic SDF".
- plan-98-A invariants 5 and 7 (software backend first-class, tolerance policy).
- `.ai/testing-gates.md` — byte-identity + golden harness conventions.
- The existing headless blit paths: `MFB_MACAPP_HEADLESS`, GTK-headless,
  `MFB_WINAPP_HEADLESS`; `scripts/snap-term.py` / `scripts/snap-macos.py`.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-98-B complete (scene arena, hashing, cache mechanics, resource table) | `ls planning/completed/plan-98-B-*` → hit | NOT MET |
| Geometry cache miss reaches a generation hook | plan-98-B Phase 3 acceptance met | NOT MET |
| Full suite green at HEAD | `cargo test` → pass | UNVERIFIED |

## 1. Goal

- A **software rasteriser** turns cached geometry into pixels: fill a runtime-owned
  RGBA8 buffer for the current surface size using **premultiplied alpha, linear
  blending, Y-down top-left pixel coordinates** (design "Rendering Notes"). Rectangles,
  lines, polygons render via tessellation/stroke expansion (now real, feeding the B
  cache); RoundedRect renders via analytic SDF (circles for free at `radius =
  min(w,h)/2`). Text is still stubbed (G).
- The rendered buffer is blitted to the A-built surface on the UI thread (reusing the
  existing headless/real blit paths per platform).
- A **golden-image harness** renders a fixed scene headless to the buffer and compares
  byte-identically to a stored PNG/raw reference; a mismatch is a bug-hunt trigger,
  never a re-baseline.
- **Invariant 5 is codified:** the software golden is byte-identical; GPU backends (E/F)
  will diff against it with a documented tolerance.

### Non-goals (explicit constraints)

- **No graphics thread, no scene ring, no GPU.** Rendering is synchronous on the
  present/UI path (the ring is D; GPU is E/F). This is deliberate: get the pixel oracle
  right before threading and GPU land.
- **No text rendering** (G). Text items render nothing (or a debug box) and are excluded
  from goldens until G.
- **No sRGB *hardware* linearisation** — the software path does sRGB encode/decode in
  code to match what the GPU sRGB surface will do, so the software golden is a faithful
  reference. Document the exact encode so E/F can match within tolerance.
- No change to the scene model, resource table, or non-canvas codegen.

## 2. Current State

- **B provides:** the scene arena, per-item hashes, and a geometry cache whose miss
  path currently inserts an empty vertex range. C replaces that stub with real geometry
  generation feeding `params[]` → vertices.
- **Blit paths exist per platform** from A's surface: macOS layer-backed view, GTK
  window, Win32 HWND. UNVERIFIED whether the A surfaces expose a CPU-buffer blit entry
  (they were built empty). C adds a "blit this RGBA buffer" UI-thread operation to each,
  reusing the term draw/paint marshaling precedent
  (`src/target/linux_gtk/term_draw.rs`, macOS `term_view.rs`, Windows memDC BitBlt).
- **No automated golden-image diff exists today** (research §7): `snap-term.py`/
  `snap-macos.py` produce PNGs but nothing wires a diff into `cargo test`. C builds that
  wiring for the software buffer (which, being deterministic and headless, doesn't need a
  window server).

### Measured populations

| What | Count | Command |
|---|---|---|
| Primitive kinds needing a software rasteriser path | 5 (Rect, Line, Polygon, RoundedRect/SDF; Image blit) | design variant set minus Text |
| Platform blit entry points to add | 3 | A's three surfaces |
| Existing golden-image diffs in the Rust suite | 0 | research §7 (`rg -n "golden" tests/ \| rg -i png`) — run to confirm |

### Verified properties

- **A deterministic software rasteriser can produce byte-identical goldens.** VERIFIED
  in principle (no floating driver, fixed rounding) — contingent on C fixing the sRGB
  encode and AA math in integer/deterministic form. This is the whole reason the software
  backend is the oracle (invariant 5/7).
- UNVERIFIED: whether analytic-SDF AA (`fwidth`/`smoothstep` analog) is reproducible
  deterministically in a CPU rasteriser to the bit. Phase task pins the AA math to a
  deterministic form (e.g. exact coverage integration or a fixed-point smoothstep) so the
  golden is stable across machines.

## 3. Design Overview

Three layered pieces:

1. **Geometry generation (fills B's cache).** Tessellate polygons, expand strokes,
   emit `Vertex{x,y,u,v,color}` quads for rects/images, and emit SDF quads for rounded
   shapes. This is the content-dependent work invariant 1 assigns to `present()`.
2. **Software rasteriser.** Rasterise the vertex buffer into an RGBA8 buffer:
   premultiplied-alpha over-blend in linear space, sRGB encode on store, Y-down. SDF
   shapes evaluate the distance field per pixel for perfect AA. One code path mirrors the
   "one pipeline, many shapes" GPU design so the golden faithfully predicts GPU output.
3. **Blit + golden harness.** UI-thread blit of the buffer to each platform surface;
   a headless golden test that renders fixed scenes and byte-compares.

**Where correctness risk concentrates:** the **determinism of AA and sRGB** (so the
golden is stable) and the **blit correctness** per platform. Determinism lands first
(it gates whether goldens are even possible); blit lands last per platform.

**This is the byte-identity plan of the set.** For the *software* backend, "byte-identical
rendered buffer vs stored golden" IS the acceptance gate — legitimately, because the
software path is provably deterministic. A golden mismatch is root-caused (diff the buffer,
localize the primitive), then fixed — never re-baselined without proving the golden wrong
per AGENTS.md's four-question rule. For GPU backends (E/F) byte-identity is explicitly the
wrong gate; C writes the tolerance comparator they will use.

**Rejected alternatives:**
- *MSAA / tessellated curves.* Rejected per design: analytic SDF gives perfect AA at any
  scale, one quad, no MSAA cost — and is deterministic on the CPU, which MSAA sampling is
  not.
- *Skip the software backend, golden the GPU directly.* Rejected (invariant 7): GPU output
  isn't byte-stable across drivers, so there'd be no deterministic oracle and no headless
  CI path.

## Compatibility / Format Impact

- **Changes:** software rasteriser + per-platform CPU-buffer blit; a golden-image test
  corpus (new `tests/` fixtures + a comparator). Rendering conventions (premultiplied
  alpha, sRGB encode, Y-down) become an observable contract that E/F must match within
  tolerance.
- **Unchanged:** scene model, resource table, `Mode`/gate semantics, non-canvas codegen.

## Phases

### Phase 1 — Deterministic geometry generation + software rasteriser core

- [ ] Implement geometry generation for Rect, Line (stroke expansion), Polygon
      (tessellation), Image (textured quad), RoundedRect (SDF quad) — feeding B's cache
      miss path with real `Vertex` ranges.
- [ ] Implement the software rasteriser: premultiplied-alpha linear-space over-blend,
      deterministic sRGB encode on store, Y-down top-left pixel coords, per-pixel SDF
      evaluation with **deterministic AA** (fixed-point/exact-coverage — pin the math).
- [ ] Tests: unit tests rasterising each primitive to a small buffer with hand-checked
      pixel values (corner AA, blend over a background, SDF circle at `radius=min(w,h)/2`).

Acceptance: each primitive rasterises to expected pixels deterministically on the test
machine; AA and sRGB encode are reproducible (same bytes on re-run). No GPU, no blit yet.
Commit: —

### Phase 2 — Golden-image harness + tolerance comparator

- [ ] Add a headless golden test that renders a fixed multi-primitive scene to the RGBA
      buffer and byte-compares to a stored reference; store references under
      `tests/golden/canvas/` (raw + PNG).
- [ ] Implement the **tolerance comparator** (per-channel epsilon / SSIM) as a separate
      entry point, documented as the GPU-backend comparator for E/F. Software goldens use
      the byte-exact path; the tolerance path is unused until E but is written and
      unit-tested here so invariant 5 is real, not aspirational.
- [ ] Tests: byte-exact golden for the fixed scene; a deliberately-perturbed buffer fails
      byte-exact but passes tolerance within threshold and fails beyond it.

Acceptance: the fixed-scene software golden passes byte-exact; the tolerance comparator
accepts/rejects at documented thresholds — both test-proven and headless.
Commit: —

### Phase 3 — Per-platform CPU-buffer blit (largest blast radius last)

- [ ] macOS: blit the RGBA buffer into the A-built layer-backed view on the main thread
      (reuse the `term_view.rs` marshaling precedent).
- [ ] Linux: blit into the GTK window (Cairo/`gdk` surface or the term_draw precedent),
      UI-thread only.
- [ ] Windows: blit via memDC `BitBlt` into the HWND client area on `WM_PAINT`
      (mirroring the term GDI path).
- [ ] Tests: headless render → blit round-trip on each platform asserts the blitted
      region matches the source buffer (readback where the headless path allows; else a
      lifecycle+no-crash assertion plus the byte-exact buffer golden from Phase 2).

Acceptance: a scene renders and blits to each platform surface headless without crash;
where readback is available the blitted pixels match the source buffer; the byte-exact
golden still passes. Full `cargo test` green.
Commit: —

## Validation Plan

- Tests: per-primitive rasteriser unit tests, the byte-exact golden, the tolerance
  comparator, and per-platform blit round-trips.
- Coverage check: rasteriser + comparator in the `--bin mfb` denominator; the headless
  blit subprocess is integration (uncaptured) — add in-process unit coverage for the
  buffer-fill and comparator logic so the changed code is measured.
- Runtime proof: a headless `--app` program presents a fixed scene; the golden test
  renders the same scene and byte-matches the stored reference.
- Doc sync: `src/docs/spec/app/` canvas rendering-conventions section (premultiplied
  alpha, sRGB encode formula, Y-down) so E/F have a spec to match within tolerance;
  `.ai/testing-gates.md` note on the canvas golden corpus (byte-exact software /
  tolerance GPU).
- Acceptance: full `cargo test`; canvas software golden byte-exact; non-canvas
  byte-identity corpus unchanged; fmt.

## Open Decisions

- **AA math form for CPU determinism** — recommended: exact analytic coverage for edges
  and a fixed-point SDF smoothstep, so the golden is machine-independent. (§Phase 1)
- **Golden storage format** — recommended: raw RGBA `.bin` as the byte-exact oracle
  (PNG codecs can vary) plus a PNG for human inspection. (§Phase 2)
- **Tolerance metric for GPU** — recommended: per-channel epsilon with a small max-diff
  budget as the primary gate, SSIM as a secondary sanity check. Finalize the thresholds
  when E produces real GPU output; document the placeholder now. (§Phase 2)

## Corrections

<Filled in during execution — especially the pinned AA/sRGB math.>

## Summary

C delivers the permanent oracle: a deterministic software rasteriser whose output is
byte-identical golden-checkable headless, plus the tolerance comparator E/F will use.
The risk is determinism (AA/sRGB must be bit-stable) and per-platform blit. With A–C
landed, canvas mode is a shippable, GPU-free, golden-tested product; D adds the graphics
thread and the concurrent resource protocol on top without changing the pixel oracle.
