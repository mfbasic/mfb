use crate::ast::{
    AstFile, AstProject, ConstructorArg, DocBlock, DocHeaderKind, Expression, Function,
    FunctionKind, Item, MatchPattern, Statement, TopLevelBinding, TypeDecl, TypeDeclKind, TypeField,
    Visibility,
};
use crate::binary_repr;
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
    "ErrorLoc",
    "Fixed",
    "Float",
    "Integer",
    "Json",
    "Nothing",
    "Result",
    "String",
    builtins::fs::FILE_TYPE,
    builtins::term::TERM_COLOR_TYPE,
    builtins::term::TERM_SIZE_TYPE,
    builtins::net::SOCKET_TYPE,
    builtins::net::LISTENER_TYPE,
    builtins::net::ADDRESS_TYPE,
    builtins::net::UDP_SOCKET_TYPE,
    builtins::net::DATAGRAM_TYPE,
    builtins::net::DATAGRAM_TEXT_TYPE,
    builtins::tls::TLS_SOCKET_TYPE,
    builtins::tls::TLS_LISTENER_TYPE,
];

pub fn resolve_project(
    project_dir: &Path,
    manifest: &HashMap<String, JsonValue>,
    ast: &AstProject,
) -> Result<(), ()> {
    resolve_project_with(project_dir, manifest, ast, true)
}

/// Validate only the `DOC` blocks of an already-parsed project, without running
/// full name resolution. Used by `mfb doc` on a single source file, where the
/// surrounding project context (and lockfile) is unavailable. Returns `true`
/// when every block is valid.
pub fn validate_project_docs(project_dir: &Path, ast: &AstProject) -> bool {
    let mut resolver = Resolver::new(project_dir, &HashMap::new(), ast);
    resolver.resolve_doc_blocks();
    !resolver.had_error
}

/// Resolve the project. `validate_docs` enables `DOC` block validation; it must
/// be set only for the pre-monomorphization pass, since monomorphization renames
/// overloaded and generic declarations and would make their doc headers appear
/// unresolved on a second pass.
pub fn resolve_project_with(
    project_dir: &Path,
    manifest: &HashMap<String, JsonValue>,
    ast: &AstProject,
    validate_docs: bool,
) -> Result<(), ()> {
    let augmented = builtins::json::augmented_project(ast)?;
    let augmented = builtins::csv::augmented_project(&augmented)?;
    let augmented = builtins::regex::augmented_project(&augmented)?;
    let augmented = builtins::datetime::augmented_project(&augmented)?;
    // `vector` imports only the intrinsic `math` package, so it has no source
    // ordering dependency (plan-06-vector.md §5).
    let augmented = builtins::vector::augmented_project(&augmented)?;
    // `http` is injected before `net`: `http_package.mfb` imports `net`, so the
    // net source companion must be added only after http's source is present for
    // `net::uses_package` to see the dependency (plan-03-http.md Phase 4).
    let augmented = builtins::http::augmented_project(&augmented)?;
    let augmented = builtins::net::augmented_project(&augmented)?;
    let augmented = builtins::encoding::augmented_project(&augmented)?;
    let mut resolver = Resolver::new(project_dir, manifest, &augmented);
    resolver.resolve();
    if validate_docs {
        resolver.resolve_doc_blocks();
    }
    if resolver.had_error {
        Err(())
    } else {
        Ok(())
    }
}

fn constructor_arg_value(argument: &ConstructorArg) -> &Expression {
    match argument {
        ConstructorArg::Positional(value) => value,
        ConstructorArg::Named { value, .. } => value,
    }
}

/// Whether a function overload's parameter types match the type list a `DOC`
/// header named (whitespace-normalized, in order).
fn overload_types_match(function: &Function, wanted: &[String]) -> bool {
    if function.params.len() != wanted.len() {
        return false;
    }
    function.params.iter().zip(wanted).all(|(param, want)| {
        crate::ast::normalize_ws(param.type_name.as_deref().unwrap_or(""))
            == crate::ast::normalize_ws(want)
    })
}

fn call_arg_value(argument: &crate::ast::CallArg) -> &Expression {
    match argument {
        crate::ast::CallArg::Positional(value) => value,
        crate::ast::CallArg::Named { value, .. } => value,
    }
}

