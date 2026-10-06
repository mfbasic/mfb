//! Stroke joins (plan-163-B): `Paint.join` other than `Round` at a polygon's, a
//! polyline's and a rectangle's corners.
//!
//! A round join is what the distance field has always drawn — the stroke is `|d| -
//! half` of the outline's distance, and the distance to a corner is round — so a `Round`
//! paint builds nothing here and every such item is what it was. A `Miter` or `Bevel`
//! paint's stroke is instead the SVG construction: the union of every edge's **butt**
//! band (the rectangle `half` either side of the edge, cut square at its ends) and one
//! convex **join polygon** per corner, filling the wedge the two bands leave open on
//! the corner's outer side:
//!
//! * `Bevel` — the triangle of the vertex and the two bands' outer corners;
//! * `Miter` — that triangle plus the point where the bands' outer edges meet, unless
//!   the corner is so sharp that `1 / sin(θ/2)` exceeds `Paint.miterLimit` (θ the
//!   interior angle), which draws the bevel instead.
//!
//! The polygons are computed once, at geometry build, into a **join tail** of 9-float
//! records `n, x0, y0, x1, y1, x2, y2, x3, y3` (`n` = 3 or 4 points, the unused pair
//! zero), between the edge tail and the gradient stops; header slot
//! `__CANVAS_GEO_JOIN_COUNT` counts them. Nothing per pixel decides a corner's shape:
//! each renderer only takes a `min` over edges and records. The native builder
//! (`func_geo_build.rs`) writes the same records bit for bit.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

/// The build side: the records, their tail and the bounds they need.
///
/// A corner is the start of a live edge (one with `dx² + dy² > 0`, the test every edge
/// walk uses) whose ring has a live edge before it: an open path (a polyline) has none
/// at its two ends, which take caps; a closed ring (a polygon ring, a rectangle) also
/// joins its last live edge to its first. Zero-length edges between two live ones are
/// skipped, so a repeated point still joins the edges either side of it. Two edges in
/// the same or the opposite direction (`cross = 0`) leave no wedge: their record has 0
/// points and draws nothing. Every corner still has a record, so the count is a property
/// of which edges are live and nothing else — which is what lets the native builder size
/// a record before it computes one.
#[rustfmt::skip]
const JOIN_BUILD: &str =
r#"' plan-163-B: a paint's join as the tag the builders test -- 0 Round (no join tail),
' 1 Miter, 2 Bevel.
FUNC __canvas_joinTag(paint AS Paint) AS Integer
  IF paint.join = JoinStyle.Miter THEN
    RETURN 1
  END IF
  IF paint.join = JoinStyle.Bevel THEN
    RETURN 2
  END IF
  RETURN 0
END FUNC

' How far past its outline a stroke of half-width `half` can reach at a corner: a miter
' point is `half / sin(theta / 2)` from its vertex, at most `half * miterLimit` (a
' limit below 1.0 is 1.0); a bevel or round corner stays within `half`.
FUNC __canvas_joinReach(half AS Float, paint AS Paint) AS Float
  IF paint.join = JoinStyle.Miter THEN
    RETURN half * __canvas_maxF(paint.miterLimit, 1.0)
  END IF
  RETURN half
END FUNC

' The bounds pad of a cornered kind's stroke (a polygon, a rectangle, stroked text):
' the half-width, or the miter reach, plus the one-pixel guard. A Round or Bevel paint
' pads exactly as before joins existed.
FUNC __canvas_strokePad(paint AS Paint) AS Float
  LET half AS Float = __canvas_maxF(__canvas_strokeHalf(paint), 0.0)
  IF paint.join = JoinStyle.Miter THEN
    RETURN half * __canvas_maxF(paint.miterLimit, 1.0) + 1.0
  END IF
  RETURN half + 1.0
END FUNC

