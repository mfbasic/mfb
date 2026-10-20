//! plan-176-Q: bulk append in place, including a self-slice append. The shared grow/repack
//! helpers live in `collection/list/list_mutate/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// `x = collections::append(x, collections::mid(x, start, count))` on a
    /// uniquely-owned fixed-width (kind 2) list, **in place and without the `mid`
    /// temporary** (bug-774).
    ///
    /// The value is exactly the two-step form's: the slice's elements are copies of
    /// `x`'s own, appended after them. The steps are the two-step form's too, minus
    /// the temporary list: `mid`'s range checks run first, in `mid`'s order and
    /// with `mid`'s error (`collections.mid`, `ErrIndexOutOfRange`, at `mid_loc`);
    /// then the append's room check and geometric grow; then one forward copy from
    /// `x`'s data into its own spare tail. Every error is raised before the first
    /// write, so a failed statement leaves `x` as it was. The copy cannot overlap:
    /// the source ends at `start + count <= count(x)`, where the destination
    /// begins, and both are re-derived from the block after any grow. The
    /// temporary was a block that nothing but the append ever saw, so eliding it
    /// is copy elision, not a change in when a copy happens (`mfb spec memory
    /// collections`, Self-updates).
    pub(crate) fn lower_list_self_slice_append_in_place(
        &mut self,
        buffer_slot: usize,
        start_slot: usize,
        count_slot: usize,
        list_type: &ParameterType,
        element_type: &ParameterType,
        mid_loc: crate::target::shared::nir::NirSourceLoc,
    ) -> Result<(), String> {
        let width = kind2_payload_size(element_type).ok_or_else(|| {
            format!("native self-slice append needs a fixed-width element, got {element_type}")
        })?;
        let layout = CollectionTypeLayout::from_type(list_type)
            .ok_or_else(|| format!("native code collection type '{list_type}' is not supported"))?;
        let block = self.temporary_vreg();
        let len = self.temporary_vreg();
        let start = self.temporary_vreg();
        let count = self.temporary_vreg();
        let end = self.temporary_vreg();
        let need_count = self.temporary_vreg();
        let need_data = self.temporary_vreg();
        let added = self.temporary_vreg();
        let width_reg = self.temporary_vreg();
        let capacity = self.temporary_vreg();
        let new_capacity = self.temporary_vreg();
        let new_data_capacity = self.temporary_vreg();
        let step_scratch = self.temporary_vreg();
        let new_block = self.temporary_vreg();
        let dst = self.temporary_vreg();
        let src = self.temporary_vreg();
        let remaining = self.temporary_vreg();
        let word = self.temporary_vreg();
        let old_size = self.temporary_vreg();
        let data_length = self.temporary_vreg();

        let invalid_range = self.label("self_slice_append_invalid_range");
        let realloc = self.label("self_slice_append_realloc");
        let write = self.label("self_slice_append_write");
        let alloc_ok = self.label("self_slice_append_alloc_ok");
        let size_overflow = self.label("self_slice_append_size_overflow");
        let cap_keep = self.label("self_slice_append_cap_keep");
        let dcap_keep = self.label("self_slice_append_dcap_keep");

        // `mid`'s checks, in `lower_list_mid`'s order: start >= 0, count >= 0,
        // start <= len, start + count does not wrap, start + count <= len.
        self.emit(abi::load_u64(&block, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&start, abi::stack_pointer(), start_slot));
        self.emit(abi::load_u64(&count, abi::stack_pointer(), count_slot));
        self.emit(abi::load_u64(&len, &block, COLLECTION_OFFSET_COUNT));
        self.emit(abi::compare_immediate(&start, "0"));
        self.emit(abi::branch_lt(&invalid_range));
        self.emit(abi::compare_immediate(&count, "0"));
        self.emit(abi::branch_lt(&invalid_range));
        self.emit(abi::compare_registers(&start, &len));
        self.emit(abi::branch_gt(&invalid_range));
        self.emit(abi::add_registers(&end, &start, &count));
        self.emit(abi::compare_registers(&end, &start));
        self.emit(abi::branch_lt(&invalid_range));
        self.emit(abi::compare_registers(&end, &len));
        self.emit(abi::branch_gt(&invalid_range));
        let saved_loc = self.current_loc;
        self.current_loc = mid_loc;
        let raised = self.emit_cold_block(&invalid_range, |builder| {
            builder.raise_error("collections.mid", "ErrIndexOutOfRange")
        });
        self.current_loc = saved_loc;
        raised?;

        // need_count = len + count; need_data = dataLength + count * width. Both
        // are bounded by twice the existing block (count <= len), so neither wraps.
        self.emit(abi::add_registers(&need_count, &len, &count));
        if width == 1 {
            self.emit(abi::move_register(&added, &count));
        } else {
            self.emit(abi::shift_left_immediate(
                &added,
                &count,
                width.trailing_zeros() as u8,
            ));
        }
        self.emit(abi::load_u64(
            &data_length,
            &block,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::add_registers(&need_data, &data_length, &added));
        self.emit(abi::load_u64(&capacity, &block, COLLECTION_OFFSET_CAPACITY));
        self.emit(abi::compare_registers(&need_count, &capacity));
        self.emit(abi::branch_hi(&realloc));
        self.emit(abi::load_u64(
            &word,
            &block,
            COLLECTION_OFFSET_DATA_CAPACITY,
        ));
        self.emit(abi::compare_registers(&need_data, &word));
        self.emit(abi::branch_ls(&write));

        // --- Grow, exactly as the bulk append does for a fixed-width list:
        // newCapacity = max(step(capacity), need_count), newDataCapacity =
        // max(newCapacity * width, need_data) (bug-621); copy the data region,
        // free the old block, install the new one. ---
        self.emit(abi::label(&realloc));
        self.emit_geometric_step(
            &capacity,
            &new_capacity,
            &step_scratch,
            COLLECTION_GROW_LOOKUP_INIT,
            COLLECTION_GROW_LOOKUP_TAPER,
            "self_slice_append_grow_cap",
        );
        self.emit(abi::compare_registers(&new_capacity, &need_count));
        self.emit(abi::branch_hi(&cap_keep));
        self.emit(abi::branch_eq(&cap_keep));
        self.emit(abi::move_register(&new_capacity, &need_count));
        self.emit(abi::label(&cap_keep));
        self.emit_fixed_width_data_capacity(
            &new_capacity,
            &new_data_capacity,
            &width_reg,
            width,
            &size_overflow,
        );
        self.emit(abi::compare_registers(&new_data_capacity, &need_data));
        self.emit(abi::branch_hi(&dcap_keep));
        self.emit(abi::branch_eq(&dcap_keep));
        self.emit(abi::move_register(&new_data_capacity, &need_data));
        self.emit(abi::label(&dcap_keep));
        // A kind-2 block has no entry table: size = HEADER + newDataCapacity.
        self.emit_checked_size_add_immediate(
            abi::return_register(),
            &new_data_capacity,
            COLLECTION_HEADER_SIZE,
            &size_overflow,
        );
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&alloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&size_overflow));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::move_register(&new_block, abi::mfb_return(1)));
        self.emit(abi::load_u64(&block, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&len, &block, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(
            &data_length,
            &block,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit_write_collection_header_full(
            &layout,
            &new_block,
            &len,
            &new_capacity,
            &data_length,
            &new_data_capacity,
        );
        self.emit(abi::add_immediate(&dst, &new_block, COLLECTION_HEADER_SIZE));
        self.emit(abi::add_immediate(&src, &block, COLLECTION_HEADER_SIZE));
        self.emit(abi::move_register(&remaining, &data_length));
        self.emit_block_copy_advance(&dst, &src, &remaining, &word, "self_slice_append_grow_data");
        // Free the old block (HEADER + dataCapacity for kind 2), install the new.
        self.emit(abi::load_u64(
            &old_size,
            &block,
            COLLECTION_OFFSET_DATA_CAPACITY,
        ));
        self.emit(abi::add_immediate(
            abi::c_arg(1),
            &old_size,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::move_register(abi::c_arg(0), &block));
        self.emit_arena_free_call();
        self.emit(abi::store_u64(
            &new_block,
            abi::stack_pointer(),
            buffer_slot,
        ));

        // --- Write: x.data[dataLength ..] <- x.data[start * width ..], count elements.
        self.emit(abi::label(&write));
        self.emit(abi::load_u64(&block, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&start, abi::stack_pointer(), start_slot));
        self.emit(abi::load_u64(&count, abi::stack_pointer(), count_slot));
        self.emit(abi::load_u64(
            &data_length,
            &block,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::add_immediate(&dst, &block, COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers(&dst, &dst, &data_length));
        if width == 1 {
            self.emit(abi::add_registers(&src, &block, &start));
            self.emit(abi::move_register(&remaining, &count));
        } else {
            let shift = width.trailing_zeros() as u8;
            self.emit(abi::shift_left_immediate(&src, &start, shift));
            self.emit(abi::add_registers(&src, &block, &src));
            self.emit(abi::shift_left_immediate(&remaining, &count, shift));
        }
        self.emit(abi::add_immediate(&src, &src, COLLECTION_HEADER_SIZE));
        self.emit(abi::move_register(&added, &remaining));
        self.emit_disjoint_copy_word_tail(&dst, &src, &remaining, &word, "self_slice_append_data");
        // count += count(slice); dataLength += its bytes.
        self.emit(abi::load_u64(&len, &block, COLLECTION_OFFSET_COUNT));
        self.emit(abi::add_registers(&len, &len, &count));
        self.emit(abi::store_u64(&len, &block, COLLECTION_OFFSET_COUNT));
        self.emit(abi::add_registers(&data_length, &data_length, &added));
        self.emit(abi::store_u64(
            &data_length,
            &block,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        Ok(())
    }

    /// Copy `remaining` bytes from `src` to `dst`, two ranges that do not overlap
    /// (bug-774). Eight bytes at a time while at least eight remain; then, when the
    /// copy was at least eight bytes long, the last 0–7 are one final word read
    /// from `src + n - 8` and written to `dst + n - 8`. That word rewrites up to seven
    /// bytes the loop already wrote with the same values, which is exact only
    /// because the source is not the destination. A copy shorter than eight bytes
    /// goes byte by byte. A match in `compress::inflate` averages 14 bytes, which
    /// this copies in two words instead of one word and six single bytes. Clobbers
    /// all four registers.
    fn emit_disjoint_copy_word_tail(
        &mut self,
        dst: &VirtualRegister,
        src: &VirtualRegister,
        remaining: &VirtualRegister,
        word: &VirtualRegister,
        prefix: &str,
    ) {
        let tail_src = self.temporary_vreg();
        let tail_dst = self.temporary_vreg();
        let bytes = self.label(&format!("{prefix}_bytes"));
        let word_loop = self.label(&format!("{prefix}_wloop"));
        let done = self.label(&format!("{prefix}_done"));
        self.emit(abi::compare_immediate(remaining, "8"));
        self.emit(abi::branch_lo(&bytes));
        self.emit(abi::add_registers(&tail_src, src, remaining));
        self.emit(abi::subtract_immediate(&tail_src, &tail_src, 8));
        self.emit(abi::add_registers(&tail_dst, dst, remaining));
        self.emit(abi::subtract_immediate(&tail_dst, &tail_dst, 8));
        self.emit(abi::label(&word_loop));
        self.emit(abi::load_u64(word, src, 0));
        self.emit(abi::store_u64(word, dst, 0));
        self.emit(abi::add_immediate(src, src, 8));
        self.emit(abi::add_immediate(dst, dst, 8));
        self.emit(abi::subtract_immediate(remaining, remaining, 8));
        self.emit(abi::compare_immediate(remaining, "7"));
        self.emit(abi::branch_hi(&word_loop));
        self.emit(abi::compare_immediate(remaining, "0"));
        self.emit(abi::branch_eq(&done));
        self.emit(abi::load_u64(word, &tail_src, 0));
        self.emit(abi::store_u64(word, &tail_dst, 0));
        self.emit(abi::branch(&done));
        self.emit(abi::label(&bytes));
        self.emit(abi::compare_immediate(remaining, "0"));
        self.emit(abi::branch_eq(&done));
        self.emit(abi::load_u8(word, src, 0));
        self.emit(abi::store_u8(word, dst, 0));
        self.emit(abi::add_immediate(src, src, 1));
        self.emit(abi::add_immediate(dst, dst, 1));
        self.emit(abi::subtract_immediate(remaining, remaining, 1));
        self.emit(abi::branch(&bytes));
        self.emit(abi::label(&done));
    }

    /// Bulk-append the list at `rhs_slot` onto the list buffer at `buffer_slot`,
    /// **mutating `buffer_slot` in place** (plan-25-B B1). This is the list-RHS
    /// sibling of [`Self::lower_list_append_in_place`]: it grows the uniquely-owned
    /// working buffer once (geometric headroom, sized for the whole batch) when
    /// `count(self) + count(rhs)` entries or `dataLength(self) + dataLength(rhs)`
    /// sibling of [`Self::lower_list_append_in_place`]: it grows the uniquely-owned
    /// working buffer once (geometric headroom, sized for the whole batch) when
    /// `count(self) + count(rhs)` entries or `dataLength(self) + dataLength(rhs)`
    /// bytes do not fit, then bulk-copies the RHS data region into the spare data
    /// tail and the RHS lookup entries into the spare slots — each RHS entry's
    /// `valueOffset` shifted by the pre-append `dataLength(self)` since its payload
    /// now sits after `self`'s data. Amortized O(rhs) per call instead of the
    /// value-semantic rebuild that copies the whole accumulated result each call
    /// (the O(n²) `flatten`/`append_batch` path). The caller guarantees `self` is
    /// uniquely owned and that `rhs` is a distinct buffer (the `append(list, list)`
    /// self-alias is excluded at the gate and takes the value path).
    pub(crate) fn lower_list_bulk_append_in_place(
        &mut self,
        buffer_slot: usize,
        rhs_slot: usize,
        list_type: &ParameterType,
        element_type: &ParameterType,
    ) -> Result<ValueResult, String> {
        // Zero for a kind-2 list: the sizing arithmetic below then reserves no
        // entry array, and the formulas keep their shape (plan-57-D).
        let entry_stride = list_entry_stride(element_type);
        let layout = CollectionTypeLayout::from_type(list_type)
            .ok_or_else(|| format!("native code collection type '{list_type}' is not supported"))?;
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
        let scratch20 = self.temporary_vreg();
        let scratch22 = self.temporary_vreg();

        // bug-175 E: pad self's data length up to the element alignment before rhs's
        // data region is concatenated after it, so rhs's internally-aligned payloads
        // stay aligned. 1 (a no-op at every align site) for fixed-size /
        // byte-addressed payloads keeps primitive lists byte-identical.
        let value_alignment = self.list_element_padding_alignment(element_type);
        let need_count_slot = self.allocate_stack_object("bulk_append_need_count", 8);
        let need_data_slot = self.allocate_stack_object("bulk_append_need_data", 8);
        let new_cap_slot = self.allocate_stack_object("bulk_append_newcap", 8);
        let new_dcap_slot = self.allocate_stack_object("bulk_append_newdcap", 8);
        let new_buf_slot = self.allocate_stack_object("bulk_append_newbuf", 8);

        let realloc = self.label("bulk_append_realloc");
        let write = self.label("bulk_append_write");
        let alloc_ok = self.label("bulk_append_alloc_ok");
        let cap_keep = self.label("bulk_append_cap_keep");
        let dcap_keep = self.label("bulk_append_dcap_keep");
        // bug-732: a variable-width grow may first reclaim dead bytes and retry.
        let retry = (entry_stride != 0).then(|| self.label("bulk_append_retry"));

        // need_count = count(self) + count(rhs); need_data = dataLen(self) + dataLen(rhs).
        if let Some(retry) = &retry {
            self.emit(abi::label(retry));
        }
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), rhs_slot));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch10,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::add_registers(&scratch12, &scratch9, &scratch11));
        self.emit(abi::store_u64(
            &scratch12,
            abi::stack_pointer(),
            need_count_slot,
        ));
        self.emit(abi::load_u64(
            &scratch13,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        // bug-175 E: need_data = align(dataLen_self)+dataLen_rhs. This drives the
        // room check, the grown dataCapacity, AND the stored dataLength, so the pad
        // between the two data regions is reserved everywhere it is consumed.
        if value_alignment > 1 {
            let align_scratch = self.temporary_vreg();
            self.emit_align_offset_register(&scratch13, value_alignment, &align_scratch);
        }
        self.emit(abi::load_u64(
            &scratch14,
            &scratch10,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::add_registers(&scratch15, &scratch13, &scratch14));
        self.emit(abi::store_u64(
            &scratch15,
            abi::stack_pointer(),
            need_data_slot,
        ));

        // Room check: need_count <= capacity AND need_data <= dataCapacity.
        self.emit(abi::load_u64(
            &scratch16,
            &scratch8,
            COLLECTION_OFFSET_CAPACITY,
        ));
        self.emit(abi::compare_registers(&scratch12, &scratch16));
        self.emit(abi::branch_hi(&realloc));
        self.emit(abi::load_u64(
            &scratch17,
            &scratch8,
            COLLECTION_OFFSET_DATA_CAPACITY,
        ));
        self.emit(abi::compare_registers(&scratch15, &scratch17));
        self.emit(abi::branch_hi(&realloc));
        self.emit(abi::branch(&write));

        // --- Grow: allocate a headroom buffer sized for the whole batch; copy
        // self's entries + data verbatim; free the old buffer; install. ---
        self.emit(abi::label(&realloc));
        if let Some(retry) = &retry {
            let extra_slot = self.emit_bulk_append_extra(rhs_slot);
            self.emit_list_reclaim_before_grow(
                buffer_slot,
                extra_slot,
                list_type,
                element_type,
                None,
                retry,
            )?;
        }
        // newCapacity = max(step(capacity), need_count).
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
            "bulk_append_grow_cap",
        );
        self.emit(abi::load_u64(
            &scratch11,
            abi::stack_pointer(),
            need_count_slot,
        ));
        self.emit(abi::compare_registers(&scratch14, &scratch11));
        self.emit(abi::branch_hi(&cap_keep));
        self.emit(abi::branch_eq(&cap_keep));
        self.emit(abi::move_register(&scratch14, &scratch11));
        self.emit(abi::label(&cap_keep));
        self.emit(abi::store_u64(
            &scratch14,
            abi::stack_pointer(),
            new_cap_slot,
        ));
        // newDataCapacity = max(newCapacity * width, need_data) for a fixed-width
        // list (bug-621), max(step(dataCapacity), need_data) otherwise. The overflow
        // label is taken here only when the product needs it, so a variable-width
        // list keeps its label numbering.
        let fixed_size_overflow = match list_element_is_fixed_width(element_type) {
            Some(width) => {
                let overflow = self.label("list_bulk_append_size_overflow");
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
                    "bulk_append_grow_dcap",
                );
                None
            }
        };
        self.emit(abi::load_u64(
            &scratch11,
            abi::stack_pointer(),
            need_data_slot,
        ));
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

        // alloc size = HEADER + newCapacity * ENTRY + newDataCapacity.
        self.emit(abi::load_u64(
            &scratch14,
            abi::stack_pointer(),
            new_cap_slot,
        ));
        let size_overflow =
            fixed_size_overflow.unwrap_or_else(|| self.label("list_bulk_append_size_overflow"));
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

        // Header: old count / old dataLength, new capacity / new data capacity.
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

        // Copy self's data region verbatim (dataLength bytes), capacity-based base.
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
            "bulk_append_grow_data",
        );

        // Copy self's live lookup entries verbatim (count * ENTRY bytes).
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
        self.emit(abi::multiply_registers(&scratch14, &scratch9, &scratch16));
        self.emit_block_copy_advance(
            &scratch17,
            &scratch20,
            &scratch14,
            &scratch22,
            "bulk_append_grow_entries",
        );

        // Free the old buffer (capacity-based size), then install the grown one.
        // Unique ownership means it has no other reference — leaving it unfreed
        // would leak one buffer per outgrown bulk append.
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(
            &scratch9,
            &scratch8,
            COLLECTION_OFFSET_CAPACITY,
        ));
        self.emit(abi::move_immediate(
            &scratch16,
            "Integer",
            &entry_stride.to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch10, &scratch9, &scratch16));
        self.emit(abi::add_immediate(
            &scratch10,
            &scratch10,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch8,
            COLLECTION_OFFSET_DATA_CAPACITY,
        ));
        self.emit(abi::add_registers(abi::c_arg(1), &scratch10, &scratch11));
        // plan-71-C Family-1a: the pointer is arg 0 of the arena-free call → `%arg0`.
        self.emit(abi::move_register(abi::c_arg(0), &scratch8));
        self.emit_arena_free_call();
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), new_buf_slot));
        self.emit(abi::store_u64(&nb, abi::stack_pointer(), buffer_slot));
        self.emit(abi::branch(&write));

        // --- Write: bulk-copy the RHS into self's spare region. ---
        self.emit(abi::label(&write));
        // dst.data + dataLength(self) <- rhs.data (dataLength(rhs) bytes).
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
        self.emit_collection_data_pointer_for(&scratch17, &scratch8, &element_type);
        self.emit(abi::load_u64(
            &scratch9,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        // bug-175 E: place rhs's data at the padded self data length. The data base
        // is 8-aligned, so aligning the byte offset here yields `base +
        // align(dataLen_self)`, matching the padded need_data reservation.
        if value_alignment > 1 {
            let align_scratch = self.temporary_vreg();
            self.emit_align_offset_register(&scratch9, value_alignment, &align_scratch);
        }
        self.emit(abi::add_registers(&scratch17, &scratch17, &scratch9));
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), rhs_slot));
        self.emit_collection_data_pointer_for(&scratch20, &scratch10, &element_type);
        self.emit(abi::load_u64(
            &scratch14,
            &scratch10,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit_block_copy_advance(
            &scratch17,
            &scratch20,
            &scratch14,
            &scratch22,
            "bulk_append_data",
        );

        // dst.table[count(self)..] <- rhs entries, each valueOffset += dataLength(self).
        // Skipped for kind 2: there is no destination entry table, and emitting
        // this would copy `count(rhs)` 40-byte entry records straight over the
        // data region that was just written above (plan-57-D).
        if entry_stride != 0 {
            self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
            self.emit(abi::add_immediate(
                &scratch17,
                &scratch8,
                COLLECTION_HEADER_SIZE,
            ));
            self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
            self.emit(abi::move_immediate(
                &scratch16,
                "Integer",
                &entry_stride.to_string(),
            ));
            self.emit(abi::multiply_registers(&scratch11, &scratch9, &scratch16));
            self.emit(abi::add_registers(&scratch17, &scratch17, &scratch11)); // dst entry[count(self)]
            self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), rhs_slot));
            self.emit(abi::add_immediate(
                &scratch20,
                &scratch10,
                COLLECTION_HEADER_SIZE,
            )); // rhs entry base
            self.emit(abi::load_u64(
                &scratch11,
                &scratch10,
                COLLECTION_OFFSET_COUNT,
            )); // count(rhs)
            self.emit(abi::load_u64(
                &scratch12,
                &scratch8,
                COLLECTION_OFFSET_DATA_LENGTH,
            )); // shift = dataLength(self)
                // bug-175 E: shift rhs valueOffsets by the padded self data length to match
                // the aligned destination of rhs's copied data region above.
            if value_alignment > 1 {
                let align_scratch = self.temporary_vreg();
                self.emit_align_offset_register(&scratch12, value_alignment, &align_scratch);
            }
            self.emit_bulk_copy_entries_shift(
                &scratch20,
                &scratch17,
                &scratch11,
                Some((&scratch12, false)),
                "bulk_append_entries",
            );
        }

        // Bump count += count(rhs); dataLength += dataLength(rhs).
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(
            &scratch9,
            abi::stack_pointer(),
            need_count_slot,
        ));
        self.emit(abi::store_u64(
            &scratch9,
            &scratch8,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::load_u64(
            &scratch9,
            abi::stack_pointer(),
            need_data_slot,
        ));
        self.emit(abi::store_u64(
            &scratch9,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));

        let result = self.allocate_register();
        self.emit(abi::load_u64(&result, abi::stack_pointer(), buffer_slot));
        Ok(ValueResult {
            origin: None,
            type_: list_type.clone(),
            location: Operand::from(result),
            text: format!("bulk append in place {list_type} over {element_type}"),
        })
    }
}
