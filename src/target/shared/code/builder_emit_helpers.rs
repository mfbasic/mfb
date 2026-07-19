use super::*;

impl CodeBuilder<'_> {
    pub(super) fn emit_symbol_call(&mut self, symbol: &str) {
        self.emit(abi::branch_link(symbol));
        let (binding, library) = if let Some(library) = self.platform_imports.get(symbol) {
            ("external".to_string(), Some(library.clone()))
        } else {
            ("internal".to_string(), None)
        };
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: symbol.to_string(),
            kind: RelocIntent::Call,
            binding,
            library,
        });
    }

    /// Call `_mfb_arena_alloc` (size in `x0`, alignment in `x1`) and compare the
    /// result tag against `RESULT_OK_TAG`, leaving the caller to branch.
    ///
    /// This exact twelve-line sequence was open-coded at 45 sites across 11
    /// files (bug-322). It routes through `emit_symbol_call`, which is
    /// output-identical here: that helper emits `("internal", None)` for any
    /// symbol `platform_imports` does not carry, and no backend ever lists an
    /// arena symbol as a platform import — pinned by
    /// `arena_symbols_are_never_platform_imports` so the equivalence cannot
    /// quietly lapse.
    pub(super) fn emit_arena_alloc_call(&mut self) {
        self.emit_symbol_call(ARENA_ALLOC_SYMBOL);
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
    }

    fn emit_prepared_call_args(
        &mut self,
        args: &[NirValue],
        slot_name: &str,
    ) -> Result<Vec<ValueResult>, String> {
        let scratch9 = self.temporary_vreg();
        let mut arg_values = Vec::new();
        let mut arg_slots = Vec::new();
        for arg in args {
            let value = self.lower_value(arg)?;
            // Observation boundary: a `Float` argument is read by the callee
            // (user FUNC/SUB, runtime helper, or native `LINK` thunk) and must
            // be finite (plan-17).
            self.observe_float(arg, &value)?;
            // Arguments are marshalled through integer slots/registers, so a
            // `d`-native float is materialized into a GPR first (ABI option (b),
            // plan-01 float-dnative §4.3), and a register-native vector into its
            // block pointer. Identity for GP-native values.
            let value = self.materialize_value(value)?;
            let slot = self.allocate_stack_object(slot_name, 8);
            self.emit(abi::store_u64(&value.location, abi::stack_pointer(), slot));
            arg_values.push(value);
            arg_slots.push(slot);
            self.reset_temporary_registers();
        }
        self.reset_temporary_registers();
        // Arguments beyond the 8 register slots are marshalled first into the
        // caller's reserved outgoing stack tail (bug-08); doing the stack stores
        // before the register moves keeps `x0`–`x7` set last, immediately before
        // the call, so nothing clobbers them. For a call of 8 or fewer arguments
        // this loop is empty and the code below is byte-identical to the
        // register-only convention.
        for (index, slot) in arg_slots.iter().enumerate() {
            if index < abi::REGISTER_ARGUMENT_COUNT {
                continue;
            }
            self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), *slot));
            self.emit(abi::outgoing_stack_arg_store(
                &scratch9,
                index - abi::REGISTER_ARGUMENT_COUNT,
            ));
        }
        for (index, slot) in arg_slots.iter().enumerate() {
            if index >= abi::REGISTER_ARGUMENT_COUNT {
                continue;
            }
            self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), *slot));
            self.emit(abi::move_register(
                &abi::argument_register(index)?,
                &scratch9,
            ));
        }
        Ok(arg_values)
    }

    pub(super) fn emit_raw_call(
        &mut self,
        symbol: &str,
        args: &[NirValue],
        slot_name: &str,
    ) -> Result<Vec<ValueResult>, String> {
        let arg_values = self.emit_prepared_call_args(args, slot_name)?;
        self.emit_symbol_call(symbol);
        Ok(arg_values)
    }

    pub(super) fn load_empty_string_constant(&mut self) -> Result<String, String> {
        let register = self.allocate_register()?;
        self.emit_load_static_string_symbol(&register, EMPTY_STRING_SYMBOL);
        Ok(register)
    }

    pub(super) fn load_string_constant(&mut self, value: &str) -> Result<String, String> {
        let register = self.allocate_register()?;
        self.emit_load_string_constant(&register, value)?;
        Ok(register)
    }

    pub(super) fn emit_load_string_constant(
        &mut self,
        register: &str,
        value: &str,
    ) -> Result<(), String> {
        let symbol = self
            .string_symbols
            .get(value)
            .ok_or_else(|| format!("native code string literal '{value}' has no data object"))?
            .clone();
        self.emit(abi::load_page_address(register, &symbol));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: symbol.clone(),
            kind: RelocIntent::DataAddrHi,
            binding: "data".to_string(),
            library: None,
        });
        self.emit(abi::add_page_offset(register, register, &symbol));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: symbol,
            kind: RelocIntent::DataAddrLo,
            binding: "data".to_string(),
            library: None,
        });
        Ok(())
    }

    pub(super) fn emit_load_static_string_symbol(&mut self, register: &str, symbol: &str) {
        self.emit(abi::load_page_address(register, symbol));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: symbol.to_string(),
            kind: RelocIntent::DataAddrHi,
            binding: "data".to_string(),
            library: None,
        });
        self.emit(abi::add_page_offset(register, register, symbol));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: symbol.to_string(),
            kind: RelocIntent::DataAddrLo,
            binding: "data".to_string(),
            library: None,
        });
    }

    pub(super) fn emit_call(
        &mut self,
        target: &str,
        symbol: &str,
        args: &[NirValue],
        return_type: Option<&str>,
    ) -> Result<ValueResult, String> {
        let arg_values = self.emit_raw_call(symbol, args, "call_arg")?;
        let result_type = return_type
            .map(|type_| type_.to_string())
            .or_else(|| {
                self.functions
                    .get(target)
                    .map(|function| function.returns.clone())
            })
            .or_else(|| self.package_return_types.get(target).cloned())
            .unwrap_or_else(|| "Unknown".to_string());
        if result_type == "Nothing" {
            if return_type.is_none() {
                let ok_label = self.label("call_ok");
                self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
                self.emit(abi::branch_eq(&ok_label));
                self.emit_current_result_exit(self.error_exit_destination())?;
                self.emit(abi::label(&ok_label));
            }
            self.deactivate_moved_thread_arguments(target, args);
            self.deactivate_moved_resource_arguments(target, args);
            return Ok(ValueResult {
                type_: result_type,
                location: "void".to_string(),
                text: format!("call {target}({})", join_texts(&arg_values)),
            });
        }
        if return_type.is_none() {
            let ok_label = self.label("call_ok");
            self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
            self.emit(abi::branch_eq(&ok_label));
            self.emit_current_result_exit(self.error_exit_destination())?;
            self.emit(abi::label(&ok_label));
        }
        self.deactivate_moved_thread_arguments(target, args);
        self.deactivate_moved_resource_arguments(target, args);
        let register = self.allocate_register()?;
        self.emit(abi::move_register(&register, RESULT_VALUE_REGISTER));
        Ok(ValueResult {
            type_: result_type,
            location: register,
            text: format!("call {target}({})", join_texts(&arg_values)),
        })
    }

    pub(super) fn emit_function_value_call(
        &mut self,
        target: &str,
        callable: &ValueResult,
        args: &[NirValue],
        return_type: Option<&str>,
    ) -> Result<ValueResult, String> {
        let arg_values = self.emit_prepared_call_args(args, "call_arg")?;
        let saved_env_slot = self.allocate_stack_object("closure_saved_env", 8);
        let code_register = self.allocate_register()?;
        let env_register = self.allocate_register()?;
        self.emit(abi::store_u64(
            CLOSURE_ENV_REGISTER,
            abi::stack_pointer(),
            saved_env_slot,
        ));
        self.emit(abi::load_u64(
            &code_register,
            &callable.location,
            CLOSURE_OFFSET_CODE,
        ));
        self.emit(abi::load_u64(
            &env_register,
            &callable.location,
            CLOSURE_OFFSET_ENV,
        ));
        self.emit(abi::move_register(CLOSURE_ENV_REGISTER, &env_register));
        self.emit(abi::branch_link_register(&code_register));
        self.emit(abi::load_u64(
            CLOSURE_ENV_REGISTER,
            abi::stack_pointer(),
            saved_env_slot,
        ));
        let result_type = return_type
            .map(|type_| type_.to_string())
            .unwrap_or_else(|| "Unknown".to_string());
        if result_type == "Nothing" {
            if return_type.is_none() {
                let ok_label = self.label("call_value_ok");
                self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
                self.emit(abi::branch_eq(&ok_label));
                self.emit_current_result_exit(self.error_exit_destination())?;
                self.emit(abi::label(&ok_label));
            }
            for arg in args {
                self.maybe_deactivate_moved_thread_local(arg);
            }
            self.deactivate_moved_resource_arguments(target, args);
            return Ok(ValueResult {
                type_: result_type,
                location: "void".to_string(),
                text: format!("call {target}({})", join_texts(&arg_values)),
            });
        }
        if return_type.is_none() {
            let ok_label = self.label("call_value_ok");
            self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
            self.emit(abi::branch_eq(&ok_label));
            self.emit_current_result_exit(self.error_exit_destination())?;
            self.emit(abi::label(&ok_label));
        }
        for arg in args {
            self.maybe_deactivate_moved_thread_local(arg);
        }
        self.deactivate_moved_resource_arguments(target, args);
        let register = self.allocate_register()?;
        self.emit(abi::move_register(&register, RESULT_VALUE_REGISTER));
        Ok(ValueResult {
            type_: result_type,
            location: register,
            text: format!("call {target}({})", join_texts(&arg_values)),
        })
    }

    pub(super) fn emit_runtime_helper_call(
        &mut self,
        target: &str,
        symbol: &str,
        args: &[NirValue],
        result_type: &str,
        raw: bool,
    ) -> Result<ValueResult, String> {
        if matches!(
            target,
            "thread.send" | "thread.emit" | "thread.transferResource" | "thread.emitResource"
        ) {
            return self.emit_thread_send_runtime_helper_call(
                target,
                symbol,
                args,
                result_type,
                raw,
            );
        }

        let arg_values = self.emit_raw_call(symbol, args, "runtime_call_arg")?;
        // A moved cross-arena data argument (e.g. `thread.start`) must not be freed
        // by this statement's temp cleanup (plan-25).
        self.claim_moved_thread_arg_temp(target, &arg_values);

        // An inline `TRAP` traps the raw `Result`: do not auto-propagate on
        // error; materialize the outcome (with the success value copied into the
        // current arena) for the trap to inspect. Owned handles/resources passed
        // to a consuming helper are consumed regardless of success or failure.
        if raw {
            self.deactivate_moved_thread_arguments(target, args);
            self.deactivate_moved_resource_arguments(target, args);
            let _ = arg_values;
            return self.materialize_current_result(
                result_type,
                format!("callResult {target}"),
                target == "thread.waitFor",
            );
        }

        let ok_label = self.label("runtime_call_ok");
        self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
        self.emit(abi::branch_eq(&ok_label));
        // A runtime helper error originates at this call site: stamp the origin
        // before propagating so a trapped error reports the true location.
        // `thread.waitFor` instead propagates a worker's terminal error whose
        // origin (and message) must be deep-copied out of the worker arena before
        // the impending `thread.drop` cleanup frees it.
        if target == "thread.waitFor" {
            self.emit_finalize_worker_error_source()?;
        } else {
            self.emit_stamp_current_error_source()?;
        }
        self.emit_current_result_exit(self.error_exit_destination())?;
        self.emit(abi::label(&ok_label));
        self.deactivate_moved_thread_arguments(target, args);
        self.deactivate_moved_resource_arguments(target, args);

        if result_type == "Nothing" {
            return Ok(ValueResult {
                type_: result_type.to_string(),
                location: "void".to_string(),
                text: format!("call {target}({})", join_texts(&arg_values)),
            });
        }

        let register = if matches!(
            target,
            "thread.waitFor"
                | "thread.read"
                | "thread.receive"
                | "thread.acceptResource"
                | "thread.readResource"
        ) {
            self.reset_temporary_registers();
            self.copy_value_to_current_arena(result_type, RESULT_VALUE_REGISTER)?
        } else {
            let register = self.allocate_register()?;
            self.emit(abi::move_register(&register, RESULT_VALUE_REGISTER));
            register
        };
        Ok(ValueResult {
            type_: result_type.to_string(),
            location: register,
            text: format!("call {target}({})", join_texts(&arg_values)),
        })
    }

    fn emit_thread_send_runtime_helper_call(
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

    /// Load the address of a string constant into the given register without
    /// allocating from the temporary-register pool.
    pub(super) fn emit_load_string_address_into(
        &mut self,
        register: &str,
        value: &str,
    ) -> Result<(), String> {
        let symbol = self
            .string_symbols
            .get(value)
            .ok_or_else(|| format!("native code string literal '{value}' has no data object"))?
            .clone();
        self.emit_load_static_string_symbol(register, &symbol);
        Ok(())
    }
}

