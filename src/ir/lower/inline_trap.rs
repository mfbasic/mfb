//! plan-176-N: IR lowering of inline `TRAP` — the trap target, hoisted trap slots, and
//! treeifying a handler's continuation. The lowering context and entry points live in
//! `ir/lower/mod.rs`.

use super::*;

/// Where the recovered/`Ok` value of an inline `TRAP` is delivered.
pub(super) enum InlineTrapTarget {
    /// `LET`/`MUT name = <call> TRAP(e) …`
    Bind {
        mutable: bool,
        name: String,
        type_: ParameterType,
        explicit_type: bool,
    },
    /// `name = <call> TRAP(e) …`
    Assign { name: String },
    /// `resource.state = <call> TRAP(e) …`, and the `resource.state.field =`
    /// form the parser desugars into a single-field `WITH` update over it.
    ///
    /// bug-551: this variant did not exist and `HirStatement::StateAssign` did
    /// not look for a `Trapped` value, so a trap in either position reached
    /// value lowering and hit `unreachable!("inline TRAP must be lowered as a
    /// statement value")` — a compiler panic on a program the parser had
    /// already accepted, and one it accepted deliberately: `ast::stmt` skips the
    /// statement terminator for exactly this shape.
    StateAssign { resource: String },
    /// `<call> TRAP(e) …` as a bare statement (value discarded).
    Discard,
}

/// Whether any op in `ops` (recursively, including nested control-flow bodies)
/// reads the local `name`. plan-64-I: an inline-`TRAP` handler that never reads
/// its bound error `binding` needs no `Bind err = ResultError`, so eliding it
/// drops a dead `Error` deep-copy (`ResultError` is an aliasing source, so the
/// bind lowers to `copy_flat_block`) on every err-ignoring `TRAP`. Conservative:
/// a shadowing rebind of the same name that is read only keeps the (correct)
/// full error assembly.
fn ops_read_local(ops: &[IrOp], name: &str) -> bool {
    fn value_reads(value: &IrValue, name: &str) -> bool {
        let mut found = false;
        crate::ir::value::visit_value(value, &mut |v| {
            if let IrValue::Local(local) = v {
                if local == name {
                    found = true;
                }
            }
        });
        found
    }
    ops.iter().any(|op| match op {
        IrOp::Bind {
            value: Some(value), ..
        }
        | IrOp::Assign { value, .. }
        | IrOp::AssignGlobal { value, .. }
        | IrOp::StateAssign { value, .. }
        | IrOp::ExitProgram { code: value, .. }
        | IrOp::Fail { error: value, .. }
        | IrOp::Eval { value, .. } => value_reads(value, name),
        IrOp::Bind { value: None, .. } | IrOp::ExitLoop { .. } | IrOp::ContinueLoop { .. } => false,
        IrOp::Return { value, .. } => value.as_ref().is_some_and(|v| value_reads(v, name)),
        IrOp::If {
            condition,
            then_body,
            else_body,
            ..
        } => {
            value_reads(condition, name)
                || ops_read_local(then_body, name)
                || ops_read_local(else_body, name)
        }
        IrOp::Match { value, cases, .. } => {
            value_reads(value, name)
                || cases.iter().any(|case| {
                    case.guard.as_ref().is_some_and(|g| value_reads(g, name))
                        || ops_read_local(&case.body, name)
                })
        }
        IrOp::While {
            condition, body, ..
        } => value_reads(condition, name) || ops_read_local(body, name),
        IrOp::For {
            start,
            end,
            step,
            body,
            ..
        } => {
            value_reads(start, name)
                || value_reads(end, name)
                || value_reads(step, name)
                || ops_read_local(body, name)
        }
        IrOp::DoUntil {
            body, condition, ..
        } => ops_read_local(body, name) || value_reads(condition, name),
        IrOp::ForEach { iterable, body, .. } => {
            value_reads(iterable, name) || ops_read_local(body, name)
        }
        IrOp::Trap { body, .. } => ops_read_local(body, name),
    })
}

