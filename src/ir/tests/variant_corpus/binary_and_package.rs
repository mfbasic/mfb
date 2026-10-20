//! plan-176-G: the variant corpus's binary round-trip, malformed-input, package-verify and
//! package-identity tests. The corpus builders live in `variant_corpus/mod.rs`.

use super::*;

/// `decode_binary_repr(..).unwrap_err()` without requiring `IrProject: Debug`.
fn decode_err(bytes: &[u8]) -> String {
    match decode_binary_repr(bytes) {
        Ok(_) => panic!("expected decode to fail"),
        Err(e) => e,
    }
}

// --- binary round-trip -----------------------------------------------------

#[test]
fn binary_round_trip_over_full_surface() {
    let project = variant_corpus();
    let bytes = encode_binary_repr(&project);
    let decoded = decode_binary_repr(&bytes).expect("decode");
    // The decoded project drops native_resources/docs by contract (they live in
    // separate package sections), so compare the round-tripped fields via the
    // JSON projection plus a re-encode identity on the decoded value.
    let bytes2 = encode_binary_repr(&decoded);
    // Re-encoding the decode of the *decoded* bytes is a fixed point.
    let decoded2 = decode_binary_repr(&bytes2).expect("decode2");
    assert_eq!(bytes2, encode_binary_repr(&decoded2));
    // Link tables survived the round trip.
    assert_eq!(decoded.link_functions.len(), 1);
    assert_eq!(decoded.link_functions[0].symbol, "sqlite3_open");
    assert_eq!(
        decoded.link_functions[0].consts,
        vec![("flags".to_string(), 6)]
    );
    assert_eq!(decoded.link_aliases.len(), 1);
    // plan-50-C: the CSTRUCT table, the slot direction byte, and the Var name
    // payload all survive the round trip.
    assert_eq!(decoded.link_cstructs.len(), 1);
    assert_eq!(decoded.link_cstructs[0].name, "SfFormatInfo");
    assert_eq!(decoded.link_cstructs[0].maps_to.name(), "AudioFormat");
    assert_eq!(decoded.link_cstructs[0].fields.len(), 3);
    assert_eq!(decoded.link_cstructs[0].fields[1].ctype.name(), "CString");
    assert_eq!(
        decoded.link_functions[0].abi_slots[1].direction,
        crate::ir::AbiDirection::Out
    );
    // Resource owners survived (Local + Float).
    let owners = &decoded.functions[0].resource_owners;
    assert!(matches!(owners.get("db"), Some(ResOwner::Local)));
    assert!(matches!(owners.get("f"), Some(ResOwner::Float(c)) if c == "files"));
}

#[test]
fn binary_round_trip_without_link_tables_is_a_bare_trailer() {
    // A project with no LINK data omits the optional trailer entirely, so the
    // decoder must hit its `at_end` branch and default the tables to empty.
    let project = empty_project("plain");
    let bytes = encode_binary_repr(&project);
    let decoded = decode_binary_repr(&bytes).expect("decode");
    assert!(decoded.link_functions.is_empty());
    assert!(decoded.link_aliases.is_empty());
    assert!(decoded.link_cstructs.is_empty());
}

/// plan-50-C: the version gate is an exact match, not a floor. Without the bump
/// a v4 package would decode its old `is_out` bool as a direction byte and its
/// payload-less `Var` tag as a length prefix — garbage fed straight to a layout
/// computer and a marshaling thunk.
#[test]
fn rejects_a_previous_format_version() {
    let project = empty_project("plain");
    let mut bytes = encode_binary_repr(&project);
    // Rewrite the version word (after the 4 magic bytes) to the previous version.
    //
    // Derived from the constant rather than hardcoded: this test pinned a literal
    // `4` through the 4->5 bump and had to be hand-edited at 5->6, which is
    // exactly the kind of edit that gets forgotten and leaves the gate untested
    // against the version that actually precedes the current one.
    let previous = crate::ir::binary::BINARY_REPR_VERSION - 1;
    bytes[4..6].copy_from_slice(&previous.to_le_bytes());
    let err = match decode_binary_repr(&bytes) {
        Err(err) => err,
        Ok(_) => panic!("v{previous} must be rejected"),
    };
    assert!(
        err.contains(&format!("version {previous} unsupported")),
        "{err}"
    );
}

