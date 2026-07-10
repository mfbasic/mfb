use super::*;

pub(super) fn module_requires_empty_string_constant(module: &NirModule) -> bool {
    let type_model = TypeModel::from_module(module).unwrap_or_else(|_| TypeModel::empty());
    module.functions.iter().any(|function| {
        function
            .body
            .iter()
            .any(|op| op_requires_empty_string_constant(op, &type_model))
    })
}

fn op_requires_empty_string_constant(op: &NirOp, type_model: &TypeModel) -> bool {
    match op {
        NirOp::Bind {
            type_, value: None, ..
        } => type_requires_empty_string_constant(type_, type_model, &mut HashSet::new()),
        NirOp::If {
            then_body,
            else_body,
            ..
        } => {
            then_body
                .iter()
                .any(|op| op_requires_empty_string_constant(op, type_model))
                || else_body
                    .iter()
                    .any(|op| op_requires_empty_string_constant(op, type_model))
        }
        NirOp::Match { cases, .. } => cases.iter().any(|case| {
            case.body
                .iter()
                .any(|op| op_requires_empty_string_constant(op, type_model))
        }),
        NirOp::While { body, .. }
        | NirOp::For { body, .. }
        | NirOp::DoUntil { body, .. }
        | NirOp::ForEach { body, .. }
        | NirOp::Trap { body, .. } => body
            .iter()
            .any(|op| op_requires_empty_string_constant(op, type_model)),
        // An initialized bind supplies its own value; the remaining ops carry no
        // loop/branch bodies to descend into. Enumerated exhaustively (no
        // `_ => false`) so a future `NirOp` variant with a body cannot silently
        // regress this analysis — the same gap that produced bug-45 and bug-67.
        NirOp::Bind { value: Some(_), .. }
        | NirOp::StoreGlobal { .. }
        | NirOp::Assign { .. }
        | NirOp::StateAssign { .. }
        | NirOp::Return { .. }
        | NirOp::ExitLoop { .. }
        | NirOp::ContinueLoop { .. }
        | NirOp::ExitProgram { .. }
        | NirOp::Fail { .. }
        | NirOp::Eval { .. } => false,
    }
}

fn type_requires_empty_string_constant(
    type_: &str,
    type_model: &TypeModel,
    seen: &mut HashSet<String>,
) -> bool {
    if type_ == "String" {
        return true;
    }
    let Some(fields) = type_model.record_fields.get(type_) else {
        return false;
    };
    if !seen.insert(type_.to_string()) {
        return false;
    }
    let result = fields
        .iter()
        .any(|(_, field_type)| type_requires_empty_string_constant(field_type, type_model, seen));
    seen.remove(type_);
    result
}

pub(super) fn module_uses_type_name(module: &NirModule) -> bool {
    module
        .functions
        .iter()
        .any(|function| ops_use_type_name(&function.body))
}

pub(super) fn module_uses_call(module: &NirModule, target: &str) -> bool {
    module
        .functions
        .iter()
        .any(|function| ops_use_call(&function.body, target))
        || module_drops_resource_union_close(module, target)
}

/// Whether the module uses a migrated `collections::`/`strings::` member whose
/// bare native lowering name is `bare` (e.g. `bare = "find"` checks both
/// `collections.find` and `strings.find`). The native ops keep their bare
/// lowering but arrive with the qualified target (plan-01-functions.md §5).

/// Whether the module uses a migrated `collections::`/`strings::` member whose
/// bare native lowering name is `bare` (e.g. `bare = "find"` checks both
/// `collections.find` and `strings.find`). The native ops keep their bare
/// lowering but arrive with the qualified target (plan-01-functions.md §5).
pub(super) fn module_uses_migrated(module: &NirModule, bare: &str) -> bool {
    module_uses_call(module, &format!("collections.{bare}"))
        || module_uses_call(module, &format!("strings.{bare}"))
}

/// Whether the module binds a resource union whose tag-dispatched drop calls
/// `target` (a variant's close op). These calls are codegen-emitted rather than
/// NIR calls, so they must still pull in the close helper.

