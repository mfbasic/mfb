use crate::ast::{
    AstProject, DocBlock, DocHeaderKind, FunctionKind, ResourceDecl, TypeDeclKind, Visibility,
};
use crate::binary_repr;
use crate::codegen::builtins;
use crate::hir::{
    HirConstructorArg, HirExpression, HirFile, HirFunction, HirItem, HirMatchPattern, HirProject,
    HirStatement, HirTopLevelBinding, HirTypeDecl, HirTypeField,
};
use crate::manifest::package::{resolved_package_file, source_dependency, SourceDependency};
use crate::rules;
use crate::types::ParameterType;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use tinyjson::JsonValue;

const BUILTIN_TYPES: &[&str] = &[
    "AttributedString",
    "Boolean",
    "Byte",
    "Error",
    "ErrorLoc",
    "Fixed",
    "Float",
    "Integer",
    "Money",
    "Nothing",
    "Result",
    "Scalar",
    "String",
    crate::codegen::builtins::fs::FILE_TYPE_ID,
    // bug-484: every entry below is a PACKAGE-QUALIFIED id, and that is the whole
    // point of the list. It seeds the resolver's known-type set, so a bare leaf
    // here makes that name resolvable from ANY file with no `pkg::` prefix —
    // against the governing rule, and silently, because resolution simply
    // succeeds. Six entries used to be bare (`Address`, `Datagram`, `TermColor`,
    // `TermSize`, `AudioDevice`, `Json`), which is why `AS Address` compiled from
    // a consumer while the sibling `AS Url` — same package, same kind, but absent
    // from this list — was correctly refused.
    crate::codegen::builtins::color::COLOR_TYPE_ID,
    crate::codegen::builtins::color::HSL_TYPE_ID,
    crate::codegen::builtins::term::TERM_SIZE_TYPE_ID,
    crate::codegen::builtins::net::ADDRESS_TYPE_ID,
    // plan-110-B/C: the transport types moved out of `net`. `DatagramText` is gone
    // entirely — a datagram's encoding is not something the network reports.
    crate::codegen::builtins::tcp::SOCKET_TYPE_ID,
    crate::codegen::builtins::tcp::LISTENER_TYPE_ID,
    crate::codegen::builtins::udp::SOCKET_TYPE_ID,
    crate::codegen::builtins::udp::DATAGRAM_TYPE_ID,
    crate::codegen::builtins::tls::TLS_SOCKET_TYPE_ID,
    crate::codegen::builtins::tls::TLS_LISTENER_TYPE_ID,
    crate::codegen::builtins::audio::AUDIO_INPUT_TYPE_ID,
    crate::codegen::builtins::audio::AUDIO_OUTPUT_TYPE_ID,
    crate::codegen::builtins::audio::AUDIO_DEVICE_TYPE_ID,
    crate::codegen::builtins::json::JSON_TYPE_ID,
    crate::codegen::builtins::process::PROCESS_TYPE_ID,
];

pub fn resolve_project(
    project_dir: &Path,
    manifest: &HashMap<String, JsonValue>,
    ast: &AstProject,
) -> Result<(), ()> {
    resolve_project_with(project_dir, manifest, ast, true)
}

/// Elaborate a source AST for the resolver (plan-106-D).
///
/// The resolver consumes [`HirProject`], so the AST-domain entry points above
/// convert once here. This is a FORWARD (AST→HIR) conversion — the direction the
/// compile path already runs — not a re-introduction of the de-elaboration seam
/// this letter deletes. The build path never reaches it: it already holds a
/// concrete `HirProject` and calls [`resolve_augmented`] directly.
fn elaborated(ast: &AstProject) -> HirProject {
    crate::hir::elaborate(ast)
}

/// Validate only the `DOC` blocks of an already-parsed project, without running
/// full name resolution. Used by `mfb doc` on a single source file, where the
/// surrounding project context (and lockfile) is unavailable. Returns `true`
/// when every block is valid.
pub fn validate_project_docs(project_dir: &Path, ast: &AstProject) -> bool {
    let hir = elaborated(ast);
    let mut resolver = Resolver::new(project_dir, &HashMap::new(), &hir);
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
    let augmented = augment_project(ast)?;
    resolve_augmented(
        project_dir,
        manifest,
        &elaborated(&augmented),
        validate_docs,
    )
}

