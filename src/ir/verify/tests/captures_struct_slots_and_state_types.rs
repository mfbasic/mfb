//! plan-176-G: `ir::verify::tests` — package-path captures and LINK struct slots, the transfer
//! success edge, and state types. The shared builders and fixtures live in
//! `verify/tests/mod.rs`.

use super::*;

#[test]
fn rejects_a_thread_carrying_union_as_a_map_key() {
    // A union whose variant holds a thread handle: the front end's walk reached
    // the variant's fields; verify's predicate used to stop at the union name.
    let mut holder = record_typed("Holder", &[("t", "Thread OF Integer TO Integer")]);
    holder.visibility = "public".to_string();
    let plain = record_typed("Plain", &[("n", "Integer")]);
    let union = IrType {
        kind: "union".to_string(),
        visibility: "export".to_string(),
        name: "Either".to_string(),
        fields: vec![],
        includes: vec![],
        variants: vec![
            IrVariant {
                name: "Holder".to_string(),
                type_: ParameterType::parse("Holder"),
                fields: vec![IrField {
                    visibility: None,
                    name: "t".to_string(),
                    type_: ParameterType::parse("Thread OF Integer TO Integer"),
                    loc: IrSourceLoc::default(),
                }],
                loc: IrSourceLoc::default(),
            },
            IrVariant {
                name: "Plain".to_string(),
                type_: ParameterType::parse("Plain"),
                fields: vec![IrField {
                    visibility: None,
                    name: "n".to_string(),
                    type_: ParameterType::Integer,
                    loc: IrSourceLoc::default(),
                }],
                loc: IrSourceLoc::default(),
            },
        ],
        members: vec![],
        loc: IrSourceLoc::default(),
        file: String::new(),
    };
    let f = func(
        "run",
        vec![param("m", "Map OF Either TO Integer", None)],
        vec![ret(int_const("0"))],
    );
    expect_rule(
        &project(vec![f], vec![holder, plain, union]),
        "TYPE_COLLECTION_OWNERSHIP_VIOLATION",
    );
}

/// `LET f AS FUNC(Integer) AS Integer = LAMBDA(v AS Integer) -> v + <capture>`
/// as lowering shapes it: the closure value binds the lambda by name with its
/// capture list; `by_ref` captures arrive as `LocalRef`.
fn closure_bind(captures: Vec<IrValue>) -> IrOp {
    bind(
        "f",
        "FUNC(Integer) AS Integer",
        Some(IrValue::Closure {
            name: "$lambda0".to_string(),
            type_: ParameterType::parse("FUNC(Integer) AS Integer"),
            captures,
        }),
        true,
        false,
    )
}

#[test]
fn rejects_a_by_value_capture_of_a_mut_local() {
    let body = vec![
        bind("offset", "Integer", Some(int_const("1")), true, true),
        closure_bind(vec![IrValue::Local("offset".to_string())]),
        ret(int_const("0")),
    ];
    expect_rule(
        &project(vec![func("run", vec![], body)], vec![]),
        "TYPE_LAMBDA_CAPTURE_UNSUPPORTED",
    );
}

#[test]
fn accepts_a_by_ref_capture_of_a_mut_local() {
    // The compiler-proven non-escaping position (`forEach`'s action) captures a
    // MUT local by slot reference — lowering's `LocalRef`.
    let body = vec![
        bind("total", "Integer", Some(int_const("0")), true, true),
        closure_bind(vec![IrValue::LocalRef {
            name: "total".to_string(),
            type_: ParameterType::Integer,
        }]),
        ret(int_const("0")),
    ];
    let got = rules(&project(vec![func("run", vec![], body)], vec![]));
    assert!(
        !got.iter().any(|r| r == "TYPE_LAMBDA_CAPTURE_UNSUPPORTED"),
        "{got:?}"
    );
}

/// bug-843 (owner decision): a closure's capture of a resource works exactly like
/// `collections::append` of it into a `List` — a reference, never the resource — so
/// neither shape is `TYPE_LAMBDA_CAPTURE_UNSUPPORTED` any longer. This test pinned
/// the retired rejection; it now pins the acceptance. (The binding is a `LET`:
/// a by-value capture of a `MUT` is the separate mutable-capture rejection.)
#[test]
fn accepts_a_resource_capture_in_either_shape() {
    for capture in [
        IrValue::Local("handle".to_string()),
        IrValue::LocalRef {
            name: "handle".to_string(),
            type_: ParameterType::named("fs.File"),
        },
    ] {
        let body = vec![
            bind("handle", "fs.File", Some(int_const("0")), true, false),
            closure_bind(vec![capture]),
            ret(int_const("0")),
        ];
        let got = rules(&project(vec![func("run", vec![], body)], vec![]));
        assert!(
            !got.iter().any(|r| r == "TYPE_LAMBDA_CAPTURE_UNSUPPORTED"),
            "{got:?}"
        );
    }
}

