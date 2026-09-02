//! Geometry generation and the geometry cache.
//!
//! A `DrawItem` is what the *program* wrote; **geometry** is what the *renderer*
//! draws. Generation turns one into the other: a fixed 27-float header carrying the
//! shape's kind, its distance-function parameters, its two colours and its bounds,
//! followed by a per-kind tail (a polygon's precomputed edge array).
//!
//! Slots 20 and 21 are the per-kind auxiliary pair: an arc's start and end angle, or
//! a polygon's edge count. They mean different things because only one kind reads
//! each — a shape never needs both — and giving each its own slot would widen every
//! record to carry fields no primitive uses at once.
//!
//! `Line` and `Arc` have no interior. Their generators put the **stroke** colour in
//! the fill slots and set the stroke half-width negative
//! (`__canvas_strokeAsFill`), so the single rasterisation loop draws them in one pass
//! with no special case: the band the distance function already describes *is* the
//! shape.
//!
//! This flat float buffer is deliberately the shape a GPU backend uploads. plan-98-E
//! and F consume exactly this: the header is an SDF quad's per-instance parameter
//! block, and the tail is the only per-item vertex data any primitive needs. Keeping
//! the software oracle and the GPU backends on one geometry representation is what
//! makes the oracle predictive rather than merely similar.
//!
//! ## Why the cache pays
//!
//! For an SDF shape the header is a handful of field reads and caching it saves
//! little. The tail is where the work is: a polygon's edge array turns the per-pixel
//! distance query from "recompute every edge vector" into "read five floats", and
//! building it is `O(points)` per item. A scene re-presenting an unchanged polygon
//! rebuilds nothing — which is plan-98-A invariant 2, and is what
//! `__canvas_geoGenerations` counts so a test can see it happen.
//!
//! ## Why a hash *and* a comparison
//!
//! The probe is by hash, but a hit is confirmed by comparing the 27-float header
//! exactly before the tail is reused. A hash alone would let a collision reuse
//! another item's geometry and silently draw the wrong picture — a rare wrong answer
//! is worse than a common slow one, and the confirmation costs 27 float compares
//! against a tail that can be thousands.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

/// The geometry record's fixed header, and the kind tags the rasteriser switches on.
///
/// `__CANVAS_GEO_NONE` is a real kind rather than an absent record: `Text` and
/// `Picture` still occupy a scene slot and still have a cache entry, so the item
/// indices, the hash list and the geometry offsets all stay parallel. Dropping them
/// would make every downstream index depend on which variants happened to be
/// present.
#[rustfmt::skip]
const GEO_LAYOUT: &str =
r#"LET __CANVAS_GEO_HEADER AS Integer = 27
LET __CANVAS_GEO_TEXT AS Integer = 6
LET __CANVAS_GEO_NONE AS Integer = 5
LET __CANVAS_GEO_POLYGON AS Integer = 4
LET __CANVAS_GEO_ARC AS Integer = 3"#;

/// The cache, as parallel lists rather than a list of records.
///
/// A `GeoCacheEntry` record would have to be declared in the `canvas` package, where
/// it would appear in `mfb man canvas types` as a type no program can name or use —
/// the same reason `__CANVAS_KIND_*` are bare integers. Parallel lists also match
/// how the data is consumed: the rasteriser walks `__CANVAS_GEO_DATA` linearly and
/// never wants a whole entry at once.
///
/// `__CANVAS_GEO_REV` is a monotonically increasing use counter, not the scene
/// revision: eviction wants "least recently *used*", and an entry can be used many
/// times within one revision (a repaint) or not at all across several.
#[rustfmt::skip]
const GEO_CACHE_STATE: &str =
r#"MUT __CANVAS_GEO_HASHES AS List OF Integer = []
MUT __CANVAS_GEO_OFFSETS AS List OF Integer = []
MUT __CANVAS_GEO_COUNTS AS List OF Integer = []
MUT __CANVAS_GEO_LASTUSED AS List OF Integer = []
MUT __CANVAS_GEO_DATA AS List OF Float = []
MUT __CANVAS_GEO_REV AS Integer = 0
MUT __CANVAS_GEO_GENERATIONS AS Integer = 0
LET __CANVAS_GEO_CAPACITY AS Integer = 256

' The geometry offsets the frame being rendered is holding.
'
' A frame resolves every item's offset before it draws any of them, and the cache holds
' fewer entries than a large scene has items -- so an offset can outlive its cache entry
' by most of a frame. The offsets stay READABLE (`__CANVAS_GEO_DATA` is never compacted),
' but the glyph indices inside them do not stay VALID, because glyph eviction renumbers.
' This list is how eviction knows which of them are still live.
MUT __CANVAS_GEO_LIVE AS List OF Integer = []"#;

