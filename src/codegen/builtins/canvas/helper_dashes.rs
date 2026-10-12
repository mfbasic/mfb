//! Stroke dashes (plan-163-B): `Paint.dashes` and `Paint.dashOffset`.
//!
//! A dashed stroke is the solid stroke cut along its outline. Every point of the outline
//! has a **perimeter position** `s`, the length of outline from where it starts, and the
//! pattern decides which `s` are drawn: `dashes` alternates on and off lengths, read from
//! `s + dashOffset`. Where `s = 0` is, per kind: a Line at its first point, an Arc at
//! `startAngle`, a PolyLine at its first point, each Polygon ring at its first point
//! (each ring restarts the pattern), a Rectangle at its top-left corner going clockwise
//! (towards +X under Y-down), a rounded Rectangle at the start of its top straight edge, a
//! Circle or Ellipse at angle 0 (+X in its own frame) going the way angles increase.
//!
//! **An outline of edges is cut edge by edge.** A pixel's position on an edge is its
//! projection onto the edge's line, `u = s0 + (p - a)·d / |d|`, and the edge's piece of
//! the stroke is its band `|perp| - half` cut along the edge by the dashes that lie on
//! it: `max(band, along)`, where `along` is the signed distance along the edge from `u`
//! to the nearest dash clipped to the edge's own `[s0, s1]` (negative inside). The stroke
//! is the `min` over edges, unioned with the corner piece (`helper_joins.rs`) at every
//! vertex a dash runs through. Taking the pixel's `s` from the NEAREST outline point
//! instead is the obvious design and is wrong on a corner's inner side: a pixel there is
//! nearest the second edge, while the first edge's band — still inside the dash — is
//! what covers it in SVG. Cutting each edge by its own position is exactly the SVG
//! construction (every dash is a sub-path, stroked with butt ends, joins and caps). A
//! Round join paint carries corner pieces too when it is dashed (discs), because the
//! round corner of an undashed stroke must disappear where the dash pattern is off.
//!
//! **A curve is cut at its nearest point.** A Circle, Arc, Ellipse or rounded Rectangle has
//! no corner, so its `s` is the perimeter position of the pixel's nearest outline point
//! and `along` is measured in outline length there: a dash end is cut along the
//! outline's normal, to within the band's curvature (`half / radius` of the antialiased
//! pixel's width).
//!
//! **Dash ends take the cap.** `Butt` cuts at the end; `Square` extends each dash by
//! `half` past it; `Round` unions a disc of `half` about the end point. An end is a TRUE
//! end — capped — only where the dash stops; where it runs on past an edge's end into the
//! next edge, the edge is cut butt and the corner piece joins the two.
//!
//! **A closed outline's seam.** A ring, rectangle, circle, ellipse or rounded rectangle
//! starts and ends at one point. A dash on at both sides of it is one dash; one on at one
//! side only ends there. `__canvas_dashLocate` reads the pattern on an infinite line and
//! then moves its boundaries to the seam accordingly, so no false end and no missing one
//! appears at `s = 0`. An open path's ends (a Line, Arc or PolyLine) always end a dash.
//!
//! Nothing here uses a transcendental: angles come from `__canvas_atan2`, a series in
//! `+ - * / sqrt`, and points from the deterministic `__canvas_cos`/`__canvas_sin`.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

/// The build side: the normalized pattern, the dash tail and the header slots.
///
/// The dash tail sits between the join tail and the gradient stops: `[n, period, c1, ..,
/// cn]` — the entry count, the pattern's period, and each entry's END along the period
/// (`c1 = d1`, `c2 = d1 + d2`, ...) — followed by a kind's own perimeter data:
///
/// * an outline of edges (a polygon, polyline, line, rectangle, picture, stroked text):
///   three floats per edge — its start and end position and its ring's length, negative
///   for an open path;
/// * an ellipse: its cumulative arc length at `__CANVAS_DASH_ELLIPSE_STEPS + 1` evenly
///   spaced parametric angles from 0 to 2π;
/// * an arc: its start angle and its sweep, `endAngle - startAngle` taken into [0, 2π]
///   — the GPU item block carries the sweep's end points, not its angles;
/// * a circle and a rounded rectangle: nothing — their perimeter is a formula of the
///   header.
///
/// `__CANVAS_GEO_DASH_COUNT` is the tail's length and `__CANVAS_GEO_DASH_PHASE` the offset
/// wrapped into one period. A dashed item also writes its cap into `__CANVAS_GEO_CAP`,
/// which only the open kinds wrote before, since a dash end takes it on every kind.
#[rustfmt::skip]
const DASH_BUILD: &str =
r#"' plan-163-B: how finely an ellipse's arc length is tabulated -- measured in
' `rt_canvas_rasteriser.rs` (`an_ellipse_dash_table_is_within_a_tenth_of_a_pixel`: 0.030 px worst at 256 steps, 0.119 at 128).
LET __CANVAS_DASH_ELLIPSE_STEPS AS Integer = 256

' A paint's pattern ready to draw: its lengths, an odd number of them repeated once to
' make the count even; [] for a solid stroke -- no lengths, any negative (or NaN) one, or
' a sum that is not positive -- and for a stroke that draws nothing (`half <= 0`).
FUNC __canvas_dashPattern(paint AS Paint, half AS Float) AS List OF Float
  LET n AS Integer = len(paint.dashes)
  IF n = 0 OR NOT (half > 0.0) THEN
    RETURN []
  END IF
  MUT total AS Float = 0.0
  FOR EACH d IN paint.dashes
    IF NOT (d >= 0.0) THEN
      RETURN []
    END IF
    total = total + d
  NEXT
  IF NOT (total > 0.0) THEN
    RETURN []
  END IF
  IF n MOD 2 = 1 THEN
    RETURN collections::append(paint.dashes, paint.dashes)
  END IF
  RETURN paint.dashes
END FUNC

