//! Geometry generation and the geometry cache.
//!
//! A `DrawItem` is what the *program* wrote; **geometry** is what the *renderer*
//! draws. Generation turns one into the other: a fixed 51-float header carrying the
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
//! The probe is by hash, but a hit is confirmed by comparing the 51-float header
//! exactly before the tail is reused. A hash alone would let a collision reuse
//! another item's geometry and silently draw the wrong picture — a rare wrong answer
//! is worse than a common slow one, and the confirmation costs 51 float compares
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
r#"LET __CANVAS_GEO_HEADER AS Integer = 69
' bug-1083: how much of the item's box (slots 16-19) is rim, on the x sides (67) and
' on the y sides (68), in the units the box is in. The box less this is the item's
' extent and stroke reach under its own transform and nothing more; the sites that
' carry a box through a group's matrix replace the rim there (`__canvas_drawBound`).
' Zero for a box that is not an extent and a rim (`__canvas_boundsHeader`).
LET __CANVAS_GEO_RIM AS Integer = 67
' plan-163-D: the gradient's per-pixel words, after plan-163-C's. 55 is the spread
' (0 Pad, 1 Reflect, 2 Repeat), 56 whether the ramp also paints the stroke (1 for
' FillAndStroke on a kind that has a stroke to paint), 57-62 a RADIAL gradient's inverse
' transform as float32 bit patterns (the slots 27-32 convention) with 63 its flag, and
' 64-66 the focal offset from the centre and the focal radius, already clamped. A linear
' gradient's transform is folded into slots 43-46 at build and leaves 57-63 at zero.
LET __CANVAS_GEO_GRADIENT_SPREAD AS Integer = 55
LET __CANVAS_GEO_GRADIENT_APPLY AS Integer = 56
LET __CANVAS_GEO_GRADIENT_INV AS Integer = 57
LET __CANVAS_GEO_GRADIENT_HAS_INV AS Integer = 63
LET __CANVAS_GEO_GRADIENT_FOCALX AS Integer = 64
LET __CANVAS_GEO_GRADIENT_FOCALY AS Integer = 65
LET __CANVAS_GEO_GRADIENT_FOCALR AS Integer = 66
' plan-163-C: the clip and transparency slots, after plan-163-B's. 51 is the clip's mode
' (0 none, 1 an axis-aligned rectangle in slots 22-25, 2 a path of clip edges), 52 the
' clip edge count, 53 the clip's FillRule ordinal, 54 a picture's transparency step T
' (0..255) -- every other kind's fill and stroke alphas are faded at build instead.
LET __CANVAS_GEO_CLIP_MODE AS Integer = 51
LET __CANVAS_GEO_CLIP_EDGE_COUNT AS Integer = 52
LET __CANVAS_GEO_CLIP_RULE AS Integer = 53
LET __CANVAS_GEO_PICTURE_TRANSPARENCY AS Integer = 54
' plan-163-B: the stroke-style slots, appended after every older slot so none of them
' moved. 47 is the number of join records in the tail (`__canvas_joinBase`), 48 how far
' a record reaches from its vertex (the band index widens to it); 49 is the dash tail's
' float count (`__canvas_dashBase`) and 50 the dash offset wrapped into one period.
LET __CANVAS_GEO_JOIN_COUNT AS Integer = 47
LET __CANVAS_GEO_JOIN_REACH AS Integer = 48
LET __CANVAS_GEO_DASH_COUNT AS Integer = 49
LET __CANVAS_GEO_DASH_PHASE AS Integer = 50
LET __CANVAS_GEO_GRADIENT_COUNT AS Integer = 41
LET __CANVAS_GEO_GRADIENT_KIND AS Integer = 42
LET __CANVAS_GEO_GRADIENT_FROMX AS Integer = 43
LET __CANVAS_GEO_GRADIENT_FROMY AS Integer = 44
LET __CANVAS_GEO_GRADIENT_TOX AS Integer = 45
LET __CANVAS_GEO_GRADIENT_TOY AS Integer = 46
LET __CANVAS_GEO_ELLIPSE AS Integer = 7
LET __CANVAS_GEO_ELLIPSE_COS AS Integer = 39
LET __CANVAS_GEO_ELLIPSE_SIN AS Integer = 40
LET __CANVAS_GEO_CAP AS Integer = 34
LET __CANVAS_GEO_CAPSTARTX AS Integer = 35
LET __CANVAS_GEO_CAPSTARTY AS Integer = 36
LET __CANVAS_GEO_CAPENDX AS Integer = 37
LET __CANVAS_GEO_CAPENDY AS Integer = 38
LET __CANVAS_GEO_TEXT AS Integer = 6
LET __CANVAS_GEO_NONE AS Integer = 5
LET __CANVAS_GEO_POLYGON AS Integer = 4
' plan-163-A. A polygon's `FillRule` as its ordinal (0 EvenOdd, 1 NonZero), in the
' per-kind aux slot 21 no polygon otherwise writes or reads -- the POLYLINE_FIRST/LAST
' aliasing precedent. Stroked text is also a polygon and leaves it 0, so text stays
' even-odd.
LET __CANVAS_GEO_POLYGON_RULE AS Integer = 21
LET __CANVAS_GEO_ARC AS Integer = 3
' plan-116-G. 8 is the next free value; a Group's header carries this rather than
' NONE so the deferred-hash path and the renderers can tell "a group node" from
' "an item with no geometry", which are different things.
LET __CANVAS_GEO_GROUP AS Integer = 8
' bug-484. A Picture: the destination rectangle in the rectangle's own slots (centre
' 2-3, half-extent 4-5, radius 6 = 0), so its coverage and stroke ARE a rectangle's;
' the image's width and height in the per-kind aux pair 20-21; and its pixel block's
' address in two 24-bit halves. Halves because a slot holding a whole 48-bit address
' would not survive the float round trips the renderers make through `toInt` (the font
' handle note in `__canvas_hashItem` measures exactly that overflow).
' 35-36 are the cap slots, which only Line and Arc read.
LET __CANVAS_GEO_PICTURE AS Integer = 9
LET __CANVAS_GEO_PICTURE_SHADOW_HI AS Integer = 35
LET __CANVAS_GEO_PICTURE_SHADOW_LO AS Integer = 36
LET __CANVAS_GEO_PICTURE_SPLIT AS Integer = 16777216
' plan-173. A Tile uses the picture image words and rectangle destination slots, with
' its requested source rectangle in a four-float tail.
LET __CANVAS_GEO_TILE AS Integer = 13
LET __CANVAS_GEO_TILE_SRC_X AS Integer = 69
LET __CANVAS_GEO_TILE_SRC_Y AS Integer = 70
LET __CANVAS_GEO_TILE_SRC_W AS Integer = 71
LET __CANVAS_GEO_TILE_SRC_H AS Integer = 72
' plan-154-A. A PolyLine: stroke-as-fill like a Line (the stroke colour in the fill
' slots, the half-width in the radius slot 6, slot 7 at -1), its segment count in the
' aux slot 20 as a polygon's edge count is, and a tail of that many edges in the
' polygon layout followed by its per-point colours as gradient stops. The first and
' last segments with length go in the gradient axis slots 43-44, which a polyline does
' not otherwise use: its colours are parameterised by the path, not by an axis.
LET __CANVAS_GEO_POLYLINE AS Integer = 10
LET __CANVAS_GEO_POLYLINE_FIRST AS Integer = 43
LET __CANVAS_GEO_POLYLINE_LAST AS Integer = 44
' plan-164-A. The two curve shapes, as POLYLINE and POLYGON are two kinds: both carry
' the same tail of rational-quadratic pieces (`helper_curves.rs`), and the kind says
' whether there is an interior.
LET __CANVAS_GEO_POLYCURVE AS Integer = 11
LET __CANVAS_GEO_CURVEPOLYGON AS Integer = 12"#;

/// The cache, as flat lists rather than a list of records.
///
/// A `GeoCacheEntry` record would have to be declared in the `canvas` package, where
/// it would appear in `mfb man canvas types` as a type no program can name or use —
/// the same reason `__CANVAS_KIND_*` are bare integers. Flat lists also match how the
/// data is consumed: the rasteriser walks `__CANVAS_GEO_DATA` linearly and never wants a
/// whole entry at once.
///
/// bug-686: the store is kept natively (`func_geo_cache.rs`): `__CANVAS_GEO_SLOTS` holds
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
const GEO_CACHE_STATE: &str =
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
const GEO_HASH: &str =
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

/// The fixed header builders, one per kind.
///
/// Every builder writes the same 51 slots, so the rasteriser and the emitters can be
/// written once against the layout instead of per kind. `__canvas_geometryFor` picks the
/// builder in one `MATCH` that is exhaustive over the frozen `DrawItem` set, so a new
/// variant fails to compile there rather than silently generating nothing.
#[rustfmt::skip]
const GEO_HEADER: &str =
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

FUNC __canvas_strokeAsFill(h AS List OF Float) AS List OF Float
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
  ' An arc's box goes over whole: its distance jumps at the ends of its sweep, so no rim
  ' bounds where it takes coverage under a transform (`__canvas_boundsHeader`).
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
  LET shaped AS List OF Integer = __canvas_shapeRun(handles, t.text)
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
  ' The file, for the outlines `__canvas_glyphEntry` rasterises on a miss: the one read
  ' that needs it, everything else coming from the face index (plan-162-A). It is
  ' fetched again only when the run moves to another font (plan-162-C), so a one-font
  ' run fetches it once.
  MUT b AS List OF Byte = []
  MUT bFont AS Integer = 0 - 1
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
      IF which <> bFont THEN
        b = __canvas_fontBlob(font)
        bFont = which
      END IF
      entry = __canvas_glyphEntry(b, font, gid, t.fontSize, scale)
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

' Each glyph's outline comes from the file of the font its triple names -- the only
' thing a file is fetched for -- fetched again only when the run moves to another font.
FUNC __canvas_textEdges(t AS Text) AS List OF Float
  __CANVAS_RING_MARKS = []
  LET handles AS List OF Integer = __canvas_textHandles(t)
  LET scales AS List OF Float = __canvas_fontScales(handles, t.fontSize)
  LET shaped AS List OF Integer = __canvas_shapeRun(handles, t.text)
  IF NOT __canvas_runDrawable(shaped, scales) THEN
    RETURN []
  END IF
  MUT b AS List OF Byte = []
  MUT bFont AS Integer = 0 - 1
  MUT edges AS List OF Float = []
  MUT pen AS Float = __canvas_textX(t)
  MUT c AS Integer = 0
  __CANVAS_RING_MARKING = TRUE
  WHILE c * __CANVAS_RUN_STRIDE < len(shaped)
    LET which AS Integer = collections::getOr(shaped, c * __CANVAS_RUN_STRIDE + __CANVAS_RUN_FONT, 0)
    LET scale AS Float = collections::getOr(scales, which, 0.0)
    IF scale > 0.0 THEN
      IF which <> bFont THEN
        b = __canvas_fontBlob(collections::getOr(handles, which, 0))
        bFont = which
      END IF
      edges = __canvas_glyphEdges(b, collections::getOr(shaped, c * __CANVAS_RUN_STRIDE + __CANVAS_RUN_GLYPH, 0), scale, pen, __canvas_textY(t), edges)
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
const GEO_CACHE: &str =
r#"' Count a slot as used by the frame in progress, once per frame however many times the
' scene names it. What `canvas::geoBeginFrame` knows about the live set comes from here.
FUNC __canvas_geoStamp(slot AS Integer) AS Nothing
  LET at AS Integer = slot * 4
  IF collections::getOr(__CANVAS_GEO_SLOTS, at + 3, 0) <> __CANVAS_GEO_FRAME THEN
    __CANVAS_GEO_SLOTS = collections::set(__CANVAS_GEO_SLOTS, at + 3, __CANVAS_GEO_FRAME)
    __CANVAS_GEO_USED_FLOATS = __CANVAS_GEO_USED_FLOATS + collections::getOr(__CANVAS_GEO_SLOTS, at + 2, 0)
  END IF
END FUNC

' plan-172-L: a `Picture` is not built here. Its records are made, cached and -- the
' one kind a hash cannot vouch for, since `setBytes` swaps an image's pixels without
' changing the item -- confirmed in `helper_picture.rs`, which the walks hand a picture
' to before they get here (`__canvas_appendDraw`, `__canvas_memoGroup`).
FUNC __canvas_geometryFor(item AS DrawItem, hash AS Integer) AS Integer
  MATCH item
    CASE Tile(tile)
      RETURN __canvas_tileGeometry(tile, hash)
    CASE ELSE
  END MATCH
  LET slot AS Integer = canvas::geoFind(hash)
  IF slot >= 0 THEN
    __canvas_geoStamp(slot)
    RETURN collections::getOr(__CANVAS_GEO_SLOTS, slot * 4 + 1, 0)
  END IF

  ' A miss: a new slot.
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
    RETURN canvas::geoInsert(hash, native, [])
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
    RETURN canvas::geoInsert(hash, header, tail)
  END IF
  MUT fixed AS List OF Float = []
  MUT i AS Integer = 0
  WHILE i < __CANVAS_GEO_HEADER
    fixed = collections::append(fixed, collections::getOr(header, i, 0.0))
    i = i + 1
  END WHILE
  RETURN canvas::geoInsert(hash, fixed, tail)
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
  LET line AS List OF Float = __canvas_runLine(handles, scales, __canvas_shapeRun(handles, t.text))
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

' Everything in a `Paint`: both colours, the stroke width, the blend mode, the clip, the
' transform, and the fill gradient with its stops. A gradient on a kind that ignores it
' (a `Line`, an `Arc`, `Text`) only splits cache entries that draw identically, which
' costs a build and never a wrong picture.
FUNC __canvas_hashPaint(acc AS Integer, paint AS Paint, transform AS Transform) AS Integer
  MUT h AS Integer = acc
  ' Each colour's four channels (0-255 each) as one packed value: one step, not four.
  h = __canvas_hashStep(h, ((toInt(paint.fill.red) * 256 + toInt(paint.fill.green)) * 256 + toInt(paint.fill.blue)) * 256 + toInt(paint.fill.alpha))
  h = __canvas_hashStep(h, ((toInt(paint.stroke.red) * 256 + toInt(paint.stroke.green)) * 256 + toInt(paint.stroke.blue)) * 256 + toInt(paint.stroke.alpha))
  h = __canvas_hashFloat(h, paint.strokeWidth)
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
  ' The clip, the transform and the gradient are folded in only when present, after
  ' ONE step carrying which of them are (with the blend mode). Their zero values are
  ' the no-ops -- an empty clip ring list is "unclipped", an all-zero Transform the
  ' identity, fewer than two stops no gradient -- and most paints set none of them, so
  ' this is what keeps a plain item's hash to a handful of steps.
  LET t AS Transform = transform
  LET hasClip AS Boolean = len(paint.clip) > 0
  LET hasTransform AS Boolean = NOT (t.a = 0.0 AND t.b = 0.0 AND t.c = 0.0 AND t.d = 0.0 AND t.tx = 0.0 AND t.ty = 0.0)
  LET stopCount AS Integer = len(paint.gradient.stops)
  MUT flags AS Integer = 0
  IF hasClip THEN
    flags = flags + 1
  END IF
  IF hasTransform THEN
    flags = flags + 2
  END IF
  IF stopCount >= 2 THEN
    flags = flags + 4
  END IF
  h = __canvas_hashStep(h, blend * 8 + flags)
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
  ' A gradient's points and kind mean nothing without two stops: the renderers read
  ' those slots only when the count says there is a ramp.
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
    ' plan-163-D: the spread and stroke apply as one step, then the transform, the focal
    ' offset and the focal radius.
    MUT ramp AS Integer = 0
    IF paint.gradient.spread = GradientSpread.Reflect THEN
      ramp = 1
    END IF
    IF paint.gradient.spread = GradientSpread.Repeat THEN
      ramp = 2
    END IF
    IF paint.gradient.strokeApply = GradientApply.FillAndStroke THEN
      ramp = ramp + 4
    END IF
    LET gt AS Transform = paint.gradient.transform
    h = __canvas_hashStep(h, ramp)
    h = __canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(h, gt.a), gt.b), gt.c)
    h = __canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(h, gt.d), gt.tx), gt.ty)
    h = __canvas_hashFloat(__canvas_hashPoint(h, paint.gradient.focalOffset), paint.gradient.focalRadius)
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
  h = __canvas_hashFloat(h, paint.dashOffset)
  ' plan-163-C.
  RETURN __canvas_hashFloat(h, paint.transparency)
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
      h = __canvas_hashText(h, t.id)
      ' plan-162-D: the spacing moves every glyph after the first, so it keys the cache --
      ' a spacing-only change must rebuild. Folded only when set, so an unspaced `Text`
      ' keeps the key it always had.
      IF t.letterSpacing <> 0.0 THEN
        h = __canvas_hashFloat(h, t.letterSpacing)
      END IF
      RETURN __canvas_hashPaint(h, t.paint, t.transform)
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

