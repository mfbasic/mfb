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

/// Whether `value` is an operand whose lowering consults an expected
/// (contextual) type: a call, where a return-type overload set needs the context
/// to resolve (plan-01-overload.md §F.2), or a bare name, which may reference an
/// overloaded function as a value and select the member of that callable type
/// (bug-822). Other operands keep their own inferred typing.
pub(super) fn consults_expected(value: &HirExpression) -> bool {
    matches!(
        value,
        HirExpression::Call { .. } | HirExpression::Identifier(_)
    )
}

/// The expected (contextual) type for an argument slot: the selected parameter's
/// declared type, but only for an operand that [`consults_expected`]. Returns
/// `None` otherwise so literals keep their own inferred typing.
pub(super) fn arg_slot_expected<'a>(
    value: &HirExpression,
    params: Option<&'a [HirParam]>,
    select: impl FnOnce(&'a [HirParam]) -> Option<&'a HirParam>,
) -> Option<ParameterType> {
    if !consults_expected(value) {
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
/// bug-752: `type_` without a top-level `RES` ownership marker.
pub(super) fn strip_res(type_: &ParameterType) -> &ParameterType {
    match type_ {
        ParameterType::Res(inner) => inner,
        other => other,
    }
}

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
                // Two concrete actuals for the same param must agree. bug-752: the
                // `RES` ownership marker is not part of the type — a `List OF RES
                // File` binds `T := RES File` while a callback over its elements
                // declares `FUNC(File)` — so the two agree modulo the marker, and
                // the marked binding (the element's) is kept.
                Some(existing) => {
                    if strip_res(existing) != strip_res(actual) {
                        return false;
                    }
                    if matches!(actual, ParameterType::Res(_)) {
                        substitutions.insert(sym, actual.clone());
                    }
                }
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
        // A callable substitutes its parameter and return types, keeping its
        // isolation. Without this a generic body's `LET f AS FUNC(T) AS T = …`
        // kept the template parameter in its expected type, so no return-type
        // overload of the call could ever equal it (found with bug-822).
        ParameterType::Func(params, ret, isolated) => ParameterType::Func(
            params
                .iter()
                .map(|param| substitute_type_params(param, substitutions))
                .collect(),
            Box::new(substitute_type_params(ret, substitutions)),
            *isolated,
        ),
        // Any other type (scalar, concrete nominal, `RES`) is identity — the old
        // string algorithm had no arm for these and did not descend into them.
        other => other.clone(),
    }
}

/// The record layouts every imported package's `.mfp` exports, keyed by the bare
/// declared nominal, so the monomorphizer types `record.field` on an imported
/// record exactly as on a local one (bug-631). Resolves each import binding's
/// package through the same `resolved_package_file` as
/// [`collect_imported_overloads`]; a built-in package has no `.mfp` and
/// contributes nothing (its types are already in the augmented HIR).
pub(super) fn collect_imported_records(
    project_dir: &Path,
    source: &HirProject,
) -> HashMap<ParameterType, Vec<HirTypeField>> {
    let packages = source
        .files
        .iter()
        .flat_map(|file| file.import_bindings())
        .map(|(_, package)| package)
        .collect::<HashSet<_>>()
        .into_iter()
        .filter_map(|package| {
            crate::manifest::package::resolved_package_file(project_dir, &package)
        })
        .collect::<Vec<_>>();
    crate::manifest::package::imported_type_defs_from_files(&packages)
        .into_iter()
        .filter(|def| matches!(def.kind, crate::ir::ImportedTypeKind::Record))
        .map(|def| {
            let fields = def
                .fields
                .into_iter()
                .map(|field| HirTypeField {
                    visibility: None,
                    name: field.name,
                    type_: field.type_,
                    line: 0,
                })
                .collect();
            (ParameterType::declared(&def.name), fields)
        })
        .collect()
}

/// The ENUM types exported by imported packages, keyed by the bare declared
/// nominal exactly as [`collect_imported_records`] keys a record (plan-140-A).
pub(super) fn collect_imported_enums(
    project_dir: &Path,
    source: &HirProject,
) -> HashSet<ParameterType> {
    let packages = source
        .files
        .iter()
        .flat_map(|file| file.import_bindings())
        .map(|(_, package)| package)
        .collect::<HashSet<_>>()
        .into_iter()
        .filter_map(|package| {
            crate::manifest::package::resolved_package_file(project_dir, &package)
        })
        .collect::<Vec<_>>();
    crate::manifest::package::imported_type_defs_from_files(&packages)
        .into_iter()
        .filter(|def| matches!(def.kind, crate::ir::ImportedTypeKind::Enum))
        .map(|def| ParameterType::named(&def.name))
        .collect()
}

