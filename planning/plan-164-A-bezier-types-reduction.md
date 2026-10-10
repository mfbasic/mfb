# plan-164-A: Bézier curve types, validation, and reduction to quadratic pieces

Last updated: 2026-09-28
Overall Effort: huge (>3d) — the whole plan-164 curve feature, split A–C below
Effort: large (3h–1d)
Depends on: plan-163 (all of A–D) and plan-172 (all of A–N); see Prerequisites

plan-164 gives the general canvas (not just SVG content) true curves: a `canvas::Path`
of subpaths made of `canvas::BezierCurve`s. Supported curves are linear, quadratic,
cubic, rational quadratic, rational cubic, higher-degree polynomial, higher-degree
rational, and composite piecewise splines. They are drawn **on the GPU per pixel**,
not flattened to straight lines. The letters are in implementation order:

| Letter | Delivers | Effort |
|---|---|---|
| **A** (this) | The public types, validation, `canvas::nurbsToBeziers`, `canvas::flatten`, and the CPU **reduction** of every curve to y-monotone rational-quadratic pieces | large |
| B | The software renderer (the oracle) draws `Path`: fill winding, distance, stroke, caps and joins at real corners, dashes along curves, and per-curve paints | large |
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

**This letter's single behavioral outcome:** the types exist and are validated;
`canvas::nurbsToBeziers` converts NURBS exactly; `canvas::flatten` returns points
within tolerance; and the internal reducer turns any valid curve into y-monotone
rational-quadratic pieces whose max deviation from the input curve is within the
reduction tolerance (and zero for quadratic and rational-quadratic input). Drawing is
B and C. In this letter a `Path` **draws nothing** and does not trap.

References:

- plan-163 A–D (`planning/plan-163-*.md`). `Path` reuses `FillRule`, B's
  caps/joins/dashes, C's clip/`transparency`, and D's gradients.
- Precedents in the code today:
  - TrueType quadratic curves are split into straight edges in
    `helper_glyph.rs:__canvas_quadEdges` (:266);
  - the ellipse distance uses a fixed-step bisection
    (`helper_shapes.rs:__canvas_ellipseDistance`, 24 halvings, rationale at :201).
- `src/docs/spec/app/06_canvas.md` (the zero-value rule :67–87; transform :378–399).
- `.ai/man-content.md` (the registry descriptor prose), `.ai/specifications.md`.
- The `DrawItem` union's variant set is frozen by a test (`mod.rs`, near :1482–1509).
  Adding `Path` changes that test deliberately.

## Prerequisites

This gate is the only hard stop for plan-164. Each row names the letters it gates.

| Must be true | Gates | Command | Status |
|---|---|---|---|
| plan-163-A..D all complete and archived | A–C | `ls planning/completed/plan-163-{A,B,C,D}-*` → 4 matches | NOT MET (2026-09-28: none started) |
| plan-172 (the canvas shape model) is complete and archived, every letter A–N | A–C | `ls planning/plan-172-*` → no matches, and `ls planning/completed/plan-172-M-*` → present | NOT MET (2026-10-10: in progress) |

If plan-172 is not complete, this plan cannot start, full stop (plan-172-A Open
Decision 4, answered by the owner 2026-10-10). **This document predates plan-172 and
must be re-derived against it before Phase 1**, as plan-150-A must: `canvas::Path` is
appended to `DrawItem`'s own members beside `Picture` and `Group` and carries its own
`id`, `position` and `transform`; every `canvas::Point` here is `vector::Float2`;
there is no `Paint.transform`; and every `file:line` citation, the `DrawItem` variant
count and the frozen-set test's list are re-measured.

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run
> it before you continue and before you stop, and report it if you stop.

## 1. Goal

**The public types:**

```
canvas::BezierCurve
  points   AS List OF canvas::Point   ' degree = len(points) - 1; 2..17 points (degree 1..16)
  weights  AS List OF Float           ' empty = polynomial; else one per point, each > 0 and finite
  paints   AS List OF canvas::Paint   ' empty = stroked by the Path's paint; else stroked once per
                                      '   paint, in order (stacked strokes); fills always come from
                                      '   the Path's paint

canvas::Subpath
  curves   AS List OF canvas::BezierCurve
  closed   AS Boolean                 ' closes the STROKE; the fill always treats a subpath as closed

canvas::Path            ' a new DrawItem variant, appended LAST to the union (tag 11)
  subpaths AS List OF canvas::Subpath
  rule     AS canvas::FillRule
  paint    AS canvas::Paint
```

**The public functions:**

- `canvas::nurbsToBeziers(controlPoints AS List OF canvas::Point, weights AS List OF
  Float, knots AS List OF Float, degree AS Integer) AS List OF canvas::BezierCurve`.
  It converts exactly by knot insertion (Boehm): each interior knot is raised to full
  multiplicity, and each span becomes one rational Bézier of the same degree. An
  invalid input (knot count ≠ points + degree + 1, a decreasing knot vector, a weight
  ≤ 0, or a degree outside 1..16) returns `[]`. It does not trap.
