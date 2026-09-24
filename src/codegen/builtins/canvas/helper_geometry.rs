//! Geometry generation and the geometry cache.
//!
//! A `DrawItem` is what the *program* wrote; **geometry** is what the *renderer*
//! draws. Generation turns one into the other: a fixed 47-float header carrying the
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
//! The probe is by hash, but a hit is confirmed by comparing the 47-float header
//! exactly before the tail is reused. A hash alone would let a collision reuse
//! another item's geometry and silently draw the wrong picture — a rare wrong answer
//! is worse than a common slow one, and the confirmation costs 47 float compares
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
r#"LET __CANVAS_GEO_HEADER AS Integer = 47
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
LET __CANVAS_GEO_PICTURE_SPLIT AS Integer = 16777216"#;

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
/// Every builder writes the same 47 slots, so the rasteriser and the emitters can be
/// written once against the layout instead of per kind. `__canvas_geometryFor` picks the
/// builder in one `MATCH` that is exhaustive over the frozen `DrawItem` set, so a new
/// variant fails to compile there rather than silently generating nothing.
#[rustfmt::skip]
const GEO_HEADER: &str =
r#"' bug-686: a copy of one prebuilt all-zero header rather than 47 appends -- one small
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
  RETURN 0
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
  MUT stopCount AS Integer = len(paint.fillGradient.stops)
  IF kind = __CANVAS_GEO_TEXT THEN
    stopCount = 0
  END IF
  IF kind = __CANVAS_GEO_NONE THEN
    stopCount = 0
  END IF
  IF kind = __CANVAS_KIND_SEGMENT THEN
    stopCount = 0
  END IF
  IF kind = __CANVAS_GEO_ARC THEN
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
  IF paint.fillGradient.kind = GradientKind.Radial THEN
    gkind = 1.0
  END IF
  out = collections::set(out, __CANVAS_GEO_GRADIENT_KIND, gkind)
  out = collections::set(out, __CANVAS_GEO_GRADIENT_FROMX, paint.fillGradient.startPoint.x)
  out = collections::set(out, __CANVAS_GEO_GRADIENT_FROMY, paint.fillGradient.startPoint.y)
  out = collections::set(out, __CANVAS_GEO_GRADIENT_TOX, paint.fillGradient.endPoint.x)
  out = collections::set(out, __CANVAS_GEO_GRADIENT_TOY, paint.fillGradient.endPoint.y)
  ' plan-116-C: the transform, INVERTED once here rather than per pixel in each of
  ' three renderers. `__canvas_invertTransform` is also the only place that knows an
  ' all-zero `Transform` means the identity, and the only place that decides what a
  ' singular one does.
  '
  ' bug-686: the all-zero transform -- every item that never set one -- is written as
  ' the identity's two non-zero bit patterns (1.0f is 1065353216) into slots that are
  ' already zero, rather than inverted into a fresh seven-float list and copied. Exactly
  ' the values `__canvas_invertTransform` returns for it.
  LET t AS Transform = paint.transform
  IF t.a = 0.0 AND t.b = 0.0 AND t.c = 0.0 AND t.d = 0.0 AND t.tx = 0.0 AND t.ty = 0.0 THEN
    out = collections::set(out, 27, 1065353216.0)
    out = collections::set(out, 30, 1065353216.0)
    RETURN out
  END IF
  LET inv AS List OF Float = __canvas_invertTransform(t)
  MUT ti AS Integer = 0
  WHILE ti < 7
    out = collections::set(out, 27 + ti, collections::getOr(inv, ti, 0.0))
    ti = ti + 1
  END WHILE
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

