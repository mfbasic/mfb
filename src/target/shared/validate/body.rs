use super::*;

pub(super) fn validate_entry(
    module: &NirModule,
    function_names: &HashSet<String>,
) -> Result<(), String> {
    let entry = module
        .entry
        .as_ref()
        .ok_or_else(|| "executable NIR requires an entry point".to_string())?;
    if !function_names.contains(&entry.name) {
        return Err(format!(
            "NIR entry point '{}' does not resolve to a function",
            entry.name
        ));
    }
    let function = module
        .functions
        .iter()
        .find(|function| function.name == entry.name)
        .expect("entry name checked above");
    if function.returns != entry.returns {
        return Err(format!(
            "NIR entry point '{}' return type '{}' does not match function return type '{}'",
            entry.name, entry.returns, function.returns
        ));
    }
    if !entry.accepts_args && !function.params.is_empty() {
        return Err(format!(
            "NIR entry point '{}' does not accept args but function has parameters",
            entry.name
        ));
    }
    Ok(())
}

pub(super) fn validate_function(
    function: &NirFunction,
    function_names: &HashSet<String>,
    global_names: &HashSet<String>,
    import_names: &HashSet<String>,
    type_value_names: &TypeValueNames,
    used_helpers: &mut Vec<RuntimeHelper>,
) -> Result<(), String> {
    let mut locals = HashMap::new();
    for param in &function.params {
        validate_param(function, param, &mut locals)?;
        if let Some(default) = &param.default {
            validate_value(
                default,
                &locals,
                function_names,
                global_names,
                import_names,
                type_value_names,
                used_helpers,
            )?;
        }
    }
    validate_ops(
        &function.body,
        &mut locals,
        function_names,
        global_names,
        import_names,
        type_value_names,
        used_helpers,
    )
}

pub(super) fn validate_param(
    function: &NirFunction,
    param: &NirParam,
    locals: &mut HashMap<String, LocalBinding>,
) -> Result<(), String> {
    if param.name.is_empty() || param.type_.is_empty() {
        return Err(format!(
            "NIR function '{}' has a parameter with empty name or type",
            function.name
        ));
    }
    if locals
        .insert(
            param.name.clone(),
            LocalBinding {
                mutable: false,
                type_: param.type_.clone(),
            },
        )
        .is_some()
    {
        return Err(format!(
            "NIR function '{}' has duplicate local '{}'",
            function.name, param.name
        ));
    }
    Ok(())
}

