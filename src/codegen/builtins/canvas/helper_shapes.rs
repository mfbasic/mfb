//! Shape rasterisation: one signed-distance loop, five primitives.
//!
//! Rectangle (square or rounded), Circle, Arc and Line are all drawn by the *same* routine
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
//! No transcendental appears anywhere here. The arc's sweep in particular avoids
//! `atan2`: it is two signed half-plane distances through the end radii, cross products
//! against the sweep's direction vectors (`__canvas_geoDistance`'s arc arm, bug-1088).

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
/// rectangle's centre, then measure to the corner. A rounded rectangle is the same function
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

/// The same segment, cut square at each endpoint instead of capped with a disc.
///
/// A **sibling** rather than a parameter on `__canvas_segmentDistance`, because that
/// function is also the polygon edge walk's inner loop (`helper_draw.rs`), where a cap
/// is meaningless — threading a flag through it would put a per-edge argument and a
/// per-edge branch on the hottest path in the rasteriser to serve a primitive that
/// never calls it.
///
/// A butt-capped stroke is the round-capped **band** intersected with the slab between
/// the two end planes, and the SDF of an intersection of convex sets is the `max` of
/// their SDFs. So this returns the finished band distance and takes `half` as an
/// argument, rather than returning a distance the caller then subtracts `half` from:
///
/// ```text
/// d = max( d_round - half , -t*|v| , (t-1)*|v| )
/// ```
///
/// where `t` is the **unclamped** projection. Past the start, `-t*|v|` is the positive
/// distance back to the start plane; past the end, `(t-1)*|v|` is the distance forward
/// from the end plane; between them both are negative and the band wins.
///
/// **Subtracting `half` afterwards instead is the bug this was written with**, and it
/// is worth stating because it looks right and is nearly right. `max(d_round, plane)`
/// then has `half` taken off the whole thing, which compares the plane distance against
/// the *stroke's half-width* rather than against zero — so the cap does not cut until a
/// pixel is more than `half` past the endpoint. Measured on a 20 px line ending at
/// `x = 400`: the pixel at 405 stayed painted, because `max(5.52, 5.5) - 10 < 0`. The
/// end plane has to be `max`'d against a distance that is already zero *at the edge of
/// the band*.
///
/// Both terms stay true signed distances, so the result composes with
/// `clamp(0.5 - d, 0, 1)` like any other edge and a butt end is **antialiased**, not
/// stair-stepped.
///
/// A zero-length butt segment is empty: with `len2` at 0 there is no direction for the
/// planes to be perpendicular to, so it answers "far outside" rather than dividing by
/// zero. That is the deliberate difference from a zero-length *round* segment, which is
/// a dot — both are asserted, because "the degenerate case does something sensible" is
/// exactly what a `max` of three terms can get wrong.
#[rustfmt::skip]
const SEGMENT_DISTANCE_BUTT: &str =
r#"FUNC __canvas_segmentDistanceButt(px AS Float, py AS Float, ax AS Float, ay AS Float, bx AS Float, by AS Float, half AS Float) AS Float
  LET vx AS Float = bx - ax
  LET vy AS Float = by - ay
  LET wx AS Float = px - ax
  LET wy AS Float = py - ay
  LET len2 AS Float = vx * vx + vy * vy
  IF len2 <= 0.0 THEN
    RETURN 1000000.0
  END IF
  LET length AS Float = math::sqrt(len2)
  LET t AS Float = (wx * vx + wy * vy) / len2
  LET clamped AS Float = __canvas_minF(__canvas_maxF(t, 0.0), 1.0)
  LET dx AS Float = wx - vx * clamped
  LET dy AS Float = wy - vy * clamped
  MUT d AS Float = math::sqrt(dx * dx + dy * dy) - half
  d = __canvas_maxF(d, 0.0 - t * length)
  d = __canvas_maxF(d, (t - 1.0) * length)
  RETURN d
END FUNC"#;

