//! bug-778: the frame form of an inline-`TRAP` desugar temporary.
//!
//! `LET x = f(a) TRAP(e) … END TRAP` desugars (`mfb spec language error-model`
//! §8.8) to `Bind $trap_resN : Result OF T = CallResult(f(a))` read only by
//! `ResultIsOk` / `ResultValue` / `ResultError`. The heap form of that
//! temporary is a `{tag, size, payload}` block allocated, filled, read and freed
//! on every evaluation, the successful ones included. Nothing can observe it:
//! it never escapes the statement, and on success only its tag and payload are
//! read. So a temporary whose uses are exactly those readers
//! (`unboxed_trap_result_candidates`) keeps the same facts in frame slots
//! ([`UnboxedTrapResult`]) and the success path allocates nothing the untrapped
//! expression does not.
//!
//! The error path still materializes the `Error` the handler binds, exactly as
//! `mfb spec memory fallible-call-abi` requires — it is the same adopt-or-rebuild
//! assembly ([`CodeBuilder::emit_trapped_error_block`]), minus the wrap.

use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::error::constants::*;
use crate::codegen::memory::arena::TrappedErrorSource;
use crate::target::shared::abi;
use crate::target::shared::nir::NirValue;
use crate::types::ParameterType;

/// Who owns the block a trapped producer's raw success value points at, as the
/// unboxed form needs to know it: take the block as it is, or take a copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UnboxedPayloadSource {
    /// This frame owns the producer's block outright and nothing else frees it:
    /// the temporary takes it (no copy). The same answer an owning `Bind` of the
    /// untrapped call reaches (`lower_value_owned`).
    Owned,
    /// The block belongs to someone else — a parameter borrow, a statement-scope
    /// temp already registered for its free, a rodata constant — so the
    /// temporary owns a copy, as an owning `Bind` of the untrapped value would.
    Copy,
}

