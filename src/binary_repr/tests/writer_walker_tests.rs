// ---------------------------------------------------------------------------
// writer.rs — resource + imported-call IR walkers over every op/value arm.
// ---------------------------------------------------------------------------

use super::fixtures::*;
use super::*;
use crate::ast::LoopKind;
use crate::ir::{IrMatchCase, IrMatchPattern, IrParam, IrRecordUpdate, IrSourceLoc};
use crate::types::ParameterType;

fn file_local() -> IrValue {
    IrValue::LocalRef {
        name: "h".to_string(),
        type_: crate::types::ParameterType::parse("fs.File"),
    }
}

// A value that touches every IrValue arm the walkers recurse through, each
// carrying a `File` resource type so the resource walkers record it.
fn every_value() -> Vec<IrValue> {
    vec![
        IrValue::Const {
            type_: crate::types::ParameterType::parse("fs.File"),
            value: "0".to_string(),
        },
        IrValue::Local("a".to_string()),
        IrValue::Global("g".to_string()),
        IrValue::LocalRef {
            name: "a".to_string(),
            type_: crate::types::ParameterType::parse("fs.File"),
        },
        IrValue::FunctionRef {
            name: "dep.helper".to_string(),
            type_: crate::types::ParameterType::parse("fs.File"),
        },
        IrValue::Closure {
            name: "dep.helper".to_string(),
            type_: crate::types::ParameterType::parse("fs.File"),
            captures: vec![IrValue::Local("a".to_string())],
        },
        IrValue::Capture {
            index: 0,
            type_: crate::types::ParameterType::parse("fs.File"),
            by_ref: true,
        },
        IrValue::Call {
            target: "dep.helper".to_string(),
            args: vec![file_local()],
            loc: IrSourceLoc::default(),
            type_: crate::types::ParameterType::parse("fs.File"),
        },
        IrValue::CallResult {
            target: "dep.helper".to_string(),
            args: vec![file_local()],
            loc: IrSourceLoc::default(),
            type_: crate::types::ParameterType::parse("fs.File"),
        },
        IrValue::Constructor {
            type_: crate::types::ParameterType::parse("fs.File"),
            args: vec![file_local()],
        },
        IrValue::UnionWrap {
            union_type: crate::types::ParameterType::parse("U"),
            member_type: crate::types::ParameterType::parse("fs.File"),
            value: Box::new(file_local()),
        },
        IrValue::UnionExtract {
            type_: crate::types::ParameterType::parse("fs.File"),
            value: Box::new(file_local()),
        },
        IrValue::ResultIsOk {
            value: Box::new(file_local()),
        },
        IrValue::ResultValue {
            type_: crate::types::ParameterType::parse("fs.File"),
            value: Box::new(file_local()),
        },
        IrValue::ResultError {
            value: Box::new(file_local()),
        },
        IrValue::WithUpdate {
            type_: crate::types::ParameterType::parse("fs.File"),
            target: Box::new(file_local()),
            updates: vec![IrRecordUpdate {
                field: "x".to_string(),
                value: file_local(),
            }],
        },
        IrValue::ListLiteral {
            type_: crate::types::ParameterType::parse("fs.File"),
            values: vec![file_local()],
        },
        IrValue::MapLiteral {
            type_: crate::types::ParameterType::parse("fs.File"),
            entries: vec![(file_local(), file_local())],
        },
        IrValue::MemberAccess {
            target: Box::new(file_local()),
            member: "m".to_string(),
            type_: crate::types::ParameterType::parse("fs.File"),
        },
        IrValue::Binary {
            op: "+".to_string(),
            left: Box::new(file_local()),
            right: Box::new(file_local()),
            loc: IrSourceLoc::default(),
            type_: crate::types::ParameterType::parse("fs.File"),
        },
        IrValue::Unary {
            op: "-".to_string(),
            operand: Box::new(file_local()),
            loc: IrSourceLoc::default(),
            type_: crate::types::ParameterType::parse("fs.File"),
        },
    ]
}

