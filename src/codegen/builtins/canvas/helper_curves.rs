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

/// What both gated chunks stand on: the constants, and the halving, flattening and
/// knot insertion `canvas::flatten` and `canvas::nurbsToBeziers` are made of.
///
/// A curve arrives as `h`, three floats a control point: `w·x, w·y, w` (a polynomial
/// curve has every `w = 1`). Homogeneous coordinates are what make de Casteljau's
/// halving work unchanged for a rational curve.
#[rustfmt::skip]
pub(super) const CURVE_BASE: &str =
r#"' plan-164-A: the floats of one piece -- x0, y0, x1, y1, x2, y2, w1.
LET __CANVAS_CURVE_PIECE AS Integer = 7

' How many times a curve of degree 3 or more may be halved before a span is taken as
' it is. 2^16 spans of one curve is far below a pixel for anything a surface can show.
LET __CANVAS_CURVE_MAX_DEPTH AS Integer = 16

' The samples a fitted span is checked at.
LET __CANVAS_CURVE_FIT_SAMPLES AS Integer = 16

' plan-164-B: the steps a piece's arc length is tabulated at for dashes, and the floats
' of a piece's row in a dash tail -- its position at each step, and its contour's length.
LET __CANVAS_CURVE_ARC_STEPS AS Integer = 16

LET __CANVAS_CURVE_DASH_ROW AS Integer = 18

FUNC __canvas_cvAbs(v AS Float) AS Float
  IF v < 0.0 THEN
    RETURN 0.0 - v
  END IF
  RETURN v
END FUNC

' The curve `h` cut at its middle: the left half's `n` points, then the right half's.
' Each level of de Casteljau gives the left half its next point (the first of the
' level) and the right half its next from the end (the last of the level).
FUNC __canvas_cvHalves(h AS List OF Float, n AS Integer) AS List OF Float
  MUT work AS List OF Float = h
  MUT left AS List OF Float = []
  MUT back AS List OF Float = []
  MUT level AS Integer = 0
  WHILE level < n
    LET lastAt AS Integer = (n - 1 - level) * 3
    left = collections::append(left, collections::getOr(work, 0, 0.0))
    left = collections::append(left, collections::getOr(work, 1, 0.0))
    left = collections::append(left, collections::getOr(work, 2, 0.0))
    back = collections::append(back, collections::getOr(work, lastAt, 0.0))
    back = collections::append(back, collections::getOr(work, lastAt + 1, 0.0))
    back = collections::append(back, collections::getOr(work, lastAt + 2, 0.0))
    MUT i AS Integer = 0
    WHILE i < lastAt
      work = collections::set(work, i, (collections::getOr(work, i, 0.0) + collections::getOr(work, i + 3, 0.0)) / 2.0)
      i = i + 1
    END WHILE
    level = level + 1
  END WHILE
  MUT j AS Integer = n - 1
  WHILE j >= 0
    left = collections::append(left, collections::getOr(back, j * 3, 0.0))
    left = collections::append(left, collections::getOr(back, j * 3 + 1, 0.0))
    left = collections::append(left, collections::getOr(back, j * 3 + 2, 0.0))
    j = j - 1
  END WHILE
  RETURN left
END FUNC

' The curve `h` as straight chords within `tol` of it: the end of each chord, appended
' as x, y. (The curve's first point is the caller's to write.)
'
' A span is flat when every control point between its ends is within `tol` of the chord
' -- of the SEGMENT, so a span that doubles back past an end is not called flat. A curve
' with positive weights lies inside the hull of its control points, so that bounds the
' curve too. Any other span is halved, down to `__CANVAS_CURVE_MAX_DEPTH`.
FUNC __canvas_cvFlatten(out AS List OF Float, h AS List OF Float, n AS Integer, tol AS Float, depth AS Integer) AS List OF Float
  LET w0 AS Float = collections::getOr(h, 2, 1.0)
  LET x0 AS Float = collections::getOr(h, 0, 0.0) / w0
  LET y0 AS Float = collections::getOr(h, 1, 0.0) / w0
  LET endAt AS Integer = (n - 1) * 3
  LET w2 AS Float = collections::getOr(h, endAt + 2, 1.0)
  LET x2 AS Float = collections::getOr(h, endAt, 0.0) / w2
  LET y2 AS Float = collections::getOr(h, endAt + 1, 0.0) / w2
  LET dx AS Float = x2 - x0
  LET dy AS Float = y2 - y0
  LET len2 AS Float = dx * dx + dy * dy
  MUT flat AS Boolean = TRUE
  MUT j AS Integer = 1
  WHILE j < n - 1 AND flat
    LET wj AS Float = collections::getOr(h, j * 3 + 2, 1.0)
    LET qx AS Float = collections::getOr(h, j * 3, 0.0) / wj - x0
    LET qy AS Float = collections::getOr(h, j * 3 + 1, 0.0) / wj - y0
    MUT t AS Float = 0.0
    IF len2 > 0.0 THEN
      t = (qx * dx + qy * dy) / len2
    END IF
    IF t < 0.0 THEN
      t = 0.0
    END IF
    IF t > 1.0 THEN
      t = 1.0
    END IF
    LET ex AS Float = qx - t * dx
    LET ey AS Float = qy - t * dy
    IF ex * ex + ey * ey > tol * tol THEN
      flat = FALSE
    END IF
    j = j + 1
  END WHILE
  IF flat OR depth >= __CANVAS_CURVE_MAX_DEPTH THEN
    RETURN collections::append(collections::append(out, x2), y2)
  END IF
  LET halves AS List OF Float = __canvas_cvHalves(h, n)
  LET o AS List OF Float = __canvas_cvFlatten(out, collections::mid(halves, 0, n * 3), n, tol, depth + 1)
  RETURN __canvas_cvFlatten(o, collections::mid(halves, n * 3, n * 3), n, tol, depth + 1)
END FUNC

' Boehm's knot insertion. `h` is a B-spline's control points (homogeneous), `u` its knot
' vector and `p` its degree; the answer is the control points of the SAME curve once the
' knot `v` has been added, where `k` is the last index with `u[k] <= v`. There is one
' more point than before: the first `k - p + 1` are unchanged, the last ones move up a
' place, and the `p` between are each a blend of two neighbours,
'   Q[i] = (1 - a) P[i - 1] + a P[i],   a = (v - u[i]) / (u[i + p] - u[i]).
FUNC __canvas_cvKnotPoints(h AS List OF Float, u AS List OF Float, p AS Integer, k AS Integer, v AS Float) AS List OF Float
  LET n AS Integer = len(h) / 3
  MUT out AS List OF Float = []
  MUT i AS Integer = 0
  WHILE i <= n
    MUT alpha AS Float = 1.0
    IF i > k THEN
      alpha = 0.0
    ELSEIF i > k - p THEN
      LET lo AS Float = collections::getOr(u, i, 0.0)
      LET hi AS Float = collections::getOr(u, i + p, 0.0)
      alpha = 0.0
      IF hi > lo THEN
        alpha = (v - lo) / (hi - lo)
      END IF
    END IF
    MUT c AS Integer = 0
    WHILE c < 3
      ' A point taken whole is copied, not multiplied by one.
      IF alpha = 1.0 THEN
        out = collections::append(out, collections::getOr(h, i * 3 + c, 0.0))
      ELSEIF alpha = 0.0 THEN
        out = collections::append(out, collections::getOr(h, (i - 1) * 3 + c, 0.0))
      ELSE
        out = collections::append(out, (1.0 - alpha) * collections::getOr(h, (i - 1) * 3 + c, 0.0) + alpha * collections::getOr(h, i * 3 + c, 0.0))
      END IF
      c = c + 1
    END WHILE
    i = i + 1
  END WHILE
  RETURN out
END FUNC

' The knot vector `__canvas_cvRefine` left, beside the control points it returned.
MUT __CANVAS_CURVE_KNOTS AS List OF Float = []

' A B-spline with every knot of its domain raised to multiplicity `p`: the same curve,
' whose control points now fall into runs of `p + 1` that are each one Bezier segment.
' The domain is `u[p]` to `u[n]` for `n` control points -- for a clamped vector, all of
' it. Returns the control points; the knots are left in `__CANVAS_CURVE_KNOTS`.
FUNC __canvas_cvRefine(h AS List OF Float, knots AS List OF Float, p AS Integer) AS List OF Float
  MUT pts AS List OF Float = h
  MUT u AS List OF Float = knots
  LET last AS Float = collections::getOr(knots, len(h) / 3, 0.0)
  MUT v AS Float = collections::getOr(knots, p, 0.0)
  MUT going AS Boolean = TRUE
  WHILE going
    ' How often `v` is in the vector, the last index at or below it, and the next knot
    ' above it.
    MUT mult AS Integer = 0
    MUT k AS Integer = 0
    MUT above AS Float = v
    MUT found AS Boolean = FALSE
    MUT i AS Integer = 0
    WHILE i < len(u)
      LET ui AS Float = collections::getOr(u, i, 0.0)
      IF ui = v THEN
        mult = mult + 1
      END IF
      IF ui <= v THEN
        k = i
      ELSEIF NOT found THEN
        above = ui
        found = TRUE
      END IF
      i = i + 1
    END WHILE
    WHILE mult < p
      pts = __canvas_cvKnotPoints(pts, u, p, k, v)
      u = collections::append(collections::append(collections::mid(u, 0, k + 1), v), collections::mid(u, k + 1, len(u) - k - 1))
      k = k + 1
      mult = mult + 1
    END WHILE
    IF v >= last OR NOT found THEN
      going = FALSE
    ELSE
      v = above
    END IF
  END WHILE
  __CANVAS_CURVE_KNOTS = u
  RETURN pts
END FUNC"#;

/// The reduction, on homogeneous control points, and what a record is built from
/// beside it: a piece's turn, its speed, its arc-length row.
#[rustfmt::skip]
pub(super) const CURVE_REDUCE: &str =
r#"FUNC __canvas_cvPiece(out AS List OF Float, x0 AS Float, y0 AS Float, x1 AS Float, y1 AS Float, x2 AS Float, y2 AS Float, w AS Float) AS List OF Float
  MUT o AS List OF Float = out
  o = collections::append(o, x0)
  o = collections::append(o, y0)
  o = collections::append(o, x1)
  o = collections::append(o, y1)
  o = collections::append(o, x2)
  o = collections::append(o, y2)
  RETURN collections::append(o, w)
END FUNC

' A straight segment as a piece: the control point is the midpoint, the weight 1.
FUNC __canvas_cvLine(out AS List OF Float, x0 AS Float, y0 AS Float, x2 AS Float, y2 AS Float) AS List OF Float
  RETURN __canvas_cvPiece(out, x0, y0, (x0 + x2) / 2.0, (y0 + y2) / 2.0, x2, y2, 1.0)
END FUNC

