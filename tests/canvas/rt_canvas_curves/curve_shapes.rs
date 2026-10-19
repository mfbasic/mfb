//! plan-176-D: `rt_canvas_curves`'s drawn curve shapes: curve polygons and their fill
//! rules, GPU frames, joins, caps and dashes, and placement, groups, clips, paint and
//! hit testing. The scene helpers live in the root, `rt_canvas_curves.rs`.

use super::*;

/// A `CurvePolygon` circle is a `canvas::Circle`.
///
/// Four exact conic arcs of radius 80.3 about (0, 0), at `position` (600, 200), beside a
/// `canvas::Circle` of the same radius at (200, 200), both white on black. The centre
/// is covered; a pixel centre 2.2 px outside the radius is not; and the two 200x200
/// blocks are compared pixel for pixel — the curve's distance is a root found to the
/// last places of a Float where the circle's is a formula, so they may differ where a
/// coverage lands exactly on a rounding boundary, and nowhere else.
#[test]
fn canvas_curve_polygon_circle_matches_a_circle() {
    let run = scene(
        "canvas_curve_circle",
        r#"  LET white AS canvas::Paint = canvas::fill(color::rgb(255, 255, 255))
  LET disc AS canvas::DrawItem = canvas::Circle[id := "", position := at(200.0, 200.0), radius := 80.3, paint := white, transform := none()]
  canvas::present([disc, blob("", 600.0, 200.0, [circle(0.0, 0.0, 80.3)], canvas::FillRule.EvenOdd, white)])
"#,
        "",
        &[],
    );
    assert_eq!(run.pixel(600, 200), WHITE, "the centre");
    assert_eq!(
        run.pixel(600 + 82, 200),
        BLACK,
        "a pixel centre 2.2 px outside"
    );
    assert_eq!(
        run.pixel(600 - 79, 200),
        WHITE,
        "a pixel centre 1.8 px inside"
    );
    let mut differing = 0;
    let mut worst = 0;
    for y in 100..300 {
        for x in 100..300 {
            let (a, b) = (run.pixel(x, y), run.pixel(x + 400, y));
            if a != b {
                differing += 1;
                worst = worst.max((a.0 as i32 - b.0 as i32).abs());
            }
        }
    }
    println!("circle vs CurvePolygon circle: {differing} of 40000 pixels differ, worst channel delta {worst}");
    assert_eq!(
        differing, 0,
        "the curve circle is the circle, pixel for pixel"
    );
    assert_eq!(run.stat("geoVerifyMismatches"), "0");
}

/// Moving a `CurvePolygon` by its `position` alone moves its pixels and changes none:
/// an egg of two cubics and two conics at (150.5, 200.25) and again at (150.5 + 300,
/// 200.25 + 170) — whole pixels apart, so the antialiased edge is the same edge.
#[test]
fn canvas_curve_polygon_moves_with_its_position() {
    let run = scene(
        "canvas_curve_position",
        r#"  LET paint AS canvas::Paint = canvas::fill(color::rgb(255, 255, 255))
  canvas::present([blob("", 150.5, 200.25, [egg()], canvas::FillRule.EvenOdd, paint), blob("", 450.5, 370.25, [egg()], canvas::FillRule.EvenOdd, paint)])
"#,
        r#"
FUNC egg() AS List OF canvas::BezierCurve
  RETURN [cubic(0.0, 0.0 - 90.0, 70.0, 0.0 - 90.0, 100.0, 0.0 - 20.0, 100.0, 30.0), conic(100.0, 30.0, 100.0, 100.0, 0.0, 100.0, 0.7071067811865476), conic(0.0, 100.0, 0.0 - 100.0, 100.0, 0.0 - 100.0, 30.0, 0.7071067811865476), cubic(0.0 - 100.0, 30.0, 0.0 - 100.0, 0.0 - 20.0, 0.0 - 70.0, 0.0 - 90.0, 0.0, 0.0 - 90.0)]
END FUNC
"#,
        &[],
    );
    assert_eq!(run.pixel(150, 200), WHITE, "inside the first");
    assert_eq!(run.pixel(450, 370), WHITE, "inside the second");
    let mut edge = 0;
    for y in 100..310 {
        for x in 40..260 {
            let a = run.pixel(x, y);
            assert_eq!(
                a,
                run.pixel(x + 300, y + 170),
                "the pixel at ({x}, {y}), moved"
            );
            if a != WHITE && a != BLACK {
                edge += 1;
            }
        }
    }
    assert!(
        edge > 400,
        "the comparison crossed an antialiased edge ({edge} pixels)"
    );
}

