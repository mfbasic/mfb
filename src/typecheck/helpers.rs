use super::*;

pub(super) fn statement_line(statement: &Statement) -> usize {
    match statement {
        Statement::Let { line, .. }
        | Statement::Return { line, .. }
        | Statement::Exit { line, .. }
        | Statement::Continue { line, .. }
        | Statement::Fail { line, .. }
        | Statement::Propagate { line }
        | Statement::Recover { line, .. }
        | Statement::Assign { line, .. }
        | Statement::StateAssign { line, .. }
        | Statement::Expression { line, .. }
        | Statement::If { line, .. }
        | Statement::Match { line, .. }
        | Statement::For { line, .. }
        | Statement::ForEach { line, .. }
        | Statement::While { line, .. }
        | Statement::DoUntil { line, .. } => *line,
    }
}

pub(super) fn integer_constant_value(expression: &Expression) -> Option<i128> {
    match expression {
        Expression::Number(value) => value.parse::<i128>().ok(),
        Expression::Unary {
            operator, operand, ..
        } if operator == "-" => integer_constant_value(operand).map(|value| -value),
        _ => None,
    }
}

pub(super) fn integer_literal_in_range(expression: &Expression) -> bool {
    match expression {
        Expression::Number(value) if !value.contains('.') => value.parse::<i64>().is_ok(),
        Expression::Unary {
            operator, operand, ..
        } if operator == "-" => {
            let Expression::Number(value) = operand.as_ref() else {
                return true;
            };
            if value.contains('.') {
                return true;
            }
            value
                .parse::<u64>()
                .is_ok_and(|number| number <= (i64::MAX as u64) + 1)
        }
        _ => true,
    }
}

pub(super) fn effective_field_visibility(
    declared: Option<Visibility>,
    containing_visibility: Visibility,
) -> Visibility {
    declared.unwrap_or(match containing_visibility {
        Visibility::Export => Visibility::Export,
        Visibility::Package | Visibility::Private => Visibility::Package,
    })
}

pub(super) fn function_type(sig: &FunctionSig) -> Type {
    Type::Function {
        params: sig.params.iter().map(|param| param.type_.clone()).collect(),
        return_type: Box::new(sig.return_type.clone()),
        isolated: sig.isolated,
    }
}

