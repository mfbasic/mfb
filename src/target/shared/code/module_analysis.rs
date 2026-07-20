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
        // A `RES` binding always supplies a value (the handle), but codegen
        // still default-initializes its `STATE` payload at the bind
        // (`emit_resource_state_init`, `builder_control.rs`), independently of
        // where the handle came from. So a `STATE` record carrying a `String`
        // demands the sentinel even though `value` is `Some` — checking only
        // `value: None` left the relocation dangling (bug-256, bug-05 class).
        NirOp::Bind { type_, value, .. } => {
            crate::builtins::resource::state_type_name(type_).is_some_and(|state| {
                type_requires_empty_string_constant(state, type_model, &mut HashSet::new())
            }) || (value.is_none()
                && type_requires_empty_string_constant(type_, type_model, &mut HashSet::new()))
        }
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
        // The remaining ops carry no loop/branch bodies to descend into.
        // Enumerated exhaustively (no `_ => false`) so a future `NirOp` variant
        // with a body cannot silently regress this analysis — the same gap that
        // produced bug-45 and bug-67.
        NirOp::StoreGlobal { .. }
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
pub(super) fn module_uses_migrated(module: &NirModule, bare: &str) -> bool {
    module_uses_call(module, &format!("collections.{bare}"))
        || module_uses_call(module, &format!("strings.{bare}"))
}

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

