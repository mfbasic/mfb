//! The arithmetic lowerings (`engine/operators/builder_numeric/operator_lowering.rs`)
//! driven directly: plan-39 I1's overflow-check elision fires only for the exact
//! shape its proof covers, and unary `-` refuses an operand it has no negation for.

use crate::codegen::engine::builder::{CodeBuilder, LocalValue, ValueResult};
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::tests::test_support::{BuilderHarness, TestPlatform};
use crate::operators::BinaryOp;
use crate::target::shared::abi;
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;

fn constant(type_: ParameterType, value: &str) -> NirValue {
    NirValue::Const {
        type_,
        value: value.to_string(),
    }
}

/// Bind an `Integer` local `i` on `builder`.
fn bind_i(builder: &mut CodeBuilder<'_>) {
    let stack_offset = builder.allocate_stack_object("i", 8);
    builder.locals.insert(
        "i".to_string(),
        LocalValue {
            type_: ParameterType::Integer,
            stack_offset,
            constant: None,
            by_ref: false,
        },
    );
}

/// The instructions `i <op> right` lowers to, with `i` proven `>= 10` when `bounded`
/// and proven `< S` for some `S` when `below`.
fn lowered(op: BinaryOp, right: NirValue, bounded: bool, below: bool) -> usize {
    let mut harness = BuilderHarness::default();
    // The overflow exit's message, which the plan would have registered.
    harness.string_symbols.insert(
        "Arithmetic overflow or numeric conversion outside the destination range.".to_string(),
        "_mfb_str_overflow".to_string(),
    );
    let mut builder = harness.builder("arithmetic_forms", &TestPlatform);
    bind_i(&mut builder);
    if bounded {
        builder.integer_lower_bounds.insert("i".to_string(), 10);
    }
    if below {
        builder.integer_strict_upper.insert("i".to_string());
    }
    let start = builder.instructions.len();
    builder
        .lower_arithmetic_binary(op, &NirValue::Local("i".to_string()), &right)
        .expect("an Integer operation lowers");
    builder.instructions.len() - start
}

/// `i - c` drops its overflow check only for a strictly positive `Integer`
/// constant `c` under a proven lower bound; a `Byte` constant, an unparsable one,
/// or no bound at all keep the check — the fail-closed side.
#[test]
fn a_subtraction_drops_its_check_only_for_a_positive_integer_constant() {
    let one = || constant(ParameterType::Integer, "1");
    let checked = lowered(BinaryOp::Subtract, one(), false, false);
    let elided = lowered(BinaryOp::Subtract, one(), true, false);
    assert!(elided < checked, "the proven subtraction drops its check");
    for right in [
        constant(ParameterType::Byte, "1"),
        constant(ParameterType::Integer, "one"),
        constant(ParameterType::Integer, "-1"),
    ] {
        assert_eq!(lowered(BinaryOp::Subtract, right, true, false), checked);
    }
}

/// `i + 1` drops its check only when `i` is proven below some bound.
#[test]
fn an_increment_drops_its_check_only_below_a_proven_bound() {
    let one = || constant(ParameterType::Integer, "1");
    let checked = lowered(BinaryOp::Add, one(), false, false);
    assert!(lowered(BinaryOp::Add, one(), false, true) < checked);
    assert_eq!(
        lowered(
            BinaryOp::Add,
            constant(ParameterType::Integer, "2"),
            false,
            true
        ),
        checked
    );
}

/// Unary `-` negates `Byte`, `Integer`, `Fixed`, `Money` and `Float`; any other
/// operand is refused naming its type rather than negated as an integer.
#[test]
fn unary_minus_refuses_an_operand_it_cannot_negate() {
    let harness = BuilderHarness::default();
    let mut builder = harness.builder("arithmetic_forms", &TestPlatform);
    let flag = ValueResult {
        origin: None,
        type_: ParameterType::Boolean,
        location: Operand::from(abi::c_arg(0)),
        text: "flag".to_string(),
    };
    assert_eq!(
        builder.lower_numeric_unary_negation(flag).err().as_deref(),
        Some("native code plan does not lower unary operator '-' for Boolean")
    );
}
