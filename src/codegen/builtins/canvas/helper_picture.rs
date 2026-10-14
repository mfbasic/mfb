//! `canvas::Picture` drawn inside a shape (plan-172-L).
//!
//! [`PICTURE_GEOMETRY`] is the arithmetic, as pure functions over numbers and point
//! lists — where an image goes inside a box under each `PictureFit`, the flattened
//! outline of a circle or an ellipse, and the intersection of two clips — so each is a
//! row of inputs and an expected output (`tests/canvas/rt_canvas_picture_fit.rs`, which
//! compiles this text and runs it).
//!
//! [`PICTURE`] is the item: what records a `Picture` draws from, how they are cached,
//! and what it is hit as. A picture is **one or two ordinary draw entries**, made where
//! the draw list is built —
//!
//! * a **plain** picture — an image stretched over a square-cornered rectangle — is one
//!   picture record, the rectangle's own re-kinded, exactly as it was before a picture
//!   had a shape;
//! * a **shaped** one is the image, a picture record over the fitted destination
//!   rectangle with the shape's outline for its clip, and then the outline, the shape
//!   itself with no fill —
//!
//! so the software renderer, both GPU backends, the damage diff and the group matrix
//! draw them with no change, and there is no new geometry kind, header slot or shader.
//!
//! Both chunks are gated on the program constructing a `Picture`
//! ([`HelperGate::WhenUsedOrConstructed`]): a canvas program with none compiles none of
//! it, and gets [`ABSENT`] for the names the unconditional source calls.

use crate::codegen::registry::{HelperGate, RegistryHelper, RegistryPackage};

/// The record types whose construction opens this chunk.
pub(super) const CONSTRUCTED: &[&str] = &["Picture", "Tile"];

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

/// The item (the module note). Everything here runs on the graphics thread or, for
/// the hit test's three, on the worker; nothing can raise.
///
/// **The cache.** A plain picture's record is under the item's own hash, as every
/// item's is. A shaped picture's two are under that hash stepped
/// (`__canvas_pictureKey`) and nothing is under the hash itself — so the native first
/// pass (`canvas::sceneResolve`) never resolves one and it always reaches
/// `__canvas_pictureAppend`. Either way a picture record is confirmed against the
/// image's current pixel block before it is used (`__canvas_pictureCached`).
///
/// **An `AutoPaint` for a shape** makes the picture an instance
/// (`helper_autopaint.rs`): always two entries, the second of which — the outline, so
/// the shape — is the one the pointer is tested against, and both of which the frame's
/// resolve swaps for the state paint's.
#[rustfmt::skip]
const PICTURE: &str =
r#"' The cache keys of a shaped picture's two records under hash `hash`: `part` is 201 for
' the image and 202 for the outline -- 211 and 212 when the shape is an `AutoPaint`,
' whose records must not be found by the quick path below -- and `shown` the state paint
' they are built under (0: the shape's own). Never the item's hash itself: only a plain
' picture's one record is cached under that.
FUNC __canvas_pictureKey(hash AS Integer, part AS Integer, shown AS Integer) AS Integer
  RETURN __canvas_hashStep(__canvas_hashStep(hash, part), shown)
END FUNC

' The record cached under `key`, or -1 when there is none -- or when it is a picture
' record built from other pixels: `setBytes` swaps the image's pixel block without
' changing the item, so this is the one cached kind a hash cannot vouch for.
FUNC __canvas_pictureCached(key AS Integer, shadow AS Integer) AS Integer
  LET slot AS Integer = canvas::geoFind(key)
  IF slot < 0 THEN
    RETURN 0 - 1
  END IF
  LET offset AS Integer = collections::getOr(__CANVAS_GEO_SLOTS, slot * 4 + 1, 0)
  LET kind AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, offset, 0.0))
  IF kind = __CANVAS_GEO_PICTURE OR kind = __CANVAS_GEO_TILE THEN
    LET hi AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, offset + __CANVAS_GEO_PICTURE_SHADOW_HI, 0.0))
    LET lo AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, offset + __CANVAS_GEO_PICTURE_SHADOW_LO, 0.0))
    IF hi * __CANVAS_GEO_PICTURE_SPLIT + lo <> shadow THEN
      RETURN 0 - 1
    END IF
  END IF
  __canvas_geoStamp(slot)
  RETURN offset
END FUNC

FUNC __canvas_pictureInsert(key AS Integer, built AS List OF List OF Float) AS Integer
  LET none AS List OF Float = []
  LET empty AS List OF Float = __canvas_emptyHeader()
  RETURN canvas::geoInsert(key, collections::getOr(built, 0, empty), collections::getOr(built, 1, none))
END FUNC

FUNC __canvas_pictureNothing() AS List OF List OF Float
  LET none AS List OF Float = []
  RETURN [__canvas_emptyHeader(), none]
END FUNC

' plan-173: one tile record. The destination is an ordinary square-cornered rectangle;
' the source rectangle is its four-float tail. The image block is confirmed on every
' cache hit because setBytes changes the pixels without changing the Tile value.
FUNC __canvas_tileRecord(tile AS Tile, shadow AS Integer, iw AS Integer, ih AS Integer) AS List OF List OF Float
  IF shadow = 0 OR iw <= 0 OR ih <= 0 OR tile.size.x <= 0.0 OR tile.size.y <= 0.0 OR tile.srcSize.x <= 0.0 OR tile.srcSize.y <= 0.0 THEN
    RETURN __canvas_pictureNothing()
  END IF
  MUT paint AS Paint
  LET base AS List OF Float = __canvas_rectHeader(tile.position.x, tile.position.y, tile.size.x, tile.size.y, 0.0, paint, __canvas_noTransform(), tile.position.x, tile.position.y)
  MUT header AS List OF Float = __canvas_pictureStamp(base, paint, shadow, iw, ih)
  IF toInt(collections::getOr(header, 0, 0.0)) <> __CANVAS_GEO_PICTURE THEN
    RETURN __canvas_pictureNothing()
  END IF
  header = collections::set(header, 0, toFloat(__CANVAS_GEO_TILE))
  header = collections::set(header, 1, toFloat(__CANVAS_GEO_HEADER + 4))
  header = collections::set(header, 7, 0.0 - 1.0)
  header = collections::set(header, 8, 255.0)
  header = collections::set(header, 9, 255.0)
  header = collections::set(header, 10, 255.0)
  header = collections::set(header, 11, 255.0)
  RETURN [header, [tile.srcPosition.x, tile.srcPosition.y, tile.srcSize.x, tile.srcSize.y]]
