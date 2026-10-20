//! plan-176-N: the shape checker's declarations — the walker's constructor, the project,
//! imported packages, files, functions and `LINK` blocks. The `Walker` struct and the free
//! helpers live in `ir/shape/mod.rs`.

use super::*;

impl<'a> Walker<'a> {
    pub(super) fn new(
        project_dir: &'a Path,
        facts: &'a LowerFacts,
        hir: &HirProject,
        imported_types: &[ImportedTypeDef],
        imported_signatures: &'a HashMap<String, ExternalSignature>,
        imported_resource_types: &[String],
    ) -> Self {
        // plan-111-B: a resource type is a nominal, so the set holds types.
        let resource_types: HashSet<ParameterType> =
            super::super::lower_link::native_resource_types(hir)
                .into_iter()
                .chain(
                    imported_resource_types
                        .iter()
                        .map(|name| ParameterType::declared(name)),
                )
                .collect();
        // Declared unions with their own variants and INCLUDES, for the
        // transitive expansion below (a `UNION B INCLUDES A` matches A's
        // variants too — the checker's and lowering's shared rule).
        let union_decls: HashMap<&str, (&crate::hir::HirTypeDecl, &str)> = hir
            .files
            .iter()
            .flat_map(|file| file.items.iter().map(move |item| (file, item)))
            .filter_map(|(file, item)| match item {
                HirItem::Type(type_decl) if type_decl.kind == crate::ast::TypeDeclKind::Union => {
                    Some((type_decl.name.as_str(), (type_decl, file.path.as_str())))
                }
                _ => None,
            })
            .collect();
        fn expanded_variants(
            name: &str,
            union_decls: &HashMap<&str, (&crate::hir::HirTypeDecl, &str)>,
            visiting: &mut HashSet<String>,
        ) -> Vec<String> {
            let Some((type_decl, _)) = union_decls.get(name) else {
                return Vec::new();
            };
            if !visiting.insert(name.to_string()) {
                return Vec::new();
            }
            let mut variants = Vec::new();
            for include in &type_decl.includes {
                variants.extend(expanded_variants(
                    include.name().as_ref(),
                    union_decls,
                    visiting,
                ));
            }
            variants.extend(
                type_decl
                    .variants
                    .iter()
                    .map(|variant| variant.type_.name().into_owned()),
            );
            visiting.remove(name);
            variants
        }
        let mut types = HashMap::new();
        for file in &hir.files {
            for item in &file.items {
                if let HirItem::Type(type_decl) = item {
                    let id = types.len();
                    let variants = if type_decl.kind == crate::ast::TypeDeclKind::Union {
                        expanded_variants(&type_decl.name, &union_decls, &mut HashSet::new())
                    } else {
                        Vec::new()
                    };
                    types.insert(
                        ParameterType::declared(&type_decl.name),
                        TypeShape {
                            id,
                            variants,
                            is_record: type_decl.kind == crate::ast::TypeDeclKind::Type,
                            is_union: type_decl.kind == crate::ast::TypeDeclKind::Union,
                            is_enum: type_decl.kind == crate::ast::TypeDeclKind::Enum,
                            fields: type_decl
                                .fields
                                .iter()
                                .map(|field| field.type_.clone())
                                .collect(),
                            variant_types: type_decl
                                .variants
                                .iter()
                                .map(|variant| variant.type_.clone())
                                .collect(),
                            members: type_decl
                                .members
                                .iter()
                                .map(|member| member.name.clone())
                                .collect(),
                            visibility: type_decl.visibility,
                            file: file.path.clone(),
                        },
                    );
                }
            }
        }
        for imported in imported_types {
            let id = types.len();
            let variants = if imported.kind == super::super::ImportedTypeKind::Union {
                imported
                    .variants
                    .iter()
                    .map(|variant| variant.name.clone())
                    .collect()
            } else {
                Vec::new()
            };
            types.insert(
                ParameterType::declared(&imported.name),
                TypeShape {
                    id,
                    variants,
                    is_record: imported.kind == super::super::ImportedTypeKind::Record,
                    is_union: imported.kind == super::super::ImportedTypeKind::Union,
                    is_enum: imported.kind == super::super::ImportedTypeKind::Enum,
                    fields: imported
                        .fields
                        .iter()
                        .map(|field| field.type_.clone())
                        .collect(),
                    variant_types: imported
                        .variants
                        .iter()
                        .flat_map(|variant| variant.fields.iter())
                        .map(|field| field.type_.clone())
                        .collect(),
                    members: imported.members.clone(),
                    visibility: Visibility::Export,
                    file: String::new(),
                },
            );
        }
        let mut functions = HashMap::new();
        for file in &hir.files {
            for item in &file.items {
                if let HirItem::Function(function) = item {
                    functions.insert(
                        function.name.clone(),
                        DeclaredFunction {
                            params: function
                                .params
                                .iter()
                                .map(|param| ShapeParam {
                                    name: param.name.clone(),
                                    type_: param.type_.clone(),
                                    has_default: param.default.is_some(),
                                })
                                .collect(),
                            visibility: function.visibility,
                            owner_file: file.path.clone(),
                            isolated: function.isolated,
                        },
                    );
                }
            }
        }
        let astrings_imported = hir.files.iter().any(|file| {
            file.imports
                .iter()
                .any(|import| import.package_name() == "astrings")
        });
        // bug-466: the builtin records whose FIELDS need their own package's
        // `IMPORT`. A nominal the project itself declares (in a file the
        // compiler did not inject) or an imported `.mfp` exports means that
        // type, not the builtin's, so it is excluded — as is a name two builtin
        // packages both declare, which no single import would name.
        //
        // `shadowed` is read off the `types` table built above rather than
        // re-walking the HIR: that table already holds exactly these two
        // populations, keyed by the very `ParameterType` a lookup will use, so
        // the two cannot disagree about a key. An entry's `file` is the
        // declaring file, or empty for an imported type.
        let internal_files: HashSet<&str> = hir
            .files
            .iter()
            .filter(|file| file.internal)
            .map(|file| file.path.as_str())
            .collect();
        let shadowed: HashSet<&ParameterType> = types
            .iter()
            .filter(|(_, shape)| !internal_files.contains(shape.file.as_str()))
            .map(|(type_, _)| type_)
            .collect();
        let mut builtin_record_owner: HashMap<ParameterType, &'static str> = HashMap::new();
        let mut ambiguous: HashSet<ParameterType> = HashSet::new();
        for package in crate::codegen::registry::registry().packages() {
            for record in package.records() {
                // bug-480 Phase 4b: a builtin record's DECLARED identity is
                // package-qualified (`net.Address`), so that is the type a value
                // of it now carries and the key this map must use. Keyed bare, the
                // lookup missed and bug-466's gate silently stopped firing --
                // `tcp::localAddress(s).port` became readable without
                // `IMPORT net`.
                //
                // `named`, not `declared`: this is a constructor call from a
                // `&'static str`, not a grammar entry.
                let type_ =
                    ParameterType::named(&format!("{}.{}", package.import_name(), record.name));
                if shadowed.contains(&type_) {
                    continue;
                }
                if builtin_record_owner
                    .insert(type_.clone(), package.import_name())
                    .is_some()
                {
                    ambiguous.insert(type_);
                }
            }
        }
        for type_ in &ambiguous {
            builtin_record_owner.remove(type_);
        }
        Walker {
            project_dir,
            context: facts.context(),
            functions,
            imported_signatures,
            astrings_imported,
            inline_trap_types: Vec::new(),
            resource_types,
            handler_depth: 0,
            types,
            file: String::new(),
            sites: Vec::new(),
            current_line: 0,
            call_typed_unknown: false,
            call_verdicts: HashMap::new(),
            state_dropped: HashSet::new(),
            builtin_record_owner,
            current_file_internal: false,
            current_own_imports: HashSet::new(),
            diagnostics: Vec::new(),
            #[cfg(test)]
            bound_types: Vec::new(),
        }
    }

