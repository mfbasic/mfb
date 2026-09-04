use super::{check, collect_diagnostics};
use crate::ir::{
    IrBinding, IrField, IrFunction, IrMatchCase, IrMatchPattern, IrOp, IrParam, IrProject,
    IrSourceLoc, IrType, IrValue, IrVariant,
};
use crate::operators::{BinaryOp, UnaryOp};
use crate::types::ParameterType;
use std::collections::HashMap;

fn project(functions: Vec<IrFunction>, types: Vec<IrType>) -> IrProject {
    crate::ir::test_support::project_fixture("t", functions, types)
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

/// Assert `project` yields NO diagnostic with `rule`. Weaker than `expect_clean`
/// on purpose: it pins that one specific rule stopped firing without also
/// asserting the project is diagnostic-free, so it survives unrelated rules.
fn expect_no_rule(project: &IrProject, rule: &str) {
    let got = rules(project);
    assert!(
        !got.iter().any(|r| r == rule),
        "expected no {rule}, got {got:?}"
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
        type_: crate::types::ParameterType::parse(ty),
        value: v.to_string(),
    }
}

fn binary(op: BinaryOp, left: IrValue, right: IrValue, ty: &str) -> IrValue {
    IrValue::Binary {
        op,
        left: Box::new(left),
        right: Box::new(right),
        type_: crate::types::ParameterType::parse(ty),
        loc: IrSourceLoc::default(),
    }
}

fn unary(op: UnaryOp, operand: IrValue, ty: &str) -> IrValue {
    IrValue::Unary {
        op,
        operand: Box::new(operand),
        type_: crate::types::ParameterType::parse(ty),
        loc: IrSourceLoc::default(),
    }
}

fn bind(name: &str, ty: &str, value: Option<IrValue>, explicit: bool, mutable: bool) -> IrOp {
    IrOp::Bind {
        mutable,
        name: name.to_string(),
        type_: ParameterType::parse(ty),
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
                type_: crate::types::ParameterType::parse(*t),
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
        type_: crate::types::ParameterType::parse(ty),
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
        returns: crate::types::ParameterType::parse(returns),
        body,
        file: "src/main.mfb".to_string(),
        resource_owners: HashMap::new(),
        loc: IrSourceLoc::default(),
    }
}

fn param(name: &str, type_: &str, default: Option<IrValue>) -> IrParam {
    IrParam {
        name: name.to_string(),
        type_: crate::types::ParameterType::parse(type_),
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
                type_: crate::types::ParameterType::parse("Integer"),
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
        type_: crate::types::ParameterType::parse("Integer"),
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
        vec![param("r", "Result OF Integer", None)],
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
        vec![param("r", "Result OF Integer", None)],
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
            Some(unary(UnaryOp::Negate, int_const("1"), "Integer")),
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
            Some(unary(
                UnaryOp::Negate,
                int_const("99999999999999999999"),
                "Integer",
            )),
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
            Some(unary(UnaryOp::Negate, const_of("Float", "1e400"), "Float")),
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
            Some(unary(
                UnaryOp::Negate,
                const_of("Fixed", "3000000000"),
                "Fixed",
            )),
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

// bug-265 / PKG-08: a hand-crafted `.mfp` can carry a Scalar const the parse-time
// range check never saw; `verify_semantics` must reject an out-of-range codepoint
// or a UTF-16 surrogate, mirroring the Byte/Money literal-range checks.
#[test]
fn rejects_scalar_out_of_range() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind(
            "c",
            "Scalar",
            Some(const_of("Scalar", "1114112")),
            true,
            false,
        )],
    );
    expect_rule(&project(vec![f], vec![]), "TYPE_SCALAR_LITERAL_INVALID");
}

#[test]
fn rejects_scalar_surrogate() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind(
            "c",
            "Scalar",
            Some(const_of("Scalar", "55296")),
            true,
            false,
        )],
    );
    expect_rule(&project(vec![f], vec![]), "TYPE_SCALAR_LITERAL_INVALID");
}

#[test]
fn rejects_negated_scalar() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind(
            "c",
            "Scalar",
            Some(unary(UnaryOp::Negate, const_of("Scalar", "65"), "Scalar")),
            true,
            false,
        )],
    );
    expect_rule(&project(vec![f], vec![]), "TYPE_SCALAR_LITERAL_INVALID");
}

#[test]
fn accepts_scalar_in_range() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind(
            "c",
            "Scalar",
            Some(const_of("Scalar", "1114111")),
            true,
            false,
        )],
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
            type_: crate::types::ParameterType::parse("Nothing"),
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

/// bug-301 G2: `allow_sub_call` is a single shared `Cell`, set for a
/// statement-position value and consumed by the first `Call` the walker reaches.
/// Because the walker descends into operands before applying the wrapping node's
/// own rule, a call nested under a non-call node was reached while the flag was
/// still set -- so it was treated as statement position and
/// `TYPE_SUB_HAS_NO_VALUE` never fired. Only a value whose ROOT is the call may be
/// value-less.
#[test]
fn rejects_a_sub_call_nested_under_a_statement_position_expression() {
    let s = sub("doit", vec![], vec![]);
    let sub_call = || IrValue::Call {
        target: "doit".to_string(),
        args: vec![],
        type_: crate::types::ParameterType::parse("Nothing"),
        loc: IrSourceLoc::default(),
    };
    // `Eval(Binary(1, doit()))` -- statement position, but the SUB call is an
    // OPERAND, which is value position.
    let body = vec![IrOp::Eval {
        value: IrValue::Binary {
            op: BinaryOp::Add,
            left: Box::new(int_const("1")),
            right: Box::new(sub_call()),
            type_: crate::types::ParameterType::parse("Integer"),
            loc: IrSourceLoc::default(),
        },
        loc: IrSourceLoc::default(),
    }];
    expect_rule(
        &project(
            vec![s.clone(), func_returns("run", "Nothing", vec![], body)],
            vec![],
        ),
        "TYPE_SUB_HAS_NO_VALUE",
    );

    // A unary operand is the same shape one level shallower.
    let body = vec![IrOp::Eval {
        value: IrValue::Unary {
            op: UnaryOp::Not,
            operand: Box::new(sub_call()),
            type_: crate::types::ParameterType::parse("Boolean"),
            loc: IrSourceLoc::default(),
        },
        loc: IrSourceLoc::default(),
    }];
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
            type_: crate::types::ParameterType::parse("Nothing"),
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

#[test]
fn rejects_exit_program_i128_min_without_panic() {
    // `Unary("-", Const{Integer, i128::MIN})` parses to i128::MIN and the
    // verifier's negation of it must not overflow-panic (debug build); it is
    // out of the 0..255 host range, so it must be reported as such.
    let body = vec![IrOp::ExitProgram {
        code: unary(
            UnaryOp::Negate,
            const_of("Integer", "-170141183460469231731687303715884105728"),
            "Integer",
        ),
        loc: IrSourceLoc::default(),
    }];
    expect_rule(
        &project(vec![func_returns("run", "Nothing", vec![], body)], vec![]),
        "EXIT_PROGRAM_CODE_OUT_OF_RANGE",
    );
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

#[test]
fn rejects_recover_literal_lowered_into_a_byte_slot_out_of_range() {
    // `RECOVER 300` into a `Byte` success type lowers to a `Const Byte "300"`
    // (the literal is coerced, not range-checked) — verify must still reject
    // it as the RECOVER mismatch the source checker reported.
    for (literal, actual) in [("300", "Integer"), ("1.5", "Float")] {
        let body = vec![
            IrOp::Bind {
                mutable: true,
                name: "$trap_val0".to_string(),
                type_: ParameterType::Byte,
                value: None,
                loc: IrSourceLoc::default(),
                explicit_type: false,
            },
            IrOp::Assign {
                name: "$trap_val0".to_string(),
                value: IrValue::Const {
                    type_: ParameterType::Byte,
                    value: literal.to_string(),
                },
                loc: IrSourceLoc::default(),
            },
        ];
        let diagnostics = collect_diagnostics(&project(
            vec![func_returns("run", "Nothing", vec![], body)],
            vec![],
        ));
        let details: Vec<_> = diagnostics
            .iter()
            .map(|d| (d.rule.as_str(), d.detail.as_str()))
            .collect();
        assert_eq!(
            details,
            [(
                "TYPE_RECOVER_TYPE_MISMATCH",
                format!("RECOVER has type {actual}, expected Byte.").as_str()
            )]
        );
    }
}

#[test]
fn rejects_attributed_string_constructor() {
    // `AttributedString[...]` never lowers to anything but a Constructor of the
    // opaque nominal; it is created with `astrings::fromString`.
    let body = vec![IrOp::Bind {
        mutable: false,
        name: "a".to_string(),
        type_: ParameterType::parse("AttributedString"),
        value: Some(IrValue::Constructor {
            type_: ParameterType::parse("AttributedString"),
            args: vec![IrValue::Const {
                type_: ParameterType::String,
                value: "hi".to_string(),
            }],
        }),
        loc: IrSourceLoc::default(),
        explicit_type: true,
    }];
    let diagnostics = collect_diagnostics(&project(
        vec![func_returns("run", "Nothing", vec![], body)],
        vec![],
    ));
    // (The package path's own TYPE_UNKNOWN_VALUE cascade follows; the source
    // path narrows that to operator nodes and leaves it to ir::shape.)
    let first = diagnostics
        .first()
        .map(|d| (d.rule.as_str(), d.detail.as_str()));
    assert_eq!(
        first,
        Some((
            "TYPE_READ_ONLY_RECORD_CONSTRUCTOR",
            "`AttributedString` is an opaque built-in type and cannot be constructed; use `astrings::fromString(text)` to create one."
        ))
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
        type_: ParameterType::Integer,
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
        type_: ParameterType::Integer,
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
        type_: ParameterType::Integer,
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
            type_: ParameterType::Integer,
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
            type_: ParameterType::Integer,
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
                type_: crate::types::ParameterType::parse("List OF Integer"),
                values: vec![int_const("1")],
            }),
            false,
            false,
        ),
        IrOp::ForEach {
            name: "e".to_string(),
            type_: ParameterType::Integer,
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
                type_: crate::types::ParameterType::parse("Color"),
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
        type_: crate::types::ParameterType::parse("Shape"),
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
        type_: crate::types::ParameterType::parse("Color"),
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
        type_: crate::types::ParameterType::parse("Point"),
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
        type_: crate::types::ParameterType::parse("Point"),
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
        type_: crate::types::ParameterType::parse("Point"),
        args: vec![int_const("1"), int_const("2")],
    })];
    let f = func_returns("run", "Point", vec![], body);
    accept(&project(vec![f], vec![record("Point", &["x", "y"])]));
}

#[test]
fn rejects_construct_result_implicit() {
    let body = vec![ret(IrValue::Constructor {
        type_: crate::types::ParameterType::parse("Ok"),
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
            type_: crate::types::ParameterType::parse("Error"),
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
        type_: crate::types::ParameterType::parse("Point"),
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
        type_: crate::types::ParameterType::parse("Point"),
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
        type_: crate::types::ParameterType::parse("Point"),
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
        type_: crate::types::ParameterType::parse("List OF Integer"),
        values: vec![const_of("String", "x")],
    })];
    let f = func_returns("run", "List OF Integer", vec![], body);
    expect_rule(&project(vec![f], vec![]), "TYPE_LIST_ELEMENT_MISMATCH");
}

#[test]
fn accepts_valid_list_literal() {
    let body = vec![ret(IrValue::ListLiteral {
        type_: crate::types::ParameterType::parse("List OF Integer"),
        values: vec![int_const("1"), int_const("2")],
    })];
    let f = func_returns("run", "List OF Integer", vec![], body);
    accept(&project(vec![f], vec![]));
}

#[test]
fn rejects_map_key_mismatch() {
    let body = vec![ret(IrValue::MapLiteral {
        type_: crate::types::ParameterType::parse("Map OF String TO Integer"),
        entries: vec![(int_const("1"), int_const("2"))],
    })];
    let f = func_returns("run", "Map OF String TO Integer", vec![], body);
    expect_rule(&project(vec![f], vec![]), "TYPE_MAP_KEY_MISMATCH");
}

#[test]
fn rejects_map_value_mismatch() {
    let body = vec![ret(IrValue::MapLiteral {
        type_: crate::types::ParameterType::parse("Map OF String TO Integer"),
        entries: vec![(const_of("String", "k"), const_of("String", "v"))],
    })];
    let f = func_returns("run", "Map OF String TO Integer", vec![], body);
    expect_rule(&project(vec![f], vec![]), "TYPE_MAP_VALUE_MISMATCH");
}

