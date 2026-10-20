//! plan-176-U: whether a module can raise a Float arithmetic error (`module_may_emit_float_numeric_error`).
//! The other module analyses live in `engine/analysis/module_analysis/mod.rs`.

use super::*;

pub(crate) fn module_may_emit_float_numeric_error(module: &NirModule) -> bool {
    if module_uses_any_call(
        module,
        &[
            "math.sqrt",
            "math.pow",
            "math.atan2",
            "math.exp",
            "math.log",
            "math.log10",
            "math.sin",
            "math.cos",
            "math.tan",
            "math.asin",
            "math.acos",
            "math.atan",
        ],
    ) {
        return true;
    }
    // bug-762: with each function's declared return, so a Float operation over a
    // user call (`f() * a`) types as the lowering types it.
    let fields = module_field_types(module).with_function_returns(module);
    if module.globals.iter().any(|global| {
        global.value.as_ref().is_some_and(|value| {
            value_may_emit_float_arithmetic_error(value, &HashMap::new(), &fields)
        })
    }) {
        return true;
    }
    module.functions.iter().any(|function| {
        let mut locals = function
            .params
            .iter()
            .map(|param| (param.name.clone(), param.type_.clone()))
            .collect::<HashMap<_, _>>();
        ops_may_emit_float_arithmetic_error(&function.body, &mut locals, &fields)
    })
}

fn ops_may_emit_float_arithmetic_error(
    ops: &[NirOp],
    locals: &mut HashMap<String, ParameterType>,
    fields: &FieldTypes,
) -> bool {
    for op in ops {
        let emits = match op {
            NirOp::Bind {
                name, type_, value, ..
            } => {
                let emits = value.as_ref().is_some_and(|value| {
                    value_may_emit_float_arithmetic_error(value, locals, fields)
                });
                if !type_.name().is_empty() {
                    locals.insert(name.clone(), type_.clone());
                }
                emits
            }
            NirOp::StoreGlobal { value, .. } | NirOp::Return { value } => value
                .as_ref()
                .is_some_and(|value| value_may_emit_float_arithmetic_error(value, locals, fields)),
            NirOp::ExitLoop { .. } | NirOp::ContinueLoop { .. } => false,
            NirOp::ExitProgram { code } => {
                value_may_emit_float_arithmetic_error(code, locals, fields)
            }
            NirOp::Fail { error } => value_may_emit_float_arithmetic_error(error, locals, fields),
            NirOp::Assign { value, .. }
            | NirOp::StateAssign { value, .. }
            | NirOp::Eval { value } => value_may_emit_float_arithmetic_error(value, locals, fields),
            NirOp::If {
                condition,
                then_body,
                else_body,
            } => {
                value_may_emit_float_arithmetic_error(condition, locals, fields)
                    || ops_may_emit_float_arithmetic_error(then_body, &mut locals.clone(), fields)
                    || ops_may_emit_float_arithmetic_error(else_body, &mut locals.clone(), fields)
            }
            NirOp::Match { value, cases } => {
                value_may_emit_float_arithmetic_error(value, locals, fields)
                    || cases.iter().any(|case| {
                        matches!(
                            &case.pattern,
                            NirMatchPattern::Value(value)
                                if value_may_emit_float_arithmetic_error(value, locals, fields)
                        ) || ops_may_emit_float_arithmetic_error(
                            &case.body,
                            &mut locals.clone(),
                            fields,
                        )
                    })
            }
            NirOp::While {
                condition, body, ..
            } => {
                value_may_emit_float_arithmetic_error(condition, locals, fields)
                    || ops_may_emit_float_arithmetic_error(body, &mut locals.clone(), fields)
            }
            NirOp::For {
                name,
                type_,
                start,
                end,
                step,
                body,
                ..
            } => {
                let mut body_locals = locals.clone();
                body_locals.insert(name.clone(), type_.clone());
                matches!(type_, ParameterType::Float)
                    || value_may_emit_float_arithmetic_error(start, locals, fields)
                    || value_may_emit_float_arithmetic_error(end, locals, fields)
                    || value_may_emit_float_arithmetic_error(step, locals, fields)
                    || ops_may_emit_float_arithmetic_error(body, &mut body_locals, fields)
            }
            NirOp::DoUntil { body, condition } => {
                ops_may_emit_float_arithmetic_error(body, &mut locals.clone(), fields)
                    || value_may_emit_float_arithmetic_error(condition, locals, fields)
            }
            NirOp::ForEach {
                name,
                type_,
                iterable,
                body,
            } => {
                let mut body_locals = locals.clone();
                body_locals.insert(name.clone(), type_.clone());
                value_may_emit_float_arithmetic_error(iterable, locals, fields)
                    || ops_may_emit_float_arithmetic_error(body, &mut body_locals, fields)
            }
            NirOp::Trap { body, .. } => {
                ops_may_emit_float_arithmetic_error(body, &mut locals.clone(), fields)
            }
        };
        if emits {
            return true;
        }
    }
    false
}

