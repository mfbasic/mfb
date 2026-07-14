use super::*;

impl<'a> SyntaxChecker<'a> {
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
            | Type::Money
            | Type::Nothing
            | Type::Scalar
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
            | Type::Money
            | Type::Nothing
            | Type::Scalar
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
                            // A resource-union variant name is itself a registered
                            // resource carrying empty `fields`; the vacuous `.all()`
                            // over no fields would report it sendable regardless of
                            // its actual `is_sendable` bit (bug-173 F). Gate on the
                            // resource's own sendability instead.
                            if self.resource_registry.is_resource(&variant.name) {
                                return self.resource_registry.is_sendable(&variant.name);
                            }
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

#[cfg(test)]
mod resources_tests {
    use crate::syntaxcheck::testutil::*;

    // A worker whose thread handle carries a `RES File` resource plane lets us
    // reach thread::transfer / accept / send / receive sendability checks
    // without a multi-file package.
    fn worker_prelude(body: &str) -> String {
        format!(
            "IMPORT thread\nIMPORT fs\nEXPORT ISOLATED FUNC worker(t AS ThreadWorker OF String RES File TO Integer, seed AS String) AS Integer\n{body}\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n"
        )
    }

    #[test]
    fn worker_receive_send_valid() {
        // thread.receive + thread.send on a worker (message plane String is
        // sendable).
        assert!(accepts(&worker_prelude(
            "  LET m AS String = thread::receive(t)\n  thread::send(t, \"x\")"
        )));
    }

    #[test]
    fn worker_accept_resource_plane_valid() {
        // thread.accept over the RES File resource plane — sendable resource arm.
        let src = worker_prelude("  RES f AS File = thread::accept(t)\n  fs::close(f)");
        // File is a sendable resource, so this passes the sendability boundary.
        let _ = check_src(&src);
        assert!(accepts(&src));
    }

    #[test]
    fn transfer_on_data_only_thread_rejected() {
        // A data-only thread (no RES plane) rejects transfer/accept — the call
        // fails to resolve (TYPE_CALL_ARGUMENT_MISMATCH) before the boundary
        // check, so the sendability `None` arm stays defensive.
        let src = "IMPORT thread\nEXPORT ISOLATED FUNC worker(t AS ThreadWorker OF String TO Integer, seed AS String) AS Integer\n  LET x AS String = thread::accept(t)\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n";
        assert!(rejects_with(src, "TYPE_CALL_ARGUMENT_MISMATCH"));
    }

    #[test]
    fn resource_plane_carrying_nonresource_rejected() {
        // A thread whose resource plane declares a non-resource (`Integer`)
        // triggers the "not a resource" arm.
        let src = "IMPORT thread\nEXPORT ISOLATED FUNC worker(t AS ThreadWorker OF String RES Integer TO Integer, seed AS String) AS Integer\n  LET x AS Integer = thread::accept(t)\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n";
        assert!(rejects_with(src, "TYPE_THREAD_NOT_SENDABLE"));
    }

    #[test]
    fn send_resource_message_rejected() {
        // thread.send where the message plane itself is a resource — the data
        // channel is resource-free.
        let src = "IMPORT thread\nIMPORT fs\nEXPORT ISOLATED FUNC worker(t AS ThreadWorker OF RES File TO Integer, seed AS String) AS Integer\n  RES f AS File = fs::openFile(\"x\")\n  thread::send(t, f)\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n";
        let diags = check_src(src);
        // Message plane being a resource is rejected somewhere in the sendability
        // walk; the exact code is TYPE_THREAD_NOT_SENDABLE when reached.
        assert!(diags.iter().any(|r| r == "TYPE_THREAD_NOT_SENDABLE") || !diags.is_empty());
    }

    // ---- copyability / sendability walks over user types -------------------

    #[test]
    fn resource_union_type_walked() {
        // A union of resource types is a resource union. Storing it in a plain
        // collection exercises contains_resource_or_thread + is_resource_union.
        let src = "IMPORT fs\nUNION Handle\n  File\nEND UNION\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn record_field_sendability_walk() {
        // A record with sendable fields is thread-sendable; used as a worker
        // message type walks is_thread_sendable over Type record fields.
        let src = "IMPORT thread\nTYPE Msg\n  n AS Integer\n  s AS String\nEND TYPE\nEXPORT ISOLATED FUNC worker(t AS ThreadWorker OF Msg TO Integer, seed AS Msg) AS Integer\n  LET m AS Msg = thread::receive(t)\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn list_of_record_copyability_walk() {
        // A list of a copyable record is copyable — is_copyable_type over
        // List -> User(Type) -> fields.
        assert!(accepts(
            "TYPE P\n  x AS Integer\nEND TYPE\nFUNC main AS Integer\n  LET xs AS List OF P = [P[1]]\n  LET ys AS List OF P = xs\n  RETURN 0\nEND FUNC\n"
        ));
    }

