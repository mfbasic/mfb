// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::target::shared::abi;
use crate::target::shared::nir::*;
impl CodeBuilder<'_> {
    pub(crate) fn emit_symbol_call(&mut self, symbol: &str) {
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

    /// The inline-`TRAP` raw-result capture label, when the current lowering sits
    /// inside an inline `TRAP` on a raw-supported builtin. Read by the collections
    /// callback members' failure exit (`emit_callback_failure_exit`, now in
    /// `codegen::builtins::collections::gen_flow`) without exposing the field.
    pub(crate) fn raw_result_capture_label(&self) -> Option<String> {
        self.raw_result_capture.clone()
    }

    /// plan-86 E: whether the enclosing `LET e = get(...)` binding is consumed
    /// read-only over an immutable container, so `get`'s result may alias the
    /// container's inline element instead of owning a copy. Read by
    /// `materialize_owned_element` (now in `codegen::memory`) without exposing the
    /// field.
    pub(crate) fn borrow_get_result(&self) -> bool {
        self.borrow_get_result
    }

    /// Record an internal (`binding: "internal"`, no library) call relocation from
    /// the current function to `to`, without emitting the branch itself. Extracted
    /// verbatim from `emit_map_probe`'s open-coded push so that helper can live in
    /// `codegen::builtins::collections` without reaching `CodeBuilder`'s
    /// private `relocations`/`current_symbol` fields. Byte-identical to the inline
    /// push it replaces (the caller still emits its own `branch_link`).
    pub(crate) fn push_internal_call_relocation(&mut self, to: &str) {
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: to.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
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
    pub(crate) fn emit_arena_alloc_call(&mut self) {
        self.emit_symbol_call(ARENA_ALLOC_SYMBOL);
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
    }

    /// Call `_mfb_arena_free` (block pointer in `x0`, size in `x1`).
    ///
    /// The free twin of `emit_arena_alloc_call` (bug-322); it has no result to
    /// compare, so there is no tag check. Same neutrality argument: an arena
    /// symbol is never a platform import, pinned by
    /// `arena_symbols_are_never_platform_imports`.
    pub(crate) fn emit_arena_free_call(&mut self) {
        self.emit_symbol_call(ARENA_FREE_SYMBOL);
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

    pub(crate) fn emit_raw_call(
        &mut self,
        symbol: &str,
        args: &[NirValue],
        slot_name: &str,
    ) -> Result<Vec<ValueResult>, String> {
        let arg_values = self.emit_prepared_call_args(args, slot_name)?;
        self.emit_symbol_call(symbol);
        Ok(arg_values)
    }

    pub(crate) fn load_empty_string_constant(&mut self) -> Result<VirtualRegister, String> {
        let register = self.allocate_register();
        self.emit_load_static_string_symbol(&register, EMPTY_STRING_SYMBOL);
        Ok(register)
    }

    pub(crate) fn load_string_constant(&mut self, value: &str) -> Result<VirtualRegister, String> {
        let register = self.allocate_register();
        self.emit_load_string_constant(&register, value)?;
        Ok(register)
    }

    pub(crate) fn emit_load_string_constant(
        &mut self,
        register: impl Into<Operand>,
        value: &str,
    ) -> Result<(), String> {
        let register = register.into();
        let symbol = self
            .string_symbols
            .get(value)
            .ok_or_else(|| format!("native code string literal '{value}' has no data object"))?
            .clone();
        self.emit(abi::load_page_address(register.clone(), &symbol));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: symbol.clone(),
            kind: RelocIntent::DataAddrHi,
            binding: "data".to_string(),
            library: None,
        });
        self.emit(abi::add_page_offset(register.clone(), register, &symbol));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: symbol,
            kind: RelocIntent::DataAddrLo,
            binding: "data".to_string(),
            library: None,
        });
        Ok(())
    }

    pub(crate) fn emit_load_static_string_symbol(
        &mut self,
        register: impl Into<Operand>,
        symbol: &str,
    ) {
        let register = register.into();
        self.emit(abi::load_page_address(register.clone(), symbol));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: symbol.to_string(),
            kind: RelocIntent::DataAddrHi,
            binding: "data".to_string(),
            library: None,
        });
        self.emit(abi::add_page_offset(register.clone(), register, symbol));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: symbol.to_string(),
            kind: RelocIntent::DataAddrLo,
            binding: "data".to_string(),
            library: None,
        });
    }

    pub(crate) fn emit_call(
        &mut self,
        target: &str,
        symbol: &str,
        args: &[NirValue],
        return_type: Option<&str>,
    ) -> Result<ValueResult, String> {
        let arg_values = self.emit_raw_call(symbol, args, "call_arg")?;
        let result_type = return_type
            .map(ParameterType::declared)
            .or_else(|| {
                self.functions
                    .get(target)
                    .map(|function| function.returns.clone())
            })
            .or_else(|| self.package_return_types.get(target).cloned())
            .unwrap_or(ParameterType::Unknown);
        if result_type == ParameterType::Nothing {
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
                origin: None,
                type_: result_type.clone(),
                location: Operand::from("void"),
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
        let register = self.allocate_register();
        self.emit(abi::move_register(&register, RESULT_VALUE_REGISTER));
        Ok(ValueResult {
            origin: None,
            type_: result_type.clone(),
            location: Operand::from(register.render()),
            text: format!("call {target}({})", join_texts(&arg_values)),
        })
    }

    /// Emit the indirect-call sequence for a function value: prepare the argument
    /// registers, save the caller's closure env, load the callee's code pointer and
    /// env from the closure object, install the callee env, `blr`, then restore the
    /// caller's env. Leaves the fallible result in the standard (tag, value,
    /// message, source) registers. Returns the rendered argument texts. Shared by
    /// the normal ([`Self::emit_function_value_call`]) and raw-capture
    /// ([`Self::emit_function_value_call_raw`]) paths so the two cannot drift.
    fn emit_function_value_invoke(
        &mut self,
        callable: &ValueResult,
        args: &[NirValue],
    ) -> Result<Vec<ValueResult>, String> {
        let arg_values = self.emit_prepared_call_args(args, "call_arg")?;
        let saved_env_slot = self.allocate_stack_object("closure_saved_env", 8);
        let code_register = self.allocate_register();
        let env_register = self.allocate_register();
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
        Ok(arg_values)
    }

    /// bug-448: invoke a function value under a raw capture (an inline `TRAP` on a
    /// function-value call). Emits the same indirect call as
    /// [`Self::emit_function_value_call`] but leaves the fallible result in the
    /// standard (tag, value, message, source) registers instead of checking the
    /// tag / extracting the value — so the caller materializes the boxed `Result`
    /// exactly as it does for a direct user-function call. Without this the trapped
    /// path treated the raw success *value* as a `Result`-object pointer and
    /// dereferenced it, segfaulting.
    pub(crate) fn emit_function_value_call_raw(
        &mut self,
        callable: &ValueResult,
        args: &[NirValue],
    ) -> Result<(), String> {
        self.emit_function_value_invoke(callable, args)?;
        Ok(())
    }

    pub(crate) fn emit_function_value_call(
        &mut self,
        target: &str,
        callable: &ValueResult,
        args: &[NirValue],
        return_type: Option<&str>,
    ) -> Result<ValueResult, String> {
        let arg_values = self.emit_function_value_invoke(callable, args)?;
        let result_type = return_type
            .map(ParameterType::declared)
            .unwrap_or(ParameterType::Unknown);
        // plan-120-E: the error-tag check is UNCONDITIONAL here, unlike
        // [`Self::emit_call`], where `return_type: Some(..)` doubles as "raw result
        // — the caller stores tag/value/message/source itself and handles the tag"
        // (`builder_values.rs`'s inline-TRAP machinery is its only `Some` caller).
        //
        // This function is the ORDINARY indirect-call path; the raw one is the
        // separate [`Self::emit_function_value_call_raw`]. But both of its callers
        // pass `Some(&return_type.name())` simply to communicate the callee's
        // declared return type, which — with the old `if return_type.is_none()`
        // guard copied from `emit_call` — silently suppressed propagation with
        // nobody materializing the `Result`.
        //
        // The effect was not a diagnostic. Per the fallible-call ABI, `x1` holds
        // the success value OR the error code, so an unchecked failing call
        // returned the CODE as the value: `apply(boom, 1)` with
        // `FUNC boom(v AS Integer) AS Integer` that FAILs returned `77050002`, and
        // for a pointer-typed return (`json::Json`) the caller dereferenced that
        // code and SIGSEGVed. Same callee called DIRECTLY by name trapped
        // correctly, which is what isolated it to this path. Cousin of bug-448,
        // fixed in the raw path next door.
        let emit_tag_check = |builder: &mut Self| -> Result<(), String> {
            let ok_label = builder.label("call_value_ok");
            builder.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
            builder.emit(abi::branch_eq(&ok_label));
            builder.emit_current_result_exit(builder.error_exit_destination())?;
            builder.emit(abi::label(&ok_label));
            Ok(())
        };
        if result_type == ParameterType::Nothing {
            emit_tag_check(self)?;
            for arg in args {
                self.maybe_deactivate_moved_thread_local(arg);
            }
            self.deactivate_moved_resource_arguments(target, args);
            return Ok(ValueResult {
                origin: None,
                type_: result_type.clone(),
                location: Operand::from("void"),
                text: format!("call {target}({})", join_texts(&arg_values)),
            });
        }
        emit_tag_check(self)?;
        for arg in args {
            self.maybe_deactivate_moved_thread_local(arg);
        }
        self.deactivate_moved_resource_arguments(target, args);
        let register = self.allocate_register();
        self.emit(abi::move_register(&register, RESULT_VALUE_REGISTER));
        Ok(ValueResult {
            origin: None,
            type_: result_type.clone(),
            location: Operand::from(register.render()),
            text: format!("call {target}({})", join_texts(&arg_values)),
        })
    }

    pub(crate) fn emit_runtime_helper_call(
        &mut self,
        target: &str,
        symbol: &str,
        args: &[NirValue],
        result_type: &ParameterType,
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

        if *result_type == ParameterType::Nothing {
            return Ok(ValueResult {
                origin: None,
                type_: result_type.clone(),
                location: Operand::from("void"),
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
            let register = self.allocate_register();
            self.emit(abi::move_register(&register, RESULT_VALUE_REGISTER));
            register
        };
        Ok(ValueResult {
            origin: None,
            type_: result_type.clone(),
            location: Operand::from(register.render()),
            text: format!("call {target}({})", join_texts(&arg_values)),
        })
    }

    /// Load the address of a string constant into the given register without
    /// allocating from the temporary-register pool.
    pub(crate) fn emit_load_string_address_into(
        &mut self,
        register: impl Into<Operand>,
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
        use crate::codegen::error::constants::{ARENA_ALLOC_SYMBOL, ARENA_FREE_SYMBOL};
        let plans = [
            (
                "linux_aarch64",
                include_str!("../../../target/linux_aarch64/plan.rs"),
            ),
            (
                "linux_x86_64",
                include_str!("../../../target/linux_x86_64/plan.rs"),
            ),
            (
                "linux_riscv64",
                include_str!("../../../target/linux_riscv64/plan.rs"),
            ),
            (
                "macos_aarch64",
                include_str!("../../../target/macos_aarch64/plan.rs"),
            ),
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
