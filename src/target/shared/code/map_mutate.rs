use super::*;

impl CodeBuilder<'_> {
    /// Set `key -> value` in the map whose buffer pointer lives in `map_slot`,
    /// **mutating the buffer in place** (plan-02 §4.3). Linear-scans for the key
    /// (linear scan): on a hit, overwrites the value bytes when the new value fits
    /// the old slot (`newLen <= oldLen`), else appends the new value to the spare
    /// data tail and repoints the entry (old value becomes dead slack, tightened on
    /// copy — amortized O(1) per set even when values grow). On a miss, writes the
    /// key+value into a
    /// spare lookup slot and the spare data tail (the entry packed exactly like
    /// `emit_write_collection_entry` — key then value, each aligned), bumping
    /// `count`/`dataLength`; when the live buffer is full it grows geometrically
    /// (copying entries + data verbatim, capacity-based base) and then writes. The
    /// caller guarantees unique ownership and not an active `FOR EACH` iterable.
    pub(super) fn lower_map_set_in_place(
        &mut self,
        map_slot: usize,
        key_slot: usize,
        value_slot: usize,
        map_type: &str,
        key_type: &str,
        value_type: &str,
    ) -> Result<ValueResult, String> {
        let scratch20 = self.temporary_vreg();
        let scratch21 = self.temporary_vreg();
        let scratch22 = self.temporary_vreg();
        let scratch23 = self.temporary_vreg();
        let scratch24 = self.temporary_vreg();
        let scratch25 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch16 = self.temporary_vreg();
        let scratch8 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch15 = self.temporary_vreg();
        let scratch17 = self.temporary_vreg();
        let layout = CollectionTypeLayout::from_type(map_type)
            .ok_or_else(|| format!("native code collection type '{map_type}' is not supported"))?;
        for register in [
            scratch20.as_str(),
            scratch21.as_str(),
            scratch22.as_str(),
            scratch23.as_str(),
            scratch24.as_str(),
            scratch25.as_str(),
        ] {
            self.mark_register_used(register);
        }
        let key_align = self.collection_payload_alignment(key_type);
        let value_align = self.collection_payload_alignment(value_type);
        let key_payload = PayloadSlot {
            slot: key_slot,
            type_: key_type.to_string(),
        };
        let value_payload = PayloadSlot {
            slot: value_slot,
            type_: value_type.to_string(),
        };
        let key_len_slot = self.emit_payload_length_to_stack(&key_payload, "mapset_klen")?;
        let val_len_slot = self.emit_payload_length_to_stack(&value_payload, "mapset_vlen")?;
        let found_entry_slot = self.allocate_stack_object("mapset_found_entry", 8);
        let found_index_slot = self.allocate_stack_object("mapset_found_index", 8);
        let new_data_len_slot = self.allocate_stack_object("mapset_newdlen", 8);
        let new_cap_slot = self.allocate_stack_object("mapset_newcap", 8);
        let new_dcap_slot = self.allocate_stack_object("mapset_newdcap", 8);
        let new_buf_slot = self.allocate_stack_object("mapset_newbuf", 8);
        let data_offset_slot = self.allocate_stack_object("mapset_doff", 8);
        let voff_slot = self.allocate_stack_object("mapset_voff", 8);
        let entry_addr_slot = self.allocate_stack_object("mapset_entry_addr", 8);

        let loop_label = self.label("mapset_loop");
        let next = self.label("mapset_next");
        let found = self.label("mapset_found");
        let not_found = self.label("mapset_not_found");
        let value_grow = self.label("mapset_value_grow");
        let vgrow = self.label("mapset_vgrow");
        let vwrite = self.label("mapset_vwrite");
        let valloc_ok = self.label("mapset_valloc_ok");
        let vdcap_keep = self.label("mapset_vdcap_keep");
        let grow = self.label("mapset_grow");
        let write = self.label("mapset_write");
        let alloc_ok = self.label("mapset_alloc_ok");
        let dcap_keep = self.label("mapset_dcap_keep");
        let done = self.label("mapset_done");

        // --- Locate the key: O(1) hash probe for eligible key types, else the
        // linear scan. Both store the found entry address + index and branch to
        // `found_handle`, or branch to `not_found`. The probe also lazily builds the
        // bucket index, so a build-via-`set` loop stays O(n). ---
        let found_handle = self.label("mapset_found_handle");
        if Self::map_key_probe_eligible(key_type) {
            let entry_slot = self.emit_map_probe(map_slot, key_slot, key_type, &not_found)?;
            // emit_map_probe already stored the entry address; record the index too
            // (x0 held it before the entry-address math, so recompute it from the
            // entry address: index = (entry - map - HEADER) / ENTRY).
            self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), entry_slot));
            self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), map_slot));
            self.emit(abi::subtract_registers(&scratch9, &scratch9, &scratch10));
            self.emit(abi::subtract_immediate(
                &scratch9,
                &scratch9,
                COLLECTION_HEADER_SIZE,
            ));
            self.emit(abi::move_immediate(
                &scratch16,
                "Integer",
                &COLLECTION_ENTRY_SIZE.to_string(),
            ));
            self.emit(abi::unsigned_divide_registers(
                &scratch9, &scratch9, &scratch16,
            ));
            self.emit(abi::store_u64(
                &scratch9,
                abi::stack_pointer(),
                found_index_slot,
            ));
            self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), entry_slot));
            self.emit(abi::store_u64(
                &scratch9,
                abi::stack_pointer(),
                found_entry_slot,
            ));
            self.emit(abi::branch(&found_handle));
        } else {
            self.reset_temporary_registers();
            let collection = self.allocate_register()?;
            let key = self.allocate_register()?;
            let count = self.allocate_register()?;
            let index = self.allocate_register()?;
            let entry = self.allocate_register()?;
            let key_offset = self.allocate_register()?;
            let key_length = self.allocate_register()?;
            self.emit(abi::load_u64(&collection, abi::stack_pointer(), map_slot));
            self.emit(abi::load_u64(&key, abi::stack_pointer(), key_slot));
            self.emit(abi::load_u64(&count, &collection, COLLECTION_OFFSET_COUNT));
            self.emit(abi::move_immediate(&index, "Integer", "0"));
            self.emit(abi::add_immediate(
                &entry,
                &collection,
                COLLECTION_HEADER_SIZE,
            ));
            self.emit(abi::label(&loop_label));
            self.emit(abi::compare_registers(&index, &count));
            self.emit(abi::branch_ge(&not_found));
            self.emit(abi::load_u64(
                &key_offset,
                &entry,
                COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
            ));
            self.emit(abi::load_u64(
                &key_length,
                &entry,
                COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
            ));
            self.emit_collection_payload_match_branch(
                key_type,
                "",
                &collection,
                &key_offset,
                &key_length,
                &key,
                &found,
                &next,
            )?;
            self.emit(abi::label(&next));
            self.emit(abi::add_immediate(&entry, &entry, COLLECTION_ENTRY_SIZE));
            self.emit(abi::add_immediate(&index, &index, 1));
            self.emit(abi::branch(&loop_label));
            self.emit(abi::label(&found));
            self.emit(abi::store_u64(
                &entry,
                abi::stack_pointer(),
                found_entry_slot,
            ));
            self.emit(abi::store_u64(
                &index,
                abi::stack_pointer(),
                found_index_slot,
            ));
            self.emit(abi::branch(&found_handle));
        }

        // --- Found handling (shared): overwrite the value when it fits, else
        // append-and-repoint. Slot-based so it serves both the probe and scan. ---
        self.emit(abi::label(&found_handle));
        self.emit(abi::load_u64(
            &scratch8,
            abi::stack_pointer(),
            found_entry_slot,
        ));
        self.emit(abi::load_u64(
            &scratch9,
            &scratch8,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        )); // oldValLen
        self.emit(abi::load_u64(
            &scratch14,
            abi::stack_pointer(),
            val_len_slot,
        )); // newValLen
        self.emit(abi::compare_registers(&scratch14, &scratch9));
        self.emit(abi::branch_hi(&value_grow)); // newLen > oldLen → append + repoint
        self.emit(abi::load_u64(
            &scratch8,
            abi::stack_pointer(),
            found_entry_slot,
        ));
        self.emit(abi::load_u64(
            &scratch13,
            &scratch8,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::store_u64(&scratch13, abi::stack_pointer(), voff_slot));
        self.emit_copy_payload_to_collection(
            map_slot,
            val_len_slot,
            &value_payload,
            voff_slot,
            "",
        )?;
        self.emit(abi::load_u64(
            &scratch8,
            abi::stack_pointer(),
            found_entry_slot,
        ));
        self.emit(abi::load_u64(
            &scratch14,
            abi::stack_pointer(),
            val_len_slot,
        ));
        self.emit(abi::store_u64(
            &scratch14,
            &scratch8,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::branch(&done));

        // --- Value grew: append the new value to the spare data tail and repoint
        // the entry's valueOffset/valueLength; the old value bytes become dead
        // slack (tightened away on copy, which copies dataLength verbatim). The
        // key, the lookup entry, and `count` are untouched — only the data region
        // grows, geometrically, when there is no headroom. This keeps a map whose
        // values grow (e.g. groupBy's per-key bucket list) amortized O(1) per set
        // instead of the O(map size) remove+concat rebuild. ---
        self.emit(abi::label(&value_grow));
        // newValOffset = align(dataLength, value_align); newDataLength += valLen.
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(
            &scratch9,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::store_u64(
            &scratch9,
            abi::stack_pointer(),
            new_data_len_slot,
        ));
        self.emit_align_offset_slot(new_data_len_slot, value_align);
        self.emit(abi::load_u64(
            &scratch8,
            abi::stack_pointer(),
            new_data_len_slot,
        ));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), val_len_slot));
        self.emit(abi::add_registers(&scratch8, &scratch8, &scratch9));
        self.emit(abi::store_u64(
            &scratch8,
            abi::stack_pointer(),
            new_data_len_slot,
        ));
        // Room: newDataLength <= dataCapacity?
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch8,
            COLLECTION_OFFSET_DATA_CAPACITY,
        ));
        self.emit(abi::load_u64(
            &scratch12,
            abi::stack_pointer(),
            new_data_len_slot,
        ));
        self.emit(abi::compare_registers(&scratch12, &scratch11));
        self.emit(abi::branch_hi(&vgrow));
        self.emit(abi::branch(&vwrite));

        // Grow the data region only (capacity unchanged); copy entries + data
        // verbatim against the capacity-based base, then repoint.
        self.emit(abi::label(&vgrow));
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
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
            "mapset_vgrow_dcap",
        );
        self.emit(abi::load_u64(
            &scratch11,
            abi::stack_pointer(),
            new_data_len_slot,
        ));
        self.emit(abi::compare_registers(&scratch14, &scratch11));
        self.emit(abi::branch_hi(&vdcap_keep));
        self.emit(abi::branch_eq(&vdcap_keep));
        self.emit(abi::move_register(&scratch14, &scratch11));
        self.emit(abi::label(&vdcap_keep));
        self.emit(abi::store_u64(
            &scratch14,
            abi::stack_pointer(),
            new_dcap_slot,
        ));
        // alloc = HEADER + capacity * ENTRY + newDataCapacity (capacity unchanged).
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(
            &scratch14,
            &scratch8,
            COLLECTION_OFFSET_CAPACITY,
        ));
        let size_overflow = self.label("map_vgrow_size_overflow");
        self.emit(abi::move_immediate(
            &scratch16,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
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
        // Reserve the map hash bucket region (x14 = capacity, unchanged on vgrow).
        self.emit_reserve_map_buckets(true, &scratch14, abi::return_register(), &scratch16);
        self.emit(abi::move_immediate(abi::ARG[1], "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&valloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&size_overflow));
        self.emit_error_code_return(ERR_OUT_OF_MEMORY_CODE, ERR_ALLOCATION_MESSAGE)?;
        self.emit(abi::label(&valloc_ok));
        self.emit(abi::store_u64(
            abi::RET[1],
            abi::stack_pointer(),
            new_buf_slot,
        ));
        // Header: old count / old dataLength, same capacity, new data capacity.
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(
            &scratch14,
            &scratch8,
            COLLECTION_OFFSET_CAPACITY,
        ));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
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
        self.emit_collection_data_pointer_for(&scratch17, &nb, "");
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
        self.emit_collection_data_pointer_for(&scratch20, &scratch8, "");
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
            "mapset_vgrow_data",
        );
        // Copy the lookup entries verbatim (count * ENTRY bytes).
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), new_buf_slot));
        self.emit(abi::add_immediate(&scratch17, &nb, COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
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
            "mapset_vgrow_entries",
        );
        // Free the abandoned pre-grow buffer (still in `map_slot`, sized with its
        // bucket region) before installing the grown one — otherwise a value-growing
        // map-set in a loop leaks the old buffer on every grow (bug-47).
        self.emit_free_pre_grow_buffer(map_slot, map_type)?;
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), new_buf_slot));
        self.emit(abi::store_u64(&nb, abi::stack_pointer(), map_slot));
        self.emit(abi::branch(&vwrite));

        // Write the new value at the aligned data tail; repoint the found entry.
        self.emit(abi::label(&vwrite));
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(
            &scratch9,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::store_u64(
            &scratch9,
            abi::stack_pointer(),
            data_offset_slot,
        ));
        self.emit_align_offset_slot(data_offset_slot, value_align);
        // entryAddr = map + HEADER + foundIndex * ENTRY (the buffer may have moved).
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(
            &scratch9,
            abi::stack_pointer(),
            found_index_slot,
        ));
        self.emit(abi::move_immediate(
            &scratch16,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch13, &scratch9, &scratch16));
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch8,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(&scratch12, &scratch12, &scratch13));
        self.emit(abi::store_u64(
            &scratch12,
            abi::stack_pointer(),
            entry_addr_slot,
        ));
        // valueOffset = aligned data offset, valueLength = newValLen.
        self.emit(abi::load_u64(
            &scratch13,
            abi::stack_pointer(),
            data_offset_slot,
        ));
        self.emit(abi::store_u64(
            &scratch13,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            &scratch13,
            abi::stack_pointer(),
            val_len_slot,
        ));
        self.emit(abi::store_u64(
            &scratch13,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit_copy_payload_to_collection(
            map_slot,
            val_len_slot,
            &value_payload,
            data_offset_slot,
            "",
        )?;
        // dataLength = final data offset (includes the alignment pad + new value).
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(
            &scratch9,
            abi::stack_pointer(),
            data_offset_slot,
        ));
        self.emit(abi::store_u64(
            &scratch9,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::branch(&done));

        // --- Not found: compute the would-be new dataLength after the insert. ---
        self.emit(abi::label(&not_found));
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(
            &scratch9,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::store_u64(
            &scratch9,
            abi::stack_pointer(),
            new_data_len_slot,
        ));
        self.emit_align_offset_slot(new_data_len_slot, key_align);
        self.emit(abi::load_u64(
            &scratch8,
            abi::stack_pointer(),
            new_data_len_slot,
        ));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), key_len_slot));
        self.emit(abi::add_registers(&scratch8, &scratch8, &scratch9));
        self.emit(abi::store_u64(
            &scratch8,
            abi::stack_pointer(),
            new_data_len_slot,
        ));
        self.emit_align_offset_slot(new_data_len_slot, value_align);
        self.emit(abi::load_u64(
            &scratch8,
            abi::stack_pointer(),
            new_data_len_slot,
        ));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), val_len_slot));
        self.emit(abi::add_registers(&scratch8, &scratch8, &scratch9));
        self.emit(abi::store_u64(
            &scratch8,
            abi::stack_pointer(),
            new_data_len_slot,
        ));
        // Room check: count < capacity AND newDataLength <= dataCapacity.
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(
            &scratch10,
            &scratch8,
            COLLECTION_OFFSET_CAPACITY,
        ));
        self.emit(abi::compare_registers(&scratch9, &scratch10));
        self.emit(abi::branch_ge(&grow));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch8,
            COLLECTION_OFFSET_DATA_CAPACITY,
        ));
        self.emit(abi::load_u64(
            &scratch12,
            abi::stack_pointer(),
            new_data_len_slot,
        ));
        self.emit(abi::compare_registers(&scratch12, &scratch11));
        self.emit(abi::branch_hi(&grow));
        self.emit(abi::branch(&write));

        // --- Grow: geometric capacity + data, copy entries/data verbatim. ---
        self.emit(abi::label(&grow));
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
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
            "mapset_grow_cap",
        );
        self.emit(abi::store_u64(
            &scratch14,
            abi::stack_pointer(),
            new_cap_slot,
        ));
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
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
            "mapset_grow_dcap",
        );
        // newDataCapacity = max(step(dataCapacity), newDataLength).
        self.emit(abi::load_u64(
            &scratch11,
            abi::stack_pointer(),
            new_data_len_slot,
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
        // alloc = HEADER + newCapacity * ENTRY + newDataCapacity.
        self.emit(abi::load_u64(
            &scratch14,
            abi::stack_pointer(),
            new_cap_slot,
        ));
        let size_overflow = self.label("map_grow_size_overflow");
        self.emit(abi::move_immediate(
            &scratch16,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
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
        // Reserve the map hash bucket region (x14 = new capacity).
        self.emit_reserve_map_buckets(true, &scratch14, abi::return_register(), &scratch16);
        self.emit(abi::move_immediate(abi::ARG[1], "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&size_overflow));
        self.emit_error_code_return(ERR_OUT_OF_MEMORY_CODE, ERR_ALLOCATION_MESSAGE)?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(
            abi::RET[1],
            abi::stack_pointer(),
            new_buf_slot,
        ));
        // Header: old count / old dataLength, new capacity / data capacity.
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
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
        self.emit_collection_data_pointer_for(&scratch17, &nb, "");
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
        self.emit_collection_data_pointer_for(&scratch20, &scratch8, "");
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
            "mapset_grow_data",
        );
        // Copy the live lookup entries verbatim (count * ENTRY bytes).
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), new_buf_slot));
        self.emit(abi::add_immediate(&scratch17, &nb, COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
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
            "mapset_grow_entries",
        );
        // Free the abandoned pre-grow buffer (still in `map_slot`, sized with its
        // bucket region) before installing the grown one — otherwise a capacity-
        // growing map-set in a loop leaks the old buffer on every grow (bug-47).
        self.emit_free_pre_grow_buffer(map_slot, map_type)?;
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), new_buf_slot));
        self.emit(abi::store_u64(&nb, abi::stack_pointer(), map_slot));
        self.emit(abi::branch(&write));

        // --- Write the new entry into slot[count], key+value aligned in data. ---
        self.emit(abi::label(&write));
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(
            &scratch9,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::store_u64(
            &scratch9,
            abi::stack_pointer(),
            data_offset_slot,
        ));
        // entryAddr = map + HEADER + count * ENTRY.
        self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::move_immediate(
            &scratch16,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch13, &scratch9, &scratch16));
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch8,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(&scratch12, &scratch12, &scratch13));
        self.emit(abi::store_u64(
            &scratch12,
            abi::stack_pointer(),
            entry_addr_slot,
        ));
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
        // Key: align, record keyOffset/keyLength, copy bytes.
        self.emit_align_offset_slot(data_offset_slot, key_align);
        self.emit(abi::load_u64(
            &scratch12,
            abi::stack_pointer(),
            entry_addr_slot,
        ));
        self.emit(abi::load_u64(
            &scratch13,
            abi::stack_pointer(),
            data_offset_slot,
        ));
        self.emit(abi::store_u64(
            &scratch13,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::load_u64(
            &scratch13,
            abi::stack_pointer(),
            key_len_slot,
        ));
        self.emit(abi::store_u64(
            &scratch13,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit_copy_payload_to_collection(
            map_slot,
            key_len_slot,
            &key_payload,
            data_offset_slot,
            "",
        )?;
        // Value: align, record valueOffset/valueLength, copy bytes.
        self.emit_align_offset_slot(data_offset_slot, value_align);
        self.emit(abi::load_u64(
            &scratch12,
            abi::stack_pointer(),
            entry_addr_slot,
        ));
        self.emit(abi::load_u64(
            &scratch13,
            abi::stack_pointer(),
            data_offset_slot,
        ));
        self.emit(abi::store_u64(
            &scratch13,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            &scratch13,
            abi::stack_pointer(),
            val_len_slot,
        ));
        self.emit(abi::store_u64(
            &scratch13,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit_copy_payload_to_collection(
            map_slot,
            val_len_slot,
            &value_payload,
            data_offset_slot,
            "",
        )?;
        // Header: count++, dataLength = final data offset.
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(&scratch9, &scratch8, COLLECTION_OFFSET_COUNT));
        self.emit(abi::add_immediate(&scratch9, &scratch9, 1));
        self.emit(abi::store_u64(
            &scratch9,
            &scratch8,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::load_u64(
            &scratch9,
            abi::stack_pointer(),
            data_offset_slot,
        ));
        self.emit(abi::store_u64(
            &scratch9,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        // Keep the hash index current: if the buckets are already built (a prior
        // probe), insert the new entry incrementally so a build-via-`set` loop stays
        // O(n). The grow path reset the ready flag (the bucket region moved), so it
        // falls through here and is rebuilt lazily on the next probe. The 2*capacity
        // load factor guarantees a free slot for a spare-slot insert.
        let skip_put = self.label("mapset_skip_put");
        self.emit(abi::load_u8(
            &scratch9,
            &scratch8,
            COLLECTION_OFFSET_BUCKETS_READY,
        ));
        self.emit(abi::compare_immediate(&scratch9, "0"));
        self.emit(abi::branch_eq(&skip_put));
        self.emit(abi::load_u64(abi::ARG[0], abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(
            abi::ARG[1],
            abi::ARG[0],
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::subtract_immediate(abi::ARG[1], abi::ARG[1], 1)); // new entry index
        self.emit(abi::branch_link(MAP_BUCKET_PUT_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: MAP_BUCKET_PUT_SYMBOL.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::label(&skip_put));

        self.emit(abi::label(&done));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), map_slot));
        Ok(ValueResult {
            type_: map_type.to_string(),
            location: result,
            text: format!("map set in place {map_type}"),
        })
    }

    pub(super) fn lower_map_concat(
        &mut self,
        left_slot: usize,
        right_slot: usize,
        map_type: &str,
    ) -> Result<ValueResult, String> {
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
        let scratch15 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        let scratch16 = self.temporary_vreg();
        let scratch17 = self.temporary_vreg();
        let layout = CollectionTypeLayout::from_type(map_type)
            .ok_or_else(|| format!("native code collection type '{map_type}' is not supported"))?;
        let key_payload_align = collection_payload_alignment_for_code(layout.key_type_code);
        let value_payload_align = collection_payload_alignment_for_code(layout.value_type_code);
        let map_max_align = key_payload_align.max(value_payload_align);
        for register in [
            scratch20.as_str(),
            scratch21.as_str(),
            scratch22.as_str(),
            scratch23.as_str(),
            scratch24.as_str(),
            scratch25.as_str(),
        ] {
            self.mark_register_used(register);
        }
        let result_slot = self.allocate_stack_object("map_concat_result", 8);
        let alloc_ok = self.label("map_concat_alloc_ok");

        // Offset-stable merge (plan-01 §4.1): copy A's and B's data regions
        // verbatim — B placed at `align(dataLen_A, map_max_align)` so its packed
        // payloads keep their alignment relative to the new base — then concat the
        // lookup tables, shifting every B key/value offset by that same boundary.
        // The B boundary doubles as the per-entry offset shift.
        //
        // Size: HEADER + (count_A+count_B)*ENTRY + (align(dataLen_A)+dataLen_B).
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), left_slot));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), right_slot));
        self.emit(abi::load_u64(
            &scratch10,
            &scratch8,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch9,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::add_registers(&scratch12, &scratch10, &scratch11));
        self.emit(abi::load_u64(
            &scratch13,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit_align_offset_register(&scratch13, map_max_align, &scratch15);
        self.emit(abi::load_u64(
            &scratch14,
            &scratch9,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::add_registers(&scratch14, &scratch13, &scratch14));
        let size_overflow = self.label("map_concat_size_overflow");
        self.emit(abi::move_immediate(
            &scratch15,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        // Checked collection-size arithmetic (bug-147.7): the total count and both
        // data lengths come from live map headers, so guard count*ENTRY + HEADER +
        // dataLen against overflow before allocating.
        self.emit_checked_size_multiply(&scratch16, &scratch12, &scratch15, &size_overflow);
        self.emit_checked_size_add_immediate(
            abi::return_register(),
            &scratch16,
            COLLECTION_HEADER_SIZE,
            &size_overflow,
        );
        self.emit_checked_size_add(
            abi::return_register(),
            abi::return_register(),
            &scratch14,
            &size_overflow,
        );
        // Reserve the map hash bucket region (x12 = total count = capacity).
        self.emit_reserve_map_buckets(true, &scratch12, abi::return_register(), &scratch15);
        self.emit(abi::move_immediate(abi::ARG[1], "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&size_overflow));
        self.emit_error_code_return(ERR_OUT_OF_MEMORY_CODE, ERR_ALLOCATION_MESSAGE)?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(
            abi::RET[1],
            abi::stack_pointer(),
            result_slot,
        ));
        let nb = self.temporary_vreg();
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), result_slot));

        // Header: recompute total count / total data length from the pointer slots
        // (the pre-alloc registers do not survive `arena_alloc`).
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), left_slot));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), right_slot));
        self.emit(abi::load_u64(
            &scratch10,
            &scratch8,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch9,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::add_registers(&scratch12, &scratch10, &scratch11));
        self.emit(abi::load_u64(
            &scratch13,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit_align_offset_register(&scratch13, map_max_align, &scratch15);
        self.emit(abi::load_u64(
            &scratch14,
            &scratch9,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::add_registers(&scratch14, &scratch13, &scratch14));
        self.emit_write_list_header_from_registers(&layout, &nb, &scratch12, &scratch14);

        // --- Data region: A verbatim at base, B verbatim at align(dataLen_A). ---
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), result_slot));
        self.emit_collection_data_pointer_for(&scratch17, &nb, ""); // x17 = dst data base (stable)
        self.emit(abi::move_register(&scratch23, &scratch17)); // moving copy dst
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), left_slot));
        self.emit_collection_data_pointer_for(&scratch20, &scratch8, ""); // A data base
        self.emit(abi::load_u64(
            &scratch14,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit_block_copy_advance(
            &scratch23,
            &scratch20,
            &scratch14,
            &scratch22,
            "map_concat_dataA",
        );
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), left_slot));
        self.emit(abi::load_u64(
            &scratch13,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit_align_offset_register(&scratch13, map_max_align, &scratch22); // alignedA
        self.emit(abi::add_registers(&scratch23, &scratch17, &scratch13)); // B dest = base + alignedA
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), right_slot));
        self.emit_collection_data_pointer_for(&scratch20, &scratch9, ""); // B data base
        self.emit(abi::load_u64(
            &scratch15,
            &scratch9,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit_block_copy_advance(
            &scratch23,
            &scratch20,
            &scratch15,
            &scratch22,
            "map_concat_dataB",
        );

        // --- Lookup table: A entries verbatim, then B entries shifted. ---
        self.emit(abi::move_immediate(
            &scratch16,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), result_slot));
        self.emit(abi::add_immediate(&scratch17, &nb, COLLECTION_HEADER_SIZE)); // dst table cursor
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), left_slot));
        self.emit(abi::add_immediate(
            &scratch20,
            &scratch8,
            COLLECTION_HEADER_SIZE,
        )); // A table cursor
        self.emit(abi::load_u64(
            &scratch10,
            &scratch8,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::multiply_registers(&scratch21, &scratch10, &scratch16)); // count_A * ENTRY
        self.emit_block_copy_advance(
            &scratch17,
            &scratch20,
            &scratch21,
            &scratch22,
            "map_concat_tableA",
        );

        // B entries: keyOffset and valueOffset each += align(dataLen_A).
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), right_slot));
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch9,
            COLLECTION_HEADER_SIZE,
        )); // B table cursor
        self.emit(abi::load_u64(
            &scratch11,
            &scratch9,
            COLLECTION_OFFSET_COUNT,
        )); // remaining
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), left_slot));
        self.emit(abi::load_u64(
            &scratch14,
            &scratch8,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit_align_offset_register(&scratch14, map_max_align, &scratch22); // shift = alignedA
        let copy_loop = self.label("map_concat_b_loop");
        let copy_done = self.label("map_concat_b_done");
        self.emit(abi::label(&copy_loop));
        self.emit(abi::compare_immediate(&scratch11, "0"));
        self.emit(abi::branch_eq(&copy_done));
        self.emit(abi::move_immediate(
            &scratch22,
            "Byte",
            &COLLECTION_ENTRY_FLAG_USED.to_string(),
        ));
        self.emit(abi::store_u8(
            &scratch22,
            &scratch17,
            COLLECTION_ENTRY_OFFSET_FLAGS,
        ));
        self.emit(abi::load_u64(
            &scratch22,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::add_registers(&scratch22, &scratch22, &scratch14));
        self.emit(abi::store_u64(
            &scratch22,
            &scratch17,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::load_u64(
            &scratch22,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::store_u64(
            &scratch22,
            &scratch17,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::load_u64(
            &scratch22,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::add_registers(&scratch22, &scratch22, &scratch14));
        self.emit(abi::store_u64(
            &scratch22,
            &scratch17,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            &scratch22,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::store_u64(
            &scratch22,
            &scratch17,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_immediate(
            &scratch17,
            &scratch17,
            COLLECTION_ENTRY_SIZE,
        ));
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch12,
            COLLECTION_ENTRY_SIZE,
        ));
        self.emit(abi::subtract_immediate(&scratch11, &scratch11, 1));
        self.emit(abi::branch(&copy_loop));
        self.emit(abi::label(&copy_done));

        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: map_type.to_string(),
            location: result,
            text: format!("map concat {map_type}"),
        })
    }

    pub(super) fn lower_map_remove_key(
        &mut self,
        map_slot: usize,
        key_slot: usize,
        map_type: &str,
        key_type: &str,
    ) -> Result<ValueResult, String> {
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
        let scratch14 = self.temporary_vreg();
        let scratch15 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let scratch16 = self.temporary_vreg();
        let scratch17 = self.temporary_vreg();
        let layout = CollectionTypeLayout::from_type(map_type)
            .ok_or_else(|| format!("native code collection type '{map_type}' is not supported"))?;
        let key_payload_align = collection_payload_alignment_for_code(layout.key_type_code);
        let value_payload_align = collection_payload_alignment_for_code(layout.value_type_code);
        for register in [
            scratch20.as_str(),
            scratch21.as_str(),
            scratch22.as_str(),
            scratch23.as_str(),
            scratch24.as_str(),
            scratch25.as_str(),
        ] {
            self.mark_register_used(register);
        }
        let result_slot = self.allocate_stack_object("map_remove_result", 8);
        let count_slot = self.allocate_stack_object("map_remove_count", 8);
        let data_len_slot = self.allocate_stack_object("map_remove_data_len", 8);
        let scan_loop = self.label("map_remove_scan_loop");
        let scan_keep = self.label("map_remove_scan_keep");
        let scan_next = self.label("map_remove_scan_next");
        let scan_done = self.label("map_remove_scan_done");
        let alloc_ok = self.label("map_remove_alloc_ok");
        let copy_loop = self.label("map_remove_copy_loop");
        let copy_keep = self.label("map_remove_copy_keep");
        let copy_next = self.label("map_remove_copy_next");
        let copy_done = self.label("map_remove_copy_done");

        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), key_slot));
        self.emit(abi::load_u64(
            &scratch10,
            &scratch8,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::move_immediate(&scratch11, "Integer", "0"));
        self.emit(abi::move_immediate(&scratch14, "Integer", "0"));
        self.emit(abi::move_immediate(&scratch15, "Integer", "0"));
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch8,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::label(&scan_loop));
        self.emit(abi::compare_registers(&scratch11, &scratch10));
        self.emit(abi::branch_ge(&scan_done));
        self.emit(abi::load_u64(
            &scratch13,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::load_u64(
            &scratch16,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit_collection_payload_matches_value_branch(
            key_type, "", &scratch8, &scratch13, &scratch16, &scratch9, &scan_next, &scan_keep,
        )?;
        self.emit(abi::label(&scan_keep));
        self.emit(abi::add_immediate(&scratch14, &scratch14, 1));
        // Accumulate the retained data length with the same per-payload
        // alignment the copy phase applies, so the precomputed allocation
        // matches the bytes actually written.
        self.emit_align_offset_register(&scratch15, key_payload_align, &scratch16);
        self.emit(abi::load_u64(
            &scratch16,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::add_registers(&scratch15, &scratch15, &scratch16));
        self.emit_align_offset_register(&scratch15, value_payload_align, &scratch16);
        self.emit(abi::load_u64(
            &scratch17,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers(&scratch15, &scratch15, &scratch17));
        self.emit(abi::label(&scan_next));
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch12,
            COLLECTION_ENTRY_SIZE,
        ));
        self.emit(abi::add_immediate(&scratch11, &scratch11, 1));
        self.emit(abi::branch(&scan_loop));
        self.emit(abi::label(&scan_done));
        self.emit(abi::move_immediate(
            &scratch16,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch17, &scratch14, &scratch16));
        self.emit(abi::add_immediate(
            abi::return_register(),
            &scratch17,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(
            abi::return_register(),
            abi::return_register(),
            &scratch15,
        ));
        // `arena_alloc` clobbers both x14 and x15 in its block-grow path; persist
        // the retained count and data length so the header write below does not
        // store stale pointers.
        self.emit(abi::store_u64(&scratch14, abi::stack_pointer(), count_slot));
        self.emit(abi::store_u64(
            &scratch15,
            abi::stack_pointer(),
            data_len_slot,
        ));
        // Reserve the map hash bucket region (x14 = remaining count = capacity).
        self.emit_reserve_map_buckets(true, &scratch14, abi::return_register(), &scratch16);
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
        let nb = self.temporary_vreg();
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64(&scratch14, abi::stack_pointer(), count_slot));
        self.emit(abi::load_u64(
            &scratch15,
            abi::stack_pointer(),
            data_len_slot,
        ));
        self.emit_write_list_header_from_registers(&layout, &nb, &scratch14, &scratch15);

        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), key_slot));
        self.emit(abi::load_u64(&nb, abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64(
            &scratch10,
            &scratch8,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::move_immediate(&scratch11, "Integer", "0"));
        self.emit(abi::move_immediate(&scratch13, "Integer", "0"));
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch8,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_immediate(&scratch17, &nb, COLLECTION_HEADER_SIZE));
        self.emit_collection_data_pointer_for(&scratch20, &scratch8, "");
        self.emit(abi::load_u64(&scratch14, &nb, COLLECTION_OFFSET_COUNT));
        self.emit(abi::move_immediate(
            &scratch16,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch21, &scratch14, &scratch16));
        self.emit(abi::add_registers(&scratch21, &scratch17, &scratch21));
        self.emit(abi::label(&copy_loop));
        self.emit(abi::compare_registers(&scratch11, &scratch10));
        self.emit(abi::branch_ge(&copy_done));
        self.emit(abi::load_u64(
            &scratch14,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::load_u64(
            &scratch15,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit_collection_payload_matches_value_branch(
            key_type, "", &scratch8, &scratch14, &scratch15, &scratch9, &copy_next, &copy_keep,
        )?;
        self.emit(abi::label(&copy_keep));
        self.emit_copy_one_map_entry(
            &scratch12,
            &scratch20,
            &scratch17,
            &scratch21,
            &scratch13,
            key_payload_align,
            value_payload_align,
        );
        self.emit(abi::label(&copy_next));
        self.emit(abi::add_immediate(
            &scratch12,
            &scratch12,
            COLLECTION_ENTRY_SIZE,
        ));
        self.emit(abi::add_immediate(&scratch11, &scratch11, 1));
        self.emit(abi::branch(&copy_loop));
        self.emit(abi::label(&copy_done));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: map_type.to_string(),
            location: result,
            text: format!("removeKey({map_type}, {key_type})"),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn emit_copy_one_map_entry(
        &mut self,
        source_entry: &str,
        source_data: &str,
        dest_entry: &str,
        dest_data: &str,
        dest_data_offset: &str,
        key_align: usize,
        value_align: usize,
    ) {
        let scratch22 = self.temporary_vreg();
        let scratch23 = self.temporary_vreg();
        let scratch24 = self.temporary_vreg();
        let scratch25 = self.temporary_vreg();
        self.emit(abi::move_immediate(
            &scratch22,
            "Byte",
            &COLLECTION_ENTRY_FLAG_USED.to_string(),
        ));
        self.emit(abi::store_u8(
            &scratch22,
            dest_entry,
            COLLECTION_ENTRY_OFFSET_FLAGS,
        ));
        // Align the destination cursor to the key payload alignment before
        // recording its offset, matching the packing used when the map was
        // first built. Idempotent when the cursor is already aligned.
        self.emit_align_offset_register(dest_data_offset, key_align, &scratch22);
        self.emit(abi::load_u64(
            &scratch22,
            source_entry,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::load_u64(
            &scratch23,
            source_entry,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::store_u64(
            dest_data_offset,
            dest_entry,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::store_u64(
            &scratch23,
            dest_entry,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::add_registers(&scratch24, source_data, &scratch22));
        self.emit(abi::add_registers(&scratch25, dest_data, dest_data_offset));
        self.emit_block_copy_advance(
            &scratch25,
            &scratch24,
            &scratch23,
            &scratch22,
            "map_entry_key_copy",
        );
        self.emit(abi::load_u64(
            &scratch23,
            dest_entry,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::add_registers(
            dest_data_offset,
            dest_data_offset,
            &scratch23,
        ));

        // Align the destination cursor to the value payload alignment before
        // recording its offset.
        self.emit_align_offset_register(dest_data_offset, value_align, &scratch22);
        self.emit(abi::load_u64(
            &scratch22,
            source_entry,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            &scratch23,
            source_entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::store_u64(
            dest_data_offset,
            dest_entry,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::store_u64(
            &scratch23,
            dest_entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers(&scratch24, source_data, &scratch22));
        self.emit(abi::add_registers(&scratch25, dest_data, dest_data_offset));
        self.emit_block_copy_advance(
            &scratch25,
            &scratch24,
            &scratch23,
            &scratch22,
            "map_entry_value_copy",
        );
        self.emit(abi::load_u64(
            &scratch23,
            dest_entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers(
            dest_data_offset,
            dest_data_offset,
            &scratch23,
        ));
        self.emit(abi::add_immediate(
            dest_entry,
            dest_entry,
            COLLECTION_ENTRY_SIZE,
        ));
    }
}
