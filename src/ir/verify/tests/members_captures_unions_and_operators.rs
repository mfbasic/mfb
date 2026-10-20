//! plan-176-G: `ir::verify::tests` — member access and call arity, capture bounds, union wrap
//! and annotations, function returns and params, and operators. The shared builders and
//! fixtures live in `verify/tests/mod.rs`.

use super::*;

// --- member access ---------------------------------------------------------

#[test]
fn accepts_member_access_on_known_record_field() {
    let body = vec![IrOp::Return {
        value: Some(IrValue::MemberAccess {
            target: Box::new(IrValue::Local("p".to_string())),
            member: "x".to_string(),
            type_: crate::types::ParameterType::parse("Unknown"),
        }),
        loc: IrSourceLoc::default(),
    }];
    let f = func("run", vec![param("p", "Point", None)], body);
    check(&project(vec![f], vec![record("Point", &["x", "y"])])).expect("valid member access");
}

#[test]
fn rejects_member_access_on_integer() {
    // The PKG-02 attack shape: a member access on a primitive local.
    let body = vec![IrOp::Return {
        value: Some(IrValue::MemberAccess {
            target: Box::new(int_const("0")),
            member: "x".to_string(),
            type_: crate::types::ParameterType::parse("Unknown"),
        }),
        loc: IrSourceLoc::default(),
    }];
    let f = func("run", vec![], body);
    let err = check(&project(vec![f], vec![])).expect_err("member on Integer must be rejected");
    assert!(err.contains("TYPE_FIELD_ACCESS_REQUIRES_RECORD"), "{err}");
}

#[test]
fn rejects_member_access_on_money_and_scalar() {
    // bug-190: PRIMITIVE_TYPES omitted Money (plan-29) and Scalar (plan-41), so
    // a crafted `.mfp` could smuggle a MemberAccess on a Money/Scalar-typed
    // local past merge_packages' verifier and reach codegen as an offset load on
    // a scalar register value (PKG-02 type confusion / OOB read). Both must be
    // rejected exactly as Integer is.
    for ty in ["Money", "Scalar"] {
        let body = vec![IrOp::Return {
            value: Some(IrValue::MemberAccess {
                target: Box::new(IrValue::Local("v".to_string())),
                member: "x".to_string(),
                type_: crate::types::ParameterType::parse("Integer"),
            }),
            loc: IrSourceLoc::default(),
        }];
        let f = func("run", vec![param("v", ty, None)], body);
        let err = check(&project(vec![f], vec![]))
            .err()
            .unwrap_or_else(|| panic!("member on {ty} must be rejected"));
        assert!(
            err.contains("TYPE_FIELD_ACCESS_REQUIRES_RECORD"),
            "{ty}: {err}"
        );
    }
}

#[test]
fn rejects_member_access_missing_field_on_record() {
    let body = vec![IrOp::Return {
        value: Some(IrValue::MemberAccess {
            target: Box::new(IrValue::Local("p".to_string())),
            member: "z".to_string(),
            type_: crate::types::ParameterType::parse("Unknown"),
        }),
        loc: IrSourceLoc::default(),
    }];
    let f = func("run", vec![param("p", "Point", None)], body);
    let err = check(&project(vec![f], vec![record("Point", &["x", "y"])]))
        .expect_err("missing field must be rejected");
    assert!(err.contains("no member `z`"), "{err}");
}

#[test]
fn skips_member_access_on_unknown_type() {
    // A member access whose target type is not a known record is left alone so
    // the checker never rejects IR whose types it cannot reconstruct.
    let body = vec![IrOp::Return {
        value: Some(IrValue::MemberAccess {
            target: Box::new(IrValue::Local("w".to_string())),
            member: "anything".to_string(),
            type_: crate::types::ParameterType::parse("Unknown"),
        }),
        loc: IrSourceLoc::default(),
    }];
    let f = func("run", vec![param("w", "Widget", None)], body);
    check(&project(vec![f], vec![])).expect("unknown target type is skipped");
}

