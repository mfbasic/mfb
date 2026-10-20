//! plan-176-G: `ir::verify::tests` — traps and closures, diagnostics/visibility/builtin
//! arguments, exhaustive returns and moves, and capture value positions. The shared builders
//! and fixtures live in `verify/tests/mod.rs`.

use super::*;

// --- trap ------------------------------------------------------------------

// bug-797: a function-level handler may read only locals bound on every failure
// edge into it. These run on the verifier alone, which is also the decoded-package
// path, so a package cannot carry the invalid read past it.

fn call_eval(target: &str) -> IrOp {
    IrOp::Eval {
        value: IrValue::Call {
            target: target.to_string(),
            args: vec![],
            type_: crate::types::ParameterType::parse("Nothing"),
            loc: IrSourceLoc::default(),
        },
        loc: IrSourceLoc::default(),
    }
}

fn trap_returning_local(name: &str) -> IrOp {
    IrOp::Trap {
        name: "e".to_string(),
        body: vec![ret(IrValue::Local(name.to_string()))],
        loc: IrSourceLoc::default(),
    }
}

#[test]
fn rejects_a_handler_read_of_a_local_declared_after_a_failure() {
    // `pkg.mightFail` is not a project function, so it is assumed to fail.
    let body = vec![
        call_eval("pkg.mightFail"),
        bind("value", "Integer", Some(int_const("42")), false, false),
        ret(IrValue::Local("value".to_string())),
        trap_returning_local("value"),
    ];
    let f = func_returns("run", "Integer", vec![], body);
    expect_rule(&project(vec![f], vec![]), "TYPE_TRAP_LOCAL_UNINITIALIZED");
}

#[test]
fn accepts_a_handler_read_of_a_local_declared_before_any_failure() {
    let body = vec![
        bind("value", "Integer", Some(int_const("42")), false, false),
        call_eval("pkg.mightFail"),
        ret(IrValue::Local("value".to_string())),
        trap_returning_local("value"),
    ];
    let f = func_returns("run", "Integer", vec![], body);
    expect_no_rule(&project(vec![f], vec![]), "TYPE_TRAP_LOCAL_UNINITIALIZED");
}

#[test]
fn a_call_to_a_total_project_function_is_not_a_failure_edge() {
    // `quiet` cannot fail, so `value` is bound on every edge into the handler.
    let quiet = func_returns("quiet", "Nothing", vec![], vec![ret_none()]);
    let body = vec![
        call_eval("quiet"),
        bind("value", "Integer", Some(int_const("42")), false, false),
        call_eval("pkg.mightFail"),
        ret(IrValue::Local("value".to_string())),
        trap_returning_local("value"),
    ];
    let f = func_returns("run", "Integer", vec![], body);
    expect_no_rule(
        &project(vec![quiet, f], vec![]),
        "TYPE_TRAP_LOCAL_UNINITIALIZED",
    );
}

#[test]
fn an_unproved_built_in_call_is_a_failure_edge() {
    // plan-165-B: `isEmpty` declares no error but no raise-checking guard proves
    // it, so missing proof counts as "can fail" and `value` is unbound on the
    // edge it adds.
    let body = vec![
        call_eval("isEmpty"),
        bind("value", "Integer", Some(int_const("42")), false, false),
        ret(IrValue::Local("value".to_string())),
        trap_returning_local("value"),
    ];
    let f = func_returns("run", "Integer", vec![], body);
    expect_rule(&project(vec![f], vec![]), "TYPE_TRAP_LOCAL_UNINITIALIZED");
}

#[test]
fn a_proven_total_built_in_call_is_not_a_failure_edge() {
    // `len` and `bits.band` are proven total (`totality_proof`), so `value` is
    // bound on every edge into the handler.
    let body = vec![
        call_eval("len"),
        call_eval("bits.band"),
        bind("value", "Integer", Some(int_const("42")), false, false),
        call_eval("pkg.mightFail"),
        ret(IrValue::Local("value".to_string())),
        trap_returning_local("value"),
    ];
    let f = func_returns("run", "Integer", vec![], body);
    expect_no_rule(&project(vec![f], vec![]), "TYPE_TRAP_LOCAL_UNINITIALIZED");
}

