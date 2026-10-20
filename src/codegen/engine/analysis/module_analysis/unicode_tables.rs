//! plan-176-U: whether a module needs the Unicode runtime tables
//! (`module_uses_unicode_runtime_tables`). The other module analyses live in
//! `engine/analysis/module_analysis/mod.rs`.

use super::*;

pub(crate) fn module_uses_unicode_runtime_tables(module: &NirModule) -> bool {
    let fields = module_field_types(module);
    module.functions.iter().any(|function| {
        let mut constants = HashMap::new();
        let mut types = HashMap::new();
        ops_use_unicode_runtime_tables(&function.body, &mut constants, &mut types, &fields)
    })
}

fn ops_use_unicode_runtime_tables(
    ops: &[NirOp],
    constants: &mut HashMap<String, NirValue>,
    types: &mut HashMap<String, ParameterType>,
    fields: &FieldTypes,
) -> bool {
    for op in ops {
        match op {
            NirOp::Bind {
                name, type_, value, ..
            } => {
                types.insert(name.clone(), type_.clone());
                if let Some(value) = value {
                    if value_uses_unicode_runtime_tables(value, constants, types, fields) {
                        return true;
                    }
                    if let Some(constant) =
                        local_constant_value_with_constants(value, constants, types, fields)
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
                if value_uses_unicode_runtime_tables(value, constants, types, fields) {
                    return true;
                }
            }
            NirOp::Assign { name, value } => {
                if value_uses_unicode_runtime_tables(value, constants, types, fields) {
                    return true;
                }
                if let Some(constant) =
                    local_constant_value_with_constants(value, constants, types, fields)
                {
                    constants.insert(name.clone(), constant);
                } else {
                    constants.remove(name);
                }
            }
            NirOp::StoreGlobal { value, .. } => {
                if value.as_ref().is_some_and(|value| {
                    value_uses_unicode_runtime_tables(value, constants, types, fields)
                }) {
                    return true;
                }
            }
            NirOp::Eval { value } | NirOp::Fail { error: value } => {
                if value_uses_unicode_runtime_tables(value, constants, types, fields) {
                    return true;
                }
            }
            NirOp::Return { value } => {
                if value.as_ref().is_some_and(|value| {
                    value_uses_unicode_runtime_tables(value, constants, types, fields)
                }) {
                    return true;
                }
            }
            NirOp::ExitLoop { .. } | NirOp::ContinueLoop { .. } => {}
            NirOp::ExitProgram { code } => {
                if value_uses_unicode_runtime_tables(code, constants, types, fields) {
                    return true;
                }
            }
            NirOp::If {
                condition,
                then_body,
                else_body,
            } => {
                if value_uses_unicode_runtime_tables(condition, constants, types, fields) {
                    return true;
                }
                let mut then_constants = constants.clone();
                let mut then_types = types.clone();
                let mut else_constants = constants.clone();
                let mut else_types = types.clone();
                if ops_use_unicode_runtime_tables(
                    then_body,
                    &mut then_constants,
                    &mut then_types,
                    fields,
                ) || ops_use_unicode_runtime_tables(
                    else_body,
                    &mut else_constants,
                    &mut else_types,
                    fields,
                ) {
                    return true;
                }
            }
            NirOp::Match { value, cases } => {
                if value_uses_unicode_runtime_tables(value, constants, types, fields) {
                    return true;
                }
                for case in cases {
                    if let NirMatchPattern::Value(value) = &case.pattern {
                        if value_uses_unicode_runtime_tables(value, constants, types, fields) {
                            return true;
                        }
                    }
                    let mut case_constants = constants.clone();
                    let mut case_types = types.clone();
                    if ops_use_unicode_runtime_tables(
                        &case.body,
                        &mut case_constants,
                        &mut case_types,
                        fields,
                    ) {
                        return true;
                    }
                }
            }
            NirOp::While {
                condition, body, ..
            } => {
                if value_uses_unicode_runtime_tables(condition, constants, types, fields) {
                    return true;
                }
                let mut body_constants = constants.clone();
                let mut body_types = types.clone();
                if ops_use_unicode_runtime_tables(
                    body,
                    &mut body_constants,
                    &mut body_types,
                    fields,
                ) {
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
                if value_uses_unicode_runtime_tables(start, constants, types, fields)
                    || value_uses_unicode_runtime_tables(end, constants, types, fields)
                    || value_uses_unicode_runtime_tables(step, constants, types, fields)
                {
                    return true;
                }
                let mut body_constants = constants.clone();
                let mut body_types = types.clone();
                body_constants.remove(name);
                body_types.insert(name.clone(), type_.clone());
                if ops_use_unicode_runtime_tables(
                    body,
                    &mut body_constants,
                    &mut body_types,
                    fields,
                ) {
                    return true;
                }
            }
            NirOp::DoUntil { body, condition } => {
                let mut body_constants = constants.clone();
                let mut body_types = types.clone();
                if ops_use_unicode_runtime_tables(
                    body,
                    &mut body_constants,
                    &mut body_types,
                    fields,
                ) || value_uses_unicode_runtime_tables(condition, constants, types, fields)
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
                if value_uses_unicode_runtime_tables(iterable, constants, types, fields) {
                    return true;
                }
                let mut body_constants = constants.clone();
                let mut body_types = types.clone();
                body_constants.remove(name);
                body_types.insert(name.clone(), type_.clone());
                if ops_use_unicode_runtime_tables(
                    body,
                    &mut body_constants,
                    &mut body_types,
                    fields,
                ) {
                    return true;
                }
            }
            NirOp::Trap { body, .. } => {
                let mut trap_constants = constants.clone();
                let mut trap_types = types.clone();
                if ops_use_unicode_runtime_tables(
                    body,
                    &mut trap_constants,
                    &mut trap_types,
                    fields,
                ) {
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
    types: &HashMap<String, ParameterType>,
    fields: &FieldTypes,
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
                    | "term.measureText"
                    | "term.measureTextRaw"
                    | "term.scalarWidthRaw"
            ) && !unicode_string_call_is_static(target, args, constants, types, fields)
                || args
                    .iter()
                    .any(|arg| value_uses_unicode_runtime_tables(arg, constants, types, fields))
        }
        NirValue::Constructor { args, .. } => args
            .iter()
            .any(|arg| value_uses_unicode_runtime_tables(arg, constants, types, fields)),
        NirValue::UnionWrap { value, .. }
        | NirValue::UnionExtract { value, .. }
        | NirValue::ResultIsOk { value }
        | NirValue::ResultValue { value }
        | NirValue::ResultError { value }
        | NirValue::Checked { value, .. } => {
            value_uses_unicode_runtime_tables(value, constants, types, fields)
        }
        NirValue::WithUpdate {
            target, updates, ..
        } => {
            value_uses_unicode_runtime_tables(target, constants, types, fields)
                || updates.iter().any(|update| {
                    value_uses_unicode_runtime_tables(&update.value, constants, types, fields)
                })
        }
        NirValue::ListLiteral { values, .. } | NirValue::SetLiteral { values, .. } => values
            .iter()
            .any(|value| value_uses_unicode_runtime_tables(value, constants, types, fields)),
        NirValue::MapLiteral { entries, .. } => entries.iter().any(|(key, value)| {
            value_uses_unicode_runtime_tables(key, constants, types, fields)
                || value_uses_unicode_runtime_tables(value, constants, types, fields)
        }),
        NirValue::MemberAccess { target, .. } => {
            value_uses_unicode_runtime_tables(target, constants, types, fields)
        }
        NirValue::Binary { left, right, .. } => {
            value_uses_unicode_runtime_tables(left, constants, types, fields)
                || value_uses_unicode_runtime_tables(right, constants, types, fields)
        }
        NirValue::Unary { operand, .. } => {
            value_uses_unicode_runtime_tables(operand, constants, types, fields)
        }
        NirValue::Closure { captures, .. } => captures
            .iter()
            .any(|value| value_uses_unicode_runtime_tables(value, constants, types, fields)),
        NirValue::Capture { .. }
        | NirValue::Const { .. }
        | NirValue::Local(_)
        | NirValue::LocalRef { .. }
        | NirValue::Global { .. }
        | NirValue::FunctionRef { .. } => false,
    }
}
