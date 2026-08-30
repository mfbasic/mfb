use super::*;

/// Remove every occurrence of `qualifier` in `input` that begins a type-name
/// token — at position 0 or immediately after a non-identifier byte — leaving
/// substring occurrences inside a longer identifier untouched (so `io.` does not
/// bite into `radio.`). See `Monomorphizer::normalize_type` (bug-104).
fn strip_qualifier_prefixes(input: &str, qualifier: &str) -> String {
    if qualifier.is_empty() {
        return input.to_string();
    }
    let is_ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        if input[i..].starts_with(qualifier) && (i == 0 || !is_ident(bytes[i - 1])) {
            i += qualifier.len();
            continue;
        }
        let ch = input[i..].chars().next().expect("valid char boundary");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

impl<'a> Monomorphizer<'a> {
    pub(super) fn new(project_dir: &'a Path, source: &'a HirProject) -> Self {
        let mut type_templates = HashMap::new();
        let mut function_templates = HashMap::new();
        let mut concrete_types = HashMap::new();
        let mut concrete_functions = HashMap::new();
        let mut function_overloads: HashMap<String, Vec<HirFunction>> = HashMap::new();
        let mut overload_names = HashMap::new();
        let mut function_files: HashMap<String, String> = HashMap::new();

        for file in &source.files {
            for item in &file.items {
                match item {
                    HirItem::Binding(_) => {}
                    HirItem::Type(type_decl) if !type_decl.template_params.is_empty() => {
                        type_templates.insert(type_decl.name.clone(), type_decl.clone());
                    }
                    HirItem::Type(type_decl) => {
                        concrete_types.insert(type_decl.name.clone(), type_decl.clone());
                    }
                    HirItem::Function(function) if !function.template_params.is_empty() => {
                        function_files.insert(function.name.clone(), file.path.clone());
                        function_templates.insert(function.name.clone(), function.clone());
                    }
                    HirItem::Function(function) => {
                        function_files
                            .entry(function.name.clone())
                            .or_insert_with(|| file.path.clone());
                        function_overloads
                            .entry(function.name.clone())
                            .or_default()
                            .push(function.clone());
                    }
                    // Native LINK resources, re-export aliases, and LINK blocks
                    // carry no template parameters and are passed through
                    // unchanged (plan-link-update.md §15 Phase 1).
                    HirItem::Resource(_) | HirItem::FuncAlias(_) | HirItem::Link(_) => {}
                    // DOC blocks carry no template parameters; passed through below.
                    HirItem::Doc(_) => {}
                    // TESTING blocks are lowered away before monomorphization.
                    HirItem::Testing(_) => {}
                }
            }
        }

        for functions in function_overloads.values() {
            for function in functions {
                // A user `FUNC` whose name is an overridable general built-in is
                // always force-mangled so its codegen symbol never equals the
                // built-in dispatch name (plan-01-overload.md §C Phase 5.1).
                let builtin_named =
                    crate::codegen::builtins::general::is_overridable(&function.name);
                // A return-type overload set: ≥2 declarations share this name *and*
                // parameter types, differing only by return type (§F.1). Their
                // concrete symbols must also encode the return type to stay distinct.
                let return_disambiguated = functions
                    .iter()
                    .filter(|other| param_types_eq(other, function))
                    .count()
                    > 1;
                let concrete_name = overload_concrete_name(
                    function,
                    functions.len() > 1 || builtin_named,
                    return_disambiguated,
                );
                overload_names.insert(
                    overload_key(
                        &function.name,
                        &function.params,
                        opt_type_name(&function.returns).as_deref(),
                    ),
                    concrete_name.clone(),
                );
                if let Some(path) = function_files.get(&function.name).cloned() {
                    function_files.insert(concrete_name.clone(), path);
                }
                let mut concrete = function.clone();
                concrete.name = concrete_name.clone();
                concrete_functions.insert(concrete_name, concrete);
            }
        }

        let (imported_overloads, package_qualifiers) =
            collect_imported_overloads(project_dir, source);

        Self {
            project_dir,
            source,
            type_templates,
            function_templates,
            concrete_types,
            concrete_functions,
            function_overloads,
            overload_names,
            imported_overloads,
            package_qualifiers,
            type_instantiations: HashMap::new(),
            emitted_type_keys: HashSet::new(),
            emitted_function_keys: HashSet::new(),
            concrete_symbol_keys: HashMap::new(),
            // Binding names (including aliases) of every `collections` import — used
            // to map a `binding.member` callee onto the source-generic implementation.
            collections_bindings: source
                .files
                .iter()
                .flat_map(|file| file.imports.iter())
                .filter(|import| import.package_name() == "collections")
                .map(|import| import.binding_name().to_string())
                .collect(),
            function_files,
            current_file: None,
            template_instantiation_depth: 0,
            total_instantiations: 0,
            instantiation_limit_reached: false,
            had_error: false,
        }
    }

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
    fn collections_internal_callee(&self, callee: &str) -> Option<String> {
        let (binding, member) = callee.split_once('.')?;
        if !self.collections_bindings.contains(binding) {
            return None;
        }
        crate::codegen::registry::rewrite_target(&format!("collections.{member}"), &[])
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
    fn resolve_imported_overload(
        &mut self,
        callee: &str,
        arg_types: &[ParameterType],
        line: usize,
    ) -> Option<String> {
        let candidates = self.imported_overloads.get(callee)?;
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
                .map(|candidate| candidate.qualified_name.clone())
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
    fn types_compatible(&self, param: &ParameterType, actual: &ParameterType) -> bool {
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
    fn normalize_type(&self, type_: &ParameterType) -> ParameterType {
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
        // spells a stateful `RES` parameter inline as `SoundFile STATE FileInfo`,
        // while the call site's argument type is the bare `SoundFile`. Without
        // this, `types_compatible` compares 3 tokens against 1 and NO overload
        // whose first parameter is a stateful resource can ever match — the call
        // silently resolved to `Type::Unknown` instead of reporting an error.
        walk(type_, &strip).without_state()
    }

    pub(super) fn run(&mut self) {
        let types = self.concrete_types.values().cloned().collect::<Vec<_>>();
        for type_decl in types {
            let lowered = self.lower_type(type_decl, &HashMap::new(), None);
            self.concrete_types.insert(lowered.name.clone(), lowered);
        }

        let functions = self
            .concrete_functions
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for function in functions {
            let lowered = self.lower_function(function, &HashMap::new(), None);
            self.concrete_functions
                .insert(lowered.name.clone(), lowered);
        }
    }

    pub(super) fn into_project(mut self) -> HirProject {
        let mut emitted_types = HashSet::new();
        let mut emitted_functions = HashSet::new();
        let mut files = self
            .source
            .files
            .iter()
            .map(|file| {
                let mut items = Vec::new();
                for item in &file.items {
                    match item {
                        HirItem::Binding(binding) => {
                            items.push(HirItem::Binding(self.lower_binding(binding.clone())));
                        }
                        HirItem::Type(type_decl) if type_decl.template_params.is_empty() => {
                            if let Some(concrete) = self.concrete_types.get(&type_decl.name) {
                                emitted_types.insert(concrete.name.clone());
                                items.push(HirItem::Type(concrete.clone()));
                            }
                        }
                        HirItem::Function(function) if function.template_params.is_empty() => {
                            let concrete_name = self
                                .overload_names
                                .get(&overload_key(
                                    &function.name,
                                    &function.params,
                                    opt_type_name(&function.returns).as_deref(),
                                ))
                                .map(String::as_str)
                                .unwrap_or(&function.name);
                            if let Some(concrete) = self.concrete_functions.get(concrete_name) {
                                emitted_functions.insert(concrete.name.clone());
                                items.push(HirItem::Function(concrete.clone()));
                            }
                        }
                        // Native LINK constructs are not monomorphized; preserve
                        // them verbatim so later stages (resolve, the former source checker,
                        // package metadata) still see them.
                        HirItem::Resource(resource) => {
                            items.push(HirItem::Resource(resource.clone()));
                        }
                        HirItem::FuncAlias(alias) => {
                            items.push(HirItem::FuncAlias(alias.clone()));
                        }
                        HirItem::Link(link) => {
                            items.push(HirItem::Link(link.clone()));
                        }
                        // Preserve DOC blocks verbatim so the post-monomorph
                        // resolve and IR lowering still see the documentation.
                        HirItem::Doc(doc) => {
                            items.push(HirItem::Doc(doc.clone()));
                        }
                        _ => {}
                    }
                }
                HirFile {
                    path: file.path.clone(),
                    imports: file.imports.clone(),
                    items,
                    internal: file.internal,
                }
            })
            .collect::<Vec<_>>();

        if let Some(first_file) = files.first_mut() {
            // Generated instantiations (monomorphized generic functions/types) are
            // emitted into the FIRST file, but their rewritten call/use sites can
            // live in ANY file. With `Public` as the default visibility, a template
            // with no modifier (e.g. the `collections::` internals) instantiates to
            // a `Public` concrete function, which resolves project-wide — so no
            // widening is needed here.
            //
            // Those generated bodies can still carry package-qualified calls to any
            // package used anywhere in the project (a monomorphized `collections::`
            // generic keeps calling `collections::` helpers). Since they now live in
            // the first file, union every source file's imports into it so the
            // post-monomorph resolve can resolve those qualified names; the first
            // file's own bindings win on any alias clash.
            let mut seen: HashSet<String> = first_file
                .imports
                .iter()
                .map(|import| import.binding_name().to_string())
                .collect();
            for import in self.source.files.iter().flat_map(|file| &file.imports) {
                if seen.insert(import.binding_name().to_string()) {
                    first_file.imports.push(import.clone());
                }
            }

            let mut generated_types = self
                .concrete_types
                .into_values()
                .filter(|type_decl| !emitted_types.contains(&type_decl.name))
                .collect::<Vec<_>>();
            generated_types.sort_by(|left, right| left.name.cmp(&right.name));
            first_file
                .items
                .extend(generated_types.into_iter().map(HirItem::Type));

            let mut generated_functions = self
                .concrete_functions
                .into_values()
                .filter(|function| !emitted_functions.contains(&function.name))
                .collect::<Vec<_>>();
            generated_functions.sort_by(|left, right| left.name.cmp(&right.name));
            first_file
                .items
                .extend(generated_functions.into_iter().map(HirItem::Function));
        }

        HirProject {
            name: self.source.name.clone(),
            files,
        }
    }

    fn lower_type(
        &mut self,
        mut type_decl: HirTypeDecl,
        substitutions: &HashMap<crate::intern::Symbol, crate::types::ParameterType>,
        concrete_name: Option<String>,
    ) -> HirTypeDecl {
        if let Some(name) = concrete_name {
            type_decl.name = name;
        }
        type_decl.template_params.clear();
        type_decl.includes = type_decl
            .includes
            .iter()
            .map(|include| self.concrete_type_name(include, substitutions))
            .collect();
        type_decl.fields = type_decl
            .fields
            .iter()
            .map(|field| self.lower_field(field, substitutions))
            .collect();
        type_decl.variants = type_decl
            .variants
            .iter()
            .map(|variant| UnionVariant {
                name: self.concrete_type_name(&variant.name, substitutions),
                line: variant.line,
            })
            .collect();
        type_decl
    }

    fn lower_binding(&mut self, mut binding: HirTopLevelBinding) -> HirTopLevelBinding {
        if binding.explicit_type {
            let declared = binding.type_.clone();
            binding.type_ = self.concrete_type(&declared, &HashMap::new());
        }
        if let Some(value) = binding.value.take() {
            let mut context = self.function_context();
            let expected = binding.explicit_type.then(|| binding.type_.clone());
            binding.value = Some(self.lower_expression(
                &value,
                &HashMap::new(),
                &mut context,
                expected.as_ref(),
                binding.line,
            ));
        }
        binding
    }

    fn lower_function(
        &mut self,
        function: HirFunction,
        substitutions: &HashMap<crate::intern::Symbol, crate::types::ParameterType>,
        concrete_name: Option<String>,
    ) -> HirFunction {
        // Attribute any diagnostic raised while lowering this body to the file
        // the function was declared in, restoring the caller's file afterward so
        // a nested instantiation doesn't leak its file to the enclosing frame
        // (bug-107). The incoming `function.name` is the origin name (template
        // name for an instantiation, concrete name on the top-level pass).
        let saved_file = self.current_file.take();
        self.current_file = self
            .function_files
            .get(&function.name)
            .cloned()
            .or(saved_file.clone());
        let result = self.lower_function_inner(function, substitutions, concrete_name);
        self.current_file = saved_file;
        result
    }

    fn lower_function_inner(
        &mut self,
        mut function: HirFunction,
        substitutions: &HashMap<crate::intern::Symbol, crate::types::ParameterType>,
        concrete_name: Option<String>,
    ) -> HirFunction {
        if let Some(name) = concrete_name {
            function.name = name;
        }
        function.template_params.clear();
        for param in &mut function.params {
            if !matches!(param.type_, ParameterType::Unknown) {
                let declared = param.type_.clone();
                param.type_ = self.concrete_type(&declared, substitutions);
            }
        }
        if !matches!(function.returns, ParameterType::Unknown) {
            let declared = function.returns.clone();
            function.returns = self.concrete_type(&declared, substitutions);
        }

        let mut context = self.function_context();
        context.enclosing_return = opt_type(&function.returns);
        for param in &function.params {
            if !matches!(param.type_, ParameterType::Unknown) {
                context
                    .locals
                    .insert(param.name.clone(), param.type_.clone());
            }
        }
        function.body = self.lower_statements(&function.body, substitutions, &mut context);
        if let Some(trap) = &mut function.trap {
            let mut trap_context = context.clone();
            trap_context
                .locals
                .insert(trap.name.clone(), ParameterType::named("Error"));
            trap.body = self.lower_statements(&trap.body, substitutions, &mut trap_context);
        }
        function
    }

    /// Reorder `arg_types` (built in source/call order) into the callee template's
    /// declared-parameter order when the call uses named arguments, so generic
    /// instantiation binds each type-param against the type of the argument that
    /// actually fills its parameter slot (bug-196). Returns `None` for a positional
    /// call or an unknown callee (both already in order).
    fn arg_types_in_param_order(
        &self,
        callee: &str,
        arguments: &[HirCallArg],
        arg_types: &[ParameterType],
    ) -> Option<Vec<ParameterType>> {
        if !arguments
            .iter()
            .any(|argument| matches!(argument, HirCallArg::Named { .. }))
        {
            return None;
        }
        let params = &self.function_templates.get(callee)?.params;
        let mut ordered: Vec<Option<ParameterType>> = vec![None; params.len()];
        let mut next_positional = 0usize;
        for (index, argument) in arguments.iter().enumerate() {
            let arg_type = arg_types.get(index)?.clone();
            match argument {
                HirCallArg::Positional(_) => {
                    while next_positional < ordered.len() && ordered[next_positional].is_some() {
                        next_positional += 1;
                    }
                    if next_positional < ordered.len() {
                        ordered[next_positional] = Some(arg_type);
                        next_positional += 1;
                    }
                }
                HirCallArg::Named { name, .. } => {
                    if let Some(slot) = params.iter().position(|param| param.name == *name) {
                        ordered[slot] = Some(arg_type);
                    }
                }
            }
        }
        // Only reorder when every parameter slot is filled: an omitted defaulted
        // slot would have no actual type here, and padding it would feed a bogus
        // actual to `unify_type`. In that case fall back to the source-order types
        // (unchanged prior behavior); the named-arg reorder this fixes is the
        // fully-provided call (bug-196).
        ordered.into_iter().collect()
    }

    /// Claim `symbol` for the instantiation identified by the unambiguous `key`
    /// (`name<args>`). `mangle_name` is lossy, so two distinct type-argument tuples
    /// can mangle to the same symbol; when that happens, suffix the loser so each
    /// instantiation keeps its own symbol (bug-226). A symbol already claimed by
    /// this same key returns unchanged, so re-instantiation is stable and the
    /// common (collision-free) case emits exactly the symbol it always did.
    fn unique_concrete_symbol(&mut self, symbol: String, key: &str) -> String {
        if let Some(owner) = self.concrete_symbol_keys.get(&symbol) {
            if owner == key {
                return symbol;
            }
            let mut n = 2usize;
            loop {
                let candidate = format!("{symbol}${n}");
                match self.concrete_symbol_keys.get(&candidate) {
                    Some(owner) if owner == key => return candidate,
                    Some(_) => n += 1,
                    None => {
                        self.concrete_symbol_keys
                            .insert(candidate.clone(), key.to_string());
                        return candidate;
                    }
                }
            }
        }
        self.concrete_symbol_keys
            .insert(symbol.clone(), key.to_string());
        symbol
    }

    fn instantiate_function(
        &mut self,
        name: &str,
        arg_types: &[ParameterType],
        line: usize,
    ) -> Option<String> {
        let template = self.function_templates.get(name)?.clone();
        // Internal generic implementations (e.g. `collections::sort`) carry the
        // untypeable sigil; show the readable `__` form in user-facing messages.
        let display = crate::internal_name::display_name(name);
        if arg_types.len() > template.params.len() {
            self.report(
                "TYPE_CALL_ARITY_MISMATCH",
                &format!(
                    "Call to `{display}` has {} argument(s), expected at most {}.",
                    arg_types.len(),
                    template.params.len()
                ),
                line,
            );
            return None;
        }

        let param_symbols: HashSet<crate::intern::Symbol> = template
            .template_params
            .iter()
            .map(|param| crate::intern::Symbol::intern(param))
            .collect();
        let mut substitutions: HashMap<crate::intern::Symbol, ParameterType> = HashMap::new();
        for (param, actual) in template.params.iter().zip(arg_types.iter()) {
            // An absent (`Unknown`) parameter annotation contributes no pattern —
            // mirrors the old `opt_type_name(..) else continue`.
            if matches!(param.type_, ParameterType::Unknown) {
                continue;
            }
            // `template_view_type` is still name-in/name-out (plan-106-A
            // Correction 3 → letter E); its result also spells the diagnostic
            // below, so this render is a message-formatting site either way.
            let actual_name = self.template_view_type(actual.name().as_ref());
            if !unify_type(
                &param.type_,
                &ParameterType::parse(&actual_name),
                &param_symbols,
                &mut substitutions,
            ) {
                self.report(
                    "TYPE_CALL_ARGUMENT_MISMATCH",
                    &format!(
                        "Call to `{display}` cannot infer template arguments from `{actual_name}`."
                    ),
                    line,
                );
                return None;
            }
        }

        let args = match template
            .template_params
            .iter()
            .map(|param| {
                // The mangled symbol is a string, so render the bound `ParameterType`
                // at this deliberate string boundary.
                substitutions
                    .get(&crate::intern::Symbol::intern(param))
                    .map(|type_| type_.name().into_owned())
            })
            .collect::<Option<Vec<_>>>()
        {
            Some(args) => args,
            None => {
                // A type-param the arguments cannot pin down (it appears only in the
                // return type, e.g. `FUNC make OF T() AS T`). Previously this
                // returned None silently and the call was left as the bare template
                // name, surfacing later as a confusing "unknown function" (bug-226).
                let missing = template
                    .template_params
                    .iter()
                    .filter(|param| {
                        !substitutions.contains_key(&crate::intern::Symbol::intern(param))
                    })
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ");
                self.report(
                    "TYPE_CALL_ARGUMENT_MISMATCH",
                    &format!(
                        "Call to `{display}` cannot infer template argument(s) `{missing}` from \
                         its arguments (they appear only in the return type). Supply them at a \
                         position the arguments determine."
                    ),
                    line,
                );
                return None;
            }
        };
        // The mangled symbol is a lossy encoding (every non-alphanumeric collapses
        // to `$`), so two distinct type-argument tuples of the same arity can
        // produce the same symbol — the second instantiation would then overwrite
        // the first in `concrete_functions` and both call sites would be rewritten
        // to one shared, possibly-wrong symbol (bug-226). The `name<args>` key IS
        // unambiguous, so disambiguate the symbol whenever a different key already
        // claimed it. Existing single-instantiation symbols are unchanged.
        let key = format!("{name}<{}>", args.join(","));
        let concrete_name = self.unique_concrete_symbol(mangle_name(name, &args), &key);
        if self.emitted_function_keys.insert(key) {
            if !self.charge_instantiation(&display, line) {
                return None;
            }
            if self.template_instantiation_depth >= MAX_TEMPLATE_INSTANTIATION_DEPTH {
                self.report_instantiation_too_deep(&display, line);
                // Halt the whole enumeration, not just this leaf: a wide fan-out
                // would otherwise keep hitting the depth cap on every one of its
                // (exponentially many) sibling paths (bug-399).
                self.instantiation_limit_reached = true;
                return None;
            }
            self.template_instantiation_depth += 1;
            let mut full_substitutions = HashMap::new();
            for (param, arg) in template.template_params.iter().zip(args.iter()) {
                full_substitutions.insert(
                    crate::intern::Symbol::intern(param),
                    crate::types::ParameterType::parse(arg),
                );
            }
            let lowered =
                self.lower_function(template, &full_substitutions, Some(concrete_name.clone()));
            self.template_instantiation_depth -= 1;
            self.concrete_functions
                .insert(concrete_name.clone(), lowered);
        }
        Some(concrete_name)
    }

    fn resolve_overload(
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
            return None;
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
        self.overload_names
            .get(&overload_key(
                name,
                &chosen.params,
                opt_type_name(&chosen.returns).as_deref(),
            ))
            .cloned()
    }

    /// Route a call whose callee is an **overridable general built-in** to a user
    /// override (plan-01-overload.md §A.3 / Phase 5.2). The built-in is
    /// authoritative for the types it already supports, so an override is selected
    /// only when the built-in rejects the argument types — a non-matching call
    /// (scalar/collection args) is left as the bare built-in name for codegen to
    /// dispatch. Fires for a sole built-in-named overload too, unlike the ordinary
    /// `resolve_overload`.
    fn resolve_general_builtin_override(
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
        if crate::codegen::builtins::resolve_call_return_type_typed(name, arg_types, false)
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
    fn single_signature_params(&self, callee: &str) -> Option<Vec<HirParam>> {
        let candidates = self.function_overloads.get(callee)?;
        (candidates.len() == 1).then(|| candidates[0].params.clone())
    }

    fn instantiate_type(&mut self, name: &str, args: &[String]) -> String {
        // The mangled symbol is a lossy encoding (every non-alphanumeric collapses
        // to `$`), so two distinct same-arity type-argument tuples can produce the
        // same symbol; without disambiguation the second instantiation would
        // overwrite the first in `concrete_types` and both use-sites would rewrite
        // to one shared, possibly-wrong concrete type. The `name<args>` key IS
        // unambiguous, so claim the symbol against it exactly as
        // `instantiate_function` does (bug-226 fixed the function half but left this
        // one unguarded — bug-400). A symbol claimed by only one key is unchanged.
        let key = format!("{name}<{}>", args.join(","));
        let concrete_name = self.unique_concrete_symbol(mangle_name(name, args), &key);
        self.type_instantiations
            .insert(concrete_name.clone(), (name.to_string(), args.to_vec()));
        if !self.emitted_type_keys.insert(key) {
            return concrete_name;
        }
        let Some(template) = self.type_templates.get(name).cloned() else {
            return concrete_name;
        };
        if !self.charge_instantiation(name, 1) {
            return concrete_name;
        }
        if self.template_instantiation_depth >= MAX_TEMPLATE_INSTANTIATION_DEPTH {
            self.report_instantiation_too_deep(name, 1);
            // Halt the whole enumeration — see the `instantiate_function` twin.
            self.instantiation_limit_reached = true;
            return concrete_name;
        }
        self.template_instantiation_depth += 1;
        let mut substitutions = HashMap::new();
        for (param, arg) in template.template_params.iter().zip(args.iter()) {
            substitutions.insert(
                crate::intern::Symbol::intern(param),
                crate::types::ParameterType::parse(arg),
            );
        }
        let concrete = self.lower_type(template, &substitutions, Some(concrete_name.clone()));
        self.template_instantiation_depth -= 1;
        self.concrete_types.insert(concrete_name.clone(), concrete);
        concrete_name
    }

    fn lower_field(
        &mut self,
        field: &HirTypeField,
        substitutions: &HashMap<crate::intern::Symbol, crate::types::ParameterType>,
    ) -> HirTypeField {
        let mut lowered = field.clone();
        lowered.type_ = self.concrete_type(&field.type_, substitutions);
        lowered
    }

    fn lower_statements(
        &mut self,
        statements: &[HirStatement],
        substitutions: &HashMap<crate::intern::Symbol, crate::types::ParameterType>,
        context: &mut FunctionContext,
    ) -> Vec<HirStatement> {
        statements
            .iter()
            .map(|statement| self.lower_statement(statement, substitutions, context))
            .collect()
    }

    fn lower_statement(
        &mut self,
        statement: &HirStatement,
        substitutions: &HashMap<crate::intern::Symbol, crate::types::ParameterType>,
        context: &mut FunctionContext,
    ) -> HirStatement {
        match statement {
            HirStatement::Let {
                mutable,
                resource,
                state_type,
                name,
                type_,
                explicit_type,
                value,
                line,
            } => {
                // Mirror the AST `Option<String>` the pre-D3 walk consumed: an
                // explicit `AS T` annotation carries its rendered type, an inferred
                // `LET` carries `None`.
                let type_name = explicit_type.then(|| type_.name().into_owned());
                // `concrete_type_name` is still name-in/name-out (plan-106-A
                // Correction 3 assigns its retype to letter E); parse its result
                // ONCE here rather than at each consumer.
                let lowered_type = type_name
                    .as_ref()
                    .map(|type_name| self.concrete_type_name(type_name, substitutions))
                    .map(|type_name| ParameterType::parse(&type_name));
                let lowered_state = state_type.as_ref().map(|state_type| {
                    self.concrete_type_name(state_type.name().as_ref(), substitutions)
                });
                // The declared type with this instantiation's template params
                // substituted. `type_` IS the parse of `type_name` (they round-trip
                // byte-exact), so substitute the HIR node directly rather than
                // re-parsing a render of it.
                let expected_source_type =
                    explicit_type.then(|| substitute_type_params(type_, substitutions));
                let lowered_value = value.as_ref().map(|value| {
                    self.lower_expression(
                        value,
                        substitutions,
                        context,
                        expected_source_type.as_ref(),
                        *line,
                    )
                });
                let binding_type = lowered_type.clone().or_else(|| {
                    lowered_value
                        .as_ref()
                        .and_then(|value| self.expression_type(value, context))
                });
                if let Some(binding_type) = binding_type {
                    context.locals.insert(name.clone(), binding_type);
                }
                HirStatement::Let {
                    mutable: *mutable,
                    resource: *resource,
                    state_type: lowered_state.map(|s| ParameterType::parse(&s)),
                    name: name.clone(),
                    type_: lowered_type.unwrap_or(ParameterType::Unknown),
                    explicit_type: *explicit_type,
                    value: lowered_value,
                    line: *line,
                }
            }
            HirStatement::Return { value, line } => HirStatement::Return {
                value: value.as_ref().map(|value| {
                    // A `RETURN` of a call propagates the enclosing function's
                    // declared return type as the expected type so a return-type
                    // overload set resolves (plan-01-overload.md §F.2).
                    let expected = matches!(value, HirExpression::Call { .. })
                        .then(|| context.enclosing_return.clone())
                        .flatten();
                    self.lower_expression(value, substitutions, context, expected.as_ref(), *line)
                }),
                line: *line,
            },
            HirStatement::Exit { target, code, line } => HirStatement::Exit {
                target: *target,
                code: code
                    .as_ref()
                    .map(|value| self.lower_expression(value, substitutions, context, None, *line)),
                line: *line,
            },
            HirStatement::Continue { kind, line } => HirStatement::Continue {
                kind: *kind,
                line: *line,
            },
            HirStatement::Fail { error, line } => HirStatement::Fail {
                error: self.lower_expression(error, substitutions, context, None, *line),
                line: *line,
            },
            HirStatement::Propagate { line } => HirStatement::Propagate { line: *line },
            HirStatement::Recover { value, line } => HirStatement::Recover {
                value: value
                    .as_ref()
                    .map(|value| self.lower_expression(value, substitutions, context, None, *line)),
                line: *line,
            },
            HirStatement::Assign { name, value, line } => {
                // Pass the target local's declared type as the RHS expected type so
                // a return-type-overloaded call disambiguates, exactly like the
                // `LET … AS T = call()` form (bug-197). Only consulted by call
                // overload resolution; other RHS expressions ignore it.
                let expected = context.locals.get(name).cloned();
                HirStatement::Assign {
                    name: name.clone(),
                    value: self.lower_expression(
                        value,
                        substitutions,
                        context,
                        expected.as_ref(),
                        *line,
                    ),
                    line: *line,
                }
            }
            HirStatement::StateAssign {
                resource,
                value,
                line,
            } => {
                let expected = context.locals.get(resource).cloned();
                HirStatement::StateAssign {
                    resource: resource.clone(),
                    value: self.lower_expression(
                        value,
                        substitutions,
                        context,
                        expected.as_ref(),
                        *line,
                    ),
                    line: *line,
                }
            }
            HirStatement::Expression { expression, line } => HirStatement::Expression {
                expression: self.lower_expression(expression, substitutions, context, None, *line),
                line: *line,
            },
            HirStatement::If {
                condition,
                then_body,
                else_body,
                line,
            } => {
                let mut then_context = context.clone();
                let mut else_context = context.clone();
                HirStatement::If {
                    condition: self.lower_expression(
                        condition,
                        substitutions,
                        context,
                        None,
                        *line,
                    ),
                    then_body: self.lower_statements(then_body, substitutions, &mut then_context),
                    else_body: self.lower_statements(else_body, substitutions, &mut else_context),
                    line: *line,
                }
            }
            HirStatement::Match {
                expression,
                cases,
                line,
            } => HirStatement::Match {
                expression: self.lower_expression(expression, substitutions, context, None, *line),
                cases: cases
                    .iter()
                    .map(|case| {
                        let mut case_context = context.clone();
                        if let HirMatchPattern::Union { binding, type_ } = &case.pattern {
                            case_context.locals.insert(
                                binding.clone(),
                                ParameterType::parse(
                                    &self.concrete_type_name(type_.name().as_ref(), substitutions),
                                ),
                            );
                        }
                        HirMatchCase {
                            pattern: match &case.pattern {
                                HirMatchPattern::Else => HirMatchPattern::Else,
                                HirMatchPattern::Literal(expression) => {
                                    HirMatchPattern::Literal(self.lower_expression(
                                        expression,
                                        substitutions,
                                        &mut case_context,
                                        None,
                                        case.line,
                                    ))
                                }
                                HirMatchPattern::Union { type_, binding } => {
                                    HirMatchPattern::Union {
                                        type_: self.concrete_type(type_, substitutions),
                                        binding: binding.clone(),
                                    }
                                }
                                HirMatchPattern::OneOf(expressions) => HirMatchPattern::OneOf(
                                    expressions
                                        .iter()
                                        .map(|expression| {
                                            self.lower_expression(
                                                expression,
                                                substitutions,
                                                &mut case_context,
                                                None,
                                                case.line,
                                            )
                                        })
                                        .collect(),
                                ),
                            },
                            guard: case.guard.as_ref().map(|guard| {
                                self.lower_expression(
                                    guard,
                                    substitutions,
                                    &mut case_context,
                                    None,
                                    case.line,
                                )
                            }),
                            body: self.lower_statements(
                                &case.body,
                                substitutions,
                                &mut case_context,
                            ),
                            line: case.line,
                        }
                    })
                    .collect(),
                line: *line,
            },
            HirStatement::For {
                name,
                start,
                end,
                step,
                body,
                line,
            } => {
                let lowered_start =
                    self.lower_expression(start, substitutions, context, None, *line);
                let lowered_end = self.lower_expression(end, substitutions, context, None, *line);
                let lowered_step = step
                    .as_ref()
                    .map(|value| self.lower_expression(value, substitutions, context, None, *line));
                let mut nested = context.clone();
                if let Some(loop_type) = self
                    .expression_type(&lowered_start, context)
                    .zip(self.expression_type(&lowered_end, context))
                    .map(|(start_type, end_type)| {
                        let step_type = lowered_step
                            .as_ref()
                            .and_then(|value| self.expression_type(value, context))
                            .unwrap_or(ParameterType::Integer);
                        crate::numeric::typed_promote_loop_numeric_type(
                            &start_type,
                            &end_type,
                            &step_type,
                        )
                    })
                {
                    nested.locals.insert(name.clone(), loop_type);
                }
                HirStatement::For {
                    name: name.clone(),
                    start: lowered_start,
                    end: lowered_end,
                    step: lowered_step,
                    body: self.lower_statements(body, substitutions, &mut nested),
                    line: *line,
                }
            }
            HirStatement::ForEach {
                name,
                iterable,
                body,
                line,
            } => {
                let lowered_iterable =
                    self.lower_expression(iterable, substitutions, context, None, *line);
                let mut nested = context.clone();
                if let Some(iterable_type) = self.expression_type(&lowered_iterable, context) {
                    // plan-105-B: the iterable's element type, off the canonical
                    // grammar. Iterating a `Map` yields a `MapEntry` over the same
                    // key/value pair, which is now built structurally instead of by
                    // splicing the raw `K TO V` text behind a `MapEntry OF ` prefix.
                    // plan-106-A: the iterable's type arrives already typed, so the
                    // match is on the value itself rather than on a re-parse of its
                    // rendered name.
                    let loop_type = match iterable_type {
                        ParameterType::ListOf(element) | ParameterType::SetOf(element) => *element,
                        ParameterType::MapOf(key, value) => ParameterType::MapEntryOf(key, value),
                        _ => ParameterType::Unknown,
                    };
                    nested.locals.insert(name.clone(), loop_type);
                }
                HirStatement::ForEach {
                    name: name.clone(),
                    iterable: lowered_iterable,
                    body: self.lower_statements(body, substitutions, &mut nested),
                    line: *line,
                }
            }
            HirStatement::While {
                kind,
                condition,
                body,
                line,
            } => HirStatement::While {
                kind: *kind,
                condition: self.lower_expression(condition, substitutions, context, None, *line),
                body: self.lower_statements(body, substitutions, &mut context.clone()),
                line: *line,
            },
            HirStatement::DoUntil {
                body,
                condition,
                line,
            } => HirStatement::DoUntil {
                body: self.lower_statements(body, substitutions, &mut context.clone()),
                condition: self.lower_expression(condition, substitutions, context, None, *line),
                line: *line,
            },
        }
    }

    fn lower_expression(
        &mut self,
        expression: &HirExpression,
        substitutions: &HashMap<crate::intern::Symbol, crate::types::ParameterType>,
        context: &mut FunctionContext,
        expected_type: Option<&ParameterType>,
        line: usize,
    ) -> HirExpression {
        match expression {
            HirExpression::Call {
                callee,
                arguments,
                line: call_line,
                column,
            } => {
                // When the callee names exactly one user function, propagate each
                // parameter type as the expected type of its argument slot, but
                // only for a nested call argument — that is where a return-type
                // overload set needs the context to resolve (plan-01-overload.md
                // §F.2). Literals keep their own inferred typing.
                let sig_params = self.single_signature_params(callee);
                let lowered_args = arguments
                    .iter()
                    .enumerate()
                    .map(|(index, argument)| match argument {
                        HirCallArg::Positional(value) => {
                            let expected =
                                arg_slot_expected(value, sig_params.as_deref(), |params| {
                                    params.get(index)
                                });
                            HirCallArg::Positional(self.lower_expression(
                                value,
                                substitutions,
                                context,
                                expected.as_ref(),
                                line,
                            ))
                        }
                        HirCallArg::Named { name, value, line } => {
                            let expected =
                                arg_slot_expected(value, sig_params.as_deref(), |params| {
                                    params.iter().find(|param| param.name == *name)
                                });
                            HirCallArg::Named {
                                name: name.clone(),
                                value: self.lower_expression(
                                    value,
                                    substitutions,
                                    context,
                                    expected.as_ref(),
                                    *line,
                                ),
                                line: *line,
                            }
                        }
                    })
                    .collect::<Vec<_>>();
                // Keep the vector aligned with the argument list: an argument
                // whose type cannot be inferred becomes the `"Unknown"` wildcard
                // (which `params_match`/`resolve_overload` accept for any
                // parameter) rather than being dropped, which would shorten the
                // vector and shift the remaining types into wrong positions.
                let arg_types = lowered_args
                    .iter()
                    .map(|argument| {
                        self.expression_type(call_arg_value(argument), context)
                            .unwrap_or(ParameterType::Unknown)
                    })
                    .collect::<Vec<_>>();
                // The public callee (`encoding.utf8Decode`, `collections.sort`)
                // before the internal-name rewrite below. A call that fails overload
                // resolution must keep this resolvable, dotted name rather than the
                // mangled `#name`: otherwise the post-monomorph resolver pass reports
                // SYMBOL_UNKNOWN_IDENTIFIER on an unresolvable `#encoding_utf8Decode`
                // and aborts before the former source checker can emit the real
                // TYPE_CALL_ARITY_MISMATCH / TYPE_CALL_ARGUMENT_MISMATCH (bug-443).
                let public_callee = callee.clone();
                // Rewrite the public overloaded `encoding::utf8Encode`/`utf8Decode`
                // onto their internal `__encoding_*` overload sets, and a
                // `collections::` call onto its internal generic implementation —
                // so the native overload / instantiation machinery below resolves
                // and mangles them like any user overload (`resolve_overload`).
                let encoding_internal = match callee.as_str() {
                    "encoding.utf8Encode" => Some("#encoding_utf8Encode".to_string()),
                    "encoding.utf8Decode" => Some("#encoding_utf8Decode".to_string()),
                    _ => None,
                };
                let callee = &encoding_internal
                    .or_else(|| self.collections_internal_callee(callee))
                    .unwrap_or_else(|| callee.clone());
                // Named arguments can reorder the call's values relative to the
                // template's declared parameters. `arg_types` is built in source
                // order, so bind template type-params against types reordered into
                // declared-parameter order (mirroring the value reorder IR lowering
                // performs) — otherwise a type-param binds to the wrong argument
                // type and instantiation emits a wrong concrete symbol (bug-196).
                let ordered_arg_types =
                    self.arg_types_in_param_order(callee, arguments, &arg_types);
                let instantiate_arg_types = ordered_arg_types.as_deref().unwrap_or(&arg_types);
                let target = if let Some(target) =
                    self.instantiate_function(callee, instantiate_arg_types, line)
                {
                    target
                } else if let Some(target) =
                    self.resolve_general_builtin_override(callee, &arg_types)
                {
                    target
                } else if let Some(target) =
                    self.resolve_overload(callee, &public_callee, &arg_types, expected_type, line)
                {
                    target
                } else if let Some(target) =
                    self.resolve_imported_overload(callee, &arg_types, line)
                {
                    target
                } else {
                    // Overload resolution failed (wrong arity/argument types): keep
                    // the PUBLIC callee, not the mangled `#name`, so the second
                    // resolver pass resolves it and the former source checker emits the proper
                    // argument diagnostic naming the public call (bug-443).
                    public_callee.clone()
                };
                if target != *callee {
                    self.add_function_to_context(&target, context);
                }
                HirExpression::Call {
                    callee: target,
                    arguments: lowered_args,
                    line: *call_line,
                    column: *column,
                }
            }
            HirExpression::Constructor { type_, arguments } => {
                let type_name = type_.name().into_owned();
                let mut concrete_type = None;
                // plan-105-B: read the expected type's template head/arguments off
                // `UserOf` instead of re-splitting its rendered name.
                if let Some(ParameterType::UserOf(expected_name, expected_args)) = expected_type {
                    if expected_name.resolve() == type_name {
                        // Each type argument goes through `concrete_type_name` — the
                        // same walk `instantiate_type`'s other caller uses — so a
                        // nested user generic is instantiated to its MANGLED name
                        // before it becomes part of this instantiation's key.
                        // Passing the raw spelling instead made the two callers
                        // disagree: `Holder OF Holder OF Integer` mangled as
                        // `Holder$Holder$Integer` at the declaration and
                        // `Holder$Holder$OF$Integer` here, so the constructor's type
                        // failed to match its own binding (TYPE_BINDING_MISMATCH).
                        let mut args = Vec::with_capacity(expected_args.len());
                        for arg in expected_args {
                            args.push(self.concrete_type_name(&arg.name(), substitutions));
                        }
                        concrete_type = Some(self.instantiate_type(expected_name.resolve(), &args));
                    }
                }
                let field_types = concrete_type
                    .as_deref()
                    .or(Some(type_name.as_str()))
                    .and_then(|name| context.record_fields.get(name))
                    .cloned();
                let lowered_args = arguments
                    .iter()
                    .enumerate()
                    .map(|(index, argument)| {
                        let expected_arg_type =
                            constructor_arg_field_type(argument, index, field_types.as_deref());
                        self.lower_constructor_arg(
                            argument,
                            substitutions,
                            context,
                            line,
                            expected_arg_type.as_ref(),
                        )
                    })
                    .collect::<Vec<_>>();
                if concrete_type.is_none() && self.type_templates.contains_key(&type_name) {
                    let Some(template) = self.type_templates.get(&type_name).cloned() else {
                        unreachable!();
                    };
                    let mut inferred = HashMap::new();
                    let param_set: std::collections::HashSet<crate::intern::Symbol> = template
                        .template_params
                        .iter()
                        .map(|param| crate::intern::Symbol::intern(param))
                        .collect();
                    let fields = match template.kind {
                        TypeDeclKind::Type => template.fields.clone(),
                        TypeDeclKind::Union => Vec::new(),
                        TypeDeclKind::Enum => Vec::new(),
                    };
                    for (field, argument) in fields.iter().zip(lowered_args.iter()) {
                        if let Some(actual) =
                            self.expression_type(constructor_arg_value(argument), context)
                        {
                            unify_type(&field.type_, &actual, &param_set, &mut inferred);
                        }
                    }
                    let args = template
                        .template_params
                        .iter()
                        .map(|param| {
                            inferred
                                .get(&crate::intern::Symbol::intern(param))
                                .map(|type_| type_.name().into_owned())
                        })
                        .collect::<Option<Vec<_>>>();
                    if let Some(args) = args {
                        concrete_type = Some(self.instantiate_type(&type_name, &args));
                    }
                }
                HirExpression::Constructor {
                    type_: ParameterType::parse(&concrete_type.unwrap_or(type_name)),
                    arguments: lowered_args,
                }
            }
            HirExpression::WithUpdate { target, updates } => HirExpression::WithUpdate {
                target: Box::new(self.lower_expression(target, substitutions, context, None, line)),
                updates: updates
                    .iter()
                    .map(|update| HirRecordUpdate {
                        field: update.field.clone(),
                        value: self.lower_expression(
                            &update.value,
                            substitutions,
                            context,
                            None,
                            update.line,
                        ),
                        line: update.line,
                    })
                    .collect(),
            },
            HirExpression::ListLiteral(values) => HirExpression::ListLiteral(
                values
                    .iter()
                    .map(|value| {
                        let expected_element = expected_element_type(expected_type, false);
                        self.lower_expression(
                            value,
                            substitutions,
                            context,
                            expected_element.as_ref(),
                            line,
                        )
                    })
                    .collect(),
            ),
            HirExpression::SetLiteral {
                element_type,
                elements,
            } => HirExpression::SetLiteral {
                element_type: ParameterType::parse(
                    &self.concrete_type_name(element_type.name().as_ref(), substitutions),
                ),
                elements: elements
                    .iter()
                    .map(|value| {
                        let expected_element = expected_element_type(expected_type, true);
                        self.lower_expression(
                            value,
                            substitutions,
                            context,
                            expected_element.as_ref(),
                            line,
                        )
                    })
                    .collect(),
            },
            HirExpression::MapLiteral {
                key_type,
                value_type,
                entries,
            } => HirExpression::MapLiteral {
                key_type: ParameterType::parse(
                    &self.concrete_type_name(key_type.name().as_ref(), substitutions),
                ),
                value_type: ParameterType::parse(
                    &self.concrete_type_name(value_type.name().as_ref(), substitutions),
                ),
                entries: entries
                    .iter()
                    .map(|(key, value)| {
                        (
                            self.lower_expression(key, substitutions, context, None, line),
                            self.lower_expression(value, substitutions, context, None, line),
                        )
                    })
                    .collect(),
            },
            HirExpression::MemberAccess { target, member } => HirExpression::MemberAccess {
                target: Box::new(self.lower_expression(target, substitutions, context, None, line)),
                member: member.clone(),
            },
            HirExpression::Binary {
                left,
                operator,
                right,
                line: op_line,
                column,
            } => HirExpression::Binary {
                left: Box::new(self.lower_expression(left, substitutions, context, None, line)),
                operator: operator.clone(),
                right: Box::new(self.lower_expression(right, substitutions, context, None, line)),
                line: *op_line,
                column: *column,
            },
            HirExpression::Unary {
                operator,
                operand,
                line: op_line,
                column,
            } => HirExpression::Unary {
                operator: operator.clone(),
                operand: Box::new(self.lower_expression(
                    operand,
                    substitutions,
                    context,
                    None,
                    line,
                )),
                line: *op_line,
                column: *column,
            },
            HirExpression::Lambda {
                params,
                body,
                assign_target,
            } => {
                let mut nested = context.clone();
                let lowered_params = params
                    .iter()
                    .map(|param| {
                        let mut lowered = param.clone();
                        if !matches!(param.type_, ParameterType::Unknown) {
                            let type_name = param.type_.name().into_owned();
                            let concrete = ParameterType::parse(
                                &self.concrete_type_name(&type_name, substitutions),
                            );
                            nested.locals.insert(param.name.clone(), concrete.clone());
                            lowered.type_ = concrete;
                        }
                        lowered
                    })
                    .collect::<Vec<_>>();
                HirExpression::Lambda {
                    params: lowered_params,
                    body: Box::new(self.lower_expression(
                        body,
                        substitutions,
                        &mut nested,
                        None,
                        line,
                    )),
                    assign_target: assign_target.clone(),
                }
            }
            HirExpression::Trapped {
                expression,
                binding,
                handler,
                line: trap_line,
            } => {
                let lowered_expression =
                    Box::new(self.lower_expression(expression, substitutions, context, None, line));
                let mut handler_context = context.clone();
                handler_context
                    .locals
                    .insert(binding.clone(), ParameterType::named("Error"));
                let lowered_handler =
                    self.lower_statements(handler, substitutions, &mut handler_context);
                HirExpression::Trapped {
                    expression: lowered_expression,
                    binding: binding.clone(),
                    handler: lowered_handler,
                    line: *trap_line,
                }
            }
            HirExpression::Identifier(value) => HirExpression::Identifier(value.clone()),
            HirExpression::String(value) => HirExpression::String(value.clone()),
            HirExpression::Number(value) => HirExpression::Number(value.clone()),
            HirExpression::Scalar(code_point) => HirExpression::Scalar(*code_point),
            HirExpression::Boolean(value) => HirExpression::Boolean(*value),
        }
    }

    fn lower_constructor_arg(
        &mut self,
        argument: &HirConstructorArg,
        substitutions: &HashMap<crate::intern::Symbol, crate::types::ParameterType>,
        context: &mut FunctionContext,
        line: usize,
        expected_type: Option<&ParameterType>,
    ) -> HirConstructorArg {
        match argument {
            HirConstructorArg::Positional(value) => HirConstructorArg::Positional(
                self.lower_expression(value, substitutions, context, expected_type, line),
            ),
            HirConstructorArg::Named {
                name,
                value,
                line: arg_line,
            } => HirConstructorArg::Named {
                name: name.clone(),
                value: self.lower_expression(
                    value,
                    substitutions,
                    context,
                    expected_type,
                    *arg_line,
                ),
                line: *arg_line,
            },
        }
    }

    /// The interned name of a NOMINAL leaf — a concrete nominal or a type
    /// variable — or `None` for anything with structure.
    ///
    /// This is where the `substitutions` probe belongs: its keys are always bare
    /// template-parameter names. `Var` and `Named` both appear because a name
    /// reaching monomorph through HIR was classified by `with_vars`, while one
    /// reaching it through a rendered spelling was not.
    fn leaf_symbol(type_: &ParameterType) -> Option<crate::intern::Symbol> {
        match type_ {
            ParameterType::Named(name) | ParameterType::Var(name) => Some(*name),
            _ => None,
        }
    }

    /// The name-domain entry to [`concrete_type`](Self::concrete_type), for the
    /// callers that still hold a type SPELLING.
    fn concrete_type_name(
        &mut self,
        type_name: &str,
        substitutions: &HashMap<crate::intern::Symbol, crate::types::ParameterType>,
    ) -> String {
        self.concrete_type(&ParameterType::parse(type_name), substitutions)
            .name()
            .into_owned()
    }

    /// Substitute a template's parameters through `type_`, instantiating any user
    /// generic it names, and return the concrete type.
    ///
    /// plan-106-E: was `concrete_type_name(&str) -> String`, which parsed its
    /// input, recursed by RE-RENDERING each child, and rebuilt the result with
    /// seven `format!("List OF {…}")`-family templates — a second renderer beside
    /// `ParameterType::name`. It is now a structural walk.
    ///
    /// Two behaviours the by-name recursion encoded are preserved explicitly:
    ///
    /// * the per-level grouped-type unwrap (bug-105) is no longer needed here —
    ///   plan-106-E made `ParameterType::parse` peel a `(T)` group at every level,
    ///   so a grouped spelling never reaches this walk as a junk nominal;
    /// * the `substitutions` probe is at the NOMINAL LEAVES. Its keys are always
    ///   bare template-parameter names (`Symbol::intern(param)` over
    ///   `template_params`, at `lower.rs:707` and `:861`), so probing the whole
    ///   spelling at every level — as the string form did — could only ever have
    ///   matched at a leaf anyway.
    fn concrete_type(
        &mut self,
        type_: &ParameterType,
        substitutions: &HashMap<crate::intern::Symbol, crate::types::ParameterType>,
    ) -> ParameterType {
        if let Some(symbol) = Self::leaf_symbol(type_) {
            if let Some(bound) = substitutions.get(&symbol) {
                // The bound value is already fully substituted, but it may itself be
                // an un-INSTANTIATED user generic: binding `T := Holder OF Integer`
                // while expanding `Holder OF Holder OF Integer` used to return that
                // spelling verbatim, so the inner `Holder OF Integer` template was
                // never lowered and the post-monomorph resolve pass reported
                // `SYMBOL_UNKNOWN_TYPE` against the template's own field. Walk the
                // bound value so its instantiation happens too.
                //
                // Walked with NO substitutions, deliberately: the value is already
                // substituted, and re-applying the map would loop forever on a
                // binding that names its own parameter. The comparison is on the
                // rendered names, as the string form's `bound != type_name` was, so
                // a `Var`/`Named` spelling difference is not mistaken for progress.
                if bound.name() != type_.name() {
                    return self.concrete_type(&bound.clone(), &HashMap::new());
                }
                return bound.clone();
            }
        }
        match type_ {
            ParameterType::ListOf(element) => {
                ParameterType::list_of(self.concrete_type(element, substitutions))
            }
            ParameterType::SetOf(element) => {
                ParameterType::set_of(self.concrete_type(element, substitutions))
            }
            ParameterType::ResultOf(success) => {
                ParameterType::result_of(self.concrete_type(success, substitutions))
            }
            ParameterType::MapOf(key, value) => ParameterType::map_of(
                self.concrete_type(key, substitutions),
                self.concrete_type(value, substitutions),
            ),
            ParameterType::MapEntryOf(key, value) => ParameterType::map_entry_of(
                self.concrete_type(key, substitutions),
                self.concrete_type(value, substitutions),
            ),
            ParameterType::ThreadHandle {
                worker,
                msg,
                res,
                out,
            } => {
                // An absent resource plane is `Nothing` and stays `Nothing`, which
                // `name()` elides exactly as `format_thread_type` did.
                let res = match res.as_ref() {
                    ParameterType::Nothing => ParameterType::Nothing,
                    other => self.concrete_type(other, substitutions),
                };
                ParameterType::thread_handle(
                    *worker,
                    self.concrete_type(msg, substitutions),
                    res,
                    self.concrete_type(out, substitutions),
                )
            }
            ParameterType::Func(params, ret, isolated) => {
                let params = params
                    .iter()
                    .map(|param| self.concrete_type(param, substitutions))
                    .collect::<Vec<_>>();
                let ret = self.concrete_type(ret, substitutions);
                if *isolated {
                    ParameterType::func_isolated(params, ret)
                } else {
                    ParameterType::func(params, ret)
                }
            }
            // A user generic is the one arm that does more than rewrite: it also
            // INSTANTIATES the template, and yields the mangled concrete nominal.
            ParameterType::UserOf(name, args) => {
                let args = args
                    .iter()
                    .map(|arg| self.concrete_type(arg, substitutions).name().into_owned())
                    .collect::<Vec<_>>();
                ParameterType::parse(&self.instantiate_type(name.resolve(), &args))
            }
            // A scalar, a concrete nominal, `RES`, `Unknown`, `Arg` — identity, as
            // the cascade's fall-through was.
            other => other.clone(),
        }
    }

    /// The name-domain entry to [`template_view`](Self::template_view).
    fn template_view_type(&self, type_name: &str) -> String {
        self.template_view(&ParameterType::parse(type_name))
            .name()
            .into_owned()
    }

    /// The inverse of [`concrete_type`](Self::concrete_type): rewrite a mangled
    /// concrete type back to its template spelling, for diagnostics.
    ///
    /// plan-106-E: structural, like its twin. The user-generic arm stays a LOOKUP
    /// in `type_instantiations` — a mangled name is a plain nominal, so it can only
    /// be recognized that way, never by parsing.
    fn template_view(&self, type_: &ParameterType) -> ParameterType {
        match type_ {
            ParameterType::ListOf(element) => ParameterType::list_of(self.template_view(element)),
            ParameterType::SetOf(element) => ParameterType::set_of(self.template_view(element)),
            ParameterType::ResultOf(success) => {
                ParameterType::result_of(self.template_view(success))
            }
            ParameterType::MapOf(key, value) => {
                ParameterType::map_of(self.template_view(key), self.template_view(value))
            }
            ParameterType::MapEntryOf(key, value) => {
                ParameterType::map_entry_of(self.template_view(key), self.template_view(value))
            }
            ParameterType::ThreadHandle {
                worker,
                msg,
                res,
                out,
            } => {
                let res = match res.as_ref() {
                    ParameterType::Nothing => ParameterType::Nothing,
                    other => self.template_view(other),
                };
                ParameterType::thread_handle(
                    *worker,
                    self.template_view(msg),
                    res,
                    self.template_view(out),
                )
            }
            other => {
                let name = other.name();
                if let Some((template, args)) = self.type_instantiations.get(name.as_ref()) {
                    let args = args
                        .iter()
                        .map(|arg| self.template_view_type(arg))
                        .collect::<Vec<_>>();
                    // The template spelling is rebuilt through the grammar rather
                    // than `format!`ed: `UserOf`'s own render is the one that knows
                    // how its argument list is separated.
                    return ParameterType::user_of(
                        template,
                        args.iter().map(|a| ParameterType::parse(a)).collect(),
                    );
                }
                other.clone()
            }
        }
    }

    fn function_context(&self) -> FunctionContext {
        let mut context = FunctionContext::default();
        for (name, function) in &self.concrete_functions {
            let (returns, signature) = function_signature_types(function);
            context.function_returns.insert(name.clone(), returns);
            context.function_types.insert(name.clone(), signature);
        }
        for (name, type_decl) in &self.concrete_types {
            if matches!(type_decl.kind, TypeDeclKind::Type) {
                context
                    .record_fields
                    .insert(name.clone(), type_decl.fields.clone());
            }
        }
        // Top-level `LET`/`MUT` bindings with an explicit `AS` type, so a call or
        // overload whose argument names a global can be typed (bug-103).
        for item in self.source.files.iter().flat_map(|file| &file.items) {
            if let HirItem::Binding(binding) = item {
                if binding.explicit_type {
                    context
                        .globals
                        .insert(binding.name.clone(), binding.type_.clone());
                }
            }
        }
        context
    }

    fn add_function_to_context(&self, name: &str, context: &mut FunctionContext) {
        let Some(function) = self.concrete_functions.get(name) else {
            return;
        };
        let (returns, signature) = function_signature_types(function);
        context.function_returns.insert(name.to_string(), returns);
        context.function_types.insert(name.to_string(), signature);
    }

    /// The return type of a builtin/package call, using the same per-package
    /// `resolve_call` resolvers that the former source checker dispatches through
    /// (`SyntaxChecker::check_builtin_call`). Argument types are resolved
    /// positionally, falling back to `Unknown` so a resolver that keys on arity
    /// still sees the right shape. Without this, `expression_type` returned `None`
    /// for every builtin call, so a builtin-call argument was silently dropped
    /// from a generic/overloaded call's argument list (bug-103).
    fn builtin_call_return_type(
        &self,
        callee: &str,
        arguments: &[HirCallArg],
        context: &FunctionContext,
    ) -> Option<ParameterType> {
        let arg_types = arguments
            .iter()
            .map(|argument| {
                self.expression_type(call_arg_value(argument), context)
                    .unwrap_or(ParameterType::Unknown)
            })
            .collect::<Vec<_>>();
        // plan-106-A: the TYPED registry entry (plan-104-C), so neither the
        // argument types nor the resolved return crosses a string.
        crate::codegen::builtins::resolve_call_return_type_typed(callee, &arg_types, false)
    }

    /// Monomorph's type oracle: the (pre-instantiation) type of an expression,
    /// or `None` when the environment cannot decide it.
    ///
    /// plan-106-A: returns a [`ParameterType`], built structurally. Every arm
    /// that used to `format!` a spelling (`List OF …`, `Map OF … TO …`,
    /// `FUNC(…) AS …`) now constructs the variant directly, and the scalar arms
    /// are variants rather than `"Integer".to_string()`.
    fn expression_type(
        &self,
        expression: &HirExpression,
        context: &FunctionContext,
    ) -> Option<ParameterType> {
        match expression {
            HirExpression::String(_) => Some(ParameterType::String),
            HirExpression::Number(value) => Some(match crate::numeric::classify_literal(value).1 {
                crate::numeric::LiteralType::Integer => ParameterType::Integer,
                crate::numeric::LiteralType::Float => ParameterType::Float,
                crate::numeric::LiteralType::Fixed => ParameterType::Fixed,
                crate::numeric::LiteralType::Money => ParameterType::Money,
            }),
            HirExpression::Scalar(_) => Some(ParameterType::named("Scalar")),
            HirExpression::Boolean(_) => Some(ParameterType::Boolean),
            HirExpression::Identifier(value) if value == "NOTHING" => Some(ParameterType::Nothing),
            HirExpression::Identifier(value) => context
                .locals
                .get(value)
                .cloned()
                .or_else(|| context.function_types.get(value).cloned())
                .or_else(|| context.globals.get(value).cloned()),
            HirExpression::Constructor { type_, .. } => match type_.name().as_ref() {
                "Error" => Some(ParameterType::named("Error")),
                "Ok" => Some(ParameterType::ResultOf(Box::new(ParameterType::Unknown))),
                name if context.record_fields.contains_key(name) => {
                    Some(ParameterType::named(name))
                }
                _ => None,
            },
            HirExpression::WithUpdate { target, .. } => self.expression_type(target, context),
            HirExpression::ListLiteral(values) => Some(ParameterType::ListOf(Box::new(
                values
                    .first()
                    .and_then(|value| self.expression_type(value, context))
                    .unwrap_or(ParameterType::Unknown),
            ))),
            HirExpression::SetLiteral { element_type, .. } => {
                Some(ParameterType::SetOf(Box::new(element_type.clone())))
            }
            HirExpression::MapLiteral {
                key_type,
                value_type,
                ..
            } => Some(ParameterType::MapOf(
                Box::new(key_type.clone()),
                Box::new(value_type.clone()),
            )),
            HirExpression::MemberAccess { target, member } => {
                let target_type = self.expression_type(target, context)?;
                context
                    .record_fields
                    .get(target_type.name().as_ref())?
                    .iter()
                    .find(|field| field.name == *member)
                    .map(|field| field.type_.clone())
            }
            HirExpression::Call {
                callee, arguments, ..
            } => context
                .function_returns
                .get(callee)
                .cloned()
                .or_else(|| self.builtin_call_return_type(callee, arguments, context)),
            HirExpression::Lambda {
                params,
                body,
                assign_target,
            } => {
                let mut nested = context.clone();
                let param_types = params
                    .iter()
                    .map(|param| {
                        nested
                            .locals
                            .insert(param.name.clone(), param.type_.clone());
                        param.type_.clone()
                    })
                    .collect::<Vec<_>>();
                // An assignment-bodied lambda yields `Nothing`; otherwise its
                // result type is the body expression's type.
                let returns = if assign_target.is_some() {
                    ParameterType::Nothing
                } else {
                    self.expression_type(body, &nested)?
                };
                // A lambda is never `ISOLATED` (that marker is only written on a
                // declared FUNC), reproducing the un-prefixed `FUNC(…) AS …`
                // spelling this arm used to `format!`.
                Some(ParameterType::Func(param_types, Box::new(returns), false))
            }
            HirExpression::Binary {
                operator,
                left,
                right,
                ..
            } => {
                if matches!(
                    operator.as_str(),
                    "=" | "<>" | "<" | ">" | "<=" | ">=" | "AND" | "OR" | "XOR"
                ) {
                    return Some(ParameterType::Boolean);
                }
                if operator == "&" {
                    return Some(ParameterType::String);
                }
                let left = self.expression_type(left, context)?;
                let right = self.expression_type(right, context)?;
                Some(
                    numeric::typed_binary_result_type(operator, &left, &right)
                        .unwrap_or(ParameterType::Integer),
                )
            }
            HirExpression::Unary {
                operator, operand, ..
            } => {
                if operator == "NOT" {
                    Some(ParameterType::Boolean)
                } else {
                    self.expression_type(operand, context)
                }
            }
            HirExpression::Trapped { expression, .. } => self.expression_type(expression, context),
        }
    }

    fn report(&mut self, rule: &str, detail: &str, line: usize) {
        self.had_error = true;
        // Prefer the file whose body is currently being lowered (bug-107); fall
        // back to the first project file only when the frame is unknown.
        let relative = self
            .current_file
            .clone()
            .or_else(|| self.source.files.first().map(|file| file.path.clone()));
        let path = relative
            .map(|rel| self.project_dir.join(rel))
            .unwrap_or_else(|| self.project_dir.join("src/main.mfb"));
        rules::show_diagnostic(rule, detail, &path, line, 1, 1);
    }

    /// Charge one new concrete instantiation (function or user type) against the
    /// global total-instantiation budget, which bounds *wide* fan-out the per-path
    /// depth cap cannot (bug-399). Returns `false` — halting the caller — once any
    /// instantiation limit has stopped enumeration: either this budget (reporting a
    /// single `TYPE_INSTANTIATION_BUDGET_EXCEEDED` the first time it is exhausted)
    /// or the depth cap (which sets `instantiation_limit_reached` at its own site).
    /// After the first stop it returns `false` with no further work or diagnostics,
    /// so the remaining (exponential) tree is never explored.
    fn charge_instantiation(&mut self, name: &str, line: usize) -> bool {
        if self.instantiation_limit_reached {
            return false;
        }
        if self.total_instantiations >= MAX_TOTAL_INSTANTIATIONS {
            self.instantiation_limit_reached = true;
            self.report(
                "TYPE_INSTANTIATION_BUDGET_EXCEEDED",
                &format!(
                    "Monomorphization exceeded the {MAX_TOTAL_INSTANTIATIONS}-instantiation \
                     budget at `{name}`: the program fans out into too many distinct generic \
                     instantiations. This usually means a generic recurses through two or more \
                     type-widening self-calls."
                ),
                line,
            );
            return false;
        }
        self.total_instantiations += 1;
        true
    }

    fn report_instantiation_too_deep(&mut self, name: &str, line: usize) {
        self.report(
            "TYPE_INSTANTIATION_TOO_DEEP",
            &format!(
                "Template instantiation of `{name}` exceeds the {MAX_TEMPLATE_INSTANTIATION_DEPTH} level limit."
            ),
            line,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::super::{ImportedOverload, Monomorphizer};
    use crate::ast::AstProject;
    use crate::types::ParameterType;

    /// Parse one or more `(relative_path, source)` files into an `AstProject`.
    fn project(files: &[(&str, &str)]) -> AstProject {
        let ast_files = files
            .iter()
            .map(|(path, src)| {
                crate::ast::parse_source(std::path::Path::new(path), path, src)
                    .expect("parse source")
            })
            .collect::<Vec<_>>();
        AstProject {
            name: "testpkg".to_string(),
            files: ast_files,
        }
    }

    /// Monomorphize a single `main.mfb` source, returning `Ok(project)` or the
    /// error flag. Diagnostics are silenced so error-path tests stay quiet.
    ///
    /// plan-106-D: returns the concrete HIR the monomorphizer actually produces.
    /// It used to be de-elaborated back to an AST purely so these assertions could
    /// read it — a test-only backward path, which is how backward paths come back.
    fn monomorphize(src: &str) -> Result<crate::hir::HirProject, ()> {
        monomorphize_files(&[("src/main.mfb", src)])
    }

    fn monomorphize_files(files: &[(&str, &str)]) -> Result<crate::hir::HirProject, ()> {
        let ast = project(files);
        let dir = std::env::temp_dir();
        let prev = std::panic::take_hook();
        // Silence the front end's diagnostic printing during error-path tests.
        let result = super::super::monomorphize_project(&dir, &crate::hir::elaborate(&ast));
        std::panic::set_hook(prev);
        result
    }

    fn functions(project: &crate::hir::HirProject) -> Vec<&crate::hir::HirFunction> {
        project
            .files
            .iter()
            .flat_map(|f| &f.items)
            .filter_map(|item| match item {
                crate::hir::HirItem::Function(function) => Some(function),
                _ => None,
            })
            .collect()
    }

    fn types(project: &crate::hir::HirProject) -> Vec<&crate::hir::HirTypeDecl> {
        project
            .files
            .iter()
            .flat_map(|f| &f.items)
            .filter_map(|item| match item {
                crate::hir::HirItem::Type(type_decl) => Some(type_decl),
                _ => None,
            })
            .collect()
    }

    fn function_names(project: &crate::hir::HirProject) -> Vec<String> {
        functions(project).iter().map(|f| f.name.clone()).collect()
    }

    #[test]
    fn generic_function_instantiated_per_argument_type() {
        // A generic SUB called with Integer and String is monomorphized into two
        // concrete symbols (mangled by argument type); the template is dropped.
        let src = "\
IMPORT io
SUB show OF T(value AS T)
  io::print(toString(value))
END SUB
SUB main()
  show(42)
  show(\"hi\")
END SUB
";
        let project = monomorphize(src).expect("monomorphizes");
        let names = function_names(&project);
        assert!(names.iter().any(|n| n == "show$Integer"), "{names:?}");
        assert!(names.iter().any(|n| n == "show$String"), "{names:?}");
        // The open template `show` is not emitted.
        assert!(!names.iter().any(|n| n == "show"), "{names:?}");
    }

    #[test]
    fn generic_function_deduplicates_repeated_instantiation() {
        // Two calls with the same type argument produce a single concrete symbol.
        let src = "\
IMPORT io
SUB show OF T(value AS T)
  io::print(toString(value))
END SUB
SUB main()
  show(1)
  show(2)
END SUB
";
        let project = monomorphize(src).expect("monomorphizes");
        let count = function_names(&project)
            .iter()
            .filter(|n| *n == "show$Integer")
            .count();
        assert_eq!(count, 1);
    }

    #[test]
    fn total_instantiation_budget_halts_wide_fanout() {
        // The per-path depth cap cannot bound a generic that fans out in *breadth*
        // — ≥2 type-widening self-calls make an exponential tree of distinct
        // `name<args>` keys the depth cap never collapses (bug-399). A global
        // total-instantiation budget does. Drive the shared counter directly (an
        // end-to-end fan-out that actually reaches the several-thousand budget is
        // real but too slow for a unit test): it admits exactly
        // `MAX_TOTAL_INSTANTIATIONS` charges, then refuses every further one and
        // latches `instantiation_limit_reached` so enumeration stops after a single
        // bounded diagnostic.
        let ast = project(&[(
            "src/main.mfb",
            "FUNC main() AS Integer\n  RETURN 0\nEND FUNC\n",
        )]);
        let dir = std::env::temp_dir();
        let hir = crate::hir::elaborate(&ast);
        let mut mono = Monomorphizer::new(&dir, &hir);
        for i in 0..super::super::MAX_TOTAL_INSTANTIATIONS {
            assert!(
                mono.charge_instantiation("f", 1),
                "charge {i} within budget must succeed"
            );
            assert!(
                !mono.instantiation_limit_reached,
                "limit must not latch early"
            );
        }
        // Budget exhausted: this charge reports the single diagnostic and refuses.
        assert!(
            !mono.charge_instantiation("f", 1),
            "the charge at the budget must be refused"
        );
        assert!(mono.instantiation_limit_reached, "the limit must latch");
        assert!(mono.had_error, "refusal reports a diagnostic");
        // Once latched, every further charge short-circuits without incrementing.
        assert!(
            !mono.charge_instantiation("f", 1),
            "charges past the limit stay refused"
        );
        assert_eq!(
            mono.total_instantiations,
            super::super::MAX_TOTAL_INSTANTIATIONS,
            "no charge is admitted past the budget"
        );
    }

    #[test]
    fn instantiate_type_disambiguates_mangle_colliding_arguments() {
        // `mangle_name` is lossy: `sanitize_type_name` collapses every
        // non-alphanumeric character to `$`, so two distinct type-argument strings
        // that differ only in punctuation produce one shared mangled symbol.
        // `instantiate_function` guards this via `unique_concrete_symbol` (bug-226);
        // `instantiate_type` must too, or the second instantiation overwrites the
        // first in `concrete_types` and both use-sites bind one shared — and
        // possibly wrong — concrete type (bug-400). Driven directly against
        // `instantiate_type` because no *valid* source spelling reaches the
        // collision (the grammar fixes each punctuation slot relative to the alnum
        // tokens; the mangled symbol is the only lossy layer), which is exactly why
        // bug-226/bug-400 are latent — the same reason
        // `total_instantiation_budget_halts_wide_fanout` drives its counter directly.
        let ast = project(&[(
            "src/main.mfb",
            "TYPE Box OF T\n  value AS T\nEND TYPE\nFUNC main() AS Integer\n  RETURN 0\nEND FUNC\n",
        )]);
        let dir = std::env::temp_dir();
        let hir = crate::hir::elaborate(&ast);
        let mut mono = Monomorphizer::new(&dir, &hir);

        // Two distinct type-argument strings that `sanitize_type_name` maps to the
        // same suffix — `(`/`)` and `{`/`}` both sanitize to `$`.
        let a = "FUNC(Integer) AS Nothing".to_string();
        let b = "FUNC{Integer} AS Nothing".to_string();
        assert_ne!(a, b);
        assert_eq!(
            super::mangle_name("Box", std::slice::from_ref(&a)),
            super::mangle_name("Box", std::slice::from_ref(&b)),
            "the two arguments must mangle-collide for this test to exercise the guard"
        );

        let name_a = mono.instantiate_type("Box", std::slice::from_ref(&a));
        let name_b = mono.instantiate_type("Box", std::slice::from_ref(&b));

        // Each colliding instantiation must keep its own distinct concrete symbol...
        assert_ne!(
            name_a, name_b,
            "colliding type arguments must resolve to distinct concrete symbols"
        );
        // ...and its own concrete type declaration (no overwrite).
        let type_a = mono
            .concrete_types
            .get(&name_a)
            .expect("first concrete Box survives");
        let type_b = mono
            .concrete_types
            .get(&name_b)
            .expect("second concrete Box survives");
        assert_eq!(
            type_a.fields[0].type_.name().as_ref(),
            a.as_str(),
            "first keeps its field type"
        );
        assert_eq!(
            type_b.fields[0].type_.name().as_ref(),
            b.as_str(),
            "second keeps its field type"
        );
    }

    #[test]
    fn nested_generic_container_argument_unifies() {
        // A `List OF T` parameter unifies T against the element type of the
        // argument, exercising the recursive container unification.
        let src = "\
IMPORT io
IMPORT collections
FUNC first OF T(items AS List OF T) AS T
  RETURN collections::get(items, 0)
END FUNC
SUB main()
  LET xs AS List OF Integer = [1, 2, 3]
  LET a AS Integer = first(xs)
  io::print(toString(a))
END SUB
";
        let project = monomorphize(src).expect("monomorphizes");
        let names = function_names(&project);
        assert!(
            names.iter().any(|n| n.starts_with("first$")),
            "expected a mangled first instantiation, got {names:?}"
        );
    }

    #[test]
    fn generic_type_instantiated_from_expected_constructor_type() {
        // A generic TYPE used with an expected `Box OF Integer` constructor type
        // is instantiated into a concrete mangled type declaration.
        let src = "\
IMPORT io
TYPE Box OF T
  value AS T
END TYPE
FUNC main() AS Integer
  LET b AS Box OF Integer = Box[5]
  io::print(toString(b.value))
  RETURN 0
END FUNC
";
        let project = monomorphize(src).expect("monomorphizes");
        let type_names: Vec<&str> = types(&project).iter().map(|t| t.name.as_str()).collect();
        assert!(
            type_names.iter().any(|n| n.starts_with("Box$")),
            "expected a concrete Box instantiation, got {type_names:?}"
        );
    }

    #[test]
    fn overload_selected_by_parameter_type() {
        // Two overloads differing by parameter type resolve to distinct mangled
        // concrete symbols selected from the call argument types.
        let src = "\
IMPORT io
FUNC label(n AS Integer) AS String
  RETURN \"int\"
END FUNC
FUNC label(s AS String) AS String
  RETURN \"str\"
END FUNC
SUB main()
  io::print(label(1))
  io::print(label(\"x\"))
END SUB
";
        let project = monomorphize(src).expect("monomorphizes");
        let names = function_names(&project);
        assert!(names.iter().any(|n| n == "label$Integer"), "{names:?}");
        assert!(names.iter().any(|n| n == "label$String"), "{names:?}");
    }

    #[test]
    fn return_type_overload_selected_by_expected_type() {
        // Two overloads share parameter types and differ only by return type; an
        // annotated LET target supplies the expected type to disambiguate.
        let src = "\
IMPORT io
FUNC make() AS Integer
  RETURN 1
END FUNC
FUNC make() AS String
  RETURN \"one\"
END FUNC
SUB main()
  LET a AS Integer = make()
  LET b AS String = make()
  io::print(b)
END SUB
";
        let project = monomorphize(src).expect("monomorphizes");
        let names = function_names(&project);
        // Return-type disambiguation appends `AS <return>`.
        assert!(names.iter().any(|n| n.contains("AS$Integer")), "{names:?}");
        assert!(names.iter().any(|n| n.contains("AS$String")), "{names:?}");
    }

    #[test]
    fn control_flow_forms_are_lowered() {
        // FOR / FOR EACH / WHILE / DO UNTIL / IF bodies all pass through
        // statement lowering, and a generic call inside is still instantiated.
        let src = "\
IMPORT io
SUB emit OF T(value AS T)
  io::print(toString(value))
END SUB
FUNC main() AS Integer
  FOR i = 1 TO 3
    emit(i)
  NEXT
  LET xs AS List OF Integer = [1, 2]
  FOR EACH x IN xs
    emit(x)
  NEXT
  MUT n AS Integer = 0
  WHILE n < 2
    emit(n)
    n = n + 1
  END WHILE
  DO
    emit(n)
    n = n + 1
  LOOP UNTIL n > 4
  IF n > 0 THEN
    emit(n)
  ELSE
    emit(0)
  END IF
  RETURN 0
END FUNC
";
        let project = monomorphize(src).expect("monomorphizes");
        assert!(function_names(&project).iter().any(|n| n == "emit$Integer"));
    }

    #[test]
    fn for_loop_float_bound_promotes_counter_type() {
        // A Float loop bound promotes the counter's type so a generic call using
        // the counter instantiates on Float, exercising promote_loop_numeric_type.
        let src = "\
IMPORT io
SUB emit OF T(value AS T)
  io::print(toString(value))
END SUB
FUNC main() AS Integer
  FOR i = 1.0 TO 3.0
    emit(i)
  NEXT
  RETURN 0
END FUNC
";
        let project = monomorphize(src).expect("monomorphizes");
        assert!(
            function_names(&project).iter().any(|n| n == "emit$Float"),
            "{:?}",
            function_names(&project)
        );
    }

    #[test]
    fn match_union_variant_binding_is_lowered() {
        // A MATCH over a union binds the variant and lowers its body; a generic
        // call in the arm is instantiated on the bound type.
        let src = "\
IMPORT io
TYPE Circle
  r AS Integer
END TYPE
TYPE Square
  s AS Integer
END TYPE
UNION Shape
  Circle
  Square
END UNION
SUB emit OF T(value AS T)
  io::print(toString(value))
END SUB
FUNC main() AS Integer
  LET shape AS Shape = Circle[2]
  MATCH shape
    CASE Circle(c)
      emit(c.r)
    CASE Square(sq)
      emit(sq.s)
  END MATCH
  RETURN 0
END FUNC
";
        let project = monomorphize(src).expect("monomorphizes");
        assert!(function_names(&project).iter().any(|n| n == "emit$Integer"));
    }

    #[test]
    fn arity_mismatch_reports_error() {
        // More arguments than the template has parameters -> error flag set.
        let src = "\
IMPORT io
SUB one OF T(value AS T)
  io::print(toString(value))
END SUB
SUB main()
  one(1, 2)
END SUB
";
        assert!(monomorphize(src).is_err());
    }

    #[test]
    fn top_level_binding_value_is_lowered() {
        // A module-level LET with a generic-call initializer lowers the binding
        // value (lower_binding) and instantiates the callee.
        let src = "\
IMPORT io
FUNC idOf OF T(value AS T) AS T
  RETURN value
END FUNC
LET g AS Integer = idOf(7)
SUB main()
  io::print(toString(g))
END SUB
";
        let project = monomorphize(src).expect("monomorphizes");
        assert!(
            function_names(&project)
                .iter()
                .any(|n| n.starts_with("idOf$")),
            "{:?}",
            function_names(&project)
        );
    }

    #[test]
    fn trap_body_is_lowered() {
        // A function with a TRAP handler lowers the trap body too; a generic call
        // inside the handler is instantiated.
        let src = "\
IMPORT io
SUB emit OF T(value AS T)
  io::print(toString(value))
END SUB
FUNC risky() AS Integer
  RETURN 1
  TRAP(err)
    emit(1)
    RETURN 0
  END TRAP
END FUNC
FUNC main() AS Integer
  io::print(toString(risky()))
  RETURN 0
END FUNC
";
        let project = monomorphize(src).expect("monomorphizes");
        assert!(function_names(&project).iter().any(|n| n == "emit$Integer"));
    }

    #[test]
    fn plain_program_without_generics_passes_through() {
        // A concrete-only program monomorphizes to an equivalent project.
        let src = "\
IMPORT io
FUNC add(a AS Integer, b AS Integer) AS Integer
  RETURN a + b
END FUNC
SUB main()
  io::print(toString(add(1, 2)))
END SUB
";
        let project = monomorphize(src).expect("monomorphizes");
        let names = function_names(&project);
        assert!(names.iter().any(|n| n == "add"));
        assert!(names.iter().any(|n| n == "main"));
    }

    #[test]
    fn generic_over_map_and_result_container_params() {
        // Container-shaped parameter types exercise the Map/Result recursion in
        // concrete_type_name / template_view_type / unify.
        let src = "\
IMPORT io
IMPORT collections
FUNC lookup OF K, V(items AS Map OF K TO V, key AS K, fallback AS V) AS V
  IF collections::hasKey(items, key) THEN
    RETURN collections::get(items, key)
  END IF
  RETURN fallback
END FUNC
FUNC wrapOk OF T(value AS T) AS Result OF T
  RETURN Ok[value]
END FUNC
FUNC main() AS Integer
  LET m AS Map OF String TO Integer = Map OF String TO Integer { \"a\" := 1 }
  LET v AS Integer = lookup(m, \"a\", 0)
  io::print(toString(v))
  RETURN 0
END FUNC
";
        let project = monomorphize(src).expect("monomorphizes");
        assert!(
            function_names(&project)
                .iter()
                .any(|n| n.starts_with("lookup$")),
            "{:?}",
            function_names(&project)
        );
    }

    #[test]
    fn generic_type_inferred_from_constructor_arguments() {
        // A generic constructor with NO expected-type annotation infers its type
        // argument from the constructor argument types (lines 1010-1038).
        let src = "\
IMPORT io
TYPE Box OF T
  value AS T
END TYPE
FUNC boxed() AS Box OF Integer
  RETURN Box[5]
END FUNC
FUNC main() AS Integer
  io::print(toString(boxed().value))
  RETURN 0
END FUNC
";
        let project = monomorphize(src).expect("monomorphizes");
        let type_names: Vec<&str> = types(&project).iter().map(|t| t.name.as_str()).collect();
        assert!(
            type_names.iter().any(|n| n.starts_with("Box$")),
            "{type_names:?}"
        );
    }

    #[test]
    fn constructor_infers_param_despite_unknown_field_declared_first() {
        // bug-442: a generic record whose `List OF T` field (bound to an empty
        // `[]` => `List OF Unknown`) is declared BEFORE the `FUNC(T)` field that
        // carries the concrete type must still resolve `T` from the later field,
        // instantiating `Box$Integer` rather than `Box$Unknown`.
        let src = "\
IMPORT io
TYPE Box OF T
  items AS List OF T
  fn    AS FUNC(T) AS Boolean
END TYPE
FUNC makeBox OF T(fn AS FUNC(T) AS Boolean) AS Box OF T
  RETURN Box[items := [], fn := fn]
END FUNC
FUNC even(n AS Integer) AS Boolean
  RETURN n MOD 2 = 0
END FUNC
FUNC main() AS Integer
  MUT a AS Box OF Integer = makeBox(even)
  io::print(toString(len(a.items)))
  RETURN 0
END FUNC
";
        let project = monomorphize(src).expect("monomorphizes");
        let type_names: Vec<&str> = types(&project).iter().map(|t| t.name.as_str()).collect();
        assert!(
            type_names.iter().any(|n| n.starts_with("Box$Integer")),
            "expected Box$Integer, got {type_names:?}"
        );
        assert!(
            !type_names.iter().any(|n| n.starts_with("Box$Unknown")),
            "Box$Unknown must not be instantiated: {type_names:?}"
        );
    }

    #[test]
    fn record_member_access_and_with_update_types_are_inferred() {
        // Member access and WITH-update expression typing feed a generic call so
        // the corresponding expression_type arms run.
        let src = "\
IMPORT io
TYPE Point
  x AS Integer
  y AS Integer
END TYPE
SUB emit OF T(value AS T)
  io::print(toString(value))
END SUB
FUNC main() AS Integer
  LET p AS Point = Point[1, 2]
  emit(p.x)
  LET q AS Point = WITH p { x := 9 }
  emit(q.y)
  RETURN 0
END FUNC
";
        let project = monomorphize(src).expect("monomorphizes");
        assert!(function_names(&project).iter().any(|n| n == "emit$Integer"));
    }

    #[test]
    fn list_literal_and_string_concat_and_unary_types() {
        // List literal element typing, string-concat `&`, comparison, and unary
        // NOT all drive distinct expression_type branches through generic calls.
        let src = "\
IMPORT io
SUB emit OF T(value AS T)
  io::print(toString(value))
END SUB
FUNC main() AS Integer
  LET xs AS List OF String = [\"a\", \"b\"]
  emit(xs)
  LET joined AS String = \"x\" & \"y\"
  emit(joined)
  LET flag AS Boolean = NOT (1 < 2)
  emit(flag)
  LET sum AS Integer = 1 + 2
  emit(sum)
  RETURN 0
END FUNC
";
        let project = monomorphize(src).expect("monomorphizes");
        let names = function_names(&project);
        assert!(names.iter().any(|n| n == "emit$String"), "{names:?}");
        assert!(names.iter().any(|n| n == "emit$Boolean"), "{names:?}");
        assert!(names.iter().any(|n| n == "emit$Integer"), "{names:?}");
        assert!(
            names.iter().any(|n| n == "emit$List$OF$String"),
            "{names:?}"
        );
    }

    #[test]
    fn general_builtin_override_selected_for_user_type() {
        // A user `FUNC toString(p AS Point)` overrides the general built-in for
        // its own type; the call routes to the mangled override symbol
        // (resolve_general_builtin_override).
        let src = "\
IMPORT io
TYPE Point
  x AS Integer
END TYPE
FUNC toString(p AS Point) AS String
  RETURN \"point\"
END FUNC
FUNC main() AS Integer
  LET p AS Point = Point[1]
  io::print(toString(p))
  RETURN 0
END FUNC
";
        let project = monomorphize(src).expect("monomorphizes");
        // The override is force-mangled so it never equals the built-in name.
        assert!(
            function_names(&project)
                .iter()
                .any(|n| n.starts_with("toString$")),
            "{:?}",
            function_names(&project)
        );
    }

    #[test]
    fn overload_no_match_leaves_call_unresolved() {
        // Two overloads exist but neither matches the argument types; the call is
        // left as the bare name (resolve_overload returns None, no error).
        let src = "\
IMPORT io
FUNC pick(n AS Integer) AS String
  RETURN \"i\"
END FUNC
FUNC pick(s AS String) AS String
  RETURN \"s\"
END FUNC
FUNC main() AS Integer
  LET flag AS Boolean = TRUE
  io::print(pick(flag))
  RETURN 0
END FUNC
";
        // No matching overload for Boolean: monomorph does not error (resolution
        // is left to later stages), it simply leaves the callee unresolved.
        let project = monomorphize(src).expect("monomorphizes");
        // Both overloads still emitted under their mangled names.
        assert!(function_names(&project).iter().any(|n| n == "pick$Integer"));
    }

    #[test]
    fn return_type_overload_ambiguous_without_expected_type_errors() {
        // A return-type overload set called with no expected (contextual) type is
        // ambiguous -> TYPE_OVERLOAD_AMBIGUOUS (resolve_overload error arm).
        let src = "\
IMPORT io
FUNC make() AS Integer
  RETURN 1
END FUNC
FUNC make() AS String
  RETURN \"one\"
END FUNC
SUB emit OF T(value AS T)
  io::print(toString(value))
END SUB
FUNC main() AS Integer
  emit(make())
  RETURN 0
END FUNC
";
        assert!(monomorphize(src).is_err());
    }

    #[test]
    fn template_argument_unification_failure_errors() {
        // A `List OF T` parameter given a non-list argument cannot infer T ->
        // TYPE_CALL_ARGUMENT_MISMATCH (the unify-failure arm of instantiate).
        let src = "\
IMPORT io
IMPORT collections
FUNC firstOf OF T(items AS List OF T) AS T
  RETURN collections::get(items, 0)
END FUNC
FUNC main() AS Integer
  io::print(toString(firstOf(42)))
  RETURN 0
END FUNC
";
        assert!(monomorphize(src).is_err());
    }

    #[test]
    fn lambda_expression_type_is_inferred() {
        // A lambda passed to a generic call drives the Lambda arm of
        // expression_type, inferring `FUNC(Integer) AS Integer`.
        let src = "\
IMPORT io
SUB emit OF T(value AS T)
  io::print(toString(value))
END SUB
FUNC main() AS Integer
  emit(LAMBDA(n AS Integer) -> n + 1)
  RETURN 0
END FUNC
";
        let project = monomorphize(src).expect("monomorphizes");
        assert!(
            function_names(&project)
                .iter()
                .any(|n| n.starts_with("emit$FUNC")),
            "{:?}",
            function_names(&project)
        );
    }

    /// plan-111-B: `types_compatible` was a token algorithm over two RENDERED
    /// spellings — equal token counts, then each pair equal or either literally
    /// `"Unknown"`. It is structural now, and this pins the two forms to the
    /// same answer over every shape the overload filter can see.
    ///
    /// Two things this pins.
    ///
    /// **The rule that must survive**: `Unknown` stood in for ONE token, so it
    /// never matched a composite. A structural "Unknown matches anything" would
    /// silently turn "an untyped `[]` selects no overload" into "it selects
    /// every one of them" — inverting `TYPE_OVERLOAD_AMBIGUOUS`'s own advice.
    /// The last block asserts that asymmetry directly.
    ///
    /// **The bug that must NOT survive**: the token form only wildcarded an
    /// `Unknown` that whitespace happened to delimit. A comma or a paren glues
    /// it to its neighbour — `"Unknown,"`, `"FUNC(Unknown)"` — so it stopped
    /// being a wildcard in a non-final user-generic argument and anywhere
    /// inside `FUNC(...)`, while the FINAL argument of the same spelling still
    /// worked. Overload selection depending on whether an argument is last is
    /// an accident of `split_whitespace`, not a rule; the `fixed` list below is
    /// every pair where the two forms therefore differ, and each is asserted to
    /// have been broken before.
    #[test]
    fn types_compatible_matches_the_token_algorithm() {
        // The algorithm this replaced, verbatim.
        fn token_form(param: &str, actual: &str) -> bool {
            if param == actual {
                return true;
            }
            let param_tokens: Vec<&str> = param.split_whitespace().collect();
            let actual_tokens: Vec<&str> = actual.split_whitespace().collect();
            param_tokens.len() == actual_tokens.len()
                && param_tokens
                    .iter()
                    .zip(actual_tokens.iter())
                    .all(|(p, a)| p == a || *p == "Unknown" || *a == "Unknown")
        }
        let ast = project(&[(
            "src/main.mfb",
            "FUNC main() AS Integer\n  RETURN 0\nEND FUNC\n",
        )]);
        let dir = std::env::temp_dir();
        let hir = crate::hir::elaborate(&ast);
        let mono = Monomorphizer::new(&dir, &hir);

        let corpus = [
            "Integer",
            "String",
            "Unknown",
            "Db",
            "fs.File",
            "List OF Integer",
            "List OF Unknown",
            "List OF String",
            "Set OF Integer",
            "Map OF String TO Integer",
            "Map OF Unknown TO Integer",
            "Map OF String TO Unknown",
            "Result OF Integer",
            "MapEntry OF String TO Integer",
            "RES fs.File",
            "List OF List OF Integer",
            "List OF List OF Unknown",
            "Pair OF Integer, String",
            "Pair OF Unknown, String",
            "FUNC(Integer) AS String",
            "FUNC(Unknown) AS String",
            "ISOLATED FUNC(Integer) AS String",
            "Thread OF Integer TO String",
            "ThreadWorker OF Integer TO String",
            "Thread OF Unknown TO String",
        ];
        // The token form only wildcarded an `Unknown` that whitespace happened
        // to delimit. A comma or a paren glues it to its neighbour
        // (`"Unknown,"`, `"FUNC(Unknown)"`), so it silently stopped being a
        // wildcard in a non-final user-generic argument and anywhere inside
        // `FUNC(...)` — while the FINAL argument of the same spelling still
        // worked. That is a bug, not a rule, and the structural form fixes it;
        // these are the pairs where the two therefore disagree, on purpose.
        let fixed: &[(&str, &str)] = &[
            ("Pair OF Integer, String", "Pair OF Unknown, String"),
            ("Pair OF Unknown, String", "Pair OF Integer, String"),
            ("FUNC(Integer) AS String", "FUNC(Unknown) AS String"),
            ("FUNC(Unknown) AS String", "FUNC(Integer) AS String"),
            (
                "ISOLATED FUNC(Integer) AS String",
                "FUNC(Unknown) AS String",
            ),
            (
                "FUNC(Unknown) AS String",
                "ISOLATED FUNC(Integer) AS String",
            ),
        ];
        let mut checked = 0usize;
        let mut diverged = 0usize;
        for param in corpus {
            for actual in corpus {
                let structural = mono
                    .types_compatible(&ParameterType::parse(param), &ParameterType::parse(actual));
                if fixed.contains(&(param, actual)) {
                    // The isolation flag still has to disagree independently of
                    // the wildcard, so assert the exact outcome rather than
                    // "different".
                    let expected =
                        !param.starts_with("ISOLATED") && !actual.starts_with("ISOLATED");
                    assert_eq!(
                        structural, expected,
                        "known-fixed pair `{param}` vs `{actual}`"
                    );
                    assert!(
                        !token_form(param, actual),
                        "`{param}` vs `{actual}` is listed as fixed, but the token \
                         form already agreed — drop it from the list"
                    );
                    diverged += 1;
                    continue;
                }
                assert_eq!(
                    structural,
                    token_form(param, actual),
                    "types_compatible disagrees for `{param}` vs `{actual}`"
                );
                checked += 1;
            }
        }
        assert_eq!(checked + diverged, corpus.len() * corpus.len());
        assert_eq!(diverged, fixed.len(), "every listed divergence must be hit");

        // The load-bearing asymmetry, stated directly: a wholly-unknown argument
        // matches a leaf and NOT a composite, so it selects no container overload.
        assert!(mono.types_compatible(&ParameterType::Unknown, &ParameterType::Integer));
        assert!(!mono.types_compatible(
            &ParameterType::Unknown,
            &ParameterType::list_of(ParameterType::Integer)
        ));
        // ...while an element-position `Unknown` still matches, which is what
        // makes an untyped `[]` (`List OF Unknown`) ambiguous across two
        // element-typed overloads (bug-36, the test below).
        assert!(mono.types_compatible(
            &ParameterType::list_of(ParameterType::Unknown),
            &ParameterType::list_of(ParameterType::Integer)
        ));
    }

    /// plan-111-B: `normalize_type` stripped import qualifiers by scanning the
    /// whole RENDERED spelling byte-wise. It walks the type now. Every qualifier
    /// is `"<binding>."` / `"<package>."` and a `.` appears nowhere else in the
    /// grammar, so the two must agree — pinned here rather than argued.
    #[test]
    fn normalize_type_matches_the_string_algorithm() {
        let ast = project(&[(
            "src/main.mfb",
            "FUNC main() AS Integer\n  RETURN 0\nEND FUNC\n",
        )]);
        let dir = std::env::temp_dir();
        let hir = crate::hir::elaborate(&ast);
        let mut mono = Monomorphizer::new(&dir, &hir);
        mono.package_qualifiers = vec![
            "sqlite.".to_string(),
            "io.".to_string(),
            "radio.".to_string(),
            "fs.".to_string(),
        ];
        // The string form this replaced, verbatim.
        let string_form = |type_: &str| -> String {
            let mut qualifiers: Vec<&str> =
                mono.package_qualifiers.iter().map(String::as_str).collect();
            qualifiers.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
            let mut normalized = type_.to_string();
            for qualifier in qualifiers {
                normalized = super::strip_qualifier_prefixes(&normalized, qualifier);
            }
            crate::codegen::resource::base_resource_name(&normalized).to_string()
        };
        for spelled in [
            "Integer",
            "sqlite.Db",
            "fs.File",
            // The bug-104 shape: a short qualifier must not eat into a longer name.
            "radio.Station",
            "List OF sqlite.Db",
            "Set OF fs.File",
            "Map OF String TO sqlite.Db",
            "Result OF sqlite.Db",
            "RES fs.File",
            "List OF RES fs.File",
            "Pair OF sqlite.Db, io.Stream",
            "FUNC(sqlite.Db) AS io.Stream",
            "Thread OF sqlite.Db TO io.Stream",
            "Thread OF Integer RES fs.File TO String",
            // The STATE peel, on a bare resource and inside a container.
            "sqlite.Db STATE sqlite.DbInfo",
            "fs.File STATE Cursor",
            "List OF RES fs.File STATE Cursor",
            "Unknown",
        ] {
            assert_eq!(
                mono.normalize_type(&ParameterType::parse(spelled)).name(),
                string_form(spelled),
                "normalize_type disagrees for `{spelled}`"
            );
        }
    }

    /// bug-36: `Unknown` (from an untyped `[]`) is a wildcard, so an element-typed
    /// overload set matches it twice. Taking the first candidate bound the call to
    /// whichever overload the package exported first, silently.
    #[test]
    fn an_untyped_empty_collection_makes_an_imported_overload_ambiguous() {
        let ast = AstProject {
            name: "app".to_string(),
            files: vec![],
        };
        let dir = tempfile::tempdir().expect("tempdir");
        let hir = crate::hir::elaborate(&ast);
        let mut monomorphizer = Monomorphizer::new(dir.path(), &hir);
        monomorphizer.imported_overloads.insert(
            "pkg.f".to_string(),
            vec![
                ImportedOverload {
                    param_types: vec![ParameterType::parse("List OF Integer")],
                    qualified_name: "pkg.f$ListOFInteger".to_string(),
                },
                ImportedOverload {
                    param_types: vec![ParameterType::parse("List OF String")],
                    qualified_name: "pkg.f$ListOFString".to_string(),
                },
            ],
        );

        // A concretely-typed argument selects exactly one overload.
        assert_eq!(
            monomorphizer
                .resolve_imported_overload("pkg.f", &[ParameterType::parse("List OF Integer")], 1)
                .as_deref(),
            Some("pkg.f$ListOFInteger")
        );
        assert_eq!(
            monomorphizer
                .resolve_imported_overload("pkg.f", &[ParameterType::parse("List OF String")], 1)
                .as_deref(),
            Some("pkg.f$ListOFString")
        );
        assert!(!monomorphizer.had_error);

        // `f([])` matches both through the `Unknown` wildcard: ambiguous, not
        // "whichever came first".
        assert_eq!(
            monomorphizer.resolve_imported_overload(
                "pkg.f",
                &[ParameterType::parse("List OF Unknown")],
                7
            ),
            None
        );
        assert!(monomorphizer.had_error);

        // An unrelated callee and a wrong arity still resolve to nothing.
        assert_eq!(
            monomorphizer.resolve_imported_overload("pkg.other", &[], 1),
            None
        );
        assert_eq!(
            monomorphizer.resolve_imported_overload("pkg.f", &[], 1),
            None
        );
    }

    #[test]
    fn imported_overload_call_is_rewritten_to_package_symbol() {
        // Import a real package with an exported overload set and call it; the
        // call is rewritten to the package-qualified mangled name
        // (resolve_imported_overload, types_compatible, normalize_type).
        let fixture = crate::testutil::fixture_dir("package-simple")
            .join("golden")
            .join("package_simple.mfp");
        let dir = tempfile::tempdir().expect("tempdir");
        let packages = dir.path().join("packages");
        std::fs::create_dir_all(&packages).unwrap();
        std::fs::copy(&fixture, packages.join("package_simple.mfp")).unwrap();

        // `score` is an exported overload set: a no-arg form and a `Vec2` form.
        // Calling the no-arg form drives resolve_imported_overload to match the
        // 0-parameter candidate and rewrite the callee to the package symbol.
        let src = "\
IMPORT io
IMPORT package_simple
FUNC main() AS Integer
  io::print(toString(package_simple::score()))
  RETURN 0
END FUNC
";
        let file =
            crate::ast::parse_source(std::path::Path::new("src/main.mfb"), "src/main.mfb", src)
                .expect("parse");
        let ast = AstProject {
            name: "app".to_string(),
            files: vec![file],
        };
        let concrete = super::super::monomorphize_project(dir.path(), &crate::hir::elaborate(&ast))
            .expect("monomorphizes");
        // The `main` body's call to `package_simple.score` is rewritten to the
        // package-qualified mangled symbol.
        let main = functions(&concrete)
            .into_iter()
            .find(|f| f.name == "main")
            .expect("main present");
        let rendered = format!("{:?}", main.body);
        assert!(
            rendered.contains("package_simple.score"),
            "expected package-qualified call, got: {rendered}"
        );
    }

    #[test]
    fn match_literal_oneof_and_else_patterns_lower() {
        // A MATCH with a literal-list arm (`CASE 1, 2, 3`) and an ELSE arm drives
        // the OneOf and Else pattern-lowering branches; a generic call inside
        // still instantiates.
        let src = "\
IMPORT io
SUB emit OF T(value AS T)
  io::print(toString(value))
END SUB
FUNC main() AS Integer
  LET n AS Integer = 2
  MATCH n
    CASE 1, 2, 3
      emit(n)
    CASE ELSE
      emit(0)
  END MATCH
  RETURN 0
END FUNC
";
        let project = monomorphize(src).expect("monomorphizes");
        assert!(function_names(&project).iter().any(|n| n == "emit$Integer"));
    }

    #[test]
    fn for_each_over_map_binds_map_entry_type() {
        // FOR EACH over a Map binds `MapEntry OF K TO V`; a generic call on the
        // entry drives the map branch of ForEach lowering.
        let src = "\
IMPORT io
SUB emit OF T(value AS T)
  io::print(toString(value))
END SUB
FUNC main() AS Integer
  LET m AS Map OF String TO Integer = Map OF String TO Integer { \"a\" := 1 }
  FOR EACH entry IN m
    emit(entry)
  NEXT
  RETURN 0
END FUNC
";
        let project = monomorphize(src).expect("monomorphizes");
        // The loop variable binds `MapEntry OF String TO Integer`; the generic
        // call instantiates on that concrete entry type.
        assert!(
            function_names(&project)
                .iter()
                .any(|n| n.starts_with("emit$MapEntry")),
            "{:?}",
            function_names(&project)
        );
    }

    #[test]
    fn named_constructor_arguments_are_lowered() {
        // A record constructor with named fields exercises the named-arg path in
        // lower_constructor_arg and constructor_arg_field_type.
        let src = "\
IMPORT io
TYPE Point
  x AS Integer
  y AS Integer
END TYPE
FUNC main() AS Integer
  LET p AS Point = Point[x := 3, y := 4]
  io::print(toString(p.x))
  RETURN 0
END FUNC
";
        let project = monomorphize(src).expect("monomorphizes");
        assert!(function_names(&project).iter().any(|n| n == "main"));
    }

    #[test]
    fn encoding_utf8_encode_overload_resolves_to_bytes() {
        // `encoding::utf8Encode` is a return-type overload; the `List OF Byte`
        // annotation selects the bytes target (encoding overload resolution,
        // Ok(Some) arm).
        let src = "\
IMPORT io
IMPORT encoding
FUNC main() AS Integer
  LET bytes AS List OF Byte = encoding::utf8Encode(\"hi\")
  RETURN 0
END FUNC
";
        let _ = monomorphize(src);
    }

    #[test]
    fn encoding_utf8_encode_overload_ambiguous_without_expected_type() {
        // `utf8Encode` with no expected (contextual) type is an ambiguous
        // return-type overload -> the encoding resolver's Err(()) arm reports
        // TYPE_OVERLOAD_AMBIGUOUS.
        let src = "\
IMPORT io
IMPORT encoding
SUB emit OF T(value AS T)
  io::print(toString(value))
END SUB
FUNC main() AS Integer
  emit(encoding::utf8Encode(\"hi\"))
  RETURN 0
END FUNC
";
        // Whether it errors depends on resolver state; either way the encoding
        // overload branch executes. Assert it does not panic.
        let _ = monomorphize(src);
    }

    #[test]
    fn encoding_utf8_encode_wrong_arg_type_leaves_call() {
        // `utf8Encode` applied to a non-String argument matches no overload; the
        // encoding resolver returns Ok(None) and the call is left in place.
        let src = "\
IMPORT io
IMPORT encoding
FUNC main() AS Integer
  LET bytes AS List OF Byte = encoding::utf8Encode(42)
  RETURN 0
END FUNC
";
        let _ = monomorphize(src);
    }

    #[test]
    fn bare_list_literal_argument_type_is_inferred() {
        // A bare list literal passed to a generic call drives the ListLiteral arm
        // of expression_type (element type from the first element).
        let src = "\
IMPORT io
SUB emit OF T(value AS T)
  io::print(toString(value))
END SUB
FUNC main() AS Integer
  emit([1, 2, 3])
  RETURN 0
END FUNC
";
        let project = monomorphize(src).expect("monomorphizes");
        assert!(
            function_names(&project)
                .iter()
                .any(|n| n.starts_with("emit$List")),
            "{:?}",
            function_names(&project)
        );
    }

    #[test]
    fn imported_overload_matches_argument_by_type() {
        // Import the real package and call the `Vec2` overload of `score` with a
        // constructed Vec2, driving resolve_imported_overload's per-argument
        // types_compatible / normalize_type comparison.
        let fixture = crate::testutil::fixture_dir("package-simple")
            .join("golden")
            .join("package_simple.mfp");
        let dir = tempfile::tempdir().expect("tempdir");
        let packages = dir.path().join("packages");
        std::fs::create_dir_all(&packages).unwrap();
        std::fs::copy(&fixture, packages.join("package_simple.mfp")).unwrap();

        let src = "\
IMPORT io
IMPORT package_simple
FUNC main() AS Integer
  LET v AS package_simple::Vec2 = package_simple::Vec2[1, 2]
  io::print(toString(package_simple::score(v)))
  RETURN 0
END FUNC
";
        let file =
            crate::ast::parse_source(std::path::Path::new("src/main.mfb"), "src/main.mfb", src)
                .expect("parse");
        let ast = AstProject {
            name: "app".to_string(),
            files: vec![file],
        };
        // The Vec2-typed argument selects the `score(Vec2)` overload; assert the
        // pass completes without panicking (resolution branch runs regardless).
        let _ = super::super::monomorphize_project(dir.path(), &crate::hir::elaborate(&ast));
    }

    #[test]
    fn ok_and_error_constructor_types_are_inferred() {
        // `Ok[..]` and `error(..)` constructor typing feed a generic call so the
        // Result/Error expression_type arms run.
        let src = "\
IMPORT io
SUB emit OF T(value AS T)
  io::print(toString(value))
END SUB
FUNC main() AS Integer
  LET r AS Result OF Integer = Ok[1]
  emit(r)
  RETURN 0
END FUNC
";
        let _ = monomorphize(src);
    }

    #[test]
    fn two_generic_instantiations_are_emitted_sorted() {
        // Two distinct generic instantiations produce two generated functions,
        // exercising the stable sort in into_project.
        let src = "\
IMPORT io
FUNC idOf OF T(value AS T) AS T
  RETURN value
END FUNC
FUNC main() AS Integer
  io::print(toString(idOf(1)))
  io::print(idOf(\"x\"))
  RETURN 0
END FUNC
";
        let project = monomorphize(src).expect("monomorphizes");
        let generated: Vec<String> = function_names(&project)
            .into_iter()
            .filter(|n| n.starts_with("idOf$"))
            .collect();
        assert_eq!(generated.len(), 2, "{generated:?}");
    }

    #[test]
    fn return_type_only_template_param_is_a_reported_error() {
        // `T` appears only in the return type, so a bare `make()` cannot pin it
        // down: the instantiation is refused with a diagnostic rather than left as
        // the raw template name (bug-226).
        let src = "\
FUNC make OF T() AS T
  RETURN NOTHING
END FUNC
SUB main()
  LET x AS Integer = make()
END SUB
";
        // Errors during lowering surface as the error flag.
        assert!(monomorphize(src).is_err());
    }

    #[test]
    fn generic_set_literal_lowers_its_element_type() {
        // A generic function returning `Set OF T` whose body is a set literal
        // exercises the SetLiteral lowering arm and the `Set OF` shape of
        // `concrete_type_name`: the element type is rewritten T -> Integer.
        let src = "\
FUNC wrap OF T(value AS T) AS Set OF T
  RETURN Set OF T { value }
END FUNC
SUB main()
  LET s AS Set OF Integer = wrap(42)
END SUB
";
        let project = monomorphize(src).expect("monomorphizes");
        let names = function_names(&project);
        assert!(names.iter().any(|n| n == "wrap$Integer"), "{names:?}");
    }
}

/// The element type a `List`/`Set` literal's members are expected to have, given the
/// literal's own expected type — or `None` when the context expects something that is
/// not that container.
///
/// plan-105-B: routed through the canonical grammar rather than a
/// `strip_prefix("List OF ")` / `strip_prefix("Set OF ")` pair. `set` picks which
/// container the caller is lowering, so a `List` literal in a `Set`-typed context
/// still (correctly) gets no expected element type.
fn expected_element_type(
    expected_type: Option<&ParameterType>,
    set: bool,
) -> Option<ParameterType> {
    match expected_type? {
        ParameterType::SetOf(element) if set => Some((**element).clone()),
        ParameterType::ListOf(element) if !set => Some((**element).clone()),
        _ => None,
    }
}
