//! The float observation predicate treats a `TRAP`'d builtin call as its plain twin,
//! and never observes a `Checked` capture again.

use crate::codegen::builtins::math::gen_math::float_arith_node;
use crate::operators::BinaryOp;
use crate::target::shared::nir::{NirSourceLoc, NirValue};
use crate::types::ParameterType;

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
