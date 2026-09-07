// --- codegen tier imports (migration) ---
use crate::codegen::collection::layout::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::types::ParameterType;
use std::collections::HashMap;

/// How much to copy for a resource live slot that points into the sender's arena
/// (bug-464): a size the descriptor knows at compile time, or a NUL-terminated
/// string measured at run time.
#[derive(Clone, Copy)]
enum BlockLength {
    Fixed(usize),
    CString,
}

/// bug-565: what a raw result tag says about who owns the `Error` behind it, at
/// the moment an inline `TRAP` assembles its `Result`.
///
/// The trapped-error assembly ADOPTS on one tag and REBUILDS on the others, and
/// then frees what it ended up holding. Getting that partition wrong in the
/// adopt direction is a double free, so it is written once, here, as a total
/// function over the tag vocabulary — `tests::the_result_tag_partition_is_total`
/// pins it against the `RESULT_*_TAG` constants actually declared in
/// `error_constants.rs`, so adding a fifth tag reds a test instead of silently
/// falling into `LooseRegisters`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TrappedErrorTagClass {
    /// `RESULT_OK_TAG` — not an error at all; the wrap-error branch is not taken.
    NotAnError,
    /// `RESULT_ERR_BLOCK_TAG` — the raiser built ONE owned flat `Error` block and
    /// PARKED it in the per-thread current-error slot for the catcher to adopt
    /// (design "b"). Rebuilding here orphans it, which is half of bug-565.
    ParkedBlock,
    /// `RESULT_ERR_TAG` (a legacy/OOM-degraded loose error) and
    /// `RESULT_PROGRAM_EXIT_TAG` (`os::exit` unwinding through a fallible call).
    /// No block is parked; the flat `Error` must be rebuilt from the registers.
    LooseRegisters,
}

/// The class of `tag`, which must be one of the `RESULT_*_TAG` constants.
pub(crate) fn trapped_error_tag_class(tag: &str) -> TrappedErrorTagClass {
    match tag {
        RESULT_OK_TAG => TrappedErrorTagClass::NotAnError,
        RESULT_ERR_BLOCK_TAG => TrappedErrorTagClass::ParkedBlock,
        RESULT_ERR_TAG | RESULT_PROGRAM_EXIT_TAG => TrappedErrorTagClass::LooseRegisters,
        // Unreachable for the declared vocabulary (the test above proves the
        // vocabulary is exactly those four). An unknown tag is treated as a loose
        // error: it REBUILDS, which leaks at worst, rather than adopting a slot
        // nobody parked and freeing a block someone else owns.
        _ => TrappedErrorTagClass::LooseRegisters,
    }
}

/// The one tag whose error arrives as a parked, adoptable block. Derived from the
/// partition above rather than spelled again at the emitter, so the compare the
/// generated code performs cannot drift from the classification this file
/// documents.
fn adoptable_error_tag() -> &'static str {
    for tag in [
        RESULT_OK_TAG,
        RESULT_ERR_TAG,
        RESULT_PROGRAM_EXIT_TAG,
        RESULT_ERR_BLOCK_TAG,
    ] {
        if trapped_error_tag_class(tag) == TrappedErrorTagClass::ParkedBlock {
            return tag;
        }
    }
    unreachable!("the partition declares exactly one parked-block tag")
}