' How far past its outline a stroke of half-width `half` reaches at a dash end: a Square
' dash end's corners are `half * sqrt(2)` from the outline point it ends at; anything
' else stays within `half`.
FUNC __canvas_dashCapReach(half AS Float, paint AS Paint) AS Float
  IF paint.cap = CapStyle.Square AND len(__canvas_dashPattern(paint, half)) > 0 THEN
    RETURN half * 1.4142135623730951
  END IF
  RETURN half
END FUNC

' `dashOffset` wrapped into [0, period). An offset more than 10^15 periods away has no
' fraction of a period left to keep, and wraps to 0.
FUNC __canvas_dashPhase(offset AS Float, period AS Float) AS Float
  LET turns AS Float = offset / period
  IF NOT (__canvas_absF(turns) < 1000000000000000.0) THEN
    RETURN 0.0
  END IF
  LET p AS Float = offset - toFloat(math::floor(turns)) * period
  IF p < 0.0 OR p >= period THEN
    RETURN 0.0
  END IF
  RETURN p
END FUNC

' An outline of edges' perimeter data: for each edge, where its ring's length before it
' ends, where it ends itself, and its ring's whole length -- negative for an open path.
' A zero-length edge adds no length.
FUNC __canvas_dashEdgeData(edges AS List OF Float, starts AS List OF Integer, closed AS Boolean) AS List OF Float
  MUT out AS List OF Float = []
  LET total AS Integer = len(edges) / 5
  LET rings AS Integer = len(starts)
  MUT r AS Integer = 0
  WHILE r < rings
    LET first AS Integer = collections::getOr(starts, r, 0)
    MUT stop AS Integer = total
    IF r + 1 < rings THEN
      stop = collections::getOr(starts, r + 1, total)
    END IF
    MUT ends AS List OF Float = []
    MUT at AS Float = 0.0
    MUT i AS Integer = first
    WHILE i < stop
      LET dx AS Float = collections::getOr(edges, i * 5 + 2, 0.0)
      LET dy AS Float = collections::getOr(edges, i * 5 + 3, 0.0)
      at = at + math::sqrt(dx * dx + dy * dy)
      ends = collections::append(ends, at)
      i = i + 1
    END WHILE
    MUT length AS Float = at
    IF NOT closed THEN
      length = 0.0 - at
    END IF
    MUT before AS Float = 0.0
    FOR EACH e IN ends
      out = collections::append(out, [before, e, length])
      before = e
    NEXT
    r = r + 1
  END WHILE
  RETURN out
END FUNC

' An ellipse's cumulative arc length at STEPS + 1 evenly spaced parametric angles from 0
' to 2pi, each step Simpson's rule over 8 sub-steps of `sqrt(rx^2 sin^2 + ry^2 cos^2)`.
FUNC __canvas_dashEllipseData(rx AS Float, ry AS Float) AS List OF Float
  LET steps AS Integer = __CANVAS_DASH_ELLIPSE_STEPS
  LET dt AS Float = 6.283185307179586 / toFloat(steps)
  MUT out AS List OF Float = [0.0]
  MUT at AS Float = 0.0
  MUT k AS Integer = 0
  WHILE k < steps
    LET t0 AS Float = toFloat(k) * dt
    MUT sum AS Float = 0.0
    MUT j AS Integer = 0
    WHILE j <= 8
      LET t AS Float = t0 + toFloat(j) * dt / 8.0
      LET sn AS Float = __canvas_sin(t)
      LET cs AS Float = __canvas_cos(t)
      LET f AS Float = math::sqrt(rx * rx * sn * sn + ry * ry * cs * cs)
      MUT w AS Float = 2.0
      IF j = 0 OR j = 8 THEN
        w = 1.0
      ELSEIF j MOD 2 = 1 THEN
        w = 4.0
      END IF
      sum = sum + w * f
      j = j + 1
    END WHILE
    at = at + sum * dt / 24.0
    out = collections::append(out, at)
    k = k + 1
  END WHILE
  RETURN out
END FUNC

' The dash tail of a pattern with a kind's perimeter data: [n, period, c1, .., cn] and
' the data; nothing for a solid stroke.
FUNC __canvas_dashTail(pattern AS List OF Float, data AS List OF Float) AS List OF Float
  IF len(pattern) = 0 THEN
    RETURN []
  END IF
  MUT out AS List OF Float = [toFloat(len(pattern)), 0.0]
  MUT at AS Float = 0.0
  FOR EACH d IN pattern
    at = at + d
    out = collections::append(out, at)
  NEXT
  out = collections::set(out, 1, at)
  RETURN collections::append(out, data)
END FUNC

' Whether a built header draws anything -- a dash tail rides only on one that does.
FUNC __canvas_dashable(header AS List OF Float) AS Boolean
  RETURN toInt(collections::getOr(header, 0, 0.0)) <> __CANVAS_GEO_NONE
END FUNC

' The dash tail of an outline of edges whose rings start at `starts` (a polygon, polyline
' or stroked text), or nothing.
FUNC __canvas_edgeDashes(header AS List OF Float, paint AS Paint, half AS Float, edges AS List OF Float, starts AS List OF Integer, closed AS Boolean) AS List OF Float
  LET pattern AS List OF Float = __canvas_dashPattern(paint, half)
  IF len(pattern) = 0 OR NOT __canvas_dashable(header) THEN
    RETURN []
  END IF
  RETURN __canvas_dashTail(pattern, __canvas_dashEdgeData(edges, starts, closed))
END FUNC

' A Line's dash tail: its one edge, from the header's two points.
FUNC __canvas_lineDashes(header AS List OF Float, l AS Line) AS List OF Float
  LET pattern AS List OF Float = __canvas_dashPattern(l.paint, __canvas_strokeHalf(l.paint))
  IF len(pattern) = 0 OR NOT __canvas_dashable(header) THEN
    RETURN []
  END IF
  LET x1 AS Float = collections::getOr(header, 2, 0.0)
  LET y1 AS Float = collections::getOr(header, 3, 0.0)
  LET edge AS List OF Float = [x1, y1, collections::getOr(header, 4, 0.0) - x1, collections::getOr(header, 5, 0.0) - y1, 0.0]
  RETURN __canvas_dashTail(pattern, __canvas_dashEdgeData(edge, [0], FALSE))
