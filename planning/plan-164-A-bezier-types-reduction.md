# plan-164-A: Bézier curve shapes, validation, and reduction to quadratic pieces

Last updated: 2026-10-11
Overall Effort: huge (>3d) — the whole plan-164 curve feature, split A–C below
Effort: large (3h–1d)
Depends on: plan-163 (all of A–D) and plan-172 (all of A–N); both complete and
archived — see Prerequisites

plan-164 gives the general canvas (not just SVG content) true curves, as **two new
shapes** that are the curved counterparts of the two the package already has:

| Straight | Curved | What it is |
|---|---|---|
| `canvas::PolyLine` | **`canvas::PolyCurve`** | a chain of curves joined end to end, drawn from its stroke; no interior |
| `canvas::Polygon` | **`canvas::CurvePolygon`** | closed contours of curves, filled under a `FillRule`, holes and all |

Both are built from `canvas::BezierCurve`, one segment: linear, quadratic, cubic,
rational quadratic, rational cubic, higher-degree polynomial or higher-degree rational.
A chain of them is a composite piecewise spline. They are drawn **on the GPU per
pixel**, not flattened to straight lines. The letters are in implementation order:

| Letter | Delivers | Effort |
|---|---|---|
| **A** (this) | The public types, both shapes in the `Shape` union, validation, `canvas::nurbsToBeziers`, `canvas::flatten`, and the CPU **reduction** of every curve to y-monotone rational-quadratic pieces | large |
| B | The software renderer (the oracle) draws both shapes: fill winding, distance, stroke, caps and joins at real corners, dashes along curves; the hit region; `AutoPaint` over either; a `Picture` cut to a `CurvePolygon` | large |
| C | Metal and Vulkan draw the pieces per pixel (a curve region and band index), with a measured curve tolerance, plus docs and spec | large |

**The design in one paragraph.** Every curve, whatever its kind or degree, is reduced
**once, on the CPU, when the item's geometry is built** into a list of **y-monotone
rational quadratic pieces**. Quadratics and rational quadratics convert exactly; lines
are the degenerate case; cubics and higher degrees are fitted within a stated
tolerance. The GPU draws only that one piece type, per pixel:

- a quadratic equation gives the exact inside/outside test;
- a closed-form cubic solve gives a polynomial piece's distance;
- a fixed-step search gives a rational piece's distance, as `Ellipse` already does
  with 24 halvings (`helper_shapes.rs:__canvas_ellipseDistance`).

Rational quadratics are exact conics, so circles, ellipses and SVG arcs stay exact.
The GPU is compared against the software renderer within a **curve-specific
tolerance**, measured in plan-164-C. That is the price of doing the work on the GPU,
and the user accepted it (2026-09-28).

**This letter's single behavioral outcome:** the types exist and are validated; both
shapes are members of `Shape`; `canvas::nurbsToBeziers` converts NURBS exactly;
`canvas::flatten` returns points within tolerance; and the internal reducer turns any
valid curve into y-monotone rational-quadratic pieces whose max deviation from the
input curve is within the reduction tolerance (and zero for quadratic and
rational-quadratic input). Drawing is B and C. In this letter both shapes **draw
nothing**, are hit by nothing, and do not trap.

References:

- `planning/new_canvas.md` — the shape model plan-172 built: every shape leads with
  `id`, is placed by `position`, carries its own `transform`, and has its geometry in
  offsets from `position`. Both shapes are written to that model.
- plan-163 A–D (`planning/completed/plan-163-*.md`). The shapes reuse `FillRule`, B's
  caps/joins/dashes, C's clip/`transparency`, and D's gradients.
- Precedents in the code today:
  - TrueType quadratic curves are split into straight edges in
    `helper_glyph.rs:__canvas_quadEdges`;
  - the ellipse distance uses a fixed-step bisection
    (`helper_shapes.rs:__canvas_ellipseDistance`, 24 halvings, with the measured
    rationale in the doc comment above it).
- `src/docs/spec/app/06_canvas.md` — "The item unions", "Coordinates and angles" and
  its "Placement" subsection (the transform order and the zero-value rule).
- `.ai/man-content.md` (the registry descriptor prose), `.ai/specifications.md`.
- The three unions' variant sets are frozen by three tests in `mod.rs`:
  `shape_variant_set_is_frozen`, `auto_shape_variant_set_is_frozen` and
  `draw_item_variant_set_is_frozen`. Adding two shapes changes the first and the third
  deliberately.

## Prerequisites

This gate is the only hard stop for plan-164. Each row names the letters it gates.

| Must be true | Gates | Command | Status |
|---|---|---|---|
| plan-163-A..D all complete and archived | A–C | `ls planning/completed/plan-163-{A,B,C,D}-*` → 4 matches | MET (re-run 2026-10-11 at the start of execution: 4) |
| plan-172 (the canvas shape model) is complete and archived, every letter A–N | A–C | `ls planning/plan-172-*` → no matches, and `ls planning/completed/plan-172-M-*` → present | MET (re-run at the start of execution: no matches; `plan-172-M-canvas-closeout.md` present) |
| Neither shape exists yet | A | `grep -c '"PolyCurve"\|"CurvePolygon"\|"BezierCurve"' src/codegen/builtins/canvas/mod.rs` → 0 | MET (re-run at the start of execution, before the first edit: 0) |
| Whether plan-150 has landed, which decides what else is renumbered | A | `grep -c '"ParticleSystem"' src/codegen/builtins/canvas/mod.rs` | 0, re-run at the start of execution: plan-150 has not landed, so three tags move and plan-150-A does the two-table edit when it starts. If it answers 1, `ParticleSystem` moves by two tags with `AutoPaint`, `Picture` and `Group`, and plan-150-A's emitter and template tables each need the two new shapes (Open Decisions) |

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run
> it before you continue and before you stop, and report it if you stop.

### Decisions this plan rests on (owner, 2026-10-11)

