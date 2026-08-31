# plan-98-C: Canvas software rasteriser + golden-image harness

Last updated: 2026-08-30
Effort: large (3h–1d)
Depends on: plan-98-B (scene model, arena, deep copy, frame skip, `Image` resource).
The per-item hashing and the geometry cache moved INTO this letter — see Phase 1.

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
| plan-98-B complete (scene arena, deep copy, frame skip, RES resources) | `ls planning/completed/plan-98-B-*` → hit | MET (archived after `48d5d544d`; its four phases landed as `d3cd3a0f6`, `118837f5a`, `47034488f`, `8c2ebb103`) |
| ~~Geometry cache miss reaches a generation hook~~ **N/A** — the cache moved into this letter's Phase 1 (B Correction 18), so there is no cross-letter hook to check | — | N/A |
| The scene region holds offset 24 open for the per-item hashes | `rg -n "reserved for the pointer to the per-item content hashes" src/codegen/error/constants/error_constants.rs` → hit | MET (B Phase 2 reserved it; it is a comment, not a constant, because an unused constant is dead code — C declares the constant when it fills the slot) |
| Working tree builds | `cargo build` → pass | MET (re-run: `Finished `dev` profile`) |

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

- [x] **Per-item content hashing** (moved from B Phase 3): hash each item's
      **fields** (Correction 1 — not its bytes) and publish the resulting
      `List OF Integer` into the scene region's reserved `hashes` slot — offset **24**
      in the canvas scene region, held open by B. `CANVAS_SCENE_HASHES_OFFSET` is
      declared here, where it is first used, via the internal-only
      `canvas::publishHashes` / `canvas::installedHashes` pair.
- [x] **Geometry cache** (moved from B Phase 3): the entry's five fields live as
      parallel lists (`__CANVAS_GEO_HASHES` / `_OFFSETS` / `_COUNTS` / `_LASTUSED`,
      with `bounds` inside the geometry record itself at slots 16-19); probe on the
      item hash, **confirm the hit by comparing the 22-float header exactly**
      (Correction 2), miss generates and inserts, LRU-evict by `lastUsedRev` at 256
      entries. `rt_canvas_rasteriser::cache_hit_skips_geometry_generation` shows the
      hit skipping generation: three frames report `generations = 3, 4, 4`.
- [x] Implement geometry generation for Rect, Line (stroke expansion), Polygon
      (**precomputed edge array**, Correction 3 — not a triangle tessellation),
      RoundedRect / Circle / Arc (SDF quad; Arc = angle-wedge-clipped stroked ring) —
      feeding the cache miss path with real geometry records. Image is `Picture`, which
      generates the `__CANVAS_GEO_NONE` kind until plan-98-G brings the sampler
      (Correction 4).
- [x] Implement the software rasteriser: linear-space over-blend through a literal
      256-entry sRGB table, deterministic sRGB encode on store, Y-down top-left pixel
      coords, per-pixel SDF evaluation with **exact-coverage AA** pinned to
      `clamp(0.5 - d, 0, 1)` — `+ - * / sqrt` only, no transcendental anywhere
      (Correction 5 pins the math and records why `smoothstep` was rejected).
- [x] Tests: `tests/rt_canvas_rasteriser.rs` — 8 tests rendering each primitive
      headless and checking hand-derived pixel values (rectangle span, circle AA edge
      `= 203`, rounded-rect cut corner, polygon interior, linear-space blend `= 188`
      where an sRGB-space blend would give `128`, an `Arc` sweeping `0..PI` present
      below its centre and **absent above it**), plus byte-reproducibility across two
      independent builds and the cache-hit test above. Four `--bin mfb` unit tests pin
      the sRGB table's length, endpoints, monotonicity and transfer function.

Acceptance: each primitive rasterises to expected pixels deterministically on the test
machine; AA and sRGB encode are reproducible (same bytes on re-run); and a re-`present`
that changes one item of many regenerates exactly one cache entry, the rest hitting.
No GPU, no blit yet.
Commit: `b33cbfea3`

