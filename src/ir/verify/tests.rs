use super::{check, collect_diagnostics};
use crate::ir::{
    IrBinding, IrField, IrFunction, IrMatchCase, IrMatchPattern, IrOp, IrParam, IrProject,
    IrSourceLoc, IrType, IrValue, IrVariant, ProjectDocs,
};
use std::collections::HashMap;

fn project(functions: Vec<IrFunction>, types: Vec<IrType>) -> IrProject {
    IrProject {
        name: "t".to_string(),
        entry: None,
        bindings: vec![],
        types,
        functions,
        native_resources: vec![],
        link_functions: vec![],
        link_aliases: vec![],
        docs: ProjectDocs::default(),
    }
}

/// Rule ids of every diagnostic collected for a project.
fn rules(project: &IrProject) -> Vec<String> {
    collect_diagnostics(project)
        .into_iter()
        .map(|d| d.rule)
        .collect()
}

/// Assert `project` yields no diagnostics.
fn accept(project: &IrProject) {
    let diags = collect_diagnostics(project);
    assert!(
        diags.is_empty(),
        "expected clean, got {:?}",
        diags.iter().map(|d| &d.rule).collect::<Vec<_>>()
    );
}

/// Assert `project` yields a diagnostic with `rule`.
fn expect_rule(project: &IrProject, rule: &str) {
    let got = rules(project);
    assert!(
        got.iter().any(|r| r == rule),
        "expected {rule}, got {got:?}"
    );
}

fn ret(value: IrValue) -> IrOp {
    IrOp::Return {
        value: Some(value),
        loc: IrSourceLoc::default(),
    }
}

fn ret_none() -> IrOp {
    IrOp::Return {
        value: None,
        loc: IrSourceLoc::default(),
    }
}

fn const_of(ty: &str, v: &str) -> IrValue {
    IrValue::Const {
        type_: ty.to_string(),
        value: v.to_string(),
    }
}

fn binary(op: &str, left: IrValue, right: IrValue, ty: &str) -> IrValue {
    IrValue::Binary {
        op: op.to_string(),
        left: Box::new(left),
        right: Box::new(right),
        type_: ty.to_string(),
        loc: IrSourceLoc::default(),
    }
}

fn unary(op: &str, operand: IrValue, ty: &str) -> IrValue {
    IrValue::Unary {
        op: op.to_string(),
        operand: Box::new(operand),
        type_: ty.to_string(),
        loc: IrSourceLoc::default(),
    }
}

fn bind(name: &str, ty: &str, value: Option<IrValue>, explicit: bool, mutable: bool) -> IrOp {
    IrOp::Bind {
        mutable,
        name: name.to_string(),
        type_: ty.to_string(),
        value,
        explicit_type: explicit,
        loc: IrSourceLoc::default(),
    }
}

/// A record whose fields carry the given `(name, type)` pairs.
fn record_typed(name: &str, fields: &[(&str, &str)]) -> IrType {
    IrType {
        kind: "type".to_string(),
        visibility: "export".to_string(),
        name: name.to_string(),
        fields: fields
            .iter()
            .map(|(n, t)| IrField {
                visibility: None,
                name: (*n).to_string(),
                type_: (*t).to_string(),
                loc: IrSourceLoc::default(),
            })
            .collect(),
        includes: vec![],
        variants: vec![],
        members: vec![],
        loc: IrSourceLoc::default(),
        file: String::new(),
    }
}

fn enum_type(name: &str, members: &[&str]) -> IrType {
    IrType {
        kind: "enum".to_string(),
        visibility: "export".to_string(),
        name: name.to_string(),
        fields: vec![],
        includes: vec![],
        variants: vec![],
        members: members
            .iter()
            .map(|m| crate::ir::IrEnumMember {
                name: (*m).to_string(),
            })
            .collect(),
        loc: IrSourceLoc::default(),
        file: String::new(),
    }
}

fn binding(
    name: &str,
    ty: &str,
    value: Option<IrValue>,
    mutable: bool,
    explicit: bool,
) -> IrBinding {
    IrBinding {
        name: name.to_string(),
        visibility: "export".to_string(),
        mutable,
        type_: ty.to_string(),
        value,
        loc: IrSourceLoc::default(),
        file: "src/main.mfb".to_string(),
        explicit_type: explicit,
    }
}

fn sub(name: &str, params: Vec<IrParam>, body: Vec<IrOp>) -> IrFunction {
    let mut f = func_returns(name, "Nothing", params, body);
    f.kind = "sub".to_string();
    f
}

fn func(name: &str, params: Vec<IrParam>, body: Vec<IrOp>) -> IrFunction {
    func_returns(name, "Integer", params, body)
}

fn func_returns(name: &str, returns: &str, params: Vec<IrParam>, body: Vec<IrOp>) -> IrFunction {
    IrFunction {
        name: name.to_string(),
        visibility: "export".to_string(),
        kind: "func".to_string(),
        isolated: false,
        params,
        returns: returns.to_string(),
        body,
        file: "src/main.mfb".to_string(),
        resource_owners: HashMap::new(),
        loc: IrSourceLoc::default(),
    }
}

fn param(name: &str, type_: &str, default: Option<IrValue>) -> IrParam {
    IrParam {
        name: name.to_string(),
        type_: type_.to_string(),
        default,
        loc: IrSourceLoc::default(),
    }
}

fn record(name: &str, fields: &[&str]) -> IrType {
    IrType {
        kind: "type".to_string(),
        visibility: "export".to_string(),
        name: name.to_string(),
        fields: fields
            .iter()
            .map(|f| IrField {
                visibility: None,
                name: (*f).to_string(),
                type_: "Integer".to_string(),
                loc: IrSourceLoc::default(),
            })
            .collect(),
        includes: vec![],
        variants: vec![],
        members: vec![],
        loc: IrSourceLoc::default(),
        file: String::new(),
    }
}

fn int_const(v: &str) -> IrValue {
    IrValue::Const {
        type_: "Integer".to_string(),
        value: v.to_string(),
    }
}

// --- member access ---------------------------------------------------------

#[test]
fn accepts_member_access_on_known_record_field() {
    let body = vec![IrOp::Return {
        value: Some(IrValue::MemberAccess {
            target: Box::new(IrValue::Local("p".to_string())),
            member: "x".to_string(),
            type_: "Unknown".to_string(),
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
            type_: "Unknown".to_string(),
        }),
        loc: IrSourceLoc::default(),
    }];
    let f = func("run", vec![], body);
    let err = check(&project(vec![f], vec![])).expect_err("member on Integer must be rejected");
    assert!(err.contains("TYPE_FIELD_ACCESS_REQUIRES_RECORD"), "{err}");
}

#[test]
fn rejects_member_access_missing_field_on_record() {
    let body = vec![IrOp::Return {
        value: Some(IrValue::MemberAccess {
            target: Box::new(IrValue::Local("p".to_string())),
            member: "z".to_string(),
            type_: "Unknown".to_string(),
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
            type_: "Unknown".to_string(),
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
            type_: "Unknown".to_string(),
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
            type_: "Unknown".to_string(),
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
            type_: "Unknown".to_string(),
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
            type_: "Point".to_string(),
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
                type_: "Integer".to_string(),
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
                type_: "FUNC() AS Integer".to_string(),
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
                type_: "Integer".to_string(),
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
                type_: "FUNC() AS Integer".to_string(),
                captures: vec![int_const("7")],
            }),
            loc: IrSourceLoc::default(),
        }],
    );
    check(&project(vec![closure_body, maker], vec![])).expect("in-range capture is valid");
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
            type_: "Integer".to_string(),
            by_ref: false,
        })],
    );
    let closure = |captures: Vec<IrValue>| IrValue::Closure {
        name: "body".to_string(),
        type_: "FUNC() AS Integer".to_string(),
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
            type_: "Integer".to_string(),
            by_ref: false,
        })],
    );
    let closure = |captures: Vec<IrValue>| IrValue::Closure {
        name: "body".to_string(),
        type_: "FUNC() AS Integer".to_string(),
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

// --- union wrap ------------------------------------------------------------

fn union(name: &str, variants: &[&str]) -> IrType {
    IrType {
        kind: "union".to_string(),
        visibility: "export".to_string(),
        name: name.to_string(),
        fields: vec![],
        includes: vec![],
        variants: variants
            .iter()
            .map(|v| IrVariant {
                name: (*v).to_string(),
                fields: vec![],
                loc: IrSourceLoc::default(),
            })
            .collect(),
        members: vec![],
        loc: IrSourceLoc::default(),
        file: String::new(),
    }
}

#[test]
fn rejects_union_wrap_of_foreign_variant() {
    let body = vec![IrOp::Return {
        value: Some(IrValue::UnionWrap {
            union_type: "Shape".to_string(),
            member_type: "Ghost".to_string(),
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
    let body = vec![IrOp::Return {
        value: Some(IrValue::UnionWrap {
            union_type: "Shape".to_string(),
            member_type: "Circle".to_string(),
            value: Box::new(int_const("0")),
        }),
        loc: IrSourceLoc::default(),
    }];
    let f = func_returns("run", "Shape", vec![], body);
    check(&project(
        vec![f],
        vec![union("Shape", &["Circle", "Square"])],
    ))
    .expect("real variant is valid");
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
            type_: "Integer".to_string(),
            value: Some(int_const("1")),
            loc: IrSourceLoc::default(),
            explicit_type: false,
        },
        IrOp::Return {
            value: Some(IrValue::Binary {
                op: "+".to_string(),
                left: Box::new(IrValue::Local("n".to_string())),
                right: Box::new(int_const("2")),
                loc: IrSourceLoc::default(),
                type_: "Unknown".to_string(),
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
        "-",
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
        "AND",
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
    let body = vec![ret(binary("&", int_const("1"), int_const("2"), "String"))];
    expect_rule(
        &project(vec![func_returns("run", "String", vec![], body)], vec![]),
        "TYPE_BINARY_OPERATOR_MISMATCH",
    );
}

#[test]
fn rejects_relational_on_boolean() {
    let body = vec![ret(binary(
        "<",
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
        "<",
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
        "&",
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
        "AND",
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
        "=",
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
        "=",
        IrValue::ListLiteral {
            type_: "List OF Integer".to_string(),
            values: vec![],
        },
        IrValue::ListLiteral {
            type_: "List OF Integer".to_string(),
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
    let body = vec![ret(binary("=", int_const("1"), int_const("2"), "Boolean"))];
    accept(&project(
        vec![func_returns("run", "Boolean", vec![], body)],
        vec![],
    ));
}

// --- unary operators -------------------------------------------------------

#[test]
fn rejects_not_on_numeric() {
    let body = vec![ret(unary("NOT", int_const("1"), "Boolean"))];
    expect_rule(
        &project(vec![func_returns("run", "Boolean", vec![], body)], vec![]),
        "TYPE_UNARY_OPERATOR_MISMATCH",
    );
}

#[test]
fn rejects_negate_on_string() {
    let body = vec![ret(unary("-", const_of("String", "a"), "Integer"))];
    expect_rule(
        &project(vec![func("run", vec![], body)], vec![]),
        "TYPE_UNARY_OPERATOR_MISMATCH",
    );
}

#[test]
fn rejects_unknown_unary_operator() {
    let body = vec![ret(unary("~", int_const("1"), "Integer"))];
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
            Some(unary("NOT", const_of("Boolean", "true"), "Boolean")),
            false,
            false,
        ),
        ret(unary("-", int_const("1"), "Integer")),
    ];
    accept(&project(vec![func("run", vec![], body)], vec![]));
}

// --- literal ranges --------------------------------------------------------

#[test]
fn rejects_byte_overflow() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind("b", "Byte", Some(int_const("300")), true, false)],
    );
    expect_rule(&project(vec![f], vec![]), "TYPE_BYTE_LITERAL_OVERFLOW");
}

#[test]
fn rejects_byte_underflow() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind(
            "b",
            "Byte",
            Some(unary("-", int_const("1"), "Integer")),
            true,
            false,
        )],
    );
    expect_rule(&project(vec![f], vec![]), "TYPE_BYTE_LITERAL_UNDERFLOW");
}

#[test]
fn rejects_integer_overflow() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind(
            "n",
            "Integer",
            Some(int_const("99999999999999999999")),
            true,
            false,
        )],
    );
    expect_rule(&project(vec![f], vec![]), "TYPE_INTEGER_LITERAL_OVERFLOW");
}

