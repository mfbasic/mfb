//! plan-176-G: `ir::verify::tests` — match coverage, nested moves and thread views, early exits
//! and RES kinds, and computed node annotations. The shared builders and fixtures live in
//! `verify/tests/mod.rs`.

use super::*;

// --- match_covers_all: union / else / oneof arms ---------------------------

#[test]
fn func_returns_via_exhaustive_union_match() {
    let m = IrOp::Match {
        value: IrValue::Local("s".to_string()),
        cases: vec![
            union_variant_case("Circle", vec![ret(int_const("1"))]),
            union_variant_case("Square", vec![ret(int_const("2"))]),
        ],
        loc: IrSourceLoc::default(),
    };
    let f = func_returns("run", "Integer", vec![param("s", "Shape", None)], vec![m]);
    accept(&project(
        vec![f],
        vec![union("Shape", &["Circle", "Square"])],
    ));
}

#[test]
fn func_returns_via_match_else() {
    let m = IrOp::Match {
        value: IrValue::Local("s".to_string()),
        cases: vec![
            union_variant_case("Circle", vec![ret(int_const("1"))]),
            IrMatchCase {
                pattern: IrMatchPattern::Else,
                guard: None,
                body: vec![ret(int_const("2"))],
                loc: IrSourceLoc::default(),
            },
        ],
        loc: IrSourceLoc::default(),
    };
    let f = func_returns("run", "Integer", vec![param("s", "Shape", None)], vec![m]);
    accept(&project(
        vec![f],
        vec![union("Shape", &["Circle", "Square"])],
    ));
}

#[test]
fn func_returns_via_oneof_exhaustive() {
    // An enum match with a single OneOf arm covering all members, all returning.
    let m = IrOp::Match {
        value: IrValue::Local("c".to_string()),
        cases: vec![IrMatchCase {
            pattern: IrMatchPattern::OneOf(vec![
                IrValue::MemberAccess {
                    target: Box::new(IrValue::Local("Color".to_string())),
                    member: "Red".to_string(),
                    type_: crate::types::ParameterType::parse("Color"),
                },
                IrValue::MemberAccess {
                    target: Box::new(IrValue::Local("Color".to_string())),
                    member: "Green".to_string(),
                    type_: crate::types::ParameterType::parse("Color"),
                },
            ]),
            guard: None,
            body: vec![ret(int_const("1"))],
            loc: IrSourceLoc::default(),
        }],
        loc: IrSourceLoc::default(),
    };
    let f = func_returns("run", "Integer", vec![param("c", "Color", None)], vec![m]);
    accept(&project(
        vec![f],
        vec![enum_type("Color", &["Red", "Green"])],
    ));
}

#[test]
fn match_on_record_scrutinee_never_covers() {
    // A MATCH whose scrutinee is a record (not enum/union): match_covers_all
    // returns false, so the func still needs a return -> missing.
    let m = IrOp::Match {
        value: IrValue::Local("p".to_string()),
        cases: vec![IrMatchCase {
            pattern: IrMatchPattern::Else,
            guard: None,
            body: vec![ret(int_const("1"))],
            loc: IrSourceLoc::default(),
        }],
        loc: IrSourceLoc::default(),
    };
    // Else makes it exhaustive AND all arms return -> block_always_returns true.
    let f = func_returns("run", "Integer", vec![param("p", "Point", None)], vec![m]);
    accept(&project(vec![f], vec![record("Point", &["x"])]));
}

// --- oneof exhaustiveness check (check_match_exhaustive) --------------------

#[test]
fn union_oneof_partial_not_exhaustive() {
    let m = IrOp::Match {
        value: IrValue::Local("s".to_string()),
        cases: vec![IrMatchCase {
            pattern: IrMatchPattern::OneOf(vec![IrValue::Local("Circle".to_string())]),
            guard: None,
            body: vec![ret_none()],
            loc: IrSourceLoc::default(),
        }],
        loc: IrSourceLoc::default(),
    };
    let f = func_returns("run", "Nothing", vec![param("s", "Shape", None)], vec![m]);
    // Missing Square -> the union missing-member wording path.
    expect_rule(
        &project(vec![f], vec![union("Shape", &["Circle", "Square"])]),
        "TYPE_MATCH_NOT_EXHAUSTIVE",
    );
}

