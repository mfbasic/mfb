# plan-116-C: `Paint.transform` becomes real

Last updated: 2026-09-01
Effort: x-large (1d–3d) — revised 2026-09-01; see Corrections C1
Depends on: plan-116-B

The third of the three declared-but-unread `Paint` fields. `canvas::Transform` is a
2×3 affine documented at `mod.rs:343` with a carefully-chosen rule — **the all-zero
value means the identity**, not the degenerate collapse-to-origin matrix — and
`mfb spec` §"Paint is a value, not ambient state" calls that out as the one
consequence worth stating explicitly. None of it is read: the same three-hit grep
from plan-116-B covers `.transform` too.

Behavioral outcome: an item carrying a non-identity `Paint.transform` renders with its
geometry transformed by that affine — a scaled, rotated or sheared shape, antialiased
on the transformed edges, not on the original ones — identically on the software,
Metal and Vulkan paths. An item carrying the all-zero (identity) transform renders
exactly as it does today, byte for byte.

References:

- `src/codegen/builtins/canvas/mod.rs:343-378` — the `Transform` record and the
  all-zero-is-identity rule.
- `src/docs/spec/app/06_canvas.md` §"Paint is a value, not ambient state".
- `src/codegen/builtins/canvas/helper_paint_defaults.rs:25` — `__canvas_noTransform`,
  which returns the all-zero matrix and is the *only* existing code that touches
  `Transform` at all.
- `src/codegen/builtins/canvas/helper_draw.rs:61` — `__canvas_geoDistance`, the
  dispatch every distance field goes through.
- plan-116-B §4.1 — the header and item-block layout this letter extends again.

## Prerequisites

See plan-116-A §Prerequisites for the three environment gates.

| Must be true | Command | Status |
|---|---|---|
| plan-116-B complete and archived | `ls planning/completed/plan-116-B-*` → one match | **MET** (2026-09-01: exactly one match, `planning/completed/plan-116-B-canvas-blend-and-clip.md`, archived by `467b32a4f` and merged to main. Every B phase acceptance measured — 90 test binaries, artifact-gate 1823 goldens 0 diffs, test-accept 1346, man-census 0 unclassified, man-run-examples canvas 20/20, Vulkan 12/12 on box 2228.) |

If plan-116-B is not complete, this letter cannot start, full stop. B establishes
the widened header and the widened item block; C adds six more words to both, and
doing that against the pre-B layout would mean laying out the same two structures
twice.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command before you continue and again before you stop.

## 1. Goal

- A non-identity `Paint.transform` transforms the item's geometry on all three paths,
  with antialiasing evaluated in transformed space so a rotated edge is as smooth as
  an axis-aligned one.
- A non-identity transform on a `Text` item transforms its glyphs too — by
  inverse-transformed sampling of the glyph coverage bitmaps (§4.5) — so no
  documented `Paint` field is silently dead for any variant that renders.
- The all-zero `Transform` is the identity and produces today's exact bytes.
- The bounds an item is rasterised over are the **transformed** bounds, so a rotated
  shape is not clipped to its untransformed box.

### Non-goals (explicit constraints)

- **No new `canvas::` surface.** `Transform` exists; this letter only makes it work.
- **`Paint.clip` is NOT transformed.** It stays axis-aligned in surface pixels —
  the decision recorded in plan-116-B §Open Decisions, and `Bounds` cannot express
  anything else.
- **No non-affine transforms.** A 2×3 matrix is what the type is.
- **`Picture` is out of scope** — it has no renderer at all today (bug-484:
  `__canvas_headerFor` gives it an empty `NONE` header and no draw path exists), so
  there is nothing to transform. Bug-484's fix must implement `Paint.transform` for
  pictures as part of its own design, against the semantics this letter pins.
- **Stroke width is transformed with the geometry**, not held constant in surface
  pixels — see §4.3, and note this is a *decision*, recorded because both readings are
  defensible.
- **The identity path must not regress by one byte.**

## 2. Current State

### How geometry reaches a pixel today

