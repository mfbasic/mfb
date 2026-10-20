//! plan-176-R: the `helper_items` MFBASIC helper text — drawing a geometry item
//! (`DRAW_GEOMETRY`). Registration lives in `builtins/canvas/helper_items/mod.rs`.

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
pub(super) const DRAW_GEOMETRY: &str =
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
  ' bug-1089: the pixels this draw may write, inclusive -- the surface, narrowed to the
  ' frame's scissor (`__CANVAS_SCISSOR_*`). Every loop below is bounded by these and by
  ' nothing wider.
  LET drawX0 AS Integer = __canvas_maxI(__CANVAS_SCISSOR_X0, 0)
  LET drawY0 AS Integer = __canvas_maxI(__CANVAS_SCISSOR_Y0, 0)
  LET drawX1 AS Integer = __canvas_minI(__CANVAS_SCISSOR_X1, width) - 1
  LET drawY1 AS Integer = __canvas_minI(__CANVAS_SCISSOR_Y1, height) - 1
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
        LET loX AS Integer = __canvas_maxI(toInt(__canvas_minF(__canvas_minF(cx0, cx1), __canvas_minF(cx2, cx3))) - 1, drawX0)
        LET loY AS Integer = __canvas_maxI(toInt(__canvas_minF(__canvas_minF(cy0, cy1), __canvas_minF(cy2, cy3))) - 1, drawY0)
        LET hiX AS Integer = __canvas_minI(toInt(__canvas_maxF(__canvas_maxF(cx0, cx1), __canvas_maxF(cx2, cx3))) + 1, drawX1)
        LET hiY AS Integer = __canvas_minI(toInt(__canvas_maxF(__canvas_maxF(cy0, cy1), __canvas_maxF(cy2, cy3))) + 1, drawY1)
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
        IF sy >= drawY0 AND sy <= drawY1 THEN
          LET gRowBase AS Integer = sy * width * 4
          MUT gCol AS Integer = 0
          WHILE gCol < gw
            LET sx AS Integer = gsx + gCol
            IF sx >= drawX0 AND sx <= drawX1 THEN
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
  LET isTile AS Boolean = kind = __CANVAS_GEO_TILE
  MUT distKind AS Integer = kind
  MUT picShadow AS Integer = 0
  MUT picW AS Integer = 0
  MUT picH AS Integer = 0
  MUT picStep AS Integer = 0
  IF isPicture OR isTile THEN
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
  MUT picSX AS Float = 0.0
  MUT picSY AS Float = 0.0
  MUT picSW AS Float = toFloat(picW)
  MUT picSH AS Float = toFloat(picH)
  IF isTile THEN
    picSX = __canvas_geoAt(offset, __CANVAS_GEO_TILE_SRC_X)
    picSY = __canvas_geoAt(offset, __CANVAS_GEO_TILE_SRC_Y)
    picSW = __canvas_geoAt(offset, __CANVAS_GEO_TILE_SRC_W)
    picSH = __canvas_geoAt(offset, __CANVAS_GEO_TILE_SRC_H)
  END IF
  ' bug-1093: the texels a tile's source rectangle touches, `[floor(src), ceil(src + size) - 1]`
  ' on each axis. A partly covered edge pixel's centre lies outside the destination, so its
  ' mapped texel lies outside the source; the index is clamped to this range (before the
  ' image-bounds test) so that pixel takes the source's own edge texel. Both shaders'
  ' `tileColour` take the same step.
  MUT tileLoU AS Integer = 0
  MUT tileHiU AS Integer = 0
  MUT tileLoV AS Integer = 0
  MUT tileHiV AS Integer = 0
  IF isTile THEN
    tileLoU = math::floor(picSX)
    tileHiU = math::ceil(picSX + picSW) - 1
    tileLoV = math::floor(picSY)
    tileHiV = math::ceil(picSY + picSH) - 1
  END IF

  ' Arc sweep vectors: per-shape constants, so the only sin/cos in the renderer runs
  ' once per arc rather than once per pixel.
  MUT sx AS Float = 0.0
  MUT sy AS Float = 0.0
  MUT ex AS Float = 0.0
  MUT ey AS Float = 0.0
  MUT turn AS Integer = 0
  IF kind = __CANVAS_GEO_ARC THEN
    LET startAngle AS Float = __canvas_geoAt(offset, 20)
    LET endAngle AS Float = __canvas_geoAt(offset, 21)
    turn = __canvas_arcTurn(startAngle, endAngle)
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
  MUT firstX AS Integer = __canvas_maxI(toInt(__canvas_drawBound(offset, 0, gdx, gdy)), drawX0)
  MUT lastX AS Integer = __canvas_minI(toInt(__canvas_drawBound(offset, 2, gdx, gdy)), drawX1)
  MUT lastY AS Integer = __canvas_minI(toInt(__canvas_drawBound(offset, 3, gdx, gdy)), drawY1)
  MUT y AS Integer = __canvas_maxI(toInt(__canvas_drawBound(offset, 1, gdx, gdy)), drawY0)
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
        LET d0 AS Float = __canvas_geoDistance(distKind, tail, edges, tx, ty, p0, p1, p2, p3, radius, sx, sy, ex, ey, turn, cap, capSX, capSY, capEX, capEY, ellCos, ellSin, dashed)
        LET dxp AS Float = __canvas_geoDistance(distKind, tail, edges, __canvas_invX(offset, px + 0.5, py), __canvas_invY(offset, px + 0.5, py), p0, p1, p2, p3, radius, sx, sy, ex, ey, turn, cap, capSX, capSY, capEX, capEY, ellCos, ellSin, dashed)
        LET dxm AS Float = __canvas_geoDistance(distKind, tail, edges, __canvas_invX(offset, px - 0.5, py), __canvas_invY(offset, px - 0.5, py), p0, p1, p2, p3, radius, sx, sy, ex, ey, turn, cap, capSX, capSY, capEX, capEY, ellCos, ellSin, dashed)
        LET dyp AS Float = __canvas_geoDistance(distKind, tail, edges, __canvas_invX(offset, px, py + 0.5), __canvas_invY(offset, px, py + 0.5), p0, p1, p2, p3, radius, sx, sy, ex, ey, turn, cap, capSX, capSY, capEX, capEY, ellCos, ellSin, dashed)
        LET dym AS Float = __canvas_geoDistance(distKind, tail, edges, __canvas_invX(offset, px, py - 0.5), __canvas_invY(offset, px, py - 0.5), p0, p1, p2, p3, radius, sx, sy, ex, ey, turn, cap, capSX, capSY, capEX, capEY, ellCos, ellSin, dashed)
        LET gx AS Float = dxp - dxm
        LET gy AS Float = dyp - dym
        LET g AS Float = math::sqrt(gx * gx + gy * gy)
        dRaw = d0
        IF g > 0.000001 THEN
          dScale = g
        END IF
        distance = dRaw / dScale
      ELSE
        dRaw = __canvas_geoDistance(distKind, tail, edges, px, py, p0, p1, p2, p3, radius, sx, sy, ex, ey, turn, cap, capSX, capSY, capEX, capEY, ellCos, ellSin, dashed)
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
      IF isPicture OR isTile THEN
        '
        ' bug-1079: the index is `floor(t / d)` corrected by at most one from the exact
        ' products, so it does not hang on the last bit of the quotient. A GPU driver may
        ' round `/` either way (Vulkan allows 2.5 ULP), and where a texel boundary is a
        ' pixel centre -- `t` exactly a whole number of `d` -- that bit chose the texel.
        ' Both shaders' `pictureTexel` take these same three steps.
        LET picTX AS Float = picSX * picDW + (qx - picX0) * picSW
        LET picTY AS Float = picSY * picDH + (qy - picY0) * picSH
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
        IF isTile THEN
          tu = __canvas_maxI(__canvas_minI(tu, tileHiU), tileLoU)
          tv = __canvas_maxI(__canvas_minI(tv, tileHiV), tileLoV)
        END IF
        IF isTile AND (tu < 0 OR tv < 0 OR tu >= picW OR tv >= picH) THEN
          gR = toByte(0)
          gG = toByte(0)
          gB = toByte(0)
          gA = 0
        ELSE
          tu = __canvas_maxI(__canvas_minI(tu, picW - 1), 0)
          tv = __canvas_maxI(__canvas_minI(tv, picH - 1), 0)
          LET texel AS Integer = canvas::shadowTexel(picShadow, tv * picW + tu)
          IF isTile THEN
            gR = toByte(texel MOD 256)
            gG = toByte((texel / 256) MOD 256)
            gB = toByte((texel / 65536) MOD 256)
            gA = texel / 16777216
          ELSE
            gR = __canvas_tintChannel(texel MOD 256, toInt(fillR), fillA)
            gG = __canvas_tintChannel((texel / 256) MOD 256, toInt(fillG), fillA)
            gB = __canvas_tintChannel((texel / 65536) MOD 256, toInt(fillB), fillA)
            gA = __canvas_fadeAlpha(texel / 16777216, picStep)
          END IF
        END IF
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