END FUNC

' A rectangle's, rounded rectangle's or picture's dash tail: a rectangle's four sides
' (`__canvas_rectSides`); a rounded rectangle's perimeter is a formula and needs none.
FUNC __canvas_rectDashes(header AS List OF Float, paint AS Paint) AS List OF Float
  LET pattern AS List OF Float = __canvas_dashPattern(paint, __canvas_strokeHalf(paint))
  IF len(pattern) = 0 OR NOT __canvas_dashable(header) THEN
    RETURN []
  END IF
  IF collections::getOr(header, 6, 0.0) <> 0.0 THEN
    RETURN __canvas_dashTail(pattern, [])
  END IF
  RETURN __canvas_dashTail(pattern, __canvas_dashEdgeData(__canvas_rectSides(header), [0], TRUE))
END FUNC

' A circle's dash tail: the pattern alone.
FUNC __canvas_roundDashes(header AS List OF Float, paint AS Paint) AS List OF Float
  LET pattern AS List OF Float = __canvas_dashPattern(paint, __canvas_strokeHalf(paint))
  IF len(pattern) = 0 OR NOT __canvas_dashable(header) THEN
    RETURN []
  END IF
  RETURN __canvas_dashTail(pattern, [])
END FUNC

' An arc's dash tail: the pattern, then its start angle and its sweep -- `endAngle -
' startAngle` taken into [0, 2pi], a full turn or more being 2pi.
FUNC __canvas_arcDashes(header AS List OF Float, a AS Arc) AS List OF Float
  LET pattern AS List OF Float = __canvas_dashPattern(a.paint, __canvas_strokeHalf(a.paint))
  IF len(pattern) = 0 OR NOT __canvas_dashable(header) THEN
    RETURN []
  END IF
  MUT sweep AS Float = a.endAngle - a.startAngle
  IF sweep >= 6.283185307179586 THEN
    sweep = 6.283185307179586
  ELSEIF sweep < 0.0 THEN
    sweep = sweep - toFloat(math::floor(sweep / 6.283185307179586)) * 6.283185307179586
  END IF
  RETURN __canvas_dashTail(pattern, [a.startAngle, sweep])
END FUNC

' An ellipse's dash tail: the pattern and its arc-length table.
FUNC __canvas_ellipseDashes(header AS List OF Float, e AS Ellipse) AS List OF Float
  LET pattern AS List OF Float = __canvas_dashPattern(e.paint, __canvas_strokeHalf(e.paint))
  IF len(pattern) = 0 OR NOT __canvas_dashable(header) THEN
    RETURN []
  END IF
  RETURN __canvas_dashTail(pattern, __canvas_dashEllipseData(e.radiusX, e.radiusY))
END FUNC

' A header with its dash tail counted: `__CANVAS_GEO_DASH_COUNT`, the phase, the cap, and
' the record length in slot 1. A solid stroke leaves the header untouched.
FUNC __canvas_withDashes(header AS List OF Float, dashes AS List OF Float, paint AS Paint) AS List OF Float
  IF len(dashes) = 0 THEN
    RETURN header
  END IF
  MUT out AS List OF Float = header
  out = collections::set(out, __CANVAS_GEO_DASH_COUNT, toFloat(len(dashes)))
  out = collections::set(out, __CANVAS_GEO_DASH_PHASE, __canvas_dashPhase(paint.dashOffset, collections::getOr(dashes, 1, 1.0)))
  out = collections::set(out, __CANVAS_GEO_CAP, toFloat(__canvas_capTag(paint.cap)))
  out = collections::set(out, 1, collections::getOr(out, 1, 0.0) + toFloat(len(dashes)))
  RETURN out
END FUNC"#;

/// The draw side: where a pixel is in the pattern, and each kind's dashed stroke.
#[rustfmt::skip]
const DASH_DRAW: &str =
r#"' plan-163-B: `__canvas_dashLocate`'s boundaries, `__canvas_dashAlongPiece`'s true dash
' ends and `__canvas_dashCurveX`'s Y -- results kept here rather than returned in a list, so the per-pixel calls
' allocate nothing.
MUT __CANVAS_DASH_LO AS Float = 0.0
MUT __CANVAS_DASH_HI AS Float = 0.0
MUT __CANVAS_DASH_HAS_A AS Boolean = FALSE
MUT __CANVAS_DASH_END_A AS Float = 0.0
MUT __CANVAS_DASH_HAS_B AS Boolean = FALSE
MUT __CANVAS_DASH_END_B AS Float = 0.0
MUT __CANVAS_DASH_Y AS Float = 0.0

' Where a record's dash tail starts: right before its clip edges (plan-163-C), which are
' right before its gradient stops, which stay last.
FUNC __canvas_dashBase(offset AS Integer) AS Integer
  RETURN __canvas_clipEdgeBase(offset) - toInt(__canvas_geoAt(offset, __CANVAS_GEO_DASH_COUNT))
END FUNC

' plan-163-C: where a record's path-clip edges start -- right before its gradient stops,
' counted back from the end of the record as `__canvas_gradientStopBase` counts them.
FUNC __canvas_clipEdgeBase(offset AS Integer) AS Integer
  RETURN __canvas_gradientStopBase(offset) - toInt(__canvas_geoAt(offset, __CANVAS_GEO_CLIP_EDGE_COUNT)) * 5
END FUNC

' Whether pattern position `x` (the phase already added) is in a dash: entry j covers
' [c(j-1), c(j)) of the period, and the even entries are dashes.
FUNC __canvas_dashOnAt(dbase AS Integer, x AS Float) AS Boolean
  LET n AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, dbase, 0.0))
  LET period AS Float = collections::getOr(__CANVAS_GEO_DATA, dbase + 1, 1.0)
  LET w AS Float = x - toFloat(math::floor(x / period)) * period
  MUT j AS Integer = 0
  WHILE j < n
    IF w < collections::getOr(__CANVAS_GEO_DATA, dbase + 2 + j, 0.0) THEN
      RETURN j MOD 2 = 0
    END IF
    j = j + 1
  END WHILE
  RETURN TRUE