// A function body touching every IrOp arm the walkers recurse through.
fn every_op_body() -> Vec<IrOp> {
    let call = IrValue::Call {
        target: "dep.helper".to_string(),
        args: every_value(),
        loc: IrSourceLoc::default(),
        type_: crate::types::ParameterType::parse("fs.File"),
    };
    vec![
        IrOp::Bind {
            mutable: true,
            name: "a".to_string(),
            type_: ParameterType::parse("fs.File"),
            value: Some(call.clone()),
            loc: IrSourceLoc::default(),
            explicit_type: true,
        },
        IrOp::Assign {
            name: "a".to_string(),
            value: file_local(),
            loc: IrSourceLoc::default(),
        },
        IrOp::AssignGlobal {
            name: "g".to_string(),
            value: file_local(),
            loc: IrSourceLoc::default(),
        },
        IrOp::StateAssign {
            resource: "a".to_string(),
            value: file_local(),
            loc: IrSourceLoc::default(),
        },
        IrOp::Eval {
            value: call.clone(),
            loc: IrSourceLoc::default(),
        },
        IrOp::Return {
            value: Some(file_local()),
            loc: IrSourceLoc::default(),
        },
        IrOp::ExitProgram {
            code: file_local(),
            loc: IrSourceLoc::default(),
        },
        IrOp::Fail {
            error: file_local(),
            loc: IrSourceLoc::default(),
        },
        IrOp::ExitLoop {
            kind: LoopKind::While,
            loc: IrSourceLoc::default(),
        },
        IrOp::ContinueLoop {
            kind: LoopKind::While,
            loc: IrSourceLoc::default(),
        },
        IrOp::If {
            condition: file_local(),
            then_body: vec![IrOp::Eval {
                value: file_local(),
                loc: IrSourceLoc::default(),
            }],
            else_body: vec![IrOp::Eval {
                value: file_local(),
                loc: IrSourceLoc::default(),
            }],
            loc: IrSourceLoc::default(),
        },
        IrOp::Match {
            value: file_local(),
            cases: vec![IrMatchCase {
                pattern: IrMatchPattern::Value(file_local()),
                guard: Some(file_local()),
                body: vec![IrOp::Eval {
                    value: file_local(),
                    loc: IrSourceLoc::default(),
                }],
                loc: IrSourceLoc::default(),
            }],
            loc: IrSourceLoc::default(),
        },
        IrOp::While {
            kind: LoopKind::While,
            condition: file_local(),
            body: vec![IrOp::Eval {
                value: file_local(),
                loc: IrSourceLoc::default(),
            }],
            loc: IrSourceLoc::default(),
        },
        IrOp::For {
            name: "i".to_string(),
            type_: ParameterType::parse("fs.File"),
            start: file_local(),
            end: file_local(),
            step: file_local(),
            body: vec![IrOp::Eval {
                value: file_local(),
                loc: IrSourceLoc::default(),
            }],
            loc: IrSourceLoc::default(),
        },
        IrOp::DoUntil {
            body: vec![IrOp::Eval {
                value: file_local(),
                loc: IrSourceLoc::default(),
            }],
            condition: file_local(),
            loc: IrSourceLoc::default(),
        },
        IrOp::ForEach {
            name: "e".to_string(),
            type_: ParameterType::parse("fs.File"),
            iterable: file_local(),
            body: vec![IrOp::Eval {
                value: file_local(),
                loc: IrSourceLoc::default(),
            }],
            loc: IrSourceLoc::default(),
        },
        IrOp::Trap {
            name: "err".to_string(),
            body: vec![IrOp::Eval {
                value: file_local(),
                loc: IrSourceLoc::default(),
            }],
            loc: IrSourceLoc::default(),
        },
    ]
}

fn corpus_function() -> IrFunction {
    let mut f = fn_named("corpus", "export", "function", "fs.File");
    f.params = vec![IrParam {
        name: "h".to_string(),
        type_: crate::types::ParameterType::parse("fs.File"),
        default: None,
        loc: loc(),
    }];
    f.body = every_op_body();
    f
}

