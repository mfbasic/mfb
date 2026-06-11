use crate::ast::{AstFile, AstProject, Expression, Item, Statement};
use crate::rules;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use tinyjson::JsonValue;

const BUILTIN_IMPORTS: &[&str] = &["io"];
const BUILTIN_TYPES: &[&str] = &[
    "Boolean", "Byte", "Fixed", "Float", "Integer", "Nothing", "Result", "String",
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
    functions: HashMap<String, Symbol>,
    types: HashSet<String>,
    had_error: bool,
}

#[derive(Clone)]
struct Symbol {
    file_path: String,
    line: usize,
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
                        if self.insert_top_level(file, &function.name, function.line) {
                            self.functions.insert(
                                function.name.clone(),
                                Symbol {
                                    file_path: file.path.clone(),
                                    line: function.line,
                                },
                            );
                        }
                    }
                    Item::Type(type_decl) => {
                        if self.insert_top_level(file, &type_decl.name, type_decl.line) {
                            self.types.insert(type_decl.name.clone());
                        }
                    }
                }
            }
        }
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
            if let Item::Function(function) = item {
                self.resolve_function(file, function, &imports);
            }
        }
    }

    fn resolve_function(
        &mut self,
        file: &AstFile,
        function: &crate::ast::Function,
        imports: &HashSet<String>,
    ) {
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

        for statement in &function.body {
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
                        self.resolve_expression(file, value, *line, imports, &locals);
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
                            &format!(
                                "Local binding `{name}` is already declared in this function."
                            ),
                            file,
                            *line,
                        );
                    }
                }
                Statement::Return { value, line } => {
                    if let Some(value) = value {
                        self.resolve_expression(file, value, *line, imports, &locals);
                    }
                }
                Statement::Expression { expression, line } => {
                    self.resolve_expression(file, expression, *line, imports, &locals);
                }
            }
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
            Expression::Call { callee, arguments } => {
                self.resolve_callable(file, callee, line, imports, locals);
                for argument in arguments {
                    self.resolve_expression(file, argument, line, imports, locals);
                }
            }
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
        } else if locals.contains_key(callee) {
            self.report(
                "SYMBOL_NOT_CALLABLE",
                &format!("Local binding or parameter `{callee}` is not callable."),
                file,
                line,
            );
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
        } else if self.functions.contains_key(name) {
            self.report(
                "SYMBOL_NOT_VALUE",
                &format!("Function `{name}` must be called with arguments before it can be used as a value."),
                file,
                line,
            );
        } else if !locals.contains_key(name) && !self.functions.contains_key(name) {
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
        if let Some(element) = type_name.strip_prefix("List OF ") {
            self.resolve_type_name(file, element, line, imports);
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
            .join(format!("{name}.mfl"));
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
        if kind.map(String::as_str) != Some("library") {
            self.report(
                "IMPORT_PACKAGE_KIND_INVALID",
                &format!(
                    "Imported source package `{expected_name}` must declare `\"kind\": \"library\"` in `{}`.",
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
    BUILTIN_IMPORTS.contains(&name)
}
