//! plan-176-D: `rt_canvas_rasteriser`'s joins, dashes (ellipses, lines, rings, rounded
//! rectangles and arcs) and caps. The render helpers live in the root,
//! `rt_canvas_rasteriser.rs`.

use super::*;

/// plan-163-B: `Paint.join` at a corner, in the software renderer — the oracle, so every
/// expectation is a hand-computed fact about a white 20 px stroke (`half` = 10) over
/// black, at pixel centres at least half a pixel inside or outside each shape.
///
/// * **A 90° polygon corner.** Each square's top-left vertex V has the bands' outer
///   corners A = V − (10, 0) and B = V − (0, 10):
///   - `Round` (today's join) is the disc of radius 10 about V: the centre (−6.5, −6.5)
///     from V is 9.19 away, covered; (−8.5, −8.5) is 12.02 away, not;
///   - `Miter` adds the square V, A, (V − (10, 10)), B: (−8.5, −8.5) and (−9.5, −9.5)
///     are inside it, (−10.5, −10.5) is not;
///   - `Bevel` adds the triangle V, A, B, the half-plane `x + y ≥ −10` from V:
///     (−4.5, −4.5) is inside by 0.71, (−5.5, −5.5) outside by 0.71 and (−6.5, −6.5)
///     outside by 2.12 — where the round join covers.
/// * **A 20° spike.** The apex V = (x, 300) of a triangle whose sides leave V at ±10°
///   from straight down: a miter would reach `10 / sin(10°) = 57.6` px above V, a ratio
///   of 5.76. Under `miterLimit := 4.0` that bevels — (x, 290) is 7.8 px above the
///   bevel's top edge, empty — and under `10.0` it miters — (x, 290) is inside the
///   spike, where it is 8.5 px wide each side, and so is (x, 250).
/// * **A PolyLine's interior corner** (100, 500) → (200, 500) → (200, 600), Butt ends:
///   V = (200, 500), A = (200, 490), B = (210, 500): the round join covers the centre
///   (206.5, 494.5) (8.5 from V) but not (209.5, 491.5) (12.75 from V); the bevel
///   covers (204.5, 496.5) and not (206.5, 494.5); the miter covers (209.5, 491.5). The
///   path's two ends are Butt and take no join: (99, 500) is empty under every join.
/// * **Rectangles**: a mitered one's corner is square — its top-left (−8.5, −8.5) is
///   covered — and a beveled one's is cut, as the polygon's.
#[test]
fn a_join_shapes_every_corner_of_a_stroke() {
    let corners = |x: f64, y: f64| -> String {
        format!(
            "[[vector::Float2[x := {x:?}, y := {y:?}], vector::Float2[x := {:?}, y := {y:?}], \
             vector::Float2[x := {:?}, y := {:?}], vector::Float2[x := {x:?}, y := {:?}]]]",
            x + 120.0,
            x + 120.0,
            y + 120.0,
            y + 120.0
        )
    };
    let spike = |x: f64| -> String {
        format!(
            "[[vector::Float2[x := {x:?}, y := 300.0], vector::Float2[x := {:?}, y := 447.72116], \
             vector::Float2[x := {:?}, y := 447.72116]]]",
            x + 26.04723,
            x - 26.04723
        )
    };
    let bend = |x: f64| -> String {
        format!(
            "[vector::Float2[x := {:?}, y := 500.0], vector::Float2[x := {:?}, y := 500.0], \
             vector::Float2[x := {:?}, y := 600.0]]",
            x,
            x + 100.0,
            x + 100.0
        )
    };
    let body = format!(
        "  LET ink AS canvas::Paint = canvas::stroke(color::rgb(255, 255, 255), 20.0)\n  \
         LET miter AS canvas::Paint = WITH ink {{ join := canvas::JoinStyle.Miter }}\n  \
         LET bevel AS canvas::Paint = WITH ink {{ join := canvas::JoinStyle.Bevel }}\n  \
         LET r1 AS canvas::DrawItem = canvas::Polygon[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], contours := {}, rule := canvas::FillRule.EvenOdd, paint := ink, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
         LET r2 AS canvas::DrawItem = canvas::Polygon[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], contours := {}, rule := canvas::FillRule.EvenOdd, paint := miter, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
         LET r3 AS canvas::DrawItem = canvas::Polygon[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], contours := {}, rule := canvas::FillRule.EvenOdd, paint := bevel, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
         LET s4 AS canvas::DrawItem = canvas::Polygon[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], contours := {}, rule := canvas::FillRule.EvenOdd, paint := miter, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
         LET s10 AS canvas::DrawItem = canvas::Polygon[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], contours := {}, rule := canvas::FillRule.EvenOdd, paint := WITH miter {{ miterLimit := 10.0 }}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
         LET b1 AS canvas::DrawItem = canvas::PolyLine[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], points := {}, colors := [], paint := ink, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
         LET b2 AS canvas::DrawItem = canvas::PolyLine[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], points := {}, colors := [], paint := miter, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
         LET b3 AS canvas::DrawItem = canvas::PolyLine[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], points := {}, colors := [], paint := bevel, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
         LET q1 AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 700.0, y := 100.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 120.0, height := 120.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := miter, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
         LET q2 AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 700.0, y := 300.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 120.0, height := 120.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := bevel, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
         canvas::present([r1, r2, r3, s4, s10, b1, b2, b3, q1, q2])\n",
        corners(100.0, 100.0),
        corners(300.0, 100.0),
        corners(500.0, 100.0),
        spike(400.0),
        spike(600.0),
        bend(50.0),
        bend(300.0),
        bend(550.0),
    );
    let (frame, _) = render("canvas_joins", &scene(&body));
    let white = (255, 255, 255, 255);
    let black = (0, 0, 0, 255);
    // Round, V = (100, 100).
    assert_eq!(
        pixel(&frame, 93, 93),
        white,
        "a round join covers 9.19 from its vertex"
    );
    assert_eq!(pixel(&frame, 91, 91), black, "but not 12.02");
    // Miter, V = (300, 100).
    assert_eq!(
        pixel(&frame, 291, 91),
        white,
        "a miter fills the square corner"
    );
    assert_eq!(pixel(&frame, 290, 90), white, "to its point");
    assert_eq!(pixel(&frame, 289, 89), black, "and no further");
    // Bevel, V = (500, 100).
    assert_eq!(
        pixel(&frame, 495, 95),
        white,
        "a bevel covers inside its cut"
    );
    assert_eq!(pixel(&frame, 494, 94), black, "and stops at it");
    assert_eq!(
        pixel(&frame, 493, 93),
        black,
        "where a round join would cover"
    );
    // The 20° spike: limit 4 bevels, limit 10 miters.
    assert_eq!(
        pixel(&frame, 400, 290),
        black,
        "miterLimit 4 bevels a 20° spike"
    );
    assert_eq!(
        pixel(&frame, 400, 250),
        black,
        "so nothing reaches up the spike"
    );
    assert_eq!(pixel(&frame, 600, 290), white, "miterLimit 10 miters it");
    assert_eq!(pixel(&frame, 600, 250), white, "up to its point");
    // PolyLine corners, V = (x + 100, 500) for x = 50, 300, 550.
    assert_eq!(pixel(&frame, 156, 494), white, "a round polyline turn");
    assert_eq!(pixel(&frame, 159, 491), black, "is round");
    assert_eq!(
        pixel(&frame, 409, 491),
        white,
        "a mitered polyline turn is square"
    );
    assert_eq!(
        pixel(&frame, 654, 496),
        white,
        "a beveled polyline turn covers inside its cut"
    );
    assert_eq!(pixel(&frame, 656, 494), black, "and not past it");
    for x in [49usize, 299, 549] {
        assert_eq!(
            pixel(&frame, x, 500),
            black,
            "a Butt end at x = {x} takes no join"
        );
    }
    // Rectangles.
    assert_eq!(
        pixel(&frame, 691, 91),
        white,
        "a mitered rectangle's corner is square"
    );
    assert_eq!(pixel(&frame, 689, 89), black, "and ends at its point");
    assert_eq!(
        pixel(&frame, 695, 295),
        white,
        "a beveled rectangle covers inside its cut"
    );
    assert_eq!(pixel(&frame, 693, 293), black, "and not past it");
}

