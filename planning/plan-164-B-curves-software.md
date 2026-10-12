# plan-164-B: the software renderer draws `canvas::PolyCurve` and `canvas::CurvePolygon`

Last updated: 2026-10-11
Effort: large (3h–1d)
Depends on: plan-164-A (see plan-164-A Prerequisites; this letter also needs A complete)

The software renderer is the reference the GPUs are compared against. This letter
teaches it to draw both curve shapes from the y-monotone rational-quadratic pieces
plan-164-A's reducer produces. That covers a `CurvePolygon`'s fill under `rule`, the
stroke of both with caps at a `PolyCurve`'s ends and joins at real corners, dashes along
the curves, and everything plan-163 gave `Paint` (clip, `transparency`, gradients).

Being a `Shape` is more than being drawn, so this letter also delivers the three things
every shape has since plan-172: a **hit region**, an **`AutoPaint`** that can wrap it,
and — for the one that encloses an area — a **`Picture`** that can be cut to it. The
GPUs follow in C.

**The single behavioral outcome:** both shapes draw in software exactly as §3 defines,
proven by hand-computed pixels; `MouseEvent.overID` names them where they are drawn; an
`AutoPaint` over either changes its paint under the pointer; and a `Picture` on a
`CurvePolygon` shows its image inside the curved outline. Every other item stays
byte-identical.