pub(super) fn captured_locals(
    expression: &Expression,
    outer_locals: &HashMap<String, LocalInfo>,
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

pub(super) fn collect_captured_locals(
    expression: &Expression,
    outer_locals: &HashMap<String, LocalInfo>,
    local_names: &HashSet<String>,
    seen: &mut HashSet<String>,
    captures: &mut Vec<CapturedLocal>,
) {
    match expression {
        Expression::Identifier(name) => {
            if let Some(local) = outer_locals.get(name) {
                if !local_names.contains(name) && seen.insert(name.clone()) {
                    captures.push(CapturedLocal {
                        name: name.clone(),
                        type_: local.type_.clone(),
                        mutable: local.mutable,
                    });
                }
            }
        }
        Expression::Call {
            callee, arguments, ..
        } => {
            if let Some(local) = outer_locals.get(callee) {
                if !local_names.contains(callee) && seen.insert(callee.clone()) {
                    captures.push(CapturedLocal {
                        name: callee.clone(),
                        type_: local.type_.clone(),
                        mutable: local.mutable,
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
        Expression::Lambda { .. } => {}
        Expression::Binary { left, right, .. } => {
            collect_captured_locals(left, outer_locals, local_names, seen, captures);
            collect_captured_locals(right, outer_locals, local_names, seen, captures);
        }
        Expression::Unary { operand, .. } => {
            collect_captured_locals(operand, outer_locals, local_names, seen, captures);
        }
        Expression::Constructor { arguments, .. } => {
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
        Expression::ListLiteral(values) => {
            for value in values {
                collect_captured_locals(value, outer_locals, local_names, seen, captures);
            }
        }
        Expression::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                collect_captured_locals(key, outer_locals, local_names, seen, captures);
                collect_captured_locals(value, outer_locals, local_names, seen, captures);
            }
        }
        Expression::MemberAccess { target, .. } => {
            collect_captured_locals(target, outer_locals, local_names, seen, captures);
        }
        Expression::Trapped { expression, .. } => {
            collect_captured_locals(expression, outer_locals, local_names, seen, captures);
        }
        Expression::WithUpdate { target, updates } => {
            collect_captured_locals(target, outer_locals, local_names, seen, captures);
            for update in updates {
                collect_captured_locals(&update.value, outer_locals, local_names, seen, captures);
            }
        }
        Expression::String(_) | Expression::Number(_) | Expression::Boolean(_) => {}
    }
}

pub(super) fn constructor_arg_value(argument: &ConstructorArg) -> &Expression {
    match argument {
        ConstructorArg::Positional(value) => value,
        ConstructorArg::Named { value, .. } => value,
    }
}

pub(super) fn call_arg_value(argument: &CallArg) -> &Expression {
    match argument {
        CallArg::Positional(value) => value,
        CallArg::Named { value, .. } => value,
    }
}

/// Unwrap a `RES`-marked collection element (`Type::Res`) to the underlying
/// type; a no-op for any other type.
pub(super) fn strip_res(type_: &Type) -> &Type {
    match type_ {
        Type::Res(inner) => inner,
        other => other,
    }
}

/// Whether an expression reads a single element out of a collection (`get` /
/// `getOr`). Of resource type, the result is a borrow that may not be `RES`-bound
/// (§15.6).
pub(super) fn is_resource_element_borrow(expression: &Expression) -> bool {
    matches!(
        expression,
        Expression::Call { callee, .. }
            if matches!(
                crate::builtins::collections::native_member_bare(callee),
                Some("get" | "getOr")
            )
    )
}

/// Whether `type_name` is a raw C ABI type that may appear only inside an
/// `ABI (...)` slot, never in a wrapper's MFBASIC-facing signature
/// (plan-link-update.md §5/§11). `CPtr` is the resource representation; the
/// others are scalar marshaling types.
pub(super) fn is_c_abi_type(type_name: &str) -> bool {
    matches!(
        type_name,
        "CPtr"
            | "CString"
            | "CInt8"
            | "CInt16"
            | "CInt32"
            | "CInt64"
            | "CUInt8"
            | "CUInt16"
            | "CUInt32"
            | "CUInt64"
            | "CFloat"
            | "CDouble"
    )
}

pub(super) fn numeric_literal_type(expression: &Expression) -> Option<Type> {
    match expression {
        Expression::Number(number) if number.contains('.') => Some(Type::Float),
        Expression::Number(_) => Some(Type::Integer),
        Expression::Unary {
            operator, operand, ..
        } if operator == "-" && matches!(operand.as_ref(), Expression::Number(_)) => {
            numeric_literal_type(operand)
        }
        _ => None,
    }
}

pub(super) fn numeric_literal_is_zero(expression: &Expression) -> bool {
    match expression {
        Expression::Number(value) => value.parse::<f64>().is_ok_and(|number| number == 0.0),
        Expression::Unary {
            operator, operand, ..
        } if operator == "-" && matches!(operand.as_ref(), Expression::Number(_)) => {
            numeric_literal_is_zero(operand)
        }
        _ => false,
    }
}

pub(super) fn promote_loop_numeric_type(start: &Type, end: &Type, step: &Type) -> Type {
    let Some(start_name) = numeric_type_name(start) else {
        return Type::Unknown;
    };
    let Some(end_name) = numeric_type_name(end) else {
        return Type::Unknown;
    };
    let Some(step_name) = numeric_type_name(step) else {
        return Type::Unknown;
    };
    let first =
        numeric::binary_result_type("+", start_name, end_name).unwrap_or(numeric::TYPE_INTEGER);
    let second =
        numeric::binary_result_type("+", first, step_name).unwrap_or(numeric::TYPE_INTEGER);
    type_from_numeric_name(second)
}

pub(super) fn type_from_numeric_name(type_name: &str) -> Type {
    match type_name {
        numeric::TYPE_BYTE => Type::Byte,
        numeric::TYPE_INTEGER => Type::Integer,
        numeric::TYPE_FIXED => Type::Fixed,
        numeric::TYPE_FLOAT => Type::Float,
        _ => Type::Unknown,
    }
}

pub(super) fn numeric_binary_result_type(operator: &str, left: &Type, right: &Type) -> Type {
    let Some(left) = numeric_type_name(left) else {
        return Type::Unknown;
    };
    let Some(right) = numeric_type_name(right) else {
        return Type::Unknown;
    };
    match numeric::binary_result_type(operator, left, right) {
        Some("Byte") => Type::Byte,
        Some("Fixed") => Type::Fixed,
        Some("Float") => Type::Float,
        Some("Integer") => Type::Integer,
        _ => Type::Unknown,
    }
}

pub(super) fn numeric_type_name(type_: &Type) -> Option<&'static str> {
    match type_ {
        Type::Byte => Some(numeric::TYPE_BYTE),
        Type::Fixed => Some(numeric::TYPE_FIXED),
        Type::Float => Some(numeric::TYPE_FLOAT),
        Type::Integer => Some(numeric::TYPE_INTEGER),
        _ => None,
    }
}

pub(super) fn read_only_record_type(type_name: &str) -> bool {
    type_name == builtins::term::TERM_COLOR_TYPE
        || type_name == builtins::term::TERM_SIZE_TYPE
        || type_name == builtins::net::ADDRESS_TYPE
        || type_name.starts_with("MapEntry OF ")
}
