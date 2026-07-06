use super::*;

use std::collections::HashMap;

pub(super) fn runtime_symbols(module: &NirModule) -> Vec<String> {
    let mut symbols = Vec::new();
    for function in &module.functions {
        collect_runtime_symbols_from_ops(&function.body, &mut symbols);
    }
    if module_has_thread_owner(module) {
        push_unique(
            &mut symbols,
            runtime::symbol_for_call(runtime::RuntimeHelper::Thread, "thread.drop"),
        );
    }
    // A resource-union bind drops by dispatching to each variant's close op
    // (codegen-emitted on scope exit), so pull in every variant's close helper.
    let mut bind_types = std::collections::HashSet::new();
    for function in &module.functions {
        collect_bind_type_names(&function.body, &mut bind_types);
    }
    for type_ in &module.types {
        if type_.kind != "union"
            || !bind_types.contains(&type_.name)
            || type_.variants.is_empty()
            || !type_
                .variants
                .iter()
                .all(|variant| crate::builtins::is_resource_type(&variant.name))
        {
            continue;
        }
        for variant in &type_.variants {
            if let Some(close) = crate::builtins::resource_close_function(&variant.name) {
                if let Some(helper) = runtime::helper_for_call(close) {
                    push_unique(&mut symbols, runtime::symbol_for_call(helper, close));
                }
            }
        }
    }
    symbols
}

pub(super) fn collect_bind_type_names(
    ops: &[NirOp],
    types: &mut std::collections::HashSet<String>,
) {
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
                collect_bind_type_names(then_body, types);
                collect_bind_type_names(else_body, types);
            }
            NirOp::Match { cases, .. } => {
                for case in cases {
                    collect_bind_type_names(&case.body, types);
                }
            }
            NirOp::While { body, .. }
            | NirOp::For { body, .. }
            | NirOp::DoUntil { body, .. }
            | NirOp::ForEach { body, .. }
            | NirOp::Trap { body, .. } => collect_bind_type_names(body, types),
            _ => {}
        }
    }
}

pub(super) fn platform_imports(
    module: &NirModule,
    platform: &dyn NativePlanPlatform,
) -> Vec<PlatformImport> {
    let mut imports = Vec::new();
    for import in platform.entry_imports(module) {
        push_platform_import(&mut imports, import);
    }
    for import in platform.entry_error_imports(module) {
        push_platform_import(&mut imports, import);
    }
    for function in &module.functions {
        collect_platform_imports_from_ops(
            platform,
            &nir::function_symbol(&function.name),
            &function.body,
            &mut imports,
        );
    }
    if module_has_thread_owner(module) {
        for import in platform_imports_for_runtime_call(platform, "thread.drop") {
            push_platform_import(&mut imports, import);
        }
    }
    if !module.link_functions.is_empty() {
        for import in platform.link_imports(nir::LINK_INIT_SYMBOL) {
            push_platform_import(&mut imports, import);
        }
    }
    if module.build_mode.is_app() {
        // App mode binds the toolkit the `_main` bootstrap drives: the Obj-C
        // runtime/AppKit/Foundation on macOS (plan-04-macos-app.md §6.5) or
        // GTK4/GObject/GLib/GIO on Linux (plan-05-linux-app.md §6.4). The platform
        // chooses; shared lowering just pulls in whatever it declares.
        for import in platform.app_mode_imports() {
            push_platform_import(&mut imports, import);
        }
    }
    imports
}

pub(super) fn is_thread_type(type_: &str) -> bool {
    type_.starts_with("Thread OF ")
}

pub(super) fn module_has_thread_owner(module: &NirModule) -> bool {
    module.functions.iter().any(|function| {
        function
            .params
            .iter()
            .any(|param| is_thread_type(&param.type_))
            || ops_have_thread_owner(&function.body)
    })
}

pub(super) fn ops_have_thread_owner(ops: &[NirOp]) -> bool {
    ops.iter().any(|op| match op {
        NirOp::Bind { type_, .. } | NirOp::StoreGlobal { type_, .. } => is_thread_type(type_),
        NirOp::ForEach { type_, body, .. } => is_thread_type(type_) || ops_have_thread_owner(body),
        NirOp::If {
            then_body,
            else_body,
            ..
        } => ops_have_thread_owner(then_body) || ops_have_thread_owner(else_body),
        NirOp::Match { cases, .. } => cases.iter().any(|case| ops_have_thread_owner(&case.body)),
        NirOp::While { body, .. } | NirOp::Trap { body, .. } => ops_have_thread_owner(body),
        NirOp::For { body, .. } | NirOp::DoUntil { body, .. } => ops_have_thread_owner(body),
        NirOp::Assign { .. }
        | NirOp::StateAssign { .. }
        | NirOp::Return { .. }
        | NirOp::ExitLoop { .. }
        | NirOp::ContinueLoop { .. }
        | NirOp::ExitProgram { .. }
        | NirOp::Fail { .. }
        | NirOp::Eval { .. } => false,
    })
}

