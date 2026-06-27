use super::*;

impl CodeBuilder<'_> {
    pub(super) fn lower_ops(&mut self, ops: &[NirOp]) -> Result<(), String> {
        let cleanup_scope_start = self.active_cleanups.len();
        self.cleanup_scope_starts.push(cleanup_scope_start);
        let result = self.lower_ops_inner(ops, cleanup_scope_start);
        self.cleanup_scope_starts.pop();
        result
    }

    fn lower_ops_inner(&mut self, ops: &[NirOp], cleanup_scope_start: usize) -> Result<(), String> {
        for op in ops {
            let result = (|| -> Result<(), String> {
                match op {
                    NirOp::Bind {
                        name, type_, value, ..
                    } => {
                        let stack_offset = self.allocate_stack_object(name, 8);
                        // A non-escaping `MUT` borrow capture: the env slot holds a
                        // pointer to the parent binding's slot, so this binding is a
                        // *reference* local — its slot stores that pointer and reads
                        // /writes deref through it. It is a borrow:
                        // never deep-copied and never freed here (the parent owns
                        // and frees the value).
                        let borrows_capture_slot =
                            matches!(value, Some(NirValue::Capture { by_ref: true, .. }));
                        // A reference local must never carry a folded constant: its
                        // value lives in the parent slot and can change underneath
                        // it, so every read must deref.
                        let constant = if borrows_capture_slot {
                            None
                        } else {
                            value
                                .as_ref()
                                .and_then(|value| self.local_constant_value(value))
                        };
                        self.locals.insert(
                            name.clone(),
                            LocalValue {
                                type_: type_.clone(),
                                stack_offset,
                                constant,
                                by_ref: borrows_capture_slot,
                            },
                        );
                        // A `MATCH` variant binding (`UnionExtract`) is a borrow
                        // into the matched union's inlined variant block: the union
                        // owns the data and frees it as one block on its own drop,
                        // so the binding is neither deep-copied nor freed here.
                        let borrows_union_variant =
                            matches!(value, Some(NirValue::UnionExtract { .. }));
                        // A thread-boundary result (`thread::receive`/`waitFor`/…)
                        // is owned by the thread runtime / worker arena, not this
                        // scope, so it is neither zero-initialized nor freed here.
                        let runtime_managed =
                            value.as_ref().is_some_and(Self::value_is_runtime_managed);
                        // This binding owns a freeable flat block that scope-drop
                        // must free (plan-02 Phase 8). A borrowed capture slot is
                        // not owned here — the parent binding remains the freer.
                        let owns_freeable_value = !borrows_union_variant
                            && !borrows_capture_slot
                            && !runtime_managed
                            && self.is_freeable_flat_value(type_);
                        // Zero the slot before a (possibly fallible) initializer
                        // runs. If the initializer traps before storing, the slot
                        // stays null and the scope-drop free skips it instead of
                        // freeing an uninitialized pointer.
                        if owns_freeable_value {
                            self.emit(abi::move_immediate("x9", "Integer", "0"));
                            self.emit(abi::store_u64("x9", abi::stack_pointer(), stack_offset));
                        }
                        if let Some(value) = value {
                            // Deep-copy aliasing sources so this binding owns an
                            // independent flat block (plan-02 Phase 8); a borrowed
                            // variant binding or borrowed capture slot aliases its
                            // source deliberately and is stored without copying.
                            let result = if borrows_union_variant || borrows_capture_slot {
                                self.lower_value(value)?
                            } else {
                                self.lower_value_owned(value)?
                            };
                            self.emit(abi::store_u64(
                                &result.location,
                                abi::stack_pointer(),
                                stack_offset,
                            ));
                        } else {
                            let result = self.lower_default_value(type_)?;
                            // The default empty `String` is static rodata; copy it
                            // into the arena so this binding owns an arena block its
                            // scope-drop free can reclaim (collections/records
                            // default to arena allocations already).
                            let location = if result.type_ == "String" {
                                self.copy_flat_block("String", &result.location)?
                            } else {
                                result.location
                            };
                            self.emit(abi::store_u64(
                                &location,
                                abi::stack_pointer(),
                                stack_offset,
                            ));
                        }
                        // A (re)bind installs a fresh tight buffer; clear any stale
                        // self-append capacity headroom recorded for this name.
                        self.reset_string_capacity_shadow(name);
                        // A collection that owns resources floated up from inner
                        // blocks (§15.6) gets a runtime owned-list anchored at
                        // this scope; it is drained on every exit path.
                        if self.owner_collections.contains(name) {
                            self.setup_owned_list(name, type_)?;
                        } else if Self::is_res_marked_resource_collection(type_)
                            && matches!(
                                value,
                                Some(NirValue::Call { .. } | NirValue::CallResult { .. })
                            )
                        {
                            // A `List OF RES File` bound from a call adopts the
                            // resources transferred out by the callee: this scope
                            // owns them and closes each once at exit (§15.6).
                            self.setup_owned_list(name, type_)?;
                            if let Some(element_type) = super::list_element_type(type_) {
                                self.emit_owned_list_seed_from_collection(
                                    name,
                                    stack_offset,
                                    &element_type,
                                )?;
                            }
                        }
                        // Where this binding's close obligation lives (§15.6).
                        let resource_owner = self
                            .resource_owners
                            .get(name)
                            .cloned()
                            .unwrap_or(crate::escape::ResOwner::Local);
                        if Self::is_thread_type(type_) {
                            self.active_cleanups
                                .push(ActiveCleanup::Thread(ThreadCleanup {
                                    name: name.clone(),
                                    symbol: Self::thread_drop_symbol(),
                                }));
                        } else if borrows_union_variant || borrows_capture_slot {
                            // Borrowed — no cleanup (the parent binding frees it).
                        } else if let crate::escape::ResOwner::Float(collection) = &resource_owner {
                            // Ownership floated to an outer collection's scope:
                            // register the record in that owned-list. This binding
                            // is now a borrow and registers no static cleanup.
                            let collection = collection.clone();
                            self.emit_owned_list_push(&collection, stack_offset)?;
                        } else if let Some(symbol) = self.resource_cleanup_symbol(type_) {
                            self.active_cleanups
                                .push(ActiveCleanup::Resource(ResourceCleanup {
                                    name: name.clone(),
                                    symbol,
                                }));
                        } else if let Some(variants) = self.resource_union_cleanup(type_) {
                            // A resource union drops by dispatching on its tag to
                            // the active variant's registered close op.
                            self.active_cleanups.push(ActiveCleanup::ResourceUnion(
                                ResourceUnionCleanup {
                                    name: name.clone(),
                                    variants,
                                },
                            ));
                        } else if owns_freeable_value {
                            // An owned, non-escaping flat value (plan-01 Phase 5 /
                            // plan-02 Phase 8): a single `arena_free` of its block
                            // reclaims everything at scope-drop. Copy-insertion
                            // (`lower_value_owned`) guarantees this block is
                            // unaliased, so the free is sound and once-only.
                            self.active_cleanups.push(ActiveCleanup::OwnedValue(
                                OwnedValueCleanup {
                                    type_: type_.clone(),
                                    stack_offset,
                                },
                            ));
                            self.owned_value_slots.push(stack_offset);
                        }
                        // Default-initialize a `RES` binding's `STATE` payload.
                        // The owning binding allocates the state record on first
                        // bind; a moved/returned resource that already carries a
                        // state keeps it (the slot is non-null).
                        if let Some(state_type) = crate::builtins::resource::state_type_name(type_)
                        {
                            let state_type = state_type.to_string();
                            self.emit_resource_state_init(stack_offset, &state_type)?;
                        }
                    }
                    NirOp::StoreGlobal { name, type_, value } => {
                        let global = self.global_value(name)?;
                        let value_type = if type_.is_empty() {
                            global.type_.clone()
                        } else {
                            type_.clone()
                        };
                        // A global outlives every scope, so it must own its value
                        // independently: deep-copy an aliasing source so freeing a
                        // local never dangles the global (plan-02 Phase 8).
                        let result = if let Some(value) = value {
                            self.lower_value_owned(value)?
                        } else {
                            self.lower_default_value(&value_type)?
                        };
                        let address = self.load_global_address(name)?;
                        self.emit(abi::store_u64(&result.location, &address, 0));
                    }
                    NirOp::Assign { name, value } => {
                        let (stack_offset, by_ref) = {
                            let local = self.locals.get(name).ok_or_else(|| {
                                format!("native code assignment unknown local '{name}'")
                            })?;
                            (local.stack_offset, local.by_ref)
                        };
                        // `name = collections::append(name, item)` on a uniquely
                        // owned `MUT` list mutates the live buffer in place
                        // (plan-01 §4.2): the helper updates the slot, so skip the
                        // general reassignment path entirely.
                        if !self.try_inplace_append_assign(name, value, stack_offset, by_ref)?
                            && !self.try_inplace_set_assign(name, value, stack_offset, by_ref)?
                            && !self.try_inplace_concat_assign(name, value, stack_offset, by_ref)?
                        {
                            // Reassignment installs a fresh independent block; the old
                            // block remains owned by this binding's scope-drop free
                            // (the slot is overwritten with the new owner). Deep-copy
                            // an aliasing source so the binding stays unaliased.
                            let result = self.lower_value_owned(value)?;
                            let assign_slot = if Self::is_thread_type(&result.type_) {
                                let slot = self.allocate_stack_object("thread_assign_value", 8);
                                self.emit(abi::store_u64(
                                    &result.location,
                                    abi::stack_pointer(),
                                    slot,
                                ));
                                self.emit_thread_cleanup_for_name(name)?;
                                Some(slot)
                            } else if let Some(symbol) = self.resource_cleanup_symbol(&result.type_)
                            {
                                let slot = self.allocate_stack_object("resource_assign_value", 8);
                                self.emit(abi::store_u64(
                                    &result.location,
                                    abi::stack_pointer(),
                                    slot,
                                ));
                                let cleanup = ResourceCleanup {
                                    name: name.clone(),
                                    symbol,
                                };
                                self.emit_resource_cleanup_call(&cleanup)?;
                                Some(slot)
                            } else {
                                None
                            };
                            let result_location = if let Some(slot) = assign_slot {
                                let register = self.allocate_register()?;
                                self.emit(abi::load_u64(&register, abi::stack_pointer(), slot));
                                register
                            } else {
                                result.location.clone()
                            };
                            if by_ref {
                                // A reference local (non-escaping `MUT` borrow): write
                                // through the slot pointer so the live parent binding is
                                // updated, not a local copy.
                                let slot_pointer = self.allocate_register()?;
                                self.emit(abi::load_u64(
                                    &slot_pointer,
                                    abi::stack_pointer(),
                                    stack_offset,
                                ));
                                self.emit(abi::store_u64(&result_location, &slot_pointer, 0));
                            } else {
                                self.emit(abi::store_u64(
                                    &result_location,
                                    abi::stack_pointer(),
                                    stack_offset,
                                ));
                            }
                            // A reference local never folds to a constant (see Bind).
                            let constant = if by_ref {
                                None
                            } else {
                                self.local_constant_value(value)
                            };
                            if let Some(local) = self.locals.get_mut(name) {
                                local.constant = constant;
                            }
                            // A non-self-append reassignment installs a fresh tight
                            // buffer; clear any stale self-append capacity headroom.
                            if !by_ref {
                                self.reset_string_capacity_shadow(name);
                            }
                        }
                    }
                    NirOp::StateAssign { resource, value } => {
                        // Replace the resource's `STATE` payload: store the new
                        // record pointer into the resource record's state slot.
                        // The resource value is itself a pointer, so the update
                        // is visible to the owner and any borrower.
                        let stack_offset = self
                            .locals
                            .get(resource)
                            .ok_or_else(|| {
                                format!("native code state assignment unknown local '{resource}'")
                            })?
                            .stack_offset;
                        let result = self.lower_value(value)?;
                        let value_slot = self.allocate_stack_object("state_assign_value", 8);
                        self.emit(abi::store_u64(
                            &result.location,
                            abi::stack_pointer(),
                            value_slot,
                        ));
                        let ptr = self.allocate_register()?;
                        self.emit(abi::load_u64(&ptr, abi::stack_pointer(), stack_offset));
                        let val = self.allocate_register()?;
                        self.emit(abi::load_u64(&val, abi::stack_pointer(), value_slot));
                        self.emit(abi::store_u64(&val, &ptr, FILE_OFFSET_STATE));
                    }
                    NirOp::Eval { value } => {
                        self.lower_value(value)?;
                    }
                    NirOp::Return { value } => {
                        self.emit_return_exit(value.as_ref())?;
                    }
                    NirOp::ExitLoop { kind } => {
                        let target = self
                            .loop_stack
                            .iter()
                            .rev()
                            .find(|labels| labels.kind == *kind)
                            .cloned()
                            .ok_or_else(|| "native code EXIT has no matching loop".to_string())?;
                        self.emit_cleanup_branch_to_depth(
                            &target.exit_label,
                            target.cleanup_depth,
                        )?;
                    }
                    NirOp::ContinueLoop { kind } => {
                        let target = self
                            .loop_stack
                            .iter()
                            .rev()
                            .find(|labels| labels.kind == *kind)
                            .cloned()
                            .ok_or_else(|| {
                                "native code CONTINUE has no matching loop".to_string()
                            })?;
                        self.emit_cleanup_branch_to_depth(
                            &target.continue_label,
                            target.cleanup_depth,
                        )?;
                    }
                    NirOp::ExitProgram { code } => {
                        self.emit_program_exit_value(code)?;
                    }
                    NirOp::Fail { error } => {
                        self.emit_error_value_exit(error, self.error_exit_destination())?;
                    }
                    NirOp::If {
                        condition,
                        then_body,
                        else_body,
                    } => {
                        let condition = self.lower_value(condition)?;
                        let else_label = self.label("if_else");
                        let end_label = self.label("if_end");
                        let constants_before_if = self.local_constants();
                        self.emit(abi::compare_immediate(&condition.location, "0"));
                        self.emit(abi::branch_eq(&else_label).field("reason", "ifFalse"));
                        self.lower_ops(then_body)?;
                        if !self.current_block_returns() {
                            self.emit(abi::branch(&end_label));
                        }
                        self.emit(abi::label(&else_label));
                        self.restore_local_constants(&constants_before_if);
                        self.lower_ops(else_body)?;
                        self.emit(abi::label(&end_label));
                        self.clear_local_constants();
                    }
                    NirOp::Match { value, cases } => {
                        let matched = self.lower_value(value)?;
                        let matched_slot = self.allocate_stack_object("match_value", 8);
                        self.emit(abi::store_u64(
                            &matched.location,
                            abi::stack_pointer(),
                            matched_slot,
                        ));
                        let end_label = self.label("match_end");
                        for case in cases {
                            let matched_register = self.allocate_register()?;
                            self.emit(abi::load_u64(
                                &matched_register,
                                abi::stack_pointer(),
                                matched_slot,
                            ));
                            let case_matched = ValueResult {
                                type_: matched.type_.clone(),
                                location: matched_register,
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
                        let loop_label = self.label("while_loop");
                        let end_label = self.label("while_end");
                        self.emit(abi::label(&loop_label));
                        let condition = self.lower_value(condition)?;
                        self.emit(abi::compare_immediate(&condition.location, "0"));
                        self.emit(abi::branch_eq(&end_label));
                        self.clear_local_constants();
                        self.loop_stack.push(LoopLabels {
                            kind: *kind,
                            continue_label: loop_label.clone(),
                            exit_label: end_label.clone(),
                            cleanup_depth: self.active_cleanups.len(),
                        });
                        self.lower_ops(body)?;
                        self.loop_stack.pop();
                        self.emit(abi::branch(&loop_label));
                        self.emit(abi::label(&end_label));
                        self.clear_local_constants();
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
                            continue_label: condition_label.clone(),
                            exit_label: end_label.clone(),
                            cleanup_depth: self.active_cleanups.len(),
                        });
                        self.lower_ops(body)?;
                        self.loop_stack.pop();
                        self.emit(abi::label(&condition_label));
                        let condition = self.lower_value(condition)?;
                        self.emit(abi::compare_immediate(&condition.location, "0"));
                        self.emit(abi::branch_eq(&loop_label));
                        self.emit(abi::label(&end_label));
                        self.clear_local_constants();
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
                        let label = self
                            .trap
                            .as_ref()
                            .map(|trap| trap.label.clone())
                            .expect("trap op requires trap state");
                        self.emit(abi::label(&label));
                        if let Some(trap) = &mut self.trap {
                            trap.in_trap_body = true;
                        }
                        self.lower_ops(body)?;
                        if let Some(trap) = &mut self.trap {
                            trap.in_trap_body = false;
                        }
                    }
                }
                Ok(())
            })();
            result.map_err(|err| format!("{err} while lowering {}", nir_op_context(op)))?;
            self.reset_temporary_registers();
        }
        let scope_returns = self.current_block_returns();
        while self.active_cleanups.len() > cleanup_scope_start {
            let cleanup = self
                .active_cleanups
                .pop()
                .expect("cleanup scope length already checked");
            if !scope_returns {
                match cleanup {
                    ActiveCleanup::Thread(cleanup) => self.emit_thread_cleanup_call(&cleanup)?,
                    ActiveCleanup::Resource(cleanup) => {
                        self.emit_resource_cleanup_call(&cleanup)?
                    }
                    ActiveCleanup::ResourceUnion(cleanup) => {
                        self.emit_resource_union_cleanup_call(&cleanup)?
                    }
                    ActiveCleanup::OwnedList(cleanup) => self.emit_owned_list_drain(&cleanup)?,
                    ActiveCleanup::OwnedValue(cleanup) => self.emit_owned_value_drop(&cleanup)?,
                }
            }
        }
        Ok(())
    }

    pub(super) fn lower_numeric_for(
        &mut self,
        name: &str,
        type_: &str,
        start: &NirValue,
        end: &NirValue,
        step: &NirValue,
        body: &[NirOp],
        loc: NirSourceLoc,
    ) -> Result<(), String> {
        let local_slot = self.allocate_stack_object(name, 8);
        let start_value = self.lower_value(start)?;
        self.emit(abi::store_u64(
            &start_value.location,
            abi::stack_pointer(),
            local_slot,
        ));
        let previous = self.locals.insert(
            name.to_string(),
            LocalValue {
                type_: type_.to_string(),
                stack_offset: local_slot,
                constant: None,
                by_ref: false,
            },
        );

        let loop_label = self.label("for_loop");
        let continue_label = self.label("for_continue");
        let end_label = self.label("for_end");
        self.emit(abi::label(&loop_label));
        let iter = NirValue::Local(name.to_string());
        let zero = NirValue::Const {
            type_: type_.to_string(),
            value: "0".to_string(),
        };
        // The loop bound comparisons are infallible (comparisons never overflow),
        // so a default source location is correct here; only the increment below
        // can originate an overflow error and it carries the loop's location.
        let cmp = NirSourceLoc::default();
        let condition = NirValue::Binary {
            op: "OR".to_string(),
            left: Box::new(NirValue::Binary {
                op: "AND".to_string(),
                left: Box::new(NirValue::Binary {
                    op: ">=".to_string(),
                    left: Box::new(step.clone()),
                    right: Box::new(zero.clone()),
                    loc: cmp,
                }),
                right: Box::new(NirValue::Binary {
                    op: "<=".to_string(),
                    left: Box::new(iter.clone()),
                    right: Box::new(end.clone()),
                    loc: cmp,
                }),
                loc: cmp,
            }),
            right: Box::new(NirValue::Binary {
                op: "AND".to_string(),
                left: Box::new(NirValue::Binary {
                    op: "<".to_string(),
                    left: Box::new(step.clone()),
                    right: Box::new(zero),
                    loc: cmp,
                }),
                right: Box::new(NirValue::Binary {
                    op: ">=".to_string(),
                    left: Box::new(iter.clone()),
                    right: Box::new(end.clone()),
                    loc: cmp,
                }),
                loc: cmp,
            }),
            loc: cmp,
        };
        let condition = self.lower_value(&condition)?;
        self.emit(abi::compare_immediate(&condition.location, "0"));
        self.emit(abi::branch_eq(&end_label));
        self.clear_local_constants();
        self.loop_stack.push(LoopLabels {
            kind: crate::ast::LoopKind::For,
            continue_label: continue_label.clone(),
            exit_label: end_label.clone(),
            cleanup_depth: self.active_cleanups.len(),
        });
        self.lower_ops(body)?;
        self.loop_stack.pop();
        self.emit(abi::label(&continue_label));
        let increment = NirValue::Binary {
            op: "+".to_string(),
            left: Box::new(iter),
            right: Box::new(step.clone()),
            loc,
        };
        let increment = self.lower_value(&increment)?;
        self.emit(abi::store_u64(
            &increment.location,
            abi::stack_pointer(),
            local_slot,
        ));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&end_label));
        if let Some(previous) = previous {
            self.locals.insert(name.to_string(), previous);
        } else {
            self.locals.remove(name);
        }
        self.clear_local_constants();
        Ok(())
    }

    /// Recognize `name = collections::append(name, item)` for a single element
    /// appended to a uniquely-owned `MUT` list local, and lower it as an in-place
    /// grow (plan-01 §4.2). Returns `true` when handled (the local's slot was
    /// updated in place); `false` to fall back to the general reassignment path.
    ///
    /// Soundness: under MFBASIC value semantics every binding owns its buffer and
    /// copy-insertion deep-copies any aliasing assignment, so the local's buffer
    /// has no live alias. `FOR EACH` snapshots the buffer pointer and count at
    /// loop entry, and in-place append only writes *beyond* that snapshot count
    /// without moving existing entries or payloads, so iteration is unaffected.
    /// Reference (`by_ref`) locals are excluded — their slot holds a pointer to
    /// the parent slot, not the buffer — and bulk `append(list, otherList)` is
    /// excluded (the item must be a single element).
    fn try_inplace_append_assign(
        &mut self,
        name: &str,
        value: &NirValue,
        stack_offset: usize,
        by_ref: bool,
    ) -> Result<bool, String> {
        if by_ref {
            return Ok(false);
        }
        let NirValue::Call { target, args, .. } = value else {
            return Ok(false);
        };
        if crate::builtins::native_builtin_target(target) != Some("append") || args.len() != 2 {
            return Ok(false);
        }
        let NirValue::Local(arg0) = &args[0] else {
            return Ok(false);
        };
        if arg0 != name {
            return Ok(false);
        }
        let Some(local) = self.locals.get(name) else {
            return Ok(false);
        };
        let list_type = local.type_.clone();
        let Some(element_type) = super::list_element_type(&list_type) else {
            return Ok(false);
        };
        if super::CollectionTypeLayout::from_type(&list_type).is_none() {
            return Ok(false);
        }
        // Commit only for a statically-known single element of the list's element
        // type. A bulk `append(list, otherList)` has item type == list_type and
        // falls through to the general (concatenating) path.
        match self.static_type_name(&args[1]) {
            Some(item_type) if item_type == element_type => {}
            _ => return Ok(false),
        }
        let item = self.lower_value(&args[1])?;
        let item_slot = self.allocate_stack_object("inplace_append_item", 8);
        self.emit(abi::store_u64(
            &item.location,
            abi::stack_pointer(),
            item_slot,
        ));
        self.lower_list_append_in_place(stack_offset, item_slot, &list_type, &element_type)?;
        if let Some(local) = self.locals.get_mut(name) {
            local.constant = None;
        }
        Ok(true)
    }

    /// Recognize `name = collections::set(name, index, item)` on a uniquely-owned
    /// `MUT` **list** local and lower it as an in-place overwrite (plan-02 §4.1).
    /// When the replacement payload fits the target slot (`newLen <= oldLen`, the
    /// fixed-width and same-size record cases always do) the value bytes are
    /// overwritten at the entry's `valueOffset` and `valueLength` patched — no
    /// allocation, no copy. Otherwise it falls back to the rebuild (remove+insert)
    /// path, which is always correct (D1). Returns `true` when handled.
    ///
    /// Soundness mirrors `try_inplace_append_assign`: value semantics + copy
    /// insertion guarantee the buffer is unaliased, and `by_ref` locals are
    /// excluded. Unlike append, an overwrite is observable to an enclosing
    /// `FOR EACH` over the same binding, so that case is excluded
    /// (`for_each_iterable_locals`). The map overload stays on the rebuild path
    /// until Phase 3.
    fn try_inplace_set_assign(
        &mut self,
        name: &str,
        value: &NirValue,
        stack_offset: usize,
        by_ref: bool,
    ) -> Result<bool, String> {
        if by_ref {
            return Ok(false);
        }
        let NirValue::Call { target, args, .. } = value else {
            return Ok(false);
        };
        if crate::builtins::native_builtin_target(target) != Some("set") || args.len() != 3 {
            return Ok(false);
        }
        let NirValue::Local(arg0) = &args[0] else {
            return Ok(false);
        };
        if arg0 != name {
            return Ok(false);
        }
        if self.for_each_iterable_locals.iter().any(|n| n == name) {
            return Ok(false);
        }
        let Some(local) = self.locals.get(name) else {
            return Ok(false);
        };
        let collection_type = local.type_.clone();
        // Phase 1 is the LIST overload only; the map overload (Phase 3) falls
        // through to the rebuild path.
        let Some(element_type) = super::list_element_type(&collection_type) else {
            return Ok(false);
        };
        if super::CollectionTypeLayout::from_type(&collection_type).is_none() {
            return Ok(false);
        }
        // The replacement must be a single element of the list element type.
        match self.static_type_name(&args[2]) {
            Some(item_type) if item_type == element_type => {}
            _ => return Ok(false),
        }
        let index = self.lower_value(&args[1])?;
        if index.type_ != "Integer" {
            return Err(format!(
                "native collection set list index must be Integer, got {}",
                index.type_
            ));
        }
        let index_slot = self.allocate_stack_object("inplace_set_index", 8);
        self.emit(abi::store_u64(
            &index.location,
            abi::stack_pointer(),
            index_slot,
        ));
        let item = self.lower_value(&args[2])?;
        if item.type_ != element_type {
            return Err(format!(
                "native collection set list item must be {element_type}, got {}",
                item.type_
            ));
        }
        let item_slot = self.allocate_stack_object("inplace_set_item", 8);
        self.emit(abi::store_u64(
            &item.location,
            abi::stack_pointer(),
            item_slot,
        ));
        self.lower_list_set_in_place(
            stack_offset,
            index_slot,
            item_slot,
            &collection_type,
            &element_type,
        )?;
        if let Some(local) = self.locals.get_mut(name) {
            local.constant = None;
        }
        Ok(true)
    }

    /// Recognize `name = name & x` (and the left-associated chain
    /// `name = name & a & b …`) on a uniquely-owned `MUT` `String` local and lower
    /// it as an in-place self-append (plan-02 §4.1, the string sibling of
    /// `try_inplace_append_assign`). The grown buffer carries geometric capacity
    /// headroom tracked in a frame-local shadow slot, so each append writes the
    /// operand's bytes into the spare tail and bumps the length — amortized O(1) —
    /// instead of `lower_string_concat` allocating a fresh tight buffer every time.
    /// The shadow never escapes: any copy/return/transfer reads only `len` bytes,
    /// freezing the value to the canonical tight `[len][bytes][NUL]` form (D9). A
    /// `String` can never be a `FOR EACH` iterable, so this needs no iterator gate.
    /// Returns `true` when handled.
    fn try_inplace_concat_assign(
        &mut self,
        name: &str,
        value: &NirValue,
        stack_offset: usize,
        by_ref: bool,
    ) -> Result<bool, String> {
        if by_ref {
            return Ok(false);
        }
        // Only fire for a name we pre-allocated a capacity shadow for (a self-append
        // target discovered by the prescan); the shadow is reset on every other
        // bind/assign so it always reflects the live buffer's spare bytes.
        let Some(&shadow_slot) = self.string_capacity_slots.get(name) else {
            return Ok(false);
        };
        let Some(operands) = string_self_append_operands(value, name) else {
            return Ok(false);
        };
        for operand in operands {
            self.lower_string_self_append_one(stack_offset, shadow_slot, operand)?;
        }
        if let Some(local) = self.locals.get_mut(name) {
            local.constant = None;
        }
        Ok(true)
    }

    /// Append one `String` operand's bytes to the grown self-append buffer whose
    /// pointer lives at `name_slot`, using/maintaining the spare-capacity shadow at
    /// `shadow_slot`. Writes into the spare tail when `rlen <= spare`; otherwise
    /// allocates a geometric-headroom buffer, copies the current bytes + the
    /// operand, and repoints `name_slot`. Mirrors `lower_list_append_in_place`.
    fn lower_string_self_append_one(
        &mut self,
        name_slot: usize,
        shadow_slot: usize,
        operand: &NirValue,
    ) -> Result<(), String> {
        let right = self.lower_value(operand)?;
        if right.type_ != "String" {
            return Err(format!(
                "native string self-append operand must be String, got {}",
                right.type_
            ));
        }
        let right_slot = self.allocate_stack_object("concat_self_right", 8);
        self.emit(abi::store_u64(
            &right.location,
            abi::stack_pointer(),
            right_slot,
        ));
        let newlen_slot = self.allocate_stack_object("concat_self_newlen", 8);
        let newcap_slot = self.allocate_stack_object("concat_self_newcap", 8);
        let newbuf_slot = self.allocate_stack_object("concat_self_newbuf", 8);

        let regrow = self.label("concat_self_regrow");
        let write = self.label("concat_self_write");
        let alloc_ok = self.label("concat_self_alloc_ok");
        let cap_keep = self.label("concat_self_cap_keep");
        let done = self.label("concat_self_done");

        // newlen = len + rlen; decide in-place vs regrow on rlen vs spare.
        self.emit(abi::load_u64("x8", abi::stack_pointer(), name_slot));
        self.emit(abi::load_u64("x9", "x8", 0)); // len
        self.emit(abi::load_u64("x10", abi::stack_pointer(), right_slot));
        self.emit(abi::load_u64("x11", "x10", 0)); // rlen
        self.emit(abi::add_registers("x12", "x9", "x11"));
        self.emit(abi::store_u64("x12", abi::stack_pointer(), newlen_slot));
        self.emit(abi::load_u64("x13", abi::stack_pointer(), shadow_slot)); // spare
        self.emit(abi::compare_registers("x11", "x13"));
        self.emit(abi::branch_hi(&regrow)); // rlen > spare → regrow
        self.emit(abi::branch(&write));

        // --- Regrow: alloc newcap_payload + 9; copy old + operand; install. ---
        self.emit(abi::label(&regrow));
        self.emit(abi::load_u64("x8", abi::stack_pointer(), name_slot));
        self.emit(abi::load_u64("x9", "x8", 0)); // len
        self.emit(abi::load_u64("x13", abi::stack_pointer(), shadow_slot)); // spare
        self.emit(abi::add_registers("x10", "x9", "x13")); // current payload capacity
        self.emit_geometric_step(
            "x10",
            "x14",
            "x15",
            COLLECTION_GROW_DATA_INIT,
            COLLECTION_GROW_DATA_TAPER,
            "concat_self_step",
        );
        // newcap_payload = max(step, newlen).
        self.emit(abi::load_u64("x12", abi::stack_pointer(), newlen_slot));
        self.emit(abi::compare_registers("x14", "x12"));
        self.emit(abi::branch_hi(&cap_keep));
        self.emit(abi::branch_eq(&cap_keep));
        self.emit(abi::move_register("x14", "x12"));
        self.emit(abi::label(&cap_keep));
        self.emit(abi::store_u64("x14", abi::stack_pointer(), newcap_slot));
        // alloc size = 8 (len word) + newcap_payload + 1 (NUL).
        self.emit(abi::add_immediate(abi::return_register(), "x14", 9));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64("x1", abi::stack_pointer(), newbuf_slot));
        // newbuf[0] = newlen.
        self.emit(abi::load_u64("x12", abi::stack_pointer(), newlen_slot));
        self.emit(abi::store_u64("x12", "x1", 0));
        // Copy the current bytes (len) to newbuf+8.
        self.emit(abi::load_u64("x8", abi::stack_pointer(), name_slot));
        self.emit(abi::load_u64("x9", "x8", 0)); // len
        self.emit(abi::add_immediate("x8", "x8", 8)); // old data
        self.emit(abi::add_immediate("x17", "x1", 8)); // new data
        self.emit_copy_bytes("x17", "x8", "x9", "concat_self_old");
        // Copy the operand bytes (rlen) to newbuf+8+len. x17 now points at +8+len.
        self.emit(abi::load_u64("x10", abi::stack_pointer(), right_slot));
        self.emit(abi::load_u64("x11", "x10", 0)); // rlen
        self.emit(abi::add_immediate("x10", "x10", 8)); // operand data
        self.emit_copy_bytes("x17", "x10", "x11", "concat_self_new");
        // NUL terminator at newbuf+8+newlen.
        self.emit(abi::move_immediate("x16", "Integer", "0"));
        self.emit(abi::store_u8("x16", "x17", 0));
        // Install new buffer; spare = newcap_payload - newlen.
        self.emit(abi::load_u64("x1", abi::stack_pointer(), newbuf_slot));
        self.emit(abi::store_u64("x1", abi::stack_pointer(), name_slot));
        self.emit(abi::load_u64("x14", abi::stack_pointer(), newcap_slot));
        self.emit(abi::load_u64("x12", abi::stack_pointer(), newlen_slot));
        self.emit(abi::subtract_registers("x14", "x14", "x12"));
        self.emit(abi::store_u64("x14", abi::stack_pointer(), shadow_slot));
        self.emit(abi::branch(&done));

        // --- In place: write operand bytes into the spare tail. ---
        self.emit(abi::label(&write));
        self.emit(abi::load_u64("x8", abi::stack_pointer(), name_slot));
        self.emit(abi::load_u64("x9", "x8", 0)); // len
        self.emit(abi::add_immediate("x17", "x8", 8));
        self.emit(abi::add_registers("x17", "x17", "x9")); // dst = ptr+8+len
        self.emit(abi::load_u64("x10", abi::stack_pointer(), right_slot));
        self.emit(abi::load_u64("x11", "x10", 0)); // rlen
        self.emit(abi::add_immediate("x10", "x10", 8)); // operand data
        self.emit_copy_bytes("x17", "x10", "x11", "concat_self_inplace");
        // NUL after the new end; ptr[0] = newlen; spare -= rlen.
        self.emit(abi::move_immediate("x16", "Integer", "0"));
        self.emit(abi::store_u8("x16", "x17", 0));
        self.emit(abi::load_u64("x8", abi::stack_pointer(), name_slot));
        self.emit(abi::load_u64("x12", abi::stack_pointer(), newlen_slot));
        self.emit(abi::store_u64("x12", "x8", 0));
        self.emit(abi::load_u64("x13", abi::stack_pointer(), shadow_slot));
        self.emit(abi::load_u64("x10", abi::stack_pointer(), right_slot));
        self.emit(abi::load_u64("x11", "x10", 0)); // rlen
        self.emit(abi::subtract_registers("x13", "x13", "x11"));
        self.emit(abi::store_u64("x13", abi::stack_pointer(), shadow_slot));
        self.emit(abi::label(&done));
        Ok(())
    }

    /// Reset a `String` local's capacity shadow to 0 ("tight, no spare") after any
    /// non-self-append bind/assign installs a fresh tight buffer. Keeps the shadow
    /// from claiming spare that the new buffer does not have (plan-02 §4.1).
    pub(super) fn reset_string_capacity_shadow(&mut self, name: &str) {
        if let Some(&slot) = self.string_capacity_slots.get(name) {
            self.emit(abi::move_immediate("x9", "Integer", "0"));
            self.emit(abi::store_u64("x9", abi::stack_pointer(), slot));
        }
    }

    /// Pre-allocate a capacity shadow slot for every `String` local targeted by an
    /// in-place self-append (`name = name & …`) anywhere in `ops`, recursing into
    /// nested blocks. Done before lowering so bind/assign sites can reset the shadow
    /// and the prologue can zero it (plan-02 §4.1).
    pub(super) fn prescan_string_self_appends(&mut self, ops: &[NirOp]) {
        for op in ops {
            match op {
                NirOp::Assign { name, value } => {
                    if string_self_append_operands(value, name).is_some()
                        && !self.string_capacity_slots.contains_key(name)
                    {
                        let slot = self.allocate_stack_object(&format!("strcap_{name}"), 8);
                        self.string_capacity_slots.insert(name.clone(), slot);
                    }
                }
                NirOp::If {
                    then_body,
                    else_body,
                    ..
                } => {
                    self.prescan_string_self_appends(then_body);
                    self.prescan_string_self_appends(else_body);
                }
                NirOp::Match { cases, .. } => {
                    for case in cases {
                        self.prescan_string_self_appends(&case.body);
                    }
                }
                NirOp::While { body, .. }
                | NirOp::For { body, .. }
                | NirOp::DoUntil { body, .. }
                | NirOp::ForEach { body, .. }
                | NirOp::Trap { body, .. } => {
                    self.prescan_string_self_appends(body);
                }
                _ => {}
            }
        }
    }

    pub(super) fn lower_for_each(
        &mut self,
        name: &str,
        type_: &str,
        iterable: &NirValue,
        body: &[NirOp],
    ) -> Result<(), String> {
        let iterable_value = self.lower_value(iterable)?;
        if !is_collection_type(&iterable_value.type_) {
            return Err(format!(
                "native code FOR EACH target '{}' is not a collection",
                iterable_value.type_
            ));
        }
        let map_entry_types = if iterable_value.type_.starts_with("Map OF ") {
            Some(map_type_parts(&iterable_value.type_).ok_or_else(|| {
                format!(
                    "native code FOR EACH target '{}' is not a valid map type",
                    iterable_value.type_
                )
            })?)
        } else {
            None
        };
        let list_element_type = super::list_element_type(&iterable_value.type_);
        let item_value_type = list_element_type.as_deref();
        let collection_slot = self.allocate_stack_object("for_each_collection", 8);
        let cursor_slot = self.allocate_stack_object("for_each_cursor", 8);
        let remaining_slot = self.allocate_stack_object("for_each_remaining", 8);
        let local_slot = self.allocate_stack_object(name, 8);
        let entry_payload_slot = if map_entry_types.is_some() {
            Some(self.allocate_stack_object("for_each_map_entry", 16))
        } else {
            None
        };

        self.emit(abi::store_u64(
            &iterable_value.location,
            abi::stack_pointer(),
            collection_slot,
        ));
        // When the iterable is a plain local, its live buffer is the one this loop
        // snapshots and re-reads each step; record it so an in-place `set`/`prepend`
        // that overwrites an existing entry (observable to this iterator) is
        // excluded for the binding inside the loop body (plan-02 §4.1, D1).
        let pushed_iterable = if let NirValue::Local(local_name) = iterable {
            self.for_each_iterable_locals.push(local_name.clone());
            true
        } else {
            false
        };
        self.emit(abi::load_u64("x8", abi::stack_pointer(), collection_slot));
        self.emit(abi::load_u64("x9", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::add_immediate("x10", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::store_u64("x10", abi::stack_pointer(), cursor_slot));
        self.emit(abi::store_u64("x9", abi::stack_pointer(), remaining_slot));

        let loop_label = self.label("for_each_loop");
        let end_label = self.label("for_each_end");
        self.emit(abi::label(&loop_label));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), remaining_slot));
        self.emit(abi::compare_immediate("x9", "0"));
        self.emit(abi::branch_eq(&end_label));
        self.emit(abi::load_u64("x10", abi::stack_pointer(), cursor_slot));
        if let (Some(entry_payload_slot), Some((key_type, value_type))) =
            (entry_payload_slot, map_entry_types.as_ref())
        {
            self.emit(abi::load_u64(
                "x11",
                "x10",
                COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
            ));
            self.emit(abi::load_u64(
                "x12",
                "x10",
                COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
            ));
            self.emit(abi::load_u64("x8", abi::stack_pointer(), collection_slot));
            let key_value = self.emit_load_collection_payload(key_type, "x8", "x11", "x12")?;
            self.emit(abi::store_u64(
                &key_value,
                abi::stack_pointer(),
                entry_payload_slot,
            ));
            self.emit(abi::load_u64("x10", abi::stack_pointer(), cursor_slot));
            self.emit(abi::load_u64(
                "x11",
                "x10",
                COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
            ));
            self.emit(abi::load_u64(
                "x12",
                "x10",
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ));
            self.emit(abi::load_u64("x8", abi::stack_pointer(), collection_slot));
            let item_value = self.emit_load_collection_payload(value_type, "x8", "x11", "x12")?;
            self.emit(abi::store_u64(
                &item_value,
                abi::stack_pointer(),
                entry_payload_slot + 8,
            ));
            self.emit(abi::add_immediate(
                "x11",
                abi::stack_pointer(),
                entry_payload_slot,
            ));
            self.emit(abi::store_u64("x11", abi::stack_pointer(), local_slot));
        } else {
            let item_value_type = item_value_type.ok_or_else(|| {
                format!(
                    "native code FOR EACH target '{}' is not a list",
                    iterable_value.type_
                )
            })?;
            self.emit(abi::load_u64(
                "x11",
                "x10",
                COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
            ));
            self.emit(abi::load_u64(
                "x12",
                "x10",
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ));
            self.emit(abi::load_u64("x8", abi::stack_pointer(), collection_slot));
            let item_value =
                self.emit_load_collection_payload(item_value_type, "x8", "x11", "x12")?;
            self.emit(abi::store_u64(
                &item_value,
                abi::stack_pointer(),
                local_slot,
            ));
        }
        self.emit(abi::load_u64("x10", abi::stack_pointer(), cursor_slot));
        self.emit(abi::add_immediate("x10", "x10", COLLECTION_ENTRY_SIZE));
        self.emit(abi::store_u64("x10", abi::stack_pointer(), cursor_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), remaining_slot));
        self.emit(abi::subtract_immediate("x9", "x9", 1));
        self.emit(abi::store_u64("x9", abi::stack_pointer(), remaining_slot));

        let previous = self.locals.insert(
            name.to_string(),
            LocalValue {
                type_: type_.to_string(),
                stack_offset: local_slot,
                constant: None,
                by_ref: false,
            },
        );
        self.clear_local_constants();
        self.loop_stack.push(LoopLabels {
            kind: crate::ast::LoopKind::For,
            continue_label: loop_label.clone(),
            exit_label: end_label.clone(),
            cleanup_depth: self.active_cleanups.len(),
        });
        self.lower_ops(body)?;
        self.loop_stack.pop();
        if pushed_iterable {
            self.for_each_iterable_locals.pop();
        }
        if let Some(previous) = previous {
            self.locals.insert(name.to_string(), previous);
        } else {
            self.locals.remove(name);
        }
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&end_label));
        self.clear_local_constants();
        Ok(())
    }
}