/// The capture rule a `List` element has (bug-843: capture works exactly like
/// `collections::append`): a value that reaches a `ThreadWorker` may not be held
/// by a closure, because no collection may hold one
/// (`TYPE_COLLECTION_OWNERSHIP_VIOLATION`). bug-975 made the view a resource, so
/// a "neither a resource nor copyable" premise no longer singles it out; the
/// rule is the `List` rule's own `contains_thread_worker`.
#[test]
fn rejects_a_capture_of_a_worker_view() {
    let body = vec![closure_bind(vec![IrValue::Local("w".to_string())])];
    let f = func_returns("run", "Nothing", vec![param("w", WORKER_VIEW, None)], body);
    expect_rule(&project(vec![f], vec![]), "TYPE_LAMBDA_CAPTURE_UNSUPPORTED");
}

/// A record whose `RES` field holds the view reaches it, and a `List OF Holder`
/// is refused for the same reason, so the capture is too.
#[test]
fn rejects_a_capture_of_a_record_reaching_a_worker_view() {
    let body = vec![closure_bind(vec![IrValue::Local("h".to_string())])];
    let f = func_returns("run", "Nothing", vec![param("h", "Holder", None)], body);
    let holder = record_typed("Holder", &[("view", &format!("RES {WORKER_VIEW}"))]);
    expect_rule(
        &project(vec![f], vec![holder]),
        "TYPE_LAMBDA_CAPTURE_UNSUPPORTED",
    );
}

/// A parent `Thread` may be a `List OF RES Thread` element, so it may be captured.
#[test]
fn accepts_a_capture_of_a_parent_thread_view() {
    let body = vec![closure_bind(vec![IrValue::Local("t".to_string())])];
    let f = func_returns("run", "Nothing", vec![param("t", THREAD_VIEW, None)], body);
    expect_no_rule(&project(vec![f], vec![]), "TYPE_LAMBDA_CAPTURE_UNSUPPORTED");
}

#[test]
fn accepts_a_copyable_immutable_capture() {
    let body = vec![
        bind("offset", "Integer", Some(int_const("1")), true, false),
        closure_bind(vec![IrValue::Local("offset".to_string())]),
        ret(int_const("0")),
    ];
    let got = rules(&project(vec![func("run", vec![], body)], vec![]));
    assert!(
        !got.iter().any(|r| r == "TYPE_LAMBDA_CAPTURE_UNSUPPORTED"),
        "{got:?}"
    );
}

// ---------------------------------------------------------------------------
// plan-107-C — package-path twins of the LINK sub-forms verify lacked before
// the front end's rules moved here. Each mutates one thing about a well-formed
// wrapper over `CSTRUCT S AS Rec { a CInt32 }`.
// ---------------------------------------------------------------------------

/// A project with `CSTRUCT S AS Rec { a CInt32 }`, record `Rec { a AS Integer }`
/// and one wrapper whose ABI has a single struct slot `s` of `direction`.
fn struct_slot_project(
    direction: crate::ir::AbiDirection,
) -> (IrProject, crate::ir::IrLinkFunction) {
    let mut p = project_with_cstructs(vec![cstruct("S", &[("a", "CInt32")])]);
    p.types = vec![record_typed("Rec", &[("a", "Integer")])];
    let mut f = link_fn();
    f.params = vec![];
    f.abi_slots = vec![crate::ir::IrAbiSlot {
        name: "s".to_string(),
        ctype: crate::types::ParameterType::parse("S"),
        direction,
    }];
    (p, f)
}

#[test]
fn rejects_inout_on_a_non_cstruct_slot() {
    // A scalar slot is either a C argument or a produced value; INOUT means
    // nothing for it.
    let mut p = project(vec![func_returns("run", "Nothing", vec![], vec![])], vec![]);
    let mut f = link_fn();
    f.abi_slots[0].direction = crate::ir::AbiDirection::InOut;
    p.link_functions = vec![f];
    expect_rule(&p, "NATIVE_ABI_UNKNOWN_CTYPE");
}

