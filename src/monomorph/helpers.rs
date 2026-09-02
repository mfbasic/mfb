use super::*;
#[cfg(test)]
use crate::operators::BinaryOp;

/// The declared type of an optionally-annotated slot: [`ParameterType::Unknown`]
/// (an absent `AS T` annotation) → `None`, any concrete type → `Some(type)`.
///
/// plan-106-A: yields the type itself. It used to render `.name()`, which made
/// every downstream comparison a string compare — the `Unknown`-means-absent
/// distinction is the only thing this helper actually encodes.
pub(super) fn opt_type(type_: &ParameterType) -> Option<ParameterType> {
    match type_ {
        ParameterType::Unknown => None,
        other => Some(other.clone()),
    }
}

/// The render-out companion to [`opt_type`], for the SYMBOL-MANGLING sites
/// ([`mangle_name`], [`overload_key`]) where the product is deliberately a
/// string: a concrete symbol and a map key. Expressed over [`opt_type`] so the
/// "`Unknown` means absent" rule has exactly one definition.
pub(super) fn opt_type_name(type_: &ParameterType) -> Option<String> {
    opt_type(type_).map(|type_| type_.name().into_owned())
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
) -> Option<ParameterType> {
    if !matches!(value, HirExpression::Call { .. }) {
        return None;
    }
    opt_type(&select(params?)?.type_)
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
) -> Option<ParameterType> {
    let fields = fields?;
    match argument {
        HirConstructorArg::Positional(_) => fields.get(index).map(|field| field.type_.clone()),
        HirConstructorArg::Named { name, .. } => fields
            .iter()
            .find(|field| field.name == *name)
            .map(|field| field.type_.clone()),
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
        // plan-113: a C ABI spelling is a bare token like any other leaf, and
        // `TYPE Box OF CPtr` may name a template parameter with one. Before the
        // `C` variant it arrived here as a `Named` and matched; a spelling that
        // reached monomorph re-parsed (rather than classified by `with_vars`)
        // still must.
        ParameterType::C(ctype) => Some(crate::intern::Symbol::intern(ctype.name())),
        _ => None,
    }
}

