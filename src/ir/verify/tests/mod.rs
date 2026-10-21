use super::{
    check, collect_diagnostics, collect_source_diagnostics, is_thread_type, read_only_record_type,
    resources, TypeEnv,
};

mod captures_struct_slots_and_state_types;
mod cbuffers_and_thread_messages;
mod coercion_closure_walk_and_defaultability;
mod declarations_and_resource_states;
mod handler_reads_nested;
mod literals_bindings_and_loops;
mod match_coverage_moves_and_annotations;
mod matches_with_updates_and_money;
mod members_captures_unions_and_operators;
mod named_loop_exits;
mod state_bindings_and_native_links;
mod traps_diagnostics_and_capture_positions;
use crate::ir::{
    IrBinding, IrField, IrFunction, IrMatchCase, IrMatchPattern, IrOp, IrParam, IrProject,
    IrSourceLoc, IrType, IrValue, IrVariant,
};
use crate::operators::{BinaryOp, UnaryOp};
use crate::types::ParameterType;
use std::collections::HashMap;

fn project(functions: Vec<IrFunction>, types: Vec<IrType>) -> IrProject {
    crate::ir::test_support::project_fixture("t", functions, types)
}

/// Rule ids of every diagnostic collected for a project.
fn rules(project: &IrProject) -> Vec<String> {
    collect_diagnostics(project)
        .into_iter()
        .map(|d| d.rule)
        .collect()
}

/// Assert `project` yields no diagnostics.
fn accept(project: &IrProject) {
    let diags = collect_diagnostics(project);
    assert!(
        diags.is_empty(),
        "expected clean, got {:?}",
        diags.iter().map(|d| &d.rule).collect::<Vec<_>>()
    );
}

/// Assert `project` yields a diagnostic with `rule`.
fn expect_rule(project: &IrProject, rule: &str) {
    let got = rules(project);
    assert!(
        got.iter().any(|r| r == rule),
        "expected {rule}, got {got:?}"
    );
}

/// Assert `project` yields NO diagnostic with `rule`. Weaker than `expect_clean`
/// on purpose: it pins that one specific rule stopped firing without also
/// asserting the project is diagnostic-free, so it survives unrelated rules.
fn expect_no_rule(project: &IrProject, rule: &str) {
    let got = rules(project);
    assert!(
        !got.iter().any(|r| r == rule),
        "expected no {rule}, got {got:?}"
    );
}

fn ret(value: IrValue) -> IrOp {
    IrOp::Return {
        value: Some(value),
        loc: IrSourceLoc::default(),
    }
}

fn ret_none() -> IrOp {
    IrOp::Return {
        value: None,
        loc: IrSourceLoc::default(),
    }
}

fn const_of(ty: &str, v: &str) -> IrValue {
    IrValue::Const {
        type_: crate::types::ParameterType::parse(ty),
        value: v.to_string(),
    }
}

fn binary(op: BinaryOp, left: IrValue, right: IrValue, ty: &str) -> IrValue {
    IrValue::Binary {
        op,
        left: Box::new(left),
        right: Box::new(right),
        type_: crate::types::ParameterType::parse(ty),
        loc: IrSourceLoc::default(),
    }
}

fn unary(op: UnaryOp, operand: IrValue, ty: &str) -> IrValue {
    IrValue::Unary {
        op,
        operand: Box::new(operand),
        type_: crate::types::ParameterType::parse(ty),
        loc: IrSourceLoc::default(),
    }
}

fn bind(name: &str, ty: &str, value: Option<IrValue>, explicit: bool, mutable: bool) -> IrOp {
    IrOp::Bind {
        mutable,
        name: name.to_string(),
        type_: ParameterType::parse(ty),
        value,
        explicit_type: explicit,
        loc: IrSourceLoc::default(),
    }
}

/// A record whose fields carry the given `(name, type)` pairs.
fn record_typed(name: &str, fields: &[(&str, &str)]) -> IrType {
    IrType {
        kind: "type".to_string(),
        visibility: "export".to_string(),
        name: name.to_string(),
        fields: fields
            .iter()
            .map(|(n, t)| IrField {
                visibility: None,
                name: (*n).to_string(),
                type_: crate::types::ParameterType::parse(*t),
                loc: IrSourceLoc::default(),
            })
            .collect(),
        includes: vec![],
        variants: vec![],
        members: vec![],
        loc: IrSourceLoc::default(),
        file: String::new(),
    }
}

fn enum_type(name: &str, members: &[&str]) -> IrType {
    IrType {
        kind: "enum".to_string(),
        visibility: "export".to_string(),
        name: name.to_string(),
        fields: vec![],
        includes: vec![],
        variants: vec![],
        members: members
            .iter()
            .map(|m| crate::ir::IrEnumMember {
                name: (*m).to_string(),
            })
            .collect(),
        loc: IrSourceLoc::default(),
        file: String::new(),
    }
}