/// plan-163-B: a dash that crosses a corner stays one dash round it — the premise the
/// dash design rests on (§8), for each cap.
///
/// Three 120 px squares (first point the top-left, so `s = 0` there, going clockwise)
/// stroked 20 px wide (`half` = 10) with `dashes := [150.0, 30.0]`: the perimeter is
/// 480, so the dashes are `s` ∈ [0, 150) — the top side and 30 px down the right side —
/// [180, 330) — the right side's last 60 px and 90 px of the bottom side, from its right
/// end — and [360, 510), which starts EXACTLY at the bottom-left corner and runs up the
/// left side to the seam, where it meets the first dash: on at both sides of the seam,
/// the two are one dash through the top-left corner. With the square at X:
///
/// * the top-right corner (X + 120, 100) is inside the first dash, so it takes its
///   (Round) join: the centre (X + 125.5, 95.5) is 7.1 from it, covered; the inner corner
///   (X + 112.5, 108.5) is inside both sides' bands;
/// * the right side's gap is y ∈ [130, 160): its centre row 145.5 is empty under every
///   cap — 15.5 and 14.5 from the two dash ends, beyond a Round disc and a Square's 10;
/// * at the dash end y = 130: row 129.5 (`along` = −0.5) is covered under every cap; row
///   131.5 only under Round (1.58 from the end point (X + 120, 130)) and Square (1.5 past
///   the end, within 10); row 137.5 under Round (7.5) and Square; row 139.5 under Square
///   alone (9.5 past, coverage exactly 255) and row 141.5 under none;
/// * the bottom-left corner (X, 220) is where the third dash STARTS, so it takes no join:
///   the centre (X − 4.5, 225.5) — 5.5 below the corner, beside the left side's line — is
///   empty under Butt, inside the Round end's disc (7.1 from the corner), and inside the
///   Square end, which runs 10 straight down the left side's line from the corner;
/// * the bottom side's gap is x ∈ (X, X + 30]: x = X + 15.5 is empty under every cap,
///   and X + 25.5 — 4.5 past the second dash's end at X + 30 — is covered by its Round
///   disc and its Square extension, not by Butt;
/// * the seam corner (X, 100) is inside the merged dash, so it takes its join:
///   (X − 4.5, 95.5) is 6.4 from it.
#[test]
fn a_dash_crossing_a_corner_stays_one_dash() {
    let (frame, _) = render(
        "canvas_dash_corners",
        &(scene(
            "  LET ink AS canvas::Paint = WITH canvas::stroke(color::rgb(255, 255, 255), 20.0) { dashes := [150.0, 30.0] }\n  \
             LET butt AS canvas::DrawItem = canvas::Polygon[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], contours := [sq(100.0, 100.0, 120.0)], rule := canvas::FillRule.EvenOdd, paint := ink, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET round AS canvas::DrawItem = canvas::Polygon[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], contours := [sq(300.0, 100.0, 120.0)], rule := canvas::FillRule.EvenOdd, paint := WITH ink { cap := canvas::CapStyle.Round }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET square AS canvas::DrawItem = canvas::Polygon[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], contours := [sq(500.0, 100.0, 120.0)], rule := canvas::FillRule.EvenOdd, paint := WITH ink { cap := canvas::CapStyle.Square }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([butt, round, square])\n",
        ) + RING_HELPERS),
    );
    let white = (255, 255, 255, 255);
    let black = (0, 0, 0, 255);
    for (x, cap) in [(100usize, "Butt"), (300, "Round"), (500, "Square")] {
        let on = |px: usize, py: usize, what: &str| {
            assert_eq!(pixel(&frame, px, py), white, "{cap}: {what}");
        };
        let off = |px: usize, py: usize, what: &str| {
            assert_eq!(pixel(&frame, px, py), black, "{cap}: {what}");
        };
        on(
            x + 125,
            95,
            "the dash crossing the top-right corner takes its join",
        );
        on(x + 112, 108, "and the corner's inner side");
        off(x + 120, 145, "the right side's gap");
        on(x + 120, 129, "the dash runs to its end");
        on(x - 5, 95, "the seam corner joins the dash through it");
        off(x + 15, 220, "the bottom side's gap");
        if cap == "Butt" {
            off(x + 120, 131, "a butt dash end stops at the end");
            off(x + 120, 137, "a butt dash end does not reach 7.5 past");
            off(x - 5, 225, "a dash starting at a corner takes no join");
            off(x + 25, 220, "a butt end leaves the gap");
        }
        if cap == "Round" {
            on(x + 120, 131, "a round dash end's disc");
            on(x + 120, 137, "reaches 7.5 past the end");
            off(x + 120, 141, "not 11.5");
            on(
                x - 5,
                225,
                "the dash starting at the corner takes its round cap",
            );
            on(x + 25, 220, "the round end's disc reaches 4.5 into the gap");
        }
        if cap == "Square" {
            on(x + 120, 131, "a square dash end");
            on(x + 120, 139, "reaches 9.5 past the end");
            off(x + 120, 141, "not 11.5");
            on(
                x - 5,
                225,
                "the dash starting at the corner runs its square end straight on",
            );
            on(x + 25, 220, "the square end reaches 4.5 into the gap");
        }
    }
}

