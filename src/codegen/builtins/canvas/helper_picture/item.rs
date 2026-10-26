//! plan-176-R: the `helper_picture` MFBASIC helper text — the picture item (`PICTURE`).
//! Registration lives in `builtins/canvas/helper_picture/mod.rs`.

/// The item (the module note). Everything here runs on the graphics thread or, for
/// the hit test's three, on the worker; nothing can raise.
///
/// **The cache.** A plain picture's record is under the item's own hash, as every
/// item's is. A shaped picture's two are under that hash stepped
/// (`__canvas_pictureKey`) and nothing is under the hash itself — so the native first
/// pass (`canvas::sceneResolve`) never resolves one and it always reaches
/// `__canvas_pictureAppend`. Either way a picture record is confirmed against the
/// image's current pixel block before it is used (`__canvas_pictureCachedOf`).
///
/// **An `AutoPaint` for a shape** makes the picture an instance
/// (`helper_autopaint.rs`): always two entries, the second of which — the outline, so
/// the shape — is the one the pointer is tested against, and both of which the frame's
/// resolve swaps for the state paint's.
#[rustfmt::skip]
pub(super) const PICTURE: &str =
r#"' The cache keys of a shaped picture's two records under hash `hash`: `part` is 201 for
' the image and 202 for the outline -- 211 and 212 when the shape is an `AutoPaint`,
' whose records must not be found by the quick path below -- and `shown` the state paint
' they are built under (0: the shape's own). Never the item's hash itself: only a plain
' picture's one record is cached under that.
FUNC __canvas_pictureKey(hash AS Integer, part AS Integer, shown AS Integer) AS Integer
  RETURN __canvas_hashStep(__canvas_hashStep(hash, part), shown)
END FUNC

' Whether the record at `offset` may be used for an image whose pixel block is `shadow`:
' `setBytes` swaps an image's pixel block without changing the item, so a picture or a
' tile record is the one cached kind a hash cannot vouch for.
'
' a picture or tile record must name that block; any other record names none.
FUNC __canvas_pictureShadowOk(offset AS Integer, shadow AS Integer) AS Boolean
  LET kind AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, offset, 0.0))
  IF kind = __CANVAS_GEO_PICTURE OR kind = __CANVAS_GEO_TILE THEN
    LET hi AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, offset + __CANVAS_GEO_PICTURE_SHADOW_HI, 0.0))
    LET lo AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, offset + __CANVAS_GEO_PICTURE_SHADOW_LO, 0.0))
    RETURN hi * __CANVAS_GEO_PICTURE_SPLIT + lo = shadow
  END IF
  RETURN TRUE
END FUNC

' plan-175-A: the picture record keyed by a geometry identity `g` and
' holding a damage identity `d` -- `__canvas_geoLookup`'s branches, with the pixel block
' confirmed before a record is trusted. A record whose pixels or paint are not the item's
' is rebuilt in place when nothing else this frame uses it (`__canvas_pictureStore`).
FUNC __canvas_pictureCachedOf(g AS Integer, d AS Integer, shadow AS Integer) AS Integer
  LET slot AS Integer = canvas::geoFind(g)
  IF slot < 0 THEN
    __CANVAS_GEO_PLAN_SLOT = 0 - 1
    __CANVAS_GEO_PLAN_KEY = g
    RETURN 0 - 1
  END IF
  LET offset AS Integer = collections::getOr(__CANVAS_GEO_SLOTS, slot * 4 + 1, 0)
  IF collections::getOr(__CANVAS_GEO_DRAWS, slot, 0 - 1) = d AND __canvas_pictureShadowOk(offset, shadow) THEN
    __canvas_geoStamp(slot)
    RETURN offset
  END IF
  IF collections::getOr(__CANVAS_GEO_SLOTS, slot * 4 + 3, 0) <> __CANVAS_GEO_FRAME THEN
    __CANVAS_GEO_PLAN_SLOT = slot
    __CANVAS_GEO_PLAN_KEY = g
    RETURN 0 - 1
  END IF
  LET shared AS Integer = canvas::geoFind(d)
  IF shared >= 0 THEN
    LET at AS Integer = collections::getOr(__CANVAS_GEO_SLOTS, shared * 4 + 1, 0)
    IF __canvas_pictureShadowOk(at, shadow) THEN
      __canvas_geoStamp(shared)
      RETURN at
    END IF
  END IF
  __CANVAS_GEO_PLAN_SLOT = 0 - 1
  __CANVAS_GEO_PLAN_KEY = d
  RETURN 0 - 1
