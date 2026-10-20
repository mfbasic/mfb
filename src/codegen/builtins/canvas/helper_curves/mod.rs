//! Bézier curves: validation, and the reduction of every curve to y-monotone
//! rational-quadratic pieces (plan-164-A).
//!
//! A `canvas::PolyCurve` and a `canvas::CurvePolygon` hold `canvas::BezierCurve`s of
//! any kind — a line, a quadratic, a cubic, a rational curve, anything up to degree 16.
//! No renderer draws those. Each curve is **reduced once, when the item's geometry is
//! built**, to a list of pieces of ONE type, and the three renderers draw only that:
//!
//! ```text
//! P(t) = (P0·(1−t)² + 2·w1·P1·t(1−t) + P2·t²) / ((1−t)² + 2·w1·t(1−t) + t²)
//! ```
//!
//! a rational quadratic with its end weights normalised to 1, stored as seven floats
//! `x0, y0, x1, y1, x2, y2, w1` ([`PIECE_FLOATS`]). A polynomial piece has `w1 = 1`; a
//! straight segment is the piece whose `P1` is its midpoint.
//!
//! Every piece is **y-monotone**. That is what lets the fill test be the half-open
//! endpoint rule a polygon edge uses — `(y0 > py) <> (y2 > py)` — with the crossing's
//! `x` from one quadratic root: a piece that turned back in `y` would be crossed twice
//! by one scanline and counted once.
//!
//! ## What is exact and what is fitted
//!
//! - Degree 1 is one line piece.
//! - Degree 2, polynomial or rational, is **exact**: the piece is the curve, with the
//!   weights normalised (`w1 / sqrt(w0·w2)`, a reparametrisation that keeps the shape).
//!   A rational quadratic is a conic, so circles and ellipse arcs stay exact.
//! - Degree 3 and above is **fitted**: the curve is halved by de Casteljau, in
//!   homogeneous coordinates, until each span is within `eps` of a polynomial quadratic
//!   through its two ends whose control point is where the end tangents meet.
//!
//! ## Why the math is its own chunks
//!
//! [`CURVE_BASE`] and [`CURVE_REDUCE`] use nothing of the canvas package — only `Float`
//! lists, `math::sqrt` and `collections::` — so `tests/canvas/rt_canvas_curves.rs` can
//! lift their text out of this file and run it as a program of its own, the way
//! `rt_canvas_auto_step.rs` runs `AUTO_STEP`. What is tested is the text the compiler
//! injects.
//!
//! ## A canvas program pays for this file only if it can hold a curve shape
//!
//! Everything that reduces, builds and draws is one chunk gated on the program NAMING
//! `PolyCurve` or `CurvePolygon` as a type ([`HelperGate::WhenCalledOrNamed`]); every
//! other canvas program gets [`ABSENT`] in its place, the same ten names with the
//! bodies that are right when no curve shape can exist. Measured before the gate, the
//! chunk was 161 KB of code in every canvas program (`otool -l`, `__text`, macOS
//! AArch64, a scene of circles).
//!
//! The gate is on the type being named, not constructed as `AutoPaint`'s is: every
//! field of the curve records has a default, so `MUT c AS canvas::PolyCurve` followed by
//! `WITH c { … }` holds a curve no constructor expression made, and a constructor gate
//! would hand that program the stand-in and draw nothing. A program whose source never
//! names either shape cannot hold one.
//!
//! `canvas::flatten` and `canvas::nurbsToBeziers` are gated the same way, on being
//! CALLED: a function's registered body is compiled whether or not it is called, so
//! each is one call into [`CURVE_FUNCS`], and a program that calls neither gets
//! [`FUNCS_ABSENT`]. [`CURVE_BASE`] and [`CURVE_VALID`] — halving, flattening, knot
//! insertion, validity — are what both gated chunks stand on, and ride with either.
//! A canvas program that touches no curve carries the arms and the two stand-ins.

use crate::codegen::registry::{HelperGate, RegistryHelper, RegistryPackage};

/// The record types whose being NAMED opens the gated chunk: the two shapes. A
/// `BezierCurve` alone opens nothing — it is not drawn until a shape holds it.
pub(super) const NAMED: &[&str] = &["PolyCurve", "CurvePolygon"];

/// The floats one piece occupies in a list of pieces: `x0, y0, x1, y1, x2, y2, w1`.
/// `__CANVAS_CURVE_PIECE` in [`CURVE_BASE`] is the same number, and the GPU emitters
/// step a record's tail by it.
#[cfg(test)]
use crate::codegen::runtime::canvas::CURVE_PIECE_SLOTS as PIECE_FLOATS;