/// A CSTRUCT count is an attacker-controlled u32, so it is an allocation
/// primitive unless bounded.
#[test]
fn rejects_an_absurd_cstruct_count() {
    let project = empty_project("plain");
    let mut bytes = encode_binary_repr(&project);
    // Append a trailer: 0 link functions, 0 aliases, then a huge CSTRUCT count.
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&u32::MAX.to_le_bytes());
    let err = match decode_binary_repr(&bytes) {
        Err(err) => err,
        Ok(_) => panic!("absurd CSTRUCT count must be rejected"),
    };
    assert!(err.contains("exceeds the 256 limit"), "{err}");
}

/// An unknown direction byte must be an error, never a silent default: it decides
/// whether the callee writes through the slot.
#[test]
fn rejects_an_invalid_slot_direction() {
    let mut project = empty_project("plain");
    project.link_functions = vec![link_function()];
    let bytes = encode_binary_repr(&project);
    // The direction byte is the only 3-valued byte in the payload; find the
    // encoded slot direction by re-encoding with each legal value and diffing.
    let mut in_dir = project.clone();
    in_dir.link_functions[0].abi_slots[0].direction = crate::ir::AbiDirection::In;
    let a = encode_binary_repr(&in_dir);
    let mut out_dir = project.clone();
    out_dir.link_functions[0].abi_slots[0].direction = crate::ir::AbiDirection::Out;
    let b = encode_binary_repr(&out_dir);
    let idx = a
        .iter()
        .zip(b.iter())
        .position(|(x, y)| x != y)
        .expect("directions differ somewhere");
    let mut corrupt = bytes.clone();
    corrupt[idx] = 7;
    let err = match decode_binary_repr(&corrupt) {
        Err(err) => err,
        Ok(_) => panic!("an invalid direction byte must be rejected"),
    };
    assert!(err.contains("invalid ABI slot direction 7"), "{err}");
}

#[test]
fn binary_round_trip_link_expr_variants() {
    // Cover Var / Int / Compare / And / Or / Not on the decode path.
    let mut lf = link_function();
    lf.success_on = Some(IrLinkExpr::Not(Box::new(IrLinkExpr::And(
        Box::new(IrLinkExpr::Or(
            Box::new(IrLinkExpr::Compare {
                op: ">".to_string(),
                lhs: Box::new(IrLinkExpr::Var("status".to_string())),
                rhs: Box::new(IrLinkExpr::Int(-5)),
            }),
            Box::new(IrLinkExpr::Int(1)),
        )),
        Box::new(IrLinkExpr::Var("status".to_string())),
    ))));
    lf.result = None;
    lf.free = None;
    let mut project = empty_project("le");
    project.link_functions = vec![lf];
    let bytes = encode_binary_repr(&project);
    let decoded = decode_binary_repr(&bytes).expect("decode");
    assert!(decoded.link_functions[0].success_on.is_some());
    assert!(decoded.link_functions[0].result.is_none());
    assert!(decoded.link_functions[0].free.is_none());
}