/// The same segment with **square** ends (plan-163-B): the stroke continues `half` past
/// each endpoint and is cut square there, so the stroke is a rectangle `half` wider than
/// the segment on every side.
///
/// ```text
/// d = max( |perp| - half , -t*|v| - half , (t-1)*|v| - half )
/// ```
///
/// with `perp` the distance to the segment's LINE and `t` the unclamped projection. It
/// is **not** the butt planes moved out by `half` over the round band: that band is
/// round past the endpoint, so it would round the square's corners. Each term is a true
/// signed distance to one of the rectangle's sides, so the edges antialias like a butt
/// end's; the corners are the `max` of two sides, the butt cap's rule.
///
/// A zero-length segment has no direction, and SVG 2 draws a zero-length square cap as
/// an axis-aligned square of side `2 * half` centred on the point — `max(|wx|, |wy|) -
/// half`.
#[rustfmt::skip]
const SEGMENT_DISTANCE_SQUARE: &str =
r#"FUNC __canvas_segmentDistanceSquare(px AS Float, py AS Float, ax AS Float, ay AS Float, bx AS Float, by AS Float, half AS Float) AS Float
  LET vx AS Float = bx - ax
  LET vy AS Float = by - ay
  LET wx AS Float = px - ax
  LET wy AS Float = py - ay
  LET len2 AS Float = vx * vx + vy * vy
  IF len2 <= 0.0 THEN
    RETURN __canvas_maxF(__canvas_absF(wx), __canvas_absF(wy)) - half
  END IF
  LET length AS Float = math::sqrt(len2)
  LET t AS Float = (wx * vx + wy * vy) / len2
  MUT d AS Float = __canvas_absF(wx * vy - wy * vx) / length - half
  d = __canvas_maxF(d, 0.0 - t * length - half)
  d = __canvas_maxF(d, (t - 1.0) * length - half)
  RETURN d
END FUNC

' plan-163-B: an arc's square cap at one sweep endpoint `(ex, ey)` of the arc centred on
' `(cx, cy)` -- the `half` x `2 * half` box past the end, along the arc's tangent there:
' `max(|across| - half, along - half, -along)`, `along` measured outward from the end and
' `across` along the radius. The radial direction is the endpoint less the centre,
' normalised; `isEnd` turns the outward tangent from the start's (against the sweep) to
' the end's (with it). A zero radial vector has no direction and adds nothing.
FUNC __canvas_arcSquareCap(px AS Float, py AS Float, ex AS Float, ey AS Float, cx AS Float, cy AS Float, half AS Float, isEnd AS Boolean) AS Float
  LET rx AS Float = ex - cx
  LET ry AS Float = ey - cy
  LET rl AS Float = math::sqrt(rx * rx + ry * ry)
  IF rl <= 0.0 THEN
    RETURN 1000000.0
  END IF
  LET ux AS Float = rx / rl
  LET uy AS Float = ry / rl
  LET ox AS Float = px - ex
  LET oy AS Float = py - ey
  MUT along AS Float = ox * uy - oy * ux
  IF isEnd THEN
    along = oy * ux - ox * uy
  END IF
  LET across AS Float = ox * ux + oy * uy
  RETURN __canvas_maxF(__canvas_maxF(__canvas_absF(across) - half, along - half), 0.0 - along)
END FUNC"#;