#[test]
fn enum_missing_member_wording() {
    let m = IrOp::Match {
        value: IrValue::Local("c".to_string()),
        cases: vec![IrMatchCase {
            pattern: IrMatchPattern::OneOf(vec![IrValue::MemberAccess {
                target: Box::new(IrValue::Local("Color".to_string())),
                member: "Red".to_string(),
                type_: crate::types::ParameterType::parse("Color"),
            }]),
            guard: None,
            body: vec![ret_none()],
            loc: IrSourceLoc::default(),
        }],
        loc: IrSourceLoc::default(),
    };
    let f = func_returns("run", "Nothing", vec![param("c", "Color", None)], vec![m]);
    let diags = collect_diagnostics(&project(
        vec![f],
        vec![enum_type("Color", &["Red", "Green"])],
    ));
    assert!(
        diags
            .iter()
            .any(|d| d.rule == "TYPE_MATCH_NOT_EXHAUSTIVE" && d.detail.contains("Color.Green")),
        "{:?}",
        diags.iter().map(|d| &d.detail).collect::<Vec<_>>()
    );
}

// --- compatible: qualified bare-name ---------------------------------------

#[test]
fn a_qualified_type_name_and_a_bare_one_are_different_types() {
    // bug-632: a `pkg.Point` value and a local `Point` are DIFFERENT types —
    // spec §13 gives an imported type a package-qualified identity, and the
    // bare-name equality this case used to assert is the confusion that
    // qualification exists to prevent (an importer's own `Point` would have
    // accepted the package's, whatever its fields).
    let body = vec![ret(IrValue::Local("p".to_string()))];
    let f = func_returns("run", "Point", vec![param("p", "pkg.Point", None)], body);
    expect_rule(
        &project(vec![f], vec![record("Point", &["x"])]),
        "TYPE_RETURN_MISMATCH",
    );

    // The same identity on both sides still matches: a package's own type is
    // compatible with itself.
    let body = vec![ret(IrValue::Local("p".to_string()))];
    let f = func_returns(
        "run",
        "pkg.Point",
        vec![param("p", "pkg.Point", None)],
        body,
    );
    accept(&project(vec![f], vec![record("Point", &["x"])]));
}

// --- guard referencing leading union-extract binds -------------------------

#[test]
fn match_guard_reads_union_extract_bind() {
    // A CASE body starts with a Bind (the union extract); the guard references
    // it — check_ops registers the leading binds for the guard scope.
    let m = IrOp::Match {
        value: IrValue::Local("s".to_string()),
        cases: vec![
            IrMatchCase {
                pattern: IrMatchPattern::Value(IrValue::Local("Circle".to_string())),
                guard: Some(binary(
                    BinaryOp::Greater,
                    IrValue::Local("r".to_string()),
                    int_const("0"),
                    "Boolean",
                )),
                body: vec![
                    bind("r", "Integer", Some(int_const("5")), false, false),
                    ret_none(),
                ],
                loc: IrSourceLoc::default(),
            },
            IrMatchCase {
                pattern: IrMatchPattern::Else,
                guard: None,
                body: vec![ret_none()],
                loc: IrSourceLoc::default(),
            },
        ],
        loc: IrSourceLoc::default(),
    };
    let f = func_returns("run", "Nothing", vec![param("s", "Shape", None)], vec![m]);
    accept(&project(
        vec![f],
        vec![union("Shape", &["Circle", "Square"])],
    ));
}

// --- state assign on data local is skipped ---------------------------------

#[test]
fn state_assign_on_data_local_skipped() {
    // StateAssign where the resource local is actually a data type (not a
    // resource): no STATE-invalid emitted (the guard requires resource-ness).
    let body = vec![
        bind("d", "Integer", Some(int_const("1")), false, true),
        IrOp::StateAssign {
            resource: "d".to_string(),
            value: int_const("2"),
            loc: IrSourceLoc::default(),
        },
    ];
    let f = func_returns("run", "Nothing", vec![], body);
    let got = rules(&project(vec![f], vec![]));
    assert!(!got.iter().any(|r| r == "TYPE_STATE_INVALID"), "{got:?}");
}

