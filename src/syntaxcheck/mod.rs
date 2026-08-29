use crate::ast::{
    AstProject, ExitTarget, FunctionKind, LoopKind, TypeDeclKind, Visibility, SELF_IMPORT,
};
use crate::binary_repr::{
    self, BinaryReprExportKind, BinaryReprTypeExport, BinaryReprTypeField, BinaryReprTypeVariant,
    BinaryReprTypeVisibility,
};
use crate::codegen::builtins;
use crate::hir::{
    HirCallArg, HirConstructorArg, HirExpression, HirFile, HirFunction, HirItem, HirMatchPattern,
    HirProject, HirRecordUpdate, HirStatement, HirTopLevelBinding, HirTypeDecl, HirTypeField,
};
use crate::numeric;
use std::collections::{HashMap, HashSet};
use std::path::Path;

#[path = "builtins.rs"]
mod builtins_check;
mod checking;
mod helpers;
mod inference;
mod link;
mod resources;
mod types;

use self::helpers::*;

/// The built-in NOMINAL types: named types the language always has in scope,
/// which carry no structure of their own.
///
/// plan-106-C rung 2d: these were four `Type` variants
/// (`AttributedString`/`Error`/`ErrorLoc`/`Scalar`). They are now
/// [`Type::Named`] like any other nominal — which is what `ParameterType` models
/// them as ([`Named`](crate::types::ParameterType::Named)), and the last
/// structural difference between the two enums apart from `User` itself.
///
/// The predicate exists because two questions genuinely need it: "is this a
/// KNOWN type?" (a package's metadata may reference `Error` without declaring
/// it — `validate_package_metadata_type`) and "is this primitive-like?" (the
/// copyable / thread-sendable / comparable predicates). `ir::verify` already
/// models the same set the same way, in `is_comparable_defaultable_primitive`.
///
/// `AttributedString` is deliberately absent from
/// [`is_comparable_builtin_nominal`]: it wraps an attribute overlay like a
/// `List`, so it is copyable and defaultable but NOT comparable — never a `Map`
/// key or `Set` element (plan-89-A).
pub(super) fn is_builtin_nominal(name: &str) -> bool {
    matches!(name, "AttributedString" | "Error" | "ErrorLoc" | "Scalar")
}

/// The subset of [`is_builtin_nominal`] that is *comparable* — everything but
/// `AttributedString` (plan-89-A).
pub(super) fn is_comparable_builtin_nominal(name: &str) -> bool {
    matches!(name, "Error" | "ErrorLoc" | "Scalar")
}

/// The `Error` built-in nominal. plan-106-C rung 2d replaced the `Type::Error`
/// variant with a nominal; these constructors give each name one spelling.
pub(super) fn error_type() -> Type {
    Type::named("Error")
}

/// The `ErrorLoc` built-in nominal.
pub(super) fn error_loc_type() -> Type {
    Type::named("ErrorLoc")
}

/// The `Scalar` built-in nominal — a 32-bit Unicode scalar value (plan-41-A).
/// Register-carried like `Byte`, written with a backtick literal `` `x` ``;
/// comparable and orderable by codepoint, but **not numeric** — it never enters
/// the promotion lattice.
pub(super) fn scalar_type() -> Type {
    Type::named("Scalar")
}

/// The `AttributedString` built-in nominal (plan-89-A) — an opaque,
/// value-semantic wrapper over a visible `String` plus an attribute overlay. It
/// exposes no user-visible fields, and is copyable/defaultable but NOT
/// comparable.
///
/// Deliberately `Named("AttributedString")` and NOT
/// [`ParameterType::AttributeString`](crate::types::ParameterType), which
/// renders `"AttributeString"` — no `d` — a spelling the language's
/// attributed-text type never uses.
pub(super) fn attributed_string_type() -> Type {
    Type::named("AttributedString")
}

/// syntaxcheck's type representation **is** the compiler's one type vocabulary.
///
/// plan-106-C rung 2e: this was a private `enum Type` — the compiler's sixth
/// type representation, carrying its own copy of the type grammar. Rungs 2a-2d
/// deleted the grammar and brought the enum shape-for-shape onto
/// [`ParameterType`](crate::types::ParameterType); this alias retires it.
///
/// Two consequences are worth stating, because they are the only places
/// syntaxcheck's model and `ParameterType`'s differ in spirit:
///
/// * A resource's ` STATE T` clause rides INSIDE a nominal's spelling in
///   `ParameterType`, whereas syntaxcheck wants it beside the type
///   (`LocalInfo::state_type`). `parse_type` therefore still peels it at every
///   leaf (plan-52-D §4) — except in a thread handle's resource plane, which is
///   exactly where plan-54 wants it kept and where `split_state` hands it back.
/// * `ParameterType` has variants syntaxcheck's own parser never produces
///   (`Var`, `Arg`, `UserOf`, `MapEntryOf`, `AttributeString`). They can still
///   arrive from a decoded package signature, so matches over a type keep a tail
///   arm rather than assuming they cannot occur.
type Type = crate::types::ParameterType;

#[derive(Clone)]
struct LocalInfo {
    type_: Type,
    mutable: bool,
    /// The `STATE T` type attached to a `RES` binding/parameter, if any. Drives
    /// `s.state` member access typing.
    state_type: Option<Type>,
}

#[derive(Clone)]
struct CapturedLocal {
    name: String,
    type_: Type,
    mutable: bool,
}

#[derive(Clone)]
struct FunctionSig {
    kind: FunctionKind,
    params: Vec<ParamSig>,
    return_type: Type,
    isolated: bool,
    imported_package_export: bool,
    visibility: Visibility,
    owner_file_path: String,
}

#[derive(Clone)]
struct BindingSig {
    type_: Type,
    visibility: Visibility,
    owner_file_path: String,
}

#[derive(Clone)]
struct ParamSig {
    name: String,
    type_: Type,
    has_default: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Flow {
    FallsThrough,
    AlwaysReturns,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExprMode {
    Read,
    Transfer,
    Use,
}

/// Elaborate and check `ast`, returning the rejections collected in source
/// order **without rendering them**. The caller merges these with
/// `ir::verify`'s relocated diagnostics and renders both in one line-ordered
/// pass (plan-20-Z). An `Err` is a pre-check augmentation failure that already
/// reported itself.
/// Every identifier a LINK clause expression reads, in source order.
///
/// One copy, shared by the `SUCCESS_ON`/`RETURN` resolution check, the
/// `BUFFER … SIZE` rule-9 check, and the unbound-parameter check. It was three
/// nested `fn idents` before plan-58-B; a walker that three rules disagree about
/// is how a name gets treated as "read" by one check and unread by another.
///
/// `HirExpression::Identifier` carries no line of its own, which is why every caller
/// reports at the `ABI` line rather than the expression's.
fn link_expr_idents(expr: &crate::ast::Expression, out: &mut Vec<String>) {
    match expr {
        crate::ast::Expression::Identifier(name) => out.push(name.clone()),
        crate::ast::Expression::Binary { left, right, .. } => {
            link_expr_idents(left, out);
            link_expr_idents(right, out);
        }
        crate::ast::Expression::Unary { operand, .. } => link_expr_idents(operand, out),
        _ => {}
    }
}

pub fn check_project_collect(
    project_dir: &Path,
    hir: &crate::hir::HirProject,
) -> Result<Vec<crate::rules::PendingDiagnostic>, ()> {
    // plan-106-D: the injection runs in the HIR domain, through the SAME chain
    // the AST pipeline uses (`resolver::augment_hir_project`). syntaxcheck used to
    // carry its own copy of the four-pass sequence — a second place to keep in
    // dependency order, and to forget to update.
    let augmented = crate::resolver::augment_hir_project(hir)?;

    // `term`'s source companion (`package.mfb`) and the `term`↔`astrings`
    // `drawText(AttributedString)` bridge are injected by the clean-room
    // `registry::augment_project` above (an `Always` helper on the migrated `term`
    // package and a `WhenImported("astrings")` gated helper).
    // `astrings`' source companion (`package.mfb`) is injected by the generic
    // clean-room `registry::augment_project` above (plan-99 PART C), as an `Always`
    // helper on the migrated `astrings` package.
    // app + datetime + money source is injected by the clean-room
    // `registry::augment_project` above.
    // `vector` source is injected by the clean-room `registry::augment_project` above.
    // `http` before `net`: `http_package.mfb` imports `net` (plan-03-http.md Phase 4).
    let augmented = crate::codegen::builtins::http::augmented_hir_project(&augmented)?;
    let augmented = crate::codegen::builtins::net::augmented_hir_project(&augmented)?;
    // `audio` source (render/play synthesis + records) is injected by the generic
    // clean-room `registry::augment_project` above.
    // `process` (its `Stream`/`Signal` enum companion) is injected by the generic
    // clean-room `registry::augment_project` above.
    // `crypto` source is injected by the generic clean-room `registry::augment_project`
    // above, before the `strings`/`encoding` late passes (plan-04-crypto.md Part C).
    // `strings`' scalar-seam companion (which `IMPORT encoding`s, plan-41-D) is
    // injected by the generic clean-room `registry::augment_project` above as a
    // `WhenUsed` gated helper (plan-99 PART B) — before this `encoding` late pass, so
    // `encoding::uses_package` still sees the seam's transitive `IMPORT encoding`.
    let augmented = crate::codegen::builtins::encoding::augmented_hir_project(&augmented)?;
    let mut checker = SyntaxChecker::new(project_dir, &augmented);
    checker.check();
    Ok(checker.diagnostics)
}

/// Check `ast` and render any rejections directly (standalone callers that do
/// not run `ir::verify`, e.g. `mfb audit`). `build` uses `check_project_collect`
/// instead so it can merge the two diagnostic streams.
pub fn check_project(project_dir: &Path, hir: &crate::hir::HirProject) -> Result<(), ()> {
    let diagnostics = check_project_collect(project_dir, hir)?;
    // Warnings (`Severity::Warn`) are rendered but never fail the check — only
    // real errors do, mirroring the `build` pipeline (which gates on
    // `crate::rules::is_error`).
    let had_error = diagnostics.iter().any(|d| crate::rules::is_error(&d.rule));
    crate::rules::render_pending(diagnostics);
    if had_error {
        Err(())
    } else {
        Ok(())
    }
}

/// `EXPORT` is only meaningful in a package project — it is the flag that writes a
/// symbol into the compiled `.mfp` public API. An executable produces no `.mfp`,
/// so a top-level `EXPORT` declaration there is an error (`EXPORT_IN_EXECUTABLE`);
/// project-wide visibility inside an executable is `PUBLIC` (the default). This
/// runs in the build pipeline, where the manifest `kind` is known, so it does not
/// thread through `SyntaxChecker` (keeping `check_project_collect`'s callers, and
/// their inline `EXPORT ISOLATED` unit-test sources, unaffected).
pub fn export_in_executable_diagnostics(
    is_package: bool,
    ast: &AstProject,
) -> Vec<crate::rules::PendingDiagnostic> {
    // Reads the ORIGINAL source AST at the build boundary, before elaboration —
    // `EXPORT` placement is a source-syntax fact, and the pre-monomorph AST is
    // where the user's own declarations still are.
    use crate::ast::Item;
    if is_package {
        return Vec::new();
    }
    let mut diagnostics = Vec::new();
    for file in &ast.files {
        // Skip toolchain-provided source: injected builtin packages
        // (`HirFile::internal`) and the synthetic prelude (`<builtin …>` path),
        // which legitimately carry EXPORT declarations.
        if file.internal || file.path.starts_with('<') {
            continue;
        }
        for item in &file.items {
            let (visibility, line) = match item {
                Item::Binding(binding) => (binding.visibility, binding.line),
                Item::Function(function) => (function.visibility, function.line),
                Item::Type(type_decl) => (type_decl.visibility, type_decl.line),
                Item::Resource(resource) => (resource.visibility, resource.line),
                Item::FuncAlias(alias) => (alias.visibility, alias.line),
                Item::Link(_) | Item::Doc(_) | Item::Testing(_) => continue,
            };
            if matches!(visibility, Visibility::Export) {
                diagnostics.push(crate::rules::PendingDiagnostic {
                    rule: "EXPORT_IN_EXECUTABLE".to_string(),
                    detail: "EXPORT is only valid in a package project; use PUBLIC (the \
                             default) in an executable."
                        .to_string(),
                    path: std::path::PathBuf::from(&file.path),
                    line,
                });
            }
        }
    }
    diagnostics
}

struct SyntaxChecker<'a> {
    project_dir: &'a Path,
    hir: &'a HirProject,
    functions: HashMap<String, Vec<FunctionSig>>,
    bindings: HashMap<String, BindingSig>,
    user_types: HashSet<String>,
    user_type_kinds: HashMap<String, TypeDeclKind>,
    type_infos: HashMap<String, TypeInfo>,
    had_error: bool,
    /// Rejections collected in traversal (source) order, rendered by the caller
    /// after merging with `ir::verify`'s relocated diagnostics (plan-20-Z).
    diagnostics: Vec<crate::rules::PendingDiagnostic>,
    /// Return type of the function currently being checked. Used to validate
    /// `RETURN` inside an inline-`TRAP` handler, which is reached from
    /// `infer_expression` where the function context is otherwise unavailable.
    current_return: Type,
    /// Whether the function currently being checked is a `SUB`. A `SUB` is
    /// value-less: `RETURN` takes no value and a `SUB` call cannot be used in
    /// value position.
    current_is_sub: bool,
    /// Set true only while inferring the top expression of a bare expression
    /// statement (or the inner call of an inline `TRAP` in that position), where
    /// a value-less `SUB` call is permitted. Reset to false on entry to every
    /// other expression so a nested `SUB` call in value position is rejected.
    allow_value_less_call: bool,
    /// Stack of success types for the inline-`TRAP` handlers currently being
    /// checked (innermost last). Non-empty means a `RECOVER` is legal and must
    /// match the top type. Empty means `RECOVER` is illegal.
    inline_trap_types: Vec<Type>,
    loop_stack: Vec<LoopKind>,
    /// Resource types known to this compilation: the built-ins plus any
    /// contributed by imported packages' `RESOURCE_TABLE`. Replaces hardcoded
    /// resource recognition.
    resource_registry: crate::codegen::resource::ResourceRegistry,
    /// Callee names that act as a *re-export alias* of a registered close op,
    /// mapped to the bare resource type they close. Calling such an alias is
    /// invalidation event #1 just like the registered close op itself
    /// (plan-link-update.md §5a).
    close_op_aliases: HashMap<String, String>,
    /// Set true only while inferring the argument in a compiler-known
    /// *non-escaping* callback position (e.g. `forEach`'s action). A lambda
    /// inferred here may capture an outer `MUT` binding by-ref for the call.
    /// `infer_lambda` consumes (resets) it on entry so nested lambdas in the
    /// callback body do not inherit the licence.
    nonescaping_callback: bool,
}

#[derive(Clone)]
struct TypeInfo {
    kind: TypeDeclKind,
    visibility: Visibility,
    file_path: String,
    fields: Vec<FieldInfo>,
    variants: Vec<VariantConstructor>,
    members: HashSet<String>,
}

#[derive(Clone)]
struct FieldInfo {
    name: String,
    type_: Type,
    /// Computed per the spec's field-visibility default rule, but not consulted
    /// here: the field-visibility *rule* is enforced by `ir::verify` (plan-20),
    /// and bug-325 removed syntaxcheck's emptied shells that used to read this.
    /// It is retained rather than deleted because dropping it would leave
    /// `effective_field_visibility` (`helpers.rs:49`) with no caller, and that
    /// function is the implementation the language spec cites by name for the
    /// default rule (`spec/language/13_modules-and-packages.md`).
    #[allow(dead_code)]
    visibility: Visibility,
}

#[derive(Clone)]
struct VariantConstructor {
    name: String,
    union_name: String,
    fields: Vec<FieldInfo>,
}

impl<'a> SyntaxChecker<'a> {
    pub(super) fn new(project_dir: &'a Path, hir: &'a HirProject) -> Self {
        let mut checker = Self {
            project_dir,
            hir,
            functions: HashMap::new(),
            bindings: HashMap::new(),
            user_types: HashSet::new(),
            user_type_kinds: HashMap::new(),
            type_infos: HashMap::new(),
            had_error: false,
            diagnostics: Vec::new(),
            current_return: Type::Nothing,
            current_is_sub: false,
            allow_value_less_call: false,
            inline_trap_types: Vec::new(),
            loop_stack: Vec::new(),
            resource_registry: crate::codegen::resource::ResourceRegistry::with_builtins(),
            close_op_aliases: HashMap::new(),
            nonescaping_callback: false,
        };
        checker.collect_types();
        checker.collect_package_types();
        checker.collect_native_resources();
        checker.collect_bindings();
        checker.collect_functions();
        checker.collect_native_functions();
        checker.collect_package_functions();
        checker.collect_close_op_aliases();
        checker
    }