/// The arithmetic variants must round-trip **structurally**, not merely survive.
///
/// `binary_round_trip_link_expr_variants` above asserts only `is_some()`, so it
/// passes even when a decode arm builds the wrong node — swapping tag 8's `Sub`
/// for `Add` does not fail it. That is a real gap for plan-58-B's operators,
/// where confusing two of them silently mis-sizes a buffer, so this renders the
/// decoded tree and compares it.
#[test]
fn binary_round_trip_link_expr_arithmetic_is_structural() {
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

    // Deliberately asymmetric: `Sub` and `Add` swapped, or either confused with
    // `Mul`, changes the rendering.
    let expr = IrLinkExpr::Sub(
        Box::new(IrLinkExpr::Add(
            Box::new(IrLinkExpr::Mul(
                Box::new(IrLinkExpr::Var("status".to_string())),
                Box::new(IrLinkExpr::Int(3)),
            )),
            Box::new(IrLinkExpr::Int(16)),
        )),
        Box::new(IrLinkExpr::Int(1)),
    );
    let want = "(((status * 3) + 16) - 1)";
    assert_eq!(
        render(&expr),
        want,
        "the fixture itself must render as expected"
    );

    let mut lf = link_function();
    lf.result = Some(expr);
    lf.success_on = None;
    let mut project = empty_project("arith");
    project.link_functions = vec![lf];
    let bytes = encode_binary_repr(&project);
    let decoded = decode_binary_repr(&bytes).expect("decode");
    let got = render(
        decoded.link_functions[0]
            .result
            .as_ref()
            .expect("result survives the round trip"),
    );
    assert_eq!(got, want, "arithmetic must round-trip structurally");
}

#[test]
fn binary_round_trip_while_op_with_non_while_loop_kind() {
    // The `While` op encodes a distinct tag when its LoopKind is not `While`
    // (a lowered DO/loop uses the same op with kind Do/For), exercising the
    // put_loop_kind path and its For/Do arms.
    for kind in [LoopKind::Do, LoopKind::For] {
        let mut project = empty_project("loopkind");
        project.functions = vec![fn_body(
            "f",
            vec![IrOp::While {
                kind,
                condition: c("Boolean", "true"),
                body: vec![IrOp::ExitLoop { kind, loc: loc() }],
                loc: loc(),
            }],
        )];
        let bytes = encode_binary_repr(&project);
        let decoded = decode_binary_repr(&bytes).expect("decode");
        match &decoded.functions[0].body[0] {
            IrOp::While { kind: k, .. } => {
                assert_eq!(std::mem::discriminant(k), std::mem::discriminant(&kind))
            }
            _ => panic!("expected While op"),
        }
    }
}

// --- binary malformed-input error paths ------------------------------------

fn full_bytes() -> Vec<u8> {
    encode_binary_repr(&variant_corpus())
}

#[test]
fn decode_rejects_bad_magic() {
    let mut bytes = full_bytes();
    bytes[0] = b'X';
    let err = decode_err(&bytes);
    assert!(err.contains("bad magic"), "{err}");
}

#[test]
fn decode_rejects_bad_version() {
    let mut bytes = full_bytes();
    bytes[4] = 0xFF;
    bytes[5] = 0xFF;
    let err = decode_err(&bytes);
    assert!(err.contains("version"), "{err}");
}

#[test]
fn decode_rejects_too_short_for_magic() {
    let err = decode_err(b"MF");
    assert!(err.contains("truncated"), "{err}");
}

#[test]
fn decode_rejects_truncated_body() {
    // Valid header, then cut off mid-project so a reader `need` fails.
    let bytes = full_bytes();
    let truncated = &bytes[..10];
    assert!(decode_binary_repr(truncated).is_err());
}

#[test]
fn decode_rejects_invalid_utf8_string() {
    // Hand-assemble a minimal payload whose project name length claims 1 byte
    // but that byte is not valid UTF-8.
    let mut bytes = Vec::new();
    bytes.extend_from_slice(super::binary::BINARY_REPR_MAGIC);
    bytes.extend_from_slice(&super::binary::BINARY_REPR_VERSION.to_le_bytes());
    bytes.extend_from_slice(&1u32.to_le_bytes()); // name length = 1
    bytes.push(0xFF); // invalid UTF-8 byte
    let err = decode_err(&bytes);
    assert!(err.contains("invalid UTF-8"), "{err}");
}

