//! plan-176-R: the `helper_picture` MFBASIC helper text — picture fit geometry
//! (`PICTURE_GEOMETRY`). Registration lives in `builtins/canvas/helper_picture/mod.rs`.

/// Fit, outline and clip intersection.
///
/// **Fit.** `__canvas_pictureFit` answers the destination rectangle `[x, y, w, h]` of
/// an image of `iw` by `ih` pixels in the box `(x0, y0)`–`(x1, y1)`: the box itself for
/// `Stretch` (0); the largest rectangle of the image's proportions inside the box,
/// centred, for `Contain` (1); the smallest one that covers the box, centred, for
/// `Cover` (2). The fit is an Integer here so this text names no canvas type.
///
/// **Outline.** `__canvas_ellipseRing` is an ellipse as a closed ring, flattened so no
/// chord is more than 0.05 pixels from the curve (the bound a rounded rectangle's
/// corners are flattened to, `helper_shapes.rs:RRECT_RING`), up to 512 points.
///
/// **Intersection.** `__canvas_clipIntersect` answers the boundary of `A ∩ B` for two
/// ring sets, each under its own fill rule, as edges in the polygon edge layout
/// (`x, y, dx, dy, 1/len²`) to be read under the even-odd rule.
///
/// It works by horizontal bands. The band boundaries are every vertex `y` of either
/// set and every `y` at which two edges cross — two of one set or one of each. Inside
/// a band no two edges cross, so on any horizontal line through it each set's interior
/// is a list of x-intervals bounded by the same edges: the band's edges sorted at its
/// mid-line, and the set's rule applied to the running winding count. The two interval
/// lists are intersected. Each surviving interval gives its left and its right bounding
/// edge, cut to the band; and where two bands meet, the horizontal boundary is the
/// symmetric difference of the upper band's intervals at its bottom and the lower
/// band's at its top.
///
/// The result is the outline of the intersection and nothing else. No edge lies in the
/// interior of the region, and that is the point: a clip's coverage is taken from the
/// distance to its nearest edge, so an interior edge — the shared side of two
/// trapezoids, say — would draw a seam. A source edge that bounds the region through
/// several bands in a row is emitted once, as one edge, not as a piece per band.
///
/// The edges are a soup, not ordered rings, but they are oriented: a left boundary runs
/// down, a right one up, a bottom one left to right and a top one right to left, so the
/// shoelace sum over them is the region's area — which is how the tests measure it.
#[rustfmt::skip]
pub(super) const PICTURE_GEOMETRY: &str =
r#"' Where an image of `iw` by `ih` goes in the box (x0, y0)-(x1, y1): `[x, y, w, h]`.
' `fit` is 0 Stretch, 1 Contain, 2 Cover.
FUNC __canvas_pictureFit(x0 AS Float, y0 AS Float, x1 AS Float, y1 AS Float, iw AS Float, ih AS Float, fit AS Integer) AS List OF Float
  LET bw AS Float = x1 - x0
  LET bh AS Float = y1 - y0
  IF fit = 0 OR iw <= 0.0 OR ih <= 0.0 THEN
    RETURN [x0, y0, bw, bh]
  END IF
  LET sx AS Float = bw / iw
  LET sy AS Float = bh / ih
  MUT s AS Float = __canvas_minF(sx, sy)
  IF fit = 2 THEN
    s = __canvas_maxF(sx, sy)
  END IF
  LET w AS Float = iw * s
  LET h AS Float = ih * s
  RETURN [x0 + (bw - w) / 2.0, y0 + (bh - h) / 2.0, w, h]
END FUNC

' An ellipse centred on (cx, cy) as one closed ring, clockwise from +X, flattened so no
' chord is further than 0.05 px from the curve: the turn per chord is sqrt(0.4 / r) for
' the larger radius r, which is the angle whose sagitta is 0.05.
FUNC __canvas_ellipseRing(cx AS Float, cy AS Float, rx AS Float, ry AS Float) AS List OF vector::Float2
  MUT ring AS List OF vector::Float2 = []
  IF rx <= 0.0 OR ry <= 0.0 THEN
    RETURN ring
  END IF
  LET big AS Float = __canvas_maxF(rx, ry)
  MUT n AS Integer = toInt(6.283185307179586 / math::sqrt(0.4 / big)) + 1
  IF n < 16 THEN
    n = 16
  END IF
  IF n > 512 THEN
    n = 512
  END IF
  MUT k AS Integer = 0
  WHILE k < n
    LET a AS Float = 6.283185307179586 * toFloat(k) / toFloat(n)
    ring = collections::append(ring, vector::Float2[x := cx + rx * __canvas_cos(a), y := cy + ry * __canvas_sin(a)])
    k = k + 1
  END WHILE
  RETURN ring
