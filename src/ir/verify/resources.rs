use super::*;

impl TypeEnv {
    // 7. Resource moves, defaultability, collection RES axis
    // ===========================================================================

    /// Reject a read of a resource binding after it was moved (closed, returned)
    /// — `syntaxcheck`'s `TYPE_USE_AFTER_MOVE`. On decoded package IR a
    /// use-after-move is a use-after-free / double-free: the resource's backing
    /// handle is released by the move, so a later read hands codegen a dangling
    /// handle. Conservative straight-line dataflow: a move is only tracked
    /// within a linear op sequence (nested blocks get a fresh copy that does not
    /// leak moves back out), so no valid program is ever rejected; it catches
    /// the common close-then-use and double-close. Consumption = a call to the
    /// resource type's registered close op with the binding as its first
    /// argument, or `RETURN <resource>`.
    pub(super) fn check_resource_moves(
        &self,
        ops: &[IrOp],
        locals: &mut HashMap<String, ParameterType>,
        moved: &mut HashSet<String>,
        owners: &HashMap<String, crate::ir::resource_escape::ResOwner>,
        non_owning: &HashSet<String>,
        aliases: &mut HashMap<String, HashSet<String>>,
    ) {
        /// plan-59-E: every binding that may denote the same resource as `name`,
        /// transitively. `moved` is keyed by binding NAME, so once two names can
        /// denote one resource, closing through one must mark the others or the
        /// rule reports a false negative — it would stay silent on a genuine
        /// use-after-close.
        fn alias_closure(
            name: &str,
            aliases: &HashMap<String, HashSet<String>>,
        ) -> HashSet<String> {
            let mut seen: HashSet<String> = HashSet::new();
            let mut stack = vec![name.to_string()];
            while let Some(current) = stack.pop() {
                if !seen.insert(current.clone()) {
                    continue;
                }
                if let Some(next) = aliases.get(&current) {
                    stack.extend(next.iter().cloned());
                }
            }
            seen.remove(name);
            seen
        }
        // A branch that always leaves the function never reaches the join, so
        // its moves must not leak past it (syntaxcheck merges only fall-through
        // branches). Top-level test is enough: a mid-block Return makes the
        // rest unreachable anyway.
        fn diverges(ops: &[IrOp]) -> bool {
            ops.iter().any(|op| {
                matches!(
                    op,
                    IrOp::Return { .. } | IrOp::Fail { .. } | IrOp::ExitProgram { .. }
                )
            })
        }
        // Run `body` as a branch: fresh scope, then merge the new moves of a
        // fall-through branch back into the outer set (syntaxcheck's MaybeMoved —
        // moved on *some* path means unusable after the join).
        let run_branch = |body: &[IrOp],
                          locals: &HashMap<String, ParameterType>,
                          moved: &mut HashSet<String>,
                          aliases: &mut HashMap<String, HashSet<String>>| {
            let mut branch_moved = moved.clone();
            // Aliases discovered inside a branch merge back the same way moves do:
            // "may alias on *some* fall-through path" is still may-alias after the
            // join, and treating it otherwise would lose the relation exactly where
            // it is needed.
            let mut branch_aliases = aliases.clone();
            self.check_resource_moves(
                body,
                &mut locals.clone(),
                &mut branch_moved,
                owners,
                non_owning,
                &mut branch_aliases,
            );
            if !diverges(body) {
                for name in branch_moved {
                    // Only propagate moves of bindings the outer scope knows;
                    // branch-local resources die with the branch.
                    if locals.contains_key(&name) {
                        moved.insert(name);
                    }
                }
                for (name, targets) in branch_aliases {
                    if locals.contains_key(&name) {
                        let kept: HashSet<String> = targets
                            .into_iter()
                            .filter(|t| locals.contains_key(t))
                            .collect();
                        if !kept.is_empty() {
                            aliases.entry(name).or_default().extend(kept);
                        }
                    }
                }
            }
        };
        for op in ops {
            self.current_line.set(op.loc().line);
            // A read of an already-moved binding is a use-after-move. The
            // consuming op reads the binding too, but at that point it is not
            // yet in `moved` (we insert below), so the consume itself is fine
            // and a *second* consume (double close) is correctly flagged.
            let mut reads = Vec::new();
            collect_local_reads_op(op, &mut reads);
            for name in &reads {
                if moved.contains(name) {
                    self.emit(
                        "TYPE_USE_AFTER_MOVE",
                        format!("Binding `{name}` was moved and cannot be used again."),
                    );
                }
            }
            if let Some(consumed) = self.consumed_resource(op, locals) {
                // plan-59-E: a non-owning pointer (a `RES` parameter, a `FOR EACH`
                // element) used to be forbidden from closing/returning/transferring
                // here (`TYPE_RESOURCE_INVALIDATE_NOT_OWNER`, retired). That rule
                // is what made `closeSound(RES sound AS SoundFile)` — "take a
                // handle, give it back" — unwritable in any form.
                //
                // Under scope ownership ANY holder of the pointer may close it, and
                // the outermost scope that touches it closes it once if nobody
                // already did. `non_owning` is therefore no longer consulted to
                // reject; the consume is tracked for every binding alike, which is
                // what keeps `TYPE_USE_AFTER_MOVE` honest afterwards.
                //
                // Closing/returning/transferring through ONE name consumes the
                // resource, so every name that MAY denote it is consumed too.
                // Without this the rule stays silent on a real use-after-close
                // reached through an alias — a false negative, and the invisible
                // failure mode this sub-plan guards against (Phase 2).
                for alias in alias_closure(&consumed, aliases) {
                    moved.insert(alias);
                }
                moved.insert(consumed);
            }
            match op {
                IrOp::Bind {
                    name, type_, value, ..
                } => {
                    // `RES new = old` transfers ownership: the source binding is
                    // moved. Only a RES-declared bind (an entry in the
                    // function's resource-owner table) moves; a plain LET of a
                    // resource local does not move ownership.
                    if owners.contains_key(name) {
                        if let Some(IrValue::Local(source)) = value {
                            if locals.get(source).is_some_and(|t| {
                                self.close_op_for(&resource_base_type(t).name()).is_some()
                            }) {
                                moved.insert(source.clone());
                            }
                        }
                    }
                    // A rebind of a resource name reopens ownership.
                    //
                    // ORDER MATTERS and getting it wrong is silent: this severs
                    // whatever the PREVIOUS binding of this name aliased, so it
                    // must run BEFORE the new alias is recorded below. Recording
                    // first and severing after deletes the relation on the very
                    // statement that establishes it -- the map ends up empty and
                    // the tracking is inert while still looking correct.
                    if value.is_some() {
                        moved.remove(name);
                        aliases.remove(name);
                        for targets in aliases.values_mut() {
                            targets.remove(name);
                        }
                    }
                    // plan-59-E: `RES g = f(h, …)` where `f` returns a resource may
                    // hand back the very resource `h` denotes — "take a handle,
                    // give it back" is the shape this whole plan exists to make
                    // writable, and the signature `AS RES File` does not encode
                    // identity (`res.md` §3.3). So `g` and `h` MAY alias, and the
                    // relation is recorded rather than proved.
                    //
                    // **No diagnostic is emitted here** (DECIDED, Open Decisions):
                    // a warning at every call returning `RES` would fire on correct
                    // code and train people to ignore it. The state exists only so
                    // a later close through either name marks both.
                    //
                    // Restricted to arguments of the SAME resource type as the
                    // return. A `Stmt` produced from a `Db` cannot BE that `Db`, so
                    // relating them would reject `prepare(db); finalize(s);
                    // exec(db)` — correct code every sqlite binding writes, and the
                    // in-tree fixtures do.
                    //
                    // Within one type it still over-approximates (a callee
                    // returning a *fresh* resource is recorded as a possible alias
                    // too). That is the safe direction: a missed alias is a silent
                    // use-after-close, an extra one is a visible false positive.
                    if owners.contains_key(name) {
                        if let Some(IrValue::Call { args, type_, .. })
                        | Some(IrValue::CallResult { args, type_, .. }) = value
                        {
                            let returned = resource_base_type(type_);
                            if self.close_op_for(&returned.name()).is_some() {
                                for arg in args {
                                    if let IrValue::Local(source) = arg {
                                        if locals
                                            .get(source)
                                            .is_some_and(|t| resource_base_type(t) == returned)
                                        {
                                            aliases
                                                .entry(name.clone())
                                                .or_default()
                                                .insert(source.clone());
                                            aliases
                                                .entry(source.clone())
                                                .or_default()
                                                .insert(name.clone());
                                        }
                                    }
                                }
                            }
                        }
                    }
                    locals.insert(name.clone(), type_.clone());
                }
                IrOp::If {
                    then_body,
                    else_body,
                    ..
                } => {
                    run_branch(then_body, locals, moved, aliases);
                    run_branch(else_body, locals, moved, aliases);
                }
                IrOp::Match { cases, .. } => {
                    for case in cases {
                        run_branch(&case.body, locals, moved, aliases);
                    }
                }
                IrOp::ForEach {
                    name, type_, body, ..
                } => {
                    // The element binding is a non-owning pointer copied from the collection's slot.
                    let mut fe_locals = locals.clone();
                    fe_locals.insert(name.clone(), type_.clone());
                    let mut fe_non_owning = non_owning.clone();
                    fe_non_owning.insert(name.clone());
                    let mut branch_moved = moved.clone();
                    let mut fe_aliases = aliases.clone();
                    self.check_resource_moves(
                        body,
                        &mut fe_locals,
                        &mut branch_moved,
                        owners,
                        &fe_non_owning,
                        &mut fe_aliases,
                    );
                    for n in branch_moved {
                        if locals.contains_key(&n) {
                            moved.insert(n);
                        }
                    }
                }
                IrOp::While { body, .. }
                | IrOp::For { body, .. }
                | IrOp::DoUntil { body, .. }
                | IrOp::Trap { body, .. } => {
                    run_branch(body, locals, moved, aliases);
                }
                _ => {}
            }
        }
    }

