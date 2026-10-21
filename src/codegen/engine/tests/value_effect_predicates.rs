//! plan-145-C's value-effect predicates (`builder_control/value_effects.rs`),
//! asked directly of a builder that knows two locals.
//!
//! `nir_value_is_effect_free` and `nir_value_cannot_fail` decide whether a field
//! arm or an element update may run out of order or be skipped, so each answers
//! `true` only for what it has proved: reads and operators over them, and calls
//! of infallible built-ins over arguments whose types it knows.

use crate::codegen::engine::builder::LocalValue;
use crate::codegen::engine::tests::test_support::{BuilderHarness, TestPlatform};
use crate::operators::{BinaryOp, UnaryOp};
use crate::target::shared::nir::{NirSourceLoc, NirValue};
use crate::types::ParameterType;

fn local(name: &str) -> NirValue {
    NirValue::Local(name.to_string())
}

fn unary(op: UnaryOp, operand: NirValue) -> NirValue {
    NirValue::Unary {
        op,
        operand: Box::new(operand),
        loc: NirSourceLoc::default(),
    }
}

fn len_of(arg: NirValue) -> NirValue {
    NirValue::Call {
        target: "len".to_string(),
        args: vec![arg],
        loc: NirSourceLoc::default(),
    }
}

#[test]
fn effects_and_failures_are_answered_from_proof_only() {
    let harness = BuilderHarness::default();
    let mut builder = harness.builder("probe", &TestPlatform);
    for (name, type_) in [
        (
            "xs",
            ParameterType::ListOf(Box::new(ParameterType::Integer)),
        ),
        ("n", ParameterType::Integer),
        ("flag", ParameterType::Boolean),
    ] {
        builder.locals.insert(
            name.to_string(),
            LocalValue {
                type_,
                stack_offset: 0,
                constant: None,
                by_ref: false,
            },
        );
    }
    // A negation reads and computes: no effect, but it can overflow (the minimum
    // `Integer`), so it can fail; a logical NOT cannot.
    let negated = unary(UnaryOp::Negate, local("n"));
    assert!(builder.nir_value_is_effect_free(&negated));
    assert!(!builder.nir_value_cannot_fail(&negated));
    assert!(builder.nir_value_cannot_fail(&unary(UnaryOp::Not, local("flag"))));
    // `len` of a typed local is an infallible built-in over a known type.
    assert!(builder.nir_value_is_effect_free(&len_of(local("xs"))));
    assert!(builder.nir_value_cannot_fail(&len_of(local("xs"))));
    // Over a local nothing types, the census has no key to look up: unproven.
    assert!(!builder.nir_value_is_effect_free(&len_of(local("unknown"))));
    assert!(!builder.nir_value_cannot_fail(&len_of(local("unknown"))));
    // A value kind neither predicate knows (here a literal, which allocates) is
    // neither effect-free nor infallible.
    let literal = NirValue::ListLiteral {
        type_: ParameterType::ListOf(Box::new(ParameterType::Integer)),
        values: vec![local("n")],
    };
    assert!(!builder.nir_value_is_effect_free(&literal));
    assert!(!builder.nir_value_cannot_fail(&literal));
    // Arithmetic can overflow; a comparison of the same operands cannot fail.
    let add = |op| NirValue::Binary {
        op,
        left: Box::new(local("n")),
        right: Box::new(local("n")),
        loc: NirSourceLoc::default(),
    };
    assert!(!builder.nir_value_cannot_fail(&add(BinaryOp::Add)));
    assert!(builder.nir_value_cannot_fail(&add(BinaryOp::Less)));
}
