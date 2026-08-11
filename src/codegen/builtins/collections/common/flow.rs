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
//! from the verbatim bodies: `emit_callable_branch`'s open-coded relocation push
//! is now `push_internal_call_relocation`, and `emit_callback_failure_exit` reads
//! the inline-TRAP capture label through the `raw_result_capture_label` accessor,
//! so this module needs no access to `CodeBuilder`'s private fields.

use crate::target::shared::abi;
use crate::target::shared::code::{
    regalloc, CodeBuilder, Operand, OwnedValueCleanup, ValueResult, CLOSURE_ENV_REGISTER,
    CLOSURE_OFFSET_CODE, CLOSURE_OFFSET_ENV, RESULT_ERROR_MESSAGE_REGISTER,
    RESULT_ERROR_SOURCE_REGISTER, RESULT_TAG_REGISTER, RESULT_VALUE_REGISTER,
};

impl CodeBuilder<'_> {
    pub(crate) fn emit_callback_failure_exit(
        &mut self,
        cleanup: Option<(usize, String)>,
    ) -> Result<(), String> {
        let Some(label) = self.raw_result_capture_label() else {
            self.emit(abi::return_());
            return Ok(());
        };
        if let Some((block_slot, type_)) = cleanup {
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
            self.emit_owned_value_drop(&OwnedValueCleanup {
                type_,
                stack_offset: block_slot,
                closure_captures: None,
            })?;
            for (reg, slot) in regs.iter().zip(&slots) {
                self.emit(abi::load_u64(reg, abi::stack_pointer(), *slot));
            }
        }
        self.emit(abi::branch(&label));
        Ok(())
    }

    pub(crate) fn require_direct_callable(
        &self,
        name: &str,
        action: &ValueResult,
    ) -> Result<(), String> {
        if !action.type_.starts_with("FUNC(") {
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

    pub(crate) fn emit_direct_callable_branch(&mut self, location: impl Into<Operand>) {
        let saved_env_slot = self.allocate_stack_object("closure_saved_env", 8);
        // Infallible vreg minters: an exhaustion under `-regalloc bump` is recorded
        // and surfaced by `run_register_allocation` instead of panicking (bug-70).
        let code_register = self.temporary_vreg();
        let env_register = self.temporary_vreg();
        let location = location.into();
        self.emit(abi::store_u64(
            CLOSURE_ENV_REGISTER,
            abi::stack_pointer(),
            saved_env_slot,
        ));
        self.emit(abi::load_u64(
            &code_register,
            location.clone(),
            CLOSURE_OFFSET_CODE,
        ));
        self.emit(abi::load_u64(
            &env_register,
            location.clone(),
            CLOSURE_OFFSET_ENV,
        ));
        self.emit(abi::move_register(CLOSURE_ENV_REGISTER, &env_register));
        self.emit_callable_branch(&code_register.render());
        self.emit(abi::load_u64(
            CLOSURE_ENV_REGISTER,
            abi::stack_pointer(),
            saved_env_slot,
        ));
    }

    pub(crate) fn emit_callable_branch(&mut self, location: &str) {
        // A callable held in a register (a physical `x*` or a not-yet-colored
        // virtual register) is an indirect `blr`; a bare function symbol is a
        // direct `bl` + relocation.
        if location.starts_with('x') || regalloc::parse_vreg(location).is_some() {
            self.emit(abi::branch_link_register(location));
            return;
        }
        self.emit(abi::branch_link(location));
        self.push_internal_call_relocation(location);
    }
}
