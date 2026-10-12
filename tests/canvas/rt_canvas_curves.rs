//! Bézier curves: the reduction to y-monotone rational-quadratic pieces, validation
//! and the chain walk (plan-164-A).
//!
//! The reduction is MFBASIC the compiler injects
//! (`src/codegen/builtins/canvas/helper_curves.rs:CURVE_MATH` and `CURVE_CHAIN`). The
//! compiler is a binary crate, so a test cannot link that text, and the `canvas`
//! package cannot be imported outside app mode; this file reads the two chunks out of
//! the source file they are declared in, compiles them as a console program behind a
//! small driver, and runs it — the way `rt_canvas_auto_step.rs` runs `AUTO_STEP`. What
//! runs is the text the compiler injects, character for character, with the
//! `__canvas_` prefix (which user source may not spell) renamed, and with the two
//! record types the chain names declared by the driver, field for field.
//!
//! Everything asserted is computed here, in Rust, from the pieces the program prints
//! and from the INPUT curve evaluated independently: the reduction is never asked how
//! well it did. A float is printed as two integers (`v · 2^20` split at the binary
//! point, the fraction to 40 bits), which is exact to `2^-60` — every bit of a
//! coordinate or a weight here — because `toString` of a `Float` keeps two decimals.

#[path = "../common/mod.rs"]
mod common;

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

/// The reduction tolerance in surface pixels, `__CANVAS_CURVE_EPS`.
const EPS: f64 = 0.05;

/// The text of the raw string a `const` in `file` holds, with the reserved prefixes
/// renamed.
fn chunk(file: &str, opens: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/codegen/builtins/canvas")
        .join(file);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {file}: {e}"));
    let start = text
        .find(opens)
        .unwrap_or_else(|| panic!("{file} declares `{opens}`"))
        + opens.len();
    let end = start + text[start..].find("\"#;").expect("the raw string closes");
    text[start..end]
        .replace("__canvas_", "k_")
        .replace("__CANVAS_", "K_")
}

const DRIVER: &str = r#"
FUNC k_absF(v AS Float) AS Float
  IF v < 0.0 THEN
    RETURN 0.0 - v
  END IF
  RETURN v
END FUNC

FUNC floats(text AS String) AS List OF Float
  MUT out AS List OF Float = []
  IF text = "" THEN
    RETURN out
  END IF
  FOR EACH part IN strings::split(text, ",")
    out = collections::append(out, toFloat(part))
  NEXT
  RETURN out
END FUNC

' `x,y,x,y/w,w|x,y,...` -- curves apart at `|`, a curve's weights after its `/`.
FUNC curves(text AS String) AS List OF BezierCurve
  MUT out AS List OF BezierCurve = []
  IF text = "" THEN
    RETURN out
  END IF
  FOR EACH part IN strings::split(text, "|")
    LET halves AS List OF String = strings::split(part, "/")
    LET xy AS List OF Float = floats(collections::getOr(halves, 0, ""))
    MUT pts AS List OF vector::Float2 = []
    MUT i AS Integer = 0
    WHILE i + 1 < len(xy)
      pts = collections::append(pts, vector::Float2[x := collections::getOr(xy, i, 0.0), y := collections::getOr(xy, i + 1, 0.0)])
      i = i + 2
    END WHILE
    out = collections::append(out, BezierCurve[points := pts, weights := floats(collections::getOr(halves, 1, ""))])
  NEXT
  RETURN out
END FUNC

FUNC exact(v AS Float) AS String
  LET scaled AS Float = v * 1048576.0
  LET hi AS Integer = toInt(scaled)
  LET lo AS Integer = toInt((scaled - toFloat(hi)) * 1099511627776.0)
  RETURN toString(hi) & ":" & toString(lo)
END FUNC

FUNC show(pieces AS List OF Float) AS Nothing
  MUT i AS Integer = 0
  WHILE i + 6 < len(pieces)
    MUT line AS String = "piece"
    MUT k AS Integer = 0
    WHILE k < 7
      line = line & " " & exact(collections::getOr(pieces, i + k, 0.0))
      k = k + 1
    END WHILE
    io::print(line)
    i = i + 7
  END WHILE
END FUNC

