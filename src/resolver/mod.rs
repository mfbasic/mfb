use crate::ast::{AstProject, DocBlock, DocHeaderKind, ResourceDecl, TypeDeclKind, Visibility};
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
    // Resolution checks the program, not a build flavor: the normal sources.
    let augmented = augment_project(ast, false)?;
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
///
/// `debug` is the build's `--debug` flag: it selects the half of each
/// `RegistryHelper::debug_split` helper that is injected (plan-130-E). Every caller
/// that is not building a program passes `false`.
pub fn augment_project(ast: &AstProject, debug: bool) -> Result<AstProject, ()> {
    // registry-driven augmentation.
    let augmented = crate::codegen::registry::registry().augment_project(ast, debug)?;

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
    // `compress` after the generic pass, for the same reason: canvas's injected PNG
    // decoder calls `compress::zlibDecode`, whose gated helpers the generic pass over
    // the pre-injection AST cannot see a use of (plan-137-C).
    let augmented = crate::codegen::builtins::compress::augmented_project(&augmented)?;
    // `vector` after the generic pass, for the same reason: canvas's injected companion
    // carries `IMPORT vector` and its records name `vector::Float2`, which the generic
    // pass over the pre-injection AST cannot see (plan-172-B).
    let augmented = crate::codegen::builtins::vector::augmented_project(&augmented)?;
    Ok(augmented)
}

/// [`augment_project`]'s chain, in the HIR domain (plan-106-D).
///
/// The same passes in the same dependency order. Each is a thin adapter over
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
    let augmented = crate::codegen::builtins::compress::augmented_hir_project(&augmented)?;
    let augmented = crate::codegen::builtins::vector::augmented_hir_project(&augmented)?;
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
    /// Set while resolving a monomorph of a built-in package template
    /// (`HirFunction::internal_origin`): its locals are exempt from
    /// `SYMBOL_SHADOWS_TOP_LEVEL_BINDING` as its internal file's are.
    resolving_internal_origin: bool,
    /// plan-136-A: set while a parameter default is being resolved. A default is
    /// evaluated outside its function, so it resolves with no locals; a name it
    /// cannot resolve that is one of the function's parameters is reported as
    /// `SYMBOL_DEFAULT_NAMES_PARAMETER` rather than as an unknown identifier.
    default_scope: Option<DefaultScope>,
    /// plan-136-C: every top-level `LET`/`MUT` of a user (non-internal) file, keyed
    /// by its bare display name — a file-scoped `PRIVATE` binding's key in
    /// `top_levels` is its mangled `#<hash>$name` — to the `top_levels` keys that
    /// carry that bare name. A local binding checks it (`check_new_local`).
    top_level_bindings: HashMap<String, Vec<String>>,
    /// Every `ENUM` of a user (non-internal) file, keyed by its bare display name
    /// to the `top_levels` keys that carry it. A local of that name would make
    /// `Name.Member` ambiguous between the enum member and a field read, so a
    /// local checks it (`check_new_local`, `SYMBOL_SHADOWS_ENUM`).
    top_level_enums: HashMap<String, Vec<String>>,
    had_error: bool,
}