#[test]
fn a_failed_reassignment_keeps_the_mutable_local_readable() {
    // `m` is declared before anything can fail; a later failing `m = …` leaves
    // the earlier value in place (§14.2), so the handler may read it.
    let body = vec![
        bind("m", "Integer", Some(int_const("5")), false, true),
        IrOp::Assign {
            name: "m".to_string(),
            value: IrValue::Call {
                target: "pkg.risky".to_string(),
                args: vec![],
                type_: crate::types::ParameterType::parse("Integer"),
                loc: IrSourceLoc::default(),
            },
            loc: IrSourceLoc::default(),
        },
        ret(IrValue::Local("m".to_string())),
        trap_returning_local("m"),
    ];
    let f = func_returns("run", "Integer", vec![], body);
    expect_no_rule(&project(vec![f], vec![]), "TYPE_TRAP_LOCAL_UNINITIALIZED");
}

#[test]
fn rejects_trap_fallthrough() {
    let body = vec![IrOp::Trap {
        name: "e".to_string(),
        body: vec![IrOp::Eval {
            value: int_const("1"),
            loc: IrSourceLoc::default(),
        }],
        loc: IrSourceLoc::default(),
    }];
    let f = func_returns("run", "Nothing", vec![], body);
    expect_rule(&project(vec![f], vec![]), "TYPE_TRAP_FALLTHROUGH");
}

#[test]
fn accepts_trap_that_returns() {
    let body = vec![
        ret_none(),
        IrOp::Trap {
            name: "e".to_string(),
            body: vec![ret_none()],
            loc: IrSourceLoc::default(),
        },
    ];
    let f = func_returns("run", "Nothing", vec![], body);
    accept(&project(vec![f], vec![]));
}

#[test]
fn recover_type_mismatch() {
    // A $trap_val assign whose value type disagrees with the slot type.
    let body = vec![
        bind("$trap_val0", "Integer", Some(int_const("0")), false, false),
        IrOp::Assign {
            name: "$trap_val0".to_string(),
            value: const_of("String", "x"),
            loc: IrSourceLoc::default(),
        },
    ];
    let f = func_returns("run", "Nothing", vec![], body);
    expect_rule(&project(vec![f], vec![]), "TYPE_RECOVER_TYPE_MISMATCH");
}

// --- closures --------------------------------------------------------------

#[test]
fn accepts_closure_valid_capture_in_bind() {
    // A closure body reading a captured slot via a Bind capture value.
    let closure_body = func_returns(
        "body",
        "Integer",
        vec![],
        vec![ret(IrValue::Capture {
            index: 0,
            type_: crate::types::ParameterType::parse("Integer"),
            by_ref: false,
        })],
    );
    let maker = func_returns(
        "make",
        "FUNC() AS Integer",
        vec![],
        vec![ret(IrValue::Closure {
            name: "body".to_string(),
            type_: crate::types::ParameterType::parse("FUNC() AS Integer"),
            captures: vec![int_const("7")],
        })],
    );
    accept(&project(vec![closure_body, maker], vec![]));
}

// --- unreachable / statement nesting cap -----------------------------------

#[test]
fn accepts_deeply_nested_but_bounded() {
    // Build a nested If chain within the MAX_DEPTH cap.
    let mut inner = vec![ret_none()];
    for _ in 0..10 {
        inner = vec![IrOp::If {
            condition: const_of("Boolean", "true"),
            then_body: inner,
            else_body: vec![],
            loc: IrSourceLoc::default(),
        }];
    }
    let f = func_returns("run", "Nothing", vec![], inner);
    accept(&project(vec![f], vec![]));
}

// --- exercise global binding value walk + literal range --------------------

#[test]
fn accepts_global_list_and_map_values() {
    let mut p = project(vec![func_returns("run", "Nothing", vec![], vec![])], vec![]);
    p.bindings = vec![
        binding(
            "xs",
            "List OF Integer",
            Some(IrValue::ListLiteral {
                type_: crate::types::ParameterType::parse("List OF Integer"),
                values: vec![int_const("1")],
            }),
            false,
            true,
        ),
        binding(
            "m",
            "Map OF String TO Integer",
            Some(IrValue::MapLiteral {
                type_: crate::types::ParameterType::parse("Map OF String TO Integer"),
                entries: vec![(const_of("String", "k"), int_const("2"))],
            }),
            false,
            true,
        ),
    ];
    accept(&p);
}

