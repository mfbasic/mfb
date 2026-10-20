use super::*;

mod defaultability;
mod flow;
mod sendability;
#[cfg(test)]
mod tests;

/// Which thread plane a boundary type sits on (plan-114-A).
///
/// A thread has two channels: the **data** plane (message, output, input, and a
/// resource plane's deep-copied `STATE` payload) and the **resource** plane
/// (`Thread OF … RES R TO …`, moved with `thread::transfer`/`thread::accept`).
/// A resource that reaches the data plane is a plane mix-up with a remedy worth
/// naming; the same cause on the resource plane means the resource type itself
/// may not cross, which is a different fault with a different rule.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Plane {
    Data,
    Resource,
}

/// Why a type cannot cross a thread boundary — the leaf
/// [`TypeEnv::thread_unsendable_cause`] stopped at, classified by which rule
/// should report it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) enum Unsendable {
    /// A resource reached the boundary: `Res(_)`, or a bare resource nominal
    /// whose type is not registered thread-sendable.
    Resource(ParameterType),
    /// `Func` — genuinely unsendable, not a plane mix-up, so no resource-plane
    /// remedy exists to point at.
    Other(ParameterType),
    /// A `ThreadWorker`: it belongs to its worker for the worker's whole run
    /// (`TYPE_THREAD_WORKER_NOT_TRANSFERABLE`).
    Worker(ParameterType),
}

impl Unsendable {
    /// The offending leaf type. For a nested type this is not the type the
    /// caller passed in — naming the leaf rather than the enclosing spelling is
    /// the whole point of walking for a cause.
    pub(super) fn leaf(&self) -> &ParameterType {
        match self {
            Unsendable::Resource(type_) | Unsendable::Other(type_) | Unsendable::Worker(type_) => {
                type_
            }
        }
    }
}

impl TypeEnv {
    // 7. Resource moves, defaultability, collection RES axis
    // ===========================================================================