/// Whether `type_name` is a raw C ABI type (mirrors `typecheck::is_c_abi_type`),
/// which may appear only inside ABI slots (plan-link-update.md §5/§11).
fn is_c_abi_type(type_name: &str) -> bool {
    matches!(
        type_name,
        "CPtr"
            | "CString"
            | "CInt8"
            | "CInt16"
            | "CInt32"
            | "CInt64"
            | "CUInt8"
            | "CUInt16"
            | "CUInt32"
            | "CUInt64"
            | "CFloat"
            | "CDouble"
    )
}

/// The bare resource type name with any `STATE T` suffix removed, mirroring
/// `builtins::resource::base_resource_name`.
fn resource_base_type(type_name: &str) -> &str {
    match type_name.split_once(" STATE ") {
        Some((base, _)) => base,
        None => type_name,
    }
}

struct Resolver<'a> {
    project_dir: &'a Path,
    ast: &'a AstProject,
    dependency_packages: HashMap<String, DependencyPackage>,
    top_levels: HashMap<String, Symbol>,
    functions: HashMap<String, Vec<FunctionSymbol>>,
    types: HashSet<String>,
    /// LINK alias namespaces: alias (e.g. `sqliteLink`) → its native functions
    /// keyed by name. Members are resolved as `alias::func` qualified names
    /// (plan-link-update.md §5b).
    link_functions: HashMap<String, HashMap<String, LinkFnSig>>,
    active_template_params: HashSet<String>,
    had_error: bool,
}

#[derive(Clone)]
struct LinkFnSig {
    params: Vec<Option<String>>,
    param_resource: Vec<bool>,
    // Consumed by later native-resource phases (producer typing); recorded now.
    #[allow(dead_code)]
    return_type: Option<String>,
    #[allow(dead_code)]
    return_resource: bool,
    line: usize,
}

#[derive(Clone)]
struct Symbol {
    file_path: String,
    line: usize,
    visibility: Visibility,
}