' One corner's record at the vertex (vx, vy) between an incoming edge (ax, ay) and an
' outgoing one (bx, by), appended to `out` -- 0 points when the two run in the same or
' the opposite direction, which leaves no wedge. The outer side is the one the turn opens
' towards: right of the edges for a turn with positive cross product under Y-down, left
' otherwise. A miter is kept when `1 / sin(theta / 2) <= limit`, which is
' `(1 + u1.u2) * limit^2 >= 2` without a root; its point is where the two offset edges
' meet, `V + (n1 + n2) * half / (1 + n1.n2)`.
FUNC __canvas_joinRecord(out AS List OF Float, vx AS Float, vy AS Float, ax AS Float, ay AS Float, bx AS Float, by AS Float, half AS Float, miter AS Boolean, limit AS Float) AS List OF Float
  LET l1 AS Float = math::sqrt(ax * ax + ay * ay)
  LET l2 AS Float = math::sqrt(bx * bx + by * by)
  LET u1x AS Float = ax / l1
  LET u1y AS Float = ay / l1
  LET u2x AS Float = bx / l2
  LET u2y AS Float = by / l2
  LET cross AS Float = u1x * u2y - u1y * u2x
  IF cross = 0.0 THEN
    RETURN collections::append(out, [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0])
  END IF
  MUT n1x AS Float = u1y
  MUT n1y AS Float = 0.0 - u1x
  MUT n2x AS Float = u2y
  MUT n2y AS Float = 0.0 - u2x
  IF cross < 0.0 THEN
    n1x = 0.0 - u1y
    n1y = u1x
    n2x = 0.0 - u2y
    n2y = u2x
  END IF
  LET dot AS Float = u1x * u2x + u1y * u2y
  MUT r AS List OF Float = out
  IF miter AND (1.0 + dot) * limit * limit >= 2.0 THEN
    LET k AS Float = half / (1.0 + dot)
    r = collections::append(r, 4.0)
    r = collections::append(r, vx)
    r = collections::append(r, vy)
    r = collections::append(r, vx + n1x * half)
    r = collections::append(r, vy + n1y * half)
    r = collections::append(r, vx + (n1x + n2x) * k)
    r = collections::append(r, vy + (n1y + n2y) * k)
    r = collections::append(r, vx + n2x * half)
    r = collections::append(r, vy + n2y * half)
    RETURN r
  END IF
  r = collections::append(r, 3.0)
  r = collections::append(r, vx)
  r = collections::append(r, vy)
  r = collections::append(r, vx + n1x * half)
  r = collections::append(r, vy + n1y * half)
  r = collections::append(r, vx + n2x * half)
  r = collections::append(r, vy + n2y * half)
  r = collections::append(r, 0.0)
  r = collections::append(r, 0.0)
  RETURN r
END FUNC

' The join records of an edge tail (`x0, y0, dx, dy, invLenSq` per edge) whose rings
' start at the edge indices in `starts`, or nothing for a Round paint or a stroke that
' draws nothing.
FUNC __canvas_joinTail(edges AS List OF Float, starts AS List OF Integer, closed AS Boolean, half AS Float, paint AS Paint) AS List OF Float
  MUT out AS List OF Float = []
  LET tag AS Integer = __canvas_joinTag(paint)
  IF tag = 0 OR half <= 0.0 THEN
    RETURN out
  END IF
  LET miter AS Boolean = tag = 1
  LET limit AS Float = __canvas_maxF(paint.miterLimit, 1.0)
  LET total AS Integer = len(edges) / 5
  LET rings AS Integer = len(starts)
  MUT r AS Integer = 0
  WHILE r < rings
    LET s AS Integer = collections::getOr(starts, r, 0)
    MUT e AS Integer = total
    IF r + 1 < rings THEN
      e = collections::getOr(starts, r + 1, total)
    END IF
    MUT first AS Integer = 0 - 1
    MUT prev AS Integer = 0 - 1
    MUT i AS Integer = s
    WHILE i < e
      LET dx AS Float = collections::getOr(edges, i * 5 + 2, 0.0)
      LET dy AS Float = collections::getOr(edges, i * 5 + 3, 0.0)
      IF dx * dx + dy * dy > 0.0 THEN
        IF prev >= 0 THEN
          out = __canvas_joinRecord(out, collections::getOr(edges, i * 5, 0.0), collections::getOr(edges, i * 5 + 1, 0.0), collections::getOr(edges, prev * 5 + 2, 0.0), collections::getOr(edges, prev * 5 + 3, 0.0), dx, dy, half, miter, limit)
        ELSE
          first = i
        END IF
        prev = i
      END IF
      i = i + 1
    END WHILE
    IF closed AND first >= 0 AND prev <> first THEN
      out = __canvas_joinRecord(out, collections::getOr(edges, first * 5, 0.0), collections::getOr(edges, first * 5 + 1, 0.0), collections::getOr(edges, prev * 5 + 2, 0.0), collections::getOr(edges, prev * 5 + 3, 0.0), collections::getOr(edges, first * 5 + 2, 0.0), collections::getOr(edges, first * 5 + 3, 0.0), half, miter, limit)
    END IF
    r = r + 1
  END WHILE
  RETURN out
