use super::*;

impl<'a> SyntaxChecker<'a> {
    pub(super) fn is_resource_type(&self, type_: &Type) -> bool {
        match type_ {
            Type::Named(name) => {
                let name = name.resolve();
                self.resource_registry.is_resource(name) || self.is_resource_union(name)
            }
            // A `RES`-marked element (`RES fs::File`) is a resource (a pointer to one).
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

    /// Whether `value` is an identifier naming a resource `RES` binding or
    /// parameter — the only resource expression that may be stored in a
    /// collection (its slot holds a pointer copied from that binding).
    pub(super) fn collection_element_is_resource_binding(
        &self,
        value: &HirExpression,
        locals: &HashMap<String, LocalInfo>,
    ) -> bool {
        let HirExpression::Identifier(name) = value else {
            return false;
        };
        locals
            .get(name)
            .is_some_and(|info| self.is_resource_type(&info.type_))
    }

    /// The expression mode for a collection element: a resource binding is a
    /// pointer copy (it stays usable after insertion), everything else is consumed.
    pub(super) fn collection_element_mode(
        &self,
        value: &HirExpression,
        locals: &HashMap<String, LocalInfo>,
    ) -> ExprMode {
        if self.collection_element_is_resource_binding(value, locals) {
            ExprMode::Use
        } else {
            ExprMode::Transfer
        }
    }

    pub(super) fn is_copyable_type(&self, type_: &Type) -> bool {
        self.is_copyable_type_with_seen(type_, &mut HashSet::new())
    }

    pub(super) fn is_copyable_type_with_seen(
        &self,
        type_: &Type,
        seen: &mut HashSet<String>,
    ) -> bool {
        match type_ {
            Type::Boolean
            | Type::Byte
            | Type::Fixed
            | Type::Float
            | Type::Integer
            | Type::Money
            | Type::Nothing
            | Type::String
            | Type::Unknown => true,
            // The built-in nominals carry no fields, so they are copyable — the
            // general `User` arm below would answer the same (they are not
            // resources and not in `type_infos`), but saying it here keeps the
            // primitive set readable and independent of that arm's shape.
            Type::Named(name) if is_builtin_nominal(name.resolve()) => true,
            // A collection slot holds a *pointer* to a resource (`RES fs::File`),
            // which copies freely — copying the collection makes more pointers,
            // never another resource. A standalone resource stays non-copyable
            // (the `Type::Named` arm below); §15.6.
            Type::Res(_) => true,
            Type::ListOf(element) => self.is_copyable_type_with_seen(element, seen),
            Type::SetOf(element) => self.is_copyable_type_with_seen(element, seen),
            Type::MapOf(key, value) => {
                self.is_copyable_type_with_seen(key, seen)
                    && self.is_copyable_type_with_seen(value, seen)
            }
            Type::ResultOf(success) => self.is_copyable_type_with_seen(success, seen),
            Type::Func(..) => true,
            Type::ThreadHandle { .. } => false,
            Type::Named(name) => {
                let name = name.resolve();
                if self.resource_registry.is_resource(name) {
                    return false;
                }
                if !seen.insert(name.to_string()) {
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
                        // A resource-union variant is a registered resource (not
                        // copyable) with empty `fields`; the vacuous `.all()` over no
                        // fields would report it copyable (bug-231, bug-173-F pattern).
                        !self.resource_registry.is_resource(&variant.name)
                            && variant
                                .fields
                                .iter()
                                .all(|field| self.is_copyable_type_with_seen(&field.type_, seen))
                    }),
                };
                seen.remove(name);
                result
            }
            // `ParameterType` carries variants syntaxcheck's own parser never
            // produces (`Var`, `Arg`, `UserOf`, `MapEntryOf`, `AttributeString`);
            // a decoded package signature can still hold one. Before plan-106-C
            // rung 2e each arrived spelled out as `Type::User(<spelling>)` and so
            // took the NOMINAL arm above — routing the render back through it
            // reproduces that exactly, rather than guessing a new answer for a
            // shape this checker has never had to answer for.
            other => self.is_copyable_type_with_seen(&Type::named(&other.name()), seen),
        }
    }
}

#[cfg(test)]
mod resources_tests {
    use crate::syntaxcheck::testutil::*;

