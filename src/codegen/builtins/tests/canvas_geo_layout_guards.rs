//! The native geometry builder refuses a record layout it cannot read, by name.
//!
//! `canvas::geoBuild` (`canvas/func_geo_build/`) reads each `DrawItem` variant's fields
//! by the offsets the type model gives them (`record.rs`): a scalar is read from its own
//! slot, a composite (a nested record or a list) through the slot's block offset or
//! pointer, an enum member by its ordinal. Read with the wrong rule, a field would be
//! a float taken from a block offset or a pointer taken from a float, so each read
//! checks the model first and fails the build, naming the field, if the canvas records
//! ever stop matching what the builder expects.
//!
//! The canvas package's records match, so no program reaches those refusals. These
//! tests take the type model of a real canvas program and change one entry at a time:
//! a scalar field made a record, a composite field made a scalar, a list field made a
//! record, an enum member removed.

use crate::codegen::engine::builder::{TypeModel, ValueResult};
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::tests::test_support::{BuilderHarness, TestPlatform};
use crate::codegen::registry::abi_function_lower;
use crate::testutil::{nir_for_src, CodeTarget};
use crate::types::ParameterType;

const CANVAS: &str = "\
IMPORT app
IMPORT canvas
IMPORT io

FUNC main() AS Integer
  app::setMode(app::Mode.Canvas)
  LET size = canvas::getSize()
  io::print(\"x\")
  RETURN 0
END FUNC
";

/// The model of a canvas program, as `geoBuild` reads it.
fn canvas_model() -> TypeModel {
    let target = CodeTarget::LinuxX86_64;
    let mode = target.app_mode().expect("linux-x86_64 has an app mode");
    let module = nir_for_src(CANVAS, target, mode).expect("the canvas program lowers");
    TypeModel::from_module(&module).expect("the model builds")
}

/// Set field `name` of `record` to `ty` in `model`.
fn retype(model: &mut TypeModel, record: &str, name: &str, ty: ParameterType) {
    let fields = model
        .record_fields
        .get_mut(&ParameterType::named(record))
        .unwrap_or_else(|| panic!("the model has {record}"));
    let slot = fields
        .iter_mut()
        .find(|(field, _)| field == name)
        .unwrap_or_else(|| panic!("{record} has a field {name}"));
    slot.1 = ty;
}

/// `canvas.geoBuild` lowered against `model`.
fn geo_build(model: TypeModel) -> Result<(), String> {
    let mut harness = BuilderHarness {
        type_model: model,
        ..BuilderHarness::default()
    };
    // The result list's allocation-failure exit names its message object.
    harness
        .string_symbols
        .insert("Allocation failed.".to_string(), "_mfb_str_oom".to_string());
    let platform = TestPlatform;
    let mut builder = harness.builder("_mfb_canvas_geo_probe", &platform);
    let ctx = harness.abi_ctx(&platform);
    let item = ValueResult {
        origin: None,
        type_: ParameterType::named("canvas.DrawItem"),
        location: Operand::from(builder.temporary_vreg()),
        text: "item".to_string(),
    };
    let (lower, _) = abi_function_lower("canvas.geoBuild").expect("geoBuild is an abi_function");
    lower(&mut builder, &[item], &ctx).map(|_| ())
}

#[test]
fn the_unchanged_model_builds() {
    geo_build(canvas_model()).expect("the canvas records are what the builder reads");
}

#[test]
fn a_scalar_field_held_as_a_composite_is_refused() {
    let mut model = canvas_model();
    retype(
        &mut model,
        "canvas.Circle",
        "radius",
        ParameterType::named("canvas.Size"),
    );
    let err = geo_build(model).err().expect("radius is read as a float");
    assert_eq!(
        err,
        "canvas: 'canvas.Circle.radius' is a composite (canvas.Size), not a scalar slot"
    );
}

#[test]
fn a_composite_field_held_as_a_scalar_is_refused() {
    let mut model = canvas_model();
    retype(
        &mut model,
        "canvas.Rectangle",
        "paint",
        ParameterType::Float,
    );
    let err = geo_build(model)
        .err()
        .expect("paint is read through its block");
    assert_eq!(
        err,
        "canvas: 'canvas.Rectangle.paint' is a scalar (Float), not a composite"
    );
}

#[test]
fn a_list_field_held_as_a_record_is_refused() {
    let mut model = canvas_model();
    retype(
        &mut model,
        "canvas.Polygon",
        "contours",
        ParameterType::named("canvas.Size"),
    );
    let err = geo_build(model)
        .err()
        .expect("contours is walked as a list");
    assert_eq!(err, "canvas: 'canvas.Size' is not a list");
}

#[test]
fn an_enum_member_the_builder_names_must_exist() {
    let mut model = canvas_model();
    // Under every spelling the model keys it by.
    let before = model.enum_members.len();
    model.enum_members.retain(|(ty, member), _| {
        !(member == "Center" && ty.name().trim_start_matches("canvas.") == "BindTo")
    });
    assert!(
        model.enum_members.len() < before,
        "the model has BindTo.Center"
    );
    let err = geo_build(model)
        .err()
        .expect("a centred rectangle needs BindTo.Center");
    assert_eq!(
        err,
        "canvas: the enum 'canvas.BindTo' has no member 'Center'"
    );
}
