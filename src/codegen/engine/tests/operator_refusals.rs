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

/// The three arithmetic emitters are generic over their destination, and the
/// register-allocated form (a borrowed virtual register, as every builder-lowered
/// expression passes it) refuses exactly what the fixed-register form above does.
#[test]
fn a_virtual_register_destination_refuses_the_same_operators() {
    let integer = refused_by(ParameterType::Integer, |b, op, l, r| {
        let dst = b.temporary_vreg();
        b.emit_integer_binary(op, l, r, &dst, false)
    });
    assert_eq!(integer.len(), 10);
    let fixed = refused_by(ParameterType::Fixed, |b, op, l, r| {
        let dst = b.temporary_vreg();
        b.emit_fixed_binary(op, l, r, &dst)
    });
    assert_eq!(fixed.len(), 11);
    let float = refused_by(ParameterType::Float, |b, op, l, r| {
        let dst = b.temporary_vreg();
        b.emit_float_binary(op, l, r, &dst).map(|_| ())
    });
    assert_eq!(float.len(), 10);
    for (kind, refused) in [("integer", integer), ("Fixed", fixed), ("Float", float)] {
        for (op, error) in refused {
            assert_eq!(
                error,
                format!(
                    "native code plan does not lower {kind} operator '{}'",
                    op.name()
                )
            );
        }
    }
}

/// Money arithmetic has `+ - * / MOD DIV`; plan-29-A's dimensional check rejects
/// every other operator before lowering, and the emitter refuses each by name.
#[test]
fn money_arithmetic_refuses_the_operators_it_has_no_form_for() {
    let harness = BuilderHarness::default();
    let mut refused = Vec::new();
    for op in ALL {
        let mut builder = harness.builder("operator_refusals_money", &TestPlatform);
        let left = operand(ParameterType::Money, 0);
        let right = operand(ParameterType::Money, 1);
        let dst = builder.temporary_vreg();
        if let Err(error) = builder.emit_money_binary(op, &left, &right, &dst) {
            if error.starts_with("native code plan cannot lower Money operator") {
                refused.push(op);
            }
        }
    }
    assert_eq!(
        refused,
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
            BinaryOp::Power,
        ]
    );
}

/// `Money * x` and `Money / x` scale by an `Integer`, `Byte`, `Fixed` or `Float`;
/// any other factor is refused naming its type.
#[test]
fn money_scaling_refuses_a_factor_that_is_not_numeric() {
    let harness = BuilderHarness::default();
    for (op, verb) in [(BinaryOp::Multiply, "scale"), (BinaryOp::Divide, "divide")] {
        let mut builder = harness.builder("operator_refusals_money", &TestPlatform);
        let money = operand(ParameterType::Money, 0);
        let flag = operand(ParameterType::Boolean, 1);
        let dst = builder.temporary_vreg();
        assert_eq!(
            builder.emit_money_binary(op, &money, &flag, &dst).err(),
            Some(format!(
                "native code plan cannot {verb} Money by operand type 'Boolean'"
            ))
        );
    }
}

/// Only the six comparisons produce a `Boolean`; `ir::verify` keeps every other
/// operator off the comparison path, and both of its lowerings — the numeric one
/// and the bitwise-equal one — refuse the rest by name.
#[test]
fn a_comparison_lowering_refuses_a_non_comparison_operator() {
    let harness = BuilderHarness::default();
    let integer = NirValue::Const {
        type_: ParameterType::Integer,
        value: "4".to_string(),
    };
    let yes = NirValue::Const {
        type_: ParameterType::Boolean,
        value: "true".to_string(),
    };
    for operand in [&integer, &yes] {
        for op in [BinaryOp::Add, BinaryOp::Power, BinaryOp::Concat] {
            let mut builder = harness.builder("operator_refusals_compare", &TestPlatform);
            assert_eq!(
                builder.lower_comparison_binary(op, operand, operand).err(),
                Some(format!(
                    "native code plan does not lower comparison operator '{}'",
                    op.name()
                )),
            );
        }
    }
}
