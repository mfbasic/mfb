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
        locals: &mut HashMap<String, String>,
        moved: &mut HashSet<String>,
        owners: &HashMap<String, crate::escape::ResOwner>,
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
                          locals: &HashMap<String, String>,
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
                            if locals
                                .get(source)
                                .is_some_and(|t| self.close_op_for(resource_base_type(t)).is_some())
                            {
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
                            if self.close_op_for(returned).is_some() {
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
        locals: &HashMap<String, String>,
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

    /// Whether a type has a defined default value: primitives yes, functions/
    /// results/resources/threads/unions/enums no, collections and records
    /// recurse (cycle-guarded).
    pub(super) fn is_defaultable(&self, type_: &str, seen: &mut HashSet<String>) -> bool {
        match type_ {
            "Boolean" | "Byte" | "Error" | "ErrorLoc" | "Fixed" | "Float" | "Integer" | "Money"
            | "Nothing" | "Scalar" | "String" | "Unknown" => return true,
            _ => {}
        }
        if let Some(element) = type_.strip_prefix("List OF ") {
            return self.is_defaultable(element, seen);
        }
        if let Some((k, v)) = parse_map(type_) {
            return self.is_defaultable(k, seen) && self.is_defaultable(v, seen);
        }
        if type_.starts_with("FUNC")
            || type_.starts_with("Result")
            || type_.starts_with("RES ")
            || type_.starts_with("Thread")
            || type_.contains(" STATE ")
        {
            return false;
        }
        if self.close_op_for(type_).is_some()
            || self.unions.contains_key(type_)
            || self.enums.contains_key(type_)
        {
            return false;
        }
        if !seen.insert(type_.to_string()) {
            return false;
        }
        let result = match self.record_field_lists.get(type_) {
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
        seen.remove(type_);
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
        locals: &HashMap<String, String>,
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
        locals: &HashMap<String, String>,
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
        let mut covered: HashSet<String> = HashSet::new();
        for case in cases {
            if case.guard.is_some() {
                continue;
            }
            let name_of = |v: &IrValue| match v {
                IrValue::Local(name) => Some(name.clone()),
                IrValue::MemberAccess { member, .. } => Some(member.clone()),
                _ => None,
            };
            match &case.pattern {
                super::super::IrMatchPattern::Else => return true,
                super::super::IrMatchPattern::Value(v) => {
                    if let Some(n) = name_of(v) {
                        covered.insert(n);
                    }
                }
                super::super::IrMatchPattern::OneOf(vs) => {
                    for v in vs {
                        if let Some(n) = name_of(v) {
                            covered.insert(n);
                        }
                    }
                }
            }
        }
        all.difference(&covered).next().is_none()
    }

    /// The `RES` ownership axis on collection element/value types (§15.6, the
    /// sole rejecter): a resource element must be `RES`-marked (`List OF RES
    /// File`), and `RES` may mark only a resource. Recurses through nested
    /// collections; `line` positions are the caller's.
    pub(super) fn check_collection_res_axis(&self, type_: &str) {
        if let Some(element) = type_.strip_prefix("List OF ") {
            self.collection_axis_element(element, "element");
            return;
        }
        if let Some((_, value)) = parse_map(type_) {
            self.collection_axis_element(value, "value");
        }
    }

    pub(super) fn collection_axis_element(&self, element: &str, role: &str) {
        let bare = element.strip_prefix("RES ");
        let inner = bare.unwrap_or(element);
        let is_res_marked = bare.is_some();
        let is_resource = self.is_resource_or_resource_union(inner);
        if is_resource && !is_res_marked {
            self.emit(
                "TYPE_RESOURCE_REQUIRES_RES",
                format!(
                    "Collection {role} type `{inner}` is a resource; mark it `RES` (e.g. `List OF RES File`), not a bare resource type."
                ),
            );
        } else if is_res_marked && !is_resource && self.provably_data_type(inner) {
            self.emit(
                "TYPE_RES_REQUIRES_RESOURCE",
                format!(
                    "Collection {role} is marked `RES` but `{inner}` is not a resource type; drop the `RES`."
                ),
            );
        }
        // Nested collections (`List OF List OF RES File`).
        self.check_collection_res_axis(inner);
    }

    // ===========================================================================
}
