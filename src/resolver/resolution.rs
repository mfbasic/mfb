use super::*;

impl Resolver<'_> {
    pub(super) fn resolve(&mut self) {
        for file in &self.hir.files {
            self.resolve_file(file);
        }
    }

    /// Validate every `DOC` block in the package (plan-09-doc.md §2/§4): resolve
    /// each header to a declaration of the right kind, then check the body's
    /// `ARG`/`PROP`/`RET`/`ERROR`/`EXAMPLE` lines, attributes, and `DEPRECATED`
    /// against that declaration.
    pub(super) fn resolve_doc_blocks(&mut self) {
        let hir = self.hir;

        // Index user declarations by name. Functions share one namespace,
        // and a name may carry several overloads.
        let mut funcs: HashMap<&str, Vec<&HirFunction>> = HashMap::new();
        let mut types: HashMap<&str, &HirTypeDecl> = HashMap::new();
        let mut resources: HashMap<&str, &ResourceDecl> = HashMap::new();
        for file in &hir.files {
            for item in &file.items {
                match item {
                    HirItem::Function(function) => {
                        funcs
                            .entry(function.name.as_str())
                            .or_default()
                            .push(function);
                    }
                    HirItem::Type(type_decl) => {
                        types.entry(type_decl.name.as_str()).or_insert(type_decl);
                    }
                    HirItem::Resource(resource) => {
                        resources.entry(resource.name.as_str()).or_insert(resource);
                    }
                    _ => {}
                }
            }
        }

        // Dedup key: (kind, name, overload-signature). The signature is the
        // parenthesized header parameter types (empty when none), so each overload
        // can carry its own DOC block.
        let mut seen: HashSet<(DocHeaderKind, String, String)> = HashSet::new();
        let mut package_doc_seen = false;
        for file in &hir.files {
            for item in &file.items {
                let HirItem::Doc(doc) = item else {
                    continue;
                };
                self.validate_doc_block(
                    file,
                    doc,
                    &funcs,
                    &types,
                    &resources,
                    &mut seen,
                    &mut package_doc_seen,
                );
            }
        }
    }

    fn validate_doc_block(
        &mut self,
        file: &HirFile,
        doc: &DocBlock,
        funcs: &HashMap<&str, Vec<&HirFunction>>,
        types: &HashMap<&str, &HirTypeDecl>,
        resources: &HashMap<&str, &ResourceDecl>,
        seen: &mut HashSet<(DocHeaderKind, String, String)>,
        package_doc_seen: &mut bool,
    ) {
        // --- Attributes (only INTERNAL is recognized) ---
        let mut internal_count = 0;
        for attr in &doc.attrs {
            if attr.eq_ignore_ascii_case("INTERNAL") {
                internal_count += 1;
            } else {
                self.report(
                    "DOC_UNKNOWN_ATTR",
                    &format!("`{attr}` is not a valid DOC attribute; only INTERNAL is recognized."),
                    file,
                    doc.line,
                );
            }
        }
        if internal_count > 1 {
            self.report(
                "DOC_DUPLICATE_ATTR",
                "The INTERNAL attribute may appear at most once on a DOC line.",
                file,
                doc.line,
            );
        }

        let kind = doc.header_kind;
        let is_callable = matches!(kind, DocHeaderKind::Func);
        let is_member_kind = matches!(
            kind,
            DocHeaderKind::Type | DocHeaderKind::Union | DocHeaderKind::Enum
        );

        // --- Context restrictions ---
        if !is_callable {
            if let Some(arg) = doc.args.first() {
                self.report(
                    "DOC_ARG_INVALID_CONTEXT",
                    "ARG is valid only on FUNC doc blocks.",
                    file,
                    arg.line,
                );
            }
            if let Some((_, line)) = doc.rets.first() {
                self.report(
                    "DOC_RET_INVALID_CONTEXT",
                    "RET is valid only on FUNC doc blocks.",
                    file,
                    *line,
                );
            }
            if let Some(error) = doc.errors.first() {
                self.report(
                    "DOC_ERROR_INVALID_CONTEXT",
                    "ERROR is valid only on FUNC doc blocks.",
                    file,
                    error.line,
                );
            }
        }
        if !is_member_kind {
            if let Some(prop) = doc.props.first() {
                self.report(
                    "DOC_PROP_INVALID_CONTEXT",
                    "PROP is valid only on TYPE, UNION, and ENUM doc blocks.",
                    file,
                    prop.line,
                );
            }
        }
        if !is_callable {
            if let Some((_, line)) = doc.groups.first() {
                self.report(
                    "DOC_GROUP_INVALID_CONTEXT",
                    "GROUP is valid only on FUNC doc blocks.",
                    file,
                    *line,
                );
            }
        }

        // --- Duplicate body lines ---
        if let Some((_, line)) = doc.rets.get(1) {
            self.report(
                "DOC_DUPLICATE_RET",
                "A DOC block may have at most one RET line.",
                file,
                *line,
            );
        }
        if let Some((_, line)) = doc.examples.get(1) {
            self.report(
                "DOC_DUPLICATE_EXAMPLE",
                "A DOC block may have at most one EXAMPLE block.",
                file,
                *line,
            );
        }
        if let Some((_, line)) = doc.deprecated.get(1) {
            self.report(
                "DOC_DUPLICATE_DEPRECATED",
                "A DOC block may have at most one DEPRECATED line.",
                file,
                *line,
            );
        }
        if let Some((_, line)) = doc.groups.get(1) {
            self.report(
                "DOC_DUPLICATE_GROUP",
                "A DOC block may have at most one GROUP line.",
                file,
                *line,
            );
        }

        // --- Header resolution & member checks ---
        match kind {
            DocHeaderKind::Package => {
                if internal_count > 0 {
                    self.report(
                        "DOC_INTERNAL_INVALID_CONTEXT",
                        "INTERNAL is not valid on a PACKAGE doc block.",
                        file,
                        doc.line,
                    );
                }
                if *package_doc_seen {
                    self.report(
                        "DOC_DUPLICATE_PACKAGE",
                        "Only one PACKAGE doc block is allowed per package.",
                        file,
                        doc.header_line,
                    );
                } else {
                    *package_doc_seen = true;
                }
            }
            DocHeaderKind::Func => {
                let resolved = match funcs.get(doc.header_name.as_str()) {
                    Some(list) => {
                        let matching: Vec<&&HirFunction> = list.iter().collect();
                        if let Some(types_wanted) = &doc.header_params {
                            // The header named a specific overload by its parameter
                            // types; validate ARGs against exactly that overload.
                            match matching
                                .iter()
                                .find(|f| overload_types_match(f, types_wanted))
                            {
                                Some(target) => {
                                    let valid: HashSet<&str> =
                                        target.params.iter().map(|p| p.name.as_str()).collect();
                                    self.check_doc_named(
                                        file,
                                        &doc.args,
                                        &valid,
                                        "DOC_ARG_UNKNOWN",
                                        "DOC_ARG_DUPLICATE",
                                        "parameter",
                                    );
                                    true
                                }
                                None => {
                                    self.report(
                                        "DOC_OVERLOAD_UNRESOLVED",
                                        &format!(
                                            "No overload of `{}` has parameter types ({}).",
                                            doc.header_name,
                                            types_wanted.join(", ")
                                        ),
                                        file,
                                        doc.header_line,
                                    );
                                    false
                                }
                            }
                        } else {
                            // No disambiguator: validate ARGs against the union of
                            // every matching overload's parameters.
                            let valid: HashSet<&str> = matching
                                .iter()
                                .flat_map(|f| f.params.iter().map(|p| p.name.as_str()))
                                .collect();
                            self.check_doc_named(
                                file,
                                &doc.args,
                                &valid,
                                "DOC_ARG_UNKNOWN",
                                "DOC_ARG_DUPLICATE",
                                "parameter",
                            );
                            true
                        }
                    }
                    None => {
                        let rule = if types.contains_key(doc.header_name.as_str()) {
                            "DOC_NAME_MISMATCH"
                        } else {
                            "DOC_UNRESOLVED"
                        };
                        self.report(
                            rule,
                            &format!(
                                "DOC header `{} {}` does not name a {} in this package.",
                                kind.keyword(),
                                doc.header_name,
                                kind.keyword()
                            ),
                            file,
                            doc.header_line,
                        );
                        false
                    }
                };
                if resolved {
                    self.note_doc_target(file, doc, seen);
                }
            }
            DocHeaderKind::Type | DocHeaderKind::Union | DocHeaderKind::Enum => {
                let want = match kind {
                    DocHeaderKind::Type => TypeDeclKind::Type,
                    DocHeaderKind::Union => TypeDeclKind::Union,
                    _ => TypeDeclKind::Enum,
                };
                // plan-111-B: a union's members are types now, so their names
                // are materialized once here for the borrowed `valid` set.
                let variant_names: Vec<String>;
                let resolved = match types.get(doc.header_name.as_str()) {
                    Some(type_decl) if type_decl.kind == want => {
                        let valid: HashSet<&str> = match want {
                            TypeDeclKind::Type => {
                                type_decl.fields.iter().map(|f| f.name.as_str()).collect()
                            }
                            TypeDeclKind::Union => {
                                // bug-680: a template-instantiated member
                                // (`Some OF T`) is named by its template, as a
                                // CASE names it.
                                variant_names = type_decl
                                    .variants
                                    .iter()
                                    .map(|v| match &v.type_ {
                                        ParameterType::UserOf(template, _) => {
                                            template.resolve().to_string()
                                        }
                                        other => other.name().into_owned(),
                                    })
                                    .collect::<Vec<_>>();
                                variant_names.iter().map(String::as_str).collect()
                            }
                            TypeDeclKind::Enum => {
                                type_decl.members.iter().map(|m| m.name.as_str()).collect()
                            }
                        };
                        self.check_doc_named(
                            file,
                            &doc.props,
                            &valid,
                            "DOC_PROP_UNKNOWN",
                            "DOC_PROP_DUPLICATE",
                            "member",
                        );
                        true
                    }
                    Some(_) => {
                        self.report(
                            "DOC_NAME_MISMATCH",
                            &format!(
                                "`{}` is not a {} in this package.",
                                doc.header_name,
                                kind.keyword()
                            ),
                            file,
                            doc.header_line,
                        );
                        false
                    }
                    None => {
                        // A RESOURCE of that name is the likely mix-up (`TYPE
                        // SoundFile` for an `EXPORT RESOURCE SoundFile`), so name
                        // the kind clash rather than claiming nothing resolves.
                        let rule = if funcs.contains_key(doc.header_name.as_str())
                            || resources.contains_key(doc.header_name.as_str())
                        {
                            "DOC_NAME_MISMATCH"
                        } else {
                            "DOC_UNRESOLVED"
                        };
                        self.report(
                            rule,
                            &format!(
                                "DOC header `{} {}` does not name a {} in this package.",
                                kind.keyword(),
                                doc.header_name,
                                kind.keyword()
                            ),
                            file,
                            doc.header_line,
                        );
                        false
                    }
                };
                if resolved {
                    self.note_doc_target(file, doc, seen);
                }
            }
            DocHeaderKind::Resource => {
                // A resource is an opaque handle with no fields, variants or
                // members, so there is nothing for PROP to name and no member set
                // to validate against. `is_member_kind` above already excludes
                // RESOURCE, so a stray PROP is reported once as
                // DOC_PROP_INVALID_CONTEXT; checking it again here would only
                // double-report the same line.
                let resolved = match resources.get(doc.header_name.as_str()) {
                    Some(_) => true,
                    None => {
                        let rule = if funcs.contains_key(doc.header_name.as_str())
                            || types.contains_key(doc.header_name.as_str())
                        {
                            "DOC_NAME_MISMATCH"
                        } else {
                            "DOC_UNRESOLVED"
                        };
                        self.report(
                            rule,
                            &format!(
                                "DOC header `RESOURCE {}` does not name a RESOURCE in this package.",
                                doc.header_name
                            ),
                            file,
                            doc.header_line,
                        );
                        false
                    }
                };
                if resolved {
                    self.note_doc_target(file, doc, seen);
                }
            }
        }
    }

    /// (See free function `overload_types_match`.)
    ///
    /// Record a successfully resolved doc target and flag a second block naming
    /// the same declaration (same overload signature) as `DOC_DUPLICATE`.
    fn note_doc_target(
        &mut self,
        file: &HirFile,
        doc: &DocBlock,
        seen: &mut HashSet<(DocHeaderKind, String, String)>,
    ) {
        let signature = match &doc.header_params {
            Some(types) => types.join(", "),
            None => String::new(),
        };
        if !seen.insert((doc.header_kind, doc.header_name.clone(), signature)) {
            let which = match &doc.header_params {
                Some(types) => format!(
                    "`{} {}({})`",
                    doc.header_kind.keyword(),
                    doc.header_name,
                    types.join(", ")
                ),
                None => format!("`{} {}`", doc.header_kind.keyword(), doc.header_name),
            };
            self.report(
                "DOC_DUPLICATE",
                &format!("{which} already has a DOC block in this package."),
                file,
                doc.header_line,
            );
        }
    }

    /// Validate `ARG`/`PROP` lines against the set of valid names, reporting
    /// duplicates and unknown names.
    fn check_doc_named(
        &mut self,
        file: &HirFile,
        named: &[crate::ast::DocNamed],
        valid: &HashSet<&str>,
        unknown_rule: &str,
        duplicate_rule: &str,
        noun: &str,
    ) {
        let mut documented: HashSet<&str> = HashSet::new();
        for entry in named {
            if !documented.insert(entry.name.as_str()) {
                self.report(
                    duplicate_rule,
                    &format!("{noun} `{}` is documented more than once.", entry.name),
                    file,
                    entry.line,
                );
            } else if !valid.contains(entry.name.as_str()) {
                self.report(
                    unknown_rule,
                    &format!(
                        "`{}` is not a {noun} of the documented declaration.",
                        entry.name
                    ),
                    file,
                    entry.line,
                );
            }
        }
    }

    fn resolve_file(&mut self, file: &HirFile) {
        let mut imports = HashMap::new();

        for import in &file.imports {
            let binding = import.binding_name();
            if let Some(previous) =
                imports.insert(binding.to_string(), import.package_name().to_string())
            {
                self.report(
                    "SYMBOL_DUPLICATE_IMPORT",
                    &format!(
                        "Import binding `{binding}` is already used for package `{previous}` in this file."
                    ),
                    file,
                    import.line,
                );
            }

            if builtins::is_builtin_import(binding) && import.alias.is_some() {
                self.report(
                    "SYMBOL_DUPLICATE_IMPORT",
                    &format!(
                        "Import alias `{binding}` conflicts with built-in package `{binding}`."
                    ),
                    file,
                    import.line,
                );
            }

            if self.top_level_visible_in_file(file, binding)
                || self.function_visible_in_file(file, binding)
            {
                self.report(
                    "SYMBOL_DUPLICATE_IMPORT",
                    &format!(
                        "Import alias `{binding}` conflicts with a visible top-level declaration."
                    ),
                    file,
                    import.line,
                );
            }

            self.resolve_imported_package(file, import.package_name(), import.line);
        }

        for item in &file.items {
            match item {
                HirItem::Binding(binding) => self.resolve_binding(file, binding, &imports),
                HirItem::Function(function) => self.resolve_function(file, function, &imports),
                HirItem::Type(type_decl) => self.resolve_type_decl(file, type_decl, &imports),
                HirItem::Resource(resource) => self.resolve_resource_decl(file, resource, &imports),
                HirItem::FuncAlias(alias) => self.resolve_func_alias(file, alias, &imports),
                HirItem::Link(link) => self.resolve_link_block(file, link, &imports),
                // DOC blocks are validated package-wide in `resolve_doc_blocks`.
                HirItem::Doc(_) => {}
                // TESTING blocks are lowered away before resolution (plan-18-A §3).
                HirItem::Testing(_) => {}
            }
        }
    }

    fn resolve_resource_decl(
        &mut self,
        file: &HirFile,
        resource: &crate::ast::ResourceDecl,
        imports: &HashMap<String, String>,
    ) {
        // The close op must name a LINK function in this package (plan-link-update.md
        // §5/§12). Naming an ordinary MFBASIC function would reintroduce the cut
        // source-defined-resource design.
        let Some((alias, func)) = resource.close_fn.split_once('.') else {
            self.report(
                "RESOURCE_CLOSE_NOT_NATIVE",
                &format!(
                    "RESOURCE `{}` CLOSE BY `{}` must name a native LINK function (alias::func).",
                    resource.name, resource.close_fn
                ),
                file,
                resource.line,
            );
            return;
        };
        let Some(link) = self.link_functions.get(alias) else {
            self.report(
                "RESOURCE_CLOSE_NOT_NATIVE",
                &format!(
                    "RESOURCE `{}` CLOSE BY `{}` references unknown LINK alias `{alias}`.",
                    resource.name, resource.close_fn
                ),
                file,
                resource.line,
            );
            return;
        };
        let Some(close_sig) = link.get(func) else {
            self.report(
                "RESOURCE_CLOSE_MISSING",
                &format!(
                    "RESOURCE `{}` CLOSE BY `{}` names a function not declared in LINK `{alias}`.",
                    resource.name, resource.close_fn
                ),
                file,
                resource.line,
            );
            return;
        };
        // The close op must consume exactly one parameter of this resource. A
        // parameter is a resource by its type (bug-976), so `param_resource` holds
        // whenever the type names a resource; it is checked alongside the type so
        // the rule reads exactly as it did when the flag was the `RES` keyword.
        let single_resource_param = close_sig.params.len() == 1
            && close_sig.param_resource.first().copied().unwrap_or(false)
            && close_sig
                .params
                .first()
                .and_then(|param| param.as_ref())
                .is_some_and(|param| resource_base_type(param).is_named(&resource.name));
        if !single_resource_param {
            self.report(
                "RESOURCE_CLOSE_SIGNATURE",
                &format!(
                    "Close op `{}` for resource `{}` must take exactly one parameter, of type `{}`.",
                    resource.close_fn, resource.name, resource.name
                ),
                file,
                close_sig.line,
            );
        }
        let _ = imports;
    }

    fn resolve_func_alias(
        &mut self,
        file: &HirFile,
        alias: &crate::ast::FuncAlias,
        imports: &HashMap<String, String>,
    ) {
        // The alias target must be a known LINK function (plan-link-update.md §5a).
        if self.link_target_signature(&alias.target).is_none() {
            self.report(
                "SYMBOL_UNKNOWN_IDENTIFIER",
                &format!(
                    "Function alias `{}` targets `{}`, which is not a native LINK function.",
                    alias.name, alias.target
                ),
                file,
                alias.line,
            );
        }
        let _ = imports;
    }

    fn resolve_link_block(
        &mut self,
        file: &HirFile,
        link: &crate::hir::HirLinkBlock,
        imports: &HashMap<String, String>,
    ) {
        // plan-111-B: a `LINK` block used to be the one node HIR kept as the
        // verbatim AST struct, so its parameter spellings entered the type
        // domain HERE. HIR elaborates it now (`hir::elaborate_link_block`), so
        // this reads types.
        for function in &link.functions {
            let parameters = function
                .params
                .iter()
                .map(|param| param.name.clone())
                .collect::<HashSet<_>>();
            for param in &function.params {
                // plan-136-A: a LINK parameter default resolves at the declaration,
                // exactly like a FUNC one. It was never resolved before, so an
                // unknown name in it went unreported.
                if let Some(default) = &param.default {
                    self.resolve_parameter_default(
                        file,
                        &param.name,
                        &parameters,
                        default,
                        param.line,
                        imports,
                    );
                }
                if let Some(type_) = &param.type_ {
                    // A raw C ABI type in a wrapper signature is reported by
                    // the former source checker as NATIVE_CPTR_ESCAPE; don't double-report it here
                    // as an unknown type.
                    if is_c_abi_type(type_) {
                        continue;
                    }
                    self.resolve_type(file, type_, param.line, imports);
                }
                // bug-595: a LINK parameter's `STATE S` is carried apart from its type.
                if let Some(state_type) = &param.state_type {
                    self.resolve_type(file, state_type, param.line, imports);
                }
            }
            if let Some(return_type) = &function.return_type {
                if !is_c_abi_type(return_type) {
                    self.resolve_type(file, return_type, function.line, imports);
                }
            }
            if let Some(state_type) = &function.return_state_type {
                self.resolve_type(file, state_type, function.line, imports);
            }
        }
    }

    fn resolve_binding(
        &mut self,
        file: &HirFile,
        binding: &HirTopLevelBinding,
        imports: &HashMap<String, String>,
    ) {
        self.resolve_type(file, &binding.type_, binding.line, imports);
        // bug-595: the STATE clause is carried apart from `type_`.
        if let Some(state_type) = &binding.state_type {
            self.resolve_type(file, state_type, binding.line, imports);
        }
        let locals = HashMap::new();
        if let Some(value) = &binding.value {
            self.resolve_expression(file, value, binding.line, imports, &locals);
        }
    }

    fn resolve_type_decl(
        &mut self,
        file: &HirFile,
        type_decl: &HirTypeDecl,
        imports: &HashMap<String, String>,
    ) {
        let previous_template_params = std::mem::replace(
            &mut self.active_template_params,
            type_decl.template_params.iter().cloned().collect(),
        );
        match type_decl.kind {
            TypeDeclKind::Type => {
                let mut names = HashMap::new();
                for field in &type_decl.fields {
                    self.resolve_member_field(file, field, imports);
                    if let Some(previous) = names.insert(field.name.clone(), field.line) {
                        self.report(
                            "TYPE_DUPLICATE_FIELD",
                            &format!(
                                "Field `{}` in TYPE `{}` was already declared on line {}.",
                                field.name, type_decl.name, previous
                            ),
                            file,
                            field.line,
                        );
                    }
                }
            }
            TypeDeclKind::Union => {
                for include in &type_decl.includes {
                    self.resolve_type(file, include, type_decl.line, imports);
                }

                let mut variants = HashMap::new();
                // bug-680: a CASE names a member by its type name, and for a
                // template instantiation that is the template's name — so two
                // instantiations of one template (`Box OF Integer`, `Box OF
                // String`) could never be told apart by a CASE.
                let mut templates = HashMap::new();
                for variant in &type_decl.variants {
                    if let Some(previous) = variants.insert(variant.type_.clone(), variant.line) {
                        self.report(
                            "TYPE_DUPLICATE_VARIANT",
                            &format!(
                                "Member type `{}` in UNION `{}` was already declared on line {}.",
                                variant.type_.name(),
                                type_decl.name,
                                previous
                            ),
                            file,
                            variant.line,
                        );
                    } else if let ParameterType::UserOf(template, _) = &variant.type_ {
                        if let Some(previous) = templates.insert(*template, variant.line) {
                            self.report(
                                "TYPE_DUPLICATE_VARIANT",
                                &format!(
                                    "Member type `{}` in UNION `{}` instantiates template `{}` \
                                     again (line {}); a CASE names a member by its type name, \
                                     so a union may carry only one instantiation of a template.",
                                    variant.type_.name(),
                                    type_decl.name,
                                    template.resolve(),
                                    previous
                                ),
                                file,
                                variant.line,
                            );
                        }
                    }
                    self.resolve_type(file, &variant.type_, variant.line, imports);
                }
            }
            TypeDeclKind::Enum => {
                let mut members = HashMap::new();
                for member in &type_decl.members {
                    if let Some(previous) = members.insert(member.name.clone(), member.line) {
                        self.report(
                            "TYPE_DUPLICATE_ENUM_MEMBER",
                            &format!(
                                "Member `{}` in ENUM `{}` was already declared on line {}.",
                                member.name, type_decl.name, previous
                            ),
                            file,
                            member.line,
                        );
                    }
                }
            }
        }
        self.active_template_params = previous_template_params;
    }

    fn resolve_member_field(
        &mut self,
        file: &HirFile,
        field: &HirTypeField,
        imports: &HashMap<String, String>,
    ) {
        self.resolve_type(file, &field.type_, field.line, imports);
    }

    fn resolve_function(
        &mut self,
        file: &HirFile,
        function: &HirFunction,
        imports: &HashMap<String, String>,
    ) {
        let previous_template_params = std::mem::replace(
            &mut self.active_template_params,
            function.template_params.iter().cloned().collect(),
        );
        let previous_internal_origin = std::mem::replace(
            &mut self.resolving_internal_origin,
            function.internal_origin,
        );
        let mut locals = HashMap::new();
        let parameters = function
            .params
            .iter()
            .map(|param| param.name.clone())
            .collect::<HashSet<_>>();

        for param in &function.params {
            self.check_new_local(
                file,
                &param.name,
                param.line,
                &locals,
                format!(
                    "Parameter `{}` is already declared in this function.",
                    param.name
                ),
            );
            locals.insert(
                param.name.clone(),
                Symbol {
                    file_path: file.path.clone(),
                    line: param.line,
                    visibility: Visibility::Private,
                },
            );

            self.resolve_type(file, &param.type_, param.line, imports);
            // bug-595: `RES p AS T STATE S` carries `S` apart from `type_`. Before
            // this, an unknown `S` on a parameter was not reported at all.
            if let Some(state_type) = &param.state_type {
                self.resolve_type(file, state_type, param.line, imports);
            }

            if let Some(default) = &param.default {
                self.resolve_parameter_default(
                    file,
                    &param.name,
                    &parameters,
                    default,
                    param.line,
                    imports,
                );
            }
        }

        self.resolve_type(file, &function.returns, function.line, imports);
        // bug-595: likewise a return's `STATE S`.
        if let Some(state_type) = &function.return_state_type {
            self.resolve_type(file, state_type, function.line, imports);
        }

        self.resolve_block(file, &function.body, imports, &mut locals);
        if let Some(trap) = &function.trap {
            self.check_new_local(
                file,
                &trap.name,
                trap.line,
                &locals,
                format!(
                    "Local binding `{}` is already declared in this function.",
                    trap.name
                ),
            );
            let mut trap_locals = locals.clone();
            trap_locals.insert(
                trap.name.clone(),
                Symbol {
                    file_path: file.path.clone(),
                    line: trap.line,
                    visibility: Visibility::Private,
                },
            );
            self.resolve_block(file, &trap.body, imports, &mut trap_locals);
        }
        self.active_template_params = previous_template_params;
        self.resolving_internal_origin = previous_internal_origin;
    }

    fn resolve_block(
        &mut self,
        file: &HirFile,
        body: &[HirStatement],
        imports: &HashMap<String, String>,
        locals: &mut HashMap<String, Symbol>,
    ) {
        for statement in body {
            self.resolve_statement(file, statement, imports, locals);
        }
    }

    fn resolve_statement(
        &mut self,
        file: &HirFile,
        statement: &HirStatement,
        imports: &HashMap<String, String>,
        locals: &mut HashMap<String, Symbol>,
    ) {
        match statement {
            HirStatement::Let {
                name,
                type_,
                state_type,
                value,
                line,
                ..
            } => {
                self.resolve_type(file, type_, *line, imports);
                // bug-595: a `RES … STATE T` binding carries `T` apart from `type_`;
                // it is a type position like any other and must resolve, or an
                // unknown or unqualified STATE name escapes to the verifier as an
                // unlocated `TYPE_STATE_INVALID` or a misleading `TYPE_STATE_MISMATCH`.
                if let Some(state_type) = state_type {
                    self.resolve_type(file, state_type, *line, imports);
                }
                if let Some(value) = value {
                    self.resolve_expression(file, value, *line, imports, locals);
                }
                self.check_new_local(
                    file,
                    name,
                    *line,
                    locals,
                    format!("Local binding `{name}` is already declared in this function."),
                );
                locals.insert(
                    name.clone(),
                    Symbol {
                        file_path: file.path.clone(),
                        line: *line,
                        visibility: Visibility::Private,
                    },
                );
            }
            HirStatement::Return { value, line } => {
                if let Some(value) = value {
                    self.resolve_expression(file, value, *line, imports, locals);
                }
            }
            HirStatement::Exit { code, line, .. } => {
                if let Some(code) = code {
                    self.resolve_expression(file, code, *line, imports, locals);
                }
            }
            HirStatement::Continue { .. } => {}
            HirStatement::Fail { error, line } => {
                self.resolve_expression(file, error, *line, imports, locals);
            }
            HirStatement::Propagate { .. } => {}
            HirStatement::Recover { value, line } => {
                if let Some(value) = value {
                    self.resolve_expression(file, value, *line, imports, locals);
                }
            }
            HirStatement::Assign { name, value, line } => {
                self.resolve_identifier(file, name, *line, imports, locals);
                self.resolve_expression(file, value, *line, imports, locals);
            }
            HirStatement::StateAssign {
                resource,
                value,
                line,
            } => {
                self.resolve_identifier(file, resource, *line, imports, locals);
                self.resolve_expression(file, value, *line, imports, locals);
            }
            HirStatement::Expression { expression, line } => {
                self.resolve_expression(file, expression, *line, imports, locals);
            }
            HirStatement::If {
                condition,
                then_body,
                else_body,
                line,
            } => {
                self.resolve_expression(file, condition, *line, imports, locals);
                self.resolve_nested_block(file, then_body, imports, locals);
                self.resolve_nested_block(file, else_body, imports, locals);
            }
            HirStatement::Match {
                expression,
                cases,
                line,
            } => {
                self.resolve_expression(file, expression, *line, imports, locals);
                for case in cases {
                    self.resolve_match_pattern(file, &case.pattern, case.line, imports, locals);
                    // One check per pattern binding: the same name is inserted into
                    // the guard's scope (when there is a `WHEN`) and the body's, so
                    // checking at either insert would report a guarded case twice.
                    if let HirMatchPattern::Union { binding, .. } = &case.pattern {
                        self.check_new_local(
                            file,
                            binding,
                            case.line,
                            locals,
                            format!(
                                "Local binding `{binding}` is already declared in this function."
                            ),
                        );
                    }
                    if let Some(guard) = &case.guard {
                        let mut guard_locals = locals.clone();
                        if let HirMatchPattern::Union { binding, .. } = &case.pattern {
                            guard_locals.insert(
                                binding.clone(),
                                Symbol {
                                    file_path: file.path.clone(),
                                    line: case.line,
                                    visibility: Visibility::Private,
                                },
                            );
                        }
                        self.resolve_expression(file, guard, case.line, imports, &guard_locals);
                    }
                    let mut case_locals = locals.clone();
                    if let HirMatchPattern::Union { binding, .. } = &case.pattern {
                        case_locals.insert(
                            binding.clone(),
                            Symbol {
                                file_path: file.path.clone(),
                                line: case.line,
                                visibility: Visibility::Private,
                            },
                        );
                    }
                    self.resolve_block(file, &case.body, imports, &mut case_locals);
                }
            }
            HirStatement::For {
                name,
                start,
                end,
                step,
                body,
                line,
            } => {
                self.resolve_expression(file, start, *line, imports, locals);
                self.resolve_expression(file, end, *line, imports, locals);
                if let Some(step) = step {
                    self.resolve_expression(file, step, *line, imports, locals);
                }
                self.check_new_local(
                    file,
                    name,
                    *line,
                    locals,
                    format!("Local binding `{name}` is already declared in this function."),
                );
                let mut nested = locals.clone();
                nested.insert(
                    name.clone(),
                    Symbol {
                        file_path: file.path.clone(),
                        line: *line,
                        visibility: Visibility::Private,
                    },
                );
                self.resolve_block(file, body, imports, &mut nested);
            }
            HirStatement::ForEach {
                name,
                iterable,
                body,
                line,
            } => {
                self.resolve_expression(file, iterable, *line, imports, locals);
                self.check_new_local(
                    file,
                    name,
                    *line,
                    locals,
                    format!("Local binding `{name}` is already declared in this function."),
                );
                let mut nested = locals.clone();
                nested.insert(
                    name.clone(),
                    Symbol {
                        file_path: file.path.clone(),
                        line: *line,
                        visibility: Visibility::Private,
                    },
                );
                self.resolve_block(file, body, imports, &mut nested);
            }
            HirStatement::While {
                kind: _,
                condition,
                body,
                line,
            } => {
                self.resolve_expression(file, condition, *line, imports, locals);
                self.resolve_nested_block(file, body, imports, locals);
            }
            HirStatement::DoUntil {
                body,
                condition,
                line,
            } => {
                self.resolve_nested_block(file, body, imports, locals);
                self.resolve_expression(file, condition, *line, imports, locals);
            }
        }
    }

    fn resolve_nested_block(
        &mut self,
        file: &HirFile,
        body: &[HirStatement],
        imports: &HashMap<String, String>,
        locals: &HashMap<String, Symbol>,
    ) {
        let mut nested = locals.clone();
        self.resolve_block(file, body, imports, &mut nested);
    }

    fn resolve_match_pattern(
        &mut self,
        file: &HirFile,
        pattern: &HirMatchPattern,
        line: usize,
        imports: &HashMap<String, String>,
        locals: &HashMap<String, Symbol>,
    ) {
        match pattern {
            HirMatchPattern::Else => {}
            HirMatchPattern::Literal(pattern) => {
                self.resolve_expression(file, pattern, line, imports, locals);
            }
            HirMatchPattern::Union { type_, .. } => {
                self.resolve_type(file, type_, line, imports);
            }
            HirMatchPattern::OneOf(patterns) => {
                for pattern in patterns {
                    self.resolve_expression(file, pattern, line, imports, locals);
                }
            }
        }
    }

    fn resolve_expression(
        &mut self,
        file: &HirFile,
        expression: &HirExpression,
        line: usize,
        imports: &HashMap<String, String>,
        locals: &HashMap<String, Symbol>,
    ) {
        match expression {
            HirExpression::String(_)
            | HirExpression::Number(_)
            | HirExpression::Scalar(_)
            | HirExpression::Boolean(_) => {}
            HirExpression::Binary { left, right, .. } => {
                self.resolve_expression(file, left, line, imports, locals);
                self.resolve_expression(file, right, line, imports, locals);
            }
            HirExpression::Unary { operand, .. } => {
                self.resolve_expression(file, operand, line, imports, locals);
            }
            HirExpression::Call {
                callee, arguments, ..
            } => {
                self.resolve_callable(file, callee, line, imports, locals);
                for argument in arguments {
                    self.resolve_expression(file, call_arg_value(argument), line, imports, locals);
                }
            }
            HirExpression::Lambda { params, body, .. } => {
                let mut lambda_locals = locals.clone();
                for param in params {
                    self.resolve_type(file, &param.type_, param.line, imports);
                    // bug-595: a lambda's `RES p AS T STATE S` carries `S` apart.
                    if let Some(state_type) = &param.state_type {
                        self.resolve_type(file, state_type, param.line, imports);
                    }
                    self.check_new_local(
                        file,
                        &param.name,
                        param.line,
                        &lambda_locals,
                        format!(
                            "Lambda parameter `{}` is already declared in this function.",
                            param.name
                        ),
                    );
                    lambda_locals.insert(
                        param.name.clone(),
                        Symbol {
                            file_path: file.path.clone(),
                            line: param.line,
                            visibility: Visibility::Private,
                        },
                    );
                    // plan-136-A: a lambda is called through a function value, which
                    // carries no defaults, so a default here could never be used.
                    if param.default.is_some() {
                        self.report(
                            "SYMBOL_LAMBDA_PARAMETER_DEFAULT",
                            &format!(
                                "Lambda parameter `{}` cannot declare a default value.",
                                crate::internal_name::display_name(&param.name)
                            ),
                            file,
                            param.line,
                        );
                    }
                }
                self.resolve_expression(file, body, line, imports, &lambda_locals);
            }
            HirExpression::Constructor { type_, arguments } => {
                // `Error`/`Err` are the error-model constructors, not declared
                // types.
                if !["Error", "Err"].iter().any(|name| type_.is_named(name)) {
                    self.resolve_type(file, type_, line, imports);
                }
                for argument in arguments {
                    self.resolve_expression(
                        file,
                        constructor_arg_value(argument),
                        line,
                        imports,
                        locals,
                    );
                }
            }
            HirExpression::WithUpdate { target, updates } => {
                self.resolve_expression(file, target, line, imports, locals);
                for update in updates {
                    self.resolve_expression(file, &update.value, update.line, imports, locals);
                }
            }
            HirExpression::ListLiteral(values) => {
                for value in values {
                    self.resolve_expression(file, value, line, imports, locals);
                }
            }
            HirExpression::SetLiteral {
                element_type,
                elements,
            } => {
                self.resolve_type(file, peel_res(element_type), line, imports);
                for value in elements {
                    self.resolve_expression(file, value, line, imports, locals);
                }
            }
            HirExpression::MapLiteral {
                key_type,
                value_type,
                entries,
            } => {
                self.resolve_type(file, key_type, line, imports);
                // A `Map OF K TO RES fs::File` literal value carries the resource
                // ownership-axis marker (§15.6); resolve the underlying type.
                self.resolve_type(file, peel_res(value_type), line, imports);
                for (key, value) in entries {
                    self.resolve_expression(file, key, line, imports, locals);
                    self.resolve_expression(file, value, line, imports, locals);
                }
            }
            HirExpression::MemberAccess { target, .. } => {
                if let HirExpression::Identifier(name) = target.as_ref() {
                    if self.types.contains(&ParameterType::declared(name)) {
                        return;
                    }
                }
                self.resolve_expression(file, target, line, imports, locals);
            }
            HirExpression::Trapped {
                expression,
                binding,
                handler,
                line: trap_line,
            } => {
                self.resolve_expression(file, expression, line, imports, locals);
                self.check_new_local(
                    file,
                    binding,
                    *trap_line,
                    locals,
                    format!("Local binding `{binding}` is already declared in this function."),
                );
                let mut handler_locals = locals.clone();
                handler_locals.insert(
                    binding.clone(),
                    Symbol {
                        file_path: file.path.clone(),
                        line: *trap_line,
                        visibility: Visibility::Private,
                    },
                );
                self.resolve_block(file, handler, imports, &mut handler_locals);
            }
            HirExpression::Identifier(name) if name == "NOTHING" => {}
            HirExpression::Identifier(name) => {
                self.resolve_identifier(file, name, line, imports, locals);
            }
        }
    }

    fn resolve_callable(
        &mut self,
        file: &HirFile,
        callee: &str,
        line: usize,
        imports: &HashMap<String, String>,
        locals: &HashMap<String, Symbol>,
    ) {
        if callee.contains('.') {
            self.resolve_package_qualified_name(file, callee, line, imports);
        } else if crate::codegen::builtins::general::is_general_call(callee) {
        } else if crate::codegen::builtins_testing::is_testing_call(callee) {
            // Assertion builtins are compiler-lowered; their arguments are
            // resolved by the caller. Placement (TCASE-only) is enforced earlier
            // by `crate::testing::validate_expect_placement`.
        } else if locals.contains_key(callee) {
        } else if !self.function_visible_in_file(file, callee)
            && !self.top_level_visible_in_file(file, callee)
        {
            if self.report_default_names_parameter(file, callee, line) {
                return;
            }
            // A top-level (global) binding holding a function value is callable,
            // exactly like a local binding; `resolve_identifier` already consults
            // `top_level_visible_in_file`, so value position works while call
            // position did not (bug-198). Defer the callability check to typecheck.
            self.report(
                "SYMBOL_UNKNOWN_IDENTIFIER",
                &format!("Callable `{callee}` is not a top-level function."),
                file,
                line,
            );
        }
    }

    /// Resolve a parameter default in its function's DECLARATION scope: no locals
    /// at all, so not the function's own parameters and never a caller's locals
    /// (plan-136-A). The file's top level — globals, functions, imports and
    /// own-file `PRIVATE` names — is what remains visible.
    fn resolve_parameter_default(
        &mut self,
        file: &HirFile,
        parameter: &str,
        parameters: &HashSet<String>,
        default: &HirExpression,
        line: usize,
        imports: &HashMap<String, String>,
    ) {
        let previous = self.default_scope.replace(DefaultScope {
            parameter: parameter.to_string(),
            parameters: parameters.clone(),
        });
        self.resolve_expression(file, default, line, imports, &HashMap::new());
        self.default_scope = previous;
    }

    /// While a default is being resolved, report an unresolved `name` that is a
    /// parameter of the same function as `SYMBOL_DEFAULT_NAMES_PARAMETER`, and
    /// return whether it did.
    fn report_default_names_parameter(&mut self, file: &HirFile, name: &str, line: usize) -> bool {
        let Some(scope) = &self.default_scope else {
            return false;
        };
        if !scope.parameters.contains(name) {
            return false;
        }
        let detail = format!(
            "The default value of `{}` cannot use parameter `{}`; a default is evaluated outside the function.",
            crate::internal_name::display_name(&scope.parameter),
            crate::internal_name::display_name(name)
        );
        self.report("SYMBOL_DEFAULT_NAMES_PARAMETER", &detail, file, line);
        true
    }

    /// The checks every site that introduces a local runs before it inserts
    /// `name` (plan-136-C). A name already live in `locals` is
    /// `SYMBOL_DUPLICATE_LOCAL`, reported with the site's own `duplicate` detail;
    /// otherwise a name that is a top-level `LET`/`MUT` visible from this file —
    /// `PUBLIC`/`EXPORT` anywhere in the project, or `PRIVATE` in this same file —
    /// is `SYMBOL_SHADOWS_TOP_LEVEL_BINDING`. A compiler-internal name, which no
    /// user can spell, is exempt from both; a built-in package's source file is
    /// exempt from the top-level rule, since the user's bindings are not its own.
    pub(super) fn check_new_local(
        &mut self,
        file: &HirFile,
        name: &str,
        line: usize,
        locals: &HashMap<String, Symbol>,
        duplicate: String,
    ) {
        if name.starts_with(crate::internal_name::INTERNAL_SIGIL) || name.starts_with('$') {
            return;
        }
        if locals.contains_key(name) {
            self.report("SYMBOL_DUPLICATE_LOCAL", &duplicate, file, line);
            return;
        }
        if file.internal || self.resolving_internal_origin {
            return;
        }
        let Some(keys) = self.top_level_bindings.get(name) else {
            return;
        };
        let shadowed = keys
            .iter()
            .filter_map(|key| self.top_levels.get(key))
            .find(|symbol| self.visible_from(file, symbol.visibility, &symbol.file_path))
            .cloned();
        if let Some(symbol) = shadowed {
            self.report(
                "SYMBOL_SHADOWS_TOP_LEVEL_BINDING",
                &format!(
                    "`{}` is already a top-level binding (declared at {}:{}); a local cannot reuse its name.",
                    crate::internal_name::display_name(name),
                    symbol.file_path,
                    symbol.line
                ),
                file,
                line,
            );
        }
    }

    fn resolve_identifier(
        &mut self,
        file: &HirFile,
        name: &str,
        line: usize,
        imports: &HashMap<String, String>,
        locals: &HashMap<String, Symbol>,
    ) {
        if name.contains('.') {
            self.resolve_package_qualified_name(file, name, line, imports);
        } else if !locals.contains_key(name)
            && !self.top_level_visible_in_file(file, name)
            && !self.function_visible_in_file(file, name)
            && !crate::codegen::builtins::general::is_general_call(name)
        {
            if self.report_default_names_parameter(file, name, line) {
                return;
            }
            self.report(
                "SYMBOL_UNKNOWN_IDENTIFIER",
                &format!("Identifier `{name}` is not declared in this scope."),
                file,
                line,
            );
        }
    }

    /// Resolve a type spelling that HIR still stores as a `String`.
    ///
    /// Resolve every nominal reachable from `type_`, structurally.
    ///
    /// This replaces `resolve_type_name(&str)`, which was 120 lines of grammar
    /// re-implementation — `strip_type_group`, `starts_with("Result OF ")`,
    /// `strip_prefix("FUNC(")` + `split_func_params_and_return`,
    /// `thread_parts_full`, `state_type_name`/`base_resource_name`, and four
    /// `ParameterType::parse` arms that immediately re-rendered each child with
    /// `.name()` so the recursion could go back through a string. Every one of
    /// those decisions is a variant here.
    ///
    /// The leaf tail is still a NAME lookup, and that is not a residue of the old
    /// design: a nominal's identity *is* its name, and `self.types` is a symbol
    /// table. What is gone is deriving a type's SHAPE from its spelling.
    fn resolve_type(
        &mut self,
        file: &HirFile,
        type_: &ParameterType,
        line: usize,
        imports: &HashMap<String, String>,
    ) {
        match type_ {
            // plan-114-D: a `RES`-marked type in its OWN right — a record field,
            // `handle AS RES fs::File`. The marker rides on the field and names a
            // type that must still be resolved, exactly as a collection element's
            // does below.
            //
            // Before this arm, a `Res` reaching the top level fell through to the
            // leaf tail, which sees a name containing `.` and calls
            // `resolve_package_qualified_name` on the WHOLE spelling — reporting
            // `SYMBOL_UNKNOWN_IMPORT` for a package named `RES fs`. The same
            // failure shape as bug-463, and the same cause: a `ParameterType`
            // variant that a structural walk does not have an arm for is silent
            // until some spelling reaches it.
            ParameterType::Res(inner) => {
                self.resolve_type(file, inner, line, imports);
            }
            // A `List OF RES fs::File` element carries the resource ownership-axis
            // marker (§15.6); the marker rides on the element and names a type that
            // must still be resolved.
            ParameterType::ListOf(element) => {
                self.resolve_type(file, peel_res(element), line, imports);
            }
            // `Set OF T` (plan-63): a single element type, no `RES`, no `TO`. A
            // `RES`-marked element in this position is not peeled — it reaches the
            // leaf tail and is reported, exactly as the string cascade left it.
            // The internal `#Result OF T` (bug-945: no user spelling reaches it)
            // resolves its success type like any single-child shape.
            ParameterType::SetOf(element) | ParameterType::ResultOf(element) => {
                self.resolve_type(file, element, line, imports);
            }
            ParameterType::MapOf(key, value) => {
                self.resolve_type(file, key, line, imports);
                self.resolve_type(file, peel_res(value), line, imports);
            }
            ParameterType::MapEntryOf(key, value) => {
                self.resolve_type(file, key, line, imports);
                self.resolve_type(file, value, line, imports);
            }
            ParameterType::Func(params, returns, _isolated) => {
                for param in params {
                    self.resolve_type(file, param, line, imports);
                }
                self.resolve_type(file, returns, line, imports);
            }
            // A parametric thread type's three planes. An absent resource plane is
            // `Nothing`, whose arm below is a no-op — so this resolves exactly the
            // planes the old `thread_parts_full` handed back.
            ParameterType::ThreadHandle { msg, res, out, .. } => {
                self.resolve_type(file, msg, line, imports);
                self.resolve_type(file, res, line, imports);
                self.resolve_type(file, out, line, imports);
            }
            // A user generic (`Stack OF T`, `Pair OF A, B`). An unknown head is
            // deliberately NOT consumed: it falls to the unknown-type report, which
            // names the whole spelling.
            ParameterType::UserOf(head, args) => {
                let head = head.resolve();
                if self.types.contains(&ParameterType::declared(head))
                    || self.active_template_params.contains(head)
                {
                    for arg in args {
                        self.resolve_type(file, arg, line, imports);
                    }
                } else {
                    self.report_unknown_type(file, &type_.name(), line);
                }
            }
            // The absent annotation.
            ParameterType::Unknown => {}
            // A type variable in a generic signature. `hir::elaborate` classified it
            // from the enclosing declaration's `template_params`, which is the same
            // set `active_template_params` holds — so the membership check agrees
            // with the classification, and a stray variable is still reported.
            ParameterType::Var(name) => {
                let name = name.resolve();
                if !self.active_template_params.contains(name) {
                    self.report_unknown_type(file, name, line);
                }
            }
            // A leaf: a scalar, a nominal, or a `RES`-marked type in a position the
            // grammar does not allow one. All three resolve by NAME, which is what
            // the string cascade's tail did after every structural peel.
            _ => self.resolve_leaf(file, type_, line, imports),
        }
    }

    /// The leaf tail: a `STATE` clause, then the nominal itself.
    fn resolve_leaf(
        &mut self,
        file: &HirFile,
        type_: &ParameterType,
        line: usize,
        imports: &HashMap<String, String>,
    ) {
        // A resource type carrying `STATE T` (plan-52/54): resolve the base resource
        // type and the STATE payload independently — `fs::File STATE Cursor` is not
        // one symbol. Reached only after the compound arms above have peeled any
        // enclosing `Thread`/`List`/`Map` (whose own spellings also contain
        // ` STATE `), so this fires on the bare resource element: a stateful
        // binding/return, or a thread plane's `RES` element (plan-54).
        //
        // `split_state` is TOP-LEVEL only, which is what keeps a composite like
        // `List OF RES Handle STATE Cursor` splitting at its element rather than at
        // the container (bug-429).
        let (base, state) = type_.split_state();
        if let Some(state) = state {
            self.resolve_type(file, &base, line, imports);
            self.resolve_type(file, &state, line, imports);
            return;
        }

        // plan-111-B: `Unknown` is a nominal here — the `ParameterType::Unknown`
        // variant has its own arm above, so a spelling reaching this leaf tail
        // is a decoded-IR `Named("Unknown")`. The template-parameter membership
        // is a NAME set (the declaring scope's parameter list), so it renders.
        let name = type_.name();
        let name = name.as_ref();
        if type_.is_named("Unknown") || self.active_template_params.contains(name) {
            return;
        }
        if let Some((root, _)) = name.split_once('.') {
            // bug-632: an imported user package's type, already canonicalized to
            // `<package>.<Name>` by the parser (`IMPORT ov AS o` / `o::A` arrives
            // as `ov.A`). It resolves only where this file imports that package.
            if imports.values().any(|package| package == root) && self.types.contains(type_) {
                return;
            }
            self.resolve_package_qualified_name(file, name, line, imports);
        } else if !self.types.contains(&ParameterType::declared(name)) {
            self.report_unknown_type(file, name, line);
        }
    }

    fn report_unknown_type(&mut self, file: &HirFile, name: &str, line: usize) {
        self.report(
            "SYMBOL_UNKNOWN_TYPE",
            &format!("Type `{name}` is not a built-in or top-level project type."),
            file,
            line,
        );
    }

    fn resolve_package_qualified_name(
        &mut self,
        file: &HirFile,
        name: &str,
        line: usize,
        imports: &HashMap<String, String>,
    ) {
        let root = name.split('.').next().unwrap_or(name);
        // bug-480 Phase 4b: a built-in package's injected companion refers to its
        // OWN package-qualified types (`http.Response` inside `builtins/http.mfb`).
        // That is a self-reference, not an import — the file IS the package — so it
        // neither needs nor can have an `IMPORT http` line. Same rule `IMPORT self`
        // encodes for a source package; without it every companion that names one
        // of its own value types reports SYMBOL_UNKNOWN_IMPORT against a file the
        // developer never wrote.
        if file
            .path
            .strip_prefix("builtins/")
            .and_then(|rest| rest.strip_suffix(".mfb"))
            == Some(root)
        {
            return;
        }
        // A LINK alias is a package-local namespace, not an import: resolve its
        // members against the block's native functions (plan-link-update.md §5b).
        if let Some(link) = self.link_functions.get(root) {
            let member = name.split_once('.').map(|(_, rest)| rest).unwrap_or("");
            if !link.contains_key(member) {
                self.report(
                    "SYMBOL_UNKNOWN_IDENTIFIER",
                    &format!("LINK `{root}` does not declare a native function `{member}`."),
                    file,
                    line,
                );
            }
            return;
        }
        let Some(package) = imports.get(root) else {
            self.report(
                "SYMBOL_UNKNOWN_IMPORT",
                &format!("Package `{root}` is used but not imported in this file."),
                file,
                line,
            );
            return;
        };

        if builtins::is_builtin_import(package) {
            let qualified_name = qualify_package_name(name, root, package);
            // A package-qualified built-in type (`net::Url`, `http::Result`)
            // resolves to its bare internal id (plan-03-http.md §A.1/§B.2).
            if builtins::qualified_builtin_type(&qualified_name).is_some() {
                return;
            }
            // An internal-only member resolves for the injected package glue that
            // needs it and is invisible to user source, which is what makes it
            // internal (bug-337-D9).
            let internal_only = builtins::is_internal_only_call(&qualified_name) && !file.internal;
            if internal_only || !builtins::is_builtin_member(&qualified_name) {
                self.report(
                    "SYMBOL_UNKNOWN_IDENTIFIER",
                    &format!("Built-in package `{package}` does not export `{qualified_name}`."),
                    file,
                    line,
                );
            }
            return;
        }

        // bug-480: the same gate for an imported USER package. Without it a
        // `pkg::member` naming nothing the package exports typed as `Unknown` and
        // was reported hundreds of lines away as `TYPE_CALL_ARGUMENT_MISMATCH ...
        // argument type(s) (Unknown)` against whatever consumed it, naming
        // neither the package nor the member.
        //
        // Only a package whose interface was READ contributes an entry, so an
        // unreadable or not-yet-installed dependency still resolves silently here
        // and is reported by the import gate that saw the real reason. `IMPORT
        // self` never records one either: a package's own members are ordinary
        // top-level declarations, resolved by the bare-name path.
        let Some(exports) = self.package_exports.get(package) else {
            return;
        };
        let Some((_, member)) = name.split_once('.') else {
            return;
        };
        // The rule governs the HEAD of a path, not field selection: `pkg::rec.field`
        // resolves `rec` against the package and leaves `.field` to the type
        // checker.
        let head = member.split('.').next().unwrap_or(member);
        if exports.contains(member) || exports.contains(head) {
            return;
        }
        self.report(
            "SYMBOL_UNKNOWN_IDENTIFIER",
            &format!("Package `{package}` does not export `{head}`."),
            file,
            line,
        );
    }
}

/// A collection element/value with its `RES` ownership-axis marker peeled off
/// (§15.6). The marker rides on the element, not the container, and wraps a type
/// that must still be resolved — `List OF RES fs::File` resolves `fs::File`.
///
/// plan-106-D: was `strip_res(&str) -> String`, which round-tripped the element
/// through `parse` and `name()` to drop one variant. Peeling the variant is the
/// whole operation.
fn peel_res(type_: &ParameterType) -> &ParameterType {
    match type_ {
        ParameterType::Res(inner) => inner,
        other => other,
    }
}

#[cfg(test)]
mod tests;