// --- call arity ------------------------------------------------------------

#[test]
fn rejects_call_with_too_many_arguments() {
    // `Nothing`-returning: an empty body must not trip TYPE_FUNC_MISSING_RETURN.
    let callee = func_returns(
        "helper",
        "Nothing",
        vec![param("a", "Integer", None)],
        vec![],
    );
    let body = vec![IrOp::Return {
        value: Some(IrValue::Call {
            target: "helper".to_string(),
            args: vec![int_const("1"), int_const("2")],
            loc: IrSourceLoc::default(),
            type_: crate::types::ParameterType::parse("Unknown"),
        }),
        loc: IrSourceLoc::default(),
    }];
    let caller = func("run", vec![], body);
    let err = check(&project(vec![callee, caller], vec![]))
        .expect_err("over-arity call must be rejected");
    assert!(err.contains("Call to `helper`"), "{err}");
}

#[test]
fn accepts_call_omitting_defaulted_argument() {
    let callee = func_returns(
        "helper",
        "Nothing",
        vec![
            param("a", "Integer", None),
            param("b", "Integer", Some(int_const("0"))),
        ],
        vec![],
    );
    let body = vec![IrOp::Return {
        value: Some(IrValue::Call {
            target: "helper".to_string(),
            args: vec![int_const("1")],
            loc: IrSourceLoc::default(),
            type_: crate::types::ParameterType::parse("Unknown"),
        }),
        loc: IrSourceLoc::default(),
    }];
    let caller = func("run", vec![], body);
    check(&project(vec![callee, caller], vec![])).expect("omitting a default is valid");
}

#[test]
fn skips_arity_for_unknown_call_targets() {
    // A dotted target whose module is neither a known builtin package nor an
    // internal function is left alone: the checker cannot reconstruct its
    // signature, so it never invents an arity/argument rejection. (`io.print`
    // would resolve as a real builtin and be argument-checked, so use a name
    // that resolves to nothing.)
    let body = vec![IrOp::Return {
        value: Some(IrValue::Call {
            target: "mystery.helper".to_string(),
            args: vec![int_const("1"), int_const("2"), int_const("3")],
            loc: IrSourceLoc::default(),
            type_: crate::types::ParameterType::parse("Unknown"),
        }),
        loc: IrSourceLoc::default(),
    }];
    let f = func("run", vec![], body);
    check(&project(vec![f], vec![])).expect("unknown call target is skipped");
}

// --- constructor arity -----------------------------------------------------

#[test]
fn rejects_constructor_with_extra_arguments() {
    let body = vec![IrOp::Return {
        value: Some(IrValue::Constructor {
            type_: crate::types::ParameterType::parse("Point"),
            args: vec![int_const("1"), int_const("2"), int_const("3")],
        }),
        loc: IrSourceLoc::default(),
    }];
    let f = func("run", vec![], body);
    let err = check(&project(vec![f], vec![record("Point", &["x", "y"])]))
        .expect_err("over-arity constructor must be rejected");
    assert!(err.contains("Constructor `Point`"), "{err}");
}

// --- capture bounds --------------------------------------------------------

#[test]
fn rejects_capture_index_past_slot_count() {
    // `make` creates a closure `body` with one captured slot; `body` reads slot 5.
    let closure_body = func(
        "body",
        vec![],
        vec![IrOp::Return {
            value: Some(IrValue::Capture {
                index: 5,
                type_: crate::types::ParameterType::parse("Integer"),
                by_ref: false,
            }),
            loc: IrSourceLoc::default(),
        }],
    );
    let maker = func_returns(
        "make",
        "FUNC() AS Integer",
        vec![],
        vec![IrOp::Return {
            value: Some(IrValue::Closure {
                name: "body".to_string(),
                type_: crate::types::ParameterType::parse("FUNC() AS Integer"),
                captures: vec![int_const("7")],
            }),
            loc: IrSourceLoc::default(),
        }],
    );
    let err = check(&project(vec![closure_body, maker], vec![]))
        .expect_err("out-of-range capture must be rejected");
    assert!(err.contains("capture index 5"), "{err}");
}

