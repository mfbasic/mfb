//! Recovery-region simplification (`optimizer::opt1::recovery`) over a
//! guarded region that uses every statement kind, and the same region with one
//! fallible value hidden in a `MATCH` guard.
//!
//! The row's contract: a function-level `TRAP` is dropped exactly when nothing
//! in the region it guards — at any depth, in any statement's own values or a
//! case's guard — can enter the error path. Dropping it when a guard can raise
//! would leave that raise with no handler.

use super::nir_build::*;
use crate::ast::LoopKind;
use crate::operators::BinaryOp;
use crate::target::shared::nir::{NirMatchCase, NirMatchPattern, NirOp, NirValue};
use crate::types::ParameterType;

fn region(guard: NirValue) -> Vec<NirOp> {
    vec![
        bind("x", int_const("1")),
        assign("x", local("y")),
        NirOp::Eval { value: local("x") },
        NirOp::StateAssign {
            resource: "h".to_string(),
            value: local("x"),
        },
        NirOp::While {
            kind: LoopKind::While,
            condition: binary(BinaryOp::Less, local("x"), local("y")),
            body: vec![NirOp::ExitLoop {
                kind: LoopKind::While,
                target: None,
            }],
        },
        NirOp::DoUntil {
            body: vec![NirOp::ContinueLoop {
                kind: LoopKind::Do,
                target: None,
            }],
            condition: binary(BinaryOp::Less, local("y"), local("x")),
        },
        NirOp::ForEach {
            name: "item".to_string(),
            type_: ParameterType::Integer,
            iterable: local("xs"),
            body: vec![],
        },
        for_loop("i", int_const("0"), int_const("9"), int_const("1"), vec![]),
        NirOp::Match {
            value: local("x"),
            cases: vec![NirMatchCase {
                pattern: NirMatchPattern::Else,
                guard: Some(guard),
                body: vec![],
            }],
        },
        NirOp::Return {
            value: Some(local("x")),
        },
        NirOp::Trap {
            name: "e".to_string(),
            body: vec![NirOp::Return {
                value: Some(int_const("0")),
            }],
        },
    ]
}

fn simplify(body: Vec<NirOp>) -> Vec<NirOp> {
    run_row(3, body, crate::optimizer::opt1::recovery::simplify)
}

#[test]
fn a_handler_guarding_only_pure_statements_of_every_kind_is_dropped() {
    let before = region(binary(BinaryOp::Less, local("x"), local("y")));
    let count = before.len();
    let after = simplify(before);
    assert_eq!(after.len(), count - 1);
    assert!(!after.iter().any(|op| matches!(op, NirOp::Trap { .. })));
}

#[test]
fn a_fallible_match_guard_keeps_the_handler() {
    let before = region(binary(BinaryOp::Add, local("x"), local("y")));
    let count = before.len();
    let after = simplify(before);
    assert_eq!(after.len(), count, "`x + y` can overflow inside the guard");
    assert!(matches!(after.last(), Some(NirOp::Trap { .. })));
}