/// Whether `specific` is `general` with some [`Unknown`](ParameterType::Unknown)
/// leaves replaced by concrete types — i.e. the two describe the SAME shape and
/// `general` simply carries less information about it.
///
/// This is what makes a provisional template binding refinable (bug-442). An empty
/// `[]` literal types as `List OF Unknown`, so a parameter bound from it holds an
/// `Unknown` *nested inside a container*; a later argument spelling the same shape
/// concretely (`List OF Integer`) must win rather than conflict.
///
/// Reflexive: `refines(t, t)` is true for every `t` (zero replacements), so the
/// equal-bindings case falls out and re-inserting the identical type is a no-op.
fn refines(general: &ParameterType, specific: &ParameterType) -> bool {
    if matches!(general, ParameterType::Unknown) {
        return true;
    }
    match (general, specific) {
        (ParameterType::ListOf(a), ParameterType::ListOf(b))
        | (ParameterType::SetOf(a), ParameterType::SetOf(b))
        | (ParameterType::ResultOf(a), ParameterType::ResultOf(b))
        | (ParameterType::Res(a), ParameterType::Res(b)) => refines(a, b),
        (ParameterType::MapOf(ak, av), ParameterType::MapOf(bk, bv))
        | (ParameterType::MapEntryOf(ak, av), ParameterType::MapEntryOf(bk, bv)) => {
            refines(ak, bk) && refines(av, bv)
        }
        (ParameterType::UserOf(a_name, a_args), ParameterType::UserOf(b_name, b_args)) => {
            a_name == b_name
                && a_args.len() == b_args.len()
                && a_args.iter().zip(b_args.iter()).all(|(a, b)| refines(a, b))
        }
        (
            ParameterType::Func(a_params, a_ret, a_isolated),
            ParameterType::Func(b_params, b_ret, b_isolated),
        ) => {
            a_isolated == b_isolated
                && a_params.len() == b_params.len()
                && a_params
                    .iter()
                    .zip(b_params.iter())
                    .all(|(a, b)| refines(a, b))
                && refines(a_ret, b_ret)
        }
        (
            ParameterType::ThreadHandle {
                worker: a_worker,
                msg: a_msg,
                res: a_res,
                out: a_out,
            },
            ParameterType::ThreadHandle {
                worker: b_worker,
                msg: b_msg,
                res: b_res,
                out: b_out,
            },
        ) => {
            a_worker == b_worker
                && refines(a_msg, b_msg)
                && refines(a_res, b_res)
                && refines(a_out, b_out)
        }
        // Different shapes, or two leaves: only equality refines.
        _ => general == specific,
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
/// * A *user generic* is a [`UserOf`](ParameterType::UserOf): same template name,
///   same arity, then each type argument recursively — the same shape as the
///   built-in container arms (plan-105-B replaced the string fallback that used to
///   serve this case).
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
            match substitutions.get(&sym) {
                // First occurrence: record it, even an `Unknown` (a no-information
                // actual such as an empty `[]` literal, which types as `List OF
                // Unknown`). Keeping it as a *provisional* binding preserves the
                // degenerate `T := Unknown` instantiation that width-agnostic native
                // ops (e.g. `collections::flatten`) rely on when no argument ever
                // supplies a concrete element type.
                None => {
                    substitutions.insert(sym, actual.clone());
                }
                // A provisional binding is refined by any later, more concrete actual,
                // so inference no longer depends on field/argument order (bug-442): a
                // field carrying the concrete type (e.g. `FUNC(T) AS Boolean`) wins
                // over an earlier empty-collection field (`List OF T` := `[]`),
                // regardless of which is declared first.
                //
                // "More concrete" is [`refines`] — `Unknown` replaced by a real type
                // ANYWHERE in the shape, not only at the whole-binding root. The root-
                // only rule this replaced could not see an `Unknown` nested inside a
                // container, so `pick([], rows)` against
                // `pick OF T(fallback AS T, xs AS List OF T)` bound `T := List OF
                // Unknown` from the empty literal and then REJECTED the concrete
                // `List OF Integer` that followed, reporting
                // `TYPE_CALL_ARGUMENT_MISMATCH: cannot infer template arguments`
                // on a call whose second argument determines `T` completely.
                Some(existing) if refines(existing, actual) => {
                    substitutions.insert(sym, actual.clone());
                }
                // The existing binding is already at least as concrete (the actual is
                // `Unknown`, or an `Unknown`-bearing shape of the same form). It
                // agrees — mirroring the `actual is Unknown` terminal rule below — so
                // it neither overwrites nor conflicts.
                Some(existing) if refines(actual, existing) => {}
                // Two concrete actuals for the same param must agree.
                Some(existing) => return existing == actual,
            }
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

    // A user generic (`Name OF a, b`) unifies structurally on its own variant
    // (plan-105-B). This replaces the string fallback that rendered both sides with
    // `.name()`, re-split them with the private `user_template_parts`, and re-parsed
    // each argument — the last copy of the type grammar inside monomorph. Only fires
    // when BOTH sides are user generics, exactly as that arm did; otherwise fall
    // through to the container/FUNC arms below.
    if let (
        ParameterType::UserOf(pattern_base, pattern_args),
        ParameterType::UserOf(actual_base, actual_args),
    ) = (pattern, actual)
    {
        return pattern_base == actual_base
            && pattern_args.len() == actual_args.len()
            && pattern_args
                .iter()
                .zip(actual_args.iter())
                .all(|(p, a)| unify_type(p, a, params, substitutions));
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
/// * A *user generic* ([`UserOf`](ParameterType::UserOf)) substitutes each type
///   argument; its template head is a nominal, never a param.
/// * Everything else is identity: a scalar, a concrete nominal, and — matching the
///   old algorithm, which had no `FUNC`/`RES` arm — a `FUNC(...)` or `RES` type.
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
        // A user generic substitutes each type ARGUMENT; the template head is a
        // nominal and never a param. Structural (plan-105-B) where this used to
        // render `.name()`, re-split the string, substitute, and re-`parse` the
        // reassembled spelling.
        ParameterType::UserOf(base, args) => ParameterType::UserOf(
            *base,
            args.iter()
                .map(|arg| substitute_type_params(arg, substitutions))
                .collect(),
        ),
        // Any other type (scalar, concrete nominal, `FUNC(...)`, `RES`) is identity —
        // the old string algorithm had no arm for these and did not descend into them.
        other => other.clone(),
    }
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
        // bug-480: the shared resolver, so a source-directory dependency's
        // overload set is collected from the `.mfp` this build compiled into
        // `build/packages/` exactly as an installed one is.
        let Some(package_file) =
            crate::manifest::package::resolved_package_file(project_dir, package)
        else {
            continue;
        };
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
pub(super) fn overload_key(name: &str, params: &[HirParam], return_type: Option<&str>) -> String {
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
/// Whether a candidate overload's declared parameter types match a call's
/// argument types exactly, position for position.
///
/// plan-106-A: compares [`ParameterType`]s structurally. An un-annotated
/// parameter ([`ParameterType::Unknown`]) matches nothing, exactly as the
/// previous `opt_type_name(..) == Some(actual)` form did by yielding `None`.
pub(super) fn params_match(function: &HirFunction, arg_types: &[ParameterType]) -> bool {
    function.params.len() == arg_types.len()
        && function
            .params
            .iter()
            .zip(arg_types.iter())
            .all(|(param, actual)| opt_type(&param.type_).as_ref() == Some(actual))
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

pub(super) fn constructor_arg_value(argument: &HirConstructorArg) -> &HirExpression {
    match argument {
        HirConstructorArg::Positional(value) => value,
        HirConstructorArg::Named { value, .. } => value,
    }
}

/// The two type-environment entries a concrete function contributes: its return
/// type, and its type as a first-class *value*.
///
/// plan-106-A: both are [`ParameterType`]s. The signature used to be
/// `format!("{ISOLATED }FUNC({params}) AS {returns}")`; it is now a
/// [`Func`](ParameterType::Func) built from the declared types, which renders to
/// exactly that spelling. An un-annotated slot becomes
/// [`Unknown`](ParameterType::Unknown) (rendering `"Unknown"`) and a `SUB`
/// returns [`Nothing`](ParameterType::Nothing), reproducing the two
/// `unwrap_or_else` defaults the string form applied.
///
/// plan-117 Phase 2: called on demand by `expression_type`, for the ONE function
/// a query names. It used to seed a whole-program snapshot rebuilt for every
/// lowered function, which made the seeding O(F^2) in the function count.
pub(super) fn function_signature_types(function: &HirFunction) -> (ParameterType, ParameterType) {
    let returns = match function.kind {
        crate::ast::FunctionKind::Func => {
            opt_type(&function.returns).unwrap_or(ParameterType::Unknown)
        }
        crate::ast::FunctionKind::Sub => ParameterType::Nothing,
    };
    let params = function
        .params
        .iter()
        .map(|param| opt_type(&param.type_).unwrap_or(ParameterType::Unknown))
        .collect::<Vec<_>>();
    let signature = ParameterType::Func(params, Box::new(returns.clone()), function.isolated);
    (returns, signature)
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
    fn unknown_actual_never_poisons_a_param_binding() {
        // bug-442: an `Unknown` actual (e.g. an empty `[]` literal types as
        // `List OF Unknown`) must not occupy a param slot and block a later
        // concrete binding, in EITHER field order.
        let params = vec!["T".to_string()];
        // Unknown first, concrete second.
        let mut s = HashMap::new();
        assert!(unify_str("List OF T", "List OF Unknown", &params, &mut s));
        assert!(unify_str(
            "FUNC(T) AS Boolean",
            "FUNC(Integer) AS Boolean",
            &params,
            &mut s
        ));
        assert_eq!(s.get("T"), Some(&"Integer".to_string()));
        // Concrete first, Unknown second — the concrete binding survives.
        let mut s2 = HashMap::new();
        assert!(unify_str(
            "FUNC(T) AS Boolean",
            "FUNC(Integer) AS Boolean",
            &params,
            &mut s2
        ));
        assert!(unify_str("List OF T", "List OF Unknown", &params, &mut s2));
        assert_eq!(s2.get("T"), Some(&"Integer".to_string()));
        // A genuine conflict between two concretes still fails.
        let mut s3 = HashMap::new();
        assert!(unify_str("T", "Integer", &params, &mut s3));
        assert!(!unify_str("T", "String", &params, &mut s3));
        // All-Unknown records a *provisional* `Unknown` binding (never dropped),
        // preserving the degenerate `T := Unknown` instantiation that width-agnostic
        // native ops such as `collections::flatten` rely on. A later concrete actual
        // still refines it (asserted above); with none, it stays Unknown.
        let mut s4 = HashMap::new();
        assert!(unify_str("List OF T", "List OF Unknown", &params, &mut s4));
        assert_eq!(s4.get("T"), Some(&"Unknown".to_string()));
        assert!(unify_str("T", "Integer", &params, &mut s4));
        assert_eq!(s4.get("T"), Some(&"Integer".to_string()));
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

    /// plan-105-B retired monomorph's private `func_type_parts`; the canonical
    /// grammar decomposes a function type into the same pieces (and, unlike the
    /// private copy, preserves the `ISOLATED` marker instead of erasing it).
    #[test]
    fn func_type_parts_handles_isolated_and_empty_params() {
        assert_eq!(
            func_parts("FUNC(Integer, String) AS Boolean"),
            Some((
                vec!["Integer".to_string(), "String".to_string()],
                "Boolean".to_string(),
                false
            ))
        );
        assert_eq!(
            func_parts("ISOLATED FUNC() AS Nothing"),
            Some((Vec::new(), "Nothing".to_string(), true))
        );
        assert_eq!(func_parts("Integer"), None);
        assert_eq!(func_parts("FUNC(Integer)"), None);
    }

    /// The parameter/return decomposition of a `FUNC(...) AS R` spelling, read off
    /// the canonical [`ParameterType::Func`].
    fn func_parts(type_name: &str) -> Option<(Vec<String>, String, bool)> {
        match PARSE(type_name) {
            crate::types::ParameterType::Func(params, ret, isolated) => Some((
                params.iter().map(|p| p.name().into_owned()).collect(),
                ret.name().into_owned(),
                isolated,
            )),
            _ => None,
        }
    }

    /// The template head and rendered type arguments of a user generic, read off the
    /// canonical [`ParameterType::UserOf`] — the successor of the deleted
    /// `user_template_parts`.
    fn user_parts(type_name: &str) -> Option<(String, Vec<String>)> {
        match PARSE(type_name) {
            crate::types::ParameterType::UserOf(name, args) => Some((
                name.resolve().to_string(),
                args.iter().map(|a| a.name().into_owned()).collect(),
            )),
            _ => None,
        }
    }

    #[allow(non_snake_case)]
    fn PARSE(type_name: &str) -> crate::types::ParameterType {
        crate::types::ParameterType::parse(type_name)
    }

    /// bug-35: a type argument that is itself a comma-bearing function type must
    /// survive the split, or unification and mangling operate on garbage.
    #[test]
    fn nested_function_type_arguments_are_not_shredded() {
        assert_eq!(
            func_parts("FUNC(FUNC(Integer, String) AS Boolean, Integer) AS Nothing"),
            Some((
                vec![
                    "FUNC(Integer, String) AS Boolean".to_string(),
                    "Integer".to_string()
                ],
                "Nothing".to_string(),
                false
            ))
        );
        assert_eq!(
            func_parts("ISOLATED FUNC(FUNC(A, B) AS C) AS D"),
            Some((vec!["FUNC(A, B) AS C".to_string()], "D".to_string(), true))
        );
        // A two-argument template whose first argument is a two-parameter FUNC.
        assert_eq!(
            user_parts("Pair OF FUNC(Integer, String) AS Boolean, Integer"),
            Some((
                "Pair".to_string(),
                vec![
                    "FUNC(Integer, String) AS Boolean".to_string(),
                    "Integer".to_string()
                ]
            ))
        );
        // The depth-aware top-level comma split now lives in the canonical grammar.
        assert_eq!(
            crate::codegen::builtins::split_top_level_commas("Pair OF Integer, String"),
            vec!["Pair OF Integer", "String"]
        );
        assert_eq!(
            crate::codegen::builtins::split_top_level_commas("FUNC(A, B) AS C, D"),
            vec!["FUNC(A, B) AS C", "D"]
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

    /// A built-in constructor spelled with ` OF ` must never be read as a user
    /// generic named `List`/`Map`/… — the ordering rule `ParameterType::parse`'s
    /// `UserOf` arm inherited from the deleted `user_template_parts`.
    #[test]
    fn user_template_parts_excludes_builtin_shapes() {
        assert_eq!(
            user_parts("Pair OF Integer, String"),
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
            "Thread OF Integer TO String",
            "ThreadWorker OF Integer TO String",
            "FUNC(Integer) AS String",
            "ISOLATED FUNC() AS Nothing",
        ] {
            assert_eq!(user_parts(builtin), None, "{builtin}");
        }
        assert_eq!(user_parts("Integer"), None);
    }

    #[test]
    fn substitute_type_params_rewrites_every_shape() {
        let s = subs(&[("T", "Integer"), ("U", "String")]);
        assert_eq!(substitute_str("T", &s), "Integer");
        assert_eq!(substitute_str("List OF T", &s), "List OF Integer");
        assert_eq!(substitute_str("Set OF T", &s), "Set OF Integer");
        assert_eq!(substitute_str("Result OF T", &s), "Result OF Integer");
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
        assert!(params_match(&a, &[ParameterType::Integer]));
        assert!(!params_match(&a, &[ParameterType::String]));
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
            arg_slot_expected(&call, Some(&params), |p| p.first()),
            Some(ParameterType::Integer)
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
            constructor_arg_field_type(&pos, 1, Some(&fields)),
            Some(ParameterType::String)
        );
        let named = HirConstructorArg::Named {
            name: "x".to_string(),
            value: HirExpression::Number("1".to_string()),
            line: 1,
        };
        assert_eq!(
            constructor_arg_field_type(&named, 0, Some(&fields)),
            Some(ParameterType::Integer)
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
        // plan-106-A deleted monomorph's `numeric_binary_result_type` wrapper;
        // the engine calls the one typed source directly.
        assert_eq!(
            crate::numeric::typed_binary_result_type(
                BinaryOp::Add,
                &ParameterType::Integer,
                &ParameterType::Integer
            ),
            Some(ParameterType::Integer)
        );
        assert_eq!(
            crate::numeric::typed_binary_result_type(
                BinaryOp::Add,
                &ParameterType::Integer,
                &ParameterType::Float
            ),
            Some(ParameterType::Float)
        );
        // A Float bound anywhere in a FOR loop promotes the counter type.
        assert_eq!(
            crate::numeric::typed_promote_loop_numeric_type(
                &ParameterType::Integer,
                &ParameterType::Float,
                &ParameterType::Integer
            ),
            ParameterType::Float
        );
        assert_eq!(
            crate::numeric::typed_promote_loop_numeric_type(
                &ParameterType::Integer,
                &ParameterType::Integer,
                &ParameterType::Integer
            ),
            ParameterType::Integer
        );
    }

    /// plan-105-B moved both splitters into the canonical grammar
    /// (`crate::types::split_top_level_to`, `builtins::split_top_level_commas`);
    /// these assertions follow them there.
    #[test]
    fn split_helpers() {
        assert_eq!(
            crate::types::split_top_level_to("Integer TO String"),
            Some(("Integer", "String"))
        );
        assert_eq!(crate::types::split_top_level_to("Integer"), None);
        assert_eq!(
            crate::codegen::builtins::split_top_level_commas("Integer, String"),
            vec!["Integer", "String"]
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
        let (overloads, qualifiers) =
            collect_imported_overloads(&dir, &crate::hir::elaborate(&project));
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
