use super::*;

pub(crate) fn validate_capabilities(
    module: &NirModule,
    capabilities: &BackendCapabilities,
) -> Result<(), String> {
    let mut runtime_calls = Vec::new();
    for function in &module.functions {
        collect_runtime_calls_from_ops(&function.body, &mut runtime_calls);
    }
    for call in &runtime_calls {
        if runtime::is_native_direct_call(call) {
            continue;
        }
        if !capabilities.runtime_calls.contains(&call.as_str()) {
            return Err(format!(
                "native backend does not support runtime call '{call}'"
            ));
        }
    }
    for helper in &module.runtime_helpers {
        let helper_used_by_emitted_call = runtime_calls
            .iter()
            .any(|call| runtime::helper_for_call(call) == Some(*helper));
        if !helper_used_by_emitted_call {
            continue;
        }
        // A family is implemented when at least one catalogued spec exists for
        // it with a non-empty `returns`. The former `params`/`clobbers`
        // conditions went away with the fields themselves (bug-329): they were
        // unread transcriptions, and because this is an `any()` over the whole
        // family, a single sibling spec satisfied them anyway — they could
        // never detect an under-described helper. `catalog_is_consistent`
        // asserts every catalogued spec has a non-empty `returns`.
        let helper_supported = runtime::supported_helper_specs()
            .iter()
            .any(|spec| spec.helper == *helper && !spec.abi.returns.is_empty());
        if !helper_supported {
            return Err(format!(
                "native backend does not implement runtime helper '{}'",
                helper.name()
            ));
        }
    }
    Ok(())
}

/// Collect the type strings of every `Bind` op (recursively) so resource-union
/// binds can be matched against union type definitions.
pub(super) fn collect_bind_types(ops: &[NirOp], types: &mut HashSet<String>) {
    for op in ops {
        match op {
            NirOp::Bind { type_, .. } => {
                types.insert(type_.clone());
            }
            NirOp::If {
                then_body,
                else_body,
                ..
            } => {
                collect_bind_types(then_body, types);
                collect_bind_types(else_body, types);
            }
            NirOp::Match { cases, .. } => {
                for case in cases {
                    collect_bind_types(&case.body, types);
                }
            }
            NirOp::While { body, .. }
            | NirOp::For { body, .. }
            | NirOp::DoUntil { body, .. }
            | NirOp::ForEach { body, .. }
            | NirOp::Trap { body, .. } => {
                collect_bind_types(body, types);
            }
            _ => {}
        }
    }
}

pub(super) fn collect_runtime_calls_from_ops(ops: &[NirOp], calls: &mut Vec<String>) {
    let mut constants = HashMap::new();
    collect_runtime_calls_from_ops_with_constants(ops, calls, &mut constants);
}

/// The constant environment a loop body is analyzed under (bug-300 E12).
///
/// Empty, mirroring codegen: `builder_control` calls `clear_local_constants()`
/// before every loop body, because a local can be reassigned inside the body and a
/// loop-entry value therefore says nothing about later iterations. This pass used
/// to clone the enclosing constants instead and never invalidate anything, so a
/// call like `strings.upper(s)` folded away here while codegen emitted it for
/// real -- validate could clear a capability gate for a call the binary actually
/// makes. Clearing outright is exactly what codegen does, so the two now agree by
/// construction rather than by a second, parallel invalidation rule that could
/// drift.
pub(super) fn loop_body_constants() -> HashMap<String, NirValue> {
    HashMap::new()
}

