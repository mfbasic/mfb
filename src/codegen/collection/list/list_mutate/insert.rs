//! plan-176-Q: inserting into a list — a collection at an index, prepend and insert in place.
//! The shared grow/repack helpers live in `collection/list/list_mutate/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// Insert collection `B` (insert_slot) into list `A` (base_slot) at
    /// `index_slot` using the offset-stable scheme (plan-01 §4.1): copy `A`'s and
    /// `B`'s data regions verbatim (no per-entry repack), then splice the lookup
    /// table with three block moves — head `A` entries verbatim, `B` entries with
    /// each `valueOffset` shifted by `A.dataLength`, tail `A` entries verbatim.
    /// Append is `index == count`, prepend is `index == 0`. Phase 2 keeps the
    /// result tight (`capacity == count`).
    pub(crate) fn lower_list_insert_collection(
        &mut self,
        base_slot: usize,
        index_slot: usize,
        insert_slot: usize,
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
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        let scratch15 = self.temporary_vreg();
        let scratch16 = self.temporary_vreg();
        let scratch17 = self.temporary_vreg();
        let layout = CollectionTypeLayout::from_type(list_type)
            .ok_or_else(|| format!("native code collection type '{list_type}' is not supported"))?;
        for register in [
            scratch20, scratch21, scratch22, scratch23, scratch24, scratch25,
        ] {
            self.mark_register_used(&register.render());
        }
        // bug-175 E: a variable-length element payload (a record with an inline
        // String, a data union, or a flat nested collection) must start on an
        // aligned offset, exactly as the literal writer pads it
        // (builder_collection_layout/). `list_element_padding_alignment` returns
        // 1 for every fixed-size / byte-addressed payload, so every guarded
        // `emit_align_offset_*` below is elided and primitive lists stay
        // byte-identical.
        let value_alignment = self.list_element_padding_alignment(element_type);
        let result_slot = self.allocate_stack_object("list_insert_result", 8);
        let valid_start = self.label("list_insert_valid_start");
        let alloc_ok = self.label("list_insert_alloc_ok");
        let invalid = self.label("list_insert_invalid");
        let done = self.label("list_insert_done");

        // Validate 0 <= index <= count(A), then size the allocation:
        //   HEADER + (count_A + count_B) * ENTRY + (dataLen_A + dataLen_B).
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), base_slot));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), insert_slot));
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
        self.emit(abi::branch_gt(&invalid));
        // bug-732: how many of A's data bytes the result carries — all of them, or
        // only the live ones when A's dead bytes (left by in-place removals)
        // outnumber them. Kind 2 has no dead bytes and keeps its instructions.
        let packed_a = if entry_stride != 0 {
            let packed = self.emit_list_packed_length(base_slot, element_type);
            // The walk used its own registers; reload the operands it may share.
            self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), base_slot));
            self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), insert_slot));
            self.emit(abi::load_u64(
                &scratch11,
                &scratch8,
                COLLECTION_OFFSET_COUNT,
            ));
            Some(packed)
        } else {
            None
        };
        self.emit(abi::load_u64(
            &scratch12,
            &scratch9,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::add_registers(&scratch13, &scratch11, &scratch12));
        match &packed_a {
            Some(packed) => self.emit(abi::load_u64(
                &scratch14,
                abi::stack_pointer(),
                packed.length_slot,
            )),
            None => self.emit(abi::load_u64(
                &scratch14,
                &scratch8,
                COLLECTION_OFFSET_DATA_LENGTH,
            )),
        }
        // bug-175 E: pad A's data length up to the element alignment so B's data
        // region — and this reserved allocation size — start on an aligned
        // boundary (consistent with the header write and the data/entry copies).
        if value_alignment > 1 {
            let align_scratch = self.temporary_vreg();
            self.emit_align_offset_register(&scratch14, value_alignment, &align_scratch);
        }
        self.emit(abi::load_u64(
            &scratch15,
            &scratch9,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::add_registers(&scratch15, &scratch14, &scratch15));
        let size_overflow = self.label("list_insert_size_overflow");
        self.emit(abi::move_immediate(
            &scratch16,
            "Integer",
            &entry_stride.to_string(),
        ));
        // Checked collection-size arithmetic (bug-147.7): count and dataLength are
        // read from live collection headers, so route count*ENTRY + HEADER + dataLen
        // through the overflow-guarded helpers — a wrapped size would under-allocate
        // and corrupt the heap.
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

        // Header: total count / total data length (recomputed from the pointer
        // slots, which survive `arena_alloc`; the pre-alloc registers do not).
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), base_slot));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), insert_slot));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch8,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::load_u64(
            &scratch12,
            &scratch9,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::add_registers(&scratch13, &scratch11, &scratch12));
        match &packed_a {
            Some(packed) => self.emit(abi::load_u64(
                &scratch14,
                abi::stack_pointer(),
                packed.length_slot,
            )),
            None => self.emit(abi::load_u64(
                &scratch14,
                &scratch8,
                COLLECTION_OFFSET_DATA_LENGTH,
            )),
        }
        // bug-175 E: the stored dataLength must include the pad between A's and B's
        // data regions so it matches the reserved size and the aligned copy dst.
        if value_alignment > 1 {
            let align_scratch = self.temporary_vreg();
            self.emit_align_offset_register(&scratch14, value_alignment, &align_scratch);
        }
        self.emit(abi::load_u64(
            &scratch15,
            &scratch9,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::add_registers(&scratch15, &scratch14, &scratch15));
        self.emit_write_list_header_from_registers(&layout, &nb, &scratch13, &scratch15);

        // bug-365: for a fixed-width element type the data region is laid out in
        // INDEX order, not in the offset-stable order below. The result block is
        // fresh, so ordering it costs one extra block copy — A's head, then B,
        // then A's tail — instead of A-verbatim-then-B-verbatim plus an entry
        // splice. The lookup entries then degrade to the identity mapping.
        //
        // The offset-stable scheme (plan-01 §4.1) exists to avoid moving payload
        // bytes, but for these types the payload is SMALLER than the 40-byte
        // entry record it moves instead — 5x smaller for an `Integer`, 40x for a
        // `Byte`. So this is strictly less memory traffic as well as correct.
        //
        // Correctness rests on an induction: A's and B's own data regions are
        // already in index order. Every producer of a fixed-width list maintains
        // that — literals and `append` build in order, `removeAt`/`sort`/grow and
        // `copy_collection_tight` preserve it, `mid`/`slice`/`transform`/`filter`
        // rebuild in order, and this function and `lower_list_prepend_in_place`
        // and `lower_collection_set` are the three that used to break it.
        if let Some(payload) = list_element_is_fixed_width(element_type) {
            // `list_element_padding_alignment` is 1 for every fixed-width scalar,
            // so there is no inter-element pad to reserve and every `emit_align_*`
            // in the variable-width path below would be elided anyway.
            let payload_text = payload.to_string();

            // --- Data region, in index order: A[0..i), then B, then A[i..n). ---
            self.emit_collection_data_pointer_for(&scratch17, &nb, &element_type); // dst data cursor
            self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), base_slot));
            self.emit_collection_data_pointer_for(&scratch20, &scratch8, &element_type); // A data cursor
            self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), index_slot));
            self.emit(abi::move_immediate(&scratch16, "Integer", &payload_text));
            self.emit(abi::multiply_registers(&scratch14, &scratch10, &scratch16)); // i * p
                                                                                    // Advances BOTH cursors, so `scratch20` is left at A's element `i` —
                                                                                    // exactly where the tail copy below must resume.
            self.emit_block_copy_advance(
                &scratch17,
                &scratch20,
                &scratch14,
                &scratch22,
                "list_insert_ord_head",
            );
            self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), insert_slot));
            self.emit_collection_data_pointer_for(&scratch12, &scratch9, &element_type); // B data base
            self.emit(abi::load_u64(
                &scratch15,
                &scratch9,
                COLLECTION_OFFSET_DATA_LENGTH,
            )); // m * p
            self.emit_block_copy_advance(
                &scratch17,
                &scratch12,
                &scratch15,
                &scratch22,
                "list_insert_ord_mid",
            );
            self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), base_slot));
            self.emit(abi::load_u64(
                &scratch13,
                &scratch8,
                COLLECTION_OFFSET_COUNT,
            ));
            self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), index_slot));
            self.emit(abi::subtract_registers(&scratch13, &scratch13, &scratch10)); // n - i
            self.emit(abi::move_immediate(&scratch16, "Integer", &payload_text));
            self.emit(abi::multiply_registers(&scratch13, &scratch13, &scratch16));
            self.emit_block_copy_advance(
                &scratch17,
                &scratch20,
                &scratch13,
                &scratch22,
                "list_insert_ord_tail",
            );

            // No lookup table to write: this branch is `list_element_is_fixed_width`,
            // and `list_entry_stride` returns 0 for exactly that predicate
            // (`builder_collection_layout/`), so the result list is entry-FREE.
            // Element `k` is found at `k * payload` by arithmetic, which is what lets a
            // linear reader be correct — not an entry record.
            //
            // The identity-mapping loop that used to stand here was dead for the same
            // reason its in-place twin in `lower_list_splice_in_place` was: every store
            // sat behind `entry_stride != 0`, unreachable inside this branch, so it ran
            // `n + m` iterations per call and wrote nothing into the block.
        } else {
            let packed = packed_a
                .as_ref()
                .ok_or("native list insert: an entry list lost its packed length")?;
            // --- Data region: A verbatim, then B verbatim at offset dataLen_A. ---
            // bug-732: a packed A is laid out by the pass after the entry splice;
            // the cursor then skips its (live) length.
            let a_verbatim = self.label("list_insert_dataA_verbatim");
            let a_placed = self.label("list_insert_dataA_placed");
            self.emit_collection_data_pointer_for(&scratch17, &nb, &element_type); // dst data base
            self.emit(abi::load_u64(
                &scratch14,
                abi::stack_pointer(),
                packed.length_slot,
            ));
            self.emit(abi::load_u64(
                &scratch16,
                abi::stack_pointer(),
                packed.packs_slot,
            ));
            self.emit(abi::compare_immediate(&scratch16, "0"));
            self.emit(abi::branch_eq(&a_verbatim));
            self.emit(abi::add_registers(&scratch17, &scratch17, &scratch14));
            self.emit(abi::branch(&a_placed));
            self.emit(abi::label(&a_verbatim));
            self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), base_slot));
            self.emit_collection_data_pointer_for(&scratch20, &scratch8, &element_type); // A data base
            self.emit_block_copy_advance(
                &scratch17,
                &scratch20,
                &scratch14,
                &scratch22,
                "list_insert_dataA",
            );
            self.emit(abi::label(&a_placed));
            self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), insert_slot));
            self.emit_collection_data_pointer_for(&scratch20, &scratch9, &element_type); // B data base
            self.emit(abi::load_u64(
                &scratch15,
                &scratch9,
                COLLECTION_OFFSET_DATA_LENGTH,
            ));
            // bug-175 E: round the dst data cursor (data base + dataLen_A) up to the
            // element alignment before copying B's region. The data base is 8-aligned
            // (HEADER and ENTRY are multiples of 8, block alloc is 8-aligned), so this
            // is exactly `base + align(dataLen_A)` — the same padded dataLen_A the size,
            // header, and entry-offset shift use.
            if value_alignment > 1 {
                let align_scratch = self.temporary_vreg();
                self.emit_align_offset_register(&scratch17, value_alignment, &align_scratch);
            }
            self.emit_block_copy_advance(
                &scratch17,
                &scratch20,
                &scratch15,
                &scratch22,
                "list_insert_dataB",
            );

            // --- Lookup table splice. ---
            self.emit(abi::move_immediate(
                &scratch16,
                "Integer",
                &entry_stride.to_string(),
            ));
            // Head: dst.table[0..i) <- A.table[0..i) verbatim.
            self.emit(abi::load_u64(&nb, abi::stack_pointer(), result_slot));
            self.emit(abi::add_immediate(&scratch17, &nb, COLLECTION_HEADER_SIZE)); // dst table cursor
            self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), base_slot));
            self.emit(abi::add_immediate(
                &scratch20,
                &scratch8,
                COLLECTION_HEADER_SIZE,
            )); // A table cursor
            self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), index_slot));
            self.emit(abi::multiply_registers(&scratch21, &scratch10, &scratch16)); // i * ENTRY
            self.emit_block_copy_advance(
                &scratch17,
                &scratch20,
                &scratch21,
                &scratch22,
                "list_insert_head",
            );

            // Inserted: dst.table[i..i+count_B) <- B entries, valueOffset += dataLen_A.
            self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), insert_slot));
            self.emit(abi::add_immediate(
                &scratch12,
                &scratch9,
                COLLECTION_HEADER_SIZE,
            )); // B table cursor
            self.emit(abi::load_u64(
                &scratch11,
                &scratch9,
                COLLECTION_OFFSET_COUNT,
            )); // remaining B entries
            self.emit(abi::load_u64(
                &scratch14,
                abi::stack_pointer(),
                packed.length_slot,
            )); // dataLen_A shift
                // Bulk-copy the `count_B` inserted entries verbatim, then shift each
                // valueOffset up by dataLen_A — their payloads now sit after A's data
                // region (plan-25-B B2). Copying B's entries and B's data region both
                // verbatim (a single uniform offset shift) preserves B's internal layout
                // even when B is not packed in entry order, unlike a per-entry re-pack.
                // The copy advances `scratch17` to dst.table[i+count_B], where the tail
                // copy below resumes.
                // bug-175 E: shift B's valueOffsets by the padded A data length so they
                // match the aligned destination of B's copied data region above.
            if value_alignment > 1 {
                let align_scratch = self.temporary_vreg();
                self.emit_align_offset_register(&scratch14, value_alignment, &align_scratch);
            }
            self.emit_bulk_copy_entries_shift(
                &scratch12,
                &scratch17,
                &scratch11,
                Some((&scratch14, false)),
                "list_insert_b",
            );

            // Tail: dst.table[i+count_B..] <- A.table[i..) verbatim. x20 already points
            // at A entry i (advanced past the head copy).
            self.emit(abi::move_immediate(
                &scratch16,
                "Integer",
                &entry_stride.to_string(),
            ));
            self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), base_slot));
            self.emit(abi::load_u64(
                &scratch13,
                &scratch8,
                COLLECTION_OFFSET_COUNT,
            ));
            self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), index_slot));
            self.emit(abi::subtract_registers(&scratch13, &scratch13, &scratch10)); // count_A - i
            self.emit(abi::multiply_registers(&scratch21, &scratch13, &scratch16));
            self.emit_block_copy_advance(
                &scratch17,
                &scratch20,
                &scratch21,
                &scratch22,
                "list_insert_tail",
            );
            self.emit_list_insert_pack_a(
                base_slot,
                index_slot,
                insert_slot,
                result_slot,
                packed,
                element_type,
            );
        }
        self.emit(abi::branch(&done));
        self.emit(abi::label(&invalid));
        self.raise_error("collections.insert", "ErrIndexOutOfRange")?;
        self.emit(abi::label(&done));

        let result = self.allocate_register();
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            origin: None,
            type_: list_type.clone(),
            location: Operand::from(result),
            text: format!("list update {list_type} over {element_type}"),
        })
    }

    /// Prepend a single element (held in `item_slot`) to the front of the list
    /// whose pointer lives in `buffer_slot`, **mutating `buffer_slot` in place**
    /// (plan-02 §3). Ensures room exactly like `lower_list_append_in_place`
    /// (geometric grow only when full), then shifts the live lookup entries right
    /// by one (so the new entry takes index 0) and appends the element's payload to
    /// the spare data tail — entry offsets are independent of position, so no data
    /// move is needed. Still O(n) per call (the entry shift), but it drops the
    /// per-call allocation + double copy of the value-semantic insert. The caller
    /// guarantees unique ownership and not an active `FOR EACH` iterable.
    pub(crate) fn lower_list_prepend_in_place(
        &mut self,
        buffer_slot: usize,
        item_slot: usize,
        list_type: &ParameterType,
        element_type: &ParameterType,
    ) -> Result<ValueResult, String> {
        self.lower_list_splice_in_place(
            buffer_slot,
            SpliceAt::Front,
            item_slot,
            list_type,
            element_type,
            None,
        )
    }

    /// Insert `item_slot`'s element at the runtime index in `index_slot`,
    /// **mutating the live buffer in place** (plan-121-B).
    ///
    /// The out-of-place `collections::insert` allocated a fresh block and copied
    /// the whole list on every call, which is why `list (Fixed) insert` measured
    /// 61x c -O0 and `list (Dynamic) insert` 75x. The shift itself stays O(N) —
    /// see [`Self::lower_list_splice_in_place`].
    ///
    /// The caller guarantees the buffer is uniquely owned and that **no
    /// `FOR EACH` is iterating it**: the shift moves entries the loop already
    /// snapshotted, so this is not a case `append`'s permissive gate covers.
    pub(crate) fn lower_list_insert_in_place(
        &mut self,
        buffer_slot: usize,
        index_slot: usize,
        item_slot: usize,
        list_type: &ParameterType,
        element_type: &ParameterType,
    ) -> Result<ValueResult, String> {
        self.lower_list_splice_in_place(
            buffer_slot,
            SpliceAt::At(index_slot),
            item_slot,
            list_type,
            element_type,
            None,
        )
    }
}