/// Lowers an inline `TRAP` to existing IR primitives (no backend support is
/// required). Every fallible call in the trapped expression is evaluated as a
/// raw `Result`; on `Ok` its value flows on, on `Err` the handler runs with `e`
/// bound. `RECOVER` stores its value into a shared slot and then falls through
/// to the delivery of the target, while diverging handler paths
/// (`RETURN`/`FAIL`/`PROPAGATE`) leave as usual. The handler is normalized so
/// that statements following a `RECOVER` in a branch do not execute after
/// recovery (see [`treeify_handler`]).
///
/// Two shapes come out of here.
///
/// * **One check** — the scrutinee has no fallible call nested inside it, so
///   only its outermost node is converted:
///
///   ```text
///   Bind $trap_res0 = CallResult(g(x))
///   Bind MUT $trap_val1 : T
///   If ResultIsOk($trap_res0) { $trap_val1 = ResultValue($trap_res0) }
///                        else { Bind e = ResultError($trap_res0); <handler> }
///   Bind y = $trap_val1
///   ```
///
///   This is the overwhelmingly common case and the shape `ir::verify`'s
///   `check_inline_trap_scrutinee` reads to report
///   `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE` (a scrutinee that is not a call at
///   all) and `TYPE_INLINE_TRAP_DEAD_HANDLER` (a provably-infallible built-in),
///   so a non-call scrutinee still lands here unchanged.
///
/// * **A check chain** (bug-457) — the scrutinee *does* contain a fallible call
///   nested inside it. Converting only the outermost node left the nested call a
///   plain `Call`, which auto-propagated straight past the handler to the
///   function-level trap: the handler never ran and nothing was reported. Each
///   nested fallible call is now lifted into its own check ahead of the residual
///   expression, nested so that a failure skips the rest of the expression:
///
///   ```text
///   Bind MUT $trap_failed : Boolean = false
///   Bind MUT $trap_err : Error
///   Bind MUT $trap_val : T
///   Bind $trap_res = CallResult(inner(-1))
///   If ResultIsOk($trap_res) {
///       Bind $trap_arg = ResultValue($trap_res)
///       $trap_val = outer($trap_arg)
///   } else { $trap_failed = true; $trap_err = ResultError($trap_res) }
///   If $trap_failed { Bind e = $trap_err; <handler> }
///   Bind y = $trap_val
///   ```
///
///   The handler is emitted **once**, behind the shared `$trap_failed` flag,
///   rather than cloned into every check's `else`: cloning would duplicate the
///   handler's own diagnostics and its lowered temps, and the flag reproduces
///   `RECOVER`'s fall-through (recovery assigns the slot and continues to the
///   delivery below) with a single copy.
pub(super) fn lower_inline_trap(
    inner: &HirExpression,
    binding: &str,
    handler: &[HirStatement],
    target: InlineTrapTarget,
    locals: &mut HashMap<String, ParameterType>,
    context: &mut LowerContext<'_>,
) -> Vec<IrOp> {
    // The inline-TRAP statement's span: the handler block below re-sets
    // `context.current_loc` per handler statement, so ops synthesized after it
    // must use this captured copy.
    let stmt_loc = context.current_loc;
    let success_type = expression_type(inner, locals, context).unwrap_or(ParameterType::Unknown);
    let result_type = ParameterType::result_of(success_type.clone());
    let mut root = lower_expression(inner, locals, context);
    // bug-457: lift the fallible calls nested inside the scrutinee. With none to
    // lift this is a no-op and the lowering below is byte-for-byte the one that
    // shipped before.
    //
    // plan-165-D (review 3.1, Option 1): any well-typed expression may be
    // trapped, so every scrutinee is lifted — a call-free `i * 2 + 3` included
    // (bug-779 used to leave it unlifted so `ir::verify` would reject it).
    let hoists = hoist_trap_calls(&mut root, locals, context);
    // Whether anything lifted can route an error to the handler. When nothing
    // can, the scrutinee's own node is the one check (see `check_root`), and any
    // lift is an unchecked call kept only for evaluation order.
    let chain_can_fail = hoists.iter().any(TrapHoist::can_fail);
    // plan-165-D: a package constant is a `Call` node but not a runtime call; as a
    // root it is checked like any other non-call value.
    let root_is_runtime_call = matches!(
        &root,
        IrValue::Call { target, .. } if !builtins::is_package_constant(target)
    );
    // plan-165-D: a discarded `Float` value has no observation boundary (an
    // untrapped `f * f` statement does not observe it), so its root check
    // captures domain errors only; every other target observes the value it
    // delivers, as the untrapped binding or assignment does.
    let root_observes = !(matches!(target, InlineTrapTarget::Discard)
        && matches!(success_type, ParameterType::Float));

    // Whether the scrutinee's own outermost node still gets a `Result` check.
    // With nothing lifted it always does, including when it is not a call —
    // that bind is the evidence `ir::verify` reports the non-call scrutinee on.
    // Once a nested call has been lifted the handler is demonstrably live, so an
    // outermost node that cannot fail (`toString(parse(s))`, or no call at all)
    // is left as a plain value instead of a needless always-`Ok` check.
    // bug-471: with something already lifted, a raising *operator* at the root
    // is checked too — `LET c = inner(x) + 1 TRAP(e)` (the shape `mfb spec
    // language error-model` §8.6 rule 11 explicitly blesses) must catch the
    // `+`'s overflow, not just `inner`'s failure. With nothing lifted the root
    // is left unchecked whatever it is: that bare bind is the evidence
    // `ir::verify` reports `TYPE_INLINE_TRAP_REQUIRES_FALLIBLE` on, and an
    // inline `TRAP` still traps a *call* — a scrutinee that is only an operator
    // is rejected exactly as before.
    let root_operator_raises = chain_can_fail
        && matches!(&root, IrValue::Binary { .. } | IrValue::Unary { .. })
        && !is_conditional_root(&root)
        && trap_hoist_kind(
            &root,
            false,
            context.fallible,
            locals,
            &context.binding_types,
            context.type_index,
        ) == Some(true);
    let check_root = match &root {
        // bug-486: the census is asked with the root call's ARGUMENT types, not
        // its name alone, because some verdicts turn on the overload
        // (`builtins::inline_builtin_fallibility_depends_on_args`). bug-486's case
        // was `toString(<bytes>)`, then the one fallible `toString`: answering by
        // name left it a plain `Call` whenever the scrutinee also hoisted, and the
        // raise walked straight past the handler. (Since bug-807 every `toString`
        // is fallible by name; `replace` and `toInt` still turn on the overload.)
        IrValue::Call { target, args, .. } if root_is_runtime_call => {
            !chain_can_fail
                || context.fallible.call_is_fallible(
                    target,
                    &ir_call_arg_types(target, args, locals, &context.binding_types),
                    context.type_index,
                )
        }
        // plan-165-D: with nothing lifted that can fail, a non-call root is the
        // one check — a raising operator captured, a known-total expression
        // wrapped in a `Checked` that cannot fail (its dead handler is
        // `ir::verify`'s `TYPE_INLINE_TRAP_DEAD_HANDLER`).
        _ => !chain_can_fail || root_operator_raises,
    };
    // Allocated before the value slot so the no-hoist lowering keeps its
    // historical temp numbering.
    let res_name = make_temp_local_name(context, "trap_res");
    let (root_result, root_plain) = if check_root {
        locals.insert(res_name.clone(), result_type.clone());
        let checked = match root {
            IrValue::Call {
                target, args, loc, ..
            } if root_is_runtime_call => IrValue::CallResult {
                target,
                args,
                // The fallible form's success type is the call's own result type.
                type_: success_type.clone(),
                loc,
            },
            // A non-call root has no callee to return a `Result`, so it is
            // wrapped instead (bug-471; plan-165-D for a root nothing was lifted
            // from, raising or total).
            other => IrValue::Checked {
                type_: success_type.clone(),
                value: Box::new(other),
                observe: root_observes,
            },
        };
        (Some(checked), None)
    } else {
        (None, Some(root))
    };

    // A shared slot carries the value on both the Ok and RECOVER paths so the
    // target binding/assignment is produced exactly once after the branch.
    let slot = match &target {
        InlineTrapTarget::Bind { .. }
        | InlineTrapTarget::Assign { .. }
        | InlineTrapTarget::StateAssign { .. } => {
            let val_name = make_temp_local_name(context, "trap_val");
            locals.insert(val_name.clone(), success_type.clone());
            Some(val_name)
        }
        InlineTrapTarget::Discard => None,
    };

    let mut handler_locals = locals.clone();
    handler_locals.insert(binding.to_string(), ParameterType::named("Error"));
    context.recover_targets.push(RecoverTarget {
        slot: slot.clone(),
        type_: success_type.clone(),
    });
    let normalized = treeify_handler(handler);
    let handler_ops = lower_statement_block(&normalized, &handler_locals, context, Some(binding));
    context.recover_targets.pop();
    // plan-64-I: only bind the error `err` when the handler actually reads it.
    // `ResultError` is an aliasing source, so `Bind err = ResultError(res)`
    // lowers to a deep `copy_flat_block` of the `Error`; when `RECOVER` ignores
    // `err`, that copy (and, for a conversion `CallResult` receiver, the whole
    // ErrorLoc/Error assembly it forces — see `emit_error_register_return`'s
    // discard path) is dead work. Eliding it is what lets the conversion error
    // path materialize only a bare tag.
    let handler_reads_err = ops_read_local(&handler_ops, binding);

    // The Ok arm of the outermost node's check: deliver its value into the slot.
    let root_then = match (&root_result, &slot) {
        (Some(_), Some(val_name)) => vec![IrOp::Assign {
            name: val_name.clone(),
            value: IrValue::ResultValue {
                type_: success_type.clone(),
                value: Box::new(IrValue::Local(res_name.clone())),
            },
            loc: stmt_loc,
        }],
        _ => Vec::new(),
    };

    let mut ops = Vec::new();
    if !chain_can_fail {
        let call_result =
            root_result.expect("a scrutinee with nothing lifted that can fail is always checked");
        ops.push(IrOp::Bind {
            mutable: false,
            name: res_name.clone(),
            type_: result_type,
            value: Some(call_result),
            loc: stmt_loc,
            explicit_type: false,
        });
        if let Some(val_name) = &slot {
            ops.push(IrOp::Bind {
                mutable: true,
                name: val_name.clone(),
                type_: success_type.clone(),
                value: None,
                loc: stmt_loc,
                explicit_type: false,
            });
        }
        let mut else_body = Vec::new();
        if handler_reads_err {
            else_body.push(IrOp::Bind {
                mutable: false,
                name: binding.to_string(),
                type_: ParameterType::named("Error"),
                value: Some(IrValue::ResultError {
                    value: Box::new(IrValue::Local(res_name.clone())),
                }),
                loc: stmt_loc,
                explicit_type: false,
            });
        }
        else_body.extend(handler_ops);
        ops.push(IrOp::If {
            condition: IrValue::ResultIsOk {
                value: Box::new(IrValue::Local(res_name.clone())),
            },
            then_body: root_then,
            else_body,
            loc: stmt_loc,
        });
        // plan-165-D: calls lifted only to keep evaluation order (none of them can
        // fail) run ahead of the one check, as plain binds; with nothing lifted
        // this is a no-op.
        ops = wrap_trap_hoists(hoists, ops, "", &|_| {
            unreachable!("an unchecked lift has no failure arm")
        });
    } else {
        // The shared failure flag and error slot the whole chain reports through.
        let failed_name = make_temp_local_name(context, "trap_failed");
        locals.insert(failed_name.clone(), ParameterType::Boolean);
        let err_name = handler_reads_err.then(|| {
            let name = make_temp_local_name(context, "trap_err");
            locals.insert(name.clone(), ParameterType::named("Error"));
            name
        });
        // The `else` arm every check shares: record the failure and the error,
        // then fall out of the chain to the single handler below.
        let fail_arm = |res: &str| {
            let mut arm = vec![IrOp::Assign {
                name: failed_name.clone(),
                value: IrValue::Const {
                    type_: ParameterType::Boolean,
                    value: "true".to_string(),
                },
                loc: stmt_loc,
            }];
            if let Some(err_name) = &err_name {
                arm.push(IrOp::Assign {
                    name: err_name.clone(),
                    value: IrValue::ResultError {
                        value: Box::new(IrValue::Local(res.to_string())),
                    },
                    loc: stmt_loc,
                });
            }
            arm
        };

        ops.push(IrOp::Bind {
            mutable: true,
            name: failed_name.clone(),
            type_: ParameterType::Boolean,
            value: Some(IrValue::Const {
                type_: ParameterType::Boolean,
                value: "false".to_string(),
            }),
            loc: stmt_loc,
            explicit_type: false,
        });
        if let Some(err_name) = &err_name {
            ops.push(IrOp::Bind {
                mutable: true,
                name: err_name.clone(),
                type_: ParameterType::named("Error"),
                value: None,
                loc: stmt_loc,
                explicit_type: false,
            });
        }
        if let Some(val_name) = &slot {
            ops.push(IrOp::Bind {
                mutable: true,
                name: val_name.clone(),
                type_: success_type.clone(),
                value: None,
                loc: stmt_loc,
                explicit_type: false,
            });
        }

        // Innermost: the residual expression, checked or plain.
        let body = match (root_result, root_plain) {
            (Some(call_result), _) => vec![
                IrOp::Bind {
                    mutable: false,
                    name: res_name.clone(),
                    type_: result_type,
                    value: Some(call_result),
                    loc: stmt_loc,
                    explicit_type: false,
                },
                IrOp::If {
                    condition: IrValue::ResultIsOk {
                        value: Box::new(IrValue::Local(res_name.clone())),
                    },
                    then_body: root_then,
                    else_body: fail_arm(&res_name),
                    loc: stmt_loc,
                },
            ],
            (None, Some(plain)) => match &slot {
                Some(val_name) => vec![IrOp::Assign {
                    name: val_name.clone(),
                    value: plain,
                    loc: stmt_loc,
                }],
                None => vec![IrOp::Eval {
                    value: plain,
                    loc: stmt_loc,
                }],
            },
            (None, None) => Vec::new(),
        };

        // Wrap the lifted calls around it, innermost last, so each one's Ok arm
        // carries everything evaluated after it and a failure skips the rest.
        let mut body = wrap_trap_hoists(hoists, body, &failed_name, &fail_arm);
        ops.append(&mut body);

        // The single handler, reached from whichever check failed.
        let mut handler_block = Vec::new();
        if let Some(err_name) = &err_name {
            handler_block.push(IrOp::Bind {
                mutable: false,
                name: binding.to_string(),
                type_: ParameterType::named("Error"),
                value: Some(IrValue::Local(err_name.clone())),
                loc: stmt_loc,
                explicit_type: false,
            });
        }
        handler_block.extend(handler_ops);
        ops.push(IrOp::If {
            condition: IrValue::Local(failed_name),
            then_body: handler_block,
            else_body: Vec::new(),
            loc: stmt_loc,
        });
    }

    match target {
        InlineTrapTarget::Bind {
            mutable,
            name,
            type_,
            explicit_type,
        } => {
            let value = wrap_trap_slot_value(
                IrValue::Local(slot.expect("bind target has a value slot")),
                &success_type,
                Some(&type_),
                context,
            );
            ops.push(IrOp::Bind {
                mutable,
                name: name.clone(),
                type_: type_.clone(),
                value: Some(value),
                explicit_type,
                loc: stmt_loc,
            });
            if mutable {
                context.mutable_locals.insert(name.clone());
            }
            locals.insert(name, type_);
        }
        InlineTrapTarget::Assign { name } => {
            // The ordinary assign path resolves its target's declared type the
            // same way before lowering the value with it as `expected`; see the
            // `HirStatement::Assign` arm.
            let expected = locals
                .get(&name)
                .or_else(|| context.binding_types.get(&name))
                .cloned();
            let value = wrap_trap_slot_value(
                IrValue::Local(slot.expect("assign target has a value slot")),
                &success_type,
                expected.as_ref(),
                context,
            );
            if locals.contains_key(&name) {
                ops.push(IrOp::Assign {
                    name,
                    value,
                    loc: stmt_loc,
                });
            } else {
                ops.push(IrOp::AssignGlobal {
                    name,
                    value,
                    loc: stmt_loc,
                });
            }
        }
        InlineTrapTarget::StateAssign { resource } => {
            // No wrap here: a STATE type must be "a copyable, defaultable data
            // type" (`TYPE_STATE_INVALID`, 2-203-0085), which rejects a union
            // outright, so a STATE write can never need the variant->union
            // coercion the two arms above do.
            ops.push(IrOp::StateAssign {
                resource,
                value: IrValue::Local(slot.expect("state-assign target has a value slot")),
                loc: stmt_loc,
            });
        }
        InlineTrapTarget::Discard => {}
    }

    ops
}

