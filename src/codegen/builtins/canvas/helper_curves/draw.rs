//! plan-176-R: the `helper_curves` MFBASIC helper text — drawing a curve (`CURVE_DRAW`).
//! Registration lives in `builtins/canvas/helper_curves/mod.rs`.

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
