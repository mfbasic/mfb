//! Default values (`builder_value_semantics/default_value.rs`) for the shapes the
//! corpus does not reach, driven directly on hand-built type models: a data union
//! whose first defaultable variant holds a `RES` handle, and a type nothing can
//! default.
//!
//! For a data union the default is the first canonically ordered variant whose payload
//! is defaultable (`default_value_materializable`), and a `RES` field IS defaultable —
//! as a CLOSED handle, on which every operation and the drop are no-ops. So a variant
//! holding one is not skipped: it is chosen, and its field is a closed record.

use crate::arch::ops::CodeOp;
use crate::codegen::engine::tests::test_support::TestPlatform;
use crate::codegen::memory::arena::graph_copy::tests::{field, harness, model, record, union};
use crate::types::ParameterType;

/// A declared `RESOURCE Db`.
fn db_resource() -> crate::target::shared::nir::NirType {
    let mut db = record("Db", Vec::new());
    db.kind = "resource".to_string();
    db
}

/// The labels the default of `type_` emits over `model`, or its refusal.
fn default_labels(
    model: &crate::codegen::engine::builder::TypeModel,
    type_: &ParameterType,
) -> Result<Vec<String>, String> {
    let platform = TestPlatform;
    let harness = harness(model);
    let mut builder = harness.builder("default", &platform);
    builder.lower_default_value(type_)?;
    Ok(builder
        .instructions
        .iter()
        .filter(|instruction| instruction.op == CodeOp::Label)
        .filter_map(|instruction| instruction.get("name"))
        .collect())
}

/// `Opened { f AS RES Db }` is `Outcome`'s only defaultable variant (`Missing` holds a
/// type nothing can default), so the default must pick it — a `RES` field does not
/// disqualify a variant — and build its field as a closed handle record.
#[test]
fn a_union_default_through_a_res_field_holds_a_closed_handle() {
    let model = model(vec![
        db_resource(),
        record("Opened", vec![field("f", "RES Db")]),
        record("Missing", vec![field("why", "Undeclared")]),
        union(
            "Outcome",
            vec![
                ("Opened", vec![field("f", "RES Db")]),
                ("Missing", vec![field("why", "Undeclared")]),
            ],
        ),
    ]);
    let labels = default_labels(&model, &ParameterType::declared("Outcome"))
        .expect("the union has a default");
    assert!(
        labels
            .iter()
            .any(|name| name.starts_with("default_resource_alloc_ok")),
        "the chosen variant's RES field is a closed handle: {labels:?}"
    );
    assert!(
        labels
            .iter()
            .any(|name| name.starts_with("union_wrap_alloc_ok")),
        "the variant record is wrapped as the union: {labels:?}"
    );
}

/// A type with no default arm and no record fields is refused by name, not given a
/// zero word.
#[test]
fn a_type_with_no_default_is_refused_by_name() {
    let model = model(Vec::new());
    match default_labels(&model, &ParameterType::declared("Undeclared")) {
        Err(error) => assert!(
            error.contains("cannot materialize default value for type 'Undeclared'"),
            "{error}"
        ),
        Ok(_) => panic!("an undeclared type has no default"),
    }
}
