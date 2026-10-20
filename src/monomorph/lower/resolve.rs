//! plan-176-M: the monomorphizer's call resolution — imported overload sets, overload and
//! function-reference resolution, builtin overrides and type compatibility. The driver
//! (`new`, `run`, `into_project`) lives in `monomorph/lower/mod.rs`.

use super::*;

impl<'a> Monomorphizer<'a> {
    /// Rewrites a `collections::` call callee (`collections.sort`, or an aliased
    /// `c.sort`) to its internal generic implementation (`__collections_sort`).
    /// Returns the callee unchanged when it is not a `collections::` call.
    ///
    /// The rewrite symbol comes from the member's registered `Body::Mfb` descriptor
    /// (`registry::rewrite_target`), so only the source-generic members rewrite here
    /// — a native member (`get`, `transform`, …) has a non-rewrite body and stays a
    /// `collections.` call for the IR-lower native path. The rewrite happens at
    /// monomorph (not IR lowering, where the other packages' `Body::Mfb` members
    /// rewrite) because these bodies are GENERIC — the rewritten `#collections_sort`
    /// must flow into `instantiate_function` to be type-mangled and instantiated.
    ///
    /// Routed by ARGUMENT COUNT (bug-527). A member whose optional trailing parameter
    /// selects a different body — `findLastIndex`, whose two-argument form scans from
    /// the last element and whose three-argument form scans from the index it is given
    /// — has one rewrite target per form, and only the count tells them apart: both
    /// forms take the same types in the same order. `rewrite_target`'s type-based
    /// selection is answered here with an empty argument list (a `collections::` call
    /// is generic, so its argument types are pre-substitution and would mis-select),
    /// so the count is the only signal available and `rewrite_target_for_arity` falls
    /// back to the historical first-implementation answer for every other member.
    pub(super) fn collections_internal_callee(&self, callee: &str, argc: usize) -> Option<String> {
        let (binding, member) = callee.split_once('.')?;
        if !self.collections_bindings.contains(binding) {
            return None;
        }
        crate::codegen::registry::rewrite_target_for_arity(&format!("collections.{member}"), argc)
            .map(crate::internal_name::internalize)
    }

    /// Rewrite a call to an imported overloaded function to the package's mangled
    /// name, selecting the overload whose declared parameter types match the
    /// argument types (after stripping package qualifiers). Returns `None` for a
    /// non-imported call, a non-overloaded import, or an unresolved match.
    ///
    /// The match must be *unique*. `Unknown` (from an untyped `[]` literal) is a
    /// wildcard, so `f([])` matches both `f(List OF Integer)` and
    /// `f(List OF String)`; taking the first would silently bind the call to
    /// whichever overload the package happened to export first. That is ambiguous,
    /// exactly as it is for a local overload set.
    /// The imported overload set a `binding.member` callee names, with that
    /// binding: the binding is resolved through the file being lowered, since
    /// import bindings are per file (bug-914) — one alias may name different
    /// packages in different files.
    fn imported_overload_set(&self, callee: &str) -> Option<(String, &Vec<ImportedOverload>)> {
        let (binding, member) = callee.split_once('.')?;
        let package = self
            .file_imports
            .get(self.current_file.as_ref()?)?
            .get(binding)?;
        self.imported_overloads
            .get(&format!("{package}.{member}"))
            .map(|set| (binding.to_string(), set))
    }

    pub(super) fn resolve_imported_overload(
        &mut self,
        callee: &str,
        arg_types: &[ParameterType],
        line: usize,
    ) -> Option<String> {
        let (binding, candidates) = self.imported_overload_set(callee)?;
        let matches: Vec<String> =
            candidates
                .iter()
                .filter(|candidate| {
                    candidate.param_types.len() == arg_types.len()
                        && candidate.param_types.iter().zip(arg_types.iter()).all(
                            |(param, actual)| {
                                // plan-111-B: both sides are types now. The decoded
                                // package signature (`ImportedOverload::param_types`)
                                // arrives typed from `binary_repr::builder`
                                // (boundary #4), and both algorithms below turned out
                                // to be expressible structurally after all — see their
                                // doc comments and the equivalence tests that pin them
                                // against the token forms they replace.
                                self.types_compatible(
                                    &self.normalize_type(param),
                                    &self.normalize_type(actual),
                                )
                            },
                        )
                })
                .map(|candidate| format!("{binding}.{}", candidate.export_name))
                .collect();
        match matches.len() {
            0 => None,
            1 => Some(matches.into_iter().next().expect("one match")),
            count => {
                self.report(
                    "TYPE_OVERLOAD_AMBIGUOUS",
                    &format!(
                        "Call to `{callee}` matches {count} imported overloads; annotate the \
                         argument types (an untyped `[]` selects none of them) to choose one."
                    ),
                    line,
                );
                None
            }
        }
    }