#[test]
fn accepts_capture_index_within_slot_count() {
    let closure_body = func(
        "body",
        vec![],
        vec![IrOp::Return {
            value: Some(IrValue::Capture {
                index: 0,
                type_: crate::types::ParameterType::parse("Integer"),
                by_ref: false,
            }),
            loc: IrSourceLoc::default(),
        }],
    );
    let maker = func_returns(
        "make",
        "FUNC() AS Integer",
        vec![],
        vec![IrOp::Return {
            value: Some(IrValue::Closure {
                name: "body".to_string(),
                type_: crate::types::ParameterType::parse("FUNC() AS Integer"),
                captures: vec![int_const("7")],
            }),
            loc: IrSourceLoc::default(),
        }],
    );
    check(&project(vec![closure_body, maker], vec![])).expect("in-range capture is valid");
}

/// bug-99: a `Capture` sitting in a function that is never targeted by any
/// `Closure` node has no captured environment, so the slot count was `None` and
/// the bounds check was skipped entirely — a crafted `.mfp` could then drive an
/// out-of-bounds env-relative load off `CLOSURE_ENV_REGISTER` at an
/// attacker-chosen index. The legitimate front end never emits such a `Capture`
/// (zero-capture lambdas lower to a plain `FunctionRef`), so it is malformed IR
/// and must be rejected.
#[test]
fn rejects_capture_in_non_closure_body() {
    // `f` is a plain function (no `Closure` node names it) yet reads a capture.
    let f = func_returns(
        "f",
        "FUNC() AS Integer",
        vec![],
        vec![IrOp::Return {
            value: Some(IrValue::Capture {
                index: 9999,
                type_: crate::types::ParameterType::parse("Integer"),
                by_ref: false,
            }),
            loc: IrSourceLoc::default(),
        }],
    );
    let err = check(&project(vec![f], vec![]))
        .expect_err("a capture outside any closure body must be rejected");
    assert!(err.contains("capture index 9999"), "{err}");
}

/// bug-32: two closures over one body with differing capture counts used to make
/// the slot count "ambiguous", which skipped the bounds check entirely — so the
/// body could read `Capture{index: 9999}` off the end of its environment.
#[test]
fn ambiguous_closure_arity_does_not_disarm_the_capture_bounds_check() {
    let closure_body = func(
        "body",
        vec![],
        vec![ret(IrValue::Capture {
            index: 9999,
            type_: crate::types::ParameterType::parse("Integer"),
            by_ref: false,
        })],
    );
    let closure = |captures: Vec<IrValue>| IrValue::Closure {
        name: "body".to_string(),
        type_: crate::types::ParameterType::parse("FUNC() AS Integer"),
        captures,
    };
    let maker = func_returns(
        "make",
        "FUNC() AS Integer",
        vec![],
        vec![
            bind(
                "one",
                "FUNC() AS Integer",
                Some(closure(vec![int_const("7")])),
                true,
                false,
            ),
            bind(
                "two",
                "FUNC() AS Integer",
                Some(closure(vec![int_const("7"), int_const("8")])),
                true,
                false,
            ),
            ret(IrValue::Local("one".to_string())),
        ],
    );
    let diags = collect_diagnostics(&project(vec![closure_body, maker], vec![]));
    let details = diags.iter().map(|d| &d.detail).collect::<Vec<_>>();
    // The index is bounded by the smallest capture vector, and the
    // front-end-impossible differing arity is itself reported.
    assert!(
        details.iter().any(|d| d.contains("capture index 9999")),
        "{details:?}"
    );
    assert!(
        details
            .iter()
            .any(|d| d.contains("differing capture counts (1, 2)")),
        "{details:?}"
    );
}