#[test]
fn rejects_global_byte_overflow() {
    let mut p = project(vec![func_returns("run", "Nothing", vec![], vec![])], vec![]);
    p.bindings = vec![binding("b", "Byte", Some(int_const("999")), false, true)];
    expect_rule(&p, "TYPE_BYTE_LITERAL_OVERFLOW");
}

// --- source diagnostics filter ---------------------------------------------

#[test]
fn collect_source_diagnostics_maps_rules_to_pending() {
    use std::path::Path;
    let body = vec![ret(binary(
        BinaryOp::Subtract,
        const_of("String", "a"),
        int_const("1"),
        "Integer",
    ))];
    let p = project(vec![func("run", vec![], body)], vec![]);
    let diags = super::collect_source_diagnostics(
        &p,
        Path::new("/proj"),
        &[],
        &[],
        &[],
        &crate::ir::LinkSpans::default(),
    );
    assert!(diags
        .iter()
        .any(|d| d.rule == "TYPE_BINARY_OPERATOR_MISMATCH"));
}

#[test]
fn collect_source_diagnostics_generated_path_when_file_empty() {
    use std::path::Path;
    // A type-declaration diagnostic with an empty file -> <generated> path.
    let ty = record_typed("Node", &[("next", "Node")]);
    let p = project(
        vec![func_returns("run", "Nothing", vec![], vec![])],
        vec![ty],
    );
    let diags = super::collect_source_diagnostics(
        &p,
        Path::new("/proj"),
        &[],
        &[],
        &[],
        &crate::ir::LinkSpans::default(),
    );
    assert!(diags
        .iter()
        .any(|d| d.rule == "TYPE_RECURSIVE_RECORD_REQUIRES_INDIRECTION"
            && d.path.ends_with("<generated>")));
}

// --- constructor visibility ------------------------------------------------

fn private_record(name: &str, file: &str, fields: &[(&str, &str)]) -> IrType {
    let mut ty = record_typed(name, fields);
    ty.visibility = "private".to_string();
    ty.file = file.to_string();
    ty
}

#[test]
fn rejects_construct_private_type_cross_file() {
    // Type declared in other.mfb, constructed from src/main.mfb.
    let ty = private_record("Secret", "src/other.mfb", &[("x", "Integer")]);
    let body = vec![ret(IrValue::Constructor {
        type_: crate::types::ParameterType::parse("Secret"),
        args: vec![int_const("1")],
    })];
    let f = func_returns("run", "Secret", vec![], body);
    expect_rule(&project(vec![f], vec![ty]), "TYPE_MEMBER_NOT_VISIBLE");
}

#[test]
fn rejects_construct_hidden_field_cross_file() {
    // A public type in other.mfb with a private field, constructed from main.
    let mut ty = record_typed("Widget", &[("pub", "Integer"), ("secret", "Integer")]);
    ty.file = "src/other.mfb".to_string();
    ty.fields[1].visibility = Some("private".to_string());
    let body = vec![ret(IrValue::Constructor {
        type_: crate::types::ParameterType::parse("Widget"),
        args: vec![int_const("1"), int_const("2")],
    })];
    let f = func_returns("run", "Widget", vec![], body);
    expect_rule(&project(vec![f], vec![ty]), "TYPE_MEMBER_NOT_VISIBLE");
}

#[test]
fn rejects_member_access_hidden_field() {
    // Reading a private field of a type declared in another file.
    let mut ty = record_typed("Widget", &[("pub", "Integer"), ("secret", "Integer")]);
    ty.file = "src/other.mfb".to_string();
    ty.fields[1].visibility = Some("private".to_string());
    let body = vec![ret(IrValue::MemberAccess {
        target: Box::new(IrValue::Local("w".to_string())),
        member: "secret".to_string(),
        type_: crate::types::ParameterType::parse("Unknown"),
    })];
    let f = func_returns("run", "Integer", vec![param("w", "Widget", None)], body);
    expect_rule(&project(vec![f], vec![ty]), "TYPE_MEMBER_NOT_VISIBLE");
}

