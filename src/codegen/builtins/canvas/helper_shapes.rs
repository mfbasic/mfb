//! Shape rasterisation: one signed-distance loop, five primitives.
//!
//! Rectangle, RoundedRect, Circle, Arc and Line are all drawn by the *same* routine
//! over a bounding box, differing only in the distance function evaluated per pixel.
//! That is not a shortcut — it is the design the GPU backends will use ("one
//! pipeline, many shapes"), so the software oracle predicting their output means
//! predicting it through the same structure rather than a parallel one.
//!
//! **Antialiasing is exact coverage, not a smoothstep.** `clamp(0.5 - d, 0, 1)` on
//! the signed distance is the fraction of a pixel inside the shape for a locally
//! straight edge, and it is computed with `+ - * /` and `sqrt` only — all exactly
//! specified by IEEE-754. A `smoothstep`/`fwidth` formulation would have been closer
//! to the usual shader idiom and is what the GPU will do, but it is an approximation
//! whose result depends on the derivative estimate, and the oracle has to be the
//! exact answer for the GPU to be compared *against*.
//!
//! No transcendental appears anywhere here. The arc's angular test in particular
//! avoids `atan2` — see `__canvas_arcInSweep`.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

/// Pixel coverage from a signed distance, as `0..255`.
///
/// Negative distance is inside. Sampling at the pixel *centre* means the half-open
/// band `-0.5..0.5` is the antialiased edge; outside it the pixel is wholly in or
/// wholly out, which is what keeps large flat areas free of rounding noise.
#[rustfmt::skip]
const COVERAGE: &str =
r#"FUNC __canvas_coverage(distance AS Float) AS Integer
  LET inside AS Float = 0.5 - distance
  IF inside <= 0.0 THEN
    RETURN 0
  END IF
  IF inside >= 1.0 THEN
    RETURN 255
  END IF
  RETURN toInt(inside * 255.0 + 0.5)
END FUNC"#;

/// Signed distance from a point to an axis-aligned rectangle, negative inside.
///
/// The standard formulation: fold the point into the first quadrant relative to the
/// rectangle's centre, then measure to the corner. `RoundedRect` is the same function
/// with the radius subtracted, which is why there is no separate rounded-rect
/// distance.
#[rustfmt::skip]
const RECT_DISTANCE: &str =
r#"FUNC __canvas_rectDistance(px AS Float, py AS Float, cx AS Float, cy AS Float, hw AS Float, hh AS Float) AS Float
  LET dx AS Float = __canvas_absF(px - cx) - hw
  LET dy AS Float = __canvas_absF(py - cy) - hh
  LET ox AS Float = __canvas_maxF(dx, 0.0)
  LET oy AS Float = __canvas_maxF(dy, 0.0)
  LET outside AS Float = math::sqrt(ox * ox + oy * oy)
  LET inside AS Float = __canvas_minF(__canvas_maxF(dx, dy), 0.0)
  RETURN outside + inside
END FUNC"#;

/// `|x|`, `max`, `min` on `Float`.
///
/// Spelled out rather than reached for in `math::` so the whole rasteriser reads
/// against one obvious definition, and so a future `math::` change to NaN handling
/// cannot silently move the oracle's output.
#[rustfmt::skip]
const FLOAT_UTIL: &str =
r#"FUNC __canvas_absF(v AS Float) AS Float
  IF v < 0.0 THEN
    RETURN 0.0 - v
  END IF
  RETURN v
END FUNC

FUNC __canvas_maxF(a AS Float, b AS Float) AS Float
  IF a > b THEN
    RETURN a
  END IF
  RETURN b
END FUNC

FUNC __canvas_minF(a AS Float, b AS Float) AS Float
  IF a < b THEN
    RETURN a
  END IF
  RETURN b
END FUNC"#;

/// Distance from a point to a line segment — the `Line` primitive's distance
/// function, and the one a stroke width is subtracted from.
///
/// Projecting onto the segment and clamping `t` to `0..1` is what gives the round
/// caps; it also makes a zero-length segment degrade to a dot rather than dividing
/// by zero, because the clamp happens before the distance is taken.
#[rustfmt::skip]
const SEGMENT_DISTANCE: &str =
r#"FUNC __canvas_segmentDistance(px AS Float, py AS Float, ax AS Float, ay AS Float, bx AS Float, by AS Float) AS Float
  LET vx AS Float = bx - ax
  LET vy AS Float = by - ay
  LET wx AS Float = px - ax
  LET wy AS Float = py - ay
  LET len2 AS Float = vx * vx + vy * vy
  MUT t AS Float = 0.0
  IF len2 > 0.0 THEN
    t = (wx * vx + wy * vy) / len2
    t = __canvas_minF(__canvas_maxF(t, 0.0), 1.0)
  END IF
  LET dx AS Float = wx - vx * t
  LET dy AS Float = wy - vy * t
  RETURN math::sqrt(dx * dx + dy * dy)
END FUNC"#;