    pub(super) fn walk_project(&mut self, hir: &HirProject) {
        self.check_imported_packages(hir);
        for file in &hir.files {
            self.walk_file(file);
        }
    }

    /// PACKAGE_INVALID (plan-107-D row 20, an (I) relocation from the source
    /// checker's package collectors): an imported package's three metadata
    /// tables must decode, and every type its exported records/unions reference
    /// must be declared — here or in an imported package — with comparable map
    /// keys. The container as a whole is verified at the decode boundary
    /// (`verify_and_report_packages`, before any checker runs), so what the
    /// three read checks catch is an unreadable TABLE inside a well-formed
    /// container. The type walk lives here rather than at that boundary because
    /// it needs the full type table and resource registry this pass already
    /// owns (`is_comparable`). The checker walked types + resources per import
    /// and the function signatures in a second pass; the order is kept.
    fn check_imported_packages(&mut self, hir: &HirProject) {
        let mut seen_packages = HashSet::new();
        for file in &hir.files {
            self.file = file.path.clone();
            for import in &file.imports {
                let package = import.package_name();
                if builtins::is_builtin_import(package)
                    || !seen_packages.insert(package.to_string())
                {
                    continue;
                }
                // bug-480: resolve the compiled interface through the shared
                // resolver, so a dependency declared by source directory (whose
                // `.mfp` this build compiled into `build/packages/`) is walked
                // exactly like an installed one instead of being skipped.
                let Some(package_file) =
                    crate::manifest::package::resolved_package_file(self.project_dir, package)
                else {
                    continue;
                };
                // bug-632: this pass's tables key an imported type by its
                // package-qualified identity (`manifest::package::imported_type_def`),
                // so the package's own bare `.mfp` spellings are qualified the same
                // way before they are looked up.
                let owners = crate::binary_repr::BinaryReprPackageDecode::read(&package_file)
                    .map(|decode| crate::manifest::package::package_type_owners(&decode, package))
                    .unwrap_or_default();
                match crate::binary_repr::read_package_type_exports(&package_file) {
                    Ok(type_exports) => {
                        for export in &type_exports {
                            let context = match export.kind {
                                crate::binary_repr::BinaryReprExportKind::Type => {
                                    format!("exported type `{}`", export.name)
                                }
                                crate::binary_repr::BinaryReprExportKind::Union => {
                                    format!("exported union `{}`", export.name)
                                }
                                _ => continue,
                            };
                            self.validate_package_type(
                                &package_file,
                                &crate::manifest::package::qualify_package_type(
                                    &ParameterType::declared(&export.name),
                                    &owners,
                                ),
                                &context,
                                import.line,
                                &mut HashSet::new(),
                            );
                        }
                    }
                    Err(_) => self.emit(
                        "PACKAGE_INVALID",
                        format!(
                            "Imported package `{package}` has unreadable or invalid type metadata."
                        ),
                        import.line,
                    ),
                }
                if crate::binary_repr::read_package_resources(&package_file).is_err() {
                    self.emit(
                        "PACKAGE_INVALID",
                        format!(
                            "Imported package `{}` has an unreadable resource table.",
                            package_file.display()
                        ),
                        import.line,
                    );
                }
            }
        }
        let mut seen_bindings = HashSet::new();
        for file in &hir.files {
            self.file = file.path.clone();
            for import in &file.imports {
                let package = import.package_name();
                if builtins::is_builtin_import(package)
                    || !seen_bindings.insert(import.binding_name().to_string())
                {
                    continue;
                }
                // bug-480: resolve the compiled interface through the shared
                // resolver, so a dependency declared by source directory (whose
                // `.mfp` this build compiled into `build/packages/`) is walked
                // exactly like an installed one instead of being skipped.
                let Some(package_file) =
                    crate::manifest::package::resolved_package_file(self.project_dir, package)
                else {
                    continue;
                };
                // bug-632: qualified exactly as the type-export walk above.
                let owners = crate::binary_repr::BinaryReprPackageDecode::read(&package_file)
                    .map(|decode| crate::manifest::package::package_type_owners(&decode, package))
                    .unwrap_or_default();
                let qualify = |type_: &ParameterType| {
                    crate::manifest::package::qualify_package_type(type_, &owners)
                };
                match crate::binary_repr::read_package_exports(&package_file) {
                    Ok(exports) => {
                        for export in &exports {
                            if !matches!(
                                export.kind,
                                crate::binary_repr::BinaryReprExportKind::Func
                            ) {
                                continue;
                            }
                            // Every exported function's parameter and return
                            // types (the checker's
                            // `validate_imported_function_signature`).
                            let mut seen = HashSet::new();
                            for param in &export.params {
                                self.validate_package_type(
                                    &package_file,
                                    &qualify(&param.type_),
                                    &format!(
                                        "exported function `{}` parameter `{}`",
                                        export.name, param.name
                                    ),
                                    import.line,
                                    &mut seen,
                                );
                            }
                            self.validate_package_type(
                                &package_file,
                                &qualify(&export.return_type),
                                &format!("exported function `{}` return type", export.name),
                                import.line,
                                &mut seen,
                            );
                        }
                    }
                    Err(_) => self.emit(
                        "PACKAGE_INVALID",
                        format!(
                            "Imported package `{package}` has unreadable or invalid function metadata."
                        ),
                        import.line,
                    ),
                }
            }
        }
    }

