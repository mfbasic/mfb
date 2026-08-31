//! Geometry generation and the geometry cache.
//!
//! A `DrawItem` is what the *program* wrote; **geometry** is what the *renderer*
//! draws. Generation turns one into the other: a fixed 22-float header carrying the
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
//! The probe is by hash, but a hit is confirmed by comparing the 22-float header
//! exactly before the tail is reused. A hash alone would let a collision reuse
//! another item's geometry and silently draw the wrong picture — a rare wrong answer
//! is worse than a common slow one, and the confirmation costs 22 float compares
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
r#"LET __CANVAS_GEO_HEADER AS Integer = 22
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
LET __CANVAS_GEO_CAPACITY AS Integer = 256"#;

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
/// Every arm writes the same 22 slots in the same order, so the rasteriser and the
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
      RETURN []
  END MATCH
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
  LET header AS List OF Float = __canvas_headerFor(item)
  __CANVAS_GEO_REV = __CANVAS_GEO_REV + 1
  MUT slot AS Integer = 0
  LET slots AS Integer = len(__CANVAS_GEO_HASHES)
  WHILE slot < slots
    IF collections::getOr(__CANVAS_GEO_HASHES, slot, 0) = hash THEN
      LET offset AS Integer = collections::getOr(__CANVAS_GEO_OFFSETS, slot, 0)
      IF __canvas_headerMatches(offset, header) THEN
        __CANVAS_GEO_LASTUSED = collections::set(__CANVAS_GEO_LASTUSED, slot, __CANVAS_GEO_REV)
        RETURN offset
      END IF
    END IF
    slot = slot + 1
  END WHILE

  LET evicted AS Integer = __canvas_geoEvict()
  LET tail AS List OF Float = __canvas_tailFor(item)
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

FUNC __canvas_hashItem(item AS DrawItem) AS Integer
  LET header AS List OF Float = __canvas_headerFor(item)
  RETURN __canvas_hashGeometry(header, 0, __CANVAS_GEO_HEADER)
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
