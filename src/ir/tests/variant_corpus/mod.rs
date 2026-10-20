//! The IR variant corpus and the tests built on it (plan-12; renamed from
//! `coverage_tests` by bug-342 C2, which named a tooling motivation rather than a
//! subject). `variant_corpus` is the single project exercising every `IrType` /
//! `IrOp` / `IrValue` / `IrMatchPattern` kind plus the LINK/resource surface —
//! a strict superset of what `ir::tests`'s round-trip suite needs, so both suites
//! build from this one builder. The tests here cover the binary encode/decode
//! round-trip over that full surface and its malformed-input error paths, the
//! package identity/merge rewrites over every op and value shape, and the
//! `to_json` / `annotated_type` / `loc` projections.
//!
//! These build IR directly (rather than lowering source) so they can exercise
//! decode error branches and node shapes a well-formed lowering never produces.

use super::binary::{self, decode_binary_repr, encode_binary_repr, verify_package};
use super::link::{IrAbiSlot, IrFree, IrLinkExpr, IrLinkFunction, IrNativeResource};
use super::package::{
    apply_package_identity, merge_package, package_qualified_reference_names,
    prefix_package_symbols,
};
use super::*;
use crate::ir::resource_escape::ResOwner;
use crate::operators::{BinaryOp, UnaryOp};
use crate::types::ParameterType;

mod binary_and_package;
mod projections_and_op_loc;

// --- builders --------------------------------------------------------------

fn loc() -> IrSourceLoc {
    IrSourceLoc::default()
}

fn c(type_: &str, value: &str) -> IrValue {
    IrValue::Const {
        type_: crate::types::ParameterType::parse(type_),
        value: value.to_string(),
    }
}

fn empty_project(name: &str) -> IrProject {
    crate::ir::test_support::project_fixture(name, vec![], vec![])
}

