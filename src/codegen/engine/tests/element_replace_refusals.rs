//! An element-bound owner's variable-width field replace
//! (`engine/control/builder_control/element_update.rs`, bug-784), driven on a builder.
//!
//! `try_inplace_scalar_fields` hands `emit_element_replace_or_write_back` an owner it
//! already found to be an element-bound record local, a record type and field index
//! it read from the model, and one spilled value per update; the write-back rebuilds
//! the `WITH` over hidden locals holding those values. Each step still checks what it
//! relies on and refuses by name — the alternative is a write-back that stores some
//! other update's value into a field, or rebuilds a record from the wrong layout.

use crate::codegen::collection::assign::self_update::FieldContainer;
use crate::codegen::engine::analysis::borrow_get::ElementBinding;
use crate::codegen::engine::builder::{CodeBuilder, LocalValue};
use crate::codegen::engine::control::builder_control::{FieldReplace, SpilledValue};
use crate::codegen::engine::tests::test_support::TestPlatform;
use crate::codegen::memory::arena::graph_copy::tests::{field, harness, model, record};
use crate::target::shared::nir::{NirOp, NirRecordUpdate, NirValue};
use crate::types::ParameterType;

fn refused<T>(result: Result<T, String>, what: &str) -> String {
    match result {
        Err(error) => error,
        Ok(_) => panic!("{what} must be refused"),
    }
}

/// `TYPE Pt { x AS Integer, name AS String }`.
fn with_builder<R>(body: impl FnOnce(&mut CodeBuilder<'_>) -> R) -> R {
    let model = model(vec![record(
        "Pt",
        vec![field("x", "Integer"), field("name", "String")],
    )]);
    let harness = harness(&model);
    let platform = TestPlatform;
    let mut builder = harness.builder("element_replace", &platform);
    body(&mut builder)
}

fn int(value: &str) -> NirValue {
    NirValue::Const {
        type_: ParameterType::Integer,
        value: value.to_string(),
    }
}

fn with(type_: &str, updates: &[(&str, NirValue)]) -> NirValue {
    NirValue::WithUpdate {
        type_: ParameterType::declared(type_),
        target: Box::new(NirValue::Local("p".to_string())),
        updates: updates
            .iter()
            .map(|(field, value)| NirRecordUpdate {
                field: field.to_string(),
                value: value.clone(),
            })
            .collect(),
    }
}

/// The write-back item: the element's `WITH` with each update value replaced by a
/// hidden local over the slot it was spilled to.
#[test]
fn the_write_back_item_refuses_a_with_it_cannot_rebuild() {
    with_builder(|builder| {
        let mut hidden = Vec::new();
        let slot = builder.allocate_stack_object("spilled", 8);
        let one = [SpilledValue::One(slot)];
        let mut item = |builder: &mut CodeBuilder<'_>, value: &NirValue, depth, spilled| {
            refused(
                builder.element_write_back_item(value, depth, spilled, &mut hidden),
                "a write-back item",
            )
        };
        assert_eq!(
            item(builder, &int("1"), 0, &one),
            "native element replace: the update is not a WITH"
        );
        assert_eq!(
            item(builder, &with("Ghost", &[("x", int("1"))]), 0, &one),
            "native element replace: 'Ghost' is not a record"
        );
        assert_eq!(
            item(
                builder,
                &with("Pt", &[("x", int("1")), ("x", int("2"))]),
                1,
                &one
            ),
            "native element replace: a path level has one update"
        );
        assert_eq!(
            item(builder, &with("Pt", &[("x", int("1"))]), 0, &[]),
            "native element replace: one spilled value per update"
        );
        assert_eq!(
            item(builder, &with("Pt", &[("nope", int("1"))]), 0, &one),
            "native element replace: no field 'nope'"
        );

        // The well-formed one: the update's value becomes a hidden local.
        let mut hidden = Vec::new();
        let rebuilt = builder
            .element_write_back_item(&with("Pt", &[("x", int("1"))]), 0, &one, &mut hidden)
            .expect("one update, one spilled value");
        assert_eq!(hidden.len(), 1);
        match rebuilt {
            NirValue::WithUpdate { updates, .. } => assert!(
                matches!(&updates[0].value, NirValue::Local(name) if *name == hidden[0]),
                "the update reads the hidden local"
            ),
            _ => panic!("the write-back item is a WITH"),
        }
    });
}