/// bug-686: the check on `canvas::geoBuild`, the native geometry builder — normal-build
/// body.
///
/// A normal build has nothing to check against and nowhere to report, so the hook is an
/// empty `FUNC … AS Nothing`, like `__canvas_phaseMark`.
#[rustfmt::skip]
const GEO_VERIFY: &str = r#"FUNC __canvas_geoVerify(item AS DrawItem, native AS List OF Float) AS Nothing
END FUNC

FUNC __canvas_geoVerifyFrame(pending AS Integer) AS Nothing
END FUNC

FUNC __canvas_drawsVerify() AS Nothing
END FUNC"#;

/// [`GEO_VERIFY`] for a `--debug` build.
///
/// Reports every record the native builder produced (`geoNative=` on the stats line,
/// `__CANVAS_GEO_NATIVE_BUILDS`), and while `MFB_CANVAS_GEO_VERIFY=1` builds the same
/// item's record with the MFBASIC builders and counts the ones that differ in any bit
/// (`geoVerified=`, `geoVerifyMismatches=`) — both the records `__canvas_geometryFor`
/// built through `canvas::geoBuild` and the ones `canvas::sceneResolve` built in its
/// frame pass (`__canvas_geoVerifyFrame`), plus the draw hashes that pass folded, and
/// the draw list `canvas::sceneDrawsFlat` laid out (`__canvas_drawsVerify`, counted in
/// `drawsVerified=`; its mismatches count in `geoVerifyMismatches=` too). Records are
/// compared with `canvas::geoSame`, which compares the 64-bit patterns: a float `=`
/// would call `-0.0` and `0.0` the same, and the software rasteriser's goldens would not. The variable is read once and cached (0 unresolved,
/// 1 off, 2 on), like `__canvas_dumpRequested`.
///
/// `__canvas_geoReference` is `__canvas_geometryFor`'s own arms for the five kinds the
/// native builder takes: the header followed by the tail, as the cache appends them.
#[rustfmt::skip]
const GEO_VERIFY_DEBUG: &str = r#"MUT __CANVAS_GEO_VERIFY_MODE AS Integer = 0
MUT __CANVAS_GEO_VERIFIED AS Integer = 0
MUT __CANVAS_DRAWS_VERIFIED AS Integer = 0
MUT __CANVAS_GEO_MISMATCHES AS Integer = 0
MUT __CANVAS_GEO_RESOLVED_CHECKED AS Integer = 0

