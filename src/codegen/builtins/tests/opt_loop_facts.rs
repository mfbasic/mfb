//! The loop facts every Opt1 loop row consults
//! (`optimizer::opt1::plans::loops`), at the shapes the rows' own tests do not
//! hand them: a clone whose declarations span every statement kind, a fresh
//! spelling that is already taken, a global read, loop control inside a
//! handler, and the stable-leaf comparisons.
//!
//! `freshened_clone` is the one that matters most. NIR locals are
//! function-unique, so a row that duplicates statements (peeling, unswitching,
//! fission's carried binds) must rename every name the copy declares — and
//! every READ of it — or validation rejects the function. A rename that missed
//! a statement or value kind would leave the copy reading the original's
//! variable.

use super::nir_build::*;
use crate::ast::LoopKind;
use crate::operators::BinaryOp;
use crate::optimizer::opt1::plans::loops::{
    captures_loop_control, declared_names, freshened_clone, invariant, leaf_names,
    same_stable_leaf, value_reads,
};
use crate::optimizer::opt1::plans::reads::NameUses;
use crate::target::shared::nir::{NirMatchCase, NirOp, NirValue};
use crate::types::ParameterType;

fn boxed(value: NirValue) -> Box<NirValue> {
    Box::new(value)
}

/// A statement list that declares a name in every declaring construct and
/// reads each declared name from every value kind the rename must reach.
fn every_kind() -> Vec<NirOp> {
    vec![
        bind("x", int_const("1")),
        NirOp::StateAssign {
            resource: "x".to_string(),
            value: local("x"),
        },
        NirOp::StoreGlobal {
            name: "g".to_string(),
            type_: ParameterType::Integer,
            value: Some(local("x")),
        },
        NirOp::DoUntil {
            body: vec![bind("d", local("x")), assign("d", local("d"))],
            condition: binary(BinaryOp::Less, local("d"), local("x")),
        },
        NirOp::ForEach {
            name: "item".to_string(),
            type_: ParameterType::Integer,
            iterable: local("xs"),
            body: vec![NirOp::Eval {
                value: NirValue::LocalRef {
                    name: "item".to_string(),
                    type_: ParameterType::Integer,
                },
            }],
        },
        NirOp::Trap {
            name: "e".to_string(),
            body: vec![NirOp::Eval {
                value: NirValue::ResultError {
                    value: boxed(local("e")),
                },
            }],
        },
        NirOp::Match {
            value: local("x"),
            cases: vec![NirMatchCase {
                pattern: crate::target::shared::nir::NirMatchPattern::Else,
                guard: Some(local("x")),
                body: vec![NirOp::Eval {
                    value: NirValue::UnionWrap {
                        union_type: ParameterType::named("U"),
                        member_type: ParameterType::named("A"),
                        value: boxed(local("x")),
                    },
                }],
            }],
        },
        NirOp::While {
            kind: LoopKind::While,
            condition: local("x"),
            body: vec![NirOp::Eval {
                value: NirValue::Checked {
                    type_: ParameterType::Integer,
                    value: boxed(local("x")),
                    observe: false,
                },
            }],
        },
        for_loop(
            "k",
            int_const("0"),
            int_const("3"),
            int_const("1"),
            vec![NirOp::ContinueLoop {
                kind: LoopKind::For,
                target: Some("k".to_string()),
            }],
        ),
        NirOp::ExitProgram { code: local("x") },
        NirOp::Fail { error: local("e") },
        NirOp::Return {
            value: Some(local("x")),
        },
    ]
}