/// Resolve an already-elaborated project, injecting the builtin package sources
/// first — the HIR analogue of [`resolve_project_with`] (plan-106-D).
///
/// Test-only: the build path resolves the pre-monomorph AST and then calls
/// [`resolve_augmented`] on the concrete HIR it already holds. The `ir` lowering
/// tests monomorphize a BARE project, so they need the injection first.
#[cfg(test)]
pub fn resolve_hir_project(
    project_dir: &Path,
    manifest: &HashMap<String, JsonValue>,
    hir: &HirProject,
    validate_docs: bool,
) -> Result<(), ()> {
    resolve_augmented(
        project_dir,
        manifest,
        &augment_hir_project(hir)?,
        validate_docs,
    )
}

/// Resolve an already-augmented project — the builtin package sources are already
/// injected (`augment_project`), so this does NOT re-augment. Used on the
/// post-monomorphization AST, where the monomorphizer already ran over the injected
/// sources (so their overload sets could be mangled) and re-augmenting would
/// declare every package source twice.
pub fn resolve_augmented(
    project_dir: &Path,
    manifest: &HashMap<String, JsonValue>,
    augmented: &HirProject,
    validate_docs: bool,
) -> Result<(), ()> {
    let mut resolver = Resolver::new(project_dir, manifest, augmented);
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

/// Inject every builtin package's source companion into the project, in dependency
/// order (a package importing another must be injected first so the latter's
/// `uses_package` sees the dependency). Run BEFORE monomorphization so the
/// monomorphizer sees the injected sources — in particular so a builtin's native
/// overload set (`encoding`'s `__encoding_utf8Encode`/`utf8Decode`) is mangled to
/// private `$`-symbols like any user overload, instead of colliding at codegen.
pub fn augment_project(ast: &AstProject) -> Result<AstProject, ()> {
    // registry-driven augmentation.
    let augmented = crate::codegen::registry::registry().augment_project(ast)?;

    // `term`'s injected source (the registry-modeled `LineStyle`/`FillStyle` enums)
    // and the `term`↔`astrings` `drawText(AttributedString)` bridge are injected by
    // the clean-room `registry::augment_project` above (the package's `get_mfb`
    // assembly and a `WhenBothImported("term", "astrings")` gated helper chunk).
    // `astrings`' injected source is emitted by the generic clean-room
    // `registry::augment_project` above (plan-99 PART C) whenever a program
    // `IMPORT astrings`.
    // app + datetime + money source is injected by the clean-room
    // `registry::augment_project` above.
    // `vector` source is injected by the clean-room `registry::augment_project` above
    // (it imports only the intrinsic `math` package, so it has no source-ordering
    // dependency).
    // `http` is injected before `net`: http's injected source imports `net`, so the
    // net source must be added only after http's source is present for
    // `net::uses_package` to see the dependency (plan-03-http.md Phase 4).
    let augmented = crate::codegen::builtins::http::augmented_project(&augmented)?;
    let augmented = crate::codegen::builtins::net::augmented_project(&augmented)?;
    // `audio` source (render/play synthesis + records) is injected by the generic
    // clean-room `registry::augment_project` above.
    // `process` (its `Stream`/`Signal` enum companion) is injected by the generic
    // clean-room `registry::augment_project` above.
    // `crypto` source is injected by the generic clean-room `registry::augment_project`
    // above; it runs before the `strings`/`encoding` late passes, so
    // `encoding::uses_package` still sees crypto's injected `IMPORT encoding`
    // (mirrors `http` before `net`; plan-04-crypto.md Part C).
    // `strings`' scalar-seam companion (which `IMPORT encoding`s) is injected by the
    // generic clean-room `registry::augment_project` above as a `WhenUsed` gated
    // helper (plan-99 PART B) — before this `encoding` late pass, so
    // `encoding::uses_package` still sees the seam's transitive `IMPORT encoding`.
    let augmented = crate::codegen::builtins::encoding::augmented_project(&augmented)?;
    // `color` after the generic pass, for the same reason `net` follows `http`:
    // canvas's injected companion carries `IMPORT color` and calls
    // `color::toLinear`/`fromLinear`, and the generic pass over the pre-injection
    // AST cannot see that transitive import (plan-122-B).
    let augmented = crate::codegen::builtins::color::augmented_project(&augmented)?;
    Ok(augmented)
}

/// [`augment_project`]'s chain, in the HIR domain (plan-106-D).
///
/// The same four passes in the same dependency order. Each is a thin adapter over
/// one decision procedure — `codegen::registry::ProjectView` gates the injection
/// from either domain — so the two chains cannot drift apart the way two copies
/// of the gate logic would have.
///
/// Test-only since plan-107-D: the build path augments the pre-monomorph AST
/// once and every later pass consumes that concrete HIR; the in-process tests
/// that monomorphize a BARE project are the chain's remaining callers.
#[cfg(test)]
pub fn augment_hir_project(hir: &HirProject) -> Result<HirProject, ()> {
    let augmented = crate::codegen::registry::registry().augment_hir_project(hir)?;
    let augmented = crate::codegen::builtins::http::augmented_hir_project(&augmented)?;
    let augmented = crate::codegen::builtins::net::augmented_hir_project(&augmented)?;
    let augmented = crate::codegen::builtins::encoding::augmented_hir_project(&augmented)?;
    let augmented = crate::codegen::builtins::color::augmented_hir_project(&augmented)?;
    Ok(augmented)
}

fn constructor_arg_value(argument: &HirConstructorArg) -> &HirExpression {
    match argument {
        HirConstructorArg::Positional(value) => value,
        HirConstructorArg::Named { value, .. } => value,
    }
}

/// Whether a function overload's parameter types match the type list a `DOC`
/// header named (whitespace-normalized, in order).
fn overload_types_match(function: &HirFunction, wanted: &[String]) -> bool {
    // `wanted` is raw `DOC`-header text, so it still needs whitespace
    // normalization. The declared side is a `ParameterType`, whose `name()` is
    // canonical by construction — the normalization is kept on it only so the two
    // sides stay literally the same function, as `ast::param_types` had them.
    let declared: Vec<String> = function
        .params
        .iter()
        .map(|param| match &param.type_ {
            // `ast::param_types` spelled an unannotated parameter as the empty
            // string; `Unknown` is that same absent annotation.
            ParameterType::Unknown => String::new(),
            type_ => crate::ast::normalize_ws(&type_.name()),
        })
        .collect();
    declared == crate::ast::normalize_types(wanted)
}

fn call_arg_value(argument: &crate::hir::HirCallArg) -> &HirExpression {
    use crate::hir::HirCallArg;
    match argument {
        HirCallArg::Positional(value) => value,
        HirCallArg::Named { value, .. } => value,
    }
}

/// Whether `type_name` is a raw C ABI type (mirrors the former source checker's `is_c_abi_type`),
/// which may appear only inside ABI slots (plan-link-update.md §5/§11).
fn is_c_abi_type(type_: &crate::types::ParameterType) -> bool {
    use crate::types::CAbiType;
    // plan-113: every C ABI spelling is now a `ParameterType::C`, so this asks
    // the variant instead of the interned `Symbol` a `Named` used to hold.
    //
    // **This is 12 of the 16 on purpose, and must NOT become
    // `type_.c_abi().is_some()`.** `CBool`, `CByte` and `CVoid` are excluded per
    // `17_native-libraries.md:94` ("it does **not** include `CBool`, `CByte`, or
    // `CVoid`"), and `CBuffer` with them. Widening this widens what
    // `NATIVE_CPTR_ESCAPE` rejects from a wrapper's MFBASIC-facing signature —
    // a silent behaviour change, since the obvious conversion still compiles.
    // `is_c_abi_type_recognizes_and_rejects` asserts all four negatives.
    matches!(
        type_.c_abi(),
        Some(
            CAbiType::Ptr
                | CAbiType::Str
                | CAbiType::Int8
                | CAbiType::Int16
                | CAbiType::Int32
                | CAbiType::Int64
                | CAbiType::UInt8
                | CAbiType::UInt16
                | CAbiType::UInt32
                | CAbiType::UInt64
                | CAbiType::Float
                | CAbiType::Double
        )
    )
}

/// The bare resource type with any `STATE T` clause removed.
///
/// plan-111-B deleted this module's own copy of the split — a THIRD hand-rolled
/// `STATE` grammar, and the only one that carried no composite guard at all, so
/// `List OF RES File STATE Cursor` would have truncated to `List OF RES File`
/// (the bug-429 shape). `ParameterType::without_state` is the structural
/// answer, top-level only by construction.
fn resource_base_type(type_: &crate::types::ParameterType) -> crate::types::ParameterType {
    type_.without_state()
}

struct Resolver<'a> {
    project_dir: &'a Path,
    hir: &'a HirProject,
    dependency_packages: HashMap<String, DependencyPackage>,
    top_levels: HashMap<String, Symbol>,
    functions: HashMap<String, Vec<FunctionSymbol>>,
    /// plan-111-B: the declared/imported TYPES this project knows.
    types: HashSet<crate::types::ParameterType>,
    /// LINK alias namespaces: alias (e.g. `sqliteLink`) → its native functions
    /// keyed by name. Members are resolved as `alias::func` qualified names
    /// (plan-link-update.md §5b).
    link_functions: HashMap<String, HashMap<String, LinkFnSig>>,
    /// Every name an imported non-builtin package exports, keyed by PACKAGE name
    /// (not by import binding — several bindings may name one package).
    ///
    /// Present only for a package whose interface was read successfully, which is
    /// what makes the membership test safe to reject on: a package that could not
    /// be read has already been reported, and an absent entry means "no positive
    /// knowledge", never "exports nothing" (bug-480).
    package_exports: HashMap<String, HashSet<String>>,
    active_template_params: HashSet<String>,
    had_error: bool,
}

