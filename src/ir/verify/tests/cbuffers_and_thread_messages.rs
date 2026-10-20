//! plan-176-G: `ir::verify::tests` — cbuffer positions, package-path pilots and thread
//! messages, the unsendability cause walk, and package-path general checks. The shared builders
//! and fixtures live in `verify/tests/mod.rs`.

use super::*;

// ---------------------------------------------------------------------------
// LINK rule parity (bug-325)
// ---------------------------------------------------------------------------

/// the former source checker's `check_link_function_in` and `verify::check_link_functions` are
/// two independently-maintained ~320-line bodies validating the same LINK ABI
/// facts. They are kept in sync by hand, and adding a `NATIVE_*` rule to one
/// side and forgetting the other is silent — this test makes it loud.
///
/// The invariant, with `ir::verify` as the sole rejecter for every relocated
/// rule (plan-20):
///
// --- plan-58-A: CBuffer position rules on the PACKAGE path -----------------
//
// One twin per `check_buffer_slots` rule. These are the reason the checker is a
// shared function in `ir::link` rather than two hand-mirrored implementations:
// a crafted `.mfp` must get exactly the source-path treatment, and the two
// `is_c_abi_type` copies are what happens when it is written twice.
//
// The source-path halves live in `tests/syntax/native/native-cbuffer-*`.

/// A well-formed `OUT CBuffer` wrapper: every rule below mutates one thing about
/// it, so a test that stops failing means that rule stopped firing — not that the
/// fixture drifted into some other rejection.
fn cbuffer_fn() -> crate::ir::IrLinkFunction {
    let mut lf = link_fn();
    lf.params = vec![(
        "n".to_string(),
        crate::types::ParameterType::parse("Integer"),
    )];
    lf.return_type = crate::types::ParameterType::parse(crate::ir::BYTE_LIST_TYPE);
    lf.abi_slots = vec![
        crate::ir::IrAbiSlot {
            name: "buf".to_string(),
            ctype: crate::types::ParameterType::parse("CBuffer"),
            direction: crate::ir::AbiDirection::Out,
        },
        crate::ir::IrAbiSlot {
            name: "n".to_string(),
            ctype: crate::types::ParameterType::parse("CInt64"),
            direction: crate::ir::AbiDirection::In,
        },
    ];
    lf.abi_return_name = "status".to_string();
    lf.abi_return_ctype = crate::types::ParameterType::parse("CInt32");
    lf.result = Some(crate::ir::IrLinkExpr::Var("buf".to_string()));
    lf.buffers = vec![crate::ir::IrBuffer {
        slot: "buf".to_string(),
        size: crate::ir::IrLinkExpr::Var("n".to_string()),
    }];
    // Mandatory since plan-58-B rule 10: `status` is what the callee reports it
    // wrote. Unlike SIZE, a LENGTH expression MAY read the ABI return, because it
    // is evaluated after the call.
    lf.result_length = Some(crate::ir::IrLinkExpr::Var("status".to_string()));
    lf
}

fn cbuffer_project(lf: crate::ir::IrLinkFunction) -> crate::ir::IrProject {
    let mut p = project(vec![func_returns("run", "Nothing", vec![], vec![])], vec![]);
    p.link_functions = vec![lf];
    p
}

/// The baseline must be ACCEPTED, or every rejection test below is vacuous — it
/// would pass on a fixture that was already invalid for some unrelated reason.
#[test]
fn accepts_well_formed_cbuffer_link_function() {
    accept(&cbuffer_project(cbuffer_fn()));
}

/// Rule 1: OUT-only. There is no `List OF Byte` input marshal, so no send
/// direction exists to give `IN`/`INOUT CBuffer` a meaning.
#[test]
fn rejects_cbuffer_in_slot() {
    let mut lf = cbuffer_fn();
    lf.abi_slots[0].direction = crate::ir::AbiDirection::In;
    expect_rule(&cbuffer_project(lf), "NATIVE_BUFFER_INVALID");
}

#[test]
fn rejects_cbuffer_inout_slot() {
    let mut lf = cbuffer_fn();
    lf.abi_slots[0].direction = crate::ir::AbiDirection::InOut;
    expect_rule(&cbuffer_project(lf), "NATIVE_BUFFER_INVALID");
}

/// Rule 2: exactly one BUFFER clause. Zero leaves the capacity undefined — and a
/// decoded `.mfp` carries NO buffers today (plan-58-C owns the wire format), so
/// this is the rule that actually fires on a packaged CBuffer binding.
#[test]
fn rejects_cbuffer_without_buffer_clause() {
    let mut lf = cbuffer_fn();
    lf.buffers.clear();
    expect_rule(&cbuffer_project(lf), "NATIVE_BUFFER_INVALID");
}

#[test]
fn rejects_cbuffer_with_two_buffer_clauses() {
    let mut lf = cbuffer_fn();
    lf.buffers.push(crate::ir::IrBuffer {
        slot: "buf".to_string(),
        size: crate::ir::IrLinkExpr::Int(4096),
    });
    expect_rule(&cbuffer_project(lf), "NATIVE_BUFFER_INVALID");
}

/// Rule 3: a BUFFER clause must name a CBuffer slot of this function.
#[test]
fn rejects_buffer_clause_naming_unknown_slot() {
    let mut lf = cbuffer_fn();
    lf.buffers[0].slot = "nosuch".to_string();
    expect_rule(&cbuffer_project(lf), "NATIVE_BUFFER_INVALID");
}

