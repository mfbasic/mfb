//! Pipeline `|>` placeholder desugaring.
//!
//! `parse_pipeline` (see [`super::expr`]) lowers `left |> right` by substituting
//! `left` for each `_` placeholder in `right`. These helpers do the expression-
//! tree walk: [`contains_placeholder`] decides whether `right` names a
//! placeholder at all, and [`substitute_placeholder`] performs the rewrite. They
//! previously lived in the `-ast` JSON dumper (`serialize.rs`) and were re-imported
//! back out of it through `mod.rs` to reach the parser — a semantic AST rewrite
//! has nothing to do with dumping AST, so it lives here, imported directly by the
//! parser.

use super::*;

pub(super) fn contains_placeholder(expression: &Expression) -> bool {
    match expression {
        Expression::Identifier(value) => value == "_",
        Expression::Binary { left, right, .. } => {
            contains_placeholder(left) || contains_placeholder(right)
        }
        Expression::Unary { operand, .. } => contains_placeholder(operand),
        Expression::Call { arguments, .. } => arguments.iter().any(call_arg_contains_placeholder),
        Expression::Constructor { arguments, .. } => {
            arguments.iter().any(constructor_arg_contains_placeholder)
        }
        Expression::Lambda { body, .. } => contains_placeholder(body),
        Expression::ListLiteral(values) => values.iter().any(contains_placeholder),
        Expression::MapLiteral { entries, .. } => entries
            .iter()
            .any(|(key, value)| contains_placeholder(key) || contains_placeholder(value)),
        Expression::MemberAccess { target, .. } => contains_placeholder(target),
        Expression::Trapped { expression, .. } => contains_placeholder(expression),
        Expression::WithUpdate { target, updates } => {
            contains_placeholder(target)
                || updates
                    .iter()
                    .any(|update| contains_placeholder(&update.value))
        }
        Expression::String(_)
        | Expression::Number(_)
        | Expression::Scalar(_)
        | Expression::Boolean(_) => false,
    }
}

fn constructor_arg_contains_placeholder(argument: &ConstructorArg) -> bool {
    match argument {
        ConstructorArg::Positional(value) => contains_placeholder(value),
        ConstructorArg::Named { value, .. } => contains_placeholder(value),
    }
}

fn call_arg_contains_placeholder(argument: &CallArg) -> bool {
    match argument {
        CallArg::Positional(value) => contains_placeholder(value),
        CallArg::Named { value, .. } => contains_placeholder(value),
    }
}

pub(super) fn substitute_placeholder(expression: Expression, input: &Expression) -> Expression {
    match expression {
        Expression::Identifier(value) if value == "_" => input.clone(),
        Expression::Binary {
            left,
            operator,
            right,
            line,
            column,
        } => Expression::Binary {
            left: Box::new(substitute_placeholder(*left, input)),
            operator,
            right: Box::new(substitute_placeholder(*right, input)),
            line,
            column,
        },
        Expression::Unary {
            operator,
            operand,
            line,
            column,
        } => Expression::Unary {
            operator,
            operand: Box::new(substitute_placeholder(*operand, input)),
            line,
            column,
        },
        Expression::Call {
            callee,
            arguments,
            line,
            column,
        } => Expression::Call {
            callee,
            arguments: arguments
                .into_iter()
                .map(|argument| substitute_placeholder_call_arg(argument, input))
                .collect(),
            line,
            column,
        },
        Expression::Lambda {
            params,
            body,
            assign_target,
        } => Expression::Lambda {
            params,
            body: Box::new(substitute_placeholder(*body, input)),
            assign_target,
        },
        Expression::Constructor {
            type_name,
            arguments,
        } => Expression::Constructor {
            type_name,
            arguments: arguments
                .into_iter()
                .map(|argument| substitute_placeholder_constructor_arg(argument, input))
                .collect(),
        },
        Expression::ListLiteral(values) => Expression::ListLiteral(
            values
                .into_iter()
                .map(|value| substitute_placeholder(value, input))
                .collect(),
        ),
        Expression::MapLiteral {
            key_type,
            value_type,
            entries,
        } => Expression::MapLiteral {
            key_type,
            value_type,
            entries: entries
                .into_iter()
                .map(|(key, value)| {
                    (
                        substitute_placeholder(key, input),
                        substitute_placeholder(value, input),
                    )
                })
                .collect(),
        },
        Expression::MemberAccess { target, member } => Expression::MemberAccess {
            target: Box::new(substitute_placeholder(*target, input)),
            member,
        },
        // Mirror `contains_placeholder`, which walks a `Trapped`'s inner
        // expression: substitute there too so a `_` inside a trapped subexpression
        // is rewritten rather than silently left behind (bug-171 finding C). The
        // handler body holds statements (not the pipeline input) and is left as-is.
        Expression::Trapped {
            expression,
            binding,
            handler,
            line,
        } => Expression::Trapped {
            expression: Box::new(substitute_placeholder(*expression, input)),
            binding,
            handler,
            line,
        },
        Expression::WithUpdate { target, updates } => Expression::WithUpdate {
            target: Box::new(substitute_placeholder(*target, input)),
            updates: updates
                .into_iter()
                .map(|update| RecordUpdate {
                    field: update.field,
                    value: substitute_placeholder(update.value, input),
                    line: update.line,
                })
                .collect(),
        },
        other => other,
    }
}

fn substitute_placeholder_constructor_arg(
    argument: ConstructorArg,
    input: &Expression,
) -> ConstructorArg {
    match argument {
        ConstructorArg::Positional(value) => {
            ConstructorArg::Positional(substitute_placeholder(value, input))
        }
        ConstructorArg::Named { name, value, line } => ConstructorArg::Named {
            name,
            value: substitute_placeholder(value, input),
            line,
        },
    }
}

fn substitute_placeholder_call_arg(argument: CallArg, input: &Expression) -> CallArg {
    match argument {
        CallArg::Positional(value) => CallArg::Positional(substitute_placeholder(value, input)),
        CallArg::Named { name, value, line } => CallArg::Named {
            name,
            value: substitute_placeholder(value, input),
            line,
        },
    }
}