    /// The checker's `validate_package_metadata_type`: every nominal a package
    /// type reaches must be declared (a resource or a built-in nominal is always
    /// in scope), and a map key must be comparable.
    pub(super) fn validate_package_type(
        &mut self,
        package_file: &Path,
        type_: &ParameterType,
        context: &str,
        line: usize,
        seen: &mut HashSet<ParameterType>,
    ) {
        match type_ {
            ParameterType::ListOf(element)
            | ParameterType::SetOf(element)
            | ParameterType::ResultOf(element)
            | ParameterType::Res(element) => {
                self.validate_package_type(package_file, element, context, line, seen);
            }
            ParameterType::MapOf(key, value) => {
                self.validate_package_type(package_file, key, context, line, seen);
                self.validate_package_type(package_file, value, context, line, seen);
                if !self.is_comparable(key) {
                    self.emit(
                        "PACKAGE_INVALID",
                        format!(
                            "Imported package `{}` has {context} with non-comparable map key type `{}`.",
                            package_file.display(),
                            key.name()
                        ),
                        line,
                    );
                }
            }
            ParameterType::Func(params, return_type, _) => {
                for param in params {
                    self.validate_package_type(package_file, param, context, line, seen);
                }
                self.validate_package_type(package_file, return_type, context, line, seen);
            }
            ParameterType::ThreadHandle { msg, res, out, .. } => {
                self.validate_package_type(package_file, msg, context, line, seen);
                // An absent resource plane is `Nothing`; the plane's ` STATE T`
                // rides inside its spelling (plan-106-C rung 2e).
                let (plane_resource, plane_state) = res.split_state();
                if !matches!(plane_resource, ParameterType::Nothing) {
                    self.validate_package_type(package_file, &plane_resource, context, line, seen);
                }
                if let Some(plane_state) = &plane_state {
                    self.validate_package_type(package_file, plane_state, context, line, seen);
                }
                self.validate_package_type(package_file, out, context, line, seen);
            }
            ParameterType::Named(_) => {
                let name = type_.clone();
                if is_always_in_scope_nominal(&name) {
                    return;
                }
                if self.is_resource_type(type_) || !seen.insert(name.clone()) {
                    return;
                }
                let Some(shape) = self.types.get(type_) else {
                    if is_builtin_package_type(type_) {
                        return;
                    }
                    self.emit(
                        "PACKAGE_INVALID",
                        format!(
                            "Imported package `{}` has {context} that references unknown type `{name}`.",
                            package_file.display()
                        ),
                        line,
                    );
                    return;
                };
                let referenced = if shape.is_record {
                    shape.fields.clone()
                } else if shape.is_union {
                    shape.variant_types.clone()
                } else {
                    Vec::new()
                };
                for referenced in &referenced {
                    self.validate_package_type(package_file, referenced, context, line, seen);
                }
                seen.remove(&name);
            }
            // A stateful resource, as an imported signature spells it
            // (`Db STATE DbInfo`). plan-111-B: this arm is new because the
            // clause is now STRUCTURE. Before it, the type reached the `other`
            // arm below, was re-wrapped as one opaque `Named`, and the old
            // `is_resource_type(&str)` split the base back out of that
            // spelling — accepting the whole thing without walking the STATE
            // payload. `without_state` cannot peel a re-wrapped nominal, so the
            // peel has to happen before the re-wrap. Same outcome, same
            // non-recursion; a stateful type whose base is NOT a resource still
            // falls through and is reported by its full spelling.
            ParameterType::Stateful { .. } if self.is_resource_type(type_) => {}
            ParameterType::Boolean
            | ParameterType::Byte
            | ParameterType::Fixed
            | ParameterType::Float
            | ParameterType::Integer
            | ParameterType::Money
            | ParameterType::Nothing
            | ParameterType::String
            | ParameterType::Unknown => {}
            // Every other spelling took the checker's nominal arm.
            other => self.validate_package_type(
                package_file,
                &ParameterType::named(&other.name()),
                context,
                line,
                seen,
            ),
        }
    }

