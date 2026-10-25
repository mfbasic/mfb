//! plan-176-R: `helper_geometry`'s MFBASIC helper text — the geometry cache — its state, the
//! item hash and the cached lookup (`GEO_CACHE_STATE`, `GEO_HASH`, `GEO_CACHE`). Registration
//! lives in `builtins/canvas/helper_geometry/mod.rs`.

/// The cache, as flat lists rather than a list of records.
///
/// A `GeoCacheEntry` record would have to be declared in the `canvas` package, where
/// it would appear in `mfb man canvas types` as a type no program can name or use —
/// the same reason `__CANVAS_KIND_*` are bare integers. Flat lists also match how the
/// data is consumed: the rasteriser walks `__CANVAS_GEO_DATA` linearly and never wants a
/// whole entry at once.
///
/// bug-686: the store is kept natively (`func_geo_cache/`): `__CANVAS_GEO_SLOTS` holds
/// four words per slot and `__CANVAS_GEO_TABLE` indexes them by item hash. A slot lives
/// until the frame boundary after the first frame that did not use it. It used to be a
/// 256-slot list scanned linearly per probe, then a `Map` whose per-frame rebuild and
/// per-item probes were most of a moving scene's frame.
///
/// A slot's fourth word is the FRAME it was last used in (`__CANVAS_GEO_FRAME`), and
/// nothing is evicted inside a frame: `canvas::geoBeginFrame` is the only place a slot is
/// dropped, and it runs where no offset is live. So every offset a frame resolves stays
/// valid — and its glyph indices stay valid — until that frame is drawn.
#[rustfmt::skip]
pub(super) const GEO_CACHE_STATE: &str =
r#"' bug-686: the cache's store, kept by the native members in `func_geo_cache.rs`: four
' words per slot (hash, offset, count, lastUsed -- -1 once forgotten), and the hash index
' as an open-addressing table of (hash, slot) buckets.
MUT __CANVAS_GEO_SLOTS AS List OF Integer = []
MUT __CANVAS_GEO_TABLE AS List OF Integer = []
MUT __CANVAS_GEO_DATA AS List OF Float = []
' Records built by the native builder, and this frame's (index, slot) pairs of those
' `canvas::sceneResolve` built -- what the `--debug` geometry check re-checks.
MUT __CANVAS_GEO_NATIVE_BUILDS AS Integer = 0
MUT __CANVAS_GEO_BUILT AS List OF Integer = []
MUT __CANVAS_GEO_GENERATIONS AS Integer = 0
MUT __CANVAS_GEO_COMPACTIONS AS Integer = 0
' plan-175-A: the table is keyed by an item's GEOMETRY identity `G`, and this is the
' damage identity `D` each slot's record holds, one word a slot in slot order: a slot
' found by `G` is a hit only when its `D` is the item's. Kept beside the four-word slots
' rather than as a fifth word, so every `slot * 4` reader is unchanged.
MUT __CANVAS_GEO_DRAWS AS List OF Integer = []
' Records rewritten in place for a change of appearance alone: no allocation, no insert,
' no generation (`refreshes=` on the stats line).
MUT __CANVAS_GEO_REFRESHES AS Integer = 0
' What `__canvas_geoLookup` decided for the record its caller builds next: the slot to
' rewrite in place (-1: none), else the key to insert it under (`__canvas_geoStore`).
MUT __CANVAS_GEO_PLAN_SLOT AS Integer = 0 - 1
MUT __CANVAS_GEO_PLAN_KEY AS Integer = 0

' The frame being resolved, and what it and the frame before it used. A slot counts once
' per frame however many times the scene names it, so these are the live set's size.
MUT __CANVAS_GEO_FRAME AS Integer = 1
MUT __CANVAS_GEO_USED_FLOATS AS Integer = 0
MUT __CANVAS_GEO_LAST_FLOATS AS Integer = 0

' The all-zero header `__canvas_blankHeader` hands out copies of, built on first use.
MUT __CANVAS_GEO_BLANK AS List OF Float = []"#;

/// The item content hash: the geometry cache's key, and the damage diff's "did this item
/// change".
///
/// bug-686: a cache hit is trusted on this hash alone — there is no longer a header
/// rebuild per probe to confirm it — so it is two independent 31-bit lanes (62 bits) and
/// resolves a coordinate to 2^-46 px (`__canvas_hashFloat`). Each lane is reduced below
/// 2^31 before it is multiplied, so nothing here can overflow a 64-bit `Integer` — an
/// overflow trap would turn a drawing call into an error.
#[rustfmt::skip]
pub(super) const GEO_HASH: &str =
r#"' bug-686: TWO independent 31-bit lanes, packed as `a * 2^31 + b` (62 bits), because
' the geometry cache now trusts a hash hit without rebuilding the item's header to
' confirm it. A single 31-bit lane made that unsafe at scale: 10,000 live items against
' a cache of tens of thousands of entries collide with probability ~0.3 per frame, and a
' collision draws one item's geometry for another. Two lanes put that at ~1e-10.
'
' Each lane reduces the incoming value first, so a 48-bit font handle or a large scaled
' coordinate never overflows `a * 131` / `b * 257` -- both stay below 2^40.
'
' Division-free: both lanes are reduced modulo the Mersenne prime 2^31 - 1 by folding
' (`x & M + x >> 31`, twice), which is shifts and adds rather than the four divides a
' `MOD` step cost -- this runs ~20 times per item on every present. The value enters
' as three pieces (bits 0-30, 31-61, 62-63), each below 2^31, so any Integer --
' negative, a 48-bit handle, a scaled coordinate -- folds in whole, and every sum below
' stays under 2^42. The lanes use different multipliers AND different mixes of the
' pieces, so they are not one hash computed twice.
FUNC __canvas_hashStep(acc AS Integer, value AS Integer) AS Integer
  LET a AS Integer = bits::sr(acc, 31)
  LET b AS Integer = bits::band(acc, 2147483647)
  LET lo AS Integer = bits::band(value, 2147483647)
  LET hi AS Integer = bits::band(bits::sr(value, 31), 2147483647)
  LET top AS Integer = bits::sr(value, 62)
  LET xa AS Integer = a * 131 + lo + hi * 3 + top
  LET ra AS Integer = bits::band(xa, 2147483647) + bits::sr(xa, 31)
  MUT na AS Integer = bits::band(ra, 2147483647) + bits::sr(ra, 31)
  IF na >= 2147483647 THEN
    na = na - 2147483647
  END IF
  LET xb AS Integer = b * 257 + hi + lo * 5 + top * 7
  LET rb AS Integer = bits::band(xb, 2147483647) + bits::sr(xb, 31)
  MUT nb AS Integer = bits::band(rb, 2147483647) + bits::sr(rb, 31)
  IF nb >= 2147483647 THEN
    nb = nb - 2147483647
  END IF
  RETURN na * 2147483648 + nb
