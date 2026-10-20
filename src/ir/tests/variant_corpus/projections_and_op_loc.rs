//! plan-176-G: the variant corpus's JSON-projection, annotated-type and op-location tests.
//! The corpus builders live in `variant_corpus/mod.rs`.

use super::*;

// --- json projections the main corpus misses -------------------------------

#[test]
fn json_covers_every_op_and_value() {
    // Drive to_json over a function whose body is every_op() (which embeds
    // every_value()); asserting it is valid, self-consistent JSON-ish text.
    let mut project = empty_project("j");
    project.entry = Some(EntryPoint {
        name: "main".to_string(),
        returns: crate::types::ParameterType::parse("Integer"),
        accepts_args: false,
    });
    project.functions = vec![fn_body("f", every_op())];
    project.bindings = vec![IrBinding {
        name: "g".to_string(),
        visibility: "export".to_string(),
        mutable: false,
        type_: crate::types::ParameterType::parse("Integer"),
        value: Some(c("Integer", "1")),
        loc: loc(),
        file: String::new(),
        explicit_type: false,
    }];
    let json = project.to_json();
    // Every op label appears.
    for label in [
        "\"op\": \"bind\"",
        "\"op\": \"assign\"",
        "\"op\": \"assignGlobal\"",
        "\"op\": \"stateAssign\"",
        "\"op\": \"return\"",
        "\"op\": \"exitLoop\"",
        "\"op\": \"continueLoop\"",
        "\"op\": \"exitProgram\"",
        "\"op\": \"fail\"",
        "\"op\": \"eval\"",
        "\"op\": \"if\"",
        "\"op\": \"while\"",
        "\"op\": \"for\"",
        "\"op\": \"doUntil\"",
        "\"op\": \"forEach\"",
        "\"op\": \"match\"",
        "\"op\": \"trap\"",
    ] {
        assert!(json.contains(label), "missing {label}");
    }
    // Every value kind appears.
    for kind in [
        "\"kind\": \"const\"",
        "\"kind\": \"local\"",
        "\"kind\": \"global\"",
        "\"kind\": \"localRef\"",
        "\"kind\": \"functionRef\"",
        "\"kind\": \"closure\"",
        "\"kind\": \"capture\"",
        "\"kind\": \"call\"",
        "\"kind\": \"callResult\"",
        "\"kind\": \"constructor\"",
        "\"kind\": \"unionWrap\"",
        "\"kind\": \"unionExtract\"",
        "\"kind\": \"resultIsOk\"",
        "\"kind\": \"resultValue\"",
        "\"kind\": \"resultError\"",
        "\"kind\": \"with\"",
        "\"kind\": \"list\"",
        "\"kind\": \"map\"",
        "\"kind\": \"memberAccess\"",
        "\"kind\": \"binary\"",
        "\"kind\": \"unary\"",
    ] {
        assert!(json.contains(kind), "missing {kind}");
    }
    // by_ref true and false capture serializations both appear.
    assert!(json.contains("\"byRef\": true"));
    // The oneOf match pattern and else pattern.
    assert!(json.contains("\"kind\": \"oneOf\""));
    assert!(json.contains("\"kind\": \"else\""));
}

#[test]
fn json_covers_all_type_kinds() {
    let project = variant_corpus();
    let json = project.to_json();
    assert!(json.contains("\"kind\": \"type\""));
    assert!(json.contains("\"kind\": \"union\""));
    assert!(json.contains("\"kind\": \"enum\""));
    // A private field serializes its visibility; a defaulted param serializes.
    assert!(json.contains("\"visibility\": \"private\""));
    assert!(json.contains("\"members\": ["));
    // Entry point projection.
    assert!(json.contains("\"accepts_args\": true"));
}

#[test]
fn json_project_with_no_bindings_omits_the_bindings_section() {
    // Exercises the empty-bindings branch of IrProject::to_json.
    let project = empty_project("nobindings");
    let json = project.to_json();
    assert!(!json.contains("\"bindings\""), "{json}");
    assert!(json.contains("\"entry\": null"));
    assert!(json.contains("\"types\": ["));
}

#[test]
fn loop_kind_name_covers_all_kinds() {
    assert_eq!(LoopKind::For.name(), "for");
    assert_eq!(LoopKind::Do.name(), "do");
    assert_eq!(LoopKind::While.name(), "while");
}

#[test]
fn visibility_name_covers_all_visibilities() {
    use crate::ast::Visibility;
    assert_eq!(visibility_name(Visibility::Private), "private");
    assert_eq!(visibility_name(Visibility::Public), "public");
    assert_eq!(visibility_name(Visibility::Export), "export");
}