#[derive(Clone)]
struct FunctionSymbol {
    symbol: Symbol,
    params: Vec<Option<String>>,
    /// Declared return type (`None` for a `SUB`). Part of the duplicate-detection
    /// key so two declarations sharing a name and parameter types but differing in
    /// return type form a legal return-type overload set (plan-01-overload.md §F.1).
    return_type: Option<String>,
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
            link_functions: HashMap::new(),
            active_template_params: HashSet::new(),
            had_error: false,
        };
        resolver.collect_top_level_symbols(ast);
        resolver
    }

    fn collect_top_level_symbols(&mut self, ast: &AstProject) {
        // First pass: register LINK namespaces so resource declarations and
        // re-export aliases (which reference `alias::func`) can be resolved.
        for file in &ast.files {
            for item in &file.items {
                if let Item::Link(link) = item {
                    let entry = self.link_functions.entry(link.alias.clone()).or_default();
                    for function in &link.functions {
                        entry.insert(
                            function.name.clone(),
                            LinkFnSig {
                                params: function
                                    .params
                                    .iter()
                                    .map(|param| param.type_name.clone())
                                    .collect(),
                                param_resource: function
                                    .params
                                    .iter()
                                    .map(|param| param.resource)
                                    .collect(),
                                return_type: function.return_type.clone(),
                                return_resource: function.return_resource,
                                line: function.line,
                            },
                        );
                    }
                }
            }
        }

        for file in &ast.files {
            for item in &file.items {
                match item {
                    Item::Binding(binding) => {
                        self.insert_top_level(
                            file,
                            &binding.name,
                            binding.line,
                            binding.visibility,
                        );
                    }
                    Item::Function(function) => {
                        self.insert_function(file, function);
                    }
                    Item::Type(type_decl) => {
                        if self.insert_top_level(
                            file,
                            &type_decl.name,
                            type_decl.line,
                            type_decl.visibility,
                        ) {
                            self.types.insert(type_decl.name.clone());
                        }
                    }
                    // A native resource declaration introduces an opaque type at
                    // package scope (plan-link-update.md §5/§5a).
                    Item::Resource(resource) => {
                        if self.insert_top_level(
                            file,
                            &resource.name,
                            resource.line,
                            resource.visibility,
                        ) {
                            self.types.insert(resource.name.clone());
                        }
                    }
                    // A re-export alias publishes a LINK function under a package
                    // name; register it as a callable carrying the target's
                    // parameter types (plan-link-update.md §5a).
                    Item::FuncAlias(alias) => {
                        let params = self
                            .link_target_signature(&alias.target)
                            .map(|sig| sig.params.clone())
                            .unwrap_or_default();
                        self.insert_alias_function(
                            file,
                            &alias.name,
                            alias.line,
                            alias.visibility,
                            params,
                        );
                    }
                    Item::Link(_) => {}
                    // DOC blocks declare no symbols; they are resolved separately
                    // after symbol collection (see `resolve_doc_blocks`).
                    Item::Doc(_) => {}
                }
            }
        }
    }

    /// Look up a LINK function signature from a dotted `alias.func` target.
    fn link_target_signature(&self, target: &str) -> Option<&LinkFnSig> {
        let (alias, func) = target.split_once('.')?;
        self.link_functions.get(alias)?.get(func)
    }

    fn insert_alias_function(
        &mut self,
        file: &AstFile,
        name: &str,
        line: usize,
        visibility: Visibility,
        params: Vec<Option<String>>,
    ) {
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
            return;
        }
        self.functions
            .entry(name.to_string())
            .or_default()
            .push(FunctionSymbol {
                symbol: Symbol {
                    file_path: file.path.clone(),
                    line,
                    visibility,
                },
                params,
                // A re-export alias never participates in return-type overloading.
                return_type: None,
            });
    }

    fn insert_function(&mut self, file: &AstFile, function: &crate::ast::Function) {
        // A reserved general built-in (`error`) is a language primitive and may not
        // be redeclared as a user `FUNC`/`SUB` (plan-01-overload.md §A.5). Every
        // other overridable built-in name (`toString`, `len`, …) is accepted.
        if builtins::general::reserved_builtin_name(&function.name) {
            self.report(
                "SYMBOL_RESERVED_BUILTIN_NAME",
                &format!(
                    "`{}` is a reserved built-in and cannot be redeclared.",
                    function.name
                ),
                file,
                function.line,
            );
            return;
        }

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
        // The duplicate-detection key is (name, parameter types, return type): two
        // declarations collide only when all three match. Sharing name + parameter
        // types but differing in return type is a legal return-type overload set
        // (plan-01-overload.md §F.1).
        let return_type = function.return_type.clone();
        if let Some(previous) = self
            .functions
            .get(&function.name)
            .and_then(|functions| {
                functions
                    .iter()
                    .find(|candidate| {
                        candidate.params == params && candidate.return_type == return_type
                    })
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
                    visibility: function.visibility,
                },
                params,
                return_type,
            });
    }

    fn insert_top_level(
        &mut self,
        file: &AstFile,
        name: &str,
        line: usize,
        visibility: Visibility,
    ) -> bool {
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
                visibility,
            },
        );
        true
    }

    fn top_level_visible_in_file(&self, file: &AstFile, name: &str) -> bool {
        self.top_levels
            .get(name)
            .is_some_and(|symbol| self.visible_from(file, symbol.visibility, &symbol.file_path))
    }

    fn function_visible_in_file(&self, file: &AstFile, name: &str) -> bool {
        self.functions.get(name).is_some_and(|functions| {
            functions.iter().any(|function| {
                self.visible_from(file, function.symbol.visibility, &function.symbol.file_path)
            })
        })
    }

    fn visible_from(&self, file: &AstFile, visibility: Visibility, owner_file_path: &str) -> bool {
        match visibility {
            Visibility::Export | Visibility::Package => true,
            Visibility::Private => file.path == owner_file_path,
        }
    }

    fn report(&mut self, rule: &str, detail: &str, file: &AstFile, line: usize) {
        self.had_error = true;
        rules::show_diagnostic(rule, detail, &self.project_dir.join(&file.path), line, 1, 1);
    }
}

mod packages;
mod resolution;

use packages::{dependency_packages, qualify_package_name, DependencyPackage};