/// The fill rule decides a region by its winding, as it does for a polygon.
///
/// * A figure of eight — two cubics crossing at (0, 0), one lobe wound each way — at
///   x = 150: each lobe has winding ±1, so both lobes are filled under both rules, and
///   the pixels left and right of the crossing are outside.
/// * One contour that goes round twice the same way — a circle of radius 90, then one
///   of radius 40 inside it, joined by the implied segment and closed by another along
///   the same line — at x = 450 (EvenOdd) and x = 720 (NonZero). The ring has winding 1
///   and is filled under both; the inner disc has winding 2, a hole under EvenOdd and
///   filled under NonZero.
#[test]
fn canvas_curve_polygon_fills_by_its_rule() {
    let run = scene(
        "canvas_curve_rule",
        r#"  LET paint AS canvas::Paint = canvas::fill(color::rgb(0, 0, 255))
  LET eight AS List OF canvas::BezierCurve = [cubic(0.0, 0.0, 120.0, 0.0 - 160.0, 120.0, 160.0, 0.0, 0.0), cubic(0.0, 0.0, 0.0 - 120.0, 0.0 - 160.0, 0.0 - 120.0, 160.0, 0.0, 0.0)]
  LET twice AS List OF canvas::BezierCurve = collections::append(circle(0.0, 0.0, 90.0), circle(0.0, 0.0, 40.0))
  canvas::present([blob("", 150.0, 150.0, [eight], canvas::FillRule.EvenOdd, paint), blob("", 150.0, 450.0, [eight], canvas::FillRule.NonZero, paint), blob("", 450.0, 300.0, [twice], canvas::FillRule.EvenOdd, paint), blob("", 720.0, 300.0, [twice], canvas::FillRule.NonZero, paint)])
"#,
        "",
        &[],
    );
    // Each lobe's cubic reaches x = ±90 at t = 1/2 (0.75 * 120), on y = 0.
    for (what, cy) in [("EvenOdd", 150), ("NonZero", 450)] {
        assert_eq!(run.pixel(150 + 60, cy), BLUE, "{what}: the right lobe");
        assert_eq!(run.pixel(150 - 60, cy), BLUE, "{what}: the left lobe");
        assert_eq!(
            run.pixel(150 + 95, cy),
            BLACK,
            "{what}: right of the right lobe"
        );
        assert_eq!(run.pixel(150, cy - 40), BLACK, "{what}: above the crossing");
        assert_eq!(run.pixel(150, cy + 40), BLACK, "{what}: below the crossing");
    }
    assert_eq!(
        run.pixel(450 + 65, 300),
        BLUE,
        "EvenOdd: the ring, winding 1"
    );
    assert_eq!(
        run.pixel(450, 300),
        BLACK,
        "EvenOdd: the inner disc, winding 2"
    );
    assert_eq!(
        run.pixel(450 + 20, 300 + 20),
        BLACK,
        "EvenOdd: the inner disc again"
    );
    assert_eq!(run.pixel(720 + 65, 300), BLUE, "NonZero: the ring");
    assert_eq!(
        run.pixel(720, 300),
        BLUE,
        "NonZero: the inner disc, winding 2"
    );
    assert_eq!(run.pixel(450 + 95, 300), BLACK, "outside the outer circle");
    assert_eq!(run.pixel(720 + 95, 300), BLACK, "outside the outer circle");
    // The implied segments lie on y = 0 from x = 40 to 90, there and back: they add no
    // edge to the picture. Just above and below them is ring, under both rules.
    assert_eq!(run.pixel(450 + 65, 300 - 1), BLUE);
    assert_eq!(run.pixel(720 + 65, 300), BLUE);
}

/// Two contours, one inside the other (plan-163-A's rule, with curves): the inner is a
/// hole under EvenOdd whichever way it winds; under NonZero it is a hole only when it
/// winds against the outer, and filled when it winds with it.
#[test]
fn canvas_curve_polygon_holes_follow_the_rule() {
    let run = scene(
        "canvas_curve_holes",
        r#"  LET paint AS canvas::Paint = canvas::fill(color::rgb(0, 0, 255))
  LET outer AS List OF canvas::BezierCurve = circle(0.0, 0.0, 90.0)
  LET same AS List OF canvas::BezierCurve = circle(0.0, 0.0, 40.0)
  LET against AS List OF canvas::BezierCurve = circleBack(0.0, 0.0, 40.0)
  canvas::present([blob("", 120.0, 120.0, [outer, same], canvas::FillRule.EvenOdd, paint), blob("", 340.0, 120.0, [outer, against], canvas::FillRule.EvenOdd, paint), blob("", 560.0, 120.0, [outer, same], canvas::FillRule.NonZero, paint), blob("", 780.0, 120.0, [outer, against], canvas::FillRule.NonZero, paint)])
"#,
        "",
        &[],
    );
    for (what, cx, hole) in [
        ("EvenOdd, wound the same way", 120, true),
        ("EvenOdd, wound against", 340, true),
        ("NonZero, wound the same way", 560, false),
        ("NonZero, wound against", 780, true),
    ] {
        assert_eq!(run.pixel(cx + 65, 120), BLUE, "{what}: the ring");
        assert_eq!(run.pixel(cx, 120 - 65), BLUE, "{what}: the ring, above");
        let want = if hole { BLACK } else { BLUE };
        assert_eq!(run.pixel(cx, 120), want, "{what}: the middle");
        assert_eq!(
            run.pixel(cx - 25, 120 + 25),
            want,
            "{what}: inside the inner"
        );
        assert_eq!(run.pixel(cx + 93, 120), BLACK, "{what}: outside");
    }
}

