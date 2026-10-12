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
| plan-164-A complete and archived | `ls planning/completed/plan-164-A-*` → match | MET (2026-10-11: `planning/completed/plan-164-A-bezier-types-reduction.md`) |

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

- [x] Resolve §2's UNVERIFIED row and make a frame containing either shape — at the top
      level or inside a group — take the whole-frame software fallback on the GPU
      backends.
      — resolved: both predicates walk the frame's flat offsets and read each record's
      kind from its first float, so each got one early `RETURN FALSE` for the two
      kinds (`helper_render.rs:__canvas_metalRenderable`, `__canvas_vulkanRenderable`).
      `canvas_curve_frames_fall_back_to_software_on_the_gpu` under `MFB_CANVAS_GPU=1`:
      four frames — a rectangle; the rectangle and a `CurvePolygon`; the rectangle and a
      use of a group holding a `PolyCurve`; the rectangle — report `gpuFrames=1,1,1,2`.
- [x] The record builders (`helper_geometry.rs`): the piece tail for both kinds and the
      per-contour table for `CURVEPOLYGON`, per §3, in local space.
      — `helper_curves.rs:__canvas_curveBuild`, called from both arms of
      `__canvas_geometryFor` and `__canvas_geoReference`. The tail is pieces | join
      records | dash tail | clip edges | gradient stops. There is no per-contour table
      (Corrections 2). A frame of both shapes under `MFB_CANVAS_GEO_VERIFY=1` reports
      `geoVerifyMismatches=0`.
- [x] `helper_draw.rs`: the piece fill test, the polynomial distance and the rational
      fixed search, per §3. Dispatch from `__canvas_geoDistance` on both kinds.
      — the fill test is `helper_curves.rs:__canvas_pieceCrossX` in
      `__canvas_curveDistance`. The distance is ONE function for both kinds of piece,
      `__canvas_pieceNearest`, and it isolates roots where §3 sampled (Corrections 1).
      `__canvas_geoDistance` dispatches `POLYCURVE` to `__canvas_curveScan` and
      `CURVEPOLYGON` to `__canvas_curveDistance`.
- [x] Measure the rational search's error against a 4096-sample brute-force minimum on
      a quarter circle (`w1 = √2/2`), a thin conic (`w1 = 5`) and a near-parabola
      (`w1 = 1.01`). Record the numbers and the chosen step counts here.
      — `canvas_piece_nearest_matches_a_brute_force_search`, 443 query points a piece
      (a 19 x 19 grid over the piece's box and 60 px round it, and 82 points on and
      beside the piece), against 4096 samples refined by a golden-section search:

      | Piece | Worst error, px |
      |---|---|
      | quarter circle, `w1 = √2/2`, r 100 | 7.1e-14 |
      | thin conic, `w1 = 5` | 3.1e-13 |
      | near-parabola, `w1 = 1.01` | 1.4e-13 |
      | flat conic, `w1 = 0.05` | 1.3e-13 |
      | parabola (`w1 = 1`) | 1.3e-13 |
      | tight parabola, a hairpin | 1.5e-13 |
      | straight | 5.7e-14 |
      | 2000 px conic | 8.3e-13 |

      Step count: `__CANVAS_PIECE_STEPS = 12` bracketed-Newton steps a root, the same
      for every piece. The limit §3 set was 0.01 px.
- [x] Tests (hand-computed, `tests/canvas/rt_canvas_curves.rs`):
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
      — `canvas_curve_polygon_circle_matches_a_circle`: **exactly** — `0 of 40000
      pixels differ` at radius 80.3. `canvas_curve_polygon_moves_with_its_position`,
      `…_fills_by_its_rule` (a figure of eight, whose two lobes are filled under both
      rules, and a contour that goes round twice, whose inner disc is a hole under
      EvenOdd and filled under NonZero — the case the two rules disagree on),
      `…_holes_follow_the_rule` (all four of rule x winding) and
      `…_closes_and_a_poly_curve_has_no_interior`. Also
      `canvas_piece_crossing_is_on_the_piece`.

