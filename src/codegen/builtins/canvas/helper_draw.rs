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
r#"FUNC __canvas_geoDistance(kind AS Integer, tail AS Integer, edges AS Integer, px AS Float, py AS Float, p0 AS Float, p1 AS Float, p2 AS Float, p3 AS Float, radius AS Float, sx AS Float, sy AS Float, ex AS Float, ey AS Float, reflex AS Boolean, cap AS Integer, capSX AS Float, capSY AS Float, capEX AS Float, capEY AS Float, ca AS Float, sa AS Float, dashed AS Boolean) AS Float
  IF kind = __CANVAS_KIND_RECT THEN
    RETURN __canvas_rectDistance(px, py, p0, p1, p2, p3) - radius
  END IF
  IF kind = __CANVAS_KIND_CIRCLE THEN
    LET cdx AS Float = px - p0
    LET cdy AS Float = py - p1
    RETURN math::sqrt(cdx * cdx + cdy * cdy) - p2 - radius
  END IF
  ' plan-163-B: a dashed Line, Arc or PolyLine is its dashed stroke (`helper_dashes.rs`);
  ' `dashed` is read once per item, so an undashed one pays one test.
  IF dashed THEN
    IF kind = __CANVAS_KIND_SEGMENT OR kind = __CANVAS_GEO_ARC OR kind = __CANVAS_GEO_POLYLINE THEN
      RETURN __canvas_dashedStroke(kind, tail - __CANVAS_GEO_HEADER, px, py, radius)
    END IF
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
    ' plan-163-B: Square (2) is the butt cut moved out by the half-width, measured as a
    ' rectangle so its corners stay square (`__canvas_segmentDistanceSquare`).
    IF cap = 2 THEN
      RETURN __canvas_segmentDistanceSquare(px, py, p0, p1, p2, p3, radius)
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
    ' plan-163-B: Square unions a half-width box past each sweep endpoint, along the
    ' tangent there (`__canvas_arcSquareCap`).
    IF cap = 2 THEN
      arcD = __canvas_minF(arcD, __canvas_arcSquareCap(px, py, capSX, capSY, p0, p1, radius, FALSE))
      RETURN __canvas_minF(arcD, __canvas_arcSquareCap(px, py, capEX, capEY, p0, p1, radius, TRUE))
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
  IF kind = __CANVAS_GEO_POLYLINE THEN
    RETURN __canvas_polylineScan(tail, edges, px, py, radius, cap, FALSE)
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
///
/// **The sign follows the polygon's `FillRule`** (plan-163-A), read once from header
/// slot `__CANVAS_GEO_POLYGON_RULE` at `tail - __CANVAS_GEO_HEADER` (the
/// `__canvas_polylineScan` precedent). Both rules use the same crossing test — an edge
/// crosses when it straddles the pixel's row and passes left of the pixel. `EvenOdd`
/// flips `inside` per crossing, the statement sequence it has always had; `NonZero`
/// instead adds +1 for a crossing edge running down the screen (`by > ay`) and -1 for
/// one running up, and the pixel is inside when the sum is not zero. The sign convention
/// is invisible (only zero-or-not is observed) but both shaders use the same one. Stroked
/// text is a polygon whose slot 21 is 0, so glyphs stay even-odd.
#[rustfmt::skip]
const EDGE_DISTANCE: &str =
r#"FUNC __canvas_edgeDistance(tail AS Integer, edges AS Integer, px AS Float, py AS Float) AS Float
  RETURN __canvas_edgeDistanceRule(tail, edges, px, py, collections::getOr(__CANVAS_GEO_DATA, tail - __CANVAS_GEO_HEADER + __CANVAS_GEO_POLYGON_RULE, 0.0) <> 0.0)
END FUNC

' plan-163-C: the same walk over any edge set under an explicit rule -- a path clip's
' edges under its `clipRule` (`__canvas_clipPathDistance`).
FUNC __canvas_edgeDistanceRule(tail AS Integer, edges AS Integer, px AS Float, py AS Float, nonZero AS Boolean) AS Float
  MUT best AS Float = 1000000.0
  MUT inside AS Boolean = FALSE
  MUT winding AS Integer = 0
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
        IF nonZero THEN
          IF by > ay THEN
            winding = winding + 1
          ELSE
            winding = winding - 1
          END IF
        ELSE
          inside = NOT inside
        END IF
      END IF
    END IF
    e = e + 1
  END WHILE
  IF nonZero THEN
    inside = winding <> 0
  END IF
  IF inside THEN
    RETURN 0.0 - best
  END IF
  RETURN best
END FUNC"#;