/// A contour whose last point is not its first is filled to the implied closing
/// segment. A half disc: two arcs from (90, 0) over the BOTTOM to (-90, 0), and
/// nothing else — the diameter along y = 0 is implied. At `position` (200, 200) the
/// pixel 2 rows below the diameter is filled, the pixel 3 rows above it is not, and
/// the row of pixel centres half a pixel below the diameter is fully covered while the
/// one half a pixel above is not at all: the closing segment is an edge like any other.
///
/// Beside it, a `PolyCurve` over the same two arcs covers no interior pixel: only its
/// stroke is drawn.
#[test]
fn canvas_curve_polygon_closes_and_a_poly_curve_has_no_interior() {
    let run = scene(
        "canvas_curve_closing",
        r#"  LET k AS Float = 0.7071067811865476
  LET half AS List OF canvas::BezierCurve = [conic(90.0, 0.0, 90.0, 90.0, 0.0, 90.0, k), conic(0.0, 90.0, 0.0 - 90.0, 90.0, 0.0 - 90.0, 0.0, k)]
  canvas::present([blob("", 200.0, 200.0, [half], canvas::FillRule.EvenOdd, canvas::fill(color::rgb(0, 0, 255))), wave("", 500.0, 200.0, half, canvas::stroke(color::rgb(0, 0, 255), 4.0))])
"#,
        "",
        &[],
    );
    assert_eq!(run.pixel(200, 202), BLUE, "2 rows below the diameter");
    assert_eq!(run.pixel(200, 240), BLUE, "deep in the half disc");
    assert_eq!(run.pixel(200, 197), BLACK, "3 rows above the diameter");
    assert_eq!(
        run.pixel(200 + 60, 200),
        BLUE,
        "centre 0.5 px below the diameter"
    );
    assert_eq!(
        run.pixel(200 + 60, 199),
        BLACK,
        "centre 0.5 px above the diameter"
    );
    assert_eq!(run.pixel(200, 293), BLACK, "below the arc");

    // The chain: its stroke is on the arc, 2 px either side of radius 90.
    assert_eq!(
        run.pixel(500, 200 + 89),
        BLUE,
        "on the arc, at its lowest point"
    );
    assert_eq!(
        run.pixel(500, 240),
        BLACK,
        "inside what the chain would enclose"
    );
    assert_eq!(
        run.pixel(500, 202),
        BLACK,
        "where a closing segment would be"
    );
    assert_eq!(run.pixel(500 + 60, 200), BLACK, "on the diameter");
    let interior = (0..80)
        .flat_map(|dy| (-60..60).map(move |dx: i32| (dx, dy)))
        .filter(|(dx, dy)| ((dx * dx + dy * dy) as f64).sqrt() < 85.0 && *dy > 3)
        .filter(|(dx, dy)| run.pixel((500 + dx) as usize, (200 + dy) as usize) != BLACK)
        .count();
    assert_eq!(interior, 0, "a PolyCurve fills nothing");
}

/// A frame that holds a curve shape -- in the scene itself or inside a group -- is drawn
/// on the GPU, like the frames either side of it (plan-164-C; letter B had both
/// backends hand such a frame to the software renderer, and this test pinned that).
///
/// Four frames under `MFB_CANVAS_GPU=1`: a rectangle alone; the rectangle and a
/// `CurvePolygon`; the rectangle and a use of a group that holds a `PolyCurve`; the
/// rectangle alone again. `gpuFrames` counts the frames the GPU drew: 1, 2, 3, 4. The
/// last frame's pixels are the rectangle's, so a curve frame leaves nothing behind.
/// What the GPU draws for a curve is `rt_canvas_metal.rs`'s to compare.
///
/// Decided from the program's own stats line, as `rt_canvas_metal.rs` decides: a host
/// with no GPU renderer ready reports so, and the test has nothing to say there.
#[test]
fn canvas_curve_frames_are_drawn_on_the_gpu() {
    let run = scene_on(
        "canvas_curve_gpu_frames",
        r#"  MUT box AS canvas::Rectangle
  box = WITH box { position := at(50.0, 50.0), size := canvas::Size[width := 100.0, height := 60.0], paint := canvas::fill(color::rgb(255, 255, 255)) }
  LET boxItem AS canvas::DrawItem = box
  LET arcs AS List OF canvas::BezierCurve = circle(0.0, 0.0, 60.0)
  canvas::setGroup("g", [wave("", 0.0, 0.0, arcs, canvas::stroke(color::rgb(0, 0, 255), 6.0))])
  LET use AS canvas::DrawItem = canvas::Group[id := "", position := at(400.0, 300.0), transform := none(), name := "g"]
  canvas::present([boxItem])
  canvas::present([boxItem, blob("", 300.0, 300.0, [arcs], canvas::FillRule.EvenOdd, canvas::fill(color::rgb(0, 0, 255)))])
  canvas::present([boxItem, use])
  canvas::present([boxItem])
"#,
        "",
        &[],
        true,
    );
    let ready = run
        .stats
        .first()
        .is_some_and(|line| line.contains("metalReady=TRUE") || line.contains("vulkanReady=TRUE"));
    if !ready {
        eprintln!(
            "skipped: no GPU renderer is ready on this host: {:?}",
            run.stats.first()
        );
        return;
    }
    assert_eq!(
        run.stats_of("gpuFrames"),
        ["1", "2", "3", "4"],
        "{:?}",
        run.stats
    );
    assert_eq!(run.pixel(100, 80), WHITE, "the rectangle");
    assert_eq!(
        run.pixel(300, 300),
        BLACK,
        "nothing of the curve frames is left"
    );
}

// ---- plan-164-B Phase 2: the stroke, caps, joins and dashes --------------------------

/// A stroke paint as MFBASIC source: white, `width` wide, with `more` fields set.
fn stroke_paint(width: f64, more: &str) -> String {
    format!("WITH canvas::stroke(color::rgb(255, 255, 255), {width:?}) {{ {more} }}")
}

/// The pixel that holds the point `(x, y)`.
fn pixel_of(x: f64, y: f64) -> (usize, usize) {
    (x.floor() as usize, y.floor() as usize)
}

