use super::*;

/// The expected (contextual) type for an argument slot: the selected parameter's
/// declared type, but only when the argument is itself a call — the one position
/// where a return-type overload set needs the context to resolve
/// (plan-01-overload.md §F.2). Returns `None` otherwise so literals keep their own
/// inferred typing.
pub(super) fn arg_slot_expected<'a>(
    value: &Expression,
    params: Option<&'a [crate::ast::Param]>,
    select: impl FnOnce(&'a [crate::ast::Param]) -> Option<&'a crate::ast::Param>,
) -> Option<&'a str> {
    if !matches!(value, Expression::Call { .. }) {
        return None;
    }
    select(params?)?.type_name.as_deref()
}

pub(super) fn call_arg_value(argument: &CallArg) -> &Expression {
    match argument {
        CallArg::Positional(value) => value,
        CallArg::Named { value, .. } => value,
    }
}

pub(super) fn constructor_arg_field_type<'a>(
    argument: &ConstructorArg,
    index: usize,
    fields: Option<&'a [TypeField]>,
) -> Option<&'a str> {
    let fields = fields?;
    match argument {
        ConstructorArg::Positional(_) => fields.get(index).map(|field| field.type_name.as_str()),
        ConstructorArg::Named { name, .. } => fields
            .iter()
            .find(|field| field.name == *name)
            .map(|field| field.type_name.as_str()),
    }
}


pub(super) fn unify_type(
    pattern: &str,
    actual: &str,
    params: &[String],
    substitutions: &mut HashMap<String, String>,
) -> bool {
    if params.iter().any(|param| param == pattern) {
        if let Some(existing) = substitutions.get(pattern) {
            return existing == actual;
        }
        substitutions.insert(pattern.to_string(), actual.to_string());
        return true;
    }

    if let Some(pattern_element) = pattern.strip_prefix("List OF ") {
        let Some(actual_element) = actual.strip_prefix("List OF ") else {
            return false;
        };
        return unify_type(pattern_element, actual_element, params, substitutions);
    }
    if let Some(pattern_success) = pattern.strip_prefix("Result OF ") {
        let Some(actual_success) = actual.strip_prefix("Result OF ") else {
            return false;
        };
        return unify_type(pattern_success, actual_success, params, substitutions);
    }
    if let Some(pattern_rest) = pattern.strip_prefix("Map OF ") {
        let Some(actual_rest) = actual.strip_prefix("Map OF ") else {
            return false;
        };
        let Some((pattern_key, pattern_value)) = split_top_level_to(pattern_rest) else {
            return false;
        };
        let Some((actual_key, actual_value)) = split_top_level_to(actual_rest) else {
            return false;
        };
        return unify_type(&pattern_key, &actual_key, params, substitutions)
            && unify_type(&pattern_value, &actual_value, params, substitutions);
    }
    if let Some(pattern_rest) = pattern.strip_prefix("MapEntry OF ") {
        let Some(actual_rest) = actual.strip_prefix("MapEntry OF ") else {
            return false;
        };
        let Some((pattern_key, pattern_value)) = split_top_level_to(pattern_rest) else {
            return false;
        };
        let Some((actual_key, actual_value)) = split_top_level_to(actual_rest) else {
            return false;
        };
        return unify_type(&pattern_key, &actual_key, params, substitutions)
            && unify_type(&pattern_value, &actual_value, params, substitutions);
    }
    if let Some((pattern_kind, pattern_message, pattern_resource, pattern_output)) =
        crate::builtins::thread::thread_parts_full(pattern)
    {
        let Some((actual_kind, actual_message, actual_resource, actual_output)) =
            crate::builtins::thread::thread_parts_full(actual)
        else {
            return false;
        };
        let resource_unifies = match (pattern_resource, actual_resource) {
            (None, None) => true,
            (Some(pattern_resource), Some(actual_resource)) => {
                unify_type(pattern_resource, actual_resource, params, substitutions)
            }
            _ => false,
        };
        return pattern_kind == actual_kind
            && unify_type(pattern_message, actual_message, params, substitutions)
            && resource_unifies
            && unify_type(pattern_output, actual_output, params, substitutions);
    }
    if let (Some((pattern_name, pattern_args)), Some((actual_name, actual_args))) =
        (user_template_parts(pattern), user_template_parts(actual))
    {
        return pattern_name == actual_name
            && pattern_args.len() == actual_args.len()
            && pattern_args
                .iter()
                .zip(actual_args.iter())
                .all(|(pattern, actual)| unify_type(pattern, actual, params, substitutions));
    }

    if let (Some((pattern_params, pattern_ret)), Some((actual_params, actual_ret))) =
        (func_type_parts(pattern), func_type_parts(actual))
    {
        return pattern_params.len() == actual_params.len()
            && pattern_params
                .iter()
                .zip(actual_params.iter())
                .all(|(pattern, actual)| unify_type(pattern, actual, params, substitutions))
            && unify_type(pattern_ret, actual_ret, params, substitutions);
    }

    pattern == actual || actual == "Unknown"
}