    /// Whether a declared parameter type and an actual argument type match,
    /// treating `Unknown` (e.g. from an empty `[]` literal) as a wildcard so an
    /// untyped empty collection still selects an overload.
    ///
    /// plan-111-B: this was a token algorithm over the two RENDERED spellings —
    /// equal token counts, then each pair equal or either literally `"Unknown"`.
    /// The structural form reproduces it exactly, and the one subtlety is worth
    /// stating because it looks like a bug: **`Unknown` is a LEAF wildcard, not
    /// a universal one.** It stood in for a single whitespace token, so
    /// `Unknown` never matched `List OF Integer` — the token counts differed (1
    /// vs 3) — and it must not start matching it now. That is what the
    /// "an untyped `[]` selects none of them" half of `TYPE_OVERLOAD_AMBIGUOUS`
    /// depends on: a wholly-unknown argument selects NO overload rather than
    /// every one of them. `types_compatible_matches_the_token_algorithm` pins
    /// the equivalence over a corpus that includes exactly that case.
    pub(super) fn types_compatible(&self, param: &ParameterType, actual: &ParameterType) -> bool {
        if param == actual {
            return true;
        }
        // A single-token type: what one `Unknown` token could stand in for.
        fn is_leaf(type_: &ParameterType) -> bool {
            match type_ {
                ParameterType::ListOf(_)
                | ParameterType::SetOf(_)
                | ParameterType::MapOf(_, _)
                | ParameterType::MapEntryOf(_, _)
                | ParameterType::ResultOf(_)
                | ParameterType::Res(_)
                | ParameterType::Stateful { .. }
                | ParameterType::UserOf(_, _)
                | ParameterType::Func(_, _, _)
                | ParameterType::ThreadHandle { .. } => false,
                // A nominal is one token only if its spelling holds no space; a
                // composite that `parse` declined to decompose is not.
                ParameterType::Named(sym) => !sym.resolve().contains(' '),
                _ => true,
            }
        }
        match (param, actual) {
            (ParameterType::Unknown, other) | (other, ParameterType::Unknown) => is_leaf(other),
            (ParameterType::ListOf(p), ParameterType::ListOf(a))
            | (ParameterType::SetOf(p), ParameterType::SetOf(a))
            | (ParameterType::ResultOf(p), ParameterType::ResultOf(a))
            | (ParameterType::Res(p), ParameterType::Res(a)) => self.types_compatible(p, a),
            (ParameterType::MapOf(pk, pv), ParameterType::MapOf(ak, av))
            | (ParameterType::MapEntryOf(pk, pv), ParameterType::MapEntryOf(ak, av)) => {
                self.types_compatible(pk, ak) && self.types_compatible(pv, av)
            }
            (
                ParameterType::Stateful {
                    base: pb,
                    state: ps,
                },
                ParameterType::Stateful {
                    base: ab,
                    state: as_,
                },
            ) => self.types_compatible(pb, ab) && self.types_compatible(ps, as_),
            (ParameterType::UserOf(ph, pa), ParameterType::UserOf(ah, aa)) => {
                ph == ah
                    && pa.len() == aa.len()
                    && pa
                        .iter()
                        .zip(aa.iter())
                        .all(|(p, a)| self.types_compatible(p, a))
            }
            (ParameterType::Func(pp, pr, pi), ParameterType::Func(ap, ar, ai)) => {
                pi == ai
                    && pp.len() == ap.len()
                    && pp
                        .iter()
                        .zip(ap.iter())
                        .all(|(p, a)| self.types_compatible(p, a))
                    && self.types_compatible(pr, ar)
            }
            (
                ParameterType::ThreadHandle {
                    worker: pw,
                    msg: pm,
                    res: pres,
                    out: po,
                },
                ParameterType::ThreadHandle {
                    worker: aw,
                    msg: am,
                    res: ares,
                    out: ao,
                },
            ) => {
                pw == aw
                    && self.types_compatible(pm, am)
                    && self.types_compatible(pres, ares)
                    && self.types_compatible(po, ao)
            }
            _ => false,
        }
    }

