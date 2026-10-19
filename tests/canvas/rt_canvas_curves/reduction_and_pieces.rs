//! plan-176-D: `rt_canvas_curves`'s model tests: curve reduction, chains and hashing,
//! NURBS conversion, flattening, and the nearest-point and crossing queries on pieces.
//! The driver, the curve and NURBS models and the scene helpers live in the root,
//! `rt_canvas_curves.rs`.

use super::*;

/// A quadratic is exact: a monotone one is returned as it is, one piece whose seven
/// floats are the input's own; one that turns is cut at the turn, and nowhere else.
#[test]
fn canvas_curve_a_quadratic_reduces_to_itself_cut_at_its_turn() {
    let monotone = reduce(&poly(&[(0.0, 0.0), (50.0, 0.0), (100.0, 100.0)]), EPS);
    assert_eq!(monotone, vec![[0.0, 0.0, 50.0, 0.0, 100.0, 100.0, 1.0]]);

    // (0,0) (50,100) (100,0): y'(t) = 200(1 - 2t), zero at t = 1/2, where the curve is
    // at (50, 50). De Casteljau there: left (0,0) (25,50) (50,50), right (50,50)
    // (75,50) (100,0). Every number is exact in binary.
    let turning = reduce(&poly(&[(0.0, 0.0), (50.0, 100.0), (100.0, 0.0)]), EPS);
    assert_eq!(
        turning,
        vec![
            [0.0, 0.0, 25.0, 50.0, 50.0, 50.0, 1.0],
            [50.0, 50.0, 75.0, 50.0, 100.0, 0.0, 1.0],
        ]
    );
    assert_monotone(&turning, "the turning quadratic");

    // A line is the piece whose control point is its midpoint.
    let line = reduce(&poly(&[(10.0, 20.0), (30.0, 60.0)]), EPS);
    assert_eq!(line, vec![[10.0, 20.0, 20.0, 40.0, 30.0, 60.0, 1.0]]);
}

/// A rational quadratic is a conic and stays one: a quarter circle (`w1 = √2/2`) is one
/// piece whose every point is at radius `r`, and a 120° arc over the top of a circle —
/// which turns in `y` and is cut — is two pieces whose every point still is.
#[test]
fn canvas_curve_a_rational_quadratic_circle_arc_stays_exact() {
    let r = 100.0;
    let quarter = reduce(
        &rational(&[(r, 0.0), (r, r), (0.0, r)], &[1.0, SQRT_HALF, 1.0]),
        EPS,
    );
    assert_eq!(
        quarter.len(),
        1,
        "a quarter circle is y-monotone: {quarter:?}"
    );
    assert_eq!(quarter[0][6], SQRT_HALF);

    // The arc from 30° to 150°: ends (±r cos30°, r sin30°), tangents meeting at
    // (0, r / cos60°) = (0, 2r), weight cos60° = 1/2.
    let half = 3.0_f64.sqrt() / 2.0;
    let over = reduce(
        &rational(
            &[(r * half, r * 0.5), (0.0, 2.0 * r), (-r * half, r * 0.5)],
            &[1.0, 0.5, 1.0],
        ),
        EPS,
    );
    assert_eq!(over.len(), 2, "cut once, at the top: {over:?}");
    assert!((over[0][4]).abs() < 1e-12 && (over[0][5] - r).abs() < 1e-12);
    assert_connected(&over, "the arc over the top");
    assert_monotone(&over, "the arc over the top");

    for (what, reduced) in [("quarter", &quarter), ("over the top", &over)] {
        for p in reduced {
            for i in 0..=256 {
                let (x, y) = piece_at(p, i as f64 / 256.0);
                let radius = (x * x + y * y).sqrt();
                assert!(
                    (radius - r).abs() <= 1e-12,
                    "{what}: a point of the arc is at radius {radius:?}, not {r}"
                );
            }
        }
    }

    // End weights that are not 1 are normalised away: the same quarter circle written
    // with weights (2, √2, 2) is the same piece.
    let scaled = reduce(
        &rational(&[(r, 0.0), (r, r), (0.0, r)], &[2.0, 2.0 * SQRT_HALF, 2.0]),
        EPS,
    );
    assert_eq!(scaled.len(), 1);
    assert!((scaled[0][6] - SQRT_HALF).abs() <= 1e-15);
}

/// A cubic is fitted: every point of the S is within `eps` of its pieces and every
/// point of the pieces within `eps` of the S, measured densely; the pieces run end to
/// end from the curve's first point to its last; and each is y-monotone.
#[test]
fn canvas_curve_a_cubic_is_fitted_within_the_tolerance() {
    let s = cubic_s();
    let reduced = reduce(&s, EPS);
    assert_connected(&reduced, "the cubic S");
    assert_monotone(&reduced, "the cubic S");
    assert_eq!((reduced[0][0], reduced[0][1]), (0.0, 0.0));
    let last = reduced.last().expect("pieces");
    assert_eq!((last[4], last[5]), (300.0, 200.0));
    let deviation = dense_deviation(&s, &reduced);
    println!(
        "cubic S: {} pieces, dense max deviation {deviation:.6}",
        reduced.len()
    );
    assert!(deviation <= EPS, "the S deviates by {deviation}");
    assert!(
        reduced.len() <= 16,
        "{} pieces for one cubic",
        reduced.len()
    );
}