/// The ambiguous shape is rejected even when every capture index is in range for
/// the smaller environment — lowering never produces it.
#[test]
fn a_body_captured_with_two_arities_is_rejected() {
    let closure_body = func(
        "body",
        vec![],
        vec![ret(IrValue::Capture {
            index: 0,
            type_: crate::types::ParameterType::parse("Integer"),
            by_ref: false,
        })],
    );
    let closure = |captures: Vec<IrValue>| IrValue::Closure {
        name: "body".to_string(),
        type_: crate::types::ParameterType::parse("FUNC() AS Integer"),
        captures,
    };
    let maker = func_returns(
        "make",
        "FUNC() AS Integer",
        vec![],
        vec![
            bind(
                "one",
                "FUNC() AS Integer",
                Some(closure(vec![int_const("7")])),
                true,
                false,
            ),
            bind(
                "two",
                "FUNC() AS Integer",
                Some(closure(vec![int_const("7"), int_const("8")])),
                true,
                false,
            ),
            ret(IrValue::Local("one".to_string())),
        ],
    );
    let err = check(&project(vec![closure_body, maker], vec![]))
        .expect_err("differing capture arities must be rejected");
    assert!(err.contains("differing capture counts"), "{err}");
}

/// plan-13-A: a variant type name widens to its resource union in a
/// (non-owning) parameter position, but the reverse is rejected. `compatible`
/// is the sole seam that decides this on the IR call-argument path
/// (`check_call_argument_types`) and the return path — the directionality here
/// is what keeps every concrete-typed close op, `thread::transfer`, and
/// `thread::accept` unreachable by a whole union (a use-after-free class bug if
/// widening were symmetric). The `RES` ownership marker is stripped on both
/// sides, so a `RES`-marked parameter widens identically.
#[test]
fn resource_union_parameter_widening_is_directional() {
    let proj = project(vec![], vec![union("Stream", &["fs.File", "Socket"])]);
    let env = super::TypeEnv::build(&proj);
    // variant -> union (widen): accepted
    assert!(
        env.compatible(
            &ParameterType::parse("Stream"),
            &ParameterType::parse("fs.File")
        ),
        "File must widen to Stream"
    );
    assert!(
        env.compatible(
            &ParameterType::parse("Stream"),
            &ParameterType::parse("Socket")
        ),
        "Socket must widen to Stream"
    );
    // union -> concrete (reverse): rejected
    assert!(
        !env.compatible(
            &ParameterType::parse("fs.File"),
            &ParameterType::parse("Stream")
        ),
        "a Stream union must not narrow into a concrete File parameter"
    );
    assert!(
        !env.compatible(
            &ParameterType::parse("Socket"),
            &ParameterType::parse("Stream")
        ),
        "a Stream union must not narrow into a concrete Socket parameter"
    );
    // the RES ownership marker is stripped, so widening/narrowing is unchanged
    assert!(
        env.compatible(
            &ParameterType::parse("RES Stream"),
            &ParameterType::parse("RES fs.File")
        ),
        "the RES marker must not block variant->union widening"
    );
    assert!(
        !env.compatible(
            &ParameterType::parse("RES fs.File"),
            &ParameterType::parse("RES Stream")
        ),
        "the RES marker must not enable union->concrete narrowing"
    );
}

/// bug-786: an `ISOLATED FUNC` is assignable to the ordinary `FUNC` of the same
/// shape (isolation is an extra guarantee). The conversion is one-way, needs the
/// same parameters and return, and is NOT part of `compatible`. That one also
/// judges package annotations against derived types, where the two must agree.
#[test]
fn an_isolated_callable_is_assignable_to_the_ordinary_one_only() {
    let proj = project(vec![], vec![]);
    let env = super::TypeEnv::build(&proj);
    let value = IrValue::Local("f".to_string());
    let plain = ParameterType::parse("FUNC(Integer) AS Integer");
    let isolated = ParameterType::parse("ISOLATED FUNC(Integer) AS Integer");
    assert!(
        env.assignable(&plain, &isolated, &value),
        "isolated -> ordinary"
    );
    assert!(
        !env.assignable(&isolated, &plain, &value),
        "ordinary -> isolated"
    );
    assert!(
        !env.assignable(
            &ParameterType::parse("FUNC(String) AS Integer"),
            &isolated,
            &value
        ),
        "different parameters"
    );
    assert!(
        !env.assignable(
            &ParameterType::parse("FUNC(Integer) AS String"),
            &isolated,
            &value
        ),
        "different return"
    );
    assert!(
        !env.compatible(&plain, &isolated),
        "annotation integrity still requires agreement"
    );
}