/// A stroked `CurvePolygon` circle of four arcs under `join := Miter` is the stroked
/// circle: the four places one arc ends and the next begins are smooth, so nothing is
/// mitered there, and a closed contour has no cap anywhere.
///
/// Compared with a `canvas::Circle` under the same paint, 400 px to its left. With a
/// Miter paint the curve's stroke is built as the polygon's is — each piece's band cut
/// square at its ends, and a disc at each joint to cover the seam — so it is not the
/// same arithmetic as the circle's `|d| - half`, and the test states what it measures:
/// no pixel beyond the stroke's outer edge at a joint (where a miter would point), and
/// the whole 220x220 block within one coverage step of the circle's.
#[test]
fn canvas_curve_stroke_has_no_join_where_the_curve_is_smooth() {
    let paint = stroke_paint(10.0, "join := canvas::JoinStyle.Miter");
    let run = scene(
        "canvas_curve_smooth",
        &format!(
            "  LET paint AS canvas::Paint = {paint}\n  \
             LET ring AS canvas::DrawItem = canvas::Circle[id := \"\", position := at(200.0, 200.0), radius := 80.0, paint := paint, transform := none()]\n  \
             canvas::present([ring, blob(\"\", 600.0, 200.0, [circle(0.0, 0.0, 80.0)], canvas::FillRule.EvenOdd, paint)])\n"
        ),
        "",
        &[],
    );
    // The joints are at the circle's four axis points. The stroke's outer edge is at
    // radius 85; a pixel centre 2.5 px beyond it is background on each axis.
    for (dx, dy) in [(87, 0), (-88, 0), (0, 87), (0, -88)] {
        let (x, y) = ((600 + dx) as usize, (200 + dy) as usize);
        assert_eq!(run.pixel(x, y), BLACK, "beyond the joint at ({dx}, {dy})");
    }
    assert_eq!(run.pixel(600 + 80, 200), WHITE, "on the stroke, at a joint");
    assert_eq!(run.pixel(600, 200), BLACK, "the unfilled middle");
    let mut differing = 0;
    let mut worst = 0;
    for y in 90..310 {
        for x in 90..310 {
            let (a, b) = (run.pixel(x, y), run.pixel(x + 400, y));
            if a != b {
                differing += 1;
                worst = worst.max((a.0 as i32 - b.0 as i32).abs());
            }
        }
    }
    println!("mitered curve circle vs circle: {differing} of 48400 pixels differ, worst channel delta {worst}");
    assert_eq!(
        differing, 0,
        "the joined stroke of a smooth outline is the plain one"
    );
}

/// A corner between two curves takes the paint's join. A cubic arriving at (100, 0)
/// heading +x, then a line leaving it heading +y: a right turn, whose outer side is up
/// and to the right. With a 12 px stroke the two bands' outer edges are y = -6 and
/// x = 106, and they meet at (106, -6).
///
/// * `Miter`: the pixel centred (105.5, -5.5) is half a pixel inside both edges —
///   fully covered.
/// * `Round` (the default): that centre is 7.78 px from the corner, outside the 6 px
///   round — background.
/// * `Bevel`: it is beyond the chord from (100, -6) to (106, 0) — background.
///
/// Under all three the pixel centred (102.5, -2.5), 3.5 px from the corner, is covered.
#[test]
fn canvas_poly_curve_corner_takes_the_join() {
    let shape = |name: &str, x: f64, join: &str| {
        format!(
            "wave(\"{name}\", {x:?}, 200.0, [cubic(0.0, 60.0, 30.0, 0.0, 70.0, 0.0, 100.0, 0.0), line(100.0, 0.0, 100.0, 80.0)], {})",
            stroke_paint(12.0, &format!("join := canvas::JoinStyle.{join}"))
        )
    };
    let run = scene(
        "canvas_curve_corner",
        &format!(
            "  canvas::present([{}, {}, {}])\n",
            shape("miter", 100.0, "Miter"),
            shape("round", 350.0, "Round"),
            shape("bevel", 600.0, "Bevel")
        ),
        "",
        &[],
    );
    for (what, x, tip) in [
        ("Miter", 100, WHITE),
        ("Round", 350, BLACK),
        ("Bevel", 600, BLACK),
    ] {
        assert_eq!(run.pixel(x + 105, 200 - 6), tip, "{what}: the miter point");
        assert_eq!(
            run.pixel(x + 102, 200 - 3),
            WHITE,
            "{what}: just outside the corner"
        );
        assert_eq!(run.pixel(x + 100, 200 + 40), WHITE, "{what}: on the line");
        assert_eq!(
            run.pixel(x + 108, 200 - 8),
            BLACK,
            "{what}: beyond every join"
        );
        // The cubic's own pieces meet smoothly: nowhere along it is there a spike. Its
        // middle is at (50, 7.5); 9 px above that is outside a 6 px half-width.
        assert_eq!(
            run.pixel(x + 50, 200 - 2),
            BLACK,
            "{what}: above the cubic's middle"
        );
    }
}

/// A curve that starts away from where the one before it ended is joined to it by a
/// straight segment, which is stroked, and which has a corner at each end. Two
/// horizontal lines, (0, 0)-(60, 0) and (60, 40)-(120, 40): the implied segment runs
/// down x = 60. Under `Miter` with a 12 px stroke the corner at (60, 0) is a right
/// turn with its point at (66, -6), and the corner at (60, 40) a left turn with its
/// point at (54, 46).
#[test]
fn canvas_poly_curve_strokes_its_implied_segment() {
    let run = scene(
        "canvas_curve_implied",
        &format!(
            "  canvas::present([wave(\"\", 200.0, 200.0, [line(0.0, 0.0, 60.0, 0.0), line(60.0, 40.0, 120.0, 40.0)], {})])\n",
            stroke_paint(12.0, "join := canvas::JoinStyle.Miter")
        ),
        "",
        &[],
    );
    assert_eq!(
        run.pixel(200 + 60, 200 + 20),
        WHITE,
        "the middle of the implied segment"
    );
    assert_eq!(
        run.pixel(200 + 65, 200 - 6),
        WHITE,
        "the miter point of the first corner"
    );
    assert_eq!(
        run.pixel(200 + 54, 200 + 45),
        WHITE,
        "the miter point of the second"
    );
    assert_eq!(
        run.pixel(200 + 67, 200 - 8),
        BLACK,
        "beyond the first corner's point"
    );
    assert_eq!(run.pixel(200 + 52, 200 + 47), BLACK, "beyond the second's");
    assert_eq!(run.pixel(200 + 30, 200 + 20), BLACK, "left of the segment");
    assert_eq!(run.pixel(200 + 90, 200 + 20), BLACK, "right of the segment");
    assert_eq!(run.pixel(200 + 30, 200), WHITE, "the first line");
    assert_eq!(run.pixel(200 + 90, 200 + 40), WHITE, "the second line");
}