#[test]
fn accepts_valid_map_literal() {
    let body = vec![ret(IrValue::MapLiteral {
        type_: crate::types::ParameterType::parse("Map OF String TO Integer"),
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

// --- values.rs literal/money/member/comparability arms (plan-68-D4) --------

fn money_const(v: &str) -> IrValue {
    const_of("Money", v)
}

fn eval(value: IrValue) -> IrOp {
    IrOp::Eval {
        value,
        loc: IrSourceLoc::default(),
    }
}

/// A value nested past `MAX_DEPTH` fails gracefully with `VERIFY_TYPE`
/// (values.rs:26-30).
#[test]
fn deeply_nested_value_hits_depth_cap() {
    let mut v = int_const("1");
    for _ in 0..300 {
        v = unary(UnaryOp::Negate, v, "Integer");
    }
    let f = func_returns("run", "Nothing", vec![], vec![eval(v)]);
    expect_rule(
        &project(vec![f], vec![]),
        "PACKAGE_BINARY_REPRESENTATION_VERIFY_TYPE",
    );
}

/// A `WITH` update whose stamped type is `Unknown` infers the target's type
/// (values.rs:139-142) — here an `Error` local, which is read-only.
#[test]
fn with_update_unknown_type_infers_read_only_target() {
    let body = vec![
        bind("e", "Error", None, false, false),
        ret(IrValue::WithUpdate {
            type_: crate::types::ParameterType::parse("Unknown"),
            target: Box::new(IrValue::Local("e".to_string())),
            updates: vec![],
        }),
    ];
    let f = func_returns("run", "Error", vec![param("e", "Error", None)], body);
    expect_rule(&project(vec![f], vec![]), "TYPE_READ_ONLY_RECORD_UPDATE");
}

/// A `WITH` update of a read-only non-`Error` builtin record (a `MapEntry`)
/// (values.rs:149-153).
#[test]
fn rejects_with_update_read_only_mapentry() {
    let body = vec![ret(IrValue::WithUpdate {
        type_: crate::types::ParameterType::parse("MapEntry OF String TO Integer"),
        target: Box::new(IrValue::Local("e".to_string())),
        updates: vec![],
    })];
    let f = func_returns(
        "run",
        "MapEntry OF String TO Integer",
        vec![param("e", "MapEntry OF String TO Integer", None)],
        body,
    );
    expect_rule(&project(vec![f], vec![]), "TYPE_READ_ONLY_RECORD_UPDATE");
}

/// A `WITH` update naming a field the record does not declare (values.rs:169) and
/// one whose value type cannot be inferred (values.rs:172) are both skipped.
#[test]
fn with_update_unknown_field_and_uninferable_value_are_skipped() {
    let body = vec![ret(IrValue::WithUpdate {
        type_: crate::types::ParameterType::parse("Point"),
        target: Box::new(IrValue::Local("p".to_string())),
        updates: vec![
            crate::ir::IrRecordUpdate {
                field: "ghost".to_string(),
                value: int_const("1"),
            },
            crate::ir::IrRecordUpdate {
                field: "x".to_string(),
                value: IrValue::Local("missing".to_string()),
            },
        ],
    })];
    let f = func_returns("run", "Point", vec![param("p", "Point", None)], body);
    let got = rules(&project(vec![f], vec![record("Point", &["x", "y"])]));
    assert!(
        !got.iter()
            .any(|r| r == "TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH"),
        "{got:?}"
    );
}

/// A `Set OF T` literal whose element type disagrees (values.rs:212-231).
#[test]
fn rejects_set_literal_element_mismatch() {
    let body = vec![ret(IrValue::SetLiteral {
        type_: crate::types::ParameterType::parse("Set OF Integer"),
        values: vec![const_of("String", "x")],
    })];
    let f = func_returns("run", "Set OF Integer", vec![], body);
    expect_rule(&project(vec![f], vec![]), "TYPE_SET_ELEMENT_MISMATCH");
}

/// A valid `Set OF T` literal — the non-emitting fallthrough of the element arm.
#[test]
fn accepts_valid_set_literal() {
    let body = vec![ret(IrValue::SetLiteral {
        type_: crate::types::ParameterType::parse("Set OF Integer"),
        values: vec![int_const("1"), int_const("2")],
    })];
    let f = func_returns("run", "Set OF Integer", vec![], body);
    accept(&project(vec![f], vec![]));
}

/// Money literal with more than five fractional digits (values.rs:373-379).
#[test]
fn rejects_money_literal_precision() {
    let body = vec![bind(
        "m",
        "Money",
        Some(money_const("1.123456")),
        true,
        false,
    )];
    let f = func_returns("run", "Nothing", vec![], body);
    expect_rule(&project(vec![f], vec![]), "TYPE_MONEY_LITERAL_PRECISION");
}

/// Money literal outside the representable range (values.rs:381-383).
#[test]
fn rejects_money_literal_overflow() {
    let body = vec![bind(
        "m",
        "Money",
        Some(money_const("100000000000000")),
        true,
        false,
    )];
    let f = func_returns("run", "Nothing", vec![], body);
    expect_rule(&project(vec![f], vec![]), "TYPE_MONEY_LITERAL_OVERFLOW");
}

/// A valid Money literal — the `Ok(_)` accept arm (values.rs:380).
#[test]
fn accepts_money_literal_in_range() {
    let body = vec![bind("m", "Money", Some(money_const("1.25")), true, false)];
    let f = func_returns("run", "Nothing", vec![], body);
    accept(&project(vec![f], vec![]));
}

/// Negated Money literal with excess precision (values.rs:431-435).
#[test]
fn rejects_negated_money_precision() {
    let body = vec![bind(
        "m",
        "Money",
        Some(unary(UnaryOp::Negate, money_const("1.123456"), "Money")),
        true,
        false,
    )];
    let f = func_returns("run", "Nothing", vec![], body);
    expect_rule(&project(vec![f], vec![]), "TYPE_MONEY_LITERAL_PRECISION");
}

/// Negated Money literal below the representable range (values.rs:438-440).
#[test]
fn rejects_negated_money_underflow() {
    let body = vec![bind(
        "m",
        "Money",
        Some(unary(
            UnaryOp::Negate,
            money_const("100000000000000"),
            "Money",
        )),
        true,
        false,
    )];
    let f = func_returns("run", "Nothing", vec![], body);
    expect_rule(&project(vec![f], vec![]), "TYPE_MONEY_LITERAL_UNDERFLOW");
}

/// A valid negated Money literal — the `Ok(_)` accept arm (values.rs:437).
#[test]
fn accepts_negated_money_literal_in_range() {
    let body = vec![bind(
        "m",
        "Money",
        Some(unary(UnaryOp::Negate, money_const("1.25"), "Money")),
        true,
        false,
    )];
    let f = func_returns("run", "Nothing", vec![], body);
    accept(&project(vec![f], vec![]));
}

/// A `Scalar` literal too large to parse as `u64` (values.rs:360).
#[test]
fn rejects_scalar_literal_overflowing_u64() {
    let body = vec![bind(
        "c",
        "Scalar",
        Some(const_of("Scalar", "99999999999999999999999")),
        true,
        false,
    )];
    let f = func_returns("run", "Nothing", vec![], body);
    expect_rule(&project(vec![f], vec![]), "TYPE_SCALAR_LITERAL_INVALID");
}

/// A finite in-range `Float`/`Fixed` literal — the non-emitting fallthrough of
/// those arms (values.rs:340,350).
#[test]
fn accepts_finite_float_and_fixed_literals() {
    let body = vec![
        bind("f", "Float", Some(const_of("Float", "1.5")), true, false),
        bind("x", "Fixed", Some(const_of("Fixed", "1.5")), true, false),
    ];
    let f = func_returns("run", "Nothing", vec![], body);
    accept(&project(vec![f], vec![]));
}

/// A `MemberAccess` on an enum type name selects a member (values.rs:472-482).
#[test]
fn accepts_enum_member_access() {
    let body = vec![ret(IrValue::MemberAccess {
        target: Box::new(IrValue::Local("Color".to_string())),
        member: "Red".to_string(),
        type_: crate::types::ParameterType::parse("Color"),
    })];
    let f = func_returns("run", "Color", vec![], body);
    let got = rules(&project(
        vec![f],
        vec![enum_type("Color", &["Red", "Green"])],
    ));
    assert!(
        !got.iter().any(|r| r == "TYPE_UNKNOWN_ENUM_MEMBER"),
        "{got:?}"
    );
}

/// A `MemberAccess` whose target type cannot be inferred is skipped
/// (values.rs:485-486).
#[test]
fn member_access_uninferable_target_is_skipped() {
    let body = vec![eval(IrValue::MemberAccess {
        target: Box::new(IrValue::Local("missing".to_string())),
        member: "field".to_string(),
        type_: crate::types::ParameterType::parse("Unknown"),
    })];
    let f = func_returns("run", "Nothing", vec![], body);
    let got = rules(&project(vec![f], vec![]));
    assert!(
        !got.iter().any(|r| r == "TYPE_FIELD_ACCESS_REQUIRES_RECORD"),
        "{got:?}"
    );
}

/// Reading `.state` off a resource that declares none (values.rs:505-517).
#[test]
fn rejects_read_state_on_stateless_resource() {
    let body = vec![eval(IrValue::MemberAccess {
        target: Box::new(IrValue::Local("h".to_string())),
        member: "state".to_string(),
        type_: crate::types::ParameterType::parse("Unknown"),
    })];
    let f = func_returns("run", "Nothing", vec![param("h", "fs.File", None)], body);
    expect_rule(&project(vec![f], vec![]), "TYPE_STATE_INVALID");
}

/// A comparison between a Money and a non-Money operand (values.rs:601-602,
/// 684-692).
#[test]
fn rejects_money_compared_with_non_money() {
    let body = vec![eval(binary(
        BinaryOp::Less,
        money_const("1.00"),
        int_const("2"),
        "Boolean",
    ))];
    let f = func_returns("run", "Nothing", vec![], body);
    expect_rule(&project(vec![f], vec![]), "TYPE_MONEY_OPERATION_INVALID");
}

/// `Money = Money` is accepted — the comparison return arm (values.rs:694).
#[test]
fn accepts_money_equality() {
    let body = vec![eval(binary(
        BinaryOp::Equal,
        money_const("1.00"),
        money_const("2.00"),
        "Boolean",
    ))];
    let f = func_returns("run", "Nothing", vec![], body);
    let got = rules(&project(vec![f], vec![]));
    assert!(
        !got.iter().any(|r| r == "TYPE_MONEY_OPERATION_INVALID"),
        "{got:?}"
    );
}

/// `Money + non-Money` is invalid (values.rs:701-703).
#[test]
fn rejects_money_plus_non_money() {
    let body = vec![eval(binary(
        BinaryOp::Add,
        money_const("1.00"),
        int_const("2"),
        "Money",
    ))];
    let f = func_returns("run", "Nothing", vec![], body);
    expect_rule(&project(vec![f], vec![]), "TYPE_MONEY_OPERATION_INVALID");
}

/// `Money + Money` and `Money * scalar` are accepted (values.rs:696-697).
#[test]
fn accepts_money_add_and_scale() {
    let add = eval(binary(
        BinaryOp::Add,
        money_const("1.00"),
        money_const("2.00"),
        "Money",
    ));
    let scale = eval(binary(
        BinaryOp::Multiply,
        money_const("1.00"),
        int_const("3"),
        "Money",
    ));
    let f = func_returns("run", "Nothing", vec![], vec![add, scale]);
    let got = rules(&project(vec![f], vec![]));
    assert!(
        !got.iter().any(|r| r == "TYPE_MONEY_OPERATION_INVALID"),
        "{got:?}"
    );
}

/// `Money * Money` is invalid — money² is not Money (values.rs:704).
#[test]
fn rejects_money_times_money() {
    let body = vec![eval(binary(
        BinaryOp::Multiply,
        money_const("1.00"),
        money_const("2.00"),
        "Money",
    ))];
    let f = func_returns("run", "Nothing", vec![], body);
    expect_rule(&project(vec![f], vec![]), "TYPE_MONEY_OPERATION_INVALID");
}

/// `non-Money / Money` is invalid (values.rs:705-707).
#[test]
fn rejects_non_money_divided_by_money() {
    let body = vec![eval(binary(
        BinaryOp::Divide,
        int_const("6"),
        money_const("2.00"),
        "Money",
    ))];
    let f = func_returns("run", "Nothing", vec![], body);
    expect_rule(&project(vec![f], vec![]), "TYPE_MONEY_OPERATION_INVALID");
}

/// `Money ^ x` is invalid (values.rs:708).
#[test]
fn rejects_money_exponentiation() {
    let body = vec![eval(binary(
        BinaryOp::Power,
        money_const("1.00"),
        int_const("2"),
        "Money",
    ))];
    let f = func_returns("run", "Nothing", vec![], body);
    expect_rule(&project(vec![f], vec![]), "TYPE_MONEY_OPERATION_INVALID");
}

/// A `Set OF <union>` whose element is not comparable (values.rs:802).
#[test]
fn rejects_set_of_union_element_not_comparable() {
    let f = func_returns("run", "Nothing", vec![param("s", "Set OF U", None)], vec![]);
    // A data-only union (no resource variants) is not comparable.
    expect_rule(
        &project(
            vec![f],
            vec![
                union("U", &["A", "B"]),
                record("A", &["x"]),
                record("B", &["y"]),
            ],
        ),
        "TYPE_REQUIRES_COMPARABLE",
    );
}

/// A `Set OF <enum>` is comparable — the enum accept arm (values.rs:805).
#[test]
fn accepts_set_of_enum_element() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![param("s", "Set OF Color", None)],
        vec![],
    );
    let got = rules(&project(
        vec![f],
        vec![enum_type("Color", &["Red", "Green"])],
    ));
    assert!(
        !got.iter().any(|r| r == "TYPE_REQUIRES_COMPARABLE"),
        "{got:?}"
    );
}

/// A `Set OF <unknown type>` is permissively comparable (values.rs:818).
#[test]
fn accepts_set_of_unknown_type_element() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![param("s", "Set OF Widget", None)],
        vec![],
    );
    let got = rules(&project(vec![f], vec![]));
    assert!(
        !got.iter().any(|r| r == "TYPE_REQUIRES_COMPARABLE"),
        "{got:?}"
    );
}

// --- compat.rs + mod.rs deep arms (plan-68-D6) -----------------------------

/// A resource that floats into a collection declared *after* it cannot take
/// ownership — `TYPE_RESOURCE_RETURN_ORDER` (mod.rs:378-385).
#[test]
fn rejects_resource_return_order() {
    let mut f = func_returns("run", "Nothing", vec![], vec![]);
    f.resource_owners.insert(
        "h".to_string(),
        crate::ir::resource_escape::ResOwner::FloatBlocked("coll".to_string()),
    );
    expect_rule(&project(vec![f], vec![]), "TYPE_RESOURCE_RETURN_ORDER");
}

/// A resource whose ownership floats into a collection is marked non-owning
/// (mod.rs:365-367); no return-order fault fires for a plain `Float`.
#[test]
fn float_owner_is_non_owning() {
    let mut f = func_returns("run", "Nothing", vec![], vec![]);
    f.resource_owners.insert(
        "h".to_string(),
        crate::ir::resource_escape::ResOwner::Float("coll".to_string()),
    );
    let got = rules(&project(vec![f], vec![]));
    assert!(
        !got.iter().any(|r| r == "TYPE_RESOURCE_RETURN_ORDER"),
        "{got:?}"
    );
}

/// A well-typed `os` built-in call drives the `os` overload probe
/// (compat.rs:315-317).
#[test]
fn os_call_with_valid_args_is_accepted() {
    let body = vec![eval(IrValue::Call {
        target: "os.getEnv".to_string(),
        args: vec![const_of("String", "HOME")],
        type_: crate::types::ParameterType::parse("Result OF String"),
        loc: IrSourceLoc::default(),
    })];
    let f = func_returns("run", "Nothing", vec![], body);
    let got = rules(&project(vec![f], vec![]));
    assert!(
        !got.iter().any(|r| r == "TYPE_CALL_ARGUMENT_MISMATCH"),
        "{got:?}"
    );
}

/// `collections.find` with too many arguments — a ranged-arity built-in error
/// (compat.rs:130-140, the `min != max` message).
#[test]
fn rejects_collections_find_wrong_arity() {
    let body = vec![eval(IrValue::Call {
        target: "collections.find".to_string(),
        args: vec![
            IrValue::Local("xs".to_string()),
            int_const("1"),
            int_const("2"),
            int_const("3"),
        ],
        type_: crate::types::ParameterType::parse("Integer"),
        loc: IrSourceLoc::default(),
    })];
    let f = func_returns(
        "run",
        "Nothing",
        vec![param("xs", "List OF Integer", None)],
        body,
    );
    expect_rule(&project(vec![f], vec![]), "TYPE_CALL_ARITY_MISMATCH");
}

/// `collections.contains` on a comparable-element list — the non-emitting
/// fallthrough of the comparability arm (compat.rs:178-189).
#[test]
fn accepts_collections_contains_comparable() {
    let body = vec![eval(IrValue::Call {
        target: "collections.contains".to_string(),
        args: vec![IrValue::Local("xs".to_string()), int_const("1")],
        type_: crate::types::ParameterType::parse("Boolean"),
        loc: IrSourceLoc::default(),
    })];
    let f = func_returns(
        "run",
        "Nothing",
        vec![param("xs", "List OF Integer", None)],
        body,
    );
    let got = rules(&project(vec![f], vec![]));
    assert!(
        !got.iter().any(|r| r == "TYPE_REQUIRES_COMPARABLE"),
        "{got:?}"
    );
}

/// A `Result OF` binding whose inner element type disagrees drives the
/// `compatible` `Result OF` recursion (compat.rs:354-358).
#[test]
fn rejects_result_of_inner_mismatch() {
    let body = vec![bind(
        "r",
        "Result OF Integer",
        Some(IrValue::Local("b".to_string())),
        true,
        false,
    )];
    let f = func_returns(
        "run",
        "Nothing",
        vec![param("b", "Result OF Byte", None)],
        body,
    );
    expect_rule(&project(vec![f], vec![]), "TYPE_BINDING_MISMATCH");
}

/// A `UnionExtract` naming a type that is not a variant of the value's union
/// (compat.rs:664-671).
#[test]
fn rejects_union_extract_foreign_variant() {
    let body = vec![eval(IrValue::UnionExtract {
        type_: crate::types::ParameterType::parse("Ghost"),
        value: Box::new(IrValue::Local("u".to_string())),
    })];
    let f = func_returns("run", "Nothing", vec![param("u", "U", None)], body);
    expect_rule(
        &project(vec![f], vec![union("U", &["A", "B"])]),
        "PACKAGE_BINARY_REPRESENTATION_VERIFY_TYPE",
    );
}

/// An equality comparison where one operand's type is `Unknown` is permissive —
/// `compatible` Unknown short-circuit (compat.rs:340-341).
#[test]
fn equality_with_unknown_operand_is_permissive() {
    let mystery = IrValue::Call {
        target: "mystery".to_string(),
        args: vec![],
        type_: crate::types::ParameterType::parse("Unknown"),
        loc: IrSourceLoc::default(),
    };
    let body = vec![eval(binary(
        BinaryOp::Equal,
        const_of("String", "x"),
        mystery,
        "Boolean",
    ))];
    let f = func_returns("run", "Nothing", vec![], body);
    let got = rules(&project(vec![f], vec![]));
    assert!(
        !got.iter().any(|r| r == "TYPE_BINARY_OPERATOR_MISMATCH"),
        "{got:?}"
    );
}

// --- Set element comparability (plan-63) ------------------------------------

#[test]
fn accepts_set_of_comparable_element() {
    // `Set OF Integer` passes and returns cleanly — a comparable element.
    let f = func_returns(
        "run",
        "Set OF Integer",
        vec![param("s", "Set OF Integer", None)],
        vec![IrOp::Return {
            value: Some(IrValue::Local("s".to_string())),
            loc: IrSourceLoc::default(),
        }],
    );
    accept(&project(vec![f], vec![]));
}

#[test]
fn rejects_set_of_resource_element() {
    // `Set OF fs.File`: a resource handle is not comparable and can't be owned by an
    // ordinary collection.
    let f = func_returns(
        "run",
        "Nothing",
        vec![param("s", "Set OF fs.File", None)],
        vec![],
    );
    expect_rule(&project(vec![f], vec![]), "TYPE_REQUIRES_COMPARABLE");
}

#[test]
fn rejects_set_of_function_element() {
    // `Set OF FUNC() AS Integer`: a function value is not comparable.
    let f = func_returns(
        "run",
        "Nothing",
        vec![param("s", "Set OF FUNC() AS Integer", None)],
        vec![],
    );
    expect_rule(&project(vec![f], vec![]), "TYPE_REQUIRES_COMPARABLE");
}

#[test]
fn rejects_set_of_collection_element() {
    // `Set OF List OF Integer`: a nested collection is not comparable.
    let f = func_returns(
        "run",
        "Nothing",
        vec![param("s", "Set OF List OF Integer", None)],
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
        type_: crate::types::ParameterType::parse("Color"),
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
        type_: crate::types::ParameterType::parse("Color"),
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
            type_: crate::types::ParameterType::parse("Unknown"),
        }),
        member: "line".to_string(),
        type_: crate::types::ParameterType::parse("Unknown"),
    })];
    let f = func_returns("run", "Integer", vec![param("err", "Error", None)], body);
    accept(&project(vec![f], vec![]));
}

// --- type declarations -----------------------------------------------------

/// plan-114-D: a **bare** resource field is still rejected — but by the marker
/// rule a collection element takes, not by the retired ban. This is the test §3
/// calls non-optional: if the field walk stopped emitting
/// `TYPE_RESOURCE_FIELD_FORBIDDEN` without routing to
/// `TYPE_RESOURCE_REQUIRES_RES`, an unmarked resource field would be silently
/// accepted, and letter C's analysis would not float it — a leak with no
/// diagnostic.
#[test]
fn rejects_unmarked_resource_field_in_record() {
    let mut ty = record_typed("Holder", &[("f", "fs.File")]);
    ty.file = "src/main.mfb".to_string();
    let f = func_returns("run", "Nothing", vec![], vec![]);
    let got = rules(&project(vec![f], vec![ty]));
    assert!(
        got.iter().any(|r| r == "TYPE_RESOURCE_REQUIRES_RES"),
        "a bare resource field must be told to add the RES marker: {got:?}"
    );
    assert!(
        !got.iter().any(|r| r == "TYPE_RESOURCE_FIELD_FORBIDDEN"),
        "the ban is retired and must never be emitted again: {got:?}"
    );
}

/// The other half of the marker axis: `RES` on a non-resource field.
#[test]
fn rejects_res_marker_on_a_non_resource_field() {
    let mut ty = record_typed("Holder", &[("n", "RES Integer")]);
    ty.file = "src/main.mfb".to_string();
    let f = func_returns("run", "Nothing", vec![], vec![]);
    expect_rule(&project(vec![f], vec![ty]), "TYPE_RES_REQUIRES_RESOURCE");
}

/// And the accept path this letter exists for: a correctly-marked resource field
/// is legal. Guards against the routing over-rejecting.
#[test]
fn accepts_a_res_marked_resource_field_in_record() {
    let mut ty = record_typed("Holder", &[("name", "String"), ("handle", "RES fs.File")]);
    ty.file = "src/main.mfb".to_string();
    let f = func_returns("run", "Nothing", vec![], vec![]);
    let got = rules(&project(vec![f], vec![ty]));
    for rule in [
        "TYPE_RESOURCE_FIELD_FORBIDDEN",
        "TYPE_RESOURCE_REQUIRES_RES",
        "TYPE_RES_REQUIRES_RESOURCE",
    ] {
        assert!(
            !got.iter().any(|r| r == rule),
            "`RES fs.File` is a legal field; {rule} must not fire: {got:?}"
        );
    }
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
    let u = union("Mixed", &["fs.File", "Circle"]);
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
            type_: crate::types::ParameterType::parse("Nothing"),
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
            type_: crate::types::ParameterType::parse("Nothing"),
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
        type_: crate::types::ParameterType::parse("Float"),
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
            type_: crate::types::ParameterType::parse("Unknown"),
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
        type_: crate::types::ParameterType::parse("Float"),
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
        type_: crate::types::ParameterType::parse("Float"),
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
        "fs.File",
        Some(IrValue::Local("g".to_string())),
        true,
        false,
    )];
    let f = func_returns("run", "Nothing", vec![param("g", "fs.File", None)], body);
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
        .insert("x".to_string(), crate::ir::resource_escape::ResOwner::Local);
    expect_rule(&project(vec![f], vec![]), "TYPE_RES_REQUIRES_RESOURCE");
}