#[test]
fn move_in_if_branch_propagates_past_join() {
    // Close h inside an IF then-branch (fall-through), then use it after the IF.
    let body = vec![
        bind("h", "fs.File", None, true, false),
        IrOp::If {
            condition: const_of("Boolean", "true"),
            then_body: vec![close_eval("h")],
            else_body: vec![],
            loc: IrSourceLoc::default(),
        },
        close_eval("h"),
    ];
    let f = owner_fn("run", "Nothing", body, &["h"]);
    expect_rule(&project(vec![f], vec![]), "TYPE_USE_AFTER_MOVE");
}

#[test]
fn move_in_match_case_propagates() {
    let m = IrOp::Match {
        value: IrValue::Local("s".to_string()),
        cases: vec![
            IrMatchCase {
                pattern: IrMatchPattern::Value(IrValue::Local("Circle".to_string())),
                guard: None,
                body: vec![close_eval("h")],
                loc: IrSourceLoc::default(),
            },
            IrMatchCase {
                pattern: IrMatchPattern::Else,
                guard: None,
                body: vec![],
                loc: IrSourceLoc::default(),
            },
        ],
        loc: IrSourceLoc::default(),
    };
    let body = vec![bind("h", "fs.File", None, true, false), m, close_eval("h")];
    let mut f = owner_fn("run", "Nothing", body, &["h"]);
    f.params = vec![param("s", "Shape", None)];
    expect_rule(
        &project(vec![f], vec![union("Shape", &["Circle", "Square"])]),
        "TYPE_USE_AFTER_MOVE",
    );
}

#[test]
fn move_in_while_body_propagates() {
    let body = vec![
        bind("h", "fs.File", None, true, false),
        IrOp::While {
            kind: crate::ast::LoopKind::While,
            condition: const_of("Boolean", "true"),
            body: vec![close_eval("h")],
            loc: IrSourceLoc::default(),
        },
        close_eval("h"),
    ];
    let f = owner_fn("run", "Nothing", body, &["h"]);
    expect_rule(&project(vec![f], vec![]), "TYPE_USE_AFTER_MOVE");
}

#[test]
fn close_in_foreach_body_is_accepted() {
    // plan-59-E: was `move_in_foreach_body_non_owning`. Closing a `FOR EACH`
    // element used to be a not-owner error (`TYPE_RESOURCE_INVALIDATE_NOT_OWNER`,
    // retired); under scope ownership any holder may close. Still exercises the
    // ForEach arm of `check_resource_moves`, which is what the original was
    // really protecting — only the verdict changed.
    let fe = IrOp::ForEach {
        name: "el".to_string(),
        type_: ParameterType::parse("fs.File"),
        iterable: IrValue::Local("xs".to_string()),
        body: vec![close_eval("el")],
        loc: IrSourceLoc::default(),
    };
    let mut f = func_returns(
        "run",
        "Nothing",
        vec![param("xs", "List OF RES fs.File", None)],
        vec![fe],
    );
    let _ = &mut f;
    accept(&project(vec![f], vec![]));
}

#[test]
fn res_transfer_moves_source() {
    // RES b = a — an ownership transfer moves `a`; a later use is after-move.
    let body = vec![
        bind("a", "fs.File", None, true, false),
        bind(
            "b",
            "fs.File",
            Some(IrValue::Local("a".to_string())),
            true,
            false,
        ),
        close_eval("a"),
    ];
    let f = owner_fn("run", "Nothing", body, &["a", "b"]);
    expect_rule(&project(vec![f], vec![]), "TYPE_USE_AFTER_MOVE");
}

fn thread_op(target: &str, handle: &str, returns: &str) -> IrOp {
    eval(IrValue::Call {
        target: target.to_string(),
        args: vec![IrValue::Local(handle.to_string())],
        type_: crate::types::ParameterType::parse(returns),
        loc: IrSourceLoc::default(),
    })
}

