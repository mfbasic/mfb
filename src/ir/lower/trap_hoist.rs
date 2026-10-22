//! plan-176-N: IR lowering of hoisting fallible calls out of trap operands. The lowering
//! context and entry points live in `ir/lower/mod.rs`.

use super::*;

#[cfg(test)]
mod tests;

/// Lifts the calls nested inside an inline-`TRAP` scrutinee out in front of it,
/// rewriting `root` to read their results from locals (bug-457).
///
/// Returns them in evaluation order, empty when the scrutinee has no fallible
/// call nested inside it — which is the common single-call shape, left
/// untouched.
///
/// Everything up to and including the **last** fallible call is lifted, not just
/// the fallible ones: the lifted binds all run before the residual expression,
/// so a side-effecting infallible call left behind would move *after* a fallible
/// call it used to precede. Calls past the last fallible one need no such care
/// and stay in the expression — except under a root that becomes a `Checked`,
/// which may hold no call, where every indexed node is lifted.
pub(super) fn hoist_trap_calls(
    root: &mut IrValue,
    locals: &mut HashMap<String, ParameterType>,
    context: &mut LowerContext<'_>,
) -> Vec<TrapHoist> {
    // The scrutinee's own outermost node is handled by the caller, so only its
    // operands are scanned — except a conditional root (plan-165-C): an `AND`/
    // `OR` whose right operand must stay conditional is itself a lifted step, so
    // it is scanned (and below, rewritten) as a node and the residual becomes a
    // read of its result local.
    let root_is_conditional_step = is_conditional_root(root)
        && trap_hoist_kind(
            root,
            false,
            context.fallible,
            locals,
            &context.binding_types,
            context.type_index,
        )
        .is_some();
    let mut fallible = Vec::new();
    if root_is_conditional_step {
        scan_trap_call(
            root,
            false,
            context.fallible,
            locals,
            &context.binding_types,
            context.type_index,
            0,
            &mut fallible,
        );
    } else {
        scan_trap_operands(
            root,
            context.fallible,
            locals,
            &context.binding_types,
            context.type_index,
            0,
            &mut fallible,
        );
    }
    // A raising operator at the root is wrapped in `Checked` once something is lifted
    // (bug-471), and a `Checked` may hold no call (`ir::verify`): a call past the last
    // fallible one would stay inside it. So under such a root every indexed node is lifted —
    // `len(stringify(parse(a))) + len(stringify(parse(b)))` left the second `len` behind and
    // failed the build with `Checked wraps a call to len`.
    let root_is_checked_operator = matches!(root, IrValue::Binary { .. } | IrValue::Unary { .. })
        && !root_is_conditional_step
        && trap_hoist_kind(
            root,
            false,
            context.fallible,
            locals,
            &context.binding_types,
            context.type_index,
        ) == Some(true);
    let limit = match fallible.iter().rposition(|f| *f) {
        Some(_) if root_is_checked_operator => fallible.len(),
        Some(last_fallible) => last_fallible + 1,
        // plan-165-D: nothing can fail. A runtime-call root is the one check and
        // evaluates its own arguments in order, so nothing is lifted (the
        // historical shape). Any other root becomes a `Checked`, which may hold no
        // call either, so every indexed node — each an unchecked call or
        // conditional — is lifted ahead of it.
        None => {
            let root_is_runtime_call = matches!(
                root,
                IrValue::Call { target, .. } if !builtins::is_package_constant(target)
            );
            if root_is_runtime_call || fallible.is_empty() {
                return Vec::new();
            }
            fallible.len()
        }
    };
    let mut hoists = Vec::new();
    let mut index = 0;
    if root_is_conditional_step {
        rewrite_trap_call(
            root,
            false,
            &fallible,
            limit,
            &mut index,
            0,
            &mut hoists,
            locals,
            context,
        );
    } else {
        rewrite_trap_operands(
            root,
            &fallible,
            limit,
            &mut index,
            0,
            &mut hoists,
            locals,
            context,
        );
    }
    hoists
}

