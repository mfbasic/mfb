//! plan-176-Q: removing a list element, by copy or in place. The shared grow/repack helpers
//! live in `collection/list/list_mutate/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    pub(crate) fn lower_list_remove_at(
        &mut self,
        base_slot: usize,
        index_slot: usize,
        list_type: &ParameterType,
        element_type: &ParameterType,
    ) -> Result<ValueResult, String> {
        // Zero for a kind-2 list: the sizing arithmetic below then reserves no
        // entry array, and the formulas keep their shape (plan-57-D).
        let entry_stride = list_entry_stride(element_type);
        let scratch20 = self.temporary_vreg();
        let scratch21 = self.temporary_vreg();
        let scratch22 = self.temporary_vreg();
        let scratch23 = self.temporary_vreg();
        let scratch24 = self.temporary_vreg();
        let scratch25 = self.temporary_vreg();
        let scratch8 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch16 = self.temporary_vreg();
        let scratch17 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        let scratch15 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let layout = CollectionTypeLayout::from_type(list_type)
            .ok_or_else(|| format!("native code collection type '{list_type}' is not supported"))?;
        for register in [
            scratch20, scratch21, scratch22, scratch23, scratch24, scratch25,
        ] {
            self.mark_register_used(&register.render());
        }
        let result_slot = self.allocate_stack_object("list_remove_result", 8);
        let data_len_slot = self.allocate_stack_object("list_remove_data_len", 8);
        let valid_start = self.label("list_remove_valid_start");
        let alloc_ok = self.label("list_remove_alloc_ok");
        let invalid = self.label("list_remove_invalid");
        let done = self.label("list_remove_done");

        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), base_slot));
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), index_slot));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch8,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::compare_immediate(&scratch10, "0"));
        self.emit(abi::branch_ge(&valid_start));
        self.emit(abi::branch(&invalid));
        self.emit(abi::label(&valid_start));
        self.emit(abi::compare_registers(&scratch10, &scratch11));
        self.emit(abi::branch_ge(&invalid));
        self.emit(abi::move_immediate(
            &scratch16,
            "Integer",
            &entry_stride.to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch17, &scratch10, &scratch16));
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch8,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(&scratch12, &scratch12, &scratch17));
        self.emit(abi::load_u64(
            &scratch14,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        if let Some(payload) = kind2_payload_size(element_type) {
            // kind 2: the hole is exactly one payload wide and there is no entry
            // to read its length from (plan-57-D).
            self.emit(abi::move_immediate(
                &scratch15,
                "Integer",
                &payload.to_string(),
            ));
            self.emit(abi::subtract_registers(&scratch15, &scratch14, &scratch15));
        } else {
            self.emit(abi::load_u64(
                &scratch15,
                &scratch12,
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ));
            self.emit(abi::subtract_registers(&scratch15, &scratch14, &scratch15));
        }
        // `arena_alloc` clobbers x15 in its block-grow path; persist the data
        // length so the header write below does not store a stale pointer.
        self.emit(abi::store_u64(
            &scratch15,
            abi::stack_pointer(),
            data_len_slot,
        ));
        self.emit(abi::subtract_immediate(&scratch13, &scratch11, 1));
        let size_overflow = self.label("list_remove_at_size_overflow");
        // Checked collection-size arithmetic (bug-147.7): (count-1) and dataLength
        // are runtime-derived, so guard count*ENTRY + HEADER + dataLen against
        // overflow.
        self.emit_checked_size_multiply(&scratch17, &scratch13, &scratch16, &size_overflow);
        self.emit_checked_size_add_immediate(
            abi::return_register(),
            &scratch17,
            COLLECTION_HEADER_SIZE,
            &size_overflow,
        );
        self.emit_checked_size_add(
            abi::return_register(),
            abi::return_register(),
            &scratch15,
            &size_overflow,
        );
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&alloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&size_overflow));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            result_slot,
        ));
        let nb = self.temporary_vreg();
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64(
            &scratch15,
            abi::stack_pointer(),
            data_len_slot,
        ));
        self.emit_write_list_header_from_registers(&layout, &nb, &scratch13, &scratch15);

        // Copy phase (no allocations, so registers hold across it). removeAt
        // punches a single contiguous hole in the data region — the removed
        // payload `[holeOffset, holeOffset + holeLen)`. Everything before it stays;
        // everything after shifts left by `holeLen`. So the payloads move as two
        // verbatim block copies (before-hole, after-hole) whatever order the data
        // region is in, the entry table copies as two verbatim spans (prefix
        // `[0..index)`, suffix `[index+1..count)`), and a single cheap pass fixes
        // each surviving `valueOffset` that sat past the hole — no per-payload copy
        // (plan-25-B B2). Testing each entry's own offset keeps it correct for a
        // list whose data is out of entry order after an insert/prepend/set.
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), base_slot)); // source
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), index_slot)); // index
        self.emit(abi::move_immediate(
            &scratch16,
            "Integer",
            &entry_stride.to_string(),
        ));
        // removed entry = source + HEADER + index*ENTRY; grab the hole span.
        self.emit(abi::multiply_registers(&scratch11, &scratch10, &scratch16));
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch8,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(&scratch12, &scratch12, &scratch11));
        // scratch23 = holeOffset (removedValueOffset); scratch14 = holeLen.
        // kind 2 derives both from the index — element `i` is at `i * payload`
        // with a constant length, so there is nothing to load.
        let kind2_payload = kind2_payload_size(element_type);
        if let Some(payload) = kind2_payload {
            self.emit(abi::move_immediate(
                &scratch23,
                "Integer",
                &payload.to_string(),
            ));
            self.emit(abi::multiply_registers(&scratch23, &scratch10, &scratch23));
        } else {
            self.emit(abi::load_u64(
                &scratch23,
                &scratch12,
                COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
            ));
        }
        if let Some(payload) = kind2_payload {
            self.emit(abi::move_immediate(
                &scratch14,
                "Integer",
                &payload.to_string(),
            ));
        } else {
            self.emit(abi::load_u64(
                &scratch14,
                &scratch12,
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ));
        }

        // --- Entry table: two verbatim spans (no per-copy offset shift). ---
        // Skipped entirely for kind 2: there is no entry table to copy, and no
        // surviving `valueOffset` to fix up, because element `i` is addressed
        // from its index (plan-57-D).
        if kind2_payload.is_none() {
            self.emit(abi::add_immediate(
                &scratch12,
                &scratch8,
                COLLECTION_HEADER_SIZE,
            )); // src.entry[0]
            self.emit(abi::load_u64(&nb, abi::stack_pointer(), result_slot));
            self.emit(abi::add_immediate(&scratch17, &nb, COLLECTION_HEADER_SIZE)); // dst.entry[0]
                                                                                    // Prefix [0..index): index*ENTRY bytes. Advances scratch17 -> dst.entry[index]
                                                                                    // and scratch12 -> src.entry[index] (the removed entry).
            self.emit(abi::multiply_registers(&scratch15, &scratch10, &scratch16));
            self.emit_block_copy_advance(
                &scratch17,
                &scratch12,
                &scratch15,
                &scratch22,
                "list_remove_prefix_e",
            );
            // Suffix [index+1..count): skip the removed src entry, copy suffixCount*ENTRY.
            self.emit(abi::add_immediate(&scratch12, &scratch12, entry_stride)); // src.entry[index+1]
            self.emit(abi::load_u64(
                &scratch11,
                &scratch8,
                COLLECTION_OFFSET_COUNT,
            ));
            self.emit(abi::subtract_registers(&scratch11, &scratch11, &scratch10));
            self.emit(abi::subtract_immediate(&scratch11, &scratch11, 1)); // suffixCount
            self.emit(abi::multiply_registers(&scratch15, &scratch11, &scratch16));
            self.emit_block_copy_advance(
                &scratch17,
                &scratch12,
                &scratch15,
                &scratch22,
                "list_remove_suffix_e",
            );
        }

        // --- Data region: two verbatim blocks around the hole. ---
        self.emit_collection_data_pointer_for(&scratch20, &scratch8, &element_type); // src data base (capacity-based)
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), result_slot));
        self.emit_collection_data_pointer_for(&scratch21, &nb, &element_type); // dst data base (tight)
                                                                               // Before-hole [0, holeOffset): advances scratch21 -> dst.data[holeOffset]
                                                                               // and scratch20 -> src.data[holeOffset].
        self.emit(abi::move_register(&scratch15, &scratch23)); // holeOffset (copy consumes it)
        self.emit_block_copy_advance(
            &scratch21,
            &scratch20,
            &scratch15,
            &scratch22,
            "list_remove_prefix_d",
        );
        // After-hole [holeOffset+holeLen, dataLength) -> dst.data[holeOffset]. Skip
        // the removed span in the source, then copy the tail.
        self.emit(abi::add_registers(&scratch20, &scratch20, &scratch14));
        self.emit(abi::load_u64(
            &scratch15,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::subtract_registers(&scratch15, &scratch15, &scratch23));
        self.emit(abi::subtract_registers(&scratch15, &scratch15, &scratch14)); // tailLen
        self.emit_block_copy_advance(
            &scratch21,
            &scratch20,
            &scratch15,
            &scratch22,
            "list_remove_suffix_d",
        );

        // --- Fix up each surviving entry whose payload sat past the hole. ---
        if kind2_payload.is_none() {
            self.emit(abi::load_u64(&nb, abi::stack_pointer(), result_slot));
            self.emit(abi::add_immediate(&scratch17, &nb, COLLECTION_HEADER_SIZE)); // dst.entry[0]
            self.emit(abi::load_u64(
                &scratch11,
                &scratch8,
                COLLECTION_OFFSET_COUNT,
            ));
            self.emit(abi::subtract_immediate(&scratch11, &scratch11, 1)); // survivor count
            self.emit_offset_compaction_fixup(
                &scratch17,
                &scratch11,
                &scratch23,
                &scratch14,
                "list_remove_fix",
            );
        }
        self.emit(abi::branch(&done));
        self.emit(abi::label(&invalid));
        self.raise_error("collections.removeAt", "ErrIndexOutOfRange")?;
        self.emit(abi::label(&done));

        let result = self.allocate_register();
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            origin: None,
            type_: list_type.clone(),
            location: Operand::from(result),
            text: format!("removeAt({list_type}, Integer) over {element_type}"),
        })
    }

    /// Remove the element at `index_slot` from the list whose buffer pointer
    /// lives in `buffer_slot`, **mutating that buffer in place** (plan-121-B):
    /// close the hole the removed element leaves and decrement `count` /
    /// `dataLength`. No allocation, no copy of the surviving elements into a
    /// fresh block — the freed capacity simply becomes headroom a later `append`
    /// reuses.
    ///
    /// The out-of-place sibling [`Self::lower_list_remove_at`] does the same
    /// arithmetic into a freshly allocated tight buffer. That allocate + copy +
    /// free is the whole cost: at N = 6400 a `List OF Integer` is 51 KB, a
    /// `memmove` of it is ~2 us, and `removeAt` measured 72 us — 36x the data
    /// movement (spike 3). The shift itself is the operation's defined cost and C
    /// pays it too; this deletes everything around it.
    ///
    /// Two representations, and they need different work:
    ///
    /// * **Fixed-width elements** (`list_entry_stride == 0`) have no entry table
    ///   at all — element `i` lives at `i * payload` (plan-57-D). Removing `i` is
    ///   one forward shift of `[(i+1)*p, count*p)` down by `p`. Nothing else
    ///   changes: the surviving element that was at `k > i` lands at `(k-1)*p`,
    ///   which is exactly where index `k-1` is read from.
    /// * **Variable-width elements** keep a lookup table over a data region. The
    ///   entry table shifts down one stride and nothing else moves (bug-732): the
    ///   removed payload's bytes become dead space, or, when they are the data
    ///   tail, `dataLength` retracts over them. The next grow reclaims dead bytes
    ///   once they outnumber the live ones (`emit_list_reclaim_before_grow`).
    ///   Closing the hole instead moved every later byte and fixed up every
    ///   entry's offset, so removing near the end cost O(count).
    ///
    /// Both shifts run **downward**, so the destination trails the source and
    /// [`Self::emit_block_copy_advance`]'s forward copy is safe on the overlap.
    /// (Its doc comment says the destination must not overlap the source *ahead*
    /// of it; behind is the direction that works.) That is the opposite of
    /// `insert`/`prepend`, which shift up and need
    /// [`Self::emit_block_copy_backward`].
    ///
    /// An out-of-range index raises the same `ErrIndexOutOfRange` as the
    /// out-of-place path, before anything is mutated. The caller guarantees the
    /// buffer is uniquely owned and — unlike `append` — that **no `FOR EACH` is
    /// iterating it**: this shift rewrites entries *below* the count a live loop
    /// snapshotted at entry, which that loop can observe (see
    /// `planning/plan-121-gate-inventory.md`, the `removeAt` asymmetry).
    pub(crate) fn lower_list_remove_at_in_place(
        &mut self,
        buffer_slot: usize,
        index_slot: usize,
        list_type: &ParameterType,
        element_type: &ParameterType,
    ) -> Result<(), String> {
        let entry_stride = list_entry_stride(element_type);
        let kind2_payload = kind2_payload_size(element_type);
        let _layout = CollectionTypeLayout::from_type(list_type)
            .ok_or_else(|| format!("native code collection type '{list_type}' is not supported"))?;

        let base = self.temporary_vreg();
        let index = self.temporary_vreg();
        let count = self.temporary_vreg();
        let scratch = self.temporary_vreg();
        let src = self.temporary_vreg();
        let dst = self.temporary_vreg();
        let len = self.temporary_vreg();
        let copy_scratch = self.temporary_vreg();
        let hole_off = self.temporary_vreg();
        let hole_len = self.temporary_vreg();

        let hole_off_slot = self.allocate_stack_object("remove_inplace_hole_off", 8);
        let hole_len_slot = self.allocate_stack_object("remove_inplace_hole_len", 8);

        let valid_start = self.label("remove_inplace_valid_start");
        let invalid = self.label("remove_inplace_invalid");
        let done = self.label("remove_inplace_done");

        // Bounds: 0 <= index < count, checked before any byte moves so a raise
        // leaves the list exactly as it was.
        self.emit(abi::load_u64(&base, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&index, abi::stack_pointer(), index_slot));
        self.emit(abi::load_u64(&count, &base, COLLECTION_OFFSET_COUNT));
        self.emit(abi::compare_immediate(&index, "0"));
        self.emit(abi::branch_ge(&valid_start));
        self.emit(abi::branch(&invalid));
        self.emit(abi::label(&valid_start));
        self.emit(abi::compare_registers(&index, &count));
        self.emit(abi::branch_ge(&invalid));

        if let Some(payload) = kind2_payload {
            // --- Entry-free representation: one downward shift of the data. ---
            // dst = data + index*p ; src = dst + p ; len = (count-1-index)*p.
            self.emit_collection_data_pointer_for(&dst, &base, element_type);
            self.emit(abi::move_immediate(
                &scratch,
                "Integer",
                &payload.to_string(),
            ));
            self.emit(abi::multiply_registers(&len, &index, &scratch));
            self.emit(abi::add_registers(&dst, &dst, &len));
            self.emit(abi::add_immediate(&src, &dst, payload));
            self.emit(abi::subtract_registers(&len, &count, &index));
            self.emit(abi::subtract_immediate(&len, &len, 1));
            self.emit(abi::multiply_registers(&len, &len, &scratch));
            self.emit_block_copy_advance(&dst, &src, &len, &copy_scratch, "remove_inplace_k2");

            // count -= 1; dataLength -= payload.
            self.emit(abi::load_u64(&base, abi::stack_pointer(), buffer_slot));
            self.emit(abi::load_u64(&count, &base, COLLECTION_OFFSET_COUNT));
            self.emit(abi::subtract_immediate(&count, &count, 1));
            self.emit(abi::store_u64(&count, &base, COLLECTION_OFFSET_COUNT));
            self.emit(abi::load_u64(
                &scratch,
                &base,
                COLLECTION_OFFSET_DATA_LENGTH,
            ));
            self.emit(abi::subtract_immediate(&scratch, &scratch, payload));
            self.emit(abi::store_u64(
                &scratch,
                &base,
                COLLECTION_OFFSET_DATA_LENGTH,
            ));
        } else {
            // plan-134-H: the removed element owns a graph nothing else reaches; free it before
            // its entry is shifted away. The call clobbers every register, so reload them.
            if self.owns_graph(element_type) {
                self.emit_drop_list_element(buffer_slot, index_slot, element_type)?;
                self.emit(abi::load_u64(&base, abi::stack_pointer(), buffer_slot));
                self.emit(abi::load_u64(&index, abi::stack_pointer(), index_slot));
                self.emit(abi::load_u64(&count, &base, COLLECTION_OFFSET_COUNT));
            }
            // --- Lookup table + packed data. ---
            // Read the removed entry's payload span BEFORE the entry shift
            // overwrites it.
            self.emit(abi::move_immediate(
                &scratch,
                "Integer",
                &entry_stride.to_string(),
            ));
            self.emit(abi::multiply_registers(&len, &index, &scratch));
            self.emit(abi::add_immediate(&dst, &base, COLLECTION_HEADER_SIZE));
            self.emit(abi::add_registers(&dst, &dst, &len)); // &entry[index]
            self.emit(abi::load_u64(
                &hole_off,
                &dst,
                COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
            ));
            self.emit(abi::load_u64(
                &hole_len,
                &dst,
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ));
            self.emit(abi::store_u64(
                &hole_off,
                abi::stack_pointer(),
                hole_off_slot,
            ));
            self.emit(abi::store_u64(
                &hole_len,
                abi::stack_pointer(),
                hole_len_slot,
            ));

            // Entry table: shift [index+1, count) down one stride. `dst` already
            // points at entry[index].
            self.emit(abi::add_immediate(&src, &dst, entry_stride));
            self.emit(abi::subtract_registers(&len, &count, &index));
            self.emit(abi::subtract_immediate(&len, &len, 1)); // suffix count
            self.emit(abi::multiply_registers(&len, &len, &scratch));
            self.emit_block_copy_advance(&dst, &src, &len, &copy_scratch, "remove_inplace_e");

            // bug-732: the data region is not compacted. Closing the hole moved
            // every byte after it and fixed up every entry's offset — O(count)
            // whatever the index, so a loop removing near the end was O(n²). When
            // the removed payload is the data tail, `dataLength` retracts over it
            // (the list stays as tight as it was); otherwise its bytes stay behind
            // as dead space, which the next grow reclaims
            // (`emit_list_reclaim_before_grow`). The call costs the entries after
            // `index`, which the shift above had to move anyway.
            let alignment = self.list_element_padding_alignment(element_type);
            let keep_length = self.label("remove_inplace_hole");
            let emptied = self.label("remove_inplace_emptied");
            self.emit(abi::load_u64(&base, abi::stack_pointer(), buffer_slot));
            self.emit(abi::load_u64(&count, &base, COLLECTION_OFFSET_COUNT));
            self.emit(abi::subtract_immediate(&count, &count, 1));
            self.emit(abi::store_u64(&count, &base, COLLECTION_OFFSET_COUNT));
            self.emit(abi::compare_immediate(&count, "0"));
            self.emit(abi::branch_eq(&emptied));
            // Tail test: align(holeOffset + holeLen) >= dataLength.
            self.emit(abi::load_u64(
                &hole_off,
                abi::stack_pointer(),
                hole_off_slot,
            ));
            self.emit(abi::load_u64(
                &hole_len,
                abi::stack_pointer(),
                hole_len_slot,
            ));
            self.emit(abi::add_registers(&len, &hole_off, &hole_len));
            self.emit_align_offset_register(&len, alignment, &scratch);
            self.emit(abi::load_u64(
                &scratch,
                &base,
                COLLECTION_OFFSET_DATA_LENGTH,
            ));
            self.emit(abi::compare_registers(&len, &scratch));
            self.emit(abi::branch_lo(&keep_length));
            // Only ever retract: a zero-length payload may carry any offset.
            self.emit(abi::compare_registers(&hole_off, &scratch));
            self.emit(abi::branch_ge(&keep_length));
            self.emit(abi::store_u64(
                &hole_off,
                &base,
                COLLECTION_OFFSET_DATA_LENGTH,
            ));
            self.emit(abi::branch(&keep_length));
            // Nothing left: the whole data region is dead, so it resets.
            self.emit(abi::label(&emptied));
            self.emit(abi::store_u64(
                abi::ZERO,
                &base,
                COLLECTION_OFFSET_DATA_LENGTH,
            ));
            self.emit(abi::label(&keep_length));
        }
        self.emit(abi::branch(&done));
        self.emit(abi::label(&invalid));
        self.raise_error("collections.removeAt", "ErrIndexOutOfRange")?;
        self.emit(abi::label(&done));
        Ok(())
    }
}
