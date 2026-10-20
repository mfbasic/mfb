//! Geometry generation and the geometry cache.
//!
//! A `DrawItem` is what the *program* wrote; **geometry** is what the *renderer*
//! draws. Generation turns one into the other: a fixed 51-float header carrying the
//! shape's kind, its distance-function parameters, its two colours and its bounds,
//! followed by a per-kind tail (a polygon's precomputed edge array).
//!
//! Slots 20 and 21 are the per-kind auxiliary pair: an arc's start and end angle, or
//! a polygon's edge count. They mean different things because only one kind reads
//! each — a shape never needs both — and giving each its own slot would widen every
//! record to carry fields no primitive uses at once.
//!
//! `Line` and `Arc` have no interior. Their generators put the **stroke** colour in
//! the fill slots and set the stroke half-width negative
//! (`__canvas_strokeAsFill`), so the single rasterisation loop draws them in one pass
//! with no special case: the band the distance function already describes *is* the
//! shape.
//!
//! This flat float buffer is deliberately the shape a GPU backend uploads. plan-98-E
//! and F consume exactly this: the header is an SDF quad's per-instance parameter
//! block, and the tail is the only per-item vertex data any primitive needs. Keeping
//! the software oracle and the GPU backends on one geometry representation is what
//! makes the oracle predictive rather than merely similar.
//!
//! ## Why the cache pays
//!
//! For an SDF shape the header is a handful of field reads and caching it saves
//! little. The tail is where the work is: a polygon's edge array turns the per-pixel
//! distance query from "recompute every edge vector" into "read five floats", and
//! building it is `O(points)` per item. A scene re-presenting an unchanged polygon
//! rebuilds nothing — which is plan-98-A invariant 2, and is what
//! `__canvas_geoGenerations` counts so a test can see it happen.
//!
//! ## Why a hash *and* a comparison
//!
//! The probe is by hash, but a hit is confirmed by comparing the 51-float header
//! exactly before the tail is reused. A hash alone would let a collision reuse
//! another item's geometry and silently draw the wrong picture — a rare wrong answer
//! is worse than a common slow one, and the confirmation costs 51 float compares
//! against a tail that can be thousands.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};

mod cache;
mod header_kinds;
mod header_paint;
mod tail;
mod verify;
use cache::{GEO_CACHE, GEO_CACHE_STATE, GEO_HASH};
use tail::GEO_TAIL;
use verify::{GEO_VERIFY, GEO_VERIFY_DEBUG};

