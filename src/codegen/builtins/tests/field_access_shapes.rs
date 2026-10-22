//! A field read (`lower_field_access`) and a `WITH` update (`lower_with_update`) over
//! the shapes the checker stops before codegen, driven directly on a hand-built type
//! model: a value typed by a union VARIANT the model knows only as a variant (no
//! record of its own — a package can export a union without its member types), a
//! union value, a map entry's unknown member, and a `WITH` naming a field the record
//! does not have. Each is either read at the variant's payload offset or refused by
//! name — never read at a guessed offset.

use crate::arch::ops::CodeOp;
use crate::codegen::engine::builder::GlobalValue;
use crate::codegen::engine::tests::test_support::TestPlatform;
use crate::codegen::memory::arena::graph_copy::tests::{field, harness, model, record, union};
use crate::target::shared::nir::{NirRecordUpdate, NirValue};
use crate::types::ParameterType;

fn global(name: &str, type_: ParameterType) -> NirValue {
    NirValue::Global {
        name: name.to_string(),
        type_,
    }
}

/// `UNION Shape { Circle }` whose `Circle` variant has fields but no record, and a
/// record `Pt { x, y }`.
fn shapes() -> crate::codegen::engine::builder::TypeModel {
    model(vec![
        union(
            "Shape",
            vec![("Circle", vec![field("r", "Integer"), field("d", "Integer")])],
        ),
        record("Pt", vec![field("x", "Integer"), field("y", "Integer")]),
    ])
}

/// Read `member` off a global `g` of `type_`; the instructions, or the refusal.
fn read(
    type_: ParameterType,
    member: &str,
) -> Result<Vec<crate::codegen::engine::types::CodeInstruction>, String> {
    let model = shapes();
    let mut harness = harness(&model);
    harness.globals.insert(
        "g".to_string(),
        GlobalValue {
            type_: type_.clone(),
            offset: 64,
        },
    );
    let platform = TestPlatform;
    let mut builder = harness.builder("field", &platform);
    let before = builder.instructions.len();
    builder.lower_field_access(&global("g", type_), member)?;
    Ok(builder.instructions[before..].to_vec())
}

fn refused<T>(result: Result<T, String>, what: &str) -> String {
    match result {
        Err(error) => error,
        Ok(_) => panic!("{what} must be refused"),
    }
}

/// A variant known only as a variant is laid out as its union's payload: its field
/// `d` (index 1) is read one word past the tag, at `8 + 8 * 1`.
#[test]
fn a_variant_without_a_record_reads_its_field_past_the_tag() {
    let instructions = read(ParameterType::declared("Circle"), "d").expect("the field reads");
    assert!(
        instructions
            .iter()
            .any(|instruction| instruction.op == CodeOp::LdrU64
                && instruction
                    .fields
                    .iter()
                    .any(|(_, operand)| operand.render() == "16")),
        "field 1 of the payload is at +16"
    );
}

/// The refusals, each naming what was wrong.
#[test]
fn a_field_the_value_does_not_have_is_refused_by_name() {
    let error = refused(
        read(ParameterType::declared("Circle"), "nope"),
        "an unknown variant field",
    );
    assert!(
        error.contains("variant 'Circle' has no field 'nope'"),
        "{error}"
    );
    let error = refused(
        read(ParameterType::declared("Shape"), "r"),
        "a field read off a union",
    );
    assert!(
        error.contains("'Shape' is not a record or variant"),
        "{error}"
    );
    let error = refused(
        read(
            ParameterType::map_entry_of(ParameterType::String, ParameterType::Integer),
            "size",
        ),
        "a map entry's unknown member",
    );
    assert!(error.contains("has no field 'size'"), "{error}");
}

/// A `WITH` naming a field the record does not have is refused before anything is
/// stored.
#[test]
fn a_with_update_of_an_unknown_field_is_refused() {
    let model = shapes();
    let pt = ParameterType::declared("Pt");
    let mut harness = harness(&model);
    harness.globals.insert(
        "p".to_string(),
        GlobalValue {
            type_: pt.clone(),
            offset: 64,
        },
    );
    let platform = TestPlatform;
    let mut builder = harness.builder("with", &platform);
    let error = refused(
        builder.lower_with_update(
            &pt,
            &global("p", pt.clone()),
            &[NirRecordUpdate {
                field: "z".to_string(),
                value: NirValue::Const {
                    type_: ParameterType::Integer,
                    value: "1".to_string(),
                },
            }],
        ),
        "a WITH of an unknown field",
    );
    assert!(error.contains("unknown field 'z'"), "{error}");
}