#[test]
fn rejects_negated_integer_overflow() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind(
            "n",
            "Integer",
            Some(unary("-", int_const("99999999999999999999"), "Integer")),
            true,
            false,
        )],
    );
    expect_rule(&project(vec![f], vec![]), "TYPE_INTEGER_LITERAL_OVERFLOW");
}

#[test]
fn rejects_float_overflow() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind(
            "f",
            "Float",
            Some(const_of("Float", "1e400")),
            true,
            false,
        )],
    );
    expect_rule(&project(vec![f], vec![]), "TYPE_FLOAT_LITERAL_OVERFLOW");
}

#[test]
fn rejects_float_underflow() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind(
            "f",
            "Float",
            Some(unary("-", const_of("Float", "1e400"), "Float")),
            true,
            false,
        )],
    );
    expect_rule(&project(vec![f], vec![]), "TYPE_FLOAT_LITERAL_UNDERFLOW");
}

#[test]
fn rejects_fixed_overflow() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind(
            "x",
            "Fixed",
            Some(const_of("Fixed", "3000000000")),
            true,
            false,
        )],
    );
    expect_rule(&project(vec![f], vec![]), "TYPE_FIXED_LITERAL_OVERFLOW");
}

#[test]
fn rejects_fixed_underflow() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind(
            "x",
            "Fixed",
            Some(unary("-", const_of("Fixed", "3000000000"), "Fixed")),
            true,
            false,
        )],
    );
    expect_rule(&project(vec![f], vec![]), "TYPE_FIXED_LITERAL_UNDERFLOW");
}

#[test]
fn accepts_byte_in_range() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind("b", "Byte", Some(int_const("200")), true, false)],
    );
    accept(&project(vec![f], vec![]));
}

// --- binding shape rules ---------------------------------------------------

#[test]
fn rejects_let_requires_value() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind("x", "Integer", None, true, false)],
    );
    expect_rule(&project(vec![f], vec![]), "TYPE_LET_REQUIRES_VALUE");
}

#[test]
fn rejects_binding_requires_type_or_value() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind("x", "Integer", None, false, false)],
    );
    expect_rule(
        &project(vec![f], vec![]),
        "TYPE_BINDING_REQUIRES_TYPE_OR_VALUE",
    );
}

#[test]
fn rejects_mut_requires_defaultable() {
    // A MUT binding with no value whose type is not defaultable (a union).
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind("x", "Shape", None, true, true)],
    );
    expect_rule(
        &project(vec![f], vec![union("Shape", &["Circle", "Square"])]),
        "TYPE_MUT_REQUIRES_DEFAULTABLE_TYPE",
    );
}

#[test]
fn accepts_mut_defaultable_without_value() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind("x", "Integer", None, true, true)],
    );
    accept(&project(vec![f], vec![]));
}

#[test]
fn rejects_binding_type_mismatch() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind(
            "x",
            "Integer",
            Some(const_of("String", "hi")),
            true,
            false,
        )],
    );
    expect_rule(&project(vec![f], vec![]), "TYPE_BINDING_MISMATCH");
}

// --- assignment ------------------------------------------------------------

#[test]
fn rejects_assign_to_immutable() {
    let body = vec![
        bind("x", "Integer", Some(int_const("1")), false, false),
        IrOp::Assign {
            name: "x".to_string(),
            value: int_const("2"),
            loc: IrSourceLoc::default(),
        },
    ];
    expect_rule(
        &project(vec![func_returns("run", "Nothing", vec![], body)], vec![]),
        "TYPE_ASSIGN_REQUIRES_MUT",
    );
}

#[test]
fn rejects_assignment_type_mismatch() {
    let body = vec![
        bind("x", "Integer", Some(int_const("1")), false, true),
        IrOp::Assign {
            name: "x".to_string(),
            value: const_of("String", "no"),
            loc: IrSourceLoc::default(),
        },
    ];
    expect_rule(
        &project(vec![func_returns("run", "Nothing", vec![], body)], vec![]),
        "TYPE_ASSIGNMENT_MISMATCH",
    );
}

#[test]
fn accepts_valid_assignment() {
    let body = vec![
        bind("x", "Integer", Some(int_const("1")), false, true),
        IrOp::Assign {
            name: "x".to_string(),
            value: int_const("2"),
            loc: IrSourceLoc::default(),
        },
    ];
    accept(&project(
        vec![func_returns("run", "Nothing", vec![], body)],
        vec![],
    ));
}

// --- global bindings -------------------------------------------------------

#[test]
fn rejects_global_binding_requires_type_or_value() {
    let mut p = project(vec![func_returns("run", "Nothing", vec![], vec![])], vec![]);
    p.bindings = vec![binding("g", "Integer", None, false, false)];
    expect_rule(&p, "TYPE_BINDING_REQUIRES_TYPE_OR_VALUE");
}

#[test]
fn rejects_global_let_requires_value() {
    let mut p = project(vec![func_returns("run", "Nothing", vec![], vec![])], vec![]);
    p.bindings = vec![binding("g", "Integer", None, false, true)];
    expect_rule(&p, "TYPE_LET_REQUIRES_VALUE");
}

#[test]
fn rejects_global_mut_requires_defaultable() {
    let mut p = project(
        vec![func_returns("run", "Nothing", vec![], vec![])],
        vec![union("Shape", &["A", "B"])],
    );
    p.bindings = vec![binding("g", "Shape", None, true, true)];
    expect_rule(&p, "TYPE_MUT_REQUIRES_DEFAULTABLE_TYPE");
}

#[test]
fn accepts_global_with_value() {
    let mut p = project(vec![func_returns("run", "Nothing", vec![], vec![])], vec![]);
    p.bindings = vec![binding("g", "Integer", Some(int_const("5")), false, true)];
    accept(&p);
}

#[test]
fn rejects_global_binding_type_mismatch() {
    let mut p = project(vec![func_returns("run", "Nothing", vec![], vec![])], vec![]);
    p.bindings = vec![binding(
        "g",
        "Integer",
        Some(const_of("String", "x")),
        false,
        true,
    )];
    expect_rule(&p, "TYPE_BINDING_MISMATCH");
}

#[test]
fn rejects_assign_global_immutable() {
    let mut p = project(
        vec![func_returns(
            "run",
            "Nothing",
            vec![],
            vec![IrOp::AssignGlobal {
                name: "g".to_string(),
                value: int_const("2"),
                loc: IrSourceLoc::default(),
            }],
        )],
        vec![],
    );
    p.bindings = vec![binding("g", "Integer", Some(int_const("1")), false, true)];
    expect_rule(&p, "TYPE_ASSIGN_REQUIRES_MUT");
}

#[test]
fn rejects_assign_global_type_mismatch() {
    let mut p = project(
        vec![func_returns(
            "run",
            "Nothing",
            vec![],
            vec![IrOp::AssignGlobal {
                name: "g".to_string(),
                value: const_of("String", "z"),
                loc: IrSourceLoc::default(),
            }],
        )],
        vec![],
    );
    p.bindings = vec![binding("g", "Integer", Some(int_const("1")), true, true)];
    expect_rule(&p, "TYPE_ASSIGNMENT_MISMATCH");
}

// --- return / sub rules ----------------------------------------------------

#[test]
fn rejects_return_mismatch() {
    let body = vec![ret(const_of("String", "no"))];
    expect_rule(
        &project(vec![func_returns("run", "Integer", vec![], body)], vec![]),
        "TYPE_RETURN_MISMATCH",
    );
}

#[test]
fn rejects_sub_return_value() {
    let s = sub("doit", vec![], vec![ret(int_const("1"))]);
    expect_rule(&project(vec![s], vec![]), "SUB_RETURN_FORBIDDEN");
}

#[test]
fn rejects_sub_call_in_value_position() {
    let s = sub("doit", vec![], vec![]);
    let body = vec![bind(
        "x",
        "Integer",
        Some(IrValue::Call {
            target: "doit".to_string(),
            args: vec![],
            type_: "Nothing".to_string(),
            loc: IrSourceLoc::default(),
        }),
        false,
        false,
    )];
    expect_rule(
        &project(
            vec![s, func_returns("run", "Nothing", vec![], body)],
            vec![],
        ),
        "TYPE_SUB_HAS_NO_VALUE",
    );
}

#[test]
fn accepts_sub_call_in_statement_position() {
    let s = sub("doit", vec![], vec![]);
    let body = vec![IrOp::Eval {
        value: IrValue::Call {
            target: "doit".to_string(),
            args: vec![],
            type_: "Nothing".to_string(),
            loc: IrSourceLoc::default(),
        },
        loc: IrSourceLoc::default(),
    }];
    accept(&project(
        vec![s, func_returns("run", "Nothing", vec![], body)],
        vec![],
    ));
}

// --- exit program ----------------------------------------------------------

#[test]
fn rejects_exit_program_non_integer() {
    let body = vec![IrOp::ExitProgram {
        code: const_of("String", "x"),
        loc: IrSourceLoc::default(),
    }];
    expect_rule(
        &project(vec![func_returns("run", "Nothing", vec![], body)], vec![]),
        "TYPE_EXIT_PROGRAM_REQUIRES_INTEGER",
    );
}

#[test]
fn rejects_exit_program_out_of_range() {
    let body = vec![IrOp::ExitProgram {
        code: int_const("300"),
        loc: IrSourceLoc::default(),
    }];
    expect_rule(
        &project(vec![func_returns("run", "Nothing", vec![], body)], vec![]),
        "EXIT_PROGRAM_CODE_OUT_OF_RANGE",
    );
}

#[test]
fn accepts_exit_program_in_range() {
    let body = vec![IrOp::ExitProgram {
        code: int_const("0"),
        loc: IrSourceLoc::default(),
    }];
    accept(&project(
        vec![func_returns("run", "Nothing", vec![], body)],
        vec![],
    ));
}

// --- fail / propagate ------------------------------------------------------

#[test]
fn rejects_propagate_outside_trap() {
    let body = vec![IrOp::Fail {
        error: IrValue::Local("$error".to_string()),
        loc: IrSourceLoc::default(),
    }];
    expect_rule(
        &project(vec![func_returns("run", "Nothing", vec![], body)], vec![]),
        "TYPE_PROPAGATE_REQUIRES_TRAP",
    );
}

#[test]
fn rejects_fail_non_error() {
    let body = vec![IrOp::Fail {
        error: int_const("1"),
        loc: IrSourceLoc::default(),
    }];
    expect_rule(
        &project(vec![func_returns("run", "Nothing", vec![], body)], vec![]),
        "TYPE_FAIL_REQUIRES_ERROR",
    );
}

// --- exit/continue loop ----------------------------------------------------

#[test]
fn rejects_exit_without_loop() {
    let body = vec![IrOp::ExitLoop {
        kind: crate::ast::LoopKind::For,
        loc: IrSourceLoc::default(),
    }];
    expect_rule(
        &project(vec![func_returns("run", "Nothing", vec![], body)], vec![]),
        "EXIT_NO_MATCHING_LOOP",
    );
}

#[test]
fn rejects_continue_without_loop() {
    let body = vec![IrOp::ContinueLoop {
        kind: crate::ast::LoopKind::While,
        loc: IrSourceLoc::default(),
    }];
    expect_rule(
        &project(vec![func_returns("run", "Nothing", vec![], body)], vec![]),
        "CONTINUE_NO_MATCHING_LOOP",
    );
}

#[test]
fn accepts_exit_inside_matching_loop() {
    let body = vec![IrOp::While {
        kind: crate::ast::LoopKind::While,
        condition: const_of("Boolean", "true"),
        body: vec![IrOp::ExitLoop {
            kind: crate::ast::LoopKind::While,
            loc: IrSourceLoc::default(),
        }],
        loc: IrSourceLoc::default(),
    }];
    accept(&project(
        vec![func_returns("run", "Nothing", vec![], body)],
        vec![],
    ));
}

