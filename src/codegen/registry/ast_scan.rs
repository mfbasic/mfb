//! plan-176-M: the AST walk that collects the types and callees a project references.
//! `Registry` and `build()` live in `codegen/registry/mod.rs`.

use super::*;

/// The type annotations of one item, at any depth: its own, its parameters', its
/// fields', and those of every binding and lambda parameter in its body.
pub(super) fn item_types(item: &crate::ast::Item, out: &mut std::collections::HashSet<String>) {
    use crate::ast::Item;
    match item {
        Item::Function(f) => {
            params_types(&f.params, out);
            if let Some(returns) = &f.return_type {
                note_type(returns, out);
            }
            f.body.iter().for_each(|s| stmt_types(s, out));
            if let Some(trap) = &f.trap {
                trap.body.iter().for_each(|s| stmt_types(s, out));
            }
        }
        Item::Binding(b) => {
            if let Some(name) = &b.type_name {
                note_type(name, out);
            }
            if let Some(value) = &b.value {
                expr_types(value, out);
            }
        }
        Item::Type(decl) => {
            decl.fields
                .iter()
                .for_each(|f| note_type(&f.type_name, out));
            decl.variants.iter().for_each(|v| note_type(&v.name, out));
            decl.includes.iter().for_each(|i| note_type(i, out));
        }
        Item::Testing(block) => block.groups.iter().for_each(|g| group_types(g, out)),
        _ => {}
    }
}

fn params_types(params: &[crate::ast::Param], out: &mut std::collections::HashSet<String>) {
    for param in params {
        if let Some(name) = &param.type_name {
            note_type(name, out);
        }
        if let Some(default) = &param.default {
            expr_types(default, out);
        }
    }
}

pub(super) fn group_types(
    group: &crate::ast::TestGroup,
    out: &mut std::collections::HashSet<String>,
) {
    use crate::ast::TestGroupMember;
    for member in &group.members {
        match member {
            TestGroupMember::Case(case) => case.body.iter().for_each(|s| stmt_types(s, out)),
            TestGroupMember::Group(nested) => group_types(nested, out),
        }
    }
}

fn stmt_types(stmt: &crate::ast::Statement, out: &mut std::collections::HashSet<String>) {
    use crate::ast::{MatchPattern, Statement};
    match stmt {
        Statement::Let {
            type_name, value, ..
        } => {
            if let Some(name) = type_name {
                note_type(name, out);
            }
            if let Some(value) = value {
                expr_types(value, out);
            }
        }
        Statement::Return { value, .. }
        | Statement::Recover { value, .. }
        | Statement::Exit { code: value, .. } => {
            if let Some(value) = value {
                expr_types(value, out);
            }
        }
        Statement::Fail { error, .. } => expr_types(error, out),
        Statement::Assign { value, .. } | Statement::StateAssign { value, .. } => {
            expr_types(value, out)
        }
        Statement::Expression { expression, .. } => expr_types(expression, out),
        Statement::If {
            condition,
            then_body,
            else_body,
            ..
        } => {
            expr_types(condition, out);
            then_body.iter().for_each(|s| stmt_types(s, out));
            else_body.iter().for_each(|s| stmt_types(s, out));
        }
        Statement::Match {
            expression, cases, ..
        } => {
            expr_types(expression, out);
            for case in cases {
                // A `CASE T(x)` names `T`, and binds a value of it.
                if let MatchPattern::Union { type_name, .. } = &case.pattern {
                    note_type(type_name, out);
                }
                case.body.iter().for_each(|s| stmt_types(s, out));
            }
        }
        Statement::For {
            start,
            end,
            step,
            body,
            ..
        } => {
            expr_types(start, out);
            expr_types(end, out);
            if let Some(step) = step {
                expr_types(step, out);
            }
            body.iter().for_each(|s| stmt_types(s, out));
        }
        Statement::ForEach { iterable, body, .. } => {
            expr_types(iterable, out);
            body.iter().for_each(|s| stmt_types(s, out));
        }
        Statement::While {
            condition, body, ..
        }
        | Statement::DoUntil {
            condition, body, ..
        } => {
            expr_types(condition, out);
            body.iter().for_each(|s| stmt_types(s, out));
        }
        Statement::Continue { .. } | Statement::Propagate { .. } => {}
    }
}

/// The annotations inside an expression: a lambda's parameters, and the statements of
/// an inline `TRAP` handler.
fn expr_types(expr: &crate::ast::Expression, out: &mut std::collections::HashSet<String>) {
    use crate::ast::{CallArg, ConstructorArg, Expression};
    match expr {
        Expression::Call { arguments, .. } => {
            for argument in arguments {
                match argument {
                    CallArg::Positional(v) | CallArg::Named { value: v, .. } => expr_types(v, out),
                }
            }
        }
        Expression::Binary { left, right, .. } => {
            expr_types(left, out);
            expr_types(right, out);
        }
        Expression::Unary { operand, .. } => expr_types(operand, out),
        Expression::Lambda { params, body, .. } => {
            params_types(params, out);
            expr_types(body, out);
        }
        Expression::Constructor { arguments, .. } => {
            for argument in arguments {
                match argument {
                    ConstructorArg::Positional(v) | ConstructorArg::Named { value: v, .. } => {
                        expr_types(v, out)
                    }
                }
            }
        }
        Expression::WithUpdate { target, updates } => {
            expr_types(target, out);
            updates.iter().for_each(|u| expr_types(&u.value, out));
        }
        Expression::ListLiteral(values) => values.iter().for_each(|v| expr_types(v, out)),
        Expression::SetLiteral { elements, .. } => elements.iter().for_each(|v| expr_types(v, out)),
        Expression::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                expr_types(key, out);
                expr_types(value, out);
            }
        }
        Expression::MemberAccess { target, .. } => expr_types(target, out),
        Expression::Trapped {
            expression,
            handler,
            ..
        } => {
            expr_types(expression, out);
            handler.iter().for_each(|s| stmt_types(s, out));
        }
        Expression::String(_)
        | Expression::Number(_)
        | Expression::Scalar(_)
        | Expression::Boolean(_)
        | Expression::Identifier(_) => {}
    }
}