/// Coerce an inline-`TRAP`'s staged slot value to the target's declared type.
///
/// The slot is typed with the PRODUCER's type, which for `LET v AS Val =
/// makeNum(7) TRAP …` is the variant `Num`, not the union `Val`. Every other
/// expected-type position lowers its value through
/// [`lower_expression_with_expected`], whose [`wrap_union_value`] inserts the
/// `UnionWrap` that gives the value its `{tag, payload}` union representation.
/// The staged slot is not a lowered expression, so it gets no such wrap, and
/// [`lower_inline_trap`] delivered a bare
/// `IrValue::Local(slot)` instead, so the binding held an untagged variant:
/// `MATCH` on it read a tag that was never written and fell through EVERY case,
/// silently, with a clean exit (bug-642's widened scope). For a RESOURCE union
/// the same omission also made the bind look like the aliasing shape
/// (`NirValue::Local` rather than `NirValue::UnionWrap`) to
/// `runtime::usage::push_op_helpers`, which then declared none of the variants'
/// close helpers while the used-side scan counted them all — the build failure
/// bug-642 was filed for.
///
/// `variant_belongs_to_union` is a lookup in a by-name table, so a union is not
/// a variant of itself and an already-union-typed producer (`shapeOf` returning
/// `Shape`) is left alone. The declared type is `STATE`-stripped for both the
/// membership test and the wrap, matching what the ordinary path emits for
/// `RES c AS Chan STATE Cur` (`union: "Chan"`).
fn wrap_trap_slot_value(
    value: IrValue,
    slot_type: &ParameterType,
    declared: Option<&ParameterType>,
    context: &mut LowerContext<'_>,
) -> IrValue {
    let Some(declared) = declared else {
        return value;
    };
    // The same coercion every lowered expression gets: a variant is wrapped, and
    // a collection of variants is converted (bug-989).
    union_conversion(value, slot_type, &declared.without_state(), context)
}