/// Read each imported package's exported functions and collect the overloaded
/// ones (more than one export sharing a base name), keyed by `package.base` —
/// the PACKAGE, not an import binding: bindings are per file, so one alias may
/// name different packages in different files, and a call is resolved through
/// the calling file's own binding (`Monomorphizer::imported_overload_set`,
/// bug-914). Also returns the set of `binding.`/`package.` qualifier prefixes for
/// argument-type normalization (plan-linker.md §12, overloads).
pub(super) fn collect_imported_overloads(
    project_dir: &Path,
    source: &HirProject,
) -> (HashMap<String, Vec<ImportedOverload>>, Vec<String>) {
    let mut overloads: HashMap<String, Vec<ImportedOverload>> = HashMap::new();
    let mut qualifiers: HashSet<String> = HashSet::new();
    // Every package any file imports, under whichever binding.
    let mut packages: HashSet<String> = HashSet::new();
    for file in &source.files {
        for (binding, package) in file.import_bindings() {
            // bug-632: only a BUILT-IN package's qualifier is stripped. An imported
            // user package's type is package-qualified on both sides of the match
            // — the argument by the parser (`ov.A`) and the candidate below — so
            // stripping it would turn `ov.A` and another package's `pb.A` back
            // into one bare `A`.
            if crate::codegen::builtins::is_builtin_import(&package) {
                qualifiers.insert(format!("{binding}."));
                qualifiers.insert(format!("{package}."));
            }
            packages.insert(package);
        }
    }
    for package in &packages {
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
        // bug-632: the package's own type names, so each candidate's parameter
        // types carry the same `<package>.<Name>` identity the argument does.
        let owners = crate::binary_repr::BinaryReprPackageDecode::read(&package_file)
            .map(|decode| crate::manifest::package::package_type_owners(&decode, package))
            .unwrap_or_default();
        // Group exported functions by base name (the part before `$`).
        let mut by_base: HashMap<String, Vec<crate::binary_repr::BinaryReprExport>> =
            HashMap::new();
        for export in exports {
            if !matches!(export.kind, crate::binary_repr::BinaryReprExportKind::Func) {
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
            let entry = overloads.entry(format!("{package}.{base}")).or_default();
            for export in exports {
                entry.push(ImportedOverload {
                    param_types: export
                        .params
                        .iter()
                        .map(|param| {
                            crate::manifest::package::qualify_package_type(&param.type_, &owners)
                        })
                        .collect(),
                    export_name: export.name.clone(),
                    return_type: crate::manifest::package::qualify_package_type(
                        &export.return_type,
                        &owners,
                    ),
                    isolated: export.isolated,
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
/// [`Unknown`](ParameterType::Unknown) (rendering `"Unknown"`) and a function with no `AS` clause
/// returns [`Nothing`](ParameterType::Nothing), reproducing the two
/// `unwrap_or_else` defaults the string form applied.
///
/// plan-117 Phase 2: called on demand by `expression_type`, for the ONE function
/// a query names. It used to seed a whole-program snapshot rebuilt for every
/// lowered function, which made the seeding O(F^2) in the function count.
pub(super) fn function_signature_types(function: &HirFunction) -> (ParameterType, ParameterType) {
    // bug-671: a `FUNC … AS RES T STATE S` return carries its payload, as the
    // IR's `function_return_type` does, so `f().state.x` types.
    let returns = match opt_type(&function.returns) {
        Some(returns) => match (&function.return_state_type, function.return_resource) {
            (Some(state), true) => returns.with_state(state),
            _ => returns,
        },
        None => ParameterType::Unknown,
    };
    let params = function
        .params
        .iter()
        .map(|param| opt_type(&param.type_).unwrap_or(ParameterType::Unknown))
        .collect::<Vec<_>>();
    let signature = ParameterType::Func(params, Box::new(returns.clone()), function.isolated);
    (returns, signature)
}

/// The callable type a reference to `function` as a value has: its declared
/// parameter types, its return type — `Nothing` when it declares none — and its
/// isolation. Parameter names and defaults are not part of it.
pub(super) fn reference_signature(function: &HirFunction) -> ParameterType {
    match function_signature_types(function).1 {
        ParameterType::Func(params, returns, isolated) if *returns == ParameterType::Unknown => {
            ParameterType::Func(params, Box::new(ParameterType::Nothing), isolated)
        }
        signature => signature,
    }
}

/// Whether a member of callable type `candidate` may be selected where the
/// callable type `expected` is required: the same number of parameters, each
/// equal to the expected one up to a top-level `RES` marker (bug-752), an equal
/// return type, and isolation whenever `expected` demands it (an `ISOLATED`
/// member may widen to a plain `FUNC`, bug-786). A non-callable `expected`
/// selects nothing.
pub(super) fn callable_matches(expected: &ParameterType, candidate: &ParameterType) -> bool {
    callable_matches_with(expected, candidate, |want, have| {
        strip_res(want) == strip_res(have)
    })
}

/// [`callable_matches`] with the per-type comparison supplied by the caller
/// (imported members compare package types the way an imported call does).
pub(super) fn callable_matches_with(
    expected: &ParameterType,
    candidate: &ParameterType,
    same: impl Fn(&ParameterType, &ParameterType) -> bool,
) -> bool {
    let (
        ParameterType::Func(want_params, want_return, want_isolated),
        ParameterType::Func(have_params, have_return, have_isolated),
    ) = (expected, candidate)
    else {
        return false;
    };
    want_params.len() == have_params.len()
        && (!want_isolated || *have_isolated)
        && same(want_return, have_return)
        && want_params
            .iter()
            .zip(have_params)
            .all(|(want, have)| same(want, have))
}

#[cfg(test)]
mod tests;
