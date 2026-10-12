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
//! ## Why the math is its own chunk
//!
//! [`CURVE_MATH`] uses nothing of the canvas package — only `Float` lists, `math::sqrt`
//! and `collections::` — so `tests/canvas/rt_canvas_curves.rs` can lift its text out of
//! this file and run it as a program of its own, the way `rt_canvas_auto_step.rs` runs
//! `AUTO_STEP`. What is tested is the text the compiler injects.
//!
//! The chunk is unconditional, not gated on a constructor as `AutoPaint`'s is: every
//! field of the three curve records has a default, so a program can hold a curve it
//! never wrote a constructor for (`MUT c AS canvas::PolyCurve`, then `WITH`), and a
//! gate keyed on constructor expressions would draw nothing for it.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

/// The floats one piece occupies in a list of pieces: `x0, y0, x1, y1, x2, y2, w1`.
/// `__CANVAS_CURVE_PIECE` in [`CURVE_MATH`] is the same number.
#[cfg(test)]
pub(crate) const PIECE_FLOATS: usize = 7;

/// The reduction, on homogeneous control points.
///
/// A curve arrives as `h`, three floats a control point: `w·x, w·y, w` (a polynomial
/// curve has every `w = 1`). Homogeneous coordinates are what make de Casteljau's
/// halving work unchanged for a rational curve.
#[rustfmt::skip]
pub(super) const CURVE_MATH: &str =
r#"' plan-164-A: the floats of one piece -- x0, y0, x1, y1, x2, y2, w1.
LET __CANVAS_CURVE_PIECE AS Integer = 7
' How many times a curve of degree 3 or more may be halved before a span is taken as
' it is. 2^16 spans of one curve is far below a pixel for anything a surface can show.
LET __CANVAS_CURVE_MAX_DEPTH AS Integer = 16
' The samples a fitted span is checked at.
LET __CANVAS_CURVE_FIT_SAMPLES AS Integer = 16

FUNC __canvas_cvAbs(v AS Float) AS Float
  IF v < 0.0 THEN
    RETURN 0.0 - v
  END IF
  RETURN v
END FUNC

FUNC __canvas_cvPiece(out AS List OF Float, x0 AS Float, y0 AS Float, x1 AS Float, y1 AS Float, x2 AS Float, y2 AS Float, w AS Float) AS List OF Float
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

/// The typed half: what a `BezierCurve` is valid as, the tolerance a shape's transform
/// asks for, and the walk along a chain of curves that adds the implied segments.
///
/// `__CANVAS_CURVE_MARKS` is the piece index each **input segment** of the chain begins
/// at — a valid curve, an implied join, the closing segment — in order. A corner can
/// only be at one of those; the reduction's own cuts are smooth.
#[rustfmt::skip]
pub(super) const CURVE_CHAIN: &str =
r#"' The reduction's tolerance, in surface pixels.
LET __CANVAS_CURVE_EPS AS Float = 0.05

MUT __CANVAS_CURVE_MARKS AS List OF Integer = []

' plan-164-A: whether a curve can be drawn. 2 to 17 points; weights either absent or one
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
          __CANVAS_CURVE_MARKS = collections::append(__CANVAS_CURVE_MARKS, base + len(out) / __CANVAS_CURVE_PIECE)
          out = __canvas_cvLine(out, lastX, lastY, hx, hy)
        END IF
      ELSE
        firstX = hx
        firstY = hy
        have = TRUE
      END IF
      __CANVAS_CURVE_MARKS = collections::append(__CANVAS_CURVE_MARKS, base + len(out) / __CANVAS_CURVE_PIECE)
      lastX = tail.x + ox
      lastY = tail.y + oy
      out = collections::append(out, __canvas_cvPin(__canvas_cvReduce(__canvas_curveHomog(curve, ox, oy), eps), hx, hy, lastX, lastY))
    END IF
  NEXT
  IF closed AND have THEN
    IF lastX <> firstX OR lastY <> firstY THEN
      __CANVAS_CURVE_MARKS = collections::append(__CANVAS_CURVE_MARKS, base + len(out) / __CANVAS_CURVE_PIECE)
      out = __canvas_cvLine(out, lastX, lastY, firstX, firstY)
    END IF
  END IF
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("canvas_curveMath", CURVE_MATH));
    pkg.add_helper(RegistryHelper::always("canvas_curveChain", CURVE_CHAIN));
}

#[cfg(test)]
mod tests {
    use super::{CURVE_MATH, PIECE_FLOATS};

    /// The piece width is written twice: in the MFBASIC every reader of a piece list
    /// uses, and here for the Rust side. They are one number.
    #[test]
    fn canvas_curve_piece_width_is_pinned() {
        assert!(CURVE_MATH.contains(&format!(
            "LET __CANVAS_CURVE_PIECE AS Integer = {PIECE_FLOATS}\n"
        )));
    }

    /// `CURVE_MATH` is lifted out of this file and run on its own by
    /// `tests/canvas/rt_canvas_curves.rs`, so it may name nothing of the canvas
    /// package but its own functions and constants.
    #[test]
    fn canvas_curve_math_names_nothing_outside_itself() {
        for (at, _) in CURVE_MATH.match_indices("__canvas_") {
            let name: String = CURVE_MATH[at..]
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            assert!(
                name.starts_with("__canvas_cv"),
                "CURVE_MATH calls `{name}`, which the standalone harness does not have"
            );
        }
        for (at, _) in CURVE_MATH.match_indices("__CANVAS_") {
            let name: String = CURVE_MATH[at..]
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            assert!(
                name.starts_with("__CANVAS_CURVE_"),
                "CURVE_MATH reads `{name}`, which the standalone harness does not have"
            );
        }
    }
}