    // A worker whose thread handle carries a `RES fs::File` resource plane lets us
    // reach thread::transfer / accept / send / receive sendability checks
    // without a multi-file package.
    fn worker_prelude(body: &str) -> String {
        format!(
            "IMPORT thread\nIMPORT fs\nEXPORT ISOLATED FUNC worker(t AS ThreadWorker OF String RES fs::File TO Integer, seed AS String) AS Integer\n{body}\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n"
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
        // thread.accept over the RES fs::File resource plane — sendable resource arm.
        let src = worker_prelude("  RES f AS fs::File = thread::accept(t)\n  fs::close(f)");
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

    /// bug-301 G4: the resource plane's `STATE T` payload crosses the thread
    /// boundary with the resource -- plan-54 deep-copies it into the receiver's
    /// arena -- but only the resource itself was sendability-checked. `ir::verify`
    /// constrains a STATE type to be copyable and defaultable, which does NOT imply
    /// sendable: a record holding `List OF RES fs::File` satisfies both, yet carries
    /// resource pointers to sender-owned resources that §15.6 forbids from crossing.
    // The thread-sendability rejections (TYPE_THREAD_NOT_SENDABLE) moved to
    // `ir::verify` (plan-107-A); their twins are
    // `verify::tests::rejects_unsendable_resource_plane_state_payload`,
    // `rejects_a_non_resource_on_the_resource_plane`,
    // `rejects_a_resource_in_the_message_plane` and
    // `rejects_unsendable_message_sent_across_a_thread`.

    // ---- copyability / sendability walks over user types -------------------

    // ---- resource-union parameter widening (plan-13-A) ---------------------

    /// plan-13-A: a variant value widens into a `RES` parameter that names the
    /// resource union — a `File` into a `RES s AS Stream` parameter. The
    /// widening is reached through `expression_compatible`/`compatible`, which
    /// already subsumes a variant into its union and strips the `RES` marker;
    /// the parameter position now consults it (spec §15.4).
    #[test]
    fn resource_union_variant_widens_into_union_param() {
        let src = "IMPORT fs\nIMPORT net\nUNION Stream\n  fs::File\n  Socket\nEND UNION\nFUNC useStream(RES s AS Stream) AS Integer\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  RES f AS fs::File = fs::createTempFile()\n  RETURN useStream(f)\nEND FUNC\n";
        assert!(
            accepts(src),
            "a variant must widen into a resource-union RES parameter"
        );
    }

    /// The load-bearing direction: a resource union value into a *concrete*
    /// resource parameter is rejected. Symmetric widening would let a union
    /// reach a concrete close op whose real type it cannot know — a
    /// use-after-free class bug, not a type-checker inconvenience.
    #[test]
    fn resource_union_actual_rejected_by_concrete_param() {
        let src = "IMPORT fs\nIMPORT net\nUNION Stream\n  fs::File\n  Socket\nEND UNION\nFUNC useFile(RES f AS fs::File) AS Integer\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  RES s AS Stream = fs::createTempFile()\n  RETURN useFile(s)\nEND FUNC\n";
        assert!(rejects_with(src, "TYPE_CALL_ARGUMENT_MISMATCH"));
    }

    /// A registered close op (`fs::close`, concrete `RES fs::File`) handed a union
    /// is rejected for the same directional reason — the concrete-typed close
    /// op stays unreachable by a whole union.
    #[test]
    fn resource_union_actual_rejected_by_close_op() {
        let src = "IMPORT fs\nIMPORT net\nUNION Stream\n  fs::File\n  Socket\nEND UNION\nFUNC main AS Integer\n  RES s AS Stream = fs::createTempFile()\n  fs::close(s)\n  RETURN 0\nEND FUNC\n";
        assert!(rejects_with(src, "TYPE_CALL_ARGUMENT_MISMATCH"));
    }

    #[test]
    fn resource_union_type_walked() {
        // A union of resource types is a resource union. Storing it in a plain
        // collection exercises contains_resource_or_thread + is_resource_union.
        let src = "IMPORT fs\nUNION Handle\n  fs::File\nEND UNION\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n";
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
        let src = "IMPORT fs\nTYPE Holder\n  f AS List OF RES fs::File\nEND TYPE\nFUNC main AS Integer\n  LET m AS Map OF Holder TO Integer = Map OF Holder TO Integer {}\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn union_with_resource_field_walks() {
        // A union variant with a resource-bearing field walks the Union arm.
        let src = "IMPORT fs\nTYPE A\n  f AS List OF RES fs::File\nEND TYPE\nTYPE B\n  n AS Integer\nEND TYPE\nUNION AB\n  A\n  B\nEND UNION\nFUNC main AS Integer\n  LET m AS Map OF AB TO Integer = Map OF AB TO Integer {}\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    // ---- collection element axis (RES marker mismatch) ---------------------

    #[test]
    fn resource_element_without_res_marker() {
        // The `RES` ownership axis on a collection element is enforced solely by
        // `ir::verify` (plan-20), never by syntaxcheck: a bare `List OF fs::File`
        // (resource element, no `RES`) must pass syntaxcheck silently and be
        // rejected downstream with `TYPE_RESOURCE_REQUIRES_RES`. Guards against
        // reintroducing a syntaxcheck double-rejecter (bug-43). The real
        // rejection is guarded by `ir::verify::tests::
        // rejects_collection_resource_element_without_res` and the
        // `tests/syntax/resources/native-resource-in-list-invalid` fixture.
        let src = "IMPORT fs\nFUNC main AS Integer\n  LET xs AS List OF fs::File = []\n  RETURN 0\nEND FUNC\n";
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
        crate::testutil::fixture_dir(name)
            .to_string_lossy()
            .into_owned()
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
    fn sendable_map_and_result_message_walk() {
        // A worker message of Map/Result-shaped sendable types walks the Map and
        // Result arms of is_thread_sendable_type.
        let src = "IMPORT thread\nEXPORT ISOLATED FUNC worker(t AS ThreadWorker OF (List OF Integer) TO Integer, seed AS List OF Integer) AS Integer\n  LET m AS List OF Integer = thread::receive(t)\n  thread::send(t, m)\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    // ---- non-owning list literal (collection_element_mode Use) --------------

    #[test]
    fn resource_binding_in_list_literal_stores_pointer() {
        // A `List OF RES fs::File` literal `[f]` naming a RES binding stores a pointer
        // (collection_element_mode Use path) and is accepted.
        let src = "IMPORT fs\nFUNC main AS Integer\n  RES f AS fs::File = fs::openFile(\"x\")\n  LET xs AS List OF RES fs::File = [f]\n  RETURN 0\nEND FUNC\n";
        assert!(accepts(src));
    }

    #[test]
    fn resource_list_copyability_and_res_arm() {
        // Copying a `List OF RES fs::File` walks the is_copyable Res arm (a resource
        // a pointer copies freely) and is accepted.
        let src = "IMPORT fs\nFUNC main AS Integer\n  RES f AS fs::File = fs::openFile(\"x\")\n  LET xs AS List OF RES fs::File = [f]\n  LET ys AS List OF RES fs::File = xs\n  RETURN 0\nEND FUNC\n";
        assert!(accepts(src));
    }

    #[test]
    fn non_resource_temporary_in_resource_list_walk() {
        // A non-binding element (a call result) in a resource list is *not* an
        // owner and is rejected — but by `ir::verify` (plan-20), not syntaxcheck,
        // which stays silent here (bug-43). The real rejection
        // (`TYPE_RESOURCE_ELEMENT_NOT_OWNER`) is guarded in `ir::verify::tests`.
        let src = "IMPORT fs\nFUNC main AS Integer\n  LET xs AS List OF RES fs::File = [fs::openFile(\"x\")]\n  RETURN 0\nEND FUNC\n";
        assert!(
            accepts(src),
            "owner-only storage is an ir::verify rule, not syntaxcheck"
        );
    }

    // ---- copyability / sendability recursion arms over nested shapes -------

    #[test]
    fn resource_list_argument_copyability_arm() {
        // Passing a `List OF RES fs::File` as a call argument runs argument_mode_for_type
        // which walks is_copyable_type over List -> Res (a pointer copies freely).
        let src = "IMPORT fs\nFUNC use(xs AS List OF RES fs::File) AS Integer\n  RETURN len(xs)\nEND FUNC\nFUNC main AS Integer\n  RES f AS fs::File = fs::openFile(\"x\")\n  LET xs AS List OF RES fs::File = [f]\n  RETURN use(xs)\nEND FUNC\n";
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
        // A worker whose message type is a `List OF RES fs::File` walks the Res arm of
        // is_thread_sendable_type (a resource collection is not thread-sendable).
        let src = "IMPORT thread\nIMPORT fs\nEXPORT ISOLATED FUNC worker(t AS ThreadWorker OF (List OF RES fs::File) TO Integer, seed AS List OF RES fs::File) AS Integer\n  LET m AS List OF RES fs::File = thread::receive(t)\n  thread::send(t, m)\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n";
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
        // Passing a `List OF RES fs::File` value where argument-mode inspects the
        // element walks is_resource_type over a `Res` wrapper.
        let src = "IMPORT fs\nFUNC useAll(xs AS List OF RES fs::File) AS Integer\n  RETURN len(xs)\nEND FUNC\nFUNC main AS Integer\n  RES f AS fs::File = fs::openFile(\"x\")\n  LET xs AS List OF RES fs::File = [f]\n  RETURN useAll(xs)\nEND FUNC\n";
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
        // A self-referential record carrying a resource pointer walks the seen-set
        // collision return in contains_resource_or_thread over User(Type).
        let src = "IMPORT fs\nTYPE Wrap\n  inner AS List OF Wrap\n  files AS List OF RES fs::File\nEND TYPE\nFUNC main AS Integer\n  LET m AS Map OF Wrap TO Integer = Map OF Wrap TO Integer {}\n  RETURN 0\nEND FUNC\n";
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
        // Appending a resource temporary (a call result) to a `List OF RES fs::File`
        // exercises is_resource_type over the `Res` element wrapper.
        let src = "IMPORT collections\nIMPORT fs\nFUNC main AS Integer\n  MUT xs AS List OF RES fs::File = []\n  xs = collections::append(xs, fs::openFile(\"x\"))\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn map_res_file_value_walk() {
        // A `Map OF String TO RES fs::File` value type carries `Type::Res`
        // and the RES-marked value axis check.
        let src = "IMPORT fs\nFUNC main AS Integer\n  MUT m AS Map OF String TO RES fs::File = Map OF String TO RES fs::File {}\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }
}
