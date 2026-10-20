//! plan-176-N: IR lowering of folding a loop's string concatenation into a builder. The
//! lowering context and entry points live in `ir/lower/mod.rs`.

use super::*;

/// plan-121-G: a reducer whose whole body is `RETURN <acc> & <rhs>`, recognized
/// by shape so `collections::reduce` can be rewritten into the loop it is sugar
/// for instead of calling it N times.
///
/// Only the MATCHED shape is carried, never an arbitrary body — the lowering
/// context has no business holding HIR statements, and a narrow table is a
/// narrow blast radius. The accumulator parameter is *checked* (it must be the
/// concat's left operand, and the right operand must not name it) but not
/// carried: the rewrite replaces it with a generated local, so its source name
/// is never needed again.
#[derive(Debug, Clone)]
pub(super) struct ConcatReducer {
    /// The element parameter, substituted by the loop variable at the call site.
    pub(super) item_param: String,
    /// The concat's right operand, i.e. what each step appends.
    pub(super) rhs: HirExpression,
}

/// Does `expr` mention `name`?
///
/// Used to reject a reducer that READS the accumulator for anything other than
/// being the left operand — `acc & toString(len(acc)) & x` observes `acc`'s
/// value mid-fold, so rewriting it into an append would change what it observes.
fn hir_mentions(expr: &HirExpression, name: &str) -> bool {
    match expr {
        HirExpression::Identifier(id) => id == name,
        HirExpression::String(_)
        | HirExpression::Number(_)
        | HirExpression::Scalar(_)
        | HirExpression::Boolean(_) => false,
        HirExpression::Binary { left, right, .. } => {
            hir_mentions(left, name) || hir_mentions(right, name)
        }
        HirExpression::Unary { operand, .. } => hir_mentions(operand, name),
        HirExpression::Call { arguments, .. } => arguments.iter().any(|a| match a {
            HirCallArg::Positional(e) => hir_mentions(e, name),
            HirCallArg::Named { value, .. } => hir_mentions(value, name),
        }),
        // Anything whose shape this function does not enumerate is treated as
        // MENTIONING the name. That is the safe direction: an unrecognized
        // expression declines the rewrite, where a `false` default would silently
        // admit a shape nobody checked. A missed decline miscompiles; a spurious
        // one is only slow.
        _ => true,
    }
}

/// Recognize `FUNC f(acc AS String, x AS T) AS String / RETURN acc & <rhs>`.
///
/// Every condition here is a decline, and each is load-bearing:
/// * exactly one statement — anything before the `RETURN` could have effects;
/// * the operator is `&` and the LEFT operand is the accumulator **identifier**
///   (`x & acc` prepends, which an append cannot express);
/// * the right operand does not mention the accumulator (see `hir_mentions`);
/// * the accumulator's declared type is `String`, which is the only type
///   `try_inplace_concat_assign` has a grown buffer for.
fn concat_reducer_shape(function: &HirFunction) -> Option<ConcatReducer> {
    if function.params.len() != 2 {
        return None;
    }
    let acc = &function.params[0];
    let item = &function.params[1];
    if acc.type_ != ParameterType::String {
        return None;
    }
    let [HirStatement::Return {
        value:
            Some(HirExpression::Binary {
                left,
                operator: BinaryOp::Concat,
                right,
                ..
            }),
        ..
    }] = function.body.as_slice()
    else {
        return None;
    };
    let HirExpression::Identifier(left_name) = left.as_ref() else {
        return None;
    };
    if left_name != &acc.name || hir_mentions(right, &acc.name) {
        return None;
    }
    Some(ConcatReducer {
        item_param: item.name.clone(),
        rhs: right.as_ref().clone(),
    })
}

/// The project's `ConcatReducer`s, keyed by function name.
pub(super) fn concat_reducers(hir: &HirProject) -> HashMap<String, ConcatReducer> {
    let mut found = HashMap::new();
    for file in &hir.files {
        for item in &file.items {
            if let HirItem::Function(function) = item {
                if let Some(shape) = concat_reducer_shape(function) {
                    found.insert(function.name.clone(), shape);
                }
            }
        }
    }
    found
}

/// Is `value` free of anything observable — no call, no closure?
///
/// The fold is hoisted to the front of its statement, so this is what makes that
/// unobservable: if every node evaluated *before* the fold is effect-free, the
/// fold is the FIRST effectful thing in the statement and moving it to the front
/// reorders nothing. Nodes after it keep their relative order.
///
/// Unenumerated shapes answer `false` (i.e. "may have effects"), which declines
/// the rewrite. That is the safe direction.
fn ir_value_is_effect_free(value: &IrValue) -> bool {
    match value {
        IrValue::Const { .. }
        | IrValue::Local(_)
        | IrValue::Global(_)
        | IrValue::LocalRef { .. }
        | IrValue::FunctionRef { .. }
        | IrValue::Capture { .. } => true,
        IrValue::Unary { operand, .. } => ir_value_is_effect_free(operand),
        IrValue::Binary { left, right, .. } => {
            ir_value_is_effect_free(left) && ir_value_is_effect_free(right)
        }
        IrValue::MemberAccess { target, .. } => ir_value_is_effect_free(target),
        _ => false,
    }
}

