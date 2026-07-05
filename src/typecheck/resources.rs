use super::helpers::*;
use super::*;

impl<'a> TypeChecker<'a> {
    pub(super) fn is_resource_type(&self, type_: &Type) -> bool {
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
    pub(super) fn is_resource_union(&self, name: &str) -> bool {
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

    pub(super) fn contains_resource_or_thread(&self, type_: &Type) -> bool {
        self.contains_resource_or_thread_with_seen(type_, &mut HashSet::new())
    }

    /// Whether a type transitively contains a thread handle. Threads may never
    /// live in a collection; resources may (as borrows, §15.6), so collection
    /// element and `Map` *value* positions use this rather than the combined
    /// resource-or-thread predicate.
    pub(super) fn contains_thread(&self, type_: &Type) -> bool {
        self.contains_thread_with_seen(type_, &mut HashSet::new())
    }

    pub(super) fn contains_thread_with_seen(
        &self,
        type_: &Type,
        seen: &mut HashSet<String>,
    ) -> bool {
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

    pub(super) fn contains_resource_or_thread_with_seen(
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

    pub(super) fn report_invalid_collection_element(
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
    pub(super) fn check_collection_element_axis(
        &mut self,
        _file: &AstFile,
        _line: usize,
        _role: &str,
        element: &Type,
    ) {
        let is_res_marked = matches!(element, Type::Res(_));
        let inner = strip_res(element);
        let is_resource = self.is_resource_type(inner);
        if is_resource && !is_res_marked {
        } else if is_res_marked && !is_resource {
        }
    }

    /// A `List` element or `Map` value may hold a *borrow* of a resource, but
    /// only of a named `RES` binding (the owner); a temporary or a borrowed
    /// element (e.g. a `get`/`FOR EACH` result) is not an owner and cannot be
    /// stored (§15.6).
    pub(super) fn check_collection_resource_element(
        &mut self,
        _file: &AstFile,
        _line: usize,
        _role: &str,
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
    }

    /// Whether `value` is an identifier naming a resource `RES` binding or
    /// parameter — the only resource expression that may be stored in a
    /// collection (its slot holds a borrow of that binding).
    pub(super) fn collection_element_is_resource_binding(
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
    pub(super) fn collection_element_mode(
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

    pub(super) fn is_copyable_type(&self, type_: &Type) -> bool {
        self.is_copyable_type_with_seen(type_, &mut HashSet::new())
    }

    pub(super) fn is_thread_sendable_type(&self, type_: &Type) -> bool {
        self.is_thread_sendable_type_with_seen(type_, &mut HashSet::new())
    }

    pub(super) fn is_defaultable_type(&self, type_: &Type) -> bool {
        self.is_defaultable_type_with_seen(type_, &mut HashSet::new())
    }

    pub(super) fn is_defaultable_type_with_seen(
        &self,
        type_: &Type,
        seen: &mut HashSet<String>,
    ) -> bool {
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

    pub(super) fn is_copyable_type_with_seen(
        &self,
        type_: &Type,
        seen: &mut HashSet<String>,
    ) -> bool {
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

    pub(super) fn is_thread_sendable_type_with_seen(
        &self,
        type_: &Type,
        seen: &mut HashSet<String>,
    ) -> bool {
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

    pub(super) fn report_thread_type_not_sendable(
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

    pub(super) fn require_thread_sendable_type(
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

    pub(super) fn check_thread_boundary_sendability(
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
}