    /// Whether the just-checked value's type is undeterminable the way
    /// syntaxcheck's inference would see it: either a poisoning rule fired and
    /// the value's own result rides on the failed node (a Binary/Unary chain,
    /// where lowering stamps a nominal type the failure invalidates), or the
    /// type simply cannot be reconstructed *and* something was reported. The
    /// caller must reset `self.poisoned` before checking the value.
    pub(super) fn value_type_poisoned(
        &self,
        value: &IrValue,
        locals: &HashMap<String, ParameterType>,
    ) -> bool {
        if !self.poisoned.get() {
            return false;
        }
        matches!(
            value,
            IrValue::Binary { .. }
                | IrValue::Unary { .. }
                | IrValue::Constructor { .. }
                | IrValue::WithUpdate { .. }
        ) || self.infer_type(value, locals).is_none()
    }

    // ===========================================================================
    // Thread sendability + the inline-TRAP scrutinee (relocated by plan-107-A)
    // ===========================================================================

    /// Whether a resource type may cross a thread boundary: the project's own
    /// `RESOURCE … THREAD_SENDABLE` opt-in or an imported package's
    /// `RESOURCE_TABLE` bit (`resource_sendable`), else the built-in registry.
    fn is_resource_sendable(&self, base: &str) -> bool {
        self.resource_sendable
            .get(base)
            .copied()
            .unwrap_or_else(|| crate::codegen::resource::is_builtin_sendable_resource_type(base))
    }

