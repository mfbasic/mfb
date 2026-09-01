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
| plan-116-D complete and archived | `ls planning/completed/plan-116-D-*` → one match | NOT MET |

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
| Exhaustive `MATCH`es over `DrawItem` in canvas | 2 | `grep -n 'MATCH item' src/codegen/builtins/canvas/helper_geometry.rs` → `:128`, `:340` |
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

Header (40 slots after plan-116-D) uses the existing shape slots 2–5 for
`x, y, radiusX, radiusY` and **slot 40** for `angle`'s cosine and **41** for its sine —
precomputed once per ellipse, as `__canvas_arcHeader` already precomputes its sweep
vectors. Header becomes **42**; the item block takes one more `ivec4`, reaching
**192** bytes.

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

The solve is a **fixed-count Newton iteration** on the parametric angle `t`,
minimising `‖q - (rx·cos t, ry·sin t)‖`. Seed `t` from `atan2`-free initial guess
`t₀ = (|q|/r)` normalised — a cheap starting point that is within one quadrant — and
iterate a fixed `N` times. `cos t` and `sin t` inside the loop come from the same
deterministic `__canvas_cos`/`__canvas_sin` pair (`helper_shapes.rs:165,176`), which is
`+ - * /`-only.

The sign is `‖q‖ ≥ ‖nearest‖ ? +1 : -1` — equivalently, inside is
`(qx/rx)² + (qy/ry)² < 1`, which is one comparison and needs no iteration.

**`N` is fixed by Phase 1's measurement**, not by a convergence test. A
`WHILE |Δt| > ε` loop would make the iteration count depend on the input, which is
fine numerically and fatal for the oracle: the three renderers would take different
numbers of steps on the same pixel on different hardware.

**Circle equivalence.** At `rx == ry == r` the nearest-point solve is exact in one
step and `d = ‖q‖ - r`, which is `__canvas_geoDistance`'s circle arm exactly. Phase 3
asserts the bytes match rather than trusting the algebra.

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
- **`HEADER_SLOTS` 40 → 42**, **`ITEM_BLOCK_SIZE` 176 → 192** — internal.
- **`.ncodesum` churn**; both `.spv` blobs regenerate.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick in the same commit as the
> work; `- [~]` for partial with a one-line remainder; fill `Commit:` on landing.
> **An unticked box means NOT DONE.**

### Phase 1 — Fix the iteration count by measurement

The letter's one unproven premise, settled before any renderer changes.

- [ ] Write a harness (a `#[test]` in `tests/rt_canvas_rasteriser.rs`, kept) that
      evaluates the §4.2 solve at `N = 1..8` against a densely supersampled ground
      truth, for eccentricities 1:1, 2:1, 4:1 and 10:1, at radii 5 px and 300 px.
- [ ] **Record in §4.2 the smallest `N` whose worst-case coverage error is under one
      1/255 step, with the command that produced it.** A number from a run.
- [ ] Confirm the seed guess never leaves the solve in a different basin for any of
      those cases (a fixed-count Newton that starts in the wrong quadrant does not
      converge and will not announce it).
- [ ] If no `N ≤ 8` reaches one step, add an Open Decision with the measured figures
      and a recommendation — do **not** stop.

Acceptance: `N` is written into this document with its measured worst-case error, and
the choice is settled by that number rather than by argument.
Commit: —

### Phase 2 — The type, the variant, and the frozen-set amendment

- [ ] Add the `Ellipse` record to `mod.rs` with the six props and their descriptions.
- [ ] Append `Ellipse` **last** to the `DrawItem` union's variant list.
- [ ] Amend `draw_item_variant_set_is_frozen` (`mod.rs:884`): add `"Ellipse"` at the
      end of the pinned list, and extend the doc comment to name plan-116-E as the
      deliberate extension. **Keep the assertion message's warning intact** — the test
      does not become laxer, it records a new decision.
- [ ] Add the `Ellipse` arm to `__canvas_headerFor` (`helper_geometry.rs:127`) and
      `__canvas_tailFor` (`:339`) — the exhaustive `MATCH`es will not compile without
      them, which is the design working.
- [ ] Add `__canvas_ellipseHeader` writing kind 7, the five shape values, the
      precomputed `cos`/`sin`, the paint, and the §4.1 bounds; degenerate radii return
      `__canvas_emptyHeader()`.
- [ ] `HEADER_SLOTS` → 42; every `__CANVAS_GEO_HEADER` reader updated.
- [ ] Tests: `tests/cli_canvas_package.rs` constructs an `Ellipse` and compiles;
      `every_draw_item_variant_has_a_record` and `…carries_a_paint` stay green with no
      edit.

Acceptance: an `Ellipse` in a scene compiles and **draws nothing** (kind 7 has no
distance arm yet, so it falls through to the `1.0e6` default), every existing golden is
byte-identical, and `cargo test --no-fail-fast` is green.
Commit: —

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

<!-- Filled in during execution. -->

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