#[test]
fn rejects_collection_resource_element_without_res() {
    // A List OF fs.File (bare resource, not RES-marked).
    let f = func_returns(
        "run",
        "Nothing",
        vec![param("xs", "List OF fs.File", None)],
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
        vec![param("xs", "List OF RES fs.File", None)],
        vec![],
    );
    accept(&project(vec![f], vec![]));
}

#[test]
fn accepts_state_on_union() {
    // plan-74: a resource union may carry a uniform (defaultable) STATE at a
    // binding — the former TYPE_UNION_STATE_FORBIDDEN ban is retired.
    let mut f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind("r", "Res STATE Integer", None, true, false)],
    );
    // a resource union type "Res" with a File variant so is_resource is true and unions contains it.
    f.resource_owners
        .insert("r".to_string(), crate::ir::resource_escape::ResOwner::Local);
    let u = union("Res", &["fs.File"]);
    let got = rules(&project(vec![f], vec![u]));
    assert!(
        !got.iter().any(|r| r == "TYPE_UNION_STATE_FORBIDDEN"),
        "union STATE must be accepted (ban retired): {got:?}"
    );
}

#[test]
fn rejects_state_type_not_defaultable() {
    // A File resource with STATE of a union type (not defaultable).
    let mut f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind("h", "fs.File STATE Shape", None, true, false)],
    );
    f.resource_owners
        .insert("h".to_string(), crate::ir::resource_escape::ResOwner::Local);
    expect_rule(
        &project(vec![f], vec![union("Shape", &["A", "B"])]),
        "TYPE_STATE_INVALID",
    );
}

#[test]
fn rejects_state_assign_no_state() {
    // Assign state on a File binding declared without STATE.
    let body = vec![
        bind("h", "fs.File", None, true, false),
        IrOp::StateAssign {
            resource: "h".to_string(),
            value: int_const("1"),
            loc: IrSourceLoc::default(),
        },
    ];
    let mut f = func_returns("run", "Nothing", vec![], body);
    f.resource_owners
        .insert("h".to_string(), crate::ir::resource_escape::ResOwner::Local);
    expect_rule(&project(vec![f], vec![]), "TYPE_STATE_INVALID");
}

#[test]
fn rejects_state_assign_mismatch() {
    let body = vec![
        bind("h", "fs.File STATE Integer", None, true, false),
        IrOp::StateAssign {
            resource: "h".to_string(),
            value: const_of("String", "x"),
            loc: IrSourceLoc::default(),
        },
    ];
    let mut f = func_returns("run", "Nothing", vec![], body);
    f.resource_owners
        .insert("h".to_string(), crate::ir::resource_escape::ResOwner::Local);
    expect_rule(&project(vec![f], vec![]), "TYPE_ASSIGNMENT_MISMATCH");
}

// --- use after move --------------------------------------------------------

#[test]
fn rejects_use_after_close() {
    // fs.close(h) then read h again.
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
        IrOp::Eval {
            value: IrValue::Call {
                target: "fs.close".to_string(),
                args: vec![IrValue::Local("h".to_string())],
                type_: crate::types::ParameterType::parse("Nothing"),
                loc: IrSourceLoc::default(),
            },
            loc: IrSourceLoc::default(),
        },
    ];
    let mut f = func_returns("run", "Nothing", vec![], body);
    f.resource_owners
        .insert("h".to_string(), crate::ir::resource_escape::ResOwner::Local);
    expect_rule(&project(vec![f], vec![]), "TYPE_USE_AFTER_MOVE");
}

#[test]
fn accepts_close_of_a_res_parameter() {
    // plan-59-E: was `rejects_non_owner_resource_close`. A `RES` parameter used to
    // be a non-owning pointer that could not be closed
    // (`TYPE_RESOURCE_INVALIDATE_NOT_OWNER`, retired). Under scope ownership any
    // holder may close, which is exactly what makes `closeSound(RES sound AS
    // SoundFile)` — "take a handle, give it back" — writable at all.
    //
    // Converted rather than deleted: what it protected (that this shape is
    // *decided*, not accidental) is still worth asserting, with the opposite
    // verdict.
    let body = vec![IrOp::Eval {
        value: IrValue::Call {
            target: "fs.close".to_string(),
            args: vec![IrValue::Local("h".to_string())],
            type_: crate::types::ParameterType::parse("Nothing"),
            loc: IrSourceLoc::default(),
        },
        loc: IrSourceLoc::default(),
    }];
    let f = func_returns("run", "Nothing", vec![param("h", "fs.File", None)], body);
    accept(&project(vec![f], vec![]));
}

// --- resources.rs alias/move + defaultability arms (plan-68-D5) ------------

fn fs_close(res: &str) -> IrOp {
    eval(IrValue::Call {
        target: "fs.close".to_string(),
        args: vec![IrValue::Local(res.to_string())],
        type_: crate::types::ParameterType::parse("Nothing"),
        loc: IrSourceLoc::default(),
    })
}

/// A call that returns a `RES fs.File` produced from an argument resource.
fn grab(src: &str) -> IrValue {
    IrValue::Call {
        target: "grab".to_string(),
        args: vec![IrValue::Local(src.to_string())],
        type_: crate::types::ParameterType::parse("fs.File"),
        loc: IrSourceLoc::default(),
    }
}

/// `RES b = grab(a)` records `a`/`b` as possible aliases (resources.rs:196-222);
/// closing one then reading the other is a use-after-move via the alias closure
/// (resources.rs:39,42,136-139).
#[test]
fn rejects_use_after_close_through_alias() {
    let body = vec![
        bind("b", "fs.File", Some(grab("a")), true, false),
        fs_close("b"),
        fs_close("a"),
    ];
    let mut f = func_returns("run", "Nothing", vec![param("a", "fs.File", None)], body);
    f.resource_owners
        .insert("b".to_string(), crate::ir::resource_escape::ResOwner::Local);
    expect_rule(&project(vec![f], vec![]), "TYPE_USE_AFTER_MOVE");
}

/// An alias established before an `IF` survives the branch merge
/// (resources.rs:89-98), so a later close-through still flags the read.
#[test]
fn alias_survives_branch_merge() {
    let body = vec![
        bind("b", "fs.File", Some(grab("a")), true, false),
        IrOp::If {
            condition: const_of("Boolean", "true"),
            then_body: vec![],
            else_body: vec![],
            loc: IrSourceLoc::default(),
        },
        fs_close("b"),
        fs_close("a"),
    ];
    let mut f = func_returns("run", "Nothing", vec![param("a", "fs.File", None)], body);
    f.resource_owners
        .insert("b".to_string(), crate::ir::resource_escape::ResOwner::Local);
    expect_rule(&project(vec![f], vec![]), "TYPE_USE_AFTER_MOVE");
}

/// Rebinding a name that is an alias target severs the relation
/// (resources.rs:167-173): after the rebind, `a` is a fresh resource and closing
/// `b` does not move it.
#[test]
fn rebind_severs_alias() {
    let fresh = IrValue::Call {
        target: "fresh".to_string(),
        args: vec![],
        type_: crate::types::ParameterType::parse("fs.File"),
        loc: IrSourceLoc::default(),
    };
    let body = vec![
        bind("b", "fs.File", Some(grab("a")), true, false),
        bind("a", "fs.File", Some(fresh), true, false),
        fs_close("b"),
        fs_close("a"),
    ];
    let mut f = func_returns("run", "Nothing", vec![param("a", "fs.File", None)], body);
    f.resource_owners
        .insert("b".to_string(), crate::ir::resource_escape::ResOwner::Local);
    f.resource_owners
        .insert("a".to_string(), crate::ir::resource_escape::ResOwner::Local);
    let got = rules(&project(vec![f], vec![]));
    assert!(!got.iter().any(|r| r == "TYPE_USE_AFTER_MOVE"), "{got:?}");
}

/// Closing an outer resource inside a `FOR EACH` body moves it past the loop
/// (resources.rs:255-258).
#[test]
fn foreach_body_move_leaks_to_outer() {
    let body = vec![
        IrOp::ForEach {
            name: "x".to_string(),
            type_: ParameterType::parse("fs.File"),
            iterable: IrValue::Local("items".to_string()),
            body: vec![fs_close("a")],
            loc: IrSourceLoc::default(),
        },
        fs_close("a"),
    ];
    let f = func_returns(
        "run",
        "Nothing",
        vec![
            param("a", "fs.File", None),
            param("items", "List OF fs.File", None),
        ],
        body,
    );
    expect_rule(&project(vec![f], vec![]), "TYPE_USE_AFTER_MOVE");
}

/// A self-referential record STATE type is not defaultable — the cycle guard of
/// `is_defaultable` (resources.rs:326-327).
#[test]
fn rejects_state_of_cyclic_record() {
    let body = vec![bind("h", "fs.File STATE R", None, true, false)];
    let mut f = func_returns("run", "Nothing", vec![], body);
    f.resource_owners
        .insert("h".to_string(), crate::ir::resource_escape::ResOwner::Local);
    expect_rule(
        &project(vec![f], vec![record_typed("R", &[("self", "R")])]),
        "TYPE_STATE_INVALID",
    );
}

/// A value-returning FUNC whose body is only a `TRAP` handler never always-returns
/// — the `Trap` arm of `block_always_returns` (resources.rs:405).
#[test]
fn trap_only_body_does_not_always_return() {
    let body = vec![IrOp::Trap {
        name: "e".to_string(),
        body: vec![ret(int_const("1"))],
        loc: IrSourceLoc::default(),
    }];
    let f = func_returns("run", "Integer", vec![], body);
    expect_rule(&project(vec![f], vec![]), "TYPE_FUNC_MISSING_RETURN");
}

/// A `MATCH` whose scrutinee type cannot be inferred is not exhaustive —
/// `match_covers_all` infer-None arm (resources.rs:423-424).
#[test]
fn match_on_uninferable_scrutinee_not_exhaustive() {
    let m = IrOp::Match {
        value: IrValue::Local("missing".to_string()),
        cases: vec![union_variant_case("X", vec![ret(int_const("1"))])],
        loc: IrSourceLoc::default(),
    };
    let f = func_returns("run", "Integer", vec![], vec![m]);
    expect_rule(&project(vec![f], vec![]), "TYPE_FUNC_MISSING_RETURN");
}

/// A `MATCH` on a non-union/non-enum scrutinee is not exhaustive —
/// `match_covers_all` else arm (resources.rs:431-432).
#[test]
fn match_on_primitive_scrutinee_not_exhaustive() {
    let m = IrOp::Match {
        value: int_const("1"),
        cases: vec![union_variant_case("X", vec![ret(int_const("1"))])],
        loc: IrSourceLoc::default(),
    };
    let f = func_returns("run", "Integer", vec![], vec![m]);
    expect_rule(&project(vec![f], vec![]), "TYPE_FUNC_MISSING_RETURN");
}

// --- calls.rs: STATE agreement (plan-68-D2) --------------------------------

/// A resource binding whose owner is recorded (so it is not rejected as a
/// non-`RES` resource hold), typed `type_`, initialized from `value`.
fn res_bind_owned(f: &mut IrFunction, name: &str, type_: &str, value: Option<IrValue>) -> IrOp {
    f.resource_owners.insert(
        name.to_string(),
        crate::ir::resource_escape::ResOwner::Local,
    );
    bind(name, type_, value, true, false)
}

fn transfer_call(handle: &str, res: Option<&str>) -> IrOp {
    let mut args = vec![IrValue::Local(handle.to_string())];
    if let Some(res) = res {
        args.push(IrValue::Local(res.to_string()));
    }
    IrOp::Eval {
        value: IrValue::Call {
            target: crate::codegen::builtins::thread::TRANSFER_RESOURCE.to_string(),
            args,
            type_: crate::types::ParameterType::parse("Nothing"),
            loc: IrSourceLoc::default(),
        },
        loc: IrSourceLoc::default(),
    }
}

#[test]
fn unary_operand_of_uninferable_value_is_skipped() {
    // `check_unary_operand` early-returns when the operand type cannot be
    // inferred (calls.rs:18) — an unknown local read as unary `-`.
    let body = vec![IrOp::Eval {
        value: unary(
            UnaryOp::Negate,
            IrValue::Local("missing".to_string()),
            "Integer",
        ),
        loc: IrSourceLoc::default(),
    }];
    let f = func_returns("run", "Nothing", vec![], body);
    let got = rules(&project(vec![f], vec![]));
    assert!(!got.iter().any(|r| r.starts_with("TYPE_UNARY")), "{got:?}");
}

#[test]
fn call_argument_of_uninferable_value_is_skipped() {
    // `check_call_argument_types` continues past an argument whose type cannot be
    // inferred (calls.rs:121) — an unknown local passed to a known function.
    let callee = func_returns(
        "helper",
        "Nothing",
        vec![param("a", "Integer", None)],
        vec![],
    );
    let body = vec![IrOp::Eval {
        value: IrValue::Call {
            target: "helper".to_string(),
            args: vec![IrValue::Local("missing".to_string())],
            type_: crate::types::ParameterType::parse("Nothing"),
            loc: IrSourceLoc::default(),
        },
        loc: IrSourceLoc::default(),
    }];
    let caller = func_returns("run", "Nothing", vec![], body);
    let got = rules(&project(vec![callee, caller], vec![]));
    assert!(
        !got.iter().any(|r| r == "TYPE_CALL_ARGUMENT_MISMATCH"),
        "{got:?}"
    );
}

#[test]
fn rejects_argument_state_retype() {
    // Callee param declares `STATE Cursor`; argument carries `STATE Label` — a
    // parameter observes a state, it cannot re-type it (calls.rs:252-264).
    let callee = func_returns(
        "helper",
        "Nothing",
        vec![param("h", "fs.File STATE Cursor", None)],
        vec![],
    );
    let body = vec![IrOp::Eval {
        value: IrValue::Call {
            target: "helper".to_string(),
            args: vec![IrValue::Local("g".to_string())],
            type_: crate::types::ParameterType::parse("Nothing"),
            loc: IrSourceLoc::default(),
        },
        loc: IrSourceLoc::default(),
    }];
    let caller = func_returns(
        "run",
        "Nothing",
        vec![param("g", "fs.File STATE Label", None)],
        body,
    );
    expect_rule(
        &project(vec![callee, caller], vec![]),
        "TYPE_STATE_MISMATCH",
    );
}

#[test]
fn rejects_argument_state_missing() {
    // Callee param declares `STATE Cursor`; argument carries no state — a
    // parameter cannot attach one (calls.rs:256-264).
    let callee = func_returns(
        "helper",
        "Nothing",
        vec![param("h", "fs.File STATE Cursor", None)],
        vec![],
    );
    let body = vec![IrOp::Eval {
        value: IrValue::Call {
            target: "helper".to_string(),
            args: vec![IrValue::Local("g".to_string())],
            type_: crate::types::ParameterType::parse("Nothing"),
            loc: IrSourceLoc::default(),
        },
        loc: IrSourceLoc::default(),
    }];
    let caller = func_returns("run", "Nothing", vec![param("g", "fs.File", None)], body);
    expect_rule(
        &project(vec![callee, caller], vec![]),
        "TYPE_STATE_MISMATCH",
    );
}

#[test]
fn accepts_argument_state_agreement() {
    // Matching states — the agreeing arm (calls.rs:249-250); no STATE mismatch.
    let callee = func_returns(
        "helper",
        "Nothing",
        vec![param("h", "fs.File STATE Cursor", None)],
        vec![],
    );
    let body = vec![IrOp::Eval {
        value: IrValue::Call {
            target: "helper".to_string(),
            args: vec![IrValue::Local("g".to_string())],
            type_: crate::types::ParameterType::parse("Nothing"),
            loc: IrSourceLoc::default(),
        },
        loc: IrSourceLoc::default(),
    }];
    let caller = func_returns(
        "run",
        "Nothing",
        vec![param("g", "fs.File STATE Cursor", None)],
        body,
    );
    let got = rules(&project(vec![callee, caller], vec![]));
    assert!(!got.iter().any(|r| r == "TYPE_STATE_MISMATCH"), "{got:?}");
}

#[test]
fn rejects_thread_transfer_state_retype() {
    // Plane declares `STATE Cursor`; transferred resource carries `STATE Label`
    // (calls.rs:187-190).
    let f = func_returns(
        "run",
        "Nothing",
        vec![
            param(
                "t",
                "Thread OF Nothing RES fs.File STATE Cursor TO Nothing",
                None,
            ),
            param("r", "fs.File STATE Label", None),
        ],
        vec![transfer_call("t", Some("r"))],
    );
    expect_rule(&project(vec![f], vec![]), "TYPE_STATE_MISMATCH");
}

#[test]
fn rejects_thread_transfer_state_missing() {
    // Plane declares `STATE Cursor`; transferred resource is bare
    // (calls.rs:191-193).
    let f = func_returns(
        "run",
        "Nothing",
        vec![
            param(
                "t",
                "Thread OF Nothing RES fs.File STATE Cursor TO Nothing",
                None,
            ),
            param("r", "fs.File", None),
        ],
        vec![transfer_call("t", Some("r"))],
    );
    expect_rule(&project(vec![f], vec![]), "TYPE_STATE_MISMATCH");
}

#[test]
fn rejects_thread_transfer_state_on_bare_plane() {
    // Bare plane; transferred resource carries `STATE Cursor` (calls.rs:194-197).
    let f = func_returns(
        "run",
        "Nothing",
        vec![
            param("t", "Thread OF Nothing RES fs.File TO Nothing", None),
            param("r", "fs.File STATE Cursor", None),
        ],
        vec![transfer_call("t", Some("r"))],
    );
    expect_rule(&project(vec![f], vec![]), "TYPE_STATE_MISMATCH");
}

#[test]
fn thread_transfer_with_one_arg_is_skipped() {
    // Fewer than two args — the early return (calls.rs:170).
    let f = func_returns(
        "run",
        "Nothing",
        vec![param("t", "Thread OF Nothing RES fs.File TO Nothing", None)],
        vec![transfer_call("t", None)],
    );
    let got = rules(&project(vec![f], vec![]));
    assert!(!got.iter().any(|r| r == "TYPE_STATE_MISMATCH"), "{got:?}");
}

#[test]
fn thread_transfer_uninferable_args_are_skipped() {
    // Both args unknown locals — infer fails (calls.rs:173-177).
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![transfer_call("missing_handle", Some("missing_res"))],
    );
    let got = rules(&project(vec![f], vec![]));
    assert!(!got.iter().any(|r| r == "TYPE_STATE_MISMATCH"), "{got:?}");
}