/// Records, in evaluation order, whether each call node under `value` is
/// fallible. `value`'s own node is not recorded — only its operands.
fn scan_trap_operands(
    value: &IrValue,
    fallible: &Fallibility,
    locals: &HashMap<String, ParameterType>,
    globals: &HashMap<String, ParameterType>,
    // bug-679: the declared-kind oracle, so `toInt(<enum>)` is seen as the
    // infallible overload it is and stays a plain `Call`.
    kinds: &dyn builtins::TypeKinds,
    depth: usize,
    out: &mut Vec<bool>,
) {
    let next = depth + 1;
    match value {
        IrValue::Const { .. }
        | IrValue::Local(_)
        | IrValue::Global(_)
        | IrValue::LocalRef { .. }
        | IrValue::FunctionRef { .. }
        | IrValue::Capture { .. } => {}
        // A lambda's body runs at the callback's call site, not here; only the
        // captured values are evaluated in this expression.
        IrValue::Closure { captures, .. } => {
            for capture in captures {
                scan_trap_call(capture, false, fallible, locals, globals, kinds, next, out);
            }
        }
        IrValue::Call { args, .. }
        | IrValue::CallResult { args, .. }
        | IrValue::Constructor { args, .. } => {
            for arg in args {
                scan_trap_call(arg, false, fallible, locals, globals, kinds, next, out);
            }
        }
        IrValue::UnionWrap { value, .. }
        | IrValue::UnionExtract { value, .. }
        | IrValue::ResultIsOk { value }
        | IrValue::ResultValue { value, .. }
        | IrValue::ResultError { value }
        | IrValue::Checked { value, .. }
        | IrValue::MemberAccess { target: value, .. } => {
            scan_trap_call(value, false, fallible, locals, globals, kinds, next, out)
        }
        IrValue::Unary { operand, .. } => {
            scan_trap_call(operand, true, fallible, locals, globals, kinds, next, out)
        }
        IrValue::WithUpdate {
            target, updates, ..
        } => {
            scan_trap_call(target, false, fallible, locals, globals, kinds, next, out);
            for update in updates {
                scan_trap_call(
                    &update.value,
                    false,
                    fallible,
                    locals,
                    globals,
                    kinds,
                    next,
                    out,
                );
            }
        }
        IrValue::ListLiteral { values, .. } | IrValue::SetLiteral { values, .. } => {
            for value in values {
                scan_trap_call(value, false, fallible, locals, globals, kinds, next, out);
            }
        }
        IrValue::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                scan_trap_call(key, false, fallible, locals, globals, kinds, next, out);
                scan_trap_call(value, false, fallible, locals, globals, kinds, next, out);
            }
        }
        IrValue::Binary {
            op, left, right, ..
        } => {
            scan_trap_call(left, true, fallible, locals, globals, kinds, next, out);
            if !is_short_circuit_operator(*op) {
                scan_trap_call(right, true, fallible, locals, globals, kinds, next, out);
            }
        }
    }
}

