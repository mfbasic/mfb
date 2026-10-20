//! plan-176-R: `helper_geometry`'s MFBASIC helper text — the geometry tail builders
//! (`GEO_TAIL`). Registration lives in `builtins/canvas/helper_geometry/mod.rs`.

/// The per-kind tail: the work the cache exists to skip.
///
/// Only a polygon has one. Each edge is stored as `x0, y0, dx, dy, invLenSq`, which
/// is exactly what the per-pixel segment-distance query needs — so the query reads
/// five floats instead of recomputing the edge vector and its length for every pixel
/// of every frame. A degenerate (zero-length) edge stores `invLenSq = 0`, which makes
/// the projection parameter clamp to the endpoint rather than dividing by zero.
#[rustfmt::skip]
pub(super) const GEO_TAIL: &str =
r#"' plan-163-A: every contributing ring's edges, one ring after another, each ring closing
' from its own last point back to its own first. No separator and no ring count: the
' fill sums crossings over all edges and the stroke is a min over all of them, so
' neither needs to know where a ring ends (the stroked-text precedent,
' `__canvas_textEdges`). A ring of fewer than 2 points adds nothing, as the header says.
FUNC __canvas_polygonEdges(rings AS List OF List OF vector::Float2) AS List OF Float
  MUT out AS List OF Float = []
  LET origin AS vector::Float2 = vector::Float2[x := 0.0, y := 0.0]
  FOR EACH points IN rings
    LET count AS Integer = len(points)
    IF count >= 2 THEN
      MUT i AS Integer = 0
      WHILE i < count
        LET a AS vector::Float2 = collections::getOr(points, i, origin)
        LET b AS vector::Float2 = collections::getOr(points, (i + 1) MOD count, origin)
        LET dx AS Float = b.x - a.x
        LET dy AS Float = b.y - a.y
        LET lenSq AS Float = dx * dx + dy * dy
        out = collections::append(out, a.x)
        out = collections::append(out, a.y)
        out = collections::append(out, dx)
        out = collections::append(out, dy)
        IF lenSq > 0.0 THEN
          out = collections::append(out, 1.0 / lenSq)
        ELSE
          out = collections::append(out, 0.0)
        END IF
        i = i + 1
      END WHILE
    END IF
  NEXT
  RETURN out
END FUNC

' plan-154-A: a PolyLine's tail -- its segments in the polygon edge layout, WITHOUT the
' wrap edge a polygon closes with, then (when it has colours) one gradient stop per
' point.
'
' The stops are what lets both GPUs draw the colours with no new region: they upload a
' record's stops to the frame's gradient region already. Point k's stop sits at offset
' k / (count - 1), and the renderer asks `__canvas_gradientColor` for the colour at the
' path parameter (segment + t) / (count - 1), which falls between the stops of that
' segment's two points -- so the colour is the gradient mix of those two, in linear
' light, exactly as a `Paint.gradient` ramp mixes.
FUNC __canvas_polylineTail(p AS PolyLine) AS List OF Float
  LET segments AS List OF Float = __canvas_polylineSegments(p)
  ' plan-163-B: the join records go between the segments and the colour stops, which
  ' stay last.
  LET joins AS List OF Float = __canvas_joinTail(segments, [0], FALSE, __canvas_polylineHalf(p), p.paint)
  LET dashes AS List OF Float = __canvas_dashTail(__canvas_dashPattern(p.paint, __canvas_polylineHalf(p)), __canvas_dashEdgeData(segments, [0], FALSE))
  RETURN collections::append(collections::append(collections::append(segments, joins), dashes), __canvas_polylineStops(p))
END FUNC

' The join records a polyline's tail holds (`__canvas_polylineTail`), one per segment.
FUNC __canvas_polylineJoins(p AS PolyLine) AS List OF Float
  RETURN __canvas_joinTail(__canvas_polylineSegments(p), [0], FALSE, __canvas_polylineHalf(p), p.paint)
END FUNC

' The dash tail a polyline's tail holds (`__canvas_polylineTail`).
FUNC __canvas_polylineDashes(p AS PolyLine) AS List OF Float
  RETURN __canvas_dashTail(__canvas_dashPattern(p.paint, __canvas_polylineHalf(p)), __canvas_dashEdgeData(__canvas_polylineSegments(p), [0], FALSE))