/// One call lifted out of an inline-`TRAP` scrutinee (bug-457).
pub(super) struct TrapHoist {
    /// The local the residual expression reads in the call's place.
    pub(super) name: String,
    /// The call's own value type.
    pub(super) type_: ParameterType,
    /// The lifted call: a `CallResult` when it is checked, a plain `Call` when
    /// it is lifted only to keep evaluation order.
    pub(super) value: IrValue,
    /// The `Result` temp its check binds, when it is checked.
    pub(super) res: Option<String>,
    pub(super) loc: IrSourceLoc,
    /// plan-165-C: a short-circuit `AND`/`OR` lifted as a conditional step. Then
    /// `name` is the Boolean result local, `value` the (already rewritten) left
    /// operand, and `res` is `None`; the right operand's own lifts run only when
    /// the left one does not decide the result.
    pub(super) conditional: Option<ConditionalStep>,
}

/// The right half of a conditional lift (plan-165-C).
pub(super) struct ConditionalStep {
    /// `AND` (right runs when the left is true) or `OR` (when it is false).
    pub(super) op: BinaryOp,
    /// The right operand's lifts, in evaluation order.
    pub(super) right: Vec<TrapHoist>,
    /// The right operand's residual, assigned to the result local once its lifts
    /// have succeeded.
    pub(super) right_value: IrValue,
}