Acceptance: `CurvePolygon` fill and antialiasing are correct in software, and a
`PolyCurve` fills nothing.
  Check: `cargo test --test rt_canvas_curves --test rt_canvas_rasteriser` → pass,
  rasteriser goldens unmodified (est. 8 min, guess).
  Result (2026-10-11): `rt_canvas_curves` 24 passed; `rt_canvas_rasteriser` 106 passed,
  2 ignored; no file under `tests/canvas/` modified but `rt_canvas_curves.rs`.
Commit: —

### Phase 2 — Stroke, caps, joins, dashes

- [x] The stroke along the pieces, caps at a `PolyCurve`'s two ends, and joins at real
      corners only (the join tail built from the input tangents).
      — `__canvas_pieceBand` / `__canvas_pieceCut` (one piece's band and its two
      ends), `__canvas_curveScan` (a chain), `__canvas_curveJoined` (a contour with
      join records), and `__canvas_curveJoins`, which builds one record a piece from
      `__CANVAS_CURVE_TANGENTS` — the input curves' own end tangents, recorded by the
      chain walk. Corrections 3 and 4.
- [x] The dash arc-length table per piece, `s` along a chain and per contour.
      — `__canvas_curveDashData` (17 positions and the contour's length a piece,
      Simpson's rule over 8 sub-steps of each sixteenth) and `__canvas_dashedCurves`.
- [x] Tests (hand-computed):
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
      — `canvas_curve_stroke_has_no_join_where_the_curve_is_smooth` (the mitered curve
      circle against `canvas::Circle` under the same paint: `0 of 48400 pixels
      differ`), `canvas_poly_curve_corner_takes_the_join` (Miter, Round and Bevel at the
      same corner), `…_strokes_its_implied_segment`,
      `…_that_returns_has_two_caps_not_a_join` (and the same triangle as a
      `CurvePolygon`, which does join there), `…_dashes_follow_arc_length` (ten dashes
      and ten gaps, and six points of the dash that crosses the boundary at 78.54) and
      `canvas_curve_polygon_dashes_restart_at_each_contour`.

Acceptance: the stroke is correct per §3 in software.
  Check: `cargo test --test rt_canvas_curves --test rt_canvas_rasteriser --test
  rt_canvas_golden` → pass, goldens unmodified (est. 10 min, guess).
  Result: see the end of Phase 4 — one run covers Phases 2 to 4.
Commit: —

### Phase 3 — Placement, paint integration and the hit region

- [x] Tests (hand-computed), for each shape unless it says otherwise:
      - its own `transform` as a 30° rotation turns it about its `position`;
      - inside a translated group;
      - inside a group that is turned and scaled by 2 — its stroke is twice as wide;
      - clipped by a ring clip (plan-163-C), the ring given in the shape's local space;
      - `transparency := 0.5`;
      - a radial gradient on a `CurvePolygon`'s fill, and on either shape's stroke when
        `gradient.strokeApply` gives it one (plan-163-D).
      Fix whatever fails in the curve arms. These use shared paint code, so a failure
      is a missing arm, not a design question.
      — `canvas_curve_shapes_turn_about_their_position`, `…_in_groups`,
      `…_are_clipped_in_their_own_space`, `…_take_the_whole_paint`. The last compares
      a `CurvePolygon` circle with a `canvas::Circle` under a half-transparent fill and
      a radial gradient that also paints the stroke, and a one-curve `PolyCurve` with a
      `canvas::Line` under a linear gradient on the stroke: both pairs equal pixel for
      pixel. What it found and fixed: a `PolyCurve`'s stroke gradient needed
      `POLYCURVE` beside `SEGMENT` and `ARC` in `__canvas_paintHeader`'s stroke-only
      rule and in `__canvas_drawGeometry`'s `gradTintFill`; and a cap's cut was not
      antialiased from INSIDE the end (Corrections 4).
- [x] `helper_hit.rs`: the two arms in `__canvas_hitDistance`, and the worker's record
      for each shape.
      — `__canvas_hitDistance` needed no arm: it calls `__canvas_geoDistance`, which
      has them. `__canvas_hitCurrent` got `POLYCURVE` in its open-kind list;
      `CURVEPOLYGON` takes the closed-kind path as it stands, with
      `__canvas_joinedStroke` dispatching to `__canvas_curveJoined`. The worker's
      record is the renderer's (`__canvas_hitGeometry`'s `CASE ELSE`).