#[test]
fn rejects_buffer_clause_naming_non_cbuffer_slot() {
    let mut lf = cbuffer_fn();
    lf.buffers[0].slot = "n".to_string();
    expect_rule(&cbuffer_project(lf), "NATIVE_BUFFER_INVALID");
}

/// Rule 4: a CONST pin on a CBuffer. It is an OUT slot, so this is the existing
/// `NATIVE_CONST_OUT` — no code is minted for a condition an existing rule names.
#[test]
fn rejects_cbuffer_const_pin() {
    let mut lf = cbuffer_fn();
    lf.consts = vec![("buf".to_string(), 0)];
    expect_rule(&cbuffer_project(lf), "NATIVE_CONST_OUT");
}

/// Rule 5: CBuffer as the ABI return proper. `abi_ctype_valid_as_return` cannot
/// express this — an OUT slot is checked through that same predicate — so it is a
/// position rule in `check_buffer_slots`, reusing the existing ctype rule.
#[test]
fn rejects_cbuffer_as_abi_return() {
    let mut lf = cbuffer_fn();
    lf.abi_return_ctype = crate::types::ParameterType::parse("CBuffer");
    expect_rule(&cbuffer_project(lf), "NATIVE_ABI_UNKNOWN_CTYPE");
}

/// Rule 6: a CBuffer `RETURN` does not name is unreachable, and unlike a scalar
/// OUT it costs a runtime-sized allocation nothing can observe.
#[test]
fn rejects_cbuffer_not_named_by_return() {
    let mut lf = cbuffer_fn();
    lf.return_type = crate::types::ParameterType::parse("Integer");
    lf.result = Some(crate::ir::IrLinkExpr::Var("status".to_string()));
    expect_rule(&cbuffer_project(lf), "NATIVE_BUFFER_INVALID");
}

/// Rule 7: RETURN names a CBuffer, so the wrapper must surface it as bytes.
#[test]
fn rejects_cbuffer_return_with_wrong_wrapper_type() {
    let mut lf = cbuffer_fn();
    lf.return_type = crate::types::ParameterType::parse("String");
    expect_rule(&cbuffer_project(lf), "NATIVE_BUFFER_INVALID");
}

/// Rule 8 — the pre-existing garbage-codegen hole (plan-58-A §2.3). Before
/// plan-58 this function COMPILED: `emit_return_passthrough` has no List-building
/// arm, so the caller dereferenced a raw scalar as a collection block with no
/// diagnostic. This is the package-path half, which never had even the CSTRUCT
/// return-type check the source path had.
#[test]
fn rejects_byte_list_return_without_cbuffer_slot() {
    let mut lf = link_fn();
    lf.return_type = crate::types::ParameterType::parse(crate::ir::BYTE_LIST_TYPE);
    expect_rule(&cbuffer_project(lf), "NATIVE_BUFFER_INVALID");
}

/// Rule 9: a SIZE expression naming nothing real. This is the capacity of a
/// buffer a C function is about to write into, so an unresolved name here is not
/// cosmetic.
#[test]
fn rejects_buffer_size_naming_unknown_slot() {
    let mut lf = cbuffer_fn();
    lf.buffers[0].size = crate::ir::IrLinkExpr::Var("nosuch".to_string());
    expect_rule(&cbuffer_project(lf), "NATIVE_ABI_UNBOUND_SLOT");
}

/// A SIZE expression may NOT read the ABI return — a causality error, not an
/// unbound name. `SUCCESS_ON`/`RETURN` are evaluated after the call and may read
/// it; a `BUFFER SIZE` is evaluated during staging, so at that point the status
/// word is uninitialized frame memory. Sizing an allocation from it would be a
/// silent wrong answer of the worst kind.
///
/// plan-58-A shipped rule 9 accepting this. Caught while implementing plan-58-B's
/// staging pass, where there is demonstrably nothing to load.
#[test]
fn rejects_buffer_size_reading_the_abi_return() {
    let mut lf = cbuffer_fn();
    lf.buffers[0].size = crate::ir::IrLinkExpr::Var("status".to_string());
    expect_rule(&cbuffer_project(lf), "NATIVE_ABI_UNBOUND_SLOT");
}

/// Same for an OUT slot: its value is whatever the callee writes, which has not
/// happened yet.
#[test]
fn rejects_buffer_size_reading_an_out_slot() {
    let mut lf = cbuffer_fn();
    lf.abi_slots.push(crate::ir::IrAbiSlot {
        name: "written".to_string(),
        ctype: crate::types::ParameterType::parse("CInt64"),
        direction: crate::ir::AbiDirection::Out,
    });
    lf.buffers[0].size = crate::ir::IrLinkExpr::Var("written".to_string());
    expect_rule(&cbuffer_project(lf), "NATIVE_ABI_UNBOUND_SLOT");
}

/// A CONST pin IS readable: it is a compile-time immediate, so it exists during
/// staging.
#[test]
fn accepts_buffer_size_reading_a_const_pin() {
    let mut lf = cbuffer_fn();
    lf.abi_slots.push(crate::ir::IrAbiSlot {
        name: "cap".to_string(),
        ctype: crate::types::ParameterType::parse("CInt64"),
        direction: crate::ir::AbiDirection::In,
    });
    lf.consts = vec![("cap".to_string(), 4096)];
    lf.buffers[0].size = crate::ir::IrLinkExpr::Var("cap".to_string());
    accept(&cbuffer_project(lf));
}