- **Two shapes, not one item.** The curve items are "the BezierCurve version of
  Polygon and PolyLine": `PolyCurve` and `CurvePolygon`.
- **Both are members of `Shape`**, so an `AutoPaint` may wrap either, plan-150 may emit
  from either, and a `Picture` may be cut to a `CurvePolygon` — "Add support".
- **The tag renumbering is approved.** An included union's members come first, so two
  new members of `Shape` move `AutoPaint`, `Picture` and `Group`.
- **No per-curve paint.** A `BezierCurve` is points and weights; the shape's one
  `paint` draws all of it. A second stroke is a second item.
- **plan-164 waits on plan-172** (plan-172-A Open Decision 4, 2026-10-10).

## 1. Goal

**The public types:**

```
canvas::BezierCurve     ' one segment. A record, NOT a DrawItem
  points   AS List OF vector::Float2  ' degree = len(points) - 1; 2..17 points (degree 1..16);
                                      '   offsets from the shape's position
  weights  AS List OF Float           ' empty = polynomial; else one per point, each > 0 and finite

canvas::PolyCurve       ' a Shape: the curved PolyLine. Stroke only; it has no interior
  id        AS String
  position  AS vector::Float2         ' the reference point; the transform pivots about it
  curves    AS List OF canvas::BezierCurve    ' joined end to end, in order
  paint     AS canvas::Paint
  transform AS canvas::Transform

canvas::CurvePolygon    ' a Shape: the curved Polygon. Filled under rule
  id        AS String
  position  AS vector::Float2
  contours  AS List OF List OF canvas::BezierCurve   ' each contour is closed
  rule      AS canvas::FillRule
  paint     AS canvas::Paint
  transform AS canvas::Transform
```

The field order is the shapes': `id_prop()` first, `position` next, the geometry, then
`paint_prop()` and `transform_prop()` last. `CurvePolygon.contours` and `.rule` are
`Polygon`'s fields with curves where it has points, on purpose: a user who knows one
knows the other.

**Placement** is the one rule plan-172 has. A point `p` of a curve reaches the surface
at `position + transform(p)`, then through every enclosing group
(`planning/new_canvas.md`, "Transform order, spelled out"). The all-zero `Transform` is
the identity, and so is one with no area. `paint.clip` and `paint.gradient` are in the
same local space as the curve points.

**Validity** (a curve that breaks a rule is **absent**: it draws nothing and
contributes no edge):

- 2..17 points;
- `weights` either empty or exactly `len(points)` long, with every weight finite and
  > 0.

**Joining.** Curves are joined in list order. If a curve's first point differs from the
previous valid curve's last point, an implied straight segment joins them, and it is
part of the shape: stroked in a `PolyCurve`, an edge of the outline in a `CurvePolygon`.
A `CurvePolygon` contour is also closed by an implied straight segment from its last
point to its first when they differ, as a `Polygon`'s ring is. A `PolyCurve` is never
closed: one that ends where it began has two caps there, not a join. A chain or contour
with no valid curve is absent.

**The public functions:**

- `canvas::nurbsToBeziers(controlPoints AS List OF vector::Float2, weights AS List OF
  Float, knots AS List OF Float, degree AS Integer) AS List OF canvas::BezierCurve`.
  It converts exactly by knot insertion (Boehm): each interior knot is raised to full
  multiplicity, and each span becomes one rational Bézier of the same degree. The
  result is a chain: it is a `PolyCurve`'s `curves` or one `CurvePolygon` contour as it
  stands. An invalid input (knot count ≠ points + degree + 1, a decreasing knot vector,
  a weight ≤ 0, or a degree outside 1..16) returns `[]`. It does not trap.
- `canvas::flatten(curve AS canvas::BezierCurve, tolerance AS Float) AS List OF
  vector::Float2`. It returns points along the curve whose polyline deviates from the
  curve by at most `tolerance` (clamped to ≥ 0.001), including both endpoints. It is
  for building a `Polygon` or `PolyLine` yourself, or for geometry of your own; the
  points are in the curve's own space, ready to be a `Polygon`'s contour at the same
  `position`. An invalid curve returns `[]`.

Neither is a constructor in the sense the owner declined for the shapes (plan-172-A
Open Decision 1): each computes something a program could not reasonably write itself.

### Non-goals (explicit constraints)

- **No NURBS drawing item.** NURBS arrive through `nurbsToBeziers`, which is exact, so
  nothing drawable is lost.
- **No drawing in this letter.** Each shape builds the empty geometry header and draws
  nothing until B. For the same reason it is hit by nothing until B: a worker record
  with no region is the honest answer for an item with no pixels. An `AutoPaint` over
  one, and a `Picture` on one, therefore draw nothing until B either.
- **No change to any existing item's behavior.** Three variants' tag numbers change;
  nothing a program can observe does.
- **No per-point `colors` on a `PolyCurve`.** `PolyLine` has them; a curve chain's
  natural parameter is arc length, not vertices, and a gradient already runs along it.
