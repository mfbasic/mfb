//! plan-176-R: `GEO_HEADER`'s first piece — the blank, cap, paint, gradient and clip header
//! builders. `mod.rs` joins the two pieces with `concat!` into the one registered
//! `canvas_geoHeader` helper.

macro_rules! geo_header_paint {
    () => {
        r#"' bug-686: a copy of one prebuilt all-zero header rather than 51 appends -- one small
' block copy against ~0.9 us per item, on every geometry build.
FUNC __canvas_blankHeader() AS List OF Float
  IF len(__CANVAS_GEO_BLANK) <> __CANVAS_GEO_HEADER THEN
    MUT h AS List OF Float = []
    MUT i AS Integer = 0
    WHILE i < __CANVAS_GEO_HEADER
      h = collections::append(h, 0.0)
      i = i + 1
    END WHILE
    __CANVAS_GEO_BLANK = h
  END IF
  RETURN __CANVAS_GEO_BLANK
END FUNC

FUNC __canvas_emptyHeader() AS List OF Float
  MUT h AS List OF Float = __canvas_blankHeader()
  h = collections::set(h, 0, toFloat(__CANVAS_GEO_NONE))
  h = collections::set(h, 1, toFloat(__CANVAS_GEO_HEADER))
  RETURN h
END FUNC

' plan-116-D: a CapStyle as the 0..1 tag the header slot carries.
'
' A function rather than an inline IF at each of the two call sites, because `Line`
' and `Arc` must agree on the encoding: they read the same slot and the renderers
' branch on it with one comparison for both kinds. Butt is 0 for the same reason
' BlendMode.Normal is -- it is the enum's zero -- but note that unlike blend, the zero
' is NOT what preserved today's rendering for both variants: a Line was round before
' this letter and an Arc was butt, so the existing sites had to be split.
FUNC __canvas_capTag(cap AS CapStyle) AS Integer
  IF cap = CapStyle.Round THEN
    RETURN 1
  END IF
  IF cap = CapStyle.Square THEN
    RETURN 2
  END IF
  RETURN 0
END FUNC

' plan-163-B: how far a stroke of half-width `half` reaches past its outline with this
' cap tag: `half`, or `half * sqrt(2)` for Square, whose corners are a diagonal half
' away from the endpoint.
FUNC __canvas_capReach(half AS Float, cap AS Integer) AS Float
  IF cap = 2 THEN
    RETURN half * 1.4142135623730951
  END IF
  RETURN half
END FUNC

' Where a record's gradient stops begin, in slots from its offset (plan-116-F).
'
' Derived from the record itself rather than from a per-kind tail formula: slot 1 is
' the whole record's length and the stop tail is always LAST, so the base is
' `length - stopCount * 5` whatever the other tail happens to be. The plan's
' `HEADER + edgeCount * EDGE_SLOTS` is right only for a polygon -- see Correction F4.
'
' Every reader calls this. Two readers computing it independently is how the polygon
' cache bug of 2026-09-01 happened.
FUNC __canvas_gradientStopBase(offset AS Integer) AS Integer
  LET count AS Integer = toInt(__canvas_geoAt(offset, __CANVAS_GEO_GRADIENT_COUNT))
  LET length AS Integer = toInt(__canvas_geoAt(offset, 1))
  RETURN offset + length - count * 5
END FUNC

FUNC __canvas_paintHeader(h AS List OF Float, rawPaint AS Paint, transform AS Transform, ox AS Float, oy AS Float) AS List OF Float
  ' plan-172-D: `paint.clip` and a gradient's two points are in the shape's local
  ' space, and (ox, oy) is where this record's geometry puts the local origin. Moved
  ' once here, so everything below reads them in the geometry's own coordinates.
  LET paint AS Paint = __canvas_placedPaint(rawPaint, ox, oy)
  MUT out AS List OF Float = h
  out = collections::set(out, 7, __canvas_strokeHalf(paint))
  out = collections::set(out, 8, toFloat(toInt(paint.fill.red)))
  out = collections::set(out, 9, toFloat(toInt(paint.fill.green)))
  out = collections::set(out, 10, toFloat(toInt(paint.fill.blue)))
  ' plan-163-C: both alphas faded by `transparency` here, once, so no renderer reads it
  ' (`__canvas_fadeAlpha`; the identity at the zero value).
  LET fade AS Integer = __canvas_transparencyStep(paint)
  out = collections::set(out, 11, toFloat(__canvas_fadeAlpha(toInt(paint.fill.alpha), fade)))
  out = collections::set(out, 12, toFloat(toInt(paint.stroke.red)))
  out = collections::set(out, 13, toFloat(toInt(paint.stroke.green)))
  out = collections::set(out, 14, toFloat(toInt(paint.stroke.blue)))
  out = collections::set(out, 15, toFloat(__canvas_fadeAlpha(toInt(paint.stroke.alpha), fade)))
  ' The blend mode as its tag. Compared variant by variant rather than converted,
  ' because Normal must land as 0 -- the zero value being the no-op is the rule the
  ' whole of Paint follows, and it is what keeps every pre-plan-116-B scene rendering
  ' exactly as it did.
  MUT blend AS Float = 0.0
  IF paint.blend = BlendMode.Multiply THEN
    blend = 1.0
  END IF
  IF paint.blend = BlendMode.Screen THEN
    blend = 2.0
  END IF
  IF paint.blend = BlendMode.Add THEN
    blend = 3.0
  END IF
  out = collections::set(out, 26, blend)
  ' plan-116-F: the gradient's scalars. The STOPS live in the tail -- these six slots
  ' are what the renderer needs before it walks them, and the count doubles as the
  ' has-a-gradient test: fewer than two stops is no gradient, so one comparison here
  ' decides it rather than a flag that could disagree with the list.
  ' A kind with no interior takes no gradient, and there are FOUR of them: `Text`
  ' draws from a cached coverage bitmap, `NONE` draws nothing, and a `Line` and an
  ' `Arc` are drawn entirely from `Paint.stroke` -- `mfb spec app canvas` says so in
  ' the same words ("both are drawn entirely from `Paint.stroke` and have no
  ' interior"), which is also why those two are the only variants carrying a `cap`.
  '
  ' Skipping them here is what keeps the stop tail off records whose tail means
  ' something else -- a glyph run's is three floats per glyph, not five per stop.
  '
  ' The segment and arc half of this was missed when the rule was written (plan-116-F
  ' Correction F17) and it was not merely redundant: `__canvas_hashItem` returns a bare
  ' `acc` for `CASE Line` and `CASE Arc`, and `__canvas_tailMatches` returns `TRUE` for
  ' both, so stops stored on those kinds were never part of the cache key. Two `Line`s
  ' differing only in their gradient shared an entry and the second drew the first's
  ' stops -- the same collision the polygon points guard against below. Skipping is what
  ' makes those two bare returns correct by construction rather than by luck.
  LET kind AS Integer = toInt(collections::getOr(out, 0, 0.0))
  MUT stopCount AS Integer = len(paint.gradient.stops)
  IF kind = __CANVAS_GEO_TEXT THEN
    stopCount = 0
  END IF
  IF kind = __CANVAS_GEO_NONE THEN
    stopCount = 0
  END IF
  ' plan-164-B: a `PolyCurve` is the third kind drawn entirely from its stroke.
  LET strokeOnly AS Boolean = kind = __CANVAS_KIND_SEGMENT OR kind = __CANVAS_GEO_ARC OR kind = __CANVAS_GEO_POLYCURVE
  IF strokeOnly THEN
    stopCount = 0
  END IF
  ' plan-163-D: unless the ramp paints the stroke (`FillAndStroke`): a Line or Arc is
  ' then drawn in it, so it keeps the stops -- `__canvas_strokeRampTail` appends them.
  IF strokeOnly AND paint.gradient.strokeApply = GradientApply.FillAndStroke THEN
    stopCount = len(paint.gradient.stops)
  END IF
  ' plan-154-A: a PolyLine is stroke-as-fill too. Its tail does carry stops -- its
  ' per-point colours, never `gradient`'s -- and `__canvas_polylineHeader` writes
  ' their count itself after this returns.
  IF kind = __CANVAS_GEO_POLYLINE THEN
    stopCount = 0
  END IF
  ' Fewer than two stops is not a gradient (plan-116-F section 4.1), so it carries no
  ' tail either -- one comparison decides both.
  IF stopCount < 2 THEN
    stopCount = 0
  END IF
  out = collections::set(out, 1, collections::getOr(out, 1, 0.0) + toFloat(stopCount * 5))
  out = collections::set(out, __CANVAS_GEO_GRADIENT_COUNT, toFloat(stopCount))
  MUT gkind AS Float = 0.0
  IF paint.gradient.kind = GradientKind.Radial THEN
    gkind = 1.0
  END IF
  out = collections::set(out, __CANVAS_GEO_GRADIENT_KIND, gkind)
  out = collections::set(out, __CANVAS_GEO_GRADIENT_FROMX, paint.gradient.startPoint.x)
  out = collections::set(out, __CANVAS_GEO_GRADIENT_FROMY, paint.gradient.startPoint.y)
  out = collections::set(out, __CANVAS_GEO_GRADIENT_TOX, paint.gradient.endPoint.x)
  out = collections::set(out, __CANVAS_GEO_GRADIENT_TOY, paint.gradient.endPoint.y)
  IF stopCount >= 2 THEN
    out = __canvas_gradientHeader(out, paint, kind)
  END IF
  ' plan-116-C: the transform, INVERTED once here rather than per pixel in each of
  ' three renderers. `__canvas_invertTransform` is also the only place that knows an
  ' all-zero `Transform` means the identity, and the only place that decides what a
  ' singular one does.
  '
  ' bug-686: the all-zero transform -- every item that never set one -- is written as
  ' the identity's two non-zero bit patterns (1.0f is 1065353216) into slots that are
  ' already zero, rather than inverted into a fresh seven-float list and copied. Exactly
  ' the values `__canvas_invertTransform` returns for it.
  LET t AS Transform = transform
  IF t.a = 0.0 AND t.b = 0.0 AND t.c = 0.0 AND t.d = 0.0 AND t.tx = 0.0 AND t.ty = 0.0 THEN
    out = collections::set(out, 27, 1065353216.0)
    out = collections::set(out, 30, 1065353216.0)
    RETURN __canvas_clipHeader(out, paint)
  END IF
  LET inv AS List OF Float = __canvas_invertTransform(t)
  MUT ti AS Integer = 0
  WHILE ti < 7
    out = collections::set(out, 27 + ti, collections::getOr(inv, ti, 0.0))
    ti = ti + 1
  END WHILE
  RETURN __canvas_clipHeader(out, paint)
END FUNC

' plan-163-C: the transparency step T, `round(clamp(transparency, 0, 1) * 255)` -- a NaN
' clamps to 1.0 (`__canvas_minF` answers its second argument), so it hides the item.
FUNC __canvas_transparencyStep(paint AS Paint) AS Integer
  LET t AS Float = __canvas_maxF(__canvas_minF(paint.transparency, 1.0), 0.0)
  RETURN toInt(t * 255.0 + 0.5)
END FUNC

' An alpha faded by the transparency step T: `(alpha * (255 - T) + 127) / 255` in
' integers, which is `alpha` itself at T = 0.
FUNC __canvas_fadeAlpha(alpha AS Integer, fade AS Integer) AS Integer
  RETURN (alpha * (255 - fade) + 127) / 255
END FUNC

' plan-163-D: a gradient's transform inverted in f64, `[ia, ib, ic, id, itx, ity, 1]`
' with `x' = ia*x + ic*y + itx`; the identity `[1, 0, 0, 1, 0, 0, 0]` for the all-zero
' transform and for one `__canvas_invertTransform` would refuse (a determinant under
' 1e-12) -- that helper's rule, kept in f64 for the linear fold.
FUNC __canvas_gradientInverse(t AS Transform) AS List OF Float
  LET identity AS List OF Float = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0]
  IF t.a = 0.0 AND t.b = 0.0 AND t.c = 0.0 AND t.d = 0.0 AND t.tx = 0.0 AND t.ty = 0.0 THEN
    RETURN identity
  END IF
  LET det AS Float = t.a * t.d - t.b * t.c
  IF __canvas_absF(det) < 0.000000000001 THEN
    RETURN identity
  END IF
  LET ia AS Float = t.d / det
  LET ib AS Float = (0.0 - t.b) / det
  LET ic AS Float = (0.0 - t.c) / det
  LET id AS Float = t.a / det
  RETURN [ia, ib, ic, id, 0.0 - (ia * t.tx + ic * t.ty), 0.0 - (ib * t.tx + id * t.ty), 1.0]
END FUNC

' plan-163-D: the gradient's spread, stroke apply, transform and focal slots, for a paint
' whose ramp the record carries (two or more stops on a kind that draws one).
'
' A LINEAR gradient's transform G folds into its two points: its `t = dot(G^-1 q - s, d)
' / |d|^2` is affine in the item point q, `t = u.q + c`, which is the plain linear ramp
' from `-c u / |u|^2` along `u / |u|^2`. So the renderers see an ordinary linear
' gradient. A RADIAL one cannot fold (a transformed circle is an ellipse), so its inverse
' rides slots 57-62 and the renderers map q through it per pixel. The focal offset is
' pulled inside `(r - fr)(1 - 2^-10)` of the centre and the focal radius into `[0,
' r (1 - 2^-10)]`, which keeps the focal quadratic's leading coefficient below zero.
' The identity transform, no focal offset and a zero focal radius leave every slot at
' zero, so such a gradient's record is the one it was before this letter.
FUNC __canvas_gradientHeader(h AS List OF Float, paint AS Paint, kind AS Integer) AS List OF Float
  LET g AS Gradient = paint.gradient
  MUT out AS List OF Float = h
  IF g.spread = GradientSpread.Reflect THEN
    out = collections::set(out, __CANVAS_GEO_GRADIENT_SPREAD, 1.0)
  END IF
  IF g.spread = GradientSpread.Repeat THEN
    out = collections::set(out, __CANVAS_GEO_GRADIENT_SPREAD, 2.0)
  END IF
  ' Under FillAndStroke the stroke alpha (slot 15, and a Line's or Arc's fill slot 11
  ' after `__canvas_strokeAsFill`) is the tint's strength, so it keeps the RAW alpha: the
  ' ramp's own stops carry the transparency fade, as a picture's texels do.
  IF g.strokeApply = GradientApply.FillAndStroke AND kind <> __CANVAS_GEO_PICTURE THEN
    out = collections::set(out, __CANVAS_GEO_GRADIENT_APPLY, 1.0)
    out = collections::set(out, 15, toFloat(toInt(paint.stroke.alpha)))
  END IF
  LET inv AS List OF Float = __canvas_gradientInverse(g.transform)
  LET hasInv AS Boolean = collections::getOr(inv, 6, 0.0) > 0.5
  LET sx AS Float = g.startPoint.x
  LET sy AS Float = g.startPoint.y
  LET dx AS Float = g.endPoint.x - sx
  LET dy AS Float = g.endPoint.y - sy
  IF g.kind = GradientKind.Linear THEN
    LET len2 AS Float = dx * dx + dy * dy
    IF hasInv AND len2 > 0.0 THEN
      LET ia AS Float = collections::getOr(inv, 0, 1.0)
      LET ib AS Float = collections::getOr(inv, 1, 0.0)
      LET ic AS Float = collections::getOr(inv, 2, 0.0)
      LET id AS Float = collections::getOr(inv, 3, 1.0)
      LET itx AS Float = collections::getOr(inv, 4, 0.0)
      LET ity AS Float = collections::getOr(inv, 5, 0.0)
      LET ux AS Float = (dx * ia + dy * ib) / len2
      LET uy AS Float = (dx * ic + dy * id) / len2
      LET c AS Float = (dx * (itx - sx) + dy * (ity - sy)) / len2
      LET u2 AS Float = ux * ux + uy * uy
      IF u2 > 0.0 THEN
        LET fromX AS Float = (0.0 - c) * ux / u2
        LET fromY AS Float = (0.0 - c) * uy / u2
        out = collections::set(out, __CANVAS_GEO_GRADIENT_FROMX, fromX)
        out = collections::set(out, __CANVAS_GEO_GRADIENT_FROMY, fromY)
        out = collections::set(out, __CANVAS_GEO_GRADIENT_TOX, fromX + ux / u2)
        out = collections::set(out, __CANVAS_GEO_GRADIENT_TOY, fromY + uy / u2)
      END IF
    END IF
    RETURN out
  END IF
  IF hasInv THEN
    MUT k AS Integer = 0
    WHILE k < 6
      out = collections::set(out, __CANVAS_GEO_GRADIENT_INV + k, toFloat(__canvas_float32Bits(collections::getOr(inv, k, 0.0))))
      k = k + 1
    END WHILE
    out = collections::set(out, __CANVAS_GEO_GRADIENT_HAS_INV, 1.0)
  END IF
  LET r AS Float = math::sqrt(dx * dx + dy * dy)
  LET frMax AS Float = r * 0.9990234375
  LET fr AS Float = __canvas_maxF(__canvas_minF(g.focalRadius, frMax), 0.0)
  MUT ox AS Float = g.focalOffset.x
  MUT oy AS Float = g.focalOffset.y
  LET reach AS Float = (r - fr) * 0.9990234375
  LET olen AS Float = math::sqrt(ox * ox + oy * oy)
  IF olen > reach THEN
    IF olen > 0.0 THEN
      ox = ox * reach / olen
      oy = oy * reach / olen
    END IF
  END IF
  out = collections::set(out, __CANVAS_GEO_GRADIENT_FOCALX, ox)
  out = collections::set(out, __CANVAS_GEO_GRADIENT_FOCALY, oy)
  RETURN collections::set(out, __CANVAS_GEO_GRADIENT_FOCALR, fr)
END FUNC

' plan-163-D: a Line's or Arc's tail with the ramp's stops after it when the ramp paints
' the stroke (`FillAndStroke`) -- the only time `__canvas_paintHeader` counts them for
' those two kinds.
FUNC __canvas_strokeRampTail(base AS List OF Float, paint AS Paint) AS List OF Float
  IF paint.gradient.strokeApply = GradientApply.FillAndStroke THEN
    RETURN __canvas_appendGradientTail(base, paint)
  END IF
  RETURN base
END FUNC

' A clip of one ring of four points that is an axis-aligned rectangle (its sides
' alternate horizontal and vertical, from either), as [x0, y0, x1, y1]; [] for any other
' clip.
FUNC __canvas_clipRect(rings AS List OF List OF vector::Float2) AS List OF Float
  IF len(rings) <> 1 THEN
    RETURN []
  END IF
  LET origin AS vector::Float2 = vector::Float2[x := 0.0, y := 0.0]
  LET none AS List OF vector::Float2 = []
  LET ring AS List OF vector::Float2 = collections::getOr(rings, 0, none)
  IF len(ring) <> 4 THEN
    RETURN []
  END IF
  LET a AS vector::Float2 = collections::getOr(ring, 0, origin)
  LET b AS vector::Float2 = collections::getOr(ring, 1, origin)
  LET c AS vector::Float2 = collections::getOr(ring, 2, origin)
  LET d AS vector::Float2 = collections::getOr(ring, 3, origin)
  LET across AS Boolean = a.y = b.y AND b.x = c.x AND c.y = d.y AND d.x = a.x
  LET down AS Boolean = a.x = b.x AND b.y = c.y AND c.x = d.x AND d.y = a.y
  IF NOT across AND NOT down THEN
    RETURN []
  END IF
  RETURN [__canvas_minF(a.x, c.x), __canvas_minF(a.y, c.y), __canvas_maxF(a.x, c.x), __canvas_maxF(a.y, c.y)]
END FUNC

' plan-163-C: the clip's header slots. The empty ring list is no clip (mode 0, every slot
' left zero). One axis-aligned rectangular ring on an item with no transform is mode 1,
' the rectangle in slots 22-25 in the item's own coordinates -- a zero-area one hides
' the item. Every other clip is mode 2, the PATH clip: its edges ride the record's tail
' (`__canvas_withClipEdges`), slot 52 counts them, and slots 22-25 hold the SURFACE
' bounds of its rings mapped by the item's transform, a pixel wider each way, which
' narrow the pixels drawn. A path clip whose rings contribute no edge encloses nothing:
' it keeps zero bounds and hides the item.
FUNC __canvas_clipHeader(h AS List OF Float, paint AS Paint) AS List OF Float
  IF len(paint.clip) = 0 THEN
    RETURN h
  END IF
  MUT out AS List OF Float = h
  IF paint.clipRule = FillRule.NonZero THEN
    out = collections::set(out, __CANVAS_GEO_CLIP_RULE, 1.0)
  END IF
  LET mapped AS Boolean = collections::getOr(h, 33, 0.0) > 0.5
  IF NOT mapped THEN
    LET r AS List OF Float = __canvas_clipRect(paint.clip)
    IF len(r) > 0 THEN
      out = collections::set(out, 22, collections::getOr(r, 0, 0.0))
      out = collections::set(out, 23, collections::getOr(r, 1, 0.0))
      out = collections::set(out, 24, collections::getOr(r, 2, 0.0))
      out = collections::set(out, 25, collections::getOr(r, 3, 0.0))
      RETURN collections::set(out, __CANVAS_GEO_CLIP_MODE, 1.0)
    END IF
  END IF
  out = collections::set(out, __CANVAS_GEO_CLIP_MODE, 2.0)
  MUT fa AS Float = 1.0
  MUT fb AS Float = 0.0
  MUT fc AS Float = 0.0
  MUT fd AS Float = 1.0
  MUT ftx AS Float = 0.0
  MUT fty AS Float = 0.0
  IF mapped THEN
    LET fwd AS List OF Float = __canvas_forwardOf(__canvas_f32FromBits(collections::getOr(h, 27, 0.0)), __canvas_f32FromBits(collections::getOr(h, 28, 0.0)), __canvas_f32FromBits(collections::getOr(h, 29, 0.0)), __canvas_f32FromBits(collections::getOr(h, 30, 0.0)), __canvas_f32FromBits(collections::getOr(h, 31, 0.0)), __canvas_f32FromBits(collections::getOr(h, 32, 0.0)))
    fa = collections::getOr(fwd, 0, 1.0)
    fb = collections::getOr(fwd, 1, 0.0)
    fc = collections::getOr(fwd, 2, 0.0)
    fd = collections::getOr(fwd, 3, 1.0)
    ftx = collections::getOr(fwd, 4, 0.0)
    fty = collections::getOr(fwd, 5, 0.0)
  END IF
  MUT count AS Integer = 0
  MUT minX AS Float = 0.0
  MUT minY AS Float = 0.0
  MUT maxX AS Float = 0.0
  MUT maxY AS Float = 0.0
  FOR EACH ring IN paint.clip
    IF len(ring) >= 2 THEN
      FOR EACH q IN ring
        MUT x AS Float = q.x
        MUT y AS Float = q.y
        IF mapped THEN
          x = fa * q.x + fc * q.y + ftx
          y = fb * q.x + fd * q.y + fty
        END IF
        IF count = 0 THEN
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
        count = count + 1
      NEXT
    END IF
  NEXT
  IF count = 0 THEN
    RETURN out
  END IF
  out = collections::set(out, __CANVAS_GEO_CLIP_EDGE_COUNT, toFloat(count))
  out = collections::set(out, 22, minX - 1.0)
  out = collections::set(out, 23, minY - 1.0)
  out = collections::set(out, 24, maxX + 1.0)
  RETURN collections::set(out, 25, maxY + 1.0)
END FUNC

' plan-163-C: an item's clip rings, whatever its kind; none for a group.
FUNC __canvas_clipOf(item AS DrawItem) AS List OF List OF vector::Float2
  MATCH item
    CASE Rectangle(r)
      RETURN __canvas_placedClip(r.paint.clip, __canvas_buildX(r.position, r.transform), __canvas_buildY(r.position, r.transform))
    CASE Circle(c)
      RETURN __canvas_placedClip(c.paint.clip, __canvas_buildX(c.position, c.transform), __canvas_buildY(c.position, c.transform))
    CASE Line(l)
      RETURN __canvas_placedClip(l.paint.clip, __canvas_buildX(l.position, l.transform), __canvas_buildY(l.position, l.transform))
    CASE Arc(a)
      RETURN __canvas_placedClip(a.paint.clip, __canvas_buildX(a.position, a.transform), __canvas_buildY(a.position, a.transform))
    CASE Polygon(p)
      RETURN __canvas_placedClip(p.paint.clip, __canvas_buildX(p.position, p.transform), __canvas_buildY(p.position, p.transform))
    ' plan-172-L: a picture's clip is its shape's, and is written into its records
    ' where they are built (`helper_picture.rs`); it never reaches this.
    CASE Picture(pic)
      RETURN []
    CASE Tile(tile)
      RETURN []
    CASE Ellipse(e)
      RETURN __canvas_placedClip(e.paint.clip, __canvas_buildX(e.position, e.transform), __canvas_buildY(e.position, e.transform))
    CASE Text(t)
      RETURN __canvas_placedClip(t.paint.clip, __canvas_buildX(t.position, t.transform), __canvas_buildY(t.position, t.transform))
    CASE PolyLine(pl)
      RETURN __canvas_placedClip(pl.paint.clip, __canvas_buildX(pl.position, pl.transform), __canvas_buildY(pl.position, pl.transform))
    CASE PolyCurve(pc)
      RETURN __canvas_placedClip(pc.paint.clip, __canvas_buildX(pc.position, pc.transform), __canvas_buildY(pc.position, pc.transform))
    CASE CurvePolygon(cp)
      RETURN __canvas_placedClip(cp.paint.clip, __canvas_buildX(cp.position, cp.transform), __canvas_buildY(cp.position, cp.transform))
    CASE Group(g)
      RETURN []
    ' plan-150-A: nor has a particle system -- its particles are items of their own.
    CASE ParticleSystem(ps)
      RETURN []
    CASE AutoPaint(a)
      RETURN __canvas_clipOf(__canvas_autoItem(a))
  END MATCH
  RETURN []
END FUNC

' plan-163-C: a path clip's edges into a record's tail, right before its gradient stops
' (which stay last): edges | joins | dashes | clip edges | stops. The header has counted
' them (slot 52); this also grows slot 1 by their floats. Every other record is returned
' untouched.
FUNC __canvas_withClipEdges(header AS List OF Float, tail AS List OF Float, item AS DrawItem) AS List OF List OF Float
  LET count AS Integer = toInt(collections::getOr(header, __CANVAS_GEO_CLIP_EDGE_COUNT, 0.0))
  IF count = 0 THEN
    RETURN [header, tail]
  END IF
  LET stops AS Integer = toInt(collections::getOr(header, __CANVAS_GEO_GRADIENT_COUNT, 0.0)) * 5
  LET keep AS Integer = len(tail) - stops
  LET edges AS List OF Float = __canvas_polygonEdges(__canvas_clipOf(item))
  MUT out AS List OF Float = []
  MUT i AS Integer = 0
  WHILE i < keep
    out = collections::append(out, collections::getOr(tail, i, 0.0))
    i = i + 1
  END WHILE
  out = collections::append(out, edges)
  WHILE i < len(tail)
    out = collections::append(out, collections::getOr(tail, i, 0.0))
    i = i + 1
  END WHILE
  RETURN [collections::set(header, 1, collections::getOr(header, 1, 0.0) + toFloat(len(edges))), out]
END FUNC

"#
    };
}
pub(super) use geo_header_paint;