/// Whether `value`'s own node is one the desugar lifts out of the scrutinee: a
/// call, or (bug-471) an operator that can raise while the expression is
/// evaluated. Both walks below index exactly these nodes, in evaluation order,
/// so a node's position means the same thing to each.
///
/// `Some(true)` = lifted **and checked** (its error routes to the handler);
/// `Some(false)` = lifted but not checked — an infallible call kept in place
/// only so a later lift cannot reorder it past this one; `None` = not indexed.
pub(super) fn trap_hoist_kind(
    value: &IrValue,
    // bug-798: whether `value` is an operand of an arithmetic or comparison
    // operator. An operator operand is not an observation boundary, so a `Float`
    // operator whose only failure is observation is not lifted there: it stays in
    // the tree it belongs to (which also keeps `a * b ± c` whole for contraction).
    under_operator: bool,
    fallible: &Fallibility,
    locals: &HashMap<String, ParameterType>,
    globals: &HashMap<String, ParameterType>,
    // bug-679: the declared-kind oracle, so `toInt(<enum>)` is seen as the
    // infallible overload it is and stays a plain `Call`.
    kinds: &dyn builtins::TypeKinds,
) -> Option<bool> {
    match value {
        IrValue::Call { target, args, .. } => Some(fallible.call_is_fallible(
            target,
            &ir_call_arg_types(target, args, locals, globals),
            kinds,
        )),
        // The spelling of a negative literal, which cannot raise — see
        // `fallible::is_total_literal_negation` for why, and why `Byte` is not
        // exempt.
        IrValue::Unary {
            op, type_, operand, ..
        } if matches!(operand.as_ref(), IrValue::Const { .. })
            && is_total_literal_negation(*op, type_) =>
        {
            None
        }
        // plan-165-C: a short-circuit `AND`/`OR` is lifted as one conditional step
        // when its right operand holds anything that must stay conditional — a
        // call (an effect that must neither run when unselected nor move past a
        // later lift) or a raise site. It is checked when the right operand can
        // fail. The right operand's own nodes are indexed by their own scan when
        // the step is built, never in the enclosing expression's index.
        IrValue::Binary { op, right, .. } if is_short_circuit_operator(*op) => {
            let mut inner = Vec::new();
            scan_trap_call(right, true, fallible, locals, globals, kinds, 0, &mut inner);
            (!inner.is_empty()).then(|| inner.contains(&true))
        }
        // A raising operator is always checked: unlike a call there is no
        // declaration to prove it total, and `operator_can_raise` is already the
        // conservative side of that question.
        IrValue::Binary {
            op,
            type_,
            left,
            right,
            ..
        } => ((operator_can_raise(*op, type_)
            && !(under_operator && float_operator_raises_only_when_observed(*op, type_)))
            || comparison_can_raise(
                *op,
                &ir_operand_type(left, locals, globals),
                &ir_operand_type(right, locals, globals),
            ))
        .then_some(true),
        IrValue::Unary { op, type_, .. } => (unary_operator_can_raise(*op, type_)
            && !(under_operator && float_unary_raises_only_when_observed(*op, type_)))
        .then_some(true),
        _ => None,
    }
}

/// [`scan_trap_operands`] plus `value`'s own node, so lifted nodes are recorded
/// in evaluation order (operands first, then the node they feed).
fn scan_trap_call(
    value: &IrValue,
    under_operator: bool,
    fallible: &Fallibility,
    locals: &HashMap<String, ParameterType>,
    globals: &HashMap<String, ParameterType>,
    // bug-679: the declared-kind oracle, so `toInt(<enum>)` is seen as the
    // infallible overload it is and stays a plain `Call`.
    kinds: &dyn builtins::TypeKinds,
    depth: usize,
    out: &mut Vec<bool>,
) {
    if depth > TRAP_SCRUTINEE_MAX_DEPTH {
        return;
    }
    scan_trap_operands(value, fallible, locals, globals, kinds, depth, out);
    if let Some(checked) = trap_hoist_kind(value, under_operator, fallible, locals, globals, kinds)
    {
        out.push(checked);
    }
}

/// The static types of a lowered call's arguments, for the overload-aware half of
/// the fallibility census (bug-486).
///
/// Every `IrValue` that carries a `type_` answers with it; a `Local` is looked up
/// in the enclosing function's map (which is where a hoisted `$trap_arg*` and
/// every source local live) and a `Global` in the top-level binding map —
/// `IrValue::Global` is a bare name with no `type_` on the node, so without
/// `globals` a `LET` at file scope typed `Unknown` and its byte decode was hoisted
/// UNCHECKED, re-entering the bug this fixes. Anything else answers `Unknown`,
/// which lands the census on its name-keyed verdict — the answer it gave before
/// argument types reached it.
///
/// Gated on `target` so the walk only pays for the names whose verdict can turn
/// on an argument type; every other callee is decided by name alone.
pub(super) fn ir_call_arg_types(
    target: &str,
    args: &[IrValue],
    locals: &HashMap<String, ParameterType>,
    globals: &HashMap<String, ParameterType>,
) -> Vec<ParameterType> {
    if !builtins::inline_builtin_fallibility_depends_on_args(target) {
        return Vec::new();
    }
    args.iter()
        .map(|arg| ir_operand_type(arg, locals, globals))
        .collect()
}

/// The type of a call argument or operator operand, `Unknown` when unproven.
///
/// A bare name — the two node kinds `annotated_parameter_type` answers `None`
/// for — is typed from its binding environment. Every other node carries its own
/// type. Delegated rather than re-listed: a hand-written copy of that match is a
/// second list to keep in step, and the first draft of this one had already
/// dropped `MemberAccess` — which is the documented real-world idiom,
/// `toString(resp.body)`.
fn ir_operand_type(
    value: &IrValue,
    locals: &HashMap<String, ParameterType>,
    globals: &HashMap<String, ParameterType>,
) -> ParameterType {
    match value {
        IrValue::Local(name) => locals.get(name).cloned(),
        IrValue::Global(name) => globals.get(name).cloned(),
        other => other.annotated_parameter_type(),
    }
    .unwrap_or(ParameterType::Unknown)
}