/// The tolerance is in surface pixels, so a shape drawn four times larger is reduced
/// four times finer in its own space: `0.05 / 4`. A rotation stretches nothing and
/// asks for `0.05`; a transform not in effect — all zero, or with no area — is the
/// identity; a shear is judged by its larger singular value.
#[test]
fn canvas_curve_the_tolerance_follows_the_shapes_own_transform() {
    let eps_for = |t: [f64; 6]| -> f64 {
        let spelled: Vec<String> = t.iter().map(|v| format!("{v:?}")).collect();
        let lines = run(&[("MODE", "eps".to_string()), ("T", spelled.join(","))]);
        value(&lines[0])
    };
    assert_eq!(eps_for([0.0; 6]), EPS);
    assert_eq!(eps_for([4.0, 0.0, 0.0, 4.0, 0.0, 0.0]), EPS / 4.0);
    assert_eq!(eps_for([1.0, 0.0, 0.0, 0.0, 0.0, 0.0]), EPS, "no area");
    let turned = eps_for([0.6, 0.8, -0.8, 0.6, 30.0, 40.0]);
    assert!((turned - EPS).abs() < 1e-15, "a rotation: {turned:?}");
    // x' = 3x, y' = 0.5y: the larger stretch is 3.
    let squashed = eps_for([3.0, 0.0, 0.0, 0.5, 0.0, 0.0]);
    assert!((squashed - EPS / 3.0).abs() < 1e-15, "{squashed:?}");

    let s = cubic_s();
    let fine = reduce(&s, EPS / 4.0);
    assert_monotone(&fine, "the cubic S at a quarter of the tolerance");
    let deviation = dense_deviation(&s, &fine);
    println!(
        "cubic S under a x4 transform: {} pieces, dense max deviation {deviation:.6} (local)",
        fine.len()
    );
    assert!(deviation <= EPS / 4.0, "the S deviates by {deviation}");
    assert!(fine.len() >= reduce(&s, EPS).len());
}

/// A degree-16 curve — the most a `BezierCurve` may hold — reduces within the
/// tolerance, and so does a rational cubic. (That the pieces are finite needs no
/// assertion: a `Float` cannot hold a NaN or an infinity, so arithmetic that would
/// make one fails the driver, and `run` checks it exited cleanly.)
#[test]
fn canvas_curve_high_degree_and_rational_cubics_reduce() {
    // Seventeen points zig-zagging across 400 px: x advances, y alternates.
    let points: Vec<(f64, f64)> = (0..17)
        .map(|i| (i as f64 * 25.0, if i % 2 == 0 { 0.0 } else { 120.0 }))
        .collect();
    let high = poly(&points);
    let reduced = reduce(&high, EPS);
    assert!(!reduced.is_empty());
    assert_connected(&reduced, "degree 16");
    assert_monotone(&reduced, "degree 16");
    let deviation = dense_deviation(&high, &reduced);
    println!(
        "degree 16: {} pieces, dense max deviation {deviation:.6}",
        reduced.len()
    );
    assert!(deviation <= EPS, "degree 16 deviates by {deviation}");

    let cubic = rational(
        &[(0.0, 0.0), (0.0, 150.0), (200.0, 150.0), (200.0, 0.0)],
        &[1.0, 3.0, 0.25, 1.0],
    );
    let reduced = reduce(&cubic, EPS);
    assert_connected(&reduced, "the rational cubic");
    assert_monotone(&reduced, "the rational cubic");
    let deviation = dense_deviation(&cubic, &reduced);
    println!(
        "rational cubic: {} pieces, dense max deviation {deviation:.6}",
        reduced.len()
    );
    assert!(
        deviation <= EPS,
        "the rational cubic deviates by {deviation}"
    );
}

/// The adversarial curves the 16-sample acceptance test could be wrong about: a cusp, a
/// loop, a curve that doubles back along its own chord, weights four orders of
/// magnitude apart, and control points that coincide. Each is measured densely; the
/// numbers are what plan-164-A records.
#[test]
fn canvas_curve_adversarial_curves_stay_within_the_tolerance() {
    let cases: Vec<(&str, Curve)> = vec![
        (
            "cusp",
            poly(&[(0.0, 0.0), (200.0, 200.0), (0.0, 200.0), (200.0, 0.0)]),
        ),
        (
            "loop",
            poly(&[(0.0, 0.0), (300.0, 250.0), (-100.0, 250.0), (200.0, 0.0)]),
        ),
        (
            "doubling back",
            poly(&[(0.0, 0.0), (300.0, 0.0), (-100.0, 0.0), (200.0, 0.0)]),
        ),
        (
            "weights 1e-2 and 1e2",
            rational(
                &[(0.0, 0.0), (50.0, 200.0), (250.0, 200.0), (300.0, 0.0)],
                &[1.0, 0.01, 100.0, 1.0],
            ),
        ),
        (
            "coincident controls",
            poly(&[(0.0, 0.0), (0.0, 0.0), (150.0, 100.0), (150.0, 100.0)]),
        ),
        (
            "quintic wave",
            poly(&[
                (0.0, 0.0),
                (60.0, 300.0),
                (120.0, -300.0),
                (180.0, 300.0),
                (240.0, -300.0),
                (300.0, 0.0),
            ]),
        ),
    ];
    for (what, curve) in cases {
        let reduced = reduce(&curve, EPS);
        assert!(!reduced.is_empty(), "{what}");
        assert_connected(&reduced, what);
        assert_monotone(&reduced, what);
        let deviation = dense_deviation(&curve, &reduced);
        println!(
            "{what}: {} pieces, dense max deviation {deviation:.6}",
            reduced.len()
        );
        assert!(deviation <= EPS, "{what} deviates by {deviation}");
    }
}

