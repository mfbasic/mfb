//! Bézier curves: the reduction to y-monotone rational-quadratic pieces, validation
//! and the chain walk (plan-164-A).
//!
//! The reduction is MFBASIC the compiler injects
//! (`src/codegen/builtins/canvas/helper_curves/pieces.rs:CURVE_BASE`, `CURVE_VALID` and
//! `CURVE_NEAREST`; `helper_curves/reduce.rs:CURVE_REDUCE` and `CURVE_CHAIN`). The
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

#[path = "rt_canvas_curves/curve_shapes.rs"]
mod curve_shapes;
#[path = "rt_canvas_curves/picture_curves.rs"]
mod picture_curves;
#[path = "rt_canvas_curves/reduction_and_pieces.rs"]
mod reduction_and_pieces;

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
            chunk("helper_curves/pieces.rs", "pub(super) const CURVE_BASE: &str =\nr#\""),
            chunk("helper_curves/reduce.rs", "pub(super) const CURVE_REDUCE: &str =\nr#\""),
            chunk("helper_curves/pieces.rs", "pub(super) const CURVE_VALID: &str =\nr#\""),
            chunk("helper_curves/reduce.rs", "pub(super) const CURVE_CHAIN: &str =\nr#\""),
            chunk("helper_curves/pieces.rs", "pub(super) const CURVE_NEAREST: &str =\nr#\""),
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

// ---- plan-164-B Phase 1: the fill and the distance -----------------------------------

const BLACK: (u8, u8, u8, u8) = (0, 0, 0, 255);
const WHITE: (u8, u8, u8, u8) = (255, 255, 255, 255);
const BLUE: (u8, u8, u8, u8) = (0, 0, 255, 255);
