//! Unit tests for place copy forwarding (plan-149). Each builds NIR by hand,
//! runs the row, and asserts the body after it: the forwarding cases check the
//! binding is gone and its reads name the place; the declining cases check the
//! body is untouched.

use super::super::local_rewrites::testutil::*;
use super::*;
use crate::optimizer::{with_opt_level, OptLevel};
use crate::target::shared::nir::{NirParam, NirRecordUpdate, NirSourceLoc};

fn list_int() -> ParameterType {
    ParameterType::ListOf(Box::new(ParameterType::Integer))
}

fn rec_type() -> ParameterType {
    ParameterType::named("R")
}

fn function(body: Vec<NirOp>) -> NirFunction {
    NirFunction {
        name: "f".to_string(),
        visibility: "private".to_string(),
        kind: "function".to_string(),
        isolated: false,
        params: vec![],
        returns: ParameterType::Integer,
        body,
        file: "main.mfb".to_string(),
        resource_owners: HashMap::new(),
    }
}

fn param(name: &str, type_: ParameterType) -> NirParam {
    NirParam {
        name: name.to_string(),
        type_,
        default: None,
    }
}

fn bind(name: &str, type_: ParameterType, value: NirValue) -> NirOp {
    NirOp::Bind {
        mutable: false,
        name: name.to_string(),
        type_,
        value: Some(value),
    }
}

fn bind_mut(name: &str, type_: ParameterType, value: NirValue) -> NirOp {
    NirOp::Bind {
        mutable: true,
        name: name.to_string(),
        type_,
        value: Some(value),
    }
}

fn member(target: NirValue, field: &str) -> NirValue {
    NirValue::MemberAccess {
        target: Box::new(target),
        member: field.to_string(),
    }
}

fn call(target: &str, args: Vec<NirValue>) -> NirValue {
    NirValue::Call {
        target: target.to_string(),
        args,
        loc: NirSourceLoc::default(),
    }
}

fn with_field(target: NirValue, field: &str, value: NirValue) -> NirValue {
    NirValue::WithUpdate {
        type_: rec_type(),
        target: Box::new(target),
        updates: vec![NirRecordUpdate {
            field: field.to_string(),
            value,
        }],
    }
}

fn assign(name: &str, value: NirValue) -> NirOp {
    NirOp::Assign {
        name: name.to_string(),
        value,
    }
}

fn eval(value: NirValue) -> NirOp {
    NirOp::Eval { value }
}

/// `MUT rec AS R = mk()` — the owned record local every record case starts from.
fn rec_local() -> NirOp {
    bind_mut("rec", rec_type(), call("mk", vec![]))
}

fn run(module: &mut NirModule, level: u8) {
    with_opt_level(OptLevel(level), || forward(module));
}