Every shape is a **signed distance field evaluated at the pixel centre in surface
coordinates**. `__canvas_drawGeometry` (`helper_items.rs:83`) walks
`px = x + 0.5, py = y + 0.5` over the header's bounds and calls `__canvas_geoDistance`
(`helper_draw.rs:61`), which dispatches on `kind` to `__canvas_rectDistance`,
`length(p - c) - r`, `__canvas_segmentDistance`, the arc sweep, or
`__canvas_edgeDistance`. Both GPU fragment shaders have the identical dispatch
(`mfb_canvas.frag:geoDistance`, `metal.rs:geoDistance`) evaluated at
`gl_FragCoord.xy` / `[[position]]`.

`06_canvas.md` §"Rendering conventions" pins the coverage rule to
`clamp(0.5 - d, 0, 1)` on that signed distance, and pins it *because* it makes the
software path reproducible with only `+ - * /` and `sqrt`.

### Why that shape makes this letter tractable

An SDF is transformed by transforming the **query point**, not the shape: to draw
`T(shape)`, evaluate `shape` at `T⁻¹(p)`. So no distance function changes. What
changes is one inverse-transform applied to `(px, py)` before the dispatch, plus a
correction to the returned distance (§4.2).

This is the single most important property in the letter and it is why the design is
five words of plumbing plus one multiply, rather than a rewrite of every primitive.

### Measured populations

| What | Count | Command |
|---|---|---|
| Existing `Transform` readers in canvas | 1 (`__canvas_noTransform`, which only *constructs* the zero) | `grep -rn "Transform" src/codegen/builtins/canvas/` |
| Distance-field kinds needing no change | all of them | `helper_draw.rs:61-99`, `mfb_canvas.frag:geoDistance` |
| Header slots after plan-116-B | 27 | plan-116-B §4.1 |
| `ITEM_BLOCK_SIZE` after plan-116-B | 128 | plan-116-B §4.1 |

### Verified properties

- **`__canvas_noTransform` returns all zeros**, not the literal identity matrix — read
  `helper_paint_defaults.rs:25-27`. Its doc comment states the reason: writing the
  literal identity would make an explicitly-zero transform and a defaulted one behave
  differently under a later `WITH`. So **every transform read must map all-zero to
  identity before use**, and that mapping must live in exactly one place or the two
  will drift.
- **No distance function needs modification** — verified by reading all five arms of
  `__canvas_geoDistance` and both shaders' `geoDistance`: each takes `p` and shape
  parameters and returns a distance. None reads a global or a pixel index.
- **Two variants are not SDFs and need their own treatment.** A `Text` item is a
  per-glyph coverage-bitmap blit (`helper_items.rs`, the `__CANVAS_GEO_TEXT` arm;
  `mfb_canvas.frag:160-165`), so "transform the query point" must become "transform
  the sample point into the bitmap" — §4.5. A `Picture` has no renderer at all
  (bug-484) and is scoped out above.
- **UNVERIFIED: whether the distance correction (§4.2) is exact enough for the
  oracle.** For a non-uniform scale or a shear, `|T⁻¹|` is direction-dependent, so a
  single scalar correction is an approximation. Phase 1 measures the worst-case error
  on a sheared rectangle before anything is built on it. **This is the letter's real
  uncertainty and it is scheduled first.**

## 3. Design Overview

Four pieces:

1. **Carry the inverse.** The header and item block carry `T⁻¹` (six floats),
   computed once on the CPU when the geometry is built — never per pixel, never on
   the GPU.
2. **Transform the query point.** One `p' = T⁻¹ · p` before the existing dispatch, in
   all three renderers.
3. **Correct the distance.** `d_surface ≈ d_shape / ‖T⁻¹‖`, so antialiasing is
   correct in surface pixels. §4.2.
4. **Transform the bounds.** The four corners of the item's untransformed bounds map
   through `T`, and the new bounds are their axis-aligned hull.

**Where the design uncertainty concentrates:** §4.2's distance correction under shear
and non-uniform scale. Scheduled as Phase 1, as a measurement, before any renderer
changes.

**Where the correctness risk concentrates:** the all-zero-is-identity mapping. Get it
wrong in one of the three renderers and that renderer collapses every transformed item
to the origin — or, worse, collapses *untransformed* items, which is every existing
scene. Phase 2 puts that mapping in exactly one CPU-side function so the GPUs never see
a zero matrix at all.