END FUNC

FUNC __canvas_tileGeometry(tile AS Tile, hash AS Integer) AS Integer
  LET shadow AS Integer = canvas::imageShadow(tile.image)
  LET cached AS Integer = __canvas_pictureCached(hash, shadow)
  IF cached >= 0 THEN
    RETURN cached
  END IF
  LET built AS List OF List OF Float = __canvas_tileRecord(tile, shadow, canvas::imageWidthOf(tile.image), canvas::imageHeightOf(tile.image))
  RETURN __canvas_pictureInsert(hash, built)
END FUNC

' bug-484: a rectangle's header made a picture's -- re-kinded, plus what the sampler
' needs: the image's size and its pixel block's address, split across two slots. A
' header that is not a rectangle's (a zero-area one) is handed back as it is.
'
' The shadow address being IN the header is what makes a `setBytes` visible: it swaps a
' fresh block in, so the header differs, the cache confirmation fails, and the damage
' diff sees a changed item -- although nothing in the scene the program wrote changed.
FUNC __canvas_pictureStamp(base AS List OF Float, paint AS Paint, shadow AS Integer, iw AS Integer, ih AS Integer) AS List OF Float
  IF toInt(collections::getOr(base, 0, 0.0)) <> __CANVAS_KIND_RECT THEN
    RETURN base
  END IF
  MUT out AS List OF Float = base
  out = collections::set(out, 0, toFloat(__CANVAS_GEO_PICTURE))
  ' The Correction F18 trap, the other way round: `__canvas_paintHeader` ran while slot
  ' 0 still said RECT, a kind with an interior, so it counted a gradient's stops into
  ' slot 1 -- and a picture's record carries none, because the image, not a ramp, is
  ' what fills it. Undone here so the record declares only what it holds.
  out = collections::set(out, 1, toFloat(__CANVAS_GEO_HEADER))
  out = collections::set(out, __CANVAS_GEO_GRADIENT_COUNT, 0.0)
  ' plan-163-C: a picture's fill is a TINT, whose alpha is the tint's strength, so it is
  ' not faded; the picture's own alpha is faded at draw time by T.
  out = collections::set(out, 11, toFloat(toInt(paint.fill.alpha)))
  out = collections::set(out, __CANVAS_GEO_PICTURE_TRANSPARENCY, toFloat(__canvas_transparencyStep(paint)))
  out = collections::set(out, 20, toFloat(iw))
  out = collections::set(out, 21, toFloat(ih))
  out = collections::set(out, __CANVAS_GEO_PICTURE_SHADOW_HI, toFloat(shadow / __CANVAS_GEO_PICTURE_SPLIT))
  out = collections::set(out, __CANVAS_GEO_PICTURE_SHADOW_LO, toFloat(shadow MOD __CANVAS_GEO_PICTURE_SPLIT))
  RETURN out
END FUNC

' A picture record's tail: a rectangle's joins and dashes, then -- when the header
' counts a path clip (slot 52) -- `clipEdges`, which slot 1 grows by.
FUNC __canvas_pictureFinish(stamped AS List OF Float, paint AS Paint, clipEdges AS List OF Float) AS List OF List OF Float
  LET joins AS List OF Float = __canvas_rectJoins(stamped, paint)
  LET dashes AS List OF Float = __canvas_rectDashes(stamped, paint)
  MUT tail AS List OF Float = collections::append(joins, dashes)
  MUT header AS List OF Float = __canvas_withDashes(__canvas_withJoins(stamped, joins, __canvas_joinReach(__canvas_strokeHalf(paint), paint)), dashes, paint)
  IF toInt(collections::getOr(header, __CANVAS_GEO_CLIP_EDGE_COUNT, 0.0)) > 0 THEN
    tail = collections::append(tail, clipEdges)
    header = collections::set(header, 1, collections::getOr(header, 1, 0.0) + toFloat(len(clipEdges)))
  END IF
  RETURN [header, tail]
END FUNC

' THE PLAIN PICTURE: an image stretched over a square-cornered rectangle that is not
' turned under `ClipOnly`. One record, the rectangle's own re-kinded, carrying the
' rectangle's stroke and its `paint.clip` -- the record a picture has always been.
'
' The image is read through `canvas::imageShadow` and friends, which answer 0 for a
' destroyed image instead of raising: this runs on the graphics thread, over a scene the
' program may have built before destroying what it names. Any of the three reading 0 --
' including a destroy landing between the reads -- is "no such image" and the item draws
' nothing, exactly as a zero-area rectangle does. The width and height never change and
' the shadow is never freed, so a header built from a live read stays drawable for the
' rest of the frame whatever the worker does next (`.ai/canvas-threading.md` section 7).
FUNC __canvas_picturePlain(r AS Rectangle, shadow AS Integer, iw AS Integer, ih AS Integer) AS List OF List OF Float
  IF shadow = 0 OR iw <= 0 OR ih <= 0 THEN
    RETURN __canvas_pictureNothing()
  END IF
  LET bx AS Float = __canvas_buildX(r.position, r.transform)
  LET by AS Float = __canvas_buildY(r.position, r.transform)
  LET base AS List OF Float = __canvas_rectHeader(__canvas_rectLeft(r), __canvas_rectTop(r), r.size.width, r.size.height, 0.0, r.paint, __canvas_placement(r.position, r.transform), bx, by)
  LET stamped AS List OF Float = __canvas_pictureStamp(base, r.paint, shadow, iw, ih)
  IF toInt(collections::getOr(stamped, 0, 0.0)) <> __CANVAS_GEO_PICTURE THEN
    LET none AS List OF Float = []
    RETURN [stamped, none]
  END IF
  RETURN __canvas_pictureFinish(stamped, r.paint, __canvas_polygonEdges(__canvas_placedClip(r.paint.clip, bx, by)))
