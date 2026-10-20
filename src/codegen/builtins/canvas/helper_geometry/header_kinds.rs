//! plan-176-R: `GEO_HEADER`'s second piece — `__canvas_strokeAsFill`, the bounds header and the
//! per-kind header builders. `mod.rs` joins the two pieces with `concat!` into the one
//! registered `canvas_geoHeader` helper.

macro_rules! geo_header_kinds {
    () => {
        r#"FUNC __canvas_strokeAsFill(h AS List OF Float) AS List OF Float
  MUT out AS List OF Float = h
  out = collections::set(out, 8, collections::getOr(h, 12, 0.0))
  out = collections::set(out, 9, collections::getOr(h, 13, 0.0))
  out = collections::set(out, 10, collections::getOr(h, 14, 0.0))
  out = collections::set(out, 11, collections::getOr(h, 15, 0.0))
  out = collections::set(out, 7, 0.0 - 1.0)
  RETURN out
END FUNC

' The rim every builder's untransformed box carries past its extent and its stroke, cap
' or join reach: one unit, and untransformed a unit is a surface pixel. It is the `1.0`
' each builder's pad ends in, handed to `__canvas_boundsHeader` beside the pad.
LET __CANVAS_BOUNDS_RIM AS Float = 1.0

' An item's box: slots 16 to 19. A builder hands over its extent, how far its pad reaches
' past the extent on each axis (the reach of its stroke, caps and joins, in its own
' units, with the rim), and how much of that pad is the rim.
'
' Untransformed the box is the extent grown by the pad: a unit is a pixel, so the rim is
' the one pixel of antialiasing room. `canvas::geoBuild` writes the same four values, bit
' for bit (`MFB_CANVAS_GEO_VERIFY`).
'
' Under a transform the item covers the FORWARD-mapped rectangle, so the header carries
' that rectangle's axis-aligned hull -- a rotated square whose bounds were left
' untransformed would be clipped to its own unrotated box, losing its corners. The rim
' is not a unit there (bug-991: under a scale of 170 a unit is 170 pixels, on every
' side of every item). A pixel takes coverage only where `d < 0.5 * g`, `d` the distance
' in the item's space and `g` the length of its central differences half a surface
' pixel apart; a distance is 1-Lipschitz, so `g` is at most the inverse's Frobenius norm
' `F`, the bound `shapeCoverage` skips its neighbour samples by. So the rim is `0.5 * F`
' of the item's units -- `F` widened as the shaders widen it -- in place of the rim the
' builder handed over, and the hull is then padded by a pixel. Under a uniform scale
' that is 0.71 of a pixel and the pixel, whatever the scale.
'
' A builder whose pad is not such a rim hands over its box whole, with no pad and no rim,
' and it is mapped as it is: a metrics box (a glyph run, a text box), a picture's two
' units, and an arc, whose distance is not Lipschitz (out of its sweep it jumps to 1e6,
' so `g` has no bound and `shapeCoverage` never skips for one).
'
' `pad - rim` is exact for a pad that ends in `+ rim`, to half a unit in the pad's last
' place, which `F`'s widening covers many times over.
'
' bug-1083: the box says how much of itself is rim, in slots `__CANVAS_GEO_RIM` (x
' sides) and the one after (y sides), because a group's matrix changes the rim a pixel
' needs and only the site that has that matrix can size it (`__canvas_drawBound`).
' Untransformed that is the builder's rim. Under a transform it is what the mapped rim
' adds to the hull -- `0.5 * F` through the forward matrix is `(|a| + |c|) * 0.5 * F`
' on the x sides and `(|b| + |d|) * 0.5 * F` on the y sides -- and the pixel. A box
' handed over whole has none: nothing may be taken off it.
'
' The forward matrix is recovered by inverting the stored inverse rather than carried in
' six more slots. Two reasons that is safe: a bounding box only has to be CONSERVATIVE,
' so the float32 round-trip's last-bit error cannot matter, and the result is padded by
' a pixel anyway. Extra pixels cost a coverage evaluation that returns 0; missing ones
' cut the shape.
FUNC __canvas_boundsHeader(h AS List OF Float, minX AS Float, minY AS Float, maxX AS Float, maxY AS Float, padX AS Float, padY AS Float, rim AS Float) AS List OF Float
  MUT out AS List OF Float = h
  IF collections::getOr(h, 33, 0.0) > 0.5 THEN
    LET ia AS Float = __canvas_f32FromBits(collections::getOr(h, 27, 0.0))
    LET ib AS Float = __canvas_f32FromBits(collections::getOr(h, 28, 0.0))
    LET ic AS Float = __canvas_f32FromBits(collections::getOr(h, 29, 0.0))
    LET id AS Float = __canvas_f32FromBits(collections::getOr(h, 30, 0.0))
    LET itx AS Float = __canvas_f32FromBits(collections::getOr(h, 31, 0.0))
    LET ity AS Float = __canvas_f32FromBits(collections::getOr(h, 32, 0.0))
    LET fwd AS List OF Float = __canvas_forwardOf(ia, ib, ic, id, itx, ity)
    LET fa AS Float = collections::getOr(fwd, 0, 1.0)
    LET fb AS Float = collections::getOr(fwd, 1, 0.0)
    LET fc AS Float = collections::getOr(fwd, 2, 0.0)
    LET fd AS Float = collections::getOr(fwd, 3, 1.0)
    LET ftx AS Float = collections::getOr(fwd, 4, 0.0)
    LET fty AS Float = collections::getOr(fwd, 5, 0.0)
    MUT growX AS Float = padX
    MUT growY AS Float = padY
    ' Written either way: a picture's header arrives here already built as a rectangle's.
    MUT rimX AS Float = 0.0
    MUT rimY AS Float = 0.0
    IF rim > 0.0 THEN
      LET frob AS Float = math::sqrt(ia * ia + ib * ib + ic * ic + id * id) * (1.0 + 1.0 / 256.0) + 1.0 / 256.0
      growX = (padX - rim) + 0.5 * frob
      growY = (padY - rim) + 0.5 * frob
      rimX = (__canvas_absF(fa) + __canvas_absF(fc)) * (0.5 * frob) + 1.0
      rimY = (__canvas_absF(fb) + __canvas_absF(fd)) * (0.5 * frob) + 1.0
    END IF
    out = collections::set(out, __CANVAS_GEO_RIM, rimX)
    out = collections::set(out, __CANVAS_GEO_RIM + 1, rimY)
    LET lowX AS Float = minX - growX
    LET lowY AS Float = minY - growY
    LET highX AS Float = maxX + growX
    LET highY AS Float = maxY + growY
    IF TRUE THEN
      LET x0 AS Float = fa * lowX + fc * lowY + ftx
      LET y0 AS Float = fb * lowX + fd * lowY + fty
      LET x1 AS Float = fa * highX + fc * lowY + ftx
      LET y1 AS Float = fb * highX + fd * lowY + fty
      LET x2 AS Float = fa * lowX + fc * highY + ftx
      LET y2 AS Float = fb * lowX + fd * highY + fty
      LET x3 AS Float = fa * highX + fc * highY + ftx
      LET y3 AS Float = fb * highX + fd * highY + fty
      out = collections::set(out, 16, __canvas_minF(__canvas_minF(x0, x1), __canvas_minF(x2, x3)) - 1.0)
      out = collections::set(out, 17, __canvas_minF(__canvas_minF(y0, y1), __canvas_minF(y2, y3)) - 1.0)
      out = collections::set(out, 18, __canvas_maxF(__canvas_maxF(x0, x1), __canvas_maxF(x2, x3)) + 1.0)
      out = collections::set(out, 19, __canvas_maxF(__canvas_maxF(y0, y1), __canvas_maxF(y2, y3)) + 1.0)
      RETURN out
    END IF
  END IF
  out = collections::set(out, 16, minX - padX)
  out = collections::set(out, 17, minY - padY)
  out = collections::set(out, 18, maxX + padX)
  out = collections::set(out, 19, maxY + padY)
  out = collections::set(out, __CANVAS_GEO_RIM, rim)
  out = collections::set(out, __CANVAS_GEO_RIM + 1, rim)
  RETURN out
END FUNC

FUNC __canvas_rectHeader(x AS Float, y AS Float, w AS Float, h AS Float, corner AS Float, paint AS Paint, transform AS Transform, ox AS Float, oy AS Float) AS List OF Float
  MUT out AS List OF Float = __canvas_blankHeader()
  IF w <= 0.0 THEN
    RETURN __canvas_emptyHeader()
  END IF
  IF h <= 0.0 THEN
    RETURN __canvas_emptyHeader()
  END IF
  LET limit AS Float = __canvas_minF(w, h) / 2.0
  LET radius AS Float = __canvas_minF(__canvas_maxF(corner, 0.0), limit)
  out = collections::set(out, 0, toFloat(__CANVAS_KIND_RECT))
  out = collections::set(out, 1, toFloat(__CANVAS_GEO_HEADER))
  out = collections::set(out, 2, x + w / 2.0)
  out = collections::set(out, 3, y + h / 2.0)
  out = collections::set(out, 4, w / 2.0 - radius)
  out = collections::set(out, 5, h / 2.0 - radius)
  out = collections::set(out, 6, radius)
  out = __canvas_paintHeader(out, paint, transform, ox, oy)
  LET pad AS Float = __canvas_strokePad(paint)
  RETURN __canvas_boundsHeader(out, x, y, x + w, y + h, pad, pad, __CANVAS_BOUNDS_RIM)
END FUNC

' plan-172-E: a rectangle's corners as ONE circular radius, or -1.0 when they are not
' one (`__canvas_uniformRadius`), in which case it is drawn from `__canvas_rectRing`.
FUNC __canvas_rectRadius(r AS Rectangle) AS Float
  RETURN __canvas_uniformRadius(r.size.width, r.size.height, r.radiusTL, r.radiusTR, r.radiusBL, r.radiusBR)
END FUNC

' The top-left corner of a rectangle's record: the build origin (`position`, or the
' local origin when its transform is in effect) plus where `bind` puts that corner.
FUNC __canvas_rectLeft(r AS Rectangle) AS Float
  RETURN __canvas_buildX(r.position, r.transform) + __canvas_bindOffset(r.bind, r.size.width)
END FUNC

FUNC __canvas_rectTop(r AS Rectangle) AS Float
  RETURN __canvas_buildY(r.position, r.transform) + __canvas_bindOffset(r.bind, r.size.height)
END FUNC

' The exact record: four square corners, or one circular radius.
FUNC __canvas_rectItem(r AS Rectangle, radius AS Float) AS List OF Float
  RETURN __canvas_rectHeader(__canvas_rectLeft(r), __canvas_rectTop(r), r.size.width, r.size.height, radius, r.paint, __canvas_placement(r.position, r.transform), __canvas_buildX(r.position, r.transform), __canvas_buildY(r.position, r.transform))
END FUNC

' The flattened outline, for corners that are not one circular radius. A rectangle with
' no width or no height has none: it draws nothing, as the exact record does.
FUNC __canvas_rectRing(r AS Rectangle) AS List OF List OF vector::Float2
  IF r.size.width <= 0.0 OR r.size.height <= 0.0 THEN
    RETURN []
  END IF
  RETURN [__canvas_rrectRing(__canvas_rectLeft(r), __canvas_rectTop(r), r.size.width, r.size.height, r.radiusTL, r.radiusTR, r.radiusBL, r.radiusBR)]
END FUNC

FUNC __canvas_circleHeader(x AS Float, y AS Float, radius AS Float, paint AS Paint, transform AS Transform) AS List OF Float
  IF radius <= 0.0 THEN
    RETURN __canvas_emptyHeader()
  END IF
  MUT out AS List OF Float = __canvas_blankHeader()
  out = collections::set(out, 0, toFloat(__CANVAS_KIND_CIRCLE))
  out = collections::set(out, 1, toFloat(__CANVAS_GEO_HEADER))
  out = collections::set(out, 2, x)
  out = collections::set(out, 3, y)
  out = collections::set(out, 4, radius)
  out = __canvas_paintHeader(out, paint, transform, x, y)
  LET reach AS Float = radius + __canvas_dashCapReach(__canvas_maxF(__canvas_strokeHalf(paint), 0.0), paint) + 1.0
  RETURN __canvas_boundsHeader(out, x, y, x, y, reach, reach, __CANVAS_BOUNDS_RIM)
END FUNC

' plan-116-E: an ellipse's header.
'
' The rotation is stored as its COSINE and SINE rather than as the angle, and that is
' the whole reason this variant can be byte-identical across the three renderers.
' `helper_shapes.rs`'s TRIG note explains at length why `math::sin`/`cos` are unusable
' here -- libm is not correctly rounded, so its last bit differs between platforms --
' and the two shaders' hardware trigonometry differs again. Evaluating the
' deterministic Taylor pair ONCE on the CPU means all three renderers read the same two
' numbers, and no trigonometry appears in any per-pixel path.
'
' Bounds: the axis-aligned hull of a rotated ellipse is
' `hx = sqrt((rx*cos)^2 + (ry*sin)^2)`, `hy = sqrt((rx*sin)^2 + (ry*cos)^2)` -- exact,
' and `sqrt`-only, so it costs the reproducibility rule nothing.
FUNC __canvas_circleItem(c AS Circle) AS List OF Float
  LET bx AS Float = __canvas_buildX(c.position, c.transform)
  LET by AS Float = __canvas_buildY(c.position, c.transform)
  RETURN __canvas_circleHeader(bx, by, c.radius, c.paint, __canvas_placement(c.position, c.transform))
END FUNC

' plan-172-D: a text's build origin and carried transform. The glyph run,
' the outline edges and both headers each need them, so they are functions of the item
' rather than four locals repeated in five builders.
FUNC __canvas_textX(t AS Text) AS Float
  RETURN __canvas_buildX(t.position, t.transform)
END FUNC

FUNC __canvas_textY(t AS Text) AS Float
  RETURN __canvas_buildY(t.position, t.transform)
END FUNC

FUNC __canvas_textTransform(t AS Text) AS Transform
  RETURN __canvas_placement(t.position, t.transform)
END FUNC

FUNC __canvas_ellipseHeader(e AS Ellipse) AS List OF Float
  LET bx AS Float = __canvas_buildX(e.position, e.transform)
  LET by AS Float = __canvas_buildY(e.position, e.transform)
  LET placed AS Paint = e.paint
  LET carried AS Transform = __canvas_placement(e.position, e.transform)
  IF e.radiusX <= 0.0 THEN
    RETURN __canvas_emptyHeader()
  END IF
  IF e.radiusY <= 0.0 THEN
    RETURN __canvas_emptyHeader()
  END IF
  ' plan-172-C: an ellipse has no angle of its own; it is turned by its `transform`.
  ' The two slots keep their meaning and carry the identity rotation.
  LET ca AS Float = 1.0
  LET sa AS Float = 0.0
  MUT out AS List OF Float = __canvas_blankHeader()
  out = collections::set(out, 0, toFloat(__CANVAS_GEO_ELLIPSE))
  out = collections::set(out, 1, toFloat(__CANVAS_GEO_HEADER))
  out = collections::set(out, 2, bx)
  out = collections::set(out, 3, by)
  out = collections::set(out, 4, e.radiusX)
  out = collections::set(out, 5, e.radiusY)
  out = collections::set(out, __CANVAS_GEO_ELLIPSE_COS, ca)
  out = collections::set(out, __CANVAS_GEO_ELLIPSE_SIN, sa)
  out = __canvas_paintHeader(out, placed, carried, bx, by)
  LET pad AS Float = __canvas_dashCapReach(__canvas_maxF(__canvas_strokeHalf(placed), 0.0), placed) + 1.0
  LET ex AS Float = e.radiusX * ca
  LET ey AS Float = e.radiusY * sa
  LET fx AS Float = e.radiusX * sa
  LET fy AS Float = e.radiusY * ca
  LET hx AS Float = math::sqrt(ex * ex + ey * ey) + pad
  LET hy AS Float = math::sqrt(fx * fx + fy * fy) + pad
  RETURN __canvas_boundsHeader(out, bx, by, bx, by, hx, hy, __CANVAS_BOUNDS_RIM)
END FUNC

FUNC __canvas_segmentHeader(x1 AS Float, y1 AS Float, x2 AS Float, y2 AS Float, cap AS Integer, paint AS Paint, transform AS Transform, ox AS Float, oy AS Float) AS List OF Float
  LET half AS Float = __canvas_strokeHalf(paint)
  IF half <= 0.0 THEN
    RETURN __canvas_emptyHeader()
  END IF
  MUT out AS List OF Float = __canvas_blankHeader()
  out = collections::set(out, 0, toFloat(__CANVAS_KIND_SEGMENT))
  out = collections::set(out, 1, toFloat(__CANVAS_GEO_HEADER))
  out = collections::set(out, 2, x1)
  out = collections::set(out, 3, y1)
  out = collections::set(out, 4, x2)
  out = collections::set(out, 5, y2)
  out = collections::set(out, 6, half)
  out = collections::set(out, __CANVAS_GEO_CAP, toFloat(cap))
  out = __canvas_paintHeader(out, paint, transform, ox, oy)
  out = __canvas_strokeAsFill(out)
  LET pad AS Float = __canvas_capReach(half, cap) + 1.0
  RETURN __canvas_boundsHeader(out, __canvas_minF(x1, x2), __canvas_minF(y1, y2), __canvas_maxF(x1, x2), __canvas_maxF(y1, y2), pad, pad, __CANVAS_BOUNDS_RIM)
END FUNC

' plan-172-F: a line's header. `startAt` and `endAt` are offsets from `position`, so
' each end is the build origin plus its offset -- `position` when no transform is in
' effect, the local origin when one is. `__canvas_lineDashes` reads the two ends back
' from the header, so the dashes follow without an edit.
FUNC __canvas_lineHeader(l AS Line) AS List OF Float
  LET bx AS Float = __canvas_buildX(l.position, l.transform)
  LET by AS Float = __canvas_buildY(l.position, l.transform)
  RETURN __canvas_segmentHeader(l.startAt.x + bx, l.startAt.y + by, l.endAt.x + bx, l.endAt.y + by, __canvas_capTag(l.paint.cap), l.paint, __canvas_placement(l.position, l.transform), bx, by)
END FUNC

FUNC __canvas_arcHeader(a AS Arc) AS List OF Float
  LET bx AS Float = __canvas_buildX(a.position, a.transform)
  LET by AS Float = __canvas_buildY(a.position, a.transform)
  LET placed AS Paint = a.paint
  LET carried AS Transform = __canvas_placement(a.position, a.transform)
  LET half AS Float = __canvas_strokeHalf(placed)
  IF half <= 0.0 THEN
    RETURN __canvas_emptyHeader()
  END IF
  IF a.radius <= 0.0 THEN
    RETURN __canvas_emptyHeader()
  END IF
  ' bug-1088: a sweep of zero (or less: the end before the start) has no length, and a
  ' butt end adds nothing past an end: like a zero-length butt line, it draws nothing. A
  ' Round or Square one draws its two ends.
  IF a.endAngle <= a.startAngle AND __canvas_capTag(placed.cap) = 0 THEN
    RETURN __canvas_emptyHeader()
  END IF
  MUT out AS List OF Float = __canvas_blankHeader()
  out = collections::set(out, 0, toFloat(__CANVAS_GEO_ARC))
  out = collections::set(out, 1, toFloat(__CANVAS_GEO_HEADER))
  out = collections::set(out, 2, bx)
  out = collections::set(out, 3, by)
  out = collections::set(out, 4, a.radius)
  out = collections::set(out, 6, half)
  out = __canvas_paintHeader(out, placed, carried, bx, by)
  out = __canvas_strokeAsFill(out)
  out = collections::set(out, 20, a.startAngle)
  out = collections::set(out, 21, a.endAngle)
  out = collections::set(out, __CANVAS_GEO_CAP, toFloat(__canvas_capTag(placed.cap)))
  ' plan-116-D: the two sweep endpoints, in surface pixels. Per-shape constants, so
  ' they are computed ONCE here rather than per pixel in each of three renderers --
  ' which is the same reason the sweep vectors are turned into sin/cos here. The two
  ' extra multiply-adds ride on `__canvas_cos`/`__canvas_sin` calls the arc already
  ' makes, so a round-capped arc costs no extra transcendental.
  out = collections::set(out, __CANVAS_GEO_CAPSTARTX, bx + a.radius * __canvas_cos(a.startAngle))
  out = collections::set(out, __CANVAS_GEO_CAPSTARTY, by + a.radius * __canvas_sin(a.startAngle))
  out = collections::set(out, __CANVAS_GEO_CAPENDX, bx + a.radius * __canvas_cos(a.endAngle))
  out = collections::set(out, __CANVAS_GEO_CAPENDY, by + a.radius * __canvas_sin(a.endAngle))
  LET reach AS Float = a.radius + __canvas_capReach(half, __canvas_capTag(placed.cap)) + 1.0
  ' An arc's box goes over whole, with no rim (`__canvas_boundsHeader`): the square about
  ' its whole ring, not an extent of the swept part. It was chosen while the distance
  ' jumped at the ends of the sweep, which bug-1088 removed; the box is unchanged.
  RETURN __canvas_boundsHeader(out, bx - reach, by - reach, bx + reach, by + reach, 0.0, 0.0, 0.0)
END FUNC

' plan-163-A: a polygon's rings. A ring of fewer than 2 points contributes no edge and
' no bounds; the others contribute every point, so the edge count is the sum of their
' lengths. The bounds start from the first contributing ring's first point and take
' minF/maxF over every later point in order -- for one ring exactly the walk this had
' before rings, so a single-ring EvenOdd polygon's header is unchanged (slot 21 is 0 for
' EvenOdd). `func_geo_build.rs:fill_polygon` is the native twin, in the same order.
FUNC __canvas_polygonHeader(rings AS List OF List OF vector::Float2, rule AS FillRule, paint AS Paint, transform AS Transform, ox AS Float, oy AS Float) AS List OF Float
  LET origin AS vector::Float2 = vector::Float2[x := 0.0, y := 0.0]
  MUT count AS Integer = 0
  MUT minX AS Float = 0.0
  MUT maxX AS Float = 0.0
  MUT minY AS Float = 0.0
  MUT maxY AS Float = 0.0
  FOR EACH ring IN rings
    LET n AS Integer = len(ring)
    IF n >= 2 THEN
      MUT i AS Integer = 0
      IF count = 0 THEN
        LET first AS vector::Float2 = collections::getOr(ring, 0, origin)
        minX = first.x
        maxX = first.x
        minY = first.y
        maxY = first.y
        i = 1
      END IF
      WHILE i < n
        LET q AS vector::Float2 = collections::getOr(ring, i, origin)
        minX = __canvas_minF(minX, q.x)
        maxX = __canvas_maxF(maxX, q.x)
        minY = __canvas_minF(minY, q.y)
        maxY = __canvas_maxF(maxY, q.y)
        i = i + 1
      END WHILE
      count = count + n
    END IF
  NEXT
  IF count = 0 THEN
    RETURN __canvas_emptyHeader()
  END IF
  MUT out AS List OF Float = __canvas_blankHeader()
  out = collections::set(out, 0, toFloat(__CANVAS_GEO_POLYGON))
  out = collections::set(out, 1, toFloat(__CANVAS_GEO_HEADER + count * 5))
  out = __canvas_paintHeader(out, paint, transform, ox, oy)
  out = collections::set(out, 20, toFloat(count))
  IF rule = FillRule.NonZero THEN
    out = collections::set(out, __CANVAS_GEO_POLYGON_RULE, 1.0)
  END IF
  LET pad AS Float = __canvas_strokePad(paint)
  RETURN __canvas_boundsHeader(out, minX, minY, maxX, maxY, pad, pad, __CANVAS_BOUNDS_RIM)
END FUNC

' plan-154-A: a PolyLine's stroke half-width, or a negative number when it draws nothing.
'
' With no colours it is exactly `__canvas_strokeHalf` -- a transparent `paint.stroke`
' draws nothing, as it does for a Line. With colours the per-point colours ARE the
' stroke, so only the width decides: a program that fades a trail from colour to
' colour should not also have to set `paint.stroke` to something opaque that is never
' drawn.
FUNC __canvas_polylineHalf(p AS PolyLine) AS Float
  IF len(p.colors) = 0 THEN
    RETURN __canvas_strokeHalf(p.paint)
  END IF
  IF p.paint.strokeWidth <= 0.0 THEN
    RETURN 0.0 - 1.0
  END IF
  RETURN p.paint.strokeWidth / 2.0
END FUNC

' plan-154-A: a PolyLine's header.
'
' The first and last segments WITH LENGTH are found here, once, because they are where
' a butt cap cuts. A repeated end point makes a zero-length first or last segment, and
' cutting a zero-length segment square has no direction to cut along -- so the cut moves
' to the nearest segment that has one, and the repeated point adds nothing. The length
' test is `dx * dx + dy * dy > 0.0` on the same differences the tail stores, so the
' distance walk (`__canvas_polylineScan`) and this agree on which segments are live.
'
' A path with no live segment at all -- every point the same -- draws what a
' zero-length Line with its cap draws: a dot for Round (segment 0, which the walk then
' measures as a point), nothing for Butt (`__canvas_segmentDistanceButt` answers "far
' outside" for a zero-length segment, so the empty header is the same picture).
' plan-172-F: `p` arrives through `__canvas_polylinePlaced`, so `p.points` are already
' in the coordinates the record is built in.
FUNC __canvas_polylineHeader(p AS PolyLine) AS List OF Float
  LET count AS Integer = len(p.points)
  IF count < 2 THEN
    RETURN __canvas_emptyHeader()
  END IF
  LET half AS Float = __canvas_polylineHalf(p)
  IF half <= 0.0 THEN
    RETURN __canvas_emptyHeader()
  END IF
  LET origin AS vector::Float2 = vector::Float2[x := 0.0, y := 0.0]
  LET start AS vector::Float2 = collections::getOr(p.points, 0, origin)
  MUT minX AS Float = start.x
  MUT maxX AS Float = start.x
  MUT minY AS Float = start.y
  MUT maxY AS Float = start.y
  MUT first AS Integer = 0 - 1
  MUT last AS Integer = 0 - 1
  MUT i AS Integer = 0
  WHILE i + 1 < count
    LET a AS vector::Float2 = collections::getOr(p.points, i, origin)
    LET b AS vector::Float2 = collections::getOr(p.points, i + 1, origin)
    LET dx AS Float = b.x - a.x
    LET dy AS Float = b.y - a.y
    IF dx * dx + dy * dy > 0.0 THEN
      IF first < 0 THEN
        first = i
      END IF
      last = i
    END IF
    minX = __canvas_minF(minX, b.x)
    maxX = __canvas_maxF(maxX, b.x)
    minY = __canvas_minF(minY, b.y)
    maxY = __canvas_maxF(maxY, b.y)
    i = i + 1
  END WHILE
  LET cap AS Integer = __canvas_capTag(p.paint.cap)
  IF first < 0 THEN
    IF cap = 0 THEN
      RETURN __canvas_emptyHeader()
    END IF
    first = 0
    last = 0
  END IF
  MUT stops AS Integer = 0
  IF len(p.colors) > 0 THEN
    stops = count
  END IF
  MUT out AS List OF Float = __canvas_blankHeader()
  out = collections::set(out, 0, toFloat(__CANVAS_GEO_POLYLINE))
  out = collections::set(out, 6, half)
  out = collections::set(out, __CANVAS_GEO_CAP, toFloat(cap))
  out = __canvas_paintHeader(out, p.paint, __canvas_placement(p.position, p.transform), __canvas_buildX(p.position, p.transform), __canvas_buildY(p.position, p.transform))
  out = __canvas_strokeAsFill(out)
  out = collections::set(out, 1, toFloat(__CANVAS_GEO_HEADER + (count - 1) * 5 + stops * 5))
  out = collections::set(out, __CANVAS_GEO_GRADIENT_COUNT, toFloat(stops))
  out = collections::set(out, __CANVAS_GEO_GRADIENT_KIND, 0.0)
  out = collections::set(out, 20, toFloat(count - 1))
  out = collections::set(out, __CANVAS_GEO_POLYLINE_FIRST, toFloat(first))
  out = collections::set(out, __CANVAS_GEO_POLYLINE_LAST, toFloat(last))
  out = collections::set(out, __CANVAS_GEO_GRADIENT_TOX, 0.0)
  out = collections::set(out, __CANVAS_GEO_GRADIENT_TOY, 0.0)
  LET pad AS Float = __canvas_maxF(__canvas_capReach(half, cap), __canvas_joinReach(half, p.paint)) + 1.0
  RETURN __canvas_boundsHeader(out, minX, minY, maxX, maxY, pad, pad, __CANVAS_BOUNDS_RIM)
END FUNC"#
    };
}
pub(super) use geo_header_kinds;