#[test]
fn rejects_unreachable_after_exit() {
    let body = vec![IrOp::While {
        kind: crate::ast::LoopKind::While,
        condition: const_of("Boolean", "true"),
        body: vec![
            IrOp::ExitLoop {
                kind: crate::ast::LoopKind::While,
                loc: IrSourceLoc::default(),
            },
            IrOp::Eval {
                value: int_const("1"),
                loc: IrSourceLoc::default(),
            },
        ],
        loc: IrSourceLoc::default(),
    }];
    expect_rule(
        &project(vec![func_returns("run", "Nothing", vec![], body)], vec![]),
        "UNREACHABLE_AFTER_EXIT",
    );
}

// --- if / while / do-until conditions --------------------------------------

#[test]
fn rejects_if_condition_non_boolean() {
    let body = vec![IrOp::If {
        condition: int_const("1"),
        then_body: vec![],
        else_body: vec![],
        loc: IrSourceLoc::default(),
    }];
    expect_rule(
        &project(vec![func_returns("run", "Nothing", vec![], body)], vec![]),
        "TYPE_CONDITION_REQUIRES_BOOLEAN",
    );
}

#[test]
fn rejects_while_condition_non_boolean() {
    let body = vec![IrOp::While {
        kind: crate::ast::LoopKind::While,
        condition: int_const("1"),
        body: vec![],
        loc: IrSourceLoc::default(),
    }];
    expect_rule(
        &project(vec![func_returns("run", "Nothing", vec![], body)], vec![]),
        "TYPE_CONDITION_REQUIRES_BOOLEAN",
    );
}

#[test]
fn rejects_do_until_condition_non_boolean() {
    let body = vec![IrOp::DoUntil {
        body: vec![],
        condition: int_const("1"),
        loc: IrSourceLoc::default(),
    }];
    expect_rule(
        &project(vec![func_returns("run", "Nothing", vec![], body)], vec![]),
        "TYPE_CONDITION_REQUIRES_BOOLEAN",
    );
}

#[test]
fn accepts_do_until_valid() {
    let body = vec![IrOp::DoUntil {
        body: vec![IrOp::ContinueLoop {
            kind: crate::ast::LoopKind::Do,
            loc: IrSourceLoc::default(),
        }],
        condition: const_of("Boolean", "true"),
        loc: IrSourceLoc::default(),
    }];
    accept(&project(
        vec![func_returns("run", "Nothing", vec![], body)],
        vec![],
    ));
}

// --- for loops -------------------------------------------------------------

#[test]
fn rejects_for_non_numeric_bound() {
    let body = vec![IrOp::For {
        name: "i".to_string(),
        type_: "Integer".to_string(),
        start: const_of("String", "a"),
        end: int_const("10"),
        step: int_const("1"),
        body: vec![],
        loc: IrSourceLoc::default(),
    }];
    expect_rule(
        &project(vec![func_returns("run", "Nothing", vec![], body)], vec![]),
        "TYPE_FOR_REQUIRES_NUMERIC",
    );
}

#[test]
fn rejects_for_step_zero() {
    let body = vec![IrOp::For {
        name: "i".to_string(),
        type_: "Integer".to_string(),
        start: int_const("0"),
        end: int_const("10"),
        step: int_const("0"),
        body: vec![],
        loc: IrSourceLoc::default(),
    }];
    expect_rule(
        &project(vec![func_returns("run", "Nothing", vec![], body)], vec![]),
        "TYPE_FOR_STEP_ZERO",
    );
}

#[test]
fn accepts_valid_for() {
    let body = vec![IrOp::For {
        name: "i".to_string(),
        type_: "Integer".to_string(),
        start: int_const("0"),
        end: int_const("10"),
        step: int_const("1"),
        body: vec![IrOp::ExitLoop {
            kind: crate::ast::LoopKind::For,
            loc: IrSourceLoc::default(),
        }],
        loc: IrSourceLoc::default(),
    }];
    accept(&project(
        vec![func_returns("run", "Nothing", vec![], body)],
        vec![],
    ));
}

#[test]
fn for_step_resolved_through_temp() {
    // A `$for` temp binds the step; the checker resolves it.
    let body = vec![
        bind("$for0", "Integer", Some(int_const("0")), false, false),
        IrOp::For {
            name: "i".to_string(),
            type_: "Integer".to_string(),
            start: int_const("0"),
            end: int_const("10"),
            step: IrValue::Local("$for0".to_string()),
            body: vec![],
            loc: IrSourceLoc::default(),
        },
    ];
    expect_rule(
        &project(vec![func_returns("run", "Nothing", vec![], body)], vec![]),
        "TYPE_FOR_STEP_ZERO",
    );
}

// --- for each --------------------------------------------------------------

#[test]
fn rejects_for_each_non_collection() {
    let body = vec![
        bind("x", "Integer", Some(int_const("1")), false, false),
        IrOp::ForEach {
            name: "e".to_string(),
            type_: "Integer".to_string(),
            iterable: IrValue::Local("x".to_string()),
            body: vec![],
            loc: IrSourceLoc::default(),
        },
    ];
    expect_rule(
        &project(vec![func_returns("run", "Nothing", vec![], body)], vec![]),
        "TYPE_FOR_EACH_REQUIRES_COLLECTION",
    );
}

#[test]
fn accepts_for_each_list() {
    let body = vec![
        bind(
            "xs",
            "List OF Integer",
            Some(IrValue::ListLiteral {
                type_: "List OF Integer".to_string(),
                values: vec![int_const("1")],
            }),
            false,
            false,
        ),
        IrOp::ForEach {
            name: "e".to_string(),
            type_: "Integer".to_string(),
            iterable: IrValue::Local("xs".to_string()),
            body: vec![IrOp::ContinueLoop {
                kind: crate::ast::LoopKind::For,
                loc: IrSourceLoc::default(),
            }],
            loc: IrSourceLoc::default(),
        },
    ];
    accept(&project(
        vec![func_returns("run", "Nothing", vec![], body)],
        vec![],
    ));
}

// --- match -----------------------------------------------------------------

fn union_variant_case(name: &str, body: Vec<IrOp>) -> IrMatchCase {
    IrMatchCase {
        pattern: IrMatchPattern::Value(IrValue::Local(name.to_string())),
        guard: None,
        body,
        loc: IrSourceLoc::default(),
    }
}

#[test]
fn rejects_non_exhaustive_union_match() {
    let m = IrOp::Match {
        value: IrValue::Local("s".to_string()),
        cases: vec![union_variant_case("Circle", vec![ret_none()])],
        loc: IrSourceLoc::default(),
    };
    let f = func_returns("run", "Nothing", vec![param("s", "Shape", None)], vec![m]);
    expect_rule(
        &project(vec![f], vec![union("Shape", &["Circle", "Square"])]),
        "TYPE_MATCH_NOT_EXHAUSTIVE",
    );
}

#[test]
fn accepts_exhaustive_union_match() {
    let m = IrOp::Match {
        value: IrValue::Local("s".to_string()),
        cases: vec![
            union_variant_case("Circle", vec![ret_none()]),
            union_variant_case("Square", vec![ret_none()]),
        ],
        loc: IrSourceLoc::default(),
    };
    let f = func_returns("run", "Nothing", vec![param("s", "Shape", None)], vec![m]);
    accept(&project(
        vec![f],
        vec![union("Shape", &["Circle", "Square"])],
    ));
}

