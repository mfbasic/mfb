//! plan-176-R: the `helper_curves` MFBASIC helper text — curve pieces — the base, validation
//! and nearest-point helpers. Registration lives in `builtins/canvas/helper_curves/mod.rs`.

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
