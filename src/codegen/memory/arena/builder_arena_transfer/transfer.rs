//! plan-176-P: copying a value into the current arena — records, resources, unions and the
//! cross-thread copy. The result-tag classes live in `memory/arena/builder_arena_transfer/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    pub(crate) fn copy_value_to_current_arena(
        &mut self,
        type_: &ParameterType,
        source: impl Into<abi::Reg>,
    ) -> Result<VirtualRegister, String> {
        // A recursive value (e.g. `dom::Node`, whose `ElementNode.children` is
        // `List OF Node`) is a pointer-linked graph; copying it inline would make
        // the code generator recurse over the *type* without bound (bug-391).
        // Route it through its per-type entry point instead, which runs the
        // module's non-recursive walker (`graph_copy.rs`, plan-134-B): the walker
        // copies one block at a time and keeps the pending edges on an arena work
        // stack, so neither the compiler nor the program recurses per level of the
        // data. Non-recursive values are unaffected (copied inline as before).
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
        source: impl Into<abi::Reg>,
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
        source: impl Into<abi::Reg>,
    ) -> Result<VirtualRegister, String> {
        match type_ {
            // bug-656: `self.is_enum_type(__t)` is in this arm because an enum IS a
            // scalar — its value is the member ordinal, in the slot, with no block
            // anywhere. It is a `Named` nominal, though, so it used to fall past this
            // arm into the memcpy-copyable one below: `flatness_of_model_type` answers
            // "flat" for a name it does not recognise as a record/union/resource, which
            // sent an enum to `copy_flat_block`, which asked
            // `emit_inlined_block_size_from_ptr_slot` for the block size of a value that
            // has no block. That is the reported internal error, "native inlined field
            // size not available for type 'Colour'", on a program the frontend accepts —
            // the verifier's own classifier says enums are sendable in as many words
            // ("enums yes", `thread_unsendable_cause`), and `mfb spec language threads`
            // makes every primitive owned value sendable.
            __t if matches!(
                __t,
                ParameterType::Nothing
                    | ParameterType::Boolean
                    | ParameterType::Byte
                    | ParameterType::Integer
                    | ParameterType::Float
                    | ParameterType::Fixed
                    | ParameterType::Money
            ) || __t.is_named("Scalar")
                || self.is_enum_type(__t) =>
            {
                let result = self.allocate_register();
                self.emit(abi::move_register(&result, source));
                Ok(result)
            }
            // bug-781: a function value's copy is a new closure over a deep copy of
            // its environment, made by the environment's own routine — the type does
            // not say what was captured. A static descriptor (and a null word) is
            // returned as it is.
            ParameterType::Func(..) => {
                self.emit(abi::move_register(abi::c_arg(0), source));
                self.emit_symbol_call(CLOSURE_COPY_SYMBOL);
                let result = self.allocate_register();
                self.emit(abi::move_register(&result, abi::return_register()));
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
            // in-arena — the `Result` wrap in `emit_build_result_inline` is reached by any `TRAP` in
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
            // record (`link_thunk/lower_thunk.rs`, `if function.return_resource`), so the same
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
            // bug-479: a `ThreadWorker` view is carried by POINTER. A parent `Thread`
            // view is a sendable resource (plan-156-D) and took the deep record copy
            // above — the control block it names is shared, not copied. The worker's
            // view never crosses a thread (`TYPE_THREAD_WORKER_NOT_TRANSFERABLE`), so
            // the only materialization that could reach this arm is a same-arena one.
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
        source: impl Into<abi::Reg>,
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
    pub(super) fn copy_resource_to_current_arena(
        &mut self,
        type_: &ParameterType,
        source: impl Into<abi::Reg>,
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
        // bug-755: the receiver's copy is unowned. The sender's owner word is a scope
        // token of the sender's stack; the accepting binding claims the copy.
        self.emit(abi::store_u64(
            abi::ZERO,
            abi::mfb_return(1),
            RESOURCE_OFFSET_OWNER,
        ));
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

    /// True when a record field of type `field_type` is a pointer to a separate
    /// allocation that a whole-block `memcpy` would alias and must therefore be
    /// deep-copied. Inlined fields (`String` and fully-flat nested records) come
    /// along with the block copy; only still-pointer composites (`Union`/`List`/
    /// `Map`/`Result`/`Error`, a not-yet-flat nested record) need the fix.
    ///
    /// A `String` field is always inlined — plan-132 removed the pointer-`String`
    /// records whose `String` fields once needed a separate copy here.
    fn record_field_is_pointer_in(&self, field_type: &ParameterType) -> bool {
        if self.record_field_is_inlined(field_type) {
            return false;
        }
        self.record_field_is_pointer(field_type)
    }

    /// plan-134-F: a record block's pointer edges — every field that is its own allocation —
    /// as `(offset in the block, field type)`, in field order. The ONE enumeration the copy
    /// (`copy_record_fields_into_existing`) and the drop (`graph_drop.rs`) both walk.
    pub(crate) fn record_pointer_edges(
        &self,
        type_: &ParameterType,
    ) -> Result<Vec<(usize, ParameterType)>, String> {
        let fields = self.type_model.record_fields.get(type_).ok_or_else(|| {
            format!("native thread transfer record type '{type_}' does not resolve")
        })?;
        Ok(fields
            .iter()
            .enumerate()
            .filter(|(_, (_, field_type))| self.record_field_is_pointer_in(field_type))
            .map(|(index, (_, field_type))| (index * 8, field_type.clone()))
            .collect())
    }

    /// plan-134-F: `union_base`'s variants with their tags and fields, in tag order — shared
    /// by the copy's and the drop's tag dispatch.
    #[allow(clippy::type_complexity)]
    pub(crate) fn union_variants_by_tag(
        &self,
        union_base: &ParameterType,
    ) -> Result<Vec<(ParameterType, usize, Vec<(String, ParameterType)>)>, String> {
        let mut variants = self
            .type_model
            .variants_for_union(union_base)
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
        Ok(variants)
    }

    pub(super) fn record_needs_pointer_field_fix(&self, record_type: &ParameterType) -> bool {
        self.type_model
            .record_fields
            .get(record_type)
            .map(|fields| {
                fields
                    .iter()
                    .any(|(_, ft)| self.record_field_is_pointer_in(ft))
            })
            .unwrap_or(false)
    }

    fn copy_union_to_current_arena(
        &mut self,
        type_: &ParameterType,
        source: impl Into<abi::Reg>,
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
}