/// Whether the module binds a resource union whose tag-dispatched drop calls
/// `target` (a variant's close op). These calls are codegen-emitted rather than
/// NIR calls, so they must still pull in the close helper.
fn module_drops_resource_union_close(module: &NirModule, target: &str) -> bool {
    let unions: std::collections::HashSet<&str> = module
        .types
        .iter()
        .filter(|type_| {
            type_.kind == "union"
                && !type_.variants.is_empty()
                && type_
                    .variants
                    .iter()
                    .all(|variant| crate::builtins::is_resource_type(&variant.name))
                && type_.variants.iter().any(|variant| {
                    crate::builtins::resource_close_function(&variant.name) == Some(target)
                })
        })
        .map(|type_| type_.name.as_str())
        .collect();
    if unions.is_empty() {
        return false;
    }
    module
        .functions
        .iter()
        .any(|function| ops_bind_type_in(&function.body, &unions))
}

fn ops_bind_type_in(ops: &[NirOp], names: &std::collections::HashSet<&str>) -> bool {
    ops.iter().any(|op| match op {
        NirOp::Bind { type_, .. } => names.contains(type_.as_str()),
        NirOp::If {
            then_body,
            else_body,
            ..
        } => ops_bind_type_in(then_body, names) || ops_bind_type_in(else_body, names),
        NirOp::Match { cases, .. } => cases.iter().any(|case| ops_bind_type_in(&case.body, names)),
        NirOp::While { body, .. }
        | NirOp::For { body, .. }
        | NirOp::DoUntil { body, .. }
        | NirOp::ForEach { body, .. }
        | NirOp::Trap { body, .. } => ops_bind_type_in(body, names),
        // Enumerated exhaustively (no `_ => false`) so a future body-bearing
        // `NirOp` variant cannot silently skip this traversal (bug-67 class).
        NirOp::StoreGlobal { .. }
        | NirOp::Assign { .. }
        | NirOp::StateAssign { .. }
        | NirOp::Return { .. }
        | NirOp::ExitLoop { .. }
        | NirOp::ContinueLoop { .. }
        | NirOp::ExitProgram { .. }
        | NirOp::Fail { .. }
        | NirOp::Eval { .. } => false,
    })
}

pub(super) fn module_may_record_cleanup_failure(module: &NirModule) -> bool {
    module
        .functions
        .iter()
        .any(|function| ops_may_record_cleanup_failure(&function.body))
}

fn ops_may_record_cleanup_failure(ops: &[NirOp]) -> bool {
    ops.iter().any(|op| match op {
        NirOp::Bind { type_, .. } => crate::builtins::resource_close_function(type_).is_some(),
        NirOp::If {
            then_body,
            else_body,
            ..
        } => ops_may_record_cleanup_failure(then_body) || ops_may_record_cleanup_failure(else_body),
        NirOp::Match { cases, .. } => cases
            .iter()
            .any(|case| ops_may_record_cleanup_failure(&case.body)),
        NirOp::While { body, .. }
        | NirOp::For { body, .. }
        | NirOp::DoUntil { body, .. }
        | NirOp::ForEach { body, .. }
        | NirOp::Trap { body, .. } => ops_may_record_cleanup_failure(body),
        NirOp::StoreGlobal { .. }
        | NirOp::Assign { .. }
        | NirOp::StateAssign { .. }
        | NirOp::Return { .. }
        | NirOp::ExitLoop { .. }
        | NirOp::ContinueLoop { .. }
        | NirOp::ExitProgram { .. }
        | NirOp::Fail { .. }
        | NirOp::Eval { .. } => false,
    })
}

pub(super) fn module_uses_any_call(module: &NirModule, targets: &[&str]) -> bool {
    targets
        .iter()
        .any(|target| module_uses_call(module, target))
}

pub(super) fn module_may_emit_float_numeric_error(module: &NirModule) -> bool {
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
    if module.globals.iter().any(|global| {
        global
            .value
            .as_ref()
            .is_some_and(|value| value_may_emit_float_arithmetic_error(value, &HashMap::new()))
    }) {
        return true;
    }
    module.functions.iter().any(|function| {
        let mut locals = function
            .params
            .iter()
            .map(|param| (param.name.clone(), param.type_.clone()))
            .collect::<HashMap<_, _>>();
        ops_may_emit_float_arithmetic_error(&function.body, &mut locals)
    })
}