    fn walk_file(&mut self, file: &HirFile) {
        self.context.current_imports = file.import_bindings();
        self.context.current_file = file.path.clone();
        self.file = file.path.clone();
        self.current_file_internal = file.internal;
        self.current_own_imports = file.own_imports.iter().cloned().collect();
        for item in &file.items {
            match item {
                HirItem::Binding(binding) => {
                    // A top-level binding's `STATE` clause rides in its lowered
                    // type (bug-1030), so both clause rules are `ir::verify`'s
                    // (`check_state_clauses`), as for a local binding.
                    if let Some(value) = &binding.value {
                        let locals = HashMap::new();
                        self.current_line = binding.line;
                        self.walk_expression(value, &locals);
                        self.check_initializer_known(
                            &binding.name,
                            value,
                            &locals,
                            binding.explicit_type.then_some(&binding.type_),
                            binding.line,
                        );
                    }
                }
                HirItem::Function(function) => {
                    self.sites = function.instantiated_at.clone();
                    self.walk_function(function);
                    self.sites.clear();
                }
                HirItem::Link(link) => self.walk_link(link),
                // Declarations without executable bodies: their rules are
                // `ir::verify`'s (types, LINK blocks, resources) or the parser's.
                HirItem::Type(_)
                | HirItem::Resource(_)
                | HirItem::FuncAlias(_)
                | HirItem::Doc(_)
                | HirItem::Testing(_) => {}
            }
        }
    }

