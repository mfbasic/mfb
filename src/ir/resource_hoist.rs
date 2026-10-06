//! bug-695: hoist a resource produced straight into a container.
//!
//! Spec §15.6: "A resource element placed into a collection may be any pointer to
//! the resource — a named `RES` binding or a temporary alike." The escape analysis
//! ([`super::resource_escape`]) and everything downstream of it float only NAMED
//! `RES` bindings into a container's owned-list, so a producer call written straight
//! into an element position — `pool = collections::append(pool, fs::openFile(p))`,
//! `H[t := thread::start(work, 5)]` — had no close obligation anywhere: the handle
//! was never closed, and a thread never cancelled.
//!
//! This pass gives such a temporary the name the rest of the pipeline needs. Before
//! a function is lowered (and before its escape analysis runs), each producer call
//! in an element position whose type is a resource is moved into a synthesized
//! `RES $res_tmpN AS T = <call>` placed immediately before the statement, and the
//! element becomes `$res_tmpN`. From there the resource is exactly the named-binding
//! case §15.6 already handles: owned by the outermost scope that touches it.
//!
//! **Evaluation order is never changed.** A statement's element positions are
//! walked in the order they evaluate, and hoisting stops at the first earlier
//! expression that could have an effect and was not itself hoisted: moving a later
//! call ahead of it would reorder the two. What is not hoisted keeps today's
//! behavior. So does anything whose type the lowering cannot name, or that is not
//! known to be a resource.
//!
//! The element positions are the ones [`super::resource_escape`] scans: list and
//! map-literal values, record-constructor arguments, `WITH` updates, and the
//! element arguments of the `collections::` insertion builtins.

use std::collections::HashMap;

use super::lower::{expression_type, make_temp_local_name, LowerContext};
use super::resource_escape::{call_arg_expr_mut, is_insertion_builtin};
use crate::hir::{HirConstructorArg, HirExpression, HirFunction, HirStatement};
use crate::types::ParameterType;

/// `function` with every hoistable resource temporary named (see the module doc).
pub(super) fn hoist_resource_temporaries(
    function: &HirFunction,
    context: &mut LowerContext<'_>,
) -> HirFunction {
    let mut function = function.clone();
    let mut locals: HashMap<String, ParameterType> = function
        .params
        .iter()
        .map(|param| (param.name.clone(), param.type_.clone()))
        .collect();
    function.body = hoist_block(std::mem::take(&mut function.body), &mut locals, context);
    if let Some(trap) = &mut function.trap {
        let mut trap_locals = locals.clone();
        trap.body = hoist_block(std::mem::take(&mut trap.body), &mut trap_locals, context);
    }
    function
}

fn hoist_block(
    body: Vec<HirStatement>,
    locals: &mut HashMap<String, ParameterType>,
    context: &mut LowerContext<'_>,
) -> Vec<HirStatement> {
    let mut out = Vec::with_capacity(body.len());
    for mut statement in body {
        let mut hoisted = Vec::new();
        let line = statement_line(&statement);
        match &mut statement {
            HirStatement::Let {
                value: Some(value), ..
            }
            | HirStatement::Assign { value, .. }
            | HirStatement::Return {
                value: Some(value), ..
            } => {
                let mut ordered = true;
                hoist_container(value, &mut hoisted, &mut ordered, locals, context, line);
            }
            HirStatement::If {
                then_body,
                else_body,
                ..
            } => {
                *then_body = hoist_block(std::mem::take(then_body), &mut locals.clone(), context);
                *else_body = hoist_block(std::mem::take(else_body), &mut locals.clone(), context);
            }
            HirStatement::Match { cases, .. } => {
                for case in cases {
                    case.body =
                        hoist_block(std::mem::take(&mut case.body), &mut locals.clone(), context);
                }
            }
            HirStatement::For { body, .. }
            | HirStatement::ForEach { body, .. }
            | HirStatement::While { body, .. }
            | HirStatement::DoUntil { body, .. } => {
                *body = hoist_block(std::mem::take(body), &mut locals.clone(), context);
            }
            _ => {}
        }
        for binding in &hoisted {
            if let HirStatement::Let { name, type_, .. } = binding {
                locals.insert(name.clone(), type_.clone());
            }
        }
        if let HirStatement::Let {
            name,
            type_,
            explicit_type,
            value,
            ..
        } = &statement
        {
            let known = if *explicit_type {
                Some(type_.clone())
            } else {
                value
                    .as_ref()
                    .and_then(|value| expression_type(value, locals, context))
            };
            if let Some(known) = known {
                locals.insert(name.clone(), known);
            }
        }
        out.extend(hoisted);
        out.push(statement);
    }
    out
}

