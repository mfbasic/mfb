use crate::ast::{
    AstFile, AstProject, Expression, Item, MatchPattern, Statement, TypeDecl, TypeDeclKind,
    TypeField,
};
use crate::builtins;
use crate::rules;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use tinyjson::JsonValue;

const BUILTIN_TYPES: &[&str] = &[
    "Boolean",
    "Byte",
    "Error",
    "Fixed",
    "Float",
    "Integer",
    "Nothing",
    "Result",
    "String",
    builtins::fs::FILE_TYPE,
    builtins::io::TERMINAL_SIZE_TYPE,
];

pub fn resolve_project(
    project_dir: &Path,
    manifest: &HashMap<String, JsonValue>,
    ast: &AstProject,
) -> Result<(), ()> {
    let mut resolver = Resolver::new(project_dir, manifest, ast);
    resolver.resolve();
    if resolver.had_error {
        Err(())
    } else {
        Ok(())
    }
}

struct Resolver<'a> {
    project_dir: &'a Path,
    ast: &'a AstProject,
    dependency_packages: HashMap<String, DependencyPackage>,
    top_levels: HashMap<String, Symbol>,
    functions: HashMap<String, Vec<FunctionSymbol>>,
    types: HashSet<String>,
    variant_constructors: HashSet<String>,
    active_template_params: HashSet<String>,
    had_error: bool,
}

#[derive(Clone)]
struct Symbol {
    file_path: String,
    line: usize,
}

#[derive(Clone)]
struct FunctionSymbol {
    symbol: Symbol,
    params: Vec<Option<String>>,
}

impl<'a> Resolver<'a> {
    fn new(
        project_dir: &'a Path,
        manifest: &HashMap<String, JsonValue>,
        ast: &'a AstProject,
    ) -> Self {
        let mut resolver = Self {
            project_dir,
            ast,
            dependency_packages: dependency_packages(manifest),
            top_levels: HashMap::new(),
            functions: HashMap::new(),
            types: BUILTIN_TYPES
                .iter()
                .map(|name| (*name).to_string())
                .collect(),
            variant_constructors: HashSet::new(),
            active_template_params: HashSet::new(),
            had_error: false,
        };
        resolver.collect_top_level_symbols(ast);
        resolver
    }

    fn collect_top_level_symbols(&mut self, ast: &AstProject) {
        for file in &ast.files {
            for item in &file.items {
                match item {
                    Item::Function(function) => {
                        self.insert_function(file, function);
                    }
                    Item::Type(type_decl) => {
                        if self.insert_top_level(file, &type_decl.name, type_decl.line) {
                            self.types.insert(type_decl.name.clone());
                            for variant in &type_decl.variants {
                                self.variant_constructors.insert(variant.name.clone());
                            }
                        }
                    }
                }
            }
        }
    }

    fn insert_function(&mut self, file: &AstFile, function: &crate::ast::Function) {
        if let Some(previous) = self.top_levels.get(&function.name).cloned() {
            self.report(
                "SYMBOL_DUPLICATE_TOP_LEVEL",
                &format!(
                    "Top-level symbol `{}` was already declared in {}:{}.",
                    function.name, previous.file_path, previous.line
                ),
                file,
                function.line,
            );
            return;
        }

        let params = function
            .params
            .iter()
            .map(|param| param.type_name.clone())
            .collect::<Vec<_>>();
        if let Some(previous) = self
            .functions
            .get(&function.name)
            .and_then(|functions| {
                functions
                    .iter()
                    .find(|candidate| candidate.params == params)
            })
            .cloned()
        {
            self.report(
                "SYMBOL_DUPLICATE_TOP_LEVEL",
                &format!(
                    "Top-level symbol `{}` was already declared in {}:{}.",
                    function.name, previous.symbol.file_path, previous.symbol.line
                ),
                file,
                function.line,
            );
            return;
        }

        self.functions
            .entry(function.name.clone())
            .or_default()
            .push(FunctionSymbol {
                symbol: Symbol {
                    file_path: file.path.clone(),
                    line: function.line,
                },
                params,
            });
    }