    /// Record each `FUNC alias AS pkg::func` re-export whose target is a
    /// resource's registered close op, so calling the alias consumes its resource
    /// argument exactly as the close op does (plan-link-update.md §5a).
    pub(super) fn collect_close_op_aliases(&mut self) {
        // close op (dotted `alias.func`) -> bare resource type it closes.
        let mut close_to_type: HashMap<String, String> = HashMap::new();
        for file in &self.hir.files {
            for item in &file.items {
                if let HirItem::Resource(resource) = item {
                    close_to_type.insert(resource.close_fn.clone(), resource.name.clone());
                }
            }
        }
        for file in &self.hir.files {
            for item in &file.items {
                if let HirItem::FuncAlias(alias) = item {
                    if let Some(type_name) = close_to_type.get(&alias.target) {
                        self.close_op_aliases
                            .insert(alias.name.clone(), type_name.clone());
                    }
                }
            }
        }
    }

    pub(super) fn collect_types(&mut self) {
        for file in &self.hir.files {
            for item in &file.items {
                if let HirItem::Type(type_decl) = item {
                    self.user_types.insert(type_decl.name.clone());
                    self.user_type_kinds
                        .insert(type_decl.name.clone(), type_decl.kind);
                }
            }
        }

        for file in &self.hir.files {
            for item in &file.items {
                if let HirItem::Type(type_decl) = item {
                    let info = self.type_info(file, type_decl);
                    self.type_infos.insert(type_decl.name.clone(), info);
                }
            }
        }

        let names = self.type_infos.keys().cloned().collect::<Vec<_>>();
        for name in names {
            let Some(TypeInfo {
                kind: TypeDeclKind::Union,
                ..
            }) = self.type_infos.get(&name)
            else {
                continue;
            };
            let expanded = self.expanded_union_variants(&name, &mut HashSet::new());
            if let Some(info) = self.type_infos.get_mut(&name) {
                info.variants = expanded;
            }
        }
    }

    pub(super) fn collect_package_types(&mut self) {
        let mut seen = HashSet::new();
        for file in &self.hir.files {
            for import in &file.imports {
                let package = import.package_name().to_string();
                if !seen.insert(package.clone()) || builtins::is_builtin_import(&package) {
                    continue;
                }
                let package_file = self
                    .project_dir
                    .join("packages")
                    .join(format!("{package}.mfp"));
                if !package_file.is_file() {
                    continue;
                }
                let Ok(type_exports) = binary_repr::read_package_type_exports(&package_file) else {
                    self.report(
                        "PACKAGE_INVALID",
                        &format!(
                            "Imported package `{package}` has unreadable or invalid type metadata."
                        ),
                        file,
                        import.line,
                    );
                    continue;
                };
                for type_export in &type_exports {
                    self.install_package_type_info(&package_file, type_export.clone());
                }
                for type_export in type_exports {
                    self.validate_imported_package_type(
                        file,
                        import.line,
                        &package_file,
                        &type_export,
                    );
                }
                self.collect_package_resources(
                    file,
                    import.binding_name(),
                    import.line,
                    &package_file,
                );
            }
        }
    }

    /// Register the resource types declared by an imported package's
    /// `RESOURCE_TABLE` so resource recognition, sendability, and the close op
    /// are driven by package metadata rather than hardcoded names. Entries are
    /// keyed by the importer-facing qualified name `binding.Type` (how the type
    /// appears in source), so `RES db AS sqlite::Db` is recognized as a resource.
    pub(super) fn collect_package_resources(
        &mut self,
        file: &HirFile,
        binding: &str,
        line: usize,
        package_file: &Path,
    ) {
        let resources = match binary_repr::read_package_resources(package_file) {
            Ok(resources) => resources,
            Err(_) => {
                self.report(
                    "PACKAGE_INVALID",
                    &format!(
                        "Imported package `{}` has an unreadable resource table.",
                        package_file.display()
                    ),
                    file,
                    line,
                );
                return;
            }
        };
        for resource in resources {
            // Built-in resources are authoritative: a package's table merely
            // references them (and older packages predate the sendable bit), so
            // never let an imported entry override the built-in's semantics. A
            // referenced builtin may be recorded by its bare base name (`File`)
            // though the builtin's identity is package-qualified (`fs.File`, plan-97).
            if crate::codegen::resource::is_builtin_backed_resource(&resource.type_name) {
                continue;
            }
            let Some(close_function) = resource.close_function else {
                // A resource entry with an unresolvable close op cannot be
                // closed safely; skip rather than register a half-formed type.
                continue;
            };
            // A native resource serializes its close op as the bare exported
            // alias name (plan-link-update.md §5a); importers call it qualified as
            // `binding.alias`, so qualify it to match (built-in close names like
            // `fs.close` are already dotted and stay as-is).
            let close_function = if resource.native && !close_function.contains('.') {
                format!("{binding}.{close_function}")
            } else {
                close_function
            };
            let info = crate::codegen::resource::ResourceInfo {
                close_function,
                sendable: resource.sendable,
                close_may_fail: resource.close_may_fail,
                kind: crate::codegen::resource::ResourceKind::Imported,
            };
            // Importer source names the type as `binding.Type`; register under
            // that key (and the bare name, for unqualified internal references).
            self.resource_registry
                .register(format!("{binding}.{}", resource.type_name), info.clone());
            self.resource_registry.register(resource.type_name, info);
        }
    }

    pub(super) fn validate_imported_package_type(
        &mut self,
        file: &HirFile,
        line: usize,
        package_file: &Path,
        type_export: &BinaryReprTypeExport,
    ) {
        let mut seen = HashSet::new();
        match type_export.kind {
            BinaryReprExportKind::Type => {
                let type_ = Type::named(&type_export.name);
                self.validate_package_metadata_type(
                    file,
                    line,
                    package_file,
                    &type_,
                    &format!("exported type `{}`", type_export.name),
                    &mut seen,
                );
            }
            BinaryReprExportKind::Union => {
                let type_ = Type::named(&type_export.name);
                self.validate_package_metadata_type(
                    file,
                    line,
                    package_file,
                    &type_,
                    &format!("exported union `{}`", type_export.name),
                    &mut seen,
                );
            }
            BinaryReprExportKind::Enum => {}
            BinaryReprExportKind::Func | BinaryReprExportKind::Sub => {}
        }
    }