/// A `PolyCurve` is never closed: one that ends where it began has two caps there and
/// no join. A triangle of three lines, (0, 0) → (80, 0) → (40, 60) → (0, 0), 12 px wide
/// under `Miter` with `Butt` caps.
///
/// * At (80, 0), a real corner, the miter is drawn. Its quad is V (80, 0), A (80, -6),
///   M (91.2, -6), B (85, 3.3); the pixel centred (88.5, -5.5) is inside it.
/// * At (0, 0) the path starts and ends. A join there would be the mirror image, with
///   its point at (-11.2, -6); the pixel centred (-8.5, -5.5) is background, and so is
///   the one centred (-3.5, -2.5), just outside both square-cut ends.
///
/// The same triangle as a `CurvePolygon` IS closed, and has the join at (0, 0) too.
#[test]
fn canvas_poly_curve_that_returns_has_two_caps_not_a_join() {
    let tri =
        "[line(0.0, 0.0, 80.0, 0.0), line(80.0, 0.0, 40.0, 60.0), line(40.0, 60.0, 0.0, 0.0)]";
    let paint = stroke_paint(12.0, "join := canvas::JoinStyle.Miter");
    let run = scene(
        "canvas_curve_two_caps",
        &format!(
            "  canvas::present([wave(\"\", 200.0, 200.0, {tri}, {paint}), blob(\"\", 500.0, 200.0, [{tri}], canvas::FillRule.EvenOdd, {paint})])\n"
        ),
        "",
        &[],
    );
    assert_eq!(
        run.pixel(200 + 88, 200 - 6),
        WHITE,
        "chain: the miter at (80, 0)"
    );
    assert_eq!(
        run.pixel(200 - 9, 200 - 6),
        BLACK,
        "chain: no miter where it began"
    );
    assert_eq!(run.pixel(200 - 4, 200 - 3), BLACK, "chain: nor a round");
    assert_eq!(run.pixel(200 + 40, 200), WHITE, "chain: its first line");
    assert_eq!(
        run.pixel(500 + 88, 200 - 6),
        WHITE,
        "contour: the miter at (80, 0)"
    );
    assert_eq!(
        run.pixel(500 - 9, 200 - 6),
        WHITE,
        "contour: and the one at (0, 0)"
    );
}

/// Dashes run along the curve by arc length. A quarter circle of radius 100 as two
/// 45° arcs, dashed `[10, 5]` with a 6 px stroke: the pattern is on over arc lengths
/// [0, 10), [15, 25), [30, 40) … and off between. The point at arc length `s` is at
/// angle `s / 100`. A pixel holding the middle of a dash is white and one holding the
/// middle of a gap is black — and the dash [75, 85) runs unbroken across the place the
/// first arc ends and the second begins, at 78.54.
#[test]
fn canvas_poly_curve_dashes_follow_arc_length() {
    let k = (std::f64::consts::PI / 8.0).cos();
    let c = SQRT_HALF * 100.0;
    // Each 45° arc's control point is where its end tangents meet: at radius
    // 100 / cos(22.5°), half-way round the arc.
    let (m1, m2) = (
        (100.0, 100.0 * (std::f64::consts::PI / 8.0).tan()),
        (100.0 * (std::f64::consts::PI / 8.0).tan(), 100.0),
    );
    let arcs = format!(
        "[conic(100.0, 0.0, {:?}, {:?}, {c:?}, {c:?}, {k:?}), conic({c:?}, {c:?}, {:?}, {:?}, 0.0, 100.0, {k:?})]",
        m1.0, m1.1, m2.0, m2.1
    );
    let run = scene(
        "canvas_curve_dashes",
        &format!(
            "  canvas::present([wave(\"\", 200.0, 100.0, {arcs}, {})])\n",
            stroke_paint(6.0, "dashes := [10.0, 5.0]")
        ),
        "",
        &[],
    );
    let at = |s: f64| {
        pixel_of(
            200.0 + 100.0 * (s / 100.0).cos(),
            100.0 + 100.0 * (s / 100.0).sin(),
        )
    };
    for dash in 0..10 {
        let start = dash as f64 * 15.0;
        let (x, y) = at(start + 5.0);
        assert_eq!(
            run.pixel(x, y),
            WHITE,
            "the middle of the dash from {start}"
        );
        let (x, y) = at(start + 12.5);
        assert_eq!(run.pixel(x, y), BLACK, "the middle of the gap after it");
    }
    for s in [76.0, 77.5, 78.54, 79.5, 81.0, 84.0] {
        let (x, y) = at(s);
        assert_eq!(
            run.pixel(x, y),
            WHITE,
            "arc length {s}, across the curve boundary"
        );
    }
    // The path is 157.08 long: its last dash is [150, 157.08), cut by the end.
    let (x, y) = at(153.0);
    assert_eq!(run.pixel(x, y), WHITE, "the last dash");
}