END FUNC

' THE SHAPED PICTURE'S IMAGE: a picture record over the fitted destination rectangle,
' clipped to the shape's outline -- and to `paint.clip` too when the shape has one,
' the two intersected into one edge set (`__canvas_clipIntersect`), because a record
' has one clip. It carries the paint's tint, transparency and blend and no stroke, so
' no joins and no dashes: the outline is another record
' (`__canvas_pictureOutlineItem`).
'
' `outline` is the outline where the shape's own record would put it: at the shape's
' position, or about the origin when its transform is in effect and the record
' carries it. Under `ClipAndTransform` that is where the image goes too -- fitted to
' the outline's box, and carried by the same transform. Under `ClipOnly` with a
' transform in effect the outline and the clip are sent through it first, the image
' is fitted to the box of the result, and the record carries no transform: the image
' stays upright.
'
' An outline with no point, an image that covers nothing, or an intersection that
' encloses nothing is the empty record.
FUNC __canvas_pictureImageOf(outline AS List OF List OF vector::Float2, exact AS List OF Float, nonZero AS Boolean, paint AS Paint, position AS vector::Float2, t AS Transform, clipOnly AS Boolean, fit AS Integer, shadow AS Integer, iw AS Integer, ih AS Integer) AS List OF List OF Float
  MUT rings AS List OF List OF vector::Float2 = outline
  MUT clipRings AS List OF List OF vector::Float2 = __canvas_placedClip(paint.clip, __canvas_buildX(position, t), __canvas_buildY(position, t))
  MUT carried AS Transform = __canvas_placement(position, t)
  LET turned AS Boolean = __canvas_transformActive(t)
  IF clipOnly AND turned THEN
    rings = __canvas_mapRings(rings, carried.a, carried.b, carried.c, carried.d, carried.tx, carried.ty)
    clipRings = __canvas_mapRings(clipRings, carried.a, carried.b, carried.c, carried.d, carried.tx, carried.ty)
    carried = __canvas_noTransform()
  END IF
  ' The box the image is fitted to. A rectangle, a circle and an ellipse have an exact
  ' one, and it is used wherever the shape is not sent through a transform first: the
  ' flattened outline of a curve is its box only to the accuracy of this package's
  ' sine (4.6e-7), and a destination rectangle a hair off a whole number moves a texel
  ' boundary off a pixel centre it should sit exactly on -- where a renderer working
  ' in float32 and one working in float64 then pick different texels. Found by the
  ' Metal comparison of `picture_shapes_on_black`, 44 pixels along one such row.
  MUT box AS List OF Float = exact
  IF len(box) < 4 OR (clipOnly AND turned) THEN
    box = __canvas_ringBounds(rings)
  END IF
  IF len(box) < 4 THEN
    RETURN __canvas_pictureNothing()
  END IF
  LET dest AS List OF Float = __canvas_pictureFit(collections::getOr(box, 0, 0.0), collections::getOr(box, 1, 0.0), collections::getOr(box, 2, 0.0), collections::getOr(box, 3, 0.0), toFloat(iw), toFloat(ih), fit)
  LET dx AS Float = collections::getOr(dest, 0, 0.0)
  LET dy AS Float = collections::getOr(dest, 1, 0.0)
  LET dw AS Float = collections::getOr(dest, 2, 0.0)
  LET dh AS Float = collections::getOr(dest, 3, 0.0)
  ' ONE antialiased edge. A record's coverage is its rectangle's times its clip's, and
  ' where the image's edge lies along the outline -- every straight side of a
  ' rectangle under `Stretch` -- an edge pixel half covered by each would come out a
  ' quarter covered. So the clip alone draws the edge: `grown` below pushes the
  ' rectangle's own edge a pixel out, where the clip has already reached zero.
  '
  ' Under `Stretch` and `Cover` the image reaches the whole outline, so the outline is
  ' the edge. Under `Contain` it stops short on two sides, and those sides are part of
  ' the edge too: the outline is intersected with the image's rectangle. That is one
  ' intersection, and a record has room for one -- so a `Contain` picture whose paint
  ' ALSO has a clip keeps the rectangle's own edge (`grown` stays off), and is the one
  ' case with two.
  MUT edges AS List OF Float = []
  MUT rule AS Float = 0.0
  MUT grown AS Boolean = TRUE
  IF len(clipRings) > 0 THEN
    edges = __canvas_clipIntersect(rings, nonZero, clipRings, paint.clipRule = FillRule.NonZero)
    grown = fit <> 1
  ELSEIF fit = 1 THEN
    LET frame AS List OF List OF vector::Float2 = [[vector::Float2[x := dx, y := dy], vector::Float2[x := dx + dw, y := dy], vector::Float2[x := dx + dw, y := dy + dh], vector::Float2[x := dx, y := dy + dh]]]
    edges = __canvas_clipIntersect(rings, nonZero, frame, FALSE)
  ELSE
    edges = __canvas_polygonEdges(rings)
    IF nonZero THEN
      rule = 1.0
    END IF
  END IF
  IF len(edges) < 5 THEN
    RETURN __canvas_pictureNothing()
  END IF
  ' The rectangle's header under the shape's paint -- its tint, transparency and blend
  ' -- and then what the paint says of a stroke and a clip is rewritten: this record
  ' strokes nothing (slot 7; the outline is another record), is as large as the image
  ' alone, and takes the clip computed above in place of the paint's own.
  LET base AS List OF Float = __canvas_rectHeader(dx, dy, dw, dh, 0.0, paint, carried, 0.0, 0.0)
  LET stamped AS List OF Float = __canvas_pictureStamp(base, paint, shadow, iw, ih)
  IF toInt(collections::getOr(stamped, 0, 0.0)) <> __CANVAS_GEO_PICTURE THEN
    RETURN __canvas_pictureNothing()
  END IF
  MUT header AS List OF Float = __canvas_boundsHeader(stamped, dx - 2.0, dy - 2.0, dx + dw + 2.0, dy + dh + 2.0, 0.0, 0.0, 0.0)
  header = collections::set(header, 7, 0.0 - 1.0)
  ' `grown`: a rectangle's distance is its box's less its radius slot, and the image is
  ' sampled from the box alone (slots 2 to 5) -- on all three renderers -- so a radius
  ' written here, with the box left whole, moves the rectangle's edge a pixel out and
  ' the image not at all. A pixel out there takes the border texel, as an edge pixel
  ' always has.
  IF grown THEN
    header = collections::set(header, 6, 1.0)
  END IF
  ' The path clip, written as `__canvas_clipHeader` writes one (mode 2): the edge count,
  ' the rule, and the SURFACE box of the edges -- through the transform the record
  ' carries -- a pixel wider each way.
  LET mapped AS Boolean = turned AND NOT clipOnly
  MUT minX AS Float = 0.0
  MUT minY AS Float = 0.0
  MUT maxX AS Float = 0.0
  MUT maxY AS Float = 0.0
  MUT e AS Integer = 0
  WHILE e + 4 < len(edges)
    MUT k AS Integer = 0
    WHILE k < 2
      MUT x AS Float = collections::getOr(edges, e, 0.0) + toFloat(k) * collections::getOr(edges, e + 2, 0.0)
      MUT y AS Float = collections::getOr(edges, e + 1, 0.0) + toFloat(k) * collections::getOr(edges, e + 3, 0.0)
      IF mapped THEN
        LET lx AS Float = x
        x = carried.a * lx + carried.c * y + carried.tx
        y = carried.b * lx + carried.d * y + carried.ty
      END IF
      IF e = 0 AND k = 0 THEN
        minX = x
        maxX = x
        minY = y
        maxY = y
      ELSE
        minX = __canvas_minF(minX, x)
        maxX = __canvas_maxF(maxX, x)
        minY = __canvas_minF(minY, y)
        maxY = __canvas_maxF(maxY, y)
      END IF
      k = k + 1
    END WHILE
    e = e + 5
  END WHILE
  header = collections::set(header, __CANVAS_GEO_CLIP_MODE, 2.0)
  header = collections::set(header, __CANVAS_GEO_CLIP_RULE, rule)
  header = collections::set(header, __CANVAS_GEO_CLIP_EDGE_COUNT, toFloat(len(edges) / 5))
  header = collections::set(header, 22, minX - 1.0)
  header = collections::set(header, 23, minY - 1.0)
  header = collections::set(header, 24, maxX + 1.0)
  header = collections::set(header, 25, maxY + 1.0)
  header = collections::set(header, 1, collections::getOr(header, 1, 0.0) + toFloat(len(edges)))
  RETURN [header, edges]