/// `RES b = a` hands the view over, exactly as for `fs.File`: `a` is moved.
#[test]
fn a_thread_view_transfer_moves_the_source() {
    let body = vec![
        bind("a", THREAD_VIEW, None, true, false),
        bind(
            "b",
            THREAD_VIEW,
            Some(IrValue::Local("a".to_string())),
            true,
            false,
        ),
        thread_op("thread.isRunning", "a", "Boolean"),
    ];
    let f = owner_fn("run", "Nothing", body, &["a", "b"]);
    expect_rule(&project(vec![f], vec![]), "TYPE_USE_AFTER_MOVE");
}

/// bug-975: a `ThreadWorker` is a resource with no close op, so a `LET` of one is
/// `TYPE_RESOURCE_REQUIRES_RES`, exactly as a `LET` of a parent `Thread` is.
#[test]
fn a_let_of_a_worker_view_requires_res() {
    let body = vec![bind(
        "w2",
        WORKER_VIEW,
        Some(IrValue::Local("w".to_string())),
        true,
        false,
    )];
    let f = func_returns("run", "Nothing", vec![param("w", WORKER_VIEW, None)], body);
    expect_rule(&project(vec![f], vec![]), "TYPE_RESOURCE_REQUIRES_RES");
}

/// bug-975: `RES b = a` of a worker view moves `a`, as for every resource.
#[test]
fn a_worker_view_alias_moves_the_source() {
    let body = vec![
        bind(
            "w2",
            WORKER_VIEW,
            Some(IrValue::Local("w".to_string())),
            true,
            false,
        ),
        thread_op("thread.isCancelled", "w", "Boolean"),
    ];
    let mut f = owner_fn("run", "Nothing", body, &["w2"]);
    f.params = vec![param("w", WORKER_VIEW, None)];
    expect_rule(&project(vec![f], vec![]), "TYPE_USE_AFTER_MOVE");
}

/// bug-975: being a resource is not having a close op. The worker view is a
/// resource for the binding rule, yet nothing names a close op for it.
#[test]
fn a_worker_view_is_a_resource_with_no_close_op() {
    let worker = ParameterType::parse(WORKER_VIEW);
    assert!(crate::codegen::resource::is_builtin_resource(&worker));
    assert_eq!(
        crate::codegen::resource::builtin_resource_close_function(&worker),
        None
    );
    let parent = ParameterType::parse(THREAD_VIEW);
    assert!(crate::codegen::resource::is_builtin_resource(&parent));
    assert!(crate::codegen::resource::builtin_resource_close_function(&parent).is_some());
}

/// `RETURN t` consumes the view: its close obligation leaves with it, so a read of
/// `t` after the return is a read of a moved resource.
#[test]
fn returning_a_thread_view_consumes_it() {
    let body = vec![
        bind("t", THREAD_VIEW, None, true, false),
        IrOp::Return {
            value: Some(IrValue::Local("t".to_string())),
            loc: IrSourceLoc::default(),
        },
        thread_op("thread.isRunning", "t", "Boolean"),
    ];
    let f = owner_fn("make", THREAD_VIEW, body, &["t"]);
    expect_rule(&project(vec![f], vec![]), "TYPE_USE_AFTER_MOVE");
}

/// Passing the view to a function lends it (§15): the caller's next op on it is a
/// read of a live resource, not a use after move.
#[test]
fn passing_a_thread_view_to_a_function_borrows_it() {
    let body = vec![
        bind("t", THREAD_VIEW, None, true, false),
        thread_op("peek", "t", "Boolean"),
        thread_op("thread.isRunning", "t", "Boolean"),
        thread_op("thread.waitFor", "t", "Integer"),
    ];
    let f = owner_fn("run", "Nothing", body, &["t"]);
    let got = rules(&project(vec![f], vec![]));
    assert!(
        !got.iter().any(|r| r == "TYPE_USE_AFTER_MOVE"),
        "a lent thread view was treated as moved: {got:?}"
    );
}