#[test]
fn rejects_union_wrap_of_foreign_variant() {
    let body = vec![IrOp::Return {
        value: Some(IrValue::UnionWrap {
            union_type: crate::types::ParameterType::parse("Shape"),
            member_type: crate::types::ParameterType::parse("Ghost"),
            value: Box::new(int_const("0")),
        }),
        loc: IrSourceLoc::default(),
    }];
    let f = func_returns("run", "Shape", vec![], body);
    let err = check(&project(
        vec![f],
        vec![union("Shape", &["Circle", "Square"])],
    ))
    .expect_err("foreign variant must be rejected");
    assert!(err.contains("not a variant of union `Shape`"), "{err}");
}

#[test]
fn accepts_union_wrap_of_real_variant() {
    // A real variant tag wrapping a correctly-typed payload. bug-404 added the
    // payload-type reconciliation; real lowering sets `member_type` to the
    // wrapped value's own type (lower.rs:3312), so the payload must be a
    // `Circle` here — the previous `int_const("0")` payload was never legitimate
    // IR, it only exercised the tag check in isolation.
    let body = vec![IrOp::Return {
        value: Some(IrValue::UnionWrap {
            union_type: crate::types::ParameterType::parse("Shape"),
            member_type: crate::types::ParameterType::parse("Circle"),
            value: Box::new(IrValue::Local("c".to_string())),
        }),
        loc: IrSourceLoc::default(),
    }];
    let f = func_returns("run", "Shape", vec![param("c", "Circle", None)], body);
    check(&project(
        vec![f],
        vec![union("Shape", &["Circle", "Square"])],
    ))
    .expect("real variant is valid");
}

// --- bug-404: reconcile ResultValue/UnionWrap/WithUpdate annotations --------
// The IR verifier is the sole safety net for untrusted imported-package IR
// (`check()` runs on decoded `.mfp`). bug-162 reconciled `UnionExtract` (the
// read side); these three sibling sites — `ResultValue.type_`, the `UnionWrap`
// payload, and `WithUpdate.type_` — trusted their attacker-controlled
// annotation against the actual value, allowing type/layout confusion.

#[test]
fn rejects_result_value_with_fabricated_success_type() {
    // A `ResultValue` annotated `Account` over a `Result OF Integer` — the
    // annotation disagrees with the Result's real element type. `infer_type`
    // trusts the annotation, so a later member access reads `Account`'s record
    // layout off an Integer.
    let body = vec![ret(IrValue::ResultValue {
        type_: crate::types::ParameterType::parse("Account"),
        value: Box::new(IrValue::Local("r".to_string())),
    })];
    let f = func_returns(
        "run",
        "Account",
        vec![param("r", "#Result OF Integer", None)],
        body,
    );
    let err = check(&project(vec![f], vec![record("Account", &["balance"])]))
        .expect_err("fabricated ResultValue success type must be rejected");
    assert!(err.contains("Account") || err.contains("Integer"), "{err}");
}

#[test]
fn accepts_result_value_with_matching_success_type() {
    let body = vec![ret(IrValue::ResultValue {
        type_: crate::types::ParameterType::parse("Integer"),
        value: Box::new(IrValue::Local("r".to_string())),
    })];
    let f = func_returns(
        "run",
        "Integer",
        vec![param("r", "#Result OF Integer", None)],
        body,
    );
    check(&project(vec![f], vec![])).expect("matching success type is valid");
}