#[test]
fn rejects_returning_an_in_struct_slot() {
    // An IN slot is zeroed and never read back, so `RETURN s` names nothing.
    let (mut p, mut f) = struct_slot_project(crate::ir::AbiDirection::In);
    f.bind_in = vec![bind_in("s", &[("a", None, Some(1))])];
    f.return_type = crate::types::ParameterType::parse("Rec");
    f.result = Some(crate::ir::IrLinkExpr::Var("s".to_string()));
    p.link_functions = vec![f];
    expect_rule(&p, "NATIVE_ABI_RESULT_MARKER");
}

#[test]
fn rejects_returning_a_struct_slot_as_another_type() {
    // A wrapper that returns a struct slot must declare its mapped record.
    let (mut p, mut f) = struct_slot_project(crate::ir::AbiDirection::Out);
    f.return_type = crate::types::ParameterType::parse("Integer");
    f.result = Some(crate::ir::IrLinkExpr::Var("s".to_string()));
    p.link_functions = vec![f];
    expect_rule(&p, "NATIVE_STRUCT_FIELD_MISMATCH");
}

#[test]
fn rejects_bind_in_on_an_out_slot() {
    let (mut p, mut f) = struct_slot_project(crate::ir::AbiDirection::Out);
    f.bind_in = vec![bind_in("s", &[("a", None, Some(1))])];
    f.result = Some(crate::ir::IrLinkExpr::Var("s".to_string()));
    f.return_type = crate::types::ParameterType::parse("Rec");
    p.link_functions = vec![f];
    expect_rule(&p, "NATIVE_BIND_IN_INVALID");
}

#[test]
fn rejects_bind_in_setting_a_field_twice() {
    let (mut p, mut f) = struct_slot_project(crate::ir::AbiDirection::In);
    f.bind_in = vec![bind_in("s", &[("a", None, Some(1)), ("a", None, Some(2))])];
    p.link_functions = vec![f];
    expect_rule(&p, "NATIVE_BIND_IN_INVALID");
}

#[test]
fn rejects_bind_in_field_bound_to_nothing() {
    // Lowering represents an unmarshalable value as neither param nor literal.
    let (mut p, mut f) = struct_slot_project(crate::ir::AbiDirection::In);
    f.bind_in = vec![bind_in("s", &[("a", None, None)])];
    p.link_functions = vec![f];
    expect_rule(&p, "NATIVE_BIND_IN_INVALID");
}

#[test]
fn rejects_an_unbound_in_struct_slot() {
    // An IN struct slot with neither a parameter nor a BIND IN block is unbound
    // — the front end's second slot pass never exempted struct slots.
    let (mut p, f) = struct_slot_project(crate::ir::AbiDirection::In);
    p.link_functions = vec![f];
    expect_rule(&p, "NATIVE_ABI_UNBOUND_SLOT");
}

#[test]
fn accepts_a_bound_in_struct_slot() {
    let (mut p, mut f) = struct_slot_project(crate::ir::AbiDirection::In);
    f.bind_in = vec![bind_in("s", &[("a", None, Some(1))])];
    p.link_functions = vec![f];
    let got = rules(&p);
    assert!(!got.iter().any(|r| r.starts_with("NATIVE_")), "{got:?}");
}

#[test]
fn accepts_a_body_that_returns_before_its_trap() {
    let body = vec![
        ret(int_const("1")),
        IrOp::Trap {
            name: "e".to_string(),
            body: vec![ret(int_const("0"))],
            loc: IrSourceLoc::default(),
        },
    ];
    let f = func("run", vec![], body);
    let got = rules(&project(vec![f], vec![]));
    assert!(!got.iter().any(|r| r == "TYPE_TRAP_FALLTHROUGH"), "{got:?}");
}

