//! plan-175-A Phase 1: every field of every natively hashed canvas kind, and every field
//! of `Paint`, has the hash lane the plan gives it (`canvas/func_item_lanes.rs`).
//!
//! The emitter's rule for an item field is total — a field a kind gains folds into the
//! geometry lane — so this test is where the classification of the canvas package's OWN
//! fields is written down: a field added to a kind, or a rule that starts classifying a
//! field differently, fails here and has to be looked at. `Paint` is stricter still: a
//! field `PAINT_FIELDS` does not name fails the build, and a row naming no field fails
//! here.

use crate::codegen::builtins::canvas::func_item_lanes::{
    item_field, paint_field, ItemField, PAINT_FIELDS,
};
use crate::codegen::engine::builder::TypeModel;
use crate::testutil::{nir_for_src, CodeTarget};
use crate::types::ParameterType;

use ItemField::{Colors, Geometry, Identity, Omitted, Paint, Shape};

/// The class every field of every kind `canvas::itemHash` folds must have.
const EXPECTED: [(&str, &[(&str, ItemField)]); 11] = [
    (
        "Rectangle",
        &[
            ("id", Omitted),
            ("position", Geometry),
            ("bind", Geometry),
            ("size", Geometry),
            ("radiusTL", Geometry),
            ("radiusTR", Geometry),
            ("radiusBL", Geometry),
            ("radiusBR", Geometry),
            ("paint", Paint),
            ("transform", Geometry),
        ],
    ),
    (
        "Circle",
        &[
            ("id", Omitted),
            ("position", Geometry),
            ("radius", Geometry),
            ("paint", Paint),
            ("transform", Geometry),
        ],
    ),
    (
        "Line",
        &[
            ("id", Omitted),
            ("position", Geometry),
            ("startAt", Geometry),
            ("endAt", Geometry),
            ("paint", Paint),
            ("transform", Geometry),
        ],
    ),
    (
        "Arc",
        &[
            ("id", Omitted),
            ("position", Geometry),
            ("radius", Geometry),
            ("startAngle", Geometry),
            ("endAngle", Geometry),
            ("paint", Paint),
            ("transform", Geometry),
        ],
    ),
    (
        "Polygon",
        &[
            ("id", Omitted),
            ("position", Geometry),
            ("contours", Geometry),
            ("rule", Geometry),
            ("paint", Paint),
            ("transform", Geometry),
        ],
    ),
    (
        "Picture",
        &[
            ("id", Omitted),
            ("shape", Shape),
            ("clip", Geometry),
            ("fit", Geometry),
            ("image", Identity),
        ],
    ),
    (
        "Ellipse",
        &[
            ("id", Omitted),
            ("position", Geometry),
            ("radiusX", Geometry),
            ("radiusY", Geometry),
            ("paint", Paint),
            ("transform", Geometry),
        ],
    ),
    (
        "PolyLine",
        &[
            ("id", Omitted),
            ("position", Geometry),
            ("points", Geometry),
            ("colors", Colors),
            ("paint", Paint),
            ("transform", Geometry),
        ],
    ),
    (
        "PolyCurve",
        &[
            ("id", Omitted),
            ("position", Geometry),
            ("curves", Geometry),
            ("paint", Paint),
            ("transform", Geometry),
        ],
    ),
    (
        "CurvePolygon",
        &[
            ("id", Omitted),
            ("position", Geometry),
            ("contours", Geometry),
            ("rule", Geometry),
            ("paint", Paint),
            ("transform", Geometry),
        ],
    ),
    // plan-173: the five fields of a `Tile`. No `id`, no `paint`: its appearance lane is
    // empty, so a tile is never refreshed.
    (
        "Tile",
        &[
            ("image", Identity),
            ("srcPosition", Geometry),
            ("srcSize", Geometry),
            ("position", Geometry),
            ("size", Geometry),
        ],
    ),
];

/// The canvas package's records, as every program's type model carries them.
fn canvas_model() -> TypeModel {
    let module = nir_for_src(
        "IMPORT canvas\n\nFUNC main() AS Integer\n  RETURN 0\nEND FUNC\n",
        CodeTarget::LinuxX86_64,
        crate::target::NativeBuildMode::Console,
    )
    .expect("the program lowers");
    TypeModel::from_module(&module).expect("the model builds")
}

fn fields(model: &TypeModel, record: &str) -> Vec<(String, ParameterType)> {
    model
        .record_fields
        .get(&ParameterType::named(&format!("canvas.{record}")))
        .unwrap_or_else(|| panic!("canvas.{record} is in the type model"))
        .clone()
}

fn is_shape(ty: &ParameterType) -> bool {
    ty.is_named("AutoShape") || ty.is_named("canvas.AutoShape")
}

#[test]
fn every_field_of_every_hashed_kind_has_its_lane() {
    let model = canvas_model();
    for (kind, expected) in EXPECTED {
        let actual = fields(&model, kind);
        let names: Vec<&str> = actual.iter().map(|(name, _)| name.as_str()).collect();
        let listed: Vec<&str> = expected.iter().map(|(name, _)| *name).collect();
        assert_eq!(
            names, listed,
            "canvas.{kind}'s fields are not the ones this test classifies: a field was \
             added, removed or reordered, so say which lane it folds into"
        );
        for ((name, ty), (_, class)) in actual.iter().zip(expected.iter()) {
            assert_eq!(
                item_field(&format!("canvas.{kind}"), name, ty, is_shape(ty)),
                *class,
                "canvas.{kind}.{name} (of type {ty})"
            );
        }
    }
}

#[test]
fn every_paint_field_has_a_lane_and_every_lane_names_a_field() {
    let model = canvas_model();
    let paint: Vec<String> = fields(&model, "Paint")
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    for name in &paint {
        paint_field(name).unwrap_or_else(|e| panic!("{e}"));
    }
    for (name, _) in PAINT_FIELDS {
        assert!(
            paint.iter().any(|field| field == name),
            "PAINT_FIELDS names '{name}', which Paint does not have"
        );
    }
}

/// The guard itself: a `Paint` field nobody classified is a build error, naming it.
#[test]
fn an_unclassified_paint_field_fails() {
    assert_eq!(
        paint_field("glow").err().as_deref(),
        Some(
            "canvas::itemHash: 'Paint.glow' has no hash lane in PAINT_FIELDS \
             (func_item_lanes.rs): classify it as geometry or appearance"
        )
    );
}