#[test]
fn ir_uses_resource_type_walks_every_op_and_value_arm() {
    let mut project = empty_project("walk");
    project.functions = vec![corpus_function()];
    assert!(ir_uses_resource_type(&project));

    // ops_use_resource_type and value_uses_resource_type directly.
    let body = every_op_body();
    assert!(ops_use_resource_type(&body));
    for value in every_value() {
        // Every arm carries a File type somewhere, so the value walker sees it.
        assert!(
            value_uses_resource_type(&value)
                || matches!(value, IrValue::Local(_) | IrValue::Global(_))
        );
    }
}

// A `File`-free twin of `file_local`, so a body built from it makes every
// op/value arm evaluate to `false` and `.any(..)` must visit them all.
fn plain_local() -> IrValue {
    IrValue::Local("x".to_string())
}

fn every_plain_value() -> Vec<IrValue> {
    vec![
        IrValue::Const {
            type_: crate::types::ParameterType::parse("Integer"),
            value: "0".to_string(),
        },
        IrValue::Local("a".to_string()),
        IrValue::Global("g".to_string()),
        IrValue::LocalRef {
            name: "a".to_string(),
            type_: crate::types::ParameterType::parse("Integer"),
        },
        IrValue::FunctionRef {
            name: "f".to_string(),
            type_: crate::types::ParameterType::parse("Integer"),
        },
        IrValue::Closure {
            name: "f".to_string(),
            type_: crate::types::ParameterType::parse("Integer"),
            captures: vec![plain_local()],
        },
        IrValue::Capture {
            index: 0,
            type_: crate::types::ParameterType::parse("Integer"),
            by_ref: false,
        },
        IrValue::Call {
            target: "f".to_string(),
            args: vec![plain_local()],
            loc: IrSourceLoc::default(),
            type_: crate::types::ParameterType::parse("Integer"),
        },
        IrValue::CallResult {
            target: "f".to_string(),
            args: vec![plain_local()],
            loc: IrSourceLoc::default(),
            type_: crate::types::ParameterType::parse("Integer"),
        },
        IrValue::Constructor {
            type_: crate::types::ParameterType::parse("Integer"),
            args: vec![plain_local()],
        },
        IrValue::UnionWrap {
            union_type: crate::types::ParameterType::parse("U"),
            member_type: crate::types::ParameterType::parse("Integer"),
            value: Box::new(plain_local()),
        },
        IrValue::UnionExtract {
            type_: crate::types::ParameterType::parse("Integer"),
            value: Box::new(plain_local()),
        },
        IrValue::ResultIsOk {
            value: Box::new(plain_local()),
        },
        IrValue::ResultValue {
            type_: crate::types::ParameterType::parse("Integer"),
            value: Box::new(plain_local()),
        },
        IrValue::ResultError {
            value: Box::new(plain_local()),
        },
        IrValue::WithUpdate {
            type_: crate::types::ParameterType::parse("Integer"),
            target: Box::new(plain_local()),
            updates: vec![IrRecordUpdate {
                field: "x".to_string(),
                value: plain_local(),
            }],
        },
        IrValue::ListLiteral {
            type_: crate::types::ParameterType::parse("List OF Integer"),
            values: vec![plain_local()],
        },
        IrValue::MapLiteral {
            type_: crate::types::ParameterType::parse("Map OF String TO Integer"),
            entries: vec![(plain_local(), plain_local())],
        },
        IrValue::MemberAccess {
            target: Box::new(plain_local()),
            member: "m".to_string(),
            type_: crate::types::ParameterType::parse("Integer"),
        },
        IrValue::Binary {
            op: "+".to_string(),
            left: Box::new(plain_local()),
            right: Box::new(plain_local()),
            loc: IrSourceLoc::default(),
            type_: crate::types::ParameterType::parse("Integer"),
        },
        IrValue::Unary {
            op: "-".to_string(),
            operand: Box::new(plain_local()),
            loc: IrSourceLoc::default(),
            type_: crate::types::ParameterType::parse("Integer"),
        },
    ]
}