pub(super) fn collect_platform_imports_from_ops(
    platform: &dyn NativePlanPlatform,
    required_by: &str,
    ops: &[NirOp],
    imports: &mut Vec<PlatformImport>,
) {
    for op in ops {
        match op {
            NirOp::Bind { value, .. }
            | NirOp::StoreGlobal { value, .. }
            | NirOp::Return { value } => {
                if let Some(value) = value {
                    collect_platform_imports_from_value(platform, required_by, value, imports);
                }
            }
            NirOp::ExitLoop { .. } | NirOp::ContinueLoop { .. } => {}
            NirOp::ExitProgram { code } => {
                collect_platform_imports_from_value(platform, required_by, code, imports);
                for import in platform.program_exit_imports(required_by) {
                    push_platform_import(imports, import);
                }
            }
            NirOp::Fail { error } => {
                collect_platform_imports_from_value(platform, required_by, error, imports);
            }
            NirOp::Assign { value, .. }
            | NirOp::StateAssign { value, .. }
            | NirOp::Eval { value } => {
                collect_platform_imports_from_value(platform, required_by, value, imports);
            }
            NirOp::If {
                condition,
                then_body,
                else_body,
            } => {
                collect_platform_imports_from_value(platform, required_by, condition, imports);
                collect_platform_imports_from_ops(platform, required_by, then_body, imports);
                collect_platform_imports_from_ops(platform, required_by, else_body, imports);
            }
            NirOp::Match { value, cases } => {
                collect_platform_imports_from_value(platform, required_by, value, imports);
                for case in cases {
                    collect_platform_imports_from_ops(platform, required_by, &case.body, imports);
                }
            }
            NirOp::While {
                condition, body, ..
            } => {
                collect_platform_imports_from_value(platform, required_by, condition, imports);
                collect_platform_imports_from_ops(platform, required_by, body, imports);
            }
            NirOp::For {
                start,
                end,
                step,
                body,
                ..
            } => {
                collect_platform_imports_from_value(platform, required_by, start, imports);
                collect_platform_imports_from_value(platform, required_by, end, imports);
                collect_platform_imports_from_value(platform, required_by, step, imports);
                collect_platform_imports_from_ops(platform, required_by, body, imports);
            }
            NirOp::DoUntil { body, condition } => {
                collect_platform_imports_from_ops(platform, required_by, body, imports);
                collect_platform_imports_from_value(platform, required_by, condition, imports);
            }
            NirOp::ForEach { iterable, body, .. } => {
                collect_platform_imports_from_value(platform, required_by, iterable, imports);
                collect_platform_imports_from_ops(platform, required_by, body, imports);
            }
            NirOp::Trap { body, .. } => {
                collect_platform_imports_from_ops(platform, required_by, body, imports);
            }
        }
    }
}