#[test]
fn accepts_union_match_with_else() {
    let m = IrOp::Match {
        value: IrValue::Local("s".to_string()),
        cases: vec![
            union_variant_case("Circle", vec![ret_none()]),
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

#[test]
fn rejects_enum_match_not_exhaustive() {
    let m = IrOp::Match {
        value: IrValue::Local("c".to_string()),
        cases: vec![IrMatchCase {
            pattern: IrMatchPattern::Value(IrValue::MemberAccess {
                target: Box::new(IrValue::Local("Color".to_string())),
                member: "Red".to_string(),
                type_: "Color".to_string(),
            }),
            guard: None,
            body: vec![ret_none()],
            loc: IrSourceLoc::default(),
        }],
        loc: IrSourceLoc::default(),
    };
    let f = func_returns("run", "Nothing", vec![param("c", "Color", None)], vec![m]);
    expect_rule(
        &project(vec![f], vec![enum_type("Color", &["Red", "Green"])]),
        "TYPE_MATCH_NOT_EXHAUSTIVE",
    );
}

#[test]
fn rejects_match_open_type_without_else() {
    // A MATCH on Integer (an open type) with no CASE ELSE.
    let m = IrOp::Match {
        value: int_const("1"),
        cases: vec![IrMatchCase {
            pattern: IrMatchPattern::Value(int_const("1")),
            guard: None,
            body: vec![ret_none()],
            loc: IrSourceLoc::default(),
        }],
        loc: IrSourceLoc::default(),
    };
    expect_rule(
        &project(
            vec![func_returns("run", "Nothing", vec![], vec![m])],
            vec![],
        ),
        "TYPE_MATCH_NOT_EXHAUSTIVE",
    );
}

#[test]
fn rejects_match_pattern_not_a_variant() {
    let m = IrOp::Match {
        value: IrValue::Local("s".to_string()),
        cases: vec![
            union_variant_case("Ghost", vec![ret_none()]),
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
    // Ghost is a declared record but not a variant of Shape.
    expect_rule(
        &project(
            vec![f],
            vec![
                union("Shape", &["Circle", "Square"]),
                record("Ghost", &["x"]),
            ],
        ),
        "TYPE_MATCH_PATTERN_MISMATCH",
    );
}

#[test]
fn rejects_result_case_not_matchable() {
    let m = IrOp::Match {
        value: IrValue::Local("s".to_string()),
        cases: vec![
            IrMatchCase {
                pattern: IrMatchPattern::Value(IrValue::Local("Ok".to_string())),
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
    let f = func_returns("run", "Nothing", vec![param("s", "Shape", None)], vec![m]);
    expect_rule(
        &project(vec![f], vec![union("Shape", &["Circle", "Square"])]),
        "TYPE_RESULT_NOT_MATCHABLE",
    );
}

#[test]
fn rejects_match_pattern_requires_union() {
    // A type-named CASE against an enum scrutinee.
    let m = IrOp::Match {
        value: IrValue::Local("c".to_string()),
        cases: vec![
            union_variant_case("Ghost", vec![ret_none()]),
            IrMatchCase {
                pattern: IrMatchPattern::Else,
                guard: None,
                body: vec![ret_none()],
                loc: IrSourceLoc::default(),
            },
        ],
        loc: IrSourceLoc::default(),
    };
    let f = func_returns("run", "Nothing", vec![param("c", "Color", None)], vec![m]);
    expect_rule(
        &project(
            vec![f],
            vec![
                enum_type("Color", &["Red", "Green"]),
                record("Ghost", &["x"]),
            ],
        ),
        "TYPE_MATCH_PATTERN_MISMATCH",
    );
}

#[test]
fn match_guard_and_oneof() {
    // OneOf pattern with a guard; exercises the guard-bind registration path.
    let m = IrOp::Match {
        value: IrValue::Local("n".to_string()),
        cases: vec![
            IrMatchCase {
                pattern: IrMatchPattern::OneOf(vec![int_const("1"), int_const("2")]),
                guard: Some(const_of("Boolean", "true")),
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
    let f = func_returns("run", "Nothing", vec![param("n", "Integer", None)], vec![m]);
    accept(&project(vec![f], vec![]));
}

#[test]
fn rejects_when_guard_non_boolean() {
    let m = IrOp::Match {
        value: IrValue::Local("n".to_string()),
        cases: vec![
            IrMatchCase {
                pattern: IrMatchPattern::Value(int_const("1")),
                guard: Some(int_const("5")),
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
    let f = func_returns("run", "Nothing", vec![param("n", "Integer", None)], vec![m]);
    expect_rule(&project(vec![f], vec![]), "TYPE_CONDITION_REQUIRES_BOOLEAN");
}

// --- constructors ----------------------------------------------------------

#[test]
fn rejects_constructor_requires_record_for_union() {
    let body = vec![ret(IrValue::Constructor {
        type_: "Shape".to_string(),
        args: vec![],
    })];
    let f = func_returns("run", "Shape", vec![], body);
    expect_rule(
        &project(vec![f], vec![union("Shape", &["A", "B"])]),
        "TYPE_CONSTRUCTOR_REQUIRES_RECORD",
    );
}

#[test]
fn rejects_constructor_requires_record_for_enum() {
    let body = vec![ret(IrValue::Constructor {
        type_: "Color".to_string(),
        args: vec![],
    })];
    let f = func_returns("run", "Color", vec![], body);
    expect_rule(
        &project(vec![f], vec![enum_type("Color", &["Red"])]),
        "TYPE_CONSTRUCTOR_REQUIRES_RECORD",
    );
}

#[test]
fn rejects_constructor_arity() {
    let body = vec![ret(IrValue::Constructor {
        type_: "Point".to_string(),
        args: vec![int_const("1")],
    })];
    let f = func_returns("run", "Point", vec![], body);
    expect_rule(
        &project(vec![f], vec![record("Point", &["x", "y"])]),
        "TYPE_CONSTRUCTOR_ARITY_MISMATCH",
    );
}

#[test]
fn rejects_constructor_argument_mismatch() {
    let body = vec![ret(IrValue::Constructor {
        type_: "Point".to_string(),
        args: vec![const_of("String", "a"), int_const("2")],
    })];
    let f = func_returns("run", "Point", vec![], body);
    expect_rule(
        &project(vec![f], vec![record("Point", &["x", "y"])]),
        "TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH",
    );
}

#[test]
fn accepts_valid_constructor() {
    let body = vec![ret(IrValue::Constructor {
        type_: "Point".to_string(),
        args: vec![int_const("1"), int_const("2")],
    })];
    let f = func_returns("run", "Point", vec![], body);
    accept(&project(vec![f], vec![record("Point", &["x", "y"])]));
}

#[test]
fn rejects_construct_result_implicit() {
    let body = vec![ret(IrValue::Constructor {
        type_: "Ok".to_string(),
        args: vec![int_const("1")],
    })];
    let f = func_returns("run", "Integer", vec![], body);
    expect_rule(&project(vec![f], vec![]), "TYPE_RESULT_IS_IMPLICIT");
}

// --- with update -----------------------------------------------------------

#[test]
fn rejects_read_only_record_update_error() {
    let body = vec![
        bind("e", "Error", None, false, false),
        ret(IrValue::WithUpdate {
            type_: "Error".to_string(),
            target: Box::new(IrValue::Local("e".to_string())),
            updates: vec![],
        }),
    ];
    let f = func_returns("run", "Error", vec![param("e", "Error", None)], body);
    expect_rule(&project(vec![f], vec![]), "TYPE_READ_ONLY_RECORD_UPDATE");
}

#[test]
fn rejects_duplicate_with_field() {
    let body = vec![ret(IrValue::WithUpdate {
        type_: "Point".to_string(),
        target: Box::new(IrValue::Local("p".to_string())),
        updates: vec![
            crate::ir::IrRecordUpdate {
                field: "x".to_string(),
                value: int_const("1"),
            },
            crate::ir::IrRecordUpdate {
                field: "x".to_string(),
                value: int_const("2"),
            },
        ],
    })];
    let f = func_returns("run", "Point", vec![param("p", "Point", None)], body);
    expect_rule(
        &project(vec![f], vec![record("Point", &["x", "y"])]),
        "TYPE_DUPLICATE_FIELD",
    );
}

#[test]
fn rejects_with_update_field_mismatch() {
    let body = vec![ret(IrValue::WithUpdate {
        type_: "Point".to_string(),
        target: Box::new(IrValue::Local("p".to_string())),
        updates: vec![crate::ir::IrRecordUpdate {
            field: "x".to_string(),
            value: const_of("String", "no"),
        }],
    })];
    let f = func_returns("run", "Point", vec![param("p", "Point", None)], body);
    expect_rule(
        &project(vec![f], vec![record("Point", &["x", "y"])]),
        "TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH",
    );
}

#[test]
fn accepts_valid_with_update() {
    let body = vec![ret(IrValue::WithUpdate {
        type_: "Point".to_string(),
        target: Box::new(IrValue::Local("p".to_string())),
        updates: vec![crate::ir::IrRecordUpdate {
            field: "x".to_string(),
            value: int_const("9"),
        }],
    })];
    let f = func_returns("run", "Point", vec![param("p", "Point", None)], body);
    accept(&project(vec![f], vec![record("Point", &["x", "y"])]));
}

// --- list / map literals ---------------------------------------------------

#[test]
fn rejects_list_element_mismatch() {
    let body = vec![ret(IrValue::ListLiteral {
        type_: "List OF Integer".to_string(),
        values: vec![const_of("String", "x")],
    })];
    let f = func_returns("run", "List OF Integer", vec![], body);
    expect_rule(&project(vec![f], vec![]), "TYPE_LIST_ELEMENT_MISMATCH");
}

#[test]
fn accepts_valid_list_literal() {
    let body = vec![ret(IrValue::ListLiteral {
        type_: "List OF Integer".to_string(),
        values: vec![int_const("1"), int_const("2")],
    })];
    let f = func_returns("run", "List OF Integer", vec![], body);
    accept(&project(vec![f], vec![]));
}

#[test]
fn rejects_map_key_mismatch() {
    let body = vec![ret(IrValue::MapLiteral {
        type_: "Map OF String TO Integer".to_string(),
        entries: vec![(int_const("1"), int_const("2"))],
    })];
    let f = func_returns("run", "Map OF String TO Integer", vec![], body);
    expect_rule(&project(vec![f], vec![]), "TYPE_MAP_KEY_MISMATCH");
}

#[test]
fn rejects_map_value_mismatch() {
    let body = vec![ret(IrValue::MapLiteral {
        type_: "Map OF String TO Integer".to_string(),
        entries: vec![(const_of("String", "k"), const_of("String", "v"))],
    })];
    let f = func_returns("run", "Map OF String TO Integer", vec![], body);
    expect_rule(&project(vec![f], vec![]), "TYPE_MAP_VALUE_MISMATCH");
}

#[test]
fn accepts_valid_map_literal() {
    let body = vec![ret(IrValue::MapLiteral {
        type_: "Map OF String TO Integer".to_string(),
        entries: vec![(const_of("String", "k"), int_const("1"))],
    })];
    let f = func_returns("run", "Map OF String TO Integer", vec![], body);
    accept(&project(vec![f], vec![]));
}

#[test]
fn rejects_map_key_not_comparable() {
    // A map keyed on List (not comparable).
    let f = func_returns(
        "run",
        "Nothing",
        vec![param("m", "Map OF List OF Integer TO Integer", None)],
        vec![],
    );
    expect_rule(&project(vec![f], vec![]), "TYPE_REQUIRES_COMPARABLE");
}

// --- member access chains --------------------------------------------------

#[test]
fn rejects_unknown_enum_member() {
    let body = vec![ret(IrValue::MemberAccess {
        target: Box::new(IrValue::Local("Color".to_string())),
        member: "Purple".to_string(),
        type_: "Color".to_string(),
    })];
    let f = func_returns("run", "Color", vec![], body);
    expect_rule(
        &project(vec![f], vec![enum_type("Color", &["Red", "Green"])]),
        "TYPE_UNKNOWN_ENUM_MEMBER",
    );
}

#[test]
fn accepts_enum_member() {
    let body = vec![ret(IrValue::MemberAccess {
        target: Box::new(IrValue::Local("Color".to_string())),
        member: "Red".to_string(),
        type_: "Color".to_string(),
    })];
    let f = func_returns("run", "Color", vec![], body);
    accept(&project(
        vec![f],
        vec![enum_type("Color", &["Red", "Green"])],
    ));
}

#[test]
fn accepts_error_member_access_chain() {
    // err.source.line resolves through the builtin Error/ErrorLoc field tables.
    let body = vec![ret(IrValue::MemberAccess {
        target: Box::new(IrValue::MemberAccess {
            target: Box::new(IrValue::Local("err".to_string())),
            member: "source".to_string(),
            type_: "Unknown".to_string(),
        }),
        member: "line".to_string(),
        type_: "Unknown".to_string(),
    })];
    let f = func_returns("run", "Integer", vec![param("err", "Error", None)], body);
    accept(&project(vec![f], vec![]));
}

// --- type declarations -----------------------------------------------------

#[test]
fn rejects_resource_field_in_record() {
    let mut ty = record_typed("Holder", &[("f", "File")]);
    ty.file = "src/main.mfb".to_string();
    let f = func_returns("run", "Nothing", vec![], vec![]);
    expect_rule(&project(vec![f], vec![ty]), "TYPE_RESOURCE_FIELD_FORBIDDEN");
}

#[test]
fn rejects_recursive_record() {
    let ty = record_typed("Node", &[("next", "Node")]);
    let f = func_returns("run", "Nothing", vec![], vec![]);
    expect_rule(
        &project(vec![f], vec![ty]),
        "TYPE_RECURSIVE_RECORD_REQUIRES_INDIRECTION",
    );
}

#[test]
fn accepts_recursive_record_through_list() {
    let ty = record_typed("Node", &[("kids", "List OF Node")]);
    let f = func_returns("run", "Nothing", vec![], vec![]);
    accept(&project(vec![f], vec![ty]));
}

#[test]
fn rejects_empty_enum() {
    let ty = enum_type("Empty", &[]);
    let f = func_returns("run", "Nothing", vec![], vec![]);
    expect_rule(&project(vec![f], vec![ty]), "TYPE_ENUM_REQUIRES_MEMBER");
}

#[test]
fn rejects_union_include_requires_union() {
    let mut u = union("Shape", &["Circle"]);
    u.includes = vec!["Point".to_string()];
    let f = func_returns("run", "Nothing", vec![], vec![]);
    expect_rule(
        &project(vec![f], vec![u, record("Point", &["x"])]),
        "TYPE_UNION_INCLUDE_REQUIRES_UNION",
    );
}

#[test]
fn rejects_union_member_requires_type() {
    // A union whose variant name is itself a union.
    let mut u = union("Shape", &["Inner"]);
    u.name = "Shape".to_string();
    let inner = union("Inner", &["A"]);
    let f = func_returns("run", "Nothing", vec![], vec![]);
    expect_rule(
        &project(vec![f], vec![u, inner]),
        "TYPE_UNION_MEMBER_REQUIRES_TYPE",
    );
}

#[test]
fn rejects_mixed_resource_union() {
    // A union with one resource variant (File) and one data variant.
    let u = union("Mixed", &["File", "Circle"]);
    let f = func_returns("run", "Nothing", vec![], vec![]);
    expect_rule(&project(vec![f], vec![u]), "TYPE_MIXED_RESOURCE_UNION");
}

#[test]
fn rejects_duplicate_variant_via_include() {
    let mut outer = union("Outer", &[]);
    outer.includes = vec!["A".to_string(), "B".to_string()];
    let mut a = union("A", &["Shared"]);
    a.name = "A".to_string();
    let b = union("B", &["Shared"]);
    let f = func_returns("run", "Nothing", vec![], vec![]);
    expect_rule(
        &project(vec![f], vec![outer, a, b]),
        "TYPE_DUPLICATE_VARIANT",
    );
}

#[test]
fn rejects_local_variant_conflicts_with_include() {
    let mut outer = union("Outer", &["Shared"]);
    outer.includes = vec!["A".to_string()];
    let a = union("A", &["Shared"]);
    let f = func_returns("run", "Nothing", vec![], vec![]);
    expect_rule(&project(vec![f], vec![outer, a]), "TYPE_DUPLICATE_VARIANT");
}

// --- call arity/args -------------------------------------------------------

#[test]
fn rejects_call_too_few_args() {
    let callee = func_returns(
        "helper",
        "Nothing",
        vec![param("a", "Integer", None), param("b", "Integer", None)],
        vec![],
    );
    let body = vec![IrOp::Eval {
        value: IrValue::Call {
            target: "helper".to_string(),
            args: vec![int_const("1")],
            type_: "Nothing".to_string(),
            loc: IrSourceLoc::default(),
        },
        loc: IrSourceLoc::default(),
    }];
    let caller = func_returns("run", "Nothing", vec![], body);
    expect_rule(
        &project(vec![callee, caller], vec![]),
        "TYPE_CALL_ARITY_MISMATCH",
    );
}

#[test]
fn rejects_call_argument_type() {
    let callee = func_returns(
        "helper",
        "Nothing",
        vec![param("a", "Integer", None)],
        vec![],
    );
    let body = vec![IrOp::Eval {
        value: IrValue::Call {
            target: "helper".to_string(),
            args: vec![const_of("String", "no")],
            type_: "Nothing".to_string(),
            loc: IrSourceLoc::default(),
        },
        loc: IrSourceLoc::default(),
    }];
    let caller = func_returns("run", "Nothing", vec![], body);
    expect_rule(
        &project(vec![callee, caller], vec![]),
        "TYPE_CALL_ARGUMENT_MISMATCH",
    );
}

#[test]
fn rejects_package_constant_not_callable() {
    let body = vec![ret(IrValue::Call {
        target: "math.pi".to_string(),
        args: vec![],
        type_: "Float".to_string(),
        loc: IrSourceLoc::default(),
    })];
    let f = func_returns("run", "Float", vec![], body);
    expect_rule(&project(vec![f], vec![]), "SYMBOL_NOT_CALLABLE");
}

#[test]
fn rejects_calling_non_function_local() {
    let body = vec![
        bind("x", "Integer", Some(int_const("1")), false, false),
        ret(IrValue::Call {
            target: "x".to_string(),
            args: vec![],
            type_: "Unknown".to_string(),
            loc: IrSourceLoc::default(),
        }),
    ];
    let f = func_returns("run", "Integer", vec![], body);
    expect_rule(&project(vec![f], vec![]), "SYMBOL_NOT_CALLABLE");
}

#[test]
fn rejects_builtin_math_bad_args() {
    let body = vec![ret(IrValue::Call {
        target: "math.sqrt".to_string(),
        args: vec![const_of("String", "x")],
        type_: "Float".to_string(),
        loc: IrSourceLoc::default(),
    })];
    let f = func_returns("run", "Float", vec![], body);
    expect_rule(&project(vec![f], vec![]), "TYPE_CALL_ARGUMENT_MISMATCH");
}

#[test]
fn accepts_builtin_math_good_args() {
    let body = vec![ret(IrValue::Call {
        target: "math.sqrt".to_string(),
        args: vec![const_of("Float", "4.0")],
        type_: "Float".to_string(),
        loc: IrSourceLoc::default(),
    })];
    let f = func_returns("run", "Float", vec![], body);
    accept(&project(vec![f], vec![]));
}

// --- resource axis ---------------------------------------------------------

#[test]
fn rejects_resource_without_res() {
    // A binding holding File but not RES-declared.
    let body = vec![bind(
        "f",
        "File",
        Some(IrValue::Local("g".to_string())),
        true,
        false,
    )];
    let f = func_returns("run", "Nothing", vec![param("g", "File", None)], body);
    expect_rule(&project(vec![f], vec![]), "TYPE_RESOURCE_REQUIRES_RES");
}

#[test]
fn rejects_res_on_non_resource() {
    // A RES-declared binding whose type is provably data.
    let mut f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind("x", "Integer", Some(int_const("1")), true, false)],
    );
    f.resource_owners
        .insert("x".to_string(), crate::escape::ResOwner::Local);
    expect_rule(&project(vec![f], vec![]), "TYPE_RES_REQUIRES_RESOURCE");
}

#[test]
fn rejects_collection_resource_element_without_res() {
    // A List OF File (bare resource, not RES-marked).
    let f = func_returns(
        "run",
        "Nothing",
        vec![param("xs", "List OF File", None)],
        vec![],
    );
    expect_rule(&project(vec![f], vec![]), "TYPE_RESOURCE_REQUIRES_RES");
}

#[test]
fn rejects_collection_res_on_data() {
    // List OF RES Integer — RES on a non-resource.
    let f = func_returns(
        "run",
        "Nothing",
        vec![param("xs", "List OF RES Integer", None)],
        vec![],
    );
    expect_rule(&project(vec![f], vec![]), "TYPE_RES_REQUIRES_RESOURCE");
}

#[test]
fn accepts_collection_res_file() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![param("xs", "List OF RES File", None)],
        vec![],
    );
    accept(&project(vec![f], vec![]));
}

#[test]
fn rejects_state_on_union() {
    let mut f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind("r", "Res STATE Integer", None, true, false)],
    );
    // craft a resource union type "Res" with a File variant so is_resource true and unions contains it.
    f.resource_owners
        .insert("r".to_string(), crate::escape::ResOwner::Local);
    let u = union("Res", &["File"]);
    expect_rule(&project(vec![f], vec![u]), "TYPE_UNION_STATE_FORBIDDEN");
}

#[test]
fn rejects_state_type_not_defaultable() {
    // A File resource with STATE of a union type (not defaultable).
    let mut f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind("h", "File STATE Shape", None, true, false)],
    );
    f.resource_owners
        .insert("h".to_string(), crate::escape::ResOwner::Local);
    expect_rule(
        &project(vec![f], vec![union("Shape", &["A", "B"])]),
        "TYPE_STATE_INVALID",
    );
}

#[test]
fn rejects_state_assign_no_state() {
    // Assign state on a File binding declared without STATE.
    let body = vec![
        bind("h", "File", None, true, false),
        IrOp::StateAssign {
            resource: "h".to_string(),
            value: int_const("1"),
            loc: IrSourceLoc::default(),
        },
    ];
    let mut f = func_returns("run", "Nothing", vec![], body);
    f.resource_owners
        .insert("h".to_string(), crate::escape::ResOwner::Local);
    expect_rule(&project(vec![f], vec![]), "TYPE_STATE_INVALID");
}

#[test]
fn rejects_state_assign_mismatch() {
    let body = vec![
        bind("h", "File STATE Integer", None, true, false),
        IrOp::StateAssign {
            resource: "h".to_string(),
            value: const_of("String", "x"),
            loc: IrSourceLoc::default(),
        },
    ];
    let mut f = func_returns("run", "Nothing", vec![], body);
    f.resource_owners
        .insert("h".to_string(), crate::escape::ResOwner::Local);
    expect_rule(&project(vec![f], vec![]), "TYPE_ASSIGNMENT_MISMATCH");
}

// --- use after move --------------------------------------------------------

#[test]
fn rejects_use_after_close() {
    // fs.close(h) then read h again.
    let body = vec![
        bind("h", "File", None, true, false),
        IrOp::Eval {
            value: IrValue::Call {
                target: "fs.close".to_string(),
                args: vec![IrValue::Local("h".to_string())],
                type_: "Nothing".to_string(),
                loc: IrSourceLoc::default(),
            },
            loc: IrSourceLoc::default(),
        },
        IrOp::Eval {
            value: IrValue::Call {
                target: "fs.close".to_string(),
                args: vec![IrValue::Local("h".to_string())],
                type_: "Nothing".to_string(),
                loc: IrSourceLoc::default(),
            },
            loc: IrSourceLoc::default(),
        },
    ];
    let mut f = func_returns("run", "Nothing", vec![], body);
    f.resource_owners
        .insert("h".to_string(), crate::escape::ResOwner::Local);
    expect_rule(&project(vec![f], vec![]), "TYPE_USE_AFTER_MOVE");
}

#[test]
fn rejects_borrowed_resource_close() {
    // A RES parameter is borrowed; closing it is invalid.
    let body = vec![IrOp::Eval {
        value: IrValue::Call {
            target: "fs.close".to_string(),
            args: vec![IrValue::Local("h".to_string())],
            type_: "Nothing".to_string(),
            loc: IrSourceLoc::default(),
        },
        loc: IrSourceLoc::default(),
    }];
    let f = func_returns("run", "Nothing", vec![param("h", "File", None)], body);
    expect_rule(&project(vec![f], vec![]), "TYPE_RESOURCE_BORROW_INVALIDATE");
}

// --- link functions --------------------------------------------------------

fn link_fn() -> crate::ir::IrLinkFunction {
    crate::ir::IrLinkFunction {
        alias: "lib".to_string(),
        name: "open".to_string(),
        library: "sqlite3".to_string(),
        symbol: "sqlite3_open".to_string(),
        params: vec![("path".to_string(), "String".to_string())],
        return_type: "Integer".to_string(),
        return_resource: false,
        abi_slots: vec![crate::ir::IrAbiSlot {
            name: "path".to_string(),
            ctype: "CString".to_string(),
            is_out: false,
        }],
        abi_return_name: "return".to_string(),
        abi_return_ctype: "CInt32".to_string(),
        consts: vec![],
        success_on: None,
        result: None,
        free: None,
    }
}

#[test]
fn accepts_valid_link_function() {
    let mut p = project(vec![func_returns("run", "Nothing", vec![], vec![])], vec![]);
    p.link_functions = vec![link_fn()];
    accept(&p);
}

#[test]
fn rejects_link_cptr_escape_in_param() {
    let mut lf = link_fn();
    lf.params = vec![("p".to_string(), "CPtr".to_string())];
    lf.abi_slots = vec![crate::ir::IrAbiSlot {
        name: "p".to_string(),
        ctype: "CPtr".to_string(),
        is_out: false,
    }];
    let mut p = project(vec![func_returns("run", "Nothing", vec![], vec![])], vec![]);
    p.link_functions = vec![lf];
    expect_rule(&p, "NATIVE_CPTR_ESCAPE");
}

#[test]
fn rejects_link_cptr_escape_in_return() {
    let mut lf = link_fn();
    lf.return_type = "CPtr".to_string();
    let mut p = project(vec![func_returns("run", "Nothing", vec![], vec![])], vec![]);
    p.link_functions = vec![lf];
    expect_rule(&p, "NATIVE_CPTR_ESCAPE");
}

#[test]
fn rejects_link_result_marker_not_out() {
    let mut lf = link_fn();
    lf.abi_slots = vec![
        crate::ir::IrAbiSlot {
            name: "path".to_string(),
            ctype: "CString".to_string(),
            is_out: false,
        },
        crate::ir::IrAbiSlot {
            name: "return".to_string(),
            ctype: "CInt32".to_string(),
            is_out: false,
        },
    ];
    lf.abi_return_name = "status".to_string();
    let mut p = project(vec![func_returns("run", "Nothing", vec![], vec![])], vec![]);
    p.link_functions = vec![lf];
    expect_rule(&p, "NATIVE_ABI_RESULT_MARKER");
}

#[test]
fn rejects_link_unbound_slot() {
    let mut lf = link_fn();
    lf.abi_slots = vec![
        crate::ir::IrAbiSlot {
            name: "path".to_string(),
            ctype: "CString".to_string(),
            is_out: false,
        },
        crate::ir::IrAbiSlot {
            name: "mystery".to_string(),
            ctype: "CInt32".to_string(),
            is_out: false,
        },
    ];
    let mut p = project(vec![func_returns("run", "Nothing", vec![], vec![])], vec![]);
    p.link_functions = vec![lf];
    expect_rule(&p, "NATIVE_ABI_UNBOUND_SLOT");
}

#[test]
fn rejects_link_out_slot_not_return() {
    let mut lf = link_fn();
    lf.abi_slots = vec![
        crate::ir::IrAbiSlot {
            name: "path".to_string(),
            ctype: "CString".to_string(),
            is_out: false,
        },
        crate::ir::IrAbiSlot {
            name: "extra".to_string(),
            ctype: "CInt32".to_string(),
            is_out: true,
        },
    ];
    lf.abi_return_name = "status".to_string();
    let mut p = project(vec![func_returns("run", "Nothing", vec![], vec![])], vec![]);
    p.link_functions = vec![lf];
    expect_rule(&p, "NATIVE_ABI_UNBOUND_SLOT");
}

#[test]
fn rejects_link_const_out() {
    let mut lf = link_fn();
    lf.consts = vec![("flags".to_string(), 1)];
    lf.abi_slots = vec![
        crate::ir::IrAbiSlot {
            name: "path".to_string(),
            ctype: "CString".to_string(),
            is_out: false,
        },
        crate::ir::IrAbiSlot {
            name: "flags".to_string(),
            ctype: "CInt32".to_string(),
            is_out: true,
        },
    ];
    let mut p = project(vec![func_returns("run", "Nothing", vec![], vec![])], vec![]);
    p.link_functions = vec![lf];
    expect_rule(&p, "NATIVE_CONST_OUT");
}

#[test]
fn rejects_link_no_result() {
    let mut lf = link_fn();
    lf.abi_slots = vec![crate::ir::IrAbiSlot {
        name: "path".to_string(),
        ctype: "CString".to_string(),
        is_out: false,
    }];
    lf.abi_return_name = "status".to_string();
    let mut p = project(vec![func_returns("run", "Nothing", vec![], vec![])], vec![]);
    p.link_functions = vec![lf];
    expect_rule(&p, "NATIVE_ABI_NO_RESULT");
}

#[test]
fn rejects_link_unbound_param() {
    let mut lf = link_fn();
    lf.params = vec![
        ("path".to_string(), "String".to_string()),
        ("extra".to_string(), "Integer".to_string()),
    ];
    let mut p = project(vec![func_returns("run", "Nothing", vec![], vec![])], vec![]);
    p.link_functions = vec![lf];
    expect_rule(&p, "NATIVE_ABI_UNBOUND_PARAM");
}

#[test]
fn rejects_link_const_unknown_slot() {
    let mut lf = link_fn();
    lf.consts = vec![("nope".to_string(), 1)];
    let mut p = project(vec![func_returns("run", "Nothing", vec![], vec![])], vec![]);
    p.link_functions = vec![lf];
    expect_rule(&p, "NATIVE_CONST_UNKNOWN_SLOT");
}

#[test]
fn rejects_link_invalid_free() {
    let mut lf = link_fn();
    lf.free = Some(crate::ir::IrFree {
        slot: "return".to_string(),
        symbol: String::new(),
    });
    let mut p = project(vec![func_returns("run", "Nothing", vec![], vec![])], vec![]);
    p.link_functions = vec![lf];
    expect_rule(&p, "NATIVE_FREE_INVALID");
}

#[test]
fn accepts_link_const_pin() {
    let mut lf = link_fn();
    lf.consts = vec![("flags".to_string(), 1)];
    lf.abi_slots = vec![
        crate::ir::IrAbiSlot {
            name: "path".to_string(),
            ctype: "CString".to_string(),
            is_out: false,
        },
        crate::ir::IrAbiSlot {
            name: "flags".to_string(),
            ctype: "CInt32".to_string(),
            is_out: false,
        },
    ];
    let mut p = project(vec![func_returns("run", "Nothing", vec![], vec![])], vec![]);
    p.link_functions = vec![lf];
    accept(&p);
}

// --- trap ------------------------------------------------------------------

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
            type_: "Integer".to_string(),
            by_ref: false,
        })],
    );
    let maker = func_returns(
        "make",
        "FUNC() AS Integer",
        vec![],
        vec![ret(IrValue::Closure {
            name: "body".to_string(),
            type_: "FUNC() AS Integer".to_string(),
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
                type_: "List OF Integer".to_string(),
                values: vec![int_const("1")],
            }),
            false,
            true,
        ),
        binding(
            "m",
            "Map OF String TO Integer",
            Some(IrValue::MapLiteral {
                type_: "Map OF String TO Integer".to_string(),
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
fn collect_source_diagnostics_filters_relocated() {
    use std::path::Path;
    // TYPE_BINARY_OPERATOR_MISMATCH is relocated; UNREACHABLE_AFTER_EXIT is not.
    let body = vec![ret(binary(
        "-",
        const_of("String", "a"),
        int_const("1"),
        "Integer",
    ))];
    let p = project(vec![func("run", vec![], body)], vec![]);
    let diags = super::collect_source_diagnostics(&p, Path::new("/proj"));
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
    let diags = super::collect_source_diagnostics(&p, Path::new("/proj"));
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
        type_: "Secret".to_string(),
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
        type_: "Widget".to_string(),
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
        type_: "Unknown".to_string(),
    })];
    let f = func_returns("run", "Integer", vec![param("w", "Widget", None)], body);
    expect_rule(&project(vec![f], vec![ty]), "TYPE_MEMBER_NOT_VISIBLE");
}

#[test]
fn rejects_read_only_record_constructor() {
    // Constructing a MapEntry (read-only builtin record).
    let body = vec![ret(IrValue::Constructor {
        type_: "MapEntry OF String TO Integer".to_string(),
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
            type_: "Nothing".to_string(),
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
            type_: "Nothing".to_string(),
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
            type_: "Nothing".to_string(),
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
            type_: "List OF Integer".to_string(),
            values: vec![],
        }],
        type_: "Unknown".to_string(),
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
                type_: "List OF Integer".to_string(),
                values: vec![],
            },
        ],
        type_: "Boolean".to_string(),
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
        type_: "Integer".to_string(),
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
        type_: "Boolean".to_string(),
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
        type_: "Integer".to_string(),
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
        type_: "Integer".to_string(),
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
                    type_: "Color".to_string(),
                }),
                guard: None,
                body: vec![ret(int_const("1"))],
                loc: IrSourceLoc::default(),
            },
            IrMatchCase {
                pattern: IrMatchPattern::Value(IrValue::MemberAccess {
                    target: Box::new(IrValue::Local("Color".to_string())),
                    member: "Green".to_string(),
                    type_: "Color".to_string(),
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
                type_: "Color".to_string(),
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

// --- link: more than one result marker -------------------------------------

#[test]
fn rejects_link_multiple_result_markers() {
    let mut lf = link_fn();
    lf.abi_slots = vec![
        crate::ir::IrAbiSlot {
            name: "path".to_string(),
            ctype: "CString".to_string(),
            is_out: false,
        },
        crate::ir::IrAbiSlot {
            name: "return".to_string(),
            ctype: "CInt32".to_string(),
            is_out: true,
        },
    ];
    // abi_return_name is also "return" -> two markers.
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
        bind("h", "File", None, true, false),
        ret(IrValue::Local("h".to_string())),
    ];
    let mut f = func_returns("run", "File", vec![], body);
    f.resource_owners
        .insert("h".to_string(), crate::escape::ResOwner::Local);
    let got = rules(&project(vec![f], vec![]));
    assert!(
        !got.contains(&"TYPE_USE_AFTER_MOVE".to_string()),
        "unexpected use-after-move: {got:?}"
    );
}

#[test]
fn rejects_double_move_close_then_return() {
    let body = vec![
        bind("h", "File", None, true, false),
        IrOp::Eval {
            value: IrValue::Call {
                target: "fs.close".to_string(),
                args: vec![IrValue::Local("h".to_string())],
                type_: "Nothing".to_string(),
                loc: IrSourceLoc::default(),
            },
            loc: IrSourceLoc::default(),
        },
        ret(IrValue::Local("h".to_string())),
    ];
    let mut f = func_returns("run", "File", vec![], body);
    f.resource_owners
        .insert("h".to_string(), crate::escape::ResOwner::Local);
    expect_rule(&project(vec![f], vec![]), "TYPE_USE_AFTER_MOVE");
}

// --- resource element not owner (list literal + get borrow) ----------------

#[test]
fn rejects_temporary_in_resource_list() {
    // List OF RES File with a non-local element (a call result) — not an owner.
    let body = vec![ret(IrValue::ListLiteral {
        type_: "List OF RES File".to_string(),
        values: vec![IrValue::Call {
            target: "fs.open".to_string(),
            args: vec![const_of("String", "f")],
            type_: "File".to_string(),
            loc: IrSourceLoc::default(),
        }],
    })];
    let f = func_returns("run", "List OF RES File", vec![], body);
    expect_rule(&project(vec![f], vec![]), "TYPE_RESOURCE_ELEMENT_NOT_OWNER");
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
                    type_: "Integer".to_string(),
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
            type_: "FUNC() AS Integer".to_string(),
            captures: vec![int_const("1")],
        })],
    );
    let err = check(&project(vec![closure_body, maker], vec![])).expect_err("capture out of range");
    assert!(err.contains("out of range"), "{err}");
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
            op: "+".to_string(),
            left: Box::new(IrValue::Capture {
                index: 0,
                type_: "Integer".to_string(),
                by_ref: false,
            }),
            right: Box::new(IrValue::Capture {
                index: 9,
                type_: "Integer".to_string(),
                by_ref: false,
            }),
            type_: "Integer".to_string(),
            loc: IrSourceLoc::default(),
        })],
    );
    let maker = func_returns(
        "make",
        "FUNC() AS Integer",
        vec![],
        vec![ret(IrValue::Closure {
            name: "body".to_string(),
            type_: "FUNC() AS Integer".to_string(),
            captures: vec![int_const("1")],
        })],
    );
    let err = check(&project(vec![closure_body, maker], vec![]))
        .expect_err("nested capture out of range");
    assert!(err.contains("out of range"), "{err}");
}

