use super::*;
use crate::codegen::runtime::canvas::{
    GEO_KIND_GROUP, GEO_KIND_PICTURE, GEO_KIND_POLYGON, GEO_KIND_POLYLINE, GEO_KIND_TEXT,
    HEADER_CAP, HEADER_HAS_TRANSFORM, HEADER_PICTURE_SHADOW_HI, HEADER_PICTURE_SHADOW_LO,
    HEADER_CLIP_EDGE_COUNT, HEADER_DASH_COUNT, HEADER_JOIN_COUNT, HEADER_JOIN_REACH, HEADER_POLYGON_RULE, HEADER_POLYLINE_FIRST,
    HEADER_GRADIENT_APPLY, HEADER_GRADIENT_FOCAL_X, HEADER_GRADIENT_HAS_INV, HEADER_GRADIENT_INV,
    HEADER_GRADIENT_SPREAD, HEADER_POLYLINE_LAST, HEADER_SLOTS,
    PICTURE_SHADOW_SPLIT_BITS,
};

/// The decimal literal `GEO_LAYOUT` binds to `name`.
fn declared(name: &str) -> usize {
    let needle = format!("LET {name} AS Integer = ");
    let start = GEO_LAYOUT
        .find(&needle)
        .unwrap_or_else(|| panic!("{name} is not declared in GEO_LAYOUT"))
        + needle.len();
    let rest = &GEO_LAYOUT[start..];
    let end = rest.find('\n').unwrap_or(rest.len());
    rest[..end].trim().parse().expect("a decimal literal")
}

/// `GEO_LAYOUT`'s constants equal their Rust counterparts.
///
/// Each of these numbers is spelled **twice** — once in MFBASIC here, once in
/// `runtime::canvas` for the emitters — with no compiler between them. Nothing
/// else relates the two spellings, so this test is the whole guard.
///
/// The header length is the dangerous one. Every geometry record is
/// `__CANVAS_GEO_HEADER` floats followed by a per-kind tail, and both GPU emitters
/// find that tail at `HEADER_SLOTS * 8` bytes. If the two disagree by even one
/// slot, a polygon's first edge coordinate is read as a header field and a header
/// field as an edge — which draws a *plausible wrong shape* rather than failing.
/// That is exactly the failure mode a change to the header length invites, because
/// such a change has to touch both spellings and can be applied to only one.
#[test]
fn the_geo_layout_constants_match_their_rust_counterparts() {
    assert_eq!(
        declared("__CANVAS_GEO_HEADER"),
        HEADER_SLOTS,
        "the MFBASIC header length and the emitters' HEADER_SLOTS disagree: the \
             tail would be read at the wrong offset, so a polygon's first edge \
             coordinate becomes a header field",
    );
    // plan-116-D. Both GPU emitters read the cap out of this slot now, so the
    // equality is the real guard; the two bounds below stay because they name what
    // goes wrong, and both failures draw a plausible wrong picture rather than
    // failing outright.
    let cap = declared("__CANVAS_GEO_CAP");
    assert_eq!(
        cap, HEADER_CAP,
        "the MFBASIC cap slot and the emitters' HEADER_CAP disagree, so a Line or \
             Arc writes its cap where the GPU paths do not look — and they read \
             whatever the neighbouring field happens to hold",
    );
    assert!(
        cap < HEADER_SLOTS,
        "__CANVAS_GEO_CAP {cap} is past the end of a {HEADER_SLOTS}-slot header, so \
             a Line or Arc would write its cap over a polygon's first edge coordinate",
    );
    assert!(
        cap > HEADER_HAS_TRANSFORM,
        "__CANVAS_GEO_CAP {cap} collides with a slot the emitters already read \
             (the last named one is HEADER_HAS_TRANSFORM at {HEADER_HAS_TRANSFORM})",
    );
    for (name, kind) in [
        ("__CANVAS_GEO_TEXT", GEO_KIND_TEXT),
        ("__CANVAS_GEO_POLYGON", GEO_KIND_POLYGON),
        // plan-116-G.
        ("__CANVAS_GEO_GROUP", GEO_KIND_GROUP),
        // bug-484.
        ("__CANVAS_GEO_PICTURE", GEO_KIND_PICTURE),
        // plan-154-A.
        ("__CANVAS_GEO_POLYLINE", GEO_KIND_POLYLINE),
    ] {
        assert_eq!(
            declared(name).to_string(),
            kind,
            "{name} and its emitter-side kind constant disagree, so the predicates \
                 and the emitters would branch on different values for the same kind",
        );
    }
    // plan-154-B: the emitters copy a polyline's first and last live segments out of
    // these two slots as whole numbers.
    assert_eq!(
        declared("__CANVAS_GEO_POLYLINE_FIRST"),
        HEADER_POLYLINE_FIRST
    );
    assert_eq!(declared("__CANVAS_GEO_POLYLINE_LAST"), HEADER_POLYLINE_LAST);
    // plan-163-A: both emitters copy a polygon's fill rule out of this slot into the
    // item block; a mismatch draws every NonZero polygon even-odd, or the reverse.
    assert_eq!(declared("__CANVAS_GEO_POLYGON_RULE"), HEADER_POLYGON_RULE);
    // plan-163-B: both emitters upload the join records by this count and widen the band
    // index by this reach.
    assert_eq!(declared("__CANVAS_GEO_JOIN_COUNT"), HEADER_JOIN_COUNT);
    assert_eq!(declared("__CANVAS_GEO_JOIN_REACH"), HEADER_JOIN_REACH);
    // plan-163-B: both emitters find the join records past this many dash floats.
    assert_eq!(declared("__CANVAS_GEO_DASH_COUNT"), HEADER_DASH_COUNT);
    // plan-163-C: and past this many clip edges, five floats each.
    assert_eq!(declared("__CANVAS_GEO_CLIP_EDGE_COUNT"), HEADER_CLIP_EDGE_COUNT);
}