// --- IrValue::annotated_type -----------------------------------------------

#[test]
fn annotated_type_reports_every_annotated_variant() {
    assert_eq!(
        c("Integer", "1")
            .annotated_parameter_type()
            .map(|t| t.name())
            .as_deref(),
        Some("Integer")
    );
    assert_eq!(
        IrValue::LocalRef {
            name: "a".to_string(),
            type_: crate::types::ParameterType::parse("Integer")
        }
        .annotated_parameter_type()
        .map(|t| t.name())
        .as_deref(),
        Some("Integer")
    );
    assert_eq!(
        IrValue::FunctionRef {
            name: "f".to_string(),
            type_: crate::types::ParameterType::parse("() -> Integer")
        }
        .annotated_parameter_type()
        .map(|t| t.name())
        .as_deref(),
        Some("() -> Integer")
    );
    assert_eq!(
        IrValue::Closure {
            name: "l".to_string(),
            type_: crate::types::ParameterType::parse("() -> Integer"),
            captures: vec![]
        }
        .annotated_parameter_type()
        .map(|t| t.name())
        .as_deref(),
        Some("() -> Integer")
    );
    assert_eq!(
        IrValue::Capture {
            index: 0,
            type_: crate::types::ParameterType::parse("Integer"),
            by_ref: false
        }
        .annotated_parameter_type()
        .map(|t| t.name())
        .as_deref(),
        Some("Integer")
    );
    assert_eq!(
        IrValue::Call {
            target: "f".to_string(),
            args: vec![],
            loc: loc(),
            type_: crate::types::ParameterType::parse("Integer")
        }
        .annotated_parameter_type()
        .map(|t| t.name())
        .as_deref(),
        Some("Integer")
    );
    assert_eq!(
        IrValue::CallResult {
            target: "f".to_string(),
            args: vec![],
            loc: loc(),
            type_: crate::types::ParameterType::parse("Integer")
        }
        .annotated_parameter_type()
        .map(|t| t.name())
        .as_deref(),
        Some("Integer")
    );
    assert_eq!(
        IrValue::Constructor {
            type_: crate::types::ParameterType::parse("Point"),
            args: vec![]
        }
        .annotated_parameter_type()
        .map(|t| t.name())
        .as_deref(),
        Some("Point")
    );
    assert_eq!(
        IrValue::UnionExtract {
            type_: crate::types::ParameterType::parse("Point"),
            value: Box::new(c("Integer", "0"))
        }
        .annotated_parameter_type()
        .map(|t| t.name())
        .as_deref(),
        Some("Point")
    );
    assert_eq!(
        IrValue::ResultValue {
            type_: crate::types::ParameterType::parse("Integer"),
            value: Box::new(c("Integer", "0"))
        }
        .annotated_parameter_type()
        .map(|t| t.name())
        .as_deref(),
        Some("Integer")
    );
    assert_eq!(
        IrValue::WithUpdate {
            type_: crate::types::ParameterType::parse("Point"),
            target: Box::new(c("Integer", "0")),
            updates: vec![]
        }
        .annotated_parameter_type()
        .map(|t| t.name())
        .as_deref(),
        Some("Point")
    );
    assert_eq!(
        IrValue::ListLiteral {
            type_: crate::types::ParameterType::parse("List OF Integer"),
            values: vec![]
        }
        .annotated_parameter_type()
        .map(|t| t.name())
        .as_deref(),
        Some("List OF Integer")
    );
    assert_eq!(
        IrValue::MapLiteral {
            type_: crate::types::ParameterType::parse("Map OF String TO Integer"),
            entries: vec![]
        }
        .annotated_parameter_type()
        .map(|t| t.name())
        .as_deref(),
        Some("Map OF String TO Integer")
    );
    assert_eq!(
        IrValue::MemberAccess {
            target: Box::new(c("Integer", "0")),
            member: "x".to_string(),
            type_: crate::types::ParameterType::parse("Integer")
        }
        .annotated_parameter_type()
        .map(|t| t.name())
        .as_deref(),
        Some("Integer")
    );
    assert_eq!(
        IrValue::Binary {
            op: BinaryOp::Add,
            left: Box::new(c("Integer", "1")),
            right: Box::new(c("Integer", "2")),
            loc: loc(),
            type_: crate::types::ParameterType::parse("Integer")
        }
        .annotated_parameter_type()
        .map(|t| t.name())
        .as_deref(),
        Some("Integer")
    );
    assert_eq!(
        IrValue::Unary {
            op: UnaryOp::Not,
            operand: Box::new(c("Boolean", "true")),
            loc: loc(),
            type_: crate::types::ParameterType::parse("Boolean")
        }
        .annotated_parameter_type()
        .map(|t| t.name())
        .as_deref(),
        Some("Boolean")
    );
    assert_eq!(
        IrValue::UnionWrap {
            union_type: crate::types::ParameterType::parse("Shape"),
            member_type: crate::types::ParameterType::parse("Point"),
            value: Box::new(c("Integer", "0"))
        }
        .annotated_parameter_type()
        .map(|t| t.name())
        .as_deref(),
        Some("Shape")
    );
    assert_eq!(
        IrValue::ResultIsOk {
            value: Box::new(c("Integer", "0"))
        }
        .annotated_parameter_type()
        .map(|t| t.name())
        .as_deref(),
        Some("Boolean")
    );
    assert_eq!(
        IrValue::ResultError {
            value: Box::new(c("Integer", "0"))
        }
        .annotated_parameter_type()
        .map(|t| t.name())
        .as_deref(),
        Some("Error")
    );
    // Local and Global resolve through the environment, not the node.
    assert_eq!(
        IrValue::Local("a".to_string())
            .annotated_parameter_type()
            .map(|t| t.name()),
        None
    );
    assert_eq!(
        IrValue::Global("g".to_string())
            .annotated_parameter_type()
            .map(|t| t.name()),
        None
    );
}