// --- compatible / expression_compatible coercion paths ---------------------

#[test]
fn accepts_byte_literal_into_byte_param() {
    let callee = func_returns("helper", "Nothing", vec![param("b", "Byte", None)], vec![]);
    let body = vec![IrOp::Eval {
        value: IrValue::Call {
            target: "helper".to_string(),
            args: vec![int_const("5")],
            type_: "Nothing".to_string(),
            loc: IrSourceLoc::default(),
        },
        loc: IrSourceLoc::default(),
    }];
    let caller = func_returns("run", "Nothing", vec![], body);
    accept(&project(vec![callee, caller], vec![]));
}

#[test]
fn accepts_integer_literal_into_fixed_param() {
    let callee = func_returns("helper", "Nothing", vec![param("x", "Fixed", None)], vec![]);
    let body = vec![IrOp::Eval {
        value: IrValue::Call {
            target: "helper".to_string(),
            args: vec![int_const("5")],
            type_: "Nothing".to_string(),
            loc: IrSourceLoc::default(),
        },
        loc: IrSourceLoc::default(),
    }];
    let caller = func_returns("run", "Nothing", vec![], body);
    accept(&project(vec![callee, caller], vec![]));
}

#[test]
fn accepts_negated_literal_into_fixed_binding() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind(
            "x",
            "Fixed",
            Some(unary("-", int_const("1"), "Integer")),
            true,
            false,
        )],
    );
    accept(&project(vec![f], vec![]));
}