fn fn_body(name: &str, body: Vec<IrOp>) -> IrFunction {
    IrFunction {
        name: name.to_string(),
        visibility: "export".to_string(),
        kind: "func".to_string(),
        isolated: false,
        params: vec![],
        returns: crate::types::ParameterType::parse("Integer"),
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
            type_: crate::types::ParameterType::parse("Integer"),
        },
        IrValue::FunctionRef {
            name: "f".to_string(),
            type_: crate::types::ParameterType::parse("() -> Integer"),
        },
        IrValue::Closure {
            name: "lam".to_string(),
            type_: crate::types::ParameterType::parse("() -> Integer"),
            captures: vec![
                IrValue::Local("a".to_string()),
                IrValue::Global("g".to_string()),
            ],
        },
        IrValue::Capture {
            index: 0,
            type_: crate::types::ParameterType::parse("Integer"),
            by_ref: false,
        },
        IrValue::Capture {
            index: 1,
            type_: crate::types::ParameterType::parse("Integer"),
            by_ref: true,
        },
        IrValue::Call {
            target: "f".to_string(),
            args: vec![c("Integer", "2")],
            loc: loc(),
            type_: crate::types::ParameterType::parse("Integer"),
        },
        IrValue::CallResult {
            target: "toInt".to_string(),
            args: vec![IrValue::Local("s".to_string())],
            loc: loc(),
            type_: crate::types::ParameterType::parse("Integer"),
        },
        IrValue::Constructor {
            type_: crate::types::ParameterType::parse("Point"),
            args: vec![c("Integer", "1"), c("Integer", "2")],
        },
        IrValue::UnionWrap {
            union_type: crate::types::ParameterType::parse("Shape"),
            member_type: crate::types::ParameterType::parse("Point"),
            value: Box::new(IrValue::Local("p".to_string())),
        },
        IrValue::UnionExtract {
            type_: crate::types::ParameterType::parse("Point"),
            value: Box::new(IrValue::Local("s".to_string())),
        },
        IrValue::ResultIsOk {
            value: Box::new(IrValue::Local("r".to_string())),
        },
        IrValue::ResultValue {
            type_: crate::types::ParameterType::parse("Integer"),
            value: Box::new(IrValue::Local("r".to_string())),
        },
        IrValue::ResultError {
            value: Box::new(IrValue::Local("r".to_string())),
        },
        // bug-471. The wrapped value is call-free on purpose: that is the shape
        // the inline-TRAP desugar builds and the only one `ir::verify` accepts
        // (`check_checked_has_no_call`).
        IrValue::Checked {
            type_: crate::types::ParameterType::parse("Integer"),
            value: Box::new(IrValue::Binary {
                op: BinaryOp::Divide,
                left: Box::new(IrValue::Local("a".to_string())),
                right: Box::new(IrValue::Global("g".to_string())),
                loc: loc(),
                type_: crate::types::ParameterType::parse("Integer"),
            }),
            observe: true,
        },
        // bug-798: the capture-only form (a `Float` `MOD` lifted from inside an
        // operator tree for its domain error), which encodes as its own tag.
        IrValue::Checked {
            type_: crate::types::ParameterType::parse("Float"),
            value: Box::new(IrValue::Binary {
                op: BinaryOp::Mod,
                left: Box::new(IrValue::Local("a".to_string())),
                right: Box::new(IrValue::Global("g".to_string())),
                loc: loc(),
                type_: crate::types::ParameterType::parse("Float"),
            }),
            observe: false,
        },
        IrValue::WithUpdate {
            type_: crate::types::ParameterType::parse("Point"),
            target: Box::new(IrValue::Local("p".to_string())),
            updates: vec![IrRecordUpdate {
                field: "x".to_string(),
                value: c("Integer", "9"),
            }],
        },
        IrValue::ListLiteral {
            type_: crate::types::ParameterType::parse("List OF Integer"),
            values: vec![c("Integer", "1"), IrValue::Global("g".to_string())],
        },
        IrValue::MapLiteral {
            type_: crate::types::ParameterType::parse("Map OF String TO Integer"),
            entries: vec![(c("String", "k"), IrValue::Global("g".to_string()))],
        },
        IrValue::SetLiteral {
            type_: crate::types::ParameterType::parse("Set OF Integer"),
            values: vec![c("Integer", "1"), IrValue::Global("g".to_string())],
        },
        IrValue::MemberAccess {
            target: Box::new(IrValue::Local("p".to_string())),
            member: "x".to_string(),
            type_: crate::types::ParameterType::parse("Integer"),
        },
        IrValue::Binary {
            op: BinaryOp::Add,
            left: Box::new(IrValue::Local("a".to_string())),
            right: Box::new(IrValue::Global("g".to_string())),
            loc: loc(),
            type_: crate::types::ParameterType::parse("Integer"),
        },
        IrValue::Unary {
            op: UnaryOp::Not,
            operand: Box::new(IrValue::Local("b".to_string())),
            loc: loc(),
            type_: crate::types::ParameterType::parse("Boolean"),
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
            type_: ParameterType::Integer,
            value: Some(c("Integer", "1")),
            explicit_type: true,
            loc: loc(),
        },
        IrOp::Bind {
            mutable: false,
            name: "z".to_string(),
            type_: ParameterType::Integer,
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
                type_: crate::types::ParameterType::parse("Integer"),
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
            type_: ParameterType::Integer,
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
            type_: ParameterType::Integer,
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
        params: vec![(
            "path".to_string(),
            crate::types::ParameterType::parse("String"),
        )],
        return_type: crate::types::ParameterType::parse("Db"),
        return_resource: true,
        return_state_type: None,
        abi_slots: vec![
            IrAbiSlot {
                name: "path".to_string(),
                ctype: crate::types::ParameterType::parse("CPtr"),
                direction: crate::ir::AbiDirection::In,
            },
            IrAbiSlot {
                name: "db".to_string(),
                ctype: crate::types::ParameterType::parse("CPtr"),
                direction: crate::ir::AbiDirection::Out,
            },
        ],
        abi_return_name: "rc".to_string(),
        abi_return_ctype: crate::types::ParameterType::parse("CInt32"),
        consts: vec![("flags".to_string(), 6)],
        bind_in: vec![],
        bind_state: None,
        bind_state_resource: None,
        // plan-58-C: `buffers` and `result_length` now ride the wire, so the
        // full-surface round trip covers them here. The SIZE expression uses all
        // three arithmetic arms so tags 6-8 decode inside a buffer too, not only
        // through `result`.
        buffers: vec![crate::ir::IrBuffer {
            slot: "db".to_string(),
            size: IrLinkExpr::Sub(
                Box::new(IrLinkExpr::Add(
                    Box::new(IrLinkExpr::Mul(
                        Box::new(IrLinkExpr::Var("path".to_string())),
                        Box::new(IrLinkExpr::Int(2)),
                    )),
                    Box::new(IrLinkExpr::Int(16)),
                )),
                Box::new(IrLinkExpr::Int(1)),
            ),
        }],
        result_length: Some(IrLinkExpr::Var("status".to_string())),
        // Exercise every IrLinkExpr arm across success_on/result.
        success_on: Some(IrLinkExpr::Compare {
            op: "=".to_string(),
            lhs: Box::new(IrLinkExpr::Var("status".to_string())),
            rhs: Box::new(IrLinkExpr::Int(0)),
        }),
        result: Some(IrLinkExpr::Or(
            Box::new(IrLinkExpr::And(
                Box::new(IrLinkExpr::Var("status".to_string())),
                Box::new(IrLinkExpr::Not(Box::new(IrLinkExpr::Int(1)))),
            )),
            // plan-58-B tags 6-8: Mul/Add/Sub. Nested so all three decode.
            Box::new(IrLinkExpr::Sub(
                Box::new(IrLinkExpr::Add(
                    Box::new(IrLinkExpr::Mul(
                        Box::new(IrLinkExpr::Var("status".to_string())),
                        Box::new(IrLinkExpr::Int(2)),
                    )),
                    Box::new(IrLinkExpr::Int(16)),
                )),
                Box::new(IrLinkExpr::Int(1)),
            )),
        )),
        free: Some(IrFree {
            slot: "return".to_string(),
            symbol: "sqlite3_free".to_string(),
        }),
    }
}