/// A bounded, order-independent hash over the geometry header.
///
/// Floats are quantised to 1/65536 before mixing. That is a *probe* quality choice
/// and cannot cause a wrong answer: `__canvas_geoFind` confirms every hit by
/// comparing the header exactly, so a quantisation collision costs one wasted
/// comparison, never an incorrect reuse.
///
/// The mix is kept under 2^31 by the `MOD`, so the multiply cannot overflow a 64-bit
/// `Integer` — an overflow trap here would turn a drawing call into an error.
#[rustfmt::skip]
const GEO_HASH: &str =
r#"FUNC __canvas_hashStep(acc AS Integer, value AS Integer) AS Integer
  RETURN (acc * 131 + value) MOD 2147483647
END FUNC

FUNC __canvas_hashFloat(acc AS Integer, value AS Float) AS Integer
  RETURN __canvas_hashStep(acc, toInt(value * 65536.0))
END FUNC

FUNC __canvas_hashGeometry(geo AS List OF Float, offset AS Integer, count AS Integer) AS Integer
  MUT acc AS Integer = 2166136261
  MUT i AS Integer = 0
  WHILE i < count
    acc = __canvas_hashFloat(acc, collections::getOr(geo, offset + i, 0.0))
    i = i + 1
  END WHILE
  RETURN acc
END FUNC"#;

/// Build the fixed header for one item.
///
/// Every arm writes the same 27 slots in the same order, so the rasteriser and the
/// cache comparison can both be written once against the layout instead of per kind.
/// The `MATCH` is exhaustive over the frozen `DrawItem` set, so a ninth variant would
/// fail to compile here rather than silently generating nothing.
#[rustfmt::skip]
const GEO_HEADER: &str =
r#"FUNC __canvas_headerFor(item AS DrawItem) AS List OF Float
  MATCH item
    CASE Rectangle(r)
      RETURN __canvas_rectHeader(r.x, r.y, r.w, r.h, 0.0, r.paint)
    CASE RoundedRect(rr)
      RETURN __canvas_rectHeader(rr.x, rr.y, rr.w, rr.h, rr.cornerRadius, rr.paint)
    CASE Circle(c)
      RETURN __canvas_circleHeader(c.x, c.y, c.radius, c.paint)
    CASE Line(l)
      RETURN __canvas_segmentHeader(l.x1, l.y1, l.x2, l.y2, l.paint)
    CASE Arc(a)
      RETURN __canvas_arcHeader(a)
    CASE Polygon(p)
      RETURN __canvas_polygonHeader(p)
    CASE Picture(pic)
      RETURN __canvas_emptyHeader()
    CASE Text(t)
      RETURN __canvas_emptyHeader()
  END MATCH
END FUNC

FUNC __canvas_blankHeader() AS List OF Float
  MUT h AS List OF Float = []
  MUT i AS Integer = 0
  WHILE i < __CANVAS_GEO_HEADER
    h = collections::append(h, 0.0)
    i = i + 1
  END WHILE
  RETURN h
END FUNC

FUNC __canvas_emptyHeader() AS List OF Float
  MUT h AS List OF Float = __canvas_blankHeader()
  h = collections::set(h, 0, toFloat(__CANVAS_GEO_NONE))
  h = collections::set(h, 1, toFloat(__CANVAS_GEO_HEADER))
  RETURN h
END FUNC

FUNC __canvas_paintHeader(h AS List OF Float, paint AS Paint) AS List OF Float
  MUT out AS List OF Float = h
  out = collections::set(out, 7, __canvas_strokeHalf(paint))
  out = collections::set(out, 8, toFloat(toInt(paint.fill.red)))
  out = collections::set(out, 9, toFloat(toInt(paint.fill.green)))
  out = collections::set(out, 10, toFloat(toInt(paint.fill.blue)))
  out = collections::set(out, 11, toFloat(toInt(paint.fill.alpha)))
  out = collections::set(out, 12, toFloat(toInt(paint.stroke.red)))
  out = collections::set(out, 13, toFloat(toInt(paint.stroke.green)))
  out = collections::set(out, 14, toFloat(toInt(paint.stroke.blue)))
  out = collections::set(out, 15, toFloat(toInt(paint.stroke.alpha)))
  ' plan-116-B: the clip, RESOLVED to x0,y0,x1,y1 rather than kept as x,y,w,h, so
  ' neither the rasteriser nor either shader repeats the addition per pixel.
  '
  ' Written unconditionally, with no zero-area special case, because none is needed:
  ' w = 0 gives x1 = x + 0 = x, so the `x0 >= x1` test that means "unclipped" already
  ' holds, and an all-zero Bounds -- what an unset Paint.clip is -- resolves to four
  ' zeros and satisfies it too. A branch here would only be a second way to say the
  ' same thing.
  out = collections::set(out, 22, paint.clip.x)
  out = collections::set(out, 23, paint.clip.y)
  out = collections::set(out, 24, paint.clip.x + paint.clip.w)
  out = collections::set(out, 25, paint.clip.y + paint.clip.h)
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
  RETURN out
END FUNC