/// bug-483 sub-issue B: the compiler-owned records a program may neither
/// construct nor `WITH`-update are recognised by NAME, and bug-480 Phase 4b made
/// a builtin value type's declared identity package-qualified. Matching only the
/// bare leaf left the rule looking for a spelling nothing produces any more, so
/// `net::Address["1.2.3.4", 80]` compiled and ran — silently handing a program a
/// record whose layout only the runtime helpers are allowed to write.
///
/// The qualified identity must be refused: a source `net::Address[…]` resolves to
/// the qualified id.
///
/// plan-132 D1 corrected the other half of this test, which also required the BARE
/// leaves to be refused "because a record FIELD type still arrives bare". This rule
/// is asked only about a constructor's or a `WITH` target's type, never a field
/// type, and source cannot name an imported builtin record bare (`AS Address` under
/// `IMPORT net` is `SYMBOL_UNKNOWN_TYPE`, measured). So a bare leaf here is always a
/// project type of the same name — and requiring it refused made a project's own
/// `TYPE Address`, `TYPE AudioDevice` or `TYPE TermSize` unconstructible in every
/// program (`rt_shadowing_type_name_diagnostics`), while `TYPE Url` built.
#[test]
fn read_only_records_are_refused_under_their_qualified_identity() {
    for name in ["net.Address", "audio.AudioDevice", "term.TermSize"] {
        assert!(
            super::read_only_record_type(&ParameterType::declared(name)),
            "`{name}` must stay a read-only compiler-owned record (bug-483)"
        );
    }
    for name in ["Address", "AudioDevice", "TermSize"] {
        assert!(
            !super::read_only_record_type(&ParameterType::declared(name)),
            "a bare `{name}` is a project type and must stay constructible (plan-132 D1)"
        );
    }
    // An ordinary record is still constructible.
    assert!(!super::read_only_record_type(&ParameterType::declared(
        "net.Url"
    )));
    // plan-122-F: `term::TermColor` was a read-only compiler-owned record and is
    // retired; `term::getForeground`/`getBackground` return a `color::Color`, which
    // is an ORDINARY VALUE RECORD a program may build and `WITH`-update. Asserting
    // both spellings are constructible is what keeps the retirement real — a future
    // change that re-added `color.Color` to the read-only set would silently take
    // away a capability this letter deliberately granted.
    for name in ["Color", "color.Color"] {
        assert!(
            !super::read_only_record_type(&ParameterType::declared(name)),
            "`{name}` is an ordinary value record and must stay constructible"
        );
    }
}

#[test]
fn a_close_on_one_arm_does_not_move_the_resource_on_its_sibling() {
    // Each arm starts from the state before the IF; the ELSE never follows the THEN.
    let body = vec![
        bind("f", "fs.File", None, true, false),
        if_op(
            const_of("Boolean", "true"),
            vec![close_eval("f")],
            vec![read_of("f")],
        ),
    ];
    let p = project(vec![owner_fn("run", "Nothing", body, &["f"])], vec![]);
    expect_no_rule(&p, "TYPE_USE_AFTER_MOVE");
}

// --- bug-804: a transfer moves its resource on the success edge only ---------
//
// Built as IR directly, so these pin the rule on the decoded-package path too,
// where nothing but the verifier sees the shape.

const RES_PLANE: &str = "Thread OF String RES fs.File TO Integer";

/// A `thread::transfer(t, f)` as `target` lowers it: a propagating `Call`, or
/// the `CallResult` an inline `TRAP` binds to `$trap_res`.
fn transfer_value(target: &str, res: &str, checked: bool) -> IrValue {
    let target = target.to_string();
    let args = vec![
        IrValue::Local("t".to_string()),
        IrValue::Local(res.to_string()),
    ];
    let type_ = ParameterType::parse("Nothing");
    let loc = IrSourceLoc::default();
    if checked {
        IrValue::CallResult {
            target,
            args,
            type_,
            loc,
        }
    } else {
        IrValue::Call {
            target,
            args,
            type_,
            loc,
        }
    }
}

fn result_is_ok(result: &str) -> IrValue {
    IrValue::ResultIsOk {
        value: Box::new(IrValue::Local(result.to_string())),
    }
}

/// A function holding `t` (a resource-plane thread) and owning `f`.
fn transfer_fn(body: Vec<IrOp>) -> IrProject {
    let mut f = owner_fn("run", "Nothing", body, &["f", "g"]);
    f.params.push(param("t", RES_PLANE, None));
    project(vec![f], vec![])
}

#[test]
fn a_read_after_a_propagating_transfer_is_a_use_after_move() {
    for target in [
        crate::codegen::builtins::thread::TRANSFER_RESOURCE,
        crate::codegen::builtins::thread::EMIT_RESOURCE,
    ] {
        let p = transfer_fn(vec![
            bind("f", "fs.File", None, true, false),
            eval(transfer_value(target, "f", false)),
            read_of("f"),
        ]);
        expect_rule(&p, "TYPE_USE_AFTER_MOVE");
    }
}

