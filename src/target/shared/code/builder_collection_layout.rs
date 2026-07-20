use super::*;

impl CodeBuilder<'_> {
    pub(super) fn inline_collection_payload_size(&self, type_: &str) -> Option<usize> {
        if let Some(fields) = self.type_model.record_fields.get(type_) {
            return Some(8 * fields.len());
        }
        if let Some(union_name) = self.type_model.union_variants.get(type_) {
            return self.inline_collection_payload_size(union_name);
        }
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
                    if crate::builtins::is_resource_type(variant) {
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

    pub(super) fn is_pointer_collection_payload_type(&self, type_: &str) -> bool {
        // A resource handle is a single 8-byte pointer to its record; a collection
        // slot stores a borrow of that pointer exactly like any other pointer
        // payload (§15.6). Resource *unions* carry a tag and are not pointer
        // payloads. A **flat** nested collection is inlined as its own block in
        // the data region (plan-02 §4.4, Phase 5a); only a *non-flat* nested
        // collection (one that itself embeds a pointer/resource payload) stays a
        // pointer handle.
        if is_collection_type(type_) {
            return !self.type_is_flat(type_);
        }
        crate::builtins::is_resource_type(type_) && !self.type_model.union_names.contains(type_)
    }

    /// Alignment, in bytes, that a packed collection payload of `type_` requires
    /// in the data region. 8-byte scalars (`Integer`/`Float`/`Fixed`), native
    /// collection/object pointers, and inline record/union slot payloads must
    /// begin at 8-byte boundaries; 1-byte scalars (`Boolean`/`Byte`) and UTF-8
    /// `String` bytes have no alignment requirement. `memory_layouts.md`
    /// (Scalar Storage) requires every payload to begin at an offset valid for
    /// its type, with padding bytes unobservable.
    pub(super) fn collection_payload_alignment(&self, type_: &str) -> usize {
        match type_ {
            "Boolean" | "Byte" | "String" => 1,
            "Integer" | "Float" | "Fixed" | "Money" => 8,
            // A Scalar is a 4-byte codepoint payload with alignment 4 (plan-41-C).
            "Scalar" => 4,
            // A function value is an 8-byte code/closure pointer (bug-73).
            other if is_function_type(other) => 8,
            other if self.is_pointer_collection_payload_type(other) => 8,
            other if self.inline_collection_payload_size(other).is_some() => 8,
            // An inlined flat collection block begins with `U64` header fields.
            other if is_collection_type(other) => 8,
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
    pub(super) fn list_element_padding_alignment(&self, type_: &str) -> usize {
        if self.record_has_inline_data(type_)
            || self.union_is_data(type_)
            || (is_collection_type(type_) && self.type_is_flat(type_))
        {
            8
        } else {
            1
        }
    }

    /// Rounds the unsigned offset stored at `slot` up to `alignment`. A no-op
    /// for `alignment <= 1`. Uses temporary scratch vregs (colored by regalloc),
    /// so it does not disturb the surrounding collection-writer code's values.
    pub(super) fn emit_align_offset_slot(&mut self, slot: usize, alignment: usize) {
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
    pub(super) fn emit_align_offset_register(
        &mut self,
        reg: &str,
        alignment: usize,
        scratch: &str,
    ) {
        if alignment <= 1 {
            return;
        }
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
    pub(super) fn emit_copy_bytes(&mut self, dst: &str, src: &str, len: &str, prefix: &str) {
        let scratch13 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        let remaining = scratch13.as_str();
        self.emit(abi::move_register(remaining, len));
        self.emit_block_copy_advance(dst, src, remaining, &scratch14, prefix);
    }

    /// Word-then-byte block copy that advances `dst`, `src`, and consumes
    /// `remaining` (decremented to 0). `scratch` holds the in-flight word/byte
    /// and is clobbered. Shared by `emit_copy_bytes` and the collection
    /// entry/payload copy loops so every payload move is word-sized.
    pub(super) fn emit_block_copy_advance(
        &mut self,
        dst: &str,
        src: &str,
        remaining: &str,
        scratch: &str,
        prefix: &str,
    ) {
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
    pub(super) fn emit_block_copy_backward(
        &mut self,
        dst_end: &str,
        src_end: &str,
        remaining: &str,
        scratch: &str,
        prefix: &str,
    ) {
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
    pub(super) fn emit_flat_block_size(
        &mut self,
        type_: &str,
        ptr_reg: &str,
        out_reg: &str,
        scratch: &str,
    ) -> Result<(), String> {
        match type_ {
            "String" => {
                // byteLength(+0) + 8 (length word) + 1 (trailing NUL).
                self.emit(abi::load_u64(out_reg, ptr_reg, 0));
                self.emit(abi::add_immediate(out_reg, out_reg, 9));
                Ok(())
            }
            other if is_collection_type(other) => {
                // header + capacity * entryStride + dataCapacity (+ a map's
                // bucket region).
                //
                // The stride MUST match what the allocator reserved. This is the
                // size `arena_free` releases on scope drop, so a kind-0 stride on
                // a kind-2 block frees `capacity * 40` bytes past the end and
                // corrupts the free list — that is bug-02's exact failure mode,
                // and plan-57-D names this function as the one edit whose
                // mistake is heap corruption rather than a wrong value.
                let element = list_element_type(other).unwrap_or_default();
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
                let is_map = CollectionTypeLayout::from_type(other)
                    .is_some_and(|layout| layout.kind == COLLECTION_KIND_MAP);
                if is_map {
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
    pub(super) fn copy_flat_block(&mut self, type_: &str, source: &str) -> Result<String, String> {
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        // A collection value is copied **shrink-to-fit** (plan-01 §4.3): headroom
        // is a property of a mutable working buffer, never of a value, so a copy
        // drops any spare capacity. A whole-block `memcpy` would carry the
        // headroom (and the gap between the live entries and the data region)
        // into the snapshot; the tight copy compacts both.
        if is_collection_type(type_) {
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
        // `type_is_flat` value (plan-02 §4.1).
        self.emit_inlined_block_size_from_ptr_slot(type_, source_slot, size_slot)?;
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
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), source_slot));
        let dst_base = self.temporary_vreg();
        self.emit(abi::load_u64(&dst_base, abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), size_slot));
        self.emit_copy_bytes(&dst_base, &scratch9, &scratch10, "flat_copy");
        let result = self.allocate_register()?;
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
    pub(super) fn copy_collection_tight(
        &mut self,
        type_: &str,
        source: &str,
    ) -> Result<String, String> {
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
            &COLLECTION_ENTRY_SIZE.to_string(),
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
        // A map's tight copy reserves its (count-sized) hash bucket region; x9
        // still holds count. The copy is marked not-ready so the buckets are
        // recomputed on first probe (no stale offsets across copy/transfer).
        self.emit_reserve_map_buckets(
            layout.kind == COLLECTION_KIND_MAP,
            &scratch9,
            abi::return_register(),
            &scratch10,
        );
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
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch21, &scratch9, &scratch16));
        self.emit_block_copy_advance(
            &scratch17,
            &scratch20,
            &scratch21,
            &scratch22,
            "tight_copy_entries",
        );

        // Copy the data region verbatim (dataLength bytes). Source base is
        // capacity-based (it may have headroom); destination base is count-based
        // (tight) — both resolve through emit_collection_data_pointer.
        self.emit(abi::load_u64(
            &block_base,
            abi::stack_pointer(),
            result_slot,
        ));
        let element = list_element_type(type_).unwrap_or_default();
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

        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(result)
    }

    /// Built-in records that are constructed by bespoke runtime helpers (which
    /// still write their `String` fields as pointers) rather than the codegen
    /// `Constructor` path. They are excluded from the inline-`String` record
    /// layout so that machinery — and field reads of values it produces — stay on
    /// the pointer layout consistently (plan-02 Phase 2):
    ///   - `Error`/`ErrorLoc`: the fallible-call ABI, trap materialization, `FAIL`.
    ///   - `Address`/`Datagram`/`DatagramText`: the `net::` socket helpers
    ///     (`emit_address_from_sockaddr`, etc.).
    ///
    /// Every other record inlines its `String` fields.
    pub(super) fn is_pointer_string_record(&self, type_: &str) -> bool {
        matches!(
            type_,
            "Address" | "Datagram" | "DatagramText" | "AudioDevice"
        )
    }

    /// True when `field_type` occupies a record slot as a pointer to a separate
    /// allocation (nested record/union/collection/`Result`/`Error`). These stay
    /// pointers in Phase 2 (later phases inline them).
    pub(super) fn record_field_is_pointer(&self, field_type: &str) -> bool {
        is_collection_type(field_type)
            || self.type_model.record_fields.contains_key(field_type)
            || self.type_model.union_names.contains(field_type)
            || field_type.starts_with("Result OF ")
            || field_type == "Error"
    }

    /// The payload value types a collection stores: the element type for a
    /// `List`, the key and value types for a `Map`.
    fn collection_payload_types(&self, type_: &str) -> Vec<String> {
        if let Some(value) = type_.strip_prefix("List OF ") {
            vec![value.to_string()]
        } else if let Some((key, value)) = map_type_parts(type_) {
            vec![key, value]
        } else {
            Vec::new()
        }
    }

    /// True when a value of `type_` is **fully flat** — a single pointer-free
    /// block that a `memcpy` deep-copies. Flat types: scalars, `String`, a record
    /// whose every field is flat, a **data** union whose every variant is flat,
    /// and a collection whose payloads are flat **and not themselves collections**
    /// (nested collections are still pointers — plan-02 §4.4 pending). Not flat:
    /// resource unions/handles, `Result`, `Error`/`ErrorLoc` and the other
    /// helper-built pointer-`String` records, and any recursive type (broken by
    /// the `visited` path set, so a cyclic type stays a pointer).
    pub(super) fn type_is_flat(&self, type_: &str) -> bool {
        let mut visited = std::collections::HashSet::new();
        self.type_is_flat_inner(type_, &mut visited)
    }

    fn type_is_flat_inner(
        &self,
        type_: &str,
        visited: &mut std::collections::HashSet<String>,
    ) -> bool {
        if !visited.insert(type_.to_string()) {
            // Already on the current path: a type cycle. Cyclic values cannot be
            // a single finite flat block, so treat them as pointers.
            return false;
        }
        let result = if type_ == "String" {
            true
        } else if let Some(payload) = type_.strip_prefix("Result OF ") {
            // A flat `Result` `{tag, size, payload}` is pointer-free when its
            // success payload is flat (the `Err` variant is the now-flat `Error`).
            self.type_is_flat_inner(payload, visited)
        } else if is_collection_type(type_) {
            // A collection is flat when every payload is flat — including a nested
            // flat collection, which is inlined in the data region (plan-02 §4.4,
            // Phase 5a). A resource or recursive payload makes it non-flat.
            self.collection_payload_types(type_)
                .into_iter()
                .all(|p| self.type_is_flat_inner(&p, visited))
        } else if self.type_model.record_fields.contains_key(type_) {
            !self.is_pointer_string_record(type_)
                && self
                    .type_model
                    .record_fields
                    .get(type_)
                    .cloned()
                    .unwrap_or_default()
                    .iter()
                    .all(|(_, ft)| self.type_is_flat_inner(ft, visited))
        } else if self.union_is_data(type_) {
            self.type_model
                .variants_for_union(type_)
                .map(|variant| variant.to_string())
                .collect::<Vec<_>>()
                .iter()
                .all(|variant| self.type_is_flat_inner(variant, visited))
        } else if crate::builtins::is_resource_type(type_) {
            // A resource is a move-only handle to its single instance, never a
            // copyable flat block.
            false
        } else {
            // A scalar (anything that is not a pointer composite, `String`, or
            // resource) is flat; resource unions / `Result` are excluded above.
            !self.record_field_is_pointer(type_)
        };
        visited.remove(type_);
        result
    }

    /// True when field `field_type` of `record_type` is inlined into the record's
    /// trailing data region (the slot holds a block-relative offset): an inlined
    /// `String`, or a fully-flat composite — a nested record, a flat data union,
    /// or a flat collection (plan-02 §4.2–§4.4). Scalars stay inline in the slot;
    /// not-yet-flat composites stay pointers.
    pub(super) fn record_field_is_inlined(&self, record_type: &str, field_type: &str) -> bool {
        if self.is_pointer_string_record(record_type) {
            return false;
        }
        if field_type == "String" {
            return true;
        }
        let is_composite = self.type_model.record_fields.contains_key(field_type)
            || self.type_model.union_names.contains(field_type)
            || is_collection_type(field_type)
            || field_type.starts_with("Result OF ");
        is_composite && self.type_is_flat(field_type)
    }

    /// True when `record_type` has at least one inlined field (so its block is
    /// variable-length and carries a trailing data region).
    pub(super) fn record_has_inline_data(&self, record_type: &str) -> bool {
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
    pub(super) fn emit_inlined_block_size_from_ptr_slot(
        &mut self,
        field_type: &str,
        ptr_slot: usize,
        out_slot: usize,
    ) -> Result<(), String> {
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        if field_type == "String" {
            self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), ptr_slot));
            self.emit(abi::load_u64(&scratch9, &scratch8, 0));
            self.emit(abi::add_immediate(&scratch9, &scratch9, 9));
            self.emit(abi::store_u64(&scratch9, abi::stack_pointer(), out_slot));
            Ok(())
        } else if self.type_model.record_fields.contains_key(field_type) {
            self.emit_record_block_size_to_slot(field_type, ptr_slot, out_slot)
        } else if self.union_is_data(field_type) || field_type.starts_with("Result OF ") {
            // A data union and a flat `Result` are self-describing: their `size`
            // word lives at +8 (plan-02 §4.3).
            self.emit_data_union_size_to_slot(ptr_slot, out_slot);
            Ok(())
        } else if is_collection_type(field_type) {
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
    pub(super) fn union_is_data(&self, type_: &str) -> bool {
        if !self.type_model.union_names.contains(type_) {
            return false;
        }
        let mut saw_variant = false;
        for variant in self.type_model.variants_for_union(type_) {
            saw_variant = true;
            if crate::builtins::is_resource_type(variant) {
                return false;
            }
        }
        saw_variant
    }

    /// Total byte size of a data union into `out_slot`: the `size` word at `+8`
    /// (plan-02 §4.3). `ptr_slot` holds the union pointer. Clobbers a scratch vreg.
    pub(super) fn emit_data_union_size_to_slot(&mut self, ptr_slot: usize, out_slot: usize) {
        let scratch8 = self.temporary_vreg();
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), ptr_slot));
        self.emit(abi::load_u64(&scratch8, &scratch8, 8));
        self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), out_slot));
    }

    /// Wrap a built variant record (pointer in `record_ptr_slot`) into a data
    /// union value `{U64 tag@0, U64 size@8, variant-record-block@16}` (plan-02
    /// §4.3): the variant's flat record block is inlined at `+16`. Returns a
    /// register holding the union pointer.
    pub(super) fn emit_wrap_record_in_union(
        &mut self,
        member_type: &str,
        tag: usize,
        record_ptr_slot: usize,
    ) -> Result<String, String> {
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
        // tag@0, size@8.
        self.emit(abi::move_immediate(&scratch9, "UnionTag", &tag.to_string()));
        self.emit(abi::store_u64(&scratch9, abi::RET[1], 0));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), size_slot));
        self.emit(abi::store_u64(&scratch9, abi::RET[1], 8));
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
        let register = self.allocate_register()?;
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
    pub(super) fn emit_record_block_size_to_slot(
        &mut self,
        record_type: &str,
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
        for (_, field_type) in &fields {
            if !self.record_field_is_inlined(record_type, field_type) {
                continue;
            }
            self.emit_align_offset_slot(out_slot, 8);
            // inner_base = base + current offset (where this sub-block begins).
            let inner_base_slot = self.allocate_stack_object("record_size_inner_base", 8);
            self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), base_slot));
            self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), out_slot));
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
            self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), out_slot));
            self.emit(abi::load_u64(
                &scratch8,
                abi::stack_pointer(),
                inner_size_slot,
            ));
            self.emit(abi::add_registers(&scratch9, &scratch9, &scratch8));
            self.emit(abi::store_u64(&scratch9, abi::stack_pointer(), out_slot));
        }
        Ok(())
    }

    /// Build a flat record of `record_type` from `field_slots` (one stack slot
    /// per field, in field order). A `String` field slot holds a pointer to a
    /// source `String` block (its bytes are inlined into the record's data
    /// region and the slot stores the block-relative offset); every other field
    /// slot holds the scalar value or pointer, stored inline at `8*index`.
    /// Returns a register holding the new record pointer. plan-02 §4.2.
    pub(super) fn emit_build_inlined_record(
        &mut self,
        record_type: &str,
        field_slots: &[usize],
    ) -> Result<String, String> {
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
        let register = self.allocate_register()?;
        self.emit(abi::load_u64(&register, abi::stack_pointer(), result_slot));
        Ok(register)
    }

    pub(super) fn materialize_inline_value_in_arena(
        &mut self,
        type_: &str,
        source: &str,
    ) -> Result<String, String> {
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
            self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), source_slot));
            let dst_base = self.temporary_vreg();
            self.emit(abi::load_u64(&dst_base, abi::stack_pointer(), result_slot));
            self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), size_slot));
            self.emit_copy_bytes(&dst_base, &scratch9, &scratch10, "inline_value_block_copy");
            let result = self.allocate_register()?;
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
        self.emit(abi::move_immediate(
            abi::return_register(),
            "Integer",
            &size.to_string(),
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
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), source_slot));
        self.emit(abi::move_immediate(
            &scratch13,
            "Integer",
            &size.to_string(),
        ));
        self.emit_copy_bytes(
            abi::RET[1],
            &scratch9,
            &scratch13,
            "inline_value_arena_copy",
        );
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(result)
    }

    pub(super) fn lower_len(&mut self, value: &NirValue) -> Result<ValueResult, String> {
        let value = self.lower_value(value)?;
        if value.type_ == "String" {
            let count_slot = self.allocate_stack_object("len_string_count", 8);
            let remaining = self.allocate_register()?;
            let cursor = self.allocate_register()?;
            let byte = self.allocate_register()?;
            let mask = self.allocate_register()?;
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
            let register = self.allocate_register()?;
            self.emit(abi::load_u64(&register, abi::stack_pointer(), count_slot));
            Ok(ValueResult {
                type_: "Integer".to_string(),
                location: register,
                text: format!("len({})", value.text),
            })
        } else if is_collection_type(&value.type_) {
            let register = self.allocate_register()?;
            self.emit(abi::load_u64(
                &register,
                &value.location,
                COLLECTION_OFFSET_COUNT,
            ));
            Ok(ValueResult {
                type_: "Integer".to_string(),
                location: register,
                text: format!("len({})", value.text),
            })
        } else {
            Err(format!(
                "native len does not accept argument type '{}'",
                value.type_
            ))
        }
    }

    pub(super) fn lower_empty_collection(&mut self, type_: &str) -> Result<ValueResult, String> {
        self.lower_collection_values(type_, Vec::new(), "empty collection")
    }

    pub(super) fn lower_list_literal(
        &mut self,
        type_: &str,
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
                    type_: value.type_,
                },
            });
        }
        self.lower_collection_values(type_, slots, "list")
    }

    pub(super) fn lower_map_literal(
        &mut self,
        type_: &str,
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
                    type_: key.type_,
                }),
                value: PayloadSlot {
                    slot: value_slot,
                    type_: value.type_,
                },
            });
        }
        self.lower_collection_values(type_, slots, "map")
    }

    pub(super) fn lower_collection_values(
        &mut self,
        type_: &str,
        slots: Vec<CollectionValueSlot>,
        label: &str,
    ) -> Result<ValueResult, String> {
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        self.reset_temporary_registers();
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
        // A map reserves a `2*capacity` u64 bucket array past the data region;
        // capacity == count for a literal, so fold it into the constant.
        let bucket_bytes = if layout.kind == COLLECTION_KIND_MAP {
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
        self.emit(abi::move_immediate(abi::ARG[1], "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(
            abi::RET[1],
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
        let register = self.allocate_register()?;
        self.emit(abi::load_u64(
            &register,
            abi::stack_pointer(),
            collection_slot,
        ));
        Ok(ValueResult {
            type_: type_.to_string(),
            location: register,
            text: format!("{label} {type_}"),
        })
    }

    pub(super) fn emit_write_collection_header(
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
            abi::RET[1],
            COLLECTION_OFFSET_KIND,
        ));
        self.emit(abi::move_immediate(
            &scratch8,
            "Byte",
            &layout.key_type_code.to_string(),
        ));
        self.emit(abi::store_u8(
            &scratch8,
            abi::RET[1],
            COLLECTION_OFFSET_KEY_TYPE,
        ));
        self.emit(abi::move_immediate(
            &scratch8,
            "Byte",
            &layout.value_type_code.to_string(),
        ));
        self.emit(abi::store_u8(
            &scratch8,
            abi::RET[1],
            COLLECTION_OFFSET_VALUE_TYPE,
        ));
        self.emit(abi::move_immediate(&scratch8, "Byte", "1"));
        self.emit(abi::store_u8(
            &scratch8,
            abi::RET[1],
            COLLECTION_OFFSET_FLAGS_VERSION,
        ));
        // Map hash index built lazily on first probe (no-op field for lists).
        self.emit(abi::move_immediate(&scratch8, "Byte", "0"));
        self.emit(abi::store_u8(
            &scratch8,
            abi::RET[1],
            COLLECTION_OFFSET_BUCKETS_READY,
        ));
        self.emit(abi::move_immediate(
            &scratch8,
            "Integer",
            &count.to_string(),
        ));
        self.emit(abi::store_u64(
            &scratch8,
            abi::RET[1],
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::store_u64(
            &scratch8,
            abi::RET[1],
            COLLECTION_OFFSET_CAPACITY,
        ));
        self.emit(abi::load_u64(
            &scratch8,
            abi::stack_pointer(),
            data_len_slot,
        ));
        self.emit(abi::store_u64(
            &scratch8,
            abi::RET[1],
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::store_u64(
            &scratch8,
            abi::RET[1],
            COLLECTION_OFFSET_DATA_CAPACITY,
        ));
    }

    pub(super) fn emit_write_collection_entry(
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
        let collection_register = scratch8.as_str();
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
            self.emit_copy_payload_to_collection(
                collection_slot,
                key_len_slot,
                slot.key.as_ref().unwrap(),
                data_offset_slot,
                if slot.key.is_some() { "" } else { &slot.value.type_ },
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
        self.emit_copy_payload_to_collection(
            collection_slot,
            value_len_slot,
            &slot.value,
            data_offset_slot,
            if slot.key.is_some() { "" } else { &slot.value.type_ },
        )?;
        Ok(())
    }

    pub(super) fn emit_add_payload_length(
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

    pub(super) fn emit_payload_length_to_stack(
        &mut self,
        payload: &PayloadSlot,
        label: &str,
    ) -> Result<usize, String> {
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let len_slot = self.allocate_stack_object(label, 8);
        match payload.type_.as_str() {
            "Boolean" | "Byte" => {
                self.emit(abi::move_immediate(&scratch8, "Integer", "1"));
            }
            "Scalar" => {
                self.emit(abi::move_immediate(&scratch8, "Integer", "4"));
            }
            "Integer" | "Float" | "Fixed" | "Money" => {
                self.emit(abi::move_immediate(&scratch8, "Integer", "8"));
            }
            // A function value is a single 8-byte closure pointer, stored by
            // reference exactly like a pointer payload (bug-73).
            other if is_function_type(other) => {
                self.emit(abi::move_immediate(&scratch8, "Integer", "8"));
            }
            "String" => {
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
            other if is_collection_type(other) => {
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
    pub(super) fn emit_copy_payload_to_collection(
        &mut self,
        collection_slot: usize,
        len_slot: usize,
        payload: &PayloadSlot,
        data_offset_slot: usize,
        stride_type: &str,
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

        match payload.type_.as_str() {
            "Boolean" | "Byte" => {
                self.emit(abi::load_u64(
                    &scratch12,
                    abi::stack_pointer(),
                    payload.slot,
                ));
                self.emit(abi::store_u8(&scratch12, &scratch10, 0));
            }
            "Scalar" => {
                self.emit(abi::load_u64(
                    &scratch12,
                    abi::stack_pointer(),
                    payload.slot,
                ));
                self.emit(abi::store_u32(&scratch12, &scratch10, 0));
            }
            "Integer" | "Float" | "Fixed" | "Money" => {
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
            other if is_function_type(other) => {
                self.emit(abi::load_u64(
                    &scratch12,
                    abi::stack_pointer(),
                    payload.slot,
                ));
                self.emit(abi::store_u64(&scratch12, &scratch10, 0));
            }
            "String" => {
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
                    || is_collection_type(other) =>
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
    pub(super) fn emit_reserve_map_buckets(
        &mut self,
        is_map: bool,
        capacity_reg: &str,
        size_reg: &str,
        scratch_reg: &str,
    ) {
        if !is_map {
            return;
        }
        // 2 * capacity buckets * 8 bytes = capacity << 4.
        self.emit(abi::shift_left_immediate(scratch_reg, capacity_reg, 4));
        self.emit(abi::add_registers(size_reg, size_reg, scratch_reg));
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
    pub(super) fn emit_element_value_offset(
        &mut self,
        dst_offset: &str,
        dst_length: &str,
        list: &str,
        index: &str,
        scratch_offset: &str,
        scratch_entry: &str,
        element_type: &str,
    ) {
        // kind 2: element `i` lives at `i * payloadSize` with a fixed length and
        // no entry to load (plan-57-D). Two instructions instead of six, and two
        // dependent loads removed from every indexed read.
        if let Some(payload) = kind2_payload_size(element_type) {
            self.emit(abi::move_immediate(
                dst_length,
                "Integer",
                &payload.to_string(),
            ));
            self.emit(abi::multiply_registers(dst_offset, index, dst_length));
            return;
        }
        self.emit(abi::move_immediate(
            scratch_offset,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(
            scratch_offset,
            index,
            scratch_offset,
        ));
        self.emit(abi::add_immediate(
            scratch_entry,
            list,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(
            scratch_entry,
            scratch_entry,
            scratch_offset,
        ));
        self.emit(abi::load_u64(
            dst_offset,
            scratch_entry,
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
    pub(super) fn emit_collection_data_pointer_for(
        &mut self,
        dst: &str,
        collection: &str,
        element_type: &str,
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
    fn emit_collection_data_pointer(&mut self, dst: &str, collection: &str) {
        // Scratch as vregs. Pinning these collides with the x86-64 ABI argument
        // registers and yields garbage element addresses.
        let capacity_v = self.temporary_vreg();
        let entry_size_v = self.temporary_vreg();
        let mut out = Vec::new();
        push_collection_data_pointer_into(
            &mut out,
            dst,
            collection,
            capacity_v.as_str(),
            entry_size_v.as_str(),
        );
        for instruction in out {
            self.emit(instruction);
        }
    }

    /// Load a list element's payload. The data base uses `type_`'s entry
    /// stride, which is correct only because this is a LIST block; a map must
    /// call [`Self::emit_load_map_payload`].
    pub(super) fn emit_load_collection_payload(
        &mut self,
        type_: &str,
        collection: &str,
        offset: &str,
        length: &str,
    ) -> Result<String, String> {
        self.emit_load_payload_with_stride(type_, type_, collection, offset, length)
    }

    /// Load a payload out of a MAP block. Identical to
    /// [`Self::emit_load_collection_payload`] except that the data base always
    /// uses the kind-0 stride: a map keeps its lookup table whatever its key and
    /// value types are, so selecting the entry-free base from a fixed-width key
    /// or value type would address it past its own entry array (plan-57-D).
    pub(super) fn emit_load_map_payload(
        &mut self,
        type_: &str,
        collection: &str,
        offset: &str,
        length: &str,
    ) -> Result<String, String> {
        self.emit_load_payload_with_stride(type_, "", collection, offset, length)
    }

    fn emit_load_payload_with_stride(
        &mut self,
        type_: &str,
        stride_type: &str,
        collection: &str,
        offset: &str,
        length: &str,
    ) -> Result<String, String> {
        // Inputs held in vregs, never in registers that are x86-64 ABI argument
        // registers on one backend and free scratch on another.
        let collection_input_v = self.temporary_vreg();
        let offset_input_v = self.temporary_vreg();
        let length_input_v = self.temporary_vreg();
        let collection_input = collection_input_v.as_str();
        let offset_input = offset_input_v.as_str();
        let length_input = length_input_v.as_str();
        self.emit(abi::move_register(collection_input, collection));
        self.emit(abi::move_register(offset_input, offset));
        self.emit(abi::move_register(length_input, length));
        let data = self.allocate_register()?;
        self.emit_collection_data_pointer_for(&data, collection_input, stride_type);
        self.emit(abi::add_registers(&data, &data, offset_input));
        match type_ {
            "Boolean" | "Byte" => {
                let result = self.allocate_register()?;
                self.emit(abi::load_u8(&result, &data, 0));
                Ok(result)
            }
            "Scalar" => {
                let result = self.allocate_register()?;
                self.emit(abi::load_u32(&result, &data, 0));
                Ok(result)
            }
            "Integer" | "Float" | "Fixed" | "Money" => {
                let result = self.allocate_register()?;
                self.emit(abi::load_u64(&result, &data, 0));
                Ok(result)
            }
            // A function value reads back its 8-byte closure pointer; the closure
            // object stays shared (reference semantics, bug-73).
            other if is_function_type(other) => {
                let result = self.allocate_register()?;
                self.emit(abi::load_u64(&result, &data, 0));
                Ok(result)
            }
            "String" => self.emit_materialize_string_from_bytes(&data, length_input),
            other if self.is_pointer_collection_payload_type(other) => {
                let result = self.allocate_register()?;
                self.emit(abi::load_u64(&result, &data, 0));
                Ok(result)
            }
            // An inlined record/union slot block or a flat nested collection block
            // is read as a borrow pointer to the block within the data region
            // (plan-02 §4.2–§4.4). Its own offsets are relative to that base.
            other if self.inline_collection_payload_size(other).is_some() => Ok(data),
            other if is_collection_type(other) => Ok(data),
            other => Err(format!(
                "native collection packed payload does not support type '{other}'"
            )),
        }
    }

    /// Copy an existing heap `String` value (a pointer to `[u64 len][bytes][nul]`)
    /// into a fresh owned arena string. `getOr`'s found path materializes its
    /// `String` result fresh (`emit_load_collection_payload`), so `getOr`'s
    /// default path must copy the borrowed default the same way — otherwise the
    /// owned-result contract (`materialize_owned_element` frees the result at
    /// scope end, but deliberately skips `String` assuming it is already fresh)
    /// double-frees the caller's default and corrupts the arena free-list, which
    /// only surfaces as a trap on a *later* allocation. See [[scope-drop-frees]].
    pub(super) fn emit_copy_owned_string(&mut self, source_ptr: &str) -> Result<String, String> {
        let length = self.allocate_register()?;
        self.emit(abi::load_u64(&length, source_ptr, 0));
        let bytes = self.allocate_register()?;
        self.emit(abi::add_immediate(&bytes, source_ptr, 8));
        self.emit_materialize_string_from_bytes(&bytes, &length)
    }

    pub(super) fn emit_materialize_string_from_bytes(
        &mut self,
        source: &str,
        length: &str,
    ) -> Result<String, String> {
        let scratch12 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        let scratch15 = self.temporary_vreg();
        let source_slot = self.allocate_stack_object("collection_string_source", 8);
        let length_slot = self.allocate_stack_object("collection_string_length", 8);
        let result_slot = self.allocate_stack_object("collection_string_result", 8);
        let alloc_ok = self.label("collection_string_alloc_ok");
        let copy_loop = self.label("collection_string_copy_loop");
        let copy_done = self.label("collection_string_copy_done");

        self.emit(abi::store_u64(source, abi::stack_pointer(), source_slot));
        self.emit(abi::store_u64(length, abi::stack_pointer(), length_slot));
        self.emit(abi::add_immediate(abi::return_register(), length, 9));
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
        self.emit(abi::load_u64(&scratch12, abi::stack_pointer(), length_slot));
        self.emit(abi::store_u64(&scratch12, abi::RET[1], 0));
        self.emit(abi::add_immediate(&scratch13, abi::RET[1], 8));
        self.emit(abi::load_u64(&scratch14, abi::stack_pointer(), source_slot));
        self.emit(abi::label(&copy_loop));
        self.emit(abi::compare_immediate(&scratch12, "0"));
        self.emit(abi::branch_eq(&copy_done));
        self.emit(abi::load_u8(&scratch15, &scratch14, 0));
        self.emit(abi::store_u8(&scratch15, &scratch13, 0));
        self.emit(abi::add_immediate(&scratch14, &scratch14, 1));
        self.emit(abi::add_immediate(&scratch13, &scratch13, 1));
        self.emit(abi::subtract_immediate(&scratch12, &scratch12, 1));
        self.emit(abi::branch(&copy_loop));
        self.emit(abi::label(&copy_done));
        self.emit(abi::move_immediate(&scratch15, "Integer", "0"));
        self.emit(abi::store_u8(&scratch15, &scratch13, 0));
        let result = self.allocate_register()?;
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
/// Deliberately excludes function values and pointer payloads: both are 8-byte
/// fixed, but they carry ownership that the drop and thread-transfer paths
/// reason about per entry.
///
/// Must agree with `CodeBuilder::collection_payload_alignment` for every arm;
/// `fixed_width_agrees_with_payload_alignment` asserts that so the two cannot
/// drift.
pub(super) fn list_element_is_fixed_width(element_type: &str) -> Option<usize> {
    match element_type {
        "Boolean" | "Byte" => Some(1),
        "Scalar" => Some(4),
        "Integer" | "Float" | "Fixed" | "Money" => Some(8),
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
pub(super) fn push_collection_data_pointer_into(
    out: &mut Vec<CodeInstruction>,
    dst: &str,
    collection: &str,
    scratch_capacity: &str,
    scratch_entry_size: &str,
) {
    out.push(abi::move_register(scratch_capacity, collection));
    out.push(abi::add_immediate(dst, collection, COLLECTION_HEADER_SIZE));
    out.push(abi::load_u64(
        scratch_capacity,
        scratch_capacity,
        COLLECTION_OFFSET_CAPACITY,
    ));
    out.push(abi::move_immediate(
        scratch_entry_size,
        "Integer",
        &COLLECTION_ENTRY_SIZE.to_string(),
    ));
    out.push(abi::multiply_registers(
        scratch_capacity,
        scratch_capacity,
        scratch_entry_size,
    ));
    out.push(abi::add_registers(dst, dst, scratch_capacity));
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
pub(super) fn push_collection_data_base_from_capacity(
    out: &mut Vec<CodeInstruction>,
    dst: &str,
    collection: &str,
    scratch_capacity: &str,
    scratch_entry_size: &str,
    scratch_product: &str,
) {
    out.push(abi::load_u64(
        scratch_capacity,
        collection,
        COLLECTION_OFFSET_CAPACITY,
    ));
    out.push(abi::move_immediate(
        scratch_entry_size,
        "Integer",
        &COLLECTION_ENTRY_SIZE.to_string(),
    ));
    out.push(abi::multiply_registers(
        scratch_product,
        scratch_capacity,
        scratch_entry_size,
    ));
    out.push(abi::add_immediate(
        scratch_product,
        scratch_product,
        COLLECTION_HEADER_SIZE,
    ));
    out.push(abi::add_registers(dst, collection, scratch_product));
}

/// Whether the kind-2 (entry-free) representation is live.
///
/// The representation cannot be adopted piecemeal: the allocation size, the free
/// size, the data base, element access and iteration must all agree, or a block
/// is allocated at one layout and read at another. So the plumbing is threaded
/// through every site first with this `false`, which keeps every formula at its
/// current value and lets `artifact-gate` prove the threading changed nothing —
/// and the representation is switched on in one commit once every site consults
/// these two functions.
fn kind2_enabled() -> bool {
    // Env-gated during development so ONE binary can be exercised both ways:
    // build the whole acceptance suite with and without `MFB_KIND2=1` and diff
    // the behavior. That is the same negative-control lever that proved
    // `list-order-invariant-rt` non-vacuous. It becomes a plain `true` once the
    // representation is complete and the goldens are re-baselined.
    std::env::var("MFB_KIND2").is_ok()
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
pub(super) fn list_entry_stride(element_type: &str) -> usize {
    if kind2_enabled() && list_element_is_fixed_width(element_type).is_some() {
        0
    } else {
        COLLECTION_ENTRY_SIZE
    }
}

/// The `kind` byte for a list of `element_type`.
pub(super) fn list_block_kind(element_type: &str) -> usize {
    if kind2_enabled() && list_element_is_fixed_width(element_type).is_some() {
        COLLECTION_KIND_LIST_FIXED
    } else {
        COLLECTION_KIND_LIST
    }
}

/// The payload size of `element_type` when it uses the entry-free
/// representation, or `None` when it keeps a lookup table.
pub(super) fn kind2_payload_size(element_type: &str) -> Option<usize> {
    if kind2_enabled() {
        list_element_is_fixed_width(element_type)
    } else {
        None
    }
}

/// The lookup-entry stride for a `List OF Byte`, for the runtime helpers that
/// build or read one and know their element type statically. Zero once the
/// entry-free representation is live (plan-57-D).
pub(super) fn byte_list_entry_stride() -> usize {
    list_entry_stride("Byte")
}