    fn insert_top_level(&mut self, file: &AstFile, name: &str, line: usize) -> bool {
        if let Some(previous) = self.top_levels.get(name).cloned() {
            self.report(
                "SYMBOL_DUPLICATE_TOP_LEVEL",
                &format!(
                    "Top-level symbol `{name}` was already declared in {}:{}.",
                    previous.file_path, previous.line
                ),
                file,
                line,
            );
            return false;
        }
        if let Some(previous) = self
            .functions
            .get(name)
            .and_then(|functions| functions.first())
            .cloned()
        {
            self.report(
                "SYMBOL_DUPLICATE_TOP_LEVEL",
                &format!(
                    "Top-level symbol `{name}` was already declared in {}:{}.",
                    previous.symbol.file_path, previous.symbol.line
                ),
                file,
                line,
            );
            return false;
        }

        self.top_levels.insert(
            name.to_string(),
            Symbol {
                file_path: file.path.clone(),
                line,
            },
        );
        true
    }

    fn resolve(&mut self) {
        for file in &self.ast.files {
            self.resolve_file(file);
        }
    }

    fn resolve_file(&mut self, file: &AstFile) {
        let mut imports = HashSet::new();

        for import in &file.imports {
            if !imports.insert(import.module.clone()) {
                self.report(
                    "SYMBOL_DUPLICATE_IMPORT",
                    &format!(
                        "Package `{}` is imported more than once in this file.",
                        import.module
                    ),
                    file,
                    import.line,
                );
            }

            let root_package = import
                .module
                .split('.')
                .next()
                .unwrap_or(import.module.as_str());

            self.resolve_imported_package(file, root_package, import.line);
        }

        for item in &file.items {
            match item {
                Item::Function(function) => self.resolve_function(file, function, &imports),
                Item::Type(type_decl) => self.resolve_type_decl(file, type_decl, &imports),
            }
        }
    }