#[test]
fn decode_rejects_unknown_op_tag() {
    // Locate the function body op tag and corrupt it. Simplest: build a project
    // with exactly one op, encode, and flip the op tag byte to an unknown value.
    let mut project = empty_project("op");
    project.functions = vec![fn_body(
        "f",
        vec![IrOp::Return {
            value: None,
            loc: loc(),
        }],
    )];
    let mut bytes = encode_binary_repr(&project);
    // Corrupt every byte to 0xEE one at a time until decode reports an unknown
    // IrOp tag; this is robust against layout shifts.
    let mut hit = false;
    for i in 6..bytes.len() {
        let saved = bytes[i];
        bytes[i] = 0xEE;
        if let Err(e) = decode_binary_repr(&bytes) {
            if e.contains("unknown IrOp tag") {
                hit = true;
                break;
            }
        }
        bytes[i] = saved;
    }
    assert!(hit, "expected an unknown IrOp tag rejection");
}

#[test]
fn decode_rejects_unknown_value_tag() {
    let mut project = empty_project("val");
    project.bindings = vec![IrBinding {
        name: "g".to_string(),
        visibility: "public".to_string(),
        mutable: false,
        type_: crate::types::ParameterType::parse("Integer"),
        value: Some(c("Integer", "1")),
        loc: loc(),
        file: String::new(),
        explicit_type: false,
    }];
    let mut bytes = encode_binary_repr(&project);
    let mut hit = false;
    for i in 6..bytes.len() {
        let saved = bytes[i];
        bytes[i] = 0xEE;
        if let Err(e) = decode_binary_repr(&bytes) {
            if e.contains("IrValue") || e.contains("tag") {
                hit = true;
                break;
            }
        }
        bytes[i] = saved;
    }
    assert!(hit, "expected a value-tag rejection");
}

#[test]
fn decode_rejects_depth_limit() {
    // A left-nested unary chain deeper than MAX_DECODE_DEPTH (256) must be
    // rejected by the decoder's recursion guard rather than overflowing the
    // stack. Building + encoding the deep chain is itself recursive, so run it
    // on a thread with a generous stack; the decoder guard is what we assert.
    let err = std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            let mut v = IrValue::Local("x".to_string());
            for _ in 0..300 {
                v = IrValue::Unary {
                    op: UnaryOp::Not,
                    operand: Box::new(v),
                    loc: loc(),
                    type_: crate::types::ParameterType::parse("Boolean"),
                };
            }
            let mut project = empty_project("deep");
            project.bindings = vec![IrBinding {
                name: "g".to_string(),
                visibility: "public".to_string(),
                mutable: false,
                type_: crate::types::ParameterType::parse("Boolean"),
                value: Some(v),
                loc: loc(),
                file: String::new(),
                explicit_type: false,
            }];
            let bytes = encode_binary_repr(&project);
            decode_err(&bytes)
        })
        .expect("spawn")
        .join()
        .expect("join");
    assert!(err.contains("nesting exceeds"), "{err}");
}

#[test]
fn decode_surfaces_every_tag_error_branch() {
    // `variant_corpus` encodes a Match (pattern tags), loops (loop-kind tags),
    // resource_owners (ResOwner tags), and a LINK function (LINK-expr tags).
    // Sweeping each byte to a series of out-of-range values lands on each tag
    // position at some point, exercising every "unknown/invalid tag" decode
    // branch without hand-assembling byte layouts. We assert the union of
    // messages seen covers each tag family.
    let base = full_bytes();
    let needles = [
        "unknown IrOp tag",
        "unknown loop kind tag",
        "unknown IrMatchPattern tag",
        "invalid ResOwner tag",
        "invalid LINK expr tag",
    ];
    let mut seen: Vec<String> = Vec::new();
    let mut bytes = base.clone();
    for i in 6..base.len() {
        let saved = bytes[i];
        // 0xEE is out of range for every tag byte in the format, so wherever it
        // lands on a tag it drives the corresponding "unknown/invalid tag" arm.
        bytes[i] = 0xEE;
        if let Err(e) = decode_binary_repr(&bytes) {
            seen.push(e);
        }
        bytes[i] = saved;
        if needles.iter().all(|n| seen.iter().any(|e| e.contains(n))) {
            break;
        }
    }
    let has = |needle: &str| seen.iter().any(|e| e.contains(needle));
    assert!(has("unknown IrOp tag"), "no unknown-op-tag error surfaced");
    assert!(has("unknown loop kind tag"), "no loop-kind error surfaced");
    assert!(
        has("unknown IrMatchPattern tag"),
        "no match-pattern error surfaced"
    );
    assert!(has("invalid ResOwner tag"), "no ResOwner error surfaced");
    assert!(has("invalid LINK expr tag"), "no LINK-expr error surfaced");
}