### Phase 2 — Golden-image harness + tolerance comparator

- [x] Added `tests/rt_canvas_golden.rs`: renders the plan-98-api.md smiley headless
      and compares it exactly to `tests/golden/canvas/smiley.png`. Stored as **PNG
      only, compared as decoded pixels** (Correction 9 — the encoder-variance concern
      the plan raised is about file bytes, which nothing here compares). RED-checked:
      making `__canvas_coverage` truncate instead of round reds it with
      "613 of 576000 pixels differ (max channel delta 13); first at (433, 170)".
- [x] `tests/common/canvas_image.rs` provides `compare_exact` and
      `compare_within_tolerance`. The tolerance metric is a per-channel epsilon **and**
      a differing-pixel budget, both required (Correction 10 — either alone is the
      wrong shape; SSIM was rejected). `Tolerance::GPU_DEFAULT` is 2 steps / 2% of
      pixels, documented as E/F's starting point to be re-measured against real driver
      output.
- [x] Tests: the exact-match golden above, plus four comparator tests — identical
      frames pass both; a one-step perturbation on one pixel fails exact-match and
      passes tolerance; a 40-step delta fails the epsilon; a one-step shift over 10% of
      the frame fails the pixel budget while every individual difference is inside the
      epsilon.

Acceptance: MET. `cargo test --test rt_canvas_golden` — 5 passed, headless. The
golden's ability to catch a rendering change was RED-checked, not assumed.
Commit: `33e54904a`

### Phase 3 — Per-platform CPU-buffer blit (largest blast radius last)

- [x] macOS: the frame is wrapped in a `CGImage` and set as the canvas layer's
      `contents`, marshaled to the main thread as `mfbBlit:` through the same
      `performSelectorOnMainThread:waitUntilDone:YES` the mode reconcile uses.
      `CGBitmapContextCreate` + `CGBitmapContextCreateImage` rather than
      `CGImageCreate` (Correction 12).