pub(super) fn collect_runtime_calls_from_ops_with_constants(
    ops: &[NirOp],
    calls: &mut Vec<String>,
    constants: &mut HashMap<String, NirValue>,
) {
    for op in ops {
        match op {
            NirOp::Bind { name, value, .. } => {
                if let Some(value) = value {
                    collect_runtime_calls_from_value(value, calls, constants);
                    if let Some(constant) = native_constant_value(value, constants) {
                        constants.insert(name.clone(), constant);
                    } else {
                        constants.remove(name);
                    }
                } else {
                    constants.remove(name);
                }
            }
            NirOp::Return { value } => {
                if let Some(value) = value {
                    collect_runtime_calls_from_value(value, calls, constants);
                }
            }
            NirOp::ExitLoop { .. } | NirOp::ContinueLoop { .. } => {}
            NirOp::ExitProgram { code } => {
                collect_runtime_calls_from_value(code, calls, constants);
            }
            NirOp::Fail { error } => {
                collect_runtime_calls_from_value(error, calls, constants);
            }
            NirOp::StateAssign { value, .. } => {
                collect_runtime_calls_from_value(value, calls, constants);
            }
            NirOp::Assign { name, value } => {
                collect_runtime_calls_from_value(value, calls, constants);
                if let Some(constant) = native_constant_value(value, constants) {
                    constants.insert(name.clone(), constant);
                } else {
                    constants.remove(name);
                }
            }
            NirOp::StoreGlobal { value, .. } => {
                if let Some(value) = value {
                    collect_runtime_calls_from_value(value, calls, constants);
                }
            }
            NirOp::Eval { value } => {
                collect_runtime_calls_from_value(value, calls, constants);
            }
            NirOp::If {
                condition,
                then_body,
                else_body,
            } => {
                collect_runtime_calls_from_value(condition, calls, constants);
                let mut then_constants = constants.clone();
                let mut else_constants = constants.clone();
                collect_runtime_calls_from_ops_with_constants(
                    then_body,
                    calls,
                    &mut then_constants,
                );
                collect_runtime_calls_from_ops_with_constants(
                    else_body,
                    calls,
                    &mut else_constants,
                );
            }
            NirOp::Match { value, cases } => {
                collect_runtime_calls_from_value(value, calls, constants);
                for case in cases {
                    let mut case_constants = constants.clone();
                    collect_runtime_calls_from_ops_with_constants(
                        &case.body,
                        calls,
                        &mut case_constants,
                    );
                }
            }
            NirOp::While {
                condition, body, ..
            } => {
                collect_runtime_calls_from_value(condition, calls, constants);
                let mut body_constants = loop_body_constants();
                collect_runtime_calls_from_ops_with_constants(body, calls, &mut body_constants);
            }
            NirOp::For {
                start,
                end,
                step,
                body,
                ..
            } => {
                collect_runtime_calls_from_value(start, calls, constants);
                collect_runtime_calls_from_value(end, calls, constants);
                collect_runtime_calls_from_value(step, calls, constants);
                let mut body_constants = loop_body_constants();
                collect_runtime_calls_from_ops_with_constants(body, calls, &mut body_constants);
            }
            NirOp::DoUntil { body, condition } => {
                let mut body_constants = loop_body_constants();
                collect_runtime_calls_from_ops_with_constants(body, calls, &mut body_constants);
                collect_runtime_calls_from_value(condition, calls, constants);
            }
            NirOp::ForEach { iterable, body, .. } => {
                collect_runtime_calls_from_value(iterable, calls, constants);
                let mut body_constants = loop_body_constants();
                collect_runtime_calls_from_ops_with_constants(body, calls, &mut body_constants);
            }
            NirOp::Trap { body, .. } => {
                let mut trap_constants = constants.clone();
                collect_runtime_calls_from_ops_with_constants(body, calls, &mut trap_constants);
            }
        }
    }
}

pub(super) fn collect_runtime_calls_from_value(
    value: &NirValue,
    calls: &mut Vec<String>,
    constants: &HashMap<String, NirValue>,
) {
    match value {
        NirValue::RuntimeCall { target, args, .. } => {
            if target != "typeName"
                && native_static_string_value(value, constants).is_none()
                && native_static_graphemes_value(target, args, constants).is_none()
                && !calls.contains(target)
            {
                calls.push(target.clone());
            }
            for arg in args {
                collect_runtime_calls_from_value(arg, calls, constants);
            }
        }
        NirValue::Call { args, .. }
        | NirValue::CallResult { args, .. }
        | NirValue::Constructor { args, .. } => {
            for arg in args {
                collect_runtime_calls_from_value(arg, calls, constants);
            }
        }
        NirValue::UnionWrap { value, .. }
        | NirValue::UnionExtract { value, .. }
        | NirValue::ResultIsOk { value }
        | NirValue::ResultValue { value }
        | NirValue::ResultError { value } => {
            collect_runtime_calls_from_value(value, calls, constants);
        }
        NirValue::WithUpdate {
            target, updates, ..
        } => {
            collect_runtime_calls_from_value(target, calls, constants);
            for update in updates {
                collect_runtime_calls_from_value(&update.value, calls, constants);
            }
        }
        NirValue::ListLiteral { values, .. } => {
            for value in values {
                collect_runtime_calls_from_value(value, calls, constants);
            }
        }
        NirValue::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                collect_runtime_calls_from_value(key, calls, constants);
                collect_runtime_calls_from_value(value, calls, constants);
            }
        }
        NirValue::MemberAccess { target, .. } => {
            collect_runtime_calls_from_value(target, calls, constants)
        }
        NirValue::Binary { left, right, .. } => {
            collect_runtime_calls_from_value(left, calls, constants);
            collect_runtime_calls_from_value(right, calls, constants);
        }
        NirValue::Unary { operand, .. } => {
            collect_runtime_calls_from_value(operand, calls, constants)
        }
        NirValue::Closure { captures, .. } => {
            for value in captures {
                collect_runtime_calls_from_value(value, calls, constants);
            }
        }
        NirValue::Capture { .. }
        | NirValue::Const { .. }
        | NirValue::Local(_)
        | NirValue::LocalRef { .. }
        | NirValue::Global { .. }
        | NirValue::FunctionRef { .. } => {}
    }
}