END FUNC

' A float folds in as its 1/65536 integer part AND the next 30 bits of fraction below
' that. The first alone quantised: two items differing by less than 1/65536 px hashed
' equal, which was harmless only while every hit was confirmed against a freshly built
' header. `value * 65536.0` is exact (a power of two), and so are the subtraction and the
' second scaling, so the pair resolves a coordinate to 2^-46 px.
FUNC __canvas_hashFloat(acc AS Integer, value AS Float) AS Integer
  LET scaled AS Float = value * 65536.0
  LET whole AS Integer = toInt(scaled)
  RETURN __canvas_hashStep(__canvas_hashStep(acc, whole), toInt((scaled - toFloat(whole)) * 1073741824.0))
END FUNC"#;

/// Probe by hash, and on a miss generate and insert.
///
/// Returns the offset of the item's geometry within `__CANVAS_GEO_DATA`. A hit is
/// trusted on the item's 62-bit content hash (`__canvas_hashItem`) — nothing is built
/// to confirm it — except for a `Picture`, whose image can change its pixels without the
/// item changing. `__CANVAS_GEO_GENERATIONS` counts the builds: the number a test
/// watches to prove a hit skipped generation (bug-686).
///
/// Nothing is evicted inside a frame. `canvas::geoBeginFrame`, at the one point in a
/// frame where no offset is live, drops what the previous frame did not use and
/// renumbers the rest, so a frame can never be handed a stale offset — the failure that
/// would draw one item's geometry for another.
#[rustfmt::skip]
pub(super) const GEO_CACHE: &str =
r#"' Count a slot as used by the frame in progress, once per frame however many times the
' scene names it. What `canvas::geoBeginFrame` knows about the live set comes from here.
FUNC __canvas_geoStamp(slot AS Integer) AS Nothing
  LET at AS Integer = slot * 4
  IF collections::getOr(__CANVAS_GEO_SLOTS, at + 3, 0) <> __CANVAS_GEO_FRAME THEN
    __CANVAS_GEO_SLOTS = collections::set(__CANVAS_GEO_SLOTS, at + 3, __CANVAS_GEO_FRAME)
    __CANVAS_GEO_USED_FLOATS = __CANVAS_GEO_USED_FLOATS + collections::getOr(__CANVAS_GEO_SLOTS, at + 2, 0)
  END IF
END FUNC

' plan-175-A: the record for an item of geometry identity `g` and damage identity `d` --
' `canvas::sceneResolve`'s branches, in MFBASIC. Answers the offset of a record to use as
' it is, or -1 after deciding how the record the caller then builds is stored
' (`__canvas_geoStore`):
'
' * `g` found and its record holds `d`: a hit.
' * `g` found, another paint, and nothing this frame uses the record: a REFRESH -- the
'   new record is written over the old one in place (`canvas::geoRefresh`).
' * `g` found, another paint, and an entry of this frame already uses the record (two
'   items of one geometry and two paints): this one is resolved under `d` instead --
'   found there, or built and inserted there.
' * `g` not found: built and inserted under `g`.
FUNC __canvas_geoLookup(g AS Integer, d AS Integer) AS Integer
  LET slot AS Integer = canvas::geoFind(g)
  IF slot < 0 THEN
    __CANVAS_GEO_PLAN_SLOT = 0 - 1
    __CANVAS_GEO_PLAN_KEY = g
    RETURN 0 - 1
  END IF
  IF collections::getOr(__CANVAS_GEO_DRAWS, slot, 0 - 1) = d THEN
    __canvas_geoStamp(slot)
    RETURN collections::getOr(__CANVAS_GEO_SLOTS, slot * 4 + 1, 0)
  END IF
  IF collections::getOr(__CANVAS_GEO_SLOTS, slot * 4 + 3, 0) <> __CANVAS_GEO_FRAME THEN
    __CANVAS_GEO_PLAN_SLOT = slot
    __CANVAS_GEO_PLAN_KEY = g
    RETURN 0 - 1
  END IF
  LET shared AS Integer = canvas::geoFind(d)
  IF shared >= 0 THEN
    __canvas_geoStamp(shared)
    RETURN collections::getOr(__CANVAS_GEO_SLOTS, shared * 4 + 1, 0)
  END IF
  __CANVAS_GEO_PLAN_SLOT = 0 - 1
  __CANVAS_GEO_PLAN_KEY = d
  RETURN 0 - 1
END FUNC

' Store the record built after `__canvas_geoLookup` answered -1, as it decided: over the
' slot it chose, or as a new slot under the key it chose. Answers the record's offset.
FUNC __canvas_geoStore(d AS Integer, header AS List OF Float, tail AS List OF Float) AS Integer
  IF __CANVAS_GEO_PLAN_SLOT >= 0 THEN
    RETURN canvas::geoRefresh(__CANVAS_GEO_PLAN_SLOT, d, header, tail)
  END IF
  RETURN canvas::geoInsert(__CANVAS_GEO_PLAN_KEY, d, header, tail)