pub(super) fn collect_platform_imports_from_value(
    platform: &dyn NativePlanPlatform,
    required_by: &str,
    value: &NirValue,
    imports: &mut Vec<PlatformImport>,
) {
    match value {
        NirValue::RuntimeCall { target, args, .. } => {
            if target != "typeName" && !runtime::is_native_direct_call(target) {
                for import in platform_imports_for_runtime_call(platform, target) {
                    push_platform_import(imports, import);
                }
            }
            for arg in args {
                collect_platform_imports_from_value(platform, required_by, arg, imports);
            }
        }
        NirValue::Call { target, args, .. } | NirValue::CallResult { target, args, .. } => {
            // A helper-backed `CallResult` (inline `TRAP` on a built-in) pulls in
            // the same platform imports as the equivalent `RuntimeCall`.
            if target != "typeName"
                && !runtime::is_native_direct_call(target)
                && runtime::helper_for_call(target).is_some()
            {
                for import in platform_imports_for_runtime_call(platform, target) {
                    push_platform_import(imports, import);
                }
            } else {
                for import in platform.native_call_imports(target, required_by) {
                    push_platform_import(imports, import);
                }
            }
            for arg in args {
                collect_platform_imports_from_value(platform, required_by, arg, imports);
            }
        }
        NirValue::Constructor { args, .. } => {
            for arg in args {
                collect_platform_imports_from_value(platform, required_by, arg, imports);
            }
        }
        NirValue::UnionWrap { value, .. }
        | NirValue::UnionExtract { value, .. }
        | NirValue::ResultIsOk { value }
        | NirValue::ResultValue { value }
        | NirValue::ResultError { value } => {
            collect_platform_imports_from_value(platform, required_by, value, imports);
        }
        NirValue::WithUpdate {
            target, updates, ..
        } => {
            collect_platform_imports_from_value(platform, required_by, target, imports);
            for update in updates {
                collect_platform_imports_from_value(platform, required_by, &update.value, imports);
            }
        }
        NirValue::ListLiteral { values, .. } => {
            for value in values {
                collect_platform_imports_from_value(platform, required_by, value, imports);
            }
        }
        NirValue::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                collect_platform_imports_from_value(platform, required_by, key, imports);
                collect_platform_imports_from_value(platform, required_by, value, imports);
            }
        }
        NirValue::MemberAccess { target, .. } => {
            collect_platform_imports_from_value(platform, required_by, target, imports)
        }
        NirValue::Binary { left, right, .. } => {
            // `Float MOD Float` lowers to the in-tree exact `fmod` kernel
            // (builder_numeric::emit_float_fmod), so it no longer imports libm.
            collect_platform_imports_from_value(platform, required_by, left, imports);
            collect_platform_imports_from_value(platform, required_by, right, imports);
        }
        NirValue::Unary { operand, .. } => {
            collect_platform_imports_from_value(platform, required_by, operand, imports)
        }
        NirValue::Closure { captures, .. } => {
            for value in captures {
                collect_platform_imports_from_value(platform, required_by, value, imports);
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

pub(super) fn platform_imports_for_runtime_call(
    platform: &dyn NativePlanPlatform,
    target: &str,
) -> Vec<PlatformImport> {
    let Some(spec) = runtime::spec_for_call(target) else {
        return Vec::new();
    };
    platform.runtime_imports(spec)
}

pub(super) fn collect_runtime_symbols_from_ops(ops: &[NirOp], symbols: &mut Vec<String>) {
    let mut constants = HashMap::new();
    collect_runtime_symbols_from_ops_with_constants(ops, symbols, &mut constants);
}

pub(super) fn collect_runtime_symbols_from_ops_with_constants(
    ops: &[NirOp],
    symbols: &mut Vec<String>,
    constants: &mut HashMap<String, NirValue>,
) {
    for op in ops {
        match op {
            NirOp::Bind {
                name, type_, value, ..
            } => {
                if let Some(close) = crate::builtins::resource_close_function(type_) {
                    if let Some(helper) = runtime::helper_for_call(close) {
                        push_unique(symbols, runtime::symbol_for_call(helper, close));
                    }
                }
                if let Some(value) = value {
                    collect_runtime_symbols_from_value(value, symbols, constants);
                    if let Some(constant) = native_constant_value(value, constants) {
                        constants.insert(name.clone(), constant);
                    } else {
                        constants.remove(name);
                    }
                } else {
                    constants.remove(name);
                }
            }
            NirOp::StoreGlobal { value, .. } => {
                if let Some(value) = value {
                    collect_runtime_symbols_from_value(value, symbols, constants);
                }
            }
            NirOp::Return { value } => {
                if let Some(value) = value {
                    collect_runtime_symbols_from_value(value, symbols, constants);
                }
            }
            NirOp::ExitLoop { .. } | NirOp::ContinueLoop { .. } => {}
            NirOp::ExitProgram { code } => {
                collect_runtime_symbols_from_value(code, symbols, constants);
            }
            NirOp::Fail { error } => {
                collect_runtime_symbols_from_value(error, symbols, constants);
            }
            NirOp::StateAssign { value, .. } => {
                collect_runtime_symbols_from_value(value, symbols, constants);
            }
            NirOp::Assign { name, value } => {
                collect_runtime_symbols_from_value(value, symbols, constants);
                if let Some(constant) = native_constant_value(value, constants) {
                    constants.insert(name.clone(), constant);
                } else {
                    constants.remove(name);
                }
            }
            NirOp::Eval { value } => {
                collect_runtime_symbols_from_value(value, symbols, constants);
            }
            NirOp::If {
                condition,
                then_body,
                else_body,
            } => {
                collect_runtime_symbols_from_value(condition, symbols, constants);
                let mut then_constants = constants.clone();
                let mut else_constants = constants.clone();
                collect_runtime_symbols_from_ops_with_constants(
                    then_body,
                    symbols,
                    &mut then_constants,
                );
                collect_runtime_symbols_from_ops_with_constants(
                    else_body,
                    symbols,
                    &mut else_constants,
                );
            }
            NirOp::Match { value, cases } => {
                collect_runtime_symbols_from_value(value, symbols, constants);
                for case in cases {
                    let mut case_constants = constants.clone();
                    collect_runtime_symbols_from_ops_with_constants(
                        &case.body,
                        symbols,
                        &mut case_constants,
                    );
                }
            }
            NirOp::While {
                condition, body, ..
            } => {
                collect_runtime_symbols_from_value(condition, symbols, constants);
                let mut body_constants = constants.clone();
                collect_runtime_symbols_from_ops_with_constants(body, symbols, &mut body_constants);
            }
            NirOp::For {
                start,
                end,
                step,
                body,
                ..
            } => {
                collect_runtime_symbols_from_value(start, symbols, constants);
                collect_runtime_symbols_from_value(end, symbols, constants);
                collect_runtime_symbols_from_value(step, symbols, constants);
                let mut body_constants = constants.clone();
                collect_runtime_symbols_from_ops_with_constants(body, symbols, &mut body_constants);
            }
            NirOp::DoUntil { body, condition } => {
                let mut body_constants = constants.clone();
                collect_runtime_symbols_from_ops_with_constants(body, symbols, &mut body_constants);
                collect_runtime_symbols_from_value(condition, symbols, constants);
            }
            NirOp::ForEach { iterable, body, .. } => {
                collect_runtime_symbols_from_value(iterable, symbols, constants);
                let mut body_constants = constants.clone();
                collect_runtime_symbols_from_ops_with_constants(body, symbols, &mut body_constants);
            }
            NirOp::Trap { body, .. } => {
                let mut trap_constants = constants.clone();
                collect_runtime_symbols_from_ops_with_constants(body, symbols, &mut trap_constants);
            }
        }
    }
}

pub(super) fn collect_runtime_symbols_from_value(
    value: &NirValue,
    symbols: &mut Vec<String>,
    constants: &HashMap<String, NirValue>,
) {
    match value {
        NirValue::RuntimeCall {
            helper,
            target,
            args,
            ..
        } => {
            if target != "typeName"
                && !runtime::is_native_direct_call(target)
                && native_static_string_value(value, constants).is_none()
                && native_static_graphemes_value(target, args, constants).is_none()
            {
                push_unique(symbols, runtime::symbol_for_call(*helper, target));
            }
            for arg in args {
                collect_runtime_symbols_from_value(arg, symbols, constants);
            }
        }
        NirValue::CallResult { target, args, .. } => {
            // A helper-backed `CallResult` (inline `TRAP` on a built-in) invokes
            // the runtime helper directly, so its symbol must be defined just
            // like the equivalent `RuntimeCall`.
            if !runtime::is_native_direct_call(target) {
                if let Some(helper) = runtime::helper_for_call(target) {
                    push_unique(symbols, runtime::symbol_for_call(helper, target));
                }
            }
            for arg in args {
                collect_runtime_symbols_from_value(arg, symbols, constants);
            }
        }
        NirValue::Call { args, .. } | NirValue::Constructor { args, .. } => {
            for arg in args {
                collect_runtime_symbols_from_value(arg, symbols, constants);
            }
        }
        NirValue::UnionWrap { value, .. }
        | NirValue::UnionExtract { value, .. }
        | NirValue::ResultIsOk { value }
        | NirValue::ResultValue { value }
        | NirValue::ResultError { value } => {
            collect_runtime_symbols_from_value(value, symbols, constants);
        }
        NirValue::WithUpdate {
            target, updates, ..
        } => {
            collect_runtime_symbols_from_value(target, symbols, constants);
            for update in updates {
                collect_runtime_symbols_from_value(&update.value, symbols, constants);
            }
        }
        NirValue::ListLiteral { values, .. } => {
            for value in values {
                collect_runtime_symbols_from_value(value, symbols, constants);
            }
        }
        NirValue::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                collect_runtime_symbols_from_value(key, symbols, constants);
                collect_runtime_symbols_from_value(value, symbols, constants);
            }
        }
        NirValue::MemberAccess { target, member } => {
            if member == "result" {
                push_unique(
                    symbols,
                    runtime::symbol_for_call(runtime::RuntimeHelper::Thread, "thread.waitFor"),
                );
            }
            collect_runtime_symbols_from_value(target, symbols, constants)
        }
        NirValue::Binary { left, right, .. } => {
            collect_runtime_symbols_from_value(left, symbols, constants);
            collect_runtime_symbols_from_value(right, symbols, constants);
        }
        NirValue::Unary { operand, .. } => {
            collect_runtime_symbols_from_value(operand, symbols, constants)
        }
        NirValue::Closure { captures, .. } => {
            for value in captures {
                collect_runtime_symbols_from_value(value, symbols, constants);
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
            "Integer" | "Byte" | "Float" | "Fixed" | "String" => Some(value.clone()),
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

pub(super) fn push_platform_import(imports: &mut Vec<PlatformImport>, import: PlatformImport) {
    if imports.iter().any(|existing| {
        existing.library == import.library
            && existing.symbol == import.symbol
            && existing.required_by == import.required_by
    }) {
        return;
    }
    imports.push(import);
}

pub(super) fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.contains(&value) {
        values.push(value);
    }
}