' One rational quadratic (end weights 1, middle weight `w`), appended as the pieces it
' is y-monotone on.
'
' `y(t)` turns where `Y'W - YW'` is zero. For a quadratic over a quadratic the cubic
' terms cancel, and with `y` measured from `y0` what is left is
'   N(t) = (w - 1)(y2 - y0) t^2 + ((y2 - y0) - 2w(y1 - y0)) t + w(y1 - y0)
' with N(0) = w(y1 - y0) and N(1) = w(y2 - y1). A curve with positive weights never
' turns more often than its control polygon, so it turns -- once -- exactly when those
' two have opposite signs, and that sign change is what the bisection below closes on.
' 64 halvings is every bit a Float has; bisection rather than the quadratic formula
' because it cannot lose the root to cancellation.
'
' The two halves are cut by de Casteljau in homogeneous coordinates and their weights
' normalised again. The tangent at the turn is horizontal, so each half's control point
' is given the turn's own `y`: the half is then monotone by its control polygon, not by
' the last bit of a division.
FUNC __canvas_cvMonotone(out AS List OF Float, x0 AS Float, y0 AS Float, x1 AS Float, y1 AS Float, x2 AS Float, y2 AS Float, w AS Float) AS List OF Float
  LET d1 AS Float = y1 - y0
  LET d2 AS Float = y2 - y1
  IF NOT ((d1 > 0.0 AND d2 < 0.0) OR (d1 < 0.0 AND d2 > 0.0)) THEN
    RETURN __canvas_cvPiece(out, x0, y0, x1, y1, x2, y2, w)
  END IF
  LET span AS Float = y2 - y0
  LET na AS Float = (w - 1.0) * span
  LET nb AS Float = span - 2.0 * w * d1
  LET nc AS Float = w * d1
  MUT lo AS Float = 0.0
  MUT hi AS Float = 1.0
  MUT k AS Integer = 0
  WHILE k < 64
    LET mid AS Float = (lo + hi) / 2.0
    LET nm AS Float = (na * mid + nb) * mid + nc
    IF (nm > 0.0) = (nc > 0.0) THEN
      lo = mid
    ELSE
      hi = mid
    END IF
    k = k + 1
  END WHILE
  LET t AS Float = (lo + hi) / 2.0
  LET s AS Float = 1.0 - t
  ' H0 = (x0, y0, 1), H1 = (w x1, w y1, w), H2 = (x2, y2, 1).
  LET ax AS Float = s * x0 + t * w * x1
  LET ay AS Float = s * y0 + t * w * y1
  LET aw AS Float = s + t * w
  LET bx AS Float = s * w * x1 + t * x2
  LET by AS Float = s * w * y1 + t * y2
  LET bw AS Float = s * w + t
  LET mw AS Float = s * aw + t * bw
  LET mx AS Float = (s * ax + t * bx) / mw
  LET my AS Float = (s * ay + t * by) / mw
  LET root AS Float = math::sqrt(mw)
  MUT o AS List OF Float = __canvas_cvPiece(out, x0, y0, ax / aw, my, mx, my, aw / root)
  RETURN __canvas_cvPiece(o, mx, my, bx / bw, my, x2, y2, bw / root)
END FUNC

' The point of the curve `h` (`n` control points) at `t`, as [x, y]: de Casteljau on
' the homogeneous points, then the divide.
FUNC __canvas_cvPoint(h AS List OF Float, n AS Integer, t AS Float) AS List OF Float
  MUT work AS List OF Float = h
  LET s AS Float = 1.0 - t
  MUT level AS Integer = 1
  WHILE level < n
    MUT i AS Integer = 0
    WHILE i < (n - level) * 3
      work = collections::set(work, i, s * collections::getOr(work, i, 0.0) + t * collections::getOr(work, i + 3, 0.0))
      i = i + 1
    END WHILE
    level = level + 1
  END WHILE
  LET w AS Float = collections::getOr(work, 2, 1.0)
  RETURN [collections::getOr(work, 0, 0.0) / w, collections::getOr(work, 1, 0.0) / w]
END FUNC

' How far (px, py) is from the polynomial quadratic P0, P1, P2. Only the reduction asks,
' at build time, so it is a search and not the per-pixel solve: the nearest of nine
' evenly spaced points, then Newton's method on the derivative of the squared distance
' from there, kept inside the piece. The ends are always candidates.
FUNC __canvas_cvQuadDistance(px AS Float, py AS Float, x0 AS Float, y0 AS Float, x1 AS Float, y1 AS Float, x2 AS Float, y2 AS Float) AS Float
  LET ax AS Float = x1 - x0
  LET ay AS Float = y1 - y0
  LET bx AS Float = x0 - 2.0 * x1 + x2
  LET by AS Float = y0 - 2.0 * y1 + y2
  LET mx AS Float = x0 - px
  LET my AS Float = y0 - py
  MUT best AS Float = mx * mx + my * my
  MUT bestT AS Float = 0.0
  MUT k AS Integer = 1
  WHILE k <= 8
    LET tk AS Float = toFloat(k) / 8.0
    LET qx AS Float = mx + (2.0 * ax + bx * tk) * tk
    LET qy AS Float = my + (2.0 * ay + by * tk) * tk
    LET dk AS Float = qx * qx + qy * qy
    IF dk < best THEN
      best = dk
      bestT = tk
    END IF
    k = k + 1
  END WHILE
  ' D'(t) / 4 = (b.b) t^3 + 3 (a.b) t^2 + (2 a.a + m.b) t + m.a
  LET c3 AS Float = bx * bx + by * by
  LET c2 AS Float = 3.0 * (ax * bx + ay * by)
  LET c1 AS Float = 2.0 * (ax * ax + ay * ay) + mx * bx + my * by
  LET c0 AS Float = mx * ax + my * ay
  MUT t AS Float = bestT
  k = 0
  WHILE k < 8
    LET f AS Float = ((c3 * t + c2) * t + c1) * t + c0
    LET slope AS Float = (3.0 * c3 * t + 2.0 * c2) * t + c1
    IF slope > 0.0 THEN
      t = t - f / slope
    END IF
    IF t < 0.0 THEN
      t = 0.0
    END IF
    IF t > 1.0 THEN
      t = 1.0
    END IF
    k = k + 1
  END WHILE
  LET rx AS Float = mx + (2.0 * ax + bx * t) * t
  LET ry AS Float = my + (2.0 * ay + by * t) * t
  LET dt AS Float = rx * rx + ry * ry
  IF dt < best THEN
    best = dt
  END IF
  RETURN math::sqrt(best)
END FUNC

' The furthest any of the span's samples is from the polynomial quadratic P0, C, P2.
FUNC __canvas_cvFitError(h AS List OF Float, n AS Integer, x0 AS Float, y0 AS Float, cx AS Float, cy AS Float, x2 AS Float, y2 AS Float) AS Float
  MUT worst AS Float = 0.0
  MUT i AS Integer = 0
  WHILE i < __CANVAS_CURVE_FIT_SAMPLES
    LET at AS List OF Float = __canvas_cvPoint(h, n, (toFloat(i) + 0.5) / toFloat(__CANVAS_CURVE_FIT_SAMPLES))
    LET d AS Float = __canvas_cvQuadDistance(collections::getOr(at, 0, 0.0), collections::getOr(at, 1, 0.0), x0, y0, cx, cy, x2, y2)
    IF d > worst THEN
      worst = d
    END IF
    i = i + 1
  END WHILE
  RETURN worst
END FUNC

' One span of a curve of degree 3 or more, appended as the pieces that fit it.
'
' The fit is the polynomial quadratic through the span's two ends whose control point is
' where the two end tangents meet -- so a fitted piece leaves each end in the curve's own
' direction, and two neighbouring pieces meet without a corner. The tangent at an end is
' towards the nearest control point that is not the end itself. Where the tangents are
' parallel, or meet behind an end (the span still holds an inflection), the control
' point is the middle of the chord: a straight piece, which the error test then judges
' like any other.
'
' A span whose samples are all within `eps` of its fit is taken. Any other is halved,
' down to `__CANVAS_CURVE_MAX_DEPTH`; there the better of the fit and the chord is taken
' as it is.
FUNC __canvas_cvSpan(out AS List OF Float, h AS List OF Float, n AS Integer, eps AS Float, depth AS Integer) AS List OF Float
  LET w0 AS Float = collections::getOr(h, 2, 1.0)
  LET x0 AS Float = collections::getOr(h, 0, 0.0) / w0
  LET y0 AS Float = collections::getOr(h, 1, 0.0) / w0
  LET endAt AS Integer = (n - 1) * 3
  LET w2 AS Float = collections::getOr(h, endAt + 2, 1.0)
  LET x2 AS Float = collections::getOr(h, endAt, 0.0) / w2
  LET y2 AS Float = collections::getOr(h, endAt + 1, 0.0) / w2
  MUT ax AS Float = 0.0
  MUT ay AS Float = 0.0
  MUT found AS Boolean = FALSE
  MUT j AS Integer = 1
  WHILE j < n AND NOT found
    LET wj AS Float = collections::getOr(h, j * 3 + 2, 1.0)
    LET qx AS Float = collections::getOr(h, j * 3, 0.0) / wj
    LET qy AS Float = collections::getOr(h, j * 3 + 1, 0.0) / wj
    IF qx <> x0 OR qy <> y0 THEN
      ax = qx - x0
      ay = qy - y0
      found = TRUE
    END IF
    j = j + 1
  END WHILE
  MUT bx AS Float = 0.0
  MUT by AS Float = 0.0
  found = FALSE
  j = n - 2
  WHILE j >= 0 AND NOT found
    LET wj AS Float = collections::getOr(h, j * 3 + 2, 1.0)
    LET qx AS Float = collections::getOr(h, j * 3, 0.0) / wj
    LET qy AS Float = collections::getOr(h, j * 3 + 1, 0.0) / wj
    IF qx <> x2 OR qy <> y2 THEN
      bx = qx - x2
      by = qy - y2
      found = TRUE
    END IF
    j = j - 1
  END WHILE
  LET midX AS Float = (x0 + x2) / 2.0
  LET midY AS Float = (y0 + y2) / 2.0
  MUT cx AS Float = midX
  MUT cy AS Float = midY
  LET dx AS Float = x2 - x0
  LET dy AS Float = y2 - y0
  LET cross AS Float = ax * by - ay * bx
  LET scale AS Float = math::sqrt((ax * ax + ay * ay) * (bx * bx + by * by))
  IF __canvas_cvAbs(cross) > scale * 0.000000000001 THEN
    LET s AS Float = (dx * by - dy * bx) / cross
    LET u AS Float = (dx * ay - dy * ax) / cross
    IF s > 0.0 AND u > 0.0 THEN
      cx = x0 + s * ax
      cy = y0 + s * ay
      ' An end the curve leaves level puts the control point at that end's own `y`,
      ' to the bit: `y0 + s * ay` is that already when `ay` is zero, and is only near
      ' `y2` when `by` is. A control point a last place off would read as a turn, and
      ' be cut into a piece too short to see.
      IF by = 0.0 THEN
        cy = y2
      END IF
    END IF
  END IF
  LET err AS Float = __canvas_cvFitError(h, n, x0, y0, cx, cy, x2, y2)
  IF err <= eps THEN
    RETURN __canvas_cvMonotone(out, x0, y0, cx, cy, x2, y2, 1.0)
  END IF
  IF depth >= __CANVAS_CURVE_MAX_DEPTH THEN
    IF __canvas_cvFitError(h, n, x0, y0, midX, midY, x2, y2) < err THEN
      RETURN __canvas_cvLine(out, x0, y0, x2, y2)
    END IF
    RETURN __canvas_cvMonotone(out, x0, y0, cx, cy, x2, y2, 1.0)
  END IF
  LET halves AS List OF Float = __canvas_cvHalves(h, n)
  LET o AS List OF Float = __canvas_cvSpan(out, collections::mid(halves, 0, n * 3), n, eps, depth + 1)
  RETURN __canvas_cvSpan(o, collections::mid(halves, n * 3, n * 3), n, eps, depth + 1)
END FUNC