References: plan-164-A §1 and §3 (types, joining, piece format, local space, `ε`);
`planning/new_canvas.md` ("Transform order, spelled out", "Hit-testing", the `Picture`
rules); plan-163-A (the winding count), B (caps, joins, dashes, the join tail, the
arc-length table), C (the clip, the transparency fold) and D (gradients in item space,
`strokeApply`), all under `planning/completed/`; `.ai/canvas-threading.md` §14 ("The
worker has a geometry cache too"), §16 (`AutoPaint`) and §17 ("A `Picture` is one or two
draw entries"); `.ai/testing-gates.md`, "The GPU comparison is against the ORACLE, not a
stored picture" (the oracle needs hand-computed checks).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-164-A complete and archived | `ls planning/completed/plan-164-A-*` → match | NOT MET (2026-10-11) |

> **NOTE — the Status column is a snapshot; the Command column is the truth.**

## 1. Goal

- A `CurvePolygon` fills under `rule` and strokes its closed contours; a `PolyCurve`
  strokes its chain. Both honour their own `position` and `transform`, the enclosing
  groups' offset and matrix, the clip, `transparency` and the gradient.
- Both are hit where §3 says, on the worker, by the same distance code that draws them.
- An `AutoPaint` whose `shape` is either one shows its state paints.
- A `Picture` whose `shape` is a `CurvePolygon` draws its image inside the outline, in
  both `PictureClip` modes and all three `PictureFit`s. A `Picture` on a `PolyCurve`
  draws nothing, as one on a `PolyLine` draws nothing.
- They are drawn only in the software renderer in this letter. On the GPU path, a frame
  containing either falls back to software, through the existing whole-frame
  predicates, until C.

### Non-goals

- GPU drawing (that is C).
- A native geometry builder for either shape (plan-164-A §3).
- Per-curve paints, and per-point colours on a `PolyCurve` (plan-164-A Non-goals).

## 2. Current State

- The software draw loop, `helper_items.rs:__canvas_drawGeometry`, reads one geometry
  record per draw entry and dispatches on kind. `helper_draw.rs:__canvas_geoDistance`
  is the per-kind distance. Each entry is drawn under its composed group offset and
  matrix (`__CANVAS_DRAW_DX` / `_DY` / `_MAT`, plan-172-G); the record itself is in the
  item's local space.
- The fill test is `helper_draw.rs:__canvas_edgeDistanceRule`, a crossing count under
  the rule (plan-163-A). Joins come from `helper_joins.rs:__canvas_joinTail`. Dashes use
  plan-163-B's perimeter parameter and its arc-length tables (`helper_dashes.rs`).
- **One item, one record.** `__canvas_geometryFor` returns one offset, and each curve
  shape is one record and one draw entry, like a `Polygon` or a `PolyLine`.
- The hit test is `helper_hit.rs`: `__canvas_hitAppend` walks the worker's copy of the
  scene, `__canvas_hitGeometry` builds a record in the worker's own cache, and
  `__canvas_hitDistance` asks the same distance code the renderer uses. The region rules
  are `planning/new_canvas.md`'s: a closed shape is hit on its interior plus its stroke
  whether or not it is filled; a stroke-only shape on its stroke, at least 3 pixels
  either side; the region is built from the item's own paint, and that paint's `clip`
  cuts it.
- An `AutoPaint` is its shape drawn under whichever paint its state selects
  (`helper_autopaint.rs:__canvas_autoItem`, `__canvas_autoBlock`): a record per paint
  shown, keyed by the item's hash folded with the state.
- A `Picture` is cut to its shape in `helper_picture.rs`. `__canvas_pictureClosed` says
  whether the shape encloses an area. `__canvas_pictureImage` turns each closed kind
  into an **outline of rings** — a circle or an ellipse is flattened to one
  (`__canvas_ellipseRing`) — and `__canvas_pictureImageOf` builds the image record with
  that outline as its clip, intersected with `paint.clip` when the shape has one
  (`__canvas_clipIntersect`). The shape's stroke is a second record, the shape itself
  (`__canvas_pictureOutlineItem`). On the worker a picture is hit as its shape
  (`__canvas_pictureHitItem`).
- The GPU whole-frame fallback is `__canvas_metalRenderable` /
  `__canvas_vulkanRenderable`, which read the frame's flat offsets list. A predicate
  cannot find a `Group` there (`.ai/canvas-threading.md` §13), but a curve shape is a
  leaf: its record is in the list, inside a group or not, and its kind is the first
  float. UNVERIFIED that each predicate has a per-kind decline to extend; Phase 1 reads
  them and records what it finds.

## 3. Design

- **The record.** A `POLYCURVE` or `CURVEPOLYGON` header, then its **piece tail**: 7
  floats per piece (`x0,y0,x1,y1,x2,y2,w1`), in the shape's local space, in chain order;
  a `CurvePolygon`'s contours follow one another, each ending with its closing piece
  when plan-164-A's joining rule adds one. Then plan-163-B's join tail (corners) and
  dash tail (the arc-length table per piece), then C's clip edges, then the gradient
  stops (always last). A record's tail is read back from its end
  (`.ai/canvas-threading.md`, "A geometry record's tail is read back from its END"):
  read that section before adding a tail region.
  - A header slot `CURVE_PIECE_COUNT` holds the number of pieces.
  - A `CurvePolygon` has a per-contour table in the tail — first piece and piece count —
    so the stroke knows where each closed outline starts over. A `PolyCurve` is one run
    and needs none.
  - Every piece in the tail is both a fill edge and a stroke piece. There is no piece
    that is one and not the other: a contour's closing segment is filled to and
    stroked, and a chain has none.
- **Fill (`CurvePolygon` only).** For each piece, if `(y0 > py) <> (y2 > py)`, solve
  `Y(t) − py·W(t) = 0` for its one root in `[0,1]` (the piece is y-monotone). With
  `w0 = w2 = 1` this is a quadratic in `t`, using a numerically stable form (the
  citardauq variant when `b` dominates). Take `x = X(t)/W(t)`, and when `px < x` add
  +1 if `y2 > y0`, else −1. The point is inside under `rule` exactly as for a polygon.
  `(px, py)` is the query point in local space. A `PolyCurve` has no interior: its
  distance is never negative.
- **Distance.**
  - For a polynomial piece (`w1 = 1`): the closed-form nearest point, by solving the
    cubic `(P(t) − p)·P'(t) = 0` with the trigonometric/Cardano method. Clamp the roots
    to `[0,1]`, and take the min over them and the two endpoints.
  - For a rational piece: a **fixed** search, identical in all three renderers:
    - 8 uniform samples of `|P(t) − p|²`;
    - then 6 halvings of the bracket around the best sample, choosing by comparing
      the squared distance at the two quarter-points;
    - then the min with the endpoints.

    The step counts are fixed, as in the ellipse's 24 halvings. They are recorded here
    after the Phase 1 measurement: raise them if the dense-sample error exceeds 0.01
    px.
  - The distance is the min over pieces; for a `CurvePolygon` it is signed by the fill
    test.
- **Stroke.**
  - Caps (plan-163-B `paint.cap`) go at the two ends of a `PolyCurve`. A `CurvePolygon`
    has none.
  - Joins (plan-163-B) go only at **real corners**: the boundaries between input
    curves, and at either end of an implied join or closing segment, where the unit
    tangents differ by more than 1e-6 rad. Never inside one input curve's pieces: the
    reducer's split points are smooth by construction, and must not be mitered.
  - The join tail entries are built from the true tangents of the input curves at
    those boundaries.
  - A stroke is transformed with its shape and by every enclosing group
    (`planning/new_canvas.md`), so a curve scaled by 2 strokes twice as wide. Nothing
    here is special to curves; the test is that they are not accidentally exempt.
- **Dashes.** plan-163-B's perimeter parameter `s` per piece is a cumulative
  arc-length table: 16 samples per piece, integrated in f64 at build, and interpolated
  by `t`. The nearest point's `t` comes from the distance step. `s` runs the whole
  length of a `PolyCurve`, and restarts at each contour of a `CurvePolygon`, as it does
  at each ring of a `Polygon`. Arc length is local-space length, as it is for every
  other dashed shape.
- **Hit region.** No new rule. A `PolyCurve` is a stroke-only shape: its stroke, at
  least 3 pixels either side. A `CurvePolygon` is a closed shape: its interior under
  `rule` plus its stroke, whether or not its paint fills it. Each region is built from
  the shape's own `paint` — its `strokeWidth` and its `clip`. `__canvas_hitAppend`'s
  `CASE ELSE` already builds one record per item, and `__canvas_hitDistance` gets the
  two arms, calling the same piece fill test and distance the renderer does.
- **`AutoPaint`.** Nothing new in design: the shape is rebuilt under the state's paint,
  as any shape is. The work is the two arms wherever `helper_autopaint.rs` tells shapes
  apart to swap a paint, and the tests that the curve record under a state paint is a
  different record from the one under the default.
- **A `Picture` cut to a `CurvePolygon`.**
  - `__canvas_pictureClosed` answers TRUE for a `CurvePolygon` and FALSE for a
    `PolyCurve`.
  - **The image's clip is the outline, flattened.** A picture's image record clips to
    rings of straight edges, and a circle or ellipse is already handed to it flattened.
    A `CurvePolygon` is handed to it the same way: each contour's reduced pieces are
    flattened into one ring, at a tolerance of `ε` (plan-164-A §3) in the space the ring
    is built in, and the rings go to `__canvas_pictureImageOf` with the shape's `rule`.
    `paint.clip` is then intersected with them by the code that does it for every other
    shape (`__canvas_clipIntersect`); nothing in the intersection knows about curves.
  - **The outline drawn on top is the true curve.** It is the `CurvePolygon` itself as
    a stroke-only record (`__canvas_pictureOutlineItem`), drawn per pixel like any
    other. So the visible edge of a stroked picture is exact, and the flattening is
    only where the image stops beneath it.
  - **The box `fit` fills is the curve's true extent**, not its control points' hull.
    Each piece is y-monotone, so its y-range is its endpoints'; its x-range needs the
    one root of `x'(t)`. The image must not be placed by a box the curve does not reach.
  - Under `ClipOnly` with a transform in effect the outline is sent through the
    transform before the image is fitted, as for every shape; the rings are flattened
    **after** that, so the tolerance is in surface pixels there too.
  - The flattened edges count against the frame's edge budget on the GPU like any
    clip's. Letter C measures what a curved picture costs.
  - On the worker a picture is hit as its shape, so it needs nothing beyond the
    `CurvePolygon`'s own hit region.
- **Bounds.** The union of the pieces' control-point hulls (a valid bound for a
  rational piece with positive weights), padded by the stroke reach (plan-163-B's cap
  and join padding, `helper_joins.rs:__canvas_strokePad`). Bounds are local; the entry's
  offset and matrix carry them, as for every item since plan-172-G. (The hull is a
  bound for drawing; the picture's `fit` box above is the exact extent, because one
  only has to be big enough and the other has to be right.)

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as
> the work. Use `- [~]` for partial work, with what remains. Mark a moot task
> `- [x] ~~text~~ — moot: <evidence>`. Fill `Commit:` when a phase lands. **An unticked
> box means NOT DONE.**