- [x] Tests (`tests/canvas/rt_canvas_hit.rs`):
      - a filled `CurvePolygon` reports its `id` in `overID` inside it and not outside;
      - an unfilled one still reports it inside the outline;
      - a `PolyCurve` of one cubic with a 1-pixel stroke is hit 2 pixels from the curve
        and not 5, and is **not** hit in the area its ends would enclose;
      - either shape with a `clip` is not hit where the clip cuts it away;
      - one inside a named group reports `group::shape`;
      - one with no `id` still blocks the item beneath it.
      — one test, `canvas_curve_shapes_are_hit_where_they_are_drawn`, sixteen presses,
      in `rt_canvas_curves.rs` beside the scenes it shares helpers with (Corrections 6).

Acceptance: both shapes honour their placement and every `Paint` field in software, and
the pointer finds each where it is drawn.
  Check: `cargo test --test rt_canvas_curves --test rt_canvas_hit` → pass (est. 6 min,
  guess).
  Result: see the end of Phase 4.
Commit: —

### Phase 4 — `AutoPaint` and `Picture`

The two things a shape can be held by. Last because they sit on top of everything
above, and a failure here is in the holder's code, not the curve's.

- [x] `helper_autopaint.rs`: the arms for both shapes wherever it switches on the shape
      to swap its paint. Find them with
      `grep -n 'CASE PolyLine(\|CASE Polygon(' src/codegen/builtins/canvas/helper_autopaint.rs`.
      — the grep names two `MATCH`es, `__canvas_autoItem` and `__canvas_autoItemWith`,
      and both got their arms in plan-164-A (they are exhaustive, so the program would
      not have compiled without them). Nothing else in the file tells shapes apart.
- [x] Tests (`tests/canvas/rt_canvas_autopaint.rs`, with the scripted pointer): an
      `AutoPaint` over a `CurvePolygon` shows `hoverPaint` with the pointer inside the
      curved outline and the default paint with it in the outline's bounding box but
      outside the curve; the same over a `PolyCurve` near and away from its stroke; a
      state paint with a different `strokeWidth` does not change where it is hovered.
      — `an_autopaint_over_a_curve_polygon_follows_the_curve_not_its_box`,
      `an_autopaint_over_a_poly_curve_follows_its_stroke`,
      `a_curve_shapes_hover_stroke_does_not_widen_its_hit_region`.