#[test]
fn rejects_read_only_record_constructor() {
    // Constructing a MapEntry (read-only builtin record).
    let body = vec![ret(IrValue::Constructor {
        type_: crate::types::ParameterType::parse("MapEntry OF String TO Integer"),
        args: vec![],
    })];
    let f = func_returns("run", "Nothing", vec![], body);
    expect_rule(
        &project(vec![f], vec![]),
        "TYPE_READ_ONLY_RECORD_CONSTRUCTOR",
    );
}

// --- builtin call args: term/collections/general ---------------------------

#[test]
fn rejects_term_call_arity() {
    let body = vec![IrOp::Eval {
        value: IrValue::Call {
            target: "term.moveTo".to_string(),
            args: vec![int_const("1")],
            type_: crate::types::ParameterType::parse("Nothing"),
            loc: IrSourceLoc::default(),
        },
        loc: IrSourceLoc::default(),
    }];
    let f = func_returns("run", "Nothing", vec![], body);
    expect_rule(&project(vec![f], vec![]), "TYPE_CALL_ARITY_MISMATCH");
}

#[test]
fn rejects_term_call_argument() {
    let body = vec![IrOp::Eval {
        value: IrValue::Call {
            target: "term.moveTo".to_string(),
            args: vec![const_of("String", "a"), const_of("String", "b")],
            type_: crate::types::ParameterType::parse("Nothing"),
            loc: IrSourceLoc::default(),
        },
        loc: IrSourceLoc::default(),
    }];
    let f = func_returns("run", "Nothing", vec![], body);
    expect_rule(&project(vec![f], vec![]), "TYPE_CALL_ARGUMENT_MISMATCH");
}

#[test]
fn accepts_term_call_valid() {
    let body = vec![IrOp::Eval {
        value: IrValue::Call {
            target: "term.moveTo".to_string(),
            args: vec![int_const("1"), int_const("2")],
            type_: crate::types::ParameterType::parse("Nothing"),
            loc: IrSourceLoc::default(),
        },
        loc: IrSourceLoc::default(),
    }];
    let f = func_returns("run", "Nothing", vec![], body);
    accept(&project(vec![f], vec![]));
}

#[test]
fn rejects_collections_call_arity() {
    let body = vec![ret(IrValue::Call {
        target: "collections.append".to_string(),
        args: vec![IrValue::ListLiteral {
            type_: crate::types::ParameterType::parse("List OF Integer"),
            values: vec![],
        }],
        type_: crate::types::ParameterType::parse("Unknown"),
        loc: IrSourceLoc::default(),
    })];
    let f = func_returns("run", "Integer", vec![], body);
    expect_rule(&project(vec![f], vec![]), "TYPE_CALL_ARITY_MISMATCH");
}

#[test]
fn rejects_collections_contains_not_comparable() {
    // A list of lists is not comparable for collections.contains.
    let body = vec![ret(IrValue::Call {
        target: "collections.contains".to_string(),
        args: vec![
            IrValue::Local("xs".to_string()),
            IrValue::ListLiteral {
                type_: crate::types::ParameterType::parse("List OF Integer"),
                values: vec![],
            },
        ],
        type_: crate::types::ParameterType::parse("Boolean"),
        loc: IrSourceLoc::default(),
    })];
    let f = func_returns(
        "run",
        "Boolean",
        vec![param("xs", "List OF List OF Integer", None)],
        body,
    );
    expect_rule(&project(vec![f], vec![]), "TYPE_REQUIRES_COMPARABLE");
}

#[test]
fn rejects_general_call_arity() {
    let body = vec![ret(IrValue::Call {
        target: "len".to_string(),
        args: vec![const_of("String", "a"), const_of("String", "b")],
        type_: crate::types::ParameterType::parse("Integer"),
        loc: IrSourceLoc::default(),
    })];
    let f = func_returns("run", "Integer", vec![], body);
    expect_rule(&project(vec![f], vec![]), "TYPE_CALL_ARITY_MISMATCH");
}

#[test]
fn rejects_general_call_bad_argument() {
    // isEven on a String has no overload.
    let body = vec![ret(IrValue::Call {
        target: "isEven".to_string(),
        args: vec![const_of("String", "no")],
        type_: crate::types::ParameterType::parse("Boolean"),
        loc: IrSourceLoc::default(),
    })];
    let f = func_returns("run", "Boolean", vec![], body);
    expect_rule(&project(vec![f], vec![]), "TYPE_CALL_ARGUMENT_MISMATCH");
}