    /// TYPE_STATE_REQUIRES_RESOURCE (bug-998): a `STATE` clause written on a
    /// parameter or a return whose type is not a resource.
    ///
    /// Erased fact: the clause itself. `hir::resource_params` takes it off a
    /// parameter or a return before lowering (`HirParam::stray_state`,
    /// `HirFunction::return_stray_state`), because a clause left on a data type
    /// rides into the type (`Integer STATE Cur`) and makes every use of the
    /// parameter untypable — the author used to see `TYPE_UNKNOWN_VALUE` at each
    /// use and `TYPE_STATE_MISMATCH` at each call, and nothing at the clause.
    /// So the IR `ir::verify` reads does not carry it, and this is the one place
    /// that can still name it. (`ir::verify` keeps the same rule for the
    /// positions lowering does not erase — a binding, local or top-level, a
    /// field, an element, a thread plane, a native declaration — and for
    /// decoded package IR.)
    ///
    /// The clause's own type is not examined (`TYPE_STATE_INVALID`): with no
    /// resource to attach to, there is no payload to be the wrong kind of.
    fn check_stray_state(
        &mut self,
        subject: &str,
        host: &ParameterType,
        state: &ParameterType,
        line: usize,
    ) {
        self.emit(
            "TYPE_STATE_REQUIRES_RESOURCE",
            format!(
                "{subject} attaches `STATE {state}` to `{host}`, which is not a resource; remove the STATE clause."
            ),
            line,
        );
    }

    pub(super) fn check_stray_param_state(&mut self, param: &crate::hir::HirParam, line: usize) {
        if let Some(state) = &param.stray_state {
            self.check_stray_state(
                &format!("Parameter `{}`", param.name),
                &param.type_,
                state,
                line,
            );
        }
    }