' The bounds are computed in SHAPE space by every generator. Under a transform the item
' covers the FORWARD-mapped rectangle, so the header has to carry that rectangle's
' axis-aligned hull instead -- a rotated square whose bounds were left untransformed
' would be clipped to its own unrotated box, losing its corners.
'
' The forward matrix is recovered by inverting the stored inverse rather than carried in
' six more slots. Two reasons that is safe: a bounding box only has to be CONSERVATIVE,
' so the float32 round-trip's last-bit error cannot matter, and the result is padded by
' a pixel anyway. Extra pixels cost a coverage evaluation that returns 0; missing ones
' cut the shape.
FUNC __canvas_boundsHeader(h AS List OF Float, minX AS Float, minY AS Float, maxX AS Float, maxY AS Float) AS List OF Float
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
    IF TRUE THEN
      LET x0 AS Float = fa * minX + fc * minY + ftx
      LET y0 AS Float = fb * minX + fd * minY + fty
      LET x1 AS Float = fa * maxX + fc * minY + ftx
      LET y1 AS Float = fb * maxX + fd * minY + fty
      LET x2 AS Float = fa * minX + fc * maxY + ftx
      LET y2 AS Float = fb * minX + fd * maxY + fty
      LET x3 AS Float = fa * maxX + fc * maxY + ftx
      LET y3 AS Float = fb * maxX + fd * maxY + fty
      out = collections::set(out, 16, __canvas_minF(__canvas_minF(x0, x1), __canvas_minF(x2, x3)) - 1.0)
      out = collections::set(out, 17, __canvas_minF(__canvas_minF(y0, y1), __canvas_minF(y2, y3)) - 1.0)
      out = collections::set(out, 18, __canvas_maxF(__canvas_maxF(x0, x1), __canvas_maxF(x2, x3)) + 1.0)
      out = collections::set(out, 19, __canvas_maxF(__canvas_maxF(y0, y1), __canvas_maxF(y2, y3)) + 1.0)
      RETURN out
    END IF
  END IF
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

' bug-484: a picture's header -- a rectangle's, re-kinded, plus what the sampler needs.
'
' The image is read through `canvas::imageShadow` and friends, which answer 0 for a
' destroyed image instead of raising: this runs on the graphics thread, over a scene the
' program may have built before destroying what it names. Any of the three reading 0 --
' including a destroy landing between the reads -- is "no such image" and the item draws
' nothing, exactly as a zero-area rectangle does. The width and height never change and
' the shadow is never freed, so a header built from a live read stays drawable for the
' rest of the frame whatever the worker does next (`.ai/canvas-threading.md` section 7).
'
' The shadow address being IN the header is what makes a `setBytes` visible: it swaps a
' fresh block in, so the header differs, the cache confirmation fails, and the damage
' diff sees a changed item -- although nothing in the scene the program wrote changed.
FUNC __canvas_pictureHeader(pic AS Picture) AS List OF Float
  LET shadow AS Integer = canvas::imageShadow(pic.image)
  LET iw AS Integer = canvas::imageWidthOf(pic.image)
  LET ih AS Integer = canvas::imageHeightOf(pic.image)
  IF shadow = 0 OR iw <= 0 OR ih <= 0 THEN
    RETURN __canvas_emptyHeader()
  END IF
  LET base AS List OF Float = __canvas_rectHeader(pic.x, pic.y, pic.w, pic.h, 0.0, pic.paint)
  IF toInt(collections::getOr(base, 0, 0.0)) <> __CANVAS_KIND_RECT THEN
    RETURN base
  END IF
  MUT out AS List OF Float = base
  out = collections::set(out, 0, toFloat(__CANVAS_GEO_PICTURE))
  ' The Correction F18 trap, the other way round: `__canvas_paintHeader` ran while slot
  ' 0 still said RECT, a kind with an interior, so it counted a gradient's stops into
  ' slot 1 -- and `__canvas_geometryFor`'s Picture arm builds none, because the image, not
  ' a ramp, is what fills a picture. Undone here so the record declares only what it
  ' holds.
  out = collections::set(out, 1, toFloat(__CANVAS_GEO_HEADER))
  out = collections::set(out, __CANVAS_GEO_GRADIENT_COUNT, 0.0)
  out = collections::set(out, 20, toFloat(iw))
  out = collections::set(out, 21, toFloat(ih))
  out = collections::set(out, __CANVAS_GEO_PICTURE_SHADOW_HI, toFloat(shadow / __CANVAS_GEO_PICTURE_SPLIT))
  out = collections::set(out, __CANVAS_GEO_PICTURE_SHADOW_LO, toFloat(shadow MOD __CANVAS_GEO_PICTURE_SPLIT))
  RETURN out
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
FUNC __canvas_ellipseHeader(e AS Ellipse) AS List OF Float
  IF e.radiusX <= 0.0 THEN
    RETURN __canvas_emptyHeader()
  END IF
  IF e.radiusY <= 0.0 THEN
    RETURN __canvas_emptyHeader()
  END IF
  LET ca AS Float = __canvas_cos(e.angle)
  LET sa AS Float = __canvas_sin(e.angle)
  MUT out AS List OF Float = __canvas_blankHeader()
  out = collections::set(out, 0, toFloat(__CANVAS_GEO_ELLIPSE))
  out = collections::set(out, 1, toFloat(__CANVAS_GEO_HEADER))
  out = collections::set(out, 2, e.x)
  out = collections::set(out, 3, e.y)
  out = collections::set(out, 4, e.radiusX)
  out = collections::set(out, 5, e.radiusY)
  out = collections::set(out, __CANVAS_GEO_ELLIPSE_COS, ca)
  out = collections::set(out, __CANVAS_GEO_ELLIPSE_SIN, sa)
  out = __canvas_paintHeader(out, e.paint)
  LET pad AS Float = __canvas_maxF(__canvas_strokeHalf(e.paint), 0.0) + 1.0
  LET ex AS Float = e.radiusX * ca
  LET ey AS Float = e.radiusY * sa
  LET fx AS Float = e.radiusX * sa
  LET fy AS Float = e.radiusY * ca
  LET hx AS Float = math::sqrt(ex * ex + ey * ey) + pad
  LET hy AS Float = math::sqrt(fx * fx + fy * fy) + pad
  RETURN __canvas_boundsHeader(out, e.x - hx, e.y - hy, e.x + hx, e.y + hy)
