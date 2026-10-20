//! plan-176-Q: bulk-appending to an inline-payload list in place. The shared grow/repack
//! helpers live in `collection/list/list_mutate/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// Bulk (concatenation) sibling of [`Self::lower_inline_list_append_in_place`]
    /// for an **inlined** last-field `List` (bug-430): append every element of the
    /// list at `rhs_slot` onto the collection sub-block embedded in the record at
    /// `record_slot` (`8 * field_index` offset slot). Mirrors
    /// [`Self::lower_list_bulk_append_in_place`] but reallocates the WHOLE record
    /// block (geometric headroom on the trailing sub-block) when the batch does
    /// not fit, copying the fixed prefix verbatim and repointing `record_slot`.
    /// The caller guarantees `self` is uniquely owned and `rhs` is a distinct
    /// buffer (the self-alias `append(x, x)` takes the value path at the gate).
    /// Emits an `append_inplace_realloc` label on the grow path.
    /// plan-145-F: `summed_offset` as for
    /// [`Self::lower_inline_list_append_in_place`].
    pub(crate) fn lower_inline_list_bulk_append_in_place(
        &mut self,
        record_slot: usize,
        field_index: usize,
        summed_offset: Option<usize>,
        list_type: &ParameterType,
        element_type: &ParameterType,
        rhs_slot: usize,
    ) -> Result<(), String> {
        let entry_stride = list_entry_stride(element_type);
        let layout = CollectionTypeLayout::from_type(list_type)
            .ok_or_else(|| format!("native code collection type '{list_type}' is not supported"))?;
        let value_alignment = self.list_element_padding_alignment(element_type);
        let field_slot_off = 8 * field_index;

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

        // fieldOffset: sub-block start; invariant across realloc (prefix verbatim).
        let field_off_slot = self.allocate_stack_object("inline_bulk_foff", 8);
        match summed_offset {
            Some(slot) => self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), slot)),
            None => {
                self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), record_slot));
                self.emit(abi::load_u64(&scratch9, &scratch8, field_slot_off));
            }
        }
        self.emit(abi::store_u64(
            &scratch9,
            abi::stack_pointer(),
            field_off_slot,
        ));

        let need_count_slot = self.allocate_stack_object("inline_bulk_need_count", 8);
        let need_data_slot = self.allocate_stack_object("inline_bulk_need_data", 8);
        let new_cap_slot = self.allocate_stack_object("inline_bulk_newcap", 8);
        let new_dcap_slot = self.allocate_stack_object("inline_bulk_newdcap", 8);
        let new_rec_slot = self.allocate_stack_object("inline_bulk_newrec", 8);
        let subblock_slot = self.allocate_stack_object("inline_bulk_sub", 8);

        let realloc = self.label("append_inplace_realloc");
        let write = self.label("inline_bulk_write");
        let alloc_ok = self.label("inline_bulk_alloc_ok");
        let cap_keep = self.label("inline_bulk_cap_keep");
        let dcap_keep = self.label("inline_bulk_dcap_keep");
        let size_overflow = self.label("inline_bulk_size_overflow");

        let load_sub = |b: &mut Self, dst: &VirtualRegister, tmp: &VirtualRegister| {
            b.emit(abi::load_u64(dst, abi::stack_pointer(), record_slot));
            b.emit(abi::load_u64(tmp, abi::stack_pointer(), field_off_slot));
            b.emit(abi::add_registers(dst, dst, tmp));
        };

        // bug-732: a variable-width grow may first reclaim dead bytes and retry.
        let retry = (entry_stride != 0).then(|| self.label("inline_bulk_retry"));

        // need_count = count(self) + count(rhs); need_data = align(dataLen(self)) + dataLen(rhs).
        if let Some(retry) = &retry {
            self.emit(abi::label(retry));
        }
        load_sub(self, &scratch8, &scratch16);
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
        load_sub(self, &scratch8, &scratch16);
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

        // --- Grow: reallocate the WHOLE record block sized for the batch. ---
        self.emit(abi::label(&realloc));
        if let Some(retry) = &retry {
            // The repack reallocates the record through `InlineGrow`, which needs
            // the sub-block address in a slot.
            load_sub(self, &scratch8, &scratch16);
            self.emit(abi::store_u64(
                &scratch8,
                abi::stack_pointer(),
                subblock_slot,
            ));
            let extra_slot = self.emit_bulk_append_extra(rhs_slot);
            self.emit_list_reclaim_before_grow(
                subblock_slot,
                extra_slot,
                list_type,
                element_type,
                Some(crate::codegen::collection::map::map_mutate::InlineGrow {
                    block_slot: record_slot,
                    field_off_slot,
                }),
                retry,
            )?;
        }
        // newCapacity = max(step(capacity), need_count).
        load_sub(self, &scratch8, &scratch16);
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
            "inline_bulk_grow_cap",
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
        // list (bug-621), max(step(dataCapacity), need_data) otherwise.
        if let Some(width) = list_element_is_fixed_width(element_type) {
            self.emit_fixed_width_data_capacity(
                &scratch14,
                &scratch14,
                &scratch15,
                width,
                &size_overflow,
            );
        } else {
            load_sub(self, &scratch8, &scratch16);
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
                "inline_bulk_grow_dcap",
            );
        }
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

        // new_record_size = fieldOffset + HEADER + newCapacity*ENTRY + newDataCapacity.
        self.emit(abi::load_u64(
            &scratch14,
            abi::stack_pointer(),
            new_cap_slot,
        ));
        self.emit(abi::move_immediate(
            &scratch16,
            "Integer",
            &entry_stride.to_string(),
        ));
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
        self.emit(abi::load_u64(
            &scratch16,
            abi::stack_pointer(),
            field_off_slot,
        ));
        self.emit_checked_size_add(
            abi::return_register(),
            abi::return_register(),
            &scratch16,
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
            new_rec_slot,
        ));

        // Copy the fixed prefix [0, fieldOffset) verbatim.
        self.emit(abi::load_u64(
            &scratch17,
            abi::stack_pointer(),
            new_rec_slot,
        ));
        self.emit(abi::load_u64(&scratch20, abi::stack_pointer(), record_slot));
        self.emit(abi::load_u64(
            &scratch14,
            abi::stack_pointer(),
            field_off_slot,
        ));
        self.emit_copy_bytes(&scratch17, &scratch20, &scratch14, "inline_bulk_prefix");

        // Grown sub-block header: old count / old dataLength, new capacity / dcap.
        self.emit(abi::load_u64(&scratch20, abi::stack_pointer(), record_slot));
        self.emit(abi::load_u64(
            &scratch16,
            abi::stack_pointer(),
            field_off_slot,
        ));
        self.emit(abi::add_registers(&scratch20, &scratch20, &scratch16)); // old sub
        self.emit(abi::load_u64(
            &scratch9,
            &scratch20,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch20,
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
        self.emit(abi::load_u64(
            &scratch17,
            abi::stack_pointer(),
            new_rec_slot,
        ));
        self.emit(abi::load_u64(
            &scratch16,
            abi::stack_pointer(),
            field_off_slot,
        ));
        self.emit(abi::add_registers(&scratch17, &scratch17, &scratch16)); // new sub
        self.emit_write_collection_header_full(
            &layout, &scratch17, &scratch9, &scratch14, &scratch11, &scratch15,
        );

        // Copy self's data region verbatim (dataLength bytes), capacity-based bases.
        self.emit(abi::load_u64(
            &scratch17,
            abi::stack_pointer(),
            new_rec_slot,
        ));
        self.emit(abi::load_u64(
            &scratch16,
            abi::stack_pointer(),
            field_off_slot,
        ));
        self.emit(abi::add_registers(&scratch17, &scratch17, &scratch16));
        self.emit_collection_data_pointer_for(&scratch10, &scratch17, &element_type);
        self.emit(abi::load_u64(&scratch20, abi::stack_pointer(), record_slot));
        self.emit(abi::load_u64(
            &scratch16,
            abi::stack_pointer(),
            field_off_slot,
        ));
        self.emit(abi::add_registers(&scratch20, &scratch20, &scratch16));
        self.emit_collection_data_pointer_for(&scratch11, &scratch20, &element_type);
        self.emit(abi::load_u64(
            &scratch14,
            &scratch20,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit_block_copy_advance(
            &scratch10,
            &scratch11,
            &scratch14,
            &scratch22,
            "inline_bulk_grow_data",
        );

        // Copy self's live lookup entries verbatim (count * ENTRY bytes) — kind-0/1.
        if entry_stride != 0 {
            self.emit(abi::load_u64(
                &scratch17,
                abi::stack_pointer(),
                new_rec_slot,
            ));
            self.emit(abi::load_u64(
                &scratch16,
                abi::stack_pointer(),
                field_off_slot,
            ));
            self.emit(abi::add_registers(&scratch17, &scratch17, &scratch16));
            self.emit(abi::add_immediate(
                &scratch17,
                &scratch17,
                COLLECTION_HEADER_SIZE,
            ));
            self.emit(abi::load_u64(&scratch20, abi::stack_pointer(), record_slot));
            self.emit(abi::load_u64(
                &scratch16,
                abi::stack_pointer(),
                field_off_slot,
            ));
            self.emit(abi::add_registers(&scratch20, &scratch20, &scratch16));
            // Read count from the sub-block header BEFORE advancing to the entry
            // region, then step past the header to the entry base.
            self.emit(abi::load_u64(
                &scratch9,
                &scratch20,
                COLLECTION_OFFSET_COUNT,
            ));
            self.emit(abi::add_immediate(
                &scratch20,
                &scratch20,
                COLLECTION_HEADER_SIZE,
            ));
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
                "inline_bulk_grow_entries",
            );
        }

        // Free the old record block (fieldOffset + HEADER + oldCap*ENTRY + oldDcap).
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), record_slot));
        self.emit(abi::load_u64(&scratch20, abi::stack_pointer(), record_slot));
        self.emit(abi::load_u64(
            &scratch16,
            abi::stack_pointer(),
            field_off_slot,
        ));
        self.emit(abi::add_registers(&scratch20, &scratch20, &scratch16));
        self.emit(abi::load_u64(
            &scratch9,
            &scratch20,
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
            &scratch20,
            COLLECTION_OFFSET_DATA_CAPACITY,
        ));
        self.emit(abi::add_registers(&scratch10, &scratch10, &scratch11));
        self.emit(abi::load_u64(
            &scratch16,
            abi::stack_pointer(),
            field_off_slot,
        ));
        self.emit(abi::add_registers(abi::c_arg(1), &scratch10, &scratch16));
        self.emit(abi::move_register(abi::c_arg(0), &scratch8));
        self.emit_arena_free_call();

        // Install the grown record block; fall through to write.
        self.emit(abi::load_u64(
            &scratch14,
            abi::stack_pointer(),
            new_rec_slot,
        ));
        self.emit(abi::store_u64(
            &scratch14,
            abi::stack_pointer(),
            record_slot,
        ));
        self.emit(abi::branch(&write));

        // --- Write: bulk-copy the RHS into self's spare region. ---
        self.emit(abi::label(&write));
        load_sub(self, &scratch8, &scratch16);
        self.emit(abi::store_u64(
            &scratch8,
            abi::stack_pointer(),
            subblock_slot,
        ));
        // dst.data + align(dataLength(self)) <- rhs.data (dataLength(rhs) bytes).
        self.emit_collection_data_pointer_for(&scratch17, &scratch8, &element_type);
        self.emit(abi::load_u64(
            &scratch9,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
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
            "inline_bulk_data",
        );

        // dst.table[count(self)..] <- rhs entries, each valueOffset += align(dataLen(self)).
        if entry_stride != 0 {
            self.emit(abi::load_u64(
                &scratch8,
                abi::stack_pointer(),
                subblock_slot,
            ));
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
            self.emit(abi::add_registers(&scratch17, &scratch17, &scratch11)); // dst entry[count]
            self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), rhs_slot));
            self.emit(abi::add_immediate(
                &scratch20,
                &scratch10,
                COLLECTION_HEADER_SIZE,
            ));
            self.emit(abi::load_u64(
                &scratch11,
                &scratch10,
                COLLECTION_OFFSET_COUNT,
            ));
            self.emit(abi::load_u64(
                &scratch12,
                &scratch8,
                COLLECTION_OFFSET_DATA_LENGTH,
            ));
            if value_alignment > 1 {
                let align_scratch = self.temporary_vreg();
                self.emit_align_offset_register(&scratch12, value_alignment, &align_scratch);
            }
            self.emit_bulk_copy_entries_shift(
                &scratch20,
                &scratch17,
                &scratch11,
                Some((&scratch12, false)),
                "inline_bulk_entries",
            );
        }

        // Bump count = need_count; dataLength = need_data.
        self.emit(abi::load_u64(
            &scratch8,
            abi::stack_pointer(),
            subblock_slot,
        ));
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
        Ok(())
    }
}