    fn walk_function(&mut self, function: &HirFunction) {
        // Parameter locals as `lower_function` seeds them: a resource parameter's
        // `STATE T` rides in its type.
        let mut locals = HashMap::new();
        // TYPE_STATE_REQUIRES_RESOURCE, the return and parameter forms (see
        // `check_stray_state`).
        if let Some(state) = &function.return_stray_state {
            self.check_stray_state(
                &format!("FUNC `{}` return", function.name),
                &function.returns,
                state,
                function.line,
            );
        }
        for param in &function.params {
            self.check_stray_param_state(param, param.line);
            let type_ = match &param.state_type {
                Some(state) => param.type_.with_state(state),
                None => param.type_.clone(),
            };
            if let Some(default) = &param.default {
                // plan-136-A: a default is lowered at the declaration with no
                // locals, so it is walked with none.
                let no_locals = HashMap::new();
                self.current_line = param.line;
                self.walk_expression(default, &no_locals);
                if self.checker_types_unknown(default, &no_locals, Some(&param.type_)) {
                    self.emit(
                        "TYPE_UNKNOWN_VALUE",
                        format!(
                            "Default value for `{}` does not have a known type.",
                            param.name
                        ),
                        param.line,
                    );
                }
            }
            locals.insert(param.name.clone(), type_);
        }
        let previous_return_type = self.context.current_return_type.take();
        self.context.current_return_type = Some(lower::function_return_type(function));
        self.state_dropped.clear();
        // The body's top-level locals stay visible to the function-level
        // TRAP body (bug-285), exactly as `lower_function_body` scopes them.
        let mut body_locals = locals.clone();
        self.walk_statements_flow(&function.body, &mut body_locals);
        if let Some(trap) = &function.trap {
            let mut trap_locals = body_locals;
            trap_locals.insert(trap.name.clone(), ParameterType::named("Error"));
            self.walk_block(&trap.body, &trap_locals);
        }
        self.context.current_return_type = previous_return_type;
    }

    /// The two native-ABI facts lowering erases from a `LINK` block (every
    /// other native rule is `ir::verify`'s, plan-107-C):
    ///
    /// - NATIVE_CONST_UNKNOWN_SLOT, the "not a constant the compiler can fold"
    ///   form: lowering folds a CONST pin's expression to its immediate
    ///   (`eval_link_const`), so the IR holds a number where the source held
    ///   `"literal"` or `1 + 1`. (The unknown-slot form is verify's.)
    /// - NATIVE_FREE_INVALID, the deallocator-signature form: `IrFree` keeps
    ///   only the freed slot and the symbol; the `RETURN`ed slot, the return
    ///   ctype and the deallocator's parameter/return ctypes are gone. (The
    ///   `AS RES` producer form and the empty-symbol form are verify's — both
    ///   end the checker's FREE check, so neither doubles with this one.)
    fn walk_link(&mut self, link: &crate::hir::HirLinkBlock) {
        let cstructs: Vec<&str> = link.cstructs.iter().map(|c| c.name.as_str()).collect();
        for function in &link.functions {
            for pin in &function.consts {
                if !link_const_foldable(&pin.value, &cstructs) {
                    self.emit(
                        "NATIVE_CONST_UNKNOWN_SLOT",
                        format!(
                            "Native function `{}` CONST pin `{}` is not a constant the compiler can fold: it must be an integer or boolean literal, NOTHING, or SIZEOF <CStruct>.",
                            function.name, pin.slot
                        ),
                        pin.line,
                    );
                }
            }
            let Some(free) = &function.free else {
                continue;
            };
            if function.return_resource || free.symbol.is_empty() {
                continue;
            }
            // The freed slot must be the C return, that return must be what
            // `RETURN` surfaces, it must be a CPtr copied into an owned wrapper
            // value, and the deallocator takes one CPtr and returns CVoid.
            let returns_the_c_value = matches!(
                &function.result,
                Some(crate::ast::Expression::Identifier(name)) if *name == function.abi.return_name
            );
            let well_formed = free.slot == function.abi.return_name
                && returns_the_c_value
                && function.abi.return_ctype.is_c_abi(CAbiType::Ptr)
                && free.param_ctype.is_c_abi(CAbiType::Ptr)
                && free.return_ctype.is_c_abi(CAbiType::Void);
            if !well_formed {
                self.emit(
                    "NATIVE_FREE_INVALID",
                    format!(
                        "Native function `{}` has a malformed FREE block: it must name the CPtr produced slot that `RETURN` surfaces, and its deallocator must take one CPtr parameter and return CVoid.",
                        function.name
                    ),
                    free.line,
                );
            }
        }
    }
}