#[test]
fn a_freshened_clone_renames_every_declaration_and_every_read_of_it() {
    let ops = every_kind();
    let declared = declared_names(&ops);
    for name in ["x", "d", "item", "e", "k"] {
        assert!(
            declared.contains(name),
            "`{name}` is declared: {declared:?}"
        );
    }
    // `x$dup1` is already taken in the function, so `x` must skip it.
    let mut function = ops.clone();
    function.push(bind("x$dup1", int_const("0")));
    let census = NameUses::census(&function);
    let mut salt = 0;
    let copy = freshened_clone(&ops, &census, &mut salt);
    let copy_census = NameUses::census(&copy);
    for name in &declared {
        assert_eq!(
            copy_census.count(name),
            0,
            "the copy still names the original `{name}`"
        );
    }
    let renamed = declared_names(&copy);
    assert!(
        !renamed.contains("x$dup1"),
        "a fresh spelling already in use must be skipped: {renamed:?}"
    );
    assert_eq!(renamed.len(), declared.len());
    // The undeclared global and the iterable keep their names.
    assert_eq!(copy_census.count("xs"), 1);
    let NirOp::StoreGlobal { name, .. } = &copy[2] else {
        panic!("statement order is kept");
    };
    assert_eq!(name, "g");
}

#[test]
fn a_clone_that_declares_nothing_is_the_original() {
    let ops = vec![assign("y", local("z"))];
    let census = NameUses::census(&ops);
    let mut salt = 0;
    let copy = freshened_clone(&ops, &census, &mut salt);
    assert!(
        matches!(&copy[0], NirOp::Assign { name, value: NirValue::Local(read) }
        if name == "y" && read == "z")
    );
    assert_eq!(salt, 0);
}

/// A `MATCH` pattern is a value position too: a declared name read in a
/// `CASE` value or one-of list is renamed with every other read, so the clone
/// is fresh without a verification step to fall back on.
#[test]
fn a_freshened_clone_renames_a_declared_name_read_in_a_match_pattern() {
    use crate::target::shared::nir::NirMatchPattern;
    let ops = vec![
        bind("k", int_const("1")),
        NirOp::Match {
            value: local("k"),
            cases: vec![
                NirMatchCase {
                    pattern: NirMatchPattern::Value(local("k")),
                    guard: None,
                    body: Vec::new(),
                },
                NirMatchCase {
                    pattern: NirMatchPattern::OneOf(vec![int_const("2"), local("k")]),
                    guard: None,
                    body: Vec::new(),
                },
            ],
        },
    ];
    let census = NameUses::census(&ops);
    let mut salt = 0;
    let copy = freshened_clone(&ops, &census, &mut salt);
    let copy_census = NameUses::census(&copy);
    assert_eq!(copy_census.count("k"), 0, "every read of `k` is renamed");
    let NirOp::Bind { name: fresh, .. } = &copy[0] else {
        panic!("statement order is kept");
    };
    assert_eq!(
        copy_census.count(fresh),
        4,
        "the bind, the scrutinee, both patterns"
    );
}

/// A global read is never loop-invariant: a call or `StoreGlobal` in the body
/// could change it between iterations.
#[test]
fn a_global_read_is_never_invariant() {
    let global = NirValue::Global {
        name: "limit".to_string(),
        type_: ParameterType::Integer,
    };
    assert!(value_reads(&global).is_empty());
    assert!(!invariant(&global, &Default::default()));
    assert!(invariant(&local("n"), &Default::default()));
}

/// An `EXIT FOR` inside a handler body still binds to a newly introduced
/// `FOR`.
#[test]
fn loop_control_inside_a_handler_is_captured() {
    let ops = vec![NirOp::Trap {
        name: "e".to_string(),
        body: vec![NirOp::ExitLoop {
            kind: LoopKind::For,
            target: None,
        }],
    }];
    assert!(captures_loop_control(&ops, LoopKind::For));
    assert!(!captures_loop_control(&ops, LoopKind::While));
}

#[test]
fn stable_leaves_compare_by_constant_or_by_name() {
    assert!(same_stable_leaf(&local("n"), &local("n")));
    assert!(!same_stable_leaf(&local("n"), &local("m")));
    assert!(same_stable_leaf(&int_const("9"), &int_const("9")));
    assert!(!same_stable_leaf(&int_const("9"), &local("n")));
    assert!(!same_stable_leaf(
        &binary(BinaryOp::Add, local("n"), int_const("1")),
        &binary(BinaryOp::Add, local("n"), int_const("1"))
    ));
    let names = leaf_names(&[&local("lo"), &int_const("9"), &local("step")]);
    assert_eq!(names.len(), 2);
    assert!(names.contains("lo") && names.contains("step"));
}