/// The geometry record's fixed header, and the kind tags the rasteriser switches on.
///
/// `__CANVAS_GEO_NONE` is a real kind rather than an absent record: `Text` and
/// `Picture` still occupy a scene slot and still have a cache entry, so the item
/// indices, the hash list and the geometry offsets all stay parallel. Dropping them
/// would make every downstream index depend on which variants happened to be
/// present.
#[rustfmt::skip]
const GEO_LAYOUT: &str =
r#"LET __CANVAS_GEO_HEADER AS Integer = 69
' bug-1083: how much of the item's box (slots 16-19) is rim, on the x sides (67) and
' on the y sides (68), in the units the box is in. The box less this is the item's
' extent and stroke reach under its own transform and nothing more; the sites that
' carry a box through a group's matrix replace the rim there (`__canvas_drawBound`).
' Zero for a box that is not an extent and a rim (`__canvas_boundsHeader`).
LET __CANVAS_GEO_RIM AS Integer = 67
' plan-163-D: the gradient's per-pixel words, after plan-163-C's. 55 is the spread
' (0 Pad, 1 Reflect, 2 Repeat), 56 whether the ramp also paints the stroke (1 for
' FillAndStroke on a kind that has a stroke to paint), 57-62 a RADIAL gradient's inverse
' transform as float32 bit patterns (the slots 27-32 convention) with 63 its flag, and
' 64-66 the focal offset from the centre and the focal radius, already clamped. A linear
' gradient's transform is folded into slots 43-46 at build and leaves 57-63 at zero.
LET __CANVAS_GEO_GRADIENT_SPREAD AS Integer = 55
LET __CANVAS_GEO_GRADIENT_APPLY AS Integer = 56
LET __CANVAS_GEO_GRADIENT_INV AS Integer = 57
LET __CANVAS_GEO_GRADIENT_HAS_INV AS Integer = 63
LET __CANVAS_GEO_GRADIENT_FOCALX AS Integer = 64
LET __CANVAS_GEO_GRADIENT_FOCALY AS Integer = 65
LET __CANVAS_GEO_GRADIENT_FOCALR AS Integer = 66
' plan-163-C: the clip and transparency slots, after plan-163-B's. 51 is the clip's mode
' (0 none, 1 an axis-aligned rectangle in slots 22-25, 2 a path of clip edges), 52 the
' clip edge count, 53 the clip's FillRule ordinal, 54 a picture's transparency step T
' (0..255) -- every other kind's fill and stroke alphas are faded at build instead.
LET __CANVAS_GEO_CLIP_MODE AS Integer = 51
LET __CANVAS_GEO_CLIP_EDGE_COUNT AS Integer = 52
LET __CANVAS_GEO_CLIP_RULE AS Integer = 53
LET __CANVAS_GEO_PICTURE_TRANSPARENCY AS Integer = 54
' plan-163-B: the stroke-style slots, appended after every older slot so none of them
' moved. 47 is the number of join records in the tail (`__canvas_joinBase`), 48 how far
' a record reaches from its vertex (the band index widens to it); 49 is the dash tail's
' float count (`__canvas_dashBase`) and 50 the dash offset wrapped into one period.
LET __CANVAS_GEO_JOIN_COUNT AS Integer = 47
LET __CANVAS_GEO_JOIN_REACH AS Integer = 48
LET __CANVAS_GEO_DASH_COUNT AS Integer = 49
LET __CANVAS_GEO_DASH_PHASE AS Integer = 50
LET __CANVAS_GEO_GRADIENT_COUNT AS Integer = 41
LET __CANVAS_GEO_GRADIENT_KIND AS Integer = 42
LET __CANVAS_GEO_GRADIENT_FROMX AS Integer = 43
LET __CANVAS_GEO_GRADIENT_FROMY AS Integer = 44
LET __CANVAS_GEO_GRADIENT_TOX AS Integer = 45
LET __CANVAS_GEO_GRADIENT_TOY AS Integer = 46
LET __CANVAS_GEO_ELLIPSE AS Integer = 7
LET __CANVAS_GEO_ELLIPSE_COS AS Integer = 39
LET __CANVAS_GEO_ELLIPSE_SIN AS Integer = 40
LET __CANVAS_GEO_CAP AS Integer = 34
LET __CANVAS_GEO_CAPSTARTX AS Integer = 35
LET __CANVAS_GEO_CAPSTARTY AS Integer = 36
LET __CANVAS_GEO_CAPENDX AS Integer = 37
LET __CANVAS_GEO_CAPENDY AS Integer = 38
LET __CANVAS_GEO_TEXT AS Integer = 6
LET __CANVAS_GEO_NONE AS Integer = 5
LET __CANVAS_GEO_POLYGON AS Integer = 4
' plan-163-A. A polygon's `FillRule` as its ordinal (0 EvenOdd, 1 NonZero), in the
' per-kind aux slot 21 no polygon otherwise writes or reads -- the POLYLINE_FIRST/LAST
' aliasing precedent. Stroked text is also a polygon and leaves it 0, so text stays
' even-odd.
LET __CANVAS_GEO_POLYGON_RULE AS Integer = 21
LET __CANVAS_GEO_ARC AS Integer = 3
' plan-116-G. 8 is the next free value; a Group's header carries this rather than
' NONE so the deferred-hash path and the renderers can tell "a group node" from
' "an item with no geometry", which are different things.
LET __CANVAS_GEO_GROUP AS Integer = 8
' bug-484. A Picture: the destination rectangle in the rectangle's own slots (centre
' 2-3, half-extent 4-5, radius 6 = 0), so its coverage and stroke ARE a rectangle's;
' the image's width and height in the per-kind aux pair 20-21; and its pixel block's
' address in two 24-bit halves. Halves because a slot holding a whole 48-bit address
' would not survive the float round trips the renderers make through `toInt` (the font
' handle note in `__canvas_hashItem` measures exactly that overflow).
' 35-36 are the cap slots, which only Line and Arc read.
LET __CANVAS_GEO_PICTURE AS Integer = 9
LET __CANVAS_GEO_PICTURE_SHADOW_HI AS Integer = 35
LET __CANVAS_GEO_PICTURE_SHADOW_LO AS Integer = 36
LET __CANVAS_GEO_PICTURE_SPLIT AS Integer = 16777216
' plan-173. A Tile uses the picture image words and rectangle destination slots, with
' its requested source rectangle in a four-float tail.
LET __CANVAS_GEO_TILE AS Integer = 13
LET __CANVAS_GEO_TILE_SRC_X AS Integer = 69
LET __CANVAS_GEO_TILE_SRC_Y AS Integer = 70
LET __CANVAS_GEO_TILE_SRC_W AS Integer = 71
LET __CANVAS_GEO_TILE_SRC_H AS Integer = 72
' plan-154-A. A PolyLine: stroke-as-fill like a Line (the stroke colour in the fill
' slots, the half-width in the radius slot 6, slot 7 at -1), its segment count in the
' aux slot 20 as a polygon's edge count is, and a tail of that many edges in the
' polygon layout followed by its per-point colours as gradient stops. The first and
' last segments with length go in the gradient axis slots 43-44, which a polyline does
' not otherwise use: its colours are parameterised by the path, not by an axis.
LET __CANVAS_GEO_POLYLINE AS Integer = 10
LET __CANVAS_GEO_POLYLINE_FIRST AS Integer = 43
LET __CANVAS_GEO_POLYLINE_LAST AS Integer = 44
' plan-164-A. The two curve shapes, as POLYLINE and POLYGON are two kinds: both carry
' the same tail of rational-quadratic pieces (`helper_curves.rs`), and the kind says
' whether there is an interior.
LET __CANVAS_GEO_POLYCURVE AS Integer = 11
LET __CANVAS_GEO_CURVEPOLYGON AS Integer = 12"#;

/// The fixed header builders, one per kind.
///
/// Every builder writes the same 51 slots, so the rasteriser and the emitters can be
/// written once against the layout instead of per kind. `__canvas_geometryFor` picks the
/// builder in one `MATCH` that is exhaustive over the frozen `DrawItem` set, so a new
/// variant fails to compile there rather than silently generating nothing.
#[rustfmt::skip]
const GEO_HEADER: &str = concat!(header_paint::geo_header_paint!(), header_kinds::geo_header_kinds!());

pub(crate) fn register(pkg: &mut RegistryPackage) {
    for helper in RegistryHelper::debug_split("canvas_geoVerify", GEO_VERIFY, GEO_VERIFY_DEBUG) {
        pkg.add_helper(helper);
    }
    pkg.add_helper(RegistryHelper::always("canvas_geoLayout", GEO_LAYOUT));
    pkg.add_helper(RegistryHelper::always(
        "canvas_geoCacheState",
        GEO_CACHE_STATE,
    ));
    pkg.add_helper(RegistryHelper::always("canvas_geoHash", GEO_HASH));
    pkg.add_helper(RegistryHelper::always("canvas_geoHeader", GEO_HEADER));
    pkg.add_helper(RegistryHelper::always("canvas_geoTail", GEO_TAIL));
    pkg.add_helper(RegistryHelper::always("canvas_geoCache", GEO_CACHE));
}

#[cfg(test)]
mod tests;