    pub(super) fn validate_package_metadata_type(
        &mut self,
        file: &HirFile,
        line: usize,
        package_file: &Path,
        type_: &Type,
        context: &str,
        seen: &mut HashSet<String>,
    ) {
        match type_ {
            Type::ListOf(element)
            | Type::SetOf(element)
            | Type::ResultOf(element)
            | Type::Res(element) => {
                self.validate_package_metadata_type(
                    file,
                    line,
                    package_file,
                    element,
                    context,
                    seen,
                );
            }
            Type::MapOf(key, value) => {
                self.validate_package_metadata_type(file, line, package_file, key, context, seen);
                self.validate_package_metadata_type(file, line, package_file, value, context, seen);
                if !self.is_comparable(key) {
                    self.report(
                        "PACKAGE_INVALID",
                        &format!(
                            "Imported package `{}` has {context} with non-comparable map key type `{}`.",
                            package_file.display(),
                            self.type_name(key)
                        ),
                        file,
                        line,
                    );
                }
            }
            Type::Func(params, return_type, _) => {
                for param in params {
                    self.validate_package_metadata_type(
                        file,
                        line,
                        package_file,
                        param,
                        context,
                        seen,
                    );
                }
                self.validate_package_metadata_type(
                    file,
                    line,
                    package_file,
                    return_type,
                    context,
                    seen,
                );
            }
            Type::ThreadHandle {
                msg: message,
                res: resource,
                out: output,
                ..
            } => {
                self.validate_package_metadata_type(
                    file,
                    line,
                    package_file,
                    message,
                    context,
                    seen,
                );
                // plan-106-C rung 2e: an absent resource plane is `Nothing`, and
                // the plane's ` STATE T` rides inside its spelling — so the two
                // walks the separate `res_state` slot used to need become one
                // `split_state` here.
                let (plane_resource, plane_state) = resource.split_state();
                if !matches!(plane_resource, Type::Nothing) {
                    self.validate_package_metadata_type(
                        file,
                        line,
                        package_file,
                        &plane_resource,
                        context,
                        seen,
                    );
                }
                if let Some(plane_state) = &plane_state {
                    self.validate_package_metadata_type(
                        file,
                        line,
                        package_file,
                        plane_state,
                        context,
                        seen,
                    );
                }
                self.validate_package_metadata_type(
                    file,
                    line,
                    package_file,
                    output,
                    context,
                    seen,
                );
            }
            // A built-in nominal (`Error`, `Scalar`, …) is always in scope and
            // declares no fields, so there is nothing to walk and nothing to
            // report — it was one of the four inert `=> {}` variants before rung
            // 2d. Checked BEFORE the general `Named` arm, which would otherwise
            // report it as a type the package references but does not declare.
            Type::Named(name) if is_builtin_nominal(name.resolve()) => {}
            Type::Named(name) => {
                let name = name.resolve();
                if self.resource_registry.is_resource(name) || !seen.insert(name.to_string()) {
                    return;
                }
                let Some(info) = self.type_infos.get(name).cloned() else {
                    self.report(
                        "PACKAGE_INVALID",
                        &format!(
                            "Imported package `{}` has {context} that references unknown type `{name}`.",
                            package_file.display()
                        ),
                        file,
                        line,
                    );
                    return;
                };
                match info.kind {
                    TypeDeclKind::Enum => {}
                    TypeDeclKind::Type => {
                        for field in &info.fields {
                            self.validate_package_metadata_type(
                                file,
                                line,
                                package_file,
                                &field.type_,
                                context,
                                seen,
                            );
                        }
                    }
                    TypeDeclKind::Union => {
                        for variant in &info.variants {
                            for field in &variant.fields {
                                self.validate_package_metadata_type(
                                    file,
                                    line,
                                    package_file,
                                    &field.type_,
                                    context,
                                    seen,
                                );
                            }
                        }
                    }
                }
                seen.remove(name);
            }
            Type::Boolean
            | Type::Byte
            | Type::Fixed
            | Type::Float
            | Type::Integer
            | Type::Money
            | Type::Nothing
            | Type::String
            | Type::Unknown => {}
            // `ParameterType` carries variants syntaxcheck's own parser never
            // produces (`Var`, `Arg`, `UserOf`, `MapEntryOf`, `AttributeString`);
            // a decoded package signature can still hold one. Before plan-106-C
            // rung 2e each arrived spelled out as `Type::User(<spelling>)` and so
            // took the NOMINAL arm above — routing the render back through it
            // reproduces that exactly, rather than guessing a new answer for a
            // shape this checker has never had to answer for.
            other => self.validate_package_metadata_type(
                file,
                line,
                package_file,
                &Type::named(&other.name()),
                context,
                seen,
            ),
        }
    }

    pub(super) fn collect_package_functions(&mut self) {
        let mut seen = HashSet::new();
        for file in &self.hir.files {
            for import in &file.imports {
                let binding = import.binding_name().to_string();
                let package = import.package_name().to_string();
                if !seen.insert(binding.clone()) || builtins::is_builtin_import(&package) {
                    continue;
                }
                // `IMPORT self` binds the current package's own EXPORT interface,
                // not a `.mfp` on disk. Register those exports under the `self`/alias
                // binding as imported-package signatures so `self::worker` looks up
                // like any external import and the thread-entry checker accepts it
                // with zero `self` awareness (plan-81-import-self.md §4.2). Reached
                // only in a package project — an executable's `IMPORT self` is a hard
                // resolver error (`IMPORT_SELF_IN_EXECUTABLE`) that aborts the build
                // before syntaxcheck runs.
                if package == SELF_IMPORT {
                    self.collect_self_exports(&binding);
                    continue;
                }
                let package_file = self
                    .project_dir
                    .join("packages")
                    .join(format!("{package}.mfp"));
                if !package_file.is_file() {
                    continue;
                }
                let Ok(exports) = binary_repr::read_package_exports(&package_file) else {
                    self.report(
                        "PACKAGE_INVALID",
                        &format!(
                            "Imported package `{package}` has unreadable or invalid function metadata."
                        ),
                        file,
                        import.line,
                    );
                    continue;
                };
                for export in exports {
                    let sig = FunctionSig {
                        kind: match export.kind {
                            BinaryReprExportKind::Func => FunctionKind::Func,
                            BinaryReprExportKind::Sub => FunctionKind::Sub,
                            BinaryReprExportKind::Type
                            | BinaryReprExportKind::Union
                            | BinaryReprExportKind::Enum => continue,
                        },
                        params: export
                            .params
                            .into_iter()
                            .map(|param| ParamSig {
                                name: param.name,
                                type_: self.parse_type(&param.type_),
                                has_default: param.has_default,
                            })
                            .collect(),
                        return_type: self.parse_type(&export.return_type),
                        isolated: export.isolated,
                        imported_package_export: true,
                        visibility: Visibility::Export,
                        owner_file_path: package_file.display().to_string(),
                    };
                    self.validate_imported_function_signature(
                        file,
                        import.line,
                        &package_file,
                        &export.name,
                        &sig,
                    );
                    self.functions
                        .entry(format!("{binding}.{}", export.name))
                        .or_default()
                        .push(sig);
                }
            }
        }
    }

    /// Register the current project's EXPORT top-level FUNC/SUB declarations under
    /// the `self`/alias `binding` as imported-package signatures
    /// (`imported_package_export = true`, `Visibility::Export`), mirroring the
    /// `.mfp` export-loading loop in `collect_package_functions`. This is what makes
    /// `self::worker` resolve to a signature the thread-entry checker accepts, and
    /// — because only EXPORT declarations are registered — makes `self::` expose
    /// exactly the public API an external importer would see (a `self::` reference
    /// to a PUBLIC/PRIVATE symbol finds no `self.`-keyed sig and fails, just like an
    /// external import). The existing bare in-project registrations (flag `false`,
    /// keyed by bare name) from `collect_functions` are left untouched, so ordinary
    /// unqualified in-project calls are unaffected (plan-81-import-self.md §4.2).
    fn collect_self_exports(&mut self, binding: &str) {
        for file in &self.hir.files {
            for item in &file.items {
                let HirItem::Function(function) = item else {
                    continue;
                };
                if function.visibility != Visibility::Export {
                    continue;
                }
                let return_type = match function.kind {
                    // An unannotated `FUNC` elaborates to `Unknown`, which is the
                    // same answer the `Option` map produced.
                    FunctionKind::Func => self.normalize_type(&function.returns),
                    FunctionKind::Sub => Type::Nothing,
                };
                let params = function
                    .params
                    .iter()
                    .map(|param| ParamSig {
                        name: param.name.clone(),
                        type_: self.normalize_type(&param.type_),
                        has_default: param.default.is_some(),
                    })
                    .collect();
                self.functions
                    .entry(format!("{binding}.{}", function.name))
                    .or_default()
                    .push(FunctionSig {
                        kind: function.kind,
                        params,
                        return_type,
                        isolated: function.isolated,
                        imported_package_export: true,
                        visibility: Visibility::Export,
                        owner_file_path: file.path.clone(),
                    });
            }
        }
    }

    pub(super) fn validate_imported_function_signature(
        &mut self,
        file: &HirFile,
        line: usize,
        package_file: &Path,
        function_name: &str,
        sig: &FunctionSig,
    ) {
        let mut seen = HashSet::new();
        for param in &sig.params {
            self.validate_package_metadata_type(
                file,
                line,
                package_file,
                &param.type_,
                &format!(
                    "exported function `{function_name}` parameter `{}`",
                    param.name
                ),
                &mut seen,
            );
        }
        self.validate_package_metadata_type(
            file,
            line,
            package_file,
            &sig.return_type,
            &format!("exported function `{function_name}` return type"),
            &mut seen,
        );
    }

    pub(super) fn install_package_type_info(
        &mut self,
        package_file: &Path,
        type_export: BinaryReprTypeExport,
    ) {
        let BinaryReprTypeExport {
            name,
            kind,
            fields,
            variants,
            members,
            // bug-390: fields/variants are already resolved from the owning
            // package by `read_package_type_exports`, so a re-exported type is
            // installed exactly like a locally-defined one.
            foreign_owner: _,
        } = type_export;
        self.user_types.insert(name.clone());
        let kind = match kind {
            BinaryReprExportKind::Type => TypeDeclKind::Type,
            BinaryReprExportKind::Union => TypeDeclKind::Union,
            BinaryReprExportKind::Enum => TypeDeclKind::Enum,
            BinaryReprExportKind::Func | BinaryReprExportKind::Sub => return,
        };
        self.user_type_kinds.insert(name.clone(), kind);
        self.type_infos.insert(
            name,
            TypeInfo {
                kind,
                visibility: Visibility::Export,
                file_path: package_file.display().to_string(),
                fields: fields
                    .into_iter()
                    .map(|field| self.package_field_info(field))
                    .collect(),
                variants: variants
                    .into_iter()
                    .map(|variant| self.package_variant_info(variant))
                    .collect(),
                members: members.into_iter().collect(),
            },
        );
    }

    pub(super) fn package_field_info(&self, field: BinaryReprTypeField) -> FieldInfo {
        FieldInfo {
            name: field.name,
            type_: self.parse_type(&field.type_),
            visibility: match field.visibility {
                BinaryReprTypeVisibility::Private => Visibility::Private,
                BinaryReprTypeVisibility::Public => Visibility::Public,
                BinaryReprTypeVisibility::Export => Visibility::Export,
            },
        }
    }

    pub(super) fn package_variant_info(
        &self,
        variant: BinaryReprTypeVariant,
    ) -> VariantConstructor {
        VariantConstructor {
            name: variant.name,
            union_name: String::new(),
            fields: variant
                .fields
                .into_iter()
                .map(|field| self.package_field_info(field))
                .collect(),
        }
    }

    pub(super) fn expanded_union_variants(
        &self,
        union_name: &str,
        visiting: &mut HashSet<String>,
    ) -> Vec<VariantConstructor> {
        if !visiting.insert(union_name.to_string()) {
            return Vec::new();
        }
        let mut variants = Vec::new();
        let includes = self
            .hir
            .files
            .iter()
            .flat_map(|file| &file.items)
            .find_map(|item| {
                let HirItem::Type(type_decl) = item else {
                    return None;
                };
                if type_decl.name == union_name {
                    Some(type_decl.includes.clone())
                } else {
                    None
                }
            })
            .unwrap_or_default();
        for include in includes {
            for mut variant in self.expanded_union_variants(&include, visiting) {
                variant.union_name = union_name.to_string();
                variants.push(variant);
            }
        }
        if let Some(info) = self.type_infos.get(union_name) {
            variants.extend(info.variants.iter().map(|variant| {
                let mut expanded = variant.clone();
                expanded.fields = self
                    .type_infos
                    .get(&variant.name)
                    .filter(|member| matches!(member.kind, TypeDeclKind::Type))
                    .map(|member| member.fields.clone())
                    .unwrap_or_default();
                expanded
            }));
        }
        visiting.remove(union_name);
        variants
    }

    pub(super) fn type_info(&self, file: &HirFile, type_decl: &HirTypeDecl) -> TypeInfo {
        let fields = type_decl
            .fields
            .iter()
            .map(|field| self.field_info(field, type_decl.visibility))
            .collect();
        let variants = type_decl
            .variants
            .iter()
            .map(|variant| VariantConstructor {
                name: variant.name.clone(),
                union_name: type_decl.name.clone(),
                fields: Vec::new(),
            })
            .collect();
        let members = type_decl
            .members
            .iter()
            .map(|member| member.name.clone())
            .collect();
        TypeInfo {
            kind: type_decl.kind,
            visibility: type_decl.visibility,
            file_path: file.path.clone(),
            fields,
            variants,
            members,
        }
    }