/// The mutating twin of [`scan_trap_operands`]: walks the same nodes in the same
/// order and replaces the first `limit` call nodes with reads of the locals they
/// are lifted into.
#[allow(clippy::too_many_arguments)]
fn rewrite_trap_operands(
    value: &mut IrValue,
    fallible: &[bool],
    limit: usize,
    index: &mut usize,
    depth: usize,
    hoists: &mut Vec<TrapHoist>,
    locals: &mut HashMap<String, ParameterType>,
    context: &mut LowerContext<'_>,
) {
    let next = depth + 1;
    match value {
        IrValue::Const { .. }
        | IrValue::Local(_)
        | IrValue::Global(_)
        | IrValue::LocalRef { .. }
        | IrValue::FunctionRef { .. }
        | IrValue::Capture { .. } => {}
        IrValue::Closure { captures, .. } => {
            for capture in captures {
                rewrite_trap_call(
                    capture, false, fallible, limit, index, next, hoists, locals, context,
                );
            }
        }
        IrValue::Call { args, .. }
        | IrValue::CallResult { args, .. }
        | IrValue::Constructor { args, .. } => {
            for arg in args {
                rewrite_trap_call(
                    arg, false, fallible, limit, index, next, hoists, locals, context,
                );
            }
        }
        IrValue::UnionWrap { value, .. }
        | IrValue::UnionExtract { value, .. }
        | IrValue::ResultIsOk { value }
        | IrValue::ResultValue { value, .. }
        | IrValue::ResultError { value }
        | IrValue::Checked { value, .. }
        | IrValue::MemberAccess { target: value, .. } => rewrite_trap_call(
            value, false, fallible, limit, index, next, hoists, locals, context,
        ),
        IrValue::Unary { operand, .. } => rewrite_trap_call(
            operand, true, fallible, limit, index, next, hoists, locals, context,
        ),
        IrValue::WithUpdate {
            target, updates, ..
        } => {
            rewrite_trap_call(
                target, false, fallible, limit, index, next, hoists, locals, context,
            );
            for update in updates {
                rewrite_trap_call(
                    &mut update.value,
                    false,
                    fallible,
                    limit,
                    index,
                    next,
                    hoists,
                    locals,
                    context,
                );
            }
        }
        IrValue::ListLiteral { values, .. } | IrValue::SetLiteral { values, .. } => {
            for value in values {
                rewrite_trap_call(
                    value, false, fallible, limit, index, next, hoists, locals, context,
                );
            }
        }
        IrValue::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                rewrite_trap_call(
                    key, false, fallible, limit, index, next, hoists, locals, context,
                );
                rewrite_trap_call(
                    value, false, fallible, limit, index, next, hoists, locals, context,
                );
            }
        }
        IrValue::Binary {
            op, left, right, ..
        } => {
            let short_circuit = is_short_circuit_operator(*op);
            rewrite_trap_call(
                left, true, fallible, limit, index, next, hoists, locals, context,
            );
            if !short_circuit {
                rewrite_trap_call(
                    right, true, fallible, limit, index, next, hoists, locals, context,
                );
            }
        }
    }
}