/// Rename `from` to `to` throughout a HIR expression.
///
/// The reducer's stored right-hand side names the reducer's own element
/// parameter; the generated loop binds a FRESH name (so it cannot collide with a
/// local already in scope at the call site), and this substitutes one for the
/// other before the body is lowered.
fn hir_rename(expr: &mut HirExpression, from: &str, to: &str) {
    match expr {
        HirExpression::Identifier(id) => {
            if id == from {
                *id = to.to_string();
            }
        }
        HirExpression::Binary { left, right, .. } => {
            hir_rename(left, from, to);
            hir_rename(right, from, to);
        }
        HirExpression::Unary { operand, .. } => hir_rename(operand, from, to),
        HirExpression::Call { arguments, .. } => {
            for arg in arguments {
                match arg {
                    HirCallArg::Positional(e) => hir_rename(e, from, to),
                    HirCallArg::Named { value, .. } => hir_rename(value, from, to),
                }
            }
        }
        _ => {}
    }
}

/// A `collections::reduce`/`reduceRight` call this pass will rewrite.
struct FoldSite {
    /// `true` for `reduceRight`, which folds the list in reverse.
    reverse: bool,
    /// The list being folded.
    list: IrValue,
    /// The fold's seed.
    seed: IrValue,
    /// The recognized reducer's shape.
    reducer: ConcatReducer,
}

/// If `value` contains a rewritable fold as its FIRST effectful node, take it,
/// replacing the call with a read of `tmp`.
///
/// Returns `None` — declining — whenever anything is not exactly right. Every
/// decline is correct: the fold keeps today's lowering, which is slow and known
/// good.
fn take_concat_fold(
    value: &mut IrValue,
    tmp: &str,
    context: &LowerContext<'_>,
) -> Option<FoldSite> {
    // Effect-free operands cannot contain a call at all, so nothing to find and
    // nothing that would be reordered by hoisting past them.
    if ir_value_is_effect_free(value) {
        return None;
    }
    if let IrValue::Call { target, args, .. } = value {
        let reverse = match target.as_str() {
            "collections.reduce" => false,
            "collections.reduceRight" => true,
            _ => {
                // A different call. Its ARGUMENTS are evaluated before the call
                // itself, so a fold inside the first effectful argument is still
                // the first effectful node overall and is hoistable —
                // `len(reduce(...))`, which is exactly the benchmark's shape, is
                // this case. Only once the arguments are exhausted does this
                // call's own effect happen, and a fold after that is not
                // hoistable.
                for arg in args.iter_mut() {
                    if !ir_value_is_effect_free(arg) {
                        return take_concat_fold(arg, tmp, context);
                    }
                }
                return None;
            }
        };
        if args.len() != 3 {
            return None;
        }
        // The accumulator must be a String: that is the only type
        // `try_inplace_concat_assign` has a grown buffer for.
        if args[1].annotated_parameter_type() != Some(ParameterType::String) {
            return None;
        }
        // The reducer must be a statically named function whose body this pass
        // recognized. A closure declines: substituting a body that reads a
        // capture is unsound.
        let IrValue::FunctionRef { name, .. } = &args[2] else {
            return None;
        };
        let reducer = context.concat_reducers.get(name)?.clone();
        // The list and seed are evaluated before the fold runs; if either has
        // effects, hoisting the whole call would reorder them relative to
        // anything else. Requiring them effect-free keeps that simple and true.
        if !ir_value_is_effect_free(&args[0]) || !ir_value_is_effect_free(&args[1]) {
            return None;
        }
        let list = args[0].clone();
        let seed = args[1].clone();
        *value = IrValue::Local(tmp.to_string());
        return Some(FoldSite {
            reverse,
            list,
            seed,
            reducer,
        });
    }
    // Descend in EVALUATION ORDER, stopping at the first operand that is not
    // effect-free: that operand contains the first effectful node, so the fold
    // must be inside it or it is not hoistable.
    match value {
        IrValue::Unary { operand, .. } => take_concat_fold(operand, tmp, context),
        IrValue::Binary { left, right, .. } => {
            if !ir_value_is_effect_free(left) {
                take_concat_fold(left, tmp, context)
            } else {
                take_concat_fold(right, tmp, context)
            }
        }
        IrValue::MemberAccess { target, .. } => take_concat_fold(target, tmp, context),
        _ => None,
    }
}