// --- verify_package (structural) -------------------------------------------

#[test]
fn verify_package_accepts_well_formed() {
    verify_package(&variant_corpus()).expect("full project is structurally valid");
}

#[test]
fn verify_package_rejects_duplicate_type() {
    let mut project = empty_project("dup");
    let ty = IrType {
        kind: "type".to_string(),
        visibility: "export".to_string(),
        name: "Point".to_string(),
        fields: vec![],
        includes: vec![],
        variants: vec![],
        members: vec![],
        loc: loc(),
        file: String::new(),
    };
    project.types = vec![ty.clone(), ty];
    let err = verify_package(&project).unwrap_err();
    assert!(err.contains("duplicate type"), "{err}");
}

/// bug-33: the structural walk recursed into `If`/`While`/`ForEach`/`Trap`/`Match`
/// bodies but not `For`/`DoUntil`, so the empty-`MATCH` invariant went unapplied
/// inside a counted or do-until loop.
#[test]
fn verify_package_recurses_into_for_and_do_until_bodies() {
    let empty_match = || IrOp::Match {
        value: c("Integer", "1"),
        cases: vec![],
        loc: loc(),
    };
    let for_loop = IrOp::For {
        name: "i".to_string(),
        type_: ParameterType::Integer,
        start: c("Integer", "0"),
        end: c("Integer", "1"),
        step: c("Integer", "1"),
        body: vec![empty_match()],
        loc: loc(),
    };
    let mut project = empty_project("loops");
    project.functions = vec![fn_body("f", vec![for_loop])];
    let err = verify_package(&project).unwrap_err();
    assert!(err.contains("MATCH has no cases"), "{err}");

    let do_until = IrOp::DoUntil {
        body: vec![empty_match()],
        condition: c("Boolean", "true"),
        loc: loc(),
    };
    let mut project = empty_project("loops");
    project.functions = vec![fn_body("f", vec![do_until])];
    let err = verify_package(&project).unwrap_err();
    assert!(err.contains("MATCH has no cases"), "{err}");
}

#[test]
fn verify_package_rejects_deeply_nested_ops() {
    // Nest IF bodies past the structural depth cap. Building + verifying the
    // deep tree is recursive, so run on a generous stack.
    let err = std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            let mut op = IrOp::Return {
                value: None,
                loc: loc(),
            };
            for _ in 0..300 {
                op = IrOp::If {
                    condition: c("Boolean", "true"),
                    then_body: vec![op],
                    else_body: vec![],
                    loc: loc(),
                };
            }
            let mut project = empty_project("nest");
            project.functions = vec![fn_body("f", vec![op])];
            verify_package(&project).unwrap_err()
        })
        .expect("spawn")
        .join()
        .expect("join");
    assert!(err.contains("nesting exceeds"), "{err}");
}

// --- package identity / merge ----------------------------------------------

