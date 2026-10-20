//! plan-176-R: the `helper_curves` MFBASIC helper text — building a curve (`CURVE_BUILD`).
//! Registration lives in `builtins/canvas/helper_curves/mod.rs`.

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
  RETURN [__canvas_boundsHeader(header, minX, minY, maxX, maxY, pad, pad, __CANVAS_BOUNDS_RIM), tail]
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