/// The parameter whose default is being resolved and every parameter name of its
/// function (plan-136-A).
struct DefaultScope {
    parameter: String,
    parameters: HashSet<String>,
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
    /// Declared return type (`None` when the source wrote no `AS` clause). Part of the duplicate-detection
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
            resolving_internal_origin: false,
            default_scope: None,
            top_level_bindings: HashMap::new(),
            top_level_enums: HashMap::new(),
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
                        let inserted = self.insert_top_level(
                            file,
                            &binding.name,
                            binding.line,
                            binding.visibility,
                        );
                        // plan-136-C: a built-in package's own bindings are not a
                        // user's to collide with.
                        if inserted && !file.internal {
                            self.top_level_bindings
                                .entry(
                                    crate::internal_name::display_name(&binding.name).into_owned(),
                                )
                                .or_default()
                                .push(binding.name.clone());
                        }
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
                            if type_decl.kind == TypeDeclKind::Enum && !file.internal {
                                self.top_level_enums
                                    .entry(
                                        crate::internal_name::display_name(&type_decl.name)
                                            .into_owned(),
                                    )
                                    .or_default()
                                    .push(type_decl.name.clone());
                            }
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
                    // ordinary `AS Nothing` functions) before resolution runs (plan-18-A §3).
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
                    "Top-level symbol `{}` was already declared in {}:{}.",
                    crate::internal_name::display_name(name),
                    previous.file_path,
                    previous.line
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
                    "Top-level symbol `{}` was already declared in {}:{}.",
                    crate::internal_name::display_name(name),
                    previous.symbol.file_path,
                    previous.symbol.line
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
        // be redeclared as a user `FUNC` (plan-01-overload.md §A.5). Every
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
                    crate::internal_name::display_name(&function.name),
                    previous.file_path,
                    previous.line
                ),
                file,
                function.line,
            );
            return;
        }

        // bug-913: a template's own type parameters are compared by position, so
        // `FUNC f OF T(x AS T)` and `FUNC f OF U(y AS U)` are one signature.
        let positional = |type_: &ParameterType| {
            type_.rename_vars(&|var| {
                function
                    .template_params
                    .iter()
                    .position(|param| param == var.resolve())
                    .map(|index| crate::intern::Symbol::intern(&format!("#{index}")))
            })
        };
        let params = function
            .params
            .iter()
            .map(|param| declared(&positional(&param.type_)))
            .collect::<Vec<_>>();
        // The duplicate-detection key is (name, parameter types, return type): two
        // declarations collide only when all three match. Sharing name + parameter
        // types but differing in return type is a legal return-type overload set
        // (plan-01-overload.md §F.1).
        // A FUNC with no `AS` clause elaborates to `returns: Unknown`, which
        // `declared` maps back to the `None` that distinguishes it from a
        // `FUNC … AS T` in the key.
        let return_type = declared(&positional(&function.returns));
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
                    crate::internal_name::display_name(&function.name),
                    previous.symbol.file_path,
                    previous.symbol.line
                ),
                file,
                function.line,
            );
            return;
        }

        // bug-912: a PRIVATE `FUNC f` is file-scoped to `#<hash>$f` while a
        // PUBLIC/EXPORT `FUNC f` of the same file keeps its name, so the two no
        // longer share a key above. They are still one overload set inside that
        // file, where visibility is not part of a callable's identity: the same
        // signature twice is a duplicate declaration there, whichever comes first.
        if !file.internal {
            let (private_name, sibling) =
                match crate::internal_name::private_name_parts(&function.name) {
                    Some((_, plain)) => (function.name.clone(), plain.to_string()),
                    None => {
                        let private_name = crate::internal_name::mangle_private(
                            &crate::internal_name::file_scope_hash(&file.path),
                            &function.name,
                        );
                        (private_name.clone(), private_name)
                    }
                };
            if let Some(previous) = self
                .functions
                .get(&sibling)
                .and_then(|functions| {
                    functions.iter().find(|candidate| {
                        candidate.symbol.file_path == file.path
                            && candidate.params == params
                            && candidate.return_type == return_type
                    })
                })
                .cloned()
            {
                self.report(
                    "SYMBOL_DUPLICATE_TOP_LEVEL",
                    &format!(
                        "Top-level symbol `{}` was already declared in {}:{}.",
                        crate::internal_name::display_name(&private_name),
                        previous.symbol.file_path,
                        previous.symbol.line
                    ),
                    file,
                    function.line,
                );
                return;
            }
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
                    "Top-level symbol `{}` was already declared in {}:{}.",
                    crate::internal_name::display_name(name),
                    previous.file_path,
                    previous.line
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
                    "Top-level symbol `{}` was already declared in {}:{}.",
                    crate::internal_name::display_name(name),
                    previous.symbol.file_path,
                    previous.symbol.line
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
mod tests;