#[test]
fn a_read_before_a_transfer_is_not_a_use_after_move() {
    let p = transfer_fn(vec![
        bind("f", "fs.File", None, true, false),
        read_of("f"),
        eval(transfer_value(
            crate::codegen::builtins::thread::TRANSFER_RESOURCE,
            "f",
            false,
        )),
    ]);
    expect_no_rule(&p, "TYPE_USE_AFTER_MOVE");
}

#[test]
fn a_transfer_moves_a_possible_alias() {
    // `RES g = same(f)` records g as a may-alias of f; transferring f moves g.
    let p = transfer_fn(vec![
        bind("f", "fs.File", None, true, false),
        bind(
            "g",
            "fs.File",
            Some(IrValue::Call {
                target: "same".to_string(),
                args: vec![IrValue::Local("f".to_string())],
                type_: ParameterType::parse("fs.File"),
                loc: IrSourceLoc::default(),
            }),
            true,
            false,
        ),
        eval(transfer_value(
            crate::codegen::builtins::thread::TRANSFER_RESOURCE,
            "f",
            false,
        )),
        read_of("g"),
    ]);
    expect_rule(&p, "TYPE_USE_AFTER_MOVE");
}

/// The one-check inline-TRAP shape: the failure arm keeps the resource.
fn inline_transfer(then_body: Vec<IrOp>, else_body: Vec<IrOp>, after: Vec<IrOp>) -> IrProject {
    let mut body = vec![
        bind("f", "fs.File", None, true, false),
        bind(
            "$trap_res0",
            "#Result OF Nothing",
            Some(transfer_value(
                crate::codegen::builtins::thread::TRANSFER_RESOURCE,
                "f",
                true,
            )),
            false,
            false,
        ),
        if_op(result_is_ok("$trap_res0"), then_body, else_body),
    ];
    body.extend(after);
    transfer_fn(body)
}

#[test]
fn an_inline_failure_arm_may_use_the_untransferred_resource() {
    let p = inline_transfer(vec![], vec![read_of("f"), close_eval("f")], vec![]);
    expect_no_rule(&p, "TYPE_USE_AFTER_MOVE");
}

#[test]
fn the_inline_success_arm_sees_the_resource_moved() {
    let p = inline_transfer(vec![read_of("f")], vec![], vec![]);
    expect_rule(&p, "TYPE_USE_AFTER_MOVE");
}

#[test]
fn a_read_after_an_inline_transfer_join_is_a_use_after_move() {
    let p = inline_transfer(vec![], vec![read_of("f")], vec![read_of("f")]);
    expect_rule(&p, "TYPE_USE_AFTER_MOVE");
}

#[test]
fn a_diverging_success_arm_does_not_poison_the_failure_continuation() {
    let p = inline_transfer(vec![ret_none()], vec![], vec![read_of("f")]);
    expect_no_rule(&p, "TYPE_USE_AFTER_MOVE");
}

/// The check-chain shape: a fallible argument check wraps the transfer check,
/// every failing `else` sets `$trap_failed1`, and the handler runs after the
/// chain behind that flag.
fn chain_transfer(handler: Vec<IrOp>, after: Vec<IrOp>) -> IrProject {
    let set_failed = || IrOp::Assign {
        name: "$trap_failed1".to_string(),
        value: const_of("Boolean", "true"),
        loc: IrSourceLoc::default(),
    };
    let mut body = vec![
        bind("f", "fs.File", None, true, false),
        bind(
            "$trap_failed1",
            "Boolean",
            Some(const_of("Boolean", "false")),
            false,
            true,
        ),
        bind(
            "$trap_res2",
            "#Result OF Integer",
            Some(IrValue::CallResult {
                target: "wait".to_string(),
                args: vec![],
                type_: ParameterType::parse("Integer"),
                loc: IrSourceLoc::default(),
            }),
            false,
            false,
        ),
        if_op(
            result_is_ok("$trap_res2"),
            vec![
                bind(
                    "$trap_res3",
                    "#Result OF Nothing",
                    Some(transfer_value(
                        crate::codegen::builtins::thread::TRANSFER_RESOURCE,
                        "f",
                        true,
                    )),
                    false,
                    false,
                ),
                if_op(result_is_ok("$trap_res3"), vec![], vec![set_failed()]),
            ],
            vec![set_failed()],
        ),
        if_op(IrValue::Local("$trap_failed1".to_string()), handler, vec![]),
    ];
    body.extend(after);
    transfer_fn(body)
}