fn value_may_emit_float_arithmetic_error(
    value: &NirValue,
    locals: &HashMap<String, ParameterType>,
    fields: &FieldTypes,
) -> bool {
    match value {
        NirValue::Binary {
            op, left, right, ..
        } => {
            let result_type = static_nir_value_type(left, locals, fields)
                .zip(static_nir_value_type(right, locals, fields))
                .map(|(left_type, right_type)| promoted_binary_type(*op, &left_type, &right_type));
            (matches!(
                op,
                BinaryOp::Add
                    | BinaryOp::Subtract
                    | BinaryOp::Multiply
                    | BinaryOp::Divide
                    | BinaryOp::IntDiv
                    | BinaryOp::Mod
                    | BinaryOp::Power
            ) && result_type == Some(ParameterType::Float))
                || value_may_emit_float_arithmetic_error(left, locals, fields)
                || value_may_emit_float_arithmetic_error(right, locals, fields)
        }
        NirValue::Call { target, args, .. }
        | NirValue::CallResult { target, args, .. }
        | NirValue::RuntimeCall { target, args, .. } => {
            // bug-590: a boundary re-checks the `Float` result of a builtin whose
            // callee does not constrain it (`collections::sum`), so a module that
            // calls one emits the finiteness error message strings. This recogniser
            // and the emitter (`float_arith_node`) are two lists and must agree — a
            // miss here is not a silent under-report but a hard build failure,
            // "native code string literal '…' has no data object".
            // bug-754: `collections::sum` raises `ErrFloatOverflow` itself, at the call,
            // for its `Float` overload.
            ((crate::codegen::builtins::float_result::builtin_float_result_may_be_nonfinite(
                target,
            ) || crate::codegen::builtins::native_builtin_target(target) == Some("sum"))
                && static_nir_value_type(value, locals, fields) == Some(ParameterType::Float))
                || args
                    .iter()
                    .any(|arg| value_may_emit_float_arithmetic_error(arg, locals, fields))
        }
        NirValue::Constructor { args, .. } => args
            .iter()
            .any(|arg| value_may_emit_float_arithmetic_error(arg, locals, fields)),
        NirValue::UnionWrap { value, .. }
        | NirValue::UnionExtract { value, .. }
        | NirValue::ResultIsOk { value }
        | NirValue::ResultValue { value }
        | NirValue::ResultError { value }
        | NirValue::Checked { value, .. } => {
            value_may_emit_float_arithmetic_error(value, locals, fields)
        }
        NirValue::WithUpdate {
            target, updates, ..
        } => {
            value_may_emit_float_arithmetic_error(target, locals, fields)
                || updates.iter().any(|update| {
                    value_may_emit_float_arithmetic_error(&update.value, locals, fields)
                })
        }
        NirValue::ListLiteral { values, .. } | NirValue::SetLiteral { values, .. } => values
            .iter()
            .any(|value| value_may_emit_float_arithmetic_error(value, locals, fields)),
        NirValue::MapLiteral { entries, .. } => entries.iter().any(|(key, value)| {
            value_may_emit_float_arithmetic_error(key, locals, fields)
                || value_may_emit_float_arithmetic_error(value, locals, fields)
        }),
        NirValue::MemberAccess { target, .. } => {
            value_may_emit_float_arithmetic_error(target, locals, fields)
        }
        NirValue::Unary { op, operand, .. } => {
            (*op == UnaryOp::Negate
                && static_nir_value_type(operand, locals, fields) == Some(ParameterType::Float))
                || value_may_emit_float_arithmetic_error(operand, locals, fields)
        }
        NirValue::Closure { captures, .. } => captures
            .iter()
            .any(|value| value_may_emit_float_arithmetic_error(value, locals, fields)),
        NirValue::Const { .. }
        | NirValue::Local(_)
        | NirValue::LocalRef { .. }
        | NirValue::Global { .. }
        | NirValue::FunctionRef { .. }
        | NirValue::Capture { .. } => false,
    }
}