FUNC __canvas_strokeAsFill(h AS List OF Float) AS List OF Float
  MUT out AS List OF Float = h
  out = collections::set(out, 8, collections::getOr(h, 12, 0.0))
  out = collections::set(out, 9, collections::getOr(h, 13, 0.0))
  out = collections::set(out, 10, collections::getOr(h, 14, 0.0))
  out = collections::set(out, 11, collections::getOr(h, 15, 0.0))
  out = collections::set(out, 7, 0.0 - 1.0)
  RETURN out
END FUNC

FUNC __canvas_boundsHeader(h AS List OF Float, minX AS Float, minY AS Float, maxX AS Float, maxY AS Float) AS List OF Float
  MUT out AS List OF Float = h
  out = collections::set(out, 16, minX)
  out = collections::set(out, 17, minY)
  out = collections::set(out, 18, maxX)
  out = collections::set(out, 19, maxY)
  RETURN out
END FUNC

FUNC __canvas_rectHeader(x AS Float, y AS Float, w AS Float, h AS Float, cornerRadius AS Float, paint AS Paint) AS List OF Float
  MUT out AS List OF Float = __canvas_blankHeader()
  IF w <= 0.0 THEN
    RETURN __canvas_emptyHeader()
  END IF
  IF h <= 0.0 THEN
    RETURN __canvas_emptyHeader()
  END IF
  LET limit AS Float = __canvas_minF(w, h) / 2.0
  LET radius AS Float = __canvas_minF(__canvas_maxF(cornerRadius, 0.0), limit)
  out = collections::set(out, 0, toFloat(__CANVAS_KIND_RECT))
  out = collections::set(out, 1, toFloat(__CANVAS_GEO_HEADER))
  out = collections::set(out, 2, x + w / 2.0)
  out = collections::set(out, 3, y + h / 2.0)
  out = collections::set(out, 4, w / 2.0 - radius)
  out = collections::set(out, 5, h / 2.0 - radius)
  out = collections::set(out, 6, radius)
  out = __canvas_paintHeader(out, paint)
  LET pad AS Float = __canvas_maxF(__canvas_strokeHalf(paint), 0.0) + 1.0
  RETURN __canvas_boundsHeader(out, x - pad, y - pad, x + w + pad, y + h + pad)
END FUNC

FUNC __canvas_circleHeader(x AS Float, y AS Float, radius AS Float, paint AS Paint) AS List OF Float
  IF radius <= 0.0 THEN
    RETURN __canvas_emptyHeader()
  END IF
  MUT out AS List OF Float = __canvas_blankHeader()
  out = collections::set(out, 0, toFloat(__CANVAS_KIND_CIRCLE))
  out = collections::set(out, 1, toFloat(__CANVAS_GEO_HEADER))
  out = collections::set(out, 2, x)
  out = collections::set(out, 3, y)
  out = collections::set(out, 4, radius)
  out = __canvas_paintHeader(out, paint)
  LET reach AS Float = radius + __canvas_maxF(__canvas_strokeHalf(paint), 0.0) + 1.0
  RETURN __canvas_boundsHeader(out, x - reach, y - reach, x + reach, y + reach)
END FUNC

FUNC __canvas_segmentHeader(x1 AS Float, y1 AS Float, x2 AS Float, y2 AS Float, paint AS Paint) AS List OF Float
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
  out = __canvas_paintHeader(out, paint)
  out = __canvas_strokeAsFill(out)
  LET pad AS Float = half + 1.0
  RETURN __canvas_boundsHeader(out, __canvas_minF(x1, x2) - pad, __canvas_minF(y1, y2) - pad, __canvas_maxF(x1, x2) + pad, __canvas_maxF(y1, y2) + pad)
END FUNC

FUNC __canvas_arcHeader(a AS Arc) AS List OF Float
  LET half AS Float = __canvas_strokeHalf(a.paint)
  IF half <= 0.0 THEN
    RETURN __canvas_emptyHeader()
  END IF
  IF a.radius <= 0.0 THEN
    RETURN __canvas_emptyHeader()
  END IF
  MUT out AS List OF Float = __canvas_blankHeader()
  out = collections::set(out, 0, toFloat(__CANVAS_GEO_ARC))
  out = collections::set(out, 1, toFloat(__CANVAS_GEO_HEADER))
  out = collections::set(out, 2, a.x)
  out = collections::set(out, 3, a.y)
  out = collections::set(out, 4, a.radius)
  out = collections::set(out, 6, half)
  out = __canvas_paintHeader(out, a.paint)
  out = __canvas_strokeAsFill(out)
  out = collections::set(out, 20, a.startAngle)
  out = collections::set(out, 21, a.endAngle)
  LET reach AS Float = a.radius + half + 1.0
  RETURN __canvas_boundsHeader(out, a.x - reach, a.y - reach, a.x + reach, a.y + reach)
END FUNC