/// A nested update's `WITH r.inner { … }` keeps its target and reads each value
/// from its slot.
#[test]
fn the_nested_write_back_value_refuses_a_with_it_cannot_rebuild() {
    with_builder(|builder| {
        let mut hidden = Vec::new();
        let slot = builder.allocate_stack_object("spilled", 8);
        let mut nested = |builder: &mut CodeBuilder<'_>, value: &NirValue, slots: &[usize]| {
            refused(
                builder.element_nested_write_back_value(value, slots, &mut hidden),
                "a nested write-back value",
            )
        };
        assert_eq!(
            nested(builder, &int("1"), &[slot]),
            "native element replace: a nested update is a WITH"
        );
        assert_eq!(
            nested(builder, &with("Ghost", &[("x", int("1"))]), &[slot]),
            "native element replace: 'Ghost' is not a record"
        );
        assert_eq!(
            nested(builder, &with("Pt", &[("x", int("1"))]), &[]),
            "native element replace: one slot per nested update"
        );
        assert_eq!(
            nested(builder, &with("Pt", &[("nope", int("1"))]), &[slot]),
            "native element replace: no field 'nope'"
        );
    });
}

/// The replace itself: only an element-bound record local, of a record type, at
/// a field it has, with a binding to write back through.
#[test]
fn an_element_replace_without_its_premises_is_refused() {
    with_builder(|builder| {
        let slot = builder.allocate_stack_object("new", 8);
        let size_slot = builder.allocate_stack_object("new_size", 8);
        let replace = |index| FieldReplace {
            index,
            slot,
            size_slot,
        };
        let value = with("Pt", &[("name", NirValue::Local("s".to_string()))]);
        let pt = ParameterType::declared("Pt");
        let emit = |builder: &mut CodeBuilder<'_>,
                    owner: FieldContainer<'_>,
                    type_: &ParameterType,
                    replace: &FieldReplace| {
            refused(
                builder.emit_element_replace_or_write_back(
                    owner,
                    &value,
                    type_,
                    &[],
                    &[],
                    &[],
                    replace,
                    &[SpilledValue::One(slot)],
                ),
                "an element replace",
            )
        };
        assert_eq!(
            emit(
                builder,
                FieldContainer::Global { name: "g" },
                &pt,
                &replace(1)
            ),
            "native element replace: the owner is not a record local"
        );
        assert_eq!(
            emit(
                builder,
                FieldContainer::Record { local: "p" },
                &ParameterType::declared("Ghost"),
                &replace(1)
            ),
            "native element replace: 'Ghost' is not a record"
        );
        assert_eq!(
            emit(
                builder,
                FieldContainer::Record { local: "p" },
                &pt,
                &replace(5)
            ),
            "native element replace: no field 5"
        );
        // A bound record local with no element binding: the in-place branch emits,
        // and the size-change branch has nothing to write back through.
        let stack_offset = builder.allocate_stack_object("p", 8);
        builder.locals.insert(
            "p".to_string(),
            LocalValue {
                type_: pt.clone(),
                stack_offset,
                constant: None,
                by_ref: false,
            },
        );
        assert_eq!(
            emit(
                builder,
                FieldContainer::Record { local: "p" },
                &pt,
                &replace(1)
            ),
            "native element replace: 'p' is not element-bound"
        );
    });
}

/// The update statement itself: its binding must be one the borrow analysis
/// recorded as element-bound, and bound in this frame.
#[test]
fn an_element_update_of_an_unrecorded_binding_is_refused() {
    with_builder(|builder| {
        let value = with("Pt", &[("x", int("1"))]);
        assert_eq!(
            refused(
                builder.lower_element_update("p", None, &value),
                "an update of a binding the analysis did not record"
            ),
            "native element update: 'p' is not element-bound"
        );
        builder.element_updates.bindings.insert(
            "p".to_string(),
            ElementBinding {
                type_: ParameterType::declared("Pt"),
                template: NirOp::Assign {
                    name: "xs".to_string(),
                    value: NirValue::Local("p".to_string()),
                },
                update: None,
                write_back: None,
                other_sites: Vec::new(),
            },
        );
        assert_eq!(
            refused(
                builder.lower_element_update("p", None, &value),
                "an update of a binding this frame never bound"
            ),
            "native element update: unknown local 'p'"
        );
    });
}