#[test]
fn accepts_general_len_string() {
    let body = vec![ret(IrValue::Call {
        target: "len".to_string(),
        args: vec![const_of("String", "abc")],
        type_: crate::types::ParameterType::parse("Integer"),
        loc: IrSourceLoc::default(),
    })];
    let f = func_returns("run", "Integer", vec![], body);
    accept(&project(vec![f], vec![]));
}

#[test]
fn rejects_strings_call_bad_args() {
    // strings.byteLen on an Integer has no overload.
    let body = vec![ret(IrValue::Call {
        target: "strings.byteLen".to_string(),
        args: vec![int_const("1")],
        type_: crate::types::ParameterType::parse("Integer"),
        loc: IrSourceLoc::default(),
    })];
    let f = func_returns("run", "Integer", vec![], body);
    let diags = rules(&project(vec![f], vec![]));
    // Either arg-mismatch or arity depending on the builtin table.
    assert!(
        diags.iter().any(|r| r.starts_with("TYPE_CALL")),
        "{diags:?}"
    );
}

// --- match covers all (block_always_returns via exhaustive enum match) ------

#[test]
fn func_returns_via_exhaustive_enum_match() {
    let m = IrOp::Match {
        value: IrValue::Local("c".to_string()),
        cases: vec![
            IrMatchCase {
                pattern: IrMatchPattern::Value(IrValue::MemberAccess {
                    target: Box::new(IrValue::Local("Color".to_string())),
                    member: "Red".to_string(),
                    type_: crate::types::ParameterType::parse("Color"),
                }),
                guard: None,
                body: vec![ret(int_const("1"))],
                loc: IrSourceLoc::default(),
            },
            IrMatchCase {
                pattern: IrMatchPattern::Value(IrValue::MemberAccess {
                    target: Box::new(IrValue::Local("Color".to_string())),
                    member: "Green".to_string(),
                    type_: crate::types::ParameterType::parse("Color"),
                }),
                guard: None,
                body: vec![ret(int_const("2"))],
                loc: IrSourceLoc::default(),
            },
        ],
        loc: IrSourceLoc::default(),
    };
    let f = func_returns("run", "Integer", vec![param("c", "Color", None)], vec![m]);
    accept(&project(
        vec![f],
        vec![enum_type("Color", &["Red", "Green"])],
    ));
}

#[test]
fn func_missing_return_when_match_not_exhaustive() {
    // MATCH covers only one enum member; func must still return -> missing.
    let m = IrOp::Match {
        value: IrValue::Local("c".to_string()),
        cases: vec![IrMatchCase {
            pattern: IrMatchPattern::Value(IrValue::MemberAccess {
                target: Box::new(IrValue::Local("Color".to_string())),
                member: "Red".to_string(),
                type_: crate::types::ParameterType::parse("Color"),
            }),
            guard: None,
            body: vec![ret(int_const("1"))],
            loc: IrSourceLoc::default(),
        }],
        loc: IrSourceLoc::default(),
    };
    let f = func_returns("run", "Integer", vec![param("c", "Color", None)], vec![m]);
    let got = rules(&project(
        vec![f],
        vec![enum_type("Color", &["Red", "Green"])],
    ));
    assert!(
        got.contains(&"TYPE_FUNC_MISSING_RETURN".to_string()),
        "{got:?}"
    );
}

// --- bug-764: a qualified enum arm is never a union-variant arm --------------

/// `CASE <enum>.<member>` as an IR pattern.
fn enum_member_case(enum_name: &str, member: &str, body: Vec<IrOp>) -> IrMatchCase {
    IrMatchCase {
        pattern: IrMatchPattern::Value(IrValue::MemberAccess {
            target: Box::new(IrValue::Local(enum_name.to_string())),
            member: member.to_string(),
            type_: crate::types::ParameterType::parse(enum_name),
        }),
        guard: None,
        body,
        loc: IrSourceLoc::default(),
    }
}

