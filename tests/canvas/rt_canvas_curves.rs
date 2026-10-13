//! Bézier curves: the reduction to y-monotone rational-quadratic pieces, validation
//! and the chain walk (plan-164-A).
//!
//! The reduction is MFBASIC the compiler injects
//! (`src/codegen/builtins/canvas/helper_curves.rs:CURVE_BASE`, `CURVE_REDUCE`,
//! `CURVE_VALID`, `CURVE_CHAIN` and `CURVE_NEAREST`). The
//! compiler is a binary crate, so a test cannot link that text, and the `canvas`
//! package cannot be imported outside app mode; this file reads the chunks out of
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

' What `CURVE_NEAREST` names beyond `CURVE_MATH`: the store a record's pieces are read
' from, and the package's two-argument minimum and maximum.
MUT K_GEO_DATA AS List OF Float = []

FUNC k_minF(a AS Float, b AS Float) AS Float
  IF a < b THEN
    RETURN a
  END IF
  RETURN b
END FUNC

FUNC k_maxF(a AS Float, b AS Float) AS Float
  IF a > b THEN
    RETURN a
  END IF
  RETURN b
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
  ' One piece (`PIECE`, seven floats) against points (`POINTS`, x,y pairs): for each,
  ' the distance, the parameter of the nearest point, and the bound.
  IF mode = "nearest" THEN
    K_GEO_DATA = floats(os::getEnvOr("PIECE", ""))
    LET q AS List OF Float = floats(os::getEnvOr("POINTS", ""))
    MUT i AS Integer = 0
    WHILE i + 1 < len(q)
      LET d AS Float = k_pieceNearest(0, collections::getOr(q, i, 0.0), collections::getOr(q, i + 1, 0.0))
      io::print("near " & exact(d) & " " & exact(K_PIECE_T) & " " & exact(k_pieceBound(0, collections::getOr(q, i, 0.0), collections::getOr(q, i + 1, 0.0))))
      i = i + 2
    END WHILE
  END IF
  ' One piece against rows (`ROWS`, y values): the x it crosses each at.
  IF mode = "cross" THEN
    K_GEO_DATA = floats(os::getEnvOr("PIECE", ""))
    FOR EACH y IN floats(os::getEnvOr("ROWS", ""))
      io::print("cross " & exact(k_pieceCrossX(0, y)))
    NEXT
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
             {}\n\n{}\n\n{}\n\n{}\n\n{}\n\n{}\n{DRIVER}",
            chunk("helper_paint_defaults.rs", "const TRANSFORM_ACTIVE: &str =\nr#\""),
            chunk("helper_curves.rs", "pub(super) const CURVE_BASE: &str =\nr#\""),
            chunk("helper_curves.rs", "pub(super) const CURVE_REDUCE: &str =\nr#\""),
            chunk("helper_curves.rs", "pub(super) const CURVE_VALID: &str =\nr#\""),
            chunk("helper_curves.rs", "pub(super) const CURVE_CHAIN: &str =\nr#\""),
            chunk("helper_curves.rs", "pub(super) const CURVE_NEAREST: &str =\nr#\""),
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
    assert_eq!(
        lines.pop().as_deref(),
        Some("done"),
        "the driver ran to its end"
    );
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
    curves
        .iter()
        .map(Curve::spell)
        .collect::<Vec<_>>()
        .join("|")
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

' The four arcs of a circle of radius `r` about (cx, cy), each an exact conic, from its
' rightmost point round by its lowest (clockwise on the screen, where y runs down).
FUNC circle(cx AS Float, cy AS Float, r AS Float) AS List OF canvas::BezierCurve
  LET k AS Float = 0.7071067811865476
  RETURN [conic(cx + r, cy, cx + r, cy + r, cx, cy + r, k), conic(cx, cy + r, cx - r, cy + r, cx - r, cy, k), conic(cx - r, cy, cx - r, cy - r, cx, cy - r, k), conic(cx, cy - r, cx + r, cy - r, cx + r, cy, k)]
END FUNC