' A whole curve as its pieces, in order from its first point to its last. `h` is its
' control points in homogeneous form; fewer than two is no curve and gives no piece.
FUNC __canvas_cvReduce(h AS List OF Float, eps AS Float) AS List OF Float
  LET n AS Integer = len(h) / 3
  LET none AS List OF Float = []
  IF n < 2 THEN
    RETURN none
  END IF
  LET w0 AS Float = collections::getOr(h, 2, 1.0)
  LET x0 AS Float = collections::getOr(h, 0, 0.0) / w0
  LET y0 AS Float = collections::getOr(h, 1, 0.0) / w0
  LET endAt AS Integer = (n - 1) * 3
  LET w2 AS Float = collections::getOr(h, endAt + 2, 1.0)
  LET x2 AS Float = collections::getOr(h, endAt, 0.0) / w2
  LET y2 AS Float = collections::getOr(h, endAt + 1, 0.0) / w2
  IF n = 2 THEN
    RETURN __canvas_cvLine(none, x0, y0, x2, y2)
  END IF
  IF n = 3 THEN
    LET w1 AS Float = collections::getOr(h, 5, 1.0)
    RETURN __canvas_cvMonotone(none, x0, y0, collections::getOr(h, 3, 0.0) / w1, collections::getOr(h, 4, 0.0) / w1, x2, y2, w1 / (math::sqrt(w0) * math::sqrt(w2)))
  END IF
  RETURN __canvas_cvSpan(none, h, n, eps, 0)
END FUNC

' A curve's pieces with its two ends put back where they were written. The ends of a
' rational curve come out of the reduction as `(w x) / w`, which can be one place off
' `x`; a contour's fill counts a crossing by comparing `y` against exactly these ends,
' and two curves that meet must meet in the same number.
FUNC __canvas_cvPin(pieces AS List OF Float, x0 AS Float, y0 AS Float, x2 AS Float, y2 AS Float) AS List OF Float
  LET count AS Integer = len(pieces)
  IF count < __CANVAS_CURVE_PIECE THEN
    RETURN pieces
  END IF
  MUT out AS List OF Float = pieces
  out = collections::set(out, 0, x0)
  out = collections::set(out, 1, y0)
  out = collections::set(out, count - 3, x2)
  RETURN collections::set(out, count - 2, y2)
END FUNC

' The furthest a piece reaches in ONE coordinate, between its ends: `a0`, `a1`, `a2` are
' that coordinate of its three points and `w` its weight. Asked only of a piece that
' turns in that coordinate -- `(a1 - a0)` and `(a2 - a1)` of opposite signs -- and found
' as `__canvas_cvMonotone` finds a turn in y: the same `N(t)`, the same 64 halvings.
FUNC __canvas_cvTurn(a0 AS Float, a1 AS Float, a2 AS Float, w AS Float) AS Float
  LET d1 AS Float = a1 - a0
  LET span AS Float = a2 - a0
  LET na AS Float = (w - 1.0) * span
  LET nb AS Float = span - 2.0 * w * d1
  LET nc AS Float = w * d1
  MUT lo AS Float = 0.0
  MUT hi AS Float = 1.0
  MUT k AS Integer = 0
  WHILE k < 64
    LET mid AS Float = (lo + hi) / 2.0
    LET nm AS Float = (na * mid + nb) * mid + nc
    IF (nm > 0.0) = (nc > 0.0) THEN
      lo = mid
    ELSE
      hi = mid
    END IF
    k = k + 1
  END WHILE
  LET t AS Float = (lo + hi) / 2.0
  LET s AS Float = 1.0 - t
  LET k1 AS Float = 2.0 * w * s * t
  RETURN (s * s * a0 + k1 * a1 + t * t * a2) / (s * s + k1 + t * t)
END FUNC

' The pieces of `pieces` that have length: one whose three points are the same point
' draws nothing and has no direction, so it is left out of a record.
FUNC __canvas_cvLive(pieces AS List OF Float) AS List OF Float
  MUT out AS List OF Float = []
  MUT i AS Integer = 0
  WHILE i + 6 < len(pieces)
    LET x0 AS Float = collections::getOr(pieces, i, 0.0)
    LET y0 AS Float = collections::getOr(pieces, i + 1, 0.0)
    LET same AS Boolean = collections::getOr(pieces, i + 2, 0.0) = x0 AND collections::getOr(pieces, i + 3, 0.0) = y0 AND collections::getOr(pieces, i + 4, 0.0) = x0 AND collections::getOr(pieces, i + 5, 0.0) = y0
    IF NOT same THEN
      out = collections::append(out, collections::mid(pieces, i, 7))
    END IF
    i = i + 7
  END WHILE
  RETURN out
END FUNC

' The speed of a piece at `t`: the length of the derivative of `N(t) / W(t)`.
FUNC __canvas_cvSpeed(x0 AS Float, y0 AS Float, x1 AS Float, y1 AS Float, x2 AS Float, y2 AS Float, w AS Float, t AS Float) AS Float
  LET u AS Float = 1.0 - t
  LET k1 AS Float = 2.0 * w * u * t
  LET den AS Float = u * u + k1 + t * t
  LET nx AS Float = u * u * x0 + k1 * x1 + t * t * x2
  LET ny AS Float = u * u * y0 + k1 * y1 + t * t * y2
  LET m AS Float = w * (1.0 - 2.0 * t)
  LET dden AS Float = 2.0 * (m + t - u)
  LET dnx AS Float = 2.0 * (m * x1 + t * x2 - u * x0)
  LET dny AS Float = 2.0 * (m * y1 + t * y2 - u * y0)
  LET vx AS Float = (dnx * den - nx * dden) / (den * den)
  LET vy AS Float = (dny * den - ny * dden) / (den * den)
  RETURN math::sqrt(vx * vx + vy * vy)
END FUNC

' A run of pieces' arc-length rows, one a piece: the outline position at each sixteenth
' of the piece, from the position its contour has reached before it, then the contour's
' whole length -- negative for an open chain. Each sixteenth is Simpson's rule over 8
' sub-steps of the piece's speed, in the Float the build runs in. `starts` is the piece
' each contour begins at.
FUNC __canvas_curveDashData(pieces AS List OF Float, starts AS List OF Integer, closed AS Boolean) AS List OF Float
  MUT out AS List OF Float = []
  LET total AS Integer = len(pieces) / __CANVAS_CURVE_PIECE
  LET rings AS Integer = len(starts)
  LET steps AS Integer = __CANVAS_CURVE_ARC_STEPS
  LET dt AS Float = 1.0 / toFloat(steps)
  MUT r AS Integer = 0
  WHILE r < rings
    LET first AS Integer = collections::getOr(starts, r, 0)
    MUT stop AS Integer = total
    IF r + 1 < rings THEN
      stop = collections::getOr(starts, r + 1, total)
    END IF
    MUT rows AS List OF Float = []
    MUT at AS Float = 0.0
    MUT i AS Integer = first
    WHILE i < stop
      LET b AS Integer = i * __CANVAS_CURVE_PIECE
      LET x0 AS Float = collections::getOr(pieces, b, 0.0)
      LET y0 AS Float = collections::getOr(pieces, b + 1, 0.0)
      LET x1 AS Float = collections::getOr(pieces, b + 2, 0.0)
      LET y1 AS Float = collections::getOr(pieces, b + 3, 0.0)
      LET x2 AS Float = collections::getOr(pieces, b + 4, 0.0)
      LET y2 AS Float = collections::getOr(pieces, b + 5, 0.0)
      LET w AS Float = collections::getOr(pieces, b + 6, 1.0)
      rows = collections::append(rows, at)
      MUT k AS Integer = 0
      WHILE k < steps
        LET t0 AS Float = toFloat(k) * dt
        MUT sum AS Float = 0.0
        MUT j AS Integer = 0
        WHILE j <= 8
          MUT weight AS Float = 2.0
          IF j = 0 OR j = 8 THEN
            weight = 1.0
          ELSEIF j MOD 2 = 1 THEN
            weight = 4.0
          END IF
          sum = sum + weight * __canvas_cvSpeed(x0, y0, x1, y1, x2, y2, w, t0 + toFloat(j) * dt / 8.0)
          j = j + 1
        END WHILE
        at = at + sum * dt / 24.0
        rows = collections::append(rows, at)
        k = k + 1
      END WHILE
      i = i + 1
    END WHILE
    MUT length AS Float = at
    IF NOT closed THEN
      length = 0.0 - at
    END IF
    i = 0
    WHILE i < stop - first
      out = collections::append(out, collections::mid(rows, i * (steps + 1), steps + 1))
      out = collections::append(out, length)
      i = i + 1
    END WHILE
    r = r + 1
  END WHILE
  RETURN out
END FUNC"#;

/// What a `BezierCurve` is valid as, and its control points in homogeneous form. Rides
/// with [`CURVE_BASE`]: the shapes and `canvas::flatten` both ask.
#[rustfmt::skip]
pub(super) const CURVE_VALID: &str =
r#"' plan-164-A: whether a curve can be drawn. 2 to 17 points; weights either absent or one
' per point, each above zero. Any other curve is ABSENT: it gives no piece, and its
' neighbours are joined across it. (Nothing here asks whether a number is finite: a
' `Float` cannot hold an infinity or a NaN -- the arithmetic that would make one raises.)
FUNC __canvas_curveValid(curve AS BezierCurve) AS Boolean
  LET n AS Integer = len(curve.points)
  IF n < 2 OR n > 17 THEN
    RETURN FALSE
  END IF
  LET wn AS Integer = len(curve.weights)
  IF wn <> 0 AND wn <> n THEN
    RETURN FALSE
  END IF
  FOR EACH w IN curve.weights
    IF NOT (w > 0.0) THEN
      RETURN FALSE
    END IF
  NEXT
  RETURN TRUE
END FUNC

' A valid curve's control points in homogeneous form, moved by (ox, oy).
FUNC __canvas_curveHomog(curve AS BezierCurve, ox AS Float, oy AS Float) AS List OF Float
  MUT h AS List OF Float = []
  LET weighted AS Boolean = len(curve.weights) > 0
  MUT i AS Integer = 0
  FOR EACH q IN curve.points
    MUT w AS Float = 1.0
    IF weighted THEN
      w = collections::getOr(curve.weights, i, 1.0)
    END IF
    h = collections::append(h, w * (q.x + ox))
    h = collections::append(h, w * (q.y + oy))
    h = collections::append(h, w)
    i = i + 1
  NEXT
  RETURN h
END FUNC"#;

/// The typed half of the reduction: the tolerance a shape's transform asks for, and
/// the walk along a chain of curves that adds the implied segments.
///
/// `__CANVAS_CURVE_MARKS` is the piece index each **input segment** of the chain begins
/// at — a valid curve, an implied join, the closing segment — in order. A corner can
/// only be at one of those; the reduction's own cuts are smooth.
#[rustfmt::skip]
pub(super) const CURVE_CHAIN: &str =
r#"' The reduction's tolerance, in surface pixels.
LET __CANVAS_CURVE_EPS AS Float = 0.05

' What the last chain walk left beside the pieces it returned.
'
' `__CANVAS_CURVE_MARKS` is the piece each INPUT segment begins at -- a valid curve, an
' implied join, the closing segment -- in order, and `__CANVAS_CURVE_TANGENTS` four
' floats for each of them: the direction it leaves its first point in, then the
' direction it arrives at its last point in (neither is a unit vector). A corner can
' only be at a mark; the reduction's own cuts are smooth. A segment with no length
' leaves no piece and no mark.
'
' `__CANVAS_CURVE_HAVE` is whether the chain had a valid curve at all, and
' `__CANVAS_CURVE_FIRSTX` / `_FIRSTY` where the first one began: a chain of curves that
' are each a single point has no piece, and is still somewhere.
MUT __CANVAS_CURVE_MARKS AS List OF Integer = []

MUT __CANVAS_CURVE_TANGENTS AS List OF Float = []

MUT __CANVAS_CURVE_HAVE AS Boolean = FALSE

MUT __CANVAS_CURVE_FIRSTX AS Float = 0.0

MUT __CANVAS_CURVE_FIRSTY AS Float = 0.0