/// If `value` is a left-associated string-concat chain `name & a & b …` whose
/// leftmost leaf is `Local(name)`, return the operands to append in source order
/// (`[a, b, …]`); otherwise `None`. Used to recognize the in-place self-append
/// idiom `name = name & …` (plan-02 §4.1). `&` is string concatenation, so a
/// match guarantees `name` is a `String` local.
fn string_self_append_operands<'v>(value: &'v NirValue, name: &str) -> Option<Vec<&'v NirValue>> {
    let NirValue::Binary { op, left, right, .. } = value else {
        return None;
    };
    if op != "&" {
        return None;
    }
    let mut operands = vec![right.as_ref()];
    let mut cursor = left.as_ref();
    loop {
        match cursor {
            NirValue::Local(local) if local == name => {
                operands.reverse();
                return Some(operands);
            }
            NirValue::Binary { op, left, right, .. } if op == "&" => {
                operands.push(right.as_ref());
                cursor = left.as_ref();
            }
            _ => return None,
        }
    }
}

fn nir_op_context(op: &NirOp) -> String {
    match op {
        NirOp::Bind { name, type_, .. } => format!("bind {name} AS {type_}"),
        NirOp::StoreGlobal { name, .. } => format!("store global {name}"),
        NirOp::Assign { name, .. } => format!("assign {name}"),
        NirOp::StateAssign { resource, .. } => format!("state assign {resource}"),
        NirOp::Return { .. } => "return".to_string(),
        NirOp::ExitLoop { .. } => "exit loop".to_string(),
        NirOp::ContinueLoop { .. } => "continue loop".to_string(),
        NirOp::ExitProgram { .. } => "exit program".to_string(),
        NirOp::Fail { .. } => "fail".to_string(),
        NirOp::Eval { value } => format!("eval {}", nir_value_context(value)),
        NirOp::If { .. } => "if".to_string(),
        NirOp::Match { .. } => "match".to_string(),
        NirOp::While { .. } => "while".to_string(),
        NirOp::For { name, .. } => format!("for {name}"),
        NirOp::DoUntil { .. } => "do until".to_string(),
        NirOp::ForEach { name, .. } => format!("for each {name}"),
        NirOp::Trap { .. } => "trap".to_string(),
    }
}

