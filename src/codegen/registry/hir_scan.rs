//! plan-176-M: the HIR walk that collects the types and callees a project references.
//! `Registry` and `build()` live in `codegen/registry/mod.rs`.

use super::*;

/// [`item_types`] over HIR, where an annotation is a [`ParameterType`] and is noted by
/// its spelling.
pub(super) fn hir_item_types(
    item: &crate::hir::HirItem,
    out: &mut std::collections::HashSet<String>,
) {
    use crate::hir::HirItem;
    match item {
        HirItem::Function(f) => {
            hir_params_types(&f.params, out);
            note_type(&f.returns.to_string(), out);
            f.body.iter().for_each(|s| hir_stmt_types(s, out));
            if let Some(trap) = &f.trap {
                trap.body.iter().for_each(|s| hir_stmt_types(s, out));
            }
        }
        HirItem::Binding(b) => {
            note_type(&b.type_.to_string(), out);
            if let Some(value) = &b.value {
                hir_expr_types(value, out);
            }
        }
        HirItem::Type(decl) => {
            decl.fields
                .iter()
                .for_each(|f| note_type(&f.type_.to_string(), out));
            decl.variants
                .iter()
                .for_each(|v| note_type(&v.type_.to_string(), out));
            decl.includes
                .iter()
                .for_each(|i| note_type(&i.to_string(), out));
        }
        // HIR reuses the AST `TestingBlock` verbatim.
        HirItem::Testing(block) => block.groups.iter().for_each(|g| group_types(g, out)),
        _ => {}
    }
}

fn hir_params_types(params: &[crate::hir::HirParam], out: &mut std::collections::HashSet<String>) {
    for param in params {
        note_type(&param.type_.to_string(), out);
    }
}

fn hir_stmt_types(stmt: &crate::hir::HirStatement, out: &mut std::collections::HashSet<String>) {
    use crate::hir::{HirMatchPattern, HirStatement};
    match stmt {
        HirStatement::Let { type_, value, .. } => {
            note_type(&type_.to_string(), out);
            if let Some(value) = value {
                hir_expr_types(value, out);
            }
        }
        HirStatement::Return { value, .. } | HirStatement::Recover { value, .. } => {
            if let Some(value) = value {
                hir_expr_types(value, out);
            }
        }
        HirStatement::Fail { error, .. } => hir_expr_types(error, out),
        HirStatement::Assign { value, .. } | HirStatement::StateAssign { value, .. } => {
            hir_expr_types(value, out)
        }
        HirStatement::Expression { expression, .. } => hir_expr_types(expression, out),
        HirStatement::If {
            condition,
            then_body,
            else_body,
            ..
        } => {
            hir_expr_types(condition, out);
            then_body.iter().for_each(|s| hir_stmt_types(s, out));
            else_body.iter().for_each(|s| hir_stmt_types(s, out));
        }
        HirStatement::Match {
            expression, cases, ..
        } => {
            hir_expr_types(expression, out);
            for case in cases {
                if let HirMatchPattern::Union { type_, .. } = &case.pattern {
                    note_type(&type_.to_string(), out);
                }
                case.body.iter().for_each(|s| hir_stmt_types(s, out));
            }
        }
        HirStatement::For {
            start,
            end,
            step,
            body,
            ..
        } => {
            hir_expr_types(start, out);
            hir_expr_types(end, out);
            if let Some(step) = step {
                hir_expr_types(step, out);
            }
            body.iter().for_each(|s| hir_stmt_types(s, out));
        }
        HirStatement::ForEach { iterable, body, .. } => {
            hir_expr_types(iterable, out);
            body.iter().for_each(|s| hir_stmt_types(s, out));
        }
        HirStatement::While {
            condition, body, ..
        }
        | HirStatement::DoUntil {
            condition, body, ..
        } => {
            hir_expr_types(condition, out);
            body.iter().for_each(|s| hir_stmt_types(s, out));
        }
        HirStatement::Exit { .. }
        | HirStatement::Continue { .. }
        | HirStatement::Propagate { .. } => {}
    }
}

fn hir_expr_types(expr: &crate::hir::HirExpression, out: &mut std::collections::HashSet<String>) {
    use crate::hir::{HirCallArg, HirConstructorArg, HirExpression};
    match expr {
        HirExpression::Call { arguments, .. } => {
            for argument in arguments {
                match argument {
                    HirCallArg::Positional(v) | HirCallArg::Named { value: v, .. } => {
                        hir_expr_types(v, out)
                    }
                }
            }
        }
        HirExpression::Binary { left, right, .. } => {
            hir_expr_types(left, out);
            hir_expr_types(right, out);
        }
        HirExpression::Unary { operand, .. } => hir_expr_types(operand, out),
        HirExpression::Lambda { params, body, .. } => {
            hir_params_types(params, out);
            hir_expr_types(body, out);
        }
        HirExpression::Constructor { arguments, .. } => {
            for argument in arguments {
                match argument {
                    HirConstructorArg::Positional(v)
                    | HirConstructorArg::Named { value: v, .. } => hir_expr_types(v, out),
                }
            }
        }
        HirExpression::WithUpdate { target, updates } => {
            hir_expr_types(target, out);
            updates.iter().for_each(|u| hir_expr_types(&u.value, out));
        }
        HirExpression::ListLiteral(values) => values.iter().for_each(|v| hir_expr_types(v, out)),
        HirExpression::SetLiteral { elements, .. } => {
            elements.iter().for_each(|v| hir_expr_types(v, out))
        }
        HirExpression::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                hir_expr_types(key, out);
                hir_expr_types(value, out);
            }
        }
        HirExpression::MemberAccess { target, .. } => hir_expr_types(target, out),
        HirExpression::Trapped {
            expression,
            handler,
            ..
        } => {
            hir_expr_types(expression, out);
            handler.iter().for_each(|s| hir_stmt_types(s, out));
        }
        _ => {}
    }
}