/// bug-764: `CASE Op.Add` over an `Op` scrutinee is an enum arm even when the
/// project also declares `TYPE Add` — the `Op.` qualifier decides, not the bare
/// member name. It was rejected as "CASE `Add` requires a UNION value, got Op".
#[test]
fn accepts_enum_arm_whose_member_names_a_declared_type() {
    let m = IrOp::Match {
        value: IrValue::Local("o".to_string()),
        cases: vec![
            enum_member_case("Op", "Push", vec![ret(int_const("1"))]),
            enum_member_case("Op", "Add", vec![ret(int_const("2"))]),
        ],
        loc: IrSourceLoc::default(),
    };
    let f = func_returns("run", "Integer", vec![param("o", "Op", None)], vec![m]);
    accept(&project(
        vec![f],
        vec![record("Add", &["left"]), enum_type("Op", &["Push", "Add"])],
    ));
}

/// bug-764, the reverse collision: `CASE Op.Add` over a UNION that carries a
/// variant `Add` is an `Op` value, not that variant. It was silently accepted
/// (the bare member `Add` is in the union), and native dispatch then compared
/// the union value against `Op.Add`'s ordinal.
#[test]
fn rejects_enum_arm_over_union_with_same_named_variant() {
    let m = IrOp::Match {
        value: IrValue::Local("e".to_string()),
        cases: vec![
            enum_member_case("Op", "Add", vec![ret(int_const("1"))]),
            IrMatchCase {
                pattern: IrMatchPattern::Else,
                guard: None,
                body: vec![ret(int_const("0"))],
                loc: IrSourceLoc::default(),
            },
        ],
        loc: IrSourceLoc::default(),
    };
    let f = func_returns("run", "Integer", vec![param("e", "Expr", None)], vec![m]);
    expect_rule(
        &project(
            vec![f],
            vec![
                record("Add", &["left"]),
                union("Expr", &["Add"]),
                enum_type("Op", &["Push", "Add"]),
            ],
        ),
        "TYPE_MATCH_PATTERN_MISMATCH",
    );
}

// --- link: more than one result marker -------------------------------------

#[test]
fn rejects_link_return_on_a_nothing_wrapper() {
    // plan-50-H: "more than one result marker" is now unrepresentable — there is a
    // single RETURN clause. The surviving RESULT_MARKER case is a wrapper that
    // surfaces no value yet names one.
    let mut lf = link_fn();
    lf.return_type = crate::types::ParameterType::parse("Nothing");
    lf.result = Some(crate::ir::IrLinkExpr::Var("value".to_string()));
    let mut p = project(vec![func_returns("run", "Nothing", vec![], vec![])], vec![]);
    p.link_functions = vec![lf];
    expect_rule(&p, "NATIVE_ABI_RESULT_MARKER");
}

// --- consumed via return (use-after-move on returned resource) --------------

#[test]
fn return_resource_move_is_not_use_after_move() {
    // RES h (declared, no init) then RETURN h — the Return consumes h; it is the
    // last op so this must NOT be a use-after-move. Exercises the Return-consume
    // arm of consumed_resource.
    let body = vec![
        bind("h", "fs.File", None, true, false),
        ret(IrValue::Local("h".to_string())),
    ];
    let mut f = func_returns("run", "fs.File", vec![], body);
    f.resource_owners
        .insert("h".to_string(), crate::ir::resource_escape::ResOwner::Local);
    let got = rules(&project(vec![f], vec![]));
    assert!(
        !got.contains(&"TYPE_USE_AFTER_MOVE".to_string()),
        "unexpected use-after-move: {got:?}"
    );
}

#[test]
fn rejects_double_move_close_then_return() {
    let body = vec![
        bind("h", "fs.File", None, true, false),
        IrOp::Eval {
            value: IrValue::Call {
                target: "fs.close".to_string(),
                args: vec![IrValue::Local("h".to_string())],
                type_: crate::types::ParameterType::parse("Nothing"),
                loc: IrSourceLoc::default(),
            },
            loc: IrSourceLoc::default(),
        },
        ret(IrValue::Local("h".to_string())),
    ];
    let mut f = func_returns("run", "fs.File", vec![], body);
    f.resource_owners
        .insert("h".to_string(), crate::ir::resource_escape::ResOwner::Local);
    expect_rule(&project(vec![f], vec![]), "TYPE_USE_AFTER_MOVE");
}

// --- resource element not owner (list literal + get pointer) ---------------

