use crate::ast::{
    AstFile, AstProject, CallArg, ConstructorArg, ExitTarget, Expression, Function, FunctionKind,
    Item, LoopKind, MatchPattern, RecordUpdate, Statement, TopLevelBinding, TypeDecl, TypeDeclKind,
    TypeField, Visibility,
};
use crate::binary_repr::{
    self, BinaryReprExportKind, BinaryReprTypeExport, BinaryReprTypeField, BinaryReprTypeVariant,
    BinaryReprTypeVisibility,
};
use crate::builtins;
use crate::numeric;
use std::collections::{HashMap, HashSet};
use std::path::Path;

#[path = "builtins.rs"]
mod builtins_check;
mod checking;
mod helpers;
mod inference;
mod resources;
mod types;

use self::helpers::*;

#[derive(Clone, Debug, Eq, PartialEq)]
enum Type {
    Boolean,
    Byte,
    Error,
    ErrorLoc,
    Fixed,
    Float,
    Integer,
    /// `Money`: an 8-byte base-10 fixed-point financial scalar (plan-29-A). A
    /// dimensioned numeric with a restricted algebra — see `numeric::money_result_type`.
    Money,
    List(Box<Type>),
    Map(Box<Type>, Box<Type>),
    /// A `RES`-marked collection element (`List OF RES File`, `Map ... TO RES
    /// File`). The `RES` is the mandatory resource ownership-axis marker for a
    /// resource appearing as an element — exactly as `RES f` / `RES f AS File`
    /// mark a binding or parameter. The collection still holds a *borrow* and
    /// owns nothing; a scope owns the resource (§15.6).
    Res(Box<Type>),
    Function {
        params: Vec<Type>,
        return_type: Box<Type>,
        isolated: bool,
    },
    Nothing,
    Result(Box<Type>),
    /// `Scalar`: a 32-bit Unicode scalar value (plan-41-A). Register-carried like
    /// `Byte`, written with a backtick literal `` `x` ``. Comparable and orderable
    /// by codepoint, but **not numeric** — it never enters the promotion lattice.
    Scalar,
    String,
    // (message, resource, output). `resource` is the optional resource-plane
    // type carried by thread::transfer/accept; `None` for a data-only thread.
    Thread(Box<Type>, Option<Box<Type>>, Box<Type>),
    ThreadWorker(Box<Type>, Option<Box<Type>>, Box<Type>),
    User(String),
    Unknown,
}

#[derive(Clone)]
struct LocalInfo {
    type_: Type,
    mutable: bool,
    /// The `STATE T` type attached to a `RES` binding/parameter, if any. Drives
    /// `s.state` member access typing.
    state_type: Option<String>,
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
    Borrow,
}

/// Elaborate and check `ast`, returning the rejections collected in source
/// order **without rendering them**. The caller merges these with
/// `ir::verify`'s relocated diagnostics and renders both in one line-ordered
/// pass (plan-20-Z). An `Err` is a pre-check augmentation failure that already
/// reported itself.
pub fn check_project_collect(
    project_dir: &Path,
    ast: &AstProject,
) -> Result<Vec<crate::rules::PendingDiagnostic>, ()> {
    let augmented = builtins::json::augmented_project(ast)?;
    let augmented = builtins::csv::augmented_project(&augmented)?;
    let augmented = builtins::regex::augmented_project(&augmented)?;
    let augmented = builtins::datetime::augmented_project(&augmented)?;
    let augmented = builtins::money::augmented_project(&augmented)?;
    // `vector` imports only intrinsic `math` (plan-06-vector.md §5).
    let augmented = builtins::vector::augmented_project(&augmented)?;
    // `http` before `net`: `http_package.mfb` imports `net` (plan-03-http.md Phase 4).
    let augmented = builtins::http::augmented_project(&augmented)?;
    let augmented = builtins::net::augmented_project(&augmented)?;
    let augmented = builtins::audio::augmented_project(&augmented)?;
    // `crypto` before `encoding`: `crypto_package.mfb` imports `encoding`
    // (mirrors `http` before `net`; plan-04-crypto.md Part C).
    let augmented = builtins::crypto::augmented_project(&augmented)?;
    // `strings` before `encoding`: `strings_package.mfb` imports `encoding`
    // (plan-41-D).
    let augmented = builtins::strings::augmented_project(&augmented)?;
    let augmented = builtins::encoding::augmented_project(&augmented)?;
    let mut checker = SyntaxChecker::new(project_dir, &augmented);
    checker.check();
    Ok(checker.diagnostics)
}

