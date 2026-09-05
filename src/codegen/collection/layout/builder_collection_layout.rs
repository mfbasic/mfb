// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::target::shared::nir::*;
use crate::types::ParameterType;
impl CodeBuilder<'_> {
    pub(crate) fn inline_collection_payload_size(&self, type_: &ParameterType) -> Option<usize> {
        if let Some(fields) = self.type_model.record_fields.get(type_) {
            return Some(8 * fields.len());
        }
        if let Some(union_name) = self.type_model.union_variants.get(type_) {
            return self.inline_collection_payload_size(union_name);
        }
        // A transferred stateful union arrives spelled `Stream STATE Cursor`; the
        // union set is keyed on the bare name (plan-75 gap 3). The `{tag, ptr}`
        // layout is unchanged by the STATE suffix, so size it on the base name.
        let type_ = &base_resource_type(type_);
        if self.type_model.union_names.contains(type_) {
            // A resource variant carries no record fields (validation.rs registers
            // none for `"resource"` variants) but its payload is a single resource
            // handle stored one word after the tag. Count it as one payload word so
            // an all-resource union sizes to its real `{tag@0, ptr@8}` 16-byte
            // layout instead of the tag-only 8 bytes that truncated the handle and
            // read out of block on `RETURN` (bug-141).
            let max_fields = self
                .type_model
                .variants_for_union(type_)
                .map(|variant| {
                    if crate::codegen::builtins::is_resource_type(&variant) {
                        1
                    } else {
                        self.type_model
                            .union_variant_fields
                            .get(variant)
                            .map(Vec::len)
                            .unwrap_or(0)
                    }
                })
                .max()
                .unwrap_or(0);
            return Some(8 * (1 + max_fields));
        }
        None
    }

    pub(crate) fn is_pointer_collection_payload_type(&self, type_: &ParameterType) -> bool {
        // A resource handle is a single 8-byte pointer to its record; a collection
        // slot stores a copy of that pointer exactly like any other pointer
        // payload (§15.6). Resource *unions* carry a tag and are not pointer
        // payloads. A **flat** nested collection is inlined as its own block in
        // the data region (plan-02 §4.4, Phase 5a); only a *non-flat* nested
        // collection (one that itself embeds a pointer/resource payload) stays a
        // pointer handle.
        if typed_is_collection_type(type_) {
            return !self.type_is_memcpy_copyable(type_);
        }
        crate::codegen::builtins::is_resource_type(&type_)
            && !self.type_model.union_names.contains(type_)
    }

    /// Alignment, in bytes, that a packed collection payload of `type_` requires
    /// in the data region. 8-byte scalars (`Integer`/`Float`/`Fixed`), native
    /// collection/object pointers, and inline record/union slot payloads must
    /// begin at 8-byte boundaries; 1-byte scalars (`Boolean`/`Byte`) and UTF-8
    /// `String` bytes have no alignment requirement. `memory_layouts.md`
    /// (Scalar Storage) requires every payload to begin at an offset valid for
    /// its type, with padding bytes unobservable.
    pub(crate) fn collection_payload_alignment(&self, type_: &ParameterType) -> usize {
        match type_ {
            ParameterType::Boolean | ParameterType::Byte | ParameterType::String => 1,
            ParameterType::Integer
            | ParameterType::Float
            | ParameterType::Fixed
            | ParameterType::Money => 8,
            // A Scalar is a 4-byte codepoint payload with alignment 4 (plan-41-C).
            // A bare nominal, so matched by name (see `list_element_is_fixed_width`).
            type_ if type_.is_named("Scalar") => 4,
            // A function value is an 8-byte code/closure pointer (bug-73).
            other if matches!(other, ParameterType::Func(..)) => 8,
            other if self.is_pointer_collection_payload_type(other) => 8,
            other if self.inline_collection_payload_size(other).is_some() => 8,
            // An inlined flat collection block begins with `U64` header fields.
            other if typed_is_collection_type(other) => 8,
            _ => 1,
        }
    }

    /// Inter-element padding alignment for a homogeneous **list** payload of
    /// `type_`. Fixed-size payloads (scalars, pointers, fixed records/unions,
    /// byte-addressed `String`) are always a whole multiple of their own
    /// alignment, so consecutive elements pack with no gap and need no rounding
    /// — those return 1 (a no-op) so primitive/pointer lists stay byte-identical.
    /// Only a *variable-length* element — a record with an inlined `String`
    /// field, a data union, or a flat nested collection — can end on a non-8
    /// boundary and leave the next element's `U64` slots unaligned; those round
    /// up to 8 (bug-147.4). The allocation-size pass and the writer both apply
    /// this identical rounding, and each element's absolute offset is recorded
    /// per-entry, so the reader (which loads the stored offset) stays in lockstep.
    pub(crate) fn list_element_padding_alignment(&self, type_: &ParameterType) -> usize {
        if self.record_has_inline_data(type_)
            || self.union_is_data(type_)
            || (typed_is_collection_type(type_) && self.type_is_memcpy_copyable(type_))
        {
            8
        } else {
            1
        }
    }

    /// Rounds the unsigned offset stored at `slot` up to `alignment`. A no-op
    /// for `alignment <= 1`. Uses temporary scratch vregs (colored by regalloc),
    /// so it does not disturb the surrounding collection-writer code's values.
    pub(crate) fn emit_align_offset_slot(&mut self, slot: usize, alignment: usize) {
        if alignment <= 1 {
            return;
        }
        let scratch12 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let mask = !((alignment - 1) as u64);
        self.emit(abi::load_u64(&scratch12, abi::stack_pointer(), slot));
        self.emit(abi::add_immediate(&scratch12, &scratch12, alignment - 1));
        self.emit(abi::move_immediate(
            &scratch13,
            "Integer",
            &mask.to_string(),
        ));
        self.emit(abi::and_registers(&scratch12, &scratch12, &scratch13));
        self.emit(abi::store_u64(&scratch12, abi::stack_pointer(), slot));
    }

    /// Rounds the unsigned offset held in `reg` up to `alignment`, using
    /// `scratch` for the alignment mask. A no-op for `alignment <= 1`.
    pub(crate) fn emit_align_offset_register(
        &mut self,
        reg: impl Into<Operand>,
        alignment: usize,
        scratch: impl Into<Operand>,
    ) {
        if alignment <= 1 {
            return;
        }
        let reg = reg.into();
        let scratch = scratch.into();
        let mask = !((alignment - 1) as u64);
        self.emit(abi::add_immediate(reg.clone(), reg.clone(), alignment - 1));
        self.emit(abi::move_immediate(
            scratch.clone(),
            "Integer",
            &mask.to_string(),
        ));
        self.emit(abi::and_registers(reg.clone(), reg, scratch));
    }

    /// Block-copy `len` bytes from `src` to `dst`, advancing both pointers.
    /// Copies 8 bytes per iteration with a byte tail for the remainder — an
    /// order-of-magnitude fewer iterations than a pure byte loop on payloads
    /// larger than a word. `len` is preserved (a private scratch-vreg copy drives
    /// the loop); `src`/`dst` are advanced past the copied region; the loop's
    /// scratch vregs are clobbered. The destination region must not overlap the source ahead of
    /// it (it never does here — collection buffers are freshly allocated).
    pub(crate) fn emit_copy_bytes(
        &mut self,
        dst: impl Into<Operand>,
        src: impl Into<Operand>,
        len: impl Into<Operand>,
        prefix: &str,
    ) {
        let scratch13 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        let remaining = &scratch13;
        self.emit(abi::move_register(remaining, len));
        self.emit_block_copy_advance(dst, src, remaining, &scratch14, prefix);
    }

    /// Word-then-byte block copy that advances `dst`, `src`, and consumes
    /// `remaining` (decremented to 0). `scratch` holds the in-flight word/byte
    /// and is clobbered. Shared by `emit_copy_bytes` and the collection
    /// entry/payload copy loops so every payload move is word-sized.
    pub(crate) fn emit_block_copy_advance(
        &mut self,
        dst: impl Into<Operand>,
        src: impl Into<Operand>,
        remaining: impl Into<Operand>,
        scratch: impl Into<Operand>,
        prefix: &str,
    ) {
        let dst = dst.into();
        let src = src.into();
        let remaining = remaining.into();
        let scratch = scratch.into();
        let word_loop = self.label(&format!("{prefix}_wloop"));
        let byte_tail = self.label(&format!("{prefix}_btail"));
        let done_label = self.label(&format!("{prefix}_done"));
        self.emit(abi::label(&word_loop));
        self.emit(abi::compare_immediate(remaining.clone(), "8"));
        self.emit(abi::branch_lo(&byte_tail));
        self.emit(abi::load_u64(scratch.clone(), src.clone(), 0));
        self.emit(abi::store_u64(scratch.clone(), dst.clone(), 0));
        self.emit(abi::add_immediate(src.clone(), src.clone(), 8));
        self.emit(abi::add_immediate(dst.clone(), dst.clone(), 8));
        self.emit(abi::subtract_immediate(
            remaining.clone(),
            remaining.clone(),
            8,
        ));
        self.emit(abi::branch(&word_loop));
        self.emit(abi::label(&byte_tail));
        self.emit(abi::compare_immediate(remaining.clone(), "0"));
        self.emit(abi::branch_eq(&done_label));
        self.emit(abi::load_u8(scratch.clone(), src.clone(), 0));
        self.emit(abi::store_u8(scratch, dst.clone(), 0));
        self.emit(abi::add_immediate(src.clone(), src, 1));
        self.emit(abi::add_immediate(dst.clone(), dst, 1));
        self.emit(abi::subtract_immediate(remaining.clone(), remaining, 1));
        self.emit(abi::branch(&byte_tail));
        self.emit(abi::label(&done_label));
    }

    /// Copy `remaining` bytes **backwards**, from `src_end`/`dst_end` (each one
    /// past the last byte of its range) down toward the start. Both cursors are
    /// left at the *start* of their ranges and `remaining` at zero, mirroring
    /// [`Self::emit_block_copy_advance`]'s forward contract.
    ///
    /// Backwards is the whole point: this exists to shift a list's data region
    /// **up** by one element (bug-365's ordered `prepend`), where source and
    /// destination overlap and a forward copy would smear the first element over
    /// the whole region. The overlap only bites when the shift distance is less
    /// than the region length — that is, at element counts above one — so a
    /// forward copy here would look correct on the 1–2 element lists a small test
    /// uses and corrupt every real one.
    pub(crate) fn emit_block_copy_backward(
        &mut self,
        dst_end: impl Into<Operand>,
        src_end: impl Into<Operand>,
        remaining: impl Into<Operand>,
        scratch: impl Into<Operand>,
        prefix: &str,
    ) {
        let dst_end = dst_end.into();
        let src_end = src_end.into();
        let remaining = remaining.into();
        let scratch = scratch.into();
        let word_loop = self.label(&format!("{prefix}_bwloop"));
        let byte_tail = self.label(&format!("{prefix}_bbtail"));
        let done_label = self.label(&format!("{prefix}_bdone"));
        self.emit(abi::label(&word_loop));
        self.emit(abi::compare_immediate(remaining.clone(), "8"));
        self.emit(abi::branch_lo(&byte_tail));
        self.emit(abi::subtract_immediate(src_end.clone(), src_end.clone(), 8));
        self.emit(abi::subtract_immediate(dst_end.clone(), dst_end.clone(), 8));
        self.emit(abi::load_u64(scratch.clone(), src_end.clone(), 0));
        self.emit(abi::store_u64(scratch.clone(), dst_end.clone(), 0));
        self.emit(abi::subtract_immediate(
            remaining.clone(),
            remaining.clone(),
            8,
        ));
        self.emit(abi::branch(&word_loop));
        self.emit(abi::label(&byte_tail));
        self.emit(abi::compare_immediate(remaining.clone(), "0"));
        self.emit(abi::branch_eq(&done_label));
        self.emit(abi::subtract_immediate(src_end.clone(), src_end.clone(), 1));
        self.emit(abi::subtract_immediate(dst_end.clone(), dst_end.clone(), 1));
        self.emit(abi::load_u8(scratch.clone(), src_end, 0));
        self.emit(abi::store_u8(scratch, dst_end, 0));
        self.emit(abi::subtract_immediate(remaining.clone(), remaining, 1));
        self.emit(abi::branch(&byte_tail));
        self.emit(abi::label(&done_label));
    }

    /// Emit code computing the **total byte size** of an already-flat block of
    /// `type_` located at `ptr_reg`, into `out_reg` (`scratch` is clobbered).
    /// plan-02 §4.1: a flat block is self-describing, so copy and free can be
    /// generic. This is the size primitive both rely on. `ptr_reg`, `out_reg`,
    /// and `scratch` must be three distinct registers; `ptr_reg` is preserved.
    ///
    /// Phase 1 supports the types that are already pointer-free and
    /// self-describing — `String` (length word + bytes + NUL) and collections
    /// (header + lookup table + data region). Later phases extend this to
    /// records and unions as those gain an explicit size word.
    pub(crate) fn emit_flat_block_size(
        &mut self,
        type_: &ParameterType,
        ptr_reg: impl Into<Operand>,
        out_reg: impl Into<Operand>,
        scratch: impl Into<Operand>,
    ) -> Result<(), String> {
        let ptr_reg = ptr_reg.into();
        let out_reg = out_reg.into();
        let scratch = scratch.into();
        match type_ {
            ParameterType::String => {
                // byteLength(+0) + 8 (length word) + 1 (trailing NUL).
                self.emit(abi::load_u64(out_reg.clone(), ptr_reg, 0));
                self.emit(abi::add_immediate(out_reg.clone(), out_reg, 9));
                Ok(())
            }
            other if typed_is_collection_type(other) => {
                // header + capacity * entryStride + dataCapacity (+ a map's
                // bucket region).
                //
                // The stride MUST match what the allocator reserved. This is the
                // size `arena_free` releases on scope drop, so a kind-0 stride on
                // a kind-2 block frees `capacity * 40` bytes past the end and
                // corrupts the free list — that is bug-02's exact failure mode,
                // and plan-57-D names this function as the one edit whose
                // mistake is heap corruption rather than a wrong value.
                let element = typed_list_element_type(other)
                    .cloned()
                    .unwrap_or_else(|| ParameterType::named(""));
                let stride = list_entry_stride(&element);
                self.emit(abi::load_u64(
                    out_reg.clone(),
                    ptr_reg.clone(),
                    COLLECTION_OFFSET_CAPACITY,
                ));
                self.emit(abi::move_immediate(
                    scratch.clone(),
                    "Integer",
                    &stride.to_string(),
                ));
                self.emit(abi::multiply_registers(
                    out_reg.clone(),
                    out_reg.clone(),
                    scratch.clone(),
                ));
                self.emit(abi::add_immediate(
                    out_reg.clone(),
                    out_reg.clone(),
                    COLLECTION_HEADER_SIZE,
                ));
                self.emit(abi::load_u64(
                    scratch.clone(),
                    ptr_reg.clone(),
                    COLLECTION_OFFSET_DATA_CAPACITY,
                ));
                self.emit(abi::add_registers(
                    out_reg.clone(),
                    out_reg.clone(),
                    scratch.clone(),
                ));
                // A map block also carries its hash-index bucket region
                // (2 * capacity u64 buckets = capacity << 4 bytes) past the
                // data region — the same region emit_reserve_map_buckets adds
                // on every allocation path. Omitting it here sized
                // record-embedded map construction, copies, and frees
                // 16*capacity bytes short, so the lazy `build_buckets` rebuild
                // wrote its bucket markers past the block into the adjacent
                // heap chunk (bug-02: regex `prog.names` corrupted the arena
                // free list).
                // A `Map` *or* a `Set` carries the bucket region (plan-63); a
                // `List` does not. `collection_has_buckets` is the single predicate
                // so a Set is never sized without the region it was allocated with.
                if matches!(other, ParameterType::MapOf(..) | ParameterType::SetOf(_)) {
                    self.emit(abi::load_u64(
                        scratch.clone(),
                        ptr_reg,
                        COLLECTION_OFFSET_CAPACITY,
                    ));
                    self.emit(abi::shift_left_immediate(
                        scratch.clone(),
                        scratch.clone(),
                        4,
                    ));
                    self.emit(abi::add_registers(out_reg.clone(), out_reg, scratch));
                }
                Ok(())
            }
            other => Err(format!(
                "flat block size is not available for type '{other}'"
            )),
        }
    }

    /// Generic flat-block copy (plan-02 §4.1): `size = flat_block_size(src)`,
    /// `dst = arena_alloc(size, 8)`, `memcpy(dst, src, size)`. Because a flat
    /// block has no internal pointers, the byte copy **is** a deep copy. Valid
    /// only for types `emit_flat_block_size` supports; returns the destination
    /// pointer in a fresh register.
    pub(crate) fn copy_flat_block(
        &mut self,
        type_: &ParameterType,
        source: impl Into<Operand>,
    ) -> Result<VirtualRegister, String> {
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        // A collection value is copied **shrink-to-fit** (plan-01 §4.3): headroom
        // is a property of a mutable working buffer, never of a value, so a copy
        // drops any spare capacity. A whole-block `memcpy` would carry the
        // headroom (and the gap between the live entries and the data region)
        // into the snapshot; the tight copy compacts both.
        if typed_is_collection_type(&type_) {
            return self.copy_collection_tight(type_, source);
        }
        let source_slot = self.allocate_stack_object("flat_copy_source", 8);
        let size_slot = self.allocate_stack_object("flat_copy_size", 8);
        let result_slot = self.allocate_stack_object("flat_copy_result", 8);
        let alloc_ok = self.label("flat_copy_alloc_ok");
        self.emit(abi::store_u64(source, abi::stack_pointer(), source_slot));
        // Size the flat block from its pointer slot. This dispatcher handles every
        // flat type — `String`, collection, record (walk), and data union
        // (`size@8`) — so `copy_flat_block` is a sound deep copy for any
        // `type_is_memcpy_copyable` value (plan-02 §4.1).
        self.emit_inlined_block_size_from_ptr_slot(type_, source_slot, size_slot)?;
        // plan-71-C Family-1a: the size is arg 0 of the arena-alloc call — emit it into
        // `%arg0`, not `return_register()` (`%ret0`). Byte-identical; clears
        // `builder_collection_layout.rs:332`.
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
        let dst_base = self.temporary_vreg();
        self.emit(abi::load_u64(&dst_base, abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), size_slot));
        self.emit_copy_bytes(&dst_base, &scratch9, &scratch10, "flat_copy");
        let result = self.allocate_register();
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(result)
    }

    /// Shrink-to-fit deep copy of a flat collection (plan-01 §4.3): allocate
    /// exactly `HEADER + count*ENTRY + dataLength`, write a tight header
    /// (`capacity == count`, `dataCapacity == dataLength`), then copy the live
    /// lookup entries and the data region verbatim. Entry value/key offsets are
    /// relative to the data base, so the verbatim data copy keeps them valid; the
    /// source's spare capacity slots and any trailing data slack are dropped.
    /// Returns the destination pointer in a fresh register.
    pub(crate) fn copy_collection_tight(
        &mut self,
        type_: &ParameterType,
        source: impl Into<Operand>,
    ) -> Result<VirtualRegister, String> {
        // A kind-2 source has no entry array: the tight copy neither reserves
        // one nor copies it, and the whole block is header + data (plan-57-D).
        let element = typed_list_element_type(type_)
            .cloned()
            .unwrap_or_else(|| ParameterType::named(""));
        let tight_stride = list_entry_stride(&element);
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        let scratch16 = self.temporary_vreg();
        let scratch17 = self.temporary_vreg();
        let scratch20 = self.temporary_vreg();
        let scratch21 = self.temporary_vreg();
        let scratch22 = self.temporary_vreg();
        let layout = CollectionTypeLayout::from_type(type_)
            .ok_or_else(|| format!("native code collection type '{type_}' is not supported"))?;
        let source_slot = self.allocate_stack_object("tight_copy_source", 8);
        let result_slot = self.allocate_stack_object("tight_copy_result", 8);
        let alloc_ok = self.label("tight_copy_alloc_ok");
        self.emit(abi::store_u64(source, abi::stack_pointer(), source_slot));

        // alloc size = HEADER + count * ENTRY + dataLength.
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(
            &scratch10,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::move_immediate(
            &scratch11,
            "Integer",
            &tight_stride.to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch12, &scratch9, &scratch11));
        self.emit(abi::add_immediate(
            abi::return_register(),
            &scratch12,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(
            abi::return_register(),
            abi::return_register(),
            &scratch10,
        ));
        // A map's *or* set's tight copy reserves its (count-sized) hash bucket
        // region; x9 still holds count. The copy is marked not-ready so the
        // buckets are recomputed on first probe (no stale offsets across
        // copy/transfer). `collection_has_buckets` keeps Set and Map in lockstep.
        self.emit_reserve_map_buckets(
            matches!(&type_, ParameterType::MapOf(..) | ParameterType::SetOf(_)),
            &scratch9,
            abi::return_register(),
            &scratch10,
        );
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

        // Tight header: capacity == count, dataCapacity == dataLength.
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(
            &scratch10,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        let block_base = self.temporary_vreg();
        self.emit(abi::load_u64(
            &block_base,
            abi::stack_pointer(),
            result_slot,
        ));
        self.emit_write_list_header_from_registers(&layout, &block_base, &scratch9, &scratch10);

        // Copy the live lookup entries verbatim (count * ENTRY bytes).
        self.emit(abi::load_u64(
            &block_base,
            abi::stack_pointer(),
            result_slot,
        ));
        if tight_stride != 0 {
            self.emit(abi::add_immediate(
                &scratch17,
                &block_base,
                COLLECTION_HEADER_SIZE,
            ));
            self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), source_slot));
            self.emit(abi::add_immediate(
                &scratch20,
                &scratch8,
                COLLECTION_HEADER_SIZE,
            ));
            self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
            self.emit(abi::move_immediate(
                &scratch16,
                "Integer",
                &tight_stride.to_string(),
            ));
            self.emit(abi::multiply_registers(&scratch21, &scratch9, &scratch16));
            self.emit_block_copy_advance(
                &scratch17,
                &scratch20,
                &scratch21,
                &scratch22,
                "tight_copy_entries",
            );
        }

        // Copy the data region verbatim (dataLength bytes). Source base is
        // capacity-based (it may have headroom); destination base is count-based
        // (tight) — both resolve through emit_collection_data_pointer.
        self.emit(abi::load_u64(
            &block_base,
            abi::stack_pointer(),
            result_slot,
        ));
        self.emit_collection_data_pointer_for(&scratch17, &block_base, &element);
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), source_slot));
        self.emit_collection_data_pointer_for(&scratch20, &scratch8, &element);
        self.emit(abi::load_u64(
            &scratch14,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit_block_copy_advance(
            &scratch17,
            &scratch20,
            &scratch14,
            &scratch22,
            "tight_copy_data",
        );

        let result = self.allocate_register();
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(result)
    }

    /// Built-in records that are constructed by bespoke runtime helpers (which
    /// still write their `String` fields as pointers) rather than the codegen
    /// `Constructor` path. They are excluded from the inline-`String` record
    /// layout so that machinery — and field reads of values it produces — stay on
    /// the pointer layout consistently (plan-02 Phase 2):
    ///   - `Error`/`ErrorLoc`: the fallible-call ABI, trap materialization, `FAIL`.
    ///   - `net::Address`/`udp::Datagram`/`audio::AudioDevice`: the socket and
    ///     audio-device helpers (`emit_address_from_sockaddr`, etc.).
    ///
    /// Every other record inlines its `String` fields.
    pub(crate) fn is_pointer_string_record(&self, type_: &ParameterType) -> bool {
        is_pointer_string_record(type_)
    }

    /// True when `field_type` occupies a record slot as a pointer to a separate
    /// allocation (nested record/union/collection/`Result`/`Error`). These stay
    /// pointers in Phase 2 (later phases inline them).
    pub(crate) fn record_field_is_pointer(&self, field_type: &ParameterType) -> bool {
        record_field_is_pointer(&self.type_model, field_type)
    }

    /// True when a `memcpy` of this value's block is a correct **copy within one
    /// thread**. Copyable types: scalars, `String`, a record whose every field is
    /// copyable, a **data** union whose every variant is copyable, a collection
    /// whose payloads are copyable, and a **resource handle** — the 8-byte slot
    /// is a pointer to the one resource record, and copying it aliases that
    /// resource rather than duplicating it (§15.6), which is the same rule
    /// [`Self::is_pointer_collection_payload_type`] already applies to a
    /// collection slot. Not copyable: resource *unions* (a `{tag, ptr}` block,
    /// not a plain slot), `Result` of a non-copyable payload, `Error`/`ErrorLoc`
    /// and the other helper-built pointer-`String` records, and any recursive
    /// type (broken by the `visited` path set, so a cyclic type stays a pointer).
    pub(crate) fn type_is_memcpy_copyable(&self, type_: &ParameterType) -> bool {
        type_is_memcpy_copyable(&self.type_model, type_)
    }

    /// True when this value's block may be **relocated into another thread's
    /// arena**. Strictly stronger than [`Self::type_is_memcpy_copyable`]: arenas
    /// are per-thread, so a resource handle anywhere inside the block would
    /// arrive pointing into the *sender's* arena. A resource — bare, `RES`-marked,
    /// or nested in a field or payload — makes this false; everything else
    /// answers exactly as memcpy-copyability does.
    pub(crate) fn type_is_arena_transferable(&self, type_: &ParameterType) -> bool {
        type_is_arena_transferable(&self.type_model, type_)
    }

    /// True when field `field_type` of `record_type` is inlined into the record's
    /// trailing data region (the slot holds a block-relative offset): an inlined
    /// `String`, or a fully-flat composite — a nested record, a flat data union,
    /// or a flat collection (plan-02 §4.2–§4.4). Scalars stay inline in the slot;
    /// not-yet-flat composites stay pointers.
    pub(crate) fn record_field_is_inlined(
        &self,
        record_type: &ParameterType,
        field_type: &ParameterType,
    ) -> bool {
        record_field_is_inlined(&self.type_model, record_type, field_type)
    }

    /// True when `record_type` has at least one inlined field (so its block is
    /// variable-length and carries a trailing data region).
    pub(crate) fn record_has_inline_data(&self, record_type: &ParameterType) -> bool {
        if self.is_pointer_string_record(record_type) {
            return false;
        }
        self.type_model
            .record_fields
            .get(record_type)
            .cloned()
            .map(|fields| {
                fields
                    .iter()
                    .any(|(_, ft)| self.record_field_is_inlined(record_type, ft))
            })
            .unwrap_or(false)
    }

    /// Emit the byte size of an inlined field value of `field_type` whose pointer
    /// is in `ptr_slot`, into `out_slot`. An inlined `String` is `len + 9`; an
    /// inlined nested record recurses through `emit_record_block_size_to_slot`.
    /// Clobbers its temporary scratch vregs (and the recursion's scratch).
    pub(crate) fn emit_inlined_block_size_from_ptr_slot(
        &mut self,
        field_type: &ParameterType,
        ptr_slot: usize,
        out_slot: usize,
    ) -> Result<(), String> {
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        if *field_type == ParameterType::String {
            self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), ptr_slot));
            self.emit(abi::load_u64(&scratch9, &scratch8, 0));
            self.emit(abi::add_immediate(&scratch9, &scratch9, 9));
            self.emit(abi::store_u64(&scratch9, abi::stack_pointer(), out_slot));
            Ok(())
        } else if self.type_model.record_fields.contains_key(field_type) {
            self.emit_record_block_size_to_slot(field_type, ptr_slot, out_slot)
        } else if self.union_is_data(field_type) || matches!(field_type, ParameterType::ResultOf(_))
        {
            // A data union and a flat `Result` are self-describing: their `size`
            // word lives at +8 (plan-02 §4.3).
            self.emit_data_union_size_to_slot(ptr_slot, out_slot);
            Ok(())
        } else if typed_is_collection_type(field_type) {
            self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), ptr_slot));
            self.emit_flat_block_size(field_type, &scratch8, &scratch9, &scratch10)?;
            self.emit(abi::store_u64(&scratch9, abi::stack_pointer(), out_slot));
            Ok(())
        } else {
            Err(format!(
                "native inlined field size not available for type '{field_type}'"
            ))
        }
    }

    /// True when `type_` is a **data** union (all variants are data records, no
    /// resource variants). Data unions use the flat `{tag, size, data}` layout
    /// (plan-02 §4.3); resource unions keep `{tag, resource-ptr}` and are never
    /// reshaped. A union is all-data or all-resource (`rules.rs:790`).
    pub(crate) fn union_is_data(&self, type_: &ParameterType) -> bool {
        union_is_data(&self.type_model, type_)
    }

    /// Total byte size of a data union into `out_slot`: the `size` word at `+8`
    /// (plan-02 §4.3). `ptr_slot` holds the union pointer. Clobbers a scratch vreg.
    pub(crate) fn emit_data_union_size_to_slot(&mut self, ptr_slot: usize, out_slot: usize) {
        let scratch8 = self.temporary_vreg();
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), ptr_slot));
        self.emit(abi::load_u64(&scratch8, &scratch8, 8));
        self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), out_slot));
    }

    /// Wrap a built variant record (pointer in `record_ptr_slot`) into a data
    /// union value `{U64 tag@0, U64 size@8, variant-record-block@16}` (plan-02
    /// §4.3): the variant's flat record block is inlined at `+16`. Returns a
    /// register holding the union pointer.
    pub(crate) fn emit_wrap_record_in_union(
        &mut self,
        member_type: &ParameterType,
        tag: usize,
        record_ptr_slot: usize,
    ) -> Result<VirtualRegister, String> {
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let inner_size_slot = self.allocate_stack_object("union_wrap_inner_size", 8);
        self.emit_record_block_size_to_slot(member_type, record_ptr_slot, inner_size_slot)?;
        let size_slot = self.allocate_stack_object("union_wrap_size", 8);
        self.emit(abi::load_u64(
            &scratch8,
            abi::stack_pointer(),
            inner_size_slot,
        ));
        self.emit(abi::add_immediate(&scratch8, &scratch8, 16));
        self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), size_slot));
        let result_slot = self.allocate_stack_object("union_wrap_result", 8);
        let alloc_ok = self.label("union_wrap_alloc_ok");
        // plan-71-C Family-1a: alloc size is arg 0 → `%arg0`, not `return_register()`.
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
        // tag@0, size@8.
        self.emit(abi::move_immediate(
            &scratch9,
            abi::IMMEDIATE_CLASS_UNION_TAG,
            &tag.to_string(),
        ));
        self.emit(abi::store_u64(&scratch9, abi::mfb_return(1), 0));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), size_slot));
        self.emit(abi::store_u64(&scratch9, abi::mfb_return(1), 8));
        // Inline the variant record block at +16.
        self.emit(abi::load_u64(&scratch11, abi::stack_pointer(), result_slot));
        self.emit(abi::add_immediate(&scratch11, &scratch11, 16));
        self.emit(abi::load_u64(
            &scratch12,
            abi::stack_pointer(),
            record_ptr_slot,
        ));
        self.emit(abi::load_u64(
            &scratch13,
            abi::stack_pointer(),
            inner_size_slot,
        ));
        self.emit_copy_bytes(&scratch11, &scratch12, &scratch13, "union_wrap_block");
        let register = self.allocate_register();
        self.emit(abi::load_u64(&register, abi::stack_pointer(), result_slot));
        Ok(register)
    }

    /// Emit the **total byte size** of an inlined record of `record_type` whose
    /// base pointer is in `base_slot`, into `out_slot`. Walks the fixed slot
    /// region (`8*fieldCount`) plus each inlined sub-block (8-aligned, in field
    /// order) — an inlined `String` (`len + 9`) or a fully-flat nested record
    /// (recursively) — matching `emit_build_inlined_record`'s layout. Clobbers its
    /// temporary scratch vregs (and the recursion's scratch). Recursion is bounded by the
    /// static type nesting (a record cannot directly contain itself).
    pub(crate) fn emit_record_block_size_to_slot(
        &mut self,
        record_type: &ParameterType,
        base_slot: usize,
        out_slot: usize,
    ) -> Result<(), String> {
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let fields = self
            .type_model
            .record_fields
            .get(record_type)
            .cloned()
            .ok_or_else(|| format!("native record type '{record_type}' does not resolve"))?;
        let fixed = 8 * fields.len();
        self.emit(abi::move_immediate(
            &scratch8,
            "Integer",
            &fixed.to_string(),
        ));
        self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), out_slot));
        for (index, (_, field_type)) in fields.iter().enumerate() {
            if !self.record_field_is_inlined(record_type, field_type) {
                continue;
            }
            // The field's own offset word at `8*index` is authoritative — and `0`
            // is the "sub-block absent" sentinel, since a real inlined block can
            // never start before the fixed slot region. `Error.source` is the live
            // case: an error with no origin (`ErrorLoc*` null, as every `LINK`
            // thunk returns) is built as `{code, message}` with source-offset 0
            // and nothing written past the message. Walking the running offset
            // unconditionally then sized a phantom `ErrorLoc` out of whatever
            // followed the block, so freeing that error handed `arena_free` a
            // garbage size and corrupted the free list (bug-371).
            let absent = self.label("record_size_field_absent");
            let off_slot = self.allocate_stack_object("record_size_field_off", 8);
            self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), base_slot));
            self.emit(abi::load_u64(&scratch9, &scratch8, 8 * index));
            self.emit(abi::store_u64(&scratch9, abi::stack_pointer(), off_slot));
            self.emit(abi::compare_immediate(&scratch9, "0"));
            self.emit(abi::branch_eq(&absent));
            // inner_base = base + offset (where this sub-block begins).
            let inner_base_slot = self.allocate_stack_object("record_size_inner_base", 8);
            self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), base_slot));
            self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), off_slot));
            self.emit(abi::add_registers(&scratch8, &scratch8, &scratch9));
            self.emit(abi::store_u64(
                &scratch8,
                abi::stack_pointer(),
                inner_base_slot,
            ));
            let inner_size_slot = self.allocate_stack_object("record_size_inner_size", 8);
            self.emit_inlined_block_size_from_ptr_slot(
                field_type,
                inner_base_slot,
                inner_size_slot,
            )?;
            // out = offset + sub-block size — the sub-block's end, which is the
            // block's end for the last present field.
            self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), off_slot));
            self.emit(abi::load_u64(
                &scratch8,
                abi::stack_pointer(),
                inner_size_slot,
            ));
            self.emit(abi::add_registers(&scratch9, &scratch9, &scratch8));
            self.emit(abi::store_u64(&scratch9, abi::stack_pointer(), out_slot));
            self.emit(abi::label(&absent));
        }
        Ok(())
    }

    /// Build a flat record of `record_type` from `field_slots` (one stack slot
    /// per field, in field order). A `String` field slot holds a pointer to a
    /// source `String` block (its bytes are inlined into the record's data
    /// region and the slot stores the block-relative offset); every other field
    /// slot holds the scalar value or pointer, stored inline at `8*index`.
    /// Returns a register holding the new record pointer. plan-02 §4.2.
    pub(crate) fn emit_build_inlined_record(
        &mut self,
        record_type: &ParameterType,
        field_slots: &[usize],
    ) -> Result<VirtualRegister, String> {
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let fields = self
            .type_model
            .record_fields
            .get(record_type)
            .cloned()
            .ok_or_else(|| format!("native record type '{record_type}' does not resolve"))?;
        if fields.len() != field_slots.len() {
            return Err(format!(
                "native record '{record_type}' construction expected {} fields, got {}",
                fields.len(),
                field_slots.len()
            ));
        }
        let fixed = 8 * fields.len();
        let size_slot = self.allocate_stack_object("record_build_size", 8);
        let result_slot = self.allocate_stack_object("record_build_result", 8);
        let cursor_slot = self.allocate_stack_object("record_build_cursor", 8);
        let alloc_ok = self.label("record_build_alloc_ok");

        // Pass 1: total size = fixed slots + each inlined sub-block.
        self.emit(abi::move_immediate(
            &scratch8,
            "Integer",
            &fixed.to_string(),
        ));
        self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), size_slot));
        for (index, (_, field_type)) in fields.iter().enumerate() {
            if !self.record_field_is_inlined(record_type, field_type) {
                continue;
            }
            self.emit_align_offset_slot(size_slot, 8);
            let block_size_slot = self.allocate_stack_object("record_build_block_size", 8);
            self.emit_inlined_block_size_from_ptr_slot(
                field_type,
                field_slots[index],
                block_size_slot,
            )?;
            self.emit(abi::load_u64(
                &scratch9,
                abi::stack_pointer(),
                block_size_slot,
            ));
            self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), size_slot));
            self.emit(abi::add_registers(&scratch8, &scratch8, &scratch9));
            self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), size_slot));
        }

        // plan-71-C Family-1a: alloc size is arg 0 → `%arg0`, not `return_register()`.
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

        // Pass 2: write slots; inline each flat sub-block into the data region.
        self.emit(abi::move_immediate(
            &scratch8,
            "Integer",
            &fixed.to_string(),
        ));
        self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), cursor_slot));
        for (index, (_, field_type)) in fields.iter().enumerate() {
            if self.record_field_is_inlined(record_type, field_type) {
                self.emit_align_offset_slot(cursor_slot, 8);
                // Slot stores the block-relative offset of the inlined sub-block.
                self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), result_slot));
                self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), cursor_slot));
                self.emit(abi::store_u64(&scratch9, &scratch10, 8 * index));
                // Compute the sub-block's byte size from the source pointer.
                let block_size_slot = self.allocate_stack_object("record_fill_block_size", 8);
                self.emit_inlined_block_size_from_ptr_slot(
                    field_type,
                    field_slots[index],
                    block_size_slot,
                )?;
                // dest = base + offset; copy `block_size` bytes from the source.
                self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), result_slot));
                self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), cursor_slot));
                self.emit(abi::add_registers(&scratch11, &scratch10, &scratch9));
                self.emit(abi::load_u64(
                    &scratch12,
                    abi::stack_pointer(),
                    field_slots[index],
                ));
                self.emit(abi::load_u64(
                    &scratch13,
                    abi::stack_pointer(),
                    block_size_slot,
                ));
                self.emit_copy_bytes(&scratch11, &scratch12, &scratch13, "record_inline_block");
                // Advance the cursor by the same block length.
                self.emit(abi::load_u64(
                    &scratch13,
                    abi::stack_pointer(),
                    block_size_slot,
                ));
                self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), cursor_slot));
                self.emit(abi::add_registers(&scratch9, &scratch9, &scratch13));
                self.emit(abi::store_u64(&scratch9, abi::stack_pointer(), cursor_slot));
            } else {
                self.emit(abi::load_u64(
                    &scratch9,
                    abi::stack_pointer(),
                    field_slots[index],
                ));
                self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), result_slot));
                self.emit(abi::store_u64(&scratch9, &scratch10, 8 * index));
            }
        }
        let register = self.allocate_register();
        self.emit(abi::load_u64(&register, abi::stack_pointer(), result_slot));
        Ok(register)
    }

    pub(crate) fn materialize_inline_value_in_arena(
        &mut self,
        type_: &ParameterType,
        source: impl Into<Operand>,
    ) -> Result<VirtualRegister, String> {
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        // A record with inlined fields or a data union is variable-length: size
        // its flat block at runtime, then block-copy it (plan-02 §4.2/§4.3). The
        // inlined data comes along; pointer fields keep the same shallow-share
        // semantics as the fixed path below.
        let is_record_inline = self.record_has_inline_data(type_);
        let is_data_union = self.union_is_data(type_);
        if is_record_inline || is_data_union {
            let source_slot = self.allocate_stack_object("inline_value_source", 8);
            let size_slot = self.allocate_stack_object("inline_value_size", 8);
            let result_slot = self.allocate_stack_object("inline_value_result", 8);
            let alloc_ok = self.label("inline_value_alloc_ok");
            self.emit(abi::store_u64(source, abi::stack_pointer(), source_slot));
            if is_data_union {
                self.emit_data_union_size_to_slot(source_slot, size_slot);
            } else {
                self.emit_record_block_size_to_slot(type_, source_slot, size_slot)?;
            }
            // plan-71-C Family-1a: alloc size is arg 0 → `%arg0`, not `return_register()`.
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
            let dst_base = self.temporary_vreg();
            self.emit(abi::load_u64(&dst_base, abi::stack_pointer(), result_slot));
            self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), size_slot));
            self.emit_copy_bytes(&dst_base, &scratch9, &scratch10, "inline_value_block_copy");
            let result = self.allocate_register();
            self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
            return Ok(result);
        }
        let size = self
            .inline_collection_payload_size(type_)
            .ok_or_else(|| format!("native inline type '{type_}' has no fixed storage size"))?;
        let source_slot = self.allocate_stack_object("inline_value_source", 8);
        let result_slot = self.allocate_stack_object("inline_value_result", 8);
        let alloc_ok = self.label("inline_value_alloc_ok");
        self.emit(abi::store_u64(source, abi::stack_pointer(), source_slot));
        // plan-71-C Family-1a: alloc size is arg 0 → `%arg0`, not return_register().
        self.emit(abi::move_immediate(
            abi::c_arg(0),
            "Integer",
            &size.to_string(),
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
        self.emit(abi::move_immediate(
            &scratch13,
            "Integer",
            &size.to_string(),
        ));
        self.emit_copy_bytes(
            abi::mfb_return(1),
            &scratch9,
            &scratch13,
            "inline_value_arena_copy",
        );
        let result = self.allocate_register();
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(result)
    }

    pub(crate) fn lower_len(&mut self, value: &NirValue) -> Result<ValueResult, String> {
        let value = self.lower_value(value)?;
        if value.type_ == ParameterType::String {
            let count_slot = self.allocate_stack_object("len_string_count", 8);
            let remaining = self.allocate_register();
            let cursor = self.allocate_register();
            let byte = self.allocate_register();
            let mask = self.allocate_register();
            let loop_label = self.label("len_string_loop");
            let continuation_label = self.label("len_string_continuation");
            let next_label = self.label("len_string_next");
            let done_label = self.label("len_string_done");
            self.emit(abi::move_immediate(&byte, "Integer", "0"));
            self.emit(abi::store_u64(&byte, abi::stack_pointer(), count_slot));
            self.emit(abi::load_u64(&remaining, &value.location, 0));
            self.emit(abi::add_immediate(&cursor, &value.location, 8));
            self.emit(abi::move_immediate(&mask, "Integer", "192"));
            self.emit(abi::label(&loop_label));
            self.emit(abi::compare_immediate(&remaining, "0"));
            self.emit(abi::branch_eq(&done_label));
            self.emit(abi::load_u8(&byte, &cursor, 0));
            self.emit(abi::and_registers(&byte, &byte, &mask));
            self.emit(abi::compare_immediate(&byte, "128"));
            self.emit(abi::branch_eq(&continuation_label));
            self.emit(abi::load_u64(&byte, abi::stack_pointer(), count_slot));
            self.emit(abi::add_immediate(&byte, &byte, 1));
            self.emit(abi::store_u64(&byte, abi::stack_pointer(), count_slot));
            self.emit(abi::branch(&next_label));
            self.emit(abi::label(&continuation_label));
            self.emit(abi::label(&next_label));
            self.emit(abi::add_immediate(&cursor, &cursor, 1));
            self.emit(abi::subtract_immediate(&remaining, &remaining, 1));
            self.emit(abi::branch(&loop_label));
            self.emit(abi::label(&done_label));
            let register = self.allocate_register();
            self.emit(abi::load_u64(&register, abi::stack_pointer(), count_slot));
            Ok(ValueResult {
                origin: None,
                type_: ParameterType::Integer,
                location: Operand::from(register.render()),
                text: format!("len({})", value.text),
            })
        } else if typed_is_collection_type(&value.type_) {
            let register = self.allocate_register();
            self.emit(abi::load_u64(
                &register,
                &value.location,
                COLLECTION_OFFSET_COUNT,
            ));
            Ok(ValueResult {
                origin: None,
                type_: ParameterType::Integer,
                location: Operand::from(register.render()),
                text: format!("len({})", value.text),
            })
        } else {
            Err(format!(
                "native len does not accept argument type '{}'",
                value.type_
            ))
        }
    }

    pub(crate) fn lower_empty_collection(
        &mut self,
        type_: &ParameterType,
    ) -> Result<ValueResult, String> {
        self.lower_collection_values(type_, Vec::new(), "empty collection")
    }

    pub(crate) fn lower_list_literal(
        &mut self,
        type_: &ParameterType,
        values: &[NirValue],
    ) -> Result<ValueResult, String> {
        let mut slots = Vec::new();
        for value_node in values {
            let value = self.lower_value(value_node)?;
            // Observation boundary: a `Float` list element must be finite
            // (plan-17).
            self.observe_float(value_node, &value)?;
            // The element is stored into the collection payload through an
            // integer slot, so a `d`-native float is materialized first (plan-01
            // float-dnative).
            let value = self.materialize_value(value)?;
            let slot = self.allocate_stack_object("collection_value", 8);
            self.emit(abi::store_u64(&value.location, abi::stack_pointer(), slot));
            slots.push(CollectionValueSlot {
                key: None,
                value: PayloadSlot {
                    slot,
                    type_: value.type_.clone(),
                },
            });
        }
        self.lower_collection_values(type_, slots, "list")
    }

    /// Lower a `Set OF T { … }` literal (plan-63). A `Set` block is Map-shaped
    /// with a 1-byte `Boolean` value (always TRUE); the literal is built by
    /// inserting each element into a growing, uniquely-owned buffer so duplicates
    /// collapse. Mirrors [`lower_set_add`](Self::lower_set_add)'s idioms, but the
    /// buffer starts empty and is mutated IN PLACE (no per-element copy — the
    /// literal owns its buffer exclusively).
    pub(crate) fn lower_set_literal(
        &mut self,
        type_: &ParameterType,
        values: &[NirValue],
    ) -> Result<ValueResult, String> {
        let element_type = crate::codegen::engine::types::typed_set_element_type(type_)
            .cloned()
            .ok_or_else(|| format!("lower_set_literal: not a set type '{type_}'"))?;
        // Start from an empty set and spill its pointer to a stack slot so it can
        // be reloaded and rewritten after each (possibly reallocating) insert.
        let result = self.lower_empty_collection(type_)?;
        let set_slot = self.allocate_stack_object("set_lit", 8);
        self.emit(abi::store_u64(
            &result.location,
            abi::stack_pointer(),
            set_slot,
        ));
        // A reusable 1-byte `Boolean` TRUE — every element maps to it.
        let true_slot = self.allocate_stack_object("set_lit_true", 8);
        let true_reg = self.allocate_register();
        self.emit(abi::move_immediate(&true_reg, "Boolean", "true"));
        self.emit(abi::store_u64(&true_reg, abi::stack_pointer(), true_slot));
        for value_node in values {
            let item = self.lower_value(value_node)?;
            // Observation boundary: a `Float` element must be finite (plan-17).
            self.observe_float(value_node, &item)?;
            // A `d`-native float element is materialized into a GPR before the
            // integer-slot store (plan-01 float-dnative).
            let item = self.materialize_value(item)?;
            let item_slot = self.allocate_stack_object("set_lit_item", 8);
            self.store_value_at(&item, abi::stack_pointer(), item_slot);
            // In-place insert on the uniquely-owned literal buffer, then store the
            // (possibly reallocated) pointer back.
            let inserted = self.lower_map_set_in_place(
                set_slot,
                item_slot,
                true_slot,
                type_,
                &element_type,
                &ParameterType::Boolean,
                None,
            )?;
            self.emit(abi::store_u64(
                &inserted.location,
                abi::stack_pointer(),
                set_slot,
            ));
        }
        let register = self.allocate_register();
        self.emit(abi::load_u64(&register, abi::stack_pointer(), set_slot));
        Ok(ValueResult {
            origin: None,
            type_: type_.clone(),
            location: Operand::from(register.render()),
            text: format!("set literal {type_}"),
        })
    }

    pub(crate) fn lower_map_literal(
        &mut self,
        type_: &ParameterType,
        entries: &[(NirValue, NirValue)],
    ) -> Result<ValueResult, String> {
        let mut slots = Vec::new();
        for (key_node, value_node) in entries {
            let key = self.lower_value(key_node)?;
            // Observation boundary: a `Float` map key/value must be finite
            // (a non-finite key is rejected at insert; plan-17). Map keys still
            // *compare* bitwise — only finiteness is enforced here.
            self.observe_float(key_node, &key)?;
            // A `d`-native float key/value is materialized into a GPR before the
            // integer-slot store (plan-01 float-dnative).
            let key = self.materialize_value(key)?;
            let key_slot = self.allocate_stack_object("collection_key", 8);
            self.emit(abi::store_u64(
                &key.location,
                abi::stack_pointer(),
                key_slot,
            ));
            let value = self.lower_value(value_node)?;
            self.observe_float(value_node, &value)?;
            let value = self.materialize_value(value)?;
            let value_slot = self.allocate_stack_object("collection_value", 8);
            self.emit(abi::store_u64(
                &value.location,
                abi::stack_pointer(),
                value_slot,
            ));
            slots.push(CollectionValueSlot {
                key: Some(PayloadSlot {
                    slot: key_slot,
                    type_: key.type_.clone(),
                }),
                value: PayloadSlot {
                    slot: value_slot,
                    type_: value.type_.clone(),
                },
            });
        }
        self.lower_collection_values(type_, slots, "map")
    }

    pub(crate) fn lower_collection_values(
        &mut self,
        type_: &ParameterType,
        slots: Vec<CollectionValueSlot>,
        label: &str,
    ) -> Result<ValueResult, String> {
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        self.reset_temporary_registers();
        // A list literal whose *declared* element type is `Unknown` must not be
        // laid out from that placeholder. `expression_type` falls back to
        // `List OF Unknown` whenever it cannot type the first element — a call to
        // a builtin like `toByte` is enough (`[toByte(1), toByte(2)]`) — but the
        // allocation below sizes the entry array from the first slot's ACTUAL
        // value type. Layout, header, and the scope-drop free size all key off
        // `type_`, so leaving the placeholder in place makes them disagree with
        // what was reserved: the block is allocated entry-free and later freed as
        // `capacity * 40` bytes past its end, corrupting the arena free list —
        // bug-02's exact failure mode. Refine the type from the elements so every
        // consumer sees the one type the block was actually built to.
        let first_list_element = slots
            .first()
            .filter(|slot| slot.key.is_none())
            .map(|slot| slot.value.type_.clone());
        let first_list_element = first_list_element.as_ref();
        let refined_type = refined_list_literal_type(type_, first_list_element);
        let type_ = refined_type.as_ref().unwrap_or(type_);
        let layout = CollectionTypeLayout::from_type(type_)
            .ok_or_else(|| format!("native code collection type '{type_}' is not supported"))?;
        let count = slots.len();
        let data_len_slot = self.allocate_stack_object("collection_data_len", 8);
        self.emit(abi::move_immediate(&scratch8, "Integer", "0"));
        self.emit(abi::store_u64(
            &scratch8,
            abi::stack_pointer(),
            data_len_slot,
        ));
        for slot in &slots {
            if let Some(key) = &slot.key {
                // Map entries pack a key then a value; round each payload's start
                // offset up to its type alignment so the running data length
                // accounts for the same padding the writer inserts below.
                let key_alignment = self.collection_payload_alignment(&key.type_);
                self.emit_align_offset_slot(data_len_slot, key_alignment);
                self.emit_add_payload_length(data_len_slot, key)?;
                let value_alignment = self.collection_payload_alignment(&slot.value.type_);
                self.emit_align_offset_slot(data_len_slot, value_alignment);
            } else {
                // List payloads are homogeneous. Fixed-size elements pack with no
                // gap (their size is a whole multiple of their alignment), but a
                // *variable-length* element (a record with an inlined String
                // field, a data union, or a flat nested collection) can end on a
                // non-8 boundary and leave the next element's U64 slots unaligned,
                // so round the running length up before appending the next one
                // (bug-147.4). The writer below applies the identical rounding;
                // `list_element_padding_alignment` returns 1 for every fixed-size
                // or byte-addressed payload, keeping primitive lists byte-identical.
                let value_alignment = self.list_element_padding_alignment(&slot.value.type_);
                self.emit_align_offset_slot(data_len_slot, value_alignment);
            }
            self.emit_add_payload_length(data_len_slot, &slot.value)?;
        }

        let collection_slot = self.allocate_stack_object("collection_literal", 8);
        let alloc_ok = self.label("collection_alloc_ok");
        self.emit(abi::load_u64(
            &scratch8,
            abi::stack_pointer(),
            data_len_slot,
        ));
        // A map *or* set reserves a `2*capacity` u64 bucket array past the data
        // region; capacity == count for a literal, so fold it into the constant.
        // `collection_has_buckets` keeps a Set's reservation in lockstep with the
        // sizing/copy/free paths (plan-63-B) — omitting it would size a Set literal
        // short and corrupt the arena on the lazy bucket build.
        let bucket_bytes = if matches!(&type_, ParameterType::MapOf(..) | ParameterType::SetOf(_)) {
            count * MAP_BUCKET_SIZE * 2
        } else {
            0
        };
        // The lookup-entry stride for this literal's element type: zero for a
        // fixed-width list, which drops the entry array from the allocation
        // entirely (plan-57-D). Taken from the first slot's value type — a
        // literal's elements are all the declared element type — and zero only
        // for a keyless (list) slot, since a map keeps its entries.
        let literal_entry_stride = match slots.first() {
            Some(slot) if slot.key.is_none() => list_entry_stride(&slot.value.type_),
            _ => COLLECTION_ENTRY_SIZE,
        };
        self.emit(abi::move_immediate(
            &scratch9,
            "Integer",
            &(COLLECTION_HEADER_SIZE + count * literal_entry_stride + bucket_bytes).to_string(),
        ));
        self.emit(abi::add_registers(
            abi::return_register(),
            &scratch8,
            &scratch9,
        ));
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&alloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            collection_slot,
        ));

        self.emit_write_collection_header(&layout, count, data_len_slot);

        let data_offset_slot = self.allocate_stack_object("collection_data_offset", 8);
        self.emit(abi::move_immediate(&scratch8, "Integer", "0"));
        self.emit(abi::store_u64(
            &scratch8,
            abi::stack_pointer(),
            data_offset_slot,
        ));

        for (index, slot) in slots.iter().enumerate() {
            self.emit_write_collection_entry(collection_slot, index, slot, data_offset_slot)?;
        }
        let register = self.allocate_register();
        self.emit(abi::load_u64(
            &register,
            abi::stack_pointer(),
            collection_slot,
        ));
        Ok(ValueResult {
            origin: None,
            type_: type_.clone(),
            location: Operand::from(register.render()),
            text: format!("{label} {type_}"),
        })
    }

    pub(crate) fn emit_write_collection_header(
        &mut self,
        layout: &CollectionTypeLayout,
        count: usize,
        data_len_slot: usize,
    ) {
        let scratch8 = self.temporary_vreg();
        self.emit(abi::move_immediate(
            &scratch8,
            "Byte",
            &layout.kind.to_string(),
        ));
        self.emit(abi::store_u8(
            &scratch8,
            abi::mfb_return(1),
            COLLECTION_OFFSET_KIND,
        ));
        self.emit(abi::move_immediate(
            &scratch8,
            "Byte",
            &layout.key_type_code.to_string(),
        ));
        self.emit(abi::store_u8(
            &scratch8,
            abi::mfb_return(1),
            COLLECTION_OFFSET_KEY_TYPE,
        ));
        self.emit(abi::move_immediate(
            &scratch8,
            "Byte",
            &layout.value_type_code.to_string(),
        ));
        self.emit(abi::store_u8(
            &scratch8,
            abi::mfb_return(1),
            COLLECTION_OFFSET_VALUE_TYPE,
        ));
        self.emit(abi::move_immediate(&scratch8, "Byte", "1"));
        self.emit(abi::store_u8(
            &scratch8,
            abi::mfb_return(1),
            COLLECTION_OFFSET_FLAGS_VERSION,
        ));
        // Map hash index built lazily on first probe (no-op field for lists).
        self.emit(abi::move_immediate(&scratch8, "Byte", "0"));
        self.emit(abi::store_u8(
            &scratch8,
            abi::mfb_return(1),
            COLLECTION_OFFSET_BUCKETS_READY,
        ));
        self.emit(abi::move_immediate(
            &scratch8,
            "Integer",
            &count.to_string(),
        ));
        self.emit(abi::store_u64(
            &scratch8,
            abi::mfb_return(1),
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::store_u64(
            &scratch8,
            abi::mfb_return(1),
            COLLECTION_OFFSET_CAPACITY,
        ));
        self.emit(abi::load_u64(
            &scratch8,
            abi::stack_pointer(),
            data_len_slot,
        ));
        self.emit(abi::store_u64(
            &scratch8,
            abi::mfb_return(1),
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::store_u64(
            &scratch8,
            abi::mfb_return(1),
            COLLECTION_OFFSET_DATA_CAPACITY,
        ));
    }

    pub(crate) fn emit_write_collection_entry(
        &mut self,
        collection_slot: usize,
        index: usize,
        slot: &CollectionValueSlot,
        data_offset_slot: usize,
    ) -> Result<(), String> {
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let entry_offset = COLLECTION_HEADER_SIZE + index * COLLECTION_ENTRY_SIZE;
        // A kind-2 list has no lookup entry to write: element `i` is at
        // `dataBase + i * payloadSize` by construction (plan-57-D). The payload
        // copies below still run — they are what advances `data_offset_slot` —
        // only the entry-field stores are skipped. A map slot (`key.is_some()`)
        // always writes its entry.
        let writes_entry = slot.key.is_some() || list_entry_stride(&slot.value.type_) != 0;
        let key_len_slot = if let Some(key) = &slot.key {
            Some(self.emit_payload_length_to_stack(key, "collection_key_len")?)
        } else {
            None
        };
        let value_len_slot =
            self.emit_payload_length_to_stack(&slot.value, "collection_value_len")?;
        let collection_register = &scratch8;
        self.emit(abi::load_u64(
            collection_register,
            abi::stack_pointer(),
            collection_slot,
        ));

        self.emit(abi::move_immediate(
            &scratch9,
            "Byte",
            &COLLECTION_ENTRY_FLAG_USED.to_string(),
        ));
        if writes_entry {
            self.emit(abi::store_u8(
                &scratch9,
                collection_register,
                entry_offset + COLLECTION_ENTRY_OFFSET_FLAGS,
            ));
        }

        if let Some(key_len_slot) = key_len_slot {
            // Align the key payload start to its type alignment before recording
            // its offset (map entries only; lists have no key).
            let key_alignment =
                self.collection_payload_alignment(&slot.key.as_ref().unwrap().type_);
            self.emit_align_offset_slot(data_offset_slot, key_alignment);
            self.emit(abi::load_u64(
                collection_register,
                abi::stack_pointer(),
                collection_slot,
            ));
            self.emit(abi::load_u64(
                &scratch10,
                abi::stack_pointer(),
                data_offset_slot,
            ));
            if writes_entry {
                self.emit(abi::store_u64(
                    &scratch10,
                    collection_register,
                    entry_offset + COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
                ));
            }
            self.emit(abi::load_u64(
                &scratch11,
                abi::stack_pointer(),
                key_len_slot,
            ));
            if writes_entry {
                self.emit(abi::store_u64(
                    &scratch11,
                    collection_register,
                    entry_offset + COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
                ));
            }
            let no_stride = ParameterType::named("");
            self.emit_copy_payload_to_collection(
                collection_slot,
                key_len_slot,
                slot.key.as_ref().unwrap(),
                data_offset_slot,
                if slot.key.is_some() {
                    // The EMPTY nominal is this tree's "no declared type" marker
                    // (`type_utils::is_unset_type`); it was spelled `""` here.
                    // `list_entry_stride` answers the same for both.
                    &no_stride
                } else {
                    &slot.value.type_
                },
            )?;
        } else {
            self.emit(abi::move_immediate(&scratch10, "Integer", "0"));
            if writes_entry {
                self.emit(abi::store_u64(
                    &scratch10,
                    collection_register,
                    entry_offset + COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
                ));
            }
            if writes_entry {
                self.emit(abi::store_u64(
                    &scratch10,
                    collection_register,
                    entry_offset + COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
                ));
            }
        }

        // Align the value payload start before recording its offset. Map entries
        // round to the value's type alignment (a variable-length or 1-byte key
        // preceding an 8-byte value can leave the cursor unaligned). List entries
        // only need rounding for a *variable-length* element whose size may not be
        // a multiple of 8 (bug-147.4); `list_element_padding_alignment` returns 1
        // for fixed-size list payloads, so those stay byte-identical. This
        // mirrors the allocation-size pass exactly, so the recorded offset never
        // runs past the allocated block.
        let value_alignment = if slot.key.is_some() {
            self.collection_payload_alignment(&slot.value.type_)
        } else {
            self.list_element_padding_alignment(&slot.value.type_)
        };
        self.emit_align_offset_slot(data_offset_slot, value_alignment);
        self.emit(abi::load_u64(
            collection_register,
            abi::stack_pointer(),
            collection_slot,
        ));
        self.emit(abi::load_u64(
            &scratch10,
            abi::stack_pointer(),
            data_offset_slot,
        ));
        if writes_entry {
            self.emit(abi::store_u64(
                &scratch10,
                collection_register,
                entry_offset + COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
            ));
        }
        self.emit(abi::load_u64(
            &scratch11,
            abi::stack_pointer(),
            value_len_slot,
        ));
        if writes_entry {
            self.emit(abi::store_u64(
                &scratch11,
                collection_register,
                entry_offset + COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ));
        }
        let no_stride = ParameterType::named("");
        self.emit_copy_payload_to_collection(
            collection_slot,
            value_len_slot,
            &slot.value,
            data_offset_slot,
            if slot.key.is_some() {
                // See the sibling call above: the EMPTY nominal is the tree's
                // "no declared type" marker, spelled `""` before plan-111-E.
                &no_stride
            } else {
                &slot.value.type_
            },
        )?;
        Ok(())
    }

    pub(crate) fn emit_add_payload_length(
        &mut self,
        total_slot: usize,
        payload: &PayloadSlot,
    ) -> Result<(), String> {
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let len_slot = self.emit_payload_length_to_stack(payload, "collection_payload_len")?;
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), total_slot));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), len_slot));
        self.emit(abi::add_registers(&scratch8, &scratch8, &scratch9));
        self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), total_slot));
        Ok(())
    }

    pub(crate) fn emit_payload_length_to_stack(
        &mut self,
        payload: &PayloadSlot,
        label: &str,
    ) -> Result<usize, String> {
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let len_slot = self.allocate_stack_object(label, 8);
        match &payload.type_ {
            ParameterType::Boolean | ParameterType::Byte => {
                self.emit(abi::move_immediate(&scratch8, "Integer", "1"));
            }
            type_ if type_.is_named("Scalar") => {
                self.emit(abi::move_immediate(&scratch8, "Integer", "4"));
            }
            ParameterType::Integer
            | ParameterType::Float
            | ParameterType::Fixed
            | ParameterType::Money => {
                self.emit(abi::move_immediate(&scratch8, "Integer", "8"));
            }
            // A function value is a single 8-byte closure pointer, stored by
            // reference exactly like a pointer payload (bug-73).
            other if matches!(other, ParameterType::Func(..)) => {
                self.emit(abi::move_immediate(&scratch8, "Integer", "8"));
            }
            ParameterType::String => {
                self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), payload.slot));
                self.emit(abi::load_u64(&scratch8, &scratch8, 0));
            }
            other if self.is_pointer_collection_payload_type(other) => {
                self.emit(abi::move_immediate(&scratch8, "Integer", "8"));
            }
            other if self.record_has_inline_data(other) => {
                // A record with inlined String fields is variable-length; size
                // its full flat block at runtime (plan-02 §4.2).
                self.emit_record_block_size_to_slot(other, payload.slot, len_slot)?;
                return Ok(len_slot);
            }
            other if self.union_is_data(other) => {
                // A data union is variable-length; read its `size` word at +8
                // (plan-02 §4.3).
                self.emit_data_union_size_to_slot(payload.slot, len_slot);
                return Ok(len_slot);
            }
            other if typed_is_collection_type(other) => {
                // A flat nested collection is inlined as its own block; size it at
                // runtime (plan-02 §4.4).
                self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), payload.slot));
                self.emit_flat_block_size(other, &scratch8, &scratch9, &scratch10)?;
                self.emit(abi::store_u64(&scratch9, abi::stack_pointer(), len_slot));
                return Ok(len_slot);
            }
            other if self.inline_collection_payload_size(other).is_some() => {
                let size = self
                    .inline_collection_payload_size(other)
                    .expect("guard ensures inline payload size exists");
                self.emit(abi::move_immediate(&scratch8, "Integer", &size.to_string()));
            }
            other => {
                return Err(format!(
                    "native collection packed payload does not support type '{other}'"
                ));
            }
        }
        self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), len_slot));
        Ok(len_slot)
    }

    /// Copy a payload into a collection's data region at `data_offset_slot`.
    ///
    /// `stride_type` selects the data base's entry stride, exactly as for the
    /// readers: the element type for a LIST, `""` for a MAP. Deriving it from
    /// `payload.type_` is wrong for a map — a `Map OF Scalar TO T` has a
    /// fixed-width KEY, and the entry-free base would write that key inside the
    /// map's own lookup table (plan-57-D).
    pub(crate) fn emit_copy_payload_to_collection(
        &mut self,
        collection_slot: usize,
        len_slot: usize,
        payload: &PayloadSlot,
        data_offset_slot: usize,
        stride_type: &ParameterType,
    ) -> Result<(), String> {
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        self.emit(abi::load_u64(
            &scratch8,
            abi::stack_pointer(),
            collection_slot,
        ));
        self.emit(abi::load_u64(
            &scratch9,
            abi::stack_pointer(),
            data_offset_slot,
        ));
        self.emit(abi::add_immediate(
            &scratch10,
            &scratch8,
            COLLECTION_HEADER_SIZE,
        ));
        // The data base uses this element type's entry stride, which is zero for
        // a kind-2 list — the payload write and every reader must agree on where
        // the data region starts (plan-57-D).
        if list_entry_stride(stride_type) != 0 {
            self.emit(abi::load_u64(
                &scratch11,
                &scratch8,
                COLLECTION_OFFSET_CAPACITY,
            ));
            self.emit(abi::move_immediate(
                &scratch12,
                "Integer",
                &COLLECTION_ENTRY_SIZE.to_string(),
            ));
            self.emit(abi::multiply_registers(&scratch11, &scratch11, &scratch12));
            self.emit(abi::add_registers(&scratch10, &scratch10, &scratch11));
        }
        self.emit(abi::add_registers(&scratch10, &scratch10, &scratch9));

        match &payload.type_ {
            ParameterType::Boolean | ParameterType::Byte => {
                self.emit(abi::load_u64(
                    &scratch12,
                    abi::stack_pointer(),
                    payload.slot,
                ));
                self.emit(abi::store_u8(&scratch12, &scratch10, 0));
            }
            type_ if type_.is_named("Scalar") => {
                self.emit(abi::load_u64(
                    &scratch12,
                    abi::stack_pointer(),
                    payload.slot,
                ));
                self.emit(abi::store_u32(&scratch12, &scratch10, 0));
            }
            ParameterType::Integer
            | ParameterType::Float
            | ParameterType::Fixed
            | ParameterType::Money => {
                self.emit(abi::load_u64(
                    &scratch12,
                    abi::stack_pointer(),
                    payload.slot,
                ));
                self.emit(abi::store_u64(&scratch12, &scratch10, 0));
            }
            // A function value stores its 8-byte closure pointer verbatim; the
            // closure object it points at is arena-lifetime and shared, never
            // copied on insert (reference semantics, bug-73).
            other if matches!(other, ParameterType::Func(..)) => {
                self.emit(abi::load_u64(
                    &scratch12,
                    abi::stack_pointer(),
                    payload.slot,
                ));
                self.emit(abi::store_u64(&scratch12, &scratch10, 0));
            }
            ParameterType::String => {
                let loop_label = self.label("collection_copy_string_loop");
                let done_label = self.label("collection_copy_string_done");
                self.emit(abi::load_u64(
                    &scratch12,
                    abi::stack_pointer(),
                    payload.slot,
                ));
                self.emit(abi::add_immediate(&scratch12, &scratch12, 8));
                self.emit(abi::load_u64(&scratch13, abi::stack_pointer(), len_slot));
                self.emit(abi::label(&loop_label));
                self.emit(abi::compare_immediate(&scratch13, "0"));
                self.emit(abi::branch_eq(&done_label));
                self.emit(abi::load_u8(&scratch14, &scratch12, 0));
                self.emit(abi::store_u8(&scratch14, &scratch10, 0));
                self.emit(abi::add_immediate(&scratch12, &scratch12, 1));
                self.emit(abi::add_immediate(&scratch10, &scratch10, 1));
                self.emit(abi::subtract_immediate(&scratch13, &scratch13, 1));
                self.emit(abi::branch(&loop_label));
                self.emit(abi::label(&done_label));
            }
            other if self.is_pointer_collection_payload_type(other) => {
                self.emit(abi::load_u64(
                    &scratch12,
                    abi::stack_pointer(),
                    payload.slot,
                ));
                self.emit(abi::store_u64(&scratch12, &scratch10, 0));
            }
            other
                if self.inline_collection_payload_size(other).is_some()
                    || typed_is_collection_type(other) =>
            {
                // Inline record/union slot bytes, or a flat nested collection
                // block — copy `len_slot` bytes verbatim (plan-02 §4.2–§4.4).
                self.emit(abi::load_u64(
                    &scratch12,
                    abi::stack_pointer(),
                    payload.slot,
                ));
                self.emit(abi::load_u64(&scratch13, abi::stack_pointer(), len_slot));
                self.emit_copy_bytes(&scratch10, &scratch12, &scratch13, "collection_copy_inline");
            }
            other => {
                return Err(format!(
                    "native collection packed payload does not support type '{other}'"
                ));
            }
        }

        self.emit(abi::load_u64(
            &scratch8,
            abi::stack_pointer(),
            data_offset_slot,
        ));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), len_slot));
        self.emit(abi::add_registers(&scratch8, &scratch8, &scratch9));
        self.emit(abi::store_u64(
            &scratch8,
            abi::stack_pointer(),
            data_offset_slot,
        ));
        Ok(())
    }

    /// Add the map hash-index bucket-region byte size (`2*capacity` u64 buckets =
    /// `capacity * MAP_BUCKET_SIZE * 2` bytes) held in `capacity_reg` onto the
    /// running allocation size in `size_reg`, using `scratch_reg` (plan-02
    /// Phase 6). A no-op for lists (`is_map == false`) so list allocations are
    /// unchanged. The bucket region sits past the data region, so the
    /// capacity-based data base (`emit_collection_data_pointer`) is unaffected.
    pub(crate) fn emit_reserve_map_buckets(
        &mut self,
        is_map: bool,
        capacity_reg: impl Into<Operand>,
        size_reg: impl Into<Operand>,
        scratch_reg: impl Into<Operand>,
    ) {
        if !is_map {
            return;
        }
        let size_reg = size_reg.into();
        let scratch_reg = scratch_reg.into();
        // 2 * capacity buckets * 8 bytes = capacity << 4.
        self.emit(abi::shift_left_immediate(
            scratch_reg.clone(),
            capacity_reg,
            4,
        ));
        self.emit(abi::add_registers(size_reg.clone(), size_reg, scratch_reg));
    }

    /// The payload offset and length of list element `index`, into `dst_offset`
    /// and `dst_length`.
    ///
    /// This is the single authority for "where does element `i` live". Every
    /// indexed list read goes through it;
    /// `builder_collection_compare.rs`'s offset-parameterized helpers sit one
    /// level below it and are unaffected.
    ///
    /// `scratch_offset` and `scratch_entry` are supplied by the caller rather
    /// than allocated here, so that consolidating a site cannot perturb its
    /// register numbering — every one of these call sites allocates its whole
    /// register set up front, so an allocation made *inside* the helper would
    /// land in a different order and change the emitted bytes. Byte-identity is
    /// plan-57-A's only guard, so it outranks the tidier signature.
    ///
    /// `element_type` is unused today: the lookup entry answers for every element
    /// type alike. It is threaded through because plan-57-D branches on it to
    /// give fixed-width-scalar lists an entry-free representation, where the
    /// answer becomes `index * payloadSize` with no loads at all. Adding the
    /// parameter later would mean touching all of these call sites twice.
    pub(crate) fn emit_element_value_offset(
        &mut self,
        dst_offset: impl Into<Operand>,
        dst_length: impl Into<Operand>,
        list: impl Into<Operand>,
        index: impl Into<Operand>,
        scratch_offset: impl Into<Operand>,
        scratch_entry: impl Into<Operand>,
        element_type: &ParameterType,
    ) {
        let dst_offset = dst_offset.into();
        let dst_length = dst_length.into();
        let list = list.into();
        let index = index.into();
        let scratch_offset = scratch_offset.into();
        let scratch_entry = scratch_entry.into();
        // kind 2: element `i` lives at `i * payloadSize` with a fixed length and
        // no entry to load (plan-57-D). Two instructions instead of six, and two
        // dependent loads removed from every indexed read.
        if let Some(payload) = kind2_payload_size(element_type) {
            self.emit(abi::move_immediate(
                dst_length.clone(),
                "Integer",
                &payload.to_string(),
            ));
            self.emit(abi::multiply_registers(dst_offset, index, dst_length));
            return;
        }
        self.emit(abi::move_immediate(
            scratch_offset.clone(),
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(
            scratch_offset.clone(),
            index,
            scratch_offset.clone(),
        ));
        self.emit(abi::add_immediate(
            scratch_entry.clone(),
            list,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(
            scratch_entry.clone(),
            scratch_entry.clone(),
            scratch_offset,
        ));
        self.emit(abi::load_u64(
            dst_offset,
            scratch_entry.clone(),
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            dst_length,
            scratch_entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
    }

    /// The address of a collection's packed data region.
    ///
    /// Delegates to [`push_collection_data_pointer_into`] so the layout rule
    /// lives in exactly one place; plan-57-D changes that one function to make a
    /// fixed-width list's data base a constant `block + HEADER`.
    /// The address of a collection's packed data region.
    ///
    /// `element_type` selects the lookup-entry stride
    /// ([`list_entry_stride`]), which is zero for a fixed-width list under
    /// plan-57-D — collapsing this to `collection + HEADER`. Pass the ELEMENT
    /// type of a list; for a map, or where the element type is not statically
    /// known, pass `""`, which always yields the kind-0 stride.
    pub(crate) fn emit_collection_data_pointer_for(
        &mut self,
        dst: impl Into<Operand>,
        collection: impl Into<Operand>,
        element_type: &ParameterType,
    ) {
        let stride = list_entry_stride(element_type);
        if stride == 0 {
            self.emit(abi::add_immediate(dst, collection, COLLECTION_HEADER_SIZE));
            return;
        }
        self.emit_collection_data_pointer(dst, collection);
    }

    /// The kind-0 data base. **Private on purpose**: every caller must go
    /// through [`Self::emit_collection_data_pointer_for`] and state its element
    /// type, because a site that silently kept the kind-0 stride would read a
    /// fixed-width list at the wrong base once plan-57-D flips the
    /// representation — and the gate cannot catch that, since both are correct
    /// today. Making the untyped form unreachable is what turns "did I convert
    /// every site?" from a question into a compile error.
    fn emit_collection_data_pointer(
        &mut self,
        dst: impl Into<Operand>,
        collection: impl Into<Operand>,
    ) {
        // Scratch as vregs. Pinning these collides with the x86-64 ABI argument
        // registers and yields garbage element addresses.
        let capacity_v = self.temporary_vreg();
        let entry_size_v = self.temporary_vreg();
        let mut out = Vec::new();
        push_collection_data_pointer_into(&mut out, dst, collection, &capacity_v, &entry_size_v);
        for instruction in out {
            self.emit(instruction);
        }
    }

    /// Load a list element's payload. The data base uses `type_`'s entry
    /// stride, which is correct only because this is a LIST block; a map must
    /// call [`Self::emit_load_map_payload`].
    pub(crate) fn emit_load_collection_payload(
        &mut self,
        type_: &ParameterType,
        collection: impl Into<Operand>,
        offset: impl Into<Operand>,
        length: impl Into<Operand>,
    ) -> Result<VirtualRegister, String> {
        self.emit_load_payload_with_stride(type_, type_, collection, offset, length)
    }

    /// Load a payload out of a MAP block. Identical to
    /// [`Self::emit_load_collection_payload`] except that the data base always
    /// uses the kind-0 stride: a map keeps its lookup table whatever its key and
    /// value types are, so selecting the entry-free base from a fixed-width key
    /// or value type would address it past its own entry array (plan-57-D).
    pub(crate) fn emit_load_map_payload(
        &mut self,
        type_: &ParameterType,
        collection: impl Into<Operand>,
        offset: impl Into<Operand>,
        length: impl Into<Operand>,
    ) -> Result<VirtualRegister, String> {
        self.emit_load_payload_with_stride(
            type_,
            &ParameterType::named(""),
            collection,
            offset,
            length,
        )
    }

    fn emit_load_payload_with_stride(
        &mut self,
        type_: &ParameterType,
        stride_type: &ParameterType,
        collection: impl Into<Operand>,
        offset: impl Into<Operand>,
        length: impl Into<Operand>,
    ) -> Result<VirtualRegister, String> {
        // Inputs held in vregs, never in registers that are x86-64 ABI argument
        // registers on one backend and free scratch on another.
        let collection_input_v = self.temporary_vreg();
        let offset_input_v = self.temporary_vreg();
        let length_input_v = self.temporary_vreg();
        let collection_input = &collection_input_v;
        let offset_input = &offset_input_v;
        let length_input = &length_input_v;
        self.emit(abi::move_register(collection_input, collection));
        self.emit(abi::move_register(offset_input, offset));
        self.emit(abi::move_register(length_input, length));
        let data = self.allocate_register();
        self.emit_collection_data_pointer_for(&data, collection_input, stride_type);
        self.emit(abi::add_registers(&data, &data, offset_input));
        match type_ {
            ParameterType::Boolean | ParameterType::Byte => {
                let result = self.allocate_register();
                self.emit(abi::load_u8(&result, &data, 0));
                Ok(result)
            }
            type_ if type_.is_named("Scalar") => {
                let result = self.allocate_register();
                self.emit(abi::load_u32(&result, &data, 0));
                Ok(result)
            }
            ParameterType::Integer
            | ParameterType::Float
            | ParameterType::Fixed
            | ParameterType::Money => {
                let result = self.allocate_register();
                self.emit(abi::load_u64(&result, &data, 0));
                Ok(result)
            }
            // A function value reads back its 8-byte closure pointer; the closure
            // object stays shared (reference semantics, bug-73).
            other if matches!(other, ParameterType::Func(..)) => {
                let result = self.allocate_register();
                self.emit(abi::load_u64(&result, &data, 0));
                Ok(result)
            }
            ParameterType::String => self.emit_materialize_string_from_bytes(&data, length_input),
            other if self.is_pointer_collection_payload_type(other) => {
                let result = self.allocate_register();
                self.emit(abi::load_u64(&result, &data, 0));
                Ok(result)
            }
            // An inlined record/union slot block or a flat nested collection block
            // is read as an alias pointer to the block within the data region
            // (plan-02 §4.2–§4.4). Its own offsets are relative to that base.
            other if self.inline_collection_payload_size(other).is_some() => Ok(data),
            other if typed_is_collection_type(other) => Ok(data),
            other => Err(format!(
                "native collection packed payload does not support type '{other}'"
            )),
        }
    }

    /// Copy an existing heap `String` value (a pointer to `[u64 len][bytes][nul]`)
    /// into a fresh owned arena string. `getOr`'s found path materializes its
    /// `String` result fresh (`emit_load_collection_payload`), so `getOr`'s
    /// default path must copy the aliased default the same way — otherwise the
    /// owned-result contract (`materialize_owned_element` frees the result at
    /// scope end, but deliberately skips `String` assuming it is already fresh)
    /// double-frees the caller's default and corrupts the arena free-list, which
    /// only surfaces as a trap on a *later* allocation. See [[scope-drop-frees]].
    pub(crate) fn emit_copy_owned_string(
        &mut self,
        source_ptr: impl Into<Operand>,
    ) -> Result<VirtualRegister, String> {
        let source_ptr = source_ptr.into();
        let length = self.allocate_register();
        self.emit(abi::load_u64(&length, source_ptr.clone(), 0));
        let bytes = self.allocate_register();
        self.emit(abi::add_immediate(&bytes, source_ptr, 8));
        self.emit_materialize_string_from_bytes(&bytes, &length)
    }

    pub(crate) fn emit_materialize_string_from_bytes(
        &mut self,
        source: impl Into<Operand>,
        length: impl Into<Operand>,
    ) -> Result<VirtualRegister, String> {
        let length = length.into();
        let scratch12 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        let scratch15 = self.temporary_vreg();
        let source_slot = self.allocate_stack_object("collection_string_source", 8);
        let length_slot = self.allocate_stack_object("collection_string_length", 8);
        let result_slot = self.allocate_stack_object("collection_string_result", 8);
        let alloc_ok = self.label("collection_string_alloc_ok");

        self.emit(abi::store_u64(source, abi::stack_pointer(), source_slot));
        self.emit(abi::store_u64(
            length.clone(),
            abi::stack_pointer(),
            length_slot,
        ));
        self.emit(abi::add_immediate(abi::return_register(), length, 9));
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
        self.emit(abi::load_u64(&scratch12, abi::stack_pointer(), length_slot));
        self.emit(abi::store_u64(&scratch12, abi::mfb_return(1), 0));
        self.emit(abi::add_immediate(&scratch13, abi::mfb_return(1), 8));
        self.emit(abi::load_u64(&scratch14, abi::stack_pointer(), source_slot));
        // plan-86 F2: 8-byte word-copy (+ byte tail) instead of a byte-at-a-time
        // loop — byte-exact, advances scratch13/scratch14 past the copied bytes so
        // the NUL terminator lands at result+8+length.
        self.emit_block_copy_advance(
            &scratch13,
            &scratch14,
            &scratch12,
            &scratch15,
            "collection_string_copy",
        );
        self.emit(abi::move_immediate(&scratch15, "Integer", "0"));
        self.emit(abi::store_u8(&scratch15, &scratch13, 0));
        let result = self.allocate_register();
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(result)
    }
}

/// The payload size, in bytes, of a list element type whose payloads are
/// fixed-width — and which therefore may be addressed as `dataBase + i * size`.
///
/// A `Some` here is a promise that bug-365's ordering invariant holds: for such
/// a list `entry[i].valueOffset == i * size` after **every** operation, so
/// walking the data region linearly visits elements in index order.
/// `lower_list_insert_collection`, `lower_list_prepend_in_place` and
/// `lower_collection_set` are what maintain it; every other list operation
/// either preserves order or rebuilds in order. The per-operation table lives in
/// `src/docs/spec/memory/05_collections.md`, *Payload Order*.
///
/// `None` covers the variable-width types — `String`, records, unions, nested
/// collections — which keep the offset-stable scheme (plan-01 §4.1) and must be
/// read through `entry[i].valueOffset`. A linear stride is not even expressible
/// for them, which is why no reader got those wrong.
///
/// Deliberately excludes function values and pointer payloads. Both are 8-byte
/// fixed and would qualify on every ownership question plan-57-E asked — the
/// block is freed wholesale, transfer never walks entries for them, and a
/// function value is a bare pointer with reference semantics. Widening was
/// nonetheless declined, on coverage rather than analysis: `List OF FUNC` is
/// exercised by one fixture through one operation (`collections::get`), so the
/// suite passing proves nothing about the other twenty. See plan-57-E §4.2 for
/// the criterion that would reopen it.
///
/// Must agree with `CodeBuilder::collection_payload_alignment` for every arm;
/// `fixed_width_agrees_with_payload_alignment` asserts that so the two cannot
/// drift.
pub(crate) fn list_element_is_fixed_width(element_type: &ParameterType) -> Option<usize> {
    match element_type {
        ParameterType::Boolean | ParameterType::Byte => Some(1),
        // `Scalar` is a bare nominal, not a variant (it is a Unicode scalar
        // value, `Named("Scalar")`), so it is matched by name rather than shape.
        type_ if type_.is_named("Scalar") => Some(4),
        ParameterType::Integer
        | ParameterType::Float
        | ParameterType::Fixed
        | ParameterType::Money => Some(8),
        _ => None,
    }
}

/// Append the data-region base computation for `collection` into `out`:
/// `collection + COLLECTION_HEADER_SIZE + capacity * COLLECTION_ENTRY_SIZE`.
///
/// **Capacity, never count.** An append-built list has spare capacity, and the
/// `LookupEntry[capacity]` array precedes the data region, so a count-based base
/// reads the spare entry slots as payload bytes. That trap is documented at
/// several call sites individually; this is the one place it has to be right.
///
/// The free-function form exists because ~14 sites compute this inside
/// standalone `CodeFunction` emitters that have no `CodeBuilder`
/// (`os.rs`, `fs_helpers_*`, `net/io.rs`, `tls/*`, `audio/*`, `crypto*`). That
/// structural split is why a single helper never absorbed them.
/// [`CodeBuilder::emit_collection_data_pointer`] delegates here.
///
/// Every register is a parameter so a site can keep its own register choice and
/// stay byte-identical when it is converted.
pub(crate) fn push_collection_data_pointer_into(
    out: &mut Vec<CodeInstruction>,
    dst: impl Into<Operand>,
    collection: impl Into<Operand>,
    scratch_capacity: impl Into<Operand>,
    scratch_entry_size: impl Into<Operand>,
) {
    let dst = dst.into();
    let collection = collection.into();
    let scratch_capacity = scratch_capacity.into();
    let scratch_entry_size = scratch_entry_size.into();
    out.push(abi::move_register(
        scratch_capacity.clone(),
        collection.clone(),
    ));
    out.push(abi::add_immediate(
        dst.clone(),
        collection,
        COLLECTION_HEADER_SIZE,
    ));
    out.push(abi::load_u64(
        scratch_capacity.clone(),
        scratch_capacity.clone(),
        COLLECTION_OFFSET_CAPACITY,
    ));
    out.push(abi::move_immediate(
        scratch_entry_size.clone(),
        "Integer",
        &COLLECTION_ENTRY_SIZE.to_string(),
    ));
    out.push(abi::multiply_registers(
        scratch_capacity.clone(),
        scratch_capacity.clone(),
        scratch_entry_size,
    ));
    out.push(abi::add_registers(dst.clone(), dst, scratch_capacity));
}

/// Append the data-region base computation in the form the standalone runtime
/// emitters use: `dst = collection + HEADER + capacity * ENTRY_SIZE`, computed
/// as a product first and the header folded in afterwards.
///
/// A second shape rather than a second copy of the rule.
/// [`push_collection_data_pointer_into`] computes the same address in the order
/// `CodeBuilder` emits it; these two orders are what exist in the tree, and
/// forcing one site into the other's order would change its emitted bytes for
/// no benefit. Both are edited together by plan-57-D, which is the point —
/// fourteen open-coded copies become two.
///
/// `scratch_product` may alias `scratch_entry_size` and/or `dst`; the sites in
/// `audio/` do exactly that.
pub(crate) fn push_collection_data_base_from_capacity(
    out: &mut Vec<CodeInstruction>,
    dst: impl Into<Operand>,
    collection: impl Into<Operand>,
    scratch_capacity: impl Into<Operand>,
    scratch_entry_size: impl Into<Operand>,
    scratch_product: impl Into<Operand>,
) {
    let dst = dst.into();
    let collection = collection.into();
    let scratch_capacity = scratch_capacity.into();
    let scratch_entry_size = scratch_entry_size.into();
    let scratch_product = scratch_product.into();
    out.push(abi::load_u64(
        scratch_capacity.clone(),
        collection.clone(),
        COLLECTION_OFFSET_CAPACITY,
    ));
    // Every caller of this helper addresses a `List OF Byte` (net write/sendTo,
    // both TLS backends, both audio backends), so the stride is the byte-list
    // stride: zero once kind-2 drops the entry array, which collapses the data
    // base to `collection + HEADER` (plan-57-D). The multiply is left in place
    // rather than special-cased — `capacity * 0` is already the right answer, and
    // keeping one shape means the flag-off encoding stays byte-identical.
    out.push(abi::move_immediate(
        scratch_entry_size.clone(),
        "Integer",
        &byte_list_entry_stride().to_string(),
    ));
    out.push(abi::multiply_registers(
        scratch_product.clone(),
        scratch_capacity,
        scratch_entry_size,
    ));
    out.push(abi::add_immediate(
        scratch_product.clone(),
        scratch_product.clone(),
        COLLECTION_HEADER_SIZE,
    ));
    out.push(abi::add_registers(dst, collection, scratch_product));
}

/// The lookup-entry stride for a list of `element_type`, in bytes.
///
/// `COLLECTION_ENTRY_SIZE` for a kind-0 block, and **zero** for a kind-2 one —
/// a fixed-width list has no `LookupEntry` array at all (plan-57-D).
///
/// Zero is the load-bearing choice. Every layout formula in the tree is written
/// as `HEADER + capacity * <stride>` (+ `dataCapacity` for the block size), so a
/// stride of zero collapses each of them to the kind-2 layout without the
/// formula changing shape:
///
/// | | kind 0 | kind 2 |
/// |---|---|---|
/// | data base | `block + 40 + cap*40` | `block + 40` |
/// | block size | `40 + cap*40 + dataCap` | `40 + dataCap` |
///
/// So the allocation size, the free size and the data base cannot disagree about
/// the representation — they all read the same stride. That mattered enough to
/// design around: `emit_flat_block_size` computing a size the allocator did not
/// allocate is bug-02, and it corrupts the arena free list rather than producing
/// a wrong value.
pub(crate) fn list_entry_stride(element_type: &ParameterType) -> usize {
    if list_element_is_fixed_width(element_type).is_some() {
        0
    } else {
        COLLECTION_ENTRY_SIZE
    }
}

/// The `kind` byte for a list of `element_type`.
pub(crate) fn list_block_kind(element_type: &ParameterType) -> usize {
    if list_element_is_fixed_width(element_type).is_some() {
        COLLECTION_KIND_LIST_FIXED
    } else {
        COLLECTION_KIND_LIST
    }
}

/// The payload size of `element_type` when it uses the entry-free
/// representation, or `None` when it keeps a lookup table.
///
/// Now identical to [`list_element_is_fixed_width`], which it delegates to. The
/// two names are kept apart because they answer different questions and only
/// coincide today: this one is about the **representation** — "may element `i` be
/// addressed at `i * payloadSize`, with no entry to indirect through" — while
/// `list_element_is_fixed_width` is about the **type**. A `Map OF Scalar TO T`
/// has a fixed-width key and still keeps its entries, so the caller that wants a
/// stride must ask this question, not that one. Selecting a stride from the
/// payload type rather than the block kind was one of the two mistakes that
/// produced plan-57-D's corruption bugs.
pub(crate) fn kind2_payload_size(element_type: &ParameterType) -> Option<usize> {
    list_element_is_fixed_width(element_type)
}

/// The lookup-entry stride for a `List OF Byte`, for the runtime helpers that
/// build or read one and know their element type statically. Zero once the
/// entry-free representation is live (plan-57-D).
pub(crate) fn byte_list_entry_stride() -> usize {
    list_entry_stride(&ParameterType::Byte)
}

/// The type a list literal should actually be laid out as, or `None` to keep the
/// declared one.
///
/// `expression_type` falls back to `List OF Unknown` whenever it cannot type the
/// first element — a call to a builtin like `toByte` is enough. That placeholder
/// must not reach a layout decision: `lower_collection_values` sizes the entry
/// array from the first slot's ACTUAL value type, while the layout, the header,
/// and the scope-drop free size key off the declared type. Left unrefined the two
/// disagree, and the block is allocated entry-free then freed as `capacity * 40`
/// bytes past its end — bug-02's failure mode, reached from `[toByte(1)]`.
///
/// Only list literals with at least one element can be refined; an empty `[]` has
/// nothing to learn from, and a map keeps its entries regardless.
fn refined_list_literal_type(
    declared: &ParameterType,
    first_element_type: Option<&ParameterType>,
) -> Option<ParameterType> {
    // plan-111-C Phase 4 (plan-106-E Correction 4's site): built structurally.
    // This was `format!("List OF {element}")` over two rendered spellings — the
    // last production type construction outside `ParameterType::name` and the
    // wire codec — and the caller then parsed the result straight back.
    let element = first_element_type?;
    match declared {
        ParameterType::ListOf(declared_element)
            if matches!(**declared_element, ParameterType::Unknown) =>
        {
            Some(ParameterType::list_of(element.clone()))
        }
        _ => None,
    }
}

/// Allocate a `List OF Byte` of `count_off` elements: size the block, write the
/// header, and fill the lookup table with the identity mapping
/// (`valueOffset = i`, `valueLength = 1`). The payload bytes are left
/// uninitialized for the caller to fill.
///
/// Lives here rather than in `audio/` (plan-58-B): `link_thunk`'s `OUT CBuffer`
/// staging needs the same block, and a LINK thunk reaching into the audio module
/// for it would be a dependency that means nothing. The body is unchanged by the
/// move, so every audio thunk stays byte-identical.
///
/// One copy, shared by both audio backends (plan-57-B). It existed verbatim in
/// `alsa.rs` and `macos.rs` — the two differed only in label names and
/// comments. A third near-variant that also copies from a source buffer lives at
/// `crypto_ec::emit_build_byte_list`.
///
/// Sharing it is what makes plan-57-D a small edit rather than a sweep: this is
/// one of the places that must stop writing a lookup table once a fixed-width
/// list no longer has one.
///
/// A free function rather than a `CodeBuilder` method because both callers are
/// standalone `CodeFunction` emitters with no builder in scope (plan-57-A §Open
/// Decisions).
pub(crate) fn emit_alloc_byte_list(
    symbol: &str,
    tag: &str,
    count_off: usize,
    list_off: usize,
    alloc_fail: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let entry_loop = format!("{symbol}_{tag}_bl_entry");
    let entry_done = format!("{symbol}_{tag}_bl_entry_done");
    instructions.extend([
        abi::load_u64("%v10", abi::stack_pointer(), count_off),
        abi::move_immediate("%v11", "Integer", &byte_list_entry_stride().to_string()),
        abi::multiply_registers("%v12", "%v10", "%v11"),
        abi::add_immediate("%v12", "%v12", COLLECTION_HEADER_SIZE),
        abi::add_registers(abi::return_register(), "%v12", "%v10"), // + count payload bytes
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.extend([
        abi::move_register("%v15", abi::mfb_return(1)),
        abi::store_u64("%v15", abi::stack_pointer(), list_off),
        abi::move_immediate("%v9", "Byte", &byte_list_block_kind().to_string()),
        abi::store_u8("%v9", "%v15", COLLECTION_OFFSET_KIND),
        abi::move_immediate("%v9", "Byte", &COLLECTION_TYPE_NONE.to_string()),
        abi::store_u8("%v9", "%v15", COLLECTION_OFFSET_KEY_TYPE),
        abi::move_immediate("%v9", "Byte", &COLLECTION_TYPE_BYTE.to_string()),
        abi::store_u8("%v9", "%v15", COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate("%v9", "Byte", "1"),
        abi::store_u8("%v9", "%v15", COLLECTION_OFFSET_FLAGS_VERSION),
        abi::load_u64("%v10", abi::stack_pointer(), count_off),
        abi::store_u64("%v10", "%v15", COLLECTION_OFFSET_COUNT),
        abi::store_u64("%v10", "%v15", COLLECTION_OFFSET_CAPACITY),
        abi::store_u64("%v10", "%v15", COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64("%v10", "%v15", COLLECTION_OFFSET_DATA_CAPACITY),
        // entry array: entry[i] = { USED, value_offset=i, value_length=1 }
    ]);
    // kind 2 has no entry array to fill (plan-57-D), so the ENTIRE loop is
    // skipped — not just its body.
    //
    // plan-57-D guarded only the body, which left `label; cmp i,count;
    // bge done; i++; b loop` behind: a no-op loop that still ran `count` times at
    // RUNTIME. Every audio capture allocation paid it, and it scales with the
    // buffer — a 3-minute stereo 48 kHz read burned ~34 million iterations doing
    // nothing. Correct output, silently linear waste, which is why nothing caught
    // it. The header stores above already set count/capacity/dataLength/
    // dataCapacity, so under kind 2 there is nothing left for the loop to do.
    if byte_list_entry_stride() != 0 {
        instructions.extend([
            abi::add_immediate("%v11", "%v15", COLLECTION_HEADER_SIZE), // entry cursor
            abi::move_immediate("%v13", "Integer", "0"),                // i
            abi::label(&entry_loop),
            abi::compare_registers("%v13", "%v10"),
            abi::branch_ge(&entry_done),
            abi::move_immediate("%v14", "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
            abi::store_u8("%v14", "%v11", COLLECTION_ENTRY_OFFSET_FLAGS),
            abi::store_u64(abi::ZERO, "%v11", COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
            abi::store_u64(abi::ZERO, "%v11", COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
            abi::store_u64("%v13", "%v11", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
            abi::move_immediate("%v14", "Integer", "1"),
            abi::store_u64("%v14", "%v11", COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
            abi::add_immediate("%v11", "%v11", byte_list_entry_stride()),
            abi::add_immediate("%v13", "%v13", 1),
            abi::branch(&entry_loop),
            abi::label(&entry_done),
        ]);
    }
}

/// The `kind` byte for a `List OF Byte`, for those same runtime helpers.
///
/// Nothing reads this byte at runtime — every consumer dispatches statically on
/// the source type — so stamping the wrong kind is invisible today. It is
/// written truthfully anyway: a block whose header claims a lookup table it does
/// not have is precisely the sort of latent disagreement that produced four
/// separate corruption bugs in this sub-plan, and the next person to reach for
/// the kind byte should be able to trust it.
pub(crate) fn byte_list_block_kind() -> usize {
    list_block_kind(&ParameterType::Byte)
}

// ---------------------------------------------------------------------------
// Record field-layout classification (spec/memory/03_heap-values.md §Record).
//
// These are the single source of truth for how a record slot stores each field —
// inline scalar, block-relative offset into the data region (inlined String /
// flat composite), or an 8-byte pointer to a separate allocation. The
// `CodeBuilder` methods above delegate here so the call-site record builder
// (`emit_build_inlined_record`) and the **helper-tier** record marshaller
// (`crate::codegen::memory::marshal::emit_build_inlined_record`, driven from a
// `Body::abi_function` runtime-helper emitter) classify fields identically
// and can never drift.
// ---------------------------------------------------------------------------

/// The built-in helper-constructed records whose `String`/sub-record fields are
/// kept as **pointers** to separate allocations rather than inlined into the data
/// region (spec §Record "excluded"). The socket helpers build `net::Address` and
/// `udp::Datagram` that way, and audio builds `audio::AudioDevice`.
///
/// The membership question goes through `is_builtin_named`, which accepts both
/// the bare leaf and the package-qualified id, and **both are load-bearing**
/// (bug-483). A *signature* type — a member's parameter or return, rewritten by
/// `Registry::qualify_value_type_references` — arrives as `net.Address`; a record
/// *field* type does not, because that pass deliberately leaves field types bare
/// so the injected companion source stays parseable, so `udp::Datagram`'s `from`
/// field arrives as `Address`. Matching only the bare leaf silently reclassified
/// every qualified reference as an ordinary inlined-`String` record, and its
/// readers then took the slot the socket helper had written an absolute pointer
/// into as a block-relative offset — a wild pointer, and a `SIGSEGV` the moment
/// anything touched `.host`.
pub(crate) fn is_pointer_string_record(type_: &ParameterType) -> bool {
    type_.is_builtin_named("net", "Address")
        || type_.is_builtin_named("udp", "Datagram")
        || type_.is_builtin_named("audio", "AudioDevice")
}

/// True when `field_type` occupies a record slot as a pointer to a separate
/// allocation (nested record/union/collection/`Result`/`Error`).
///
/// **A resource field is NOT one of these** (plan-114-B). A concrete resource
/// nominal matches no arm below — resources are registered in `resource_names`,
/// not `record_fields` (`engine/validation/validation.rs:264` populates the
/// latter only for `"type" | "record"` kinds) — so it is classified as a plain
/// 8-byte inline scalar slot holding the handle pointer. That is the wanted
/// layout, and it is the same rule `is_pointer_collection_payload_type` (`:49`)
/// already applies to a collection slot: "a resource handle is a single 8-byte
/// pointer to its record; a slot stores a copy of that pointer exactly like any
/// other pointer payload (§15.6)". A resource *union* IS a pointer composite and
/// is caught by the `union_names` arm.
pub(crate) fn record_field_is_pointer(model: &TypeModel, field_type: &ParameterType) -> bool {
    typed_is_collection_type(field_type)
        || model.record_fields.contains_key(field_type)
        // A resource union is a pointer composite (its value is a pointer to a
        // `{tag, ptr}` block), never a flat block. A transferred stateful union
        // is spelled `Stream STATE Cursor`; base-strip so the STATE suffix does
        // not misclassify it as a flat scalar (plan-75 gap 3, else
        // `type_is_arena_transferable` would route the transfer copy to
        // `copy_flat_block` and alias the +8 variant record).
        || model
            .union_names
            .contains(&base_resource_type(field_type))
        || matches!(field_type, ParameterType::ResultOf(_))
        || field_type.is_named("Error")
}

/// The payload value types a collection stores: the element type for a `List`,
/// the key and value types for a `Map`.
fn collection_payload_types(type_: &ParameterType) -> Vec<ParameterType> {
    if let Some(element) = typed_list_element_type(type_) {
        vec![element.clone()]
    } else if let Some((key, value)) = typed_map_type_parts(type_) {
        vec![key.clone(), value.clone()]
    } else {
        Vec::new()
    }
}

/// A resource type's base, as a type: the `STATE T` clause stripped. A
/// transferred stateful union is spelled `Stream STATE Cursor` but the union set
/// is keyed on the bare `Stream` (plan-75 gap 3), so every `union_names` lookup
/// goes through this. `Stateful` is a variant since plan-111-A, so this is a
/// match, not a suffix strip — but the composite-base spelling `parse` declines
/// to split still arrives as one opaque `Named`, which is why the `&str` adapter
/// remains the authority for that case.
fn base_resource_type(type_: &ParameterType) -> ParameterType {
    match type_ {
        ParameterType::Stateful { base, .. } => (**base).clone(),
        other => other.without_state(),
    }
}

/// Which of the two questions [`flatness_walk`] is answering. plan-114-B split
/// these apart: `type_is_flat` used to answer both with one predicate, which was
/// invisible only because no type existed for which they differ. A record holding
/// a resource pointer is the first that does — within a thread a `memcpy` is
/// exactly right (it copies the handle pointer, aliasing the one resource,
/// §15.6), but across an arena boundary the same `memcpy` produces a pointer into
/// the *sender's* arena.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Flatness {
    /// "Does a `memcpy` correctly COPY this, within one thread?"
    MemcpyCopyable,
    /// "May this block be RELOCATED into another thread's arena?"
    ArenaTransferable,
}

/// True when a `memcpy` of this value's block is a correct copy within one
/// thread. See [`CodeBuilder::type_is_memcpy_copyable`] for the rule set.
pub(crate) fn type_is_memcpy_copyable(model: &TypeModel, type_: &ParameterType) -> bool {
    let mut visited = std::collections::HashSet::new();
    flatness_walk(model, type_, Flatness::MemcpyCopyable, &mut visited)
}

/// True when this value's block may be relocated into another thread's arena.
/// See [`CodeBuilder::type_is_arena_transferable`] for the rule set.
pub(crate) fn type_is_arena_transferable(model: &TypeModel, type_: &ParameterType) -> bool {
    let mut visited = std::collections::HashSet::new();
    flatness_walk(model, type_, Flatness::ArenaTransferable, &mut visited)
}

/// The one walk behind both predicates. The structural arms (collection payloads,
/// record fields, union variants, `ResultOf`, the cycle guard) exist once and so
/// cannot drift apart; `mode` changes the answer in exactly the three leaf arms
/// marked below.
fn flatness_walk(
    model: &TypeModel,
    type_: &ParameterType,
    mode: Flatness,
    visited: &mut std::collections::HashSet<ParameterType>,
) -> bool {
    if !visited.insert(type_.clone()) {
        // Already on the current path: a type cycle. Cyclic values cannot be
        // a single finite flat block, so treat them as pointers.
        return false;
    }
    let result = if *type_ == ParameterType::String {
        true
    } else if let ParameterType::Res(_) = type_ {
        // DIVERGENCE 1 — a `RES`-marked element/map value. The slot holds one
        // 8-byte pointer to the resource record, so a `memcpy` copying it is a
        // correct alias (§15.6) but relocating it into another arena is not.
        //
        // Before plan-114-B there was no `Res` arm at all: it fell through to
        // `!record_field_is_pointer(..)`, which has no `Res` arm either, so the
        // answer was `true` INCIDENTALLY rather than by decision — and `true` is
        // the wrong answer for the transfer path. bug-483 is what that class of
        // accident costs when the default happens to be wrong.
        mode == Flatness::MemcpyCopyable
    } else if let ParameterType::ResultOf(payload) = type_ {
        // A flat `Result` `{tag, size, payload}` is pointer-free when its
        // success payload is flat (the `Err` variant is the now-flat `Error`).
        flatness_walk(model, payload, mode, visited)
    } else if typed_is_collection_type(type_) {
        // A collection is flat when every payload is flat — including a nested
        // flat collection, which is inlined in the data region (plan-02 §4.4,
        // Phase 5a). A resource or recursive payload makes it non-flat.
        collection_payload_types(type_)
            .into_iter()
            .all(|p| flatness_walk(model, &p, mode, visited))
    } else if model.record_fields.contains_key(type_) {
        !is_pointer_string_record(type_)
            && model
                .record_fields
                .get(type_)
                .cloned()
                .unwrap_or_default()
                .iter()
                .all(|(_, ft)| flatness_walk(model, ft, mode, visited))
    } else if union_is_data(model, type_) {
        model
            .variants_for_union(type_)
            .cloned()
            .collect::<Vec<_>>()
            .iter()
            .all(|variant| flatness_walk(model, variant, mode, visited))
    } else if crate::codegen::builtins::is_resource_type(&type_) {
        // NOT a divergence — `false` for both modes, unchanged from `type_is_flat`.
        //
        // plan-114-B C6: the plan's §4.1 table said `true` for MemcpyCopyable
        // here, by analogy with the `RES`-marked element above. That analogy is
        // wrong, and the artifact gate proved it. The two positions are different:
        //
        //   * `Res(inner)` is an **element/field marker**. The enclosing block owns
        //     an 8-byte slot holding the handle, so the enclosing block is still a
        //     single pointer-free run of bytes and a `memcpy` of it is correct.
        //   * A **bare resource nominal** is the value's OWN type. "Flat" would
        //     assert that the resource record itself is a copyable block that
        //     `arena_free` reclaims as a unit — and it is not. It is separately
        //     allocated with its own lifetime and its own close op.
        //
        // Answering `true` here does not stay local to resources, either: it
        // propagates through every structural arm. `Result OF tcp.Socket` became
        // "flat", so `is_freeable_flat_value` newly claimed it and registered a
        // `pending_temp` free that had never existed — measured as a real
        // `.ncode` diff in `tests/byte-identity/tcp`, a fixture with no thread in
        // it at all.
        false
    } else {
        // DIVERGENCE 3 (by inheritance) — a resource *union* reaches
        // `record_field_is_pointer`, which routes it to the pointer-composite
        // path: a `{tag, record-ptr}` block is not a plain slot, so it is false
        // for both modes. Written down here so the next reader does not have to
        // re-derive why the resource arms above do not cover it.
        //
        // Everything else is a scalar (not a pointer composite, `String`, or
        // resource) and is flat for both modes.
        !record_field_is_pointer(model, type_)
    };
    visited.remove(type_);
    result
}

/// True when field `field_type` of `record_type` is inlined into the record's
/// trailing data region (the slot holds a block-relative offset): an inlined
/// `String`, or a fully-flat composite. Scalars stay inline in the slot;
/// not-yet-flat composites stay pointers.
///
/// **A resource field is NOT inlined** (plan-114-B): it is not a composite, so
/// `is_composite` is false and the `type_is_memcpy_copyable` term is never
/// reached. It stays a value slot holding the handle pointer, which is what
/// makes `emit_record_block_size_to_slot` (`:794`) contribute exactly its 8
/// bytes from the fixed `8 * fields.len()` term and `continue` past it without
/// walking a sub-block that does not exist.
pub(crate) fn record_field_is_inlined(
    model: &TypeModel,
    record_type: &ParameterType,
    field_type: &ParameterType,
) -> bool {
    if is_pointer_string_record(record_type) {
        return false;
    }
    if *field_type == ParameterType::String {
        return true;
    }
    let is_composite = model.record_fields.contains_key(field_type)
        || model.union_names.contains(field_type)
        || typed_is_collection_type(field_type)
        || matches!(field_type, ParameterType::ResultOf(_));
    is_composite && type_is_memcpy_copyable(model, field_type)
}

/// True when `type_` is a **data** union (all variants are data records, no
/// resource variants). Data unions use the flat `{tag, size, data}` layout;
/// resource unions keep `{tag, resource-ptr}` and are never reshaped.
pub(crate) fn union_is_data(model: &TypeModel, type_: &ParameterType) -> bool {
    // A transferred stateful union spells `Stream STATE Cursor`; the union set
    // is keyed on the bare name `Stream` (plan-75 gap 3). Strip the suffix so a
    // resource union with STATE still classifies as all-resource.
    let type_ = base_resource_type(type_);
    if !model.union_names.contains(&type_) {
        return false;
    }
    let mut saw_variant = false;
    for variant in model.variants_for_union(&type_) {
        saw_variant = true;
        if crate::codegen::builtins::is_resource_type(&variant) {
            return false;
        }
    }
    saw_variant
}

/// The immediate structural components of a type — the types reachable in one
/// step: a record/variant's field types, a union's variant types, a collection's
/// payload types, a `Result`'s success type. Scalars, `String`, and resources
/// have none. Used to detect recursive types for thread-transfer deep copy
/// (bug-391): a recursive value is a pointer-linked graph that inline copy
/// codegen cannot reproduce without unbounded compile-time recursion.
pub(crate) fn type_components(model: &TypeModel, type_: &ParameterType) -> Vec<ParameterType> {
    if let Some(fields) = model.record_fields.get(type_) {
        return fields.iter().map(|(_, ft)| ft.clone()).collect();
    }
    if let Some(fields) = model.union_variant_fields.get(type_) {
        return fields.iter().map(|(_, ft)| ft.clone()).collect();
    }
    if model.union_names.contains(type_) {
        return model.variants_for_union(type_).cloned().collect();
    }
    if let Some(element) = typed_list_element_type(type_) {
        return vec![element.clone()];
    }
    if let Some((key, value)) = typed_map_type_parts(type_) {
        return vec![key.clone(), value.clone()];
    }
    if let ParameterType::ResultOf(payload) = type_ {
        return vec![(**payload).clone()];
    }
    Vec::new()
}

/// True when copying `type_` can transitively re-encounter `type_` itself — it
/// participates in a recursive type definition (e.g. `dom::Node`, whose
/// `ElementNode.children` is `List OF Node`). Such a value needs a per-type
/// runtime copy function rather than inline copy codegen.
pub(crate) fn type_participates_in_cycle(model: &TypeModel, type_: &ParameterType) -> bool {
    let mut stack = type_components(model, type_);
    let mut visited: std::collections::HashSet<ParameterType> = std::collections::HashSet::new();
    while let Some(current) = stack.pop() {
        if current == *type_ {
            return true;
        }
        if !visited.insert(current.clone()) {
            continue;
        }
        stack.extend(type_components(model, &current));
    }
    false
}

/// True when a value of `type_` is a **pointer-linked graph because of a type
/// cycle** — `type_` itself participates in one, or some type reachable from it
/// does. Strictly wider than [`type_participates_in_cycle`], which only answers
/// for the cycle members themselves: in
///
/// ```text
/// TYPE Rep      : child AS Tree, lo AS Integer
/// UNION Tree    : Leaf | Node2
/// TYPE Node2    : left AS Tree, right AS Tree
/// ```
///
/// `Tree` and `Node2` participate; `Rep` does not — yet a `Rep` value still owns
/// a pointer to a separately-allocated `Tree` graph, so it is exactly as
/// alias-prone as the cycle members are.
///
/// bug-538: this is the class where an ordinary `collections::get` handed back an
/// ALIAS into the container's data region instead of the independent value
/// `materialize_owned_element` gives every other element type, because
/// `is_freeable_flat_value` (which needs `type_is_memcpy_copyable`) is false for
/// all of them. It is used ONLY to widen that owning copy; the cycle-member
/// predicate still decides which types get a runtime copy function emitted.
pub(crate) fn type_reaches_cycle(model: &TypeModel, type_: &ParameterType) -> bool {
    let mut stack = vec![type_.clone()];
    let mut visited: std::collections::HashSet<ParameterType> = std::collections::HashSet::new();
    while let Some(current) = stack.pop() {
        if !visited.insert(current.clone()) {
            continue;
        }
        if type_participates_in_cycle(model, &current) {
            return true;
        }
        stack.extend(type_components(model, &current));
    }
    false
}

/// True when a resource is reachable from `type_` — the type itself, a `RES`
/// marker, a field, a variant or a collection payload.
///
/// A resource is move-only: its handle names an OS object with its own close op,
/// so "copy the value" is not a meaning it has. Every deep-copy path that meets
/// one therefore MOVES the handle. bug-538's owning copy must not be applied to
/// such a value — an alias is the correct and existing behaviour there — so the
/// widened gate excludes this set explicitly rather than by accident.
pub(crate) fn type_contains_resource(model: &TypeModel, type_: &ParameterType) -> bool {
    let mut stack = vec![type_.clone()];
    let mut visited: std::collections::HashSet<ParameterType> = std::collections::HashSet::new();
    while let Some(current) = stack.pop() {
        if !visited.insert(current.clone()) {
            continue;
        }
        if matches!(
            current,
            ParameterType::Res(_) | ParameterType::ThreadHandle { .. }
        ) || crate::codegen::builtins::is_resource_type(&current)
        {
            return true;
        }
        if let ParameterType::Res(inner) = &current {
            stack.push((**inner).clone());
        }
        stack.extend(type_components(model, &current));
    }
    false
}

/// Every type in the program that participates in a cycle: the set that needs a
/// runtime thread-transfer deep-copy function emitted (bug-391).
pub(crate) fn recursive_transfer_types(model: &TypeModel) -> std::collections::BTreeSet<String> {
    let mut seen: std::collections::HashSet<ParameterType> = std::collections::HashSet::new();
    let mut result: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    // plan-111-E: the WALK is typed; only the RESULT renders, because it feeds
    // emitted symbol names (`thread_copy_symbol`). The `BTreeSet<String>` return
    // also fixes the emission order to the rendered names' order, which is
    // observable in the `.ncode` — keep it.
    let mut stack: Vec<ParameterType> = model
        .record_fields
        .keys()
        .chain(model.union_names.iter())
        .cloned()
        .collect();
    while let Some(current) = stack.pop() {
        if !seen.insert(current.clone()) {
            continue;
        }
        if type_participates_in_cycle(model, &current) {
            result.insert(current.name().into_owned());
        }
        stack.extend(type_components(model, &current));
    }
    result
}

/// The internal symbol of the per-type thread-transfer deep-copy function.
pub(crate) fn thread_copy_symbol(type_: &ParameterType) -> String {
    // An emitted symbol name, so the type renders here — the one legitimate
    // render in this file, at the type -> symbol boundary.
    let type_ = type_.name();
    let mut sanitized = String::new();
    for ch in type_.chars() {
        if ch.is_ascii_alphanumeric() {
            sanitized.push(ch);
        } else {
            sanitized.push('_');
        }
    }
    let mut hash: u64 = 1469598103934665603;
    for byte in type_.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(1099511628211);
    }
    format!("_mfb_thread_copy_{sanitized}_{hash:016x}")
}

#[cfg(test)]
mod pointer_string_record_tests {
    use super::*;
    use crate::codegen::registry::registry;

    /// The three helper-built records, in both spellings the compiler can hand
    /// this predicate.
    ///
    /// bug-483: a *signature* type (a member's parameter or return) is rewritten
    /// to the package-qualified id by `Registry::qualify_value_type_references`,
    /// while a record *field* type is deliberately left bare so the injected
    /// companion source stays parseable. Both spellings therefore reach
    /// `is_pointer_string_record` for the same record, and both must answer the
    /// same — a disagreement silently reclassifies the record's layout, and its
    /// readers then dereference an absolute pointer as a block-relative offset.
    const POINTER_STRING: &[(&str, &str)] = &[
        ("net", "Address"),
        ("udp", "Datagram"),
        ("audio", "AudioDevice"),
    ];

    #[test]
    fn both_spellings_of_a_pointer_string_record_agree() {
        for (package, leaf) in POINTER_STRING {
            let qualified = format!("{package}.{leaf}");
            assert!(
                is_pointer_string_record(&ParameterType::declared(leaf)),
                "bare `{leaf}` must classify as a pointer-string record"
            );
            assert!(
                is_pointer_string_record(&ParameterType::declared(&qualified)),
                "qualified `{qualified}` must classify as a pointer-string record: \
                 a member signature returning it carries the qualified spelling, \
                 and missing it inlines the record's String fields while the \
                 runtime helper writes pointers (bug-483)"
            );
        }
    }

    /// The names above are the ones the registry actually declares. A record
    /// renamed or moved to another package would otherwise leave this predicate
    /// matching a name nothing produces — the same silent miss as bug-483, just
    /// arrived at from the other side.
    #[test]
    fn every_pointer_string_record_is_declared_where_it_claims() {
        for (package, leaf) in POINTER_STRING {
            let pkg = registry()
                .packages()
                .iter()
                .find(|p| p.import_name() == *package)
                .unwrap_or_else(|| panic!("registry has no `{package}` package"));
            assert!(
                pkg.records().iter().any(|r| r.name == *leaf),
                "`{package}` no longer declares a `{leaf}` record; \
                 `is_pointer_string_record` is matching a dead name"
            );
        }
    }

    /// The predicate must not answer true for anything else — it is a
    /// hand-maintained exception list, and a stray match would put an ordinary
    /// record's inlined `String` fields on the pointer layout.
    #[test]
    fn no_other_declared_record_is_pointer_string() {
        for pkg in registry().packages() {
            for record in pkg.records() {
                let qualified = format!("{}.{}", pkg.import_name(), record.name);
                let expected = POINTER_STRING
                    .iter()
                    .any(|(p, l)| *p == pkg.import_name() && *l == record.name);
                assert_eq!(
                    is_pointer_string_record(&ParameterType::declared(&qualified)),
                    expected,
                    "`{qualified}` classified unexpectedly"
                );
            }
        }
    }
}

#[cfg(test)]
mod kind2_layout_tests {
    use super::*;

    /// Every element type the entry-free representation applies to, with its
    /// payload width, and every one it must NOT apply to.
    const FIXED_WIDTH: &[(&str, usize)] = &[
        ("Boolean", 1),
        ("Byte", 1),
        ("Scalar", 4),
        ("Integer", 8),
        ("Float", 8),
        ("Fixed", 8),
        ("Money", 8),
    ];
    const VARIABLE_WIDTH: &[&str] = &[
        "String",
        "List OF Integer",
        "List OF List OF Integer",
        "Map OF Integer TO Integer",
        "Unknown",
        "",
    ];

    /// The three functions that select the representation must never disagree.
    ///
    /// They are consulted at different sites — `list_entry_stride` by the
    /// allocator and by `emit_flat_block_size`, `list_block_kind` by the header
    /// writers, `kind2_payload_size` by element access and iteration. A block
    /// allocated under one answer and read or freed under another is bug-02: the
    /// arena free list is corrupted rather than a wrong value returned. This is
    /// the invariant that makes the "entry stride lever" safe.
    #[test]
    fn representation_selectors_agree() {
        let every_type = FIXED_WIDTH
            .iter()
            .map(|(element, _)| *element)
            .chain(VARIABLE_WIDTH.iter().copied());
        for spelling in every_type {
            let element = ParameterType::parse(spelling);
            let entry_free = list_entry_stride(&element) == 0;
            assert_eq!(
                entry_free,
                list_block_kind(&element) == COLLECTION_KIND_LIST_FIXED,
                "stride and kind disagree for {element:?}"
            );
            assert_eq!(
                entry_free,
                kind2_payload_size(&element).is_some(),
                "stride and payload size disagree for {element:?}"
            );
        }
    }

    /// The allocation size and the free size must agree, for the reason they can
    /// actually disagree: they read the stride off DIFFERENT type strings.
    ///
    /// `lower_collection_values` allocates `HEADER + count*stride + dataLen`
    /// using the first slot's ACTUAL element type; `emit_flat_block_size` frees
    /// `HEADER + capacity*stride + dataCapacity` using the element type parsed
    /// out of the collection's DECLARED type. When inference gives up and the
    /// declared type is `List OF Unknown`, those two are different answers —
    /// stride 0 versus stride 40 — and the free runs `capacity * 40` bytes past
    /// the end of the block. That is bug-02's failure mode and it is reachable
    /// from `[toByte(1), toByte(2)]`, so the placeholder is refined before it
    /// reaches either side.
    ///
    /// Asserting `alloc == free` with both sides computed the same way would be a
    /// tautology; this models each side from its own type string.
    #[test]
    fn alloc_size_matches_free_size() {
        for &(element, width) in FIXED_WIDTH {
            for declared in [format!("List OF {element}"), "List OF Unknown".to_string()] {
                let declared_type = ParameterType::declared(&declared);
                let laid_out = refined_list_literal_type(
                    &declared_type,
                    Some(&ParameterType::declared(element)),
                )
                .unwrap_or_else(|| declared_type.clone());
                for count in [0usize, 1, 2, 7, 1000] {
                    // Allocation: stride from the element's own type.
                    let allocated = COLLECTION_HEADER_SIZE
                        + count * list_entry_stride(&ParameterType::declared(element))
                        + count * width;
                    // Free: stride from the type the block was laid out as.
                    let free_element = typed_list_element_type(&laid_out)
                        .cloned()
                        .unwrap_or_else(|| ParameterType::named(""));
                    let freed = COLLECTION_HEADER_SIZE
                        + count * list_entry_stride(&free_element)
                        + count * width;
                    assert_eq!(
                        allocated, freed,
                        "alloc/free disagree for {count} x {element} declared as {declared}"
                    );
                    // And the entry-free size really is the payoff being claimed:
                    // 40 + N*width, not 40 + N*(40 + width).
                    assert_eq!(
                        allocated,
                        COLLECTION_HEADER_SIZE + count * width,
                        "{element} x{count} is not entry-free"
                    );
                }
            }
        }
    }

    /// The refinement fires only where it is needed: a declared element type that
    /// inference actually produced is never second-guessed, and an empty literal
    /// or a map has nothing to refine from.
    #[test]
    fn literal_type_refinement_is_narrow() {
        let refine = |declared: &str, element: Option<&str>| {
            refined_list_literal_type(
                &ParameterType::declared(declared),
                element.map(ParameterType::declared).as_ref(),
            )
            .map(|t| t.name().into_owned())
        };
        assert_eq!(
            refine("List OF Unknown", Some("Byte")).as_deref(),
            Some("List OF Byte")
        );
        assert_eq!(refine("List OF Unknown", None), None);
        assert_eq!(refine("List OF Byte", Some("Byte")), None);
        assert_eq!(refine("List OF String", Some("String")), None);
        assert_eq!(refine("Map OF Integer TO Integer", Some("Integer")), None);
    }

    /// A nested list keeps its lookup table: `List OF List OF Integer` is a list
    /// whose *elements* are variable-length blocks, so only the inner lists go
    /// entry-free. §3 flagged this as a risk concentration.
    #[test]
    fn nested_list_outer_keeps_entries() {
        assert_eq!(
            list_entry_stride(&ParameterType::declared("List OF List OF Integer")),
            COLLECTION_ENTRY_SIZE
        );
        assert_eq!(
            list_entry_stride(&ParameterType::declared("List OF Integer")),
            COLLECTION_ENTRY_SIZE
        );
        assert_eq!(list_entry_stride(&ParameterType::Integer), 0);
        assert_eq!(
            list_block_kind(&ParameterType::declared("List OF Integer")),
            COLLECTION_KIND_LIST
        );
    }

    /// A `RES` element marker is an ownership axis, not part of the value type,
    /// and a resource pointer is never a fixed-width payload.
    #[test]
    fn resource_elements_keep_entries() {
        assert_eq!(
            list_entry_stride(&ParameterType::declared("fs.File")),
            COLLECTION_ENTRY_SIZE
        );
        assert_eq!(
            list_entry_stride(&ParameterType::declared("RES File")),
            COLLECTION_ENTRY_SIZE
        );
    }
}

/// plan-114-B: the two predicates that replaced `type_is_flat`.
///
/// These assert the DIVERGENCE explicitly. The whole point of the split is that
/// "a `memcpy` copies this correctly within one thread" and "this block may be
/// relocated into another thread's arena" are different questions; for every
/// type that existed before a resource could sit in a record they coincide, so
/// only the resource-carrying cases can catch a regression that re-merges them.
#[cfg(test)]
mod flatness_split_tests {
    use super::*;

    /// A model with one record: `Holder { name AS String, handle AS RES fs.File }`.
    /// The source-level ban is still up (letter D lifts it), so the only way to
    /// reach this shape is to build the model by hand.
    fn model_with_res_field_record() -> TypeModel {
        let mut model = TypeModel::empty();
        model.record_fields.insert(
            ParameterType::declared("Holder"),
            vec![
                ("name".to_string(), ParameterType::String),
                ("handle".to_string(), ParameterType::parse("RES fs.File")),
            ],
        );
        model
    }

    /// The shapes that genuinely diverge: a `RES`-marked type used directly, and
    /// a record carrying one as a field. Both are memcpy-copyable (the slot holds
    /// one 8-byte pointer, and copying it aliases the resource, §15.6) and
    /// neither is arena-transferable (that pointer would arrive pointing into the
    /// sender's arena).
    ///
    /// C7: this list is deliberately SHORT, and the collection spellings are
    /// absent on purpose — see `a_res_collection_does_not_diverge`.
    #[test]
    fn a_res_field_and_its_record_diverge_between_the_two_predicates() {
        let model = model_with_res_field_record();
        for spelling in ["RES fs.File", "Holder"] {
            let type_ = ParameterType::parse(spelling);
            assert!(
                type_is_memcpy_copyable(&model, &type_),
                "`{spelling}` must be memcpy-copyable: a handle slot is one \
                 8-byte pointer and copying it aliases the resource (§15.6)"
            );
            assert!(
                !type_is_arena_transferable(&model, &type_),
                "`{spelling}` must NOT be arena-transferable: the handle would \
                 arrive pointing into the sender's arena"
            );
        }
    }

    /// A `RES` **collection** does not reach the `Res(_)` arm at all, so it does
    /// not diverge — it is flat in neither mode, exactly as before plan-114-B.
    ///
    /// C7: `typed_list_element_type` / `typed_map_type_parts` **strip** the `RES`
    /// marker (`type_utils.rs:345`, `:352`), so `collection_payload_types`
    /// yields a BARE `fs.File` and the walk takes the bare-resource arm. The
    /// `Res(_)` arm is reachable only where a type is stored unstripped — a
    /// record field, which is exactly the new shape this letter exists for.
    ///
    /// Pinned because the plan predicted the opposite, and because it is what
    /// keeps a resource-carrying collection out of `copy_flat_block` and out of
    /// `is_freeable_flat_value` — both of which would be wrong for it.
    #[test]
    fn a_res_collection_does_not_diverge() {
        let model = model_with_res_field_record();
        for spelling in [
            "List OF RES fs.File",
            "Map OF String TO RES fs.File",
            "List OF List OF RES fs.File",
        ] {
            let type_ = ParameterType::parse(spelling);
            assert!(
                !type_is_memcpy_copyable(&model, &type_),
                "`{spelling}` is flat in neither mode: its payload strips to a \
                 bare resource, which is not a self-contained block"
            );
            assert!(!type_is_arena_transferable(&model, &type_), "`{spelling}`");
        }
    }

    /// A bare resource nominal does **not** diverge — it is `false` for both,
    /// and that is deliberate (C6). `Res(inner)` marks a *slot inside* an
    /// enclosing block, which stays flat; a bare nominal is the value's own type,
    /// and the resource record it names is separately allocated with its own
    /// lifetime, so it is not a block anything may `memcpy` or `arena_free` as a
    /// unit.
    ///
    /// This is a regression test with teeth: answering `true` for the memcpy
    /// question here propagates through every structural arm — it made
    /// `Result OF fs.File` "flat", which made `is_freeable_flat_value` claim it
    /// and emit a `pending_temp` free that had never existed. That was a measured
    /// `.ncode` diff in a thread-free fixture, not a theoretical concern.
    #[test]
    fn a_bare_resource_nominal_is_flat_in_neither_mode() {
        let model = TypeModel::empty();
        let file = ParameterType::declared("fs.File");
        assert!(!type_is_memcpy_copyable(&model, &file));
        assert!(!type_is_arena_transferable(&model, &file));

        // And it must not leak through a structural arm into a wrapper type.
        let wrapped = ParameterType::ResultOf(Box::new(file));
        assert!(
            !type_is_memcpy_copyable(&model, &wrapped),
            "`Result OF fs.File` must not become flat — is_freeable_flat_value \
             would claim it and free a block that is not one"
        );
        assert!(!type_is_arena_transferable(&model, &wrapped));
    }

    /// The regression net for the split: every type that existed before a
    /// resource could sit in a record must answer IDENTICALLY to both
    /// predicates. If a refactor makes one of these diverge, the split has
    /// leaked into ordinary types.
    #[test]
    fn every_resource_free_type_answers_both_predicates_the_same() {
        let mut model = TypeModel::empty();
        model.record_fields.insert(
            ParameterType::declared("Plain"),
            vec![
                ("count".to_string(), ParameterType::Integer),
                ("label".to_string(), ParameterType::String),
            ],
        );
        // (spelling, expected answer for BOTH predicates)
        let cases: &[(&str, bool)] = &[
            ("Integer", true),
            ("Boolean", true),
            ("Byte", true),
            ("Float", true),
            ("Money", true),
            ("String", true),
            ("List OF Integer", true),
            ("List OF String", true),
            ("Map OF String TO Integer", true),
            ("List OF List OF Integer", true),
            ("Result OF Integer", true),
            ("Plain", true),
            // `Error` is a pointer composite, so it is flat in neither mode.
            ("Error", false),
        ];
        for (spelling, expected) in cases {
            let type_ = ParameterType::parse(spelling);
            assert_eq!(
                type_is_memcpy_copyable(&model, &type_),
                *expected,
                "`{spelling}` memcpy-copyable"
            );
            assert_eq!(
                type_is_arena_transferable(&model, &type_),
                *expected,
                "`{spelling}` arena-transferable — must match memcpy-copyable \
                 for every resource-free type"
            );
        }
    }

    /// The cycle guard must survive the rewrite in both modes: a self-referential
    /// record is not a finite flat block and must stay a pointer.
    #[test]
    fn a_cyclic_record_is_flat_in_neither_mode() {
        let mut model = TypeModel::empty();
        model.record_fields.insert(
            ParameterType::declared("Node"),
            vec![("next".to_string(), ParameterType::declared("Node"))],
        );
        let node = ParameterType::declared("Node");
        assert!(!type_is_memcpy_copyable(&model, &node));
        assert!(!type_is_arena_transferable(&model, &node));
    }
}

/// plan-114-B Phase 2: the layout, copy, size and drop of a record carrying a
/// `RES` field — `Holder { name AS String, handle AS RES fs.File }`.
///
/// The source-level ban is still up (letter D lifts it), so no fixture can reach
/// this shape; the model is built by hand. Each test asserts the property at the
/// point codegen actually decides it, and names the emitter that consumes the
/// answer, so a change to either side breaks a test rather than silently
/// diverging.
#[cfg(test)]
mod res_field_record_layout_tests {
    use super::*;

    fn holder_model() -> (TypeModel, ParameterType) {
        let mut model = TypeModel::empty();
        let holder = ParameterType::declared("Holder");
        model.record_fields.insert(
            holder.clone(),
            vec![
                ("name".to_string(), ParameterType::String),
                ("handle".to_string(), ParameterType::parse("RES fs.File")),
            ],
        );
        (model, holder)
    }

    /// (a) The handle occupies a plain 8-byte value slot at `8*index`.
    ///
    /// `record_field_is_pointer` false + `record_field_is_inlined` false is
    /// exactly the classification `emit_build_inlined_record`'s `else` branch
    /// keys on (`memory/marshal/record.rs:163-168`), which emits
    /// `load [sp+field_slot]` / `store -> [record + 8*index]` — i.e. the field's
    /// lowered value, which for a resource is its record pointer.
    #[test]
    fn the_handle_field_is_a_plain_value_slot_not_a_pointer_or_inlined_block() {
        let (model, holder) = holder_model();
        let handle = ParameterType::parse("RES fs.File");

        assert!(
            !record_field_is_pointer(&model, &handle),
            "a resource field must not be classified as a pointer composite — \
             that would give it a separate allocation instead of a handle slot"
        );
        assert!(
            !record_field_is_inlined(&model, &holder, &handle),
            "a resource field must not be inlined into the data region — its \
             slot holds the handle, not a block-relative offset"
        );
        // Contrast: the String field IS inlined, so the record really does
        // exercise both branches of the write loop.
        assert!(record_field_is_inlined(
            &model,
            &holder,
            &ParameterType::String
        ));
    }

    /// (b) A `memcpy` of the block is a correct copy, and the copied handle word
    /// therefore EQUALS the source's — the copy aliases the one resource rather
    /// than duplicating it (§15.6).
    ///
    /// `copy_value_to_current_arena` routes to `copy_flat_block` on
    /// `type_is_arena_transferable`; `is_freeable_flat_value` and the in-thread
    /// layout sites key on `type_is_memcpy_copyable`. The pair below is what
    /// makes an in-thread copy a `memcpy` while a thread transfer refuses.
    #[test]
    fn the_record_copies_by_memcpy_but_may_not_cross_an_arena() {
        let (model, holder) = holder_model();
        assert!(
            type_is_memcpy_copyable(&model, &holder),
            "an in-thread copy of Holder is a memcpy: the handle word is copied \
             verbatim, aliasing the same resource record"
        );
        assert!(
            !type_is_arena_transferable(&model, &holder),
            "Holder must NOT be arena-transferable: the copied handle would \
             point into the sender's arena"
        );
    }

    /// (c) The block sizes to `8 * fields.len()` plus the inlined String's bytes
    /// — the resource field contributes exactly its 8 bytes and no sub-block.
    ///
    /// `emit_record_block_size_to_slot` (`:794`) starts at `8 * fields.len()`
    /// and `continue`s past every non-inlined field, so the fixed term below IS
    /// the resource field's whole contribution.
    #[test]
    fn the_handle_field_contributes_exactly_eight_bytes_and_no_sub_block() {
        let (model, holder) = holder_model();
        let fields = model.record_fields.get(&holder).cloned().unwrap();
        assert_eq!(fields.len(), 2);
        assert_eq!(8 * fields.len(), 16, "the fixed slot region is 8 per field");

        // Exactly one field walks the inlined-sub-block path; the handle is
        // skipped, so the only variable term is the String's.
        let inlined: Vec<&str> = fields
            .iter()
            .filter(|(_, ft)| record_field_is_inlined(&model, &holder, ft))
            .map(|(n, _)| n.as_str())
            .collect();
        assert_eq!(
            inlined,
            vec!["name"],
            "only the String field is inlined; the handle contributes no sub-block"
        );
    }

    /// (d) Scope-drop frees the record block and nothing else.
    ///
    /// `is_freeable_flat_value` gates the generic owned-value `arena_free` on
    /// memcpy-copyability plus "is a record/String/collection/data-union/Result".
    /// Holder qualifies, so ONE `arena_free` reclaims the record block. The
    /// resource record itself is not reachable from that path — it is reclaimed
    /// by the resource's own `ActiveCleanup::Resource`, which is letter C's
    /// routing — so the drop must not touch it.
    #[test]
    fn the_record_block_is_freeable_but_the_resource_record_is_not() {
        let (model, holder) = holder_model();
        assert!(
            type_is_memcpy_copyable(&model, &holder) && model.record_fields.contains_key(&holder),
            "both terms of is_freeable_flat_value hold for Holder, so scope-drop \
             emits one arena_free for its block"
        );
        // The resource itself is NOT a freeable flat value: it is not a record,
        // String, collection, data union or Result, so the generic owned-value
        // path cannot reach it however the record is dropped.
        let handle = ParameterType::parse("RES fs.File");
        assert!(
            !model.record_fields.contains_key(&handle)
                && !typed_is_collection_type(&handle)
                && handle != ParameterType::String,
            "the resource field must not satisfy is_freeable_flat_value's second \
             term, or scope-drop would arena_free the resource record too"
        );
    }
}