#[test]
fn accepts_temporary_in_resource_list() {
    // plan-59-E: was `rejects_temporary_in_resource_list`
    // (`TYPE_RESOURCE_ELEMENT_NOT_OWNER`, retired). A resource collection holds
    // pointers to resources owned by the outermost scope that touches them, so a
    // temporary is as admissible as a `RES` binding; the resource is still closed
    // exactly once, by that scope.
    // List OF RES fs.File with a non-local element (a call result).
    let body = vec![ret(IrValue::ListLiteral {
        type_: crate::types::ParameterType::parse("List OF RES fs.File"),
        values: vec![IrValue::Call {
            // `fs.openFile`, not `fs.open`: the latter takes (path, mode), and
            // the one-arg call left a TYPE_CALL_ARGUMENT_MISMATCH that the old
            // expect_rule assertion silently tolerated. Asserting cleanliness
            // surfaced it.
            target: "fs.openFile".to_string(),
            args: vec![const_of("String", "f")],
            type_: crate::types::ParameterType::parse("fs.File"),
            loc: IrSourceLoc::default(),
        }],
    })];
    let f = func_returns("run", "List OF RES fs.File", vec![], body);
    accept(&project(vec![f], vec![]));
}

// --- capture out-of-range inside a Bind value ------------------------------