END FUNC

' The box of every point of `rings`, `[minX, minY, maxX, maxY]`; [] when they hold none.
FUNC __canvas_ringBounds(rings AS List OF List OF vector::Float2) AS List OF Float
  MUT any AS Boolean = FALSE
  MUT minX AS Float = 0.0
  MUT minY AS Float = 0.0
  MUT maxX AS Float = 0.0
  MUT maxY AS Float = 0.0
  FOR EACH ring IN rings
    FOR EACH q IN ring
      IF NOT any THEN
        minX = q.x
        maxX = q.x
        minY = q.y
        maxY = q.y
        any = TRUE
      ELSE
        minX = __canvas_minF(minX, q.x)
        maxX = __canvas_maxF(maxX, q.x)
        minY = __canvas_minF(minY, q.y)
        maxY = __canvas_maxF(maxY, q.y)
      END IF
    NEXT
  NEXT
  IF NOT any THEN
    RETURN []
  END IF
  RETURN [minX, minY, maxX, maxY]
END FUNC

' `rings` with every point sent through the affine (a b c d tx ty): x' = a x + c y + tx.
FUNC __canvas_mapRings(rings AS List OF List OF vector::Float2, a AS Float, b AS Float, c AS Float, d AS Float, tx AS Float, ty AS Float) AS List OF List OF vector::Float2
  MUT out AS List OF List OF vector::Float2 = []
  FOR EACH ring IN rings
    MUT moved AS List OF vector::Float2 = []
    FOR EACH q IN ring
      moved = collections::append(moved, vector::Float2[x := a * q.x + c * q.y + tx, y := b * q.x + d * q.y + ty])
    NEXT
    out = collections::append(out, moved)
  NEXT
  RETURN out
END FUNC

' Ascending.
FUNC __canvas_sortFloats(v AS List OF Float) AS List OF Float
  IF len(v) <= 1 THEN
    RETURN v
  END IF
  LET half AS Integer = len(v) / 2
  LET left AS List OF Float = __canvas_sortFloats(collections::mid(v, 0, half))
  LET right AS List OF Float = __canvas_sortFloats(collections::mid(v, half, len(v) - half))
  MUT out AS List OF Float = []
  MUT i AS Integer = 0
  MUT j AS Integer = 0
  WHILE i < len(left) OR j < len(right)
    IF j >= len(right) THEN
      out = collections::append(out, collections::getOr(left, i, 0.0))
      i = i + 1
    ELSEIF i >= len(left) THEN
      out = collections::append(out, collections::getOr(right, j, 0.0))
      j = j + 1
    ELSEIF collections::getOr(left, i, 0.0) <= collections::getOr(right, j, 0.0) THEN
      out = collections::append(out, collections::getOr(left, i, 0.0))
      i = i + 1
    ELSE
      out = collections::append(out, collections::getOr(right, j, 0.0))
      j = j + 1
    END IF
  END WHILE
  RETURN out
END FUNC

' The non-horizontal edges of `rings`, six floats each, appended to `out`: the upper
' end, the lower end, +1 when the ring runs down it and -1 when it runs up, and `which`
' (0 for the first ring set, 1 for the second). A horizontal edge bounds no interval of
' a horizontal line, so it has no part in the sweep.
FUNC __canvas_ciEdges(out AS List OF Float, rings AS List OF List OF vector::Float2, which AS Float) AS List OF Float
  MUT e AS List OF Float = out
  LET origin AS vector::Float2 = vector::Float2[x := 0.0, y := 0.0]
  FOR EACH ring IN rings
    LET n AS Integer = len(ring)
    IF n >= 3 THEN
      MUT i AS Integer = 0
      WHILE i < n
        LET a AS vector::Float2 = collections::getOr(ring, i, origin)
        LET b AS vector::Float2 = collections::getOr(ring, (i + 1) MOD n, origin)
        IF a.y < b.y THEN
          e = collections::append(e, [a.x, a.y, b.x, b.y, 1.0, which])
        ELSEIF a.y > b.y THEN
          e = collections::append(e, [b.x, b.y, a.x, a.y, 0.0 - 1.0, which])
        END IF
        i = i + 1
      END WHILE
    END IF
  NEXT
  RETURN e