    /// A resource type: a registered resource, a `RES`-marked element, or a
    /// resource union (every variant a resource) — syntaxcheck's `is_resource_type`.
    fn is_resource_type(&self, type_: &ParameterType) -> bool {
        match type_ {
            ParameterType::Res(inner) => self.is_resource_type(inner),
            other => {
                let name = other.name();
                self.close_op_for(&name).is_some()
                    || self.unions.get(name.as_ref()).is_some_and(|union| {
                        !union.variant_order.is_empty()
                            && union
                                .variant_order
                                .iter()
                                .all(|variant| self.close_op_for(variant).is_some())
                    })
            }
        }
    }

    /// Whether a value of `type_` may cross a thread boundary (syntaxcheck's
    /// `is_thread_sendable_type`): primitives and the built-in nominals yes;
    /// collections and `Result` by their elements; a `RES`-marked element, a
    /// FUNC and a thread handle never; a resource by its declared sendability; a
    /// record by every field, a union by every variant (a resource variant by
    /// its own bit, bug-173 F); enums yes. A name the tables do not know is
    /// treated as sendable — only a positively known type may reject.
    pub(super) fn is_thread_sendable(
        &self,
        type_: &ParameterType,
        seen: &mut HashSet<String>,
    ) -> bool {
        match type_ {
            ParameterType::Boolean
            | ParameterType::Byte
            | ParameterType::Fixed
            | ParameterType::Float
            | ParameterType::Integer
            | ParameterType::Money
            | ParameterType::Nothing
            | ParameterType::String
            | ParameterType::Unknown => true,
            ParameterType::ListOf(element) | ParameterType::SetOf(element) => {
                self.is_thread_sendable(element, seen)
            }
            ParameterType::MapOf(key, value) => {
                self.is_thread_sendable(key, seen) && self.is_thread_sendable(value, seen)
            }
            ParameterType::ResultOf(success) => self.is_thread_sendable(success, seen),
            // Sharing a resource collection across threads is out of scope (§15.6).
            ParameterType::Res(_) => false,
            ParameterType::Func(..) | ParameterType::ThreadHandle { .. } => false,
            other => {
                let name = other.name();
                let name = name.as_ref();
                // The built-in nominals are plain values.
                if matches!(name, "AttributedString" | "Error" | "ErrorLoc" | "Scalar") {
                    return true;
                }
                if self.close_op_for(name).is_some() {
                    return self.is_resource_sendable(name);
                }
                if !seen.insert(name.to_string()) {
                    return true;
                }
                let result = match self.unions.get(name) {
                    Some(union) => union.variant_order.iter().all(|variant| {
                        if self.close_op_for(variant).is_some() {
                            return self.is_resource_sendable(variant);
                        }
                        self.record_fields_sendable(variant, seen)
                    }),
                    None => self.record_fields_sendable(name, seen),
                };
                seen.remove(name);
                result
            }
        }
    }