**Byte-identity is NOT this letter's gate.** Behaviour legitimately changes for
transformed items. **Expected NOT to diff:** every existing golden, including
`smiley.png` and `blendmodes.png`, because every existing scene uses the identity. A
diff there is a regression in the identity arm — root-cause it, never re-baseline.
**Expected to diff:** `.ncodesum` on every canvas-emitting target, and the two `.spv`
blobs.

### Rejected alternatives

- **Bake the transform into the shape parameters on the CPU** (rect → polygon,
  circle → ellipse, …). Rejected: it is expressible for some kinds and not others — a
  sheared `RoundedRect` is not a rounded rect, and a transformed `Arc` is not a
  circular arc — so it would need a per-kind fallback that silently changes which
  primitive a program asked for. Transforming the query point is uniform across every
  kind, present and future.
- **Send `T` and invert it in the shader.** Rejected: a 2×2 inverse per pixel, and a
  singular matrix would produce NaNs on the GPU that the CPU path would handle
  differently. Inverting once on the CPU also gives one place to define what a
  singular transform means (§4.4).
- **Evaluate coverage with `fwidth`/`smoothstep` in transformed space.** Rejected
  outright by `06_canvas.md` §"Rendering conventions": a derivative-based coverage rule
  is not reproducible, and reproducibility is what makes the software path an oracle.

## 4. Detailed Design

### 4.1 Carrying the inverse

**Header** grows 27 → **33**. Slots 27–32 hold `T⁻¹` as `ia, ib, ic, id, itx, ity`,
applied as `x' = ia*x + ic*y + itx`, `y' = ib*x + id*y + ity` — the same convention
`mod.rs:343` documents for `T` itself.

One new helper, `__canvas_invertTransform(t AS Transform) AS List OF Float`, is the
**single** place that maps all-zero → identity and inverts. It returns the identity for
an all-zero input and for a singular input (§4.4). Every path — software header build,
Metal emitter, Vulkan emitter — reads slots 27–32 and never touches the `Transform`
record itself.

Slot 33 holds a **`hasTransform` flag** (0 or 1) so the per-pixel gate is one compare
rather than six. Header becomes **34** slots.

**Item block** grows 128 → **160**: two new `ivec4`s carrying the six inverse terms
**as raw IEEE-754 float32 bits** (the shaders decode with `intBitsToFloat` /
`as_type<float>`), plus the flag and one spare. Not 16.16: a transform that scales
an item up by 100× has inverse terms near `0.01`, which 16.16 holds to only ~4
significant digits — about ¾ of a pixel of positional error at the far edge of such an
item.

**The last sentence of this paragraph used to read "The header slots are already
floats, so the CPU side needs no conversion either." That is false, and it is the one
thing that made float32 look free — see Correction C3.** A `Float` is an IEEE *double*,
and the assemblers this compiler emits through have no double→single convert and no
32-bit float store (`emit_store_f32_from_integer`'s doc in `vulkan.rs` records the same
gap: it exists precisely because of it, and says outright that it is *not* a general
double-to-float).

So the narrowing is done **once, in MFBASIC**, by `__canvas_float32Bits`, and the header
carries the six terms as whole-number bit patterns that both emitters copy straight
through. One implementation an ordinary test can check against known IEEE-754 patterns,
rather than two hand-rolled ones in generated machine code where the only symptom of an
error is a wrong picture. It is arithmetic rather than bit-twiddling because `bits::`
takes `Integer` and never `Float`, so there is no reinterpret to borrow; the three
fields are assembled by addition, which is exact because they do not overlap.

### 4.2 The distance correction

Evaluating the shape at `T⁻¹(p)` gives a distance in *shape* space. Coverage must be
computed in *surface* space, or a scaled-up shape gets a scaled-up (blurry) antialiased
edge and a scaled-down one gets a hard, aliased edge.

The correction is `d_surface = d_shape / s`, where `s` is the local scale factor of
`T⁻¹` — the norm of the gradient of the transformed distance field. For an affine `T⁻¹`
with matrix `M`, the exact factor is direction-dependent; the tractable choices are:

- **`s = sqrt(|det M|)`** — exact for any similarity (uniform scale + rotation +
  translation), and an approximation otherwise.
- **`s = ‖∇d‖`, the norm of the composed field's own gradient** — exact for any
  affine, and obtainable *without* the shape returning a gradient: take it by explicit
  central differences of the already-composed distance field.

**Phase 1 measured both, and `sqrt(|det M|)` lost decisively. The design is
`d / ‖∇d‖`.** Worst-case coverage error against a 32×32 supersampled ground truth on a
half-plane, in 1/255 steps:

| | `sqrt(\|det M\|)` | `d / ‖∇d‖` |
|---|---|---|
| identity (the control) | 3.19 | 3.19 |
| 2:1 non-uniform scale | **37.34** | 3.19 |
| 30° shear | **18.18** | 9.71 |

Two things make that table conclusive rather than merely favourable.

**3.19 is the measurement floor**, not a result: a 32×32 grid quantises a straight
edge's area at 1/32 per axis, and the identity — which every method gets exactly right
— measures 3.19 too. So `d / ‖∇d‖` is *exact* for the non-uniform scale.

**The shear's residual 9.71 is not this letter's error at all.** An **untransformed**
30° edge measures 9.71 by the same harness, and an untransformed 45° edge 13.69. That
is the inherent error of the `clamp(0.5 - d, 0, 1)` coverage model on an edge that is
not axis-aligned — the model `06_canvas.md` §"Rendering conventions" *specifies*, and
the one every rotated shape in the renderer has always been drawn with. The gradient
form therefore introduces **no error the renderer did not already have**, while
`sqrt(|det M|)` introduces up to 37 steps of new error — an eighth of the full coverage
range, on the edge of every non-uniformly scaled shape.

The gradient is taken by **central differences at a fixed epsilon**, not by `fwidth`.
That distinction is exactly the one §"Rendering conventions" draws: it bans hardware
derivative estimates because they differ between platforms, and explicit central
differences use only `+ - * /` and `sqrt`, all exactly specified by IEEE-754. The
oracle and both shaders compute the identical value.

Cost: **three** distance evaluations per pixel instead of one, and only for an item
that carries a transform — the flag gates it. That is the price of the 37 steps, and
it is worth paying: a non-similarity transform is the case the correction exists for.

**There is no scale-factor header slot.** §4.2 originally carried `sqrt(|det M|)` as a
35th slot computed once on the CPU; the gradient is per-pixel and per-direction, so
there is nothing to precompute. Header stays at **34** slots (0..33), item block
**160** bytes.

### 4.3 Stroke width

`strokeHalf` (slot 7) is in surface pixels today. Under a transform there are two
defensible readings: the stroke scales with the shape (a scaled-up circle has a
scaled-up outline), or it stays constant in surface pixels (a hairline stays a
hairline).

**Decision: the stroke scales with the shape.** It falls out of the design for free —
the stroke band is `|d| - half` evaluated in shape space, so scaling `d` scales the
band — and it is what "transform the item's geometry" means. The alternative requires
correcting the stroke separately from the fill and would make a sheared stroke
non-uniform in a way neither GPU could match cheaply. Recorded in `mod.rs`'s
`Transform` description and in the spec.

### 4.4 Degenerate transforms

`__canvas_invertTransform` returns the identity when:

- the input is all-zero (the documented identity spelling), **or**
- `|det| < 1e-12` (singular — a collapse-to-a-line or to-a-point).

Returning the identity for a singular transform, rather than drawing nothing, is the
choice that keeps a bug visible: an item that vanishes is indistinguishable from an
item that was never presented, whereas an untransformed item is obviously wrong. It
also means no renderer can produce a NaN or an infinity from a transform.

### 4.5 Text under a transform

A glyph's pixels come from a cached coverage bitmap, sampled today at
`(x - glyphX, y - glyphY)` integer offsets. Under a transform the same inverse
machinery applies, one step earlier:

- The glyph's **quad** (its blit bounds) becomes the transformed hull of the
  untransformed quad, exactly as §3's bounds rule does for shapes.
