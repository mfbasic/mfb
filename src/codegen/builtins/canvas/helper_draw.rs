//! The per-pixel distance functions the one rasterisation loop switches on.
//!
//! Every primitive is a signed distance field. Rectangle, RoundedRect, Circle, Line,
//! Arc and Polygon differ only in the function evaluated per pixel and in the radius
//! subtracted from it — which is what turns a rectangle into a rounded one and a
//! segment into a stroked line, rather than being six code paths.
//!
//! That is not a shortcut, it is the design the GPU backends will use ("one pipeline,
//! many shapes"), so the software oracle predicting their output means predicting it
//! through the same structure rather than a parallel one.
//!
//! The pixel *writes* live in `helper_items.rs`, not here, and deliberately — see the
//! comment on `__canvas_drawGeometry` for why a helper that takes and returns the
//! surface is 290x slower than writing it in place.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

/// The kind tag the distance dispatch switches on.
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

/// The signed distance to one geometry record's shape, negative inside.
///
/// `p0..p3` carry the shape's parameters positionally — centre and half-extent for a
/// rectangle, centre and radius for a circle, two endpoints for a segment. `radius`
/// is subtracted from the raw distance, which is the single term that makes a rounded
/// rectangle and a stroked line fall out of the rectangle and segment distances.
///
/// The arc's sweep vectors are passed in rather than derived here because they are
/// per-*shape* constants: computing `sin`/`cos` per pixel would be both slower and,
/// more importantly, the only place a transcendental could reach the per-pixel path.
///
/// An out-of-sweep arc pixel gets a large positive distance rather than a branch
/// around the blend, so the arc's two radial ends antialias through exactly the same
/// coverage path as its curved sides. Branching would have left them hard.
#[rustfmt::skip]
const GEO_DISTANCE: &str =
r#"FUNC __canvas_geoDistance(kind AS Integer, tail AS Integer, edges AS Integer, px AS Float, py AS Float, p0 AS Float, p1 AS Float, p2 AS Float, p3 AS Float, radius AS Float, sx AS Float, sy AS Float, ex AS Float, ey AS Float, reflex AS Boolean, cap AS Integer, capSX AS Float, capSY AS Float, capEX AS Float, capEY AS Float, ca AS Float, sa AS Float) AS Float
  IF kind = __CANVAS_KIND_RECT THEN
    RETURN __canvas_rectDistance(px, py, p0, p1, p2, p3) - radius
  END IF
  IF kind = __CANVAS_KIND_CIRCLE THEN
    LET cdx AS Float = px - p0
    LET cdy AS Float = py - p1
    RETURN math::sqrt(cdx * cdx + cdy * cdy) - p2 - radius
  END IF
  IF kind = __CANVAS_KIND_SEGMENT THEN
    ' plan-116-D. Round is 1 and is what a Line did before this letter, so the branch
    ' is written round-first: the pre-existing behaviour reads as the straight path
    ' rather than as the exception.
    '
    ' The butt arm returns the finished band distance and so does NOT subtract `radius`
    ' again -- a butt cap is the band intersected with the slab between the end planes,
    ' and that intersection has to be taken against a distance that is already zero at
    ' the band's edge. See `__canvas_segmentDistanceButt` for the measured consequence
    ' of subtracting afterwards instead.
    IF cap = 1 THEN
      RETURN __canvas_segmentDistance(px, py, p0, p1, p2, p3) - radius
    END IF
    RETURN __canvas_segmentDistanceButt(px, py, p0, p1, p2, p3, radius)
  END IF
  IF kind = __CANVAS_GEO_ARC THEN
    LET adx AS Float = px - p0
    LET ady AS Float = py - p1
    MUT arcD AS Float = 1000000.0
    IF __canvas_arcInSweep(adx, ady, sx, sy, ex, ey, reflex) THEN
      arcD = __canvas_absF(math::sqrt(adx * adx + ady * ady) - p2) - radius
    END IF
    ' plan-116-D. Butt is 0 and is what an Arc did before this letter -- the sweep test
    ' already cuts the band along a radius at each end -- so the untouched path is the
    ' one that returns here, and a butt arc is byte-for-byte what it was.
    IF cap = 0 THEN
      RETURN arcD
    END IF
    ' Round unions a disc of the stroke's half-width at each sweep endpoint. A union of
    ' SDFs is their `min`, and the endpoints are per-shape constants the header carries
    ' (`__CANVAS_GEO_CAPSTARTX`..), so this costs two distances per pixel and no
    ' transcendental at all.
    LET s0x AS Float = px - capSX
    LET s0y AS Float = py - capSY
    LET e0x AS Float = px - capEX
    LET e0y AS Float = py - capEY
    arcD = __canvas_minF(arcD, math::sqrt(s0x * s0x + s0y * s0y) - radius)
    RETURN __canvas_minF(arcD, math::sqrt(e0x * e0x + e0y * e0y) - radius)
  END IF
  IF kind = __CANVAS_GEO_ELLIPSE THEN
    ' plan-116-E. `ca`/`sa` are the rotation's cosine and sine, precomputed once per
    ' ellipse in `__canvas_ellipseHeader` -- no trigonometry on this path.
    RETURN __canvas_ellipseDistance(px, py, p0, p1, p2, p3, ca, sa) - radius
  END IF
  RETURN __canvas_edgeDistance(tail, edges, px, py)
END FUNC"#;

/// Signed distance to a closed polygon, from the cached edge array.
///
/// Nearest edge for the magnitude, a crossing count for the sign. Each edge was
/// stored as `x0, y0, dx, dy, invLenSq` at generation time, so the inner loop is
/// multiplies and adds — no per-pixel subtraction of endpoints and no per-pixel
/// reciprocal. That is what the geometry cache buys.
///
/// Making the polygon an SDF like every other shape is what lets it share the fill,
/// the stroke and the antialiasing rather than needing a scanline rasteriser with its
/// own coverage rules — and a scanline filler and an SDF filler would disagree about
/// edge pixels, which is exactly the kind of disagreement an oracle cannot have.
///
/// The cost is `O(edges)` per pixel instead of `O(1)`, which is the right trade for a
/// primitive whose scenes have a handful of vertices.
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
    pkg.add_helper(RegistryHelper::always("canvas_kinds", KINDS));
    pkg.add_helper(RegistryHelper::always("canvas_intUtil", INT_UTIL));
    pkg.add_helper(RegistryHelper::always("canvas_edgeDistance", EDGE_DISTANCE));
    pkg.add_helper(RegistryHelper::always("canvas_geoDistance", GEO_DISTANCE));
}