/// The distance to a `PolyLine`'s stroke, or the path parameter of its nearest point
/// (plan-154-A).
///
/// One walk for both answers, chosen by `wantParam`, so the segment that decides the
/// colour is by construction the one that decides the coverage — two walks could break a
/// tie at a corner differently. The draw loop asks for the parameter only for a polyline
/// with colours.
///
/// Each segment is measured with **exactly** `__canvas_segmentDistance`'s arithmetic —
/// `(w·v) / len2`, a divide, not the tail's `invLenSq` multiply — and cut with exactly
/// `__canvas_segmentDistanceButt`'s end planes. That is what makes a two-point path draw
/// the pixels of the `canvas::Line` between the same points, which the golden checks:
/// `x * (1 / y)` and `x / y` differ in the last bit, and a last bit moves an antialiased
/// edge byte.
///
/// The union of the segments' bands is the `min` of their distances, which is what makes
/// every interior corner round for free. Only the first and last live segments take a
/// butt plane, each on its outer end only, so an interior end stays round and the
/// corner stays filled. A zero-length segment is skipped: its point is already inside
/// its neighbours' round ends. The one exception is a path with no live segment, whose
/// header names segment 0 as both first and last; it is then measured as the point it is,
/// which is the round dot a zero-length round `Line` draws.
///
/// The parameter is `(segment + t) / segments` with `t` clamped to the segment, so it is
/// continuous across a corner: the end of segment `i` and the start of `i + 1` are both
/// `i + 1`, and a tie there picks the same colour either way.
#[rustfmt::skip]
const POLYLINE_SCAN: &str =
r#"FUNC __canvas_polylineScan(tail AS Integer, edges AS Integer, px AS Float, py AS Float, half AS Float, cap AS Integer, wantParam AS Boolean) AS Float
  LET offset AS Integer = tail - __CANVAS_GEO_HEADER
  LET first AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, offset + __CANVAS_GEO_POLYLINE_FIRST, 0.0))
  LET last AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, offset + __CANVAS_GEO_POLYLINE_LAST, 0.0))
  ' plan-163-B: with join records the inner ends are cut butt and the corners are the
  ' records; the colour parameter still comes from the round walk, so it stays
  ' continuous round a corner.
  LET joins AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, offset + __CANVAS_GEO_JOIN_COUNT, 0.0))
  MUT joined AS Float = 1000000.0
  MUT best AS Float = 1000000.0
  MUT bestParam AS Float = 0.0
  MUT e AS Integer = first
  WHILE e <= last
    LET base AS Integer = tail + e * 5
    LET ax AS Float = collections::getOr(__CANVAS_GEO_DATA, base, 0.0)
    LET ay AS Float = collections::getOr(__CANVAS_GEO_DATA, base + 1, 0.0)
    LET vx AS Float = collections::getOr(__CANVAS_GEO_DATA, base + 2, 0.0)
    LET vy AS Float = collections::getOr(__CANVAS_GEO_DATA, base + 3, 0.0)
    LET len2 AS Float = vx * vx + vy * vy
    IF len2 > 0.0 OR first = last THEN
      LET wx AS Float = px - ax
      LET wy AS Float = py - ay
      MUT t AS Float = 0.0
      IF len2 > 0.0 THEN
        t = (wx * vx + wy * vy) / len2
      END IF
      LET clamped AS Float = __canvas_minF(__canvas_maxF(t, 0.0), 1.0)
      ' plan-163-B: a square OUTER end measures past its endpoint to the segment's line
      ' (an unclamped `t` there), so its corners are not rounded; an inner end stays
      ' round, which is the corner's join.
      MUT at AS Float = clamped
      IF cap = 2 THEN
        IF e = first AND t < 0.0 THEN
          at = t
        END IF
        IF e = last AND t > 1.0 THEN
          at = t
        END IF
      END IF
      LET dx AS Float = wx - vx * at
      LET dy AS Float = wy - vy * at
      MUT d AS Float = math::sqrt(dx * dx + dy * dy) - half
      IF cap = 2 THEN
        IF len2 > 0.0 THEN
          LET length AS Float = math::sqrt(len2)
          IF e = first THEN
            d = __canvas_maxF(d, 0.0 - t * length - half)
          END IF
          IF e = last THEN
            d = __canvas_maxF(d, (t - 1.0) * length - half)
          END IF
        ELSE
          d = __canvas_maxF(__canvas_absF(wx), __canvas_absF(wy)) - half
        END IF
      END IF
      IF cap = 0 THEN
        LET length AS Float = math::sqrt(len2)
        IF e = first THEN
          d = __canvas_maxF(d, 0.0 - t * length)
        END IF
        IF e = last THEN
          d = __canvas_maxF(d, (t - 1.0) * length)
        END IF
      END IF
      IF d < best THEN
        best = d
        bestParam = (toFloat(e) + clamped) / toFloat(edges)
      END IF
      IF joins > 0 THEN
        MUT dj AS Float = d
        IF len2 > 0.0 THEN
          LET inner AS Float = math::sqrt(len2)
          IF e <> first THEN
            dj = __canvas_maxF(dj, 0.0 - t * inner)
          END IF
          IF e <> last THEN
            dj = __canvas_maxF(dj, (t - 1.0) * inner)
          END IF
        END IF
        joined = __canvas_minF(joined, dj)
      END IF
    END IF
    e = e + 1
  END WHILE
  IF wantParam THEN
    RETURN bestParam
  END IF
  IF joins > 0 THEN
    RETURN __canvas_minF(joined, __canvas_joinsDistance(offset, px, py))
  END IF
  RETURN best
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always("canvas_kinds", KINDS));
    pkg.add_helper(RegistryHelper::always("canvas_intUtil", INT_UTIL));
    pkg.add_helper(RegistryHelper::always("canvas_edgeDistance", EDGE_DISTANCE));
    pkg.add_helper(RegistryHelper::always("canvas_polylineScan", POLYLINE_SCAN));
    pkg.add_helper(RegistryHelper::always("canvas_geoDistance", GEO_DISTANCE));
}