impl CodeBuilder<'_> {
    /// Whether `Bind name : type_ = value` takes the unboxed form, and with what
    /// slots shape. `None` keeps the heap `Result` — every case not listed here,
    /// so an unforeseen shape costs an allocation, never correctness.
    ///
    /// The payload must be a value this form can own with the ordinary owned-value
    /// rules: a scalar word, or a flat block one `arena_free` reclaims. Resources
    /// (whose `TRAP` temps carry bug-648's lent-ownership flags), recursive graphs,
    /// nested `Result`s and callables keep the heap form, and so does
    /// `thread::waitFor`, whose error lives in the thread's block arena
    /// (`TrappedErrorSource::WorkerArena`).
    pub(crate) fn unboxed_trap_shape(
        &self,
        name: &str,
        type_: &ParameterType,
        value: &NirValue,
    ) -> Option<(UnboxedTrapUses, ParameterType, bool)> {
        let uses = *self.unboxed_trap_candidates.get(name)?;
        let ParameterType::ResultOf(payload) = type_ else {
            return None;
        };
        match value {
            NirValue::CallResult { target, .. } if target == "thread.waitFor" => return None,
            NirValue::CallResult { .. } | NirValue::Checked { .. } => {}
            _ => return None,
        }
        let payload = (**payload).clone();
        let block = self.unboxed_trap_payload_is_block(&payload)?;
        Some((uses, payload, block))
    }

    /// `Some(false)` for a scalar-word payload, `Some(true)` for an ownable flat
    /// block, `None` for anything the unboxed form does not take.
    fn unboxed_trap_payload_is_block(&self, payload: &ParameterType) -> Option<bool> {
        let scalar = matches!(
            payload,
            ParameterType::Boolean
                | ParameterType::Byte
                | ParameterType::Integer
                | ParameterType::Fixed
                | ParameterType::Float
                | ParameterType::Money
                | ParameterType::Nothing
        ) || *payload == ParameterType::named("Scalar")
            || self.is_enum_type(payload);
        if scalar {
            return Some(false);
        }
        let ownable_block = self.result_payload_is_block(payload)
            && self.is_freeable_flat_value(payload)
            && !self.owns_graph(payload)
            && !matches!(payload, ParameterType::ResultOf(_))
            && !Self::is_res_collection_block(payload)
            && !Self::is_res_marked_resource_collection(payload)
            && self.resource_cleanup_symbol(payload).is_none()
            && self.resource_union_cleanup(payload).is_none();
        ownable_block.then_some(true)
    }

    /// Lower `Bind name : Result OF T = value` in the unboxed form: arm the slots,
    /// let the trapped producer fill them, store the tag in the binding's own slot
    /// and register the drops for what the temporary may still own at scope end.
    pub(crate) fn lower_unboxed_trap_bind(
        &mut self,
        name: &str,
        value: &NirValue,
        stack_offset: usize,
        uses: UnboxedTrapUses,
        payload: ParameterType,
        block: bool,
    ) -> Result<(), String> {
        let payload_slot = self.allocate_stack_object("unboxed_trap_payload", 8);
        let error_slot = if uses.reads_error {
            self.allocate_stack_object("unboxed_trap_error", 8)
        } else {
            payload_slot
        };
        let unboxed = UnboxedTrapResult {
            tag_slot: stack_offset,
            payload_slot,
            error_slot,
            payload: payload.clone(),
            block,
            error_read: uses.reads_error,
        };
        // The owned slots read 0 on every path that does not store one — an
        // evaluation that fails, or a loop's next iteration — so their drops and
        // the readers' moves are sound (bug-246 / bug-440).
        let zero = self.temporary_vreg();
        if block {
            self.emit(abi::move_immediate(&zero, "Integer", "0"));
            self.emit(abi::store_u64(&zero, abi::stack_pointer(), payload_slot));
        }
        if uses.reads_error {
            self.emit(abi::move_immediate(&zero, "Integer", "0"));
            self.emit(abi::store_u64(&zero, abi::stack_pointer(), error_slot));
        }
        // plan-64-I: a conversion whose trapped error is provably unused builds
        // only a tag, exactly as on the heap path.
        if matches!(value, NirValue::CallResult { .. })
            && self.trap_discard_error_results.contains(name)
        {
            self.raw_result_discard_error = true;
        }
        self.unboxed_trap_filled = false;
        self.unboxed_trap_pending = Some(unboxed.clone());
        let lowered = self.lower_value(value);
        self.unboxed_trap_pending = None;
        self.raw_result_discard_error = false;
        let tag = lowered?;
        if !std::mem::take(&mut self.unboxed_trap_filled) {
            return Err(format!(
                "internal: the trapped producer of unboxed inline-TRAP temporary \
                 '{name}' did not fill its slots; every `CallResult`/`Checked` \
                 lowering must end in `finish_unboxed_trap_result`"
            ));
        }
        self.emit(abi::store_u64(
            &tag.location,
            abi::stack_pointer(),
            stack_offset,
        ));
        self.unboxed_trap_locals.insert(name.to_string(), unboxed);
        // A block the readers never moved out — a discarded `TRAP` has no
        // `ResultValue` — is still this temporary's, and so is an `Error` whose
        // read was skipped. Null after a move, so the drop frees each at most once.
        if block {
            self.register_unboxed_trap_slot_drop(payload, payload_slot);
        }
        if uses.reads_error {
            self.register_unboxed_trap_slot_drop(ParameterType::named("Error"), error_slot);
        }
        Ok(())
    }

    fn register_unboxed_trap_slot_drop(&mut self, type_: ParameterType, slot: usize) {
        self.active_cleanups
            .push(ActiveCleanup::OwnedValue(OwnedValueCleanup {
                type_,
                stack_offset: slot,
                capacity_slot: None,
                loop_alias_slot: None,
                result_wrapper: None,
            }));
        self.owned_value_slots.push(slot);
    }

    /// The trapped producer's tail in the unboxed form: the raw result registers
    /// are already spilled to `tag_slot`/`value_slot`/`message_slot`/`source_slot`.
    /// On `Ok` keep the payload (taking or copying a block per `owner`); on an
    /// error keep the flat `Error` the handler binds, or free it at once when no
    /// `ResultError` reads it. Yields the tag, which the `Bind` stores in the
    /// temporary's own slot.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn finish_unboxed_trap_result(
        &mut self,
        unboxed: UnboxedTrapResult,
        tag_slot: usize,
        value_slot: usize,
        message_slot: usize,
        source_slot: usize,
        owner: UnboxedPayloadSource,
        source: TrappedErrorSource,
        text: String,
    ) -> Result<ValueResult, String> {
        let scratch = self.temporary_vreg();
        let error_label = self.label("unboxed_trap_error");
        let done_label = self.label("unboxed_trap_done");
        self.emit(abi::load_u64(&scratch, abi::stack_pointer(), tag_slot));
        self.emit(abi::compare_immediate(&scratch, RESULT_OK_TAG));
        self.emit(abi::branch_ne(&error_label));
        // bug-744: a `Nothing` producer leaves the value register unset; the
        // reader produces `NOTHING` itself, so there is nothing to keep.
        if unboxed.payload != ParameterType::Nothing {
            let produced = self.allocate_register();
            self.emit(abi::load_u64(&produced, abi::stack_pointer(), value_slot));
            let kept = if unboxed.block && owner == UnboxedPayloadSource::Copy {
                self.copy_flat_block(&unboxed.payload, &produced)?
            } else {
                produced
            };
            self.emit(abi::store_u64(
                &kept,
                abi::stack_pointer(),
                unboxed.payload_slot,
            ));
        }
        self.emit(abi::branch(&done_label));
        self.emit(abi::label(&error_label));
        if self.raw_result_discard_error {
            // plan-64-I: `tag_slot` already holds `RESULT_ERR_TAG`; no `Error`
            // was assembled and none is read.
        } else if unboxed.error_read {
            self.emit_trapped_error_block(
                &scratch,
                tag_slot,
                value_slot,
                message_slot,
                source_slot,
                unboxed.error_slot,
                source,
            )?;
        } else {
            // Nothing reads the error, but adopting a parked block is still
            // what releases it from the current-error slot; then free it with
            // the heap path's parked-pointer guard (bug-565).
            let error_block = self.allocate_stack_object("unboxed_trap_error_block", 8);
            self.emit_trapped_error_block(
                &scratch,
                tag_slot,
                value_slot,
                message_slot,
                source_slot,
                error_block,
                source,
            )?;
            self.emit_free_trapped_error_payload(error_block)?;
        }
        self.emit(abi::label(&done_label));
        let tag = self.allocate_register();
        self.emit(abi::load_u64(&tag, abi::stack_pointer(), tag_slot));
        self.unboxed_trap_filled = true;
        Ok(ValueResult {
            origin: None,
            // The tag word, not a `Result` pointer: nothing may free or copy it.
            type_: ParameterType::Integer,
            location: Operand::from(tag),
            text,
        })
    }

    /// The unboxed temporary `value` names, when `value` is `Local(name)` of one.
    fn unboxed_trap_local(&self, value: &NirValue) -> Option<UnboxedTrapResult> {
        match value {
            NirValue::Local(name) => self.unboxed_trap_locals.get(name).cloned(),
            _ => None,
        }
    }

    /// `ResultIsOk` of an unboxed temporary: compare its tag slot.
    pub(crate) fn lower_unboxed_result_is_ok(
        &mut self,
        value: &NirValue,
    ) -> Option<Result<ValueResult, String>> {
        let unboxed = self.unboxed_trap_local(value)?;
        let tag = self.temporary_vreg();
        let register = self.allocate_register();
        let ok_label = self.label("result_is_ok_true");
        let end_label = self.label("result_is_ok_end");
        self.emit(abi::load_u64(&tag, abi::stack_pointer(), unboxed.tag_slot));
        self.emit(abi::compare_immediate(&tag, RESULT_OK_TAG));
        self.emit(abi::branch_eq(&ok_label));
        self.emit(abi::move_immediate(&register, "Boolean", "0"));
        self.emit(abi::branch(&end_label));
        self.emit(abi::label(&ok_label));
        self.emit(abi::move_immediate(&register, "Boolean", "1"));
        self.emit(abi::label(&end_label));
        Some(Ok(ValueResult {
            origin: None,
            type_: ParameterType::Boolean,
            location: Operand::from(register),
            text: "resultIsOk".to_string(),
        }))
    }

    /// `ResultValue` of an unboxed temporary: the scalar, or the owned block
    /// MOVED out (the slot is nulled, so the temporary's drop skips it). The one
    /// reader is an owning store (`unboxed_trap_result_candidates`), and
    /// `lower_value_owned` hands it the block without a copy
    /// ([`Self::unboxed_trap_read_moves`]).
    pub(crate) fn lower_unboxed_result_value(
        &mut self,
        value: &NirValue,
    ) -> Option<Result<ValueResult, String>> {
        let unboxed = self.unboxed_trap_local(value)?;
        let register = self.allocate_register();
        if unboxed.payload == ParameterType::Nothing {
            // bug-744: the one `Nothing` value is `NOTHING` (0).
            self.emit(abi::move_immediate(&register, "Integer", "0"));
        } else {
            self.emit(abi::load_u64(
                &register,
                abi::stack_pointer(),
                unboxed.payload_slot,
            ));
            if unboxed.block {
                self.emit(abi::store_u64(
                    abi::ZERO,
                    abi::stack_pointer(),
                    unboxed.payload_slot,
                ));
            }
        }
        Some(Ok(ValueResult {
            origin: None,
            type_: unboxed.payload,
            location: Operand::from(register),
            text: "resultValue".to_string(),
        }))
    }

    /// `ResultError` of an unboxed temporary: the owned flat `Error` block,
    /// MOVED out exactly like a block payload.
    pub(crate) fn lower_unboxed_result_error(
        &mut self,
        value: &NirValue,
    ) -> Option<Result<ValueResult, String>> {
        let unboxed = self.unboxed_trap_local(value)?;
        if !unboxed.error_read {
            // The candidate scan counted no `ResultError`, so no `error_slot`
            // was armed; reading one would hand out the payload slot.
            return Some(Err(
                "internal: an unboxed inline-TRAP temporary is read by a \
                 `ResultError` its candidate scan did not count"
                    .to_string(),
            ));
        }
        let register = self.allocate_register();
        self.emit(abi::load_u64(
            &register,
            abi::stack_pointer(),
            unboxed.error_slot,
        ));
        self.emit(abi::store_u64(
            abi::ZERO,
            abi::stack_pointer(),
            unboxed.error_slot,
        ));
        Some(Ok(ValueResult {
            origin: None,
            type_: ParameterType::named("Error"),
            location: Operand::from(register),
            text: "resultError".to_string(),
        }))
    }

    /// Whether `value` is a reader that MOVES a block out of an unboxed
    /// temporary, so an owning store takes it as it is: it is already this
    /// frame's own block and the temporary has given it up.
    pub(crate) fn unboxed_trap_read_moves(&self, value: &NirValue) -> bool {
        match value {
            NirValue::ResultValue { value } => self
                .unboxed_trap_local(value)
                .is_some_and(|unboxed| unboxed.block),
            NirValue::ResultError { value } => self.unboxed_trap_local(value).is_some(),
            _ => false,
        }
    }
}