/// Splits a function type `FUNC(p1, p2) AS Ret` (or `ISOLATED FUNC(...) AS Ret`)
/// into its parameter types and return type for template unification. Parameters
/// are split on top-level `", "`, matching the rest of the builtin type parsing.
pub(super) fn func_type_parts(type_name: &str) -> Option<(Vec<&str>, &str)> {
    let rest = type_name
        .strip_prefix("FUNC(")
        .or_else(|| type_name.strip_prefix("ISOLATED FUNC("))?;
    let (params, ret) = rest.split_once(") AS ")?;
    let params = if params.trim().is_empty() {
        Vec::new()
    } else {
        params.split(", ").collect()
    };
    Some((params, ret))
}

pub(super) fn user_template_parts(type_name: &str) -> Option<(String, Vec<String>)> {
    if type_name.starts_with("List OF ")
        || type_name.starts_with("Map OF ")
        || type_name.starts_with("MapEntry OF ")
        || type_name.starts_with("Result OF ")
        || type_name.starts_with("Thread OF ")
        || type_name.starts_with("ThreadWorker OF ")
        || type_name.starts_with("FUNC(")
        || type_name.starts_with("ISOLATED FUNC(")
    {
        return None;
    }
    let (name, rest) = type_name.split_once(" OF ")?;
    Some((name.to_string(), split_top_level_commas(rest)))
}

pub(super) fn substitute_type_params(type_name: &str, substitutions: &HashMap<String, String>) -> String {
    if let Some(value) = substitutions.get(type_name) {
        return value.clone();
    }
    if let Some(element) = type_name.strip_prefix("List OF ") {
        return format!("List OF {}", substitute_type_params(element, substitutions));
    }
    if let Some(success) = type_name.strip_prefix("Result OF ") {
        return format!(
            "Result OF {}",
            substitute_type_params(success, substitutions)
        );
    }
    if let Some(rest) = type_name.strip_prefix("Map OF ") {
        if let Some((key, value)) = split_top_level_to(rest) {
            return format!(
                "Map OF {} TO {}",
                substitute_type_params(&key, substitutions),
                substitute_type_params(&value, substitutions)
            );
        }
    }
    if let Some(rest) = type_name.strip_prefix("MapEntry OF ") {
        if let Some((key, value)) = split_top_level_to(rest) {
            return format!(
                "MapEntry OF {} TO {}",
                substitute_type_params(&key, substitutions),
                substitute_type_params(&value, substitutions)
            );
        }
    }
    if let Some((kind, message, resource, output)) =
        crate::builtins::thread::thread_parts_full(type_name)
    {
        let resource = resource.map(|resource| substitute_type_params(resource, substitutions));
        return crate::builtins::thread::format_thread_type(
            kind,
            &substitute_type_params(message, substitutions),
            resource.as_deref(),
            &substitute_type_params(output, substitutions),
        );
    }
    if let Some((name, args)) = user_template_parts(type_name) {
        let args = args
            .iter()
            .map(|arg| substitute_type_params(arg, substitutions))
            .collect::<Vec<_>>();
        return format!("{name} OF {}", args.join(", "));
    }
    type_name.to_string()
}

pub(super) fn split_top_level_to(value: &str) -> Option<(String, String)> {
    value
        .split_once(" TO ")
        .map(|(left, right)| (left.to_string(), right.to_string()))
}

pub(super) fn split_top_level_commas(value: &str) -> Vec<String> {
    value.split(", ").map(str::to_string).collect()
}