END FUNC

' Store a picture record built after `__canvas_pictureCachedOf` answered -1.
FUNC __canvas_pictureStore(d AS Integer, built AS List OF List OF Float) AS Integer
  LET none AS List OF Float = []
  LET empty AS List OF Float = __canvas_emptyHeader()
  RETURN __canvas_geoStore(d, collections::getOr(built, 0, empty), collections::getOr(built, 1, none))
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

' plan-175-A: a tile's appearance lane is empty, so its `d` changes only with its `g`.
FUNC __canvas_tileGeometry(tile AS Tile, g AS Integer, d AS Integer) AS Integer
  LET shadow AS Integer = canvas::imageShadow(tile.image)
  LET cached AS Integer = __canvas_pictureCachedOf(g, d, shadow)
  IF cached >= 0 THEN
    RETURN cached
  END IF
  LET built AS List OF List OF Float = __canvas_tileRecord(tile, shadow, canvas::imageWidthOf(tile.image), canvas::imageHeightOf(tile.image))
  RETURN __canvas_pictureStore(d, built)
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
'
' plan-175-A: each record is keyed by `geo` stepped (the picture's geometry identity) and
' holds `hash` stepped (its damage identity), so a picture whose tint alone changed finds
' its records and rewrites them in place. The pairs this answers carry the `hash` keys,
' which are what the draw hashes fold (`__canvas_noteEntry`): a tint change still damages.
FUNC __canvas_pictureBlocksOf(shape AS DrawItem, pic AS Picture, geo AS Integer, hash AS Integer, shown AS Integer, fixed AS Boolean) AS List OF Integer
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
  LET imageGeo AS Integer = __canvas_pictureKey(geo, part, shown)
  LET outlineGeo AS Integer = __canvas_pictureKey(geo, part + 1, shown)
  MUT image AS Integer = __canvas_pictureCachedOf(imageGeo, imageKey, shadow)
  IF image < 0 THEN
    MUT fit AS Integer = 0
    IF pic.fit = PictureFit.Contain THEN
      fit = 1
    END IF
    IF pic.fit = PictureFit.Cover THEN
      fit = 2
    END IF
    image = __canvas_pictureStore(imageKey, __canvas_pictureImage(shape, clipOnly, fit, shadow, iw, ih))
  END IF
  MUT outline AS Integer = __canvas_pictureCachedOf(outlineGeo, outlineKey, 0)
  IF outline < 0 THEN
    ' A destroyed image draws nothing, its outline included -- as a plain picture's
    ' one record draws neither.
    IF shadow = 0 OR iw <= 0 OR ih <= 0 THEN
      outline = __canvas_geometryOf(__canvas_noItem(), outlineGeo, outlineKey)
    ELSE
      outline = __canvas_geometryOf(__canvas_pictureOutlineItem(shape, fixed OR pic.fit = PictureFit.Contain), outlineGeo, outlineKey)
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
'
' plan-175-A Phase 6: `shadow` is the image's pixel block, read once by the caller for
' the whole picture.
FUNC __canvas_picturePlainOffset(pic AS Picture, geo AS Integer, hash AS Integer, shadow AS Integer) AS Integer
  LET found AS Integer = __canvas_pictureCachedOf(geo, hash, shadow)
  IF found >= 0 THEN
    RETURN found
  END IF
  IF pic.fit <> PictureFit.Stretch THEN
    RETURN 0 - 1
  END IF
  MATCH pic.shape
    CASE Rectangle(r)
      IF __canvas_rectRadius(r) = 0.0 AND NOT (pic.clip = PictureClip.ClipOnly AND __canvas_transformActive(r.transform)) THEN
        RETURN __canvas_pictureStore(hash, __canvas_picturePlain(r, shadow, canvas::imageWidthOf(pic.image), canvas::imageHeightOf(pic.image)))
      END IF
    CASE ELSE
  END MATCH
  RETURN 0 - 1
END FUNC

