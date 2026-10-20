//! plan-176-P: building and materializing `Result` values, and the trapped-error result and its
//! payload. The result-tag classes live in `memory/arena/builder_arena_transfer/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// Build a flat `Result` value `{tag @0, size @8, payload @16}` (plan-02
    /// §4.3): a scalar payload occupies the 8-byte word at +16 (total 24 bytes); a
    /// block payload (`String`/record/union/collection/`Error`/nested `Result`) is
    /// inlined whole at +16, sized by `emit_inlined_block_size_from_ptr_slot`.
    /// `tag_slot` holds the active tag; `payload_slot` holds the scalar value or
    /// the block pointer. Returns a register with the Result pointer.
    pub(crate) fn emit_build_result_inline(
        &mut self,
        tag_slot: usize,
        payload_type: &ParameterType,
        payload_slot: usize,
    ) -> Result<VirtualRegister, String> {
        let is_block = self.result_payload_is_block(payload_type);
        let size_slot = self.allocate_stack_object("result_size", 8);
        let block_slot = self.allocate_stack_object("result_block", 8);
        let result_slot = self.allocate_stack_object("result_value", 8);
        let alloc_ok = self.label("result_inline_alloc_ok");
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        if is_block {
            self.emit_inlined_block_size_from_ptr_slot(payload_type, payload_slot, block_slot)?;
            self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), block_slot));
            self.emit(abi::add_immediate(&scratch8, &scratch8, 16));
            self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), size_slot));
        } else {
            self.emit(abi::move_immediate(&scratch8, "Integer", "24"));
            self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), size_slot));
        }
        // plan-71-C Family-1a: alloc size is arg 0 of the arena-alloc call → `%arg0`.
        self.emit(abi::load_u64(
            abi::c_arg(0),
            abi::stack_pointer(),
            size_slot,
        ));
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&alloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            result_slot,
        ));
        // tag @0, size @8.
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), tag_slot));
        self.emit(abi::store_u64(&scratch9, abi::mfb_return(1), 0));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), size_slot));
        self.emit(abi::store_u64(&scratch9, abi::mfb_return(1), 8));
        // payload @16.
        if is_block {
            self.emit(abi::add_immediate(&scratch10, abi::mfb_return(1), 16));
            self.emit(abi::load_u64(
                &scratch11,
                abi::stack_pointer(),
                payload_slot,
            ));
            self.emit(abi::load_u64(&scratch12, abi::stack_pointer(), block_slot));
            self.emit_copy_bytes(&scratch10, &scratch11, &scratch12, "result_payload_copy");
        } else {
            self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), payload_slot));
            self.emit(abi::load_u64(
                abi::mfb_return(1),
                abi::stack_pointer(),
                result_slot,
            ));
            self.emit(abi::store_u64(&scratch9, abi::mfb_return(1), 16));
        }
        let result = self.allocate_register();
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(result)
    }

    pub(crate) fn materialize_current_result(
        &mut self,
        success_type: &ParameterType,
        text: String,
        // When true (an inline-trapped `thread::waitFor`), the error's message and
        // origin live in the thread's block arena and arrive in x2/x3; they are deep-copied
        // into the caller arena. Otherwise the error originates at this inline
        // expression and its `ErrorLoc` is built from the current source location.
        worker_error_source: bool,
        // bug-566: who owns the block the raw success value points at. See
        // [`RawSuccessBlock`] — every call site answers explicitly.
        raw_success: RawSuccessBlock,
    ) -> Result<ValueResult, String> {
        let tag_slot = self.allocate_stack_object("raw_result_tag", 8);
        let value_slot = self.allocate_stack_object("raw_result_value", 8);
        let message_slot = self.allocate_stack_object("raw_result_message", 8);
        let source_raw_slot = self.allocate_stack_object("raw_result_source_raw", 8);
        let unboxed = self.unboxed_trap_active.take();
        if unboxed.is_none() {
            return self.materialize_current_result_boxed(
                success_type,
                text,
                worker_error_source,
                raw_success,
                [tag_slot, value_slot, message_slot, source_raw_slot],
            );
        }
        // bug-778: the unboxed form. Its eligibility (`unboxed_trap_shape`) keeps
        // a resource payload and `thread::waitFor`'s worker-arena error out.
        let unboxed = unboxed.expect("checked above");
        if worker_error_source || raw_success == RawSuccessBlock::BorrowedResource {
            return Err(format!(
                "internal: {text} reached the unboxed inline-TRAP form with a \
                 worker-arena error or a borrowed resource"
            ));
        }
        self.emit(abi::store_u64(
            RESULT_TAG_REGISTER,
            abi::stack_pointer(),
            tag_slot,
        ));
        self.emit(abi::store_u64(
            RESULT_VALUE_REGISTER,
            abi::stack_pointer(),
            value_slot,
        ));
        self.emit(abi::store_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            message_slot,
        ));
        self.emit(abi::store_u64(
            RESULT_ERROR_SOURCE_REGISTER,
            abi::stack_pointer(),
            source_raw_slot,
        ));
        // bug-841: `EXIT PROGRAM` (a HOF callback's, a worker's) is not the
        // handler's to catch.
        self.emit_inline_trap_program_exit_passthrough(tag_slot, value_slot)?;
        // `OwnedByThisFrame`: the producer handed back the only pointer to a block
        // in this arena — the temporary takes it, where the heap form copied it
        // and freed the original. `OwnedElsewhere`: another owner frees it (a
        // statement-scope temp, an operator's own registration), so the temporary
        // owns a copy, as the heap form's `Result` did.
        let owner = match raw_success {
            RawSuccessBlock::OwnedByThisFrame => UnboxedPayloadSource::Owned,
            _ => UnboxedPayloadSource::Copy,
        };
        self.finish_unboxed_trap_result(
            unboxed,
            tag_slot,
            value_slot,
            message_slot,
            source_raw_slot,
            owner,
            TrappedErrorSource::CurrentLocation,
            text,
        )
    }

    /// The heap-`Result` form of [`Self::materialize_current_result`], for every
    /// trapped value that is not a bug-778 unboxed temporary.
    fn materialize_current_result_boxed(
        &mut self,
        success_type: &ParameterType,
        text: String,
        worker_error_source: bool,
        raw_success: RawSuccessBlock,
        [tag_slot, value_slot, message_slot, source_raw_slot]: [usize; 4],
    ) -> Result<ValueResult, String> {
        // The single parse this function already performed for its result
        // type, hoisted so the block-free above shares it (plan-111-D). Letter F
        // types `success_type` itself and deletes it.
        let success = success_type.clone();
        let payload_slot = self.allocate_stack_object("raw_result_payload", 8);
        let result_slot = self.allocate_stack_object("raw_result", 8);
        let wrap_error_label = self.label("result_wrap_error");
        let have_payload_label = self.label("result_have_payload");
        let scratch9 = self.temporary_vreg();

        self.emit(abi::store_u64(
            RESULT_TAG_REGISTER,
            abi::stack_pointer(),
            tag_slot,
        ));
        self.emit(abi::store_u64(
            RESULT_VALUE_REGISTER,
            abi::stack_pointer(),
            value_slot,
        ));
        self.emit(abi::store_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            message_slot,
        ));
        self.emit(abi::store_u64(
            RESULT_ERROR_SOURCE_REGISTER,
            abi::stack_pointer(),
            source_raw_slot,
        ));
        // bug-841: `EXIT PROGRAM` (a HOF callback's, a worker's) is not the
        // handler's to catch — checked before the worker remap below folds every
        // non-`Ok` tag into `RESULT_ERR_TAG`.
        self.emit_inline_trap_program_exit_passthrough(tag_slot, value_slot)?;
        if worker_error_source {
            // A worker's error reaches `waitFor` with the WORKER's tag: `3` when the
            // worker parked an `Error` block — in its own arena, not in this
            // thread's current-error slot, which is empty. Adopting that slot
            // yielded a null `Error` and the wrap read through it (SIGSEGV on any
            // inline-trapped `waitFor` of a failing worker). The worker's message and
            // origin are in the loose registers, which the `WorkerArena` rebuild
            // deep-copies — so, as the untrapped path's
            // `emit_finalize_worker_error_source` does, every error tag here is the
            // loose one.
            let not_an_error = self.label("worker_result_ok");
            self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), tag_slot));
            self.emit(abi::compare_immediate(&scratch9, RESULT_OK_TAG));
            self.emit(abi::branch_eq(&not_an_error));
            self.emit(abi::move_immediate(&scratch9, "Integer", RESULT_ERR_TAG));
            self.emit(abi::store_u64(&scratch9, abi::stack_pointer(), tag_slot));
            self.emit(abi::label(&not_an_error));
        }
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), tag_slot));
        self.emit(abi::compare_immediate(&scratch9, RESULT_OK_TAG));
        self.emit(abi::branch_ne(&wrap_error_label));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), value_slot));
        // bug-643: the producer's resource record is already in THIS arena and
        // nothing else names it, so carry the pointer — exactly what the
        // non-sendable resource and `ThreadHandle` arms of
        // `copy_value_to_current_arena` already do, and for the same reason.
        //
        // Sending it through the copy instead allocated a second resource record,
        // copied the canonical header into it, and tombstoned the source
        // `moved|closed` (`copy_resource_to_current_arena`, the thread hand-over
        // path). In the same arena that source has no other owner and the moved bit
        // makes `emit_resource_block_reclaim` skip it, so every inline-`TRAP` bind
        // of a SENDABLE resource — `RES c = udp::bind(…) TRAP` and the resource
        // UNION spelling of the same bind, whose `$trap_val` temp is the concrete
        // variant — orphaned one record per bind. `RawSuccessBlock::OwnedByThisFrame`
        // is the audited answer to "is this record this frame's?"; a worker's record
        // (`thread.*`) keeps the deep copy. A borrowed element is carried as well
        // (bug-648): the copy's source flag tombstoned the element its list still
        // held, and the `TRAP` binding does not own the carried pointer
        // (`trap_ownership`).
        let carry_resource_pointer = match raw_success {
            RawSuccessBlock::OwnedByThisFrame => self.is_sendable_resource_nominal(success_type),
            RawSuccessBlock::BorrowedResource => true,
            RawSuccessBlock::OwnedElsewhere => false,
        };
        let copied_success = if raw_success == RawSuccessBlock::BorrowedResource
            && self.resource_union_cleanup(success_type).is_some()
        {
            // bug-1026: a borrowed resource-UNION element is an address inside its
            // collection's block (the collection holds the `{tag, record-ptr}` box
            // inline), or the box of `getOr`'s default, which this statement frees
            // at its end. The `TRAP` temp that receives it outlives both, so the
            // `Result` carries a copy of the two words: no record is copied and
            // nothing is tombstoned. The temp frees the copy and closes nothing
            // (`emit_resource_union_cleanup_call`, a lent store).
            self.materialize_inline_value_in_arena(success_type, scratch9)?
        } else if carry_resource_pointer {
            let carried = self.allocate_register();
            self.emit(abi::move_register(&carried, &scratch9));
            carried
        } else {
            self.copy_value_to_current_arena(success_type, &scratch9)?
        };
        self.emit(abi::store_u64(
            &copied_success,
            abi::stack_pointer(),
            payload_slot,
        ));
        let ok_result = self.emit_build_result_inline(tag_slot, success_type, payload_slot)?;
        self.emit(abi::store_u64(
            &ok_result,
            abi::stack_pointer(),
            result_slot,
        ));
        // bug-379: for a block success type, `emit_build_result_inline` above
        // deep-copies the payload into the freshly-allocated `Result`, so the
        // intermediate `copied_success` block is now dead. Free it here —
        // strictly after the inline copy completes — mirroring the ERR_BLOCK
        // path's `emit_free_error_block_from_slot`. Scalar success types have no
        // intermediate block (`copy_value_to_current_arena` returns a register),
        // so there is nothing to free.
        if self.result_payload_is_block(success_type) {
            self.emit_free_flat_block_from_slot(&success, payload_slot)?;
            // bug-566: and the PRODUCER's own block — step 1's source — which the
            // copy above made dead and which nothing else owned. This is the third
            // of the three lowerings that build a `Result` for an inline `TRAP`;
            // bug-561 fixed the other two by re-running `register_pending_temp` on
            // the node the bypass skipped, and left this one fail-closed because a
            // runtime helper's block carries no `mark_fresh_string` provenance and
            // `call_returns_fresh_string` only knows about `.mfb` / user functions.
            //
            // `RawSuccessBlock` supplies the missing half. Only a call site that has
            // audited its producer says `OwnedByThisFrame`; every other site — an
            // operator temp already registered for the statement-scope free, a
            // `.mfb` callee's block bug-561 already owns, a `thread::waitFor` result
            // that lives in the thread's block arena — says `OwnedElsewhere` and nothing
            // is emitted at all.
            if raw_success == RawSuccessBlock::OwnedByThisFrame {
                // The runtime guard: `copy_value_to_current_arena` is a deep copy
                // for every block shape reached here, so the two pointers differ —
                // but if any producer or copy path ever made it an identity, the
                // free above already released this block and freeing it again is a
                // double free. Compare and skip, so soundness is local to these
                // four instructions rather than resting on the copy's behaviour.
                let kept = self.label("raw_helper_result_kept");
                let produced = self.temporary_vreg();
                let copied = self.temporary_vreg();
                self.emit(abi::load_u64(&produced, abi::stack_pointer(), value_slot));
                self.emit(abi::load_u64(&copied, abi::stack_pointer(), payload_slot));
                self.emit(abi::compare_registers(&produced, &copied));
                self.emit(abi::branch_eq(&kept));
                // plan-134: the copy above is a whole new graph, so a recursive producer
                // frees its whole graph too.
                if self.owns_graph(&success) {
                    self.emit_graph_value_drop(&success, value_slot)?;
                } else {
                    self.emit_free_flat_block_from_slot(&success, value_slot)?;
                }
                self.emit(abi::label(&kept));
            }
        }
        self.emit(abi::branch(&have_payload_label));

        self.emit(abi::label(&wrap_error_label));
        if self.raw_result_discard_error {
            // plan-64-I: the trapped error is provably unused (the `RECOVER`
            // handler never reads `err`, so the IR omits `Bind err =
            // ResultError` — see `ir::lower::lower_inline_trap`). Build a bare
            // tag-only `Result`: no ErrorLoc, no flat `Error` block, no
            // adopt/copy/free. `tag_slot` already holds `RESULT_ERR_TAG` (set on
            // the discard path in `emit_error_register_return`); zero the scalar
            // payload for determinism. `ResultIsOk` reads the tag; the block is
            // freed by its stored `size` (+8), never a walk of the absent Error.
            self.emit(abi::move_immediate(&scratch9, "Integer", "0"));
            self.emit(abi::store_u64(
                &scratch9,
                abi::stack_pointer(),
                payload_slot,
            ));
            let bare =
                self.emit_build_result_inline(tag_slot, &ParameterType::Integer, payload_slot)?;
            self.emit(abi::store_u64(&bare, abi::stack_pointer(), result_slot));
        } else {
            self.emit_trapped_error_result(
                &scratch9,
                tag_slot,
                value_slot,
                message_slot,
                source_raw_slot,
                payload_slot,
                result_slot,
                if worker_error_source {
                    TrappedErrorSource::WorkerArena
                } else {
                    TrappedErrorSource::CurrentLocation
                },
            )?;
        }

        self.emit(abi::label(&have_payload_label));
        Ok(self.fresh_trapped_result_value(result_slot, success, text))
    }

    /// bug-568: the ONE constructor of a `Result`-typed [`ValueResult`], and the
    /// single place the claim "an inline `TRAP`'s value is a block THIS FRAME
    /// allocated" is made.
    ///
    /// `result_slot` holds what [`Self::emit_build_result_inline`] returned: a
    /// `{tag, size, payload}` block from this frame's own `_mfb_arena_alloc`,
    /// into which the payload — scalar or block, `Ok` or `Error` — has been
    /// COPIED. No other owner can name it.
    ///
    /// That matters because `lower_value_owned` decides whether a bind must deep
    /// copy by asking predicates about the CALLEE (`call_returns_param_borrow`,
    /// `call_returns_rodata_string`), and those describe the callee's SUCCESS
    /// value, not this wrapper. A callee whose `RETURN` names a parameter made
    /// `LET n AS Integer = risky(i) TRAP …` copy the whole `Result` and abandon
    /// the one it copied — 134 B per call, 26.8 MB at 200 000 and 52.6 MB at
    /// 400 000, on a call that never fails. Funnelling the construction here is
    /// what makes "every `Result`-typed value is fresh" a checkable statement
    /// rather than three separate sites agreeing by accident;
    /// `codegen_trap_result_wrapper.rs::the_result_wrapper_has_exactly_one_constructor`
    /// asserts nothing else builds one.
    pub(crate) fn fresh_trapped_result_value(
        &mut self,
        result_slot: usize,
        success: ParameterType,
        text: String,
    ) -> ValueResult {
        let register = self.allocate_register();
        self.emit(abi::load_u64(&register, abi::stack_pointer(), result_slot));
        ValueResult {
            origin: None,
            type_: ParameterType::result_of(success),
            location: Operand::from(register),
            text,
        }
    }

    /// The full error-payload assembly for an inline-`TRAP`'s error branch: put
    /// the trapped `Error` block in `payload_slot`, wrap a `Result` around it at
    /// `result_slot`, and then FREE the payload block, which the wrap has copied.
    ///
    /// Shared by all three lowerings that build a trapped `Result` — the direct
    /// user/`.mfb` callee and the indirect `FUNC`-value callee in
    /// `builder_values/lower_call.rs` (`lower_call_result_value`), and [`Self::materialize_current_result`] for a runtime
    /// helper or an inline builtin. They differ only in where the rebuild branch
    /// gets its `ErrorLoc`, which is [`TrappedErrorSource`].
    ///
    /// **bug-565.** Before this was shared, only the ADOPT branch of the
    /// `materialize_current_result` copy freed anything. The other three paths
    /// each leaked TWO arena blocks per trapped error: the flat `Error` block
    /// `emit_build_error_inline` builds (which [`Self::emit_build_result_inline`]
    /// deep-copies into the `Result`, after which nothing owns the original), and
    /// — on the two `builder_values/lower_call.rs` paths, which rebuilt unconditionally —
    /// the `ERR_BLOCK` block the raiser had already PARKED for the catcher to
    /// adopt, orphaned by the very next `FAIL` that overwrote the slot. A loop
    /// whose call fails every iteration grew 780 B per trapped error: 149.8 MB at
    /// 200 000 and 298.6 MB at 400 000.
    ///
    /// `mfb spec language memory-semantics` §14 preamble: "Each live value is
    /// owned by exactly one binding, container slot, temporary, closure
    /// environment, thread message, or return slot." §14.7 lists `FAIL` and
    /// auto-propagation among the scope edges at which live values are dropped.
    /// Both orphans had no owner at all; this names one — this expression's
    /// temporary — and drops it at the end of the branch that created it.
    ///
    /// **The guard.** Adding a free is the double-free direction, and the one
    /// block on this path that another owner may still hold is the PARKED
    /// `ERR_BLOCK`: it lives in the per-thread `ARENA_CURRENT_ERROR_OFFSET` slot
    /// until somebody adopts it, and the trap route
    /// (`route_current_result_to_trap`) adopts the same slot. So the free does not
    /// rest on this emitter's copy of "which tag parks a block" agreeing with the
    /// raiser's: [`Self::emit_free_trapped_error_payload`] compares the payload
    /// pointer against whatever is in that slot AT RUN TIME and frees only on
    /// difference. `emit_adopt_current_error_block` zeroes the slot as it hands
    /// the block over, so a real adoption always differs and is always freed; a
    /// payload that is still parked is never freed, however it got there. That is
    /// `collections::reduce`'s and bug-571's model — compare the produced pointer
    /// against the value you do not own — applied to the one block an inline
    /// `TRAP` does not exclusively own.
    pub(crate) fn emit_trapped_error_result(
        &mut self,
        // Reuse the caller's scratch vreg (rather than allocating a fresh one) so
        // this stays a pure extraction of the code it replaced.
        scratch9: impl Into<abi::Reg>,
        tag_slot: usize,
        value_slot: usize,
        message_slot: usize,
        source_raw_slot: usize,
        payload_slot: usize,
        result_slot: usize,
        source: TrappedErrorSource,
    ) -> Result<(), String> {
        self.emit_trapped_error_block(
            scratch9,
            tag_slot,
            value_slot,
            message_slot,
            source_raw_slot,
            payload_slot,
            source,
        )?;
        // One wrap for both branches: `emit_build_result_inline` deep-copies the
        // block at `payload_slot` into the fresh `Result`, so from here the
        // payload block — adopted or rebuilt — has no other owner.
        let err_result =
            self.emit_build_result_inline(tag_slot, &ParameterType::named("Error"), payload_slot)?;
        self.emit(abi::store_u64(
            &err_result,
            abi::stack_pointer(),
            result_slot,
        ));
        self.emit_free_trapped_error_payload(payload_slot)?;
        Ok(())
    }

    /// The first half of [`Self::emit_trapped_error_result`]: leave the trapped
    /// flat `Error` block in `payload_slot` — ADOPTED when the raiser parked one
    /// (the tag in `tag_slot` is then rewritten to the canonical `RESULT_ERR_TAG`),
    /// otherwise REBUILT from the registers with the `ErrorLoc` `source` names —
    /// and free whatever the rebuild allocated that the block inlined.
    ///
    /// The block is this frame's from here on (an adoption zeroes the parked slot
    /// as it hands it over). The heap `Result` path wraps and frees it; bug-778's
    /// unboxed trap result keeps it as is, or frees it with
    /// [`Self::emit_free_trapped_error_payload`] when nothing reads the error.
    pub(crate) fn emit_trapped_error_block(
        &mut self,
        scratch9: impl Into<abi::Reg>,
        tag_slot: usize,
        value_slot: usize,
        message_slot: usize,
        source_raw_slot: usize,
        payload_slot: usize,
        source: TrappedErrorSource,
    ) -> Result<(), String> {
        let scratch9 = scratch9.into();
        // Design "b": an `ERR_BLOCK` error already carries its single owned flat
        // Error block, parked in the current-error slot. ADOPT it as the payload
        // directly (no source rebuild) rather than rebuilding a fresh block from
        // the loose registers and orphaning the parked one. A legacy `ERR`, a
        // `PROGRAM_EXIT`, or a worker error (never block-carried) falls through to
        // the rebuild below.
        let rebuild_label = self.label("raw_result_rebuild");
        let err_built_label = self.label("raw_result_err_built");
        self.emit(abi::load_u64(scratch9, abi::stack_pointer(), tag_slot));
        self.emit(abi::compare_immediate(scratch9, adoptable_error_tag()));
        self.emit(abi::branch_ne(&rebuild_label));
        let adopted = self.emit_adopt_current_error_block();
        self.emit(abi::store_u64(&adopted, abi::stack_pointer(), payload_slot));
        // Store the canonical error tag into the `Result` value (not the raw
        // ERR_BLOCK ABI tag) so every `Result` inspection sees a uniform error tag.
        let err_tag = self.temporary_vreg();
        self.emit(abi::move_immediate(&err_tag, "Integer", RESULT_ERR_TAG));
        self.emit(abi::store_u64(&err_tag, abi::stack_pointer(), tag_slot));
        self.emit(abi::branch(&err_built_label));

        self.emit(abi::label(&rebuild_label));
        // Where the rebuild's `ErrorLoc` (and, for a worker error, its message)
        // comes from — and, for each, the pointer that would be the RAISER's
        // rather than ours. `*_borrowed_slot == *_slot` is the compile-time
        // statement "this component is the raiser's, we allocated nothing"; where
        // they differ the free below is guarded on the two pointers at run time.
        let mut message_borrowed_slot = message_slot;
        let source_slot = match source {
            // The callee's own `x3` origin, preserved verbatim so an inline-trapped
            // error keeps the source location it was raised at.
            TrappedErrorSource::CalleeRegister => source_raw_slot,
            TrappedErrorSource::CurrentLocation => {
                let source_slot = self.allocate_stack_object("raw_result_source", 8);
                let loc_register = self.emit_build_error_loc()?;
                self.emit(abi::store_u64(
                    &loc_register,
                    abi::stack_pointer(),
                    source_slot,
                ));
                source_slot
            }
            TrappedErrorSource::WorkerArena => {
                let source_slot = self.allocate_stack_object("raw_result_source", 8);
                // The worker's own message pointer, saved before the deep copy
                // overwrites `message_slot`, so the free below can tell the copy
                // this frame allocated from the worker block it was made from.
                let raw = self.allocate_stack_object("raw_result_message_raw", 8);
                self.emit(abi::load_u64(scratch9, abi::stack_pointer(), message_slot));
                self.emit(abi::store_u64(scratch9, abi::stack_pointer(), raw));
                message_borrowed_slot = raw;
                // A propagated worker error: deep-copy its message and origin out of
                // the thread's block arena (alive until this side lets go of the
                // handle) into the caller arena. If the helper
                // raised its own error (source == 0), stamp this inline expression.
                self.emit(abi::load_u64(scratch9, abi::stack_pointer(), message_slot));
                let copied_message =
                    self.copy_value_to_current_arena(&ParameterType::String, scratch9)?;
                self.emit(abi::store_u64(
                    &copied_message,
                    abi::stack_pointer(),
                    message_slot,
                ));
                let own = self.label("raw_worker_error_own");
                let done = self.label("raw_worker_error_done");
                self.emit(abi::load_u64(
                    scratch9,
                    abi::stack_pointer(),
                    source_raw_slot,
                ));
                self.emit(abi::compare_immediate(scratch9, "0"));
                self.emit(abi::branch_eq(&own));
                let copied_source =
                    self.copy_value_to_current_arena(&ParameterType::named("ErrorLoc"), scratch9)?;
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
                source_slot
            }
        };
        let error_register = self.emit_build_error_inline(value_slot, message_slot, source_slot)?;
        self.emit(abi::store_u64(
            &error_register,
            abi::stack_pointer(),
            payload_slot,
        ));
        // bug-565: the flat `Error` INLINES its message and its `ErrorLoc`, so any
        // component this frame allocated for the rebuild is dead the moment the
        // build returns. `emit_free_borrowed_guarded` declines when the pointer is
        // the raiser's; the two `if`s only skip emitting a compare that is
        // statically the same slot, and can never turn a decline into a free.
        if message_borrowed_slot != message_slot {
            self.emit_free_borrowed_guarded(
                &ParameterType::String,
                message_slot,
                message_borrowed_slot,
                "trapped_error_message_borrowed",
            )?;
        }
        if source_slot != source_raw_slot {
            self.emit_free_borrowed_guarded(
                &ParameterType::named("ErrorLoc"),
                source_slot,
                source_raw_slot,
                "trapped_error_source_borrowed",
            )?;
        }
        self.emit(abi::label(&err_built_label));
        Ok(())
    }

    /// bug-565: free the trapped `Error` block at `payload_slot`, UNLESS it is the
    /// block still parked in the per-thread current-error slot.
    ///
    /// The compare is the whole soundness argument, and it is exact rather than
    /// conservative: `emit_adopt_current_error_block` zeroes the slot in the same
    /// breath as it yields the base, so an adopted block can never still be the
    /// parked one; and a block `emit_build_error_inline` has just allocated can
    /// never equal a live parked pointer, because the arena hands out one address
    /// to one live block. What the compare buys is that the free stops resting on
    /// a compile-time claim about which tags park a block: if any future raiser
    /// reaches this branch with its block still parked, the free is declined and
    /// the block leaks — the fail-closed direction — instead of being freed twice.
    /// bug-565: free the flat block at `ptr_slot`, UNLESS it is null or is the
    /// pointer at `borrowed_slot` — the one this frame did NOT allocate.
    ///
    /// The two components an inline `TRAP`'s error rebuild can allocate — the
    /// `ErrorLoc` it stamps for itself, and (for a worker error) the caller-arena
    /// copy of the worker's message — are both INLINED into the flat `Error` by
    /// `emit_build_error_inline` and owned by nothing afterwards. The identical
    /// slots on the other source kinds hold the RAISER's blocks, which another
    /// owner still frees. Rather than deciding that from the variant, the compare
    /// asks it of the pointers: freeing is declined whenever the payload IS the
    /// value the caller named as not-ours. Fail-closed — an unforeseen path that
    /// hands the same pointer through leaks a block instead of freeing one twice.
    fn emit_free_borrowed_guarded(
        &mut self,
        block_type: &ParameterType,
        ptr_slot: usize,
        borrowed_slot: usize,
        label: &str,
    ) -> Result<(), String> {
        let skip = self.label(label);
        let owned = self.temporary_vreg();
        let borrowed = self.temporary_vreg();
        self.emit(abi::load_u64(&owned, abi::stack_pointer(), ptr_slot));
        // A degraded (OOM) `ErrorLoc` build yields the null sentinel, which
        // `emit_build_error_inline` writes as a source offset of 0; there is no
        // block to reclaim and `arena_free` would fault sizing address 0.
        self.emit(abi::compare_immediate(&owned, "0"));
        self.emit(abi::branch_eq(&skip));
        self.emit(abi::load_u64(
            &borrowed,
            abi::stack_pointer(),
            borrowed_slot,
        ));
        self.emit(abi::compare_registers(&owned, &borrowed));
        self.emit(abi::branch_eq(&skip));
        self.emit_free_flat_block_from_slot(block_type, ptr_slot)?;
        self.emit(abi::label(&skip));
        Ok(())
    }

    pub(crate) fn emit_free_trapped_error_payload(
        &mut self,
        payload_slot: usize,
    ) -> Result<(), String> {
        let parked_label = self.label("trapped_error_still_parked");
        let payload = self.temporary_vreg();
        let parked = self.temporary_vreg();
        let slot_address = self.current_error_slot_address();
        self.emit(abi::load_u64(&payload, abi::stack_pointer(), payload_slot));
        self.emit(abi::load_u64(&parked, &slot_address, 0));
        self.emit(abi::compare_registers(&payload, &parked));
        self.emit(abi::branch_eq(&parked_label));
        self.emit_free_error_block_from_slot(payload_slot)?;
        self.emit(abi::label(&parked_label));
        Ok(())
    }
}