/// Read each imported package's exported functions and collect the overloaded
/// ones (more than one export sharing a base name), keyed by the importer-facing
/// `binding.base` name. Also returns the set of `binding.`/`package.` qualifier
/// prefixes for argument-type normalization (plan-linker.md §12, overloads).
pub(super) fn collect_imported_overloads(
    project_dir: &Path,
    source: &AstProject,
) -> (HashMap<String, Vec<ImportedOverload>>, Vec<String>) {
    let mut overloads: HashMap<String, Vec<ImportedOverload>> = HashMap::new();
    let mut qualifiers: HashSet<String> = HashSet::new();
    // Distinct (binding, package) pairs across all files.
    let mut bindings: HashMap<String, String> = HashMap::new();
    for file in &source.files {
        for (binding, package) in file.import_bindings() {
            qualifiers.insert(format!("{binding}."));
            qualifiers.insert(format!("{package}."));
            bindings.insert(binding, package);
        }
    }
    for (binding, package) in &bindings {
        let package_file = project_dir.join("packages").join(format!("{package}.mfp"));
        let Ok(exports) = crate::binary_repr::read_package_exports(&package_file) else {
            continue;
        };
        // Group exported functions/subs by base name (the part before `$`).
        let mut by_base: HashMap<String, Vec<crate::binary_repr::BinaryReprExport>> =
            HashMap::new();
        for export in exports {
            if !matches!(
                export.kind,
                crate::binary_repr::BinaryReprExportKind::Func
                    | crate::binary_repr::BinaryReprExportKind::Sub
            ) {
                continue;
            }
            let base = export
                .name
                .split('$')
                .next()
                .unwrap_or(&export.name)
                .to_string();
            by_base.entry(base).or_default().push(export);
        }
        for (base, exports) in by_base {
            if exports.len() < 2 {
                continue; // Non-overloaded imports resolve by their bare name.
            }
            let entry = overloads.entry(format!("{binding}.{base}")).or_default();
            for export in exports {
                entry.push(ImportedOverload {
                    param_types: export
                        .params
                        .iter()
                        .map(|param| param.type_.clone())
                        .collect(),
                    qualified_name: format!("{package}.{}", export.name),
                });
            }
        }
    }
    (overloads, qualifiers.into_iter().collect())
}

pub(super) fn mangle_name(name: &str, args: &[String]) -> String {
    let suffix = args
        .iter()
        .map(|arg| sanitize_type_name(arg))
        .collect::<Vec<_>>()
        .join("$");
    format!("{name}${suffix}")
}

pub(super) fn overload_concrete_name(
    function: &Function,
    overloaded: bool,
    return_disambiguated: bool,
) -> String {
    if !overloaded && !return_disambiguated {
        return function.name.clone();
    }
    let mut args = function
        .params
        .iter()
        .map(|param| {
            param
                .type_name
                .clone()
                .unwrap_or_else(|| "Unknown".to_string())
        })
        .collect::<Vec<_>>();
    // Append an `AS <return type>` segment so two overloads differing only by
    // result type get distinct concrete symbols (plan-01-overload.md §F). `AS` is
    // a reserved keyword and can never be a parameter type, so the segment can
    // never collide with a parameter-distinguished overload's mangled name.
    if return_disambiguated {
        args.push("AS".to_string());
        args.push(
            function
                .return_type
                .clone()
                .unwrap_or_else(|| "Nothing".to_string()),
        );
    }
    mangle_name(&function.name, &args)
}

/// The internal overload-map key: `name(param,types) AS ReturnType`. The return
/// type is part of the key so a return-type overload set (§F.1) maps each member
/// to its own distinct concrete symbol.
pub(super) fn overload_key(name: &str, params: &[crate::ast::Param], return_type: Option<&str>) -> String {
    let params = params
        .iter()
        .map(|param| {
            param
                .type_name
                .clone()
                .unwrap_or_else(|| "Unknown".to_string())
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("{name}({params}) AS {}", return_type.unwrap_or("Nothing"))
}

/// Whether two functions have identical ordered parameter type lists (the
/// equivalence that defines a return-type overload set, §F.1).
pub(super) fn param_types_eq(a: &Function, b: &Function) -> bool {
    a.params.len() == b.params.len()
        && a.params
            .iter()
            .zip(&b.params)
            .all(|(x, y)| x.type_name == y.type_name)
}

/// Whether a function's parameter types exactly match an argument-type list (the
/// same exact-match rule ordinary overload resolution uses).
pub(super) fn params_match(function: &Function, arg_types: &[String]) -> bool {
    function.params.len() == arg_types.len()
        && function
            .params
            .iter()
            .zip(arg_types.iter())
            .all(|(param, actual)| param.type_name.as_deref() == Some(actual.as_str()))
}

pub(super) fn sanitize_type_name(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch
            } else {
                '$'
            }
        })
        .collect()
}

pub(super) fn numeric_binary_result_type(operator: &str, left: &str, right: &str) -> &'static str {
    numeric::binary_result_type(operator, left, right).unwrap_or(numeric::TYPE_INTEGER)
}

pub(super) fn promote_loop_numeric_type_name(start: &str, end: &str, step: &str) -> String {
    let first = numeric_binary_result_type("+", start, end);
    numeric_binary_result_type("+", first, step).to_string()
}

pub(super) fn constructor_arg_value(argument: &ConstructorArg) -> &Expression {
    match argument {
        ConstructorArg::Positional(value) => value,
        ConstructorArg::Named { value, .. } => value,
    }
}