/// The ops for a chain of lifted steps around `body`: each lift's `Ok` arm
/// carries everything evaluated after it, so a failure skips the rest, and a
/// failed check runs `fail_arm` (which records the error behind the shared
/// failure flag `failed`).
fn wrap_trap_hoists(
    hoists: Vec<TrapHoist>,
    mut body: Vec<IrOp>,
    failed: &str,
    fail_arm: &dyn Fn(&str) -> Vec<IrOp>,
) -> Vec<IrOp> {
    for hoist in hoists.into_iter().rev() {
        let TrapHoist {
            name,
            type_,
            value,
            res,
            loc,
            conditional,
        } = hoist;
        if let Some(ConditionalStep {
            op,
            right,
            right_value,
        }) = conditional
        {
            // plan-165-C: bind the left operand's value, run the right operand's
            // own chain only when the left does not decide the result, and
            // continue only if that chain did not fail. The result local is
            // initialized on every path, so nothing after the step can read it
            // unset; a skipped branch binds none of the right's temporaries.
            let right_can_fail = right.iter().any(TrapHoist::can_fail);
            let right_body = wrap_trap_hoists(
                right,
                vec![IrOp::Assign {
                    name: name.clone(),
                    value: right_value,
                    loc,
                }],
                failed,
                fail_arm,
            );
            let guard = match op {
                BinaryOp::Or => IrValue::Unary {
                    op: UnaryOp::Not,
                    operand: Box::new(IrValue::Local(name.clone())),
                    type_: ParameterType::Boolean,
                    loc,
                },
                _ => IrValue::Local(name.clone()),
            };
            let mut step = vec![
                IrOp::Bind {
                    mutable: true,
                    name,
                    type_,
                    value: Some(value),
                    loc,
                    explicit_type: false,
                },
                IrOp::If {
                    condition: guard,
                    then_body: right_body,
                    else_body: Vec::new(),
                    loc,
                },
            ];
            if right_can_fail && !body.is_empty() {
                step.push(IrOp::If {
                    condition: IrValue::Unary {
                        op: UnaryOp::Not,
                        operand: Box::new(IrValue::Local(failed.to_string())),
                        type_: ParameterType::Boolean,
                        loc,
                    },
                    then_body: body,
                    else_body: Vec::new(),
                    loc,
                });
            } else {
                step.append(&mut body);
            }
            body = step;
            continue;
        }
        match res {
            Some(res) => {
                let mut then_body = vec![IrOp::Bind {
                    mutable: false,
                    name,
                    type_: type_.clone(),
                    value: Some(IrValue::ResultValue {
                        type_: type_.clone(),
                        value: Box::new(IrValue::Local(res.clone())),
                    }),
                    loc,
                    explicit_type: false,
                }];
                then_body.append(&mut body);
                let else_body = fail_arm(&res);
                body = vec![
                    IrOp::Bind {
                        mutable: false,
                        name: res.clone(),
                        type_: ParameterType::result_of(type_),
                        value: Some(value),
                        loc,
                        explicit_type: false,
                    },
                    IrOp::If {
                        condition: IrValue::ResultIsOk {
                            value: Box::new(IrValue::Local(res)),
                        },
                        then_body,
                        else_body,
                        loc,
                    },
                ];
            }
            None => {
                let mut lifted = vec![IrOp::Bind {
                    mutable: false,
                    name,
                    type_,
                    value: Some(value),
                    loc,
                    explicit_type: false,
                }];
                lifted.append(&mut body);
                body = lifted;
            }
        }
    }
    body
}

