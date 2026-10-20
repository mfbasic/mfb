//! plan-176-Q: payload sizes and alignment, and copying blocks and collections. The list-layout
//! helpers live in `collection/layout/builder_collection_layout/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    pub(crate) fn inline_collection_payload_size(&self, type_: &ParameterType) -> Option<usize> {
        if let Some(fields) = self.type_model.record_fields.get(type_) {
            return Some(8 * fields.len());
        }
        // A concrete resource value IS its record pointer, never an inline payload —
        // even when it is a variant of a declared resource union. The variant lookup
        // below answered for it with the UNION's `{tag, ptr}` size, so declaring
        // `UNION Stream { fs::File, … }` anywhere in a program made `RETURN f` of a
        // plain `File` copy 16 bytes of the record into a fresh block and hand the
        // caller that (a "moved"/"closed" read of a truncated record) — found while
        // auditing bug-805's heterogeneous-record sibling.
        if crate::codegen::builtins::is_resource_type(type_)
            || self
                .type_model
                .resource_names
                .contains(&type_.without_state())
        {
            return None;
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
                    // bug-851: a `RESOURCE`-declared variant is one handle word too.
                    if self.is_resource_nominal(variant) {
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
        // bug-546's family, third instance: this asked
        // `builtins::is_resource_type`, which answers for the BUILT-IN registry
        // only, so a user-declared `RESOURCE Db CLOSE BY sql::close` was not a
        // pointer payload. Measured before the fix — `List OF RES fs::File`
        // built and ran (`count=4`), while the identical program over a declared
        // `Db` died at build time with
        //   error: native collection packed payload does not support type 'Db'
        // even though `mfb spec` §15.6 gives collection elements the `RES` marker
        // precisely so a resource CAN be stored in one.
        //
        // That dispatch fails CLOSED (an `Err`, not a wrong size), which is why
        // this surfaced as a build error rather than a corrupted stride — the
        // same defect behind a fail-open predicate would have been silent.
        self.is_resource_nominal(type_) && !self.type_model.union_names.contains(type_)
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
            ParameterType::Nothing
            | ParameterType::Boolean
            | ParameterType::Byte
            | ParameterType::String => 1,
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
            // NOT `_ => 1`. The fallback is delegated to an exhaustive helper so
            // a new `ParameterType` variant cannot inherit "no alignment
            // requirement" by silence — see `unaligned_payload_variants`.
            other => Self::default_payload_alignment(other),
        }
    }

    /// The 1-byte fallback of [`Self::collection_payload_alignment`], written as
    /// an EXHAUSTIVE match so the answer is chosen rather than defaulted.
    ///
    /// This used to be `_ => 1`, and of the three fall-through defaults in this
    /// file it is the most dangerous kind: a silent NUMBER. "Alignment 1" for a
    /// payload that is really 8 bytes wide misaligns every element after the
    /// first, and `mfb spec` memory-layouts (Scalar Storage) requires every
    /// payload to begin at an offset valid for its type. A wrong `false` from a
    /// predicate produces a build error somewhere; a wrong `1` from here produces
    /// a silently mis-strided collection.
    ///
    /// Several variants below are unreachable in practice because a GUARDED arm
    /// above claims them first (`Func`, the collections, an inlinable record or
    /// union, a resource pointer payload). They are still listed, because a
    /// guarded arm does not count toward exhaustiveness — Rust cannot prove the
    /// guard fires, so it demands the pattern anyway, and that is exactly what
    /// makes this technique work on a guard-heavy dispatch.
    #[deny(clippy::wildcard_enum_match_arm)]
    fn default_payload_alignment(type_: &ParameterType) -> usize {
        match type_ {
            // The genuinely-reachable 1-byte cases: a `Nothing` payload occupies
            // no aligned slot, and the remaining nominals are byte-packed.
            ParameterType::Nothing
            | ParameterType::Boolean
            | ParameterType::Byte
            | ParameterType::String
            | ParameterType::AttributeString
            | ParameterType::Named(_)
            | ParameterType::UserOf(..)
            | ParameterType::Stateful { .. }
            | ParameterType::MapEntryOf(..)
            | ParameterType::ResultOf(_)
            | ParameterType::Res(_)
            | ParameterType::ThreadHandle { .. }
            | ParameterType::C(_)
            | ParameterType::Var(_)
            | ParameterType::Arg(_)
            | ParameterType::Unknown
            | ParameterType::Integer
            | ParameterType::Float
            | ParameterType::Fixed
            | ParameterType::Money
            | ParameterType::Func(..)
            | ParameterType::ListOf(_)
            | ParameterType::MapOf(..)
            | ParameterType::SetOf(_) => 1,
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
        reg: impl Into<abi::Reg>,
        alignment: usize,
        scratch: impl Into<abi::Reg>,
    ) {
        if alignment <= 1 {
            return;
        }
        let reg = reg.into();
        let scratch = scratch.into();
        let mask = !((alignment - 1) as u64);
        self.emit(abi::add_immediate(reg, reg, alignment - 1));
        self.emit(abi::move_immediate(scratch, "Integer", &mask.to_string()));
        self.emit(abi::and_registers(reg, reg, scratch));
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
        dst: impl Into<abi::Reg>,
        src: impl Into<abi::Reg>,
        len: impl Into<abi::Reg>,
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
        dst: impl Into<abi::Reg>,
        src: impl Into<abi::Reg>,
        remaining: impl Into<abi::Reg>,
        scratch: impl Into<abi::Reg>,
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
        self.emit(abi::compare_immediate(remaining, "8"));
        self.emit(abi::branch_lo(&byte_tail));
        self.emit(abi::load_u64(scratch, src, 0));
        self.emit(abi::store_u64(scratch, dst, 0));
        self.emit(abi::add_immediate(src, src, 8));
        self.emit(abi::add_immediate(dst, dst, 8));
        self.emit(abi::subtract_immediate(remaining, remaining, 8));
        self.emit(abi::branch(&word_loop));
        self.emit(abi::label(&byte_tail));
        self.emit(abi::compare_immediate(remaining, "0"));
        self.emit(abi::branch_eq(&done_label));
        self.emit(abi::load_u8(scratch, src, 0));
        self.emit(abi::store_u8(scratch, dst, 0));
        self.emit(abi::add_immediate(src, src, 1));
        self.emit(abi::add_immediate(dst, dst, 1));
        self.emit(abi::subtract_immediate(remaining, remaining, 1));
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
        dst_end: impl Into<abi::Reg>,
        src_end: impl Into<abi::Reg>,
        remaining: impl Into<abi::Reg>,
        scratch: impl Into<abi::Reg>,
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
        self.emit(abi::compare_immediate(remaining, "8"));
        self.emit(abi::branch_lo(&byte_tail));
        self.emit(abi::subtract_immediate(src_end, src_end, 8));
        self.emit(abi::subtract_immediate(dst_end, dst_end, 8));
        self.emit(abi::load_u64(scratch, src_end, 0));
        self.emit(abi::store_u64(scratch, dst_end, 0));
        self.emit(abi::subtract_immediate(remaining, remaining, 8));
        self.emit(abi::branch(&word_loop));
        self.emit(abi::label(&byte_tail));
        self.emit(abi::compare_immediate(remaining, "0"));
        self.emit(abi::branch_eq(&done_label));
        self.emit(abi::subtract_immediate(src_end, src_end, 1));
        self.emit(abi::subtract_immediate(dst_end, dst_end, 1));
        self.emit(abi::load_u8(scratch, src_end, 0));
        self.emit(abi::store_u8(scratch, dst_end, 0));
        self.emit(abi::subtract_immediate(remaining, remaining, 1));
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
        ptr_reg: impl Into<abi::Reg>,
        out_reg: impl Into<abi::Reg>,
        scratch: impl Into<abi::Reg>,
    ) -> Result<(), String> {
        let ptr_reg = ptr_reg.into();
        let out_reg = out_reg.into();
        let scratch = scratch.into();
        match type_ {
            ParameterType::String => {
                // byteLength(+0) + 8 (length word) + 1 (trailing NUL).
                self.emit(abi::load_u64(out_reg, ptr_reg, 0));
                self.emit(abi::add_immediate(out_reg, out_reg, 9));
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
                self.emit(abi::load_u64(out_reg, ptr_reg, COLLECTION_OFFSET_CAPACITY));
                self.emit(abi::move_immediate(scratch, "Integer", &stride.to_string()));
                self.emit(abi::multiply_registers(out_reg, out_reg, scratch));
                self.emit(abi::add_immediate(out_reg, out_reg, COLLECTION_HEADER_SIZE));
                self.emit(abi::load_u64(
                    scratch,
                    ptr_reg,
                    COLLECTION_OFFSET_DATA_CAPACITY,
                ));
                self.emit(abi::add_registers(out_reg, out_reg, scratch));
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
                    self.emit(abi::load_u64(scratch, ptr_reg, COLLECTION_OFFSET_CAPACITY));
                    self.emit(abi::shift_left_immediate(scratch, scratch, 4));
                    self.emit(abi::add_registers(out_reg, out_reg, scratch));
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
        source: impl Into<abi::Reg>,
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
        // `%arg0`, not `return_register()` (`%ret0`). Byte-identical; clears the plan-71-C
        // Family-1a finding at this call.
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
        source: impl Into<abi::Reg>,
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

    /// bug-770 C: the copy of a `List` that is about to be appended to in place —
    /// a working buffer, not a value. Same entries and data as
    /// [`Self::copy_collection_tight`], but the block reserves the headroom
    /// `lower_list_append_in_place`'s grow would give the tight copy:
    /// `capacity = step(count)`, and `dataCapacity = capacity × width` for a
    /// fixed-width element (bug-621) or `step(dataLength)` otherwise. The append
    /// then writes into the spare slot instead of reallocating, so the copy and the
    /// grow are one allocation and one copy. Only the `get`/`getOr` → `MUT` →
    /// `append` self-update shape asks for it (`analysis::append_headroom`); every
    /// other copy of a value stays tight. Returns the destination pointer in a fresh
    /// register.
    pub(crate) fn copy_list_with_append_headroom(
        &mut self,
        type_: &ParameterType,
        source: impl Into<abi::Reg>,
    ) -> Result<VirtualRegister, String> {
        let element = typed_list_element_type(type_)
            .cloned()
            .ok_or_else(|| format!("append-headroom copy of a non-List type '{type_}'"))?;
        let stride = list_entry_stride(&element);
        let layout = CollectionTypeLayout::from_type(type_)
            .ok_or_else(|| format!("native code collection type '{type_}' is not supported"))?;
        let count = self.temporary_vreg();
        let data_len = self.temporary_vreg();
        let capacity = self.temporary_vreg();
        let data_cap = self.temporary_vreg();
        let scratch = self.temporary_vreg();
        let product = self.temporary_vreg();
        let cursor_dst = self.temporary_vreg();
        let cursor_src = self.temporary_vreg();
        let span = self.temporary_vreg();
        let word = self.temporary_vreg();
        let source_slot = self.allocate_stack_object("headroom_copy_source", 8);
        let result_slot = self.allocate_stack_object("headroom_copy_result", 8);
        let cap_slot = self.allocate_stack_object("headroom_copy_cap", 8);
        let dcap_slot = self.allocate_stack_object("headroom_copy_dcap", 8);
        let alloc_ok = self.label("headroom_copy_alloc_ok");
        let overflow = self.label("headroom_copy_size_overflow");
        let dcap_keep = self.label("headroom_copy_dcap_keep");
        self.emit(abi::store_u64(source, abi::stack_pointer(), source_slot));

        // capacity = step(count).
        let src = self.temporary_vreg();
        self.emit(abi::load_u64(&src, abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64(&count, &src, COLLECTION_OFFSET_COUNT));
        self.emit_geometric_step(
            &count,
            &capacity,
            &scratch,
            COLLECTION_GROW_LOOKUP_INIT,
            COLLECTION_GROW_LOOKUP_TAPER,
            "headroom_copy_cap",
        );
        self.emit(abi::store_u64(&capacity, abi::stack_pointer(), cap_slot));
        // dataCapacity = max(capacity × width, dataLength) for a fixed-width
        // element, max(step(dataLength), dataLength) otherwise.
        self.emit(abi::load_u64(
            &data_len,
            &src,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        match list_element_is_fixed_width(&element) {
            Some(width) => {
                self.emit_fixed_width_data_capacity(
                    &capacity, &data_cap, &scratch, width, &overflow,
                );
            }
            None => {
                self.emit_geometric_step(
                    &data_len,
                    &data_cap,
                    &scratch,
                    COLLECTION_GROW_DATA_INIT,
                    COLLECTION_GROW_DATA_TAPER,
                    "headroom_copy_dcap",
                );
            }
        }
        self.emit(abi::compare_registers(&data_cap, &data_len));
        self.emit(abi::branch_hi(&dcap_keep));
        self.emit(abi::branch_eq(&dcap_keep));
        self.emit(abi::move_register(&data_cap, &data_len));
        self.emit(abi::label(&dcap_keep));
        self.emit(abi::store_u64(&data_cap, abi::stack_pointer(), dcap_slot));

        // alloc size = HEADER + capacity * ENTRY + dataCapacity (checked).
        self.emit(abi::load_u64(&capacity, abi::stack_pointer(), cap_slot));
        self.emit(abi::move_immediate(
            &scratch,
            "Integer",
            &stride.to_string(),
        ));
        self.emit_checked_size_multiply(&product, &capacity, &scratch, &overflow);
        self.emit_checked_size_add_immediate(
            abi::c_arg(0),
            &product,
            COLLECTION_HEADER_SIZE,
            &overflow,
        );
        self.emit(abi::load_u64(&data_cap, abi::stack_pointer(), dcap_slot));
        self.emit_checked_size_add(abi::c_arg(0), abi::c_arg(0), &data_cap, &overflow);
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&alloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&overflow));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            result_slot,
        ));

        // Header: the source's count / dataLength, the headroom capacities.
        let dst = self.temporary_vreg();
        self.emit(abi::load_u64(&src, abi::stack_pointer(), source_slot));
        self.emit(abi::load_u64(&count, &src, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(
            &data_len,
            &src,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::load_u64(&capacity, abi::stack_pointer(), cap_slot));
        self.emit(abi::load_u64(&data_cap, abi::stack_pointer(), dcap_slot));
        self.emit(abi::load_u64(&dst, abi::stack_pointer(), result_slot));
        self.emit_write_collection_header_full(
            &layout, &dst, &count, &capacity, &data_len, &data_cap,
        );

        // Copy the live lookup entries verbatim (count * ENTRY bytes).
        if stride != 0 {
            self.emit(abi::load_u64(&dst, abi::stack_pointer(), result_slot));
            self.emit(abi::add_immediate(
                &cursor_dst,
                &dst,
                COLLECTION_HEADER_SIZE,
            ));
            self.emit(abi::load_u64(&src, abi::stack_pointer(), source_slot));
            self.emit(abi::add_immediate(
                &cursor_src,
                &src,
                COLLECTION_HEADER_SIZE,
            ));
            self.emit(abi::load_u64(&count, &src, COLLECTION_OFFSET_COUNT));
            self.emit(abi::move_immediate(
                &scratch,
                "Integer",
                &stride.to_string(),
            ));
            self.emit(abi::multiply_registers(&span, &count, &scratch));
            self.emit_block_copy_advance(
                &cursor_dst,
                &cursor_src,
                &span,
                &word,
                "headroom_copy_entries",
            );
        }

        // Copy the data region verbatim (dataLength bytes); both bases are
        // capacity-derived.
        self.emit(abi::load_u64(&dst, abi::stack_pointer(), result_slot));
        self.emit_collection_data_pointer_for(&cursor_dst, &dst, &element);
        self.emit(abi::load_u64(&src, abi::stack_pointer(), source_slot));
        self.emit_collection_data_pointer_for(&cursor_src, &src, &element);
        self.emit(abi::load_u64(&span, &src, COLLECTION_OFFSET_DATA_LENGTH));
        self.emit_block_copy_advance(&cursor_dst, &cursor_src, &span, &word, "headroom_copy_data");

        let result = self.allocate_register();
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(result)
    }
}
