//! The Opt1 local-rewrite driver (`optimizer::opt1::local_rewrites`) on the
//! leaf shapes whose type it reads without a scope lookup — a global, a
//! closure capture, and a constant — which gate the algebraic identities and
//! strength reduction (`scopes_type_is`: only a leaf of the expected type may
//! be dropped or duplicated).
//!
//! - `g + 0` over an Integer global is `g`;
//! - `c * 2` over an Integer capture is `c + c`;
//! - `MAX * 2` cannot fold (it overflows, and the fold must not hide the
//!   trap), so strength reduction takes it — to `MAX + MAX`, which raises the
//!   same overflow.

use super::nir_build::*;
use crate::operators::BinaryOp;
use crate::target::shared::nir::{NirOp, NirValue};
use crate::types::ParameterType;

fn shape(value: &NirValue) -> String {
    match value {
        NirValue::Const { value, .. } => value.clone(),
        NirValue::Global { name, .. } => format!("global {name}"),
        NirValue::Capture { index, .. } => format!("capture {index}"),
        NirValue::Binary {
            op, left, right, ..
        } => format!("({} {} {})", shape(left), op.name(), shape(right)),
        _ => "other".to_string(),
    }
}

#[test]
fn leaf_types_read_without_a_scope_gate_the_rewrites() {
    let global = NirValue::Global {
        name: "g".to_string(),
        type_: ParameterType::Integer,
    };
    let capture = NirValue::Capture {
        index: 0,
        type_: ParameterType::Integer,
        by_ref: false,
    };
    let body = run_row(
        1,
        vec![
            NirOp::Eval {
                value: binary(BinaryOp::Add, global, int_const("0")),
            },
            NirOp::Eval {
                value: binary(BinaryOp::Multiply, capture, int_const("2")),
            },
            NirOp::Return {
                value: Some(binary(
                    BinaryOp::Multiply,
                    int_const("9223372036854775807"),
                    int_const("2"),
                )),
            },
        ],
        crate::optimizer::opt1::local_rewrites::apply,
    );
    let shapes: Vec<String> = body
        .iter()
        .map(|op| match op {
            NirOp::Eval { value } | NirOp::Return { value: Some(value) } => shape(value),
            _ => "other".to_string(),
        })
        .collect();
    assert_eq!(
        shapes,
        [
            "global g".to_string(),
            "(capture 0 + capture 0)".to_string(),
            "(9223372036854775807 + 9223372036854775807)".to_string(),
        ]
    );
}