fn ops_may_emit_float_arithmetic_error(
    ops: &[NirOp],
    locals: &mut HashMap<String, String>,
) -> bool {
    for op in ops {
        let emits = match op {
            NirOp::Bind {
                name, type_, value, ..
            } => {
                let emits = value
                    .as_ref()
                    .is_some_and(|value| value_may_emit_float_arithmetic_error(value, locals));
                if !type_.is_empty() {
                    locals.insert(name.clone(), type_.clone());
                }
                emits
            }
            NirOp::StoreGlobal { value, .. } | NirOp::Return { value } => value
                .as_ref()
                .is_some_and(|value| value_may_emit_float_arithmetic_error(value, locals)),
            NirOp::ExitLoop { .. } | NirOp::ContinueLoop { .. } => false,
            NirOp::ExitProgram { code } => value_may_emit_float_arithmetic_error(code, locals),
            NirOp::Fail { error } => value_may_emit_float_arithmetic_error(error, locals),
            NirOp::Assign { value, .. }
            | NirOp::StateAssign { value, .. }
            | NirOp::Eval { value } => value_may_emit_float_arithmetic_error(value, locals),
            NirOp::If {
                condition,
                then_body,
                else_body,
            } => {
                value_may_emit_float_arithmetic_error(condition, locals)
                    || ops_may_emit_float_arithmetic_error(then_body, &mut locals.clone())
                    || ops_may_emit_float_arithmetic_error(else_body, &mut locals.clone())
            }
            NirOp::Match { value, cases } => {
                value_may_emit_float_arithmetic_error(value, locals)
                    || cases.iter().any(|case| {
                        matches!(
                            &case.pattern,
                            NirMatchPattern::Value(value)
                                if value_may_emit_float_arithmetic_error(value, locals)
                        ) || ops_may_emit_float_arithmetic_error(&case.body, &mut locals.clone())
                    })
            }
            NirOp::While {
                condition, body, ..
            } => {
                value_may_emit_float_arithmetic_error(condition, locals)
                    || ops_may_emit_float_arithmetic_error(body, &mut locals.clone())
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
                type_ == "Float"
                    || value_may_emit_float_arithmetic_error(start, locals)
                    || value_may_emit_float_arithmetic_error(end, locals)
                    || value_may_emit_float_arithmetic_error(step, locals)
                    || ops_may_emit_float_arithmetic_error(body, &mut body_locals)
            }
            NirOp::DoUntil { body, condition } => {
                ops_may_emit_float_arithmetic_error(body, &mut locals.clone())
                    || value_may_emit_float_arithmetic_error(condition, locals)
            }
            NirOp::ForEach {
                name,
                type_,
                iterable,
                body,
            } => {
                let mut body_locals = locals.clone();
                body_locals.insert(name.clone(), type_.clone());
                value_may_emit_float_arithmetic_error(iterable, locals)
                    || ops_may_emit_float_arithmetic_error(body, &mut body_locals)
            }
            NirOp::Trap { body, .. } => {
                ops_may_emit_float_arithmetic_error(body, &mut locals.clone())
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
    locals: &HashMap<String, String>,
) -> bool {
    match value {
        NirValue::Binary {
            op, left, right, ..
        } => {
            let result_type = static_nir_value_type(left, locals)
                .zip(static_nir_value_type(right, locals))
                .map(|(left_type, right_type)| {
                    numeric_binary_result_type(op, &left_type, &right_type)
                });
            (matches!(op.as_str(), "+" | "-" | "*" | "/" | "DIV" | "MOD" | "^")
                && result_type == Some("Float"))
                || value_may_emit_float_arithmetic_error(left, locals)
                || value_may_emit_float_arithmetic_error(right, locals)
        }
        NirValue::Call { args, .. }
        | NirValue::CallResult { args, .. }
        | NirValue::RuntimeCall { args, .. }
        | NirValue::Constructor { args, .. } => args
            .iter()
            .any(|arg| value_may_emit_float_arithmetic_error(arg, locals)),
        NirValue::UnionWrap { value, .. }
        | NirValue::UnionExtract { value, .. }
        | NirValue::ResultIsOk { value }
        | NirValue::ResultValue { value }
        | NirValue::ResultError { value } => value_may_emit_float_arithmetic_error(value, locals),
        NirValue::WithUpdate {
            target, updates, ..
        } => {
            value_may_emit_float_arithmetic_error(target, locals)
                || updates
                    .iter()
                    .any(|update| value_may_emit_float_arithmetic_error(&update.value, locals))
        }
        NirValue::ListLiteral { values, .. } => values
            .iter()
            .any(|value| value_may_emit_float_arithmetic_error(value, locals)),
        NirValue::MapLiteral { entries, .. } => entries.iter().any(|(key, value)| {
            value_may_emit_float_arithmetic_error(key, locals)
                || value_may_emit_float_arithmetic_error(value, locals)
        }),
        NirValue::MemberAccess { target, .. } => {
            value_may_emit_float_arithmetic_error(target, locals)
        }
        NirValue::Unary { op, operand, .. } => {
            (op == "-" && static_nir_value_type(operand, locals).as_deref() == Some("Float"))
                || value_may_emit_float_arithmetic_error(operand, locals)
        }
        NirValue::Closure { captures, .. } => captures
            .iter()
            .any(|value| value_may_emit_float_arithmetic_error(value, locals)),
        NirValue::Const { .. }
        | NirValue::Local(_)
        | NirValue::LocalRef { .. }
        | NirValue::Global { .. }
        | NirValue::FunctionRef { .. }
        | NirValue::Capture { .. } => false,
    }
}

fn ops_use_call(ops: &[NirOp], target: &str) -> bool {
    ops.iter().any(|op| match op {
        NirOp::Bind { value, .. }
        | NirOp::StoreGlobal { value, .. }
        | NirOp::Return { value } => {
            value.as_ref().is_some_and(|value| value_uses_call(value, target))
        }
        NirOp::ExitLoop { .. } | NirOp::ContinueLoop { .. } => false,
        NirOp::ExitProgram { code } => value_uses_call(code, target),
        NirOp::Fail { error } => value_uses_call(error, target),
        NirOp::Assign { value, .. } | NirOp::StateAssign { value, .. } | NirOp::Eval { value } => value_uses_call(value, target),
        NirOp::If {
            condition,
            then_body,
            else_body,
        } => {
            value_uses_call(condition, target)
                || ops_use_call(then_body, target)
                || ops_use_call(else_body, target)
        }
        NirOp::Match { value, cases } => {
            value_uses_call(value, target)
                || cases.iter().any(|case| {
                    matches!(&case.pattern, NirMatchPattern::Value(value) if value_uses_call(value, target))
                        || ops_use_call(&case.body, target)
                })
        }
        NirOp::While { condition, body, .. } => {
            value_uses_call(condition, target) || ops_use_call(body, target)
        }
        NirOp::For {
            start,
            end,
            step,
            body,
            ..
        } => {
            value_uses_call(start, target)
                || value_uses_call(end, target)
                || value_uses_call(step, target)
                || ops_use_call(body, target)
        }
        NirOp::DoUntil { body, condition } => {
            ops_use_call(body, target) || value_uses_call(condition, target)
        }
        NirOp::ForEach { iterable, body, .. } => {
            value_uses_call(iterable, target) || ops_use_call(body, target)
        }
        NirOp::Trap { body, .. } => ops_use_call(body, target),
    })
}

fn value_uses_call(value: &NirValue, target: &str) -> bool {
    match value {
        NirValue::Call {
            target: call, args, ..
        }
        | NirValue::CallResult {
            target: call, args, ..
        }
        | NirValue::RuntimeCall {
            target: call, args, ..
        } => call == target || args.iter().any(|arg| value_uses_call(arg, target)),
        NirValue::Constructor { args, .. } => args.iter().any(|arg| value_uses_call(arg, target)),
        NirValue::UnionWrap { value, .. }
        | NirValue::UnionExtract { value, .. }
        | NirValue::ResultIsOk { value }
        | NirValue::ResultValue { value }
        | NirValue::ResultError { value } => value_uses_call(value, target),
        NirValue::WithUpdate {
            target: updated,
            updates,
            ..
        } => {
            value_uses_call(updated, target)
                || updates
                    .iter()
                    .any(|update| value_uses_call(&update.value, target))
        }
        NirValue::ListLiteral { values, .. } => {
            values.iter().any(|value| value_uses_call(value, target))
        }
        NirValue::MapLiteral { entries, .. } => entries
            .iter()
            .any(|(key, value)| value_uses_call(key, target) || value_uses_call(value, target)),
        NirValue::MemberAccess { target: value, .. } => value_uses_call(value, target),
        NirValue::Binary { left, right, .. } => {
            value_uses_call(left, target) || value_uses_call(right, target)
        }
        NirValue::Unary { operand, .. } => value_uses_call(operand, target),
        NirValue::Closure { captures, .. } => {
            captures.iter().any(|value| value_uses_call(value, target))
        }
        NirValue::Capture { .. }
        | NirValue::Const { .. }
        | NirValue::Local(_)
        | NirValue::LocalRef { .. }
        | NirValue::Global { .. }
        | NirValue::FunctionRef { .. } => false,
    }
}

fn ops_use_type_name(ops: &[NirOp]) -> bool {
    ops.iter().any(|op| match op {
        NirOp::Bind { value, .. } | NirOp::StoreGlobal { value, .. } | NirOp::Return { value } => {
            value.as_ref().is_some_and(value_uses_type_name)
        }
        NirOp::ExitLoop { .. } | NirOp::ContinueLoop { .. } => false,
        NirOp::ExitProgram { code } => value_uses_type_name(code),
        NirOp::Fail { error } => value_uses_type_name(error),
        NirOp::Assign { value, .. } | NirOp::StateAssign { value, .. } | NirOp::Eval { value } => {
            value_uses_type_name(value)
        }
        NirOp::If {
            condition,
            then_body,
            else_body,
        } => {
            value_uses_type_name(condition)
                || ops_use_type_name(then_body)
                || ops_use_type_name(else_body)
        }
        NirOp::Match { value, cases } => value_uses_type_name(value) || cases.iter().any(|case| {
            matches!(&case.pattern, NirMatchPattern::Value(value) if value_uses_type_name(value))
                || ops_use_type_name(&case.body)
        }),
        NirOp::While {
            condition, body, ..
        } => value_uses_type_name(condition) || ops_use_type_name(body),
        NirOp::For {
            start,
            end,
            step,
            body,
            ..
        } => {
            value_uses_type_name(start)
                || value_uses_type_name(end)
                || value_uses_type_name(step)
                || ops_use_type_name(body)
        }
        NirOp::DoUntil { body, condition } => {
            ops_use_type_name(body) || value_uses_type_name(condition)
        }
        NirOp::ForEach { iterable, body, .. } => {
            value_uses_type_name(iterable) || ops_use_type_name(body)
        }
        NirOp::Trap { body, .. } => ops_use_type_name(body),
    })
}

fn value_uses_type_name(value: &NirValue) -> bool {
    let direct = match value {
        NirValue::Call { target, .. }
        | NirValue::CallResult { target, .. }
        | NirValue::RuntimeCall { target, .. } => target == "typeName",
        _ => false,
    };
    direct
        || match value {
            NirValue::Call { args, .. }
            | NirValue::CallResult { args, .. }
            | NirValue::RuntimeCall { args, .. }
            | NirValue::Constructor { args, .. } => args.iter().any(value_uses_type_name),
            NirValue::UnionWrap {
                union_type,
                member_type,
                value,
            } => {
                let _ = (union_type, member_type);
                value_uses_type_name(value)
            }
            NirValue::UnionExtract { type_, value } => {
                let _ = type_;
                value_uses_type_name(value)
            }
            NirValue::ResultIsOk { value }
            | NirValue::ResultValue { value }
            | NirValue::ResultError { value } => value_uses_type_name(value),
            NirValue::WithUpdate {
                target, updates, ..
            } => {
                value_uses_type_name(target)
                    || updates
                        .iter()
                        .any(|update| value_uses_type_name(&update.value))
            }
            NirValue::ListLiteral { values, .. } => values.iter().any(value_uses_type_name),
            NirValue::MapLiteral { entries, .. } => entries
                .iter()
                .any(|(key, value)| value_uses_type_name(key) || value_uses_type_name(value)),
            NirValue::MemberAccess { target, .. } => value_uses_type_name(target),
            NirValue::Binary { left, right, .. } => {
                value_uses_type_name(left) || value_uses_type_name(right)
            }
            NirValue::Unary { operand, .. } => value_uses_type_name(operand),
            NirValue::Closure { captures, .. } => captures.iter().any(value_uses_type_name),
            NirValue::Capture { .. }
            | NirValue::Const { .. }
            | NirValue::Local(_)
            | NirValue::LocalRef { .. }
            | NirValue::Global { .. }
            | NirValue::FunctionRef { .. } => false,
        }
}

pub(super) fn module_uses_unicode_runtime_tables(module: &NirModule) -> bool {
    module.functions.iter().any(|function| {
        let mut constants = HashMap::new();
        let mut types = HashMap::new();
        ops_use_unicode_runtime_tables(&function.body, &mut constants, &mut types)
    })
}

fn ops_use_unicode_runtime_tables(
    ops: &[NirOp],
    constants: &mut HashMap<String, NirValue>,
    types: &mut HashMap<String, String>,
) -> bool {
    for op in ops {
        match op {
            NirOp::Bind {
                name, type_, value, ..
            } => {
                types.insert(name.clone(), type_.clone());
                if let Some(value) = value {
                    if value_uses_unicode_runtime_tables(value, constants, types) {
                        return true;
                    }
                    if let Some(constant) =
                        local_constant_value_with_constants(value, constants, types)
                    {
                        constants.insert(name.clone(), constant);
                    } else {
                        constants.remove(name);
                    }
                } else {
                    constants.remove(name);
                }
            }
            NirOp::StateAssign { value, .. } => {
                if value_uses_unicode_runtime_tables(value, constants, types) {
                    return true;
                }
            }
            NirOp::Assign { name, value } => {
                if value_uses_unicode_runtime_tables(value, constants, types) {
                    return true;
                }
                if let Some(constant) = local_constant_value_with_constants(value, constants, types)
                {
                    constants.insert(name.clone(), constant);
                } else {
                    constants.remove(name);
                }
            }
            NirOp::StoreGlobal { value, .. } => {
                if value
                    .as_ref()
                    .is_some_and(|value| value_uses_unicode_runtime_tables(value, constants, types))
                {
                    return true;
                }
            }
            NirOp::Eval { value } | NirOp::Fail { error: value } => {
                if value_uses_unicode_runtime_tables(value, constants, types) {
                    return true;
                }
            }
            NirOp::Return { value } => {
                if value
                    .as_ref()
                    .is_some_and(|value| value_uses_unicode_runtime_tables(value, constants, types))
                {
                    return true;
                }
            }
            NirOp::ExitLoop { .. } | NirOp::ContinueLoop { .. } => {}
            NirOp::ExitProgram { code } => {
                if value_uses_unicode_runtime_tables(code, constants, types) {
                    return true;
                }
            }
            NirOp::If {
                condition,
                then_body,
                else_body,
            } => {
                if value_uses_unicode_runtime_tables(condition, constants, types) {
                    return true;
                }
                let mut then_constants = constants.clone();
                let mut then_types = types.clone();
                let mut else_constants = constants.clone();
                let mut else_types = types.clone();
                if ops_use_unicode_runtime_tables(then_body, &mut then_constants, &mut then_types)
                    || ops_use_unicode_runtime_tables(
                        else_body,
                        &mut else_constants,
                        &mut else_types,
                    )
                {
                    return true;
                }
            }
            NirOp::Match { value, cases } => {
                if value_uses_unicode_runtime_tables(value, constants, types) {
                    return true;
                }
                for case in cases {
                    if let NirMatchPattern::Value(value) = &case.pattern {
                        if value_uses_unicode_runtime_tables(value, constants, types) {
                            return true;
                        }
                    }
                    let mut case_constants = constants.clone();
                    let mut case_types = types.clone();
                    if ops_use_unicode_runtime_tables(
                        &case.body,
                        &mut case_constants,
                        &mut case_types,
                    ) {
                        return true;
                    }
                }
            }
            NirOp::While {
                condition, body, ..
            } => {
                if value_uses_unicode_runtime_tables(condition, constants, types) {
                    return true;
                }
                let mut body_constants = constants.clone();
                let mut body_types = types.clone();
                if ops_use_unicode_runtime_tables(body, &mut body_constants, &mut body_types) {
                    return true;
                }
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
                if value_uses_unicode_runtime_tables(start, constants, types)
                    || value_uses_unicode_runtime_tables(end, constants, types)
                    || value_uses_unicode_runtime_tables(step, constants, types)
                {
                    return true;
                }
                let mut body_constants = constants.clone();
                let mut body_types = types.clone();
                body_constants.remove(name);
                body_types.insert(name.clone(), type_.clone());
                if ops_use_unicode_runtime_tables(body, &mut body_constants, &mut body_types) {
                    return true;
                }
            }
            NirOp::DoUntil { body, condition } => {
                let mut body_constants = constants.clone();
                let mut body_types = types.clone();
                if ops_use_unicode_runtime_tables(body, &mut body_constants, &mut body_types)
                    || value_uses_unicode_runtime_tables(condition, constants, types)
                {
                    return true;
                }
            }
            NirOp::ForEach {
                name,
                type_,
                iterable,
                body,
            } => {
                if value_uses_unicode_runtime_tables(iterable, constants, types) {
                    return true;
                }
                let mut body_constants = constants.clone();
                let mut body_types = types.clone();
                body_constants.remove(name);
                body_types.insert(name.clone(), type_.clone());
                if ops_use_unicode_runtime_tables(body, &mut body_constants, &mut body_types) {
                    return true;
                }
            }
            NirOp::Trap { body, .. } => {
                let mut trap_constants = constants.clone();
                let mut trap_types = types.clone();
                if ops_use_unicode_runtime_tables(body, &mut trap_constants, &mut trap_types) {
                    return true;
                }
            }
        }
    }
    false
}

fn value_uses_unicode_runtime_tables(
    value: &NirValue,
    constants: &HashMap<String, NirValue>,
    types: &HashMap<String, String>,
) -> bool {
    match value {
        NirValue::Call { target, args, .. }
        | NirValue::CallResult { target, args, .. }
        | NirValue::RuntimeCall { target, args, .. } => {
            matches!(
                target.as_str(),
                "strings.upper"
                    | "strings.lower"
                    | "strings.caseFold"
                    | "strings.normalizeNfc"
                    | "strings.graphemes"
                    | "strings.graphemeAt"
                    | "strings.graphemesCount"
            ) && !unicode_string_call_is_static(target, args, constants, types)
                || args
                    .iter()
                    .any(|arg| value_uses_unicode_runtime_tables(arg, constants, types))
        }
        NirValue::Constructor { args, .. } => args
            .iter()
            .any(|arg| value_uses_unicode_runtime_tables(arg, constants, types)),
        NirValue::UnionWrap { value, .. }
        | NirValue::UnionExtract { value, .. }
        | NirValue::ResultIsOk { value }
        | NirValue::ResultValue { value }
        | NirValue::ResultError { value } => {
            value_uses_unicode_runtime_tables(value, constants, types)
        }
        NirValue::WithUpdate {
            target, updates, ..
        } => {
            value_uses_unicode_runtime_tables(target, constants, types)
                || updates.iter().any(|update| {
                    value_uses_unicode_runtime_tables(&update.value, constants, types)
                })
        }
        NirValue::ListLiteral { values, .. } => values
            .iter()
            .any(|value| value_uses_unicode_runtime_tables(value, constants, types)),
        NirValue::MapLiteral { entries, .. } => entries.iter().any(|(key, value)| {
            value_uses_unicode_runtime_tables(key, constants, types)
                || value_uses_unicode_runtime_tables(value, constants, types)
        }),
        NirValue::MemberAccess { target, .. } => {
            value_uses_unicode_runtime_tables(target, constants, types)
        }
        NirValue::Binary { left, right, .. } => {
            value_uses_unicode_runtime_tables(left, constants, types)
                || value_uses_unicode_runtime_tables(right, constants, types)
        }
        NirValue::Unary { operand, .. } => {
            value_uses_unicode_runtime_tables(operand, constants, types)
        }
        NirValue::Closure { captures, .. } => captures
            .iter()
            .any(|value| value_uses_unicode_runtime_tables(value, constants, types)),
        NirValue::Capture { .. }
        | NirValue::Const { .. }
        | NirValue::Local(_)
        | NirValue::LocalRef { .. }
        | NirValue::Global { .. }
        | NirValue::FunctionRef { .. } => false,
    }
}

pub(super) fn value_may_return_invalid_format(
    value: &NirValue,
    constants: &HashMap<String, NirValue>,
    types: &HashMap<String, String>,
) -> bool {
    (match value {
        NirValue::Call { target, args, .. }
        | NirValue::CallResult { target, args, .. }
        | NirValue::RuntimeCall { target, args, .. } => match target.as_str() {
            "toInt" if args.len() == 1 => {
                static_type_name_with_types(&args[0], types).as_deref() != Some("Byte")
            }
            // The 2-arg `toInt(text, base)` form parses a String in a runtime
            // base; it FAILs `77050003` on an empty string, an out-of-range
            // base, or a digit invalid for the base (plan-02-cleanup §5).
            "toInt" if args.len() == 2 => true,
            "toFloat" | "toFixed" | "isNumeric" => true,
            _ => false,
        },
        _ => false,
    }) || match value {
        NirValue::Call { args, .. }
        | NirValue::CallResult { args, .. }
        | NirValue::RuntimeCall { args, .. }
        | NirValue::Constructor { args, .. } => args
            .iter()
            .any(|arg| value_may_return_invalid_format(arg, constants, types)),
        NirValue::UnionWrap { value, .. }
        | NirValue::UnionExtract { value, .. }
        | NirValue::ResultIsOk { value }
        | NirValue::ResultValue { value }
        | NirValue::ResultError { value } => {
            value_may_return_invalid_format(value, constants, types)
        }
        NirValue::WithUpdate {
            target, updates, ..
        } => {
            value_may_return_invalid_format(target, constants, types)
                || updates
                    .iter()
                    .any(|update| value_may_return_invalid_format(&update.value, constants, types))
        }
        NirValue::ListLiteral { values, .. } => values
            .iter()
            .any(|value| value_may_return_invalid_format(value, constants, types)),
        NirValue::MapLiteral { entries, .. } => entries.iter().any(|(key, value)| {
            value_may_return_invalid_format(key, constants, types)
                || value_may_return_invalid_format(value, constants, types)
        }),
        NirValue::MemberAccess { target, .. } => {
            value_may_return_invalid_format(target, constants, types)
        }
        NirValue::Binary {
            op, left, right, ..
        } => {
            binary_may_promote_float_to_fixed(op, left, right, types)
                || value_may_return_invalid_format(left, constants, types)
                || value_may_return_invalid_format(right, constants, types)
        }
        NirValue::Unary { operand, .. } => {
            value_may_return_invalid_format(operand, constants, types)
        }
        NirValue::Global { .. } => false,
        NirValue::Closure { captures, .. } => captures
            .iter()
            .any(|value| value_may_return_invalid_format(value, constants, types)),
        NirValue::Capture { .. }
        | NirValue::Const { .. }
        | NirValue::Local(_)
        | NirValue::LocalRef { .. }
        | NirValue::FunctionRef { .. } => false,
    }
}
