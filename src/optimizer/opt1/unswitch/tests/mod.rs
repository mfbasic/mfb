use super::super::local_rewrites::testutil::*;
use super::*;
use crate::optimizer::{with_opt_level, OptLevel};
use crate::target::shared::nir::{NirFunction, NirValue};
use crate::types::ParameterType;
use std::collections::HashMap;

fn run(body: Vec<NirOp>, level: u8) -> Vec<NirOp> {
    let function = NirFunction {
        name: "f".to_string(),
        visibility: "private".to_string(),
        kind: "function".to_string(),
        isolated: false,
        params: vec![],
        returns: ParameterType::Integer,
        body,
        file: "main.mfb".to_string(),
        resource_owners: HashMap::new(),
    };
    let mut module = test_module(vec![function]);
    with_opt_level(OptLevel(level), || unswitch(&mut module));
    module.functions.remove(0).body
}

fn assign(name: &str, value: NirValue) -> NirOp {
    NirOp::Assign {
        name: name.to_string(),
        value,
    }
}

fn while_with_if(cond: NirValue) -> Vec<NirOp> {
    vec![NirOp::While {
        kind: crate::ast::LoopKind::While,
        condition: local("c"),
        body: vec![
            assign("pre", local("x")),
            NirOp::If {
                condition: cond,
                then_body: vec![
                    assign("a", local("x")),
                    NirOp::ExitLoop {
                        target: None,
                        kind: crate::ast::LoopKind::While,
                    },
                ],
                else_body: vec![assign("b", local("x"))],
            },
            assign("post", local("x")),
        ],
    }]
}

/// An invariant IF splits the loop: one guard, two specialized copies —
/// with the surrounding statements and even an `EXIT` preserved verbatim
/// inside each same-kind copy.
#[test]
fn invariant_if_splits_the_loop() {
    let body = run(
        while_with_if(binary(BinaryOp::Less, local("p"), local("q"))),
        3,
    );
    let NirOp::If {
        then_body,
        else_body,
        ..
    } = &body[0]
    else {
        panic!("expected the unswitch guard");
    };
    let NirOp::While {
        body: then_loop, ..
    } = &then_body[0]
    else {
        panic!("then arm holds the specialized loop");
    };
    assert_eq!(then_loop.len(), 4, "pre + arm(2 ops) + post");
    assert!(matches!(then_loop[2], NirOp::ExitLoop { .. }));
    let NirOp::While {
        body: else_loop, ..
    } = &else_body[0]
    else {
        panic!("else arm holds the specialized loop");
    };
    assert_eq!(else_loop.len(), 3, "pre + arm(1 op) + post");
}

/// A condition reading a name the body assigns is variant: no unswitch.
/// So is a trap-capable condition (arithmetic can raise ErrOverflow).
#[test]
fn variant_or_trapping_conditions_stay() {
    let variant = run(
        while_with_if(binary(BinaryOp::Less, local("pre"), local("q"))),
        3,
    );
    assert!(matches!(&variant[0], NirOp::While { .. }));

    let trapping = run(
        while_with_if(binary(BinaryOp::Add, local("p"), local("q"))),
        3,
    );
    assert!(matches!(&trapping[0], NirOp::While { .. }));
}

/// bug-763: a condition reading a `MUT` the body hands to a callback by
/// reference (`LocalRef` in a closure's captures) is variant — the callee
/// writes it through the slot with no `Assign` in this body.
#[test]
fn byref_captured_condition_is_variant() {
    let mut body = while_with_if(binary(BinaryOp::Less, local("acc"), local("q")));
    let NirOp::While { body: inner, .. } = &mut body[0] else {
        unreachable!()
    };
    inner.push(NirOp::Eval {
        value: NirValue::Call {
            target: "collections.forEach".to_string(),
            args: vec![
                local("xs"),
                NirValue::Closure {
                    name: "$lambda0".to_string(),
                    type_: ParameterType::Func(
                        vec![ParameterType::Integer],
                        Box::new(ParameterType::Nothing),
                        false,
                    ),
                    captures: vec![NirValue::LocalRef {
                        name: "acc".to_string(),
                        type_: ParameterType::Integer,
                    }],
                },
            ],
            loc: Default::default(),
        },
    });
    let body = run(body, 3);
    assert!(
        matches!(&body[0], NirOp::While { .. }),
        "a by-ref captured read must not be unswitched"
    );
}

/// The loop *variable* counts as defined-per-iteration even though it is
/// no body statement: a condition reading it is variant.
#[test]
fn loop_variable_conditions_are_variant() {
    let body = run(
        vec![NirOp::For {
            name: "i".to_string(),
            type_: ParameterType::Integer,
            start: int_const("0"),
            end: int_const("9"),
            step: int_const("1"),
            body: vec![NirOp::If {
                condition: binary(BinaryOp::Less, local("i"), local("q")),
                then_body: vec![assign("a", local("x"))],
                else_body: vec![],
            }],
            loc: Default::default(),
        }],
        3,
    );
    assert!(matches!(&body[0], NirOp::For { .. }), "no unswitch");
}

/// The row is off at `-O2` (it is a Level-3 row).
#[test]
fn level_two_disables_the_row() {
    let body = run(
        while_with_if(binary(BinaryOp::Less, local("p"), local("q"))),
        2,
    );
    assert!(matches!(&body[0], NirOp::While { .. }));
}
