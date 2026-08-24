use super::*;

/// The `Option<String>` an AST type field reconstructs to from a HIR bare type:
/// [`ParameterType::Unknown`] (an absent `AS T` annotation) → `None`, any concrete
/// type → `Some(rendered)`. Mirrors `crate::hir::unrender_optional_type`, so every
/// type STRING the monomorph string algorithm reads is byte-identical to the
/// pre-D3 AST `type_name`/`return_type` field it replaced (parse↔name round-trips
/// byte-exact).
pub(super) fn opt_type_name(type_: &ParameterType) -> Option<String> {
    match type_ {
        ParameterType::Unknown => None,
        other => Some(other.name().into_owned()),
    }
}

/// The expected (contextual) type for an argument slot: the selected parameter's
/// declared type, but only when the argument is itself a call — the one position
/// where a return-type overload set needs the context to resolve
/// (plan-01-overload.md §F.2). Returns `None` otherwise so literals keep their own
/// inferred typing.
pub(super) fn arg_slot_expected<'a>(
    value: &HirExpression,
    params: Option<&'a [HirParam]>,
    select: impl FnOnce(&'a [HirParam]) -> Option<&'a HirParam>,
) -> Option<String> {
    if !matches!(value, HirExpression::Call { .. }) {
        return None;
    }
    opt_type_name(&select(params?)?.type_)
}

pub(super) fn call_arg_value(argument: &HirCallArg) -> &HirExpression {
    match argument {
        HirCallArg::Positional(value) => value,
        HirCallArg::Named { value, .. } => value,
    }
}

pub(super) fn constructor_arg_field_type<'a>(
    argument: &HirConstructorArg,
    index: usize,
    fields: Option<&'a [HirTypeField]>,
) -> Option<String> {
    let fields = fields?;
    match argument {
        HirConstructorArg::Positional(_) => {
            fields.get(index).map(|field| field.type_.name().into_owned())
        }
        HirConstructorArg::Named { name, .. } => fields
            .iter()
            .find(|field| field.name == *name)
            .map(|field| field.type_.name().into_owned()),
    }
}

/// The bindable [`Symbol`](crate::intern::Symbol) of a *leaf* type — a
/// [`Var`](ParameterType::Var) (a minted type variable) or a bare
/// [`Named`](ParameterType::Named) (a nominal whose whole name might itself be a
/// template parameter). Container/`Func`/`Res` types have no leaf symbol.
///
/// The historical string algorithm bound a type variable by testing whether the
/// *whole* pattern string equalled a declared template-param name
/// (`params.iter().any(|p| p == pattern)`) — a bare `Named`/`Var` leaf. A user
/// generic like `Pair OF Integer, String` parses to a single `Named` whose name is
/// never a bare param, so it correctly returns its (non-matching) symbol here and
/// falls through to the user-generic string fallback.
fn leaf_param_symbol(type_: &ParameterType) -> Option<crate::intern::Symbol> {
    match type_ {
        ParameterType::Var(sym) | ParameterType::Named(sym) => Some(*sym),
        _ => None,
    }
}