    /// Reject a read of a resource binding after it was moved (closed, returned)
    /// — the former source checker's `TYPE_USE_AFTER_MOVE`. On decoded package IR a
    /// use-after-move is a use-after-free / double-free: the resource's backing
    /// handle is released by the move, so a later read hands codegen a dangling
    /// handle. Conservative straight-line dataflow: a move is only tracked
    /// within a linear op sequence (nested blocks get a fresh copy that does not
    /// leak moves back out), so no valid program is ever rejected; it catches
    /// the common close-then-use and double-close. Consumption = a call to the
    /// resource type's registered close op with the binding as its first
    /// argument, `RETURN <resource>`, or a successful `thread::transfer` of it
    /// (bug-804) — on the op's continuation for a propagating call, and only on
    /// the `ResultIsOk` branch for an inline `TRAP`, whose failure arm keeps the
    /// untransferred resource.
    pub(super) fn check_resource_moves(
        &self,
        ops: &[IrOp],
        locals: &mut HashMap<String, ParameterType>,
        moved: &mut HashSet<String>,
        owners: &HashMap<String, crate::ir::resource_escape::ResOwner>,
        non_owning: &HashSet<String>,
        aliases: &mut HashMap<String, HashSet<String>>,
        contains: &mut HashMap<String, HashSet<String>>,
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
        /// plan-116-J: every resource a binding **holds inside it**, transitively.
        ///
        /// Deliberately NOT merged into `aliases`, and the difference is the whole
        /// point. Aliasing is symmetric — a consume through either name marks both —
        /// and containment is directed: consuming the container consumes what it holds,
        /// but closing a held resource must leave the container usable. `Picture.image`
        /// promises exactly that (*"closing it while a scene still names it draws
        /// nothing rather than failing"*), and
        /// `tests/cli/cli_canvas_image_resource.rs` compiles the program that relies on it.
        /// Widening `aliases` to cover containment would reject that program.
        fn contains_closure(
            name: &str,
            contains: &HashMap<String, HashSet<String>>,
        ) -> HashSet<String> {
            let mut seen: HashSet<String> = HashSet::new();
            let mut out: HashSet<String> = HashSet::new();
            let mut stack = vec![name.to_string()];
            while let Some(current) = stack.pop() {
                if !seen.insert(current.clone()) {
                    continue;
                }
                if let Some(next) = contains.get(&current) {
                    for held in next {
                        out.insert(held.clone());
                        stack.push(held.clone());
                    }
                }
            }
            out
        }
        // A branch that always leaves the function never reaches the join, so
        // its moves must not leak past it (the former source checker merges only fall-through
        // branches). Top-level test is enough: a mid-block Return makes the
        // rest unreachable anyway.
        //
        // bug-852: while scanning a loop body for the moves it carries round its back
        // edge, a branch that leaves THAT loop (an `EXIT` or `CONTINUE` aimed past every
        // loop nested in the body) never reaches the back edge either.
        let diverges = |ops: &[IrOp]| -> bool {
            ops.iter().any(|op| {
                matches!(
                    op,
                    IrOp::Return { .. } | IrOp::Fail { .. } | IrOp::ExitProgram { .. }
                ) || self.leaves_scanned_loop(op)
            })
        };
        // Run `body` as a branch: fresh scope, then merge the new moves of a
        // fall-through branch back into the outer set (the former source checker's MaybeMoved —
        // moved on *some* path means unusable after the join).
        //
        // Every branch of one construct starts from `entry`, the state BEFORE the
        // construct — never from the outer set as a sibling branch already merged
        // into it. Starting the `ELSE` from the merged set made a close in the
        // `THEN` a use-after-move in the `ELSE`, two paths that never meet.
        //
        // `start` overrides the moved set the branch begins with: a transfer's
        // success branch begins with the transferred resource moved, and a check
        // chain's failure handler begins without it (bug-804). Only the branch's OWN
        // view changes; the merge below still only adds, so an override never
        // un-moves a name in the outer set.
        type Entry = (
            HashSet<String>,
            HashMap<String, HashSet<String>>,
            HashMap<String, HashSet<String>>,
        );
        let run_branch =
            |body: &[IrOp],
             entry: &Entry,
             start: Option<HashSet<String>>,
             locals: &HashMap<String, ParameterType>,
             moved: &mut HashSet<String>,
             aliases: &mut HashMap<String, HashSet<String>>,
             contains: &mut HashMap<String, HashSet<String>>| {
                let mut branch_moved = start.unwrap_or_else(|| entry.0.clone());
                // Aliases discovered inside a branch merge back the same way moves do:
                // "may alias on *some* fall-through path" is still may-alias after the
                // join, and treating it otherwise would lose the relation exactly where
                // it is needed.
                let mut branch_aliases = entry.1.clone();
                // Containment merges the same way, and for the same reason: a relation
                // established on *some* fall-through path is still a may-hold after the
                // join, and dropping it there would lose it exactly where a later consume
                // needs it.
                let mut branch_contains = entry.2.clone();
                self.check_resource_moves(
                    body,
                    &mut locals.clone(),
                    &mut branch_moved,
                    owners,
                    non_owning,
                    &mut branch_aliases,
                    &mut branch_contains,
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
                    for (name, held) in branch_contains {
                        if locals.contains_key(&name) {
                            let kept: HashSet<String> = held
                                .into_iter()
                                .filter(|t| locals.contains_key(t))
                                .collect();
                            if !kept.is_empty() {
                                contains.entry(name).or_default().extend(kept);
                            }
                        }
                    }
                }
            };
        // bug-804: an inline-`TRAP` transfer's result temp → the resource it sends.
        // The `Bind $trap_res = CallResult(transfer(...))` and the
        // `If ResultIsOk($trap_res)` that consumes it are always siblings in one op
        // sequence (both desugar shapes in `lower_inline_trap`), so this map is
        // local to the sequence.
        let mut pending_transfers: HashMap<String, String> = HashMap::new();
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
                    let detail = if self.trap_closed.borrow().contains(name) {
                        format!(
                            "Resource `{name}` is closed when an error reaches the function's \
                             TRAP, so the handler cannot use it."
                        )
                    } else {
                        format!("Binding `{name}` was moved and cannot be used again.")
                    };
                    self.emit("TYPE_USE_AFTER_MOVE", detail);
                }
            }
            if let Some(consumed) = self.consumed_resource(op, locals) {
                // plan-59-E: a non-owning pointer (a resource parameter, a `FOR EACH`
                // element) used to be forbidden from closing/returning/transferring
                // here (`TYPE_RESOURCE_INVALIDATE_NOT_OWNER`, retired). That rule
                // is what made `closeSound(sound AS SoundFile)` — "take a
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
            // plan-116-J: a CONSUMING PARAMETER consumes every resource reachable from
            // its argument. `canvas::setGroup(name, items)` is the first: the group
            // outlives the `present` that draws it, so it takes over closing the images
            // and fonts its items name — and it can only do that if the caller's
            // bindings stop owning them. Without this, scope-drop closes them at the
            // producing scope and the group draws nothing, which is silent.
            //
            // Ordered AFTER the read check above deliberately: the consuming call reads
            // the container, and the container is not what gets moved, so there is no
            // self-report to avoid. It is ordered after `consumed_resource` for the same
            // reason a close is — a SECOND install of the same resource must see the
            // first one's move.
            //
            // bug-852: not carried round a loop's back edge. A consuming parameter
            // takes over what its argument names on EVERY pass, and a resource it
            // already owns is simply named again: the rebuild-every-frame loop
            // (`FOR … canvas::setGroup("panel", [Text[fonts := [face]]]) NEXT`) is valid
            // and the group closes the font once. Only a close, a `RETURN` and a
            // `RES` rebind are moves the next iteration must see.
            let carrying = self.back_edge_scan.borrow().is_some();
            for held in self
                .consumed_contained(op, locals, contains)
                .into_iter()
                .filter(|_| !carrying)
            {
                // The argument may hold the resource directly or through nested
                // containers (`[Picture[image := img]]` is a list holding a record
                // holding the resource), so the closure walks the whole chain.
                for name in contains_closure(&held, contains) {
                    for alias in alias_closure(&name, aliases) {
                        moved.insert(alias);
                    }
                    moved.insert(name);
                }
                // …and the argument itself, when a resource was passed where a
                // container was expected — nothing does that today, but a `Local` that
                // IS the resource is the degenerate case of "reachable from".
                if locals
                    .get(&held)
                    .is_some_and(|t| self.is_resource_base(&resource_base_type(t)))
                {
                    for alias in alias_closure(&held, aliases) {
                        moved.insert(alias);
                    }
                    moved.insert(held);
                }
            }
            // bug-804: a resource-plane `thread::transfer`. A plain `Call` propagates
            // its failure out of this scope — to the caller, or to the function-level
            // `TRAP`, whose handler already treats every local resource as closed — so
            // the only continuation of the op itself is the success edge, and the
            // resource is moved there. An inline `CallResult` defers the move to its
            // `ResultIsOk` branch (the `If` arm below): its failure arm keeps the
            // resource. Ordered after the read check for the same reason a close is.
            match op {
                IrOp::Eval { value, .. }
                | IrOp::Assign { value, .. }
                | IrOp::Bind {
                    value: Some(value), ..
                } if matches!(value, IrValue::Call { .. }) => {
                    // bug-852: a transfer's move is not carried round a loop's back
                    // edge. A retry loop that leaves once a transfer succeeds
                    // (`WHILE NOT sent … transfer(t, f) TRAP … sent = FALSE`) is
                    // valid, and the path that makes it so is a flag this check does
                    // not track; a genuinely repeated transfer is refused at run time
                    // (`ErrResourceMoved`).
                    let carrying = self.back_edge_scan.borrow().is_some();
                    if let Some(sent) = self
                        .transferred_resource(value, locals)
                        .filter(|_| !carrying)
                    {
                        for alias in alias_closure(&sent, aliases) {
                            moved.insert(alias);
                        }
                        moved.insert(sent);
                    }
                }
                IrOp::Bind {
                    name,
                    value: Some(value),
                    ..
                }
                | IrOp::Assign { name, value, .. } => {
                    pending_transfers.remove(name);
                    if matches!(value, IrValue::CallResult { .. }) {
                        // bug-852: not carried round a back edge (see above).
                        if let Some(sent) = self
                            .transferred_resource(value, locals)
                            .filter(|_| self.back_edge_scan.borrow().is_none())
                        {
                            pending_transfers.insert(name.clone(), sent);
                        }
                    }
                }
                _ => {}
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
                                .is_some_and(|t| self.is_resource_base(&resource_base_type(t)))
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
                            if self.is_resource_base(&returned) {
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
                    // plan-116-J: what this binding HOLDS. Structural only — a
                    // constructor, a literal, a union wrap — never through an opaque
                    // call, because a callee's return value does not in general hold its
                    // arguments and recording that it might would move bindings the
                    // program still owns. An over-approximation here is a false
                    // `TYPE_USE_AFTER_MOVE` on correct code, which is the unsafe
                    // direction for THIS relation (it is the safe direction for
                    // `aliases`, where a missed edge is a silent use-after-close — the
                    // two relations approximate opposite ways and that is not an
                    // inconsistency).
                    if let Some(value) = value {
                        let held = self.held_resources(value, locals, contains);
                        if !held.is_empty() {
                            contains.insert(name.clone(), held);
                        }
                    }
                    locals.insert(name.clone(), type_.clone());
                }
                // bug-1092: a reassignment records what the new value holds, exactly as
                // the `Bind` above does — `items = collections::append(items, x)` is how
                // a list acquires its resources, and only the initial `Bind` was read
                // before. `held_resources` reads a named container's existing holdings
                // (the `Local` arm), so a self-referential append yields the union of
                // the old and new holdings; a value that does not name `name` replaces
                // the entry. An empty result leaves the entry alone, matching `Bind`
                // and codegen's twin `resource_containment`.
                IrOp::Assign { name, value, .. } => {
                    let held = self.held_resources(value, locals, contains);
                    if !held.is_empty() {
                        contains.insert(name.clone(), held);
                    }
                }
                IrOp::If {
                    condition,
                    then_body,
                    else_body,
                    ..
                } => {
                    // bug-804: `If ResultIsOk($trap_res)` over an inline-`TRAP`
                    // transfer. The success branch begins with the resource (and every
                    // name that may denote it) moved; the failure branch begins without.
                    // The ordinary merge then leaves it maybe-moved after the join when
                    // the success branch falls through — which is the conservative rule:
                    // after recovery it is moved on *some* path. A diverging success
                    // branch adds nothing to the join.
                    let entry = (moved.clone(), aliases.clone(), contains.clone());
                    let success_moves: HashSet<String> = match condition {
                        IrValue::ResultIsOk { value } => match value.as_ref() {
                            IrValue::Local(result) => pending_transfers
                                .get(result)
                                .map(|sent| {
                                    let mut names = alias_closure(sent, aliases);
                                    names.insert(sent.clone());
                                    names.retain(|name| !moved.contains(name));
                                    names
                                })
                                .unwrap_or_default(),
                            _ => HashSet::new(),
                        },
                        _ => HashSet::new(),
                    };
                    // A check chain (a fallible call nested in the transfer's
                    // arguments) runs its single handler AFTER the checks, behind a
                    // failure flag every failing check's `else` sets first. The
                    // handler only runs when this transfer did not succeed, so it is
                    // entitled to see the resource unmoved — record which names this
                    // flag restores.
                    if !success_moves.is_empty() {
                        if let Some(IrOp::Assign {
                            name: flag,
                            value: IrValue::Const { type_, value },
                            ..
                        }) = else_body.first()
                        {
                            if flag.starts_with('$')
                                && *type_ == ParameterType::Boolean
                                && value == "true"
                            {
                                self.transfer_failure_flags
                                    .borrow_mut()
                                    .entry(flag.clone())
                                    .or_default()
                                    .extend(success_moves.iter().cloned());
                            }
                        }
                    }
                    let then_start = if !success_moves.is_empty() {
                        let mut start = entry.0.clone();
                        start.extend(success_moves);
                        Some(start)
                    } else if let IrValue::Local(flag) = condition {
                        // The chain's handler: `If $trap_failed { … }`.
                        self.transfer_failure_flags.borrow().get(flag).map(|kept| {
                            let mut start = entry.0.clone();
                            start.retain(|name| !kept.contains(name));
                            start
                        })
                    } else {
                        None
                    };
                    run_branch(
                        then_body, &entry, then_start, locals, moved, aliases, contains,
                    );
                    run_branch(else_body, &entry, None, locals, moved, aliases, contains);
                }
                IrOp::Match { cases, .. } => {
                    let entry = (moved.clone(), aliases.clone(), contains.clone());
                    for case in cases {
                        run_branch(&case.body, &entry, None, locals, moved, aliases, contains);
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
                    // bug-852: what an earlier iteration moved is moved here too.
                    let carried = self.loop_carried_moves(
                        crate::ast::LoopKind::For,
                        Some(name.clone()),
                        body,
                        moved,
                        &fe_locals,
                        locals,
                        owners,
                        &fe_non_owning,
                        aliases,
                        contains,
                    );
                    let mut branch_moved = moved.clone();
                    branch_moved.extend(carried);
                    let mut fe_aliases = aliases.clone();
                    let mut fe_contains = contains.clone();
                    self.scan_nested_loop(crate::ast::LoopKind::For, Some(name.clone()), || {
                        self.check_resource_moves(
                            body,
                            &mut fe_locals,
                            &mut branch_moved,
                            owners,
                            &fe_non_owning,
                            &mut fe_aliases,
                            &mut fe_contains,
                        )
                    });
                    for n in branch_moved {
                        if locals.contains_key(&n) {
                            moved.insert(n);
                        }
                    }
                }
                IrOp::While { body, .. } | IrOp::For { body, .. } | IrOp::DoUntil { body, .. } => {
                    let kind = match op {
                        IrOp::While { kind, .. } => *kind,
                        IrOp::For { .. } => crate::ast::LoopKind::For,
                        _ => crate::ast::LoopKind::Do,
                    };
                    // Only a `FOR` carries a name; a named EXIT/CONTINUE resolves to it.
                    let loop_name = match op {
                        IrOp::For { name, .. } => Some(name.clone()),
                        _ => None,
                    };
                    let entry = (moved.clone(), aliases.clone(), contains.clone());
                    // bug-852: the body's first pass runs on what reaches it from
                    // before the loop AND round the back edge from an earlier
                    // iteration, so a close of an outer resource that falls through
                    // to the next iteration is the use after move it is (§14.8).
                    let carried = self.loop_carried_moves(
                        kind,
                        loop_name.clone(),
                        body,
                        moved,
                        locals,
                        locals,
                        owners,
                        non_owning,
                        aliases,
                        contains,
                    );
                    let start = (!carried.is_empty()).then(|| {
                        let mut start = entry.0.clone();
                        start.extend(carried);
                        start
                    });
                    self.scan_nested_loop(kind, loop_name, || {
                        run_branch(body, &entry, start, locals, moved, aliases, contains)
                    });
                }
                // bug-676: every route into the function-level handler closes the
                // function's own resources first (`trap_route_cleanups`, codegen:
                // "trap-shared resources *are* still dropped here"), so inside the
                // handler each one is used after its close — which read the closed
                // handle's freed record and crashed. A resource parameter and a
                // `FOR EACH` element are not the function's to close (`non_owning`),
                // and a thread is closed but readable (bug-622), so neither is
                // counted. A resource with no close op (a `ThreadWorker`, bug-975)
                // is not closed by the routing either, so the test is the close op,
                // not resource-ness.
                IrOp::Trap { body, .. } => {
                    let closed: HashSet<String> = locals
                        .iter()
                        .filter(|(name, type_)| {
                            !non_owning.contains(*name)
                                && self.close_op_for(&resource_base_type(type_)).is_some()
                        })
                        .map(|(name, _)| name.clone())
                        .collect();
                    let mut handler_moved = moved.clone();
                    handler_moved.extend(closed.iter().cloned());
                    let previous = self.trap_closed.replace(closed);
                    self.check_resource_moves(
                        body,
                        &mut locals.clone(),
                        &mut handler_moved,
                        owners,
                        non_owning,
                        &mut aliases.clone(),
                        &mut contains.clone(),
                    );
                    self.trap_closed.replace(previous);
                }
                _ => {}
            }
        }
    }

    /// bug-852: the outer bindings a loop body leaves moved on a path that reaches
    /// its back edge — the moves an earlier iteration hands the next one. Computed
    /// by scanning the body once in back-edge mode (`back_edge_scan`) with every
    /// diagnostic of that scan discarded: the real scan that follows, seeded with
    /// these, reports each problem once. Empty while an enclosing loop's own
    /// back-edge scan is running: the moved set after one pass is already complete
    /// (a pass only adds moves), so nesting does not multiply the work.
    #[allow(clippy::too_many_arguments)]
    fn loop_carried_moves(
        &self,
        kind: crate::ast::LoopKind,
        name: Option<String>,
        body: &[IrOp],
        moved: &HashSet<String>,
        body_locals: &HashMap<String, ParameterType>,
        outer_locals: &HashMap<String, ParameterType>,
        owners: &HashMap<String, crate::ir::resource_escape::ResOwner>,
        non_owning: &HashSet<String>,
        aliases: &HashMap<String, HashSet<String>>,
        contains: &HashMap<String, HashSet<String>>,
    ) -> HashSet<String> {
        if self.back_edge_scan.borrow().is_some() {
            return HashSet::new();
        }
        let diags = self.diags.borrow().len();
        let poisoned = self.poisoned.get();
        let line = self.current_line.get();
        let trap_closed = self.trap_closed.borrow().clone();
        let transfer_flags = self.transfer_failure_flags.borrow().clone();
        self.back_edge_scan
            .replace(Some(((kind, name), Vec::new())));
        let mut scanned = moved.clone();
        self.check_resource_moves(
            body,
            &mut body_locals.clone(),
            &mut scanned,
            owners,
            non_owning,
            &mut aliases.clone(),
            &mut contains.clone(),
        );
        let leaves = body.iter().any(|op| {
            matches!(
                op,
                IrOp::Return { .. } | IrOp::Fail { .. } | IrOp::ExitProgram { .. }
            ) || self.leaves_scanned_loop(op)
        });
        self.back_edge_scan.replace(None);
        self.diags.borrow_mut().truncate(diags);
        self.poisoned.set(poisoned);
        self.current_line.set(line);
        self.trap_closed.replace(trap_closed);
        self.transfer_failure_flags.replace(transfer_flags);
        if leaves {
            return HashSet::new();
        }
        scanned
            .into_iter()
            .filter(|name| !moved.contains(name) && outer_locals.contains_key(name))
            .collect()
    }

    /// bug-852: run `scan` over a loop nested inside the body a back-edge scan is
    /// walking, with that loop's kind on the scan's stack, so an `EXIT`/`CONTINUE`
    /// aimed at it stays inside the scanned body.
    fn scan_nested_loop<T>(
        &self,
        kind: crate::ast::LoopKind,
        name: Option<String>,
        scan: impl FnOnce() -> T,
    ) -> T {
        let nested = self.back_edge_scan.borrow().is_some();
        if nested {
            if let Some((_, stack)) = self.back_edge_scan.borrow_mut().as_mut() {
                stack.push((kind, name));
            }
        }
        let result = scan();
        if nested {
            if let Some((_, stack)) = self.back_edge_scan.borrow_mut().as_mut() {
                stack.pop();
            }
        }
        result
    }

    /// bug-852: whether `op` leaves the loop a back-edge scan is walking: an `EXIT`
    /// of any kind no loop nested inside the body matches (it targets the scanned
    /// loop or one outside it), or a `CONTINUE` aimed past both the nested loops and
    /// the scanned loop itself (a `CONTINUE` of the scanned loop reaches its back
    /// edge). Always `false` outside a back-edge scan.
    fn leaves_scanned_loop(&self, op: &IrOp) -> bool {
        let scan = self.back_edge_scan.borrow();
        let Some(((own_kind, own_name), nested)) = scan.as_ref() else {
            return false;
        };
        // A named exit/continue matches a nested loop by name; a bare one by kind.
        let nested_named = |name: &String| {
            nested
                .iter()
                .any(|(_, n)| n.as_deref() == Some(name.as_str()))
        };
        let nested_kind = |kind: &crate::ast::LoopKind| nested.iter().any(|(k, _)| k == kind);
        match op {
            IrOp::ExitLoop { kind, target, .. } => match target {
                Some(name) => !nested_named(name),
                None => !nested_kind(kind),
            },
            IrOp::ContinueLoop { kind, target, .. } => match target {
                Some(name) => !nested_named(name) && own_name.as_deref() != Some(name.as_str()),
                None => !nested_kind(kind) && kind != own_kind,
            },
            _ => false,
        }
    }

    /// Whether the just-checked value's type is undeterminable the way
    /// the former source checker's inference would see it: either a poisoning rule fired and
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
        // On the source path `ir::shape` judges the HIR initializer directly —
        // a value lowering could not type, and every constructor / `WITH`
        // (the checker typed a read-only nominal's constructor and left an
        // `Ok[…]` or a union's untyped, verdicts the IR node does not carry);
        // only an operator node whose type a rule of THIS checker invalidated
        // cascades here. The package path keeps the full test.
        if self.source_path.get() {
            return matches!(value, IrValue::Binary { .. } | IrValue::Unary { .. });
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
    fn is_resource_sendable(&self, base: &ParameterType) -> bool {
        self.resource_sendable
            .get(base)
            .copied()
            .unwrap_or_else(|| {
                // plan-111-B: the registry half still speaks names (its `&str`
                // signature dies in letter E), so the type renders only there.
                crate::codegen::resource::is_builtin_sendable_resource_type(&base)
            })
    }

    /// A resource type: a registered resource, a `RES`-marked element, or a
    /// resource union (every variant a resource) — the former source checker's `is_resource_type`.
    pub(super) fn is_resource_type(&self, type_: &ParameterType) -> bool {
        match type_ {
            ParameterType::Res(inner) => self.is_resource_type(inner),
            other => {
                self.is_resource_base(other)
                    || self.unions.get(other).is_some_and(|union| {
                        !union.variant_order.is_empty()
                            && union.variant_order.iter().all(|variant| {
                                self.is_resource_base(&ParameterType::declared(variant))
                            })
                    })
            }
        }
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
            ParameterType::ListOf(element) => {
                self.res_axis_slot(element, "Collection element", "`List OF RES File`")
            }
            ParameterType::MapOf(_, value) => {
                self.res_axis_slot(value, "Collection value", "`List OF RES File`")
            }
            _ => {}
        }
    }

    /// The `RES` ownership axis for one slot that may hold a resource.
    ///
    /// plan-114-D: a record **field** is such a slot too, so this is
    /// parameterized by `subject` (what the message calls the slot) and
    /// `example` (a correct spelling to suggest) rather than being
    /// collection-only. Both call sites share this one decision so the two
    /// spellings cannot drift — a record field is governed by exactly the rules
    /// a collection element is, which is the whole point of retiring
    /// `TYPE_RESOURCE_FIELD_FORBIDDEN`.
    pub(super) fn res_axis_slot(&self, slot: &ParameterType, subject: &str, example: &str) {
        let is_res_marked = matches!(slot, ParameterType::Res(_));
        let inner = strip_res(slot);
        let inner_name = inner.name();
        let is_resource = self.is_resource_or_resource_union(inner);
        if is_resource && !is_res_marked {
            self.emit(
                "TYPE_RESOURCE_REQUIRES_RES",
                format!(
                    "{subject} type `{inner_name}` is a resource; mark it `RES` (e.g. {example}), not a bare resource type."
                ),
            );
        } else if is_res_marked && !is_resource && self.provably_data_type(inner) {
            self.emit(
                "TYPE_RES_REQUIRES_RESOURCE",
                format!(
                    "{subject} is marked `RES` but `{inner_name}` is not a resource type; drop the `RES`."
                ),
            );
        }
        // Nested collections (`List OF List OF RES File`).
        self.check_collection_res_axis(inner);
    }

    // ===========================================================================
}