/// Signed distance to a rotated ellipse — the `Ellipse` primitive's distance function
/// (plan-116-E).
///
/// An ellipse has no signed distance in closed form under this renderer's arithmetic
/// rule: the exact solutions need a cube root or trigonometry, and `06_canvas.md`
/// §"Rendering conventions" constrains the software path to `+ - * /` and `sqrt` so
/// that the same scene renders to the same bytes on every target. So the nearest point
/// is *solved for*, at a **fixed** iteration count.
///
/// Fixed rather than convergence-tested, and that is not a performance choice: a
/// `WHILE |Δ| > ε` loop makes the number of steps depend on the input, so the software
/// rasteriser, Metal and Vulkan would take different numbers of steps on the same pixel
/// on different hardware and the oracle would stop being predictive of the other two.
///
/// **The method is bisection, not Newton, and that was measured.** plan-116-E §4.2
/// originally specified a fixed-count Newton iteration on the unit pair; Phase 1
/// measured it at a worst-case **127.5 coverage steps** of 1/255 at *every* count from
/// 1 to 8, with **411 of 1608** probe points converging to a stationary point that is
/// not the nearest one. The seed is not the problem — after the `|q|` fold it is in the
/// first quadrant by construction — the step is: outside the evolute of an eccentric
/// ellipse the squared distance has three stationary points in the quadrant, and Newton
/// has no preference among them. Re-run the measurement with
/// `cargo test --release --test rt_canvas_rasteriser measure_the_ellipse -- --ignored`.
///
/// Bisection's bracket is guaranteed by construction instead of by a property of the
/// input: after the fold, `g(1,0) = qy·ry ≥ 0` and `g(0,1) = −qx·rx ≤ 0`, where
/// `g` is the derivative of the squared distance along the curve. Each halving is the
/// angular midpoint of two unit vectors — their sum, normalised — so no trigonometry
/// appears here either.
///
/// **24 halvings**, measured: 16 gives 5.5 coverage steps at `rx = 900` and 24 gives
/// **0.0215**. The error scales with the radius, so the count was chosen against radii
/// up to 900 (a canvas is 900 px wide) rather than the 300 the plan first sampled.
///
/// `rx == ry` short-circuits to the exact circle distance. A fixed-count solve is never
/// algebraically exact, and a last-bit residual can flip the coverage quantisation on
/// whichever edge pixel lands nearest a 1/255 step — so an `Ellipse` with equal radii is
/// made byte-identical to the `Circle` of that radius *by construction* rather than by
/// convergence. The handover is measured clean: at `ry = rx` the two arms agree to
/// 0.0001/0.0072/0.0215 steps at `rx` = 5/300/900, and just off it the difference is
/// linear in `|ry − rx|` — the shapes' own difference going to zero, not a jump.
#[rustfmt::skip]
const ELLIPSE_DISTANCE: &str =
r#"FUNC __canvas_ellipseDistance(px AS Float, py AS Float, cx AS Float, cy AS Float, rx AS Float, ry AS Float, ca AS Float, sa AS Float) AS Float
  ' Into the ellipse's own frame: rotate by -angle about the centre.
  LET dx AS Float = px - cx
  LET dy AS Float = py - cy
  LET rxp AS Float = dx * ca + dy * sa
  LET ryp AS Float = 0.0 - dx * sa + dy * ca
  ' Fold to the first quadrant -- the ellipse is symmetric in both axes, so one
  ' quadrant is the whole problem and the bracket below is only valid there.
  LET qx AS Float = __canvas_absF(rxp)
  LET qy AS Float = __canvas_absF(ryp)
  ' Equal radii: the exact circle distance, so an Ellipse with rx = ry is byte-identical
  ' to the Circle of that radius rather than merely close to it.
  IF rx = ry THEN
    RETURN math::sqrt(qx * qx + qy * qy) - rx
  END IF
  MUT c0 AS Float = 1.0
  MUT s0 AS Float = 0.0
  MUT c1 AS Float = 0.0
  MUT s1 AS Float = 1.0
  MUT cm AS Float = 1.0
  MUT sm AS Float = 0.0
  MUT k AS Integer = 0
  WHILE k < 24
    LET cs AS Float = c0 + c1
    LET ss AS Float = s0 + s1
    LET nn AS Float = math::sqrt(cs * cs + ss * ss)
    cm = cs / nn
    sm = ss / nn
    ' g is the derivative of the squared distance along the curve. Positive means the
    ' nearest point is further round; the bracket keeps a sign change between the ends.
    LET g AS Float = (qx - rx * cm) * (0.0 - rx * sm) + (qy - ry * sm) * (ry * cm)
    IF g > 0.0 THEN
      c0 = cm
      s0 = sm
    ELSE
      c1 = cm
      s1 = sm
    END IF
    k = k + 1
  END WHILE
  LET ex AS Float = qx - rx * cm
  LET ey AS Float = qy - ry * sm
  LET d AS Float = math::sqrt(ex * ex + ey * ey)
  ' The sign needs no iteration: one comparison on the implicit form.
  LET ux AS Float = qx / rx
  LET uy AS Float = qy / ry
  IF ux * ux + uy * uy < 1.0 THEN
    RETURN 0.0 - d
  END IF
  RETURN d
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
/// the sweep's cross-product test excluded the last sliver of the arc
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

