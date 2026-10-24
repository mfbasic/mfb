//! `WITH` fast paths (`engine/control/builder_control/field_with.rs`) that decline or
//! refuse, driven on a builder directly.
//!
//! * `try_inplace_scalar_fields` stores into the owner's block only for a `WITH`
//!   that rebuilds that owner and names fields its record has; anything else
//!   returns "not handled" so the ordinary rebuild runs.
//! * bug-766 B: a `WITH` built in the record-temp scratch must fit the region the
//!   prescan reserved; a larger record is refused rather than written past it.
//! * plan-145: only a collection field can be the growing last inlined sub-block.

use crate::codegen::collection::assign::self_update::FieldContainer;
use crate::codegen::engine::builder::{CodeBuilder, LocalValue};
use crate::codegen::engine::tests::test_support::TestPlatform;
use crate::codegen::memory::arena::graph_copy::tests::{field, harness, model, record};
use crate::target::shared::nir::{NirRecordUpdate, NirValue};
use crate::types::ParameterType;

/// `TYPE Pt { x, y AS Integer }`, with `p AS Pt` bound.
fn with_builder<R>(body: impl FnOnce(&mut CodeBuilder<'_>) -> R) -> R {
    let model = model(vec![
        record("Pt", vec![field("x", "Integer"), field("y", "Integer")]),
        record(
            "Bag",
            vec![field("n", "Integer"), field("xs", "List OF Integer")],
        ),
    ]);
    let harness = harness(&model);
    let platform = TestPlatform;
    let mut builder = harness.builder("with", &platform);
    let stack_offset = builder.allocate_stack_object("p", 8);
    builder.locals.insert(
        "p".to_string(),
        LocalValue {
            type_: ParameterType::declared("Pt"),
            stack_offset,
            constant: None,
            by_ref: false,
        },
    );
    body(&mut builder)
}

fn int(value: &str) -> NirValue {
    NirValue::Const {
        type_: ParameterType::Integer,
        value: value.to_string(),
    }
}

fn update(field: &str, value: NirValue) -> NirRecordUpdate {
    NirRecordUpdate {
        field: field.to_string(),
        value,
    }
}

fn with_p(updates: Vec<NirRecordUpdate>) -> NirValue {
    NirValue::WithUpdate {
        type_: ParameterType::declared("Pt"),
        target: Box::new(NirValue::Local("p".to_string())),
        updates,
    }
}

#[test]
fn an_in_place_with_declines_what_does_not_rebuild_its_owner() {
    with_builder(|builder| {
        let owner = FieldContainer::Record { local: "p" };
        let handled = |builder: &mut CodeBuilder<'_>, value: &NirValue| {
            builder
                .try_inplace_scalar_fields(owner, value)
                .expect("a decline is not an error")
        };
        assert!(!handled(builder, &int("1")), "not a WITH of the owner");
        assert!(
            !handled(builder, &with_p(vec![update("nope", int("1"))])),
            "a field the record does not have"
        );
        assert!(
            handled(builder, &with_p(vec![update("x", int("1"))])),
            "a scalar field of the owner is stored in place"
        );
    });
}

#[test]
fn a_record_temp_with_that_does_not_fit_its_region_is_refused() {
    with_builder(|builder| {
        let offset = builder.allocate_stack_object("with_item", 8);
        builder.record_temps.with_item = Some((offset, 8));
        match builder.lower_with_update_in_record_temp(
            &ParameterType::declared("Pt"),
            &NirValue::Local("p".to_string()),
            &[update("x", int("1"))],
        ) {
            Err(error) => assert_eq!(error, "native record-temp WITH: 'Pt' does not fit"),
            Ok(_) => panic!("a 16-byte record must not be built in an 8-byte region"),
        }
    });
}

#[test]
fn only_a_collection_field_is_a_growing_last_sub_block() {
    with_builder(|builder| {
        let bag = ParameterType::declared("Bag");
        assert!(builder.record_collection_last_inlined(&bag, "n").is_none());
        assert_eq!(
            builder
                .record_collection_last_inlined(&bag, "xs")
                .map(|(index, _)| index),
            Some(1)
        );
    });
}