END FUNC

FUNC __canvas_segmentHeader(x1 AS Float, y1 AS Float, x2 AS Float, y2 AS Float, cap AS Integer, paint AS Paint) AS List OF Float
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
  out = collections::set(out, __CANVAS_GEO_CAP, toFloat(__canvas_capTag(a.cap)))
  ' plan-116-D: the two sweep endpoints, in surface pixels. Per-shape constants, so
  ' they are computed ONCE here rather than per pixel in each of three renderers --
  ' which is the same reason the sweep vectors are turned into sin/cos here. The two
  ' extra multiply-adds ride on `__canvas_cos`/`__canvas_sin` calls the arc already
  ' makes, so a round-capped arc costs no extra transcendental.
  out = collections::set(out, __CANVAS_GEO_CAPSTARTX, a.x + a.radius * __canvas_cos(a.startAngle))
  out = collections::set(out, __CANVAS_GEO_CAPSTARTY, a.y + a.radius * __canvas_sin(a.startAngle))
  out = collections::set(out, __CANVAS_GEO_CAPENDX, a.x + a.radius * __canvas_cos(a.endAngle))
  out = collections::set(out, __CANVAS_GEO_CAPENDY, a.y + a.radius * __canvas_sin(a.endAngle))
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

' The five floats one stop contributes: offset, then its colour's four channels.
FUNC __canvas_gradientTail(paint AS Paint) AS List OF Float
  LET count AS Integer = len(paint.fillGradient.stops)
  MUT out AS List OF Float = []
  IF count < 2 THEN
    RETURN out
  END IF
  ' Offsets are clamped to 0..1 AND to the previous stop's, so the walk is monotonic
  ' without sorting -- sorting would silently redraw something other than what the
  ' program asked for (plan-116-F section 4.1). Clamping is visible and predictable.
  MUT prev AS Float = 0.0
  MUT i AS Integer = 0
  WHILE i < count
    LET st AS GradientStop = collections::getOr(paint.fillGradient.stops, i, GradientStop[offset := 0.0, color := __canvas_transparent()])
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
    out = collections::append(out, toFloat(toInt(st.color.alpha)))
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
  LET b AS List OF Byte = __canvas_fontBlob(canvas::fontHandle(t.font))
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
    LET entry AS Integer = __canvas_glyphEntry(b, canvas::fontHandle(t.font), gid, t.size, scale)
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
  LET b AS List OF Byte = __canvas_fontBlob(canvas::fontHandle(t.font))
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
  LET pad AS Float = __canvas_maxF(__canvas_strokeHalf(t.paint), 0.0) + 1.0
  RETURN __canvas_boundsHeader(out, minX - pad, minY - pad, maxX + pad, maxY + pad)
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
SUB __canvas_geoStamp(slot AS Integer)
  LET at AS Integer = slot * 4
  IF collections::getOr(__CANVAS_GEO_SLOTS, at + 3, 0) <> __CANVAS_GEO_FRAME THEN
    __CANVAS_GEO_SLOTS = collections::set(__CANVAS_GEO_SLOTS, at + 3, __CANVAS_GEO_FRAME)
    __CANVAS_GEO_USED_FLOATS = __CANVAS_GEO_USED_FLOATS + collections::getOr(__CANVAS_GEO_SLOTS, at + 2, 0)
  END IF
