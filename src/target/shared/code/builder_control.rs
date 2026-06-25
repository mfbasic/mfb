use super::*;

impl CodeBuilder<'_> {
    pub(super) fn lower_ops(&mut self, ops: &[NirOp]) -> Result<(), String> {
        let cleanup_scope_start = self.active_cleanups.len();
        for op in ops {
            let result = (|| -> Result<(), String> {
                match op {
                    NirOp::Bind {
                        name, type_, value, ..
                    } => {
                        let stack_offset = self.allocate_stack_object(name, 8);
                        let constant = value
                            .as_ref()
                            .and_then(|value| self.local_constant_value(value));
                        self.locals.insert(
                            name.clone(),
                            LocalValue {
                                type_: type_.clone(),
                                stack_offset,
                                constant,
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
                        let runtime_managed = value
                            .as_ref()
                            .is_some_and(Self::value_is_runtime_managed);
                        // This binding owns a freeable flat block that scope-drop
                        // must free (plan-02 Phase 8).
                        let owns_freeable_value = !borrows_union_variant
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
                            // variant binding aliases the union deliberately.
                            let result = if borrows_union_variant {
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
                        } else if borrows_union_variant {
                            // Borrowed — no cleanup.
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
                            self.active_cleanups
                                .push(ActiveCleanup::ResourceUnion(ResourceUnionCleanup {
                                    name: name.clone(),
                                    variants,
                                }));
                        } else if owns_freeable_value {
                            // An owned, non-escaping flat value (plan-01 Phase 5 /
                            // plan-02 Phase 8): a single `arena_free` of its block
                            // reclaims everything at scope-drop. Copy-insertion
                            // (`lower_value_owned`) guarantees this block is
                            // unaliased, so the free is sound and once-only.
                            self.active_cleanups
                                .push(ActiveCleanup::OwnedValue(OwnedValueCleanup {
                                    type_: type_.clone(),
                                    stack_offset,
                                }));
                        }
                        // Default-initialize a `RES` binding's `STATE` payload.
                        // The owning binding allocates the state record on first
                        // bind; a moved/returned resource that already carries a
                        // state keeps it (the slot is non-null).
                        if let Some(state_type) =
                            crate::builtins::resource::state_type_name(type_)
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
                        let stack_offset = self
                            .locals
                            .get(name)
                            .ok_or_else(|| {
                                format!("native code assignment unknown local '{name}'")
                            })?
                            .stack_offset;
                        // Reassignment installs a fresh independent block; the old
                        // block remains owned by this binding's scope-drop free
                        // (the slot is overwritten with the new owner). Deep-copy
                        // an aliasing source so the binding stays unaliased.
                        let result = self.lower_value_owned(value)?;
                        let assign_slot = if Self::is_thread_type(&result.type_) {
                            let slot = self.allocate_stack_object("thread_assign_value", 8);
                            self.emit(abi::store_u64(&result.location, abi::stack_pointer(), slot));
                            self.emit_thread_cleanup_for_name(name)?;
                            Some(slot)
                        } else if let Some(symbol) = self.resource_cleanup_symbol(&result.type_) {
                            let slot = self.allocate_stack_object("resource_assign_value", 8);
                            self.emit(abi::store_u64(&result.location, abi::stack_pointer(), slot));
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
                        self.emit(abi::store_u64(
                            &result_location,
                            abi::stack_pointer(),
                            stack_offset,
                        ));
                        let constant = self.local_constant_value(value);
                        if let Some(local) = self.locals.get_mut(name) {
                            local.constant = constant;
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
                                format!(
                                    "native code state assignment unknown local '{resource}'"
                                )
                            })?
                            .stack_offset;
                        let result = self.lower_value(value)?;
                        let value_slot =
                            self.allocate_stack_object("state_assign_value", 8);
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
                    ActiveCleanup::OwnedList(cleanup) => {
                        self.emit_owned_list_drain(&cleanup)?
                    }
                    ActiveCleanup::OwnedValue(cleanup) => {
                        self.emit_owned_value_drop(&cleanup)?
                    }
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