FUNC main() AS Nothing
  LET mode AS String = os::getEnvOr("MODE", "")
  LET eps AS Float = toFloat(os::getEnvOr("EPS", "0.05"))
  IF mode = "reduce" THEN
    show(k_cvReduce(floats(os::getEnvOr("H", "")), eps))
  END IF
  IF mode = "one" THEN
    FOR EACH c IN curves(os::getEnvOr("CURVES", ""))
      show(k_reduceCurve(c, eps))
    NEXT
  END IF
  IF mode = "chain" THEN
    K_CURVE_MARKS = []
    show(k_curveChain(curves(os::getEnvOr("CURVES", "")), os::getEnvOr("CLOSED", "0") = "1", eps, toFloat(os::getEnvOr("OX", "0")), toFloat(os::getEnvOr("OY", "0")), toInt(os::getEnvOr("BASE", "0"))))
    MUT line AS String = "marks"
    FOR EACH m IN K_CURVE_MARKS
      line = line & " " & toString(m)
    NEXT
    io::print(line)
  END IF
  IF mode = "valid" THEN
    MUT line AS String = "valid"
    FOR EACH c IN curves(os::getEnvOr("CURVES", ""))
      IF k_curveValid(c) THEN
        line = line & " 1"
      ELSE
        line = line & " 0"
      END IF
    NEXT
    io::print(line)
  END IF
  IF mode = "eps" THEN
    LET t AS List OF Float = floats(os::getEnvOr("T", ""))
    io::print(exact(k_curveEps(Transform[a := collections::getOr(t, 0, 0.0), b := collections::getOr(t, 1, 0.0), c := collections::getOr(t, 2, 0.0), d := collections::getOr(t, 3, 0.0), tx := collections::getOr(t, 4, 0.0), ty := collections::getOr(t, 5, 0.0)])))
  END IF
  io::print("done")
END FUNC
"#;

/// The driver, built once for every test in this file.
fn driver() -> &'static PathBuf {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    BUILT.get_or_init(|| {
        let source = format!(
            "IMPORT collections\nIMPORT io\nIMPORT math\nIMPORT os\nIMPORT strings\nIMPORT vector\n\n\
             TYPE BezierCurve\n  points AS List OF vector::Float2\n  weights AS List OF Float\nEND TYPE\n\n\
             TYPE Transform\n  a AS Float\n  b AS Float\n  c AS Float\n  d AS Float\n  tx AS Float\n  ty AS Float\nEND TYPE\n\n\
             {}\n\n{}\n\n{}\n{DRIVER}",
            chunk("helper_paint_defaults.rs", "const TRANSFORM_ACTIVE: &str =\nr#\""),
            chunk("helper_curves.rs", "pub(super) const CURVE_MATH: &str =\nr#\""),
            chunk("helper_curves.rs", "pub(super) const CURVE_CHAIN: &str =\nr#\""),
        );
        let project = common::temp_project("canvas_curves", &source);
        common::build_project(&project)
    })
}

/// Run the driver and answer its lines, the trailing `done` checked and removed.
fn run(env: &[(&str, String)]) -> Vec<String> {
    let mut command = Command::new(driver());
    for (key, value) in env {
        command.env(key, value);
    }
    let output = command.output().expect("run the curve driver");
    assert!(
        output.status.success(),
        "the driver {}:\n{}\n{}",
        common::exit_description(&output.status),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let mut lines: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_string)
        .collect();
    assert_eq!(lines.pop().as_deref(), Some("done"), "the driver ran to its end");
    lines
}

/// One printed float: `hi:lo`, `v · 2^20` split at the binary point (`lo` in 2^-40ths).
fn value(token: &str) -> f64 {
    let (hi, lo) = token.split_once(':').expect("hi:lo");
    let hi: i64 = hi.parse().expect("hi");
    let lo: i64 = lo.parse().expect("lo");
    (hi as f64 + lo as f64 / 1_099_511_627_776.0) / 1_048_576.0
}

/// A piece: `x0, y0, x1, y1, x2, y2, w1`.
type Piece = [f64; 7];

fn pieces(lines: &[String]) -> Vec<Piece> {
    lines
        .iter()
        .filter(|line| line.starts_with("piece "))
        .map(|line| {
            let v: Vec<f64> = line.split_whitespace().skip(1).map(value).collect();
            assert_eq!(v.len(), 7, "{line}");
            [v[0], v[1], v[2], v[3], v[4], v[5], v[6]]
        })
        .collect()
}