END FUNC

' Where edge `k` of `e` crosses the horizontal line at `y`.
FUNC __canvas_ciX(e AS List OF Float, k AS Integer, y AS Float) AS Float
  LET x0 AS Float = collections::getOr(e, k * 6, 0.0)
  LET y0 AS Float = collections::getOr(e, k * 6 + 1, 0.0)
  LET x1 AS Float = collections::getOr(e, k * 6 + 2, 0.0)
  LET y1 AS Float = collections::getOr(e, k * 6 + 3, 1.0)
  RETURN x0 + (y - y0) * (x1 - x0) / (y1 - y0)
END FUNC

' One output edge in the polygon edge layout, appended to `out`; nothing for a point.
FUNC __canvas_ciEmit(out AS List OF Float, x0 AS Float, y0 AS Float, x1 AS Float, y1 AS Float) AS List OF Float
  LET dx AS Float = x1 - x0
  LET dy AS Float = y1 - y0
  LET lenSq AS Float = dx * dx + dy * dy
  IF NOT (lenSq > 0.0) THEN
    RETURN out
  END IF
  RETURN collections::append(out, [x0, y0, dx, dy, 1.0 / lenSq])
END FUNC

' The interior of ring set `which` on the mid-line `ym` of a band, as the indices of the
' edges that bound it: `[left, right, left, right, ...]`, left to right.
FUNC __canvas_ciIntervals(e AS List OF Float, which AS Float, nonZero AS Boolean, ym AS Float) AS List OF Integer
  ' The edges of this set that span the band, sorted by where they cross the mid-line.
  MUT idx AS List OF Integer = []
  MUT xs AS List OF Float = []
  LET count AS Integer = len(e) / 6
  MUT k AS Integer = 0
  WHILE k < count
    IF collections::getOr(e, k * 6 + 5, 0.0) = which THEN
      IF collections::getOr(e, k * 6 + 1, 0.0) < ym AND collections::getOr(e, k * 6 + 3, 0.0) > ym THEN
        LET x AS Float = __canvas_ciX(e, k, ym)
        ' Insertion, from the end.
        MUT at AS Integer = len(idx)
        idx = collections::append(idx, k)
        xs = collections::append(xs, x)
        WHILE at > 0 AND collections::getOr(xs, at - 1, 0.0) > x
          idx = collections::set(idx, at, collections::getOr(idx, at - 1, 0))
          xs = collections::set(xs, at, collections::getOr(xs, at - 1, 0.0))
          at = at - 1
        END WHILE
        idx = collections::set(idx, at, k)
        xs = collections::set(xs, at, x)
      END IF
    END IF
    k = k + 1
  END WHILE
  MUT out AS List OF Integer = []
  MUT winding AS Integer = 0
  MUT crossed AS Integer = 0
  MUT inside AS Boolean = FALSE
  MUT left AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < len(idx)
    LET edge AS Integer = collections::getOr(idx, i, 0)
    IF collections::getOr(e, edge * 6 + 4, 0.0) > 0.0 THEN
      winding = winding + 1
    ELSE
      winding = winding - 1
    END IF
    crossed = crossed + 1
    MUT now AS Boolean = crossed MOD 2 = 1
    IF nonZero THEN
      now = winding <> 0
    END IF
    IF now AND NOT inside THEN
      left = edge
    ELSEIF inside AND NOT now THEN
      out = collections::append(out, left)
      out = collections::append(out, edge)
    END IF
    inside = now
    i = i + 1
  END WHILE
  RETURN out
END FUNC

' Is `x` inside one of the intervals whose endpoints are `xs` (`[l, r, l, r, ...]`)?
FUNC __canvas_ciCovers(xs AS List OF Float, x AS Float) AS Boolean
  MUT i AS Integer = 0
  WHILE i + 1 < len(xs)
    IF x > collections::getOr(xs, i, 0.0) AND x < collections::getOr(xs, i + 1, 0.0) THEN
      RETURN TRUE
    END IF
    i = i + 2
  END WHILE
  RETURN FALSE