/// `RES` on a thread binding is a resource binding, not `RES` on data.
#[test]
fn res_on_a_thread_binding_is_accepted() {
    let body = vec![bind("t", THREAD_VIEW, None, true, false)];
    let f = owner_fn("run", "Nothing", body, &["t"]);
    expect_no_rule(&project(vec![f], vec![]), "TYPE_RES_REQUIRES_RESOURCE");
}

/// plan-156-F: a thread is bound with `RES` only — a `LET`/`MUT` thread binding is the
/// rule every other resource gets.
#[test]
fn a_let_thread_binding_requires_res() {
    let body = vec![bind("t", THREAD_VIEW, None, true, false)];
    let f = func_returns("run", "Nothing", vec![], body);
    expect_rule(&project(vec![f], vec![]), "TYPE_RESOURCE_REQUIRES_RES");
}

// --- thread.result member --------------------------------------------------

#[test]
fn rejects_thread_result_member() {
    let body = vec![ret(IrValue::MemberAccess {
        target: Box::new(IrValue::Local("t".to_string())),
        member: "result".to_string(),
        type_: crate::types::ParameterType::parse("Unknown"),
    })];
    // A well-formed handle spells both planes (`Thread OF Msg TO Out`); the
    // truncated `Thread OF Integer` used here before plan-106-B parses to an
    // opaque `Named`, so it exercised only the defensive name arm of
    // `is_thread_type` (pinned directly by
    // `truncated_thread_spelling_still_counts_as_a_thread`).
    let f = func_returns(
        "run",
        "Integer",
        vec![param("t", "Thread OF Integer TO Integer", None)],
        body,
    );
    expect_rule(&project(vec![f], vec![]), "TYPE_THREAD_RESULT_REMOVED");
}

// --- match literal-pattern type mismatch -----------------------------------

#[test]
fn rejects_match_literal_pattern_type() {
    // A String literal CASE against an Integer scrutinee.
    let m = IrOp::Match {
        value: int_const("1"),
        cases: vec![
            IrMatchCase {
                pattern: IrMatchPattern::Value(const_of("String", "a")),
                guard: None,
                body: vec![ret_none()],
                loc: IrSourceLoc::default(),
            },
            IrMatchCase {
                pattern: IrMatchPattern::Else,
                guard: None,
                body: vec![ret_none()],
                loc: IrSourceLoc::default(),
            },
        ],
        loc: IrSourceLoc::default(),
    };
    let f = func_returns("run", "Nothing", vec![], vec![m]);
    expect_rule(&project(vec![f], vec![]), "TYPE_MATCH_PATTERN_MISMATCH");
}

// --- collections.get argument mismatch (valid arity) -----------------------

#[test]
fn rejects_collections_get_bad_args() {
    let body = vec![ret(IrValue::Call {
        target: "collections.get".to_string(),
        args: vec![int_const("1"), int_const("2")],
        type_: crate::types::ParameterType::parse("Unknown"),
        loc: IrSourceLoc::default(),
    })];
    let f = func_returns("run", "Integer", vec![], body);
    expect_rule(&project(vec![f], vec![]), "TYPE_CALL_ARGUMENT_MISMATCH");
}

// --- unresolvable argument type skips builtin arg check --------------------

#[test]
fn builtin_arg_check_skipped_when_arg_type_unknown() {
    // An argument whose inferred type is None (a nested call annotated "Unknown")
    // -> the arg_types collect yields None and the check is skipped.
    let body = vec![ret(IrValue::Call {
        target: "math.sqrt".to_string(),
        args: vec![IrValue::Call {
            target: "mystery.helper".to_string(),
            args: vec![],
            type_: crate::types::ParameterType::parse("Unknown"),
            loc: IrSourceLoc::default(),
        }],
        type_: crate::types::ParameterType::parse("Float"),
        loc: IrSourceLoc::default(),
    })];
    let f = func_returns("run", "Float", vec![], body);
    let got = rules(&project(vec![f], vec![]));
    assert!(
        !got.iter().any(|r| r == "TYPE_CALL_ARGUMENT_MISMATCH"),
        "{got:?}"
    );
}

// --- binding/condition/assignment unknown-expected early exits -------------