    /// Every field of record `name` is sendable; a name that is not a record
    /// (an enum, or a type the tables do not know) is vacuously sendable.
    fn record_fields_sendable(&self, name: &str, seen: &mut HashSet<String>) -> bool {
        self.record_field_lists.get(name).is_none_or(|fields| {
            fields
                .iter()
                .all(|(_, field_type)| self.is_thread_sendable(field_type, seen))
        })
    }

    fn require_thread_sendable(&self, context: &str, type_: &ParameterType) {
        if !self.is_thread_sendable(type_, &mut HashSet::new()) {
            self.emit(
                "TYPE_THREAD_NOT_SENDABLE",
                format!(
                    "{context} requires a thread-sendable type, got `{}`.",
                    type_.name()
                ),
            );
        }
    }

    /// The thread-handle arm of syntaxcheck's declared-type walk
    /// (`check_type_reference`): every `Thread OF M [RES R [STATE S]] TO O`
    /// nested anywhere in a declared type (a collection element, a Map key or
    /// value, a FUNC signature, a `RES` element) must name sendable planes, and
    /// the data plane must not carry a resource (§7: resources ride the resource
    /// plane only).
    pub(super) fn check_thread_sendability(&self, type_: &ParameterType) {
        fn strip_res(type_: &ParameterType) -> &ParameterType {
            match type_ {
                ParameterType::Res(inner) => inner.as_ref(),
                other => other,
            }
        }
        match type_ {
            ParameterType::ListOf(element) => self.check_thread_sendability(strip_res(element)),
            ParameterType::SetOf(element) => self.check_thread_sendability(element),
            ParameterType::MapOf(key, value) => {
                self.check_thread_sendability(key);
                self.check_thread_sendability(strip_res(value));
            }
            ParameterType::Res(inner) => self.check_thread_sendability(inner),
            ParameterType::Func(params, returns, _) => {
                for param in params {
                    self.check_thread_sendability(param);
                }
                self.check_thread_sendability(returns);
            }
            ParameterType::ThreadHandle { msg, res, out, .. } => {
                self.check_thread_sendability(msg);
                self.check_thread_sendability(out);
                self.require_thread_sendable("Thread message type", msg);
                self.require_thread_sendable("Thread output type", out);
                if self.is_resource_type(msg) {
                    let name = msg.name();
                    self.emit(
                        "TYPE_THREAD_NOT_SENDABLE",
                        format!(
                            "Thread message type `{name}` is a resource; the data channel is resource-free — declare it on the resource plane (`Thread OF … RES {name} TO …`)."
                        ),
                    );
                }
                // An absent resource plane is `Nothing`; a present one carries
                // its ` STATE T` inside the nominal's spelling.
                let (plane_resource, plane_state) = res.split_state();
                if !matches!(plane_resource, ParameterType::Nothing) {
                    self.check_thread_sendability(&plane_resource);
                    self.require_thread_sendable("Thread resource type", &plane_resource);
                }
                if let Some(plane_state) = &plane_state {
                    self.check_thread_sendability(plane_state);
                    self.require_thread_sendable("Thread resource STATE type", plane_state);
                }
            }
            _ => {}
        }
    }