FUNC __canvas_polygonHeader(p AS Polygon) AS List OF Float
  LET count AS Integer = len(p.points)
  IF count < 2 THEN
    RETURN __canvas_emptyHeader()
  END IF
  MUT out AS List OF Float = __canvas_blankHeader()
  LET first AS Point = collections::getOr(p.points, 0, Point[x := 0.0, y := 0.0])
  MUT minX AS Float = first.x
  MUT maxX AS Float = first.x
  MUT minY AS Float = first.y
  MUT maxY AS Float = first.y
  MUT i AS Integer = 1
  WHILE i < count
    LET q AS Point = collections::getOr(p.points, i, first)
    minX = __canvas_minF(minX, q.x)
    maxX = __canvas_maxF(maxX, q.x)
    minY = __canvas_minF(minY, q.y)
    maxY = __canvas_maxF(maxY, q.y)
    i = i + 1
  END WHILE
  out = collections::set(out, 0, toFloat(__CANVAS_GEO_POLYGON))
  out = collections::set(out, 1, toFloat(__CANVAS_GEO_HEADER + count * 5))
  out = __canvas_paintHeader(out, p.paint)
  out = collections::set(out, 20, toFloat(count))
  LET pad AS Float = __canvas_maxF(__canvas_strokeHalf(p.paint), 0.0) + 1.0
  RETURN __canvas_boundsHeader(out, minX - pad, minY - pad, maxX + pad, maxY + pad)
END FUNC"#;

/// The per-kind tail: the work the cache exists to skip.
///
/// Only a polygon has one. Each edge is stored as `x0, y0, dx, dy, invLenSq`, which
/// is exactly what the per-pixel segment-distance query needs — so the query reads
/// five floats instead of recomputing the edge vector and its length for every pixel
/// of every frame. A degenerate (zero-length) edge stores `invLenSq = 0`, which makes
/// the projection parameter clamp to the endpoint rather than dividing by zero.
#[rustfmt::skip]
const GEO_TAIL: &str =
r#"FUNC __canvas_polygonEdges(points AS List OF Point) AS List OF Float
  MUT out AS List OF Float = []
  LET count AS Integer = len(points)
  LET origin AS Point = Point[x := 0.0, y := 0.0]
  MUT i AS Integer = 0
  WHILE i < count
    LET a AS Point = collections::getOr(points, i, origin)
    LET b AS Point = collections::getOr(points, (i + 1) MOD count, origin)
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

FUNC __canvas_tailFor(item AS DrawItem) AS List OF Float
  MATCH item
    CASE Polygon(p)
      IF len(p.points) < 2 THEN
        RETURN []
      END IF
      RETURN __canvas_polygonEdges(p.points)
    CASE Rectangle(r)
      RETURN []
    CASE RoundedRect(rr)
      RETURN []
    CASE Circle(c)
      RETURN []
    CASE Line(l)
      RETURN []
    CASE Arc(a)
      RETURN []
    CASE Picture(pic)
      RETURN []
    CASE Text(t)
      IF __canvas_strokeHalf(t.paint) > 0.0 THEN
        RETURN __canvas_textEdges(t)
      END IF
      RETURN __canvas_textGlyphRun(t)
  END MATCH
END FUNC

FUNC __canvas_textGlyphRun(t AS Text) AS List OF Float
  LET b AS List OF Byte = __canvas_fontBlob(t.font.id)
  IF len(b) = 0 THEN
    RETURN []
  END IF
  LET upem AS Integer = __canvas_fontUnitsPerEm(b)
  IF upem <= 0 THEN
    RETURN []
  END IF
  LET scale AS Float = t.size / toFloat(upem)
  LET cps AS List OF Integer = encoding::utf32Encode(t.text)
  LET chars AS Integer = len(cps)

  ' Pass one rasterises, recording each entry in the GLOBAL pin list rather than in a
  ' local. A run being built is not yet in the geometry cache, so the pin scan cannot
  ' see it: without this list, the eleventh glyph of a string could evict the first ten
  ' -- glyphs the very item under construction is about to draw -- and the run would
  ' carry indices to entries that no longer exist. It went further than losing them:
  ' eviction renumbers survivors, so indices already copied into a local were stale
  ' whether or not their glyph was dropped. The list is global precisely so
  ' `__canvas_glyphEvict` can pin AND renumber it.
  __CANVAS_GLYPH_PINS = []
  MUT c AS Integer = 0
  WHILE c < chars
    LET gid AS Integer = __canvas_glyphIndex(b, collections::getOr(cps, c, 0))
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
    LET entry AS Integer = __canvas_glyphEntry(b, t.font.id, gid, t.size, scale)
    __CANVAS_GLYPH_PINS = collections::append(__CANVAS_GLYPH_PINS, entry)
    c = c + 1
  END WHILE

  ' Pass two reads the entries back, after any renumbering.
  MUT run AS List OF Float = []
  MUT pen AS Float = t.x
  c = 0
  WHILE c < chars
    LET gid AS Integer = __canvas_glyphIndex(b, collections::getOr(cps, c, 0))
    run = collections::append(run, toFloat(collections::getOr(__CANVAS_GLYPH_PINS, c, 0 - 1)))
    run = collections::append(run, toFloat(toInt(pen + 0.5)))
    run = collections::append(run, toFloat(toInt(t.y + 0.5)))
    pen = pen + toFloat(__canvas_glyphAdvance(b, gid)) * scale
    c = c + 1
  END WHILE
  RETURN run
