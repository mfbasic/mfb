use super::*;

impl Resolver<'_> {
    pub(super) fn resolve(&mut self) {
        for file in &self.ast.files {
            self.resolve_file(file);
        }
    }

    /// Validate every `DOC` block in the package (plan-09-doc.md §2/§4): resolve
    /// each header to a declaration of the right kind, then check the body's
    /// `ARG`/`PROP`/`RET`/`ERROR`/`EXAMPLE` lines, attributes, and `DEPRECATED`
    /// against that declaration.
    pub(super) fn resolve_doc_blocks(&mut self) {
        let ast = self.ast;

        // Index user declarations by name. Functions and subs share a namespace,
        // and a name may carry several overloads.
        let mut funcs: HashMap<&str, Vec<&Function>> = HashMap::new();
        let mut types: HashMap<&str, &TypeDecl> = HashMap::new();
        for file in &ast.files {
            for item in &file.items {
                match item {
                    Item::Function(function) => {
                        funcs
                            .entry(function.name.as_str())
                            .or_default()
                            .push(function);
                    }
                    Item::Type(type_decl) => {
                        types.entry(type_decl.name.as_str()).or_insert(type_decl);
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
        for file in &ast.files {
            for item in &file.items {
                let Item::Doc(doc) = item else {
                    continue;
                };
                self.validate_doc_block(
                    file,
                    doc,
                    &funcs,
                    &types,
                    &mut seen,
                    &mut package_doc_seen,
                );
            }
        }
    }

    fn validate_doc_block(
        &mut self,
        file: &AstFile,
        doc: &DocBlock,
        funcs: &HashMap<&str, Vec<&Function>>,
        types: &HashMap<&str, &TypeDecl>,
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
        let is_callable = matches!(kind, DocHeaderKind::Func | DocHeaderKind::Sub);
        let is_member_kind = matches!(
            kind,
            DocHeaderKind::Type | DocHeaderKind::Union | DocHeaderKind::Enum
        );

        // --- Context restrictions ---
        if !is_callable {
            if let Some(arg) = doc.args.first() {
                self.report(
                    "DOC_ARG_INVALID_CONTEXT",
                    "ARG is valid only on FUNC and SUB doc blocks.",
                    file,
                    arg.line,
                );
            }
            if let Some((_, line)) = doc.rets.first() {
                self.report(
                    "DOC_RET_INVALID_CONTEXT",
                    "RET is valid only on FUNC and SUB doc blocks.",
                    file,
                    *line,
                );
            }
            if let Some(error) = doc.errors.first() {
                self.report(
                    "DOC_ERROR_INVALID_CONTEXT",
                    "ERROR is valid only on FUNC and SUB doc blocks.",
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
                    "GROUP is valid only on FUNC and SUB doc blocks.",
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
            DocHeaderKind::Func | DocHeaderKind::Sub => {
                let want_sub = kind == DocHeaderKind::Sub;
                let resolved = match funcs.get(doc.header_name.as_str()) {
                    Some(list) => {
                        let matching: Vec<&&Function> = list
                            .iter()
                            .filter(|f| (f.kind == FunctionKind::Sub) == want_sub)
                            .collect();
                        if matching.is_empty() {
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
                        } else if let Some(types_wanted) = &doc.header_params {
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
                let resolved = match types.get(doc.header_name.as_str()) {
                    Some(type_decl) if type_decl.kind == want => {
                        let valid: HashSet<&str> = match want {
                            TypeDeclKind::Type => {
                                type_decl.fields.iter().map(|f| f.name.as_str()).collect()
                            }
                            TypeDeclKind::Union => {
                                type_decl.variants.iter().map(|v| v.name.as_str()).collect()
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
                        let rule = if funcs.contains_key(doc.header_name.as_str()) {
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
        }
    }

    /// (See free function `overload_types_match`.)
    ///
    /// Record a successfully resolved doc target and flag a second block naming
    /// the same declaration (same overload signature) as `DOC_DUPLICATE`.
    fn note_doc_target(
        &mut self,
        file: &AstFile,
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
        file: &AstFile,
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

    fn resolve_file(&mut self, file: &AstFile) {
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
                Item::Binding(binding) => self.resolve_binding(file, binding, &imports),
                Item::Function(function) => self.resolve_function(file, function, &imports),
                Item::Type(type_decl) => self.resolve_type_decl(file, type_decl, &imports),
                Item::Resource(resource) => self.resolve_resource_decl(file, resource, &imports),
                Item::FuncAlias(alias) => self.resolve_func_alias(file, alias, &imports),
                Item::Link(link) => self.resolve_link_block(file, link, &imports),
                // DOC blocks are validated package-wide in `resolve_doc_blocks`.
                Item::Doc(_) => {}
                // TESTING blocks are lowered away before resolution (plan-18-A §3).
                Item::Testing(_) => {}
            }
        }
    }

    fn resolve_resource_decl(
        &mut self,
        file: &AstFile,
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
        // The close op must consume exactly one `RES` parameter of this resource.
        let single_resource_param = close_sig.params.len() == 1
            && close_sig.param_resource.first().copied().unwrap_or(false)
            && close_sig
                .params
                .first()
                .and_then(|param| param.as_deref())
                .is_some_and(|param| resource_base_type(param) == resource.name);
        if !single_resource_param {
            self.report(
                "RESOURCE_CLOSE_SIGNATURE",
                &format!(
                    "Close op `{}` for resource `{}` must take exactly one `RES {} ...` parameter.",
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
        file: &AstFile,
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
        file: &AstFile,
        link: &crate::ast::LinkBlock,
        imports: &HashMap<String, String>,
    ) {
        for function in &link.functions {
            for param in &function.params {
                if let Some(type_name) = &param.type_name {
                    // A raw C ABI type in a wrapper signature is reported by
                    // syntaxcheck as NATIVE_CPTR_ESCAPE; don't double-report it here
                    // as an unknown type.
                    if is_c_abi_type(type_name) {
                        continue;
                    }
                    self.resolve_type_name(file, type_name, param.line, imports);
                }
            }
            if let Some(return_type) = &function.return_type {
                if !is_c_abi_type(return_type) {
                    self.resolve_type_name(file, return_type, function.line, imports);
                }
            }
        }
    }

    fn resolve_binding(
        &mut self,
        file: &AstFile,
        binding: &TopLevelBinding,
        imports: &HashMap<String, String>,
    ) {
        if let Some(type_name) = &binding.type_name {
            self.resolve_type_name(file, type_name, binding.line, imports);
        }
        let locals = HashMap::new();
        if let Some(value) = &binding.value {
            self.resolve_expression(file, value, binding.line, imports, &locals);
        }
    }

    fn resolve_type_decl(
        &mut self,
        file: &AstFile,
        type_decl: &TypeDecl,
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
                    self.resolve_type_name(file, include, type_decl.line, imports);
                }

                let mut variants = HashMap::new();
                for variant in &type_decl.variants {
                    if let Some(previous) = variants.insert(variant.name.clone(), variant.line) {
                        self.report(
                            "TYPE_DUPLICATE_VARIANT",
                            &format!(
                                "Member type `{}` in UNION `{}` was already declared on line {}.",
                                variant.name, type_decl.name, previous
                            ),
                            file,
                            variant.line,
                        );
                    }
                    self.resolve_type_name(file, &variant.name, variant.line, imports);
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
        file: &AstFile,
        field: &TypeField,
        imports: &HashMap<String, String>,
    ) {
        self.resolve_type_name(file, &field.type_name, field.line, imports);
    }

    fn resolve_function(
        &mut self,
        file: &AstFile,
        function: &crate::ast::Function,
        imports: &HashMap<String, String>,
    ) {
        let previous_template_params = std::mem::replace(
            &mut self.active_template_params,
            function.template_params.iter().cloned().collect(),
        );
        let mut locals = HashMap::new();

        for param in &function.params {
            if locals
                .insert(
                    param.name.clone(),
                    Symbol {
                        file_path: file.path.clone(),
                        line: param.line,
                        visibility: Visibility::Private,
                    },
                )
                .is_some()
            {
                self.report(
                    "SYMBOL_DUPLICATE_LOCAL",
                    &format!(
                        "Parameter `{}` is already declared in this function.",
                        param.name
                    ),
                    file,
                    param.line,
                );
            }

            if let Some(type_name) = &param.type_name {
                self.resolve_type_name(file, type_name, param.line, imports);
            }

            if let Some(default) = &param.default {
                self.resolve_expression(file, default, param.line, imports, &locals);
            }
        }

        if let Some(return_type) = &function.return_type {
            self.resolve_type_name(file, return_type, function.line, imports);
        }

        self.resolve_block(file, &function.body, imports, &mut locals);
        if let Some(trap) = &function.trap {
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
    }

    fn resolve_block(
        &mut self,
        file: &AstFile,
        body: &[Statement],
        imports: &HashMap<String, String>,
        locals: &mut HashMap<String, Symbol>,
    ) {
        for statement in body {
            self.resolve_statement(file, statement, imports, locals);
        }
    }

    fn resolve_statement(
        &mut self,
        file: &AstFile,
        statement: &Statement,
        imports: &HashMap<String, String>,
        locals: &mut HashMap<String, Symbol>,
    ) {
        match statement {
            Statement::Let {
                name,
                type_name,
                value,
                line,
                ..
            } => {
                if let Some(type_name) = type_name {
                    self.resolve_type_name(file, type_name, *line, imports);
                }
                if let Some(value) = value {
                    self.resolve_expression(file, value, *line, imports, locals);
                }
                if locals
                    .insert(
                        name.clone(),
                        Symbol {
                            file_path: file.path.clone(),
                            line: *line,
                            visibility: Visibility::Private,
                        },
                    )
                    .is_some()
                {
                    self.report(
                        "SYMBOL_DUPLICATE_LOCAL",
                        &format!("Local binding `{name}` is already declared in this function."),
                        file,
                        *line,
                    );
                }
            }
            Statement::Return { value, line } => {
                if let Some(value) = value {
                    self.resolve_expression(file, value, *line, imports, locals);
                }
            }
            Statement::Exit { code, line, .. } => {
                if let Some(code) = code {
                    self.resolve_expression(file, code, *line, imports, locals);
                }
            }
            Statement::Continue { .. } => {}
            Statement::Fail { error, line } => {
                self.resolve_expression(file, error, *line, imports, locals);
            }
            Statement::Propagate { .. } => {}
            Statement::Recover { value, line } => {
                if let Some(value) = value {
                    self.resolve_expression(file, value, *line, imports, locals);
                }
            }
            Statement::Assign { name, value, line } => {
                self.resolve_identifier(file, name, *line, imports, locals);
                self.resolve_expression(file, value, *line, imports, locals);
            }
            Statement::StateAssign {
                resource,
                value,
                line,
            } => {
                self.resolve_identifier(file, resource, *line, imports, locals);
                self.resolve_expression(file, value, *line, imports, locals);
            }
            Statement::Expression { expression, line } => {
                self.resolve_expression(file, expression, *line, imports, locals);
            }
            Statement::If {
                condition,
                then_body,
                else_body,
                line,
            } => {
                self.resolve_expression(file, condition, *line, imports, locals);
                self.resolve_nested_block(file, then_body, imports, locals);
                self.resolve_nested_block(file, else_body, imports, locals);
            }
            Statement::Match {
                expression,
                cases,
                line,
            } => {
                self.resolve_expression(file, expression, *line, imports, locals);
                for case in cases {
                    self.resolve_match_pattern(file, &case.pattern, case.line, imports, locals);
                    if let Some(guard) = &case.guard {
                        let mut guard_locals = locals.clone();
                        if let MatchPattern::Union { binding, .. } = &case.pattern {
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
                    if let MatchPattern::Union { binding, .. } = &case.pattern {
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
            Statement::For {
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
                let mut nested = locals.clone();
                if nested
                    .insert(
                        name.clone(),
                        Symbol {
                            file_path: file.path.clone(),
                            line: *line,
                            visibility: Visibility::Private,
                        },
                    )
                    .is_some()
                {
                    self.report(
                        "SYMBOL_DUPLICATE_LOCAL",
                        &format!("Local binding `{name}` is already declared in this function."),
                        file,
                        *line,
                    );
                }
                self.resolve_block(file, body, imports, &mut nested);
            }
            Statement::ForEach {
                name,
                iterable,
                body,
                line,
            } => {
                self.resolve_expression(file, iterable, *line, imports, locals);
                let mut nested = locals.clone();
                if nested
                    .insert(
                        name.clone(),
                        Symbol {
                            file_path: file.path.clone(),
                            line: *line,
                            visibility: Visibility::Private,
                        },
                    )
                    .is_some()
                {
                    self.report(
                        "SYMBOL_DUPLICATE_LOCAL",
                        &format!("Local binding `{name}` is already declared in this function."),
                        file,
                        *line,
                    );
                }
                self.resolve_block(file, body, imports, &mut nested);
            }
            Statement::While {
                kind: _,
                condition,
                body,
                line,
            } => {
                self.resolve_expression(file, condition, *line, imports, locals);
                self.resolve_nested_block(file, body, imports, locals);
            }
            Statement::DoUntil {
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
        file: &AstFile,
        body: &[Statement],
        imports: &HashMap<String, String>,
        locals: &HashMap<String, Symbol>,
    ) {
        let mut nested = locals.clone();
        self.resolve_block(file, body, imports, &mut nested);
    }

    fn resolve_match_pattern(
        &mut self,
        file: &AstFile,
        pattern: &MatchPattern,
        line: usize,
        imports: &HashMap<String, String>,
        locals: &HashMap<String, Symbol>,
    ) {
        match pattern {
            MatchPattern::Else => {}
            MatchPattern::Literal(pattern) => {
                self.resolve_expression(file, pattern, line, imports, locals);
            }
            MatchPattern::Union { type_name, .. } => {
                if type_name != "Ok" {
                    self.resolve_type_name(file, type_name, line, imports);
                }
            }
            MatchPattern::OneOf(patterns) => {
                for pattern in patterns {
                    self.resolve_expression(file, pattern, line, imports, locals);
                }
            }
        }
    }

    fn resolve_expression(
        &mut self,
        file: &AstFile,
        expression: &Expression,
        line: usize,
        imports: &HashMap<String, String>,
        locals: &HashMap<String, Symbol>,
    ) {
        match expression {
            Expression::String(_)
            | Expression::Number(_)
            | Expression::Scalar(_)
            | Expression::Boolean(_) => {}
            Expression::Binary { left, right, .. } => {
                self.resolve_expression(file, left, line, imports, locals);
                self.resolve_expression(file, right, line, imports, locals);
            }
            Expression::Unary { operand, .. } => {
                self.resolve_expression(file, operand, line, imports, locals);
            }
            Expression::Call {
                callee, arguments, ..
            } => {
                self.resolve_callable(file, callee, line, imports, locals);
                for argument in arguments {
                    self.resolve_expression(file, call_arg_value(argument), line, imports, locals);
                }
            }
            Expression::Lambda { params, body, .. } => {
                let mut lambda_locals = locals.clone();
                for param in params {
                    if let Some(type_name) = &param.type_name {
                        self.resolve_type_name(file, type_name, param.line, imports);
                    }
                    lambda_locals.insert(
                        param.name.clone(),
                        Symbol {
                            file_path: file.path.clone(),
                            line: param.line,
                            visibility: Visibility::Private,
                        },
                    );
                    if let Some(default) = &param.default {
                        self.resolve_expression(file, default, param.line, imports, &lambda_locals);
                    }
                }
                self.resolve_expression(file, body, line, imports, &lambda_locals);
            }
            Expression::Constructor {
                type_name,
                arguments,
            } => {
                if !matches!(type_name.as_str(), "Error" | "Ok" | "Err") {
                    self.resolve_type_name(file, type_name, line, imports);
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
            Expression::WithUpdate { target, updates } => {
                self.resolve_expression(file, target, line, imports, locals);
                for update in updates {
                    self.resolve_expression(file, &update.value, update.line, imports, locals);
                }
            }
            Expression::ListLiteral(values) => {
                for value in values {
                    self.resolve_expression(file, value, line, imports, locals);
                }
            }
            Expression::MapLiteral {
                key_type,
                value_type,
                entries,
            } => {
                self.resolve_type_name(file, key_type, line, imports);
                // A `Map OF K TO RES File` literal value carries the resource
                // ownership-axis marker (§15.6); resolve the underlying type.
                let value_type = value_type.strip_prefix("RES ").unwrap_or(value_type);
                self.resolve_type_name(file, value_type, line, imports);
                for (key, value) in entries {
                    self.resolve_expression(file, key, line, imports, locals);
                    self.resolve_expression(file, value, line, imports, locals);
                }
            }
            Expression::MemberAccess { target, .. } => {
                if let Expression::Identifier(name) = target.as_ref() {
                    if self.types.contains(name) {
                        return;
                    }
                }
                self.resolve_expression(file, target, line, imports, locals);
            }
            Expression::Trapped {
                expression,
                binding,
                handler,
                line: trap_line,
            } => {
                self.resolve_expression(file, expression, line, imports, locals);
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
            Expression::Identifier(name) if name == "NOTHING" => {}
            Expression::Identifier(name) => {
                self.resolve_identifier(file, name, line, imports, locals);
            }
        }
    }

    fn resolve_callable(
        &mut self,
        file: &AstFile,
        callee: &str,
        line: usize,
        imports: &HashMap<String, String>,
        locals: &HashMap<String, Symbol>,
    ) {
        if callee.contains('.') {
            self.resolve_package_qualified_name(file, callee, line, imports);
        } else if builtins::general::is_general_call(callee) {
            return;
        } else if builtins::testing::is_expect_call(callee) {
            // Assertion builtins are compiler-lowered; their arguments are
            // resolved by the caller. Placement (TCASE-only) is enforced earlier
            // by `crate::testing::validate_expect_placement`.
            return;
        } else if locals.contains_key(callee) {
            return;
        } else if !self.function_visible_in_file(file, callee) {
            self.report(
                "SYMBOL_UNKNOWN_IDENTIFIER",
                &format!("Callable `{callee}` is not a top-level function."),
                file,
                line,
            );
        }
    }

    fn resolve_identifier(
        &mut self,
        file: &AstFile,
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
            && !builtins::general::is_general_call(name)
        {
            self.report(
                "SYMBOL_UNKNOWN_IDENTIFIER",
                &format!("Identifier `{name}` is not declared in this scope."),
                file,
                line,
            );
        }
    }

    fn resolve_type_name(
        &mut self,
        file: &AstFile,
        type_name: &str,
        line: usize,
        imports: &HashMap<String, String>,
    ) {
        // Grouped type names (`(T)`) are valid syntax the parser emits verbatim
        // (ast/expr.rs). `parse_type` and `thread_parts_full` already strip the
        // group at their positions; do the same here so a parenthesized type in
        // any non-thread position (`List OF (Map OF String TO Integer)`,
        // `AS (Integer)`) resolves through its inner type instead of falling to
        // the bare-name arm and being rejected as unknown (bug-105).
        let stripped = crate::builtins::thread::strip_type_group(type_name);
        if stripped != type_name {
            self.resolve_type_name(file, stripped, line, imports);
            return;
        }

        // `Result` (and its success member `Ok`) are internal runtime types: a
        // user never names them in any type position. Intercept them here with a
        // targeted diagnostic instead of resolving them as types or falling
        // through to a generic "unknown type" error.
        if type_name == "Result" || type_name == "Ok" || type_name.starts_with("Result OF ") {
            self.report(
                "TYPE_RESULT_NOT_USER_VISIBLE",
                "`Result` is an internal type; declare the success type directly \
                 (a function call yields its value or fails with an `Error`).",
                file,
                line,
            );
            return;
        }
        if let Some(rest) = type_name.strip_prefix("ISOLATED FUNC(") {
            self.resolve_function_type_name(file, rest, line, imports);
            return;
        }
        if let Some(rest) = type_name.strip_prefix("FUNC(") {
            self.resolve_function_type_name(file, rest, line, imports);
            return;
        }
        if let Some(element) = type_name.strip_prefix("List OF ") {
            // A `List OF RES File` element carries the resource ownership-axis
            // marker; resolve the underlying type (§15.6).
            let element = element.strip_prefix("RES ").unwrap_or(element);
            self.resolve_type_name(file, element, line, imports);
            return;
        }
        if let Some((_, message, resource, output)) =
            crate::builtins::thread::thread_parts_full(type_name)
        {
            self.resolve_type_name(file, message, line, imports);
            if let Some(resource) = resource {
                self.resolve_type_name(file, resource, line, imports);
            }
            self.resolve_type_name(file, output, line, imports);
            return;
        }
        if let Some(rest) = type_name.strip_prefix("Map OF ") {
            if let Some((key, value)) = split_top_level_to(rest) {
                let value = value.strip_prefix("RES ").unwrap_or(value);
                self.resolve_type_name(file, key, line, imports);
                self.resolve_type_name(file, value, line, imports);
                return;
            }
        }
        if let Some(rest) = type_name.strip_prefix("MapEntry OF ") {
            if let Some((key, value)) = split_top_level_to(rest) {
                self.resolve_type_name(file, key, line, imports);
                self.resolve_type_name(file, value, line, imports);
                return;
            }
        }

        if let Some((base, args)) = type_name.split_once(" OF ") {
            if self.types.contains(base) || self.active_template_params.contains(base) {
                // Split at top-level commas only: a template argument may itself
                // be a nested `FUNC(...) AS R` or `Map OF K TO V` whose internal
                // commas must not split the argument list (bug-106).
                for arg in crate::builtins::split_top_level_commas(args) {
                    self.resolve_type_name(file, arg, line, imports);
                }
                return;
            }
        }

        if type_name == "Unknown" || self.active_template_params.contains(type_name) {
            return;
        }

        if type_name.contains('.') {
            self.resolve_package_qualified_name(file, type_name, line, imports);
        } else if !self.types.contains(type_name) {
            self.report(
                "SYMBOL_UNKNOWN_TYPE",
                &format!("Type `{type_name}` is not a built-in or top-level project type."),
                file,
                line,
            );
        }
    }

    fn resolve_function_type_name(
        &mut self,
        file: &AstFile,
        rest: &str,
        line: usize,
        imports: &HashMap<String, String>,
    ) {
        // Split at the depth-0 `) AS ` so a parameter that is itself a
        // `FUNC(...) AS …` (or any parenthesized/nested type) does not split at
        // an inner `) AS ` and produce garbage names (bug-106).
        let Some((params, return_type)) = crate::builtins::split_func_params_and_return(rest) else {
            self.report(
                "SYMBOL_UNKNOWN_TYPE",
                &format!("Function type `FUNC({rest}` is malformed."),
                file,
                line,
            );
            return;
        };
        for param in params {
            self.resolve_type_name(file, param, line, imports);
        }
        self.resolve_type_name(file, return_type, line, imports);
    }

    fn resolve_package_qualified_name(
        &mut self,
        file: &AstFile,
        name: &str,
        line: usize,
        imports: &HashMap<String, String>,
    ) {
        let root = name.split('.').next().unwrap_or(name);
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
            if !builtins::is_builtin_member(&qualified_name) {
                self.report(
                    "SYMBOL_UNKNOWN_IDENTIFIER",
                    &format!("Built-in package `{package}` does not export `{qualified_name}`."),
                    file,
                    line,
                );
            }
        }
    }
}

/// Split a `Map`/`MapEntry` body `K TO V` on the top-level ` TO ` separating the
/// outer key from its value. A leftmost `split_once(" TO ")` mis-parses a key
/// that itself carries a top-level ` TO ` (a nested `Map`/`Thread`/`FUNC`-typed
/// key, bug-108.2). Mirrors `syntaxcheck::types::split_map_body`: separators
/// inside parenthesized / `FUNC(...)` groups and those owned by nested
/// `Map`/`MapEntry`/`Thread`/`ThreadWorker` sub-types are skipped.
fn split_top_level_to(body: &str) -> Option<(&str, &str)> {
    let bytes = body.as_bytes();
    let mut depth: usize = 0;
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
            _ if depth == 0 && body.is_char_boundary(index) && body[index..].starts_with(" TO ") => {
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

/// Whether a `Map`/`MapEntry`/`Thread`/`ThreadWorker` `OF`-construct (each owning
/// exactly one top-level ` TO `) begins at byte `at`, on a word boundary.
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

#[cfg(test)]
mod tests {
    use crate::manifest::validate_project_manifest;
    use std::fs;
    use tempfile::TempDir;

    fn quiet<T>(f: impl FnOnce() -> T) -> T {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let out = f();
        std::panic::set_hook(prev);
        out
    }

    /// Resolve an inline single-file executable project whose `src/main.mfb` is
    /// `source`. Returns `true` when resolution failed (or the source did not
    /// even parse).
    fn resolve_source_fails(source: &str) -> bool {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("project.json"),
            r#"{ "name": "scratch", "version": "0.1.0", "mfb": "1.0", "kind": "executable",
                 "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
                 "entry": "main", "targets": ["native"] }"#,
        )
        .unwrap();
        fs::write(root.join("src").join("main.mfb"), source).unwrap();
        let manifest = validate_project_manifest(&root.join("project.json")).unwrap();
        let Ok(ast) = quiet(|| crate::ast::parse_project("scratch", root, &manifest)) else {
            return true;
        };
        quiet(|| crate::resolver::resolve_project(root, &manifest, &ast)).is_err()
    }

    fn resolve_fixture_fails(name: &str) -> bool {
        let dir = crate::testutil::fixture_dir(name);
        let manifest = validate_project_manifest(&dir.join("project.json")).unwrap();
        let pname = manifest
            .get("name")
            .and_then(|v| v.get::<String>())
            .cloned()
            .unwrap();
        let ast = crate::ast::parse_project(&pname, &dir, &manifest).unwrap();
        quiet(|| crate::resolver::resolve_project(&dir, &manifest, &ast)).is_err()
    }

    #[test]
    fn broad_valid_fixtures_resolve() {
        for name in [
            // control-flow-valid was consolidated into control-flow-behavior by
            // the tests reorganization; it stays a valid resolve fixture.
            "control-flow-behavior",
            "control-flow-match",
            "control-flow-match-when",
            "control-flow-match-destructuring",
            "control-flow-match-else",
            "control-flow-if",
            "lambda-capture-valid",
            "collection-list-bindings",
            "collection-map-bindings",
            "func_return_overload_valid",
            "native-resource-link-valid",
        ] {
            let dir = crate::testutil::fixture_dir(name);
            let manifest = validate_project_manifest(&dir.join("project.json")).unwrap();
            let pname = manifest
                .get("name")
                .and_then(|v| v.get::<String>())
                .cloned()
                .unwrap();
            let ast = crate::ast::parse_project(&pname, &dir, &manifest).unwrap();
            assert!(
                quiet(|| crate::resolver::resolve_project(&dir, &manifest, &ast)).is_ok(),
                "fixture `{name}` should resolve cleanly"
            );
        }
    }

    #[test]
    fn unknown_identifier_reports() {
        assert!(resolve_source_fails(
            "IMPORT io\n\nSUB main()\n  io::print(missingVar)\nEND SUB\n"
        ));
    }

    #[test]
    fn unknown_type_reports() {
        assert!(resolve_source_fails(
            "SUB main()\n  LET x AS NoSuchType = 0\nEND SUB\n"
        ));
    }

    #[test]
    fn result_type_not_user_visible_reports() {
        assert!(resolve_source_fails(
            "SUB main()\n  LET x AS Result = 0\nEND SUB\n"
        ));
    }

    #[test]
    fn duplicate_local_reports() {
        assert!(resolve_source_fails(
            "SUB main()\n  LET x AS Integer = 1\n  LET x AS Integer = 2\nEND SUB\n"
        ));
    }

    #[test]
    fn duplicate_parameter_reports() {
        assert!(resolve_source_fails(
            "SUB doit(a AS Integer, a AS Integer)\nEND SUB\n\nSUB main()\nEND SUB\n"
        ));
    }

    #[test]
    fn unknown_import_qualified_use_reports() {
        assert!(resolve_source_fails("SUB main()\n  foo::bar()\nEND SUB\n"));
    }

    #[test]
    fn duplicate_import_binding_reports() {
        assert!(resolve_source_fails(
            "IMPORT io\nIMPORT io\n\nSUB main()\nEND SUB\n"
        ));
    }

    #[test]
    fn unknown_type_field_reports() {
        assert!(resolve_source_fails(
            "TYPE Widget\n  size AS NoSuchType\nEND TYPE\n\nSUB main()\nEND SUB\n"
        ));
    }

    #[test]
    fn builtin_member_unknown_reports() {
        assert!(resolve_source_fails(
            "IMPORT io\n\nSUB main()\n  io::notARealFunction()\nEND SUB\n"
        ));
    }

    #[test]
    fn callable_not_top_level_reports() {
        assert!(resolve_source_fails(
            "SUB main()\n  notAFunction()\nEND SUB\n"
        ));
    }

    #[test]
    fn resolution_error_fixtures_fail() {
        for name in [
            "native-resource-close-not-native-invalid",
            "native-resource-close-signature-invalid",
            "native-link-duplicate-resource-invalid",
            "result-not-user-visible-invalid",
            "collections-cutover-invalid",
            "doc-block-invalid",
        ] {
            assert!(
                resolve_fixture_fails(name),
                "fixture `{name}` should fail to resolve"
            );
        }
    }

    /// Resolve an inline source and assert it resolves cleanly (no errors). Used
    /// to drive the *success* side of statement / expression / type_name arms.
    fn assert_source_ok(source: &str) {
        assert!(
            !resolve_source_fails(source),
            "source should resolve cleanly:\n{source}"
        );
    }

    // --- statement arms (success side) ---

    #[test]
    fn statements_return_exit_fail_recover_continue() {
        assert_source_ok(concat!(
            "IMPORT io\n\n",
            "FUNC pick AS Integer\n",
            "  FOR i = 1 TO 3\n",
            "    IF i = 2 THEN CONTINUE FOR\n",
            "  NEXT\n",
            "  RETURN 1\n",
            "  TRAP(err)\n",
            "    RECOVER err.code\n",
            "  END TRAP\n",
            "END FUNC\n\n",
            "SUB stop()\n",
            "  EXIT PROGRAM 0\n",
            "END SUB\n\n",
            "FUNC boom AS Integer\n",
            "  FAIL error(1, \"x\")\n",
            "END FUNC\n\n",
            "SUB main()\n",
            "  io::print(toString(pick()))\n",
            "END SUB\n",
        ));
    }

    #[test]
    fn statements_assign_and_propagate() {
        assert_source_ok(concat!(
            "IMPORT io\n\n",
            "FUNC helper() AS Integer\n",
            "  RETURN 1\n",
            "END FUNC\n\n",
            "SUB main()\n",
            "  MUT total AS Integer = 0\n",
            "  total = total + helper()\n",
            "  io::print(toString(total))\n",
            "END SUB\n",
        ));
    }

    #[test]
    fn nested_blocks_if_while_dountil() {
        assert_source_ok(concat!(
            "IMPORT io\n\n",
            "SUB main()\n",
            "  MUT n AS Integer = 0\n",
            "  IF n < 1 THEN\n",
            "    n = 1\n",
            "  ELSE\n",
            "    n = 2\n",
            "  END IF\n",
            "  WHILE n < 3\n",
            "    n = n + 1\n",
            "  WEND\n",
            "  DO\n",
            "    n = n + 1\n",
            "  LOOP UNTIL n > 5\n",
            "  io::print(toString(n))\n",
            "END SUB\n",
        ));
    }

    #[test]
    fn foreach_with_list_and_map_types() {
        assert_source_ok(concat!(
            "IMPORT io\n\n",
            "SUB main()\n",
            "  LET xs AS List OF Integer = [1, 2, 3]\n",
            "  FOR EACH x IN xs\n",
            "    io::print(toString(x))\n",
            "  NEXT\n",
            "  LET m AS Map OF String TO Integer = Map OF String TO Integer { \"a\" := 1 }\n",
            "  FOR EACH e IN m\n",
            "    io::print(e.key)\n",
            "  NEXT\n",
            "END SUB\n",
        ));
    }

    #[test]
    fn duplicate_local_in_for_loop_reports() {
        assert!(resolve_source_fails(concat!(
            "SUB main()\n",
            "  LET i AS Integer = 0\n",
            "  FOR i = 1 TO 3\n",
            "  NEXT\n",
            "END SUB\n",
        )));
    }

    #[test]
    fn duplicate_local_in_foreach_reports() {
        assert!(resolve_source_fails(concat!(
            "SUB main()\n",
            "  LET item AS Integer = 0\n",
            "  LET xs AS List OF Integer = [1]\n",
            "  FOR EACH item IN xs\n",
            "  NEXT\n",
            "END SUB\n",
        )));
    }

    // --- match arms ---

    #[test]
    fn match_with_union_guard_and_oneof() {
        assert_source_ok(concat!(
            "IMPORT io\n\n",
            "TYPE Circle\n  radius AS Integer\nEND TYPE\n\n",
            "UNION Shape\n  Circle\nEND UNION\n\n",
            "FUNC describe(shape AS Shape) AS Integer\n",
            "  MATCH shape\n",
            "    CASE Circle(c) WHEN c.radius\n",
            "      RETURN 1\n",
            "    CASE ELSE\n",
            "      RETURN 0\n",
            "  END MATCH\n",
            "END FUNC\n\n",
            "SUB main()\n",
            "  MATCH 2\n",
            "    CASE 1, 2\n",
            "      io::print(\"lo\")\n",
            "    CASE ELSE\n",
            "      io::print(\"hi\")\n",
            "  END MATCH\n",
            "END SUB\n",
        ));
    }

    // --- expression arms ---

    #[test]
    fn expressions_binary_unary_lambda_constructor() {
        assert_source_ok(concat!(
            "IMPORT io\n\n",
            "TYPE Point\n  x AS Integer\n  y AS Integer\nEND TYPE\n\n",
            "SUB main()\n",
            "  LET p AS Point = Point[1, 2]\n",
            "  LET q AS Point = WITH p { x := 5 }\n",
            "  LET neg AS Integer = -p.x\n",
            "  LET sum AS Integer = p.x + q.y\n",
            "  LET f AS FUNC(Integer) AS Integer = LAMBDA(v AS Integer) -> v + sum\n",
            "  LET applied AS Integer = f(neg)\n",
            "  LET xs AS List OF Integer = [1, 2, 3]\n",
            "  io::print(toString(applied))\n",
            "END SUB\n",
        ));
    }

    #[test]
    fn trapped_expression_binding_in_scope() {
        assert_source_ok(concat!(
            "IMPORT io\n\n",
            "FUNC risky AS Integer\n",
            "  RETURN 1\n",
            "END FUNC\n\n",
            "SUB main()\n",
            "  LET v AS Integer = risky() TRAP(err)\n",
            "    io::print(toString(err.code))\n",
            "    RECOVER 0\n",
            "  END TRAP\n",
            "  io::print(toString(v))\n",
            "END SUB\n",
        ));
    }

    // --- type_name grammar arms ---

    #[test]
    fn type_name_function_type_and_nested_generics() {
        assert_source_ok(concat!(
            "SUB main()\n",
            "  LET a AS FUNC(Integer, String) AS Boolean = LAMBDA(n AS Integer, s AS String) -> n > 0\n",
            "  LET b AS List OF List OF Integer = [[1], [2]]\n",
            "  LET c AS Map OF String TO List OF Integer = Map OF String TO List OF Integer {}\n",
            "  LET e AS FUNC() AS Integer = LAMBDA() -> 0\n",
            "  LET used AS Boolean = a(1, \"x\")\n",
            "  LET zero AS Integer = e()\n",
            "END SUB\n",
        ));
    }

    #[test]
    fn malformed_function_type_reports() {
        // `FUNC(...)` without `) AS` return separator is malformed.
        assert!(resolve_source_fails(concat!(
            "SUB main()\n",
            "  LET a AS FUNC(Integer = 0\n",
            "END SUB\n",
        )));
    }

    #[test]
    fn union_and_enum_declarations_resolve() {
        assert_source_ok(concat!(
            "IMPORT io\n\n",
            "TYPE Circle\n  radius AS Integer\nEND TYPE\n\n",
            "TYPE Square\n  side AS Integer\nEND TYPE\n\n",
            "UNION Shape\n  Circle\n  Square\nEND UNION\n\n",
            "ENUM Color\n  Red, Green, Blue\nEND ENUM\n\n",
            "SUB main()\n",
            "  io::print(\"ok\")\n",
            "END SUB\n",
        ));
    }

    // --- LINK / resource / func-alias arms ---

    #[test]
    fn link_block_and_resource_and_alias_resolve() {
        assert_source_ok(concat!(
            "EXPORT RESOURCE Db CLOSE BY dbLink::close\n\n",
            "LINK \"sqlite3\" AS dbLink\n",
            "  FUNC open(path AS String) AS RES Db\n",
            "    SYMBOL \"sqlite3_open\"\n",
            "    ABI (path CString, return OUT CPtr) AS status CInt32\n",
            "    SUCCESS_ON status = 0\n",
            "  END FUNC\n\n",
            "  FUNC close(RES db AS Db) AS Nothing\n",
            "    SYMBOL \"sqlite3_close\"\n",
            "    ABI (db CPtr) AS status CInt32\n",
            "    SUCCESS_ON status = 0\n",
            "  END FUNC\n",
            "END LINK\n\n",
            "EXPORT FUNC closeDb AS dbLink::close\n\n",
            "SUB main()\n",
            "END SUB\n",
        ));
    }

    #[test]
    fn resource_close_unknown_alias_reports() {
        assert!(resolve_source_fails(concat!(
            "RESOURCE Db CLOSE BY ghostLink::close\n\n",
            "SUB main()\n",
            "END SUB\n",
        )));
    }

    #[test]
    fn resource_close_missing_func_in_link_reports() {
        assert!(resolve_source_fails(concat!(
            "RESOURCE Db CLOSE BY dbLink::missing\n\n",
            "LINK \"sqlite3\" AS dbLink\n",
            "  FUNC close(RES db AS Db) AS Nothing\n",
            "    SYMBOL \"sqlite3_close\"\n",
            "    ABI (db CPtr) AS status CInt32\n",
            "    SUCCESS_ON status = 0\n",
            "  END FUNC\n",
            "END LINK\n\n",
            "SUB main()\n",
            "END SUB\n",
        )));
    }

    #[test]
    fn func_alias_unknown_target_reports() {
        assert!(resolve_source_fails(concat!(
            "EXPORT FUNC bogus AS ghostLink::nope\n\n",
            "SUB main()\n",
            "END SUB\n",
        )));
    }

    #[test]
    fn link_member_unknown_reports() {
        // `dbLink::notThere` names an unknown member of the LINK namespace.
        assert!(resolve_source_fails(concat!(
            "LINK \"sqlite3\" AS dbLink\n",
            "  FUNC close(RES db AS Db) AS Nothing\n",
            "    SYMBOL \"sqlite3_close\"\n",
            "    ABI (db CPtr) AS status CInt32\n",
            "    SUCCESS_ON status = 0\n",
            "  END FUNC\n",
            "END LINK\n\n",
            "RESOURCE Db CLOSE BY dbLink::close\n\n",
            "SUB main()\n",
            "  dbLink::notThere()\n",
            "END SUB\n",
        )));
    }

    // --- DOC block success + additional error branches ---

    #[test]
    fn doc_block_valid_variants_resolve() {
        assert_source_ok(concat!(
            "DOC\n  PACKAGE\n  DESC A package.\nEND DOC\n\n",
            "DOC INTERNAL\n  FUNC add\n  ARG a first\n  ARG b second\n  RET the sum\n",
            "  ERROR 1001 when it overflows\n  GROUP Math\nEND DOC\n",
            "EXPORT FUNC add(a AS Integer, b AS Integer) AS Integer\n",
            "  RETURN a + b\n",
            "END FUNC\n\n",
            "DOC\n  TYPE Point\n  PROP x the x\n  PROP y the y\nEND DOC\n",
            "EXPORT TYPE Point\n  x AS Integer\n  y AS Integer\nEND TYPE\n\n",
            "DOC\n  ENUM Color\n  PROP Red the red\nEND DOC\n",
            "EXPORT ENUM Color\n  Red, Green\nEND ENUM\n\n",
            "DOC\n  UNION Shape\n  PROP Point a point\nEND DOC\n",
            "EXPORT UNION Shape\n  Point\nEND UNION\n\n",
            "SUB main()\nEND SUB\n",
        ));
    }

    #[test]
    fn doc_overload_disambiguated_by_types() {
        assert_source_ok(concat!(
            "DOC\n  FUNC scale(Integer)\n  ARG n the value\nEND DOC\n",
            "EXPORT FUNC scale(n AS Integer) AS Integer\n",
            "  RETURN n * 2\n",
            "END FUNC\n\n",
            "DOC\n  FUNC scale(Float)\n  ARG n the value\nEND DOC\n",
            "EXPORT FUNC scale(n AS Float) AS Float\n",
            "  RETURN n * 2.0\n",
            "END FUNC\n\n",
            "SUB main()\nEND SUB\n",
        ));
    }

    #[test]
    fn doc_ret_on_non_callable_reports() {
        // RET is only valid on FUNC/SUB; here it sits on a TYPE block.
        assert!(resolve_source_fails(concat!(
            "DOC\n  TYPE Point\n  RET nonsense\nEND DOC\n",
            "TYPE Point\n  x AS Integer\nEND TYPE\n\n",
            "SUB main()\nEND SUB\n",
        )));
    }

    #[test]
    fn doc_type_name_mismatch_reports() {
        // `TYPE add` names a FUNC → DOC_NAME_MISMATCH.
        assert!(resolve_source_fails(concat!(
            "DOC\n  TYPE add\n  DESC wrong kind.\nEND DOC\n",
            "EXPORT FUNC add(a AS Integer) AS Integer\n",
            "  RETURN a\n",
            "END FUNC\n\n",
            "SUB main()\nEND SUB\n",
        )));
    }

    // --- additional DOC error branches ---

    #[test]
    fn doc_duplicate_internal_attr_reports() {
        assert!(resolve_source_fails(concat!(
            "DOC INTERNAL INTERNAL\n  FUNC f\nEND DOC\n",
            "EXPORT FUNC f() AS Integer\n  RETURN 0\nEND FUNC\n\n",
            "SUB main()\nEND SUB\n",
        )));
    }

    #[test]
    fn doc_arg_and_error_invalid_context_reports() {
        // ARG / RET / ERROR on a TYPE block are all invalid.
        assert!(resolve_source_fails(concat!(
            "DOC\n  TYPE Point\n  ARG a nope\n  ERROR 1 nope\nEND DOC\n",
            "EXPORT TYPE Point\n  x AS Integer\nEND TYPE\n\n",
            "SUB main()\nEND SUB\n",
        )));
    }

    #[test]
    fn doc_duplicate_ret_example_deprecated_group_reports() {
        assert!(resolve_source_fails(concat!(
            "DOC\n  FUNC f\n  RET one\n  RET two\n",
            "  EXAMPLE\n    LET a AS Integer = 1\n  END EXAMPLE\n",
            "  EXAMPLE\n    LET b AS Integer = 2\n  END EXAMPLE\n",
            "  DEPRECATED first\n  DEPRECATED second\n",
            "  GROUP A\n  GROUP B\nEND DOC\n",
            "EXPORT FUNC f() AS Integer\n  RETURN 0\nEND FUNC\n\n",
            "SUB main()\nEND SUB\n",
        )));
    }

    #[test]
    fn doc_prop_invalid_context_and_internal_on_package_reports() {
        assert!(resolve_source_fails(concat!(
            "DOC\n  FUNC f\n  PROP x nope\nEND DOC\n",
            "EXPORT FUNC f() AS Integer\n  RETURN 0\nEND FUNC\n\n",
            "DOC INTERNAL\n  PACKAGE\nEND DOC\n\n",
            "SUB main()\nEND SUB\n",
        )));
    }

    #[test]
    fn doc_func_wrong_subkind_reports() {
        // A `SUB` doc header naming a name that exists only as a FUNC → the
        // matching-overload list is empty (DOC_NAME_MISMATCH).
        assert!(resolve_source_fails(concat!(
            "DOC\n  SUB add\n  DESC add is a FUNC, not a SUB.\nEND DOC\n",
            "EXPORT FUNC add(a AS Integer) AS Integer\n  RETURN a\nEND FUNC\n\n",
            "SUB main()\nEND SUB\n",
        )));
    }

    #[test]
    fn doc_type_kind_mismatch_reports() {
        // `TYPE Color` names an ENUM (a type decl of the wrong kind).
        assert!(resolve_source_fails(concat!(
            "DOC\n  TYPE Color\n  DESC wrong kind.\nEND DOC\n",
            "EXPORT ENUM Color\n  Red, Green\nEND ENUM\n\n",
            "SUB main()\nEND SUB\n",
        )));
    }

    #[test]
    fn doc_type_unresolved_reports() {
        assert!(resolve_source_fails(concat!(
            "DOC\n  TYPE NoSuchType\n  DESC nothing.\nEND DOC\n",
            "SUB main()\nEND SUB\n",
        )));
    }

    #[test]
    fn doc_duplicate_block_for_same_target_reports() {
        assert!(resolve_source_fails(concat!(
            "DOC\n  TYPE Point\n  DESC first.\nEND DOC\n",
            "DOC\n  TYPE Point\n  DESC second (duplicate).\nEND DOC\n",
            "EXPORT TYPE Point\n  x AS Integer\nEND TYPE\n\n",
            "SUB main()\nEND SUB\n",
        )));
    }

    // --- resource close (no-dot form) ---

    #[test]
    fn resource_close_not_dotted_reports() {
        // A close op with no `alias.func` form is RESOURCE_CLOSE_NOT_NATIVE.
        assert!(resolve_source_fails(concat!(
            "RESOURCE Db CLOSE BY plainName\n\n",
            "SUB main()\nEND SUB\n",
        )));
    }

    // --- link block param/return type resolution ---

    #[test]
    fn link_block_unknown_param_type_reports() {
        // A non-C-ABI param type inside a LINK function must resolve; `Bogus`
        // does not.
        assert!(resolve_source_fails(concat!(
            "LINK \"lib\" AS l\n",
            "  FUNC f(x AS Bogus) AS Nothing\n",
            "    SYMBOL \"f\"\n",
            "    ABI (x CPtr) AS status CInt32\n",
            "    SUCCESS_ON status = 0\n",
            "  END FUNC\n",
            "END LINK\n\n",
            "SUB main()\nEND SUB\n",
        )));
    }

    // --- binding with declared type + value ---

    #[test]
    fn top_level_binding_with_type_and_value_resolves() {
        assert_source_ok(concat!(
            "IMPORT io\n\n",
            "LET GREETING AS String = \"hi\"\n\n",
            "SUB main()\n",
            "  io::print(GREETING)\n",
            "END SUB\n",
        ));
    }

    // --- UNION includes + duplicate variant / duplicate enum member ---

    #[test]
    fn union_with_includes_resolves() {
        assert_source_ok(concat!(
            "IMPORT io\n\n",
            "TYPE Circle\n  radius AS Integer\nEND TYPE\n\n",
            "TYPE Square\n  side AS Integer\nEND TYPE\n\n",
            "UNION Round\n  Circle\nEND UNION\n\n",
            "UNION Shape INCLUDES Round\n  Square\nEND UNION\n\n",
            "SUB main()\n",
            "  io::print(\"ok\")\n",
            "END SUB\n",
        ));
    }

    #[test]
    fn union_duplicate_variant_reports() {
        assert!(resolve_source_fails(concat!(
            "TYPE Circle\n  radius AS Integer\nEND TYPE\n\n",
            "UNION Shape\n  Circle\n  Circle\nEND UNION\n\n",
            "SUB main()\nEND SUB\n",
        )));
    }

    #[test]
    fn enum_duplicate_member_reports() {
        assert!(resolve_source_fails(concat!(
            "ENUM Color\n  Red, Red\nEND ENUM\n\n",
            "SUB main()\nEND SUB\n",
        )));
    }

    // --- StateAssign statement ---

    #[test]
    fn state_assign_statement_resolves() {
        assert_source_ok(concat!(
            "IMPORT fs\nIMPORT io\n\n",
            "TYPE FileState\n  pos AS Integer\nEND TYPE\n\n",
            "SUB advance(RES f AS File STATE FileState, by AS Integer)\n",
            "  f.state = WITH f.state { pos := f.state.pos + by }\n",
            "END SUB\n\n",
            "FUNC main AS Integer\n",
            "  RES f AS File STATE FileState = fs::createTempFile()\n",
            "  f.state = WITH f.state { pos := 10 }\n",
            "  advance(f, 5)\n",
            "  fs::close(f)\n",
            "  RETURN 0\n",
            "END FUNC\n",
        ));
    }

    // --- type_name grammar: thread + MapEntry ---

    #[test]
    fn thread_and_mapentry_type_names_resolve() {
        assert_source_ok(concat!(
            "IMPORT thread\nIMPORT io\n\n",
            "EXPORT ISOLATED FUNC echo(t AS ThreadWorker OF String TO Integer, seed AS String) AS Integer\n",
            "  RETURN 0\n",
            "END FUNC\n\n",
            "SUB main()\n",
            "  LET m AS Map OF String TO Integer = Map OF String TO Integer { \"a\" := 1 }\n",
            "  FOR EACH e IN m\n",
            "    LET pair AS MapEntry OF String TO Integer = e\n",
            "    io::print(pair.key)\n",
            "  NEXT\n",
            "END SUB\n",
        ));
    }

    // --- import-alias conflicts ---

    #[test]
    fn import_alias_conflicts_with_builtin_reports() {
        // Aliasing an import to a built-in package name conflicts.
        assert!(resolve_source_fails(concat!(
            "IMPORT thread AS io\n\n",
            "SUB main()\nEND SUB\n",
        )));
    }

    #[test]
    fn import_alias_conflicts_with_top_level_reports() {
        // An import binding that shadows a visible top-level declaration.
        assert!(resolve_source_fails(concat!(
            "IMPORT io AS helper\n\n",
            "FUNC helper() AS Integer\n  RETURN 0\nEND FUNC\n\n",
            "SUB main()\nEND SUB\n",
        )));
    }

    // --- duplicate TYPE field ---

    #[test]
    fn duplicate_type_field_reports() {
        assert!(resolve_source_fails(concat!(
            "TYPE Point\n  x AS Integer\n  x AS Integer\nEND TYPE\n\n",
            "SUB main()\nEND SUB\n",
        )));
    }

    // --- PROPAGATE statement ---

    #[test]
    fn propagate_statement_resolves() {
        assert_source_ok(concat!(
            "FUNC leaf AS Integer\n",
            "  FAIL error(1, \"leaf\")\n",
            "END FUNC\n\n",
            "FUNC relay AS Integer\n",
            "  RETURN leaf()\n",
            "  TRAP(err)\n",
            "    PROPAGATE\n",
            "  END TRAP\n",
            "END FUNC\n\n",
            "FUNC main AS Integer\n",
            "  RETURN relay()\n",
            "  TRAP(err)\n",
            "    RETURN err.code\n",
            "  END TRAP\n",
            "END FUNC\n",
        ));
    }

    // --- ISOLATED FUNC type name ---

    #[test]
    fn isolated_func_type_name_resolves() {
        assert_source_ok(concat!(
            "SUB run(job AS ISOLATED FUNC(Integer) AS Integer)\n",
            "  LET r AS Integer = job(1)\n",
            "END SUB\n\n",
            "SUB main()\n",
            "  run(LAMBDA(n AS Integer) -> n + 1)\n",
            "END SUB\n",
        ));
    }

    // --- DOC func header naming a type (funcs miss, types hit) ---

    #[test]
    fn doc_func_names_a_type_reports() {
        // `FUNC Point` — Point is a TYPE, so the func lookup misses but the type
        // table contains it → DOC_NAME_MISMATCH.
        assert!(resolve_source_fails(concat!(
            "DOC\n  FUNC Point\n  DESC Point is a type, not a func.\nEND DOC\n",
            "EXPORT TYPE Point\n  x AS Integer\nEND TYPE\n\n",
            "SUB main()\nEND SUB\n",
        )));
    }

    // --- duplicate DOC block for an overloaded FUNC (with header params) ---

    #[test]
    fn doc_duplicate_overload_block_reports() {
        assert!(resolve_source_fails(concat!(
            "DOC\n  FUNC scale(Integer)\n  ARG n first.\nEND DOC\n",
            "DOC\n  FUNC scale(Integer)\n  ARG n duplicate.\nEND DOC\n",
            "EXPORT FUNC scale(n AS Integer) AS Integer\n  RETURN n\nEND FUNC\n\n",
            "SUB main()\nEND SUB\n",
        )));
    }
}
