//! Per-item drawing: the bounding-box loops, and one `MATCH` arm per `DrawItem`.
//!
//! Rectangle, RoundedRect, Circle and Line all go through `__canvas_fillSpan`, which
//! walks a clipped bounding box and asks a per-kind distance function for each
//! pixel's coverage. They differ only in that function, in the box they claim, and in
//! the radius subtracted from the distance — which is what turns a rectangle into a
//! rounded one and a segment into a stroked line, rather than three more code paths.
//!
//! The arc has its own loop. It is the one shape needing an angular test as well as a
//! radial one, and squeezing that into the shared signature costs more clarity than
//! the sharing buys.
//!
//! **Fill then stroke, in that order**, matching every 2D API a user is likely to
//! have met: an outline drawn under its own fill would be half-hidden by it.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

/// The kind tag `__canvas_fillSpan` switches its distance function on.
///
/// Small integers rather than an enum because these are internal: a `canvas` enum
/// would appear in `mfb man canvas types` as a type no program can use.
#[rustfmt::skip]
const KINDS: &str =
r#"LET __CANVAS_KIND_RECT AS Integer = 0
LET __CANVAS_KIND_CIRCLE AS Integer = 1
LET __CANVAS_KIND_SEGMENT AS Integer = 2"#;

/// Integer `min`/`max`, for clipping a bounding box to the surface.
#[rustfmt::skip]
const INT_UTIL: &str =
r#"FUNC __canvas_maxI(a AS Integer, b AS Integer) AS Integer
  IF a > b THEN
    RETURN a
  END IF
  RETURN b
END FUNC

FUNC __canvas_minI(a AS Integer, b AS Integer) AS Integer
  IF a < b THEN
    RETURN a
  END IF
  RETURN b
END FUNC"#;

/// Walk the clipped bounding box, evaluating one distance function per pixel.
///
/// `p0..p3` carry the shape's parameters positionally — centre and half-extent for a
/// rectangle, centre and radius for a circle, two endpoints for a segment. A record
/// per shape kind would allocate on the per-scene path for no gain.
///
/// `radius` is subtracted from the raw distance. That single term is what makes a
/// rounded rectangle and a stroked line fall out of the rectangle and segment
/// distances rather than needing their own rasterisers.
///
/// The box is clamped to the surface **before** the loop rather than tested per
/// pixel, so a shape hanging off the edge costs nothing for the invisible part.
#[rustfmt::skip]
const FILL_SPAN: &str =
r#"FUNC __canvas_fillSpan(surface AS List OF Byte, width AS Integer, height AS Integer, minX AS Integer, minY AS Integer, maxX AS Integer, maxY AS Integer, kind AS Integer, p0 AS Float, p1 AS Float, p2 AS Float, p3 AS Float, radius AS Float, color AS Color) AS List OF Byte
  MUT out AS List OF Byte = surface
  IF toInt(color.alpha) <= 0 THEN
    RETURN out
  END IF
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
      LET distance AS Float = __canvas_shapeDistance(kind, px, py, p0, p1, p2, p3) - radius
      LET coverage AS Integer = __canvas_coverage(distance)
      IF coverage > 0 THEN
        out = __canvas_blendPixel(out, rowBase + x * 4, color.red, color.green, color.blue, color.alpha, coverage)
      END IF
      x = x + 1
    END WHILE
    y = y + 1
  END WHILE
  RETURN out
END FUNC"#;

/// The distance function for each shared shape kind.
#[rustfmt::skip]
const SHAPE_DISTANCE: &str =
r#"FUNC __canvas_shapeDistance(kind AS Integer, px AS Float, py AS Float, p0 AS Float, p1 AS Float, p2 AS Float, p3 AS Float) AS Float
  IF kind = __CANVAS_KIND_RECT THEN
    RETURN __canvas_rectDistance(px, py, p0, p1, p2, p3)
  END IF
  IF kind = __CANVAS_KIND_CIRCLE THEN
    LET dx AS Float = px - p0
    LET dy AS Float = py - p1
    RETURN math::sqrt(dx * dx + dy * dy) - p2
  END IF
  RETURN __canvas_segmentDistance(px, py, p0, p1, p2, p3)
END FUNC"#;

