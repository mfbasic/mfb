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
    MUT gi AS Integer = 0
    WHILE gi < tGlyphs
      LET runBase AS Integer = runAt + gi * 3
      LET meta AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, runBase, 0.0)) * 5
      LET gx AS Integer = collections::getOr(__CANVAS_GLYPH_META, meta, 0) + toInt(collections::getOr(__CANVAS_GEO_DATA, runBase + 1, 0.0))
      LET gy AS Integer = collections::getOr(__CANVAS_GLYPH_META, meta + 1, 0) + toInt(collections::getOr(__CANVAS_GEO_DATA, runBase + 2, 0.0))
      LET gw AS Integer = collections::getOr(__CANVAS_GLYPH_META, meta + 2, 0)
      LET gh AS Integer = collections::getOr(__CANVAS_GLYPH_META, meta + 3, 0)
      LET gStart AS Integer = collections::getOr(__CANVAS_GLYPH_META, meta + 4, 0)
      MUT gRow AS Integer = 0
      WHILE gRow < gh
        LET sy AS Integer = gy + gRow
        IF sy >= 0 AND sy < height THEN
          LET gRowBase AS Integer = sy * width * 4
          MUT gCol AS Integer = 0
          WHILE gCol < gw
            LET sx AS Integer = gx + gCol
            IF sx >= 0 AND sx < width THEN
              LET cover AS Integer = toInt(collections::getOr(__CANVAS_GLYPH_COV, gStart + gRow * gw + gCol, toByte(0)))
              IF cover > 0 THEN
                LET gAlpha AS Integer = (tA * cover) / 255
                LET gIdx AS Integer = gRowBase + sx * 4
                IF gAlpha >= 255 THEN
                  out = collections::set(out, gIdx, tR)
                  out = collections::set(out, gIdx + 1, tG)
                  out = collections::set(out, gIdx + 2, tB)
                  out = collections::set(out, gIdx + 3, toByte(255))
                ELSEIF gAlpha > 0 THEN
                  out = collections::set(out, gIdx, __canvas_blendChannel(collections::getOr(out, gIdx, toByte(0)), tR, gAlpha))
                  out = collections::set(out, gIdx + 1, __canvas_blendChannel(collections::getOr(out, gIdx + 1, toByte(0)), tG, gAlpha))
                  out = collections::set(out, gIdx + 2, __canvas_blendChannel(collections::getOr(out, gIdx + 2, toByte(0)), tB, gAlpha))
                  out = collections::set(out, gIdx + 3, toByte(255))
                END IF
              END IF
            END IF
            gCol = gCol + 1
          END WHILE
        END IF
        gRow = gRow + 1
      END WHILE
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

  LET firstX AS Integer = __canvas_maxI(toInt(__canvas_geoAt(offset, 16)), 0)
  LET lastX AS Integer = __canvas_minI(toInt(__canvas_geoAt(offset, 18)), width - 1)
  LET lastY AS Integer = __canvas_minI(toInt(__canvas_geoAt(offset, 19)), height - 1)
  MUT y AS Integer = __canvas_maxI(toInt(__canvas_geoAt(offset, 17)), 0)
  WHILE y <= lastY
    LET rowBase AS Integer = y * width * 4
    LET py AS Float = toFloat(y) + 0.5
    MUT x AS Integer = firstX
    WHILE x <= lastX
      LET px AS Float = toFloat(x) + 0.5
      LET distance AS Float = __canvas_geoDistance(kind, tail, edges, px, py, p0, p1, p2, p3, radius, sx, sy, ex, ey, reflex)
      LET idx AS Integer = rowBase + x * 4

      IF fillA > 0 THEN
        LET coverage AS Integer = __canvas_coverage(distance)
        LET alpha AS Integer = (fillA * coverage) / 255
        IF alpha >= 255 THEN
          out = collections::set(out, idx, fillR)
          out = collections::set(out, idx + 1, fillG)
          out = collections::set(out, idx + 2, fillB)
          out = collections::set(out, idx + 3, toByte(255))
        ELSEIF alpha > 0 THEN
          out = collections::set(out, idx, __canvas_blendChannel(collections::getOr(out, idx, toByte(0)), fillR, alpha))
          out = collections::set(out, idx + 1, __canvas_blendChannel(collections::getOr(out, idx + 1, toByte(0)), fillG, alpha))
          out = collections::set(out, idx + 2, __canvas_blendChannel(collections::getOr(out, idx + 2, toByte(0)), fillB, alpha))
          out = collections::set(out, idx + 3, toByte(255))
        END IF
      END IF

      IF half > 0.0 THEN
        IF strokeA > 0 THEN
          LET band AS Integer = __canvas_coverage(__canvas_absF(distance) - half)
          LET salpha AS Integer = (strokeA * band) / 255
          IF salpha >= 255 THEN
            out = collections::set(out, idx, strokeR)
            out = collections::set(out, idx + 1, strokeG)
            out = collections::set(out, idx + 2, strokeB)
            out = collections::set(out, idx + 3, toByte(255))
          ELSEIF salpha > 0 THEN
            out = collections::set(out, idx, __canvas_blendChannel(collections::getOr(out, idx, toByte(0)), strokeR, salpha))
            out = collections::set(out, idx + 1, __canvas_blendChannel(collections::getOr(out, idx + 1, toByte(0)), strokeG, salpha))
            out = collections::set(out, idx + 2, __canvas_blendChannel(collections::getOr(out, idx + 2, toByte(0)), strokeB, salpha))
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