END FUNC

FUNC __canvas_textEdges(t AS Text) AS List OF Float
  LET b AS List OF Byte = __canvas_fontBlob(t.font.id)
  IF len(b) = 0 THEN
    RETURN []
  END IF
  LET upem AS Integer = __canvas_fontUnitsPerEm(b)
  IF upem <= 0 THEN
    RETURN []
  END IF
  LET scale AS Float = t.size / toFloat(upem)
  MUT edges AS List OF Float = []
  MUT pen AS Float = t.x
  FOR EACH cp IN encoding::utf32Encode(t.text)
    LET gid AS Integer = __canvas_glyphIndex(b, cp)
    edges = __canvas_glyphEdges(b, gid, scale, pen, t.y, edges)
    pen = pen + toFloat(__canvas_glyphAdvance(b, gid)) * scale
  NEXT
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
  out = __canvas_paintHeader(out, t.paint)
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
  LET pad AS Float = __canvas_maxF(__canvas_strokeHalf(t.paint), 0.0) + 1.0
  RETURN __canvas_boundsHeader(out, minX - pad, minY - pad, maxX + pad, maxY + pad)
END FUNC"#;

/// Probe, confirm, and on a miss generate and insert.
///
/// Returns the offset of the item's geometry within `__CANVAS_GEO_DATA`. The header
/// is always built (it is cheap and it is what confirms a hit); only the tail is
/// conditional, and `__CANVAS_GEO_GENERATIONS` counts the times a tail was actually
/// built — the number a test watches to prove a hit skipped generation.
///
/// Eviction is least-recently-used over `__CANVAS_GEO_LASTUSED`. It drops the entry's
/// *slot*, not its bytes: compacting `__CANVAS_GEO_DATA` would move every other
/// entry's offset, so the buffer is rebuilt wholesale when the slot table is full
/// rather than compacted in place. That trades a rare O(n) rebuild for never holding
/// a stale offset, which is the failure that would draw one item's geometry for
/// another.
#[rustfmt::skip]
const GEO_CACHE: &str =
r#"FUNC __canvas_headerMatches(offset AS Integer, header AS List OF Float) AS Boolean
  MUT i AS Integer = 0
  WHILE i < __CANVAS_GEO_HEADER
    LET stored AS Float = collections::getOr(__CANVAS_GEO_DATA, offset + i, 0.0)
    LET wanted AS Float = collections::getOr(header, i, 0.0)
    IF stored <> wanted THEN
      RETURN FALSE
    END IF
    i = i + 1
  END WHILE
  RETURN TRUE
END FUNC

' The header compare above cannot see a polygon's point coordinates -- they live in
' the tail. A hit for a polygon is therefore confirmed against the stored edge
' origins as well, or a 31-bit hash collision between two same-header polygons
' would hand one the other's edges. The stored edge layout is `x0, y0, dx, dy,
' invLenSq` per edge (`__canvas_polygonEdges`), so slots 0-1 of each edge are the
' point itself, copied verbatim -- exact float compare is right here.
FUNC __canvas_polygonPointsMatch(offset AS Integer, points AS List OF Point) AS Boolean
  LET count AS Integer = len(points)
  MUT i AS Integer = 0
  WHILE i < count
    LET base AS Integer = offset + __CANVAS_GEO_HEADER + i * 5
    LET q AS Point = collections::getOr(points, i, Point[x := 0.0, y := 0.0])
    IF collections::getOr(__CANVAS_GEO_DATA, base, 0.0) <> q.x THEN
      RETURN FALSE
    END IF
    IF collections::getOr(__CANVAS_GEO_DATA, base + 1, 0.0) <> q.y THEN
      RETURN FALSE
    END IF
    i = i + 1
  END WHILE
  RETURN TRUE
END FUNC

FUNC __canvas_tailMatches(item AS DrawItem, offset AS Integer) AS Boolean
  MATCH item
    CASE Polygon(p)
      ' Guard on the STORED kind: a degenerate (<2 point) polygon stores an empty
      ' NONE header with no tail, and comparing points against absent slots would
      ' refuse the hit every frame and grow the cache without bound.
      IF toInt(collections::getOr(__CANVAS_GEO_DATA, offset, 0.0)) = __CANVAS_GEO_POLYGON THEN
        RETURN __canvas_polygonPointsMatch(offset, p.points)
      END IF
      RETURN TRUE
    CASE Rectangle(r)
      RETURN TRUE
    CASE RoundedRect(rr)
      RETURN TRUE
    CASE Circle(c)
      RETURN TRUE
    CASE Line(l)
      RETURN TRUE
    CASE Arc(a)
      RETURN TRUE
    CASE Text(t)
      RETURN TRUE
    CASE Picture(pic)
      RETURN TRUE
  END MATCH