#[test]
fn binding_unknown_expected_skips_mismatch() {
    // A binding declared AS Unknown (explicit) — check_binding_type early-returns.
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind("x", "Unknown", Some(int_const("1")), true, false)],
    );
    accept(&project(vec![f], vec![]));
}

// --- union wrap with empty member type is skipped --------------------------

#[test]
fn union_wrap_empty_member_skipped() {
    let body = vec![ret(IrValue::UnionWrap {
        union_type: crate::types::ParameterType::parse("Shape"),
        member_type: crate::types::ParameterType::parse(""),
        value: Box::new(int_const("0")),
    })];
    let f = func_returns("run", "Shape", vec![], body);
    // Empty member_type -> check_union_wrap early-returns, no diagnostic.
    let got = rules(&project(vec![f], vec![union("Shape", &["Circle"])]));
    assert!(
        !got.iter()
            .any(|r| r == "PACKAGE_BINARY_REPRESENTATION_VERIFY_TYPE"),
        "{got:?}"
    );
}

// --- provably_data_type: RES on enum/record/data-union rejects -------------

#[test]
fn rejects_res_on_enum() {
    let mut f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind("c", "Color", None, true, false)],
    );
    f.resource_owners
        .insert("c".to_string(), crate::ir::resource_escape::ResOwner::Local);
    expect_rule(
        &project(vec![f], vec![enum_type("Color", &["Red"])]),
        "TYPE_RES_REQUIRES_RESOURCE",
    );
}

#[test]
fn rejects_res_on_record() {
    let mut f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind("p", "Point", None, true, false)],
    );
    f.resource_owners
        .insert("p".to_string(), crate::ir::resource_escape::ResOwner::Local);
    expect_rule(
        &project(vec![f], vec![record("Point", &["x"])]),
        "TYPE_RES_REQUIRES_RESOURCE",
    );
}

#[test]
fn rejects_res_on_data_union() {
    // A union with only data variants is provably data.
    let mut f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind("s", "Shape", None, true, false)],
    );
    f.resource_owners
        .insert("s".to_string(), crate::ir::resource_escape::ResOwner::Local);
    expect_rule(
        &project(vec![f], vec![union("Shape", &["Circle", "Square"])]),
        "TYPE_RES_REQUIRES_RESOURCE",
    );
}

// --- walk_captures wrapping shapes (out-of-range in wrapped value) ----------

#[test]
fn capture_out_of_range_inside_union_extract() {
    let closure_body = func_returns(
        "body",
        "Integer",
        vec![],
        vec![ret(IrValue::UnionExtract {
            type_: crate::types::ParameterType::parse("Integer"),
            value: Box::new(IrValue::Capture {
                index: 5,
                type_: crate::types::ParameterType::parse("Integer"),
                by_ref: false,
            }),
        })],
    );
    let maker = func_returns(
        "make",
        "FUNC() AS Integer",
        vec![],
        vec![ret(IrValue::Closure {
            name: "body".to_string(),
            type_: crate::types::ParameterType::parse("FUNC() AS Integer"),
            captures: vec![int_const("1")],
        })],
    );
    let err = check(&project(vec![closure_body, maker], vec![])).expect_err("capture out of range");
    assert!(err.contains("out of range"), "{err}");
}

#[test]
fn capture_out_of_range_inside_result_value_and_member() {
    let closure_body = func_returns(
        "body",
        "Integer",
        vec![],
        vec![ret(IrValue::MemberAccess {
            target: Box::new(IrValue::ResultValue {
                type_: crate::types::ParameterType::parse("Integer"),
                value: Box::new(IrValue::Capture {
                    index: 8,
                    type_: crate::types::ParameterType::parse("Integer"),
                    by_ref: false,
                }),
            }),
            member: "x".to_string(),
            type_: crate::types::ParameterType::parse("Integer"),
        })],
    );
    let maker = func_returns(
        "make",
        "FUNC() AS Integer",
        vec![],
        vec![ret(IrValue::Closure {
            name: "body".to_string(),
            type_: crate::types::ParameterType::parse("FUNC() AS Integer"),
            captures: vec![int_const("1")],
        })],
    );
    let err = check(&project(vec![closure_body, maker], vec![])).expect_err("capture out of range");
    assert!(err.contains("out of range"), "{err}");
}