#[test]
fn thread_transfer_non_thread_handle_is_skipped() {
    // Handle is not a thread type — no plane resource (calls.rs:179-180).
    let f = func_returns(
        "run",
        "Nothing",
        vec![param("t", "Integer", None), param("r", "fs.File", None)],
        vec![transfer_call("t", Some("r"))],
    );
    let got = rules(&project(vec![f], vec![]));
    assert!(!got.iter().any(|r| r == "TYPE_STATE_MISMATCH"), "{got:?}");
}

#[test]
fn accepts_return_state_union() {
    // plan-74: a FUNC may return a resource union carrying a uniform (defaultable)
    // STATE — the former TYPE_UNION_STATE_FORBIDDEN return ban is retired.
    let f = func_returns("run", "Res STATE Integer", vec![], vec![]);
    let got = rules(&project(vec![f], vec![union("Res", &["fs.File"])]));
    assert!(
        !got.iter().any(|r| r == "TYPE_UNION_STATE_FORBIDDEN"),
        "union STATE return must be accepted (ban retired): {got:?}"
    );
}

#[test]
fn rejects_return_state_not_defaultable() {
    // FUNC return STATE type is a union (not defaultable) (calls.rs:295-303).
    let f = func_returns("run", "fs.File STATE Shape", vec![], vec![]);
    expect_rule(
        &project(vec![f], vec![union("Shape", &["A", "B"])]),
        "TYPE_STATE_INVALID",
    );
}

#[test]
fn rejects_binding_opaque_state_narrowing() {
    // Binding a bare `RES` parameter under a concrete STATE — an unprovable
    // narrowing (calls.rs:369-375).
    let mut f = func_returns("run", "Nothing", vec![param("p", "fs.File", None)], vec![]);
    let b = res_bind_owned(
        &mut f,
        "x",
        "fs.File STATE Integer",
        Some(IrValue::Local("p".to_string())),
    );
    f.body = vec![b];
    expect_rule(&project(vec![f], vec![]), "TYPE_STATE_OPAQUE_NARROWING");
}

#[test]
fn rejects_binding_state_mismatch() {
    // Binding declares `STATE Cursor`; initializer carries `STATE Label`
    // (calls.rs:387-392).
    let mut f = func_returns(
        "run",
        "Nothing",
        vec![param("src", "fs.File STATE Label", None)],
        vec![],
    );
    let b = res_bind_owned(
        &mut f,
        "x",
        "fs.File STATE Cursor",
        Some(IrValue::Local("src".to_string())),
    );
    f.body = vec![b];
    expect_rule(&project(vec![f], vec![]), "TYPE_STATE_MISMATCH");
}

#[test]
fn rejects_bare_binding_of_stateful_initializer() {
    // Bare binding of a stateful initializer (calls.rs:393-395).
    let mut f = func_returns(
        "run",
        "Nothing",
        vec![param("src", "fs.File STATE Label", None)],
        vec![],
    );
    let b = res_bind_owned(
        &mut f,
        "x",
        "fs.File",
        Some(IrValue::Local("src".to_string())),
    );
    f.body = vec![b];
    expect_rule(&project(vec![f], vec![]), "TYPE_STATE_MISMATCH");
}

#[test]
fn accepts_binding_state_agreement() {
    // Binding adopts the state it already carries — the agreeing arm
    // (calls.rs:384-386); no STATE mismatch.
    let mut f = func_returns(
        "run",
        "Nothing",
        vec![param("src", "fs.File STATE Label", None)],
        vec![],
    );
    let b = res_bind_owned(
        &mut f,
        "x",
        "fs.File STATE Label",
        Some(IrValue::Local("src".to_string())),
    );
    f.body = vec![b];
    let got = rules(&project(vec![f], vec![]));
    assert!(!got.iter().any(|r| r == "TYPE_STATE_MISMATCH"), "{got:?}");
}

#[test]
fn binding_state_of_uninferable_initializer_is_skipped() {
    // Initializer type cannot be inferred — the early return (calls.rs:379).
    let mut f = func_returns("run", "Nothing", vec![], vec![]);
    let b = res_bind_owned(
        &mut f,
        "x",
        "fs.File STATE Integer",
        Some(IrValue::Local("missing".to_string())),
    );
    f.body = vec![b];
    let got = rules(&project(vec![f], vec![]));
    assert!(!got.iter().any(|r| r == "TYPE_STATE_MISMATCH"), "{got:?}");
}

// --- link functions --------------------------------------------------------

fn link_fn() -> crate::ir::IrLinkFunction {
    crate::ir::IrLinkFunction {
        alias: "lib".to_string(),
        name: "open".to_string(),
        library: "sqlite3".to_string(),
        symbol: "sqlite3_open".to_string(),
        params: vec![(
            "path".to_string(),
            crate::types::ParameterType::parse("String"),
        )],
        return_type: crate::types::ParameterType::parse("Integer"),
        return_resource: false,
        return_state_type: None,
        abi_slots: vec![crate::ir::IrAbiSlot {
            name: "path".to_string(),
            ctype: crate::types::ParameterType::parse("CString"),
            direction: crate::ir::AbiDirection::In,
        }],
        abi_return_name: "value".to_string(),
        abi_return_ctype: crate::types::ParameterType::parse("CInt32"),
        consts: vec![],
        bind_in: vec![],
        bind_state: None,
        bind_state_resource: None,
        success_on: None,
        // plan-50-H: the result is whatever `RETURN <expr>` names; a bare Var over
        // the ABI return is the `AS value CInt32` + `RETURN value` passthrough.
        result: Some(crate::ir::IrLinkExpr::Var("value".to_string())),
        free: None,
        buffers: vec![],
        result_length: None,
    }
}

#[test]
fn accepts_valid_link_function() {
    let mut p = project(vec![func_returns("run", "Nothing", vec![], vec![])], vec![]);
    p.link_functions = vec![link_fn()];
    accept(&p);
}

/// `IrProject::link_library_names` returns the distinct library names in
/// declaration order, deduplicating a repeat (`ir/types.rs` loop + `contains`
/// guard).
#[test]
fn link_library_names_dedups_in_order() {
    let mut p = project(vec![func_returns("run", "Nothing", vec![], vec![])], vec![]);
    let mut a = link_fn();
    a.library = "a".to_string();
    let mut b = link_fn();
    b.library = "b".to_string();
    let mut a2 = link_fn();
    a2.library = "a".to_string();
    p.link_functions = vec![a, b, a2];
    assert_eq!(
        p.link_library_names(),
        vec!["a".to_string(), "b".to_string()]
    );
}

// --- CSTRUCT (plan-50-B) ---------------------------------------------------

fn cstruct(name: &str, fields: &[(&str, &str)]) -> crate::ir::IrCStruct {
    crate::ir::IrCStruct {
        alias: "lib".to_string(),
        name: name.to_string(),
        maps_to: crate::types::ParameterType::parse("Rec"),
        fields: fields
            .iter()
            .map(|(n, t)| crate::ir::IrCStructField {
                name: (*n).to_string(),
                ctype: crate::types::ParameterType::parse(t),
            })
            .collect(),
    }
}

fn project_with_cstructs(structs: Vec<crate::ir::IrCStruct>) -> IrProject {
    let mut p = project(vec![func_returns("run", "Nothing", vec![], vec![])], vec![]);
    p.link_cstructs = structs;
    p
}

#[test]
fn accepts_valid_cstruct() {
    accept(&project_with_cstructs(vec![cstruct(
        "SfFormatInfo",
        &[("format", "CInt32"), ("name", "CString")],
    )]));
}

#[test]
fn rejects_cstruct_with_no_fields() {
    expect_rule(
        &project_with_cstructs(vec![cstruct("Empty", &[])]),
        "NATIVE_CSTRUCT_INVALID",
    );
}

#[test]
fn rejects_cstruct_duplicate_field() {
    expect_rule(
        &project_with_cstructs(vec![cstruct("Dup", &[("a", "CInt32"), ("a", "CInt32")])]),
        "NATIVE_CSTRUCT_INVALID",
    );
}

#[test]
fn rejects_cstruct_unknown_field_ctype() {
    expect_rule(
        &project_with_cstructs(vec![cstruct("Bad", &[("a", "CSize")])]),
        "NATIVE_ABI_UNKNOWN_CTYPE",
    );
}

/// `CVoid` has no storage, so it cannot be a struct field.
#[test]
fn rejects_cstruct_cvoid_field() {
    expect_rule(
        &project_with_cstructs(vec![cstruct("Bad", &[("a", "CVoid")])]),
        "NATIVE_CSTRUCT_INVALID",
    );
}

/// Nesting is unsupported; reject it by name rather than letting it read as an
/// unknown ctype, which would misdescribe the cause.
#[test]
fn rejects_nested_cstruct() {
    expect_rule(
        &project_with_cstructs(vec![
            cstruct("Inner", &[("a", "CInt32")]),
            cstruct("Outer", &[("inner", "Inner")]),
        ]),
        "NATIVE_CSTRUCT_INVALID",
    );
}

#[test]
fn rejects_duplicate_cstruct_name_in_one_alias() {
    expect_rule(
        &project_with_cstructs(vec![
            cstruct("Same", &[("a", "CInt32")]),
            cstruct("Same", &[("b", "CInt32")]),
        ]),
        "NATIVE_CSTRUCT_INVALID",
    );
}

/// The size cap is what keeps a crafted `.mfp` from turning the thunk's stack
/// frame into an overflow primitive.
#[test]
fn rejects_oversized_cstruct() {
    let fields: Vec<(&str, &str)> = (0..200).map(|_| ("f", "CInt64")).collect();
    // 200 * 8 = 1600 bytes, over the 1024 cap. Duplicate field names would also
    // fault, so give each a distinct name.
    let mut decl = cstruct("Huge", &fields);
    for (i, field) in decl.fields.iter_mut().enumerate() {
        field.name = format!("f{i}");
    }
    expect_rule(
        &project_with_cstructs(vec![decl]),
        "NATIVE_CSTRUCT_TOO_LARGE",
    );
}

/// A crafted package never ran the resolver, so this is the only gate keeping a
/// private C layout out of a public wrapper signature.
#[test]
fn rejects_cstruct_escape_into_wrapper_signature() {
    let mut lf = link_fn();
    lf.params = vec![(
        "info".to_string(),
        crate::types::ParameterType::parse("SfInfo"),
    )];
    lf.abi_slots = vec![crate::ir::IrAbiSlot {
        name: "info".to_string(),
        ctype: crate::types::ParameterType::parse("CInt32"),
        direction: crate::ir::AbiDirection::In,
    }];
    let mut p = project_with_cstructs(vec![cstruct("SfInfo", &[("a", "CInt32")])]);
    p.link_functions = vec![lf];
    expect_rule(&p, "NATIVE_CSTRUCT_ESCAPE");
}

// --- link.rs native arms (plan-68-D3) --------------------------------------

fn abi_slot(name: &str, ctype: &str, dir: crate::ir::AbiDirection) -> crate::ir::IrAbiSlot {
    crate::ir::IrAbiSlot {
        name: name.to_string(),
        ctype: crate::types::ParameterType::parse(ctype),
        direction: dir,
    }
}

fn project_with_link(
    lf: crate::ir::IrLinkFunction,
    cstructs: Vec<crate::ir::IrCStruct>,
) -> IrProject {
    let mut p = project(vec![func_returns("run", "Nothing", vec![], vec![])], vec![]);
    p.link_functions = vec![lf];
    p.link_cstructs = cstructs;
    p
}

/// A struct slot whose CSTRUCT `maps_to` names a type that is not a record
/// (link.rs:74-81).
#[test]
fn rejects_cstruct_maps_to_non_record() {
    let mut lf = link_fn();
    lf.params = vec![];
    lf.abi_slots = vec![abi_slot("cfg", "Cfg", crate::ir::AbiDirection::In)];
    lf.result = None;
    lf.return_type = crate::types::ParameterType::parse("Nothing");
    // cstruct maps_to defaults to "Rec"; no "Rec" type is declared.
    let p = project_with_link(lf, vec![cstruct("Cfg", &[("a", "CInt32")])]);
    expect_rule(&p, "NATIVE_STRUCT_FIELD_MISMATCH");
}

/// A native function whose RETURN type names a sibling CSTRUCT (link.rs:191-197).
#[test]
fn rejects_cstruct_escape_in_return_type() {
    let mut lf = link_fn();
    lf.return_type = crate::types::ParameterType::parse("Cfg");
    let p = project_with_link(lf, vec![cstruct("Cfg", &[("a", "CInt32")])]);
    expect_rule(&p, "NATIVE_CSTRUCT_ESCAPE");
}

/// BIND IN naming a nonexistent ABI slot (link.rs:105-113).
#[test]
fn rejects_bind_in_unknown_slot() {
    let mut lf = link_fn();
    lf.bind_in = vec![crate::ir::IrBindIn {
        slot: "nope".to_string(),
        fields: vec![],
    }];
    let p = project_with_link(lf, vec![]);
    expect_rule(&p, "NATIVE_BIND_IN_INVALID");
}

/// BIND IN naming a slot that is not a CSTRUCT (link.rs:115-127); also exercises
/// the "IN slot satisfied by BIND IN" continue (link.rs:326-327).
#[test]
fn rejects_bind_in_slot_not_cstruct() {
    let mut lf = link_fn();
    // Default slot `path` is a scalar CString, not a CSTRUCT.
    lf.bind_in = vec![crate::ir::IrBindIn {
        slot: "path".to_string(),
        fields: vec![],
    }];
    let p = project_with_link(lf, vec![]);
    expect_rule(&p, "NATIVE_BIND_IN_INVALID");
}

/// BIND IN setting a field the CSTRUCT does not declare (link.rs:129-138).
#[test]
fn rejects_bind_in_unknown_field() {
    let mut lf = link_fn();
    lf.params = vec![(
        "p".to_string(),
        crate::types::ParameterType::parse("Integer"),
    )];
    lf.abi_slots = vec![abi_slot("cfg", "Cfg", crate::ir::AbiDirection::In)];
    lf.bind_in = vec![crate::ir::IrBindIn {
        slot: "cfg".to_string(),
        fields: vec![crate::ir::IrBindInField {
            name: "ghost".to_string(),
            param: Some("p".to_string()),
            literal: None,
        }],
    }];
    lf.result = None;
    lf.return_type = crate::types::ParameterType::parse("Nothing");
    let p = project_with_link(lf, vec![cstruct("Cfg", &[("a", "CInt32")])]);
    expect_rule(&p, "NATIVE_BIND_IN_INVALID");
}

/// BIND IN field binding both a parameter and a literal (link.rs:140-148).
#[test]
fn rejects_bind_in_field_both_param_and_literal() {
    let mut lf = link_fn();
    lf.params = vec![(
        "p".to_string(),
        crate::types::ParameterType::parse("Integer"),
    )];
    lf.abi_slots = vec![abi_slot("cfg", "Cfg", crate::ir::AbiDirection::In)];
    lf.bind_in = vec![crate::ir::IrBindIn {
        slot: "cfg".to_string(),
        fields: vec![crate::ir::IrBindInField {
            name: "a".to_string(),
            param: Some("p".to_string()),
            literal: Some(1),
        }],
    }];
    lf.result = None;
    lf.return_type = crate::types::ParameterType::parse("Nothing");
    let p = project_with_link(lf, vec![cstruct("Cfg", &[("a", "CInt32")])]);
    expect_rule(&p, "NATIVE_BIND_IN_INVALID");
}

/// BIND IN field binding an unknown parameter (link.rs:149-159).
#[test]
fn rejects_bind_in_field_unknown_param() {
    let mut lf = link_fn();
    lf.params = vec![(
        "p".to_string(),
        crate::types::ParameterType::parse("Integer"),
    )];
    lf.abi_slots = vec![abi_slot("cfg", "Cfg", crate::ir::AbiDirection::In)];
    lf.bind_in = vec![crate::ir::IrBindIn {
        slot: "cfg".to_string(),
        fields: vec![crate::ir::IrBindInField {
            name: "a".to_string(),
            param: Some("unknownp".to_string()),
            literal: None,
        }],
    }];
    lf.result = None;
    lf.return_type = crate::types::ParameterType::parse("Nothing");
    let p = project_with_link(lf, vec![cstruct("Cfg", &[("a", "CInt32")])]);
    expect_rule(&p, "NATIVE_BIND_IN_INVALID");
}

/// BIND STATE naming a slot that is not an OUT CSTRUCT slot (link.rs:506-521).
#[test]
fn rejects_bind_state_not_out_cstruct_slot() {
    let mut lf = link_fn();
    lf.bind_state = Some("nope".to_string());
    let p = project_with_link(lf, vec![]);
    expect_rule(&p, "NATIVE_BIND_STATE_INVALID");
}

/// BIND STATE but the function does not return a stateful resource
/// (link.rs:522-529).
#[test]
fn rejects_bind_state_without_stateful_return() {
    let mut lf = link_fn();
    lf.params = vec![];
    lf.abi_slots = vec![abi_slot("st", "State", crate::ir::AbiDirection::Out)];
    lf.bind_state = Some("st".to_string());
    lf.return_resource = false;
    let p = project_with_link(lf, vec![cstruct("State", &[("a", "CInt32")])]);
    expect_rule(&p, "NATIVE_BIND_STATE_INVALID");
}

/// BIND STATE where the CSTRUCT `maps_to` disagrees with the return STATE type
/// (link.rs:530-541).
#[test]
fn rejects_bind_state_maps_to_mismatch() {
    let mut lf = link_fn();
    lf.params = vec![];
    lf.abi_slots = vec![abi_slot("st", "State", crate::ir::AbiDirection::Out)];
    lf.bind_state = Some("st".to_string());
    lf.return_resource = true;
    lf.return_state_type = Some(crate::types::ParameterType::parse("Other"));
    lf.return_type = crate::types::ParameterType::parse("Db");
    // cstruct maps_to defaults to "Rec" which differs from "Other".
    let p = project_with_link(lf, vec![cstruct("State", &[("a", "CInt32")])]);
    expect_rule(&p, "NATIVE_BIND_STATE_INVALID");
}

/// BIND STATE `<res>` naming a slot other than the one the wrapper returns
/// (link.rs:548-568).
#[test]
fn rejects_bind_state_resource_wrong_slot() {
    let mut lf = link_fn();
    lf.params = vec![];
    lf.abi_slots = vec![abi_slot("st", "State", crate::ir::AbiDirection::Out)];
    lf.bind_state = Some("st".to_string());
    lf.return_resource = true;
    lf.return_state_type = Some(crate::types::ParameterType::parse("Rec"));
    lf.return_type = crate::types::ParameterType::parse("Db");
    lf.bind_state_resource = Some("wrong".to_string());
    // cstruct maps_to defaults to "Rec", matching return_state_type.
    let p = project_with_link(lf, vec![cstruct("State", &[("a", "CInt32")])]);
    expect_rule(&p, "NATIVE_BIND_STATE_INVALID");
}