    /// Strip package/import-binding qualifiers from each user/resource type name
    /// inside `type_` so an importer's `sqlite.Db` matches the package's bare `Db`.
    pub(super) fn normalize_type(&self, type_: &ParameterType) -> ParameterType {
        // Strip each qualifier only where it prefixes a type-name token — at the
        // start of the string or after a non-identifier byte — never as a bare
        // substring. An unanchored `replace` lets a short qualifier (`io.`) eat
        // into a longer name (`radio.`), and iterating `package_qualifiers` (a
        // `HashSet`-derived Vec) in hash order made the result depend on hash
        // seed, so the same source produced different overload resolutions and
        // flapping diagnostics run-to-run (bug-104). Sort longest-first for a
        // stable, prefix-preferring order.
        //
        // plan-111-B: applied per NOMINAL rather than over the whole rendered
        // spelling. Every qualifier is `"<binding>."` or `"<package>."`
        // (`helpers.rs:419`), and a `.` appears nowhere else in the type
        // grammar, so a qualifier can only ever match at the head of a nominal —
        // which is exactly what the byte-anchored scan found. Pinned by
        // `normalize_type_matches_the_string_algorithm`.
        let mut qualifiers: Vec<&str> =
            self.package_qualifiers.iter().map(String::as_str).collect();
        qualifiers.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        let strip = |name: &str| {
            let mut normalized = name.to_string();
            for qualifier in &qualifiers {
                normalized = strip_qualifier_prefixes(&normalized, qualifier);
            }
            normalized
        };
        fn walk(type_: &ParameterType, strip: &impl Fn(&str) -> String) -> ParameterType {
            match type_ {
                ParameterType::Named(sym) => ParameterType::named(&strip(sym.resolve())),
                ParameterType::UserOf(head, args) => ParameterType::user_of(
                    &strip(head.resolve()),
                    args.iter().map(|a| walk(a, strip)).collect(),
                ),
                ParameterType::ListOf(e) => ParameterType::list_of(walk(e, strip)),
                ParameterType::SetOf(e) => ParameterType::set_of(walk(e, strip)),
                ParameterType::ResultOf(e) => ParameterType::result_of(walk(e, strip)),
                ParameterType::Res(e) => ParameterType::res(walk(e, strip)),
                ParameterType::MapOf(k, v) => ParameterType::map_of(walk(k, strip), walk(v, strip)),
                ParameterType::MapEntryOf(k, v) => {
                    ParameterType::map_entry_of(walk(k, strip), walk(v, strip))
                }
                ParameterType::Stateful { base, state } => {
                    ParameterType::stateful(walk(base, strip), walk(state, strip))
                }
                ParameterType::Func(params, ret, isolated) => ParameterType::Func(
                    params.iter().map(|p| walk(p, strip)).collect(),
                    Box::new(walk(ret, strip)),
                    *isolated,
                ),
                ParameterType::ThreadHandle {
                    worker,
                    msg,
                    res,
                    out,
                } => ParameterType::ThreadHandle {
                    worker: *worker,
                    msg: Box::new(walk(msg, strip)),
                    res: Box::new(walk(res, strip)),
                    out: Box::new(walk(out, strip)),
                },
                other => other.clone(),
            }
        }
        // Drop a resource's `STATE T` suffix, exactly as the former source checker's `parse_type`
        // does and for the same reason (plan-52-D §4): an imported signature
        // spells a stateful resource parameter inline as `SoundFile STATE FileInfo`,
        // while the call site's argument type is the bare `SoundFile`. Without
        // this, `types_compatible` compares 3 tokens against 1 and NO overload
        // whose first parameter is a stateful resource can ever match — the call
        // silently resolved to `Type::Unknown` instead of reporting an error.
        walk(type_, &strip).without_state()
    }