    /// syntaxcheck's `check_thread_boundary_sendability`: the values that cross
    /// at `thread::start` (the input and the new handle's planes),
    /// `thread::send` (the message) and `thread::transfer`/`accept` (the
    /// resource plane and its STATE). Runs only for a call that resolved (its
    /// lowered type is known): an unresolvable call is an arity/argument
    /// rejection, and syntaxcheck never reached the boundary rules for it.
    pub(super) fn check_thread_boundary_sendability(
        &self,
        target: &str,
        args: &[IrValue],
        call: &IrValue,
        locals: &HashMap<String, ParameterType>,
    ) {
        use crate::codegen::builtins::thread::{ACCEPT_RESOURCE, TRANSFER_RESOURCE};
        let display = match target {
            TRANSFER_RESOURCE => "thread.transfer",
            ACCEPT_RESOURCE => "thread.accept",
            "thread.start" | "thread.send" => target,
            _ => return,
        };
        let Some(return_type) = self.infer_type(call, locals) else {
            return;
        };
        if matches!(return_type, ParameterType::Unknown) {
            return;
        }
        let arg_types: Vec<Option<ParameterType>> = args
            .iter()
            .map(|arg| self.infer_type(arg, locals))
            .collect();
        match target {
            "thread.start" => {
                // syntaxcheck reaches the boundary rules only for an entry point
                // that is an imported package's exported ISOLATED FUNC; a local
                // function or a lambda was already rejected as the argument.
                let imported_entry = matches!(
                    args.first(),
                    Some(IrValue::FunctionRef { name, .. }) if !self.functions.contains_key(name)
                );
                if !imported_entry {
                    return;
                }
                if let Some(Some(input)) = arg_types.get(1) {
                    self.require_thread_sendable(&format!("Call to `{display}` input"), input);
                }
                if let ParameterType::ThreadHandle {
                    worker: false,
                    msg,
                    res,
                    out,
                    ..
                } = &return_type
                {
                    self.require_thread_sendable(&format!("Call to `{display}` message type"), msg);
                    if !matches!(**res, ParameterType::Nothing) {
                        self.require_thread_sendable(
                            &format!("Call to `{display}` resource type"),
                            &res.without_state(),
                        );
                    }
                    self.require_thread_sendable(&format!("Call to `{display}` output type"), out);
                }
            }
            "thread.send" => {
                if let Some(Some(ParameterType::ThreadHandle { msg, .. })) = arg_types.first() {
                    self.require_thread_sendable(&format!("Call to `{display}` message type"), msg);
                    // The data plane is resource-free: a resource moves across a
                    // thread only via `thread::transfer` (§7).
                    if self.is_resource_type(msg) {
                        self.emit(
                            "TYPE_THREAD_NOT_SENDABLE",
                            format!(
                                "Call to `{display}` message type `{}` is a resource; the message channel is resource-free — use `thread::transfer`.",
                                msg.name()
                            ),
                        );
                    }
                }
            }
            _ => {
                let Some(Some(ParameterType::ThreadHandle { res, .. })) = arg_types.first() else {
                    return;
                };
                let (resource, resource_state) = res.split_state();
                // bug-301 G4: the plane's `STATE T` payload crosses the boundary
                // with the resource (deep-copied into the receiver's arena), so it
                // must be sendable too.
                if let Some(resource_state) = &resource_state {
                    self.require_thread_sendable(
                        &format!("Call to `{display}` resource STATE type"),
                        resource_state,
                    );
                }
                if matches!(resource, ParameterType::Nothing) {
                    self.emit(
                        "TYPE_THREAD_NOT_SENDABLE",
                        format!(
                            "Call to `{display}` requires a thread with a resource plane (`Thread OF … RES Res TO …`); this thread has no resource channel."
                        ),
                    );
                } else if self.is_resource_type(&resource) {
                    // The resource plane carries only thread-sendable resources.
                    self.require_thread_sendable(
                        &format!("Call to `{display}` resource type"),
                        &resource,
                    );
                } else {
                    self.emit(
                        "TYPE_THREAD_NOT_SENDABLE",
                        format!(
                            "Call to `{display}` carries `{}`, which is not a resource; the resource plane moves only resources.",
                            resource.name()
                        ),
                    );
                }
            }
        }
    }

