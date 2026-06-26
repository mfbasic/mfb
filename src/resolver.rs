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
    // `http` is injected before `net`: `http_package.mfb` imports `net`, so the
    // net source companion must be added only after http's source is present for
    // `net::uses_package` to see the dependency (plan-03-http.md Phase 4).
    let augmented = builtins::http::augmented_project(&augmented)?;
    let augmented = builtins::net::augmented_project(&augmented)?;
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
            });
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
                    visibility: function.visibility,
                },
                params,
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

    fn resolve(&mut self) {
        for file in &self.ast.files {
            self.resolve_file(file);
        }
    }

    /// Validate every `DOC` block in the package (plan-09-doc.md §2/§4): resolve
    /// each header to a declaration of the right kind, then check the body's
    /// `ARG`/`PROP`/`RET`/`ERROR`/`EXAMPLE` lines, attributes, and `DEPRECATED`
    /// against that declaration.
    fn resolve_doc_blocks(&mut self) {
        let ast = self.ast;

        // Index user declarations by name. Functions and subs share a namespace,
        // and a name may carry several overloads.
        let mut funcs: HashMap<&str, Vec<&Function>> = HashMap::new();
        let mut types: HashMap<&str, &TypeDecl> = HashMap::new();
        for file in &ast.files {
            for item in &file.items {
                match item {
                    Item::Function(function) => {
                        funcs.entry(function.name.as_str()).or_default().push(function);
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
                            &format!("`{}` is not a {} in this package.", doc.header_name, kind.keyword()),
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
                Some(types) => format!("`{} {}({})`", doc.header_kind.keyword(), doc.header_name, types.join(", ")),
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
                    &format!("`{}` is not a {noun} of the documented declaration.", entry.name),
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
                    // typecheck as NATIVE_CPTR_ESCAPE; don't double-report it here
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
            Expression::String(_) | Expression::Number(_) | Expression::Boolean(_) => {}
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
            if let Some((key, value)) = rest.split_once(" TO ") {
                let value = value.strip_prefix("RES ").unwrap_or(value);
                self.resolve_type_name(file, key, line, imports);
                self.resolve_type_name(file, value, line, imports);
                return;
            }
        }
        if let Some(rest) = type_name.strip_prefix("MapEntry OF ") {
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
        imports: &HashMap<String, String>,
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
            self.install_package_type_names(file, name, &package_file, line);
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

    fn install_package_type_names(
        &mut self,
        file: &AstFile,
        name: &str,
        package_file: &Path,
        line: usize,
    ) {
        let exports = match binary_repr::read_package_type_exports(package_file) {
            Ok(exports) => exports,
            Err(err) => {
                self.report(
                    "IMPORT_PACKAGE_INVALID",
                    &format!("Package `{name}` type exports could not be read: {err}"),
                    file,
                    line,
                );
                return;
            }
        };
        for export in exports {
            self.types.insert(export.name);
            for variant in export.variants {
                self.types.insert(variant.name);
            }
        }
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

fn qualify_package_name(name: &str, binding: &str, package: &str) -> String {
    if binding == package {
        return name.to_string();
    }
    format!("{package}.{}", &name[binding.len() + 1..])
}

fn is_builtin_import(name: &str) -> bool {
    builtins::is_builtin_import(name)
}