fn marks(lines: &[String]) -> Vec<usize> {
    lines
        .iter()
        .find(|line| line.starts_with("marks"))
        .expect("a marks line")
        .split_whitespace()
        .skip(1)
        .map(|m| m.parse().expect("a mark"))
        .collect()
}

/// A curve as the test states it: control points, and weights or none.
#[derive(Clone)]
struct Curve {
    points: Vec<(f64, f64)>,
    weights: Vec<f64>,
}

fn poly(points: &[(f64, f64)]) -> Curve {
    Curve {
        points: points.to_vec(),
        weights: vec![],
    }
}

fn rational(points: &[(f64, f64)], weights: &[f64]) -> Curve {
    Curve {
        points: points.to_vec(),
        weights: weights.to_vec(),
    }
}

impl Curve {
    /// The driver's spelling: `x,y,x,y/w,w`.
    fn spell(&self) -> String {
        let xy: Vec<String> = self
            .points
            .iter()
            .map(|(x, y)| format!("{x:?},{y:?}"))
            .collect();
        let w: Vec<String> = self.weights.iter().map(|w| format!("{w:?}")).collect();
        format!("{}/{}", xy.join(","), w.join(","))
    }

    /// The point at `t`, by de Casteljau on the homogeneous points — written here, not
    /// read from the reduction.
    fn at(&self, t: f64) -> (f64, f64) {
        let mut h: Vec<(f64, f64, f64)> = self
            .points
            .iter()
            .enumerate()
            .map(|(i, (x, y))| {
                let w = self.weights.get(i).copied().unwrap_or(1.0);
                (w * x, w * y, w)
            })
            .collect();
        while h.len() > 1 {
            h = h
                .windows(2)
                .map(|p| {
                    (
                        (1.0 - t) * p[0].0 + t * p[1].0,
                        (1.0 - t) * p[0].1 + t * p[1].1,
                        (1.0 - t) * p[0].2 + t * p[1].2,
                    )
                })
                .collect();
        }
        (h[0].0 / h[0].2, h[0].1 / h[0].2)
    }
}

fn spell(curves: &[Curve]) -> String {
    curves.iter().map(Curve::spell).collect::<Vec<_>>().join("|")
}

/// One curve reduced at `eps`.
fn reduce(curve: &Curve, eps: f64) -> Vec<Piece> {
    pieces(&run(&[
        ("MODE", "one".to_string()),
        ("CURVES", curve.spell()),
        ("EPS", format!("{eps:?}")),
    ]))
}

/// A chain reduced at the default tolerance: its pieces and its marks.
fn chain(curves: &[Curve], closed: bool) -> (Vec<Piece>, Vec<usize>) {
    let lines = run(&[
        ("MODE", "chain".to_string()),
        ("CURVES", spell(curves)),
        ("CLOSED", if closed { "1" } else { "0" }.to_string()),
    ]);
    (pieces(&lines), marks(&lines))
}

/// The point of a piece at `t`.
fn piece_at(p: &Piece, t: f64) -> (f64, f64) {
    let s = 1.0 - t;
    let (a, b, c) = (s * s, 2.0 * p[6] * s * t, t * t);
    let w = a + b + c;
    (
        (a * p[0] + b * p[2] + c * p[4]) / w,
        (a * p[1] + b * p[3] + c * p[5]) / w,
    )
}

fn distance(a: (f64, f64), b: (f64, f64)) -> f64 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

/// How far `q` is from a piece: the best of 256 samples, then a golden-section search
/// of the two intervals either side of it.
fn piece_distance(p: &Piece, q: (f64, f64)) -> f64 {
    let n = 256;
    let mut best = (f64::INFINITY, 0usize);
    for i in 0..=n {
        let d = distance(piece_at(p, i as f64 / n as f64), q);
        if d < best.0 {
            best = (d, i);
        }
    }
    let mut lo = (best.1.saturating_sub(1)) as f64 / n as f64;
    let mut hi = ((best.1 + 1).min(n)) as f64 / n as f64;
    let ratio = 0.618_033_988_749_894_9;
    for _ in 0..60 {
        let a = hi - (hi - lo) * ratio;
        let b = lo + (hi - lo) * ratio;
        if distance(piece_at(p, a), q) < distance(piece_at(p, b), q) {
            hi = b;
        } else {
            lo = a;
        }
    }
    best.0.min(distance(piece_at(p, (lo + hi) / 2.0), q))
}