    pub(super) fn resolve_overload(
        &mut self,
        name: &str,
        display: &str,
        arg_types: &[ParameterType],
        expected: Option<&ParameterType>,
        line: usize,
    ) -> Option<String> {
        // Built-in-named overrides are routed by `resolve_general_builtin_override`,
        // which enforces the gap-fill rule (the built-in wins for its own types).
        if crate::codegen::builtins::general::is_overridable(name) {
            return None;
        }
        let candidates = self.function_overloads.get(name)?;
        if candidates.len() <= 1 {
            return self.sole_file_sibling(name);
        }
        let param_matches = candidates
            .iter()
            .filter(|function| params_match(function, arg_types))
            .cloned()
            .collect::<Vec<_>>();
        let chosen = match param_matches.len() {
            0 => return None,
            1 => param_matches.into_iter().next()?,
            _ => {
                // A return-type overload set: every candidate shares these
                // parameter types and differs only by result type, so the call's
                // expected (contextual) type selects one (plan-01-overload.md
                // §F.2.3). With no expected type, or none uniquely matching, the
                // call is ambiguous.
                let mut by_return = param_matches
                    .iter()
                    .filter(|function| opt_type(&function.returns).as_ref() == expected);
                match (by_return.next(), by_return.next()) {
                    (Some(unique), None) => unique.clone(),
                    _ => {
                        self.report(
                            "TYPE_OVERLOAD_AMBIGUOUS",
                            &format!(
                                "Call to `{display}` matches {} overloads that differ only by \
                                 return type; supply the expected type (e.g. a `LET … AS` \
                                 annotation) to select one.",
                                param_matches.len()
                            ),
                            line,
                        );
                        return None;
                    }
                }
            }
        };
        // Keyed by the member's own name: a file-scoped set also holds the
        // file's PUBLIC/EXPORT siblings, which are declared under the bare name.
        self.overload_names
            .get(&overload_key(
                &chosen.name,
                &chosen.params,
                opt_type_name(&chosen.returns).as_deref(),
            ))
            .cloned()
    }

    /// The template `name` denotes: itself, or — for a file-scoped PRIVATE name
    /// with no template of its own — the same file's PUBLIC/EXPORT template of
    /// that name (bug-912), which is instantiated under its own name.
    pub(super) fn template_name<'n>(&'n self, name: &'n str) -> &'n str {
        self.template_views.get(name).map_or(name, String::as_str)
    }

    /// The concrete symbol of the one function a file-scoped PRIVATE name
    /// denotes when that function is the same file's PUBLIC/EXPORT declaration
    /// (bug-912) — the name's PRIVATE declaration being a template, which is
    /// tried first. `None` for any other name: a set of one that is declared
    /// under the name itself needs no rewrite.
    fn sole_file_sibling(&self, name: &str) -> Option<String> {
        let [only] = self.function_overloads.get(name)?.as_slice() else {
            return None;
        };
        if only.name == name {
            return None;
        }
        self.overload_names
            .get(&overload_key(
                &only.name,
                &only.params,
                opt_type_name(&only.returns).as_deref(),
            ))
            .cloned()
    }

