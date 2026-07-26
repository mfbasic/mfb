use super::*;

impl CodeBuilder<'_> {
    pub(super) fn is_thread_type(type_: &str) -> bool {
        type_.starts_with("Thread OF ")
    }

    pub(super) fn thread_drop_symbol() -> String {
        runtime::symbol_for_call(runtime::RuntimeHelper::Thread, "thread.drop")
    }

    pub(super) fn deactivate_thread_cleanup(&mut self, name: &str) {
        if let Some(index) = self.active_cleanups.iter().rposition(
            |cleanup| matches!(cleanup, ActiveCleanup::Thread(thread) if thread.name == name),
        ) {
            self.active_cleanups.remove(index);
        }
    }

    pub(super) fn maybe_deactivate_moved_thread_local(&mut self, value: &NirValue) {
        let NirValue::Local(name) = value else {
            return;
        };
        if self
            .locals
            .get(name)
            .is_some_and(|local| Self::is_thread_type(&local.type_))
        {
            self.deactivate_thread_cleanup(name);
        }
    }

    /// A thread `start`/`send`/`emit`/`transferResource`/`emitResource` moves its
    /// data argument (`args[1]`) across the arena boundary. If that argument was a
    /// fresh heap temporary, claim it so the statement-scope free never reclaims a
    /// block the worker/queue may still reference — conservatively preserving the
    /// pre-plan-25 behaviour (these cross-arena values were never freed by the
    /// sender). A `Local` data argument is an aliasing source that was never
    /// registered, so this is a no-op for it (plan-25).
    pub(super) fn claim_moved_thread_arg_temp(&mut self, target: &str, arg_values: &[ValueResult]) {
        if matches!(
            target,
            "thread.start"
                | "thread.send"
                | "thread.emit"
                | "thread.transferResource"
                | "thread.emitResource"
        ) {
            if let Some(arg) = arg_values.get(1) {
                self.claim_pending_temp(arg);
            }
        }
    }

    pub(super) fn deactivate_moved_thread_arguments(&mut self, target: &str, args: &[NirValue]) {
        match target {
            "thread.start"
            | "thread.send"
            | "thread.emit"
            | "thread.transferResource"
            | "thread.emitResource" => {
                if let Some(arg) = args.get(1) {
                    self.maybe_deactivate_moved_thread_local(arg);
                }
            }
            target if !target.starts_with("thread.") => {
                for arg in args {
                    self.maybe_deactivate_moved_thread_local(arg);
                }
            }
            _ => {}
        }
    }

    pub(super) fn emit_thread_send_runtime_helper_call(
        &mut self,
        target: &str,
        symbol: &str,
        args: &[NirValue],
        result_type: &str,
        raw: bool,
    ) -> Result<ValueResult, String> {
        if args.len() < 2 {
            return Err(format!(
                "native runtime call '{target}' expects a handle and message"
            ));
        }
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let mut arg_values = Vec::new();
        let mut arg_slots = Vec::new();
        self.reset_temporary_registers();
        for arg in args {
            let value = self.lower_value(arg)?;
            // Observation boundary: a `Float` sent across a thread boundary is
            // observable on the other side and must be finite (plan-17).
            self.observe_float(arg, &value)?;
            // Materialize a `d`-native float before marshalling it across the
            // thread boundary (plan-01 float-dnative).
            let value = self.materialize_float(value)?;
            let slot = self.allocate_stack_object("runtime_thread_send_arg", 8);
            self.emit(abi::store_u64(&value.location, abi::stack_pointer(), slot));
            arg_values.push(value);
            arg_slots.push(slot);
            self.reset_temporary_registers();
        }
        // The message argument is copied into the destination arena below and then
        // moved; keep the statement-scope temp cleanup off it (plan-25).
        self.claim_moved_thread_arg_temp(target, &arg_values);

        self.reset_temporary_registers();
        let saved_arena_slot = self.allocate_stack_object("runtime_thread_send_saved_arena", 8);
        let copied_message_slot =
            self.allocate_stack_object("runtime_thread_send_copied_message", 8);
        let arena_offset = if target == "thread.emit" {
            THREAD_OFFSET_PARENT_ARENA_STATE
        } else {
            THREAD_OFFSET_ARENA_STATE
        };
        self.emit(abi::store_u64(
            ARENA_STATE_REGISTER,
            abi::stack_pointer(),
            saved_arena_slot,
        ));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), arg_slots[0]));
        self.emit(abi::load_u64(&scratch10, &scratch9, arena_offset));
        self.emit(abi::move_register(ARENA_STATE_REGISTER, &scratch10));
        self.error_arena_restore_slot = Some(saved_arena_slot);
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), arg_slots[1]));
        let copied = self.copy_value_to_current_arena(&arg_values[1].type_, &scratch9)?;
        self.error_arena_restore_slot = None;
        self.emit(abi::store_u64(
            &copied,
            abi::stack_pointer(),
            copied_message_slot,
        ));
        self.reset_temporary_registers();
        self.emit(abi::load_u64(
            ARENA_STATE_REGISTER,
            abi::stack_pointer(),
            saved_arena_slot,
        ));
        self.emit(abi::load_u64(
            &scratch9,
            abi::stack_pointer(),
            copied_message_slot,
        ));
        self.emit(abi::store_u64(
            &scratch9,
            abi::stack_pointer(),
            arg_slots[1],
        ));

        // Byte size of the message copy, passed as arg 3 so a failed send can reclaim
        // it via the queue's pending-free list (bug-147.5b). Computed ONLY for a flat
        // block type whose exact copy size `emit_inlined_block_size_from_ptr_slot`
        // returns (the `copy_flat_block` path — String / record / data-union / Result
        // / flat collection, all copied tight so the block size equals the alloc);
        // otherwise 0 = "not reclaimable" (a scalar has no copy block; a resource or
        // resource-embedding value copies through a path we do not size here and keeps
        // the pre-existing bounded leak rather than risk a wrong-size free).
        let msg_type = arg_values[1].type_.clone();
        let size_slot = self.allocate_stack_object("runtime_thread_send_copy_size", 8);
        let size_computable = self.type_is_flat(&msg_type)
            && (msg_type == "String"
                || self.type_model.record_fields.contains_key(&msg_type)
                || self.union_is_data(&msg_type)
                || msg_type.starts_with("Result OF ")
                || is_collection_type(&msg_type));
        if size_computable {
            self.emit_inlined_block_size_from_ptr_slot(&msg_type, copied_message_slot, size_slot)?;
        } else {
            self.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), size_slot));
        }
        self.reset_temporary_registers();

        for (index, slot) in arg_slots.iter().enumerate() {
            self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), *slot));
            self.emit(abi::move_register(
                &abi::argument_register(index)?,
                &scratch9,
            ));
        }
        // Arg 3: the message-copy size (0 when not reclaimable).
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), size_slot));
        self.emit(abi::move_register(&abi::argument_register(3)?, &scratch9));
        self.emit_symbol_call(symbol);

        // An inline `TRAP` traps the raw send `Result`. On failure the sent value
        // remains owned by the caller (the syntaxchecker restores the binding into
        // the handler scope); the success continuation treats it as moved.
        if raw {
            self.deactivate_moved_thread_arguments(target, args);
            self.deactivate_moved_resource_arguments(target, args);
            let _ = arg_values;
            // thread.send/emit errors originate at this call site, not a worker.
            return self.materialize_current_result(
                result_type,
                format!("callResult {target}"),
                false,
            );
        }

        let ok_label = self.label("runtime_thread_send_ok");
        self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
        self.emit(abi::branch_eq(&ok_label));
        self.emit_stamp_current_error_source()?;
        self.emit_current_result_exit(self.error_exit_destination())?;
        self.emit(abi::label(&ok_label));
        self.deactivate_moved_thread_arguments(target, args);
        self.deactivate_moved_resource_arguments(target, args);

        if result_type != "Nothing" {
            return Err(format!(
                "native runtime call '{target}' expected Nothing result, got '{result_type}'"
            ));
        }
        Ok(ValueResult {
            type_: result_type.to_string(),
            location: "void".to_string(),
            text: format!("call {target}({})", join_texts(&arg_values)),
        })
    }
}