#[test]
fn rejects_capture_out_of_range_in_bind_value() {
    let closure_body = func_returns(
        "body",
        "Integer",
        vec![],
        vec![
            bind(
                "x",
                "Integer",
                Some(IrValue::Capture {
                    index: 3,
                    type_: crate::types::ParameterType::parse("Integer"),
                    by_ref: false,
                }),
                false,
                false,
            ),
            ret(IrValue::Local("x".to_string())),
        ],
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

// --- bug-297: capture bounds at the value positions the walker was not called on

/// The capture-bounds defense (bug-99/bug-32) lives in `check_value_captures`,
/// a walker separate from `check_value` -- whose own `Capture` arm is a no-op.
/// Every value position in `check_ops` called both, EXCEPT MATCH case patterns
/// and WHEN guards, so an out-of-range `Capture` there passed verification and
/// lowered to `load_u64(CLOSURE_ENV_REGISTER, index*8)` -- an out-of-bounds env
/// read in the victim binary. Not front-end reachable (source lambdas lower to a
/// single RETURN), so this is purely a crafted-`.mfp` trust-boundary gap.
#[test]
fn rejects_capture_out_of_range_in_match_pattern_and_guard() {
    // One capture slot exists; index 9999 is far outside it.
    let oob = || IrValue::Capture {
        index: 9999,
        type_: crate::types::ParameterType::parse("Integer"),
        by_ref: false,
    };
    let maker = |body: &str| {
        func_returns(
            "make",
            "FUNC() AS Integer",
            vec![],
            vec![ret(IrValue::Closure {
                name: body.to_string(),
                type_: crate::types::ParameterType::parse("FUNC() AS Integer"),
                captures: vec![int_const("1")],
            })],
        )
    };

    // (a) a pattern VALUE position
    let pattern_body = func_returns(
        "body",
        "Integer",
        vec![],
        vec![
            IrOp::Match {
                value: int_const("1"),
                cases: vec![
                    IrMatchCase {
                        pattern: IrMatchPattern::Value(oob()),
                        guard: None,
                        body: vec![ret(int_const("0"))],
                        loc: IrSourceLoc::default(),
                    },
                    IrMatchCase {
                        pattern: IrMatchPattern::Else,
                        guard: None,
                        body: vec![ret(int_const("0"))],
                        loc: IrSourceLoc::default(),
                    },
                ],
                loc: IrSourceLoc::default(),
            },
            ret(int_const("0")),
        ],
    );
    let err = check(&project(vec![pattern_body, maker("body")], vec![]))
        .expect_err("an out-of-range capture in a MATCH pattern must be rejected");
    assert!(err.contains("out of range"), "{err}");

    // (b) a WHEN guard
    let guard_body = func_returns(
        "body",
        "Integer",
        vec![],
        vec![
            IrOp::Match {
                value: int_const("1"),
                cases: vec![
                    IrMatchCase {
                        pattern: IrMatchPattern::Value(int_const("1")),
                        guard: Some(oob()),
                        body: vec![ret(int_const("0"))],
                        loc: IrSourceLoc::default(),
                    },
                    IrMatchCase {
                        pattern: IrMatchPattern::Else,
                        guard: None,
                        body: vec![ret(int_const("0"))],
                        loc: IrSourceLoc::default(),
                    },
                ],
                loc: IrSourceLoc::default(),
            },
            ret(int_const("0")),
        ],
    );
    let err = check(&project(vec![guard_body, maker("body")], vec![]))
        .expect_err("an out-of-range capture in a WHEN guard must be rejected");
    assert!(err.contains("out of range"), "{err}");

    // (c) an IN-RANGE capture in the same positions still verifies, so the new
    // calls reject the crafted shape rather than closures generally.
    let ok_body = func_returns(
        "body",
        "Integer",
        vec![],
        vec![
            IrOp::Match {
                value: int_const("1"),
                cases: vec![
                    IrMatchCase {
                        pattern: IrMatchPattern::Value(IrValue::Capture {
                            index: 0,
                            type_: crate::types::ParameterType::parse("Integer"),
                            by_ref: false,
                        }),
                        guard: None,
                        body: vec![ret(int_const("0"))],
                        loc: IrSourceLoc::default(),
                    },
                    IrMatchCase {
                        pattern: IrMatchPattern::Else,
                        guard: None,
                        body: vec![ret(int_const("0"))],
                        loc: IrSourceLoc::default(),
                    },
                ],
                loc: IrSourceLoc::default(),
            },
            ret(int_const("0")),
        ],
    );
    accept(&project(vec![ok_body, maker("body")], vec![]));
}

/// A parameter default is evaluated in the CALLER's frame and a global
/// initializer runs before any closure exists, so neither has a captured
/// environment at all -- any `Capture` in one is malformed IR that would lower to
/// an env-relative load off whatever the env register happens to hold. Both
/// positions called `check_value` alone.
#[test]
fn rejects_stray_capture_in_parameter_default_and_global_initializer() {
    let stray = || IrValue::Capture {
        index: 0,
        type_: crate::types::ParameterType::parse("Integer"),
        by_ref: false,
    };

    let with_default = func_returns(
        "run",
        "Integer",
        vec![param("n", "Integer", Some(stray()))],
        vec![ret(int_const("0"))],
    );
    let err = check(&project(vec![with_default], vec![]))
        .expect_err("a capture in a parameter default must be rejected");
    assert!(err.contains("not a closure body"), "{err}");

    let mut with_global = project(
        vec![func_returns(
            "run",
            "Integer",
            vec![],
            vec![ret(int_const("0"))],
        )],
        vec![],
    );
    with_global.bindings = vec![binding("g", "Integer", Some(stray()), false, true)];
    let err = check(&with_global).expect_err("a capture in a global initializer must be rejected");
    assert!(err.contains("not a closure body"), "{err}");
}

// --- infer_type through global + walk_captures over many value shapes -------

#[test]
fn accepts_global_read_in_function() {
    let mut p = project(
        vec![func_returns(
            "run",
            "Integer",
            vec![],
            vec![ret(IrValue::Global("g".to_string()))],
        )],
        vec![],
    );
    p.bindings = vec![binding("g", "Integer", Some(int_const("5")), false, true)];
    accept(&p);
}

#[test]
fn captures_walked_through_nested_value_shapes() {
    // A closure whose captures include nested constructors/lists/maps/binary,
    // with an out-of-range capture buried inside — exercises walk_captures arms.
    let closure_body = func_returns(
        "body",
        "Integer",
        vec![],
        vec![ret(IrValue::Binary {
            op: BinaryOp::Add,
            left: Box::new(IrValue::Capture {
                index: 0,
                type_: crate::types::ParameterType::parse("Integer"),
                by_ref: false,
            }),
            right: Box::new(IrValue::Capture {
                index: 9,
                type_: crate::types::ParameterType::parse("Integer"),
                by_ref: false,
            }),
            type_: crate::types::ParameterType::parse("Integer"),
            loc: IrSourceLoc::default(),
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
    let err = check(&project(vec![closure_body, maker], vec![]))
        .expect_err("nested capture out of range");
    assert!(err.contains("out of range"), "{err}");
}