END FUNC

' Whether the pattern is in a dash just BEFORE position `x`: entry j covers (c(j-1),
' c(j)] from the left, and a position on a period's start is the previous period's end.
FUNC __canvas_dashOnBefore(dbase AS Integer, x AS Float) AS Boolean
  LET n AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, dbase, 0.0))
  LET period AS Float = collections::getOr(__CANVAS_GEO_DATA, dbase + 1, 1.0)
  MUT w AS Float = x - toFloat(math::floor(x / period)) * period
  IF w <= 0.0 THEN
    w = period
  END IF
  MUT j AS Integer = 0
  WHILE j < n
    IF w <= collections::getOr(__CANVAS_GEO_DATA, dbase + 2 + j, 0.0) THEN
      RETURN j MOD 2 = 0
    END IF
    j = j + 1
  END WHILE
  RETURN FALSE
END FUNC

' Whether outline position `s` is in a dash, and the interval of the pattern around it:
' `__CANVAS_DASH_LO`..`__CANVAS_DASH_HI`, the dash it is in or the gap between two. The
' pattern is read on an infinite line from `s + phase`. On an outline of `length` that
' is `closed`, a boundary past either end moves to the seam (a dash on at one side of it
' only ends there) or vanishes to +-1e9 (a dash on at both sides is one dash); on an open
' one, the path's ends end a dash and nothing lies past them.
FUNC __canvas_dashLocate(offset AS Integer, s AS Float, length AS Float, closed AS Boolean) AS Boolean
  LET dbase AS Integer = __canvas_dashBase(offset)
  LET phase AS Float = __canvas_geoAt(offset, __CANVAS_GEO_DASH_PHASE)
  LET n AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, dbase, 0.0))
  LET period AS Float = collections::getOr(__CANVAS_GEO_DATA, dbase + 1, 1.0)
  LET u AS Float = s + phase
  LET w AS Float = u - toFloat(math::floor(u / period)) * period
  MUT lowW AS Float = 0.0
  MUT highW AS Float = period
  MUT j AS Integer = 0
  MUT found AS Boolean = FALSE
  WHILE j < n AND NOT found
    LET c AS Float = collections::getOr(__CANVAS_GEO_DATA, dbase + 2 + j, 0.0)
    IF w < c THEN
      highW = c
      found = TRUE
    ELSE
      lowW = c
      j = j + 1
    END IF
  END WHILE
  MUT on AS Boolean = j MOD 2 = 0
  IF NOT found THEN
    ' `w` rounded up onto the period: the next period's first entry, a dash.
    on = TRUE
    lowW = period
    highW = period + collections::getOr(__CANVAS_GEO_DATA, dbase + 2, 0.0)
  END IF
  MUT lo AS Float = s - (w - lowW)
  MUT hi AS Float = s + (highW - w)
  IF closed THEN
    LET onStart AS Boolean = __canvas_dashOnAt(dbase, phase)
    LET onEnd AS Boolean = __canvas_dashOnBefore(dbase, length + phase)
    IF lo < 0.0 THEN
      IF on = onEnd THEN
        lo = 0.0 - 1000000000.0
      ELSE
        lo = 0.0
      END IF
    END IF
    IF hi > length THEN
      IF on = onStart THEN
        hi = 1000000000.0
      ELSE
        hi = length
      END IF
    END IF
  ELSE
    IF lo < 0.0 THEN
      IF on THEN
        lo = 0.0
      ELSE
        lo = 0.0 - 1000000000.0
      END IF
    END IF
    IF hi > length THEN
      IF on THEN
        hi = length
      ELSE
        hi = 1000000000.0
      END IF
    END IF
  END IF
  __CANVAS_DASH_LO = lo
  __CANVAS_DASH_HI = hi
  RETURN on
END FUNC

' The signed distance along the outline from position `u` to the dashes on the piece
' [s0, s1] of it (negative inside one), each dash extended by `ext` past a TRUE end --
' one where it stops, not where it runs on past the piece. Those ends near `u` are left
' in `__CANVAS_DASH_END_A`/`_B`, for a Round cap. `u` may lie past the piece; it is
' located at the nearest point of it.
FUNC __canvas_dashAlongPiece(offset AS Integer, u AS Float, s0 AS Float, s1 AS Float, length AS Float, closed AS Boolean, ext AS Float) AS Float
  LET at AS Float = __canvas_minF(__canvas_maxF(u, s0), s1)
  LET on AS Boolean = __canvas_dashLocate(offset, at, length, closed)
  LET lo AS Float = __CANVAS_DASH_LO
  LET hi AS Float = __CANVAS_DASH_HI
  __CANVAS_DASH_HAS_A = FALSE
  __CANVAS_DASH_HAS_B = FALSE
  IF on THEN
    MUT left AS Float = s0
    IF lo >= s0 THEN
      left = lo - ext
      __CANVAS_DASH_HAS_A = TRUE
      __CANVAS_DASH_END_A = lo
    END IF
    MUT right AS Float = s1
    IF hi <= s1 THEN
      right = hi + ext
      __CANVAS_DASH_HAS_B = TRUE
      __CANVAS_DASH_END_B = hi
    END IF
    RETURN __canvas_maxF(left - u, u - right)
  END IF
  MUT d AS Float = 1000000.0
  IF lo > s0 THEN
    d = u - lo - ext
    __CANVAS_DASH_HAS_A = TRUE
    __CANVAS_DASH_END_A = lo
  END IF
  IF hi < s1 THEN
    d = __canvas_minF(d, hi - ext - u)
    __CANVAS_DASH_HAS_B = TRUE
    __CANVAS_DASH_END_B = hi
  END IF
  RETURN d
END FUNC

