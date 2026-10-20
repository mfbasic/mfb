//! plan-176-G: `ir::verify::tests` — match exhaustiveness, match patterns and constructors,
//! with-update and literals, value literals and members, and money and set elements. The shared
//! builders and fixtures live in `verify/tests/mod.rs`.

use super::*;

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

// bug-945: `Ok` is an ordinary user name. A union member named `Ok` is matched
// like any member, with no reserved-name rule in the way.
#[test]
fn accepts_user_union_member_named_ok() {
    let m = IrOp::Match {
        value: IrValue::Local("o".to_string()),
        cases: vec![
            IrMatchCase {
                pattern: IrMatchPattern::Value(IrValue::Local("Ok".to_string())),
                guard: None,
                body: vec![ret_none()],
                loc: IrSourceLoc::default(),
            },
            IrMatchCase {
                pattern: IrMatchPattern::Value(IrValue::Local("Bad".to_string())),
                guard: None,
                body: vec![ret_none()],
                loc: IrSourceLoc::default(),
            },
        ],
        loc: IrSourceLoc::default(),
    };
    let f = func_returns("run", "Nothing", vec![param("o", "Outcome", None)], vec![m]);
    accept(&project(
        vec![f],
        vec![
            union("Outcome", &["Ok", "Bad"]),
            record("Ok", &["v"]),
            record("Bad", &["why"]),
        ],
    ));
}

// bug-945: `CASE Error(e)` names the built-in `Error` record, so against a union
// that does not carry it, it is the ordinary not-a-member mismatch.
#[test]
fn rejects_case_error_not_a_union_member() {
    let m = IrOp::Match {
        value: IrValue::Local("s".to_string()),
        cases: vec![
            IrMatchCase {
                pattern: IrMatchPattern::Value(IrValue::Local("Error".to_string())),
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
        "TYPE_MATCH_PATTERN_MISMATCH",
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

// bug-945: a user record named `Ok` constructs like any record.
#[test]
fn accepts_construct_user_record_named_ok() {
    let body = vec![ret(IrValue::Constructor {
        type_: crate::types::ParameterType::parse("Ok"),
        args: vec![int_const("1")],
    })];
    let f = func_returns("run", "Ok", vec![], body);
    accept(&project(vec![f], vec![record("Ok", &["v"])]));
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