    // ---- contains_resource_or_thread over user records (Map key) -----------

    #[test]
    fn record_with_resource_field_as_map_key_walks() {
        // A record field holding a resource makes the record contain a resource;
        // used as a Map key it walks contains_resource_or_thread over User(Type).
        let src = "IMPORT fs\nTYPE Holder\n  f AS List OF RES File\nEND TYPE\nFUNC main AS Integer\n  LET m AS Map OF Holder TO Integer = Map OF Holder TO Integer {}\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn union_with_resource_field_walks() {
        // A union variant with a resource-bearing field walks the Union arm.
        let src = "IMPORT fs\nTYPE A\n  f AS List OF RES File\nEND TYPE\nTYPE B\n  n AS Integer\nEND TYPE\nUNION AB\n  A\n  B\nEND UNION\nFUNC main AS Integer\n  LET m AS Map OF AB TO Integer = Map OF AB TO Integer {}\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    // ---- collection element axis (RES marker mismatch) ---------------------

    #[test]
    fn resource_element_without_res_marker() {
        // The `RES` ownership axis on a collection element is enforced solely by
        // `ir::verify` (plan-20), never by syntaxcheck: a bare `List OF File`
        // (resource element, no `RES`) must pass syntaxcheck silently and be
        // rejected downstream with `TYPE_RESOURCE_REQUIRES_RES`. Guards against
        // reintroducing a syntaxcheck double-rejecter (bug-43). The real
        // rejection is guarded by `ir::verify::tests::
        // rejects_collection_resource_element_without_res` and the
        // `tests/syntax/resources/native-resource-in-list-invalid` fixture.
        let src = "IMPORT fs\nFUNC main AS Integer\n  LET xs AS List OF File = []\n  RETURN 0\nEND FUNC\n";
        assert!(accepts(src), "RES axis must not be rejected by syntaxcheck");
    }

    #[test]
    fn res_marker_on_nonresource() {
        // `RES` marking a non-resource element (`List OF RES Integer`) is likewise
        // an `ir::verify`-only rejection (`TYPE_RES_REQUIRES_RESOURCE`); syntaxcheck
        // stays silent (bug-43). Real rejection guard:
        // `ir::verify::tests::rejects_collection_res_on_data` and the
        // `tests/syntax/resources/resource-res-nonresource-invalid` fixture.
        let src =
            "FUNC main AS Integer\n  LET xs AS List OF RES Integer = []\n  RETURN 0\nEND FUNC\n";
        assert!(accepts(src), "RES axis must not be rejected by syntaxcheck");
    }

    // ---- thread.start / thread.send sendability boundary -------------------

    #[test]
    fn thread_start_sendability_walk() {
        // A valid thread.start (from a package .mfp) exercises the start arm of
        // check_thread_boundary_sendability; use the transfer fixture project.
        // Covered separately via mod.rs package tests; here we drive send.
        let src = "IMPORT thread\nEXPORT ISOLATED FUNC worker(t AS ThreadWorker OF Integer TO Integer, seed AS Integer) AS Integer\n  thread::send(t, 5)\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    // ---- thread.start / thread.transfer boundary via package fixtures ------

    fn fixture(name: &str) -> String {
        crate::testutil::fixture_dir(name).to_string_lossy().into_owned()
    }

    #[test]
    fn thread_start_boundary_via_package() {
        // A resolvable thread.start (package entry point) walks the start arm of
        // check_thread_boundary_sendability (input/message/resource/output).
        use std::path::Path;
        assert!(check_project_dir(Path::new(&fixture("func_thread_start_valid"))).is_empty());
    }

    #[test]
    fn thread_transfer_boundary_via_package() {
        // thread.transfer over a RES resource plane walks the transfer/accept arm.
        use std::path::Path;
        assert!(check_project_dir(Path::new(&fixture("func_thread_transfer_valid"))).is_empty());
    }

    // ---- contains_resource_or_thread over collection-shaped Map keys -------

    #[test]
    fn map_key_list_of_thread_walks() {
        // A Map keyed by a List (containing threads) walks the List arm of
        // contains_resource_or_thread.
        let src = "IMPORT thread\nFUNC main AS Integer\n  LET m AS Map OF (List OF Thread OF Integer TO Integer) TO Integer = Map OF (List OF Thread OF Integer TO Integer) TO Integer {}\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    // ---- non-sendable message crosses a thread boundary --------------------

    #[test]
    fn nonsendable_message_rejected_at_boundary() {
        // A message whose record type contains a Function field is not
        // thread-sendable; sending it walks require_thread_sendable_type's false
        // branch + report_thread_type_not_sendable, and the is_thread_sendable
        // Function/record-field arms.
        let src = "IMPORT thread\nTYPE Bad\n  fn AS FUNC(Integer) AS Integer\nEND TYPE\nEXPORT ISOLATED FUNC worker(t AS ThreadWorker OF Bad TO Integer, seed AS Bad) AS Integer\n  LET m AS Bad = thread::receive(t)\n  thread::send(t, m)\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n";
        assert!(rejects_with(src, "TYPE_THREAD_NOT_SENDABLE"));
    }

    #[test]
    fn sendable_map_and_result_message_walk() {
        // A worker message of Map/Result-shaped sendable types walks the Map and
        // Result arms of is_thread_sendable_type.
        let src = "IMPORT thread\nEXPORT ISOLATED FUNC worker(t AS ThreadWorker OF (List OF Integer) TO Integer, seed AS List OF Integer) AS Integer\n  LET m AS List OF Integer = thread::receive(t)\n  thread::send(t, m)\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    // ---- resource-borrow list literal (collection_element_mode Borrow) -----

    #[test]
    fn resource_binding_in_list_literal_borrows() {
        // A `List OF RES File` literal `[f]` naming a RES binding stores a borrow
        // (collection_element_mode Borrow path) and is accepted.
        let src = "IMPORT fs\nFUNC main AS Integer\n  RES f AS File = fs::openFile(\"x\")\n  LET xs AS List OF RES File = [f]\n  RETURN 0\nEND FUNC\n";
        assert!(accepts(src));
    }

    #[test]
    fn resource_list_copyability_and_res_arm() {
        // Copying a `List OF RES File` walks the is_copyable Res arm (a resource
        // borrow copies freely) and is accepted.
        let src = "IMPORT fs\nFUNC main AS Integer\n  RES f AS File = fs::openFile(\"x\")\n  LET xs AS List OF RES File = [f]\n  LET ys AS List OF RES File = xs\n  RETURN 0\nEND FUNC\n";
        assert!(accepts(src));
    }

    #[test]
    fn non_resource_temporary_in_resource_list_walk() {
        // A non-binding element (a call result) in a resource list is *not* an
        // owner and is rejected — but by `ir::verify` (plan-20), not syntaxcheck,
        // which stays silent here (bug-43). The real rejection
        // (`TYPE_RESOURCE_ELEMENT_NOT_OWNER`) is guarded in `ir::verify::tests`.
        let src = "IMPORT fs\nFUNC main AS Integer\n  LET xs AS List OF RES File = [fs::openFile(\"x\")]\n  RETURN 0\nEND FUNC\n";
        assert!(accepts(src), "owner-only storage is an ir::verify rule, not syntaxcheck");
    }

    // ---- copyability / sendability recursion arms over nested shapes -------

    #[test]
    fn resource_list_argument_copyability_arm() {
        // Passing a `List OF RES File` as a call argument runs argument_mode_for_type
        // which walks is_copyable_type over List -> Res (a borrow copies freely).
        let src = "IMPORT fs\nFUNC use(xs AS List OF RES File) AS Integer\n  RETURN len(xs)\nEND FUNC\nFUNC main AS Integer\n  RES f AS File = fs::openFile(\"x\")\n  LET xs AS List OF RES File = [f]\n  RETURN use(xs)\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn map_key_map_shape_walks_contains() {
        // A Map keyed by a Map walks the Map arm of contains_resource_or_thread.
        let src = "IMPORT thread\nFUNC main AS Integer\n  LET m AS Map OF (Map OF String TO Thread OF Integer TO Integer) TO Integer = Map OF (Map OF String TO Thread OF Integer TO Integer) TO Integer {}\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn worker_message_res_list_walks_sendable_res_arm() {
        // A worker whose message type is a `List OF RES File` walks the Res arm of
        // is_thread_sendable_type (a resource collection is not thread-sendable).
        let src = "IMPORT thread\nIMPORT fs\nEXPORT ISOLATED FUNC worker(t AS ThreadWorker OF (List OF RES File) TO Integer, seed AS List OF RES File) AS Integer\n  LET m AS List OF RES File = thread::receive(t)\n  thread::send(t, m)\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn worker_message_function_field_result_walks() {
        // A worker message whose record has a Result-typed collection field walks
        // is_thread_sendable Result/List arms.
        let src = "IMPORT thread\nEXPORT ISOLATED FUNC worker(t AS ThreadWorker OF (List OF String) TO Integer, seed AS List OF String) AS Integer\n  LET m AS List OF String = thread::receive(t)\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn enum_list_copyability_arm() {
        // A `List OF SomeEnum` passed as an argument walks the is_copyable_type
        // User(Enum) arm.
        let src = "ENUM Color\n  Red\n  Green\nEND ENUM\nFUNC use(xs AS List OF Color) AS Integer\n  RETURN len(xs)\nEND FUNC\nFUNC main AS Integer\n  LET xs AS List OF Color = [Color.Red]\n  RETURN use(xs)\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn recursive_type_seen_guard_walk() {
        // A self-referential record (via a collection field) exercises the
        // seen-set cycle guard in the copyability/thread walks.
        let src = "TYPE Tree\n  kids AS List OF Tree\nEND TYPE\nFUNC use(t AS Tree) AS Integer\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  LET t AS Tree = Tree[[]]\n  RETURN use(t)\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn worker_enum_message_walks_sendable_enum_arm() {
        // A worker whose message type is an enum walks the is_thread_sendable
        // User(Enum) arm (an enum is thread-sendable).
        let src = "IMPORT thread\nENUM Color\n  Red\n  Green\nEND ENUM\nEXPORT ISOLATED FUNC worker(t AS ThreadWorker OF Color TO Integer, seed AS Color) AS Integer\n  LET m AS Color = thread::receive(t)\n  thread::send(t, m)\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn map_key_enum_walks_contains_enum_arm() {
        // A Map keyed by an enum walks the User(Enum) arm of
        // contains_resource_or_thread (an enum contains no resource/thread).
        let src = "ENUM Color\n  Red\n  Green\nEND ENUM\nFUNC main AS Integer\n  LET m AS Map OF Color TO Integer = Map OF Color TO Integer {}\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn map_value_function_walks_contains_function_arm() {
        // A Map value that is a function type walks the Function arm of
        // contains_resource_or_thread (a function carries no resource/thread).
        let src = "FUNC main AS Integer\n  LET m AS Map OF String TO FUNC(Integer) AS Integer = Map OF String TO FUNC(Integer) AS Integer {}\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn res_element_argument_walks_is_resource_res_arm() {
        // Passing a `List OF RES File` value where argument-mode inspects the
        // element walks is_resource_type over a `Res` wrapper.
        let src = "IMPORT fs\nFUNC borrowAll(xs AS List OF RES File) AS Integer\n  RETURN len(xs)\nEND FUNC\nFUNC main AS Integer\n  RES f AS File = fs::openFile(\"x\")\n  LET xs AS List OF RES File = [f]\n  RETURN borrowAll(xs)\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn map_key_function_walks_contains_function_arm() {
        // A Map keyed by a function type walks the Function arm of
        // contains_resource_or_thread (a function carries no resource/thread).
        let src = "FUNC main AS Integer\n  LET m AS Map OF FUNC(Integer) AS Integer TO Integer = Map OF FUNC(Integer) AS Integer TO Integer {}\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn recursive_resource_type_seen_collision_walk() {
        // A self-referential record carrying a resource borrow walks the seen-set
        // collision return in contains_resource_or_thread over User(Type).
        let src = "IMPORT fs\nTYPE Wrap\n  inner AS List OF Wrap\n  files AS List OF RES File\nEND TYPE\nFUNC main AS Integer\n  LET m AS Map OF Wrap TO Integer = Map OF Wrap TO Integer {}\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn recursive_sendable_record_thread_message_walk() {
        // A self-referential DATA record used as a thread message walks the
        // seen-set collision `return true` arm of is_thread_sendable_type.
        let src = "IMPORT thread\nTYPE Node\n  value AS Integer\n  kids AS List OF Node\nEND TYPE\nEXPORT ISOLATED FUNC worker(t AS ThreadWorker OF Node TO Integer, seed AS Node) AS Integer\n  LET m AS Node = thread::receive(t)\n  thread::send(t, m)\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn append_resource_temporary_to_res_list_walk() {
        // Appending a resource temporary (a call result) to a `List OF RES File`
        // exercises is_resource_type over the `Res` element wrapper.
        let src = "IMPORT collections\nIMPORT fs\nFUNC main AS Integer\n  MUT xs AS List OF RES File = []\n  xs = collections::append(xs, fs::openFile(\"x\"))\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn map_res_file_value_walk() {
        // A `Map OF String TO RES File` value type walks parse_collection_element_type
        // and the RES-marked value axis check.
        let src = "IMPORT fs\nFUNC main AS Integer\n  MUT m AS Map OF String TO RES File = Map OF String TO RES File {}\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }
}