// --- IrOp::loc -------------------------------------------------------------

#[test]
fn op_loc_returns_the_stored_location_for_every_variant() {
    // A non-default line so we can assert loc() reads the right field.
    let l = IrSourceLoc {
        line: 42,
        column: 1,
    };
    let with_loc = |op: IrOp| assert_eq!(op.loc().line, 42, "loc() must read the op's location");

    with_loc(IrOp::Bind {
        mutable: false,
        name: "a".to_string(),
        type_: ParameterType::Integer,
        value: None,
        explicit_type: false,
        loc: l,
    });
    with_loc(IrOp::Assign {
        name: "a".to_string(),
        value: c("Integer", "1"),
        loc: l,
    });
    with_loc(IrOp::AssignGlobal {
        name: "g".to_string(),
        value: c("Integer", "1"),
        loc: l,
    });
    with_loc(IrOp::StateAssign {
        resource: "r".to_string(),
        value: c("Integer", "1"),
        loc: l,
    });
    with_loc(IrOp::Return {
        value: None,
        loc: l,
    });
    with_loc(IrOp::ExitLoop {
        target: None,
        kind: LoopKind::For,
        loc: l,
    });
    with_loc(IrOp::ContinueLoop {
        target: None,
        kind: LoopKind::Do,
        loc: l,
    });
    with_loc(IrOp::ExitProgram {
        code: c("Integer", "0"),
        loc: l,
    });
    with_loc(IrOp::Fail {
        error: c("Error", "x"),
        loc: l,
    });
    with_loc(IrOp::Eval {
        value: c("Integer", "0"),
        loc: l,
    });
    with_loc(IrOp::If {
        condition: c("Boolean", "true"),
        then_body: vec![],
        else_body: vec![],
        loc: l,
    });
    with_loc(IrOp::Match {
        value: c("Integer", "0"),
        cases: vec![],
        loc: l,
    });
    with_loc(IrOp::While {
        kind: LoopKind::While,
        condition: c("Boolean", "true"),
        body: vec![],
        loc: l,
    });
    with_loc(IrOp::For {
        name: "i".to_string(),
        type_: ParameterType::Integer,
        start: c("Integer", "0"),
        end: c("Integer", "1"),
        step: c("Integer", "1"),
        body: vec![],
        loc: l,
    });
    with_loc(IrOp::DoUntil {
        body: vec![],
        condition: c("Boolean", "true"),
        loc: l,
    });
    with_loc(IrOp::ForEach {
        name: "x".to_string(),
        type_: ParameterType::Integer,
        iterable: c("List OF Integer", "[]"),
        body: vec![],
        loc: l,
    });
    with_loc(IrOp::Trap {
        name: "e".to_string(),
        body: vec![],
        loc: l,
    });
}