#[derive(Clone)]
struct LinkFnSig {
    params: Vec<Option<crate::types::ParameterType>>,
    param_resource: Vec<bool>,
    line: usize,
}

#[derive(Clone)]
struct Symbol {
    file_path: String,
    line: usize,
    visibility: Visibility,
}

/// A declared parameter/return type in the overload-duplicate key, where `None`
/// means "no `AS` annotation" (plan-106-D).
///
/// HIR represents an absent annotation as [`ParameterType::Unknown`], so the
/// `Option` is reconstructed by [`declared`]. That is exactly the mapping the
/// de-elaboration seam this letter deletes already applied at this position
/// (`hir::unrender_optional_type`), so the post-monomorph pass is unchanged.
type Declared = Option<ParameterType>;

/// An HIR type field as a declared annotation: [`ParameterType::Unknown`] is the
/// absent one. `AS Unknown` is not a spellable annotation — a parameter written
/// that way is rejected as `TYPE_PARAM_REQUIRES_TYPE` ("must declare an `AS`
/// type"), which is the same conflation stated in the language.
fn declared(type_: &ParameterType) -> Declared {
    match type_ {
        ParameterType::Unknown => None,
        other => Some(other.clone()),
    }
}

#[derive(Clone)]
struct FunctionSymbol {
    symbol: Symbol,
    params: Vec<Declared>,
    /// Declared return type (`None` for a `SUB`). Part of the duplicate-detection
    /// key so two declarations sharing a name and parameter types but differing in
    /// return type form a legal return-type overload set (plan-01-overload.md §F.1).
    return_type: Declared,
}