fn nir_value_context(value: &NirValue) -> String {
    match value {
        NirValue::Call { target, .. }
        | NirValue::CallResult { target, .. }
        | NirValue::RuntimeCall { target, .. } => format!("call {target}"),
        NirValue::Constructor { type_, .. } => format!("construct {type_}"),
        NirValue::MemberAccess { member, .. } => format!("member {member}"),
        NirValue::Local(name) => format!("local {name}"),
        NirValue::LocalRef { name, .. } => format!("local ref {name}"),
        NirValue::Global { name, .. } => format!("global {name}"),
        NirValue::FunctionRef { name, .. } => format!("function {name}"),
        NirValue::Closure { name, .. } => format!("closure {name}"),
        NirValue::Const { type_, .. } => format!("const {type_}"),
        NirValue::ListLiteral { type_, .. } | NirValue::MapLiteral { type_, .. } => {
            format!("literal {type_}")
        }
        NirValue::Unary { op, .. } | NirValue::Binary { op, .. } => format!("operator {op}"),
        NirValue::UnionWrap {
            union_type,
            member_type,
            ..
        } => format!("wrap {member_type} AS {union_type}"),
        NirValue::UnionExtract { type_, .. } => format!("extract {type_}"),
        NirValue::ResultIsOk { .. } => "result is ok".to_string(),
        NirValue::ResultValue { .. } => "result value".to_string(),
        NirValue::ResultError { .. } => "result error".to_string(),
        NirValue::WithUpdate { type_, .. } => format!("with update {type_}"),
        NirValue::Capture { index, .. } => format!("capture {index}"),
    }
}