/// A `CurvePolygon`'s dashes restart at each contour, as a polygon's do at each ring.
/// Two circles of radius 60, dashed `[20, 10]`. Each starts at its rightmost point and
/// runs down the screen. The first is 376.99 long, so a pattern carried on into the
/// second would be 17 into its period there: at 5 px along the second it would be at
/// 22 (off), and at 25 px along at 12 (on). Restarted, 5 is on and 25 is off.
#[test]
fn canvas_curve_polygon_dashes_restart_at_each_contour() {
    let run = scene(
        "canvas_curve_dash_restart",
        &format!(
            "  canvas::present([blob(\"\", 0.0, 0.0, [circle(200.0, 200.0, 60.0), circle(500.0, 200.0, 60.0)], canvas::FillRule.EvenOdd, {})])\n",
            stroke_paint(6.0, "dashes := [20.0, 10.0]")
        ),
        "",
        &[],
    );
    let at = |cx: f64, s: f64| {
        pixel_of(
            cx + 60.0 * (s / 60.0).cos(),
            200.0 + 60.0 * (s / 60.0).sin(),
        )
    };
    for (cx, what) in [(200.0, "first"), (500.0, "second")] {
        let (x, y) = at(cx, 5.0);
        assert_eq!(
            run.pixel(x, y),
            WHITE,
            "{what} contour: 5 px along, in its first dash"
        );
        let (x, y) = at(cx, 25.0);
        assert_eq!(
            run.pixel(x, y),
            BLACK,
            "{what} contour: 25 px along, in its first gap"
        );
        let (x, y) = at(cx, 40.0);
        assert_eq!(
            run.pixel(x, y),
            WHITE,
            "{what} contour: 40 px along, in its second dash"
        );
    }
}

// ---- plan-164-B Phase 3: placement, the paint, and the hit region --------------------

/// A bar with a round far end, as one contour: (0, -10) → (100, -10), a half circle of
/// radius 10 about (100, 0), → (0, 10), closed by the implied segment. And an arch: one
/// quadratic from (0, 0) through a control point at (50, -60) to (100, 0), whose
/// highest point is (50, -30).
const SHAPES: &str = r#"
FUNC bar() AS List OF canvas::BezierCurve
  LET k AS Float = 0.7071067811865476
  RETURN [line(0.0, 0.0 - 10.0, 100.0, 0.0 - 10.0), conic(100.0, 0.0 - 10.0, 110.0, 0.0 - 10.0, 110.0, 0.0, k), conic(110.0, 0.0, 110.0, 10.0, 100.0, 10.0, k), line(100.0, 10.0, 0.0, 10.0)]
END FUNC

FUNC arch() AS List OF canvas::BezierCurve
  RETURN [canvas::BezierCurve[points := [at(0.0, 0.0), at(50.0, 0.0 - 60.0), at(100.0, 0.0)], weights := []]]
END FUNC
"#;

/// A shape's own `transform` turns it about its `position`. A 30° turn (cos 0.8660254,
/// sin 0.5; `x' = a x + c y`, `y' = b x + d y`, so a = d = cos, b = sin, c = -sin).
///
/// * The bar at (300, 300): its round end's centre, local (100, 0), lands at
///   (386.6, 350) and is filled; where that point was before the turn, (400, 300), is
///   local (86.6, -50) — 40 px off the bar — and is empty.
/// * The arch at (600, 300), 8 px wide: its highest point, local (50, -30), lands at
///   (658.3, 299.0) and is stroked; where it was, (650, 270), is local (28.3, -51) and
///   is empty.
#[test]
fn canvas_curve_shapes_turn_about_their_position() {
    let run = scene(
        "canvas_curve_turn",
        r#"  LET turn AS canvas::Transform = canvas::Transform[0.8660254037844386, 0.5, 0.0 - 0.5, 0.8660254037844386, 0.0, 0.0]
  LET a AS canvas::DrawItem = canvas::CurvePolygon[id := "", position := at(300.0, 300.0), contours := [bar()], rule := canvas::FillRule.EvenOdd, paint := canvas::fill(color::rgb(0, 0, 255)), transform := turn]
  LET b AS canvas::DrawItem = canvas::PolyCurve[id := "", position := at(600.0, 300.0), curves := arch(), paint := canvas::stroke(color::rgb(255, 255, 255), 8.0), transform := turn]
  canvas::present([a, b])
"#,
        SHAPES,
        &[],
    );
    assert_eq!(run.pixel(386, 350), BLUE, "the bar's round end, turned");
    assert_eq!(run.pixel(343, 325), BLUE, "the bar's middle, local (50, 0)");
    assert_eq!(run.pixel(400, 300), BLACK, "where the round end was");
    assert_eq!(
        run.pixel(300, 300),
        BLUE,
        "the pivot is on the bar's near end"
    );
    assert_eq!(
        run.pixel(658, 299),
        WHITE,
        "the arch's highest point, turned"
    );
    assert_eq!(run.pixel(650, 270), BLACK, "where it was");
    assert_eq!(run.stat("geoVerifyMismatches"), "0");
}

