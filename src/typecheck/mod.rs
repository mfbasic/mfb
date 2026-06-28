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
use crate::rules;
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
    ownership: OwnershipState,
    /// A borrowed resource (a `RES` parameter): the caller retains ownership, so
    /// the callee may use it and mutate its `STATE` but may not invalidate it
    /// (close, `RETURN`, or `thread::transfer` require ownership).
    borrowed: bool,
    /// The `STATE T` type attached to a `RES` binding/parameter, if any. Drives
    /// `s.state` member access typing.
    state_type: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OwnershipState {
    Available,
    Moved,
    MaybeMoved,
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
    mutable: bool,
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

pub fn check_project(project_dir: &Path, ast: &AstProject) -> Result<(), ()> {
    let augmented = builtins::json::augmented_project(ast)?;
    let augmented = builtins::csv::augmented_project(&augmented)?;
    let augmented = builtins::regex::augmented_project(&augmented)?;
    let augmented = builtins::datetime::augmented_project(&augmented)?;
    // `vector` imports only intrinsic `math` (plan-06-vector.md §5).
    let augmented = builtins::vector::augmented_project(&augmented)?;
    // `http` before `net`: `http_package.mfb` imports `net` (plan-03-http.md Phase 4).
    let augmented = builtins::http::augmented_project(&augmented)?;
    let augmented = builtins::net::augmented_project(&augmented)?;
    let augmented = builtins::encoding::augmented_project(&augmented)?;
    let mut checker = TypeChecker::new(project_dir, &augmented);
    checker.check();
    if checker.had_error {
        Err(())
    } else {
        Ok(())
    }
}

struct TypeChecker<'a> {
    project_dir: &'a Path,
    ast: &'a AstProject,
    functions: HashMap<String, Vec<FunctionSig>>,
    bindings: HashMap<String, BindingSig>,
    user_types: HashSet<String>,
    user_type_kinds: HashMap<String, TypeDeclKind>,
    type_infos: HashMap<String, TypeInfo>,
    had_error: bool,
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
    /// Resource ownership decisions (escape analysis, §15.6) for the function
    /// currently being checked. Drives borrow-only demotion of `RES` bindings
    /// whose ownership has floated into an outer-scope collection.
    current_resource_owners: crate::escape::FunctionEscape,
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

impl<'a> TypeChecker<'a> {
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
            current_return: Type::Nothing,
            current_is_sub: false,
            allow_value_less_call: false,
            inline_trap_types: Vec::new(),
            loop_stack: Vec::new(),
            resource_registry: builtins::ResourceRegistry::with_builtins(),
            close_op_aliases: HashMap::new(),
            current_resource_owners: crate::escape::FunctionEscape::default(),
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
    pub(super) fn check_resource_decl(&mut self, _file: &AstFile, _resource: &crate::ast::ResourceDecl) {}

    /// Native-specific checks on a `LINK` block: `CPtr` containment and ABI
    /// slot/parameter consistency (plan-link-update.md §5b/§5c/§11/§12).
    pub(super) fn check_link_block(&mut self, file: &AstFile, link: &crate::ast::LinkBlock) {
        for function in &link.functions {
            self.check_link_function(file, function);
        }
    }

    pub(super) fn check_link_function(&mut self, file: &AstFile, function: &crate::ast::LinkFunction) {
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
            | Type::Nothing
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
                BinaryReprTypeVisibility::Package => Visibility::Package,
                BinaryReprTypeVisibility::Export => Visibility::Export,
            },
        }
    }

    pub(super) fn package_variant_info(&self, variant: BinaryReprTypeVariant) -> VariantConstructor {
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

    pub(super) fn report_expanded_union_member_conflicts(&mut self, file: &AstFile, type_decl: &TypeDecl) {
        let mut included_members: HashMap<String, String> = HashMap::new();
        for include in &type_decl.includes {
            for variant in self.expanded_union_variants(include, &mut HashSet::new()) {
                if let Some(previous_include) =
                    included_members.insert(variant.name.clone(), include.clone())
                {
                    self.report(
                        "TYPE_DUPLICATE_VARIANT",
                        &format!(
                            "Member type `{}` in UNION `{}` is provided by both included UNION `{}` and included UNION `{}`.",
                            variant.name, type_decl.name, previous_include, include
                        ),
                        file,
                        type_decl.line,
                    );
                }
            }
        }

        for variant in &type_decl.variants {
            if let Some(include) = included_members.get(&variant.name) {
                self.report(
                    "TYPE_DUPLICATE_VARIANT",
                    &format!(
                        "Member type `{}` in UNION `{}` conflicts with a member included from UNION `{}`.",
                        variant.name, type_decl.name, include
                    ),
                    file,
                    variant.line,
                );
            }
        }
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

    pub(super) fn field_info(&self, field: &TypeField, containing_visibility: Visibility) -> FieldInfo {
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
                            mutable: binding.mutable,
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
            visibility: Visibility::Package,
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

    pub(super) fn visible_function_sigs<'b>(&'b self, file: &AstFile, name: &str) -> Vec<&'b FunctionSig> {
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

    pub(super) fn lookup_visible_binding<'b>(&'b self, file: &AstFile, name: &str) -> Option<&'b BindingSig> {
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
                    // DOC blocks carry no executable code to typecheck.
                    Item::Doc(_) => {}
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
                    if self.is_resource_type(&type_) {
                        self.report(
                            "TYPE_RESOURCE_FIELD_FORBIDDEN",
                            &format!(
                                "Record `{}` field `{}` is resource `{}`; records cannot own resources. Hold a `RES` binding or carry the data in the resource's STATE.",
                                type_decl.name,
                                field.name,
                                self.type_name(&type_)
                            ),
                            file,
                            field.line,
                        );
                    }
                }
                if self.record_field_cycle(&type_decl.name) {
                    self.report(
                        "TYPE_RECURSIVE_RECORD_REQUIRES_INDIRECTION",
                        &format!(
                            "Record `{}` refers to itself without passing through a List, Map, or UNION; such a record has no base case and cannot be constructed.",
                            type_decl.name
                        ),
                        file,
                        type_decl.line,
                    );
                }
            }
            TypeDeclKind::Union => {
                for include in &type_decl.includes {
                    let type_ = self.parse_type(include);
                    self.check_type_reference(file, &type_, type_decl.line);
                    if let Some(kind) = self.user_type_kinds.get(include) {
                        if !matches!(kind, TypeDeclKind::Union) {
                            self.report(
                                "TYPE_UNION_INCLUDE_REQUIRES_UNION",
                                &format!(
                                    "UNION `{}` includes `{include}`, but `{include}` is not a UNION.",
                                    type_decl.name
                                ),
                                file,
                                type_decl.line,
                            );
                        }
                    }
                }
                self.report_expanded_union_member_conflicts(file, type_decl);

                for variant in &type_decl.variants {
                    let type_ = self.parse_type(&variant.name);
                    self.check_type_reference(file, &type_, variant.line);
                    if let Some(kind) = self.user_type_kinds.get(&variant.name) {
                        if !matches!(kind, TypeDeclKind::Type) {
                            self.report(
                                "TYPE_UNION_MEMBER_REQUIRES_TYPE",
                                &format!(
                                    "UNION `{}` member `{}` must be a concrete TYPE.",
                                    type_decl.name, variant.name
                                ),
                                file,
                                variant.line,
                            );
                        }
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
                if resource_variants > 0 && resource_variants < type_decl.variants.len() {
                    self.report(
                        "TYPE_MIXED_RESOURCE_UNION",
                        &format!(
                            "UNION `{}` mixes data and resource variants; a union must be all-data or all-resource.",
                            type_decl.name
                        ),
                        file,
                        type_decl.line,
                    );
                }
            }
            TypeDeclKind::Enum => {
                if type_decl.members.is_empty() {
                    self.report(
                        "TYPE_ENUM_REQUIRES_MEMBER",
                        &format!(
                            "ENUM `{}` must declare at least one member.",
                            type_decl.name
                        ),
                        file,
                        type_decl.line,
                    );
                }
            }
        }
    }

    pub(super) fn check_function(&mut self, file: &AstFile, function: &Function) {
        self.current_resource_owners = crate::escape::analyze_function(function);
        if function.isolated
            && (!matches!(function.kind, FunctionKind::Func)
                || !matches!(function.visibility, Visibility::Export))
        {
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "ISOLATED function `{}` must be an exported FUNC declaration.",
                    function.name
                ),
                file,
                function.line,
            );
        }

        let expected_return = match function.kind {
            FunctionKind::Func => {
                if function.return_type.is_none() {
                    self.report(
                        "TYPE_FUNC_REQUIRES_RETURN_TYPE",
                        &format!("FUNC `{}` must declare an `AS` return type.", function.name),
                        file,
                        function.line,
                    );
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

            if param.type_name.is_none() {
                self.report(
                    "TYPE_PARAM_REQUIRES_TYPE",
                    &format!("Parameter `{}` must declare an `AS` type.", param.name),
                    file,
                    param.line,
                );
            }

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
                self.report(
                    "TYPE_DEFAULT_ARG_ORDER",
                    &format!(
                        "Parameter `{}` must have a default because an earlier parameter has one.",
                        param.name
                    ),
                    file,
                    param.line,
                );
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
                if !self.expression_compatible(&param_type, &default_type, Some(default)) {
                    self.report(
                        "TYPE_DEFAULT_VALUE_MISMATCH",
                        &format!(
                            "Default value for `{}` has type {}, expected {}.",
                            param.name,
                            self.type_name(&default_type),
                            self.type_name(&param_type)
                        ),
                        file,
                        param.line,
                    );
                }
            }

            let borrowed = self.is_resource_type(&param_type);
            let state_type = param.state_type.clone();
            locals.insert(
                param.name.clone(),
                LocalInfo {
                    type_: param_type,
                    mutable: false,
                    ownership: OwnershipState::Available,
                    borrowed,
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
                    ownership: OwnershipState::Available,
                    borrowed: false,
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
            if trap_flow != Flow::AlwaysReturns {
                self.report(
                    "TYPE_TRAP_FALLTHROUGH",
                    &format!("TRAP `{}` must return, fail, or propagate.", trap.name),
                    file,
                    trap.line,
                );
            }
            if flow != Flow::AlwaysReturns {
                self.report(
                    "TYPE_TRAP_FALLTHROUGH",
                    &format!(
                        "Normal flow in `{}` reaches TRAP `{}`; body paths before TRAP must end with RETURN or FAIL.",
                        function.name, trap.name
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
        {
            self.report(
                "TYPE_FUNC_MISSING_RETURN",
                &format!(
                    "FUNC `{}` must return a {} value.",
                    function.name,
                    self.type_name(&expected_return)
                ),
                file,
                function.line,
            );
        }
    }

    pub(super) fn visible_from(&self, file: &AstFile, visibility: Visibility, owner_file_path: &str) -> bool {
        match visibility {
            Visibility::Export | Visibility::Package => true,
            Visibility::Private => file.path == owner_file_path,
        }
    }

    pub(super) fn check_type_reference(&mut self, file: &AstFile, type_: &Type, line: usize) {
        match type_ {
            Type::List(element) => {
                let inner = strip_res(element);
                self.check_type_reference(file, inner, line);
                self.check_collection_element_axis(file, line, "element", element);
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
                self.check_collection_element_axis(file, line, "value", value);
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
                if !self.visible_from(file, info.visibility, &info.file_path) {
                    self.report(
                        "TYPE_MEMBER_NOT_VISIBLE",
                        &format!("Type `{name}` is not visible from this file."),
                        file,
                        line,
                    );
                }
            }
            Type::Boolean
            | Type::Byte
            | Type::Error
            | Type::ErrorLoc
            | Type::Fixed
            | Type::Float
            | Type::Integer
            | Type::Nothing
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
        self.had_error = true;
        rules::show_diagnostic(rule, detail, &self.project_dir.join(&file.path), line, 1, 1);
    }

    pub(super) fn report_primitive_literal_range_error(
        &mut self,
        expected: &Type,
        expression: &Expression,
        file: &AstFile,
        line: usize,
    ) -> bool {
        match expected {
            Type::Byte => {
                let Some(range_error) = byte_literal_range_error(expression) else {
                    return false;
                };
                match range_error {
                    ByteLiteralRangeError::Overflow(value) => self.report(
                        "TYPE_BYTE_LITERAL_OVERFLOW",
                        &format!("Integer literal `{value}` is outside the Byte range 0..255."),
                        file,
                        line,
                    ),
                    ByteLiteralRangeError::Underflow(value) => self.report(
                        "TYPE_BYTE_LITERAL_UNDERFLOW",
                        &format!("Integer literal `{value}` is outside the Byte range 0..255."),
                        file,
                        line,
                    ),
                }
                true
            }
            Type::Float => {
                let Some(range_error) = float_literal_range_error(expression) else {
                    return false;
                };
                match range_error {
                    SignedLiteralRangeError::Overflow(value) => self.report(
                        "TYPE_FLOAT_LITERAL_OVERFLOW",
                        &format!("Numeric literal `{value}` is outside the Float range."),
                        file,
                        line,
                    ),
                    SignedLiteralRangeError::Underflow(value) => self.report(
                        "TYPE_FLOAT_LITERAL_UNDERFLOW",
                        &format!("Numeric literal `{value}` is outside the Float range."),
                        file,
                        line,
                    ),
                }
                true
            }
            Type::Fixed => {
                let Some(range_error) = fixed_literal_range_error(expression) else {
                    return false;
                };
                match range_error {
                    SignedLiteralRangeError::Overflow(value) => self.report(
                        "TYPE_FIXED_LITERAL_OVERFLOW",
                        &format!("Numeric literal `{value}` is outside the Fixed range."),
                        file,
                        line,
                    ),
                    SignedLiteralRangeError::Underflow(value) => self.report(
                        "TYPE_FIXED_LITERAL_UNDERFLOW",
                        &format!("Numeric literal `{value}` is outside the Fixed range."),
                        file,
                        line,
                    ),
                }
                true
            }
            _ => false,
        }
    }
}