END FUNC

' The record cached for `key`, for a caller whose key IS its damage identity: an
' `AutoPaint` state's (`__canvas_autoGeoHash`), a picture's outline, the worker's hit
' records. What `__canvas_geometryOf` is for an item hashed whole.
FUNC __canvas_geometryFor(item AS DrawItem, hash AS Integer) AS Integer
  RETURN __canvas_geometryOf(item, hash, hash)
END FUNC

' plan-175-A Phase 3: refresh the record of `slot` for `item`, whose paint alone changed,
' without its tail: the kind's own header builder, alone, gives every appearance slot
' afresh, and `canvas::geoRepaint` copies those and the ramp's stops into the record.
' Answers the record's offset, or -1 for a kind this does not refresh (the caller then
' rebuilds the whole record in place). The kinds are the ones whose rebuild is costly
' and whose header builder is the record's first step: a shape whose gradient or
' transform the native builder declines, and a `Text`, whose glyph run is the costly
' part and is geometry.
FUNC __canvas_geoRepaint(item AS DrawItem, slot AS Integer, draw AS Integer) AS Integer
  LET none AS List OF Float = []
  MATCH item
    CASE Circle(c)
      RETURN canvas::geoRepaint(slot, draw, __canvas_circleItem(c), __canvas_gradientTail(c.paint))
    CASE Ellipse(e)
      RETURN canvas::geoRepaint(slot, draw, __canvas_ellipseHeader(e), __canvas_gradientTail(e.paint))
    CASE Rectangle(r)
      ' A rectangle whose corners are not one radius is built as a polygon ring.
      LET radius AS Float = __canvas_rectRadius(r)
      IF radius >= 0.0 THEN
        RETURN canvas::geoRepaint(slot, draw, __canvas_rectItem(r, radius), __canvas_gradientTail(r.paint))
      END IF
    CASE Text(t)
      ' A ramp on text is a stroked run's polygon business; that is rebuilt.
      IF len(t.paint.gradient.stops) < 2 THEN
        LET offset AS Integer = collections::getOr(__CANVAS_GEO_SLOTS, slot * 4 + 1, 0)
        MUT h AS List OF Float = __canvas_blankHeader()
        h = collections::set(h, 0, __canvas_geoAt(offset, 0))
        h = __canvas_autoTextBox(__canvas_paintHeader(h, t.paint, __canvas_textTransform(t), __canvas_textX(t), __canvas_textY(t)), t)
        RETURN canvas::geoRepaint(slot, draw, h, none)
      END IF
    CASE ELSE
  END MATCH
  RETURN 0 - 1
END FUNC

' plan-172-L: a `Picture` is not built here. Its records are made, cached and -- the
' one kind a hash cannot vouch for, since `setBytes` swaps an image's pixels without
' changing the item -- confirmed in `helper_picture.rs`, which the walks hand a picture
' to before they get here (`__canvas_appendDraw`, `__canvas_memoGroup`).
FUNC __canvas_geometryOf(item AS DrawItem, geo AS Integer, draw AS Integer) AS Integer
  MATCH item
    CASE Tile(tile)
      RETURN __canvas_tileGeometry(tile, geo, draw)
    CASE ELSE
  END MATCH
  LET found AS Integer = __canvas_geoLookup(geo, draw)
  IF found >= 0 THEN
    RETURN found
  END IF
  IF __CANVAS_GEO_PLAN_SLOT >= 0 THEN
    LET repainted AS Integer = __canvas_geoRepaint(item, __CANVAS_GEO_PLAN_SLOT, draw)
    IF repainted >= 0 THEN
      RETURN repainted
    END IF
  END IF

  ' Not a hit: build the record, then store it as the lookup decided.
  '
  ' bug-686: the common kinds -- a rectangle, rounded rectangle, circle, line or polygon
  ' with no transform and no gradient -- are built natively, header and tail in one
  ' record, bit-identical to the builders below (`func_geo_build.rs`). Anything else
  ' answers an empty list and takes the MFBASIC path. `__canvas_geoVerify` is empty in a
  ' normal build; a `--debug` build's checks the record against the builders.
  LET native AS List OF Float = canvas::geoBuild(item)
  IF len(native) > 0 THEN
    __canvas_geoVerify(item, native)
    __CANVAS_GEO_NATIVE_BUILDS = __CANVAS_GEO_NATIVE_BUILDS + 1
    RETURN __canvas_geoStore(draw, native, [])
  END IF

  ' ONE `MATCH` for both halves (bug-686): extracting a variant copies it, and doing it
  ' once for the tail and again for the header cost ~250 ns an item on every build.
  '
  ' Every kind's header is a handful of arithmetic on the item's own fields -- except a
  ' `canvas::Text`, whose bounds and edge count are properties of the flattened glyph
  ' outlines, so it is built from the tail. A tail is written EDGES FIRST, then gradient
  ' stops, so `__canvas_gradientStopBase` finds the stops from the end of the record.
  MUT tail AS List OF Float = []
  MUT header AS List OF Float = []
  MATCH item
    CASE Rectangle(r)
      LET radius AS Float = __canvas_rectRadius(r)
      IF radius >= 0.0 THEN
        header = __canvas_rectItem(r, radius)
        LET joins AS List OF Float = __canvas_rectJoins(header, r.paint)
        LET dashes AS List OF Float = __canvas_rectDashes(header, r.paint)
        tail = __canvas_appendGradientTail(collections::append(joins, dashes), r.paint)
        header = __canvas_withDashes(__canvas_withJoins(header, joins, __canvas_joinReach(__canvas_strokeHalf(r.paint), r.paint)), dashes, r.paint)
      ELSE
        ' plan-172-E: corners that are not one circular radius. The outline is a polygon
        ' ring, so the rest is the polygon's own arm: edges, joins, dashes, stops.
        LET ring AS List OF List OF vector::Float2 = __canvas_rectRing(r)
        LET edges AS List OF Float = __canvas_polygonEdges(ring)
        LET joins AS List OF Float = __canvas_joinTail(edges, __canvas_ringStarts(ring), TRUE, __canvas_strokeHalf(r.paint), r.paint)
        header = __canvas_polygonHeader(ring, FillRule.EvenOdd, r.paint, __canvas_placement(r.position, r.transform), __canvas_buildX(r.position, r.transform), __canvas_buildY(r.position, r.transform))
        LET dashes AS List OF Float = __canvas_edgeDashes(header, r.paint, __canvas_strokeHalf(r.paint), edges, __canvas_ringStarts(ring), TRUE)
        IF len(edges) > 0 THEN
          tail = __canvas_appendGradientTail(collections::append(collections::append(edges, joins), dashes), r.paint)
        END IF
        header = __canvas_withDashes(__canvas_withJoins(header, joins, __canvas_joinReach(__canvas_strokeHalf(r.paint), r.paint)), dashes, r.paint)
      END IF
    CASE Circle(c)
      header = __canvas_circleItem(c)
      LET dashes AS List OF Float = __canvas_roundDashes(header, c.paint)
      tail = __canvas_appendGradientTail(dashes, c.paint)
      header = __canvas_withDashes(header, dashes, c.paint)
    CASE Line(l)
      header = __canvas_lineHeader(l)
      LET dashes AS List OF Float = __canvas_lineDashes(header, l)
      tail = __canvas_strokeRampTail(dashes, l.paint)
      header = __canvas_withDashes(header, dashes, l.paint)
    CASE Arc(a)
      header = __canvas_arcHeader(a)
      LET dashes AS List OF Float = __canvas_arcDashes(header, a)
      tail = __canvas_strokeRampTail(dashes, a.paint)
      header = __canvas_withDashes(header, dashes, a.paint)
    CASE Polygon(p)
      ' plan-172-F: the points are offsets from `position`. The joins and the dashes are
      ' computed from `edges`, never from the item's points, so they follow.
      LET placed AS List OF List OF vector::Float2 = __canvas_placedRings(p.contours, p.position, p.transform)
      LET edges AS List OF Float = __canvas_polygonEdges(placed)
      LET joins AS List OF Float = __canvas_joinTail(edges, __canvas_ringStarts(placed), TRUE, __canvas_strokeHalf(p.paint), p.paint)
      header = __canvas_polygonHeader(placed, p.rule, p.paint, __canvas_placement(p.position, p.transform), __canvas_buildX(p.position, p.transform), __canvas_buildY(p.position, p.transform))
      LET dashes AS List OF Float = __canvas_edgeDashes(header, p.paint, __canvas_strokeHalf(p.paint), edges, __canvas_ringStarts(placed), TRUE)
      IF len(edges) > 0 THEN
        tail = __canvas_appendGradientTail(collections::append(collections::append(edges, joins), dashes), p.paint)
      END IF
      header = __canvas_withDashes(__canvas_withJoins(header, joins, __canvas_joinReach(__canvas_strokeHalf(p.paint), p.paint)), dashes, p.paint)
    ' plan-172-L: never reached (see above); the arm keeps MATCH exhaustive, as a
    ' group's does.
    CASE Picture(pic)
      header = __canvas_emptyHeader()
    CASE Tile(tile)
      header = __canvas_emptyHeader()
    CASE Ellipse(e)
      header = __canvas_ellipseHeader(e)
      LET dashes AS List OF Float = __canvas_ellipseDashes(header, e)
      tail = __canvas_appendGradientTail(dashes, e.paint)
      header = __canvas_withDashes(header, dashes, e.paint)
    CASE Text(t)
      IF __canvas_strokeHalf(t.paint) > 0.0 THEN
        ' plan-163-B: a stroked run is a polygon of the glyph contours, so it joins
        ' like one -- each contour a ring (`__CANVAS_RING_MARKS`).
        LET edges AS List OF Float = __canvas_textEdges(t)
        LET joins AS List OF Float = __canvas_joinTail(edges, __CANVAS_RING_MARKS, TRUE, __canvas_strokeHalf(t.paint), t.paint)
        header = __canvas_textHeader(t, edges)
        LET dashes AS List OF Float = __canvas_edgeDashes(header, t.paint, __canvas_strokeHalf(t.paint), edges, __CANVAS_RING_MARKS, TRUE)
        tail = collections::append(collections::append(edges, joins), dashes)
        header = __canvas_withDashes(__canvas_withJoins(header, joins, __canvas_joinReach(__canvas_strokeHalf(t.paint), t.paint)), dashes, t.paint)
      ELSE
        tail = __canvas_textGlyphRun(t)
        header = __canvas_textHeader(t, tail)
      END IF
      ' plan-172-K: in a program with an `AutoPaint`, the record also says where the
      ' run's LAYOUT BOX is, which is what a `Text` is hit on (`helper_hit.rs`). In any
      ' other program this hands the header straight back.
      header = __canvas_autoTextBox(header, t)
    ' plan-116-G: a Group has no geometry of its own -- it names one -- so no tail, and
    ' the empty header, whose NONE kind every renderer skips. It never reaches here from
    ' the walk (a group is expanded, not cached); the arm is what keeps MATCH exhaustive.
    CASE Group(g)
      header = __canvas_emptyHeader()
    ' plan-172-K: nor has an `AutoPaint` -- it draws its shape, and the walk hands this
    ' function the SHAPE (`__canvas_appendDraw`, `__canvas_autoItem`). The same arm as a
    ' group's, for the same reason.
    CASE AutoPaint(a)
      header = __canvas_emptyHeader()
    ' plan-150-A: a `ParticleSystem` is expanded into its particles before anything asks
    ' for geometry (`__canvas_appendDraw`), so it never reaches here either.
    CASE ParticleSystem(ps)
      header = __canvas_emptyHeader()
    ' plan-154-A. The tail only when the header is a real polyline: an empty header
    ' declares a 51-float record, and a tail appended behind it would be floats the
    ' record does not own.
    CASE PolyLine(pl)
      LET moved AS PolyLine = __canvas_polylinePlaced(pl)
      header = __canvas_polylineHeader(moved)
      IF toInt(collections::getOr(header, 0, 0.0)) = __CANVAS_GEO_POLYLINE THEN
        tail = __canvas_polylineTail(moved)
        header = __canvas_withDashes(__canvas_withJoins(header, __canvas_polylineJoins(moved), __canvas_joinReach(__canvas_polylineHalf(moved), moved.paint)), __canvas_polylineDashes(moved), moved.paint)
      END IF
    ' plan-164-B: the two curve shapes, one builder (`helper_curves.rs`): a chain is
    ' one open contour.
    CASE PolyCurve(pc)
      LET built AS List OF List OF Float = __canvas_polyCurveRecord(pc)
      header = collections::getOr(built, 0, header)
      tail = collections::getOr(built, 1, tail)
    CASE CurvePolygon(cp)
      LET built AS List OF List OF Float = __canvas_curvePolygonRecord(cp)
      header = collections::getOr(built, 0, header)
      tail = collections::getOr(built, 1, tail)
  END MATCH
  ' plan-163-C: a path clip's edges, right before the stops.
  LET clipped AS List OF List OF Float = __canvas_withClipEdges(header, tail, item)
  header = collections::getOr(clipped, 0, header)
  tail = collections::getOr(clipped, 1, tail)
  ' The header is always exactly `__CANVAS_GEO_HEADER` floats -- every builder starts
  ' from `__canvas_blankHeader` -- and slot 1 declares the record's length on that
  ' assumption. The padded copy below is the guard for a builder that ever breaks it.
  ' `canvas::geoInsert` appends header then tail straight into `__CANVAS_GEO_DATA`, in
  ' place (bug-682, bug-686), and counts the generation.
  IF len(header) = __CANVAS_GEO_HEADER THEN
    RETURN __canvas_geoStore(draw, header, tail)
  END IF
  MUT fixed AS List OF Float = []
  MUT i AS Integer = 0
  WHILE i < __CANVAS_GEO_HEADER
    fixed = collections::append(fixed, collections::getOr(header, i, 0.0))
    i = i + 1
  END WHILE
  RETURN __canvas_geoStore(draw, fixed, tail)
END FUNC

FUNC __canvas_glyphRunHeader(t AS Text, run AS List OF Float) AS List OF Float
  LET glyphs AS Integer = len(run) / 3
  IF glyphs <= 0 THEN
    RETURN __canvas_emptyHeader()
  END IF
  MUT out AS List OF Float = __canvas_blankHeader()
  out = collections::set(out, 0, toFloat(__CANVAS_GEO_TEXT))
  out = collections::set(out, 1, toFloat(__CANVAS_GEO_HEADER + len(run)))
  out = __canvas_paintHeader(out, t.paint, __canvas_textTransform(t), __canvas_textX(t), __canvas_textY(t))
  ' Slots 2 and 3 carry the first font's handle and the em size. Nothing reads slot 2
  ' back (plan-162-A Phase 1): each glyph's entry already names its font's raster.
  LET handles AS List OF Integer = __canvas_textHandles(t)
  out = collections::set(out, 2, toFloat(collections::getOr(handles, 0, 0)))
  out = collections::set(out, 3, t.fontSize)
  out = collections::set(out, 20, toFloat(glyphs))
  ' The ink box comes from the metrics rather than from the outlines: this header is
  ' built on a cache miss, when the glyphs have not been rasterised yet, and an
  ' ascent/descent box always contains the ink. It is only used to clip and to
  ' invalidate, so a box that is too large costs nothing and one that is too small
  ' would clip the glyphs it was meant to bound. With several fonts it is the tallest
  ' of the ones the run draws from.
  LET scales AS List OF Float = __canvas_fontScales(handles, t.fontSize)
  LET line AS List OF Float = __canvas_runLine(handles, scales, __canvas_shapeRun(handles, t.text, t.letterSpacing))
  LET ascent AS Float = collections::getOr(line, 0, 0.0)
  LET descent AS Float = collections::getOr(line, 1, 0.0)
  MUT minX AS Float = collections::getOr(run, 1, __canvas_textX(t))
  MUT maxX AS Float = minX
  MUT i AS Integer = 0
  WHILE i < len(run)
    LET px AS Float = collections::getOr(run, i + 1, 0.0)
    minX = __canvas_minF(minX, px)
    maxX = __canvas_maxF(maxX, px)
    i = i + 3
  END WHILE
  LET advance AS Float = t.fontSize * 2.0
  RETURN __canvas_boundsHeader(out, minX - advance, __canvas_textY(t) - ascent - 2.0, maxX + advance, __canvas_textY(t) + descent + 2.0, 0.0, 0.0, 0.0)
END FUNC

' bug-686: every item's hash is folded from its own FIELDS, kind by kind. It used to be
' a hash of the item's built geometry header -- 51 floats, the transform inverted, the
' bounds computed -- which made `canvas::present` build every item's header on the
' worker only to hash it, and was most of the 3-4 us per item a present cost. The
' fields determine the header, so hashing them keys the cache exactly as well; what the
' header could not carry (a polygon's points, a gradient's stops, a string, a font or
' image) is folded in explicitly, as it always was.
'
' Each kind starts from its own tag, so two kinds with coincidentally equal fields
' (a `Rectangle` and a `Picture` at the same box) never share a key.
FUNC __canvas_hashStart(tag AS Integer) AS Integer
  RETURN __canvas_hashStep(2166136261, tag)
END FUNC

FUNC __canvas_hashPoint(acc AS Integer, p AS vector::Float2) AS Integer
  RETURN __canvas_hashFloat(__canvas_hashFloat(acc, p.x), p.y)
END FUNC

' plan-175-A: a paint's two lanes. `func_item_lanes.rs` classifies every `Paint` field
' for the native walk; these two are its MFBASIC twin, for the kinds hashed here (a
' `Text`). GEOMETRY is what decides the shape of a record: the stroke's width, cap, join,
' miter limit, dashes and offset, the clip, the transform -- and three predicates of
' appearance fields, each folded as its own word: whether the stroke's alpha is zero
' (`__canvas_strokeHalf` answers -1 for it), the gradient's stop count (it sizes the
' tail), and whether the ramp paints the stroke. APPEARANCE is the rest: both colours,
' the blend mode, the ramp, the transparency.
FUNC __canvas_hashPaintGeometry(acc AS Integer, paint AS Paint, transform AS Transform) AS Integer
  MUT h AS Integer = acc
  MUT strokeZero AS Integer = 0
  IF toInt(paint.stroke.alpha) <= 0 THEN
    strokeZero = 1
  END IF
  h = __canvas_hashFloat(__canvas_hashStep(h, strokeZero), paint.strokeWidth)
  ' The clip and the transform are folded in only when present, after ONE step carrying
  ' which of them are (with the stroke's ramp). Their zero values are the no-ops -- an
  ' empty clip ring list is "unclipped", an all-zero Transform the identity -- and most
  ' paints set neither, so this is what keeps a plain item's hash to a handful of steps.
  LET t AS Transform = transform
  LET hasClip AS Boolean = len(paint.clip) > 0
  LET hasTransform AS Boolean = NOT (t.a = 0.0 AND t.b = 0.0 AND t.c = 0.0 AND t.d = 0.0 AND t.tx = 0.0 AND t.ty = 0.0)
  MUT flags AS Integer = 0
  IF hasClip THEN
    flags = flags + 1
  END IF
  IF hasTransform THEN
    flags = flags + 2
  END IF
  IF paint.gradient.strokeApply = GradientApply.FillAndStroke THEN
    flags = flags + 4
  END IF
  h = __canvas_hashStep(__canvas_hashStep(h, flags), len(paint.gradient.stops))
  ' plan-163-C: the clip's rule, then each ring's count and points.
  IF hasClip THEN
    MUT rule AS Integer = 0
    IF paint.clipRule = FillRule.NonZero THEN
      rule = 1
    END IF
    h = __canvas_hashStep(__canvas_hashStep(h, rule), len(paint.clip))
    FOR EACH ring IN paint.clip
      h = __canvas_hashStep(h, len(ring))
      FOR EACH q IN ring
        h = __canvas_hashPoint(h, q)
      NEXT
    NEXT
  END IF
  IF hasTransform THEN
    h = __canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(h, t.a), t.b), t.c)
    h = __canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(h, t.d), t.tx), t.ty)
  END IF
  ' plan-163-B: the stroke's shape -- cap and join as one step, then the miter limit,
  ' the dash lengths after their count, and the offset.
  MUT style AS Integer = 0
  IF paint.cap = CapStyle.Round THEN
    style = 1
  END IF
  IF paint.cap = CapStyle.Square THEN
    style = 2
  END IF
  IF paint.join = JoinStyle.Miter THEN
    style = style + 4
  END IF
  IF paint.join = JoinStyle.Bevel THEN
    style = style + 8
  END IF
  h = __canvas_hashStep(h, style)
  h = __canvas_hashFloat(h, paint.miterLimit)
  h = __canvas_hashStep(h, len(paint.dashes))
  FOR EACH dash IN paint.dashes
    h = __canvas_hashFloat(h, dash)
  NEXT
  RETURN __canvas_hashFloat(h, paint.dashOffset)
