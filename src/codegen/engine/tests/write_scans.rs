//! The two write scans of `function_lowering/promotion_analysis.rs` that gate
//! plan-142-F's entry snapshot and plan-145-H's `STATE` in-place stores: a local
//! (or a handle's `STATE`) counts as written by an assignment (a `STATE` store)
//! anywhere in the ops, AND by a by-reference capture of it — a callback holding
//! the reference may write it later — and by nothing else.

use crate::codegen::engine::function::function_lowering::{ops_write_local, ops_write_state};
use crate::target::shared::nir::{NirOp, NirSourceLoc, NirValue};
use crate::types::ParameterType;

fn by_ref(name: &str) -> NirOp {
    NirOp::Eval {
        value: NirValue::Call {
            target: "collections.forEach".to_string(),
            args: vec![NirValue::LocalRef {
                name: name.to_string(),
                type_: ParameterType::Integer,
            }],
            loc: NirSourceLoc::default(),
        },
    }
}

fn read(name: &str) -> NirOp {
    NirOp::Eval {
        value: NirValue::Local(name.to_string()),
    }
}

#[test]
fn a_local_is_written_by_an_assignment_or_a_by_reference_capture() {
    let assign = NirOp::Assign {
        name: "x".to_string(),
        value: NirValue::Const {
            type_: ParameterType::Integer,
            value: "1".to_string(),
        },
    };
    assert!(ops_write_local(std::slice::from_ref(&assign), "x"));
    assert!(ops_write_local(&[by_ref("x")], "x"));
    assert!(!ops_write_local(&[by_ref("y"), read("x")], "x"));
}

#[test]
fn a_state_is_written_by_a_state_store_or_a_by_reference_capture_of_its_handle() {
    let store = NirOp::StateAssign {
        resource: "h".to_string(),
        value: NirValue::Local("v".to_string()),
    };
    assert!(ops_write_state(std::slice::from_ref(&store), "h"));
    assert!(ops_write_state(&[by_ref("h")], "h"));
    assert!(!ops_write_state(&[by_ref("g"), read("h")], "h"));
}