/// plan-163-D: the gradient's per-pixel slots sit after every older slot, one each, in
/// the order the header comment gives -- spread, apply, the six radial-inverse words, its
/// flag, then the focal offset and radius -- and end exactly at the header's length, so
/// none overlaps another reader's slot or the first tail float.
#[test]
fn the_gradient_slots_follow_the_clip_slots_and_fill_the_header() {
    let slots = [
        ("__CANVAS_GEO_GRADIENT_SPREAD", HEADER_GRADIENT_SPREAD),
        ("__CANVAS_GEO_GRADIENT_APPLY", HEADER_GRADIENT_APPLY),
        ("__CANVAS_GEO_GRADIENT_INV", HEADER_GRADIENT_INV),
        ("__CANVAS_GEO_GRADIENT_HAS_INV", HEADER_GRADIENT_HAS_INV),
        ("__CANVAS_GEO_GRADIENT_FOCALX", HEADER_GRADIENT_FOCAL_X),
        ("__CANVAS_GEO_GRADIENT_FOCALY", HEADER_GRADIENT_FOCAL_X + 1),
        ("__CANVAS_GEO_GRADIENT_FOCALR", HEADER_GRADIENT_FOCAL_X + 2),
    ];
    for (name, slot) in slots {
        assert_eq!(declared(name), slot, "{name} and the emitters' constant disagree");
    }
    assert_eq!(HEADER_GRADIENT_SPREAD, 55, "the first gradient slot follows the clip slots");
    assert_eq!(HEADER_GRADIENT_INV + 6, HEADER_GRADIENT_HAS_INV);
    assert!(declared("__CANVAS_GEO_PICTURE_TRANSPARENCY") < 55);
    assert_eq!(declared("__CANVAS_GEO_HEADER"), 67);
    assert_eq!(HEADER_SLOTS, 67);
}

/// bug-484: a picture's pixel-block slots and split equal what the emitters rebuild
/// the address from, and they sit where no other reader of a picture looks.
///
/// Wrong here, the GPU copies texels from an address that is valid-looking garbage —
/// another block, or the middle of one — and draws a plausible wrong image.
#[test]
fn the_picture_layout_constants_match_their_rust_counterparts() {
    assert_eq!(
        declared("__CANVAS_GEO_PICTURE_SHADOW_HI"),
        HEADER_PICTURE_SHADOW_HI
    );
    assert_eq!(
        declared("__CANVAS_GEO_PICTURE_SHADOW_LO"),
        HEADER_PICTURE_SHADOW_LO
    );
    assert_eq!(
        declared("__CANVAS_GEO_PICTURE_SPLIT"),
        1usize << PICTURE_SHADOW_SPLIT_BITS,
        "the MFBASIC split and the emitters' shift disagree, so the rebuilt address is \
         not the block's",
    );
    // The slots are the Line/Arc cap endpoints, free on a picture; they must not be the
    // aux pair (the image size) or anything the rectangle fields use.
    for slot in [HEADER_PICTURE_SHADOW_HI, HEADER_PICTURE_SHADOW_LO] {
        assert!(slot > HEADER_CAP && slot < HEADER_SLOTS);
    }
}