/// Rule 10: a returned CBuffer must carry a LENGTH clause.
///
/// Without one the list's `count` is its full capacity, so a callee that writes
/// fewer bytes than the buffer holds leaves the remainder as uninitialized arena
/// memory that ordinary code reads as data. Observed during plan-58-B Phase 2
/// before this rule existed: a short `pread` surfaced stale bytes as the result.
#[test]
fn rejects_returned_cbuffer_without_length() {
    let mut lf = cbuffer_fn();
    lf.result_length = None;
    expect_rule(&cbuffer_project(lf), "NATIVE_BUFFER_INVALID");
}

/// Rule 11: a LENGTH clause with no CBuffer to apply it to. Every other result is
/// a scalar, which has no length to set, so this is a mistake rather than a no-op.
#[test]
fn rejects_length_without_a_cbuffer_result() {
    let mut lf = link_fn();
    lf.result_length = Some(crate::ir::IrLinkExpr::Var("value".to_string()));
    expect_rule(&cbuffer_project(lf), "NATIVE_BUFFER_INVALID");
}

/// Rule 12: a LENGTH expression naming nothing real.
#[test]
fn rejects_length_naming_unknown_slot() {
    let mut lf = cbuffer_fn();
    lf.result_length = Some(crate::ir::IrLinkExpr::Var("nosuch".to_string()));
    expect_rule(&cbuffer_project(lf), "NATIVE_ABI_UNBOUND_SLOT");
}

/// The accept set for LENGTH is the WIDE one — it is evaluated after the call, so
/// an OUT slot holds a real value by then. This is the deliberate asymmetry with
/// rule 9's SIZE, and asserting it stops the two from being "unified" later.
#[test]
fn accepts_length_reading_an_out_slot() {
    let mut lf = cbuffer_fn();
    lf.abi_slots.push(crate::ir::IrAbiSlot {
        name: "written".to_string(),
        ctype: crate::types::ParameterType::parse("CInt64"),
        direction: crate::ir::AbiDirection::Out,
    });
    lf.result_length = Some(crate::ir::IrLinkExpr::Var("written".to_string()));
    accept(&cbuffer_project(lf));
}

/// A LENGTH may scale a callee's ELEMENT count to bytes — the plan-58-B Phase 2
/// arithmetic, on the post-call side.
#[test]
fn accepts_length_scaling_the_abi_return() {
    let mut lf = cbuffer_fn();
    lf.result_length = Some(crate::ir::IrLinkExpr::Mul(
        Box::new(crate::ir::IrLinkExpr::Var("status".to_string())),
        Box::new(crate::ir::IrLinkExpr::Int(2)),
    ));
    accept(&cbuffer_project(lf));
}

/// plan-106-B: `is_thread_type` keeps a NAME arm beside its structural one, and
/// this pins it.
///
/// `ParameterType::parse` builds a `ThreadHandle` only from the complete
/// `Thread OF Msg [RES R] TO Out` shape. Decoded package IR is
/// attacker-controlled (PKG-02) and need not be well formed, so a truncated
/// `Thread OF Integer` arrives as an opaque `Named` — and must still be treated
/// as a thread handle by the ownership and `.result` rules, exactly as the
/// pre-plan-106 `starts_with("Thread")` did.
#[test]
fn truncated_thread_spelling_still_counts_as_a_thread() {
    // Well formed → the structural arm.
    let handle = ParameterType::parse("Thread OF Integer TO Integer");
    assert!(matches!(handle, ParameterType::ThreadHandle { .. }));
    assert!(super::is_thread_type(&handle));

    // Truncated / crafted → an opaque `Named`, caught by the name arm.
    let truncated = ParameterType::parse("Thread OF Integer");
    assert!(matches!(truncated, ParameterType::Named(_)));
    assert!(super::is_thread_type(&truncated));

    let worker = ParameterType::parse("ThreadWorker OF Integer");
    assert!(super::is_thread_type(&worker));

    // An ordinary nominal is not a thread, however it is spelled.
    assert!(!super::is_thread_type(&ParameterType::named("Threadbare")));
    assert!(!super::is_thread_type(&ParameterType::Integer));
    assert!(!super::is_thread_type(&ParameterType::list_of(
        ParameterType::Integer
    )));
}

// ---------------------------------------------------------------------------
// plan-107-A pilots — the package-path twins of the relocated rules. A crafted
// `.mfp` gets exactly the source-path treatment; these prove the rule fires on
// decoded IR, where no the former source checker ever ran.
// ---------------------------------------------------------------------------

/// plan-115-A: `ISOLATED` is orthogonal to visibility, so a `PRIVATE ISOLATED
/// FUNC` is well-formed. This is the converted twin of the former
/// `rejects_private_isolated_func`, which pinned the bug-227 restriction that
/// plan-115-A deliberately lifts (an entry no longer has to be reachable from
/// another package, so "project-visible" has no surviving rationale). The
/// declaration-form half of the rule is still pinned by `rejects_isolated_sub`
/// below — that pairing is the point: only the visibility half was lifted.
#[test]
fn accepts_private_isolated_func() {
    let mut f = func("w", vec![], vec![ret(int_const("0"))]);
    f.isolated = true;
    f.visibility = "private".to_string();
    expect_no_rule(&project(vec![f], vec![]), "TYPE_ISOLATED_NOT_VISIBLE");
}