- Per pixel, map `p` through `T⁻¹` (the same six header terms), subtract the glyph
  origin, and sample the coverage bitmap at the **nearest** integer sample; outside
  the bitmap, coverage is 0.
- The sampled coverage then multiplies through the existing text blend unchanged.

Nearest sampling is the deliberate choice: it is integer index arithmetic plus the
same `+ - * /` the oracle already allows, so all three renderers agree exactly, and
the glyph caches stay untransformed (one cache entry serves every transform — the
same sharing argument the geometry cache makes). The cost is that a rotated glyph
edge is nearest-sampled rather than resampled with filtering; the docs must say so
plainly ("rotated or scaled text keeps its rendered crispness but may stair-step;
render at the target size rather than scaling up"). Bilinear filtering is rejected
for this letter: it changes every transformed glyph's edge bytes between renderers
unless all three implement identical fixed-point weights, which is a letter of its
own if ever wanted.

The identity path is gated on the same `hasTransform` flag, so untransformed text —
every existing scene — is byte-identical.

## Compatibility / Format Impact

- **`canvas::` surface unchanged.** No new type, function, field or variant.
- **Observable rendering changes** for any scene already setting a non-identity
  `transform`, which today draws untransformed.
- **`HEADER_SLOTS` 27 → 35**, **`ITEM_BLOCK_SIZE` 128 → 160** — internal.
- **`.ncodesum` churn** on every canvas-emitting target; both `.spv` blobs regenerate.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick in the same commit as the
> work; `- [~]` for partial with a one-line remainder; fill `Commit:` on landing.
> **An unticked box means NOT DONE.**

### Phase 1 — Measure the distance correction before building on it

The letter's one unproven premise, tested as cheaply as possible.

- [x] Write a throwaway harness (or a `#[test]` in `tests/rt_canvas_rasteriser.rs`
      marked `#[ignore]` and kept) that, for a 2:1 non-uniform scale and a 30° shear,
      compares coverage computed with `d/sqrt(|det M|)` against coverage computed
      from a densely supersampled ground truth on the same shape.
      Kept, as `measure_the_transformed_distance_correction`. It measures a
      **half-plane** rather than a curved shape: a circle mixes the correction's error
      with the coverage model's curvature error, and the first run did exactly that —
      the two were indistinguishable until the shape was straightened.
- [x] **Record the worst-case coverage error, in 1/255 steps, in §4.2 of this
      document.** A number, from a run — not an estimate.
- [x] If it exceeds one step, add an Open Decision with the measured figure and a
      recommendation (carry a per-item pair of axis scales, or accept and document a
      tolerance for non-similarity transforms). **Do not stop** — record it and
      proceed with the recommendation.
      It exceeds one step by a wide margin, and the recommendation is **neither** of
      the two the plan offered — see §4.2 as rewritten and Correction C2.

Acceptance: the worst-case error is written into this document with the command that
produced it, and the choice of correction is settled by that number rather than by
argument.

**MET, and it settled the question against §4.2's proposal.**
`cargo test --release --test rt_canvas_rasteriser measure_the_transformed -- --ignored --nocapture`:

```
identity     sqrt(|det|)   3.19/255   d/||grad||   3.19/255
2:1 scale    sqrt(|det|)  37.34/255   d/||grad||   3.19/255
30deg shear  sqrt(|det|)  18.18/255   d/||grad||   9.71/255
measurement floor 3.19/255
```
Commit: ba2240639

### Phase 2 — Carry the inverse; identity path unchanged

- [x] `HEADER_SLOTS` 27 → 34; add slot constants for `ia..ity` and `hasTransform`.
      **34, not 35** — §4.2's scale-factor slot is gone with the formula it carried;
      the gradient is per-pixel, so there is nothing to precompute per item.
- [x] Add `__canvas_invertTransform` to `helper_paint_defaults.rs`, handling both
      degenerate cases per §4.4; it is the sole all-zero→identity mapping.
- [x] `__canvas_paintHeader` writes slots 27–34 from `paint.transform`.
      Slots 27–33 (seven): six terms and the flag. The identity is written as float32
      bit patterns directly (`1.0` is `0x3F800000` = 1065353216), so the identity path
      does not even call the encoder.
- [x] `ITEM_BLOCK_SIZE` 128 → 160; extend the MSL struct and both GLSL blocks;
      `scripts/regen-spirv.sh`.
      Re-measured rather than assumed: glslang now reports `topLevelArrayStride 160`
      with `xform0` at **128** and `xform1` at **144**, matching `ITEM_OFFSET_TRANSFORM`.
      `METAL_EDGE_BASE` moved 131072 → 163840 with the block, and
      `the_metal_shader_edge_base_matches_the_buffer_layout` caught it the moment it
      did not — the third time that guard has fired on a real drift.
- [x] Every `__CANVAS_GEO_HEADER` / `HEADER_SLOTS` reader updated (the same sweep
      plan-116-B Phase 1 did — grep both spellings, fix all hits).
      `the_geo_layout_constants_match_their_rust_counterparts` makes a half-applied
      edit a test failure rather than a rendering one, which is exactly why
      plan-116-B added it before touching the header.
- [x] Tests: existing suite green, no golden change.

Acceptance: `cargo test --no-fail-fast` green and every canvas golden unchanged on
disk. Nothing reads the new slots yet, so any rendering change is a plumbing bug.

**MET**, and the plan's warning was right for the second letter running: widening the
block produced a rendering change, it was a plumbing bug, and it was root-caused rather
than re-baselined. `the_draw_frame_slots_do_not_overlap` — added in plan-116-B for
precisely this — named it exactly: *"`item` at 192 is 160 bytes, so it runs to 352 and
overlaps `texture` at 320"*. Metal's frame is hand-assigned, so every slot above the
item block moved up 32 and `DRAW_FRAME` went 480 → 512.

`rt_canvas_metal` 4, `rt_canvas_font` 10, `rt_canvas_golden` 6,
`rt_canvas_rasteriser` 17 (+1 ignored), and `scripts/test-canvas-vulkan.sh` 12/12 at
`worst=2 differing=0.7748%` — **the same three numbers as before the widening**.
`git status --short tests/golden/canvas/` is empty.
Commit: —

### Phase 3 — The software renderer transforms

- [ ] `__canvas_drawGeometry`: when `hasTransform`, map `(px, py)` through the
      inverse before `__canvas_geoDistance` and divide the result by `‖∇d‖`, taken by
      central differences of the composed field (§4.2 as re-derived in Phase 1 — **not**
      by a precomputed `sqrt(|det M|)`, which measured 37/255 wrong). Gate on the flag
      so the identity path adds one compare per item, not per pixel, and pays for
      neither the mapping nor the two extra distance evaluations.
- [ ] Transform the bounds: `__canvas_boundsHeader` takes the four transformed
      corners' axis-aligned hull when a transform is present.
- [ ] `__canvas_drawGeometry`'s `__CANVAS_GEO_TEXT` arm: when `hasTransform`, map
      the pixel through the inverse and nearest-sample the glyph coverage per §4.5;
      transform the glyph run's blit bounds.
- [ ] Tests: `tests/rt_canvas_rasteriser.rs` gains — a 45°-rotated square (assert the
      corners land where the matrix says); a 2× uniform scale (assert the radius
      doubles and the stroke doubles, per §4.3); an all-zero transform (must be
      byte-identical to the same scene with no transform named); a singular transform
      (must render untransformed, per §4.4); a rotated shape whose transformed bounds
      exceed its untransformed ones (must not be clipped).

Acceptance: the five new cases pass, and every pre-existing canvas golden and
rasteriser case is byte-identical.
Commit: —

### Phase 4 — Metal and Vulkan transform

- [ ] Both fragment shaders: transform `gl_FragCoord.xy` / `[[position]].xy` through
      the inverse and divide the distance by `‖∇d‖`, taken by the same central
      differences the oracle uses at the same fixed epsilon, gated on the flag. The
      epsilon has to match the oracle's exactly — it is part of the specified result,
      not a tuning knob.
- [ ] Both vertex shaders already expand the item's `quad`, which Phase 3 made the
      transformed hull — so **no vertex-stage change should be needed**. Verify this
      rather than assume it; a rotated shape clipped to a stale quad is the failure
      mode.
- [ ] `scripts/regen-spirv.sh`.
- [ ] Both glyph fragment paths (MSL and GLSL) take the same §4.5 inverse-sample,
      gated on the flag; glyph quads take the transformed hull.
- [ ] Neither `*Renderable` predicate needs to decline a transform — including on a
      transformed `Text`. Confirm by test.
- [ ] Tests: a new reference image `tests/golden/canvas/transforms.png` — a rotated
      rect, a scaled circle, a sheared polygon, **and a rotated text label** —
      rendered by the oracle in Phase 3 and matched here by both GPUs within
      `Tolerance::GPU_DEFAULT`.

Acceptance: on a Metal host and a Vulkan box, `transforms.png`'s scene matches the
oracle within `Tolerance::GPU_DEFAULT`, with `MFB_CANVAS_STATS` confirming the GPU
path ran.
Commit: —

### Phase 5 — Docs, and the three-field defect closed

- [ ] `mod.rs` — the `Transform` and `Paint.transform` descriptions describe behaviour;
      state the stroke-scales decision (§4.3) and the singular-transform rule (§4.4).
- [ ] `src/docs/spec/app/06_canvas.md` — the transform's effect on geometry, on
      stroke width, and its non-effect on `clip`.
- [ ] Add a worked `mfb man` example on `canvas::fillStroke` or `canvas::present`
      showing a rotated item, and add that member to `MEMBERS` in
      `tests/cli_canvas_man_examples_compile.rs` if it is not already there (it lists
      13 members — `sed -n 23,37p tests/cli_canvas_man_examples_compile.rs`).
- [ ] `scripts/man-census.sh --memory-scope` → 0 unclassified hits;
      `scripts/man-run-examples.sh canvas --run` passes.
- [ ] `scripts/regen-ncodesum.sh`; prove the delta is this letter's.

Acceptance: `cargo test --no-fail-fast` green on mac+RELEASE and linux+DEBUG,
`scripts/test-accept.sh` green, `scripts/artifact-gate.sh all` 0 diffs, and
`grep -rn "\.transform\|\.clip\|\.blend" src/codegen/builtins/canvas/` now finds
**real reads**, not only doc strings — which is the defect this and plan-116-B set out
to close.
Commit: —

## Validation Plan

- **Tests:** `tests/rt_canvas_rasteriser.rs` (5 transform cases),
  `tests/rt_canvas_golden.rs` (+`transforms.png`), `tests/rt_canvas_metal.rs`,
  `tests/cli_canvas_package.rs`. Negative cases: all-zero transform ≡ no transform;
  singular transform ≡ no transform; transformed bounds not clipped.
- **Coverage check:** as in plan-116-B, the renderer is MFBASIC source compiled into
  emitted programs, so `cargo llvm-cov --bin mfb` does not see it. Coverage is the rt
  tests; confirm the `hasTransform = 0` and `= 1` arms are both exercised.
- **Runtime proof:** render `transforms.png`'s scene three ways (software, Metal,
  Vulkan) and diff.