' The same circle the other way round: from its rightmost point by its highest.
FUNC circleBack(cx AS Float, cy AS Float, r AS Float) AS List OF canvas::BezierCurve
  LET k AS Float = 0.7071067811865476
  RETURN [conic(cx + r, cy, cx + r, cy - r, cx, cy - r, k), conic(cx, cy - r, cx - r, cy - r, cx - r, cy, k), conic(cx - r, cy, cx - r, cy + r, cx, cy + r, k), conic(cx, cy + r, cx + r, cy + r, cx + r, cy, k)]
END FUNC

' A cubic curve through four control points.
FUNC cubic(x0 AS Float, y0 AS Float, x1 AS Float, y1 AS Float, x2 AS Float, y2 AS Float, x3 AS Float, y3 AS Float) AS canvas::BezierCurve
  RETURN canvas::BezierCurve[points := [at(x0, y0), at(x1, y1), at(x2, y2), at(x3, y3)], weights := []]
END FUNC

' A `CurvePolygon` at (x, y).
FUNC blob(name AS String, x AS Float, y AS Float, contours AS List OF List OF canvas::BezierCurve, rule AS canvas::FillRule, paint AS canvas::Paint) AS canvas::DrawItem
  LET item AS canvas::DrawItem = canvas::CurvePolygon[id := name, position := at(x, y), contours := contours, rule := rule, paint := paint, transform := none()]
  RETURN item
END FUNC

' A `CurvePolygon` at (x, y) under `t`, as the shape a `Picture` is cut to.
FUNC curved(name AS String, x AS Float, y AS Float, contours AS List OF List OF canvas::BezierCurve, rule AS canvas::FillRule, paint AS canvas::Paint, t AS canvas::Transform) AS canvas::AutoShape
  LET shape AS canvas::AutoShape = canvas::CurvePolygon[id := name, position := at(x, y), contours := contours, rule := rule, paint := paint, transform := t]
  RETURN shape
END FUNC

' A `PolyCurve` at (x, y).
FUNC wave(name AS String, x AS Float, y AS Float, curves AS List OF canvas::BezierCurve, paint AS canvas::Paint) AS canvas::DrawItem
  LET item AS canvas::DrawItem = canvas::PolyCurve[id := name, position := at(x, y), curves := curves, paint := paint, transform := none()]
  RETURN item
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
    scene_on(name, body, functions, presses, false)
}