### Phase 1 — Fill and distance

- [ ] Resolve §2's UNVERIFIED row and make a frame containing either shape — at the top
      level or inside a group — take the whole-frame software fallback on the GPU
      backends.
- [ ] The record builders (`helper_geometry.rs`): the piece tail for both kinds and the
      per-contour table for `CURVEPOLYGON`, per §3, in local space.
- [ ] `helper_draw.rs`: the piece fill test, the polynomial distance and the rational
      fixed search, per §3. Dispatch from `__canvas_geoDistance` on both kinds.
- [ ] Measure the rational search's error against a 4096-sample brute-force minimum on
      a quarter circle (`w1 = √2/2`), a thin conic (`w1 = 5`) and a near-parabola
      (`w1 = 1.01`). Record the numbers and the chosen step counts here.
- [ ] Tests (hand-computed, `tests/canvas/rt_canvas_curves.rs`):
      - a filled `CurvePolygon` circle built from 4 rational quadratics has its centre
        pixel covered and the pixel at `r + 2` not covered, and matches
        `canvas::Circle`'s pixels at the same `position` and radius exactly or within 1
        coverage step (record which);
      - the same shape moved by changing `position` alone draws the same pixels, moved;
      - a figure-8 cubic contour under EvenOdd and under NonZero gives the
        loop-crossing pixels by rule;
      - two contours, one inside the other, give the hole under both rules, per
        plan-163-A;
      - a contour whose last point is not its first fills to the implied closing
        segment;
      - a `PolyCurve` over the same curves covers no interior pixel.