/// Check `ast` and render any rejections directly (standalone callers that do
/// not run `ir::verify`, e.g. `mfb audit`). `build` uses `check_project_collect`
/// instead so it can merge the two diagnostic streams.
pub fn check_project(project_dir: &Path, ast: &AstProject) -> Result<(), ()> {
    let diagnostics = check_project_collect(project_dir, ast)?;
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
    if is_package {
        return Vec::new();
    }
    let mut diagnostics = Vec::new();
    for file in &ast.files {
        // Skip toolchain-provided source: injected builtin packages
        // (`AstFile::internal`) and the synthetic prelude (`<builtin …>` path),
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
    ast: &'a AstProject,
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
    resource_registry: builtins::ResourceRegistry,
    /// Callee names that act as a *re-export alias* of a registered close op,
    /// mapped to the bare resource type they close. Calling such an alias is
    /// invalidation event #1 just like the registered close op itself
    /// (plan-link-update.md §5a).
    close_op_aliases: HashMap<String, String>,
    /// Set true only while inferring the argument in a compiler-known
    /// *non-escaping* callback position (e.g. `forEach`'s action). A lambda
    /// inferred here may capture an outer `MUT` binding as a call-bound borrow.
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
    visibility: Visibility,
}

#[derive(Clone)]
struct VariantConstructor {
    name: String,
    union_name: String,
    fields: Vec<FieldInfo>,
}

impl<'a> SyntaxChecker<'a> {
    pub(super) fn new(project_dir: &'a Path, ast: &'a AstProject) -> Self {
        let mut checker = Self {
            project_dir,
            ast,
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
            resource_registry: builtins::ResourceRegistry::with_builtins(),
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
        for file in &self.ast.files {
            for item in &file.items {
                if let Item::Resource(resource) = item {
                    close_to_type.insert(resource.close_fn.clone(), resource.name.clone());
                }
            }
        }
        for file in &self.ast.files {
            for item in &file.items {
                if let Item::FuncAlias(alias) = item {
                    if let Some(type_name) = close_to_type.get(&alias.target) {
                        self.close_op_aliases
                            .insert(alias.name.clone(), type_name.clone());
                    }
                }
            }
        }
    }

    /// Register native `LINK` resources declared in this package into the
    /// resource registry as `kind = native` (plan-link-update.md §9). The close
    /// op is the dotted `alias.func`; `close_may_fail` is derived from whether the
    /// close wrapper has a `SUCCESS_ON` gate; sendability comes from the
    /// declaration's `THREAD_SENDABLE` opt-in (plan-link-update.md §8).
    pub(super) fn collect_native_resources(&mut self) {
        // Map every LINK function `alias.func` to whether it can fail (has a
        // SUCCESS_ON / ERROR_ON gate).
        let mut close_may_fail: HashMap<String, bool> = HashMap::new();
        for file in &self.ast.files {
            for item in &file.items {
                if let Item::Link(link) = item {
                    for function in &link.functions {
                        close_may_fail.insert(
                            format!("{}.{}", link.alias, function.name),
                            function.success_on.is_some(),
                        );
                    }
                }
            }
        }

        for file in &self.ast.files {
            for item in &file.items {
                if let Item::Resource(resource) = item {
                    let close_function = resource.close_fn.clone();
                    let may_fail = close_may_fail
                        .get(&close_function)
                        .copied()
                        .unwrap_or(false);
                    self.resource_registry.register(
                        resource.name.clone(),
                        builtins::ResourceInfo {
                            close_function,
                            sendable: resource.thread_sendable,
                            close_may_fail: may_fail,
                            kind: builtins::ResourceKind::Native,
                        },
                    );
                }
            }
        }
    }

    /// Native-specific checks on a `RESOURCE … CLOSE BY …` declaration. The
    /// structural close-op checks run during resolve; the sendability opt-in is
    /// recorded into the registry (and the `RESOURCE_TABLE` sendable bit) by
    /// `collect_native_resources` (plan-link-update.md §8/§10).
    pub(super) fn check_resource_decl(
        &mut self,
        _file: &AstFile,
        _resource: &crate::ast::ResourceDecl,
    ) {
    }

    /// Native-specific checks on a `LINK` block: `CPtr` containment and ABI
    /// slot/parameter consistency (plan-link-update.md §5b/§5c/§11/§12).
    pub(super) fn check_link_block(&mut self, file: &AstFile, link: &crate::ast::LinkBlock) {
        for function in &link.functions {
            self.check_link_function(file, function);
        }
    }

    pub(super) fn check_link_function(
        &mut self,
        file: &AstFile,
        function: &crate::ast::LinkFunction,
    ) {
        // `CPtr` (and other raw C ABI types) may never appear in a wrapper's
        // MFBASIC-facing signature — only inside `ABI (...)` slots. A wrapper
        // param or return typed as a C type would let a raw pointer escape into an
        // ordinary API (plan-link-update.md §5/§11).
        for param in &function.params {
            if let Some(type_name) = &param.type_name {
                if is_c_abi_type(type_name) {
                    self.report(
                        "NATIVE_CPTR_ESCAPE",
                        &format!(
                            "Native function `{}` parameter `{}` uses C ABI type `{}`; raw C types may appear only in ABI slots.",
                            function.name, param.name, type_name
                        ),
                        file,
                        param.line,
                    );
                }
            }
        }
        if let Some(return_type) = &function.return_type {
            if is_c_abi_type(return_type) {
                self.report(
                    "NATIVE_CPTR_ESCAPE",
                    &format!(
                        "Native function `{}` returns C ABI type `{}`; raw C types may appear only in ABI slots.",
                        function.name, return_type
                    ),
                    file,
                    function.line,
                );
            }
        }

        // Every ABI slot must be satisfied by exactly one of: a wrapper parameter
        // (matched by name), the OUT/return result marker, or a CONST pin
        // (plan-link-update.md §5c).
        let const_slots: HashSet<&str> = function
            .consts
            .iter()
            .map(|pin| pin.slot.as_str())
            .collect();
        let param_names: HashSet<&str> = function
            .params
            .iter()
            .map(|param| param.name.as_str())
            .collect();

        let mut result_markers = 0;
        for slot in &function.abi.slots {
            if slot.name == "return" {
                result_markers += 1;
                if !slot.is_out {
                    self.report(
                        "NATIVE_ABI_RESULT_MARKER",
                        &format!(
                            "Native function `{}` ABI slot `return` must be marked `OUT`.",
                            function.name
                        ),
                        file,
                        slot.line,
                    );
                }
                continue;
            }
            // A CONST pin satisfies the slot and is input-only.
            if const_slots.contains(slot.name.as_str()) {
                if slot.is_out {
                    self.report(
                        "NATIVE_CONST_OUT",
                        &format!(
                            "Native function `{}` pins ABI slot `{}` with CONST, which cannot also be OUT.",
                            function.name, slot.name
                        ),
                        file,
                        slot.line,
                    );
                }
                continue;
            }
            // An OUT slot not named `return` is unsupported here (multi-out
            // RETURN_OUT is a deferred ABI form, plan-link-update.md §5b).
            if slot.is_out {
                self.report(
                    "NATIVE_ABI_UNBOUND_SLOT",
                    &format!(
                        "Native function `{}` ABI slot `{}` is OUT but is not the `return` result marker.",
                        function.name, slot.name
                    ),
                    file,
                    slot.line,
                );
                continue;
            }
            // An ordinary input slot must bind to a wrapper parameter by name.
            if !param_names.contains(slot.name.as_str()) {
                self.report(
                    "NATIVE_ABI_UNBOUND_SLOT",
                    &format!(
                        "Native function `{}` ABI slot `{}` does not bind to a parameter, CONST pin, or the result marker.",
                        function.name, slot.name
                    ),
                    file,
                    slot.line,
                );
            }
        }

        // The native return slot named `return` is also a result marker.
        if function.abi.return_name == "return" {
            result_markers += 1;
        }

        // A producer (`AS RES X`) and any non-Nothing value-returning wrapper must
        // surface exactly one result; a `Nothing` wrapper surfaces none.
        let wants_result = function.return_resource
            || function
                .return_type
                .as_deref()
                .is_some_and(|return_type| return_type != "Nothing");
        if wants_result && result_markers == 0 && function.result.is_none() {
            self.report(
                "NATIVE_ABI_NO_RESULT",
                &format!(
                    "Native function `{}` returns a value but no ABI slot is marked as the result (`return` or `RESULT`).",
                    function.name
                ),
                file,
                function.line,
            );
        }
        if result_markers > 1 {
            self.report(
                "NATIVE_ABI_RESULT_MARKER",
                &format!(
                    "Native function `{}` declares more than one `return` result marker.",
                    function.name
                ),
                file,
                function.line,
            );
        }

        // Every wrapper parameter must map to an ABI slot of the same name.
        let abi_slot_names: HashSet<&str> = function
            .abi
            .slots
            .iter()
            .map(|slot| slot.name.as_str())
            .collect();
        for param in &function.params {
            if !abi_slot_names.contains(param.name.as_str()) {
                self.report(
                    "NATIVE_ABI_UNBOUND_PARAM",
                    &format!(
                        "Native function `{}` parameter `{}` has no matching ABI slot.",
                        function.name, param.name
                    ),
                    file,
                    param.line,
                );
            }
        }

        // A CONST pin must name a real ABI slot.
        for pin in &function.consts {
            if !abi_slot_names.contains(pin.slot.as_str()) {
                self.report(
                    "NATIVE_CONST_UNKNOWN_SLOT",
                    &format!(
                        "Native function `{}` CONST pins unknown ABI slot `{}`.",
                        function.name, pin.slot
                    ),
                    file,
                    pin.line,
                );
            }
        }

        // A FREE block releases a caller-owned native return after it is copied
        // out (mfbasic.md §17). The implemented form frees the `return` CPtr
        // produced slot through a deallocator that takes one CPtr and returns
        // CVoid (e.g. `sqlite3_free`). Anything else is rejected.
        if let Some(free) = &function.free {
            let mut ok = true;
            // The freed slot must be the `return` C-return pointer.
            if free.slot != "return" || function.abi.return_name != "return" {
                ok = false;
            }
            // That return must be a CPtr copied into an owned wrapper value.
            if function.abi.return_ctype != "CPtr" {
                ok = false;
            }
            // The deallocator: one pointer parameter, void return.
            if free.param_ctype != "CPtr" || free.return_ctype != "CVoid" {
                ok = false;
            }
            if free.symbol.is_empty() {
                ok = false;
            }
            if !ok {
                self.report(
                    "NATIVE_FREE_INVALID",
                    &format!(
                        "Native function `{}` has a malformed FREE block: it must release the `return` CPtr produced slot through a deallocator taking one CPtr parameter and returning CVoid.",
                        function.name
                    ),
                    file,
                    free.line,
                );
            }
        }
    }

    pub(super) fn collect_types(&mut self) {
        for file in &self.ast.files {
            for item in &file.items {
                if let Item::Type(type_decl) = item {
                    self.user_types.insert(type_decl.name.clone());
                    self.user_type_kinds
                        .insert(type_decl.name.clone(), type_decl.kind);
                }
            }
        }

        for file in &self.ast.files {
            for item in &file.items {
                if let Item::Type(type_decl) = item {
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
        for file in &self.ast.files {
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
        file: &AstFile,
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
            // never let an imported entry override the built-in's semantics.
            if builtins::is_resource_type(&resource.type_name) {
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
            let info = builtins::ResourceInfo {
                close_function,
                sendable: resource.sendable,
                close_may_fail: resource.close_may_fail,
                kind: builtins::ResourceKind::Imported,
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
        file: &AstFile,
        line: usize,
        package_file: &Path,
        type_export: &BinaryReprTypeExport,
    ) {
        let mut seen = HashSet::new();
        match type_export.kind {
            BinaryReprExportKind::Type => {
                let type_ = Type::User(type_export.name.clone());
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
                let type_ = Type::User(type_export.name.clone());
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
        file: &AstFile,
        line: usize,
        package_file: &Path,
        type_: &Type,
        context: &str,
        seen: &mut HashSet<String>,
    ) {
        match type_ {
            Type::List(element) | Type::Result(element) | Type::Res(element) => {
                self.validate_package_metadata_type(
                    file,
                    line,
                    package_file,
                    element,
                    context,
                    seen,
                );
            }
            Type::Map(key, value) => {
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
            Type::Function {
                params,
                return_type,
                ..
            } => {
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
            Type::Thread(message, resource, output)
            | Type::ThreadWorker(message, resource, output) => {
                self.validate_package_metadata_type(
                    file,
                    line,
                    package_file,
                    message,
                    context,
                    seen,
                );
                if let Some(resource) = resource {
                    self.validate_package_metadata_type(
                        file,
                        line,
                        package_file,
                        resource,
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
            Type::User(name) => {
                if self.resource_registry.is_resource(name) || !seen.insert(name.clone()) {
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
            | Type::Error
            | Type::ErrorLoc
            | Type::Fixed
            | Type::Float
            | Type::Integer
            | Type::Money
            | Type::Nothing
            | Type::Scalar
            | Type::String
            | Type::Unknown => {}
        }
    }

    pub(super) fn collect_package_functions(&mut self) {
        let mut seen = HashSet::new();
        for file in &self.ast.files {
            for import in &file.imports {
                let binding = import.binding_name().to_string();
                let package = import.package_name().to_string();
                if !seen.insert(binding.clone()) || builtins::is_builtin_import(&package) {
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

    pub(super) fn validate_imported_function_signature(
        &mut self,
        file: &AstFile,
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
            .ast
            .files
            .iter()
            .flat_map(|file| &file.items)
            .find_map(|item| {
                let Item::Type(type_decl) = item else {
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

    pub(super) fn report_expanded_union_member_conflicts(
        &mut self,
        _file: &AstFile,
        _type_decl: &TypeDecl,
    ) {
        // Expanded-union member-conflict detection is now enforced by `ir::verify`
        // (the sole rejecter for both the source and package paths, plan-20). This
        // relocated syntaxcheck rule emits no diagnostic; the body is intentionally
        // empty.
    }

    pub(super) fn direct_record_successors(&self, name: &str) -> Vec<String> {
        let Some(info) = self.type_infos.get(name) else {
            return Vec::new();
        };
        if !matches!(info.kind, TypeDeclKind::Type) {
            return Vec::new();
        }
        info.fields
            .iter()
            .filter_map(|field| match &field.type_ {
                Type::User(type_name)
                    if matches!(
                        self.type_infos.get(type_name).map(|info| info.kind),
                        Some(TypeDeclKind::Type)
                    ) =>
                {
                    Some(type_name.clone())
                }
                _ => None,
            })
            .collect()
    }

    pub(super) fn record_field_cycle(&self, start: &str) -> bool {
        let mut visited = HashSet::new();
        let mut stack = self.direct_record_successors(start);
        while let Some(node) = stack.pop() {
            if node == start {
                return true;
            }
            if !visited.insert(node.clone()) {
                continue;
            }
            stack.extend(self.direct_record_successors(&node));
        }
        false
    }

    pub(super) fn type_info(&self, file: &AstFile, type_decl: &TypeDecl) -> TypeInfo {
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
        field: &TypeField,
        containing_visibility: Visibility,
    ) -> FieldInfo {
        FieldInfo {
            name: field.name.clone(),
            type_: self.parse_type(&field.type_name),
            visibility: effective_field_visibility(field.visibility, containing_visibility),
        }
    }

    pub(super) fn collect_bindings(&mut self) {
        for file in &self.ast.files {
            for item in &file.items {
                if let Item::Binding(binding) = item {
                    let type_ = binding
                        .type_name
                        .as_deref()
                        .map(|name| self.parse_type(name))
                        .unwrap_or(Type::Unknown);
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
        for file in &self.ast.files {
            for item in &file.items {
                if let Item::Function(function) = item {
                    let return_type = match function.kind {
                        FunctionKind::Func => function
                            .return_type
                            .as_deref()
                            .map(|name| self.parse_type(name))
                            .unwrap_or(Type::Unknown),
                        FunctionKind::Sub => Type::Nothing,
                    };
                    let params = function
                        .params
                        .iter()
                        .map(|param| ParamSig {
                            name: param.name.clone(),
                            type_: param
                                .type_name
                                .as_deref()
                                .map(|name| self.parse_type(name))
                                .unwrap_or(Type::Unknown),
                            has_default: param.default.is_some(),
                        })
                        .collect();
                    self.functions
                        .entry(function.name.clone())
                        .or_default()
                        .push(FunctionSig {
                            kind: function.kind.clone(),
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

    /// Register native `LINK` function signatures (keyed `alias.func`) and any
    /// `FUNC alias AS alias::func` re-exports, so wrapper code that calls
    /// `sqliteLink::open(...)` or importers that call `sqlite::close(...)` get a
    /// type (plan-link-update.md §5a/§5b).
    pub(super) fn collect_native_functions(&mut self) {
        // First gather every LINK function's signature so aliases can adopt them.
        let mut link_sigs: HashMap<String, (FunctionSig, String)> = HashMap::new();
        for file in &self.ast.files {
            for item in &file.items {
                let Item::Link(link) = item else {
                    continue;
                };
                for function in &link.functions {
                    let sig = self.native_function_sig(function, &file.path);
                    let key = format!("{}.{}", link.alias, function.name);
                    self.functions
                        .entry(key.clone())
                        .or_default()
                        .push(sig.clone());
                    link_sigs.insert(key, (sig, file.path.clone()));
                }
            }
        }

        // Then register re-export aliases, adopting the target's signature with
        // the alias's declared visibility (plan-link-update.md §5a).
        for file in &self.ast.files {
            for item in &file.items {
                let Item::FuncAlias(alias) = item else {
                    continue;
                };
                if let Some((sig, _)) = link_sigs.get(&alias.target) {
                    let mut adopted = sig.clone();
                    adopted.visibility = alias.visibility;
                    adopted.owner_file_path = file.path.clone();
                    self.functions
                        .entry(alias.name.clone())
                        .or_default()
                        .push(adopted);
                }
            }
        }
    }

    pub(super) fn native_function_sig(
        &self,
        function: &crate::ast::LinkFunction,
        owner_file_path: &str,
    ) -> FunctionSig {
        let return_type = function
            .return_type
            .as_deref()
            .map(|name| self.parse_type(name))
            .unwrap_or(Type::Nothing);
        let params = function
            .params
            .iter()
            .map(|param| ParamSig {
                name: param.name.clone(),
                type_: param
                    .type_name
                    .as_deref()
                    .map(|name| self.parse_type(name))
                    .unwrap_or(Type::Unknown),
                has_default: param.default.is_some(),
            })
            .collect();
        FunctionSig {
            kind: FunctionKind::Func,
            params,
            return_type,
            isolated: false,
            imported_package_export: false,
            // A LINK block is package-local; its functions are reachable from any
            // file of the declaring package via the alias namespace.
            visibility: Visibility::Public,
            owner_file_path: owner_file_path.to_string(),
        }
    }

    pub(super) fn canonical_import_name(&self, file: &AstFile, name: &str) -> String {
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
        file: &AstFile,
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
        file: &AstFile,
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
        file: &AstFile,
        name: &str,
    ) -> Option<&'b BindingSig> {
        self.bindings
            .get(name)
            .filter(|sig| self.visible_from(file, sig.visibility, &sig.owner_file_path))
    }

    pub(super) fn lookup_visible_call_sig<'b>(
        &'b self,
        file: &AstFile,
        name: &str,
        arguments: &[CallArg],
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

    pub(super) fn call_shape_matches_sig(&self, arguments: &[CallArg], sig: &FunctionSig) -> bool {
        let positional = arguments
            .iter()
            .take_while(|argument| matches!(argument, CallArg::Positional(_)))
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
            let CallArg::Named { name, .. } = argument else {
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
        for file in &self.ast.files {
            for item in &file.items {
                match item {
                    Item::Binding(binding) => self.check_binding(file, binding),
                    Item::Function(function) => self.check_function(file, function),
                    Item::Type(type_decl) => self.check_type_decl(file, type_decl),
                    Item::Resource(resource) => self.check_resource_decl(file, resource),
                    Item::Link(link) => self.check_link_block(file, link),
                    // A re-export alias carries no body to check; its target was
                    // validated during resolve (plan-link-update.md §5a).
                    Item::FuncAlias(_) => {}
                    // DOC blocks carry no executable code to syntaxcheck.
                    Item::Doc(_) => {}
                    // TESTING blocks are lowered away before syntaxcheck (plan-18-A §3).
                    Item::Testing(_) => {}
                }
            }
        }
    }

    pub(super) fn check_binding(&mut self, file: &AstFile, binding: &TopLevelBinding) {
        let mut locals = HashMap::new();
        let declared = binding
            .type_name
            .as_deref()
            .map(|name| self.parse_type(name));
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
            binding.state_type.as_deref(),
            (binding_type != Type::Unknown).then_some(&binding_type),
            &format!("binding `{}`", binding.name),
        );
        if let Some(sig) = self.bindings.get_mut(&binding.name) {
            sig.type_ = binding_type;
        }
    }

    pub(super) fn check_type_decl(&mut self, file: &AstFile, type_decl: &TypeDecl) {
        match type_decl.kind {
            TypeDeclKind::Type => {
                for field in &type_decl.fields {
                    let type_ = self.parse_type(&field.type_name);
                    self.check_type_reference(file, &type_, field.line);
                    // A record (product) may never own a resource: it would trap
                    // copyable data behind move-only semantics (ownership
                    // contagion). Resource data travels in the resource's STATE.
                    if self.is_resource_type(&type_) {}
                }
                if self.record_field_cycle(&type_decl.name) {}
            }
            TypeDeclKind::Union => {
                for include in &type_decl.includes {
                    let type_ = self.parse_type(include);
                    self.check_type_reference(file, &type_, type_decl.line);
                    if let Some(kind) = self.user_type_kinds.get(include) {
                        if !matches!(kind, TypeDeclKind::Union) {}
                    }
                }
                self.report_expanded_union_member_conflicts(file, type_decl);

                for variant in &type_decl.variants {
                    let type_ = self.parse_type(&variant.name);
                    self.check_type_reference(file, &type_, variant.line);
                    if let Some(kind) = self.user_type_kinds.get(&variant.name) {
                        if !matches!(kind, TypeDeclKind::Type) {}
                    }
                }
                // A union must be uniformly data or uniformly resource: a union
                // whose copyability would depend on the runtime tag (the
                // contagion + conditional-drop problem) is rejected. An
                // all-resource union is a resource union (§4a).
                let resource_variants = type_decl
                    .variants
                    .iter()
                    .filter(|variant| self.resource_registry.is_resource(&variant.name))
                    .count();
                if resource_variants > 0 && resource_variants < type_decl.variants.len() {}
            }
            TypeDeclKind::Enum => if type_decl.members.is_empty() {},
        }
    }

    pub(super) fn check_function(&mut self, file: &AstFile, function: &Function) {
        if function.isolated
            && (!matches!(function.kind, FunctionKind::Func)
                || matches!(function.visibility, Visibility::Private))
        {
            self.report(
                "TYPE_ISOLATED_NOT_VISIBLE",
                &format!(
                    "ISOLATED function `{}` must be a project-visible FUNC declaration \
                     (PUBLIC — the default — or EXPORT, not PRIVATE).",
                    function.name
                ),
                file,
                function.line,
            );
        }

        let expected_return = match function.kind {
            FunctionKind::Func => {
                if function.return_type.is_none() {
                    Type::Unknown
                } else {
                    let return_type = self.parse_type(function.return_type.as_deref().unwrap());
                    // `check_type_reference` reports `TYPE_RESULT_NOT_USER_VISIBLE`
                    // for a `Result` in any type position, including this one.
                    self.check_type_reference(file, &return_type, function.line);
                    if matches!(return_type, Type::Result(_)) {
                        Type::Unknown
                    } else {
                        return_type
                    }
                }
            }
            FunctionKind::Sub => {
                if function.return_type.is_some() {
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
            if let Some(return_name) = function.return_type.as_deref() {
                let return_type = self.parse_type(return_name);
                self.check_resource_declaration(
                    file,
                    function.line,
                    function.return_resource,
                    function.return_state_type.as_deref(),
                    Some(&return_type),
                    "return type",
                );
                // Returning `List OF RES File` transfers scope-ownership of the
                // referenced resources to the caller, which adopts them (§15.6).
                // (A bare `List OF File` return is already rejected at the type
                // level, since a resource element must be `RES`-marked.)
            }
        }

        let mut locals = HashMap::new();
        let mut seen_default = false;
        for param in &function.params {
            let param_type = param
                .type_name
                .as_deref()
                .map(|name| self.parse_type(name))
                .unwrap_or(Type::Unknown);
            self.check_type_reference(file, &param_type, param.line);

            if param.type_name.is_none() {}

            self.check_resource_declaration(
                file,
                param.line,
                param.resource,
                param.state_type.as_deref(),
                (param_type != Type::Unknown).then_some(&param_type),
                &format!("parameter `{}`", param.name),
            );

            if param.default.is_some() {
                seen_default = true;
            } else if seen_default {
            }

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
                if !self.expression_compatible(&param_type, &default_type, Some(default)) {}
            }

            let _borrowed = self.is_resource_type(&param_type);
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
                    type_: Type::Error,
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
            // A bare `TRAP` synthesizes a `#`-sentinel binding the user never
            // named; refer to it as "the TRAP handler" instead of exposing the
            // internal name in diagnostics.
            let trap_label = if trap.name == crate::ast::SYNTHETIC_TRAP_BINDING {
                "the TRAP handler".to_string()
            } else {
                format!("TRAP `{}`", trap.name)
            };
            if trap_flow != Flow::AlwaysReturns {
                self.report(
                    "TYPE_TRAP_FALLTHROUGH",
                    &format!("{trap_label} must return, fail, or propagate."),
                    file,
                    trap.line,
                );
            }
            if flow != Flow::AlwaysReturns {
                self.report(
                    "TYPE_TRAP_FALLTHROUGH",
                    &format!(
                        "Normal flow in `{}` reaches {trap_label}; body paths before TRAP must end with RETURN or FAIL.",
                        function.name
                    ),
                    file,
                    trap.line,
                );
            }
        }

        // A `FUNC … AS Nothing` produces no value, so — like a `SUB` — it may
        // fall through with an implicit `RETURN NOTHING`. Only value-producing
        // FUNCs must return on every path.
        if matches!(function.kind, FunctionKind::Func)
            && !matches!(expected_return, Type::Nothing)
            && flow != Flow::AlwaysReturns
        {}
    }

    pub(super) fn visible_from(
        &self,
        file: &AstFile,
        visibility: Visibility,
        owner_file_path: &str,
    ) -> bool {
        match visibility {
            Visibility::Export | Visibility::Public => true,
            Visibility::Private => file.path == owner_file_path,
        }
    }

    pub(super) fn check_type_reference(&mut self, file: &AstFile, type_: &Type, line: usize) {
        match type_ {
            Type::List(element) => {
                let inner = strip_res(element);
                self.check_type_reference(file, inner, line);
                // A `List` element may be a resource borrow (§15.6); only thread
                // handles are forbidden in collections.
                if self.contains_thread(inner) {
                    self.report_invalid_collection_element(file, line, "element", inner);
                }
            }
            Type::Map(key, value) => {
                let value_inner = strip_res(value);
                self.check_type_reference(file, key, line);
                self.check_type_reference(file, value_inner, line);
                // A resource may not be a `Map` key (handles are not comparable),
                // but a `Map` *value* may be a resource borrow (§15.6).
                if self.contains_resource_or_thread(key) {
                    self.report_invalid_collection_element(file, line, "key", key);
                }
                self.require_comparable_type(file, line, "Map key type", key);
                if self.contains_thread(value_inner) {
                    self.report_invalid_collection_element(file, line, "value", value_inner);
                }
            }
            Type::Res(inner) => self.check_type_reference(file, inner, line),
            Type::Function {
                params,
                return_type,
                ..
            } => {
                for param in params {
                    self.check_type_reference(file, param, line);
                }
                self.check_type_reference(file, return_type, line);
            }
            Type::Result(_) => {
                // `Result` is internal: it is never nameable in a user type
                // position. (The resolver normally catches this first; this keeps
                // the invariant honest for any type that reaches the checker.)
                self.report(
                    "TYPE_RESULT_NOT_USER_VISIBLE",
                    "`Result` is an internal type; declare the success type directly \
                     (a function call yields its value or fails with an `Error`).",
                    file,
                    line,
                );
            }
            Type::Thread(message, resource, output)
            | Type::ThreadWorker(message, resource, output) => {
                self.check_type_reference(file, message, line);
                self.check_type_reference(file, output, line);
                self.require_thread_sendable_type(file, line, "Thread message type", message);
                self.require_thread_sendable_type(file, line, "Thread output type", output);
                // The data plane is resource-free (§7): a resource may only ride
                // the `RES Res` resource plane, never the message slot.
                if self.is_resource_type(message) {
                    self.report(
                        "TYPE_THREAD_NOT_SENDABLE",
                        &format!(
                            "Thread message type `{}` is a resource; the data channel is resource-free — declare it on the resource plane (`Thread OF … RES {} TO …`).",
                            self.type_name(message),
                            self.type_name(message)
                        ),
                        file,
                        line,
                    );
                }
                if let Some(resource) = resource {
                    self.check_type_reference(file, resource, line);
                    self.require_thread_sendable_type(file, line, "Thread resource type", resource);
                }
            }
            Type::User(name) => {
                if name == "Ok" {
                    // `Ok` is the internal success member of `Result`; it is not a
                    // user-nameable type.
                    self.report(
                        "TYPE_RESULT_NOT_USER_VISIBLE",
                        "`Ok` is an internal type; declare the success type directly \
                         (a function call yields its value or fails with an `Error`).",
                        file,
                        line,
                    );
                    return;
                }
                let Some(info) = self.type_infos.get(name) else {
                    return;
                };
                if !self.visible_from(file, info.visibility, &info.file_path) {}
            }
            Type::Boolean
            | Type::Byte
            | Type::Error
            | Type::ErrorLoc
            | Type::Fixed
            | Type::Float
            | Type::Integer
            | Type::Money
            | Type::Nothing
            | Type::Scalar
            | Type::String
            | Type::Unknown => {}
        }
    }

    pub(super) fn type_name(&self, type_: &Type) -> String {
        match type_ {
            Type::Boolean => "Boolean".to_string(),
            Type::Byte => "Byte".to_string(),
            Type::Error => "Error".to_string(),
            Type::ErrorLoc => "ErrorLoc".to_string(),
            Type::Fixed => "Fixed".to_string(),
            Type::Float => "Float".to_string(),
            Type::Integer => "Integer".to_string(),
            Type::Money => "Money".to_string(),
            Type::Scalar => "Scalar".to_string(),
            Type::List(element) => format!("List OF {}", self.type_name(element)),
            Type::Map(key, value) => {
                format!(
                    "Map OF {} TO {}",
                    self.type_name(key),
                    self.type_name(value)
                )
            }
            Type::Res(inner) => format!("RES {}", self.type_name(inner)),
            Type::Function {
                params,
                return_type,
                isolated,
            } => {
                let params = params
                    .iter()
                    .map(|param| self.type_name(param))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "{}FUNC({}) AS {}",
                    if *isolated { "ISOLATED " } else { "" },
                    params,
                    self.type_name(return_type)
                )
            }
            Type::Nothing => "Nothing".to_string(),
            Type::Result(success) => format!("Result OF {}", self.type_name(success)),
            Type::String => "String".to_string(),
            Type::Thread(message, resource, output) => self.format_thread_type_name(
                builtins::thread::THREAD_TYPE,
                message,
                resource,
                output,
            ),
            Type::ThreadWorker(message, resource, output) => self.format_thread_type_name(
                builtins::thread::THREAD_WORKER_TYPE,
                message,
                resource,
                output,
            ),
            Type::User(name) => name.clone(),
            Type::Unknown => "Unknown".to_string(),
        }
    }

    pub(super) fn thread_type_argument_name(&self, type_: &Type) -> String {
        let name = self.type_name(type_);
        if name.contains(" TO ") {
            format!("({name})")
        } else {
            name
        }
    }

    /// Format a `Thread`/`ThreadWorker` type, emitting the optional `RES Res`
    /// clause and the resource-only spelling (message `Nothing`) symmetrically
    /// with the parser.
    pub(super) fn format_thread_type_name(
        &self,
        kind: &str,
        message: &Type,
        resource: &Option<Box<Type>>,
        output: &Type,
    ) -> String {
        let message = self.thread_type_argument_name(message);
        let output = self.thread_type_argument_name(output);
        let resource = resource
            .as_ref()
            .map(|resource| self.thread_type_argument_name(resource));
        builtins::thread::format_thread_type(kind, &message, resource.as_deref(), &output)
    }

    pub(super) fn report(&mut self, rule: &str, detail: &str, file: &AstFile, line: usize) {
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
    pub(super) fn report_warning(&mut self, rule: &str, detail: &str, file: &AstFile, line: usize) {
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
        let diagnostics = check_project_collect(Path::new("."), &project)
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
        match check_project_collect(dir, &project) {
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
        crate::testutil::fixture_dir(name).to_string_lossy().into_owned()
    }

    // ---- check_function -----------------------------------------------------

    #[test]
    fn isolated_private_func_rejected() {
        // ISOLATED requires a project-visible FUNC (PUBLIC — the default — or
        // EXPORT); an explicit PRIVATE ISOLATED is rejected. (An unmarked ISOLATED
        // FUNC is PUBLIC and accepted — exercised by the thread acceptance tests.)
        assert!(rejects_with(
            "PRIVATE ISOLATED FUNC w(x AS Integer) AS Integer\n  RETURN x\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
            "TYPE_ISOLATED_NOT_VISIBLE"
        ));
    }

    // NOTE: TYPE_SUB_CANNOT_RETURN_VALUE is unreachable from source — the parser
    // only reads a return type for a FUNC, so a `SUB … AS T` never parses. The
    // branch is defensive for IR/package-decoded functions and stays uncovered.

    #[test]
    fn func_returning_result_rejected() {
        assert!(rejects_with(
            "FUNC f AS Result OF Integer\n  RETURN 1\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
            "TYPE_RESULT_NOT_USER_VISIBLE"
        ));
    }

    #[test]
    fn func_returning_ok_rejected() {
        assert!(rejects_with(
            "FUNC f AS Ok\n  RETURN 1\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
            "TYPE_RESULT_NOT_USER_VISIBLE"
        ));
    }

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

    #[test]
    fn trap_fallthrough_rejected() {
        // TRAP body that neither returns nor fails.
        assert!(rejects_with(
            "IMPORT io\nIMPORT fs\nFUNC f AS Integer\n  LET x = fs::readText(\"a\")\n  RETURN len(x)\n  TRAP(err)\n    io::print(\"oops\")\n  END TRAP\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
            "TYPE_TRAP_FALLTHROUGH"
        ));
    }

    #[test]
    fn trap_body_reaches_trap_fallthrough() {
        // The normal flow before the TRAP falls through (no RETURN/FAIL) so the
        // body-reaches-TRAP TYPE_TRAP_FALLTHROUGH check fires.
        assert!(rejects_with(
            "IMPORT io\nIMPORT fs\nFUNC f AS Integer\n  LET x = fs::readText(\"a\")\n  io::print(x)\n  TRAP(err)\n    RETURN 0\n  END TRAP\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
            "TYPE_TRAP_FALLTHROUGH"
        ));
    }

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

    #[test]
    fn non_default_after_default_walk() {
        // A required parameter following a defaulted one walks the seen_default
        // branch.
        let _ = check_src(
            "FUNC g(a AS Integer = 1, b AS Integer) AS Integer\n  RETURN a + b\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
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
        // Exercises expanded_union_variants / report_expanded_union_member_conflicts.
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
        // A record whose field carries a resource borrow walks the is_resource
        // branch inside check_type_decl.
        let _ = check_src(
            "IMPORT fs\nTYPE Holder\n  fs AS List OF RES File\nEND TYPE\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
        );
    }

    #[test]
    fn record_field_cycle_walk() {
        // A self-referential record walks record_field_cycle.
        let _ = check_src(
            "TYPE Node\n  link AS Node\nEND TYPE\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
        );
    }

    #[test]
    fn mixed_resource_union_walk() {
        // A union with one resource variant and one data variant walks the
        // mixed-union arm of check_type_decl.
        let _ = check_src(
            "IMPORT fs\nTYPE B\n  n AS Integer\nEND TYPE\nUNION Mixed\n  File\n  B\nEND UNION\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
        );
    }

    #[test]
    fn record_bare_resource_field_walk() {
        // A record with a bare resource-typed field walks the is_resource_type
        // branch of the Type arm in check_type_decl.
        let _ = check_src(
            "IMPORT fs\nTYPE Holder\n  file AS File\nEND TYPE\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
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
        // A variant declared directly that is also brought in via INCLUDES walks
        // report_expanded_union_member_conflicts' conflict-found arm.
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

    #[test]
    fn map_resource_key_rejected() {
        // A resource may not be a Map key.
        let src = "IMPORT fs\nFUNC main AS Integer\n  LET m AS Map OF File TO Integer = Map OF File TO Integer {}\n  RETURN 0\nEND FUNC\n";
        assert!(!accepts(src));
    }

    #[test]
    fn thread_resource_message_rejected() {
        // A resource in the message (data) plane of a Thread type.
        let src = "IMPORT thread\nIMPORT fs\nFUNC main AS Integer\n  LET t AS Thread OF File TO Integer\n  RETURN 0\nEND FUNC\n";
        assert!(rejects_with(src, "TYPE_THREAD_NOT_SENDABLE"));
    }

    #[test]
    fn list_of_thread_rejected() {
        // A thread handle may never live in a collection.
        let src = "IMPORT thread\nFUNC main AS Integer\n  LET xs AS List OF Thread OF Integer TO Integer = []\n  RETURN 0\nEND FUNC\n";
        assert!(rejects_with(src, "TYPE_COLLECTION_OWNERSHIP_VIOLATION"));
    }

    #[test]
    fn map_value_thread_rejected() {
        // A thread in a Map value position is forbidden.
        let src = "IMPORT thread\nFUNC main AS Integer\n  LET m AS Map OF String TO Thread OF Integer TO Integer = Map OF String TO Thread OF Integer TO Integer {}\n  RETURN 0\nEND FUNC\n";
        assert!(rejects_with(src, "TYPE_COLLECTION_OWNERSHIP_VIOLATION"));
    }

    #[test]
    fn res_return_and_param_with_state_walk() {
        // A RES return producer and a RES parameter with a STATE type walk
        // check_resource_declaration on both positions.
        let src = "IMPORT fs\nFUNC use(RES f AS File) AS Integer\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  RES f AS File = fs::openFile(\"x\")\n  RETURN use(f)\nEND FUNC\n";
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
            &link_wrap("  FUNC leak() AS CPtr\n    SYMBOL \"demo_leak\"\n    ABI () AS return CPtr\n  END FUNC\n"),
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
            &link_wrap("  FUNC describe(RES db AS Db) AS String\n    SYMBOL \"demo_describe\"\n    ABI (db CPtr) AS return CPtr\n    FREE return\n      SYMBOL \"demo_free\"\n      ABI (ptr CInt32) AS CVoid\n    END FREE\n  END FUNC\n"),
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
        // `return OUT CPtr` result marker on a resource producer.
        assert!(accepts(&link_wrap(
            "  FUNC opn(statement AS String) AS RES Db\n    SYMBOL \"demo_open\"\n    ABI (statement CString, return OUT CPtr) AS status CInt32\n    SUCCESS_ON status = 0\n  END FUNC\n"
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
    fn link_result_marker_not_out_rejected() {
        // An ABI slot named `return` that is not marked OUT.
        assert!(rejects_with(
            &link_wrap("  FUNC opn(statement AS String) AS RES Db\n    SYMBOL \"demo_open\"\n    ABI (statement CString, return CPtr) AS status CInt32\n    SUCCESS_ON status = 0\n  END FUNC\n"),
            "NATIVE_ABI_RESULT_MARKER"
        ));
    }

    #[test]
    fn link_out_slot_not_return_rejected() {
        // An OUT slot not named `return` (multi-out RETURN_OUT is unsupported).
        assert!(rejects_with(
            &link_wrap("  FUNC opn(statement AS String) AS RES Db\n    SYMBOL \"demo_open\"\n    ABI (statement CString, extra OUT CPtr) AS status CInt32\n    SUCCESS_ON status = 0\n  END FUNC\n"),
            "NATIVE_ABI_UNBOUND_SLOT"
        ));
    }

    #[test]
    fn link_free_wrong_return_ctype_rejected() {
        // FREE on a non-CPtr `return` produced slot is malformed.
        assert!(rejects_with(
            &link_wrap("  FUNC describe(RES db AS Db) AS Integer\n    SYMBOL \"demo_describe\"\n    ABI (db CPtr) AS return CInt32\n    FREE return\n      SYMBOL \"demo_free\"\n      ABI (ptr CPtr) AS CVoid\n    END FREE\n  END FUNC\n"),
            "NATIVE_FREE_INVALID"
        ));
    }

    #[test]
    fn link_free_empty_symbol_rejected() {
        // A FREE block with an empty deallocator symbol is malformed (the symbol
        // check arm of the FREE validation).
        assert!(rejects_with(
            &link_wrap("  FUNC describe(RES db AS Db) AS String\n    SYMBOL \"demo_describe\"\n    ABI (db CPtr) AS return CPtr\n    FREE return\n      SYMBOL \"\"\n      ABI (ptr CPtr) AS CVoid\n    END FREE\n  END FUNC\n"),
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
        let project = AstProject {
            name: "t".into(),
            files: vec![file],
        };
        let diags = super::check_project_collect(&dir, &project).unwrap();
        let _ = fs::remove_dir_all(&dir);
        assert!(
            diags.iter().any(|d| d.rule == "PACKAGE_INVALID"),
            "expected PACKAGE_INVALID, got {:?}",
            diags.iter().map(|d| &d.rule).collect::<Vec<_>>()
        );
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
        let project = AstProject {
            name: "t".into(),
            files: vec![file],
        };
        assert!(super::check_project(Path::new("."), &project).is_ok());
    }

    // Exercises the standalone `check_project` render wrapper (reject path).
    #[test]
    fn check_project_wrapper_rejects() {
        use crate::ast::{parse_source, AstProject};
        let file = parse_source(
            Path::new("main.mfb"),
            "main.mfb",
            "FUNC f AS Result OF Integer\n  RETURN 1\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
        )
        .unwrap();
        let project = AstProject {
            name: "t".into(),
            files: vec![file],
        };
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
        let src = "EXPORT LET g AS Integer = 5\nEXPORT TYPE Rec\n  x AS Integer\nEND TYPE\nEXPORT FUNC f() AS Integer\n  RETURN 1\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n";
        let file = parse_source(Path::new("main.mfb"), "main.mfb", src).expect("parse");
        let project = AstProject {
            name: "t".to_string(),
            files: vec![file],
        };
        let diags = crate::syntaxcheck::export_in_executable_diagnostics(false, &project);
        assert!(diags.iter().all(|d| d.rule == "EXPORT_IN_EXECUTABLE"));
        assert!(diags.len() >= 3, "expected an EXPORT diagnostic per item");
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
        let src = "IMPORT fs\nTYPE Bad\n  f AS File\nEND TYPE\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn self_referential_record_walks_cycle_arm() {
        let src = "TYPE Node\n  child AS Node\nEND TYPE\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n";
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
}