- **No per-curve paint.** One shape, one `paint` (the owner's decision, above).

## 2. Current State

- `Shape` has 8 members: `Rectangle`, `Line`, `Polygon`, `Circle`, `Arc`, `Text`,
  `Ellipse`, `PolyLine` (`shape_variant_set_is_frozen`). `AutoShape INCLUDES Shape` and
  adds `AutoPaint`; `DrawItem INCLUDES AutoShape` and adds `Picture` and `Group`. An
  included union's members come first, in its own order, so the three unions agree on
  every tag they share: today `AutoPaint` 8, `Picture` 9, `Group` 10 (the plan-172-K
  comment above the three `add_union` calls in `mod.rs`).
- Tags are positions, and native code looks them up **by name** through the type model
  (`func_geo_build.rs:variant`). The plan-172-K doc comment on
  `draw_item_variant_set_is_frozen` records that every tag moved once already and that
  "nothing may depend on a tag's number".
- The geometry kind numbers are 3 ARC, 4 POLYGON, 5 NONE, 6 TEXT, 7 ELLIPSE,
  8 GROUP, 9 PICTURE and 10 POLYLINE (`grep -n "LET __CANVAS_GEO_[A-Z]* AS Integer =
  [0-9]*$" src/codegen/builtins/canvas/helper_geometry.rs`, 2026-10-11). 11 and 12 are
  unused. The kind number and the variant tag are unrelated.
- `helper_geometry.rs:__canvas_geometryFor` returns **one** record offset per item.
- The frame is resolved in three passes (`helper_render.rs:__canvas_sceneOffsets`):
  native `canvas::sceneResolve` builds what it can and leaves the rest at -1; the
  MFBASIC walk `__canvas_appendDraw` builds those; native `canvas::sceneLayout` lays the
  entries out. Both curve shapes are among "the rest" (§3).
- The native item hash folds records structurally (`func_item_hash.rs`), recursing into
  nested lists with `fold_list`. A `CurvePolygon` nests a list of lists of
  `BezierCurve`, each holding a list of `vector::Float2` and a list of `Float`, so the
  recursion must handle records inside lists inside lists. That is UNVERIFIED and gets a
  task. `Polygon.contours` (a list of lists of `vector::Float2`) is the nearest
  precedent, and it stops one level short.
- The native geometry builder (`func_geo_build.rs:emit_paint_supported`) refuses what
  it cannot build, and the item falls back to the MFBASIC builder. Both shapes are
  MFBASIC-built in plan-164 (see §3).
- The worker has its own walk and its own geometry cache for hit-testing
  (`helper_hit.rs:__canvas_hitAppend`, `__canvas_hitGeometry`). A shape falls through
  its `CASE ELSE`.
- An `AutoPaint` draws its `shape` under one of five paints (`helper_autopaint.rs`), and
  a `Picture` is cut to its `shape` (`helper_picture.rs`). Each has code that switches
  on which shape it holds; being a `Shape` means being a case in it.

### Measured populations

| What | Count | Command |
|---|---|---|
| `Shape` / `DrawItem` members today | 8 / 11 | the lists `shape_variant_set_is_frozen` and `draw_item_variant_set_is_frozen` pin (2026-10-11) |
| Existing sites constructing either shape | 0 | `grep -rn "canvas::PolyCurve\[\|canvas::CurvePolygon\[" --exclude-dir=target --exclude-dir=.git --exclude-dir=.claude . \| wc -l` |
| Canvas records / unions / enums / functions | 20 / 3 / 13 / 95 | `grep -c 'pkg.add_record(RegistryRecord {'`, `'pkg.add_union(RegistryUnion {'`, `'pkg.add_enum('` on `src/codegen/builtins/canvas/mod.rs`; `grep -c 'pkg.add_function' src/codegen/builtins/canvas/*.rs \| awk -F: '{s+=$2} END {print s}'` (2026-10-11) |
| MFBASIC `MATCH` arms naming `PolyLine` / `Polygon` | 23, in 4 files | `grep -c 'CASE PolyLine(\|CASE Polygon(' src/codegen/builtins/canvas/*.rs` before the first edit: `helper_geometry.rs` 8, `helper_picture.rs` 9, `helper_autopaint.rs` 4, `helper_hit.rs` 2. The per-site answer is in Phase 1, task 4 |
| Fixtures the letter moves | 2 fixtures, 8 goldens | `bash scripts/artifact-gate.sh target/release/mfb all` before regenerating anything → `1684 tests, 1863 build(s), 2350 golden(s) checked, 8 diff(s)`: `syntax/app/app-mouse-surface` (`.ir`, `.nir`, `.nplan`, four `.ncodesum`) and `syntax/resources/loop-body-resource-reference-valid` (`.ir`). They are the two fixtures that `IMPORT canvas` and build: `grep -rl 'IMPORT canvas' tests/syntax tests/rt-behavior tests/byte-identity` names 9, and the other 7 are programs the compiler rejects, whose only golden is a `build.log`. See Corrections 1 |

### Verified properties

- **Tags are read by name, so moving three of them is legal.** Verified by reading
  `func_geo_build.rs:variant` and the plan-172-K comments in `mod.rs` cited above.
- **Neither shape needs an exemption in `every_draw_item_variant_carries_a_paint`.**
  Verified by reading the test: a variant in neither `CONTAINERS` nor `DECORATORS` must
  declare `paint AS Paint`, and both do. Its `DECORATORS` check — that every member of
  `Shape` has a paint to give — then covers them with no edit.
- **`auto_shape_variant_set_is_frozen` needs no edit.** Verified by reading it: it
  asserts `AutoShape` is `Shape`'s members followed by `AutoPaint`, and that `DrawItem`
  begins with `AutoShape`'s. It is derived from `Shape`, so it follows.
- UNVERIFIED: that `func_item_hash.rs:fold_list` handles a list of lists whose element
  is a record that itself holds lists. There is a task for it in Phase 1.
- UNVERIFIED: that no golden or fixture outside the artifact gate records a tag number.
  Phase 1 measures it; see "Gate" below.

## 3. Design Overview

- **Where the shapes go in the unions.** Both are appended to `Shape`'s `variants`,
  after `PolyLine`: `PolyCurve` takes tag 8 and `CurvePolygon` tag 9. `AutoPaint` moves
  to 10, `Picture` to 11 and `Group` to 12 (and `ParticleSystem`, if plan-150 has
  landed, to 13). The owner approved the renumbering. `PolyCurve` precedes
  `CurvePolygon` to match the order their straight counterparts were added in, and for
  no other reason.
- **One piece type for all three renderers: the y-monotone rational quadratic**
  `P(t) = (w0·P0·(1−t)² + 2·w1·P1·t(1−t) + w2·P2·t²) / (w0(1−t)² + 2w1·t(1−t) +
  w2·t²)`, normalised to `w0 = w2 = 1` (a reparametrisation that keeps the shape:
  `w1' = w1/√(w0·w2)`).
  - Polynomial pieces have `w1 = 1`.
  - A line is `P1 = (P0+P2)/2`, `w1 = 1`. Every solver handles the degenerate leading
    coefficient.
  - **y-monotone** means `y` is monotone over `t ∈ [0,1]`. That makes the
    inside/outside test the same half-open endpoint rule polygons use,
    `(y0 > py) <> (y2 > py)`, with the crossing's `x` from one quadratic root.
- **Pieces are in the shape's local space** — the space its curve points are in, with
  `position` at the origin. The record is built there and the renderers carry a query
  point into it through the inverse of `position + transform`, then through the inverse
  of the enclosing groups' affine, exactly as they do for a `Polygon`. y-monotone
  therefore means monotone in **local** y, which is the axis the fill test runs along.
- **Two geometry kinds, one tail.** `__CANVAS_GEO_POLYCURVE = 11` and
  `__CANVAS_GEO_CURVEPOLYGON = 12`, as `POLYLINE` and `POLYGON` are two kinds. Both
  records carry the same piece tail (letter B §3); the kind says whether there is an
  interior.
- **The reduction** (`__canvas_reduceCurve`, MFBASIC, f64), run at geometry build:
  - A degree-1 curve gives one line piece.
  - A degree-2 curve (polynomial or rational) is exact. It is split at its
    y-extremum(s), found by solving the derivative of `y(t)` in homogeneous form, which
    is closed form.
  - A degree-3 or higher curve (polynomial or rational) is split adaptively by de
    Casteljau in homogeneous coordinates. Each candidate span is fitted with a
    polynomial quadratic through both endpoints, with its control point at the
    intersection of the endpoint tangents (or the midpoint of the chord when the
    tangents are parallel). The span is accepted when the max distance between the
    span and the fit, sampled at 16 evenly spaced `t`, is at most `ε`; otherwise it is
    halved. There is a depth cap of 16, at which the fit is accepted as is. Accepted
    pieces are then split at their y-extrema.
  - **`ε` = 0.05 surface pixels**, converted to local space by dividing by the larger
    singular value of the shape's own `transform` (the identity, and a transform not in
    effect per `__canvas_transformActive`, give 0.05). Both are computed at build time.
  - **A group's transform is not in `ε`.** A group's blocks are recorded once and shared
    by every use of the group (`.ai/canvas-threading.md` §10), so the geometry cannot
    know how far a group will magnify it. A curve shape inside a group scaled by `k` has
    a reduction error of up to `0.05·k` surface pixels. Polynomial and rational
    quadratics are unaffected (they are exact); cubics and above are. To zoom a curve
    and keep it within `ε`, scale the shape's own `transform`, which changes its hash
    and rebuilds it. This is stated in the manual (letter C).
