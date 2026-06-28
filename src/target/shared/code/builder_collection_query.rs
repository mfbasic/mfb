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
        self.emit(abi::move_immediate(
            &entry_offset,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(
            &entry_offset,
            &index,
            &entry_offset,
        ));
        self.emit(abi::add_immediate(
            &entry,
            &collection,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(&entry, &entry, &entry_offset));
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
        match key_type {
            "String" => {
                self.emit(abi::load_u64("x9", abi::stack_pointer(), key_slot));
                self.emit(abi::load_u64("x2", "x9", 0));
                self.emit(abi::add_immediate("x1", "x9", 8));
            }
            "Boolean" | "Byte" => {
                self.emit(abi::add_immediate("x1", abi::stack_pointer(), key_slot));
                self.emit(abi::move_immediate("x2", "Integer", "1"));
            }
            "Integer" | "Float" | "Fixed" => {
                self.emit(abi::add_immediate("x1", abi::stack_pointer(), key_slot));
                self.emit(abi::move_immediate("x2", "Integer", "8"));
            }
            other => {
                return Err(format!(
                    "native map probe does not support key type '{other}'"
                ));
            }
        }
        Ok(())
    }

    /// Probe the map (pointer in `collection_slot`) for the key (in `key_slot`) via
    /// `_mfb_rt_map_probe`; branch to `not_found_label` when absent, otherwise store
    /// the matching entry address into a fresh stack slot and return its offset.
    pub(super) fn emit_map_probe(
        &mut self,
        collection_slot: usize,
        key_slot: usize,
        key_type: &str,
        not_found_label: &str,
    ) -> Result<usize, String> {
        self.emit_map_query_key(key_type, key_slot)?;
        self.emit(abi::load_u64("x0", abi::stack_pointer(), collection_slot));
        self.emit(abi::branch_link(MAP_PROBE_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: MAP_PROBE_SYMBOL.to_string(),
            kind: "branch26".to_string(),
            binding: "internal".to_string(),
            library: None,
        });
        // x0 = entry index, or -1 (signed negative) when absent.
        self.emit(abi::compare_immediate("x0", "0"));
        self.emit(abi::branch_lt(not_found_label));
        let entry_slot = self.allocate_stack_object("map_probe_entry", 8);
        self.emit(abi::move_immediate(
            "x16",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x9", "x0", "x16"));
        self.emit(abi::load_u64("x10", abi::stack_pointer(), collection_slot));
        self.emit(abi::add_registers("x9", "x9", "x10"));
        self.emit(abi::add_immediate("x9", "x9", COLLECTION_HEADER_SIZE));
        self.emit(abi::store_u64("x9", abi::stack_pointer(), entry_slot));
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
            let entry_slot = self.emit_map_probe(collection_slot, key_slot, key_type, &not_found)?;
            self.reset_temporary_registers();
            let collection = self.allocate_register()?;
            let entry = self.allocate_register()?;
            let value_offset = self.allocate_register()?;
            let value_length = self.allocate_register()?;
            self.emit(abi::load_u64(&collection, abi::stack_pointer(), collection_slot));
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
            let result = self.emit_load_collection_payload(
                value_type,
                &collection,
                &value_offset,
                &value_length,
            )?;
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
        let result = self.emit_load_collection_payload(
            value_type,
            &collection,
            &value_offset,
            &value_length,
        )?;
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
        self.emit(abi::move_immediate(
            &entry_offset,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(
            &entry_offset,
            &index,
            &entry_offset,
        ));
        self.emit(abi::add_immediate(
            &entry,
            &collection,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(&entry, &entry, &entry_offset));
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
        let result = self.emit_load_collection_payload(
            element_type,
            &collection,
            &value_offset,
            &value_length,
        )?;
        self.emit(abi::branch(&done));
        self.emit(abi::label(&use_default));
        self.emit(abi::load_u64(&result, abi::stack_pointer(), default_slot));
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
        let result = self.emit_load_collection_payload(
            value_type,
            &collection,
            &value_offset,
            &value_length,
        )?;
        self.emit(abi::branch(&done));

        self.emit(abi::label(&next));
        self.emit(abi::add_immediate(&entry, &entry, COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate(&index, &index, 1));
        self.emit(abi::branch(&loop_label));

        self.emit(abi::label(&use_default));
        self.emit(abi::load_u64(&result, abi::stack_pointer(), default_slot));
        self.emit(abi::label(&done));

        Ok(ValueResult {
            type_: value_type.to_string(),
            location: result,
            text: format!("getOr({collection_type}, {key_type}, {value_type})"),
        })
    }
}