/// [`rewrite_trap_operands`] plus `value`'s own node, lifting it when its
/// evaluation-order index is within `limit`.
#[allow(clippy::too_many_arguments)]
fn rewrite_trap_call(
    value: &mut IrValue,
    under_operator: bool,
    fallible: &[bool],
    limit: usize,
    index: &mut usize,
    depth: usize,
    hoists: &mut Vec<TrapHoist>,
    locals: &mut HashMap<String, ParameterType>,
    context: &mut LowerContext<'_>,
) {
    if depth > TRAP_SCRUTINEE_MAX_DEPTH {
        return;
    }
    rewrite_trap_operands(
        value, fallible, limit, index, depth, hoists, locals, context,
    );
    if !matches!(
        value,
        IrValue::Call { .. } | IrValue::Binary { .. } | IrValue::Unary { .. }
    ) {
        return;
    }
    // The scan's fallibility verdict is recomputed here rather than read off
    // `fallible[position]`, because a non-raising operator is not indexed at all
    // and must not consume a position.
    let Some(checked) = trap_hoist_kind(
        value,
        under_operator,
        context.fallible,
        locals,
        &context.binding_types,
        context.type_index,
    ) else {
        return;
    };
    assert_eq!(
        fallible.get(*index).copied(),
        Some(checked),
        "the scan and the rewrite disagree on which nodes are lifted"
    );
    let position = *index;
    *index += 1;
    if position >= limit {
        return;
    }
    if is_conditional_root(value) {
        let IrValue::Binary {
            op,
            left,
            right,
            loc,
            ..
        } = std::mem::replace(value, IrValue::Local(String::new()))
        else {
            unreachable!("is_conditional_root matched a Binary");
        };
        {
            // plan-165-C: the left operand was rewritten above as part of the
            // enclosing expression; the right one is lifted on its own, up to its
            // last fallible node, so its lifts run inside the selected branch.
            let mut right = *right;
            let mut inner = Vec::new();
            scan_trap_call(
                &right,
                true,
                context.fallible,
                locals,
                &context.binding_types,
                context.type_index,
                0,
                &mut inner,
            );
            let right_limit = inner.iter().rposition(|f| *f).map_or(0, |last| last + 1);
            let mut right_hoists = Vec::new();
            let mut right_index = 0;
            rewrite_trap_call(
                &mut right,
                true,
                &inner,
                right_limit,
                &mut right_index,
                0,
                &mut right_hoists,
                locals,
                context,
            );
            let name = make_temp_local_name(context, "trap_cond");
            locals.insert(name.clone(), ParameterType::Boolean);
            hoists.push(TrapHoist {
                name: name.clone(),
                type_: ParameterType::Boolean,
                value: *left,
                res: None,
                loc,
                conditional: Some(ConditionalStep {
                    op,
                    right: right_hoists,
                    right_value: right,
                }),
            });
            *value = IrValue::Local(name);
            return;
        }
    }
    let lifted = std::mem::replace(value, IrValue::Local(String::new()));
    let (type_, loc, value_node) = match lifted {
        IrValue::Call {
            target,
            args,
            type_,
            loc,
        } => {
            let node = if checked {
                IrValue::CallResult {
                    target,
                    args,
                    type_: type_.clone(),
                    loc,
                }
            } else {
                IrValue::Call {
                    target,
                    args,
                    type_: type_.clone(),
                    loc,
                }
            };
            (type_, loc, node)
        }
        // bug-471: a raising operator has no callee whose error return could be
        // turned into a `Result`, so it is wrapped in a `Checked` — "evaluate
        // this with its domain-error exits captured". Its own operands have
        // already been rewritten above, so every call inside it is a `Local`
        // read of an earlier lift and the `Checked` wraps pure arithmetic.
        operator => {
            let type_ = operator
                .annotated_parameter_type()
                .unwrap_or(ParameterType::Unknown);
            let loc = match &operator {
                IrValue::Binary { loc, .. } | IrValue::Unary { loc, .. } => *loc,
                _ => unreachable!("only a Binary/Unary node reaches here"),
            };
            // bug-798: an operator operand is not an observation boundary, so a
            // `Float` capture there (the only ones lifted under an operator are
            // `MOD`/`^`, for their domain errors) must not check finiteness —
            // the enclosing tree's boundary does that, as it does untrapped.
            let observe = !(under_operator && matches!(type_, ParameterType::Float));
            (
                type_.clone(),
                loc,
                IrValue::Checked {
                    type_,
                    value: Box::new(operator),
                    observe,
                },
            )
        }
    };
    let name = make_temp_local_name(context, "trap_arg");
    locals.insert(name.clone(), type_.clone());
    let res = if checked {
        let res = make_temp_local_name(context, "trap_res");
        locals.insert(res.clone(), ParameterType::result_of(type_.clone()));
        Some(res)
    } else {
        None
    };
    hoists.push(TrapHoist {
        name: name.clone(),
        type_,
        value: value_node,
        res,
        conditional: None,
        loc,
    });
    *value = IrValue::Local(name);
}