#[test]
fn rejects_union_wrap_with_mismatched_payload() {
    // `Circle` is a real variant of `Shape`, but the wrapped value is an
    // Integer, not a `Circle`. A later MATCH/UnionExtract reads `Circle`'s
    // layout off the Integer — the read side is guarded (bug-162); this is the
    // wrap side.
    let body = vec![ret(IrValue::UnionWrap {
        union_type: crate::types::ParameterType::parse("Shape"),
        member_type: crate::types::ParameterType::parse("Circle"),
        value: Box::new(int_const("0")),
    })];
    let f = func_returns("run", "Shape", vec![], body);
    let err = check(&project(
        vec![f],
        vec![
            union("Shape", &["Circle", "Square"]),
            record("Circle", &["r"]),
        ],
    ))
    .expect_err("mismatched UnionWrap payload must be rejected");
    assert!(err.contains("Circle") || err.contains("Integer"), "{err}");
}

#[test]
fn rejects_with_update_with_fabricated_type() {
    // `type_` claims `Account`, but the target is a `Widget`. The update is
    // checked entirely against `Account`'s fields and `infer_type` returns the
    // trusted `Account`, so codegen updates by `Account`'s offsets.
    let body = vec![ret(IrValue::WithUpdate {
        type_: crate::types::ParameterType::parse("Account"),
        target: Box::new(IrValue::Local("b".to_string())),
        updates: vec![],
    })];
    let f = func_returns("run", "Account", vec![param("b", "Widget", None)], body);
    let err = check(&project(
        vec![f],
        vec![record("Account", &["balance"]), record("Widget", &["size"])],
    ))
    .expect_err("fabricated WithUpdate type must be rejected");
    assert!(err.contains("Account") || err.contains("Widget"), "{err}");
}

#[test]
fn accepts_with_update_matching_target_type() {
    let body = vec![ret(IrValue::WithUpdate {
        type_: crate::types::ParameterType::parse("Account"),
        target: Box::new(IrValue::Local("a".to_string())),
        updates: vec![],
    })];
    let f = func_returns("run", "Account", vec![param("a", "Account", None)], body);
    check(&project(vec![f], vec![record("Account", &["balance"])]))
        .expect("matching WithUpdate target type is valid");
}

// --- match -----------------------------------------------------------------

#[test]
fn rejects_empty_match() {
    let body = vec![IrOp::Match {
        value: int_const("0"),
        cases: vec![],
        loc: IrSourceLoc::default(),
    }];
    // `Nothing`-returning so the empty-match rejection is the first (and
    // only) diagnostic rather than TYPE_FUNC_MISSING_RETURN.
    let f = func_returns("run", "Nothing", vec![], body);
    let err = check(&project(vec![f], vec![])).expect_err("empty match must be rejected");
    assert!(err.contains("MATCH has no cases"), "{err}");
}

// --- a realistic accept ----------------------------------------------------

#[test]
fn accepts_ordinary_function() {
    let body = vec![
        IrOp::Bind {
            mutable: false,
            name: "n".to_string(),
            type_: ParameterType::Integer,
            value: Some(int_const("1")),
            loc: IrSourceLoc::default(),
            explicit_type: false,
        },
        IrOp::Return {
            value: Some(IrValue::Binary {
                op: BinaryOp::Add,
                left: Box::new(IrValue::Local("n".to_string())),
                right: Box::new(int_const("2")),
                loc: IrSourceLoc::default(),
                type_: crate::types::ParameterType::parse("Unknown"),
            }),
            loc: IrSourceLoc::default(),
        },
    ];
    let f = func("run", vec![], body);
    check(&project(vec![f], vec![])).expect("ordinary function is valid");
}

// --- function-level return rules -------------------------------------------

#[test]
fn rejects_func_missing_return_type() {
    // A `func` whose return is "Unknown" (no AS clause) is rejected.
    let f = func_returns("run", "Unknown", vec![], vec![]);
    expect_rule(&project(vec![f], vec![]), "TYPE_FUNC_REQUIRES_RETURN_TYPE");
}

