# plan-116-E: `canvas::Ellipse` joins the `DrawItem` set

Last updated: 2026-08-31
Effort: large (3h–1d)
Depends on: plan-116-D

The `DrawItem` set has a `Circle` and no ellipse, so a squashed or tilted round shape
has to be approximated by a `Polygon` — which is why the face in
`canvas::present`'s own man example draws its eyes as circles and any non-round eye
would have to be a polygon. This letter adds a ninth variant:

```
canvas::Ellipse
    x       AS Float   ' The centre's X coordinate in pixels.
    y       AS Float   ' The centre's Y coordinate in pixels.
    radiusX AS Float   ' The horizontal radius in pixels.
    radiusY AS Float   ' The vertical radius in pixels.
    angle   AS Float   ' Rotation in radians clockwise from +X. 0.0 is axis-aligned.
    paint   AS Paint   ' How to fill, stroke, blend, transform and clip the item.
```

Behavioral outcome: `Ellipse[x, y, radiusX, radiusY, angle, paint]` renders as a
filled and/or stroked ellipse with correctly antialiased edges at any rotation, on the
software, Metal and Vulkan paths, and `radiusX = radiusY` with `angle = 0.0` renders
**byte-identically** to the `Circle` of the same centre, radius and paint.

References:

- `src/codegen/builtins/canvas/mod.rs:160` — the `DrawItem` frozen-set comment, and
  `:884` — `draw_item_variant_set_is_frozen`, the test this letter must amend.
- `src/codegen/builtins/canvas/helper_geometry.rs:127` — `__canvas_headerFor`, whose
  `MATCH` is exhaustive over the union, so a ninth variant is a compile error there
  until it is handled (stated at `:123`).
- `src/docs/spec/app/06_canvas.md` §"Rendering conventions" — the reproducibility rule
  that constrains the SDF to `+ - * /` and `sqrt`.
- plan-116-C §3 — the SDF-transforms-the-query-point property this letter reuses.

## Prerequisites

See plan-116-A §Prerequisites for the three environment gates.

| Must be true | Command | Status |
|---|---|---|
| plan-116-D complete and archived | `ls planning/completed/plan-116-D-*` → one match | **MET** (2026-09-02: exactly one match, `planning/completed/plan-116-D-canvas-endcaps.md`, archived by `c85876287` and landed on main at `c704db4da`. Every D phase acceptance measured — 96 test binaries on mac RELEASE, 3722 `--bin mfb` unit tests on box 2228, artifact-gate 1828 goldens 0 diffs, test-accept 1348, Vulkan 12/12 on both 2228 glibc and 2227 musl, `endcaps.png` matched exactly by the oracle and within `Tolerance::GPU_DEFAULT` by Metal, man-census 0 unclassified, man-run-examples canvas 21/21.) |

If plan-116-D is not complete, this letter cannot start, full stop. D is the last
letter before this one to grow `HEADER_SLOTS` / `ITEM_BLOCK_SIZE`, and D also
establishes the pattern for adding a required field to a `DrawItem` record — which
this letter extends to adding a whole variant.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command before you continue and again before you stop.

## 1. Goal

- `canvas::Ellipse` is an exported record and the ninth `DrawItem` variant.
- It renders with exact-coverage antialiasing at any rotation, matching the coverage
  rule `06_canvas.md` pins.
- A degenerate ellipse (`radiusX <= 0` or `radiusY <= 0`) draws nothing, matching
  `Circle`'s `radius <= 0` rule (`helper_geometry.rs:220-223`).