' The direction of (x, y) as an angle in [0, 2pi), measured the way canvas angles are
' (from +X towards +Y), in `+ - * / sqrt` only: folded to a ratio in [0, 1], halved twice
' with `atan(z) = 2 atan(z / (1 + sqrt(1 + z^2)))` to under tan(pi/16), then the series
' to z^21, whose truncation there is under 1e-16.
FUNC __canvas_atan2(y AS Float, x AS Float) AS Float
  LET ax AS Float = __canvas_absF(x)
  LET ay AS Float = __canvas_absF(y)
  IF ax = 0.0 AND ay = 0.0 THEN
    RETURN 0.0
  END IF
  MUT z AS Float = 0.0
  MUT swapped AS Boolean = FALSE
  IF ay > ax THEN
    z = ax / ay
    swapped = TRUE
  ELSE
    z = ay / ax
  END IF
  z = z / (1.0 + math::sqrt(1.0 + z * z))
  z = z / (1.0 + math::sqrt(1.0 + z * z))
  LET w AS Float = z * z
  MUT p AS Float = 1.0 / 21.0
  p = w * p - 1.0 / 19.0
  p = w * p + 1.0 / 17.0
  p = w * p - 1.0 / 15.0
  p = w * p + 1.0 / 13.0
  p = w * p - 1.0 / 11.0
  p = w * p + 1.0 / 9.0
  p = w * p - 1.0 / 7.0
  p = w * p + 1.0 / 5.0
  p = w * p - 1.0 / 3.0
  p = w * p + 1.0
  MUT a AS Float = 4.0 * z * p
  IF swapped THEN
    a = 1.5707963267948966 - a
  END IF
  IF x < 0.0 THEN
    a = 3.141592653589793 - a
  END IF
  IF y < 0.0 THEN
    a = 6.283185307179586 - a
  END IF
  IF a >= 6.283185307179586 THEN
    RETURN 0.0
  END IF
  RETURN a
END FUNC

' The parametric angle t, in [0, 2pi), of the point (rx cos t, ry sin t) of an ellipse's
' own frame nearest (px, py): `__canvas_ellipseDistance`'s bisection, unfolded from its
' quadrant.
FUNC __canvas_ellipseAngle(px AS Float, py AS Float, cx AS Float, cy AS Float, rx AS Float, ry AS Float, ca AS Float, sa AS Float) AS Float
  LET dx AS Float = px - cx
  LET dy AS Float = py - cy
  LET rxp AS Float = dx * ca + dy * sa
  LET ryp AS Float = 0.0 - dx * sa + dy * ca
  IF rx = ry THEN
    RETURN __canvas_atan2(ryp, rxp)
  END IF
  LET qx AS Float = __canvas_absF(rxp)
  LET qy AS Float = __canvas_absF(ryp)
  MUT c0 AS Float = 1.0
  MUT s0 AS Float = 0.0
  MUT c1 AS Float = 0.0
  MUT s1 AS Float = 1.0
  MUT cm AS Float = 1.0
  MUT sm AS Float = 0.0
  MUT k AS Integer = 0
  WHILE k < 24
    LET cs AS Float = c0 + c1
    LET ss AS Float = s0 + s1
    LET nn AS Float = math::sqrt(cs * cs + ss * ss)
    cm = cs / nn
    sm = ss / nn
    LET g AS Float = (qx - rx * cm) * (0.0 - rx * sm) + (qy - ry * sm) * (ry * cm)
    IF g > 0.0 THEN
      c0 = cm
      s0 = sm
    ELSE
      c1 = cm
      s1 = sm
    END IF
    k = k + 1
  END WHILE
  IF rxp < 0.0 THEN
    cm = 0.0 - cm
  END IF
  IF ryp < 0.0 THEN
    sm = 0.0 - sm
  END IF
  RETURN __canvas_atan2(sm, cm)
END FUNC

' A rounded rectangle's perimeter position of the outline point nearest (qx, qy), taken
' from its centre: `iw`/`ih` are the half-extents of the straight sides' rectangle and `r`
' the corner radius. The outline runs from the top side's left end clockwise: top, the
' top-right quarter, right, bottom-right, bottom, bottom-left, left, top-left.
FUNC __canvas_rrectPosition(qx AS Float, qy AS Float, iw AS Float, ih AS Float, r AS Float) AS Float
  LET q AS Float = r * 1.5707963267948966
  LET ax AS Float = __canvas_absF(qx) - iw
  LET ay AS Float = __canvas_absF(qy) - ih
  IF ax > 0.0 AND ay > 0.0 THEN
    IF qx > 0.0 AND qy < 0.0 THEN
      LET a AS Float = __canvas_atan2(qy + ih, qx - iw)
      RETURN 2.0 * iw + r * __canvas_minF(__canvas_maxF(a - 4.71238898038469, 0.0), 1.5707963267948966)
    END IF
    IF qx > 0.0 THEN
      LET a AS Float = __canvas_atan2(qy - ih, qx - iw)
      RETURN 2.0 * iw + q + 2.0 * ih + r * __canvas_minF(a, 1.5707963267948966)
    END IF
    IF qy > 0.0 THEN
      LET a AS Float = __canvas_atan2(qy - ih, qx + iw)
      RETURN 4.0 * iw + 2.0 * q + 2.0 * ih + r * __canvas_minF(__canvas_maxF(a - 1.5707963267948966, 0.0), 1.5707963267948966)
    END IF
    LET a AS Float = __canvas_atan2(qy + ih, qx + iw)
    RETURN 4.0 * iw + 3.0 * q + 4.0 * ih + r * __canvas_minF(__canvas_maxF(a - 3.141592653589793, 0.0), 1.5707963267948966)
  END IF
  IF ay >= ax THEN
    IF qy < 0.0 THEN
      RETURN qx + iw
    END IF
    RETURN 2.0 * iw + 2.0 * q + 2.0 * ih + (iw - qx)
  END IF
  IF qx > 0.0 THEN
    RETURN 2.0 * iw + q + (qy + ih)
  END IF
  RETURN 4.0 * iw + 3.0 * q + 2.0 * ih + (ih - qy)
END FUNC