END FUNC

FUNC __canvas_hashPaintAppearance(acc AS Integer, paint AS Paint) AS Integer
  MUT h AS Integer = acc
  ' Each colour's four channels (0-255 each) as one packed value: one step, not four.
  h = __canvas_hashStep(h, ((toInt(paint.fill.red) * 256 + toInt(paint.fill.green)) * 256 + toInt(paint.fill.blue)) * 256 + toInt(paint.fill.alpha))
  h = __canvas_hashStep(h, ((toInt(paint.stroke.red) * 256 + toInt(paint.stroke.green)) * 256 + toInt(paint.stroke.blue)) * 256 + toInt(paint.stroke.alpha))
  MUT blend AS Integer = 0
  IF paint.blend = BlendMode.Multiply THEN
    blend = 1
  END IF
  IF paint.blend = BlendMode.Screen THEN
    blend = 2
  END IF
  IF paint.blend = BlendMode.Add THEN
    blend = 3
  END IF
  LET stopCount AS Integer = len(paint.gradient.stops)
  ' A gradient's points and kind mean nothing without two stops: the renderers read
  ' those slots only when the count says there is a ramp. Whether there is one is the
  ' geometry lane's (the count); its ramp is this lane's.
  MUT ramped AS Integer = 0
  IF stopCount >= 2 THEN
    ramped = 1
  END IF
  h = __canvas_hashStep(h, blend * 2 + ramped)
  IF stopCount >= 2 THEN
    MUT radial AS Integer = 0
    IF paint.gradient.kind = GradientKind.Radial THEN
      radial = 1
    END IF
    h = __canvas_hashStep(h, radial)
    h = __canvas_hashPoint(h, paint.gradient.startPoint)
    h = __canvas_hashPoint(h, paint.gradient.endPoint)
    h = __canvas_hashStep(h, stopCount)
    FOR EACH stop IN paint.gradient.stops
      h = __canvas_hashFloat(h, stop.offset)
      h = __canvas_hashStep(h, ((toInt(stop.color.red) * 256 + toInt(stop.color.green)) * 256 + toInt(stop.color.blue)) * 256 + toInt(stop.color.alpha))
    NEXT
    ' plan-163-D: the spread, then the transform, the focal offset and the focal radius.
    ' (The stroke apply is the geometry lane's.)
    MUT ramp AS Integer = 0
    IF paint.gradient.spread = GradientSpread.Reflect THEN
      ramp = 1
    END IF
    IF paint.gradient.spread = GradientSpread.Repeat THEN
      ramp = 2
    END IF
    LET gt AS Transform = paint.gradient.transform
    h = __canvas_hashStep(h, ramp)
    h = __canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(h, gt.a), gt.b), gt.c)
    h = __canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(h, gt.d), gt.tx), gt.ty)
    h = __canvas_hashFloat(__canvas_hashPoint(h, paint.gradient.focalOffset), paint.gradient.focalRadius)
  END IF
  ' plan-163-C.
  RETURN __canvas_hashFloat(h, paint.transparency)