/// What makes a curve absent: fewer than 2 points or more than 17, a weight list of
/// the wrong length, a weight that is zero or negative.
#[test]
fn canvas_curve_validity() {
    let seventeen: Vec<(f64, f64)> = (0..17).map(|i| (i as f64, 0.0)).collect();
    let eighteen: Vec<(f64, f64)> = (0..18).map(|i| (i as f64, 0.0)).collect();
    let three = [(0.0, 0.0), (1.0, 1.0), (2.0, 0.0)];
    let cases = vec![
        (poly(&[(0.0, 0.0)]), false),
        (poly(&[(0.0, 0.0), (1.0, 1.0)]), true),
        (poly(&seventeen), true),
        (poly(&eighteen), false),
        (rational(&three, &[1.0, 2.0, 1.0]), true),
        (rational(&three, &[1.0, 2.0]), false),
        (rational(&three, &[1.0, 2.0, 1.0, 1.0]), false),
        (rational(&three, &[1.0, 0.0, 1.0]), false),
        (rational(&three, &[1.0, -1.0, 1.0]), false),
    ];
    let curves: Vec<Curve> = cases.iter().map(|(c, _)| c.clone()).collect();
    let lines = run(&[("MODE", "valid".to_string()), ("CURVES", spell(&curves))]);
    let got: Vec<bool> = lines[0]
        .split_whitespace()
        .skip(1)
        .map(|v| v == "1")
        .collect();
    let want: Vec<bool> = cases.iter().map(|(_, valid)| *valid).collect();
    assert_eq!(got, want);

    // An invalid curve gives no piece.
    assert!(reduce(&rational(&three, &[1.0, 0.0, 1.0]), EPS).is_empty());
    assert!(reduce(&poly(&[(0.0, 0.0)]), EPS).is_empty());
}

/// The chain walk. Curves that meet are joined by nothing; a curve that begins away
/// from where the one before ended is joined to it by one straight piece; an invalid
/// curve is skipped, and its neighbours joined across it the same way. The marks are
/// the piece each input segment — curve or implied line — begins at.
#[test]
fn canvas_curve_a_chain_joins_its_curves() {
    let first = poly(&[(0.0, 0.0), (50.0, 0.0), (100.0, 100.0)]);
    let meets = poly(&[(100.0, 100.0), (200.0, 100.0)]);
    let (reduced, starts) = chain(&[first.clone(), meets.clone()], false);
    assert_eq!(
        reduced,
        vec![
            [0.0, 0.0, 50.0, 0.0, 100.0, 100.0, 1.0],
            [100.0, 100.0, 150.0, 100.0, 200.0, 100.0, 1.0],
        ]
    );
    assert_eq!(starts, vec![0, 1]);

    // The second curve starts 20 px to the right of where the first ended.
    let apart = poly(&[(120.0, 100.0), (200.0, 100.0)]);
    let (reduced, starts) = chain(&[first.clone(), apart.clone()], false);
    assert_eq!(
        reduced,
        vec![
            [0.0, 0.0, 50.0, 0.0, 100.0, 100.0, 1.0],
            [100.0, 100.0, 110.0, 100.0, 120.0, 100.0, 1.0],
            [120.0, 100.0, 160.0, 100.0, 200.0, 100.0, 1.0],
        ]
    );
    assert_eq!(starts, vec![0, 1, 2]);

    // An invalid curve between them (a zero weight): skipped, and the same join.
    let broken = rational(
        &[(100.0, 100.0), (110.0, 300.0), (120.0, 100.0)],
        &[1.0, 0.0, 1.0],
    );
    let (reduced, starts) = chain(&[first.clone(), broken.clone(), apart], false);
    assert_eq!(reduced.len(), 3);
    assert_eq!(reduced[1], [100.0, 100.0, 110.0, 100.0, 120.0, 100.0, 1.0]);
    assert_eq!(starts, vec![0, 1, 2]);

    // A leading invalid curve adds nothing, not even a join.
    let (reduced, starts) = chain(&[broken.clone(), first.clone()], false);
    assert_eq!(reduced.len(), 1);
    assert_eq!(starts, vec![0]);

    // No valid curve: nothing.
    let (reduced, starts) = chain(&[broken], true);
    assert!(reduced.is_empty());
    assert!(starts.is_empty());
    let (reduced, _) = chain(&[], true);
    assert!(reduced.is_empty());
}