' The outline point at perimeter position `s` of a curve kind -- a circle, arc, ellipse
' or rounded rectangle: its X, with its Y left in `__CANVAS_DASH_Y`.
FUNC __canvas_dashCurveX(kind AS Integer, offset AS Integer, s AS Float) AS Float
  LET cx AS Float = __canvas_geoAt(offset, 2)
  LET cy AS Float = __canvas_geoAt(offset, 3)
  IF kind = __CANVAS_KIND_CIRCLE OR kind = __CANVAS_GEO_ARC THEN
    LET r AS Float = __canvas_geoAt(offset, 4)
    MUT a AS Float = s / r
    IF kind = __CANVAS_GEO_ARC THEN
      LET dbase AS Integer = __canvas_dashBase(offset)
      a = collections::getOr(__CANVAS_GEO_DATA, dbase + 2 + toInt(collections::getOr(__CANVAS_GEO_DATA, dbase, 0.0)), 0.0) + a
    END IF
    __CANVAS_DASH_Y = cy + r * __canvas_sin(a)
    RETURN cx + r * __canvas_cos(a)
  END IF
  IF kind = __CANVAS_GEO_ELLIPSE THEN
    LET rx AS Float = __canvas_geoAt(offset, 4)
    LET ry AS Float = __canvas_geoAt(offset, 5)
    LET ca AS Float = __canvas_geoAt(offset, __CANVAS_GEO_ELLIPSE_COS)
    LET sa AS Float = __canvas_geoAt(offset, __CANVAS_GEO_ELLIPSE_SIN)
    LET dbase AS Integer = __canvas_dashBase(offset)
    LET table AS Integer = dbase + 2 + toInt(collections::getOr(__CANVAS_GEO_DATA, dbase, 0.0))
    MUT lo AS Integer = 0
    MUT hi AS Integer = __CANVAS_DASH_ELLIPSE_STEPS
    WHILE hi - lo > 1
      LET mid AS Integer = (lo + hi) / 2
      IF collections::getOr(__CANVAS_GEO_DATA, table + mid, 0.0) <= s THEN
        lo = mid
      ELSE
        hi = mid
      END IF
    END WHILE
    LET t0 AS Float = collections::getOr(__CANVAS_GEO_DATA, table + lo, 0.0)
    LET t1 AS Float = collections::getOr(__CANVAS_GEO_DATA, table + lo + 1, 0.0)
    LET t AS Float = (toFloat(lo) + (s - t0) / (t1 - t0)) * (6.283185307179586 / toFloat(__CANVAS_DASH_ELLIPSE_STEPS))
    LET ox AS Float = rx * __canvas_cos(t)
    LET oy AS Float = ry * __canvas_sin(t)
    __CANVAS_DASH_Y = cy + ox * sa + oy * ca
    RETURN cx + ox * ca - oy * sa
  END IF
  LET iw AS Float = __canvas_geoAt(offset, 4)
  LET ih AS Float = __canvas_geoAt(offset, 5)
  LET r AS Float = __canvas_geoAt(offset, 6)
  LET q AS Float = r * 1.5707963267948966
  LET right AS Float = 2.0 * iw + q
  LET bottomRight AS Float = right + 2.0 * ih
  LET bottom AS Float = bottomRight + q
  LET bottomLeft AS Float = bottom + 2.0 * iw
  LET left AS Float = bottomLeft + q
  LET topLeft AS Float = left + 2.0 * ih
  MUT x AS Float = 0.0
  MUT y AS Float = 0.0
  IF s < 2.0 * iw THEN
    x = cx - iw + s
    y = cy - ih - r
  ELSEIF s < right THEN
    LET a AS Float = 4.71238898038469 + (s - 2.0 * iw) / r
    x = cx + iw + r * __canvas_cos(a)
    y = cy - ih + r * __canvas_sin(a)
  ELSEIF s < bottomRight THEN
    x = cx + iw + r
    y = cy - ih + (s - right)
  ELSEIF s < bottom THEN
    LET a AS Float = (s - bottomRight) / r
    x = cx + iw + r * __canvas_cos(a)
    y = cy + ih + r * __canvas_sin(a)
  ELSEIF s < bottomLeft THEN
    x = cx + iw - (s - bottom)
    y = cy + ih + r
  ELSEIF s < left THEN
    LET a AS Float = 1.5707963267948966 + (s - bottomLeft) / r
    x = cx - iw + r * __canvas_cos(a)
    y = cy + ih + r * __canvas_sin(a)
  ELSEIF s < topLeft THEN
    x = cx - iw - r
    y = cy + ih - (s - left)
  ELSE
    LET a AS Float = 3.141592653589793 + (s - topLeft) / r
    x = cx - iw + r * __canvas_cos(a)
    y = cy - ih + r * __canvas_sin(a)
  END IF
  __CANVAS_DASH_Y = y
  RETURN x
END FUNC

