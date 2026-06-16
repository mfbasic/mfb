use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::builtins;
use crate::ir::IrProject;
use crate::target::{BackendCapabilities, BuildTarget};

use super::nir::{NirFunction, NirMatchPattern, NirModule, NirOp, NirParam, NirValue};
use super::runtime::{self, RuntimeHelper};

struct TypeValueNames {
    namespaces: HashSet<String>,
    constructors: HashSet<String>,
}

pub fn validate_target(target: &BuildTarget) -> Result<(), String> {
    if target.os.is_empty() || target.arch.is_empty() {
        return Err("native target must include an OS and architecture".to_string());
    }
    Ok(())
}

pub fn validate_project(_ir: &IrProject, _packages: &[PathBuf]) -> Result<(), String> {
    Ok(())
}

pub fn validate_nir(module: &NirModule) -> Result<(), String> {
    if module.target.is_empty() {
        return Err("NIR target must not be empty".to_string());
    }
    if module.project.is_empty() {
        return Err("NIR project name must not be empty".to_string());
    }

    let function_names = unique_function_names(&module.functions)?;
    let import_names = unique_import_names(module)?;
    let type_value_names = type_value_names(module)?;
    validate_entry(module, &function_names)?;

    for helper in &module.runtime_helpers {
        if module
            .runtime_helpers
            .iter()
            .filter(|candidate| *candidate == helper)
            .count()
            > 1
        {
            return Err(format!(
                "NIR runtime helper '{}' is declared more than once",
                helper.name()
            ));
        }
    }

    let mut used_helpers = Vec::new();
    for function in &module.functions {
        validate_function(
            function,
            &function_names,
            &import_names,
            &type_value_names,
            &mut used_helpers,
        )?;
    }

    for helper in &used_helpers {
        if !module.runtime_helpers.contains(helper) {
            return Err(format!(
                "NIR runtime call requires undeclared helper '{}'",
                helper.name()
            ));
        }
    }
    for helper in &module.runtime_helpers {
        if !used_helpers.contains(helper) {
            return Err(format!(
                "NIR declares unused runtime helper '{}'",
                helper.name()
            ));
        }
    }

    Ok(())
}

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
        let helper_supported = runtime::supported_helper_specs().iter().any(|spec| {
            spec.helper == *helper
                && !spec.abi.params.is_empty()
                && !spec.abi.returns.is_empty()
                && !spec.abi.clobbers.is_empty()
        });
        if !helper_supported {
            return Err(format!(
                "native backend does not implement runtime helper '{}'",
                helper.name()
            ));
        }
    }
    Ok(())
}

fn collect_runtime_calls_from_ops(ops: &[NirOp], calls: &mut Vec<String>) {
    let mut constants = HashMap::new();
    collect_runtime_calls_from_ops_with_constants(ops, calls, &mut constants);
}

fn collect_runtime_calls_from_ops_with_constants(
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
            NirOp::Fail { error } => {
                collect_runtime_calls_from_value(error, calls, constants);
            }
            NirOp::Assign { name, value } => {
                collect_runtime_calls_from_value(value, calls, constants);
                if let Some(constant) = native_constant_value(value, constants) {
                    constants.insert(name.clone(), constant);
                } else {
                    constants.remove(name);
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
            NirOp::While { condition, body } => {
                collect_runtime_calls_from_value(condition, calls, constants);
                let mut body_constants = constants.clone();
                collect_runtime_calls_from_ops_with_constants(body, calls, &mut body_constants);
            }
            NirOp::ForEach { iterable, body, .. } => {
                collect_runtime_calls_from_value(iterable, calls, constants);
                let mut body_constants = constants.clone();
                collect_runtime_calls_from_ops_with_constants(body, calls, &mut body_constants);
            }
            NirOp::Using { value, body, .. } => {
                collect_runtime_calls_from_value(value, calls, constants);
                let mut body_constants = constants.clone();
                collect_runtime_calls_from_ops_with_constants(body, calls, &mut body_constants);
            }
            NirOp::Trap { body, .. } => {
                let mut trap_constants = constants.clone();
                collect_runtime_calls_from_ops_with_constants(body, calls, &mut trap_constants);
            }
        }
    }
}