- **Why the reduction is on the CPU and not the GPU.**
  - Degree ≥ 3 distance has no closed form, and every renderer would need per-pixel
    numeric root finding at up to degree 16.
  - Reducing once to a single low-degree piece type keeps the GPU work bounded and
    identical in structure across the three renderers.
  - The pieces are curves, not lines, so a typical cubic becomes about 2–4 pieces
    instead of dozens of edges, and the edges stay smooth at any zoom within `ε`.
- **The native builder builds neither shape.** They are MFBASIC-only:
  `canvas::sceneResolve` leaves them at -1, via `emit_paint_supported` returning
  "unsupported" for the two variants, and `__canvas_appendDraw` builds them. A native
  twin is a later optimisation, and it would need its own bit-identity test.

**Correctness risk concentrates in** the reduction's y-extremum splits: a missed split
breaks the fill test. Phase 1 tests every piece's monotonicity directly.

**Design uncertainty concentrates in** whether 16 samples and `ε = 0.05` hold the
true max deviation. Phase 1 measures the true deviation densely (4096 samples) on
adversarial curves (cusps, loops, near-degenerate weights), and records the numbers.

**Gate:** behavioral for the new code; **and a renumbering that is expected to move
emitted code**. A program that constructs or matches an `AutoPaint`, a `Picture` or a
`Group` holds that variant's tag as a constant, and the constant changes by two. So:

- Expected diffs: `tests/syntax/app/app-mouse-surface/golden/*` (the union and type
  surface), the two frozen-variant lists, and the `.ncode` / `.ncodesum` of every
  program that names one of the three moved variants.
- Phase 1 runs `scripts/artifact-gate.sh` **before** regenerating anything, lists the
  fixtures that moved, and inspects ONE of them to confirm the diff is the tag constant
  and nothing else. A fixture that moves and names none of the three is a bug in this
  letter, not part of the renumbering (`AGENTS.md`, "Unexpected diff in a golden").
- The proof for editing the frozen lists and for re-baselining those goldens is this
  plan and the owner's approval of the renumbering (2026-10-11). Re-baseline only the
  fixtures the inspection covers; run the full suite first.

**Rejected alternatives:**

- *One item holding both open and closed runs of curves, with a `closed` flag on
  each.* The package already splits "a line of points" from "a filled region of
  points", and curves follow the same split. One item would also need a rule for what
  an open, unfilled run encloses before it could be a `Shape`.
- *Appending the shapes to `DrawItem`'s own members instead of `Shape`'s.* It moves no
  tag, and it makes them things no `AutoPaint` can wrap, no `Picture` can be cut to and
  no particle system can emit from. The owner approved the renumbering instead.
- *NURBS as the drawing primitive.* Covered exactly by `nurbsToBeziers`, with a simpler
  renderer.
- *Flattening to straight lines on the CPU.* It needs many more edges, and the curves
  go faceted when zoomed. The user asked for GPU work.
- *Per-pixel evaluation of every degree on the GPU.* That would mean numeric roots up
  to degree 16 per pixel in three renderers.
- *Curve points in surface coordinates.* Every shape's geometry is local to its
  `position`, so that one `Transform` pivots about the reference point and a shape is
  moved by changing one field.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as