impl<'a> Resolver<'a> {
    fn new(
        project_dir: &'a Path,
        manifest: &HashMap<String, JsonValue>,
        hir: &'a HirProject,
    ) -> Self {
        let mut resolver = Self {
            project_dir,
            hir,
            dependency_packages: dependency_packages(manifest),
            top_levels: HashMap::new(),
            functions: HashMap::new(),
            types: BUILTIN_TYPES
                .iter()
                .map(|name| crate::types::ParameterType::declared(name))
                .collect(),
            link_functions: HashMap::new(),
            package_exports: HashMap::new(),
            active_template_params: HashSet::new(),
            had_error: false,
        };
        resolver.collect_top_level_symbols(hir);
        resolver
    }

    fn collect_top_level_symbols(&mut self, hir: &HirProject) {
        // First pass: register LINK namespaces so resource declarations and
        // re-export aliases (which reference `alias::func`) can be resolved.
        for file in &hir.files {
            for item in &file.items {
                if let HirItem::Link(link) = item {
                    let entry = self.link_functions.entry(link.alias.clone()).or_default();
                    for function in &link.functions {
                        entry.insert(
                            function.name.clone(),
                            LinkFnSig {
                                params: function
                                    .params
                                    .iter()
                                    .map(|param| param.type_.clone())
                                    .collect(),
                                param_resource: function
                                    .params
                                    .iter()
                                    .map(|param| param.resource)
                                    .collect(),
                                line: function.line,
                            },
                        );
                    }
                }
            }
        }

        for file in &hir.files {
            for item in &file.items {
                match item {
                    HirItem::Binding(binding) => {
                        self.insert_top_level(
                            file,
                            &binding.name,
                            binding.line,
                            binding.visibility,
                        );
                    }
                    HirItem::Function(function) => {
                        self.insert_function(file, function);
                    }
                    HirItem::Type(type_decl) => {
                        if self.insert_top_level(
                            file,
                            &type_decl.name,
                            type_decl.line,
                            type_decl.visibility,
                        ) {
                            self.types
                                .insert(crate::types::ParameterType::declared(&type_decl.name));
                        }
                    }
                    // A native resource declaration introduces an opaque type at
                    // package scope (plan-link-update.md §5/§5a).
                    HirItem::Resource(resource) => {
                        if self.insert_top_level(
                            file,
                            &resource.name,
                            resource.line,
                            resource.visibility,
                        ) {
                            self.types
                                .insert(crate::types::ParameterType::declared(&resource.name));
                        }
                    }
                    // A re-export alias publishes a LINK function under a package
                    // name; register it as a callable carrying the target's
                    // parameter types (plan-link-update.md §5a).
                    HirItem::FuncAlias(alias) => {
                        // A LINK signature is the verbatim AST node (HIR does not
                        // elaborate `LINK` blocks), so its parameter spellings enter
                        // the `Declared` key domain here, at that one boundary.
                        let params = self
                            .link_target_signature(&alias.target)
                            .map(|sig| sig.params.iter().map(|param| param.clone()).collect())
                            .unwrap_or_default();
                        self.insert_alias_function(
                            file,
                            &alias.name,
                            alias.line,
                            alias.visibility,
                            params,
                        );
                    }
                    HirItem::Link(_) => {}
                    // DOC blocks declare no symbols; they are resolved separately
                    // after symbol collection (see `resolve_doc_blocks`).
                    HirItem::Doc(_) => {}
                    // TESTING blocks are lowered away (dropped or desugared into
                    // ordinary SUBs) before resolution runs (plan-18-A §3).
                    HirItem::Testing(_) => {}
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
        file: &HirFile,
        name: &str,
        line: usize,
        visibility: Visibility,
        params: Vec<Declared>,
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

    fn insert_function(&mut self, file: &HirFile, function: &HirFunction) {
        // A reserved general built-in (`error`) is a language primitive and may not
        // be redeclared as a user `FUNC`/`SUB` (plan-01-overload.md §A.5). Every
        // other overridable built-in name (`toString`, `len`, …) is accepted.
        if crate::codegen::builtins::general::reserved_builtin_name(&function.name) {
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
            .map(|param| declared(&param.type_))
            .collect::<Vec<_>>();
        // The duplicate-detection key is (name, parameter types, return type): two
        // declarations collide only when all three match. Sharing name + parameter
        // types but differing in return type is a legal return-type overload set
        // (plan-01-overload.md §F.1).
        // A `SUB` elaborates to `returns: Unknown`, which `declared` maps back to
        // the `None` that distinguishes it from a `FUNC … AS T` in the key.
        let return_type = declared(&function.returns);
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
        file: &HirFile,
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

    fn top_level_visible_in_file(&self, file: &HirFile, name: &str) -> bool {
        self.top_levels
            .get(name)
            .is_some_and(|symbol| self.visible_from(file, symbol.visibility, &symbol.file_path))
    }

    fn function_visible_in_file(&self, file: &HirFile, name: &str) -> bool {
        self.functions.get(name).is_some_and(|functions| {
            functions.iter().any(|function| {
                self.visible_from(file, function.symbol.visibility, &function.symbol.file_path)
            })
        })
    }

    fn visible_from(&self, file: &HirFile, visibility: Visibility, owner_file_path: &str) -> bool {
        match visibility {
            Visibility::Export | Visibility::Public => true,
            Visibility::Private => file.path == owner_file_path,
        }
    }

    fn report(&mut self, rule: &str, detail: &str, file: &HirFile, line: usize) {
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
    use crate::ast::{FunctionKind, Param, Visibility};
    use crate::hir::{HirCallArg, HirParam};
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

    fn empty_expr() -> HirExpression {
        HirExpression::Number("1".into())
    }

    /// A `LINK` block's parameter — the one node HIR keeps as the verbatim AST
    /// struct, so this stays in the AST domain.
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

    fn hir_param(name: &str, type_name: Option<&str>) -> HirParam {
        HirParam {
            name: name.into(),
            type_: type_name.map_or(ParameterType::Unknown, ParameterType::parse),
            resource: false,
            state_type: None,
            default: None,
            line: 1,
        }
    }

    fn func(name: &str, params: Vec<HirParam>) -> HirFunction {
        HirFunction {
            kind: FunctionKind::Func,
            visibility: Visibility::Export,
            isolated: false,
            name: name.into(),
            template_params: Vec::new(),
            params,
            returns: ParameterType::Unknown,
            return_resource: false,
            return_state_type: None,
            body: Vec::new(),
            trap: None,
            line: 1,
        }
    }

    fn hir_file(path: &str) -> HirFile {
        HirFile {
            path: path.into(),
            imports: Vec::new(),
            own_imports: Vec::new(),
            items: Vec::new(),
            internal: false,
        }
    }

    /// A one-file `HirProject` carrying `items`.
    fn project_of(items: Vec<HirItem>) -> HirProject {
        HirProject {
            name: "p".into(),
            files: vec![HirFile {
                path: "a.mfb".into(),
                imports: Vec::new(),
                own_imports: Vec::new(),
                items,
                internal: false,
            }],
        }
    }

    #[test]
    fn constructor_arg_value_positional_and_named() {
        let pos = HirConstructorArg::Positional(empty_expr());
        let named = HirConstructorArg::Named {
            name: "x".into(),
            value: empty_expr(),
            line: 1,
        };
        assert!(matches!(
            constructor_arg_value(&pos),
            HirExpression::Number(_)
        ));
        assert!(matches!(
            constructor_arg_value(&named),
            HirExpression::Number(_)
        ));
    }

    #[test]
    fn call_arg_value_positional_and_named() {
        let pos = HirCallArg::Positional(empty_expr());
        let named = HirCallArg::Named {
            name: "x".into(),
            value: empty_expr(),
            line: 1,
        };
        assert!(matches!(call_arg_value(&pos), HirExpression::Number(_)));
        assert!(matches!(call_arg_value(&named), HirExpression::Number(_)));
    }

    #[test]
    fn overload_types_match_variants() {
        let f = func(
            "g",
            vec![
                hir_param("a", Some("Integer")),
                hir_param("b", Some("String")),
            ],
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
        let f = func("g", vec![hir_param("a", None)]);
        assert!(overload_types_match(&f, &[String::new()]));
        assert!(!overload_types_match(&f, &["Integer".to_string()]));
    }

    #[test]
    fn is_c_abi_type_recognizes_and_rejects() {
        // plan-113: these go through `parse`, not `named`. A source-written
        // `CPtr` is a `ParameterType::C` now, and `named("CPtr")` is a nominal
        // that merely *spells* one — a value the compiler no longer mints for a
        // C ABI type, so asserting over it would test an unreachable shape.
        for t in [
            "CPtr", "CString", "CInt8", "CInt16", "CInt32", "CInt64", "CUInt8", "CUInt16",
            "CUInt32", "CUInt64", "CFloat", "CDouble",
        ] {
            assert!(
                is_c_abi_type(&crate::types::ParameterType::parse(t)),
                "{t} should be a C ABI type"
            );
        }
        // The 12-of-16 list is DELIBERATE (§3 Risk 2 of plan-113): `CBool`,
        // `CByte` and `CVoid` are excluded per `17_native-libraries.md:94`, and
        // `CBuffer` with them. Rewriting the predicate as
        // `type_.c_abi().is_some()` compiles and silently adds these four to
        // what `NATIVE_CPTR_ESCAPE` rejects; these four assertions are the only
        // thing that catches it.
        for t in ["CBool", "CByte", "CVoid", "CBuffer"] {
            assert!(
                !is_c_abi_type(&crate::types::ParameterType::parse(t)),
                "{t} must NOT be a C ABI type for NATIVE_CPTR_ESCAPE"
            );
        }
        assert!(!is_c_abi_type(&crate::types::ParameterType::parse(
            "Integer"
        )));
        assert!(!is_c_abi_type(&crate::types::ParameterType::parse("CPtrX")));
        assert!(!is_c_abi_type(&crate::types::ParameterType::parse("")));
        // A nominal that merely spells a C type is not one.
        assert!(!is_c_abi_type(&crate::types::ParameterType::named("CPtr")));
    }

    #[test]
    fn resource_base_type_strips_state_suffix() {
        assert_eq!(
            resource_base_type(&crate::types::ParameterType::parse("Handle STATE Open")).name(),
            "Handle"
        );
        assert_eq!(
            resource_base_type(&crate::types::ParameterType::parse("Handle")).name(),
            "Handle"
        );
        assert_eq!(
            resource_base_type(&crate::types::ParameterType::parse("")).name(),
            ""
        );
    }

    #[test]
    fn visible_from_rules() {
        let empty = HirProject {
            name: "p".into(),
            files: Vec::new(),
        };
        let dir = std::path::Path::new(".");
        let resolver = Resolver::new(dir, &HashMap::new(), &empty);
        let here = hir_file("a.mfb");
        assert!(resolver.visible_from(&here, Visibility::Export, "other.mfb"));
        assert!(resolver.visible_from(&here, Visibility::Public, "other.mfb"));
        assert!(resolver.visible_from(&here, Visibility::Private, "a.mfb"));
        assert!(!resolver.visible_from(&here, Visibility::Private, "other.mfb"));
    }

    #[test]
    fn top_level_and_function_visibility_lookups() {
        let file = HirFile {
            path: "a.mfb".into(),
            imports: Vec::new(),
            own_imports: Vec::new(),
            items: vec![binding("GLOBAL"), HirItem::Function(func("helper", vec![]))],
            internal: false,
        };
        let hir = HirProject {
            name: "p".into(),
            files: vec![file.clone()],
        };
        let dir = std::path::Path::new(".");
        let resolver = Resolver::new(dir, &HashMap::new(), &hir);
        assert!(resolver.top_level_visible_in_file(&file, "GLOBAL"));
        assert!(!resolver.top_level_visible_in_file(&file, "MISSING"));
        assert!(resolver.function_visible_in_file(&file, "helper"));
        assert!(!resolver.function_visible_in_file(&file, "missingfn"));
    }

    #[test]
    fn link_target_signature_lookup() {
        let dir = std::path::Path::new(".");
        let hir = HirProject {
            name: "p".into(),
            files: vec![HirFile {
                path: "lib.mfb".into(),
                imports: Vec::new(),
                own_imports: Vec::new(),
                items: vec![HirItem::Link(crate::hir::elaborate_link_block(
                    &crate::ast::LinkBlock {
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
                            buffers: Vec::new(),
                            result_length: None,
                            success_on: None,
                            result: None,
                            free: None,
                            line: 3,
                        }],
                        line: 1,
                    },
                ))],
                internal: false,
            }],
        };
        let resolver = Resolver::new(dir, &HashMap::new(), &hir);
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
        let f1 = func("dup", vec![hir_param("a", Some("Integer"))]);
        let f2 = func("dup", vec![hir_param("a", Some("Integer"))]);
        let mut f3 = func("dup2", vec![hir_param("a", Some("Integer"))]);
        f3.returns = ParameterType::Integer;
        let mut f4 = func("dup2", vec![hir_param("a", Some("Integer"))]);
        f4.returns = ParameterType::String;
        let hir = project_of(vec![
            HirItem::Function(f1),
            HirItem::Function(f2),
            HirItem::Function(f3),
            HirItem::Function(f4),
        ]);
        let dir = std::path::Path::new(".");
        let resolver = quiet(|| Resolver::new(dir, &HashMap::new(), &hir));
        assert!(resolver.had_error);
    }

    #[test]
    fn reserved_builtin_name_rejected() {
        let hir = project_of(vec![HirItem::Function(func("error", vec![]))]);
        let dir = std::path::Path::new(".");
        let resolver = quiet(|| Resolver::new(dir, &HashMap::new(), &hir));
        assert!(resolver.had_error);
    }

    #[test]
    fn type_and_resource_names_registered() {
        let hir = project_of(vec![
            HirItem::Type(HirTypeDecl {
                kind: TypeDeclKind::Type,
                visibility: Visibility::Export,
                name: "Widget".into(),
                template_params: Vec::new(),
                fields: Vec::new(),
                includes: Vec::new(),
                variants: Vec::new(),
                members: Vec::new(),
                line: 1,
            }),
            binding_at("Widget", 2),
        ]);
        let dir = std::path::Path::new(".");
        let resolver = quiet(|| Resolver::new(dir, &HashMap::new(), &hir));
        assert!(resolver
            .types
            .contains(&crate::types::ParameterType::declared("Widget")));
        assert!(resolver.had_error, "duplicate top-level should report");
    }

    fn binding(name: &str) -> HirItem {
        binding_at(name, 1)
    }

    fn binding_at(name: &str, line: usize) -> HirItem {
        HirItem::Binding(HirTopLevelBinding {
            mutable: false,
            resource: false,
            state_type: None,
            name: name.into(),
            type_: ParameterType::Unknown,
            explicit_type: false,
            value: None,
            visibility: Visibility::Export,
            line,
        })
    }

    #[test]
    fn function_name_collides_with_prior_binding_reports() {
        // A FUNC declared after a top-level binding of the same name collides.
        let hir = project_of(vec![binding("dup"), HirItem::Function(func("dup", vec![]))]);
        let dir = std::path::Path::new(".");
        let resolver = quiet(|| Resolver::new(dir, &HashMap::new(), &hir));
        assert!(resolver.had_error);
    }

    #[test]
    fn binding_name_collides_with_prior_function_reports() {
        // A binding declared after a FUNC of the same name collides (the
        // `insert_top_level` function-table branch).
        let hir = project_of(vec![HirItem::Function(func("dup", vec![])), binding("dup")]);
        let dir = std::path::Path::new(".");
        let resolver = quiet(|| Resolver::new(dir, &HashMap::new(), &hir));
        assert!(resolver.had_error);
    }

    #[test]
    fn alias_function_name_collides_with_prior_binding_reports() {
        // A FUNC re-export alias whose name matches a prior top-level binding
        // hits the `insert_alias_function` duplicate branch. The alias also needs
        // a LINK namespace so its target resolves.
        let link = HirItem::Link(crate::hir::elaborate_link_block(&crate::ast::LinkBlock {
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
                buffers: Vec::new(),
                result_length: None,
                success_on: None,
                result: None,
                free: None,
                line: 2,
            }],
            line: 1,
        }));
        let alias = HirItem::FuncAlias(crate::ast::FuncAlias {
            visibility: Visibility::Export,
            name: "dup".into(),
            target: "db.close".into(),
            line: 5,
        });
        let hir = project_of(vec![link, binding("dup"), alias]);
        let dir = std::path::Path::new(".");
        let resolver = quiet(|| Resolver::new(dir, &HashMap::new(), &hir));
        assert!(resolver.had_error);
    }

    #[test]
    fn alias_function_registers_when_unique() {
        // A unique alias registers as a callable carrying the target's params.
        let link = HirItem::Link(crate::hir::elaborate_link_block(&crate::ast::LinkBlock {
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
                buffers: Vec::new(),
                result_length: None,
                success_on: None,
                result: None,
                free: None,
                line: 2,
            }],
            line: 1,
        }));
        let alias = HirItem::FuncAlias(crate::ast::FuncAlias {
            visibility: Visibility::Export,
            name: "closeDb".into(),
            target: "db.close".into(),
            line: 5,
        });
        let hir = project_of(vec![link, alias]);
        let dir = std::path::Path::new(".");
        let file = hir.files[0].clone();
        let resolver = quiet(|| Resolver::new(dir, &HashMap::new(), &hir));
        assert!(!resolver.had_error);
        assert!(resolver.function_visible_in_file(&file, "closeDb"));
    }
}