Acceptance: `CurvePolygon` fill and antialiasing are correct in software, and a
`PolyCurve` fills nothing.
  Check: `cargo test --test rt_canvas_curves --test rt_canvas_rasteriser` → pass,
  rasteriser goldens unmodified (est. 8 min, guess).
Commit: —

### Phase 2 — Stroke, caps, joins, dashes

- [ ] The stroke along the pieces, caps at a `PolyCurve`'s two ends, and joins at real
      corners only (the join tail built from the input tangents).
- [ ] The dash arc-length table per piece, `s` along a chain and per contour.
- [ ] Tests (hand-computed):
      - a stroked `CurvePolygon` circle of 4 rational quadratics under `join := Miter`
        has no miter spikes at the 4 curve boundaries (they are smooth) and no cap
        anywhere;
      - a `PolyCurve` of a cubic followed by a line meeting at 90° under Miter gives a
        sharp corner;
      - a `PolyCurve` whose second curve starts away from where the first ended strokes
        the implied segment between them, with a join at each end of it;
      - a `PolyCurve` that ends where it began has two caps there and no join;
      - dashes `[10, 5]` on a quarter-circle `PolyCurve` of radius 100 give on-pixels at
        arc lengths [0,10), [15,25), …, and run on unbroken across a curve boundary;
      - a two-contour `CurvePolygon` restarts its dashes at the second contour.

Acceptance: the stroke is correct per §3 in software.
  Check: `cargo test --test rt_canvas_curves --test rt_canvas_rasteriser --test
  rt_canvas_golden` → pass, goldens unmodified (est. 10 min, guess).
Commit: —

### Phase 3 — Placement, paint integration and the hit region

- [ ] Tests (hand-computed), for each shape unless it says otherwise:
      - its own `transform` as a 30° rotation turns it about its `position`;
      - inside a translated group;
      - inside a group that is turned and scaled by 2 — its stroke is twice as wide;
      - clipped by a ring clip (plan-163-C), the ring given in the shape's local space;
      - `transparency := 0.5`;
      - a radial gradient on a `CurvePolygon`'s fill, and on either shape's stroke when
        `gradient.strokeApply` gives it one (plan-163-D).
      Fix whatever fails in the curve arms. These use shared paint code, so a failure
      is a missing arm, not a design question.
- [ ] `helper_hit.rs`: the two arms in `__canvas_hitDistance`, and the worker's record
      for each shape.
- [ ] Tests (`tests/canvas/rt_canvas_hit.rs`):
      - a filled `CurvePolygon` reports its `id` in `overID` inside it and not outside;
      - an unfilled one still reports it inside the outline;
      - a `PolyCurve` of one cubic with a 1-pixel stroke is hit 2 pixels from the curve
        and not 5, and is **not** hit in the area its ends would enclose;
      - either shape with a `clip` is not hit where the clip cuts it away;
      - one inside a named group reports `group::shape`;
      - one with no `id` still blocks the item beneath it.

Acceptance: both shapes honour their placement and every `Paint` field in software, and
the pointer finds each where it is drawn.
  Check: `cargo test --test rt_canvas_curves --test rt_canvas_hit` → pass (est. 6 min,
  guess).
