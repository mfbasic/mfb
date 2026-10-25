//! plan-175-A: which lane of an item's hash every field folds into — the one
//! classification the native item hash is generated from.
//!
//! An item has two identities. `G`, its GEOMETRY identity, folds everything that decides
//! the shape of its geometry record: the kind's own fields, its transform, and the parts
//! of its paint that are geometry. `A`, its APPEARANCE identity, folds everything else it
//! draws with: colours, the blend mode, a gradient's ramp, the transparency. `G` keys the
//! geometry cache; `D = mix(G, A)` is the damage identity and changes exactly when either
//! does (`func_item_hash.rs`).
//!
//! **Every field of `Paint` is named in [`PAINT_FIELDS`]**, and a `Paint` field it does
//! not name fails the BUILD (`paint_field`), as a field of a type the walk cannot fold
//! does: a new `Paint` field cannot be folded into the wrong lane, or into neither, by
//! accident.
//!
//! **An item's own fields are geometry, with four exceptions** ([`item_field`]): its `id`
//! (in no lane), a resource (content identity, folded into `G`), its `Paint` (field by
//! field, above), and a `PolyLine`'s per-point colours (the count into `G`, the colours
//! into `A`). The rule is total on purpose: a field a kind gains folds into `G`, the lane
//! where a change can only ever rebuild a record, never leave a stale one. Which class
//! every field of every canvas kind gets is pinned by a test
//! (`builtins/tests/canvas_item_lanes.rs`), so a change to a class is a visible one.
//!
//! Three things in a `Paint` are geometry although they sit in appearance fields, and
//! each is folded into `G` as its own word:
//!
//! * whether the stroke's alpha is zero. `__canvas_strokeHalf` (and its native twin)
//!   answers -1 for it, which moves the bounds, drops the joins and dashes, and makes a
//!   `Line`, an `Arc`, a `PolyLine` or a `PolyCurve` an empty record. Its other inputs —
//!   the width, the stop count, `strokeApply` — are geometry already, so `G` determines
//!   its answer without a second copy of its rule;
//! * the gradient's stop COUNT, which sizes the record's tail and decides "is there a
//!   ramp", and its `strokeApply`, which decides whether a stroke-only kind carries one;
//! * a `PolyLine`'s colour COUNT (`__canvas_polylineHalf`), whose values are appearance.
//!
//! A fill's alpha never changes what is built, and the transparency only fades the two
//! alphas the header stores (`__canvas_paintHeader`), so both are appearance.

use crate::types::ParameterType;

/// How a field of a hashed item folds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ItemField {
    /// Into `G`, whole.
    Geometry,
    /// A resource's backend id, into `G`: content identity (a different image is a
    /// different record, its size being geometry). A picture's record is still
    /// confirmed against the image's pixel block every frame (`__canvas_pictureCachedOf`).
    Identity,
    /// Into neither lane, nor into `D` (plan-175-A decision 1: `id` is only meant for
    /// mouse feedback, so a changed `id` redraws nothing).
    Omitted,
    /// The item's `Paint`, field by field through [`paint_field`].
    Paint,
    /// A `PolyLine`'s per-point colours: the count into `G`, the colours into `A`.
    Colors,
    /// A `Picture`'s shape union, folded last by going round the variant dispatch
    /// (`func_item_hash.rs`), so its fields fold by their own kind's rule.
    Shape,
}

/// How a field of a `Paint` folds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PaintField {
    Geometry,
    Appearance,
    /// The stroke colour: whole into `A`, and whether its alpha is zero into `G`.
    Stroke,
    /// The gradient: whole into `A`, and its stop count and `strokeApply` into `G`.
    Gradient,
}

/// Every field of `Paint`.
pub(crate) const PAINT_FIELDS: [(&str, PaintField); 13] = [
    ("fill", PaintField::Appearance),
    ("stroke", PaintField::Stroke),
    ("strokeWidth", PaintField::Geometry),
    ("blend", PaintField::Appearance),
    ("clip", PaintField::Geometry),
    ("clipRule", PaintField::Geometry),
    ("gradient", PaintField::Gradient),
    ("cap", PaintField::Geometry),
    ("join", PaintField::Geometry),
    ("miterLimit", PaintField::Geometry),
    ("dashes", PaintField::Geometry),
    ("dashOffset", PaintField::Geometry),
    ("transparency", PaintField::Appearance),
];

/// The class of `kind.field` of type `ty`. `is_shape` says the field is the shape union.
pub(crate) fn item_field(kind: &str, field: &str, ty: &ParameterType, is_shape: bool) -> ItemField {
    let bare = kind.strip_prefix("canvas.").unwrap_or(kind);
    if is_shape {
        ItemField::Shape
    } else if field == "id" && matches!(ty, ParameterType::String) {
        ItemField::Omitted
    } else if matches!(ty, ParameterType::Res(_)) {
        ItemField::Identity
    } else if ty.is_named("Paint") || ty.is_named("canvas.Paint") {
        ItemField::Paint
    } else if bare == "PolyLine" && field == "colors" {
        ItemField::Colors
    } else {
        ItemField::Geometry
    }
}

/// The class of `Paint.field`, or the build error that an unclassified field is.
pub(crate) fn paint_field(field: &str) -> Result<PaintField, String> {
    PAINT_FIELDS
        .iter()
        .find(|(name, _)| *name == field)
        .map(|(_, class)| *class)
        .ok_or_else(|| {
            format!(
                "canvas::itemHash: 'Paint.{field}' has no hash lane in PAINT_FIELDS \
                 (func_item_lanes.rs): classify it as geometry or appearance"
            )
        })
}
