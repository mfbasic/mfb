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
    fn new(project_dir: &'a Path, ast: &'a AstProject) -> Self {
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
    fn collect_close_op_aliases(&mut self) {
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
    fn collect_native_resources(&mut self) {
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
    fn check_resource_decl(&mut self, _file: &AstFile, _resource: &crate::ast::ResourceDecl) {}

    /// Native-specific checks on a `LINK` block: `CPtr` containment and ABI
    /// slot/parameter consistency (plan-link-update.md §5b/§5c/§11/§12).
    fn check_link_block(&mut self, file: &AstFile, link: &crate::ast::LinkBlock) {
        for function in &link.functions {
            self.check_link_function(file, function);
        }
    }

    fn check_link_function(&mut self, file: &AstFile, function: &crate::ast::LinkFunction) {
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

    fn collect_types(&mut self) {
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

    fn collect_package_types(&mut self) {
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
    fn collect_package_resources(
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

    fn validate_imported_package_type(
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

    fn validate_package_metadata_type(
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

    fn collect_package_functions(&mut self) {
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

    fn validate_imported_function_signature(
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

    fn install_package_type_info(
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

    fn package_field_info(&self, field: BinaryReprTypeField) -> FieldInfo {
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

    fn package_variant_info(&self, variant: BinaryReprTypeVariant) -> VariantConstructor {
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

    fn expanded_union_variants(
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

    fn report_expanded_union_member_conflicts(&mut self, file: &AstFile, type_decl: &TypeDecl) {
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

    fn direct_record_successors(&self, name: &str) -> Vec<String> {
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

    fn record_field_cycle(&self, start: &str) -> bool {
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

    fn type_info(&self, file: &AstFile, type_decl: &TypeDecl) -> TypeInfo {
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

    fn field_info(&self, field: &TypeField, containing_visibility: Visibility) -> FieldInfo {
        FieldInfo {
            name: field.name.clone(),
            type_: self.parse_type(&field.type_name),
            visibility: effective_field_visibility(field.visibility, containing_visibility),
        }
    }

    fn collect_bindings(&mut self) {
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

    fn collect_functions(&mut self) {
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
    fn collect_native_functions(&mut self) {
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

    fn native_function_sig(
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

    fn canonical_import_name(&self, file: &AstFile, name: &str) -> String {
        let Some((binding, rest)) = name.split_once('.') else {
            return name.to_string();
        };
        let imports = file.import_bindings();
        let Some(package) = imports.get(binding) else {
            return name.to_string();
        };
        format!("{package}.{rest}")
    }

    fn visible_function_sigs<'b>(&'b self, file: &AstFile, name: &str) -> Vec<&'b FunctionSig> {
        self.functions
            .get(name)
            .into_iter()
            .flatten()
            .filter(|sig| self.visible_from(file, sig.visibility, &sig.owner_file_path))
            .collect()
    }

    fn lookup_visible_function<'b>(
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

    fn lookup_visible_binding<'b>(&'b self, file: &AstFile, name: &str) -> Option<&'b BindingSig> {
        self.bindings
            .get(name)
            .filter(|sig| self.visible_from(file, sig.visibility, &sig.owner_file_path))
    }

    fn lookup_visible_call_sig<'b>(
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

    fn call_shape_matches_sig(&self, arguments: &[CallArg], sig: &FunctionSig) -> bool {
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

    fn check(&mut self) {
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

    fn check_binding(&mut self, file: &AstFile, binding: &TopLevelBinding) {
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

    fn check_type_decl(&mut self, file: &AstFile, type_decl: &TypeDecl) {
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

    fn check_function(&mut self, file: &AstFile, function: &Function) {
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

    fn check_block(
        &mut self,
        file: &AstFile,
        body: &[Statement],
        expected_return: &Type,
        locals: &mut HashMap<String, LocalInfo>,
        trap_name: Option<&str>,
    ) -> Flow {
        for (index, statement) in body.iter().enumerate() {
            let flow = self.check_statement(file, statement, expected_return, locals, trap_name);
            if flow == Flow::AlwaysReturns {
                if matches!(
                    statement,
                    Statement::Exit { .. } | Statement::Continue { .. }
                ) {
                    for unreachable in &body[index + 1..] {
                        self.report(
                            "UNREACHABLE_AFTER_EXIT",
                            "Statement is unreachable after EXIT or CONTINUE.",
                            file,
                            statement_line(unreachable),
                        );
                    }
                }
                return Flow::AlwaysReturns;
            }
        }
        Flow::FallsThrough
    }

    fn merge_branch_locals(
        &self,
        current: &mut HashMap<String, LocalInfo>,
        fallthrough_branches: Vec<HashMap<String, LocalInfo>>,
    ) {
        if fallthrough_branches.is_empty() {
            return;
        }
        let keys = current.keys().cloned().collect::<Vec<_>>();
        for key in keys {
            let mut merged = current.get(&key).cloned();
            for branch in &fallthrough_branches {
                let Some(branch_info) = branch.get(&key) else {
                    continue;
                };
                merged =
                    merged.map(|current_info| self.merge_local_info(current_info, branch_info));
            }
            if let Some(merged) = merged {
                current.insert(key, merged);
            }
        }
    }

    fn merge_local_info(&self, left: LocalInfo, right: &LocalInfo) -> LocalInfo {
        let ownership = match (left.ownership, right.ownership) {
            (OwnershipState::Available, OwnershipState::Available) => OwnershipState::Available,
            (OwnershipState::Moved, OwnershipState::Moved) => OwnershipState::Moved,
            (OwnershipState::MaybeMoved, _) | (_, OwnershipState::MaybeMoved) => {
                OwnershipState::MaybeMoved
            }
            (OwnershipState::Available, OwnershipState::Moved)
            | (OwnershipState::Moved, OwnershipState::Available) => OwnershipState::MaybeMoved,
        };
        LocalInfo {
            type_: left.type_,
            mutable: left.mutable,
            ownership,
            borrowed: left.borrowed,
            state_type: left.state_type,
        }
    }

    fn require_local_owned(
        &mut self,
        file: &AstFile,
        line: usize,
        name: &str,
        info: &LocalInfo,
    ) -> bool {
        match info.ownership {
            OwnershipState::Available => true,
            OwnershipState::Moved => {
                self.report(
                    "TYPE_USE_AFTER_MOVE",
                    &format!("Binding `{name}` was moved and cannot be used again."),
                    file,
                    line,
                );
                false
            }
            OwnershipState::MaybeMoved => {
                self.report(
                    "TYPE_USE_AFTER_MOVE",
                    &format!(
                        "Binding `{name}` may have been moved on another control-flow path and cannot be used here."
                    ),
                    file,
                    line,
                );
                false
            }
        }
    }

    fn consume_local_if_needed(
        &mut self,
        file: &AstFile,
        line: usize,
        name: &str,
        locals: &mut HashMap<String, LocalInfo>,
    ) {
        let Some(info) = locals.get(name).cloned() else {
            return;
        };
        if !self.require_local_owned(file, line, name, &info) {
            return;
        }
        // A borrowed resource cannot be invalidated: close, `RETURN`, and
        // `thread::transfer` all require ownership, which a borrow does not grant.
        if info.borrowed && self.is_resource_type(&info.type_) {
            self.report(
                "TYPE_RESOURCE_BORROW_INVALIDATE",
                &format!(
                    "Binding `{name}` is a borrowed resource; only its owner may close, `RETURN`, or transfer it."
                ),
                file,
                line,
            );
            return;
        }
        if !self.is_copyable_type(&info.type_) {
            if let Some(local) = locals.get_mut(name) {
                local.ownership = OwnershipState::Moved;
            }
        }
    }

    /// Enforce the `RES` ownership axis: the `RES` keyword must be present
    /// exactly when the declared type is a resource, and any `STATE T` must be a
    /// copyable, defaultable data type. `context` labels the declaration site.
    fn check_resource_declaration(
        &mut self,
        file: &AstFile,
        line: usize,
        resource: bool,
        state_type: Option<&str>,
        declared: Option<&Type>,
        context: &str,
    ) {
        let is_resource = declared.is_some_and(|type_| self.is_resource_type(type_));
        if is_resource && !resource {
            let type_name = declared.map(|t| self.type_name(t)).unwrap_or_default();
            self.report(
                "TYPE_RESOURCE_REQUIRES_RES",
                &format!(
                    "{context} holds resource `{type_name}`; bind it with `RES`, not `LET`/`MUT`."
                ),
                file,
                line,
            );
        } else if resource && declared.is_some() && !is_resource {
            let type_name = self.type_name(declared.unwrap());
            self.report(
                "TYPE_RES_REQUIRES_RESOURCE",
                &format!(
                    "{context} is declared `RES` but `{type_name}` is not a resource type; use `LET`/`MUT`."
                ),
                file,
                line,
            );
        }

        if let Some(state) = state_type {
            // A resource union abstracts over *which* resource it holds, so a
            // union-level STATE is undefined — it would vary by tag and be absent
            // for stateless variants. STATE belongs to one concrete resource.
            let on_resource_union =
                matches!(declared, Some(Type::User(name)) if self.is_resource_union(name));
            if on_resource_union {
                let type_name = self.type_name(declared.unwrap());
                self.report(
                    "TYPE_UNION_STATE_FORBIDDEN",
                    &format!(
                        "{context} attaches STATE to resource union `{type_name}`; a resource union carries no STATE — use a concrete stateful resource."
                    ),
                    file,
                    line,
                );
            }
            let state_resolved = self.parse_type(state);
            self.check_type_reference(file, &state_resolved, line);
            if !self.is_copyable_type(&state_resolved) || !self.is_defaultable_type(&state_resolved)
            {
                self.report(
                    "TYPE_STATE_INVALID",
                    &format!(
                        "{context} STATE type `{state}` must be a copyable, defaultable data type."
                    ),
                    file,
                    line,
                );
            }
        }
    }

    fn check_binding_shape(
        &mut self,
        file: &AstFile,
        name: &str,
        mutable: bool,
        line: usize,
        declared: Option<&Type>,
        inferred: Option<&Type>,
        value: Option<&Expression>,
    ) {
        if matches!(inferred, Some(Type::Unknown)) {
            self.report(
                "TYPE_UNKNOWN_VALUE",
                &format!("Initializer for binding `{name}` does not have a known type."),
                file,
                line,
            );
        }

        let reported_range_error = declared.zip(value).is_some_and(|(declared, value)| {
            self.report_primitive_literal_range_error(declared, value, file, line)
        });

        match (declared, inferred) {
            (Some(declared), Some(inferred))
                if !reported_range_error
                    && !self.expression_compatible(declared, inferred, value) =>
            {
                self.report(
                    "TYPE_BINDING_MISMATCH",
                    &format!(
                        "Binding `{name}` has initializer type {}, expected {}.",
                        self.type_name(inferred),
                        self.type_name(declared)
                    ),
                    file,
                    line,
                );
            }
            (None, None) => {
                self.report(
                    "TYPE_BINDING_REQUIRES_TYPE_OR_VALUE",
                    &format!("Binding `{name}` needs a type annotation or initializer."),
                    file,
                    line,
                );
            }
            (Some(_), None) if !mutable => {
                self.report(
                    "TYPE_LET_REQUIRES_VALUE",
                    &format!("Immutable binding `{name}` must have an initializer."),
                    file,
                    line,
                );
            }
            (Some(declared), None) if mutable && !self.is_defaultable_type(declared) => {
                self.report(
                    "TYPE_MUT_REQUIRES_DEFAULTABLE_TYPE",
                    &format!(
                        "Mutable binding `{name}` cannot omit its initializer because type `{}` does not have a defined default value.",
                        self.type_name(declared)
                    ),
                    file,
                    line,
                );
            }
            _ => {}
        }
    }

    fn check_statement(
        &mut self,
        file: &AstFile,
        statement: &Statement,
        expected_return: &Type,
        locals: &mut HashMap<String, LocalInfo>,
        trap_name: Option<&str>,
    ) -> Flow {
        match statement {
            Statement::Let {
                name,
                type_name,
                value,
                line,
                mutable,
                resource,
                state_type,
            } => {
                let declared = type_name.as_deref().map(|name| self.parse_type(name));
                if let Some(declared) = &declared {
                    self.check_type_reference(file, declared, *line);
                }
                let inferred = value.as_ref().map(|value| {
                    self.infer_expression_with_expected(
                        file,
                        value,
                        locals,
                        *line,
                        declared.as_ref(),
                        ExprMode::Transfer,
                    )
                });

                self.check_binding_shape(
                    file,
                    name,
                    *mutable,
                    *line,
                    declared.as_ref(),
                    inferred.as_ref(),
                    value.as_ref(),
                );

                let binding_type = declared.or(inferred).unwrap_or(Type::Unknown);
                self.check_resource_declaration(
                    file,
                    *line,
                    *resource,
                    state_type.as_deref(),
                    (binding_type != Type::Unknown).then_some(&binding_type),
                    &format!("binding `{name}`"),
                );
                // A `get`/`getOr` of a resource element yields a *borrow*, not an
                // owner; it cannot be bound with `RES` (§15.6). Use it inline or
                // through `FOR EACH`.
                if *resource
                    && self.is_resource_type(&binding_type)
                    && value
                        .as_ref()
                        .is_some_and(|value| is_resource_element_borrow(value))
                {
                    self.report(
                        "TYPE_RESOURCE_ELEMENT_NOT_OWNER",
                        &format!(
                            "Binding `{name}` is a borrowed collection element, not an owner; a borrowed resource cannot be bound with `RES`. Use it inline or via `FOR EACH` (§15.6)."
                        ),
                        file,
                        *line,
                    );
                }
                // A `RES` binding whose ownership floats into an outer-scope
                // collection (or out via a returned collection) becomes
                // borrow-only: it may not close, `RETURN`, or transfer the
                // resource — the owning scope does that (§15.6).
                let borrowed = *resource
                    && self.is_resource_type(&binding_type)
                    && self.current_resource_owners.floats(name);
                locals.insert(
                    name.clone(),
                    LocalInfo {
                        type_: binding_type,
                        mutable: *mutable,
                        ownership: OwnershipState::Available,
                        borrowed,
                        state_type: state_type.clone(),
                    },
                );
                Flow::FallsThrough
            }
            Statement::Return { value, line } => {
                if self.current_is_sub {
                    if let Some(value) = value {
                        self.infer_expression(file, value, locals, *line, ExprMode::Transfer);
                    }
                    self.report(
                        "SUB_RETURN_FORBIDDEN",
                        "A SUB returns no value; use `EXIT SUB`.",
                        file,
                        *line,
                    );
                    return Flow::AlwaysReturns;
                }
                let actual = value
                    .as_ref()
                    .map(|value| {
                        self.infer_expression_with_expected(
                            file,
                            value,
                            locals,
                            *line,
                            Some(expected_return),
                            ExprMode::Transfer,
                        )
                    })
                    .unwrap_or(Type::Nothing);
                if matches!(actual, Type::Unknown) {
                    self.report(
                        "TYPE_UNKNOWN_VALUE",
                        "RETURN value does not have a known type.",
                        file,
                        *line,
                    );
                }
                // A `get`/`getOr` of a resource element is a borrow, not an
                // owner; it cannot be returned (§15.6).
                if self.is_resource_type(&actual)
                    && value
                        .as_ref()
                        .is_some_and(|value| is_resource_element_borrow(value))
                {
                    self.report(
                        "TYPE_RESOURCE_ELEMENT_NOT_OWNER",
                        "RETURN value is a borrowed collection element, not an owner; a borrowed resource cannot be returned (§15.6).",
                        file,
                        *line,
                    );
                }
                if !self.expression_compatible(expected_return, &actual, value.as_ref()) {
                    self.report(
                        "TYPE_RETURN_MISMATCH",
                        &format!(
                            "RETURN has type {}, expected {}.",
                            self.type_name(&actual),
                            self.type_name(expected_return)
                        ),
                        file,
                        *line,
                    );
                }
                Flow::AlwaysReturns
            }
            Statement::Exit { target, code, line } => {
                match target {
                    ExitTarget::For | ExitTarget::Do | ExitTarget::While => {
                        let kind = match target {
                            ExitTarget::For => LoopKind::For,
                            ExitTarget::Do => LoopKind::Do,
                            ExitTarget::While => LoopKind::While,
                            _ => unreachable!(),
                        };
                        if !self.loop_stack.iter().rev().any(|item| *item == kind) {
                            self.report(
                                "EXIT_NO_MATCHING_LOOP",
                                &format!(
                                    "EXIT {} has no matching enclosing loop.",
                                    loop_kind_keyword(kind)
                                ),
                                file,
                                *line,
                            );
                        }
                    }
                    ExitTarget::Sub => {
                        if !self.current_is_sub {
                            self.report(
                                "EXIT_SUB_IN_FUNC",
                                "EXIT SUB is valid only inside a SUB; use RETURN <value> in a FUNC.",
                                file,
                                *line,
                            );
                        }
                    }
                    ExitTarget::Func => {
                        self.report(
                            "EXIT_FUNC_FORBIDDEN",
                            "Functions must RETURN a value; EXIT FUNC is not allowed.",
                            file,
                            *line,
                        );
                    }
                    ExitTarget::Program => {
                        let Some(code) = code else {
                            self.report(
                                "TYPE_UNKNOWN_VALUE",
                                "EXIT PROGRAM requires an Integer exit code.",
                                file,
                                *line,
                            );
                            return Flow::AlwaysReturns;
                        };
                        let actual =
                            self.infer_expression(file, code, locals, *line, ExprMode::Read);
                        if !self.expression_compatible(&Type::Integer, &actual, Some(code)) {
                            self.report(
                                "TYPE_EXIT_PROGRAM_REQUIRES_INTEGER",
                                &format!(
                                    "EXIT PROGRAM code has type {}, expected Integer.",
                                    self.type_name(&actual)
                                ),
                                file,
                                *line,
                            );
                        }
                        if let Some(value) = integer_constant_value(code) {
                            if !(0..=255).contains(&value) {
                                self.report(
                                    "EXIT_PROGRAM_CODE_OUT_OF_RANGE",
                                    "EXIT PROGRAM constant exit code must be in the host range 0..255.",
                                    file,
                                    *line,
                                );
                            }
                        }
                    }
                }
                Flow::AlwaysReturns
            }
            Statement::Continue { kind, line } => {
                if !self.loop_stack.iter().rev().any(|item| *item == *kind) {
                    self.report(
                        "CONTINUE_NO_MATCHING_LOOP",
                        &format!(
                            "CONTINUE {} has no matching enclosing loop.",
                            loop_kind_keyword(*kind)
                        ),
                        file,
                        *line,
                    );
                }
                Flow::AlwaysReturns
            }
            Statement::Fail { error, line } => {
                let actual = self.infer_expression(file, error, locals, *line, ExprMode::Transfer);
                if !self.compatible(&Type::Error, &actual) {
                    self.report(
                        "TYPE_FAIL_REQUIRES_ERROR",
                        &format!("FAIL has type {}, expected Error.", self.type_name(&actual)),
                        file,
                        *line,
                    );
                }
                Flow::AlwaysReturns
            }
            Statement::Propagate { line } => {
                if trap_name.is_none() {
                    self.report(
                        "TYPE_PROPAGATE_REQUIRES_TRAP",
                        "PROPAGATE is valid only inside a TRAP.",
                        file,
                        *line,
                    );
                }
                Flow::AlwaysReturns
            }
            Statement::Recover { value, line } => {
                let Some(recover_type) = self.inline_trap_types.last().cloned() else {
                    if let Some(value) = value {
                        self.infer_expression(file, value, locals, *line, ExprMode::Read);
                    }
                    self.report(
                        "TYPE_RECOVER_OUTSIDE_INLINE_TRAP",
                        "RECOVER is valid only inside an inline TRAP handler.",
                        file,
                        *line,
                    );
                    return Flow::AlwaysReturns;
                };
                let produces_value = !matches!(recover_type, Type::Nothing);
                match (value, produces_value) {
                    (Some(value), true) => {
                        let actual = self.infer_expression_with_expected(
                            file,
                            value,
                            locals,
                            *line,
                            Some(&recover_type),
                            ExprMode::Transfer,
                        );
                        if !self.expression_compatible(&recover_type, &actual, Some(value)) {
                            self.report(
                                "TYPE_RECOVER_TYPE_MISMATCH",
                                &format!(
                                    "RECOVER has type {}, expected {}.",
                                    self.type_name(&actual),
                                    self.type_name(&recover_type)
                                ),
                                file,
                                *line,
                            );
                        }
                    }
                    (None, true) => {
                        self.report(
                            "TYPE_RECOVER_TYPE_MISMATCH",
                            &format!(
                                "RECOVER must supply a {} value for the trapped expression.",
                                self.type_name(&recover_type)
                            ),
                            file,
                            *line,
                        );
                    }
                    (Some(value), false) => {
                        self.infer_expression(file, value, locals, *line, ExprMode::Read);
                        self.report(
                            "TYPE_RECOVER_TYPE_MISMATCH",
                            "RECOVER must not supply a value for a value-less trapped expression.",
                            file,
                            *line,
                        );
                    }
                    (None, false) => {}
                }
                Flow::AlwaysReturns
            }
            Statement::Assign { name, value, line } => {
                let Some(local) = locals.get(name).cloned() else {
                    if let Some(binding) = self.lookup_visible_binding(file, name).cloned() {
                        if !binding.mutable {
                            self.report(
                                "TYPE_ASSIGN_REQUIRES_MUT",
                                &format!("Binding `{name}` is immutable and cannot be assigned."),
                                file,
                                *line,
                            );
                        }
                        let actual =
                            self.infer_expression(file, value, locals, *line, ExprMode::Transfer);
                        let reported_range_error = self.report_primitive_literal_range_error(
                            &binding.type_,
                            value,
                            file,
                            *line,
                        );
                        if !reported_range_error
                            && !self.expression_compatible(&binding.type_, &actual, Some(value))
                        {
                            self.report(
                                "TYPE_ASSIGNMENT_MISMATCH",
                                &format!(
                                    "Assignment to `{name}` has type {}, expected {}.",
                                    self.type_name(&actual),
                                    self.type_name(&binding.type_)
                                ),
                                file,
                                *line,
                            );
                        }
                        return Flow::FallsThrough;
                    }
                    self.report(
                        "TYPE_UNKNOWN_VALUE",
                        &format!("Assignment target `{name}` is not a local binding."),
                        file,
                        *line,
                    );
                    return Flow::FallsThrough;
                };
                if !local.mutable {
                    self.report(
                        "TYPE_ASSIGN_REQUIRES_MUT",
                        &format!("Binding `{name}` is immutable and cannot be assigned."),
                        file,
                        *line,
                    );
                }
                let actual = self.infer_expression(file, value, locals, *line, ExprMode::Transfer);
                if !self.require_local_owned(file, *line, name, &local) {
                    return Flow::FallsThrough;
                }
                let reported_range_error =
                    self.report_primitive_literal_range_error(&local.type_, value, file, *line);
                if !reported_range_error
                    && !self.expression_compatible(&local.type_, &actual, Some(value))
                {
                    self.report(
                        "TYPE_ASSIGNMENT_MISMATCH",
                        &format!(
                            "Assignment to `{name}` has type {}, expected {}.",
                            self.type_name(&actual),
                            self.type_name(&local.type_)
                        ),
                        file,
                        *line,
                    );
                }
                Flow::FallsThrough
            }
            Statement::StateAssign {
                resource,
                value,
                line,
            } => {
                let Some(local) = locals.get(resource).cloned() else {
                    self.report(
                        "TYPE_UNKNOWN_VALUE",
                        &format!("State assignment target `{resource}` is not a local binding."),
                        file,
                        *line,
                    );
                    self.infer_expression(file, value, locals, *line, ExprMode::Read);
                    return Flow::FallsThrough;
                };
                let Some(state_name) = local.state_type.clone() else {
                    self.report(
                        "TYPE_STATE_INVALID",
                        &format!(
                            "`{resource}` has no STATE to assign; declare the resource with `STATE T`."
                        ),
                        file,
                        *line,
                    );
                    self.infer_expression(file, value, locals, *line, ExprMode::Read);
                    return Flow::FallsThrough;
                };
                // Both the owner and a borrower may mutate STATE; only liveness
                // (not ownership) is required.
                if !self.require_local_owned(file, *line, resource, &local) {
                    return Flow::FallsThrough;
                }
                let state_type = self.parse_type(&state_name);
                let actual = self.infer_expression_with_expected(
                    file,
                    value,
                    locals,
                    *line,
                    Some(&state_type),
                    ExprMode::Transfer,
                );
                if !self.expression_compatible(&state_type, &actual, Some(value)) {
                    self.report(
                        "TYPE_ASSIGNMENT_MISMATCH",
                        &format!(
                            "State assignment to `{resource}.state` has type {}, expected {}.",
                            self.type_name(&actual),
                            self.type_name(&state_type)
                        ),
                        file,
                        *line,
                    );
                }
                Flow::FallsThrough
            }
            Statement::Expression { expression, line } => {
                // A bare expression statement is the one position where a
                // value-less `SUB` call is allowed (it discards no value).
                self.allow_value_less_call = true;
                self.infer_expression(file, expression, locals, *line, ExprMode::Read);
                self.allow_value_less_call = false;
                Flow::FallsThrough
            }
            Statement::If {
                condition,
                then_body,
                else_body,
                line,
            } => {
                let condition_type =
                    self.infer_expression(file, condition, locals, *line, ExprMode::Read);
                if !self.expression_compatible(&Type::Boolean, &condition_type, Some(condition)) {
                    self.report(
                        "TYPE_CONDITION_REQUIRES_BOOLEAN",
                        &format!(
                            "IF condition has type {}, expected Boolean.",
                            self.type_name(&condition_type)
                        ),
                        file,
                        *line,
                    );
                }
                let mut then_locals = locals.clone();
                let then_flow = self.check_block(
                    file,
                    then_body,
                    expected_return,
                    &mut then_locals,
                    trap_name,
                );
                let mut else_locals = locals.clone();
                let else_flow = self.check_block(
                    file,
                    else_body,
                    expected_return,
                    &mut else_locals,
                    trap_name,
                );
                let mut fallthroughs = Vec::new();
                if then_flow == Flow::FallsThrough {
                    fallthroughs.push(then_locals);
                }
                if else_flow == Flow::FallsThrough {
                    fallthroughs.push(else_locals);
                } else if else_body.is_empty() {
                    fallthroughs.push(locals.clone());
                }
                if then_flow == Flow::AlwaysReturns
                    && else_flow == Flow::AlwaysReturns
                    && !else_body.is_empty()
                {
                    Flow::AlwaysReturns
                } else {
                    self.merge_branch_locals(locals, fallthroughs);
                    Flow::FallsThrough
                }
            }
            Statement::Match {
                expression,
                cases,
                line,
            } => {
                let send_failure_restore =
                    self.thread_send_failure_restore(file, expression, locals);
                let matched_type = self.infer_match_scrutinee(file, expression, locals, *line);
                let mut has_unguarded_else = false;
                let mut all_return = !cases.is_empty();
                let mut covered_cases = HashSet::new();
                let mut fallthroughs = Vec::new();
                for case in cases {
                    if case.guard.is_none() {
                        if matches!(case.pattern, MatchPattern::Else) {
                            has_unguarded_else = true;
                        } else if let Some(name) = self.match_case_name(&case.pattern) {
                            covered_cases.insert(name);
                        }
                    }
                    let mut case_locals = locals.clone();
                    if matches!(
                        (&case.pattern, &send_failure_restore),
                        (
                            MatchPattern::Union { type_name, .. },
                            Some((_, _))
                        ) if type_name == "Error"
                    ) {
                        if let Some((name, info)) = &send_failure_restore {
                            case_locals.insert(name.clone(), info.clone());
                        }
                    }
                    self.check_match_pattern(
                        file,
                        &case.pattern,
                        &matched_type,
                        &mut case_locals,
                        case.line,
                    );
                    if let Some(guard) = &case.guard {
                        let guard_type = self.infer_expression(
                            file,
                            guard,
                            &mut case_locals,
                            case.line,
                            ExprMode::Read,
                        );
                        if !self.expression_compatible(&Type::Boolean, &guard_type, Some(guard)) {
                            self.report(
                                "TYPE_CONDITION_REQUIRES_BOOLEAN",
                                &format!(
                                    "WHEN guard has type {}, expected Boolean.",
                                    self.type_name(&guard_type)
                                ),
                                file,
                                case.line,
                            );
                        }
                    }
                    let case_flow = self.check_block(
                        file,
                        &case.body,
                        expected_return,
                        &mut case_locals,
                        trap_name,
                    );
                    all_return &= case_flow == Flow::AlwaysReturns;
                    if case_flow == Flow::FallsThrough {
                        fallthroughs.push(case_locals);
                    }
                }
                let exhaustive =
                    has_unguarded_else || self.match_is_exhaustive(&matched_type, &covered_cases);
                // A `Result` scrutinee can only arise from an already-rejected
                // type annotation; its `CASE Ok`/`CASE Error` arms already
                // reported `TYPE_RESULT_NOT_MATCHABLE`, so suppress the secondary
                // exhaustiveness cascade.
                if !exhaustive && !matches!(matched_type, Type::Unknown | Type::Result(_)) {
                    self.report_match_not_exhaustive(file, *line, &matched_type, &covered_cases);
                }
                if all_return && exhaustive {
                    Flow::AlwaysReturns
                } else {
                    if !exhaustive {
                        fallthroughs.push(locals.clone());
                    }
                    self.merge_branch_locals(locals, fallthroughs);
                    Flow::FallsThrough
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
                let start_type = self.infer_expression(file, start, locals, *line, ExprMode::Read);
                let end_type = self.infer_expression(file, end, locals, *line, ExprMode::Read);
                let step_type = match step {
                    Some(step) => self.infer_expression(file, step, locals, *line, ExprMode::Read),
                    None => Type::Integer,
                };
                let numeric_types = [&start_type, &end_type, &step_type];
                let all_numeric = numeric_types.iter().all(|type_| self.is_numeric(type_));
                let loop_type = if all_numeric {
                    promote_loop_numeric_type(&start_type, &end_type, &step_type)
                } else {
                    for (label, type_) in [
                        ("start", &start_type),
                        ("end", &end_type),
                        ("step", &step_type),
                    ] {
                        if !self.is_numeric(type_) {
                            self.report(
                                "TYPE_FOR_REQUIRES_NUMERIC",
                                &format!(
                                    "FOR loop {label} value has type {}, expected numeric.",
                                    self.type_name(type_)
                                ),
                                file,
                                *line,
                            );
                        }
                    }
                    Type::Unknown
                };
                if let Some(step) = step {
                    if numeric_literal_is_zero(step) {
                        self.report(
                            "TYPE_FOR_STEP_ZERO",
                            "FOR loop STEP must not be zero.",
                            file,
                            *line,
                        );
                    }
                }
                let mut nested = locals.clone();
                nested.insert(
                    name.clone(),
                    LocalInfo {
                        type_: loop_type,
                        mutable: false,
                        ownership: OwnershipState::Available,
                        borrowed: false,
                        state_type: None,
                    },
                );
                self.loop_stack.push(LoopKind::For);
                let body_flow =
                    self.check_block(file, body, expected_return, &mut nested, trap_name);
                self.loop_stack.pop();
                if body_flow == Flow::FallsThrough {
                    self.merge_branch_locals(locals, vec![nested]);
                }
                Flow::FallsThrough
            }
            Statement::ForEach {
                name,
                iterable,
                body,
                line,
            } => {
                let iterable_type =
                    self.infer_expression(file, iterable, locals, *line, ExprMode::Read);
                let element_type = match iterable_type {
                    // Iterating `List OF RES File` yields a *borrow* of each
                    // element (`File`), not the `RES`-marked slot type (§15.6).
                    Type::List(element) => strip_res(&element).clone(),
                    Type::Map(key, value) => Type::User(format!(
                        "MapEntry OF {} TO {}",
                        self.type_name(&key),
                        self.type_name(strip_res(&value))
                    )),
                    other => {
                        self.report(
                            "TYPE_FOR_EACH_REQUIRES_COLLECTION",
                            &format!(
                                "FOR EACH source has type {}, expected List or Map.",
                                self.type_name(&other)
                            ),
                            file,
                            *line,
                        );
                        Type::Unknown
                    }
                };
                // Iterating a resource collection yields a *borrow* of each
                // element; the loop variable may not close, `RETURN`, or transfer
                // the resource (§15.6).
                let element_borrowed = self.is_resource_type(&element_type);
                let mut nested = locals.clone();
                nested.insert(
                    name.clone(),
                    LocalInfo {
                        type_: element_type,
                        mutable: false,
                        ownership: OwnershipState::Available,
                        borrowed: element_borrowed,
                        state_type: None,
                    },
                );
                self.loop_stack.push(LoopKind::For);
                let body_flow =
                    self.check_block(file, body, expected_return, &mut nested, trap_name);
                self.loop_stack.pop();
                if body_flow == Flow::FallsThrough {
                    self.merge_branch_locals(locals, vec![nested]);
                }
                Flow::FallsThrough
            }
            Statement::While {
                kind,
                condition,
                body,
                line,
            } => {
                let condition_type =
                    self.infer_expression(file, condition, locals, *line, ExprMode::Read);
                if !self.expression_compatible(&Type::Boolean, &condition_type, Some(condition)) {
                    self.report(
                        "TYPE_CONDITION_REQUIRES_BOOLEAN",
                        &format!(
                            "WHILE condition has type {}, expected Boolean.",
                            self.type_name(&condition_type)
                        ),
                        file,
                        *line,
                    );
                }
                let mut nested = locals.clone();
                self.loop_stack.push(*kind);
                let body_flow =
                    self.check_block(file, body, expected_return, &mut nested, trap_name);
                self.loop_stack.pop();
                if body_flow == Flow::FallsThrough {
                    self.merge_branch_locals(locals, vec![nested]);
                }
                Flow::FallsThrough
            }
            Statement::DoUntil {
                body,
                condition,
                line,
            } => {
                let mut nested = locals.clone();
                self.loop_stack.push(LoopKind::Do);
                let body_flow =
                    self.check_block(file, body, expected_return, &mut nested, trap_name);
                self.loop_stack.pop();
                let condition_type =
                    self.infer_expression(file, condition, locals, *line, ExprMode::Read);
                if !self.expression_compatible(&Type::Boolean, &condition_type, Some(condition)) {
                    self.report(
                        "TYPE_CONDITION_REQUIRES_BOOLEAN",
                        &format!(
                            "LOOP UNTIL condition has type {}, expected Boolean.",
                            self.type_name(&condition_type)
                        ),
                        file,
                        *line,
                    );
                }
                if body_flow == Flow::FallsThrough {
                    self.merge_branch_locals(locals, vec![nested]);
                }
                Flow::FallsThrough
            }
        }
    }

    fn infer_expression(
        &mut self,
        file: &AstFile,
        expression: &Expression,
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
        mode: ExprMode,
    ) -> Type {
        self.infer_expression_with_expected(file, expression, locals, line, None, mode)
    }

    fn infer_expression_with_expected(
        &mut self,
        file: &AstFile,
        expression: &Expression,
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
        expected: Option<&Type>,
        mode: ExprMode,
    ) -> Type {
        // A value-less `SUB` call is permitted only as the top expression of a
        // bare statement (or the inner call of an inline `TRAP` there). Consume
        // the permission here so it applies to this expression alone; nested
        // sub-expressions see it reset to false and reject `SUB` calls.
        let value_less_call_ok = self.allow_value_less_call;
        self.allow_value_less_call = false;
        match expression {
            Expression::String(_) => Type::String,
            Expression::Boolean(_) => Type::Boolean,
            Expression::Number(value) => {
                if value.contains('.') {
                    Type::Float
                } else if value.parse::<i64>().is_ok() {
                    Type::Integer
                } else {
                    self.report(
                        "TYPE_INTEGER_LITERAL_OVERFLOW",
                        &format!("Integer literal `{value}` is outside the Integer range."),
                        file,
                        line,
                    );
                    Type::Integer
                }
            }
            Expression::Identifier(name) if name == "NOTHING" => Type::Nothing,
            Expression::Identifier(name) => {
                let canonical_name = self.canonical_import_name(file, name);
                if canonical_name == "NOTHING" {
                    Type::Nothing
                } else if builtins::is_package_constant(&canonical_name) {
                    self.parse_type(
                        builtins::package_constant_type_name(&canonical_name).unwrap_or("Unknown"),
                    )
                } else {
                    if let Some(local) = locals.get(name).cloned() {
                        self.require_local_owned(file, line, name, &local);
                        if matches!(mode, ExprMode::Transfer) {
                            self.consume_local_if_needed(file, line, name, locals);
                        }
                        local.type_
                    } else {
                        self.lookup_visible_function(file, name)
                            .map(function_type)
                            .or_else(|| {
                                self.lookup_visible_binding(file, name)
                                    .map(|binding| binding.type_.clone())
                            })
                            .or_else(|| {
                                self.lookup_visible_function(file, &canonical_name)
                                    .map(function_type)
                            })
                            .unwrap_or(Type::Unknown)
                    }
                }
            }
            Expression::Constructor {
                type_name,
                arguments,
            } => self.infer_constructor(file, type_name, arguments, locals, line, expected),
            Expression::WithUpdate { target, updates } => {
                self.infer_with_update(file, target, updates, locals, line)
            }
            Expression::MemberAccess { target, member } => {
                self.infer_member_access(file, target, member, locals, line)
            }
            Expression::Trapped {
                expression: inner,
                binding,
                handler,
                line: trap_line,
            } => {
                let trapped_callee = match inner.as_ref() {
                    Expression::Call { callee, .. } => {
                        Some(self.canonical_import_name(file, callee))
                    }
                    _ => None,
                };
                let fallible = match &trapped_callee {
                    Some(canonical) => !builtins::is_package_constant(canonical),
                    None => false,
                };
                // A failed `thread.send` returns ownership of the sent value to
                // the caller so the handler can release it. Capture it before
                // the call consumes it, then restore it into the handler scope.
                let send_failure_restore = self.thread_send_failure_restore(file, inner, locals);
                // A trapped `SUB` call as a bare statement is value-less too;
                // forward the permission to the inner call.
                self.allow_value_less_call = value_less_call_ok;
                let success_type =
                    self.infer_expression_with_expected(file, inner, locals, line, expected, mode);
                if !fallible {
                    self.report(
                        "TYPE_INLINE_TRAP_REQUIRES_FALLIBLE",
                        "Inline TRAP requires a fallible call; this expression cannot fail.",
                        file,
                        *trap_line,
                    );
                }
                // An inline-lowered built-in (string/collection member, `bits::*`
                // op, or `len`/`toString`/`typeName`) has its code spliced in at
                // the call site and owns no callable symbol, so codegen's raw-TRAP
                // path cannot trap it — it would emit a `bl` to a missing symbol.
                // Reject it here with a located diagnostic and the workaround.
                // Report-and-continue so the rest of the expression still checks.
                if fallible {
                    if let Some(canonical) = &trapped_callee {
                        if builtins::inline_trap_unsupported(canonical) {
                            self.report(
                                "TYPE_INLINE_TRAP_ON_INLINED_BUILTIN",
                                &format!(
                                    "Inline TRAP is not supported on `{canonical}` (it is compiled inline). Move the call into a FUNC/SUB and TRAP on that call instead."
                                ),
                                file,
                                *trap_line,
                            );
                        }
                    }
                }
                let mut handler_locals = locals.clone();
                if let Some((name, info)) = send_failure_restore {
                    handler_locals.insert(name, info);
                }
                handler_locals.insert(
                    binding.clone(),
                    LocalInfo {
                        type_: Type::Error,
                        mutable: false,
                        ownership: OwnershipState::Available,
                        borrowed: false,
                        state_type: None,
                    },
                );
                self.inline_trap_types.push(success_type.clone());
                let current_return = self.current_return.clone();
                let handler_flow = self.check_block(
                    file,
                    handler,
                    &current_return,
                    &mut handler_locals,
                    Some(binding.as_str()),
                );
                self.inline_trap_types.pop();
                if handler_flow != Flow::AlwaysReturns {
                    self.report(
                        "TYPE_INLINE_TRAP_FALLS_THROUGH",
                        "Inline TRAP handler must end every path in RECOVER or a diverging statement (RETURN, FAIL, or PROPAGATE).",
                        file,
                        *trap_line,
                    );
                }
                success_type
            }
            Expression::Binary {
                left,
                operator,
                right,
                ..
            } => {
                let left_type = self.infer_expression(file, left, locals, line, ExprMode::Read);
                let right_type = self.infer_expression(file, right, locals, line, ExprMode::Read);
                self.infer_binary(file, operator, &left_type, &right_type, line)
            }
            Expression::Unary {
                operator, operand, ..
            } => {
                if operator == "-" && !integer_literal_in_range(expression) {
                    if let Expression::Number(value) = operand.as_ref() {
                        self.report(
                            "TYPE_INTEGER_LITERAL_OVERFLOW",
                            &format!("Integer literal `-{value}` is outside the Integer range."),
                            file,
                            line,
                        );
                    }
                    return Type::Integer;
                }
                if operator == "-"
                    && matches!(operand.as_ref(), Expression::Number(value) if !value.contains('.'))
                {
                    return Type::Integer;
                }
                let operand_type =
                    self.infer_expression(file, operand, locals, line, ExprMode::Read);
                self.infer_unary(file, operator, &operand_type, line)
            }
            Expression::Call {
                callee, arguments, ..
            } => {
                let canonical_callee = self.canonical_import_name(file, callee);
                if builtins::is_package_constant(&canonical_callee) {
                    self.report(
                        "SYMBOL_NOT_CALLABLE",
                        &format!("Package constant `{callee}` is not callable."),
                        file,
                        line,
                    );
                    for argument in arguments {
                        self.infer_expression(
                            file,
                            call_arg_value(argument),
                            locals,
                            line,
                            ExprMode::Read,
                        );
                    }
                    return self.parse_type(
                        builtins::package_constant_type_name(&canonical_callee)
                            .unwrap_or("Unknown"),
                    );
                }
                if builtins::is_builtin_call(&canonical_callee) {
                    return self.check_builtin_call(
                        file,
                        callee,
                        &canonical_callee,
                        arguments,
                        locals,
                        line,
                        expected,
                    );
                }

                if let Some(sig) = self
                    .lookup_visible_call_sig(file, callee, arguments, expected)
                    .cloned()
                    .or_else(|| {
                        self.lookup_visible_call_sig(file, &canonical_callee, arguments, expected)
                            .cloned()
                    })
                {
                    self.check_call(file, callee, &sig, arguments, locals, line);
                    if matches!(sig.kind, FunctionKind::Sub) && !value_less_call_ok {
                        self.report(
                            "TYPE_SUB_HAS_NO_VALUE",
                            &format!(
                                "SUB `{callee}` produces no value; its call is a statement, \
                                 not an expression."
                            ),
                            file,
                            line,
                        );
                    }
                    return sig.return_type;
                }

                if callee.contains('.') {
                    for argument in arguments {
                        self.infer_expression(
                            file,
                            call_arg_value(argument),
                            locals,
                            line,
                            ExprMode::Read,
                        );
                    }
                    return Type::Unknown;
                }

                if let Some(local) = locals.get(callee).cloned() {
                    return self.check_function_value_call(
                        file,
                        callee,
                        &local.type_,
                        arguments,
                        locals,
                        line,
                    );
                }

                Type::Unknown
            }
            Expression::Lambda {
                params,
                body,
                assign_target,
            } => self.infer_lambda(file, params, body, assign_target.as_deref(), locals, line),
            Expression::ListLiteral(values) => {
                self.infer_list_literal(file, values, locals, line, expected)
            }
            Expression::MapLiteral {
                key_type,
                value_type,
                entries,
            } => self.infer_map_literal(file, key_type, value_type, entries, locals, line),
        }
    }

    fn infer_match_scrutinee(
        &mut self,
        file: &AstFile,
        expression: &Expression,
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        // A call scrutinee auto-unwraps like every other call site (its `Ok`
        // value is matched). Local error handling now uses an inline `TRAP`
        // (§8.4); `MATCH` only matches enum/union/`Result` *values*. A
        // `Result`-typed value (a local or field) still infers to
        // `Type::Result(..)`, preserving `CASE Ok`/`CASE Error` matching.
        self.infer_expression(file, expression, locals, line, ExprMode::Read)
    }

    fn thread_send_failure_restore(
        &self,
        file: &AstFile,
        expression: &Expression,
        locals: &HashMap<String, LocalInfo>,
    ) -> Option<(String, LocalInfo)> {
        let Expression::Call {
            callee, arguments, ..
        } = expression
        else {
            return None;
        };
        // Both `thread.send` and the resource-plane `thread.transfer` move on
        // success and return ownership to the sender on failure, so a `TRAP`
        // handler may use the binding again.
        let canonical = self.canonical_import_name(file, callee);
        if canonical != "thread.send" && canonical != "thread.transfer" {
            return None;
        }
        let Some(argument) = arguments.get(1).map(call_arg_value) else {
            return None;
        };
        let Expression::Identifier(name) = argument else {
            return None;
        };
        let info = locals.get(name)?;
        if info.ownership != OwnershipState::Available || self.is_copyable_type(&info.type_) {
            return None;
        }
        Some((name.clone(), info.clone()))
    }

    fn check_match_pattern(
        &mut self,
        file: &AstFile,
        pattern: &MatchPattern,
        matched_type: &Type,
        case_locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) {
        match pattern {
            MatchPattern::Else => {}
            MatchPattern::Literal(expression) => {
                let pattern_type =
                    self.infer_expression(file, expression, case_locals, line, ExprMode::Read);
                if !self.expression_compatible(matched_type, &pattern_type, Some(expression)) {
                    self.report(
                        "TYPE_MATCH_PATTERN_MISMATCH",
                        &format!(
                            "CASE pattern has type {}, expected {}.",
                            self.type_name(&pattern_type),
                            self.type_name(matched_type)
                        ),
                        file,
                        line,
                    );
                }
            }
            MatchPattern::OneOf(expressions) => {
                for expression in expressions {
                    self.check_match_pattern(
                        file,
                        &MatchPattern::Literal(expression.clone()),
                        matched_type,
                        case_locals,
                        line,
                    );
                }
            }
            MatchPattern::Union { type_name, binding } => {
                if matches!(type_name.as_str(), "Ok" | "Error" | "Err") {
                    // `Result`/`Ok` are internal: a user `MATCH` can never
                    // scrutinize a `Result`, so `CASE Ok`/`CASE Error` are not
                    // valid match arms. Failures are handled with inline `TRAP`.
                    self.report(
                        "TYPE_RESULT_NOT_MATCHABLE",
                        &format!(
                            "`CASE {type_name}` is not a valid match arm; \
                             handle failure with an inline `TRAP` instead."
                        ),
                        file,
                        line,
                    );
                    return;
                }
                match matched_type {
                    Type::User(union_name) => {
                        let Some(info) = self.type_infos.get(union_name) else {
                            return;
                        };
                        if !matches!(info.kind, TypeDeclKind::Union)
                            || !info
                                .variants
                                .iter()
                                .any(|variant| variant.name == *type_name)
                        {
                            self.report(
                                "TYPE_MATCH_PATTERN_MISMATCH",
                                &format!(
                                    "CASE `{type_name}` is not a member of UNION `{union_name}`."
                                ),
                                file,
                                line,
                            );
                            return;
                        }
                        case_locals.insert(
                            binding.clone(),
                            LocalInfo {
                                type_: Type::User(type_name.clone()),
                                mutable: false,
                                ownership: OwnershipState::Available,
                                borrowed: false,
                                state_type: None,
                            },
                        );
                    }
                    _ => self.report(
                        "TYPE_MATCH_PATTERN_MISMATCH",
                        &format!(
                            "CASE `{type_name}` requires a UNION value, got {}.",
                            self.type_name(matched_type)
                        ),
                        file,
                        line,
                    ),
                }
            }
        }
    }

    fn match_case_name(&self, pattern: &MatchPattern) -> Option<String> {
        match pattern {
            MatchPattern::Union { type_name, .. } => Some(type_name.clone()),
            MatchPattern::Literal(Expression::MemberAccess { target, member }) => {
                if let Expression::Identifier(type_name) = target.as_ref() {
                    Some(format!("{type_name}::{member}"))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn match_is_exhaustive(&self, matched_type: &Type, covered_cases: &HashSet<String>) -> bool {
        let Type::User(type_name) = matched_type else {
            return false;
        };
        let Some(info) = self.type_infos.get(type_name) else {
            return false;
        };
        match info.kind {
            TypeDeclKind::Enum => info
                .members
                .iter()
                .all(|member| covered_cases.contains(&format!("{type_name}::{member}"))),
            TypeDeclKind::Union => info
                .variants
                .iter()
                .all(|variant| covered_cases.contains(&variant.name)),
            TypeDeclKind::Type => false,
        }
    }

    fn report_match_not_exhaustive(
        &mut self,
        file: &AstFile,
        line: usize,
        matched_type: &Type,
        covered_cases: &HashSet<String>,
    ) {
        let detail = match matched_type {
            Type::User(type_name) => {
                let Some(info) = self.type_infos.get(type_name) else {
                    return;
                };
                match info.kind {
                    TypeDeclKind::Enum => {
                        let mut missing = info
                            .members
                            .iter()
                            .filter_map(|member| {
                                let case_name = format!("{type_name}::{member}");
                                if covered_cases.contains(&case_name) {
                                    None
                                } else {
                                    Some(format!("{type_name}.{member}"))
                                }
                            })
                            .collect::<Vec<_>>();
                        missing.sort();
                        format!(
                            "MATCH on enum `{type_name}` does not cover {}; add unguarded CASE arms or CASE ELSE.",
                            missing.join(", ")
                        )
                    }
                    TypeDeclKind::Union => {
                        let missing = info
                            .variants
                            .iter()
                            .filter_map(|variant| {
                                if covered_cases.contains(&variant.name) {
                                    None
                                } else {
                                    Some(variant.name.clone())
                                }
                            })
                            .collect::<Vec<_>>();
                        format!(
                            "MATCH on UNION `{type_name}` does not cover {}; add unguarded CASE arms or CASE ELSE.",
                            missing.join(", ")
                        )
                    }
                    TypeDeclKind::Type => format!(
                        "MATCH on open type {} requires an unguarded CASE ELSE.",
                        self.type_name(matched_type)
                    ),
                }
            }
            _ => format!(
                "MATCH on open type {} requires an unguarded CASE ELSE.",
                self.type_name(matched_type)
            ),
        };
        self.report("TYPE_MATCH_NOT_EXHAUSTIVE", &detail, file, line);
    }

    fn infer_list_literal(
        &mut self,
        file: &AstFile,
        values: &[Expression],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
        expected: Option<&Type>,
    ) -> Type {
        if let Some(Type::List(expected_element)) = expected {
            if self.contains_thread(expected_element) {
                self.report_invalid_collection_element(file, line, "element", expected_element);
            }
            for value in values {
                let mode = self.collection_element_mode(value, locals);
                let actual = self.infer_expression_with_expected(
                    file,
                    value,
                    locals,
                    line,
                    Some(expected_element),
                    mode,
                );
                self.check_collection_resource_element(
                    file, line, "element", value, &actual, locals,
                );
                if !self.expression_compatible(expected_element, &actual, Some(value)) {
                    self.report(
                        "TYPE_LIST_ELEMENT_MISMATCH",
                        &format!(
                            "List element has type {}, expected {}.",
                            self.type_name(&actual),
                            self.type_name(expected_element)
                        ),
                        file,
                        line,
                    );
                }
            }
            return Type::List(expected_element.clone());
        }

        let Some(first) = values.first() else {
            return Type::List(Box::new(Type::Unknown));
        };
        let first_mode = self.collection_element_mode(first, locals);
        let element_type = self.infer_expression(file, first, locals, line, first_mode);
        if self.contains_thread(&element_type) {
            self.report_invalid_collection_element(file, line, "element", &element_type);
        }
        self.check_collection_resource_element(file, line, "element", first, &element_type, locals);
        for value in values.iter().skip(1) {
            let mode = self.collection_element_mode(value, locals);
            let actual = self.infer_expression(file, value, locals, line, mode);
            self.check_collection_resource_element(file, line, "element", value, &actual, locals);
            if !self.expression_compatible(&element_type, &actual, Some(value)) {
                self.report(
                    "TYPE_LIST_ELEMENT_MISMATCH",
                    &format!(
                        "List element has type {}, expected {}.",
                        self.type_name(&actual),
                        self.type_name(&element_type)
                    ),
                    file,
                    line,
                );
            }
        }
        Type::List(Box::new(element_type))
    }

    fn infer_map_literal(
        &mut self,
        file: &AstFile,
        key_type: &str,
        value_type: &str,
        entries: &[(Expression, Expression)],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let key_type = self.parse_type(key_type);
        // The value may carry the `RES` ownership-axis marker (`Map OF K TO RES
        // File`, §15.6).
        let value_type = self.parse_collection_element_type(value_type);
        self.check_type_reference(file, &key_type, line);
        self.check_type_reference(file, strip_res(&value_type), line);
        if self.contains_resource_or_thread(&key_type) {
            self.report_invalid_collection_element(file, line, "key", &key_type);
        }
        self.require_comparable_type(file, line, "Map key type", &key_type);
        if self.contains_thread(&value_type) {
            self.report_invalid_collection_element(file, line, "value", &value_type);
        }
        for (key, value) in entries {
            let actual_key = self.infer_expression(file, key, locals, line, ExprMode::Transfer);
            if !self.expression_compatible(&key_type, &actual_key, Some(key)) {
                self.report(
                    "TYPE_MAP_KEY_MISMATCH",
                    &format!(
                        "Map key has type {}, expected {}.",
                        self.type_name(&actual_key),
                        self.type_name(&key_type)
                    ),
                    file,
                    line,
                );
            }
            let value_mode = self.collection_element_mode(value, locals);
            let actual_value = self.infer_expression(file, value, locals, line, value_mode);
            self.check_collection_resource_element(
                file,
                line,
                "value",
                value,
                &actual_value,
                locals,
            );
            if !self.expression_compatible(&value_type, &actual_value, Some(value)) {
                self.report(
                    "TYPE_MAP_VALUE_MISMATCH",
                    &format!(
                        "Map value has type {}, expected {}.",
                        self.type_name(&actual_value),
                        self.type_name(&value_type)
                    ),
                    file,
                    line,
                );
            }
        }
        Type::Map(Box::new(key_type), Box::new(value_type))
    }

    fn infer_constructor(
        &mut self,
        file: &AstFile,
        type_name: &str,
        arguments: &[ConstructorArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
        _expected: Option<&Type>,
    ) -> Type {
        // `Error` and `ErrorLoc` are read-only compiler/runtime-generated records.
        // Direct construction is rejected; user errors are created with the
        // `error(code, message)` built-in instead.
        if matches!(type_name, "Error" | "ErrorLoc") {
            self.report(
                "TYPE_READ_ONLY_RECORD_CONSTRUCTOR",
                &format!(
                    "`{type_name}` is a read-only built-in record and cannot be constructed; use `error(code, message)` to create an Error."
                ),
                file,
                line,
            );
            for argument in arguments {
                self.infer_expression(
                    file,
                    constructor_arg_value(argument),
                    locals,
                    line,
                    ExprMode::Transfer,
                );
            }
            return if type_name == "Error" {
                Type::Error
            } else {
                Type::ErrorLoc
            };
        }

        if matches!(type_name, "Ok" | "Result") {
            self.report(
                "TYPE_RESULT_IS_IMPLICIT",
                &format!("`{type_name}` is compiler-owned and cannot be constructed directly."),
                file,
                line,
            );
            for argument in arguments {
                self.infer_expression(
                    file,
                    constructor_arg_value(argument),
                    locals,
                    line,
                    ExprMode::Transfer,
                );
            }
            return Type::Unknown;
        }

        if read_only_record_type(type_name) {
            self.report(
                "TYPE_READ_ONLY_RECORD_CONSTRUCTOR",
                &format!("TYPE `{type_name}` is compiler-owned and cannot be constructed."),
                file,
                line,
            );
            for argument in arguments {
                self.infer_expression(
                    file,
                    constructor_arg_value(argument),
                    locals,
                    line,
                    ExprMode::Transfer,
                );
            }
            return Type::Unknown;
        }

        if let Some(info) = self.type_infos.get(type_name).cloned() {
            if !self.visible_from(file, info.visibility, &info.file_path) {
                self.report(
                    "TYPE_MEMBER_NOT_VISIBLE",
                    &format!("Constructor `{type_name}` is not visible from this file."),
                    file,
                    line,
                );
                return Type::Unknown;
            }
            if !matches!(info.kind, TypeDeclKind::Type) {
                self.report(
                    "TYPE_CONSTRUCTOR_REQUIRES_RECORD",
                    &format!(
                        "`{type_name}` is a {}, not a record TYPE.",
                        type_kind_name(info.kind)
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
            self.check_constructor_arguments(
                file,
                type_name,
                &info.fields,
                &info.file_path,
                arguments,
                locals,
                line,
            );
            return Type::User(type_name.to_string());
        }

        for argument in arguments {
            self.infer_expression(
                file,
                constructor_arg_value(argument),
                locals,
                line,
                ExprMode::Transfer,
            );
        }
        Type::Unknown
    }

    fn infer_with_update(
        &mut self,
        file: &AstFile,
        target: &Expression,
        updates: &[RecordUpdate],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let target_type = self.infer_expression(file, target, locals, line, ExprMode::Transfer);
        if matches!(target_type, Type::Error | Type::ErrorLoc) {
            self.report(
                "TYPE_READ_ONLY_RECORD_UPDATE",
                &format!(
                    "`{}` is a read-only built-in record and cannot be updated.",
                    self.type_name(&target_type)
                ),
                file,
                line,
            );
            for update in updates {
                self.infer_expression(file, &update.value, locals, update.line, ExprMode::Transfer);
            }
            return target_type;
        }
        let Type::User(type_name) = &target_type else {
            self.report(
                "TYPE_FIELD_ACCESS_REQUIRES_RECORD",
                &format!(
                    "WITH update requires a record value, got {}.",
                    self.type_name(&target_type)
                ),
                file,
                line,
            );
            return Type::Unknown;
        };
        if read_only_record_type(type_name) {
            self.report(
                "TYPE_READ_ONLY_RECORD_UPDATE",
                &format!("TYPE `{type_name}` is read-only and cannot be updated."),
                file,
                line,
            );
            for update in updates {
                self.infer_expression(file, &update.value, locals, update.line, ExprMode::Transfer);
            }
            return Type::Unknown;
        }
        let Some(info) = self.type_infos.get(type_name).cloned() else {
            return Type::Unknown;
        };
        if !matches!(info.kind, TypeDeclKind::Type) {
            self.report(
                "TYPE_FIELD_ACCESS_REQUIRES_RECORD",
                &format!(
                    "WITH update requires a record value, got {} `{type_name}`.",
                    type_kind_name(info.kind)
                ),
                file,
                line,
            );
            return Type::Unknown;
        }
        let mut seen = HashSet::new();
        for update in updates {
            if !seen.insert(update.field.clone()) {
                self.report(
                    "TYPE_DUPLICATE_FIELD",
                    &format!("WITH update sets field `{}` more than once.", update.field),
                    file,
                    update.line,
                );
            }
            let Some(field) = info.fields.iter().find(|field| field.name == update.field) else {
                self.report(
                    "TYPE_UNKNOWN_FIELD",
                    &format!("TYPE `{type_name}` has no field `{}`.", update.field),
                    file,
                    update.line,
                );
                self.infer_expression(file, &update.value, locals, update.line, ExprMode::Transfer);
                continue;
            };
            if !self.visible_from(file, field.visibility, &info.file_path) {
                self.report(
                    "TYPE_MEMBER_NOT_VISIBLE",
                    &format!(
                        "Field `{type_name}::{}` is not visible from this file.",
                        update.field
                    ),
                    file,
                    update.line,
                );
            }
            let actual = self.infer_expression_with_expected(
                file,
                &update.value,
                locals,
                update.line,
                Some(&field.type_),
                ExprMode::Transfer,
            );
            if !self.expression_compatible(&field.type_, &actual, Some(&update.value)) {
                self.report(
                    "TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH",
                    &format!(
                        "WITH update for `{}` has type {}, expected {}.",
                        update.field,
                        self.type_name(&actual),
                        self.type_name(&field.type_)
                    ),
                    file,
                    update.line,
                );
            }
        }
        target_type
    }

    fn infer_member_access(
        &mut self,
        file: &AstFile,
        target: &Expression,
        member: &str,
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        // `s.state` on a `RES` binding/parameter yields its declared `STATE`
        // record. The owner and a borrower may both read it (and replace it via
        // `s.state = WITH s.state { ... }`).
        if member == "state" {
            if let Expression::Identifier(name) = target {
                if let Some(state) = locals.get(name).and_then(|info| info.state_type.clone()) {
                    if let Some(info) = locals.get(name).cloned() {
                        if !self.require_local_owned(file, line, name, &info) {
                            return Type::Unknown;
                        }
                    }
                    return self.parse_type(&state);
                }
            }
        }

        if let Expression::Identifier(type_name) = target {
            if let Some(info) = self.type_infos.get(type_name).cloned() {
                if matches!(info.kind, TypeDeclKind::Enum) {
                    if !self.visible_from(file, info.visibility, &info.file_path) {
                        self.report(
                            "TYPE_MEMBER_NOT_VISIBLE",
                            &format!("ENUM `{type_name}` is not visible from this file."),
                            file,
                            line,
                        );
                        return Type::Unknown;
                    }
                    if !info.members.contains(member) {
                        self.report(
                            "TYPE_UNKNOWN_ENUM_MEMBER",
                            &format!("ENUM `{type_name}` has no member `{member}`."),
                            file,
                            line,
                        );
                        return Type::Unknown;
                    }
                    return Type::User(type_name.clone());
                }
            }
        }

        let target_type = self.infer_expression(file, target, locals, line, ExprMode::Read);
        if let Type::Thread(..) = &target_type {
            if member == "result" {
                // The `t.result` field is removed: a worker outcome is retrieved
                // only through `thread::waitFor(t)`, which auto-unwraps the value
                // or auto-propagates the `Error` like any other call.
                self.report(
                    "TYPE_THREAD_RESULT_REMOVED",
                    "Thread values have no `result` field; use `thread::waitFor(t)` \
                     to retrieve the worker outcome.",
                    file,
                    line,
                );
                return Type::Unknown;
            }
            self.report(
                "TYPE_UNKNOWN_FIELD",
                &format!("Thread value has no field `{member}`."),
                file,
                line,
            );
            return Type::Unknown;
        }
        if matches!(target_type, Type::Error) {
            return match member {
                "code" => Type::Integer,
                "message" => Type::String,
                "source" => Type::ErrorLoc,
                _ => {
                    self.report(
                        "TYPE_UNKNOWN_FIELD",
                        &format!("Error value has no field `{member}`."),
                        file,
                        line,
                    );
                    Type::Unknown
                }
            };
        }
        if matches!(target_type, Type::ErrorLoc) {
            return match member {
                "filename" => Type::String,
                "line" => Type::Integer,
                "char" => Type::Integer,
                _ => {
                    self.report(
                        "TYPE_UNKNOWN_FIELD",
                        &format!("ErrorLoc value has no field `{member}`."),
                        file,
                        line,
                    );
                    Type::Unknown
                }
            };
        }
        let Type::User(type_name) = target_type else {
            self.report(
                "TYPE_FIELD_ACCESS_REQUIRES_RECORD",
                &format!(
                    "Field access `{member}` requires a record value, got {}.",
                    self.type_name(&target_type)
                ),
                file,
                line,
            );
            return Type::Unknown;
        };
        if let Some(rest) = type_name.strip_prefix("MapEntry OF ") {
            if let Some((key, value)) = rest.split_once(" TO ") {
                return match member {
                    "key" => self.parse_type(key),
                    "value" => self.parse_type(value),
                    _ => {
                        self.report(
                            "TYPE_UNKNOWN_FIELD",
                            &format!("Map entry has no field `{member}`."),
                            file,
                            line,
                        );
                        Type::Unknown
                    }
                };
            }
        }
        let Some(info) = self.type_infos.get(&type_name).cloned() else {
            if let Some(field_type) = builtins::io::builtin_type_fields(&type_name)
                .or_else(|| builtins::net::builtin_type_fields(&type_name))
                .or_else(|| builtins::term::builtin_type_fields(&type_name))
                .and_then(|fields| fields.iter().find(|(name, _)| *name == member))
                .map(|(_, type_name)| self.parse_type(type_name))
            {
                return field_type;
            }
            return Type::Unknown;
        };
        if !matches!(info.kind, TypeDeclKind::Type) {
            self.report(
                "TYPE_FIELD_ACCESS_REQUIRES_RECORD",
                &format!(
                    "Field access `{member}` requires a record value, got {} `{type_name}`.",
                    type_kind_name(info.kind)
                ),
                file,
                line,
            );
            return Type::Unknown;
        }
        let Some(field) = info
            .fields
            .iter()
            .find(|field| field.name == member)
            .cloned()
        else {
            self.report(
                "TYPE_UNKNOWN_FIELD",
                &format!("TYPE `{type_name}` has no field `{member}`."),
                file,
                line,
            );
            return Type::Unknown;
        };
        if !self.visible_from(file, field.visibility, &info.file_path) {
            self.report(
                "TYPE_MEMBER_NOT_VISIBLE",
                &format!("Field `{type_name}::{member}` is not visible from this file."),
                file,
                line,
            );
        }
        field.type_
    }

    fn check_constructor_arguments(
        &mut self,
        file: &AstFile,
        constructor: &str,
        fields: &[FieldInfo],
        owner_file_path: &str,
        arguments: &[ConstructorArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) {
        if arguments.len() != fields.len() {
            self.report(
                "TYPE_CONSTRUCTOR_ARITY_MISMATCH",
                &format!(
                    "Constructor `{constructor}` has {} argument(s), expected {}.",
                    arguments.len(),
                    fields.len()
                ),
                file,
                line,
            );
        }

        for field in fields {
            if !self.visible_from(file, field.visibility, owner_file_path) {
                self.report(
                    "TYPE_MEMBER_NOT_VISIBLE",
                    &format!(
                        "Constructor `{constructor}` cannot set hidden field `{}` from this file.",
                        field.name
                    ),
                    file,
                    line,
                );
            }
        }

        let mut seen_named = HashSet::new();
        for (index, argument) in arguments.iter().enumerate() {
            let (field, argument_value, argument_line) = match argument {
                ConstructorArg::Positional(value) => (fields.get(index), value, line),
                ConstructorArg::Named {
                    name,
                    value,
                    line: argument_line,
                } => {
                    if !seen_named.insert(name.clone()) {
                        self.report(
                            "TYPE_DUPLICATE_FIELD",
                            &format!(
                                "Constructor `{constructor}` sets field `{name}` more than once."
                            ),
                            file,
                            *argument_line,
                        );
                    }
                    (
                        fields.iter().find(|field| field.name == *name),
                        value,
                        *argument_line,
                    )
                }
            };
            let actual = if let Some(field) = field {
                self.infer_expression_with_expected(
                    file,
                    argument_value,
                    locals,
                    argument_line,
                    Some(&field.type_),
                    ExprMode::Transfer,
                )
            } else {
                self.infer_expression(
                    file,
                    argument_value,
                    locals,
                    argument_line,
                    ExprMode::Transfer,
                )
            };
            let Some(field) = field else {
                if let ConstructorArg::Named { name, .. } = argument {
                    self.report(
                        "TYPE_UNKNOWN_FIELD",
                        &format!("Constructor `{constructor}` has no field `{name}`."),
                        file,
                        argument_line,
                    );
                }
                continue;
            };
            if !self.expression_compatible(&field.type_, &actual, Some(argument_value)) {
                self.report(
                    "TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH",
                    &format!(
                        "Argument {} for `{constructor}` has type {}, expected {} for field `{}`.",
                        index + 1,
                        self.type_name(&actual),
                        self.type_name(&field.type_),
                        field.name
                    ),
                    file,
                    argument_line,
                );
            }
        }
    }

    fn infer_binary(
        &mut self,
        file: &AstFile,
        operator: &str,
        left: &Type,
        right: &Type,
        line: usize,
    ) -> Type {
        if matches!(operator, "AND" | "OR" | "XOR") {
            if self.compatible(&Type::Boolean, left) && self.compatible(&Type::Boolean, right) {
                return Type::Boolean;
            }
            self.report(
                "TYPE_BINARY_OPERATOR_MISMATCH",
                &format!(
                    "Operator `{operator}` requires Boolean operands, got {} and {}.",
                    self.type_name(left),
                    self.type_name(right)
                ),
                file,
                line,
            );
            return Type::Unknown;
        }

        if matches!(operator, "=" | "<>") {
            if self.is_numeric(left) && self.is_numeric(right) {
                return Type::Boolean;
            }
            if (self.compatible(left, right) || self.compatible(right, left))
                && self.is_comparable(left)
                && self.is_comparable(right)
            {
                return Type::Boolean;
            }
            self.report(
                if self.compatible(left, right) || self.compatible(right, left) {
                    "TYPE_REQUIRES_COMPARABLE"
                } else {
                    "TYPE_BINARY_OPERATOR_MISMATCH"
                },
                &format!(
                    "Operator `{operator}` requires compatible comparable operands, got {} and {}.",
                    self.type_name(left),
                    self.type_name(right)
                ),
                file,
                line,
            );
            return Type::Unknown;
        }

        if matches!(operator, "<" | ">" | "<=" | ">=") {
            if self.is_numeric(left) && self.is_numeric(right) {
                return Type::Boolean;
            }
            // String is orderable: `<`, `>`, `<=`, `>=` compare two String operands
            // lexicographically by Unicode scalar value. Mixed String/numeric stays a
            // type error. Unknown is permissive on either side to avoid cascades.
            if self.is_orderable_string(left) && self.is_orderable_string(right) {
                return Type::Boolean;
            }
            self.report(
                "TYPE_BINARY_OPERATOR_MISMATCH",
                &format!(
                    "Operator `{operator}` requires numeric or String operands, got {} and {}.",
                    self.type_name(left),
                    self.type_name(right)
                ),
                file,
                line,
            );
            return Type::Unknown;
        }

        if operator == "&" {
            if self.compatible(&Type::String, left) && self.compatible(&Type::String, right) {
                return Type::String;
            }
            self.report(
                "TYPE_BINARY_OPERATOR_MISMATCH",
                &format!(
                    "Operator `&` requires String operands, got {} and {}.",
                    self.type_name(left),
                    self.type_name(right)
                ),
                file,
                line,
            );
            return Type::Unknown;
        }

        if self.is_numeric(left) && self.is_numeric(right) {
            numeric_binary_result_type(operator, left, right)
        } else {
            self.report(
                "TYPE_BINARY_OPERATOR_MISMATCH",
                &format!(
                    "Operator `{operator}` requires numeric operands, got {} and {}.",
                    self.type_name(left),
                    self.type_name(right)
                ),
                file,
                line,
            );
            Type::Unknown
        }
    }

    fn infer_unary(&mut self, file: &AstFile, operator: &str, operand: &Type, line: usize) -> Type {
        match operator {
            "NOT" => {
                if self.compatible(&Type::Boolean, operand) {
                    Type::Boolean
                } else {
                    self.report(
                        "TYPE_UNARY_OPERATOR_MISMATCH",
                        &format!(
                            "Operator `NOT` requires a Boolean operand, got {}.",
                            self.type_name(operand)
                        ),
                        file,
                        line,
                    );
                    Type::Unknown
                }
            }
            "-" => {
                if self.is_numeric(operand) {
                    operand.clone()
                } else {
                    self.report(
                        "TYPE_UNARY_OPERATOR_MISMATCH",
                        &format!(
                            "Unary `-` requires a numeric operand, got {}.",
                            self.type_name(operand)
                        ),
                        file,
                        line,
                    );
                    Type::Unknown
                }
            }
            _ => {
                self.report(
                    "TYPE_UNARY_OPERATOR_UNKNOWN",
                    &format!("Unknown unary operator `{operator}`."),
                    file,
                    line,
                );
                Type::Unknown
            }
        }
    }

    fn check_call(
        &mut self,
        file: &AstFile,
        callee: &str,
        sig: &FunctionSig,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) {
        let arguments =
            self.normalize_named_arguments(file, callee, arguments, &sig.params, line, false);

        for (index, argument) in arguments.iter().enumerate() {
            let Some(argument) = argument else {
                continue;
            };
            let actual = self.infer_expression(
                file,
                argument,
                locals,
                line,
                self.call_argument_mode(callee, index, sig),
            );
            let Some(param) = sig.params.get(index) else {
                continue;
            };
            if !self.expression_compatible(&param.type_, &actual, Some(argument)) {
                self.report(
                    "TYPE_CALL_ARGUMENT_MISMATCH",
                    &format!(
                        "Argument {} for `{callee}` has type {}, expected {}.",
                        index + 1,
                        self.type_name(&actual),
                        self.type_name(&param.type_)
                    ),
                    file,
                    line,
                );
            }
        }
    }

    fn check_function_value_call(
        &mut self,
        file: &AstFile,
        callee: &str,
        type_: &Type,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let Type::Function {
            params,
            return_type,
            ..
        } = type_
        else {
            self.report(
                "SYMBOL_NOT_CALLABLE",
                &format!("Local binding or parameter `{callee}` is not callable."),
                file,
                line,
            );
            for argument in arguments {
                self.infer_expression(file, call_arg_value(argument), locals, line, ExprMode::Read);
            }
            return Type::Unknown;
        };

        if arguments
            .iter()
            .any(|argument| matches!(argument, CallArg::Named { .. }))
        {
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to function value `{callee}` cannot use named arguments because the callable type does not preserve parameter names."
                ),
                file,
                line,
            );
        }

        if arguments.len() != params.len() {
            self.report(
                "TYPE_CALL_ARITY_MISMATCH",
                &format!(
                    "Call to `{callee}` has {} argument(s), expected {}.",
                    arguments.len(),
                    params.len()
                ),
                file,
                line,
            );
        }

        for (index, argument) in arguments.iter().enumerate() {
            let argument = call_arg_value(argument);
            let actual = self.infer_expression(
                file,
                argument,
                locals,
                line,
                self.argument_mode_for_type(&params.get(index)),
            );
            let Some(expected) = params.get(index) else {
                continue;
            };
            if !self.expression_compatible(expected, &actual, Some(argument)) {
                self.report(
                    "TYPE_CALL_ARGUMENT_MISMATCH",
                    &format!(
                        "Argument {} for `{callee}` has type {}, expected {}.",
                        index + 1,
                        self.type_name(&actual),
                        self.type_name(expected)
                    ),
                    file,
                    line,
                );
            }
        }

        *return_type.clone()
    }

    fn infer_lambda(
        &mut self,
        file: &AstFile,
        params: &[crate::ast::Param],
        body: &Expression,
        assign_target: Option<&str>,
        outer_locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let mut locals = outer_locals.clone();
        let mut param_types = Vec::new();
        for param in params {
            let type_ = param
                .type_name
                .as_deref()
                .map(|name| self.parse_type(name))
                .unwrap_or(Type::Unknown);
            if param.type_name.is_none() {
                self.report(
                    "TYPE_PARAM_REQUIRES_TYPE",
                    &format!(
                        "Lambda parameter `{}` must declare an `AS` type.",
                        param.name
                    ),
                    file,
                    param.line,
                );
            }
            if param.default.is_some() {
                self.report(
                    "TYPE_DEFAULT_ARG_ORDER",
                    "Lambda parameters cannot declare default values.",
                    file,
                    param.line,
                );
            }
            locals.insert(
                param.name.clone(),
                LocalInfo {
                    type_: type_.clone(),
                    mutable: false,
                    ownership: OwnershipState::Available,
                    borrowed: false,
                    state_type: None,
                },
            );
            param_types.push(type_);
        }
        // Consume the non-escaping callback licence so it applies only to this
        // lambda, never to a lambda nested inside its body.
        let nonescaping = self.nonescaping_callback;
        self.nonescaping_callback = false;
        let param_names = params
            .iter()
            .map(|param| param.name.clone())
            .collect::<HashSet<_>>();
        let mut captures = captured_locals(body, outer_locals, &param_names);
        // An assignment-bodied lambda mutates its target, so the target is a
        // capture too even when it never appears on the right-hand side (e.g.
        // `LAMBDA(x) -> total = x`). A target that is a lambda parameter is an
        // ordinary local, not a capture, and is rejected below as immutable.
        if let Some(target) = assign_target {
            if !param_names.contains(target)
                && !captures.iter().any(|capture| capture.name == target)
            {
                if let Some(local) = outer_locals.get(target) {
                    captures.push(CapturedLocal {
                        name: target.to_string(),
                        type_: local.type_.clone(),
                        mutable: local.mutable,
                    });
                }
            }
        }
        for capture in &captures {
            if capture.mutable && !nonescaping {
                // `MUT` capture is rejected by default: an ordinary closure would
                // observe a frozen copy, never the live binding. The
                // sole exception is a compiler-proven non-escaping callback
                // position, handled below.
                self.report(
                    "TYPE_LAMBDA_CAPTURE_UNSUPPORTED",
                    &format!(
                        "Lambda captures mutable local `{}`; mutable captures are invalid.",
                        capture.name
                    ),
                    file,
                    line,
                );
            } else if capture.mutable && self.is_resource_type(&capture.type_) {
                // A non-escaping callback may borrow a `MUT` binding, but never a
                // resource: resource ownership rules are unchanged (§12.4).
                self.report(
                    "TYPE_LAMBDA_CAPTURE_UNSUPPORTED",
                    &format!(
                        "Lambda captures resource local `{}`; resource captures are invalid.",
                        capture.name
                    ),
                    file,
                    line,
                );
            } else if capture.mutable {
                // A permitted non-escaping `MUT` borrow: the binding is loaned to
                // the callback for the duration of the synchronous call and is the
                // outer binding's again once it returns.
            } else if self.is_resource_type(&capture.type_) {
                self.report(
                    "TYPE_LAMBDA_CAPTURE_UNSUPPORTED",
                    &format!(
                        "Lambda captures resource local `{}`; resource captures are invalid.",
                        capture.name
                    ),
                    file,
                    line,
                );
            } else if !self.is_copyable_type(&capture.type_) {
                self.report(
                    "TYPE_LAMBDA_CAPTURE_UNSUPPORTED",
                    &format!(
                        "Lambda captures non-copyable local `{}` of type `{}`; non-copyable captures are invalid.",
                        capture.name,
                        self.type_name(&capture.type_)
                    ),
                    file,
                    line,
                );
            }
        }
        let return_type = match assign_target {
            Some(target) => {
                // `name = <body>`: validate the assignment the same way the
                // statement form does — the target must be a mutable binding and
                // the body type must match it — then yield `Nothing`.
                let target_type = match locals.get(target).cloned() {
                    Some(local) => {
                        if !local.mutable {
                            self.report(
                                "TYPE_ASSIGN_REQUIRES_MUT",
                                &format!("Binding `{target}` is immutable and cannot be assigned."),
                                file,
                                line,
                            );
                        }
                        Some(local.type_)
                    }
                    None => {
                        self.report(
                            "TYPE_UNKNOWN_VALUE",
                            &format!("Assignment target `{target}` is not a local binding."),
                            file,
                            line,
                        );
                        None
                    }
                };
                let actual =
                    self.infer_expression(file, body, &mut locals, line, ExprMode::Transfer);
                if let Some(target_type) = target_type {
                    let reported_range_error =
                        self.report_primitive_literal_range_error(&target_type, body, file, line);
                    if !reported_range_error
                        && !self.expression_compatible(&target_type, &actual, Some(body))
                    {
                        self.report(
                            "TYPE_ASSIGNMENT_MISMATCH",
                            &format!(
                                "Assignment to `{target}` has type {}, expected {}.",
                                self.type_name(&actual),
                                self.type_name(&target_type)
                            ),
                            file,
                            line,
                        );
                    }
                }
                Type::Nothing
            }
            None => self.infer_expression(file, body, &mut locals, line, ExprMode::Read),
        };
        Type::Function {
            params: param_types,
            return_type: Box::new(return_type),
            isolated: false,
        }
    }

    fn check_builtin_call(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
        expected: Option<&Type>,
    ) -> Type {
        if builtins::encoding::is_encoding_call(callee) {
            return self.check_encoding_builtin_call(
                file,
                display_callee,
                callee,
                arguments,
                locals,
                line,
                expected,
            );
        }
        if builtins::general::is_general_call(callee) {
            return self.check_general_builtin_call(
                file,
                display_callee,
                callee,
                arguments,
                locals,
                line,
            );
        }
        if builtins::collections::is_native_member_call(callee) {
            return self.check_collections_builtin_call(
                file,
                display_callee,
                callee,
                arguments,
                locals,
                line,
            );
        }
        if builtins::strings::is_strings_call(callee) {
            return self.check_strings_builtin_call(
                file,
                display_callee,
                callee,
                arguments,
                locals,
                line,
            );
        }
        if builtins::math::is_math_call(callee) {
            return self.check_math_builtin_call(
                file,
                display_callee,
                callee,
                arguments,
                locals,
                line,
            );
        }
        if builtins::bits::is_bits_call(callee) {
            return self.check_bits_builtin_call(
                file,
                display_callee,
                callee,
                arguments,
                locals,
                line,
            );
        }
        if builtins::fs::is_fs_call(callee) {
            return self.check_fs_builtin_call(
                file,
                display_callee,
                callee,
                arguments,
                locals,
                line,
            );
        }
        if builtins::net::is_net_call(callee) {
            return self.check_net_builtin_call(
                file,
                display_callee,
                callee,
                arguments,
                locals,
                line,
            );
        }
        if builtins::tls::is_tls_call(callee) {
            return self.check_tls_builtin_call(
                file,
                display_callee,
                callee,
                arguments,
                locals,
                line,
            );
        }
        if builtins::io::is_io_call(callee) {
            return self.check_io_builtin_call(
                file,
                display_callee,
                callee,
                arguments,
                locals,
                line,
            );
        }
        if builtins::term::is_term_call(callee) {
            return self.check_term_builtin_call(
                file,
                display_callee,
                callee,
                arguments,
                locals,
                line,
            );
        }
        if builtins::json::is_json_call(callee) {
            return self.check_json_builtin_call(
                file,
                display_callee,
                callee,
                arguments,
                locals,
                line,
            );
        }
        if builtins::csv::is_csv_call(callee) {
            return self.check_csv_builtin_call(
                file,
                display_callee,
                callee,
                arguments,
                locals,
                line,
            );
        }
        if builtins::regex::is_regex_call(callee) {
            return self.check_regex_builtin_call(
                file,
                display_callee,
                callee,
                arguments,
                locals,
                line,
            );
        }
        if builtins::datetime::is_datetime_call(callee) {
            return self.check_datetime_builtin_call(
                file,
                display_callee,
                callee,
                arguments,
                locals,
                line,
            );
        }
        if builtins::http::is_http_call(callee) {
            return self.check_http_builtin_call(
                file,
                display_callee,
                callee,
                arguments,
                locals,
                line,
            );
        }
        if builtins::thread::is_thread_call(callee) {
            return self.check_thread_builtin_call(
                file,
                display_callee,
                callee,
                arguments,
                locals,
                line,
            );
        }

        for argument in arguments {
            self.infer_expression(file, call_arg_value(argument), locals, line, ExprMode::Read);
        }
        Type::Unknown
    }

    fn check_fs_builtin_call(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arguments =
            self.normalize_builtin_call_arguments(file, display_callee, callee, arguments, line);
        let arg_types = arguments
            .iter()
            .enumerate()
            .map(|(index, argument)| {
                let mode = if callee == "fs.close" && index == 0 {
                    ExprMode::Transfer
                } else {
                    ExprMode::Borrow
                };
                let type_ = self.infer_expression(file, argument, locals, line, mode);
                self.type_name(&type_)
            })
            .collect::<Vec<_>>();

        if let Some((min, max)) = builtins::fs::arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                let expected = if min == max {
                    min.to_string()
                } else {
                    format!("{min} to {max}")
                };
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{display_callee}` has {} argument(s), expected {expected}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        let Some(resolved) = builtins::fs::resolve_call(callee, &arg_types) else {
            let expected = builtins::fs::expected_arguments(callee).unwrap_or("supported overload");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{display_callee}` has argument type(s) ({}), expected {expected}.",
                    arg_types.join(", ")
                ),
                file,
                line,
            );
            return Type::Unknown;
        };

        self.parse_type(&resolved.return_type)
    }

    fn check_net_builtin_call(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arguments =
            self.normalize_builtin_call_arguments(file, display_callee, callee, arguments, line);
        let arg_types = arguments
            .iter()
            .enumerate()
            .map(|(index, argument)| {
                // `net.close` consumes the socket/listener handle it closes.
                let mode = if callee == "net.close" && index == 0 {
                    ExprMode::Transfer
                } else {
                    ExprMode::Borrow
                };
                let type_ = self.infer_expression(file, argument, locals, line, mode);
                self.type_name(&type_)
            })
            .collect::<Vec<_>>();

        if let Some((min, max)) = builtins::net::arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                let expected = if min == max {
                    min.to_string()
                } else {
                    format!("{min} to {max}")
                };
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{display_callee}` has {} argument(s), expected {expected}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        let Some(resolved) = builtins::net::resolve_call(callee, &arg_types) else {
            let expected =
                builtins::net::expected_arguments(callee).unwrap_or("supported overload");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{display_callee}` has argument type(s) ({}), expected {expected}.",
                    arg_types.join(", ")
                ),
                file,
                line,
            );
            return Type::Unknown;
        };

        self.parse_type(&resolved.return_type)
    }

    fn check_tls_builtin_call(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arguments =
            self.normalize_builtin_call_arguments(file, display_callee, callee, arguments, line);
        let arg_types = arguments
            .iter()
            .enumerate()
            .map(|(index, argument)| {
                // `tls.close` consumes the `TlsSocket` it closes.
                let mode = if builtins::tls::consumes_argument(callee, index) {
                    ExprMode::Transfer
                } else {
                    ExprMode::Borrow
                };
                let type_ = self.infer_expression(file, argument, locals, line, mode);
                self.type_name(&type_)
            })
            .collect::<Vec<_>>();

        if let Some((min, max)) = builtins::tls::arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                let expected = if min == max {
                    min.to_string()
                } else {
                    format!("{min} to {max}")
                };
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{display_callee}` has {} argument(s), expected {expected}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        let Some(resolved) = builtins::tls::resolve_call(callee, &arg_types) else {
            let expected =
                builtins::tls::expected_arguments(callee).unwrap_or("supported overload");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{display_callee}` has argument type(s) ({}), expected {expected}.",
                    arg_types.join(", ")
                ),
                file,
                line,
            );
            return Type::Unknown;
        };

        self.parse_type(&resolved.return_type)
    }

    fn check_json_builtin_call(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arguments =
            self.normalize_builtin_call_arguments(file, display_callee, callee, arguments, line);
        let arg_types = arguments
            .iter()
            .map(|argument| {
                let type_ = self.infer_expression(file, argument, locals, line, ExprMode::Read);
                self.type_name(&type_)
            })
            .collect::<Vec<_>>();

        if let Some((min, max)) = builtins::json::arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                let expected = if min == max {
                    min.to_string()
                } else {
                    format!("{min} to {max}")
                };
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{display_callee}` has {} argument(s), expected {expected}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        let Some(resolved) = builtins::json::resolve_call(callee, &arg_types) else {
            let expected =
                builtins::json::expected_arguments(callee).unwrap_or("supported overload");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{display_callee}` has argument type(s) ({}), expected {expected}.",
                    arg_types.join(", ")
                ),
                file,
                line,
            );
            return Type::Unknown;
        };

        self.parse_type(&resolved.return_type)
    }

    fn check_csv_builtin_call(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arguments =
            self.normalize_builtin_call_arguments(file, display_callee, callee, arguments, line);
        let arg_types = arguments
            .iter()
            .map(|argument| {
                let type_ = self.infer_expression(file, argument, locals, line, ExprMode::Read);
                self.type_name(&type_)
            })
            .collect::<Vec<_>>();

        if let Some((min, max)) = builtins::csv::arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                let expected = if min == max {
                    min.to_string()
                } else {
                    format!("{min} to {max}")
                };
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{display_callee}` has {} argument(s), expected {expected}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        let Some(resolved) = builtins::csv::resolve_call(callee, &arg_types) else {
            let expected =
                builtins::csv::expected_arguments(callee).unwrap_or("supported overload");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{display_callee}` has argument type(s) ({}), expected {expected}.",
                    arg_types.join(", ")
                ),
                file,
                line,
            );
            return Type::Unknown;
        };

        self.parse_type(&resolved.return_type)
    }

    fn check_http_builtin_call(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arguments =
            self.normalize_builtin_call_arguments(file, display_callee, callee, arguments, line);
        let arg_types = arguments
            .iter()
            .map(|argument| {
                let type_ = self.infer_expression(file, argument, locals, line, ExprMode::Read);
                self.type_name(&type_)
            })
            .collect::<Vec<_>>();

        if let Some((min, max)) = builtins::http::arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                let expected = if min == max {
                    min.to_string()
                } else {
                    format!("{min} to {max}")
                };
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{display_callee}` has {} argument(s), expected {expected}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        let Some(resolved) = builtins::http::resolve_call(callee, &arg_types) else {
            let expected =
                builtins::http::expected_arguments(callee).unwrap_or("supported overload");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{display_callee}` has argument type(s) ({}), expected {expected}.",
                    arg_types.join(", ")
                ),
                file,
                line,
            );
            return Type::Unknown;
        };

        self.parse_type(&resolved.return_type)
    }

    fn check_regex_builtin_call(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arguments =
            self.normalize_builtin_call_arguments(file, display_callee, callee, arguments, line);
        let arg_types = arguments
            .iter()
            .map(|argument| {
                let type_ = self.infer_expression(file, argument, locals, line, ExprMode::Read);
                self.type_name(&type_)
            })
            .collect::<Vec<_>>();

        if let Some((min, max)) = builtins::regex::arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                let expected = if min == max {
                    min.to_string()
                } else {
                    format!("{min} to {max}")
                };
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{display_callee}` has {} argument(s), expected {expected}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        let Some(resolved) = builtins::regex::resolve_call(callee, &arg_types) else {
            let expected =
                builtins::regex::expected_arguments(callee).unwrap_or("supported overload");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{display_callee}` has argument type(s) ({}), expected {expected}.",
                    arg_types.join(", ")
                ),
                file,
                line,
            );
            return Type::Unknown;
        };

        self.parse_type(&resolved.return_type)
    }

    fn check_datetime_builtin_call(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arguments =
            self.normalize_builtin_call_arguments(file, display_callee, callee, arguments, line);
        let arg_types = arguments
            .iter()
            .map(|argument| {
                let type_ = self.infer_expression(file, argument, locals, line, ExprMode::Read);
                self.type_name(&type_)
            })
            .collect::<Vec<_>>();

        if let Some((min, max)) = builtins::datetime::arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                let expected = if min == max {
                    min.to_string()
                } else {
                    format!("{min} to {max}")
                };
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{display_callee}` has {} argument(s), expected {expected}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        let Some(resolved) = builtins::datetime::resolve_call(callee, &arg_types) else {
            let expected =
                builtins::datetime::expected_arguments(callee).unwrap_or("supported overload");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{display_callee}` has argument type(s) ({}), expected {expected}.",
                    arg_types.join(", ")
                ),
                file,
                line,
            );
            return Type::Unknown;
        };

        self.parse_type(&resolved.return_type)
    }

    fn check_io_builtin_call(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arguments =
            self.normalize_builtin_call_arguments(file, display_callee, callee, arguments, line);
        let arg_types = arguments
            .iter()
            .map(|argument| {
                let type_ = self.infer_expression(file, argument, locals, line, ExprMode::Read);
                self.type_name(&type_)
            })
            .collect::<Vec<_>>();

        if let Some((min, max)) = builtins::io::arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                let expected = if min == max {
                    if min == 0 {
                        "0".to_string()
                    } else {
                        min.to_string()
                    }
                } else {
                    format!("{min} to {max}")
                };
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{display_callee}` has {} argument(s), expected {expected}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        let Some(resolved) = builtins::io::resolve_call(callee, &arg_types) else {
            let expected = builtins::io::expected_arguments(callee).unwrap_or("supported overload");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{display_callee}` has argument type(s) ({}), expected {expected}.",
                    arg_types.join(", ")
                ),
                file,
                line,
            );
            return Type::Unknown;
        };

        self.parse_type(&resolved.return_type)
    }

    fn check_term_builtin_call(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arguments =
            self.normalize_builtin_call_arguments(file, display_callee, callee, arguments, line);

        if let Some((min, max)) = builtins::term::arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                let expected = if min == max {
                    min.to_string()
                } else {
                    format!("{min} to {max}")
                };
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{display_callee}` has {} argument(s), expected {expected}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                // Still infer the arguments so nested errors are reported.
                for argument in &arguments {
                    self.infer_expression(file, argument, locals, line, ExprMode::Read);
                }
                return self.term_return_type(callee);
            }
        }

        let param_types = builtins::term::param_types(callee).unwrap_or(&[]);
        let arg_types = arguments
            .iter()
            .map(|argument| self.infer_expression(file, argument, locals, line, ExprMode::Read))
            .collect::<Vec<_>>();

        let mut mismatch = false;
        for (index, expected_name) in param_types.iter().enumerate() {
            let expected = self.parse_type(expected_name);
            let actual = &arg_types[index];
            if !self.expression_compatible(&expected, actual, Some(&arguments[index])) {
                mismatch = true;
            }
        }

        if mismatch {
            let expected = builtins::term::expected_arguments(callee)
                .unwrap_or_else(|| "no arguments".to_string());
            let actual = arg_types
                .iter()
                .map(|type_| self.type_name(type_))
                .collect::<Vec<_>>()
                .join(", ");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{display_callee}` has argument type(s) ({actual}), expected {expected}."
                ),
                file,
                line,
            );
        }

        self.term_return_type(callee)
    }

    fn term_return_type(&mut self, callee: &str) -> Type {
        match builtins::term::resolve_call(callee) {
            Some(resolved) => self.parse_type(&resolved.return_type),
            None => Type::Unknown,
        }
    }

    fn check_thread_builtin_call(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arguments =
            self.normalize_builtin_call_arguments(file, display_callee, callee, arguments, line);
        let arg_types = arguments
            .iter()
            .enumerate()
            .map(|(index, argument)| {
                self.infer_expression(
                    file,
                    argument,
                    locals,
                    line,
                    self.thread_argument_mode(callee, index),
                )
            })
            .collect::<Vec<_>>();
        let arg_type_names = arg_types
            .iter()
            .map(|type_| self.type_name(type_))
            .collect::<Vec<_>>();

        if callee == "thread.start" {
            let valid_entry = match arguments.first() {
                Some(Expression::Identifier(name)) => {
                    let canonical_name = self.canonical_import_name(file, name);
                    self.lookup_visible_function(file, name)
                        .or_else(|| self.lookup_visible_function(file, &canonical_name))
                        .is_some_and(|sig| {
                            sig.imported_package_export
                                && matches!(sig.kind, FunctionKind::Func)
                                && sig.isolated
                        })
                }
                _ => false,
            };
            if !valid_entry {
                self.report(
                    "TYPE_CALL_ARGUMENT_MISMATCH",
                    "thread.start entry point must be an exported ISOLATED FUNC from an imported package.",
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        if let Some((min, max)) = builtins::thread::arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                let expected = if min == max {
                    min.to_string()
                } else {
                    format!("{min} to {max}")
                };
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{display_callee}` has {} argument(s), expected {expected}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        let Some(resolved) = builtins::thread::resolve_call(callee, &arg_type_names) else {
            let expected =
                builtins::thread::expected_arguments(callee).unwrap_or("supported overload");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{display_callee}` has argument type(s) ({}), expected {expected}.",
                    arg_type_names.join(", ")
                ),
                file,
                line,
            );
            return Type::Unknown;
        };

        let return_type = self.parse_type(&resolved.return_type);
        self.check_thread_boundary_sendability(
            file,
            display_callee,
            callee,
            &arg_types,
            &return_type,
            line,
        );
        return_type
    }

    fn check_strings_builtin_call(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arguments =
            self.normalize_builtin_call_arguments(file, display_callee, callee, arguments, line);
        let arg_types = arguments
            .iter()
            .map(|argument| {
                let type_ = self.infer_expression(file, argument, locals, line, ExprMode::Read);
                self.type_name(&type_)
            })
            .collect::<Vec<_>>();

        if let Some((min, max)) = builtins::strings::arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                let expected = if min == max {
                    min.to_string()
                } else {
                    format!("{min} to {max}")
                };
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{display_callee}` has {} argument(s), expected {expected}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        let Some(resolved) = builtins::strings::resolve_call(callee, &arg_types) else {
            let expected =
                builtins::strings::expected_arguments(callee).unwrap_or("supported overload");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{display_callee}` has argument type(s) ({}), expected {expected}.",
                    arg_types.join(", ")
                ),
                file,
                line,
            );
            return Type::Unknown;
        };

        self.parse_type(&resolved.return_type)
    }

    fn check_math_builtin_call(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arguments =
            self.normalize_builtin_call_arguments(file, display_callee, callee, arguments, line);
        let arg_types = arguments
            .iter()
            .map(|argument| {
                let type_ = self.infer_expression(file, argument, locals, line, ExprMode::Read);
                self.type_name(&type_)
            })
            .collect::<Vec<_>>();

        if let Some((min, max)) = builtins::math::arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                let expected = if min == max {
                    min.to_string()
                } else {
                    format!("{min} to {max}")
                };
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{display_callee}` has {} argument(s), expected {expected}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        let Some(resolved) = builtins::math::resolve_call(callee, &arg_types) else {
            let expected =
                builtins::math::expected_arguments(callee).unwrap_or("supported overload");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{display_callee}` has argument type(s) ({}), expected {expected}.",
                    arg_types.join(", ")
                ),
                file,
                line,
            );
            return Type::Unknown;
        };

        self.parse_type(&resolved.return_type)
    }

    fn check_bits_builtin_call(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arguments =
            self.normalize_builtin_call_arguments(file, display_callee, callee, arguments, line);
        let arg_types = arguments
            .iter()
            .map(|argument| {
                let type_ = self.infer_expression(file, argument, locals, line, ExprMode::Read);
                self.type_name(&type_)
            })
            .collect::<Vec<_>>();

        if let Some((min, max)) = builtins::bits::arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                let expected = if min == max {
                    min.to_string()
                } else {
                    format!("{min} to {max}")
                };
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{display_callee}` has {} argument(s), expected {expected}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        let Some(resolved) = builtins::bits::resolve_call(callee, &arg_types) else {
            let expected =
                builtins::bits::expected_arguments(callee).unwrap_or("supported overload");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{display_callee}` has argument type(s) ({}), expected {expected}.",
                    arg_types.join(", ")
                ),
                file,
                line,
            );
            return Type::Unknown;
        };

        self.parse_type(&resolved.return_type)
    }

    fn check_encoding_builtin_call(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
        expected: Option<&Type>,
    ) -> Type {
        let arguments =
            self.normalize_builtin_call_arguments(file, display_callee, callee, arguments, line);
        let arg_types = arguments
            .iter()
            .map(|argument| {
                let type_ = self.infer_expression(file, argument, locals, line, ExprMode::Read);
                self.type_name(&type_)
            })
            .collect::<Vec<_>>();

        if let Some((min, max)) = builtins::encoding::arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                let expected_count = if min == max {
                    min.to_string()
                } else {
                    format!("{min} to {max}")
                };
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{display_callee}` has {} argument(s), expected {expected_count}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        let Some(resolved) = builtins::encoding::resolve_call(callee, &arg_types) else {
            let expected_args =
                builtins::encoding::expected_arguments(callee).unwrap_or("supported overload");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{display_callee}` has argument type(s) ({}), expected {expected_args}.",
                    arg_types.join(", ")
                ),
                file,
                line,
            );
            return Type::Unknown;
        };

        // `utf8Encode` is a return-type overload (List OF Byte | List OF Integer).
        // When the call has an expected (contextual) type of one of the two, adopt
        // it; otherwise fall back to the default (List OF Byte). The hard
        // `TYPE_OVERLOAD_AMBIGUOUS` error for an unannotated call is raised later,
        // in the monomorphizer (plan-01-overload.md §F.2).
        if callee == "encoding.utf8Encode" {
            if let Some(expected) = expected {
                let expected_name = self.type_name(expected);
                if expected_name == "List OF Byte" || expected_name == "List OF Integer" {
                    return expected.clone();
                }
            }
        }

        self.parse_type(&resolved.return_type)
    }

    fn check_general_builtin_call(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arguments =
            self.normalize_builtin_call_arguments(file, display_callee, callee, arguments, line);
        if callee == "filter" && arguments.len() == 2 {
            if let Expression::Identifier(predicate) = &arguments[1] {
                if builtins::general::builtin_function_id(predicate).is_some() {
                    let collection_type =
                        self.infer_expression(file, &arguments[0], locals, line, ExprMode::Read);
                    let collection_type_name = self.type_name(&collection_type);
                    let predicate_type =
                        collection_type_name
                            .strip_prefix("List OF ")
                            .and_then(|element| {
                                builtins::general::filter_predicate_type(predicate, element)
                            });

                    let Some(predicate_type) = predicate_type else {
                        self.report(
                            "TYPE_CALL_ARGUMENT_MISMATCH",
                            &format!(
                                "Call to `{display_callee}` has argument type(s) ({collection_type_name}, {predicate}), expected {}.",
                                builtins::general::expected_arguments(callee)
                                    .unwrap_or("supported overload")
                            ),
                            file,
                            line,
                        );
                        return Type::Unknown;
                    };

                    let arg_types = vec![collection_type_name, predicate_type];
                    let Some(resolved) = builtins::general::resolve_call(callee, &arg_types) else {
                        self.report(
                            "TYPE_CALL_ARGUMENT_MISMATCH",
                            &format!(
                                "Call to `{display_callee}` has argument type(s) ({}), expected {}.",
                                arg_types.join(", "),
                                builtins::general::expected_arguments(callee)
                                    .unwrap_or("supported overload")
                            ),
                            file,
                            line,
                        );
                        return Type::Unknown;
                    };

                    return self.parse_type(&resolved.return_type);
                }
            }
        }

        let arg_types = arguments
            .iter()
            .enumerate()
            .map(|(index, argument)| {
                self.infer_expression(
                    file,
                    argument,
                    locals,
                    line,
                    self.general_argument_mode(callee, index),
                )
            })
            .collect::<Vec<_>>();
        let arg_type_names = arg_types
            .iter()
            .map(|type_| self.type_name(type_))
            .collect::<Vec<_>>();

        // A resource added to a collection through an update builtin must be a
        // `RES` binding (the owner); its slot holds a borrow (§15.6). The op
        // arrives qualified as `collections.append` after the §5 migration.
        if matches!(
            crate::builtins::collections::native_member_bare(callee),
            Some("append" | "prepend" | "insert" | "set")
        ) {
            for (index, (argument, arg_type)) in arguments.iter().zip(arg_types.iter()).enumerate()
            {
                if index == 0 {
                    continue;
                }
                self.check_collection_resource_element(
                    file, line, "element", argument, arg_type, locals,
                );
            }
        }

        if let Some((min, max)) = builtins::general::arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                let expected = if min == max {
                    min.to_string()
                } else {
                    format!("{min} to {max}")
                };
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{display_callee}` has {} argument(s), expected {expected}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        let Some(resolved) = builtins::general::resolve_call(callee, &arg_type_names) else {
            // The built-in rejected these argument types, so an override may fill
            // the gap (plan-01-overload.md §A.3.2). A *user* override has already
            // been rewritten to its mangled concrete symbol by the monomorphizer
            // (§B.1 / Phase 5), so it never reaches this bare-name path; only a
            // *package*-provided override (the registry, §B.2) is resolved here —
            // e.g. `toString(net::Url)` routes to the package's internal renderer
            // and yields the built-in's conventional result type.
            if builtins::general::is_overridable(callee)
                && arg_type_names.len() == 1
                && builtins::general_override_target(callee, &arg_type_names[0]).is_some()
            {
                return self.parse_type(
                    builtins::general::override_result_type(callee).unwrap_or("Unknown"),
                );
            }
            let expected =
                builtins::general::expected_arguments(callee).unwrap_or("supported overload");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{display_callee}` has argument type(s) ({}), expected {expected}.",
                    arg_type_names.join(", ")
                ),
                file,
                line,
            );
            return Type::Unknown;
        };

        self.check_general_builtin_comparability(file, display_callee, callee, &arg_types, line);

        self.parse_type(&resolved.return_type)
    }

    /// Typechecks a migrated `collections::` native member call (plan-01 §5).
    /// Mirrors `check_general_builtin_call` but resolves through the `collections`
    /// helper set; `callee` is the canonical `collections.<member>` form.
    fn check_collections_builtin_call(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let member = builtins::collections::native_member_bare(callee).unwrap_or(callee);
        let arguments =
            self.normalize_builtin_call_arguments(file, display_callee, callee, arguments, line);
        if callee == "collections.filter" && arguments.len() == 2 {
            if let Expression::Identifier(predicate) = &arguments[1] {
                if builtins::general::builtin_function_id(predicate).is_some() {
                    let collection_type =
                        self.infer_expression(file, &arguments[0], locals, line, ExprMode::Read);
                    let collection_type_name = self.type_name(&collection_type);
                    let predicate_type =
                        collection_type_name
                            .strip_prefix("List OF ")
                            .and_then(|element| {
                                builtins::general::filter_predicate_type(predicate, element)
                            });

                    let Some(predicate_type) = predicate_type else {
                        self.report(
                            "TYPE_CALL_ARGUMENT_MISMATCH",
                            &format!(
                                "Call to `{display_callee}` has argument type(s) ({collection_type_name}, {predicate}), expected {}.",
                                builtins::collections::expected_arguments(callee)
                                    .unwrap_or("supported overload")
                            ),
                            file,
                            line,
                        );
                        return Type::Unknown;
                    };

                    let arg_types = vec![collection_type_name, predicate_type];
                    let Some(resolved) = builtins::collections::resolve_call(callee, &arg_types)
                    else {
                        self.report(
                            "TYPE_CALL_ARGUMENT_MISMATCH",
                            &format!(
                                "Call to `{display_callee}` has argument type(s) ({}), expected {}.",
                                arg_types.join(", "),
                                builtins::collections::expected_arguments(callee)
                                    .unwrap_or("supported overload")
                            ),
                            file,
                            line,
                        );
                        return Type::Unknown;
                    };

                    return self.parse_type(&resolved.return_type);
                }
            }
        }

        let arg_types = arguments
            .iter()
            .enumerate()
            .map(|(index, argument)| {
                // License a `MUT` borrow for a lambda in a non-escaping callback
                // position (e.g. `forEach`'s action). `infer_lambda` consumes it;
                // reset afterward so a non-lambda argument never carries it.
                self.nonescaping_callback = builtins::is_nonescaping_callback_arg(member, index);
                let arg_type = self.infer_expression(
                    file,
                    argument,
                    locals,
                    line,
                    self.general_argument_mode(member, index),
                );
                self.nonescaping_callback = false;
                arg_type
            })
            .collect::<Vec<_>>();
        let arg_type_names = arg_types
            .iter()
            .map(|type_| self.type_name(type_))
            .collect::<Vec<_>>();

        if matches!(member, "append" | "prepend" | "insert" | "set") {
            for (index, (argument, arg_type)) in arguments.iter().zip(arg_types.iter()).enumerate()
            {
                if index == 0 {
                    continue;
                }
                self.check_collection_resource_element(
                    file, line, "element", argument, arg_type, locals,
                );
            }
        }

        if let Some((min, max)) = builtins::collections::arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                let expected = if min == max {
                    min.to_string()
                } else {
                    format!("{min} to {max}")
                };
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{display_callee}` has {} argument(s), expected {expected}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        let Some(resolved) = builtins::collections::resolve_call(callee, &arg_type_names) else {
            let expected =
                builtins::collections::expected_arguments(callee).unwrap_or("supported overload");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{display_callee}` has argument type(s) ({}), expected {expected}.",
                    arg_type_names.join(", ")
                ),
                file,
                line,
            );
            return Type::Unknown;
        };

        self.check_general_builtin_comparability(file, display_callee, member, &arg_types, line);

        self.parse_type(&resolved.return_type)
    }

    fn check_general_builtin_comparability(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arg_types: &[Type],
        line: usize,
    ) {
        match callee {
            "contains" | "replace" => {
                let Some(Type::List(element)) = arg_types.first() else {
                    return;
                };
                self.require_comparable_type(
                    file,
                    line,
                    &format!("Call to `{display_callee}`"),
                    element,
                );
            }
            "find" => {
                let Some(first) = arg_types.first() else {
                    return;
                };
                if let Type::List(element) = first {
                    self.require_comparable_type(
                        file,
                        line,
                        &format!("Call to `{display_callee}`"),
                        element,
                    );
                }
            }
            _ => {}
        }
    }

    fn normalize_builtin_call_arguments(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        line: usize,
    ) -> Vec<Expression> {
        if !arguments
            .iter()
            .any(|argument| matches!(argument, CallArg::Named { .. }))
        {
            return arguments
                .iter()
                .map(|argument| call_arg_value(argument).clone())
                .collect();
        }
        let Some(param_names) = builtins::call_param_names(callee) else {
            return arguments
                .iter()
                .map(|argument| call_arg_value(argument).clone())
                .collect();
        };
        let mut ordered = vec![None; param_names.len()];
        let mut next_positional = 0usize;
        let mut extras = Vec::new();
        let mut saw_unknown_named = false;
        for argument in arguments {
            match argument {
                CallArg::Positional(value) => {
                    while next_positional < ordered.len() && ordered[next_positional].is_some() {
                        next_positional += 1;
                    }
                    if next_positional < ordered.len() {
                        ordered[next_positional] = Some(value.clone());
                        next_positional += 1;
                    } else {
                        extras.push(value.clone());
                    }
                }
                CallArg::Named { name, value, line } => {
                    let Some(index) = param_names
                        .iter()
                        .position(|aliases| aliases.iter().any(|alias| alias == name))
                    else {
                        self.report(
                            "TYPE_UNKNOWN_ARGUMENT_NAME",
                            &format!(
                                "Call to `{display_callee}` does not have a parameter named `{name}`."
                            ),
                            file,
                            *line,
                        );
                        saw_unknown_named = true;
                        continue;
                    };
                    if ordered[index].is_some() {
                        self.report(
                            "TYPE_DUPLICATE_ARGUMENT_NAME",
                            &format!(
                                "Call to `{display_callee}` supplies parameter `{}` more than once.",
                                param_names[index][0]
                            ),
                            file,
                            *line,
                        );
                        continue;
                    }
                    ordered[index] = Some(value.clone());
                }
            }
        }
        if !saw_unknown_named {
            for (index, aliases) in param_names.iter().enumerate() {
                if ordered[index].is_none()
                    && ordered
                        .iter()
                        .skip(index + 1)
                        .any(|argument| argument.is_some())
                {
                    self.report(
                        "TYPE_CALL_ARITY_MISMATCH",
                        &format!(
                            "Call to `{display_callee}` omits parameter `{}` before a later supplied argument.",
                            aliases[0]
                        ),
                        file,
                        line,
                    );
                    break;
                }
            }
        }
        let mut normalized = ordered.into_iter().flatten().collect::<Vec<_>>();
        normalized.extend(extras);
        normalized
    }

    fn normalize_named_arguments(
        &mut self,
        file: &AstFile,
        callee: &str,
        arguments: &[CallArg],
        params: &[ParamSig],
        line: usize,
        allow_trailing_omission: bool,
    ) -> Vec<Option<Expression>> {
        let mut ordered = vec![None; params.len()];
        let mut next_positional = 0usize;
        let mut supplied = 0usize;
        let mut arity_error = false;

        for argument in arguments {
            match argument {
                CallArg::Positional(value) => {
                    while next_positional < ordered.len() && ordered[next_positional].is_some() {
                        next_positional += 1;
                    }
                    if next_positional >= ordered.len() {
                        arity_error = true;
                        continue;
                    }
                    ordered[next_positional] = Some(value.clone());
                    next_positional += 1;
                    supplied += 1;
                }
                CallArg::Named { name, value, line } => {
                    let Some(index) = params.iter().position(|param| param.name == *name) else {
                        self.report(
                            "TYPE_UNKNOWN_ARGUMENT_NAME",
                            &format!(
                                "Call to `{callee}` does not have a parameter named `{name}`."
                            ),
                            file,
                            *line,
                        );
                        continue;
                    };
                    if ordered[index].is_some() {
                        self.report(
                            "TYPE_DUPLICATE_ARGUMENT_NAME",
                            &format!(
                                "Call to `{callee}` supplies parameter `{name}` more than once."
                            ),
                            file,
                            *line,
                        );
                        continue;
                    }
                    ordered[index] = Some(value.clone());
                    supplied += 1;
                }
            }
        }

        let required = params.iter().filter(|param| !param.has_default).count();
        let missing_required = ordered
            .iter()
            .zip(params.iter())
            .any(|(argument, param)| argument.is_none() && !param.has_default);
        let max_supplied = ordered
            .iter()
            .rposition(Option::is_some)
            .map(|index| index + 1)
            .unwrap_or(0);
        let has_internal_gap = allow_trailing_omission
            && ordered
                .iter()
                .zip(params.iter())
                .take(max_supplied)
                .any(|(argument, param)| argument.is_none() && !param.has_default);

        if arity_error
            || supplied < required
            || supplied > params.len()
            || missing_required
            || has_internal_gap
        {
            self.report(
                "TYPE_CALL_ARITY_MISMATCH",
                &format!(
                    "Call to `{callee}` has {} argument(s), expected {} to {}.",
                    supplied,
                    required,
                    params.len()
                ),
                file,
                line,
            );
        }

        ordered
    }

    /// Parse a collection element / `Map` value type, honoring the `RES` marker
    /// (`List OF RES File`). The marker wraps the element in [`Type::Res`]; the
    /// element validation later checks it matches the element's resource-ness.
    fn parse_collection_element_type(&self, name: &str) -> Type {
        if let Some(inner) = name.strip_prefix("RES ") {
            return Type::Res(Box::new(self.parse_type(inner)));
        }
        self.parse_type(name)
    }

    fn parse_type(&self, name: &str) -> Type {
        let name = builtins::thread::strip_type_group(name);
        // A package-qualified built-in type (`net.Url`, `http.Result`) resolves to
        // its bare internal id (plan-03-http.md §A.1/§B.2).
        if let Some(bare) = builtins::qualified_builtin_type(name) {
            return Type::User(bare);
        }
        if let Some(rest) = name.strip_prefix("ISOLATED FUNC(") {
            return self.parse_function_type(rest, true);
        }
        if let Some(rest) = name.strip_prefix("FUNC(") {
            return self.parse_function_type(rest, false);
        }
        if let Some(element) = name.strip_prefix("List OF ") {
            return Type::List(Box::new(self.parse_collection_element_type(element)));
        }
        if let Some(success) = name.strip_prefix("Result OF ") {
            return Type::Result(Box::new(self.parse_type(success)));
        }
        if let Some((kind, message, resource, output)) = builtins::thread::thread_parts_full(name) {
            let resource = resource.map(|resource| Box::new(self.parse_type(resource)));
            if kind == builtins::thread::THREAD_WORKER_TYPE {
                return Type::ThreadWorker(
                    Box::new(self.parse_type(message)),
                    resource,
                    Box::new(self.parse_type(output)),
                );
            }
            return Type::Thread(
                Box::new(self.parse_type(message)),
                resource,
                Box::new(self.parse_type(output)),
            );
        }
        if let Some(rest) = name.strip_prefix("Map OF ") {
            if let Some((key, value)) = rest.split_once(" TO ") {
                return Type::Map(
                    Box::new(self.parse_type(key)),
                    Box::new(self.parse_collection_element_type(value)),
                );
            }
        }

        match name {
            "Boolean" => Type::Boolean,
            "Byte" => Type::Byte,
            "Error" => Type::Error,
            "ErrorLoc" => Type::ErrorLoc,
            "Fixed" => Type::Fixed,
            "Float" => Type::Float,
            "Integer" => Type::Integer,
            "Nothing" => Type::Nothing,
            "String" => Type::String,
            "Unknown" => Type::Unknown,
            "Result" => Type::Result(Box::new(Type::Unknown)),
            other if builtins::is_builtin_type(other) => Type::User(other.to_string()),
            other if self.user_types.contains(other) => Type::User(other.to_string()),
            other => Type::User(other.to_string()),
        }
    }

    fn parse_function_type(&self, rest: &str, isolated: bool) -> Type {
        let Some((params, return_type)) = rest.split_once(") AS ") else {
            return Type::Unknown;
        };
        let params = if params.trim().is_empty() {
            Vec::new()
        } else {
            params
                .split(", ")
                .map(|param| self.parse_type(param))
                .collect()
        };
        Type::Function {
            params,
            return_type: Box::new(self.parse_type(return_type)),
            isolated,
        }
    }

    fn compatible(&self, expected: &Type, actual: &Type) -> bool {
        if matches!(expected, Type::Unknown) || matches!(actual, Type::Unknown) {
            return true;
        }
        // The `RES` element marker is an ownership-axis annotation (§15.6), not a
        // distinct value type: a `File` value fits a `RES File` slot and vice
        // versa. Strip it before comparing.
        let (expected, actual) = (strip_res(expected), strip_res(actual));
        match (expected, actual) {
            (Type::List(expected), Type::List(actual)) => self.compatible(expected, actual),
            (Type::Map(expected_key, expected_value), Type::Map(actual_key, actual_value)) => {
                self.compatible(expected_key, actual_key)
                    && self.compatible(expected_value, actual_value)
            }
            (Type::Result(expected), Type::Result(actual)) => self.compatible(expected, actual),
            (
                Type::Thread(expected_message, expected_resource, expected_output),
                Type::Thread(actual_message, actual_resource, actual_output),
            ) => {
                self.compatible(expected_message, actual_message)
                    && self.compatible_optional(expected_resource, actual_resource)
                    && self.compatible(expected_output, actual_output)
            }
            (
                Type::ThreadWorker(expected_message, expected_resource, expected_output),
                Type::ThreadWorker(actual_message, actual_resource, actual_output),
            ) => {
                self.compatible(expected_message, actual_message)
                    && self.compatible_optional(expected_resource, actual_resource)
                    && self.compatible(expected_output, actual_output)
            }
            (
                Type::Function {
                    params: expected_params,
                    return_type: expected_return,
                    isolated: expected_isolated,
                },
                Type::Function {
                    params: actual_params,
                    return_type: actual_return,
                    isolated: actual_isolated,
                },
            ) => {
                (!expected_isolated || *actual_isolated)
                    && expected_params.len() == actual_params.len()
                    && expected_params
                        .iter()
                        .zip(actual_params.iter())
                        .all(|(expected, actual)| self.compatible(expected, actual))
                    && self.compatible(expected_return, actual_return)
            }
            (Type::User(expected_name), Type::User(actual_name)) => {
                // An imported package's types are registered under their bare name
                // (`Db`), while a qualified reference written by the importer
                // resolves to `binding.Db` (plan-link-update.md §5a). Treat a
                // qualified name as equal to its bare form so an imported
                // resource/user type returned from a package function matches the
                // importer's `binding::Type` annotation.
                let expected_bare = expected_name.rsplit('.').next().unwrap_or(expected_name);
                let actual_bare = actual_name.rsplit('.').next().unwrap_or(actual_name);
                expected_name == actual_name
                    || expected_bare == actual_bare
                    || self
                        .type_infos
                        .get(expected_name)
                        .or_else(|| self.type_infos.get(expected_bare))
                        .is_some_and(|info| {
                            matches!(info.kind, TypeDeclKind::Union)
                                && info
                                    .variants
                                    .iter()
                                    .any(|variant| variant.name == *actual_bare)
                        })
            }
            _ => expected == actual,
        }
    }

    /// Compatibility for the optional resource plane of a thread type: both
    /// absent, or both present and compatible.
    fn compatible_optional(
        &self,
        expected: &Option<Box<Type>>,
        actual: &Option<Box<Type>>,
    ) -> bool {
        match (expected, actual) {
            (None, None) => true,
            (Some(expected), Some(actual)) => self.compatible(expected, actual),
            _ => false,
        }
    }

    fn expression_compatible(
        &self,
        expected: &Type,
        actual: &Type,
        expression: Option<&Expression>,
    ) -> bool {
        if self.compatible(expected, actual) {
            return true;
        }
        match (expected, actual, expression) {
            (Type::Byte, Type::Integer, Some(Expression::Number(value))) => value
                .parse::<u16>()
                .is_ok_and(|number| number <= u8::MAX as u16),
            (Type::Fixed, Type::Integer | Type::Float, Some(Expression::Number(_))) => true,
            (
                Type::Fixed,
                Type::Integer | Type::Float,
                Some(Expression::Unary {
                    operator, operand, ..
                }),
            ) if operator == "-" && matches!(operand.as_ref(), Expression::Number(_)) => true,
            (
                Type::List(expected_element),
                Type::List(_),
                Some(Expression::ListLiteral(values)),
            ) => values.iter().all(|value| {
                let Some(actual_element) = numeric_literal_type(value) else {
                    return false;
                };
                self.expression_compatible(expected_element, &actual_element, Some(value))
            }),
            _ => false,
        }
    }

    fn is_numeric(&self, type_: &Type) -> bool {
        matches!(
            type_,
            Type::Byte | Type::Fixed | Type::Float | Type::Integer | Type::Unknown
        )
    }

    fn is_comparable(&self, type_: &Type) -> bool {
        self.is_comparable_with_seen(type_, &mut HashSet::new())
    }

    /// An operand acceptable on either side of a `String` ordering comparison
    /// (`<`, `>`, `<=`, `>=`). `Unknown` is permitted so a prior error does not
    /// cascade. Numeric operands are handled separately by `is_numeric`.
    fn is_orderable_string(&self, type_: &Type) -> bool {
        matches!(type_, Type::String | Type::Unknown)
    }

    fn is_comparable_with_seen(&self, type_: &Type, seen: &mut HashSet<String>) -> bool {
        match type_ {
            Type::Boolean
            | Type::Byte
            | Type::Error
            | Type::ErrorLoc
            | Type::Fixed
            | Type::Float
            | Type::Integer
            | Type::Nothing
            | Type::String
            | Type::Unknown => true,
            Type::List(_)
            | Type::Map(_, _)
            | Type::Function { .. }
            | Type::Result(_)
            | Type::Res(_)
            | Type::Thread(..)
            | Type::ThreadWorker(..) => false,
            Type::User(name) => {
                if self.resource_registry.is_resource(name) || !seen.insert(name.clone()) {
                    return false;
                }
                let Some(info) = self.type_infos.get(name) else {
                    return true;
                };
                let result = match info.kind {
                    TypeDeclKind::Enum => true,
                    TypeDeclKind::Type => info
                        .fields
                        .iter()
                        .all(|field| self.is_comparable_with_seen(&field.type_, seen)),
                    TypeDeclKind::Union => false,
                };
                seen.remove(name);
                result
            }
        }
    }

    fn require_comparable_type(
        &mut self,
        file: &AstFile,
        line: usize,
        context: &str,
        type_: &Type,
    ) {
        if self.is_comparable(type_) {
            return;
        }
        self.report(
            "TYPE_REQUIRES_COMPARABLE",
            &format!(
                "{context} requires a comparable type, got `{}`.",
                self.type_name(type_)
            ),
            file,
            line,
        );
    }

    /// The argument mode for argument `index` of a call to `callee`. A call to a
    /// resource's *registered close op* consumes its single resource argument
    /// (overhaul invalidation event #1) — for native LINK resources this is the
    /// `LINK` CLOSE wrapper (plan-link-update.md §6). All other resource arguments
    /// borrow by default.
    fn call_argument_mode(&self, callee: &str, index: usize, sig: &FunctionSig) -> ExprMode {
        let param_type = sig.params.get(index).map(|param| &param.type_);
        if index == 0 {
            if let Some(Type::User(name)) = param_type {
                let base = builtins::resource::base_resource_name(name);
                let is_close_op = self.resource_registry.close_function(base) == Some(callee)
                    || self.resource_registry.close_function(name.as_str()) == Some(callee)
                    // A re-export alias of the close op consumes too (§5a).
                    || self
                        .close_op_aliases
                        .get(callee)
                        .is_some_and(|type_name| type_name == base || type_name == name);
                if is_close_op {
                    return ExprMode::Transfer;
                }
            }
        }
        self.argument_mode_for_type(&param_type)
    }

    fn argument_mode_for_type(&self, expected: &Option<&Type>) -> ExprMode {
        match expected {
            // Resources borrow by default: an ordinary call uses the handle for
            // the duration of the call but does not take ownership. Only the
            // fixed invalidation events (a registered close op, `thread::transfer`,
            // `RETURN`, and scope-drop) end a resource's life.
            Some(type_) if self.is_resource_type(type_) => ExprMode::Borrow,
            Some(type_) if !self.is_copyable_type(type_) => ExprMode::Transfer,
            _ => ExprMode::Read,
        }
    }

    fn thread_argument_mode(&self, callee: &str, index: usize) -> ExprMode {
        match (callee, index) {
            // `thread.transfer` is resource-plane invalidation event #2: the
            // resource moves to the worker, so the sender binding is consumed.
            ("thread.start", 1) | ("thread.send", 1) | ("thread.transfer", 1) => ExprMode::Transfer,
            ("thread.start", _) | ("thread.send", _) | ("thread.transfer", _) => ExprMode::Borrow,
            _ => ExprMode::Borrow,
        }
    }

    /// Argument evaluation mode for a builtin collection op, keyed on the BARE op
    /// name. Callers pass the dequalified member (`append`, not
    /// `collections.append`); this is only ever reached for recognised builtin
    /// calls, so a freed bare name from user code never gets here
    /// (plan-01-functions.md §5).
    fn general_argument_mode(&self, callee: &str, index: usize) -> ExprMode {
        if matches!(
            callee,
            "len"
                | "get"
                | "getOr"
                | "find"
                | "keys"
                | "values"
                | "hasKey"
                | "contains"
                | "forEach"
                | "transform"
                | "filter"
                | "reduce"
                | "sum"
        ) {
            return ExprMode::Read;
        }
        if matches!(
            callee,
            "removeAt" | "removeKey" | "replace" | "set" | "append" | "prepend" | "insert"
        ) {
            return if index == 0 {
                ExprMode::Transfer
            } else {
                ExprMode::Read
            };
        }
        ExprMode::Read
    }

    fn is_resource_type(&self, type_: &Type) -> bool {
        match type_ {
            Type::User(name) => {
                self.resource_registry.is_resource(name) || self.is_resource_union(name)
            }
            // A `RES`-marked element (`RES File`) is a resource (a borrow of one).
            Type::Res(inner) => self.is_resource_type(inner),
            _ => false,
        }
    }

    /// A union whose every variant is a resource type is itself a resource (a
    /// resource union): move-only, `RES`-bound, dropped by dispatching on the
    /// tag to the active variant's close op. Variants are bare resource types.
    fn is_resource_union(&self, name: &str) -> bool {
        let Some(info) = self.type_infos.get(name) else {
            return false;
        };
        matches!(info.kind, TypeDeclKind::Union)
            && !info.variants.is_empty()
            && info
                .variants
                .iter()
                .all(|variant| self.resource_registry.is_resource(&variant.name))
    }

    fn contains_resource_or_thread(&self, type_: &Type) -> bool {
        self.contains_resource_or_thread_with_seen(type_, &mut HashSet::new())
    }

    /// Whether a type transitively contains a thread handle. Threads may never
    /// live in a collection; resources may (as borrows, §15.6), so collection
    /// element and `Map` *value* positions use this rather than the combined
    /// resource-or-thread predicate.
    fn contains_thread(&self, type_: &Type) -> bool {
        self.contains_thread_with_seen(type_, &mut HashSet::new())
    }

    fn contains_thread_with_seen(&self, type_: &Type, seen: &mut HashSet<String>) -> bool {
        match type_ {
            Type::Thread(..) | Type::ThreadWorker(..) => true,
            Type::List(element) => self.contains_thread_with_seen(element, seen),
            Type::Map(key, value) => {
                self.contains_thread_with_seen(key, seen)
                    || self.contains_thread_with_seen(value, seen)
            }
            Type::Result(success) => self.contains_thread_with_seen(success, seen),
            Type::Res(inner) => self.contains_thread_with_seen(inner, seen),
            Type::User(name) => {
                if !seen.insert(name.clone()) {
                    return false;
                }
                let Some(info) = self.type_infos.get(name) else {
                    return false;
                };
                let result = match info.kind {
                    TypeDeclKind::Enum => false,
                    TypeDeclKind::Type => info
                        .fields
                        .iter()
                        .any(|field| self.contains_thread_with_seen(&field.type_, seen)),
                    TypeDeclKind::Union => info.variants.iter().any(|variant| {
                        variant
                            .fields
                            .iter()
                            .any(|field| self.contains_thread_with_seen(&field.type_, seen))
                    }),
                };
                seen.remove(name);
                result
            }
            _ => false,
        }
    }

    fn contains_resource_or_thread_with_seen(
        &self,
        type_: &Type,
        seen: &mut HashSet<String>,
    ) -> bool {
        match type_ {
            Type::Thread(..) | Type::ThreadWorker(..) => true,
            Type::User(name) if self.resource_registry.is_resource(name) => true,
            Type::List(element) => self.contains_resource_or_thread_with_seen(element, seen),
            Type::Map(key, value) => {
                self.contains_resource_or_thread_with_seen(key, seen)
                    || self.contains_resource_or_thread_with_seen(value, seen)
            }
            Type::Result(success) => self.contains_resource_or_thread_with_seen(success, seen),
            Type::Res(inner) => self.contains_resource_or_thread_with_seen(inner, seen),
            Type::Function { .. } => false,
            Type::User(name) => {
                if !seen.insert(name.clone()) {
                    return false;
                }
                let Some(info) = self.type_infos.get(name) else {
                    return false;
                };
                let result = match info.kind {
                    TypeDeclKind::Enum => false,
                    TypeDeclKind::Type => info.fields.iter().any(|field| {
                        self.contains_resource_or_thread_with_seen(&field.type_, seen)
                    }),
                    TypeDeclKind::Union => info.variants.iter().any(|variant| {
                        variant.fields.iter().any(|field| {
                            self.contains_resource_or_thread_with_seen(&field.type_, seen)
                        })
                    }),
                };
                seen.remove(name);
                result
            }
            _ => false,
        }
    }

    fn report_invalid_collection_element(
        &mut self,
        file: &AstFile,
        line: usize,
        role: &str,
        type_: &Type,
    ) {
        self.report(
            "TYPE_COLLECTION_OWNERSHIP_VIOLATION",
            &format!(
                "Ordinary collections cannot store {role} values of type `{}` because they contain a resource or thread handle.",
                self.type_name(type_)
            ),
            file,
            line,
        );
    }

    /// Enforce the `RES` ownership axis on a collection element / `Map` value
    /// type (§15.6): a resource element must be marked `RES` (`List OF RES File`),
    /// and `RES` may mark only a resource — exactly as for a `RES` binding or
    /// parameter. `role` is "element" or "value".
    fn check_collection_element_axis(
        &mut self,
        file: &AstFile,
        line: usize,
        role: &str,
        element: &Type,
    ) {
        let is_res_marked = matches!(element, Type::Res(_));
        let inner = strip_res(element);
        let is_resource = self.is_resource_type(inner);
        if is_resource && !is_res_marked {
            self.report(
                "TYPE_RESOURCE_REQUIRES_RES",
                &format!(
                    "Collection {role} type `{}` is a resource; mark it `RES` (e.g. `List OF RES File`), not a bare resource type.",
                    self.type_name(inner)
                ),
                file,
                line,
            );
        } else if is_res_marked && !is_resource {
            self.report(
                "TYPE_RES_REQUIRES_RESOURCE",
                &format!(
                    "Collection {role} is marked `RES` but `{}` is not a resource type; drop the `RES`.",
                    self.type_name(inner)
                ),
                file,
                line,
            );
        }
    }

    /// A `List` element or `Map` value may hold a *borrow* of a resource, but
    /// only of a named `RES` binding (the owner); a temporary or a borrowed
    /// element (e.g. a `get`/`FOR EACH` result) is not an owner and cannot be
    /// stored (§15.6).
    fn check_collection_resource_element(
        &mut self,
        file: &AstFile,
        line: usize,
        role: &str,
        value: &Expression,
        type_: &Type,
        locals: &HashMap<String, LocalInfo>,
    ) {
        if !self.is_resource_type(type_) {
            return;
        }
        if self.collection_element_is_resource_binding(value, locals) {
            return;
        }
        self.report(
            "TYPE_RESOURCE_ELEMENT_NOT_OWNER",
            &format!(
                "Only a `RES` binding may be added as a collection {role}; `{}` is a temporary or borrowed resource, not an owner. Bind it with `RES` first (§15.6).",
                self.type_name(type_)
            ),
            file,
            line,
        );
    }

    /// Whether `value` is an identifier naming a resource `RES` binding or
    /// parameter — the only resource expression that may be stored in a
    /// collection (its slot holds a borrow of that binding).
    fn collection_element_is_resource_binding(
        &self,
        value: &Expression,
        locals: &HashMap<String, LocalInfo>,
    ) -> bool {
        let Expression::Identifier(name) = value else {
            return false;
        };
        locals
            .get(name)
            .is_some_and(|info| self.is_resource_type(&info.type_))
    }

    /// The expression mode for a collection element: a resource binding is a
    /// borrow (it stays usable after insertion), everything else is consumed.
    fn collection_element_mode(
        &self,
        value: &Expression,
        locals: &HashMap<String, LocalInfo>,
    ) -> ExprMode {
        if self.collection_element_is_resource_binding(value, locals) {
            ExprMode::Borrow
        } else {
            ExprMode::Transfer
        }
    }

    fn is_copyable_type(&self, type_: &Type) -> bool {
        self.is_copyable_type_with_seen(type_, &mut HashSet::new())
    }

    fn is_thread_sendable_type(&self, type_: &Type) -> bool {
        self.is_thread_sendable_type_with_seen(type_, &mut HashSet::new())
    }

    fn is_defaultable_type(&self, type_: &Type) -> bool {
        self.is_defaultable_type_with_seen(type_, &mut HashSet::new())
    }

    fn is_defaultable_type_with_seen(&self, type_: &Type, seen: &mut HashSet<String>) -> bool {
        match type_ {
            Type::Boolean
            | Type::Byte
            | Type::Error
            | Type::ErrorLoc
            | Type::Fixed
            | Type::Float
            | Type::Integer
            | Type::Nothing
            | Type::String
            | Type::Unknown => true,
            Type::List(element) => self.is_defaultable_type_with_seen(element, seen),
            Type::Map(key, value) => {
                self.is_defaultable_type_with_seen(key, seen)
                    && self.is_defaultable_type_with_seen(value, seen)
            }
            Type::Function { .. }
            | Type::Result(_)
            | Type::Res(_)
            | Type::Thread(..)
            | Type::ThreadWorker(..) => false,
            Type::User(name) => {
                if self.resource_registry.is_resource(name) {
                    return false;
                }
                if !seen.insert(name.clone()) {
                    return false;
                }
                let Some(info) = self.type_infos.get(name) else {
                    return false;
                };
                let result = match info.kind {
                    TypeDeclKind::Enum | TypeDeclKind::Union => false,
                    TypeDeclKind::Type => info
                        .fields
                        .iter()
                        .all(|field| self.is_defaultable_type_with_seen(&field.type_, seen)),
                };
                seen.remove(name);
                result
            }
        }
    }

    fn is_copyable_type_with_seen(&self, type_: &Type, seen: &mut HashSet<String>) -> bool {
        match type_ {
            Type::Boolean
            | Type::Byte
            | Type::Error
            | Type::ErrorLoc
            | Type::Fixed
            | Type::Float
            | Type::Integer
            | Type::Nothing
            | Type::String
            | Type::Unknown => true,
            // A collection slot holds a *borrow* of a resource (`RES File`),
            // which copies freely — copying the collection makes more borrows,
            // never another resource. A standalone resource stays non-copyable
            // (the `Type::User` arm below); §15.6.
            Type::Res(_) => true,
            Type::List(element) => self.is_copyable_type_with_seen(element, seen),
            Type::Map(key, value) => {
                self.is_copyable_type_with_seen(key, seen)
                    && self.is_copyable_type_with_seen(value, seen)
            }
            Type::Result(success) => self.is_copyable_type_with_seen(success, seen),
            Type::Function { .. } => true,
            Type::Thread(..) | Type::ThreadWorker(..) => false,
            Type::User(name) => {
                if self.resource_registry.is_resource(name) {
                    return false;
                }
                if !seen.insert(name.clone()) {
                    return true;
                }
                let Some(info) = self.type_infos.get(name) else {
                    return true;
                };
                let result = match info.kind {
                    TypeDeclKind::Enum => true,
                    TypeDeclKind::Type => info
                        .fields
                        .iter()
                        .all(|field| self.is_copyable_type_with_seen(&field.type_, seen)),
                    TypeDeclKind::Union => info.variants.iter().all(|variant| {
                        variant
                            .fields
                            .iter()
                            .all(|field| self.is_copyable_type_with_seen(&field.type_, seen))
                    }),
                };
                seen.remove(name);
                result
            }
        }
    }

    fn is_thread_sendable_type_with_seen(&self, type_: &Type, seen: &mut HashSet<String>) -> bool {
        match type_ {
            Type::Boolean
            | Type::Byte
            | Type::Error
            | Type::ErrorLoc
            | Type::Fixed
            | Type::Float
            | Type::Integer
            | Type::Nothing
            | Type::String
            | Type::Unknown => true,
            Type::List(element) => self.is_thread_sendable_type_with_seen(element, seen),
            Type::Map(key, value) => {
                self.is_thread_sendable_type_with_seen(key, seen)
                    && self.is_thread_sendable_type_with_seen(value, seen)
            }
            Type::Result(success) => self.is_thread_sendable_type_with_seen(success, seen),
            // Sharing a resource collection across threads is out of scope (§15.6).
            Type::Res(_) => false,
            Type::Function { .. } | Type::Thread(..) | Type::ThreadWorker(..) => false,
            Type::User(name) => {
                if self.resource_registry.is_resource(name) {
                    return self.resource_registry.is_sendable(name);
                }
                if !seen.insert(name.clone()) {
                    return true;
                }
                let Some(info) = self.type_infos.get(name) else {
                    return true;
                };
                let result =
                    match info.kind {
                        TypeDeclKind::Enum => true,
                        TypeDeclKind::Type => info.fields.iter().all(|field| {
                            self.is_thread_sendable_type_with_seen(&field.type_, seen)
                        }),
                        TypeDeclKind::Union => info.variants.iter().all(|variant| {
                            variant.fields.iter().all(|field| {
                                self.is_thread_sendable_type_with_seen(&field.type_, seen)
                            })
                        }),
                    };
                seen.remove(name);
                result
            }
        }
    }

    fn report_thread_type_not_sendable(
        &mut self,
        file: &AstFile,
        line: usize,
        context: &str,
        type_: &Type,
    ) {
        self.report(
            "TYPE_THREAD_NOT_SENDABLE",
            &format!(
                "{context} requires a thread-sendable type, got `{}`.",
                self.type_name(type_)
            ),
            file,
            line,
        );
    }

    fn require_thread_sendable_type(
        &mut self,
        file: &AstFile,
        line: usize,
        context: &str,
        type_: &Type,
    ) {
        if !self.is_thread_sendable_type(type_) {
            self.report_thread_type_not_sendable(file, line, context, type_);
        }
    }

    fn check_thread_boundary_sendability(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arg_types: &[Type],
        return_type: &Type,
        line: usize,
    ) {
        match callee {
            "thread.start" => {
                if let Some(input) = arg_types.get(1) {
                    self.require_thread_sendable_type(
                        file,
                        line,
                        &format!("Call to `{display_callee}` input"),
                        input,
                    );
                }
                if let Type::Thread(message, resource, output) = return_type {
                    self.require_thread_sendable_type(
                        file,
                        line,
                        &format!("Call to `{display_callee}` message type"),
                        message,
                    );
                    if let Some(resource) = resource {
                        // The resource plane carries only thread-sendable resources.
                        self.require_thread_sendable_type(
                            file,
                            line,
                            &format!("Call to `{display_callee}` resource type"),
                            resource,
                        );
                    }
                    self.require_thread_sendable_type(
                        file,
                        line,
                        &format!("Call to `{display_callee}` output type"),
                        output,
                    );
                }
            }
            "thread.send" => {
                if let Some(handle) = arg_types.first() {
                    match handle {
                        Type::Thread(message, _, _) | Type::ThreadWorker(message, _, _) => {
                            self.require_thread_sendable_type(
                                file,
                                line,
                                &format!("Call to `{display_callee}` message type"),
                                message,
                            );
                            // The data plane is resource-free: a resource moves
                            // across a thread only via `thread::transfer` (§7).
                            if self.is_resource_type(message) {
                                self.report(
                                    "TYPE_THREAD_NOT_SENDABLE",
                                    &format!(
                                        "Call to `{display_callee}` message type `{}` is a resource; the message channel is resource-free — use `thread::transfer`.",
                                        self.type_name(message)
                                    ),
                                    file,
                                    line,
                                );
                            }
                        }
                        _ => {}
                    }
                }
            }
            "thread.transfer" | "thread.accept" => {
                if let Some(handle) = arg_types.first() {
                    if let Type::Thread(_, resource, _) | Type::ThreadWorker(_, resource, _) =
                        handle
                    {
                        match resource {
                            // The resource plane carries only thread-sendable
                            // resources, and only when the thread declares one.
                            Some(resource) if self.is_resource_type(resource) => {
                                self.require_thread_sendable_type(
                                    file,
                                    line,
                                    &format!("Call to `{display_callee}` resource type"),
                                    resource,
                                );
                            }
                            Some(resource) => {
                                self.report(
                                    "TYPE_THREAD_NOT_SENDABLE",
                                    &format!(
                                        "Call to `{display_callee}` carries `{}`, which is not a resource; the resource plane moves only resources.",
                                        self.type_name(resource)
                                    ),
                                    file,
                                    line,
                                );
                            }
                            None => {
                                self.report(
                                    "TYPE_THREAD_NOT_SENDABLE",
                                    &format!(
                                        "Call to `{display_callee}` requires a thread with a resource plane (`Thread OF … RES Res TO …`); this thread has no resource channel."
                                    ),
                                    file,
                                    line,
                                );
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn visible_from(&self, file: &AstFile, visibility: Visibility, owner_file_path: &str) -> bool {
        match visibility {
            Visibility::Export | Visibility::Package => true,
            Visibility::Private => file.path == owner_file_path,
        }
    }

    fn check_type_reference(&mut self, file: &AstFile, type_: &Type, line: usize) {
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

    fn type_name(&self, type_: &Type) -> String {
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

    fn thread_type_argument_name(&self, type_: &Type) -> String {
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
    fn format_thread_type_name(
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

    fn report(&mut self, rule: &str, detail: &str, file: &AstFile, line: usize) {
        self.had_error = true;
        rules::show_diagnostic(rule, detail, &self.project_dir.join(&file.path), line, 1, 1);
    }

    fn report_primitive_literal_range_error(
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

enum ByteLiteralRangeError<'a> {
    Overflow(&'a str),
    Underflow(String),
}

enum SignedLiteralRangeError {
    Overflow(String),
    Underflow(String),
}

fn byte_literal_range_error(expression: &Expression) -> Option<ByteLiteralRangeError<'_>> {
    match expression {
        Expression::Number(value) if !value.contains('.') => value
            .parse::<u16>()
            .map_or(Some(ByteLiteralRangeError::Overflow(value)), |number| {
                (number > u8::MAX as u16).then_some(ByteLiteralRangeError::Overflow(value))
            }),
        Expression::Unary {
            operator, operand, ..
        } if operator == "-" => {
            let Expression::Number(value) = operand.as_ref() else {
                return None;
            };
            if value.contains('.') {
                return None;
            }
            let Ok(number) = value.parse::<u128>() else {
                return Some(ByteLiteralRangeError::Underflow(format!("-{value}")));
            };
            (number != 0).then_some(ByteLiteralRangeError::Underflow(format!("-{value}")))
        }
        _ => None,
    }
}

fn float_literal_range_error(expression: &Expression) -> Option<SignedLiteralRangeError> {
    let (text, negative) = signed_numeric_literal(expression)?;
    let parsed = text.parse::<f64>().ok()?;
    if parsed.is_finite() {
        return None;
    }
    if negative {
        Some(SignedLiteralRangeError::Underflow(format!("-{text}")))
    } else {
        Some(SignedLiteralRangeError::Overflow(text.to_string()))
    }
}

fn fixed_literal_range_error(expression: &Expression) -> Option<SignedLiteralRangeError> {
    let (text, negative) = signed_numeric_literal(expression)?;
    let parsed = text.parse::<f64>().ok()?;
    let value = if negative { -parsed } else { parsed };
    if value < -2147483648.0 {
        Some(SignedLiteralRangeError::Underflow(format!("-{text}")))
    } else if value >= 2147483648.0 {
        Some(SignedLiteralRangeError::Overflow(text.to_string()))
    } else {
        None
    }
}

fn statement_line(statement: &Statement) -> usize {
    match statement {
        Statement::Let { line, .. }
        | Statement::Return { line, .. }
        | Statement::Exit { line, .. }
        | Statement::Continue { line, .. }
        | Statement::Fail { line, .. }
        | Statement::Propagate { line }
        | Statement::Recover { line, .. }
        | Statement::Assign { line, .. }
        | Statement::StateAssign { line, .. }
        | Statement::Expression { line, .. }
        | Statement::If { line, .. }
        | Statement::Match { line, .. }
        | Statement::For { line, .. }
        | Statement::ForEach { line, .. }
        | Statement::While { line, .. }
        | Statement::DoUntil { line, .. } => *line,
    }
}

fn loop_kind_keyword(kind: LoopKind) -> &'static str {
    match kind {
        LoopKind::For => "FOR",
        LoopKind::Do => "DO",
        LoopKind::While => "WHILE",
    }
}

fn integer_constant_value(expression: &Expression) -> Option<i128> {
    match expression {
        Expression::Number(value) => value.parse::<i128>().ok(),
        Expression::Unary {
            operator, operand, ..
        } if operator == "-" => integer_constant_value(operand).map(|value| -value),
        _ => None,
    }
}

fn signed_numeric_literal(expression: &Expression) -> Option<(&str, bool)> {
    match expression {
        Expression::Number(value) => Some((value.as_str(), false)),
        Expression::Unary {
            operator, operand, ..
        } if operator == "-" => {
            let Expression::Number(value) = operand.as_ref() else {
                return None;
            };
            Some((value.as_str(), true))
        }
        _ => None,
    }
}

fn integer_literal_in_range(expression: &Expression) -> bool {
    match expression {
        Expression::Number(value) if !value.contains('.') => value.parse::<i64>().is_ok(),
        Expression::Unary {
            operator, operand, ..
        } if operator == "-" => {
            let Expression::Number(value) = operand.as_ref() else {
                return true;
            };
            if value.contains('.') {
                return true;
            }
            value
                .parse::<u64>()
                .is_ok_and(|number| number <= (i64::MAX as u64) + 1)
        }
        _ => true,
    }
}

fn effective_field_visibility(
    declared: Option<Visibility>,
    containing_visibility: Visibility,
) -> Visibility {
    declared.unwrap_or(match containing_visibility {
        Visibility::Export => Visibility::Export,
        Visibility::Package | Visibility::Private => Visibility::Package,
    })
}

fn function_type(sig: &FunctionSig) -> Type {
    Type::Function {
        params: sig.params.iter().map(|param| param.type_.clone()).collect(),
        return_type: Box::new(sig.return_type.clone()),
        isolated: sig.isolated,
    }
}

fn captured_locals(
    expression: &Expression,
    outer_locals: &HashMap<String, LocalInfo>,
    local_names: &HashSet<String>,
) -> Vec<CapturedLocal> {
    let mut captures = Vec::new();
    let mut seen = HashSet::new();
    collect_captured_locals(
        expression,
        outer_locals,
        local_names,
        &mut seen,
        &mut captures,
    );
    captures
}

fn collect_captured_locals(
    expression: &Expression,
    outer_locals: &HashMap<String, LocalInfo>,
    local_names: &HashSet<String>,
    seen: &mut HashSet<String>,
    captures: &mut Vec<CapturedLocal>,
) {
    match expression {
        Expression::Identifier(name) => {
            if let Some(local) = outer_locals.get(name) {
                if !local_names.contains(name) && seen.insert(name.clone()) {
                    captures.push(CapturedLocal {
                        name: name.clone(),
                        type_: local.type_.clone(),
                        mutable: local.mutable,
                    });
                }
            }
        }
        Expression::Call {
            callee, arguments, ..
        } => {
            if let Some(local) = outer_locals.get(callee) {
                if !local_names.contains(callee) && seen.insert(callee.clone()) {
                    captures.push(CapturedLocal {
                        name: callee.clone(),
                        type_: local.type_.clone(),
                        mutable: local.mutable,
                    });
                }
            }
            for argument in arguments {
                collect_captured_locals(
                    call_arg_value(argument),
                    outer_locals,
                    local_names,
                    seen,
                    captures,
                );
            }
        }
        Expression::Lambda { .. } => {}
        Expression::Binary { left, right, .. } => {
            collect_captured_locals(left, outer_locals, local_names, seen, captures);
            collect_captured_locals(right, outer_locals, local_names, seen, captures);
        }
        Expression::Unary { operand, .. } => {
            collect_captured_locals(operand, outer_locals, local_names, seen, captures);
        }
        Expression::Constructor { arguments, .. } => {
            for argument in arguments {
                collect_captured_locals(
                    constructor_arg_value(argument),
                    outer_locals,
                    local_names,
                    seen,
                    captures,
                );
            }
        }
        Expression::ListLiteral(values) => {
            for value in values {
                collect_captured_locals(value, outer_locals, local_names, seen, captures);
            }
        }
        Expression::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                collect_captured_locals(key, outer_locals, local_names, seen, captures);
                collect_captured_locals(value, outer_locals, local_names, seen, captures);
            }
        }
        Expression::MemberAccess { target, .. } => {
            collect_captured_locals(target, outer_locals, local_names, seen, captures);
        }
        Expression::Trapped { expression, .. } => {
            collect_captured_locals(expression, outer_locals, local_names, seen, captures);
        }
        Expression::WithUpdate { target, updates } => {
            collect_captured_locals(target, outer_locals, local_names, seen, captures);
            for update in updates {
                collect_captured_locals(&update.value, outer_locals, local_names, seen, captures);
            }
        }
        Expression::String(_) | Expression::Number(_) | Expression::Boolean(_) => {}
    }
}

fn constructor_arg_value(argument: &ConstructorArg) -> &Expression {
    match argument {
        ConstructorArg::Positional(value) => value,
        ConstructorArg::Named { value, .. } => value,
    }
}

fn call_arg_value(argument: &CallArg) -> &Expression {
    match argument {
        CallArg::Positional(value) => value,
        CallArg::Named { value, .. } => value,
    }
}

/// Unwrap a `RES`-marked collection element (`Type::Res`) to the underlying
/// type; a no-op for any other type.
fn strip_res(type_: &Type) -> &Type {
    match type_ {
        Type::Res(inner) => inner,
        other => other,
    }
}

/// Whether an expression reads a single element out of a collection (`get` /
/// `getOr`). Of resource type, the result is a borrow that may not be `RES`-bound
/// (§15.6).
fn is_resource_element_borrow(expression: &Expression) -> bool {
    matches!(
        expression,
        Expression::Call { callee, .. }
            if matches!(
                crate::builtins::collections::native_member_bare(callee),
                Some("get" | "getOr")
            )
    )
}

/// Whether `type_name` is a raw C ABI type that may appear only inside an
/// `ABI (...)` slot, never in a wrapper's MFBASIC-facing signature
/// (plan-link-update.md §5/§11). `CPtr` is the resource representation; the
/// others are scalar marshaling types.
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

fn type_kind_name(kind: TypeDeclKind) -> &'static str {
    match kind {
        TypeDeclKind::Type => "TYPE",
        TypeDeclKind::Union => "UNION",
        TypeDeclKind::Enum => "ENUM",
    }
}

fn numeric_literal_type(expression: &Expression) -> Option<Type> {
    match expression {
        Expression::Number(number) if number.contains('.') => Some(Type::Float),
        Expression::Number(_) => Some(Type::Integer),
        Expression::Unary {
            operator, operand, ..
        } if operator == "-" && matches!(operand.as_ref(), Expression::Number(_)) => {
            numeric_literal_type(operand)
        }
        _ => None,
    }
}

fn numeric_literal_is_zero(expression: &Expression) -> bool {
    match expression {
        Expression::Number(value) => value.parse::<f64>().is_ok_and(|number| number == 0.0),
        Expression::Unary {
            operator, operand, ..
        } if operator == "-" && matches!(operand.as_ref(), Expression::Number(_)) => {
            numeric_literal_is_zero(operand)
        }
        _ => false,
    }
}

fn promote_loop_numeric_type(start: &Type, end: &Type, step: &Type) -> Type {
    let Some(start_name) = numeric_type_name(start) else {
        return Type::Unknown;
    };
    let Some(end_name) = numeric_type_name(end) else {
        return Type::Unknown;
    };
    let Some(step_name) = numeric_type_name(step) else {
        return Type::Unknown;
    };
    let first =
        numeric::binary_result_type("+", start_name, end_name).unwrap_or(numeric::TYPE_INTEGER);
    let second =
        numeric::binary_result_type("+", first, step_name).unwrap_or(numeric::TYPE_INTEGER);
    type_from_numeric_name(second)
}

fn type_from_numeric_name(type_name: &str) -> Type {
    match type_name {
        numeric::TYPE_BYTE => Type::Byte,
        numeric::TYPE_INTEGER => Type::Integer,
        numeric::TYPE_FIXED => Type::Fixed,
        numeric::TYPE_FLOAT => Type::Float,
        _ => Type::Unknown,
    }
}

fn numeric_binary_result_type(operator: &str, left: &Type, right: &Type) -> Type {
    let Some(left) = numeric_type_name(left) else {
        return Type::Unknown;
    };
    let Some(right) = numeric_type_name(right) else {
        return Type::Unknown;
    };
    match numeric::binary_result_type(operator, left, right) {
        Some("Byte") => Type::Byte,
        Some("Fixed") => Type::Fixed,
        Some("Float") => Type::Float,
        Some("Integer") => Type::Integer,
        _ => Type::Unknown,
    }
}

fn numeric_type_name(type_: &Type) -> Option<&'static str> {
    match type_ {
        Type::Byte => Some(numeric::TYPE_BYTE),
        Type::Fixed => Some(numeric::TYPE_FIXED),
        Type::Float => Some(numeric::TYPE_FLOAT),
        Type::Integer => Some(numeric::TYPE_INTEGER),
        _ => None,
    }
}

fn read_only_record_type(type_name: &str) -> bool {
    type_name == builtins::term::TERM_COLOR_TYPE
        || type_name == builtins::term::TERM_SIZE_TYPE
        || type_name == builtins::net::ADDRESS_TYPE
        || type_name.starts_with("MapEntry OF ")
}