/// [`scene`], asking for the GPU renderer (`MFB_CANVAS_GPU=1`) when `gpu` is set.
fn scene_on(name: &str, body: &str, functions: &str, presses: &[(u32, u32)], gpu: bool) -> Scene {
    let source = format!(
        "{PRELUDE}\nFUNC main() AS Nothing\n  app::setMode(app::Mode.Canvas)\n{body}  io::print(\"rendered\")\nEND FUNC\n{functions}"
    );
    let project = common::temp_project(name, &source);
    let frame = project.join("frame.rgba");
    let stats = project.join("stats.txt");
    let binary = common::build_app_debug(&project, name);
    let inject: String = presses.iter().map(|(x, y)| press(*x, *y)).collect();
    let mut command = Command::new(&binary);
    if gpu {
        command.env("MFB_CANVAS_GPU", "1");
    }
    let run = command
        .current_dir(&project)
        .env("MFB_HEADLESS", "1")
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

// ---- plan-164-B Phase 1: the fill and the distance -----------------------------------

const BLACK: (u8, u8, u8, u8) = (0, 0, 0, 255);
const WHITE: (u8, u8, u8, u8) = (255, 255, 255, 255);
const BLUE: (u8, u8, u8, u8) = (0, 0, 255, 255);

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

// ---- plan-164-B Phase 4: a `Picture` cut to a `CurvePolygon` -------------------------

const RED: (u8, u8, u8, u8) = (255, 0, 0, 255);

/// `RES name AS canvas::Image = …`: an image of `w` x `h` opaque pixels, row by row.
fn image(name: &str, w: usize, h: usize, px: &[(u8, u8, u8)]) -> String {
    assert_eq!(px.len(), w * h);
    let bytes: Vec<String> = px
        .iter()
        .flat_map(|&(r, g, b)| [r, g, b, 255])
        .map(|v| format!("toByte({v})"))
        .collect();
    format!(
        "  RES {name} AS canvas::Image = canvas::createImage({w}, {h}, [{}])\n",
        bytes.join(", ")
    )
}

/// The coverage a white-on-black pixel shows, 0 to 1: the software renderer blends in
/// linear light, so a channel `v` is the sRGB encoding of the coverage.
fn coverage_of(v: u8) -> f64 {
    let c = v as f64 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// The largest distance between the edge a white picture's clip draws and the true
/// circle of radius `r` about `(cx, cy)`, in surface pixels, read off the frame: an
/// edge pixel's coverage is `0.5 - d` for its centre's signed distance `d` to the
/// clip's edge, so `d` is `0.5 - coverage`, and the true circle's is the centre's
/// distance less `r`. Only pixels between 15% and 85% covered are read — outside that
/// a byte of the frame is a larger step of distance than the gap being measured.
fn edge_gap(run: &Scene, cx: f64, cy: f64, r: f64) -> (f64, usize) {
    let mut worst: f64 = 0.0;
    let mut read = 0;
    let reach = r as i32 + 3;
    for dy in -reach..=reach {
        for dx in -reach..=reach {
            let (x, y) = ((cx as i32 + dx) as usize, (cy as i32 + dy) as usize);
            let c = coverage_of(run.pixel(x, y).0);
            if !(0.15..=0.85).contains(&c) {
                continue;
            }
            let true_d = ((x as f64 + 0.5 - cx).powi(2) + (y as f64 + 0.5 - cy).powi(2)).sqrt() - r;
            worst = worst.max(((0.5 - c) - true_d).abs());
            read += 1;
        }
    }
    (worst, read)
}

/// The image's clip is the curve flattened, and the gap between that clip's edge and
/// the true curve is within the reduction's tolerance, 0.05 surface pixels — under the
/// identity, and under a x8 `transform`, where the shape is a circle of radius 7.5 and
/// the flattening has to be eight times finer in its own space to stay there.
///
/// Beside each, the same white picture on a `canvas::Circle`, whose outline is
/// flattened by the package's own ring: the two edges are each within 0.05 px of the
/// circle, so no pixel of one differs from the other by more than a tenth of a pixel's
/// coverage, and none away from the edge differs at all.
#[test]
fn canvas_picture_curve_clip_is_within_the_tolerance_of_the_curve() {
    let white = [(255, 255, 255)];
    let body = image("img", 1, 1, &white)
        + r#"  LET plain AS canvas::Paint = canvas::fill(color::rgb(255, 255, 255))
  LET big AS canvas::Transform = canvas::Transform[8.0, 0.0, 0.0, 8.0, 0.0, 0.0]
  LET disc AS canvas::AutoShape = canvas::Circle[id := "", position := at(150.5, 150.25), radius := 60.0, paint := plain, transform := none()]
  canvas::present([canvas::Picture[id := "", shape := disc, clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img], canvas::Picture[id := "", shape := curved("", 450.5, 150.25, [circle(0.0, 0.0, 60.0)], canvas::FillRule.EvenOdd, plain, none()), clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img], canvas::Picture[id := "", shape := curved("", 150.5, 450.25, [circle(0.0, 0.0, 7.5)], canvas::FillRule.EvenOdd, plain, big), clip := canvas::PictureClip.ClipAndTransform, fit := canvas::PictureFit.Stretch, image := img], canvas::Picture[id := "", shape := curved("", 450.5, 450.25, [circle(0.0, 0.0, 7.5)], canvas::FillRule.EvenOdd, plain, big), clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img]])
"#;
    let run = scene("canvas_picture_curve_gap", &body, "", &[]);
    assert_eq!(run.pixel(450, 150), WHITE, "the middle of the picture");
    for (what, cx, cy) in [
        ("the identity", 450.5, 150.25),
        ("x8, ClipAndTransform", 150.5, 450.25),
        ("x8, ClipOnly", 450.5, 450.25),
    ] {
        let (gap, read) = edge_gap(&run, cx, cy, 60.0);
        println!("picture clip vs the true circle, {what}: worst gap {gap:.4} px over {read} edge pixels");
        assert!(read > 150, "{what}: the edge was read ({read} pixels)");
        // 0.05 is the flattening's own bound; the rest is the frame's resolution of a
        // distance, one byte of an sRGB channel at mid coverage.
        assert!(
            gap <= 0.05 + 0.006,
            "{what}: the clip is {gap} px from the curve"
        );
    }
    let mut worst: f64 = 0.0;
    for y in 80..221 {
        for x in 80..221 {
            let (a, b) = (run.pixel(x, y), run.pixel(x + 300, y));
            let delta = (coverage_of(a.0) - coverage_of(b.0)).abs();
            worst = worst.max(delta);
            let from_edge =
                (((x as f64 + 0.5 - 150.5).powi(2) + (y as f64 + 0.5 - 150.25).powi(2)).sqrt()
                    - 60.0)
                    .abs();
            if from_edge > 1.0 {
                assert_eq!(a, b, "({x}, {y}) is {from_edge:.2} px from the edge");
            }
        }
    }
    println!(
        "picture on a CurvePolygon circle vs on a Circle: worst coverage difference {worst:.4}"
    );
    assert!(
        worst <= 0.1,
        "the two outlines are {worst} of a pixel apart"
    );
}

/// A picture on a two-contour `CurvePolygon` shows its image in the ring and not in
/// the hole, under both rules (the inner contour is wound against the outer, which is
/// a hole under both). And a stroked shape draws its outline over the image as the
/// curve: at (600, 150), radius 60 with a 6 px white stroke over a red image — the
/// outline is white on the radius, the image red 5 px inside it, nothing 5 px outside.
#[test]
fn canvas_picture_curve_holes_and_outline() {
    let body = image("img", 1, 1, &[(255, 0, 0)])
        + r#"  LET plain AS canvas::Paint = canvas::fill(color::rgb(255, 255, 255))
  LET ring AS List OF List OF canvas::BezierCurve = [circle(0.0, 0.0, 80.0), circleBack(0.0, 0.0, 35.0)]
  LET edged AS canvas::Paint = canvas::fillStroke(color::rgb(255, 255, 255), color::rgb(255, 255, 255), 6.0)
  canvas::present([canvas::Picture[id := "", shape := curved("", 150.0, 150.0, ring, canvas::FillRule.EvenOdd, plain, none()), clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img], canvas::Picture[id := "", shape := curved("", 380.0, 150.0, ring, canvas::FillRule.NonZero, plain, none()), clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img], canvas::Picture[id := "", shape := curved("", 600.0, 150.0, [circle(0.0, 0.0, 60.0)], canvas::FillRule.EvenOdd, edged, none()), clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img]])
"#;
    let run = scene("canvas_picture_curve_holes", &body, "", &[]);
    for (what, cx) in [("EvenOdd", 150), ("NonZero", 380)] {
        assert_eq!(run.pixel(cx + 55, 150), RED, "{what}: the ring");
        assert_eq!(run.pixel(cx, 150 - 55), RED, "{what}: the ring, above");
        assert_eq!(run.pixel(cx, 150), BLACK, "{what}: the hole");
        assert_eq!(
            run.pixel(cx + 20, 150 + 20),
            BLACK,
            "{what}: the hole again"
        );
        assert_eq!(run.pixel(cx + 84, 150), BLACK, "{what}: outside");
    }
    assert_eq!(
        run.pixel(600 + 59, 150),
        WHITE,
        "the outline, on the radius"
    );
    assert_eq!(
        run.pixel(600 + 61, 150),
        WHITE,
        "the outline, just outside it"
    );
    assert_eq!(run.pixel(600 + 54, 150), RED, "the image, 5.5 px inside");
    assert_eq!(run.pixel(600 + 65, 150), BLACK, "5.5 px outside");
    assert_eq!(run.pixel(600, 150), RED, "the middle");
}

/// The box an image is fitted to is the curve's TRUE extent, not its control points'.
///
/// A bump: one quadratic from (0, 0) through a control point at (100, -160) to
/// (200, 0), closed along y = 0. The curve only reaches y = -80; its control point is
/// twice as far. The image is 1 x 2, red over blue.
///
/// * `Stretch` (at x = 50): the box is 200 x 80, the seam at y = -40. Local (100, -60)
///   is red and (100, -20) blue. Fitted to the control points' box (200 x 160) the
///   seam would be at -80 and (100, -60) blue.
/// * `Contain` (at x = 330): the image keeps 1:2, so it is 40 x 80, centred on x = 100:
///   (100, -60) red, (100, -20) blue, and (60, -20) — inside the bump, outside the
///   image — nothing.
/// * `Cover` (at x = 610): the image is 200 x 400 centred on the box, so the seam is
///   still at -40: (100, -60) red, (100, -20) blue.
#[test]
fn canvas_picture_curve_fits_the_true_extent() {
    let body = image("img", 1, 2, &[(255, 0, 0), (0, 0, 255)])
        + r#"  LET plain AS canvas::Paint = canvas::fill(color::rgb(255, 255, 255))
  LET bump AS List OF List OF canvas::BezierCurve = [[canvas::BezierCurve[points := [at(0.0, 0.0), at(100.0, 0.0 - 160.0), at(200.0, 0.0)], weights := []]]]
  canvas::present([canvas::Picture[id := "", shape := curved("", 50.0, 300.0, bump, canvas::FillRule.EvenOdd, plain, none()), clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img], canvas::Picture[id := "", shape := curved("", 330.0, 300.0, bump, canvas::FillRule.EvenOdd, plain, none()), clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Contain, image := img], canvas::Picture[id := "", shape := curved("", 610.0, 300.0, bump, canvas::FillRule.EvenOdd, plain, none()), clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Cover, image := img]])
"#;
    let run = scene("canvas_picture_curve_fit", &body, "", &[]);
    for (what, x) in [("Stretch", 50), ("Contain", 330), ("Cover", 610)] {
        assert_eq!(run.pixel(x + 100, 300 - 60), RED, "{what}: above the seam");
        assert_eq!(run.pixel(x + 100, 300 - 20), BLUE, "{what}: below it");
        assert_eq!(
            run.pixel(x + 100, 300 - 42),
            RED,
            "{what}: 1.5 px above the seam"
        );
        assert_eq!(
            run.pixel(x + 100, 300 - 39),
            BLUE,
            "{what}: 1.5 px below it"
        );
        assert_eq!(
            run.pixel(x + 100, 300 - 84),
            BLACK,
            "{what}: above the bump"
        );
        assert_eq!(
            run.pixel(x + 100, 300 + 3),
            BLACK,
            "{what}: below the closing segment"
        );
    }
    assert_eq!(
        run.pixel(50 + 60, 300 - 20),
        BLUE,
        "Stretch: the image fills the bump"
    );
    assert_eq!(
        run.pixel(330 + 60, 300 - 20),
        BLACK,
        "Contain: inside the bump, outside the image"
    );
    assert_eq!(
        run.pixel(610 + 60, 300 - 20),
        BLUE,
        "Cover: the image covers the bump"
    );
}

/// `ClipAndTransform` turns the image with the shape; `ClipOnly` turns only the outline
/// and leaves the image upright, fitted to the box of the turned outline. The bump,
/// turned 30° about its `position`, with a 2 x 1 image, red beside blue.
///
/// * `ClipAndTransform` (at (150, 200)): the seam is local x = 100. Local (90, -60)
///   lands at (107.9, -7.0) and is red; local (110, -60) at (125.3, 3.0), blue.
/// * `ClipOnly` (at (500, 200)): the turned outline spans x from 0 to 173.2, so the
///   seam is the upright line x = 86.6. The same point, (107.9, -7.0), is blue; local
///   (30, -10), at (31.0, 6.3), is red.
///
/// And the shape's `paint.clip`, which keeps local x from 100 on (at (150, 450)), cuts
/// the picture as well as the outline does.
#[test]
fn canvas_picture_curve_clip_modes_and_the_paints_clip() {
    let body = image("img", 2, 1, &[(255, 0, 0), (0, 0, 255)])
        + r#"  LET plain AS canvas::Paint = canvas::fill(color::rgb(255, 255, 255))
  LET turn AS canvas::Transform = canvas::Transform[0.8660254037844386, 0.5, 0.0 - 0.5, 0.8660254037844386, 0.0, 0.0]
  LET bump AS List OF List OF canvas::BezierCurve = [[canvas::BezierCurve[points := [at(0.0, 0.0), at(100.0, 0.0 - 160.0), at(200.0, 0.0)], weights := []]]]
  LET cut AS canvas::Paint = WITH plain { clip := [[at(100.0, 0.0 - 200.0), at(300.0, 0.0 - 200.0), at(300.0, 200.0), at(100.0, 200.0)]] }
  canvas::present([canvas::Picture[id := "", shape := curved("", 150.0, 200.0, bump, canvas::FillRule.EvenOdd, plain, turn), clip := canvas::PictureClip.ClipAndTransform, fit := canvas::PictureFit.Stretch, image := img], canvas::Picture[id := "", shape := curved("", 500.0, 200.0, bump, canvas::FillRule.EvenOdd, plain, turn), clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img], canvas::Picture[id := "", shape := curved("", 150.0, 450.0, bump, canvas::FillRule.EvenOdd, cut, none()), clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img]])
"#;
    let run = scene("canvas_picture_curve_modes", &body, "", &[]);
    assert_eq!(
        run.pixel(150 + 107, 200 - 7),
        RED,
        "ClipAndTransform: local (90, -60)"
    );
    assert_eq!(
        run.pixel(150 + 125, 200 + 3),
        BLUE,
        "ClipAndTransform: local (110, -60)"
    );
    assert_eq!(
        run.pixel(150 + 31, 200 + 6),
        RED,
        "ClipAndTransform: local (30, -10)"
    );
    assert_eq!(
        run.pixel(500 + 107, 200 - 7),
        BLUE,
        "ClipOnly: right of the upright seam"
    );
    assert_eq!(run.pixel(500 + 31, 200 + 6), RED, "ClipOnly: left of it");
    // The unturned bump's top is local (100, -80), which is not on the turned one:
    // it is turned-local (46.6, -119).
    assert_eq!(
        run.pixel(150 + 100, 200 - 75),
        BLACK,
        "the outline is turned"
    );
    assert_eq!(run.pixel(500 + 100, 200 - 75), BLACK, "under both modes");
    // The paint's clip: the image is fitted to the whole bump (seam at x = 100), and
    // only local x >= 100 is left.
    assert_eq!(
        run.pixel(150 + 140, 450 - 30),
        BLUE,
        "right of the paint's clip"
    );
    assert_eq!(run.pixel(150 + 60, 450 - 30), BLACK, "left of it: cut away");
    assert_eq!(
        run.pixel(150 + 98, 450 - 30),
        BLACK,
        "1.5 px left of the clip's edge"
    );
    assert_eq!(run.pixel(150 + 101, 450 - 30), BLUE, "1.5 px right of it");
}

/// A picture on a `PolyCurve` draws nothing and is hit by nothing: a chain has no
/// inside to cut an image to, as a `PolyLine` has none. The frame is the square
/// beneath it alone, and a press where the chain would be reports the square.
#[cfg(unix)]
#[test]
fn canvas_picture_on_a_poly_curve_draws_nothing() {
    let body = image("img", 1, 1, &[(0, 255, 0)])
        + r#"  MUT under AS canvas::Rectangle
  under = WITH under { id := "under", position := at(100.0, 100.0), size := canvas::Size[width := 200.0, height := 200.0], paint := canvas::fill(color::rgb(255, 0, 0)) }
  LET underItem AS canvas::DrawItem = under
  LET chain AS canvas::AutoShape = canvas::PolyCurve[id := "chain", position := at(100.0, 200.0), curves := [line(0.0, 0.0, 200.0, 0.0)], paint := canvas::stroke(color::rgb(0, 0, 255), 40.0), transform := none()]
  canvas::present([underItem, canvas::Picture[id := "pic", shape := chain, clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img]])
  report("over")
"#;
    let run = scene("canvas_picture_poly_curve", &body, "", &[(200, 200)]);
    assert_eq!(run.over(), ["under"]);
    assert_eq!(run.pixel(200, 200), RED);
    let other = run
        .frame
        .chunks(4)
        .filter(|p| *p != [0, 0, 0, 255] && *p != [255, 0, 0, 255])
        .count();
    assert_eq!(
        other, 0,
        "nothing but the square and the background is drawn"
    );
}