END FUNC

' Both lanes of a paint in one: what a kind hashed as a whole folds -- an `AutoPaint`'s
' four state paints, and the arms kept for the kinds hashed natively.
FUNC __canvas_hashPaint(acc AS Integer, paint AS Paint, transform AS Transform) AS Integer
  RETURN __canvas_hashPaintAppearance(__canvas_hashPaintGeometry(acc, paint, transform), paint)
END FUNC

' plan-175-A: the damage identity `D = mix(G, A)` of an item hashed here, from its
' geometry identity `g` and its appearance lane `a`. It changes exactly when either does.
FUNC __canvas_mixHash(g AS Integer, a AS Integer) AS Integer
  RETURN __canvas_hashStep(__canvas_hashStep(1099511628211, g), a)
END FUNC

' A String folds in a codepoint at a time -- a String is not otherwise hashable here --
' after its length, so "ab" + "c" and "a" + "bc" in two adjacent fields cannot collide.
FUNC __canvas_hashText(acc AS Integer, text AS String) AS Integer
  LET cps AS List OF Integer = encoding::utf32Encode(text)
  MUT h AS Integer = __canvas_hashStep(acc, len(cps))
  FOR EACH cp IN cps
    h = __canvas_hashStep(h, cp)
  NEXT
  RETURN h