#[test]
fn a_check_chain_handler_may_use_the_untransferred_resource() {
    let p = chain_transfer(vec![read_of("f")], vec![]);
    expect_no_rule(&p, "TYPE_USE_AFTER_MOVE");
}

#[test]
fn a_read_after_a_check_chain_join_is_a_use_after_move() {
    let p = chain_transfer(vec![read_of("f")], vec![read_of("f")]);
    expect_rule(&p, "TYPE_USE_AFTER_MOVE");
}

// --- bug-1013: a STATE type is a record TYPE --------------------------------

/// The rules a `fs.File` binding declared `type_` reports, over `types`.
fn state_binding_rules(type_: &str, types: Vec<IrType>) -> Vec<String> {
    let mut f = func_returns("run", "Nothing", vec![], vec![]);
    let b = res_bind_owned(&mut f, "h", type_, None);
    f.body = vec![b];
    rules(&project(vec![f], types))
}

/// bug-1013: a `STATE` type is a record `TYPE`. Every other kind is refused —
/// the scalars the back end could not hold, the `String` that crashed, and the
/// collections that happened to work.
#[test]
fn rejects_state_type_that_is_not_a_record() {
    for state in [
        "Integer",
        "Boolean",
        "Float",
        "Fixed",
        "Money",
        "Byte",
        "String",
        "List OF Integer",
        "Set OF Integer",
        "Map OF String TO Integer",
        "Mode",
        "Shape",
        "fs.File",
        "FUNC(Integer) AS Integer",
        "AttributedString",
    ] {
        let got = state_binding_rules(
            &format!("fs.File STATE {state}"),
            vec![
                enum_type("Mode", &["Fast", "Slow"]),
                union("Shape", &["Dot"]),
                record_typed("Dot", &[("x", "Integer")]),
            ],
        );
        assert_eq!(
            count_rule(&got, "TYPE_STATE_INVALID"),
            1,
            "STATE {state} must be refused once: {got:?}"
        );
    }
}

/// bug-1013's other half: a record stays a legal `STATE` type, whatever its
/// fields — including the very types refused bare — and so does a union's
/// member, which is a record `TYPE` in its own right.
#[test]
fn accepts_state_type_that_is_a_record() {
    let types = || {
        vec![
            record_typed(
                "Cursor",
                &[
                    ("pos", "Integer"),
                    ("label", "String"),
                    ("marks", "List OF Integer"),
                    ("seen", "Map OF String TO Integer"),
                ],
            ),
            record_typed("Dot", &[("x", "Integer")]),
            union("Shape", &["Dot"]),
        ]
    };
    // `Error` is the built-in record.
    for state in ["Cursor", "Dot", "Error"] {
        let got = state_binding_rules(&format!("fs.File STATE {state}"), types());
        assert_eq!(
            count_rule(&got, "TYPE_STATE_INVALID"),
            0,
            "{state}: {got:?}"
        );
    }
}