// --- enum member access when the enum name is a bare TYPE (no local) --------

#[test]
fn enum_member_access_returns_after_check() {
    // Two accesses on Color: one valid member and the whole thing type-checks;
    // exercises the early `return` after the enum-member branch.
    let body = vec![
        bind(
            "a",
            "Color",
            Some(IrValue::MemberAccess {
                target: Box::new(IrValue::Local("Color".to_string())),
                member: "Red".to_string(),
                type_: crate::types::ParameterType::parse("Color"),
            }),
            false,
            false,
        ),
        ret_none(),
    ];
    let f = func_returns("run", "Nothing", vec![], body);
    accept(&project(
        vec![f],
        vec![enum_type("Color", &["Red", "Green"])],
    ));
}

// --- bug-31: computed nodes must not be trusted to report their own type ------
//
// On the package path every `type_` annotation is attacker-controlled. Each test
// below crafts the IR a hostile `.mfp` would carry and asserts the verifier
// contradicts the annotation from an independent source of truth.

/// `getName` really returns `String`; the call node claims it returns `Account`,
/// so the member access reads a string at `Account.balance`'s offset.
#[test]
fn call_result_annotated_as_a_foreign_record_is_rejected() {
    let get_name = func_returns(
        "getName",
        "String",
        vec![],
        vec![ret(const_of("String", "a"))],
    );
    let confused = IrValue::MemberAccess {
        target: Box::new(IrValue::Call {
            target: "getName".to_string(),
            args: vec![],
            type_: crate::types::ParameterType::parse("Account"),
            loc: IrSourceLoc::default(),
        }),
        member: "balance".to_string(),
        type_: crate::types::ParameterType::parse("Integer"),
    };
    let caller = func("run", vec![], vec![ret(confused)]);
    expect_rule(
        &project(
            vec![get_name, caller],
            vec![record_typed("Account", &[("balance", "Integer")])],
        ),
        "PACKAGE_BINARY_REPRESENTATION_VERIFY_TYPE",
    );
}

/// A `String`-returning call annotated `Integer` used to satisfy the numeric
/// operand rule, so codegen emitted an integer subtract over a string pointer.
#[test]
fn string_call_annotated_integer_cannot_feed_arithmetic() {
    let get_name = func_returns(
        "getName",
        "String",
        vec![],
        vec![ret(const_of("String", "a"))],
    );
    let confused = binary(
        BinaryOp::Subtract,
        IrValue::Call {
            target: "getName".to_string(),
            args: vec![],
            type_: crate::types::ParameterType::parse("Integer"),
            loc: IrSourceLoc::default(),
        },
        int_const("5"),
        "Integer",
    );
    let caller = func("run", vec![], vec![ret(confused)]);
    expect_rule(
        &project(vec![get_name, caller], vec![]),
        "PACKAGE_BINARY_REPRESENTATION_VERIFY_TYPE",
    );
}

/// The same lie through a fallible call node.
#[test]
fn call_result_node_annotation_is_reconciled_too() {
    let get_name = func_returns(
        "getName",
        "String",
        vec![],
        vec![ret(const_of("String", "a"))],
    );
    let caller = func(
        "run",
        vec![],
        vec![ret(IrValue::CallResult {
            target: "getName".to_string(),
            args: vec![],
            type_: crate::types::ParameterType::parse("Integer"),
            loc: IrSourceLoc::default(),
        })],
    );
    expect_rule(
        &project(vec![get_name, caller], vec![]),
        "PACKAGE_BINARY_REPRESENTATION_VERIFY_TYPE",
    );
}