' The direction a valid curve leaves its first point in, or (`atEnd`) arrives at its
' last in, as [x, y]: towards, or from, the nearest control point that is not that end
' itself. A curve's weights do not turn its end tangents. [0, 0] for a curve whose
' points are all one point.
FUNC __canvas_curveTangent(curve AS BezierCurve, atEnd AS Boolean) AS List OF Float
  LET origin AS vector::Float2 = vector::Float2[x := 0.0, y := 0.0]
  LET n AS Integer = len(curve.points)
  MUT at AS Integer = 0
  MUT dir AS Integer = 1
  IF atEnd THEN
    at = n - 1
    dir = 0 - 1
  END IF
  LET e AS vector::Float2 = collections::getOr(curve.points, at, origin)
  MUT j AS Integer = at + dir
  WHILE j >= 0 AND j < n
    LET q AS vector::Float2 = collections::getOr(curve.points, j, origin)
    IF q.x <> e.x OR q.y <> e.y THEN
      IF atEnd THEN
        RETURN [e.x - q.x, e.y - q.y]
      END IF
      RETURN [q.x - e.x, q.y - e.y]
    END IF
    j = j + dir
  END WHILE
  RETURN [0.0, 0.0]
END FUNC

' One curve as its y-monotone pieces, within `eps` of it; none for an invalid curve.
FUNC __canvas_reduceCurve(curve AS BezierCurve, eps AS Float) AS List OF Float
  IF NOT __canvas_curveValid(curve) THEN
    RETURN []
  END IF
  LET origin AS vector::Float2 = vector::Float2[x := 0.0, y := 0.0]
  LET head AS vector::Float2 = collections::getOr(curve.points, 0, origin)
  LET tail AS vector::Float2 = collections::getOr(curve.points, len(curve.points) - 1, origin)
  RETURN __canvas_cvPin(__canvas_cvReduce(__canvas_curveHomog(curve, 0.0, 0.0), eps), head.x, head.y, tail.x, tail.y)
END FUNC

' The tolerance for a shape drawn through `t`, in the shape's own space: 0.05 surface
' pixels divided by the most `t` stretches anything -- the larger singular value of its
' linear part. A transform not in effect stretches nothing. A group's transform is not
' in here: a group's records are built once and shared by every use of it.
FUNC __canvas_curveEps(t AS Transform) AS Float
  IF NOT __canvas_transformActive(t) THEN
    RETURN __CANVAS_CURVE_EPS
  END IF
  LET sum AS Float = t.a * t.a + t.b * t.b + t.c * t.c + t.d * t.d
  LET det AS Float = t.a * t.d - t.b * t.c
  MUT disc AS Float = sum * sum - 4.0 * det * det
  IF disc < 0.0 THEN
    disc = 0.0
  END IF
  RETURN __CANVAS_CURVE_EPS / math::sqrt((sum + math::sqrt(disc)) / 2.0)
END FUNC

' One input segment's pieces appended to a chain's, with its mark and its two
' tangents; a segment with no length -- no piece that has any -- adds nothing.
FUNC __canvas_curveSegment(out AS List OF Float, pieces AS List OF Float, base AS Integer, sx AS Float, sy AS Float, ex AS Float, ey AS Float) AS List OF Float
  LET live AS List OF Float = __canvas_cvLive(pieces)
  IF len(live) = 0 THEN
    RETURN out
  END IF
  __CANVAS_CURVE_MARKS = collections::append(__CANVAS_CURVE_MARKS, base + len(out) / __CANVAS_CURVE_PIECE)
  __CANVAS_CURVE_TANGENTS = collections::append(__CANVAS_CURVE_TANGENTS, [sx, sy, ex, ey])
  RETURN collections::append(out, live)
END FUNC

' A chain of curves as one run of pieces, moved by (ox, oy).
'
' The curves are joined in list order. Where a valid curve does not begin at the point
' the one before it ended on, a straight piece joins the two; and a `closed` chain whose
' last point is not its first is closed by a straight piece. Those pieces are part of
' the shape. A chain with no valid curve gives no piece.
'
' `base` is the number of pieces already ahead of this chain in the record, so that the
' indices appended to `__CANVAS_CURVE_MARKS` are the record's.
FUNC __canvas_curveChain(curves AS List OF BezierCurve, closed AS Boolean, eps AS Float, ox AS Float, oy AS Float, base AS Integer) AS List OF Float
  MUT out AS List OF Float = []
  LET origin AS vector::Float2 = vector::Float2[x := 0.0, y := 0.0]
  LET none AS List OF Float = []
  MUT have AS Boolean = FALSE
  MUT firstX AS Float = 0.0
  MUT firstY AS Float = 0.0
  MUT lastX AS Float = 0.0
  MUT lastY AS Float = 0.0
  FOR EACH curve IN curves
    IF __canvas_curveValid(curve) THEN
      LET head AS vector::Float2 = collections::getOr(curve.points, 0, origin)
      LET tail AS vector::Float2 = collections::getOr(curve.points, len(curve.points) - 1, origin)
      LET hx AS Float = head.x + ox
      LET hy AS Float = head.y + oy
      IF have THEN
        IF hx <> lastX OR hy <> lastY THEN
          out = __canvas_curveSegment(out, __canvas_cvLine(none, lastX, lastY, hx, hy), base, hx - lastX, hy - lastY, hx - lastX, hy - lastY)
        END IF
      ELSE
        firstX = hx
        firstY = hy
        have = TRUE
      END IF
      lastX = tail.x + ox
      lastY = tail.y + oy
      LET leaves AS List OF Float = __canvas_curveTangent(curve, FALSE)
      LET arrives AS List OF Float = __canvas_curveTangent(curve, TRUE)
      out = __canvas_curveSegment(out, __canvas_cvPin(__canvas_cvReduce(__canvas_curveHomog(curve, ox, oy), eps), hx, hy, lastX, lastY), base, collections::getOr(leaves, 0, 0.0), collections::getOr(leaves, 1, 0.0), collections::getOr(arrives, 0, 0.0), collections::getOr(arrives, 1, 0.0))
    END IF
  NEXT
  IF closed AND have THEN
    IF lastX <> firstX OR lastY <> firstY THEN
      out = __canvas_curveSegment(out, __canvas_cvLine(none, lastX, lastY, firstX, firstY), base, firstX - lastX, firstY - lastY, firstX - lastX, firstY - lastY)
    END IF
  END IF
  __CANVAS_CURVE_HAVE = have
  __CANVAS_CURVE_FIRSTX = firstX
  __CANVAS_CURVE_FIRSTY = firstY
  RETURN out
END FUNC"#;

/// The record: a curve shape's header and tail (plan-164-B).
///
/// The tail is **pieces | join records | dash tail | gradient stops** — a polygon's order
/// with pieces for edges. The pieces are found from the front (header slot 20 counts
/// them); everything after is found from the END of the record by the functions that
/// find it for every kind (`.ai/canvas-threading.md`, "A geometry record's tail is read
/// back from its END"), which is why a curve record needs no reader of its own for its
/// joins, dashes, clip or stops.
///
/// There is one join record a piece and one dash row a piece, as a polygon has one of
/// each an edge: the stroke needs no table of where a contour starts, because each
/// row carries its contour's length and each record its own corner.
#[rustfmt::skip]
pub(super) const CURVE_BUILD: &str =
r#"' The disc record at (vx, vy): what a join record is where the outline is smooth -- no
' corner to shape, only the overlap two square-cut bands need where they meet.
FUNC __canvas_curveDisc(out AS List OF Float, vx AS Float, vy AS Float, half AS Float) AS List OF Float
  RETURN collections::append(out, [vx, vy, vx + half, vy, vx + half, vy, vx + half, vy])
END FUNC

' A curve shape's join records, one a piece, the corner at the piece's START
' (`helper_joins.rs`); nothing for an undashed Round paint or a stroke that draws
' nothing.
'
' A corner can only be where one INPUT segment ends and the next begins -- a curve, an
' implied join, the closing segment (`__CANVAS_CURVE_MARKS`) -- and is one only when
' the direction the first arrives in and the direction the second leaves in differ by
' more than a millionth of a radian. Those take the paint's join, built from the input
' curves' own tangents (`__CANVAS_CURVE_TANGENTS`). Every other piece start is smooth
' and takes a disc: the cuts the reduction made inside one curve are never mitered. An
' open chain's first piece has nothing before it and takes no record; a closed
' contour's first piece joins its last.
FUNC __canvas_curveJoins(pieces AS List OF Float, starts AS List OF Integer, closed AS Boolean, half AS Float, paint AS Paint) AS List OF Float
  MUT out AS List OF Float = []
  LET tag AS Integer = __canvas_joinTag(paint)
  IF half <= 0.0 THEN
    RETURN out
  END IF
  IF tag = 0 AND len(__canvas_dashPattern(paint, half)) = 0 THEN
    RETURN out
  END IF
  LET round AS Boolean = tag = 0
  LET miter AS Boolean = tag = 1
  LET limit AS Float = __canvas_maxF(paint.miterLimit, 1.0)
  LET total AS Integer = len(pieces) / __CANVAS_CURVE_PIECE
  LET rings AS Integer = len(starts)
  LET marks AS List OF Integer = __CANVAS_CURVE_MARKS
  LET tangents AS List OF Float = __CANVAS_CURVE_TANGENTS
  MUT m AS Integer = 0
  MUT r AS Integer = 0
  WHILE r < rings
    LET s AS Integer = collections::getOr(starts, r, 0)
    MUT e AS Integer = total
    IF r + 1 < rings THEN
      e = collections::getOr(starts, r + 1, total)
    END IF
    ' This contour's marks are m up to, not including, markEnd.
    LET markFirst AS Integer = m
    MUT markEnd AS Integer = m
    WHILE markEnd < len(marks) AND collections::getOr(marks, markEnd, total) < e
      markEnd = markEnd + 1
    END WHILE
    MUT i AS Integer = s
    WHILE i < e
      LET vx AS Float = collections::getOr(pieces, i * __CANVAS_CURVE_PIECE, 0.0)
      LET vy AS Float = collections::getOr(pieces, i * __CANVAS_CURVE_PIECE + 1, 0.0)
      IF m < markEnd AND collections::getOr(marks, m, 0 - 1) = i THEN
        MUT prev AS Integer = m - 1
        IF m = markFirst THEN
          prev = 0 - 1
          IF closed THEN
            prev = markEnd - 1
          END IF
        END IF
        IF prev < 0 THEN
          out = __canvas_noJoin(out, vx, vy)
        ELSE
          LET ax AS Float = collections::getOr(tangents, prev * 4 + 2, 0.0)
          LET ay AS Float = collections::getOr(tangents, prev * 4 + 3, 0.0)
          LET bx AS Float = collections::getOr(tangents, m * 4, 0.0)
          LET by AS Float = collections::getOr(tangents, m * 4 + 1, 0.0)
          LET la AS Float = ax * ax + ay * ay
          LET lb AS Float = bx * bx + by * by
          LET cross AS Float = ax * by - ay * bx
          IF la > 0.0 AND lb > 0.0 AND (cross * cross > la * lb * 0.000000000001 OR ax * bx + ay * by < 0.0) THEN
            out = __canvas_joinRecord(out, vx, vy, ax, ay, bx, by, half, round, miter, limit)
          ELSE
            out = __canvas_curveDisc(out, vx, vy, half)
          END IF
        END IF
        m = m + 1
      ELSE
        out = __canvas_curveDisc(out, vx, vy, half)
      END IF
      i = i + 1
    END WHILE
    m = markEnd
    r = r + 1
  END WHILE
  RETURN out
END FUNC