END FUNC

' Where each contributing ring of a polygon starts in its edge tail: the running sum of
' the lengths of the rings of at least 2 points (`__canvas_polygonEdges`).
FUNC __canvas_ringStarts(rings AS List OF List OF Point) AS List OF Integer
  MUT out AS List OF Integer = []
  MUT at AS Integer = 0
  FOR EACH ring IN rings
    LET n AS Integer = len(ring)
    IF n >= 2 THEN
      out = collections::append(out, at)
      at = at + n
    END IF
  NEXT
  RETURN out
END FUNC

' A rectangle's or picture's corners, from its built header: the four sides clockwise
' from the top-left, measured from the centre and half-extents the renderers read, so
' the records and the draw-time sides agree to the bit. A rounded rectangle (radius slot
' non-zero) has no corners and builds nothing.
FUNC __canvas_rectJoins(header AS List OF Float, paint AS Paint) AS List OF Float
  LET kind AS Integer = toInt(collections::getOr(header, 0, 0.0))
  IF kind <> __CANVAS_KIND_RECT AND kind <> __CANVAS_GEO_PICTURE THEN
    RETURN []
  END IF
  IF collections::getOr(header, 6, 0.0) <> 0.0 THEN
    RETURN []
  END IF
  LET cx AS Float = collections::getOr(header, 2, 0.0)
  LET cy AS Float = collections::getOr(header, 3, 0.0)
  LET hw AS Float = collections::getOr(header, 4, 0.0)
  LET hh AS Float = collections::getOr(header, 5, 0.0)
  LET x0 AS Float = cx - hw
  LET x1 AS Float = cx + hw
  LET y0 AS Float = cy - hh
  LET y1 AS Float = cy + hh
  LET sides AS List OF Float = [x0, y0, x1 - x0, 0.0, 0.0, x1, y0, 0.0, y1 - y0, 0.0, x1, y1, x0 - x1, 0.0, 0.0, x0, y1, 0.0, y0 - y1, 0.0]
  RETURN __canvas_joinTail(sides, [0], TRUE, __canvas_strokeHalf(paint), paint)
END FUNC

' A header with its join tail counted: slot `__CANVAS_GEO_JOIN_COUNT` and the record
' length in slot 1. No records leaves the header untouched.
FUNC __canvas_withJoins(header AS List OF Float, joins AS List OF Float) AS List OF Float
  IF len(joins) = 0 THEN
    RETURN header
  END IF
  MUT out AS List OF Float = header
  out = collections::set(out, __CANVAS_GEO_JOIN_COUNT, toFloat(len(joins) / 9))
  out = collections::set(out, 1, collections::getOr(out, 1, 0.0) + toFloat(len(joins)))
  RETURN out
END FUNC"#;

/// The draw side: the join tail's distance and the joined stroke.
///
/// A join record is a convex polygon of 3 or 4 points; its signed distance is the
/// nearest of its edges, negative inside by the crossing test `__canvas_edgeDistance`
/// uses. The joined stroke is the `min` of that over every record and of each edge's
/// butt band (`__canvas_segmentDistanceButt`) — a union of the pieces SVG defines the
/// stroke as.
#[rustfmt::skip]
const JOIN_DISTANCE: &str =
r#"' plan-163-B: where a record's join tail starts -- right before its gradient stops,
' which stay last, counted back from the end of the record as
' `__canvas_gradientStopBase` counts them.
FUNC __canvas_joinBase(offset AS Integer) AS Integer
  LET joins AS Integer = toInt(__canvas_geoAt(offset, __CANVAS_GEO_JOIN_COUNT))
  RETURN __canvas_gradientStopBase(offset) - joins * 9