END SUB

' Is the geometry at `offset` still this picture's? Its header holds the pixel-block
' address current when it was built, split across two slots; `setBytes` swaps the block
' without changing the item, so this is the one cached kind a hash cannot vouch for.
FUNC __canvas_pictureIsCurrent(offset AS Integer, pic AS Picture) AS Boolean
  LET shadow AS Integer = canvas::imageShadow(pic.image)
  LET hi AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, offset + __CANVAS_GEO_PICTURE_SHADOW_HI, 0.0))
  LET lo AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, offset + __CANVAS_GEO_PICTURE_SHADOW_LO, 0.0))
  RETURN hi * __CANVAS_GEO_PICTURE_SPLIT + lo = shadow
END FUNC

FUNC __canvas_geometryFor(item AS DrawItem, hash AS Integer) AS Integer
  LET slot AS Integer = canvas::geoFind(hash)
  IF slot >= 0 THEN
    LET offset AS Integer = collections::getOr(__CANVAS_GEO_SLOTS, slot * 4 + 1, 0)
    MUT current AS Boolean = TRUE
    MATCH item
      CASE Picture(pic)
        IF toInt(collections::getOr(__CANVAS_GEO_DATA, offset, 0.0)) = __CANVAS_GEO_PICTURE THEN
          current = __canvas_pictureIsCurrent(offset, pic)
        END IF
      CASE ELSE
        current = TRUE
    END MATCH
    IF current THEN
      __canvas_geoStamp(slot)
      RETURN offset
    END IF
  END IF

  ' A miss -- or a picture whose image's pixels were swapped. Either way a new slot: the
  ' old one (if any) keeps its floats until the frame boundary, and the hash now names
  ' the new one.
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
      tail = __canvas_appendGradientTail([], r.paint)
      header = __canvas_rectHeader(r.x, r.y, r.w, r.h, 0.0, r.paint)
    CASE RoundedRect(rr)
      tail = __canvas_appendGradientTail([], rr.paint)
      header = __canvas_rectHeader(rr.x, rr.y, rr.w, rr.h, rr.cornerRadius, rr.paint)
    CASE Circle(c)
      tail = __canvas_appendGradientTail([], c.paint)
      header = __canvas_circleHeader(c.x, c.y, c.radius, c.paint)
    CASE Line(l)
      header = __canvas_segmentHeader(l.x1, l.y1, l.x2, l.y2, __canvas_capTag(l.cap), l.paint)
    CASE Arc(a)
      header = __canvas_arcHeader(a)
    CASE Polygon(p)
      IF len(p.points) >= 2 THEN
        tail = __canvas_appendGradientTail(__canvas_polygonEdges(p.points), p.paint)
      END IF
      header = __canvas_polygonHeader(p)
    CASE Picture(pic)
      header = __canvas_pictureHeader(pic)
    CASE Ellipse(e)
      tail = __canvas_appendGradientTail([], e.paint)
      header = __canvas_ellipseHeader(e)
    CASE Text(t)
      IF __canvas_strokeHalf(t.paint) > 0.0 THEN
        tail = __canvas_textEdges(t)
      ELSE
        tail = __canvas_textGlyphRun(t)
      END IF
      header = __canvas_textHeader(t, tail)
    ' plan-116-G: a Group has no geometry of its own -- it names one -- so no tail, and
    ' the empty header, whose NONE kind every renderer skips. It never reaches here from
    ' the walk (a group is expanded, not cached); the arm is what keeps MATCH exhaustive.
    CASE Group(g)
      header = __canvas_emptyHeader()
  END MATCH
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
  out = __canvas_paintHeader(out, t.paint)
  ' Slots 2 and 3 carry the font handle and the em size: a glyph run needs both to
  ' rasterise, and they are the shape parameters no other kind uses for text.
  out = collections::set(out, 2, toFloat(canvas::fontHandle(t.font)))
  out = collections::set(out, 3, t.size)
  out = collections::set(out, 20, toFloat(glyphs))
  ' The ink box comes from the metrics rather than from the outlines: this header is
  ' built on a cache miss, when the glyphs have not been rasterised yet, and an
  ' ascent/descent box always contains the ink. It is only used to clip and to
  ' invalidate, so a box that is too large costs nothing and one that is too small
  ' would clip the glyphs it was meant to bound.
  LET b AS List OF Byte = __canvas_fontBlob(canvas::fontHandle(t.font))
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

