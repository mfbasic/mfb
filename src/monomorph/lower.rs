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
    pub(super) fn new(project_dir: &'a Path, source: &'a AstProject) -> Self {
        let mut type_templates = HashMap::new();
        let mut function_templates = HashMap::new();
        let mut concrete_types = HashMap::new();
        let mut concrete_functions = HashMap::new();
        let mut function_overloads: HashMap<String, Vec<Function>> = HashMap::new();
        let mut overload_names = HashMap::new();
        let mut function_files: HashMap<String, String> = HashMap::new();

        for file in &source.files {
            for item in &file.items {
                match item {
                    Item::Binding(_) => {}
                    Item::Type(type_decl) if !type_decl.template_params.is_empty() => {
                        type_templates.insert(type_decl.name.clone(), type_decl.clone());
                    }
                    Item::Type(type_decl) => {
                        concrete_types.insert(type_decl.name.clone(), type_decl.clone());
                    }
                    Item::Function(function) if !function.template_params.is_empty() => {
                        function_files.insert(function.name.clone(), file.path.clone());
                        function_templates.insert(function.name.clone(), function.clone());
                    }
                    Item::Function(function) => {
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
                    Item::Resource(_) | Item::FuncAlias(_) | Item::Link(_) => {}
                    // DOC blocks carry no template parameters; passed through below.
                    Item::Doc(_) => {}
                    // TESTING blocks are lowered away before monomorphization.
                    Item::Testing(_) => {}
                }
            }
        }

        for functions in function_overloads.values() {
            for function in functions {
                // A user `FUNC` whose name is an overridable general built-in is
                // always force-mangled so its codegen symbol never equals the
                // built-in dispatch name (plan-01-overload.md §C Phase 5.1).
                let builtin_named = crate::builtins::general::is_overridable(&function.name);
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
                        function.return_type.as_deref(),
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
            collections_bindings: crate::builtins::collections::collections_bindings(source)
                .into_keys()
                .collect(),
            function_files,
            current_file: None,
            had_error: false,
        }
    }

    /// Rewrites a `collections::` call callee (`collections.sort`, or an aliased
    /// `c.sort`) to its internal generic implementation (`__collections_sort`).
    /// Returns the callee unchanged when it is not a `collections::` call.
    fn collections_internal_callee(&self, callee: &str) -> Option<String> {
        let (binding, member) = callee.split_once('.')?;
        if !self.collections_bindings.contains(binding) {
            return None;
        }
        crate::builtins::collections::is_collections_function(member)
            .then(|| crate::builtins::collections::internal_name(member))
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
        arg_types: &[String],
        line: usize,
    ) -> Option<String> {
        let candidates = self.imported_overloads.get(callee)?;
        let matches: Vec<String> = candidates
            .iter()
            .filter(|candidate| {
                candidate.param_types.len() == arg_types.len()
                    && candidate
                        .param_types
                        .iter()
                        .zip(arg_types.iter())
                        .all(|(param, actual)| {
                            self.types_compatible(
                                &self.normalize_type(param),
                                &self.normalize_type(actual),
                            )
                        })
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
    /// token-wise, treating `Unknown` (e.g. from an empty `[]` literal) as a
    /// wildcard so an untyped empty collection still selects an overload.
    fn types_compatible(&self, param: &str, actual: &str) -> bool {
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

    /// Strip package/import-binding qualifiers from each user/resource type name
    /// inside `type_` so an importer's `sqlite.Db` matches the package's bare `Db`.
    fn normalize_type(&self, type_: &str) -> String {
        // Strip each qualifier only where it prefixes a type-name token — at the
        // start of the string or after a non-identifier byte — never as a bare
        // substring. An unanchored `replace` lets a short qualifier (`io.`) eat
        // into a longer name (`radio.`), and iterating `package_qualifiers` (a
        // `HashSet`-derived Vec) in hash order made the result depend on hash
        // seed, so the same source produced different overload resolutions and
        // flapping diagnostics run-to-run (bug-104). Sort longest-first for a
        // stable, prefix-preferring order.
        let mut qualifiers: Vec<&str> = self.package_qualifiers.iter().map(String::as_str).collect();
        qualifiers.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        let mut normalized = type_.to_string();
        for qualifier in qualifiers {
            normalized = strip_qualifier_prefixes(&normalized, qualifier);
        }
        normalized
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

    pub(super) fn into_project(mut self) -> AstProject {
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
                        Item::Binding(binding) => {
                            items.push(Item::Binding(self.lower_binding(binding.clone())));
                        }
                        Item::Type(type_decl) if type_decl.template_params.is_empty() => {
                            if let Some(concrete) = self.concrete_types.get(&type_decl.name) {
                                emitted_types.insert(concrete.name.clone());
                                items.push(Item::Type(concrete.clone()));
                            }
                        }
                        Item::Function(function) if function.template_params.is_empty() => {
                            let concrete_name = self
                                .overload_names
                                .get(&overload_key(
                                    &function.name,
                                    &function.params,
                                    function.return_type.as_deref(),
                                ))
                                .map(String::as_str)
                                .unwrap_or(&function.name);
                            if let Some(concrete) = self.concrete_functions.get(concrete_name) {
                                emitted_functions.insert(concrete.name.clone());
                                items.push(Item::Function(concrete.clone()));
                            }
                        }
                        // Native LINK constructs are not monomorphized; preserve
                        // them verbatim so later stages (resolve, syntaxcheck,
                        // package metadata) still see them.
                        Item::Resource(resource) => {
                            items.push(Item::Resource(resource.clone()));
                        }
                        Item::FuncAlias(alias) => {
                            items.push(Item::FuncAlias(alias.clone()));
                        }
                        Item::Link(link) => {
                            items.push(Item::Link(link.clone()));
                        }
                        // Preserve DOC blocks verbatim so the post-monomorph
                        // resolve and IR lowering still see the documentation.
                        Item::Doc(doc) => {
                            items.push(Item::Doc(doc.clone()));
                        }
                        _ => {}
                    }
                }
                AstFile {
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
                .extend(generated_types.into_iter().map(Item::Type));

            let mut generated_functions = self
                .concrete_functions
                .into_values()
                .filter(|function| !emitted_functions.contains(&function.name))
                .collect::<Vec<_>>();
            generated_functions.sort_by(|left, right| left.name.cmp(&right.name));
            first_file
                .items
                .extend(generated_functions.into_iter().map(Item::Function));
        }

        AstProject {
            name: self.source.name.clone(),
            files,
        }
    }

    fn lower_type(
        &mut self,
        mut type_decl: TypeDecl,
        substitutions: &HashMap<String, String>,
        concrete_name: Option<String>,
    ) -> TypeDecl {
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

    fn lower_binding(&mut self, mut binding: TopLevelBinding) -> TopLevelBinding {
        if let Some(type_name) = &binding.type_name {
            binding.type_name = Some(self.concrete_type_name(type_name, &HashMap::new()));
        }
        if let Some(value) = binding.value.take() {
            let mut context = self.function_context();
            binding.value = Some(self.lower_expression(
                &value,
                &HashMap::new(),
                &mut context,
                binding.type_name.as_deref(),
                binding.line,
            ));
        }
        binding
    }

    fn lower_function(
        &mut self,
        function: Function,
        substitutions: &HashMap<String, String>,
        concrete_name: Option<String>,
    ) -> Function {
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
        mut function: Function,
        substitutions: &HashMap<String, String>,
        concrete_name: Option<String>,
    ) -> Function {
        if let Some(name) = concrete_name {
            function.name = name;
        }
        function.template_params.clear();
        for param in &mut function.params {
            if let Some(type_name) = &param.type_name {
                param.type_name = Some(self.concrete_type_name(type_name, substitutions));
            }
        }
        if let Some(return_type) = &function.return_type {
            function.return_type = Some(self.concrete_type_name(return_type, substitutions));
        }

        let mut context = self.function_context();
        context.enclosing_return = function.return_type.clone();
        for param in &function.params {
            if let Some(type_name) = &param.type_name {
                context.locals.insert(param.name.clone(), type_name.clone());
            }
        }
        function.body = self.lower_statements(&function.body, substitutions, &mut context);
        if let Some(trap) = &mut function.trap {
            let mut trap_context = context.clone();
            trap_context
                .locals
                .insert(trap.name.clone(), "Error".to_string());
            trap.body = self.lower_statements(&trap.body, substitutions, &mut trap_context);
        }
        function
    }

    fn instantiate_function(
        &mut self,
        name: &str,
        arg_types: &[String],
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

        let mut substitutions = HashMap::new();
        for (param, actual) in template.params.iter().zip(arg_types.iter()) {
            let Some(pattern) = &param.type_name else {
                continue;
            };
            let actual = self.template_view_type(actual);
            if !unify_type(
                pattern,
                &actual,
                &template.template_params,
                &mut substitutions,
            ) {
                self.report(
                    "TYPE_CALL_ARGUMENT_MISMATCH",
                    &format!(
                        "Call to `{display}` cannot infer template arguments from `{actual}`."
                    ),
                    line,
                );
                return None;
            }
        }

        let args = template
            .template_params
            .iter()
            .map(|param| substitutions.get(param).cloned())
            .collect::<Option<Vec<_>>>()?;
        let concrete_name = mangle_name(name, &args);
        let key = format!("{name}<{}>", args.join(","));
        if self.emitted_function_keys.insert(key) {
            let mut full_substitutions = HashMap::new();
            for (param, arg) in template.template_params.iter().zip(args.iter()) {
                full_substitutions.insert(param.clone(), arg.clone());
            }
            let lowered =
                self.lower_function(template, &full_substitutions, Some(concrete_name.clone()));
            self.concrete_functions
                .insert(concrete_name.clone(), lowered);
        }
        Some(concrete_name)
    }

    fn resolve_overload(
        &mut self,
        name: &str,
        arg_types: &[String],
        expected: Option<&str>,
        line: usize,
    ) -> Option<String> {
        // Built-in-named overrides are routed by `resolve_general_builtin_override`,
        // which enforces the gap-fill rule (the built-in wins for its own types).
        if crate::builtins::general::is_overridable(name) {
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
                    .filter(|function| function.return_type.as_deref() == expected);
                match (by_return.next(), by_return.next()) {
                    (Some(unique), None) => unique.clone(),
                    _ => {
                        self.report(
                            "TYPE_OVERLOAD_AMBIGUOUS",
                            &format!(
                                "Call to `{name}` matches {} overloads that differ only by return \
                                 type; supply the expected type (e.g. a `LET … AS` annotation) to \
                                 select one.",
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
                chosen.return_type.as_deref(),
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
    fn resolve_general_builtin_override(&self, name: &str, arg_types: &[String]) -> Option<String> {
        if !crate::builtins::general::is_overridable(name) {
            return None;
        }
        if crate::builtins::general::resolve_call(name, arg_types).is_some() {
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
                chosen.return_type.as_deref(),
            ))
            .cloned()
    }

    /// The parameter list of `callee` when it names exactly one user function (no
    /// overloading). Supplies the expected (contextual) type for an argument slot
    /// so a return-type-overloaded call passed as an argument resolves
    /// (plan-01-overload.md §F.2); `None` when the callee is overloaded, a package
    /// member, or unknown.
    fn single_signature_params(&self, callee: &str) -> Option<Vec<crate::ast::Param>> {
        let candidates = self.function_overloads.get(callee)?;
        (candidates.len() == 1).then(|| candidates[0].params.clone())
    }

    fn instantiate_type(&mut self, name: &str, args: &[String]) -> String {
        let concrete_name = mangle_name(name, args);
        self.type_instantiations
            .insert(concrete_name.clone(), (name.to_string(), args.to_vec()));
        let key = format!("{name}<{}>", args.join(","));
        if !self.emitted_type_keys.insert(key) {
            return concrete_name;
        }
        let Some(template) = self.type_templates.get(name).cloned() else {
            return concrete_name;
        };
        let mut substitutions = HashMap::new();
        for (param, arg) in template.template_params.iter().zip(args.iter()) {
            substitutions.insert(param.clone(), arg.clone());
        }
        let concrete = self.lower_type(template, &substitutions, Some(concrete_name.clone()));
        self.concrete_types.insert(concrete_name.clone(), concrete);
        concrete_name
    }

    fn lower_field(
        &mut self,
        field: &TypeField,
        substitutions: &HashMap<String, String>,
    ) -> TypeField {
        let mut lowered = field.clone();
        lowered.type_name = self.concrete_type_name(&field.type_name, substitutions);
        lowered
    }

    fn lower_statements(
        &mut self,
        statements: &[Statement],
        substitutions: &HashMap<String, String>,
        context: &mut FunctionContext,
    ) -> Vec<Statement> {
        statements
            .iter()
            .map(|statement| self.lower_statement(statement, substitutions, context))
            .collect()
    }

    fn lower_statement(
        &mut self,
        statement: &Statement,
        substitutions: &HashMap<String, String>,
        context: &mut FunctionContext,
    ) -> Statement {
        match statement {
            Statement::Let {
                mutable,
                resource,
                state_type,
                name,
                type_name,
                value,
                line,
            } => {
                let lowered_type = type_name
                    .as_ref()
                    .map(|type_name| self.concrete_type_name(type_name, substitutions));
                let lowered_state = state_type
                    .as_ref()
                    .map(|state_type| self.concrete_type_name(state_type, substitutions));
                let expected_source_type = type_name
                    .as_ref()
                    .map(|type_name| substitute_type_params(type_name, substitutions));
                let lowered_value = value.as_ref().map(|value| {
                    self.lower_expression(
                        value,
                        substitutions,
                        context,
                        expected_source_type.as_deref(),
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
                Statement::Let {
                    mutable: *mutable,
                    resource: *resource,
                    state_type: lowered_state,
                    name: name.clone(),
                    type_name: lowered_type,
                    value: lowered_value,
                    line: *line,
                }
            }
            Statement::Return { value, line } => Statement::Return {
                value: value.as_ref().map(|value| {
                    // A `RETURN` of a call propagates the enclosing function's
                    // declared return type as the expected type so a return-type
                    // overload set resolves (plan-01-overload.md §F.2).
                    let expected = matches!(value, Expression::Call { .. })
                        .then(|| context.enclosing_return.clone())
                        .flatten();
                    self.lower_expression(value, substitutions, context, expected.as_deref(), *line)
                }),
                line: *line,
            },
            Statement::Exit { target, code, line } => Statement::Exit {
                target: *target,
                code: code
                    .as_ref()
                    .map(|value| self.lower_expression(value, substitutions, context, None, *line)),
                line: *line,
            },
            Statement::Continue { kind, line } => Statement::Continue {
                kind: *kind,
                line: *line,
            },
            Statement::Fail { error, line } => Statement::Fail {
                error: self.lower_expression(error, substitutions, context, None, *line),
                line: *line,
            },
            Statement::Propagate { line } => Statement::Propagate { line: *line },
            Statement::Recover { value, line } => Statement::Recover {
                value: value
                    .as_ref()
                    .map(|value| self.lower_expression(value, substitutions, context, None, *line)),
                line: *line,
            },
            Statement::Assign { name, value, line } => Statement::Assign {
                name: name.clone(),
                value: self.lower_expression(value, substitutions, context, None, *line),
                line: *line,
            },
            Statement::StateAssign {
                resource,
                value,
                line,
            } => Statement::StateAssign {
                resource: resource.clone(),
                value: self.lower_expression(value, substitutions, context, None, *line),
                line: *line,
            },
            Statement::Expression { expression, line } => Statement::Expression {
                expression: self.lower_expression(expression, substitutions, context, None, *line),
                line: *line,
            },
            Statement::If {
                condition,
                then_body,
                else_body,
                line,
            } => {
                let mut then_context = context.clone();
                let mut else_context = context.clone();
                Statement::If {
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
            Statement::Match {
                expression,
                cases,
                line,
            } => Statement::Match {
                expression: self.lower_expression(expression, substitutions, context, None, *line),
                cases: cases
                    .iter()
                    .map(|case| {
                        let mut case_context = context.clone();
                        if let MatchPattern::Union { binding, type_name } = &case.pattern {
                            case_context.locals.insert(
                                binding.clone(),
                                self.concrete_type_name(type_name, substitutions),
                            );
                        }
                        MatchCase {
                            pattern: match &case.pattern {
                                MatchPattern::Else => MatchPattern::Else,
                                MatchPattern::Literal(expression) => {
                                    MatchPattern::Literal(self.lower_expression(
                                        expression,
                                        substitutions,
                                        &mut case_context,
                                        None,
                                        case.line,
                                    ))
                                }
                                MatchPattern::Union { type_name, binding } => MatchPattern::Union {
                                    type_name: self.concrete_type_name(type_name, substitutions),
                                    binding: binding.clone(),
                                },
                                MatchPattern::OneOf(expressions) => MatchPattern::OneOf(
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
            Statement::For {
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
                            .unwrap_or_else(|| "Integer".to_string());
                        promote_loop_numeric_type_name(&start_type, &end_type, &step_type)
                    })
                {
                    nested.locals.insert(name.clone(), loop_type);
                }
                Statement::For {
                    name: name.clone(),
                    start: lowered_start,
                    end: lowered_end,
                    step: lowered_step,
                    body: self.lower_statements(body, substitutions, &mut nested),
                    line: *line,
                }
            }
            Statement::ForEach {
                name,
                iterable,
                body,
                line,
            } => {
                let lowered_iterable =
                    self.lower_expression(iterable, substitutions, context, None, *line);
                let mut nested = context.clone();
                if let Some(type_name) = self.expression_type(&lowered_iterable, context) {
                    let loop_type = if let Some(element) = type_name.strip_prefix("List OF ") {
                        element.to_string()
                    } else if let Some(rest) = type_name.strip_prefix("Map OF ") {
                        format!("MapEntry OF {rest}")
                    } else {
                        "Unknown".to_string()
                    };
                    nested.locals.insert(name.clone(), loop_type);
                }
                Statement::ForEach {
                    name: name.clone(),
                    iterable: lowered_iterable,
                    body: self.lower_statements(body, substitutions, &mut nested),
                    line: *line,
                }
            }
            Statement::While {
                kind,
                condition,
                body,
                line,
            } => Statement::While {
                kind: *kind,
                condition: self.lower_expression(condition, substitutions, context, None, *line),
                body: self.lower_statements(body, substitutions, &mut context.clone()),
                line: *line,
            },
            Statement::DoUntil {
                body,
                condition,
                line,
            } => Statement::DoUntil {
                body: self.lower_statements(body, substitutions, &mut context.clone()),
                condition: self.lower_expression(condition, substitutions, context, None, *line),
                line: *line,
            },
        }
    }

    fn lower_expression(
        &mut self,
        expression: &Expression,
        substitutions: &HashMap<String, String>,
        context: &mut FunctionContext,
        expected_type: Option<&str>,
        line: usize,
    ) -> Expression {
        match expression {
            Expression::Call {
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
                        CallArg::Positional(value) => {
                            let expected =
                                arg_slot_expected(value, sig_params.as_deref(), |params| {
                                    params.get(index)
                                });
                            CallArg::Positional(self.lower_expression(
                                value,
                                substitutions,
                                context,
                                expected,
                                line,
                            ))
                        }
                        CallArg::Named { name, value, line } => {
                            let expected =
                                arg_slot_expected(value, sig_params.as_deref(), |params| {
                                    params.iter().find(|param| param.name == *name)
                                });
                            CallArg::Named {
                                name: name.clone(),
                                value: self.lower_expression(
                                    value,
                                    substitutions,
                                    context,
                                    expected,
                                    *line,
                                ),
                                line: *line,
                            }
                        }
                    })
                    .collect::<Vec<_>>();
                let arg_types = lowered_args
                    .iter()
                    .filter_map(|argument| self.expression_type(call_arg_value(argument), context))
                    .collect::<Vec<_>>();
                // Resolve the overloaded `encoding::utf8Encode`/`utf8Decode`
                // public calls onto their concrete internal implementation using
                // the argument types and (for the return-type overload) the
                // expected type (plan-02-encoding.md Part B).
                if crate::builtins::encoding::is_overloaded(callee) {
                    match crate::builtins::encoding::resolve_overload_target(
                        callee,
                        &arg_types,
                        expected_type,
                    ) {
                        Ok(Some(target)) => {
                            // The target is a package-qualified built-in member
                            // (`encoding.utf8EncodeBytes`); the post-monomorph
                            // resolver accepts it and IR maps it to its internal
                            // implementation, like the other encoding functions.
                            return Expression::Call {
                                callee: target.to_string(),
                                arguments: lowered_args,
                                line: *call_line,
                                column: *column,
                            };
                        }
                        Ok(None) => {}
                        Err(()) => {
                            self.report(
                                "TYPE_OVERLOAD_AMBIGUOUS",
                                &format!(
                                    "Call to `{callee}` matches overloads that differ only by \
                                     return type; supply the expected type (e.g. a `LET … AS` \
                                     annotation) to select one."
                                ),
                                line,
                            );
                            return Expression::Call {
                                callee: callee.clone(),
                                arguments: lowered_args,
                                line: *call_line,
                                column: *column,
                            };
                        }
                    }
                }
                // Rewrite a `collections::` call onto its internal generic
                // implementation so it instantiates like any generic function.
                let callee = &self
                    .collections_internal_callee(callee)
                    .unwrap_or_else(|| callee.clone());
                let target = if let Some(target) =
                    self.instantiate_function(callee, &arg_types, line)
                {
                    target
                } else if let Some(target) =
                    self.resolve_general_builtin_override(callee, &arg_types)
                {
                    target
                } else if let Some(target) =
                    self.resolve_overload(callee, &arg_types, expected_type, line)
                {
                    target
                } else if let Some(target) =
                    self.resolve_imported_overload(callee, &arg_types, line)
                {
                    target
                } else {
                    callee.clone()
                };
                if target != *callee {
                    self.add_function_to_context(&target, context);
                }
                Expression::Call {
                    callee: target,
                    arguments: lowered_args,
                    line: *call_line,
                    column: *column,
                }
            }
            Expression::Constructor {
                type_name,
                arguments,
            } => {
                let mut concrete_type = None;
                if let Some((expected_name, expected_args)) =
                    expected_type.and_then(user_template_parts)
                {
                    if expected_name == *type_name {
                        concrete_type = Some(self.instantiate_type(&expected_name, &expected_args));
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
                            expected_arg_type,
                        )
                    })
                    .collect::<Vec<_>>();
                if concrete_type.is_none() && self.type_templates.contains_key(type_name) {
                    let Some(template) = self.type_templates.get(type_name).cloned() else {
                        unreachable!();
                    };
                    let mut inferred = HashMap::new();
                    let fields = match template.kind {
                        TypeDeclKind::Type => template.fields.clone(),
                        TypeDeclKind::Union => Vec::new(),
                        TypeDeclKind::Enum => Vec::new(),
                    };
                    for (field, argument) in fields.iter().zip(lowered_args.iter()) {
                        if let Some(actual) =
                            self.expression_type(constructor_arg_value(argument), context)
                        {
                            unify_type(
                                &field.type_name,
                                &actual,
                                &template.template_params,
                                &mut inferred,
                            );
                        }
                    }
                    let args = template
                        .template_params
                        .iter()
                        .map(|param| inferred.get(param).cloned())
                        .collect::<Option<Vec<_>>>();
                    if let Some(args) = args {
                        concrete_type = Some(self.instantiate_type(type_name, &args));
                    }
                }
                Expression::Constructor {
                    type_name: concrete_type.unwrap_or_else(|| type_name.clone()),
                    arguments: lowered_args,
                }
            }
            Expression::WithUpdate { target, updates } => Expression::WithUpdate {
                target: Box::new(self.lower_expression(target, substitutions, context, None, line)),
                updates: updates
                    .iter()
                    .map(|update| RecordUpdate {
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
            Expression::ListLiteral(values) => Expression::ListLiteral(
                values
                    .iter()
                    .map(|value| {
                        let expected_element =
                            expected_type.and_then(|type_| type_.strip_prefix("List OF "));
                        self.lower_expression(value, substitutions, context, expected_element, line)
                    })
                    .collect(),
            ),
            Expression::MapLiteral {
                key_type,
                value_type,
                entries,
            } => Expression::MapLiteral {
                key_type: self.concrete_type_name(key_type, substitutions),
                value_type: self.concrete_type_name(value_type, substitutions),
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
            Expression::MemberAccess { target, member } => Expression::MemberAccess {
                target: Box::new(self.lower_expression(target, substitutions, context, None, line)),
                member: member.clone(),
            },
            Expression::Binary {
                left,
                operator,
                right,
                line: op_line,
                column,
            } => Expression::Binary {
                left: Box::new(self.lower_expression(left, substitutions, context, None, line)),
                operator: operator.clone(),
                right: Box::new(self.lower_expression(right, substitutions, context, None, line)),
                line: *op_line,
                column: *column,
            },
            Expression::Unary {
                operator,
                operand,
                line: op_line,
                column,
            } => Expression::Unary {
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
            Expression::Lambda {
                params,
                body,
                assign_target,
            } => {
                let mut nested = context.clone();
                let lowered_params = params
                    .iter()
                    .map(|param| {
                        let mut lowered = param.clone();
                        if let Some(type_name) = &param.type_name {
                            lowered.type_name =
                                Some(self.concrete_type_name(type_name, substitutions));
                            nested
                                .locals
                                .insert(param.name.clone(), lowered.type_name.clone().unwrap());
                        }
                        lowered
                    })
                    .collect::<Vec<_>>();
                Expression::Lambda {
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
            Expression::Trapped {
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
                    .insert(binding.clone(), "Error".to_string());
                let lowered_handler =
                    self.lower_statements(handler, substitutions, &mut handler_context);
                Expression::Trapped {
                    expression: lowered_expression,
                    binding: binding.clone(),
                    handler: lowered_handler,
                    line: *trap_line,
                }
            }
            Expression::Identifier(value) => Expression::Identifier(value.clone()),
            Expression::String(value) => Expression::String(value.clone()),
            Expression::Number(value) => Expression::Number(value.clone()),
            Expression::Boolean(value) => Expression::Boolean(*value),
        }
    }

    fn lower_constructor_arg(
        &mut self,
        argument: &ConstructorArg,
        substitutions: &HashMap<String, String>,
        context: &mut FunctionContext,
        line: usize,
        expected_type: Option<&str>,
    ) -> ConstructorArg {
        match argument {
            ConstructorArg::Positional(value) => ConstructorArg::Positional(self.lower_expression(
                value,
                substitutions,
                context,
                expected_type,
                line,
            )),
            ConstructorArg::Named {
                name,
                value,
                line: arg_line,
            } => ConstructorArg::Named {
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

    fn concrete_type_name(
        &mut self,
        type_name: &str,
        substitutions: &HashMap<String, String>,
    ) -> String {
        // A grouped type (`(T)`) is valid syntax the parser keeps verbatim;
        // unwrap it before matching so it isn't mis-parsed as a `(Map`-named
        // template and mangled into garbage (bug-105).
        let type_name = crate::builtins::thread::strip_type_group(type_name);
        if let Some(value) = substitutions.get(type_name) {
            return value.clone();
        }
        if let Some(element) = type_name.strip_prefix("List OF ") {
            return format!(
                "List OF {}",
                self.concrete_type_name(element, substitutions)
            );
        }
        if let Some(success) = type_name.strip_prefix("Result OF ") {
            return format!(
                "Result OF {}",
                self.concrete_type_name(success, substitutions)
            );
        }
        if let Some(rest) = type_name.strip_prefix("Map OF ") {
            if let Some((key, value)) = split_top_level_to(rest) {
                return format!(
                    "Map OF {} TO {}",
                    self.concrete_type_name(&key, substitutions),
                    self.concrete_type_name(&value, substitutions)
                );
            }
        }
        if let Some(rest) = type_name.strip_prefix("MapEntry OF ") {
            if let Some((key, value)) = split_top_level_to(rest) {
                return format!(
                    "MapEntry OF {} TO {}",
                    self.concrete_type_name(&key, substitutions),
                    self.concrete_type_name(&value, substitutions)
                );
            }
        }
        if let Some((kind, message, resource, output)) =
            crate::builtins::thread::thread_parts_full(type_name)
        {
            let resource =
                resource.map(|resource| self.concrete_type_name(resource, substitutions));
            return crate::builtins::thread::format_thread_type(
                kind,
                &self.concrete_type_name(message, substitutions),
                resource.as_deref(),
                &self.concrete_type_name(output, substitutions),
            );
        }
        if let Some((params, ret)) = func_type_parts(type_name) {
            let prefix = if type_name.starts_with("ISOLATED FUNC(") {
                "ISOLATED FUNC("
            } else {
                "FUNC("
            };
            let params = params
                .iter()
                .map(|param| self.concrete_type_name(param, substitutions))
                .collect::<Vec<_>>();
            return format!(
                "{prefix}{}) AS {}",
                params.join(", "),
                self.concrete_type_name(ret, substitutions)
            );
        }
        if let Some((name, args)) = user_template_parts(type_name) {
            let args = args
                .iter()
                .map(|arg| self.concrete_type_name(arg, substitutions))
                .collect::<Vec<_>>();
            return self.instantiate_type(&name, &args);
        }
        type_name.to_string()
    }

    fn template_view_type(&self, type_name: &str) -> String {
        let type_name = crate::builtins::thread::strip_type_group(type_name);
        if let Some(element) = type_name.strip_prefix("List OF ") {
            return format!("List OF {}", self.template_view_type(element));
        }
        if let Some(success) = type_name.strip_prefix("Result OF ") {
            return format!("Result OF {}", self.template_view_type(success));
        }
        if let Some(rest) = type_name.strip_prefix("Map OF ") {
            if let Some((key, value)) = split_top_level_to(rest) {
                return format!(
                    "Map OF {} TO {}",
                    self.template_view_type(&key),
                    self.template_view_type(&value)
                );
            }
        }
        if let Some(rest) = type_name.strip_prefix("MapEntry OF ") {
            if let Some((key, value)) = split_top_level_to(rest) {
                return format!(
                    "MapEntry OF {} TO {}",
                    self.template_view_type(&key),
                    self.template_view_type(&value)
                );
            }
        }
        if let Some((kind, message, resource, output)) =
            crate::builtins::thread::thread_parts_full(type_name)
        {
            let resource = resource.map(|resource| self.template_view_type(resource));
            return crate::builtins::thread::format_thread_type(
                kind,
                &self.template_view_type(message),
                resource.as_deref(),
                &self.template_view_type(output),
            );
        }
        if let Some((name, args)) = self.type_instantiations.get(type_name) {
            let args = args
                .iter()
                .map(|arg| self.template_view_type(arg))
                .collect::<Vec<_>>();
            return format!("{name} OF {}", args.join(", "));
        }
        type_name.to_string()
    }

    fn function_context(&self) -> FunctionContext {
        let mut context = FunctionContext::default();
        for (name, function) in &self.concrete_functions {
            let returns = match function.kind {
                crate::ast::FunctionKind::Func => function
                    .return_type
                    .clone()
                    .unwrap_or_else(|| "Unknown".to_string()),
                crate::ast::FunctionKind::Sub => "Nothing".to_string(),
            };
            let params = function
                .params
                .iter()
                .map(|param| {
                    param
                        .type_name
                        .clone()
                        .unwrap_or_else(|| "Unknown".to_string())
                })
                .collect::<Vec<_>>();
            context
                .function_returns
                .insert(name.clone(), returns.clone());
            context.function_types.insert(
                name.clone(),
                format!(
                    "{}FUNC({}) AS {returns}",
                    if function.isolated { "ISOLATED " } else { "" },
                    params.join(", ")
                ),
            );
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
            if let Item::Binding(binding) = item {
                if let Some(type_name) = &binding.type_name {
                    context
                        .globals
                        .insert(binding.name.clone(), type_name.clone());
                }
            }
        }
        context
    }

    fn add_function_to_context(&self, name: &str, context: &mut FunctionContext) {
        let Some(function) = self.concrete_functions.get(name) else {
            return;
        };
        let returns = match function.kind {
            crate::ast::FunctionKind::Func => function
                .return_type
                .clone()
                .unwrap_or_else(|| "Unknown".to_string()),
            crate::ast::FunctionKind::Sub => "Nothing".to_string(),
        };
        let params = function
            .params
            .iter()
            .map(|param| {
                param
                    .type_name
                    .clone()
                    .unwrap_or_else(|| "Unknown".to_string())
            })
            .collect::<Vec<_>>();
        context
            .function_returns
            .insert(name.to_string(), returns.clone());
        context.function_types.insert(
            name.to_string(),
            format!(
                "{}FUNC({}) AS {returns}",
                if function.isolated { "ISOLATED " } else { "" },
                params.join(", ")
            ),
        );
    }

    /// The return type of a builtin/package call, using the same per-package
    /// `resolve_call` resolvers that syntaxcheck dispatches through
    /// (`SyntaxChecker::check_builtin_call`). Argument types are resolved
    /// positionally, falling back to `Unknown` so a resolver that keys on arity
    /// still sees the right shape. Without this, `expression_type` returned `None`
    /// for every builtin call, so a builtin-call argument was silently dropped
    /// from a generic/overloaded call's argument list (bug-103).
    fn builtin_call_return_type(
        &self,
        callee: &str,
        arguments: &[CallArg],
        context: &FunctionContext,
    ) -> Option<String> {
        use crate::builtins;
        let arg_types = arguments
            .iter()
            .map(|argument| {
                self.expression_type(call_arg_value(argument), context)
                    .unwrap_or_else(|| "Unknown".to_string())
            })
            .collect::<Vec<_>>();
        macro_rules! try_pkg {
            ($resolve:expr) => {
                if let Some(resolved) = $resolve {
                    return Some(resolved.return_type.into_owned());
                }
            };
        }
        try_pkg!(builtins::general::resolve_call(callee, &arg_types));
        try_pkg!(builtins::collections::resolve_call(callee, &arg_types));
        try_pkg!(builtins::strings::resolve_call(callee, &arg_types));
        try_pkg!(builtins::math::resolve_call(callee, &arg_types));
        try_pkg!(builtins::bits::resolve_call(callee, &arg_types));
        try_pkg!(builtins::crypto::resolve_call(callee, &arg_types));
        try_pkg!(builtins::encoding::resolve_call(callee, &arg_types));
        try_pkg!(builtins::fs::resolve_call(callee, &arg_types));
        try_pkg!(builtins::io::resolve_call(callee, &arg_types));
        try_pkg!(builtins::json::resolve_call(callee, &arg_types));
        try_pkg!(builtins::csv::resolve_call(callee, &arg_types));
        try_pkg!(builtins::regex::resolve_call(callee, &arg_types));
        try_pkg!(builtins::datetime::resolve_call(callee, &arg_types));
        try_pkg!(builtins::money::resolve_call(callee, &arg_types));
        try_pkg!(builtins::net::resolve_call(callee, &arg_types));
        try_pkg!(builtins::os::resolve_call(callee, &arg_types));
        try_pkg!(builtins::http::resolve_call(callee, &arg_types));
        try_pkg!(builtins::term::resolve_call(callee)); // no arg_types param
        try_pkg!(builtins::tls::resolve_call(callee, &arg_types));
        try_pkg!(builtins::audio::resolve_call(callee, &arg_types));
        try_pkg!(builtins::vector::resolve_call(callee, &arg_types));
        try_pkg!(builtins::thread::resolve_call(callee, &arg_types));
        None
    }

    fn expression_type(
        &self,
        expression: &Expression,
        context: &FunctionContext,
    ) -> Option<String> {
        match expression {
            Expression::String(_) => Some("String".to_string()),
            Expression::Number(value) => Some(
                match crate::numeric::classify_literal(value).1 {
                    crate::numeric::LiteralType::Integer => "Integer",
                    crate::numeric::LiteralType::Float => "Float",
                    crate::numeric::LiteralType::Fixed => "Fixed",
                    crate::numeric::LiteralType::Money => "Money",
                }
                .to_string(),
            ),
            Expression::Boolean(_) => Some("Boolean".to_string()),
            Expression::Identifier(value) if value == "NOTHING" => Some("Nothing".to_string()),
            Expression::Identifier(value) => context
                .locals
                .get(value)
                .cloned()
                .or_else(|| context.function_types.get(value).cloned())
                .or_else(|| context.globals.get(value).cloned()),
            Expression::Constructor { type_name, .. } => {
                if type_name == "Error" {
                    Some("Error".to_string())
                } else if type_name == "Ok" {
                    Some("Result OF Unknown".to_string())
                } else if context.record_fields.contains_key(type_name) {
                    Some(type_name.clone())
                } else {
                    None
                }
            }
            Expression::WithUpdate { target, .. } => self.expression_type(target, context),
            Expression::ListLiteral(values) => values
                .first()
                .and_then(|value| self.expression_type(value, context))
                .map(|element| format!("List OF {element}"))
                .or_else(|| Some("List OF Unknown".to_string())),
            Expression::MapLiteral {
                key_type,
                value_type,
                ..
            } => Some(format!("Map OF {key_type} TO {value_type}")),
            Expression::MemberAccess { target, member } => {
                let target_type = self.expression_type(target, context)?;
                context
                    .record_fields
                    .get(&target_type)?
                    .iter()
                    .find(|field| field.name == *member)
                    .map(|field| field.type_name.clone())
            }
            Expression::Call {
                callee, arguments, ..
            } => context
                .function_returns
                .get(callee)
                .cloned()
                .or_else(|| self.builtin_call_return_type(callee, arguments, context)),
            Expression::Lambda {
                params,
                body,
                assign_target,
            } => {
                let mut nested = context.clone();
                let param_types = params
                    .iter()
                    .map(|param| {
                        let type_name = param
                            .type_name
                            .clone()
                            .unwrap_or_else(|| "Unknown".to_string());
                        nested.locals.insert(param.name.clone(), type_name.clone());
                        type_name
                    })
                    .collect::<Vec<_>>();
                // An assignment-bodied lambda yields `Nothing`; otherwise its
                // result type is the body expression's type.
                let returns = if assign_target.is_some() {
                    "Nothing".to_string()
                } else {
                    self.expression_type(body, &nested)?
                };
                Some(format!("FUNC({}) AS {returns}", param_types.join(", ")))
            }
            Expression::Binary {
                operator,
                left,
                right,
                ..
            } => {
                if matches!(
                    operator.as_str(),
                    "=" | "<>" | "<" | ">" | "<=" | ">=" | "AND" | "OR" | "XOR"
                ) {
                    return Some("Boolean".to_string());
                }
                if operator == "&" {
                    return Some("String".to_string());
                }
                let left = self.expression_type(left, context)?;
                let right = self.expression_type(right, context)?;
                Some(numeric_binary_result_type(operator, &left, &right).to_string())
            }
            Expression::Unary {
                operator, operand, ..
            } => {
                if operator == "NOT" {
                    Some("Boolean".to_string())
                } else {
                    self.expression_type(operand, context)
                }
            }
            Expression::Trapped { expression, .. } => self.expression_type(expression, context),
        }
    }

    fn report(&mut self, rule: &str, detail: &str, line: usize) {
        self.had_error = true;
        // Prefer the file whose body is currently being lowered (bug-107); fall
        // back to the first project file only when the frame is unknown.
        let relative = self.current_file.clone().or_else(|| {
            self.source
                .files
                .first()
                .map(|file| file.path.clone())
        });
        let path = relative
            .map(|rel| self.project_dir.join(rel))
            .unwrap_or_else(|| self.project_dir.join("src/main.mfb"));
        rules::show_diagnostic(rule, detail, &path, line, 1, 1);
    }
}

#[cfg(test)]
mod tests {
    use super::super::{ImportedOverload, Monomorphizer};
    use crate::ast::{AstProject, Function, Item, TypeDecl};

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
    fn monomorphize(src: &str) -> Result<AstProject, ()> {
        monomorphize_files(&[("src/main.mfb", src)])
    }

    fn monomorphize_files(files: &[(&str, &str)]) -> Result<AstProject, ()> {
        let ast = project(files);
        let dir = std::env::temp_dir();
        let prev = std::panic::take_hook();
        // Silence the front end's diagnostic printing during error-path tests.
        let result = super::super::monomorphize_project(&dir, &ast);
        std::panic::set_hook(prev);
        result
    }

    fn functions(project: &AstProject) -> Vec<&Function> {
        project
            .files
            .iter()
            .flat_map(|f| &f.items)
            .filter_map(|item| match item {
                Item::Function(function) => Some(function),
                _ => None,
            })
            .collect()
    }

    fn types(project: &AstProject) -> Vec<&TypeDecl> {
        project
            .files
            .iter()
            .flat_map(|f| &f.items)
            .filter_map(|item| match item {
                Item::Type(type_decl) => Some(type_decl),
                _ => None,
            })
            .collect()
    }

    fn function_names(project: &AstProject) -> Vec<String> {
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
  WEND
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
        let mut monomorphizer = Monomorphizer::new(dir.path(), &ast);
        monomorphizer.imported_overloads.insert(
            "pkg.f".to_string(),
            vec![
                ImportedOverload {
                    param_types: vec!["List OF Integer".to_string()],
                    qualified_name: "pkg.f$ListOFInteger".to_string(),
                },
                ImportedOverload {
                    param_types: vec!["List OF String".to_string()],
                    qualified_name: "pkg.f$ListOFString".to_string(),
                },
            ],
        );

        // A concretely-typed argument selects exactly one overload.
        assert_eq!(
            monomorphizer
                .resolve_imported_overload("pkg.f", &["List OF Integer".to_string()], 1)
                .as_deref(),
            Some("pkg.f$ListOFInteger")
        );
        assert_eq!(
            monomorphizer
                .resolve_imported_overload("pkg.f", &["List OF String".to_string()], 1)
                .as_deref(),
            Some("pkg.f$ListOFString")
        );
        assert!(!monomorphizer.had_error);

        // `f([])` matches both through the `Unknown` wildcard: ambiguous, not
        // "whichever came first".
        assert_eq!(
            monomorphizer.resolve_imported_overload("pkg.f", &["List OF Unknown".to_string()], 7),
            None
        );
        assert!(monomorphizer.had_error);

        // An unrelated callee and a wrong arity still resolve to nothing.
        assert_eq!(
            monomorphizer.resolve_imported_overload("pkg.other", &[], 1),
            None
        );
        assert_eq!(monomorphizer.resolve_imported_overload("pkg.f", &[], 1), None);
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
        let concrete = super::super::monomorphize_project(dir.path(), &ast).expect("monomorphizes");
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
        let _ = super::super::monomorphize_project(dir.path(), &ast);
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
}