#[test]
fn rejects_isolated_on_a_non_func_kind() {
    // `ISOLATED` marks a top-level FUNC only; a body of any other kind (here the
    // optimizer's synthesized `function`) carrying it is rejected.
    let mut s = func_returns("w", "Nothing", vec![], vec![ret_none()]);
    s.kind = "function".to_string();
    s.isolated = true;
    expect_rule(&project(vec![s], vec![]), "TYPE_ISOLATED_NOT_VISIBLE");
}

#[test]
fn accepts_public_isolated_func() {
    let mut f = func("w", vec![], vec![ret(int_const("0"))]);
    f.isolated = true;
    f.visibility = "public".to_string();
    accept(&project(vec![f], vec![]));
}

#[test]
fn rejects_inline_trap_on_a_non_call() {
    let body = inline_trap_ops(int_const("5"), vec![recover_zero()]);
    expect_rule(
        &project(vec![func("run", vec![], body)], vec![]),
        "TYPE_INLINE_TRAP_REQUIRES_FALLIBLE",
    );
}

#[test]
fn rejects_inline_trap_on_a_package_constant() {
    let scrutinee = IrValue::CallResult {
        target: "math.pi".to_string(),
        args: vec![],
        type_: ParameterType::Float,
        loc: IrSourceLoc::default(),
    };
    let body = inline_trap_ops(scrutinee, vec![recover_zero()]);
    expect_rule(
        &project(vec![func("run", vec![], body)], vec![]),
        "TYPE_INLINE_TRAP_REQUIRES_FALLIBLE",
    );
}

#[test]
fn accepts_inline_trap_on_a_call() {
    let scrutinee = IrValue::CallResult {
        target: "getName".to_string(),
        args: vec![],
        type_: ParameterType::Integer,
        loc: IrSourceLoc::default(),
    };
    let body = inline_trap_ops(scrutinee, vec![recover_zero()]);
    let callee = func("getName", vec![], vec![ret(int_const("1"))]);
    let got = rules(&project(vec![func("run", vec![], body), callee], vec![]));
    assert!(
        !got.iter()
            .any(|r| r == "TYPE_INLINE_TRAP_REQUIRES_FALLIBLE"),
        "{got:?}"
    );
}

#[test]
fn skips_the_testing_desugared_trap_guard() {
    // `expectTrap(5)` desugars into the inline-trap shape whose handler sets a
    // `$expect_trapped` temp; that form has its own TESTING rule in the
    // front end and must not be reported as an inline TRAP.
    let handler = vec![IrOp::Assign {
        name: "$expect_trapped0".to_string(),
        value: const_of("Boolean", "true"),
        loc: IrSourceLoc::default(),
    }];
    let mut body = inline_trap_ops(int_const("5"), handler);
    body.insert(
        0,
        bind(
            "$expect_trapped0",
            "Boolean",
            Some(const_of("Boolean", "false")),
            false,
            true,
        ),
    );
    let got = rules(&project(vec![func("run", vec![], body)], vec![]));
    assert!(
        !got.iter()
            .any(|r| r == "TYPE_INLINE_TRAP_REQUIRES_FALLIBLE"),
        "{got:?}"
    );
}

#[test]
fn rejects_unsendable_thread_message_in_a_parameter() {
    let f = func(
        "run",
        vec![param("t", "Thread OF BadMessage TO Integer", None)],
        vec![ret(int_const("0"))],
    );
    expect_rule(
        &project(vec![f], vec![unsendable_record()]),
        "TYPE_THREAD_NOT_SENDABLE",
    );
}

#[test]
fn rejects_unsendable_thread_message_in_a_record_field() {
    let holder = record_typed("Holder", &[("t", "Thread OF BadMessage TO Integer")]);
    let f = func("run", vec![], vec![ret(int_const("0"))]);
    expect_rule(
        &project(vec![f], vec![unsendable_record(), holder]),
        "TYPE_THREAD_NOT_SENDABLE",
    );
}

#[test]
fn rejects_unsendable_message_sent_across_a_thread() {
    let send = IrOp::Eval {
        value: IrValue::Call {
            target: "thread.send".to_string(),
            args: vec![
                IrValue::Local("t".to_string()),
                IrValue::Local("m".to_string()),
            ],
            type_: ParameterType::Nothing,
            loc: IrSourceLoc::default(),
        },
        loc: IrSourceLoc::default(),
    };
    let f = func_returns(
        "run",
        "Nothing",
        vec![
            param("t", "Thread OF BadMessage TO Integer", None),
            param("m", "BadMessage", None),
        ],
        vec![send, ret_none()],
    );
    // Both the declared handle type and the call boundary reject.
    let got = rules(&project(vec![f], vec![unsendable_record()]));
    assert!(
        got.iter()
            .filter(|r| *r == "TYPE_THREAD_NOT_SENDABLE")
            .count()
            >= 2,
        "{got:?}"
    );
}