' The record `__canvas_geometryFor` would build for `item`, from its own arms, for every
' kind. `offset` is where the record under test starts, or -1: a glyph run's entries are
' glyph-cache INDICES, which only the cache can assign, so they are taken from the
' record under test -- each checked against the glyph key it must name -- and everything
' else about the run (its length, pens, baseline, header) is rebuilt.
FUNC __canvas_geoReference(item AS DrawItem, offset AS Integer) AS List OF Float
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
    CASE Arc(a)
      header = __canvas_arcHeader(a)
      LET dashes AS List OF Float = __canvas_arcDashes(header, a)
      tail = __canvas_strokeRampTail(dashes, a.paint)
      header = __canvas_withDashes(header, dashes, a.paint)
    ' plan-172-L: a picture's records have ONE builder (`helper_picture.rs`) -- the
    ' native one declines a picture -- so there is no second answer to hold one against,
    ' and the record under test is its own reference. What a shaped picture's outline is
    ' built from IS checked, as the item it is, where it is built
    ' (`__canvas_geoVerify`).
    CASE Picture(pic)
      IF offset < 0 THEN
        RETURN __canvas_emptyHeader()
      END IF
      RETURN collections::mid(__CANVAS_GEO_DATA, offset, toInt(collections::getOr(__CANVAS_GEO_DATA, offset + 1, 0.0)))
    CASE Tile(tile)
      IF offset < 0 THEN
        RETURN __canvas_emptyHeader()
      END IF
      RETURN collections::mid(__CANVAS_GEO_DATA, offset, toInt(collections::getOr(__CANVAS_GEO_DATA, offset + 1, 0.0)))
    CASE Ellipse(e)
      header = __canvas_ellipseHeader(e)
      LET dashes AS List OF Float = __canvas_ellipseDashes(header, e)
      tail = __canvas_appendGradientTail(dashes, e.paint)
      header = __canvas_withDashes(header, dashes, e.paint)
    CASE Text(t)
      IF __canvas_strokeHalf(t.paint) > 0.0 THEN
        LET edges AS List OF Float = __canvas_textEdges(t)
        LET joins AS List OF Float = __canvas_joinTail(edges, __CANVAS_RING_MARKS, TRUE, __canvas_strokeHalf(t.paint), t.paint)
        header = __canvas_textHeader(t, edges)
        LET dashes AS List OF Float = __canvas_edgeDashes(header, t.paint, __canvas_strokeHalf(t.paint), edges, __CANVAS_RING_MARKS, TRUE)
        tail = collections::append(collections::append(edges, joins), dashes)
        header = __canvas_withDashes(__canvas_withJoins(header, joins, __canvas_joinReach(__canvas_strokeHalf(t.paint), t.paint)), dashes, t.paint)
      ELSE
        tail = __canvas_geoRunReference(t, offset)
        header = __canvas_textHeader(t, tail)
      END IF
      ' plan-172-K: the layout box `__canvas_geometryFor` writes into a text's record
      ' in a program with an `AutoPaint`.
      header = __canvas_autoTextBox(header, t)
    CASE Group(g)
      header = __canvas_emptyHeader()
    ' plan-172-K: the record the frame holds for an `AutoPaint`, before its resolve, is
    ' its shape's with the shape's own paint -- so that is its reference. Found by
    ' `scripts/test-canvas-gpu-rows.sh`, whose first `AutoPaint` rows counted three
    ' mismatches: this arm answered the empty header, and the two labels had no box.
    CASE AutoPaint(a)
      RETURN __canvas_geoReference(__canvas_autoItem(a), offset)
    ' plan-150-A: expanded before any record is built for it, like a group.
    CASE ParticleSystem(ps)
      header = __canvas_emptyHeader()
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
  LET clipped AS List OF List OF Float = __canvas_withClipEdges(header, tail, item)
  MUT out AS List OF Float = collections::getOr(clipped, 0, header)
  out = collections::append(out, collections::getOr(clipped, 1, tail))
  RETURN out
