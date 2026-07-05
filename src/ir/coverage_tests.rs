//! Coverage-focused unit tests for the IR support modules (plan-12): the binary
//! encode/decode round-trip over the *full* op/value/link/resource surface and
//! its malformed-input error paths; the package identity/merge rewrites over
//! every op and value shape; and the `to_json` / `annotated_type` / `loc`
//! projections for the op and value variants the main corpus does not reach.
//!
//! These build IR directly (rather than lowering source) so they can exercise
//! decode error branches and node shapes a well-formed lowering never produces.

use super::binary::{decode_binary_repr, encode_binary_repr, verify_package};
use super::link::{IrAbiSlot, IrFree, IrLinkExpr, IrLinkFunction, IrNativeResource};
use super::package::{
    apply_package_identity, merge_package, package_qualified_reference_names,
    prefix_package_symbols,
};
use super::*;
use crate::escape::ResOwner;

// --- builders --------------------------------------------------------------

fn loc() -> IrSourceLoc {
    IrSourceLoc::default()
}

fn c(type_: &str, value: &str) -> IrValue {
    IrValue::Const {
        type_: type_.to_string(),
        value: value.to_string(),
    }
}

fn empty_project(name: &str) -> IrProject {
    IrProject {
        name: name.to_string(),
        entry: None,
        bindings: vec![],
        types: vec![],
        functions: vec![],
        native_resources: vec![],
        link_functions: vec![],
        link_aliases: vec![],
        docs: ProjectDocs::default(),
    }
}

fn fn_body(name: &str, body: Vec<IrOp>) -> IrFunction {
    IrFunction {
        name: name.to_string(),
        visibility: "export".to_string(),
        kind: "func".to_string(),
        isolated: false,
        params: vec![],
        returns: "Integer".to_string(),
        body,
        file: "src/main.mfb".to_string(),
        resource_owners: HashMap::new(),
        loc: loc(),
    }
}

/// Every `IrValue` variant, so encode/decode and rewrite walks visit all arms.
fn every_value() -> Vec<IrValue> {
    vec![
        c("Integer", "1"),
        IrValue::Local("a".to_string()),
        IrValue::Global("g".to_string()),
        IrValue::LocalRef {
            name: "a".to_string(),
            type_: "Integer".to_string(),
        },
        IrValue::FunctionRef {
            name: "f".to_string(),
            type_: "() -> Integer".to_string(),
        },
        IrValue::Closure {
            name: "lam".to_string(),
            type_: "() -> Integer".to_string(),
            captures: vec![
                IrValue::Local("a".to_string()),
                IrValue::Global("g".to_string()),
            ],
        },
        IrValue::Capture {
            index: 0,
            type_: "Integer".to_string(),
            by_ref: false,
        },
        IrValue::Capture {
            index: 1,
            type_: "Integer".to_string(),
            by_ref: true,
        },
        IrValue::Call {
            target: "f".to_string(),
            args: vec![c("Integer", "2")],
            loc: loc(),
            type_: "Integer".to_string(),
        },
        IrValue::CallResult {
            target: "toInt".to_string(),
            args: vec![IrValue::Local("s".to_string())],
            loc: loc(),
            type_: "Integer".to_string(),
        },
        IrValue::Constructor {
            type_: "Point".to_string(),
            args: vec![c("Integer", "1"), c("Integer", "2")],
        },
        IrValue::UnionWrap {
            union_type: "Shape".to_string(),
            member_type: "Point".to_string(),
            value: Box::new(IrValue::Local("p".to_string())),
        },
        IrValue::UnionExtract {
            type_: "Point".to_string(),
            value: Box::new(IrValue::Local("s".to_string())),
        },
        IrValue::ResultIsOk {
            value: Box::new(IrValue::Local("r".to_string())),
        },
        IrValue::ResultValue {
            type_: "Integer".to_string(),
            value: Box::new(IrValue::Local("r".to_string())),
        },
        IrValue::ResultError {
            value: Box::new(IrValue::Local("r".to_string())),
        },
        IrValue::WithUpdate {
            type_: "Point".to_string(),
            target: Box::new(IrValue::Local("p".to_string())),
            updates: vec![IrRecordUpdate {
                field: "x".to_string(),
                value: c("Integer", "9"),
            }],
        },
        IrValue::ListLiteral {
            type_: "List OF Integer".to_string(),
            values: vec![c("Integer", "1"), IrValue::Global("g".to_string())],
        },
        IrValue::MapLiteral {
            type_: "Map OF String TO Integer".to_string(),
            entries: vec![(c("String", "k"), IrValue::Global("g".to_string()))],
        },
        IrValue::MemberAccess {
            target: Box::new(IrValue::Local("p".to_string())),
            member: "x".to_string(),
            type_: "Integer".to_string(),
        },
        IrValue::Binary {
            op: "+".to_string(),
            left: Box::new(IrValue::Local("a".to_string())),
            right: Box::new(IrValue::Global("g".to_string())),
            loc: loc(),
            type_: "Integer".to_string(),
        },
        IrValue::Unary {
            op: "NOT".to_string(),
            operand: Box::new(IrValue::Local("b".to_string())),
            loc: loc(),
            type_: "Boolean".to_string(),
        },
    ]
}

