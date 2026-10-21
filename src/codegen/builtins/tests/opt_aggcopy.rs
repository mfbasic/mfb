//! Aggregate copy propagation (`optimizer::opt1::aggcopy`) for a copy bound
//! inside a nested body, read from a `MATCH` guard and from the case body, in
//! a function that has parameters of its own.
//!
//! The row's contract: a stable `LET b = a` on a block-copying type is deleted
//! wherever it sits and every read of `b` — in values, guards and nested
//! bodies alike — reads `a`. A read it missed would name a binding that no
//! longer exists.

use super::nir_build::*;
use crate::operators::BinaryOp;
use crate::target::shared::nir::visit::{walk_value, NirVisitor};
use crate::target::shared::nir::{NirMatchCase, NirMatchPattern, NirOp, NirParam, NirValue};
use crate::types::ParameterType;

fn list_bind(name: &str, value: NirValue) -> NirOp {
    NirOp::Bind {
        mutable: false,
        name: name.to_string(),
        type_: ParameterType::list_of(ParameterType::Integer),
        value: Some(value),
    }
}

/// Every local `ops` reads, in visit order.
fn reads(ops: &[NirOp]) -> Vec<String> {
    #[derive(Default)]
    struct Reads(Vec<String>);
    impl NirVisitor for Reads {
        fn visit_value(&mut self, value: &NirValue) {
            if let NirValue::Local(name) = value {
                self.0.push(name.clone());
            }
            walk_value(self, value);
        }
    }
    let mut found = Reads::default();
    found.visit_ops(ops);
    found.0
}

#[test]
fn a_nested_copy_is_forwarded_into_guards_and_case_bodies() {
    let body = vec![
        list_bind(
            "a",
            NirValue::ListLiteral {
                type_: ParameterType::list_of(ParameterType::Integer),
                values: vec![int_const("1"), int_const("2")],
            },
        ),
        NirOp::If {
            condition: binary(BinaryOp::Less, local("p"), int_const("3")),
            then_body: vec![
                list_bind("b", local("a")),
                bind(
                    "c",
                    NirValue::Closure {
                        name: "$lambda1".to_string(),
                        type_: ParameterType::Unknown,
                        captures: vec![NirValue::MemberAccess {
                            target: Box::new(local("b")),
                            member: "count".to_string(),
                        }],
                    },
                ),
                NirOp::Match {
                    value: local("p"),
                    cases: vec![NirMatchCase {
                        pattern: NirMatchPattern::Else,
                        guard: Some(binary(BinaryOp::Equal, local("b"), local("a"))),
                        body: vec![NirOp::Return {
                            value: Some(local("b")),
                        }],
                    }],
                },
            ],
            else_body: vec![],
        },
    ];
    let mut function = function("f", body);
    function.params = vec![NirParam {
        name: "p".to_string(),
        type_: ParameterType::Integer,
        default: None,
    }];
    let mut module = module(vec![function]);
    crate::optimizer::with_opt_level(crate::optimizer::OptLevel(3), || {
        crate::optimizer::opt1::aggcopy::propagate(&mut module)
    });
    let body = &module.functions[0].body;
    let NirOp::If { then_body, .. } = &body[1] else {
        panic!("the branch stays");
    };
    assert_eq!(
        then_body.len(),
        2,
        "the copy binding inside the branch is gone"
    );
    let names = reads(body);
    assert!(
        !names.iter().any(|name| name == "b"),
        "a read of the dropped copy survived: {names:?}"
    );
    assert_eq!(
        names.iter().filter(|name| *name == "a").count(),
        4,
        "the closure's capture, the guard and the case body all read the source: {names:?}"
    );
}

/// A copy whose source is the function's own parameter is not forwarded: the
/// parameter's block belongs to the caller.
#[test]
fn a_copy_of_a_parameter_is_kept() {
    let body = vec![
        list_bind("b", local("p")),
        NirOp::Return {
            value: Some(local("b")),
        },
    ];
    let mut function = function("f", body);
    function.params = vec![NirParam {
        name: "p".to_string(),
        type_: ParameterType::list_of(ParameterType::Integer),
        default: None,
    }];
    let mut module = module(vec![function]);
    crate::optimizer::with_opt_level(crate::optimizer::OptLevel(3), || {
        crate::optimizer::opt1::aggcopy::propagate(&mut module)
    });
    assert_eq!(module.functions[0].body.len(), 2);
}