impl TrapHoist {
    /// Whether this step can route an error to the handler: a checked lift, or
    /// a conditional whose right operand holds one.
    fn can_fail(&self) -> bool {
        self.res.is_some()
            || self
                .conditional
                .as_ref()
                .is_some_and(|step| step.right.iter().any(TrapHoist::can_fail))
    }
}

/// Whether `value` is a short-circuit `AND`/`OR` whose right operand holds
/// something the inline-`TRAP` desugar must keep conditional (plan-165-C).
pub(super) fn is_conditional_root(value: &IrValue) -> bool {
    matches!(value, IrValue::Binary { op, .. } if is_short_circuit_operator(*op))
}

/// Depth cap for the two inline-`TRAP` scrutinee walks. Mirrors
/// [`crate::ir::value::VALUE_VISIT_MAX_DEPTH`], the IR verifier's own value-walk
/// bound; the scan and the rewrite must stop at the *same* depth or they would
/// disagree on which call node an index names.
pub(super) const TRAP_SCRUTINEE_MAX_DEPTH: usize = crate::ir::value::VALUE_VISIT_MAX_DEPTH;

/// Whether `op` only evaluates its right operand when the left one does not
/// already decide the result (`mfb spec language operators`). A call in that
/// position cannot be hoisted whole — that would evaluate it unconditionally —
/// so an `AND`/`OR` holding one is lifted as a conditional step instead
/// (plan-165-C, [`wrap_trap_hoists`]).
pub(super) fn is_short_circuit_operator(op: BinaryOp) -> bool {
    matches!(op, BinaryOp::And | BinaryOp::Or)
}