mod build;
mod draw;
mod pieces;
mod reduce;
use build::CURVE_BUILD;
use draw::CURVE_DRAW;
use pieces::{CURVE_BASE, CURVE_NEAREST, CURVE_VALID};
use reduce::{CURVE_CHAIN, CURVE_REDUCE};

/// The names whose being CALLED opens [`CURVE_FUNCS`]. Matched by final segment, so a
/// program calling `collections::flatten` carries the chunk too — unnecessarily, never
/// wrongly.
pub(super) const CALLED: &[&str] = &["nurbsToBeziers", "flatten"];

/// The bodies of `canvas::nurbsToBeziers` and `canvas::flatten`.
#[rustfmt::skip]
pub(super) const CURVE_FUNCS: &str =
r#"FUNC __canvas_nurbsImpl(controlPoints AS List OF vector::Float2, weights AS List OF Float, knots AS List OF Float, degree AS Integer) AS List OF BezierCurve
  MUT out AS List OF BezierCurve = []
  LET n AS Integer = len(controlPoints)
  IF degree < 1 OR degree > 16 THEN
    RETURN out
  END IF
  IF n < degree + 1 OR len(knots) <> n + degree + 1 THEN
    RETURN out
  END IF
  LET rational AS Boolean = len(weights) > 0
  IF rational AND len(weights) <> n THEN
    RETURN out
  END IF
  FOR EACH w IN weights
    IF NOT (w > 0.0) THEN
      RETURN out
    END IF
  NEXT
  MUT prev AS Float = collections::getOr(knots, 0, 0.0)
  FOR EACH knot IN knots
    IF knot < prev THEN
      RETURN out
    END IF
    prev = knot
  NEXT
  ' The domain: the knots the curve has a full set of control points between.
  LET first AS Float = collections::getOr(knots, degree, 0.0)
  LET last AS Float = collections::getOr(knots, n, 0.0)
  IF NOT (first < last) THEN
    RETURN out
  END IF
  LET origin AS vector::Float2 = vector::Float2[x := 0.0, y := 0.0]
  MUT h AS List OF Float = []
  MUT i AS Integer = 0
  WHILE i < n
    LET q AS vector::Float2 = collections::getOr(controlPoints, i, origin)
    MUT w AS Float = 1.0
    IF rational THEN
      w = collections::getOr(weights, i, 1.0)
    END IF
    h = collections::append(h, w * q.x)
    h = collections::append(h, w * q.y)
    h = collections::append(h, w)
    i = i + 1
  END WHILE
  LET pts AS List OF Float = __canvas_cvRefine(h, knots, degree)
  LET u AS List OF Float = __CANVAS_CURVE_KNOTS
  ' Every knot of the domain now has multiplicity `degree`, so the span that begins at
  ' the last copy of one -- index k -- is the Bezier whose points are k - degree to k.
  MUT k AS Integer = degree
  WHILE k + 1 < len(u)
    LET lo AS Float = collections::getOr(u, k, 0.0)
    IF lo < collections::getOr(u, k + 1, 0.0) AND lo >= first AND lo < last THEN
      MUT points AS List OF vector::Float2 = []
      MUT ws AS List OF Float = []
      MUT j AS Integer = k - degree
      WHILE j <= k
        LET pw AS Float = collections::getOr(pts, j * 3 + 2, 1.0)
        points = collections::append(points, vector::Float2[x := collections::getOr(pts, j * 3, 0.0) / pw, y := collections::getOr(pts, j * 3 + 1, 0.0) / pw])
        IF rational THEN
          ws = collections::append(ws, pw)
        END IF
        j = j + 1
      END WHILE
      out = collections::append(out, BezierCurve[points := points, weights := ws])
    END IF
    k = k + 1
  END WHILE
  RETURN out
END FUNC

FUNC __canvas_flattenImpl(curve AS BezierCurve, tolerance AS Float) AS List OF vector::Float2
  MUT out AS List OF vector::Float2 = []
  IF NOT __canvas_curveValid(curve) THEN
    RETURN out
  END IF
  MUT tol AS Float = tolerance
  IF tol < 0.001 THEN
    tol = 0.001
  END IF
  LET origin AS vector::Float2 = vector::Float2[x := 0.0, y := 0.0]
  LET n AS Integer = len(curve.points)
  LET none AS List OF Float = []
  LET ends AS List OF Float = __canvas_cvFlatten(none, __canvas_curveHomog(curve, 0.0, 0.0), n, tol, 0)
  ' The two ends are the curve's own, as written: a rational curve's come back from the
  ' halving as `(w x) / w`, which can be one place off.
  out = collections::append(out, collections::getOr(curve.points, 0, origin))
  MUT i AS Integer = 0
  WHILE i + 3 < len(ends)
    out = collections::append(out, vector::Float2[x := collections::getOr(ends, i, 0.0), y := collections::getOr(ends, i + 1, 0.0)])
    i = i + 2
  END WHILE
  RETURN collections::append(out, collections::getOr(curve.points, n - 1, origin))
