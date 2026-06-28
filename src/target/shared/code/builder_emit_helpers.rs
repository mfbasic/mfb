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
            kind: "branch26".to_string(),
            binding,
            library,
        });
    }

    fn emit_prepared_call_args(
        &mut self,
        args: &[NirValue],
        slot_name: &str,
    ) -> Result<Vec<ValueResult>, String> {
        let mut arg_values = Vec::new();
        let mut arg_slots = Vec::new();
        for arg in args {
            let value = self.lower_value(arg)?;
            let slot = self.allocate_stack_object(slot_name, 8);
            self.emit(abi::store_u64(&value.location, abi::stack_pointer(), slot));
            arg_values.push(value);
            arg_slots.push(slot);
            self.reset_temporary_registers();
        }
        self.reset_temporary_registers();
        for (index, slot) in arg_slots.iter().enumerate() {
            self.emit(abi::load_u64("x9", abi::stack_pointer(), *slot));
            self.emit(abi::move_register(&abi::argument_register(index)?, "x9"));
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
            kind: "page21".to_string(),
            binding: "data".to_string(),
            library: None,
        });
        self.emit(abi::add_page_offset(register, register, &symbol));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: symbol,
            kind: "pageoff12".to_string(),
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
            kind: "page21".to_string(),
            binding: "data".to_string(),
            library: None,
        });
        self.emit(abi::add_page_offset(register, register, symbol));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: symbol.to_string(),
            kind: "pageoff12".to_string(),
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
        let mut arg_values = Vec::new();
        let mut arg_slots = Vec::new();
        self.reset_temporary_registers();
        for arg in args {
            let value = self.lower_value(arg)?;
            let slot = self.allocate_stack_object("runtime_thread_send_arg", 8);
            self.emit(abi::store_u64(&value.location, abi::stack_pointer(), slot));
            arg_values.push(value);
            arg_slots.push(slot);
            self.reset_temporary_registers();
        }

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
        self.emit(abi::load_u64("x9", abi::stack_pointer(), arg_slots[0]));
        self.emit(abi::load_u64("x10", "x9", arena_offset));
        self.emit(abi::move_register(ARENA_STATE_REGISTER, "x10"));
        self.error_arena_restore_slot = Some(saved_arena_slot);
        self.emit(abi::load_u64("x9", abi::stack_pointer(), arg_slots[1]));
        let copied = self.copy_value_to_current_arena(&arg_values[1].type_, "x9")?;
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
            "x9",
            abi::stack_pointer(),
            copied_message_slot,
        ));
        self.emit(abi::store_u64("x9", abi::stack_pointer(), arg_slots[1]));

        for (index, slot) in arg_slots.iter().enumerate() {
            self.emit(abi::load_u64("x9", abi::stack_pointer(), *slot));
            self.emit(abi::move_register(&abi::argument_register(index)?, "x9"));
        }
        self.emit_symbol_call(symbol);

        // An inline `TRAP` traps the raw send `Result`. On failure the sent value
        // remains owned by the caller (the typechecker restores the binding into
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