/// Normalizes an inline-`TRAP` handler so that a `RECOVER` (which is lowered as
/// an assignment that falls through to the post-trap continuation) never lets
/// statements that follow it in a sibling position execute. Statements after a
/// branching statement (`IF`/`MATCH`) whose branch falls through are pushed into
/// that fall-through branch, so each leaf path ends in its own terminator and
/// the structured lowering needs no jumps. Statements after a terminator are
/// unreachable and dropped.
fn treeify_handler(stmts: &[HirStatement]) -> Vec<HirStatement> {
    let Some((head, tail)) = stmts.split_first() else {
        return Vec::new();
    };

    if tail.is_empty() {
        return vec![treeify_statement(head)];
    }
    if statement_terminates(head) {
        // Anything after a terminator cannot run.
        return vec![treeify_statement(head)];
    }

    match head {
        HirStatement::If {
            condition,
            then_body,
            else_body,
            line,
        } => {
            // The continuation only has to be pushed *inside* a branch when that
            // branch can reach a terminator (`RECOVER`/`RETURN`/…), so a
            // recovered path does not fall through into the shared continuation.
            // When *neither* branch can terminate, the `IF` always falls through
            // to the continuation regardless of which branch runs, so keep the
            // continuation as a single shared sibling instead of cloning it into
            // both branches. Cloning into both is what makes N sequential
            // fall-through branches emit 2^N copies of the tail (bug-401).
            if !block_can_terminate(then_body) && !block_can_terminate(else_body) {
                let mut result = vec![HirStatement::If {
                    condition: condition.clone(),
                    then_body: treeify_handler(then_body),
                    else_body: treeify_handler(else_body),
                    line: *line,
                }];
                result.extend(treeify_handler(tail));
                result
            } else {
                let then_body = distribute_continuation(then_body, tail);
                let else_body = distribute_continuation(else_body, tail);
                vec![HirStatement::If {
                    condition: condition.clone(),
                    then_body,
                    else_body,
                    line: *line,
                }]
            }
        }
        HirStatement::Match {
            expression,
            cases,
            line,
        } => {
            // As with `IF` above: only distribute the continuation into the arms
            // when some arm can terminate. Otherwise the match always falls
            // through (a matched non-terminating arm or an unmatched scrutinee
            // alike), so keep the continuation as a shared sibling — no per-arm
            // clone and no synthesized `ELSE` (bug-401).
            if !cases.iter().any(|case| block_can_terminate(&case.body)) {
                let new_cases: Vec<HirMatchCase> = cases
                    .iter()
                    .map(|case| HirMatchCase {
                        pattern: case.pattern.clone(),
                        guard: case.guard.clone(),
                        body: treeify_handler(&case.body),
                        line: case.line,
                    })
                    .collect();
                let mut result = vec![HirStatement::Match {
                    expression: expression.clone(),
                    cases: new_cases,
                    line: *line,
                }];
                result.extend(treeify_handler(tail));
                return result;
            }
            let mut new_cases: Vec<HirMatchCase> = cases
                .iter()
                .map(|case| HirMatchCase {
                    pattern: case.pattern.clone(),
                    guard: case.guard.clone(),
                    body: distribute_continuation(&case.body, tail),
                    line: case.line,
                })
                .collect();
            // An unmatched scrutinee falls through to the continuation, so make
            // that path explicit unless an ELSE arm already covers it.
            let has_else = cases
                .iter()
                .any(|case| matches!(case.pattern, HirMatchPattern::Else) && case.guard.is_none());
            if !has_else {
                new_cases.push(HirMatchCase {
                    pattern: HirMatchPattern::Else,
                    guard: None,
                    body: treeify_handler(tail),
                    line: *line,
                });
            }
            vec![HirStatement::Match {
                expression: expression.clone(),
                cases: new_cases,
                line: *line,
            }]
        }
        _ => {
            // A non-branching, non-terminating statement falls through to the
            // continuation; keep it and continue normalizing the tail.
            let mut result = vec![treeify_statement(head)];
            result.extend(treeify_handler(tail));
            result
        }
    }
}