#[test]
fn rejects_func_missing_return_value() {
    // A value FUNC that never returns on all paths.
    let f = func_returns(
        "run",
        "Integer",
        vec![],
        vec![bind("x", "Integer", Some(int_const("1")), false, false)],
    );
    expect_rule(&project(vec![f], vec![]), "TYPE_FUNC_MISSING_RETURN");
}

#[test]
fn accepts_func_returning_on_all_paths_via_if() {
    let body = vec![IrOp::If {
        condition: const_of("Boolean", "true"),
        then_body: vec![ret(int_const("1"))],
        else_body: vec![ret(int_const("2"))],
        loc: IrSourceLoc::default(),
    }];
    accept(&project(vec![func("run", vec![], body)], vec![]));
}

#[test]
fn nothing_func_may_fall_through() {
    accept(&project(
        vec![func_returns("run", "Nothing", vec![], vec![])],
        vec![],
    ));
}

// --- parameters ------------------------------------------------------------

#[test]
fn rejects_param_missing_type() {
    let f = func_returns("run", "Nothing", vec![param("a", "Unknown", None)], vec![]);
    expect_rule(&project(vec![f], vec![]), "TYPE_PARAM_REQUIRES_TYPE");
}

#[test]
fn rejects_default_arg_order() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![
            param("a", "Integer", Some(int_const("0"))),
            param("b", "Integer", None),
        ],
        vec![],
    );
    expect_rule(&project(vec![f], vec![]), "TYPE_DEFAULT_ARG_ORDER");
}

#[test]
fn rejects_default_value_mismatch() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![param("a", "Integer", Some(const_of("String", "hi")))],
        vec![],
    );
    expect_rule(&project(vec![f], vec![]), "TYPE_DEFAULT_VALUE_MISMATCH");
}

#[test]
fn accepts_matching_default_value() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![param("a", "Integer", Some(int_const("0")))],
        vec![],
    );
    accept(&project(vec![f], vec![]));
}

// --- binary operators ------------------------------------------------------

#[test]
fn rejects_arithmetic_on_string() {
    let body = vec![ret(binary(
        BinaryOp::Subtract,
        const_of("String", "a"),
        int_const("1"),
        "Integer",
    ))];
    expect_rule(
        &project(vec![func("run", vec![], body)], vec![]),
        "TYPE_BINARY_OPERATOR_MISMATCH",
    );
}

#[test]
fn rejects_and_on_numeric() {
    let body = vec![ret(binary(
        BinaryOp::And,
        int_const("1"),
        int_const("2"),
        "Boolean",
    ))];
    expect_rule(
        &project(vec![func_returns("run", "Boolean", vec![], body)], vec![]),
        "TYPE_BINARY_OPERATOR_MISMATCH",
    );
}

#[test]
fn rejects_concat_on_numeric() {
    let body = vec![ret(binary(
        BinaryOp::Concat,
        int_const("1"),
        int_const("2"),
        "String",
    ))];
    expect_rule(
        &project(vec![func_returns("run", "String", vec![], body)], vec![]),
        "TYPE_BINARY_OPERATOR_MISMATCH",
    );
}

#[test]
fn rejects_relational_on_boolean() {
    let body = vec![ret(binary(
        BinaryOp::Less,
        const_of("Boolean", "true"),
        const_of("Boolean", "false"),
        "Boolean",
    ))];
    expect_rule(
        &project(vec![func_returns("run", "Boolean", vec![], body)], vec![]),
        "TYPE_BINARY_OPERATOR_MISMATCH",
    );
}

#[test]
fn accepts_string_relational() {
    let body = vec![ret(binary(
        BinaryOp::Less,
        const_of("String", "a"),
        const_of("String", "b"),
        "Boolean",
    ))];
    accept(&project(
        vec![func_returns("run", "Boolean", vec![], body)],
        vec![],
    ));
}