/// BIND STATE `<res>` with a computed (non-`Var`) RETURN: `produced` is `None`,
/// so the resource-slot arm is skipped (link.rs:554,557).
#[test]
fn bind_state_resource_with_computed_result_is_skipped() {
    let mut lf = link_fn();
    lf.params = vec![];
    lf.abi_slots = vec![abi_slot("st", "State", crate::ir::AbiDirection::Out)];
    lf.bind_state = Some("st".to_string());
    lf.return_resource = true;
    lf.return_state_type = Some(crate::types::ParameterType::parse("Rec"));
    lf.return_type = crate::types::ParameterType::parse("Db");
    lf.bind_state_resource = Some("wrong".to_string());
    lf.result = Some(crate::ir::IrLinkExpr::Int(100));
    let p = project_with_link(lf, vec![cstruct("State", &[("a", "CInt32")])]);
    // maps_to matches and the produced slot is unknowable, so no BIND STATE fault.
    assert!(
        !rules(&p).iter().any(|r| r == "NATIVE_BIND_STATE_INVALID"),
        "unexpected BIND STATE fault"
    );
}

/// BIND STATE `<res>` with no RETURN: `produced` falls back to the ABI return
/// name (link.rs:555), which the named resource slot must match.
#[test]
fn rejects_bind_state_resource_against_abi_return() {
    let mut lf = link_fn();
    lf.params = vec![];
    lf.abi_slots = vec![abi_slot("st", "State", crate::ir::AbiDirection::Out)];
    lf.bind_state = Some("st".to_string());
    lf.return_resource = true;
    lf.return_state_type = Some(crate::types::ParameterType::parse("Rec"));
    lf.return_type = crate::types::ParameterType::parse("Db");
    lf.bind_state_resource = Some("wrong".to_string());
    lf.result = None; // produced = abi_return_name ("value") != "wrong"
    let p = project_with_link(lf, vec![cstruct("State", &[("a", "CInt32")])]);
    expect_rule(&p, "NATIVE_BIND_STATE_INVALID");
}

/// Two native declarations of the same resource base with different STATE types
/// (link.rs:581-602); the middle one agrees (the `Some(_)` arm), the last one
/// conflicts.
#[test]
fn rejects_native_resource_state_disagreement() {
    let mut producer = link_fn();
    producer.name = "prod".to_string();
    producer.return_resource = true;
    producer.return_state_type = Some(crate::types::ParameterType::parse("S1"));
    producer.return_type = crate::types::ParameterType::parse("Db");

    let mut agree = link_fn();
    agree.name = "agree".to_string();
    agree.params = vec![(
        "x".to_string(),
        crate::types::ParameterType::parse("Db STATE S1"),
    )];

    let mut bad = link_fn();
    bad.name = "bad".to_string();
    bad.params = vec![(
        "x".to_string(),
        crate::types::ParameterType::parse("Db STATE S2"),
    )];

    let mut p = project(vec![func_returns("run", "Nothing", vec![], vec![])], vec![]);
    p.link_functions = vec![producer, agree, bad];
    expect_rule(&p, "TYPE_STATE_MISMATCH");
}

/// `Set OF Map OF fs.File TO Integer` drives `contains_resource_or_thread` through
/// its Map recursion arm (link.rs:622-624).
#[test]
fn set_of_map_of_resource_is_ownership_violation() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![param("s", "Set OF Map OF fs.File TO Integer", None)],
        vec![],
    );
    expect_rule(
        &project(vec![f], vec![]),
        "TYPE_COLLECTION_OWNERSHIP_VIOLATION",
    );
}

/// A self-referential record containing a resource exercises the cycle guard of
/// `contains_resource_or_thread` (link.rs:626-627).
#[test]
fn set_of_cyclic_record_with_resource_is_ownership_violation() {
    let rec = record_typed("R", &[("self", "R"), ("h", "fs.File")]);
    let f = func_returns("run", "Nothing", vec![param("s", "Set OF R", None)], vec![]);
    expect_rule(
        &project(vec![f], vec![rec]),
        "TYPE_COLLECTION_OWNERSHIP_VIOLATION",
    );
}

// --- link expressions (plan-50-I) -------------------------------------------

/// The bug plan-50-I fixes: `lower_link_expr` mapped every identifier onto one
/// nameless "native return" variable, so a typo in a gate silently compared the
/// status instead. Now the name is carried and must resolve.
#[test]
fn rejects_link_expr_naming_no_slot() {
    let mut lf = link_fn();
    lf.success_on = Some(crate::ir::IrLinkExpr::Compare {
        op: "=".to_string(),
        lhs: Box::new(crate::ir::IrLinkExpr::Var("typo".to_string())),
        rhs: Box::new(crate::ir::IrLinkExpr::Int(0)),
    });
    let mut p = project(vec![func_returns("run", "Nothing", vec![], vec![])], vec![]);
    p.link_functions = vec![lf];
    expect_rule(&p, "NATIVE_ABI_UNBOUND_SLOT");
}

/// A gate may name the ABI return — the only shape that worked before plan-50-I,
/// and the one every in-tree binding uses.
#[test]
fn accepts_link_expr_naming_the_abi_return() {
    let mut lf = link_fn();
    lf.success_on = Some(crate::ir::IrLinkExpr::Compare {
        op: "=".to_string(),
        lhs: Box::new(crate::ir::IrLinkExpr::Var("value".to_string())),
        rhs: Box::new(crate::ir::IrLinkExpr::Int(0)),
    });
    let mut p = project(vec![func_returns("run", "Nothing", vec![], vec![])], vec![]);
    p.link_functions = vec![lf];
    accept(&p);
}

/// A gate may now name an ordinary ABI slot — impossible to express before
/// plan-50-I, where it silently read the status.
#[test]
fn accepts_link_expr_naming_an_abi_slot() {
    let mut lf = link_fn();
    lf.success_on = Some(crate::ir::IrLinkExpr::Compare {
        op: "=".to_string(),
        lhs: Box::new(crate::ir::IrLinkExpr::Var("path".to_string())),
        rhs: Box::new(crate::ir::IrLinkExpr::Int(0)),
    });
    let mut p = project(vec![func_returns("run", "Nothing", vec![], vec![])], vec![]);
    p.link_functions = vec![lf];
    accept(&p);
}

/// plan-50-E: a struct slot's record mapping must cover every field both ways —
/// a silently-unmapped field is zeroed in and dropped out, a wrong answer with no
/// diagnostic. Enforced on the package path too: a crafted `.mfp` never ran the
/// frontend.
#[test]
fn rejects_struct_slot_with_uncovered_record_field() {
    let mut lf = link_fn();
    lf.return_type = crate::types::ParameterType::parse("Rec");
    lf.params = vec![];
    lf.abi_slots = vec![crate::ir::IrAbiSlot {
        name: "s".to_string(),
        ctype: crate::types::ParameterType::parse("S"),
        direction: crate::ir::AbiDirection::Out,
    }];
    lf.result = Some(crate::ir::IrLinkExpr::Var("s".to_string()));
    let mut p = project(
        vec![func_returns("run", "Nothing", vec![], vec![])],
        vec![record_typed(
            "Rec",
            &[("a", "Integer"), ("extra", "Integer")],
        )],
    );
    p.link_cstructs = vec![cstruct("S", &[("a", "CInt32")])];
    p.link_functions = vec![lf];
    expect_rule(&p, "NATIVE_STRUCT_FIELD_MISMATCH");
}

#[test]
fn rejects_struct_slot_with_mistyped_record_field() {
    let mut lf = link_fn();
    lf.return_type = crate::types::ParameterType::parse("Rec");
    lf.params = vec![];
    lf.abi_slots = vec![crate::ir::IrAbiSlot {
        name: "s".to_string(),
        ctype: crate::types::ParameterType::parse("S"),
        direction: crate::ir::AbiDirection::Out,
    }];
    lf.result = Some(crate::ir::IrLinkExpr::Var("s".to_string()));
    let mut p = project(
        vec![func_returns("run", "Nothing", vec![], vec![])],
        // CInt32 maps to Integer, not String.
        vec![record_typed("Rec", &[("a", "String")])],
    );
    p.link_cstructs = vec![cstruct("S", &[("a", "CInt32")])];
    p.link_functions = vec![lf];
    expect_rule(&p, "NATIVE_STRUCT_FIELD_MISMATCH");
}

/// plan-50-A: the package path is a marshaling-safety gate — a crafted `.mfp`
/// link table drives raw C calls, and an unknown slot ctype used to fall through
/// to a raw 64-bit marshal in the thunk's default arm.
#[test]
fn rejects_link_unknown_slot_ctype() {
    let mut lf = link_fn();
    lf.abi_slots = vec![crate::ir::IrAbiSlot {
        name: "path".to_string(),
        ctype: crate::types::ParameterType::parse("CIint32"),
        direction: crate::ir::AbiDirection::In,
    }];
    let mut p = project(vec![func_returns("run", "Nothing", vec![], vec![])], vec![]);
    p.link_functions = vec![lf];
    expect_rule(&p, "NATIVE_ABI_UNKNOWN_CTYPE");
}

#[test]
fn rejects_link_unknown_return_ctype() {
    let mut lf = link_fn();
    lf.abi_return_ctype = crate::types::ParameterType::parse("CFloat32");
    let mut p = project(vec![func_returns("run", "Nothing", vec![], vec![])], vec![]);
    p.link_functions = vec![lf];
    expect_rule(&p, "NATIVE_ABI_UNKNOWN_CTYPE");
}

/// `CVoid` is a return-only type; a void *argument* is meaningless.
#[test]
fn rejects_link_cvoid_argument_slot() {
    let mut lf = link_fn();
    lf.abi_slots = vec![crate::ir::IrAbiSlot {
        name: "path".to_string(),
        ctype: crate::types::ParameterType::parse("CVoid"),
        direction: crate::ir::AbiDirection::In,
    }];
    let mut p = project(vec![func_returns("run", "Nothing", vec![], vec![])], vec![]);
    p.link_functions = vec![lf];
    expect_rule(&p, "NATIVE_ABI_UNKNOWN_CTYPE");
}

#[test]
fn rejects_link_cptr_escape_in_param() {
    let mut lf = link_fn();
    lf.params = vec![("p".to_string(), crate::types::ParameterType::parse("CPtr"))];
    lf.abi_slots = vec![crate::ir::IrAbiSlot {
        name: "p".to_string(),
        ctype: crate::types::ParameterType::parse("CPtr"),
        direction: crate::ir::AbiDirection::In,
    }];
    let mut p = project(vec![func_returns("run", "Nothing", vec![], vec![])], vec![]);
    p.link_functions = vec![lf];
    expect_rule(&p, "NATIVE_CPTR_ESCAPE");
}

#[test]
fn rejects_link_cptr_escape_in_return() {
    let mut lf = link_fn();
    lf.return_type = crate::types::ParameterType::parse("CPtr");
    let mut p = project(vec![func_returns("run", "Nothing", vec![], vec![])], vec![]);
    p.link_functions = vec![lf];
    expect_rule(&p, "NATIVE_CPTR_ESCAPE");
}

#[test]
fn rejects_link_unbound_input_slot() {
    // plan-50-H: a slot named `return` carries no meaning, so an input slot that
    // binds to no parameter and no CONST pin is just an unbound slot.
    let mut lf = link_fn();
    lf.abi_slots = vec![
        crate::ir::IrAbiSlot {
            name: "path".to_string(),
            ctype: crate::types::ParameterType::parse("CString"),
            direction: crate::ir::AbiDirection::In,
        },
        crate::ir::IrAbiSlot {
            name: "stray".to_string(),
            ctype: crate::types::ParameterType::parse("CInt32"),
            direction: crate::ir::AbiDirection::In,
        },
    ];
    let mut p = project(vec![func_returns("run", "Nothing", vec![], vec![])], vec![]);
    p.link_functions = vec![lf];
    expect_rule(&p, "NATIVE_ABI_UNBOUND_SLOT");
}

#[test]
fn rejects_link_unbound_slot() {
    let mut lf = link_fn();
    lf.abi_slots = vec![
        crate::ir::IrAbiSlot {
            name: "path".to_string(),
            ctype: crate::types::ParameterType::parse("CString"),
            direction: crate::ir::AbiDirection::In,
        },
        crate::ir::IrAbiSlot {
            name: "mystery".to_string(),
            ctype: crate::types::ParameterType::parse("CInt32"),
            direction: crate::ir::AbiDirection::In,
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
            ctype: crate::types::ParameterType::parse("CString"),
            direction: crate::ir::AbiDirection::In,
        },
        crate::ir::IrAbiSlot {
            name: "extra".to_string(),
            ctype: crate::types::ParameterType::parse("CInt32"),
            direction: crate::ir::AbiDirection::Out,
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
            ctype: crate::types::ParameterType::parse("CString"),
            direction: crate::ir::AbiDirection::In,
        },
        crate::ir::IrAbiSlot {
            name: "flags".to_string(),
            ctype: crate::types::ParameterType::parse("CInt32"),
            direction: crate::ir::AbiDirection::Out,
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
        ctype: crate::types::ParameterType::parse("CString"),
        direction: crate::ir::AbiDirection::In,
    }];
    // A value-returning wrapper with no RETURN clause names no result.
    lf.result = None;
    let mut p = project(vec![func_returns("run", "Nothing", vec![], vec![])], vec![]);
    p.link_functions = vec![lf];
    expect_rule(&p, "NATIVE_ABI_NO_RESULT");
}

