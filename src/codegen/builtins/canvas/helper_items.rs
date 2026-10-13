//! `__canvas_drawGeometry` — the one rasterisation loop.
//!
//! The rasteriser never sees a `DrawItem`. It reads the flat float record
//! `helper_geometry.rs` produced, which is the same buffer plan-98-E/F upload to the
//! GPU — so the oracle and the GPU backends consume one representation rather than
//! two that could drift.
//!
//! ## Why one loop, and why the pixel writes are inline
//!
//! Every `collections::set` on the surface **must** appear in the function that owns
//! the surface local, spelled `out = collections::set(out, i, v)`. That exact shape is
//! what `try_inplace_set_assign` recognises; anything else falls back to copying the
//! whole 2.3 MB list per write.
//!
//! This is not a micro-optimisation. Measured on 20,000 writes into a 200,000-byte
//! list:
//!
//! | shape | time |
//! |---|---|
//! | `out = collections::set(out, i, v)` on a local | **5 ms** |
//! | threaded through a helper's parameter and return | 1179 ms |
//! | through a module-level `MUT` | 1161 ms |
//!
//! 290x. The first version of this rasteriser had a `__canvas_blendPixel(surface, …)
//! AS List OF Byte` helper, and one 900x640 frame took **53 seconds**. A read
//! (`collections::getOr`) is free either way, so only the writes are constrained —
//! which is why the blend *arithmetic* still lives in `helper_color.rs` and only the
//! four `set` calls are inline here.
//!
//! Collapsing the four separate loops (rect/circle span, stroke span, arc, polygon)
//! into this one is what keeps that inlining from becoming four copies of the same
//! blend block. The loops only ever differed by their distance function, which is now
//! `__canvas_geoDistance`.
//!
//! ## Fill then stroke
//!
//! In that order, matching every 2D API a user is likely to have met: an outline drawn
//! under its own fill would be half-hidden by it. `Line` and `Arc` have no interior,
//! so their generator puts the *stroke* colour in the fill slots and leaves the stroke
//! half-width negative — one pass, no special case here.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

/// The half-width of a stroke, or a negative number when there is nothing to stroke.
///
/// Folding "is there a stroke at all" into the same value the band test uses keeps the
/// caller from repeating the two-part check (`alpha > 0` and `width > 0`) everywhere.
#[rustfmt::skip]
const STROKE_HALF: &str =
r#"FUNC __canvas_strokeHalf(paint AS Paint) AS Float
  ' plan-163-D: a stroke the gradient paints (`FillAndStroke`) is the ramp tinted by the
  ' stroke colour, whose alpha is only the tint's strength -- so a zero-alpha stroke
  ' still strokes, in the plain ramp.
  LET ramped AS Boolean = len(paint.gradient.stops) >= 2 AND paint.gradient.strokeApply = GradientApply.FillAndStroke
  IF toInt(paint.stroke.alpha) <= 0 AND NOT ramped THEN
    RETURN 0.0 - 1.0
  END IF
  IF paint.strokeWidth <= 0.0 THEN
    RETURN 0.0 - 1.0
  END IF
  RETURN paint.strokeWidth / 2.0
END FUNC"#;

/// Read one slot of a geometry record.
#[rustfmt::skip]
const GEO_READ: &str =
r#"FUNC __canvas_geoAt(offset AS Integer, slot AS Integer) AS Float
  RETURN collections::getOr(__CANVAS_GEO_DATA, offset + slot, 0.0)
END FUNC

FUNC __canvas_geoByte(offset AS Integer, slot AS Integer) AS Byte
  RETURN __canvas_clampByte(toInt(__canvas_geoAt(offset, slot)))
END FUNC

' plan-116-B: does this item clip at all?
'
' plan-163-C: the clip's mode, slot `__CANVAS_GEO_CLIP_MODE` -- 0 for the empty ring list,
' which is what an unset `Paint.clip` is. Mode 1 holds an axis-aligned rectangle
' x0,y0,x1,y1 in slots 22-25, in the item's own coordinates.
'
' The per-pixel clip coverage is gated on this, so an unclipped item -- every item in
' every scene written before this letter -- pays one compare per ITEM and nothing per
' pixel.
FUNC __canvas_hasClip(offset AS Integer) AS Boolean
  RETURN __canvas_geoAt(offset, __CANVAS_GEO_CLIP_MODE) <> 0.0
END FUNC

' plan-116-C: does this item carry a transform that is not the identity?
'
' Slot 33 is the flag `__canvas_invertTransform` computed, so "is this the identity"
' is decided once, by the same code that decides what the identity IS -- rather than
' by six compares here that could disagree with it.
'
' plan-172-G: or is it being drawn inside a group whose composed affine is more than a
' translation (`__CANVAS_GM`)? Either way the query point is inverse-mapped, so every
' consumer that asks takes the transformed path.
FUNC __canvas_hasTransform(offset AS Integer) AS Boolean
  IF __CANVAS_GM THEN
    RETURN TRUE
  END IF
  RETURN __canvas_geoAt(offset, 33) > 0.5
END FUNC

' plan-172-G: the linear part of the enclosing groups' composed affine, for the draw
' entry being rasterised -- forward (`F*`) and inverse (`I*`) -- and whether there is
' one. Set by `__canvas_enterGroupMatrix` for one entry at a time and FALSE otherwise,
' so nothing outside the software walk ever sees it set.
'
' Globals rather than six more parameters on `__canvas_drawGeometry` and on every
' helper it calls: the inverse map is taken in five places per pixel and in two clip
' helpers, and each would have had to carry them.
'
' The group's TRANSLATION is not here. It stays the entry's `gdx`/`gdy`, which every
' consumer already subtracts from the surface point, so an item inside a turned group
' reaches its own space as `item⁻¹(L⁻¹(p − t))`: the same subtraction as before, then
' this linear part, then the item's own inverse if it has one.
MUT __CANVAS_GM AS Boolean = FALSE
MUT __CANVAS_GM_FA AS Float = 1.0
MUT __CANVAS_GM_FB AS Float = 0.0
MUT __CANVAS_GM_FC AS Float = 0.0
MUT __CANVAS_GM_FD AS Float = 1.0
MUT __CANVAS_GM_IA AS Float = 1.0
MUT __CANVAS_GM_IB AS Float = 0.0
MUT __CANVAS_GM_IC AS Float = 0.0
MUT __CANVAS_GM_ID AS Float = 1.0
' bug-1083: the Frobenius norm of that inverse, which `__canvas_drawBound` scales an
' item's rim by. Kept beside the matrix so it is one square root an entry and not four
' an item.
MUT __CANVAS_GM_INORM AS Float = 1.0

' Make draw entry matrix `mat` current: a row of `__CANVAS_GROUP_ROWS` plus one, or 0
' for an entry that is only translated.
FUNC __canvas_enterGroupMatrix(mat AS Integer) AS Nothing
  IF mat <= 0 THEN
    __CANVAS_GM = FALSE
    RETURN
  END IF
  LET row AS Integer = (mat - 1) * 8
  __CANVAS_GM_FA = collections::getOr(__CANVAS_GROUP_ROWS, row, 1.0)
  __CANVAS_GM_FB = collections::getOr(__CANVAS_GROUP_ROWS, row + 1, 0.0)
  __CANVAS_GM_FC = collections::getOr(__CANVAS_GROUP_ROWS, row + 2, 0.0)
  __CANVAS_GM_FD = collections::getOr(__CANVAS_GROUP_ROWS, row + 3, 1.0)
  __CANVAS_GM_IA = collections::getOr(__CANVAS_GROUP_ROWS, row + 4, 1.0)
  __CANVAS_GM_IB = collections::getOr(__CANVAS_GROUP_ROWS, row + 5, 0.0)
  __CANVAS_GM_IC = collections::getOr(__CANVAS_GROUP_ROWS, row + 6, 0.0)
  __CANVAS_GM_ID = collections::getOr(__CANVAS_GROUP_ROWS, row + 7, 1.0)
  __CANVAS_GM_INORM = math::sqrt(__CANVAS_GM_IA * __CANVAS_GM_IA + __CANVAS_GM_IB * __CANVAS_GM_IB + __CANVAS_GM_IC * __CANVAS_GM_IC + __CANVAS_GM_ID * __CANVAS_GM_ID)
  __CANVAS_GM = TRUE
END FUNC

