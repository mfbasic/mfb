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
  IF toInt(paint.stroke.alpha) <= 0 THEN
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
FUNC __canvas_hasTransform(offset AS Integer) AS Boolean
  RETURN __canvas_geoAt(offset, 33) > 0.5
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

FUNC __canvas_invX(offset AS Integer, px AS Float, py AS Float) AS Float
  RETURN __canvas_f32FromBits(__canvas_geoAt(offset, 27)) * px + __canvas_f32FromBits(__canvas_geoAt(offset, 29)) * py + __canvas_f32FromBits(__canvas_geoAt(offset, 31))
END FUNC

FUNC __canvas_invY(offset AS Integer, px AS Float, py AS Float) AS Float
  RETURN __canvas_f32FromBits(__canvas_geoAt(offset, 28)) * px + __canvas_f32FromBits(__canvas_geoAt(offset, 30)) * py + __canvas_f32FromBits(__canvas_geoAt(offset, 32))
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
FUNC __canvas_clipPathSurface(offset AS Integer, px AS Float, py AS Float, qx AS Float, qy AS Float, transformed AS Boolean) AS Float
  LET d AS Float = __canvas_clipPathDistance(offset, qx, qy)
  IF NOT transformed THEN
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
' (`qx`, `qy`; a glyph is a bitmap, so there is no distance gradient to divide by), a
' rectangle at its group-relative one (`px`, `py`).
FUNC __canvas_glyphClip(offset AS Integer, qx AS Float, qy AS Float, px AS Float, py AS Float) AS Integer
  IF __canvas_geoAt(offset, __CANVAS_GEO_CLIP_MODE) = 2.0 THEN
    RETURN __canvas_coverage(__canvas_clipPathDistance(offset, qx, qy))
  END IF
  RETURN __canvas_clipCoverage(offset, px, py)
END FUNC

' plan-163-C: a path clip's signed distance at the item's own point (`x`, `y`, the
' shape-space point): its edges under its rule, negative inside.
FUNC __canvas_clipPathDistance(offset AS Integer, x AS Float, y AS Float) AS Float
  LET edges AS Integer = toInt(__canvas_geoAt(offset, __CANVAS_GEO_CLIP_EDGE_COUNT))
  RETURN __canvas_edgeDistanceRule(__canvas_clipEdgeBase(offset), edges, x, y, __canvas_geoAt(offset, __CANVAS_GEO_CLIP_RULE) <> 0.0)
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
' established for `Paint.transform`, specialised to a translation, so no distance
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
      ' one, which forward-maps this box through `Paint.transform` -- the translation
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
        LET fwd AS List OF Float = __canvas_forwardOf(__canvas_f32FromBits(__canvas_geoAt(offset, 27)), __canvas_f32FromBits(__canvas_geoAt(offset, 28)), __canvas_f32FromBits(__canvas_geoAt(offset, 29)), __canvas_f32FromBits(__canvas_geoAt(offset, 30)), __canvas_f32FromBits(__canvas_geoAt(offset, 31)), __canvas_f32FromBits(__canvas_geoAt(offset, 32)))
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
        ' surface translation, and passing it through `Paint.transform` would rotate
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
  MUT firstX AS Integer = __canvas_maxI(toInt(__canvas_geoAt(offset, 16) + gdx), 0)
  MUT lastX AS Integer = __canvas_minI(toInt(__canvas_geoAt(offset, 18) + gdx), width - 1)
  MUT lastY AS Integer = __canvas_minI(toInt(__canvas_geoAt(offset, 19) + gdy), height - 1)
  MUT y AS Integer = __canvas_maxI(toInt(__canvas_geoAt(offset, 17) + gdy), 0)
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
  IF clipped THEN
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
        ELSE
          clipCov = __canvas_clipCoverage(offset, px, py)
        END IF
      END IF

      ' plan-116-F: the gradient replaces the fill COLOUR and nothing else -- the
      ' shape's distance, its coverage and its stroke are untouched, which is what
      ' keeps a gradient-filled shape's edge identical to a flat-filled one's. `t` is
      ' computed in surface pixels, so the ramp composes with `Paint.transform` the
      ' same way the shape does.
      MUT gR AS Byte = fillR
      MUT gG AS Byte = fillG
      MUT gB AS Byte = fillB
      MUT gA AS Integer = fillA
      IF gradCount >= 2 THEN
        MUT t AS Float = 0.0
        ' plan-154-A: a PolyLine's stops are its per-point colours, and its ramp runs
        ' along the PATH -- the parameter of the nearest point, taken in shape space
        ' (`qx`/`qy`) because that is where the path's own coordinates live.
        IF kind = __CANVAS_GEO_POLYLINE THEN
          t = __canvas_polylineScan(tail, edges, qx, qy, radius, cap, TRUE)
        ELSEIF gradRadial THEN
          IF gradLen > 0.0 THEN
            LET rdx AS Float = px - gradFX
            LET rdy AS Float = py - gradFY
            t = math::sqrt(rdx * rdx + rdy * rdy) / gradLen
          END IF
        ELSE
          IF gradLen2 > 0.0 THEN
            t = ((px - gradFX) * gradAX + (py - gradFY) * gradAY) / gradLen2
          END IF
        END IF
        LET gc AS color::Color = __canvas_gradientColor(gradBase, gradCount, t)
        gR = gc.red
        gG = gc.green
        gB = gc.blue
        gA = toInt(gc.alpha)
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
        LET tu AS Integer = __canvas_maxI(__canvas_minI(math::floor((qx - picX0) * toFloat(picW) / picDW), picW - 1), 0)
        LET tv AS Integer = __canvas_maxI(__canvas_minI(math::floor((qy - picY0) * toFloat(picH) / picDH), picH - 1), 0)
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
        IF strokeA > 0 THEN
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
          LET salpha AS Integer = (strokeA * band) / 255
          IF salpha >= 255 AND blendMode = 0 THEN
            out = collections::set(out, idx, strokeR)
            out = collections::set(out, idx + 1, strokeG)
            out = collections::set(out, idx + 2, strokeB)
            out = collections::set(out, idx + 3, toByte(255))
          ELSEIF salpha > 0 THEN
            out = collections::set(out, idx, __canvas_blendChannelMode(collections::getOr(out, idx, toByte(0)), strokeR, salpha, blendMode))
            out = collections::set(out, idx + 1, __canvas_blendChannelMode(collections::getOr(out, idx + 1, toByte(0)), strokeG, salpha, blendMode))
            out = collections::set(out, idx + 2, __canvas_blendChannelMode(collections::getOr(out, idx + 2, toByte(0)), strokeB, salpha, blendMode))
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