/// bug-1013: the rule is one walk over the declared type, so it holds at every
/// position a clause can be written — the binding was the only one checked for
/// a nested clause, and an element, a field, a thread plane and a thread view
/// all let `STATE Integer` through.
#[test]
fn rejects_non_record_state_at_every_position() {
    let invalid = |p: &IrProject, what: &str| {
        let got = rules(p);
        assert_eq!(
            count_rule(&got, "TYPE_STATE_INVALID"),
            1,
            "{what} must be refused once: {got:?}"
        );
    };
    // A parameter, and a parameter's collection element.
    for type_ in [
        "fs.File STATE Integer",
        "List OF RES fs.File STATE Integer",
        "Map OF String TO RES fs.File STATE Integer",
        "ThreadWorker OF Integer RES fs.File STATE Integer TO Integer",
        "ThreadWorker OF Integer TO Integer STATE Integer",
        "Thread OF Integer TO Integer STATE Integer",
    ] {
        let f = func_returns("run", "Nothing", vec![param("p", type_, None)], vec![]);
        invalid(&project(vec![f], vec![]), &format!("parameter {type_}"));
    }
    // A return.
    let f = func_returns("run", "fs.File STATE Integer", vec![], vec![]);
    invalid(&project(vec![f], vec![]), "a return");
    // A binding's collection element.
    let f = func_returns(
        "run",
        "Nothing",
        vec![],
        vec![bind(
            "xs",
            "List OF RES fs.File STATE Integer",
            None,
            true,
            true,
        )],
    );
    invalid(&project(vec![f], vec![]), "a binding's element");
    // A record field, and a field's collection element.
    for field in [
        "RES fs.File STATE Integer",
        "List OF RES fs.File STATE Integer",
    ] {
        let p = project(
            vec![func_returns("run", "Nothing", vec![], vec![])],
            vec![record_typed("Holder", &[("h", field)])],
        );
        invalid(&p, &format!("field {field}"));
    }
    // A top-level binding.
    let mut p = project(vec![func_returns("run", "Nothing", vec![], vec![])], vec![]);
    p.bindings = vec![binding("g", "fs.File STATE Integer", None, true, true)];
    invalid(&p, "a top-level binding");
    // A native declaration: its return and its parameter.
    let mut p = project(vec![func_returns("run", "Nothing", vec![], vec![])], vec![]);
    let mut open = link_fn();
    open.return_type = crate::types::ParameterType::parse("fs.File");
    open.return_resource = true;
    open.return_state_type = Some(crate::types::ParameterType::Integer);
    p.link_functions = vec![open];
    invalid(&p, "a native return");
    let mut p = project(vec![func_returns("run", "Nothing", vec![], vec![])], vec![]);
    let mut close = link_fn();
    close.params = vec![(
        "path".to_string(),
        crate::types::ParameterType::parse("fs.File STATE Integer"),
    )];
    p.link_functions = vec![close];
    invalid(&p, "a native parameter");
}

/// bug-998: a `STATE` clause on a type that is not a resource is its own rule,
/// reported once, at every position. The clause reports nothing else: not
/// `TYPE_STATE_INVALID` for a clause whose type is also not a record, and not
/// `TYPE_STATE_MISMATCH`.
#[test]
fn rejects_state_on_a_non_resource_at_every_position() {
    let types = || {
        vec![
            record_typed("Cur", &[("n", "Integer")]),
            enum_type("Mode", &["Fast", "Slow"]),
        ]
    };
    let only = |p: &IrProject, what: &str| {
        let got = rules(p);
        assert_eq!(
            count_rule(&got, "TYPE_STATE_REQUIRES_RESOURCE"),
            1,
            "{what} must be refused once: {got:?}"
        );
        for other in ["TYPE_STATE_INVALID", "TYPE_STATE_MISMATCH"] {
            assert_eq!(count_rule(&got, other), 0, "{what}: {got:?}");
        }
    };
    // A parameter: a scalar, a record, an enum; and a non-record STATE on one.
    for type_ in [
        "Integer STATE Cur",
        "String STATE Cur",
        "Cur STATE Cur",
        "Mode STATE Cur",
        "Integer STATE Mode",
        "Integer STATE Integer",
        "List OF RES Integer STATE Cur",
        "ThreadWorker OF Integer RES Integer STATE Cur TO Integer",
    ] {
        let f = func_returns("run", "Nothing", vec![param("p", type_, None)], vec![]);
        only(&project(vec![f], types()), &format!("parameter {type_}"));
    }
    // A return.
    let f = func_returns("run", "Integer STATE Cur", vec![], vec![]);
    let got = rules(&project(vec![f], types()));
    assert_eq!(
        count_rule(&got, "TYPE_STATE_REQUIRES_RESOURCE"),
        1,
        "a return: {got:?}"
    );
    // A `RES` binding.
    let mut f = func_returns("run", "Nothing", vec![], vec![]);
    let b = res_bind_owned(&mut f, "x", "Integer STATE Cur", Some(int_const("1")));
    f.body = vec![b];
    only(&project(vec![f], types()), "a RES binding");
    // A record field, and a field's element.
    for field in ["RES Integer STATE Cur", "List OF RES String STATE Cur"] {
        let mut all = types();
        all.push(record_typed("Holder", &[("h", field)]));
        let p = project(vec![func_returns("run", "Nothing", vec![], vec![])], all);
        only(&p, &format!("field {field}"));
    }
    // A native declaration's parameter.
    let mut p = project(
        vec![func_returns("run", "Nothing", vec![], vec![])],
        types(),
    );
    let mut poke = link_fn();
    poke.params = vec![(
        "path".to_string(),
        crate::types::ParameterType::parse("String STATE Cur"),
    )];
    p.link_functions = vec![poke];
    only(&p, "a native parameter");
}