    /// syntaxcheck's TYPE_INLINE_TRAP_REQUIRES_FALLIBLE on the lowered
    /// inline-TRAP shape: `Bind $trap_resN = <scrutinee>` then
    /// `If ResultIsOk($trap_resN)`. The scrutinee survives as the temp's bind
    /// value — a `CallResult` when the source trapped a call, anything else when
    /// it did not — so the two source forms (a non-call; a package constant,
    /// which is not a runtime call) are both readable here.
    ///
    /// `expectTrap`/`expectNTrap` desugar (`testing::desugar`) into this same
    /// shape, but syntaxcheck saw them as assertion calls with their own rule
    /// (`TESTING_EXPECT_TRAP_REQUIRES_FALLIBLE`), so a handler that sets an
    /// `$expect_` temp is skipped.
    pub(super) fn check_inline_trap_scrutinee(
        &self,
        condition: &IrValue,
        handler: &[IrOp],
        temp_consts: &HashMap<&str, &IrValue>,
    ) {
        let IrValue::ResultIsOk { value } = condition else {
            return;
        };
        let IrValue::Local(res) = value.as_ref() else {
            return;
        };
        if !res.starts_with("$trap_res") {
            return;
        }
        if handler
            .iter()
            .any(|op| matches!(op, IrOp::Assign { name, .. } if name.starts_with("$expect_")))
        {
            return;
        }
        let Some(scrutinee) = temp_consts.get(res.as_str()) else {
            return;
        };
        match scrutinee {
            IrValue::CallResult { target, .. } | IrValue::Call { target, .. } => {
                if builtins::is_package_constant(target) {
                    self.emit(
                        "TYPE_INLINE_TRAP_REQUIRES_FALLIBLE",
                        "Inline TRAP requires a fallible call; a package constant is not a call."
                            .to_string(),
                    );
                }
            }
            _ => self.emit(
                "TYPE_INLINE_TRAP_REQUIRES_FALLIBLE",
                "Inline TRAP requires a call to trap; this expression is not a call.".to_string(),
            ),
        }
    }

