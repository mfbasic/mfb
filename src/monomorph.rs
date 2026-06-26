use crate::ast::{
    AstFile, AstProject, CallArg, ConstructorArg, Expression, Function, Item, MatchCase,
    MatchPattern, RecordUpdate, Statement, TopLevelBinding, TypeDecl, TypeDeclKind, TypeField,
    UnionVariant,
};
use crate::numeric;
use crate::rules;
use std::collections::{HashMap, HashSet};
use std::path::Path;

pub fn monomorphize_project(project_dir: &Path, ast: &AstProject) -> Result<AstProject, ()> {
    let mut mono = Monomorphizer::new(project_dir, ast);
    mono.run();
    if mono.had_error {
        Err(())
    } else {
        Ok(mono.into_project())
    }
}

struct Monomorphizer<'a> {
    project_dir: &'a Path,
    source: &'a AstProject,
    type_templates: HashMap<String, TypeDecl>,
    function_templates: HashMap<String, Function>,
    concrete_types: HashMap<String, TypeDecl>,
    concrete_functions: HashMap<String, Function>,
    function_overloads: HashMap<String, Vec<Function>>,
    overload_names: HashMap<String, String>,
    /// Overloaded functions exported by imported packages, keyed by the
    /// importer-facing `binding.base` name. Lets a call to an imported overload
    /// be rewritten to the package's mangled `package.base$Types` name, which the
    /// package merge then identity-prefixes (plan-linker.md §12, overloads).
    imported_overloads: HashMap<String, Vec<ImportedOverload>>,
    /// All known import-binding/package qualifier prefixes (e.g. `sqlite.`), used
    /// to normalize an argument's qualified user/resource type to the bare name
    /// the package stored in its mangled overload names.
    package_qualifiers: Vec<String>,
    type_instantiations: HashMap<String, (String, Vec<String>)>,
    emitted_type_keys: HashSet<String>,
    emitted_function_keys: HashSet<String>,
    /// Import-binding names that refer to the built-in `collections` package
    /// (including aliases). A call `binding.member` with `binding` in this set
    /// and `member` a `collections::` function is rewritten to the internal
    /// generic implementation `__collections_member` before instantiation.
    collections_bindings: HashSet<String>,
    had_error: bool,
}

/// One overload of an imported package function.
struct ImportedOverload {
    /// Declared parameter types in order (bare, as the package stored them).
    param_types: Vec<String>,
    /// The fully package-qualified mangled name (`package.base$Types`) the merge
    /// expects.
    qualified_name: String,
}

#[derive(Default)]
struct FunctionContext {
    locals: HashMap<String, String>,
    function_returns: HashMap<String, String>,
    function_types: HashMap<String, String>,
    record_fields: HashMap<String, Vec<TypeField>>,
}