/// plan-172-E: a rectangle's corners.
///
/// One radius value has two meanings, chosen by its size (`planning/new_canvas.md`):
///
/// * `0.0` or less — a square corner;
/// * above `0.0` through `1.0` — a fraction of the half-size, so the corner is an
///   ellipse quadrant `r * width / 2` across and `r * height / 2` down;
/// * above `1.0` — pixels, a circular corner no larger than half the shorter side.
///
/// The jump at 1 is intended: `1.0` is the full pill and `1.01` a one-pixel corner.
///
/// `__canvas_uniformRadius` answers the question the builder asks: do all four corners
/// resolve to ONE circular radius? If so the rectangle is the exact rounded-rectangle
/// record (a box distance less that radius, on all three renderers) and the answer is
/// the radius — `0.0` for four square corners. If not it answers `-1.0` and the outline
/// is flattened to a ring (`__canvas_rrectRing`).
///
/// For four EQUAL pixel radii the answer is the radius as given, not clamped: the
/// header builder clamps it to half the shorter side, exactly as it clamped the one
/// radius a rounded rectangle had before plan-172-E, and the native builder does the
/// same arithmetic on the same value. Clamping here as well would be harmless, and one clamp is one place.
#[rustfmt::skip]
const CORNERS: &str =
r#"FUNC __canvas_cornerRadii(w AS Float, h AS Float, r AS Float) AS vector::Float2
  IF r <= 0.0 THEN
    RETURN vector::Float2[x := 0.0, y := 0.0]
  END IF
  IF r <= 1.0 THEN
    RETURN vector::Float2[x := r * w / 2.0, y := r * h / 2.0]
  END IF
  LET c AS Float = __canvas_minF(r, __canvas_minF(w / 2.0, h / 2.0))
  RETURN vector::Float2[x := c, y := c]
END FUNC

FUNC __canvas_uniformRadius(w AS Float, h AS Float, tl AS Float, tr AS Float, bl AS Float, br AS Float) AS Float
  IF tl = tr AND tl = bl AND tl = br THEN
    IF tl <= 0.0 THEN
      RETURN 0.0
    END IF
    IF tl > 1.0 THEN
      RETURN tl
    END IF
  END IF
  LET a AS vector::Float2 = __canvas_cornerRadii(w, h, tl)
  LET b AS vector::Float2 = __canvas_cornerRadii(w, h, tr)
  LET c AS vector::Float2 = __canvas_cornerRadii(w, h, bl)
  LET d AS vector::Float2 = __canvas_cornerRadii(w, h, br)
  IF a.x = a.y AND b.x = a.x AND b.y = a.x AND c.x = a.x AND c.y = a.x AND d.x = a.x AND d.y = a.x THEN
    RETURN a.x
  END IF
  RETURN 0.0 - 1.0
END FUNC

' Where the bind point sits relative to the rectangle's top-left corner, negated: the
' local x (or y) of the top-left corner. `TopLeft` is the local origin; `Center` puts
' the origin at the middle, so the corner is half the extent back.
FUNC __canvas_bindOffset(bind AS BindTo, extent AS Float) AS Float
  IF bind = BindTo.Center THEN
    RETURN 0.0 - extent / 2.0
  END IF
  RETURN 0.0