' bug-686: every item's hash is folded from its own FIELDS, kind by kind. It used to be
' a hash of the item's built geometry header -- 47 floats, the transform inverted, the
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

FUNC __canvas_hashPoint(acc AS Integer, p AS Point) AS Integer
  RETURN __canvas_hashFloat(__canvas_hashFloat(acc, p.x), p.y)
END FUNC

' Everything in a `Paint`: both colours, the stroke width, the blend mode, the clip, the
' transform, and the fill gradient with its stops. A gradient on a kind that ignores it
' (a `Line`, an `Arc`, `Text`) only splits cache entries that draw identically, which
' costs a build and never a wrong picture.
FUNC __canvas_hashPaint(acc AS Integer, paint AS Paint) AS Integer
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
  ' the no-ops -- an all-zero Bounds is "unclipped", an all-zero Transform the
  ' identity, fewer than two stops no gradient -- and most paints set none of them, so
  ' this is what keeps a plain item's hash to a handful of steps.
  LET c AS Bounds = paint.clip
  LET t AS Transform = paint.transform
  LET hasClip AS Boolean = NOT (c.x = 0.0 AND c.y = 0.0 AND c.w = 0.0 AND c.h = 0.0)
  LET hasTransform AS Boolean = NOT (t.a = 0.0 AND t.b = 0.0 AND t.c = 0.0 AND t.d = 0.0 AND t.tx = 0.0 AND t.ty = 0.0)
  LET stopCount AS Integer = len(paint.fillGradient.stops)
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
  IF hasClip THEN
    h = __canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(h, c.x), c.y), c.w), c.h)
  END IF
  IF hasTransform THEN
    h = __canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(h, t.a), t.b), t.c)
    h = __canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(h, t.d), t.tx), t.ty)
  END IF
  ' A gradient's points and kind mean nothing without two stops: the renderers read
  ' those slots only when the count says there is a ramp.
  IF stopCount >= 2 THEN
    MUT radial AS Integer = 0
    IF paint.fillGradient.kind = GradientKind.Radial THEN
      radial = 1
    END IF
    h = __canvas_hashStep(h, radial)
    h = __canvas_hashPoint(h, paint.fillGradient.startPoint)
    h = __canvas_hashPoint(h, paint.fillGradient.endPoint)
    h = __canvas_hashStep(h, stopCount)
    FOR EACH stop IN paint.fillGradient.stops
      h = __canvas_hashFloat(h, stop.offset)
      h = __canvas_hashStep(h, ((toInt(stop.color.red) * 256 + toInt(stop.color.green)) * 256 + toInt(stop.color.blue)) * 256 + toInt(stop.color.alpha))
    NEXT
  END IF
  RETURN h
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
      h = __canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(h, r.x), r.y), r.w), r.h)
      RETURN __canvas_hashPaint(h, r.paint)
    CASE RoundedRect(rr)
      MUT h AS Integer = __canvas_hashStart(2)
      h = __canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(h, rr.x), rr.y), rr.w), rr.h)
      h = __canvas_hashFloat(h, rr.cornerRadius)
      RETURN __canvas_hashPaint(h, rr.paint)
    CASE Circle(c)
      MUT h AS Integer = __canvas_hashStart(3)
      h = __canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(h, c.x), c.y), c.radius)
      RETURN __canvas_hashPaint(h, c.paint)
    CASE Line(l)
      MUT h AS Integer = __canvas_hashStart(4)
      h = __canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(h, l.x1), l.y1), l.x2), l.y2)
      h = __canvas_hashStep(h, __canvas_capTag(l.cap))
      RETURN __canvas_hashPaint(h, l.paint)
    CASE Arc(a)
      MUT h AS Integer = __canvas_hashStart(5)
      h = __canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(h, a.x), a.y), a.radius)
      h = __canvas_hashFloat(__canvas_hashFloat(h, a.startAngle), a.endAngle)
      h = __canvas_hashStep(h, __canvas_capTag(a.cap))
      RETURN __canvas_hashPaint(h, a.paint)
    CASE Polygon(p)
      MUT h AS Integer = __canvas_hashStart(6)
      h = __canvas_hashStep(h, len(p.points))
      FOR EACH q IN p.points
        h = __canvas_hashPoint(h, q)
      NEXT
      RETURN __canvas_hashPaint(h, p.paint)
    CASE Picture(pic)
      ' The image by its backend id: a destroyed image answers 0, so the item's key
      ' changes and its geometry is rebuilt as the empty header it now is. A `setBytes`
      ' does NOT change this key -- the pixel block is re-read on every frame instead
      ' (`canvas::sceneResolve`, `__canvas_pictureIsCurrent`).
      MUT h AS Integer = __canvas_hashStart(7)
      h = __canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(h, pic.x), pic.y), pic.w), pic.h)
      h = __canvas_hashStep(h, canvas::imageHandle(pic.image))
      RETURN __canvas_hashPaint(h, pic.paint)
    CASE Text(t)
      ' The font id is a resource HANDLE -- an address -- and must be folded in as the
      ' integer it is, never through `__canvas_hashFloat`, whose `value * 65536.0`
      ' overflows `toInt` for a 48-bit address. `__canvas_hashStep` reduces it first.
      MUT h AS Integer = __canvas_hashStart(8)
      h = __canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(h, t.x), t.y), t.size)
      h = __canvas_hashStep(h, canvas::fontHandle(t.font))
      h = __canvas_hashText(h, t.text)
      RETURN __canvas_hashPaint(h, t.paint)
    CASE Ellipse(e)
      MUT h AS Integer = __canvas_hashStart(9)
      h = __canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(h, e.x), e.y), e.radiusX), e.radiusY)
      h = __canvas_hashFloat(h, e.angle)
      RETURN __canvas_hashPaint(h, e.paint)
    ' plan-116-G: a group node is its name and its offset. Two nodes naming different
    ' groups at the same offset must not collide, and neither must one group at two.
    CASE Group(g)
      MUT h AS Integer = __canvas_hashStart(10)
      h = __canvas_hashFloat(__canvas_hashFloat(h, g.dx), g.dy)
      RETURN __canvas_hashText(h, g.name)
  END MATCH
