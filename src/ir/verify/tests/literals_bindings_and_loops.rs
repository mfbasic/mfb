//! plan-176-G: `ir::verify::tests` — literal ranges, scalar literals and bindings,
//! globals/returns/exit, loop exits and conditions, and for loops. The shared builders and
//! fixtures live in `verify/tests/mod.rs`.

use super::*;

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

/// bug-813: the Fixed literal range check decides on the converter's signed,
/// rounded carrier, not on a binary64 approximation of the spelling.
fn fixed_bind(value: IrValue) -> IrProject {
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind("x", "Fixed", Some(value), true, false)],
    );
    project(vec![f], vec![])
}

fn negated_fixed(magnitude: &str) -> IrValue {
    unary(UnaryOp::Negate, const_of("Fixed", magnitude), "Fixed")
}

#[test]
fn accepts_fixed_exact_maximum_and_minimum() {
    // binary64 rounds this spelling to 2147483648.0; its carrier is i64::MAX.
    accept(&fixed_bind(const_of(
        "Fixed",
        "2147483647.99999999976716935634613037109375",
    )));
    accept(&fixed_bind(const_of(
        "Fixed",
        "2147483647.999999999883584678173065185546874",
    )));
    accept(&fixed_bind(negated_fixed("2147483648")));
    accept(&fixed_bind(negated_fixed(
        "2147483648.000000000116415321826934814453124",
    )));
    // The lowering fold's signed minimum, and a tiny extreme exponent (0).
    accept(&fixed_bind(const_of("Fixed", "-2147483648.0")));
    accept(&fixed_bind(const_of("Fixed", "1e-1000000000")));
}

#[test]
fn rejects_fixed_literal_rounding_past_either_end() {
    // Exact half-quantum ties beyond each end round away from zero, out of range.
    expect_rule(
        &fixed_bind(const_of(
            "Fixed",
            "2147483647.999999999883584678173065185546875",
        )),
        "TYPE_FIXED_LITERAL_OVERFLOW",
    );
    // binary64 rounds this magnitude to 2147483648.0, which the old check
    // accepted; the carrier is one below i64::MIN.
    expect_rule(
        &fixed_bind(negated_fixed(
            "2147483648.000000000116415321826934814453125",
        )),
        "TYPE_FIXED_LITERAL_UNDERFLOW",
    );
    // A signed Const (decoded package IR) is judged by its own sign.
    expect_rule(
        &fixed_bind(const_of("Fixed", "-3000000000")),
        "TYPE_FIXED_LITERAL_UNDERFLOW",
    );
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

// --- return rules --------------------------------------------------------

#[test]
fn rejects_return_mismatch() {
    let body = vec![ret(const_of("String", "no"))];
    expect_rule(
        &project(vec![func_returns("run", "Integer", vec![], body)], vec![]),
        "TYPE_RETURN_MISMATCH",
    );
}

#[test]
fn accepts_a_nothing_call_in_statement_position() {
    let s = func_returns("doit", "Nothing", vec![], vec![]);
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

// The exit code's type and host-range checks are source-level now (the
// `os::exit` signature and `OS_EXIT_CODE_OUT_OF_RANGE` in `ir::shape`), so the
// verifier has no rule left to reject an out-of-range or non-Integer code.
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