#[test]
fn rejects_transfer_on_a_thread_without_a_resource_plane() {
    let transfer = IrOp::Eval {
        value: IrValue::Call {
            target: crate::codegen::builtins::thread::TRANSFER_RESOURCE.to_string(),
            args: vec![
                IrValue::Local("t".to_string()),
                IrValue::Local("v".to_string()),
            ],
            type_: ParameterType::Nothing,
            loc: IrSourceLoc::default(),
        },
        loc: IrSourceLoc::default(),
    };
    let f = func_returns(
        "run",
        "Nothing",
        vec![
            param("t", "Thread OF String TO Integer", None),
            param("v", "Integer", None),
        ],
        vec![transfer, ret_none()],
    );
    expect_rule(&project(vec![f], vec![]), "TYPE_THREAD_NOT_SENDABLE");
}

/// bug-301 G4: the resource plane's `STATE T` payload crosses the boundary with
/// the resource (deep-copied into the receiver's arena), so it must be sendable
/// too — copyable + defaultable does not imply it: a record holding
/// `List OF RES fs.File` satisfies both yet carries sender-owned pointers.
#[test]
fn rejects_unsendable_resource_plane_state_payload() {
    // plan-114-A: the payload is deep-copied DATA riding the resource plane, so
    // the resource inside it is on a data plane and the rejection names the
    // plane remedy (`2-203-0138`) rather than the generic unsendable rule.
    let holder = record_typed("Holder", &[("files", "List OF RES fs.File")]);
    let f = func(
        "worker",
        vec![param(
            "t",
            "ThreadWorker OF Integer RES fs.File STATE Holder TO Integer",
            None,
        )],
        vec![ret(int_const("0"))],
    );
    expect_rule(
        &project(vec![f], vec![holder]),
        "TYPE_THREAD_RESOURCE_PLANE_REQUIRED",
    );

    // A STATE of plain sendable fields is accepted — the rule rejects the
    // unsendable payload, not stateful planes generally.
    let plain = record_typed("Holder", &[("count", "Integer"), ("label", "String")]);
    let f = func(
        "worker",
        vec![param(
            "t",
            "ThreadWorker OF Integer RES fs.File STATE Holder TO Integer",
            None,
        )],
        vec![ret(int_const("0"))],
    );
    let got = rules(&project(vec![f], vec![plain]));
    assert!(
        !got.iter()
            .any(|r| r == "TYPE_THREAD_NOT_SENDABLE" || r == "TYPE_THREAD_RESOURCE_PLANE_REQUIRED"),
        "{got:?}"
    );
}

#[test]
fn rejects_a_non_resource_on_the_resource_plane() {
    // `thread::accept` on a thread whose resource plane names `Integer`: the
    // resource plane moves only resources.
    let accept = IrOp::Eval {
        value: IrValue::Call {
            target: crate::codegen::builtins::thread::ACCEPT_RESOURCE.to_string(),
            args: vec![IrValue::Local("t".to_string())],
            type_: ParameterType::Integer,
            loc: IrSourceLoc::default(),
        },
        loc: IrSourceLoc::default(),
    };
    let f = func(
        "worker",
        vec![param(
            "t",
            "ThreadWorker OF String RES Integer TO Integer",
            None,
        )],
        vec![accept, ret(int_const("0"))],
    );
    expect_rule(&project(vec![f], vec![]), "TYPE_THREAD_NOT_SENDABLE");
}

#[test]
fn rejects_a_resource_in_the_message_plane() {
    // The data plane is resource-free (§7): a resource rides the `RES` plane.
    // plan-114-A gives that its own rule, since the remedy is nameable.
    let f = func(
        "run",
        vec![param("t", "Thread OF fs.File TO Integer", None)],
        vec![ret(int_const("0"))],
    );
    let got = rules(&project(vec![f], vec![]));
    assert!(
        got.iter()
            .any(|r| r == "TYPE_THREAD_RESOURCE_PLANE_REQUIRED"),
        "{got:?}"
    );
    // One mistake, one diagnostic: the plane rule and the sendability rule are
    // mutually exclusive, so the generic rule must NOT also fire here.
    assert!(
        !got.iter().any(|r| r == "TYPE_THREAD_NOT_SENDABLE"),
        "both rules fired for one cause: {got:?}"
    );
}

#[test]
fn accepts_a_sendable_thread_message() {
    let f = func(
        "run",
        vec![param("t", "Thread OF String TO Integer", None)],
        vec![ret(int_const("0"))],
    );
    let got = rules(&project(vec![f], vec![]));
    assert!(
        !got.iter().any(|r| r == "TYPE_THREAD_NOT_SENDABLE"),
        "{got:?}"
    );
}

// ---------------------------------------------------------------------------
// plan-114-A — the thread-unsendability CAUSE walk.
//
// `thread_unsendable_cause` is the single walk; "is it sendable" is `.is_none()`
// on it. These assert the walk's classification directly, so a future edit that
// makes it disagree with the rule it feeds is caught here rather than as a
// silently-missing diagnostic downstream.
// ---------------------------------------------------------------------------

/// Build the checker's type environment over a project fixture, so the cause
/// walk can be asked about a type directly.
fn type_env(project: &IrProject) -> super::TypeEnv {
    super::TypeEnv::build(project)
}

/// The cause the walk reports for `spelling`, given `types` are declared.
fn cause_for(types: Vec<IrType>, spelling: &str) -> Option<super::resources::Unsendable> {
    let f = func("run", vec![], vec![ret(int_const("0"))]);
    let project = project(vec![f], types);
    let env = type_env(&project);
    env.thread_unsendable_cause(
        &ParameterType::parse(spelling),
        &mut std::collections::HashSet::new(),
    )
}