    /// Select the member of an overloaded function that a bare-name VALUE
    /// reference denotes, returning its concrete symbol (bug-822).
    ///
    /// A call picks its overload from argument types; a reference has none, so
    /// it is selected by the complete callable type its context expects — the
    /// `FUNC` annotation of a `LET`/`MUT`/global, an assignment target, a
    /// callback parameter, the enclosing return type, or a typed field, element
    /// or map value. A member matches when its ordered parameter types and its
    /// return type equal the expected type's (exactly, as overload resolution is
    /// exact) and it is `ISOLATED` whenever the expected type is — an isolated
    /// member may stand where a plain `FUNC` is expected (bug-786), never the
    /// reverse. Parameter names, defaults and visibility are not part of the
    /// callable type. Exactly one member must match.
    ///
    /// The overload set is the one a direct call resolves against: the
    /// project's own declarations of `name`, or an imported package's `EXPORT`
    /// members of `binding.name`. Returns `None` — leaving the name unrevised —
    /// for anything else: a local that shadows the name, a non-overloaded or
    /// unknown name, or a built-in-named override (routed by its own rule). An
    /// ambiguous or unmatched reference is diagnosed HERE, while the overload set
    /// is still visible; once monomorphization has mangled every member, the
    /// bare name would only read as undeclared.
    pub(super) fn resolve_function_reference(
        &mut self,
        name: &str,
        expected: Option<&ParameterType>,
        substitutions: &HashMap<crate::intern::Symbol, crate::types::ParameterType>,
        context: &FunctionContext,
        line: usize,
    ) -> Option<String> {
        if context.bound.contains(name) || context.locals.contains_key(name) {
            return None;
        }
        // Each member's concrete symbol and callable type, and whether the set
        // is imported (its package types compare as an imported call's do).
        let (display, imported, members): (String, bool, Vec<(String, ParameterType)>) =
            if let Some(candidates) = self.function_overloads.get(name) {
                if crate::codegen::builtins::general::is_overridable(name) {
                    return self.resolve_builtin_override_reference(name, expected, substitutions);
                }
                if candidates.len() <= 1 {
                    return self.sole_file_sibling(name);
                }
                let members = candidates
                    .iter()
                    .map(|function| {
                        let symbol = self
                            .overload_names
                            .get(&overload_key(
                                &function.name,
                                &function.params,
                                opt_type_name(&function.returns).as_deref(),
                            ))
                            .cloned()
                            .unwrap_or_else(|| function.name.clone());
                        (symbol, reference_signature(function))
                    })
                    .collect::<Vec<_>>();
                (
                    crate::internal_name::display_name(name).into_owned(),
                    false,
                    members,
                )
            } else if let Some((binding, candidates)) = self.imported_overload_set(name) {
                let members = candidates
                    .iter()
                    .map(|candidate| {
                        (
                            format!("{binding}.{}", candidate.export_name),
                            ParameterType::Func(
                                candidate.param_types.clone(),
                                Box::new(candidate.return_type.clone()),
                                candidate.isolated,
                            ),
                        )
                    })
                    .collect();
                (name.replacen('.', "::", 1), true, members)
            } else {
                return None;
            };
        // Compare concrete types: inside a generic body the annotation still
        // names the template's parameters (`FUNC(T) AS T`), and a user generic is
        // spelled `Box OF Integer` in a declaration but `Box$Integer` once
        // instantiated. `concrete_type` settles both, exactly as it does for the
        // declarations themselves when they are lowered.
        let expected = expected.map(|expected| self.concrete_type(expected, substitutions));
        let members = if imported {
            members
        } else {
            members
                .into_iter()
                .map(|(symbol, signature)| {
                    (symbol, self.concrete_type(&signature, &HashMap::new()))
                })
                .collect()
        };
        let selected = |signature: &ParameterType| match &expected {
            Some(expected) if imported => {
                callable_matches_with(expected, signature, |want, have| {
                    self.types_compatible(&self.normalize_type(have), &self.normalize_type(want))
                })
            }
            Some(expected) => callable_matches(expected, signature),
            None => false,
        };
        let matching = members
            .iter()
            .filter(|(_, signature)| selected(signature))
            .map(|(symbol, _)| symbol.clone())
            .collect::<Vec<_>>();
        let signatures = members
            .iter()
            .map(|(_, signature)| format!("`{}`", signature.name()))
            .collect::<Vec<_>>()
            .join(", ");
        let Some(expected) = expected else {
            self.report(
                "TYPE_OVERLOAD_AMBIGUOUS",
                &format!(
                    "`{display}` names {} overloaded functions ({signatures}); a reference to \
                     it as a value needs an expected `FUNC` type to select one — annotate it \
                     (e.g. `LET f AS FUNC(…) AS … = {display}`).",
                    members.len()
                ),
                line,
            );
            return None;
        };
        match matching.len() {
            1 => matching.into_iter().next(),
            0 if matches!(expected, ParameterType::Func(..)) => {
                self.report(
                    "TYPE_OVERLOAD_NO_MATCH",
                    &format!(
                        "No overload of `{display}` has the callable type `{}`; its overloads \
                         are {signatures}.",
                        expected.name()
                    ),
                    line,
                );
                None
            }
            0 => {
                self.report(
                    "TYPE_OVERLOAD_NO_MATCH",
                    &format!(
                        "`{display}` names overloaded functions ({signatures}), which cannot be \
                         used where `{}` is expected.",
                        expected.name()
                    ),
                    line,
                );
                None
            }
            count => {
                self.report(
                    "TYPE_OVERLOAD_AMBIGUOUS",
                    &format!(
                        "{count} overloads of `{display}` have the callable type `{}`.",
                        expected.name()
                    ),
                    line,
                );
                None
            }
        }
    }