- `canvas::flatten(curve AS canvas::BezierCurve, tolerance AS Float) AS List OF
  canvas::Point`. It returns points along the curve whose polyline deviates from the
  curve by at most `tolerance` (clamped to ≥ 0.001), including both endpoints. It is
  for hit testing and for building a `Polygon`/`PolyLine` yourself. An invalid curve
  returns `[]`.

**Validity** (a curve that breaks a rule is **absent**: it draws nothing and
contributes no fill edge):

- 2..17 points;
- `weights` either empty or exactly `len(points)` long, with every weight finite and
  > 0.

**Joining between curves.** Within a subpath, if a curve's first point differs from
the previous curve's last point, an implied straight segment joins them. So the fill
outline is always connected, as SVG path commands are. A subpath with no valid curve is
absent.

### Non-goals (explicit constraints)

- **No NURBS drawing item.** NURBS arrive through `nurbsToBeziers`, which is exact, so
  nothing drawable is lost.
- **No drawing in this letter.** A `Path` builds the empty geometry header and draws
  nothing until B.
- **No change to any existing item's behavior.** The union gains a variant, and nothing
  else in it changes.

## 2. Current State

- `DrawItem` has 11 variants (tags 0–10) (`mod.rs:1096–1155`). Tags are declaration
  order, and native code looks them up through the type model
  (`func_geo_build.rs:variant`). Appending `Path` last gives it tag 11 and moves no
  existing tag.
- The geometry kind numbers are 3 ARC, 4 POLYGON, 5 NONE, 6 TEXT, 7 ELLIPSE,
  8 GROUP, 9 PICTURE and 10 POLYLINE (`grep -n "LET __CANVAS_GEO_[A-Z]* AS Integer =
  [0-9]*$" src/codegen/builtins/canvas/helper_geometry.rs`). A new `PATH` kind takes 11.
- `__canvas_geometryFor` (`helper_geometry.rs:1032`) returns **one** record offset per
  item.
- The native item hash folds records structurally, recursing into nested lists after
  plan-163-A's `fold_list` arm. `Path` nests a list of `Subpath`, then a list of
  `BezierCurve`, then lists of `Point`/`Float`/`Paint`, so the recursion must handle
  records inside lists inside records. That arm is UNVERIFIED for the record-in-list
  case and gets a task.
- The native geometry builder (`func_geo_build.rs:emit_paint_supported`) refuses what
  it cannot build, and the item falls back to the MFBASIC builder. `Path` is
  MFBASIC-built in plan-164 (see §3).

### Measured populations