END FUNC

' The image record of a picture on `shape`: each closed kind's outline, where its own
' record puts it. A shape with no inside -- a `Line`, an `Arc`, a `PolyLine`, a
' `PolyCurve`, a `Text` -- has no outline, and the picture draws nothing.
FUNC __canvas_pictureImage(shape AS DrawItem, clipOnly AS Boolean, fit AS Integer, shadow AS Integer, iw AS Integer, ih AS Integer) AS List OF List OF Float
  IF shadow = 0 OR iw <= 0 OR ih <= 0 THEN
    RETURN __canvas_pictureNothing()
  END IF
  MATCH shape
    CASE Rectangle(r)
      LET rings AS List OF List OF vector::Float2 = __canvas_rectRing(r)
      LET left AS Float = __canvas_rectLeft(r)
      LET top AS Float = __canvas_rectTop(r)
      LET exact AS List OF Float = [left, top, left + r.size.width, top + r.size.height]
      RETURN __canvas_pictureImageOf(rings, exact, FALSE, r.paint, r.position, r.transform, clipOnly, fit, shadow, iw, ih)
    CASE Circle(c)
      LET cx AS Float = __canvas_buildX(c.position, c.transform)
      LET cy AS Float = __canvas_buildY(c.position, c.transform)
      LET rings AS List OF List OF vector::Float2 = [__canvas_ellipseRing(cx, cy, c.radius, c.radius)]
      LET exact AS List OF Float = [cx - c.radius, cy - c.radius, cx + c.radius, cy + c.radius]
      RETURN __canvas_pictureImageOf(rings, exact, FALSE, c.paint, c.position, c.transform, clipOnly, fit, shadow, iw, ih)
    CASE Ellipse(e)
      LET cx AS Float = __canvas_buildX(e.position, e.transform)
      LET cy AS Float = __canvas_buildY(e.position, e.transform)
      LET rings AS List OF List OF vector::Float2 = [__canvas_ellipseRing(cx, cy, e.radiusX, e.radiusY)]
      LET exact AS List OF Float = [cx - e.radiusX, cy - e.radiusY, cx + e.radiusX, cy + e.radiusY]
      RETURN __canvas_pictureImageOf(rings, exact, FALSE, e.paint, e.position, e.transform, clipOnly, fit, shadow, iw, ih)
    CASE Polygon(p)
      LET rings AS List OF List OF vector::Float2 = __canvas_placedRings(p.contours, p.position, p.transform)
      ' A polygon's box is its points', which are exact as they are.
      LET exact AS List OF Float = []
      RETURN __canvas_pictureImageOf(rings, exact, p.rule = FillRule.NonZero, p.paint, p.position, p.transform, clipOnly, fit, shadow, iw, ih)
    ' plan-164-B: a curved outline, flattened for the image's clip, and the curve's true
    ' extent for its box (`helper_curves.rs:__canvas_curveRings`).
    CASE CurvePolygon(cp)
      LET rings AS List OF List OF vector::Float2 = __canvas_curveRings(cp)
      RETURN __canvas_pictureImageOf(rings, __canvas_curveExtent(), cp.rule = FillRule.NonZero, cp.paint, cp.position, cp.transform, clipOnly, fit, shadow, iw, ih)
    CASE ELSE
      RETURN __canvas_pictureNothing()
  END MATCH
  RETURN __canvas_pictureNothing()
END FUNC