    /// Whether a type has a defined default value: primitives yes, functions/
    /// results/resources/threads/unions/enums no, collections and records
    /// recurse (cycle-guarded).
    ///
    /// plan-106-B: the container/FUNC/Result/RES/Thread tests are variant matches.
    /// The ` STATE ` test stays a name test on purpose — the clause rides INSIDE a
    /// resource's nominal spelling (`parse` has no `STATE` arm), so there is no
    /// variant to match. The remaining tests are name lookups into the declaration
    /// tables.
    pub(super) fn is_defaultable(&self, type_: &ParameterType, seen: &mut HashSet<String>) -> bool {
        let type_name = type_.name();
        if is_comparable_defaultable_primitive(&type_name) {
            return true;
        }
        // `AttributedString` (plan-89-A) is defaultable (its default is empty
        // text + empty overlay) but NOT comparable, so it is a defaultable-only
        // delta here rather than in `is_comparable_defaultable_primitive`.
        if type_name == "AttributedString" {
            return true;
        }
        // bug-434: a collection is ALWAYS defaultable — its default is the empty
        // collection, which materializes NO element, so the element type's own
        // defaultability is irrelevant. Recursing into the element (as this did
        // before) wrongly rejected `List/Set/Map OF <union|enum|FUNC|RES>` and
        // cascaded that rejection into any record embedding such a field. Codegen
        // (`lower_default_value` → `lower_empty_collection`) already materializes
        // the empty form without recursing into the element, so this only lifts a
        // front-end block. Comparability of a `Set`/`Map`-key element is a
        // SEPARATE check and is unaffected. Kept ahead of the FUNC/RES/STATE and
        // union/enum arms so the collection short-circuit wins.
        if matches!(
            type_,
            ParameterType::ListOf(_) | ParameterType::SetOf(_) | ParameterType::MapOf(_, _)
        ) {
            return true;
        }
        if matches!(
            type_,
            ParameterType::Func(_, _, _) | ParameterType::ResultOf(_) | ParameterType::Res(_)
        ) || is_thread_type(type_)
            || type_name.contains(" STATE ")
        {
            return false;
        }
        if self.close_op_for(&type_name).is_some()
            || self.unions.contains_key(type_name.as_ref())
            || self.enums.contains_key(type_name.as_ref())
        {
            return false;
        }
        if !seen.insert(type_name.clone().into_owned()) {
            return false;
        }
        let result = match self.record_field_lists.get(type_name.as_ref()) {
            Some(fields) => fields.iter().all(|(_, ft)| self.is_defaultable(ft, seen)),
            // On the SOURCE path a name this table has never heard of is an
            // IMPORTED type, not an undefaultable one, and the difference is not
            // observable from here: `build` lowers with deliberately empty external
            // maps, so an importer's `record_field_lists` holds only its own types
            // and every imported record misses, whatever its spelling. Answering
            // "false" rejected legal programs (`MUT c AS pkg::Cursor`, and
            // `STATE Cursor` on an imported record — libsnd's exact shape) for
            // "having no default" when the record is very likely all-Integer
            // (bug-258).
            //
            // Same stance the RES axis takes a few hundred lines up: only a
            // POSITIVELY known type rejects, because an unknown name may be an
            // external package's. A typo cannot ride in on it — syntaxcheck rejects
            // an unresolvable name with `SYMBOL_UNKNOWN_TYPE` before this matters.
            //
            // The PACKAGE path keeps rejecting: there the merged IR carries the full
            // type table and every name is decoded from an id that must exist in it
            // (`decode_type_name` errors on an unknown id), so a miss is genuine
            // absence — and ir::verify is the sole rejecter for decoded `.mfp`, with
            // no syntaxcheck behind it.
            None => self.imported_types_unknown,
        };
        seen.remove(type_name.as_ref());
        result
    }