END FUNC"#;

/// bug-686: the check on `canvas::geoBuild`, the native geometry builder — normal-build
/// body.
///
/// A normal build has nothing to check against and nowhere to report, so the hook is an
/// empty `SUB`, like `__canvas_phaseMark`.
#[rustfmt::skip]
const GEO_VERIFY: &str = r#"SUB __canvas_geoVerify(item AS DrawItem, native AS List OF Float)
END SUB

SUB __canvas_geoVerifyFrame(pending AS Integer)
END SUB

SUB __canvas_drawsVerify()
END SUB"#;

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
      tail = __canvas_appendGradientTail([], r.paint)
      header = __canvas_rectHeader(r.x, r.y, r.w, r.h, 0.0, r.paint)
    CASE RoundedRect(rr)
      tail = __canvas_appendGradientTail([], rr.paint)
      header = __canvas_rectHeader(rr.x, rr.y, rr.w, rr.h, rr.cornerRadius, rr.paint)
    CASE Circle(c)
      tail = __canvas_appendGradientTail([], c.paint)
      header = __canvas_circleHeader(c.x, c.y, c.radius, c.paint)
    CASE Line(l)
      header = __canvas_segmentHeader(l.x1, l.y1, l.x2, l.y2, __canvas_capTag(l.cap), l.paint)
    CASE Polygon(p)
      IF len(p.points) >= 2 THEN
        tail = __canvas_appendGradientTail(__canvas_polygonEdges(p.points), p.paint)
      END IF
      header = __canvas_polygonHeader(p)
    CASE Arc(a)
      header = __canvas_arcHeader(a)
    CASE Picture(pic)
      header = __canvas_pictureHeader(pic)
    CASE Ellipse(e)
      tail = __canvas_appendGradientTail([], e.paint)
      header = __canvas_ellipseHeader(e)
    CASE Text(t)
      IF __canvas_strokeHalf(t.paint) > 0.0 THEN
        tail = __canvas_textEdges(t)
      ELSE
        tail = __canvas_geoRunReference(t, offset)
      END IF
      header = __canvas_textHeader(t, tail)
    CASE Group(g)
      header = __canvas_emptyHeader()
  END MATCH
  MUT out AS List OF Float = header
  out = collections::append(out, tail)
  RETURN out