/// bug-998: the `RES` marker's own rule is unchanged. It is reported for a
/// `RES` binding of a data type with or without a `STATE` clause — the clause's
/// rule is added beside it, it does not replace it.
#[test]
fn res_on_a_non_resource_is_still_the_res_rule() {
    for (type_, state_rule) in [("Integer", 0), ("Integer STATE Cur", 1)] {
        let mut f = func_returns("run", "Nothing", vec![], vec![]);
        let b = res_bind_owned(&mut f, "x", type_, Some(int_const("1")));
        f.body = vec![b];
        let got = rules(&project(
            vec![f],
            vec![record_typed("Cur", &[("n", "Integer")])],
        ));
        assert_eq!(
            count_rule(&got, "TYPE_RES_REQUIRES_RESOURCE"),
            1,
            "{type_}: {got:?}"
        );
        assert_eq!(
            count_rule(&got, "TYPE_STATE_REQUIRES_RESOURCE"),
            state_rule,
            "{type_}: {got:?}"
        );
    }
}

/// bug-1013 across a package boundary, on the SOURCE path, where the
/// importer's own type table holds only its own declarations: the kind of an
/// imported type is read from the package's type table. An imported record is
/// accepted (libsnd's `STATE libsnd::SoundInfo`); an imported ENUM or UNION is
/// refused; and an imported record or enum is positively NOT a resource
/// (bug-998).
#[test]
fn imported_types_are_judged_by_their_kind() {
    use crate::ir::{ImportedTypeDef, ImportedTypeField, ImportedTypeKind, ImportedTypeVariant};
    let imported = vec![
        ImportedTypeDef {
            name: "pkg.Cursor".to_string(),
            kind: ImportedTypeKind::Record,
            fields: vec![ImportedTypeField {
                name: "pos".to_string(),
                type_: crate::types::ParameterType::Integer,
            }],
            variants: vec![],
            members: vec![],
        },
        ImportedTypeDef {
            name: "pkg.Mode".to_string(),
            kind: ImportedTypeKind::Enum,
            fields: vec![],
            variants: vec![],
            members: vec!["Fast".to_string(), "Slow".to_string()],
        },
        ImportedTypeDef {
            name: "pkg.Shape".to_string(),
            kind: ImportedTypeKind::Union,
            fields: vec![],
            variants: vec![ImportedTypeVariant {
                name: "pkg.Dot".to_string(),
                fields: vec![],
            }],
            members: vec![],
        },
    ];
    let source_rules = |param_type: &str| -> Vec<String> {
        let f = func_returns("run", "Nothing", vec![param("p", param_type, None)], vec![]);
        super::collect_source_diagnostics(
            &project(vec![f], vec![]),
            std::path::Path::new("/proj"),
            &[],
            &imported,
            &[],
            &crate::ir::LinkSpans::default(),
        )
        .into_iter()
        .map(|d| d.rule)
        .collect()
    };
    let got = source_rules("fs.File STATE pkg.Cursor");
    assert_eq!(count_rule(&got, "TYPE_STATE_INVALID"), 0, "{got:?}");
    for state in ["pkg.Mode", "pkg.Shape"] {
        let got = source_rules(&format!("fs.File STATE {state}"));
        assert_eq!(
            count_rule(&got, "TYPE_STATE_INVALID"),
            1,
            "{state}: {got:?}"
        );
    }
    for host in ["pkg.Cursor", "pkg.Mode"] {
        let got = source_rules(&format!("{host} STATE pkg.Cursor"));
        assert_eq!(
            count_rule(&got, "TYPE_STATE_REQUIRES_RESOURCE"),
            1,
            "{host}: {got:?}"
        );
        assert_eq!(count_rule(&got, "TYPE_STATE_INVALID"), 0, "{host}: {got:?}");
    }
    // A name no table has heard of may be an external package's resource, and
    // its STATE type an external record: neither rule guesses.
    let got = source_rules("ext.Handle STATE ext.Info");
    assert_eq!(count_rule(&got, "TYPE_STATE_INVALID"), 0, "{got:?}");
    assert_eq!(
        count_rule(&got, "TYPE_STATE_REQUIRES_RESOURCE"),
        0,
        "{got:?}"
    );
}