> the work. Use `- [~]` for partial work, with what remains. Mark a moot task
> `- [x] ~~text~~ — moot: <evidence>`. Fill `Commit:` when a phase lands. **An unticked
> box means NOT DONE.**

### Phase 1 — Types, the renumbering, validation, reducer

- [x] `mod.rs`: add the `BezierCurve` record, and the `PolyCurve` and `CurvePolygon`
      records with `id_prop()`, `position`, `paint_prop()` and `transform_prop()` in the
      shapes' order. Append both to `Shape`'s `variants`, `PolyCurve` first. Write
      man-standard descriptions of validity, joining, the implied closing segment, and
      that curve points are offsets from `position`. Update the plan-172-K comment
      above the unions with the new tag list.
      — `cargo test --release --bin mfb canvas` → `128 passed; 0 failed`, which
      includes `every_draw_item_variant_has_a_record`.
- [x] Update `shape_variant_set_is_frozen` (append the two names) and
      `draw_item_variant_set_is_frozen` (insert them after `PolyLine`, where the
      included union puts them), and each doc comment: the order moves here, as it
      did once in plan-172-K, with the owner's approval and the reason. Neither assertion becomes
      laxer. `auto_shape_variant_set_is_frozen` and
      `every_draw_item_variant_carries_a_paint` must pass **unedited**.
      — both lists are still exact and ordered. Of the two tests that had to pass
      unedited, one word of `auto_shape_variant_set_is_frozen`'s doc comment changed
      ("`Shape`'s eight" → "`Shape`'s members") and no line of either body. All four
      pass in the 128 above.
- [x] Run `scripts/artifact-gate.sh` and record which fixtures moved; inspect one, per
      "Gate" above. Record the count in "Measured populations".
      — `bash scripts/artifact-gate.sh target/release/mfb all` → `1684 tests, 1863
      build(s), 2350 golden(s) checked, 8 diff(s)`, in 2 fixtures. Inspected:
      `loop_body_resource_reference_valid.ir` (Corrections 1).
- [x] Give every MFBASIC `MATCH` over `DrawItem`, `AutoShape` or `Shape` that has no
      `CASE ELSE` the two arms it needs, each drawing nothing for now. The compiler's
      exhaustiveness check finds them. Record the `CASE PolyLine(` / `CASE Polygon(`
      count in "Measured populations" and say, per site, whether it got an arm.
      — ten `MATCH`es have no `CASE ELSE` and got both arms; a canvas program that
      names both shapes builds (`mfb build --app --debug` → "Wrote executable"), which
      is the exhaustiveness check passing:

      | Site | Arms |
      |---|---|
      | `helper_geometry.rs:__canvas_clipOf` | the shape's own `paint.clip`, placed |
      | `helper_geometry.rs:__canvas_geometryFor` | the empty header |
      | `helper_geometry.rs:__canvas_hashItem` | the fields folded (`__canvas_hashCurves`) |
      | `helper_geometry.rs:__canvas_geoReference` (`--debug`) | the empty header |
      | `helper_autopaint.rs:__canvas_autoItem` | the shape as a `DrawItem` |
      | `helper_autopaint.rs:__canvas_autoItemWith` | the shape with the state's paint |
      | `helper_hit.rs:__canvas_itemId` | its `id` |
      | `helper_picture.rs:__canvas_shapeAsItem` | the shape as a `DrawItem` |
      | `helper_picture.rs:__canvas_pictureShapeId` (two nested `MATCH`es) | its `id` |

      The other `CASE PolyLine(` / `CASE Polygon(` sites are in `MATCH`es that end in
      `CASE ELSE`, and their `ELSE` is the right answer for a shape that draws nothing:
      `helper_picture.rs:__canvas_pictureImage` and `__canvas_pictureOutlineItem`
      (nothing to cut an image to), `__canvas_pictureClosed` (FALSE), and
      `helper_hit.rs:__canvas_hitGeometry` (the record `__canvas_geometryFor` builds).
      plan-164-B revisits each when the shapes gain a record.
- [x] `func_item_hash.rs`: confirm, or add, recursion for a record inside a list
      inside a list. Test that two `CurvePolygon`s differing only in one deep weight
      hash differently, and that two differing only in `position` do.
      — confirmed, no recursion added (Corrections 11); the two names joined `HASHED`.
      `canvas_curve_shapes_hash_every_point_and_weight` → `generations=1,2,3,3` for
      the `CurvePolygon` (one weight of the second curve of the second contour; then
      `position`; then the same item again) and `4,5,5` for a `PolyCurve`.
- [x] `helper_geometry.rs`:
      - `__CANVAS_GEO_POLYCURVE = 11` and `__CANVAS_GEO_CURVEPOLYGON = 12`, with Rust
        mirrors (beside `GEO_KIND_POLYLINE` in `src/codegen/runtime/canvas/mod.rs`) and
        pin tests;
      - arms for both in `__canvas_geometryFor`, `__canvas_hashItem` and
        `__canvas_geoReference` that build the empty header (nothing is drawn until B);
      - `func_geo_build.rs:emit_paint_supported` refuses both, and a frame holding one
        is not laid out by `canvas::sceneDrawsFlat` as if it were built.
      — the pins are two rows of
      `the_geo_layout_constants_match_their_rust_counterparts` (in the 128). The
      native builder needed no edit to refuse them: it builds only the five kinds in
      `func_geo_build.rs:Kind::ALL`, and every other variant already falls to the
      MFBASIC builder. A frame of both shapes under `MFB_CANVAS_GEO_VERIFY=1` reports
      `geoNative=0 geoVerifyMismatches=0 drawsVerified=1 geoResolvedChecked=2`.
- [x] `helper_hit.rs`: confirm each shape on the worker builds the empty header too and
      is hit by nothing — it does not block what is beneath it until B gives it a region.
      — `canvas_curve_shapes_draw_nothing_and_block_nothing_yet`: a press on a square
      under a `CurvePolygon`, a 40 px `PolyCurve`, an `AutoPaint` and a `Picture`
      reports `overID = "under"`.