END FUNC

' `__canvas_textGlyphRun` without touching the glyph cache: each glyph's entry is read
' from the record at `offset` and kept only if the cache entry it names holds this
' glyph's key (else -2, which no run holds).
FUNC __canvas_geoRunReference(t AS Text, offset AS Integer) AS List OF Float
  LET handles AS List OF Integer = __canvas_textHandles(t)
  LET scales AS List OF Float = __canvas_fontScales(handles, t.fontSize)
  LET shaped AS List OF Integer = __canvas_shapeRun(handles, t.text)
  IF NOT __canvas_runDrawable(shaped, scales) THEN
    RETURN []
  END IF
  MUT run AS List OF Float = []
  MUT pen AS Float = __canvas_textX(t)
  ' The record's triples are only the drawn glyphs', so the stored one is counted apart.
  MUT at AS Integer = 0
  MUT c AS Integer = 0
  WHILE c * __CANVAS_RUN_STRIDE < len(shaped)
    LET which AS Integer = collections::getOr(shaped, c * __CANVAS_RUN_STRIDE + __CANVAS_RUN_FONT, 0)
    LET scale AS Float = collections::getOr(scales, which, 0.0)
    IF scale > 0.0 THEN
      LET gid AS Integer = collections::getOr(shaped, c * __CANVAS_RUN_STRIDE + __CANVAS_RUN_GLYPH, 0)
      MUT entry AS Integer = 0 - 2
      IF offset >= 0 THEN
        LET stored AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, offset + __CANVAS_GEO_HEADER + at * 3, 0.0 - 2.0))
        IF stored >= 0 AND collections::getOr(__CANVAS_GLYPH_KEYS, stored, 0 - 1) = __canvas_glyphKey(collections::getOr(handles, which, 0), __canvas_sizeQ(t.fontSize), gid) THEN
          entry = stored
        END IF
      END IF
      run = collections::append(run, toFloat(entry))
      run = collections::append(run, toFloat(toInt(pen + 0.5)))
      run = collections::append(run, toFloat(toInt(__canvas_textY(t) + 0.5)))
      at = at + 1
    END IF
    pen = pen + toFloat(collections::getOr(shaped, c * __CANVAS_RUN_STRIDE + __CANVAS_RUN_ADVANCE, 0)) * scale
    IF t.letterSpacing <> 0.0 AND collections::getOr(shaped, c * __CANVAS_RUN_STRIDE + __CANVAS_RUN_GAP, 0) = 1 THEN
      pen = pen + t.letterSpacing
    END IF
    c = c + 1
  END WHILE
  RETURN run