END FUNC

' `__canvas_textGlyphRun` without touching the glyph cache: each glyph's entry is read
' from the record at `offset` and kept only if the cache entry it names holds this
' glyph's key (else -2, which no run holds).
FUNC __canvas_geoRunReference(t AS Text, offset AS Integer) AS List OF Float
  LET font AS Integer = canvas::fontHandle(t.font)
  LET b AS List OF Byte = __canvas_fontBlob(font)
  IF len(b) = 0 THEN
    RETURN []
  END IF
  LET upem AS Integer = __canvas_fontUnitsPerEm(b)
  IF upem <= 0 THEN
    RETURN []
  END IF
  LET scale AS Float = t.size / toFloat(upem)
  LET cps AS List OF Integer = encoding::utf32Encode(t.text)
  MUT run AS List OF Float = []
  MUT pen AS Float = t.x
  MUT c AS Integer = 0
  WHILE c < len(cps)
    LET gid AS Integer = __canvas_glyphIndex(b, collections::getOr(cps, c, 0))
    MUT entry AS Integer = 0 - 2
    IF offset >= 0 THEN
      LET stored AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, offset + __CANVAS_GEO_HEADER + c * 3, 0.0 - 2.0))
      IF stored >= 0 AND collections::getOr(__CANVAS_GLYPH_KEYS, stored, 0 - 1) = __canvas_glyphKey(font, __canvas_sizeQ(t.size), gid) THEN
        entry = stored
      END IF
    END IF
    run = collections::append(run, toFloat(entry))
    run = collections::append(run, toFloat(toInt(pen + 0.5)))
    run = collections::append(run, toFloat(toInt(t.y + 0.5)))
    pen = pen + toFloat(__canvas_glyphAdvance(b, gid)) * scale
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

SUB __canvas_geoVerify(item AS DrawItem, native AS List OF Float)
  IF __canvas_geoVerifying() THEN
    __CANVAS_GEO_VERIFIED = __CANVAS_GEO_VERIFIED + 1
    IF NOT canvas::geoSame(native, __canvas_geoReference(item, 0 - 1)) THEN
      __CANVAS_GEO_MISMATCHES = __CANVAS_GEO_MISMATCHES + 1
    END IF
  END IF
END SUB

' The frame `canvas::sceneResolve` just laid out: every record it built natively,
' against the MFBASIC builders, and -- when no index went to MFBASIC, so the draw
' entries are the scene's one for one -- every draw hash it folded, against
' `__canvas_hashFloat`.
SUB __canvas_geoVerifyFrame(pending AS Integer)
  IF NOT __canvas_geoVerifying() THEN
    EXIT SUB
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
END SUB

' The draw list `canvas::sceneDrawsFlat` laid out, against the MFBASIC walk it replaces
' on a frame with no group -- which is re-run here and left in place.
SUB __canvas_drawsVerify()
  IF NOT __canvas_geoVerifying() THEN
    EXIT SUB
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
END SUB

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