#[test]
fn cause_walk_reports_a_res_marked_element_as_a_resource() {
    // `RES fs.File` in a collection: the §15.6 case. The reported leaf is the
    // `RES`-marked element itself, not the enclosing `List`.
    let cause = cause_for(vec![], "List OF RES fs.File");
    assert_eq!(
        cause,
        Some(super::resources::Unsendable::Resource(
            ParameterType::parse("RES fs.File")
        )),
        "expected the RES element as the blocking leaf"
    );
}

#[test]
fn cause_walk_reports_a_res_map_value_through_both_positions() {
    // The Map arm reports key first, then value; only the value blocks here.
    let cause = cause_for(vec![], "Map OF String TO RES fs.File");
    assert_eq!(
        cause,
        Some(super::resources::Unsendable::Resource(
            ParameterType::parse("RES fs.File")
        ))
    );
}

#[test]
fn cause_walk_accepts_a_sendable_bare_resource_but_rejects_an_unsendable_one() {
    // A bare resource nominal is judged by its registered sendability, NOT by
    // being a resource: `fs.File` is `sendable: true`
    // (src/codegen/builtins/fs/mod.rs), `process.Process` is `sendable: false`
    // (src/codegen/builtins/process/mod.rs). This is why the bare-resource
    // DATA-plane rejection cannot come from this walk — see the plane rule in
    // `check_thread_sendability`.
    //
    // Both spellings are package-qualified because that is the only form the
    // resource tables answer to: `registry().resolve_type` splits on the `.`
    // (`Registry::resolve_type`, src/codegen/registry/mod.rs), so a bare `Process` resolves to
    // nothing and reads as an unknown name — which is vacuously sendable.
    assert_eq!(cause_for(vec![], "fs.File"), None, "fs.File is sendable");
    assert_eq!(
        cause_for(vec![], "process.Process"),
        Some(super::resources::Unsendable::Resource(
            ParameterType::parse("process.Process")
        )),
        "process.Process is not thread-sendable"
    );
    // The unqualified spelling is not a resource as far as the tables are
    // concerned. Pinned so the contrast above is not mistaken for a typo.
    assert_eq!(cause_for(vec![], "Process"), None);
}

#[test]
fn cause_walk_reports_func_and_thread_handle_as_other_not_resource() {
    // These are genuinely unsendable — no resource plane exists to move them
    // to — so they must NOT be classified as a plane mix-up.
    assert_eq!(
        cause_for(vec![], "FUNC(Integer) AS String"),
        Some(super::resources::Unsendable::Other(ParameterType::parse(
            "FUNC(Integer) AS String"
        )))
    );
    // plan-156-D: a worker's own view never crosses, and says so by its own cause.
    assert_eq!(
        cause_for(vec![], "ThreadWorker OF String TO Integer"),
        Some(super::resources::Unsendable::Worker(ParameterType::parse(
            "ThreadWorker OF String TO Integer"
        )))
    );
    // A parent `Thread` is a sendable resource: it crosses on a resource plane.
    assert_eq!(cause_for(vec![], "Thread OF String TO Integer"), None);
}

/// plan-156-E: each view of a thread carries its own STATE, and `thread::start` never
/// compares the parent's with the worker's — here they are different types, and the
/// program is clean.
#[test]
fn thread_start_does_not_compare_parent_and_worker_state() {
    let src = "IMPORT thread\n\
TYPE P\n  n AS Integer\nEND TYPE\n\
TYPE W\n  label AS String\nEND TYPE\n\
ISOLATED FUNC work(worker AS ThreadWorker OF Integer TO Integer STATE W, n AS Integer) AS Integer\n  worker.state.label = \"w\"\n  RETURN n\nEND FUNC\n\
FUNC main AS Integer\n  RES t AS Thread OF Integer TO Integer STATE P = thread::start(work, 5)\n  t.state.n = 1\n  RETURN thread::waitFor(t)\nEND FUNC\n";
    let got = crate::testutil::check_src(src);
    assert!(
        got.is_empty(),
        "the two views' STATE types need not agree: {got:?}"
    );
}

/// plan-156-D: a parent `Thread` is a sendable resource, so a thread may declare a
/// resource plane of them — no sendability rule fires.
#[test]
fn a_thread_may_carry_parent_threads_on_its_resource_plane() {
    let f = func(
        "worker",
        vec![param(
            "t",
            "ThreadWorker OF Integer RES (Thread OF Integer TO Integer) TO Integer",
            None,
        )],
        vec![ret(int_const("0"))],
    );
    let got = rules(&project(vec![f], vec![]));
    for rule in [
        "TYPE_THREAD_NOT_SENDABLE",
        "TYPE_THREAD_RESOURCE_PLANE_REQUIRED",
        "TYPE_THREAD_WORKER_NOT_TRANSFERABLE",
    ] {
        assert!(!got.iter().any(|r| r == rule), "{rule}: {got:?}");
    }
}

/// A `ThreadWorker` on a resource plane gets its own rule, not the generic one.
#[test]
fn a_thread_worker_on_a_resource_plane_is_not_transferable() {
    let f = func(
        "worker",
        vec![param(
            "t",
            "ThreadWorker OF Integer RES (ThreadWorker OF Integer TO Integer) TO Integer",
            None,
        )],
        vec![ret(int_const("0"))],
    );
    let got = rules(&project(vec![f], vec![]));
    assert!(
        got.iter()
            .any(|r| r == "TYPE_THREAD_WORKER_NOT_TRANSFERABLE"),
        "{got:?}"
    );
    assert!(
        !got.iter().any(|r| r == "TYPE_THREAD_NOT_SENDABLE"),
        "{got:?}"
    );
}