' One edge of the surface box a geometry record occupies as the current draw entry
' places it: `k` is 0, 1, 2, 3 for x0, y0, x1, y1.
'
' Translate-only, it is the header's bound plus the offset -- the expression every
' consumer used to write out.
'
' Under a group matrix the box is carried through the groups' linear part and the hull
' of its four corners is padded by a pixel. The box carried is not the header's as it
' stands (bug-1083). The header's box is the item's extent and stroke reach under its
' own transform, and a rim past that (`__CANVAS_GEO_RIM`) sized for the item drawn with
' no group: a unit untransformed, half the item's inverse's Frobenius norm of its own
' units and a pixel under a transform. A group changes the rim a pixel needs. A pixel
' takes coverage only where `d < 0.5 * g`, `d` the distance in the item's space and `g`
' the length of its central differences half a SURFACE pixel apart; a distance is
' 1-Lipschitz, so `g` is at most the norm of the COMPOSED inverse, the item's after the
' groups', and that is at most the product of the two norms. So the rim here is the
' header's rim scaled by the norm of the groups' inverse, `N`:
'
'   lower = (bound + rim) - rim * N        upper = (bound - rim) + rim * N
'
' That covers every pixel that can take coverage, for each kind of rim:
'
' * No transform of its own. The composed inverse IS the groups', so the pixel needs
'   `0.5 * N` of the group's units and gets `1.0 * N`, the builder's rim being a unit
'   (`__CANVAS_BOUNDS_RIM`), which is more than `0.5 * sqrt(2)`: what the bound would
'   ask if the item's identity were counted in the product.
' * Its own transform, inverse norm `F`. The pixel needs `0.5 * F * N` of the ITEM's
'   units around its extent; the item's forward matrix makes that `(|a| + |c|) * 0.5 *
'   F * N` on the x sides of the hull, and the header's rim there is `(|a| + |c|) * 0.5
'   * F`, widened, and a pixel -- so `rim * N` is that and `N` to spare. A transform
'   that shrinks the item only makes `F` large; nothing above assumes it is small.
' * A group that magnifies has `N` under 1 and the box comes in from the header's: the
'   unit that was 170 pixels a side under a scale of 170 is 1.41 pixels. A group that
'   shrinks has `N` over 1 and the box goes out past it, which is what an unevenly
'   transformed item needs there and a finished box could not give.
'
' A box with no rim (a glyph run, a text box, a picture, an arc: `__canvas_boundsHeader`)
' is carried as it stands, both lines above being `bound` then. The Vulkan and Metal
' vertex stages take the same two lines (`mfb_canvas.vert`, `mfbVertex`) and differ
' only in the pixel: theirs is `N` of the group's units before the map, because their
' quad is the mapped parallelogram and not its hull, and a parallelogram has no "one
' pixel out" to add afterwards.
FUNC __canvas_drawBound(offset AS Integer, k AS Integer, gdx AS Float, gdy AS Float) AS Float
  IF NOT __CANVAS_GM THEN
    IF k = 0 OR k = 2 THEN
      RETURN __canvas_geoAt(offset, 16 + k) + gdx
    END IF
    RETURN __canvas_geoAt(offset, 16 + k) + gdy
  END IF
  LET rimX AS Float = __canvas_geoAt(offset, __CANVAS_GEO_RIM)
  LET rimY AS Float = __canvas_geoAt(offset, __CANVAS_GEO_RIM + 1)
  LET x0 AS Float = (__canvas_geoAt(offset, 16) + rimX) - rimX * __CANVAS_GM_INORM
  LET y0 AS Float = (__canvas_geoAt(offset, 17) + rimY) - rimY * __CANVAS_GM_INORM
  LET x1 AS Float = (__canvas_geoAt(offset, 18) - rimX) + rimX * __CANVAS_GM_INORM
  LET y1 AS Float = (__canvas_geoAt(offset, 19) - rimY) + rimY * __CANVAS_GM_INORM
  IF k = 0 OR k = 2 THEN
    LET a AS Float = __CANVAS_GM_FA * x0 + __CANVAS_GM_FC * y0
    LET b AS Float = __CANVAS_GM_FA * x1 + __CANVAS_GM_FC * y0
    LET c AS Float = __CANVAS_GM_FA * x0 + __CANVAS_GM_FC * y1
    LET d AS Float = __CANVAS_GM_FA * x1 + __CANVAS_GM_FC * y1
    IF k = 0 THEN
      RETURN __canvas_minF(__canvas_minF(a, b), __canvas_minF(c, d)) + gdx - 1.0
    END IF
    RETURN __canvas_maxF(__canvas_maxF(a, b), __canvas_maxF(c, d)) + gdx + 1.0
  END IF
  LET ya AS Float = __CANVAS_GM_FB * x0 + __CANVAS_GM_FD * y0
  LET yb AS Float = __CANVAS_GM_FB * x1 + __CANVAS_GM_FD * y0
  LET yc AS Float = __CANVAS_GM_FB * x0 + __CANVAS_GM_FD * y1
  LET yd AS Float = __CANVAS_GM_FB * x1 + __CANVAS_GM_FD * y1
  IF k = 1 THEN
    RETURN __canvas_minF(__canvas_minF(ya, yb), __canvas_minF(yc, yd)) + gdy - 1.0
  END IF
  RETURN __canvas_maxF(__canvas_maxF(ya, yb), __canvas_maxF(yc, yd)) + gdy + 1.0
END FUNC

' The forward affine an item is drawn through, as six floats: its own (the inverse of
' the header's inverse), under the groups' linear part when there is one. The group's
' translation is the entry's `gdx`/`gdy` and is not in it. A glyph run walks the surface
' box this maps each glyph to.
FUNC __canvas_drawForward(offset AS Integer) AS List OF Float
  MUT f AS List OF Float = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]
  IF __canvas_geoAt(offset, 33) > 0.5 THEN
    f = __canvas_forwardOf(__canvas_f32FromBits(__canvas_geoAt(offset, 27)), __canvas_f32FromBits(__canvas_geoAt(offset, 28)), __canvas_f32FromBits(__canvas_geoAt(offset, 29)), __canvas_f32FromBits(__canvas_geoAt(offset, 30)), __canvas_f32FromBits(__canvas_geoAt(offset, 31)), __canvas_f32FromBits(__canvas_geoAt(offset, 32)))
  END IF
  IF NOT __CANVAS_GM THEN
    RETURN f
  END IF
  LET a AS Float = collections::getOr(f, 0, 1.0)
  LET b AS Float = collections::getOr(f, 1, 0.0)
  LET c AS Float = collections::getOr(f, 2, 0.0)
  LET d AS Float = collections::getOr(f, 3, 1.0)
  LET tx AS Float = collections::getOr(f, 4, 0.0)
  LET ty AS Float = collections::getOr(f, 5, 0.0)
  RETURN [__CANVAS_GM_FA * a + __CANVAS_GM_FC * b, __CANVAS_GM_FB * a + __CANVAS_GM_FD * b, __CANVAS_GM_FA * c + __CANVAS_GM_FC * d, __CANVAS_GM_FB * c + __CANVAS_GM_FD * d, __CANVAS_GM_FA * tx + __CANVAS_GM_FC * ty, __CANVAS_GM_FB * tx + __CANVAS_GM_FD * ty]
END FUNC

' The inverse-mapped x (or y) of a surface point.
'
' The header carries the six terms as float32 BIT PATTERNS, so they are decoded back
' to a Float here -- see `__canvas_float32Bits` for why the narrowing happens at all.
' Decoding is arithmetic for the same reason encoding was: `bits::` never takes a
' Float, so there is no reinterpret to borrow.
FUNC __canvas_f32FromBits(raw AS Float) AS Float
  LET bits AS Integer = toInt(raw)
  IF bits = 0 OR bits = 2147483648 THEN
    RETURN 0.0
  END IF
  MUT sign AS Float = 1.0
  MUT rest AS Integer = bits
  IF rest >= 2147483648 THEN
    sign = 0.0 - 1.0
    rest = rest - 2147483648
  END IF
  LET e AS Integer = rest / 8388608
  LET mantissa AS Integer = rest - e * 8388608
  MUT value AS Float = 1.0 + toFloat(mantissa) / 8388608.0
  MUT k AS Integer = e - 127
  WHILE k > 0
    value = value * 2.0
    k = k - 1
  END WHILE
  WHILE k < 0
    value = value / 2.0
    k = k + 1
  END WHILE
  RETURN sign * value
END FUNC

'
' plan-172-G: inside a group with a matrix the point goes through the groups' inverse
' linear part first, then through the item's own inverse if it has one. With no group
' matrix the expression is the one it always was.
FUNC __canvas_invX(offset AS Integer, px AS Float, py AS Float) AS Float
  IF __CANVAS_GM THEN
    LET gx AS Float = __CANVAS_GM_IA * px + __CANVAS_GM_IC * py
    IF __canvas_geoAt(offset, 33) > 0.5 THEN
      LET gy AS Float = __CANVAS_GM_IB * px + __CANVAS_GM_ID * py
      RETURN __canvas_f32FromBits(__canvas_geoAt(offset, 27)) * gx + __canvas_f32FromBits(__canvas_geoAt(offset, 29)) * gy + __canvas_f32FromBits(__canvas_geoAt(offset, 31))
    END IF
    RETURN gx
  END IF
  RETURN __canvas_f32FromBits(__canvas_geoAt(offset, 27)) * px + __canvas_f32FromBits(__canvas_geoAt(offset, 29)) * py + __canvas_f32FromBits(__canvas_geoAt(offset, 31))
END FUNC

FUNC __canvas_invY(offset AS Integer, px AS Float, py AS Float) AS Float
  IF __CANVAS_GM THEN
    LET gy AS Float = __CANVAS_GM_IB * px + __CANVAS_GM_ID * py
    IF __canvas_geoAt(offset, 33) > 0.5 THEN
      LET gx AS Float = __CANVAS_GM_IA * px + __CANVAS_GM_IC * py
      RETURN __canvas_f32FromBits(__canvas_geoAt(offset, 28)) * gx + __canvas_f32FromBits(__canvas_geoAt(offset, 30)) * gy + __canvas_f32FromBits(__canvas_geoAt(offset, 32))
    END IF
    RETURN gy
  END IF
  RETURN __canvas_f32FromBits(__canvas_geoAt(offset, 28)) * px + __canvas_f32FromBits(__canvas_geoAt(offset, 30)) * py + __canvas_f32FromBits(__canvas_geoAt(offset, 32))