    pub(super) fn field_info(
        &self,
        field: &HirTypeField,
        containing_visibility: Visibility,
    ) -> FieldInfo {
        FieldInfo {
            name: field.name.clone(),
            type_: self.normalize_type(&field.type_),
            visibility: effective_field_visibility(field.visibility, containing_visibility),
        }
    }

    pub(super) fn collect_bindings(&mut self) {
        for file in &self.hir.files {
            for item in &file.items {
                if let HirItem::Binding(binding) = item {
                    let type_ = self.normalize_type(&binding.type_);
                    self.bindings.insert(
                        binding.name.clone(),
                        BindingSig {
                            type_,
                            visibility: binding.visibility,
                            owner_file_path: file.path.clone(),
                        },
                    );
                }
            }
        }
    }

    pub(super) fn collect_functions(&mut self) {
        for file in &self.hir.files {
            for item in &file.items {
                if let HirItem::Function(function) = item {
                    let return_type = match function.kind {
                        FunctionKind::Func => self.normalize_type(&function.returns),
                        FunctionKind::Sub => Type::Nothing,
                    };
                    let params = function
                        .params
                        .iter()
                        .map(|param| ParamSig {
                            name: param.name.clone(),
                            type_: self.normalize_type(&param.type_),
                            has_default: param.default.is_some(),
                        })
                        .collect();
                    self.functions
                        .entry(function.name.clone())
                        .or_default()
                        .push(FunctionSig {
                            kind: function.kind,
                            params,
                            return_type,
                            isolated: function.isolated,
                            imported_package_export: false,
                            visibility: function.visibility,
                            owner_file_path: file.path.clone(),
                        });
                }
            }
        }
    }

    pub(super) fn canonical_import_name(&self, file: &HirFile, name: &str) -> String {
        let Some((binding, rest)) = name.split_once('.') else {
            return name.to_string();
        };
        let imports = file.import_bindings();
        let Some(package) = imports.get(binding) else {
            return name.to_string();
        };
        format!("{package}.{rest}")
    }

    pub(super) fn visible_function_sigs<'b>(
        &'b self,
        file: &HirFile,
        name: &str,
    ) -> Vec<&'b FunctionSig> {
        self.functions
            .get(name)
            .into_iter()
            .flatten()
            .filter(|sig| self.visible_from(file, sig.visibility, &sig.owner_file_path))
            .collect()
    }

    pub(super) fn lookup_visible_function<'b>(
        &'b self,
        file: &HirFile,
        name: &str,
    ) -> Option<&'b FunctionSig> {
        let visible = self.visible_function_sigs(file, name);
        if visible.len() == 1 {
            return visible.into_iter().next();
        }
        visible.into_iter().last()
    }

    pub(super) fn lookup_visible_binding<'b>(
        &'b self,
        file: &HirFile,
        name: &str,
    ) -> Option<&'b BindingSig> {
        self.bindings
            .get(name)
            .filter(|sig| self.visible_from(file, sig.visibility, &sig.owner_file_path))
    }

    pub(super) fn lookup_visible_call_sig<'b>(
        &'b self,
        file: &HirFile,
        name: &str,
        arguments: &[HirCallArg],
        expected: Option<&Type>,
    ) -> Option<&'b FunctionSig> {
        let visible = self.visible_function_sigs(file, name);
        if visible.len() <= 1 {
            return visible.into_iter().next();
        }

        let matching = visible
            .into_iter()
            .filter(|sig| self.call_shape_matches_sig(arguments, sig))
            .collect::<Vec<_>>();
        if matching.len() == 1 {
            return matching.into_iter().next();
        }
        // More than one candidate survives the shape filter — a return-type
        // overload set (param-distinguished sets resolve above, since the
        // monomorphizer has already rewritten each call to a single mangled
        // symbol). Disambiguate by the call's expected (contextual) type
        // (plan-01-overload.md §F.2.3); fall back to the last candidate when no
        // expected type uniquely selects one, preserving prior behaviour.
        if let Some(expected) = expected {
            let mut by_return = matching
                .iter()
                .filter(|sig| sig.return_type == *expected)
                .copied();
            if let Some(unique) = by_return.next() {
                if by_return.next().is_none() {
                    return Some(unique);
                }
            }
        }
        matching.into_iter().last()
    }

    pub(super) fn call_shape_matches_sig(
        &self,
        arguments: &[HirCallArg],
        sig: &FunctionSig,
    ) -> bool {
        let positional = arguments
            .iter()
            .take_while(|argument| matches!(argument, HirCallArg::Positional(_)))
            .count();
        if positional > sig.params.len() {
            return false;
        }

        let required = sig.params.iter().filter(|param| !param.has_default).count();
        if arguments.len() < required || arguments.len() > sig.params.len() {
            return false;
        }

        let mut seen = HashSet::new();
        for argument in arguments {
            let HirCallArg::Named { name, .. } = argument else {
                continue;
            };
            if !seen.insert(name) {
                return false;
            }
            if !sig.params.iter().any(|param| param.name == *name) {
                return false;
            }
        }
        true
    }

    pub(super) fn check(&mut self) {
        for file in &self.hir.files {
            for item in &file.items {
                match item {
                    HirItem::Binding(binding) => self.check_binding(file, binding),
                    HirItem::Function(function) => self.check_function(file, function),
                    HirItem::Type(type_decl) => self.check_type_decl(file, type_decl),
                    // A RESOURCE declaration's structural checks run during
                    // resolve; the built-in-shadow rule that lived here
                    // (RESOURCE_SHADOWS_BUILTIN) compared a bare declaration name
                    // against package-qualified built-in keys and could not fire
                    // since plan-97 — retired (plan-107-B).
                    HirItem::Resource(_) => {}
                    HirItem::Link(link) => self.check_link_block(file, link),
                    // A re-export alias carries no body to check; its target was
                    // validated during resolve (plan-link-update.md §5a).
                    HirItem::FuncAlias(_) => {}
                    // DOC blocks carry no executable code to syntaxcheck.
                    HirItem::Doc(_) => {}
                    // TESTING blocks are lowered away before syntaxcheck (plan-18-A §3).
                    HirItem::Testing(_) => {}
                }
            }
        }
    }

    pub(super) fn check_binding(&mut self, file: &HirFile, binding: &HirTopLevelBinding) {
        let mut locals = HashMap::new();
        let declared = declared(&binding.type_).map(|type_| self.normalize_type(type_));
        if let Some(declared) = &declared {
            self.check_type_reference(file, declared, binding.line);
        }
        let inferred = binding.value.as_ref().map(|value| {
            self.infer_expression_with_expected(
                file,
                value,
                &mut locals,
                binding.line,
                declared.as_ref(),
                ExprMode::Read,
            )
        });
        self.check_binding_shape(
            file,
            &binding.name,
            binding.mutable,
            binding.line,
            declared.as_ref(),
            inferred.as_ref(),
            binding.value.as_ref(),
        );
        let binding_type = declared.or(inferred).unwrap_or(Type::Unknown);
        self.check_resource_declaration(
            file,
            binding.line,
            binding.resource,
            binding.state_type.as_ref(),
            (binding_type != Type::Unknown).then_some(&binding_type),
            &format!("binding `{}`", binding.name),
        );
        if let Some(sig) = self.bindings.get_mut(&binding.name) {
            sig.type_ = binding_type;
        }
    }

    pub(super) fn check_type_decl(&mut self, file: &HirFile, type_decl: &HirTypeDecl) {
        match type_decl.kind {
            TypeDeclKind::Type => {
                for field in &type_decl.fields {
                    let type_ = self.normalize_type(&field.type_);
                    self.check_type_reference(file, &type_, field.line);
                }
            }
            TypeDeclKind::Union => {
                for include in &type_decl.includes {
                    let type_ = self.parse_type(include);
                    self.check_type_reference(file, &type_, type_decl.line);
                }

                for variant in &type_decl.variants {
                    let type_ = self.parse_type(&variant.name);
                    self.check_type_reference(file, &type_, variant.line);
                }
            }
            TypeDeclKind::Enum => {}
        }
    }

    pub(super) fn check_function(&mut self, file: &HirFile, function: &HirFunction) {
        // TYPE_ISOLATED_NOT_VISIBLE is `ir::verify`'s (plan-107-A).
        let expected_return = match function.kind {
            FunctionKind::Func => {
                if declared(&function.returns).is_none() {
                    Type::Unknown
                } else {
                    let return_type = self.normalize_type(&function.returns);
                    // `check_type_reference` reports `TYPE_RESULT_NOT_USER_VISIBLE`
                    // for a `Result` in any type position, including this one.
                    self.check_type_reference(file, &return_type, function.line);
                    if matches!(return_type, Type::ResultOf(_)) {
                        Type::Unknown
                    } else {
                        return_type
                    }
                }
            }
            FunctionKind::Sub => {
                if declared(&function.returns).is_some() {
                    self.report(
                        "TYPE_SUB_CANNOT_RETURN_VALUE",
                        &format!("SUB `{}` cannot declare a return type.", function.name),
                        file,
                        function.line,
                    );
                }
                Type::Nothing
            }
        };

        if matches!(function.kind, FunctionKind::Func) {
            if let Some(declared_return) = declared(&function.returns) {
                let return_type = self.normalize_type(declared_return);
                self.check_resource_declaration(
                    file,
                    function.line,
                    function.return_resource,
                    function.return_state_type.as_ref(),
                    Some(&return_type),
                    "return type",
                );
                // Returning `List OF RES fs::File` transfers scope-ownership of the
                // referenced resources to the caller, which adopts them (§15.6).
                // (A bare `List OF fs::File` return is already rejected at the type
                // level, since a resource element must be `RES`-marked.)
            }
        }

        let mut locals = HashMap::new();
        for param in &function.params {
            let param_type = self.normalize_type(&param.type_);
            self.check_type_reference(file, &param_type, param.line);

            self.check_resource_declaration(
                file,
                param.line,
                param.resource,
                param.state_type.as_ref(),
                (param_type != Type::Unknown).then_some(&param_type),
                &format!("parameter `{}`", param.name),
            );

            if let Some(default) = &param.default {
                let default_type =
                    self.infer_expression(file, default, &mut locals, param.line, ExprMode::Read);
                if matches!(default_type, Type::Unknown) {
                    self.report(
                        "TYPE_UNKNOWN_VALUE",
                        &format!(
                            "Default value for `{}` does not have a known type.",
                            param.name
                        ),
                        file,
                        param.line,
                    );
                }
            }

            let state_type = param.state_type.clone();
            locals.insert(
                param.name.clone(),
                LocalInfo {
                    type_: param_type,
                    mutable: false,
                    state_type,
                },
            );
        }

        self.current_return = expected_return.clone();
        self.current_is_sub = matches!(function.kind, FunctionKind::Sub);
        self.inline_trap_types.clear();
        let flow = self.check_block(file, &function.body, &expected_return, &mut locals, None);
        if let Some(trap) = &function.trap {
            let mut trap_locals = locals.clone();
            trap_locals.insert(
                trap.name.clone(),
                LocalInfo {
                    type_: error_type(),
                    mutable: false,
                    state_type: None,
                },
            );
            let trap_flow = self.check_block(
                file,
                &trap.body,
                &expected_return,
                &mut trap_locals,
                Some(trap.name.as_str()),
            );
            // Both TYPE_TRAP_FALLTHROUGH forms (the handler falling through,
            // the normal flow reaching the handler) are `ir::verify`'s
            // (plan-107-B); the flows are still computed for their inference
            // side effects.
            let _ = (flow, trap_flow);
        }
    }

    pub(super) fn visible_from(
        &self,
        file: &HirFile,
        visibility: Visibility,
        owner_file_path: &str,
    ) -> bool {
        match visibility {
            Visibility::Export | Visibility::Public => true,
            Visibility::Private => file.path == owner_file_path,
        }
    }

    pub(super) fn check_type_reference(&mut self, file: &HirFile, type_: &Type, line: usize) {
        match type_ {
            Type::ListOf(element) => {
                let inner = strip_res(element);
                self.check_type_reference(file, inner, line);
            }
            // The collection ownership rejections
            // (TYPE_COLLECTION_OWNERSHIP_VIOLATION) are `ir::verify`'s
            // (plan-107-B); only the type-reference walk into the element,
            // key and value positions remains here.
            Type::SetOf(element) => {
                self.check_type_reference(file, element, line);
            }
            Type::MapOf(key, value) => {
                let value_inner = strip_res(value);
                self.check_type_reference(file, key, line);
                self.check_type_reference(file, value_inner, line);
                self.require_comparable_type(file, line, "Map key type", key);
            }
            Type::Res(inner) => self.check_type_reference(file, inner, line),
            Type::Func(params, return_type, _) => {
                for param in params {
                    self.check_type_reference(file, param, line);
                }
                self.check_type_reference(file, return_type, line);
            }
            // `Result`/`Ok` in a type position is TYPE_RESULT_NOT_USER_VISIBLE —
            // the RESOLVER's rule (`resolution.rs::resolve_type`), which reports
            // every such position and short-circuits the build before this
            // checker runs. The copies that lived here were unreachable
            // (plan-107-B, evidence in plan-107-A Corrections C-dead-rules).
            Type::ResultOf(_) => {}
            Type::ThreadHandle {
                msg: message,
                res: resource,
                out: output,
                ..
            } => {
                // The planes' sendability (TYPE_THREAD_NOT_SENDABLE) is
                // `ir::verify`'s (plan-107-A); only the type-reference walk into
                // each plane remains here.
                self.check_type_reference(file, message, line);
                self.check_type_reference(file, output, line);
                // plan-106-C rung 2e: an absent plane is `Nothing`, and the
                // plane's ` STATE T` rides inside its spelling — `split_state`
                // gives back exactly what the separate `res_state` slot held.
                let (plane_resource, plane_state) = resource.split_state();
                if !matches!(plane_resource, Type::Nothing) {
                    self.check_type_reference(file, &plane_resource, line);
                }
                // The plane's `STATE T` payload type (plan-54) must resolve, and
                // its defaultability/copyability is enforced by ir::verify as for a
                // stateful binding.
                //
                // bug-301 G4: it must ALSO be sendable, which nothing checked. The
                // STATE rides the transfer plane into the receiving thread and is
                // deep-copied into its arena, so it crosses the boundary exactly as
                // the message and resource types do. `ir::verify`'s copyable +
                // defaultable rule does not imply sendable: a record like
                // `TYPE S { files AS List OF RES fs::File }` satisfies both yet carries
                // resource pointers to sender-owned resources, which §15.6 forbids
                // from crossing.
                if let Some(plane_state) = &plane_state {
                    self.check_type_reference(file, plane_state, line);
                }
            }
            Type::Named(_) => {}
            Type::Boolean
            | Type::Byte
            | Type::Fixed
            | Type::Float
            | Type::Integer
            | Type::Money
            | Type::Nothing
            | Type::String
            | Type::Unknown => {}
            // `ParameterType` carries variants syntaxcheck's own parser never
            // produces (`Var`, `Arg`, `UserOf`, `MapEntryOf`, `AttributeString`);
            // a decoded package signature can still hold one. Before plan-106-C
            // rung 2e each arrived spelled out as `Type::User(<spelling>)` and so
            // took the NOMINAL arm above — routing the render back through it
            // reproduces that exactly, rather than guessing a new answer for a
            // shape this checker has never had to answer for.
            other => self.check_type_reference(file, &Type::named(&other.name()), line),
        }
    }

    /// The rendered name of a type, for diagnostics.
    ///
    /// plan-106-C rung 2e: [`ParameterType::name`](crate::types::ParameterType)
    /// IS this function. A 50-line match, plus `format_thread_type_name` and
    /// `thread_type_argument_name`, collapsed into it when `Type` became the
    /// alias. Kept as a named seam because its 55 call sites read better saying
    /// what they want than reaching for `.name().into_owned()`.
    pub(super) fn type_name(&self, type_: &Type) -> String {
        type_.name().into_owned()
    }

    pub(super) fn report(&mut self, rule: &str, detail: &str, file: &HirFile, line: usize) {
        // plan-20-Z: every relocated rule's emission site has been DELETED from
        // `syntaxcheck` — `ir::verify` is the single source of truth for them.
        // This function now carries only the erased-syntax rules (constructs
        // total lowering removes, which no IR checker can see) and elaboration
        // stays untouched.
        debug_assert!(
            !crate::ir::RELOCATED_TO_IR_VERIFY.contains(&rule),
            "rule {rule} is relocated to ir::verify; syntaxcheck must not emit it"
        );
        self.had_error = true;
        self.diagnostics.push(crate::rules::PendingDiagnostic {
            rule: rule.to_string(),
            detail: detail.to_string(),
            path: self.project_dir.join(&file.path),
            line,
        });
    }

    /// Emit a **non-fatal** advisory diagnostic (a `Severity::Warn` rule). It is
    /// collected and rendered like any diagnostic but does not fail the build
    /// (`crate::rules::is_error` gates the pipeline), so `had_error` stays unset.
    /// Used for rules that flag a benign condition (e.g. a provably-dead inline
    /// TRAP handler) without rejecting the program.
    pub(super) fn report_warning(&mut self, rule: &str, detail: &str, file: &HirFile, line: usize) {
        debug_assert!(
            !crate::ir::RELOCATED_TO_IR_VERIFY.contains(&rule),
            "rule {rule} is relocated to ir::verify; syntaxcheck must not emit it"
        );
        self.diagnostics.push(crate::rules::PendingDiagnostic {
            rule: rule.to_string(),
            detail: detail.to_string(),
            path: self.project_dir.join(&file.path),
            line,
        });
    }
}