- [x] `helper_autopaint.rs` and `helper_picture.rs`: an `AutoPaint` over either shape,
      and a `Picture` on either, compile and draw nothing. `__canvas_pictureClosed`
      answers FALSE for both until B.
      — the same test: the frame holds the square's red and the background and no
      other colour, `geoVerifyMismatches=0`. `__canvas_pictureClosed` was not edited:
      its `CASE ELSE` is the FALSE.
- [x] `__canvas_reduceCurve(curve, eps) AS List OF Float`: pieces as `x0,y0, x1,y1,
      x2,y2, w1`, per §3, plus `__canvas_curveValid`, and the chain walk that inserts
      the implied join and closing segments.
      — `src/codegen/builtins/canvas/helper_curves.rs`: `CURVE_MATH`
      (`__canvas_cvReduce` and what it is made of) and `CURVE_CHAIN`
      (`__canvas_curveValid`, `__canvas_reduceCurve`, `__canvas_curveEps`,
      `__canvas_curveChain`). Corrections 3–7 and 9.
- [x] Tests (a new `tests/canvas/rt_canvas_curves.rs`, run through small `--app`-free
      MFBASIC harness programs that print the reduced pieces):
      - a quadratic reduces to itself, plus its y-extremum split;
      - a rational quadratic quarter-circle (`w1 = √2/2`) stays exact: every point of
        every piece is within 1e-12 of radius r;
      - a cubic S-curve's dense max deviation is ≤ ε (record the measured value);
      - the same cubic under a `transform` scaled by 4 has a dense max deviation ≤ ε/4
        in local space;
      - a degree-16 curve reduces under the depth cap without NaN;
      - every piece of every test curve is y-monotone (checked on 256 samples each);
      - an invalid curve gives no pieces, and the chain joins its neighbours with one
        line piece;
      - a contour whose last point differs from its first gains one closing line piece,
        and a chain does not.
      — `cargo test --release --test rt_canvas_curves` → `12 passed; 0 failed` (10
      harness tests and the two scene tests above). Dense max deviation, in pixels,
      4096 samples of the curve against the pieces and 256 of each piece against the
      curve, `eps = 0.05`:

      | Curve | Pieces | Max deviation |
      |---|---|---|
      | cubic S, 300 px | 10 | 0.024291 |
      | the same at `eps / 4` (a ×4 `transform`) | 14 | 0.009052 (limit 0.0125) |
      | cusp | 8 | 0.032721 |
      | loop | 12 | 0.023814 |
      | cubic that doubles back along its chord | 1 | 0.000000 |
      | rational cubic, weights 1, 3, 0.25, 1 | 11 | 0.035216 |
      | rational cubic, weights 1, 0.01, 100, 1 | 13 | 0.044669 |
      | four control points on two (a straight line) | 1 | 0.000000 |
      | quintic wave | 18 | 0.038935 |
      | degree 16, seventeen points | 14 | 0.017553 |

      The worst is 0.0447 of 0.05, so 16 samples and `eps = 0.05` hold on everything
      measured, with the least margin on weights four orders of magnitude apart. "Without
      NaN" is the driver exiting cleanly (Corrections 2). The 120° arc over the top of a
      circle is the rational curve that IS cut at a y-extremum; both halves stay at
      radius 100 to 1e-12.
- [x] Regenerate `tests/syntax/app/app-mouse-surface/golden/*` (expected). Inspect the
      `.ir` diff: it should be the added types, the two variants and the moved tags.
      — regenerated with Phase 2's functions in, so the two fixtures are re-baselined
      once: `mfb build -q -ast -ir` for the two `.ir` files and
      `scripts/regen-native-goldens.sh target/release/mfb
      tests/syntax/app/app-mouse-surface` → `4 build(s), 6 golden(s) rewritten, 0
      failure(s)`. The `.ir` inspection is Corrections 1 (line fields, the new
      functions and types, the new `MATCH` arms; variants are matched by name there, so
      no tag appears). Before re-baselining, every other canvas suite was run on the
      Phase 1 compiler (Corrections 15): 20 suites, all `ok`.

Acceptance: the reduction meets its tolerance and exactness claims; both shapes are
`Shape`s that every walk accepts; and every existing scene draws what it drew.
  Check: `cargo test --test rt_canvas_curves` → pass with the recorded max deviations
  ≤ 0.05 (est. 5 min, guess); `cargo test --bin mfb canvas` → pass (est. 3 min, guess);
  `cargo test --test rt_canvas_rasteriser --test rt_canvas_hit --test rt_canvas_autopaint
  --test rt_canvas_picture` → pass with the image goldens unmodified, proving the
  renumbering moved no pixel (est. 10 min, guess).
  Result (2026-10-11): `rt_canvas_curves` 12 passed; `--bin mfb canvas` 128 passed;
  `rt_canvas_rasteriser` 106 passed (2 ignored), `rt_canvas_hit` 13, `rt_canvas_autopaint`
  18, `rt_canvas_picture` 16. No file under `tests/canvas/` is modified but the new
  `rt_canvas_curves.rs`.
Commit: cb658a67d

### Phase 2 — `canvas::nurbsToBeziers` and `canvas::flatten`

- [x] `func_nurbs_to_beziers.rs`: a registry function with its descriptor, `intro`,
      `desc` and a runnable `example`. The body is MFBASIC Boehm knot insertion, with
      the §1 validation. Its points are `vector::Float2`.
      — the insertion itself is `helper_curves.rs:__canvas_cvKnotPoints` and
      `__canvas_cvRefine` (every knot of the domain raised to multiplicity `degree`);
      the body validates, refines and cuts the control points into runs of
      `degree + 1`. `bash scripts/man-run-examples.sh canvas --run` →
      `canvas::nurbsToBeziers example 1 — ran`.