fn binding(
    name: &str,
    ty: &str,
    value: Option<IrValue>,
    mutable: bool,
    explicit: bool,
) -> IrBinding {
    IrBinding {
        name: name.to_string(),
        visibility: "export".to_string(),
        mutable,
        type_: crate::types::ParameterType::parse(ty),
        value,
        loc: IrSourceLoc::default(),
        file: "src/main.mfb".to_string(),
        explicit_type: explicit,
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
        returns: crate::types::ParameterType::parse(returns),
        body,
        file: "src/main.mfb".to_string(),
        resource_owners: HashMap::new(),
        loc: IrSourceLoc::default(),
    }
}

fn param(name: &str, type_: &str, default: Option<IrValue>) -> IrParam {
    IrParam {
        name: name.to_string(),
        type_: crate::types::ParameterType::parse(type_),
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
                type_: crate::types::ParameterType::parse("Integer"),
                loc: IrSourceLoc::default(),
            })
            .collect(),
        includes: vec![],
        variants: vec![],
        members: vec![],
        loc: IrSourceLoc::default(),
        file: String::new(),
    }
}

fn int_const(v: &str) -> IrValue {
    IrValue::Const {
        type_: crate::types::ParameterType::parse("Integer"),
        value: v.to_string(),
    }
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
                type_: crate::types::ParameterType::parse(v),
                fields: vec![],
                loc: IrSourceLoc::default(),
            })
            .collect(),
        members: vec![],
        loc: IrSourceLoc::default(),
        file: String::new(),
    }
}

// --- match -----------------------------------------------------------------

fn union_variant_case(name: &str, body: Vec<IrOp>) -> IrMatchCase {
    IrMatchCase {
        pattern: IrMatchPattern::Value(IrValue::Local(name.to_string())),
        guard: None,
        body,
        loc: IrSourceLoc::default(),
    }
}

// --- values.rs literal/money/member/comparability arms (plan-68-D4) --------

fn money_const(v: &str) -> IrValue {
    const_of("Money", v)
}

fn eval(value: IrValue) -> IrOp {
    IrOp::Eval {
        value,
        loc: IrSourceLoc::default(),
    }
}

// --- calls.rs: STATE agreement (plan-68-D2) --------------------------------

/// A resource binding whose owner is recorded (so it is not rejected as a
/// non-`RES` resource hold), typed `type_`, initialized from `value`.
fn res_bind_owned(f: &mut IrFunction, name: &str, type_: &str, value: Option<IrValue>) -> IrOp {
    f.resource_owners.insert(
        name.to_string(),
        crate::ir::resource_escape::ResOwner::Local,
    );
    bind(name, type_, value, true, false)
}

fn transfer_call(handle: &str, res: Option<&str>) -> IrOp {
    let mut args = vec![IrValue::Local(handle.to_string())];
    if let Some(res) = res {
        args.push(IrValue::Local(res.to_string()));
    }
    IrOp::Eval {
        value: IrValue::Call {
            target: crate::codegen::builtins::thread::TRANSFER_RESOURCE.to_string(),
            args,
            type_: crate::types::ParameterType::parse("Nothing"),
            loc: IrSourceLoc::default(),
        },
        loc: IrSourceLoc::default(),
    }
}

// --- link functions --------------------------------------------------------

fn link_fn() -> crate::ir::IrLinkFunction {
    crate::ir::IrLinkFunction {
        alias: "lib".to_string(),
        name: "open".to_string(),
        library: "sqlite3".to_string(),
        symbol: "sqlite3_open".to_string(),
        params: vec![(
            "path".to_string(),
            crate::types::ParameterType::parse("String"),
        )],
        return_type: crate::types::ParameterType::parse("Integer"),
        return_resource: false,
        return_state_type: None,
        abi_slots: vec![crate::ir::IrAbiSlot {
            name: "path".to_string(),
            ctype: crate::types::ParameterType::parse("CString"),
            direction: crate::ir::AbiDirection::In,
        }],
        abi_return_name: "value".to_string(),
        abi_return_ctype: crate::types::ParameterType::parse("CInt32"),
        consts: vec![],
        bind_in: vec![],
        bind_state: None,
        bind_state_resource: None,
        success_on: None,
        // plan-50-H: the result is whatever `RETURN <expr>` names; a bare Var over
        // the ABI return is the `AS value CInt32` + `RETURN value` passthrough.
        result: Some(crate::ir::IrLinkExpr::Var("value".to_string())),
        free: None,
        buffers: vec![],
        result_length: None,
    }
}

// --- CSTRUCT (plan-50-B) ---------------------------------------------------

fn cstruct(name: &str, fields: &[(&str, &str)]) -> crate::ir::IrCStruct {
    crate::ir::IrCStruct {
        alias: "lib".to_string(),
        name: name.to_string(),
        maps_to: crate::types::ParameterType::parse("Rec"),
        fields: fields
            .iter()
            .map(|(n, t)| crate::ir::IrCStructField {
                name: (*n).to_string(),
                ctype: crate::types::ParameterType::parse(t),
            })
            .collect(),
    }
}

fn project_with_cstructs(structs: Vec<crate::ir::IrCStruct>) -> IrProject {
    let mut p = project(vec![func_returns("run", "Nothing", vec![], vec![])], vec![]);
    p.link_cstructs = structs;
    p
}