END FUNC

' plan-172-F: a path with its points in the coordinates its record is built in. They
' are offsets from `position`; built at its position (no transform in effect) each is
' `position + point`, added whatever the position, for the reason
' `__canvas_placedRings` gives. Every polyline builder below reads `p.points`, so the
' path is moved once here and they are unchanged. A turned path is built at the local
' origin and returned as it is.
FUNC __canvas_polylinePlaced(p AS PolyLine) AS PolyLine
  IF __canvas_transformActive(p.transform) THEN
    RETURN p
  END IF
  MUT moved AS List OF vector::Float2 = []
  FOR EACH q IN p.points
    moved = collections::append(moved, vector::Float2[x := q.x + p.position.x, y := q.y + p.position.y])
  NEXT
  RETURN WITH p { points := moved }
END FUNC

FUNC __canvas_polylineSegments(p AS PolyLine) AS List OF Float
  LET count AS Integer = len(p.points)
  MUT out AS List OF Float = []
  IF count < 2 THEN
    RETURN out
  END IF
  LET origin AS vector::Float2 = vector::Float2[x := 0.0, y := 0.0]
  MUT i AS Integer = 0
  WHILE i + 1 < count
    LET a AS vector::Float2 = collections::getOr(p.points, i, origin)
    LET b AS vector::Float2 = collections::getOr(p.points, i + 1, origin)
    LET dx AS Float = b.x - a.x
    LET dy AS Float = b.y - a.y
    LET lenSq AS Float = dx * dx + dy * dy
    out = collections::append(out, a.x)
    out = collections::append(out, a.y)
    out = collections::append(out, dx)
    out = collections::append(out, dy)
    IF lenSq > 0.0 THEN
      out = collections::append(out, 1.0 / lenSq)
    ELSE
      out = collections::append(out, 0.0)
    END IF
    i = i + 1
  END WHILE
  RETURN out
END FUNC

FUNC __canvas_polylineStops(p AS PolyLine) AS List OF Float
  LET count AS Integer = len(p.points)
  MUT out AS List OF Float = []
  LET given AS Integer = len(p.colors)
  IF given = 0 OR count < 2 THEN
    RETURN out
  END IF
  LET segs AS Float = toFloat(count - 1)
  ' plan-163-C: each colour's alpha faded as the stroke's is.
  LET fade AS Integer = __canvas_transparencyStep(p.paint)
  MUT k AS Integer = 0
  WHILE k < count
    MUT c AS color::Color = p.paint.stroke
    IF k < given THEN
      c = collections::getOr(p.colors, k, p.paint.stroke)
    END IF
    out = collections::append(out, toFloat(k) / segs)
    out = collections::append(out, toFloat(toInt(c.red)))
    out = collections::append(out, toFloat(toInt(c.green)))
    out = collections::append(out, toFloat(toInt(c.blue)))
    out = collections::append(out, toFloat(__canvas_fadeAlpha(toInt(c.alpha), fade)))
    k = k + 1
  END WHILE
  RETURN out
END FUNC

' The five floats one stop contributes: offset, then its colour's four channels.
FUNC __canvas_gradientTail(paint AS Paint) AS List OF Float
  LET count AS Integer = len(paint.gradient.stops)
  MUT out AS List OF Float = []
  IF count < 2 THEN
    RETURN out
  END IF
  ' plan-163-C: each stop's alpha faded as the fill's is.
  LET fade AS Integer = __canvas_transparencyStep(paint)
  ' Offsets are clamped to 0..1 AND to the previous stop's, so the walk is monotonic
  ' without sorting -- sorting would silently redraw something other than what the
  ' program asked for (plan-116-F section 4.1). Clamping is visible and predictable.
  MUT prev AS Float = 0.0
  MUT i AS Integer = 0
  WHILE i < count
    LET st AS GradientStop = collections::getOr(paint.gradient.stops, i, GradientStop[offset := 0.0, color := __canvas_transparent()])
    MUT o AS Float = st.offset
    IF o < 0.0 THEN
      o = 0.0
    END IF
    IF o > 1.0 THEN
      o = 1.0
    END IF
    IF o < prev THEN
      o = prev
    END IF
    prev = o
    out = collections::append(out, o)
    out = collections::append(out, toFloat(toInt(st.color.red)))
    out = collections::append(out, toFloat(toInt(st.color.green)))
    out = collections::append(out, toFloat(toInt(st.color.blue)))
    out = collections::append(out, toFloat(__canvas_fadeAlpha(toInt(st.color.alpha), fade)))
    i = i + 1
  END WHILE
  RETURN out
