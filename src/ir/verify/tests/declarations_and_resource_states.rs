//! plan-176-G: `ir::verify::tests` — compat deep arms, type declarations and call arguments,
//! the resource axis and use after move, resource aliases, and state agreement for calls. The
//! shared builders and fixtures live in `verify/tests/mod.rs`.

use super::*;

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
        type_: crate::types::ParameterType::parse("#Result OF String"),
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
        "#Result OF Integer",
        Some(IrValue::Local("b".to_string())),
        true,
        false,
    )];
    let f = func_returns(
        "run",
        "Nothing",
        vec![param("b", "#Result OF Byte", None)],
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

/// bug-674: a resource parameter's `STATE` is held to the same rule as a binding's.
#[test]
fn rejects_param_state_type_not_defaultable() {
    let f = func_returns(
        "run",
        "Nothing",
        vec![param("h", "fs.File STATE Shape", None)],
        vec![],
    );
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
    // plan-59-E: was `rejects_non_owner_resource_close`. A resource parameter used to
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