    /// The user override a value reference to an overridable general built-in
    /// name selects (bug-822). Such a declaration is always force-mangled, so the
    /// bare name never reaches it; it is selected exactly as a call to it is — by
    /// the gap-fill rule: only when the built-in rejects the expected callable
    /// type's parameter types, and only the one override whose complete callable
    /// type is the expected one. Anything else keeps the bare built-in name for IR
    /// lowering's own built-in reference handling, undiagnosed here.
    fn resolve_builtin_override_reference(
        &mut self,
        name: &str,
        expected: Option<&ParameterType>,
        substitutions: &HashMap<crate::intern::Symbol, crate::types::ParameterType>,
    ) -> Option<String> {
        let expected = self.concrete_type(expected?, substitutions);
        let ParameterType::Func(params, _, _) = &expected else {
            return None;
        };
        if crate::codegen::builtins::resolve_call_return_type_with_kinds(name, params, false, self)
            .is_some()
        {
            return None;
        }
        let candidates = self.function_overloads.get(name)?.clone();
        let mut matching = Vec::new();
        for function in &candidates {
            let signature = self.concrete_type(&reference_signature(function), &HashMap::new());
            if callable_matches(&expected, &signature) {
                matching.extend(
                    self.overload_names
                        .get(&overload_key(
                            &function.name,
                            &function.params,
                            opt_type_name(&function.returns).as_deref(),
                        ))
                        .cloned(),
                );
            }
        }
        (matching.len() == 1).then(|| matching.remove(0))
    }

    /// Route a call whose callee is an **overridable general built-in** to a user
    /// override (plan-01-overload.md §A.3 / Phase 5.2). The built-in is
    /// authoritative for the types it already supports, so an override is selected
    /// only when the built-in rejects the argument types — a non-matching call
    /// (scalar/collection args) is left as the bare built-in name for codegen to
    /// dispatch. Fires for a sole built-in-named overload too, unlike the ordinary
    /// `resolve_overload`.
    pub(super) fn resolve_general_builtin_override(
        &self,
        name: &str,
        arg_types: &[ParameterType],
    ) -> Option<String> {
        if !crate::codegen::builtins::general::is_overridable(name) {
            return None;
        }
        // `name` is already gated to a general-overridable builtin above, so the
        // registry aggregate resolves it exactly as `general::resolve_call` did
        // (plan-72-BB). plan-106-A: through the TYPED entry (plan-104-C's
        // `resolve_call_return_type_typed`), so no type is rendered here.
        // plan-140-A: with this monomorphizer as the enum oracle, so a built-in
        // that supports enums is authoritative for them here too, while a record
        // override keeps its gap-fill dispatch.
        if crate::codegen::builtins::resolve_call_return_type_with_kinds(
            name, arg_types, false, self,
        )
        .is_some()
        {
            return None;
        }
        let chosen = self
            .function_overloads
            .get(name)?
            .iter()
            .find(|function| params_match(function, arg_types))?;
        self.overload_names
            .get(&overload_key(
                name,
                &chosen.params,
                opt_type_name(&chosen.returns).as_deref(),
            ))
            .cloned()
    }

    /// The parameter list of `callee` when it names exactly one user function (no
    /// overloading). Supplies the expected (contextual) type for an argument slot
    /// so a return-type-overloaded call passed as an argument resolves
    /// (plan-01-overload.md §F.2); `None` when the callee is overloaded, a package
    /// member, or unknown.
    pub(super) fn single_signature_params(&self, callee: &str) -> Option<Vec<HirParam>> {
        let candidates = self.function_overloads.get(callee)?;
        (candidates.len() == 1).then(|| candidates[0].params.clone())
    }
}