/// A group places what it holds, and a group that turns and scales it scales its
/// stroke with it.
///
/// * `plain`, used at (100, 100): the bar and the arch are drawn 100 px right and down
///   of where the group holds them.
/// * `big`, used at (500, 200) under `x' = -2y, y' = 2x` — a quarter turn, twice the
///   size. It holds the arch (6 px stroke) at its origin: the arch's highest point,
///   local (50, -30), lands at (560, 300), where the curve runs down the screen. Its
///   stroke is 12 px wide there: 5 px either side is stroked and 8 px is not — at the
///   6 px it has in the group, 5 px would not be.
///   It also holds a ring, a circle of radius 30 about (0, -80) with a 6 px stroke and
///   no fill: on the surface a circle of radius 60 about (660, 200), stroked 12 wide.
#[test]
fn canvas_curve_shapes_in_groups() {
    let run = scene(
        "canvas_curve_groups",
        r#"  canvas::setGroup("plain", [blob("", 50.0, 50.0, [bar()], canvas::FillRule.EvenOdd, canvas::fill(color::rgb(0, 0, 255))), wave("", 50.0, 150.0, arch(), canvas::stroke(color::rgb(255, 255, 255), 8.0))])
  canvas::setGroup("big", [wave("", 0.0, 0.0, arch(), canvas::stroke(color::rgb(255, 255, 255), 6.0)), blob("", 0.0, 0.0 - 80.0, [circle(0.0, 0.0, 30.0)], canvas::FillRule.EvenOdd, canvas::stroke(color::rgb(255, 255, 255), 6.0))])
  LET one AS canvas::DrawItem = canvas::Group[id := "", position := at(100.0, 100.0), transform := none(), name := "plain"]
  LET two AS canvas::DrawItem = canvas::Group[id := "", position := at(500.0, 200.0), transform := canvas::Transform[0.0, 2.0, 0.0 - 2.0, 0.0, 0.0, 0.0], name := "big"]
  canvas::present([one, two])
"#,
        SHAPES,
        &[],
    );
    // `plain`: the bar's middle is local (50, 0) of an item at (50, 50) of a group at
    // (100, 100); the arch's top is local (50, -30) of an item at (50, 150).
    assert_eq!(run.pixel(200, 150), BLUE, "the bar, moved by the group");
    assert_eq!(
        run.pixel(100, 50),
        BLACK,
        "where the bar is in the group's own space"
    );
    assert_eq!(run.pixel(200, 220), WHITE, "the arch, moved by the group");
    // `big`: the arch's top.
    assert_eq!(run.pixel(560, 300), WHITE, "the arch's highest point");
    assert_eq!(
        run.pixel(565, 300),
        WHITE,
        "5 px to one side: inside a 12 px stroke"
    );
    assert_eq!(run.pixel(554, 300), WHITE, "5 px to the other");
    assert_eq!(run.pixel(568, 300), BLACK, "8 px to one side: outside it");
    assert_eq!(run.pixel(551, 300), BLACK, "8 px to the other");
    // `big`: the ring, radius 60 about (660, 200).
    assert_eq!(run.pixel(660 + 60, 200), WHITE, "on the ring");
    assert_eq!(
        run.pixel(660 + 64, 200),
        WHITE,
        "4.5 px outside its centre line"
    );
    assert_eq!(run.pixel(660 + 55, 200), WHITE, "4.5 px inside it");
    assert_eq!(run.pixel(660 + 68, 200), BLACK, "8.5 px outside");
    assert_eq!(run.pixel(660 + 51, 200), BLACK, "8.5 px inside");
    assert_eq!(run.pixel(660, 200), BLACK, "the unfilled middle");
}

/// `paint.clip` is in the shape's own space and cuts both shapes. A clip that keeps
/// local x from 50 to 200: the bar at (100, 100) keeps its right half, and the arch at
/// (100, 300) its right half.
#[test]
fn canvas_curve_shapes_are_clipped_in_their_own_space() {
    let run = scene(
        "canvas_curve_clip",
        r#"  LET ring AS List OF List OF vector::Float2 = [[at(50.0, 0.0 - 100.0), at(200.0, 0.0 - 100.0), at(200.0, 100.0), at(50.0, 100.0)]]
  canvas::present([blob("", 100.0, 100.0, [bar()], canvas::FillRule.EvenOdd, WITH canvas::fill(color::rgb(0, 0, 255)) { clip := ring }), wave("", 100.0, 300.0, arch(), WITH canvas::stroke(color::rgb(255, 255, 255), 8.0) { clip := ring })])
"#,
        SHAPES,
        &[],
    );
    assert_eq!(
        run.pixel(100 + 75, 100),
        BLUE,
        "the bar, right of the clip's edge"
    );
    assert_eq!(run.pixel(100 + 105, 100), BLUE, "its round end");
    assert_eq!(run.pixel(100 + 25, 100), BLACK, "the bar, left of it");
    assert_eq!(run.pixel(100 + 48, 100), BLACK, "1.5 px left of the edge");
    assert_eq!(run.pixel(100 + 51, 100), BLUE, "1.5 px right of it");
    // The arch is at height -22.5 (y = -120 t(1 - t)) at x = 25 and x = 75.
    assert_eq!(
        run.pixel(100 + 75, 300 - 23),
        WHITE,
        "the arch, right of the clip's edge"
    );
    assert_eq!(run.pixel(100 + 25, 300 - 23), BLACK, "the arch, left of it");
}

/// `transparency`, and a gradient on the fill and on the stroke, are the paint's own
/// code: a curve shape under a paint draws what the shape it is the curved form of
/// draws under the same paint.
///
/// * A `CurvePolygon` circle beside a `canvas::Circle`, both under a half-transparent
///   fill and a radial gradient that also paints the stroke: the two 220x220 blocks are
///   equal pixel for pixel.
/// * A `PolyCurve` of one straight curve beside a `canvas::Line`, both under a linear
///   gradient applied to the stroke: equal pixel for pixel.
#[test]
fn canvas_curve_shapes_take_the_whole_paint() {
    let run = scene(
        "canvas_curve_paint",
        r#"  LET stops AS List OF canvas::GradientStop = [canvas::GradientStop[offset := 0.0, color := color::rgb(255, 0, 0)], canvas::GradientStop[offset := 1.0, color := color::rgb(0, 0, 255)]]
  LET radial AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Radial, startPoint := at(0.0 - 20.0, 0.0 - 10.0), endPoint := at(70.0, 0.0 - 10.0), stops := stops, spread := canvas::GradientSpread.Pad, transform := none(), focalOffset := at(0.0, 0.0), focalRadius := 0.0, strokeApply := canvas::GradientApply.FillAndStroke]
  LET disc AS canvas::Paint = WITH canvas::fillStroke(color::rgb(0, 0, 0), color::rgb(255, 255, 255), 8.0) { gradient := radial, transparency := 0.5 }
  LET linear AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, startPoint := at(0.0, 0.0), endPoint := at(160.0, 0.0), stops := stops, spread := canvas::GradientSpread.Pad, transform := none(), focalOffset := at(0.0, 0.0), focalRadius := 0.0, strokeApply := canvas::GradientApply.FillAndStroke]
  LET ink AS canvas::Paint = WITH canvas::stroke(color::rgb(255, 255, 255), 10.0) { gradient := linear }
  LET c AS canvas::DrawItem = canvas::Circle[id := "", position := at(150.0, 150.0), radius := 80.0, paint := disc, transform := none()]
  LET l AS canvas::DrawItem = canvas::Line[id := "", position := at(70.0, 400.0), startAt := at(0.0, 0.0), endAt := at(160.0, 50.0), paint := ink, transform := none()]
  canvas::present([c, blob("", 550.0, 150.0, [circle(0.0, 0.0, 80.0)], canvas::FillRule.EvenOdd, disc), l, wave("", 470.0, 400.0, [line(0.0, 0.0, 160.0, 50.0)], ink)])