END FUNC

FUNC __canvas_geoVerifying() AS Boolean
  IF __CANVAS_GEO_VERIFY_MODE = 0 THEN
    __CANVAS_GEO_VERIFY_MODE = 1
    IF os::getEnvOr("MFB_CANVAS_GEO_VERIFY", "") = "1" THEN
      __CANVAS_GEO_VERIFY_MODE = 2
    END IF
  END IF
  RETURN __CANVAS_GEO_VERIFY_MODE = 2
END FUNC

FUNC __canvas_geoVerify(item AS DrawItem, native AS List OF Float) AS Nothing
  IF __canvas_geoVerifying() THEN
    __CANVAS_GEO_VERIFIED = __CANVAS_GEO_VERIFIED + 1
    IF NOT canvas::geoSame(native, __canvas_geoReference(item, 0 - 1)) THEN
      __CANVAS_GEO_MISMATCHES = __CANVAS_GEO_MISMATCHES + 1
    END IF
  END IF
END FUNC

' The frame `canvas::sceneResolve` just laid out: every record it built natively,
' against the MFBASIC builders, and -- when no index went to MFBASIC, so the draw
' entries are the scene's one for one -- every draw hash it folded, against
' `__canvas_hashFloat`.
FUNC __canvas_geoVerifyFrame(pending AS Integer) AS Nothing
  IF NOT __canvas_geoVerifying() THEN
    RETURN
  END IF
  IF len(__CANVAS_GEO_BUILT) > 0 THEN
    LET items AS List OF DrawItem = __canvas_frameItems()
    LET none AS DrawItem = __canvas_noItem()
    MUT k AS Integer = 0
    WHILE k + 1 < len(__CANVAS_GEO_BUILT)
      LET index AS Integer = collections::getOr(__CANVAS_GEO_BUILT, k, 0)
      LET slot AS Integer = collections::getOr(__CANVAS_GEO_BUILT, k + 1, 0)
      LET offset AS Integer = collections::getOr(__CANVAS_GEO_SLOTS, slot * 4 + 1, 0)
      LET count AS Integer = collections::getOr(__CANVAS_GEO_SLOTS, slot * 4 + 2, 0)
      LET record AS List OF Float = collections::mid(__CANVAS_GEO_DATA, offset, count)
      __CANVAS_GEO_VERIFIED = __CANVAS_GEO_VERIFIED + 1
      IF NOT canvas::geoSame(record, __canvas_geoReference(collections::getOr(items, index, none), offset)) THEN
        __CANVAS_GEO_MISMATCHES = __CANVAS_GEO_MISMATCHES + 1
      END IF
      k = k + 2
    END WHILE
  END IF
  ' Every index the frame resolved, whatever its kind and however it was resolved --
  ' a hit included: the record at its offset must be the one the item the frame draws
  ' at that index builds. A hit is trusted on its hash alone, so this is the check that
  ' the hash and the item came from the same scene.
  LET top AS Integer = len(__CANVAS_TOP_OFFSETS)
  IF top > 0 THEN
    LET drawn AS List OF DrawItem = __canvas_frameItems()
    LET absent AS DrawItem = __canvas_noItem()
    MUT r AS Integer = 0
    WHILE r < top
      LET at AS Integer = collections::getOr(__CANVAS_TOP_OFFSETS, r, 0 - 1)
      IF at >= 0 THEN
        LET length AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, at + 1, 0.0))
        __CANVAS_GEO_RESOLVED_CHECKED = __CANVAS_GEO_RESOLVED_CHECKED + 1
        IF NOT canvas::geoSame(collections::mid(__CANVAS_GEO_DATA, at, length), __canvas_geoReference(collections::getOr(drawn, r, absent), at)) THEN
          __CANVAS_GEO_MISMATCHES = __CANVAS_GEO_MISMATCHES + 1
        END IF
      END IF
      r = r + 1
    END WHILE
  END IF
  IF pending = 0 THEN
    LET hashes AS List OF Integer = __CANVAS_FRAME_HASHES
    MUT i AS Integer = 0
    WHILE i < len(__CANVAS_DRAW_HASHES) AND i < len(hashes)
      LET folded AS Integer = __canvas_hashFloat(__canvas_hashFloat(collections::getOr(hashes, i, 0), 0.0), 0.0)
      IF collections::getOr(__CANVAS_DRAW_HASHES, i, 0) <> folded THEN
        __CANVAS_GEO_MISMATCHES = __CANVAS_GEO_MISMATCHES + 1
      END IF
      i = i + 1
    END WHILE
  END IF