pub(super) fn validate_ops(
    ops: &[NirOp],
    locals: &mut HashMap<String, LocalBinding>,
    function_names: &HashSet<String>,
    global_names: &HashSet<String>,
    import_names: &HashSet<String>,
    type_value_names: &TypeValueNames,
    used_helpers: &mut Vec<RuntimeHelper>,
) -> Result<(), String> {
    for op in ops {
        match op {
            NirOp::Bind {
                name,
                type_,
                value,
                mutable,
            } => {
                if name.is_empty() || type_.is_empty() {
                    return Err("NIR bind op has empty name or type".to_string());
                }
                if let Some(value) = value {
                    validate_value(
                        value,
                        locals,
                        function_names,
                        global_names,
                        import_names,
                        type_value_names,
                        used_helpers,
                    )?;
                }
                if locals
                    .insert(
                        name.clone(),
                        LocalBinding {
                            mutable: *mutable,
                            type_: type_.clone(),
                        },
                    )
                    .is_some()
                {
                    return Err(format!("NIR local '{}' is declared more than once", name));
                }
            }
            NirOp::Assign { name, value } => {
                let local = locals
                    .get(name)
                    .ok_or_else(|| format!("NIR assignment targets unknown local '{name}'"))?;
                if !local.mutable {
                    return Err(format!("NIR assignment targets immutable local '{name}'"));
                }
                validate_value(
                    value,
                    locals,
                    function_names,
                    global_names,
                    import_names,
                    type_value_names,
                    used_helpers,
                )?;
            }
            NirOp::StateAssign { resource, value } => {
                if !locals.contains_key(resource) {
                    return Err(format!(
                        "NIR state assignment targets unknown local '{resource}'"
                    ));
                }
                validate_value(
                    value,
                    locals,
                    function_names,
                    global_names,
                    import_names,
                    type_value_names,
                    used_helpers,
                )?;
            }
            NirOp::StoreGlobal { name, value, .. } => {
                if !global_names.contains(name) {
                    return Err(format!("NIR global store targets unknown global '{name}'"));
                }
                if let Some(value) = value {
                    validate_value(
                        value,
                        locals,
                        function_names,
                        global_names,
                        import_names,
                        type_value_names,
                        used_helpers,
                    )?;
                }
            }
            NirOp::Return { value } => {
                if let Some(value) = value {
                    validate_value(
                        value,
                        locals,
                        function_names,
                        global_names,
                        import_names,
                        type_value_names,
                        used_helpers,
                    )?;
                }
            }
            NirOp::ExitLoop { .. } | NirOp::ContinueLoop { .. } => {}
            NirOp::ExitProgram { code } => {
                validate_value(
                    code,
                    locals,
                    function_names,
                    global_names,
                    import_names,
                    type_value_names,
                    used_helpers,
                )?;
            }
            NirOp::Fail { error } => {
                validate_value(
                    error,
                    locals,
                    function_names,
                    global_names,
                    import_names,
                    type_value_names,
                    used_helpers,
                )?;
            }
            NirOp::Eval { value } => {
                validate_value(
                    value,
                    locals,
                    function_names,
                    global_names,
                    import_names,
                    type_value_names,
                    used_helpers,
                )?;
            }
            NirOp::If {
                condition,
                then_body,
                else_body,
            } => {
                validate_value(
                    condition,
                    locals,
                    function_names,
                    global_names,
                    import_names,
                    type_value_names,
                    used_helpers,
                )?;
                let mut then_locals = locals.clone();
                validate_ops(
                    then_body,
                    &mut then_locals,
                    function_names,
                    global_names,
                    import_names,
                    type_value_names,
                    used_helpers,
                )?;
                let mut else_locals = locals.clone();
                validate_ops(
                    else_body,
                    &mut else_locals,
                    function_names,
                    global_names,
                    import_names,
                    type_value_names,
                    used_helpers,
                )?;
            }
            NirOp::Match { value, cases } => {
                validate_value(
                    value,
                    locals,
                    function_names,
                    global_names,
                    import_names,
                    type_value_names,
                    used_helpers,
                )?;
                for case in cases {
                    match &case.pattern {
                        NirMatchPattern::Else => {}
                        NirMatchPattern::Value(value) => {
                            validate_value(
                                value,
                                locals,
                                function_names,
                                global_names,
                                import_names,
                                type_value_names,
                                used_helpers,
                            )?;
                        }
                        NirMatchPattern::OneOf(values) => {
                            for value in values {
                                validate_value(
                                    value,
                                    locals,
                                    function_names,
                                    global_names,
                                    import_names,
                                    type_value_names,
                                    used_helpers,
                                )?;
                            }
                        }
                    }
                    let mut guard_locals = locals.clone();
                    let mut body_start = 0;
                    for op in &case.body {
                        let NirOp::Bind {
                            name,
                            type_,
                            value: Some(extract @ NirValue::UnionExtract { .. }),
                            ..
                        } = op
                        else {
                            break;
                        };
                        // bug-300 E13: these leading extract binds were added to
                        // scope but their VALUES were never validated -- only
                        // `case.body[body_start..]` is, and these sit before it. So
                        // the one class of expression this backstop exists to catch
                        // (hand-crafted or corrupted NIR) escaped it here. Validate
                        // against the locals accumulated so far, before this bind's
                        // own name enters scope.
                        validate_value(
                            extract,
                            &guard_locals,
                            function_names,
                            global_names,
                            import_names,
                            type_value_names,
                            used_helpers,
                        )?;
                        guard_locals.insert(
                            name.clone(),
                            LocalBinding {
                                type_: type_.clone(),
                                mutable: false,
                            },
                        );
                        body_start += 1;
                    }
                    if let Some(guard) = &case.guard {
                        validate_value(
                            guard,
                            &guard_locals,
                            function_names,
                            global_names,
                            import_names,
                            type_value_names,
                            used_helpers,
                        )?;
                    }
                    let mut case_locals = guard_locals;
                    validate_ops(
                        &case.body[body_start..],
                        &mut case_locals,
                        function_names,
                        global_names,
                        import_names,
                        type_value_names,
                        used_helpers,
                    )?;
                }
            }
            NirOp::ForEach {
                name,
                type_,
                iterable,
                body,
            } => {
                if name.is_empty() || type_.is_empty() {
                    return Err("NIR forEach op has empty name or type".to_string());
                }
                validate_value(
                    iterable,
                    locals,
                    function_names,
                    global_names,
                    import_names,
                    type_value_names,
                    used_helpers,
                )?;
                let mut body_locals = locals.clone();
                body_locals.insert(
                    name.clone(),
                    LocalBinding {
                        mutable: false,
                        type_: type_.clone(),
                    },
                );
                validate_ops(
                    body,
                    &mut body_locals,
                    function_names,
                    global_names,
                    import_names,
                    type_value_names,
                    used_helpers,
                )?;
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
                if name.is_empty() || type_.is_empty() {
                    return Err("NIR for op has empty name or type".to_string());
                }
                validate_value(
                    start,
                    locals,
                    function_names,
                    global_names,
                    import_names,
                    type_value_names,
                    used_helpers,
                )?;
                validate_value(
                    end,
                    locals,
                    function_names,
                    global_names,
                    import_names,
                    type_value_names,
                    used_helpers,
                )?;
                validate_value(
                    step,
                    locals,
                    function_names,
                    global_names,
                    import_names,
                    type_value_names,
                    used_helpers,
                )?;
                let mut body_locals = locals.clone();
                body_locals.insert(
                    name.clone(),
                    LocalBinding {
                        mutable: true,
                        type_: type_.clone(),
                    },
                );
                validate_ops(
                    body,
                    &mut body_locals,
                    function_names,
                    global_names,
                    import_names,
                    type_value_names,
                    used_helpers,
                )?;
            }
            NirOp::While {
                condition, body, ..
            } => {
                validate_value(
                    condition,
                    locals,
                    function_names,
                    global_names,
                    import_names,
                    type_value_names,
                    used_helpers,
                )?;
                let mut body_locals = locals.clone();
                validate_ops(
                    body,
                    &mut body_locals,
                    function_names,
                    global_names,
                    import_names,
                    type_value_names,
                    used_helpers,
                )?;
            }
            NirOp::DoUntil { body, condition } => {
                let mut body_locals = locals.clone();
                validate_ops(
                    body,
                    &mut body_locals,
                    function_names,
                    global_names,
                    import_names,
                    type_value_names,
                    used_helpers,
                )?;
                validate_value(
                    condition,
                    locals,
                    function_names,
                    global_names,
                    import_names,
                    type_value_names,
                    used_helpers,
                )?;
            }
            NirOp::Trap { name, body } => {
                let mut trap_locals = locals.clone();
                trap_locals.insert(
                    name.clone(),
                    LocalBinding {
                        mutable: false,
                        type_: "Error".to_string(),
                    },
                );
                validate_ops(
                    body,
                    &mut trap_locals,
                    function_names,
                    global_names,
                    import_names,
                    type_value_names,
                    used_helpers,
                )?;
            }
        }
    }
    Ok(())
}