"#,
        "",
        &[],
    );
    let (r, _, b, _) = run.pixel(150, 150);
    assert!(
        r > 0 && b > 0,
        "the circle's middle is in the ramp, faded: {:?}",
        run.pixel(150, 150)
    );
    for (what, x0, y0, x1, y1) in [("circle", 40, 40, 260, 260), ("line", 50, 380, 250, 470)] {
        let mut painted = 0;
        for y in y0..y1 {
            for x in x0..x1 {
                let a = run.pixel(x, y);
                assert_eq!(a, run.pixel(x + 400, y), "{what}: the pixel at ({x}, {y})");
                if a != BLACK {
                    painted += 1;
                }
            }
        }
        assert!(
            painted > 1000,
            "{what}: the block compared holds the shape ({painted})"
        );
    }
}

/// The hit region of both shapes (`mfb spec app canvas`, "The region of an item").
///
/// * `solid`, a filled `CurvePolygon` circle of radius 60 at (150, 150): inside and not
///   outside.
/// * `hollow`, the same at (400, 150) with a 4 px stroke and NO fill: deep in the
///   unpainted interior is still the shape. Its stroke reaches radius 62.
/// * `thin`, a `PolyCurve` of one cubic at (100, 400) with a 1 px stroke — the cubic
///   from (0, 0) to (200, 0) with both control points 80 below, whose lowest point is
///   (100, 60): hit 2 px from the curve, not 5 px from it, and not in the area between
///   the curve and its chord, which a closed shape would count as inside.
/// * `cut`, a filled circle at (650, 150) whose clip keeps its right half: not hit
///   where the clip cuts it away.
/// * `btn::face`, a `CurvePolygon` inside a group used under the id `btn`.
/// * an unnamed `CurvePolygon` over `under`: it has no `id`, and still blocks.
#[cfg(unix)]
#[test]
fn canvas_curve_shapes_are_hit_where_they_are_drawn() {
    let expected: [((u32, u32), &str); 16] = [
        ((150, 150), "solid"),
        ((150 + 58, 150), "solid"),
        ((150 + 62, 150), ""),
        ((400, 150), "hollow"),
        ((400 + 61, 150), "hollow"),
        ((400 + 64, 150), ""),
        ((100 + 100, 400 + 60), "thin"),
        ((100 + 100, 400 + 62), "thin"),
        ((100 + 100, 400 + 65), ""),
        ((100 + 100, 400 + 25), ""),
        ((650 + 30, 150), "cut"),
        ((650 - 30, 150), ""),
        ((600 + 20, 420), "btn::face"),
        ((600 + 75, 420), ""),
        ((300 + 20, 560), ""),
        ((300 + 80, 560), "under"),
    ];
    let points: Vec<(u32, u32)> = expected.iter().map(|(p, _)| *p).collect();
    let run = scene(
        "canvas_curve_hit",
        r#"  LET grey AS canvas::Paint = canvas::fill(color::rgb(200, 200, 200))
  LET half AS List OF List OF vector::Float2 = [[at(0.0, 0.0 - 100.0), at(100.0, 0.0 - 100.0), at(100.0, 100.0), at(0.0, 100.0)]]
  canvas::setGroup("button", [blob("face", 0.0, 0.0, [circle(0.0, 0.0, 40.0)], canvas::FillRule.EvenOdd, grey)])
  LET use AS canvas::DrawItem = canvas::Group[id := "btn", position := at(600.0, 420.0), transform := none(), name := "button"]
  MUT under AS canvas::Rectangle
  under = WITH under { id := "under", position := at(260.0, 520.0), size := canvas::Size[width := 160.0, height := 80.0], paint := grey }
  LET underItem AS canvas::DrawItem = under
  canvas::present([blob("solid", 150.0, 150.0, [circle(0.0, 0.0, 60.0)], canvas::FillRule.EvenOdd, grey), blob("hollow", 400.0, 150.0, [circle(0.0, 0.0, 60.0)], canvas::FillRule.EvenOdd, canvas::stroke(color::rgb(200, 200, 200), 4.0)), wave("thin", 100.0, 400.0, [cubic(0.0, 0.0, 0.0, 80.0, 200.0, 80.0, 200.0, 0.0)], canvas::stroke(color::rgb(200, 200, 200), 1.0)), blob("cut", 650.0, 150.0, [circle(0.0, 0.0, 60.0)], canvas::FillRule.EvenOdd, WITH grey { clip := half }), use, underItem, blob("", 300.0, 560.0, [circle(0.0, 0.0, 30.0)], canvas::FillRule.EvenOdd, grey)])
  report("over")
"#,
        "",
        &points,
    );
    let got = run.over();
    assert_eq!(got.len(), expected.len(), "{}", run.stdout);
    for (answer, ((x, y), want)) in got.iter().zip(&expected) {
        assert_eq!(answer, want, "overID at ({x}, {y})");
    }
}