fn every_plain_op_body() -> Vec<IrOp> {
    vec![
        IrOp::Bind {
            mutable: true,
            name: "a".to_string(),
            type_: ParameterType::Integer,
            value: Some(plain_local()),
            loc: IrSourceLoc::default(),
            explicit_type: true,
        },
        IrOp::Assign {
            name: "a".to_string(),
            value: plain_local(),
            loc: IrSourceLoc::default(),
        },
        IrOp::AssignGlobal {
            name: "g".to_string(),
            value: plain_local(),
            loc: IrSourceLoc::default(),
        },
        IrOp::StateAssign {
            resource: "a".to_string(),
            value: plain_local(),
            loc: IrSourceLoc::default(),
        },
        IrOp::Eval {
            value: plain_local(),
            loc: IrSourceLoc::default(),
        },
        IrOp::Return {
            value: Some(plain_local()),
            loc: IrSourceLoc::default(),
        },
        IrOp::ExitProgram {
            code: plain_local(),
            loc: IrSourceLoc::default(),
        },
        IrOp::Fail {
            error: plain_local(),
            loc: IrSourceLoc::default(),
        },
        IrOp::ExitLoop {
            kind: LoopKind::While,
            loc: IrSourceLoc::default(),
        },
        IrOp::ContinueLoop {
            kind: LoopKind::While,
            loc: IrSourceLoc::default(),
        },
        IrOp::If {
            condition: plain_local(),
            then_body: vec![IrOp::Eval {
                value: plain_local(),
                loc: IrSourceLoc::default(),
            }],
            else_body: vec![IrOp::Eval {
                value: plain_local(),
                loc: IrSourceLoc::default(),
            }],
            loc: IrSourceLoc::default(),
        },
        IrOp::Match {
            value: plain_local(),
            cases: vec![IrMatchCase {
                pattern: IrMatchPattern::Value(plain_local()),
                guard: Some(plain_local()),
                body: vec![IrOp::Eval {
                    value: plain_local(),
                    loc: IrSourceLoc::default(),
                }],
                loc: IrSourceLoc::default(),
            }],
            loc: IrSourceLoc::default(),
        },
        IrOp::While {
            kind: LoopKind::While,
            condition: plain_local(),
            body: vec![IrOp::Eval {
                value: plain_local(),
                loc: IrSourceLoc::default(),
            }],
            loc: IrSourceLoc::default(),
        },
        IrOp::For {
            name: "i".to_string(),
            type_: ParameterType::Integer,
            start: plain_local(),
            end: plain_local(),
            step: plain_local(),
            body: vec![IrOp::Eval {
                value: plain_local(),
                loc: IrSourceLoc::default(),
            }],
            loc: IrSourceLoc::default(),
        },
        IrOp::DoUntil {
            body: vec![IrOp::Eval {
                value: plain_local(),
                loc: IrSourceLoc::default(),
            }],
            condition: plain_local(),
            loc: IrSourceLoc::default(),
        },
        IrOp::ForEach {
            name: "e".to_string(),
            type_: ParameterType::Integer,
            iterable: plain_local(),
            body: vec![IrOp::Eval {
                value: plain_local(),
                loc: IrSourceLoc::default(),
            }],
            loc: IrSourceLoc::default(),
        },
        IrOp::Trap {
            name: "err".to_string(),
            body: vec![IrOp::Eval {
                value: plain_local(),
                loc: IrSourceLoc::default(),
            }],
            loc: IrSourceLoc::default(),
        },
    ]
}