// --- link.rs native arms (plan-68-D3) --------------------------------------

fn abi_slot(name: &str, ctype: &str, dir: crate::ir::AbiDirection) -> crate::ir::IrAbiSlot {
    crate::ir::IrAbiSlot {
        name: name.to_string(),
        ctype: crate::types::ParameterType::parse(ctype),
        direction: dir,
    }
}

fn project_with_link(
    lf: crate::ir::IrLinkFunction,
    cstructs: Vec<crate::ir::IrCStruct>,
) -> IrProject {
    let mut p = project(vec![func_returns("run", "Nothing", vec![], vec![])], vec![]);
    p.link_functions = vec![lf];
    p.link_cstructs = cstructs;
    p
}

// --- more builtin package overload branches --------------------------------

fn eval_call(target: &str, args: Vec<IrValue>) -> IrOp {
    IrOp::Eval {
        value: IrValue::Call {
            target: target.to_string(),
            args,
            type_: crate::types::ParameterType::parse("Unknown"),
            loc: IrSourceLoc::default(),
        },
        loc: IrSourceLoc::default(),
    }
}

// --- resource moves inside nested blocks -----------------------------------

fn close_eval(h: &str) -> IrOp {
    IrOp::Eval {
        value: IrValue::Call {
            target: "fs.close".to_string(),
            args: vec![IrValue::Local(h.to_string())],
            type_: crate::types::ParameterType::parse("Nothing"),
            loc: IrSourceLoc::default(),
        },
        loc: IrSourceLoc::default(),
    }
}

fn owner_fn(name: &str, ret: &str, body: Vec<IrOp>, owners: &[&str]) -> IrFunction {
    let mut f = func_returns(name, ret, vec![], body);
    for o in owners {
        f.resource_owners.insert(
            (*o).to_string(),
            crate::ir::resource_escape::ResOwner::Local,
        );
    }
    f
}

// --- plan-156-C: a parent Thread view is a resource --------------------------

const THREAD_VIEW: &str = "Thread OF Integer TO Integer";

const WORKER_VIEW: &str = "ThreadWorker OF Integer TO Integer";

/// The lowered shape of `LET a = <scrutinee> TRAP(e) … END TRAP` with `handler`
/// as the handler's ops (`lower_inline_trap`).
fn inline_trap_ops(scrutinee: IrValue, handler: Vec<IrOp>) -> Vec<IrOp> {
    let res = || IrValue::Local("$trap_res0".to_string());
    vec![
        IrOp::Bind {
            mutable: false,
            name: "$trap_res0".to_string(),
            type_: ParameterType::result_of(ParameterType::Integer),
            value: Some(scrutinee),
            explicit_type: false,
            loc: IrSourceLoc::default(),
        },
        bind("$trap_val0", "Integer", None, false, true),
        IrOp::If {
            condition: IrValue::ResultIsOk {
                value: Box::new(res()),
            },
            then_body: vec![IrOp::Assign {
                name: "$trap_val0".to_string(),
                value: IrValue::ResultValue {
                    type_: ParameterType::Integer,
                    value: Box::new(res()),
                },
                loc: IrSourceLoc::default(),
            }],
            else_body: handler,
            loc: IrSourceLoc::default(),
        },
        bind(
            "a",
            "Integer",
            Some(IrValue::Local("$trap_val0".to_string())),
            false,
            false,
        ),
        ret(IrValue::Local("a".to_string())),
    ]
}

fn recover_zero() -> IrOp {
    IrOp::Assign {
        name: "$trap_val0".to_string(),
        value: int_const("0"),
        loc: IrSourceLoc::default(),
    }
}

/// A record that is NOT thread-sendable because it holds a worker's own view.
/// (plan-156-D: a parent `Thread` is a sendable resource now, so it no longer
/// makes a record unsendable — a `ThreadWorker` still does.)
fn unsendable_record() -> IrType {
    record_typed(
        "BadMessage",
        &[("handle", "ThreadWorker OF String TO Integer")],
    )
}

fn bind_in(slot: &str, fields: &[(&str, Option<&str>, Option<i64>)]) -> crate::ir::IrBindIn {
    crate::ir::IrBindIn {
        slot: slot.to_string(),
        fields: fields
            .iter()
            .map(|(name, param, literal)| crate::ir::IrBindInField {
                name: (*name).to_string(),
                param: param.map(str::to_string),
                literal: *literal,
            })
            .collect(),
    }
}

// --- branch arms start from the construct's entry state ---------------------

fn read_of(name: &str) -> IrOp {
    eval(IrValue::Call {
        target: "fs.readLine".to_string(),
        args: vec![IrValue::Local(name.to_string())],
        type_: ParameterType::parse("String"),
        loc: IrSourceLoc::default(),
    })
}

fn if_op(condition: IrValue, then_body: Vec<IrOp>, else_body: Vec<IrOp>) -> IrOp {
    IrOp::If {
        condition,
        then_body,
        else_body,
        loc: IrSourceLoc::default(),
    }
}

fn count_rule(got: &[String], rule: &str) -> usize {
    got.iter().filter(|r| *r == rule).count()
}