/// Closing. A contour whose last point is not its first gains one straight piece back
/// to the first; one that already ends where it began gains nothing; and a chain that
/// is not closed never gains one, even when it ends away from where it began.
#[test]
fn canvas_curve_a_contour_is_closed_and_a_chain_is_not() {
    let a = poly(&[(0.0, 0.0), (100.0, 0.0)]);
    let b = poly(&[(100.0, 0.0), (100.0, 80.0), (0.0, 80.0)]);

    let (open, starts) = chain(&[a.clone(), b.clone()], false);
    assert_eq!(open.len(), 2, "a chain is not closed");
    assert_eq!(starts, vec![0, 1]);

    let (closed, starts) = chain(&[a.clone(), b.clone()], true);
    assert_eq!(closed.len(), 3, "a contour is");
    assert_eq!(closed[2], [0.0, 80.0, 0.0, 40.0, 0.0, 0.0, 1.0]);
    assert_eq!(starts, vec![0, 1, 2]);

    let back = poly(&[(0.0, 80.0), (0.0, 0.0)]);
    let (already, starts) = chain(&[a, b, back], true);
    assert_eq!(already.len(), 3, "already closed: no fourth piece");
    assert_eq!(starts, vec![0, 1, 2]);
}

/// A chain is built where its shape is: every point is moved by the offset the shape's
/// record is built at, and the marks count from the pieces already ahead of the chain.
#[test]
fn canvas_curve_a_chain_is_moved_and_its_marks_are_based() {
    let lines = run(&[
        ("MODE", "chain".to_string()),
        (
            "CURVES",
            spell(&[
                poly(&[(0.0, 0.0), (50.0, 0.0), (100.0, 100.0)]),
                poly(&[(100.0, 100.0), (0.0, 0.0)]),
            ]),
        ),
        ("OX", "10".to_string()),
        ("OY", "20".to_string()),
        ("BASE", "5".to_string()),
    ]);
    assert_eq!(
        pieces(&lines),
        vec![
            [10.0, 20.0, 60.0, 20.0, 110.0, 120.0, 1.0],
            [110.0, 120.0, 60.0, 70.0, 10.0, 20.0, 1.0],
        ]
    );
    assert_eq!(marks(&lines), vec![5, 6]);
}

/// A curve shape's hash covers everything it is made of, however deep: a
/// `CurvePolygon` holds lists of lists of records that hold lists, and a
/// geometry-cache hit is trusted on the item hash alone
/// (`.ai/canvas-threading.md` §14).
///
/// Four presents of one `CurvePolygon`: as it is; with ONE weight of the second
/// curve of its second contour changed; with only its `position` changed; and that
/// last one again, rebuilt and presented twice over (a scene equal to the installed one
/// publishes nothing and draws no frame, bug-968, so the rebuilt item is shown in a
/// scene that differs). Each of the first three is a different item and costs one
/// generation; the fourth is the third and costs none (`generations=1,2,3,3`). A hash
/// that stopped short of the inner lists would hit on the second present.
///
/// Then the same for a `PolyCurve`, with one coordinate of a middle control point.
#[test]
fn canvas_curve_shapes_hash_every_point_and_weight() {
    let run = scene(
        "canvas_curve_hash",
        r#"  canvas::present([blob(50.0, 0.7)])
  canvas::present([blob(50.0, 0.8)])
  canvas::present([blob(60.0, 0.8)])
  canvas::present([blob(60.0, 0.8), blob(60.0, 0.8)])
  canvas::present([wave(40.0)])
  canvas::present([wave(41.0)])
  canvas::present([wave(41.0), wave(41.0)])
"#,
        r#"
FUNC blob(x AS Float, w AS Float) AS canvas::DrawItem
  LET outer AS List OF canvas::BezierCurve = [conic(100.0, 0.0, 100.0, 100.0, 0.0, 100.0, 0.7), line(0.0, 100.0, 0.0, 0.0)]
  LET inner AS List OF canvas::BezierCurve = [line(20.0, 20.0, 60.0, 20.0), conic(60.0, 20.0, 60.0, 60.0, 20.0, 60.0, w)]
  LET item AS canvas::DrawItem = canvas::CurvePolygon[id := "", position := at(x, 50.0), contours := [outer, inner], rule := canvas::FillRule.EvenOdd, paint := canvas::fill(color::rgb(0, 0, 255)), transform := none()]
  RETURN item
END FUNC

FUNC wave(y AS Float) AS canvas::DrawItem
  LET item AS canvas::DrawItem = canvas::PolyCurve[id := "", position := at(50.0, 300.0), curves := [line(0.0, 0.0, 50.0, 0.0), canvas::BezierCurve[points := [at(50.0, 0.0), at(80.0, y), at(120.0, 0.0 - 40.0), at(150.0, 0.0)], weights := []]], paint := canvas::stroke(color::rgb(0, 0, 255), 3.0), transform := none()]
  RETURN item
END FUNC
"#,
        &[],
    );
    assert_eq!(
        run.stats_of("generations"),
        ["1", "2", "3", "3", "4", "5", "5"],
        "{:?}",
        run.stats
    );
    assert_eq!(run.stat("geoVerifyMismatches"), "0");
}