/// Rewrite each statement's rewritable fold into a bind plus a loop.
///
/// Only the ops this pass produces directly are scanned. Ops carrying nested
/// bodies (`If`, `While`, `ForEach`, `Trap`, …) are left alone here because
/// their inner statements go through `lower_statement` themselves and are
/// rewritten there.
pub(super) fn rewrite_concat_folds(
    ops: Vec<IrOp>,
    locals: &mut HashMap<String, ParameterType>,
    context: &mut LowerContext<'_>,
) -> Vec<IrOp> {
    if context.concat_reducers.is_empty() {
        return ops;
    }
    let mut out = Vec::with_capacity(ops.len());
    for mut op in ops {
        let loc = op.loc();
        let value = match &mut op {
            IrOp::Bind {
                value: Some(value), ..
            }
            | IrOp::Assign { value, .. }
            | IrOp::AssignGlobal { value, .. }
            | IrOp::Eval { value, .. } => Some(value),
            IrOp::Return {
                value: Some(value), ..
            } => Some(value),
            _ => None,
        };
        let Some(value) = value else {
            out.push(op);
            continue;
        };
        let tmp = context.next_temp_name("fold");
        let Some(site) = take_concat_fold(value, &tmp, context) else {
            out.push(op);
            continue;
        };
        let item = context.next_temp_name("folditem");
        // The list's type. `IrValue::Local` carries no type of its own, so a
        // local is resolved through `locals` — without this the element type came
        // back `Unknown` and codegen refused the loop with "no storage class for
        // type 'Unknown'". DECLINE rather than emit an `Unknown`-typed loop: a
        // type this pass cannot name is a shape it has no business rewriting.
        let list_type = match &site.list {
            IrValue::Local(name) => locals.get(name).cloned(),
            other => other.annotated_parameter_type(),
        };
        let Some(element_type) = list_type
            .as_ref()
            .and_then(crate::codegen::engine::types::typed_list_element_type)
            .cloned()
            .filter(|t| *t != ParameterType::Unknown)
        else {
            out.push(op);
            continue;
        };

        // `MUT <tmp> AS String = <seed>`
        out.push(IrOp::Bind {
            mutable: true,
            name: tmp.clone(),
            type_: ParameterType::String,
            value: Some(site.seed),
            explicit_type: false,
            loc,
        });
        locals.insert(tmp.clone(), ParameterType::String);
        locals.insert(item.clone(), element_type.clone());

        // The reducer's right-hand side, with its element parameter renamed to
        // the generated loop variable, lowered in the caller's scope.
        let mut rhs = site.reducer.rhs.clone();
        hir_rename(&mut rhs, &site.reducer.item_param, &item);
        let rhs = lower_expression(&rhs, locals, context);

        // `<tmp> = <tmp> & <rhs>` -- the shape `try_inplace_concat_assign` matches.
        let step = IrOp::Assign {
            name: tmp.clone(),
            value: IrValue::Binary {
                op: BinaryOp::Concat,
                left: Box::new(IrValue::Local(tmp.clone())),
                right: Box::new(rhs),
                type_: ParameterType::String,
                loc,
            },
            loc,
        };

        if site.reverse {
            // `reduceRight` folds in reverse. Phase 1 established by test that a
            // self-concat reduceRight is STILL a left-append (`543210`), just fed
            // in the opposite order -- so the body is the same and only the
            // iteration direction differs. `ForEach` is forward-only, so this is
            // a counted loop with a negative step over `collections::get`.
            let idx = context.next_temp_name("foldidx");
            locals.insert(idx.clone(), ParameterType::Integer);
            let mut body_rhs = site.reducer.rhs.clone();
            hir_rename(&mut body_rhs, &site.reducer.item_param, &item);
            let get = IrValue::Call {
                target: "collections.get".to_string(),
                args: vec![site.list.clone(), IrValue::Local(idx.clone())],
                type_: element_type.clone(),
                loc,
            };
            let bind_item = IrOp::Bind {
                mutable: false,
                name: item.clone(),
                type_: element_type.clone(),
                value: Some(get),
                explicit_type: false,
                loc,
            };
            let len = IrValue::Call {
                target: "len".to_string(),
                args: vec![site.list.clone()],
                type_: ParameterType::Integer,
                loc,
            };
            out.push(IrOp::For {
                name: idx,
                type_: ParameterType::Integer,
                start: IrValue::Binary {
                    op: BinaryOp::Subtract,
                    left: Box::new(len),
                    right: Box::new(IrValue::Const {
                        type_: ParameterType::Integer,
                        value: "1".to_string(),
                    }),
                    type_: ParameterType::Integer,
                    loc,
                },
                end: IrValue::Const {
                    type_: ParameterType::Integer,
                    value: "0".to_string(),
                },
                step: IrValue::Const {
                    type_: ParameterType::Integer,
                    value: "-1".to_string(),
                },
                body: vec![bind_item, step],
                loc,
            });
        } else {
            out.push(IrOp::ForEach {
                name: item,
                type_: element_type,
                iterable: site.list,
                body: vec![step],
                loc,
            });
        }
        out.push(op);
    }
    out
}