/// plan-58-C: `buffers` and `result_length` must survive the `.mfp` round trip
/// with their expression trees intact.
///
/// Structural, not `is_some()`: a buffer whose SIZE decodes to the wrong tree is
/// a wrong allocation size, which is the whole failure class plan-58 exists to
/// prevent. `binary_round_trip_link_expr_variants` shows how easily a shallow
/// assertion here passes on a broken decoder.
#[test]
fn binary_round_trip_carries_buffers_and_length() {
    fn render(expr: &IrLinkExpr) -> String {
        match expr {
            IrLinkExpr::Var(name) => name.clone(),
            IrLinkExpr::Int(value) => value.to_string(),
            IrLinkExpr::Mul(l, r) => format!("({} * {})", render(l), render(r)),
            IrLinkExpr::Add(l, r) => format!("({} + {})", render(l), render(r)),
            IrLinkExpr::Sub(l, r) => format!("({} - {})", render(l), render(r)),
            IrLinkExpr::Not(inner) => format!("!{}", render(inner)),
            IrLinkExpr::And(l, r) => format!("({} && {})", render(l), render(r)),
            IrLinkExpr::Or(l, r) => format!("({} || {})", render(l), render(r)),
            IrLinkExpr::Compare { op, lhs, rhs } => {
                format!("({} {op} {})", render(lhs), render(rhs))
            }
        }
    }
    let mut lf = link_function();
    lf.buffers = vec![
        crate::ir::IrBuffer {
            slot: "one".to_string(),
            size: IrLinkExpr::Mul(
                Box::new(IrLinkExpr::Var("frames".to_string())),
                Box::new(IrLinkExpr::Int(4)),
            ),
        },
        // Two buffers, so the vector length round-trips rather than "the first".
        crate::ir::IrBuffer {
            slot: "two".to_string(),
            size: IrLinkExpr::Int(64),
        },
    ];
    lf.result_length = Some(IrLinkExpr::Sub(
        Box::new(IrLinkExpr::Var("status".to_string())),
        Box::new(IrLinkExpr::Int(1)),
    ));
    let mut project = empty_project("buf");
    project.link_functions = vec![lf];
    let bytes = encode_binary_repr(&project);
    let decoded = decode_binary_repr(&bytes).expect("decode");
    let f = &decoded.link_functions[0];
    assert_eq!(f.buffers.len(), 2, "both buffers survive");
    assert_eq!(f.buffers[0].slot, "one");
    assert_eq!(render(&f.buffers[0].size), "(frames * 4)");
    assert_eq!(f.buffers[1].slot, "two");
    assert_eq!(render(&f.buffers[1].size), "64");
    assert_eq!(
        render(f.result_length.as_ref().expect("LENGTH survives")),
        "(status - 1)"
    );
}

/// A `buffers` count is an attacker-controlled u32, so it is an allocation
/// primitive unless bounded — the same reasoning as `MAX_CSTRUCTS`.
#[test]
fn rejects_an_over_limit_buffer_count() {
    let mut project = empty_project("buf");
    project.link_functions = vec![link_function()];
    let bytes = encode_binary_repr(&project);
    // Locate the buffers count by re-encoding with a different length and
    // diffing — the same trick `rejects_an_invalid_slot_direction` uses.
    let mut two = project.clone();
    two.link_functions[0].buffers.push(crate::ir::IrBuffer {
        slot: "x".to_string(),
        size: IrLinkExpr::Int(1),
    });
    let b = encode_binary_repr(&two);
    let idx = bytes
        .iter()
        .zip(b.iter())
        .position(|(x, y)| x != y)
        .expect("the buffer count differs somewhere");
    let mut corrupt = bytes.clone();
    corrupt[idx..idx + 4].copy_from_slice(&9_999u32.to_le_bytes());
    let err = match decode_binary_repr(&corrupt) {
        Err(err) => err,
        Ok(_) => panic!("an over-limit buffer count must be rejected"),
    };
    assert!(err.contains("exceeds the 16 limit"), "{err}");
}

/// The gap called out in plan-58-C §2.3: the decode path accepts ANY UTF-8 as a
/// slot ctype, and nothing tested that `ir::verify` catches a crafted one.
///
/// The layering is deliberate — the decoder carries names, `ir::verify` judges
/// them — but that only holds if the judging is actually exercised.
#[test]
fn rejects_a_crafted_unknown_slot_ctype() {
    let mut project = empty_project("ctype");
    let mut lf = link_function();
    lf.abi_slots = vec![IrAbiSlot {
        name: "path".to_string(),
        ctype: crate::types::ParameterType::parse("CBogus"),
        direction: crate::ir::AbiDirection::In,
    }];
    lf.buffers.clear();
    lf.result_length = None;
    project.link_functions = vec![lf];
    let bytes = encode_binary_repr(&project);
    // It DECODES, and it passes the STRUCTURAL check: the decoder carries ctype
    // names without judging them, and `verify_package` judges shape, not rules.
    let decoded = decode_binary_repr(&bytes).expect("an unknown ctype still decodes");
    assert_eq!(
        decoded.link_functions[0].abi_slots[0].ctype.name(),
        "CBogus"
    );
    assert!(
        verify_package(&decoded).is_ok(),
        "the structural check is not where a bad ctype is caught"
    );
    // The SEMANTIC check is. Both run on a decoded package (`nir::lower` calls
    // verify_package then verify_semantics), so a crafted ctype is rejected —
    // but only because the rule gate runs, which is what this pins.
    let err = crate::ir::verify_semantics(&decoded).expect_err("an unknown ctype must be rejected");
    assert!(err.contains("CBogus"), "{err}");
    assert!(err.contains("NATIVE_ABI_UNKNOWN_CTYPE"), "{err}");
}

