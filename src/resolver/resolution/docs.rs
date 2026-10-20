//! plan-176-M: resolving documentation blocks — each block's target, its named lines and
//! their checks. Item resolution lives in `resolver/resolution/mod.rs`.

use super::*;

impl Resolver<'_> {
    /// Validate every `DOC` block in the package (plan-09-doc.md §2/§4): resolve
    /// each header to a declaration of the right kind, then check the body's
    /// `ARG`/`PROP`/`RET`/`ERROR`/`EXAMPLE` lines, attributes, and `DEPRECATED`
    /// against that declaration.
    pub(in crate::resolver) fn resolve_doc_blocks(&mut self) {
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
}