END FUNC"#;

/// [`CURVE_FUNCS`] for a program that calls neither function: the two names its
/// registered bodies call, which nothing then reaches.
#[rustfmt::skip]
const FUNCS_ABSENT: &str =
r#"IMPORT canvas
IMPORT collections
IMPORT vector

FUNC __canvas_nurbsImpl(controlPoints AS List OF vector::Float2, weights AS List OF Float, knots AS List OF Float, degree AS Integer) AS List OF BezierCurve
  RETURN []
END FUNC

FUNC __canvas_flattenImpl(curve AS BezierCurve, tolerance AS Float) AS List OF vector::Float2
  RETURN []
END FUNC"#;

/// What the unconditional source calls, by name, into the gated chunk.
#[cfg(test)]
const HOOKS: [&str; 10] = [
    "__canvas_polyCurveRecord",
    "__canvas_curvePolygonRecord",
    "__canvas_polyCurveHash",
    "__canvas_curvePolygonHash",
    "__canvas_curveScan",
    "__canvas_curveDistance",
    "__canvas_curveJoined",
    "__canvas_dashedCurves",
    "__canvas_curveRings",
    "__canvas_curveExtent",
];

/// [`HOOKS`] for a program that names neither curve shape
/// ([`HelperGate::WhenNeitherCalledNorNamed`]). Such a program holds none, so none of
/// these can be reached with one; they are here because the item unions have the two
/// variants in every program, and a `MATCH` over them must have its arms. Each answers
/// what an absent shape is: the empty record, a distance far outside, no outline.
#[rustfmt::skip]
const ABSENT: &str =
r#"IMPORT canvas
IMPORT collections
IMPORT vector

FUNC __canvas_polyCurveRecord(pc AS PolyCurve) AS List OF List OF Float
  LET none AS List OF Float = []
  RETURN [__canvas_emptyHeader(), none]
END FUNC

FUNC __canvas_curvePolygonRecord(cp AS CurvePolygon) AS List OF List OF Float
  LET none AS List OF Float = []
  RETURN [__canvas_emptyHeader(), none]
END FUNC

FUNC __canvas_polyCurveHash(pc AS PolyCurve) AS Integer
  RETURN __canvas_hashStart(12)
END FUNC

FUNC __canvas_curvePolygonHash(cp AS CurvePolygon) AS Integer
  RETURN __canvas_hashStart(13)
END FUNC

FUNC __canvas_curveScan(tail AS Integer, pieces AS Integer, px AS Float, py AS Float, half AS Float, cap AS Integer) AS Float
  RETURN 1000000.0
END FUNC

FUNC __canvas_curveDistance(tail AS Integer, pieces AS Integer, px AS Float, py AS Float) AS Float
  RETURN 1000000.0
END FUNC

FUNC __canvas_curveJoined(offset AS Integer, tail AS Integer, pieces AS Integer, px AS Float, py AS Float, half AS Float) AS Float
  RETURN 1000000.0
END FUNC

FUNC __canvas_dashedCurves(kind AS Integer, offset AS Integer, px AS Float, py AS Float, half AS Float, cap AS Integer) AS Float
  RETURN 1000000.0
END FUNC

FUNC __canvas_curveRings(cp AS CurvePolygon) AS List OF List OF vector::Float2
  RETURN []
END FUNC

FUNC __canvas_curveExtent() AS List OF Float
  RETURN []
END FUNC"#;

/// The gated chunk: its own synthetic file, so it imports what it names.
static BODY: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    format!(
        "IMPORT canvas\nIMPORT collections\nIMPORT math\nIMPORT vector\n\n{CURVE_REDUCE}\n\n{CURVE_CHAIN}\n\n{CURVE_BUILD}\n\n{CURVE_NEAREST}\n\n{CURVE_DRAW}"
    )
});

/// The base both gated chunks stand on, as its own synthetic file.
static BASE: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    format!("IMPORT canvas\nIMPORT collections\nIMPORT math\nIMPORT vector\n\n{CURVE_BASE}\n\n{CURVE_VALID}")
});

static FUNCS: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    format!("IMPORT canvas\nIMPORT collections\nIMPORT vector\n\n{CURVE_FUNCS}")
});