END FUNC

FUNC __canvas_geoEvict() AS Integer
  LET slots AS Integer = len(__CANVAS_GEO_HASHES)
  MUT evicted AS Integer = 0
  IF slots >= __CANVAS_GEO_CAPACITY THEN
    MUT worst AS Integer = 0
    MUT worstRev AS Integer = collections::getOr(__CANVAS_GEO_LASTUSED, 0, 0)
    MUT i AS Integer = 1
    WHILE i < slots
      LET rev AS Integer = collections::getOr(__CANVAS_GEO_LASTUSED, i, 0)
      IF rev < worstRev THEN
        worstRev = rev
        worst = i
      END IF
      i = i + 1
    END WHILE
    __CANVAS_GEO_HASHES = collections::removeAt(__CANVAS_GEO_HASHES, worst)
    __CANVAS_GEO_OFFSETS = collections::removeAt(__CANVAS_GEO_OFFSETS, worst)
    __CANVAS_GEO_COUNTS = collections::removeAt(__CANVAS_GEO_COUNTS, worst)
    __CANVAS_GEO_LASTUSED = collections::removeAt(__CANVAS_GEO_LASTUSED, worst)
    evicted = 1
  END IF
  RETURN evicted
END FUNC

FUNC __canvas_geometryFor(item AS DrawItem, hash AS Integer) AS Integer
  ' Every other kind's header is a handful of arithmetic on the item's own fields, so
  ' building it on every probe costs nothing and it doubles as the hash-collision
  ' guard. A `canvas::Text` header is not like that: its bounds and its edge count are
  ' properties of the *flattened glyph outlines*, so building it per frame would
  ' re-read `glyf` for every character of every string on screen and the cache would
  ' save nothing at all. Text therefore probes on the hash alone and builds its header
  ' from the tail on a miss.
  LET deferred AS Boolean = __canvas_headerIsDeferred(item)
  MUT header AS List OF Float = []
  IF NOT deferred THEN
    header = __canvas_headerFor(item)
  END IF
  __CANVAS_GEO_REV = __CANVAS_GEO_REV + 1
  MUT slot AS Integer = 0
  LET slots AS Integer = len(__CANVAS_GEO_HASHES)
  WHILE slot < slots
    IF collections::getOr(__CANVAS_GEO_HASHES, slot, 0) = hash THEN
      LET offset AS Integer = collections::getOr(__CANVAS_GEO_OFFSETS, slot, 0)
      IF deferred OR (__canvas_headerMatches(offset, header) AND __canvas_tailMatches(item, offset)) THEN
        __CANVAS_GEO_LASTUSED = collections::set(__CANVAS_GEO_LASTUSED, slot, __CANVAS_GEO_REV)
        RETURN offset
      END IF
    END IF
    slot = slot + 1
  END WHILE

  LET evicted AS Integer = __canvas_geoEvict()
  LET tail AS List OF Float = __canvas_tailFor(item)
  IF deferred THEN
    header = __canvas_deferredHeader(item, tail)
  END IF
  __CANVAS_GEO_GENERATIONS = __CANVAS_GEO_GENERATIONS + 1
  LET offset AS Integer = len(__CANVAS_GEO_DATA)
  ' Append into a LOCAL and write the global back once. `collections::append` is
  ' in-place only for a local of the function doing the write, so appending straight
  ' into the global copies the whole buffer per element — 27 copies per new item
  ' instead of two, and enough allocation churn to grow a 200-frame animation by
  ' ~0.6 MB a frame. See the `collections::set` note in `.ai/collections.md`.
  MUT buffer AS List OF Float = __CANVAS_GEO_DATA
  MUT i AS Integer = 0
  WHILE i < __CANVAS_GEO_HEADER
    buffer = collections::append(buffer, collections::getOr(header, i, 0.0))
    i = i + 1
  END WHILE
  MUT j AS Integer = 0
  LET tailCount AS Integer = len(tail)
  WHILE j < tailCount
    buffer = collections::append(buffer, collections::getOr(tail, j, 0.0))
    j = j + 1
  END WHILE
  __CANVAS_GEO_DATA = buffer
  __CANVAS_GEO_HASHES = collections::append(__CANVAS_GEO_HASHES, hash)
  __CANVAS_GEO_OFFSETS = collections::append(__CANVAS_GEO_OFFSETS, offset)
  __CANVAS_GEO_COUNTS = collections::append(__CANVAS_GEO_COUNTS, __CANVAS_GEO_HEADER + tailCount)
  __CANVAS_GEO_LASTUSED = collections::append(__CANVAS_GEO_LASTUSED, __CANVAS_GEO_REV)
  RETURN offset