- **Doc sync:** `src/docs/spec/app/06_canvas.md`; `Transform`/`Paint.transform`
  descriptions in `mod.rs`.
- **Acceptance:** `cargo test --no-fail-fast`, `scripts/test-accept.sh`,
  `scripts/artifact-gate.sh all`, `rustup run 1.96.0 cargo fmt --all &&
  (cd repository && rustup run 1.96.0 cargo fmt)`.

## Open Decisions

- **`sqrt(|det M|)` as the distance correction (§4.2).** Recommended, pending
  Phase 1's measurement. Exact for similarities; approximate for shear and
  non-uniform scale. If Phase 1 shows more than one coverage step of error, the
  fallback is to carry the two singular values of `M` and correct per axis — more
  words in the block, same shape of change.
- **Nearest sampling for transformed glyphs (§4.5).** Recommended and effectively
  decided — it is the only sampling all three renderers reproduce exactly under the
  oracle's arithmetic rules. Filtered sampling, if ever wanted, is its own letter.
- **Stroke scales with the shape (§4.3).** Recommended and effectively decided — the
  alternative is materially more work on both GPUs. Recorded here because both
  readings are defensible and a user will ask.

## Corrections

- **C3 (2026-09-01, Phase 2) — §4.1's "the CPU side needs no conversion either" is
  false, and float32 is not free.** A `Float` is an IEEE **double**; the item block
  needs **binary32**; and the assemblers have no double→single convert and no 32-bit
  float store. `vulkan.rs`'s `emit_store_f32_from_integer` exists because of that exact
  gap and its own doc says it is deliberately *not* a general double-to-float — no
  rounding, no denormals, no zero case — "which is why it is spelled as a private helper
  for this one use rather than an `abi::` primitive that would invite the general one".
  Taking §4.1 at its word would have meant writing that general conversion twice, in two
  different code-emission APIs, where a mistake shows up as a wrong picture rather than
  a failed assertion.
  Resolved by narrowing **once in MFBASIC** (`__canvas_float32Bits`) and carrying bit
  patterns in the header, so both emitters do nothing but `toInt` and `store_u32`.
  Validated against `struct.pack('>f')` over 16 values spanning the range a real inverse
  produces — `1.0`, `-1.0`, `0.0`, `0.01`, `100.0`, `-0.866025403784`,
  `0.5773502691896`, `1e-6`, `1e6`, `3.14159265358979`, `65536.0`, `-0.0001`,
  `123456.75` and three others — **0 mismatches**, including the sign bit and the
  round-carries-into-the-exponent case.
  The extremes are documented rather than discovered: too large saturates to the largest
  finite float, too small flushes to zero. Neither is reachable from a non-singular
  transform, and both beat handing an infinity to a distance field, which poisons a
  whole frame rather than one item.
