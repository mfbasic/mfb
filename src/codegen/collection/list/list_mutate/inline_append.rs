//! plan-176-Q: appending one element to an inline-payload list in place. The shared grow/repack
//! helpers live in `collection/list/list_mutate/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// Append a single element (held in `item_slot`) to an **inlined** `List`
    /// field that is the *last inlined field* of the record whose pointer lives
    /// in `record_slot` (bug-430). The collection sub-block sits at
    /// `record_ptr + fieldOffset`, where `fieldOffset` is read from the field's
    /// fixed offset slot (`8 * field_index`) and is invariant across a realloc
    /// (the fixed prefix + any earlier inlined sub-blocks are copied verbatim).
    ///
    /// This is the inlined sibling of [`Self::lower_list_append_in_place`]. A
    /// standalone list owns its own arena block, so that helper reallocates just
    /// the list; here the list is embedded in the record's block, so growth must
    /// reallocate the **whole record block** (geometric headroom on the
    /// sub-block's trailing data region — the collection must be the last inlined
    /// field so nothing after it shifts), copy the fixed prefix verbatim, relay
    /// the collection entries/data into the grown sub-block, free the old record
    /// block, and repoint `record_slot`. When the sub-block already has spare
    /// capacity + data bytes, the element is written in place with no allocation
    /// (amortized O(1) — the whole point). On return `record_slot` holds the
    /// (possibly new) record pointer; the caller propagates it to the canonical
    /// owner (a resource STATE slot or the local's slot). Emits an
    /// `append_inplace_realloc` label on the grow path.
    ///
    /// plan-145-F: `summed_offset` is a frame slot already holding the field's
    /// offset from the record block (a nested path's summed offset,
    /// `emit_path_field_offset`); `None` reads it from `8 * field_index`.
    pub(crate) fn lower_inline_list_append_in_place(
        &mut self,
        record_slot: usize,
        field_index: usize,
        summed_offset: Option<usize>,
        list_type: &ParameterType,
        element_type: &ParameterType,
        item_slot: usize,
    ) -> Result<(), String> {
        // Zero for a kind-2 list: no entry array; the formulas keep their shape.
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

        let item = PayloadSlot {
            slot: item_slot,
            type_: element_type.clone(),
        };
        // Byte size of the new payload's data bytes.
        let need_slot = self.emit_payload_length_to_stack(&item, "inline_append_need")?;

        // fieldOffset: the sub-block's block-relative start. Invariant across the
        // realloc (the prefix is copied verbatim), so read it once.
        let field_off_slot = self.allocate_stack_object("inline_append_foff", 8);
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

        let data_offset_slot = self.allocate_stack_object("inline_append_doff", 8);
        let new_cap_slot = self.allocate_stack_object("inline_append_newcap", 8);
        let new_dcap_slot = self.allocate_stack_object("inline_append_newdcap", 8);
        let new_rec_slot = self.allocate_stack_object("inline_append_newrec", 8);
        let subblock_slot = self.allocate_stack_object("inline_append_sub", 8);

        let realloc = self.label("append_inplace_realloc");
        let write = self.label("inline_append_write");
        let alloc_ok = self.label("inline_append_alloc_ok");
        let dcap_keep = self.label("inline_append_dcap_keep");
        let size_overflow = self.label("inline_append_size_overflow");

        // sub-block base into scratch8 (record_ptr + fieldOffset).
        let load_sub = |b: &mut Self, dst: &VirtualRegister| {
            b.emit(abi::load_u64(dst, abi::stack_pointer(), record_slot));
            b.emit(abi::load_u64(
                &scratch16,
                abi::stack_pointer(),
                field_off_slot,
            ));
            b.emit(abi::add_registers(dst, dst, &scratch16));
        };

        // bug-732: a variable-width grow may first reclaim dead bytes and retry.
        let retry = (entry_stride != 0).then(|| self.label("inline_append_retry"));

        // --- Room check: count < capacity AND align(dataLength)+need <= dataCapacity. ---
        if let Some(retry) = &retry {
            self.emit(abi::label(retry));
        }
        load_sub(self, &scratch8);
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

        // --- Grow: reallocate the WHOLE record block with headroom on the tail. ---
        self.emit(abi::label(&realloc));
        if let Some(retry) = &retry {
            // The repack reallocates the record through `InlineGrow`, which needs
            // the sub-block address in a slot.
            load_sub(self, &scratch8);
            self.emit(abi::store_u64(
                &scratch8,
                abi::stack_pointer(),
                subblock_slot,
            ));
            self.emit_list_reclaim_before_grow(
                subblock_slot,
                need_slot,
                list_type,
                element_type,
                Some(crate::codegen::collection::map::map_mutate::InlineGrow {
                    block_slot: record_slot,
                    field_off_slot,
                }),
                retry,
            )?;
        }
        // newCapacity = step(capacity).
        load_sub(self, &scratch8);
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
            "inline_append_grow_cap",
        );
        self.emit(abi::store_u64(
            &scratch14,
            abi::stack_pointer(),
            new_cap_slot,
        ));
        // newDataCapacity = max(newCapacity * width, align(dataLength)+need) for a
        // fixed-width list (bug-621), max(step(dataCapacity), align(dataLength)+need)
        // otherwise.
        if let Some(width) = list_element_is_fixed_width(element_type) {
            self.emit_fixed_width_data_capacity(
                &scratch14,
                &scratch14,
                &scratch15,
                width,
                &size_overflow,
            );
        } else {
            load_sub(self, &scratch8);
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
                "inline_append_grow_dcap",
            );
        }
        load_sub(self, &scratch8);
        self.emit(abi::load_u64(
            &scratch11,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        if value_alignment > 1 {
            let align_scratch = self.temporary_vreg();
            self.emit_align_offset_register(&scratch11, value_alignment, &align_scratch);
        }
        self.emit(abi::load_u64(&scratch12, abi::stack_pointer(), need_slot));
        self.emit(abi::add_registers(&scratch11, &scratch11, &scratch12)); // required
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

        // Copy the fixed prefix [0, fieldOffset) verbatim (fixed slots + any
        // earlier inlined sub-blocks): dst = new_rec, src = old_rec.
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
        self.emit_copy_bytes(&scratch17, &scratch20, &scratch14, "inline_append_prefix");

        // Write the grown sub-block header at new_rec + fieldOffset: old count /
        // old dataLength, new capacity / new dataCapacity.
        self.emit(abi::load_u64(&scratch20, abi::stack_pointer(), record_slot));
        self.emit(abi::load_u64(
            &scratch16,
            abi::stack_pointer(),
            field_off_slot,
        ));
        self.emit(abi::add_registers(&scratch20, &scratch20, &scratch16)); // old sub-block
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
        self.emit(abi::add_registers(&scratch17, &scratch17, &scratch16)); // new sub-block
        self.emit_write_collection_header_full(
            &layout, &scratch17, &scratch9, &scratch14, &scratch11, &scratch15,
        );

        // Copy the data region verbatim (dataLength bytes), capacity-based bases.
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
            "inline_append_grow_data",
        );

        // Copy the live lookup entries verbatim (count * ENTRY bytes) — kind-0/1
        // only; a kind-2 list has no entry array (entry_stride == 0).
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
                "inline_append_grow_entries",
            );
        }

        // Free the old record block. Old size = fieldOffset + HEADER +
        // oldCapacity*ENTRY + oldDataCapacity, matching what it was allocated with.
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), record_slot));
        self.emit(abi::load_u64(&scratch20, abi::stack_pointer(), record_slot));
        self.emit(abi::load_u64(
            &scratch16,
            abi::stack_pointer(),
            field_off_slot,
        ));
        self.emit(abi::add_registers(&scratch20, &scratch20, &scratch16)); // old sub-block
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

        // Install the grown record block; fall through to write the new element.
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

        // --- Write the new element into slot[count], payload at dataLength. ---
        self.emit(abi::label(&write));
        // Materialize the current sub-block pointer into its own slot (needed by
        // emit_copy_payload_to_collection, which reads a collection pointer slot).
        load_sub(self, &scratch8);
        self.emit(abi::store_u64(
            &scratch8,
            abi::stack_pointer(),
            subblock_slot,
        ));
        self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        if value_alignment > 1 {
            let align_scratch = self.temporary_vreg();
            self.emit_align_offset_register(&scratch11, value_alignment, &align_scratch);
        }
        // Entry write (kind-0/1 only).
        if entry_stride != 0 {
            self.emit(abi::add_immediate(
                &scratch12,
                &scratch8,
                COLLECTION_HEADER_SIZE,
            ));
            self.emit(abi::move_immediate(
                &scratch16,
                "Integer",
                &entry_stride.to_string(),
            ));
            self.emit(abi::multiply_registers(&scratch13, &scratch9, &scratch16));
            self.emit(abi::add_registers(&scratch12, &scratch12, &scratch13)); // entry addr
            self.emit(abi::move_immediate(
                &scratch13,
                "Byte",
                &COLLECTION_ENTRY_FLAG_USED.to_string(),
            ));
            self.emit(abi::store_u8(
                &scratch13,
                &scratch12,
                COLLECTION_ENTRY_OFFSET_FLAGS,
            ));
            self.emit(abi::move_immediate(&scratch13, "Integer", "0"));
            self.emit(abi::store_u64(
                &scratch13,
                &scratch12,
                COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
            ));
            self.emit(abi::store_u64(
                &scratch13,
                &scratch12,
                COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
            ));
            self.emit(abi::store_u64(
                &scratch11,
                &scratch12,
                COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
            )); // valueOffset = align(dataLength)
            self.emit(abi::load_u64(&scratch13, abi::stack_pointer(), need_slot));
            self.emit(abi::store_u64(
                &scratch13,
                &scratch12,
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            )); // valueLength = need
        }
        // Copy the payload bytes to data base + align(dataLength).
        self.emit(abi::store_u64(
            &scratch11,
            abi::stack_pointer(),
            data_offset_slot,
        ));
        self.emit_copy_payload_to_collection(
            subblock_slot,
            need_slot,
            &item,
            data_offset_slot,
            element_type,
        )?;
        // Bump count and dataLength on the sub-block.
        load_sub(self, &scratch8);
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
        Ok(())
    }
}