END FUNC

FUNC __canvas_glyphRunHeader(t AS Text, run AS List OF Float) AS List OF Float
  LET glyphs AS Integer = len(run) / 3
  IF glyphs <= 0 THEN
    RETURN __canvas_emptyHeader()
  END IF
  MUT out AS List OF Float = __canvas_blankHeader()
  out = collections::set(out, 0, toFloat(__CANVAS_GEO_TEXT))
  out = collections::set(out, 1, toFloat(__CANVAS_GEO_HEADER + len(run)))
  out = __canvas_paintHeader(out, t.paint)
  ' Slots 2 and 3 carry the font handle and the em size: a glyph run needs both to
  ' rasterise, and they are the shape parameters no other kind uses for text.
  out = collections::set(out, 2, toFloat(t.font.id))
  out = collections::set(out, 3, t.size)
  out = collections::set(out, 20, toFloat(glyphs))
  ' The ink box comes from the metrics rather than from the outlines: this header is
  ' built on a cache miss, when the glyphs have not been rasterised yet, and an
  ' ascent/descent box always contains the ink. It is only used to clip and to
  ' invalidate, so a box that is too large costs nothing and one that is too small
  ' would clip the glyphs it was meant to bound.
  LET b AS List OF Byte = __canvas_fontBlob(t.font.id)
  LET upem AS Integer = __canvas_fontUnitsPerEm(b)
  LET scale AS Float = t.size / toFloat(__canvas_maxI(upem, 1))
  LET ascent AS Float = toFloat(__canvas_fontAscent(b)) * scale
  LET descent AS Float = toFloat(0 - __canvas_fontDescent(b)) * scale
  MUT minX AS Float = collections::getOr(run, 1, t.x)
  MUT maxX AS Float = minX
  MUT i AS Integer = 0
  WHILE i < len(run)
    LET px AS Float = collections::getOr(run, i + 1, 0.0)
    minX = __canvas_minF(minX, px)
    maxX = __canvas_maxF(maxX, px)
    i = i + 3
  END WHILE
  LET advance AS Float = t.size * 2.0
  RETURN __canvas_boundsHeader(out, minX - advance, t.y - ascent - 2.0, maxX + advance, t.y + descent + 2.0)
END FUNC

FUNC __canvas_headerIsDeferred(item AS DrawItem) AS Boolean
  MATCH item
    CASE Text(t)
      RETURN TRUE
    CASE Rectangle(r)
      RETURN FALSE
    CASE RoundedRect(rr)
      RETURN FALSE
    CASE Circle(c)
      RETURN FALSE
    CASE Line(l)
      RETURN FALSE
    CASE Arc(a)
      RETURN FALSE
    CASE Polygon(p)
      RETURN FALSE
    CASE Picture(pic)
      RETURN FALSE
  END MATCH
END FUNC

FUNC __canvas_deferredHeader(item AS DrawItem, tail AS List OF Float) AS List OF Float
  MATCH item
    CASE Text(t)
      RETURN __canvas_textHeader(t, tail)
    CASE Rectangle(r)
      RETURN __canvas_emptyHeader()
    CASE RoundedRect(rr)
      RETURN __canvas_emptyHeader()
    CASE Circle(c)
      RETURN __canvas_emptyHeader()
    CASE Line(l)
      RETURN __canvas_emptyHeader()
    CASE Arc(a)
      RETURN __canvas_emptyHeader()
    CASE Polygon(p)
      RETURN __canvas_emptyHeader()
    CASE Picture(pic)
      RETURN __canvas_emptyHeader()
  END MATCH
END FUNC

FUNC __canvas_textHash(t AS Text) AS Integer
  MUT h AS List OF Float = __canvas_blankHeader()
  h = collections::set(h, 0, toFloat(__CANVAS_GEO_TEXT))
  h = collections::set(h, 2, toFloat(t.font.id))
  h = collections::set(h, 3, t.size)
  h = collections::set(h, 4, t.x)
  h = collections::set(h, 5, t.y)
  h = __canvas_paintHeader(h, t.paint)
  MUT acc AS Integer = __canvas_hashGeometry(h, 0, __CANVAS_GEO_HEADER)
  FOR EACH cp IN encoding::utf32Encode(t.text)
    acc = __canvas_hashStep(acc, cp)
  NEXT
  RETURN acc
END FUNC

FUNC __canvas_deferredHash(item AS DrawItem) AS Integer
  MATCH item
    CASE Text(t)
      RETURN __canvas_textHash(t)
    CASE Rectangle(r)
      RETURN 0
    CASE RoundedRect(rr)
      RETURN 0
    CASE Circle(c)
      RETURN 0
    CASE Line(l)
      RETURN 0
    CASE Arc(a)
      RETURN 0
    CASE Polygon(p)
      RETURN 0
    CASE Picture(pic)
      RETURN 0
  END MATCH