END FUNC

' The draw list `canvas::sceneDrawsFlat` laid out, against the MFBASIC walk it replaces
' on a frame with no group -- which is re-run here and left in place.
FUNC __canvas_drawsVerify() AS Nothing
  IF NOT __canvas_geoVerifying() THEN
    RETURN
  END IF
  LET draws AS List OF Integer = __CANVAS_DRAWS
  LET blocks AS List OF Integer = __CANVAS_DRAW_BLOCKS
  LET inst AS List OF Integer = __CANVAS_DRAW_INST
  LET nextInst AS Integer = __CANVAS_DRAW_NEXT_INST
  LET walked AS List OF Integer = __canvas_sceneDrawsWalk()
  __CANVAS_DRAWS_VERIFIED = __CANVAS_DRAWS_VERIFIED + 1
  IF NOT (__canvas_intListEquals(draws, walked) AND __canvas_intListEquals(blocks, __CANVAS_DRAW_BLOCKS) AND __canvas_intListEquals(inst, __CANVAS_DRAW_INST) AND nextInst = __CANVAS_DRAW_NEXT_INST) THEN
    __CANVAS_GEO_MISMATCHES = __CANVAS_GEO_MISMATCHES + 1
  END IF
END FUNC

FUNC __canvas_geoVerifyText() AS String
  RETURN " geoNative=" & toString(__CANVAS_GEO_NATIVE_BUILDS) & " geoVerified=" & toString(__CANVAS_GEO_VERIFIED) & " geoVerifyMismatches=" & toString(__CANVAS_GEO_MISMATCHES) & " drawsVerified=" & toString(__CANVAS_DRAWS_VERIFIED) & " geoResolvedChecked=" & toString(__CANVAS_GEO_RESOLVED_CHECKED) & " snapshotRehashed=" & toString(__CANVAS_SNAP_REHASHED)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    for helper in RegistryHelper::debug_split("canvas_geoVerify", GEO_VERIFY, GEO_VERIFY_DEBUG) {
        pkg.add_helper(helper);
    }
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
mod tests;
