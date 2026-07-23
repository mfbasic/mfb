use super::*;

impl CodeBuilder<'_> {
    pub(super) fn emit_cleanup_sequence(&mut self) -> Result<(), String> {
        let cleanups = self.active_cleanups.clone();
        self.emit_cleanups(&cleanups)
    }

    /// The cleanups present at this function's top-level (trap-handler) scope.
    /// The trap body runs at the function's top-level scope, so an error routed
    /// to it stays inside the function and these locals remain in scope. Only
    /// cleanups belonging to inner blocks being exited by the jump to the trap
    /// are *fully* out of scope. `cleanup_scope_starts[0]` is the function body;
    /// `[1]`, when present, is the first nested block's entry depth — i.e. the
    /// function-level cleanup count. With no inner block open, every active
    /// cleanup is function-level.
    pub(super) fn trap_cleanup_floor(&self) -> usize {
        self.cleanup_scope_starts
            .get(1)
            .copied()
            .unwrap_or(self.active_cleanups.len())
    }

    /// The cleanups to run when an error is routed to this function's trap
    /// handler. Inner-block locals being exited (`index >= floor`) are dropped
    /// like any other exit. A function-level (trap-shared) **owned arena value**
    /// is *not* dropped here: it stays live for the handler to read and is freed
    /// exactly once on the handler's own exit — dropping it here would
    /// double-free it. Trap-shared threads/resources *are* still dropped here:
    /// their drop is idempotent (the handler's later drop is a harmless no-op)
    /// and propagating an error past them cancels/closes them as before.
    pub(super) fn trap_route_cleanups(&self) -> Vec<ActiveCleanup> {
        let floor = self.trap_cleanup_floor();
        self.active_cleanups
            .iter()
            .enumerate()
            .filter(|(index, cleanup)| {
                !(*index < floor && matches!(cleanup, ActiveCleanup::OwnedValue(_)))
            })
            .map(|(_, cleanup)| cleanup.clone())
            .collect()
    }

    /// Emit the scope-drop frees for `cleanups` (innermost/last first).
    pub(super) fn emit_cleanups(&mut self, cleanups: &[ActiveCleanup]) -> Result<(), String> {
        for cleanup in cleanups.iter().rev() {
            match cleanup {
                ActiveCleanup::Thread(cleanup) => {
                    self.emit_thread_cleanup_call(cleanup)?;
                }
                ActiveCleanup::Resource(cleanup) => {
                    self.emit_resource_cleanup_call(cleanup)?;
                }
                ActiveCleanup::ResourceUnion(cleanup) => {
                    self.emit_resource_union_cleanup_call(cleanup)?;
                }
                ActiveCleanup::OwnedList(cleanup) => {
                    self.emit_owned_list_drain(cleanup)?;
                }
                ActiveCleanup::OwnedValue(cleanup) => {
                    self.emit_owned_value_drop(cleanup)?;
                }
            }
        }
        Ok(())
    }

    pub(super) fn emit_cleanup_branch_to_depth(
        &mut self,
        target: &str,
        cleanup_depth: usize,
    ) -> Result<(), String> {
        let cleanups = self.active_cleanups[cleanup_depth..].to_vec();
        for cleanup in cleanups.iter().rev() {
            match cleanup {
                ActiveCleanup::Thread(cleanup) => self.emit_thread_cleanup_call(cleanup)?,
                ActiveCleanup::Resource(cleanup) => self.emit_resource_cleanup_call(cleanup)?,
                ActiveCleanup::ResourceUnion(cleanup) => {
                    self.emit_resource_union_cleanup_call(cleanup)?
                }
                ActiveCleanup::OwnedList(cleanup) => self.emit_owned_list_drain(cleanup)?,
                ActiveCleanup::OwnedValue(cleanup) => self.emit_owned_value_drop(cleanup)?,
            }
        }
        self.emit(abi::branch(target));
        Ok(())
    }

    pub(super) fn emit_program_exit_value(&mut self, code: &NirValue) -> Result<(), String> {
        let result = self.lower_value(code)?;
        self.emit(abi::move_register(abi::return_register(), &result.location));
        self.emit(abi::move_register(
            RESULT_VALUE_REGISTER,
            abi::return_register(),
        ));
        self.emit(abi::move_immediate(
            RESULT_TAG_REGISTER,
            "Integer",
            RESULT_PROGRAM_EXIT_TAG,
        ));
        self.emit(abi::move_immediate(
            RESULT_ERROR_MESSAGE_REGISTER,
            "Integer",
            "0",
        ));
        self.emit_current_result_exit(ExitDestination::Return)
    }

    pub(super) fn emit_current_result_exit(
        &mut self,
        destination: ExitDestination,
    ) -> Result<(), String> {
        // A `Return` leaves the function and frees every live local. A `Trap`
        // jumps to the same-function handler, where function-level locals stay in
        // scope; `trap_route_cleanups` omits their owned arena values so the
        // handler can read them and free them once on its own exit (freeing them
        // here would double-free).
        let cleanups = match destination {
            ExitDestination::Return => self.active_cleanups.clone(),
            ExitDestination::Trap => self.trap_route_cleanups(),
        };
        if !cleanups.is_empty() {
            self.store_pending_current_result();
            // plan-59-D: only a `RETURN` carries a value out of this scope, so
            // only here is a cleanup allowed to skip on pointer identity. The
            // slot is the one `store_pending_current_result` just wrote, which is
            // live precisely across `emit_cleanups` (cleanups clobber every
            // caller-saved register, so the result must be parked anyway).
            //
            // Left `None` for `ExitDestination::Trap`: routing to a handler is
            // not an escape, and §15 requires the resource still be closed.
            let previous_escaping = self.escaping_value_slot;
            if matches!(destination, ExitDestination::Return) {
                self.escaping_value_slot = self.pending_result_slots.map(|slots| slots.value);
            }
            let result = self.emit_cleanups(&cleanups);
            self.escaping_value_slot = previous_escaping;
            result?;
            self.load_pending_result_registers();
        }
        match destination {
            ExitDestination::Return => self.emit(abi::return_()),
            ExitDestination::Trap => self.route_current_result_to_trap()?,
        }
        Ok(())
    }

    pub(super) fn emit_error_value_exit(
        &mut self,
        error: &NirValue,
        destination: ExitDestination,
    ) -> Result<(), String> {
        // See `emit_current_result_exit`: a trap route keeps the function-level
        // locals' owned arena values live for the handler, freeing only the rest.
        let cleanups = match destination {
            ExitDestination::Return => self.active_cleanups.clone(),
            ExitDestination::Trap => self.trap_route_cleanups(),
        };
        if cleanups.is_empty() {
            return match destination {
                ExitDestination::Return => self.emit_direct_error_return(error),
                ExitDestination::Trap => self.emit_direct_error_route_to_trap(error),
            };
        }
        self.store_pending_error_from_value(error)?;
        self.emit_cleanups(&cleanups)?;
        self.load_pending_result_registers();
        match destination {
            ExitDestination::Return => self.emit(abi::return_()),
            ExitDestination::Trap => self.route_current_result_to_trap()?,
        }
        Ok(())
    }

    /// Lower a returned value as a caller-owned, standalone block. An aliasing
    /// source of a freeable flat type is deep-copied here (plan-02 Phase 8): the
    /// returned block must outlive this scope's frees and is owned/freed by the
    /// caller, so it cannot remain an alias into a local that is about to be
    /// freed. The bool is `already_standalone` — true when the result is a fresh
    /// standalone allocation (a copy made here) that must NOT be re-materialized;
    /// false for a fresh value or an alias of a non-flat type, which keep the
    /// existing inline-payload materialization. A returned thread/resource local
    /// is a move (never freeable-flat) and is handled by cleanup deactivation.
    pub(super) fn lower_returned_value(
        &mut self,
        value: &NirValue,
        move_elided: bool,
    ) -> Result<(ValueResult, bool), String> {
        // Copy elision via ownership transfer (plan-25-C C1): `emit_return_exit`
        // has already removed this owned local's scope-drop free for this return
        // path, transferring its uniquely-owned block to the caller — so return the
        // existing block pointer directly (a standalone arena block) instead of
        // deep-copying it. Copy insertion (`lower_value_owned`) guarantees the block
        // has no live alias, so the move creates exactly one owner, never two.
        if move_elided {
            return Ok((self.lower_value(value)?, true));
        }
        if self.value_needs_owning_copy(value) {
            let lowered = self.lower_value(value)?;
            if self.is_freeable_flat_value(&lowered.type_) {
                let copied = self.copy_flat_block(&lowered.type_, &lowered.location)?;
                return Ok((
                    ValueResult {
                        type_: lowered.type_,
                        location: copied,
                        text: lowered.text,
                    },
                    true,
                ));
            }
            return Ok((lowered, false));
        }
        Ok((self.lower_value(value)?, false))
    }

    /// Plan a return-value copy elision (plan-25-C C1). A `RETURN <owned-local>`
    /// of a freeable-flat binding that owns its block moves the block to the
    /// caller instead of deep-copying it: the function exits at the return, so the
    /// local is dead, and dropping its scope-drop free leaves the caller the sole
    /// owner (one free total). When eligible this removes the binding's
    /// `OwnedValue` cleanup from the live set and returns the pre-removal cleanup
    /// stack, which `emit_return_exit` restores after the return is emitted — a
    /// sibling return path or the enclosing block's normal exit must still free the
    /// binding. Returns `None` (cleanup untouched, copy kept) when the move would
    /// be unsound:
    ///
    /// - A **parameter** or `by_ref` local is a reference into the caller's block (it
    ///   has no `OwnedValue` free), so returning its pointer without copying would
    ///   let the caller's binding double-free the source — the copy is load-bearing.
    /// - A **`FOR EACH` iterable** whose iterator still reads the block, or an
    ///   **address-taken** local an escaping closure env may reference, could leave a
    ///   dangling reader if the block moved out.
    pub(super) fn plan_returned_move(
        &mut self,
        value: Option<&NirValue>,
    ) -> Option<Vec<ActiveCleanup>> {
        let NirValue::Local(name) = value? else {
            return None;
        };
        let local = self.locals.get(name)?;
        if local.by_ref {
            return None;
        }
        let stack_offset = local.stack_offset;
        if self.for_each_iterable_locals.iter().any(|n| n == name)
            || self.address_taken_locals.contains(name)
        {
            return None;
        }
        // Only a binding that owns its block (has a live `OwnedValue` free at this
        // slot) can be moved; parameters and aliases have none, so this is the
        // authoritative ownership gate.
        let index = self.active_cleanups.iter().rposition(|cleanup| {
            matches!(cleanup, ActiveCleanup::OwnedValue(c) if c.stack_offset == stack_offset)
        })?;
        let saved = self.active_cleanups.clone();
        self.active_cleanups.remove(index);
        Some(saved)
    }

    pub(super) fn emit_return_exit(&mut self, value: Option<&NirValue>) -> Result<(), String> {
        // Plan a return-copy elision (plan-25-C C1) before emitting: a movable
        // `RETURN <owned-local>` removes the binding's scope-drop free for this
        // path so the block moves to the caller uncopied. Restore the live cleanup
        // set afterward so a sibling return path or the block's normal exit still
        // frees the binding.
        let restore_cleanups = self.plan_returned_move(value);
        let result = self.emit_return_exit_inner(value, restore_cleanups.is_some());
        if let Some(saved) = restore_cleanups {
            self.active_cleanups = saved;
        }
        result
    }

    pub(super) fn emit_return_exit_inner(
        &mut self,
        value: Option<&NirValue>,
        move_elided: bool,
    ) -> Result<(), String> {
        let lowered = if let Some(value) = value {
            Some(self.lower_returned_value(value, move_elided)?)
        } else {
            None
        };
        let already_standalone = lowered
            .as_ref()
            .map(|(_, standalone)| *standalone)
            .unwrap_or(true);
        let result = lowered.map(|(result, _)| result);
        // Observation boundary: a returned `Float` becomes the caller's value
        // and must be finite (plan-17).
        if let (Some(value), Some(result)) = (value, result.as_ref()) {
            self.observe_float(value, result)?;
        }
        // The return value travels in a GPR (`RESULT_VALUE_REGISTER`), so a
        // `d`-native float is materialized into one first (ABI option (b),
        // plan-01 float-dnative §4.3), and a register-native vector into its block
        // pointer. Identity for every GP-native value.
        let result = match result {
            Some(result) => Some(self.materialize_value(result)?),
            None => None,
        };
        if self.active_cleanups.is_empty() {
            if let Some(result) = &result {
                if result.type_ != "Nothing" {
                    let location = if !already_standalone
                        && self.inline_collection_payload_size(&result.type_).is_some()
                    {
                        self.materialize_inline_value_in_arena(&result.type_, &result.location)?
                    } else {
                        result.location.clone()
                    };
                    self.emit(abi::move_register(RESULT_VALUE_REGISTER, &location));
                }
            }
            self.emit(abi::move_immediate(
                RESULT_TAG_REGISTER,
                "Integer",
                RESULT_OK_TAG,
            ));
            self.emit(abi::return_());
            return Ok(());
        }
        self.store_pending_success_result(result.as_ref(), already_standalone)?;
        if let Some(value) = value {
            if let NirValue::Local(name) = value {
                if result
                    .as_ref()
                    .is_some_and(|result| Self::is_thread_type(&result.type_))
                {
                    self.deactivate_thread_cleanup(name);
                }
                if result.as_ref().is_some_and(|result| {
                    crate::builtins::resource_close_function(&result.type_).is_some()
                }) {
                    self.deactivate_resource_cleanup(name);
                }
                // A returned resource union transfers ownership to the caller;
                // deactivate its tag-dispatched drop so the callee does not close
                // the resource the caller now owns (bug-141). `resource_close_function`
                // is `None` for a union type, so the plain-resource branch above
                // misses it — key off the union cleanup shape instead.
                if result
                    .as_ref()
                    .is_some_and(|result| self.resource_union_cleanup(&result.type_).is_some())
                {
                    self.deactivate_resource_cleanup(name);
                }
                // Returning a `List OF RES File` transfers its owned-list to the
                // caller: drop this scope's drain so the resources are not closed
                // here (§15.6).
                if result
                    .as_ref()
                    .is_some_and(|result| Self::is_res_marked_resource_collection(&result.type_))
                {
                    self.deactivate_owned_list(name);
                }
            }
        }
        // plan-59-D: this is the real `RETURN <value>` path, so the identity skip
        // belongs here — around the cleanup sequence, with the escaping value in
        // the slot `store_pending_success_result` wrote above.
        //
        // It BACKSTOPS the static deactivations above rather than replacing them.
        // Those are keyed on the returned value being syntactically the
        // `NirValue::Local` that owns the cleanup; a returned resource that is not
        // that local is invisible to them, and the runtime compare catches it.
        // Where both apply the skip is simply never reached, because the cleanup
        // has already been removed from the list.
        let previous_escaping = self.escaping_value_slot;
        self.escaping_value_slot = self.pending_result_slots.map(|slots| slots.value);
        let cleanup_result = self.emit_cleanup_sequence();
        self.escaping_value_slot = previous_escaping;
        cleanup_result?;
        self.load_pending_result_registers();
        self.emit(abi::return_());
        Ok(())
    }

    pub(super) fn route_current_result_to_trap(&mut self) -> Result<(), String> {
        self.emit(abi::compare_immediate(
            RESULT_TAG_REGISTER,
            RESULT_PROGRAM_EXIT_TAG,
        ));
        let trap_label = self.label("trap_route_error");
        self.emit(abi::branch_ne(&trap_label));
        self.emit(abi::return_());
        self.emit(abi::label(&trap_label));

        let code_slot = self.allocate_stack_object("trap_error_code", 8);
        let message_slot = self.allocate_stack_object("trap_error_message", 8);
        let source_slot = self.allocate_stack_object("trap_error_source", 8);
        // The function-level trap local's slot is pinned in `TrapState`, not read
        // from `self.locals[name]`: an inline `TRAP(e)` in the body rebinds the
        // shared name `e` to a different slot, so a `self.locals` lookup here would
        // store the built `Error` to whichever slot was last bound, desyncing it
        // from the handler's read of the pinned slot (bug-148).
        let (stack_offset, label) = self
            .trap
            .as_ref()
            .map(|trap| (trap.stack_offset, trap.label.clone()))
            .ok_or_else(|| "trap routing requires bound trap local".to_string())?;

        // Design "b": an `ERR_BLOCK` error parked its owned Error block base in the
        // current-error slot. ADOPT it as the trap local (freed once on the
        // handler's exit) and clear the slot, instead of rebuilding a fresh block
        // and orphaning the parked one (bug-152). A legacy `ERR` falls through to
        // the rebuild below.
        let rebuild_label = self.label("trap_route_rebuild");
        self.emit(abi::compare_immediate(
            RESULT_TAG_REGISTER,
            RESULT_ERR_BLOCK_TAG,
        ));
        self.emit(abi::branch_ne(&rebuild_label));
        let adopted = self.emit_adopt_current_error_block();
        self.emit(abi::store_u64(&adopted, abi::stack_pointer(), stack_offset));
        self.emit(abi::branch(&label));

        self.emit(abi::label(&rebuild_label));
        self.emit(abi::store_u64(
            RESULT_VALUE_REGISTER,
            abi::stack_pointer(),
            code_slot,
        ));
        self.emit(abi::store_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            message_slot,
        ));
        self.emit(abi::store_u64(
            RESULT_ERROR_SOURCE_REGISTER,
            abi::stack_pointer(),
            source_slot,
        ));
        let error_register = self.emit_build_error_inline(code_slot, message_slot, source_slot)?;
        self.emit(abi::store_u64(
            &error_register,
            abi::stack_pointer(),
            stack_offset,
        ));
        self.emit(abi::branch(&label));
        Ok(())
    }

    pub(super) fn current_block_returns(&self) -> bool {
        self.instructions
            .last()
            .is_some_and(|instruction| matches!(instruction.op, CodeOp::Ret | CodeOp::Branch))
    }
}