- [x] `func_flatten.rs`: a registry function with its descriptor and a runnable
      `example`. The body is adaptive de Casteljau with a chord-distance test against
      `tolerance`.
      — `helper_curves.rs:__canvas_cvFlatten`: a span is flat when every inner control
      point is within `tolerance` of the chord SEGMENT, which bounds the curve by the
      hull property. `canvas::flatten example 1 — ran`.
- [x] Tests:
      - a NURBS unit circle (9 points, degree 2, the standard knot vector) converts to
        4 rational quadratics whose points are within 1e-12 of radius 1, each ending
        where the next begins, so the result is a contour with no implied segment;
      - a uniform cubic B-spline converts to Béziers that agree with direct B-spline
        evaluation at 256 parameters within 1e-12;
      - each invalid NURBS input gives `[]`;
      - `flatten` on a cubic keeps every chord within `tolerance` on dense samples;
      - `flatten` includes both endpoints;
      - `flatten`'s points, used as a `Polygon` contour at the same `position`, build.
      — `cargo test --release --test rt_canvas_curves` → the five tests named
      `canvas_nurbs_*` and `canvas_flatten_*` pass. The B-spline test also converts a
      rational cubic on a clamped vector with a single and a doubled interior knot (3
      spans). Eleven invalid inputs each return `[]`. Flatten, 4096 samples of the
      curve against the chords: tolerance 2.0 → 15 points, 0.622 px; 0.1 → 57 points,
      0.058 px; 0.001 → 461 points, 0.0007 px; a quarter circle at 0.05 → 63 points,
      0.025 px. A tolerance of 0.0 and of -5.0 return the 0.001 points exactly.
- [x] Function fixtures (added: `.ai/compiler.md` asks for an invalid fixture for every
      new function): `tests/syntax/canvas/func_canvas_nurbsToBeziers_invalid` and
      `func_canvas_flatten_invalid` — wrong argument count, wrong argument types, and
      a result bound to the wrong type are each refused. The valid half is the runtime
      tests above; a canvas function cannot run in a fixture, which has no app mode.

Acceptance: both functions are exact or within tolerance, as stated.
  Check: `cargo test --test rt_canvas_curves` → pass (est. 5 min, guess);
  `bash scripts/man-run-examples.sh canvas --run` → pass (est. 3 min, guess).
  Result (2026-10-11): the five Phase 2 tests pass (`5 passed; 0 failed; 12 filtered
  out`); `examples: 37   built: 37   ran: 37   not run: 0   failed: 0`.
Commit: —

## Validation Plan

- Tests: Phases 1–2, with negative cases (invalid curves, invalid NURBS, the depth cap,
  degenerate weights).
- Coverage check: the reducer runs in `rt_canvas_curves`, and the geometry arms run in
  the rasteriser suite's build of a scene containing each shape (it draws nothing).
- Runtime proof: a harness program prints the reduced pieces of a circle, a cubic S and
  a degree-7 curve. Plot them with `tools/`-free inspection of the printed coordinates
  against the analytic curve (the measured max deviation is recorded here).
- Doc sync: man prose on the new types and functions; `src/docs/spec/app/06_canvas.md`'s
  "The item unions" gains both shapes and the new tag list in this letter, because the
  unions are an observable contract (the Hard Completion Gate in
  `.ai/specifications.md`). The rest of the spec follows in C.
- Final gate: run once, at the end of plan-164-C (the whole plan).

## Open Decisions

1. **Ordering against plan-150.** If plan-150 lands first, this letter also moves
   `ParticleSystem`'s tag, and must add both shapes to plan-150-A's two tables: as
   emitters (a `PolyCurve` emits uniformly along its length, a `CurvePolygon` uniformly
   over its interior, by the bounded rejection a `Polygon` uses) and as templates (each
   is rewritten like any shape). If this plan lands first, plan-150-A does that when it
   starts: its "eight shapes" become ten, and its prerequisite row that greps for another
   plan's appended variant must grep for these names. Recommended: whichever lands second does
   the edit, and the row in Prerequisites above is what detects which that is.

The curve design's own choices are recorded in §3.

## Corrections

1. **The gate's prediction named the wrong population** (§3 "Gate", and the
   "Programs whose emitted code holds a moved tag" row). It said a fixture moves when
   it names `AutoPaint`, `Picture` or `Group`, and that "a fixture that moves and names
   none of the three is a bug". Measured: every program that `IMPORT`s `canvas` carries
   the whole package's source, so its `.ir` holds every canvas helper and moves when a
   helper gains an arm or the package gains a function, whichever variants the program
   names. The gate before any regeneration reported
   `1684 tests, 1863 build(s), 2350 golden(s) checked, 8 diff(s)` in two fixtures:
   `syntax/app/app-mouse-surface` (7) and
   `syntax/resources/loop-body-resource-reference-valid` (1, its `.ir`). The second
   names none of the three and is not a bug: `diff` of its `.ir` against the golden is
   the `"line"` field of declarations that moved down, the new curve functions, the
   three new types, and three `MATCH` arms on `canvas.PolyCurve` /
   `canvas.CurvePolygon` — and nothing else. A variant is matched **by name** in the
   `.ir` (`"pattern": … "name": "canvas.PolyCurve"`), so no tag number appears in it at
   all; the moved tags live only in the native dumps. The corrected rule: the letter
   moves exactly the fixtures that build a program importing `canvas`, and each later
   phase that touches a canvas helper moves the same two again.
2. **"Every weight finite" needs no check, and neither do the points.** A `Float`
   cannot hold an infinity or a NaN: `toFloat("inf")` and `toFloat("nan")` raise
   `7-705-0003`, `1e308 * 10.0` raises `7-705-0015`, and `toFloat("1e999")` raises
   `7-705-0010` (probe programs, 2026-10-11). `__canvas_curveValid` tests the counts
   and `w > 0.0`; the finiteness half of the rule is the language's.