END FUNC

' The horizontal boundary at `y` between the region above it, whose intervals end at
' `upper`, and the region below it, whose intervals start at `lower`: their symmetric
' difference. Every endpoint of either list switches exactly one of the two on or off,
' so between consecutive endpoints of the two lists merged, covered-by-one alternates
' with covered-by-both-or-neither, starting with neither. An edge the region is ABOVE is
' a bottom edge and runs left to right; one it is below runs right to left.
FUNC __canvas_ciHorizontals(out AS List OF Float, upper AS List OF Float, lower AS List OF Float, y AS Float) AS List OF Float
  LET all AS List OF Float = __canvas_sortFloats(collections::append(upper, lower))
  MUT edges AS List OF Float = out
  MUT i AS Integer = 0
  WHILE i + 1 < len(all)
    LET x0 AS Float = collections::getOr(all, i, 0.0)
    LET x1 AS Float = collections::getOr(all, i + 1, 0.0)
    IF x1 - x0 > 0.000000001 THEN
      IF __canvas_ciCovers(upper, (x0 + x1) / 2.0) THEN
        edges = __canvas_ciEmit(edges, x0, y, x1, y)
      ELSE
        edges = __canvas_ciEmit(edges, x1, y, x0, y)
      END IF
    END IF
    i = i + 2
  END WHILE
  RETURN edges
END FUNC

