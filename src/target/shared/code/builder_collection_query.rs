use super::*;

impl CodeBuilder<'_> {
    pub(super) fn lower_list_get(
        &mut self,
        collection_slot: usize,
        key_slot: usize,
        collection_type: &str,
        element_type: &str,
    ) -> Result<ValueResult, String> {
        self.reset_temporary_registers();
        let collection = self.allocate_register()?;
        let index = self.allocate_register()?;
        let count = self.allocate_register()?;
        let entry_offset = self.allocate_register()?;
        let entry = self.allocate_register()?;
        let value_offset = self.allocate_register()?;
        let value_length = self.allocate_register()?;
        let invalid = self.label("list_get_invalid");
        let done = self.label("list_get_done");

        self.emit(abi::load_u64(
            &collection,
            abi::stack_pointer(),
            collection_slot,
        ));
        self.emit(abi::load_u64(&index, abi::stack_pointer(), key_slot));
        self.emit(abi::compare_immediate(&index, "0"));
        self.emit(abi::branch_lt(&invalid));
        self.emit(abi::load_u64(&count, &collection, COLLECTION_OFFSET_COUNT));
        self.emit(abi::compare_registers(&index, &count));
        self.emit(abi::branch_ge(&invalid));
        self.emit_element_value_offset(
            &value_offset,
            &value_length,
            &collection,
            &index,
            &entry_offset,
            &entry,
            element_type,
        );
        let result = self.emit_load_collection_payload(
            element_type,
            &collection,
            &value_offset,
            &value_length,
        )?;
        self.emit(abi::branch(&done));
        self.emit(abi::label(&invalid));
        self.emit_index_out_of_range_return()?;
        self.emit(abi::label(&done));

        Ok(ValueResult {
            type_: element_type.to_string(),
            location: result,
            text: format!("get({collection_type}, Integer)"),
        })
    }

    /// Whether a map key type uses the FNV-1a bucket probe (plan-02 Phase 6). The
    /// probe compares key bytes, which is exactly the linear scan's comparison for
    /// these types; other key types keep the scan.
    pub(super) fn map_key_probe_eligible(key_type: &str) -> bool {
        matches!(
            key_type,
            "String" | "Integer" | "Float" | "Fixed" | "Byte" | "Boolean"
        )
    }

    /// Materialize the query key as a (pointer in `x1`, byte length in `x2`) pair
    /// for the map probe — the same bytes `emit_copy_payload_to_collection` stored
    /// for the key. `String` keys point past the length word; fixed-width keys
    /// point at their stack slot.
    pub(super) fn emit_map_query_key(
        &mut self,
        key_type: &str,
        key_slot: usize,
    ) -> Result<(), String> {
        let scratch9 = self.temporary_vreg();
        match key_type {
            "String" => {
                self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), key_slot));
                self.emit(abi::load_u64(abi::ARG[2], &scratch9, 0));
                self.emit(abi::add_immediate(abi::ARG[1], &scratch9, 8));
            }
            "Boolean" | "Byte" => {
                self.emit(abi::add_immediate(
                    abi::ARG[1],
                    abi::stack_pointer(),
                    key_slot,
                ));
                self.emit(abi::move_immediate(abi::ARG[2], "Integer", "1"));
            }
            "Integer" | "Float" | "Fixed" => {
                self.emit(abi::add_immediate(
                    abi::ARG[1],
                    abi::stack_pointer(),
                    key_slot,
                ));
                self.emit(abi::move_immediate(abi::ARG[2], "Integer", "8"));
            }
            other => {
                return Err(format!(
                    "native map probe does not support key type '{other}'"
                ));
            }
        }
        Ok(())
    }

    /// Probe the map (pointer in `collection_slot`) for the key (in `key_slot`);
    /// branch to `not_found_label` when absent, otherwise store the matching entry
    /// address into a fresh stack slot and return its offset.
    ///
    /// plan-25-D §D1: the common case — the buckets are already built and the key
    /// hashes to its home slot with no collision — is inlined (FNV-1a hash +
    /// first-bucket probe + one key compare), so a lookup/`set` loop pays no `bl`
    /// to `_mfb_rt_map_probe` per operation. Only the slow paths (buckets not yet
    /// built, or a hash collision at the home slot) fall back to the runtime helper,
    /// which re-hashes and walks the full linear-probe chain. The inline arithmetic
    /// mirrors `lower_map_probe_helper` exactly, so the entry it resolves — and thus
    /// every observable map value and iteration order — is byte-identical.
    pub(super) fn emit_map_probe(
        &mut self,
        collection_slot: usize,
        key_slot: usize,
        key_type: &str,
        not_found_label: &str,
    ) -> Result<usize, String> {
        let entry_slot = self.allocate_stack_object("map_probe_entry", 8);
        let entry_size = COLLECTION_ENTRY_SIZE.to_string();

        // Materialize the query key bytes (x1 = ptr, x2 = len) and capture them
        // into vregs we own — used both for the inline hash/compare and, on the
        // collision/unbuilt fallback, to re-arm the helper's argument registers.
        self.emit_map_query_key(key_type, key_slot)?;
        let key_ptr = self.temporary_vreg();
        let key_len = self.temporary_vreg();
        self.emit(abi::move_register(&key_ptr, abi::ARG[1]));
        self.emit(abi::move_register(&key_len, abi::ARG[2]));

        let map = self.temporary_vreg();
        self.emit(abi::load_u64(&map, abi::stack_pointer(), collection_slot));

        let fallback = self.label("map_probe_fallback");
        let store_entry = self.label("map_probe_store");
        let done = self.label("map_probe_done");
        let hash_loop = self.label("map_probe_hloop");
        let hash_done = self.label("map_probe_hdone");
        let cmp_loop = self.label("map_probe_cloop");

        let scratch = self.temporary_vreg();
        // count == 0 -> absent (matches the helper's early-out).
        let count = self.temporary_vreg();
        self.emit(abi::load_u64(&count, &map, COLLECTION_OFFSET_COUNT));
        self.emit(abi::compare_immediate(&count, "0"));
        self.emit(abi::branch_eq(not_found_label));
        // Buckets not built yet (fresh/copied/grown map) -> the helper builds them.
        let ready = self.temporary_vreg();
        self.emit(abi::load_u8(&ready, &map, COLLECTION_OFFSET_BUCKETS_READY));
        self.emit(abi::compare_immediate(&ready, "0"));
        self.emit(abi::branch_eq(&fallback));

        // dataBase = map + HEADER + capacity*ENTRY; bucketBase = dataBase +
        // dataCapacity; bucketCount = capacity * 2.
        let capacity = self.temporary_vreg();
        self.emit(abi::load_u64(&capacity, &map, COLLECTION_OFFSET_CAPACITY));
        let data_base = self.temporary_vreg();
        self.emit(abi::move_immediate(&scratch, "Integer", &entry_size));
        self.emit(abi::multiply_registers(&data_base, &capacity, &scratch));
        self.emit(abi::add_registers(&data_base, &data_base, &map));
        self.emit(abi::add_immediate(
            &data_base,
            &data_base,
            COLLECTION_HEADER_SIZE,
        ));
        let bucket_base = self.temporary_vreg();
        self.emit(abi::load_u64(
            &bucket_base,
            &map,
            COLLECTION_OFFSET_DATA_CAPACITY,
        ));
        self.emit(abi::add_registers(&bucket_base, &bucket_base, &data_base));
        let bucket_count = self.temporary_vreg();
        self.emit(abi::shift_left_immediate(&bucket_count, &capacity, 1));

        // FNV-1a hash of the query key (bytewise), matching the helper.
        let prime = self.temporary_vreg();
        self.emit(abi::move_immediate(&prime, "Integer", FNV1A_PRIME));
        let hash = self.temporary_vreg();
        self.emit(abi::move_immediate(&hash, "Integer", FNV1A_BASIS));
        let cursor = self.temporary_vreg();
        let remaining = self.temporary_vreg();
        self.emit(abi::move_register(&cursor, &key_ptr));
        self.emit(abi::move_register(&remaining, &key_len));
        self.emit(abi::label(&hash_loop));
        self.emit(abi::compare_immediate(&remaining, "0"));
        self.emit(abi::branch_eq(&hash_done));
        let byte = self.temporary_vreg();
        self.emit(abi::load_u8(&byte, &cursor, 0));
        self.emit(abi::exclusive_or_registers(&hash, &hash, &byte));
        self.emit(abi::multiply_registers(&hash, &hash, &prime));
        self.emit(abi::add_immediate(&cursor, &cursor, 1));
        self.emit(abi::subtract_immediate(&remaining, &remaining, 1));
        self.emit(abi::branch(&hash_loop));
        self.emit(abi::label(&hash_done));

        // slot = hash mod bucketCount; bucket = buckets[slot] (0 => absent, since
        // linear-probe insertion fills the first empty slot from the home).
        let slot = self.temporary_vreg();
        self.emit(abi::unsigned_divide_registers(&slot, &hash, &bucket_count));
        self.emit(abi::multiply_subtract_registers(
            &slot,
            &slot,
            &bucket_count,
            &hash,
        ));
        let bucket = self.temporary_vreg();
        self.emit(abi::shift_left_immediate(&bucket, &slot, 3));
        self.emit(abi::add_registers(&bucket, &bucket_base, &bucket));
        self.emit(abi::load_u64(&bucket, &bucket, 0));
        self.emit(abi::compare_immediate(&bucket, "0"));
        self.emit(abi::branch_eq(not_found_label));

        // candidateIdx = bucket - 1; entryAddr = map + HEADER + idx*ENTRY.
        let entry_addr = self.temporary_vreg();
        self.emit(abi::subtract_immediate(&bucket, &bucket, 1));
        self.emit(abi::move_immediate(&scratch, "Integer", &entry_size));
        self.emit(abi::multiply_registers(&entry_addr, &bucket, &scratch));
        self.emit(abi::add_registers(&entry_addr, &entry_addr, &map));
        self.emit(abi::add_immediate(
            &entry_addr,
            &entry_addr,
            COLLECTION_HEADER_SIZE,
        ));

        // A length mismatch means a hash collision at the home slot — hand the full
        // probe walk to the helper (rare); otherwise byte-compare the stored key.
        let stored_len = self.temporary_vreg();
        self.emit(abi::load_u64(
            &stored_len,
            &entry_addr,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::compare_registers(&stored_len, &key_len));
        self.emit(abi::branch_ne(&fallback));
        let stored_ptr = self.temporary_vreg();
        self.emit(abi::load_u64(
            &stored_ptr,
            &entry_addr,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::add_registers(&stored_ptr, &data_base, &stored_ptr));
        self.emit(abi::move_register(&cursor, &key_ptr));
        self.emit(abi::move_register(&remaining, &key_len));
        self.emit(abi::label(&cmp_loop));
        self.emit(abi::compare_immediate(&remaining, "0"));
        self.emit(abi::branch_eq(&store_entry));
        let query_byte = self.temporary_vreg();
        let stored_byte = self.temporary_vreg();
        self.emit(abi::load_u8(&query_byte, &cursor, 0));
        self.emit(abi::load_u8(&stored_byte, &stored_ptr, 0));
        self.emit(abi::compare_registers(&query_byte, &stored_byte));
        self.emit(abi::branch_ne(&fallback));
        self.emit(abi::add_immediate(&cursor, &cursor, 1));
        self.emit(abi::add_immediate(&stored_ptr, &stored_ptr, 1));
        self.emit(abi::subtract_immediate(&remaining, &remaining, 1));
        self.emit(abi::branch(&cmp_loop));

        self.emit(abi::label(&store_entry));
        self.emit(abi::store_u64(
            &entry_addr,
            abi::stack_pointer(),
            entry_slot,
        ));
        self.emit(abi::branch(&done));

        // Fallback: full probe via `_mfb_rt_map_probe` (also lazily builds buckets).
        self.emit(abi::label(&fallback));
        self.emit(abi::load_u64(
            abi::ARG[0],
            abi::stack_pointer(),
            collection_slot,
        ));
        self.emit(abi::move_register(abi::ARG[1], &key_ptr));
        self.emit(abi::move_register(abi::ARG[2], &key_len));
        self.emit(abi::branch_link(MAP_PROBE_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: MAP_PROBE_SYMBOL.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
        // x0 = entry index, or -1 (signed negative) when absent.
        self.emit(abi::compare_immediate(abi::RET[0], "0"));
        self.emit(abi::branch_lt(not_found_label));
        let fb_scratch = self.temporary_vreg();
        let fb_map = self.temporary_vreg();
        let fb_entry = self.temporary_vreg();
        self.emit(abi::move_immediate(&fb_scratch, "Integer", &entry_size));
        self.emit(abi::multiply_registers(&fb_entry, abi::RET[0], &fb_scratch));
        self.emit(abi::load_u64(
            &fb_map,
            abi::stack_pointer(),
            collection_slot,
        ));
        self.emit(abi::add_registers(&fb_entry, &fb_entry, &fb_map));
        self.emit(abi::add_immediate(
            &fb_entry,
            &fb_entry,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::store_u64(&fb_entry, abi::stack_pointer(), entry_slot));

        self.emit(abi::label(&done));
        Ok(entry_slot)
    }

    pub(super) fn lower_map_get(
        &mut self,
        collection_slot: usize,
        key_slot: usize,
        collection_type: &str,
        key_type: &str,
        value_type: &str,
    ) -> Result<ValueResult, String> {
        if Self::map_key_probe_eligible(key_type) {
            let not_found = self.label("map_get_not_found");
            let done = self.label("map_get_done");
            let entry_slot =
                self.emit_map_probe(collection_slot, key_slot, key_type, &not_found)?;
            self.reset_temporary_registers();
            let collection = self.allocate_register()?;
            let entry = self.allocate_register()?;
            let value_offset = self.allocate_register()?;
            let value_length = self.allocate_register()?;
            self.emit(abi::load_u64(
                &collection,
                abi::stack_pointer(),
                collection_slot,
            ));
            self.emit(abi::load_u64(&entry, abi::stack_pointer(), entry_slot));
            self.emit(abi::load_u64(
                &value_offset,
                &entry,
                COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
            ));
            self.emit(abi::load_u64(
                &value_length,
                &entry,
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ));
            let result =
                self.emit_load_map_payload(value_type, &collection, &value_offset, &value_length)?;
            self.emit(abi::branch(&done));
            self.emit(abi::label(&not_found));
            self.emit_not_found_return()?;
            self.emit(abi::label(&done));
            return Ok(ValueResult {
                type_: value_type.to_string(),
                location: result,
                text: format!("get({collection_type}, {key_type}) [hash]"),
            });
        }
        self.reset_temporary_registers();
        let collection = self.allocate_register()?;
        let key = self.allocate_register()?;
        let count = self.allocate_register()?;
        let index = self.allocate_register()?;
        let entry = self.allocate_register()?;
        let key_offset = self.allocate_register()?;
        let key_length = self.allocate_register()?;
        let value_offset = self.allocate_register()?;
        let value_length = self.allocate_register()?;
        let loop_label = self.label("map_get_loop");
        let found = self.label("map_get_found");
        let next = self.label("map_get_next");
        let not_found = self.label("map_get_not_found");
        let done = self.label("map_get_done");

        self.emit(abi::load_u64(
            &collection,
            abi::stack_pointer(),
            collection_slot,
        ));
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

        self.emit(abi::label(&found));
        self.emit(abi::load_u64(
            &value_offset,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            &value_length,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        let result =
            self.emit_load_map_payload(value_type, &collection, &value_offset, &value_length)?;
        self.emit(abi::branch(&done));

        self.emit(abi::label(&next));
        self.emit(abi::add_immediate(&entry, &entry, COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate(&index, &index, 1));
        self.emit(abi::branch(&loop_label));

        self.emit(abi::label(&not_found));
        self.emit_not_found_return()?;
        self.emit(abi::label(&done));

        Ok(ValueResult {
            type_: value_type.to_string(),
            location: result,
            text: format!("get({collection_type}, {key_type})"),
        })
    }

    pub(super) fn lower_list_get_or(
        &mut self,
        collection_slot: usize,
        key_slot: usize,
        default_slot: usize,
        collection_type: &str,
        element_type: &str,
    ) -> Result<ValueResult, String> {
        self.reset_temporary_registers();
        let collection = self.allocate_register()?;
        let index = self.allocate_register()?;
        let count = self.allocate_register()?;
        let entry_offset = self.allocate_register()?;
        let entry = self.allocate_register()?;
        let value_offset = self.allocate_register()?;
        let value_length = self.allocate_register()?;
        let use_default = self.label("list_get_or_default");
        let done = self.label("list_get_or_done");

        self.emit(abi::load_u64(
            &collection,
            abi::stack_pointer(),
            collection_slot,
        ));
        self.emit(abi::load_u64(&index, abi::stack_pointer(), key_slot));
        self.emit(abi::compare_immediate(&index, "0"));
        self.emit(abi::branch_lt(&use_default));
        self.emit(abi::load_u64(&count, &collection, COLLECTION_OFFSET_COUNT));
        self.emit(abi::compare_registers(&index, &count));
        self.emit(abi::branch_ge(&use_default));
        self.emit_element_value_offset(
            &value_offset,
            &value_length,
            &collection,
            &index,
            &entry_offset,
            &entry,
            element_type,
        );
        let result = self.emit_load_collection_payload(
            element_type,
            &collection,
            &value_offset,
            &value_length,
        )?;
        self.emit(abi::branch(&done));
        self.emit(abi::label(&use_default));
        if element_type == "String" {
            // See `lower_map_get_or`: the found path materializes a fresh owned
            // string, so the default must be copied too — returning the alias
            // double-frees it and corrupts the arena.
            let default_ptr = self.allocate_register()?;
            self.emit(abi::load_u64(
                &default_ptr,
                abi::stack_pointer(),
                default_slot,
            ));
            let copied = self.emit_copy_owned_string(&default_ptr)?;
            self.emit(abi::move_register(&result, &copied));
        } else {
            self.emit(abi::load_u64(&result, abi::stack_pointer(), default_slot));
        }
        self.emit(abi::label(&done));

        Ok(ValueResult {
            type_: element_type.to_string(),
            location: result,
            text: format!("getOr({collection_type}, Integer, {element_type})"),
        })
    }

    pub(super) fn lower_map_get_or(
        &mut self,
        collection_slot: usize,
        key_slot: usize,
        default_slot: usize,
        collection_type: &str,
        key_type: &str,
        value_type: &str,
    ) -> Result<ValueResult, String> {
        self.reset_temporary_registers();
        let collection = self.allocate_register()?;
        let key = self.allocate_register()?;
        let count = self.allocate_register()?;
        let index = self.allocate_register()?;
        let entry = self.allocate_register()?;
        let key_offset = self.allocate_register()?;
        let key_length = self.allocate_register()?;
        let value_offset = self.allocate_register()?;
        let value_length = self.allocate_register()?;
        let loop_label = self.label("map_get_or_loop");
        let found = self.label("map_get_or_found");
        let next = self.label("map_get_or_next");
        let use_default = self.label("map_get_or_default");
        let done = self.label("map_get_or_done");

        self.emit(abi::load_u64(
            &collection,
            abi::stack_pointer(),
            collection_slot,
        ));
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
        self.emit(abi::branch_ge(&use_default));
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

        self.emit(abi::label(&found));
        self.emit(abi::load_u64(
            &value_offset,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            &value_length,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        let result =
            self.emit_load_map_payload(value_type, &collection, &value_offset, &value_length)?;
        self.emit(abi::branch(&done));

        self.emit(abi::label(&next));
        self.emit(abi::add_immediate(&entry, &entry, COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate(&index, &index, 1));
        self.emit(abi::branch(&loop_label));

        self.emit(abi::label(&use_default));
        if value_type == "String" {
            // Copy the aliased default into a fresh owned string so both paths
            // return an owned `String` (found path materializes fresh); returning
            // the alias double-frees it and corrupts the arena. See
            // `emit_copy_owned_string`.
            let default_ptr = self.allocate_register()?;
            self.emit(abi::load_u64(
                &default_ptr,
                abi::stack_pointer(),
                default_slot,
            ));
            let copied = self.emit_copy_owned_string(&default_ptr)?;
            self.emit(abi::move_register(&result, &copied));
        } else {
            self.emit(abi::load_u64(&result, abi::stack_pointer(), default_slot));
        }
        self.emit(abi::label(&done));

        Ok(ValueResult {
            type_: value_type.to_string(),
            location: result,
            text: format!("getOr({collection_type}, {key_type}, {value_type})"),
        })
    }
}