/// Deterministic `sin`/`cos`, used to turn an arc's start and end **angles** into
/// direction vectors.
///
/// `math::sin`/`cos` would be the obvious call and cannot be used: libm's
/// trigonometric functions are not correctly rounded, so their last bit differs
/// between platforms, and a sub-ULP difference in an arc endpoint moves an
/// antialiased edge byte — which is exactly what the oracle must not do.
///
/// Range-reduce to `-PI..PI`, **then fold into `-PI/2..PI/2`** using
/// `sin(PI - x) = sin(x)` and `cos(PI - x) = -cos(x)`, and evaluate the Taylor
/// series there. Every operation is a multiply, divide or add — all exactly
/// specified by IEEE-754 — so the result stays bit-identical everywhere.
///
/// **The fold is not an optimization; without it these were wrong at the ends.** A
/// Taylor series about zero has its error concentrated at the far end of its
/// interval, and this file previously claimed the ninth-degree truncation error was
/// "below 1e-8" over `-PI..PI`. That is the error near *zero*. Measured at the other
/// end, `x = 3.14159`, the old series gave `sin = 6.93e-3` where the true value is
/// `2.65e-6`, and `cos = -0.976` where it is `-1.0`.
///
/// The consequence was visible, not theoretical: an `Arc` swept to `endAngle = PI`
/// had its end direction vector off by ~1.4 degrees, so
/// `__canvas_arcInSweep`'s cross-product test excluded the last sliver of the arc
/// and the stroke stopped ~0.6 px short of where it was asked to. It surfaced when
/// the Metal backend — using the hardware `sin`/`cos` — drew 14 pixels of a smile's
/// end cap that this path did not (plan-98-E Phase 2).
///
/// Folded, and with one more term each, the worst error over the whole circle is
/// `5.6e-8` for `sin` and `4.6e-7` for `cos` — under `1e-4` of a pixel at radius
/// 150, against `2.4e-2` before.
#[rustfmt::skip]
const TRIG: &str =
r#"FUNC __canvas_wrapAngle(angle AS Float) AS Float
  LET twoPi AS Float = 6.283185307179586
  MUT a AS Float = angle
  WHILE a > 3.141592653589793
    a = a - twoPi
  END WHILE
  WHILE a < 0.0 - 3.141592653589793
    a = a + twoPi
  END WHILE
  RETURN a
END FUNC

FUNC __canvas_foldQuadrant(angle AS Float) AS Float
  LET x AS Float = __canvas_wrapAngle(angle)
  IF x > 1.5707963267948966 THEN
    RETURN 3.141592653589793 - x
  END IF
  IF x < 0.0 - 1.5707963267948966 THEN
    RETURN 0.0 - 3.141592653589793 - x
  END IF
  RETURN x
END FUNC

FUNC __canvas_sin(angle AS Float) AS Float
  LET x AS Float = __canvas_foldQuadrant(angle)
  LET x2 AS Float = x * x
  LET x3 AS Float = x2 * x
  LET x5 AS Float = x3 * x2
  LET x7 AS Float = x5 * x2
  LET x9 AS Float = x7 * x2
  LET x11 AS Float = x9 * x2
  RETURN x - x3 / 6.0 + x5 / 120.0 - x7 / 5040.0 + x9 / 362880.0 - x11 / 39916800.0
END FUNC

FUNC __canvas_cos(angle AS Float) AS Float
  LET wrapped AS Float = __canvas_wrapAngle(angle)
  LET x AS Float = __canvas_foldQuadrant(angle)
  LET x2 AS Float = x * x
  LET x4 AS Float = x2 * x2
  LET x6 AS Float = x4 * x2
  LET x8 AS Float = x6 * x2
  LET x10 AS Float = x8 * x2
  LET magnitude AS Float = 1.0 - x2 / 2.0 + x4 / 24.0 - x6 / 720.0 + x8 / 40320.0 - x10 / 3628800.0
  IF __canvas_absF(wrapped) > 1.5707963267948966 THEN
    RETURN 0.0 - magnitude
  END IF
  RETURN magnitude
END FUNC"#;

/// Whether a point's direction from the arc centre lies within the swept sector.
///
/// **Deliberately not `atan2`.** That is the obvious way to write it and it would
/// make the oracle platform-dependent for the same reason as `sin`/`cos` above.
/// Instead the test is two 2D cross products against the sector's start and end
/// direction vectors — pure multiply-and-compare, exact everywhere.
///
/// The two cases are what makes it correct for a sector wider than a half-turn:
/// for `sweep <= PI` the sector is the *intersection* of the two half-planes, and for
/// a reflex sector it is their *union*. Writing only the first — the natural thing —
/// silently clips every arc longer than 180 degrees.
///
/// `+Y` is downward and angles run clockwise from `+X` (plan-98-api.md), which is
/// why "left of the start direction" is `cross >= 0` here rather than `<= 0`.
#[rustfmt::skip]
const ARC_SWEEP: &str =
r#"FUNC __canvas_arcInSweep(dx AS Float, dy AS Float, sx AS Float, sy AS Float, ex AS Float, ey AS Float, reflex AS Boolean) AS Boolean
  LET afterStart AS Boolean = sx * dy - sy * dx >= 0.0
  LET beforeEnd AS Boolean = ex * dy - ey * dx <= 0.0
  IF reflex THEN
    RETURN afterStart OR beforeEnd
  END IF
  RETURN afterStart AND beforeEnd
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("canvas_floatUtil", FLOAT_UTIL));
    pkg.add_helper(RegistryHelper::always("canvas_coverage", COVERAGE));
    pkg.add_helper(RegistryHelper::always("canvas_rectDistance", RECT_DISTANCE));
    pkg.add_helper(RegistryHelper::always(
        "canvas_segmentDistance",
        SEGMENT_DISTANCE,
    ));
    pkg.add_helper(RegistryHelper::always("canvas_trig", TRIG));
    pkg.add_helper(RegistryHelper::always("canvas_arcSweep", ARC_SWEEP));
}