/// Shared test harness for the `syntaxcheck` unit tests. Builds a single-file
/// `AstProject` from an MFBASIC source string and runs the checker, returning
/// the collected rule codes (in traversal order). Builtin package sources are
/// injected on demand by `check_project_collect` when their imports appear, so
/// tests can freely `USES collections`, `strings`, etc.
#[cfg(test)]
pub(crate) mod testutil {
    use super::*;
    use crate::ast::parse_source;
    use std::path::Path;

    /// Parse `src` as `main.mfb`, run the checker, and return the emitted rule
    /// codes in order. Panics on a lex/parse failure (test-author error).
    pub(crate) fn check_src(src: &str) -> Vec<String> {
        let file = parse_source(Path::new("main.mfb"), "main.mfb", src)
            .expect("test source must lex+parse");
        let project = AstProject {
            name: "test".to_string(),
            files: vec![file],
        };
        let diagnostics = check_project_collect(Path::new("."), &crate::hir::elaborate(&project))
            .expect("builtin augmentation must succeed");
        diagnostics.into_iter().map(|d| d.rule).collect()
    }

    /// True when `src` passes the checker with no rejections.
    pub(crate) fn accepts(src: &str) -> bool {
        check_src(src).is_empty()
    }

    /// True when `src` is rejected and `rule` is among the emitted codes.
    pub(crate) fn rejects_with(src: &str, rule: &str) -> bool {
        check_src(src).iter().any(|r| r == rule)
    }

    /// Load a project from a directory on disk (fixtures for `.mfp` package
    /// metadata validation) and return the emitted rule codes.
    pub(crate) fn check_project_dir(dir: &Path) -> Vec<String> {
        let manifest = crate::manifest::validate_project_manifest(&dir.join("project.json"))
            .expect("manifest must validate");
        let name = manifest
            .get("name")
            .and_then(|v| v.get::<String>())
            .cloned()
            .unwrap_or_else(|| "test".to_string());
        let project = crate::ast::parse_project(&name, dir, &manifest).expect("project must parse");
        match check_project_collect(dir, &crate::hir::elaborate(&project)) {
            Ok(diags) => diags.into_iter().map(|d| d.rule).collect(),
            Err(()) => vec!["AUGMENTATION_FAILED".to_string()],
        }
    }

    #[test]
    fn harness_accepts_trivial_program() {
        assert!(accepts("SUB main()\nEND SUB\n"));
    }
}

#[cfg(test)]
mod checker_tests {
    use super::testutil::*;
    use std::path::Path;

    fn fixture(name: &str) -> String {
        crate::testutil::fixture_dir(name)
            .to_string_lossy()
            .into_owned()
    }

    // ---- check_function -----------------------------------------------------

    // TYPE_ISOLATED_NOT_VISIBLE moved to `ir::verify` (plan-107-A); its twin is
    // `verify::tests::rejects_private_isolated_func`.

    // NOTE: TYPE_SUB_CANNOT_RETURN_VALUE is unreachable from source — the parser
    // only reads a return type for a FUNC, so a `SUB … AS T` never parses. The
    // branch is defensive for IR/package-decoded functions and stays uncovered.

    // TYPE_RESULT_NOT_USER_VISIBLE is the resolver's; the copies this checker
    // carried were unreachable on the build path and were deleted (plan-107-B).
    // The goldens `tests/syntax/types/result-not-user-visible-invalid` and
    // `tests/syntax/functions/func_typesystem_result_invalid` pin the resolver.