/// Walk `expr` as a container expression (the shapes
/// `resource_escape::scan_collection_expr` routes), hoisting each element that is a
/// resource producer while `ordered` still holds.
fn hoist_container(
    expr: &mut HirExpression,
    hoisted: &mut Vec<HirStatement>,
    ordered: &mut bool,
    locals: &HashMap<String, ParameterType>,
    context: &mut LowerContext<'_>,
    line: usize,
) {
    match expr {
        HirExpression::ListLiteral(values) => {
            for value in values {
                hoist_element(value, hoisted, ordered, locals, context, line);
            }
        }
        HirExpression::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                note_effects(key, ordered);
                hoist_element(value, hoisted, ordered, locals, context, line);
            }
        }
        HirExpression::Constructor { arguments, .. } => {
            for argument in arguments {
                let value = match argument {
                    HirConstructorArg::Positional(value) => value,
                    HirConstructorArg::Named { value, .. } => value,
                };
                hoist_element(value, hoisted, ordered, locals, context, line);
            }
        }
        HirExpression::WithUpdate { target, updates } => {
            hoist_container(target, hoisted, ordered, locals, context, line);
            for update in updates {
                hoist_element(&mut update.value, hoisted, ordered, locals, context, line);
            }
        }
        HirExpression::Call {
            callee, arguments, ..
        } if is_insertion_builtin(callee) => {
            for (index, arg) in arguments.iter_mut().enumerate() {
                let value = call_arg_expr_mut(arg);
                if index == 0 {
                    hoist_container(value, hoisted, ordered, locals, context, line);
                } else {
                    hoist_element(value, hoisted, ordered, locals, context, line);
                }
            }
        }
        other => note_effects(other, ordered),
    }
}

/// One element position: a resource producer is hoisted (while `ordered` holds);
/// a nested container is walked; anything else is only checked for effects.
fn hoist_element(
    expr: &mut HirExpression,
    hoisted: &mut Vec<HirStatement>,
    ordered: &mut bool,
    locals: &HashMap<String, ParameterType>,
    context: &mut LowerContext<'_>,
    line: usize,
) {
    if matches!(expr, HirExpression::Call { callee, .. } if !is_insertion_builtin(callee)) {
        // bug-643: a call that hands back a pointer to a resource another binding
        // already owns — `collections::get`/`getOr` of a `RES` element, a
        // `tcp`/`udp`/`tls::poll` borrow — is no producer. Naming it would make the
        // temporary an owner, float it into the container, and close the caller's
        // handle when the container's scope exits (and, for a record element such
        // as `zip`'s `Pair`, fail to build a drop for it). It stays in place.
        let borrowed = matches!(
            expr,
            HirExpression::Call { callee, .. }
                if crate::codegen::engine::builder::CodeBuilder::target_returns_borrowed_resource(callee)
        );
        if *ordered && !borrowed {
            if let Some(resource) = expression_type(expr, locals, context)
                .and_then(|type_| resource_binding_type(&type_, context))
            {
                let name = make_temp_local_name(context, "res_tmp");
                let (base, state) = resource.split_state();
                let value = std::mem::replace(expr, HirExpression::Identifier(name.clone()));
                hoisted.push(HirStatement::Let {
                    mutable: false,
                    resource: true,
                    state_type: state,
                    name,
                    type_: base,
                    explicit_type: true,
                    value: Some(value),
                    line,
                });
                return;
            }
        }
        *ordered = false;
        return;
    }
    hoist_container(expr, hoisted, ordered, locals, context, line);
}

/// The type a synthesized `RES` binding of a value of `type_` declares, when
/// `type_` is a resource: a built-in (either thread view included), a project
/// `RESOURCE`, or a `RES`-returning function's result.
fn resource_binding_type(
    type_: &ParameterType,
    context: &LowerContext<'_>,
) -> Option<ParameterType> {
    let bare = match type_ {
        ParameterType::Res(inner) => (**inner).clone(),
        other => other.clone(),
    };
    let base = bare.without_state();
    let is_resource =
        crate::codegen::resource::is_builtin_resource(&base) || context.is_declared_resource(&base);
    is_resource.then_some(bare)
}

/// Whether evaluating `expr` could have an effect this pass does not order: a call,
/// or anything containing one. Once one is seen, no later element is hoisted.
fn note_effects(expr: &HirExpression, ordered: &mut bool) {
    if !is_effect_free(expr) {
        *ordered = false;
    }
}

fn is_effect_free(expr: &HirExpression) -> bool {
    match expr {
        HirExpression::String(_)
        | HirExpression::Number(_)
        | HirExpression::Scalar(_)
        | HirExpression::Boolean(_)
        | HirExpression::Identifier(_) => true,
        HirExpression::MemberAccess { target, .. } => is_effect_free(target),
        HirExpression::ListLiteral(values) => values.iter().all(is_effect_free),
        _ => false,
    }
}

fn statement_line(statement: &HirStatement) -> usize {
    match statement {
        HirStatement::Let { line, .. }
        | HirStatement::Assign { line, .. }
        | HirStatement::Return { line, .. } => *line,
        _ => 0,
    }
}