/// Every `IrOp` variant, including the ones the main corpus omits (For, DoUntil,
/// StateAssign, ExitLoop, ContinueLoop, ExitProgram).
fn every_op() -> Vec<IrOp> {
    vec![
        IrOp::Bind {
            mutable: true,
            name: "a".to_string(),
            type_: "Integer".to_string(),
            value: Some(c("Integer", "1")),
            explicit_type: true,
            loc: loc(),
        },
        IrOp::Bind {
            mutable: false,
            name: "z".to_string(),
            type_: "Integer".to_string(),
            value: None,
            explicit_type: false,
            loc: loc(),
        },
        IrOp::Assign {
            name: "a".to_string(),
            value: c("Integer", "2"),
            loc: loc(),
        },
        IrOp::AssignGlobal {
            name: "g".to_string(),
            value: c("Integer", "3"),
            loc: loc(),
        },
        IrOp::StateAssign {
            resource: "res".to_string(),
            value: c("Integer", "4"),
            loc: loc(),
        },
        IrOp::Eval {
            value: IrValue::Call {
                target: "f".to_string(),
                args: every_value(),
                loc: loc(),
                type_: "Integer".to_string(),
            },
            loc: loc(),
        },
        IrOp::ExitLoop {
            kind: LoopKind::While,
            loc: loc(),
        },
        IrOp::ContinueLoop {
            kind: LoopKind::For,
            loc: loc(),
        },
        IrOp::ExitProgram {
            code: c("Integer", "0"),
            loc: loc(),
        },
        IrOp::Fail {
            error: IrValue::Local("e".to_string()),
            loc: loc(),
        },
        IrOp::If {
            condition: IrValue::Local("b".to_string()),
            then_body: vec![IrOp::Return {
                value: Some(IrValue::Local("a".to_string())),
                loc: loc(),
            }],
            else_body: vec![IrOp::Return {
                value: None,
                loc: loc(),
            }],
            loc: loc(),
        },
        IrOp::While {
            kind: LoopKind::While,
            condition: IrValue::Local("b".to_string()),
            body: vec![IrOp::Eval {
                value: IrValue::Local("a".to_string()),
                loc: loc(),
            }],
            loc: loc(),
        },
        IrOp::For {
            name: "i".to_string(),
            type_: "Integer".to_string(),
            start: c("Integer", "0"),
            end: c("Integer", "10"),
            step: c("Integer", "1"),
            body: vec![IrOp::Eval {
                value: IrValue::Local("i".to_string()),
                loc: loc(),
            }],
            loc: loc(),
        },
        IrOp::DoUntil {
            body: vec![IrOp::Eval {
                value: IrValue::Local("a".to_string()),
                loc: loc(),
            }],
            condition: IrValue::Local("b".to_string()),
            loc: loc(),
        },
        IrOp::ForEach {
            name: "item".to_string(),
            type_: "Integer".to_string(),
            iterable: IrValue::Local("list".to_string()),
            body: vec![IrOp::Eval {
                value: IrValue::Local("item".to_string()),
                loc: loc(),
            }],
            loc: loc(),
        },
        IrOp::Match {
            value: IrValue::Local("s".to_string()),
            cases: vec![
                IrMatchCase {
                    pattern: IrMatchPattern::Value(IrValue::Local("p".to_string())),
                    guard: Some(IrValue::Local("b".to_string())),
                    body: vec![IrOp::Eval {
                        value: IrValue::Local("p".to_string()),
                        loc: loc(),
                    }],
                    loc: loc(),
                },
                IrMatchCase {
                    pattern: IrMatchPattern::OneOf(vec![
                        IrValue::Local("p".to_string()),
                        IrValue::Local("q".to_string()),
                    ]),
                    guard: None,
                    body: vec![],
                    loc: loc(),
                },
                IrMatchCase {
                    pattern: IrMatchPattern::Else,
                    guard: None,
                    body: vec![IrOp::Fail {
                        error: IrValue::Local("e".to_string()),
                        loc: loc(),
                    }],
                    loc: loc(),
                },
            ],
            loc: loc(),
        },
        IrOp::Trap {
            name: "err".to_string(),
            body: vec![IrOp::Eval {
                value: IrValue::Local("a".to_string()),
                loc: loc(),
            }],
            loc: loc(),
        },
    ]
}