END FUNC

' plan-175-A: a `Text`'s geometry identity -- its font list, size, string, spacing,
' position and transform, and its paint's geometry lane. Its `id` is in no identity
' (decision 1: an `id` is only for mouse feedback, so a renamed item redraws nothing).
FUNC __canvas_textGeoHash(t AS Text) AS Integer
  ' The font id is a resource HANDLE -- an address -- and must be folded in as the
  ' integer it is, never through `__canvas_hashFloat`, whose `value * 65536.0`
  ' overflows `toInt` for a 48-bit address. `__canvas_hashStep` reduces it first.
  MUT h AS Integer = __canvas_hashStart(7)
  h = __canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(h, t.position.x), t.position.y), t.fontSize)
  ' Every font, in order, after their count: `[a, b]` and `[b, a]` draw different
  ' glyphs, and `[a]` and `[a, a]` must not collide by accident of the fold.
  h = __canvas_hashStep(h, len(t.fonts))
  FOR EACH f IN t.fonts
    h = __canvas_hashStep(h, canvas::fontHandle(f))
  NEXT
  h = __canvas_hashText(h, t.text)
  ' plan-162-D: the spacing moves every glyph after the first, so it keys the cache --
  ' a spacing-only change must rebuild. Folded only when set, so an unspaced `Text`
  ' keeps the key it always had.
  IF t.letterSpacing <> 0.0 THEN
    h = __canvas_hashFloat(h, t.letterSpacing)
  END IF
  RETURN __canvas_hashPaintGeometry(h, t.paint, t.transform)