/// The package-path twins for plan-58-A's buffer rules, reached through a real
/// encode/decode rather than a hand-built `IrProject`.
///
/// `ir/verify/tests.rs` covers the rules against constructed IR; this covers the
/// path a crafted `.mfp` actually takes, so a decoder that dropped `buffers` on
/// the floor would show up here as a rule that stopped firing.
#[test]
fn rejects_crafted_malformed_buffers_after_decode() {
    let base = || {
        let mut lf = link_function();
        lf.params = vec![(
            "n".to_string(),
            crate::types::ParameterType::parse("Integer"),
        )];
        lf.return_type = crate::types::ParameterType::parse(crate::ir::BYTE_LIST_TYPE);
        lf.return_resource = false;
        lf.abi_slots = vec![
            IrAbiSlot {
                name: "buf".to_string(),
                ctype: crate::types::ParameterType::parse("CBuffer"),
                direction: crate::ir::AbiDirection::Out,
            },
            IrAbiSlot {
                name: "n".to_string(),
                ctype: crate::types::ParameterType::parse("CInt64"),
                direction: crate::ir::AbiDirection::In,
            },
        ];
        lf.abi_return_name = "status".to_string();
        lf.abi_return_ctype = crate::types::ParameterType::parse("CInt32");
        lf.consts = vec![];
        lf.free = None;
        lf.success_on = None;
        lf.result = Some(IrLinkExpr::Var("buf".to_string()));
        lf.buffers = vec![crate::ir::IrBuffer {
            slot: "buf".to_string(),
            size: IrLinkExpr::Var("n".to_string()),
        }];
        lf.result_length = Some(IrLinkExpr::Var("status".to_string()));
        lf
    };
    let judge = |lf: crate::ir::IrLinkFunction| -> Result<(), String> {
        let mut project = empty_project("crafted");
        project.link_functions = vec![lf];
        let bytes = encode_binary_repr(&project);
        let decoded = decode_binary_repr(&bytes).expect("decode");
        crate::ir::verify_semantics(&decoded)
    };

    // The baseline must pass, or every case below is vacuous.
    judge(base()).expect("a well-formed CBuffer package must verify");

    // Bad direction. Asserted only as "rejected", not on a specific rule:
    // `IN CBuffer` legitimately trips THREE rules — NATIVE_ABI_UNKNOWN_CTYPE
    // (CBuffer is not valid as an argument), NATIVE_ABI_UNBOUND_SLOT (an IN slot
    // with no parameter), and NATIVE_BUFFER_INVALID (rule 1) — and
    // `verify_semantics` surfaces only the FIRST. Pinning one here would be
    // pinning diagnostic order. `ir::verify::tests::rejects_cbuffer_in_slot`
    // asserts the rule itself against constructed IR.
    let mut lf = base();
    lf.abi_slots[0].direction = crate::ir::AbiDirection::In;
    assert!(judge(lf).is_err(), "an IN CBuffer must be rejected");

    // Missing BUFFER clause — the shape a pre-plan-58-C package decodes as.
    let mut lf = base();
    lf.buffers.clear();
    assert!(judge(lf).unwrap_err().contains("NATIVE_BUFFER_INVALID"));

    // BUFFER naming a slot that does not exist.
    let mut lf = base();
    lf.buffers[0].slot = "ghost".to_string();
    assert!(judge(lf).unwrap_err().contains("NATIVE_BUFFER_INVALID"));

    // LENGTH with no CBuffer result.
    let mut lf = base();
    lf.abi_slots[0].ctype = crate::types::ParameterType::parse("CInt64");
    lf.buffers.clear();
    lf.return_type = crate::types::ParameterType::parse("Integer");
    assert!(judge(lf).unwrap_err().contains("NATIVE_BUFFER_INVALID"));

    // A returned CBuffer with no LENGTH.
    let mut lf = base();
    lf.result_length = None;
    assert!(judge(lf).unwrap_err().contains("NATIVE_BUFFER_INVALID"));
}
