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
/// into its parameter types and return type for template unification. A parameter
/// may itself be a comma-bearing function type, so the split is paren-depth aware.
pub(super) fn func_type_parts(type_name: &str) -> Option<(Vec<&str>, &str)> {
    let rest = type_name
        .strip_prefix("FUNC(")
        .or_else(|| type_name.strip_prefix("ISOLATED FUNC("))?;
    crate::builtins::split_func_params_and_return(rest)
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

pub(super) fn substitute_type_params(
    type_name: &str,
    substitutions: &HashMap<String, String>,
) -> String {
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

/// The type arguments of `Name OF A, B` — split only on the commas at paren depth
/// 0, so a `FUNC(Integer, String) AS Boolean` argument stays one argument.
pub(super) fn split_top_level_commas(value: &str) -> Vec<String> {
    crate::builtins::split_top_level_commas(value)
        .into_iter()
        .map(str::to_string)
        .collect()
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
pub(super) fn overload_key(
    name: &str,
    params: &[crate::ast::Param],
    return_type: Option<&str>,
) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{FunctionKind, Param, Visibility};

    fn subs(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn param(name: &str, type_name: Option<&str>) -> Param {
        Param {
            name: name.to_string(),
            type_name: type_name.map(str::to_string),
            resource: false,
            state_type: None,
            default: None,
            line: 1,
        }
    }

    fn func(name: &str, params: Vec<Param>, return_type: Option<&str>) -> Function {
        Function {
            kind: FunctionKind::Func,
            visibility: Visibility::Private,
            isolated: false,
            name: name.to_string(),
            template_params: Vec::new(),
            params,
            return_type: return_type.map(str::to_string),
            return_resource: false,
            return_state_type: None,
            body: Vec::new(),
            trap: None,
            line: 1,
        }
    }

    #[test]
    fn unify_binds_and_checks_template_params() {
        let params = vec!["T".to_string()];
        let mut s = HashMap::new();
        // First occurrence binds T -> Integer.
        assert!(unify_type("T", "Integer", &params, &mut s));
        assert_eq!(s.get("T").map(String::as_str), Some("Integer"));
        // Second occurrence must agree.
        assert!(unify_type("T", "Integer", &params, &mut s));
        // Conflicting binding fails.
        assert!(!unify_type("T", "String", &params, &mut s));
    }

    #[test]
    fn unify_recurses_into_all_container_shapes() {
        let params = vec!["T".to_string(), "U".to_string()];
        let cases = [
            ("List OF T", "List OF Integer"),
            ("Result OF T", "Result OF String"),
            ("Map OF T TO U", "Map OF String TO Integer"),
            ("MapEntry OF T TO U", "MapEntry OF String TO Integer"),
            ("Box OF T", "Box OF Integer"),
            ("FUNC(T) AS U", "FUNC(Integer) AS String"),
        ];
        for (pattern, actual) in cases {
            let mut s = HashMap::new();
            assert!(
                unify_type(pattern, actual, &params, &mut s),
                "unify {pattern} vs {actual}"
            );
        }
    }

    #[test]
    fn unify_recurses_into_thread_shapes() {
        // Thread types unify by kind, message, optional resource, and output.
        let params = vec!["T".to_string(), "U".to_string()];
        let mut s = HashMap::new();
        assert!(unify_type(
            "Thread OF T TO U",
            "Thread OF Integer TO String",
            &params,
            &mut s
        ));
        // A thread with a resource clause on both sides unifies its resource slot.
        let mut s2 = HashMap::new();
        assert!(unify_type(
            "ThreadWorker OF T RES U TO Nothing",
            "ThreadWorker OF Integer RES String TO Nothing",
            &params,
            &mut s2
        ));
        // Resource present on one side only fails to unify.
        let mut s3 = HashMap::new();
        assert!(!unify_type(
            "Thread OF T RES U TO Nothing",
            "Thread OF Integer TO Nothing",
            &params,
            &mut s3
        ));
        // A thread pattern against a non-thread actual fails.
        let mut s4 = HashMap::new();
        assert!(!unify_type("Thread OF T TO U", "Integer", &params, &mut s4));
    }

    #[test]
    fn unify_rejects_mismatched_container_shapes() {
        let params = vec!["T".to_string(), "U".to_string()];
        let cases = [
            ("List OF T", "Integer"),
            ("Result OF T", "Integer"),
            ("Map OF T TO U", "Integer"),
            ("Map OF T TO U", "Map OF Integer"),
            ("MapEntry OF T TO U", "Integer"),
            ("MapEntry OF T TO U", "MapEntry OF Integer"),
            ("Box OF T", "Other OF Integer"),
            ("Box OF T, U", "Box OF Integer"),
            ("FUNC(T) AS U", "FUNC(Integer, String) AS Integer"),
            ("FUNC(T) AS U", "Integer"),
        ];
        for (pattern, actual) in cases {
            let mut s = HashMap::new();
            assert!(
                !unify_type(pattern, actual, &params, &mut s),
                "expected mismatch {pattern} vs {actual}"
            );
        }
    }

    #[test]
    fn unify_treats_unknown_actual_as_wildcard_and_matches_concretes() {
        let params: Vec<String> = Vec::new();
        let mut s = HashMap::new();
        assert!(unify_type("Integer", "Integer", &params, &mut s));
        assert!(unify_type("Integer", "Unknown", &params, &mut s));
        assert!(!unify_type("Integer", "String", &params, &mut s));
    }

    #[test]
    fn func_type_parts_handles_isolated_and_empty_params() {
        assert_eq!(
            func_type_parts("FUNC(Integer, String) AS Boolean"),
            Some((vec!["Integer", "String"], "Boolean"))
        );
        assert_eq!(
            func_type_parts("ISOLATED FUNC() AS Nothing"),
            Some((Vec::new(), "Nothing"))
        );
        assert_eq!(func_type_parts("Integer"), None);
        assert_eq!(func_type_parts("FUNC(Integer)"), None);
    }

    /// bug-35: a type argument that is itself a comma-bearing function type must
    /// survive the split, or unification and mangling operate on garbage.
    #[test]
    fn nested_function_type_arguments_are_not_shredded() {
        assert_eq!(
            func_type_parts("FUNC(FUNC(Integer, String) AS Boolean, Integer) AS Nothing"),
            Some((vec!["FUNC(Integer, String) AS Boolean", "Integer"], "Nothing"))
        );
        assert_eq!(
            func_type_parts("ISOLATED FUNC(FUNC(A, B) AS C) AS D"),
            Some((vec!["FUNC(A, B) AS C"], "D"))
        );
        // A two-argument template whose first argument is a two-parameter FUNC.
        assert_eq!(
            user_template_parts("Pair OF FUNC(Integer, String) AS Boolean, Integer"),
            Some((
                "Pair".to_string(),
                vec![
                    "FUNC(Integer, String) AS Boolean".to_string(),
                    "Integer".to_string()
                ]
            ))
        );
        // A nested user template argument keeps its own arguments.
        assert_eq!(
            split_top_level_commas("Pair OF Integer, String"),
            vec!["Pair OF Integer".to_string(), "String".to_string()]
        );
        assert_eq!(
            split_top_level_commas("FUNC(A, B) AS C, D"),
            vec!["FUNC(A, B) AS C".to_string(), "D".to_string()]
        );
    }

    /// Substitution walks the type arguments the depth-aware split produces, so a
    /// nested function-typed argument no longer swallows the argument after it.
    #[test]
    fn substitution_walks_each_top_level_type_argument() {
        let mut substitutions = HashMap::new();
        substitutions.insert("T".to_string(), "Integer".to_string());
        assert_eq!(
            substitute_type_params("Pair OF List OF T, T", &substitutions),
            "Pair OF List OF Integer, Integer"
        );
        assert_eq!(
            substitute_type_params("List OF T", &substitutions),
            "List OF Integer"
        );
    }

    #[test]
    fn user_template_parts_excludes_builtin_shapes() {
        assert_eq!(
            user_template_parts("Pair OF Integer, String"),
            Some((
                "Pair".to_string(),
                vec!["Integer".to_string(), "String".to_string()]
            ))
        );
        for builtin in [
            "List OF Integer",
            "Map OF Integer TO String",
            "MapEntry OF Integer TO String",
            "Result OF Integer",
            "Thread OF Integer",
            "ThreadWorker OF Integer",
            "FUNC(Integer) AS String",
            "ISOLATED FUNC() AS Nothing",
        ] {
            assert_eq!(user_template_parts(builtin), None, "{builtin}");
        }
        assert_eq!(user_template_parts("Integer"), None);
    }

    #[test]
    fn substitute_type_params_rewrites_every_shape() {
        let s = subs(&[("T", "Integer"), ("U", "String")]);
        assert_eq!(substitute_type_params("T", &s), "Integer");
        assert_eq!(substitute_type_params("List OF T", &s), "List OF Integer");
        assert_eq!(
            substitute_type_params("Result OF T", &s),
            "Result OF Integer"
        );
        assert_eq!(
            substitute_type_params("Map OF T TO U", &s),
            "Map OF Integer TO String"
        );
        assert_eq!(
            substitute_type_params("MapEntry OF T TO U", &s),
            "MapEntry OF Integer TO String"
        );
        assert_eq!(
            substitute_type_params("Pair OF T, U", &s),
            "Pair OF Integer, String"
        );
        // Thread shape substitutes message and output slots.
        assert_eq!(
            substitute_type_params("Thread OF T TO U", &s),
            "Thread OF Integer TO String"
        );
        // Unknown names pass through unchanged.
        assert_eq!(substitute_type_params("Boolean", &s), "Boolean");
        // Malformed Map (no TO) falls through to the identity return.
        assert_eq!(substitute_type_params("Map OF T", &s), "Map OF T");
        // Malformed MapEntry (no TO) also falls through.
        assert_eq!(substitute_type_params("MapEntry OF T", &s), "MapEntry OF T");
    }

    #[test]
    fn mangle_and_sanitize_encode_types() {
        assert_eq!(mangle_name("push", &["Integer".into()]), "push$Integer");
        assert_eq!(
            mangle_name("f", &["List OF Integer".into(), "String".into()]),
            "f$List$OF$Integer$String"
        );
        assert_eq!(sanitize_type_name("Map OF K TO V"), "Map$OF$K$TO$V");
        assert_eq!(sanitize_type_name("plain_1"), "plain_1");
    }

    #[test]
    fn overload_concrete_name_encodes_params_and_return() {
        let f = func("g", vec![param("a", Some("Integer"))], Some("String"));
        // Neither overloaded nor return-disambiguated: bare name.
        assert_eq!(overload_concrete_name(&f, false, false), "g");
        // Overloaded by params only.
        assert_eq!(overload_concrete_name(&f, true, false), "g$Integer");
        // Return-disambiguated appends the AS <return> segment.
        assert_eq!(
            overload_concrete_name(&f, true, true),
            "g$Integer$AS$String"
        );
        // Missing param/return types fall back to Unknown/Nothing.
        let bare = func("h", vec![param("a", None)], None);
        assert_eq!(
            overload_concrete_name(&bare, true, true),
            "h$Unknown$AS$Nothing"
        );
    }

    #[test]
    fn overload_key_includes_return_type() {
        let params = vec![param("a", Some("Integer")), param("b", None)];
        assert_eq!(
            overload_key("f", &params, Some("Boolean")),
            "f(Integer,Unknown) AS Boolean"
        );
        assert_eq!(overload_key("f", &[], None), "f() AS Nothing");
    }

    #[test]
    fn param_types_eq_and_params_match() {
        let a = func("f", vec![param("x", Some("Integer"))], None);
        let b = func("f", vec![param("y", Some("Integer"))], Some("String"));
        let c = func("f", vec![param("z", Some("String"))], None);
        assert!(param_types_eq(&a, &b));
        assert!(!param_types_eq(&a, &c));
        assert!(params_match(&a, &["Integer".to_string()]));
        assert!(!params_match(&a, &["String".to_string()]));
        assert!(!params_match(&a, &[]));
    }

    #[test]
    fn arg_slot_expected_only_for_call_arguments() {
        use crate::ast::Expression;
        let params = [param("a", Some("Integer"))];
        let call = Expression::Call {
            callee: "f".to_string(),
            arguments: Vec::new(),
            line: 1,
            column: 1,
        };
        assert_eq!(
            arg_slot_expected(&call, Some(&params), |p| p.first()),
            Some("Integer")
        );
        // Non-call arguments get no contextual type.
        let lit = Expression::Number("1".to_string());
        assert_eq!(arg_slot_expected(&lit, Some(&params), |p| p.first()), None);
        // No params available.
        assert_eq!(arg_slot_expected(&call, None, |p| p.first()), None);
    }

    #[test]
    fn constructor_arg_field_type_positional_and_named() {
        use crate::ast::Expression;
        let fields = [
            TypeField {
                visibility: None,
                name: "x".to_string(),
                type_name: "Integer".to_string(),
                line: 1,
            },
            TypeField {
                visibility: None,
                name: "y".to_string(),
                type_name: "String".to_string(),
                line: 1,
            },
        ];
        let pos = ConstructorArg::Positional(Expression::Number("1".to_string()));
        assert_eq!(
            constructor_arg_field_type(&pos, 1, Some(&fields)),
            Some("String")
        );
        let named = ConstructorArg::Named {
            name: "x".to_string(),
            value: Expression::Number("1".to_string()),
            line: 1,
        };
        assert_eq!(
            constructor_arg_field_type(&named, 0, Some(&fields)),
            Some("Integer")
        );
        // No fields known.
        assert_eq!(constructor_arg_field_type(&pos, 0, None), None);
    }

    #[test]
    fn arg_and_constructor_value_accessors() {
        use crate::ast::Expression;
        let pos = CallArg::Positional(Expression::Number("1".to_string()));
        let named = CallArg::Named {
            name: "a".to_string(),
            value: Expression::Number("2".to_string()),
            line: 1,
        };
        assert!(matches!(call_arg_value(&pos), Expression::Number(n) if n == "1"));
        assert!(matches!(call_arg_value(&named), Expression::Number(n) if n == "2"));
        let cpos = ConstructorArg::Positional(Expression::Number("3".to_string()));
        let cnamed = ConstructorArg::Named {
            name: "a".to_string(),
            value: Expression::Number("4".to_string()),
            line: 1,
        };
        assert!(matches!(constructor_arg_value(&cpos), Expression::Number(n) if n == "3"));
        assert!(matches!(constructor_arg_value(&cnamed), Expression::Number(n) if n == "4"));
    }

    #[test]
    fn numeric_result_and_loop_promotion() {
        assert_eq!(
            numeric_binary_result_type("+", "Integer", "Integer"),
            "Integer"
        );
        assert_eq!(numeric_binary_result_type("+", "Integer", "Float"), "Float");
        // A Float bound anywhere in a FOR loop promotes the counter type.
        assert_eq!(
            promote_loop_numeric_type_name("Integer", "Float", "Integer"),
            "Float"
        );
        assert_eq!(
            promote_loop_numeric_type_name("Integer", "Integer", "Integer"),
            "Integer"
        );
    }

    #[test]
    fn split_helpers() {
        assert_eq!(
            split_top_level_to("Integer TO String"),
            Some(("Integer".to_string(), "String".to_string()))
        );
        assert_eq!(split_top_level_to("Integer"), None);
        assert_eq!(
            split_top_level_commas("Integer, String"),
            vec!["Integer".to_string(), "String".to_string()]
        );
    }

    #[test]
    fn collect_imported_overloads_empty_without_imports() {
        // A project with no import bindings and no packages directory yields no
        // overloads and no qualifiers.
        let dir = std::env::temp_dir();
        let project = AstProject {
            name: "p".to_string(),
            files: Vec::new(),
        };
        let (overloads, qualifiers) = collect_imported_overloads(&dir, &project);
        assert!(overloads.is_empty());
        assert!(qualifiers.is_empty());
    }

    #[test]
    fn collect_imported_overloads_reads_package_overload_set() {
        // Import a real compiled package whose exports include overload sets
        // (`score$`/`score$Vec2`, `mark$`/`mark$Vec2`) so the by-base grouping,
        // the ≥2 overload gate, and the qualifier collection all run.
        let fixture = crate::testutil::fixture_dir("package-simple")
            .join("golden")
            .join("package_simple.mfp");
        let dir = tempfile::tempdir().expect("tempdir");
        let packages = dir.path().join("packages");
        std::fs::create_dir_all(&packages).unwrap();
        std::fs::copy(&fixture, packages.join("package_simple.mfp")).unwrap();

        let src = "IMPORT package_simple\nFUNC main() AS Integer\n  RETURN 0\nEND FUNC\n";
        let file =
            crate::ast::parse_source(std::path::Path::new("src/main.mfb"), "src/main.mfb", src)
                .expect("parse");
        let project = AstProject {
            name: "app".to_string(),
            files: vec![file],
        };

        let (overloads, qualifiers) = collect_imported_overloads(dir.path(), &project);
        // Overload sets are keyed by `binding.base`.
        assert!(
            overloads.contains_key("package_simple.score"),
            "keys: {:?}",
            overloads.keys().collect::<Vec<_>>()
        );
        let score = &overloads["package_simple.score"];
        assert!(score.len() >= 2);
        // Each collected overload carries the package-qualified mangled name.
        assert!(score
            .iter()
            .all(|o| o.qualified_name.starts_with("package_simple.score")));
        // The binding/package qualifier prefix is captured.
        assert!(qualifiers.iter().any(|q| q == "package_simple."));
    }
}