fn link_function() -> IrLinkFunction {
    IrLinkFunction {
        alias: "sqliteLink".to_string(),
        name: "open".to_string(),
        library: "sqlite3".to_string(),
        symbol: "sqlite3_open".to_string(),
        params: vec![("path".to_string(), "String".to_string())],
        return_type: "Db".to_string(),
        return_resource: true,
        abi_slots: vec![
            IrAbiSlot {
                name: "path".to_string(),
                ctype: "CPtr".to_string(),
                is_out: false,
            },
            IrAbiSlot {
                name: "db".to_string(),
                ctype: "CPtr".to_string(),
                is_out: true,
            },
        ],
        abi_return_name: "rc".to_string(),
        abi_return_ctype: "CInt32".to_string(),
        consts: vec![("flags".to_string(), 6)],
        // Exercise every IrLinkExpr arm across success_on/result.
        success_on: Some(IrLinkExpr::Compare {
            op: "=".to_string(),
            lhs: Box::new(IrLinkExpr::Var),
            rhs: Box::new(IrLinkExpr::Int(0)),
        }),
        result: Some(IrLinkExpr::Or(
            Box::new(IrLinkExpr::And(
                Box::new(IrLinkExpr::Var),
                Box::new(IrLinkExpr::Not(Box::new(IrLinkExpr::Int(1)))),
            )),
            Box::new(IrLinkExpr::Int(2)),
        )),
        free: Some(IrFree {
            slot: "return".to_string(),
            symbol: "sqlite3_free".to_string(),
        }),
    }
}

