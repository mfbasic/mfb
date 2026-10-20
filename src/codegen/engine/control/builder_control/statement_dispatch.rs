//! plan-176-U: `lower_ops_inner`, the per-`NirOp` statement dispatch. The statement entry
//! (`lower_ops`) and shared types live in `engine/control/builder_control/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    pub(super) fn lower_ops_inner(
        &mut self,
        ops: &[NirOp],
        cleanup_scope_start: usize,
    ) -> Result<(), String> {
        let zero_slot = self.temporary_vreg();
        for op in ops {
            // plan-39 I1: keep the guard-derived range maps sound. Any op that can
            // re-enter or transfer control non-linearly (loops / Match / Trap) drops
            // all bounds *before* it is lowered (a `While`'s own condition then
            // re-establishes the strict-upper bound for its body). Per-local
            // invalidation on `Bind`/`Assign` happens *after* the op is lowered
            // (below), so the RHS of `i = i + 1` can still use `i`'s pre-assignment
            // bound. Dropping a bound only ever keeps an overflow check, so this is
            // unconditionally safe.
            match op {
                NirOp::While { .. }
                | NirOp::For { .. }
                | NirOp::ForEach { .. }
                | NirOp::DoUntil { .. }
                | NirOp::Match { .. }
                | NirOp::Trap { .. } => {
                    self.integer_lower_bounds.clear();
                    self.integer_strict_upper.clear();
                }
                _ => {}
            }
            // Fresh interior heap temporaries produced while lowering this
            // statement are freed when it finishes (plan-25 temp-lifetime fix), so
            // a hot loop body does not accumulate them until the function returns.
            let temp_watermark = self.pending_temp_frees.len();
            // plan-118-A: attribute the instructions this statement emits to its
            // op kind. The closure below is what makes the pairing safe — no `?`
            // can jump over the `exit`.
            // plan-134-D: a store lowered for this op asks plan-134-C's analysis about
            // THIS op; a nested body records its own and this op's key comes back after.
            let enclosing_op_key = self
                .current_op_key
                .replace(crate::codegen::engine::analysis::last_use::op_key(op));
            crate::codegen::engine::expansion::enter(
                || crate::codegen::engine::expansion::op_key(op).to_string(),
                self.instructions.len(),
            );
            // bug-842: a binding, an assignment and a module-level store take the
            // resources their own value carries, so that root is no temporary.
            let enclosing_temp_exempt = std::mem::replace(
                &mut self.resource_temp_exempt,
                match op {
                    NirOp::Bind {
                        value: Some(value), ..
                    }
                    | NirOp::Assign { value, .. }
                    | NirOp::StoreGlobal {
                        value: Some(value), ..
                    } => {
                        // A resource union's bind wraps its producer
                        // (`RES s AS Stream = tls::connect(…)`): the producer is
                        // the root the binding claims.
                        let mut root = value;
                        while let NirValue::UnionWrap { value: inner, .. } = root {
                            root = inner;
                        }
                        Some(root as *const NirValue as usize)
                    }
                    _ => None,
                },
            );
            let result = (|| -> Result<(), String> {
                match op {
                    NirOp::Bind { .. } => self.lower_bind_op(op, zero_slot)?,
                    NirOp::StoreGlobal { name, type_, value } => {
                        // bug-806: a module-level binding is the outermost scope
                        // there is. Every resource a store puts into one is raised
                        // to the module instance, which closes it at instance
                        // teardown — before the store when the stored operands are
                        // named values, after it (over the whole global) when an
                        // operand is a call whose resources have no name.
                        let walk_after =
                            self.emit_module_store_float(name, type_, value.as_ref())?;
                        self.lower_store_global(op, name, type_, value.as_ref())?;
                        if walk_after {
                            self.emit_module_float_global(name)?;
                        }
                        // bug-1030: a module-level `RES g AS T STATE S` owns its
                        // state as a local owner does, so the store
                        // default-initializes the payload the same way (a
                        // resource that arrives carrying one keeps it).
                        let declared = self.global_value(name)?.type_.clone();
                        if let Some(state_type) = declared.state() {
                            let address = self.load_global_address(name)?;
                            let stored = self.allocate_register();
                            self.emit(abi::load_u64(stored, address, 0));
                            let slot = self.spill_to_slot("module_state_init", stored);
                            self.emit_resource_state_init(slot, &state_type, &declared)?;
                        }
                    }
                    NirOp::Assign { .. } => self.lower_assign_op(op)?,
                    NirOp::StateAssign { .. } => self.lower_state_assign_op(op)?,
                    NirOp::Eval { value } => {
                        // bug-744: a statement discards its value, so a
                        // `Nothing` call here needs no `NOTHING` materialized.
                        self.value_discarded_armed = true;
                        let lowered = self.lower_value(value);
                        self.value_discarded_armed = false;
                        let lowered = lowered?;
                        // bug-755: a discarded call result closes what nobody owns —
                        // a record the callee produced or released — and leaves a
                        // handle the callee was passed and handed back to its owner.
                        if matches!(
                            value,
                            NirValue::Call { .. }
                                | NirValue::CallResult { .. }
                                | NirValue::RuntimeCall { .. }
                        ) {
                            self.emit_close_discarded_value(&lowered.location, &lowered.type_)?;
                        }
                    }
                    NirOp::Return { value } => {
                        // bug-567: the watermark travels with the statement so the
                        // return can free its own interior temps while the code is
                        // still reachable — before the `ret`, not after it.
                        self.emit_return_exit(value.as_ref(), Some(temp_watermark))?;
                    }
                    NirOp::ExitLoop { kind, target } => {
                        let target = self
                            .loop_stack
                            .iter()
                            .rev()
                            .find(|labels| labels.resolves(*kind, target))
                            .cloned()
                            .ok_or_else(|| "native code EXIT has no matching loop".to_string())?;
                        self.emit_cleanup_branch_to_depth(
                            &target.exit_label,
                            target.cleanup_depth,
                            target.temp_depth,
                        )?;
                    }
                    NirOp::ContinueLoop { kind, target } => {
                        let target = self
                            .loop_stack
                            .iter()
                            .rev()
                            .find(|labels| labels.resolves(*kind, target))
                            .cloned()
                            .ok_or_else(|| {
                                "native code CONTINUE has no matching loop".to_string()
                            })?;
                        self.emit_cleanup_branch_to_depth(
                            &target.continue_label,
                            target.cleanup_depth,
                            target.temp_depth,
                        )?;
                    }
                    NirOp::ExitProgram { code } => {
                        self.emit_program_exit_value(code)?;
                    }
                    NirOp::Fail { error } => {
                        self.emit_error_value_exit(
                            error,
                            self.error_exit_destination(),
                            temp_watermark,
                        )?;
                    }
                    NirOp::If {
                        condition,
                        then_body,
                        else_body,
                    } => {
                        // plan-39 I1: derive the fall-through lower bound from the
                        // raw condition before it is lowered/shadowed.
                        let guard = self.guard_lower_bound(condition);
                        let bounds_before_if = self.integer_lower_bounds.clone();
                        let condition = self.lower_value(condition)?;
                        let else_label = self.label("if_else");
                        let end_label = self.label("if_end");
                        let constants_before_if = self.local_constants();
                        self.emit_branch_unless(&condition.location, &else_label, Some("ifFalse"));
                        self.lower_ops(then_body)?;
                        let then_terminal = self.current_block_returns();
                        if !then_terminal {
                            self.emit(abi::branch(&end_label));
                        }
                        self.emit(abi::label(&else_label));
                        self.restore_local_constants(&constants_before_if);
                        self.integer_lower_bounds = bounds_before_if;
                        self.lower_ops(else_body)?;
                        self.emit(abi::label(&end_label));
                        self.clear_local_constants();
                        // The merged path can inherit no bound (either branch may
                        // have reassigned), so clear conservatively — then, if the
                        // then-branch always exits and the else is empty, the
                        // guard condition is provably false here: record its bound.
                        self.integer_lower_bounds.clear();
                        if then_terminal && else_body.is_empty() {
                            if let Some((name, bound)) = guard {
                                self.integer_lower_bounds.insert(name, bound);
                            }
                        }
                    }
                    NirOp::Match { value, cases } => {
                        let matched = self.lower_value(value)?;
                        let matched_slot = self.allocate_stack_object("match_value", 8);
                        // bug-845: a `d`-native `Float` scrutinee is stored from its
                        // FP register; a GP store of it wrote some other register's
                        // bits, so a `CASE 1.5` missed a scrutinee of `1.5`.
                        self.store_value_at(&matched, abi::stack_pointer(), matched_slot);
                        let end_label = self.label("match_end");
                        for case in cases {
                            let matched_register = self.allocate_register();
                            self.emit(abi::load_u64(
                                &matched_register,
                                abi::stack_pointer(),
                                matched_slot,
                            ));
                            let case_matched = ValueResult {
                                origin: None,
                                type_: matched.type_.clone(),
                                location: Operand::from(matched_register),
                                text: matched.text.clone(),
                            };
                            let next_label = self.label("match_next");
                            match &case.pattern {
                                NirMatchPattern::Else => {}
                                NirMatchPattern::Value(pattern) => {
                                    let case_label = self.label("match_case");
                                    self.lower_match_compare(&case_matched, pattern, &case_label)?;
                                    self.emit(abi::branch(&next_label));
                                    self.emit(abi::label(&case_label));
                                }
                                NirMatchPattern::OneOf(patterns) => {
                                    let case_label = self.label("match_case");
                                    for pattern in patterns {
                                        self.lower_match_compare(
                                            &case_matched,
                                            pattern,
                                            &case_label,
                                        )?;
                                    }
                                    self.emit(abi::branch(&next_label));
                                    self.emit(abi::label(&case_label));
                                }
                            }
                            let constants_before_case = self.local_constants();
                            let mut case_locals = self.locals.clone();
                            let mut body_index = 0;
                            while let Some(NirOp::Bind {
                                name,
                                type_,
                                value: Some(NirValue::UnionExtract { .. }),
                                ..
                            }) = case.body.get(body_index)
                            {
                                let bind = &case.body[body_index..body_index + 1];
                                self.lower_ops(bind)?;
                                if let Some(local) = self.locals.get(name).cloned() {
                                    case_locals.insert(
                                        name.clone(),
                                        LocalValue {
                                            type_: type_.clone(),
                                            stack_offset: local.stack_offset,
                                            constant: local.constant,
                                            by_ref: local.by_ref,
                                        },
                                    );
                                }
                                body_index += 1;
                            }
                            if let Some(guard) = &case.guard {
                                let saved_locals = self.locals.clone();
                                self.locals = case_locals.clone();
                                let guard_value = self.lower_value(guard)?;
                                self.emit(abi::compare_immediate(&guard_value.location, "0"));
                                self.emit(
                                    abi::branch_eq(&next_label).field("reason", "matchGuardFalse"),
                                );
                                self.locals = saved_locals;
                            }
                            self.lower_ops(&case.body[body_index..])?;
                            if !self.current_block_returns() {
                                self.emit(abi::branch(&end_label));
                            }
                            self.restore_local_constants(&constants_before_case);
                            self.emit(abi::label(&next_label));
                        }
                        self.emit(abi::label(&end_label));
                        self.clear_local_constants();
                    }
                    NirOp::While {
                        kind,
                        condition,
                        body,
                    } => {
                        // plan-39 I1 (upper side): a `WHILE local < S` body runs only
                        // when `local < S`, so `local` is strictly below an i64 there
                        // and `local + 1 <= S <= i64::MAX` cannot overflow. Capture
                        // the guarded local from the raw condition before it is
                        // lowered/shadowed, and mark it for the body below.
                        let strict_upper_name = match condition {
                            NirValue::Binary { op, left, .. } if *op == BinaryOp::Less => {
                                match left.as_ref() {
                                    NirValue::Local(n) => Some(n.clone()),
                                    _ => None,
                                }
                            }
                            _ => None,
                        };
                        let promoted = self.begin_loop_promotion(body, None)?;
                        let loop_label = self.label("while_loop");
                        let end_label = self.label("while_end");
                        self.emit(abi::label(&loop_label));
                        // The back-edge jumps to `loop_label` above the condition, so
                        // the condition (and body) is re-tested each iteration from
                        // this one emitted comparison. Constants known at loop entry
                        // (e.g. a `MUT` local's literal initializer) must not fold reads
                        // in the condition or body — they go stale once the body
                        // reassigns them. A string-producing condition like
                        // `WHILE toString(c) <> "3"` would otherwise freeze the fold to
                        // `c`'s entry value and loop forever (bug-57). Mirrors the
                        // `clear_local_constants()` the `DoUntil` path runs before its
                        // body+condition.
                        self.clear_local_constants();
                        let condition = self.lower_loop_condition(condition)?;
                        self.emit_branch_unless(&condition, &end_label, None);
                        self.loop_stack.push(LoopLabels {
                            kind: *kind,
                            name: None,
                            continue_label: loop_label.clone(),
                            exit_label: end_label.clone(),
                            cleanup_depth: self.active_cleanups.len(),
                            temp_depth: self.pending_temp_frees.len(),
                        });
                        if let Some(ref name) = strict_upper_name {
                            self.integer_strict_upper.insert(name.clone());
                        }
                        self.lower_loop_body(body)?;
                        self.loop_stack.pop();
                        self.emit(abi::branch(&loop_label));
                        self.emit(abi::label(&end_label));
                        self.clear_local_constants();
                        self.end_loop_promotion(promoted)?;
                    }
                    NirOp::For {
                        name,
                        type_,
                        start,
                        end,
                        step,
                        body,
                        loc,
                    } => {
                        self.lower_numeric_for(name, type_, start, end, step, body, *loc)?;
                    }
                    NirOp::DoUntil { body, condition } => {
                        let promoted = self.begin_loop_promotion(body, None)?;
                        let loop_label = self.label("do_loop");
                        let condition_label = self.label("do_until");
                        let end_label = self.label("do_end");
                        self.emit(abi::label(&loop_label));
                        // The back-edge jumps to `loop_label` above the body, so
                        // constants known at loop entry (e.g. a `MUT` local's literal
                        // initializer) must not fold reads inside the body — they go
                        // stale once the body reassigns them on later iterations.
                        // Matches the `clear_local_constants()` the `While` path runs
                        // before its body.
                        self.clear_local_constants();
                        self.loop_stack.push(LoopLabels {
                            kind: crate::ast::LoopKind::Do,
                            name: None,
                            continue_label: condition_label.clone(),
                            exit_label: end_label.clone(),
                            cleanup_depth: self.active_cleanups.len(),
                            temp_depth: self.pending_temp_frees.len(),
                        });
                        self.lower_loop_body(body)?;
                        self.loop_stack.pop();
                        self.emit(abi::label(&condition_label));
                        let condition = self.lower_loop_condition(condition)?;
                        self.emit(abi::compare_immediate(&condition, "0"));
                        self.emit(abi::branch_eq(&loop_label));
                        self.emit(abi::label(&end_label));
                        self.clear_local_constants();
                        self.end_loop_promotion(promoted)?;
                    }
                    NirOp::ForEach {
                        name,
                        type_,
                        iterable,
                        body,
                    } => {
                        self.lower_for_each(name, type_, iterable, body)?;
                    }
                    NirOp::Trap { body, .. } => {
                        let (label, trap_name, trap_offset) = self
                            .trap
                            .as_ref()
                            .map(|trap| (trap.label.clone(), trap.name.clone(), trap.stack_offset))
                            .expect("trap op requires trap state");
                        // Re-pin the trap error local to its function-level slot: an
                        // inline `TRAP(e)` in the body reuses the shared name `e` and
                        // leaves `self.locals[e]` pointing at that inline handler's
                        // slot, so the function-level handler would resolve `e` (and
                        // read `e.message`) from the wrong slot — a null `Error`
                        // pointer → segfault (bug-148).
                        self.locals.insert(
                            trap_name,
                            LocalValue {
                                type_: ParameterType::named("Error"),
                                stack_offset: trap_offset,
                                constant: None,
                                by_ref: false,
                            },
                        );
                        self.emit(abi::label(&label));
                        if let Some(trap) = &mut self.trap {
                            trap.in_trap_body = true;
                        }
                        // Free the caught `Error` block on every exit from the
                        // handler (bug-151): the routed error is a fresh arena block
                        // stored into the trap slot, and it was never registered for
                        // scope-drop, so a `TRAP(e)` taken in a loop leaked one block
                        // per catch. Register `e` as the FIRST owned value of the
                        // handler body's own cleanup scope, mirroring `lower_ops`, so
                        // the body's scope-drop frees it exactly once on RETURN / FAIL
                        // / fall-through and NOT on the success path that branches over
                        // the handler (where the slot is never written). Escapes are
                        // already safe: `RETURN e` elides via `plan_returned_move`, and
                        // `FAIL e` deep-copies the error in `store_pending_error_from_value`
                        // before the cleanup frees the original.
                        let handler_scope_start = self.active_cleanups.len();
                        self.cleanup_scope_starts.push(handler_scope_start);
                        self.active_cleanups
                            .push(ActiveCleanup::OwnedValue(OwnedValueCleanup {
                                type_: ParameterType::named("Error"),
                                stack_offset: trap_offset,
                                capacity_slot: None,
                                loop_alias_slot: None,
                                result_wrapper: None,
                            }));
                        self.owned_value_slots.push(trap_offset);
                        let handler_result = self.lower_ops_inner(body, handler_scope_start);
                        self.cleanup_scope_starts.pop();
                        handler_result?;
                        if let Some(trap) = &mut self.trap {
                            trap.in_trap_body = false;
                        }
                    }
                }
                Ok(())
            })();
            crate::codegen::engine::expansion::exit(self.instructions.len());
            self.current_op_key = enclosing_op_key;
            self.resource_temp_exempt = enclosing_temp_exempt;
            result.map_err(|err| format!("{err} while lowering {}", nir_op_context(op)))?;
            // plan-134-F: the drop walker's symmetry probe — emits nothing unless the
            // build's environment names this local (`graph_drop.rs`).
            if let NirOp::Bind { name, .. } | NirOp::Assign { name, .. } = op {
                self.emit_test_graph_drop_hook(name)?;
            }
            // plan-39 I1: after lowering the op, invalidate range facts. A `Bind`/
            // `Assign` drops just the reassigned local's bounds (its RHS, already
            // lowered above, used the valid pre-assignment bound). A loop / Match /
            // Trap body may reassign a guarded local, so any bound it established
            // must not leak past it — drop all. Straight-line ops keep their bounds;
            // the `If` arm has already set its own post-condition bound.
            match op {
                NirOp::Bind { name, .. } | NirOp::Assign { name, .. } => {
                    self.integer_lower_bounds.remove(name);
                    self.integer_strict_upper.remove(name);
                }
                NirOp::While { .. }
                | NirOp::For { .. }
                | NirOp::ForEach { .. }
                | NirOp::DoUntil { .. }
                | NirOp::Match { .. }
                | NirOp::Trap { .. } => {
                    self.integer_lower_bounds.clear();
                    self.integer_strict_upper.clear();
                }
                _ => {}
            }
            // Every statement retires its own temps here. A fall-through statement
            // frees them; a control transfer has already dealt with them according
            // to its `TransferTemps` class, and `forget_pending_temps_to` checks
            // the two classes that claim to have nothing left (bug-567).
            // Rule 4: an assignment whose value can carry a resource the statement
            // owns stores it into a binding that outlives the statement, so every
            // resource the binding now reaches is raised to the binding's scope
            // before the statement's temporaries close what it still owns. Done
            // here, after the store, so it holds for every way an assignment is
            // lowered (a plain store, an in-place append or set), as the
            // module-level store walks its global after a call (bug-806).
            if let NirOp::Assign { name, value } = op {
                self.statement_temps.assign_target = None;
                self.emit_assigned_container_float(name, value)?;
            }
            match Self::transfer_temp_disposition(op) {
                TransferTemps::StatementScope => self.drop_pending_temps_to(temp_watermark)?,
                disposition => self.forget_pending_temps_to(temp_watermark, disposition)?,
            }
            self.reset_temporary_registers();
        }
        let scope_returns = self.current_block_returns();
        // bug-1053: the lists of this scope whose drain takes records, in the order
        // their drains ran; the records are freed after the scope's last cleanup.
        let mut reclaiming = Vec::new();
        while self.active_cleanups.len() > cleanup_scope_start {
            let cleanup = self
                .active_cleanups
                .pop()
                .expect("cleanup scope length already checked");
            if !scope_returns {
                match cleanup {
                    ActiveCleanup::Resource(cleanup) => {
                        self.emit_resource_cleanup_call(&cleanup)?;
                        if cleanup.reclaim_slot.is_some() {
                            reclaiming.push(ActiveCleanup::Resource(cleanup));
                        }
                    }
                    ActiveCleanup::ResourceUnion(cleanup) => {
                        self.emit_resource_union_cleanup_call(&cleanup)?;
                        if cleanup.reclaim_slot.is_some() {
                            reclaiming.push(ActiveCleanup::ResourceUnion(cleanup));
                        }
                    }
                    ActiveCleanup::OwnedList(cleanup) => {
                        self.emit_owned_list_drain(&cleanup)?;
                        if cleanup.reclaim_slot.is_some() {
                            reclaiming.push(ActiveCleanup::OwnedList(cleanup));
                        }
                    }
                    ActiveCleanup::OwnedValue(cleanup) => self.emit_owned_value_drop(&cleanup)?,
                }
            }
        }
        reclaiming.reverse();
        self.emit_owned_list_reclaims(&reclaiming)?;
        Ok(())
    }
}
