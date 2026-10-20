//! plan-176-G: `ir::verify::tests` — state agreement for bindings, cstruct declarations, LINK
//! native arms and bind state, and link expression slots and pins. The shared builders and
//! fixtures live in `verify/tests/mod.rs`.

use super::*;

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
    // Binding a bare resource parameter under a concrete STATE — an unprovable
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
