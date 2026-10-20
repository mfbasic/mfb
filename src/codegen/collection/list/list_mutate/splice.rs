//! plan-176-Q: splicing a list in place (`lower_list_splice_in_place`). The shared grow/repack
//! helpers live in `collection/list/list_mutate/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// Splice one element into the list whose buffer pointer lives in
    /// `buffer_slot`, **mutating that buffer in place**: make room (reusing the
    /// spare slot and data bytes, growing geometrically only when full), shift
    /// what follows the insertion point up by one, and write the new entry.
    ///
    /// [`SpliceAt::Front`] is `prepend`; [`SpliceAt::At`] is
    /// `collections::insert(list, i, v)`, whose index is read from a frame slot at
    /// runtime. The two share everything except where the shift starts and where
    /// the new element lands, which is why `insert` is this function rather than a
    /// second copy of the room check and the geometric grow.
    ///
    /// **`Front` is byte-identical to what `prepend` emitted before this became a
    /// splice.** Every index-dependent site below special-cases it and emits the
    /// original instructions; a constant zero is never materialized into a
    /// register, because that would be an extra instruction and a golden churn
    /// buying nothing.
    ///
    /// The shift runs **upward**, so the destination leads the source and the
    /// overlap needs [`Self::emit_block_copy_backward`] — the opposite of
    /// `removeAt`, which shifts down and can copy forward. Still O(N) per call:
    /// that is the operation's defined cost and C pays it too (`memmove`). What
    /// this deletes is the per-call allocate + copy + free around it, which
    /// spike 3 measured at 36x the data movement.
    pub(crate) fn lower_list_splice_in_place(
        &mut self,
        buffer_slot: usize,
        at: SpliceAt,
        item_slot: usize,
        list_type: &ParameterType,
        element_type: &ParameterType,
        inline: Option<crate::codegen::collection::map::map_mutate::InlineGrow>,
    ) -> Result<ValueResult, String> {
        let prefix = match at {
            SpliceAt::Front => "prepend",
            SpliceAt::At(_) => "insert",
        };
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
        let item = PayloadSlot {
            slot: item_slot,
            type_: element_type.clone(),
        };
        // bug-175 E: pad the element start offset like the literal writer; 1 (a
        // no-op at every align site) for fixed-size / byte-addressed payloads keeps
        // primitive lists byte-identical.
        let value_alignment = self.list_element_padding_alignment(element_type);
        let need_slot = self.emit_payload_length_to_stack(&item, "prepend_inplace_need")?;
        let data_offset_slot = self.allocate_stack_object("prepend_inplace_doff", 8);
        let new_cap_slot = self.allocate_stack_object("prepend_inplace_newcap", 8);
        let new_dcap_slot = self.allocate_stack_object("prepend_inplace_newdcap", 8);
        let new_buf_slot = self.allocate_stack_object("prepend_inplace_newbuf", 8);

        let realloc = self.label(&format!("{prefix}_inplace_realloc"));
        let write = self.label(&format!("{prefix}_inplace_write"));
        let alloc_ok = self.label(&format!("{prefix}_inplace_alloc_ok"));
        let dcap_keep = self.label(&format!("{prefix}_inplace_dcap_keep"));
        let shift_loop = self.label(&format!("{prefix}_inplace_shift_loop"));
        let shift_done = self.label(&format!("{prefix}_inplace_shift_done"));

        // Bounds, for `insert` only: `0 <= index <= count`, matching the
        // out-of-place `lower_list_insert`'s gate exactly (index == count is an
        // append and is legal). This runs BEFORE anything is written, so a raise
        // leaves the list exactly as it was — the same contract the copying path
        // has, and the reason it cannot be folded into the shift below.
        //
        // `Front` emits none of this: index 0 is always in range, and adding a
        // check would be dead code AND a golden churn for `prepend`.
        let splice_done = if let SpliceAt::At(index_slot) = at {
            let invalid = self.label(&format!("{prefix}_inplace_invalid"));
            let valid_start = self.label(&format!("{prefix}_inplace_valid_start"));
            let done = self.label(&format!("{prefix}_inplace_done"));
            let idx = self.temporary_vreg();
            let cnt = self.temporary_vreg();
            let base = self.temporary_vreg();
            self.emit(abi::load_u64(&base, abi::stack_pointer(), buffer_slot));
            self.emit(abi::load_u64(&idx, abi::stack_pointer(), index_slot));
            self.emit(abi::load_u64(&cnt, &base, COLLECTION_OFFSET_COUNT));
            self.emit(abi::compare_immediate(&idx, "0"));
            self.emit(abi::branch_ge(&valid_start));
            self.emit(abi::branch(&invalid));
            self.emit(abi::label(&valid_start));
            self.emit(abi::compare_registers(&idx, &cnt));
            self.emit(abi::branch_gt(&invalid));
            let ok = self.label(&format!("{prefix}_inplace_in_range"));
            self.emit(abi::branch(&ok));
            self.emit(abi::label(&invalid));
            self.raise_error("collections.insert", "ErrIndexOutOfRange")?;
            self.emit(abi::branch(&done));
            self.emit(abi::label(&ok));
            Some(done)
        } else {
            None
        };

        // bug-732: a variable-width grow may first reclaim dead bytes and retry.
        let retry = (entry_stride != 0).then(|| self.label(&format!("{prefix}_inplace_retry")));

        // Room check: count < capacity AND dataLength + need <= dataCapacity.
        if let Some(retry) = &retry {
            self.emit(abi::label(retry));
        }
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(
            &scratch10,
            &scratch8,
            COLLECTION_OFFSET_CAPACITY,
        ));
        self.emit(abi::compare_registers(&scratch9, &scratch10));
        self.emit(abi::branch_ge(&realloc));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        // bug-175 E: reserve for align(dataLength)+need, matching the aligned write
        // offset below — a "just barely fits" unaligned list must not skip the grow.
        if value_alignment > 1 {
            let align_scratch = self.temporary_vreg();
            self.emit_align_offset_register(&scratch11, value_alignment, &align_scratch);
        }
        self.emit(abi::load_u64(&scratch12, abi::stack_pointer(), need_slot));
        self.emit(abi::add_registers(&scratch11, &scratch11, &scratch12));
        self.emit(abi::load_u64(
            &scratch13,
            &scratch8,
            COLLECTION_OFFSET_DATA_CAPACITY,
        ));
        self.emit(abi::compare_registers(&scratch11, &scratch13));
        self.emit(abi::branch_hi(&realloc));
        self.emit(abi::branch(&write));

        // --- Grow: geometric headroom; copy entries + data verbatim. ---
        self.emit(abi::label(&realloc));
        if let Some(retry) = &retry {
            self.emit_list_reclaim_before_grow(
                buffer_slot,
                need_slot,
                list_type,
                element_type,
                inline,
                retry,
            )?;
        }
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(
            &scratch10,
            &scratch8,
            COLLECTION_OFFSET_CAPACITY,
        ));
        self.emit_geometric_step(
            &scratch10,
            &scratch14,
            &scratch15,
            COLLECTION_GROW_LOOKUP_INIT,
            COLLECTION_GROW_LOOKUP_TAPER,
            &format!("{prefix}_grow_cap"),
        );
        self.emit(abi::store_u64(
            &scratch14,
            abi::stack_pointer(),
            new_cap_slot,
        ));
        // newDataCapacity = max(newCapacity * width, align(dataLength)+need) for a
        // fixed-width list (bug-621), max(step(dataCapacity), align(dataLength)+need)
        // otherwise. The overflow label is taken here only when the product needs
        // it, so a variable-width list keeps its label numbering.
        let fixed_size_overflow = match list_element_is_fixed_width(element_type) {
            Some(width) => {
                let overflow = self.label(&format!("list_{prefix}_size_overflow"));
                self.emit_fixed_width_data_capacity(
                    &scratch14, &scratch14, &scratch15, width, &overflow,
                );
                Some(overflow)
            }
            None => {
                self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
                self.emit(abi::load_u64(
                    &scratch10,
                    &scratch8,
                    COLLECTION_OFFSET_DATA_CAPACITY,
                ));
                self.emit_geometric_step(
                    &scratch10,
                    &scratch14,
                    &scratch15,
                    COLLECTION_GROW_DATA_INIT,
                    COLLECTION_GROW_DATA_TAPER,
                    &format!("{prefix}_grow_dcap"),
                );
                None
            }
        };
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        // bug-175 E: required = align(dataLength)+need, consistent with the room
        // check and the write so the grown dataCapacity always holds the payload.
        if value_alignment > 1 {
            let align_scratch = self.temporary_vreg();
            self.emit_align_offset_register(&scratch11, value_alignment, &align_scratch);
        }
        self.emit(abi::load_u64(&scratch12, abi::stack_pointer(), need_slot));
        self.emit(abi::add_registers(&scratch11, &scratch11, &scratch12));
        self.emit(abi::compare_registers(&scratch14, &scratch11));
        self.emit(abi::branch_hi(&dcap_keep));
        self.emit(abi::branch_eq(&dcap_keep));
        self.emit(abi::move_register(&scratch14, &scratch11));
        self.emit(abi::label(&dcap_keep));
        self.emit(abi::store_u64(
            &scratch14,
            abi::stack_pointer(),
            new_dcap_slot,
        ));
        // alloc = HEADER + newCapacity * ENTRY + newDataCapacity.
        self.emit(abi::load_u64(
            &scratch14,
            abi::stack_pointer(),
            new_cap_slot,
        ));
        let size_overflow = fixed_size_overflow
            .unwrap_or_else(|| self.label(&format!("list_{prefix}_size_overflow")));
        self.emit(abi::move_immediate(
            &scratch16,
            "Integer",
            &entry_stride.to_string(),
        ));
        // Checked collection-size arithmetic (bug-147.7): capacity/dataCapacity are
        // runtime-derived, so guard count*ENTRY + HEADER + dataCap against overflow.
        self.emit_checked_size_multiply(&scratch17, &scratch14, &scratch16, &size_overflow);
        self.emit_checked_size_add_immediate(
            abi::return_register(),
            &scratch17,
            COLLECTION_HEADER_SIZE,
            &size_overflow,
        );
        self.emit(abi::load_u64(
            &scratch15,
            abi::stack_pointer(),
            new_dcap_slot,
        ));
        self.emit_checked_size_add(
            abi::return_register(),
            abi::return_register(),
            &scratch15,
            &size_overflow,
        );
        // plan-121-C: an inlined field grows the RECORD, so widen the request by
        // the record prefix before asking the allocator for it.
        if let Some(g) = inline {
            self.emit_inline_grow_extend_size(&g, &size_overflow);
        }
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
            new_buf_slot,
        ));
        // plan-121-C: for an inlined field the allocation IS the record; split it
        // so every line below writes through the sub-block unchanged.
        let splice_new_rec = inline.map(|g| self.emit_inline_grow_split(&g, new_buf_slot));
        // Header: old count / old dataLength, new capacity / data capacity.
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::load_u64(
            &scratch14,
            abi::stack_pointer(),
            new_cap_slot,
        ));
        self.emit(abi::load_u64(
            &scratch15,
            abi::stack_pointer(),
            new_dcap_slot,
        ));
        let nb = self.temporary_vreg();
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), new_buf_slot));
        self.emit_write_collection_header_full(
            &layout, &nb, &scratch9, &scratch14, &scratch11, &scratch15,
        );
        // Copy the data region verbatim (dataLength bytes), capacity-based base.
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), new_buf_slot));
        self.emit_collection_data_pointer_for(&scratch17, &nb, &element_type);
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
        self.emit_collection_data_pointer_for(&scratch20, &scratch8, &element_type);
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
            &format!("{prefix}_grow_data"),
        );
        // Copy the live lookup entries verbatim (count * ENTRY bytes).
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), new_buf_slot));
        self.emit(abi::add_immediate(&scratch17, &nb, COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
        self.emit(abi::add_immediate(
            &scratch20,
            &scratch8,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::move_immediate(
            &scratch16,
            "Integer",
            &entry_stride.to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch21, &scratch9, &scratch16));
        self.emit_block_copy_advance(
            &scratch17,
            &scratch20,
            &scratch21,
            &scratch22,
            &format!("{prefix}_grow_entries"),
        );
        // Free the abandoned pre-grow buffer (still in `buffer_slot`) before
        // installing the grown one — otherwise a prepend-in-a-loop leaks the old
        // buffer on every geometric grow (bug-47, mirrors `append` at :957-991).
        // plan-121-C: an inlined sub-block is not its own allocation, so what is
        // released is the whole old RECORD block instead.
        match (inline, splice_new_rec) {
            (Some(g), Some(new_rec_slot)) => {
                self.emit_inline_grow_free_old(&g, buffer_slot, list_type, new_rec_slot)?
            }
            _ => self.emit_free_pre_grow_buffer(buffer_slot, list_type)?,
        }
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), new_buf_slot));
        self.emit(abi::store_u64(&nb, abi::stack_pointer(), buffer_slot));
        self.emit(abi::branch(&write));

        // --- Write: shift entries right by one, new entry at slot[0]. ---
        self.emit(abi::label(&write));
        // bug-365: for a fixed-width element type the payload goes to the FRONT
        // of the data region, not to the spare tail. The data region shifts up by
        // one element and the lookup entries become the identity mapping, so a
        // reader walking the data region linearly sees index order.
        //
        // The old scheme wrote the new payload at `dataLength` and shifted only
        // the 40-byte entry records, which is why `prepend` was the single most
        // common way to arm bug-365. It moved MORE bytes than this does: 40 per
        // element rather than `payload` (1 for a `Byte`, 8 for an `Integer`).
        //
        // The grow path above is untouched — it copies data and entries verbatim,
        // so an ordered buffer stays ordered — but note it may have reallocated,
        // so every bound below is re-derived from `buffer_slot` rather than held
        // in a register across `_mfb_arena_alloc` (which destroys all
        // caller-saved registers).
        if let Some(payload) = list_element_is_fixed_width(element_type) {
            let payload_text = payload.to_string();

            // Shift the whole live data region up by one payload, backwards —
            // source and destination overlap for any count above one.
            self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
            self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
            self.emit_collection_data_pointer_for(&scratch17, &scratch8, &element_type); // data base
            self.emit(abi::move_immediate(&scratch16, "Integer", &payload_text));
            self.emit(abi::multiply_registers(&scratch11, &scratch9, &scratch16)); // n * p
            self.emit(abi::add_registers(&scratch12, &scratch17, &scratch11)); // src end
            self.emit(abi::add_immediate(&scratch13, &scratch12, payload)); // dst end
                                                                            // `Front` moves the whole live region, so `n * p` is already the
                                                                            // length. `At(i)` moves only the tail `[i, count)`, i.e.
                                                                            // `(count - i) * p` -- the head stays put, which is the entire
                                                                            // difference between an insert and a prepend on this path.
            if let SpliceAt::At(index_slot) = at {
                let idx = self.temporary_vreg();
                self.emit(abi::load_u64(&idx, abi::stack_pointer(), index_slot));
                self.emit(abi::subtract_registers(&scratch11, &scratch9, &idx));
                self.emit(abi::multiply_registers(&scratch11, &scratch11, &scratch16));
            }
            self.emit_block_copy_backward(
                &scratch13,
                &scratch12,
                &scratch11,
                &scratch22,
                &format!("{prefix}_inplace_shift_data"),
            );

            // The new payload occupies offset 0 for a prepend, `index * p` for
            // an insert -- the slot the shift above just vacated.
            match at {
                SpliceAt::Front => {
                    self.emit(abi::move_immediate(&scratch13, "Integer", "0"));
                }
                SpliceAt::At(index_slot) => {
                    self.emit(abi::load_u64(&scratch13, abi::stack_pointer(), index_slot));
                    self.emit(abi::move_immediate(&scratch16, "Integer", &payload_text));
                    self.emit(abi::multiply_registers(&scratch13, &scratch13, &scratch16));
                }
            }
            self.emit(abi::store_u64(
                &scratch13,
                abi::stack_pointer(),
                data_offset_slot,
            ));
            self.emit_copy_payload_to_collection(
                buffer_slot,
                need_slot,
                &item,
                data_offset_slot,
                element_type,
            )?;

            // Bump count and dataLength before writing entries, so the identity
            // loop covers the new element too.
            self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
            self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
            self.emit(abi::add_immediate(&scratch9, &scratch9, 1));
            self.emit(abi::store_u64(
                &scratch9,
                &scratch8,
                COLLECTION_OFFSET_COUNT,
            ));
            self.emit(abi::load_u64(
                &scratch10,
                &scratch8,
                COLLECTION_OFFSET_DATA_LENGTH,
            ));
            self.emit(abi::add_immediate(&scratch10, &scratch10, payload));
            self.emit(abi::store_u64(
                &scratch10,
                &scratch8,
                COLLECTION_OFFSET_DATA_LENGTH,
            ));

            // No lookup table to write: this branch is `list_element_is_fixed_width`,
            // and `list_entry_stride` returns 0 for exactly that predicate
            // (`builder_collection_layout/`), so a fixed-width list is entry-FREE —
            // element `i` lives at `i * payload` and is found by arithmetic, not by a
            // 40-byte entry record.
            //
            // A loop writing "the identity mapping over entries 0..count" used to stand
            // here (and still does in the `else` arm, where entries exist). Every store
            // in it was guarded by `entry_stride != 0`, which is unreachable inside this
            // branch, so it ran `count` iterations per call and wrote nothing into the
            // block: 20 instructions per element whose only effects were a spill slot
            // overwritten three times and never read, and an `add_imm x8, x8, 0`.
            // That made a fixed-width `insert`/`prepend` O(N) in dead work on top of the
            // O(N) shift, which is why `list (Fixed) removeAt` (no such loop) ran at
            // memmove rate while `insert` sat ~10x below it.
        } else {
            // Shift lookup entries [0..count) → [1..count+1), backward to avoid overlap.
            self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
            self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
            self.emit(abi::subtract_immediate(&scratch10, &scratch9, 1)); // i = count - 1
                                                                          // `Front` walks down to 0; `At(k)` stops at k, leaving entries
                                                                          // `[0, k)` where they are.
            let stop = match at {
                SpliceAt::Front => None,
                SpliceAt::At(index_slot) => {
                    let s = self.temporary_vreg();
                    self.emit(abi::load_u64(&s, abi::stack_pointer(), index_slot));
                    Some(s)
                }
            };
            self.emit(abi::label(&shift_loop));
            match &stop {
                None => {
                    self.emit(abi::compare_immediate(&scratch10, "0"));
                    self.emit(abi::branch_lt(&shift_done));
                }
                Some(s) => {
                    self.emit(abi::compare_registers(&scratch10, s));
                    self.emit(abi::branch_lt(&shift_done));
                }
            }
            // src = buffer + HEADER + i*ENTRY ; dst = src + ENTRY (x8 = buffer, live).
            self.emit(abi::move_immediate(
                &scratch16,
                "Integer",
                &entry_stride.to_string(),
            ));
            self.emit(abi::multiply_registers(&scratch11, &scratch10, &scratch16));
            self.emit(abi::add_immediate(
                &scratch12,
                &scratch8,
                COLLECTION_HEADER_SIZE,
            ));
            self.emit(abi::add_registers(&scratch11, &scratch12, &scratch11)); // src = entry[i]
            self.emit(abi::add_immediate(&scratch12, &scratch11, entry_stride)); // dst = entry[i+1]
            for offset in [0usize, 8, 16, 24, 32] {
                self.emit(abi::load_u64(&scratch13, &scratch11, offset));
                self.emit(abi::store_u64(&scratch13, &scratch12, offset));
            }
            self.emit(abi::subtract_immediate(&scratch10, &scratch10, 1));
            self.emit(abi::branch(&shift_loop));
            self.emit(abi::label(&shift_done));
            // New entry at slot[0]: payload at dataLength, valueOffset = dataLength.
            self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
            self.emit(abi::load_u64(
                &scratch11,
                &scratch8,
                COLLECTION_OFFSET_DATA_LENGTH,
            ));
            // bug-175 E: the new payload starts at the aligned dataLength; this
            // scratch11 feeds both the entry valueOffset and the data-copy offset.
            if value_alignment > 1 {
                let align_scratch = self.temporary_vreg();
                self.emit_align_offset_register(&scratch11, value_alignment, &align_scratch);
            }
            // entry[0] for a prepend; entry[index] for an insert.
            self.emit(abi::add_immediate(
                &scratch12,
                &scratch8,
                COLLECTION_HEADER_SIZE,
            ));
            if let SpliceAt::At(index_slot) = at {
                let idx = self.temporary_vreg();
                let stride = self.temporary_vreg();
                self.emit(abi::load_u64(&idx, abi::stack_pointer(), index_slot));
                self.emit(abi::move_immediate(
                    &stride,
                    "Integer",
                    &entry_stride.to_string(),
                ));
                self.emit(abi::multiply_registers(&idx, &idx, &stride));
                self.emit(abi::add_registers(&scratch12, &scratch12, &idx));
            }
            self.emit(abi::move_immediate(
                &scratch13,
                "Byte",
                &COLLECTION_ENTRY_FLAG_USED.to_string(),
            ));
            if entry_stride != 0 {
                self.emit(abi::store_u8(
                    &scratch13,
                    &scratch12,
                    COLLECTION_ENTRY_OFFSET_FLAGS,
                ));
            }
            self.emit(abi::move_immediate(&scratch13, "Integer", "0"));
            if entry_stride != 0 {
                self.emit(abi::store_u64(
                    &scratch13,
                    &scratch12,
                    COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
                ));
            }
            if entry_stride != 0 {
                self.emit(abi::store_u64(
                    &scratch13,
                    &scratch12,
                    COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
                ));
            }
            if entry_stride != 0 {
                self.emit(abi::store_u64(
                    &scratch11,
                    &scratch12,
                    COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
                ));
            }
            self.emit(abi::load_u64(&scratch13, abi::stack_pointer(), need_slot));
            if entry_stride != 0 {
                self.emit(abi::store_u64(
                    &scratch13,
                    &scratch12,
                    COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
                ));
            }
            // Copy the payload bytes to data base + dataLength.
            self.emit(abi::store_u64(
                &scratch11,
                abi::stack_pointer(),
                data_offset_slot,
            ));
            self.emit_copy_payload_to_collection(
                buffer_slot,
                need_slot,
                &item,
                data_offset_slot,
                element_type,
            )?;
            // Bump count and dataLength.
            self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
            self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
            self.emit(abi::add_immediate(&scratch9, &scratch9, 1));
            self.emit(abi::store_u64(
                &scratch9,
                &scratch8,
                COLLECTION_OFFSET_COUNT,
            ));
            self.emit(abi::load_u64(
                &scratch9,
                &scratch8,
                COLLECTION_OFFSET_DATA_LENGTH,
            ));
            // bug-175 E: the new dataLength is align(old)+need so it accounts for the
            // pad the payload was written past (keeps the next element aligned too).
            if value_alignment > 1 {
                let align_scratch = self.temporary_vreg();
                self.emit_align_offset_register(&scratch9, value_alignment, &align_scratch);
            }
            self.emit(abi::load_u64(&scratch13, abi::stack_pointer(), need_slot));
            self.emit(abi::add_registers(&scratch9, &scratch9, &scratch13));
            self.emit(abi::store_u64(
                &scratch9,
                &scratch8,
                COLLECTION_OFFSET_DATA_LENGTH,
            ));
        }

        if let Some(done) = splice_done {
            self.emit(abi::label(&done));
        }

        let result = self.allocate_register();
        self.emit(abi::load_u64(&result, abi::stack_pointer(), buffer_slot));
        Ok(ValueResult {
            origin: None,
            type_: list_type.clone(),
            location: Operand::from(result),
            text: format!("{prefix} in place {list_type} over {element_type}"),
        })
    }
}