pub(crate) fn register(pkg: &mut RegistryPackage) {
    let gated = |name: &'static str, gate: HelperGate, body: &'static str| RegistryHelper {
        name,
        gate,
        body: Some(body),
        import_name: None,
        natively_called: false,
    };
    // Needed by either gated chunk, so opened by either's condition; nothing
    // unconditional names it, so it has no stand-in.
    pkg.add_helper(gated(
        "canvas_curveBase",
        HelperGate::WhenCalledOrNamed(CALLED, NAMED),
        BASE.as_str(),
    ));
    pkg.add_helper(gated(
        "canvas_curveFuncs",
        HelperGate::WhenCalledOrNamed(CALLED, &[]),
        FUNCS.as_str(),
    ));
    pkg.add_helper(gated(
        "canvas_curveFuncs_absent",
        HelperGate::WhenNeitherCalledNorNamed(CALLED, &[]),
        FUNCS_ABSENT,
    ));
    pkg.add_helper(RegistryHelper {
        name: "canvas_curves",
        gate: HelperGate::WhenCalledOrNamed(&[], NAMED),
        body: Some(BODY.as_str()),
        import_name: None,
        natively_called: false,
    });
    pkg.add_helper(RegistryHelper {
        name: "canvas_curves_absent",
        gate: HelperGate::WhenNeitherCalledNorNamed(&[], NAMED),
        body: Some(ABSENT),
        import_name: None,
        natively_called: false,
    });
}

#[cfg(test)]
mod tests {
    use super::{
        ABSENT, BODY, CURVE_BASE, CURVE_FUNCS, CURVE_NEAREST, CURVE_REDUCE, FUNCS_ABSENT, HOOKS,
        PIECE_FLOATS,
    };

    /// The piece width is written twice: in the MFBASIC every reader of a piece list
    /// uses, and here for the Rust side. They are one number.
    #[test]
    fn canvas_curve_piece_width_is_pinned() {
        assert!(CURVE_BASE.contains(&format!(
            "LET __CANVAS_CURVE_PIECE AS Integer = {PIECE_FLOATS}\n"
        )));
    }

    /// Every `__canvas_` / `__CANVAS_` name `source` spells.
    fn names(source: &str) -> Vec<String> {
        let mut out = vec![];
        for prefix in ["__canvas_", "__CANVAS_"] {
            for (at, _) in source.match_indices(prefix) {
                out.push(
                    source[at..]
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect(),
                );
            }
        }
        out
    }

    /// `CURVE_BASE` and `CURVE_REDUCE` are lifted out of this file and run on their own
    /// by `tests/canvas/rt_canvas_curves.rs`, so they may name nothing of the canvas
    /// package but their own functions and constants; `CURVE_NEAREST` is lifted too, and
    /// may add the store and the two-argument minimum and maximum.
    #[test]
    fn canvas_curve_math_names_nothing_outside_itself() {
        // `__canvas_curveDashData` is the one function of the reduction not named `cv…`,
        // and it is defined there.
        let own = |name: &str| {
            name.starts_with("__canvas_cv")
                || name.starts_with("__CANVAS_CURVE_")
                || name == "__canvas_curveDashData"
        };
        for name in names(CURVE_BASE).into_iter().chain(names(CURVE_REDUCE)) {
            assert!(
                own(&name),
                "the reduction names `{name}`, which the harness does not have"
            );
        }
        for name in names(CURVE_NEAREST) {
            assert!(
                own(&name)
                    || name.starts_with("__canvas_pc")
                    || name.starts_with("__canvas_piece")
                    || name.starts_with("__CANVAS_PC_")
                    || name.starts_with("__CANVAS_PIECE_")
                    || ["__CANVAS_GEO_DATA", "__canvas_minF", "__canvas_maxF"]
                        .contains(&name.as_str()),
                "CURVE_NEAREST names `{name}`, which the harness does not have"
            );
        }
    }

    /// The signature line of `FUNC name(` in `source`.
    fn signature<'a>(source: &'a str, name: &str) -> &'a str {
        let opens = format!("FUNC {name}(");
        let at = source
            .find(&opens)
            .unwrap_or_else(|| panic!("no `{opens}` in the chunk"));
        source[at..].lines().next().expect("a signature line")
    }

    /// The two chunks are alternatives: every hook is declared in both, with the same
    /// parameters and the same return type, so the unconditional source compiles
    /// against either; and the stand-in declares nothing else.
    #[test]
    fn canvas_curve_hooks_are_declared_alike_in_both_chunks() {
        for hook in HOOKS {
            assert_eq!(signature(&BODY, hook), signature(ABSENT, hook), "{hook}");
        }
        let declared = ABSENT.matches("\nFUNC ").count();
        assert_eq!(
            declared,
            HOOKS.len(),
            "ABSENT declares a function that is not a hook"
        );
        for hook in ["__canvas_nurbsImpl", "__canvas_flattenImpl"] {
            assert_eq!(
                signature(CURVE_FUNCS, hook),
                signature(FUNCS_ABSENT, hook),
                "{hook}"
            );
        }
        assert_eq!(FUNCS_ABSENT.matches("\nFUNC ").count(), 2);
    }
}
