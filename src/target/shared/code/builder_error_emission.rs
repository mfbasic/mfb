use super::*;

impl CodeBuilder<'_> {
    pub(super) fn emit_overflow_return(&mut self) -> Result<(), String> {
        self.emit_error_code_return(ERR_OVERFLOW_CODE, ERR_OVERFLOW_MESSAGE)
    }

    pub(super) fn emit_underflow_return(&mut self) -> Result<(), String> {
        self.emit_error_code_return(ERR_UNDERFLOW_CODE, ERR_UNDERFLOW_MESSAGE)
    }

    pub(super) fn emit_float_domain_return(&mut self) -> Result<(), String> {
        self.emit_error_code_return(ERR_FLOAT_DOMAIN_CODE, ERR_FLOAT_DOMAIN_MESSAGE)
    }

    pub(super) fn emit_float_nan_return(&mut self) -> Result<(), String> {
        self.emit_error_code_return(ERR_FLOAT_NAN_CODE, ERR_FLOAT_NAN_MESSAGE)
    }

    pub(super) fn emit_float_inf_return(&mut self) -> Result<(), String> {
        self.emit_error_code_return(ERR_FLOAT_INF_CODE, ERR_FLOAT_INF_MESSAGE)
    }

    pub(super) fn emit_float_overflow_return(&mut self) -> Result<(), String> {
        self.emit_error_code_return(ERR_FLOAT_OVERFLOW_CODE, ERR_FLOAT_OVERFLOW_MESSAGE)
    }

    pub(super) fn emit_invalid_argument_return(&mut self) -> Result<(), String> {
        self.emit_error_code_return(ERR_INVALID_ARGUMENT_CODE, ERR_INVALID_ARGUMENT_MESSAGE)
    }

    pub(super) fn emit_invalid_format_return(&mut self) -> Result<(), String> {
        self.emit_error_code_return(ERR_INVALID_FORMAT_CODE, ERR_INVALID_FORMAT_MESSAGE)
    }

    pub(super) fn emit_allocation_error_return(&mut self) -> Result<(), String> {
        self.emit_error_register_return(RESULT_TAG_REGISTER, ERR_ALLOCATION_MESSAGE)
    }

    pub(super) fn emit_index_out_of_range_return(&mut self) -> Result<(), String> {
        self.emit_error_code_return(ERR_INDEX_OUT_OF_RANGE_CODE, ERR_INDEX_OUT_OF_RANGE_MESSAGE)
    }

    pub(super) fn emit_not_found_return(&mut self) -> Result<(), String> {
        self.emit_error_code_return(ERR_NOT_FOUND_CODE, ERR_NOT_FOUND_MESSAGE)
    }

    pub(super) fn emit_encoding_error_return(&mut self) -> Result<(), String> {
        self.emit_error_code_return(ERR_ENCODING_CODE, ERR_ENCODING_MESSAGE)
    }

    /// `product = lhs * rhs` for an allocation size, branching to `overflow`
    /// when the mathematical product does not fit in 64 bits (audit-unicode #1/
    /// #2/#8). The high half is computed first so `product` may alias `lhs` or
    /// `rhs`.
    pub(super) fn emit_checked_size_multiply(
        &mut self,
        product: &str,
        lhs: &str,
        rhs: &str,
        overflow: &str,
    ) {
        let high = self.temporary_vreg();
        self.emit(abi::unsigned_multiply_high_registers(&high, lhs, rhs));
        self.emit(abi::compare_immediate(&high, "0"));
        self.emit(abi::branch_ne(overflow));
        self.emit(abi::multiply_registers(product, lhs, rhs));
    }

    /// `dst = lhs + rhs` for an allocation size, branching to `overflow` on
    /// unsigned wrap. `dst` may alias `lhs` but must not alias `rhs` (the wrap
    /// test compares the sum against `rhs`).
    pub(super) fn emit_checked_size_add(
        &mut self,
        dst: &str,
        lhs: &str,
        rhs: &str,
        overflow: &str,
    ) {
        self.emit(abi::add_registers(dst, lhs, rhs));
        self.emit(abi::compare_registers(dst, rhs));
        self.emit(abi::branch_lo(overflow));
    }

    /// `dst = src + immediate` for an allocation size, branching to `overflow`
    /// on unsigned wrap. `dst` must not alias `src`.
    pub(super) fn emit_checked_size_add_immediate(
        &mut self,
        dst: &str,
        src: &str,
        immediate: usize,
        overflow: &str,
    ) {
        self.emit(abi::add_immediate(dst, src, immediate));
        self.emit(abi::compare_registers(dst, src));
        self.emit(abi::branch_lo(overflow));
    }

    /// Assert a two-pass writer ended exactly where its counting pass said it
    /// would (audit-unicode #9). A count/write divergence has already written
    /// past the allocation, so this cannot recover — it faults deterministically
    /// (null load) instead of letting the heap corruption propagate silently.
    pub(super) fn emit_write_cursor_assert(&mut self, cursor: &str, expected: &str, tag: &str) {
        let ok = self.label(&format!("{tag}_write_assert_ok"));
        self.emit(abi::compare_registers(cursor, expected));
        self.emit(abi::branch_eq(&ok));
        let zero = self.temporary_vreg();
        self.emit(abi::move_immediate(&zero, "Integer", "0"));
        self.emit(abi::load_u8(&zero, &zero, 0));
        self.emit(abi::label(&ok));
    }

    pub(super) fn emit_error_code_return(
        &mut self,
        code: &str,
        message: &str,
    ) -> Result<(), String> {
        let code_register = self.allocate_register()?;
        self.emit(abi::move_immediate(&code_register, "Integer", code));
        self.emit_error_register_return(&code_register, message)
    }

    /// Build an `ErrorLoc` record for the current source location and return a
    /// register holding its pointer. The pointer is left null only when the
    /// allocation itself fails (OOM), where no `ErrorLoc` could be allocated
    /// regardless. This never routes back through the error-return path, so it is
    /// safe to call from `emit_error_register_return`.
    /// Allocation-free: uses only a temporary scratch vreg and stack slots, and
    /// returns the pointer in that vreg. Error-emitting paths are terminal, so they
    /// must not consume the temporary-register pool (the surrounding expression
    /// may already be near the physical-register limit). Callers must save any
    /// live register inputs to the stack before invoking this.
    pub(super) fn emit_build_error_loc(&mut self) -> Result<String, String> {
        // `ErrorLoc` is a flat record `{filename(String) @0, line @8, char @16}`
        // (plan-02): the `filename` slot holds a block-relative offset to the
        // inlined `String` block. Construction is out-of-line in the shared
        // `_mfb_build_error_loc` helper (plan-16): ~48 inline instructions per
        // trap site collapse to passing `filename`/`line`/`char` and a call. The
        // helper returns a **null** pointer on OOM rather than propagating an
        // error (building an `ErrorLoc` happens *during* error handling, so a
        // propagated alloc error would recurse). Callers already treat this as
        // clobbering caller-saved registers (the former inline `arena_alloc` did),
        // so the contract is unchanged.
        self.emit(abi::move_immediate(
            abi::ARG[1],
            "Integer",
            &self.current_loc.line.to_string(),
        ));
        self.emit(abi::move_immediate(
            abi::ARG[2],
            "Integer",
            &self.current_loc.column.to_string(),
        ));
        // Resolve the filename String pointer (an empty String when unknown, never
        // null — the helper dereferences it for the length) into the first arg.
        let filename = self.current_file.clone();
        if filename.is_empty() {
            let register = self.load_empty_string_constant()?;
            self.emit(abi::move_register(abi::ARG[0], &register));
        } else {
            self.emit_load_string_constant(abi::ARG[0], &filename)?;
        }
        self.emit(abi::branch_link(BUILD_ERROR_LOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: BUILD_ERROR_LOC_SYMBOL.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
        let result = self.temporary_vreg();
        self.emit(abi::move_register(&result, abi::return_register()));
        Ok(result)
    }

    /// Build a flat `Error` value `{code @0, message(String offset) @8,
    /// source(ErrorLoc offset) @16, [inlined message][inlined source]}` from the
    /// raw code/message-pointer/source-pointer in the given stack slots (plan-02).
    /// `message` is always a valid String pointer; `source` may be **null** (an
    /// OOM-degraded error with no origin), represented by an offset-`0` sentinel
    /// (offset 0 can never address a real inlined block — the data region starts
    /// at 24). Propagates an allocation error like the previous fixed-size build.
    /// Returns a register holding the Error pointer.
    pub(super) fn emit_build_error_inline(
        &mut self,
        code_slot: usize,
        message_slot: usize,
        source_slot: usize,
    ) -> Result<String, String> {
        let msg_block_slot = self.allocate_stack_object("error_msg_block", 8);
        let src_block_slot = self.allocate_stack_object("error_src_block", 8);
        let src_off_slot = self.allocate_stack_object("error_src_off", 8);
        let size_slot = self.allocate_stack_object("error_size", 8);
        let result_slot = self.allocate_stack_object("error_result", 8);
        let src_null_size = self.label("error_src_null_size");
        let src_size_done = self.label("error_src_size_done");
        let alloc_ok = self.label("error_inline_alloc_ok");
        let src_null_fill = self.label("error_src_null_fill");
        let src_fill_done = self.label("error_src_fill_done");
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();

        // message block size = len + 9 (message is never null).
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), message_slot));
        self.emit(abi::load_u64(&scratch9, &scratch8, 0));
        self.emit(abi::add_immediate(&scratch9, &scratch9, 9));
        self.emit(abi::store_u64(
            &scratch9,
            abi::stack_pointer(),
            msg_block_slot,
        ));
        // source block size + offset: 0 (sentinel) when null, else its flat
        // ErrorLoc block size at the 8-aligned offset past the message block.
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), source_slot));
        self.emit(abi::compare_immediate(&scratch8, "0"));
        self.emit(abi::branch_eq(&src_null_size));
        self.emit_record_block_size_to_slot("ErrorLoc", source_slot, src_block_slot)?;
        // src_off = align8(24 + msg_block)
        self.emit(abi::move_immediate(
            &scratch8,
            "Integer",
            &ERROR_OBJECT_SIZE.to_string(),
        ));
        self.emit(abi::load_u64(
            &scratch9,
            abi::stack_pointer(),
            msg_block_slot,
        ));
        self.emit(abi::add_registers(&scratch8, &scratch8, &scratch9));
        self.emit(abi::store_u64(
            &scratch8,
            abi::stack_pointer(),
            src_off_slot,
        ));
        self.emit_align_offset_slot(src_off_slot, 8);
        // size = src_off + src_block
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), src_off_slot));
        self.emit(abi::load_u64(
            &scratch9,
            abi::stack_pointer(),
            src_block_slot,
        ));
        self.emit(abi::add_registers(&scratch8, &scratch8, &scratch9));
        self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), size_slot));
        self.emit(abi::branch(&src_size_done));
        self.emit(abi::label(&src_null_size));
        // No source: offset sentinel 0, size = 24 + msg_block.
        self.emit(abi::move_immediate(&scratch8, "Integer", "0"));
        self.emit(abi::store_u64(
            &scratch8,
            abi::stack_pointer(),
            src_off_slot,
        ));
        self.emit(abi::move_immediate(&scratch8, "Integer", "0"));
        self.emit(abi::store_u64(
            &scratch8,
            abi::stack_pointer(),
            src_block_slot,
        ));
        self.emit(abi::move_immediate(
            &scratch8,
            "Integer",
            &ERROR_OBJECT_SIZE.to_string(),
        ));
        self.emit(abi::load_u64(
            &scratch9,
            abi::stack_pointer(),
            msg_block_slot,
        ));
        self.emit(abi::add_registers(&scratch8, &scratch8, &scratch9));
        self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), size_slot));
        self.emit(abi::label(&src_size_done));

        // Allocate the Error block.
        self.emit(abi::load_u64(
            abi::return_register(),
            abi::stack_pointer(),
            size_slot,
        ));
        self.emit(abi::move_immediate(abi::ARG[1], "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(
            abi::RET[1],
            abi::stack_pointer(),
            result_slot,
        ));
        // code @0.
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), code_slot));
        self.emit(abi::store_u64(&scratch9, abi::RET[1], 0));
        // message-offset @8 = 24; inline message block at +24.
        self.emit(abi::move_immediate(
            &scratch9,
            "Integer",
            &ERROR_OBJECT_SIZE.to_string(),
        ));
        self.emit(abi::store_u64(&scratch9, abi::RET[1], 8));
        self.emit(abi::add_immediate(
            &scratch10,
            abi::RET[1],
            ERROR_OBJECT_SIZE,
        ));
        self.emit(abi::load_u64(
            &scratch11,
            abi::stack_pointer(),
            message_slot,
        ));
        self.emit(abi::load_u64(
            &scratch12,
            abi::stack_pointer(),
            msg_block_slot,
        ));
        self.emit_copy_bytes(&scratch10, &scratch11, &scratch12, "error_msg_copy");
        // source-offset @16; inline source block when present.
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), src_off_slot));
        self.emit(abi::load_u64(
            abi::RET[1],
            abi::stack_pointer(),
            result_slot,
        ));
        self.emit(abi::store_u64(&scratch9, abi::RET[1], 16));
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), source_slot));
        self.emit(abi::compare_immediate(&scratch8, "0"));
        self.emit(abi::branch_eq(&src_null_fill));
        self.emit(abi::load_u64(
            abi::RET[1],
            abi::stack_pointer(),
            result_slot,
        ));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), src_off_slot));
        self.emit(abi::add_registers(&scratch10, abi::RET[1], &scratch9));
        self.emit(abi::load_u64(&scratch11, abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64(
            &scratch12,
            abi::stack_pointer(),
            src_block_slot,
        ));
        self.emit_copy_bytes(&scratch10, &scratch11, &scratch12, "error_src_copy");
        self.emit(abi::branch(&src_fill_done));
        self.emit(abi::label(&src_null_fill));
        self.emit(abi::label(&src_fill_done));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(result)
    }

    /// Finalize a `thread::waitFor` error so it survives the worker arena being
    /// freed by the impending `thread.drop` cleanup. A propagated worker error
    /// arrives with its origin `ErrorLoc` in `x3` and its message in `x2`, both
    /// living in the worker arena which is still alive at this point — so they are
    /// deep-copied into the caller arena here. `waitFor`'s own errors arrive with
    /// `x3 == 0` (their message is a static string) and are stamped with this call
    /// site. All raw inputs are saved to the stack first because every copy/alloc
    /// clobbers the caller-saved registers.
    pub(super) fn emit_finalize_worker_error_source(&mut self) -> Result<(), String> {
        let code_slot = self.allocate_stack_object("worker_error_code", 8);
        let message_raw_slot = self.allocate_stack_object("worker_error_message_raw", 8);
        let source_raw_slot = self.allocate_stack_object("worker_error_source_raw", 8);
        let message_slot = self.allocate_stack_object("worker_error_message", 8);
        let source_slot = self.allocate_stack_object("worker_error_source", 8);
        let scratch9 = self.temporary_vreg();
        self.emit(abi::store_u64(
            RESULT_VALUE_REGISTER,
            abi::stack_pointer(),
            code_slot,
        ));
        self.emit(abi::store_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            message_raw_slot,
        ));
        self.emit(abi::store_u64(
            RESULT_ERROR_SOURCE_REGISTER,
            abi::stack_pointer(),
            source_raw_slot,
        ));
        // Deep-copy the message into the caller arena.
        self.emit(abi::load_u64(
            &scratch9,
            abi::stack_pointer(),
            message_raw_slot,
        ));
        let copied_message = self.copy_value_to_current_arena("String", &scratch9)?;
        self.emit(abi::store_u64(
            &copied_message,
            abi::stack_pointer(),
            message_slot,
        ));
        // Deep-copy the worker source `ErrorLoc`, or stamp the call site if the
        // error originated in `waitFor` itself (no worker origin).
        let own = self.label("worker_error_own_origin");
        let done = self.label("worker_error_source_done");
        self.emit(abi::load_u64(
            &scratch9,
            abi::stack_pointer(),
            source_raw_slot,
        ));
        self.emit(abi::compare_immediate(&scratch9, "0"));
        self.emit(abi::branch_eq(&own));
        let copied_source = self.copy_value_to_current_arena("ErrorLoc", &scratch9)?;
        self.emit(abi::store_u64(
            &copied_source,
            abi::stack_pointer(),
            source_slot,
        ));
        self.emit(abi::branch(&done));
        self.emit(abi::label(&own));
        let loc = self.emit_build_error_loc()?;
        self.emit(abi::store_u64(&loc, abi::stack_pointer(), source_slot));
        self.emit(abi::label(&done));
        self.emit(abi::move_immediate(
            RESULT_TAG_REGISTER,
            "Integer",
            RESULT_ERR_TAG,
        ));
        self.emit(abi::load_u64(
            RESULT_VALUE_REGISTER,
            abi::stack_pointer(),
            code_slot,
        ));
        self.emit(abi::load_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            message_slot,
        ));
        self.emit(abi::load_u64(
            RESULT_ERROR_SOURCE_REGISTER,
            abi::stack_pointer(),
            source_slot,
        ));
        // Design "b" origin (plan-error-block-in-slot stage 5): the worker's message
        // and origin have been deep-copied into the caller arena and stamped. Build
        // the single owned Error block (in the caller arena) and park it so the
        // catcher ADOPTS it, matching every other propagated error.
        self.emit_park_error_block_from_registers()?;
        Ok(())
    }

    /// Stamp the current source location into the error-source register for an
    /// error that a native runtime helper just returned in the standard error
    /// registers. The helper sets code (x1) and message (x2) but not the origin,
    /// so the call site (whose location is in `self.current_loc`) supplies it.
    /// The error code/message are preserved across the `ErrorLoc` allocation.
    pub(super) fn emit_stamp_current_error_source(&mut self) -> Result<(), String> {
        let code_slot = self.allocate_stack_object("error_source_code", 8);
        let message_slot = self.allocate_stack_object("error_source_message", 8);
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
        let loc_register = self.emit_build_error_loc()?;
        self.emit(abi::move_register(
            RESULT_ERROR_SOURCE_REGISTER,
            &loc_register,
        ));
        // Building the ErrorLoc allocates, which clobbers the tag register (x0):
        // re-assert the error tag along with the restored code/message.
        self.emit(abi::move_immediate(
            RESULT_TAG_REGISTER,
            "Integer",
            RESULT_ERR_TAG,
        ));
        self.emit(abi::load_u64(
            RESULT_VALUE_REGISTER,
            abi::stack_pointer(),
            code_slot,
        ));
        self.emit(abi::load_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            message_slot,
        ));
        let _ = loc_register;
        // Design "b" origin (plan-error-block-in-slot stage 5): a raw runtime helper
        // just returned this error in the loose registers and we have stamped its
        // origin. Build the single owned Error block and park it so the catcher
        // ADOPTS it (freed once) rather than rebuilding — the same funnel every
        // domain error uses.
        self.emit_park_error_block_from_registers()?;
        Ok(())
    }

    pub(super) fn emit_error_register_return(
        &mut self,
        code_register: &str,
        message: &str,
    ) -> Result<(), String> {
        // The whole error-Result assembly (build the ErrorLoc, then land
        // tag/value/message/source in the return registers) is out-of-line in
        // `_mfb_make_error_result` (plan-16): each trap site just loads the five
        // inputs and calls. Move the code to its arg slot (x3) first — the code
        // may currently live in one of the other arg registers (the allocation
        // path passes it in x0), so set it before x1/x2/x4/x0 are overwritten.
        self.emit(abi::move_register(abi::ARG[3], code_register));
        self.emit(abi::move_immediate(
            abi::ARG[1],
            "Integer",
            &self.current_loc.line.to_string(),
        ));
        self.emit(abi::move_immediate(
            abi::ARG[2],
            "Integer",
            &self.current_loc.column.to_string(),
        ));
        self.emit_load_string_address_into(abi::ARG[4], message)?;
        // x0 = filename String pointer (empty String when the file is unknown).
        let filename = self.current_file.clone();
        if filename.is_empty() {
            let register = self.load_empty_string_constant()?;
            self.emit(abi::move_register(abi::ARG[0], &register));
        } else {
            self.emit_load_string_constant(abi::ARG[0], &filename)?;
        }
        self.emit(abi::branch_link(MAKE_ERROR_RESULT_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: MAKE_ERROR_RESULT_SYMBOL.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
        if let Some(slot) = self.error_arena_restore_slot {
            self.emit(abi::load_u64(
                ARENA_STATE_REGISTER,
                abi::stack_pointer(),
                slot,
            ));
        }
        // Design "b" origin (plan-error-block-in-slot stage 4): `make_error_result`
        // has landed the loose error in the registers. Build the single owned Error
        // block once and park it so whoever catches this domain error ADOPTS it
        // (freed once) instead of rebuilding. Skip this on the OOM re-entry paths
        // (`building_error_block` = building a block itself failed to allocate;
        // `emitting_error_route` = the trap-route rebuild's own OOM fallback): there
        // is no memory to park a block, so those stay the loose `RESULT_ERR_TAG`
        // legacy path that the catcher rebuilds.
        if !self.building_error_block && !self.emitting_error_route {
            self.emit_park_error_block_from_registers()?;
        }
        // Inside a raw-capture region (inline `TRAP` on an inline built-in) the
        // error is not propagated: leave the raw `Result` in the standard
        // registers and join the capture point so it can be materialized. That
        // takes precedence over everything else.
        //
        // Otherwise route the freshly assembled error `Result` (now in the standard
        // registers) exactly like call-site auto-propagation: to the enclosing
        // function-level `TRAP` when one is active (bug-03 — an inline failure must
        // reach the bottom `TRAP`, same as `FAIL` and call-boundary failures, spec
        // §8.3), or back to the caller otherwise. `emit_current_result_exit` runs
        // the scope-drop walk with the trap-safe cleanup deferral (§8.1) so live
        // RES/owned values are freed exactly once; inside a `TRAP` body
        // (`in_trap_body`) `error_exit_destination` yields `Return`, so errors there
        // still propagate out (§8.6).
        if let Some(label) = self.raw_result_capture.clone() {
            self.emit(abi::branch(&label));
        } else if !self.emitting_error_route
            && matches!(self.error_exit_destination(), ExitDestination::Trap)
        {
            // Route this inline failure to the enclosing function-level `TRAP`. The
            // trap route builds an `Error` inline; its OOM fallback re-enters here
            // with the guard set, so it returns to the caller instead of recursing.
            self.emitting_error_route = true;
            self.emit_current_result_exit(ExitDestination::Trap)?;
            self.emitting_error_route = false;
        } else {
            self.emit(abi::return_());
        }
        Ok(())
    }

    pub(super) fn ensure_pending_result_slots(&mut self) -> PendingResultSlots {
        if let Some(slots) = self.pending_result_slots {
            return slots;
        }
        let slots = PendingResultSlots {
            value: self.allocate_stack_object("pending_result_value", 8),
            tag: self.allocate_stack_object("pending_result_tag", 8),
            message: self.allocate_stack_object("pending_result_message", 8),
            source: self.allocate_stack_object("pending_result_source", 8),
        };
        self.pending_result_slots = Some(slots);
        slots
    }

    pub(super) fn store_pending_success_result(
        &mut self,
        value: Option<&ValueResult>,
        already_standalone: bool,
    ) -> Result<(), String> {
        let slots = self.ensure_pending_result_slots();
        let scratch9 = self.temporary_vreg();
        let value_register = if let Some(value) = value {
            if value.type_ == "Nothing" {
                let register = self.allocate_register()?;
                self.emit(abi::move_immediate(&register, "Integer", "0"));
                register
            } else if !already_standalone
                && self.inline_collection_payload_size(&value.type_).is_some()
            {
                // An alias / inline-payload return is promoted to a standalone
                // arena block. A value already deep-copied by
                // `lower_returned_value` is standalone and skips this.
                self.materialize_inline_value_in_arena(&value.type_, &value.location)?
            } else {
                value.location.clone()
            }
        } else {
            let register = self.allocate_register()?;
            self.emit(abi::move_immediate(&register, "Integer", "0"));
            register
        };
        let message_register = self.allocate_register()?;
        self.emit(abi::move_immediate(&message_register, "Integer", "0"));
        self.emit(abi::store_u64(
            &value_register,
            abi::stack_pointer(),
            slots.value,
        ));
        self.emit(abi::move_immediate(&scratch9, "Integer", RESULT_OK_TAG));
        self.emit(abi::store_u64(&scratch9, abi::stack_pointer(), slots.tag));
        self.emit(abi::store_u64(
            &message_register,
            abi::stack_pointer(),
            slots.message,
        ));
        // Success results carry no error source.
        self.emit(abi::move_immediate(&scratch9, "Integer", "0"));
        self.emit(abi::store_u64(
            &scratch9,
            abi::stack_pointer(),
            slots.source,
        ));
        Ok(())
    }

    /// Address of the per-thread current-error slot in a fresh temporary register.
    /// The slot lives past the V128 arena-state region, beyond rv64's 12-bit `addi`
    /// immediate, so the offset is materialized in a register and added to the arena
    /// base rather than used as a load/store displacement (plan-error-block-in-slot).
    pub(super) fn current_error_slot_address(&mut self) -> String {
        let addr = self.temporary_vreg();
        self.emit(abi::move_immediate(
            &addr,
            "Integer",
            &ARENA_CURRENT_ERROR_OFFSET.to_string(),
        ));
        self.emit(abi::add_registers(&addr, ARENA_STATE_REGISTER, &addr));
        addr
    }

    /// Park an owned Error block base in the current-error slot so the catching
    /// trap route ADOPTS it (design "b"); pair with `RESULT_ERR_BLOCK_TAG`.
    pub(super) fn emit_store_current_error(&mut self, base_register: &str) {
        let addr = self.current_error_slot_address();
        self.emit(abi::store_u64(base_register, &addr, 0));
    }

    /// Adopt the owned Error block parked in the current-error slot: return a fresh
    /// register holding its base and clear the slot to 0, so the adopting consumer
    /// becomes the block's single owner and frees it exactly once (design "b").
    /// Only valid when the current result tag is `RESULT_ERR_BLOCK_TAG`.
    pub(super) fn emit_adopt_current_error_block(&mut self) -> String {
        let addr = self.current_error_slot_address();
        let base = self.temporary_vreg();
        self.emit(abi::load_u64(&base, &addr, 0));
        let zero = self.temporary_vreg();
        self.emit(abi::move_immediate(&zero, "Integer", "0"));
        self.emit(abi::store_u64(&zero, &addr, 0));
        base
    }

    /// Error-block origin funnel (plan-error-block-in-slot stages 4-5). The current
    /// result registers hold a loose error — `code`=`RESULT_VALUE_REGISTER`,
    /// `message*`=`RESULT_ERROR_MESSAGE_REGISTER`, `source*`=`RESULT_ERROR_SOURCE_REGISTER`
    /// (a null source is the no-origin sentinel). Build the single owned flat Error
    /// block once, park its base in the per-thread current-error slot, and set the
    /// tag to `RESULT_ERR_BLOCK_TAG` so whoever catches it ADOPTS the block instead
    /// of rebuilding a fresh one. The loose registers are re-loaded and left set
    /// (the block build's `arena_alloc` clobbers them) so the top-level exit printer
    /// and the OOM legacy path still read `code`/`message` from them.
    ///
    /// Guarded by `building_error_block`: building the block can itself hit OOM,
    /// whose fallback routes through `emit_allocation_error_return` →
    /// `emit_error_register_return`; that nested return must stay a loose
    /// `RESULT_ERR_TAG` (no memory to park a block), so the funnel suppresses a
    /// nested park while the flag is set.
    pub(super) fn emit_park_error_block_from_registers(&mut self) -> Result<(), String> {
        let code_slot = self.allocate_stack_object("park_error_code", 8);
        let message_slot = self.allocate_stack_object("park_error_message", 8);
        let source_slot = self.allocate_stack_object("park_error_source", 8);
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
        let previous = self.building_error_block;
        self.building_error_block = true;
        let base = self.emit_build_error_inline(code_slot, message_slot, source_slot)?;
        self.building_error_block = previous;
        self.emit_store_current_error(&base);
        // Restore the loose registers (the build's `arena_alloc` clobbered them) and
        // stamp the ERR_BLOCK tag.
        self.emit(abi::load_u64(
            RESULT_VALUE_REGISTER,
            abi::stack_pointer(),
            code_slot,
        ));
        self.emit(abi::load_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            message_slot,
        ));
        self.emit(abi::load_u64(
            RESULT_ERROR_SOURCE_REGISTER,
            abi::stack_pointer(),
            source_slot,
        ));
        self.emit(abi::move_immediate(
            RESULT_TAG_REGISTER,
            "Integer",
            RESULT_ERR_BLOCK_TAG,
        ));
        Ok(())
    }

    /// Free an owned flat Error block whose base is in `ptr_slot` (design "b"): size
    /// it from the `Error` type and `arena_free`. Used after an adopted block has
    /// been copied into a materialized `Result` value, so the adopted owner is
    /// released exactly once. `ptr_slot` must hold a non-null arena block base.
    pub(super) fn emit_free_error_block_from_slot(
        &mut self,
        ptr_slot: usize,
    ) -> Result<(), String> {
        let size_slot = self.allocate_stack_object("adopt_free_size", 8);
        self.emit_inlined_block_size_from_ptr_slot("Error", ptr_slot, size_slot)?;
        self.emit(abi::load_u64(
            abi::return_register(),
            abi::stack_pointer(),
            ptr_slot,
        ));
        self.emit(abi::load_u64(abi::ARG[1], abi::stack_pointer(), size_slot));
        self.emit_arena_free_call();
        Ok(())
    }

    pub(super) fn store_pending_error_registers(
        &mut self,
        code_register: &str,
        message_register: &str,
        source_register: &str,
        tag: &str,
    ) {
        let slots = self.ensure_pending_result_slots();
        let scratch9 = self.temporary_vreg();
        self.emit(abi::store_u64(
            code_register,
            abi::stack_pointer(),
            slots.value,
        ));
        self.emit(abi::move_immediate(&scratch9, "Integer", tag));
        self.emit(abi::store_u64(&scratch9, abi::stack_pointer(), slots.tag));
        self.emit(abi::store_u64(
            message_register,
            abi::stack_pointer(),
            slots.message,
        ));
        self.emit(abi::store_u64(
            source_register,
            abi::stack_pointer(),
            slots.source,
        ));
    }

    /// Load a flat `Error`'s `code`/`message`/`source` into the given registers
    /// for the fallible-call ABI (plan-02). `message`/`source` are stored as
    /// block-relative offsets, so the pointer is `errorBase + offset`; a `source`
    /// offset of `0` is the null sentinel (no origin) and yields a null pointer.
    /// `error_location` is preserved.
    pub(super) fn emit_load_error_fields(
        &mut self,
        error_location: &str,
        code_register: &str,
        message_register: &str,
        source_register: &str,
    ) {
        let src_null = self.label("error_read_src_null");
        let src_done = self.label("error_read_src_done");
        self.emit(abi::load_u64(code_register, error_location, 0));
        self.emit(abi::load_u64(message_register, error_location, 8));
        self.emit(abi::add_registers(
            message_register,
            error_location,
            message_register,
        ));
        self.emit(abi::load_u64(source_register, error_location, 16));
        self.emit(abi::compare_immediate(source_register, "0"));
        self.emit(abi::branch_eq(&src_null));
        self.emit(abi::add_registers(
            source_register,
            error_location,
            source_register,
        ));
        self.emit(abi::branch(&src_done));
        self.emit(abi::label(&src_null));
        self.emit(abi::label(&src_done));
    }

    pub(super) fn store_pending_error_from_value(
        &mut self,
        error: &NirValue,
    ) -> Result<(), String> {
        // The error's message/source are read as block-relative pointers, then
        // used after `emit_cleanup_sequence` frees this scope's owned values.
        // Deep-copy an aliasing-source error so those pointers reference a
        // standalone block that the frees cannot scrub (plan-02 Phase 8).
        //
        // `lower_value_owned` always yields a STANDALONE Error block this scope does
        // NOT register for cleanup: an aliasing source (`FAIL e` re-raising an owned
        // Error local) is deep-copied (Error is a flat record), and a fresh
        // `error(...)` is claimed out of the pending-temp set (plan-25, line ~138 of
        // `lower_value_owned`). Either way nobody else frees it — so park its base in
        // the per-thread current-error slot and tag `ERR_BLOCK` so the catching trap
        // route ADOPTS it (freed exactly once) instead of rebuilding a fresh block
        // and orphaning this one (design "b"). Parking unconditionally fixes bug-152
        // (re-raise) AND its cousin — a fresh `FAIL error(...)` with live cleanups,
        // whose standalone block was previously left on the legacy `ERR` path and
        // orphaned on every failure (a per-FAIL leak). The loose registers stay set
        // for the top-level exit printer and the OOM fallback.
        let error = self.lower_value_owned(error)?;
        if error.type_ != "Error" {
            return Err(format!(
                "cleanup error exit expects Error value, got `{}`",
                error.type_
            ));
        }
        let code_register = self.allocate_register()?;
        let message_register = self.allocate_register()?;
        let source_register = self.allocate_register()?;
        self.emit_load_error_fields(
            &error.location,
            &code_register,
            &message_register,
            &source_register,
        );
        self.emit_store_current_error(&error.location);
        self.store_pending_error_registers(
            &code_register,
            &message_register,
            &source_register,
            RESULT_ERR_BLOCK_TAG,
        );
        Ok(())
    }

    pub(super) fn emit_direct_error_return(&mut self, error: &NirValue) -> Result<(), String> {
        // A fresh Error block (e.g. `FAIL error(...)`) is a standalone owned block
        // that the FAIL's control transfer CLEARS (not frees) from the pending-temp
        // set, so it is safe to park it in the current-error slot and let whoever
        // catches it ADOPT the block (freed once), rather than propagating loose
        // interior pointers that force the catcher to rebuild — which orphaned this
        // block on every cross-call propagation (design "b"). An alias / aliasing
        // source is NOT owned here (`lower_value` returns another owner's pointer),
        // so it keeps the legacy loose-register path and the catcher rebuilds a copy.
        let adopt = !Self::value_is_aliasing_source(error);
        let error = self.lower_value(error)?;
        if error.type_ != "Error" {
            return Err(format!(
                "native code fail expects Error value, got `{}`",
                error.type_
            ));
        }
        let code_register = self.allocate_register()?;
        let message_register = self.allocate_register()?;
        let source_register = self.allocate_register()?;
        self.emit_load_error_fields(
            &error.location,
            &code_register,
            &message_register,
            &source_register,
        );
        self.emit(abi::move_register(RESULT_VALUE_REGISTER, &code_register));
        let tag = if adopt {
            self.emit_store_current_error(&error.location);
            RESULT_ERR_BLOCK_TAG
        } else {
            RESULT_ERR_TAG
        };
        self.emit(abi::move_immediate(RESULT_TAG_REGISTER, "Integer", tag));
        self.emit(abi::move_register(
            RESULT_ERROR_MESSAGE_REGISTER,
            &message_register,
        ));
        self.emit(abi::move_register(
            RESULT_ERROR_SOURCE_REGISTER,
            &source_register,
        ));
        self.emit(abi::return_());
        Ok(())
    }

    pub(super) fn emit_direct_error_route_to_trap(
        &mut self,
        error: &NirValue,
    ) -> Result<(), String> {
        let error = self.lower_value(error)?;
        if error.type_ != "Error" {
            return Err(format!(
                "trap routing expects Error value, got `{}`",
                error.type_
            ));
        }
        // Pinned trap-local slot from `TrapState` (see `route_current_result_to_trap`):
        // an inline `TRAP(e)` rebinds the shared name `e`, so `self.locals[name]`
        // is not a reliable source for the function-level trap slot (bug-148).
        let (stack_offset, label) = self
            .trap
            .as_ref()
            .map(|trap| (trap.stack_offset, trap.label.clone()))
            .ok_or_else(|| "trap routing requires bound trap local".to_string())?;
        self.emit(abi::store_u64(
            &error.location,
            abi::stack_pointer(),
            stack_offset,
        ));
        self.emit(abi::branch(&label));
        Ok(())
    }

    pub(super) fn store_pending_current_result(&mut self) {
        let slots = self.ensure_pending_result_slots();
        self.emit(abi::store_u64(
            RESULT_VALUE_REGISTER,
            abi::stack_pointer(),
            slots.value,
        ));
        self.emit(abi::store_u64(
            RESULT_TAG_REGISTER,
            abi::stack_pointer(),
            slots.tag,
        ));
        self.emit(abi::store_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            slots.message,
        ));
        self.emit(abi::store_u64(
            RESULT_ERROR_SOURCE_REGISTER,
            abi::stack_pointer(),
            slots.source,
        ));
    }

    pub(super) fn load_pending_result_registers(&mut self) {
        let slots = self
            .pending_result_slots
            .expect("pending result slots must exist before loading");
        self.emit(abi::load_u64(
            RESULT_VALUE_REGISTER,
            abi::stack_pointer(),
            slots.value,
        ));
        self.emit(abi::load_u64(
            RESULT_TAG_REGISTER,
            abi::stack_pointer(),
            slots.tag,
        ));
        self.emit(abi::load_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            slots.message,
        ));
        self.emit(abi::load_u64(
            RESULT_ERROR_SOURCE_REGISTER,
            abi::stack_pointer(),
            slots.source,
        ));
    }

    pub(super) fn error_exit_destination(&self) -> ExitDestination {
        if self.trap.as_ref().is_some_and(|trap| !trap.in_trap_body) {
            ExitDestination::Trap
        } else {
            ExitDestination::Return
        }
    }
}