/// bug-565: where an inline `TRAP`'s error REBUILD branch gets the `ErrorLoc` it
/// inlines into the flat `Error` block.
///
/// This is the ONLY axis on which the three trapped-`Result` lowerings differ, and
/// the partition is total by construction — [`CodeBuilder::emit_trapped_error_result`]
/// `match`es it, so a fourth lowering cannot reach the shared free without
/// declaring which of these it is. `tests/codegen/codegen_trap_error_free.rs`
/// asserts every variant is reachable and that no other emitter builds a trapped
/// error `Result`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TrappedErrorSource {
    /// The callee's own origin, arriving in `RESULT_ERROR_SOURCE_REGISTER` and
    /// preserved verbatim: the direct user/`.mfb` callee and the indirect
    /// `FUNC`-value callee (`builder_values.rs`, `NirValue::CallResult`).
    CalleeRegister,
    /// A fresh `ErrorLoc` for the current inline expression: an inline builtin or
    /// a runtime helper trapped here, which has no MFB-level origin of its own.
    CurrentLocation,
    /// An inline-trapped `thread::waitFor`: the message and origin live in the
    /// WORKER arena and are deep-copied into this one before they are inlined.
    WorkerArena,
}

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
        // origin live in the worker arena and arrive in x2/x3; they are deep-copied
        // into the caller arena. Otherwise the error originates at this inline
        // expression and its `ErrorLoc` is built from the current source location.
        worker_error_source: bool,
    ) -> Result<ValueResult, String> {
        // The single parse this function already performed for its result
        // type, hoisted so the block-free above shares it (plan-111-D). Letter F
        // types `success_type` itself and deletes it.
        let success = success_type.clone();
        let tag_slot = self.allocate_stack_object("raw_result_tag", 8);
        let value_slot = self.allocate_stack_object("raw_result_value", 8);
        let message_slot = self.allocate_stack_object("raw_result_message", 8);
        let source_raw_slot = self.allocate_stack_object("raw_result_source_raw", 8);
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
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), tag_slot));
        self.emit(abi::compare_immediate(&scratch9, RESULT_OK_TAG));
        self.emit(abi::branch_ne(&wrap_error_label));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), value_slot));
        let copied_success = self.copy_value_to_current_arena(success_type, &scratch9)?;
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
        let register = self.allocate_register();
        self.emit(abi::load_u64(&register, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            origin: None,
            type_: ParameterType::result_of(success),
            location: Operand::from(register.render()),
            text,
        })
    }

    /// The full error-payload assembly for an inline-`TRAP`'s error branch: put
    /// the trapped `Error` block in `payload_slot`, wrap a `Result` around it at
    /// `result_slot`, and then FREE the payload block, which the wrap has copied.
    ///
    /// Shared by all three lowerings that build a trapped `Result` — the direct
    /// user/`.mfb` callee and the indirect `FUNC`-value callee in
    /// `builder_values.rs`, and [`Self::materialize_current_result`] for a runtime
    /// helper or an inline builtin. They differ only in where the rebuild branch
    /// gets its `ErrorLoc`, which is [`TrappedErrorSource`].
    ///
    /// **bug-565.** Before this was shared, only the ADOPT branch of the
    /// `materialize_current_result` copy freed anything. The other three paths
    /// each leaked TWO arena blocks per trapped error: the flat `Error` block
    /// `emit_build_error_inline` builds (which [`Self::emit_build_result_inline`]
    /// deep-copies into the `Result`, after which nothing owns the original), and
    /// — on the two `builder_values.rs` paths, which rebuilt unconditionally —
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
        scratch9: impl Into<Operand>,
        tag_slot: usize,
        value_slot: usize,
        message_slot: usize,
        source_raw_slot: usize,
        payload_slot: usize,
        result_slot: usize,
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
        self.emit(abi::load_u64(
            scratch9.clone(),
            abi::stack_pointer(),
            tag_slot,
        ));
        self.emit(abi::compare_immediate(
            scratch9.clone(),
            adoptable_error_tag(),
        ));
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
                self.emit(abi::load_u64(
                    scratch9.clone(),
                    abi::stack_pointer(),
                    message_slot,
                ));
                self.emit(abi::store_u64(scratch9.clone(), abi::stack_pointer(), raw));
                message_borrowed_slot = raw;
                // A propagated worker error: deep-copy its message and origin out of
                // the (still-alive) worker arena into the caller arena. If the helper
                // raised its own error (source == 0), stamp this inline expression.
                self.emit(abi::load_u64(
                    scratch9.clone(),
                    abi::stack_pointer(),
                    message_slot,
                ));
                let copied_message =
                    self.copy_value_to_current_arena(&ParameterType::String, scratch9.clone())?;
                self.emit(abi::store_u64(
                    &copied_message,
                    abi::stack_pointer(),
                    message_slot,
                ));
                let own = self.label("raw_worker_error_own");
                let done = self.label("raw_worker_error_done");
                self.emit(abi::load_u64(
                    scratch9.clone(),
                    abi::stack_pointer(),
                    source_raw_slot,
                ));
                self.emit(abi::compare_immediate(scratch9.clone(), "0"));
                self.emit(abi::branch_eq(&own));
                let copied_source = self.copy_value_to_current_arena(
                    &ParameterType::named("ErrorLoc"),
                    scratch9.clone(),
                )?;
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

    fn emit_free_trapped_error_payload(&mut self, payload_slot: usize) -> Result<(), String> {
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

    pub(crate) fn copy_value_to_current_arena(
        &mut self,
        type_: &ParameterType,
        source: impl Into<Operand>,
    ) -> Result<VirtualRegister, String> {
        // A recursive value (e.g. `dom::Node`, whose `ElementNode.children` is
        // `List OF Node`) is a pointer-linked graph; copying it inline would make
        // the code generator recurse over the *type* without bound (bug-391).
        // Route it through a per-type runtime deep-copy function instead, so the
        // recursion runs at run time over the finite *data* and terminates. Every
        // such function's body is `emit_thread_copy_real`, whose own field/element
        // edges come back here — so a recursive sub-edge becomes a call, not more
        // inline code. Non-recursive values are unaffected (copied inline as before).
        if type_participates_in_cycle(&self.type_model, type_) {
            return self.emit_thread_copy_call(type_, source);
        }
        self.emit_thread_copy_real(type_, source)
    }

    /// Call the per-type deep-copy function (`thread_copy_symbol`), passing
    /// `source` in the first argument register and returning the copied pointer.
    fn emit_thread_copy_call(
        &mut self,
        type_: &ParameterType,
        source: impl Into<Operand>,
    ) -> Result<VirtualRegister, String> {
        let symbol = thread_copy_symbol(type_);
        self.emit(abi::move_register(abi::c_arg(0), source));
        self.emit_symbol_call(&symbol);
        let result = self.allocate_register();
        self.emit(abi::move_register(&result, abi::return_register()));
        Ok(result)
    }

    /// The concrete per-shape deep copy. Used inline for a non-recursive value,
    /// and as the body of each per-type copy function for a recursive one.
    pub(crate) fn emit_thread_copy_real(
        &mut self,
        type_: &ParameterType,
        source: impl Into<Operand>,
    ) -> Result<VirtualRegister, String> {
        match type_ {
            __t if matches!(
                __t,
                ParameterType::Nothing
                    | ParameterType::Boolean
                    | ParameterType::Byte
                    | ParameterType::Integer
                    | ParameterType::Float
                    | ParameterType::Fixed
                    | ParameterType::Money
            ) || __t.is_named("Scalar") =>
            {
                let result = self.allocate_register();
                self.emit(abi::move_register(&result, source));
                Ok(result)
            }
            // Any fully-flat value — `String`, a flat record/data-union, or a flat
            // collection — is a single pointer-free block, so the generic flat
            // copy (`arena_alloc` + `memcpy`) is a sound deep copy (plan-02 §4.1,
            // Phase 6). Only types that still embed pointers fall through to the
            // per-type glue below.
            //
            // plan-114-C4: this asks **memcpy**-copyability, not
            // arena-transferability, and the distinction is the function's name.
            // It copies into the CURRENT arena, and most of its callers are
            // in-arena — the `Result` wrap at `:145` is reached by any `TRAP` in
            // a thread-free program. "Did the source come from another thread's
            // arena?" is the *caller's* question, not this value's shape, and it
            // is answered where the cross-thread decision is actually made:
            // `collection_payload_needs_transfer_fix` and the thread-send
            // `size_computable`, both of which do take arena-transferability.
            // Asking it here changed codegen for `tests/byte-identity/tcp`,
            // which has no thread in it at all.
            other if self.type_is_memcpy_copyable(other) => {
                self.copy_flat_block(&other.clone(), source)
            }
            // The only non-flat values left are resources and the collections /
            // unions that embed them (the single remaining pointer, plan-02 §9).
            // Their transfer copy is still a `memcpy` that moves the resource
            // handle verbatim, plus the per-payload no-op kept for symmetry.
            other if typed_is_collection_type(other) => {
                self.copy_collection_to_current_arena(other, source)
            }
            // bug-546: the sendability question is the MODEL's, not the builtin
            // registry's — a user-declared `RESOURCE Db … THREAD_SENDABLE` is as
            // sendable as `fs.File`, and its record is the same canonical plan-80
            // record (`link_thunk.rs`, `if function.return_resource`), so the same
            // deep copy is correct for it. Before this the predicate was
            // builtin-only, so a declared resource reached neither resource arm.
            other if self.is_sendable_resource_nominal(other) => {
                self.copy_resource_to_current_arena(other, source)
            }
            // A non-sendable resource (audio streams, TLS sockets/listeners) is a
            // pointer to its arena record and never crosses a thread boundary —
            // the frontend forbids transferring it. The only same-arena
            // materialization that reaches here is a `TRAP` wrapping the open
            // result in `Result OF <resource>`: carry the move-only handle by
            // pointer (a deep record clone would both duplicate the OS handle and
            // assume the fixed `File` layout, which audio's larger `AudioHandle`
            // does not share). The source temporary is consumed, so the handle is
            // owned and closed exactly once.
            // bug-479: a thread handle is carried by POINTER, exactly like the
            // non-sendable resource arm below and for the same reasons. It is a
            // 120-byte block holding pointers to its two message queues and two
            // resource queues, so a deep copy would duplicate the block while the
            // queues stayed behind — every send into the copy would be lost.
            //
            // Carrying the pointer is sound here because the only materialization
            // that reaches this arm is a SAME-ARENA one: an inline `TRAP` wrapping
            // `thread::start`'s result in `Result OF Thread OF …`. A thread handle
            // can never cross a thread boundary — `ir::verify`'s `is_copyable`
            // answers `false` for `ThreadHandle`, and the frontend rejects sending
            // one — so there is no cross-arena caller to strand a pointer for.
            other if matches!(other, ParameterType::ThreadHandle { .. }) => {
                let result = self.allocate_register();
                self.emit(abi::move_register(&result, source));
                Ok(result)
            }
            // bug-546: likewise model-aware. A user-declared resource WITHOUT
            // `THREAD_SENDABLE` belongs here and not on the arm above — the
            // frontend forbids transferring it, so the only materialization that
            // reaches this arm is the same-arena `TRAP` wrap, and routing it to
            // the deep copy would quietly grant a capability its author declined.
            other if self.is_resource_nominal(other) => {
                let result = self.allocate_register();
                self.emit(abi::move_register(&result, source));
                Ok(result)
            }
            // A resource union transferred with its STATE is spelled
            // `Stream STATE Cursor`; the union set is keyed on the bare name, so
            // base-strip to route it to the union deep-copy (plan-75 gap 2). The
            // full type (with STATE) is passed on so the variant-record copy can
            // deep-copy the uniform STATE payload.
            other
                if self
                    .type_model
                    .union_names
                    .contains(&ParameterType::declared(&other.without_state().name())) =>
            {
                self.copy_union_to_current_arena(other, source)
            }
            // A non-flat record (its fields embed pointers — a recursive field, a
            // nested non-flat record/collection). Deep-copy the block, then fix up
            // its pointer fields (bug-391). A flat record was handled by the flat
            // arm above; this arm exists for the non-flat case only.
            other if self.type_model.record_fields.contains_key(other) => {
                self.copy_record_to_current_arena(other, source)
            }
            other => Err(format!(
                "native thread transfer cannot copy value of type '{other}'"
            )),
        }
    }

    /// Deep-copy a non-flat record: size it, `arena_alloc`, whole-block `memcpy`
    /// (fixed slots + inlined flat fields), then deep-copy its pointer fields so
    /// nothing aliases the source arena. Twin of `copy_union_to_current_arena`.
    fn copy_record_to_current_arena(
        &mut self,
        type_: &ParameterType,
        source: impl Into<Operand>,
    ) -> Result<VirtualRegister, String> {
        let source_slot = self.allocate_stack_object("thread_copy_record_source", 8);
        let size_slot = self.allocate_stack_object("thread_copy_record_size", 8);
        let result_slot = self.allocate_stack_object("thread_copy_record_result", 8);
        let alloc_ok = self.label("thread_copy_record_alloc_ok");
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        self.emit(abi::store_u64(source, abi::stack_pointer(), source_slot));
        self.emit_record_block_size_to_slot(type_, source_slot, size_slot)?;
        self.emit(abi::load_u64(
            abi::return_register(),
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
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            result_slot,
        ));
        self.emit(abi::load_u64(&scratch13, abi::stack_pointer(), size_slot));
        self.emit_copy_bytes(
            abi::mfb_return(1),
            &scratch9,
            &scratch13,
            "thread_copy_record_raw",
        );
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), result_slot));
        self.copy_record_fields_into_existing(type_, &scratch9, &scratch10)?;
        let result = self.allocate_register();
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(result)
    }

    /// Materialize a thread-sendable resource handle (e.g. `File`, `tls::Socket`)
    /// into the current arena, so the receiver owns the underlying OS resource.
    /// The sender's lexical cleanup is deactivated on the successful-transfer
    /// path, so the resource is closed exactly once by the receiver.
    ///
    /// A record is the **canonical header** — tag @0, handle @8, closed @16,
    /// STATE @24 — followed by a **type-specific tail** from 32. The header and
    /// STATE are handled below; the tail is described by the resource's registry
    /// [`live_slots`] and carried by `emit_copy_resource_live_slots`.
    ///
    /// This doc comment used to claim "the handle is a two-word struct (a host
    /// resource word such as a file descriptor, followed by a closed flag)".
    /// That was true of `fs::File`/`tcp::Socket`/`udp::Socket` and of nothing
    /// else, and believing it is what fenced `tls::Socket`, `tls::Listener` and
    /// (needlessly) `tcp::Listener` off behind `sendable: false` (bug-464). The
    /// routine is type-agnostic and reached for *every* resource kind, so the
    /// `fs::File` buffer/cache zeroing below silently truncated any record with
    /// live state in its tail — a transferred TLS socket arrived with a NULL
    /// `SSL*`, not with garbage. The zeroing is still the right default for an
    /// **undeclared** slot; a declared one is copied over it.
    ///
    /// [`live_slots`]: crate::codegen::registry::RegistryResource::live_slots
    ///
    /// When the resource carries a `STATE` payload (plan-54, `type_` spells `File
    /// STATE Cursor`), the STATE record is **deep-copied** into the current
    /// (receiver) arena rather than aliasing the sender's pointer — the transfer
    /// runs with the arena switched to the destination (`transferResource`) or is
    /// the receiver's own (`acceptResource`), so `copy_value_to_current_arena`
    /// allocates the fresh STATE in receiver memory. Copying the pointer verbatim
    /// left the receiver's `FILE_OFFSET_STATE` aliasing the sender's arena, freed
    /// at the sender thread's teardown (bug-257's second finding); the independent
    /// copy severs that lifetime. The source keeps its own STATE, freed normally.
    fn copy_resource_to_current_arena(
        &mut self,
        type_: &ParameterType,
        source: impl Into<Operand>,
    ) -> Result<VirtualRegister, String> {
        let source_slot = self.allocate_stack_object("thread_copy_resource_source", 8);
        let result_slot = self.allocate_stack_object("thread_copy_resource_result", 8);
        let alloc_ok = self.label("thread_copy_resource_alloc_ok");
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        self.emit(abi::store_u64(source, abi::stack_pointer(), source_slot));
        self.emit(abi::move_immediate(
            abi::return_register(),
            "Integer",
            RESOURCE_RECORD_SIZE,
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
        // The canonical header — tag @0, handle (fd) @8, closed flag @16 — moves
        // verbatim (the OS handle itself, and its self-describing type tag).
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64(&scratch10, &scratch9, RESOURCE_OFFSET_TAG));
        self.emit(abi::store_u64(
            &scratch10,
            abi::mfb_return(1),
            RESOURCE_OFFSET_TAG,
        ));
        self.emit(abi::load_u64(&scratch10, &scratch9, FILE_OFFSET_FD));
        self.emit(abi::store_u64(
            &scratch10,
            abi::mfb_return(1),
            FILE_OFFSET_FD,
        ));
        self.emit(abi::load_u64(&scratch10, &scratch9, FILE_OFFSET_CLOSED));
        self.emit(abi::store_u64(
            &scratch10,
            abi::mfb_return(1),
            FILE_OFFSET_CLOSED,
        ));
        // STATE @24 (plan-54 §5, relocated by plan-80).
        match type_.state() {
            // Stateful: deep-copy the STATE record into the current (receiver)
            // arena so the moved handle owns an independent payload — never an
            // alias into the sender's arena (bug-257).
            Some(state_type) => {
                let have_state = self.label("thread_copy_resource_have_state");
                let state_done = self.label("thread_copy_resource_state_done");
                self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), source_slot));
                self.emit(abi::load_u64(&scratch10, &scratch9, FILE_OFFSET_STATE));
                self.emit(abi::compare_immediate(&scratch10, "0"));
                self.emit(abi::branch_ne(&have_state));
                // No STATE attached yet: leave the receiver's slot null so its
                // `accept` binding runs the ordinary lazy init.
                self.emit(abi::load_u64(
                    abi::mfb_return(1),
                    abi::stack_pointer(),
                    result_slot,
                ));
                self.emit(abi::store_u64(
                    abi::ZERO,
                    abi::mfb_return(1),
                    FILE_OFFSET_STATE,
                ));
                self.emit(abi::branch(&state_done));
                self.emit(abi::label(&have_state));
                // `scratch10` holds the source STATE pointer; `copy_value_to_current_arena`
                // consumes it before the arena_alloc clobbers registers, matching
                // the record/collection field-copy idiom above.
                let copied_state = self.copy_value_to_current_arena(&state_type, &scratch10)?;
                let state_ptr_slot =
                    self.allocate_stack_object("thread_copy_resource_state_ptr", 8);
                self.emit(abi::store_u64(
                    &copied_state,
                    abi::stack_pointer(),
                    state_ptr_slot,
                ));
                self.emit(abi::load_u64(
                    abi::mfb_return(1),
                    abi::stack_pointer(),
                    result_slot,
                ));
                self.emit(abi::load_u64(
                    &scratch10,
                    abi::stack_pointer(),
                    state_ptr_slot,
                ));
                self.emit(abi::store_u64(
                    &scratch10,
                    abi::mfb_return(1),
                    FILE_OFFSET_STATE,
                ));
                self.emit(abi::label(&state_done));
            }
            // Bare resource: `FILE_OFFSET_STATE` carries no owned record — move the
            // (null/inert) word verbatim, exactly as before.
            None => {
                self.emit(abi::load_u64(&scratch10, &scratch9, FILE_OFFSET_STATE));
                self.emit(abi::store_u64(
                    &scratch10,
                    abi::mfb_return(1),
                    FILE_OFFSET_STATE,
                ));
            }
        }
        // Opt-in per-File output buffer (plan-14-B) is not copied across a thread
        // transfer: the buffer block lives in the sender's arena. Zero the fields so
        // the moved handle starts unbuffered in the receiver (a buffered handle
        // should be flushed before transfer, or its pending bytes are lost — the
        // same opt-in trade-off as the crash caveat). For non-File resources these
        // words are inert.
        self.emit(abi::store_u64(
            abi::ZERO,
            abi::mfb_return(1),
            FILE_OFFSET_BUF_PTR,
        ));
        self.emit(abi::store_u64(
            abi::ZERO,
            abi::mfb_return(1),
            FILE_OFFSET_BUF_FILLED,
        ));
        self.emit(abi::store_u64(
            abi::ZERO,
            abi::mfb_return(1),
            FILE_OFFSET_BUF_ENABLED,
        ));
        // The transparent read buffer (plan-14-C) is a cache, not copied: a moved
        // handle starts with an empty cache. These words are inert for non-File
        // resources.
        self.emit(abi::store_u64(
            abi::ZERO,
            abi::mfb_return(1),
            FILE_OFFSET_READ_PTR,
        ));
        self.emit(abi::store_u64(
            abi::ZERO,
            abi::mfb_return(1),
            FILE_OFFSET_READ_POS,
        ));
        self.emit(abi::store_u64(
            abi::ZERO,
            abi::mfb_return(1),
            FILE_OFFSET_READ_FILL,
        ));
        self.emit(abi::store_u64(
            abi::ZERO,
            abi::mfb_return(1),
            FILE_OFFSET_READ_AT_EOF,
        ));
        // The type-specific tail (bug-464). MUST come after the zeroing above,
        // which covers 32..80 unconditionally and would otherwise clobber every
        // slot carried here. Reads the source record, so it must also come
        // before the source is tombstoned below.
        self.emit_copy_resource_live_slots(type_, source_slot, result_slot)?;
        // The source record's contents now live in the destination, so the source
        // is dead: flag it `moved|closed` (plan-52-B §3b). This MUST come after
        // the flag-word copy above — flagging first would hand the destination an
        // already-moved record and make the transferred handle unusable.
        //
        // `moved` is bit 1 of the same word `closed` (bit 0) lives in, so this
        // costs no space and keeps plan-38's canonical closed-flag invariant
        // (offset 16 since plan-80). Both bits are set:
        // every existing guard is a `!= 0` test, so bit 0 makes a stale alias of a
        // moved handle refuse operations with no new code, while bit 1 lets a guard
        // that cares report `ErrResourceMoved` instead of `ErrResourceClosed`.
        //
        // Without this the sender's record kept a live fd the receiver now owns, so
        // an alias the static move rules did not catch would silently operate on
        // another thread's handle. The closed flag has always been the backstop for
        // exactly that (§15.6); this extends it to moves.
        //
        // Reached on both sides of a transfer: the send path (source = the sender's
        // binding, the case that matters) and `thread.acceptResource` (source = the
        // transient queue record, already garbage — flagging it is a harmless no-op
        // that keeps one uniform rule: this helper moves, and its source is dead).
        //
        // bug-425: on the send path the outcome of the enqueue is not yet known
        // here — a full/closed/cancelled destination queue fails the transfer and
        // ownership must stay with the sender. Flagging the source now tombstones a
        // handle the sender still owns, so its `TRAP`/scope cleanup can neither use
        // nor close it. When `suppress_resource_source_flag` is set the send helper
        // is deferring this store to its enqueue-success branch; skip it here. The
        // accept side and the nested union/collection copies keep flagging inline.
        if !self.suppress_resource_source_flag {
            self.emit_flag_resource_source_moved(source_slot);
        }
        let result = self.allocate_register();
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(result)
    }

    /// Carry `type_`'s declared live slots — the type-specific record tail past
    /// the canonical header — from the source record into the freshly-allocated
    /// destination (bug-464).
    ///
    /// `source_slot` and `result_slot` are SP-relative stack slots holding the
    /// two record pointers. They are re-loaded around every `arena_alloc`, which
    /// clobbers the argument/return registers; nothing is kept live across one.
    ///
    /// Emits nothing at all when the resource declares no slots, which is every
    /// resource that was already sendable — so their transfer codegen is
    /// byte-identical to before this existed.
    fn emit_copy_resource_live_slots(
        &mut self,
        type_: &ParameterType,
        source_slot: usize,
        result_slot: usize,
    ) -> Result<(), String> {
        use crate::codegen::engine::types::PlatformFamily;
        use crate::codegen::registry::{SlotBackend, SlotTransfer};

        let family = self.platform.family();
        let slots: Vec<_> = crate::codegen::resource::builtin_resource_live_slots(type_)
            .iter()
            .filter(|slot| match slot.backend {
                SlotBackend::OpenSsl => family == PlatformFamily::Linux,
                SlotBackend::Schannel => family == PlatformFamily::Windows,
                SlotBackend::NetworkFramework => family == PlatformFamily::MacOS,
            })
            .copied()
            .collect();
        if slots.is_empty() {
            return Ok(());
        }
        // Every declared slot must live wholly inside the shared record
        // envelope and past the header the caller already copied. A descriptor
        // that violates either would corrupt a neighbouring record or re-copy
        // the header, so it is a compiler bug, not a user error.
        for slot in &slots {
            if slot.offset < RESOURCE_OFFSET_STATE + 8 {
                return Err(format!(
                    "resource `{}` declares a live slot at offset {} inside the canonical header \
                     (tag/handle/closed/STATE end at {}): {}",
                    type_.name(),
                    slot.offset,
                    RESOURCE_OFFSET_STATE + 8,
                    slot.what
                ));
            }
            if slot.offset + 8 > RESOURCE_RECORD_SIZE_BYTES {
                return Err(format!(
                    "resource `{}` declares a live slot at offset {} past the {}-byte record: {}",
                    type_.name(),
                    slot.offset,
                    RESOURCE_RECORD_SIZE_BYTES,
                    slot.what
                ));
            }
        }
        for (index, slot) in slots.iter().enumerate() {
            let prefix = format!("thread_copy_res_slot{index}");
            match slot.transfer {
                // A foreign-heap pointer, a refcounted handle, or an inert
                // scalar: nothing in the receiver's arena depends on it, so the
                // word moves as-is -- exactly like the fd in the header.
                SlotTransfer::Verbatim => {
                    let src = self.temporary_vreg();
                    let word = self.temporary_vreg();
                    self.emit(abi::load_u64(&src, abi::stack_pointer(), source_slot));
                    self.emit(abi::load_u64(&word, &src, slot.offset));
                    self.emit(abi::load_u64(
                        abi::mfb_return(1),
                        abi::stack_pointer(),
                        result_slot,
                    ));
                    self.emit(abi::store_u64(&word, abi::mfb_return(1), slot.offset));
                }
                // A block in the SENDER's arena. Arena state is per-thread and no
                // thread may free another's block, so the receiver gets its own
                // copy. A byte copy is sound because a transfer MOVES: any OS
                // handle duplicated inside the block is released exactly once, by
                // the receiver, since the sender is tombstoned and its cleanup
                // deactivated.
                SlotTransfer::ArenaBlock { size } => {
                    self.emit_copy_resource_arena_block(
                        source_slot,
                        result_slot,
                        slot.offset,
                        BlockLength::Fixed(size),
                        &prefix,
                    )?;
                }
                // A NUL-terminated string in the sender's arena: same ownership
                // problem, length measured at run time.
                SlotTransfer::ArenaCString => {
                    self.emit_copy_resource_arena_block(
                        source_slot,
                        result_slot,
                        slot.offset,
                        BlockLength::CString,
                        &prefix,
                    )?;
                }
            }
        }
        Ok(())
    }

    /// Copy the arena block the source record's `offset` word points at into the
    /// current (receiver) arena and store the fresh pointer in the destination
    /// record (bug-464). A **null** source pointer stays null — an unset slot is
    /// not an error, and the receiver's lazy init must still see zero.
    ///
    /// `length` is either a compile-time block size or "measure the C string".
    fn emit_copy_resource_arena_block(
        &mut self,
        source_slot: usize,
        result_slot: usize,
        offset: usize,
        length: BlockLength,
        prefix: &str,
    ) -> Result<(), String> {
        let src_slot = self.allocate_stack_object(&format!("{prefix}_src"), 8);
        let len_slot = self.allocate_stack_object(&format!("{prefix}_len"), 8);
        let new_slot = self.allocate_stack_object(&format!("{prefix}_new"), 8);
        let have = self.label(&format!("{prefix}_have"));
        let done = self.label(&format!("{prefix}_done"));
        let alloc_ok = self.label(&format!("{prefix}_alloc_ok"));

        let src = self.temporary_vreg();
        let word = self.temporary_vreg();
        self.emit(abi::load_u64(&src, abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64(&word, &src, offset));
        self.emit(abi::compare_immediate(&word, "0"));
        self.emit(abi::branch_ne(&have));
        // Null source: leave the receiver's slot null. The zeroing pass already
        // wrote 0 for slots under 88, but a declared slot may sit in the
        // headroom the pass does not reach, so this is written explicitly.
        self.emit(abi::load_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            result_slot,
        ));
        self.emit(abi::store_u64(abi::ZERO, abi::mfb_return(1), offset));
        self.emit(abi::branch(&done));

        self.emit(abi::label(&have));
        self.emit(abi::store_u64(&word, abi::stack_pointer(), src_slot));
        match length {
            BlockLength::Fixed(size) => {
                let len = self.temporary_vreg();
                self.emit(abi::move_immediate(&len, "Integer", &size.to_string()));
                self.emit(abi::store_u64(&len, abi::stack_pointer(), len_slot));
            }
            // strlen, then +1 so the NUL is copied too.
            BlockLength::CString => {
                let scan = self.temporary_vreg();
                let byte = self.temporary_vreg();
                let len = self.temporary_vreg();
                let scan_loop = self.label(&format!("{prefix}_strlen"));
                let scan_done = self.label(&format!("{prefix}_strlen_done"));
                self.emit(abi::load_u64(&scan, abi::stack_pointer(), src_slot));
                self.emit(abi::move_immediate(&len, "Integer", "0"));
                self.emit(abi::label(&scan_loop));
                self.emit(abi::load_u8(&byte, &scan, 0));
                self.emit(abi::compare_immediate(&byte, "0"));
                self.emit(abi::branch_eq(&scan_done));
                self.emit(abi::add_immediate(&scan, &scan, 1));
                self.emit(abi::add_immediate(&len, &len, 1));
                self.emit(abi::branch(&scan_loop));
                self.emit(abi::label(&scan_done));
                self.emit(abi::add_immediate(&len, &len, 1));
                self.emit(abi::store_u64(&len, abi::stack_pointer(), len_slot));
            }
        }
        // `arena_alloc(size, align)` clobbers the argument/return registers, so
        // every pointer is reloaded from its stack slot afterwards.
        self.emit(abi::load_u64(
            abi::return_register(),
            abi::stack_pointer(),
            len_slot,
        ));
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&alloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            new_slot,
        ));
        // `emit_copy_bytes` ADVANCES both pointers, so it gets scratch copies and
        // the destination pointer is re-read from its slot afterwards.
        let dst_p = self.temporary_vreg();
        let src_p = self.temporary_vreg();
        let len_p = self.temporary_vreg();
        self.emit(abi::load_u64(&dst_p, abi::stack_pointer(), new_slot));
        self.emit(abi::load_u64(&src_p, abi::stack_pointer(), src_slot));
        self.emit(abi::load_u64(&len_p, abi::stack_pointer(), len_slot));
        self.emit_copy_bytes(&dst_p, &src_p, &len_p, prefix);
        let fresh = self.temporary_vreg();
        self.emit(abi::load_u64(&fresh, abi::stack_pointer(), new_slot));
        self.emit(abi::load_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            result_slot,
        ));
        self.emit(abi::store_u64(&fresh, abi::mfb_return(1), offset));
        self.emit(abi::label(&done));
        Ok(())
    }

    /// Flag the resource record at the pointer held in `source_slot` (a stack slot,
    /// SP-relative) `moved|closed` — the post-copy tombstone from
    /// `copy_resource_to_current_arena`, extracted (bug-425) so the thread-send
    /// lowering can defer it from copy time to its enqueue-success branch. See the
    /// long comment at that call site for why both bits are set.
    pub(crate) fn emit_flag_resource_source_moved(&mut self, source_slot: usize) {
        let source_ptr = self.temporary_vreg();
        let moved_flag = self.temporary_vreg();
        self.emit(abi::load_u64(
            &source_ptr,
            abi::stack_pointer(),
            source_slot,
        ));
        self.emit(abi::move_immediate(
            &moved_flag,
            "Integer",
            RESOURCE_MOVED_CLOSED_VALUE,
        ));
        self.emit(abi::store_u64(&moved_flag, &source_ptr, FILE_OFFSET_CLOSED));
    }

    /// True when field `field_type` of `record_type` is a pointer to a separate
    /// allocation that a whole-block `memcpy` would alias and must therefore be
    /// deep-copied. Inlined fields (`String` and fully-flat nested records) come
    /// along with the block copy; only still-pointer composites (`Union`/`List`/
    /// `Map`/`Result`/`Error`, a not-yet-flat nested record) and the built-in
    /// pointer-`String` records' `String` fields need the fix.
    fn record_field_is_pointer_in(
        &self,
        record_type: &ParameterType,
        field_type: &ParameterType,
    ) -> bool {
        if self.record_field_is_inlined(record_type, field_type) {
            return false;
        }
        *field_type == ParameterType::String || self.record_field_is_pointer(field_type)
    }

    fn record_needs_pointer_field_fix(&self, record_type: &ParameterType) -> bool {
        self.type_model
            .record_fields
            .get(record_type)
            .map(|fields| {
                fields
                    .iter()
                    .any(|(_, ft)| self.record_field_is_pointer_in(record_type, &ft))
            })
            .unwrap_or(false)
    }

    fn copy_union_to_current_arena(
        &mut self,
        type_: &ParameterType,
        source: impl Into<Operand>,
    ) -> Result<VirtualRegister, String> {
        let source_slot = self.allocate_stack_object("thread_copy_union_source", 8);
        let size_slot = self.allocate_stack_object("thread_copy_union_size", 8);
        let result_slot = self.allocate_stack_object("thread_copy_union_result", 8);
        let alloc_ok = self.label("thread_copy_union_alloc_ok");
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        self.emit(abi::store_u64(source, abi::stack_pointer(), source_slot));
        // A data union is `{tag, size, variant-record-block}`: its total size is
        // the runtime `size` word at +8 (plan-02 §4.3). A resource union is the
        // fixed `{tag, resource-ptr}` block.
        if self.union_is_data(type_) {
            self.emit_data_union_size_to_slot(source_slot, size_slot);
        } else {
            let size = self.inline_collection_payload_size(type_).ok_or_else(|| {
                format!("native thread transfer union type '{type_}' does not resolve")
            })?;
            self.emit(abi::move_immediate(&scratch8, "Integer", &size.to_string()));
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
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            result_slot,
        ));
        self.emit(abi::load_u64(&scratch13, abi::stack_pointer(), size_slot));
        self.emit_copy_bytes(
            abi::mfb_return(1),
            &scratch9,
            &scratch13,
            "thread_copy_union_raw",
        );
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), result_slot));
        self.copy_union_fields_into_existing(type_, &scratch9, &scratch10)?;
        let result = self.allocate_register();
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(result)
    }

    /// True when a collection of `type_` embeds pointer payloads (nested
    /// collections, records, unions, `Result`/`Error`) that a plain byte copy
    /// would alias rather than deep-copy, so the per-payload transfer fix is
    /// still required. A collection whose key/value payloads are all inline
    /// (scalars, `String`) is already flat and copies generically.
    fn collection_needs_transfer_fix(&self, type_: &ParameterType) -> Result<bool, String> {
        let (key_type, value_type) = if let Some(value_type) = typed_list_element_type(type_) {
            (None, value_type)
        } else {
            let (key, value) = typed_map_type_parts(type_).ok_or_else(|| {
                format!("native thread transfer collection type '{type_}' does not resolve")
            })?;
            (Some(key), value)
        };
        if let Some(key_type) = key_type.as_ref() {
            if self.collection_payload_needs_transfer_fix(key_type) {
                return Ok(true);
            }
        }
        Ok(self.collection_payload_needs_transfer_fix(&value_type))
    }

    fn copy_collection_to_current_arena(
        &mut self,
        type_: &ParameterType,
        source: impl Into<Operand>,
    ) -> Result<VirtualRegister, String> {
        let source = source.into();
        // A collection with only inline payloads is a flat, pointer-free block:
        // copy it with the generic flat copy (plan-02 §4.1, Phase 1). Only
        // collections embedding pointer payloads keep the per-payload fix below.
        if !self.collection_needs_transfer_fix(type_)? {
            return self.copy_flat_block(&type_.clone(), source);
        }
        let source_slot = self.allocate_stack_object("thread_copy_collection_source", 8);
        let size_slot = self.allocate_stack_object("thread_copy_collection_size", 8);
        let result_slot = self.allocate_stack_object("thread_copy_collection_result", 8);
        let alloc_ok = self.label("thread_copy_collection_alloc_ok");
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        self.emit(abi::store_u64(
            source.clone(),
            abi::stack_pointer(),
            source_slot,
        ));
        // bug-538 (found reproducing it): this used to compute the block size by
        // hand as `HEADER + capacity*ENTRY + dataCapacity` — **omitting a map's or
        // set's hash-bucket region**, which `emit_reserve_map_buckets` adds to
        // every other allocation path and which `emit_inlined_block_size_from_ptr_slot`
        // (the single authority, `:301`) has always included. A deep-copied
        // `Map OF String TO json::Json` therefore arrived `capacity << 4` bytes
        // short with `BUCKETS_READY` byte-copied as 1 from the source, so the very
        // first probe read past the block and the lazy `build_buckets` rebuild
        // WROTE past it — bug-02's exact failure mode, in the transfer copier.
        // Observed before the fix (`mfb` at main `7b0f93c08`, no change of mine):
        // `thread::waitFor` a `json::Json` parsed from `{"u":{"n":"A"}}`
        // stringifies correctly but `json::get(v, ["u"])` answers `{}`.
        //
        // Use the canonical sizer. It also picks the entry stride by element type
        // instead of hardcoding `COLLECTION_ENTRY_SIZE`; every type that reaches
        // here has a 40-byte entry (a fixed-width element is pointer-free, so
        // `collection_needs_transfer_fix` is false for it), so that part is
        // byte-neutral — the bucket region is the whole behavioural delta.
        self.emit_inlined_block_size_from_ptr_slot(type_, source_slot, size_slot)?;
        self.emit(abi::load_u64(
            abi::return_register(),
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
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), size_slot));
        self.emit_copy_bytes(
            abi::mfb_return(1),
            &scratch9,
            &scratch10,
            "thread_copy_collection",
        );
        // The whole block (including a map's/set's bucket region) was copied
        // verbatim, so the destination inherited the source's `BUCKETS_READY`
        // flag. Clear it, exactly as `copy_collection_tight` does for the same
        // reason: the buckets are rebuilt on first probe, so no copy can ever
        // depend on the source's index being current. Cheap, and it makes the
        // block's validity independent of what the bucket words contain.
        if matches!(type_, ParameterType::MapOf(..) | ParameterType::SetOf(_)) {
            self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), result_slot));
            self.emit(abi::move_immediate(&scratch10, "Byte", "0"));
            self.emit(abi::store_u8(
                &scratch10,
                &scratch9,
                COLLECTION_OFFSET_BUCKETS_READY,
            ));
        }
        self.fix_collection_transfer_payloads(type_, source_slot, result_slot)?;
        let result = self.allocate_register();
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(result)
    }

    fn fix_collection_transfer_payloads(
        &mut self,
        type_: &ParameterType,
        source_slot: usize,
        result_slot: usize,
    ) -> Result<(), String> {
        let (key_type, value_type) = if let Some(value_type) = typed_list_element_type(type_) {
            (None, value_type)
        } else {
            let (key, value) = typed_map_type_parts(type_).ok_or_else(|| {
                format!("native thread transfer collection type '{type_}' does not resolve")
            })?;
            (Some(key), value)
        };
        if let Some(key_type) = key_type.as_ref() {
            if self.collection_payload_needs_transfer_fix(key_type) {
                self.fix_collection_transfer_payload(source_slot, result_slot, key_type, true)?;
            }
        }
        if self.collection_payload_needs_transfer_fix(&value_type) {
            self.fix_collection_transfer_payload(source_slot, result_slot, &value_type, false)?;
        }
        Ok(())
    }

    fn collection_payload_needs_transfer_fix(&self, type_: &ParameterType) -> bool {
        if self.type_model.record_fields.contains_key(type_) {
            // A record payload was byte-copied whole (inlined fields came along);
            // it only needs the per-payload fix if it still has pointer fields to
            // deep-copy (plan-02 §4.2).
            return self.record_needs_pointer_field_fix(type_);
        }
        if typed_is_collection_type(type_)
            || self.type_model.union_names.contains(type_)
            || matches!(type_, ParameterType::ResultOf(_))
        {
            // A flat nested collection / data union / `Result` was inlined and
            // copied whole; only a non-flat one (an embedded pointer/resource
            // payload) needs the per-payload deep-copy fix (plan-02 §4.3/§4.4).
            // Bare resource payloads fall through (moved verbatim, no fix).
            return !self.type_is_arena_transferable(type_);
        }
        false
    }

    fn fix_collection_transfer_payload(
        &mut self,
        source_slot: usize,
        result_slot: usize,
        payload_type: &ParameterType,
        key_payload: bool,
    ) -> Result<(), String> {
        let index_slot = self.allocate_stack_object("thread_copy_collection_index", 8);
        let source_entry_slot =
            self.allocate_stack_object("thread_copy_collection_source_entry", 8);
        let dest_entry_slot = self.allocate_stack_object("thread_copy_collection_dest_entry", 8);
        let source_payload_slot =
            self.allocate_stack_object("thread_copy_collection_source_payload", 8);
        let dest_payload_slot =
            self.allocate_stack_object("thread_copy_collection_dest_payload", 8);
        let loop_label = self.label("thread_copy_collection_fix_loop");
        let next_label = self.label("thread_copy_collection_fix_next");
        let done_label = self.label("thread_copy_collection_fix_done");
        let entry_offset = if key_payload {
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET
        } else {
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET
        };
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        self.emit(abi::move_immediate(&scratch9, "Integer", "0"));
        self.emit(abi::store_u64(&scratch9, abi::stack_pointer(), index_slot));
        self.emit(abi::label(&loop_label));
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), index_slot));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), source_slot));
        // Walk only the live entries `[0..count)`. The entry table is dense, but
        // slots `[count..capacity)` of a grown buffer are never initialized (grow
        // copies `count*ENTRY` bytes) and recycled arena memory is entropy-
        // scrubbed, so bounding at `capacity` deep-copied any spare entry whose
        // garbage flags byte happened to equal USED — a wild pointer walk
        // (bug-146).
        self.emit(abi::load_u64(
            &scratch10,
            &scratch9,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::compare_registers(&scratch8, &scratch10));
        self.emit(abi::branch_ge(&done_label));

        self.emit(abi::move_immediate(
            &scratch10,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch11, &scratch8, &scratch10));
        self.emit(abi::add_immediate(
            &scratch11,
            &scratch11,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), source_slot));
        self.emit(abi::add_registers(&scratch12, &scratch9, &scratch11));
        self.emit(abi::store_u64(
            &scratch12,
            abi::stack_pointer(),
            source_entry_slot,
        ));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), result_slot));
        self.emit(abi::add_registers(&scratch12, &scratch9, &scratch11));
        self.emit(abi::store_u64(
            &scratch12,
            abi::stack_pointer(),
            dest_entry_slot,
        ));
        // A `flags != USED` guard used to sit here, skipping the entry. It was
        // unconditionally true and has been removed (plan-57-E §4.1).
        //
        // The audit, because "this looks dead" is not evidence: this was the
        // ONLY read of `COLLECTION_ENTRY_OFFSET_FLAGS` anywhere in the tree.
        // Every other reference is a store, and every store writes
        // `COLLECTION_ENTRY_FLAG_USED`. Nothing clears the bit — `removeAt`
        // compacts the entry array rather than tombstoning — so the compare
        // could never fail and the branch was never taken.
        //
        // It was a guard against a tombstone representation that does not
        // exist, and it predates plan-57 rather than being made dead by it. If
        // a deletion ever DOES start tombstoning, this loop needs the check
        // back, along with every other consumer that assumes `count` entries are
        // contiguous and live.
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), source_slot));
        // Kind-0 stride, deliberately: this walks the ENTRY table, so it only
        // runs for a block that has one. `payload_type` must NOT be used here —
        // for a `Map OF String TO Integer` it is `Integer`, which would select
        // the entry-free stride for a map, and maps keep their entries forever.
        // A fixed-width LIST never reaches this path at all
        // (`collection_needs_transfer_fix` is false for pointer-free payloads).
        self.emit_collection_data_pointer_for(&scratch10, &scratch9, &ParameterType::named(""));
        self.emit(abi::load_u64(
            &scratch11,
            abi::stack_pointer(),
            source_entry_slot,
        ));
        self.emit(abi::load_u64(&scratch12, &scratch11, entry_offset));
        self.emit(abi::add_registers(&scratch10, &scratch10, &scratch12));
        self.emit(abi::store_u64(
            &scratch10,
            abi::stack_pointer(),
            source_payload_slot,
        ));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), result_slot));
        self.emit_collection_data_pointer_for(&scratch10, &scratch9, &ParameterType::named(""));
        self.emit(abi::load_u64(
            &scratch11,
            abi::stack_pointer(),
            dest_entry_slot,
        ));
        self.emit(abi::load_u64(&scratch12, &scratch11, entry_offset));
        self.emit(abi::add_registers(&scratch10, &scratch10, &scratch12));
        self.emit(abi::store_u64(
            &scratch10,
            abi::stack_pointer(),
            dest_payload_slot,
        ));

        if typed_is_collection_type(payload_type)
            || matches!(payload_type, ParameterType::ResultOf(_))
            || *payload_type == ParameterType::named("Error")
        {
            self.emit(abi::load_u64(
                &scratch9,
                abi::stack_pointer(),
                source_payload_slot,
            ));
            self.emit(abi::load_u64(&scratch10, &scratch9, 0));
            let copied = self.copy_value_to_current_arena(payload_type, &scratch10)?;
            // Stash before reloading the destination pointer: `copied` may be x9.
            let payload_copied_slot = self.allocate_stack_object("thread_copy_payload_field", 8);
            self.emit(abi::store_u64(
                &copied,
                abi::stack_pointer(),
                payload_copied_slot,
            ));
            self.emit(abi::load_u64(
                &scratch9,
                abi::stack_pointer(),
                dest_payload_slot,
            ));
            self.emit(abi::load_u64(
                &scratch10,
                abi::stack_pointer(),
                payload_copied_slot,
            ));
            self.emit(abi::store_u64(&scratch10, &scratch9, 0));
        } else if self.type_model.record_fields.contains_key(payload_type) {
            self.emit(abi::load_u64(
                &scratch9,
                abi::stack_pointer(),
                source_payload_slot,
            ));
            self.emit(abi::load_u64(
                &scratch10,
                abi::stack_pointer(),
                dest_payload_slot,
            ));
            self.copy_record_fields_into_existing(payload_type, &scratch9, &scratch10)?;
        } else if self.type_model.union_names.contains(payload_type) {
            self.emit(abi::load_u64(
                &scratch9,
                abi::stack_pointer(),
                source_payload_slot,
            ));
            self.emit(abi::load_u64(
                &scratch10,
                abi::stack_pointer(),
                dest_payload_slot,
            ));
            self.copy_union_fields_into_existing(payload_type, &scratch9, &scratch10)?;
        }

        self.emit(abi::label(&next_label));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), index_slot));
        self.emit(abi::add_immediate(&scratch9, &scratch9, 1));
        self.emit(abi::store_u64(&scratch9, abi::stack_pointer(), index_slot));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&done_label));
        Ok(())
    }

    fn copy_record_fields_into_existing(
        &mut self,
        type_: &ParameterType,
        source: impl Into<Operand>,
        destination: impl Into<Operand>,
    ) -> Result<(), String> {
        let fields = self
            .type_model
            .record_fields
            .get(type_)
            .cloned()
            .ok_or_else(|| {
                format!("native thread transfer record type '{type_}' does not resolve")
            })?;
        let source_slot = self.allocate_stack_object("thread_copy_record_inline_source", 8);
        let destination_slot =
            self.allocate_stack_object("thread_copy_record_inline_destination", 8);
        self.emit(abi::store_u64(source, abi::stack_pointer(), source_slot));
        self.emit(abi::store_u64(
            destination,
            abi::stack_pointer(),
            destination_slot,
        ));
        // The whole record block was already byte-copied into `destination`
        // (inlined String fields came along). Only deep-copy pointer fields so
        // the copy aliases nothing (plan-02 §4.2).
        let copied_slot = self.allocate_stack_object("thread_copy_into_field", 8);
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        for (index, (_, field_type)) in fields.iter().enumerate() {
            if !self.record_field_is_pointer_in(type_, &field_type) {
                continue;
            }
            self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), source_slot));
            self.emit(abi::load_u64(&scratch10, &scratch9, index * 8));
            let copied = self.copy_value_to_current_arena(field_type, &scratch10)?;
            // Stash before reloading the destination pointer: `copied` may be x9.
            self.emit(abi::store_u64(&copied, abi::stack_pointer(), copied_slot));
            self.emit(abi::load_u64(
                &scratch9,
                abi::stack_pointer(),
                destination_slot,
            ));
            self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), copied_slot));
            self.emit(abi::store_u64(&scratch10, &scratch9, index * 8));
        }
        Ok(())
    }

    fn copy_union_fields_into_existing(
        &mut self,
        type_: &ParameterType,
        source: impl Into<Operand>,
        destination: impl Into<Operand>,
    ) -> Result<(), String> {
        // A transferred stateful union is spelled `Stream STATE Cursor`; the union
        // set and variant map key on the bare name (plan-75 gap 3). Its `STATE T`
        // clause names the uniform STATE record every resource variant carries.
        let union_base = type_.without_state();
        let union_state = type_.state();
        let mut variants = self
            .type_model
            .variants_for_union(&union_base)
            .map(|variant| {
                let tag = self
                    .type_model
                    .union_variant_tags
                    .get(variant)
                    .copied()
                    .ok_or_else(|| {
                        format!("native thread transfer union variant '{variant}' has no tag")
                    })?;
                let fields = self
                    .type_model
                    .union_variant_fields
                    .get(variant)
                    .cloned()
                    .unwrap_or_default();
                Ok((variant.clone(), tag, fields))
            })
            .collect::<Result<Vec<_>, String>>()?;
        variants.sort_by_key(|(_, tag, _)| *tag);
        let source_slot = self.allocate_stack_object("thread_copy_union_inline_source", 8);
        let destination_slot =
            self.allocate_stack_object("thread_copy_union_inline_destination", 8);
        let done_label = self.label("thread_copy_union_inline_done");
        let fallback_label = self.label("thread_copy_union_inline_fallback");
        let labels = variants
            .iter()
            .map(|(variant, _, _)| {
                (
                    variant.clone(),
                    self.label("thread_copy_union_inline_variant"),
                )
            })
            .collect::<HashMap<_, _>>();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        self.emit(abi::store_u64(source, abi::stack_pointer(), source_slot));
        self.emit(abi::store_u64(
            destination,
            abi::stack_pointer(),
            destination_slot,
        ));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64(&scratch10, &scratch9, 0));
        self.emit(abi::load_u64(
            &scratch9,
            abi::stack_pointer(),
            destination_slot,
        ));
        self.emit(abi::store_u64(&scratch10, &scratch9, 0));
        for (variant, tag, _) in &variants {
            self.emit(abi::compare_immediate(&scratch10, &tag.to_string()));
            self.emit(abi::branch_eq(&labels[variant]));
        }
        self.emit(abi::branch(&fallback_label));
        let is_data_union = self.union_is_data(type_);
        let union_copied_slot = self.allocate_stack_object("thread_copy_union_field", 8);
        for (variant, _, fields) in &variants {
            self.emit(abi::label(&labels[variant]));
            if is_data_union {
                // The active variant's flat record block was byte-copied at +16
                // by the whole-union memcpy; deep-copy only its pointer fields so
                // the union copy aliases nothing (plan-02 §4.3).
                self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), source_slot));
                self.emit(abi::add_immediate(&scratch9, &scratch9, 16));
                self.emit(abi::load_u64(
                    &scratch10,
                    abi::stack_pointer(),
                    destination_slot,
                ));
                self.emit(abi::add_immediate(&scratch10, &scratch10, 16));
                self.copy_record_fields_into_existing(&variant, &scratch9, &scratch10)?;
                self.emit(abi::branch(&done_label));
                continue;
            }
            if crate::codegen::builtins::is_resource_type(&variant) {
                // Resource union `{tag@0, ptr@8}`: the whole-union memcpy copied the
                // variant record pointer at +8 verbatim, so it still aliases the
                // sender's arena (a bug-257-class UAF — true for a *stateless*
                // resource union too, plan-75 gap 3). Deep-copy the variant's record
                // (with its uniform STATE payload, if any) into the current arena and
                // repoint the copy's +8. `copy_resource_to_current_arena` sizes the
                // record, deep-copies its STATE, and flags the source `moved|closed`.
                // plan-111-C attached the STATE clause structurally but rendered
                // for a consumer that still took a spelling; plan-111-E retyped
                // that consumer, so the render is gone too.
                let variant_type = match &union_state {
                    Some(state) => variant.with_state(state),
                    None => variant.clone(),
                };
                self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), source_slot));
                self.emit(abi::load_u64(&scratch10, &scratch9, 8));
                let copied = self.copy_resource_to_current_arena(&variant_type, &scratch10)?;
                // Stash before reloading the destination pointer: `copied` may be x9.
                self.emit(abi::store_u64(
                    &copied,
                    abi::stack_pointer(),
                    union_copied_slot,
                ));
                self.emit(abi::load_u64(
                    &scratch9,
                    abi::stack_pointer(),
                    destination_slot,
                ));
                self.emit(abi::load_u64(
                    &scratch10,
                    abi::stack_pointer(),
                    union_copied_slot,
                ));
                self.emit(abi::store_u64(&scratch10, &scratch9, 8));
                self.emit(abi::branch(&done_label));
                continue;
            }
            for (index, (_, field_type)) in fields.iter().enumerate() {
                self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), source_slot));
                self.emit(abi::load_u64(&scratch10, &scratch9, 8 * (index + 1)));
                let copied = self.copy_value_to_current_arena(field_type, &scratch10)?;
                // Stash before reloading the destination pointer: `copied` may be x9.
                self.emit(abi::store_u64(
                    &copied,
                    abi::stack_pointer(),
                    union_copied_slot,
                ));
                self.emit(abi::load_u64(
                    &scratch9,
                    abi::stack_pointer(),
                    destination_slot,
                ));
                self.emit(abi::load_u64(
                    &scratch10,
                    abi::stack_pointer(),
                    union_copied_slot,
                ));
                self.emit(abi::store_u64(&scratch10, &scratch9, 8 * (index + 1)));
            }
            self.emit(abi::branch(&done_label));
        }
        self.emit(abi::label(&fallback_label));
        self.emit(abi::label(&done_label));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `Result` tag vocabulary, as declared in `error_constants.rs`.
    ///
    /// bug-565's fix decides, from this tag alone, whether the trapped error's
    /// flat `Error` block is one the raiser PARKED for us to adopt or one we must
    /// rebuild — and then frees what it ended up holding. An unclassified tag
    /// falling through to `LooseRegisters` is the safe direction (it leaks rather
    /// than double-frees), but it is still a silent wrong answer, so the partition
    /// is asserted TOTAL against the constants rather than left to a default.
    ///
    /// This reads the constants file so the check cannot be satisfied by keeping a
    /// stale copy of the list in step with itself: a fifth `RESULT_*_TAG` reds this
    /// test, and whoever adds it has to say which class it belongs to.
    const ERROR_CONSTANTS_SOURCE: &str = include_str!("../../error/constants/error_constants.rs");

    fn declared_result_tags() -> Vec<String> {
        ERROR_CONSTANTS_SOURCE
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                let rest = line.strip_prefix("pub(crate) const RESULT_")?;
                let (name, value) = rest.split_once(": &str = ")?;
                if !name.ends_with("_TAG") {
                    return None;
                }
                Some(
                    value
                        .trim()
                        .trim_end_matches(';')
                        .trim_matches('"')
                        .to_string(),
                )
            })
            .collect()
    }

    #[test]
    fn the_result_tag_partition_is_total() {
        let declared = declared_result_tags();
        assert_eq!(
            declared.len(),
            4,
            "the `RESULT_*_TAG` vocabulary changed ({declared:?}); classify the new \
             tag in `trapped_error_tag_class` — an unlisted tag REBUILDS the error, \
             which silently orphans a parked `Error` block (bug-565)"
        );
        let mut classified: Vec<(String, TrappedErrorTagClass)> = declared
            .iter()
            .map(|tag| (tag.clone(), trapped_error_tag_class(tag)))
            .collect();
        classified.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            classified,
            vec![
                ("0".to_string(), TrappedErrorTagClass::NotAnError),
                ("1".to_string(), TrappedErrorTagClass::LooseRegisters),
                ("2".to_string(), TrappedErrorTagClass::LooseRegisters),
                ("3".to_string(), TrappedErrorTagClass::ParkedBlock),
            ],
            "every declared tag must have an explicit class"
        );
    }

    /// Exactly one tag adopts, and the emitter compares against THAT one.
    ///
    /// The adopt branch is the only place the trapped-error assembly takes a block
    /// it did not allocate. If two tags ever adopted, the single-compare emitter
    /// would silently serve one of them by rebuilding — orphaning a parked block
    /// again — so the "exactly one" is what makes one compare sufficient.
    #[test]
    fn exactly_one_tag_is_adoptable_and_the_emitter_uses_it() {
        let adoptable: Vec<&str> = declared_result_tags()
            .into_iter()
            .filter(|tag| trapped_error_tag_class(tag) == TrappedErrorTagClass::ParkedBlock)
            .map(|tag| match tag.as_str() {
                "0" => RESULT_OK_TAG,
                "1" => RESULT_ERR_TAG,
                "2" => RESULT_PROGRAM_EXIT_TAG,
                "3" => RESULT_ERR_BLOCK_TAG,
                other => panic!("undeclared tag {other}"),
            })
            .collect();
        assert_eq!(adoptable, vec![RESULT_ERR_BLOCK_TAG]);
        assert_eq!(adoptable_error_tag(), RESULT_ERR_BLOCK_TAG);
    }

    /// The three trapped-`Result` lowerings, and the fact that they differ only in
    /// where the REBUILD branch gets its `ErrorLoc`.
    ///
    /// [`TrappedErrorSource`] is what the shared emitter `match`es, so a fourth
    /// lowering cannot reach the shared free without declaring itself one of these
    /// — and the free of the rebuild's own `ErrorLoc` is skipped, at compile time,
    /// exactly for the variant whose `ErrorLoc` is the RAISER's.
    #[test]
    fn every_trapped_error_source_is_accounted_for() {
        let all = [
            TrappedErrorSource::CalleeRegister,
            TrappedErrorSource::CurrentLocation,
            TrappedErrorSource::WorkerArena,
        ];
        assert_eq!(all.len(), 3);
        // The one variant that allocates NOTHING for the rebuild: it reuses the
        // callee's `x3` slot verbatim, so `source_slot == source_raw_slot` and the
        // emitter never emits a free for it.
        assert_eq!(all[0], TrappedErrorSource::CalleeRegister);
        // The two that DO allocate: a fresh `ErrorLoc` (and, for a worker error, a
        // caller-arena copy of the message) which the flat `Error` then inlines.
        assert_ne!(all[1], all[0]);
        assert_ne!(all[2], all[0]);
        assert_ne!(all[1], all[2]);
    }
}