' plan-164-B: the box `__canvas_curveRings` last measured: [minX, minY, maxX, maxY], or
' [] for an outline with no piece.
MUT __CANVAS_CURVE_EXTENT AS List OF Float = []

FUNC __canvas_curveExtent() AS List OF Float
  RETURN __CANVAS_CURVE_EXTENT
END FUNC

' A `CurvePolygon`'s outline as rings of straight edges, one ring a contour: what a
' `Picture` cut to it clips its image to (`helper_picture.rs`). An image record's clip
' is edges, so the outline is flattened for it -- the pieces the shape is DRAWN from,
' each to within the tolerance they were reduced at -- as a circle's and an ellipse's
' are. The outline stroked over the image is the curve itself.
'
' The box the picture's image is fitted to is left in `__CANVAS_CURVE_EXTENT`, and it
' is the curve's true extent, not its control points': a piece is y-monotone, so its
' ends are as far as it reaches in y, and in x it reaches as far as its ends or its one
' turn (`__canvas_cvTurn`). A control point pulls a curve towards it without the curve
' arriving, and an image placed by the control points' box would stop short of the
' outline it is cut to.
FUNC __canvas_curveRings(cp AS CurvePolygon) AS List OF List OF vector::Float2
  LET ox AS Float = __canvas_buildX(cp.position, cp.transform)
  LET oy AS Float = __canvas_buildY(cp.position, cp.transform)
  LET eps AS Float = __canvas_curveEps(cp.transform)
  MUT rings AS List OF List OF vector::Float2 = []
  MUT any AS Boolean = FALSE
  MUT minX AS Float = 0.0
  MUT minY AS Float = 0.0
  MUT maxX AS Float = 0.0
  MUT maxY AS Float = 0.0
  FOR EACH contour IN cp.contours
    __CANVAS_CURVE_MARKS = []
    __CANVAS_CURVE_TANGENTS = []
    LET pieces AS List OF Float = __canvas_curveChain(contour, TRUE, eps, ox, oy, 0)
    LET count AS Integer = len(pieces) / __CANVAS_CURVE_PIECE
    IF count > 0 THEN
      MUT flat AS List OF Float = [collections::getOr(pieces, 0, 0.0), collections::getOr(pieces, 1, 0.0)]
      MUT i AS Integer = 0
      WHILE i < count
        LET b AS Integer = i * __CANVAS_CURVE_PIECE
        LET x0 AS Float = collections::getOr(pieces, b, 0.0)
        LET y0 AS Float = collections::getOr(pieces, b + 1, 0.0)
        LET x1 AS Float = collections::getOr(pieces, b + 2, 0.0)
        LET y1 AS Float = collections::getOr(pieces, b + 3, 0.0)
        LET x2 AS Float = collections::getOr(pieces, b + 4, 0.0)
        LET y2 AS Float = collections::getOr(pieces, b + 5, 0.0)
        LET w AS Float = collections::getOr(pieces, b + 6, 1.0)
        flat = __canvas_cvFlatten(flat, [x0, y0, 1.0, w * x1, w * y1, w, x2, y2, 1.0], 3, eps, 0)
        IF NOT any THEN
          minX = x0
          maxX = x0
          minY = y0
          maxY = y0
          any = TRUE
        END IF
        minX = __canvas_minF(__canvas_minF(minX, x0), x2)
        maxX = __canvas_maxF(__canvas_maxF(maxX, x0), x2)
        minY = __canvas_minF(__canvas_minF(minY, y0), y2)
        maxY = __canvas_maxF(__canvas_maxF(maxY, y0), y2)
        IF (x1 - x0 > 0.0 AND x2 - x1 < 0.0) OR (x1 - x0 < 0.0 AND x2 - x1 > 0.0) THEN
          LET turn AS Float = __canvas_cvTurn(x0, x1, x2, w)
          minX = __canvas_minF(minX, turn)
          maxX = __canvas_maxF(maxX, turn)
        END IF
        i = i + 1
      END WHILE
      ' A contour ends on the point it began on, and a ring closes itself: the last
      ' point is left out.
      MUT ring AS List OF vector::Float2 = []
      MUT k AS Integer = 0
      WHILE k + 3 < len(flat)
        ring = collections::append(ring, vector::Float2[x := collections::getOr(flat, k, 0.0), y := collections::getOr(flat, k + 1, 0.0)])
        k = k + 2
      END WHILE
      rings = collections::append(rings, [ring])
    END IF
  NEXT
  __CANVAS_CURVE_EXTENT = []
  IF any THEN
    __CANVAS_CURVE_EXTENT = [minX, minY, maxX, maxY]
  END IF
  RETURN rings
END FUNC

' plan-164-B: a curve shape's record -- its header and its tail, as
' `__canvas_geometryFor` appends them. `contours` is a `CurvePolygon`'s, or a
' `PolyCurve`'s one chain; `kind` says which, and so whether a contour is closed.
'
' The pieces are built where every shape's geometry is: at `position` when the shape's
' transform is not in effect, at the local origin when it is (`__canvas_placement`).
' The tail is pieces | join records | dash tail | gradient stops, the polygon's order
' with pieces for edges, so the parts after the pieces are found from the END of the
' record by the functions that find them for every kind.
FUNC __canvas_curveBuild(kind AS Integer, contours AS List OF List OF BezierCurve, rule AS FillRule, paint AS Paint, position AS vector::Float2, transform AS Transform) AS List OF List OF Float
  LET closed AS Boolean = kind = __CANVAS_GEO_CURVEPOLYGON
  LET ox AS Float = __canvas_buildX(position, transform)
  LET oy AS Float = __canvas_buildY(position, transform)
  LET eps AS Float = __canvas_curveEps(transform)
  LET none AS List OF Float = []
  __CANVAS_CURVE_MARKS = []
  __CANVAS_CURVE_TANGENTS = []
  MUT pieces AS List OF Float = []
  MUT starts AS List OF Integer = []
  FOR EACH contour IN contours
    LET base AS Integer = len(pieces) / __CANVAS_CURVE_PIECE
    LET run AS List OF Float = __canvas_curveChain(contour, closed, eps, ox, oy, base)
    IF len(run) > 0 THEN
      starts = collections::append(starts, base)
      pieces = collections::append(pieces, run)
    END IF
  NEXT
  ' A chain has no interior: it is drawn from its stroke, like a PolyLine.
  MUT half AS Float = __canvas_strokeHalf(paint)
  LET cap AS Integer = __canvas_capTag(paint.cap)
  IF NOT closed THEN
    IF half <= 0.0 THEN
      RETURN [__canvas_emptyHeader(), none]
    END IF
    ' A chain whose every curve is a single point draws what a zero-length Line with
    ' its cap draws: a dot for Round, a square for Square, nothing for Butt.
    IF len(pieces) = 0 AND __CANVAS_CURVE_HAVE AND cap <> 0 THEN
      pieces = __canvas_cvLine(pieces, __CANVAS_CURVE_FIRSTX, __CANVAS_CURVE_FIRSTY, __CANVAS_CURVE_FIRSTX, __CANVAS_CURVE_FIRSTY)
      starts = [0]
    END IF
  END IF
  LET count AS Integer = len(pieces) / __CANVAS_CURVE_PIECE
  IF count = 0 THEN
    RETURN [__canvas_emptyHeader(), none]
  END IF
  MUT minX AS Float = collections::getOr(pieces, 0, 0.0)
  MUT maxX AS Float = minX
  MUT minY AS Float = collections::getOr(pieces, 1, 0.0)
  MUT maxY AS Float = minY
  MUT i AS Integer = 0
  WHILE i < count
    MUT k AS Integer = 0
    WHILE k < 3
      LET qx AS Float = collections::getOr(pieces, i * __CANVAS_CURVE_PIECE + k * 2, 0.0)
      LET qy AS Float = collections::getOr(pieces, i * __CANVAS_CURVE_PIECE + k * 2 + 1, 0.0)
      minX = __canvas_minF(minX, qx)
      maxX = __canvas_maxF(maxX, qx)
      minY = __canvas_minF(minY, qy)
      maxY = __canvas_maxF(maxY, qy)
      k = k + 1
    END WHILE
    i = i + 1
  END WHILE
  MUT header AS List OF Float = __canvas_blankHeader()
  header = collections::set(header, 0, toFloat(kind))
  header = collections::set(header, 1, toFloat(__CANVAS_GEO_HEADER + len(pieces)))
  IF NOT closed THEN
    header = collections::set(header, 6, half)
    header = collections::set(header, __CANVAS_GEO_CAP, toFloat(cap))
  END IF
  header = __canvas_paintHeader(header, paint, __canvas_placement(position, transform), ox, oy)
  IF NOT closed THEN
    header = __canvas_strokeAsFill(header)
  ELSEIF rule = FillRule.NonZero THEN
    header = collections::set(header, __CANVAS_GEO_POLYGON_RULE, 1.0)
  END IF
  header = collections::set(header, 20, toFloat(count))
  LET joins AS List OF Float = __canvas_curveJoins(pieces, starts, closed, half, paint)
  MUT dashes AS List OF Float = []
  LET pattern AS List OF Float = __canvas_dashPattern(paint, half)
  IF len(pattern) > 0 THEN
    dashes = __canvas_dashTail(pattern, __canvas_curveDashData(pieces, starts, closed))
  END IF
  MUT tail AS List OF Float = collections::append(collections::append(pieces, joins), dashes)
  MUT pad AS Float = __canvas_strokePad(paint)
  IF closed THEN
    tail = __canvas_appendGradientTail(tail, paint)
  ELSE
    tail = __canvas_strokeRampTail(tail, paint)
    pad = __canvas_maxF(pad, __canvas_capReach(half, cap) + 1.0)
  END IF
  header = __canvas_withDashes(__canvas_withJoins(header, joins, __canvas_joinReach(half, paint)), dashes, paint)
  RETURN [__canvas_boundsHeader(header, minX - pad, minY - pad, maxX + pad, maxY + pad), tail]
END FUNC

' plan-164-A: a chain of curves -- its count, then each curve's points and weights
' after their counts, so a point moved from one curve to the next changes the key.
FUNC __canvas_hashCurves(acc AS Integer, curves AS List OF BezierCurve) AS Integer
  MUT h AS Integer = __canvas_hashStep(acc, len(curves))
  FOR EACH curve IN curves
    h = __canvas_hashStep(h, len(curve.points))
    FOR EACH q IN curve.points
      h = __canvas_hashPoint(h, q)
    NEXT
    h = __canvas_hashStep(h, len(curve.weights))
    FOR EACH w IN curve.weights
      h = __canvas_hashFloat(h, w)
    NEXT
  NEXT
  RETURN h
END FUNC

' plan-164-A: the item hash of each shape, field by field. Both are hashed natively
' (`func_item_hash.rs`); these are what `__canvas_hashItem`'s arms answer if that ever
' changes.
FUNC __canvas_polyCurveHash(pc AS PolyCurve) AS Integer
  MUT h AS Integer = __canvas_hashStart(12)
  h = __canvas_hashPoint(h, pc.position)
  h = __canvas_hashCurves(h, pc.curves)
  RETURN __canvas_hashPaint(h, pc.paint, pc.transform)
END FUNC

FUNC __canvas_curvePolygonHash(cp AS CurvePolygon) AS Integer
  MUT h AS Integer = __canvas_hashStart(13)
  h = __canvas_hashPoint(h, cp.position)
  h = __canvas_hashStep(h, len(cp.contours))
  FOR EACH contour IN cp.contours
    h = __canvas_hashCurves(h, contour)
  NEXT
  IF cp.rule = FillRule.NonZero THEN
    h = __canvas_hashStep(h, 1)
  ELSE
    h = __canvas_hashStep(h, 0)
  END IF
  RETURN __canvas_hashPaint(h, cp.paint, cp.transform)