/// The arc's own loop: a stroked ring clipped to a swept sector.
///
/// The sector test pushes an out-of-sweep pixel's distance far positive rather than
/// branching around the blend, so the arc's two radial ends antialias through exactly
/// the same coverage path as its curved sides. Branching would have left them hard.
///
/// The start and end directions are computed **once per arc** with the deterministic
/// `__canvas_sin`/`__canvas_cos`, never per pixel.
#[rustfmt::skip]
const FILL_ARC: &str =
r#"FUNC __canvas_fillArc(surface AS List OF Byte, width AS Integer, height AS Integer, cx AS Float, cy AS Float, radius AS Float, startAngle AS Float, endAngle AS Float, halfWidth AS Float, color AS Color) AS List OF Byte
  MUT out AS List OF Byte = surface
  IF toInt(color.alpha) <= 0 THEN
    RETURN out
  END IF
  LET sweep AS Float = endAngle - startAngle
  LET reflex AS Boolean = sweep > 3.141592653589793
  LET sx AS Float = __canvas_cos(startAngle)
  LET sy AS Float = __canvas_sin(startAngle)
  LET ex AS Float = __canvas_cos(endAngle)
  LET ey AS Float = __canvas_sin(endAngle)
  LET reach AS Float = radius + halfWidth + 1.0
  LET firstX AS Integer = __canvas_maxI(toInt(cx - reach), 0)
  LET lastX AS Integer = __canvas_minI(toInt(cx + reach), width - 1)
  LET lastY AS Integer = __canvas_minI(toInt(cy + reach), height - 1)
  MUT y AS Integer = __canvas_maxI(toInt(cy - reach), 0)
  WHILE y <= lastY
    LET rowBase AS Integer = y * width * 4
    LET py AS Float = toFloat(y) + 0.5
    MUT x AS Integer = firstX
    WHILE x <= lastX
      LET px AS Float = toFloat(x) + 0.5
      LET dx AS Float = px - cx
      LET dy AS Float = py - cy
      MUT distance AS Float = 1000000.0
      IF __canvas_arcInSweep(dx, dy, sx, sy, ex, ey, reflex) THEN
        distance = __canvas_absF(math::sqrt(dx * dx + dy * dy) - radius) - halfWidth
      END IF
      LET coverage AS Integer = __canvas_coverage(distance)
      IF coverage > 0 THEN
        out = __canvas_blendPixel(out, rowBase + x * 4, color.red, color.green, color.blue, color.alpha, coverage)
      END IF
      x = x + 1
    END WHILE
    y = y + 1
  END WHILE
  RETURN out
END FUNC"#;

/// The outline band of the same shape: `|distance| - halfWidth`.
///
/// An outline is not a separate shape, it is the *absolute* distance offset by half
/// the stroke width — which is why every shape gets a correctly antialiased outline
/// from the distance function it already has, with no per-shape outline geometry.
#[rustfmt::skip]
const STROKE_SPAN: &str =
r#"FUNC __canvas_strokeSpan(surface AS List OF Byte, width AS Integer, height AS Integer, minX AS Integer, minY AS Integer, maxX AS Integer, maxY AS Integer, kind AS Integer, p0 AS Float, p1 AS Float, p2 AS Float, p3 AS Float, radius AS Float, halfWidth AS Float, color AS Color) AS List OF Byte
  MUT out AS List OF Byte = surface
  IF toInt(color.alpha) <= 0 THEN
    RETURN out
  END IF
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
      LET raw AS Float = __canvas_shapeDistance(kind, px, py, p0, p1, p2, p3) - radius
      LET coverage AS Integer = __canvas_coverage(__canvas_absF(raw) - halfWidth)
      IF coverage > 0 THEN
        out = __canvas_blendPixel(out, rowBase + x * 4, color.red, color.green, color.blue, color.alpha, coverage)
      END IF
      x = x + 1
    END WHILE
    y = y + 1
  END WHILE
  RETURN out
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("canvas_kinds", KINDS));
    pkg.add_helper(RegistryHelper::always("canvas_strokeSpan", STROKE_SPAN));
    pkg.add_helper(RegistryHelper::always("canvas_intUtil", INT_UTIL));
    pkg.add_helper(RegistryHelper::always(
        "canvas_shapeDistance",
        SHAPE_DISTANCE,
    ));
    pkg.add_helper(RegistryHelper::always("canvas_fillSpan", FILL_SPAN));
    pkg.add_helper(RegistryHelper::always("canvas_fillArc", FILL_ARC));
}
