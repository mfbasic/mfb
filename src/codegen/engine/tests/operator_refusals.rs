//! The arithmetic emitters (`engine/operators/builder_numeric/**`) refuse an operator
//! their operand type has no instruction sequence for — `&` on `Integer`, `AND` on
//! `Fixed` — by naming it, instead of falling through to an arm that emits the wrong
//! arithmetic. `ir::verify` keeps such a node out of every real program, so each is
//! driven directly with the whole operator table.

use crate::codegen::engine::builder::ValueResult;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::tests::test_support::{BuilderHarness, TestPlatform};
use crate::operators::BinaryOp;
use crate::target::shared::abi;
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;

const ALL: [BinaryOp; 17] = [
    BinaryOp::Or,
    BinaryOp::Xor,
    BinaryOp::And,
    BinaryOp::Equal,
    BinaryOp::NotEqual,
    BinaryOp::Less,
    BinaryOp::LessEqual,
    BinaryOp::Greater,
    BinaryOp::GreaterEqual,
    BinaryOp::Concat,
    BinaryOp::Add,
    BinaryOp::Subtract,
    BinaryOp::Multiply,
    BinaryOp::Divide,
    BinaryOp::Mod,
    BinaryOp::IntDiv,
    BinaryOp::Power,
];

fn operand(type_: ParameterType, register: usize) -> ValueResult {
    ValueResult {
        origin: None,
        type_,
        location: Operand::from(abi::c_arg(register)),
        text: format!("operand{register}"),
    }
}

/// The operators of [`ALL`] each emitter refuses, and the refusal it gives.
fn refused_by(
    type_: ParameterType,
    emit: impl Fn(
        &mut crate::codegen::engine::builder::CodeBuilder<'_>,
        BinaryOp,
        &ValueResult,
        &ValueResult,
    ) -> Result<(), String>,
) -> Vec<(BinaryOp, String)> {
    let harness = BuilderHarness::default();
    let mut refused = Vec::new();
    for op in ALL {
        let mut builder = harness.builder("operator_refusals", &TestPlatform);
        let left = operand(type_.clone(), 0);
        let right = operand(type_.clone(), 1);
        // An arithmetic operator may fail for want of the error-message data
        // object the plan would have registered; only the operator refusal is the
        // contract here.
        if let Err(error) = emit(&mut builder, op, &left, &right) {
            if error.starts_with("native code plan does not lower") {
                refused.push((op, error));
            }
        }
    }
    refused
}

#[test]
fn integer_arithmetic_refuses_the_comparison_logical_and_concat_operators() {
    let refused = refused_by(ParameterType::Integer, |b, op, l, r| {
        b.emit_integer_binary(op, l, r, abi::c_arg(2), false)
    });
    let names: Vec<BinaryOp> = refused.iter().map(|(op, _)| *op).collect();
    assert_eq!(
        names,
        vec![
            BinaryOp::Or,
            BinaryOp::Xor,
            BinaryOp::And,
            BinaryOp::Equal,
            BinaryOp::NotEqual,
            BinaryOp::Less,
            BinaryOp::LessEqual,
            BinaryOp::Greater,
            BinaryOp::GreaterEqual,
            BinaryOp::Concat,
        ]
    );
    for (op, error) in refused {
        assert_eq!(
            error,
            format!(
                "native code plan does not lower integer operator '{}'",
                op.name()
            )
        );
    }
}

#[test]
fn fixed_arithmetic_refuses_what_it_has_no_sequence_for() {
    let refused = refused_by(ParameterType::Fixed, |b, op, l, r| {
        b.emit_fixed_binary(op, l, r, abi::c_arg(2))
    });
    // The ten non-arithmetic operators and the truncating integer division.
    assert_eq!(refused.len(), 11);
    for (op, error) in refused {
        assert_eq!(
            error,
            format!(
                "native code plan does not lower Fixed operator '{}'",
                op.name()
            )
        );
    }
}

#[test]
fn float_arithmetic_refuses_the_comparison_logical_and_concat_operators() {
    let refused = refused_by(ParameterType::Float, |b, op, l, r| {
        b.emit_float_binary(op, l, r, abi::c_arg(2)).map(|_| ())
    });
    assert_eq!(refused.len(), 10);
    for (op, error) in refused {
        assert_eq!(
            error,
            format!(
                "native code plan does not lower Float operator '{}'",
                op.name()
            )
        );
    }
}

#[test]
fn a_boolean_pair_takes_only_and_or_xor() {
    let harness = BuilderHarness::default();
    let yes = NirValue::Const {
        type_: ParameterType::Boolean,
        value: "true".to_string(),
    };
    for op in ALL {
        let mut builder = harness.builder("operator_refusals_boolean", &TestPlatform);
        let result = builder.lower_boolean_binary(op, &yes, &yes);
        match op {
            BinaryOp::And | BinaryOp::Or | BinaryOp::Xor => {
                assert!(result.is_ok(), "{}", op.name())
            }
            _ => assert_eq!(
                result.err().as_deref(),
                Some(
                    format!(
                        "native code plan does not lower boolean operator '{}'",
                        op.name()
                    )
                    .as_str()
                )
            ),
        }
    }
}
