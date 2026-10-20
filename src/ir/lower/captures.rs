//! plan-176-N: IR lowering of lambda captures. The lowering context and entry points live in
//! `ir/lower/mod.rs`.

use super::*;

/// bug-843: the names a `LAMBDA` captures from the enclosing function, in the order
/// lowering lays out its environment (`captured_locals`, then the assignment
/// target). `outer` is every local and parameter name the lambda can see.
///
/// The resource escape analysis asks this because a closure is a container in the
/// §15.6 sense: capturing a value copies its resource references into the
/// environment exactly as `collections::append` copies one into a `List`.
pub(crate) fn lambda_captured_names(
    params: &[crate::hir::HirParam],
    body: &HirExpression,
    assign_target: Option<&str>,
    outer: &HashSet<String>,
) -> Vec<String> {
    let outer_locals = outer
        .iter()
        .map(|name| (name.clone(), ParameterType::Unknown))
        .collect::<HashMap<_, _>>();
    let param_names = params
        .iter()
        .map(|param| param.name.clone())
        .collect::<HashSet<_>>();
    let mut names = captured_locals(body, &outer_locals, &param_names)
        .into_iter()
        .map(|capture| capture.name)
        .collect::<Vec<_>>();
    if let Some(target) = assign_target {
        if !param_names.contains(target)
            && outer.contains(target)
            && !names.iter().any(|name| name == target)
        {
            names.push(target.to_string());
        }
    }
    names
}

pub(super) fn captured_locals(
    expression: &HirExpression,
    outer_locals: &HashMap<String, ParameterType>,
    local_names: &HashSet<String>,
) -> Vec<CapturedLocal> {
    let mut captures = Vec::new();
    let mut seen = HashSet::new();
    collect_captured_locals(
        expression,
        outer_locals,
        local_names,
        &mut seen,
        &mut captures,
    );
    captures
}

fn collect_captured_locals(
    expression: &HirExpression,
    outer_locals: &HashMap<String, ParameterType>,
    local_names: &HashSet<String>,
    seen: &mut HashSet<String>,
    captures: &mut Vec<CapturedLocal>,
) {
    match expression {
        HirExpression::Identifier(name) => {
            if let Some(type_) = outer_locals.get(name) {
                if !local_names.contains(name) && seen.insert(name.clone()) {
                    captures.push(CapturedLocal {
                        name: name.clone(),
                        type_: type_.clone(),
                    });
                }
            }
        }
        HirExpression::Call {
            callee, arguments, ..
        } => {
            if let Some(type_) = outer_locals.get(callee) {
                if !local_names.contains(callee) && seen.insert(callee.clone()) {
                    captures.push(CapturedLocal {
                        name: callee.clone(),
                        type_: type_.clone(),
                    });
                }
            }
            for argument in arguments {
                collect_captured_locals(
                    call_arg_value(argument),
                    outer_locals,
                    local_names,
                    seen,
                    captures,
                );
            }
        }
        // A nested lambda's free variables are free in this one too: lowering the
        // nested lambda builds its environment from THIS lambda's locals, so every
        // outer binding it reads must be captured here first. Skipping its body left
        // `LAMBDA(i) -> LAMBDA(v) -> v * i + k` with no `k` in either environment
        // ("NIR local reference 'k' does not resolve"). Its own parameters shadow.
        HirExpression::Lambda {
            params,
            body,
            assign_target,
        } => {
            let mut inner_names = local_names.clone();
            inner_names.extend(params.iter().map(|param| param.name.clone()));
            if let Some(target) = assign_target {
                if !inner_names.contains(target) {
                    if let Some(type_) = outer_locals.get(target) {
                        if seen.insert(target.clone()) {
                            captures.push(CapturedLocal {
                                name: target.clone(),
                                type_: type_.clone(),
                            });
                        }
                    }
                }
            }
            collect_captured_locals(body, outer_locals, &inner_names, seen, captures);
        }
        HirExpression::Binary { left, right, .. } => {
            collect_captured_locals(left, outer_locals, local_names, seen, captures);
            collect_captured_locals(right, outer_locals, local_names, seen, captures);
        }
        HirExpression::Unary { operand, .. } => {
            collect_captured_locals(operand, outer_locals, local_names, seen, captures);
        }
        HirExpression::Constructor { arguments, .. } => {
            for argument in arguments {
                collect_captured_locals(
                    constructor_arg_value(argument),
                    outer_locals,
                    local_names,
                    seen,
                    captures,
                );
            }
        }
        HirExpression::ListLiteral(values) => {
            for value in values {
                collect_captured_locals(value, outer_locals, local_names, seen, captures);
            }
        }
        HirExpression::SetLiteral { elements, .. } => {
            for value in elements {
                collect_captured_locals(value, outer_locals, local_names, seen, captures);
            }
        }
        HirExpression::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                collect_captured_locals(key, outer_locals, local_names, seen, captures);
                collect_captured_locals(value, outer_locals, local_names, seen, captures);
            }
        }
        HirExpression::MemberAccess { target, .. } => {
            collect_captured_locals(target, outer_locals, local_names, seen, captures);
        }
        HirExpression::WithUpdate { target, updates } => {
            collect_captured_locals(target, outer_locals, local_names, seen, captures);
            for update in updates {
                collect_captured_locals(&update.value, outer_locals, local_names, seen, captures);
            }
        }
        HirExpression::Trapped { expression, .. } => {
            collect_captured_locals(expression, outer_locals, local_names, seen, captures);
        }
        HirExpression::String(_)
        | HirExpression::Number(_)
        | HirExpression::Scalar(_)
        | HirExpression::Boolean(_) => {}
    }
}

pub(super) fn literal_expression_type(expression: &HirExpression) -> Option<ParameterType> {
    match expression {
        HirExpression::String(_) => Some(ParameterType::String),
        HirExpression::Number(value) => Some(match numeric::classify_literal(value).1 {
            numeric::LiteralType::Integer => ParameterType::Integer,
            numeric::LiteralType::Float => ParameterType::Float,
            numeric::LiteralType::Fixed => ParameterType::Fixed,
            numeric::LiteralType::Money => ParameterType::Money,
        }),
        HirExpression::Scalar(_) => Some(ParameterType::named("Scalar")),
        HirExpression::Boolean(_) => Some(ParameterType::Boolean),
        HirExpression::Identifier(value) if value == "NOTHING" => Some(ParameterType::Nothing),
        _ => None,
    }
}