#[test]
fn accepts_union_variant_return() {
    // Returning a variant record value where the union type is expected.
    let body = vec![ret(IrValue::Constructor {
        type_: "Circle".to_string(),
        args: vec![],
    })];
    let f = func_returns("run", "Shape", vec![], body);
    let mut u = union("Shape", &["Circle", "Square"]);
    u.variants[0].fields = vec![];
    accept(&project(vec![f], vec![u]));
}

#[test]
fn accepts_list_compatible_recursion() {
    // Return a List OF Integer where List OF Integer is expected via a param.
    let body = vec![ret(IrValue::Local("xs".to_string()))];
    let f = func_returns(
        "run",
        "List OF Integer",
        vec![param("xs", "List OF Integer", None)],
        body,
    );
    accept(&project(vec![f], vec![]));
}

#[test]
fn accepts_map_compatible_recursion() {
    let body = vec![ret(IrValue::Local("m".to_string()))];
    let f = func_returns(
        "run",
        "Map OF String TO Integer",
        vec![param("m", "Map OF String TO Integer", None)],
        body,
    );
    accept(&project(vec![f], vec![]));
}

// --- unknown value poisoning cascade ---------------------------------------

#[test]
fn poisoned_initializer_yields_unknown_value() {
    // A binary op mismatch poisons the value; the bind then reports UNKNOWN_VALUE.
    let body = vec![bind(
        "x",
        "Integer",
        Some(binary(
            "-",
            const_of("String", "a"),
            int_const("1"),
            "Integer",
        )),
        false,
        false,
    )];
    let f = func_returns("run", "Nothing", vec![], body);
    let got = rules(&project(vec![f], vec![]));
    assert!(got.contains(&"TYPE_UNKNOWN_VALUE".to_string()), "{got:?}");
}

#[test]
fn poisoned_return_yields_unknown_value() {
    let body = vec![ret(binary(
        "-",
        const_of("String", "a"),
        int_const("1"),
        "Integer",
    ))];
    let f = func_returns("run", "Integer", vec![], body);
    let got = rules(&project(vec![f], vec![]));
    assert!(got.contains(&"TYPE_UNKNOWN_VALUE".to_string()), "{got:?}");
}

// --- map-key ownership violation -------------------------------------------

