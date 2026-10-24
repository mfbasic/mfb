//! A `RETURN` that hands a block to the caller instead of copying it
//! (`engine/control/builder_exits.rs`), and when it declines — on a builder directly.
//!
//! * plan-147-B: `RETURN <local>` moves only a binding that owns its block. An
//!   unbound name and a by-reference local (a slot holding another binding's
//!   address) own nothing, so the move is not admitted and the return copies;
//!   returning such a pointer uncopied would let two owners free one block.
//! * bug-784: `RETURN WITH p { … }` writes into the caller's destination only in a
//!   destination variant, and only when the `WITH` rebuilds that parameter.
//!   Anything else emits nothing and leaves the ordinary return to stand.

use crate::codegen::engine::builder::{ActiveCleanup, CodeBuilder, LocalValue, OwnedValueCleanup};
use crate::codegen::engine::tests::test_support::TestPlatform;
use crate::codegen::memory::arena::graph_copy::tests::{field, harness, model, record};
use crate::target::shared::nir::{NirRecordUpdate, NirValue};
use crate::types::ParameterType;

fn with_builder<R>(body: impl FnOnce(&mut CodeBuilder<'_>) -> R) -> R {
    let model = model(vec![record("Pt", vec![field("x", "Integer")])]);
    let harness = harness(&model);
    let platform = TestPlatform;
    let mut builder = harness.builder("returns", &platform);
    body(&mut builder)
}

fn bind(builder: &mut CodeBuilder<'_>, name: &str, by_ref: bool) -> usize {
    let stack_offset = builder.allocate_stack_object(name, 8);
    builder.locals.insert(
        name.to_string(),
        LocalValue {
            type_: ParameterType::declared("Pt"),
            stack_offset,
            constant: None,
            by_ref,
        },
    );
    stack_offset
}

fn owned(builder: &mut CodeBuilder<'_>, slot: usize) {
    builder
        .active_cleanups
        .push(ActiveCleanup::OwnedValue(OwnedValueCleanup {
            type_: ParameterType::declared("Pt"),
            stack_offset: slot,
            capacity_slot: None,
            loop_alias_slot: None,
            result_wrapper: None,
        }));
}

#[test]
fn a_return_moves_only_a_binding_that_owns_its_block() {
    with_builder(|builder| {
        assert!(!builder.returned_move_admits("ghost"), "an unbound name");
        let slot = bind(builder, "p", true);
        owned(builder, slot);
        assert!(
            !builder.returned_move_admits("p"),
            "a by-reference local holds another binding's slot"
        );
        builder.locals.get_mut("p").expect("bound").by_ref = false;
        assert!(
            builder.returned_move_admits("p"),
            "an owning local at its return moves"
        );
    });
}

fn with_over(target: &str) -> NirValue {
    NirValue::WithUpdate {
        type_: ParameterType::declared("Pt"),
        target: Box::new(NirValue::Local(target.to_string())),
        updates: vec![NirRecordUpdate {
            field: "x".to_string(),
            value: NirValue::Const {
                type_: ParameterType::Integer,
                value: "1".to_string(),
            },
        }],
    }
}

#[test]
fn a_destination_return_declines_anything_but_a_with_over_its_parameter() {
    with_builder(|builder| {
        let declined = |builder: &mut CodeBuilder<'_>, value: &NirValue| {
            let before = builder.instructions.len();
            let handled = builder
                .try_into_return(value, None)
                .expect("a decline is not an error");
            assert!(!handled, "declined");
            assert_eq!(builder.instructions.len(), before, "and emitted nothing");
        };
        // Not a destination variant at all.
        declined(builder, &with_over("p"));
        builder.element_updates.into_param = Some("p".to_string());
        // Not a `WITH`.
        declined(builder, &NirValue::Local("p".to_string()));
        // The destination parameter, but not bound in this frame.
        declined(builder, &with_over("p"));
        // With `p` bound, a `WITH` over something other than it.
        bind(builder, "p", false);
        bind(builder, "q", false);
        declined(builder, &with_over("q"));
    });
}
