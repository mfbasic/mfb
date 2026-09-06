use super::super::local_rewrites::testutil::*;
use super::*;
use crate::optimizer::{with_opt_level, OptLevel};
use std::collections::HashMap;

fn function(body: Vec<NirOp>) -> NirFunction {
    NirFunction {
        name: "f".to_string(),
        visibility: "private".to_string(),
        kind: "function".to_string(),
        isolated: false,
        params: vec![],
        returns: ParameterType::Integer,
        body,
        file: "main.mfb".to_string(),
        resource_owners: HashMap::new(),
    }
}

fn bind(name: &str, type_: ParameterType, value: Option<NirValue>) -> NirOp {
    NirOp::Bind {
        mutable: false,
        name: name.to_string(),
        type_,
        value,
    }
}

fn run(body: Vec<NirOp>, level: u8) -> (Vec<NirOp>, u64) {
    let before = count_ops(&body);
    let mut module = test_module(vec![function(body)]);
    with_opt_level(OptLevel(level), || eliminate(&mut module));
    let body = module.functions.remove(0).body;
    let removed = before - count_ops(&body);
    (body, removed)
}

/// Recursive op count, so `run` can report how many ops a sweep dropped
/// without reading the process-global stats.
fn count_ops(ops: &[NirOp]) -> u64 {
    ops.iter()
        .map(|op| {
            1 + match op {
                NirOp::If {
                    then_body,
                    else_body,
                    ..
                } => count_ops(then_body) + count_ops(else_body),
                NirOp::Match { cases, .. } => cases.iter().map(|case| count_ops(&case.body)).sum(),
                NirOp::While { body, .. }
                | NirOp::For { body, .. }
                | NirOp::DoUntil { body, .. }
                | NirOp::ForEach { body, .. }
                | NirOp::Trap { body, .. } => count_ops(body),
                _ => 0,
            }
        })
        .sum()
}

/// An unused scalar bind chain dies transitively: removing `b` (only user
/// of `a`) exposes `a` as dead on the next fixpoint sweep.
#[test]
fn unused_bind_chains_die_to_a_fixpoint() {
    let (body, removed) = run(
        vec![
            bind("a", ParameterType::Integer, Some(int_const("1"))),
            bind("b", ParameterType::Integer, Some(local("a"))),
            bind("keep", ParameterType::Integer, Some(int_const("2"))),
            NirOp::Return {
                value: Some(local("keep")),
            },
        ],
        2,
    );
    assert_eq!(removed, 2);
    assert_eq!(body.len(), 2, "only `keep` and the return survive");
}

/// The trap gate: an unused bind whose initializer is checked arithmetic
/// (can raise ErrOverflow) or a Float computation (the bind is the
/// observation boundary) must stay; so must non-scalar types and anything
/// read, assigned, or resource-owning.
#[test]
fn trapping_effectful_or_used_binds_stay() {
    let trapping = bind(
        "t",
        ParameterType::Integer,
        Some(binary(BinaryOp::Add, local("keep"), local("keep"))),
    );
    let float_boundary = bind(
        "fb",
        ParameterType::Float,
        Some(binary(BinaryOp::Divide, local("g"), local("h"))),
    );
    let non_scalar = bind("s", ParameterType::String, Some(local("other")));
    let assigned = bind("m", ParameterType::Integer, Some(int_const("0")));
    let (body, removed) = run(
        vec![
            trapping,
            float_boundary,
            non_scalar,
            assigned,
            NirOp::Assign {
                name: "m".to_string(),
                value: int_const("5"),
            },
            bind("keep", ParameterType::Integer, Some(int_const("2"))),
            NirOp::Return {
                value: Some(local("keep")),
            },
        ],
        2,
    );
    assert_eq!(removed, 0);
    assert_eq!(body.len(), 7);
}

/// Pure comparisons/logic are removable; nested bodies are swept; a pure
/// `Eval` dies.
#[test]
fn pure_comparisons_and_evals_die_in_nested_bodies() {
    let (body, removed) = run(
        vec![
            bind("keep", ParameterType::Integer, Some(int_const("2"))),
            NirOp::If {
                condition: local("keep"),
                then_body: vec![
                    bind(
                        "cmp",
                        ParameterType::Boolean,
                        Some(binary(BinaryOp::Less, local("keep"), int_const("3"))),
                    ),
                    NirOp::Eval {
                        value: unary(UnaryOp::Not, local("keep")),
                    },
                    NirOp::Return {
                        value: Some(local("keep")),
                    },
                ],
                else_body: vec![],
            },
            NirOp::Return {
                value: Some(local("keep")),
            },
        ],
        2,
    );
    assert_eq!(removed, 2);
    let NirOp::If { then_body, .. } = &body[1] else {
        panic!("expected If");
    };
    assert_eq!(then_body.len(), 1, "only the return survives in the arm");
}

/// A shadowing rebind shares its name with the outer binding, so the
/// scope-blind census keeps both — conservative by design.
#[test]
fn shadowed_names_are_never_removed() {
    let (body, removed) = run(
        vec![
            bind("x", ParameterType::Integer, Some(int_const("1"))),
            NirOp::If {
                condition: local("c"),
                then_body: vec![bind("x", ParameterType::Integer, Some(int_const("2")))],
                else_body: vec![],
            },
        ],
        2,
    );
    assert_eq!(removed, 0);
    assert_eq!(body.len(), 2);
}

/// Level gating: the row is off at `-O1`.
#[test]
fn level_one_disables_the_row() {
    let (body, removed) = run(
        vec![bind("dead", ParameterType::Integer, Some(int_const("1")))],
        1,
    );
    assert_eq!(removed, 0);
    assert_eq!(body.len(), 1);
}