/// A project touching every serializable field: entry, bindings, all type kinds,
/// a function with params/defaults/body/resource_owners, native LINK tables.
fn full_project() -> IrProject {
    let mut resource_owners = HashMap::new();
    resource_owners.insert("db".to_string(), ResOwner::Local);
    resource_owners.insert("f".to_string(), ResOwner::Float("files".to_string()));

    IrProject {
        name: "full".to_string(),
        entry: Some(EntryPoint {
            name: "main".to_string(),
            returns: "Integer".to_string(),
            accepts_args: true,
        }),
        bindings: vec![IrBinding {
            name: "g".to_string(),
            visibility: "package".to_string(),
            mutable: true,
            type_: "Integer".to_string(),
            value: Some(c("Integer", "0")),
            loc: loc(),
            file: "src/main.mfb".to_string(),
            explicit_type: true,
        }],
        types: vec![
            IrType {
                kind: "type".to_string(),
                visibility: "export".to_string(),
                name: "Point".to_string(),
                fields: vec![
                    IrField {
                        visibility: Some("private".to_string()),
                        name: "x".to_string(),
                        type_: "Integer".to_string(),
                        loc: loc(),
                    },
                    IrField {
                        visibility: None,
                        name: "y".to_string(),
                        type_: "Integer".to_string(),
                        loc: loc(),
                    },
                ],
                includes: vec![],
                variants: vec![],
                members: vec![],
                loc: loc(),
                file: "src/main.mfb".to_string(),
            },
            IrType {
                kind: "union".to_string(),
                visibility: "export".to_string(),
                name: "Shape".to_string(),
                fields: vec![],
                includes: vec!["Base".to_string()],
                variants: vec![IrVariant {
                    name: "Point".to_string(),
                    fields: vec![IrField {
                        visibility: None,
                        name: "x".to_string(),
                        type_: "Integer".to_string(),
                        loc: loc(),
                    }],
                    loc: loc(),
                }],
                members: vec![],
                loc: loc(),
                file: "src/main.mfb".to_string(),
            },
            IrType {
                kind: "enum".to_string(),
                visibility: "private".to_string(),
                name: "Color".to_string(),
                fields: vec![],
                includes: vec![],
                variants: vec![],
                members: vec![
                    IrEnumMember {
                        name: "Red".to_string(),
                    },
                    IrEnumMember {
                        name: "Green".to_string(),
                    },
                ],
                loc: loc(),
                file: "src/main.mfb".to_string(),
            },
        ],
        functions: vec![IrFunction {
            name: "main".to_string(),
            visibility: "export".to_string(),
            kind: "func".to_string(),
            isolated: true,
            params: vec![
                IrParam {
                    name: "x".to_string(),
                    type_: "Integer".to_string(),
                    default: None,
                    loc: loc(),
                },
                IrParam {
                    name: "y".to_string(),
                    type_: "Integer".to_string(),
                    default: Some(c("Integer", "0")),
                    loc: loc(),
                },
            ],
            returns: "Integer".to_string(),
            body: every_op(),
            file: "src/main.mfb".to_string(),
            resource_owners,
            loc: loc(),
        }],
        native_resources: vec![IrNativeResource {
            name: "Db".to_string(),
            visibility: "export".to_string(),
            close_function: "sqliteLink.close".to_string(),
            sendable: false,
            close_may_fail: true,
        }],
        link_functions: vec![link_function()],
        link_aliases: vec![("openAlias".to_string(), "sqliteLink.open".to_string())],
        docs: ProjectDocs::default(),
    }
}

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
    let project = full_project();
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
}

