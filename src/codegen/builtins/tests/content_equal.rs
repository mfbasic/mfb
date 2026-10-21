//! Content equality (`emit_values_content_equal_branch`) over the value shapes
//! `canvas::present` does not happen to hand it.
//!
//! It compares what two values SAY, not their bytes: an embedded list keeps the
//! capacity it was built with, so two equal values are different bytes, and a list
//! is compared count first and then element by element. One routine is emitted per
//! compared type. The scene-diff in `canvas::present` is its only caller from a
//! program, and it passes scene records; these rows drive the emitter directly on
//! the type model of a real program, for the shapes that caller never builds:
//!
//!   * the unit type has one value, so two of them are equal and nothing is
//!     emitted to compare them (a `List OF Nothing` is equal when the counts are,
//!     and a `Nothing` field is skipped);
//!   * a `RES <resource union>` field is a two-word box, compared in place;
//!   * a `Map`, a `Set` and a record that is not flat have NO arm: the emitter
//!     answers `Err`, naming the type, rather than comparing bytes that do not
//!     mean anything.

use crate::arch::ops::CodeOp;
use crate::codegen::engine::builder::TypeModel;
use crate::codegen::engine::tests::test_support::{BuilderHarness, TestPlatform};
use crate::testutil::{nir_for_src, CodeTarget};
use crate::types::ParameterType;

const PROGRAM: &str = "\
IMPORT fs
IMPORT io
IMPORT tcp

UNION Stream
  fs::File
  tcp::Socket
END UNION

TYPE WithUnit
  u AS Nothing
  n AS Integer
END TYPE

TYPE Box
  stream AS RES Stream
  n AS Integer
END TYPE

TYPE Unflat
  m AS Map OF String TO Integer
  n AS Integer
END TYPE

TYPE Pointered
  callback AS FUNC(Integer) AS Integer
  n AS Integer
END TYPE

FUNC main() AS Integer
  io::print(\"ok\")
  RETURN 0
END FUNC
";

/// The instructions `emit_values_content_equal_branch` emits for `type_`, or its
/// refusal.
fn emit(
    type_: &ParameterType,
) -> Result<Vec<crate::codegen::engine::types::CodeInstruction>, String> {
    let module = nir_for_src(
        PROGRAM,
        CodeTarget::LinuxX86_64,
        crate::target::NativeBuildMode::Console,
    )
    .expect("the program lowers");
    let mut harness = BuilderHarness {
        type_model: TypeModel::from_module(&module).expect("the model builds"),
        ..BuilderHarness::default()
    };
    harness
        .string_symbols
        .insert("Allocation failed.".to_string(), "_mfb_str_oom".to_string());
    let platform = TestPlatform;
    let mut builder = harness.builder("_mfb_content_probe", &platform);
    let left = builder.temporary_vreg();
    let right = builder.temporary_vreg();
    builder.emit_values_content_equal_branch(type_, left, right, "probe_equal", "probe_unequal")?;
    Ok(builder.instructions)
}

fn branches_to(
    instructions: &[crate::codegen::engine::types::CodeInstruction],
    label: &str,
) -> bool {
    instructions
        .iter()
        .any(|i| i.get("target").as_deref() == Some(label))
}

#[test]
fn two_lists_of_the_unit_type_are_equal_when_their_counts_are() {
    let instructions = emit(&ParameterType::list_of(ParameterType::Nothing)).expect("emits");
    assert!(
        instructions.iter().any(|i| i.op == CodeOp::Cmp),
        "the counts are compared"
    );
    assert!(
        !instructions.iter().any(|i| i.op == CodeOp::Mul),
        "a unit element has no width to scale the byte range by: nothing is compared past the count"
    );
    assert!(branches_to(&instructions, "probe_equal"));
    assert!(branches_to(&instructions, "probe_unequal"));
}

#[test]
fn a_unit_field_is_skipped_and_a_resource_union_box_is_compared_in_place() {
    let with_unit = emit(&ParameterType::declared("WithUnit")).expect("emits");
    let plain = emit(&ParameterType::list_of(ParameterType::Integer)).expect("emits");
    assert!(
        branches_to(&with_unit, "probe_equal") && !with_unit.is_empty() && !plain.is_empty(),
        "the record compares its `Integer` and skips the `Nothing`"
    );
    let boxed = emit(&ParameterType::declared("Box")).expect("emits");
    assert!(
        boxed.iter().any(|i| i.op == CodeOp::LdrU64),
        "the two-word box is loaded and compared word by word"
    );
}

#[test]
fn a_type_with_no_content_arm_is_refused_by_name() {
    for type_ in [
        ParameterType::declared("Unflat"),
        // A record holding a function value is not a flat
        // block, and bytes that include a pointer say nothing about content.
        ParameterType::declared("Pointered"),
        // A function value has no content.
        ParameterType::Func(
            vec![ParameterType::Integer],
            Box::new(ParameterType::Integer),
            false,
        ),
        ParameterType::map_of(ParameterType::String, ParameterType::Integer),
        ParameterType::set_of(ParameterType::Integer),
    ] {
        let Err(refusal) = emit(&type_) else {
            panic!("{type_}: a map, a set, a function and a non-flat record have no content arm");
        };
        assert!(
            refusal.contains("does not support type"),
            "{type_}: the refusal says what is unsupported: {refusal}"
        );
    }
}