/// Render a value, deep enough to tell a forwarded read from the original.
fn render(value: &NirValue) -> String {
    match value {
        NirValue::Local(name) => name.clone(),
        NirValue::Global { name, .. } => format!("global {name}"),
        NirValue::Const { value, .. } => value.clone(),
        NirValue::MemberAccess { target, member } => format!("{}.{member}", render(target)),
        NirValue::Call { target, args, .. } => format!(
            "{target}({})",
            args.iter().map(render).collect::<Vec<_>>().join(", ")
        ),
        NirValue::WithUpdate {
            target, updates, ..
        } => format!(
            "WITH {} {{ {} }}",
            render(target),
            updates
                .iter()
                .map(|u| format!("{} := {}", u.field, render(&u.value)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        other => shape(other),
    }
}

fn render_ops(ops: &[NirOp]) -> Vec<String> {
    ops.iter()
        .map(|op| match op {
            NirOp::Bind { name, value, .. } => {
                format!(
                    "LET {name} = {}",
                    value.as_ref().map(render).unwrap_or_default()
                )
            }
            NirOp::Assign { name, value } => format!("{name} = {}", render(value)),
            NirOp::StoreGlobal { name, value, .. } => {
                format!(
                    "global {name} = {}",
                    value.as_ref().map(render).unwrap_or_default()
                )
            }
            NirOp::StateAssign { resource, value } => {
                format!("{resource}.state = {}", render(value))
            }
            NirOp::Eval { value } => format!("eval {}", render(value)),
            NirOp::Return { value } => {
                format!("RETURN {}", value.as_ref().map(render).unwrap_or_default())
            }
            NirOp::For { name, body, .. } => {
                format!("FOR {name} [{}]", render_ops(body).join("; "))
            }
            NirOp::Trap { body, .. } => format!("TRAP [{}]", render_ops(body).join("; ")),
            _ => "op".to_string(),
        })
        .collect()
}

fn body_after(body: Vec<NirOp>, level: u8) -> Vec<String> {
    let mut module = test_module(vec![function(body)]);
    run(&mut module, level);
    render_ops(&module.functions[0].body)
}

/// The benchmark's `set` shape: `cur` is read by `get`, then written through by
/// the owner's own `WITH`. Forwarded, the NIR is exactly the direct form.
fn set_shape() -> Vec<NirOp> {
    vec![
        rec_local(),
        bind("cur", list_int(), member(local("rec"), "xs")),
        bind(
            "v",
            ParameterType::Integer,
            call("collections.get", vec![local("cur"), int_const("0")]),
        ),
        assign(
            "rec",
            with_field(
                local("rec"),
                "xs",
                call(
                    "collections.set",
                    vec![local("cur"), int_const("0"), local("v")],
                ),
            ),
        ),
        NirOp::Return {
            value: Some(int_const("0")),
        },
    ]
}

#[test]
fn the_benchmark_set_shape_is_forwarded() {
    assert_eq!(
        body_after(set_shape(), 1),
        vec![
            "LET rec = mk()",
            "LET v = collections.get(rec.xs, 0)",
            "rec = WITH rec { xs := collections.set(rec.xs, 0, v) }",
            "RETURN 0",
        ]
    );
}

#[test]
fn the_remove_at_shape_is_forwarded() {
    let body = vec![
        rec_local(),
        bind("cur", list_int(), member(local("rec"), "xs")),
        assign(
            "rec",
            with_field(
                local("rec"),
                "xs",
                call("collections.removeAt", vec![local("cur"), int_const("0")]),
            ),
        ),
    ];
    assert_eq!(
        body_after(body, 1),
        vec![
            "LET rec = mk()",
            "rec = WITH rec { xs := collections.removeAt(rec.xs, 0) }",
        ]
    );
}

#[test]
fn a_nested_path_is_forwarded() {
    let body = vec![
        rec_local(),
        bind("cur", list_int(), member(member(local("rec"), "a"), "xs")),
        NirOp::Return {
            value: Some(call("len", vec![local("cur")])),
        },
    ];
    assert_eq!(
        body_after(body, 1),
        vec!["LET rec = mk()", "RETURN len(rec.a.xs)"]
    );
}

#[test]
fn a_bare_local_is_forwarded() {
    let body = vec![
        bind_mut("nums", list_int(), call("mk", vec![])),
        bind("snap", list_int(), local("nums")),
        assign(
            "nums",
            call(
                "collections.set",
                vec![local("snap"), int_const("0"), int_const("9")],
            ),
        ),
    ];
    assert_eq!(
        body_after(body, 1),
        vec!["LET nums = mk()", "nums = collections.set(nums, 0, 9)",]
    );
}

#[test]
fn a_read_only_window_reaching_the_block_end_is_forwarded() {
    let body = vec![
        rec_local(),
        bind("cur", list_int(), member(local("rec"), "xs")),
        eval(call("io.print", vec![local("cur")])),
        NirOp::Return {
            value: Some(call("len", vec![local("cur")])),
        },
    ];
    assert_eq!(
        body_after(body, 1),
        vec![
            "LET rec = mk()",
            "eval io.print(rec.xs)",
            "RETURN len(rec.xs)",
        ]
    );
}

fn for_loop(name: &str, body: Vec<NirOp>) -> NirOp {
    NirOp::For {
        name: name.to_string(),
        type_: ParameterType::Integer,
        start: int_const("0"),
        end: int_const("3"),
        step: int_const("1"),
        body,
        loc: NirSourceLoc::default(),
    }
}

#[test]
fn a_read_inside_a_nested_for_in_the_window_is_forwarded() {
    let body = vec![
        rec_local(),
        bind("cur", list_int(), member(local("rec"), "xs")),
        for_loop(
            "k",
            vec![eval(call(
                "collections.get",
                vec![local("cur"), local("k")],
            ))],
        ),
        assign(
            "rec",
            with_field(
                local("rec"),
                "xs",
                call("collections.removeAt", vec![local("cur"), int_const("0")]),
            ),
        ),
    ];
    assert_eq!(
        body_after(body, 1),
        vec![
            "LET rec = mk()",
            "FOR k [eval collections.get(rec.xs, k)]",
            "rec = WITH rec { xs := collections.removeAt(rec.xs, 0) }",
        ]
    );
}

/// The forward is counted once per dropped binding.
#[test]
fn a_forward_is_counted() {
    let before =
        crate::optimizer::stats::PLACE_COPIES_FORWARDED.load(std::sync::atomic::Ordering::Relaxed);
    let mut module = test_module(vec![function(set_shape())]);
    run(&mut module, 1);
    let after =
        crate::optimizer::stats::PLACE_COPIES_FORWARDED.load(std::sync::atomic::Ordering::Relaxed);
    assert!(after > before, "the forward bumps its counter");
}

// ---- declines: each keeps its binding untouched ----

fn assert_kept(body: Vec<NirOp>) {
    let expected = render_ops(&body);
    assert_eq!(body_after(body, 1), expected, "the copy must be kept");
}

/// W1: the owner is written between the binding and a read, so the read must
/// see the old value.
#[test]
fn an_owner_write_between_the_bind_and_a_read_keeps_the_copy() {
    assert_kept(vec![
        rec_local(),
        bind("cur", list_int(), member(local("rec"), "xs")),
        assign("rec", call("mk", vec![])),
        NirOp::Return {
            value: Some(call("len", vec![local("cur")])),
        },
    ]);
}

/// A read after the owner write is the same W1 violation from the other side.
#[test]
fn a_read_after_the_owner_write_keeps_the_copy() {
    assert_kept(vec![
        rec_local(),
        bind("cur", list_int(), member(local("rec"), "xs")),
        assign(
            "rec",
            with_field(
                local("rec"),
                "xs",
                call("collections.removeAt", vec![local("cur"), int_const("0")]),
            ),
        ),
        NirOp::Return {
            value: Some(call("len", vec![local("cur")])),
        },
    ]);
}

/// W2: `FOR k … get(cur) … rec = … NEXT` — the second iteration's read would
/// see the first iteration's write.
#[test]
fn an_owner_write_nested_in_the_last_op_keeps_the_copy() {
    assert_kept(vec![
        rec_local(),
        bind("cur", list_int(), member(local("rec"), "xs")),
        for_loop(
            "k",
            vec![
                eval(call("collections.get", vec![local("cur"), local("k")])),
                assign("rec", call("mk", vec![])),
            ],
        ),
    ]);
}

/// W3: a `TRAP` handler runs after the failing statement, which may be past
/// the owner's write.
#[test]
fn a_read_in_a_trap_handler_keeps_the_copy() {
    assert_kept(vec![
        rec_local(),
        bind("cur", list_int(), member(local("rec"), "xs")),
        NirOp::Trap {
            name: "e".to_string(),
            body: vec![NirOp::Return {
                value: Some(call("len", vec![local("cur")])),
            }],
        },
    ]);
}

fn closure_capturing(name: &str) -> NirValue {
    NirValue::Closure {
        name: "$lambda0".to_string(),
        type_: ParameterType::Integer,
        captures: vec![local(name)],
    }
}

#[test]
fn a_captured_binding_keeps_the_copy() {
    assert_kept(vec![
        rec_local(),
        bind("cur", list_int(), member(local("rec"), "xs")),
        eval(call(
            "collections.forEach",
            vec![int_const("0"), closure_capturing("cur")],
        )),
    ]);
}

#[test]
fn an_address_taken_owner_keeps_the_copy() {
    assert_kept(vec![
        rec_local(),
        bind("cur", list_int(), member(local("rec"), "xs")),
        eval(call(
            "collections.forEach",
            vec![NirValue::Closure {
                name: "$lambda0".to_string(),
                type_: ParameterType::Integer,
                captures: vec![NirValue::LocalRef {
                    name: "rec".to_string(),
                    type_: rec_type(),
                }],
            }],
        )),
        NirOp::Return {
            value: Some(call("len", vec![local("cur")])),
        },
    ]);
}

#[test]
fn a_captured_owner_keeps_the_copy() {
    assert_kept(vec![
        rec_local(),
        bind("cur", list_int(), member(local("rec"), "xs")),
        eval(call(
            "collections.forEach",
            vec![int_const("0"), closure_capturing("rec")],
        )),
        NirOp::Return {
            value: Some(call("len", vec![local("cur")])),
        },
    ]);
}

/// A bare local copied from a parameter: `RETURN snap` → `RETURN s` would hand
/// the return-move a parameter it may not move.
#[test]
fn a_bare_parameter_source_keeps_the_copy() {
    let body = vec![
        bind("snap", list_int(), local("s")),
        NirOp::Return {
            value: Some(local("snap")),
        },
    ];
    let expected = render_ops(&body);
    let mut f = function(body);
    f.params = vec![param("s", list_int())];
    let mut module = test_module(vec![f]);
    run(&mut module, 1);
    assert_eq!(render_ops(&module.functions[0].body), expected);
}

/// A member path of a parameter is fine: a parameter is never assigned, and a
/// member-access `RETURN` copies (plan-149-A §2).
#[test]
fn a_member_path_of_a_parameter_is_forwarded() {
    let body = vec![
        bind("cur", list_int(), member(local("p"), "xs")),
        NirOp::Return {
            value: Some(local("cur")),
        },
    ];
    let mut f = function(body);
    f.params = vec![param("p", rec_type())];
    let mut module = test_module(vec![f]);
    run(&mut module, 1);
    assert_eq!(render_ops(&module.functions[0].body), vec!["RETURN p.xs"]);
}

#[test]
fn a_scalar_binding_keeps_the_copy() {
    assert_kept(vec![
        rec_local(),
        bind("n", ParameterType::Integer, member(local("rec"), "count")),
        NirOp::Return {
            value: Some(local("n")),
        },
    ]);
}

/// An owner bound twice (shadowing somewhere) cannot be told apart by name.
#[test]
fn a_rebound_owner_keeps_the_copy() {
    assert_kept(vec![
        rec_local(),
        bind("cur", list_int(), member(local("rec"), "xs")),
        for_loop("k", vec![rec_local()]),
        NirOp::Return {
            value: Some(call("len", vec![local("cur")])),
        },
    ]);
}

#[test]
fn level_zero_keeps_the_copy() {
    let body = set_shape();
    let expected = render_ops(&body);
    assert_eq!(body_after(body, 0), expected);
}