END FUNC

' The record of each shape: a chain is one open contour.
FUNC __canvas_polyCurveRecord(pc AS PolyCurve) AS List OF List OF Float
  RETURN __canvas_curveBuild(__CANVAS_GEO_POLYCURVE, [pc.curves], FillRule.EvenOdd, pc.paint, pc.position, pc.transform)
END FUNC

FUNC __canvas_curvePolygonRecord(cp AS CurvePolygon) AS List OF List OF Float
  RETURN __canvas_curveBuild(__CANVAS_GEO_CURVEPOLYGON, cp.contours, cp.rule, cp.paint, cp.position, cp.transform)
END FUNC"#;

/// One piece at one point: its nearest point, the box that bounds it, and where it
/// crosses a row (plan-164-B).
///
/// Evaluated at a point in the record's own space — the renderer has already taken the
/// pixel through the draw entry's inverse and the item's — with `+ - * /` and `sqrt`
/// only, so the software renderer answers the same on every platform and both shaders
/// can be written from it line for line.
///
/// Its own chunk because `tests/canvas/rt_canvas_curves.rs` lifts it out and runs it
/// against a brute-force search: beyond [`CURVE_BASE`] it names only
/// `__CANVAS_GEO_DATA`, `__canvas_minF` and `__canvas_maxF`.
#[rustfmt::skip]
pub(super) const CURVE_NEAREST: &str =
r#"' plan-164-B: the nearest point of one piece.
'
' A piece is `P(t) = N(t) / W(t)`. Measured from the query point p, with
'   Q(t) = N(t) - p W(t) = A t^2 + B t + C,   W(t) = 1 + b t (1 - t),
' the squared distance is `D(t) = Q.Q / W^2`, and its slope has the sign of
'   g(t) = (Q.Q') W - (Q.Q) W'
' -- a quartic for a rational piece, a cubic for a polynomial one (`b = 0`). A local
' minimum of the distance is a root of `g` where it goes from negative to positive.
'
' The roots are ISOLATED, not searched for. `g''` is a quadratic, so its roots are
' closed form; between them `g'` is monotone, so each stretch holds at most one root of
' `g'`, found by a bracketed Newton; between THOSE `g` is monotone, and each stretch
' that goes from negative to positive holds exactly one minimum, found the same way.
' Nothing is sampled, so no minimum can fall between samples, and the answer does not
' get worse as a piece gets longer. Only `+ - * /` and `sqrt` are used.
'
' The coefficients of the piece under test live in these, so that the evaluations
' below take one argument and the per-pixel path allocates nothing.
MUT __CANVAS_PC_AX AS Float = 0.0
MUT __CANVAS_PC_AY AS Float = 0.0
MUT __CANVAS_PC_BX AS Float = 0.0
MUT __CANVAS_PC_BY AS Float = 0.0
MUT __CANVAS_PC_CX AS Float = 0.0
MUT __CANVAS_PC_CY AS Float = 0.0
MUT __CANVAS_PC_WB AS Float = 0.0
' The best squared distance so far, and the parameter it is at.
MUT __CANVAS_PC_D2 AS Float = 0.0
MUT __CANVAS_PIECE_T AS Float = 0.0
' How many steps a bracketed Newton takes. Each step halves the bracket at worst and
' squares the error at best; 12 is past a Float32's resolution of `t` either way.
LET __CANVAS_PIECE_STEPS AS Integer = 12

' `g` (order 0), `g'` (1) or `g''` (2) at `t`.
FUNC __canvas_pcF(order AS Integer, t AS Float) AS Float
  LET qx AS Float = (__CANVAS_PC_AX * t + __CANVAS_PC_BX) * t + __CANVAS_PC_CX
  LET qy AS Float = (__CANVAS_PC_AY * t + __CANVAS_PC_BY) * t + __CANVAS_PC_CY
  LET dx AS Float = 2.0 * __CANVAS_PC_AX * t + __CANVAS_PC_BX
  LET dy AS Float = 2.0 * __CANVAS_PC_AY * t + __CANVAS_PC_BY
  LET w AS Float = 1.0 + __CANVAS_PC_WB * t * (1.0 - t)
  LET dw AS Float = __CANVAS_PC_WB * (1.0 - 2.0 * t)
  LET qd AS Float = qx * dx + qy * dy
  IF order = 0 THEN
    RETURN qd * w - (qx * qx + qy * qy) * dw
  END IF
  IF order = 1 THEN
    RETURN (dx * dx + dy * dy + 2.0 * (qx * __CANVAS_PC_AX + qy * __CANVAS_PC_AY)) * w - qd * dw + 2.0 * __CANVAS_PC_WB * (qx * qx + qy * qy)
  END IF
  RETURN 6.0 * (dx * __CANVAS_PC_AX + dy * __CANVAS_PC_AY) * w + 6.0 * __CANVAS_PC_WB * qd
END FUNC

' The root of the order-`order` function between `lo` and `hi`, where it is known to
' change sign: Newton's step when it lands in the bracket -- its ends included, so a
' step that has arrived stays where it is -- and the midpoint when not.
FUNC __canvas_pcRoot(order AS Integer, lo0 AS Float, hi0 AS Float) AS Float
  MUT lo AS Float = lo0
  MUT hi AS Float = hi0
  LET rising AS Boolean = __canvas_pcF(order, lo0) < 0.0
  MUT t AS Float = (lo + hi) / 2.0
  MUT k AS Integer = 0
  WHILE k < __CANVAS_PIECE_STEPS
    LET f AS Float = __canvas_pcF(order, t)
    IF (f < 0.0) = rising THEN
      lo = t
    ELSE
      hi = t
    END IF
    LET slope AS Float = __canvas_pcF(order + 1, t)
    MUT nextT AS Float = (lo + hi) / 2.0
    ' `|f| < |slope|` keeps the step under 1, so the divide cannot overflow.
    IF __canvas_cvAbs(f) < __canvas_cvAbs(slope) THEN
      LET moved AS Float = t - f / slope
      IF moved >= lo AND moved <= hi THEN
        nextT = moved
      END IF
    END IF
    t = nextT
    k = k + 1
  END WHILE
  RETURN t
END FUNC

' A parameter as a candidate for the nearest point.
FUNC __canvas_pcTry(t AS Float) AS Nothing
  LET qx AS Float = (__CANVAS_PC_AX * t + __CANVAS_PC_BX) * t + __CANVAS_PC_CX
  LET qy AS Float = (__CANVAS_PC_AY * t + __CANVAS_PC_BY) * t + __CANVAS_PC_CY
  LET w AS Float = 1.0 + __CANVAS_PC_WB * t * (1.0 - t)
  LET d2 AS Float = (qx * qx + qy * qy) / (w * w)
  IF d2 < __CANVAS_PC_D2 THEN
    __CANVAS_PC_D2 = d2
    __CANVAS_PIECE_T = t
  END IF
END FUNC

' The root of `g'` between `lo` and `hi` when it changes sign there; 2.0 when not.
FUNC __canvas_pcTurn(lo AS Float, hi AS Float) AS Float
  LET flo AS Float = __canvas_pcF(1, lo)
  LET fhi AS Float = __canvas_pcF(1, hi)
  IF (flo < 0.0 AND fhi > 0.0) OR (flo > 0.0 AND fhi < 0.0) THEN
    RETURN __canvas_pcRoot(1, lo, hi)
  END IF
  RETURN 2.0
END FUNC

' The stretch `lo`..`hi`, on which `g` is monotone: its upper end is a candidate, and
' so is the minimum inside it when `g` goes from negative to positive there.
FUNC __canvas_pcStretch(lo AS Float, hi AS Float) AS Nothing
  IF __canvas_pcF(0, lo) < 0.0 AND __canvas_pcF(0, hi) > 0.0 THEN
    __canvas_pcTry(__canvas_pcRoot(0, lo, hi))
  END IF
  __canvas_pcTry(hi)
END FUNC