    /// Whether every path through `ops` leaves the function (mirrors
    /// syntaxcheck's `Flow::AlwaysReturns`): a Return/Fail/ExitProgram op, an If
    /// whose both branches return, a MATCH with an unguarded CASE ELSE whose
    /// every arm returns, or a TRAP whose body returns. Loops never count
    /// (they may run zero times).
    pub(super) fn block_always_returns(
        &self,
        ops: &[IrOp],
        locals: &HashMap<String, ParameterType>,
    ) -> bool {
        let mut locals = locals.clone();
        for op in ops {
            match op {
                IrOp::Return { .. } | IrOp::Fail { .. } | IrOp::ExitProgram { .. } => return true,
                IrOp::If {
                    then_body,
                    else_body,
                    ..
                } => {
                    if self.block_always_returns(then_body, &locals)
                        && self.block_always_returns(else_body, &locals)
                    {
                        return true;
                    }
                }
                IrOp::Match { value, cases, .. } => {
                    // Exhaustive = an unguarded CASE ELSE, or full enum/union
                    // coverage by unguarded arms (mirroring the relocated
                    // exhaustiveness rule, which rejects anything else).
                    let has_else = cases.iter().any(|case| {
                        case.guard.is_none()
                            && matches!(case.pattern, super::super::IrMatchPattern::Else)
                    });
                    let exhaustive = has_else || self.match_covers_all(value, cases, &locals);
                    if exhaustive
                        && cases
                            .iter()
                            .all(|case| self.block_always_returns(&case.body, &locals))
                    {
                        return true;
                    }
                }
                // A function-level `TRAP` is the error *handler* for the
                // preceding statements; on the success path control falls
                // through it without executing the handler. So a trailing
                // `Trap` never makes the block always-return — only the ops
                // *before* it (a success-path `RETURN`) can. The handler
                // returning is irrelevant to fall-through.
                IrOp::Trap { .. } => {}
                IrOp::Bind { name, type_, .. } => {
                    locals.insert(name.clone(), type_.clone());
                }
                _ => {}
            }
        }
        false
    }

    /// Whether the unguarded arms of a MATCH cover every member/variant of its
    /// enum/union scrutinee (the coverage half of `check_match_exhaustive`).
    pub(super) fn match_covers_all(
        &self,
        value: &IrValue,
        cases: &[super::super::IrMatchCase],
        locals: &HashMap<String, ParameterType>,
    ) -> bool {
        let Some(ty) = self.infer_type(value, locals) else {
            return false;
        };
        let ty = resource_base_type(&ty).to_string();
        let all = if let Some(variants) = self.union_variants(&ty) {
            variants
        } else if let Some(members) = self.enums.get(&ty) {
            members.clone()
        } else {
            return false;
        };
        let (covered, has_unguarded_else) = super::fold_match_coverage(cases);
        if has_unguarded_else {
            return true;
        }
        all.difference(&covered).next().is_none()
    }

    /// The `RES` ownership axis on collection element/value types (§15.6, the
    /// sole rejecter): a resource element must be `RES`-marked (`List OF RES
    /// File`), and `RES` may mark only a resource. Recurses through nested
    /// collections; `line` positions are the caller's.
    /// plan-106-B: structural. `strip_prefix("List OF ")` and `parse_map` became
    /// variant matches; the `RES ` marker is the [`Res`](ParameterType::Res)
    /// wrapper rather than a name prefix.
    pub(super) fn check_collection_res_axis(&self, type_: &ParameterType) {
        match type_ {
            ParameterType::ListOf(element) => self.collection_axis_element(element, "element"),
            ParameterType::MapOf(_, value) => self.collection_axis_element(value, "value"),
            _ => {}
        }
    }

    pub(super) fn collection_axis_element(&self, element: &ParameterType, role: &str) {
        let is_res_marked = matches!(element, ParameterType::Res(_));
        let inner = strip_res(element);
        let inner_name = inner.name();
        let is_resource = self.is_resource_or_resource_union(&inner_name);
        if is_resource && !is_res_marked {
            self.emit(
                "TYPE_RESOURCE_REQUIRES_RES",
                format!(
                    "Collection {role} type `{inner_name}` is a resource; mark it `RES` (e.g. `List OF RES File`), not a bare resource type."
                ),
            );
        } else if is_res_marked && !is_resource && self.provably_data_type(&inner_name) {
            self.emit(
                "TYPE_RES_REQUIRES_RESOURCE",
                format!(
                    "Collection {role} is marked `RES` but `{inner_name}` is not a resource type; drop the `RES`."
                ),
            );
        }
        // Nested collections (`List OF List OF RES File`).
        self.check_collection_res_axis(inner);
    }

    // ===========================================================================
}