| What | Count | Command |
|---|---|---|
| `DrawItem` variants today | 11 | the `DrawItem` union in `mod.rs` (the frozen-variant test's list) |
| Existing sites constructing a `Path` | 0 | `grep -rn "canvas::Path\[" --exclude-dir=target --exclude-dir=.git . \| wc -l` |

### Verified properties

- **Appending a variant moves no existing tag.** Verified by reading
  `func_geo_build.rs:variant` (the tag comes from the type model, in declaration
  order).
- UNVERIFIED: that `func_item_hash.rs:fold_list` handles a list whose element is a
  record that itself holds lists. There is a task for it in Phase 1.

## 3. Design Overview

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
  - **`ε` = 0.05 surface pixels**, converted to item space by dividing by the
    transform's larger singular value (the identity gives 0.05). Both are computed at
    build time.
- **Why the reduction is on the CPU and not the GPU.**
  - Degree ≥ 3 distance has no closed form, and every renderer would need per-pixel
    numeric root finding at up to degree 16.
  - Reducing once to a single low-degree piece type keeps the GPU work bounded and
    identical in structure across the three renderers.
  - The pieces are curves, not lines, so a typical cubic becomes about 2–4 pieces
    instead of dozens of edges, and the edges stay smooth at any zoom within `ε`.
- **The native builder does not build `Path`.** It is MFBASIC-only, via
  `emit_paint_supported` returning "unsupported" for the `Path` variant. A native twin
  is a later optimisation, and it would need its own bit-identity test.

**Correctness risk concentrates in** the reduction's y-extremum splits: a missed split
breaks the fill test. Phase 1 tests every piece's monotonicity directly.

**Design uncertainty concentrates in** whether 16 samples and `ε = 0.05` hold the
true max deviation. Phase 1 measures the true deviation densely (4096 samples) on
adversarial curves (cusps, loops, near-degenerate weights), and records the numbers.

**Gate:** behavioral (new code, no existing behavior changes). Expected diffs:
`tests/syntax/app/app-mouse-surface/golden/*` (the union and type surface), and the
frozen-variant test's list. Update that list by adding the `Path` line only. Its proof
is this plan.

**Rejected alternatives:**

- *NURBS as the drawing primitive.* Covered exactly by `nurbsToBeziers`, with a simpler
  renderer.
- *Flattening to straight lines on the CPU.* It needs many more edges, and the curves
  go faceted when zoomed. The user asked for GPU work.
- *Per-pixel evaluation of every degree on the GPU.* That would mean numeric roots up
  to degree 16 per pixel in three renderers.
- *One paint per curve (instead of a list).* A list covers none, one, or stacked
  strokes (an outline plus an inner line) in one field. The user suggested the list.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as
> the work. Use `- [~]` for partial work, with what remains. Mark a moot task
> `- [x] ~~text~~ — moot: <evidence>`. Fill `Commit:` when a phase lands. **An unticked
> box means NOT DONE.**

### Phase 1 — Types, validation, reducer

- [ ] `mod.rs`: add the `BezierCurve`, `Subpath` and `Path` records and append `Path`
      **last** to `DrawItem`. Write man-standard descriptions of validity, joining
      between curves, `closed`, and `paints`. Update the frozen-variant test (add
      `Path` only).
- [ ] `func_item_hash.rs`: confirm, or add, recursion for a record inside a list
      inside a record. Test that two `Path`s differing only in one deep weight hash
      differently.
- [ ] `helper_geometry.rs`:
      - `__CANVAS_GEO_PATH = 11`, with a Rust mirror and pin tests;
      - a `CASE Path` arm in `__canvas_geometryFor`, `__canvas_hashItem` and
        `__canvas_geoReference` that builds the empty header (it draws nothing
        until B);
      - `func_geo_build.rs:emit_paint_supported` refuses `Path`.
- [ ] `__canvas_reduceCurve(curve, eps) AS List OF Float`: pieces as `x0,y0, x1,y1,
      x2,y2, w1`, per §3, plus `__canvas_curveValid`.
- [ ] Tests (a new `tests/canvas/rt_canvas_curves.rs`, run through small `--app`-free
      MFBASIC harness programs that print the reduced pieces):
      - a quadratic reduces to itself, plus its y-extremum split;
      - a rational quadratic quarter-circle (`w1 = √2/2`) stays exact: every point of
        every piece is within 1e-12 of radius r;
      - a cubic S-curve's dense max deviation is ≤ ε (record the measured value);
      - a degree-16 curve reduces under the depth cap without NaN;
      - every piece of every test curve is y-monotone (checked on 256 samples each);
      - an invalid curve gives no pieces.
- [ ] Regenerate `tests/syntax/app/app-mouse-surface/golden/*` (expected). Inspect the
      `.ir` diff: it should be the added types and variant only.

Acceptance: the reduction meets its tolerance and exactness claims, and existing items
are unchanged.
  Check: `cargo test --test rt_canvas_curves` → pass with the recorded max deviations
  ≤ 0.05 (est. 5 min, guess); `cargo test --bin mfb canvas` → pass (est. 3 min, guess);
  `cargo test --test rt_canvas_rasteriser` → pass with goldens unmodified, proving
  nothing existing moved (est. 5 min, guess).
Commit: —

### Phase 2 — `canvas::nurbsToBeziers` and `canvas::flatten`

- [ ] `func_nurbs_to_beziers.rs`: a registry function with its descriptor, `intro`,
      `desc` and a runnable `example`. The body is MFBASIC Boehm knot insertion, with
      the §1 validation.
- [ ] `func_flatten.rs`: a registry function with its descriptor and a runnable
      `example`. The body is adaptive de Casteljau with a chord-distance test against
      `tolerance`.
- [ ] Tests:
      - a NURBS unit circle (9 points, degree 2, the standard knot vector) converts to
        4 rational quadratics whose points are within 1e-12 of radius 1;
      - a uniform cubic B-spline converts to Béziers that agree with direct B-spline
        evaluation at 256 parameters within 1e-12;
      - each invalid NURBS input gives `[]`;
      - `flatten` on a cubic keeps every chord within `tolerance` on dense samples;
      - `flatten` includes both endpoints.

Acceptance: both functions are exact or within tolerance, as stated.
  Check: `cargo test --test rt_canvas_curves` → pass (est. 5 min, guess);
  `bash scripts/man-run-examples.sh canvas --run` → pass (est. 3 min, guess).
Commit: —

## Validation Plan

- Tests: Phases 1–2, with negative cases (invalid curves, invalid NURBS, the depth cap,
  degenerate weights).
- Coverage check: the reducer runs in `rt_canvas_curves`, and the geometry arms run in
  the rasteriser suite's build of a scene containing a `Path` (it draws nothing).
- Runtime proof: a harness program prints the reduced pieces of a circle, a cubic S and
  a degree-7 curve. Plot them with `tools/`-free inspection of the printed coordinates
  against the analytic curve (the measured max deviation is recorded here).
- Doc sync: man prose on the new types and functions. The spec follows in C.
- Final gate: run once, at the end of plan-164-C (the whole plan).

## Open Decisions

None. Choices are recorded in §3.

## Corrections

(none yet)

## Summary

This letter is CPU-only and safe to land alone: it adds types and conversions, and a
`Path` draws nothing yet. The risk is the reducer's monotonicity and tolerance
guarantees, which are tested directly and measured densely.