END FUNC

FUNC __canvas_joinPolygonDistance(base AS Integer, px AS Float, py AS Float) AS Float
  LET n AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, base, 0.0))
  MUT best AS Float = 1000000.0
  MUT inside AS Boolean = FALSE
  MUT k AS Integer = 0
  WHILE k < n
    LET j AS Integer = (k + 1) MOD n
    LET ax AS Float = collections::getOr(__CANVAS_GEO_DATA, base + 1 + k * 2, 0.0)
    LET ay AS Float = collections::getOr(__CANVAS_GEO_DATA, base + 2 + k * 2, 0.0)
    LET bx AS Float = collections::getOr(__CANVAS_GEO_DATA, base + 1 + j * 2, 0.0)
    LET by AS Float = collections::getOr(__CANVAS_GEO_DATA, base + 2 + j * 2, 0.0)
    best = __canvas_minF(best, __canvas_segmentDistance(px, py, ax, ay, bx, by))
    IF (ay > py) <> (by > py) THEN
      IF px < ax + (py - ay) / (by - ay) * (bx - ax) THEN
        inside = NOT inside
      END IF
    END IF
    k = k + 1
  END WHILE
  IF inside THEN
    RETURN 0.0 - best
  END IF
  RETURN best
END FUNC

' The nearest join record of the item at `offset`, or 1e6 when it has none.
FUNC __canvas_joinsDistance(offset AS Integer, px AS Float, py AS Float) AS Float
  LET count AS Integer = toInt(__canvas_geoAt(offset, __CANVAS_GEO_JOIN_COUNT))
  LET base AS Integer = __canvas_joinBase(offset)
  MUT best AS Float = 1000000.0
  MUT k AS Integer = 0
  WHILE k < count
    best = __canvas_minF(best, __canvas_joinPolygonDistance(base + k * 9, px, py))
    k = k + 1
  END WHILE
  RETURN best
END FUNC

' The stroke distance of a polygon, rectangle or picture with a join tail: its edges'
' butt bands -- a polygon's from its edge tail, a rectangle's four sides from its centre
' and half-extents as `__canvas_rectJoins` built them -- unioned with its join records.
FUNC __canvas_joinedStroke(kind AS Integer, offset AS Integer, tail AS Integer, edges AS Integer, px AS Float, py AS Float, p0 AS Float, p1 AS Float, p2 AS Float, p3 AS Float, half AS Float) AS Float
  MUT d AS Float = __canvas_joinsDistance(offset, px, py)
  IF kind = __CANVAS_GEO_POLYGON THEN
    MUT e AS Integer = 0
    WHILE e < edges
      LET base AS Integer = tail + e * 5
      LET ax AS Float = collections::getOr(__CANVAS_GEO_DATA, base, 0.0)
      LET ay AS Float = collections::getOr(__CANVAS_GEO_DATA, base + 1, 0.0)
      LET dx AS Float = collections::getOr(__CANVAS_GEO_DATA, base + 2, 0.0)
      LET dy AS Float = collections::getOr(__CANVAS_GEO_DATA, base + 3, 0.0)
      d = __canvas_minF(d, __canvas_segmentDistanceButt(px, py, ax, ay, ax + dx, ay + dy, half))
      e = e + 1
    END WHILE
    RETURN d
  END IF
  LET x0 AS Float = p0 - p2
  LET x1 AS Float = p0 + p2
  LET y0 AS Float = p1 - p3
  LET y1 AS Float = p1 + p3
  d = __canvas_minF(d, __canvas_segmentDistanceButt(px, py, x0, y0, x1, y0, half))
  d = __canvas_minF(d, __canvas_segmentDistanceButt(px, py, x1, y0, x1, y1, half))
  d = __canvas_minF(d, __canvas_segmentDistanceButt(px, py, x1, y1, x0, y1, half))
  RETURN __canvas_minF(d, __canvas_segmentDistanceButt(px, py, x0, y1, x0, y0, half))
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("canvas_joinBuild", JOIN_BUILD));
    pkg.add_helper(RegistryHelper::always("canvas_joinDistance", JOIN_DISTANCE));
}