END FUNC

FUNC __canvas_appendGradientTail(base AS List OF Float, paint AS Paint) AS List OF Float
  MUT out AS List OF Float = base
  LET stops AS List OF Float = __canvas_gradientTail(paint)
  MUT i AS Integer = 0
  LET n AS Integer = len(stops)
  WHILE i < n
    out = collections::append(out, collections::getOr(stops, i, 0.0))
    i = i + 1
  END WHILE
  RETURN out
END FUNC

FUNC __canvas_textGlyphRun(t AS Text) AS List OF Float
  LET handles AS List OF Integer = __canvas_textHandles(t)
  LET scales AS List OF Float = __canvas_fontScales(handles, t.fontSize)
  ' Shaped once, before pass one. The run is a plain local list of glyph ids and
  ' advances, and it holds no cache index, so the eviction pass one can trigger -- which
  ' renumbers entries, see below -- cannot make it stale.
  LET shaped AS List OF Integer = __canvas_shapeRun(handles, t.text, t.letterSpacing)
  IF NOT __canvas_runDrawable(shaped, scales) THEN
    RETURN []
  END IF
  LET chars AS Integer = len(shaped) / __CANVAS_RUN_STRIDE

  ' Pass one rasterises, recording each entry in the GLOBAL pin list rather than in a
  ' local. A run being built is not yet in the geometry cache, so the pin scan cannot
  ' see it: without this list, the eleventh glyph of a string could evict the first ten
  ' -- glyphs the very item under construction is about to draw -- and the run would
  ' carry indices to entries that no longer exist. It went further than losing them:
  ' eviction renumbers survivors, so indices already copied into a local were stale
  ' whether or not their glyph was dropped. The list is global precisely so
  ' `__canvas_glyphEvict` can pin AND renumber it.
  __CANVAS_GLYPH_PINS = []
  ' The outlines `__canvas_glyphEntry` rasterises on a miss are read from the font file
  ' in place, by handle (bug-1091): a build that hits the cache for every glyph reads no
  ' byte of the file, and one that misses reads only that glyph's `loca` and `glyf`.
  MUT c AS Integer = 0
  WHILE c < chars
    LET which AS Integer = collections::getOr(shaped, c * __CANVAS_RUN_STRIDE + __CANVAS_RUN_FONT, 0)
    LET font AS Integer = collections::getOr(handles, which, 0)
    LET scale AS Float = collections::getOr(scales, which, 0.0)
    LET gid AS Integer = collections::getOr(shaped, c * __CANVAS_RUN_STRIDE + __CANVAS_RUN_GLYPH, 0)
    ' Rasterise here, at cache-fill time, not at draw time. The draw path owns a live
    ' 2.3 MB surface local and `collections::set` is in-place only while nothing else
    ' allocates underneath it, so every allocation belongs on this side of the seam --
    ' which is also the side that already runs once per changed item rather than once
    ' per frame.
    ' The entry lands in a local FIRST. `__canvas_glyphEntry` can run an eviction pass,
    ' and an eviction pass reassigns `__CANVAS_GLYPH_PINS` -- so writing
    ' `append(__CANVAS_GLYPH_PINS, __canvas_glyphEntry(...))` appends to whichever list
    ' the argument evaluation had already resolved, which is the one eviction just
    ' replaced.
    ' A glyph from a font with no scale -- one with no `head` -- has no outline to
    ' draw and is no entry; it stays -1 in the pin list, which eviction passes over.
    MUT entry AS Integer = 0 - 1
    IF scale > 0.0 THEN
      entry = __canvas_glyphEntry(font, gid, t.fontSize, scale)
    END IF
    __CANVAS_GLYPH_PINS = collections::append(__CANVAS_GLYPH_PINS, entry)
    c = c + 1
  END WHILE

  ' Pass two reads the entries back, after any renumbering. A glyph with no entry
  ' contributes no triple (and, at scale 0, no advance).
  MUT run AS List OF Float = []
  MUT pen AS Float = __canvas_textX(t)
  c = 0
  WHILE c < chars
    LET scale AS Float = collections::getOr(scales, collections::getOr(shaped, c * __CANVAS_RUN_STRIDE + __CANVAS_RUN_FONT, 0), 0.0)
    IF scale > 0.0 THEN
      run = collections::append(run, toFloat(collections::getOr(__CANVAS_GLYPH_PINS, c, 0 - 1)))
      run = collections::append(run, toFloat(toInt(pen + 0.5)))
      run = collections::append(run, toFloat(toInt(__canvas_textY(t) + 0.5)))
    END IF
    pen = pen + toFloat(collections::getOr(shaped, c * __CANVAS_RUN_STRIDE + __CANVAS_RUN_ADVANCE, 0)) * scale
    IF t.letterSpacing <> 0.0 AND collections::getOr(shaped, c * __CANVAS_RUN_STRIDE + __CANVAS_RUN_GAP, 0) = 1 THEN
      pen = pen + t.letterSpacing
    END IF
    c = c + 1
  END WHILE
  RETURN run