/// The standard NURBS circle — nine points round a square, degree 2, each interior
/// knot doubled — is four rational quadratics. Every point of each is at radius 1 to
/// `1e-12`, each ends exactly where the next begins, and the last ends where the first
/// began: the list is a closed contour as it stands, with no implied segment.
#[test]
fn canvas_nurbs_a_circle_converts_to_four_exact_arcs() {
    let corner = SQRT_HALF;
    let circle = Nurbs {
        points: vec![
            (1.0, 0.0),
            (1.0, 1.0),
            (0.0, 1.0),
            (-1.0, 1.0),
            (-1.0, 0.0),
            (-1.0, -1.0),
            (0.0, -1.0),
            (1.0, -1.0),
            (1.0, 0.0),
        ],
        weights: vec![1.0, corner, 1.0, corner, 1.0, corner, 1.0, corner, 1.0],
        knots: vec![
            0.0, 0.0, 0.0, 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1.0, 1.0, 1.0,
        ],
        degree: 2,
    };
    let run = scene("canvas_nurbs_circle", &circle.call("arc"), "", &[]);
    let arcs = run.curves("arc");
    assert_eq!(arcs.len(), 4);
    for (index, arc) in arcs.iter().enumerate() {
        assert_eq!(arc.points.len(), 3, "a rational quadratic");
        assert_eq!(arc.weights, vec![1.0, corner, 1.0]);
        for i in 0..=256 {
            let (x, y) = arc.at(i as f64 / 256.0);
            let radius = (x * x + y * y).sqrt();
            assert!(
                (radius - 1.0).abs() <= 1e-12,
                "arc {index}: radius {radius:?}"
            );
        }
        let next = &arcs[(index + 1) % 4];
        assert_eq!(arc.points[2], next.points[0], "arc {index} meets the next");
    }
    circle.assert_matches(&arcs, 1e-12, "the circle");
}

/// A B-spline whose knots are NOT clamped — a uniform cubic, knots 0 to 10 — converts
/// to the Béziers of its domain (knots 3 to 7, four spans), and each agrees with the
/// B-spline evaluated directly, at 256 parameters, to `1e-12`. Then a rational curve
/// on a clamped vector with a single and a doubled interior knot, the same way.
#[test]
fn canvas_nurbs_a_b_spline_converts_to_the_beziers_of_its_spans() {
    let uniform = Nurbs {
        points: vec![
            (0.0, 0.0),
            (1.0, 4.0),
            (3.0, 5.0),
            (4.0, 1.0),
            (6.0, 0.0),
            (7.0, 3.0),
            (8.0, 6.0),
        ],
        weights: vec![],
        knots: (0..11).map(f64::from).collect(),
        degree: 3,
    };
    let mixed = Nurbs {
        points: vec![
            (0.0, 0.0),
            (2.0, 6.0),
            (5.0, 7.0),
            (6.0, 2.0),
            (8.0, 1.0),
            (9.0, 5.0),
            (7.0, 8.0),
        ],
        weights: vec![1.0, 2.0, 0.5, 1.0, 3.0, 0.75, 1.0],
        knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 2.5, 2.5, 4.0, 4.0, 4.0, 4.0],
        degree: 3,
    };
    let run = scene(
        "canvas_nurbs_spline",
        &(uniform.call("uniform") + &mixed.call("mixed")),
        "",
        &[],
    );
    let beziers = run.curves("uniform");
    assert_eq!(beziers.len(), 4);
    assert!(
        beziers.iter().all(|b| b.weights.is_empty()),
        "no weights in, none out"
    );
    uniform.assert_matches(&beziers, 1e-12, "the uniform cubic");
    for pair in beziers.windows(2) {
        assert_eq!(pair[0].points[3], pair[1].points[0], "the chain is joined");
    }

    let beziers = run.curves("mixed");
    assert_eq!(beziers.len(), 3, "spans 0-1, 1-2.5 and 2.5-4");
    assert!(beziers.iter().all(|b| b.weights.len() == 4));
    mixed.assert_matches(&beziers, 1e-12, "the rational cubic");
    assert_eq!(
        beziers[0].points[0],
        (0.0, 0.0),
        "clamped: it starts on its first point"
    );
    assert_eq!(beziers[2].points[3], (7.0, 8.0), "and ends on its last");
}