Commit: —

### Phase 4 — `AutoPaint` and `Picture`

The two things a shape can be held by. Last because they sit on top of everything
above, and a failure here is in the holder's code, not the curve's.

- [ ] `helper_autopaint.rs`: the arms for both shapes wherever it switches on the shape
      to swap its paint. Find them with
      `grep -n 'CASE PolyLine(\|CASE Polygon(' src/codegen/builtins/canvas/helper_autopaint.rs`.
- [ ] Tests (`tests/canvas/rt_canvas_autopaint.rs`, with the scripted pointer): an
      `AutoPaint` over a `CurvePolygon` shows `hoverPaint` with the pointer inside the
      curved outline and the default paint with it in the outline's bounding box but
      outside the curve; the same over a `PolyCurve` near and away from its stroke; a
      state paint with a different `strokeWidth` does not change where it is hovered.
- [ ] `helper_picture.rs`: `__canvas_pictureClosed`, the `CurvePolygon` arm in
      `__canvas_pictureImage` (pieces → flattened rings, the exact extent, `rule`), and
      the outline item, per §3.
- [ ] Measure the flattening: for a circle `CurvePolygon`, the largest gap between the
      image's clipped edge and the true curve, in surface pixels, under the identity
      and under a ×8 `transform`. Record it. It must be within `ε`.
- [ ] Tests (`tests/canvas/rt_canvas_picture.rs`, `rt_canvas_picture_fit.rs`):
      - a picture on a circle `CurvePolygon` matches the same picture on a
        `canvas::Circle` at the same `position` and radius within 1 coverage step;
      - a picture on a two-contour `CurvePolygon` shows the image in the ring and not
        in the hole, under both rules;
      - `Stretch`, `Contain` and `Cover` fit the image to the curve's true extent — a
        contour whose control points lie well outside the curve does not shrink the
        image;
      - `ClipOnly` and `ClipAndTransform` under a 30° rotation;
      - the shape's `paint.clip` cuts the picture as well as the outline does;
      - a stroked shape draws its outline over the image as a true curve;
      - a picture on a `PolyCurve` draws nothing and is hit by nothing;
      - a picture on an `AutoPaint` over a `CurvePolygon` changes its tint on hover.

Acceptance: an `AutoPaint` and a `Picture` treat the two curve shapes as they treat the
eight shapes before them.
  Check: `cargo test --test rt_canvas_autopaint --test rt_canvas_picture --test
  rt_canvas_picture_fit --test rt_canvas_curves` → pass, existing goldens unmodified
  (est. 10 min, guess).
Commit: —

## Validation Plan

- Tests: Phases 1–4, all hand-computed. Negative cases: invalid curves inside a chain
  (skipped, with the join line implied), an empty `curves` or `contours`, and a shape
  with no valid curve (draws nothing, is hit by nothing).
- Coverage check: every new arm runs in `rt_canvas_curves`, `rt_canvas_hit`,
  `rt_canvas_autopaint` or `rt_canvas_picture`.
- Runtime proof: an `--app` program in the software renderer (no `MFB_CANVAS_GPU`)
  drawing a filled rounded-star `CurvePolygon`, a dashed spiral `PolyCurve` from cubics,
  a hover-lit curved button and a photograph cut to a curved outline, screenshotted; the
  same program printing `overID` as the pointer is scripted across the star
  (`MFB_CANVAS_POINTER_X` / `_Y`).
- Doc sync: in `src/docs/spec/app/06_canvas.md`, "Hit-testing" → "The region of an
  item" gains the two shapes in the same change as Phase 3, and "A picture is drawn
  inside a shape" gains the `CurvePolygon` outline, its flattening tolerance and the
  exact-extent rule in the same change as Phase 4 (the Hard Completion Gate). The full
  curves section is C's.
- Final gate: at the end of plan-164-C.

## Open Decisions

None. The step counts are measured and recorded in Phase 1, and the picture's
flattening in Phase 4.

## Corrections

(none yet)

## Summary

The risk in the curve code is the rational piece's fixed-step distance search, which is
measured, not assumed. One item is one record, so nothing downstream has to learn a new
shape of draw list.

The other risk is Phase 4. A `Picture` on a `CurvePolygon` is the one place a curve is
flattened — only for the image's clip, with the true curve stroked on top — and the gap
between the two is measured against `ε` rather than trusted.