fn segment_distance(a: (f64, f64), b: (f64, f64), q: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 {
        (((q.0 - a.0) * dx + (q.1 - a.1) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    distance((a.0 + t * dx, a.1 + t * dy), q)
}

/// The dense deviation between a curve and its pieces, both ways: the furthest any of
/// 4096 points of the curve is from the pieces, and the furthest any of 256 points of
/// any piece is from the curve (as its 4096-chord polyline, which is within `1e-4` of
/// it for everything here).
fn dense_deviation(curve: &Curve, reduced: &[Piece]) -> f64 {
    let samples: Vec<(f64, f64)> = (0..=4096).map(|i| curve.at(i as f64 / 4096.0)).collect();
    let mut worst: f64 = 0.0;
    for q in &samples {
        let nearest = reduced
            .iter()
            .map(|p| piece_distance(p, *q))
            .fold(f64::INFINITY, f64::min);
        worst = worst.max(nearest);
    }
    for p in reduced {
        for i in 0..=256 {
            let q = piece_at(p, i as f64 / 256.0);
            let nearest = samples
                .windows(2)
                .map(|s| segment_distance(s[0], s[1], q))
                .fold(f64::INFINITY, f64::min);
            worst = worst.max(nearest);
        }
    }
    worst
}

/// Every piece is y-monotone on 256 samples. The slack is the rounding of evaluating a
/// piece (a few units in the last place of a coordinate), never a real reversal: a
/// piece that turned back would do so by the curve's own size.
fn assert_monotone(reduced: &[Piece], what: &str) {
    for (index, p) in reduced.iter().enumerate() {
        let rising = p[5] >= p[1];
        let slack = 1e-12 * (1.0 + p[1].abs().max(p[5].abs()));
        let mut last = p[1];
        for i in 1..=256 {
            let y = piece_at(p, i as f64 / 256.0).1;
            let step = if rising { y - last } else { last - y };
            assert!(
                step >= -slack,
                "{what}: piece {index} {p:?} turns back in y at sample {i} (by {step:e})"
            );
            last = y;
        }
    }
}

/// The pieces run end to end: each begins on the point the one before ended on,
/// exactly — a gap there would be a hole in a fill.
fn assert_connected(reduced: &[Piece], what: &str) {
    for pair in reduced.windows(2) {
        assert_eq!(
            (pair[0][4], pair[0][5]),
            (pair[1][0], pair[1][1]),
            "{what}: consecutive pieces do not share their joint"
        );
    }
}

const SQRT_HALF: f64 = std::f64::consts::FRAC_1_SQRT_2;

/// The cubic S used throughout: it leaves (0, 0) rightwards, ends at (300, 200)
/// rightwards, and has an inflection between.
fn cubic_s() -> Curve {
    poly(&[(0.0, 0.0), (200.0, 0.0), (100.0, 200.0), (300.0, 200.0)])
}

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
    assert_eq!(quarter.len(), 1, "a quarter circle is y-monotone: {quarter:?}");
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
    println!("cubic S: {} pieces, dense max deviation {deviation:.6}", reduced.len());
    assert!(deviation <= EPS, "the S deviates by {deviation}");
    assert!(reduced.len() <= 16, "{} pieces for one cubic", reduced.len());
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
    println!("degree 16: {} pieces, dense max deviation {deviation:.6}", reduced.len());
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
    assert!(deviation <= EPS, "the rational cubic deviates by {deviation}");
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
        println!("{what}: {} pieces, dense max deviation {deviation:.6}", reduced.len());
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

// ---- The two shapes in a scene ----------------------------------------------------
//
// From here down the tests run a real headless `--app` program, as
// `rt_canvas_rasteriser.rs` and `rt_canvas_hit.rs` do, and ask what a program can
// observe: the frame (`MFB_CANVAS_DUMP`), the cache's counters (`MFB_CANVAS_STATS`)
// and `canvas::MouseEvent.overID`.

/// Surface dimensions, as `rt_canvas_rasteriser.rs` has them.
const WIDTH: usize = 900;
const HEIGHT: usize = 640;

/// What one run of a scene program left behind.
struct Scene {
    stdout: String,
    frame: Vec<u8>,
    /// One line per frame presented.
    stats: Vec<String>,
}

impl Scene {
    fn pixel(&self, x: usize, y: usize) -> (u8, u8, u8, u8) {
        let i = (y * WIDTH + x) * 4;
        (
            self.frame[i],
            self.frame[i + 1],
            self.frame[i + 2],
            self.frame[i + 3],
        )
    }

    /// A counter of the last frame's stats line.
    fn stat(&self, name: &str) -> String {
        self.stats_of(name).pop().expect("a frame was presented")
    }

    /// A counter, frame by frame.
    fn stats_of(&self, name: &str) -> Vec<String> {
        let key = format!("{name}=");
        self.stats
            .iter()
            .map(|line| {
                line.split_whitespace()
                    .find_map(|field| field.strip_prefix(key.as_str()))
                    .unwrap_or_else(|| panic!("no {name} field in {line:?}"))
                    .to_string()
            })
            .collect()
    }

    /// The `overID` of each injected press, as `report` printed them.
    fn over(&self) -> Vec<String> {
        self.stdout
            .lines()
            .find(|line| line.starts_with("over"))
            .expect("the program reported")
            .split('[')
            .skip(1)
            .map(|part| part.trim_end().trim_end_matches(']').to_string())
            .collect()
    }
}

/// What every scene below is written with.
const PRELUDE: &str = r#"IMPORT app
IMPORT canvas
IMPORT collections
IMPORT color
IMPORT io
IMPORT vector

FUNC at(x AS Float, y AS Float) AS vector::Float2
  RETURN vector::Float2[x := x, y := y]
END FUNC

FUNC none() AS canvas::Transform
  RETURN canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
END FUNC

' A straight curve from (x0, y0) to (x1, y1).
FUNC line(x0 AS Float, y0 AS Float, x1 AS Float, y1 AS Float) AS canvas::BezierCurve
  RETURN canvas::BezierCurve[points := [at(x0, y0), at(x1, y1)], weights := []]
END FUNC

' A conic through three points with middle weight `w`.
FUNC conic(x0 AS Float, y0 AS Float, x1 AS Float, y1 AS Float, x2 AS Float, y2 AS Float, w AS Float) AS canvas::BezierCurve
  RETURN canvas::BezierCurve[points := [at(x0, y0), at(x1, y1), at(x2, y2)], weights := [1.0, w, 1.0]]
END FUNC

' A float to every bit: `v * 2^20` split at the binary point, the fraction in 2^-40ths.
FUNC exact(v AS Float) AS String
  LET scaled AS Float = v * 1048576.0
  LET hi AS Integer = toInt(scaled)
  LET lo AS Integer = toInt((scaled - toFloat(hi)) * 1099511627776.0)
  RETURN toString(hi) & ":" & toString(lo)
END FUNC

' One line a curve: `tag x y x y ... / w w ...`, after a line counting them.
FUNC showCurves(tag AS String, curves AS List OF canvas::BezierCurve) AS Nothing
  io::print(tag & "-count " & toString(len(curves)))
  FOR EACH c IN curves
    MUT text AS String = tag
    FOR EACH q IN c.points
      text = text & " " & exact(q.x) & " " & exact(q.y)
    NEXT
    text = text & " /"
    FOR EACH w IN c.weights
      text = text & " " & exact(w)
    NEXT
    io::print(text)
  NEXT
END FUNC

' One line: `tag x y x y ...`.
FUNC showPoints(tag AS String, points AS List OF vector::Float2) AS Nothing
  MUT text AS String = tag
  FOR EACH q IN points
    text = text & " " & exact(q.x) & " " & exact(q.y)
  NEXT
  io::print(text)
END FUNC

' Every injected press, in order, as the bracketed `overID` it reported.
FUNC report(tag AS String) AS Nothing
  canvas::enableMouse(TRUE)
  MUT text AS String = tag
  MUT n AS Integer = 0
  WHILE n < 70
    LET e AS canvas::MouseEvent = canvas::pollMouse()
    IF e.kind <> canvas::MouseKind.None THEN
      text = text & " [" & e.overID & "]"
    END IF
    n = n + 1
  END WHILE
  canvas::enableMouse(FALSE)
  io::print(text)
END FUNC
"#;

/// An SGR-1006 press of button 0 at pixel `(x, y)`.
fn press(x: u32, y: u32) -> String {
    format!("\x1b[<0;{};{}M", x + 1, y + 1)
}

/// Build `PRELUDE`, a `main` whose body is `body`, and `functions` after it; run it
/// headless with `presses` injected, and return what it left. The build is `--debug`:
/// the dump and the stats exist only there, and so does `MFB_CANVAS_GEO_VERIFY`, which
/// every run sets — the records a frame resolved are each rebuilt and compared, and a
/// test may assert the count of mismatches is zero.
fn scene(name: &str, body: &str, functions: &str, presses: &[(u32, u32)]) -> Scene {
    let source = format!(
        "{PRELUDE}\nFUNC main() AS Nothing\n  app::setMode(app::Mode.Canvas)\n{body}  io::print(\"rendered\")\nEND FUNC\n{functions}"
    );
    let project = common::temp_project(name, &source);
    let frame = project.join("frame.rgba");
    let stats = project.join("stats.txt");
    let binary = common::build_app_debug(&project, name);
    let inject: String = presses.iter().map(|(x, y)| press(*x, *y)).collect();
    let run = Command::new(&binary)
        .current_dir(&project)
        .env("MFB_MACAPP_HEADLESS", "1")
        .env("MFB_WINAPP_HEADLESS", "1")
        .env("MFB_GTKAPP_HEADLESS", "1")
        .env("MFB_CANVAS_DUMP", &frame)
        .env("MFB_CANVAS_STATS", &stats)
        .env("MFB_CANVAS_GEO_VERIFY", "1")
        .env("MFB_CANVAS_SYNC", "1")
        .env("MFB_MOUSE_INJECT", inject)
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", binary.display()));
    let stdout = String::from_utf8_lossy(&run.stdout).to_string();
    assert!(
        run.status.success() && stdout.contains("rendered"),
        "program {}:\n{stdout}\n{}",
        common::exit_description(&run.status),
        String::from_utf8_lossy(&run.stderr),
    );
    // A program that presents nothing leaves no dump, and its frame is empty.
    let pixels = std::fs::read(&frame).unwrap_or_default();
    assert!(
        pixels.is_empty() || pixels.len() == WIDTH * HEIGHT * 4,
        "the dump is one frame"
    );
    let lines = std::fs::read_to_string(&stats)
        .unwrap_or_default()
        .lines()
        .filter(|line| line.starts_with("generations="))
        .map(str::to_string)
        .collect();
    let _ = std::fs::remove_dir_all(&project);
    Scene {
        stdout,
        frame: pixels,
        stats: lines,
    }
}

/// A curve shape's hash covers everything it is made of, however deep: a
/// `CurvePolygon` holds lists of lists of records that hold lists, and a
/// geometry-cache hit is trusted on the item hash alone
/// (`.ai/canvas-threading.md` §14).
///
/// Four presents of one `CurvePolygon`: as it is; with ONE weight of the second
/// curve of its second contour changed; with only its `position` changed; and that
/// last one again, rebuilt. Each of the first three is a different item and costs one
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
  canvas::present([blob(60.0, 0.8)])
  canvas::present([wave(40.0)])
  canvas::present([wave(41.0)])
  canvas::present([wave(41.0)])
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

/// plan-164-A: both shapes are items every walk accepts, and until plan-164-B gives
/// them a record they draw nothing and are hit by nothing.
///
/// A red square with the id `under`, and over it: a filled `CurvePolygon`, a thickly
/// stroked `PolyCurve`, an `AutoPaint` around a `CurvePolygon` and a `Picture` cut to
/// one — each covering the square's centre, each with an `id`. The frame is the square
/// alone, and a press on its centre reports `under`: an item with no pixels does not
/// block what is beneath it.
#[cfg(unix)]
#[test]
fn canvas_curve_shapes_draw_nothing_and_block_nothing_yet() {
    let run = scene(
        "canvas_curve_absent",
        r#"  MUT under AS canvas::Rectangle
  under = WITH under { id := "under", position := at(100.0, 100.0), size := canvas::Size[width := 200.0, height := 200.0], paint := canvas::fill(color::rgb(255, 0, 0)) }
  LET underItem AS canvas::DrawItem = under
  LET blobItem AS canvas::DrawItem = square("blob")
  MUT wave AS canvas::PolyCurve
  wave = WITH wave { id := "wave", position := at(100.0, 200.0), curves := [line(0.0, 0.0, 200.0, 0.0)], paint := canvas::stroke(color::rgb(0, 0, 255), 40.0) }
  LET waveItem AS canvas::DrawItem = wave
  LET lit AS canvas::Shape = square("lit")
  LET green AS canvas::Paint = canvas::fill(color::rgb(0, 255, 0))
  LET autoItem AS canvas::DrawItem = canvas::AutoPaint[shape := lit, useEnter := FALSE, useExit := FALSE, useHover := TRUE, usePressed := FALSE, enterPaint := green, exitPaint := green, hoverPaint := green, pressedPaint := green]
  RES img AS canvas::Image = canvas::createImage(1, 1, [toByte(0), toByte(255), toByte(0), toByte(255)])
  LET cut AS canvas::AutoShape = square("cut")
  LET picItem AS canvas::DrawItem = canvas::Picture[id := "pic", shape := cut, clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img]
  canvas::present([underItem, blobItem, waveItem, autoItem, picItem])
  report("over")
"#,
        r#"
FUNC square(name AS String) AS canvas::CurvePolygon
  MUT s AS canvas::CurvePolygon
  s = WITH s { id := name, position := at(100.0, 100.0), contours := [[line(0.0, 0.0, 200.0, 0.0), line(200.0, 0.0, 200.0, 200.0), line(200.0, 200.0, 0.0, 200.0)]], paint := canvas::fillStroke(color::rgb(0, 255, 0), color::rgb(0, 0, 255), 6.0) }
  RETURN s
END FUNC
"#,
        &[(200, 200)],
    );
    assert_eq!(run.over(), ["under"]);
    assert_eq!(run.pixel(200, 200), (255, 0, 0, 255), "the square's centre");
    assert_eq!(run.pixel(100, 100), (255, 0, 0, 255), "its first pixel");
    let other = run
        .frame
        .chunks(4)
        .filter(|p| *p != [0, 0, 0, 255] && *p != [255, 0, 0, 255])
        .count();
    assert_eq!(other, 0, "nothing but the square and the background is drawn");
    assert_eq!(run.stat("geoVerifyMismatches"), "0");
}

// ---- canvas::nurbsToBeziers and canvas::flatten (plan-164-A Phase 2) ----------------

impl Scene {
    /// The curves a `showCurves(tag, …)` printed, after checking its count line.
    fn curves(&self, tag: &str) -> Vec<Curve> {
        let count: usize = self
            .stdout
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{tag}-count ")))
            .unwrap_or_else(|| panic!("no `{tag}-count` line:\n{}", self.stdout))
            .parse()
            .expect("a count");
        let prefix = format!("{tag} ");
        let curves: Vec<Curve> = self
            .stdout
            .lines()
            .filter_map(|line| line.strip_prefix(prefix.as_str()))
            .map(|line| {
                let (xy, w) = line.split_once('/').expect("points / weights");
                let xy: Vec<f64> = xy.split_whitespace().map(value).collect();
                Curve {
                    points: xy.chunks(2).map(|p| (p[0], p[1])).collect(),
                    weights: w.split_whitespace().map(value).collect(),
                }
            })
            .collect();
        assert_eq!(curves.len(), count, "{tag}:\n{}", self.stdout);
        curves
    }

    /// The points a `showPoints(tag, …)` printed.
    fn points(&self, tag: &str) -> Vec<(f64, f64)> {
        let prefix = format!("{tag} ");
        let line = self
            .stdout
            .lines()
            .find_map(|line| {
                line.strip_prefix(prefix.as_str())
                    .or_else(|| (line == tag).then_some(""))
            })
            .unwrap_or_else(|| panic!("no `{tag}` line:\n{}", self.stdout));
        let xy: Vec<f64> = line.split_whitespace().map(value).collect();
        xy.chunks(2).map(|p| (p[0], p[1])).collect()
    }
}

/// A `Float` as MFBASIC source. A negative one is written as a subtraction, the way
/// the canvas sources write theirs.
fn float_source(v: f64) -> String {
    if v < 0.0 {
        format!("0.0 - {:?}", -v)
    } else {
        format!("{v:?}")
    }
}

/// A list of `Float` as MFBASIC source.
fn float_list(values: &[f64]) -> String {
    let spelled: Vec<String> = values.iter().map(|v| float_source(*v)).collect();
    format!("[{}]", spelled.join(", "))
}

/// A list of `vector::Float2` as MFBASIC source.
fn point_list(points: &[(f64, f64)]) -> String {
    let spelled: Vec<String> = points
        .iter()
        .map(|(x, y)| format!("at({}, {})", float_source(*x), float_source(*y)))
        .collect();
    format!("[{}]", spelled.join(", "))
}

/// A NURBS curve, evaluated directly: the Cox–de Boor recurrence over the knot vector,
/// on the weighted points. Nothing here is shared with the conversion under test.
struct Nurbs {
    points: Vec<(f64, f64)>,
    weights: Vec<f64>,
    knots: Vec<f64>,
    degree: usize,
}

impl Nurbs {
    fn basis(&self, i: usize, p: usize, u: f64) -> f64 {
        if p == 0 {
            return if self.knots[i] <= u && u < self.knots[i + 1] {
                1.0
            } else {
                0.0
            };
        }
        let mut value = 0.0;
        let left = self.knots[i + p] - self.knots[i];
        if left > 0.0 {
            value += (u - self.knots[i]) / left * self.basis(i, p - 1, u);
        }
        let right = self.knots[i + p + 1] - self.knots[i + 1];
        if right > 0.0 {
            value += (self.knots[i + p + 1] - u) / right * self.basis(i + 1, p - 1, u);
        }
        value
    }

    fn at(&self, u: f64) -> (f64, f64) {
        let (mut x, mut y, mut w) = (0.0, 0.0, 0.0);
        for (i, (px, py)) in self.points.iter().enumerate() {
            let weight = self.weights.get(i).copied().unwrap_or(1.0);
            let b = self.basis(i, self.degree, u) * weight;
            x += b * px;
            y += b * py;
            w += b;
        }
        (x / w, y / w)
    }

    /// The distinct knots of the domain `knots[degree] ..= knots[points]`.
    fn breaks(&self) -> Vec<f64> {
        let (first, last) = (self.knots[self.degree], self.knots[self.points.len()]);
        let mut out: Vec<f64> = vec![];
        for k in &self.knots {
            if *k >= first && *k <= last && out.last() != Some(k) {
                out.push(*k);
            }
        }
        out
    }

    /// The call that converts it, printed under `tag`.
    fn call(&self, tag: &str) -> String {
        format!(
            "  showCurves(\"{tag}\", canvas::nurbsToBeziers({}, {}, {}, {}))\n",
            point_list(&self.points),
            float_list(&self.weights),
            float_list(&self.knots),
            self.degree
        )
    }

    /// Every Bézier the conversion returned agrees with the NURBS itself, at 256
    /// parameters across each span, to `tolerance`.
    fn assert_matches(&self, beziers: &[Curve], tolerance: f64, what: &str) {
        let breaks = self.breaks();
        assert_eq!(beziers.len(), breaks.len() - 1, "{what}: one Bézier a span");
        for (span, bezier) in beziers.iter().enumerate() {
            assert_eq!(
                bezier.points.len(),
                self.degree + 1,
                "{what}: the degree is kept"
            );
            let (lo, hi) = (breaks[span], breaks[span + 1]);
            for i in 0..256 {
                // Strictly inside the span: the recurrence is half-open at a knot.
                let t = (i as f64 + 0.5) / 256.0;
                let want = self.at(lo + t * (hi - lo));
                let got = bezier.at(t);
                assert!(
                    distance(want, got) <= tolerance,
                    "{what}: span {span} at t={t}: {got:?}, the NURBS is at {want:?}"
                );
            }
        }
    }
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