/// A project touching every serializable field: entry, bindings, all type kinds,
/// a function with params/defaults/body/resource_owners, native LINK tables.
pub(crate) fn variant_corpus() -> IrProject {
    let mut resource_owners = HashMap::new();
    resource_owners.insert("db".to_string(), ResOwner::Local);
    resource_owners.insert("f".to_string(), ResOwner::Float("files".to_string()));

    IrProject {
        name: "full".to_string(),
        entry: Some(EntryPoint {
            name: "main".to_string(),
            returns: crate::types::ParameterType::parse("Integer"),
            accepts_args: true,
        }),
        bindings: vec![IrBinding {
            name: "g".to_string(),
            visibility: "public".to_string(),
            mutable: true,
            type_: crate::types::ParameterType::parse("Integer"),
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
                        type_: crate::types::ParameterType::parse("Integer"),
                        loc: loc(),
                    },
                    IrField {
                        visibility: None,
                        name: "y".to_string(),
                        type_: crate::types::ParameterType::parse("Integer"),
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
                    type_: crate::types::ParameterType::parse("Point"),
                    fields: vec![IrField {
                        visibility: None,
                        name: "x".to_string(),
                        type_: crate::types::ParameterType::parse("Integer"),
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
                    type_: crate::types::ParameterType::parse("Integer"),
                    default: None,
                    loc: loc(),
                },
                IrParam {
                    name: "y".to_string(),
                    type_: crate::types::ParameterType::parse("Integer"),
                    default: Some(c("Integer", "0")),
                    loc: loc(),
                },
            ],
            returns: crate::types::ParameterType::parse("Integer"),
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
        link_cstructs: vec![crate::ir::IrCStruct {
            alias: "sqliteLink".to_string(),
            name: "SfFormatInfo".to_string(),
            maps_to: crate::types::ParameterType::parse("AudioFormat"),
            fields: vec![
                crate::ir::IrCStructField {
                    name: "format".to_string(),
                    ctype: crate::types::ParameterType::parse("CInt32"),
                },
                crate::ir::IrCStructField {
                    name: "name".to_string(),
                    ctype: crate::types::ParameterType::parse("CString"),
                },
                crate::ir::IrCStructField {
                    name: "extension".to_string(),
                    ctype: crate::types::ParameterType::parse("CString"),
                },
            ],
        }],
        link_aliases: vec![("openAlias".to_string(), "sqliteLink.open".to_string())],
        docs: ProjectDocs::default(),
        native_libraries: Default::default(),
        max_buffer_bytes: crate::manifest::DEFAULT_MAX_BUFFER_MIB * 1024 * 1024,
        canvas_auto_paint_ms: crate::manifest::DEFAULT_CANVAS_AUTO_PAINT_MS,
        selected_config: Default::default(),
    }
}
