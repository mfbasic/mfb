use super::*;

mod body;
mod docs;
mod types;
use types::peel_res;

impl Resolver<'_> {
    pub(super) fn resolve(&mut self) {
        for file in &self.hir.files {
            self.resolve_file(file);
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
}

#[cfg(test)]
mod tests;