' A dashed curve's stroke distance: a circle, arc, ellipse or rounded rectangle, cut at
' the perimeter position of the pixel's nearest outline point.
FUNC __canvas_dashedCurve(kind AS Integer, offset AS Integer, px AS Float, py AS Float, half AS Float, cap AS Integer) AS Float
  LET cx AS Float = __canvas_geoAt(offset, 2)
  LET cy AS Float = __canvas_geoAt(offset, 3)
  LET dx AS Float = px - cx
  LET dy AS Float = py - cy
  MUT band AS Float = 0.0
  MUT u AS Float = 0.0
  MUT length AS Float = 0.0
  MUT closed AS Boolean = TRUE
  IF kind = __CANVAS_KIND_CIRCLE OR kind = __CANVAS_GEO_ARC THEN
    LET r AS Float = __canvas_geoAt(offset, 4)
    band = __canvas_absF(math::sqrt(dx * dx + dy * dy) - r) - half
    LET a AS Float = __canvas_atan2(dy, dx)
    IF kind = __CANVAS_KIND_CIRCLE THEN
      u = r * a
      length = 6.283185307179586 * r
    ELSE
      ' An arc's start and sweep (`__canvas_arcDashes`); a pixel outside the sweep is
      ' placed past whichever end is nearer in angle.
      LET dbase AS Integer = __canvas_dashBase(offset)
      LET data AS Integer = dbase + 2 + toInt(collections::getOr(__CANVAS_GEO_DATA, dbase, 0.0))
      LET start AS Float = collections::getOr(__CANVAS_GEO_DATA, data, 0.0)
      LET sweep AS Float = collections::getOr(__CANVAS_GEO_DATA, data + 1, 0.0)
      MUT rel AS Float = a - start
      rel = rel - toFloat(math::floor(rel / 6.283185307179586)) * 6.283185307179586
      IF rel > sweep AND rel - sweep >= 6.283185307179586 - rel THEN
        rel = rel - 6.283185307179586
      END IF
      u = r * rel
      length = r * sweep
      closed = FALSE
    END IF
  ELSEIF kind = __CANVAS_GEO_ELLIPSE THEN
    LET rx AS Float = __canvas_geoAt(offset, 4)
    LET ry AS Float = __canvas_geoAt(offset, 5)
    LET ca AS Float = __canvas_geoAt(offset, __CANVAS_GEO_ELLIPSE_COS)
    LET sa AS Float = __canvas_geoAt(offset, __CANVAS_GEO_ELLIPSE_SIN)
    band = __canvas_absF(__canvas_ellipseDistance(px, py, cx, cy, rx, ry, ca, sa)) - half
    LET steps AS Integer = __CANVAS_DASH_ELLIPSE_STEPS
    LET dbase AS Integer = __canvas_dashBase(offset)
    LET table AS Integer = dbase + 2 + toInt(collections::getOr(__CANVAS_GEO_DATA, dbase, 0.0))
    LET f AS Float = __canvas_ellipseAngle(px, py, cx, cy, rx, ry, ca, sa) / (6.283185307179586 / toFloat(steps))
    LET k AS Integer = __canvas_minI(__canvas_maxI(toInt(f), 0), steps - 1)
    LET t0 AS Float = collections::getOr(__CANVAS_GEO_DATA, table + k, 0.0)
    LET t1 AS Float = collections::getOr(__CANVAS_GEO_DATA, table + k + 1, 0.0)
    u = t0 + (t1 - t0) * (f - toFloat(k))
    length = collections::getOr(__CANVAS_GEO_DATA, table + steps, 0.0)
  ELSE
    LET iw AS Float = __canvas_geoAt(offset, 4)
    LET ih AS Float = __canvas_geoAt(offset, 5)
    LET r AS Float = __canvas_geoAt(offset, 6)
    band = __canvas_absF(__canvas_rectDistance(px, py, cx, cy, iw, ih) - r) - half
    u = __canvas_rrectPosition(dx, dy, iw, ih, r)
    length = 4.0 * iw + 4.0 * ih + 4.0 * (r * 1.5707963267948966)
  END IF
  MUT ext AS Float = 0.0
  IF cap = 2 THEN
    ext = half
  END IF
  MUT d AS Float = __canvas_maxF(band, __canvas_dashAlongPiece(offset, u, 0.0, length, length, closed, ext))
  IF cap = 1 THEN
    LET hasA AS Boolean = __CANVAS_DASH_HAS_A
    LET endA AS Float = __CANVAS_DASH_END_A
    LET hasB AS Boolean = __CANVAS_DASH_HAS_B
    LET endB AS Float = __CANVAS_DASH_END_B
    IF hasA THEN
      LET ax AS Float = __canvas_dashCurveX(kind, offset, endA)
      LET ay AS Float = __CANVAS_DASH_Y
      d = __canvas_minF(d, __canvas_segmentDistance(px, py, ax, ay, ax, ay) - half)
    END IF
    IF hasB THEN
      LET bx AS Float = __canvas_dashCurveX(kind, offset, endB)
      LET by AS Float = __CANVAS_DASH_Y
      d = __canvas_minF(d, __canvas_segmentDistance(px, py, bx, by, bx, by) - half)
    END IF
  END IF
  RETURN d
END FUNC

' The dot a dashed open path (a Line or PolyLine) with no length draws when the pattern
' is on at its start: what its undashed stroke draws there -- a disc for Round, a square
' for Square, nothing for Butt.
FUNC __canvas_dashDot(offset AS Integer, px AS Float, py AS Float, x AS Float, y AS Float, half AS Float, cap AS Integer) AS Float
  IF cap = 0 OR NOT __canvas_dashOnAt(__canvas_dashBase(offset), __canvas_geoAt(offset, __CANVAS_GEO_DASH_PHASE)) THEN
    RETURN 1000000.0
  END IF
  IF cap = 2 THEN
    RETURN __canvas_maxF(__canvas_absF(px - x), __canvas_absF(py - y)) - half
  END IF
  RETURN __canvas_segmentDistance(px, py, x, y, x, y) - half
END FUNC