/// Input that is not a curve returns `[]` and does not raise — every rule the manual
/// lists, one at a time, against a call that differs from a valid one in that alone.
#[test]
fn canvas_nurbs_invalid_input_returns_an_empty_list() {
    let points = [(0.0, 0.0), (1.0, 2.0), (3.0, 2.0), (4.0, 0.0)];
    let ones = [1.0, 1.0, 1.0, 1.0];
    let knots = [0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.0];
    let call = |tag: &str, points: &[(f64, f64)], weights: &[f64], knots: &[f64], degree: &str| {
        format!(
            "  showCurves(\"{tag}\", canvas::nurbsToBeziers({}, {}, {}, {degree}))\n",
            point_list(points),
            float_list(weights),
            float_list(knots)
        )
    };
    let cases: Vec<(&str, String)> = vec![
        ("valid", call("valid", &points, &ones, &knots, "2")),
        ("unweighted", call("unweighted", &points, &[], &knots, "2")),
        ("degree0", call("degree0", &points, &ones, &knots, "0")),
        ("degree17", call("degree17", &points, &ones, &knots, "17")),
        (
            "negdegree",
            call("negdegree", &points, &ones, &knots, "0 - 1"),
        ),
        (
            "fewknots",
            call("fewknots", &points, &ones, &knots[..6], "2"),
        ),
        (
            "manyknots",
            call(
                "manyknots",
                &points,
                &ones,
                &[0.0, 0.0, 0.0, 0.5, 0.7, 1.0, 1.0, 1.0],
                "2",
            ),
        ),
        (
            "decreasing",
            call(
                "decreasing",
                &points,
                &ones,
                &[0.0, 0.0, 0.0, 0.5, 0.25, 1.0, 1.0],
                "2",
            ),
        ),
        (
            "zeroweight",
            call("zeroweight", &points, &[1.0, 0.0, 1.0, 1.0], &knots, "2"),
        ),
        (
            "negweight",
            call("negweight", &points, &[1.0, -2.0, 1.0, 1.0], &knots, "2"),
        ),
        (
            "shortweights",
            call("shortweights", &points, &[1.0, 1.0, 1.0], &knots, "2"),
        ),
        // Degree 3 needs four points for one span; two points cannot carry it.
        (
            "fewpoints",
            call(
                "fewpoints",
                &points[..2],
                &[],
                &[0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                "3",
            ),
        ),
        // Every knot the same: `knots[degree]` is not below `knots[points]`.
        ("nodomain", call("nodomain", &points, &ones, &[2.0; 7], "2")),
    ];
    let body: String = cases.iter().map(|(_, c)| c.as_str()).collect();
    let run = scene("canvas_nurbs_invalid", &body, "", &[]);
    assert_eq!(run.curves("valid").len(), 2, "the control case converts");
    assert_eq!(run.curves("unweighted").len(), 2);
    for (tag, _) in &cases[2..] {
        assert!(run.curves(tag).is_empty(), "{tag} is not a curve");
    }
}

/// How far `q` is from the polyline through `points`.
fn polyline_distance(points: &[(f64, f64)], q: (f64, f64)) -> f64 {
    points
        .windows(2)
        .map(|s| segment_distance(s[0], s[1], q))
        .fold(f64::INFINITY, f64::min)
}

/// `canvas::flatten`: the points begin on the curve's first point and end on its last,
/// exactly; every point of the curve is within `tolerance` of the chords between them
/// (4096 samples); a smaller tolerance returns more points; a tolerance below 0.001 —
/// zero and negative included — is 0.001; a straight curve is its two ends; an invalid
/// curve is `[]`.
#[test]
fn canvas_flatten_stays_within_its_tolerance() {
    let s = cubic_s();
    let quarter = rational(
        &[(100.0, 0.0), (100.0, 100.0), (0.0, 100.0)],
        &[1.0, SQRT_HALF, 1.0],
    );
    let curve = |c: &Curve| {
        format!(
            "canvas::BezierCurve[points := {}, weights := {}]",
            point_list(&c.points),
            float_list(&c.weights)
        )
    };
    let body = format!(
        "  LET s AS canvas::BezierCurve = {}\n  \
         LET q AS canvas::BezierCurve = {}\n  \
         showPoints(\"coarse\", canvas::flatten(s, 2.0))\n  \
         showPoints(\"fine\", canvas::flatten(s, 0.1))\n  \
         showPoints(\"floor\", canvas::flatten(s, 0.001))\n  \
         showPoints(\"zero\", canvas::flatten(s, 0.0))\n  \
         showPoints(\"negative\", canvas::flatten(s, 0.0 - 5.0))\n  \
         showPoints(\"arc\", canvas::flatten(q, 0.05))\n  \
         showPoints(\"straight\", canvas::flatten(line(10.0, 20.0, 30.0, 60.0), 0.5))\n  \
         showPoints(\"broken\", canvas::flatten(canvas::BezierCurve[points := [at(0.0, 0.0), at(5.0, 5.0), at(9.0, 0.0)], weights := [1.0, 0.0, 1.0]], 0.5))\n  \
         showPoints(\"single\", canvas::flatten(canvas::BezierCurve[points := [at(0.0, 0.0)], weights := []], 0.5))\n",
        curve(&s),
        curve(&quarter),
    );
    let run = scene("canvas_flatten", &body, "", &[]);

    for (tag, tolerance, input) in [
        ("coarse", 2.0, &s),
        ("fine", 0.1, &s),
        ("floor", 0.001, &s),
        ("arc", 0.05, &quarter),
    ] {
        let points = run.points(tag);
        assert_eq!(points[0], input.points[0], "{tag}: the first point");
        assert_eq!(points.last(), input.points.last(), "{tag}: the last point");
        let worst = (0..=4096)
            .map(|i| polyline_distance(&points, input.at(i as f64 / 4096.0)))
            .fold(0.0, f64::max);
        println!(
            "flatten {tag}: {} points, max deviation {worst:.6}",
            points.len()
        );
        assert!(worst <= tolerance, "{tag}: the chords stray {worst}");
    }
    let (coarse, fine, floor) = (
        run.points("coarse"),
        run.points("fine"),
        run.points("floor"),
    );
    assert!(coarse.len() < fine.len() && fine.len() < floor.len());
    assert_eq!(run.points("zero"), floor, "0.0 is treated as 0.001");
    assert_eq!(run.points("negative"), floor, "and so is a negative one");
    assert_eq!(run.points("straight"), vec![(10.0, 20.0), (30.0, 60.0)]);
    assert!(run.points("broken").is_empty());
    assert!(run.points("single").is_empty());
}

/// `flatten`'s points are a `Polygon` contour as they stand, at the same `position` the
/// curve's shape would have. A lens: a quadratic up and a quadratic back, flattened and
/// joined. Its top edge is `y = -160 t(1 - t)` over `x = 200 t`, so it is 40 above the
/// chord at the middle; at `position` (100, 300) the lens spans y = 260 to 340 on
/// x = 200. (200, 300) is its centre; (200, 262) and (200, 337) are 2 px inside its top
/// and bottom, (200, 257) and (200, 342) 3 px outside them; (95, 300) is left of its
/// first point.
#[test]
fn canvas_flatten_points_build_a_polygon() {
    let run = scene(
        "canvas_flatten_polygon",
        r#"  LET up AS canvas::BezierCurve = canvas::BezierCurve[points := [at(0.0, 0.0), at(100.0, 0.0 - 80.0), at(200.0, 0.0)], weights := []]
  LET down AS canvas::BezierCurve = canvas::BezierCurve[points := [at(200.0, 0.0), at(100.0, 80.0), at(0.0, 0.0)], weights := []]
  MUT outline AS List OF vector::Float2 = canvas::flatten(up, 0.05)
  LET back AS List OF vector::Float2 = canvas::flatten(down, 0.05)
  MUT i AS Integer = 1
  WHILE i < len(back)
    outline = collections::append(outline, collections::getOr(back, i, at(0.0, 0.0)))
    i = i + 1
  END WHILE
  MUT lens AS canvas::Polygon
  lens = WITH lens { position := at(100.0, 300.0), contours := [outline], paint := canvas::fill(color::rgb(0, 0, 255)) }
  LET item AS canvas::DrawItem = lens
  canvas::present([item])
"#,
        "",
        &[],
    );
    let blue = (0, 0, 255, 255);
    let black = (0, 0, 0, 255);
    assert_eq!(run.pixel(200, 300), blue, "the centre");
    assert_eq!(run.pixel(200, 262), blue, "2 px inside the top");
    assert_eq!(run.pixel(200, 337), blue, "2 px inside the bottom");
    assert_eq!(run.pixel(200, 257), black, "3 px above the top");
    assert_eq!(run.pixel(200, 342), black, "3 px below the bottom");
    assert_eq!(run.pixel(95, 300), black, "left of the first point");
}

/// `__canvas_pieceNearest` against a brute-force search, in the standalone driver.
///
/// For each piece, a grid of query points round it: the distance the isolation solver
/// answers is compared with the best of 4096 samples of the piece refined by a
/// golden-section search (`piece_distance` with a finer start), and the bound
/// `__canvas_pieceBound` answers is checked to be a lower bound. The three conics are
/// the ones plan-164-B names — a quarter circle (`w1 = √2/2`), a thin conic (`w1 = 5`)
/// and a near-parabola (`w1 = 1.01`) — plus a polynomial piece, a straight one, and a
/// piece 2000 px long, where a search that sampled would be at its worst.
#[test]
fn canvas_piece_nearest_matches_a_brute_force_search() {
    let cases: Vec<(&str, Piece)> = vec![
        (
            "quarter circle",
            [100.0, 0.0, 100.0, 100.0, 0.0, 100.0, SQRT_HALF],
        ),
        (
            "thin conic, w1 = 5",
            [0.0, 0.0, 150.0, 20.0, 300.0, 200.0, 5.0],
        ),
        (
            "near-parabola, w1 = 1.01",
            [0.0, 0.0, 150.0, 20.0, 300.0, 200.0, 1.01],
        ),
        (
            "flat conic, w1 = 0.05",
            [0.0, 0.0, 150.0, 20.0, 300.0, 200.0, 0.05],
        ),
        ("parabola", [0.0, 0.0, 20.0, 180.0, 300.0, 200.0, 1.0]),
        ("tight parabola", [0.0, 0.0, 400.0, 3.0, 0.0, 6.0, 1.0]),
        ("line", [10.0, 20.0, 110.0, 70.0, 210.0, 120.0, 1.0]),
        (
            "2000 px conic",
            [0.0, 0.0, 1900.0, 100.0, 2000.0, 900.0, 0.4],
        ),
    ];
    for (what, piece) in cases {
        // A grid over the piece's box, a margin round it, and points ON the piece.
        let xs = [piece[0], piece[2], piece[4]];
        let ys = [piece[1], piece[3], piece[5]];
        let (x0, x1) = (
            xs.iter().cloned().fold(f64::INFINITY, f64::min) - 60.0,
            xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max) + 60.0,
        );
        let (y0, y1) = (
            ys.iter().cloned().fold(f64::INFINITY, f64::min) - 60.0,
            ys.iter().cloned().fold(f64::NEG_INFINITY, f64::max) + 60.0,
        );
        let mut points: Vec<(f64, f64)> = vec![];
        for i in 0..=18 {
            for j in 0..=18 {
                points.push((
                    x0 + (x1 - x0) * i as f64 / 18.0 + 0.37,
                    y0 + (y1 - y0) * j as f64 / 18.0 + 0.61,
                ));
            }
        }
        for i in 0..=40 {
            let on = piece_at(&piece, i as f64 / 40.0);
            points.push(on);
            points.push((on.0 + 0.4, on.1 - 0.3));
        }
        let spelled: Vec<String> = points.iter().map(|(x, y)| format!("{x:?},{y:?}")).collect();
        let lines = run(&[
            ("MODE", "nearest".to_string()),
            (
                "PIECE",
                piece
                    .iter()
                    .map(|v| format!("{v:?}"))
                    .collect::<Vec<_>>()
                    .join(","),
            ),
            ("POINTS", spelled.join(",")),
        ]);
        assert_eq!(lines.len(), points.len(), "{what}");
        let mut worst: f64 = 0.0;
        for (line, q) in lines.iter().zip(&points) {
            let v: Vec<f64> = line.split_whitespace().skip(1).map(value).collect();
            let (got, t, bound) = (v[0], v[1], v[2]);
            // Brute force: 4096 samples, then the golden-section search either side.
            let mut best = (f64::INFINITY, 0usize);
            for i in 0..=4096 {
                let d = distance(piece_at(&piece, i as f64 / 4096.0), *q);
                if d < best.0 {
                    best = (d, i);
                }
            }
            let (mut lo, mut hi) = (
                best.1.saturating_sub(1) as f64 / 4096.0,
                (best.1 + 1).min(4096) as f64 / 4096.0,
            );
            for _ in 0..80 {
                let (a, b) = (
                    hi - (hi - lo) * 0.618_033_988_749_894_9,
                    lo + (hi - lo) * 0.618_033_988_749_894_9,
                );
                if distance(piece_at(&piece, a), *q) < distance(piece_at(&piece, b), *q) {
                    hi = b;
                } else {
                    lo = a;
                }
            }
            let want = best.0.min(distance(piece_at(&piece, (lo + hi) / 2.0), *q));
            worst = worst.max((got - want).abs());
            assert!(
                (got - want).abs() <= 1e-6,
                "{what}: at {q:?} the solver answers {got}, a brute-force search {want}"
            );
            assert!((0.0..=1.0).contains(&t), "{what}: t = {t}");
            assert!(
                (distance(piece_at(&piece, t), *q) - got).abs() <= 1e-9,
                "{what}: the parameter left behind is not the nearest point's"
            );
            assert!(
                bound <= got + 1e-9,
                "{what}: the bound {bound} is above {got}"
            );
        }
        println!(
            "nearest, {what}: {} points, worst error {worst:.3e} px",
            points.len()
        );
    }
}

