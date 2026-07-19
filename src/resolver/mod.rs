use crate::ast::{
    AstFile, AstProject, ConstructorArg, DocBlock, DocHeaderKind, Expression, Function,
    FunctionKind, Item, MatchPattern, Statement, TopLevelBinding, TypeDecl, TypeDeclKind,
    TypeField, Visibility,
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
    "Money",
    "Nothing",
    "Result",
    "Scalar",
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
    builtins::audio::AUDIO_INPUT_TYPE,
    builtins::audio::AUDIO_OUTPUT_TYPE,
    builtins::audio::AUDIO_DEVICE_TYPE,
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
    let augmented = builtins::money::augmented_project(&augmented)?;
    // `vector` imports only the intrinsic `math` package, so it has no source
    // ordering dependency (plan-06-vector.md §5).
    let augmented = builtins::vector::augmented_project(&augmented)?;
    // `http` is injected before `net`: `http_package.mfb` imports `net`, so the
    // net source companion must be added only after http's source is present for
    // `net::uses_package` to see the dependency (plan-03-http.md Phase 4).
    let augmented = builtins::http::augmented_project(&augmented)?;
    let augmented = builtins::net::augmented_project(&augmented)?;
    let augmented = builtins::audio::augmented_project(&augmented)?;
    // `crypto` is injected before `encoding`: `crypto_package.mfb` imports
    // `encoding`, so the encoding source companion must be added only after
    // crypto's source is present for `encoding::uses_package` to see the
    // dependency (mirrors `http` before `net`; plan-04-crypto.md Part C).
    let augmented = builtins::crypto::augmented_project(&augmented)?;
    // `strings` before `encoding`: `strings_package.mfb` imports `encoding`.
    let augmented = builtins::strings::augmented_project(&augmented)?;
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

/// Whether `type_name` is a raw C ABI type (mirrors `syntaxcheck::is_c_abi_type`),
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
                    // TESTING blocks are lowered away (dropped or desugared into
                    // ordinary SUBs) before resolution runs (plan-18-A §3).
                    Item::Testing(_) => {}
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
        // Also reject a collision against an already-inserted function of the
        // same name and parameter types — mirroring `insert_function`. A re-export
        // alias never participates in return-type overloading (its `return_type`
        // is always `None`), so equal params alone is a duplicate, not a legal
        // overload set.
        if let Some(previous) = self
            .functions
            .get(name)
            .and_then(|functions| {
                functions
                    .iter()
                    .find(|candidate| candidate.params == params && candidate.return_type.is_none())
            })
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
                functions.iter().find(|candidate| {
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
            Visibility::Export | Visibility::Public => true,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{CallArg, ConstructorArg, Expression, FunctionKind, Param, Visibility};
    use crate::manifest::validate_project_manifest;

    fn quiet<T>(f: impl FnOnce() -> T) -> T {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let out = f();
        std::panic::set_hook(prev);
        out
    }

    fn resolve_fixture(name: &str) -> Result<(), ()> {
        let dir = crate::testutil::fixture_dir(name);
        let manifest = validate_project_manifest(&dir.join("project.json"))
            .expect("fixture manifest is valid");
        let pname = manifest
            .get("name")
            .and_then(|v| v.get::<String>())
            .cloned()
            .expect("fixture manifest has a name");
        let ast = crate::ast::parse_project(&pname, &dir, &manifest).expect("fixture parses");
        quiet(|| resolve_project(&dir, &manifest, &ast))
    }

    fn empty_expr() -> Expression {
        Expression::Number("1".into())
    }

    fn param(name: &str, type_name: Option<&str>) -> Param {
        Param {
            name: name.into(),
            type_name: type_name.map(str::to_string),
            resource: false,
            state_type: None,
            default: None,
            line: 1,
        }
    }

    fn func(name: &str, params: Vec<Param>) -> Function {
        Function {
            kind: FunctionKind::Func,
            visibility: Visibility::Export,
            isolated: false,
            name: name.into(),
            template_params: Vec::new(),
            params,
            return_type: None,
            return_resource: false,
            return_state_type: None,
            body: Vec::new(),
            trap: None,
            line: 1,
        }
    }

    fn ast_file(path: &str) -> AstFile {
        AstFile {
            path: path.into(),
            imports: Vec::new(),
            items: Vec::new(),
            internal: false,
        }
    }

    #[test]
    fn constructor_arg_value_positional_and_named() {
        let pos = ConstructorArg::Positional(empty_expr());
        let named = ConstructorArg::Named {
            name: "x".into(),
            value: empty_expr(),
            line: 1,
        };
        assert!(matches!(constructor_arg_value(&pos), Expression::Number(_)));
        assert!(matches!(
            constructor_arg_value(&named),
            Expression::Number(_)
        ));
    }

    #[test]
    fn call_arg_value_positional_and_named() {
        let pos = CallArg::Positional(empty_expr());
        let named = CallArg::Named {
            name: "x".into(),
            value: empty_expr(),
            line: 1,
        };
        assert!(matches!(call_arg_value(&pos), Expression::Number(_)));
        assert!(matches!(call_arg_value(&named), Expression::Number(_)));
    }

    #[test]
    fn overload_types_match_variants() {
        let f = func(
            "g",
            vec![param("a", Some("Integer")), param("b", Some("String"))],
        );
        assert!(overload_types_match(
            &f,
            &["Integer".to_string(), "String".to_string()]
        ));
        assert!(!overload_types_match(&f, &["Integer".to_string()]));
        assert!(!overload_types_match(
            &f,
            &["Integer".to_string(), "Float".to_string()]
        ));
    }

    #[test]
    fn overload_types_match_none_type_name() {
        let f = func("g", vec![param("a", None)]);
        assert!(overload_types_match(&f, &[String::new()]));
        assert!(!overload_types_match(&f, &["Integer".to_string()]));
    }

    #[test]
    fn is_c_abi_type_recognizes_and_rejects() {
        for t in [
            "CPtr", "CString", "CInt8", "CInt16", "CInt32", "CInt64", "CUInt8", "CUInt16",
            "CUInt32", "CUInt64", "CFloat", "CDouble",
        ] {
            assert!(is_c_abi_type(t), "{t} should be a C ABI type");
        }
        assert!(!is_c_abi_type("Integer"));
        assert!(!is_c_abi_type("CPtrX"));
        assert!(!is_c_abi_type(""));
    }

    #[test]
    fn resource_base_type_strips_state_suffix() {
        assert_eq!(resource_base_type("Handle STATE Open"), "Handle");
        assert_eq!(resource_base_type("Handle"), "Handle");
        assert_eq!(resource_base_type(""), "");
    }

    #[test]
    fn visible_from_rules() {
        let empty = AstProject {
            name: "p".into(),
            files: Vec::new(),
        };
        let dir = std::path::Path::new(".");
        let resolver = Resolver::new(dir, &HashMap::new(), &empty);
        let here = ast_file("a.mfb");
        assert!(resolver.visible_from(&here, Visibility::Export, "other.mfb"));
        assert!(resolver.visible_from(&here, Visibility::Public, "other.mfb"));
        assert!(resolver.visible_from(&here, Visibility::Private, "a.mfb"));
        assert!(!resolver.visible_from(&here, Visibility::Private, "other.mfb"));
    }

    #[test]
    fn top_level_and_function_visibility_lookups() {
        let file = AstFile {
            path: "a.mfb".into(),
            imports: Vec::new(),
            items: vec![
                Item::Binding(crate::ast::TopLevelBinding {
                    mutable: false,
                    resource: false,
                    state_type: None,
                    name: "GLOBAL".into(),
                    type_name: None,
                    value: None,
                    visibility: Visibility::Export,
                    line: 1,
                }),
                Item::Function(func("helper", vec![])),
            ],
            internal: false,
        };
        let ast = AstProject {
            name: "p".into(),
            files: vec![file.clone()],
        };
        let dir = std::path::Path::new(".");
        let resolver = Resolver::new(dir, &HashMap::new(), &ast);
        assert!(resolver.top_level_visible_in_file(&file, "GLOBAL"));
        assert!(!resolver.top_level_visible_in_file(&file, "MISSING"));
        assert!(resolver.function_visible_in_file(&file, "helper"));
        assert!(!resolver.function_visible_in_file(&file, "missingfn"));
    }

    #[test]
    fn link_target_signature_lookup() {
        let dir = std::path::Path::new(".");
        let ast = AstProject {
            name: "p".into(),
            files: vec![AstFile {
                path: "lib.mfb".into(),
                imports: Vec::new(),
                items: vec![Item::Link(crate::ast::LinkBlock {
                    library: "lib".into(),
                    alias: "db".into(),
                    cstructs: Vec::new(),
                    functions: vec![crate::ast::LinkFunction {
                        name: "open".into(),
                        params: vec![param("path", Some("CString"))],
                        return_type: Some("CPtr".into()),
                        return_resource: false,
                        return_state_type: None,
                        symbol: "open".into(),
                        abi: crate::ast::AbiSpec {
                            slots: Vec::new(),
                            return_name: "ret".into(),
                            return_ctype: "CPtr".into(),
                            line: 3,
                        },
                        consts: Vec::new(),
                        bind_in: Vec::new(),
                        bind_state: None,
                        bind_state_resource: None,
                        success_on: None,
                        result: None,
                        free: None,
                        line: 3,
                    }],
                    line: 1,
                })],
                internal: false,
            }],
        };
        let resolver = Resolver::new(dir, &HashMap::new(), &ast);
        assert!(resolver.link_target_signature("db.open").is_some());
        assert!(resolver.link_target_signature("db.missing").is_none());
        assert!(resolver.link_target_signature("other.open").is_none());
        assert!(resolver.link_target_signature("dbopen").is_none());
    }

    #[test]
    fn resolve_valid_fixtures_succeed() {
        for name in [
            "parser-hello-world",
            "control-flow-match",
            "control-flow-match-when",
            "control-flow-if",
            "overload-func-valid",
            "overload-sub-valid",
            "doc-block-valid",
            "native-resource-link-valid",
            "math_package_valid",
        ] {
            assert!(
                resolve_fixture(name).is_ok(),
                "fixture `{name}` should resolve"
            );
        }
    }

    #[test]
    fn validate_project_docs_true_for_valid_docs() {
        let dir = crate::testutil::fixture_dir("doc-block-valid");
        let manifest = validate_project_manifest(&dir.join("project.json")).unwrap();
        let pname = manifest
            .get("name")
            .and_then(|v| v.get::<String>())
            .cloned()
            .unwrap();
        let ast = crate::ast::parse_project(&pname, &dir, &manifest).unwrap();
        assert!(validate_project_docs(&dir, &ast));
    }

    #[test]
    fn validate_project_docs_false_for_invalid_docs() {
        let dir = crate::testutil::fixture_dir("doc-block-invalid");
        let manifest = validate_project_manifest(&dir.join("project.json")).unwrap();
        let pname = manifest
            .get("name")
            .and_then(|v| v.get::<String>())
            .cloned()
            .unwrap();
        let ast = crate::ast::parse_project(&pname, &dir, &manifest).unwrap();
        assert!(!quiet(|| validate_project_docs(&dir, &ast)));
    }

    #[test]
    fn resolve_invalid_fixtures_fail() {
        for name in [
            "collections-cutover-invalid",
            "doc-block-invalid",
            "native-link-duplicate-resource-invalid",
            "native-resource-close-not-native-invalid",
            "native-resource-close-signature-invalid",
            "result-not-user-visible-invalid",
        ] {
            assert!(
                resolve_fixture(name).is_err(),
                "fixture `{name}` should fail to resolve"
            );
        }
    }

    #[test]
    fn resolve_project_with_no_doc_validation() {
        let dir = crate::testutil::fixture_dir("doc-block-valid");
        let manifest = validate_project_manifest(&dir.join("project.json")).unwrap();
        let pname = manifest
            .get("name")
            .and_then(|v| v.get::<String>())
            .cloned()
            .unwrap();
        let ast = crate::ast::parse_project(&pname, &dir, &manifest).unwrap();
        assert!(quiet(|| resolve_project_with(&dir, &manifest, &ast, false)).is_ok());
    }

    #[test]
    fn duplicate_top_level_function_reports() {
        let f1 = func("dup", vec![param("a", Some("Integer"))]);
        let f2 = func("dup", vec![param("a", Some("Integer"))]);
        let mut f3 = func("dup2", vec![param("a", Some("Integer"))]);
        f3.return_type = Some("Integer".into());
        let mut f4 = func("dup2", vec![param("a", Some("Integer"))]);
        f4.return_type = Some("String".into());
        let ast = AstProject {
            name: "p".into(),
            files: vec![AstFile {
                path: "a.mfb".into(),
                imports: Vec::new(),
                items: vec![
                    Item::Function(f1),
                    Item::Function(f2),
                    Item::Function(f3),
                    Item::Function(f4),
                ],
                internal: false,
            }],
        };
        let dir = std::path::Path::new(".");
        let resolver = quiet(|| Resolver::new(dir, &HashMap::new(), &ast));
        assert!(resolver.had_error);
    }

    #[test]
    fn reserved_builtin_name_rejected() {
        let ast = AstProject {
            name: "p".into(),
            files: vec![AstFile {
                path: "a.mfb".into(),
                imports: Vec::new(),
                items: vec![Item::Function(func("error", vec![]))],
                internal: false,
            }],
        };
        let dir = std::path::Path::new(".");
        let resolver = quiet(|| Resolver::new(dir, &HashMap::new(), &ast));
        assert!(resolver.had_error);
    }

    #[test]
    fn type_and_resource_names_registered() {
        let ast = AstProject {
            name: "p".into(),
            files: vec![AstFile {
                path: "a.mfb".into(),
                imports: Vec::new(),
                items: vec![
                    Item::Type(crate::ast::TypeDecl {
                        kind: crate::ast::TypeDeclKind::Type,
                        visibility: Visibility::Export,
                        name: "Widget".into(),
                        template_params: Vec::new(),
                        fields: Vec::new(),
                        includes: Vec::new(),
                        variants: Vec::new(),
                        members: Vec::new(),
                        line: 1,
                    }),
                    Item::Binding(crate::ast::TopLevelBinding {
                        mutable: false,
                        resource: false,
                        state_type: None,
                        name: "Widget".into(),
                        type_name: None,
                        value: None,
                        visibility: Visibility::Export,
                        line: 2,
                    }),
                ],
                internal: false,
            }],
        };
        let dir = std::path::Path::new(".");
        let resolver = quiet(|| Resolver::new(dir, &HashMap::new(), &ast));
        assert!(resolver.types.contains("Widget"));
        assert!(resolver.had_error, "duplicate top-level should report");
    }

    fn binding(name: &str) -> Item {
        Item::Binding(crate::ast::TopLevelBinding {
            mutable: false,
            resource: false,
            state_type: None,
            name: name.into(),
            type_name: None,
            value: None,
            visibility: Visibility::Export,
            line: 1,
        })
    }

    #[test]
    fn function_name_collides_with_prior_binding_reports() {
        // A FUNC declared after a top-level binding of the same name collides.
        let ast = AstProject {
            name: "p".into(),
            files: vec![AstFile {
                path: "a.mfb".into(),
                imports: Vec::new(),
                items: vec![binding("dup"), Item::Function(func("dup", vec![]))],
                internal: false,
            }],
        };
        let dir = std::path::Path::new(".");
        let resolver = quiet(|| Resolver::new(dir, &HashMap::new(), &ast));
        assert!(resolver.had_error);
    }

    #[test]
    fn binding_name_collides_with_prior_function_reports() {
        // A binding declared after a FUNC of the same name collides (the
        // `insert_top_level` function-table branch).
        let ast = AstProject {
            name: "p".into(),
            files: vec![AstFile {
                path: "a.mfb".into(),
                imports: Vec::new(),
                items: vec![Item::Function(func("dup", vec![])), binding("dup")],
                internal: false,
            }],
        };
        let dir = std::path::Path::new(".");
        let resolver = quiet(|| Resolver::new(dir, &HashMap::new(), &ast));
        assert!(resolver.had_error);
    }

    #[test]
    fn alias_function_name_collides_with_prior_binding_reports() {
        // A FUNC re-export alias whose name matches a prior top-level binding
        // hits the `insert_alias_function` duplicate branch. The alias also needs
        // a LINK namespace so its target resolves.
        let link = Item::Link(crate::ast::LinkBlock {
            library: "lib".into(),
            alias: "db".into(),
            cstructs: Vec::new(),
            functions: vec![crate::ast::LinkFunction {
                name: "close".into(),
                params: vec![param("h", Some("CPtr"))],
                return_type: Some("Nothing".into()),
                return_resource: false,
                return_state_type: None,
                symbol: "close".into(),
                abi: crate::ast::AbiSpec {
                    slots: Vec::new(),
                    return_name: "ret".into(),
                    return_ctype: "CInt32".into(),
                    line: 2,
                },
                consts: Vec::new(),
                bind_in: Vec::new(),
                bind_state: None,
                bind_state_resource: None,
                success_on: None,
                result: None,
                free: None,
                line: 2,
            }],
            line: 1,
        });
        let alias = Item::FuncAlias(crate::ast::FuncAlias {
            visibility: Visibility::Export,
            name: "dup".into(),
            target: "db.close".into(),
            line: 5,
        });
        let ast = AstProject {
            name: "p".into(),
            files: vec![AstFile {
                path: "a.mfb".into(),
                imports: Vec::new(),
                items: vec![link, binding("dup"), alias],
                internal: false,
            }],
        };
        let dir = std::path::Path::new(".");
        let resolver = quiet(|| Resolver::new(dir, &HashMap::new(), &ast));
        assert!(resolver.had_error);
    }

    #[test]
    fn alias_function_registers_when_unique() {
        // A unique alias registers as a callable carrying the target's params.
        let link = Item::Link(crate::ast::LinkBlock {
            library: "lib".into(),
            alias: "db".into(),
            cstructs: Vec::new(),
            functions: vec![crate::ast::LinkFunction {
                name: "close".into(),
                params: vec![param("h", Some("CPtr"))],
                return_type: Some("Nothing".into()),
                return_resource: false,
                return_state_type: None,
                symbol: "close".into(),
                abi: crate::ast::AbiSpec {
                    slots: Vec::new(),
                    return_name: "ret".into(),
                    return_ctype: "CInt32".into(),
                    line: 2,
                },
                consts: Vec::new(),
                bind_in: Vec::new(),
                bind_state: None,
                bind_state_resource: None,
                success_on: None,
                result: None,
                free: None,
                line: 2,
            }],
            line: 1,
        });
        let alias = Item::FuncAlias(crate::ast::FuncAlias {
            visibility: Visibility::Export,
            name: "closeDb".into(),
            target: "db.close".into(),
            line: 5,
        });
        let ast = AstProject {
            name: "p".into(),
            files: vec![AstFile {
                path: "a.mfb".into(),
                imports: Vec::new(),
                items: vec![link, alias],
                internal: false,
            }],
        };
        let dir = std::path::Path::new(".");
        let file = ast.files[0].clone();
        let resolver = quiet(|| Resolver::new(dir, &HashMap::new(), &ast));
        assert!(!resolver.had_error);
        assert!(resolver.function_visible_in_file(&file, "closeDb"));
    }
}