impl<'a> Monomorphizer<'a> {
    fn new(project_dir: &'a Path, source: &'a AstProject) -> Self {
        let mut type_templates = HashMap::new();
        let mut function_templates = HashMap::new();
        let mut concrete_types = HashMap::new();
        let mut concrete_functions = HashMap::new();
        let mut function_overloads: HashMap<String, Vec<Function>> = HashMap::new();
        let mut overload_names = HashMap::new();

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
                        function_templates.insert(function.name.clone(), function.clone());
                    }
                    Item::Function(function) => {
                        function_overloads
                            .entry(function.name.clone())
                            .or_default()
                            .push(function.clone());
                    }
                    // Native LINK resources, re-export aliases, and LINK blocks
                    // carry no template parameters and are passed through
                    // unchanged (plan-link-update.md §15 Phase 1).
                    Item::Resource(_) | Item::FuncAlias(_) | Item::Link(_) => {}
                }
            }
        }

        for functions in function_overloads.values() {
            for function in functions {
                let concrete_name = overload_concrete_name(function, functions.len() > 1);
                overload_names.insert(
                    overload_key(&function.name, &function.params),
                    concrete_name.clone(),
                );
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
    fn resolve_imported_overload(&self, callee: &str, arg_types: &[String]) -> Option<String> {
        let candidates = self.imported_overloads.get(callee)?;
        candidates
            .iter()
            .find(|candidate| {
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
        let mut normalized = type_.to_string();
        for qualifier in &self.package_qualifiers {
            normalized = normalized.replace(qualifier, "");
        }
        normalized
    }

    fn run(&mut self) {
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

    fn into_project(mut self) -> AstProject {
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
                                .get(&overload_key(&function.name, &function.params))
                                .map(String::as_str)
                                .unwrap_or(&function.name);
                            if let Some(concrete) = self.concrete_functions.get(concrete_name) {
                                emitted_functions.insert(concrete.name.clone());
                                items.push(Item::Function(concrete.clone()));
                            }
                        }
                        // Native LINK constructs are not monomorphized; preserve
                        // them verbatim so later stages (resolve, typecheck,
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

    fn resolve_overload(&self, name: &str, arg_types: &[String]) -> Option<String> {
        let candidates = self.function_overloads.get(name)?;
        if candidates.len() <= 1 {
            return None;
        }
        candidates
            .iter()
            .find(|function| {
                function.params.len() == arg_types.len()
                    && function
                        .params
                        .iter()
                        .zip(arg_types.iter())
                        .all(|(param, actual)| param.type_name.as_deref() == Some(actual.as_str()))
            })
            .and_then(|function| {
                self.overload_names
                    .get(&overload_key(name, &function.params))
            })
            .cloned()
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
                value: value
                    .as_ref()
                    .map(|value| self.lower_expression(value, substitutions, context, None, *line)),
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
                let lowered_args =
                    arguments
                        .iter()
                        .map(|argument| match argument {
                            CallArg::Positional(value) => CallArg::Positional(
                                self.lower_expression(value, substitutions, context, None, line),
                            ),
                            CallArg::Named { name, value, line } => CallArg::Named {
                                name: name.clone(),
                                value: self.lower_expression(
                                    value,
                                    substitutions,
                                    context,
                                    None,
                                    *line,
                                ),
                                line: *line,
                            },
                        })
                        .collect::<Vec<_>>();
                let arg_types = lowered_args
                    .iter()
                    .filter_map(|argument| self.expression_type(call_arg_value(argument), context))
                    .collect::<Vec<_>>();
                // Rewrite a `collections::` call onto its internal generic
                // implementation so it instantiates like any generic function.
                let callee = &self
                    .collections_internal_callee(callee)
                    .unwrap_or_else(|| callee.clone());
                let target = self
                    .instantiate_function(callee, &arg_types, line)
                    .or_else(|| self.resolve_overload(callee, &arg_types))
                    .or_else(|| self.resolve_imported_overload(callee, &arg_types))
                    .unwrap_or_else(|| callee.clone());
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

    fn expression_type(
        &self,
        expression: &Expression,
        context: &FunctionContext,
    ) -> Option<String> {
        match expression {
            Expression::String(_) => Some("String".to_string()),
            Expression::Number(value) => Some(if value.contains('.') {
                "Float".to_string()
            } else {
                "Integer".to_string()
            }),
            Expression::Boolean(_) => Some("Boolean".to_string()),
            Expression::Identifier(value) if value == "NOTHING" => Some("Nothing".to_string()),
            Expression::Identifier(value) => context
                .locals
                .get(value)
                .cloned()
                .or_else(|| context.function_types.get(value).cloned()),
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
            Expression::Call { callee, .. } => context.function_returns.get(callee).cloned(),
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
        let path = self
            .source
            .files
            .first()
            .map(|file| self.project_dir.join(&file.path))
            .unwrap_or_else(|| self.project_dir.join("src/main.mfb"));
        rules::show_diagnostic(rule, detail, &path, line, 1, 1);
    }
}

fn call_arg_value(argument: &CallArg) -> &Expression {
    match argument {
        CallArg::Positional(value) => value,
        CallArg::Named { value, .. } => value,
    }
}

fn constructor_arg_field_type<'a>(
    argument: &ConstructorArg,
    index: usize,
    fields: Option<&'a [TypeField]>,
) -> Option<&'a str> {
    let fields = fields?;
    match argument {
        ConstructorArg::Positional(_) => fields.get(index).map(|field| field.type_name.as_str()),
        ConstructorArg::Named { name, .. } => fields
            .iter()
            .find(|field| field.name == *name)
            .map(|field| field.type_name.as_str()),
    }
}

impl Clone for FunctionContext {
    fn clone(&self) -> Self {
        Self {
            locals: self.locals.clone(),
            function_returns: self.function_returns.clone(),
            function_types: self.function_types.clone(),
            record_fields: self.record_fields.clone(),
        }
    }
}

fn unify_type(
    pattern: &str,
    actual: &str,
    params: &[String],
    substitutions: &mut HashMap<String, String>,
) -> bool {
    if params.iter().any(|param| param == pattern) {
        if let Some(existing) = substitutions.get(pattern) {
            return existing == actual;
        }
        substitutions.insert(pattern.to_string(), actual.to_string());
        return true;
    }

    if let Some(pattern_element) = pattern.strip_prefix("List OF ") {
        let Some(actual_element) = actual.strip_prefix("List OF ") else {
            return false;
        };
        return unify_type(pattern_element, actual_element, params, substitutions);
    }
    if let Some(pattern_success) = pattern.strip_prefix("Result OF ") {
        let Some(actual_success) = actual.strip_prefix("Result OF ") else {
            return false;
        };
        return unify_type(pattern_success, actual_success, params, substitutions);
    }
    if let Some(pattern_rest) = pattern.strip_prefix("Map OF ") {
        let Some(actual_rest) = actual.strip_prefix("Map OF ") else {
            return false;
        };
        let Some((pattern_key, pattern_value)) = split_top_level_to(pattern_rest) else {
            return false;
        };
        let Some((actual_key, actual_value)) = split_top_level_to(actual_rest) else {
            return false;
        };
        return unify_type(&pattern_key, &actual_key, params, substitutions)
            && unify_type(&pattern_value, &actual_value, params, substitutions);
    }
    if let Some(pattern_rest) = pattern.strip_prefix("MapEntry OF ") {
        let Some(actual_rest) = actual.strip_prefix("MapEntry OF ") else {
            return false;
        };
        let Some((pattern_key, pattern_value)) = split_top_level_to(pattern_rest) else {
            return false;
        };
        let Some((actual_key, actual_value)) = split_top_level_to(actual_rest) else {
            return false;
        };
        return unify_type(&pattern_key, &actual_key, params, substitutions)
            && unify_type(&pattern_value, &actual_value, params, substitutions);
    }
    if let Some((pattern_kind, pattern_message, pattern_resource, pattern_output)) =
        crate::builtins::thread::thread_parts_full(pattern)
    {
        let Some((actual_kind, actual_message, actual_resource, actual_output)) =
            crate::builtins::thread::thread_parts_full(actual)
        else {
            return false;
        };
        let resource_unifies = match (pattern_resource, actual_resource) {
            (None, None) => true,
            (Some(pattern_resource), Some(actual_resource)) => {
                unify_type(pattern_resource, actual_resource, params, substitutions)
            }
            _ => false,
        };
        return pattern_kind == actual_kind
            && unify_type(pattern_message, actual_message, params, substitutions)
            && resource_unifies
            && unify_type(pattern_output, actual_output, params, substitutions);
    }
    if let (Some((pattern_name, pattern_args)), Some((actual_name, actual_args))) =
        (user_template_parts(pattern), user_template_parts(actual))
    {
        return pattern_name == actual_name
            && pattern_args.len() == actual_args.len()
            && pattern_args
                .iter()
                .zip(actual_args.iter())
                .all(|(pattern, actual)| unify_type(pattern, actual, params, substitutions));
    }

    if let (Some((pattern_params, pattern_ret)), Some((actual_params, actual_ret))) =
        (func_type_parts(pattern), func_type_parts(actual))
    {
        return pattern_params.len() == actual_params.len()
            && pattern_params
                .iter()
                .zip(actual_params.iter())
                .all(|(pattern, actual)| unify_type(pattern, actual, params, substitutions))
            && unify_type(pattern_ret, actual_ret, params, substitutions);
    }

    pattern == actual || actual == "Unknown"
}

/// Splits a function type `FUNC(p1, p2) AS Ret` (or `ISOLATED FUNC(...) AS Ret`)
/// into its parameter types and return type for template unification. Parameters
/// are split on top-level `", "`, matching the rest of the builtin type parsing.
fn func_type_parts(type_name: &str) -> Option<(Vec<&str>, &str)> {
    let rest = type_name
        .strip_prefix("FUNC(")
        .or_else(|| type_name.strip_prefix("ISOLATED FUNC("))?;
    let (params, ret) = rest.split_once(") AS ")?;
    let params = if params.trim().is_empty() {
        Vec::new()
    } else {
        params.split(", ").collect()
    };
    Some((params, ret))
}

fn user_template_parts(type_name: &str) -> Option<(String, Vec<String>)> {
    if type_name.starts_with("List OF ")
        || type_name.starts_with("Map OF ")
        || type_name.starts_with("MapEntry OF ")
        || type_name.starts_with("Result OF ")
        || type_name.starts_with("Thread OF ")
        || type_name.starts_with("ThreadWorker OF ")
        || type_name.starts_with("FUNC(")
        || type_name.starts_with("ISOLATED FUNC(")
    {
        return None;
    }
    let (name, rest) = type_name.split_once(" OF ")?;
    Some((name.to_string(), split_top_level_commas(rest)))
}

fn substitute_type_params(type_name: &str, substitutions: &HashMap<String, String>) -> String {
    if let Some(value) = substitutions.get(type_name) {
        return value.clone();
    }
    if let Some(element) = type_name.strip_prefix("List OF ") {
        return format!("List OF {}", substitute_type_params(element, substitutions));
    }
    if let Some(success) = type_name.strip_prefix("Result OF ") {
        return format!(
            "Result OF {}",
            substitute_type_params(success, substitutions)
        );
    }
    if let Some(rest) = type_name.strip_prefix("Map OF ") {
        if let Some((key, value)) = split_top_level_to(rest) {
            return format!(
                "Map OF {} TO {}",
                substitute_type_params(&key, substitutions),
                substitute_type_params(&value, substitutions)
            );
        }
    }
    if let Some(rest) = type_name.strip_prefix("MapEntry OF ") {
        if let Some((key, value)) = split_top_level_to(rest) {
            return format!(
                "MapEntry OF {} TO {}",
                substitute_type_params(&key, substitutions),
                substitute_type_params(&value, substitutions)
            );
        }
    }
    if let Some((kind, message, resource, output)) =
        crate::builtins::thread::thread_parts_full(type_name)
    {
        let resource = resource.map(|resource| substitute_type_params(resource, substitutions));
        return crate::builtins::thread::format_thread_type(
            kind,
            &substitute_type_params(message, substitutions),
            resource.as_deref(),
            &substitute_type_params(output, substitutions),
        );
    }
    if let Some((name, args)) = user_template_parts(type_name) {
        let args = args
            .iter()
            .map(|arg| substitute_type_params(arg, substitutions))
            .collect::<Vec<_>>();
        return format!("{name} OF {}", args.join(", "));
    }
    type_name.to_string()
}

fn split_top_level_to(value: &str) -> Option<(String, String)> {
    value
        .split_once(" TO ")
        .map(|(left, right)| (left.to_string(), right.to_string()))
}

fn split_top_level_commas(value: &str) -> Vec<String> {
    value.split(", ").map(str::to_string).collect()
}

/// Read each imported package's exported functions and collect the overloaded
/// ones (more than one export sharing a base name), keyed by the importer-facing
/// `binding.base` name. Also returns the set of `binding.`/`package.` qualifier
/// prefixes for argument-type normalization (plan-linker.md §12, overloads).
fn collect_imported_overloads(
    project_dir: &Path,
    source: &AstProject,
) -> (HashMap<String, Vec<ImportedOverload>>, Vec<String>) {
    let mut overloads: HashMap<String, Vec<ImportedOverload>> = HashMap::new();
    let mut qualifiers: HashSet<String> = HashSet::new();
    // Distinct (binding, package) pairs across all files.
    let mut bindings: HashMap<String, String> = HashMap::new();
    for file in &source.files {
        for (binding, package) in file.import_bindings() {
            qualifiers.insert(format!("{binding}."));
            qualifiers.insert(format!("{package}."));
            bindings.insert(binding, package);
        }
    }
    for (binding, package) in &bindings {
        let package_file = project_dir.join("packages").join(format!("{package}.mfp"));
        let Ok(exports) = crate::binary_repr::read_package_exports(&package_file) else {
            continue;
        };
        // Group exported functions/subs by base name (the part before `$`).
        let mut by_base: HashMap<String, Vec<crate::binary_repr::BinaryReprExport>> =
            HashMap::new();
        for export in exports {
            if !matches!(
                export.kind,
                crate::binary_repr::BinaryReprExportKind::Func
                    | crate::binary_repr::BinaryReprExportKind::Sub
            ) {
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
            let entry = overloads.entry(format!("{binding}.{base}")).or_default();
            for export in exports {
                entry.push(ImportedOverload {
                    param_types: export
                        .params
                        .iter()
                        .map(|param| param.type_.clone())
                        .collect(),
                    qualified_name: format!("{package}.{}", export.name),
                });
            }
        }
    }
    (overloads, qualifiers.into_iter().collect())
}

fn mangle_name(name: &str, args: &[String]) -> String {
    let suffix = args
        .iter()
        .map(|arg| sanitize_type_name(arg))
        .collect::<Vec<_>>()
        .join("$");
    format!("{name}${suffix}")
}

fn overload_concrete_name(function: &Function, overloaded: bool) -> String {
    if !overloaded {
        return function.name.clone();
    }
    let args = function
        .params
        .iter()
        .map(|param| {
            param
                .type_name
                .clone()
                .unwrap_or_else(|| "Unknown".to_string())
        })
        .collect::<Vec<_>>();
    mangle_name(&function.name, &args)
}

fn overload_key(name: &str, params: &[crate::ast::Param]) -> String {
    let params = params
        .iter()
        .map(|param| {
            param
                .type_name
                .clone()
                .unwrap_or_else(|| "Unknown".to_string())
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("{name}({params})")
}

fn sanitize_type_name(value: &str) -> String {
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

fn numeric_binary_result_type(operator: &str, left: &str, right: &str) -> &'static str {
    numeric::binary_result_type(operator, left, right).unwrap_or(numeric::TYPE_INTEGER)
}

fn promote_loop_numeric_type_name(start: &str, end: &str, step: &str) -> String {
    let first = numeric_binary_result_type("+", start, end);
    numeric_binary_result_type("+", first, step).to_string()
}

fn constructor_arg_value(argument: &ConstructorArg) -> &Expression {
    match argument {
        ConstructorArg::Positional(value) => value,
        ConstructorArg::Named { value, .. } => value,
    }
}