' THE SHAPED PICTURE'S OUTLINE: the shape itself, with nothing for a fill -- the image
' is the fill -- and its own stroke, clip and transform. An ordinary item, so it is
' stroked, clipped and hit exactly as the shape alone would be.
'
' There is one when the stroke draws, and when `always` asks for one although it does
' not: under `Contain` the image may not reach the shape's edge, and an `AutoPaint`'s
' entry must be there for a state paint that does stroke -- in both, the outline is
' also what makes the whole shape, not only the image, what the pointer is over.
' Otherwise, and for a shape with no inside, the answer is a group that names nothing,
' whose record is the empty one.
FUNC __canvas_pictureOutlinePaint(paint AS Paint) AS Paint
  RETURN WITH paint { fill := __canvas_transparent(), gradient := __canvas_noGradient() }
END FUNC

FUNC __canvas_pictureStrokes(paint AS Paint, always AS Boolean) AS Boolean
  RETURN always OR (toInt(paint.stroke.alpha) > 0 AND paint.strokeWidth > 0.0)
END FUNC

FUNC __canvas_pictureOutlineItem(shape AS DrawItem, always AS Boolean) AS DrawItem
  MATCH shape
    CASE Rectangle(r)
      IF __canvas_pictureStrokes(r.paint, always) THEN
        LET changed AS Rectangle = WITH r { paint := __canvas_pictureOutlinePaint(r.paint) }
        LET item AS DrawItem = changed
        RETURN item
      END IF
    CASE Circle(c)
      IF __canvas_pictureStrokes(c.paint, always) THEN
        LET changed AS Circle = WITH c { paint := __canvas_pictureOutlinePaint(c.paint) }
        LET item AS DrawItem = changed
        RETURN item
      END IF
    CASE Ellipse(e)
      IF __canvas_pictureStrokes(e.paint, always) THEN
        LET changed AS Ellipse = WITH e { paint := __canvas_pictureOutlinePaint(e.paint) }
        LET item AS DrawItem = changed
        RETURN item
      END IF
    CASE Polygon(p)
      IF __canvas_pictureStrokes(p.paint, always) THEN
        LET changed AS Polygon = WITH p { paint := __canvas_pictureOutlinePaint(p.paint) }
        LET item AS DrawItem = changed
        RETURN item
      END IF
    ' plan-164-B: the outline over the image is the curve itself, drawn per pixel.
    CASE CurvePolygon(cp)
      IF __canvas_pictureStrokes(cp.paint, always) THEN
        LET changed AS CurvePolygon = WITH cp { paint := __canvas_pictureOutlinePaint(cp.paint) }
        LET item AS DrawItem = changed
        RETURN item
      END IF
    CASE ELSE
  END MATCH
  RETURN __canvas_noItem()
END FUNC

' A picture's shape as the `DrawItem` it would be on its own.
FUNC __canvas_shapeAsItem(s AS AutoShape) AS DrawItem
  MATCH s
    CASE Rectangle(r)
      LET item AS DrawItem = r
      RETURN item
    CASE Line(l)
      LET item AS DrawItem = l
      RETURN item
    CASE Polygon(p)
      LET item AS DrawItem = p
      RETURN item
    CASE Circle(c)
      LET item AS DrawItem = c
      RETURN item
    CASE Arc(arc)
      LET item AS DrawItem = arc
      RETURN item
    CASE Text(t)
      LET item AS DrawItem = t
      RETURN item
    CASE Ellipse(e)
      LET item AS DrawItem = e
      RETURN item
    CASE PolyLine(pl)
      LET item AS DrawItem = pl
      RETURN item
    CASE PolyCurve(pc)
      LET item AS DrawItem = pc
      RETURN item
    CASE CurvePolygon(cp)
      LET item AS DrawItem = cp
      RETURN item
    CASE AutoPaint(a)
      LET item AS DrawItem = a
      RETURN item
  END MATCH
  RETURN __canvas_noItem()
END FUNC

' The records a SHAPED picture on `shape` draws from, as (offset, cache key) pairs in
' draw order: the image, then the outline, each left out when its record is the empty
' one. (A plain picture is `__canvas_picturePlainOffset`'s, which its callers ask
' first.)
'
' `fixed` is a picture whose shape is an `AutoPaint`: always the two, empty or not, so
' the frame's resolve can swap either for a state paint's in place; `shown` is the
' state paint `shape` was given (0: its own), which the keys carry.
FUNC __canvas_pictureBlocksOf(shape AS DrawItem, pic AS Picture, hash AS Integer, shown AS Integer, fixed AS Boolean) AS List OF Integer
  LET shadow AS Integer = canvas::imageShadow(pic.image)
  LET iw AS Integer = canvas::imageWidthOf(pic.image)
  LET ih AS Integer = canvas::imageHeightOf(pic.image)
  LET clipOnly AS Boolean = pic.clip = PictureClip.ClipOnly
  MUT part AS Integer = 201
  IF fixed THEN
    part = 211
  END IF
  LET imageKey AS Integer = __canvas_pictureKey(hash, part, shown)
  LET outlineKey AS Integer = __canvas_pictureKey(hash, part + 1, shown)
  MUT image AS Integer = __canvas_pictureCached(imageKey, shadow)
  IF image < 0 THEN
    MUT fit AS Integer = 0
    IF pic.fit = PictureFit.Contain THEN
      fit = 1
    END IF
    IF pic.fit = PictureFit.Cover THEN
      fit = 2
    END IF
    image = __canvas_pictureInsert(imageKey, __canvas_pictureImage(shape, clipOnly, fit, shadow, iw, ih))
  END IF
  MUT outline AS Integer = __canvas_pictureCached(outlineKey, 0)
  IF outline < 0 THEN
    ' A destroyed image draws nothing, its outline included -- as a plain picture's
    ' one record draws neither.
    IF shadow = 0 OR iw <= 0 OR ih <= 0 THEN
      outline = __canvas_geometryFor(__canvas_noItem(), outlineKey)
    ELSE
      outline = __canvas_geometryFor(__canvas_pictureOutlineItem(shape, fixed OR pic.fit = PictureFit.Contain), outlineKey)
    END IF
  END IF
  MUT out AS List OF Integer = []
  IF fixed OR toInt(collections::getOr(__CANVAS_GEO_DATA, image, 0.0)) <> __CANVAS_GEO_NONE THEN
    out = collections::append(out, image)
    out = collections::append(out, imageKey)
  END IF
  IF fixed OR toInt(collections::getOr(__CANVAS_GEO_DATA, outline, 0.0)) <> __CANVAS_GEO_NONE THEN
    out = collections::append(out, outline)
    out = collections::append(out, outlineKey)
  END IF
  RETURN out