#[test]
fn cause_walk_descends_into_a_record_field_and_names_the_leaf() {
    // The nested case behind `rejects_unsendable_resource_plane_state_payload`:
    // the record itself is not a resource, so the cause must come from the
    // field, and the reported leaf must be the field's element — not `Holder`.
    let holder = record_typed("Holder", &[("files", "List OF RES fs.File")]);
    assert_eq!(
        cause_for(vec![holder], "Holder"),
        Some(super::resources::Unsendable::Resource(
            ParameterType::parse("RES fs.File")
        ))
    );
}

#[test]
fn cause_walk_reports_the_first_blocking_field_left_to_right() {
    // Determinism: two blocking fields, and the FIRST is reported. Without this
    // the emitted message would depend on field-map iteration order.
    let holder = record_typed(
        "Holder",
        &[
            ("ok", "Integer"),
            ("first", "List OF RES fs.File"),
            ("second", "FUNC(Integer) AS String"),
        ],
    );
    assert_eq!(
        cause_for(vec![holder], "Holder"),
        Some(super::resources::Unsendable::Resource(
            ParameterType::parse("RES fs.File")
        ))
    );
}

#[test]
fn cause_walk_accepts_every_plain_sendable_shape() {
    // The regression guard for the refactor: the walk must still say `None`
    // for everything that crossed a boundary before it existed.
    let holder = record_typed("Holder", &[("count", "Integer"), ("label", "String")]);
    for spelling in [
        "Integer",
        "String",
        "Boolean",
        "Byte",
        "Float",
        "Fixed",
        "Money",
        "Nothing",
        "List OF Integer",
        "Set OF String",
        "Map OF String TO Integer",
        "#Result OF Integer",
        "Error",
        "ErrorLoc",
        "Scalar",
        "AttributedString",
        "Holder",
    ] {
        assert_eq!(
            cause_for(vec![holder.clone()], spelling),
            None,
            "`{spelling}` must stay sendable"
        );
    }
}

#[test]
fn the_resource_plane_keeps_the_generic_unsendable_rule() {
    // plan-114-A C3: a resource plane naming a resource that is not registered
    // thread-sendable is NOT a plane mix-up — it is already on the right plane.
    // Emitting `2-203-0138` here would tell the author to move it to the plane
    // it is on, so the resource plane keeps `2-203-0063`.
    let f = func(
        "worker",
        vec![param(
            "t",
            "ThreadWorker OF Integer RES process.Process TO Integer",
            None,
        )],
        vec![ret(int_const("0"))],
    );
    let got = rules(&project(vec![f], vec![]));
    assert!(
        got.iter().any(|r| r == "TYPE_THREAD_NOT_SENDABLE"),
        "{got:?}"
    );
    assert!(
        !got.iter()
            .any(|r| r == "TYPE_THREAD_RESOURCE_PLANE_REQUIRED"),
        "the resource plane must not get the data-plane remedy: {got:?}"
    );
}

#[test]
fn func_and_thread_handle_planes_keep_the_generic_unsendable_rule() {
    // The other half of the split: a genuinely unsendable type has no resource
    // plane to be moved to, so it must keep `2-203-0063` and never get the
    // resource remedy.
    for spelling in [
        "Thread OF FUNC(Integer) AS String TO Integer",
        "Thread OF ThreadWorker OF Integer TO Integer TO Integer",
    ] {
        let f = func(
            "run",
            vec![param("t", spelling, None)],
            vec![ret(int_const("0"))],
        );
        let got = rules(&project(vec![f], vec![]));
        assert!(
            got.iter().any(|r| r == "TYPE_THREAD_NOT_SENDABLE"),
            "`{spelling}`: {got:?}"
        );
        assert!(
            !got.iter()
                .any(|r| r == "TYPE_THREAD_RESOURCE_PLANE_REQUIRED"),
            "`{spelling}` is not a plane mix-up: {got:?}"
        );
    }
}

#[test]
fn cause_walk_terminates_on_a_self_referential_record() {
    // The cycle guard: `seen` must still stop the walk now that the `all(..)`
    // fold became a `find_map`.
    let node = record_typed("Node", &[("next", "Node"), ("label", "String")]);
    assert_eq!(cause_for(vec![node], "Node"), None);
}

// ---------------------------------------------------------------------------
// plan-107-B — the general semantic cluster's package-path twins.
// ---------------------------------------------------------------------------

#[test]
fn warns_dead_handler_on_an_infallible_inline_builtin() {
    // `len(xs) TRAP(e) … END TRAP`: `len` cannot fail, so the handler is dead
    // code — an advisory warning (plan-26-A), not an error.
    let scrutinee = IrValue::CallResult {
        target: "len".to_string(),
        args: vec![IrValue::Local("xs".to_string())],
        type_: ParameterType::Integer,
        loc: IrSourceLoc::default(),
    };
    let mut body = inline_trap_ops(scrutinee, vec![recover_zero()]);
    body.insert(
        0,
        bind(
            "xs",
            "List OF Integer",
            Some(IrValue::ListLiteral {
                type_: ParameterType::list_of(ParameterType::Integer),
                values: vec![int_const("1")],
            }),
            true,
            false,
        ),
    );
    expect_rule(
        &project(vec![func("run", vec![], body)], vec![]),
        "TYPE_INLINE_TRAP_DEAD_HANDLER",
    );
}