END FUNC"#;

/// plan-172-E: a rectangle's outline as one clockwise ring, for the corners that are
/// not one circular radius.
///
/// The ring starts on the top edge where it leaves the top-left corner — where a
/// rounded rectangle's dash pattern has always started — and goes clockwise: top edge,
/// top-right corner, right edge, bottom-right, bottom edge, bottom-left, left edge,
/// top-left corner. A square corner is its one point. A rounded corner is an ellipse
/// quadrant flattened to segments: the count is the smallest for which a chord of a
/// circle of the corner's LARGER radius stays within 0.05 px of the arc, clamped to
/// 4..32. A chord subtending `t` radians of a circle of radius `R` lies at most
/// `R * (1 - cos(t / 2))` inside it, which is at most `R * t * t / 8`; so `t =
/// sqrt(0.4 / R)` keeps it within 0.05. No inverse cosine is needed, which matters:
/// this package computes its own sine and cosine so that every platform flattens to
/// the same points. Past a radius of about 166 px the 32-segment cap is what binds and
/// the error grows past 0.05 px (0.3 px at 1,000) — the same bound a circle of that
/// size would need 80 segments to keep.
///
/// Corners are resolved against each other first: two corners sharing a side whose
/// radii along it sum to more than the side would overlap, so both are scaled back by
/// the same factor, as CSS does.
#[rustfmt::skip]
const RRECT_RING: &str =
r#"FUNC __canvas_cornerSegments(rx AS Float, ry AS Float) AS Integer
  LET big AS Float = __canvas_maxF(rx, ry)
  IF big <= 0.0 THEN
    RETURN 0
  END IF
  LET turn AS Float = math::sqrt(0.4 / big)
  MUT n AS Integer = toInt(1.5707963267948966 / turn) + 1
  IF n < 4 THEN
    n = 4
  END IF
  IF n > 32 THEN
    n = 32
  END IF
  RETURN n
END FUNC

' The points of one corner, appended to `ring`. (cx, cy) is the corner ellipse's
' centre; the quadrant runs clockwise from `startAngle` (radians, clockwise from +X)
' for a quarter turn. The first point is included and the last is not: the next piece
' of the outline supplies it, so no point is repeated.
FUNC __canvas_cornerPoints(ring AS List OF vector::Float2, cx AS Float, cy AS Float, rx AS Float, ry AS Float, startAngle AS Float, cornerX AS Float, cornerY AS Float) AS List OF vector::Float2
  LET n AS Integer = __canvas_cornerSegments(rx, ry)
  IF n = 0 OR rx <= 0.0 OR ry <= 0.0 THEN
    RETURN collections::append(ring, vector::Float2[x := cornerX, y := cornerY])
  END IF
  MUT out AS List OF vector::Float2 = ring
  MUT k AS Integer = 0
  WHILE k < n
    LET a AS Float = startAngle + 1.5707963267948966 * toFloat(k) / toFloat(n)
    out = collections::append(out, vector::Float2[x := cx + rx * __canvas_cos(a), y := cy + ry * __canvas_sin(a)])
    k = k + 1
  END WHILE
  RETURN out
END FUNC

