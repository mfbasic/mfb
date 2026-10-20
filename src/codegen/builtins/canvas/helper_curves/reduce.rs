//! plan-176-R: the `helper_curves` MFBASIC helper text — curve reduction and chaining.
//! Registration lives in `builtins/canvas/helper_curves/mod.rs`.

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