pub(super) fn item_callees(
    item: &crate::ast::Item,
    out: &mut std::collections::HashSet<String>,
    made: &mut std::collections::HashSet<String>,
) {
    use crate::ast::Item;
    match item {
        Item::Function(f) => f.body.iter().for_each(|s| stmt_callees(s, out, made)),
        Item::Binding(b) => {
            if let Some(value) = &b.value {
                expr_callees(value, out, made);
            }
        }
        Item::Testing(block) => block
            .groups
            .iter()
            .for_each(|g| group_callees(g, out, made)),
        _ => {}
    }
}

/// A `TESTING` block is the verbatim AST node in HIR too, so one walk serves both.
pub(super) fn group_callees(
    group: &crate::ast::TestGroup,
    out: &mut std::collections::HashSet<String>,
    made: &mut std::collections::HashSet<String>,
) {
    use crate::ast::TestGroupMember;
    for member in &group.members {
        match member {
            TestGroupMember::Case(case) => {
                case.body.iter().for_each(|s| stmt_callees(s, out, made))
            }
            TestGroupMember::Group(nested) => group_callees(nested, out, made),
        }
    }
}

fn stmt_callees(
    stmt: &crate::ast::Statement,
    out: &mut std::collections::HashSet<String>,
    made: &mut std::collections::HashSet<String>,
) {
    use crate::ast::Statement;
    match stmt {
        Statement::Let { value, .. }
        | Statement::Return { value, .. }
        | Statement::Recover { value, .. }
        | Statement::Exit { code: value, .. } => {
            if let Some(value) = value {
                expr_callees(value, out, made);
            }
        }
        Statement::Fail { error, .. } => expr_callees(error, out, made),
        Statement::Assign { value, .. } | Statement::StateAssign { value, .. } => {
            expr_callees(value, out, made)
        }
        Statement::Expression { expression, .. } => expr_callees(expression, out, made),
        Statement::If {
            condition,
            then_body,
            else_body,
            ..
        } => {
            expr_callees(condition, out, made);
            then_body.iter().for_each(|s| stmt_callees(s, out, made));
            else_body.iter().for_each(|s| stmt_callees(s, out, made));
        }
        Statement::Match {
            expression, cases, ..
        } => {
            expr_callees(expression, out, made);
            for case in cases {
                case.body.iter().for_each(|s| stmt_callees(s, out, made));
            }
        }
        Statement::For {
            start,
            end,
            step,
            body,
            ..
        } => {
            expr_callees(start, out, made);
            expr_callees(end, out, made);
            if let Some(step) = step {
                expr_callees(step, out, made);
            }
            body.iter().for_each(|s| stmt_callees(s, out, made));
        }
        Statement::ForEach { iterable, body, .. } => {
            expr_callees(iterable, out, made);
            body.iter().for_each(|s| stmt_callees(s, out, made));
        }
        Statement::While {
            condition, body, ..
        }
        | Statement::DoUntil {
            condition, body, ..
        } => {
            expr_callees(condition, out, made);
            body.iter().for_each(|s| stmt_callees(s, out, made));
        }
        Statement::Continue { .. } | Statement::Propagate { .. } => {}
    }
}

fn expr_callees(
    expr: &crate::ast::Expression,
    out: &mut std::collections::HashSet<String>,
    made: &mut std::collections::HashSet<String>,
) {
    use crate::ast::{CallArg, ConstructorArg, Expression};
    match expr {
        Expression::Call {
            callee, arguments, ..
        } => {
            out.insert(short_callee(callee).to_string());
            for argument in arguments {
                match argument {
                    CallArg::Positional(v) | CallArg::Named { value: v, .. } => {
                        expr_callees(v, out, made)
                    }
                }
            }
        }
        Expression::Binary { left, right, .. } => {
            expr_callees(left, out, made);
            expr_callees(right, out, made);
        }
        Expression::Unary { operand, .. } => expr_callees(operand, out, made),
        Expression::Lambda { body, .. } => expr_callees(body, out, made),
        Expression::Constructor {
            type_name,
            arguments,
        } => {
            made.insert(short_callee(type_name).to_string());
            for argument in arguments {
                match argument {
                    ConstructorArg::Positional(v) | ConstructorArg::Named { value: v, .. } => {
                        expr_callees(v, out, made)
                    }
                }
            }
        }
        Expression::WithUpdate { target, updates } => {
            expr_callees(target, out, made);
            updates
                .iter()
                .for_each(|u| expr_callees(&u.value, out, made));
        }
        Expression::ListLiteral(values) => values.iter().for_each(|v| expr_callees(v, out, made)),
        Expression::SetLiteral { elements, .. } => {
            elements.iter().for_each(|v| expr_callees(v, out, made))
        }
        Expression::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                expr_callees(key, out, made);
                expr_callees(value, out, made);
            }
        }
        Expression::MemberAccess { target, .. } => expr_callees(target, out, made),
        Expression::Trapped {
            expression,
            handler,
            ..
        } => {
            expr_callees(expression, out, made);
            handler.iter().for_each(|s| stmt_callees(s, out, made));
        }
        Expression::String(_)
        | Expression::Number(_)
        | Expression::Scalar(_)
        | Expression::Boolean(_)
        | Expression::Identifier(_) => {}
    }
}
