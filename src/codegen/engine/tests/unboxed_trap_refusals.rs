//! The unboxed inline-`TRAP` temporary (`error/result/unboxed_trap.rs`, bug-778)
//! refuses or declines what breaks its scan/lowering agreement, on a builder
//! directly.
//!
//! The candidate scan (`unboxed_trap_result_candidates`) and the lowering read the
//! same function body, so in a lowered program they agree: a candidate is bound to
//! a trapped producer, that producer fills the slots, and a `ResultError` read is
//! counted before it is lowered. Each step still checks, and refuses by name or
//! keeps the heap `Result`, when the premise fails — the alternative is a tag read
//! from a slot nothing wrote, or a payload slot handed out as an `Error`.

use crate::codegen::engine::builder::{CodeBuilder, UnboxedTrapResult, UnboxedTrapUses};
use crate::codegen::engine::tests::test_support::TestPlatform;
use crate::codegen::memory::arena::graph_copy::tests::{harness, model};
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;

fn with_builder<R>(body: impl FnOnce(&mut CodeBuilder<'_>) -> R) -> R {
    let model = model(Vec::new());
    let harness = harness(&model);
    let platform = TestPlatform;
    let mut builder = harness.builder("unboxed", &platform);
    body(&mut builder)
}

fn int(value: &str) -> NirValue {
    NirValue::Const {
        type_: ParameterType::Integer,
        value: value.to_string(),
    }
}

fn result_of_integer() -> ParameterType {
    ParameterType::ResultOf(Box::new(ParameterType::Integer))
}

/// A candidate keeps the heap `Result` unless its value is a trapped producer.
#[test]
fn a_candidate_bound_to_something_else_keeps_the_heap_result() {
    with_builder(|builder| {
        let uses = UnboxedTrapUses {
            reads_value: true,
            reads_error: false,
        };
        builder
            .unboxed_trap_candidates
            .insert("t".to_string(), uses);
        assert!(builder
            .unboxed_trap_shape("t", &result_of_integer(), &int("1"))
            .is_none());
        assert!(
            builder
                .unboxed_trap_shape("t", &ParameterType::Integer, &int("1"))
                .is_none(),
            "nor when the binding is not a Result"
        );
    });
}

/// The unboxed bind arms its slots and lets the producer fill them; a value that
/// does not fill them is refused rather than leaving the tag slot unwritten.
#[test]
fn a_producer_that_does_not_fill_the_slots_is_refused() {
    with_builder(|builder| {
        let slot = builder.allocate_stack_object("t", 8);
        let uses = UnboxedTrapUses {
            reads_value: true,
            reads_error: false,
        };
        match builder.lower_unboxed_trap_bind(
            "t",
            &int("1"),
            slot,
            uses,
            ParameterType::Integer,
            false,
        ) {
            Err(error) => assert!(
                error.starts_with(
                    "internal: the trapped producer of unboxed inline-TRAP temporary 't' did \
                     not fill its slots"
                ),
                "{error}"
            ),
            Ok(()) => panic!("a constant fills no slots"),
        }
    });
}

/// A reader names an unboxed temporary only through a `Local`; and a `ResultError`
/// the scan did not count (so no error slot was armed) is refused rather than
/// handed the payload slot.
#[test]
fn an_uncounted_error_read_is_refused() {
    with_builder(|builder| {
        assert!(builder.lower_unboxed_result_is_ok(&int("1")).is_none());
        let tag_slot = builder.allocate_stack_object("tag", 8);
        let payload_slot = builder.allocate_stack_object("payload", 8);
        builder.unboxed_trap_locals.insert(
            "t".to_string(),
            UnboxedTrapResult {
                tag_slot,
                payload_slot,
                error_slot: payload_slot,
                payload: ParameterType::Integer,
                block: false,
                error_read: false,
            },
        );
        // The reader is handed the `ResultError`'s operand, the temporary itself.
        let read = NirValue::Local("t".to_string());
        match builder.lower_unboxed_result_error(&read) {
            Some(Err(error)) => assert_eq!(
                error,
                "internal: an unboxed inline-TRAP temporary is read by a `ResultError` its \
                 candidate scan did not count"
            ),
            Some(Ok(_)) => panic!("an uncounted error read must be refused"),
            None => panic!("`t` is an unboxed temporary"),
        }
    });
}
