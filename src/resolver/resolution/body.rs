//! plan-176-M: resolving function bodies — blocks, statements, match patterns, expressions,
//! callables, parameter defaults, new locals and identifiers. Item resolution lives in
//! `resolver/resolution/mod.rs`.

use super::*;

impl Resolver<'_> {
    pub(super) fn resolve_block(
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
            HirStatement::Exit { .. } | HirStatement::Continue { .. } => {}
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

    pub(super) fn resolve_expression(
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
                &format!(
                    "Callable `{}` is not a top-level function.",
                    crate::internal_name::display_name(callee)
                ),
                file,
                line,
            );
        }
    }

    /// Resolve a parameter default in its function's DECLARATION scope: no locals
    /// at all, so not the function's own parameters and never a caller's locals
    /// (plan-136-A). The file's top level — globals, functions, imports and
    /// own-file `PRIVATE` names — is what remains visible.
    pub(super) fn resolve_parameter_default(
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
    pub(in crate::resolver) fn check_new_local(
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
        let shadowed_enum = self.top_level_enums.get(name).and_then(|keys| {
            keys.iter()
                .filter_map(|key| self.top_levels.get(key))
                .find(|symbol| self.visible_from(file, symbol.visibility, &symbol.file_path))
                .cloned()
        });
        if let Some(symbol) = shadowed_enum {
            self.report(
                "SYMBOL_SHADOWS_ENUM",
                &format!(
                    "`{}` is already an enum (declared at {}:{}); a local cannot reuse its name.",
                    crate::internal_name::display_name(name),
                    symbol.file_path,
                    symbol.line
                ),
                file,
                line,
            );
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
                &format!(
                    "Identifier `{}` is not declared in this scope.",
                    crate::internal_name::display_name(name)
                ),
                file,
                line,
            );
        }
    }
}
