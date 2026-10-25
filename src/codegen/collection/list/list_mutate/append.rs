//! plan-176-Q: appending one element in place (`lower_list_append_in_place`). The shared
//! grow/repack helpers live in `collection/list/list_mutate/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// Append a single element (held in `item_slot`) to the list buffer whose
    /// pointer lives in `buffer_slot`, **mutating `buffer_slot` in place**
    /// (plan-01 §4.2). When the live buffer has a spare lookup slot and spare
    /// data bytes, the element is written into the spare slot and `count` /
    /// `dataLength` are bumped — no allocation, no copy: the amortized-O(1)
    /// lever. Otherwise a geometric-headroom buffer is allocated, the entries
    /// and data region are copied verbatim, and `buffer_slot` is repointed at it;
    /// the element is then written into the now-spare slot. The caller guarantees
    /// the buffer is uniquely owned (see the Assign-site / private-accumulator
    /// call sites). Returns the (possibly new) buffer pointer.
    pub(crate) fn lower_list_append_in_place(
        &mut self,
        buffer_slot: usize,
        item_slot: usize,
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
        let item = PayloadSlot {
            slot: item_slot,
            type_: element_type.clone(),
        };
        // bug-175 E: pad the element start offset like the literal writer. Returns
        // 1 (a no-op every align site) for fixed-size / byte-addressed payloads, so
        // primitive lists stay byte-identical.
        let value_alignment = self.list_element_padding_alignment(element_type);
        // Byte size of the new payload (its required data bytes).
        let need_slot = self.emit_payload_length_to_stack(&item, "append_inplace_need")?;
        let data_offset_slot = self.allocate_stack_object("append_inplace_doff", 8);
        let new_cap_slot = self.allocate_stack_object("append_inplace_newcap", 8);
        let new_dcap_slot = self.allocate_stack_object("append_inplace_newdcap", 8);
        let new_buf_slot = self.allocate_stack_object("append_inplace_newbuf", 8);

        let realloc = self.label("append_inplace_realloc");
        let write = self.label("append_inplace_write");
        let alloc_ok = self.label("append_inplace_alloc_ok");
        let dcap_keep = self.label("append_inplace_dcap_keep");
        // bug-732: a variable-width grow may first reclaim dead bytes and retry.
        let retry = (entry_stride != 0).then(|| self.label("append_inplace_retry"));

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

        // --- Grow: allocate a headroom buffer; copy entries + data verbatim. ---
        self.emit(abi::label(&realloc));
        if let Some(retry) = &retry {
            self.emit_list_reclaim_before_grow(
                buffer_slot,
                need_slot,
                list_type,
                element_type,
                None,
                retry,
            )?;
        }
        // newCapacity = step(capacity).
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
            "append_grow_cap",
        );
        self.emit(abi::store_u64(
            &scratch14,
            abi::stack_pointer(),
            new_cap_slot,
        ));
        // newDataCapacity = max(newCapacity * width, dataLength + need) for a
        // fixed-width list (bug-621), max(step(dataCapacity), dataLength + need)
        // otherwise. The overflow label is taken here only when the product needs
        // it, so a variable-width list keeps its label numbering.
        let fixed_size_overflow = match list_element_is_fixed_width(element_type) {
            Some(width) => {
                let overflow = self.label("list_append_size_overflow");
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
                    "append_grow_dcap",
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
        self.emit(abi::add_registers(&scratch11, &scratch11, &scratch12)); // required
        self.emit(abi::compare_registers(&scratch14, &scratch11));
        self.emit(abi::branch_hi(&dcap_keep));
        self.emit(abi::branch_eq(&dcap_keep));
        self.emit(abi::move_register(&scratch14, &scratch11)); // step < required → use required
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
            fixed_size_overflow.unwrap_or_else(|| self.label("list_append_size_overflow"));
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
        // plan-183-D: grow the block where it is when the arena can.
        let new_size = self.temporary_vreg();
        let new_cap = self.temporary_vreg();
        let new_dcap = self.temporary_vreg();
        self.emit(abi::move_register(&new_size, abi::return_register()));
        self.emit(abi::load_u64(&new_cap, abi::stack_pointer(), new_cap_slot));
        self.emit(abi::load_u64(
            &new_dcap,
            abi::stack_pointer(),
            new_dcap_slot,
        ));
        self.emit_list_try_extend(
            &crate::codegen::collection::buffer::extend_in_place::ListExtend {
                block_slot: buffer_slot,
                field_off_slot: None,
                new_size: &new_size,
                new_cap: &new_cap,
                new_dcap: &new_dcap,
                layout: &layout,
                element_type: &element_type,
                entry_stride,
                is_map: false,
                prefix: "append_grow",
            },
            &write,
        );
        self.emit(abi::move_register(abi::return_register(), &new_size));
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
            "append_grow_data",
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
            "append_grow_entries",
        );

        // Free the old buffer, then install the grown one. The in-place append
        // fires only under unique ownership, so the old buffer has no other
        // reference; freeing it here stops the geometric-grow path from leaking
        // it (bug-01: every append that outgrew its capacity accumulated the
        // abandoned buffer, so append-heavy loops grew the arena without bound).
        // Sized capacity-based (HEADER + capacity*ENTRY + dataCapacity), matching
        // what the allocation reserved.
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
        // Install the grown buffer; fall through to write the new element.
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), new_buf_slot));
        self.emit(abi::store_u64(&nb, abi::stack_pointer(), buffer_slot));
        self.emit(abi::branch(&write));

        // --- Write the new element into slot[count], payload at dataLength. ---
        self.emit(abi::label(&write));
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        // bug-175 E: the payload starts at the aligned dataLength; this scratch11
        // feeds both the entry valueOffset and the data-copy offset below.
        if value_alignment > 1 {
            let align_scratch = self.temporary_vreg();
            self.emit_align_offset_register(&scratch11, value_alignment, &align_scratch);
        }
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
            )); // valueOffset = dataLength
        }
        self.emit(abi::load_u64(&scratch13, abi::stack_pointer(), need_slot));
        if entry_stride != 0 {
            self.emit(abi::store_u64(
                &scratch13,
                &scratch12,
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            )); // valueLength = need
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

        let result = self.allocate_register();
        self.emit(abi::load_u64(&result, abi::stack_pointer(), buffer_slot));
        Ok(ValueResult {
            origin: None,
            type_: list_type.clone(),
            location: Operand::from(result),
            text: format!("append in place {list_type} over {element_type}"),
        })
    }
}
