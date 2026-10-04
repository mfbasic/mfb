//! Callback / function-value control flow shared within the `collections`
//! package (A1 code motion out of `src/target`).
//!
//! The HOF members (`filter`/`forEach`/`transform`/`reduce`) and the native
//! source-generic fast paths (`sortBy`/`groupBy`/…) invoke a user-supplied
//! function value and route its failure. The census classified this machinery
//! **A1** — its only callers are collection lowerings (the lone `builder_values`
//! reference is a comment) — so it lives here beside them, not in shared target
//! codegen.
//!
//! These stay `impl CodeBuilder` methods (call sites unchanged); they call *down*
//! into the shared owned-value-drop tier (`emit_owned_value_drop` /
//! `OwnedValueCleanup`) and the register/closure constants, which remain in
//! `src/target` — the accepted temporary `codegen -> target` edge. The one change
//! from the verbatim bodies: `emit_callable_branch` takes a typed register and
//! emits only the indirect `blr` (its symbol arm had no caller), and
//! `emit_callback_failure_exit` reads
//! the inline-TRAP capture label through the `raw_result_capture_label` accessor,
//! so this module needs no access to `CodeBuilder`'s private fields.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::target::shared::abi::Reg;
use crate::types::ParameterType;
impl CodeBuilder<'_> {
    /// The exit a builtin's loop takes when a user callback it called fails: free
    /// `cleanup` (the member's partial result), then route the callee's error like
    /// any failed call. Under an inline `TRAP` that is the capture point; otherwise
    /// it is [`Self::emit_call_error_exit`] — the enclosing function-level `TRAP`,
    /// or a return to the caller that frees the live scope first.
    ///
    /// The second case used to be a bare `ret`: the function's `TRAP` never ran,
    /// its locals were never freed, and the partial result leaked
    /// (`tests/runtime/rt_callback_failure_reaches_trap.rs`).
    ///
    /// `item` is the loop item the member materialized to pass to the callback:
    /// a `String` element is a fresh block (`load_collection_loop_item`) that the
    /// success path frees after use, so the failure path must free it too — every
    /// failing call leaked it until bug-777. A fixed-width item owns nothing and
    /// adds no code. Every owned block goes in one register spill, because
    /// `arena_free` clobbers the four result registers.
    pub(crate) fn emit_callback_failure_exit(
        &mut self,
        cleanups: &[(usize, ParameterType)],
        item: Option<(usize, &ParameterType)>,
    ) -> Result<(), String> {
        let item = item.filter(|(_, element)| **element == ParameterType::String);
        if !cleanups.is_empty() || item.is_some() {
            let regs = [
                RESULT_TAG_REGISTER,
                RESULT_VALUE_REGISTER,
                RESULT_ERROR_MESSAGE_REGISTER,
                RESULT_ERROR_SOURCE_REGISTER,
            ];
            let slots: Vec<usize> = regs
                .iter()
                .map(|_| self.allocate_stack_object("callback_fail_result", 8))
                .collect();
            for (reg, slot) in regs.iter().zip(&slots) {
                self.emit(abi::store_u64(reg, abi::stack_pointer(), *slot));
            }
            if let Some((item_slot, element)) = item {
                self.free_collection_loop_item(item_slot, element)?;
            }
            for (block_slot, type_) in cleanups {
                self.emit_owned_value_drop(&OwnedValueCleanup {
                    type_: type_.clone(),
                    stack_offset: *block_slot,
                    capacity_slot: None,
                    loop_alias_slot: None,
                    result_wrapper: None,
                })?;
            }
            for (reg, slot) in regs.iter().zip(&slots) {
                self.emit(abi::load_u64(reg, abi::stack_pointer(), *slot));
            }
        }
        match self.raw_result_capture_label() {
            Some(label) => self.emit(abi::branch(&label)),
            None => self.emit_call_error_exit()?,
        }
        Ok(())
    }

    pub(crate) fn require_direct_callable(
        &self,
        name: &str,
        action: &ValueResult,
    ) -> Result<(), String> {
        if !matches!(action.type_, ParameterType::Func(_, _, false)) {
            return Err(format!(
                "native collection {name} action must be a function, got {}",
                action.type_
            ));
        }
        if action.location == "void" {
            return Err(format!(
                "native collection {name} action does not have a callable location"
            ));
        }
        Ok(())
    }

    pub(crate) fn emit_direct_callable_branch(&mut self, location: impl Into<Reg>) {
        let location: Reg = location.into();
        let saved_env_slot = self.allocate_stack_object("closure_saved_env", 8);
        let code_register = self.temporary_vreg();
        let env_register = self.temporary_vreg();
        self.emit(abi::store_u64(
            CLOSURE_ENV_REGISTER,
            abi::stack_pointer(),
            saved_env_slot,
        ));
        self.emit(abi::load_u64(&code_register, location, CLOSURE_OFFSET_CODE));
        self.emit(abi::load_u64(&env_register, location, CLOSURE_OFFSET_ENV));
        self.emit(abi::move_register(CLOSURE_ENV_REGISTER, &env_register));
        self.emit_callable_branch(&code_register);
        self.emit(abi::load_u64(
            CLOSURE_ENV_REGISTER,
            abi::stack_pointer(),
            saved_env_slot,
        ));
    }

    pub(crate) fn emit_callable_branch(&mut self, location: impl Into<Reg>) {
        // A callable held in a register is an indirect `blr`. The only caller
        // passes the closure's minted code register; a bare function symbol never
        // reaches here (bug-721: a register is a `Reg`, never text to sniff).
        self.emit(abi::branch_link_register(location));
    }
}