END FUNC

' plan-175-A: an item's GEOMETRY identity `G` -- what keys the geometry cache. Natively
' for every kind `canvas::itemGeoHash` folds; a `Text` and a `Picture` on a shape the
' native walk does not fold have their own walks; a `Group` node, an `AutoPaint` and a
' `ParticleSystem` fold everything into one lane, so for them `G` is `D`.
FUNC __canvas_geoHash(item AS DrawItem) AS Integer
  LET native AS Integer = canvas::itemGeoHash(item)
  IF native >= 0 THEN
    RETURN native
  END IF
  MATCH item
    CASE Text(t)
      RETURN __canvas_textGeoHash(t)
    CASE Picture(pic)
      RETURN __canvas_pictureGeoHash(pic)
    CASE ELSE
      RETURN __canvas_hashItem(item)
  END MATCH
  RETURN __canvas_hashItem(item)
END FUNC

' plan-175-A: the damage identity `D` of an item whose geometry identity `g` the caller
' already has (`__canvas_geoHash`), so a `Text` folds its string once, not twice.
FUNC __canvas_drawHashOf(item AS DrawItem, g AS Integer) AS Integer
  LET native AS Integer = canvas::itemHash(item)
  IF native >= 0 THEN
    RETURN native
  END IF
  MATCH item
    CASE Text(t)
      RETURN __canvas_mixHash(g, __canvas_hashPaintAppearance(__canvas_hashStart(7), t.paint))
    CASE Picture(pic)
      RETURN __canvas_pictureHash(pic)
    CASE ELSE
      RETURN g
  END MATCH
  RETURN g
END FUNC

