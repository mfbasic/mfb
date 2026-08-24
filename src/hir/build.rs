//! HIR builder helpers — the [`crate::ast::build`] twins for code that
//! synthesizes statements *inside* the typed pipeline (post-elaboration), where
//! constructing AST and re-elaborating would be a backward seam. Kept
//! field-for-field parallel to the AST builders so a synthesized HIR tree is
//! exactly what elaborating the equivalent AST tree would produce.

use super::{HirCallArg, HirExpression, HirStatement};
use crate::types::ParameterType;

pub(crate) fn str_lit(value: String) -> HirExpression {
    HirExpression::String(value)
}

pub(crate) fn num(value: i64) -> HirExpression {
    HirExpression::Number(value.to_string())
}

pub(crate) fn boolean(value: bool) -> HirExpression {
    HirExpression::Boolean(value)
}

pub(crate) fn ident(name: &str) -> HirExpression {
    HirExpression::Identifier(name.to_string())
}

pub(crate) fn binary(left: HirExpression, operator: &str, right: HirExpression) -> HirExpression {
    HirExpression::Binary {
        left: Box::new(left),
        operator: operator.to_string(),
        right: Box::new(right),
        line: 0,
        column: 0,
    }
}

pub(crate) fn member(target: HirExpression, name: &str) -> HirExpression {
    HirExpression::MemberAccess {
        target: Box::new(target),
        member: name.to_string(),
    }
}

pub(crate) fn call(callee: &str, arguments: Vec<HirExpression>) -> HirExpression {
    HirExpression::Call {
        callee: callee.to_string(),
        arguments: arguments.into_iter().map(HirCallArg::Positional).collect(),
        line: 0,
        column: 0,
    }
}

pub(crate) fn to_string(value: HirExpression) -> HirExpression {
    call("toString", vec![value])
}

/// Fold `parts` left-to-right with the string-concatenation operator `&`.
pub(crate) fn concat(parts: Vec<HirExpression>) -> HirExpression {
    let mut iter = parts.into_iter();
    let mut acc = iter.next().expect("concat needs at least one part");
    for part in iter {
        acc = binary(acc, "&", part);
    }
    acc
}

pub(crate) fn not(operand: HirExpression) -> HirExpression {
    HirExpression::Unary {
        operator: "NOT".to_string(),
        operand: Box::new(operand),
        line: 0,
        column: 0,
    }
}

pub(crate) fn let_mut_at(
    name: &str,
    type_name: &str,
    value: HirExpression,
    line: usize,
) -> HirStatement {
    HirStatement::Let {
        mutable: true,
        resource: false,
        state_type: None,
        name: name.to_string(),
        type_: ParameterType::parse(type_name),
        explicit_type: true,
        value: Some(value),
        line,
    }
}

pub(crate) fn let_imm(name: &str, value: HirExpression, line: usize) -> HirStatement {
    HirStatement::Let {
        mutable: false,
        resource: false,
        state_type: None,
        name: name.to_string(),
        // An inferred binding: no `AS T` annotation, exactly as elaborating the
        // AST form (`type_name: None`) produces.
        type_: ParameterType::Unknown,
        explicit_type: false,
        value: Some(value),
        line,
    }
}

pub(crate) fn assign_at(name: &str, value: HirExpression, line: usize) -> HirStatement {
    HirStatement::Assign {
        name: name.to_string(),
        value,
        line,
    }
}

pub(crate) fn if_then(
    condition: HirExpression,
    then_body: Vec<HirStatement>,
    line: usize,
) -> HirStatement {
    HirStatement::If {
        condition,
        then_body,
        else_body: Vec::new(),
        line,
    }
}

/// `<inner> TRAP(binding) …handler… END TRAP` as a bare expression statement.
pub(crate) fn trap_stmt(
    inner: HirExpression,
    binding: &str,
    handler: Vec<HirStatement>,
    line: usize,
) -> HirStatement {
    HirStatement::Expression {
        expression: HirExpression::Trapped {
            expression: Box::new(inner),
            binding: binding.to_string(),
            handler,
            line,
        },
        line,
    }
}