END FUNC

' A `Text`'s fonts as handles, in fallback order: 0 for a closed one, which reads as no
' font at all (`canvas::fontHandle`).
FUNC __canvas_textHandles(t AS Text) AS List OF Integer
  MUT out AS List OF Integer = []
  FOR EACH f IN t.fonts
    out = collections::append(out, canvas::fontHandle(f))
  NEXT
  RETURN out
END FUNC

' Each font's scale at `size`, `size / unitsPerEm`: 0.0 for one that cannot be drawn --
' closed (its face index reads 0) or with no `head`.
FUNC __canvas_fontScales(handles AS List OF Integer, size AS Float) AS List OF Float
  MUT out AS List OF Float = []
  FOR EACH h IN handles
    LET upem AS Integer = __canvas_faceMetric(h, __CANVAS_FACE_UPEM)
    IF upem > 0 THEN
      out = collections::append(out, size / toFloat(upem))
    ELSE
      out = collections::append(out, 0.0)
    END IF
  NEXT
  RETURN out
END FUNC

' Does the shaped run draw anything: a glyph from a font with a scale? A run with none
' -- no font open, or only fonts with no `head` -- is no record at all.
FUNC __canvas_runDrawable(shaped AS List OF Integer, scales AS List OF Float) AS Boolean
  MUT c AS Integer = 0
  WHILE c * __CANVAS_RUN_STRIDE < len(shaped)
    IF collections::getOr(scales, collections::getOr(shaped, c * __CANVAS_RUN_STRIDE + __CANVAS_RUN_FONT, 0), 0.0) > 0.0 THEN
      RETURN TRUE
    END IF
    c = c + 1
  END WHILE
  RETURN FALSE
END FUNC

' The line a shaped run sits on, `[ascent, descent, found]` in pixels (descent positive,
' `found` 1.0 or 0.0): the largest ascent and descent of the drawable fonts that
' supplied a glyph -- `measureText`'s rule for a list -- or, for an empty run, the first
' drawable font's. With one font that is that font's own line.
FUNC __canvas_runLine(handles AS List OF Integer, scales AS List OF Float, shaped AS List OF Integer) AS List OF Float
  MUT used AS List OF Boolean = []
  FOR EACH h IN handles
    used = collections::append(used, FALSE)
  NEXT
  MUT c AS Integer = 0
  WHILE c * __CANVAS_RUN_STRIDE < len(shaped)
    used = collections::set(used, collections::getOr(shaped, c * __CANVAS_RUN_STRIDE + __CANVAS_RUN_FONT, 0), TRUE)
    c = c + 1
  END WHILE
  IF len(shaped) = 0 THEN
    MUT k AS Integer = 0
    WHILE k < len(handles)
      IF collections::getOr(scales, k, 0.0) > 0.0 THEN
        used = collections::set(used, k, TRUE)
        k = len(handles)
      END IF
      k = k + 1
    END WHILE
  END IF
  MUT ascent AS Float = 0.0
  MUT descent AS Float = 0.0
  MUT found AS Boolean = FALSE
  MUT k AS Integer = 0
  WHILE k < len(handles)
    LET scale AS Float = collections::getOr(scales, k, 0.0)
    IF scale > 0.0 AND collections::getOr(used, k, FALSE) THEN
      LET h AS Integer = collections::getOr(handles, k, 0)
      LET up AS Float = toFloat(__canvas_faceMetric(h, __CANVAS_FACE_ASCENT)) * scale
      LET down AS Float = toFloat(0 - __canvas_faceMetric(h, __CANVAS_FACE_DESCENT)) * scale
      IF found THEN
        ascent = __canvas_maxF(ascent, up)
        descent = __canvas_maxF(descent, down)
      ELSE
        ascent = up
        descent = down
        found = TRUE
      END IF
    END IF
    k = k + 1
  END WHILE
  MUT flag AS Float = 0.0
  IF found THEN
    flag = 1.0
  END IF
  RETURN [ascent, descent, flag]
