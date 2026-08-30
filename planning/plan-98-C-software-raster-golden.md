# plan-98-C: Canvas software rasteriser + golden-image harness

Last updated: 2026-08-30
Effort: large (3h–1d)
Depends on: plan-98-B (scene model, arena, hashing, geometry cache stub)

This sub-plan makes the geometry cache's generation real for a software backend and
adds the golden-image test harness. After it lands, `canvas::present(items)` in
`Mode.Canvas` renders the scene to an in-memory RGBA buffer that is blitted to the
platform surface built in A, and a golden-image test compares that buffer to a stored
reference **exactly** (the software backend is deterministic). This is the
permanent correctness oracle and the headless CI path — canvas mode can now render a
real picture with no GPU.

This is **build step 3** of the A–G sequence, and it establishes cross-cutting invariant 5
(GPU goldens tolerance-based; software reference image exact-match) for E/F to consume.

> **Terminology (A's invariant 8).** These reference images are **new artifacts this
> plan creates** — an oracle for a new rasteriser. They are deliberately exact-match
> because the software path is deterministic. They are *not* instances of the repo's
> `tests/byte-identity/` codegen drift gate, which invariant 8 puts out of scope for
> all of plan-98. Do not run `artifact-gate.sh` for this letter.

References:

- **plan-98-A** — invariants 5 and 7 (software backend first-class, exact-match
  oracle, tolerance policy for GPU) and invariant 8 (testing policy). plan-98-A's
  "Cross-cutting invariants" section is this feature's top-level design; there is no
  separate design document.
- `planning/plan-98-api.md` — the 8 `DrawItem` variants this rasteriser must draw,
  including the `Arc` angle convention (radians, clockwise from +X under Y-down).
- `.ai/testing-gates.md` — golden-harness conventions (the byte-identity codegen gate
  it also documents is out of scope here — A's invariant 8).
- The existing headless blit paths: `MFB_MACAPP_HEADLESS`, GTK-headless,
  `MFB_WINAPP_HEADLESS`; `scripts/snap-term.py` / `scripts/snap-macos.py`.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-98-B complete (scene arena, deep copy, frame skip, RES resources) | `ls planning/completed/plan-98-B-*` → hit | NOT MET |
| ~~Geometry cache miss reaches a generation hook~~ **N/A** — the cache moved into this letter's Phase 1 (B Correction 18), so there is no cross-letter hook to check | — | N/A |
| The scene region holds offset 24 open for the per-item hashes | `rg -n "reserved for the pointer to the per-item content hashes" src/codegen/error/constants/error_constants.rs` → hit | MET (B Phase 2 reserved it; it is a comment, not a constant, because an unused constant is dead code — C declares the constant when it fills the slot) |
| Working tree builds | `cargo build` → pass | UNVERIFIED (run before starting) |

> Per A's invariant 8: no "full suite green at HEAD" row, no byte-identity obligation;
> the full suite runs once, at the end of the plan (G).

## 1. Goal

- A **software rasteriser** turns cached geometry into pixels: fill a runtime-owned
  RGBA8 buffer for the current surface size using **premultiplied alpha, linear
  blending, Y-down top-left pixel coordinates** — the rendering conventions this
  plan fixes, and which E/F must match within tolerance. Rectangles,
  lines, polygons render via tessellation/stroke expansion (now real, feeding the B
  cache); RoundedRect, Circle, and Arc render via analytic SDF (Circle is the `radius`
  SDF; Arc is the same SDF wedge-clipped to `startAngle..endAngle`, stroked). Text is
  still stubbed (G).
- The rendered buffer is blitted to the A-built surface on the UI thread (reusing the
  existing headless/real blit paths per platform).
- A **golden-image harness** renders a fixed scene headless to the buffer and compares
  it **exactly** to a stored PNG/raw reference; a mismatch is a bug-hunt trigger,
  never a re-baseline.
- **Invariant 5 is codified:** the software reference image is exact-match; GPU backends
  (E/F) will diff against it with a documented tolerance.

### Non-goals (explicit constraints)

- **No graphics thread, no scene ring, no GPU.** Rendering is synchronous on the
  present/UI path (the ring is D; GPU is E/F). This is deliberate: get the pixel oracle
  right before threading and GPU land.
- **No text rendering** (G). Text items render nothing (or a debug box) and are excluded
  from goldens until G.
- **No sRGB *hardware* linearisation** — the software path does sRGB encode/decode in
  code to match what the GPU sRGB surface will do, so the software golden is a faithful
  reference. Document the exact encode so E/F can match within tolerance.
- No change to the scene model, RES resource record, or non-canvas codegen.

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
| Primitive kinds needing a software rasteriser path | 7 (Rect, Line, Polygon, RoundedRect, Circle, Arc — SDF; Image blit) | 8-variant set minus Text |
| Platform blit entry points to add | 3 | A's three surfaces |
| Existing golden-image diffs in the Rust suite | 0 | research §7 (`rg -n "golden" tests/ \| rg -i png`) — run to confirm |

### Verified properties

- **A deterministic software rasteriser can produce exact-match goldens.** VERIFIED
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

**This is the exact-match-oracle plan of the set.** For the *software* backend, "exact
rendered buffer vs stored golden" IS the acceptance gate — legitimately, because the
software path is provably deterministic. A golden mismatch is root-caused (diff the buffer,
localize the primitive), then fixed — never re-baselined without proving the golden wrong
per AGENTS.md's four-question rule. For GPU backends (E/F) an exact match is explicitly the
wrong gate; C writes the tolerance comparator they will use.

**Rejected alternatives:**
- *MSAA / tessellated curves.* Rejected: analytic SDF gives perfect AA at any
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
- **Unchanged:** scene model, RES resource record, `Mode`/gate semantics, non-canvas codegen.

## Phases

### Phase 1 — Deterministic geometry generation + software rasteriser core

> **Two tasks moved here from plan-98-B Phase 3** (B's Correction 18): the per-item
> content hashing and the geometry cache. B had them landing over a *stub* geometry
> generator, i.e. a cache whose every entry is a zero-length vertex range — machinery
> with no content, whose eviction "under arena pressure" cannot be exercised because
> the entries occupy nothing. AGENTS.md forbids shipping that. They land here, in the
> phase that first produces geometry to cache, so the cache is built once against real
> vertex data rather than built empty and re-shaped when the data arrives. B kept the
> whole-scene frame skip, which is real without them.

- [ ] **Per-item content hashing** (moved from B Phase 3): hash each item's bytes
      within the published scene block and store into the scene region's reserved
      `hashes` slot — offset **24** in the canvas scene region, held open by B. Declare
      `CANVAS_SCENE_HASHES_OFFSET` here, where it is first used. The block
      is contiguous and pointer-free (B Correction 13), so the hash spans the copied
      bytes directly.
- [ ] **Geometry cache** (moved from B Phase 3):
      `GeoCacheEntry{hash, vtxOffset, vtxCount, bounds, lastUsedRev}`; probe on the
      item hash, hit reuses the vertex range, miss generates and inserts, LRU-evict by
      `lastUsedRev` under pressure. This is what makes plan-98-A invariant 2 true —
      re-presenting an unchanged item must be free — so its test must show a **hit
      skipping generation**, which is only observable now that generation does work.
- [ ] Implement geometry generation for Rect, Line (stroke expansion), Polygon
      (tessellation), Image (textured quad), RoundedRect / Circle / Arc (SDF quad; Arc =
      angle-wedge-clipped stroked ring) — feeding B's cache miss path with real `Vertex`
      ranges.
- [ ] Implement the software rasteriser: premultiplied-alpha linear-space over-blend,
      deterministic sRGB encode on store, Y-down top-left pixel coords, per-pixel SDF
      evaluation with **deterministic AA** (fixed-point/exact-coverage — pin the math).
- [ ] Tests: unit tests rasterising each primitive to a small buffer with hand-checked
      pixel values (corner AA, blend over a background, SDF Circle, a stroked Arc sweeping
      `0..PI`). The smiley scene from plan-98-api.md is a good golden fixture.

Acceptance: each primitive rasterises to expected pixels deterministically on the test
machine; AA and sRGB encode are reproducible (same bytes on re-run); and a re-`present`
that changes one item of many regenerates exactly one cache entry, the rest hitting.
No GPU, no blit yet.
Commit: —

### Phase 2 — Golden-image harness + tolerance comparator

- [ ] Add a headless golden test that renders a fixed multi-primitive scene to the RGBA
      buffer and byte-compares to a stored reference; store references under
      `tests/golden/canvas/` (raw + PNG).
- [ ] Implement the **tolerance comparator** (per-channel epsilon / SSIM) as a separate
      entry point, documented as the GPU-backend comparator for E/F. Software goldens use
      the exact-match path; the tolerance path is unused until E but is written and
      unit-tested here so invariant 5 is real, not aspirational.
- [ ] Tests: exact-match golden for the fixed scene; a deliberately-perturbed buffer fails
      exact-match but passes tolerance within threshold and fails beyond it.

Acceptance: the fixed-scene software golden passes exact-match; the tolerance comparator
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
      lifecycle+no-crash assertion plus the exact-match buffer golden from Phase 2).

Acceptance: a scene renders and blits to each platform surface headless without crash;
where readback is available the blitted pixels match the source buffer; the exact-match
golden still passes. Run only the new rasteriser/golden tests plus A's headless
lifecycle test (the blit path is the only existing target this reaches).
Commit: —

## Validation Plan

- Tests: per-primitive rasteriser unit tests, the exact-match golden, the tolerance
  comparator, and per-platform blit round-trips.
- Coverage check: rasteriser + comparator in the `--bin mfb` denominator; the headless
  blit subprocess is integration (uncaptured) — add in-process unit coverage for the
  buffer-fill and comparator logic so the changed code is measured.
- Runtime proof: a headless `--app` program presents a fixed scene; the golden test
  renders the same scene and byte-matches the stored reference.
- Doc sync: `src/docs/spec/app/` canvas rendering-conventions section (premultiplied
  alpha, sRGB encode formula, Y-down) so E/F have a spec to match within tolerance;
  `.ai/testing-gates.md` note on the canvas golden corpus (exact-match software /
  tolerance GPU).
- Acceptance: the per-phase targeted tests above; canvas software golden exact-match.
  **No full-suite run and no codegen byte-identity check in this letter** (A's
  invariant 8); fmt.

## Open Decisions

- **AA math form for CPU determinism** — recommended: exact analytic coverage for edges
  and a fixed-point SDF smoothstep, so the golden is machine-independent. (§Phase 1)
- **Golden storage format** — recommended: raw RGBA `.bin` as the exact-match oracle
  (PNG codecs can vary) plus a PNG for human inspection. (§Phase 2)
- **Tolerance metric for GPU** — recommended: per-channel epsilon with a small max-diff
  budget as the primary gate, SSIM as a secondary sanity check. Finalize the thresholds
  when E produces real GPU output; document the placeholder now. (§Phase 2)

## Corrections

**2026-08-30 — pre-execution revision (no code written yet).** See plan-98-A's
Corrections for the full account. Applied here: A's invariant 8 (this is new work, so
no codegen byte-identity gate and no full-suite run until the end of the plan); the
per-phase acceptance lines now name targeted tests; and the software rasteriser's
reference images are described as **exact-match** rather than "byte-identity goldens",
so this plan's own new oracle is not confused with the repo's `tests/byte-identity/`
codegen drift gate. The oracle itself is unchanged — invariant 5 still stands. This
letter cited no paths that moved in the 2026-08-16/17 restructurings, so no remap was
needed.

<Further corrections filled in during execution — especially the pinned AA/sRGB math.>

## Summary

C delivers the permanent oracle: a deterministic software rasteriser whose output is
exact-match golden-checkable headless, plus the tolerance comparator E/F will use.
The risk is determinism (AA/sRGB must be bit-stable) and per-platform blit. With A–C
landed, canvas mode is a shippable, GPU-free, golden-tested product; D adds the graphics
thread and the concurrent resource protocol on top without changing the pixel oracle.