' The (offset, key) pairs of a picture whose shape is not an `AutoPaint`: the plain
' record; or a shaped picture's two, from the cache alone when they are there -- every
' frame after the first -- so the shape is not copied out of the item at all.
FUNC __canvas_pictureQuick(pic AS Picture, geo AS Integer, hash AS Integer) AS List OF Integer
  LET shadow AS Integer = canvas::imageShadow(pic.image)
  LET plain AS Integer = __canvas_picturePlainOffset(pic, geo, hash, shadow)
  IF plain >= 0 THEN
    RETURN [plain, hash]
  END IF
  IF __canvas_pictureShapedHit(geo, hash, shadow) THEN
    MUT out AS List OF Integer = []
    IF __CANVAS_PIC_IMAGE >= 0 THEN
      out = collections::append(out, __CANVAS_PIC_IMAGE)
      out = collections::append(out, __CANVAS_PIC_IMAGE_KEY)
    END IF
    IF __CANVAS_PIC_OUTLINE >= 0 THEN
      out = collections::append(out, __CANVAS_PIC_OUTLINE)
      out = collections::append(out, __CANVAS_PIC_OUTLINE_KEY)
    END IF
    RETURN out
  END IF
  RETURN __canvas_pictureBlocksOf(__canvas_shapeAsItem(pic.shape), pic, geo, hash, 0, FALSE)
END FUNC

' plan-175-A Phase 6: whether a `Tile` or a `Picture` of identities `geo` and `hash`,
' its image's pixel block `shadow` (`canvas::itemImageShadow`), finds every record it
' draws from in the cache. The answer is in `__CANVAS_PIC_*` (`__canvas_appendDraw`),
' so the walk lays the item out without copying it out of its variant. A plain picture
' and a tile are one record under `geo`; a shaped picture is two under `geo` stepped.
FUNC __canvas_imageCached(geo AS Integer, hash AS Integer, shadow AS Integer) AS Boolean
  LET plain AS Integer = __canvas_pictureCachedOf(geo, hash, shadow)
  IF plain >= 0 THEN
    __CANVAS_PIC_IMAGE = plain
    __CANVAS_PIC_IMAGE_KEY = hash
    __CANVAS_PIC_OUTLINE = 0 - 1
    RETURN TRUE
  END IF
  RETURN __canvas_pictureShapedHit(geo, hash, shadow)
END FUNC

' Whether a shaped picture of identities `geo` and `hash`, its image's pixel block
' `shadow`, finds both its records in the cache -- every frame after the first. The
' answer is in the four globals above, so the walk lays out a picture without making a
' list for it.
FUNC __canvas_pictureShapedHit(geo AS Integer, hash AS Integer, shadow AS Integer) AS Boolean
  LET imageKey AS Integer = __canvas_pictureKey(hash, 201, 0)
  LET image AS Integer = __canvas_pictureCachedOf(__canvas_pictureKey(geo, 201, 0), imageKey, shadow)
  IF image < 0 THEN
    RETURN FALSE
  END IF
  LET outlineKey AS Integer = __canvas_pictureKey(hash, 202, 0)
  LET outline AS Integer = __canvas_pictureCachedOf(__canvas_pictureKey(geo, 202, 0), outlineKey, 0)
  IF outline < 0 THEN
    RETURN FALSE
  END IF
  __CANVAS_PIC_IMAGE = image
  IF toInt(collections::getOr(__CANVAS_GEO_DATA, image, 0.0)) = __CANVAS_GEO_NONE THEN
    __CANVAS_PIC_IMAGE = 0 - 1
  END IF
  __CANVAS_PIC_IMAGE_KEY = imageKey
  __CANVAS_PIC_OUTLINE = outline
  IF toInt(collections::getOr(__CANVAS_GEO_DATA, outline, 0.0)) = __CANVAS_GEO_NONE THEN
    __CANVAS_PIC_OUTLINE = 0 - 1
  END IF
  __CANVAS_PIC_OUTLINE_KEY = outlineKey
  RETURN TRUE
END FUNC