- [x] Linux: the frame is copied (swizzled to Cairo's BGRX) into a block that
      carries its own dimensions, handed to the GTK main loop through `g_idle_add`,
      and painted by a `GtkDrawingAreaDrawFunc` via `cairo_image_surface_create_for_data`.
      Ownership passes with the pointer, so `ST_CANVAS_PIXELS` needs no lock
      (Correction 13).
- [x] Windows: `SetDIBitsToDevice` on `WM_PAINT` rather than a memDC `BitBlt`
      (Correction 14 — a canvas frame arrives complete, so a persistent DIB section
      would be an allocation, a `SelectObject` and a lifetime to hold pixels already
      in memory). The frame is posted as `WM_APP_BLIT` and the window owns it from
      there.
- [x] Tests: macOS is verified by **actual screen capture** — `test-macapp.sh`
      Case 3f renders two flat colour bars, captures the real window with
      `snap-macos.py`, and checks the pixels (Correction 15 — a no-crash assertion
      would pass for a blit that drew nothing, which is exactly what a broken one
      does). RED-checked: deleting the `setContents:` send reds it with
      `left=(236, 236, 236) right=(236, 236, 236)`, the empty window's grey. GTK and
      Windows get codegen-inspection tests for the properties that make the handoff
      correct — copy-before-handoff, single-owner free, paint-and-release — since
      neither can be executed on the build host.

Acceptance: MET, and strengthened (Correction 15): macOS is proven by a captured
screenshot of the real window, not by a no-crash assertion. `MFB_MACAPP_GUI=1
scripts/test-macapp.sh` — 18 ok, 0 fail. GTK and Windows cross-build and their
codegen-inspection tests pass (`cargo test --bin mfb canvas_reconcile_tests` — 41).
The exact-match golden and the rasteriser suite still pass (13).
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

**Correction 1 (Phase 1) — the per-item hash is over an item's *fields*, not its
bytes.** The plan said "the hash spans the copied bytes directly", which is cheaper
and was correct about the block being contiguous and pointer-free. It is still the
wrong key: a record's padding is not specified to be initialized, so two items with
identical content can differ in their padding bytes. A byte hash would then *miss* a
cache hit that is really there, silently turning plan-98-A invariant 2 off with no
visible symptom — the frame renders identically either way. `__canvas_hashItem`
therefore hashes the generated 22-float header, which is padding-independent and
stands for exactly the content that determines the drawing. The hashes are published
through the internal-only `canvas::publishHashes`, read back by
`canvas::installedHashes`, and land at `CANVAS_SCENE_HASHES_OFFSET = 24` as the plan
required.

**Correction 2 (Phase 1) — a hash probe alone would be a correctness bug; hits are
confirmed.** The plan said "probe on the item hash, hit reuses the vertex range". A
bare hash probe lets a collision reuse another item's geometry, which draws the wrong
picture — a rare wrong answer, which is worse than a common slow one, and one no
golden would reliably catch. `__canvas_geometryFor` probes by hash and then confirms
with an exact 22-float header comparison before reusing the tail. The confirmation is
22 float compares against a tail that can be thousands, so it costs nothing the cache
was buying.

**Correction 3 (Phase 1) — polygon geometry is a precomputed edge array, not a
triangle tessellation.** The plan asked for tessellation into `Vertex{x,y,u,v,color}`
triangles *and*, three paragraphs later, for analytic-SDF antialiasing with the
determinism that makes an exact-match oracle possible ("Rejected alternatives: MSAA /
tessellated curves"). Those two bullets contradict each other: a triangle list carries
no distance field, so AA over it needs coverage sampling — exactly the MSAA the same
section rejects — and a scanline filler and an SDF filler disagree about edge pixels,
which is the one disagreement an oracle cannot have. The SDF requirement wins, because
AA determinism is the phase's acceptance criterion. Geometry is therefore a flat float
buffer: a 22-slot header (kind, SDF params, both colours, bounds) plus, for a polygon,
five floats per edge (`x0, y0, dx, dy, invLenSq`). That is still real generated
geometry that the cache genuinely pays for — it turns the per-pixel distance query
from "recompute every edge vector and its reciprocal length" into five reads — and it
is still what plan-98-E/F upload, since an SDF quad's per-instance parameter block is
precisely this header.

**Correction 4 (Phase 1) — `Image` geometry moves to plan-98-G with the rest of the
sampler.** The phase listed "Image (textured quad)" among the kinds to generate. There
is nothing to sample: `canvas::loadImage` is plan-98-G's (plan-98-B Phase 4 declared
the `Image` resource but no loader), so a textured quad would reference a texture no
program can produce. `Picture` generates the `__CANVAS_GEO_NONE` kind and draws
nothing, exactly as `Text` does, and both still occupy a geometry record so item
indices, hashes and offsets stay parallel. This is deferral only in the sense the plan
already made it one — G's scope names `canvas::loadImage` explicitly.

**Correction 5 (Phase 1) — the AA and sRGB math, pinned.** The plan left this as an
Open Decision ("exact analytic coverage ... or a fixed-point SDF smoothstep").
Resolved: **coverage is `clamp(0.5 - d, 0, 1)`**, the exact fraction of a pixel inside
a locally straight edge, evaluated with `+ - * /` and `sqrt` only — all exactly
specified by IEEE-754, so the result is bit-identical on every target. `smoothstep`
and `fwidth` are rejected: their result depends on a derivative *estimate*, and the
oracle has to be the exact answer the GPU is compared against, not another
approximation. **No transcendental appears anywhere in the rasteriser**: sRGB uses a
literal 256-entry table (a `pow` on the path would make the oracle platform-
dependent), the arc's sweep test uses two cross products rather than `atan2`, and
`sin`/`cos` for the sweep endpoints are 9th-degree Taylor series evaluated once per
arc. Blending is `dst + (src - dst) * alpha / 255` on the **linear** values with
round-to-nearest (`+ 127` before the divide), so `blendChannel(d, s, 255) == s` holds
exactly.

**Correction 6 (Phase 1) — a defect found and fixed en route: the sRGB table was
truncated and wrong.** The pasted 256-entry literal held 252 entries and diverged from
the transfer function from index 121 onward. The effect was invisible to every
structural check and to the frame skip: `collections::getOr` fell back to its `0`
default for the high channels, so every *antialiased* pixel blended towards black
while every fully-covered pixel stayed correct. It was found by dumping a frame and
finding only two distinct red values in the whole image where AA should have produced
a gradient. Fixed by regenerating the table, and pinned by four `--bin mfb` unit tests
(length, endpoints, monotonicity, transfer function) which were RED-checked against the
truncated literal — the length and endpoint tests fail on it.

**Correction 7 (Phase 1) — `canvas::presentLayers` rendered nothing; added
`canvas::installedLayers`.** A scene is published in one of two shapes and the
renderer read only the flat one, so `presentLayers` published correctly and then drew
an empty frame. The internal-only `canvas::installedLayers` is the layered twin of
`canvas::installedItems`, and `__canvas_renderScene` walks both shapes in draw order
with one index into the published hash list.

**Correction 12 (Phase 3) — macOS builds the `CGImage` through a bitmap context.**
`CGImageCreate` is the direct constructor and takes eleven arguments, three of which
land on the stack under Apple's *packed* AArch64 stack-argument rules — an 8-byte
pointer, a 1-byte `bool`, then a 4-byte enum, at offsets 0, 8 and 12. That layout is
easy to get subtly wrong and its failure mode is a wrong image rather than a crash.
`CGBitmapContextCreate` (seven arguments, all in registers) followed by
`CGBitmapContextCreateImage` (one) avoids the packed-stack question entirely, and
`CreateImage` is documented to *snapshot* the pixels — which is what lets the
collection's payload pointer be passed directly instead of copied first.

**Correction 13 (Phase 3) — the frame carries its own dimensions, and that is what
makes the handoff lock-free.** Both GTK and Windows publish a frame asynchronously
(`g_idle_add` / `PostMessageW`), so the worker and the UI thread would race on any
state they share. Each blit therefore allocates one block holding width at +0, height
at +8 and pixels from +16, and hands over the *pointer*: the worker builds a block
nobody else can see, and every read and write of the published slot happens on the UI
thread. Publishing the dimensions separately would let a repaint pair new pixels with
the previous frame's height — reading past the end of the block, which is a crash
rather than a glitch. The macOS path needs none of this because
`waitUntilDone:YES` makes its handoff synchronous.

**Correction 14 (Phase 3) — Windows uses `SetDIBitsToDevice`, not a memDC `BitBlt`.**
The plan said to mirror the term GDI path. That path exists because the term surface
is a *persistent* off-screen bitmap the app draws into incrementally; a canvas frame
arrives complete and is drawn once. Reproducing the memDC would add an allocation, a
`SelectObject` and a lifetime to manage, all to hold pixels that are already in
memory. `SetDIBitsToDevice` draws them where they are. The `BITMAPINFOHEADER` uses a
**negative** height: the rasteriser's row 0 is the top, and a DIB's default bottom-up
row order would render every frame upside down.

**Correction 15 (Phase 3) — the macOS acceptance was strengthened to a real screen
capture.** The plan allowed "a lifecycle+no-crash assertion" where readback is
unavailable. That is not a test of a blit: a blit that draws *nothing* also does not
crash, and is exactly the failure being guarded against. `test-macapp.sh` Case 3f
therefore renders two flat colour bars, captures the actual window with
`snap-macos.py`, and checks the pixels — flat regions rather than an image diff,
because a capture is scaled by the display's backing factor and composited with
window chrome that the software golden knows nothing about. RED-checked by deleting
the `setContents:` send, which reds it with the empty window's grey.

**Correction 16 (Phase 3) — a defect found and fixed en route: the rasteriser was
290x slower than it needed to be.** The screenshot in Correction 15 came back blank,
because one 900x640 frame took **53 seconds** and the capture waits one. Root cause:
`__canvas_blendPixel` took and returned the surface, and `collections::set` only
mutates in place for `local = set(local, …)` on a local of the function doing the
write (`try_inplace_set_assign`). Through a helper's parameter and return it copies
the whole 2.3 MB list per channel. Measured, 20,000 writes into a 200,000-byte list:
a local in the same function **5 ms**, threaded through a helper **1179 ms**, through
a module-level `MUT` **1161 ms** — so a global does not help either. Reads are free
(read-modify-write also 5 ms). Fixed by collapsing the four rasterisation loops into
one that owns the surface local and does the `set` calls inline; the loops only ever
differed by their distance function. **53.49 s → 0.38 s**, frame byte-identical, and
the Phase 1 suite went 94 s → 3.7 s.

**Correction 9 (Phase 2) — references are PNG only, not "raw + PNG".** The plan
wanted a raw `.bin` as the exact-match oracle "because PNG codecs can vary", plus a
PNG for human inspection. Encoders do vary — in the *file bytes* they emit for given
pixels — but nothing in the harness compares file bytes: `Frame::load_png` decodes
and `compare_exact` compares the decoded pixel array. A PNG decodes to exactly one
pixel array, so this is precisely as exact as a raw blob, at 21 KB instead of 2.3 MB
for one 900x640 frame, and viewable without a converter. Storing both would have been
the same data twice.

**Correction 10 (Phase 2) — the tolerance metric is epsilon AND a pixel budget; SSIM
rejected.** The plan offered "per-channel epsilon / SSIM". Neither alone is the right
shape: an epsilon alone accepts a frame where *every* pixel is slightly wrong, which
is a systematic error (a wrong gamma, a half-pixel offset) rather than the sampling
noise the tolerance exists to permit; a differing-pixel budget alone accepts a few
catastrophically wrong pixels. `compare_within_tolerance` requires both. SSIM is not
implemented: it is a perceptual-similarity measure, and the question a GPU backend
must answer is "did this diverge from the oracle numerically", for which a structural
similarity score is both harder to threshold defensibly and blind to a uniform shift.
The plan listed it as a "secondary sanity check", and a check nothing gates on is
surface without a consumer.

**Correction 11 (Phase 2) — `canvas::getSize()` with no argument did not exist.** The
smiley in `plan-98-api.md` — which this phase's golden fixture is — opens with
`LET canvasSize AS Size = canvas::getSize()`, and the doc says "getSize() with no arg
returns the surface size". Only the `getSize(image)` overload was implemented, so the
documented example failed to compile with `TYPE_UNKNOWN_VALUE`. Added the no-arg
overload as a second `Implementation` delegating to `__canvas_surfaceSize`, so the
renderer and the program cannot disagree about how big the surface is and plan-98-D's
resize has one definition to replace.

**Correction 8 (Phase 1) — `MFB_CANVAS_STATS` added as a test affordance.** The
cache's whole claim is that re-presenting an unchanged item generates nothing, and
that claim is invisible in the pixels: an identical frame is what you get whether the
geometry was reused or rebuilt. `__canvas_presentSurface` appends one counter line per
rendered frame when the variable is set — appends, not overwrites, because the
interesting quantity is the *delta* between frames. Same shape as the `MFB_CANVAS_DUMP`
readback this phase also uses.

## Summary

C delivers the permanent oracle: a deterministic software rasteriser whose output is
exact-match golden-checkable headless, plus the tolerance comparator E/F will use.
The risk is determinism (AA/sRGB must be bit-stable) and per-platform blit. With A–C
landed, canvas mode is a shippable, GPU-free, golden-tested product; D adds the graphics
thread and the concurrent resource protocol on top without changing the pixel oracle.