fn collect_runtime_calls_from_value(
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
        | NirValue::FunctionRef { .. } => {}
    }
}

fn native_constant_value(
    value: &NirValue,
    constants: &HashMap<String, NirValue>,
) -> Option<NirValue> {
    match value {
        NirValue::Const { .. } => Some(value.clone()),
        NirValue::Local(name) => constants.get(name).cloned(),
        NirValue::Call { target, args } if target == "toString" && args.len() == 1 => {
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

fn native_static_string_value(
    value: &NirValue,
    constants: &HashMap<String, NirValue>,
) -> Option<String> {
    match value {
        NirValue::Const { type_, value } if type_ == "String" => Some(value.clone()),
        NirValue::Local(name) => constants
            .get(name)
            .and_then(|constant| native_static_string_value(constant, constants)),
        NirValue::Call { target, args } if target == "toString" && args.len() == 1 => {
            native_primitive_text(&args[0], constants)
        }
        NirValue::RuntimeCall { target, args, .. } if target == "toString" && args.len() == 1 => {
            native_primitive_text(&args[0], constants)
        }
        NirValue::Call { target, args } | NirValue::RuntimeCall { target, args, .. } => {
            native_strings_package_static_string_value(target, args, constants)
        }
        NirValue::Binary { op, left, right } if op == "&" => {
            let left = native_static_string_value(left, constants)?;
            let right = native_static_string_value(right, constants)?;
            Some(format!("{left}{right}"))
        }
        _ => None,
    }
}

fn native_strings_package_static_string_value(
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

fn native_static_graphemes_value(
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

fn native_primitive_text(
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
        _ => None,
    }
}

fn type_value_names(module: &NirModule) -> Result<TypeValueNames, String> {
    let mut namespaces = HashSet::new();
    let mut constructors = HashSet::new();
    for type_ in &module.types {
        if type_.name.is_empty() {
            return Err("NIR type has empty name".to_string());
        }
        namespaces.insert(type_.name.clone());
        match type_.kind.as_str() {
            "enum" => {
                for member in &type_.members {
                    if member.name.is_empty() {
                        return Err(format!("NIR enum '{}' has empty member name", type_.name));
                    }
                }
            }
            "union" => {
                for variant in &type_.variants {
                    if variant.name.is_empty() {
                        return Err(format!("NIR union '{}' has empty variant name", type_.name));
                    }
                    constructors.insert(variant.name.clone());
                }
            }
            "type" | "record" | "resource" => {}
            other => {
                return Err(format!(
                    "NIR type '{}' has invalid kind '{other}'",
                    type_.name
                ));
            }
        }
    }
    Ok(TypeValueNames {
        namespaces,
        constructors,
    })
}

fn unique_function_names(functions: &[NirFunction]) -> Result<HashSet<String>, String> {
    let mut names = HashSet::new();
    for function in functions {
        if function.name.is_empty() {
            return Err("NIR function name must not be empty".to_string());
        }
        if !matches!(function.kind.as_str(), "func" | "sub") {
            return Err(format!(
                "NIR function '{}' has invalid kind '{}'",
                function.name, function.kind
            ));
        }
        if !matches!(
            function.visibility.as_str(),
            "private" | "public" | "export"
        ) {
            return Err(format!(
                "NIR function '{}' has invalid visibility '{}'",
                function.name, function.visibility
            ));
        }
        if !names.insert(function.name.clone()) {
            return Err(format!(
                "NIR function '{}' is declared more than once",
                function.name
            ));
        }
    }
    Ok(names)
}

fn unique_import_names(module: &NirModule) -> Result<HashSet<String>, String> {
    let mut names = HashSet::new();
    for import in &module.imports {
        if import.name.is_empty() || import.package.is_empty() || import.symbol.is_empty() {
            return Err("NIR import package, name, and symbol must not be empty".to_string());
        }
        if !matches!(import.kind.as_str(), "func" | "sub") {
            return Err(format!(
                "NIR import '{}' has invalid kind '{}'",
                import.name, import.kind
            ));
        }
        if !names.insert(import.name.clone()) {
            return Err(format!(
                "NIR import '{}' is declared more than once",
                import.name
            ));
        }
    }
    Ok(names)
}

fn validate_entry(module: &NirModule, function_names: &HashSet<String>) -> Result<(), String> {
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

fn validate_function(
    function: &NirFunction,
    function_names: &HashSet<String>,
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
        import_names,
        type_value_names,
        used_helpers,
    )
}

fn validate_param(
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

fn validate_ops(
    ops: &[NirOp],
    locals: &mut HashMap<String, LocalBinding>,
    function_names: &HashSet<String>,
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
                    import_names,
                    type_value_names,
                    used_helpers,
                )?;
            }
            NirOp::Return { value } => {
                if let Some(value) = value {
                    validate_value(
                        value,
                        locals,
                        function_names,
                        import_names,
                        type_value_names,
                        used_helpers,
                    )?;
                }
            }
            NirOp::Fail { error } => {
                validate_value(
                    error,
                    locals,
                    function_names,
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
                    import_names,
                    type_value_names,
                    used_helpers,
                )?;
                let mut then_locals = locals.clone();
                validate_ops(
                    then_body,
                    &mut then_locals,
                    function_names,
                    import_names,
                    type_value_names,
                    used_helpers,
                )?;
                let mut else_locals = locals.clone();
                validate_ops(
                    else_body,
                    &mut else_locals,
                    function_names,
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
                            value: Some(NirValue::UnionExtract { .. }),
                            ..
                        } = op
                        else {
                            break;
                        };
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
                    import_names,
                    type_value_names,
                    used_helpers,
                )?;
            }
            NirOp::While { condition, body } => {
                validate_value(
                    condition,
                    locals,
                    function_names,
                    import_names,
                    type_value_names,
                    used_helpers,
                )?;
                let mut body_locals = locals.clone();
                validate_ops(
                    body,
                    &mut body_locals,
                    function_names,
                    import_names,
                    type_value_names,
                    used_helpers,
                )?;
            }
            NirOp::Using {
                name,
                type_,
                close,
                value,
                body,
            } => {
                if name.is_empty() || type_.is_empty() || close.is_empty() {
                    return Err("NIR using op has empty name, type, or close target".to_string());
                }
                validate_value(
                    value,
                    locals,
                    function_names,
                    import_names,
                    type_value_names,
                    used_helpers,
                )?;
                if !function_names.contains(close)
                    && !import_names.contains(close)
                    && crate::target::shared::runtime::helper_for_call(close).is_none()
                {
                    return Err(format!("NIR using close target '{close}' does not resolve"));
                }
                if let Some(helper) = crate::target::shared::runtime::helper_for_call(close) {
                    push_unique(used_helpers, helper);
                }
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
                    import_names,
                    type_value_names,
                    used_helpers,
                )?;
            }
        }
    }
    Ok(())
}