#[test]
fn resource_walkers_visit_every_arm_when_no_resource_present() {
    // `.any(..)` short-circuits on the first `true`, so a resource-free body
    // is required to make the walkers traverse every op/value match arm.
    let body = every_plain_op_body();
    assert!(!ops_use_resource_type(&body));
    for value in every_plain_value() {
        assert!(!value_uses_resource_type(&value));
    }
    // The imported-call walker also visits every arm (no import matches).
    let empty: std::collections::HashMap<String, [u8; ABI_HASH_LEN]> =
        std::collections::HashMap::new();
    let mut used = std::collections::HashSet::new();
    for op in &body {
        collect_imported_calls_op(op, &empty, &mut used);
    }
    for value in every_plain_value() {
        collect_imported_calls_value(&value, &empty, &mut used);
    }
    assert!(used.is_empty());
    // collect_resource_names_in_ops/value over a resource-free body records nothing.
    let mut names = std::collections::HashSet::new();
    let mut record = |type_: &str, names: &mut std::collections::HashSet<String>| {
        if is_resource_type_name(type_) {
            names.insert(type_.to_string());
        }
    };
    collect_resource_names_in_ops(&body, &mut names, &mut record);
    for value in every_plain_value() {
        collect_resource_names_in_value(&value, &mut names, &mut record);
    }
    assert!(names.is_empty());
}

#[test]
fn collect_resource_names_walk_gathers_file_over_every_arm() {
    // Same coverage over the name-collecting walkers, but with File present.
    let body = every_op_body();
    let mut names = std::collections::HashSet::new();
    let mut record = |type_: &str, names: &mut std::collections::HashSet<String>| {
        if is_resource_type_name(type_) {
            names.insert(type_.to_string());
        }
    };
    collect_resource_names_in_ops(&body, &mut names, &mut record);
    assert!(names.contains("fs.File"));
    for value in every_value() {
        collect_resource_names_in_value(&value, &mut names, &mut record);
    }
    assert!(names.contains("fs.File"));
}

#[test]
fn collect_resource_type_names_gathers_file() {
    let mut project = empty_project("walk");
    project.functions = vec![corpus_function()];
    let mut names = std::collections::HashSet::new();
    collect_resource_type_names(&project, &mut names);
    assert!(names.contains("fs.File"));
}

#[test]
fn lower_project_emits_file_resource_table_from_body() {
    // The resource walker drives the RESOURCE_TABLE, so lowering a body that
    // uses File must emit a standard file resource entry.
    let mut project = empty_project("walk");
    project.functions = vec![corpus_function()];
    let metadata = BinaryReprMetadata::new("walk".to_string(), "1".to_string());
    let lowered = lower_project(&project, &metadata).expect("lower");
    assert!(lowered
        .resources
        .entries
        .iter()
        .any(|e| e.close_function_id == BUILTIN_FS_CLOSE_FUNCTION_ID));
}

#[test]
fn collect_imported_calls_records_used_symbols() {
    // Build a fake imported-hash map naming `dep.helper`, then walk a body
    // that references it in every recursive position.
    let mut imported = std::collections::HashMap::new();
    imported.insert("dep.helper".to_string(), hash_bytes(b"helper"));
    let mut used = std::collections::HashSet::new();
    for op in every_op_body() {
        collect_imported_calls_op(&op, &imported, &mut used);
    }
    assert!(used.contains("dep.helper"));
}

#[test]
fn socket_and_listener_resources_are_emitted_when_used() {
    let mut project = empty_project("net");
    let mut f = fn_named("takes", "export", "sub", "Nothing");
    f.params = vec![
        IrParam {
            name: "s".to_string(),
            type_: crate::types::ParameterType::parse("tcp.Socket"),
            default: None,
            loc: loc(),
        },
        IrParam {
            name: "l".to_string(),
            type_: crate::types::ParameterType::parse("tcp.Listener"),
            default: None,
            loc: loc(),
        },
    ];
    project.functions = vec![f];
    let metadata = BinaryReprMetadata::new("net".to_string(), "1".to_string());
    let lowered = lower_project(&project, &metadata).expect("lower");
    // Socket + Listener both produce resource entries.
    assert_eq!(lowered.resources.entries.len(), 2);
}