#[test]
fn accepts_string_concat() {
    let body = vec![ret(binary(
        BinaryOp::Concat,
        const_of("String", "a"),
        const_of("String", "b"),
        "String",
    ))];
    accept(&project(
        vec![func_returns("run", "String", vec![], body)],
        vec![],
    ));
}

#[test]
fn accepts_boolean_and() {
    let body = vec![ret(binary(
        BinaryOp::And,
        const_of("Boolean", "true"),
        const_of("Boolean", "false"),
        "Boolean",
    ))];
    accept(&project(
        vec![func_returns("run", "Boolean", vec![], body)],
        vec![],
    ));
}

#[test]
fn rejects_equality_incompatible_types() {
    let body = vec![ret(binary(
        BinaryOp::Equal,
        const_of("String", "a"),
        int_const("1"),
        "Boolean",
    ))];
    expect_rule(
        &project(vec![func_returns("run", "Boolean", vec![], body)], vec![]),
        "TYPE_BINARY_OPERATOR_MISMATCH",
    );
}

#[test]
fn rejects_equality_not_comparable() {
    // Two lists are compatible but not comparable.
    let body = vec![ret(binary(
        BinaryOp::Equal,
        IrValue::ListLiteral {
            type_: crate::types::ParameterType::parse("List OF Integer"),
            values: vec![],
        },
        IrValue::ListLiteral {
            type_: crate::types::ParameterType::parse("List OF Integer"),
            values: vec![],
        },
        "Boolean",
    ))];
    expect_rule(
        &project(vec![func_returns("run", "Boolean", vec![], body)], vec![]),
        "TYPE_REQUIRES_COMPARABLE",
    );
}

#[test]
fn accepts_numeric_equality() {
    let body = vec![ret(binary(
        BinaryOp::Equal,
        int_const("1"),
        int_const("2"),
        "Boolean",
    ))];
    accept(&project(
        vec![func_returns("run", "Boolean", vec![], body)],
        vec![],
    ));
}

// --- unary operators -------------------------------------------------------

#[test]
fn rejects_not_on_numeric() {
    let body = vec![ret(unary(UnaryOp::Not, int_const("1"), "Boolean"))];
    expect_rule(
        &project(vec![func_returns("run", "Boolean", vec![], body)], vec![]),
        "TYPE_UNARY_OPERATOR_MISMATCH",
    );
}

#[test]
fn rejects_negate_on_string() {
    let body = vec![ret(unary(
        UnaryOp::Negate,
        const_of("String", "a"),
        "Integer",
    ))];
    expect_rule(
        &project(vec![func("run", vec![], body)], vec![]),
        "TYPE_UNARY_OPERATOR_MISMATCH",
    );
}

// plan-112: this used to feed the operator `~` — a spelling outside the
// vocabulary — which `UnaryOp` no longer lets anyone construct. The rule it
// pins is still live and still reachable: `SIZEOF` is a LINK-only operator that
// folds to an integer during LINK lowering, so an IR node still carrying one is
// malformed exactly as `~` was, and is the operator this arm now reports. The
// out-of-vocabulary case it also covered moved to the decode boundary, where a
// `.mfp` carrying a garbage operator string is now rejected — see
// `binary::value_op_tests::decode_rejects_garbage_binary_and_unary_ops`.
#[test]
fn rejects_unknown_unary_operator() {
    let body = vec![ret(unary(UnaryOp::SizeOf, int_const("1"), "Integer"))];
    expect_rule(
        &project(vec![func("run", vec![], body)], vec![]),
        "TYPE_UNARY_OPERATOR_UNKNOWN",
    );
}

#[test]
fn accepts_not_on_boolean_and_negate_numeric() {
    let body = vec![
        bind(
            "b",
            "Boolean",
            Some(unary(UnaryOp::Not, const_of("Boolean", "true"), "Boolean")),
            false,
            false,
        ),
        ret(unary(UnaryOp::Negate, int_const("1"), "Integer")),
    ];
    accept(&project(vec![func("run", vec![], body)], vec![]));
}