- **C2 (2026-09-01, Phase 1) — §4.2's recommended correction was measured and
  rejected.** The section recommended `s = sqrt(|det M|)` "computed once on the CPU and
  carried as a 35th header slot", and required Phase 1 to measure it before anything was
  built on it. Measured: **37.34/255 coverage steps** wrong for a 2:1 non-uniform scale
  and **18.18/255** for a 30° shear, against a 3.19/255 measurement floor. That is an
  eighth of the full coverage range on the edge of every non-uniformly scaled shape.
  Replaced by `d / ‖∇d‖` with the gradient taken by explicit central differences, which
  measures **3.19** (the floor — exact) for the scale and **9.71** for the shear, the
  latter being precisely what an *untransformed* 30° edge already measures. The plan
  offered two fallbacks — per-item axis scales, or documenting a tolerance — and neither
  was needed; the third option was better than both and needs no extra header slot at
  all, so the header is 34 slots rather than 35.
  Two details that made the measurement trustworthy and are worth keeping: it uses a
  **half-plane**, because a circle mixes the correction's error with the coverage
  model's curvature error and the first attempt could not separate them; and the
  identity row is carried as a control, which is what identified 3.19 as the grid's
  floor rather than a result.


- **C1 (2026-09-01, review — pre-execution).** As first written this letter was
  silent on `Text` and `Picture`, which are not SDFs — a transformed `Text` would
  have silently rendered untransformed, recreating for one variant the exact
  defect the letter exists to close. §4.5 (glyph inverse-sampling) was added and
  the effort re-estimated large → x-large. `Picture` was discovered to have no
  renderer at all and is scoped out against bug-484. The inverse terms also moved
  from 16.16 to raw float32 bits in the block — the precision question Phase 1 was
  going to measure is avoidable for free.

## Summary

The whole letter rests on one property: a signed distance field is transformed by
transforming the query point, so no primitive's maths changes and no new kind is
needed. The real risk is not the transform but the *identity* — every existing scene
goes through the untransformed arm, and `__canvas_noTransform` returns all zeros, so a
renderer that reads the matrix without the all-zero→identity mapping collapses the
entire canvas to the origin. That mapping lives in one CPU-side function by design.
The one genuinely unproven quantity is the antialiasing correction under shear, and
Phase 1 buys that number before anything depends on it. Untouched: the `canvas::`
type set, the scene ring, and `Paint.clip`'s surface-pixel semantics.
