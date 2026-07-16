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
    let global_names = unique_global_names(module)?;
    let import_names = unique_import_names(module)?;
    let type_value_names = type_value_names(module)?;
    validate_entry(module, &function_names)?;
    validate_resource_rules(module)?;

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
            &global_names,
            &import_names,
            &type_value_names,
            &mut used_helpers,
        )?;
    }

    // A resource-union bind drops by dispatching to each variant's close op
    // (codegen-emitted, not an NIR call), so count those closes as used helpers
    // to match `required_helpers`.
    let mut bind_types = HashSet::new();
    for function in &module.functions {
        collect_bind_types(&function.body, &mut bind_types);
    }
    for type_ in &module.types {
        if type_.kind != "union" || !bind_types.contains(&type_.name) {
            continue;
        }
        let closes: Option<Vec<&'static str>> = type_
            .variants
            .iter()
            .map(|variant| crate::builtins::resource_close_function(&variant.name))
            .collect();
        if let Some(closes) = closes {
            for close in closes {
                if let Some(helper) = runtime::helper_for_call(close) {
                    if !used_helpers.contains(&helper) {
                        used_helpers.push(helper);
                    }
                }
            }
        }
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

/// Whether a NIR type string transitively owns a resource (directly, or as a
/// collection element/value). `STATE`-suffixed resource strings are recognized
/// via `is_resource_type`.
fn type_owns_resource(type_: &str) -> bool {
    if crate::builtins::is_resource_type(type_) {
        return true;
    }
    if let Some(element) = type_.strip_prefix("List OF ") {
        return type_owns_resource(element);
    }
    if let Some(rest) = type_.strip_prefix("Map OF ") {
        if let Some((key, value)) = rest.split_once(" TO ") {
            return type_owns_resource(key) || type_owns_resource(value);
        }
    }
    if let Some(success) = type_.strip_prefix("Result OF ") {
        return type_owns_resource(success);
    }
    false
}

/// Backstop verification of the resource model's structural rules (the type
/// checker is the primary enforcer; this guards against a malformed NIR):
/// a record may not own a resource, and a union may not mix data and resource
/// variants.
fn validate_resource_rules(module: &NirModule) -> Result<(), String> {
    for type_ in &module.types {
        match type_.kind.as_str() {
            "type" => {
                for field in &type_.fields {
                    if type_owns_resource(&field.type_) {
                        return Err(format!(
                            "NIR record '{}' field '{}' owns a resource; records cannot own resources",
                            type_.name, field.name
                        ));
                    }
                }
            }
            "union" => {
                // A union must be uniformly data or uniformly resource. A
                // variant is a resource either by being a bare resource type
                // or by owning one in its payload.
                let mut has_resource = false;
                let mut has_data = false;
                for variant in &type_.variants {
                    let is_resource = crate::builtins::is_resource_type(&variant.name)
                        || variant
                            .fields
                            .iter()
                            .any(|field| type_owns_resource(&field.type_));
                    if is_resource {
                        has_resource = true;
                    } else {
                        has_data = true;
                    }
                }
                if has_resource && has_data {
                    return Err(format!(
                        "NIR union '{}' mixes data and resource variants",
                        type_.name
                    ));
                }
            }
            _ => {}
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

/// Collect the type strings of every `Bind` op (recursively) so resource-union
/// binds can be matched against union type definitions.
fn collect_bind_types(ops: &[NirOp], types: &mut HashSet<String>) {
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
                let mut body_constants = constants.clone();
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
                let mut body_constants = constants.clone();
                collect_runtime_calls_from_ops_with_constants(body, calls, &mut body_constants);
            }
            NirOp::DoUntil { body, condition } => {
                let mut body_constants = constants.clone();
                collect_runtime_calls_from_ops_with_constants(body, calls, &mut body_constants);
                collect_runtime_calls_from_value(condition, calls, constants);
            }
            NirOp::ForEach { iterable, body, .. } => {
                collect_runtime_calls_from_value(iterable, calls, constants);
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
        | NirValue::LocalRef { .. }
        | NirValue::Global { .. }
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

fn native_static_string_value(
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
            // Scientific-notation Float/Fixed literals fold to their expanded
            // plain decimal (`2.5e2` -> `250`; plan-28-B).
            "Float" | "Fixed" if value.contains('e') || value.contains('E') => {
                crate::numeric::expanded_literal_text(value)
            }
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

fn unique_global_names(module: &NirModule) -> Result<HashSet<String>, String> {
    let mut names = HashSet::new();
    let mut symbols = HashSet::new();
    for global in &module.globals {
        if global.name.is_empty() || global.symbol.is_empty() || global.type_.is_empty() {
            return Err("NIR global name, symbol, and type must not be empty".to_string());
        }
        if !matches!(global.visibility.as_str(), "private" | "public" | "export") {
            return Err(format!(
                "NIR global '{}' has invalid visibility '{}'",
                global.name, global.visibility
            ));
        }
        if !names.insert(global.name.clone()) {
            return Err(format!(
                "NIR global '{}' is declared more than once",
                global.name
            ));
        }
        if !symbols.insert(global.symbol.clone()) {
            return Err(format!(
                "NIR global symbol '{}' is declared more than once",
                global.symbol
            ));
        }
    }
    Ok(names)
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
                    // bug-176 B: track enum member names explicitly so a bare
                    // `Local(member)` reference resolves against the type table
                    // rather than the first-letter-case heuristic.
                    constructors.insert(member.name.clone());
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

fn validate_value(
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
    use crate::target::shared::nir::{
        NirEntryPoint, NirFunction, NirModule, NirOp, NirSourceLoc, NirType, NirValue, NirVariant,
    };

    fn module(runtime_helpers: Vec<RuntimeHelper>) -> NirModule {
        NirModule {
            target: "test-target".to_string(),
            build_mode: crate::target::NativeBuildMode::Console,
            stdin_log_cap: crate::target::shared::code::STDIN_LOG_CAP_DEFAULT,
            project: "hello".to_string(),
            entry: Some(NirEntryPoint {
                name: "main".to_string(),
                returns: "Nothing".to_string(),
                accepts_args: false,
            }),
            types: Vec::new(),
            globals: Vec::new(),
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
                        loc: NirSourceLoc::default(),
                    },
                }],
                file: "src/main.mfb".to_string(),
                resource_owners: std::collections::HashMap::new(),
            }],
            link_functions: Vec::new(),
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

    /// A resource-union bind nested inside a `FOR EACH` body drops by dispatching
    /// to each variant's close op, so those close helpers must be counted as used.
    /// bug-45: `collect_bind_types` skipped `NirOp::ForEach` bodies, so the union
    /// bind went unseen and `validate_nir` wrongly rejected the declared `net`
    /// helper as unused. Build the module directly so the collector is exercised
    /// in isolation from the front end.
    fn module_with_union_bind(body: Vec<NirOp>) -> NirModule {
        NirModule {
            target: "test-target".to_string(),
            build_mode: crate::target::NativeBuildMode::Console,
            stdin_log_cap: crate::target::shared::code::STDIN_LOG_CAP_DEFAULT,
            project: "hello".to_string(),
            entry: Some(NirEntryPoint {
                name: "main".to_string(),
                returns: "Integer".to_string(),
                accepts_args: false,
            }),
            types: vec![NirType {
                kind: "union".to_string(),
                visibility: "public".to_string(),
                name: "Stream".to_string(),
                fields: Vec::new(),
                includes: Vec::new(),
                variants: vec![
                    NirVariant {
                        name: "File".to_string(),
                        fields: Vec::new(),
                    },
                    NirVariant {
                        name: "Socket".to_string(),
                        fields: Vec::new(),
                    },
                ],
                members: Vec::new(),
            }],
            globals: Vec::new(),
            imports: Vec::new(),
            // `File` closes via `fs`, `Socket` via `net`; both are declared so the
            // cross-check must find both in `used_helpers`.
            runtime_helpers: vec![RuntimeHelper::Fs, RuntimeHelper::Net],
            functions: vec![NirFunction {
                name: "main".to_string(),
                visibility: "private".to_string(),
                kind: "func".to_string(),
                isolated: false,
                params: Vec::new(),
                returns: "Integer".to_string(),
                body,
                file: "src/main.mfb".to_string(),
                resource_owners: std::collections::HashMap::new(),
            }],
            link_functions: Vec::new(),
        }
    }

    fn union_bind() -> NirOp {
        NirOp::Bind {
            mutable: false,
            name: "s".to_string(),
            type_: "Stream".to_string(),
            value: None,
        }
    }

    fn integer_list() -> NirValue {
        NirValue::ListLiteral {
            type_: "List OF Integer".to_string(),
            values: vec![NirValue::Const {
                type_: "Integer".to_string(),
                value: "1".to_string(),
            }],
        }
    }

    #[test]
    fn collects_resource_union_bind_inside_for_each() {
        let module = module_with_union_bind(vec![NirOp::ForEach {
            name: "n".to_string(),
            type_: "Integer".to_string(),
            iterable: integer_list(),
            body: vec![union_bind()],
        }]);
        validate_nir(&module).expect("resource-union bind inside FOR EACH must validate");
    }

    #[test]
    fn collects_resource_union_bind_at_top_level() {
        // The contrast case the bug doc names: the same bind at function scope has
        // always been collected. Guards that the ForEach fix did not change it.
        let module = module_with_union_bind(vec![union_bind()]);
        validate_nir(&module).expect("resource-union bind at top level must validate");
    }
}