END FUNC

FUNC __canvas_hashItem(item AS DrawItem) AS Integer
  ' A deferred kind has no header until its tail is flattened, so `__canvas_headerFor`
  ' answers the SAME empty header for every one of them -- and a deferred kind probes
  ' the geometry cache on the hash alone. Hashing that empty header would therefore
  ' give every string on screen one hash, and the cache would hand all of them the
  ' first string's glyph run: a sixty-item scene drew one glyph, sixty times, in one
  ' place (plan-98-G Correction 14). The hash has to carry by hand exactly what the
  ' deferred header would otherwise have carried.
  IF __canvas_headerIsDeferred(item) THEN
    RETURN __canvas_deferredHash(item)
  END IF
  LET header AS List OF Float = __canvas_headerFor(item)
  MUT acc AS Integer = __canvas_hashGeometry(header, 0, __CANVAS_GEO_HEADER)
  ' A polygon's POINTS live only in the tail, and its header carries just their
  ' bounds and count -- so two different polygons can share a header (two same-box,
  ' same-count, same-paint triangles collided and one drew twice). The hash carries
  ' the coordinates by hand, exactly as `__canvas_textHash` carries its codepoints.
  MATCH item
    CASE Polygon(p)
      LET count AS Integer = len(p.points)
      MUT i AS Integer = 0
      WHILE i < count
        LET q AS Point = collections::getOr(p.points, i, Point[x := 0.0, y := 0.0])
        acc = __canvas_hashFloat(acc, q.x)
        acc = __canvas_hashFloat(acc, q.y)
        i = i + 1
      END WHILE
      RETURN acc
    CASE Rectangle(r)
      RETURN acc
    CASE RoundedRect(rr)
      RETURN acc
    CASE Circle(c)
      RETURN acc
    CASE Line(l)
      RETURN acc
    CASE Arc(a)
      RETURN acc
    CASE Text(t)
      RETURN acc
    CASE Picture(pic)
      RETURN acc
  END MATCH
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("canvas_geoLayout", GEO_LAYOUT));
    pkg.add_helper(RegistryHelper::always(
        "canvas_geoCacheState",
        GEO_CACHE_STATE,
    ));
    pkg.add_helper(RegistryHelper::always("canvas_geoHash", GEO_HASH));
    pkg.add_helper(RegistryHelper::always("canvas_geoHeader", GEO_HEADER));
    pkg.add_helper(RegistryHelper::always("canvas_geoTail", GEO_TAIL));
    pkg.add_helper(RegistryHelper::always("canvas_geoCache", GEO_CACHE));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::runtime::canvas::{GEO_KIND_POLYGON, GEO_KIND_TEXT, HEADER_SLOTS};

    /// The decimal literal `GEO_LAYOUT` binds to `name`.
    fn declared(name: &str) -> usize {
        let needle = format!("LET {name} AS Integer = ");
        let start = GEO_LAYOUT
            .find(&needle)
            .unwrap_or_else(|| panic!("{name} is not declared in GEO_LAYOUT"))
            + needle.len();
        let rest = &GEO_LAYOUT[start..];
        let end = rest.find('\n').unwrap_or(rest.len());
        rest[..end].trim().parse().expect("a decimal literal")
    }

    /// `GEO_LAYOUT`'s constants equal their Rust counterparts.
    ///
    /// Each of these numbers is spelled **twice** — once in MFBASIC here, once in
    /// `runtime::canvas` for the emitters — with no compiler between them. Nothing
    /// else relates the two spellings, so this test is the whole guard.
    ///
    /// The header length is the dangerous one. Every geometry record is
    /// `__CANVAS_GEO_HEADER` floats followed by a per-kind tail, and both GPU emitters
    /// find that tail at `HEADER_SLOTS * 8` bytes. If the two disagree by even one
    /// slot, a polygon's first edge coordinate is read as a header field and a header
    /// field as an edge — which draws a *plausible wrong shape* rather than failing.
    /// That is exactly the failure mode a change to the header length invites, because
    /// such a change has to touch both spellings and can be applied to only one.
    #[test]
    fn the_geo_layout_constants_match_their_rust_counterparts() {
        assert_eq!(
            declared("__CANVAS_GEO_HEADER"),
            HEADER_SLOTS,
            "the MFBASIC header length and the emitters' HEADER_SLOTS disagree: the \
             tail would be read at the wrong offset, so a polygon's first edge \
             coordinate becomes a header field",
        );
        for (name, kind) in [
            ("__CANVAS_GEO_TEXT", GEO_KIND_TEXT),
            ("__CANVAS_GEO_POLYGON", GEO_KIND_POLYGON),
        ] {
            assert_eq!(
                declared(name).to_string(),
                kind,
                "{name} and its emitter-side kind constant disagree, so the predicates \
                 and the emitters would branch on different values for the same kind",
            );
        }
    }
}