#[test]
fn rejects_map_key_thread_ownership() {
    // A Map keyed on a Thread handle: ordinary collections cannot own it.
    let f = func_returns(
        "run",
        "Nothing",
        vec![param("m", "Map OF Thread OF Integer TO Integer", None)],
        vec![],
    );
    expect_rule(
        &project(vec![f], vec![]),
        "TYPE_COLLECTION_OWNERSHIP_VIOLATION",
    );
}

#[test]
fn rejects_map_key_record_with_resource() {
    // A record field carrying a resource makes a record key an ownership
    // violation (contains_resource_or_thread over record_field_lists).
    // Craft the record with a File field (records-cannot-own is separately
    // reported, but the map-key ownership check still fires).
    let mut holder = record_typed("Holder", &[("f", "File")]);
    holder.file = "src/main.mfb".to_string();
    let f = func_returns(
        "run",
        "Nothing",
        vec![param("m", "Map OF Holder TO Integer", None)],
        vec![],
    );
    expect_rule(
        &project(vec![f], vec![holder]),
        "TYPE_COLLECTION_OWNERSHIP_VIOLATION",
    );
}

// --- compatible recursion (list / result / map) ----------------------------

#[test]
fn rejects_nested_list_mismatch_return() {
    // RETURN a List OF List OF Integer where List OF List OF String expected.
    let body = vec![ret(IrValue::Local("xs".to_string()))];
    let f = func_returns(
        "run",
        "List OF List OF String",
        vec![param("xs", "List OF List OF Integer", None)],
        body,
    );
    expect_rule(&project(vec![f], vec![]), "TYPE_RETURN_MISMATCH");
}

#[test]
fn accepts_nested_map_return() {
    let body = vec![ret(IrValue::Local("m".to_string()))];
    let f = func_returns(
        "run",
        "Map OF String TO List OF Integer",
        vec![param("m", "Map OF String TO List OF Integer", None)],
        body,
    );
    accept(&project(vec![f], vec![]));
}

#[test]
fn accepts_result_of_return() {
    // Result OF Integer compatible recursion via compatible().
    let body = vec![ret(IrValue::Local("r".to_string()))];
    let f = func_returns(
        "run",
        "Result OF Integer",
        vec![param("r", "Result OF Integer", None)],
        body,
    );
    accept(&project(vec![f], vec![]));
}

// --- union include cycle / expansion ---------------------------------------

#[test]
fn union_include_cycle_is_bounded() {
    // Two unions that include each other — the expansion is cycle-guarded.
    let mut a = union("A", &["X"]);
    a.includes = vec!["B".to_string()];
    let mut b = union("B", &["Y"]);
    b.includes = vec!["A".to_string()];
    let f = func_returns("run", "Nothing", vec![], vec![]);
    // No panic / infinite loop; may or may not emit but must terminate.
    let _ = rules(&project(vec![f], vec![a, b]));
}

#[test]
fn accepts_union_with_included_union() {
    let mut outer = union("Outer", &["Local1"]);
    outer.includes = vec!["Inner".to_string()];
    let inner = union("Inner", &["A", "B"]);
    let f = func_returns("run", "Nothing", vec![], vec![]);
    accept(&project(vec![f], vec![outer, inner]));
}

// --- record field include cycle --------------------------------------------

#[test]
fn record_include_cycle_terminates() {
    // Two records including each other via `includes` — collect_record_fields
    // cycle guard.
    let mut a = record("A", &["fa"]);
    a.includes = vec!["B".to_string()];
    let mut b = record("B", &["fb"]);
    b.includes = vec!["A".to_string()];
    // Access a field present via include chain.
    let body = vec![ret(IrValue::MemberAccess {
        target: Box::new(IrValue::Local("x".to_string())),
        member: "fb".to_string(),
        type_: "Unknown".to_string(),
    })];
    let f = func_returns("run", "Integer", vec![param("x", "A", None)], body);
    accept(&project(vec![f], vec![a, b]));
}

// --- for / foreach unknown-typed bound skip --------------------------------

#[test]
fn for_unknown_bound_is_skipped() {
    // A local typed "Unknown" as the FOR end bound is skipped, not rejected.
    let body = vec![
        bind("u", "Unknown", Some(int_const("0")), false, false),
        IrOp::For {
            name: "i".to_string(),
            type_: "Integer".to_string(),
            start: int_const("0"),
            end: IrValue::Local("u".to_string()),
            step: int_const("1"),
            body: vec![],
            loc: IrSourceLoc::default(),
        },
    ];
    let f = func_returns("run", "Nothing", vec![], body);
    let got = rules(&project(vec![f], vec![]));
    assert!(
        !got.iter().any(|r| r == "TYPE_FOR_REQUIRES_NUMERIC"),
        "{got:?}"
    );
}

#[test]
fn foreach_unknown_iterable_is_skipped() {
    let body = vec![
        bind("u", "Unknown", Some(int_const("0")), false, false),
        IrOp::ForEach {
            name: "e".to_string(),
            type_: "Integer".to_string(),
            iterable: IrValue::Local("u".to_string()),
            body: vec![],
            loc: IrSourceLoc::default(),
        },
    ];
    let f = func_returns("run", "Nothing", vec![], body);
    let got = rules(&project(vec![f], vec![]));
    assert!(
        !got.iter().any(|r| r == "TYPE_FOR_EACH_REQUIRES_COLLECTION"),
        "{got:?}"
    );
}

// --- more builtin package overload branches --------------------------------

fn eval_call(target: &str, args: Vec<IrValue>) -> IrOp {
    IrOp::Eval {
        value: IrValue::Call {
            target: target.to_string(),
            args,
            type_: "Unknown".to_string(),
            loc: IrSourceLoc::default(),
        },
        loc: IrSourceLoc::default(),
    }
}

#[test]
fn rejects_bits_bad_args() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![eval_call("bits.band", vec![const_of("String", "x")])],
    );
    expect_rule(&project(vec![f], vec![]), "TYPE_CALL_ARGUMENT_MISMATCH");
}

#[test]
fn rejects_encoding_bad_args() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![eval_call("encoding.hexDecode", vec![int_const("1")])],
    );
    let got = rules(&project(vec![f], vec![]));
    assert!(got.iter().any(|r| r.starts_with("TYPE_CALL")), "{got:?}");
}

#[test]
fn rejects_io_bad_args() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![eval_call(
            "io.print",
            vec![int_const("1"), int_const("2"), int_const("3")],
        )],
    );
    let got = rules(&project(vec![f], vec![]));
    assert!(got.iter().any(|r| r.starts_with("TYPE_CALL")), "{got:?}");
}

#[test]
fn rejects_fs_bad_args() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![eval_call("fs.appendText", vec![int_const("1")])],
    );
    let got = rules(&project(vec![f], vec![]));
    assert!(got.iter().any(|r| r.starts_with("TYPE_CALL")), "{got:?}");
}

#[test]
fn rejects_net_bad_args() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![eval_call("net.bindUdp", vec![int_const("1")])],
    );
    let got = rules(&project(vec![f], vec![]));
    assert!(got.iter().any(|r| r.starts_with("TYPE_CALL")), "{got:?}");
}

#[test]
fn rejects_vector_bad_args() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![eval_call("vector.abs", vec![const_of("String", "x")])],
    );
    let got = rules(&project(vec![f], vec![]));
    assert!(got.iter().any(|r| r.starts_with("TYPE_CALL")), "{got:?}");
}

#[test]
fn unknown_package_call_is_skipped() {
    // A dotted call that resolves to no known builtin package is left alone.
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![eval_call("nonpkg.doThing", vec![int_const("1")])],
    );
    accept(&project(vec![f], vec![]));
}

// --- rich closure body: walk_captures over every value shape ---------------

#[test]
fn closure_body_captures_walked_over_all_shapes() {
    // A closure body (1 slot) whose ops embed a Capture inside each value shape,
    // all in range (index 0). Exercises walk_captures + collect_closures arms.
    let cap = || IrValue::Capture {
        index: 0,
        type_: "Integer".to_string(),
        by_ref: false,
    };
    let body = vec![
        // Constructor(Capture)
        bind(
            "a",
            "Point",
            Some(IrValue::Constructor {
                type_: "Point".to_string(),
                args: vec![cap(), int_const("1")],
            }),
            false,
            false,
        ),
        // Call(Capture)
        eval_call("io.print", vec![cap()]),
        // ListLiteral(Capture)
        bind(
            "l",
            "List OF Integer",
            Some(IrValue::ListLiteral {
                type_: "List OF Integer".to_string(),
                values: vec![cap()],
            }),
            false,
            false,
        ),
        // MapLiteral(Capture)
        bind(
            "m",
            "Map OF Integer TO Integer",
            Some(IrValue::MapLiteral {
                type_: "Map OF Integer TO Integer".to_string(),
                entries: vec![(cap(), cap())],
            }),
            false,
            false,
        ),
        // Binary(Capture, Capture)
        bind(
            "bn",
            "Integer",
            Some(IrValue::Binary {
                op: "+".to_string(),
                left: Box::new(cap()),
                right: Box::new(cap()),
                type_: "Integer".to_string(),
                loc: IrSourceLoc::default(),
            }),
            false,
            false,
        ),
        // Unary(Capture)
        bind(
            "un",
            "Integer",
            Some(IrValue::Unary {
                op: "-".to_string(),
                operand: Box::new(cap()),
                type_: "Integer".to_string(),
                loc: IrSourceLoc::default(),
            }),
            false,
            false,
        ),
        // WithUpdate(target=Capture)
        bind(
            "wu",
            "Point",
            Some(IrValue::WithUpdate {
                type_: "Point".to_string(),
                target: Box::new(IrValue::Local("a".to_string())),
                updates: vec![crate::ir::IrRecordUpdate {
                    field: "x".to_string(),
                    value: cap(),
                }],
            }),
            false,
            false,
        ),
        // MemberAccess(target=Capture)
        bind(
            "ma",
            "Integer",
            Some(IrValue::MemberAccess {
                target: Box::new(IrValue::Local("a".to_string())),
                member: "x".to_string(),
                type_: "Integer".to_string(),
            }),
            false,
            false,
        ),
        // Closure(Capture) as a nested closure argument
        eval_call(
            "io.print",
            vec![IrValue::Closure {
                name: "inner".to_string(),
                type_: "FUNC() AS Integer".to_string(),
                captures: vec![cap()],
            }],
        ),
        ret(cap()),
    ];
    let closure_body = func_returns("body", "Integer", vec![], body);
    let inner = func_returns(
        "inner",
        "Integer",
        vec![],
        vec![ret(IrValue::Capture {
            index: 0,
            type_: "Integer".to_string(),
            by_ref: false,
        })],
    );
    let maker = func_returns(
        "make",
        "FUNC() AS Integer",
        vec![],
        vec![ret(IrValue::Closure {
            name: "body".to_string(),
            type_: "FUNC() AS Integer".to_string(),
            captures: vec![int_const("1")],
        })],
    );
    // The body reads slot 0 (in range) throughout; no capture violation.
    let diags = rules(&project(
        vec![closure_body, inner, maker],
        vec![record("Point", &["x", "y"])],
    ));
    assert!(
        !diags.iter().any(|r| r.contains("capture index")),
        "unexpected capture violation: {diags:?}"
    );
}

// --- collect_local_reads / collect_closures over union/result shapes -------

#[test]
fn accepts_function_with_union_and_result_value_shapes() {
    // Exercises collect_local_reads_value and collect_closures over
    // UnionWrap/UnionExtract/ResultIsOk/ResultValue/ResultError shapes.
    let body = vec![
        bind(
            "w",
            "Shape",
            Some(IrValue::UnionWrap {
                union_type: "Shape".to_string(),
                member_type: "Circle".to_string(),
                value: Box::new(IrValue::Local("c".to_string())),
            }),
            false,
            false,
        ),
        bind(
            "e",
            "Circle",
            Some(IrValue::UnionExtract {
                type_: "Circle".to_string(),
                value: Box::new(IrValue::Local("w".to_string())),
            }),
            false,
            false,
        ),
        bind(
            "ok",
            "Boolean",
            Some(IrValue::ResultIsOk {
                value: Box::new(IrValue::Local("r".to_string())),
            }),
            false,
            false,
        ),
        bind(
            "v",
            "Integer",
            Some(IrValue::ResultValue {
                type_: "Integer".to_string(),
                value: Box::new(IrValue::Local("r".to_string())),
            }),
            false,
            false,
        ),
        bind(
            "er",
            "Error",
            Some(IrValue::ResultError {
                value: Box::new(IrValue::Local("r".to_string())),
            }),
            false,
            false,
        ),
        ret_none(),
    ];
    let mut u = union("Shape", &["Circle", "Square"]);
    u.variants[0].fields = vec![];
    let f = func_returns(
        "run",
        "Nothing",
        vec![
            param("c", "Circle", None),
            param("r", "Result OF Integer", None),
        ],
        body,
    );
    // Circle is a variant record; register it via the union.
    accept(&project(vec![f], vec![u, record("Circle", &[])]));
}

