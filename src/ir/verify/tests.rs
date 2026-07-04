use super::check;
use crate::ir::{
    IrField, IrFunction, IrOp, IrParam, IrProject, IrSourceLoc, IrType, IrValue, IrVariant,
    ProjectDocs,
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
    assert!(err.contains("member `x`"), "{err}");
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
    let callee = func("helper", vec![param("a", "Integer", None)], vec![]);
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
    assert!(err.contains("call to `helper`"), "{err}");
}

#[test]
fn accepts_call_omitting_defaulted_argument() {
    let callee = func(
        "helper",
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
    // Builtins / native calls are not in the internal function table.
    let body = vec![IrOp::Return {
        value: Some(IrValue::Call {
            target: "io.print".to_string(),
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
    assert!(err.contains("constructor `Point`"), "{err}");
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
    let f = func("run", vec![], body);
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