' The DAMAGE identity `D` (plan-175-A): what the damage diff and every draw hash compare,
' and -- until the cache is keyed by `G` -- the geometry cache's key.
FUNC __canvas_hashItem(item AS DrawItem) AS Integer
  ' bug-686: every kind but `Text` and `Group` is hashed natively and structurally
  ' (`func_item_hash.rs`) -- the arms below are only reached for those two. A kind is
  ' hashed by exactly one of the two paths, so their keys never have to agree.
  LET native AS Integer = canvas::itemHash(item)
  IF native >= 0 THEN
    RETURN native
  END IF
  MATCH item
    CASE Rectangle(r)
      MUT h AS Integer = __canvas_hashStart(1)
      h = __canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(h, r.position.x), r.position.y), r.size.width), r.size.height)
      h = __canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(h, r.radiusTL), r.radiusTR), r.radiusBL), r.radiusBR)
      IF r.bind = BindTo.Center THEN
        h = __canvas_hashStep(h, 1)
      END IF
      RETURN __canvas_hashPaint(h, r.paint, r.transform)
    CASE Circle(c)
      MUT h AS Integer = __canvas_hashStart(2)
      h = __canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(h, c.position.x), c.position.y), c.radius)
      RETURN __canvas_hashPaint(h, c.paint, c.transform)
    CASE Line(l)
      MUT h AS Integer = __canvas_hashStart(3)
      h = __canvas_hashPoint(__canvas_hashPoint(__canvas_hashPoint(h, l.position), l.startAt), l.endAt)
      RETURN __canvas_hashPaint(h, l.paint, l.transform)
    CASE Arc(a)
      MUT h AS Integer = __canvas_hashStart(4)
      h = __canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(h, a.position.x), a.position.y), a.radius)
      h = __canvas_hashFloat(__canvas_hashFloat(h, a.startAngle), a.endAngle)
      RETURN __canvas_hashPaint(h, a.paint, a.transform)
    CASE Polygon(p)
      MUT h AS Integer = __canvas_hashStart(5)
      h = __canvas_hashPoint(h, p.position)
      h = __canvas_hashStep(h, len(p.contours))
      FOR EACH ring IN p.contours
        h = __canvas_hashStep(h, len(ring))
        FOR EACH q IN ring
          h = __canvas_hashPoint(h, q)
        NEXT
      NEXT
      IF p.rule = FillRule.NonZero THEN
        h = __canvas_hashStep(h, 1)
      ELSE
        h = __canvas_hashStep(h, 0)
      END IF
      RETURN __canvas_hashPaint(h, p.paint, p.transform)
    ' plan-172-L: a picture whose shape the native hash does not fold -- a `Text`, an
    ' `AutoPaint`. Every other picture was answered above.
    CASE Picture(pic)
      RETURN __canvas_pictureHash(pic)
    CASE Tile(tile)
      MUT h AS Integer = __canvas_hashStep(__canvas_hashStart(13), canvas::imageHandle(tile.image))
      h = __canvas_hashPoint(__canvas_hashPoint(__canvas_hashPoint(__canvas_hashPoint(h, tile.srcPosition), tile.srcSize), tile.position), tile.size)
      RETURN h
    CASE Text(t)
      RETURN __canvas_drawHashOf(item, __canvas_textGeoHash(t))
    CASE Ellipse(e)
      MUT h AS Integer = __canvas_hashStart(8)
      h = __canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(h, e.position.x), e.position.y), e.radiusX), e.radiusY)
      RETURN __canvas_hashPaint(h, e.paint, e.transform)
    ' plan-116-G: a group node is its name and its offset. Two nodes naming different
    ' groups at the same offset must not collide, and neither must one group at two.
    CASE Group(g)
      MUT h AS Integer = __canvas_hashStart(9)
      h = __canvas_hashPoint(h, g.position)
      h = __canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(h, g.transform.a), g.transform.b), g.transform.c), g.transform.d), g.transform.tx), g.transform.ty)
      RETURN __canvas_hashText(h, g.name)
    ' plan-172-K: an `AutoPaint` is its shape's hash, its four flags and its four
    ' paints (`__canvas_autoHash`, in the chunk a program that constructs one carries).
    ' Never hashed natively: its `shape` is a union field, which the structural walk
    ' does not fold.
    CASE AutoPaint(a)
      RETURN __canvas_autoHash(a)
    ' plan-154-A. Hashed natively (`func_item_hash.rs`), so this arm is what keeps the
    ' MATCH exhaustive; it folds the same fields in case that ever changes.
    CASE PolyLine(pl)
      MUT h AS Integer = __canvas_hashStart(10)
      h = __canvas_hashPoint(h, pl.position)
      h = __canvas_hashStep(h, len(pl.points))
      FOR EACH q IN pl.points
        h = __canvas_hashPoint(h, q)
      NEXT
      h = __canvas_hashStep(h, len(pl.colors))
      FOR EACH c IN pl.colors
        h = __canvas_hashStep(h, ((toInt(c.red) * 256 + toInt(c.green)) * 256 + toInt(c.blue)) * 256 + toInt(c.alpha))
      NEXT
      RETURN __canvas_hashPaint(h, pl.paint, pl.transform)
    ' plan-164-A. Both hashed natively, like a PolyLine; the arms fold the same fields,
    ' in the chunk a program that names either shape carries (`helper_curves.rs`).
    CASE PolyCurve(pc)
      RETURN __canvas_polyCurveHash(pc)
    CASE CurvePolygon(cp)
      RETURN __canvas_curvePolygonHash(cp)
    ' plan-150-A: every field of the system and its template's hash, in the chunk a
    ' program that constructs one carries (`helper_particles.rs`). Never hashed
    ' natively: its `item` is a union field.
    CASE ParticleSystem(ps)
      RETURN __canvas_particleSystemHash(ps)
  END MATCH
END FUNC"#;