    #[test]
    fn func_nothing_may_fall_through() {
        // A FUNC AS Nothing needs no explicit RETURN on every path.
        assert!(accepts(
            "FUNC f AS Nothing\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn default_value_and_defaults_accept() {
        assert!(accepts(
            "FUNC g(a AS Integer, b AS Integer = 2) AS Integer\n  RETURN a + b\nEND FUNC\nFUNC main AS Integer\n  RETURN g(1)\nEND FUNC\n"
        ));
    }

    #[test]
    fn trap_valid() {
        assert!(accepts(
            "IMPORT fs\nFUNC f AS Integer\n  LET x = fs::readText(\"a\")\n  RETURN len(x)\n  TRAP(err)\n    RETURN 0\n  END TRAP\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n"
        ));
    }

    // Both TYPE_TRAP_FALLTHROUGH forms moved to `ir::verify` (plan-107-B); the
    // twins are `verify::tests::rejects_trap_fallthrough` (handler form) and
    // `rejects_normal_flow_reaching_the_trap` (body form).

    #[test]
    fn value_func_falls_through_walk() {
        // A value-producing FUNC that does not return on every path walks the
        // final flow check (rejection relocated to ir::verify).
        let _ = check_src(
            "IMPORT io\nFUNC f AS Integer\n  io::print(\"x\")\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
        );
    }

    #[test]
    fn untyped_param_walk() {
        // A parameter with no declared type walks the `param.type_name.is_none()`
        // branch of check_function.
        let _ = check_src(
            "FUNC g(a) AS Integer\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
        );
    }

    // ---- check_type_decl / bindings ----------------------------------------

    #[test]
    fn record_type_decl_accepts() {
        assert!(accepts(
            "TYPE Point\n  x AS Integer\n  y AS Integer\nEND TYPE\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn enum_decl_accepts() {
        assert!(accepts(
            "ENUM Color\n  Red\n  Green\n  Blue\nEND ENUM\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn union_decl_accepts() {
        assert!(accepts(
            "TYPE A\n  x AS Integer\nEND TYPE\nTYPE B\n  y AS Integer\nEND TYPE\nUNION AB\n  A\n  B\nEND UNION\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn union_with_includes_accepts() {
        // Exercises expanded_union_variants; the member-conflict rule itself
        // is enforced by ir::verify (plan-20).
        assert!(accepts(
            "TYPE A\n  x AS Integer\nEND TYPE\nTYPE B\n  y AS Integer\nEND TYPE\nUNION Inner\n  A\nEND UNION\nUNION Outer INCLUDES Inner\n  B\nEND UNION\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn top_level_binding_accepts() {
        assert!(accepts(
            "LET PI AS Float = 3.14\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn top_level_binding_inferred() {
        assert!(accepts(
            "LET N = 42\nFUNC main AS Integer\n  RETURN N\nEND FUNC\n"
        ));
    }

    #[test]
    fn default_value_unknown_type_rejected() {
        // A default expression whose type cannot be inferred.
        assert!(rejects_with(
            "FUNC g(a AS Integer = mystery::thing()) AS Integer\n  RETURN a\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
            "TYPE_UNKNOWN_VALUE"
        ));
    }

    #[test]
    fn default_type_mismatch_walk() {
        // A default whose inferred type mismatches the declared param type walks
        // the expression_compatible false arm (rejection is relocated).
        let _ = check_src(
            "FUNC g(a AS String = 42) AS Integer\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
        );
    }

    #[test]
    fn resource_field_in_record_walk() {
        // A record whose field carries a resource pointer walks the is_resource
        // branch inside check_type_decl.
        let _ = check_src(
            "IMPORT fs\nTYPE Holder\n  fs AS List OF RES fs::File\nEND TYPE\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
        );
    }

    #[test]
    fn mixed_resource_union_walk() {
        // A union with one resource variant and one data variant walks the
        // mixed-union arm of check_type_decl.
        let _ = check_src(
            "IMPORT fs\nTYPE B\n  n AS Integer\nEND TYPE\nUNION Mixed\n  fs::File\n  B\nEND UNION\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
        );
    }

    #[test]
    fn record_bare_resource_field_walk() {
        // A record with a bare resource-typed field walks the is_resource_type
        // branch of the Type arm in check_type_decl.
        let _ = check_src(
            "IMPORT fs\nTYPE Holder\n  file AS fs::File\nEND TYPE\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
        );
    }

    #[test]
    fn union_variant_not_a_type_walk() {
        // A union including an enum variant walks the variant-kind check.
        let _ = check_src(
            "ENUM E\n  X\nEND ENUM\nTYPE T\n  a AS Integer\nEND TYPE\nUNION U\n  T\n  E\nEND UNION\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
        );
    }

    #[test]
    fn empty_enum_walk() {
        // An enum with no members walks the empty-enum stub arm.
        let _ = check_src("ENUM Empty\nEND ENUM\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n");
    }

    #[test]
    fn union_include_nonunion_walk() {
        // UNION INCLUDES a non-union type walks the include-kind check.
        let _ = check_src(
            "TYPE Thing\n  value AS Integer\nEND TYPE\nUNION Bad INCLUDES Thing\n  Thing\nEND UNION\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
        );
    }

    #[test]
    fn union_include_variant_conflict_walk() {
        // A variant declared directly that is also brought in via INCLUDES; the
        // conflict is reported by ir::verify (plan-20), not here.
        let _ = check_src(
            "TYPE A\n  x AS Integer\nEND TYPE\nUNION Inner\n  A\nEND UNION\nUNION Outer INCLUDES Inner\n  A\nEND UNION\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
        );
    }

    #[test]
    fn union_two_includes_share_variant_walk() {
        // A union that INCLUDES two unions sharing a variant walks the
        // included_members insert-collision arm.
        let _ = check_src(
            "TYPE A\n  x AS Integer\nEND TYPE\nUNION One\n  A\nEND UNION\nUNION Two\n  A\nEND UNION\nUNION Both INCLUDES One, Two\nEND UNION\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
        );
    }

    #[test]
    fn error_typed_thread_message_formats_type_name() {
        // A worker whose message type is `Error` forces type_name over the
        // Error/ErrorLoc scalar arms during thread-type formatting.
        let src = "IMPORT thread\nEXPORT ISOLATED FUNC worker(t AS ThreadWorker OF Error TO Integer, seed AS Error) AS Integer\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    // ---- type references ----------------------------------------------------

    // `map_resource_key_rejected` moved with TYPE_COLLECTION_OWNERSHIP_VIOLATION to
    // `ir::verify` (plan-107-B; the map-key twins pre-date it there).

    #[test]
    fn thread_resource_message_rejected() {
        // A resource in the message (data) plane of a Thread type: the
        // rejection (TYPE_THREAD_NOT_SENDABLE) is `ir::verify`'s (plan-107-A;
        // twin `verify::tests::rejects_a_resource_in_the_message_plane`); the
        // type-reference walk over the planes still runs here.
        let src = "IMPORT thread\nIMPORT fs\nFUNC main AS Integer\n  LET t AS Thread OF fs::File TO Integer\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    // TYPE_COLLECTION_OWNERSHIP_VIOLATION moved to `ir::verify` (plan-107-B); its
    // twins are `verify::tests::rejects_a_thread_handle_as_a_list_element`,
    // `rejects_a_thread_handle_as_a_map_value`, `rejects_a_resource_in_a_set_literal`,
    // `rejects_a_thread_carrying_union_as_a_map_key` and the pre-existing map-key
    // twins.

    #[test]
    fn res_return_and_param_with_state_walk() {
        // A RES return producer and a RES parameter with a STATE type walk
        // check_resource_declaration on both positions.
        let src = "IMPORT fs\nFUNC use(RES f AS fs::File) AS Integer\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  RES f AS fs::File = fs::openFile(\"x\")\n  RETURN use(f)\nEND FUNC\n";
        let _ = check_src(src);
    }

    // ---- user function overload resolution ---------------------------------

    #[test]
    fn overloaded_func_by_arity() {
        assert!(accepts(
            "FUNC f(a AS Integer) AS Integer\n  RETURN a\nEND FUNC\nFUNC f(a AS Integer, b AS Integer) AS Integer\n  RETURN a + b\nEND FUNC\nFUNC main AS Integer\n  RETURN f(1) + f(1, 2)\nEND FUNC\n"
        ));
    }

    #[test]
    fn sub_call_statement() {
        assert!(accepts(
            "IMPORT io\nSUB greet(name AS String)\n  io::print(name)\nEND SUB\nFUNC main AS Integer\n  greet(\"hi\")\n  RETURN 0\nEND FUNC\n"
        ));
    }

    // ---- LINK / native ABI --------------------------------------------------

    fn link_wrap(body: &str) -> String {
        format!("EXPORT RESOURCE Db CLOSE BY demoLink::close\nLINK \"demo\" AS demoLink\n  FUNC close(RES db AS Db) AS Nothing\n    SYMBOL \"demo_close\"\n    ABI (db CPtr) AS status CInt32\n    SUCCESS_ON status = 0\n  END FUNC\n{body}END LINK\n")
    }

    #[test]
    fn link_valid() {
        assert!(accepts(&link_wrap("")));
    }

    #[test]
    fn link_cptr_escape_param() {
        assert!(rejects_with(
            &link_wrap("  FUNC leak(handle AS CPtr) AS Nothing\n    SYMBOL \"demo_leak\"\n    ABI (handle CPtr) AS status CInt32\n    SUCCESS_ON status = 0\n  END FUNC\n"),
            "NATIVE_CPTR_ESCAPE"
        ));
    }

    #[test]
    fn link_cptr_escape_return() {
        assert!(rejects_with(
            &link_wrap("  FUNC leak() AS CPtr\n    SYMBOL \"demo_leak\"\n    ABI () AS produced CPtr\n  END FUNC\n"),
            "NATIVE_CPTR_ESCAPE"
        ));
    }

    #[test]
    fn link_unbound_slot() {
        assert!(rejects_with(
            &link_wrap("  FUNC opn(RES db AS Db) AS Nothing\n    SYMBOL \"demo_open\"\n    ABI (db CPtr, mystery CInt32) AS status CInt32\n    SUCCESS_ON status = 0\n  END FUNC\n"),
            "NATIVE_ABI_UNBOUND_SLOT"
        ));
    }

    #[test]
    fn link_free_invalid() {
        assert!(rejects_with(
            &link_wrap("  FUNC describe(RES db AS Db) AS String\n    SYMBOL \"demo_describe\"\n    ABI (db CPtr) AS produced CPtr\n    FREE produced\n      SYMBOL \"demo_free\"\n      ABI (ptr CInt32) AS CVoid\n    END FREE\n  END FUNC\n"),
            "NATIVE_FREE_INVALID"
        ));
    }

    #[test]
    fn link_const_pins_valid() {
        // A CONST pin satisfying an input slot + an OUT return producer.
        assert!(accepts(&link_wrap(
            "  FUNC exec(RES db AS Db, statement AS String) AS Nothing\n    SYMBOL \"demo_exec\"\n    ABI (db CPtr, statement CString, cb CPtr) AS status CInt32\n    CONST cb = NOTHING\n    SUCCESS_ON status = 0\n  END FUNC\n"
        )));
    }

    #[test]
    fn link_out_return_producer_valid() {
        // A resource producer: an OUT slot holds the handle, and `RETURN` names it.
        assert!(accepts(&link_wrap(
            "  FUNC opn(statement AS String) AS RES Db\n    SYMBOL \"demo_open\"\n    ABI (statement CString, produced OUT CPtr) AS status CInt32\n    RETURN produced\n    SUCCESS_ON status = 0\n  END FUNC\n"
        )));
    }

    #[test]
    fn link_const_on_out_rejected() {
        assert!(rejects_with(
            &link_wrap("  FUNC opn(statement AS String) AS RES Db\n    SYMBOL \"demo_open\"\n    ABI (statement CString, slot OUT CPtr) AS status CInt32\n    CONST slot = -1\n    SUCCESS_ON status = 0\n  END FUNC\n"),
            "NATIVE_CONST_OUT"
        ));
    }

    #[test]
    fn link_const_unknown_slot_rejected() {
        assert!(rejects_with(
            &link_wrap("  FUNC exec(RES db AS Db) AS Nothing\n    SYMBOL \"demo_exec\"\n    ABI (db CPtr) AS status CInt32\n    CONST ghost = -1\n    SUCCESS_ON status = 0\n  END FUNC\n"),
            "NATIVE_CONST_UNKNOWN_SLOT"
        ));
    }

    #[test]
    fn link_unbound_param_rejected() {
        // A wrapper param with no matching ABI slot.
        assert!(rejects_with(
            &link_wrap("  FUNC exec(RES db AS Db, extra AS Integer) AS Nothing\n    SYMBOL \"demo_exec\"\n    ABI (db CPtr) AS status CInt32\n    SUCCESS_ON status = 0\n  END FUNC\n"),
            "NATIVE_ABI_UNBOUND_PARAM"
        ));
    }

    #[test]
    fn link_no_result_rejected() {
        // A value-returning wrapper with no result marker.
        assert!(rejects_with(
            &link_wrap("  FUNC size(RES db AS Db) AS Integer\n    SYMBOL \"demo_size\"\n    ABI (db CPtr) AS status CInt32\n    SUCCESS_ON status = 0\n  END FUNC\n"),
            "NATIVE_ABI_NO_RESULT"
        ));
    }

    #[test]
    fn link_full_native_binding_with_alias_valid() {
        // A complete native binding: two resources, a LINK block, and a
        // re-exported close op. Walks collect_close_op_aliases,
        // collect_native_resources, collect_native_functions.
        assert!(check_project_dir(Path::new(&fixture("native-resource-link-valid"))).is_empty());
    }

    #[test]
    fn link_return_on_a_nothing_wrapper_rejected() {
        // plan-50-H: a `Nothing` wrapper surfaces no value, so RETURN names nothing.
        assert!(rejects_with(
            &link_wrap("  FUNC opn(statement AS String) AS Nothing\n    SYMBOL \"demo_open\"\n    ABI (statement CString) AS status CInt32\n    RETURN status\n  END FUNC\n"),
            "NATIVE_ABI_RESULT_MARKER"
        ));
    }

    // A slot named `return` is now a PARSE error, so it cannot be exercised
    // through this checker (which requires its source to parse). It is covered
    // end-to-end by tests/syntax/native/native-abi-return-slot-invalid.

    #[test]
    fn link_value_wrapper_without_return_rejected() {
        // plan-50-H: an OUT slot no longer needs to be named `return`, but a
        // value-returning wrapper must still name its result.
        assert!(rejects_with(
            &link_wrap("  FUNC opn(statement AS String) AS RES Db\n    SYMBOL \"demo_open\"\n    ABI (statement CString, extra OUT CPtr) AS status CInt32\n    SUCCESS_ON status = 0\n  END FUNC\n"),
            "NATIVE_ABI_NO_RESULT"
        ));
    }

    #[test]
    fn link_free_wrong_return_ctype_rejected() {
        // FREE on a non-CPtr `return` produced slot is malformed.
        assert!(rejects_with(
            &link_wrap("  FUNC describe(RES db AS Db) AS Integer\n    SYMBOL \"demo_describe\"\n    ABI (db CPtr) AS produced CInt32\n    FREE produced\n      SYMBOL \"demo_free\"\n      ABI (ptr CPtr) AS CVoid\n    END FREE\n  END FUNC\n"),
            "NATIVE_FREE_INVALID"
        ));
    }

    #[test]
    fn link_free_empty_symbol_rejected() {
        // A FREE block with an empty deallocator symbol is malformed (the symbol
        // check arm of the FREE validation).
        assert!(rejects_with(
            &link_wrap("  FUNC describe(RES db AS Db) AS String\n    SYMBOL \"demo_describe\"\n    ABI (db CPtr) AS produced CPtr\n    FREE produced\n      SYMBOL \"\"\n      ABI (ptr CPtr) AS CVoid\n    END FREE\n  END FUNC\n"),
            "NATIVE_FREE_INVALID"
        ));
    }

    // ---- overloaded user call with named arguments (call_shape_matches_sig) --

    #[test]
    fn overloaded_call_named_arguments() {
        // Two overloads distinguished by arity, called with a named argument, so
        // call_shape_matches_sig walks its named-argument validation.
        assert!(accepts(
            "FUNC f(a AS Integer) AS Integer\n  RETURN a\nEND FUNC\nFUNC f(a AS Integer, b AS Integer) AS Integer\n  RETURN a + b\nEND FUNC\nFUNC main AS Integer\n  RETURN f(a := 1, b := 2)\nEND FUNC\n"
        ));
    }

    #[test]
    fn overloaded_call_duplicate_named_argument() {
        // A duplicate named argument on an overloaded call fails the shape match.
        let _ = check_src(
            "FUNC f(a AS Integer) AS Integer\n  RETURN a\nEND FUNC\nFUNC f(a AS Integer, b AS Integer) AS Integer\n  RETURN a + b\nEND FUNC\nFUNC main AS Integer\n  RETURN f(a := 1, a := 2)\nEND FUNC\n",
        );
    }

    // ---- DOC block item -----------------------------------------------------

    #[test]
    fn doc_block_item_walk() {
        // A DOC block is a top-level item the checker skips (the Doc arm of check).
        let _ = check_src(
            "DOC\n  PACKAGE\n  DESC A test program.\nEND DOC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
        );
    }

    // ---- package metadata via .mfp fixtures --------------------------------

    #[test]
    fn package_metadata_thread_transfer_valid() {
        assert!(check_project_dir(Path::new(&fixture("func_thread_transfer_valid"))).is_empty());
    }

    #[test]
    fn package_metadata_thread_send_valid() {
        assert!(check_project_dir(Path::new(&fixture("func_thread_send_valid"))).is_empty());
    }

    // Diverse imported-package metadata shapes walk validate_package_metadata_type
    // over List / Map / Union return types and collect_package_* installers.
    #[test]
    fn package_metadata_diverse_shapes_valid() {
        for d in [
            "project-with-package-import-as",
            "thread-return-union",
            "thread-return-map-of-string-to-string",
            "thread-return-list-of-string",
            "package-import-as",
            "func_thread_start_valid",
            "thread-drop-cleanup",
            "native-resource-import-valid",
            "thread-import-package-print",
            "thread-import-pkg-receive-rt",
            "thread-strings-split-return",
        ] {
            let path = fixture(d);
            assert!(
                check_project_dir(Path::new(&path)).is_empty(),
                "{d} should accept"
            );
        }
    }

    // Package projects whose imported metadata exercises the resource/comparable
    // validators (they resolve without panicking; some yield diagnostics that
    // depend on monomorphization, so we only assert the checker runs).
    #[test]
    fn package_metadata_validation_walks() {
        for d in [
            "native-link-import-sqlite-rt",
            "package-comparable-import-invalid",
        ] {
            let path = fixture(d);
            let _ = check_project_dir(Path::new(&path));
        }
    }

    // A corrupt `.mfp` on an imported package drives the PACKAGE_INVALID error
    // paths in collect_package_types / collect_package_resources /
    // collect_package_functions.
    #[test]
    fn corrupt_package_metadata_rejected() {
        use crate::ast::{parse_source, AstProject};
        use std::fs;

        let dir = std::env::temp_dir().join(format!("mfb_sc_pkg_{}", std::process::id()));
        let pkgs = dir.join("packages");
        fs::create_dir_all(&pkgs).unwrap();
        fs::write(pkgs.join("brokenpkg.mfp"), b"not a valid mfp container").unwrap();

        let file = parse_source(
            Path::new("main.mfb"),
            "main.mfb",
            "IMPORT brokenpkg\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
        )
        .unwrap();
        let project = crate::hir::elaborate(&AstProject {
            name: "t".into(),
            files: vec![file],
        });
        let diags = super::check_project_collect(&dir, &project).unwrap();
        let _ = fs::remove_dir_all(&dir);
        assert!(
            diags.iter().any(|d| d.rule == "PACKAGE_INVALID"),
            "expected PACKAGE_INVALID, got {:?}",
            diags.iter().map(|d| &d.rule).collect::<Vec<_>>()
        );
    }

    // The imported-package metadata validators (`validate_package_metadata_type`,
    // `validate_imported_package_type`, `collect_package_resources`,
    // `install_package_type_info`, `package_field_info`) are driven directly with
    // synthetic `Type`/`BinaryRepr*` values. Building the equivalent `.mfp`
    // containers on disk for the Map / Function / Thread-state / unknown-type /
    // Enum / Public+Export-field arms is impractical, so we call the `pub(super)`
    // methods on a freshly constructed checker instead. These arms are reachable
    // in production from a decoded package whose metadata carries these shapes.
    #[test]
    fn package_metadata_validator_arms_direct() {
        use super::{
            BinaryReprExportKind, BinaryReprTypeExport, BinaryReprTypeField,
            BinaryReprTypeVisibility, SyntaxChecker, Type, TypeDeclKind, TypeInfo, Visibility,
        };
        use crate::ast::{parse_source, AstProject};
        use std::collections::HashSet;
        use std::path::PathBuf;

        let file = parse_source(
            Path::new("main.mfb"),
            "main.mfb",
            "FUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
        )
        .unwrap();
        let project = crate::hir::elaborate(&AstProject {
            name: "t".into(),
            files: vec![file],
        });
        let dir = Path::new(".");
        let mut checker = SyntaxChecker::new(dir, &project);
        let file_ref = &project.files[0];
        let pkg = PathBuf::from("packages/fake.mfp");

        // Map with a non-comparable (List) key AND a `Res` value: exercises the
        // Map arm's key/value recursion, the `is_comparable` rejection, and the
        // `List | Set | Result | Res` element-recursion arm.
        let mut seen = HashSet::new();
        let map_ty = Type::MapOf(
            Box::new(Type::ListOf(Box::new(Type::Integer))),
            Box::new(Type::Res(Box::new(Type::String))),
        );
        checker.validate_package_metadata_type(file_ref, 1, &pkg, &map_ty, "ctx", &mut seen);

        // The remaining single-element wrappers (`Set`, `Result`) share the same
        // recursion arm as `List`/`Res` but are distinct pattern alternatives.
        for element_ty in [
            Type::SetOf(Box::new(Type::Integer)),
            Type::ResultOf(Box::new(Type::String)),
        ] {
            checker.validate_package_metadata_type(
                file_ref,
                1,
                &pkg,
                &element_ty,
                "ctx",
                &mut seen,
            );
        }

        // Function type: parameter list + return-type recursion.
        let fn_ty = Type::Func(
            vec![Type::Integer, Type::String],
            Box::new(Type::Boolean),
            false,
        );
        checker.validate_package_metadata_type(file_ref, 1, &pkg, &fn_ty, "ctx", &mut seen);

        // Thread carrying resource + resource STATE + output: both branches of
        // the plane's `split_state`, plus the message/output recursion.
        // plan-106-C rung 2e: the STATE rides inside the plane's spelling.
        let thread_ty = Type::ThreadHandle {
            worker: false,
            msg: Box::new(Type::Integer),
            res: Box::new(Type::String.with_state(&Type::Boolean)),
            out: Box::new(Type::Float),
        };
        checker.validate_package_metadata_type(file_ref, 1, &pkg, &thread_ty, "ctx", &mut seen);

        // ThreadWorker with no resource plane: the same arm, `Nothing` branch.
        let worker_ty = Type::ThreadHandle {
            worker: true,
            msg: Box::new(Type::Integer),
            res: Box::new(Type::Nothing),
            out: Box::new(Type::Nothing),
        };
        checker.validate_package_metadata_type(file_ref, 1, &pkg, &worker_ty, "ctx", &mut seen);

        // A `User` type not present in `type_infos` and not a resource: the
        // "references unknown type" report.
        checker.validate_package_metadata_type(
            file_ref,
            1,
            &pkg,
            &Type::named("Nope"),
            "ctx",
            &mut seen,
        );

        // A `User` type that resolves to an Enum: the empty `Enum` arm of the
        // known-type match.
        checker.type_infos.insert(
            "MyEnum".into(),
            TypeInfo {
                kind: TypeDeclKind::Enum,
                visibility: Visibility::Export,
                file_path: String::new(),
                fields: Vec::new(),
                variants: Vec::new(),
                members: HashSet::new(),
            },
        );
        let mut seen2 = HashSet::new();
        checker.validate_package_metadata_type(
            file_ref,
            1,
            &pkg,
            &Type::named("MyEnum"),
            "ctx",
            &mut seen2,
        );

        // `validate_imported_package_type`: the Enum and Func/Sub export kinds are
        // no-ops (their type is not metadata-validated).
        for kind in [
            BinaryReprExportKind::Enum,
            BinaryReprExportKind::Func,
            BinaryReprExportKind::Sub,
        ] {
            let export = BinaryReprTypeExport {
                name: "E".into(),
                kind,
                fields: Vec::new(),
                variants: Vec::new(),
                members: Vec::new(),
                foreign_owner: None,
            };
            checker.validate_imported_package_type(file_ref, 1, &pkg, &export);
        }

        // `install_package_type_info`: the Enum kind maps to `TypeDeclKind::Enum`;
        // a Func/Sub "type export" is a defensive `return`.
        let enum_export = BinaryReprTypeExport {
            name: "InstalledEnum".into(),
            kind: BinaryReprExportKind::Enum,
            fields: Vec::new(),
            variants: Vec::new(),
            members: vec!["A".into(), "B".into()],
            foreign_owner: None,
        };
        checker.install_package_type_info(&pkg, enum_export);
        assert_eq!(
            checker.user_type_kinds.get("InstalledEnum"),
            Some(&TypeDeclKind::Enum)
        );
        let func_export = BinaryReprTypeExport {
            name: "NotAType".into(),
            kind: BinaryReprExportKind::Func,
            fields: Vec::new(),
            variants: Vec::new(),
            members: Vec::new(),
            foreign_owner: None,
        };
        checker.install_package_type_info(&pkg, func_export);

        // `package_field_info`: every field-visibility mapping.
        for vis in [
            BinaryReprTypeVisibility::Private,
            BinaryReprTypeVisibility::Public,
            BinaryReprTypeVisibility::Export,
        ] {
            let info = checker.package_field_info(BinaryReprTypeField {
                name: "x".into(),
                type_: "Integer".into(),
                visibility: vis,
            });
            assert_eq!(info.name, "x");
        }

        // `collect_package_resources` over an unreadable/absent `.mfp`: the
        // read-error report + early return.
        checker.collect_package_resources(file_ref, "bind", 1, &pkg);

        // Two `PACKAGE_INVALID`s at minimum: the non-comparable map key and the
        // unknown `User` type (plus the unreadable resource table).
        assert!(
            checker
                .diagnostics
                .iter()
                .filter(|d| d.rule == "PACKAGE_INVALID")
                .count()
                >= 3
        );
    }

    // An import of a non-builtin package with no `.mfp` on disk drives the
    // `!package_file.is_file()` early-continue in both `collect_package_types`
    // and `collect_package_functions`.
    #[test]
    fn imported_package_without_mfp_file_skipped() {
        use super::SyntaxChecker;
        use crate::ast::{parse_source, AstProject};

        let file = parse_source(
            Path::new("main.mfb"),
            "main.mfb",
            "IMPORT nonexistent_pkg\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
        )
        .unwrap();
        let project = crate::hir::elaborate(&AstProject {
            name: "t".into(),
            files: vec![file],
        });
        // A directory with no `packages/nonexistent_pkg.mfp`: the collectors take
        // the missing-file continue and register nothing for the import.
        let dir = std::env::temp_dir().join(format!("mfb_sc_nomfp_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let checker = SyntaxChecker::new(&dir, &project);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(!checker.functions.contains_key("nonexistent_pkg.anything"));
    }

    // Exercises the standalone `check_project` render wrapper (accept path).
    #[test]
    fn check_project_wrapper_accepts() {
        use crate::ast::{parse_source, AstProject};
        let file = parse_source(
            Path::new("main.mfb"),
            "main.mfb",
            "FUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
        )
        .unwrap();
        let project = crate::hir::elaborate(&AstProject {
            name: "t".into(),
            files: vec![file],
        });
        assert!(super::check_project(Path::new("."), &project).is_ok());
    }

    // Exercises the standalone `check_project` render wrapper (reject path).
    #[test]
    fn check_project_wrapper_rejects() {
        use crate::ast::{parse_source, AstProject};
        // `EXIT FUNC` is a syntaxcheck-owned rejection (EXIT_FUNC_FORBIDDEN), so
        // the wrapper's `Err` comes from this checker rather than a rule that has
        // since moved out of it.
        let file = parse_source(
            Path::new("main.mfb"),
            "main.mfb",
            "FUNC main AS Integer\n  EXIT FUNC\n  RETURN 0\nEND FUNC\n",
        )
        .unwrap();
        let project = crate::hir::elaborate(&AstProject {
            name: "t".into(),
            files: vec![file],
        });
        assert!(super::check_project(Path::new("."), &project).is_err());
    }

    // ---- return-type overload disambiguation (lookup_visible_call_sig) -----

    #[test]
    fn return_type_overload_disambiguated_by_expected() {
        // Two same-arity overloads differing only by return type; the binding's
        // declared type selects one (walks the expected-type disambiguation arm).
        assert!(accepts(
            "FUNC encode(v AS String) AS List OF Byte\n  RETURN [toByte(65)]\nEND FUNC\nFUNC encode(v AS String) AS List OF Integer\n  RETURN [1]\nEND FUNC\nFUNC main AS Integer\n  LET a AS List OF Byte = encode(\"x\")\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn return_type_overload_no_expected_falls_back() {
        // The same overloads called with no contextual type fall back to the last
        // candidate (the else path of the disambiguation).
        let _ = check_src(
            "IMPORT io\nFUNC encode(v AS String) AS List OF Byte\n  RETURN [toByte(65)]\nEND FUNC\nFUNC encode(v AS String) AS List OF Integer\n  RETURN [1]\nEND FUNC\nFUNC main AS Integer\n  LET a = encode(\"x\")\n  RETURN 0\nEND FUNC\n",
        );
    }

    // ---- user-function named arguments (normalize_named_arguments) ---------

    #[test]
    fn user_named_argument_valid() {
        assert!(accepts(
            "FUNC g(a AS Integer, b AS Integer) AS Integer\n  RETURN a + b\nEND FUNC\nFUNC main AS Integer\n  RETURN g(a := 1, b := 2)\nEND FUNC\n"
        ));
    }

    #[test]
    fn user_named_argument_out_of_order_valid() {
        assert!(accepts(
            "FUNC g(a AS Integer, b AS Integer) AS Integer\n  RETURN a + b\nEND FUNC\nFUNC main AS Integer\n  RETURN g(b := 2, a := 1)\nEND FUNC\n"
        ));
    }

    #[test]
    fn user_named_argument_unknown_name() {
        assert!(rejects_with(
            "FUNC g(a AS Integer) AS Integer\n  RETURN a\nEND FUNC\nFUNC main AS Integer\n  RETURN g(z := 1)\nEND FUNC\n",
            "TYPE_UNKNOWN_ARGUMENT_NAME"
        ));
    }

    #[test]
    fn user_named_argument_duplicate() {
        assert!(rejects_with(
            "FUNC g(a AS Integer) AS Integer\n  RETURN a\nEND FUNC\nFUNC main AS Integer\n  RETURN g(1, a := 2)\nEND FUNC\n",
            "TYPE_DUPLICATE_ARGUMENT_NAME"
        ));
    }

    #[test]
    fn user_named_argument_arity() {
        assert!(rejects_with(
            "FUNC g(a AS Integer) AS Integer\n  RETURN a\nEND FUNC\nFUNC main AS Integer\n  RETURN g(1, 2, 3)\nEND FUNC\n",
            "TYPE_CALL_ARITY_MISMATCH"
        ));
    }

    #[test]
    fn user_default_with_named_trailing_omission() {
        // A defaulted trailing param omitted while a named earlier one is set.
        assert!(accepts(
            "FUNC g(a AS Integer, b AS Integer = 9) AS Integer\n  RETURN a + b\nEND FUNC\nFUNC main AS Integer\n  RETURN g(a := 1)\nEND FUNC\n"
        ));
    }

    #[test]
    fn user_named_argument_internal_gap() {
        // A required middle parameter omitted while a later one is named leaves an
        // internal gap (has_internal_gap / missing_required arity error).
        assert!(rejects_with(
            "FUNC g(a AS Integer, b AS Integer, c AS Integer) AS Integer\n  RETURN a + b + c\nEND FUNC\nFUNC main AS Integer\n  RETURN g(a := 1, c := 3)\nEND FUNC\n",
            "TYPE_CALL_ARITY_MISMATCH"
        ));
    }

    #[test]
    fn user_named_positional_after_named_walk() {
        // A positional argument following a named one that fills a later slot
        // walks the slot-skipping loop of normalize_named_arguments.
        assert!(accepts(
            "FUNC g(a AS Integer, b AS Integer) AS Integer\n  RETURN a + b\nEND FUNC\nFUNC main AS Integer\n  RETURN g(b := 2, 1)\nEND FUNC\n"
        ));
    }

    // ---- builtin named-argument internal-gap (normalize_builtin_call_arguments) --

    #[test]
    fn builtin_named_argument_internal_gap() {
        // http::write has params (url, body, headers, method); supplying `method`
        // by name while omitting the required `body` leaves an internal gap.
        let src = "IMPORT http\nIMPORT net\nFUNC main AS Integer\n  LET u AS net::Url = net::toUrl(\"http://x/\")\n  LET r = http::write(u, method := \"GET\")\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    // ---- export_in_executable_diagnostics (build-pipeline entry point) ------

    #[test]
    fn export_in_executable_flags_each_item_kind() {
        use crate::ast::{parse_source, AstProject};
        use std::path::Path;
        // Cover every item kind the visibility match walks: binding, type,
        // function, resource (HirItem::Resource arm), and func alias (HirItem::FuncAlias
        // arm). The resource/alias targets need only parse — this entry point runs
        // before import resolution.
        let src = "EXPORT LET g AS Integer = 5\nEXPORT TYPE Rec\n  x AS Integer\nEND TYPE\nEXPORT FUNC f() AS Integer\n  RETURN 1\nEND FUNC\nEXPORT RESOURCE Db CLOSE BY x::close\nEXPORT FUNC ff AS x::gg\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n";
        let file = parse_source(Path::new("main.mfb"), "main.mfb", src).expect("parse");
        let project = AstProject {
            name: "t".to_string(),
            files: vec![file],
        };
        let diags = crate::syntaxcheck::export_in_executable_diagnostics(false, &project);
        assert!(diags.iter().all(|d| d.rule == "EXPORT_IN_EXECUTABLE"));
        assert!(diags.len() >= 5, "expected an EXPORT diagnostic per item");
    }

    #[test]
    fn export_in_executable_empty_for_package_project() {
        use crate::ast::{parse_source, AstProject};
        use std::path::Path;
        let src = "EXPORT FUNC f() AS Integer\n  RETURN 1\nEND FUNC\n";
        let file = parse_source(Path::new("main.mfb"), "main.mfb", src).expect("parse");
        let project = AstProject {
            name: "t".to_string(),
            files: vec![file],
        };
        // A package project never flags EXPORT (that is its purpose).
        assert!(crate::syntaxcheck::export_in_executable_diagnostics(true, &project).is_empty());
    }

    #[test]
    fn export_in_executable_no_export_no_diagnostic() {
        use crate::ast::{parse_source, AstProject};
        use std::path::Path;
        let src = "FUNC f() AS Integer\n  RETURN 1\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n";
        let file = parse_source(Path::new("main.mfb"), "main.mfb", src).expect("parse");
        let project = AstProject {
            name: "t".to_string(),
            files: vec![file],
        };
        assert!(crate::syntaxcheck::export_in_executable_diagnostics(false, &project).is_empty());
    }

    // ---- record-field cycle detection ---------------------------------------

    #[test]
    fn return_type_overload_by_expected_binding() {
        // Two overloads differ only by return type; the expected (contextual)
        // type of the binding selects the Integer one uniquely.
        let src = "FUNC pick() AS Integer\n  RETURN 1\nEND FUNC\nFUNC pick() AS String\n  RETURN \"a\"\nEND FUNC\nFUNC main AS Integer\n  LET x AS Integer = pick()\n  RETURN x\nEND FUNC\n";
        assert!(accepts(src));
    }

    #[test]
    fn testing_and_doc_items_are_walked() {
        // A top-level TESTING block and a DOC block are both no-op arms in the
        // checker's item walk.
        let src = "DOC\n  PACKAGE\n  DESC A program.\nEND DOC\nTESTING\n  TGROUP \"g\"\n    TCASE \"c\"\n      LET n AS Integer = 1\n    END TCASE\n  END TGROUP\nEND TESTING\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn record_field_referencing_resource_walks_arm() {
        // A record field of a resource type walks the `is_resource_type` arm.
        let src = "IMPORT fs\nTYPE Bad\n  f AS fs::File\nEND TYPE\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn self_referential_record_walks_cycle_arm() {
        let src =
            "TYPE Node\n  child AS Node\nEND TYPE\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn empty_enum_walks_arm() {
        let src = "ENUM Empty\nEND ENUM\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn union_including_non_union_and_non_type_variant_walks_arms() {
        // A union variant that is itself an ENUM (not a record Type) walks the
        // `!matches!(kind, Type)` variant arm.
        let src = "ENUM Color\n  Red, Green\nEND ENUM\nTYPE Dot\n  x AS Integer\nEND TYPE\nUNION Mix\n  Dot\n  Color\nEND UNION\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn record_field_diamond_is_not_a_cycle() {
        // Two fields reach the same leaf record `D` — the cycle walk must mark it
        // visited on the first path and skip it on the second (no false cycle).
        let src = "TYPE D\n  n AS Integer\nEND TYPE\nTYPE B\n  d AS D\nEND TYPE\nTYPE C\n  d AS D\nEND TYPE\nTYPE A\n  b AS B\n  c AS C\nEND TYPE\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n";
        assert!(accepts(src));
    }

    // ---- plan-68-F3: residual mod.rs branches -------------------------------

    #[test]
    fn private_type_construction_in_same_file_is_visible() {
        // mod.rs:1424 — visible_from's Private arm: a PRIVATE type constructed in
        // its own file resolves (file.path == owner_file_path).
        let src = "\
PRIVATE TYPE Secret
  a AS Integer
END TYPE

FUNC main AS Integer
  LET s = Secret[1]
  RETURN 0
END FUNC
";
        assert!(accepts(src), "{:?}", check_src(src));
    }

    #[test]
    fn set_of_resource_element_walks() {
        // check_type_reference's Set arm walks the element type; the ownership
        // rejection itself is `ir::verify`'s (plan-107-B).
        let src = "\
IMPORT fs

FUNC f(s AS Set OF fs::File) AS Nothing
END FUNC

FUNC main AS Integer
  RETURN 0
END FUNC
";
        let _ = check_src(src);
    }

    #[test]
    fn union_including_another_union_expands_variants() {
        // mod.rs:921-938 — a union that INCLUDES another union recurses through
        // expanded_union_variants and extends with the included union's variants;
        // a self-including union trips the visiting-set re-entry guard (902).
        let src = "\
TYPE Circle
  radius AS Integer
END TYPE

TYPE Rect
  width AS Integer
END TYPE

UNION Shape
  Circle
  Rect
END UNION

UNION Bigger INCLUDES Shape
  Circle
END UNION

UNION Cyclic INCLUDES Cyclic
  Rect
END UNION

FUNC main AS Integer
  RETURN 0
END FUNC
";
        let _ = check_src(src);
    }

    #[test]
    fn call_with_named_arg_for_unknown_parameter_walks_shape_check() {
        // mod.rs:1156-1157 — call_shape_matches_sig rejects a candidate when a
        // named argument names no parameter of the signature.
        let src = "\
FUNC helper(x AS Integer) AS Integer
  RETURN x
END FUNC

FUNC main AS Integer
  LET r AS Integer = helper(bogus := 1)
  RETURN 0
END FUNC
";
        let _ = check_src(src);
    }
}