/// Declared field types of every record, union variant, and builtin `net` record
/// in `module`, keyed `(owning type name, field name)`.
///
/// This mirrors the `record_fields` + `union_variant_fields` halves of
/// `TypeModel::from_module` — the tables the *builder* consults when it lowers a
/// member read. A module-level predicate that walks values without them cannot
/// type a `NirValue::MemberAccess` at all, so its model of the program is
/// strictly weaker than the builder's and it under-reports (bug-363). Rebuilt
/// here rather than taking a `TypeModel` because the string-symbol pass runs
/// before the builder's type model is constructed.
pub(super) fn module_field_types(module: &NirModule) -> FieldTypes {
    let mut fields = FieldTypes::new();
    for type_ in &module.types {
        match type_.kind.as_str() {
            "type" | "record" => {
                for field in &type_.fields {
                    fields.insert(
                        (type_.name.clone(), field.name.clone()),
                        field.type_.clone(),
                    );
                }
            }
            "union" => {
                for variant in expanded_nir_union_variants(module, &type_.name) {
                    for field in &variant.fields {
                        fields.insert(
                            (variant.name.clone(), field.name.clone()),
                            field.type_.clone(),
                        );
                    }
                }
            }
            _ => {}
        }
    }
    for type_name in ["Address", "Datagram", "DatagramText"] {
        if let Some(builtin_fields) = builtins::net::builtin_type_fields(type_name) {
            for (name, type_) in builtin_fields {
                fields.insert((type_name.to_string(), name.to_string()), type_.to_string());
            }
        }
    }
    fields
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
    let fields = module_field_types(module);
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
    locals: &mut HashMap<String, String>,
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
                if !type_.is_empty() {
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
                type_ == "Float"
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
    locals: &HashMap<String, String>,
    fields: &FieldTypes,
) -> bool {
    match value {
        NirValue::Binary {
            op, left, right, ..
        } => {
            let result_type = static_nir_value_type(left, locals, fields)
                .zip(static_nir_value_type(right, locals, fields))
                .map(|(left_type, right_type)| {
                    numeric_binary_result_type(op, &left_type, &right_type)
                });
            (matches!(op.as_str(), "+" | "-" | "*" | "/" | "DIV" | "MOD" | "^")
                && result_type == Some("Float"))
                || value_may_emit_float_arithmetic_error(left, locals, fields)
                || value_may_emit_float_arithmetic_error(right, locals, fields)
        }
        NirValue::Call { args, .. }
        | NirValue::CallResult { args, .. }
        | NirValue::RuntimeCall { args, .. }
        | NirValue::Constructor { args, .. } => args
            .iter()
            .any(|arg| value_may_emit_float_arithmetic_error(arg, locals, fields)),
        NirValue::UnionWrap { value, .. }
        | NirValue::UnionExtract { value, .. }
        | NirValue::ResultIsOk { value }
        | NirValue::ResultValue { value }
        | NirValue::ResultError { value } => {
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
        NirValue::ListLiteral { values, .. } => values
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
            (op == "-"
                && static_nir_value_type(operand, locals, fields).as_deref() == Some("Float"))
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

/// Collect the `(name, type_)` of every `FunctionRef` in the module — the
/// no-capture function values. Each gets ONE static closure descriptor
/// (`{code, env=0}`) so a `FunctionRef` loads that descriptor's address instead of
/// arena-allocating a fresh descriptor on every evaluation (bug-78). Exhaustive:
/// a missed ref would reference an undefined descriptor symbol at link time.
pub(super) fn collect_function_value_refs(module: &NirModule) -> Vec<(String, String)> {
    let mut refs = Vec::new();
    for function in &module.functions {
        collect_ops_function_refs(&function.body, &mut refs);
    }
    refs
}

fn collect_ops_function_refs(ops: &[NirOp], out: &mut Vec<(String, String)>) {
    for op in ops {
        match op {
            NirOp::Bind { value, .. }
            | NirOp::StoreGlobal { value, .. }
            | NirOp::Return { value } => {
                if let Some(value) = value {
                    collect_value_function_refs(value, out);
                }
            }
            NirOp::ExitLoop { .. } | NirOp::ContinueLoop { .. } => {}
            NirOp::ExitProgram { code } => collect_value_function_refs(code, out),
            NirOp::Fail { error } => collect_value_function_refs(error, out),
            NirOp::Assign { value, .. }
            | NirOp::StateAssign { value, .. }
            | NirOp::Eval { value } => collect_value_function_refs(value, out),
            NirOp::If {
                condition,
                then_body,
                else_body,
            } => {
                collect_value_function_refs(condition, out);
                collect_ops_function_refs(then_body, out);
                collect_ops_function_refs(else_body, out);
            }
            NirOp::Match { value, cases } => {
                collect_value_function_refs(value, out);
                for case in cases {
                    if let NirMatchPattern::Value(v) = &case.pattern {
                        collect_value_function_refs(v, out);
                    }
                    collect_ops_function_refs(&case.body, out);
                }
            }
            NirOp::While {
                condition, body, ..
            } => {
                collect_value_function_refs(condition, out);
                collect_ops_function_refs(body, out);
            }
            NirOp::For {
                start,
                end,
                step,
                body,
                ..
            } => {
                collect_value_function_refs(start, out);
                collect_value_function_refs(end, out);
                collect_value_function_refs(step, out);
                collect_ops_function_refs(body, out);
            }
            NirOp::DoUntil { body, condition } => {
                collect_ops_function_refs(body, out);
                collect_value_function_refs(condition, out);
            }
            NirOp::ForEach { iterable, body, .. } => {
                collect_value_function_refs(iterable, out);
                collect_ops_function_refs(body, out);
            }
            NirOp::Trap { body, .. } => collect_ops_function_refs(body, out),
        }
    }
}

fn collect_value_function_refs(value: &NirValue, out: &mut Vec<(String, String)>) {
    match value {
        NirValue::FunctionRef { name, type_ } => out.push((name.clone(), type_.clone())),
        NirValue::Call { args, .. }
        | NirValue::CallResult { args, .. }
        | NirValue::RuntimeCall { args, .. } => {
            for arg in args {
                collect_value_function_refs(arg, out);
            }
        }
        NirValue::Constructor { args, .. } => {
            for arg in args {
                collect_value_function_refs(arg, out);
            }
        }
        NirValue::UnionWrap { value, .. }
        | NirValue::UnionExtract { value, .. }
        | NirValue::ResultIsOk { value }
        | NirValue::ResultValue { value }
        | NirValue::ResultError { value } => collect_value_function_refs(value, out),
        NirValue::WithUpdate {
            target, updates, ..
        } => {
            collect_value_function_refs(target, out);
            for update in updates {
                collect_value_function_refs(&update.value, out);
            }
        }
        NirValue::ListLiteral { values, .. } => {
            for value in values {
                collect_value_function_refs(value, out);
            }
        }
        NirValue::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                collect_value_function_refs(key, out);
                collect_value_function_refs(value, out);
            }
        }
        NirValue::MemberAccess { target, .. } => collect_value_function_refs(target, out),
        NirValue::Binary { left, right, .. } => {
            collect_value_function_refs(left, out);
            collect_value_function_refs(right, out);
        }
        NirValue::Unary { operand, .. } => collect_value_function_refs(operand, out),
        NirValue::Closure { captures, .. } => {
            for value in captures {
                collect_value_function_refs(value, out);
            }
        }
        NirValue::Capture { .. }
        | NirValue::Const { .. }
        | NirValue::Local(_)
        | NirValue::LocalRef { .. }
        | NirValue::Global { .. } => {}
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
            // `toInt(Byte)` and `toInt(Scalar)` are infallible width-preserving
            // moves; every other 1-arg form can fail.
            "toInt" if args.len() == 1 => !matches!(
                static_type_name_with_types(&args[0], types).as_deref(),
                Some("Byte") | Some("Scalar")
            ),
            // The 2-arg `toInt(text, base)` form parses a String in a runtime
            // base; it FAILs `77050003` on an empty string, an out-of-range
            // base, or a digit invalid for the base (plan-02-cleanup §5).
            "toInt" if args.len() == 2 => true,
            "toFloat" | "toFixed" | "isNumeric" => true,
            // `toMoney(String)` (malformed) and `toMoney(Float)` (NaN/Inf) FAIL
            // with ErrInvalidFormat (plan-29-G §4.2). Register the message for any
            // `toMoney` — the Integer/Byte/Fixed overloads simply never read the
            // (harmlessly interned) data object.
            "toMoney" if args.len() == 1 => true,
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