' The boundary of (ringsA under its rule) AND (ringsB under its rule), as edges to be
' read under the even-odd rule. No edges when the two do not overlap.
FUNC __canvas_clipIntersect(ringsA AS List OF List OF vector::Float2, nonZeroA AS Boolean, ringsB AS List OF List OF vector::Float2, nonZeroB AS Boolean) AS List OF Float
  LET e AS List OF Float = __canvas_ciEdges(__canvas_ciEdges([], ringsA, 0.0), ringsB, 1.0)
  LET count AS Integer = len(e) / 6
  MUT out AS List OF Float = []
  IF count = 0 THEN
    RETURN out
  END IF

  ' The band boundaries: every vertex, and every crossing of two edges.
  MUT ys AS List OF Float = []
  MUT i AS Integer = 0
  WHILE i < count
    ys = collections::append(ys, collections::getOr(e, i * 6 + 1, 0.0))
    ys = collections::append(ys, collections::getOr(e, i * 6 + 3, 0.0))
    i = i + 1
  END WHILE
  i = 0
  WHILE i < count
    LET px AS Float = collections::getOr(e, i * 6, 0.0)
    LET py AS Float = collections::getOr(e, i * 6 + 1, 0.0)
    LET d1x AS Float = collections::getOr(e, i * 6 + 2, 0.0) - px
    LET d1y AS Float = collections::getOr(e, i * 6 + 3, 0.0) - py
    MUT j AS Integer = i + 1
    WHILE j < count
      LET qx AS Float = collections::getOr(e, j * 6, 0.0)
      LET qy AS Float = collections::getOr(e, j * 6 + 1, 0.0)
      LET d2x AS Float = collections::getOr(e, j * 6 + 2, 0.0) - qx
      LET d2y AS Float = collections::getOr(e, j * 6 + 3, 0.0) - qy
      LET denom AS Float = d1x * d2y - d1y * d2x
      IF __canvas_absF(denom) > 0.000000000001 THEN
        LET t AS Float = ((qx - px) * d2y - (qy - py) * d2x) / denom
        LET u AS Float = ((qx - px) * d1y - (qy - py) * d1x) / denom
        IF t > 0.000000001 AND t < 0.999999999 AND u > 0.000000001 AND u < 0.999999999 THEN
          ys = collections::append(ys, py + t * d1y)
        END IF
      END IF
      j = j + 1
    END WHILE
    i = i + 1
  END WHILE
  ys = __canvas_sortFloats(ys)

  ' A source edge bounding the region through consecutive bands is one output edge:
  ' its index, where its run started and where it has reached.
  MUT openEdge AS List OF Integer = []
  MUT openTop AS List OF Float = []
  MUT openBottom AS List OF Float = []
  MUT openLeft AS List OF Boolean = []
  ' The region's intervals at the bottom of the band above, `[l, r, l, r, ...]`.
  MUT above AS List OF Float = []
  MUT aboveY AS Float = collections::getOr(ys, 0, 0.0)

  MUT band AS Integer = 0
  WHILE band + 1 < len(ys)
    LET ya AS Float = collections::getOr(ys, band, 0.0)
    LET yb AS Float = collections::getOr(ys, band + 1, 0.0)
    IF yb - ya > 0.000000001 THEN
      LET ym AS Float = (ya + yb) / 2.0
      LET ia AS List OF Integer = __canvas_ciIntervals(e, 0.0, nonZeroA, ym)
      LET ib AS List OF Integer = __canvas_ciIntervals(e, 1.0, nonZeroB, ym)
      MUT top AS List OF Float = []
      MUT bottom AS List OF Float = []
      MUT a AS Integer = 0
      MUT b AS Integer = 0
      WHILE a + 1 < len(ia) AND b + 1 < len(ib)
        LET la AS Integer = collections::getOr(ia, a, 0)
        LET ra AS Integer = collections::getOr(ia, a + 1, 0)
        LET lb AS Integer = collections::getOr(ib, b, 0)
        LET rb AS Integer = collections::getOr(ib, b + 1, 0)
        MUT left AS Integer = la
        IF __canvas_ciX(e, lb, ym) > __canvas_ciX(e, la, ym) THEN
          left = lb
        END IF
        MUT right AS Integer = ra
        LET aEndsFirst AS Boolean = __canvas_ciX(e, ra, ym) <= __canvas_ciX(e, rb, ym)
        IF NOT aEndsFirst THEN
          right = rb
        END IF
        IF __canvas_ciX(e, left, ym) < __canvas_ciX(e, right, ym) THEN
          top = collections::append(top, __canvas_ciX(e, left, ya))
          top = collections::append(top, __canvas_ciX(e, right, ya))
          bottom = collections::append(bottom, __canvas_ciX(e, left, yb))
          bottom = collections::append(bottom, __canvas_ciX(e, right, yb))
          ' The two sides, each joined to its run from the band above when there is one.
          MUT side AS Integer = 0
          WHILE side < 2
            MUT edge AS Integer = left
            IF side = 1 THEN
              edge = right
            END IF
            MUT found AS Integer = 0 - 1
            MUT o AS Integer = 0
            WHILE o < len(openEdge)
              IF collections::getOr(openEdge, o, 0 - 1) = edge AND collections::getOr(openLeft, o, FALSE) = (side = 0) THEN
                found = o
              END IF
              o = o + 1
            END WHILE
            IF found >= 0 AND __canvas_absF(collections::getOr(openBottom, found, 0.0) - ya) < 0.000000001 THEN
              openBottom = collections::set(openBottom, found, yb)
            ELSE
              openEdge = collections::append(openEdge, edge)
              openTop = collections::append(openTop, ya)
              openBottom = collections::append(openBottom, yb)
              openLeft = collections::append(openLeft, side = 0)
            END IF
            side = side + 1
          END WHILE
        END IF
        IF aEndsFirst THEN
          a = a + 2
        ELSE
          b = b + 2
        END IF
      END WHILE
      out = __canvas_ciHorizontals(out, above, top, ya)
      above = bottom
      aboveY = yb
    END IF
    band = band + 1
  END WHILE
  out = __canvas_ciHorizontals(out, above, [], aboveY)

  ' The runs: a left side down, a right side up.
  MUT o AS Integer = 0
  WHILE o < len(openEdge)
    LET edge AS Integer = collections::getOr(openEdge, o, 0)
    LET y0 AS Float = collections::getOr(openTop, o, 0.0)
    LET y1 AS Float = collections::getOr(openBottom, o, 0.0)
    IF collections::getOr(openLeft, o, FALSE) THEN
      out = __canvas_ciEmit(out, __canvas_ciX(e, edge, y0), y0, __canvas_ciX(e, edge, y1), y1)
    ELSE
      out = __canvas_ciEmit(out, __canvas_ciX(e, edge, y1), y1, __canvas_ciX(e, edge, y0), y0)
    END IF
    o = o + 1
  END WHILE
  RETURN out
END FUNC"#;