#[test]
fn rejects_link_unbound_param() {
    let mut lf = link_fn();
    lf.params = vec![
        (
            "path".to_string(),
            crate::types::ParameterType::parse("String"),
        ),
        (
            "extra".to_string(),
            crate::types::ParameterType::parse("Integer"),
        ),
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
            ctype: crate::types::ParameterType::parse("CString"),
            direction: crate::ir::AbiDirection::In,
        },
        crate::ir::IrAbiSlot {
            name: "flags".to_string(),
            ctype: crate::types::ParameterType::parse("CInt32"),
            direction: crate::ir::AbiDirection::In,
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

// --- compatible / expression_compatible coercion paths ---------------------

#[test]
fn accepts_byte_literal_into_byte_param() {
    let callee = func_returns("helper", "Nothing", vec![param("b", "Byte", None)], vec![]);
    let body = vec![IrOp::Eval {
        value: IrValue::Call {
            target: "helper".to_string(),
            args: vec![int_const("5")],
            type_: crate::types::ParameterType::parse("Nothing"),
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
            type_: crate::types::ParameterType::parse("Nothing"),
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
            Some(unary(UnaryOp::Negate, int_const("1"), "Integer")),
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
        type_: crate::types::ParameterType::parse("Circle"),
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
            BinaryOp::Subtract,
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
        BinaryOp::Subtract,
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
    //
    // The key needs its OWN ` TO Out`, so the well-formed spelling carries two
    // top-level ` TO `s: the first belongs to the nested `Thread OF`, the second
    // separates the map's key from its value (`split_top_level_to`). Written
    // `Map OF Thread OF Integer TO Integer` (one ` TO `) before plan-106-B, which
    // `ParameterType::parse` correctly reads as a map with NO value type and so
    // yields an opaque `Named` — the check only ever fired because the duplicate
    // `parse_map` split naively on the FIRST ` TO `, mis-reading the key as
    // `Thread OF Integer` and the value as `Integer`.
    let f = func_returns(
        "run",
        "Nothing",
        vec![param(
            "m",
            "Map OF Thread OF Integer TO Integer TO Integer",
            None,
        )],
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
    let mut holder = record_typed("Holder", &[("f", "fs.File")]);
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
        type_: crate::types::ParameterType::parse("Unknown"),
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
            type_: ParameterType::Integer,
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
            type_: ParameterType::Integer,
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
            type_: crate::types::ParameterType::parse("Unknown"),
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
        // Two arguments (the member's arity), both of the wrong type: the arity
        // check precedes the type check, as in the source checker (plan-107-E).
        vec![eval_call(
            "bits.band",
            vec![const_of("String", "x"), const_of("String", "y")],
        )],
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
        // plan-110-C moved the datagram surface to `udp`, so `net.bindUdp` no
        // longer exists; `net.lookup` is the equivalent one-required-argument net
        // member, and one Integer where a String belongs is the same mistake.
        vec![eval_call("net.lookup", vec![int_const("1")])],
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
        type_: crate::types::ParameterType::parse("Integer"),
        by_ref: false,
    };
    let body = vec![
        // Constructor(Capture)
        bind(
            "a",
            "Point",
            Some(IrValue::Constructor {
                type_: crate::types::ParameterType::parse("Point"),
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
                type_: crate::types::ParameterType::parse("List OF Integer"),
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
                type_: crate::types::ParameterType::parse("Map OF Integer TO Integer"),
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
                op: BinaryOp::Add,
                left: Box::new(cap()),
                right: Box::new(cap()),
                type_: crate::types::ParameterType::parse("Integer"),
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
                op: UnaryOp::Negate,
                operand: Box::new(cap()),
                type_: crate::types::ParameterType::parse("Integer"),
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
                type_: crate::types::ParameterType::parse("Point"),
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
                type_: crate::types::ParameterType::parse("Integer"),
            }),
            false,
            false,
        ),
        // Closure(Capture) as a nested closure argument
        eval_call(
            "io.print",
            vec![IrValue::Closure {
                name: "inner".to_string(),
                type_: crate::types::ParameterType::parse("FUNC() AS Integer"),
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
                union_type: crate::types::ParameterType::parse("Shape"),
                member_type: crate::types::ParameterType::parse("Circle"),
                value: Box::new(IrValue::Local("c".to_string())),
            }),
            false,
            false,
        ),
        bind(
            "e",
            "Circle",
            Some(IrValue::UnionExtract {
                type_: crate::types::ParameterType::parse("Circle"),
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
                type_: crate::types::ParameterType::parse("Integer"),
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
                type_: crate::types::ParameterType::parse("Integer"),
            }),
            false,
            false,
        ),
        bind(
            "fr",
            "FUNC() AS Integer",
            Some(IrValue::FunctionRef {
                name: "helper".to_string(),
                type_: crate::types::ParameterType::parse("FUNC() AS Integer"),
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

// --- non-owning element (RES bind of collections.get) ----------------------

fn get_call(list: &str, ret_type: &str) -> IrValue {
    IrValue::Call {
        target: "collections.get".to_string(),
        args: vec![IrValue::Local(list.to_string()), int_const("0")],
        type_: crate::types::ParameterType::parse(ret_type),
        loc: IrSourceLoc::default(),
    }
}

#[test]
fn accepts_res_bind_of_a_collection_element() {
    // plan-59-E: was `rejects_res_bind_of_non_owning_element`
    // (`TYPE_RESOURCE_ELEMENT_NOT_OWNER`, retired). A `RES` is a pointer to the
    // one resource, and an element is such a pointer like any other holder.
    // RES h = collections.get(xs, 0) where the element type is a resource.
    let body = vec![bind(
        "h",
        "fs.File",
        Some(get_call("xs", "fs.File")),
        true,
        false,
    )];
    let mut f = func_returns(
        "run",
        "Nothing",
        vec![param("xs", "List OF RES fs.File", None)],
        body,
    );
    f.resource_owners
        .insert("h".to_string(), crate::ir::resource_escape::ResOwner::Local);
    accept(&project(vec![f], vec![]));
}

#[test]
fn accepts_return_of_a_resource_collection_element() {
    // plan-59-E: was `rejects_return_non_owning_resource_element`
    // (`TYPE_RESOURCE_ELEMENT_NOT_OWNER`, retired). Returning the element hands
    // the pointer to the caller, whose scope becomes the outermost one touching
    // the resource.
    // RETURN collections.get(xs, 0) whose element is a resource.
    let body = vec![ret(get_call("xs", "fs.File"))];
    let f = func_returns(
        "run",
        "fs.File",
        vec![param("xs", "List OF RES fs.File", None)],
        body,
    );
    accept(&project(vec![f], vec![]));
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
fn mut_set_is_defaultable() {
    // `MUT s AS Set OF Integer` with no initializer defaults to the empty set.
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind("s", "Set OF Integer", None, true, true)],
    );
    accept(&project(vec![f], vec![]));
}

#[test]
fn rejects_mut_set_of_resource_ownership_and_comparable() {
    // `MUT s AS Set OF fs.File`: after bug-434 a `Set OF T` is ALWAYS defaultable
    // (empty set), so the former `TYPE_MUT_REQUIRES_DEFAULTABLE_TYPE` no longer
    // fires. The binding is still rejected — on the two INDEPENDENT axes bug-434
    // deliberately left untouched: an ordinary collection cannot own a resource
    // (`TYPE_COLLECTION_OWNERSHIP_VIOLATION`) and a Set element must be
    // comparable (`TYPE_REQUIRES_COMPARABLE`, File is not). Verified against the
    // release binary: `MUT s AS Set OF fs.File = []` is likewise rejected on these
    // same axes, so the doc's premise that this becomes accepted was wrong.
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind("s", "Set OF fs.File", None, true, true)],
    );
    let got = rules(&project(vec![f], vec![]));
    assert!(
        !got.iter()
            .any(|r| r == "TYPE_MUT_REQUIRES_DEFAULTABLE_TYPE"),
        "Set OF T is defaultable after bug-434; the defaultability rule must not fire: {got:?}"
    );
    assert!(
        got.iter()
            .any(|r| r == "TYPE_COLLECTION_OWNERSHIP_VIOLATION"),
        "a resource in an ordinary collection is still rejected on the ownership axis: {got:?}"
    );
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
fn accepts_mut_list_of_union_defaultable_empty() {
    // bug-434: `List OF <union>` is always defaultable (empty list). The
    // element type's defaultability is irrelevant — an empty list materializes
    // no element.
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind("xs", "List OF Choice", None, true, true)],
    );
    accept(&project(vec![f], vec![union("Choice", &["Label"])]));
}

#[test]
fn accepts_mut_map_value_union_defaultable_empty() {
    // bug-434: `Map OF K TO <union>` is always defaultable (empty map).
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind("m", "Map OF String TO Choice", None, true, true)],
    );
    accept(&project(vec![f], vec![union("Choice", &["Label"])]));
}

#[test]
fn accepts_mut_record_with_list_of_union_field() {
    // bug-434: the non-defaultability of a `List OF <union>` field must NOT
    // cascade into the containing record. A record embedding such a field is
    // defaultable (the field defaults to the empty list).
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind("d", "Doc", None, true, true)],
    );
    accept(&project(
        vec![f],
        vec![
            record_typed("Doc", &[("attrs", "List OF Choice")]),
            union("Choice", &["Label"]),
        ],
    ));
}

#[test]
fn accepts_state_list_of_nondefaultable() {
    // bug-434 (intended STATE ripple): `fs.File STATE List OF <union>` is a valid
    // initial state — the empty list. Rides the same is_defaultable predicate
    // as the MUT axis, so it falls out for free. Mirror of
    // `rejects_state_type_not_defaultable` (a bare union STATE, still rejected):
    // only the STATE-defaultability axis is asserted here, since binding-shape
    // rules (LET-requires-value) are orthogonal to this ripple.
    let mut f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind("h", "fs.File STATE List OF Choice", None, true, false)],
    );
    f.resource_owners
        .insert("h".to_string(), crate::ir::resource_escape::ResOwner::Local);
    let got = rules(&project(vec![f], vec![union("Choice", &["Label"])]));
    assert!(
        !got.iter().any(|r| r == "TYPE_STATE_INVALID"),
        "STATE List OF <union> must be a valid (empty-list) initial state: {got:?}"
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

// --- resource moves inside nested blocks -----------------------------------

fn close_eval(h: &str) -> IrOp {
    IrOp::Eval {
        value: IrValue::Call {
            target: "fs.close".to_string(),
            args: vec![IrValue::Local(h.to_string())],
            type_: crate::types::ParameterType::parse("Nothing"),
            loc: IrSourceLoc::default(),
        },
        loc: IrSourceLoc::default(),
    }
}

fn owner_fn(name: &str, ret: &str, body: Vec<IrOp>, owners: &[&str]) -> IrFunction {
    let mut f = func_returns(name, ret, vec![], body);
    for o in owners {
        f.resource_owners.insert(
            (*o).to_string(),
            crate::ir::resource_escape::ResOwner::Local,
        );
    }
    f
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

// ---------------------------------------------------------------------------
// LINK rule parity (bug-325)
// ---------------------------------------------------------------------------

/// the former source checker's `check_link_function_in` and `verify::check_link_functions` are
/// two independently-maintained ~320-line bodies validating the same LINK ABI
/// facts. They are kept in sync by hand, and adding a `NATIVE_*` rule to one
/// side and forgetting the other is silent — this test makes it loud.
///
/// The invariant, with `ir::verify` as the sole rejecter for every relocated
/// rule (plan-20):
///
// --- plan-58-A: CBuffer position rules on the PACKAGE path -----------------
//
// One twin per `check_buffer_slots` rule. These are the reason the checker is a
// shared function in `ir::link` rather than two hand-mirrored implementations:
// a crafted `.mfp` must get exactly the source-path treatment, and the two
// `is_c_abi_type` copies are what happens when it is written twice.
//
// The source-path halves live in `tests/syntax/native/native-cbuffer-*`.

/// A well-formed `OUT CBuffer` wrapper: every rule below mutates one thing about
/// it, so a test that stops failing means that rule stopped firing — not that the
/// fixture drifted into some other rejection.
fn cbuffer_fn() -> crate::ir::IrLinkFunction {
    let mut lf = link_fn();
    lf.params = vec![(
        "n".to_string(),
        crate::types::ParameterType::parse("Integer"),
    )];
    lf.return_type = crate::types::ParameterType::parse(crate::ir::BYTE_LIST_TYPE);
    lf.abi_slots = vec![
        crate::ir::IrAbiSlot {
            name: "buf".to_string(),
            ctype: crate::types::ParameterType::parse("CBuffer"),
            direction: crate::ir::AbiDirection::Out,
        },
        crate::ir::IrAbiSlot {
            name: "n".to_string(),
            ctype: crate::types::ParameterType::parse("CInt64"),
            direction: crate::ir::AbiDirection::In,
        },
    ];
    lf.abi_return_name = "status".to_string();
    lf.abi_return_ctype = crate::types::ParameterType::parse("CInt32");
    lf.result = Some(crate::ir::IrLinkExpr::Var("buf".to_string()));
    lf.buffers = vec![crate::ir::IrBuffer {
        slot: "buf".to_string(),
        size: crate::ir::IrLinkExpr::Var("n".to_string()),
    }];
    // Mandatory since plan-58-B rule 10: `status` is what the callee reports it
    // wrote. Unlike SIZE, a LENGTH expression MAY read the ABI return, because it
    // is evaluated after the call.
    lf.result_length = Some(crate::ir::IrLinkExpr::Var("status".to_string()));
    lf
}

fn cbuffer_project(lf: crate::ir::IrLinkFunction) -> crate::ir::IrProject {
    let mut p = project(vec![func_returns("run", "Nothing", vec![], vec![])], vec![]);
    p.link_functions = vec![lf];
    p
}

/// The baseline must be ACCEPTED, or every rejection test below is vacuous — it
/// would pass on a fixture that was already invalid for some unrelated reason.
#[test]
fn accepts_well_formed_cbuffer_link_function() {
    accept(&cbuffer_project(cbuffer_fn()));
}

/// Rule 1: OUT-only. There is no `List OF Byte` input marshal, so no send
/// direction exists to give `IN`/`INOUT CBuffer` a meaning.
#[test]
fn rejects_cbuffer_in_slot() {
    let mut lf = cbuffer_fn();
    lf.abi_slots[0].direction = crate::ir::AbiDirection::In;
    expect_rule(&cbuffer_project(lf), "NATIVE_BUFFER_INVALID");
}

#[test]
fn rejects_cbuffer_inout_slot() {
    let mut lf = cbuffer_fn();
    lf.abi_slots[0].direction = crate::ir::AbiDirection::InOut;
    expect_rule(&cbuffer_project(lf), "NATIVE_BUFFER_INVALID");
}

/// Rule 2: exactly one BUFFER clause. Zero leaves the capacity undefined — and a
/// decoded `.mfp` carries NO buffers today (plan-58-C owns the wire format), so
/// this is the rule that actually fires on a packaged CBuffer binding.
#[test]
fn rejects_cbuffer_without_buffer_clause() {
    let mut lf = cbuffer_fn();
    lf.buffers.clear();
    expect_rule(&cbuffer_project(lf), "NATIVE_BUFFER_INVALID");
}

#[test]
fn rejects_cbuffer_with_two_buffer_clauses() {
    let mut lf = cbuffer_fn();
    lf.buffers.push(crate::ir::IrBuffer {
        slot: "buf".to_string(),
        size: crate::ir::IrLinkExpr::Int(4096),
    });
    expect_rule(&cbuffer_project(lf), "NATIVE_BUFFER_INVALID");
}

/// Rule 3: a BUFFER clause must name a CBuffer slot of this function.
#[test]
fn rejects_buffer_clause_naming_unknown_slot() {
    let mut lf = cbuffer_fn();
    lf.buffers[0].slot = "nosuch".to_string();
    expect_rule(&cbuffer_project(lf), "NATIVE_BUFFER_INVALID");
}

#[test]
fn rejects_buffer_clause_naming_non_cbuffer_slot() {
    let mut lf = cbuffer_fn();
    lf.buffers[0].slot = "n".to_string();
    expect_rule(&cbuffer_project(lf), "NATIVE_BUFFER_INVALID");
}

/// Rule 4: a CONST pin on a CBuffer. It is an OUT slot, so this is the existing
/// `NATIVE_CONST_OUT` — no code is minted for a condition an existing rule names.
#[test]
fn rejects_cbuffer_const_pin() {
    let mut lf = cbuffer_fn();
    lf.consts = vec![("buf".to_string(), 0)];
    expect_rule(&cbuffer_project(lf), "NATIVE_CONST_OUT");
}

/// Rule 5: CBuffer as the ABI return proper. `abi_ctype_valid_as_return` cannot
/// express this — an OUT slot is checked through that same predicate — so it is a
/// position rule in `check_buffer_slots`, reusing the existing ctype rule.
#[test]
fn rejects_cbuffer_as_abi_return() {
    let mut lf = cbuffer_fn();
    lf.abi_return_ctype = crate::types::ParameterType::parse("CBuffer");
    expect_rule(&cbuffer_project(lf), "NATIVE_ABI_UNKNOWN_CTYPE");
}

/// Rule 6: a CBuffer `RETURN` does not name is unreachable, and unlike a scalar
/// OUT it costs a runtime-sized allocation nothing can observe.
#[test]
fn rejects_cbuffer_not_named_by_return() {
    let mut lf = cbuffer_fn();
    lf.return_type = crate::types::ParameterType::parse("Integer");
    lf.result = Some(crate::ir::IrLinkExpr::Var("status".to_string()));
    expect_rule(&cbuffer_project(lf), "NATIVE_BUFFER_INVALID");
}

/// Rule 7: RETURN names a CBuffer, so the wrapper must surface it as bytes.
#[test]
fn rejects_cbuffer_return_with_wrong_wrapper_type() {
    let mut lf = cbuffer_fn();
    lf.return_type = crate::types::ParameterType::parse("String");
    expect_rule(&cbuffer_project(lf), "NATIVE_BUFFER_INVALID");
}

/// Rule 8 — the pre-existing garbage-codegen hole (plan-58-A §2.3). Before
/// plan-58 this function COMPILED: `emit_return_passthrough` has no List-building
/// arm, so the caller dereferenced a raw scalar as a collection block with no
/// diagnostic. This is the package-path half, which never had even the CSTRUCT
/// return-type check the source path had.
#[test]
fn rejects_byte_list_return_without_cbuffer_slot() {
    let mut lf = link_fn();
    lf.return_type = crate::types::ParameterType::parse(crate::ir::BYTE_LIST_TYPE);
    expect_rule(&cbuffer_project(lf), "NATIVE_BUFFER_INVALID");
}

/// Rule 9: a SIZE expression naming nothing real. This is the capacity of a
/// buffer a C function is about to write into, so an unresolved name here is not
/// cosmetic.
#[test]
fn rejects_buffer_size_naming_unknown_slot() {
    let mut lf = cbuffer_fn();
    lf.buffers[0].size = crate::ir::IrLinkExpr::Var("nosuch".to_string());
    expect_rule(&cbuffer_project(lf), "NATIVE_ABI_UNBOUND_SLOT");
}

/// A SIZE expression may NOT read the ABI return — a causality error, not an
/// unbound name. `SUCCESS_ON`/`RETURN` are evaluated after the call and may read
/// it; a `BUFFER SIZE` is evaluated during staging, so at that point the status
/// word is uninitialized frame memory. Sizing an allocation from it would be a
/// silent wrong answer of the worst kind.
///
/// plan-58-A shipped rule 9 accepting this. Caught while implementing plan-58-B's
/// staging pass, where there is demonstrably nothing to load.
#[test]
fn rejects_buffer_size_reading_the_abi_return() {
    let mut lf = cbuffer_fn();
    lf.buffers[0].size = crate::ir::IrLinkExpr::Var("status".to_string());
    expect_rule(&cbuffer_project(lf), "NATIVE_ABI_UNBOUND_SLOT");
}

/// Same for an OUT slot: its value is whatever the callee writes, which has not
/// happened yet.
#[test]
fn rejects_buffer_size_reading_an_out_slot() {
    let mut lf = cbuffer_fn();
    lf.abi_slots.push(crate::ir::IrAbiSlot {
        name: "written".to_string(),
        ctype: crate::types::ParameterType::parse("CInt64"),
        direction: crate::ir::AbiDirection::Out,
    });
    lf.buffers[0].size = crate::ir::IrLinkExpr::Var("written".to_string());
    expect_rule(&cbuffer_project(lf), "NATIVE_ABI_UNBOUND_SLOT");
}

/// A CONST pin IS readable: it is a compile-time immediate, so it exists during
/// staging.
#[test]
fn accepts_buffer_size_reading_a_const_pin() {
    let mut lf = cbuffer_fn();
    lf.abi_slots.push(crate::ir::IrAbiSlot {
        name: "cap".to_string(),
        ctype: crate::types::ParameterType::parse("CInt64"),
        direction: crate::ir::AbiDirection::In,
    });
    lf.consts = vec![("cap".to_string(), 4096)];
    lf.buffers[0].size = crate::ir::IrLinkExpr::Var("cap".to_string());
    accept(&cbuffer_project(lf));
}

/// Rule 10: a returned CBuffer must carry a LENGTH clause.
///
/// Without one the list's `count` is its full capacity, so a callee that writes
/// fewer bytes than the buffer holds leaves the remainder as uninitialized arena
/// memory that ordinary code reads as data. Observed during plan-58-B Phase 2
/// before this rule existed: a short `pread` surfaced stale bytes as the result.
#[test]
fn rejects_returned_cbuffer_without_length() {
    let mut lf = cbuffer_fn();
    lf.result_length = None;
    expect_rule(&cbuffer_project(lf), "NATIVE_BUFFER_INVALID");
}

/// Rule 11: a LENGTH clause with no CBuffer to apply it to. Every other result is
/// a scalar, which has no length to set, so this is a mistake rather than a no-op.
#[test]
fn rejects_length_without_a_cbuffer_result() {
    let mut lf = link_fn();
    lf.result_length = Some(crate::ir::IrLinkExpr::Var("value".to_string()));
    expect_rule(&cbuffer_project(lf), "NATIVE_BUFFER_INVALID");
}

/// Rule 12: a LENGTH expression naming nothing real.
#[test]
fn rejects_length_naming_unknown_slot() {
    let mut lf = cbuffer_fn();
    lf.result_length = Some(crate::ir::IrLinkExpr::Var("nosuch".to_string()));
    expect_rule(&cbuffer_project(lf), "NATIVE_ABI_UNBOUND_SLOT");
}

/// The accept set for LENGTH is the WIDE one — it is evaluated after the call, so
/// an OUT slot holds a real value by then. This is the deliberate asymmetry with
/// rule 9's SIZE, and asserting it stops the two from being "unified" later.
#[test]
fn accepts_length_reading_an_out_slot() {
    let mut lf = cbuffer_fn();
    lf.abi_slots.push(crate::ir::IrAbiSlot {
        name: "written".to_string(),
        ctype: crate::types::ParameterType::parse("CInt64"),
        direction: crate::ir::AbiDirection::Out,
    });
    lf.result_length = Some(crate::ir::IrLinkExpr::Var("written".to_string()));
    accept(&cbuffer_project(lf));
}

/// A LENGTH may scale a callee's ELEMENT count to bytes — the plan-58-B Phase 2
/// arithmetic, on the post-call side.
#[test]
fn accepts_length_scaling_the_abi_return() {
    let mut lf = cbuffer_fn();
    lf.result_length = Some(crate::ir::IrLinkExpr::Mul(
        Box::new(crate::ir::IrLinkExpr::Var("status".to_string())),
        Box::new(crate::ir::IrLinkExpr::Int(2)),
    ));
    accept(&cbuffer_project(lf));
}

/// plan-106-B: `is_thread_type` keeps a NAME arm beside its structural one, and
/// this pins it.
///
/// `ParameterType::parse` builds a `ThreadHandle` only from the complete
/// `Thread OF Msg [RES R] TO Out` shape. Decoded package IR is
/// attacker-controlled (PKG-02) and need not be well formed, so a truncated
/// `Thread OF Integer` arrives as an opaque `Named` — and must still be treated
/// as a thread handle by the ownership and `.result` rules, exactly as the
/// pre-plan-106 `starts_with("Thread")` did.
#[test]
fn truncated_thread_spelling_still_counts_as_a_thread() {
    // Well formed → the structural arm.
    let handle = ParameterType::parse("Thread OF Integer TO Integer");
    assert!(matches!(handle, ParameterType::ThreadHandle { .. }));
    assert!(super::is_thread_type(&handle));

    // Truncated / crafted → an opaque `Named`, caught by the name arm.
    let truncated = ParameterType::parse("Thread OF Integer");
    assert!(matches!(truncated, ParameterType::Named(_)));
    assert!(super::is_thread_type(&truncated));

    let worker = ParameterType::parse("ThreadWorker OF Integer");
    assert!(super::is_thread_type(&worker));

    // An ordinary nominal is not a thread, however it is spelled.
    assert!(!super::is_thread_type(&ParameterType::named("Threadbare")));
    assert!(!super::is_thread_type(&ParameterType::Integer));
    assert!(!super::is_thread_type(&ParameterType::list_of(
        ParameterType::Integer
    )));
}

// ---------------------------------------------------------------------------
// plan-107-A pilots — the package-path twins of the relocated rules. A crafted
// `.mfp` gets exactly the source-path treatment; these prove the rule fires on
// decoded IR, where no the former source checker ever ran.
// ---------------------------------------------------------------------------

/// plan-115-A: `ISOLATED` is orthogonal to visibility, so a `PRIVATE ISOLATED
/// FUNC` is well-formed. This is the converted twin of the former
/// `rejects_private_isolated_func`, which pinned the bug-227 restriction that
/// plan-115-A deliberately lifts (an entry no longer has to be reachable from
/// another package, so "project-visible" has no surviving rationale). The
/// declaration-form half of the rule is still pinned by `rejects_isolated_sub`
/// below — that pairing is the point: only the visibility half was lifted.
#[test]
fn accepts_private_isolated_func() {
    let mut f = func("w", vec![], vec![ret(int_const("0"))]);
    f.isolated = true;
    f.visibility = "private".to_string();
    expect_no_rule(&project(vec![f], vec![]), "TYPE_ISOLATED_NOT_VISIBLE");
}

#[test]
fn rejects_isolated_sub() {
    let mut s = func_returns("w", "Nothing", vec![], vec![ret_none()]);
    s.kind = "sub".to_string();
    s.isolated = true;
    expect_rule(&project(vec![s], vec![]), "TYPE_ISOLATED_NOT_VISIBLE");
}

#[test]
fn accepts_public_isolated_func() {
    let mut f = func("w", vec![], vec![ret(int_const("0"))]);
    f.isolated = true;
    f.visibility = "public".to_string();
    accept(&project(vec![f], vec![]));
}

/// The lowered shape of `LET a = <scrutinee> TRAP(e) … END TRAP` with `handler`
/// as the handler's ops (`lower_inline_trap`).
fn inline_trap_ops(scrutinee: IrValue, handler: Vec<IrOp>) -> Vec<IrOp> {
    let res = || IrValue::Local("$trap_res0".to_string());
    vec![
        IrOp::Bind {
            mutable: false,
            name: "$trap_res0".to_string(),
            type_: ParameterType::result_of(ParameterType::Integer),
            value: Some(scrutinee),
            explicit_type: false,
            loc: IrSourceLoc::default(),
        },
        bind("$trap_val0", "Integer", None, false, true),
        IrOp::If {
            condition: IrValue::ResultIsOk {
                value: Box::new(res()),
            },
            then_body: vec![IrOp::Assign {
                name: "$trap_val0".to_string(),
                value: IrValue::ResultValue {
                    type_: ParameterType::Integer,
                    value: Box::new(res()),
                },
                loc: IrSourceLoc::default(),
            }],
            else_body: handler,
            loc: IrSourceLoc::default(),
        },
        bind(
            "a",
            "Integer",
            Some(IrValue::Local("$trap_val0".to_string())),
            false,
            false,
        ),
        ret(IrValue::Local("a".to_string())),
    ]
}

fn recover_zero() -> IrOp {
    IrOp::Assign {
        name: "$trap_val0".to_string(),
        value: int_const("0"),
        loc: IrSourceLoc::default(),
    }
}

#[test]
fn rejects_inline_trap_on_a_non_call() {
    let body = inline_trap_ops(int_const("5"), vec![recover_zero()]);
    expect_rule(
        &project(vec![func("run", vec![], body)], vec![]),
        "TYPE_INLINE_TRAP_REQUIRES_FALLIBLE",
    );
}

#[test]
fn rejects_inline_trap_on_a_package_constant() {
    let scrutinee = IrValue::CallResult {
        target: "math.pi".to_string(),
        args: vec![],
        type_: ParameterType::Float,
        loc: IrSourceLoc::default(),
    };
    let body = inline_trap_ops(scrutinee, vec![recover_zero()]);
    expect_rule(
        &project(vec![func("run", vec![], body)], vec![]),
        "TYPE_INLINE_TRAP_REQUIRES_FALLIBLE",
    );
}

#[test]
fn accepts_inline_trap_on_a_call() {
    let scrutinee = IrValue::CallResult {
        target: "getName".to_string(),
        args: vec![],
        type_: ParameterType::Integer,
        loc: IrSourceLoc::default(),
    };
    let body = inline_trap_ops(scrutinee, vec![recover_zero()]);
    let callee = func("getName", vec![], vec![ret(int_const("1"))]);
    let got = rules(&project(vec![func("run", vec![], body), callee], vec![]));
    assert!(
        !got.iter()
            .any(|r| r == "TYPE_INLINE_TRAP_REQUIRES_FALLIBLE"),
        "{got:?}"
    );
}

#[test]
fn skips_the_testing_desugared_trap_guard() {
    // `expectTrap(5)` desugars into the inline-trap shape whose handler sets a
    // `$expect_trapped` temp; that form has its own TESTING rule in the
    // front end and must not be reported as an inline TRAP.
    let handler = vec![IrOp::Assign {
        name: "$expect_trapped0".to_string(),
        value: const_of("Boolean", "true"),
        loc: IrSourceLoc::default(),
    }];
    let mut body = inline_trap_ops(int_const("5"), handler);
    body.insert(
        0,
        bind(
            "$expect_trapped0",
            "Boolean",
            Some(const_of("Boolean", "false")),
            false,
            true,
        ),
    );
    let got = rules(&project(vec![func("run", vec![], body)], vec![]));
    assert!(
        !got.iter()
            .any(|r| r == "TYPE_INLINE_TRAP_REQUIRES_FALLIBLE"),
        "{got:?}"
    );
}

/// A record that is NOT thread-sendable because it holds a thread handle.
fn unsendable_record() -> IrType {
    record_typed("BadMessage", &[("handle", "Thread OF String TO Integer")])
}

#[test]
fn rejects_unsendable_thread_message_in_a_parameter() {
    let f = func(
        "run",
        vec![param("t", "Thread OF BadMessage TO Integer", None)],
        vec![ret(int_const("0"))],
    );
    expect_rule(
        &project(vec![f], vec![unsendable_record()]),
        "TYPE_THREAD_NOT_SENDABLE",
    );
}

#[test]
fn rejects_unsendable_thread_message_in_a_record_field() {
    let holder = record_typed("Holder", &[("t", "Thread OF BadMessage TO Integer")]);
    let f = func("run", vec![], vec![ret(int_const("0"))]);
    expect_rule(
        &project(vec![f], vec![unsendable_record(), holder]),
        "TYPE_THREAD_NOT_SENDABLE",
    );
}

#[test]
fn rejects_unsendable_message_sent_across_a_thread() {
    let send = IrOp::Eval {
        value: IrValue::Call {
            target: "thread.send".to_string(),
            args: vec![
                IrValue::Local("t".to_string()),
                IrValue::Local("m".to_string()),
            ],
            type_: ParameterType::Nothing,
            loc: IrSourceLoc::default(),
        },
        loc: IrSourceLoc::default(),
    };
    let f = func_returns(
        "run",
        "Nothing",
        vec![
            param("t", "Thread OF BadMessage TO Integer", None),
            param("m", "BadMessage", None),
        ],
        vec![send, ret_none()],
    );
    // Both the declared handle type and the call boundary reject.
    let got = rules(&project(vec![f], vec![unsendable_record()]));
    assert!(
        got.iter()
            .filter(|r| *r == "TYPE_THREAD_NOT_SENDABLE")
            .count()
            >= 2,
        "{got:?}"
    );
}

#[test]
fn rejects_transfer_on_a_thread_without_a_resource_plane() {
    let transfer = IrOp::Eval {
        value: IrValue::Call {
            target: crate::codegen::builtins::thread::TRANSFER_RESOURCE.to_string(),
            args: vec![
                IrValue::Local("t".to_string()),
                IrValue::Local("v".to_string()),
            ],
            type_: ParameterType::Nothing,
            loc: IrSourceLoc::default(),
        },
        loc: IrSourceLoc::default(),
    };
    let f = func_returns(
        "run",
        "Nothing",
        vec![
            param("t", "Thread OF String TO Integer", None),
            param("v", "Integer", None),
        ],
        vec![transfer, ret_none()],
    );
    expect_rule(&project(vec![f], vec![]), "TYPE_THREAD_NOT_SENDABLE");
}

/// bug-301 G4: the resource plane's `STATE T` payload crosses the boundary with
/// the resource (deep-copied into the receiver's arena), so it must be sendable
/// too — copyable + defaultable does not imply it: a record holding
/// `List OF RES fs.File` satisfies both yet carries sender-owned pointers.
#[test]
fn rejects_unsendable_resource_plane_state_payload() {
    // plan-114-A: the payload is deep-copied DATA riding the resource plane, so
    // the resource inside it is on a data plane and the rejection names the
    // plane remedy (`2-203-0138`) rather than the generic unsendable rule.
    let holder = record_typed("Holder", &[("files", "List OF RES fs.File")]);
    let f = func(
        "worker",
        vec![param(
            "t",
            "ThreadWorker OF Integer RES fs.File STATE Holder TO Integer",
            None,
        )],
        vec![ret(int_const("0"))],
    );
    expect_rule(
        &project(vec![f], vec![holder]),
        "TYPE_THREAD_RESOURCE_PLANE_REQUIRED",
    );

    // A STATE of plain sendable fields is accepted — the rule rejects the
    // unsendable payload, not stateful planes generally.
    let plain = record_typed("Holder", &[("count", "Integer"), ("label", "String")]);
    let f = func(
        "worker",
        vec![param(
            "t",
            "ThreadWorker OF Integer RES fs.File STATE Holder TO Integer",
            None,
        )],
        vec![ret(int_const("0"))],
    );
    let got = rules(&project(vec![f], vec![plain]));
    assert!(
        !got.iter()
            .any(|r| r == "TYPE_THREAD_NOT_SENDABLE" || r == "TYPE_THREAD_RESOURCE_PLANE_REQUIRED"),
        "{got:?}"
    );
}

#[test]
fn rejects_a_non_resource_on_the_resource_plane() {
    // `thread::accept` on a thread whose resource plane names `Integer`: the
    // resource plane moves only resources.
    let accept = IrOp::Eval {
        value: IrValue::Call {
            target: crate::codegen::builtins::thread::ACCEPT_RESOURCE.to_string(),
            args: vec![IrValue::Local("t".to_string())],
            type_: ParameterType::Integer,
            loc: IrSourceLoc::default(),
        },
        loc: IrSourceLoc::default(),
    };
    let f = func(
        "worker",
        vec![param(
            "t",
            "ThreadWorker OF String RES Integer TO Integer",
            None,
        )],
        vec![accept, ret(int_const("0"))],
    );
    expect_rule(&project(vec![f], vec![]), "TYPE_THREAD_NOT_SENDABLE");
}

#[test]
fn rejects_a_resource_in_the_message_plane() {
    // The data plane is resource-free (§7): a resource rides the `RES` plane.
    // plan-114-A gives that its own rule, since the remedy is nameable.
    let f = func(
        "run",
        vec![param("t", "Thread OF fs.File TO Integer", None)],
        vec![ret(int_const("0"))],
    );
    let got = rules(&project(vec![f], vec![]));
    assert!(
        got.iter()
            .any(|r| r == "TYPE_THREAD_RESOURCE_PLANE_REQUIRED"),
        "{got:?}"
    );
    // One mistake, one diagnostic: the plane rule and the sendability rule are
    // mutually exclusive, so the generic rule must NOT also fire here.
    assert!(
        !got.iter().any(|r| r == "TYPE_THREAD_NOT_SENDABLE"),
        "both rules fired for one cause: {got:?}"
    );
}

#[test]
fn accepts_a_sendable_thread_message() {
    let f = func(
        "run",
        vec![param("t", "Thread OF String TO Integer", None)],
        vec![ret(int_const("0"))],
    );
    let got = rules(&project(vec![f], vec![]));
    assert!(
        !got.iter().any(|r| r == "TYPE_THREAD_NOT_SENDABLE"),
        "{got:?}"
    );
}

// ---------------------------------------------------------------------------
// plan-114-A — the thread-unsendability CAUSE walk.
//
// `thread_unsendable_cause` is the single walk; "is it sendable" is `.is_none()`
// on it. These assert the walk's classification directly, so a future edit that
// makes it disagree with the rule it feeds is caught here rather than as a
// silently-missing diagnostic downstream.
// ---------------------------------------------------------------------------

/// Build the checker's type environment over a project fixture, so the cause
/// walk can be asked about a type directly.
fn type_env(project: &IrProject) -> super::TypeEnv {
    super::TypeEnv::build(project)
}

/// The cause the walk reports for `spelling`, given `types` are declared.
fn cause_for(types: Vec<IrType>, spelling: &str) -> Option<super::resources::Unsendable> {
    let f = func("run", vec![], vec![ret(int_const("0"))]);
    let project = project(vec![f], types);
    let env = type_env(&project);
    env.thread_unsendable_cause(
        &ParameterType::parse(spelling),
        &mut std::collections::HashSet::new(),
    )
}

#[test]
fn cause_walk_reports_a_res_marked_element_as_a_resource() {
    // `RES fs.File` in a collection: the §15.6 case. The reported leaf is the
    // `RES`-marked element itself, not the enclosing `List`.
    let cause = cause_for(vec![], "List OF RES fs.File");
    assert_eq!(
        cause,
        Some(super::resources::Unsendable::Resource(
            ParameterType::parse("RES fs.File")
        )),
        "expected the RES element as the blocking leaf"
    );
}

#[test]
fn cause_walk_reports_a_res_map_value_through_both_positions() {
    // The Map arm reports key first, then value; only the value blocks here.
    let cause = cause_for(vec![], "Map OF String TO RES fs.File");
    assert_eq!(
        cause,
        Some(super::resources::Unsendable::Resource(
            ParameterType::parse("RES fs.File")
        ))
    );
}

#[test]
fn cause_walk_accepts_a_sendable_bare_resource_but_rejects_an_unsendable_one() {
    // A bare resource nominal is judged by its registered sendability, NOT by
    // being a resource: `fs.File` is `sendable: true`
    // (src/codegen/builtins/fs/mod.rs), `process.Process` is `sendable: false`
    // (src/codegen/builtins/process/mod.rs). This is why the bare-resource
    // DATA-plane rejection cannot come from this walk — see the plane rule in
    // `check_thread_sendability`.
    //
    // Both spellings are package-qualified because that is the only form the
    // resource tables answer to: `registry().resolve_type` splits on the `.`
    // (src/codegen/registry/mod.rs:1372), so a bare `Process` resolves to
    // nothing and reads as an unknown name — which is vacuously sendable.
    assert_eq!(cause_for(vec![], "fs.File"), None, "fs.File is sendable");
    assert_eq!(
        cause_for(vec![], "process.Process"),
        Some(super::resources::Unsendable::Resource(
            ParameterType::parse("process.Process")
        )),
        "process.Process is not thread-sendable"
    );
    // The unqualified spelling is not a resource as far as the tables are
    // concerned. Pinned so the contrast above is not mistaken for a typo.
    assert_eq!(cause_for(vec![], "Process"), None);
}

#[test]
fn cause_walk_reports_func_and_thread_handle_as_other_not_resource() {
    // These are genuinely unsendable — no resource plane exists to move them
    // to — so they must NOT be classified as a plane mix-up.
    assert_eq!(
        cause_for(vec![], "FUNC(Integer) AS String"),
        Some(super::resources::Unsendable::Other(ParameterType::parse(
            "FUNC(Integer) AS String"
        )))
    );
    assert_eq!(
        cause_for(vec![], "Thread OF String TO Integer"),
        Some(super::resources::Unsendable::Other(ParameterType::parse(
            "Thread OF String TO Integer"
        )))
    );
}

#[test]
fn cause_walk_descends_into_a_record_field_and_names_the_leaf() {
    // The nested case behind `rejects_unsendable_resource_plane_state_payload`:
    // the record itself is not a resource, so the cause must come from the
    // field, and the reported leaf must be the field's element — not `Holder`.
    let holder = record_typed("Holder", &[("files", "List OF RES fs.File")]);
    assert_eq!(
        cause_for(vec![holder], "Holder"),
        Some(super::resources::Unsendable::Resource(
            ParameterType::parse("RES fs.File")
        ))
    );
}

#[test]
fn cause_walk_reports_the_first_blocking_field_left_to_right() {
    // Determinism: two blocking fields, and the FIRST is reported. Without this
    // the emitted message would depend on field-map iteration order.
    let holder = record_typed(
        "Holder",
        &[
            ("ok", "Integer"),
            ("first", "List OF RES fs.File"),
            ("second", "FUNC(Integer) AS String"),
        ],
    );
    assert_eq!(
        cause_for(vec![holder], "Holder"),
        Some(super::resources::Unsendable::Resource(
            ParameterType::parse("RES fs.File")
        ))
    );
}

#[test]
fn cause_walk_accepts_every_plain_sendable_shape() {
    // The regression guard for the refactor: the walk must still say `None`
    // for everything that crossed a boundary before it existed.
    let holder = record_typed("Holder", &[("count", "Integer"), ("label", "String")]);
    for spelling in [
        "Integer",
        "String",
        "Boolean",
        "Byte",
        "Float",
        "Fixed",
        "Money",
        "Nothing",
        "List OF Integer",
        "Set OF String",
        "Map OF String TO Integer",
        "Result OF Integer",
        "Error",
        "ErrorLoc",
        "Scalar",
        "AttributedString",
        "Holder",
    ] {
        assert_eq!(
            cause_for(vec![holder.clone()], spelling),
            None,
            "`{spelling}` must stay sendable"
        );
    }
}

#[test]
fn the_resource_plane_keeps_the_generic_unsendable_rule() {
    // plan-114-A C3: a resource plane naming a resource that is not registered
    // thread-sendable is NOT a plane mix-up — it is already on the right plane.
    // Emitting `2-203-0138` here would tell the author to move it to the plane
    // it is on, so the resource plane keeps `2-203-0063`.
    let f = func(
        "worker",
        vec![param(
            "t",
            "ThreadWorker OF Integer RES process.Process TO Integer",
            None,
        )],
        vec![ret(int_const("0"))],
    );
    let got = rules(&project(vec![f], vec![]));
    assert!(
        got.iter().any(|r| r == "TYPE_THREAD_NOT_SENDABLE"),
        "{got:?}"
    );
    assert!(
        !got.iter()
            .any(|r| r == "TYPE_THREAD_RESOURCE_PLANE_REQUIRED"),
        "the resource plane must not get the data-plane remedy: {got:?}"
    );
}

#[test]
fn func_and_thread_handle_planes_keep_the_generic_unsendable_rule() {
    // The other half of the split: a genuinely unsendable type has no resource
    // plane to be moved to, so it must keep `2-203-0063` and never get the
    // resource remedy.
    for spelling in [
        "Thread OF FUNC(Integer) AS String TO Integer",
        "Thread OF ThreadWorker OF Integer TO Integer TO Integer",
    ] {
        let f = func(
            "run",
            vec![param("t", spelling, None)],
            vec![ret(int_const("0"))],
        );
        let got = rules(&project(vec![f], vec![]));
        assert!(
            got.iter().any(|r| r == "TYPE_THREAD_NOT_SENDABLE"),
            "`{spelling}`: {got:?}"
        );
        assert!(
            !got.iter()
                .any(|r| r == "TYPE_THREAD_RESOURCE_PLANE_REQUIRED"),
            "`{spelling}` is not a plane mix-up: {got:?}"
        );
    }
}

#[test]
fn cause_walk_terminates_on_a_self_referential_record() {
    // The cycle guard: `seen` must still stop the walk now that the `all(..)`
    // fold became a `find_map`.
    let node = record_typed("Node", &[("next", "Node"), ("label", "String")]);
    assert_eq!(cause_for(vec![node], "Node"), None);
}

// ---------------------------------------------------------------------------
// plan-107-B — the general semantic cluster's package-path twins.
// ---------------------------------------------------------------------------

#[test]
fn warns_dead_handler_on_an_infallible_inline_builtin() {
    // `len(xs) TRAP(e) … END TRAP`: `len` cannot fail, so the handler is dead
    // code — an advisory warning (plan-26-A), not an error.
    let scrutinee = IrValue::CallResult {
        target: "len".to_string(),
        args: vec![IrValue::Local("xs".to_string())],
        type_: ParameterType::Integer,
        loc: IrSourceLoc::default(),
    };
    let mut body = inline_trap_ops(scrutinee, vec![recover_zero()]);
    body.insert(
        0,
        bind(
            "xs",
            "List OF Integer",
            Some(IrValue::ListLiteral {
                type_: ParameterType::list_of(ParameterType::Integer),
                values: vec![int_const("1")],
            }),
            true,
            false,
        ),
    );
    expect_rule(
        &project(vec![func("run", vec![], body)], vec![]),
        "TYPE_INLINE_TRAP_DEAD_HANDLER",
    );
}

#[test]
fn rejects_normal_flow_reaching_the_trap() {
    // The handler returns, but the body before the TRAP falls through into it.
    let body = vec![
        IrOp::Eval {
            value: int_const("1"),
            loc: IrSourceLoc::default(),
        },
        IrOp::Trap {
            name: "e".to_string(),
            body: vec![ret(int_const("0"))],
            loc: IrSourceLoc::default(),
        },
    ];
    let f = func("run", vec![], body);
    let got = rules(&project(vec![f], vec![]));
    assert!(got.iter().any(|r| r == "TYPE_TRAP_FALLTHROUGH"), "{got:?}");
}

#[test]
fn a_stray_recover_counts_as_diverging() {
    // `RECOVER 0` inside a function-level TRAP is itself an error, but the front
    // end's flow analysis treats it as diverging, so the handler is not ALSO
    // reported as falling through. The stray RECOVER lowers to a
    // `$recover_stray` bind, which the divergence predicate honours.
    let body = vec![
        ret(int_const("1")),
        IrOp::Trap {
            name: "e".to_string(),
            body: vec![bind(
                "$recover_stray0",
                "Unknown",
                Some(int_const("0")),
                false,
                false,
            )],
            loc: IrSourceLoc::default(),
        },
    ];
    let f = func("run", vec![], body);
    let got = rules(&project(vec![f], vec![]));
    assert!(!got.iter().any(|r| r == "TYPE_TRAP_FALLTHROUGH"), "{got:?}");
}

#[test]
fn rejects_a_thread_handle_as_a_list_element() {
    let f = func(
        "run",
        vec![param("xs", "List OF Thread OF Integer TO Integer", None)],
        vec![ret(int_const("0"))],
    );
    expect_rule(
        &project(vec![f], vec![]),
        "TYPE_COLLECTION_OWNERSHIP_VIOLATION",
    );
}

#[test]
fn rejects_a_thread_handle_as_a_map_value() {
    let f = func(
        "run",
        vec![param(
            "m",
            "Map OF String TO Thread OF Integer TO Integer",
            None,
        )],
        vec![ret(int_const("0"))],
    );
    expect_rule(
        &project(vec![f], vec![]),
        "TYPE_COLLECTION_OWNERSHIP_VIOLATION",
    );
}

#[test]
fn rejects_a_resource_in_a_set_literal() {
    let literal = IrValue::SetLiteral {
        type_: ParameterType::set_of(ParameterType::named("fs.File")),
        values: vec![],
    };
    let body = vec![
        bind("s", "Set OF fs.File", Some(literal), true, false),
        ret(int_const("0")),
    ];
    let got = rules(&project(vec![func("run", vec![], body)], vec![]));
    // Once for the declared type, once for the literal — as the front end did.
    assert_eq!(
        got.iter()
            .filter(|r| *r == "TYPE_COLLECTION_OWNERSHIP_VIOLATION")
            .count(),
        2,
        "{got:?}"
    );
}

#[test]
fn rejects_a_thread_carrying_union_as_a_map_key() {
    // A union whose variant holds a thread handle: the front end's walk reached
    // the variant's fields; verify's predicate used to stop at the union name.
    let mut holder = record_typed("Holder", &[("t", "Thread OF Integer TO Integer")]);
    holder.visibility = "public".to_string();
    let plain = record_typed("Plain", &[("n", "Integer")]);
    let union = IrType {
        kind: "union".to_string(),
        visibility: "export".to_string(),
        name: "Either".to_string(),
        fields: vec![],
        includes: vec![],
        variants: vec![
            IrVariant {
                name: "Holder".to_string(),
                fields: vec![IrField {
                    visibility: None,
                    name: "t".to_string(),
                    type_: ParameterType::parse("Thread OF Integer TO Integer"),
                    loc: IrSourceLoc::default(),
                }],
                loc: IrSourceLoc::default(),
            },
            IrVariant {
                name: "Plain".to_string(),
                fields: vec![IrField {
                    visibility: None,
                    name: "n".to_string(),
                    type_: ParameterType::Integer,
                    loc: IrSourceLoc::default(),
                }],
                loc: IrSourceLoc::default(),
            },
        ],
        members: vec![],
        loc: IrSourceLoc::default(),
        file: String::new(),
    };
    let f = func(
        "run",
        vec![param("m", "Map OF Either TO Integer", None)],
        vec![ret(int_const("0"))],
    );
    expect_rule(
        &project(vec![f], vec![holder, plain, union]),
        "TYPE_COLLECTION_OWNERSHIP_VIOLATION",
    );
}

/// `LET f AS FUNC(Integer) AS Integer = LAMBDA(v AS Integer) -> v + <capture>`
/// as lowering shapes it: the closure value binds the lambda by name with its
/// capture list; `by_ref` captures arrive as `LocalRef`.
fn closure_bind(captures: Vec<IrValue>) -> IrOp {
    bind(
        "f",
        "FUNC(Integer) AS Integer",
        Some(IrValue::Closure {
            name: "$lambda0".to_string(),
            type_: ParameterType::parse("FUNC(Integer) AS Integer"),
            captures,
        }),
        true,
        false,
    )
}

#[test]
fn rejects_a_by_value_capture_of_a_mut_local() {
    let body = vec![
        bind("offset", "Integer", Some(int_const("1")), true, true),
        closure_bind(vec![IrValue::Local("offset".to_string())]),
        ret(int_const("0")),
    ];
    expect_rule(
        &project(vec![func("run", vec![], body)], vec![]),
        "TYPE_LAMBDA_CAPTURE_UNSUPPORTED",
    );
}

#[test]
fn accepts_a_by_ref_capture_of_a_mut_local() {
    // The compiler-proven non-escaping position (`forEach`'s action) captures a
    // MUT local by slot reference — lowering's `LocalRef`.
    let body = vec![
        bind("total", "Integer", Some(int_const("0")), true, true),
        closure_bind(vec![IrValue::LocalRef {
            name: "total".to_string(),
            type_: ParameterType::Integer,
        }]),
        ret(int_const("0")),
    ];
    let got = rules(&project(vec![func("run", vec![], body)], vec![]));
    assert!(
        !got.iter().any(|r| r == "TYPE_LAMBDA_CAPTURE_UNSUPPORTED"),
        "{got:?}"
    );
}

#[test]
fn rejects_a_resource_capture_in_either_shape() {
    for capture in [
        IrValue::Local("handle".to_string()),
        IrValue::LocalRef {
            name: "handle".to_string(),
            type_: ParameterType::named("fs.File"),
        },
    ] {
        let body = vec![
            bind("handle", "fs.File", Some(int_const("0")), true, true),
            closure_bind(vec![capture]),
            ret(int_const("0")),
        ];
        expect_rule(
            &project(vec![func("run", vec![], body)], vec![]),
            "TYPE_LAMBDA_CAPTURE_UNSUPPORTED",
        );
    }
}

#[test]
fn rejects_a_non_copyable_capture() {
    // A thread handle is neither a resource nor copyable.
    let body = vec![
        bind(
            "t",
            "Thread OF Integer TO Integer",
            Some(int_const("0")),
            true,
            false,
        ),
        closure_bind(vec![IrValue::Local("t".to_string())]),
        ret(int_const("0")),
    ];
    expect_rule(
        &project(vec![func("run", vec![], body)], vec![]),
        "TYPE_LAMBDA_CAPTURE_UNSUPPORTED",
    );
}

#[test]
fn accepts_a_copyable_immutable_capture() {
    let body = vec![
        bind("offset", "Integer", Some(int_const("1")), true, false),
        closure_bind(vec![IrValue::Local("offset".to_string())]),
        ret(int_const("0")),
    ];
    let got = rules(&project(vec![func("run", vec![], body)], vec![]));
    assert!(
        !got.iter().any(|r| r == "TYPE_LAMBDA_CAPTURE_UNSUPPORTED"),
        "{got:?}"
    );
}

// ---------------------------------------------------------------------------
// plan-107-C — package-path twins of the LINK sub-forms verify lacked before
// the front end's rules moved here. Each mutates one thing about a well-formed
// wrapper over `CSTRUCT S AS Rec { a CInt32 }`.
// ---------------------------------------------------------------------------

/// A project with `CSTRUCT S AS Rec { a CInt32 }`, record `Rec { a AS Integer }`
/// and one wrapper whose ABI has a single struct slot `s` of `direction`.
fn struct_slot_project(
    direction: crate::ir::AbiDirection,
) -> (IrProject, crate::ir::IrLinkFunction) {
    let mut p = project_with_cstructs(vec![cstruct("S", &[("a", "CInt32")])]);
    p.types = vec![record_typed("Rec", &[("a", "Integer")])];
    let mut f = link_fn();
    f.params = vec![];
    f.abi_slots = vec![crate::ir::IrAbiSlot {
        name: "s".to_string(),
        ctype: crate::types::ParameterType::parse("S"),
        direction,
    }];
    (p, f)
}

fn bind_in(slot: &str, fields: &[(&str, Option<&str>, Option<i64>)]) -> crate::ir::IrBindIn {
    crate::ir::IrBindIn {
        slot: slot.to_string(),
        fields: fields
            .iter()
            .map(|(name, param, literal)| crate::ir::IrBindInField {
                name: (*name).to_string(),
                param: param.map(str::to_string),
                literal: *literal,
            })
            .collect(),
    }
}

#[test]
fn rejects_inout_on_a_non_cstruct_slot() {
    // A scalar slot is either a C argument or a produced value; INOUT means
    // nothing for it.
    let mut p = project(vec![func_returns("run", "Nothing", vec![], vec![])], vec![]);
    let mut f = link_fn();
    f.abi_slots[0].direction = crate::ir::AbiDirection::InOut;
    p.link_functions = vec![f];
    expect_rule(&p, "NATIVE_ABI_UNKNOWN_CTYPE");
}

#[test]
fn rejects_returning_an_in_struct_slot() {
    // An IN slot is zeroed and never read back, so `RETURN s` names nothing.
    let (mut p, mut f) = struct_slot_project(crate::ir::AbiDirection::In);
    f.bind_in = vec![bind_in("s", &[("a", None, Some(1))])];
    f.return_type = crate::types::ParameterType::parse("Rec");
    f.result = Some(crate::ir::IrLinkExpr::Var("s".to_string()));
    p.link_functions = vec![f];
    expect_rule(&p, "NATIVE_ABI_RESULT_MARKER");
}

#[test]
fn rejects_returning_a_struct_slot_as_another_type() {
    // A wrapper that returns a struct slot must declare its mapped record.
    let (mut p, mut f) = struct_slot_project(crate::ir::AbiDirection::Out);
    f.return_type = crate::types::ParameterType::parse("Integer");
    f.result = Some(crate::ir::IrLinkExpr::Var("s".to_string()));
    p.link_functions = vec![f];
    expect_rule(&p, "NATIVE_STRUCT_FIELD_MISMATCH");
}

#[test]
fn rejects_bind_in_on_an_out_slot() {
    let (mut p, mut f) = struct_slot_project(crate::ir::AbiDirection::Out);
    f.bind_in = vec![bind_in("s", &[("a", None, Some(1))])];
    f.result = Some(crate::ir::IrLinkExpr::Var("s".to_string()));
    f.return_type = crate::types::ParameterType::parse("Rec");
    p.link_functions = vec![f];
    expect_rule(&p, "NATIVE_BIND_IN_INVALID");
}

#[test]
fn rejects_bind_in_setting_a_field_twice() {
    let (mut p, mut f) = struct_slot_project(crate::ir::AbiDirection::In);
    f.bind_in = vec![bind_in("s", &[("a", None, Some(1)), ("a", None, Some(2))])];
    p.link_functions = vec![f];
    expect_rule(&p, "NATIVE_BIND_IN_INVALID");
}

#[test]
fn rejects_bind_in_field_bound_to_nothing() {
    // Lowering represents an unmarshalable value as neither param nor literal.
    let (mut p, mut f) = struct_slot_project(crate::ir::AbiDirection::In);
    f.bind_in = vec![bind_in("s", &[("a", None, None)])];
    p.link_functions = vec![f];
    expect_rule(&p, "NATIVE_BIND_IN_INVALID");
}

#[test]
fn rejects_an_unbound_in_struct_slot() {
    // An IN struct slot with neither a parameter nor a BIND IN block is unbound
    // — the front end's second slot pass never exempted struct slots.
    let (mut p, f) = struct_slot_project(crate::ir::AbiDirection::In);
    p.link_functions = vec![f];
    expect_rule(&p, "NATIVE_ABI_UNBOUND_SLOT");
}

#[test]
fn accepts_a_bound_in_struct_slot() {
    let (mut p, mut f) = struct_slot_project(crate::ir::AbiDirection::In);
    f.bind_in = vec![bind_in("s", &[("a", None, Some(1))])];
    p.link_functions = vec![f];
    let got = rules(&p);
    assert!(!got.iter().any(|r| r.starts_with("NATIVE_")), "{got:?}");
}

#[test]
fn accepts_a_body_that_returns_before_its_trap() {
    let body = vec![
        ret(int_const("1")),
        IrOp::Trap {
            name: "e".to_string(),
            body: vec![ret(int_const("0"))],
            loc: IrSourceLoc::default(),
        },
    ];
    let f = func("run", vec![], body);
    let got = rules(&project(vec![f], vec![]));
    assert!(!got.iter().any(|r| r == "TYPE_TRAP_FALLTHROUGH"), "{got:?}");
}

/// bug-483 sub-issue B: the compiler-owned records a program may neither
/// construct nor `WITH`-update are recognised by NAME, and bug-480 Phase 4b made
/// a builtin value type's declared identity package-qualified. Matching only the
/// bare leaf left the rule looking for a spelling nothing produces any more, so
/// `net::Address["1.2.3.4", 80]` compiled and ran — silently handing a program a
/// record whose layout only the runtime helpers are allowed to write.
///
/// Both spellings must be refused: a source `AS net::Address` resolves to the
/// qualified id, while a record FIELD type still arrives bare.
#[test]
fn read_only_records_are_refused_under_either_spelling() {
    for name in [
        "Address",
        "net.Address",
        "AudioDevice",
        "audio.AudioDevice",
        "TermSize",
        "term.TermSize",
    ] {
        assert!(
            super::read_only_record_type(&ParameterType::declared(name)),
            "`{name}` must stay a read-only compiler-owned record (bug-483)"
        );
    }
    // An ordinary record is still constructible.
    assert!(!super::read_only_record_type(&ParameterType::declared(
        "net.Url"
    )));
    // plan-122-F: `term::TermColor` was a read-only compiler-owned record and is
    // retired; `term::getForeground`/`getBackground` return a `color::Color`, which
    // is an ORDINARY VALUE RECORD a program may build and `WITH`-update. Asserting
    // both spellings are constructible is what keeps the retirement real — a future
    // change that re-added `color.Color` to the read-only set would silently take
    // away a capability this letter deliberately granted.
    for name in ["Color", "color.Color"] {
        assert!(
            !super::read_only_record_type(&ParameterType::declared(name)),
            "`{name}` is an ordinary value record and must stay constructible"
        );
    }
}