fn validate_value(
    value: &NirValue,
    locals: &HashMap<String, LocalBinding>,
    function_names: &HashSet<String>,
    import_names: &HashSet<String>,
    type_value_names: &TypeValueNames,
    used_helpers: &mut Vec<RuntimeHelper>,
) -> Result<(), String> {
    match value {
        NirValue::Const { type_, .. } => validate_type_name(type_),
        NirValue::Local(name) => {
            if locals.contains_key(name)
                || type_value_names.constructors.contains(name)
                || matches!(name.as_str(), "Ok" | "Error")
            {
                Ok(())
            } else {
                Err(format!("NIR local reference '{name}' does not resolve"))
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
                    import_names,
                    type_value_names,
                    used_helpers,
                )?;
            }
            Ok(())
        }
        NirValue::Capture { type_, .. } => validate_type_name(type_),
        NirValue::Call { target, args } | NirValue::CallResult { target, args } => {
            for arg in args {
                validate_value(
                    arg,
                    locals,
                    function_names,
                    import_names,
                    type_value_names,
                    used_helpers,
                )?;
            }
            if function_names.contains(target)
                || import_names.contains(target)
                || target == "contains"
                || target == "append"
                || target == "get"
                || target == "getOr"
                || target == "hasKey"
                || target == "insert"
                || target == "find"
                || target == "forEach"
                || target == "filter"
                || target == "keys"
                || target == "len"
                || target == "mid"
                || target == "prepend"
                || target == "reduce"
                || target == "removeAt"
                || target == "removeKey"
                || target == "replace"
                || target == "set"
                || target == "sum"
                || target == "transform"
                || target == "values"
                || target == "toByte"
                || target == "toFixed"
                || target == "toFloat"
                || target == "toInt"
                || target == "toString"
                || target == "isNumeric"
                || runtime::is_native_direct_call(target)
                || locals
                    .get(target)
                    .is_some_and(|local| is_function_type(&local.type_))
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
        } => {
            for arg in args {
                validate_value(
                    arg,
                    locals,
                    function_names,
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
                import_names,
                type_value_names,
                used_helpers,
            )?;
            for update in updates {
                validate_value(
                    &update.value,
                    locals,
                    function_names,
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
                    import_names,
                    type_value_names,
                    used_helpers,
                )?;
                validate_value(
                    value,
                    locals,
                    function_names,
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
                import_names,
                type_value_names,
                used_helpers,
            )?;
            validate_value(
                right,
                locals,
                function_names,
                import_names,
                type_value_names,
                used_helpers,
            )
        }
        NirValue::Unary { operand, .. } => validate_value(
            operand,
            locals,
            function_names,
            import_names,
            type_value_names,
            used_helpers,
        ),
    }
}

fn validate_type_name(type_: &str) -> Result<(), String> {
    if type_.is_empty() {
        Err("NIR type name must not be empty".to_string())
    } else {
        Ok(())
    }
}

fn is_function_type(type_: &str) -> bool {
    type_.starts_with("FUNC(") || type_.starts_with("ISOLATED FUNC(")
}

fn push_unique(helpers: &mut Vec<RuntimeHelper>, helper: RuntimeHelper) {
    if !helpers.contains(&helper) {
        helpers.push(helper);
    }
}

#[derive(Clone)]
struct LocalBinding {
    mutable: bool,
    type_: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::shared::nir::{NirEntryPoint, NirFunction, NirModule, NirOp, NirValue};

    fn module(runtime_helpers: Vec<RuntimeHelper>) -> NirModule {
        NirModule {
            target: "test-target".to_string(),
            project: "hello".to_string(),
            entry: Some(NirEntryPoint {
                name: "main".to_string(),
                returns: "Nothing".to_string(),
                accepts_args: false,
            }),
            types: Vec::new(),
            imports: Vec::new(),
            runtime_helpers,
            functions: vec![NirFunction {
                name: "main".to_string(),
                visibility: "private".to_string(),
                kind: "sub".to_string(),
                isolated: false,
                params: Vec::new(),
                returns: "Nothing".to_string(),
                body: vec![NirOp::Eval {
                    value: NirValue::RuntimeCall {
                        helper: RuntimeHelper::Io,
                        target: "io.print".to_string(),
                        args: vec![NirValue::Const {
                            type_: "String".to_string(),
                            value: "Hello World".to_string(),
                        }],
                    },
                }],
            }],
        }
    }

    #[test]
    fn validates_declared_runtime_helper() {
        validate_nir(&module(vec![RuntimeHelper::Io])).expect("valid NIR");
    }

    #[test]
    fn rejects_undeclared_runtime_helper() {
        let err = validate_nir(&module(Vec::new())).expect_err("missing helper");
        assert_eq!(err, "NIR runtime call requires undeclared helper 'io'");
    }
}
