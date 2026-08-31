//! `__canvas_drawGeometry` — rasterise one generated geometry record.
//!
//! The rasteriser never sees a `DrawItem`. It reads the flat float record
//! `helper_geometry.rs` produced, which is the same buffer plan-98-E/F upload to the
//! GPU — so the oracle and the GPU backends consume one representation rather than
//! two that could drift.
//!
//! Each kind does the same three things: claim its bounding box (already computed
//! into the header), fill the interior with the fill colour, then stroke the outline
//! with the stroke colour. Fill and stroke are the *same* distance function offset
//! differently — an outline is the band where `|distance| <= strokeWidth/2` — which
//! is why no kind needs an outline rasteriser of its own.
//!
//! `Line` and `Arc` have no interior, so they take the stroke path only. `Text` and
//! `Picture` generate the `__CANVAS_GEO_NONE` kind and draw nothing until plan-98-G
//! brings a font and an image sampler; they still occupy a geometry record so item
//! indices, hashes and offsets stay parallel.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

/// The half-width of a stroke, or a negative number when there is nothing to stroke.
///
/// Folding "is there a stroke at all" into the same value the band test uses keeps
/// the caller from repeating the two-part check (`alpha > 0` and `width > 0`) in
/// every arm.
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

/// Read a header slot, and rebuild a `Color` from the four slots at `base`.
///
/// Colours are carried through the geometry buffer as floats so the record is one
/// uniform float array — which is what makes it a GPU vertex/uniform buffer rather
/// than a struct needing a per-field upload path.
#[rustfmt::skip]
const GEO_READ: &str =
r#"FUNC __canvas_geoAt(offset AS Integer, slot AS Integer) AS Float
  RETURN collections::getOr(__CANVAS_GEO_DATA, offset + slot, 0.0)
END FUNC

FUNC __canvas_geoColor(offset AS Integer, base AS Integer) AS Color
  RETURN Color[red := __canvas_clampByte(toInt(__canvas_geoAt(offset, base))), green := __canvas_clampByte(toInt(__canvas_geoAt(offset, base + 1))), blue := __canvas_clampByte(toInt(__canvas_geoAt(offset, base + 2))), alpha := __canvas_clampByte(toInt(__canvas_geoAt(offset, base + 3)))]
END FUNC"#;

/// Rasterise one geometry record.
///
/// The bounds come from the header rather than being recomputed, which is the point
/// of generating them once: a repaint of an unchanged scene re-reads them instead of
/// re-deriving them from the item's fields.
#[rustfmt::skip]
const DRAW_GEOMETRY: &str =
r#"FUNC __canvas_drawGeometry(surface AS List OF Byte, width AS Integer, height AS Integer, offset AS Integer) AS List OF Byte
  LET kind AS Integer = toInt(__canvas_geoAt(offset, 0))
  IF kind = __CANVAS_GEO_NONE THEN
    RETURN surface
  END IF
  LET p0 AS Float = __canvas_geoAt(offset, 2)
  LET p1 AS Float = __canvas_geoAt(offset, 3)
  LET p2 AS Float = __canvas_geoAt(offset, 4)
  LET p3 AS Float = __canvas_geoAt(offset, 5)
  LET radius AS Float = __canvas_geoAt(offset, 6)
  LET half AS Float = __canvas_geoAt(offset, 7)
  LET fill AS Color = __canvas_geoColor(offset, 8)
  LET stroke AS Color = __canvas_geoColor(offset, 12)
  LET minX AS Integer = toInt(__canvas_geoAt(offset, 16))
  LET minY AS Integer = toInt(__canvas_geoAt(offset, 17))
  LET maxX AS Integer = toInt(__canvas_geoAt(offset, 18))
  LET maxY AS Integer = toInt(__canvas_geoAt(offset, 19))
  MUT out AS List OF Byte = surface

  IF kind = __CANVAS_GEO_ARC THEN
    RETURN __canvas_fillArc(out, width, height, p0, p1, p2, __canvas_geoAt(offset, 20), __canvas_geoAt(offset, 21), radius, stroke)
  END IF
  IF kind = __CANVAS_GEO_POLYGON THEN
    RETURN __canvas_drawPolygonGeometry(out, width, height, offset, minX, minY, maxX, maxY, half, fill, stroke)
  END IF
  IF kind = __CANVAS_KIND_SEGMENT THEN
    RETURN __canvas_fillSpan(out, width, height, minX, minY, maxX, maxY, __CANVAS_KIND_SEGMENT, p0, p1, p2, p3, radius, stroke)
  END IF

  out = __canvas_fillSpan(out, width, height, minX, minY, maxX, maxY, kind, p0, p1, p2, p3, radius, fill)
  IF half > 0.0 THEN
    out = __canvas_strokeSpan(out, width, height, minX, minY, maxX, maxY, kind, p0, p1, p2, p3, radius, half, stroke)
  END IF
  RETURN out
END FUNC"#;