#[test]
fn rejects_normal_flow_reaching_the_trap() {
    // The handler returns, but the body before the TRAP falls through into it.
    let body = vec![
        IrOp::Eval {
            value: int_const("1"),
            loc: IrSourceLoc::default(),
        },
        IrOp::Trap {
            name: "e".to_string(),
            body: vec![ret(int_const("0"))],
            loc: IrSourceLoc::default(),
        },
    ];
    let f = func("run", vec![], body);
    let got = rules(&project(vec![f], vec![]));
    assert!(got.iter().any(|r| r == "TYPE_TRAP_FALLTHROUGH"), "{got:?}");
}

#[test]
fn a_stray_recover_counts_as_diverging() {
    // `RECOVER 0` inside a function-level TRAP is itself an error, but the front
    // end's flow analysis treats it as diverging, so the handler is not ALSO
    // reported as falling through. The stray RECOVER lowers to a
    // `$recover_stray` bind, which the divergence predicate honours.
    let body = vec![
        ret(int_const("1")),
        IrOp::Trap {
            name: "e".to_string(),
            body: vec![bind(
                "$recover_stray0",
                "Unknown",
                Some(int_const("0")),
                false,
                false,
            )],
            loc: IrSourceLoc::default(),
        },
    ];
    let f = func("run", vec![], body);
    let got = rules(&project(vec![f], vec![]));
    assert!(!got.iter().any(|r| r == "TYPE_TRAP_FALLTHROUGH"), "{got:?}");
}

/// plan-156-D: a parent `Thread` is a resource, so a collection holds it as a `RES`
/// element — a bare one is told to add the marker, exactly like `List OF fs::File`.
#[test]
fn rejects_a_thread_handle_as_a_list_element() {
    let f = func(
        "run",
        vec![param("xs", "List OF Thread OF Integer TO Integer", None)],
        vec![ret(int_const("0"))],
    );
    expect_rule(&project(vec![f], vec![]), "TYPE_RESOURCE_REQUIRES_RES");
}

#[test]
fn rejects_a_thread_handle_as_a_map_value() {
    let f = func(
        "run",
        vec![param(
            "m",
            "Map OF String TO Thread OF Integer TO Integer",
            None,
        )],
        vec![ret(int_const("0"))],
    );
    expect_rule(&project(vec![f], vec![]), "TYPE_RESOURCE_REQUIRES_RES");
}

/// The accept path: a `RES` thread element, map value and record field are legal
/// (plan-156-D) — none of the collection or marker rules fires.
#[test]
fn accepts_res_thread_elements_values_and_fields() {
    let mut ty = record_typed("Holder", &[("t", "RES Thread OF Integer TO Integer")]);
    ty.file = "src/main.mfb".to_string();
    let f = func(
        "run",
        vec![
            param("xs", "List OF RES Thread OF Integer TO Integer", None),
            param(
                "m",
                "Map OF String TO RES Thread OF Integer TO Integer",
                None,
            ),
        ],
        vec![ret(int_const("0"))],
    );
    let got = rules(&project(vec![f], vec![ty]));
    for rule in [
        "TYPE_COLLECTION_OWNERSHIP_VIOLATION",
        "TYPE_RESOURCE_REQUIRES_RES",
        "TYPE_RES_REQUIRES_RESOURCE",
    ] {
        assert!(
            !got.iter().any(|r| r == rule),
            "a RES thread slot is legal; {rule} must not fire: {got:?}"
        );
    }
}

/// bug-692: a bare `Thread` record field needs the `RES` marker, like any resource.
#[test]
fn rejects_a_bare_thread_record_field() {
    let mut ty = record_typed("Holder", &[("t", "Thread OF Integer TO Integer")]);
    ty.file = "src/main.mfb".to_string();
    let f = func_returns("run", "Nothing", vec![], vec![]);
    expect_rule(&project(vec![f], vec![ty]), "TYPE_RESOURCE_REQUIRES_RES");
}

/// A worker's own view never lives in a collection: it is not a resource the program
/// owns, and the collection rule still refuses it.
#[test]
fn rejects_a_thread_worker_as_a_list_element() {
    let f = func(
        "run",
        vec![param(
            "xs",
            "List OF ThreadWorker OF Integer TO Integer",
            None,
        )],
        vec![ret(int_const("0"))],
    );
    expect_rule(
        &project(vec![f], vec![]),
        "TYPE_COLLECTION_OWNERSHIP_VIOLATION",
    );
}

#[test]
fn rejects_a_resource_in_a_set_literal() {
    let literal = IrValue::SetLiteral {
        type_: ParameterType::set_of(ParameterType::named("fs.File")),
        values: vec![],
    };
    let body = vec![
        bind("s", "Set OF fs.File", Some(literal), true, false),
        ret(int_const("0")),
    ];
    let got = rules(&project(vec![func("run", vec![], body)], vec![]));
    // Once for the declared type, once for the literal — as the front end did.
    assert_eq!(
        got.iter()
            .filter(|r| *r == "TYPE_COLLECTION_OWNERSHIP_VIOLATION")
            .count(),
        2,
        "{got:?}"
    );
}