' `x0`, `y0` is the top-left corner in the record's own coordinates.
FUNC __canvas_rrectRing(x0 AS Float, y0 AS Float, w AS Float, h AS Float, tl AS Float, tr AS Float, bl AS Float, br AS Float) AS List OF vector::Float2
  MUT a AS vector::Float2 = __canvas_cornerRadii(w, h, tl)
  MUT b AS vector::Float2 = __canvas_cornerRadii(w, h, tr)
  MUT c AS vector::Float2 = __canvas_cornerRadii(w, h, br)
  MUT d AS vector::Float2 = __canvas_cornerRadii(w, h, bl)
  ' Two corners on one side may not together be longer than the side.
  MUT f AS Float = 1.0
  IF a.x + b.x > w THEN
    f = __canvas_minF(f, w / (a.x + b.x))
  END IF
  IF d.x + c.x > w THEN
    f = __canvas_minF(f, w / (d.x + c.x))
  END IF
  IF a.y + d.y > h THEN
    f = __canvas_minF(f, h / (a.y + d.y))
  END IF
  IF b.y + c.y > h THEN
    f = __canvas_minF(f, h / (b.y + c.y))
  END IF
  IF f < 1.0 THEN
    a = vector::Float2[x := a.x * f, y := a.y * f]
    b = vector::Float2[x := b.x * f, y := b.y * f]
    c = vector::Float2[x := c.x * f, y := c.y * f]
    d = vector::Float2[x := d.x * f, y := d.y * f]
  END IF
  LET x1 AS Float = x0 + w
  LET y1 AS Float = y0 + h
  ' The start: the top edge where it leaves the top-left corner -- the corner itself
  ' when that corner is square.
  MUT ring AS List OF vector::Float2 = [vector::Float2[x := x0 + a.x, y := y0]]
  ' Top-right: from the top edge (angle -90 deg) round to the right edge (0 deg).
  ring = __canvas_cornerPoints(ring, x1 - b.x, y0 + b.y, b.x, b.y, 0.0 - 1.5707963267948966, x1, y0)
  ' Bottom-right: from the right edge (0 deg) to the bottom edge (90 deg).
  IF b.x > 0.0 AND b.y > 0.0 THEN
    ring = collections::append(ring, vector::Float2[x := x1, y := y0 + b.y])
  END IF
  ring = __canvas_cornerPoints(ring, x1 - c.x, y1 - c.y, c.x, c.y, 0.0, x1, y1)
  ' Bottom-left: from the bottom edge (90 deg) to the left edge (180 deg).
  IF c.x > 0.0 AND c.y > 0.0 THEN
    ring = collections::append(ring, vector::Float2[x := x1 - c.x, y := y1])
  END IF
  ring = __canvas_cornerPoints(ring, x0 + d.x, y1 - d.y, d.x, d.y, 1.5707963267948966, x0, y1)
  ' Top-left: from the left edge (180 deg) back to the top edge (270 deg).
  IF d.x > 0.0 AND d.y > 0.0 THEN
    ring = collections::append(ring, vector::Float2[x := x0, y := y1 - d.y])
  END IF
  ' A square top-left corner is the start point already; a rounded one ends just short
  ' of it.
  IF a.x > 0.0 AND a.y > 0.0 THEN
    ring = __canvas_cornerPoints(ring, x0 + a.x, y0 + a.y, a.x, a.y, 3.141592653589793, x0, y0)
  END IF
  RETURN ring
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("canvas_cornerRadii", CORNERS));
    pkg.add_helper(RegistryHelper::always("canvas_rrectRing", RRECT_RING));
    pkg.add_helper(RegistryHelper::always("canvas_floatUtil", FLOAT_UTIL));
    pkg.add_helper(RegistryHelper::always("canvas_coverage", COVERAGE));
    pkg.add_helper(RegistryHelper::always("canvas_rectDistance", RECT_DISTANCE));
    pkg.add_helper(RegistryHelper::always(
        "canvas_segmentDistance",
        SEGMENT_DISTANCE,
    ));
    pkg.add_helper(RegistryHelper::always(
        "canvas_segmentDistanceButt",
        SEGMENT_DISTANCE_BUTT,
    ));
    pkg.add_helper(RegistryHelper::always(
        "canvas_ellipseDistance",
        ELLIPSE_DISTANCE,
    ));
    pkg.add_helper(RegistryHelper::always(
        "canvas_segmentDistanceSquare",
        SEGMENT_DISTANCE_SQUARE,
    ));
    pkg.add_helper(RegistryHelper::always("canvas_trig", TRIG));
}