// --- assignment via LocalRef / FunctionRef read shapes ---------------------

#[test]
fn accepts_localref_and_functionref_values() {
    let body = vec![
        bind(
            "r",
            "Integer",
            Some(IrValue::LocalRef {
                name: "x".to_string(),
                type_: "Integer".to_string(),
            }),
            false,
            false,
        ),
        bind(
            "fr",
            "FUNC() AS Integer",
            Some(IrValue::FunctionRef {
                name: "helper".to_string(),
                type_: "FUNC() AS Integer".to_string(),
            }),
            false,
            false,
        ),
        ret_none(),
    ];
    let helper = func_returns("helper", "Integer", vec![], vec![ret(int_const("1"))]);
    let f = func_returns("run", "Nothing", vec![param("x", "Integer", None)], body);
    accept(&project(vec![helper, f], vec![]));
}

// --- resource element borrow (RES bind of collections.get) -----------------

fn get_call(list: &str, ret_type: &str) -> IrValue {
    IrValue::Call {
        target: "collections.get".to_string(),
        args: vec![IrValue::Local(list.to_string()), int_const("0")],
        type_: ret_type.to_string(),
        loc: IrSourceLoc::default(),
    }
}

#[test]
fn rejects_res_bind_of_borrowed_element() {
    // RES h = collections.get(xs, 0) where the element type is a resource.
    let body = vec![bind("h", "File", Some(get_call("xs", "File")), true, false)];
    let mut f = func_returns(
        "run",
        "Nothing",
        vec![param("xs", "List OF RES File", None)],
        body,
    );
    f.resource_owners
        .insert("h".to_string(), crate::escape::ResOwner::Local);
    expect_rule(&project(vec![f], vec![]), "TYPE_RESOURCE_ELEMENT_NOT_OWNER");
}

#[test]
fn rejects_return_borrowed_resource_element() {
    // RETURN collections.get(xs, 0) whose element is a resource.
    let body = vec![ret(get_call("xs", "File"))];
    let f = func_returns(
        "run",
        "File",
        vec![param("xs", "List OF RES File", None)],
        body,
    );
    expect_rule(&project(vec![f], vec![]), "TYPE_RESOURCE_ELEMENT_NOT_OWNER");
}

// --- is_defaultable branches (MUT without value) ---------------------------

#[test]
fn mut_list_is_defaultable() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind("xs", "List OF Integer", None, true, true)],
    );
    accept(&project(vec![f], vec![]));
}

#[test]
fn mut_map_is_defaultable() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind("m", "Map OF String TO Integer", None, true, true)],
    );
    accept(&project(vec![f], vec![]));
}

#[test]
fn rejects_mut_func_not_defaultable() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind("fn", "FUNC() AS Integer", None, true, true)],
    );
    expect_rule(
        &project(vec![f], vec![]),
        "TYPE_MUT_REQUIRES_DEFAULTABLE_TYPE",
    );
}

#[test]
fn rejects_mut_enum_not_defaultable() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind("c", "Color", None, true, true)],
    );
    expect_rule(
        &project(vec![f], vec![enum_type("Color", &["Red"])]),
        "TYPE_MUT_REQUIRES_DEFAULTABLE_TYPE",
    );
}

#[test]
fn mut_record_of_defaultable_fields_ok() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind("p", "Point", None, true, true)],
    );
    accept(&project(vec![f], vec![record("Point", &["x", "y"])]));
}

#[test]
fn rejects_mut_record_with_nondefaultable_field() {
    // A record whose field type is a FUNC — not defaultable.
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind("h", "Holder", None, true, true)],
    );
    expect_rule(
        &project(
            vec![f],
            vec![record_typed("Holder", &[("cb", "FUNC() AS Integer")])],
        ),
        "TYPE_MUT_REQUIRES_DEFAULTABLE_TYPE",
    );
}

#[test]
fn rejects_mut_unknown_record_not_defaultable() {
    // An unknown type name (not in record_field_lists) is not defaultable.
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind("w", "Widget", None, true, true)],
    );
    expect_rule(
        &project(vec![f], vec![]),
        "TYPE_MUT_REQUIRES_DEFAULTABLE_TYPE",
    );
}

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
                    type_: "Color".to_string(),
                },
                IrValue::MemberAccess {
                    target: Box::new(IrValue::Local("Color".to_string())),
                    member: "Green".to_string(),
                    type_: "Color".to_string(),
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
                type_: "Color".to_string(),
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
fn accepts_qualified_type_name_match() {
    // A return of a `pkg.Point`-typed value where `Point` is expected resolves
    // via bare-name equality in compatible().
    let body = vec![ret(IrValue::Local("p".to_string()))];
    let f = func_returns("run", "Point", vec![param("p", "pkg.Point", None)], body);
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
                    ">",
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

// --- resource moves inside nested blocks -----------------------------------

fn close_eval(h: &str) -> IrOp {
    IrOp::Eval {
        value: IrValue::Call {
            target: "fs.close".to_string(),
            args: vec![IrValue::Local(h.to_string())],
            type_: "Nothing".to_string(),
            loc: IrSourceLoc::default(),
        },
        loc: IrSourceLoc::default(),
    }
}

fn owner_fn(name: &str, ret: &str, body: Vec<IrOp>, owners: &[&str]) -> IrFunction {
    let mut f = func_returns(name, ret, vec![], body);
    for o in owners {
        f.resource_owners
            .insert((*o).to_string(), crate::escape::ResOwner::Local);
    }
    f
}

#[test]
fn move_in_if_branch_propagates_past_join() {
    // Close h inside an IF then-branch (fall-through), then use it after the IF.
    let body = vec![
        bind("h", "File", None, true, false),
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
    let body = vec![bind("h", "File", None, true, false), m, close_eval("h")];
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
        bind("h", "File", None, true, false),
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
fn move_in_foreach_body_borrowed() {
    // Inside FOR EACH, the element is borrowed; closing it is a borrow-invalidate,
    // exercising the ForEach arm of check_resource_moves.
    let fe = IrOp::ForEach {
        name: "el".to_string(),
        type_: "File".to_string(),
        iterable: IrValue::Local("xs".to_string()),
        body: vec![close_eval("el")],
        loc: IrSourceLoc::default(),
    };
    let mut f = func_returns(
        "run",
        "Nothing",
        vec![param("xs", "List OF RES File", None)],
        vec![fe],
    );
    let _ = &mut f;
    expect_rule(&project(vec![f], vec![]), "TYPE_RESOURCE_BORROW_INVALIDATE");
}

#[test]
fn res_transfer_moves_source() {
    // RES b = a — an ownership transfer moves `a`; a later use is after-move.
    let body = vec![
        bind("a", "File", None, true, false),
        bind(
            "b",
            "File",
            Some(IrValue::Local("a".to_string())),
            true,
            false,
        ),
        close_eval("a"),
    ];
    let f = owner_fn("run", "Nothing", body, &["a", "b"]);
    expect_rule(&project(vec![f], vec![]), "TYPE_USE_AFTER_MOVE");
}

// --- thread.result member --------------------------------------------------

#[test]
fn rejects_thread_result_member() {
    let body = vec![ret(IrValue::MemberAccess {
        target: Box::new(IrValue::Local("t".to_string())),
        member: "result".to_string(),
        type_: "Unknown".to_string(),
    })];
    let f = func_returns(
        "run",
        "Integer",
        vec![param("t", "Thread OF Integer", None)],
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
        type_: "Unknown".to_string(),
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
            type_: "Unknown".to_string(),
            loc: IrSourceLoc::default(),
        }],
        type_: "Float".to_string(),
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
        union_type: "Shape".to_string(),
        member_type: String::new(),
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
        .insert("c".to_string(), crate::escape::ResOwner::Local);
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
        .insert("p".to_string(), crate::escape::ResOwner::Local);
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
        .insert("s".to_string(), crate::escape::ResOwner::Local);
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
            type_: "Integer".to_string(),
            value: Box::new(IrValue::Capture {
                index: 5,
                type_: "Integer".to_string(),
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
            type_: "FUNC() AS Integer".to_string(),
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
                type_: "Integer".to_string(),
                value: Box::new(IrValue::Capture {
                    index: 8,
                    type_: "Integer".to_string(),
                    by_ref: false,
                }),
            }),
            member: "x".to_string(),
            type_: "Integer".to_string(),
        })],
    );
    let maker = func_returns(
        "make",
        "FUNC() AS Integer",
        vec![],
        vec![ret(IrValue::Closure {
            name: "body".to_string(),
            type_: "FUNC() AS Integer".to_string(),
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
                type_: "Color".to_string(),
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
    let get_name = func_returns("getName", "String", vec![], vec![ret(const_of("String", "a"))]);
    let confused = IrValue::MemberAccess {
        target: Box::new(IrValue::Call {
            target: "getName".to_string(),
            args: vec![],
            type_: "Account".to_string(),
            loc: IrSourceLoc::default(),
        }),
        member: "balance".to_string(),
        type_: "Integer".to_string(),
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
    let get_name = func_returns("getName", "String", vec![], vec![ret(const_of("String", "a"))]);
    let confused = binary(
        "-",
        IrValue::Call {
            target: "getName".to_string(),
            args: vec![],
            type_: "Integer".to_string(),
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
    let get_name = func_returns("getName", "String", vec![], vec![ret(const_of("String", "a"))]);
    let caller = func(
        "run",
        vec![],
        vec![ret(IrValue::CallResult {
            target: "getName".to_string(),
            args: vec![],
            type_: "Integer".to_string(),
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
    let get_name = func_returns("getName", "String", vec![], vec![ret(const_of("String", "a"))]);
    let caller = func_returns(
        "run",
        "String",
        vec![],
        vec![ret(IrValue::Call {
            target: "getName".to_string(),
            args: vec![],
            type_: "String".to_string(),
            loc: IrSourceLoc::default(),
        })],
    );
    accept(&project(vec![get_name, caller], vec![]));

    // An `Unknown` annotation is unresolved, not a disagreement.
    let get_name = func_returns("getName", "String", vec![], vec![ret(const_of("String", "a"))]);
    let caller = func_returns(
        "run",
        "String",
        vec![],
        vec![ret(IrValue::Call {
            target: "getName".to_string(),
            args: vec![],
            type_: "Unknown".to_string(),
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
        type_: "String".to_string(),
    };
    let body = vec![
        bind(
            "acct",
            "Account",
            Some(IrValue::Constructor {
                type_: "Account".to_string(),
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
        vec![ret(binary("<", int_const("1"), int_const("2"), "Integer"))],
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
            "&",
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
        vec![ret(binary("+", int_const("1"), int_const("2"), "String"))],
    );
    expect_rule(
        &project(vec![caller], vec![]),
        "PACKAGE_BINARY_REPRESENTATION_VERIFY_TYPE",
    );

    // `NOT` yields a Boolean; negation preserves its operand type.
    let caller = func(
        "run",
        vec![],
        vec![ret(unary("NOT", const_of("Boolean", "true"), "Integer"))],
    );
    expect_rule(
        &project(vec![caller], vec![]),
        "PACKAGE_BINARY_REPRESENTATION_VERIFY_TYPE",
    );
    let caller = func(
        "run",
        vec![],
        vec![ret(unary("-", int_const("1"), "String"))],
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
        vec![ret(binary("<", int_const("1"), int_const("2"), "Boolean"))],
    );
    accept(&project(vec![caller], vec![]));
}