/// `__canvas_pieceCrossX`: where a piece crosses a row. For rows strictly between its
/// ends the x answered is on the piece (the point of the piece with that y, found here
/// by bisection on the monotone y), and at a row through an end it is that end's x.
#[test]
fn canvas_piece_crossing_is_on_the_piece() {
    let cases: Vec<(&str, Piece)> = vec![
        (
            "quarter circle",
            [100.0, 0.0, 100.0, 100.0, 0.0, 100.0, SQRT_HALF],
        ),
        ("rising conic", [0.0, 200.0, 150.0, 180.0, 300.0, 0.0, 5.0]),
        ("parabola", [0.0, 0.0, 20.0, 180.0, 300.0, 200.0, 1.0]),
        ("line", [10.0, 20.0, 110.0, 70.0, 210.0, 120.0, 1.0]),
        ("vertical line", [40.0, 0.0, 40.0, 50.0, 40.0, 100.0, 1.0]),
    ];
    for (what, piece) in cases {
        let (ya, yb) = (piece[1], piece[5]);
        let rows: Vec<f64> = (1..40).map(|i| ya + (yb - ya) * i as f64 / 40.0).collect();
        let lines = run(&[
            ("MODE", "cross".to_string()),
            (
                "PIECE",
                piece
                    .iter()
                    .map(|v| format!("{v:?}"))
                    .collect::<Vec<_>>()
                    .join(","),
            ),
            (
                "ROWS",
                rows.iter()
                    .map(|v| format!("{v:?}"))
                    .collect::<Vec<_>>()
                    .join(","),
            ),
        ]);
        for (line, row) in lines.iter().zip(&rows) {
            let got = value(line.split_whitespace().nth(1).expect("an x"));
            let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
            for _ in 0..200 {
                let mid = (lo + hi) / 2.0;
                if (piece_at(&piece, mid).1 < *row) == (ya < yb) {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            let want = piece_at(&piece, (lo + hi) / 2.0).0;
            assert!(
                (got - want).abs() <= 1e-9,
                "{what}: row {row} is crossed at {want}, answered {got}"
            );
        }
    }
}