END FUNC

' The one record of a PLAIN picture, or -1 when the picture is not one: an image
' stretched over a square-cornered rectangle that is not turned under `ClipOnly`.
'
' This is what a tilemap pays per tile, so it is one cache probe -- under the item's
' own hash, where only a plain picture's record ever is -- and, on a miss, one look at
' the shape. No list is made and the shape is not turned into an item.
FUNC __canvas_picturePlainOffset(pic AS Picture, hash AS Integer) AS Integer
  LET shadow AS Integer = canvas::imageShadow(pic.image)
  LET found AS Integer = __canvas_pictureCached(hash, shadow)
  IF found >= 0 THEN
    RETURN found
  END IF
  IF pic.fit <> PictureFit.Stretch THEN
    RETURN 0 - 1
  END IF
  MATCH pic.shape
    CASE Rectangle(r)
      IF __canvas_rectRadius(r) = 0.0 AND NOT (pic.clip = PictureClip.ClipOnly AND __canvas_transformActive(r.transform)) THEN
        RETURN __canvas_pictureInsert(hash, __canvas_picturePlain(r, shadow, canvas::imageWidthOf(pic.image), canvas::imageHeightOf(pic.image)))
      END IF
    CASE ELSE
  END MATCH
  RETURN 0 - 1
END FUNC

' The (offset, key) pairs of a picture whose shape is not an `AutoPaint`: the plain
' record; or a shaped picture's two, from the cache alone when they are there -- every
' frame after the first -- so the shape is not copied out of the item at all.
FUNC __canvas_pictureQuick(pic AS Picture, hash AS Integer) AS List OF Integer
  LET plain AS Integer = __canvas_picturePlainOffset(pic, hash)
  IF plain >= 0 THEN
    RETURN [plain, hash]
  END IF
  LET shadow AS Integer = canvas::imageShadow(pic.image)
  LET imageKey AS Integer = __canvas_pictureKey(hash, 201, 0)
  LET image AS Integer = __canvas_pictureCached(imageKey, shadow)
  IF image >= 0 THEN
    LET outlineKey AS Integer = __canvas_pictureKey(hash, 202, 0)
    LET outline AS Integer = __canvas_pictureCached(outlineKey, 0)
    IF outline >= 0 THEN
      MUT out AS List OF Integer = []
      IF toInt(collections::getOr(__CANVAS_GEO_DATA, image, 0.0)) <> __CANVAS_GEO_NONE THEN
        out = collections::append(out, image)
        out = collections::append(out, imageKey)
      END IF
      IF toInt(collections::getOr(__CANVAS_GEO_DATA, outline, 0.0)) <> __CANVAS_GEO_NONE THEN
        out = collections::append(out, outline)
        out = collections::append(out, outlineKey)
      END IF
      RETURN out
    END IF
  END IF
  RETURN __canvas_pictureBlocksOf(__canvas_shapeAsItem(pic.shape), pic, hash, 0, FALSE)
END FUNC