' `__canvas_memoGroup`'s picture: the (offset, key) pairs of the picture at `index` of
' the group use `path`. A picture on an `AutoPaint` lays out the records of the paint
' the frame's resolve chose for that place, so the two walks name the same ones. (A
' picture in the scene itself is laid out from the frame's entries:
' `__canvas_sceneDrawsWalk`.)
'
' plan-175-A: a picture on an `AutoPaint` keys its state records by its damage identity,
' as an `AutoPaint`'s own are (`__canvas_autoGeoHash`), and never refreshes them.
FUNC __canvas_pictureBlocks(pic AS Picture, geo AS Integer, hash AS Integer, path AS Integer, index AS Integer) AS List OF Integer
  MATCH pic.shape
    CASE AutoPaint(a)
      LET shown AS Integer = __canvas_autoShownAt(path, index)
      RETURN __canvas_pictureBlocksOf(__canvas_autoShapeShown(a, shown), pic, hash, hash, shown, TRUE)
    CASE ELSE
      RETURN __canvas_pictureQuick(pic, geo, hash)
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
      RETURN __canvas_pictureBlocksOf(__canvas_autoShapeShown(a, shown), pic, hash, hash, shown, TRUE)
    END IF
    n = n + 1
  NEXT
  RETURN []
END FUNC

' `__canvas_appendDraw`'s picture: one entry for each record it draws from. A picture
' on an `AutoPaint` is noted as an instance at this place -- its entry the outline's,
' which is the shape and so is what the pointer is over -- and is appended under the
' shape's OWN paint, which is what the hit test must read.
FUNC __canvas_pictureAppend(offsets AS List OF Integer, pic AS Picture, geo AS Integer, hash AS Integer, gdx AS Float, gdy AS Float, mat AS Integer, depth AS Integer, path AS Integer, index AS Integer) AS List OF Integer
  MUT out AS List OF Integer = offsets
  MATCH pic.shape
    CASE AutoPaint(a)
      LET blocks AS List OF Integer = __canvas_pictureBlocksOf(__canvas_autoShapeShown(a, 0), pic, hash, hash, 0, TRUE)
      IF __CANVAS_PICTURE_HOSTS_FRAME <> __CANVAS_GEO_FRAME THEN
        __CANVAS_PICTURE_HOSTS = []
        __CANVAS_PICTURE_HOSTS_FRAME = __CANVAS_GEO_FRAME
      END IF
      __CANVAS_PICTURE_HOSTS = collections::append(__CANVAS_PICTURE_HOSTS, pic)
      __canvas_autoNote(a, hash, len(out) + 1, depth, path, index, len(__CANVAS_PICTURE_HOSTS) - 1)
      RETURN __canvas_pictureNoteBlocks(out, blocks, gdx, gdy, mat)
    CASE ELSE
      LET shadow AS Integer = canvas::imageShadow(pic.image)
      LET plain AS Integer = __canvas_picturePlainOffset(pic, geo, hash, shadow)
      IF plain >= 0 THEN
        out = collections::append(out, plain)
        __canvas_noteEntry(plain, hash, gdx, gdy, mat)
        RETURN out
      END IF
      ' plan-175-A Phase 6: a shaped picture found whole in the cache never gets here --
      ' `__canvas_appendDraw` laid it out (`__canvas_imageCached`) -- so this is a miss.
      RETURN __canvas_pictureNoteBlocks(out, __canvas_pictureBlocksOf(__canvas_shapeAsItem(pic.shape), pic, geo, hash, 0, FALSE), gdx, gdy, mat)
  END MATCH
  RETURN out
END FUNC

' `__canvas_pictureAppend`'s entries for the (offset, key) pairs `blocks`.
FUNC __canvas_pictureNoteBlocks(offsets AS List OF Integer, blocks AS List OF Integer, gdx AS Float, gdy AS Float, mat AS Integer) AS List OF Integer
  MUT out AS List OF Integer = offsets
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
' (`__canvas_pictureCachedOf`).
'
' plan-175-A: this is the picture's damage identity `D`, mixed from its geometry identity
' (below) and its shape's own `D`, which carries the shape's appearance. Its `id` is in
' neither (decision 1).
FUNC __canvas_pictureHash(pic AS Picture) AS Integer
  RETURN __canvas_mixHash(__canvas_pictureGeoHash(pic), __canvas_hashItem(__canvas_shapeAsItem(pic.shape)))
END FUNC

' plan-175-A: the geometry identity `G` of such a picture: its clip and fit, its image,
' and its shape's `G`.
FUNC __canvas_pictureGeoHash(pic AS Picture) AS Integer
  MUT h AS Integer = __canvas_hashStart(6)
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
  RETURN __canvas_hashStep(h, __canvas_geoHash(__canvas_shapeAsItem(pic.shape)))
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
      RETURN __canvas_textNamesFont(t, handle)
    CASE AutoPaint(a)
      MATCH a.shape
        CASE Text(t)
          RETURN __canvas_textNamesFont(t, handle)
        CASE ELSE
      END MATCH
    CASE ELSE
  END MATCH
  RETURN FALSE
END FUNC

FUNC __canvas_pictureCloseFont(pic AS Picture, scene AS List OF DrawItem) AS Nothing
  MATCH pic.shape
    CASE Text(t)
      __canvas_closeTextFonts(t, scene)
    CASE AutoPaint(a)
      MATCH a.shape
        CASE Text(t)
          __canvas_closeTextFonts(t, scene)
        CASE ELSE
      END MATCH
    CASE ELSE
  END MATCH
END FUNC
"#;
