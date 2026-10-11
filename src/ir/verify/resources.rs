use super::*;

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
            // (`FOR … canvas::setGroup("panel", [Text[font := face]]) NEXT`) is valid
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
                    self.scan_nested_loop(crate::ast::LoopKind::For, || {
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
                    let entry = (moved.clone(), aliases.clone(), contains.clone());
                    // bug-852: the body's first pass runs on what reaches it from
                    // before the loop AND round the back edge from an earlier
                    // iteration, so a close of an outer resource that falls through
                    // to the next iteration is the use after move it is (§14.8).
                    let carried = self.loop_carried_moves(
                        kind, body, moved, locals, locals, owners, non_owning, aliases, contains,
                    );
                    let start = (!carried.is_empty()).then(|| {
                        let mut start = entry.0.clone();
                        start.extend(carried);
                        start
                    });
                    self.scan_nested_loop(kind, || {
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
        self.back_edge_scan.replace(Some((kind, Vec::new())));
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
    fn scan_nested_loop<T>(&self, kind: crate::ast::LoopKind, scan: impl FnOnce() -> T) -> T {
        let nested = self.back_edge_scan.borrow().is_some();
        if nested {
            if let Some((_, stack)) = self.back_edge_scan.borrow_mut().as_mut() {
                stack.push(kind);
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
        let Some((own, nested)) = scan.as_ref() else {
            return false;
        };
        match op {
            IrOp::ExitLoop { kind, .. } => !nested.contains(kind),
            IrOp::ContinueLoop { kind, .. } => !nested.contains(kind) && kind != own,
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

    /// The first leaf of `type_` that cannot cross a thread boundary, or `None`
    /// when the whole type can — i.e. `None` *is* "thread-sendable" (the former
    /// source checker's `is_thread_sendable_type`): primitives and the built-in
    /// nominals yes; collections and `Result` by their elements; a `RES`-marked
    /// element, a FUNC and a thread handle never; a resource by its declared
    /// sendability; a record by every field, a union by every variant (a
    /// resource variant by its own bit, bug-173 F); enums yes. A name the tables
    /// do not know is treated as sendable — only a positively known type may
    /// reject.
    ///
    /// Structural arms report the **first** blocking child, left to right, so
    /// the named leaf is deterministic.
    ///
    /// plan-114-A: this replaced the boolean `is_thread_sendable`, rather than
    /// joining it. Answering "is it sendable" and "why is it not" from two
    /// independent walks would let them disagree, and a leaf that one rejects
    /// but the other reports as `None` silently drops a diagnostic. What the
    /// cause carries is which of two rules the plane emits — see [`Unsendable`]
    /// and [`Self::require_thread_sendable`].
    pub(super) fn thread_unsendable_cause(
        &self,
        type_: &ParameterType,
        seen: &mut HashSet<ParameterType>,
    ) -> Option<Unsendable> {
        match type_ {
            ParameterType::Boolean
            | ParameterType::Byte
            | ParameterType::Fixed
            | ParameterType::Float
            | ParameterType::Integer
            | ParameterType::Money
            | ParameterType::Nothing
            | ParameterType::String
            | ParameterType::Unknown => None,
            ParameterType::ListOf(element) | ParameterType::SetOf(element) => {
                self.thread_unsendable_cause(element, seen)
            }
            ParameterType::MapOf(key, value) => self
                .thread_unsendable_cause(key, seen)
                .or_else(|| self.thread_unsendable_cause(value, seen)),
            ParameterType::ResultOf(success) => self.thread_unsendable_cause(success, seen),
            // Sharing a resource collection across threads is out of scope (§15.6).
            ParameterType::Res(_) => Some(Unsendable::Resource(type_.clone())),
            ParameterType::Func(..) => Some(Unsendable::Other(type_.clone())),
            // plan-156-D: a parent view is a sendable resource (its planes are
            // checked where the type is declared); a worker's own view never crosses.
            ParameterType::ThreadHandle { worker: true, .. } => {
                Some(Unsendable::Worker(type_.clone()))
            }
            ParameterType::ThreadHandle { worker: false, .. } => {
                self.resource_sendability_cause(type_)
            }
            other => {
                // The built-in nominals are plain values.
                if other.is_named("AttributedString")
                    || other.is_named("Error")
                    || other.is_named("ErrorLoc")
                    || other.is_named("Scalar")
                {
                    return None;
                }
                if self.is_resource_base(other) {
                    return self.resource_sendability_cause(other);
                }
                if !seen.insert(other.clone()) {
                    return None;
                }
                let result = match self.unions.get(other) {
                    Some(union) => union.variant_order.iter().find_map(|variant| {
                        let variant = ParameterType::declared(variant);
                        if self.is_resource_base(&variant) {
                            return self.resource_sendability_cause(&variant);
                        }
                        self.record_fields_unsendable_cause(&variant, seen)
                    }),
                    None => self.record_fields_unsendable_cause(other, seen),
                };
                seen.remove(other);
                result
            }
        }
    }

    /// A bare resource nominal blocks the boundary unless its type is registered
    /// thread-sendable. The cause is always `Resource`: on a data plane that is a
    /// plane mix-up, on the resource plane it means the resource type itself may
    /// not cross. The emitter decides which rule says so.
    fn resource_sendability_cause(&self, base: &ParameterType) -> Option<Unsendable> {
        (!self.is_resource_sendable(base)).then(|| Unsendable::Resource(base.clone()))
    }

    /// The first field of record `type_` that cannot cross; `None` when every
    /// field can. A name that is not a record (an enum, or a type the tables do
    /// not know) is vacuously sendable.
    fn record_fields_unsendable_cause(
        &self,
        type_: &ParameterType,
        seen: &mut HashSet<ParameterType>,
    ) -> Option<Unsendable> {
        self.record_field_lists.get(type_).and_then(|fields| {
            fields
                .iter()
                .find_map(|(_, field_type)| self.thread_unsendable_cause(field_type, seen))
        })
    }

    /// Reject `type_` if it cannot cross `plane`, naming the blocking leaf.
    ///
    /// plan-114-A: a resource that reaches a **data** plane is a plane mix-up
    /// with a specific remedy, so it gets its own rule. Every other rejection
    /// keeps `TYPE_THREAD_NOT_SENDABLE` — a `Func` or thread handle anywhere,
    /// and anything at all on the resource plane, where "not sendable" means the
    /// resource type itself may not cross and the data-plane remedy would be
    /// nonsense (it would name the plane the resource is already on).
    fn require_thread_sendable(&self, context: &str, type_: &ParameterType, plane: Plane) {
        // A resource named directly on a data plane is a plane mix-up even when the
        // resource itself may cross (the cause walk answers `None` for a sendable
        // one): `thread::start`'s input and a thread's output are data planes. The
        // declared message plane checks this itself before calling here.
        if plane == Plane::Data && self.belongs_on_the_resource_plane(type_) {
            self.emit_thread_resource_plane_required(context, type_);
            return;
        }
        match self.thread_unsendable_cause(type_, &mut HashSet::new()) {
            None => {}
            // A worker's view on a RESOURCE plane is the one mistake with a rule of
            // its own; on a data plane it is simply not sendable (2-203-0063).
            Some(Unsendable::Worker(worker)) if plane == Plane::Resource => {
                self.emit_thread_worker_not_transferable(context, &worker)
            }
            Some(cause @ Unsendable::Resource(_)) if plane == Plane::Data => {
                self.emit_thread_resource_plane_required(context, cause.leaf());
            }
            Some(_) => self.emit(
                "TYPE_THREAD_NOT_SENDABLE",
                format!(
                    "{context} requires a thread-sendable type, got `{}`.",
                    type_.name()
                ),
            ),
        }
    }

    /// Whether a resource on a DATA plane is a plane mix-up whose remedy is the
    /// resource plane (`TYPE_THREAD_RESOURCE_PLANE_REQUIRED`).
    ///
    /// bug-975: a `ThreadWorker` is a resource, but it crosses on NEITHER plane —
    /// it belongs to its worker for the worker's whole run — so naming the
    /// resource plane as its remedy would be wrong advice. Its own rule keeps
    /// precedence, as on the resource plane: on a data plane it stays
    /// `TYPE_THREAD_NOT_SENDABLE`.
    fn belongs_on_the_resource_plane(&self, type_: &ParameterType) -> bool {
        self.is_resource_type(type_)
            && !matches!(
                self.thread_unsendable_cause(type_, &mut HashSet::new()),
                Some(Unsendable::Worker(_))
            )
    }

    /// The one place `2-203-0140` is worded.
    fn emit_thread_worker_not_transferable(&self, context: &str, worker: &ParameterType) {
        self.emit(
            "TYPE_THREAD_WORKER_NOT_TRANSFERABLE",
            format!(
                "{context} carries `{}`; a ThreadWorker belongs to its own worker for that worker's whole run and never crosses to another thread.",
                worker.name()
            ),
        );
    }

    /// The one place `2-203-0138` is worded. `resource` is the offending leaf,
    /// which for a nested type is not `type_` itself — naming the leaf rather
    /// than the enclosing spelling is the point of the cause walk.
    fn emit_thread_resource_plane_required(&self, context: &str, resource: &ParameterType) {
        let name = resource.name();
        let bare = strip_res(resource).name();
        self.emit(
            "TYPE_THREAD_RESOURCE_PLANE_REQUIRED",
            format!(
                "{context} carries resource `{name}`; a resource crosses a thread only on the RES plane. Declare `RES {bare}` on the thread and move it with `thread::transfer` / `thread::accept`."
            ),
        );
    }

    /// The thread-handle arm of the former source checker's declared-type walk
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
                // A resource named directly on the message plane is a plane
                // mix-up, and it is reported here rather than by the cause walk:
                // the sendable resources (`fs::File`, `tcp`, `tls`, `udp`) are
                // *sendable*, so the walk answers `None` for them. Rejecting the
                // plane is a separate rule from rejecting the type, and the two
                // are mutually exclusive so one mistake yields one diagnostic.
                if self.belongs_on_the_resource_plane(msg) {
                    self.emit_thread_resource_plane_required("Thread message type", msg);
                } else {
                    self.require_thread_sendable("Thread message type", msg, Plane::Data);
                }
                self.require_thread_sendable("Thread output type", out, Plane::Data);
                // An absent resource plane is `Nothing`; a present one carries
                // its ` STATE T` inside the nominal's spelling.
                let (plane_resource, plane_state) = res.split_state();
                if !matches!(plane_resource, ParameterType::Nothing) {
                    self.check_thread_sendability(&plane_resource);
                    // The resource plane: `Resource` here means this resource
                    // type may not cross at all, not that it is on the wrong
                    // plane — it is already on the right one.
                    self.require_thread_sendable(
                        "Thread resource type",
                        &plane_resource,
                        Plane::Resource,
                    );
                }
                if let Some(plane_state) = &plane_state {
                    self.check_thread_sendability(plane_state);
                    // The `STATE` payload is deep-copied data riding the resource
                    // plane, so it is a DATA plane for this rule (bug-301 G4).
                    self.require_thread_sendable(
                        "Thread resource STATE type",
                        plane_state,
                        Plane::Data,
                    );
                }
            }
            _ => {}
        }
    }

    /// the former source checker's `check_thread_boundary_sendability`: the values that cross
    /// at `thread::start` (the input and the new handle's planes),
    /// `thread::send` (the message) and `thread::transfer`/`accept` (the
    /// resource plane and its STATE). Runs only for a call that resolved (its
    /// lowered type is known): an unresolvable call is an arity/argument
    /// rejection, and the former source checker never reached the boundary rules for it.
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
        // bug-482, the `In` rule. This runs BEFORE the `return_type` guard below,
        // and it is the only boundary rule that may. The `data` argument's
        // sendability is a property of the argument alone — the call's own type
        // says nothing about it — so gating it on a derived return type made it
        // unreachable exactly where it matters. Measured on the repro: in the
        // source pass `infer_type(call)` is `None` (the entry is an imported
        // function whose signature that pass does not hold) while the argument
        // infers fine, so the guard returned before this rule and only the
        // merged/native pass ever reached it — see the comment on the gate below.
        //
        // Keeping it here is also what makes the diagnostic USABLE: the source
        // pass has `current_file`/`current_line` set (measured: `src/main.mfb:18`),
        // so the rule reports at the call site, whereas the native pass reports
        // through `verify_semantics`'s error-return channel, which carries no span
        // and is skipped entirely for `-ast`/`-ir` dumps — the very invocation
        // `scripts/test-accept.sh` uses for a `tests/syntax/` fixture. A rule that
        // only fires in the native pass cannot be pinned by a syntax golden.
        if target == "thread.start" {
            if let Some(Some(input)) = args.get(1).map(|arg| self.infer_type(arg, locals)) {
                self.require_thread_sendable(
                    &format!("Call to `{display}` input"),
                    &input,
                    Plane::Data,
                );
            }
        }
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
            // bug-482: `thread.start`'s only call-site rule is the `In` rule, and
            // it runs above (before the `return_type` guard, which it must not
            // depend on).
            //
            // There used to be three more here — message/resource/output — behind
            // an `imported_entry` gate whose premise ("a `FunctionRef` name absent
            // from `functions` came from an import") is backwards: `TypeEnv::build`
            // inserts every function of `IrProject::functions`, and an imported
            // package's functions are lowered into that same list under the
            // package-keyed name the entry's `FunctionRef` carries (measured:
            // `first="04187eb9abe9cc82.wpkg.w"`,
            // `fnkeys=["$lambda0", "04187eb9abe9cc82.wpkg.w", "main"]`). The gate
            // was false for an imported entry and a same-project one alike, so all
            // four rules were dead.
            //
            // Only the `In` rule was resurrected, because only it has no other
            // guard. The other three are strictly redundant with the declared-type
            // walk (`check_thread_sendability`): `thread::start`'s handle planes
            // are read off the entry's own `ThreadWorker OF Msg TO Out` parameter
            // and return type, which are declarations the walk already checks —
            // whether or not the binding is annotated, and whether the entry is
            // local or imported (the merged pass walks the imported declarations
            // too, so a crafted `.mfp` stays covered). Reviving them reported the
            // same defect twice at one line, measured on
            // `tests/syntax/threads/func_thread_start_invalid` line 18:
            //     …:18 … `Call to `thread.start` message type … got `BadStartMessage`.`
            //     …:18 … `Thread message type … got `BadStartMessage`.`
            // so they are deleted rather than restored into duplicates.
            "thread.send" => {
                if let Some(Some(ParameterType::ThreadHandle { msg, .. })) = arg_types.first() {
                    // The data plane is resource-free: a resource moves across a
                    // thread only via `thread::transfer` (§7). As on the declared
                    // message plane, the plane rule and the sendability rule are
                    // mutually exclusive — a sendable resource still fails the
                    // plane rule, and the cause walk would not report it.
                    if self.belongs_on_the_resource_plane(msg) {
                        self.emit_thread_resource_plane_required(
                            &format!("Call to `{display}` message type"),
                            msg,
                        );
                    } else {
                        self.require_thread_sendable(
                            &format!("Call to `{display}` message type"),
                            msg,
                            Plane::Data,
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
                    // Deep-copied data riding the resource plane: a DATA plane
                    // for this rule.
                    self.require_thread_sendable(
                        &format!("Call to `{display}` resource STATE type"),
                        resource_state,
                        Plane::Data,
                    );
                }
                if matches!(resource, ParameterType::Nothing) {
                    self.emit(
                        "TYPE_THREAD_NOT_SENDABLE",
                        format!(
                            "Call to `{display}` requires a thread with a resource plane (`Thread OF … RES Res TO …`); this thread has no resource channel."
                        ),
                    );
                } else if let Some(cause @ Unsendable::Worker(_)) =
                    self.thread_unsendable_cause(&resource, &mut HashSet::new())
                {
                    self.emit_thread_worker_not_transferable(
                        &format!("Call to `{display}` resource type"),
                        cause.leaf(),
                    );
                } else if self.is_resource_type(&resource) {
                    // The resource plane carries only thread-sendable resources.
                    self.require_thread_sendable(
                        &format!("Call to `{display}` resource type"),
                        &resource,
                        Plane::Resource,
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

    /// The inline-TRAP scrutinee rules on the lowered shape: `Bind $trap_resN =
    /// <scrutinee>` then `If ResultIsOk($trap_resN)`. The scrutinee survives as
    /// the temp's bind value — a `CallResult` when the source trapped a call, a
    /// `Checked` for any other expression (bug-471, plan-165-D). A scrutinee
    /// that cannot fail draws the advisory `TYPE_INLINE_TRAP_DEAD_HANDLER`. Source
    /// lowering produces nothing else since plan-165-D; a `CallResult` of a
    /// package constant or any other bind value (`TYPE_INLINE_TRAP_REQUIRES_FALLIBLE`)
    /// can only come from a crafted or stale package, whose `$trap_res` would
    /// otherwise not be a `Result` at all.
    ///
    /// `expectTrap`/`expectNTrap` desugar (`testing::desugar`) into this same
    /// shape, but the former source checker saw them as assertion calls with their own rule
    /// (`TESTING_EXPECT_TRAP_REQUIRES_FALLIBLE`), so a handler that sets an
    /// `$expect_` temp is skipped.
    pub(super) fn check_inline_trap_scrutinee(
        &self,
        condition: &IrValue,
        handler: &[IrOp],
        temp_consts: &HashMap<&str, &IrValue>,
        locals: &HashMap<String, ParameterType>,
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
        // Every assertion desugar's handler touches an `$expect_` temp: it sets
        // `$expect_trapped` (expectTrap) or reads the `$expect_err` binding the
        // lowering then binds (expectNTrap's `FAIL error(…, err.message)`).
        if handler.iter().any(|op| {
            matches!(op, IrOp::Assign { name, .. } | IrOp::Bind { name, .. } if name.starts_with("$expect_"))
        }) {
            return;
        }
        let Some(scrutinee) = temp_consts.get(res.as_str()) else {
            return;
        };
        match scrutinee {
            // bug-471: a `Checked` scrutinee is a non-call expression the desugar
            // wraps so its domain-error exits join the capture. plan-165-D (Option
            // 1): any well-typed expression may be trapped, so one that holds no
            // raising operator — `a & b`, `n = 3`, a plain read — cannot fail and
            // its handler is dead code, exactly as on a provably total call.
            IrValue::Checked { value, .. } => {
                if !self.checked_value_can_raise(value, locals) {
                    self.emit(
                        "TYPE_INLINE_TRAP_DEAD_HANDLER",
                        "Inline TRAP handler is unreachable — this expression cannot fail, so the handler is dead code."
                            .to_string(),
                    );
                }
            }
            IrValue::CallResult { target, args, .. } | IrValue::Call { target, args, .. } => {
                if builtins::is_package_constant(target) {
                    self.emit(
                        "TYPE_INLINE_TRAP_REQUIRES_FALLIBLE",
                        "Inline TRAP requires a fallible call; a package constant is not a call."
                            .to_string(),
                    );
                } else if builtins::inline_builtin_is_infallible(
                    target,
                    // bug-486: with the argument types, so an overload whose
                    // verdict differs from its name's is judged as the overload it
                    // is — advising the author to delete a live handler is advising
                    // them to delete the only thing between the error and a dead
                    // process. Typed only for the names whose verdict can turn on
                    // them.
                    &if builtins::inline_builtin_fallibility_depends_on_args(target) {
                        args.iter()
                            .map(|arg| {
                                self.infer_type(arg, locals)
                                    .unwrap_or(ParameterType::Unknown)
                            })
                            .collect::<Vec<_>>()
                    } else {
                        Vec::new()
                    },
                    // bug-679: `self` IS the declared-kind oracle (`impl TypeKinds
                    // for TypeEnv`, plan-140-A), so `toInt(<enum>)` is recognised
                    // as the one infallible `toInt` overload and its dead handler
                    // is reported. Every other `toInt`
                    // can really fail and keeps its handler.
                    self,
                ) {
                    // A provably-infallible inline built-in (`len`, `typeName`,
                    // every total `bits::*`, …) under a TRAP compiles and runs; its
                    // handler is dead code — an advisory warning, not an error
                    // (plan-26-A).
                    self.emit(
                        "TYPE_INLINE_TRAP_DEAD_HANDLER",
                        format!(
                            "Inline TRAP handler is unreachable — `{target}` cannot fail, so the handler is dead code."
                        ),
                    );
                }
            }
            _ => self.emit(
                "TYPE_INLINE_TRAP_REQUIRES_FALLIBLE",
                "Inline TRAP requires a call to trap; this expression is not a call.".to_string(),
            ),
        }
    }

    /// Whether a `Checked` value (call-free, `check_checked_has_no_call`) holds
    /// an operator that can raise: the same per-operator census `ir::lower`
    /// lifts on (`operator_can_raise`, `comparison_can_raise` on the operand
    /// types, `unary_operator_can_raise` minus a negative literal). `&` is never
    /// a raise site (owner decision 2026-10-04). An operand of unproven type
    /// counts as raising for a comparison, as the census does.
    fn checked_value_can_raise(
        &self,
        value: &IrValue,
        locals: &HashMap<String, ParameterType>,
    ) -> bool {
        use crate::ir::fallible::{
            comparison_can_raise, is_total_literal_negation, operator_can_raise,
            unary_operator_can_raise,
        };
        let mut raises = false;
        crate::ir::value::visit_value(value, &mut |node| match node {
            IrValue::Binary {
                op,
                type_,
                left,
                right,
                ..
            } => {
                let operand = |operand: &IrValue| {
                    self.infer_type(operand, locals)
                        .unwrap_or(ParameterType::Unknown)
                };
                raises |= operator_can_raise(*op, type_)
                    || comparison_can_raise(*op, &operand(left), &operand(right));
            }
            IrValue::Unary {
                op, type_, operand, ..
            } => {
                let literal = matches!(operand.as_ref(), IrValue::Const { .. })
                    && is_total_literal_negation(*op, type_);
                raises |= unary_operator_can_raise(*op, type_) && !literal;
            }
            _ => {}
        });
        raises
    }

    /// Whether a type has a defined default value: primitives yes, functions/
    /// results/resources/threads/unions/enums no, collections and records
    /// recurse (cycle-guarded).
    ///
    /// plan-106-B: the container/FUNC/Result/RES/Thread tests are variant matches.
    /// The ` STATE ` test stays a name test on purpose: `parse` builds a
    /// [`Stateful`](ParameterType::Stateful) only for a single-token base or a
    /// thread handle, and a clause on any other composite still rides inside one
    /// opaque nominal spelling, where only the name can see it. The remaining tests
    /// are name lookups into the declaration tables.
    pub(super) fn is_defaultable(
        &self,
        type_: &ParameterType,
        seen: &mut HashSet<ParameterType>,
    ) -> bool {
        let type_name = type_.name();
        if is_comparable_defaultable_primitive(type_) {
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
            // plan-111-B: `type_name.contains(" STATE ")` — the BROAD question
            // (a clause anywhere in the spelling, not just this type's own), so
            // `split_state` would be wrong here. `contains_state` is that same
            // question computed structurally, pinned equal to the rendered form
            // by `contains_state_agrees_with_the_rendered_name`. It keeps the
            // opaque-nominal case a crafted `.mfp` can produce (PKG-02).
            || type_.contains_state()
        {
            return false;
        }
        if self.is_resource_base(type_)
            || self.unions.contains_key(type_)
            || self.enums.contains_key(type_)
        {
            return false;
        }
        if !seen.insert(type_.clone()) {
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
            // external package's. A typo cannot ride in on it — the former source checker rejects
            // an unresolvable name with `SYMBOL_UNKNOWN_TYPE` before this matters.
            //
            // The PACKAGE path keeps rejecting: there the merged IR carries the full
            // type table and every name is decoded from an id that must exist in it
            // (`decode_type_name` errors on an unknown id), so a miss is genuine
            // absence — and ir::verify is the sole rejecter for decoded `.mfp`, with
            // no the former source checker behind it.
            None => self.imported_types_unknown,
        };
        seen.remove(type_);
        result
    }

    /// Why a `STATE` clause's type is not a legal payload, or `None` when it is
    /// (bug-1013): a `STATE` type is a record `TYPE` that is copyable and
    /// defaultable, and nothing else.
    ///
    /// Native code generation treats the resource record's one `STATE` word as a
    /// pointer to an arena block the resource owns, with 0 meaning "no payload
    /// yet" (`emit_resource_state_init`, `emit_free_resource_state_block`). A
    /// scalar has no block and the default `String` is a shared constant, so
    /// those two were never payloads the back end could hold; a collection
    /// happened to work and is refused by the same rule, because the contract is
    /// "a record", not "whatever has a block today".
    ///
    /// The kind is decided from what is POSITIVELY known — the project's own
    /// declarations, the imported packages' (`record_decls`, `enums`, `unions`
    /// are all seeded from the `.mfp` type table on the source path) and the
    /// built-in registry. A nominal no table has heard of falls through to
    /// `is_defaultable`, which already carries the source/package split for it
    /// (bug-258): the resolver has rejected a typo, and a decoded `.mfp` has no
    /// unknown names.
    pub(super) fn state_type_fault(&self, state: &ParameterType) -> Option<StateTypeFault> {
        match state {
            // Unresolved, or a generic's own variable: nothing to judge here.
            // The resolver reports the first; the instantiation is judged with
            // its concrete type.
            ParameterType::Unknown | ParameterType::Var(_) | ParameterType::Arg(_) => return None,
            ParameterType::Named(_) | ParameterType::UserOf(..) => {}
            // A scalar, a collection, a `FUNC`, a `Result`, a thread handle, a
            // C ABI type, a `RES` slot or another stateful resource.
            _ => return Some(StateTypeFault::NotRecord),
        }
        // `AttributedString` is spelled as a nominal but is a built-in value
        // type, not a record (`Error` and `ErrorLoc`, the other two built-in
        // nominals, ARE records and pass through to `is_defaultable`).
        if state.is_named("AttributedString")
            || self.is_resource_or_resource_union(state)
            || self.enums.contains_key(state)
            || self.unions.contains_key(state)
        {
            return Some(StateTypeFault::NotRecord);
        }
        if !self.record_decls.contains(state) {
            use crate::codegen::registry::{registry, ResolvedType};
            match registry().resolve_type(&state.name()) {
                Some(ResolvedType::Record(_)) | None => {}
                Some(_) => return Some(StateTypeFault::NotRecord),
            }
        }
        if !self.is_defaultable(state, &mut HashSet::new()) {
            return Some(StateTypeFault::NotDefaultable);
        }
        None
    }

    /// Apply the `STATE` type rule (`TYPE_STATE_INVALID`, bug-1013) to every
    /// clause `type_` spells, at any depth: its own top-level one, a collection
    /// element's, a record field's `RES` slot, a thread's resource plane, a
    /// thread view's.
    ///
    /// Every position that declares a type routes through here, so the rule
    /// cannot hold at a binding and lapse at an element — which is how a
    /// `List OF RES fs::File STATE Color`, a thread plane and a field's
    /// `STATE Integer` all went unchecked while the binding was refused.
    pub(super) fn check_state_clauses(&self, type_: &ParameterType, subject: &str) {
        match type_ {
            ParameterType::Stateful { base, state } => {
                match self.state_type_fault(state) {
                    Some(StateTypeFault::NotRecord) => self.emit(
                        "TYPE_STATE_INVALID",
                        format!(
                            "{subject} STATE type `{state}` is not a record; declare a record TYPE that holds the value and name it instead."
                        ),
                    ),
                    Some(StateTypeFault::NotDefaultable) => self.emit(
                        "TYPE_STATE_INVALID",
                        format!(
                            "{subject} STATE type `{state}` must be a copyable, defaultable record TYPE."
                        ),
                    ),
                    None => {}
                }
                self.check_state_clauses(base, subject);
            }
            ParameterType::ListOf(inner)
            | ParameterType::SetOf(inner)
            | ParameterType::ResultOf(inner)
            | ParameterType::Res(inner) => self.check_state_clauses(inner, subject),
            ParameterType::MapOf(key, value) | ParameterType::MapEntryOf(key, value) => {
                self.check_state_clauses(key, subject);
                self.check_state_clauses(value, subject);
            }
            ParameterType::UserOf(_, args) => {
                for arg in args {
                    self.check_state_clauses(arg, subject);
                }
            }
            ParameterType::Func(params, ret, _) => {
                for param in params {
                    self.check_state_clauses(param, subject);
                }
                self.check_state_clauses(ret, subject);
            }
            ParameterType::ThreadHandle { msg, res, out, .. } => {
                self.check_state_clauses(msg, subject);
                self.check_state_clauses(res, subject);
                self.check_state_clauses(out, subject);
            }
            _ => {}
        }
    }

    /// Whether every path through `ops` leaves the function (mirrors
    /// the former source checker's `Flow::AlwaysReturns`): a Return/Fail/ExitProgram op, an If
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
                // A stray RECOVER (outside any inline-TRAP handler — an error the
                // shape pass reports) lowers to a `$recover_stray` bind; the front
                // end's flow analysis treats every RECOVER as diverging, so the
                // rules built on this predicate must too.
                IrOp::Bind { name, .. } if name.starts_with("$recover_stray") => return true,
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
        let ty = resource_base_type(&ty);
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