- The circle equivalence holds byte-for-byte (§1's behavioral outcome).

### Non-goals (explicit constraints)

- **`Circle` is not removed or reimplemented in terms of `Ellipse`.** It has its own
  kind, its own one-line distance function, and every existing golden. Collapsing the
  two would put mass golden churn behind a feature addition.
- **No elliptical arc.** `Arc` stays circular. An elliptical arc is a different
  variant and is not asked for.
- **No new `Paint` field.**
- **The `DrawItem` variant ORDER is append-only.** `Ellipse` goes **last**, after
  `RoundedRect`, because the frozen-set test pins the order and the order fixes the
  union tags — inserting in the middle would renumber every later variant's tag.
- **No existing golden may move.**

## 2. Current State

### The closed set, and what opening it costs

`mod.rs:160` states the policy: *"The `DrawItem` variant set is closed. Adding a
variant later is a breaking change — a user's `SELECT CASE` over the union stops being
exhaustive."* `draw_item_variant_set_is_frozen` (`mod.rs:884`) pins the exact list and
order with the message *"the DrawItem variant set is frozen; extending it is a
breaking change"*.

**This letter deliberately breaks that freeze**, on the user's explicit instruction.
Per `AGENTS.md`'s never-edit-a-test-to-pass gate, the four questions:

1. **When/why written** — plan-98-A invariant 6, to make an addition *"a deliberate,
   visible act rather than a silent one"* (`mod.rs:881`).
2. **Behaviour it protects** — a user's `MATCH`/`SELECT CASE` over `DrawItem` stays
   exhaustive across compiler versions.
3. **Who else depends** — `__canvas_headerFor` (`helper_geometry.rs:127`) and
   `__canvas_tailFor` (`:339`) `MATCH` exhaustively; both GPU emitters dispatch on the
   geometry `kind`, not on the union, so they are unaffected by the union itself.
4. **Proof it is wrong** — **there is none, and none is claimed.** The test is not
   wrong; it is doing exactly its job. What changes is the *decision* it records, and
   the sanctioned way to change a deliberate freeze is to amend the pinned list with
   the reason recorded — which is what the test's own doc comment asks for ("a
   deliberate, visible act"). The amendment cites this plan.

So the test is **updated, not weakened**: the list grows by one entry at the end, the
assertion message keeps its warning, and the doc comment gains a line naming
plan-116-E as the deliberate extension. It never becomes a laxer assertion.

### How a shape becomes pixels

`__canvas_headerFor` (`helper_geometry.rs:127`) `MATCH`es the union and returns a
fixed-length header whose slot 0 is the geometry kind. `__canvas_geoDistance`
(`helper_draw.rs:61`) dispatches on that kind, as do both shaders' `geoDistance`.
Kinds today: `RECT` 0, `CIRCLE` 1, `SEGMENT` 2, `ARC` 3, `POLYGON` 4, `NONE` 5,
`TEXT` 6 (`helper_draw.rs:24-26`, `helper_geometry.rs:53-57`).

`GEO_KIND_POLYGON` and `GEO_KIND_TEXT` are additionally spelled as Rust string
constants in `runtime/canvas/mod.rs:283,296` because the *emitters* branch on them.
An ellipse's payload fits the item block, so it needs no such constant.

### Measured populations

| What | Count | Command |
|---|---|---|
| `DrawItem` variants today | 8 | `mod.rs:884-908` |
| Exhaustive `MATCH`es over `DrawItem` in canvas | **7** | `grep -rn "AS DrawItem\|OF DrawItem" src/codegen/builtins/canvas/*.rs` (re-measured 2026-09-02 — the plan's `grep -n 'MATCH item'` keys on the parameter name and finds five; see **E4**) |
| Geometry kinds in use | 7 (0–6) | `helper_draw.rs:24-26`, `helper_geometry.rs:53-57` |
| Tests pinning the variant list | 1 | `mod.rs:884` |
| Tests iterating the variant list | 2 | `mod.rs:913` (`every_draw_item_variant_has_a_record`), `:935` (`…carries_a_paint`) |
| `canvas::Circle[` construction sites | 41 | `grep -rn 'Circle\[' --include='*.rs' --include='*.mfb' . \| grep -v '/target/'` |

### Verified properties

- **`Ellipse` satisfies both iterating tests without amendment.** It is a record the
  package declares (`every_draw_item_variant_has_a_record`) and it carries a
  `paint AS Paint` (`every_draw_item_variant_carries_a_paint`). Only the *frozen list*
  test needs the amendment. Read all three at `mod.rs:884-957`.
- **A rotation needs no distance correction.** Unlike plan-116-C's general affine, a
  rotation is an isometry: `|R⁻¹p - R⁻¹q| = |p - q|`, so the distance in ellipse space
  *is* the distance in surface space and no `sqrt(|det|)` divide is needed. This is
  what makes `angle` cheap.
- **UNVERIFIED: how many Newton iterations the ellipse SDF needs for sub-1/255
  coverage error at the antialiased edge, and whether the count can be fixed rather
  than convergence-tested.** A data-dependent iteration count would make the oracle
  depend on arithmetic ordering. Phase 1 measures it. **This is the letter's real
  uncertainty and it is scheduled first.**

## 3. Design Overview

Four pieces:

1. **The SDF.** An ellipse has no closed-form signed distance in `+ - * / sqrt`; the
   standard exact forms need a cube root or trigonometry. The design is a
   **fixed-iteration Newton solve** for the nearest point on the ellipse — every
   operation `+ - * /` and `sqrt`, a *fixed* iteration count, so the result is
   bit-identical on every target and in all three renderers. §4.2.
2. **The type and the variant**, plus the frozen-set amendment (§2).
3. **The header and kind** — kind 7, five shape parameters.
4. **All three renderers**, sharing the one iteration count.

**Where the design uncertainty concentrates:** the iteration count and its worst-case
error. Phase 1 measures it against a supersampled ground truth before anything else is
built.

**Where the correctness risk concentrates:** the circle-equivalence claim. If the
ellipse SDF and the circle SDF disagree at `rx == ry`, a user who switches will see a
1-byte edge shift, and — worse — it would mean the Newton solve is not converging to
the true distance. §1 makes that equivalence a *test*, which is the cheapest available
check on the solve's correctness.

**Byte-identity is NOT this letter's gate**, but it applies to two named subsets:
every existing golden must be unchanged (nothing else moves), and the
circle-equivalence case must be exact. **Expected to diff:** `.ncodesum` on every
canvas-emitting target, both `.spv` blobs, and `mfb man canvas types`.

### Rejected alternatives

- **Approximate `d ≈ (‖p/r‖ - 1) · min(rx, ry)`.** Rejected: it is the common cheap
  ellipse SDF and its error grows with eccentricity, so a 4:1 ellipse's antialiased
  edge would be visibly wrong at the flat ends. It also would not satisfy the circle
  equivalence except at `rx == ry`, which conceals nothing but proves nothing either.
- **Tessellate to a polygon in the geometry builder.** Rejected: it reintroduces the
  problem `Ellipse` exists to solve, and the vertex count would have to depend on the
  radius, making the geometry cache key size-dependent.
- **Reuse plan-116-C's transform machinery — an ellipse *is* a scaled circle.**
  Rejected as the *implementation*: a non-uniform scale is exactly the case
  plan-116-C §4.2 corrects only approximately, so an ellipse built this way would
  inherit that approximation on its antialiased edge. `Ellipse` deserves an exact SDF.
  (A user may still *apply* a transform to an `Ellipse`; the two compose.)
- **Insert `Ellipse` next to `Circle` in the union for readability.** Rejected: the
  order fixes the tags (`mod.rs:882`), so inserting renumbers `Arc`, `Text` and
  `RoundedRect`.

## 4. Detailed Design

### 4.1 The type and the kind

`add_record(RegistryRecord { name: "Ellipse", … })` with the six props above, and
`UnionVariant { name: "Ellipse", … }` appended **last** in the `DrawItem` list.

New geometry kind `__CANVAS_GEO_ELLIPSE = 7`, declared beside the others in
`helper_geometry.rs:53` and mirrored in `helper_draw.rs`'s kind block.

Header (**39** slots after plan-116-D, 0–38) uses the existing shape slots 2–5 for
`x, y, radiusX, radiusY` and **slot 39** for `angle`'s cosine and **40** for its sine —
precomputed once per ellipse, as `__canvas_arcHeader` already precomputes its sweep
vectors. Header becomes **41**; the item block takes one more `ivec4`, reaching
**192** bytes.

> **Corrected 2026-09-02 (E1).** This said 40 slots after D, cos/sin at 40–41, header
> → 42. plan-116-D landed **39**
> (`grep -n "^pub(crate) const HEADER_SLOTS" src/codegen/runtime/canvas/mod.rs` → 39,
> `helper_geometry.rs:53` → `LET __CANVAS_GEO_HEADER AS Integer = 39`), because D's own
> Correction D1 found the same one-slot overestimate inherited from plan-116-C. Every
> number here shifts down by one. `ITEM_BLOCK_SIZE` is 176 as this letter assumed
> (`mod.rs:323`), so 176 → 192 stands. **The pattern is now three letters deep** —
> C, D and E each wrote slot numbers predicting where the previous letter would land,
> and each was one high — so a later letter should take the header size from the
> constant rather than from its own prose.

Storing `cos`/`sin` rather than the angle keeps the only `__canvas_cos`/`__canvas_sin`
calls out of the per-pixel path and out of the shaders, which matters for the oracle:
`helper_shapes.rs:165` explains at length that `math::sin`/`cos` cannot be used because
libm is not correctly rounded, and the shaders' hardware `sin`/`cos` differ again. One
CPU-side evaluation with the deterministic Taylor pair means **all three renderers get
the same two numbers**, which is the only way this variant can be byte-identical
across backends.

`__canvas_ellipseHeader` returns `__canvas_emptyHeader()` when `radiusX <= 0.0` or
`radiusY <= 0.0`, mirroring `__canvas_circleHeader:220`.

Bounds: the axis-aligned hull of a rotated ellipse is
`hx = sqrt((rx·cos)² + (ry·sin)²)`, `hy = sqrt((rx·sin)² + (ry·cos)²)`, plus
`strokeHalf + 1.0` as every other kind does. Exact, and `sqrt`-only.

### 4.2 The signed distance

Evaluate in the ellipse's own frame: `q = R(-angle) · (p - centre)`, using the stored
`cos`/`sin`. Then fold to the first quadrant by taking `|q|` — the ellipse is
symmetric in both axes — and solve for the nearest point on `x²/rx² + y²/ry² = 1`.

> **Phase 1 measured the Newton solve below and REJECTED it — see E2.** What ships is
> the fallback this letter already named: **fixed-count bisection on the folded
> quadrant, 24 halvings**, worst-case **0.0215 coverage steps** of 1/255. The Newton
> description is kept for the record, struck through, because the reason it fails is
> the useful part.

~~The solve is a **fixed-count Newton iteration carried on the unit pair `(c, s)`**~~
~~— never on an angle, so no trigonometric function appears anywhere in it.~~
~~Minimise `‖q - (rx·c, ry·s)‖` subject to `c² + s² = 1`, seeding from the gradient~~
~~direction and stepping by the renormalised small-angle rotation.~~

**Rejected by measurement.** At *every* count tried — N = 1, 2, 4, 6 and 8 — the worst
coverage error is **127.5 steps**, which is the signature of the solve landing on the
wrong side of the ellipse entirely rather than of slow convergence. The basin probe
puts a number on it: **411 of 1608** sample points converge to a stationary point that
is not the nearest one. A localized case, at `rx = 5, ry = 1.25, q = (4.3269,
1.8291)`: the iteration settles at angle **−2.03 rad** and reports `d = 3.40` where the
true distance is `1.13`.

That is precisely the failure the plan's own Phase 1 box anticipated — "a fixed-count
Newton that starts in the wrong quadrant does not converge and will not announce it" —
and it is worth being clear that the seed is *not* the problem: the seed is in the
first quadrant by construction after the `|q|` fold. The problem is the step. For an
eccentric ellipse the gradient-direction seed can sit outside the evolute, where the
squared-distance function has three stationary points in the quadrant, and Newton has
no preference among them.

**What ships: fixed-count bisection on the folded quadrant.**

Bisect the sign of `g(t) = (q − P(t)) · P′(t)`, the derivative of the squared distance,
with the quadrant's endpoints as the initial `(c, s)` pairs. After the `|q|` fold the
bracket is guaranteed *by construction* rather than by any property of the input:
`g(1, 0) = qy·ry ≥ 0` and `g(0, 1) = −qx·rx ≤ 0`. Each halving is the angular midpoint
of two unit vectors — their sum, normalised — so no trigonometry appears:

```
cm, sm = normalise(c₀ + c₁, s₀ + s₁)
if g(cm, sm) > 0 then (c₀, s₀) = (cm, sm) else (c₁, s₁) = (cm, sm)
```

Every operation is `+ - * /` and `sqrt`, the count is fixed, and the branch is on a
sign rather than a magnitude — so all three renderers take the same path.

**`N` = 24 halvings**, measured
(`cargo test --release --test rt_canvas_rasteriser measure_the_ellipse -- --ignored
--nocapture`):

| halvings | worst coverage error, all cases |
|---|---|
| 16 | 5.5008 steps |
| 20 | 0.3438 steps |
| **24** | **0.0215 steps** |

The plan's suggested 16 is **not enough**: the error scales with the radius, because
the angular bracket after `k` halvings spans an arc proportional to `r`. 16 halvings is
fine at `rx = 5` (0.03 steps) and 5.5 steps wrong at `rx = 900`. The measurement was
therefore extended past the plan's 5 px and 300 px to **450 and 900** — a canvas is 900
px wide, so an ellipse can legitimately be larger than the range the plan sampled, and
a count chosen at 300 and deployed at 900 would be a third as accurate. At 24 halvings
the worst case over every radius and eccentricity tried is 1/46th of a coverage step.

**Circle equivalence — by construction, not by convergence.** A fixed-`N` Newton
solve is never algebraically exact (for a circle the angle iteration is
`t₁ = t₀ - tan(t₀ - θ)`, exact only from a perfect seed), and a last-bit residual
in `d` can flip the `clamp(0.5 - d, 0, 1)` coverage quantisation on whichever edge
pixel lands nearest a 1/255 step — so "byte-identical to `Circle`" cannot be
promised from the solve. The SDF therefore **special-cases `rx = ry`**: an exact
float compare guards `RETURN sqrt(qx² + qy²) - rx` — literally the circle arm of
`__canvas_geoDistance` — before the iteration, in all three renderers. The §1
equivalence then holds by construction, and Phase 3's test pins it.

**Continuity at the guard, measured (E3).** The plan proposed to check this by
comparing the solve at `ry = rx·(1 ± 1/4096)` against the circle arm, which turns out
to ask the wrong question: those are *different shapes*, and they differ by about
`|ry − rx|` in distance however good the solve is. Measured that way `rx = 300` reads
as 18.7 steps, which is just `300/4096 × 255` and says nothing about the guard.

The two questions that matter are answered instead:

| `rx` | at the guard (`ry = rx`), solve vs circle arm | off it: 1/1024 → 1/4096 → 1/16384 |
|---|---|---|
| 5 | **0.0001 steps** | 1.245 → 0.311 → 0.078 |
| 300 | **0.0072 steps** | 74.710 → 18.682 → 4.674 |
| 900 | **0.0215 steps** | 224.121 → 56.047 → 14.026 |

At the handover point the two arms agree to well under one 1/255 step, so the exact
float compare introduces no jump. Off it the difference falls by exactly 4× as the
separation falls by 4× — linear in `|ry − rx|`, i.e. the shapes' own difference going
to zero. **There is no seam**; what the plan feared was an artefact of the comparison
it proposed.

### 4.3 The three renderers

The iteration is short and branch-free, so all three get the same code shape:

- **Software** — `__canvas_ellipseDistance` in `helper_shapes.rs`, called from
  `__canvas_geoDistance`'s new kind-7 arm.
- **Metal / Vulkan** — the same function in MSL and GLSL, using each language's
  `cos`/`sin`. **This is the one place the three renderers cannot be bit-identical**,
  because the shaders' trigonometry is the hardware's. That is already true of the
  `Arc` kind (`mfb_canvas.frag:geoDistance` calls `cos`/`sin` directly) and is why the
  GPU comparison uses `Tolerance::GPU_DEFAULT` rather than exact match. Phase 4 must
  confirm the ellipse stays inside that tolerance; if it does not, the fix is to carry
  more precomputed constants, not to loosen the tolerance.

## Compatibility / Format Impact

- **BREAKING: the `DrawItem` union gains a ninth variant.** A user's exhaustive
  `MATCH` over `DrawItem` stops compiling until it handles `Ellipse` — the exact break
  `mod.rs:160` describes. Deliberate and user-directed.
- **`canvas::Ellipse` is new exported surface**; `mfb man canvas types` grows.
- **No existing scene changes.** No existing record, field or kind is touched.
- **`HEADER_SLOTS` 39 → 41**, **`ITEM_BLOCK_SIZE` 176 → 192** — internal.
  (Corrected 2026-09-02 from 40 → 42; see E1 in §4.1.)
- **`.ncodesum` churn**; both `.spv` blobs regenerate.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick in the same commit as the
> work; `- [~]` for partial with a one-line remainder; fill `Commit:` on landing.
> **An unticked box means NOT DONE.**

### Phase 1 — Fix the iteration count by measurement

The letter's one unproven premise, settled before any renderer changes.

- [x] Write a harness (a `#[test]` in `tests/rt_canvas_rasteriser.rs`, kept) that
      evaluates the §4.2 solve at `N = 1..8` against a densely supersampled ground
      truth, for eccentricities 1:1, 2:1, 4:1 and 10:1, at radii 5 px and 300 px.
      → `measure_the_ellipse_newton_iteration_count`, `#[ignore]`d like plan-116-C's
      measurement harness. Radii **450 and 900 added** beyond the plan's range: the
      bisection error scales with the radius and a canvas is 900 px wide, so a count
      chosen at 300 would be a third as accurate where it is actually used.
- [x] **Record in §4.2 the smallest `N` whose worst-case coverage error is under one
      1/255 step, with the command that produced it.** A number from a run. → **24
      bisection halvings, 0.0215 steps**; 16 gives 5.5008 and 20 gives 0.3438. The
      Newton solve the section specified reaches one step at **no** count.
- [x] Confirm the seed guess never leaves the solve in a different basin for any of
      those cases (a fixed-count Newton that starts in the wrong quadrant does not
      converge and will not announce it). → it does, **411 of 1608** probes. See **E2**;
      the seed is fine and the *step* is the problem. The bisection that ships is
      checked by the same probe and passes it as an assertion rather than a count.
- [x] Measure the `rx = ry` special-case seam: compare the guard arm against the
      `N`-step solve at `ry = rx·(1 ± 1/4096)` and record the worst coverage
      difference (§4.2 requires it under one 1/255 step). → the proposed comparison
      asks the wrong question (**E3**); measured properly, the two arms agree to
      0.0001/0.0072/0.0215 steps *at* the guard for `rx` = 5/300/900, and the
      difference off it is linear in `|ry − rx|`. No seam.
- [x] If no `N ≤ 8` reaches one step at 10:1, switch the solve to the named
      fallback — **fixed-count bisection on the folded quadrant**, seeded with the
      quadrant's endpoints as `(c, s)` pairs and halved by midpoint-renormalise
      (`c = (c₀+c₁)/n`, same `sqrt` form), ~~16~~ **24** halvings — which is
      guaranteed-convergent, branch-count-fixed, and still `+ - * / sqrt`-only.
      Record the measured error of whichever solve ships; do **not** stop.
      → taken. 16 halvings was the plan's suggestion and is **not enough** (5.5 steps
      at `rx = 900`).

Acceptance: `N` is written into this document with its measured worst-case error, and
the choice is settled by that number rather than by argument.

**MET.** §4.2 now carries the bisection solve at **24 halvings** with its measured
worst case of **0.0215 coverage steps**, the table of 16/20/24, and the rejected
Newton solve struck through with the 127.5-step and 411/1608 numbers that rejected it.
Every figure comes from
`cargo test --release --test rt_canvas_rasteriser measure_the_ellipse -- --ignored
--nocapture`.
Commit: 5ef61834f

### Phase 2 — The type, the variant, and the frozen-set amendment

- [x] Add the `Ellipse` record to `mod.rs` with the six props and their descriptions.
- [x] Append `Ellipse` **last** to the `DrawItem` union's variant list.
- [x] Amend `draw_item_variant_set_is_frozen` (`mod.rs:884`): add `"Ellipse"` at the
      end of the pinned list, and extend the doc comment to name plan-116-E as the
      deliberate extension. **Keep the assertion message's warning intact** — the test
      does not become laxer, it records a new decision. → done; the doc comment now
      says what was added, that it was appended rather than inserted and why, and
      that the next addition stays exactly as visible as this one.
- [x] Add the `Ellipse` arm to `__canvas_headerFor` (`helper_geometry.rs:127`) and
      `__canvas_tailFor` (`:339`) — the exhaustive `MATCH`es will not compile without
      them, which is the design working. → **seven `MATCH`es, not two** (see **E4**),
      and the design did work: each missing arm was a named compile error naming the
      variant it lacked.
- [x] Add `__canvas_ellipseHeader` writing kind 7, the five shape values, the
      precomputed `cos`/`sin`, the paint, and the §4.1 bounds; degenerate radii return
      `__canvas_emptyHeader()`.
- [x] `HEADER_SLOTS` → 41; every `__CANVAS_GEO_HEADER` reader updated.
- [x] Tests: `tests/cli_canvas_package.rs` constructs an `Ellipse` and compiles;
      `every_draw_item_variant_has_a_record` and `…carries_a_paint` stay green with no
      edit. → both stayed green untouched, as §2 predicted. The construction needed
      two more edits than the box implies: that scene asserts its own item count
      (`IF len(scene) <> 8`), twice, so adding an item is not additive there.

Acceptance: an `Ellipse` in a scene compiles and **draws nothing** (kind 7 has no
distance arm yet, so it falls through to the `1.0e6` default), every existing golden is
byte-identical, and `cargo test --no-fail-fast` is green.

**MET.** A scene whose only item is
`canvas::Ellipse[x := 450, y := 320, radiusX := 200, radiusY := 80, angle := 0.4, …]`
compiles and renders **0 lit pixels of 576000** — the kind falls through to the
`1.0e6` default exactly as intended. `rt_canvas_golden` 10/10 with
`git status --short tests/golden/canvas/` empty, `cli_canvas_package` 7/7,
`draw_item_variant_set_is_frozen` and its two sibling iterating tests green.
Commit: b979761a1

### Phase 3 — The software SDF, and the circle equivalence

- [ ] `__canvas_ellipseDistance` in `helper_shapes.rs`, implementing §4.2 at Phase 1's
      `N`, using `__canvas_cos`/`__canvas_sin`.
- [ ] Kind-7 arm in `__canvas_geoDistance` (`helper_draw.rs:61`).
- [ ] Tests: `tests/rt_canvas_rasteriser.rs` —
      **circle equivalence** (`Ellipse[rx := r, ry := r, angle := 0]` byte-identical to
      `Circle[radius := r]`, same paint, same position — the load-bearing case);
      an axis-aligned 3:1 ellipse (assert the four extreme points and four interior
      points); the same ellipse at `angle := PI/4` (assert the rotated extremes);
      `radiusX = 0` (draws nothing); a stroked ellipse (assert the band's inner and
      outer edges).

Acceptance: the five cases pass, **the circle-equivalence case is exact**, and every
pre-existing golden is byte-identical.
Commit: —

### Phase 4 — Metal and Vulkan

- [ ] The same solve in MSL and in both GLSL files; extend the item block by one
      `ivec4` for `cos`/`sin`; `scripts/regen-spirv.sh`.
- [ ] Both `*Renderable` predicates: kind 7's payload fits the item block, so neither
      needs to decline it. **Confirm by test** — a predicate that silently accepts a
      kind its shader does not know renders the item as *nothing* and reports success,
      which `.ai/canvas-threading.md` §10 records as having actually happened
      (4,536 pixels wrong, reported as success).
- [ ] New reference image `tests/golden/canvas/ellipses.png`: axis-aligned, rotated,
      high-eccentricity, filled and stroked.
- [ ] Tests: both GPUs match the oracle on `ellipses.png` within
      `Tolerance::GPU_DEFAULT`.

Acceptance: `ellipses.png` matches on both GPUs within `Tolerance::GPU_DEFAULT`, with
`MFB_CANVAS_STATS` confirming `metalReady=TRUE` / `vulkanReady=TRUE`. If the tolerance
is exceeded, carry more precomputed constants — do not loosen the tolerance.
Commit: —

### Phase 5 — Docs and gates

- [ ] `mod.rs` — the `Ellipse` record and each prop's description; the `DrawItem`
      variant blurb. Update the *"eight `DrawItem` variants"* comment at `mod.rs:472`
      and the frozen-set language at `:160` to say nine and to record that the set was
      extended once, deliberately, by this plan.
- [ ] `src/docs/spec/app/06_canvas.md` — `Ellipse` in the item list; the `angle`
      convention (radians clockwise from +X, matching `Arc`'s, per §"Coordinates and
      angles").
- [ ] A worked `mfb man canvas ellipse`-reachable example; add it to `MEMBERS` in
      `tests/cli_canvas_man_examples_compile.rs` if it ships one.
- [ ] `scripts/man-census.sh --memory-scope` → 0 unclassified hits;
      `scripts/man-run-examples.sh canvas --run` passes.
- [ ] `scripts/regen-ncodesum.sh`; prove the delta is this letter's.

Acceptance: `cargo test --no-fail-fast` green on mac+RELEASE and linux+DEBUG,
`scripts/test-accept.sh` green, `scripts/artifact-gate.sh all` 0 diffs, and
`mfb man canvas types` lists `Ellipse` with all six props documented.
Commit: —

## Validation Plan

- **Tests:** `tests/rt_canvas_rasteriser.rs` (5 cases + the Phase 1 harness),
  `tests/rt_canvas_golden.rs` (+`ellipses.png`), `tests/rt_canvas_metal.rs`,
  `tests/cli_canvas_package.rs`, `mod.rs`'s three union tests. Negative cases:
  `radiusX = 0`; `radiusY = 0`; a negative radius (must behave as zero, not as its
  absolute value — assert which).
- **Coverage check:** the SDF is MFBASIC source in emitted programs, invisible to
  `cargo llvm-cov --bin mfb`. Coverage is the rt cases; confirm the inside/outside sign
  branch and the degenerate-radius early return are each exercised.
- **Runtime proof:** render `ellipses.png`'s scene software / Metal / Vulkan and diff;
  separately, render the circle-equivalence pair and diff them against each other.
- **Doc sync:** `src/docs/spec/app/06_canvas.md`; the `Ellipse` descriptions and the
  "eight variants" comments in `mod.rs`; `.ai/canvas-threading.md` §10 if the
  predicates change.
- **Acceptance:** `cargo test --no-fail-fast`, `scripts/test-accept.sh`,
  `scripts/artifact-gate.sh all`, `rustup run 1.96.0 cargo fmt --all &&
  (cd repository && rustup run 1.96.0 cargo fmt)`.

## Open Decisions

- **Fixed Newton count `N` (§4.2).** Recommended; `N` set by Phase 1. The alternative
  — iterate to convergence — is rejected in §4.2 because a data-dependent count breaks
  the oracle's cross-target reproducibility.
- **Negative `radiusX`/`radiusY`.** Recommend treating `<= 0.0` as "draw nothing",
  mirroring `__canvas_circleHeader:220-223` exactly, rather than taking an absolute
  value. Assert it either way; the point is that it is decided and tested, not
  incidental.
- **Whether the GPU ellipse can stay inside `Tolerance::GPU_DEFAULT` (§4.3).**
  Unknown until Phase 4. If not, carry more precomputed constants from the CPU. Do
  not loosen the tolerance — it is the gate that caught a real backend lie before.

## Corrections

- **E4 (2026-09-02, Phase 2) — there are seven exhaustive `MATCH`es over `DrawItem`,
  not the two the census names, and the census's own command cannot find them.** §2
  says "Exhaustive `MATCH`es over `DrawItem` in canvas | 2 |
  `grep -n 'MATCH item' src/codegen/builtins/canvas/helper_geometry.rs`". That grep
  keys on the *parameter name*: it finds five, and misses `__canvas_deferredHash` and
  `__canvas_hashItem`, whose `MATCH`es are on differently-shaped bindings. The real
  population is **seven**, all in `helper_geometry.rs`:
  `__canvas_headerFor`, `__canvas_tailFor`, `__canvas_tailMatches`,
  `__canvas_headerIsDeferred`, `__canvas_deferredHeader`, `__canvas_deferredHash`,
  `__canvas_hashItem`. The census that finds them is by *type* —
  `grep -rn "AS DrawItem\|OF DrawItem" src/codegen/builtins/canvas/*.rs`.

  This one cost nothing, which is the point worth recording: MFBASIC's exhaustiveness
  check caught every omission by name
  (`error[2-203-0062 TYPE_MATCH_NOT_EXHAUSTIVE]: MATCH on UNION canvas.DrawItem does
  not cover canvas.Ellipse`), so the wrong count was a wrong *estimate* and never a
  wrong *result*. Contrast plan-116-D's D5, where the missed sites were in a shell
  script no compiler reads and the census error would have shipped. **A census over
  something a compiler checks is advisory; a census over something it does not is
  load-bearing.**

- **E3 (2026-09-02, Phase 1) — the seam check the plan specifies asks the wrong
  question.** It says to "compare the guard arm against the `N`-step solve at
  `ry = rx·(1 ± 1/4096)`" and requires the difference to be under one 1/255 step. But at
  `ry ≠ rx` the two arms describe *different shapes*: a circle of radius `rx` and an
  ellipse that is `rx/4096` taller. They differ by about that much in distance however
  accurate the solve is. Measured as written, `rx = 300` gives **18.68 steps** — which
  is just `300/4096 × 255`, a restatement of the separation, and would have failed an
  acceptance criterion that nothing was wrong with.

  The guard's actual risk is a **discontinuity**, so the questions are (a) do the arms
  agree *at* the handover, and (b) does the difference go to zero as the shapes
  converge. Both measured: at `ry = rx` exactly, 0.0001 / 0.0072 / 0.0215 steps for
  `rx` = 5 / 300 / 900; off it, 74.71 → 18.68 → 4.67 at `rx = 300` as the separation
  falls 1/1024 → 1/4096 → 1/16384, i.e. exactly linear. No jump. §4.2's paragraph is
  replaced with that table.

- **E2 (2026-09-02, Phase 1) — §4.2's Newton solve is not viable at any iteration
  count, and the fallback's suggested 16 halvings is not enough either.** Both settled
  by the measurement the phase exists to make.

  The Newton form reaches a worst-case coverage error of **127.5 steps** at N = 1, 2, 4,
  6 *and* 8 — a flat 127.5 rather than a decreasing sequence, which is the signature of
  landing on the wrong side of the ellipse rather than of slow convergence. The basin
  probe counts it: **411 of 1608** points converge to a stationary point that is not the
  nearest. Localized: `rx = 5, ry = 1.25, q = (4.3269, 1.8291)` settles at angle
  **−2.03 rad** and reports `d = 3.40` against a true `1.13`.

  The seed is not at fault — after the `|q|` fold it is in the first quadrant by
  construction. The step is: outside the evolute of an eccentric ellipse the squared
  distance has three stationary points in the quadrant and Newton has no preference
  among them. No iteration count fixes that, which is why the numbers do not improve
  with N.

  So the plan's named fallback ships. Its suggested **16 halvings is also wrong**: the
  bisection error scales with the radius, so 16 is 0.03 steps at `rx = 5` and **5.50**
  at `rx = 900`. **24 halvings** gives 0.0215. The measurement was extended to radii 450
  and 900 to find that — the plan sampled only 5 and 300, and a canvas is 900 px wide.

  One thing this cost, worth recording: the harness's first basin assertion was
  `d.is_finite() && d > 0.0`, which **passes** for a solve that converged to the far
  side — that answer is finite and positive, just six times too large. An assertion
  about a numerical result has to compare it against the truth, not against its type.

- **E1 (2026-09-02, pre-Phase 1) — the header slot numbers were one high, for the third
  letter running.** This letter says the header is 40 slots after plan-116-D and puts
  `angle`'s cosine and sine at 40–41, making it 42. D landed **39**
  (`grep -n "^pub(crate) const HEADER_SLOTS" src/codegen/runtime/canvas/mod.rs` → 39;
  `helper_geometry.rs:53` → `LET __CANVAS_GEO_HEADER AS Integer = 39`). Corrected to
  cos/sin at **39–40** and `HEADER_SLOTS` → **41** in §4.1, §Compatibility and Phase 2.
  `ITEM_BLOCK_SIZE` was assumed correctly at 176.

  The interesting part is that this is the *same* defect D recorded as its own D1, and
  C's Correction C2 is the root of both: C predicted it would need a per-axis slot for
  the distance correction, measured that it did not, and landed one slot lower than
  every later letter had assumed. Each subsequent letter then wrote its numbers
  relative to a header that was never that size. **The general fix is not to write
  absolute slot numbers in a plan at all** — take the base from `HEADER_SLOTS` and
  describe the new slots as offsets from it. Letters F–J should be read with that in
  mind; each is checked against the constant at its own Phase 1.

## Summary

Two things carry the risk. The first is numerical: an ellipse has no closed-form
signed distance in the operations the oracle is allowed to use, so the design buys
exactness with a fixed-count Newton solve, and Phase 1 fixes that count by measurement
rather than by taste. The second is the frozen `DrawItem` set — this letter is the
first thing to open it, and it does so by amending the pinning test with the reason
recorded, never by weakening the assertion. The circle-equivalence test is the cheapest
real check that the solve is correct, which is why §1 makes it a stated outcome rather
than a nice-to-have. Untouched: `Circle`, `Arc`, every other variant, and every
existing golden.