' A dashed outline of edges' stroke distance -- a polygon's, polyline's, line's,
' rectangle's or picture's -- each edge's band cut by the dashes on it, unioned with the
' corner piece at every vertex a dash runs through.
FUNC __canvas_dashedEdges(kind AS Integer, offset AS Integer, px AS Float, py AS Float, half AS Float, cap AS Integer) AS Float
  LET tail AS Integer = offset + __CANVAS_GEO_HEADER
  LET dbase AS Integer = __canvas_dashBase(offset)
  LET data AS Integer = dbase + 2 + toInt(collections::getOr(__CANVAS_GEO_DATA, dbase, 0.0))
  LET phase AS Float = __canvas_geoAt(offset, __CANVAS_GEO_DASH_PHASE)
  LET joins AS Integer = toInt(__canvas_geoAt(offset, __CANVAS_GEO_JOIN_COUNT))
  LET jbase AS Integer = __canvas_joinBase(offset)
  LET p0 AS Float = __canvas_geoAt(offset, 2)
  LET p1 AS Float = __canvas_geoAt(offset, 3)
  LET p2 AS Float = __canvas_geoAt(offset, 4)
  LET p3 AS Float = __canvas_geoAt(offset, 5)
  MUT count AS Integer = toInt(__canvas_geoAt(offset, 20))
  IF kind = __CANVAS_KIND_SEGMENT THEN
    count = 1
  ELSEIF kind <> __CANVAS_GEO_POLYGON AND kind <> __CANVAS_GEO_POLYLINE THEN
    count = 4
  END IF
  MUT ext AS Float = 0.0
  IF cap = 2 THEN
    ext = half
  END IF
  MUT best AS Float = 1000000.0
  MUT live AS Boolean = FALSE
  MUT firstX AS Float = 0.0
  MUT firstY AS Float = 0.0
  MUT e AS Integer = 0
  WHILE e < count
    MUT ax AS Float = 0.0
    MUT ay AS Float = 0.0
    MUT dx AS Float = 0.0
    MUT dy AS Float = 0.0
    IF kind = __CANVAS_KIND_SEGMENT THEN
      ax = p0
      ay = p1
      dx = p2 - p0
      dy = p3 - p1
    ELSEIF kind = __CANVAS_GEO_POLYGON OR kind = __CANVAS_GEO_POLYLINE THEN
      ax = collections::getOr(__CANVAS_GEO_DATA, tail + e * 5, 0.0)
      ay = collections::getOr(__CANVAS_GEO_DATA, tail + e * 5 + 1, 0.0)
      dx = collections::getOr(__CANVAS_GEO_DATA, tail + e * 5 + 2, 0.0)
      dy = collections::getOr(__CANVAS_GEO_DATA, tail + e * 5 + 3, 0.0)
    ELSE
      ' A rectangle's sides, clockwise from the top-left, as `__canvas_rectSides` built
      ' them from these four slots.
      LET x0 AS Float = p0 - p2
      LET x1 AS Float = p0 + p2
      LET y0 AS Float = p1 - p3
      LET y1 AS Float = p1 + p3
      IF e = 0 THEN
        ax = x0
        ay = y0
        dx = x1 - x0
      ELSEIF e = 1 THEN
        ax = x1
        ay = y0
        dy = y1 - y0
      ELSEIF e = 2 THEN
        ax = x1
        ay = y1
        dx = x0 - x1
      ELSE
        ax = x0
        ay = y1
        dy = y0 - y1
      END IF
    END IF
    IF e = 0 THEN
      firstX = ax
      firstY = ay
    END IF
    LET s0 AS Float = collections::getOr(__CANVAS_GEO_DATA, data + e * 3, 0.0)
    LET s1 AS Float = collections::getOr(__CANVAS_GEO_DATA, data + e * 3 + 1, 0.0)
    LET ring AS Float = collections::getOr(__CANVAS_GEO_DATA, data + e * 3 + 2, 0.0)
    LET closed AS Boolean = ring > 0.0
    LET length AS Float = __canvas_absF(ring)
    LET len2 AS Float = dx * dx + dy * dy
    IF len2 > 0.0 THEN
      live = TRUE
      LET el AS Float = math::sqrt(len2)
      LET wx AS Float = px - ax
      LET wy AS Float = py - ay
      LET u AS Float = s0 + (wx * dx + wy * dy) / el
      LET band AS Float = __canvas_absF(wx * dy - wy * dx) / el - half
      ' A bound below the piece: its band, and how far past the edge's ends `u` is less
      ' the longest a dash end reaches. An edge that cannot beat the best is skipped.
      IF __canvas_maxF(band, __canvas_maxF(s0 - u, u - s1) - half) < best THEN
        MUT piece AS Float = __canvas_maxF(band, __canvas_dashAlongPiece(offset, u, s0, s1, length, closed, ext))
        IF cap = 1 THEN
          LET hasA AS Boolean = __CANVAS_DASH_HAS_A
          LET endA AS Float = __CANVAS_DASH_END_A
          LET hasB AS Boolean = __CANVAS_DASH_HAS_B
          LET endB AS Float = __CANVAS_DASH_END_B
          IF hasA THEN
            LET k AS Float = (endA - s0) / el
            LET ex AS Float = ax + dx * k
            LET ey AS Float = ay + dy * k
            piece = __canvas_minF(piece, __canvas_segmentDistance(px, py, ex, ey, ex, ey) - half)
          END IF
          IF hasB THEN
            LET k AS Float = (endB - s0) / el
            LET ex AS Float = ax + dx * k
            LET ey AS Float = ay + dy * k
            piece = __canvas_minF(piece, __canvas_segmentDistance(px, py, ex, ey, ex, ey) - half)
          END IF
        END IF
        best = __canvas_minF(best, piece)
      END IF
      ' The corner at this edge's start, drawn when a dash runs through it: on just
      ' before it and on at it. A closed ring's first corner is its seam, so "before" is
      ' the ring's end.
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
    IF kind = __CANVAS_KIND_SEGMENT OR kind = __CANVAS_GEO_POLYLINE THEN
      RETURN __canvas_dashDot(offset, px, py, firstX, firstY, half, cap)
    END IF
    RETURN 1000000.0
  END IF
  RETURN best
END FUNC

' A dashed item's stroke distance at (px, py): for a Line, Arc or PolyLine its whole
' distance, for every other kind its stroke band's.
FUNC __canvas_dashedStroke(kind AS Integer, offset AS Integer, px AS Float, py AS Float, half AS Float) AS Float
  LET cap AS Integer = toInt(__canvas_geoAt(offset, __CANVAS_GEO_CAP))
  IF kind = __CANVAS_KIND_CIRCLE OR kind = __CANVAS_GEO_ARC OR kind = __CANVAS_GEO_ELLIPSE THEN
    RETURN __canvas_dashedCurve(kind, offset, px, py, half, cap)
  END IF
  IF kind = __CANVAS_KIND_RECT AND __canvas_geoAt(offset, 6) <> 0.0 THEN
    RETURN __canvas_dashedCurve(kind, offset, px, py, half, cap)
  END IF
  RETURN __canvas_dashedEdges(kind, offset, px, py, half, cap)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("canvas_dashBuild", DASH_BUILD));
    pkg.add_helper(RegistryHelper::always("canvas_dashDraw", DASH_DRAW));
}