END FUNC

' bug-995: how far a transformed clip's CENTRE distance must be from its edge for the
' pixel's clip coverage to be decided by that one sample: `0.5 * max(F, 1)`, `F` the
' Frobenius norm of the composed inverse (the item's own, after the groups'), taken as
' the product of the two norms, which bounds it, and widened by 2^-8 as both shaders
' widen theirs (`clipReach` in `shapeCoverage`).
'
' Why one sample is enough there: the surface distance is `d / g`, `g` the length of the
' central differences of the clip's distance at the four inverse-mapped neighbours (or
' `d` itself when `g` is under 1e-6). A clip's distance is 1-Lipschitz in its own space
' and the neighbours are the inverse's columns halved either side of the centre, so
' `g <= F`. Past the reach `|d / g| > 0.5 * max(F, 1) / g >= 0.5`, and `|d| > 0.5`:
' `__canvas_coverage` answers 0 or 255 by the sign for the quotient and for `d` alike.
FUNC __canvas_clipReach(offset AS Integer) AS Float
  MUT norm AS Float = 1.0
  IF __canvas_geoAt(offset, 33) > 0.5 THEN
    LET ia AS Float = __canvas_f32FromBits(__canvas_geoAt(offset, 27))
    LET ib AS Float = __canvas_f32FromBits(__canvas_geoAt(offset, 28))
    LET ic AS Float = __canvas_f32FromBits(__canvas_geoAt(offset, 29))
    LET id AS Float = __canvas_f32FromBits(__canvas_geoAt(offset, 30))
    norm = math::sqrt(ia * ia + ib * ib + ic * ic + id * id)
  END IF
  IF __CANVAS_GM THEN
    norm = norm * math::sqrt(__CANVAS_GM_IA * __CANVAS_GM_IA + __CANVAS_GM_IB * __CANVAS_GM_IB + __CANVAS_GM_IC * __CANVAS_GM_IC + __CANVAS_GM_ID * __CANVAS_GM_ID)
  END IF
  LET frob AS Float = norm * (1.0 + 1.0 / 256.0) + 1.0 / 256.0
  IF frob > 1.0 THEN
    RETURN 0.5 * frob
  END IF
  RETURN 0.5
END FUNC

' The clip's own antialiased coverage at a pixel centre, 0..255.
'
' The same `clamp(0.5 - d, 0, 1)` form every shape edge uses, with `d` the signed
' distance to the clip rectangle -- so a fractional clip edge is antialiased exactly
' like a shape edge, which is what keeps the oracle and both GPU backends agreeing.
'
' Evaluating the rectangle SDF at the pixel CENTRE gives the covered fraction directly
' for an axis-aligned edge: a clip starting at x = 10.6 leaves pixel 10 (centre 10.5)
' at d = +0.1, hence coverage 0.4 -- which is the 40% of that pixel actually inside.
'
' plan-163-C: a path clip's SURFACE distance at the pixel (`px`, `py`, the group offset
' taken off; `qx`, `qy` its shape-space point): under a transform, its shape-space
' distance divided by the clip's own gradient there, by central differences at the same
' inverse-mapped neighbours the shape uses -- not by the SHAPE's gradient, which near the
' clip's edge belongs to whichever shape edge is nearest, so a sheared clip would
' antialias by the wrong slope (measured, plan-163-C Corrections 4).
'
' bug-995: a centre distance past `__canvas_clipReach` is returned as it is and the four
' neighbours are not evaluated -- both shaders' `clipReach` shortcut. The coverage is
' the same: see `__canvas_clipReach`.
FUNC __canvas_clipPathSurface(offset AS Integer, px AS Float, py AS Float, qx AS Float, qy AS Float, transformed AS Boolean) AS Float
  LET d AS Float = __canvas_clipPathDistance(offset, qx, qy)
  IF NOT transformed THEN
    RETURN d
  END IF
  LET reach AS Float = __canvas_clipReach(offset)
  IF d > reach OR d < 0.0 - reach THEN
    RETURN d
  END IF
  LET dxp AS Float = __canvas_clipPathDistance(offset, __canvas_invX(offset, px + 0.5, py), __canvas_invY(offset, px + 0.5, py))
  LET dxm AS Float = __canvas_clipPathDistance(offset, __canvas_invX(offset, px - 0.5, py), __canvas_invY(offset, px - 0.5, py))
  LET dyp AS Float = __canvas_clipPathDistance(offset, __canvas_invX(offset, px, py + 0.5), __canvas_invY(offset, px, py + 0.5))
  LET dym AS Float = __canvas_clipPathDistance(offset, __canvas_invX(offset, px, py - 0.5), __canvas_invY(offset, px, py - 0.5))
  LET gx AS Float = dxp - dxm
  LET gy AS Float = dyp - dym
  LET g AS Float = math::sqrt(gx * gx + gy * gy)
  IF g > 0.000001 THEN
    RETURN d / g
  END IF
  RETURN d
END FUNC

' plan-163-C: a glyph's clip coverage -- a path clip at the glyph's shape-space point
' (`qx`, `qy`) divided by the clip's own gradient (`__canvas_clipPathSurface`), as a
' shape's is; a rectangle at its group-relative point (`px`, `py`).
FUNC __canvas_glyphClip(offset AS Integer, qx AS Float, qy AS Float, px AS Float, py AS Float) AS Integer
  IF __canvas_geoAt(offset, __CANVAS_GEO_CLIP_MODE) = 2.0 THEN
    RETURN __canvas_coverage(__canvas_clipPathSurface(offset, px, py, qx, qy, __canvas_hasTransform(offset)))
  END IF
  IF __CANVAS_GM THEN
    RETURN __canvas_coverage(__canvas_clipRectSurface(offset, px, py, qx, qy))
  END IF
  RETURN __canvas_clipCoverage(offset, px, py)
END FUNC

' plan-172-G: a RECTANGLE clip under a group matrix. The rectangle is in the item's own
' space and the group has turned that space, so it is no longer axis-aligned on the
' surface: its distance is taken at the shape-space point (`qx`, `qy`) and divided by
' its own gradient there, by central differences at the inverse-mapped neighbours --
' `__canvas_clipPathSurface`'s arithmetic with the rectangle's distance in place of the
' path's. An item's own transform never reaches here: a transformed item's clip is
' built as a path. A rectangle with no area encloses nothing.
FUNC __canvas_clipRectDistance(offset AS Integer, x AS Float, y AS Float) AS Float
  LET x0 AS Float = __canvas_geoAt(offset, 22)
  LET y0 AS Float = __canvas_geoAt(offset, 23)
  LET x1 AS Float = __canvas_geoAt(offset, 24)
  LET y1 AS Float = __canvas_geoAt(offset, 25)
  IF NOT (x0 < x1 AND y0 < y1) THEN
    RETURN 1000000.0
  END IF
  RETURN __canvas_rectDistance(x, y, (x0 + x1) / 2.0, (y0 + y1) / 2.0, (x1 - x0) / 2.0, (y1 - y0) / 2.0)
END FUNC

FUNC __canvas_clipRectSurface(offset AS Integer, px AS Float, py AS Float, qx AS Float, qy AS Float) AS Float
  LET d AS Float = __canvas_clipRectDistance(offset, qx, qy)
  LET reach AS Float = __canvas_clipReach(offset)
  IF d > reach OR d < 0.0 - reach THEN
    RETURN d
  END IF
  LET dxp AS Float = __canvas_clipRectDistance(offset, __canvas_invX(offset, px + 0.5, py), __canvas_invY(offset, px + 0.5, py))
  LET dxm AS Float = __canvas_clipRectDistance(offset, __canvas_invX(offset, px - 0.5, py), __canvas_invY(offset, px - 0.5, py))
  LET dyp AS Float = __canvas_clipRectDistance(offset, __canvas_invX(offset, px, py + 0.5), __canvas_invY(offset, px, py + 0.5))
  LET dym AS Float = __canvas_clipRectDistance(offset, __canvas_invX(offset, px, py - 0.5), __canvas_invY(offset, px, py - 0.5))
  LET gx AS Float = dxp - dxm
  LET gy AS Float = dyp - dym
  LET g AS Float = math::sqrt(gx * gx + gy * gy)
  IF g > 0.000001 THEN
    RETURN d / g
  END IF
  RETURN d
END FUNC

' plan-163-C: a path clip's signed distance at the item's own point (`x`, `y`, the
' shape-space point): its edges under its rule, negative inside.
FUNC __canvas_clipPathDistance(offset AS Integer, x AS Float, y AS Float) AS Float
  LET edges AS Integer = toInt(__canvas_geoAt(offset, __CANVAS_GEO_CLIP_EDGE_COUNT))
  RETURN __canvas_edgeDistanceRule(__canvas_clipEdgeBase(offset), edges, x, y, __canvas_geoAt(offset, __CANVAS_GEO_CLIP_RULE) <> 0.0)
END FUNC

' plan-163-D: a radial ramp's position at the gradient-space point (`px`, `py`) with a
' focal circle: centre (`cx`, `cy`) and radius `r` the end circle's, the focus at the
' clamped offset (`ox`, `oy`) from the centre with radius `fr`. `t` is the largest root of
' `|p - f - t (c - f)|^2 = (fr + t (r - fr))^2`, the circle through `p` of the family that
' runs from the focal circle at t = 0 to the end circle at t = 1. The builder's clamps
' keep the leading coefficient `a = |c - f|^2 - (r - fr)^2` below zero, so the larger root
' is `(b - sqrt(b^2 - a c)) / a`, and it always has `fr + t (r - fr) >= 0`. A zero
' radius leaves `t` at 0, as a radial ramp's does.
FUNC __canvas_focalT(px AS Float, py AS Float, cx AS Float, cy AS Float, r AS Float, ox AS Float, oy AS Float, fr AS Float) AS Float
  IF NOT (r > 0.0) THEN
    RETURN 0.0
  END IF
  LET fx AS Float = cx + ox
  LET fy AS Float = cy + oy
  LET ex AS Float = px - fx
  LET ey AS Float = py - fy
  LET wx AS Float = cx - fx
  LET wy AS Float = cy - fy
  LET dr AS Float = r - fr
  LET a AS Float = wx * wx + wy * wy - dr * dr
  LET b AS Float = ex * wx + ey * wy + fr * dr
  LET c AS Float = ex * ex + ey * ey - fr * fr
  LET disc AS Float = __canvas_maxF(b * b - a * c, 0.0)
  RETURN (b - math::sqrt(disc)) / a
END FUNC

' plan-163-C: a picture's texel channel tinted by its fill: towards `texel * fill / 255`
' by `alpha / 255`, the difference rounded half away from zero -- so an alpha of 255 is
' that product exactly (today's multiply) and 0 the texel unchanged.
FUNC __canvas_tintChannel(texel AS Integer, fill AS Integer, alpha AS Integer) AS Byte
  LET tint AS Integer = (texel * fill) / 255
  LET d AS Integer = (tint - texel) * alpha
  IF d >= 0 THEN
    RETURN toByte(texel + (d + 127) / 255)
  END IF
  RETURN toByte(texel - (127 - d) / 255)
END FUNC

' plan-163-C: at the item's own point (the shape-space point, the group offset already
' taken off), because the clip is in the item's coordinates. A rectangle with no area
' encloses nothing and hides the item -- measured as a distance it would leave a
' half-covered line along a pixel centre on it.
FUNC __canvas_clipCoverage(offset AS Integer, px AS Float, py AS Float) AS Integer
  LET x0 AS Float = __canvas_geoAt(offset, 22)
  LET y0 AS Float = __canvas_geoAt(offset, 23)
  LET x1 AS Float = __canvas_geoAt(offset, 24)
  LET y1 AS Float = __canvas_geoAt(offset, 25)
  IF NOT (x0 < x1 AND y0 < y1) THEN
    RETURN 0
  END IF
  RETURN __canvas_coverage(__canvas_rectDistance(px, py, (x0 + x1) / 2.0, (y0 + y1) / 2.0, (x1 - x0) / 2.0, (y1 - y0) / 2.0))
END FUNC"#;

/// Rasterise one geometry record onto the surface.
///
/// The bounds come from the header rather than being recomputed, which is the point of
/// generating them once: a repaint of an unchanged scene re-reads them instead of
/// re-deriving them from the item's fields.
///
/// The blend block appears twice — once for the fill, once for the stroke — because it
/// cannot be a helper without reintroducing the whole-surface copy this function's
/// module comment measures. The duplication is the cheaper of the two costs, and it is
/// bounded: two copies here, not one per primitive.
#[rustfmt::skip]
const DRAW_GEOMETRY: &str =
r#"' plan-116-G: `gdx`/`gdy` are the accumulated translation of the groups this item is
' drawn inside, zero for an item in the scene itself.
'
' Applied in exactly two ways, and section 4.5's whole rule is the difference between
' them. The item's BOUNDS move by +(gdx, gdy), because that is where it lands on the
' surface. The QUERY POINT moves by -(gdx, gdy), because a distance field is translated
' by evaluating it at the inverse-translated point -- the same mechanism plan-116-C
' established for the item's `transform`, specialised to a translation, so no distance
' function changes.
'
' `px`/`py` below are therefore the SHAPE-space point and every distance call keeps
' using them unchanged. `spx`/`spy` are the surface point, and only two things want it:
' the clip, which is a surface rectangle by definition, and the surface write itself.
FUNC __canvas_drawGeometry(surface AS List OF Byte, width AS Integer, height AS Integer, offset AS Integer, gdx AS Float, gdy AS Float) AS List OF Byte
  MUT out AS List OF Byte = surface
  LET kind AS Integer = toInt(__canvas_geoAt(offset, 0))
  IF kind = __CANVAS_GEO_NONE THEN
    RETURN out
  END IF
  ' A glyph run is blitted from the coverage cache rather than evaluated per pixel:
  ' the same arithmetic, done once per (font, size, glyph) instead of once per pixel
  ' per edge (plan-98-G Correction 12). Written out here rather than called as a helper
  ' for the reason at the top of this file -- the surface cannot cross a function
  ' boundary.
  '
  ' Everything this needs is already in the cache: the run carries entry indices, not
  ' glyph ids, so drawing touches no font and allocates nothing (Correction 13).
  IF kind = __CANVAS_GEO_TEXT THEN
    LET tGlyphs AS Integer = toInt(__canvas_geoAt(offset, 20))
    LET tR AS Byte = __canvas_geoByte(offset, 8)
    LET tG AS Byte = __canvas_geoByte(offset, 9)
    LET tB AS Byte = __canvas_geoByte(offset, 10)
    LET tA AS Integer = toInt(__canvas_geoAt(offset, 11))
    IF tA <= 0 OR tGlyphs <= 0 THEN
      RETURN out
    END IF
    LET runAt AS Integer = offset + __CANVAS_GEO_HEADER
    ' plan-116-B: a glyph run clips like any other item. The blit's bounds test is
    ' already per-pixel (`sx`/`sy` against the surface), so the clip folds into the same
    ' guard rather than into loop bounds -- and the boundary pixels take the same
    ' coverage multiply the SDF path takes, so a clipped glyph edge is antialiased
    ' identically to a clipped shape edge.
    LET tClipped AS Boolean = __canvas_hasClip(offset)
    LET tTransformed AS Boolean = __canvas_hasTransform(offset)
    LET tBlend AS Integer = toInt(__canvas_geoAt(offset, 26))
    MUT gi AS Integer = 0
    WHILE gi < tGlyphs
      LET runBase AS Integer = runAt + gi * 3
      LET meta AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, runBase, 0.0)) * 5
      ' plan-116-G / G5. A glyph is a cached coverage bitmap indexed by whole pixels
      ' from this origin, not a distance field, so the "evaluate at p - offset" rule
      ' that moves every shape does not reach it: text in a translated group would
      ' sample the right texels and write them to the wrong place.
      '
      ' `gx`/`gy` stay the glyph's own origin, deliberately. Adding the group offset
      ' here would be right for the untransformed blit and WRONG for the transformed
      ' one, which forward-maps this box through the item's `transform` -- the translation
      ' would be rotated and scaled with the glyph. The composition is
      ' `surface = groupOffset + transform(shape)`, so each path applies the offset on
      ' its own side of the map: see `gsx`/`gsy` below and the AABB.
      LET gx AS Integer = collections::getOr(__CANVAS_GLYPH_META, meta, 0) + toInt(collections::getOr(__CANVAS_GEO_DATA, runBase + 1, 0.0))
      LET gy AS Integer = collections::getOr(__CANVAS_GLYPH_META, meta + 1, 0) + toInt(collections::getOr(__CANVAS_GEO_DATA, runBase + 2, 0.0))
      LET gsx AS Integer = gx + toInt(gdx)
      LET gsy AS Integer = gy + toInt(gdy)
      LET gw AS Integer = collections::getOr(__CANVAS_GLYPH_META, meta + 2, 0)
      LET gh AS Integer = collections::getOr(__CANVAS_GLYPH_META, meta + 3, 0)
      LET gStart AS Integer = collections::getOr(__CANVAS_GLYPH_META, meta + 4, 0)
      ' plan-116-C §4.5: a transformed glyph run inverts the loop. Untransformed, the
      ' blit walks the BITMAP and writes each sample to its surface pixel. That cannot
      ' work under a rotation -- the mapping is no longer one bitmap sample per surface
      ' pixel, so walking the bitmap leaves holes. Instead walk the surface region the
      ' glyph now covers and inverse-map each pixel back to a sample.
      '
      ' NEAREST sampling, deliberately: integer index arithmetic and the same `+ - * /`
      ' the oracle already allows, so all three renderers agree exactly and the glyph
      ' cache stays untransformed -- one cache entry serves every transform. The cost is
      ' that a rotated glyph edge stair-steps rather than being resampled; the man page
      ' says so plainly. Bilinear filtering would change every transformed glyph's edge
      ' bytes between renderers unless all three implemented identical fixed-point
      ' weights, which is a letter of its own.
      IF tTransformed THEN
        ' plan-172-G: the item's own forward map, under the groups' linear part when the
        ' entry has one (`__canvas_drawForward`).
        LET fwd AS List OF Float = __canvas_drawForward(offset)
        LET fa AS Float = collections::getOr(fwd, 0, 1.0)
        LET fb AS Float = collections::getOr(fwd, 1, 0.0)
        LET fc AS Float = collections::getOr(fwd, 2, 0.0)
        LET fd AS Float = collections::getOr(fwd, 3, 1.0)
        LET ftx AS Float = collections::getOr(fwd, 4, 0.0)
        LET fty AS Float = collections::getOr(fwd, 5, 0.0)
        ' The AABB of this ONE glyph's forward-mapped box, not the whole run's: a run
        ' spans many glyphs, and scanning every glyph over the run's hull would be
        ' quadratic in the string length.
        LET bx0 AS Float = toFloat(gx)
        LET by0 AS Float = toFloat(gy)
        LET bx1 AS Float = toFloat(gx + gw)
        LET by1 AS Float = toFloat(gy + gh)
        ' The group offset is added AFTER the forward map, never through it: it is a
        ' surface translation, and passing it through the item's `transform` would rotate
        ' and scale it with the glyph.
        LET cx0 AS Float = fa * bx0 + fc * by0 + ftx + gdx
        LET cy0 AS Float = fb * bx0 + fd * by0 + fty + gdy
        LET cx1 AS Float = fa * bx1 + fc * by0 + ftx + gdx
        LET cy1 AS Float = fb * bx1 + fd * by0 + fty + gdy
        LET cx2 AS Float = fa * bx0 + fc * by1 + ftx + gdx
        LET cy2 AS Float = fb * bx0 + fd * by1 + fty + gdy
        LET cx3 AS Float = fa * bx1 + fc * by1 + ftx + gdx
        LET cy3 AS Float = fb * bx1 + fd * by1 + fty + gdy
        LET loX AS Integer = __canvas_maxI(toInt(__canvas_minF(__canvas_minF(cx0, cx1), __canvas_minF(cx2, cx3))) - 1, 0)
        LET loY AS Integer = __canvas_maxI(toInt(__canvas_minF(__canvas_minF(cy0, cy1), __canvas_minF(cy2, cy3))) - 1, 0)
        LET hiX AS Integer = __canvas_minI(toInt(__canvas_maxF(__canvas_maxF(cx0, cx1), __canvas_maxF(cx2, cx3))) + 1, width - 1)
        LET hiY AS Integer = __canvas_minI(toInt(__canvas_maxF(__canvas_maxF(cy0, cy1), __canvas_maxF(cy2, cy3))) + 1, height - 1)
        MUT ty AS Integer = loY
        WHILE ty <= hiY
          LET tRowBase AS Integer = ty * width * 4
          MUT tx AS Integer = loX
          WHILE tx <= hiX
            ' The SURFACE point, for the clip; and the shape-space point the inverse
            ' map wants, which is the same rule `px`/`spx` follow for a shape.
            LET sfpx AS Float = toFloat(tx) + 0.5
            LET sfpy AS Float = toFloat(ty) + 0.5
            LET fpx AS Float = sfpx - gdx
            LET fpy AS Float = sfpy - gdy
            ' FLOOR, not truncation, and floor of the mapped coordinate BEFORE the
            ' whole-pixel glyph origin is subtracted -- the same two decisions both
            ' shaders make.
            '
            ' Truncation is wrong rather than merely different here: the texel holding a
            ' point `u` is `floor(u)`, and `toInt` rounds toward zero, so every glyph
            ' above the baseline -- which is all of them, ink runs up from the pen --
            ' has negative shape-space coordinates and picks the texel on the wrong
            ' side. And the order matters independently: mapped 4.3 with origin 12 gives
            ' -8 if you floor first and -7 if you subtract first.
            LET ux AS Integer = math::floor(__canvas_invX(offset, fpx, fpy)) - gx
            LET uy AS Integer = math::floor(__canvas_invY(offset, fpx, fpy)) - gy
            IF ux >= 0 AND uy >= 0 AND ux < gw AND uy < gh THEN
              MUT tcov AS Integer = toInt(collections::getOr(__CANVAS_GLYPH_COV, gStart + uy * gw + ux, toByte(0)))
              IF tClipped THEN
                tcov = (tcov * __canvas_glyphClip(offset, __canvas_invX(offset, fpx, fpy), __canvas_invY(offset, fpx, fpy), fpx, fpy)) / 255
              END IF
              IF tcov > 0 THEN
                LET tAlpha AS Integer = (tA * tcov) / 255
                LET tIdx AS Integer = tRowBase + tx * 4
                IF tAlpha >= 255 AND tBlend = 0 THEN
                  out = collections::set(out, tIdx, tR)
                  out = collections::set(out, tIdx + 1, tG)
                  out = collections::set(out, tIdx + 2, tB)
                  out = collections::set(out, tIdx + 3, toByte(255))
                ELSEIF tAlpha > 0 THEN
                  out = collections::set(out, tIdx, __canvas_blendChannelMode(collections::getOr(out, tIdx, toByte(0)), tR, tAlpha, tBlend))
                  out = collections::set(out, tIdx + 1, __canvas_blendChannelMode(collections::getOr(out, tIdx + 1, toByte(0)), tG, tAlpha, tBlend))
                  out = collections::set(out, tIdx + 2, __canvas_blendChannelMode(collections::getOr(out, tIdx + 2, toByte(0)), tB, tAlpha, tBlend))
                  out = collections::set(out, tIdx + 3, toByte(255))
                END IF
              END IF
            END IF
            tx = tx + 1
          END WHILE
          ty = ty + 1
        END WHILE
      ELSE
      MUT gRow AS Integer = 0
      WHILE gRow < gh
        LET sy AS Integer = gsy + gRow
        IF sy >= 0 AND sy < height THEN
          LET gRowBase AS Integer = sy * width * 4
          MUT gCol AS Integer = 0
          WHILE gCol < gw
            LET sx AS Integer = gsx + gCol
            IF sx >= 0 AND sx < width THEN
              MUT cover AS Integer = toInt(collections::getOr(__CANVAS_GLYPH_COV, gStart + gRow * gw + gCol, toByte(0)))
              IF tClipped THEN
                cover = (cover * __canvas_glyphClip(offset, toFloat(sx) + 0.5 - gdx, toFloat(sy) + 0.5 - gdy, toFloat(sx) + 0.5 - gdx, toFloat(sy) + 0.5 - gdy)) / 255
              END IF
              IF cover > 0 THEN
                LET gAlpha AS Integer = (tA * cover) / 255
                LET gIdx AS Integer = gRowBase + sx * 4
                IF gAlpha >= 255 AND tBlend = 0 THEN
                  out = collections::set(out, gIdx, tR)
                  out = collections::set(out, gIdx + 1, tG)
                  out = collections::set(out, gIdx + 2, tB)
                  out = collections::set(out, gIdx + 3, toByte(255))
                ELSEIF gAlpha > 0 THEN
                  out = collections::set(out, gIdx, __canvas_blendChannelMode(collections::getOr(out, gIdx, toByte(0)), tR, gAlpha, tBlend))
                  out = collections::set(out, gIdx + 1, __canvas_blendChannelMode(collections::getOr(out, gIdx + 1, toByte(0)), tG, gAlpha, tBlend))
                  out = collections::set(out, gIdx + 2, __canvas_blendChannelMode(collections::getOr(out, gIdx + 2, toByte(0)), tB, gAlpha, tBlend))
                  out = collections::set(out, gIdx + 3, toByte(255))
                END IF
              END IF
            END IF
            gCol = gCol + 1
          END WHILE
        END IF
        gRow = gRow + 1
      END WHILE
      END IF
      gi = gi + 1
    END WHILE
    RETURN out
  END IF

  LET p0 AS Float = __canvas_geoAt(offset, 2)
  LET p1 AS Float = __canvas_geoAt(offset, 3)
  LET p2 AS Float = __canvas_geoAt(offset, 4)
  LET p3 AS Float = __canvas_geoAt(offset, 5)
  LET radius AS Float = __canvas_geoAt(offset, 6)
  LET half AS Float = __canvas_geoAt(offset, 7)
  ' plan-116-F: the gradient's scalars, read ONCE per item -- the has-gradient test,
  ' the stop base and the axis. Hoisted for the reason plan-116-B hoisted the blend
  ' mode: an item without a gradient is every item written before this letter, and it
  ' must not pay a per-pixel branch on a field it does not use.
  LET gradCount AS Integer = toInt(__canvas_geoAt(offset, __CANVAS_GEO_GRADIENT_COUNT))
  LET gradBase AS Integer = __canvas_gradientStopBase(offset)
  LET gradRadial AS Boolean = toInt(__canvas_geoAt(offset, __CANVAS_GEO_GRADIENT_KIND)) = 1
  LET gradFX AS Float = __canvas_geoAt(offset, __CANVAS_GEO_GRADIENT_FROMX)
  LET gradFY AS Float = __canvas_geoAt(offset, __CANVAS_GEO_GRADIENT_FROMY)
  LET gradTX AS Float = __canvas_geoAt(offset, __CANVAS_GEO_GRADIENT_TOX)
  LET gradTY AS Float = __canvas_geoAt(offset, __CANVAS_GEO_GRADIENT_TOY)
  LET gradAX AS Float = gradTX - gradFX
  LET gradAY AS Float = gradTY - gradFY
  ' The axis length squared for linear, its root for radial. Zero-length is defined
  ' rather than a divide by zero: `t` stays 0, so the first stop fills.
  LET gradLen2 AS Float = gradAX * gradAX + gradAY * gradAY
  LET gradLen AS Float = math::sqrt(gradLen2)
  LET gradSpread AS Integer = toInt(__canvas_geoAt(offset, __CANVAS_GEO_GRADIENT_SPREAD))
  ' plan-163-D: a radial ramp's inverse transform (float32 bit patterns, decoded once) and
  ' its focal circle, and whether the ramp paints the stroke -- for a Line or Arc, whose
  ' stroke is drawn as a fill, the FILL tinted by the stroke colour in the fill slots.
  LET gradHasInv AS Boolean = __canvas_geoAt(offset, __CANVAS_GEO_GRADIENT_HAS_INV) > 0.5
  LET gradIA AS Float = __canvas_f32FromBits(__canvas_geoAt(offset, __CANVAS_GEO_GRADIENT_INV))
  LET gradIB AS Float = __canvas_f32FromBits(__canvas_geoAt(offset, __CANVAS_GEO_GRADIENT_INV + 1))
  LET gradIC AS Float = __canvas_f32FromBits(__canvas_geoAt(offset, __CANVAS_GEO_GRADIENT_INV + 2))
  LET gradID AS Float = __canvas_f32FromBits(__canvas_geoAt(offset, __CANVAS_GEO_GRADIENT_INV + 3))
  LET gradITX AS Float = __canvas_f32FromBits(__canvas_geoAt(offset, __CANVAS_GEO_GRADIENT_INV + 4))
  LET gradITY AS Float = __canvas_f32FromBits(__canvas_geoAt(offset, __CANVAS_GEO_GRADIENT_INV + 5))
  LET gradOX AS Float = __canvas_geoAt(offset, __CANVAS_GEO_GRADIENT_FOCALX)
  LET gradOY AS Float = __canvas_geoAt(offset, __CANVAS_GEO_GRADIENT_FOCALY)
  LET gradFR AS Float = __canvas_geoAt(offset, __CANVAS_GEO_GRADIENT_FOCALR)
  LET gradFocal AS Boolean = gradOX <> 0.0 OR gradOY <> 0.0 OR gradFR <> 0.0
  LET gradApply AS Boolean = gradCount >= 2 AND __canvas_geoAt(offset, __CANVAS_GEO_GRADIENT_APPLY) > 0.5
  LET gradTintFill AS Boolean = gradApply AND (kind = __CANVAS_KIND_SEGMENT OR kind = __CANVAS_GEO_ARC OR kind = __CANVAS_GEO_POLYCURVE)
  LET fillR AS Byte = __canvas_geoByte(offset, 8)
  LET fillG AS Byte = __canvas_geoByte(offset, 9)
  LET fillB AS Byte = __canvas_geoByte(offset, 10)
  LET fillA AS Integer = toInt(__canvas_geoAt(offset, 11))
  LET strokeR AS Byte = __canvas_geoByte(offset, 12)
  LET strokeG AS Byte = __canvas_geoByte(offset, 13)
  LET strokeB AS Byte = __canvas_geoByte(offset, 14)
  LET strokeA AS Integer = toInt(__canvas_geoAt(offset, 15))
  LET edges AS Integer = toInt(__canvas_geoAt(offset, 20))
  LET tail AS Integer = offset + __CANVAS_GEO_HEADER
  ' plan-163-B: a Miter or Bevel stroke's join records (`helper_joins.rs`); zero for
  ' every Round stroke, which keeps the band `|d| - half` it always had.
  LET joinCount AS Integer = toInt(__canvas_geoAt(offset, __CANVAS_GEO_JOIN_COUNT))
  ' plan-163-B: a dashed stroke (`helper_dashes.rs`); false for every undashed item.
  LET dashed AS Boolean = toInt(__canvas_geoAt(offset, __CANVAS_GEO_DASH_COUNT)) > 0
  ' plan-116-D. Read once per item, not per pixel: it is a per-shape constant like the
  ' arc's sweep vectors below. Only Line and Arc ever write it; every other kind leaves
  ' it at the blank header's zero and never reaches a cap branch.
  LET cap AS Integer = toInt(__canvas_geoAt(offset, __CANVAS_GEO_CAP))
  ' plan-116-D: an arc's two sweep endpoints, read once per item like the cap. Zero for
  ' every other kind, and never read by one -- only the arc arm touches them.
  LET capSX AS Float = __canvas_geoAt(offset, __CANVAS_GEO_CAPSTARTX)
  LET capSY AS Float = __canvas_geoAt(offset, __CANVAS_GEO_CAPSTARTY)
  LET capEX AS Float = __canvas_geoAt(offset, __CANVAS_GEO_CAPENDX)
  LET capEY AS Float = __canvas_geoAt(offset, __CANVAS_GEO_CAPENDY)
  ' plan-116-E: an ellipse's rotation, as its cosine and sine. Read once per item like
  ' the cap; zero for every other kind and never read by one.
  LET ellCos AS Float = __canvas_geoAt(offset, __CANVAS_GEO_ELLIPSE_COS)
  LET ellSin AS Float = __canvas_geoAt(offset, __CANVAS_GEO_ELLIPSE_SIN)
  ' bug-484: a picture IS a rectangle to the distance function -- its header carries the
  ' rectangle's centre, half-extent and zero radius -- so its coverage, antialiased edge,
  ' clip, transform and stroke are a rectangle's by construction. Only the fill colour
  ' differs, and that is sampled per pixel below. `distKind` is what the distance calls
  ' dispatch on; `kind` keeps saying PICTURE for the fill.
  LET isPicture AS Boolean = kind = __CANVAS_GEO_PICTURE
  MUT distKind AS Integer = kind
  MUT picShadow AS Integer = 0
  MUT picW AS Integer = 0
  MUT picH AS Integer = 0
  MUT picStep AS Integer = 0
  IF isPicture THEN
    distKind = __CANVAS_KIND_RECT
    picShadow = toInt(__canvas_geoAt(offset, __CANVAS_GEO_PICTURE_SHADOW_HI)) * __CANVAS_GEO_PICTURE_SPLIT + toInt(__canvas_geoAt(offset, __CANVAS_GEO_PICTURE_SHADOW_LO))
    picW = toInt(__canvas_geoAt(offset, 20))
    picH = toInt(__canvas_geoAt(offset, 21))
    picStep = toInt(__canvas_geoAt(offset, __CANVAS_GEO_PICTURE_TRANSPARENCY))
  END IF
  ' The destination's left/top edge and extent, from the rectangle slots.
  LET picX0 AS Float = p0 - p2
  LET picY0 AS Float = p1 - p3
  LET picDW AS Float = p2 * 2.0
  LET picDH AS Float = p3 * 2.0

  ' Arc sweep vectors: per-shape constants, so the only sin/cos in the renderer runs
  ' once per arc rather than once per pixel.
  MUT sx AS Float = 0.0
  MUT sy AS Float = 0.0
  MUT ex AS Float = 0.0
  MUT ey AS Float = 0.0
  MUT reflex AS Boolean = FALSE
  IF kind = __CANVAS_GEO_ARC THEN
    LET startAngle AS Float = __canvas_geoAt(offset, 20)
    LET endAngle AS Float = __canvas_geoAt(offset, 21)
    reflex = (endAngle - startAngle) > 3.141592653589793
    sx = __canvas_cos(startAngle)
    sy = __canvas_sin(startAngle)
    ex = __canvas_cos(endAngle)
    ey = __canvas_sin(endAngle)
  END IF

  ' plan-116-B: the clip narrows the SAME four expressions that already clamp the
  ' item's bounds to the surface, so the whole interior costs nothing extra -- only the
  ' at most two columns and two rows the clip edge crosses need per-pixel attention,
  ' and `__canvas_clipCoverage` handles those below.
  '
  ' `toInt` truncation is the right rounding here rather than a floor/ceil pair. A clip
  ' starting at x = 10.3 must still visit pixel 10, which covers [10, 11) and is 70%
  ' inside; toInt gives 10. A clip ENDING at x = 20.0 gives 20, so pixel 20 is visited
  ' and then contributes nothing -- its centre 20.5 is outside, so the coverage is 0.
  ' Visiting one pixel too many is free; missing one would clip a whole column.
  ' plan-116-G: offset FIRST, then clamp to the surface. A group translated partly
  ' off-screen would otherwise be clipped against the rectangle its untranslated bounds
  ' occupy, which is a different rectangle -- it would lose the columns it was moved
  ' *into* and keep ones it was moved out of.
  ' plan-172-G: `__canvas_drawBound` is that sum for a translated entry, and the hull of
  ' the box through the groups' linear part for one with a matrix.
  MUT firstX AS Integer = __canvas_maxI(toInt(__canvas_drawBound(offset, 0, gdx, gdy)), 0)
  MUT lastX AS Integer = __canvas_minI(toInt(__canvas_drawBound(offset, 2, gdx, gdy)), width - 1)
  MUT lastY AS Integer = __canvas_minI(toInt(__canvas_drawBound(offset, 3, gdx, gdy)), height - 1)
  MUT y AS Integer = __canvas_maxI(toInt(__canvas_drawBound(offset, 1, gdx, gdy)), 0)
  LET clipped AS Boolean = __canvas_hasClip(offset)
  LET clipPath AS Boolean = __canvas_geoAt(offset, __CANVAS_GEO_CLIP_MODE) = 2.0
  LET transformed AS Boolean = __canvas_hasTransform(offset)
  ' plan-116-B: the item's BlendMode, read ONCE per item rather than per pixel.
  '
  ' The dispatch lives in `__canvas_blendChannelMode` rather than in four copies of the
  ' pixel loop -- see the Corrections in the plan for the measurement behind that. Mode
  ' 0 keeps the existing full-coverage fast path below, so an unblended item's inner
  ' loop is exactly the one it had.
  LET blendMode AS Integer = toInt(__canvas_geoAt(offset, 26))
  ' plan-163-C: the clip is in the item's own coordinates, so a group's translation moves
  ' it with the item -- reversing plan-116-G's G5, where it was a surface rectangle the
  ' group moved the shape through. These four narrow the already-offset bounds by the
  ' offset clip.
  '
  ' plan-172-G: not under a group matrix, where the clip's box is no longer axis-aligned
  ' on the surface. The per-pixel clip coverage below is 0 outside it, so the loop
  ' visits more pixels and draws the same ones.
  IF clipped AND NOT __CANVAS_GM THEN
    firstX = __canvas_maxI(firstX, toInt(__canvas_geoAt(offset, 22) + gdx))
    y = __canvas_maxI(y, toInt(__canvas_geoAt(offset, 23) + gdy))
    lastX = __canvas_minI(lastX, toInt(__canvas_geoAt(offset, 24) + gdx))
    lastY = __canvas_minI(lastY, toInt(__canvas_geoAt(offset, 25) + gdy))
  END IF
  WHILE y <= lastY
    LET rowBase AS Integer = y * width * 4
    LET spy AS Float = toFloat(y) + 0.5
    LET py AS Float = spy - gdy
    MUT x AS Integer = firstX
    WHILE x <= lastX
      LET spx AS Float = toFloat(x) + 0.5
      LET px AS Float = spx - gdx
      ' plan-116-C: a transformed item is drawn by evaluating its distance field at the
      ' INVERSE-mapped query point, which is what makes one distance function serve
      ' every affine -- no kind needs a transformed variant.
      '
      ' The distance that comes back is in SHAPE space and has to be divided by the
      ' local scale of the mapping, or a scaled-up shape gets a blurry edge and a
      ' scaled-down one a hard, aliased edge. Phase 1 measured the two candidate
      ' corrections: `sqrt(|det M|)` is 37/255 coverage steps wrong for a 2:1 scale,
      ' while `d / ||grad d||` is exact. The gradient is taken by CENTRAL DIFFERENCES at
      ' a fixed epsilon -- deterministic, `+ - * /` and sqrt only, so it is not the
      ' `fwidth`-style hardware derivative `06_canvas.md` bans, and all three renderers
      ' compute the identical value.
      '
      ' Three distance evaluations instead of one, and only for a transformed item: the
      ' flag gates the whole block, so every scene written before this letter pays one
      ' compare per pixel and nothing else.
      ' `dRaw` is the distance in SHAPE space and `dScale` the local scale of the
      ' mapping; `distance` is the surface-space distance the fill uses. Keeping the
      ' raw pair is what makes the STROKE scale with the shape (§4.3): the band is
      ' `|d| - half` in shape space, converted to surface space afterwards, so a 2x
      ' scale gives a 2x-wide outline. Dividing first and then subtracting `half` would
      ' instead hold the outline at a constant surface width, which is the other
      ' defensible reading and not the one this letter chose.
      '
      ' Untransformed, `dScale` is 1.0 and both expressions collapse to exactly the
      ' ones they were before this letter -- not merely equivalent, identical.
      MUT dRaw AS Float = 0.0
      MUT dScale AS Float = 1.0
      MUT distance AS Float = 0.0
      ' The SHAPE-space point: where a picture samples its image. The inverse-mapped
      ' point under a transform, the group-relative point otherwise.
      MUT qx AS Float = px
      MUT qy AS Float = py
      IF transformed THEN
        LET tx AS Float = __canvas_invX(offset, px, py)
        LET ty AS Float = __canvas_invY(offset, px, py)
        qx = tx
        qy = ty
        LET d0 AS Float = __canvas_geoDistance(distKind, tail, edges, tx, ty, p0, p1, p2, p3, radius, sx, sy, ex, ey, reflex, cap, capSX, capSY, capEX, capEY, ellCos, ellSin, dashed)
        LET dxp AS Float = __canvas_geoDistance(distKind, tail, edges, __canvas_invX(offset, px + 0.5, py), __canvas_invY(offset, px + 0.5, py), p0, p1, p2, p3, radius, sx, sy, ex, ey, reflex, cap, capSX, capSY, capEX, capEY, ellCos, ellSin, dashed)
        LET dxm AS Float = __canvas_geoDistance(distKind, tail, edges, __canvas_invX(offset, px - 0.5, py), __canvas_invY(offset, px - 0.5, py), p0, p1, p2, p3, radius, sx, sy, ex, ey, reflex, cap, capSX, capSY, capEX, capEY, ellCos, ellSin, dashed)
        LET dyp AS Float = __canvas_geoDistance(distKind, tail, edges, __canvas_invX(offset, px, py + 0.5), __canvas_invY(offset, px, py + 0.5), p0, p1, p2, p3, radius, sx, sy, ex, ey, reflex, cap, capSX, capSY, capEX, capEY, ellCos, ellSin, dashed)
        LET dym AS Float = __canvas_geoDistance(distKind, tail, edges, __canvas_invX(offset, px, py - 0.5), __canvas_invY(offset, px, py - 0.5), p0, p1, p2, p3, radius, sx, sy, ex, ey, reflex, cap, capSX, capSY, capEX, capEY, ellCos, ellSin, dashed)
        LET gx AS Float = dxp - dxm
        LET gy AS Float = dyp - dym
        LET g AS Float = math::sqrt(gx * gx + gy * gy)
        dRaw = d0
        IF g > 0.000001 THEN
          dScale = g
        END IF
        distance = dRaw / dScale
      ELSE
        dRaw = __canvas_geoDistance(distKind, tail, edges, px, py, p0, p1, p2, p3, radius, sx, sy, ex, ey, reflex, cap, capSX, capSY, capEX, capEY, ellCos, ellSin, dashed)
        distance = dRaw
      END IF
      LET idx AS Integer = rowBase + x * 4
      ' The clip's own coverage at this pixel, folded into the shape's below. Computed
      ' only for a clipped item, and only ever non-255 on the at most two columns and
      ' two rows the clip edge crosses -- the interior was already excluded by the loop
      ' bounds, so this is the boundary's cost and nothing else's.
      MUT clipCov AS Integer = 255
      IF clipped THEN
        ' plan-163-C: a path clip is evaluated at the shape-space point and divided by
        ' ITS OWN scale there (`__canvas_clipPathSurface`), so a transformed clip's edge
        ' is antialiased as a transformed shape's edge of the same geometry is.
        IF clipPath THEN
          clipCov = __canvas_coverage(__canvas_clipPathSurface(offset, px, py, qx, qy, transformed))
        ELSEIF __CANVAS_GM THEN
          clipCov = __canvas_coverage(__canvas_clipRectSurface(offset, px, py, qx, qy))
        ELSE
          clipCov = __canvas_clipCoverage(offset, px, py)
        END IF
      END IF

      ' plan-116-F: the gradient replaces the fill COLOUR and nothing else -- the
      ' shape's distance, its coverage and its stroke are untouched, which is what
      ' keeps a gradient-filled shape's edge identical to a flat-filled one's.
      ' plan-163-D: `t` is computed at the SHAPE-space point `q`, so the item's `transform`
      ' and the group offset carry the ramp with the item (a linear gradient's own
      ' transform is already folded into its points), and the spread is applied before
      ' the stop lookup.
      MUT gR AS Byte = fillR
      MUT gG AS Byte = fillG
      MUT gB AS Byte = fillB
      MUT gA AS Integer = fillA
      MUT rampR AS Byte = fillR
      MUT rampG AS Byte = fillG
      MUT rampB AS Byte = fillB
      IF gradCount >= 2 THEN
        MUT t AS Float = 0.0
        ' plan-154-A: a PolyLine's stops are its per-point colours, and its ramp runs
        ' along the PATH -- the parameter of the nearest point, taken in shape space
        ' (`qx`/`qy`) because that is where the path's own coordinates live.
        IF kind = __CANVAS_GEO_POLYLINE THEN
          t = __canvas_polylineScan(tail, edges, qx, qy, radius, cap, TRUE)
        ELSEIF gradRadial THEN
          ' plan-163-D: the gradient-space point, through the radial ramp's own inverse.
          MUT gx AS Float = qx
          MUT gy AS Float = qy
          IF gradHasInv THEN
            gx = gradIA * qx + gradIC * qy + gradITX
            gy = gradIB * qx + gradID * qy + gradITY
          END IF
          IF gradFocal THEN
            t = __canvas_gradientSpread(__canvas_focalT(gx, gy, gradFX, gradFY, gradLen, gradOX, gradOY, gradFR), gradSpread)
          ELSEIF gradLen > 0.0 THEN
            LET rdx AS Float = gx - gradFX
            LET rdy AS Float = gy - gradFY
            t = __canvas_gradientSpread(math::sqrt(rdx * rdx + rdy * rdy) / gradLen, gradSpread)
          END IF
        ELSE
          IF gradLen2 > 0.0 THEN
            t = __canvas_gradientSpread(((qx - gradFX) * gradAX + (qy - gradFY) * gradAY) / gradLen2, gradSpread)
          END IF
        END IF
        LET gc AS color::Color = __canvas_gradientColor(gradBase, gradCount, t)
        gR = gc.red
        gG = gc.green
        gB = gc.blue
        gA = toInt(gc.alpha)
        rampR = gc.red
        rampG = gc.green
        rampB = gc.blue
        ' plan-163-D: a Line's or Arc's stroke IS its fill: the ramp tinted by the stroke
        ' colour the fill slots carry, whose alpha is the tint's strength (the picture
        ' tint, `__canvas_tintChannel`); the ramp's alpha is what draws.
        IF gradTintFill THEN
          gR = __canvas_tintChannel(toInt(gc.red), toInt(fillR), fillA)
          gG = __canvas_tintChannel(toInt(gc.green), toInt(fillG), fillA)
          gB = __canvas_tintChannel(toInt(gc.blue), toInt(fillB), fillA)
        END IF
      END IF
      ' bug-484: a picture's fill colour is its image, sampled NEAREST -- the plan-116-C
      ' section 4.5 rule glyphs follow, for the same reason: `floor` and `+ - * /` only,
      ' so all three renderers pick the same texel. The texel is then TINTED by
      ' `Paint.fill` (`__canvas_tintChannel`, plan-163-C): the fill's alpha is the tint's
      ' strength, so the zero paint and a white fill draw the image unchanged, and the
      ' picture's own alpha is faded by `transparency` alone.
      '
      ' The index is clamped, not rejected: the antialiased edge pixels just outside the
      ' destination (centre within half a pixel of it) still take coverage, and they
      ' belong to the border texel -- exactly as a rectangle's edge pixels take its fill.
      IF isPicture THEN
        '
        ' bug-1079: the index is `floor(t / d)` corrected by at most one from the exact
        ' products, so it does not hang on the last bit of the quotient. A GPU driver may
        ' round `/` either way (Vulkan allows 2.5 ULP), and where a texel boundary is a
        ' pixel centre -- `t` exactly a whole number of `d` -- that bit chose the texel.
        ' Both shaders' `pictureTexel` take these same three steps.
        LET picTX AS Float = (qx - picX0) * toFloat(picW)
        LET picTY AS Float = (qy - picY0) * toFloat(picH)
        MUT tu AS Integer = math::floor(picTX / picDW)
        MUT tv AS Integer = math::floor(picTY / picDH)
        IF toFloat(tu + 1) * picDW <= picTX THEN
          tu = tu + 1
        ELSEIF toFloat(tu) * picDW > picTX THEN
          tu = tu - 1
        END IF
        IF toFloat(tv + 1) * picDH <= picTY THEN
          tv = tv + 1
        ELSEIF toFloat(tv) * picDH > picTY THEN
          tv = tv - 1
        END IF
        tu = __canvas_maxI(__canvas_minI(tu, picW - 1), 0)
        tv = __canvas_maxI(__canvas_minI(tv, picH - 1), 0)
        LET texel AS Integer = canvas::shadowTexel(picShadow, tv * picW + tu)
        gR = __canvas_tintChannel(texel MOD 256, toInt(fillR), fillA)
        gG = __canvas_tintChannel((texel / 256) MOD 256, toInt(fillG), fillA)
        gB = __canvas_tintChannel((texel / 65536) MOD 256, toInt(fillB), fillA)
        gA = __canvas_fadeAlpha(texel / 16777216, picStep)
      END IF
      IF gA > 0 THEN
        MUT coverage AS Integer = __canvas_coverage(distance)
        IF clipped THEN
          coverage = (coverage * clipCov) / 255
        END IF
        LET alpha AS Integer = (gA * coverage) / 255
        ' The opaque fast path is Normal-ONLY. A Multiply source at full coverage is
        ' still `src * dst`, not `src` -- writing the source directly there would make
        ' every fully-covered pixel of a blended item ignore its mode.
        IF alpha >= 255 AND blendMode = 0 THEN
          out = collections::set(out, idx, gR)
          out = collections::set(out, idx + 1, gG)
          out = collections::set(out, idx + 2, gB)
          out = collections::set(out, idx + 3, toByte(255))
        ELSEIF alpha > 0 THEN
          out = collections::set(out, idx, __canvas_blendChannelMode(collections::getOr(out, idx, toByte(0)), gR, alpha, blendMode))
          out = collections::set(out, idx + 1, __canvas_blendChannelMode(collections::getOr(out, idx + 1, toByte(0)), gG, alpha, blendMode))
          out = collections::set(out, idx + 2, __canvas_blendChannelMode(collections::getOr(out, idx + 2, toByte(0)), gB, alpha, blendMode))
          out = collections::set(out, idx + 3, toByte(255))
        END IF
      END IF

      IF half > 0.0 THEN
        ' plan-163-D: a stroke the ramp paints is the ramp tinted by the stroke colour,
        ' at the ramp's alpha -- drawn even when the stroke colour's alpha is 0.
        MUT sR AS Byte = strokeR
        MUT sG AS Byte = strokeG
        MUT sB AS Byte = strokeB
        MUT sA AS Integer = strokeA
        IF gradApply THEN
          sR = __canvas_tintChannel(toInt(rampR), toInt(strokeR), strokeA)
          sG = __canvas_tintChannel(toInt(rampG), toInt(strokeG), strokeA)
          sB = __canvas_tintChannel(toInt(rampB), toInt(strokeB), strokeA)
          sA = gA
        END IF
        IF sA > 0 THEN
          MUT strokeD AS Float = __canvas_absF(dRaw) - half
          IF dashed THEN
            strokeD = __canvas_dashedStroke(distKind, offset, qx, qy, half)
          ELSEIF joinCount > 0 THEN
            strokeD = __canvas_joinedStroke(kind, offset, tail, edges, qx, qy, p0, p1, p2, p3, half)
          END IF
          MUT band AS Integer = __canvas_coverage(strokeD / dScale)
          IF clipped THEN
            band = (band * clipCov) / 255
          END IF
          LET salpha AS Integer = (sA * band) / 255
          IF salpha >= 255 AND blendMode = 0 THEN
            out = collections::set(out, idx, sR)
            out = collections::set(out, idx + 1, sG)
            out = collections::set(out, idx + 2, sB)
            out = collections::set(out, idx + 3, toByte(255))
          ELSEIF salpha > 0 THEN
            out = collections::set(out, idx, __canvas_blendChannelMode(collections::getOr(out, idx, toByte(0)), sR, salpha, blendMode))
            out = collections::set(out, idx + 1, __canvas_blendChannelMode(collections::getOr(out, idx + 1, toByte(0)), sG, salpha, blendMode))
            out = collections::set(out, idx + 2, __canvas_blendChannelMode(collections::getOr(out, idx + 2, toByte(0)), sB, salpha, blendMode))
            out = collections::set(out, idx + 3, toByte(255))
          END IF
        END IF
      END IF

      x = x + 1
    END WHILE
    y = y + 1
  END WHILE
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("canvas_strokeHalf", STROKE_HALF));
    pkg.add_helper(RegistryHelper::always("canvas_geoRead", GEO_READ));
    pkg.add_helper(RegistryHelper::always("canvas_drawGeometry", DRAW_GEOMETRY));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `__canvas_strokeHalf` reports "this paint does not stroke" as a **negative**
    /// value, not zero. Every native consumer therefore has to read it SIGNED.
    ///
    /// plan-116-H: the Vulkan emitter's blend-split test read it with `load_u32`, which
    /// zero-extends, so `-1.0` in 16.16 (`0xFFFF0000`) compared as 4294901760 and every
    /// blended fill-only item was published as TWO records instead of one. Nothing
    /// caught it for as long as the draw call's instance count came from the emitter's
    /// own cursor -- the extra record was drawn and painted nothing. It only became a
    /// visible defect when the draw list started predicting instance counts
    /// independently, at which point one extra record shifted every later draw base and
    /// the scene lost its tail.
    ///
    /// This pins the sentinel because the cheap "fix" from the other direction -- making
    /// it return `0.0` -- would silently paper over a signed-read bug and leave the next
    /// consumer to rediscover this.
    #[test]
    fn a_paint_that_does_not_stroke_reports_a_negative_stroke_half() {
        let negatives = STROKE_HALF.matches("RETURN 0.0 - 1.0").count();
        assert_eq!(
            negatives, 2,
            "__canvas_strokeHalf must return a NEGATIVE sentinel for both \
             non-stroking cases (zero alpha, non-positive width); native consumers \
             read this field signed and a 0.0 sentinel changes what they publish",
        );
        assert!(
            !STROKE_HALF.contains("RETURN 0.0\n"),
            "a bare 0.0 sentinel makes the no-stroke case indistinguishable from a \
             zero-width stroke under an unsigned read",
        );
    }
}