END FUNC

' Does a `Text` list the font `handle`? A closed font's handle reads 0, which names
' nothing, so a 0 never matches.
FUNC __canvas_textNamesFont(t AS Text, handle AS Integer) AS Boolean
  IF handle = 0 THEN
    RETURN FALSE
  END IF
  FOR EACH f IN t.fonts
    IF canvas::fontHandle(f) = handle THEN
      RETURN TRUE
    END IF
  NEXT
  RETURN FALSE
END FUNC

' Close every font a retired `Text` lists that nothing live still names (plan-162-C):
' the group ownership rule (`.ai/canvas-threading.md` R17b, R18), font by font. A font
' the list repeats closes once -- once closed, its handle reads 0 and it is passed over.
FUNC __canvas_closeTextFonts(t AS Text, scene AS List OF DrawItem) AS Nothing
  FOR EACH f IN t.fonts
    LET fh AS Integer = canvas::fontHandle(f)
    IF fh <> 0 AND NOT __canvas_anythingNamesFont(scene, fh) THEN
      canvas::destroyFont(f)
    END IF
  NEXT
END FUNC

' The pen's travel along a shaped run, from `x0`: each advance at its font's scale, and
' `spacing` at every grapheme boundary when it is not 0 (plan-162-D).
FUNC __canvas_runPen(shaped AS List OF Integer, scales AS List OF Float, x0 AS Float, spacing AS Float) AS Float
  MUT pen AS Float = x0
  MUT c AS Integer = 0
  WHILE c * __CANVAS_RUN_STRIDE < len(shaped)
    pen = pen + toFloat(collections::getOr(shaped, c * __CANVAS_RUN_STRIDE + __CANVAS_RUN_ADVANCE, 0)) * collections::getOr(scales, collections::getOr(shaped, c * __CANVAS_RUN_STRIDE + __CANVAS_RUN_FONT, 0), 0.0)
    IF spacing <> 0.0 AND collections::getOr(shaped, c * __CANVAS_RUN_STRIDE + __CANVAS_RUN_GAP, 0) = 1 THEN
      pen = pen + spacing
    END IF
    c = c + 1
  END WHILE
  RETURN pen
END FUNC

' Each glyph's outline is read in place from the file of the font its triple names
' (bug-1091): by handle, never through a copy of the file.
FUNC __canvas_textEdges(t AS Text) AS List OF Float
  __CANVAS_RING_MARKS = []
  LET handles AS List OF Integer = __canvas_textHandles(t)
  LET scales AS List OF Float = __canvas_fontScales(handles, t.fontSize)
  LET shaped AS List OF Integer = __canvas_shapeRun(handles, t.text, t.letterSpacing)
  IF NOT __canvas_runDrawable(shaped, scales) THEN
    RETURN []
  END IF
  MUT edges AS List OF Float = []
  MUT pen AS Float = __canvas_textX(t)
  MUT c AS Integer = 0
  __CANVAS_RING_MARKING = TRUE
  WHILE c * __CANVAS_RUN_STRIDE < len(shaped)
    LET which AS Integer = collections::getOr(shaped, c * __CANVAS_RUN_STRIDE + __CANVAS_RUN_FONT, 0)
    LET scale AS Float = collections::getOr(scales, which, 0.0)
    IF scale > 0.0 THEN
      edges = __canvas_glyphEdges(collections::getOr(handles, which, 0), collections::getOr(shaped, c * __CANVAS_RUN_STRIDE + __CANVAS_RUN_GLYPH, 0), scale, pen, __canvas_textY(t), edges)
    END IF
    pen = pen + toFloat(collections::getOr(shaped, c * __CANVAS_RUN_STRIDE + __CANVAS_RUN_ADVANCE, 0)) * scale
    IF t.letterSpacing <> 0.0 AND collections::getOr(shaped, c * __CANVAS_RUN_STRIDE + __CANVAS_RUN_GAP, 0) = 1 THEN
      pen = pen + t.letterSpacing
    END IF
    c = c + 1
  END WHILE
  __CANVAS_RING_MARKING = FALSE
  RETURN edges