3. **The acceptance distance is geometric, not same-parameter** (§3, "sampled at 16
   evenly spaced `t`"). Each of the span's 16 samples is measured to the nearest point
   of the fitted quadratic (`__canvas_cvQuadDistance`), not to the fit's point at the
   same `t`. The same-parameter distance also counts how differently the two curves
   are parametrised, which converges a power of the span length slower and would cut
   every cubic into more pieces than its shape needs.
4. **The fit's control point falls back to the chord's midpoint in two cases, not
   one.** §3 names parallel tangents. The tangents can also meet *behind* an end —
   the span still holds an inflection — and a control point there is on the wrong
   side of the curve. Both take the midpoint (a straight piece), which the error test
   then judges like any other fit.
5. **At the depth cap the better of the fit and the chord is taken**, where §3 said
   "the fit is accepted as is". A fit that is still outside `eps` after 16 halvings
   has near-parallel tangents and a control point that can be far from a span that
   small; the chord cannot be further from the span than the span is long. This is a
   strengthening.
6. **A horizontal end tangent is snapped.** A fitted control point is
   `P0 + s·T0`, which is exactly `y0` when the curve leaves its first end level but
   only *near* `y2` when it arrives at its last end level; one place off reads as a
   turn and is cut into a piece too short to see. `__canvas_cvSpan` writes `y2` there.
   The same reasoning is why each half of a cut at a y-extremum is given the turn's
   own `y` for its control point.
7. **A curve's ends are pinned to the points as written** (`__canvas_cvPin`). A
   rational curve's ends come out of the homogeneous arithmetic as `(w·x) / w`, which
   can be one place off `x`. The fill test compares `y` against exactly those ends, so
   two curves that meet must meet in the same number.
8. **"A typical cubic becomes about 2–4 pieces" is low** (§3). Measured at
   `eps = 0.05`: the 300 px cubic S is 10 pieces, a cusp 8, a loop 12, a degree-16
   zig-zag 14 (`cargo test --test rt_canvas_curves -- --nocapture`). The fit error of a
   cubic by quadratics is about `0.032·|P3 − 3P2 + 3P1 − P0| / n³`, which for the S
   (third difference 721) needs 8 spans before the y-extremum cuts. The design does
   not rest on the count: the pieces are still curves and still far fewer than chords.
9. **The curve chunk is unconditional**, not gated on a constructor as `AutoPaint`'s
   and `Picture`'s are. A constructor gate is decided from constructor expressions
   (`registry/mod.rs:expr_callees`), and every field of the three curve records has a
   default, so `MUT c AS canvas::PolyCurve` followed by `WITH` holds a curve no
   constructor expression made. A gated chunk would draw nothing for that program.
10. **The reducer tests cannot `IMPORT canvas`** (Phase 1, "`--app`-free harness
    programs"). The package is refused outside app mode ("the `canvas` package
    requires app mode"). The harness lifts `CURVE_MATH` and `CURVE_CHAIN` out of
    `helper_curves.rs` as text, as `rt_canvas_auto_step.rs` lifts `AUTO_STEP`, and
    declares the two record types the chain names itself; `CURVE_MATH` is kept free of
    any other canvas name so that this works
    (`canvas_curve_math_names_nothing_outside_itself`). Floats are printed as two
    integers, exact to 2^-60, because `toString` of a `Float` keeps two decimals.
11. **`func_item_hash.rs` needed no new recursion** (§2 UNVERIFIED, Phase 1 task 5).
    `fold_list` already recurses for a list element and calls `fold_record` for a
    record element, and `fold_field` calls `fold_list` for a list field, so a list of
    lists of records holding lists folds as it stands. The change is the two names in
    `HASHED`; `canvas_curve_shapes_hash_every_point_and_weight` is the proof.
12. **The Rust kind mirrors are `#[cfg(test)]`** until plan-164-C's emitters branch on
    them: nothing but `the_geo_layout_constants_match_their_rust_counterparts` reads
    them in this letter, and an unused `pub(crate)` constant is a dead-code warning.
13. **`nurbsToBeziers` has two more empty answers than §1 lists**, both of which
    describe no curve: a `weights` list that is neither empty nor one per point (an
    empty one makes every weight 1, and the curves returned then carry no weights);
    and a knot vector whose `knots[degree]` is not below `knots[len(controlPoints)]`,
    which leaves an empty domain. Fewer than `degree + 1` control points is a third.
14. **The curve the gate tolerance was checked on was not "a degree-7 curve"**
    (Validation Plan, "Runtime proof"). The curves measured are the cubic S, a cusp, a
    loop, a rational cubic with weights four orders of magnitude apart, a quintic wave
    and a degree-16 zig-zag; the numbers are on the Phase 1 test task.
15. **"Run the full suite first" was run as every canvas suite, not `cargo test`**
    (§3 "Gate"). The only programs whose emitted code moved are the ones that import
    `canvas` — the gate shows every other fixture byte-identical, and a byte-identical
    program cannot behave differently. So what was run before re-baselining is every
    test that builds a canvas program: the five suites Phase 1 names, and then
    `rt_canvas_{auto_step,damage,debug_hooks,font,frame_stats,geo_arena,geo_native,
    golden,graphics_thread,group_ownership,image_decode,metal,picture_fit,pointer,
    present_deep_copy,present_leak,system_fonts}` and
    `cli_{app_canvas_mode,canvas_image_resource,canvas_man_examples_compile,
    canvas_package}`, against a frozen copy of the Phase 1 compiler
    (`MFB_TEST_EXE`) — all `ok`. The whole `cargo test` runs once, at the end of
    plan-164-C.

## Summary

This letter is CPU-only: it adds types and conversions, and neither shape draws yet.
The risk in the new code is the reducer's monotonicity and tolerance guarantees, which
are tested directly and measured densely.

The risk in the existing code is the renumbering. Three variants' tags move, which is legal
because every tag is read by name, and which is expected to move the emitted code of
every program that names one of them. The gate is that the pixels do not move, and that
each re-baselined fixture has been shown to differ by a tag and nothing else.