/// The arc length of an ellipse from parametric angle 0 to `t`, by Simpson's rule over
/// 200,000 steps — far finer than anything the renderer uses, so it serves as the exact
/// value the dash table is measured against.
fn ellipse_arc_length(rx: f64, ry: f64, t: f64) -> f64 {
    let n = 200_000;
    let h = t / n as f64;
    let f = |a: f64| (rx * rx * a.sin() * a.sin() + ry * ry * a.cos() * a.cos()).sqrt();
    let mut sum = f(0.0) + f(t);
    for i in 1..n {
        let w = if i % 2 == 1 { 4.0 } else { 2.0 };
        sum += w * f(i as f64 * h);
    }
    sum * h / 3.0
}

/// The parametric angle at which an ellipse's arc length from angle 0 reaches `s`.
fn ellipse_angle_at(rx: f64, ry: f64, s: f64) -> f64 {
    let (mut lo, mut hi) = (0.0f64, std::f64::consts::TAU);
    for _ in 0..50 {
        let mid = (lo + hi) / 2.0;
        if ellipse_arc_length(rx, ry, mid) < s {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (lo + hi) / 2.0
}

/// plan-163-B: a dashed Ellipse is cut by arc length, from angle 0 (+X) the way angles
/// increase (towards +Y), measured against an independent arc length
/// (`ellipse_arc_length`, not the renderer's table).
///
/// rx = 160, ry = 80, 8 px wide, `dashes := [30.0, 15.0]`: every dash's middle (`s` =
/// 15 + 45k) is drawn and every gap's middle (`s` = 37.5 + 45k) is not. The pixel tested
/// is the one holding the outline point, whose centre is within 0.71 of it: well inside
/// the 4 px half-width, and 7.5 or more along the outline from the nearest dash end.
#[test]
fn a_dashed_ellipse_is_cut_by_arc_length() {
    let (frame, _) = render(
        "canvas_dash_ellipse",
        &scene(
            "  LET e AS canvas::DrawItem = canvas::Ellipse[id := \"\", position := vector::Float2[x := 450.0, y := 320.0], radiusX := 160.0, radiusY := 80.0, paint := WITH canvas::stroke(color::rgb(255, 255, 255), 8.0) { dashes := [30.0, 15.0] }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([e])\n",
        ),
    );
    let (rx, ry) = (160.0, 80.0);
    let perimeter = ellipse_arc_length(rx, ry, std::f64::consts::TAU);
    let at = |s: f64| {
        let t = ellipse_angle_at(rx, ry, s);
        let x = 450.0 + rx * t.cos();
        let y = 320.0 + ry * t.sin();
        pixel(&frame, x.floor() as usize, y.floor() as usize)
    };
    let mut k = 0;
    while 37.5 + 45.0 * k as f64 + 7.5 < perimeter {
        let s = 15.0 + 45.0 * k as f64;
        assert_eq!(
            at(s),
            (255, 255, 255, 255),
            "the middle of dash {k}, s = {s}"
        );
        let g = 37.5 + 45.0 * k as f64;
        assert_eq!(at(g), (0, 0, 0, 255), "the middle of gap {k}, s = {g}");
        k += 1;
    }
    assert!(
        k >= 16,
        "the ellipse's perimeter {perimeter} holds 16 periods"
    );
}

/// plan-163-B: the ellipse dash table's resolution, `__CANVAS_DASH_ELLIPSE_STEPS`,
/// chosen by measurement (§8 left it UNVERIFIED).
///
/// The table is the cumulative arc length at N + 1 evenly spaced parametric angles, each
/// step Simpson's rule over 8 sub-steps, and a pixel's position is the linear
/// interpolation between two entries. This reproduces that construction exactly (in f64)
/// and measures its worst error against `ellipse_arc_length` at 16 points per step,
/// for ellipses up to 800 px across and 80:1 eccentric.
#[test]
fn an_ellipse_dash_table_is_within_a_tenth_of_a_pixel() {
    const STEPS: usize = 256;
    let mut worst: f64 = 0.0;
    for (rx, ry) in [
        (400.0f64, 5.0f64),
        (400.0, 200.0),
        (200.0, 10.0),
        (50.0, 40.0),
        (8.0, 400.0),
    ] {
        let dt = std::f64::consts::TAU / STEPS as f64;
        let f = |a: f64| (rx * rx * a.sin() * a.sin() + ry * ry * a.cos() * a.cos()).sqrt();
        let mut table = vec![0.0f64];
        for k in 0..STEPS {
            let t0 = k as f64 * dt;
            let mut sum = 0.0;
            for j in 0..=8 {
                let w = if j == 0 || j == 8 {
                    1.0
                } else if j % 2 == 1 {
                    4.0
                } else {
                    2.0
                };
                sum += w * f(t0 + j as f64 * dt / 8.0);
            }
            table.push(table[k] + sum * dt / 24.0);
        }
        // Exact arc length at the probe points, by integrating step to step.
        for k in 0..STEPS {
            for i in 1..16 {
                let t = (k as f64 + i as f64 / 16.0) * dt;
                let frac = i as f64 / 16.0;
                let interpolated = table[k] + (table[k + 1] - table[k]) * frac;
                let exact = ellipse_arc_length(rx, ry, t);
                worst = worst.max((interpolated - exact).abs());
            }
        }
    }
    println!("ellipse dash table, {STEPS} steps: worst along-path error {worst:.5} px");
    assert!(worst < 0.1, "the worst along-path error is {worst} px");
}

/// plan-163-B: `Paint.dashes` on a Line, in the software renderer — the pattern, its
/// normalization (§4) and `dashOffset`.
///
/// Each line runs from x = 100 to x = 300 along a row centre (y = r + 0.5), 4 px wide,
/// Butt: a pixel at column 100 + i is drawn exactly when `s` = i + 0.5 is in a dash —
/// every dash boundary here is a whole number, so every centre is half a pixel from one.
///
/// * `[10, 5]`: on for `s` mod 15 ∈ [0, 10);
/// * `[10, 5, 5]` is odd, so it repeats once to `[10, 5, 5, 10, 5, 5]`: on for `s` mod 40
///   ∈ [0, 10) ∪ [15, 20) ∪ [30, 35);
/// * `[]`, `[0, 0]` (sum 0) and `[10, -5]` (a negative entry) are solid;
/// * `dashOffset := 3` starts 3 into `[10, 5]`: on for (`s` + 3) mod 15 ∈ [0, 10);
///   `dashOffset := -3` is 12 into it.
#[test]
fn a_dashed_line_draws_its_pattern() {
    let rows: [(&str, usize); 7] = [
        ("[10.0, 5.0]", 40),
        ("[10.0, 5.0, 5.0]", 60),
        ("[]", 80),
        ("[0.0, 0.0]", 100),
        ("[10.0, 0.0 - 5.0]", 120),
        ("[10.0, 5.0]", 140),
        ("[10.0, 5.0]", 160),
    ];
    let offsets = ["0.0", "0.0", "0.0", "0.0", "0.0", "3.0", "0.0 - 3.0"];
    let mut body = String::from(
        "  LET ink AS canvas::Paint = canvas::stroke(color::rgb(255, 255, 255), 4.0)\n",
    );
    let mut names = Vec::new();
    for (i, ((dashes, y), offset)) in rows.iter().zip(offsets).enumerate() {
        body.push_str(&format!(
            "  LET l{i} AS canvas::DrawItem = canvas::Line[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], startAt := vector::Float2[x := 100.0, y := {y}.5], endAt := vector::Float2[x := 300.0, y := {y}.5], paint := WITH ink {{ dashes := {dashes}, dashOffset := {offset} }}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n"
        ));
        names.push(format!("l{i}"));
    }
    body.push_str(&format!("  canvas::present([{}])\n", names.join(", ")));
    let (frame, _) = render("canvas_dash_line", &scene(&body));
    let white = (255, 255, 255, 255);
    let black = (0, 0, 0, 255);
    let expect = |y: usize, what: &str, on: &dyn Fn(f64) -> bool| {
        for i in 0..200usize {
            let s = i as f64 + 0.5;
            let want = if on(s) { white } else { black };
            assert_eq!(
                pixel(&frame, 100 + i, y),
                want,
                "{what}: column {}, s = {s}",
                100 + i
            );
        }
    };
    expect(40, "[10, 5]", &|s| s % 15.0 < 10.0);
    expect(60, "[10, 5, 5] repeated once", &|s| {
        let w = s % 40.0;
        w < 10.0 || (15.0..20.0).contains(&w) || (30.0..35.0).contains(&w)
    });
    expect(80, "an empty list is solid", &|_| true);
    expect(100, "a zero-sum list is solid", &|_| true);
    expect(120, "a negative entry is solid", &|_| true);
    expect(140, "dashOffset 3", &|s| (s + 3.0) % 15.0 < 10.0);
    expect(160, "dashOffset -3", &|s| (s + 12.0) % 15.0 < 10.0);
}

/// plan-163-B: a dash end takes the paint's cap, and so do a dashed path's own ends.
///
/// Lines from x = 100 to 300 at y = 50, 100, 150, 8 px wide (`half` = 4), `[10.0, 10.0]`:
/// the first dash is [100, 110). The centre (113.5, 46.5) is 3.5 past that dash's end and
/// 3.5 off the line: within a Square end (both under 4) but 4.95 from the end point,
/// outside a Round one; Butt draws neither. (111.5, 50.5) is 1.5 past the end — inside
/// Round and Square. The path's start x = 100 is a dash end too: (96.5, 150.5) is 3.5
/// before it, inside a Square end, and (97.5, 100.5) is 2.55 from it, inside a Round one;
/// Butt draws neither.
#[test]
fn a_dash_end_takes_the_cap() {
    let (frame, _) = render(
        "canvas_dash_caps",
        &scene(
            "  LET ink AS canvas::Paint = WITH canvas::stroke(color::rgb(255, 255, 255), 8.0) { dashes := [10.0, 10.0] }\n  \
             LET butt AS canvas::DrawItem = canvas::Line[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], startAt := vector::Float2[x := 100.0, y := 50.0], endAt := vector::Float2[x := 300.0, y := 50.0], paint := ink, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET round AS canvas::DrawItem = canvas::Line[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], startAt := vector::Float2[x := 100.0, y := 100.0], endAt := vector::Float2[x := 300.0, y := 100.0], paint := WITH ink { cap := canvas::CapStyle.Round }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET square AS canvas::DrawItem = canvas::Line[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], startAt := vector::Float2[x := 100.0, y := 150.0], endAt := vector::Float2[x := 300.0, y := 150.0], paint := WITH ink { cap := canvas::CapStyle.Square }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([butt, round, square])\n",
        ),
    );
    let white = (255, 255, 255, 255);
    let black = (0, 0, 0, 255);
    assert_eq!(
        pixel(&frame, 111, 50),
        black,
        "a butt dash stops at its end"
    );
    assert_eq!(pixel(&frame, 113, 46), black, "and has no corner past it");
    assert_eq!(pixel(&frame, 96, 50), black, "nor before the path's start");
    assert_eq!(
        pixel(&frame, 111, 100),
        white,
        "a round dash end reaches past it"
    );
    assert_eq!(pixel(&frame, 113, 96), black, "as a disc");
    assert_eq!(
        pixel(&frame, 97, 100),
        white,
        "and the path's start is capped round"
    );
    assert_eq!(
        pixel(&frame, 111, 150),
        white,
        "a square dash end reaches past it"
    );
    assert_eq!(pixel(&frame, 113, 146), white, "with a square corner");
    assert_eq!(
        pixel(&frame, 96, 150),
        white,
        "and the path's start is capped square"
    );
}

/// plan-163-B: each Polygon ring restarts the pattern at its own first point, and a
/// dashed Circle's `s = 0` is its +X point, running clockwise (towards +Y).
///
/// * Two square rings, sides 27.5 (perimeter 110) at (100, 300) and (200, 300), 4 px
///   wide, `[10.0, 10.0]`: each ring's top side is on for x ∈ [first.x, first.x + 10).
///   Had the second ring continued the first's pattern, it would start at `s` = 110,
///   mid-gap. The centre (205.5, 300.5) is 5.5 along the second ring.
/// * A Circle at (600, 300), radius 50, 6 px wide, `[20.0, 300.0]` (one dash per
///   perimeter of 314.16): the dash covers angles [0, 0.4) clockwise from +X. The centre
///   (648.5, 309.5) is at angle 0.194 (`s` = 9.7) and radius 49.42; (648.5, 290.5) is at
///   angle −0.194, `s` = 304.4, mid-gap.
#[test]
fn each_ring_restarts_the_pattern_and_a_circle_starts_at_plus_x() {
    let (frame, _) = render(
        "canvas_dash_rings",
        &format!(
            "{}{RING_HELPERS}",
            scene(
                "  LET ink AS canvas::Paint = WITH canvas::stroke(color::rgb(255, 255, 255), 4.0) { dashes := [10.0, 10.0] }\n  \
                 LET rings AS canvas::DrawItem = canvas::Polygon[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], contours := [sq(100.0, 300.0, 27.5), sq(200.0, 300.0, 27.5)], rule := canvas::FillRule.EvenOdd, paint := ink, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
                 LET disc AS canvas::DrawItem = canvas::Circle[id := \"\", position := vector::Float2[x := 600.0, y := 300.0], radius := 50.0, paint := WITH canvas::stroke(color::rgb(255, 255, 255), 6.0) { dashes := [20.0, 300.0] }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
                 canvas::present([rings, disc])\n",
            )
        ),
    );
    let white = (255, 255, 255, 255);
    let black = (0, 0, 0, 255);
    assert_eq!(
        pixel(&frame, 105, 300),
        white,
        "the first ring starts with a dash"
    );
    assert_eq!(pixel(&frame, 115, 300), black, "then a gap");
    assert_eq!(
        pixel(&frame, 205, 300),
        white,
        "the second ring restarts with a dash"
    );
    assert_eq!(pixel(&frame, 215, 300), black, "then a gap");
    assert_eq!(
        pixel(&frame, 648, 309),
        white,
        "the circle's dash starts at +X, going clockwise"
    );
    assert_eq!(pixel(&frame, 648, 290), black, "not counter-clockwise");
    assert_eq!(pixel(&frame, 550, 300), black, "the far side is in the gap");
}

/// plan-163-B: a dashed Rectangle-with-radius's `s = 0` is the left end of its top straight side,
/// running clockwise, and a dashed Arc's is `startAngle`, running the way the sweep does.
///
/// * The Rectangle with radius at (100, 400), 200 x 100, corner radius 20, 4 px wide, `[30.0,
///   400.0]`: its top side runs x ∈ [120, 280) at y = 400, so the dash covers x ∈ [120,
///   150). The centre (125.5, 400.5) is at `s` = 5.5; (155.5, 400.5) at `s` = 35.5, in
///   the gap; (115.5, 400.5) is in the top-left corner's arc, `s` ≈ 565 of a 565.66
///   perimeter, in the gap too — the dash starts at the corner's end, it does not run
///   back round it.
/// * The Arc at (600, 450), radius 50, from π/2 (straight down) to π, 4 px wide, `[20.0,
///   200.0]`, Butt: the dash covers the first 20 px of the sweep, towards −X. The centre
///   (590.5, 499.5) is at radius 50.40, angle 1.7602, `s` = 9.47; (553.5, 468.5) is at
///   `s` = 59.6, in the gap; (609.5, 499.5) is 9.47 BEFORE the start, past the butt end.
#[test]
fn a_rounded_rect_and_an_arc_start_their_dashes_where_the_contract_says() {
    let (frame, _) = render(
        "canvas_dash_starts",
        &scene(
            "  LET ink AS canvas::Paint = canvas::stroke(color::rgb(255, 255, 255), 4.0)\n  \
             LET box AS canvas::DrawItem = canvas::Rectangle[id := \"\", position := vector::Float2[x := 100.0, y := 400.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 200.0, height := 100.0], radiusTL := 20.0, radiusTR := 20.0, radiusBL := 20.0, radiusBR := 20.0, paint := WITH ink { dashes := [30.0, 400.0] }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             LET bow AS canvas::DrawItem = canvas::Arc[id := \"\", position := vector::Float2[x := 600.0, y := 450.0], radius := 50.0, startAngle := 1.5707963267948966, endAngle := 3.141592653589793, paint := WITH ink { dashes := [20.0, 200.0] }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([box, bow])\n",
        ),
    );
    let white = (255, 255, 255, 255);
    let black = (0, 0, 0, 255);
    assert_eq!(
        pixel(&frame, 125, 400),
        white,
        "the rounded rectangle's dash starts on its top side"
    );
    assert_eq!(pixel(&frame, 155, 400), black, "and ends 30 along it");
    assert_eq!(
        pixel(&frame, 115, 400),
        black,
        "the corner before the start is the pattern's end, in the gap"
    );
    assert_eq!(
        pixel(&frame, 590, 499),
        white,
        "the arc's dash starts at startAngle"
    );
    assert_eq!(
        pixel(&frame, 553, 468),
        black,
        "and ends 20 along the sweep"
    );
    assert_eq!(
        pixel(&frame, 609, 499),
        black,
        "nothing is drawn before the start"
    );
}

/// A butt-capped line stops at its endpoint; the same line round-capped does not.
///
/// plan-116-D Phase 2. Asserted against **the same line** a few pixels apart, because
/// that difference is the only thing the cap changes — a weaker pair of assertions
/// would also pass on a renderer that ignored the flag entirely.
///
/// Horizontal, from `x = 200` to `x = 400` at `y = 300`, stroke width 20 (half-width
/// 10). Pixel centres sit at `x + 0.5`, so pixel 405's centre is 5.5 past the end
/// plane: outside for `Butt`, inside the end disc for `Round`.
#[test]
fn a_butt_cap_stops_at_the_endpoint_and_a_round_cap_does_not() {
    let line = |cap: &str| {
        format!(
            "  LET l AS canvas::DrawItem = canvas::Line[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], startAt := vector::Float2[x := 200.0, y := 300.0], endAt := vector::Float2[x := 400.0, y := 300.0], paint := WITH canvas::stroke(color::rgb(255, 255, 255), 20.0) {{ cap := canvas::CapStyle.{cap} }}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([l])\n"
        )
    };
    let (butt, _) = render("canvas_cap_butt", &scene(&line("Butt")));
    let (round, _) = render("canvas_cap_round", &scene(&line("Round")));

    // Both must draw the body, or every assertion below is vacuous: an item that was
    // dropped entirely would "pass" all the outside-the-cap checks.
    for (name, frame) in [("butt", &butt), ("round", &round)] {
        assert_eq!(
            pixel(frame, 300, 300),
            (255, 255, 255, 255),
            "the {name} line did not draw its own middle"
        );
        assert_eq!(
            pixel(frame, 399, 300),
            (255, 255, 255, 255),
            "the {name} line stops short of its own endpoint"
        );
    }

    assert_eq!(
        pixel(&butt, 405, 300),
        (0, 0, 0, 255),
        "past the endpoint is background for a butt cap — ink here is the round \
         distance, so the flag was not read"
    );
    assert_eq!(
        pixel(&round, 405, 300),
        (255, 255, 255, 255),
        "the same pixel is inside the round cap's disc (half-width 10), so a round \
         cap must still paint it"
    );
    assert_eq!(
        pixel(&round, 415, 300),
        (0, 0, 0, 255),
        "15 px past is outside even the round cap, so the disc has the stroke's \
         half-width and not some larger reach"
    );
}

/// A zero-length line is a dot when round-capped and nothing at all when butt-capped.
///
/// The degenerate case, and the one a `max` of three terms is most likely to get
/// wrong: with `len2 = 0` there is no direction for the two end planes to be
/// perpendicular to. Butt answers "outside everywhere" deliberately rather than
/// dividing by zero; Round keeps the pre-existing behaviour, where clamping `t` makes
/// the distance radial and the shape a disc.
///
/// Both halves are needed. Asserting only that the butt one is empty would also pass
/// if zero-length lines had stopped drawing altogether.
#[test]
fn a_zero_length_line_is_a_dot_only_when_round_capped() {
    let dot = |cap: &str| {
        format!(
            "  LET l AS canvas::DrawItem = canvas::Line[id := \"\", position := vector::Float2[x := 0.0, y := 0.0], startAt := vector::Float2[x := 300.0, y := 300.0], endAt := vector::Float2[x := 300.0, y := 300.0], paint := WITH canvas::stroke(color::rgb(255, 255, 255), 20.0) {{ cap := canvas::CapStyle.{cap} }}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([l])\n"
        )
    };
    let (butt, _) = render("canvas_cap_zero_butt", &scene(&dot("Butt")));
    let (round, _) = render("canvas_cap_zero_round", &scene(&dot("Round")));

    assert_eq!(
        pixel(&round, 300, 300),
        (255, 255, 255, 255),
        "a zero-length ROUND line is a disc of the stroke's half-width, so its centre \
         is painted — the behaviour that existed before plan-116-D"
    );
    assert_eq!(
        pixel(&round, 305, 300),
        (255, 255, 255, 255),
        "5 px from the centre is inside that disc (half-width 10)"
    );
    assert_eq!(
        pixel(&butt, 300, 300),
        (0, 0, 0, 255),
        "a zero-length BUTT line has no length for its end planes to bound, so it is \
         empty — ink at the centre means the degenerate case fell through to the \
         round distance"
    );
}

/// A round-capped arc puts a disc at each sweep end; a butt-capped one does not.
///
/// plan-116-D Phase 3, and the pair is what makes it a test: an `Arc` was *butt* before
/// this letter — the sweep test already cuts the band along a radius at each end — so
/// `Butt` is the byte-identical side here and `Round` is the new geometry. That is the
/// opposite of `Line`, and getting the two backwards is the mistake this letter is
/// shaped to prevent.
///
/// The arc is centred at (300, 300), radius 100, sweeping `0.0`..`PI` — so it runs
/// below the centre (Y grows downward) and its start endpoint is at (400, 300), exactly
/// the +X extreme. Stroke width 24, so a cap disc there has half-width 12.
#[test]
fn a_round_capped_arc_caps_its_sweep_ends_and_a_butt_one_does_not() {
    let arc = |cap: &str| {
        format!(
            "  LET a AS canvas::DrawItem = canvas::Arc[id := \"\", position := vector::Float2[x := 300.0, y := 300.0], radius := 100.0, startAngle := 0.0, endAngle := 3.141592653589793, paint := WITH canvas::stroke(color::rgb(255, 255, 255), 24.0) {{ cap := canvas::CapStyle.{cap} }}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([a])\n"
        )
    };
    let (butt, _) = render("canvas_arccap_butt", &scene(&arc("Butt")));
    let (round, _) = render("canvas_arccap_round", &scene(&arc("Round")));

    // The band itself, well inside the sweep — both must draw it, or everything below
    // is vacuous.
    for (name, frame) in [("butt", &butt), ("round", &round)] {
        assert_eq!(
            pixel(frame, 300, 400),
            (255, 255, 255, 255),
            "the {name} arc did not draw the bottom of its own band"
        );
    }

    // Seven pixels above the start endpoint (400, 300). The sweep is 0..PI, so anything
    // with y < 300 is outside it and the radial cut removes it — unless a cap disc of
    // half-width 12 is centred there.
    assert_eq!(
        pixel(&round, 400, 293),
        (255, 255, 255, 255),
        "a round cap puts a disc of the stroke's half-width at the sweep endpoint, so \
         just outside the sweep is still painted"
    );
    assert_eq!(
        pixel(&butt, 400, 293),
        (0, 0, 0, 255),
        "a butt arc is cut along the radius at its end, so the same pixel is \
         background — this is the pre-plan-116-D behaviour"
    );
    // And the disc has the stroke's half-width, not some larger reach.
    assert_eq!(
        pixel(&round, 400, 285),
        (0, 0, 0, 255),
        "15 px past the endpoint is outside a 12 px cap disc"
    );
}

/// A round cap at the bounds' extreme is not clipped by the item's bounds.
///
/// The plan says to verify this rather than assume it. The arc header pads its hull by
/// `radius + half + 1.0`, and a cap disc of half-width `half` centred on a point at
/// distance `radius` from the centre reaches exactly `radius + half` — so it fits, with
/// one pixel to spare. That is an argument, not a measurement, and a hull one pixel
/// short would cut the cap's outer edge and nothing else.
///
/// The arc's start endpoint is (400, 300), the hull's +X extreme; the cap disc there
/// reaches x = 412 against a hull edge at x = 413.
#[test]
fn a_round_arc_cap_at_the_bounds_extreme_is_not_clipped() {
    let (frame, _) = render(
        "canvas_arccap_bounds",
        &scene(
            "  LET a AS canvas::DrawItem = canvas::Arc[id := \"\", position := vector::Float2[x := 300.0, y := 300.0], radius := 100.0, startAngle := 0.0, endAngle := 3.141592653589793, paint := WITH canvas::stroke(color::rgb(255, 255, 255), 24.0) { cap := canvas::CapStyle.Round }, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([a])\n",
        ),
    );
    assert_eq!(
        pixel(&frame, 411, 300),
        (255, 255, 255, 255),
        "the outermost column of the start cap's disc — a hull that did not grow for \
         the cap would have cut exactly here and left the rest of the arc intact"
    );
}

/// A full-circle arc looks the same either way, because it has no ends to cap.
///
/// The degenerate case for this phase. A `0..2*PI` sweep never leaves the sweep test,
/// so the cap discs are unioned into a band that already covers them — `min` with
/// something already inside changes nothing. Comparing the two frames byte for byte is
/// what rules out a disc drawn in the wrong place: on a closed arc that would be a
/// bulge, which no single-pixel check is positioned to see.
#[test]
fn a_full_circle_arc_is_identical_with_either_cap() {
    let ring = |cap: &str| {
        format!(
            "  LET a AS canvas::DrawItem = canvas::Arc[id := \"\", position := vector::Float2[x := 300.0, y := 300.0], radius := 100.0, startAngle := 0.0, endAngle := 6.283185307179586, paint := WITH canvas::stroke(color::rgb(255, 255, 255), 24.0) {{ cap := canvas::CapStyle.{cap} }}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]\n  \
             canvas::present([a])\n"
        )
    };
    let (butt, _) = render("canvas_arccap_ring_butt", &scene(&ring("Butt")));
    let (round, _) = render("canvas_arccap_ring_round", &scene(&ring("Round")));
    assert!(
        butt.iter().any(|&b| b != 0),
        "the ring drew nothing, so the comparison would be vacuous"
    );
    assert_eq!(
        butt, round,
        "a closed arc has no ends, so the cap must make no difference at all"
    );
}