/// The polygon's own loop, over the cached edge array.
///
/// Fill and stroke share one distance evaluation per pixel because the polygon's
/// distance is the expensive term — `O(edges)` — and computing it twice would double
/// the cost of every stroked polygon for nothing.
#[rustfmt::skip]
const DRAW_POLYGON: &str =
r#"FUNC __canvas_drawPolygonGeometry(surface AS List OF Byte, width AS Integer, height AS Integer, offset AS Integer, minX AS Integer, minY AS Integer, maxX AS Integer, maxY AS Integer, half AS Float, fill AS Color, stroke AS Color) AS List OF Byte
  LET edges AS Integer = toInt(__canvas_geoAt(offset, 20))
  IF edges < 2 THEN
    RETURN surface
  END IF
  LET tail AS Integer = offset + __CANVAS_GEO_HEADER
  MUT out AS List OF Byte = surface
  LET firstX AS Integer = __canvas_maxI(minX, 0)
  LET lastX AS Integer = __canvas_minI(maxX, width - 1)
  LET lastY AS Integer = __canvas_minI(maxY, height - 1)
  MUT y AS Integer = __canvas_maxI(minY, 0)
  WHILE y <= lastY
    LET rowBase AS Integer = y * width * 4
    LET py AS Float = toFloat(y) + 0.5
    MUT x AS Integer = firstX
    WHILE x <= lastX
      LET px AS Float = toFloat(x) + 0.5
      LET distance AS Float = __canvas_edgeDistance(tail, edges, px, py)
      LET fillCoverage AS Integer = __canvas_coverage(distance)
      IF fillCoverage > 0 THEN
        out = __canvas_blendPixel(out, rowBase + x * 4, fill.red, fill.green, fill.blue, fill.alpha, fillCoverage)
      END IF
      IF half > 0.0 THEN
        LET strokeCoverage AS Integer = __canvas_coverage(__canvas_absF(distance) - half)
        IF strokeCoverage > 0 THEN
          out = __canvas_blendPixel(out, rowBase + x * 4, stroke.red, stroke.green, stroke.blue, stroke.alpha, strokeCoverage)
        END IF
      END IF
      x = x + 1
    END WHILE
    y = y + 1
  END WHILE
  RETURN out
END FUNC"#;

/// Signed distance to the closed polygon, from the cached edge array.
///
/// Nearest edge for the magnitude, a crossing count for the sign. Each edge was
/// stored as `x0, y0, dx, dy, invLenSq` at generation time, so the inner loop is
/// multiplies and adds — no per-pixel subtraction of endpoints and no per-pixel
/// reciprocal, which is the whole reason the geometry cache exists.
///
/// Making the polygon an SDF like every other shape is what lets it share the fill,
/// the stroke and the antialiasing rather than needing a scanline rasteriser with its
/// own coverage rules — and a scanline filler and an SDF filler would disagree about
/// edge pixels, which is exactly the kind of disagreement an oracle cannot have.
#[rustfmt::skip]
const EDGE_DISTANCE: &str =
r#"FUNC __canvas_edgeDistance(tail AS Integer, edges AS Integer, px AS Float, py AS Float) AS Float
  MUT best AS Float = 1000000.0
  MUT inside AS Boolean = FALSE
  MUT e AS Integer = 0
  WHILE e < edges
    LET base AS Integer = tail + e * 5
    LET ax AS Float = collections::getOr(__CANVAS_GEO_DATA, base, 0.0)
    LET ay AS Float = collections::getOr(__CANVAS_GEO_DATA, base + 1, 0.0)
    LET dx AS Float = collections::getOr(__CANVAS_GEO_DATA, base + 2, 0.0)
    LET dy AS Float = collections::getOr(__CANVAS_GEO_DATA, base + 3, 0.0)
    LET invLenSq AS Float = collections::getOr(__CANVAS_GEO_DATA, base + 4, 0.0)
    LET wx AS Float = px - ax
    LET wy AS Float = py - ay
    LET t AS Float = __canvas_minF(__canvas_maxF((wx * dx + wy * dy) * invLenSq, 0.0), 1.0)
    LET qx AS Float = wx - t * dx
    LET qy AS Float = wy - t * dy
    best = __canvas_minF(best, math::sqrt(qx * qx + qy * qy))
    LET by AS Float = ay + dy
    IF (ay > py) <> (by > py) THEN
      LET u AS Float = (py - ay) / dy
      IF px < ax + u * dx THEN
        inside = NOT inside
      END IF
    END IF
    e = e + 1
  END WHILE
  IF inside THEN
    RETURN 0.0 - best
  END IF
  RETURN best
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("canvas_strokeHalf", STROKE_HALF));
    pkg.add_helper(RegistryHelper::always("canvas_geoRead", GEO_READ));
    pkg.add_helper(RegistryHelper::always("canvas_edgeDistance", EDGE_DISTANCE));
    pkg.add_helper(RegistryHelper::always(
        "canvas_drawPolygonGeometry",
        DRAW_POLYGON,
    ));
    pkg.add_helper(RegistryHelper::always("canvas_drawGeometry", DRAW_GEOMETRY));
}