pub(super) fn validate_value(
    value: &NirValue,
    locals: &HashMap<String, LocalBinding>,
    function_names: &HashSet<String>,
    global_names: &HashSet<String>,
    import_names: &HashSet<String>,
    type_value_names: &TypeValueNames,
    used_helpers: &mut Vec<RuntimeHelper>,
) -> Result<(), String> {
    match value {
        NirValue::Const { type_, .. } => validate_type_name(type_),
        NirValue::Local(name) => {
            // bug-176 B: resolve a capitalized `Local` against the explicit name
            // tables — union variant / enum member constructors, type namespaces,
            // and imported symbols — instead of accepting any first-letter-uppercase
            // name. A genuinely-dangling capitalized reference is now rejected here
            // rather than reaching codegen.
            if locals.contains_key(name)
                || type_value_names.constructors.contains(name)
                || type_value_names.namespaces.contains(name)
                || import_names.contains(name)
                || matches!(name.as_str(), "Ok" | "Error")
            {
                Ok(())
            } else {
                Err(format!("NIR local reference '{name}' does not resolve"))
            }
        }
        NirValue::LocalRef { name, type_ } => {
            validate_type_name(type_)?;
            if locals.contains_key(name) {
                Ok(())
            } else {
                Err(format!("NIR local ref '{name}' does not resolve"))
            }
        }
        NirValue::Global { name, type_ } => {
            if !type_.is_empty() {
                validate_type_name(type_)?;
            }
            if global_names.contains(name) {
                Ok(())
            } else {
                Err(format!("NIR global reference '{name}' does not resolve"))
            }
        }
        NirValue::FunctionRef { name, type_ } => {
            validate_type_name(type_)?;
            if function_names.contains(name)
                || import_names.contains(name)
                || builtins::general::builtin_function_id_for_type(name, type_).is_some()
            {
                Ok(())
            } else {
                Err(format!("NIR function reference '{name}' does not resolve"))
            }
        }
        NirValue::Closure {
            name,
            type_,
            captures,
        } => {
            validate_type_name(type_)?;
            if !(function_names.contains(name) || import_names.contains(name)) {
                return Err(format!("NIR closure target '{name}' does not resolve"));
            }
            for value in captures {
                validate_value(
                    value,
                    locals,
                    function_names,
                    global_names,
                    import_names,
                    type_value_names,
                    used_helpers,
                )?;
            }
            Ok(())
        }
        NirValue::Capture { type_, .. } => validate_type_name(type_),
        NirValue::Call { target, args, .. } | NirValue::CallResult { target, args, .. } => {
            for arg in args {
                validate_value(
                    arg,
                    locals,
                    function_names,
                    global_names,
                    import_names,
                    type_value_names,
                    used_helpers,
                )?;
            }
            // An inline `TRAP` on a helper-backed built-in lowers to a
            // `CallResult` whose target resolves to a runtime helper rather than
            // a user function; record the helper so its symbol is emitted.
            if let Some(helper) = runtime::helper_for_call(target) {
                if !runtime::is_native_direct_call(target) {
                    push_unique(used_helpers, helper);
                }
                return Ok(());
            }
            // The migrated `collections::`/`strings::` members and the remaining
            // global natives (len, conversions, ...) are covered by
            // `is_native_direct_call`; the bare moved names are freed for user
            // code and resolve via `function_names` (plan-01-functions.md §5).
            if function_names.contains(target)
                || import_names.contains(target)
                || target == "len"
                || target == "toByte"
                || target == "toFixed"
                || target == "toFloat"
                || target == "toInt"
                || target == "toMoney"
                || target == "toScalar"
                || target == "toString"
                || target == "isNumeric"
                || runtime::is_native_direct_call(target)
                || locals
                    .get(target)
                    .is_some_and(|local| is_function_type(&local.type_))
                // A top-level (global) binding holding a function value is a valid
                // indirect-call target too (bug-198); typecheck already rejected a
                // non-function callee, so accepting the global name here is safe and
                // codegen enforces the FUNC type when loading the pointer.
                || global_names.contains(target)
            {
                Ok(())
            } else {
                Err(format!("NIR call target '{target}' does not resolve"))
            }
        }
        NirValue::RuntimeCall {
            helper,
            target,
            args,
            ..
        } => {
            for arg in args {
                validate_value(
                    arg,
                    locals,
                    function_names,
                    global_names,
                    import_names,
                    type_value_names,
                    used_helpers,
                )?;
            }
            let Some(expected) = runtime::helper_for_call(target) else {
                return Err(format!(
                    "NIR runtime call target '{target}' is not a known runtime helper call"
                ));
            };
            if expected != *helper {
                return Err(format!(
                    "NIR runtime call target '{target}' declares helper '{}' but requires '{}'",
                    helper.name(),
                    expected.name()
                ));
            }
            if !runtime::is_native_direct_call(target) {
                push_unique(used_helpers, *helper);
            }
            Ok(())
        }
        NirValue::Constructor { type_, args } => {
            validate_type_name(type_)?;
            for arg in args {
                validate_value(
                    arg,
                    locals,
                    function_names,
                    global_names,
                    import_names,
                    type_value_names,
                    used_helpers,
                )?;
            }
            Ok(())
        }
        NirValue::UnionWrap {
            union_type,
            member_type,
            value,
        } => {
            validate_type_name(union_type)?;
            validate_type_name(member_type)?;
            validate_value(
                value,
                locals,
                function_names,
                global_names,
                import_names,
                type_value_names,
                used_helpers,
            )
        }
        NirValue::UnionExtract { type_, value } => {
            validate_type_name(type_)?;
            validate_value(
                value,
                locals,
                function_names,
                global_names,
                import_names,
                type_value_names,
                used_helpers,
            )
        }
        NirValue::ResultIsOk { value }
        | NirValue::ResultValue { value }
        | NirValue::ResultError { value } => validate_value(
            value,
            locals,
            function_names,
            global_names,
            import_names,
            type_value_names,
            used_helpers,
        ),
        NirValue::WithUpdate {
            type_,
            target,
            updates,
        } => {
            validate_type_name(type_)?;
            validate_value(
                target,
                locals,
                function_names,
                global_names,
                import_names,
                type_value_names,
                used_helpers,
            )?;
            for update in updates {
                validate_value(
                    &update.value,
                    locals,
                    function_names,
                    global_names,
                    import_names,
                    type_value_names,
                    used_helpers,
                )?;
            }
            Ok(())
        }
        NirValue::ListLiteral { type_, values } => {
            validate_type_name(type_)?;
            for value in values {
                validate_value(
                    value,
                    locals,
                    function_names,
                    global_names,
                    import_names,
                    type_value_names,
                    used_helpers,
                )?;
            }
            Ok(())
        }
        NirValue::MapLiteral { type_, entries } => {
            validate_type_name(type_)?;
            for (key, value) in entries {
                validate_value(
                    key,
                    locals,
                    function_names,
                    global_names,
                    import_names,
                    type_value_names,
                    used_helpers,
                )?;
                validate_value(
                    value,
                    locals,
                    function_names,
                    global_names,
                    import_names,
                    type_value_names,
                    used_helpers,
                )?;
            }
            Ok(())
        }
        NirValue::MemberAccess { target, .. } => match target.as_ref() {
            NirValue::Local(name) if type_value_names.namespaces.contains(name) => Ok(()),
            _ => validate_value(
                target,
                locals,
                function_names,
                global_names,
                import_names,
                type_value_names,
                used_helpers,
            ),
        },
        NirValue::Binary { left, right, .. } => {
            validate_value(
                left,
                locals,
                function_names,
                global_names,
                import_names,
                type_value_names,
                used_helpers,
            )?;
            validate_value(
                right,
                locals,
                function_names,
                global_names,
                import_names,
                type_value_names,
                used_helpers,
            )
        }
        NirValue::Unary { operand, .. } => validate_value(
            operand,
            locals,
            function_names,
            global_names,
            import_names,
            type_value_names,
            used_helpers,
        ),
    }
}

pub(super) fn validate_type_name(type_: &str) -> Result<(), String> {
    if type_.is_empty() {
        Err("NIR type name must not be empty".to_string())
    } else {
        Ok(())
    }
}

pub(super) fn is_function_type(type_: &str) -> bool {
    type_.starts_with("FUNC(") || type_.starts_with("ISOLATED FUNC(")
}

pub(super) fn push_unique(helpers: &mut Vec<RuntimeHelper>, helper: RuntimeHelper) {
    if !helpers.contains(&helper) {
        helpers.push(helper);
    }
}

#[derive(Clone)]
struct LocalBinding {
    mutable: bool,
    type_: String,
}
