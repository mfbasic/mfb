//! plan-176-R: the `helper_items` MFBASIC helper text — the geometry-record readers
//! (`GEO_READ`). Registration lives in `builtins/canvas/helper_items/mod.rs`.

/// Read one slot of a geometry record.
#[rustfmt::skip]
pub(super) const GEO_READ: &str =
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

' bug-1089: the pixel rectangle a draw may write, half-open -- the draw's scissor. A
' partial redraw (damage mode) keeps the previous frame and clears only the damage
' rectangle, so every item it draws must write inside that rectangle and nowhere else:
' an unchanged item whose box runs outside it would otherwise blend its edge a second
' time over the kept pixels. `__canvas_renderScene` makes the frame's rectangle current
' (the whole surface for a full redraw) and `__canvas_drawGeometry` narrows every pixel
' loop to it. Unbounded until set, so a draw outside the walk is clamped to the surface
' alone.
MUT __CANVAS_SCISSOR_X0 AS Integer = 0
MUT __CANVAS_SCISSOR_Y0 AS Integer = 0
MUT __CANVAS_SCISSOR_X1 AS Integer = 1073741824
MUT __CANVAS_SCISSOR_Y1 AS Integer = 1073741824

FUNC __canvas_enterScissor(x0 AS Integer, y0 AS Integer, x1 AS Integer, y1 AS Integer) AS Nothing
  __CANVAS_SCISSOR_X0 = x0
  __CANVAS_SCISSOR_Y0 = y0
  __CANVAS_SCISSOR_X1 = x1
  __CANVAS_SCISSOR_Y1 = y1
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