#[cfg(test)]
mod arena_call_tests {
    use crate::target::shared::code::{ARENA_ALLOC_SYMBOL, ARENA_FREE_SYMBOL};

    /// `emit_arena_alloc_call` (bug-322) replaced 45 hand-written blocks that
    /// pushed `binding: "internal", library: None` unconditionally. Routing them
    /// through `emit_symbol_call` is output-identical only while no backend
    /// declares an arena symbol as a platform import — if one ever did, those 45
    /// sites would silently start emitting an *external* relocation against a
    /// library, which is a linker-visible change no unit test would otherwise
    /// catch.
    ///
    /// The plan modules are the only source of `platform_imports` keys, so this
    /// scans them as text: a grep-equivalent that cannot drift from the real
    /// tables the way a hand-copied list would.
    #[test]
    fn arena_symbols_are_never_platform_imports() {
        let plans = [
            ("linux_aarch64", include_str!("../../linux_aarch64/plan.rs")),
            ("linux_x86_64", include_str!("../../linux_x86_64/plan.rs")),
            ("linux_riscv64", include_str!("../../linux_riscv64/plan.rs")),
            ("macos_aarch64", include_str!("../../macos_aarch64/plan.rs")),
        ];
        for (target, source) in plans {
            for symbol in [ARENA_ALLOC_SYMBOL, ARENA_FREE_SYMBOL] {
                assert!(
                    !source.contains(symbol),
                    "{target}'s plan mentions {symbol}: if it became a platform import, \
                     emit_arena_alloc_call would emit an external relocation where the \
                     hand-written blocks it replaced emitted an internal one"
                );
            }
        }
    }
}