' How far (px, py) is from the piece at `base` in `__CANVAS_GEO_DATA`; the parameter of
' its nearest point is left in `__CANVAS_PIECE_T`, 0.0 or 1.0 when that is an end.
FUNC __canvas_pieceNearest(base AS Integer, px AS Float, py AS Float) AS Float
  LET w AS Float = collections::getOr(__CANVAS_GEO_DATA, base + 6, 1.0)
  LET q0x AS Float = collections::getOr(__CANVAS_GEO_DATA, base, 0.0) - px
  LET q0y AS Float = collections::getOr(__CANVAS_GEO_DATA, base + 1, 0.0) - py
  LET q1x AS Float = w * (collections::getOr(__CANVAS_GEO_DATA, base + 2, 0.0) - px)
  LET q1y AS Float = w * (collections::getOr(__CANVAS_GEO_DATA, base + 3, 0.0) - py)
  LET q2x AS Float = collections::getOr(__CANVAS_GEO_DATA, base + 4, 0.0) - px
  LET q2y AS Float = collections::getOr(__CANVAS_GEO_DATA, base + 5, 0.0) - py
  __CANVAS_PC_AX = q0x - 2.0 * q1x + q2x
  __CANVAS_PC_AY = q0y - 2.0 * q1y + q2y
  __CANVAS_PC_BX = 2.0 * (q1x - q0x)
  __CANVAS_PC_BY = 2.0 * (q1y - q0y)
  __CANVAS_PC_CX = q0x
  __CANVAS_PC_CY = q0y
  __CANVAS_PC_WB = 2.0 * (w - 1.0)
  __CANVAS_PC_D2 = q0x * q0x + q0y * q0y
  __CANVAS_PIECE_T = 0.0
  ' `g''` is a quadratic: fitted through its values at 0, 1/2 and 1, then solved for
  ' its roots strictly inside the piece. A root is `q / alpha` or `gamma / q`; each is
  ' divided only once it is known to lie in (0, 1), so no divide can overflow.
  LET h0 AS Float = __canvas_pcF(2, 0.0)
  LET hm AS Float = __canvas_pcF(2, 0.5)
  LET h1 AS Float = __canvas_pcF(2, 1.0)
  LET alpha AS Float = 2.0 * (h0 + h1) - 4.0 * hm
  LET beta AS Float = h1 - h0 - alpha
  MUT r1 AS Float = 2.0
  MUT r2 AS Float = 2.0
  LET disc AS Float = beta * beta - 4.0 * alpha * h0
  IF disc >= 0.0 THEN
    MUT q AS Float = 0.0 - (beta + math::sqrt(disc)) / 2.0
    IF beta < 0.0 THEN
      q = 0.0 - (beta - math::sqrt(disc)) / 2.0
    END IF
    IF __canvas_cvAbs(q) < __canvas_cvAbs(alpha) AND (q > 0.0) = (alpha > 0.0) AND q <> 0.0 THEN
      r1 = q / alpha
    END IF
    IF __canvas_cvAbs(h0) < __canvas_cvAbs(q) AND (h0 > 0.0) = (q > 0.0) AND h0 <> 0.0 THEN
      LET other AS Float = h0 / q
      IF other < r1 THEN
        r2 = r1
        r1 = other
      ELSE
        r2 = other
      END IF
    END IF
  END IF
  ' The roots of `g'`, one to a stretch between those, in order.
  MUT e1 AS Float = 1.0
  IF r1 < 1.0 THEN
    e1 = r1
  END IF
  LET a1 AS Float = __canvas_pcTurn(0.0, e1)
  MUT a2 AS Float = 2.0
  MUT a3 AS Float = 2.0
  IF r1 < 1.0 THEN
    MUT e2 AS Float = 1.0
    IF r2 < 1.0 THEN
      e2 = r2
    END IF
    a2 = __canvas_pcTurn(r1, e2)
    IF r2 < 1.0 THEN
      a3 = __canvas_pcTurn(r2, 1.0)
    END IF
  END IF
  ' The stretches of `g` between those, each with at most one minimum.
  MUT lo AS Float = 0.0
  IF a1 < 1.5 THEN
    __canvas_pcStretch(lo, a1)
    lo = a1
  END IF
  IF a2 < 1.5 THEN
    __canvas_pcStretch(lo, a2)
    lo = a2
  END IF
  IF a3 < 1.5 THEN
    __canvas_pcStretch(lo, a3)
    lo = a3
  END IF
  __canvas_pcStretch(lo, 1.0)
  RETURN math::sqrt(__CANVAS_PC_D2)
END FUNC

' A bound below the distance from (px, py) to the piece at `base`: its distance to the
' box round the piece's three control points, which hold the piece between them.
FUNC __canvas_pieceBound(base AS Integer, px AS Float, py AS Float) AS Float
  LET x0 AS Float = collections::getOr(__CANVAS_GEO_DATA, base, 0.0)
  LET y0 AS Float = collections::getOr(__CANVAS_GEO_DATA, base + 1, 0.0)
  LET x1 AS Float = collections::getOr(__CANVAS_GEO_DATA, base + 2, 0.0)
  LET y1 AS Float = collections::getOr(__CANVAS_GEO_DATA, base + 3, 0.0)
  LET x2 AS Float = collections::getOr(__CANVAS_GEO_DATA, base + 4, 0.0)
  LET y2 AS Float = collections::getOr(__CANVAS_GEO_DATA, base + 5, 0.0)
  LET dx AS Float = __canvas_maxF(__canvas_maxF(__canvas_minF(__canvas_minF(x0, x1), x2) - px, px - __canvas_maxF(__canvas_maxF(x0, x1), x2)), 0.0)
  LET dy AS Float = __canvas_maxF(__canvas_maxF(__canvas_minF(__canvas_minF(y0, y1), y2) - py, py - __canvas_maxF(__canvas_maxF(y0, y1), y2)), 0.0)
  RETURN math::sqrt(dx * dx + dy * dy)
END FUNC

' Where the piece at `base` crosses the row `py`: the x of its point there. Asked only
' of a piece whose two ends are on opposite sides of the row -- `(y0 > py) <> (y2 > py)`
' -- and a piece is y-monotone, so there is one such point.
'
' With `a = y0 - py`, `b = w1 (y1 - py)` and `c = y2 - py`, the point is the root of
' `(a - 2b + c) t^2 + 2 (b - a) t + a` in [0, 1]. It is taken as
'   t = a / (a - b + s),   s = sqrt(b^2 - a c) with the sign of (a - c),
' the form of the quadratic formula that divides by the LARGER of its two candidates:
' `a` and `c` have opposite signs, so `b^2 - a c` is at least `b^2`, the divisor is at
' least `|a|` in size, and `t` is in [0, 1] without a cancellation or a divide by zero.
' A straight piece (`a - 2b + c = 0`) needs no case of its own.
FUNC __canvas_pieceCrossX(base AS Integer, py AS Float) AS Float
  LET x0 AS Float = collections::getOr(__CANVAS_GEO_DATA, base, 0.0)
  LET x1 AS Float = collections::getOr(__CANVAS_GEO_DATA, base + 2, 0.0)
  LET x2 AS Float = collections::getOr(__CANVAS_GEO_DATA, base + 4, 0.0)
  LET w AS Float = collections::getOr(__CANVAS_GEO_DATA, base + 6, 1.0)
  LET a AS Float = collections::getOr(__CANVAS_GEO_DATA, base + 1, 0.0) - py
  LET b AS Float = w * (collections::getOr(__CANVAS_GEO_DATA, base + 3, 0.0) - py)
  LET c AS Float = collections::getOr(__CANVAS_GEO_DATA, base + 5, 0.0) - py
  MUT t AS Float = 0.0
  IF a <> 0.0 THEN
    MUT s AS Float = math::sqrt(b * b - a * c)
    IF a < c THEN
      s = 0.0 - s
    END IF
    t = a / (a - b + s)
  END IF
  t = __canvas_minF(__canvas_maxF(t, 0.0), 1.0)
  LET u AS Float = 1.0 - t
  LET k0 AS Float = u * u
  LET k1 AS Float = 2.0 * w * u * t
  LET k2 AS Float = t * t
  RETURN (k0 * x0 + k1 * x1 + k2 * x2) / (k0 + k1 + k2)
END FUNC
"#;

/// The draw side: a `CurvePolygon`'s signed distance, and both shapes' strokes —
/// plain, joined and dashed — from the pieces (plan-164-B).
#[rustfmt::skip]
pub(super) const CURVE_DRAW: &str =
r#"' The signed distance to a `CurvePolygon`'s outline, negative inside under its rule.
' The polygon's walk (`__canvas_edgeDistanceRule`) with pieces for edges: the nearest
' piece for the size, a crossing count for the sign. A piece crosses the pixel's row
' when its ends straddle it -- the half-open test a polygon edge takes -- and counts
' when it passes left of the pixel; under `NonZero` it adds +1 running down the screen
' and -1 running up.
'
' A piece is measured only when the box round its control points is nearer than the
' best so far, and the best starts as the nearest piece END, which costs no solve: for
' most pixels that leaves one or two pieces to measure.
FUNC __canvas_curveDistance(tail AS Integer, pieces AS Integer, px AS Float, py AS Float) AS Float
  LET nonZero AS Boolean = collections::getOr(__CANVAS_GEO_DATA, tail - __CANVAS_GEO_HEADER + __CANVAS_GEO_POLYGON_RULE, 0.0) <> 0.0
  MUT best2 AS Float = 1000000000000.0
  MUT inside AS Boolean = FALSE
  MUT winding AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < pieces
    LET base AS Integer = tail + i * __CANVAS_CURVE_PIECE
    LET ex AS Float = collections::getOr(__CANVAS_GEO_DATA, base, 0.0) - px
    LET y0 AS Float = collections::getOr(__CANVAS_GEO_DATA, base + 1, 0.0)
    LET y2 AS Float = collections::getOr(__CANVAS_GEO_DATA, base + 5, 0.0)
    best2 = __canvas_minF(best2, ex * ex + (y0 - py) * (y0 - py))
    IF (y0 > py) <> (y2 > py) THEN
      IF px < __canvas_pieceCrossX(base, py) THEN
        IF nonZero THEN
          IF y2 > y0 THEN
            winding = winding + 1
          ELSE
            winding = winding - 1
          END IF
        ELSE
          inside = NOT inside
        END IF
      END IF
    END IF
    i = i + 1
  END WHILE
  MUT best AS Float = math::sqrt(best2)
  i = 0
  WHILE i < pieces
    LET base AS Integer = tail + i * __CANVAS_CURVE_PIECE
    IF __canvas_pieceBound(base, px, py) < best THEN
      best = __canvas_minF(best, __canvas_pieceNearest(base, px, py))
    END IF
    i = i + 1
  END WHILE
  IF nonZero THEN
    inside = winding <> 0
  END IF
  IF inside THEN
    RETURN 0.0 - best
  END IF
  RETURN best
END FUNC

' The frame of one end of the piece at `base`, for a pixel whose nearest point of the
' piece IS that end: `__CANVAS_PIECE_ALONG` is how far the pixel is along the curve's
' direction there, measured from the end (negative behind the start, positive past the
' far end), and `__CANVAS_PIECE_SIDE` how far it is from the tangent line. FALSE for a
' piece with no length, which has no direction.
MUT __CANVAS_PIECE_ALONG AS Float = 0.0
MUT __CANVAS_PIECE_SIDE AS Float = 0.0

FUNC __canvas_pieceEndFrame(base AS Integer, px AS Float, py AS Float, atEnd AS Boolean) AS Boolean
  LET x0 AS Float = collections::getOr(__CANVAS_GEO_DATA, base, 0.0)
  LET y0 AS Float = collections::getOr(__CANVAS_GEO_DATA, base + 1, 0.0)
  LET x1 AS Float = collections::getOr(__CANVAS_GEO_DATA, base + 2, 0.0)
  LET y1 AS Float = collections::getOr(__CANVAS_GEO_DATA, base + 3, 0.0)
  LET x2 AS Float = collections::getOr(__CANVAS_GEO_DATA, base + 4, 0.0)
  LET y2 AS Float = collections::getOr(__CANVAS_GEO_DATA, base + 5, 0.0)
  MUT ex AS Float = x0
  MUT ey AS Float = y0
  MUT tx AS Float = x1 - x0
  MUT ty AS Float = y1 - y0
  IF atEnd THEN
    ex = x2
    ey = y2
    tx = x2 - x1
    ty = y2 - y1
  END IF
  ' A control point on the end itself gives no direction; the chord does.
  IF tx = 0.0 AND ty = 0.0 THEN
    tx = x2 - x0
    ty = y2 - y0
  END IF
  LET len2 AS Float = tx * tx + ty * ty
  IF NOT (len2 > 0.0) THEN
    RETURN FALSE
  END IF
  LET length AS Float = math::sqrt(len2)
  LET wx AS Float = px - ex
  LET wy AS Float = py - ey
  __CANVAS_PIECE_ALONG = (wx * tx + wy * ty) / length
  __CANVAS_PIECE_SIDE = __canvas_absF(wx * ty - wy * tx) / length
  RETURN TRUE
END FUNC

' One end of a piece's band, cut as `cap` says -- 0 Butt, 2 Square -- given the piece's
' round band `band` there and whether the pixel's nearest point of the piece IS that
' end (`nearest`).
'
' Past the end, the answer is the two terms a `canvas::Line`'s end is the `max` of: for
' Butt the round band and the distance past the end plane, for Square the band of the
' tangent LINE (so the corners are square) and the distance past a plane `half` beyond.
' Short of the end the plane is still an edge -- a pixel within one pixel of it is partly
' cut -- so it is taken there too, but only for a pixel in the cap's own neighbourhood:
' a piece that curls back behind its own end plane keeps its band everywhere else.
FUNC __canvas_pieceCut(base AS Integer, px AS Float, py AS Float, half AS Float, atEnd AS Boolean, cap AS Integer, band AS Float, nearest AS Boolean) AS Float
  IF NOT __canvas_pieceEndFrame(base, px, py, atEnd) THEN
    IF NOT nearest THEN
      RETURN band
    END IF
    ' A piece with no length: the square its Square cap draws, nothing for Butt.
    IF cap = 2 THEN
      RETURN __canvas_maxF(__canvas_absF(px - collections::getOr(__CANVAS_GEO_DATA, base, 0.0)), __canvas_absF(py - collections::getOr(__CANVAS_GEO_DATA, base + 1, 0.0))) - half
    END IF
    RETURN 1000000.0
  END IF
  MUT plane AS Float = 0.0 - __CANVAS_PIECE_ALONG
  IF atEnd THEN
    plane = __CANVAS_PIECE_ALONG
  END IF
  IF cap = 2 THEN
    plane = plane - half
  END IF
  IF nearest THEN
    IF cap = 2 THEN
      RETURN __canvas_maxF(__CANVAS_PIECE_SIDE - half, plane)
    END IF
    RETURN __canvas_maxF(band, plane)
  END IF
  LET zone AS Float = half * 1.5 + 2.0
  IF plane > 0.0 - 1.0 AND __CANVAS_PIECE_ALONG * __CANVAS_PIECE_ALONG + __CANVAS_PIECE_SIDE * __CANVAS_PIECE_SIDE <= zone * zone THEN
    RETURN __canvas_maxF(band, plane)
  END IF
  RETURN band
END FUNC

' One piece's part of a stroke of half-width `half`: the band `half` either side of it,
' ended at each end as `startCap` and `endCap` say -- 0 Butt (cut square at the end), 1
' Round (the band of the distance, which is round there), 2 Square (cut square `half`
' past the end).
FUNC __canvas_pieceBand(base AS Integer, px AS Float, py AS Float, half AS Float, startCap AS Integer, endCap AS Integer) AS Float
  LET d AS Float = __canvas_pieceNearest(base, px, py)
  LET t AS Float = __CANVAS_PIECE_T
  MUT band AS Float = d - half
  IF startCap <> 1 THEN
    band = __canvas_pieceCut(base, px, py, half, FALSE, startCap, band, t <= 0.0)
  END IF
  IF endCap <> 1 THEN
    band = __canvas_pieceCut(base, px, py, half, TRUE, endCap, band, t >= 1.0)
  END IF
  RETURN band
END FUNC

' The distance to a `PolyCurve`'s stroke: the union of its pieces' bands. Inner ends
' are round -- two pieces of one smooth curve meet with no seam, and a corner between
' two curves is the round join -- unless the record has join records (a Miter or Bevel
' paint), when every inner end is cut square and the records are the corners and the
' overlap at the smooth joints (`helper_joins.rs`). The first piece's start and the
' last piece's end take the paint's cap.
FUNC __canvas_curveScan(tail AS Integer, pieces AS Integer, px AS Float, py AS Float, half AS Float, cap AS Integer) AS Float
  LET offset AS Integer = tail - __CANVAS_GEO_HEADER
  LET joins AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, offset + __CANVAS_GEO_JOIN_COUNT, 0.0))
  MUT inner AS Integer = 1
  IF joins > 0 THEN
    inner = 0
  END IF
  ' A Square end's corner is `half * sqrt(2)` from the end point.
  LET reach AS Float = half * 1.4142135623730951
  MUT best AS Float = 1000000.0
  MUT i AS Integer = 0
  WHILE i < pieces
    LET base AS Integer = tail + i * __CANVAS_CURVE_PIECE
    IF __canvas_pieceBound(base, px, py) - reach < best THEN
      MUT startCap AS Integer = inner
      MUT endCap AS Integer = inner
      IF i = 0 THEN
        startCap = cap
      END IF
      IF i = pieces - 1 THEN
        endCap = cap
      END IF
      best = __canvas_minF(best, __canvas_pieceBand(base, px, py, half, startCap, endCap))
    END IF
    i = i + 1
  END WHILE
  IF joins > 0 THEN
    RETURN __canvas_minF(best, __canvas_joinsDistance(offset, px, py))
  END IF
  RETURN best
END FUNC

' The stroke distance of a `CurvePolygon` with join records: every piece's band cut
' square at both ends, unioned with the records.
FUNC __canvas_curveJoined(offset AS Integer, tail AS Integer, pieces AS Integer, px AS Float, py AS Float, half AS Float) AS Float
  MUT best AS Float = __canvas_joinsDistance(offset, px, py)
  MUT i AS Integer = 0
  WHILE i < pieces
    LET base AS Integer = tail + i * __CANVAS_CURVE_PIECE
    IF __canvas_pieceBound(base, px, py) - half < best THEN
      best = __canvas_minF(best, __canvas_pieceBand(base, px, py, half, 0, 0))
    END IF
    i = i + 1
  END WHILE
  RETURN best
END FUNC

' The outline position of the point at `t` of a piece whose arc-length table starts at
' `row` in `__CANVAS_GEO_DATA`: the table holds the position at each sixteenth of the
' piece, and between two of them it is read off the straight line.
FUNC __canvas_pieceArc(row AS Integer, t AS Float) AS Float
  LET f AS Float = t * toFloat(__CANVAS_CURVE_ARC_STEPS)
  LET k AS Integer = __canvas_minI(__canvas_maxI(toInt(f), 0), __CANVAS_CURVE_ARC_STEPS - 1)
  LET c0 AS Float = collections::getOr(__CANVAS_GEO_DATA, row + k, 0.0)
  LET c1 AS Float = collections::getOr(__CANVAS_GEO_DATA, row + k + 1, 0.0)
  RETURN c0 + (c1 - c0) * (f - toFloat(k))
END FUNC

' The point of the piece at `base` at outline position `s`: its X, with its Y left in
' `__CANVAS_DASH_Y`. The table is searched for the sixteenth holding `s`, and `t` read
' back off the same straight line `__canvas_pieceArc` reads forwards.
FUNC __canvas_pieceAtArc(base AS Integer, row AS Integer, s AS Float) AS Float
  MUT lo AS Integer = 0
  MUT hi AS Integer = __CANVAS_CURVE_ARC_STEPS
  WHILE hi - lo > 1
    LET mid AS Integer = (lo + hi) / 2
    IF collections::getOr(__CANVAS_GEO_DATA, row + mid, 0.0) <= s THEN
      lo = mid
    ELSE
      hi = mid
    END IF
  END WHILE
  LET c0 AS Float = collections::getOr(__CANVAS_GEO_DATA, row + lo, 0.0)
  LET c1 AS Float = collections::getOr(__CANVAS_GEO_DATA, row + lo + 1, 0.0)
  MUT f AS Float = 0.0
  IF c1 > c0 THEN
    f = __canvas_minF(__canvas_maxF((s - c0) / (c1 - c0), 0.0), 1.0)
  END IF
  LET t AS Float = (toFloat(lo) + f) / toFloat(__CANVAS_CURVE_ARC_STEPS)
  LET w AS Float = collections::getOr(__CANVAS_GEO_DATA, base + 6, 1.0)
  LET u AS Float = 1.0 - t
  LET k0 AS Float = u * u
  LET k1 AS Float = 2.0 * w * u * t
  LET k2 AS Float = t * t
  LET den AS Float = k0 + k1 + k2
  __CANVAS_DASH_Y = (k0 * collections::getOr(__CANVAS_GEO_DATA, base + 1, 0.0) + k1 * collections::getOr(__CANVAS_GEO_DATA, base + 3, 0.0) + k2 * collections::getOr(__CANVAS_GEO_DATA, base + 5, 0.0)) / den
  RETURN (k0 * collections::getOr(__CANVAS_GEO_DATA, base, 0.0) + k1 * collections::getOr(__CANVAS_GEO_DATA, base + 2, 0.0) + k2 * collections::getOr(__CANVAS_GEO_DATA, base + 4, 0.0)) / den
END FUNC

' A dashed curve shape's stroke distance: `__canvas_dashedEdges` with pieces for edges.
' Each piece's band is cut by the dashes that lie on it, at the outline position of the
' pixel's nearest point of the piece -- read from the piece's arc-length table, or, for
' a pixel past one of the piece's ends, measured on along the tangent there, as an edge
' measures along its line. The corner piece at a piece's start is drawn when a dash
' runs through it: a disc where the curve is smooth, the join where it is not.
FUNC __canvas_dashedCurves(kind AS Integer, offset AS Integer, px AS Float, py AS Float, half AS Float, cap AS Integer) AS Float
  LET tail AS Integer = offset + __CANVAS_GEO_HEADER
  LET dbase AS Integer = __canvas_dashBase(offset)
  LET data AS Integer = dbase + 2 + toInt(collections::getOr(__CANVAS_GEO_DATA, dbase, 0.0))
  LET phase AS Float = __canvas_geoAt(offset, __CANVAS_GEO_DASH_PHASE)
  LET joins AS Integer = toInt(__canvas_geoAt(offset, __CANVAS_GEO_JOIN_COUNT))
  LET jbase AS Integer = __canvas_joinBase(offset)
  LET count AS Integer = toInt(__canvas_geoAt(offset, 20))
  MUT ext AS Float = 0.0
  IF cap = 2 THEN
    ext = half
  END IF
  LET reach AS Float = half * 1.4142135623730951
  MUT best AS Float = 1000000.0
  MUT live AS Boolean = FALSE
  MUT e AS Integer = 0
  WHILE e < count
    LET base AS Integer = tail + e * __CANVAS_CURVE_PIECE
    LET row AS Integer = data + e * __CANVAS_CURVE_DASH_ROW
    LET s0 AS Float = collections::getOr(__CANVAS_GEO_DATA, row, 0.0)
    LET s1 AS Float = collections::getOr(__CANVAS_GEO_DATA, row + __CANVAS_CURVE_ARC_STEPS, 0.0)
    LET ring AS Float = collections::getOr(__CANVAS_GEO_DATA, row + __CANVAS_CURVE_ARC_STEPS + 1, 0.0)
    LET closed AS Boolean = ring > 0.0
    LET length AS Float = __canvas_absF(ring)
    IF s1 > s0 THEN
      live = TRUE
      IF __canvas_pieceBound(base, px, py) - reach < best THEN
        LET d AS Float = __canvas_pieceNearest(base, px, py)
        LET t AS Float = __CANVAS_PIECE_T
        MUT u AS Float = 0.0
        MUT band AS Float = d - half
        IF t <= 0.0 AND __canvas_pieceEndFrame(base, px, py, FALSE) THEN
          u = s0 + __CANVAS_PIECE_ALONG
          band = __CANVAS_PIECE_SIDE - half
        ELSEIF t >= 1.0 AND __canvas_pieceEndFrame(base, px, py, TRUE) THEN
          u = s1 + __CANVAS_PIECE_ALONG
          band = __CANVAS_PIECE_SIDE - half
        ELSE
          u = __canvas_pieceArc(row, t)
        END IF
        MUT piece AS Float = __canvas_maxF(band, __canvas_dashAlongPiece(offset, u, s0, s1, length, closed, ext))
        IF cap = 1 THEN
          LET hasA AS Boolean = __CANVAS_DASH_HAS_A
          LET endA AS Float = __CANVAS_DASH_END_A
          LET hasB AS Boolean = __CANVAS_DASH_HAS_B
          LET endB AS Float = __CANVAS_DASH_END_B
          IF hasA THEN
            LET ax AS Float = __canvas_pieceAtArc(base, row, endA)
            LET ay AS Float = __CANVAS_DASH_Y
            piece = __canvas_minF(piece, __canvas_segmentDistance(px, py, ax, ay, ax, ay) - half)
          END IF
          IF hasB THEN
            LET bx AS Float = __canvas_pieceAtArc(base, row, endB)
            LET by AS Float = __CANVAS_DASH_Y
            piece = __canvas_minF(piece, __canvas_segmentDistance(px, py, bx, by, bx, by) - half)
          END IF
        END IF
        best = __canvas_minF(best, piece)
      END IF
      ' The corner at this piece's start, drawn when a dash runs through it: on just
      ' before it and on at it. A closed contour's first corner is its seam, so
      ' "before" is the contour's end.
      IF e < joins THEN
        MUT before AS Float = s0
        IF closed AND s0 = 0.0 THEN
          before = length
        END IF
        IF __canvas_dashOnBefore(dbase, before + phase) AND __canvas_dashOnAt(dbase, s0 + phase) THEN
          best = __canvas_minF(best, __canvas_joinPieceDistance(jbase + e * 8, px, py))
        END IF
      END IF
    END IF
    e = e + 1
  END WHILE
  ' An open path with no length is the dot its cap draws; a closed outline with none
  ' draws nothing dashed.
  IF NOT live THEN
    IF kind = __CANVAS_GEO_POLYCURVE AND count > 0 THEN
      RETURN __canvas_dashDot(offset, px, py, collections::getOr(__CANVAS_GEO_DATA, tail, 0.0), collections::getOr(__CANVAS_GEO_DATA, tail + 1, 0.0), half, cap)
    END IF
    RETURN 1000000.0
  END IF
  RETURN best
END FUNC"#;

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