#[test]
fn prefix_and_apply_identity_rewrite_all_reference_shapes() {
    // A package whose function `f` references its own `g`, a global `gv`, a
    // function ref, a closure, and nested references inside every op/value shape.
    let mut pkg = empty_project("pkg");
    pkg.bindings = vec![IrBinding {
        name: "gv".to_string(),
        visibility: "export".to_string(),
        mutable: true,
        type_: crate::types::ParameterType::parse("Integer"),
        value: Some(IrValue::Global("gv".to_string())),
        loc: loc(),
        file: String::new(),
        explicit_type: false,
    }];
    pkg.functions = vec![
        {
            let mut f = fn_body("f", every_op());
            // Reference g and gv from within a defaulted param too.
            f.params = vec![IrParam {
                name: "p".to_string(),
                type_: crate::types::ParameterType::parse("Integer"),
                default: Some(IrValue::Call {
                    target: "g".to_string(),
                    args: vec![IrValue::Global("gv".to_string())],
                    loc: loc(),
                    type_: crate::types::ParameterType::parse("Integer"),
                }),
                loc: loc(),
            }];
            // Ensure the body calls g / references it as a closure + fn-ref.
            f.body.push(IrOp::Eval {
                value: IrValue::FunctionRef {
                    name: "g".to_string(),
                    type_: crate::types::ParameterType::parse("() -> Integer"),
                },
                loc: loc(),
            });
            f.body.push(IrOp::Eval {
                value: IrValue::Closure {
                    name: "g".to_string(),
                    type_: crate::types::ParameterType::parse("() -> Integer"),
                    captures: vec![IrValue::Global("gv".to_string())],
                },
                loc: loc(),
            });
            f.body.push(IrOp::AssignGlobal {
                name: "gv".to_string(),
                value: IrValue::Call {
                    target: "g".to_string(),
                    args: vec![],
                    loc: loc(),
                    type_: crate::types::ParameterType::parse("Integer"),
                },
                loc: loc(),
            });
            f
        },
        fn_body("g", vec![]),
    ];

    // Give the package an entry point so prefix_package_symbols renames it.
    pkg.entry = Some(EntryPoint {
        name: "start".to_string(),
        returns: crate::types::ParameterType::parse("Integer"),
        accepts_args: false,
    });

    let (ref_fns, ref_globals) = package_qualified_reference_names(&pkg);
    assert!(ref_fns.contains("pkg.f"));
    assert!(ref_fns.contains("pkg.g"));
    assert!(ref_globals.contains("pkg.gv"));

    let id = "abc123";
    prefix_package_symbols(&mut pkg, id, &Default::default());
    assert_eq!(pkg.functions[0].name, "abc123.pkg.f");
    assert_eq!(pkg.bindings[0].name, "abc123.pkg.gv");
    assert_eq!(pkg.entry.as_ref().unwrap().name, "abc123.pkg.start");
    // The internal call/fn-ref/closure/global references got rewritten.
    match &pkg.functions[0].params[0].default {
        Some(IrValue::Call { target, args, .. }) => {
            assert_eq!(target, "abc123.pkg.g");
            assert!(matches!(&args[0], IrValue::Global(n) if n == "abc123.pkg.gv"));
        }
        _ => panic!("expected defaulted Call"),
    }

    // A consumer referencing pkg.f / pkg.g / pkg.gv is rewritten to the prefixed
    // definition names.
    let mut consumer = empty_project("app");
    consumer.functions = vec![fn_body(
        "main",
        vec![
            IrOp::Eval {
                value: IrValue::Call {
                    target: "pkg.f".to_string(),
                    args: vec![],
                    loc: loc(),
                    type_: crate::types::ParameterType::parse("Integer"),
                },
                loc: loc(),
            },
            IrOp::AssignGlobal {
                name: "pkg.gv".to_string(),
                value: IrValue::Global("pkg.gv".to_string()),
                loc: loc(),
            },
        ],
    )];
    consumer.bindings = vec![IrBinding {
        name: "b".to_string(),
        visibility: "public".to_string(),
        mutable: false,
        type_: crate::types::ParameterType::parse("Integer"),
        value: Some(IrValue::Global("pkg.gv".to_string())),
        loc: loc(),
        file: String::new(),
        explicit_type: false,
    }];
    // A defaulted parameter referencing the package's symbols exercises the
    // param-default rewrite branch of apply_package_identity.
    consumer.functions[0].params = vec![IrParam {
        name: "arg".to_string(),
        type_: crate::types::ParameterType::parse("Integer"),
        default: Some(IrValue::Call {
            target: "pkg.f".to_string(),
            args: vec![IrValue::Global("pkg.gv".to_string())],
            loc: loc(),
            type_: crate::types::ParameterType::parse("Integer"),
        }),
        loc: loc(),
    }];
    apply_package_identity(&mut consumer, &ref_fns, &ref_globals, id);
    match &consumer.functions[0].params[0].default {
        Some(IrValue::Call { target, .. }) => assert_eq!(target, "abc123.pkg.f"),
        _ => panic!("expected defaulted Call on consumer param"),
    }
    match &consumer.functions[0].body[0] {
        IrOp::Eval {
            value: IrValue::Call { target, .. },
            ..
        } => assert_eq!(target, "abc123.pkg.f"),
        _ => panic!("expected Eval(Call)"),
    }
    match &consumer.bindings[0].value {
        Some(IrValue::Global(n)) => assert_eq!(n, "abc123.pkg.gv"),
        _ => panic!("expected Global binding value"),
    }
}