/// A truthful annotation still verifies, on both call node kinds.
#[test]
fn a_truthful_call_annotation_is_accepted() {
    let get_name = func_returns(
        "getName",
        "String",
        vec![],
        vec![ret(const_of("String", "a"))],
    );
    let caller = func_returns(
        "run",
        "String",
        vec![],
        vec![ret(IrValue::Call {
            target: "getName".to_string(),
            args: vec![],
            type_: crate::types::ParameterType::parse("String"),
            loc: IrSourceLoc::default(),
        })],
    );
    accept(&project(vec![get_name, caller], vec![]));

    // An `Unknown` annotation is unresolved, not a disagreement.
    let get_name = func_returns(
        "getName",
        "String",
        vec![],
        vec![ret(const_of("String", "a"))],
    );
    let caller = func_returns(
        "run",
        "String",
        vec![],
        vec![ret(IrValue::Call {
            target: "getName".to_string(),
            args: vec![],
            type_: crate::types::ParameterType::parse("Unknown"),
            loc: IrSourceLoc::default(),
        })],
    );
    accept(&project(vec![get_name, caller], vec![]));
}

/// A member access that lies about the field's declared type poisons every rule
/// downstream of it (`infer_type` prefers the annotation).
#[test]
fn member_access_annotated_against_its_field_type_is_rejected() {
    let confused = IrValue::MemberAccess {
        target: Box::new(IrValue::Local("acct".to_string())),
        member: "balance".to_string(),
        type_: crate::types::ParameterType::parse("String"),
    };
    let body = vec![
        bind(
            "acct",
            "Account",
            Some(IrValue::Constructor {
                type_: crate::types::ParameterType::parse("Account"),
                args: vec![int_const("1")],
            }),
            true,
            false,
        ),
        ret(confused),
    ];
    expect_rule(
        &project(
            vec![func("run", vec![], body)],
            vec![record_typed("Account", &[("balance", "Integer")])],
        ),
        "PACKAGE_BINARY_REPRESENTATION_VERIFY_TYPE",
    );
}

/// Operator nodes are reconciled against the type their operands produce.
#[test]
fn operator_result_annotations_are_reconciled_with_their_operands() {
    // `1 < 2` is a Boolean, whatever the node claims.
    let caller = func(
        "run",
        vec![],
        vec![ret(binary(
            BinaryOp::Less,
            int_const("1"),
            int_const("2"),
            "Integer",
        ))],
    );
    expect_rule(
        &project(vec![caller], vec![]),
        "PACKAGE_BINARY_REPRESENTATION_VERIFY_TYPE",
    );

    // `"a" & "b"` is a String.
    let caller = func(
        "run",
        vec![],
        vec![ret(binary(
            BinaryOp::Concat,
            const_of("String", "a"),
            const_of("String", "b"),
            "Integer",
        ))],
    );
    expect_rule(
        &project(vec![caller], vec![]),
        "PACKAGE_BINARY_REPRESENTATION_VERIFY_TYPE",
    );

    // Integer arithmetic over Integer operands is an Integer.
    let caller = func(
        "run",
        vec![],
        vec![ret(binary(
            BinaryOp::Add,
            int_const("1"),
            int_const("2"),
            "String",
        ))],
    );
    expect_rule(
        &project(vec![caller], vec![]),
        "PACKAGE_BINARY_REPRESENTATION_VERIFY_TYPE",
    );

    // `NOT` yields a Boolean; negation preserves its operand type.
    let caller = func(
        "run",
        vec![],
        vec![ret(unary(
            UnaryOp::Not,
            const_of("Boolean", "true"),
            "Integer",
        ))],
    );
    expect_rule(
        &project(vec![caller], vec![]),
        "PACKAGE_BINARY_REPRESENTATION_VERIFY_TYPE",
    );
    let caller = func(
        "run",
        vec![],
        vec![ret(unary(UnaryOp::Negate, int_const("1"), "String"))],
    );
    expect_rule(
        &project(vec![caller], vec![]),
        "PACKAGE_BINARY_REPRESENTATION_VERIFY_TYPE",
    );

    // Truthful operator annotations still verify.
    let caller = func_returns(
        "run",
        "Boolean",
        vec![],
        vec![ret(binary(
            BinaryOp::Less,
            int_const("1"),
            int_const("2"),
            "Boolean",
        ))],
    );
    accept(&project(vec![caller], vec![]));
}