' `__canvas_memoGroup`'s picture: the (offset, key) pairs of the picture at `index` of
' the group use `path`. A picture on an `AutoPaint` lays out the records of the paint
' the frame's resolve chose for that place, so the two walks name the same ones. (A
' picture in the scene itself is laid out from the frame's entries:
' `__canvas_sceneDrawsWalk`.)
FUNC __canvas_pictureBlocks(pic AS Picture, hash AS Integer, path AS Integer, index AS Integer) AS List OF Integer
  MATCH pic.shape
    CASE AutoPaint(a)
      LET shown AS Integer = __canvas_autoShownAt(path, index)
      RETURN __canvas_pictureBlocksOf(__canvas_autoShapeShown(a, shown), pic, hash, shown, TRUE)
    CASE ELSE
      RETURN __canvas_pictureQuick(pic, hash)
  END MATCH
  RETURN []
END FUNC

' The pictures of the frame being walked whose shape is an `AutoPaint`, in walk order:
' what `__canvas_autoResolve` rebuilds an instance's two records from. Emptied by the
' first one a frame appends.
MUT __CANVAS_PICTURE_HOSTS AS List OF Picture = []
MUT __CANVAS_PICTURE_HOSTS_FRAME AS Integer = 0

' `__canvas_autoResolve`'s picture: the two records of an instance whose `AutoPaint` is
' the shape of picture number `host`, under the paint it shows -- or nothing when
' `host` is -1, which is every instance that is an item of its own.
FUNC __canvas_pictureAutoBlocks(host AS Integer, a AS AutoPaint, hash AS Integer, shown AS Integer) AS List OF Integer
  IF host < 0 THEN
    RETURN []
  END IF
  MUT n AS Integer = 0
  FOR EACH pic IN __CANVAS_PICTURE_HOSTS
    IF n = host THEN
      RETURN __canvas_pictureBlocksOf(__canvas_autoShapeShown(a, shown), pic, hash, shown, TRUE)
    END IF
    n = n + 1
  NEXT
  RETURN []
END FUNC

' `__canvas_appendDraw`'s picture: one entry for each record it draws from. A picture
' on an `AutoPaint` is noted as an instance at this place -- its entry the outline's,
' which is the shape and so is what the pointer is over -- and is appended under the
' shape's OWN paint, which is what the hit test must read.
FUNC __canvas_pictureAppend(offsets AS List OF Integer, pic AS Picture, hash AS Integer, gdx AS Float, gdy AS Float, mat AS Integer, depth AS Integer, path AS Integer, index AS Integer) AS List OF Integer
  MUT out AS List OF Integer = offsets
  MUT blocks AS List OF Integer = []
  MATCH pic.shape
    CASE AutoPaint(a)
      blocks = __canvas_pictureBlocksOf(__canvas_autoShapeShown(a, 0), pic, hash, 0, TRUE)
      IF __CANVAS_PICTURE_HOSTS_FRAME <> __CANVAS_GEO_FRAME THEN
        __CANVAS_PICTURE_HOSTS = []
        __CANVAS_PICTURE_HOSTS_FRAME = __CANVAS_GEO_FRAME
      END IF
      __CANVAS_PICTURE_HOSTS = collections::append(__CANVAS_PICTURE_HOSTS, pic)
      __canvas_autoNote(a, hash, len(out) + 1, depth, path, index, len(__CANVAS_PICTURE_HOSTS) - 1)
    CASE ELSE
      LET plain AS Integer = __canvas_picturePlainOffset(pic, hash)
      IF plain >= 0 THEN
        out = collections::append(out, plain)
        __canvas_noteEntry(plain, hash, gdx, gdy, mat)
        RETURN out
      END IF
      blocks = __canvas_pictureQuick(pic, hash)
  END MATCH
  MUT k AS Integer = 0
  WHILE k + 1 < len(blocks)
    out = collections::append(out, collections::getOr(blocks, k, 0))
    __canvas_noteEntry(collections::getOr(blocks, k, 0), collections::getOr(blocks, k + 1, 0), gdx, gdy, mat)
    k = k + 2
  END WHILE
  RETURN out
END FUNC

' A picture's item hash when its shape is one `canvas::itemHash` does not fold -- a
' `Text`, an `AutoPaint` (`func_item_hash.rs`; every other picture is hashed there).
' The image by its backend id: a destroyed image answers 0, so the item's key changes
' and its records are rebuilt as the empty ones they now are. A `setBytes` does NOT
' change this key -- the pixel block is checked on every frame instead
' (`__canvas_pictureCached`).
FUNC __canvas_pictureHash(pic AS Picture) AS Integer
  MUT h AS Integer = __canvas_hashText(__canvas_hashStart(6), pic.id)
  MUT mode AS Integer = 0
  IF pic.clip = PictureClip.ClipAndTransform THEN
    mode = 1
  END IF
  IF pic.fit = PictureFit.Contain THEN
    mode = mode + 2
  END IF
  IF pic.fit = PictureFit.Cover THEN
    mode = mode + 4
  END IF
  h = __canvas_hashStep(h, mode)
  h = __canvas_hashStep(h, canvas::imageHandle(pic.image))
  RETURN __canvas_hashStep(h, __canvas_hashItem(__canvas_shapeAsItem(pic.shape)))
END FUNC

FUNC __canvas_pictureClosed(shape AS DrawItem) AS Boolean
  MATCH shape
    CASE Rectangle(r)
      RETURN TRUE
    CASE Circle(c)
      RETURN TRUE
    CASE Ellipse(e)
      RETURN TRUE
    CASE Polygon(p)
      RETURN TRUE
    ' plan-164-B. A `PolyCurve` is the `ELSE`: a chain encloses nothing.
    CASE CurvePolygon(cp)
      RETURN TRUE
    CASE ELSE
      RETURN FALSE
  END MATCH
  RETURN FALSE
END FUNC

' What a picture is hit as: its shape, exactly as that shape alone would be -- a
' picture is over wherever its shape is, whatever part of it the image covers. A
' picture that draws nothing (no image, or a shape with no inside) is a group that
' names nothing, which is not there to be hit.
FUNC __canvas_pictureHitItem(pic AS Picture) AS DrawItem
  IF canvas::imageWidthOf(pic.image) <= 0 OR canvas::imageHeightOf(pic.image) <= 0 THEN
    RETURN __canvas_noItem()
  END IF
  LET shape AS DrawItem = __canvas_shapeAsItem(pic.shape)
  MATCH shape
    CASE AutoPaint(a)
      IF __canvas_pictureClosed(__canvas_autoItem(a)) THEN
        RETURN shape
      END IF
    CASE ELSE
      IF __canvas_pictureClosed(shape) THEN
        RETURN shape
      END IF
  END MATCH
  RETURN __canvas_noItem()
END FUNC

' The `id` of a picture's shape -- the part of `overID` behind the picture's own. Its
' own `MATCH`es, not `__canvas_itemId`'s: that one is compiled only into a program that
' polls the mouse (`helper_hit.rs`), and this chunk into every program with a picture.
FUNC __canvas_pictureShapeId(pic AS Picture) AS String
  MATCH pic.shape
    CASE AutoPaint(a)
      MATCH a.shape
        CASE Rectangle(r)
          RETURN r.id
        CASE Line(l)
          RETURN l.id
        CASE Polygon(p)
          RETURN p.id
        CASE Circle(c)
          RETURN c.id
        CASE Arc(arc)
          RETURN arc.id
        CASE Text(t)
          RETURN t.id
        CASE Ellipse(e)
          RETURN e.id
        CASE PolyLine(pl)
          RETURN pl.id
        CASE PolyCurve(pc)
          RETURN pc.id
        CASE CurvePolygon(cp)
          RETURN cp.id
      END MATCH
    CASE Rectangle(r)
      RETURN r.id
    CASE Line(l)
      RETURN l.id
    CASE Polygon(p)
      RETURN p.id
    CASE Circle(c)
      RETURN c.id
    CASE Arc(arc)
      RETURN arc.id
    CASE Text(t)
      RETURN t.id
    CASE Ellipse(e)
      RETURN e.id
    CASE PolyLine(pl)
      RETURN pl.id
    CASE PolyCurve(pc)
      RETURN pc.id
    CASE CurvePolygon(cp)
      RETURN cp.id
  END MATCH
  RETURN ""
END FUNC

' The font a picture's shape names, when that shape is a `Text` (or an `AutoPaint`
' around one): such a picture draws nothing, but it holds the font all the same, and
' the group ownership rules (`__canvas_closeRetired`) count it.
FUNC __canvas_pictureNamesFont(pic AS Picture, handle AS Integer) AS Boolean
  MATCH pic.shape
    CASE Text(t)
      RETURN canvas::fontHandle(t.font) = handle
    CASE AutoPaint(a)
      MATCH a.shape
        CASE Text(t)
          RETURN canvas::fontHandle(t.font) = handle
        CASE ELSE
      END MATCH
    CASE ELSE
  END MATCH
  RETURN FALSE
END FUNC

FUNC __canvas_pictureCloseFont(pic AS Picture, scene AS List OF DrawItem) AS Nothing
  MATCH pic.shape
    CASE Text(t)
      LET fh AS Integer = canvas::fontHandle(t.font)
      IF fh <> 0 AND NOT __canvas_anythingNamesFont(scene, fh) THEN
        canvas::destroyFont(t.font)
      END IF
    CASE AutoPaint(a)
      MATCH a.shape
        CASE Text(t)
          LET fh AS Integer = canvas::fontHandle(t.font)
          IF fh <> 0 AND NOT __canvas_anythingNamesFont(scene, fh) THEN
            canvas::destroyFont(t.font)
          END IF
        CASE ELSE
      END MATCH
    CASE ELSE
  END MATCH
END FUNC"#;

/// What the unconditional source calls in a program that constructs no `Picture`
/// ([`HelperGate::WhenNotConstructed`]). Such a program holds none, so these cannot be
/// reached with one; they are here because `DrawItem` has the variant in every
/// program, and a `MATCH` over it must have its arm.
#[rustfmt::skip]
const ABSENT: &str =
r#"IMPORT canvas
IMPORT collections

FUNC __canvas_pictureAppend(offsets AS List OF Integer, pic AS Picture, hash AS Integer, gdx AS Float, gdy AS Float, mat AS Integer, depth AS Integer, path AS Integer, index AS Integer) AS List OF Integer
  RETURN offsets
END FUNC

FUNC __canvas_pictureBlocks(pic AS Picture, hash AS Integer, path AS Integer, index AS Integer) AS List OF Integer
  RETURN []
END FUNC

FUNC __canvas_pictureAutoBlocks(host AS Integer, a AS AutoPaint, hash AS Integer, shown AS Integer) AS List OF Integer
  RETURN []
END FUNC

FUNC __canvas_pictureHash(pic AS Picture) AS Integer
  RETURN 0
END FUNC

FUNC __canvas_pictureHitItem(pic AS Picture) AS DrawItem
  RETURN __canvas_noItem()
END FUNC

FUNC __canvas_pictureShapeId(pic AS Picture) AS String
  RETURN ""
END FUNC

FUNC __canvas_pictureNamesFont(pic AS Picture, handle AS Integer) AS Boolean
  RETURN FALSE
END FUNC

FUNC __canvas_tileGeometry(tile AS Tile, hash AS Integer) AS Integer
  RETURN 0 - 1
END FUNC

FUNC __canvas_pictureCloseFont(pic AS Picture, scene AS List OF DrawItem) AS Nothing
END FUNC"#;

/// The names the unconditional source calls: each is defined once in [`PICTURE`] and
/// once in [`ABSENT`], and exactly one of the two chunks is in any canvas program.
#[cfg(test)]
const HOOKS: [&str; 9] = [
    "__canvas_pictureAppend",
    "__canvas_pictureBlocks",
    "__canvas_pictureAutoBlocks",
    "__canvas_pictureHash",
    "__canvas_pictureHitItem",
    "__canvas_pictureShapeId",
    "__canvas_pictureNamesFont",
    "__canvas_tileGeometry",
    "__canvas_pictureCloseFont",
];

static BODY: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    format!(
        "IMPORT canvas\nIMPORT collections\nIMPORT math\nIMPORT vector\n\n{PICTURE_GEOMETRY}\n\n{PICTURE}"
    )
});

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper {
        name: "canvas_picture",
        gate: HelperGate::WhenUsedOrConstructed(&[], CONSTRUCTED),
        body: Some(BODY.as_str()),
        import_name: None,
        natively_called: false,
    });
    pkg.add_helper(RegistryHelper {
        name: "canvas_picture_absent",
        gate: HelperGate::WhenNotConstructed(CONSTRUCTED),
        body: Some(ABSENT),
        import_name: None,
        natively_called: false,
    });
}