#[test]
fn merge_package_dedups_by_name_and_carries_link_tables() {
    let mut base = empty_project("app");
    base.types = vec![IrType {
        kind: "type".to_string(),
        visibility: "export".to_string(),
        name: "Point".to_string(),
        fields: vec![],
        includes: vec![],
        variants: vec![],
        members: vec![],
        loc: loc(),
        file: String::new(),
    }];
    base.functions = vec![fn_body("shared", vec![])];
    base.bindings = vec![IrBinding {
        name: "gshared".to_string(),
        visibility: "public".to_string(),
        mutable: false,
        type_: crate::types::ParameterType::parse("Integer"),
        value: None,
        loc: loc(),
        file: String::new(),
        explicit_type: true,
    }];

    let mut pkg = empty_project("pkg");
    // Duplicates (same names) must NOT be added twice; a NEW type must be added.
    pkg.types = base.types.clone();
    pkg.types.push(IrType {
        kind: "type".to_string(),
        visibility: "export".to_string(),
        name: "Line".to_string(),
        fields: vec![],
        includes: vec![],
        variants: vec![],
        members: vec![],
        loc: loc(),
        file: String::new(),
    });
    pkg.functions = vec![fn_body("shared", vec![]), fn_body("unique", vec![])];
    pkg.bindings = base.bindings.clone();
    pkg.bindings.push(IrBinding {
        name: "gunique".to_string(),
        visibility: "public".to_string(),
        mutable: false,
        type_: crate::types::ParameterType::parse("Integer"),
        value: None,
        loc: loc(),
        file: String::new(),
        explicit_type: true,
    });
    pkg.link_functions = vec![link_function(), link_function()];
    pkg.link_aliases = vec![("a".to_string(), "sqliteLink.open".to_string())];

    merge_package(&mut base, pkg);
    // Types deduped: still one Point; the new Line type was added.
    assert_eq!(base.types.iter().filter(|t| t.name == "Point").count(), 1);
    assert!(base.types.iter().any(|t| t.name == "Line"));
    // Functions: shared once, unique added.
    assert_eq!(
        base.functions.iter().filter(|f| f.name == "shared").count(),
        1
    );
    assert!(base.functions.iter().any(|f| f.name == "unique"));
    // Bindings: gshared once, gunique added.
    assert_eq!(
        base.bindings.iter().filter(|b| b.name == "gshared").count(),
        1
    );
    assert!(base.bindings.iter().any(|b| b.name == "gunique"));
    // Link functions dedup by (alias, name); the two identical ones collapse.
    assert_eq!(base.link_functions.len(), 1);
    // Aliases qualified with the package name.
    assert!(base.link_aliases.iter().any(|(a, _)| a == "pkg.a"));

    // Merging the same package again is idempotent for the alias set.
    let mut pkg2 = empty_project("pkg");
    pkg2.link_aliases = vec![("a".to_string(), "sqliteLink.open".to_string())];
    merge_package(&mut base, pkg2);
    assert_eq!(
        base.link_aliases
            .iter()
            .filter(|(a, _)| a == "pkg.a")
            .count(),
        1
    );
}
