//! The scalar `math::` lowerings refuse an operand type they have no kernel for, by
//! name, and the float observation predicate treats a `TRAP`'d call as its plain twin.
//!
//! `CodeBuilder::lower_math_call` routes by member and arity, then each scalar
//! lowering (`math/gen_math/scalar.rs`) dispatches on the operand's type: `Float`
//! to the shared kernels, `Integer`/`Fixed` to their own paths. The checker admits
//! nothing else, so the last arm — "math.X does not accept T" — is the lowering's
//! own guard against a caller that lets a wrong type through. Driven directly here
//! with a `String` operand: the refusal must name the member and the type, rather
//! than emit a kernel over a string pointer.

use crate::codegen::builtins::math::gen_math::float_arith_node;
use crate::codegen::engine::builder::ValueResult;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::tests::test_support::{BuilderHarness, TestPlatform};
use crate::operators::BinaryOp;
use crate::target::shared::nir::{NirSourceLoc, NirValue};
use crate::types::ParameterType;

fn refusal(member: &str, arity: usize, type_: ParameterType) -> String {
    refusal_of(member, &vec![type_; arity])
}

/// `lower_math_call(member, ...)` with one operand of each of `types`, which must be
/// refused.
fn refusal_of(member: &str, types: &[ParameterType]) -> String {
    let platform = TestPlatform;
    let harness = BuilderHarness::default();
    let mut builder = harness.builder("_mfb_probe", &platform);
    let args: Vec<ValueResult> = types
        .iter()
        .enumerate()
        .map(|(index, type_)| ValueResult {
            origin: None,
            type_: type_.clone(),
            location: Operand::from(builder.temporary_vreg()),
            text: format!("operand{index}"),
        })
        .collect();
    match builder.lower_math_call(member, &args) {
        Ok(value) => panic!("math.{member} lowered operands {types:?}: {}", value.text),
        Err(err) => err,
    }
}

#[test]
fn a_scalar_math_member_refuses_an_operand_type_it_has_no_kernel_for() {
    for (member, arity, want) in [
        ("atan2", 2, "math.atan2 does not accept String"),
        ("min", 2, "math.min does not accept String"),
        ("max", 2, "math.max does not accept String"),
        ("clamp", 3, "math.clamp does not accept String"),
    ] {
        let err = refusal(member, arity, ParameterType::String);
        assert_eq!(err, want, "math.{member}");
    }
}

#[test]
fn a_trapped_builtin_call_is_observed_like_the_plain_call_and_a_checked_value_is_not() {
    let loc = NirSourceLoc::default();
    let call = |target: &str| NirValue::Call {
        target: target.to_string(),
        args: Vec::new(),
        loc,
    };
    let trapped = |target: &str| NirValue::CallResult {
        target: target.to_string(),
        args: Vec::new(),
        loc,
    };
    // Every `Float`-returning builtin is classified finite (`float_result.rs`):
    // `collections::sum` checks its own total since bug-754, `toFloat` raises its
    // overflow at the call, `math::sqrt` is constrained by its kernel. The TRAP twin
    // of each must answer exactly as the plain call does — a boundary that re-checked
    // only one of them would trap where the other returns.
    for target in ["collections.sum", "toFloat", "math.sqrt"] {
        assert!(
            !float_arith_node(&call(target)),
            "{target}: a classified builtin's result is finite"
        );
        assert!(
            !float_arith_node(&trapped(target)),
            "{target}: the TRAP twin answers as the plain call does"
        );
    }
    // A `Checked` capture observes its wrapped value itself, inside the capture;
    // observing it again outside would trap where the program asked to catch. So a
    // `Checked` around float arithmetic answers false while the arithmetic itself
    // answers true.
    let one = || NirValue::Const {
        type_: ParameterType::Float,
        value: "1.0".to_string(),
    };
    let quotient = NirValue::Binary {
        op: BinaryOp::Divide,
        left: Box::new(one()),
        right: Box::new(one()),
        loc,
    };
    assert!(float_arith_node(&quotient), "float arithmetic is observed");
    let checked = NirValue::Checked {
        type_: ParameterType::Float,
        value: Box::new(quotient),
        observe: true,
    };
    assert!(
        !float_arith_node(&checked),
        "a Checked capture is not observed again"
    );
}
