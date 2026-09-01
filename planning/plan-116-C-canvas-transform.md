# plan-116-C: `Paint.transform` becomes real

Last updated: 2026-08-31
Effort: large (3h–1d)
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
| plan-116-B complete and archived | `ls planning/completed/plan-116-B-*` → one match | NOT MET |

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
- The all-zero `Transform` is the identity and produces today's exact bytes.
- The bounds an item is rasterised over are the **transformed** bounds, so a rotated
  shape is not clipped to its untransformed box.

### Non-goals (explicit constraints)

- **No new `canvas::` surface.** `Transform` exists; this letter only makes it work.
- **`Paint.clip` is NOT transformed.** It stays axis-aligned in surface pixels —
  the decision recorded in plan-116-B §Open Decisions, and `Bounds` cannot express
  anything else.
- **No non-affine transforms.** A 2×3 matrix is what the type is.
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

**Item block** grows 128 → **160**: two new `ivec4`s, the six inverse terms in 16.16
plus the flag and one spare.

> 16.16 for the inverse is a real precision question. A transform that scales an
> item *up* by 100× has an inverse with terms near `0.01`, which 16.16 holds to about
> 4 significant digits. Phase 1 measures whether that is enough at the pixel level; if
> not, the fix is to carry the inverse as a pair of 32-bit floats instead of 16.16,
> which is a layout change only.

### 4.2 The distance correction

Evaluating the shape at `T⁻¹(p)` gives a distance in *shape* space. Coverage must be
computed in *surface* space, or a scaled-up shape gets a scaled-up (blurry) antialiased
edge and a scaled-down one gets a hard, aliased edge.

The correction is `d_surface = d_shape / s`, where `s` is the local scale factor of
`T⁻¹` — the norm of the gradient of the transformed distance field. For an affine `T⁻¹`
with matrix `M`, the exact factor is direction-dependent; the tractable choices are:

- **`s = sqrt(|det M|)`** — exact for any similarity (uniform scale + rotation +
  translation), which is the overwhelmingly common case, and a bounded approximation
  otherwise.
- **`s = ‖M · n̂‖` for the gradient direction `n̂`** — exact, but needs the gradient,
  which the SDF does not return.

**Recommend `sqrt(|det M|)`, computed once on the CPU and carried as a 35th header
slot**, so the per-pixel cost is one multiply and the GPU never computes a determinant.
Phase 1 measures the error for a 2:1 non-uniform scale and a 30° shear and records the
number here; if it exceeds one coverage step (1/255) at the antialiased edge, escalate
to Open Decisions rather than shipping a known-wrong oracle.

Header becomes **35** slots (0..34), item block **160** bytes.

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

- [ ] Write a throwaway harness (or a `#[test]` in `tests/rt_canvas_rasteriser.rs`
      marked `#[ignore]` and kept) that, for a 2:1 non-uniform scale and a 30° shear,
      compares coverage computed with `d/sqrt(|det M|)` against coverage computed
      from a densely supersampled ground truth on the same shape.
- [ ] **Record the worst-case coverage error, in 1/255 steps, in §4.2 of this
      document.** A number, from a run — not an estimate.
- [ ] If it exceeds one step, add an Open Decision with the measured figure and a
      recommendation (carry a per-item pair of axis scales, or accept and document a
      tolerance for non-similarity transforms). **Do not stop** — record it and
      proceed with the recommendation.

Acceptance: the worst-case error is written into this document with the command that
produced it, and the choice of correction is settled by that number rather than by
argument.
Commit: —

### Phase 2 — Carry the inverse; identity path unchanged

- [ ] `HEADER_SLOTS` 27 → 35; add slot constants for `ia..ity`, `hasTransform` and
      the scale factor.
- [ ] Add `__canvas_invertTransform` to `helper_paint_defaults.rs`, handling both
      degenerate cases per §4.4; it is the sole all-zero→identity mapping.
- [ ] `__canvas_paintHeader` writes slots 27–34 from `paint.transform`.
- [ ] `ITEM_BLOCK_SIZE` 128 → 160; extend the MSL struct and both GLSL blocks;
      `scripts/regen-spirv.sh`.
- [ ] Every `__CANVAS_GEO_HEADER` / `HEADER_SLOTS` reader updated (the same sweep
      plan-116-B Phase 1 did — grep both spellings, fix all hits).
- [ ] Tests: existing suite green, no golden change.

Acceptance: `cargo test --no-fail-fast` green and every canvas golden unchanged on
disk. Nothing reads the new slots yet, so any rendering change is a plumbing bug.
Commit: —

### Phase 3 — The software renderer transforms

- [ ] `__canvas_drawGeometry`: when `hasTransform`, map `(px, py)` through the
      inverse before `__canvas_geoDistance` and divide the result by the scale
      factor. Gate on the flag so the identity path adds one compare per item, not
      per pixel.
- [ ] Transform the bounds: `__canvas_boundsHeader` takes the four transformed
      corners' axis-aligned hull when a transform is present.
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
      the inverse and divide the distance by the scale factor, gated on the flag.
- [ ] Both vertex shaders already expand the item's `quad`, which Phase 3 made the
      transformed hull — so **no vertex-stage change should be needed**. Verify this
      rather than assume it; a rotated shape clipped to a stale quad is the failure
      mode.
- [ ] `scripts/regen-spirv.sh`.
- [ ] Neither `*Renderable` predicate needs to decline a transform. Confirm by test.
- [ ] Tests: a new reference image `tests/golden/canvas/transforms.png` — a rotated
      rect, a scaled circle, a sheared polygon — rendered by the oracle in Phase 3 and
      matched here by both GPUs within `Tolerance::GPU_DEFAULT`.

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
- **16.16 for the inverse terms (§4.1).** Recommended for layout consistency with
  every other block field, but a large upscale gives small inverse terms. Phase 1's
  harness should report the precision at 100× scale too; escalate to 32-bit floats if
  it costs more than one coverage step.
- **Stroke scales with the shape (§4.3).** Recommended and effectively decided — the
  alternative is materially more work on both GPUs. Recorded here because both
  readings are defensible and a user will ask.

## Corrections

<!-- Filled in during execution. -->

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
