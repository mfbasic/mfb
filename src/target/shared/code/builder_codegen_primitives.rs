use super::*;

impl CodeBuilder<'_> {
    pub(super) fn allocate_register(&mut self) -> Result<String, String> {
        // Mint a virtual register. The physical register is assigned after the
        // whole function is lowered (`regalloc::allocate`).
        let vreg = self.next_vreg;
        self.next_vreg += 1;
        debug_assert_eq!(self.vreg_eager.len(), vreg as usize);
        // Advance the bump counter for *both* strategies. Some lowerings advance
        // it as a positional reservation (`while self.next_register <= 12 { … }`
        // in `builder_numeric`), so it must always move or those loops never
        // terminate; linear-scan simply ignores the counter when coloring.
        let slot = self.next_register;
        self.next_register += 1;
        match self.regalloc_kind {
            regalloc::RegallocKind::BumpAndReset => {
                // Compute the bump allocator's eager physical now — both to drive
                // the byte-identical `BumpAndReset` replay (index == virtual
                // register number) and to mark its callee-saved use in the legacy
                // order so the frame layout is unchanged (plan-03 Stage A §4.1).
                let physical = abi::temporary_register(slot).map_err(|err| {
                    format!(
                        "{err} while lowering native function '{}'",
                        self.current_symbol
                    )
                })?;
                self.mark_register_used(&physical);
                self.vreg_eager.push(physical);
            }
            regalloc::RegallocKind::LinearScan => {
                // No eager physical: the liveness-driven coloring assigns physical
                // registers (or spill slots) after the whole function is lowered,
                // so a deep expression that would overflow the bump pool no longer
                // fails — it spills instead (plan-03 Stage B §4.4).
                self.vreg_eager.push(String::new());
            }
        }
        Ok(regalloc::vreg_name(vreg))
    }

    /// Mint a floating-point (`d`-class) virtual register (plan-03 Stage C). The
    /// physical `d`-register is assigned after the whole function is lowered;
    /// chained float arithmetic stays resident in `d`-registers instead of
    /// round-tripping its bit pattern through a GPR.
    pub(super) fn allocate_fp_register(&mut self) -> Result<String, String> {
        let vreg = self.next_fp_vreg;
        self.next_fp_vreg += 1;
        debug_assert_eq!(self.fp_vreg_eager.len(), vreg as usize);
        match self.regalloc_kind {
            regalloc::RegallocKind::BumpAndReset => {
                // The bump oracle replays a per-statement `d0`–`d7` sequence.
                let physical = abi::fp_temporary_register(self.next_fp_register).map_err(|err| {
                    format!(
                        "{err} while lowering native function '{}'",
                        self.current_symbol
                    )
                })?;
                self.next_fp_register += 1;
                self.fp_vreg_eager.push(physical);
            }
            regalloc::RegallocKind::LinearScan => {
                self.fp_vreg_eager.push(String::new());
            }
        }
        Ok(regalloc::fp_vreg_name(vreg))
    }

    /// Color the fully-lowered instruction stream: rewrite every virtual
    /// register to a physical register (or spill slot) using the selected
    /// strategy. Allocates frame slots for any spills and records the
    /// callee-saved registers the coloring used so `finalize_frame` saves them.
    /// Must run after the body is fully emitted and before the peephole pass and
    /// `finalize_frame`, which both expect physical register names (plan-03).
    pub(super) fn run_register_allocation(&mut self) {
        let model = crate::arch::aarch64::regmodel::Aarch64RegisterModel;
        let spill_base = self.stack_size;
        let outcome = regalloc::allocate(
            self.regalloc_kind,
            &mut self.instructions,
            &self.vreg_eager,
            &self.fp_vreg_eager,
            &model,
            spill_base,
        );
        for offset in &outcome.spill_slots {
            self.stack_slots.push(CodeStackSlot {
                name: format!("spill_{}", self.stack_slots.len()),
                type_: "spill".to_string(),
                offset: *offset as i32,
            });
        }
        self.stack_size = spill_base + outcome.spill_slots.len() * 8;
        for register in outcome.extra_callee_saved {
            if !self.used_callee_saved.iter().any(|saved| *saved == register) {
                self.used_callee_saved.push(register);
            }
        }
    }

    pub(super) fn mark_register_used(&mut self, register: &str) {
        if abi::is_callee_saved(register)
            && !self.used_callee_saved.iter().any(|saved| saved == register)
        {
            self.used_callee_saved.push(register.to_string());
        }
    }

    pub(super) fn reset_temporary_registers(&mut self) {
        self.next_register = 8;
        self.next_fp_register = 0;
    }

    pub(super) fn local_constants(&self) -> HashMap<String, Option<NirValue>> {
        self.locals
            .iter()
            .map(|(name, local)| (name.clone(), local.constant.clone()))
            .collect()
    }

    pub(super) fn restore_local_constants(
        &mut self,
        constants: &HashMap<String, Option<NirValue>>,
    ) {
        for (name, local) in &mut self.locals {
            local.constant = constants.get(name).cloned().unwrap_or(None);
        }
    }

    pub(super) fn clear_local_constants(&mut self) {
        for local in self.locals.values_mut() {
            local.constant = None;
        }
    }

    pub(super) fn allocate_stack_object(&mut self, name: &str, size: usize) -> usize {
        let offset = self.stack_size;
        let size = align(size, 8);
        self.stack_size += size;
        self.stack_slots.push(CodeStackSlot {
            name: format!("{name}_{}", self.stack_slots.len()),
            type_: name.to_string(),
            offset: offset as i32,
        });
        offset
    }

    pub(super) fn label(&mut self, prefix: &str) -> String {
        let label = format!("{prefix}_{}", self.next_label);
        self.next_label += 1;
        label
    }

    pub(super) fn emit(&mut self, instruction: CodeInstruction) {
        self.instructions.push(instruction);
    }

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
    /// Allocation-free: uses only the `x9` scratch register and stack slots, and
    /// returns the pointer in `x9`. Error-emitting paths are terminal, so they
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
            "x1",
            "Integer",
            &self.current_loc.line.to_string(),
        ));
        self.emit(abi::move_immediate(
            "x2",
            "Integer",
            &self.current_loc.column.to_string(),
        ));
        // Resolve the filename String pointer (an empty String when unknown, never
        // null — the helper dereferences it for the length) into the first arg.
        let filename = self.current_file.clone();
        if filename.is_empty() {
            let register = self.load_empty_string_constant()?;
            self.emit(abi::move_register("x0", &register));
        } else {
            self.emit_load_string_constant("x0", &filename)?;
        }
        self.emit(abi::branch_link(BUILD_ERROR_LOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: BUILD_ERROR_LOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::move_register("x9", abi::return_register()));
        Ok("x9".to_string())
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

        // message block size = len + 9 (message is never null).
        self.emit(abi::load_u64("x8", abi::stack_pointer(), message_slot));
        self.emit(abi::load_u64("x9", "x8", 0));
        self.emit(abi::add_immediate("x9", "x9", 9));
        self.emit(abi::store_u64("x9", abi::stack_pointer(), msg_block_slot));
        // source block size + offset: 0 (sentinel) when null, else its flat
        // ErrorLoc block size at the 8-aligned offset past the message block.
        self.emit(abi::load_u64("x8", abi::stack_pointer(), source_slot));
        self.emit(abi::compare_immediate("x8", "0"));
        self.emit(abi::branch_eq(&src_null_size));
        self.emit_record_block_size_to_slot("ErrorLoc", source_slot, src_block_slot)?;
        // src_off = align8(24 + msg_block)
        self.emit(abi::move_immediate(
            "x8",
            "Integer",
            &ERROR_OBJECT_SIZE.to_string(),
        ));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), msg_block_slot));
        self.emit(abi::add_registers("x8", "x8", "x9"));
        self.emit(abi::store_u64("x8", abi::stack_pointer(), src_off_slot));
        self.emit_align_offset_slot(src_off_slot, 8);
        // size = src_off + src_block
        self.emit(abi::load_u64("x8", abi::stack_pointer(), src_off_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), src_block_slot));
        self.emit(abi::add_registers("x8", "x8", "x9"));
        self.emit(abi::store_u64("x8", abi::stack_pointer(), size_slot));
        self.emit(abi::branch(&src_size_done));
        self.emit(abi::label(&src_null_size));
        // No source: offset sentinel 0, size = 24 + msg_block.
        self.emit(abi::move_immediate("x8", "Integer", "0"));
        self.emit(abi::store_u64("x8", abi::stack_pointer(), src_off_slot));
        self.emit(abi::move_immediate("x8", "Integer", "0"));
        self.emit(abi::store_u64("x8", abi::stack_pointer(), src_block_slot));
        self.emit(abi::move_immediate(
            "x8",
            "Integer",
            &ERROR_OBJECT_SIZE.to_string(),
        ));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), msg_block_slot));
        self.emit(abi::add_registers("x8", "x8", "x9"));
        self.emit(abi::store_u64("x8", abi::stack_pointer(), size_slot));
        self.emit(abi::label(&src_size_done));

        // Allocate the Error block.
        self.emit(abi::load_u64(
            abi::return_register(),
            abi::stack_pointer(),
            size_slot,
        ));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64("x1", abi::stack_pointer(), result_slot));
        // code @0.
        self.emit(abi::load_u64("x9", abi::stack_pointer(), code_slot));
        self.emit(abi::store_u64("x9", "x1", 0));
        // message-offset @8 = 24; inline message block at +24.
        self.emit(abi::move_immediate(
            "x9",
            "Integer",
            &ERROR_OBJECT_SIZE.to_string(),
        ));
        self.emit(abi::store_u64("x9", "x1", 8));
        self.emit(abi::add_immediate("x10", "x1", ERROR_OBJECT_SIZE));
        self.emit(abi::load_u64("x11", abi::stack_pointer(), message_slot));
        self.emit(abi::load_u64("x12", abi::stack_pointer(), msg_block_slot));
        self.emit_copy_bytes("x10", "x11", "x12", "error_msg_copy");
        // source-offset @16; inline source block when present.
        self.emit(abi::load_u64("x9", abi::stack_pointer(), src_off_slot));
        self.emit(abi::load_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::store_u64("x9", "x1", 16));
        self.emit(abi::load_u64("x8", abi::stack_pointer(), source_slot));
        self.emit(abi::compare_immediate("x8", "0"));
        self.emit(abi::branch_eq(&src_null_fill));
        self.emit(abi::load_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), src_off_slot));
        self.emit(abi::add_registers("x10", "x1", "x9"));
        self.emit(abi::load_u64("x11", abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64("x12", abi::stack_pointer(), src_block_slot));
        self.emit_copy_bytes("x10", "x11", "x12", "error_src_copy");
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
        self.emit(abi::load_u64("x9", abi::stack_pointer(), message_raw_slot));
        let copied_message = self.copy_value_to_current_arena("String", "x9")?;
        self.emit(abi::store_u64(
            &copied_message,
            abi::stack_pointer(),
            message_slot,
        ));
        // Deep-copy the worker source `ErrorLoc`, or stamp the call site if the
        // error originated in `waitFor` itself (no worker origin).
        let own = self.label("worker_error_own_origin");
        let done = self.label("worker_error_source_done");
        self.emit(abi::load_u64("x9", abi::stack_pointer(), source_raw_slot));
        self.emit(abi::compare_immediate("x9", "0"));
        self.emit(abi::branch_eq(&own));
        let copied_source = self.copy_value_to_current_arena("ErrorLoc", "x9")?;
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
        self.emit(abi::move_register("x3", code_register));
        self.emit(abi::move_immediate(
            "x1",
            "Integer",
            &self.current_loc.line.to_string(),
        ));
        self.emit(abi::move_immediate(
            "x2",
            "Integer",
            &self.current_loc.column.to_string(),
        ));
        self.emit_load_string_address_into("x4", message)?;
        // x0 = filename String pointer (empty String when the file is unknown).
        let filename = self.current_file.clone();
        if filename.is_empty() {
            let register = self.load_empty_string_constant()?;
            self.emit(abi::move_register("x0", &register));
        } else {
            self.emit_load_string_constant("x0", &filename)?;
        }
        self.emit(abi::branch_link(MAKE_ERROR_RESULT_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: MAKE_ERROR_RESULT_SYMBOL.to_string(),
            kind: "branch26".to_string(),
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
        // Inside a raw-capture region (inline `TRAP` on an inline built-in) the
        // error is not propagated: leave the raw `Result` in the standard
        // registers and join the capture point so it can be materialized.
        if let Some(label) = self.raw_result_capture.clone() {
            self.emit(abi::branch(&label));
        } else {
            self.emit(abi::return_());
        }
        Ok(())
    }

    fn ensure_pending_result_slots(&mut self) -> PendingResultSlots {
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

    fn store_pending_success_result(
        &mut self,
        value: Option<&ValueResult>,
        already_standalone: bool,
    ) -> Result<(), String> {
        let slots = self.ensure_pending_result_slots();
        let value_register = if let Some(value) = value {
            if value.type_ == "Nothing" {
                let register = self.allocate_register()?;
                self.emit(abi::move_immediate(&register, "Integer", "0"));
                register
            } else if !already_standalone
                && self.inline_collection_payload_size(&value.type_).is_some()
            {
                // A borrow / inline-payload return is promoted to a standalone
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
        self.emit(abi::move_immediate("x9", "Integer", RESULT_OK_TAG));
        self.emit(abi::store_u64("x9", abi::stack_pointer(), slots.tag));
        self.emit(abi::store_u64(
            &message_register,
            abi::stack_pointer(),
            slots.message,
        ));
        // Success results carry no error source.
        self.emit(abi::move_immediate("x9", "Integer", "0"));
        self.emit(abi::store_u64("x9", abi::stack_pointer(), slots.source));
        Ok(())
    }

    fn store_pending_error_registers(
        &mut self,
        code_register: &str,
        message_register: &str,
        source_register: &str,
    ) {
        let slots = self.ensure_pending_result_slots();
        self.emit(abi::store_u64(
            code_register,
            abi::stack_pointer(),
            slots.value,
        ));
        self.emit(abi::move_immediate("x9", "Integer", RESULT_ERR_TAG));
        self.emit(abi::store_u64("x9", abi::stack_pointer(), slots.tag));
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
    fn emit_load_error_fields(
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

    fn store_pending_error_from_value(&mut self, error: &NirValue) -> Result<(), String> {
        // The error's message/source are read as block-relative pointers, then
        // used after `emit_cleanup_sequence` frees this scope's owned values.
        // Deep-copy an aliasing-source error so those pointers reference a
        // standalone block that the frees cannot scrub (plan-02 Phase 8).
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
        self.store_pending_error_registers(&code_register, &message_register, &source_register);
        Ok(())
    }

    fn emit_direct_error_return(&mut self, error: &NirValue) -> Result<(), String> {
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
        self.emit(abi::move_immediate(
            RESULT_TAG_REGISTER,
            "Integer",
            RESULT_ERR_TAG,
        ));
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

    fn emit_direct_error_route_to_trap(&mut self, error: &NirValue) -> Result<(), String> {
        let error = self.lower_value(error)?;
        if error.type_ != "Error" {
            return Err(format!(
                "trap routing expects Error value, got `{}`",
                error.type_
            ));
        }
        let (stack_offset, label) = self
            .trap
            .as_ref()
            .and_then(|trap| {
                self.locals
                    .get(&trap.name)
                    .map(|local| (local.stack_offset, trap.label.clone()))
            })
            .ok_or_else(|| "trap routing requires bound trap local".to_string())?;
        self.emit(abi::store_u64(
            &error.location,
            abi::stack_pointer(),
            stack_offset,
        ));
        self.emit(abi::branch(&label));
        Ok(())
    }

    fn store_pending_current_result(&mut self) {
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

    fn load_pending_result_registers(&mut self) {
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

    pub(super) fn resource_cleanup_symbol(&self, type_: &str) -> Option<String> {
        let close = crate::builtins::resource_close_function(type_)?;
        let symbol = self
            .function_symbols
            .get(close)
            .cloned()
            .or_else(|| {
                runtime::helper_for_call(close)
                    .map(|helper| runtime::symbol_for_call(helper, close))
            })
            .unwrap_or_else(|| close.to_string());
        Some(symbol)
    }

    /// If `type_` is a resource union (every variant is a resource), the
    /// `(tag, close_symbol)` pairs for tag-dispatched drop; otherwise `None`.
    pub(super) fn resource_union_cleanup(&self, type_: &str) -> Option<Vec<(usize, String)>> {
        if !self.type_model.union_names.contains(type_) {
            return None;
        }
        let variants: Vec<String> = self.type_model.variants_for_union(type_).cloned().collect();
        if variants.is_empty() {
            return None;
        }
        let mut out = Vec::new();
        for variant in variants {
            if !crate::builtins::is_resource_type(&variant) {
                return None;
            }
            let tag = *self.type_model.union_variant_tags.get(&variant)?;
            let symbol = self.resource_cleanup_symbol(&variant)?;
            out.push((tag, symbol));
        }
        Some(out)
    }

    pub(super) fn deactivate_resource_cleanup(&mut self, name: &str) {
        if let Some(index) = self.active_cleanups.iter().rposition(|cleanup| {
            matches!(cleanup, ActiveCleanup::Resource(resource) if resource.name == name)
                || matches!(cleanup, ActiveCleanup::ResourceUnion(u) if u.name == name)
        }) {
            self.active_cleanups.remove(index);
        }
    }

    /// Tag-dispatched drop of a resource union: read the union tag and call the
    /// active variant's registered close op on its resource pointer (offset 8).
    pub(super) fn emit_resource_union_cleanup_call(
        &mut self,
        cleanup: &ResourceUnionCleanup,
    ) -> Result<(), String> {
        let stack_offset = match self.locals.get(&cleanup.name) {
            Some(local) => local.stack_offset,
            None => return Ok(()),
        };
        let union_ptr = self.allocate_register()?;
        self.emit(abi::load_u64(
            &union_ptr,
            abi::stack_pointer(),
            stack_offset,
        ));
        let union_slot = self.allocate_stack_object("resource_union_drop_ptr", 8);
        self.emit(abi::store_u64(&union_ptr, abi::stack_pointer(), union_slot));
        let tag_register = self.allocate_register()?;
        self.emit(abi::load_u64(&tag_register, &union_ptr, 0));
        let tag_slot = self.allocate_stack_object("resource_union_drop_tag", 8);
        self.emit(abi::store_u64(
            &tag_register,
            abi::stack_pointer(),
            tag_slot,
        ));
        let done = self.label("resource_union_drop_done");
        let payload_slot = self.allocate_stack_object("resource_union_drop_payload", 8);
        for (tag, symbol) in cleanup.variants.clone() {
            let next = self.label("resource_union_drop_next");
            let tag_reg = self.allocate_register()?;
            self.emit(abi::load_u64(&tag_reg, abi::stack_pointer(), tag_slot));
            self.emit(abi::compare_immediate(&tag_reg, &tag.to_string()));
            self.emit(abi::branch_ne(&next));
            // Load the variant's resource pointer (payload at offset 8) and close it.
            let base = self.allocate_register()?;
            self.emit(abi::load_u64(&base, abi::stack_pointer(), union_slot));
            let payload = self.allocate_register()?;
            self.emit(abi::load_u64(&payload, &base, 8));
            self.emit(abi::store_u64(&payload, abi::stack_pointer(), payload_slot));
            let arg = NirValue::Local(format!("__resource_union_payload@{payload_slot}"));
            self.locals.insert(
                format!("__resource_union_payload@{payload_slot}"),
                LocalValue {
                    type_: "File".to_string(),
                    stack_offset: payload_slot,
                    constant: None,
                    by_ref: false,
                },
            );
            self.emit_raw_call(
                &symbol,
                std::slice::from_ref(&arg),
                "resource_union_drop_arg",
            )?;
            let after = self.label("resource_union_drop_check");
            self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
            self.emit(abi::branch_eq(&after));
            self.record_secondary_cleanup_failure();
            self.emit(abi::label(&after));
            self.emit(abi::branch(&done));
            self.emit(abi::label(&next));
        }
        self.emit(abi::label(&done));
        Ok(())
    }

    pub(super) fn deactivate_moved_resource_arguments(&mut self, target: &str, args: &[NirValue]) {
        for (index, arg) in args.iter().enumerate() {
            let NirValue::Local(name) = arg else {
                continue;
            };
            let Some(local) = self.locals.get(name) else {
                continue;
            };
            let Some(close) = crate::builtins::resource_close_function(&local.type_) else {
                continue;
            };
            let consumed = if target == close {
                index == 0
            } else if matches!(
                target,
                "thread.start"
                    | "thread.send"
                    | "thread.emit"
                    | "thread.transferResource"
                    | "thread.emitResource"
            ) {
                // A thread-sendable resource is moved into the thread on a
                // successful transfer. Deactivation runs only on the success
                // path (after the result-tag branch), so the sender keeps
                // ownership and cleanup when the transfer fails with `Err`.
                index == 1 && crate::builtins::is_thread_sendable_resource_type(&local.type_)
            } else if crate::builtins::is_builtin_call(target) {
                false
            } else {
                // Ordinary user calls borrow the resource: the caller retains
                // ownership and its scope-drop cleanup. Only the fixed
                // invalidation events (registered close, thread transfer,
                // `RETURN`) hand off ownership.
                false
            };
            if consumed {
                self.deactivate_resource_cleanup(name);
            }
        }
    }

    pub(super) fn emit_resource_cleanup_call(
        &mut self,
        cleanup: &ResourceCleanup,
    ) -> Result<(), String> {
        let arg = NirValue::Local(cleanup.name.clone());
        self.emit_raw_call(
            &cleanup.symbol,
            std::slice::from_ref(&arg),
            "resource_drop_arg",
        )?;
        let done = self.label("resource_cleanup_done");
        self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
        self.emit(abi::branch_eq(&done));
        self.record_secondary_cleanup_failure();
        self.emit(abi::label(&done));
        Ok(())
    }

    fn record_secondary_cleanup_failure(&mut self) {
        self.emit(abi::load_u64(
            "x9",
            ARENA_STATE_REGISTER,
            ARENA_CLEANUP_FAILURE_COUNT_OFFSET,
        ));
        self.emit(abi::add_immediate("x9", "x9", 1));
        self.emit(abi::store_u64(
            "x9",
            ARENA_STATE_REGISTER,
            ARENA_CLEANUP_FAILURE_COUNT_OFFSET,
        ));
        self.emit(abi::store_u64(
            RESULT_VALUE_REGISTER,
            ARENA_STATE_REGISTER,
            ARENA_CLEANUP_FAILURE_CODE_OFFSET,
        ));
        self.emit(abi::store_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            ARENA_STATE_REGISTER,
            ARENA_CLEANUP_FAILURE_MESSAGE_OFFSET,
        ));
    }

    pub(super) fn emit_thread_cleanup_call(
        &mut self,
        cleanup: &ThreadCleanup,
    ) -> Result<(), String> {
        let arg = NirValue::Local(cleanup.name.clone());
        self.emit_raw_call(
            &cleanup.symbol,
            std::slice::from_ref(&arg),
            "thread_drop_arg",
        )?;
        Ok(())
    }

    pub(super) fn emit_thread_cleanup_for_name(&mut self, name: &str) -> Result<(), String> {
        let cleanup = ThreadCleanup {
            name: name.to_string(),
            symbol: Self::thread_drop_symbol(),
        };
        self.emit_thread_cleanup_call(&cleanup)
    }

    /// The close op symbol for a resource collection's element/value type, or an
    /// error if `type_` is not a collection whose element is a single resource.
    fn collection_resource_close_symbol(&self, type_: &str) -> Result<String, String> {
        let element = list_element_type(type_)
            .or_else(|| map_type_parts(type_).map(|(_, value)| value))
            .ok_or_else(|| format!("owned-list owner '{type_}' is not a collection"))?;
        self.resource_cleanup_symbol(&element).ok_or_else(|| {
            format!("owned-list element type '{element}' has no registered close op")
        })
    }

    /// Allocate and register a runtime owned-list for an owner collection binding
    /// (§15.6): a head-pointer stack slot (initialized empty) plus an
    /// [`ActiveCleanup::OwnedList`] obligation drained on every exit path.
    pub(super) fn setup_owned_list(&mut self, name: &str, type_: &str) -> Result<(), String> {
        let close_symbol = self.collection_resource_close_symbol(type_)?;
        let head_slot = self.allocate_stack_object(&format!("owned_list_{name}"), 8);
        self.emit(abi::move_immediate("x9", "Integer", "0"));
        self.emit(abi::store_u64("x9", abi::stack_pointer(), head_slot));
        self.owned_list_heads.insert(name.to_string(), head_slot);
        self.active_cleanups
            .push(ActiveCleanup::OwnedList(OwnedListCleanup {
                name: name.to_string(),
                head_slot,
                close_symbol,
            }));
        Ok(())
    }

    /// Transfer a returned resource collection's owned-list to the caller: drop
    /// its drain obligation from this scope so the resources are not closed here
    /// (the caller's scope adopts and closes them). Other scopes' owned-lists are
    /// untouched (§15.6).
    pub(super) fn deactivate_owned_list(&mut self, name: &str) {
        if let Some(index) = self
            .active_cleanups
            .iter()
            .rposition(|cleanup| matches!(cleanup, ActiveCleanup::OwnedList(o) if o.name == name))
        {
            self.active_cleanups.remove(index);
        }
    }

    /// Whether a NIR type string is a `RES`-marked resource collection
    /// (`List OF RES File`, `Map OF K TO RES File`): its scope-ownership transfers
    /// across a function boundary (§15.6).
    pub(super) fn is_res_marked_resource_collection(type_: &str) -> bool {
        type_
            .strip_prefix("List OF ")
            .is_some_and(|e| e.starts_with("RES "))
            || type_
                .strip_prefix("Map OF ")
                .and_then(|rest| rest.split_once(" TO "))
                .is_some_and(|(_, value)| value.starts_with("RES "))
    }

    /// Push the resource record at `resource_slot` onto `collection`'s owned-list
    /// as a fresh `{record, next}` node (§15.6).
    pub(super) fn emit_owned_list_push(
        &mut self,
        collection: &str,
        resource_slot: usize,
    ) -> Result<(), String> {
        let head_slot = *self
            .owned_list_heads
            .get(collection)
            .ok_or_else(|| format!("resource floats to '{collection}', which has no owned-list"))?;
        // Allocate a 16-byte node (record ptr at 0, next at 8).
        let alloc_ok = self.label("owned_list_alloc_ok");
        self.emit(abi::move_immediate(abi::return_register(), "Integer", "16"));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&alloc_ok));
        // x1 = node pointer.
        self.emit(abi::load_u64("x9", abi::stack_pointer(), resource_slot));
        self.emit(abi::store_u64("x9", "x1", 0));
        self.emit(abi::load_u64("x10", abi::stack_pointer(), head_slot));
        self.emit(abi::store_u64("x10", "x1", 8));
        self.emit(abi::store_u64("x1", abi::stack_pointer(), head_slot));
        Ok(())
    }

    /// Adopt the resources of a `List OF RES File` value transferred in from a
    /// call: walk the collection and push each element record onto this scope's
    /// owned-list, so the scope closes each once at exit (§15.6).
    pub(super) fn emit_owned_list_seed_from_collection(
        &mut self,
        collection: &str,
        collection_slot: usize,
        element_type: &str,
    ) -> Result<(), String> {
        let cursor_slot = self.allocate_stack_object("adopt_cursor", 8);
        let remaining_slot = self.allocate_stack_object("adopt_remaining", 8);
        let elem_slot = self.allocate_stack_object("adopt_elem", 8);
        self.initialize_collection_loop_slots(collection_slot, cursor_slot, remaining_slot);
        let loop_label = self.label("owned_list_seed_loop");
        let done_label = self.label("owned_list_seed_done");
        self.emit(abi::label(&loop_label));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), remaining_slot));
        self.emit(abi::compare_immediate("x9", "0"));
        self.emit(abi::branch_eq(&done_label));
        let item = self.load_collection_loop_item(collection_slot, cursor_slot, element_type)?;
        self.emit(abi::store_u64(&item, abi::stack_pointer(), elem_slot));
        self.emit_owned_list_push(collection, elem_slot)?;
        self.advance_collection_loop(cursor_slot, remaining_slot, &loop_label);
        self.emit(abi::label(&done_label));
        Ok(())
    }

    /// Drain an owned-list: walk it head-first, closing each record once. The
    /// close is closed-flag idempotent, so a record reachable through more than
    /// one path closes exactly once (§15.6).
    pub(super) fn emit_owned_list_drain(
        &mut self,
        cleanup: &OwnedListCleanup,
    ) -> Result<(), String> {
        let loop_label = self.label("owned_list_drain_loop");
        let done_label = self.label("owned_list_drain_done");
        let close_ok = self.label("owned_list_close_ok");
        self.emit(abi::label(&loop_label));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), cleanup.head_slot));
        self.emit(abi::compare_immediate("x9", "0"));
        self.emit(abi::branch_eq(&done_label));
        // Advance the head past this node before the call, which clobbers
        // caller-saved registers; the loop reloads the head from memory.
        self.emit(abi::load_u64(abi::return_register(), "x9", 0));
        self.emit(abi::load_u64("x10", "x9", 8));
        self.emit(abi::store_u64(
            "x10",
            abi::stack_pointer(),
            cleanup.head_slot,
        ));
        self.emit_symbol_call(&cleanup.close_symbol);
        self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
        self.emit(abi::branch_eq(&close_ok));
        self.record_secondary_cleanup_failure();
        self.emit(abi::label(&close_ok));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&done_label));
        Ok(())
    }

    /// Free an owned, non-escaping flat value at scope-drop (plan-01 Phase 5 /
    /// plan-02 Phase 8): recompute the block's byte size from its static type and
    /// `arena_free(ptr, size)`. `arena_free` scrubs the bytes (entropy poison),
    /// so a later use-after-free reads garbage and traps loudly. Clobbers
    /// caller-saved scratch; the caller reloads anything it needs afterward.
    pub(super) fn emit_owned_value_drop(
        &mut self,
        cleanup: &OwnedValueCleanup,
    ) -> Result<(), String> {
        // The slot is null when the binding's initializer trapped before it was
        // stored (the slot is zero-initialized at bind, see `lower_ops`), or for
        // a moved-out value; a null free would fault scrubbing address 0, so skip.
        let skip = self.label("owned_value_free_skip");
        self.emit(abi::load_u64(
            abi::return_register(),
            abi::stack_pointer(),
            cleanup.stack_offset,
        ));
        self.emit(abi::compare_immediate(abi::return_register(), "0"));
        self.emit(abi::branch_eq(&skip));
        let size_slot = self.allocate_stack_object("owned_value_free_size", 8);
        // The slot already holds the block pointer; size it from the type.
        self.emit_inlined_block_size_from_ptr_slot(
            &cleanup.type_,
            cleanup.stack_offset,
            size_slot,
        )?;
        self.emit(abi::load_u64(
            abi::return_register(),
            abi::stack_pointer(),
            cleanup.stack_offset,
        ));
        self.emit(abi::load_u64("x1", abi::stack_pointer(), size_slot));
        self.emit(abi::branch_link(ARENA_FREE_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_FREE_SYMBOL.to_string(),
            kind: "branch26".to_string(),
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::label(&skip));
        Ok(())
    }

    fn emit_cleanup_sequence(&mut self) -> Result<(), String> {
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
    fn trap_cleanup_floor(&self) -> usize {
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
    fn trap_route_cleanups(&self) -> Vec<ActiveCleanup> {
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
    fn emit_cleanups(&mut self, cleanups: &[ActiveCleanup]) -> Result<(), String> {
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
            self.emit_cleanups(&cleanups)?;
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
    /// caller, so it cannot remain a borrow into a local that is about to be
    /// freed. The bool is `already_standalone` — true when the result is a fresh
    /// standalone allocation (a copy made here) that must NOT be re-materialized;
    /// false for a fresh value or a borrow of a non-flat type, which keep the
    /// existing inline-payload materialization. A returned thread/resource local
    /// is a move (never freeable-flat) and is handled by cleanup deactivation.
    fn lower_returned_value(&mut self, value: &NirValue) -> Result<(ValueResult, bool), String> {
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

    pub(super) fn emit_return_exit(&mut self, value: Option<&NirValue>) -> Result<(), String> {
        let lowered = if let Some(value) = value {
            Some(self.lower_returned_value(value)?)
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
        self.emit_cleanup_sequence()?;
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
        let (stack_offset, label) = self
            .trap
            .as_ref()
            .and_then(|trap| {
                self.locals
                    .get(&trap.name)
                    .map(|local| (local.stack_offset, trap.label.clone()))
            })
            .ok_or_else(|| "trap routing requires bound trap local".to_string())?;

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