/// Appends `continuation` to a block's fall-through paths, then normalizes it.
fn distribute_continuation(
    body: &[HirStatement],
    continuation: &[HirStatement],
) -> Vec<HirStatement> {
    if block_terminates(body) {
        treeify_handler(body)
    } else {
        let mut combined = body.to_vec();
        combined.extend_from_slice(continuation);
        treeify_handler(&combined)
    }
}

/// Recurses into a statement's nested blocks without distributing any
/// continuation (used when there is nothing following the statement).
fn treeify_statement(statement: &HirStatement) -> HirStatement {
    match statement {
        HirStatement::If {
            condition,
            then_body,
            else_body,
            line,
        } => HirStatement::If {
            condition: condition.clone(),
            then_body: treeify_handler(then_body),
            else_body: treeify_handler(else_body),
            line: *line,
        },
        HirStatement::Match {
            expression,
            cases,
            line,
        } => HirStatement::Match {
            expression: expression.clone(),
            cases: cases
                .iter()
                .map(|case| HirMatchCase {
                    pattern: case.pattern.clone(),
                    guard: case.guard.clone(),
                    body: treeify_handler(&case.body),
                    line: case.line,
                })
                .collect(),
            line: *line,
        },
        HirStatement::While {
            kind,
            condition,
            body,
            line,
        } => HirStatement::While {
            kind: *kind,
            condition: condition.clone(),
            body: treeify_handler(body),
            line: *line,
        },
        HirStatement::DoUntil {
            body,
            condition,
            line,
        } => HirStatement::DoUntil {
            body: treeify_handler(body),
            condition: condition.clone(),
            line: *line,
        },
        HirStatement::For {
            name,
            start,
            end,
            step,
            body,
            line,
        } => HirStatement::For {
            name: name.clone(),
            start: start.clone(),
            end: end.clone(),
            step: step.clone(),
            body: treeify_handler(body),
            line: *line,
        },
        HirStatement::ForEach {
            name,
            iterable,
            body,
            line,
        } => HirStatement::ForEach {
            name: name.clone(),
            iterable: iterable.clone(),
            body: treeify_handler(body),
            line: *line,
        },
        other => other.clone(),
    }
}

/// Whether executing `stmts` always ends in a terminator (never reaches the end
/// of the block).
fn block_terminates(stmts: &[HirStatement]) -> bool {
    stmts.iter().any(statement_terminates)
}

/// Whether *some* path through the block can reach a terminator. Unlike
/// [`block_terminates`] (which requires *every* path to terminate), this is true
/// if a terminator is reachable on any path. `treeify_handler` uses it to decide
/// whether an inline-`TRAP` handler continuation must be distributed into a
/// branch (because a recovered path must not fall into it) or may be shared as a
/// single sibling after the branch (bug-401).
fn block_can_terminate(stmts: &[HirStatement]) -> bool {
    stmts.iter().any(statement_can_terminate)
}

/// Whether a statement can reach a terminator on some path. See
/// [`block_can_terminate`]. Being conservative (returning `true` when unsure)
/// only forces distribution, which is always semantically safe; a spurious
/// `false` would be a bug, so every terminator form and every nested block is
/// covered.
fn statement_can_terminate(statement: &HirStatement) -> bool {
    match statement {
        HirStatement::Return { .. }
        | HirStatement::Exit { .. }
        | HirStatement::Continue { .. }
        | HirStatement::Fail { .. }
        | HirStatement::Propagate { .. }
        | HirStatement::Recover { .. } => true,
        HirStatement::If {
            then_body,
            else_body,
            ..
        } => block_can_terminate(then_body) || block_can_terminate(else_body),
        HirStatement::Match { cases, .. } => {
            cases.iter().any(|case| block_can_terminate(&case.body))
        }
        HirStatement::For { body, .. }
        | HirStatement::ForEach { body, .. }
        | HirStatement::While { body, .. }
        | HirStatement::DoUntil { body, .. } => block_can_terminate(body),
        _ => false,
    }
}

/// Whether a statement always diverges or recovers (ends its enclosing handler
/// path). Mirrors the former source checker's flow analysis for the constructs an inline-trap
/// handler may contain.
fn statement_terminates(statement: &HirStatement) -> bool {
    match statement {
        HirStatement::Return { .. }
        | HirStatement::Exit { .. }
        | HirStatement::Continue { .. }
        | HirStatement::Fail { .. }
        | HirStatement::Propagate { .. }
        | HirStatement::Recover { .. } => true,
        HirStatement::If {
            then_body,
            else_body,
            ..
        } => !else_body.is_empty() && block_terminates(then_body) && block_terminates(else_body),
        HirStatement::Match { cases, .. } => {
            let has_else = cases
                .iter()
                .any(|case| matches!(case.pattern, HirMatchPattern::Else) && case.guard.is_none());
            has_else && !cases.is_empty() && cases.iter().all(|case| block_terminates(&case.body))
        }
        _ => false,
    }
}