pub(super) fn hir_item_callees(
    item: &crate::hir::HirItem,
    out: &mut std::collections::HashSet<String>,
    made: &mut std::collections::HashSet<String>,
) {
    use crate::hir::HirItem;
    match item {
        HirItem::Function(f) => f.body.iter().for_each(|s| hir_stmt_callees(s, out, made)),
        HirItem::Binding(b) => {
            if let Some(value) = &b.value {
                hir_expr_callees(value, out, made);
            }
        }
        // HIR reuses the AST `TestingBlock` verbatim.
        HirItem::Testing(block) => block
            .groups
            .iter()
            .for_each(|g| group_callees(g, out, made)),
        _ => {}
    }
}

fn hir_stmt_callees(
    stmt: &crate::hir::HirStatement,
    out: &mut std::collections::HashSet<String>,
    made: &mut std::collections::HashSet<String>,
) {
    use crate::hir::HirStatement;
    match stmt {
        HirStatement::Let { value, .. }
        | HirStatement::Return { value, .. }
        | HirStatement::Recover { value, .. } => {
            if let Some(value) = value {
                hir_expr_callees(value, out, made);
            }
        }
        HirStatement::Fail { error, .. } => hir_expr_callees(error, out, made),
        HirStatement::Assign { value, .. } | HirStatement::StateAssign { value, .. } => {
            hir_expr_callees(value, out, made)
        }
        HirStatement::Expression { expression, .. } => hir_expr_callees(expression, out, made),
        HirStatement::If {
            condition,
            then_body,
            else_body,
            ..
        } => {
            hir_expr_callees(condition, out, made);
            then_body
                .iter()
                .for_each(|s| hir_stmt_callees(s, out, made));
            else_body
                .iter()
                .for_each(|s| hir_stmt_callees(s, out, made));
        }
        HirStatement::Match {
            expression, cases, ..
        } => {
            hir_expr_callees(expression, out, made);
            for case in cases {
                case.body
                    .iter()
                    .for_each(|s| hir_stmt_callees(s, out, made));
            }
        }
        HirStatement::For {
            start,
            end,
            step,
            body,
            ..
        } => {
            hir_expr_callees(start, out, made);
            hir_expr_callees(end, out, made);
            if let Some(step) = step {
                hir_expr_callees(step, out, made);
            }
            body.iter().for_each(|s| hir_stmt_callees(s, out, made));
        }
        HirStatement::ForEach { iterable, body, .. } => {
            hir_expr_callees(iterable, out, made);
            body.iter().for_each(|s| hir_stmt_callees(s, out, made));
        }
        HirStatement::While {
            condition, body, ..
        }
        | HirStatement::DoUntil {
            condition, body, ..
        } => {
            hir_expr_callees(condition, out, made);
            body.iter().for_each(|s| hir_stmt_callees(s, out, made));
        }
        HirStatement::Exit { .. }
        | HirStatement::Continue { .. }
        | HirStatement::Propagate { .. } => {}
    }
}

fn hir_expr_callees(
    expr: &crate::hir::HirExpression,
    out: &mut std::collections::HashSet<String>,
    made: &mut std::collections::HashSet<String>,
) {
    use crate::hir::{HirCallArg, HirConstructorArg, HirExpression};
    match expr {
        HirExpression::Call {
            callee, arguments, ..
        } => {
            out.insert(short_callee(callee).to_string());
            for argument in arguments {
                match argument {
                    HirCallArg::Positional(v) | HirCallArg::Named { value: v, .. } => {
                        hir_expr_callees(v, out, made)
                    }
                }
            }
        }
        HirExpression::Binary { left, right, .. } => {
            hir_expr_callees(left, out, made);
            hir_expr_callees(right, out, made);
        }
        HirExpression::Unary { operand, .. } => hir_expr_callees(operand, out, made),
        HirExpression::Lambda { body, .. } => hir_expr_callees(body, out, made),
        HirExpression::Constructor { type_, arguments } => {
            made.insert(short_callee(&type_.name()).to_string());
            for argument in arguments {
                match argument {
                    HirConstructorArg::Positional(v)
                    | HirConstructorArg::Named { value: v, .. } => hir_expr_callees(v, out, made),
                }
            }
        }
        HirExpression::WithUpdate { target, updates } => {
            hir_expr_callees(target, out, made);
            updates
                .iter()
                .for_each(|u| hir_expr_callees(&u.value, out, made));
        }
        HirExpression::ListLiteral(values) => {
            values.iter().for_each(|v| hir_expr_callees(v, out, made))
        }
        HirExpression::SetLiteral { elements, .. } => {
            elements.iter().for_each(|v| hir_expr_callees(v, out, made))
        }
        HirExpression::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                hir_expr_callees(key, out, made);
                hir_expr_callees(value, out, made);
            }
        }
        HirExpression::MemberAccess { target, .. } => hir_expr_callees(target, out, made),
        HirExpression::Trapped {
            expression,
            handler,
            ..
        } => {
            hir_expr_callees(expression, out, made);
            handler.iter().for_each(|s| hir_stmt_callees(s, out, made));
        }
        HirExpression::String(_)
        | HirExpression::Number(_)
        | HirExpression::Scalar(_)
        | HirExpression::Boolean(_)
        | HirExpression::Identifier(_) => {}
    }
}