    fn resolve_type_decl(
        &mut self,
        file: &AstFile,
        type_decl: &TypeDecl,
        imports: &HashSet<String>,
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
                                "Variant `{}` in UNION `{}` was already declared on line {}.",
                                variant.name, type_decl.name, previous
                            ),
                            file,
                            variant.line,
                        );
                    }

                    let mut fields = HashMap::new();
                    for field in &variant.fields {
                        self.resolve_member_field(file, field, imports);
                        if let Some(previous) = fields.insert(field.name.clone(), field.line) {
                            self.report(
                                "TYPE_DUPLICATE_FIELD",
                                &format!(
                                    "Field `{}` in variant `{}` was already declared on line {}.",
                                    field.name, variant.name, previous
                                ),
                                file,
                                field.line,
                            );
                        }
                    }
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
        imports: &HashSet<String>,
    ) {
        self.resolve_type_name(file, &field.type_name, field.line, imports);
    }

    fn resolve_function(
        &mut self,
        file: &AstFile,
        function: &crate::ast::Function,
        imports: &HashSet<String>,
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
        imports: &HashSet<String>,
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
        imports: &HashSet<String>,
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
            Statement::Fail { error, line } => {
                self.resolve_expression(file, error, *line, imports, locals);
            }
            Statement::Propagate { .. } => {}
            Statement::Recover { value, line } => {
                self.resolve_expression(file, value, *line, imports, locals);
            }
            Statement::Assign { name, value, line } => {
                self.resolve_identifier(file, name, *line, imports, locals);
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
                    if let MatchPattern::Expression(pattern) = &case.pattern {
                        self.resolve_match_pattern(file, pattern, case.line, imports, locals);
                    }
                    self.resolve_nested_block(file, &case.body, imports, locals);
                }
            }
            Statement::Using {
                name,
                value,
                body,
                line,
            } => {
                self.resolve_expression(file, value, *line, imports, locals);
                let mut nested = locals.clone();
                if nested
                    .insert(
                        name.clone(),
                        Symbol {
                            file_path: file.path.clone(),
                            line: *line,
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
        }
    }

    fn resolve_nested_block(
        &mut self,
        file: &AstFile,
        body: &[Statement],
        imports: &HashSet<String>,
        locals: &HashMap<String, Symbol>,
    ) {
        let mut nested = locals.clone();
        self.resolve_block(file, body, imports, &mut nested);
    }

    fn resolve_match_pattern(
        &mut self,
        file: &AstFile,
        pattern: &Expression,
        line: usize,
        imports: &HashSet<String>,
        locals: &HashMap<String, Symbol>,
    ) {
        match pattern {
            Expression::Identifier(name) if self.variant_constructors.contains(name) => {}
            _ => self.resolve_expression(file, pattern, line, imports, locals),
        }
    }

    fn resolve_expression(
        &mut self,
        file: &AstFile,
        expression: &Expression,
        line: usize,
        imports: &HashSet<String>,
        locals: &HashMap<String, Symbol>,
    ) {
        match expression {
            Expression::String(_) | Expression::Number(_) | Expression::Boolean(_) => {}
            Expression::Binary { left, right, .. } => {
                self.resolve_expression(file, left, line, imports, locals);
                self.resolve_expression(file, right, line, imports, locals);
            }
            Expression::Unary { operand, .. } => {
                self.resolve_expression(file, operand, line, imports, locals);
            }
            Expression::Call { callee, arguments } => {
                self.resolve_callable(file, callee, line, imports, locals);
                for argument in arguments {
                    self.resolve_expression(file, argument, line, imports, locals);
                }
            }
            Expression::Lambda { params, body } => {
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
                if !matches!(type_name.as_str(), "Error" | "Ok" | "Err")
                    && !self.variant_constructors.contains(type_name)
                {
                    self.resolve_type_name(file, type_name, line, imports);
                }
                for argument in arguments {
                    self.resolve_expression(file, argument, line, imports, locals);
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
        imports: &HashSet<String>,
        locals: &HashMap<String, Symbol>,
    ) {
        if callee.contains('.') {
            self.resolve_package_qualified_name(file, callee, line, imports);
        } else if builtins::general::is_general_call(callee) {
            return;
        } else if locals.contains_key(callee) {
            return;
        } else if !self.functions.contains_key(callee) {
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
        imports: &HashSet<String>,
        locals: &HashMap<String, Symbol>,
    ) {
        if name.contains('.') {
            self.resolve_package_qualified_name(file, name, line, imports);
        } else if !locals.contains_key(name)
            && !self.functions.contains_key(name)
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
        imports: &HashSet<String>,
    ) {
        if let Some(rest) = type_name.strip_prefix("ISOLATED FUNC(") {
            self.resolve_function_type_name(file, rest, line, imports);
            return;
        }
        if let Some(rest) = type_name.strip_prefix("FUNC(") {
            self.resolve_function_type_name(file, rest, line, imports);
            return;
        }
        if let Some(element) = type_name.strip_prefix("List OF ") {
            self.resolve_type_name(file, element, line, imports);
            return;
        }
        if let Some(success) = type_name.strip_prefix("Result OF ") {
            self.resolve_type_name(file, success, line, imports);
            return;
        }
        if let Some(rest) = type_name.strip_prefix("Thread OF ") {
            if let Some((message, output)) = rest.split_once(" TO ") {
                self.resolve_type_name(file, message, line, imports);
                self.resolve_type_name(file, output, line, imports);
                return;
            }
        }
        if let Some(rest) = type_name.strip_prefix("Map OF ") {
            if let Some((key, value)) = rest.split_once(" TO ") {
                self.resolve_type_name(file, key, line, imports);
                self.resolve_type_name(file, value, line, imports);
                return;
            }
        }

        if let Some((base, args)) = type_name.split_once(" OF ") {
            if self.types.contains(base) || self.active_template_params.contains(base) {
                for arg in args.split(", ") {
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
        imports: &HashSet<String>,
    ) {
        let Some((params, return_type)) = rest.split_once(") AS ") else {
            self.report(
                "SYMBOL_UNKNOWN_TYPE",
                &format!("Function type `FUNC({rest}` is malformed."),
                file,
                line,
            );
            return;
        };
        if !params.trim().is_empty() {
            for param in params.split(", ") {
                self.resolve_type_name(file, param, line, imports);
            }
        }
        self.resolve_type_name(file, return_type, line, imports);
    }

    fn resolve_package_qualified_name(
        &mut self,
        file: &AstFile,
        name: &str,
        line: usize,
        imports: &HashSet<String>,
    ) {
        let root = name.split('.').next().unwrap_or(name);
        if !imports.contains(root) {
            self.report(
                "SYMBOL_UNKNOWN_IMPORT",
                &format!("Package `{root}` is used but not imported in this file."),
                file,
                line,
            );
        } else if builtins::is_builtin_import(root) && !builtins::is_builtin_call(name) {
            self.report(
                "SYMBOL_UNKNOWN_IDENTIFIER",
                &format!("Built-in package `{root}` does not export `{name}`."),
                file,
                line,
            );
        }
    }

    fn report(&mut self, rule: &str, detail: &str, file: &AstFile, line: usize) {
        self.had_error = true;
        rules::show_diagnostic(rule, detail, &self.project_dir.join(&file.path), line, 1, 1);
    }

    fn resolve_imported_package(&mut self, file: &AstFile, name: &str, line: usize) {
        if is_builtin_import(name) {
            return;
        }

        let Some(dependency) = self.dependency_packages.get(name).cloned() else {
            self.report(
                "IMPORT_PACKAGE_NOT_DECLARED",
                &format!(
                    "Package `{name}` is not built in and is not declared in project.json packages."
                ),
                file,
                line,
            );
            return;
        };

        if let Some(source) = dependency.source.as_deref() {
            if source.starts_with("local://") {
                self.resolve_local_dependency(file, name, source, line);
                return;
            }
        }

        let package_file = self
            .project_dir
            .join("packages")
            .join(format!("{name}.mfp"));
        if package_file.is_file() {
            return;
        }

        let package_manifest = self
            .project_dir
            .join("packages")
            .join(name)
            .join("project.json");
        if package_manifest.is_file() {
            self.validate_source_package_manifest(file, name, &package_manifest, line);
            return;
        }

        self.report(
            "IMPORT_PACKAGE_NOT_INSTALLED",
            &format!(
                "Declared package `{name}` was not found at `{}` or `{}`.",
                package_file.display(),
                package_manifest.display()
            ),
            file,
            line,
        );
    }

    fn resolve_local_dependency(&mut self, file: &AstFile, name: &str, source: &str, line: usize) {
        let Some(path) = source.strip_prefix("local://") else {
            unreachable!("checked local scheme");
        };
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            self.report(
                "IMPORT_LOCAL_PATH_INVALID",
                &format!("Local package source for `{name}` must use `local:///absolute/path`."),
                file,
                line,
            );
            return;
        }

        let manifest_path = path.join("project.json");
        if !manifest_path.is_file() {
            self.report(
                "IMPORT_PACKAGE_NOT_INSTALLED",
                &format!(
                    "Local package `{name}` does not have a project.json at `{}`.",
                    manifest_path.display()
                ),
                file,
                line,
            );
            return;
        }

        self.validate_source_package_manifest(file, name, &manifest_path, line);
    }

    fn validate_source_package_manifest(
        &mut self,
        file: &AstFile,
        expected_name: &str,
        manifest_path: &Path,
        line: usize,
    ) {
        let Some(manifest) = read_manifest(manifest_path) else {
            self.report(
                "IMPORT_PACKAGE_MANIFEST_INVALID",
                &format!(
                    "Could not read package manifest `{}`.",
                    manifest_path.display()
                ),
                file,
                line,
            );
            return;
        };

        let actual_name = manifest.get("name").and_then(|value| value.get::<String>());
        if actual_name.map(String::as_str) != Some(expected_name) {
            self.report(
                "IMPORT_PACKAGE_NAME_MISMATCH",
                &format!(
                    "Imported package `{expected_name}` must have matching `name` in `{}`.",
                    manifest_path.display()
                ),
                file,
                line,
            );
            return;
        }

        let kind = manifest.get("kind").and_then(|value| value.get::<String>());
        if kind.map(String::as_str) != Some("package") {
            self.report(
                "IMPORT_PACKAGE_KIND_INVALID",
                &format!(
                    "Imported source package `{expected_name}` must declare `\"kind\": \"package\"` in `{}`.",
                    manifest_path.display()
                ),
                file,
                line,
            );
        }
    }
}

#[derive(Clone)]
struct DependencyPackage {
    source: Option<String>,
}

fn dependency_packages(
    manifest: &HashMap<String, JsonValue>,
) -> HashMap<String, DependencyPackage> {
    manifest
        .get("packages")
        .and_then(|value| value.get::<Vec<JsonValue>>())
        .into_iter()
        .flatten()
        .filter_map(|package| package.get::<HashMap<String, JsonValue>>())
        .filter_map(|package| {
            let name = package.get("name")?.get::<String>()?.clone();
            let source = package
                .get("source")
                .and_then(|value| value.get::<String>())
                .cloned();
            Some((name, DependencyPackage { source }))
        })
        .collect()
}

fn read_manifest(path: &Path) -> Option<HashMap<String, JsonValue>> {
    let contents = fs::read_to_string(path).ok()?;
    let json = contents.parse::<JsonValue>().ok()?;
    json.get::<HashMap<String, JsonValue>>().cloned()
}

fn is_builtin_import(name: &str) -> bool {
    builtins::is_builtin_import(name)
}