pub(super) fn native_constant_value(
    value: &NirValue,
    constants: &HashMap<String, NirValue>,
) -> Option<NirValue> {
    match value {
        NirValue::Const { .. } => Some(value.clone()),
        NirValue::Local(name) => constants.get(name).cloned(),
        NirValue::Global { .. } => None,
        NirValue::Call { target, args, .. } if target == "toString" && args.len() == 1 => {
            native_primitive_text(&args[0], constants).map(|value| NirValue::Const {
                type_: "String".to_string(),
                value,
            })
        }
        NirValue::RuntimeCall { target, args, .. } if target == "toString" && args.len() == 1 => {
            native_primitive_text(&args[0], constants).map(|value| NirValue::Const {
                type_: "String".to_string(),
                value,
            })
        }
        NirValue::Binary { op, .. } if op == "&" => native_static_string_value(value, constants)
            .map(|value| NirValue::Const {
                type_: "String".to_string(),
                value,
            }),
        _ => None,
    }
}

pub(super) fn native_static_string_value(
    value: &NirValue,
    constants: &HashMap<String, NirValue>,
) -> Option<String> {
    match value {
        NirValue::Const { type_, value } if type_ == "String" => Some(value.clone()),
        NirValue::Local(name) => constants
            .get(name)
            .and_then(|constant| native_static_string_value(constant, constants)),
        NirValue::Global { .. } => None,
        NirValue::Call { target, args, .. } if target == "toString" && args.len() == 1 => {
            native_primitive_text(&args[0], constants)
        }
        NirValue::RuntimeCall { target, args, .. } if target == "toString" && args.len() == 1 => {
            native_primitive_text(&args[0], constants)
        }
        NirValue::Call { target, args, .. } | NirValue::RuntimeCall { target, args, .. } => {
            native_strings_package_static_string_value(target, args, constants)
        }
        NirValue::Binary {
            op, left, right, ..
        } if op == "&" => {
            let left = native_static_string_value(left, constants)?;
            let right = native_static_string_value(right, constants)?;
            Some(format!("{left}{right}"))
        }
        _ => None,
    }
}

pub(super) fn native_strings_package_static_string_value(
    target: &str,
    args: &[NirValue],
    constants: &HashMap<String, NirValue>,
) -> Option<String> {
    let value = args
        .first()
        .and_then(|arg| native_static_string_value(arg, constants))?;
    match target {
        "strings.upper" if args.len() == 1 => Some(crate::unicode_backend::upper(&value)),
        "strings.lower" if args.len() == 1 => Some(crate::unicode_backend::lower(&value)),
        "strings.caseFold" if args.len() == 1 => Some(crate::unicode_backend::case_fold(&value)),
        "strings.normalizeNfc" if args.len() == 1 => {
            Some(crate::unicode_backend::normalize_nfc(&value))
        }
        _ => None,
    }
}

pub(super) fn native_static_graphemes_value(
    target: &str,
    args: &[NirValue],
    constants: &HashMap<String, NirValue>,
) -> Option<Vec<String>> {
    if target != "strings.graphemes" || args.len() != 1 {
        return None;
    }
    let value = native_static_string_value(&args[0], constants)?;
    Some(crate::unicode_backend::graphemes(&value))
}

pub(super) fn native_primitive_text(
    value: &NirValue,
    constants: &HashMap<String, NirValue>,
) -> Option<String> {
    match value {
        NirValue::Const { type_, value } => match type_.as_str() {
            // Float/Fixed constants fold to the runtime formatter's
            // default-precision rendering (2 places; bug-358, plan-28-B).
            "Float" | "Fixed" => crate::numeric::default_to_string_text(type_, value),
            "Integer" | "Byte" | "String" => Some(value.clone()),
            "Boolean" => match value.as_str() {
                "true" => Some("TRUE".to_string()),
                "false" => Some("FALSE".to_string()),
                _ => None,
            },
            _ => None,
        },
        NirValue::Local(name) => constants
            .get(name)
            .and_then(|constant| native_primitive_text(constant, constants)),
        NirValue::Global { .. } => None,
        _ => None,
    }
}
