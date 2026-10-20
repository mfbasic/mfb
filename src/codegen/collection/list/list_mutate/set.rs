//! plan-176-Q: setting a list element in place. The shared grow/repack helpers live in
//! `collection/list/list_mutate/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// Replace the element at `index_slot` of the list whose buffer pointer lives
    /// in `buffer_slot`, **mutating `buffer_slot` in place** (plan-02 §4.1). A
    /// same-size payload — always the case for a fixed-width element — overwrites
    /// the value bytes at the entry's `valueOffset`. A length change is resolved in
    /// O(1) amortized without touching any other element (bug-627): see the
    /// `set_inplace_resize` arm. An out-of-range index raises
    /// `ErrIndexOutOfRange`. The caller guarantees the buffer is uniquely owned and
    /// not an active `FOR EACH` iterable. Returns the (possibly new) buffer pointer.
    pub(crate) fn lower_list_set_in_place(
        &mut self,
        buffer_slot: usize,
        index_slot: usize,
        item_slot: usize,
        list_type: &ParameterType,
        element_type: &ParameterType,
    ) -> Result<ValueResult, String> {
        self.lower_list_set_in_place_with(
            buffer_slot,
            index_slot,
            item_slot,
            list_type,
            element_type,
            true,
            None,
        )
    }

    /// [`Self::lower_list_set_in_place`], with the drop of the replaced element's
    /// graph (plan-134-H) under the caller's control. `drop_replaced` is false only
    /// for a buffer that does not own its elements' graphs yet: a byte copy of
    /// another list (`copy_collection_tight`) whose payload edges still point at
    /// the source's children, and which the abi-inline dispatch deep-copies once
    /// the builtin returns (`own_collection_payload_edges`). Dropping there would
    /// free the source's graph.
    pub(crate) fn lower_list_set_in_place_with(
        &mut self,
        buffer_slot: usize,
        index_slot: usize,
        item_slot: usize,
        list_type: &ParameterType,
        element_type: &ParameterType,
        drop_replaced: bool,
        inline: Option<crate::codegen::collection::map::map_mutate::InlineGrow>,
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
        let scratch13 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        if CollectionTypeLayout::from_type(list_type).is_none() {
            return Err(format!(
                "native code collection type '{list_type}' is not supported"
            ));
        }
        for register in [
            scratch20, scratch21, scratch22, scratch23, scratch24, scratch25,
        ] {
            self.mark_register_used(&register.render());
        }
        let item = PayloadSlot {
            slot: item_slot,
            type_: element_type.clone(),
        };
        // Byte size of the replacement payload.
        let need_slot = self.emit_payload_length_to_stack(&item, "set_inplace_need")?;
        let voffset_slot = self.allocate_stack_object("set_inplace_voff", 8);

        let valid = self.label("set_inplace_valid");
        let invalid = self.label("set_inplace_invalid");
        let rebuild = self.label("set_inplace_rebuild");
        let done = self.label("set_inplace_done");
        // bug-627: the length-changing arm and its cases.
        let resize = self.label("set_inplace_resize");
        let narrow_keep = self.label("set_inplace_narrow");
        let widen = self.label("set_inplace_longer");
        let extend = self.label("set_inplace_extend");
        let relocate = self.label("set_inplace_relocate");
        let relocated = self.label("set_inplace_relocated");
        let overflow = self.label("set_inplace_overflow");
        let resize_write = self.label("set_inplace_resize_write");

        // Bounds check: 0 <= index < count.
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), index_slot));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch8,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::compare_immediate(&scratch10, "0"));
        self.emit(abi::branch_ge(&valid));
        self.emit(abi::branch(&invalid));
        self.emit(abi::label(&valid));
        self.emit(abi::compare_registers(&scratch10, &scratch11));
        self.emit(abi::branch_ge(&invalid));
        // plan-134-H: every path below discards the element at `index`, so the graph it owns
        // is freed first; the call clobbers the buffer and index registers, so reload them.
        if drop_replaced && self.owns_graph(element_type) {
            self.emit_drop_list_element(buffer_slot, index_slot, element_type)?;
            self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
            self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), index_slot));
        }

        // entry = buffer + HEADER + index * ENTRY; read valueOffset / valueLength.
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
        if let Some(payload) = kind2_payload_size(element_type) {
            // kind 2: the payload is at `index * payloadSize` and is always the
            // same size as its replacement, so the rebuild branch below is
            // unreachable and there is no entry to read (plan-57-D).
            self.emit(abi::move_immediate(
                &scratch13,
                "Integer",
                &payload.to_string(),
            ));
            self.emit(abi::multiply_registers(&scratch13, &scratch10, &scratch13));
            self.emit(abi::store_u64(
                &scratch13,
                abi::stack_pointer(),
                voffset_slot,
            ));
            self.emit(abi::move_immediate(
                &scratch9,
                "Integer",
                &payload.to_string(),
            ));
        } else {
            self.emit(abi::load_u64(
                &scratch13,
                &scratch12,
                COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
            ));
            self.emit(abi::store_u64(
                &scratch13,
                abi::stack_pointer(),
                voffset_slot,
            ));
            self.emit(abi::load_u64(
                &scratch9,
                &scratch12,
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            )); // oldLen
        }
        self.emit(abi::load_u64(&scratch14, abi::stack_pointer(), need_slot)); // need
                                                                               // A same-size replacement overwrites where it lies: offsets unchanged, no
                                                                               // gap, nothing to fix up. That is the fast path below; a length change
                                                                               // takes `resize`.
        self.emit(abi::compare_registers(&scratch14, &scratch9));
        self.emit(abi::branch_ne(&resize));

        // --- Overwrite: same-size payload at valueOffset (valueLength unchanged). ---
        self.emit_copy_payload_to_collection(
            buffer_slot,
            need_slot,
            &item,
            voffset_slot,
            element_type,
        )?;
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), index_slot));
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
        self.emit(abi::load_u64(&scratch14, abi::stack_pointer(), need_slot));
        if entry_stride != 0 {
            self.emit(abi::store_u64(
                &scratch14,
                &scratch12,
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ));
        }
        self.emit(abi::branch(&done));

        // --- bug-627: a length change, without touching any other element. ---
        //
        // plan-121-F shifted every byte after the old span and fixed up every later
        // entry's offset: O(N) per write, so widening every element of a list front
        // to back was O(N²) (6.69 s for 100,000 writes). Instead:
        //
        // * shorter — overwrite where it lies; the unused bytes stay as a hole;
        // * longer, and the payload is the last in the data region — grow it where
        //   it lies;
        // * longer, anywhere else — write it at the aligned data tail and repoint
        //   the entry; the old span becomes a hole.
        //
        // Every reader locates a variable-width payload by its own entry, never by
        // position or order (`mfb spec memory collections`, Payload Order), so holes
        // and permuted payloads are invisible. A tail write that does not fit
        // repacks (`emit_repack_list_data`), which is what bounds the holes.
        //
        // Entry-free (kind-2) lists never reach here: their payload is always
        // replaced by one of its own size, so the same-size branch above always
        // takes them (the `kind2_payload_size` arm sets `oldLen` to the fixed
        // payload size, which equals `need`).
        self.emit(abi::label(&resize));
        if entry_stride != 0 {
            let value_alignment = self.list_element_padding_alignment(element_type);
            let voff = self.temporary_vreg();
            let oldlen = self.temporary_vreg();
            let need = self.temporary_vreg();
            let dlen = self.temporary_vreg();
            let dcap = self.temporary_vreg();
            let end = self.temporary_vreg();
            let align_scratch = self.temporary_vreg();
            let newdlen_slot = self.allocate_stack_object("set_inplace_newdlen", 8);
            let oldlen_slot = self.allocate_stack_object("set_inplace_oldlen", 8);

            self.emit(abi::store_u64(&scratch9, abi::stack_pointer(), oldlen_slot));
            self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
            self.emit(abi::load_u64(
                &dlen,
                &scratch8,
                COLLECTION_OFFSET_DATA_LENGTH,
            ));
            self.emit(abi::load_u64(
                &dcap,
                &scratch8,
                COLLECTION_OFFSET_DATA_CAPACITY,
            ));
            self.emit(abi::load_u64(&voff, abi::stack_pointer(), voffset_slot));
            self.emit(abi::load_u64(&oldlen, abi::stack_pointer(), oldlen_slot));
            self.emit(abi::load_u64(&need, abi::stack_pointer(), need_slot));
            self.emit(abi::compare_registers(&need, &oldlen));
            self.emit(abi::branch_hi(&widen));

            // Shorter: same offset, same dataLength.
            self.emit(abi::label(&narrow_keep));
            self.emit(abi::store_u64(&dlen, abi::stack_pointer(), newdlen_slot));
            self.emit(abi::branch(&resize_write));

            // Longer: grow in place only when nothing lies after the old span.
            self.emit(abi::label(&widen));
            self.emit(abi::add_registers(&end, &voff, &oldlen));
            self.emit(abi::compare_registers(&end, &dlen));
            self.emit(abi::branch_ne(&relocate));
            self.emit(abi::label(&extend));
            self.emit(abi::add_registers(&end, &voff, &need));
            self.emit(abi::store_u64(&end, abi::stack_pointer(), newdlen_slot));
            self.emit(abi::compare_registers(&end, &dcap));
            self.emit(abi::branch_ls(&resize_write));
            self.emit(abi::branch_eq(&resize_write));
            self.emit(abi::branch(&overflow));

            // Longer, elsewhere: the aligned data tail, if it fits.
            self.emit(abi::label(&relocate));
            self.emit(abi::move_register(&end, &dlen));
            self.emit_align_offset_register(&end, value_alignment, &align_scratch);
            self.emit(abi::store_u64(&end, abi::stack_pointer(), voffset_slot));
            self.emit(abi::add_registers(&end, &end, &need));
            self.emit(abi::store_u64(&end, abi::stack_pointer(), newdlen_slot));
            self.emit(abi::compare_registers(&end, &dcap));
            self.emit(abi::branch_ls(&relocated));
            self.emit(abi::branch_eq(&relocated));

            // No room: drop this element's old payload from the repack (its bytes
            // are being replaced), repack into a geometrically larger block, and
            // write at the new tail.
            self.emit(abi::label(&overflow));
            self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
            self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), index_slot));
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
            self.emit(abi::store_u64(
                abi::ZERO,
                &scratch12,
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ));
            // bug-759: a list inlined in a record or `STATE` field grows the RECORD
            // (`InlineGrow`); `buffer_slot` then holds the moved sub-block.
            self.emit_repack_list_data(
                buffer_slot,
                need_slot,
                list_type,
                element_type,
                entry_stride,
                inline,
            )?;
            self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
            self.emit(abi::load_u64(
                &end,
                &scratch8,
                COLLECTION_OFFSET_DATA_LENGTH,
            ));
            self.emit(abi::store_u64(&end, abi::stack_pointer(), voffset_slot));
            self.emit(abi::load_u64(&need, abi::stack_pointer(), need_slot));
            self.emit(abi::add_registers(&end, &end, &need));
            self.emit(abi::store_u64(&end, abi::stack_pointer(), newdlen_slot));

            // Relocated (with or without a repack): repoint the entry.
            self.emit(abi::label(&relocated));
            self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
            self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), index_slot));
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
            self.emit(abi::load_u64(&voff, abi::stack_pointer(), voffset_slot));
            self.emit(abi::store_u64(
                &voff,
                &scratch12,
                COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
            ));

            // --- Every case: write the payload, then dataLength and valueLength. ---
            self.emit(abi::label(&resize_write));
            self.emit_copy_payload_to_collection(
                buffer_slot,
                need_slot,
                &item,
                voffset_slot,
                element_type,
            )?;
            self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
            self.emit(abi::load_u64(
                &scratch14,
                abi::stack_pointer(),
                newdlen_slot,
            ));
            self.emit(abi::store_u64(
                &scratch14,
                &scratch8,
                COLLECTION_OFFSET_DATA_LENGTH,
            ));
            self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), index_slot));
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
            self.emit(abi::load_u64(&scratch14, abi::stack_pointer(), need_slot));
            self.emit(abi::store_u64(
                &scratch14,
                &scratch12,
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ));
            self.emit(abi::branch(&done));
        } else {
            // Entry-free lists cannot reach the length-change path (see above),
            // but the label must still be defined and must not fall through.
            self.emit(abi::branch(&rebuild));
        }

        // --- Rebuild (payload grew): remove + insert a fresh singleton list. ---
        self.emit(abi::label(&rebuild));
        // Snapshot the original buffer pointer before the rebuild allocates over
        // `buffer_slot`. This path abandons three blocks — the original buffer,
        // the singleton, and the removeAt intermediate — and, because the in-place
        // short-circuit bypassed the general-reassignment free, it owns freeing all
        // of them (bug-47). The singleton + removeAt intermediates are freed exactly
        // as the non-in-place `lower_collection_set` frees them (:314-318); the
        // original buffer additionally, since no scope-drop reclaims it here.
        let orig_slot = self.allocate_stack_object("set_inplace_orig", 8);
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), buffer_slot));
        self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), orig_slot));
        let singleton = self.lower_collection_values(
            list_type,
            vec![CollectionValueSlot {
                key: None,
                value: PayloadSlot {
                    slot: item_slot,
                    type_: element_type.clone(),
                },
            }],
            "set rebuild singleton",
        )?;
        let singleton_slot = self.allocate_stack_object("set_inplace_singleton", 8);
        self.emit(abi::store_u64(
            &singleton.location,
            abi::stack_pointer(),
            singleton_slot,
        ));
        let removed =
            self.lower_list_remove_at(buffer_slot, index_slot, list_type, element_type)?;
        let removed_slot = self.allocate_stack_object("set_inplace_removed", 8);
        self.emit(abi::store_u64(
            &removed.location,
            abi::stack_pointer(),
            removed_slot,
        ));
        let rebuilt = self.lower_list_insert_collection(
            removed_slot,
            index_slot,
            singleton_slot,
            list_type,
            element_type,
        )?;
        self.emit(abi::store_u64(
            &rebuilt.location,
            abi::stack_pointer(),
            buffer_slot,
        ));
        // `rebuilt` (now in buffer_slot) is a fresh independent block holding copies
        // of all live data, so the three abandoned blocks are unreachable and
        // uniquely owned — free them. Each is a distinct block (insert_collection
        // allocates fresh; the non-in-place set frees the same singleton/removed
        // pair), so there is no double-free. The threaded result is discarded: the
        // function reloads the (unchanged) buffer_slot after `done`.
        let keep = self.allocate_register();
        self.emit(abi::load_u64(&keep, abi::stack_pointer(), buffer_slot));
        let threaded = ValueResult {
            origin: None,
            type_: list_type.clone(),
            location: Operand::from(keep),
            text: String::new(),
        };
        let threaded = self.free_intermediate_collection(singleton_slot, list_type, threaded)?;
        let threaded = self.free_intermediate_collection(removed_slot, list_type, threaded)?;
        let _ = self.free_intermediate_collection(orig_slot, list_type, threaded)?;
        self.emit(abi::branch(&done));

        self.emit(abi::label(&invalid));
        self.raise_error("collections.set", "ErrIndexOutOfRange")?;
        self.emit(abi::label(&done));

        let result = self.allocate_register();
        self.emit(abi::load_u64(&result, abi::stack_pointer(), buffer_slot));
        Ok(ValueResult {
            origin: None,
            type_: list_type.clone(),
            location: Operand::from(result),
            text: format!("set in place {list_type} over {element_type}"),
        })
    }
}
