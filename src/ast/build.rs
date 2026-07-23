use super::*;

// ---------------------------------------------------------------------------
// AST builder helpers
// ---------------------------------------------------------------------------

pub(crate) fn str_lit(value: String) -> Expression {
    Expression::String(value)
}

pub(crate) fn num(value: i64) -> Expression {
    Expression::Number(value.to_string())
}

pub(crate) fn boolean(value: bool) -> Expression {
    Expression::Boolean(value)
}

pub(crate) fn ident(name: &str) -> Expression {
    Expression::Identifier(name.to_string())
}

pub(crate) fn binary(left: Expression, operator: &str, right: Expression) -> Expression {
    Expression::Binary {
        left: Box::new(left),
        operator: operator.to_string(),
        right: Box::new(right),
        line: 0,
        column: 0,
    }
}

pub(crate) fn member(target: Expression, name: &str) -> Expression {
    Expression::MemberAccess {
        target: Box::new(target),
        member: name.to_string(),
    }
}

pub(crate) fn call(callee: &str, arguments: Vec<Expression>) -> Expression {
    Expression::Call {
        callee: callee.to_string(),
        arguments: arguments.into_iter().map(CallArg::Positional).collect(),
        line: 0,
        column: 0,
    }
}

pub(crate) fn to_string(value: Expression) -> Expression {
    call("toString", vec![value])
}

/// Fold `parts` left-to-right with the string-concatenation operator `&`.
pub(crate) fn concat(parts: Vec<Expression>) -> Expression {
    let mut iter = parts.into_iter();
    let mut acc = iter.next().expect("concat needs at least one part");
    for part in iter {
        acc = binary(acc, "&", part);
    }
    acc
}

pub(crate) fn print_line(value: Expression) -> Statement {
    Statement::Expression {
        expression: call("io.print", vec![value]),
        line: 0,
    }
}

pub(crate) fn not(operand: Expression) -> Expression {
    Expression::Unary {
        operator: "NOT".to_string(),
        operand: Box::new(operand),
        line: 0,
        column: 0,
    }
}

pub(crate) fn let_mut(name: &str, type_name: &str, value: Expression) -> Statement {
    let_mut_at(name, type_name, value, 0)
}

pub(crate) fn let_mut_at(name: &str, type_name: &str, value: Expression, line: usize) -> Statement {
    Statement::Let {
        mutable: true,
        resource: false,
        state_type: None,
        name: name.to_string(),
        type_name: Some(type_name.to_string()),
        value: Some(value),
        line,
    }
}

pub(crate) fn let_imm(name: &str, value: Expression, line: usize) -> Statement {
    Statement::Let {
        mutable: false,
        resource: false,
        state_type: None,
        name: name.to_string(),
        type_name: None,
        value: Some(value),
        line,
    }
}

pub(crate) fn assign(name: &str, value: Expression) -> Statement {
    assign_at(name, value, 0)
}

pub(crate) fn assign_at(name: &str, value: Expression, line: usize) -> Statement {
    Statement::Assign {
        name: name.to_string(),
        value,
        line,
    }
}

pub(crate) fn if_then(condition: Expression, then_body: Vec<Statement>, line: usize) -> Statement {
    Statement::If {
        condition,
        then_body,
        else_body: Vec::new(),
        line,
    }
}

pub(crate) fn if_else(
    condition: Expression,
    then_body: Vec<Statement>,
    else_body: Vec<Statement>,
    line: usize,
) -> Statement {
    Statement::If {
        condition,
        then_body,
        else_body,
        line,
    }
}

/// `<inner> TRAP(binding) …handler… END TRAP` as a bare expression statement.
pub(crate) fn trap_stmt(
    inner: Expression,
    binding: &str,
    handler: Vec<Statement>,
    line: usize,
) -> Statement {
    Statement::Expression {
        expression: Expression::Trapped {
            expression: Box::new(inner),
            binding: binding.to_string(),
            handler,
            line,
        },
        line,
    }
}

pub(crate) fn ret(value: Expression) -> Statement {
    Statement::Return {
        value: Some(value),
        line: 0,
    }
}

pub(crate) fn empty_list() -> Expression {
    Expression::ListLiteral(Vec::new())
}

pub(crate) fn while_loop(condition: Expression, body: Vec<Statement>) -> Statement {
    Statement::While {
        kind: LoopKind::While,
        condition,
        body,
        line: 0,
    }
}

pub(crate) fn param(name: &str, type_name: &str) -> Param {
    Param {
        name: name.to_string(),
        type_name: Some(type_name.to_string()),
        resource: false,
        state_type: None,
        default: None,
        line: 0,
    }
}

pub(crate) fn func(
    name: &str,
    params: Vec<Param>,
    return_type: Option<&str>,
    body: Vec<Statement>,
) -> Function {
    Function {
        kind: FunctionKind::Func,
        visibility: Visibility::Public,
        isolated: false,
        name: name.to_string(),
        template_params: Vec::new(),
        params,
        return_type: return_type.map(str::to_string),
        return_resource: false,
        return_state_type: None,
        body,
        trap: None,
        line: 0,
    }
}

pub(crate) fn sub(name: &str, params: Vec<Param>, body: Vec<Statement>) -> Function {
    Function {
        kind: FunctionKind::Sub,
        visibility: Visibility::Public,
        isolated: false,
        name: name.to_string(),
        template_params: Vec::new(),
        params,
        return_type: None,
        return_resource: false,
        return_state_type: None,
        body,
        trap: None,
        line: 0,
    }
}

pub(crate) fn global_mut(name: &str, type_name: &str, value: Expression) -> TopLevelBinding {
    TopLevelBinding {
        visibility: Visibility::Public,
        mutable: true,
        resource: false,
        state_type: None,
        name: name.to_string(),
        type_name: Some(type_name.to_string()),
        value: Some(value),
        line: 0,
    }
}