- [x] `helper_picture.rs`: `__canvas_pictureClosed`, the `CurvePolygon` arm in
      `__canvas_pictureImage` (pieces → flattened rings, the exact extent, `rule`), and
      the outline item, per §3.
      — the three arms, over `helper_curves.rs:__canvas_curveRings`, which flattens
      each contour's pieces and leaves the true extent in `__CANVAS_CURVE_EXTENT`
      (`__canvas_cvTurn` for a piece's turn in x).
- [x] Measure the flattening: for a circle `CurvePolygon`, the largest gap between the
      image's clipped edge and the true curve, in surface pixels, under the identity
      and under a ×8 `transform`. Record it. It must be within `ε`.
      — `canvas_picture_curve_clip_is_within_the_tolerance_of_the_curve`, read off the
      frame of a white picture (an edge pixel's coverage is 0.5 less its centre's
      distance to the clip's edge), over the 278 edge pixels between 15% and 85%
      covered, radius 60 on the surface:

      | Case | Worst gap, px |
      |---|---|
      | identity | 0.0246 |
      | ×8 `transform`, `ClipAndTransform` (radius 7.5 in its own space) | 0.0246 |
      | ×8 `transform`, `ClipOnly` | 0.0246 |

      `ε` is 0.05. The frame resolves a distance to about 0.006 px at mid coverage.
- [x] Tests (`tests/canvas/rt_canvas_picture.rs`, `rt_canvas_picture_fit.rs`):
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
      — `canvas_picture_curve_clip_is_within_the_tolerance_of_the_curve` (the circle
      comparison, restated: Corrections 5), `canvas_picture_curve_holes_and_outline`,
      `…_fits_the_true_extent`, `…_clip_modes_and_the_paints_clip`,
      `canvas_picture_on_a_poly_curve_draws_nothing`, all in `rt_canvas_curves.rs`
      (Corrections 6); and
      `a_picture_on_an_autopaint_over_a_curve_polygon_tints_on_hover` in
      `rt_canvas_autopaint.rs`, where the scripted pointer is.

Acceptance: an `AutoPaint` and a `Picture` treat the two curve shapes as they treat the
eight shapes before them.
  Check: `cargo test --test rt_canvas_autopaint --test rt_canvas_picture --test
  rt_canvas_picture_fit --test rt_canvas_curves` → pass, existing goldens unmodified
  (est. 10 min, guess).
  Result (2026-10-11), one run for Phases 2 to 4, with `rt_canvas_hit`,
  `rt_canvas_rasteriser` and `rt_canvas_golden` added: `MFB_TEST_EXE=target/p164/mfb-b5 cargo test --release --test <suite>` over twelve
  suites, all `0 failed` (`grep -E '^test result' target/p164/t-B5.log`): curves 40,
  autopaint 22, picture 16, picture_fit 9, hit 13, rasteriser 106 (2 ignored), golden 26,
  metal 52, geo_native 4, damage 7, auto_step 13, cli_canvas_man_examples_compile 2.
  `bash scripts/artifact-gate.sh target/p164/mfb-b5 all` → `1686 tests, 1865 build(s),
  2350 golden(s) checked, 8 diff(s)`, all eight in the two fixtures that build a canvas
  program (`app-mouse-surface` 7, `loop-body-resource-reference-valid` 1) — the same two
  letter A moved, and for the same reason (the canvas package's source grew). Both
  re-baselined with that compiler.
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
  — done 2026-10-11 (`tests/canvas/scenes/curves.mfb`, which plan-164-C carries to
  the GPU lanes). Headless, software: the frame shows the star with a white outline,
  the dashed spiral with round dash ends, the pill in its HOVER paint with the scripted
  pointer on it (`auto=3`), and the 64 x 64 image under `Cover` inside the cubic blob,
  outlined. Ten presses, four of them across the star along y = 170 and up its top
  point, report `[] [star] [star] [star] [] [] [] [] [button] [photo::frame]`;
  `geoVerifyMismatches=0`. (The presses are injected with `MFB_MOUSE_INJECT`, which is
  what `canvas::pollMouse` reads; `MFB_CANVAS_POINTER` scripts the hover.)
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

1. **The distance is one function, and it isolates roots; it does not sample** (§3
   "Distance", and Phase 1's "polynomial distance and rational fixed search"). §3 gave
   a polynomial piece a closed-form cubic (trigonometric / Cardano) and a rational
   piece 8 samples and 6 halvings. Both are replaced by
   `helper_curves.rs:__canvas_pieceNearest`:
   - With `Q(t) = N(t) − p·W(t)`, the squared distance is `Q·Q / W²` and its slope has
     the sign of `g(t) = (Q·Q′)W − (Q·Q)W′` — a quartic for a rational piece, a cubic
     for a polynomial one (`W ≡ 1`). `g″` is a quadratic, so its roots are closed form;
     between them `g′` is monotone and holds at most one root; between THOSE `g` is
     monotone, and each stretch that goes from negative to positive holds exactly one
     minimum. Each root is found by a bracketed Newton of 12 steps.
   - Why not the closed form: it needs a cube root and an arc cosine, and the software
     renderer is the oracle on every platform — `helper_shapes.rs`'s note on why
     `math::sin` cannot be used there applies to both. `+ − × ÷ sqrt` only is the rule
     the ellipse's distance already keeps.
   - Why not the sampled search: its error is a fraction of the PIECE'S LENGTH. Eight
     samples and six halvings leave `t` known to 1/256, which on the 157 px quarter
     circle is 0.6 px along the curve, and a pixel on the edge is then 0.6 px wrong;
     a piece has no length limit. It can also miss a minimum between two samples.
   - Measured (Phase 1): at most 8.3e-13 px on eight pieces, one 2000 px long, against
     the 0.01 px §3 allowed. A `CurvePolygon` circle is `canvas::Circle` pixel for
     pixel.
   - One bug found by that measurement: a Newton step that lands exactly on the
     bracket's end was rejected (the test was `lo < t < hi`), so a root that had
     CONVERGED was thrown back to the midpoint. The bracket's ends are now accepted.
   plan-164-C's `pieceDistancePoly` and `pieceDistanceRational` are therefore one
   shader function, written from this one.
2. **There is no per-contour table** (§3 "The record", Phase 1). Its stated reader was
   the stroke, "so the stroke knows where each closed outline starts over". Nothing
   reads one: there is one join record a piece (the corner at its start, which for a
   contour's first piece is the join with its last) and one dash row a piece (carrying
   its contour's length), exactly as a polygon has one of each an edge and no table of
   rings. `CURVE_PIECE_COUNT` is header slot 20, the slot a polygon's edge count is in.
   A `CurvePolygon`'s rule is slot 21, `__CANVAS_GEO_POLYGON_RULE`.
3. **A join record a piece, not one a corner** (§3 "Stroke"). With a Miter or Bevel
   paint the stroke is the polygon's construction: every piece's band cut square at
   both ends, and a record at every piece's start. A real corner — the boundary of two
   input segments whose tangents differ by more than 1e-6 rad — takes the paint's
   join; every other piece start takes a DISC, which shapes nothing and is the overlap
   two square-cut bands need where they meet (`helper_joins.rs`, "The pieces overlap,
   and that is load-bearing"). So the reduction's cuts are never mitered, as §3
   requires, and the GPUs can find a pixel's corners through the pieces' band index.
   An undashed Round paint builds no records, as for a polygon.
4. **A cap's cut is taken a pixel short of the end as well as past it.** The first cut
   of `__canvas_pieceBand` applied the end plane only where the pixel's nearest point
   of the piece was the end itself. A pixel just INSIDE a butt end then had full
   coverage where a `canvas::Line`'s has part — `canvas_curve_shapes_take_the_whole_paint`
   found it, pixel (71, 395), 150 against 194 of 255. `__canvas_pieceCut` takes the
   plane within one pixel short of it too, for a pixel in the cap's own neighbourhood;
   a piece that curls back behind its own end plane keeps its band everywhere else.
   A Butt end is the `max` of the round band and the plane, the two terms
   `__canvas_segmentDistanceButt` uses, so a chain of one straight curve is that line.
5. **"Within 1 coverage step" of a picture on a `canvas::Circle` cannot hold, and is
   replaced by two stronger statements** (Phase 4). The circle's outline is flattened
   by `__canvas_ellipseRing` and the curve's by `__canvas_cvFlatten`; each is within
   0.05 px of the true circle, so the two can be 0.1 px apart, which is 25 coverage
   steps. What is asserted instead: the curve's clip is within 0.05 px of the TRUE
   circle (measured 0.0246); no pixel more than 1 px from the edge differs at all; and
   no pixel differs by more than a tenth of a pixel's coverage (measured 0.0509).
6. **The tests are in `rt_canvas_curves.rs`**, not spread over `rt_canvas_hit.rs`,
   `rt_canvas_picture.rs` and `rt_canvas_picture_fit.rs` as Phases 3 and 4 name. They
   share one prelude (the arcs of a circle, a `CurvePolygon`, a `PolyCurve`) and one
   harness that reads the frame, the stats and `overID` from the same run.
   `rt_canvas_picture_fit.rs` lifts the fit and clip-intersection arithmetic out of
   `helper_picture.rs`, which this letter did not change. The two files that have
   something the curve file has not — the scripted pointer — gained their tests in
   place (`rt_canvas_autopaint.rs`).
7. **The picture's rings are the reduced pieces flattened, so the clip is within `ε` of
   the outline that is DRAWN** (§3). For a curve of degree 3 or more the pieces are
   themselves within `ε` of the curve as written, so the clip can be `2ε` from that;
   it is the gap between the image's edge and the stroked outline that shows, and that
   is the one held to `ε`.
8. **A curve shape's zero-length pieces are left out of its record**
   (`__canvas_cvLive`), and a segment with none left leaves no mark. A piece with no
   length has no direction for a cap or a join to follow. A `PolyCurve` whose every
   curve is a single point is the one exception and keeps one such piece: it draws
   what a zero-length `canvas::Line` with its cap draws, a dot for Round, a square for
   Square, nothing for Butt.
9. **Hull bounds are padded by the cap's reach as well as the join's** for a
   `PolyCurve`: a Square end's corner is `half·√2` from the end point, further than
   `__canvas_strokePad` allows for a kind with no cap.
10. **The curve code is gated, on the shapes being NAMED** — which reverses
    plan-164-A's Correction 9 (archived) and adds a gate to the registry.
    - Measured, as `.ai/canvas-threading.md` §16 asks before an always-compiled arm is
      left in ("Measure what an always-compiled arm costs"): `__text` of a canvas
      program that draws no curve (`tests/canvas/scenes/circles_on_black.mfb`,
      `otool -l`, macOS AArch64) was 2,706,656 bytes on `main`, 2,834,620 after
      plan-164-A and 2,995,552 with this letter's code unconditional — 288,896 bytes for
      a feature the program does not use. `AutoPaint` was gated for 24,744.
    - A-9's objection stands: `AutoPaint`'s gate (`WhenUsedOrConstructed`) is decided
      from constructor expressions, and `MUT c AS canvas::PolyCurve` followed by `WITH`
      constructs nothing. So the gate is a new pair,
      `HelperGate::WhenCalledOrNamed` / `WhenNeitherCalledNorNamed`
      (`src/codegen/registry/mod.rs`): the real chunk opens when the program NAMES one
      of the types anywhere — a constructor, or a binding's, parameter's, return type's
      or field's annotation, at any depth of the spelling — and the stand-in is its
      exact complement, so the two may define the same functions.
      `ProjectView` collects the annotations (`item_types`, `hir_item_types`);
      `a_named_type_opens_its_gate_and_closes_the_exact_complement` pins both views.
    - Three chunks: the shapes (`canvas_curves`, opened by naming `PolyCurve` or
      `CurvePolygon`; stand-in `ABSENT`, ten hooks); the two functions' bodies
      (`canvas_curveFuncs`, opened by calling either; stand-in `FUNCS_ABSENT`) — a
      registered function's body is compiled whether or not it is called, so each body
      is one call into its chunk; and the base both stand on (`canvas_curveBase`,
      opened by either condition, no stand-in). The record-building and hashing arms
      call hooks that take the shape, so an arm no longer copies a record's fields.
    - After: 2,728,308 bytes, 21,652 over `main`. What is left is the arms themselves,
      the kind tests in the distance dispatch, and the two stand-ins.
    - `canvas_curve_hooks_are_declared_alike_in_both_chunks` holds the two chunks'
      signatures equal, as its `AutoPaint` twin does.

## Summary

The risk in the curve code is the rational piece's fixed-step distance search, which is
measured, not assumed. One item is one record, so nothing downstream has to learn a new
shape of draw list.

The other risk is Phase 4. A `Picture` on a `CurvePolygon` is the one place a curve is
flattened — only for the image's clip, with the true curve stroked on top — and the gap
between the two is measured against `ε` rather than trusted.