/// Structurally unify a template `pattern` type against a concrete `actual` type,
/// binding each template parameter (a [`Var`](ParameterType::Var), or a
/// [`Named`](ParameterType::Named) whose symbol is in `params`) in `substitutions`.
///
/// This is the native-`ParameterType` successor of the historical string
/// algorithm; it mirrors every arm of that algorithm exactly so the *result*
/// (which type-args a call infers) is byte-identical:
///
/// * A leaf param binds on first sight and must agree on re-occurrence.
/// * The six built-in container shapes (`List`/`Set`/`Result`/`Map`/`MapEntry`/
///   thread handle) require the *actual* to be the same container — a mismatch is a
///   hard `false`, **not** the trailing `Unknown` wildcard (the string arms
///   `return false` before reaching it).
/// * A *user generic* (a `Named` leaf whose name is `Name OF a, b`) is not a
///   distinct variant — it falls back to the string algorithm (render both `.name()`,
///   split with [`user_template_parts`], recurse on the re-parsed arguments), so
///   `ParameterType::parse` gains no new variant and results are preserved.
/// * A `FUNC(...)` pattern unifies only against a `FUNC(...)` actual (isolation is
///   ignored, exactly as the string `func_type_parts` erased the `ISOLATED`
///   marker); a non-func actual falls through to the wildcard tail.
/// * The tail is `pattern == actual || actual is Unknown` — structural equality is
///   byte-identical to the old string equality because `parse`/`name` is a
///   bijection over this vocabulary, and the concrete side is never a `Var`.
pub(super) fn unify_type(
    pattern: &ParameterType,
    actual: &ParameterType,
    params: &HashSet<crate::intern::Symbol>,
    substitutions: &mut HashMap<crate::intern::Symbol, ParameterType>,
) -> bool {
    // A bare template parameter binds/checks. Mirrors the string rule
    // `params.iter().any(|p| p == pattern)` over the whole pattern name.
    if let Some(sym) = leaf_param_symbol(pattern) {
        if params.contains(&sym) {
            if let Some(existing) = substitutions.get(&sym) {
                return existing == actual;
            }
            substitutions.insert(sym, actual.clone());
            return true;
        }
    }

    // The built-in container shapes: the actual must be the same container, else a
    // hard `false` (the string arms `return false` on a strip mismatch, never
    // reaching the `Unknown` wildcard).
    match pattern {
        ParameterType::ListOf(pattern_element) => {
            let ParameterType::ListOf(actual_element) = actual else {
                return false;
            };
            return unify_type(pattern_element, actual_element, params, substitutions);
        }
        ParameterType::SetOf(pattern_element) => {
            let ParameterType::SetOf(actual_element) = actual else {
                return false;
            };
            return unify_type(pattern_element, actual_element, params, substitutions);
        }
        ParameterType::ResultOf(pattern_success) => {
            let ParameterType::ResultOf(actual_success) = actual else {
                return false;
            };
            return unify_type(pattern_success, actual_success, params, substitutions);
        }
        ParameterType::MapOf(pattern_key, pattern_value) => {
            let ParameterType::MapOf(actual_key, actual_value) = actual else {
                return false;
            };
            return unify_type(pattern_key, actual_key, params, substitutions)
                && unify_type(pattern_value, actual_value, params, substitutions);
        }
        ParameterType::MapEntryOf(pattern_key, pattern_value) => {
            let ParameterType::MapEntryOf(actual_key, actual_value) = actual else {
                return false;
            };
            return unify_type(pattern_key, actual_key, params, substitutions)
                && unify_type(pattern_value, actual_value, params, substitutions);
        }
        ParameterType::ThreadHandle {
            worker: pattern_worker,
            msg: pattern_message,
            res: pattern_resource,
            out: pattern_output,
        } => {
            let ParameterType::ThreadHandle {
                worker: actual_worker,
                msg: actual_message,
                res: actual_resource,
                out: actual_output,
            } = actual
            else {
                return false;
            };
            // The resource plane defaults to `Nothing` when the handle carries no
            // `RES` clause; the string algorithm saw that as `None`, so an absent
            // plane on one side only fails to unify.
            let resource_unifies = match (
                matches!(pattern_resource.as_ref(), ParameterType::Nothing),
                matches!(actual_resource.as_ref(), ParameterType::Nothing),
            ) {
                (true, true) => true,
                (false, false) => {
                    unify_type(pattern_resource, actual_resource, params, substitutions)
                }
                _ => false,
            };
            return pattern_worker == actual_worker
                && unify_type(pattern_message, actual_message, params, substitutions)
                && resource_unifies
                && unify_type(pattern_output, actual_output, params, substitutions);
        }
        _ => {}
    }

    // A user generic (`Name OF a, b`, a single `Named`) is not a distinct variant:
    // fall back to the string algorithm for this subtree so `parse` behavior is
    // unchanged. Mirrors the old `(user_template_parts(p), user_template_parts(a))`
    // arm — only fires when *both* sides split; otherwise fall through.
    let pattern_name = pattern.name();
    if let Some((pattern_base, pattern_args)) = user_template_parts(&pattern_name) {
        let actual_name = actual.name();
        if let Some((actual_base, actual_args)) = user_template_parts(&actual_name) {
            return pattern_base == actual_base
                && pattern_args.len() == actual_args.len()
                && pattern_args.iter().zip(actual_args.iter()).all(|(p, a)| {
                    unify_type(
                        &ParameterType::parse(p),
                        &ParameterType::parse(a),
                        params,
                        substitutions,
                    )
                });
        }
    }

    // A `FUNC(...)` pattern unifies only against a `FUNC(...)` actual (isolation
    // ignored, as `func_type_parts` erased the `ISOLATED` marker); a non-func actual
    // falls through to the wildcard tail.
    if let (
        ParameterType::Func(pattern_params, pattern_ret, _),
        ParameterType::Func(actual_params, actual_ret, _),
    ) = (pattern, actual)
    {
        return pattern_params.len() == actual_params.len()
            && pattern_params
                .iter()
                .zip(actual_params.iter())
                .all(|(p, a)| unify_type(p, a, params, substitutions))
            && unify_type(pattern_ret, actual_ret, params, substitutions);
    }

    pattern == actual || matches!(actual, ParameterType::Unknown)
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
        || type_name.starts_with("Set OF ")
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

/// Rebuild `type_` with each bound template parameter replaced by its
/// substitution, structurally. The native-`ParameterType` successor of the
/// historical string algorithm; every arm mirrors it exactly so the substituted
/// *result* is byte-identical:
///
/// * A bare param leaf ([`Var`](ParameterType::Var) or a [`Named`](ParameterType::Named)
///   whose whole name is a param) becomes its bound type.
/// * The six built-in container shapes recurse into their children.
/// * A thread handle substitutes its message/output always and its resource plane
///   only when present (`Nothing` == an absent `RES` clause, preserved as-is).
/// * Everything else falls back to the string tail: a *user generic* (`Name OF a, b`)
///   substitutes its arguments string-wise and re-`parse`s the reassembled name;
///   any other type (a scalar, a concrete nominal, and — matching the old algorithm,
///   which had no `FUNC`/`RES` arm — a `FUNC(...)` or `RES` type) is returned
///   unchanged.
pub(super) fn substitute_type_params(
    type_: &ParameterType,
    substitutions: &HashMap<crate::intern::Symbol, ParameterType>,
) -> ParameterType {
    if let Some(sym) = leaf_param_symbol(type_) {
        if let Some(value) = substitutions.get(&sym) {
            return value.clone();
        }
    }
    match type_ {
        ParameterType::ListOf(element) => {
            ParameterType::list_of(substitute_type_params(element, substitutions))
        }
        ParameterType::SetOf(element) => {
            ParameterType::set_of(substitute_type_params(element, substitutions))
        }
        ParameterType::ResultOf(success) => {
            ParameterType::result_of(substitute_type_params(success, substitutions))
        }
        ParameterType::MapOf(key, value) => ParameterType::map_of(
            substitute_type_params(key, substitutions),
            substitute_type_params(value, substitutions),
        ),
        ParameterType::MapEntryOf(key, value) => ParameterType::map_entry_of(
            substitute_type_params(key, substitutions),
            substitute_type_params(value, substitutions),
        ),
        ParameterType::ThreadHandle {
            worker,
            msg,
            res,
            out,
        } => {
            // The resource plane is only substituted when present; an absent plane
            // (`Nothing`) is preserved, mirroring the string algorithm's
            // `resource.map(...)` over the optional `RES` clause.
            let resource = match res.as_ref() {
                ParameterType::Nothing => ParameterType::Nothing,
                other => substitute_type_params(other, substitutions),
            };
            ParameterType::thread_handle(
                *worker,
                substitute_type_params(msg, substitutions),
                resource,
                substitute_type_params(out, substitutions),
            )
        }
        other => {
            // The string tail. A user generic splits, substitutes its arguments, and
            // re-`parse`s the reassembled `Name OF a, b`; any other type (scalar,
            // nominal, `FUNC(...)`, `RES` — none of which the old algorithm descended
            // into) is identity.
            let name = other.name();
            if let Some((base, args)) = user_template_parts(&name) {
                let args = args
                    .iter()
                    .map(|arg| {
                        substitute_type_params(&ParameterType::parse(arg), substitutions)
                            .name()
                            .into_owned()
                    })
                    .collect::<Vec<_>>();
                ParameterType::parse(&format!("{base} OF {}", args.join(", ")))
            } else {
                other.clone()
            }
        }
    }
}

pub(super) fn split_top_level_to(value: &str) -> Option<(String, String)> {
    split_top_level_to_str(value).map(|(left, right)| (left.to_string(), right.to_string()))
}

/// Split a `Map`/`MapEntry` body `K TO V` on the ` TO ` that separates the outer
/// key from its value. A leftmost `split_once(" TO ")` mis-parses a key that
/// itself carries a top-level ` TO ` (`Map OF Map OF String TO Integer TO
/// Boolean`, bug-108.2). This mirrors `syntaxcheck::types::split_map_body`:
/// separators inside parenthesized / `FUNC(...)` groups are skipped, and so is
/// the ` TO ` owned by each nested `Map`/`MapEntry`/`Thread`/`ThreadWorker`
/// sub-type. Returns `None` when there is no top-level ` TO `.
fn split_top_level_to_str(body: &str) -> Option<(&str, &str)> {
    let bytes = body.as_bytes();
    let mut depth: usize = 0;
    // Nested `OF`-constructs seen at depth 0 whose ` TO ` has not yet appeared.
    let mut pending: usize = 0;
    let mut index = 0;
    while index < body.len() {
        match bytes[index] {
            b'(' => {
                depth += 1;
                index += 1;
            }
            b')' => {
                depth = depth.saturating_sub(1);
                index += 1;
            }
            // `is_char_boundary` guards the slice: `.mfp`-decoded type strings are
            // not guaranteed ASCII, so `index` can land on a UTF-8 continuation
            // byte where `body[index..]` would panic (bug-169). A non-boundary
            // byte never begins ` TO ` nor a keyword, so skipping it is correct.
            _ if depth == 0
                && body.is_char_boundary(index)
                && body[index..].starts_with(" TO ") =>
            {
                if pending > 0 {
                    pending -= 1;
                    index += 4;
                } else {
                    return Some((&body[..index], &body[index + 4..]));
                }
            }
            _ if depth == 0
                && body.is_char_boundary(index)
                && type_owns_a_to_separator(body, index) =>
            {
                pending += 1;
                index += 1;
            }
            _ => index += 1,
        }
    }
    None
}

/// Whether a `Map`/`MapEntry`/`Thread`/`ThreadWorker` `OF`-construct — each of
/// which owns exactly one top-level ` TO ` — begins at byte `at` of `body`. The
/// keyword must sit on a word boundary so a template whose name merely ends in
/// `Map` is not counted.
fn type_owns_a_to_separator(body: &str, at: usize) -> bool {
    let bytes = body.as_bytes();
    if at > 0 {
        let prev = bytes[at - 1];
        if prev.is_ascii_alphanumeric()
            || prev == b'_'
            || prev == b'.'
            || prev == b':'
            || prev >= 0x80
        {
            return false;
        }
    }
    ["MapEntry OF ", "ThreadWorker OF ", "Map OF ", "Thread OF "]
        .iter()
        .any(|keyword| body[at..].starts_with(keyword))
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
    source: &HirProject,
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
                    // Rewrite to the importer-facing `binding.name`, not
                    // `package.name`: the post-monomorph resolver maps import
                    // bindings (binding→package), so a `package.name` target is
                    // unresolvable when the file imported under an alias
                    // (`IMPORT pkg AS radio`). When the import is unaliased,
                    // binding == package, so this is unchanged (bug-104).
                    qualified_name: format!("{binding}.{}", export.name),
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
    function: &HirFunction,
    overloaded: bool,
    return_disambiguated: bool,
) -> String {
    if !overloaded && !return_disambiguated {
        return function.name.clone();
    }
    let mut args = function
        .params
        .iter()
        .map(|param| opt_type_name(&param.type_).unwrap_or_else(|| "Unknown".to_string()))
        .collect::<Vec<_>>();
    // Append an `AS <return type>` segment so two overloads differing only by
    // result type get distinct concrete symbols (plan-01-overload.md §F). `AS` is
    // a reserved keyword and can never be a parameter type, so the segment can
    // never collide with a parameter-distinguished overload's mangled name.
    if return_disambiguated {
        args.push("AS".to_string());
        args.push(opt_type_name(&function.returns).unwrap_or_else(|| "Nothing".to_string()));
    }
    mangle_name(&function.name, &args)
}

/// The internal overload-map key: `name(param,types) AS ReturnType`. The return
/// type is part of the key so a return-type overload set (§F.1) maps each member
/// to its own distinct concrete symbol.
pub(super) fn overload_key(
    name: &str,
    params: &[HirParam],
    return_type: Option<&str>,
) -> String {
    let params = params
        .iter()
        .map(|param| opt_type_name(&param.type_).unwrap_or_else(|| "Unknown".to_string()))
        .collect::<Vec<_>>()
        .join(",");
    format!("{name}({params}) AS {}", return_type.unwrap_or("Nothing"))
}

/// Whether two functions have identical ordered parameter type lists (the
/// equivalence that defines a return-type overload set, §F.1).
pub(super) fn param_types_eq(a: &HirFunction, b: &HirFunction) -> bool {
    a.params.len() == b.params.len()
        && a.params
            .iter()
            .zip(&b.params)
            .all(|(x, y)| x.type_ == y.type_)
}

/// Whether a function's parameter types exactly match an argument-type list (the
/// same exact-match rule ordinary overload resolution uses).
pub(super) fn params_match(function: &HirFunction, arg_types: &[String]) -> bool {
    function.params.len() == arg_types.len()
        && function
            .params
            .iter()
            .zip(arg_types.iter())
            .all(|(param, actual)| opt_type_name(&param.type_).as_deref() == Some(actual.as_str()))
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

pub(super) fn constructor_arg_value(argument: &HirConstructorArg) -> &HirExpression {
    match argument {
        HirConstructorArg::Positional(value) => value,
        HirConstructorArg::Named { value, .. } => value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{FunctionKind, Visibility};
    use crate::hir::{
        HirCallArg, HirConstructorArg, HirExpression, HirFunction, HirParam, HirTypeField,
    };
    use crate::types::ParameterType;

    fn subs(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// String adapter over the native [`unify_type`], so the historical
    /// string-level cases keep asserting the same behavior through the
    /// `parse`↔`name` round-trip (which is byte-exact).
    fn unify_str(
        pattern: &str,
        actual: &str,
        params: &[String],
        subs: &mut HashMap<String, String>,
    ) -> bool {
        let param_set: std::collections::HashSet<crate::intern::Symbol> = params
            .iter()
            .map(|p| crate::intern::Symbol::intern(p))
            .collect();
        let mut native: HashMap<crate::intern::Symbol, ParameterType> = subs
            .iter()
            .map(|(k, v)| (crate::intern::Symbol::intern(k), ParameterType::parse(v)))
            .collect();
        let ok = unify_type(
            &ParameterType::parse(pattern),
            &ParameterType::parse(actual),
            &param_set,
            &mut native,
        );
        *subs = native
            .iter()
            .map(|(k, v)| (k.resolve().to_string(), v.name().into_owned()))
            .collect();
        ok
    }

    /// String adapter over the native [`substitute_type_params`] (same rationale).
    fn substitute_str(type_name: &str, subs: &HashMap<String, String>) -> String {
        let native: HashMap<crate::intern::Symbol, ParameterType> = subs
            .iter()
            .map(|(k, v)| (crate::intern::Symbol::intern(k), ParameterType::parse(v)))
            .collect();
        substitute_type_params(&ParameterType::parse(type_name), &native)
            .name()
            .into_owned()
    }

    fn param(name: &str, type_name: Option<&str>) -> HirParam {
        HirParam {
            name: name.to_string(),
            type_: type_name.map_or(ParameterType::Unknown, ParameterType::parse),
            resource: false,
            state_type: None,
            default: None,
            line: 1,
        }
    }

    fn func(name: &str, params: Vec<HirParam>, return_type: Option<&str>) -> HirFunction {
        HirFunction {
            kind: FunctionKind::Func,
            visibility: Visibility::Private,
            isolated: false,
            name: name.to_string(),
            template_params: Vec::new(),
            params,
            returns: return_type.map_or(ParameterType::Unknown, ParameterType::parse),
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
        assert!(unify_str("T", "Integer", &params, &mut s));
        assert_eq!(s.get("T").map(String::as_str), Some("Integer"));
        // Second occurrence must agree.
        assert!(unify_str("T", "Integer", &params, &mut s));
        // Conflicting binding fails.
        assert!(!unify_str("T", "String", &params, &mut s));
    }

    #[test]
    fn unify_recurses_into_all_container_shapes() {
        let params = vec!["T".to_string(), "U".to_string()];
        let cases = [
            ("List OF T", "List OF Integer"),
            ("Set OF T", "Set OF Integer"),
            ("Result OF T", "Result OF String"),
            ("Map OF T TO U", "Map OF String TO Integer"),
            ("MapEntry OF T TO U", "MapEntry OF String TO Integer"),
            ("Box OF T", "Box OF Integer"),
            ("FUNC(T) AS U", "FUNC(Integer) AS String"),
        ];
        for (pattern, actual) in cases {
            let mut s = HashMap::new();
            assert!(
                unify_str(pattern, actual, &params, &mut s),
                "unify {pattern} vs {actual}"
            );
        }
    }

    #[test]
    fn unify_recurses_into_thread_shapes() {
        // Thread types unify by kind, message, optional resource, and output.
        let params = vec!["T".to_string(), "U".to_string()];
        let mut s = HashMap::new();
        assert!(unify_str(
            "Thread OF T TO U",
            "Thread OF Integer TO String",
            &params,
            &mut s
        ));
        // A thread with a resource clause on both sides unifies its resource slot.
        let mut s2 = HashMap::new();
        assert!(unify_str(
            "ThreadWorker OF T RES U TO Nothing",
            "ThreadWorker OF Integer RES String TO Nothing",
            &params,
            &mut s2
        ));
        // Resource present on one side only fails to unify.
        let mut s3 = HashMap::new();
        assert!(!unify_str(
            "Thread OF T RES U TO Nothing",
            "Thread OF Integer TO Nothing",
            &params,
            &mut s3
        ));
        // A thread pattern against a non-thread actual fails.
        let mut s4 = HashMap::new();
        assert!(!unify_str("Thread OF T TO U", "Integer", &params, &mut s4));
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
                !unify_str(pattern, actual, &params, &mut s),
                "expected mismatch {pattern} vs {actual}"
            );
        }
    }

    #[test]
    fn unify_treats_unknown_actual_as_wildcard_and_matches_concretes() {
        let params: Vec<String> = Vec::new();
        let mut s = HashMap::new();
        assert!(unify_str("Integer", "Integer", &params, &mut s));
        assert!(unify_str("Integer", "Unknown", &params, &mut s));
        assert!(!unify_str("Integer", "String", &params, &mut s));
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
            Some((
                vec!["FUNC(Integer, String) AS Boolean", "Integer"],
                "Nothing"
            ))
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
            substitute_str("Pair OF List OF T, T", &substitutions),
            "Pair OF List OF Integer, Integer"
        );
        assert_eq!(
            substitute_str("List OF T", &substitutions),
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
            "Set OF Integer",
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
        assert_eq!(substitute_str("T", &s), "Integer");
        assert_eq!(substitute_str("List OF T", &s), "List OF Integer");
        assert_eq!(substitute_str("Set OF T", &s), "Set OF Integer");
        assert_eq!(
            substitute_str("Result OF T", &s),
            "Result OF Integer"
        );
        assert_eq!(
            substitute_str("Map OF T TO U", &s),
            "Map OF Integer TO String"
        );
        assert_eq!(
            substitute_str("MapEntry OF T TO U", &s),
            "MapEntry OF Integer TO String"
        );
        assert_eq!(
            substitute_str("Pair OF T, U", &s),
            "Pair OF Integer, String"
        );
        // Thread shape substitutes message and output slots.
        assert_eq!(
            substitute_str("Thread OF T TO U", &s),
            "Thread OF Integer TO String"
        );
        // Unknown names pass through unchanged.
        assert_eq!(substitute_str("Boolean", &s), "Boolean");
        // Malformed Map (no TO) falls through to the identity return.
        assert_eq!(substitute_str("Map OF T", &s), "Map OF T");
        // Malformed MapEntry (no TO) also falls through.
        assert_eq!(substitute_str("MapEntry OF T", &s), "MapEntry OF T");
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
        let params = [param("a", Some("Integer"))];
        let call = HirExpression::Call {
            callee: "f".to_string(),
            arguments: Vec::new(),
            line: 1,
            column: 1,
        };
        assert_eq!(
            arg_slot_expected(&call, Some(&params), |p| p.first()).as_deref(),
            Some("Integer")
        );
        // Non-call arguments get no contextual type.
        let lit = HirExpression::Number("1".to_string());
        assert_eq!(arg_slot_expected(&lit, Some(&params), |p| p.first()), None);
        // No params available.
        assert_eq!(arg_slot_expected(&call, None, |p| p.first()), None);
    }

    #[test]
    fn constructor_arg_field_type_positional_and_named() {
        let fields = [
            HirTypeField {
                visibility: None,
                name: "x".to_string(),
                type_: ParameterType::parse("Integer"),
                line: 1,
            },
            HirTypeField {
                visibility: None,
                name: "y".to_string(),
                type_: ParameterType::parse("String"),
                line: 1,
            },
        ];
        let pos = HirConstructorArg::Positional(HirExpression::Number("1".to_string()));
        assert_eq!(
            constructor_arg_field_type(&pos, 1, Some(&fields)).as_deref(),
            Some("String")
        );
        let named = HirConstructorArg::Named {
            name: "x".to_string(),
            value: HirExpression::Number("1".to_string()),
            line: 1,
        };
        assert_eq!(
            constructor_arg_field_type(&named, 0, Some(&fields)).as_deref(),
            Some("Integer")
        );
        // No fields known.
        assert_eq!(constructor_arg_field_type(&pos, 0, None), None);
    }

    #[test]
    fn arg_and_constructor_value_accessors() {
        let pos = HirCallArg::Positional(HirExpression::Number("1".to_string()));
        let named = HirCallArg::Named {
            name: "a".to_string(),
            value: HirExpression::Number("2".to_string()),
            line: 1,
        };
        assert!(matches!(call_arg_value(&pos), HirExpression::Number(n) if n == "1"));
        assert!(matches!(call_arg_value(&named), HirExpression::Number(n) if n == "2"));
        let cpos = HirConstructorArg::Positional(HirExpression::Number("3".to_string()));
        let cnamed = HirConstructorArg::Named {
            name: "a".to_string(),
            value: HirExpression::Number("4".to_string()),
            line: 1,
        };
        assert!(matches!(constructor_arg_value(&cpos), HirExpression::Number(n) if n == "3"));
        assert!(matches!(constructor_arg_value(&cnamed), HirExpression::Number(n) if n == "4"));
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
        let project = crate::ast::AstProject {
            name: "p".to_string(),
            files: Vec::new(),
        };
        let (overloads, qualifiers) = collect_imported_overloads(&dir, &crate::hir::elaborate(&project));
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
        let project = crate::ast::AstProject {
            name: "app".to_string(),
            files: vec![file],
        };

        let (overloads, qualifiers) =
            collect_imported_overloads(dir.path(), &crate::hir::elaborate(&project));
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