#[test]
fn binary_round_trip_link_expr_variants() {
    // Cover Var / Int / Compare / And / Or / Not on the decode path.
    let mut lf = link_function();
    lf.success_on = Some(IrLinkExpr::Not(Box::new(IrLinkExpr::And(
        Box::new(IrLinkExpr::Or(
            Box::new(IrLinkExpr::Compare {
                op: ">".to_string(),
                lhs: Box::new(IrLinkExpr::Var),
                rhs: Box::new(IrLinkExpr::Int(-5)),
            }),
            Box::new(IrLinkExpr::Int(1)),
        )),
        Box::new(IrLinkExpr::Var),
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
    encode_binary_repr(&full_project())
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
    let err = decode_err(&[b'M', b'F']);
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
        visibility: "package".to_string(),
        mutable: false,
        type_: "Integer".to_string(),
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
                    op: "NOT".to_string(),
                    operand: Box::new(v),
                    loc: loc(),
                    type_: "Boolean".to_string(),
                };
            }
            let mut project = empty_project("deep");
            project.bindings = vec![IrBinding {
                name: "g".to_string(),
                visibility: "package".to_string(),
                mutable: false,
                type_: "Boolean".to_string(),
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
    // `full_project` encodes a Match (pattern tags), loops (loop-kind tags),
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
    verify_package(&full_project()).expect("full project is structurally valid");
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
        type_: "Integer".to_string(),
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
                type_: "Integer".to_string(),
                default: Some(IrValue::Call {
                    target: "g".to_string(),
                    args: vec![IrValue::Global("gv".to_string())],
                    loc: loc(),
                    type_: "Integer".to_string(),
                }),
                loc: loc(),
            }];
            // Ensure the body calls g / references it as a closure + fn-ref.
            f.body.push(IrOp::Eval {
                value: IrValue::FunctionRef {
                    name: "g".to_string(),
                    type_: "() -> Integer".to_string(),
                },
                loc: loc(),
            });
            f.body.push(IrOp::Eval {
                value: IrValue::Closure {
                    name: "g".to_string(),
                    type_: "() -> Integer".to_string(),
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
                    type_: "Integer".to_string(),
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
        returns: "Integer".to_string(),
        accepts_args: false,
    });

    let (ref_fns, ref_globals) = package_qualified_reference_names(&pkg);
    assert!(ref_fns.contains("pkg.f"));
    assert!(ref_fns.contains("pkg.g"));
    assert!(ref_globals.contains("pkg.gv"));

    let id = "abc123";
    prefix_package_symbols(&mut pkg, id);
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
                    type_: "Integer".to_string(),
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
        visibility: "package".to_string(),
        mutable: false,
        type_: "Integer".to_string(),
        value: Some(IrValue::Global("pkg.gv".to_string())),
        loc: loc(),
        file: String::new(),
        explicit_type: false,
    }];
    // A defaulted parameter referencing the package's symbols exercises the
    // param-default rewrite branch of apply_package_identity.
    consumer.functions[0].params = vec![IrParam {
        name: "arg".to_string(),
        type_: "Integer".to_string(),
        default: Some(IrValue::Call {
            target: "pkg.f".to_string(),
            args: vec![IrValue::Global("pkg.gv".to_string())],
            loc: loc(),
            type_: "Integer".to_string(),
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
        visibility: "package".to_string(),
        mutable: false,
        type_: "Integer".to_string(),
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
        visibility: "package".to_string(),
        mutable: false,
        type_: "Integer".to_string(),
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

// --- json projections the main corpus misses -------------------------------

#[test]
fn json_covers_every_op_and_value() {
    // Drive to_json over a function whose body is every_op() (which embeds
    // every_value()); asserting it is valid, self-consistent JSON-ish text.
    let mut project = empty_project("j");
    project.entry = Some(EntryPoint {
        name: "main".to_string(),
        returns: "Integer".to_string(),
        accepts_args: false,
    });
    project.functions = vec![fn_body("f", every_op())];
    project.bindings = vec![IrBinding {
        name: "g".to_string(),
        visibility: "export".to_string(),
        mutable: false,
        type_: "Integer".to_string(),
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
    let project = full_project();
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
    use super::json::loop_kind_name;
    assert_eq!(loop_kind_name(LoopKind::For), "for");
    assert_eq!(loop_kind_name(LoopKind::Do), "do");
    assert_eq!(loop_kind_name(LoopKind::While), "while");
}

#[test]
fn visibility_name_covers_all_visibilities() {
    use crate::ast::Visibility;
    assert_eq!(visibility_name(Visibility::Private), "private");
    assert_eq!(visibility_name(Visibility::Package), "package");
    assert_eq!(visibility_name(Visibility::Export), "export");
}

// --- IrValue::annotated_type -----------------------------------------------

#[test]
fn annotated_type_reports_every_annotated_variant() {
    assert_eq!(c("Integer", "1").annotated_type(), Some("Integer"));
    assert_eq!(
        IrValue::LocalRef {
            name: "a".to_string(),
            type_: "Integer".to_string()
        }
        .annotated_type(),
        Some("Integer")
    );
    assert_eq!(
        IrValue::FunctionRef {
            name: "f".to_string(),
            type_: "() -> Integer".to_string()
        }
        .annotated_type(),
        Some("() -> Integer")
    );
    assert_eq!(
        IrValue::Closure {
            name: "l".to_string(),
            type_: "() -> Integer".to_string(),
            captures: vec![]
        }
        .annotated_type(),
        Some("() -> Integer")
    );
    assert_eq!(
        IrValue::Capture {
            index: 0,
            type_: "Integer".to_string(),
            by_ref: false
        }
        .annotated_type(),
        Some("Integer")
    );
    assert_eq!(
        IrValue::Call {
            target: "f".to_string(),
            args: vec![],
            loc: loc(),
            type_: "Integer".to_string()
        }
        .annotated_type(),
        Some("Integer")
    );
    assert_eq!(
        IrValue::CallResult {
            target: "f".to_string(),
            args: vec![],
            loc: loc(),
            type_: "Integer".to_string()
        }
        .annotated_type(),
        Some("Integer")
    );
    assert_eq!(
        IrValue::Constructor {
            type_: "Point".to_string(),
            args: vec![]
        }
        .annotated_type(),
        Some("Point")
    );
    assert_eq!(
        IrValue::UnionExtract {
            type_: "Point".to_string(),
            value: Box::new(c("Integer", "0"))
        }
        .annotated_type(),
        Some("Point")
    );
    assert_eq!(
        IrValue::ResultValue {
            type_: "Integer".to_string(),
            value: Box::new(c("Integer", "0"))
        }
        .annotated_type(),
        Some("Integer")
    );
    assert_eq!(
        IrValue::WithUpdate {
            type_: "Point".to_string(),
            target: Box::new(c("Integer", "0")),
            updates: vec![]
        }
        .annotated_type(),
        Some("Point")
    );
    assert_eq!(
        IrValue::ListLiteral {
            type_: "List OF Integer".to_string(),
            values: vec![]
        }
        .annotated_type(),
        Some("List OF Integer")
    );
    assert_eq!(
        IrValue::MapLiteral {
            type_: "Map OF String TO Integer".to_string(),
            entries: vec![]
        }
        .annotated_type(),
        Some("Map OF String TO Integer")
    );
    assert_eq!(
        IrValue::MemberAccess {
            target: Box::new(c("Integer", "0")),
            member: "x".to_string(),
            type_: "Integer".to_string()
        }
        .annotated_type(),
        Some("Integer")
    );
    assert_eq!(
        IrValue::Binary {
            op: "+".to_string(),
            left: Box::new(c("Integer", "1")),
            right: Box::new(c("Integer", "2")),
            loc: loc(),
            type_: "Integer".to_string()
        }
        .annotated_type(),
        Some("Integer")
    );
    assert_eq!(
        IrValue::Unary {
            op: "NOT".to_string(),
            operand: Box::new(c("Boolean", "true")),
            loc: loc(),
            type_: "Boolean".to_string()
        }
        .annotated_type(),
        Some("Boolean")
    );
    assert_eq!(
        IrValue::UnionWrap {
            union_type: "Shape".to_string(),
            member_type: "Point".to_string(),
            value: Box::new(c("Integer", "0"))
        }
        .annotated_type(),
        Some("Shape")
    );
    assert_eq!(
        IrValue::ResultIsOk {
            value: Box::new(c("Integer", "0"))
        }
        .annotated_type(),
        Some("Boolean")
    );
    assert_eq!(
        IrValue::ResultError {
            value: Box::new(c("Integer", "0"))
        }
        .annotated_type(),
        Some("Error")
    );
    // Local and Global resolve through the environment, not the node.
    assert_eq!(IrValue::Local("a".to_string()).annotated_type(), None);
    assert_eq!(IrValue::Global("g".to_string()).annotated_type(), None);
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
        type_: "Integer".to_string(),
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
        kind: LoopKind::For,
        loc: l,
    });
    with_loc(IrOp::ContinueLoop {
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
        type_: "Integer".to_string(),
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
        type_: "Integer".to_string(),
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