END FUNC

FUNC __canvas_textHeader(t AS Text, tail AS List OF Float) AS List OF Float
  IF __canvas_strokeHalf(t.paint) <= 0.0 THEN
    RETURN __canvas_glyphRunHeader(t, tail)
  END IF
  LET tailLen AS Integer = len(tail)
  IF tailLen < 5 THEN
    RETURN __canvas_emptyHeader()
  END IF
  MUT out AS List OF Float = __canvas_blankHeader()
  out = collections::set(out, 0, toFloat(__CANVAS_GEO_POLYGON))
  out = collections::set(out, 1, toFloat(__CANVAS_GEO_HEADER + tailLen))
  out = __canvas_paintHeader(out, t.paint, __canvas_textTransform(t), __canvas_textX(t), __canvas_textY(t))
  ' plan-116-F Correction F18. `__canvas_paintHeader` decides "has this kind an
  ' interior?" by reading slot 0 -- and by the time it runs here, slot 0 says POLYGON,
  ' because a stroked text IS lowered to one. So its `Text` skip does not fire, and it
  ' counts `stops * 5` into slot 1 for a stop tail that `__canvas_geometryFor`'s `Text` arm
  ' never appends: it returns `__canvas_textEdges(t)` and nothing else. The record then
  ' declares ten floats it does not have, `__canvas_gradientStopBase` reads from one
  ' past its end, and the run draws its fill from whatever the next record's header
  ' holds -- measured as 874 pixels going from the given `rgb(255, 255, 0)` to
  ' `rgb(11, 0, 0)`.
  '
  ' Undone rather than prevented, because slot 0 legitimately has to say POLYGON for
  ' the rasteriser. The general trap is that a kind is a proxy for "has an interior"
  ' and stops being one the moment a kind is rewritten -- worth remembering if another
  ' kind ever lowers to a different one.
  out = collections::set(out, 1, toFloat(__CANVAS_GEO_HEADER + tailLen))
  out = collections::set(out, __CANVAS_GEO_GRADIENT_COUNT, 0.0)
  out = collections::set(out, 20, toFloat(tailLen / 5))
  MUT minX AS Float = collections::getOr(tail, 0, 0.0)
  MUT maxX AS Float = minX
  MUT minY AS Float = collections::getOr(tail, 1, 0.0)
  MUT maxY AS Float = minY
  MUT i AS Integer = 0
  WHILE i < tailLen
    LET x0 AS Float = collections::getOr(tail, i, 0.0)
    LET y0 AS Float = collections::getOr(tail, i + 1, 0.0)
    LET x1 AS Float = x0 + collections::getOr(tail, i + 2, 0.0)
    LET y1 AS Float = y0 + collections::getOr(tail, i + 3, 0.0)
    minX = __canvas_minF(minX, __canvas_minF(x0, x1))
    maxX = __canvas_maxF(maxX, __canvas_maxF(x0, x1))
    minY = __canvas_minF(minY, __canvas_minF(y0, y1))
    maxY = __canvas_maxF(maxY, __canvas_maxF(y0, y1))
    i = i + 5
  END WHILE
  LET pad AS Float = __canvas_strokePad(t.paint)
  RETURN __canvas_boundsHeader(out, minX, minY, maxX, maxY, pad, pad, __CANVAS_BOUNDS_RIM)
END FUNC"#;