#[cfg(test)]
mod tests {
    use super::{ABSENT, HOOKS, PICTURE};

    fn signature<'a>(source: &'a str, name: &str) -> &'a str {
        let opens = format!("FUNC {name}(");
        let at = source
            .find(&opens)
            .unwrap_or_else(|| panic!("no `{opens}` in the chunk"));
        source[at..].lines().next().expect("a signature line")
    }

    /// The two chunks are alternatives (as `helper_autopaint.rs`'s are): every hook is
    /// declared in both with one signature, so the unconditional source compiles
    /// against either.
    #[test]
    fn canvas_picture_hooks_are_declared_alike_in_both_chunks() {
        for hook in HOOKS {
            assert_eq!(signature(PICTURE, hook), signature(ABSENT, hook), "{hook}");
        }
        let declared = ABSENT.matches("\nFUNC ").count() + usize::from(ABSENT.starts_with("FUNC "));
        assert_eq!(
            declared,
            HOOKS.len(),
            "ABSENT declares a function that is not a hook"
        );
    }

    /// A shaped picture's records are never cached under the item's own hash: the
    /// native first pass would then resolve the item to ONE record and the walk that
    /// makes its two entries would never see it. Only the plain path inserts under
    /// `hash`. A Tile is also one record and deliberately uses its own item hash; the
    /// exact count remains pinned so a shaped-picture path cannot join them.
    #[test]
    fn canvas_picture_only_the_plain_record_is_cached_under_the_item_hash() {
        assert_eq!(PICTURE.matches("__canvas_pictureInsert(hash,").count(), 2);
        let tile = PICTURE
            .split("FUNC __canvas_tileGeometry")
            .nth(1)
            .and_then(|s| s.split("END FUNC").next())
            .expect("Tile geometry helper");
        assert_eq!(tile.matches("__canvas_pictureInsert(hash,").count(), 1);
        for line in PICTURE
            .lines()
            .filter(|l| l.contains("__canvas_geometryFor("))
        {
            assert!(line.trim_end().ends_with(", outlineKey)"), "{line}");
        }
    }
}
