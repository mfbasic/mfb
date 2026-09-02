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
' Slots 22-25 hold the clip RESOLVED to x0,y0,x1,y1, and a zero-area rectangle means
' unclipped -- which is what an unset `Paint.clip` is. Testing both extents rather than
' comparing against zero also rejects a negative extent, which `Bounds` cannot forbid:
' `w := -5.0` gives x1 < x0 and must mean "draws nothing", not "draws everything".
'
' The per-pixel clip coverage is gated on this, so an unclipped item -- every item in
' every scene written before this letter -- pays one compare per ITEM and nothing per
' pixel.
FUNC __canvas_hasClip(offset AS Integer) AS Boolean
  RETURN __canvas_geoAt(offset, 22) < __canvas_geoAt(offset, 24) AND __canvas_geoAt(offset, 23) < __canvas_geoAt(offset, 25)
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
FUNC __canvas_clipCoverage(offset AS Integer, px AS Float, py AS Float) AS Integer
  LET x0 AS Float = __canvas_geoAt(offset, 22)
  LET y0 AS Float = __canvas_geoAt(offset, 23)
  LET x1 AS Float = __canvas_geoAt(offset, 24)
  LET y1 AS Float = __canvas_geoAt(offset, 25)
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
r#"FUNC __canvas_drawGeometry(surface AS List OF Byte, width AS Integer, height AS Integer, offset AS Integer) AS List OF Byte
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
      LET gx AS Integer = collections::getOr(__CANVAS_GLYPH_META, meta, 0) + toInt(collections::getOr(__CANVAS_GEO_DATA, runBase + 1, 0.0))
      LET gy AS Integer = collections::getOr(__CANVAS_GLYPH_META, meta + 1, 0) + toInt(collections::getOr(__CANVAS_GEO_DATA, runBase + 2, 0.0))
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
        LET cx0 AS Float = fa * bx0 + fc * by0 + ftx
        LET cy0 AS Float = fb * bx0 + fd * by0 + fty
        LET cx1 AS Float = fa * bx1 + fc * by0 + ftx
        LET cy1 AS Float = fb * bx1 + fd * by0 + fty
        LET cx2 AS Float = fa * bx0 + fc * by1 + ftx
        LET cy2 AS Float = fb * bx0 + fd * by1 + fty
        LET cx3 AS Float = fa * bx1 + fc * by1 + ftx
        LET cy3 AS Float = fb * bx1 + fd * by1 + fty
        LET loX AS Integer = __canvas_maxI(toInt(__canvas_minF(__canvas_minF(cx0, cx1), __canvas_minF(cx2, cx3))) - 1, 0)
        LET loY AS Integer = __canvas_maxI(toInt(__canvas_minF(__canvas_minF(cy0, cy1), __canvas_minF(cy2, cy3))) - 1, 0)
        LET hiX AS Integer = __canvas_minI(toInt(__canvas_maxF(__canvas_maxF(cx0, cx1), __canvas_maxF(cx2, cx3))) + 1, width - 1)
        LET hiY AS Integer = __canvas_minI(toInt(__canvas_maxF(__canvas_maxF(cy0, cy1), __canvas_maxF(cy2, cy3))) + 1, height - 1)
        MUT ty AS Integer = loY
        WHILE ty <= hiY
          LET tRowBase AS Integer = ty * width * 4
          MUT tx AS Integer = loX
          WHILE tx <= hiX
            LET fpx AS Float = toFloat(tx) + 0.5
            LET fpy AS Float = toFloat(ty) + 0.5
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
                tcov = (tcov * __canvas_clipCoverage(offset, fpx, fpy)) / 255
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
        LET sy AS Integer = gy + gRow
        IF sy >= 0 AND sy < height THEN
          LET gRowBase AS Integer = sy * width * 4
          MUT gCol AS Integer = 0
          WHILE gCol < gw
            LET sx AS Integer = gx + gCol
            IF sx >= 0 AND sx < width THEN
              MUT cover AS Integer = toInt(collections::getOr(__CANVAS_GLYPH_COV, gStart + gRow * gw + gCol, toByte(0)))
              IF tClipped THEN
                cover = (cover * __canvas_clipCoverage(offset, toFloat(sx) + 0.5, toFloat(sy) + 0.5)) / 255
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
  ' plan-116-D. Read once per item, not per pixel: it is a per-shape constant like the
  ' arc's sweep vectors below. Only Line and Arc ever write it; every other kind leaves
  ' it at the blank header's zero and never reaches a cap branch.
  LET cap AS Integer = toInt(__canvas_geoAt(offset, __CANVAS_GEO_CAP))

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
  MUT firstX AS Integer = __canvas_maxI(toInt(__canvas_geoAt(offset, 16)), 0)
  MUT lastX AS Integer = __canvas_minI(toInt(__canvas_geoAt(offset, 18)), width - 1)
  MUT lastY AS Integer = __canvas_minI(toInt(__canvas_geoAt(offset, 19)), height - 1)
  MUT y AS Integer = __canvas_maxI(toInt(__canvas_geoAt(offset, 17)), 0)
  LET clipped AS Boolean = __canvas_hasClip(offset)
  LET transformed AS Boolean = __canvas_hasTransform(offset)
  ' plan-116-B: the item's BlendMode, read ONCE per item rather than per pixel.
  '
  ' The dispatch lives in `__canvas_blendChannelMode` rather than in four copies of the
  ' pixel loop -- see the Corrections in the plan for the measurement behind that. Mode
  ' 0 keeps the existing full-coverage fast path below, so an unblended item's inner
  ' loop is exactly the one it had.
  LET blendMode AS Integer = toInt(__canvas_geoAt(offset, 26))
  IF clipped THEN
    firstX = __canvas_maxI(firstX, toInt(__canvas_geoAt(offset, 22)))
    y = __canvas_maxI(y, toInt(__canvas_geoAt(offset, 23)))
    lastX = __canvas_minI(lastX, toInt(__canvas_geoAt(offset, 24)))
    lastY = __canvas_minI(lastY, toInt(__canvas_geoAt(offset, 25)))
  END IF
  WHILE y <= lastY
    LET rowBase AS Integer = y * width * 4
    LET py AS Float = toFloat(y) + 0.5
    MUT x AS Integer = firstX
    WHILE x <= lastX
      LET px AS Float = toFloat(x) + 0.5
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
      IF transformed THEN
        LET tx AS Float = __canvas_invX(offset, px, py)
        LET ty AS Float = __canvas_invY(offset, px, py)
        LET d0 AS Float = __canvas_geoDistance(kind, tail, edges, tx, ty, p0, p1, p2, p3, radius, sx, sy, ex, ey, reflex, cap)
        LET dxp AS Float = __canvas_geoDistance(kind, tail, edges, __canvas_invX(offset, px + 0.5, py), __canvas_invY(offset, px + 0.5, py), p0, p1, p2, p3, radius, sx, sy, ex, ey, reflex, cap)
        LET dxm AS Float = __canvas_geoDistance(kind, tail, edges, __canvas_invX(offset, px - 0.5, py), __canvas_invY(offset, px - 0.5, py), p0, p1, p2, p3, radius, sx, sy, ex, ey, reflex, cap)
        LET dyp AS Float = __canvas_geoDistance(kind, tail, edges, __canvas_invX(offset, px, py + 0.5), __canvas_invY(offset, px, py + 0.5), p0, p1, p2, p3, radius, sx, sy, ex, ey, reflex, cap)
        LET dym AS Float = __canvas_geoDistance(kind, tail, edges, __canvas_invX(offset, px, py - 0.5), __canvas_invY(offset, px, py - 0.5), p0, p1, p2, p3, radius, sx, sy, ex, ey, reflex, cap)
        LET gx AS Float = dxp - dxm
        LET gy AS Float = dyp - dym
        LET g AS Float = math::sqrt(gx * gx + gy * gy)
        dRaw = d0
        IF g > 0.000001 THEN
          dScale = g
        END IF
        distance = dRaw / dScale
      ELSE
        dRaw = __canvas_geoDistance(kind, tail, edges, px, py, p0, p1, p2, p3, radius, sx, sy, ex, ey, reflex, cap)
        distance = dRaw
      END IF
      LET idx AS Integer = rowBase + x * 4
      ' The clip's own coverage at this pixel, folded into the shape's below. Computed
      ' only for a clipped item, and only ever non-255 on the at most two columns and
      ' two rows the clip edge crosses -- the interior was already excluded by the loop
      ' bounds, so this is the boundary's cost and nothing else's.
      MUT clipCov AS Integer = 255
      IF clipped THEN
        clipCov = __canvas_clipCoverage(offset, px, py)
      END IF

      IF fillA > 0 THEN
        MUT coverage AS Integer = __canvas_coverage(distance)
        IF clipped THEN
          coverage = (coverage * clipCov) / 255
        END IF
        LET alpha AS Integer = (fillA * coverage) / 255
        ' The opaque fast path is Normal-ONLY. A Multiply source at full coverage is
        ' still `src * dst`, not `src` -- writing the source directly there would make
        ' every fully-covered pixel of a blended item ignore its mode.
        IF alpha >= 255 AND blendMode = 0 THEN
          out = collections::set(out, idx, fillR)
          out = collections::set(out, idx + 1, fillG)
          out = collections::set(out, idx + 2, fillB)
          out = collections::set(out, idx + 3, toByte(255))
        ELSEIF alpha > 0 THEN
          out = collections::set(out, idx, __canvas_blendChannelMode(collections::getOr(out, idx, toByte(0)), fillR, alpha, blendMode))
          out = collections::set(out, idx + 1, __canvas_blendChannelMode(collections::getOr(out, idx + 1, toByte(0)), fillG, alpha, blendMode))
          out = collections::set(out, idx + 2, __canvas_blendChannelMode(collections::getOr(out, idx + 2, toByte(0)), fillB, alpha, blendMode))
          out = collections::set(out, idx + 3, toByte(255))
        END IF
      END IF

      IF half > 0.0 THEN
        IF strokeA > 0 THEN
          MUT band AS Integer = __canvas_coverage((__canvas_absF(dRaw) - half) / dScale)
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
