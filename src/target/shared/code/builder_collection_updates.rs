use super::*;

impl CodeBuilder<'_> {
    pub(super) fn lower_collection_append(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let list = self.lower_value(&args[0])?;
        let Some(element_type) = list_element_type(&list.type_) else {
            return Err(format!(
                "native collection append does not accept {}",
                list.type_
            ));
        };
        let list_slot = self.allocate_stack_object("append_list", 8);
        self.emit(abi::store_u64(
            &list.location,
            abi::stack_pointer(),
            list_slot,
        ));
        let item = self.lower_value(&args[1])?;
        let insert_slot =
            self.collection_argument_as_list_slot(&list.type_, &element_type, item)?;
        let index_slot = self.allocate_stack_object("append_index", 8);
        self.emit(abi::load_u64("x8", abi::stack_pointer(), list_slot));
        self.emit(abi::load_u64("x8", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64("x8", abi::stack_pointer(), index_slot));
        self.lower_list_insert_collection(
            list_slot,
            index_slot,
            insert_slot,
            &list.type_,
            &element_type,
        )
    }

    pub(super) fn lower_collection_prepend(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let list = self.lower_value(&args[0])?;
        let Some(element_type) = list_element_type(&list.type_) else {
            return Err(format!(
                "native collection prepend does not accept {}",
                list.type_
            ));
        };
        let list_slot = self.allocate_stack_object("prepend_list", 8);
        self.emit(abi::store_u64(
            &list.location,
            abi::stack_pointer(),
            list_slot,
        ));
        let item = self.lower_value(&args[1])?;
        if item.type_ == list.type_ {
            return Err("native collection prepend expects a single item, not a list".to_string());
        }
        let insert_slot =
            self.collection_argument_as_list_slot(&list.type_, &element_type, item)?;
        let index_slot = self.allocate_stack_object("prepend_index", 8);
        self.emit(abi::move_immediate("x8", "Integer", "0"));
        self.emit(abi::store_u64("x8", abi::stack_pointer(), index_slot));
        self.lower_list_insert_collection(
            list_slot,
            index_slot,
            insert_slot,
            &list.type_,
            &element_type,
        )
    }

    pub(super) fn lower_collection_insert(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let list = self.lower_value(&args[0])?;
        let Some(element_type) = list_element_type(&list.type_) else {
            return Err(format!(
                "native collection insert does not accept {}",
                list.type_
            ));
        };
        let list_slot = self.allocate_stack_object("insert_list", 8);
        self.emit(abi::store_u64(
            &list.location,
            abi::stack_pointer(),
            list_slot,
        ));
        let index = self.lower_value(&args[1])?;
        if index.type_ != "Integer" {
            return Err(format!(
                "native collection insert index must be Integer, got {}",
                index.type_
            ));
        }
        let index_slot = self.allocate_stack_object("insert_index", 8);
        self.emit(abi::store_u64(
            &index.location,
            abi::stack_pointer(),
            index_slot,
        ));
        let item = self.lower_value(&args[2])?;
        if item.type_ == list.type_ {
            return Err("native collection insert expects a single item, not a list".to_string());
        }
        let insert_slot =
            self.collection_argument_as_list_slot(&list.type_, &element_type, item)?;
        self.lower_list_insert_collection(
            list_slot,
            index_slot,
            insert_slot,
            &list.type_,
            &element_type,
        )
    }

    pub(super) fn collection_argument_as_list_slot(
        &mut self,
        list_type: &str,
        element_type: &str,
        item: ValueResult,
    ) -> Result<usize, String> {
        if item.type_ == list_type {
            let slot = self.allocate_stack_object("collection_insert_list", 8);
            self.emit(abi::store_u64(&item.location, abi::stack_pointer(), slot));
            return Ok(slot);
        }
        if item.type_ != element_type {
            return Err(format!(
                "native collection list item must be {}, got {}",
                element_type, item.type_
            ));
        }
        let item_slot = self.allocate_stack_object("collection_insert_item", 8);
        self.emit(abi::store_u64(
            &item.location,
            abi::stack_pointer(),
            item_slot,
        ));
        let singleton = self.lower_collection_values(
            list_type,
            vec![CollectionValueSlot {
                key: None,
                value: PayloadSlot {
                    slot: item_slot,
                    type_: element_type.to_string(),
                },
            }],
            "singleton list",
        )?;
        let slot = self.allocate_stack_object("collection_insert_singleton", 8);
        self.emit(abi::store_u64(
            &singleton.location,
            abi::stack_pointer(),
            slot,
        ));
        Ok(slot)
    }

    pub(super) fn lower_collection_remove_at(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let list = self.lower_value(&args[0])?;
        let Some(element_type) = list_element_type(&list.type_) else {
            return Err(format!(
                "native collection removeAt does not accept {}",
                list.type_
            ));
        };
        let list_slot = self.allocate_stack_object("remove_at_list", 8);
        self.emit(abi::store_u64(
            &list.location,
            abi::stack_pointer(),
            list_slot,
        ));
        let index = self.lower_value(&args[1])?;
        if index.type_ != "Integer" {
            return Err(format!(
                "native collection removeAt index must be Integer, got {}",
                index.type_
            ));
        }
        let index_slot = self.allocate_stack_object("remove_at_index", 8);
        self.emit(abi::store_u64(
            &index.location,
            abi::stack_pointer(),
            index_slot,
        ));
        self.lower_list_remove_at(list_slot, index_slot, &list.type_, &element_type)
    }

    pub(super) fn lower_collection_set(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let collection = self.lower_value(&args[0])?;
        if let Some(element_type) = list_element_type(&collection.type_) {
            let list_slot = self.allocate_stack_object("set_list", 8);
            self.emit(abi::store_u64(
                &collection.location,
                abi::stack_pointer(),
                list_slot,
            ));
            let index = self.lower_value(&args[1])?;
            if index.type_ != "Integer" {
                return Err(format!(
                    "native collection set list index must be Integer, got {}",
                    index.type_
                ));
            }
            let index_slot = self.allocate_stack_object("set_index", 8);
            self.emit(abi::store_u64(
                &index.location,
                abi::stack_pointer(),
                index_slot,
            ));
            let item = self.lower_value(&args[2])?;
            if item.type_ != element_type {
                return Err(format!(
                    "native collection set list item must be {}, got {}",
                    element_type, item.type_
                ));
            }
            let singleton_slot =
                self.collection_argument_as_list_slot(&collection.type_, &element_type, item)?;
            let removed =
                self.lower_list_remove_at(list_slot, index_slot, &collection.type_, &element_type)?;
            let removed_slot = self.allocate_stack_object("set_removed_list", 8);
            self.emit(abi::store_u64(
                &removed.location,
                abi::stack_pointer(),
                removed_slot,
            ));
            return self.lower_list_insert_collection(
                removed_slot,
                index_slot,
                singleton_slot,
                &collection.type_,
                &element_type,
            );
        }

        if let Some((key_type, value_type)) = map_type_parts(&collection.type_) {
            let map_slot = self.allocate_stack_object("set_map", 8);
            self.emit(abi::store_u64(
                &collection.location,
                abi::stack_pointer(),
                map_slot,
            ));
            let key = self.lower_value(&args[1])?;
            if key.type_ != key_type {
                return Err(format!(
                    "native collection set map key must be {}, got {}",
                    key_type, key.type_
                ));
            }
            let key_slot = self.allocate_stack_object("set_map_key", 8);
            self.emit(abi::store_u64(
                &key.location,
                abi::stack_pointer(),
                key_slot,
            ));
            let value = self.lower_value(&args[2])?;
            if value.type_ != value_type {
                return Err(format!(
                    "native collection set map value must be {}, got {}",
                    value_type, value.type_
                ));
            }
            let value_slot = self.allocate_stack_object("set_map_value", 8);
            self.emit(abi::store_u64(
                &value.location,
                abi::stack_pointer(),
                value_slot,
            ));
            let without =
                self.lower_map_remove_key(map_slot, key_slot, &collection.type_, &key_type)?;
            let without_slot = self.allocate_stack_object("set_map_without", 8);
            self.emit(abi::store_u64(
                &without.location,
                abi::stack_pointer(),
                without_slot,
            ));
            let singleton = self.lower_collection_values(
                &collection.type_,
                vec![CollectionValueSlot {
                    key: Some(PayloadSlot {
                        slot: key_slot,
                        type_: key_type.clone(),
                    }),
                    value: PayloadSlot {
                        slot: value_slot,
                        type_: value_type,
                    },
                }],
                "singleton map",
            )?;
            let singleton_slot = self.allocate_stack_object("set_map_singleton", 8);
            self.emit(abi::store_u64(
                &singleton.location,
                abi::stack_pointer(),
                singleton_slot,
            ));
            return self.lower_map_concat(without_slot, singleton_slot, &collection.type_);
        }

        Err(format!(
            "native collection set does not accept {} yet",
            collection.type_
        ))
    }

    pub(super) fn lower_collection_remove_key(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let map = self.lower_value(&args[0])?;
        let Some((key_type, _)) = map_type_parts(&map.type_) else {
            return Err(format!(
                "native collection removeKey does not accept {}",
                map.type_
            ));
        };
        let map_slot = self.allocate_stack_object("remove_key_map", 8);
        self.emit(abi::store_u64(
            &map.location,
            abi::stack_pointer(),
            map_slot,
        ));
        let key = self.lower_value(&args[1])?;
        if key.type_ != key_type {
            return Err(format!(
                "native collection removeKey key must be {}, got {}",
                key_type, key.type_
            ));
        }
        let key_slot = self.allocate_stack_object("remove_key_key", 8);
        self.emit(abi::store_u64(
            &key.location,
            abi::stack_pointer(),
            key_slot,
        ));
        self.lower_map_remove_key(map_slot, key_slot, &map.type_, &key_type)
    }

    /// Insert collection `B` (insert_slot) into list `A` (base_slot) at
    /// `index_slot` using the offset-stable scheme (plan-01 §4.1): copy `A`'s and
    /// `B`'s data regions verbatim (no per-entry repack), then splice the lookup
    /// table with three block moves — head `A` entries verbatim, `B` entries with
    /// each `valueOffset` shifted by `A.dataLength`, tail `A` entries verbatim.
    /// Append is `index == count`, prepend is `index == 0`. Phase 2 keeps the
    /// result tight (`capacity == count`).
    pub(super) fn lower_list_insert_collection(
        &mut self,
        base_slot: usize,
        index_slot: usize,
        insert_slot: usize,
        list_type: &str,
        element_type: &str,
    ) -> Result<ValueResult, String> {
        let layout = CollectionTypeLayout::from_type(list_type)
            .ok_or_else(|| format!("native code collection type '{list_type}' is not supported"))?;
        for register in ["x20", "x21", "x22", "x23", "x24", "x25"] {
            self.mark_register_used(register);
        }
        let result_slot = self.allocate_stack_object("list_insert_result", 8);
        let valid_start = self.label("list_insert_valid_start");
        let alloc_ok = self.label("list_insert_alloc_ok");
        let invalid = self.label("list_insert_invalid");
        let done = self.label("list_insert_done");

        // Validate 0 <= index <= count(A), then size the allocation:
        //   HEADER + (count_A + count_B) * ENTRY + (dataLen_A + dataLen_B).
        self.emit(abi::load_u64("x8", abi::stack_pointer(), base_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), insert_slot));
        self.emit(abi::load_u64("x10", abi::stack_pointer(), index_slot));
        self.emit(abi::load_u64("x11", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::compare_immediate("x10", "0"));
        self.emit(abi::branch_ge(&valid_start));
        self.emit(abi::branch(&invalid));
        self.emit(abi::label(&valid_start));
        self.emit(abi::compare_registers("x10", "x11"));
        self.emit(abi::branch_gt(&invalid));
        self.emit(abi::load_u64("x12", "x9", COLLECTION_OFFSET_COUNT));
        self.emit(abi::add_registers("x13", "x11", "x12"));
        self.emit(abi::load_u64("x14", "x8", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::load_u64("x15", "x9", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::add_registers("x15", "x14", "x15"));
        self.emit(abi::move_immediate(
            "x16",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x17", "x13", "x16"));
        self.emit(abi::add_immediate(
            abi::return_register(),
            "x17",
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(
            abi::return_register(),
            abi::return_register(),
            "x15",
        ));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64("x1", abi::stack_pointer(), result_slot));

        // Header: total count / total data length (recomputed from the pointer
        // slots, which survive `arena_alloc`; the pre-alloc registers do not).
        self.emit(abi::load_u64("x8", abi::stack_pointer(), base_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), insert_slot));
        self.emit(abi::load_u64("x11", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64("x12", "x9", COLLECTION_OFFSET_COUNT));
        self.emit(abi::add_registers("x13", "x11", "x12"));
        self.emit(abi::load_u64("x14", "x8", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::load_u64("x15", "x9", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::add_registers("x15", "x14", "x15"));
        self.emit_write_list_header_from_registers(&layout, "x1", "x13", "x15");

        // --- Data region: A verbatim, then B verbatim at offset dataLen_A. ---
        self.emit_collection_data_pointer("x17", "x1"); // dst data base
        self.emit(abi::load_u64("x8", abi::stack_pointer(), base_slot));
        self.emit_collection_data_pointer("x20", "x8"); // A data base
        self.emit(abi::load_u64("x14", "x8", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit_block_copy_advance("x17", "x20", "x14", "x22", "list_insert_dataA");
        self.emit(abi::load_u64("x9", abi::stack_pointer(), insert_slot));
        self.emit_collection_data_pointer("x20", "x9"); // B data base
        self.emit(abi::load_u64("x15", "x9", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit_block_copy_advance("x17", "x20", "x15", "x22", "list_insert_dataB");

        // --- Lookup table splice. ---
        self.emit(abi::move_immediate(
            "x16",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        // Head: dst.table[0..i) <- A.table[0..i) verbatim.
        self.emit(abi::load_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::add_immediate("x17", "x1", COLLECTION_HEADER_SIZE)); // dst table cursor
        self.emit(abi::load_u64("x8", abi::stack_pointer(), base_slot));
        self.emit(abi::add_immediate("x20", "x8", COLLECTION_HEADER_SIZE)); // A table cursor
        self.emit(abi::load_u64("x10", abi::stack_pointer(), index_slot));
        self.emit(abi::multiply_registers("x21", "x10", "x16")); // i * ENTRY
        self.emit_block_copy_advance("x17", "x20", "x21", "x22", "list_insert_head");

        // Inserted: dst.table[i..i+count_B) <- B entries, valueOffset += dataLen_A.
        self.emit(abi::load_u64("x9", abi::stack_pointer(), insert_slot));
        self.emit(abi::add_immediate("x12", "x9", COLLECTION_HEADER_SIZE)); // B table cursor
        self.emit(abi::load_u64("x11", "x9", COLLECTION_OFFSET_COUNT)); // remaining B entries
        self.emit(abi::load_u64("x8", abi::stack_pointer(), base_slot));
        self.emit(abi::load_u64("x14", "x8", COLLECTION_OFFSET_DATA_LENGTH)); // dataLen_A shift
        let insert_loop = self.label("list_insert_b_loop");
        let insert_done = self.label("list_insert_b_done");
        self.emit(abi::label(&insert_loop));
        self.emit(abi::compare_immediate("x11", "0"));
        self.emit(abi::branch_eq(&insert_done));
        self.emit(abi::move_immediate(
            "x22",
            "Byte",
            &COLLECTION_ENTRY_FLAG_USED.to_string(),
        ));
        self.emit(abi::store_u8("x22", "x17", COLLECTION_ENTRY_OFFSET_FLAGS));
        self.emit(abi::move_immediate("x22", "Integer", "0"));
        self.emit(abi::store_u64("x22", "x17", COLLECTION_ENTRY_OFFSET_KEY_OFFSET));
        self.emit(abi::store_u64("x22", "x17", COLLECTION_ENTRY_OFFSET_KEY_LENGTH));
        self.emit(abi::load_u64(
            "x22",
            "x12",
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::add_registers("x22", "x22", "x14"));
        self.emit(abi::store_u64(
            "x22",
            "x17",
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            "x22",
            "x12",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::store_u64(
            "x22",
            "x17",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_immediate("x17", "x17", COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate("x12", "x12", COLLECTION_ENTRY_SIZE));
        self.emit(abi::subtract_immediate("x11", "x11", 1));
        self.emit(abi::branch(&insert_loop));
        self.emit(abi::label(&insert_done));

        // Tail: dst.table[i+count_B..] <- A.table[i..) verbatim. x20 already points
        // at A entry i (advanced past the head copy).
        self.emit(abi::move_immediate(
            "x16",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::load_u64("x8", abi::stack_pointer(), base_slot));
        self.emit(abi::load_u64("x13", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64("x10", abi::stack_pointer(), index_slot));
        self.emit(abi::subtract_registers("x13", "x13", "x10")); // count_A - i
        self.emit(abi::multiply_registers("x21", "x13", "x16"));
        self.emit_block_copy_advance("x17", "x20", "x21", "x22", "list_insert_tail");
        self.emit(abi::branch(&done));
        self.emit(abi::label(&invalid));
        self.emit_index_out_of_range_return()?;
        self.emit(abi::label(&done));

        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: list_type.to_string(),
            location: result,
            text: format!("list update {list_type} over {element_type}"),
        })
    }

    pub(super) fn lower_list_remove_at(
        &mut self,
        base_slot: usize,
        index_slot: usize,
        list_type: &str,
        element_type: &str,
    ) -> Result<ValueResult, String> {
        let layout = CollectionTypeLayout::from_type(list_type)
            .ok_or_else(|| format!("native code collection type '{list_type}' is not supported"))?;
        for register in ["x20", "x21", "x22", "x23", "x24", "x25"] {
            self.mark_register_used(register);
        }
        let result_slot = self.allocate_stack_object("list_remove_result", 8);
        let data_len_slot = self.allocate_stack_object("list_remove_data_len", 8);
        let valid_start = self.label("list_remove_valid_start");
        let alloc_ok = self.label("list_remove_alloc_ok");
        let invalid = self.label("list_remove_invalid");
        let done = self.label("list_remove_done");

        self.emit(abi::load_u64("x8", abi::stack_pointer(), base_slot));
        self.emit(abi::load_u64("x10", abi::stack_pointer(), index_slot));
        self.emit(abi::load_u64("x11", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::compare_immediate("x10", "0"));
        self.emit(abi::branch_ge(&valid_start));
        self.emit(abi::branch(&invalid));
        self.emit(abi::label(&valid_start));
        self.emit(abi::compare_registers("x10", "x11"));
        self.emit(abi::branch_ge(&invalid));
        self.emit(abi::move_immediate(
            "x16",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x17", "x10", "x16"));
        self.emit(abi::add_immediate("x12", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers("x12", "x12", "x17"));
        self.emit(abi::load_u64("x14", "x8", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::load_u64(
            "x15",
            "x12",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::subtract_registers("x15", "x14", "x15"));
        // `arena_alloc` clobbers x15 in its block-grow path; persist the data
        // length so the header write below does not store a stale pointer.
        self.emit(abi::store_u64("x15", abi::stack_pointer(), data_len_slot));
        self.emit(abi::subtract_immediate("x13", "x11", 1));
        self.emit(abi::multiply_registers("x17", "x13", "x16"));
        self.emit(abi::add_immediate(
            abi::return_register(),
            "x17",
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(
            abi::return_register(),
            abi::return_register(),
            "x15",
        ));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64("x15", abi::stack_pointer(), data_len_slot));
        self.emit_write_list_header_from_registers(&layout, "x1", "x13", "x15");

        self.emit(abi::load_u64("x8", abi::stack_pointer(), base_slot));
        self.emit(abi::load_u64("x10", abi::stack_pointer(), index_slot));
        self.emit(abi::load_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::add_immediate("x17", "x1", COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64("x13", "x1", COLLECTION_OFFSET_COUNT));
        self.emit(abi::move_immediate(
            "x16",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x21", "x13", "x16"));
        self.emit(abi::add_registers("x21", "x17", "x21"));
        self.emit(abi::move_immediate("x13", "Integer", "0"));
        self.emit(abi::add_immediate("x12", "x8", COLLECTION_HEADER_SIZE));
        self.emit_collection_data_pointer("x20", "x8");
        self.emit_copy_collection_entries(
            "x12",
            "x20",
            "x17",
            "x21",
            "x13",
            "x10",
            "list_remove_prefix",
        )?;
        self.emit(abi::load_u64("x10", abi::stack_pointer(), index_slot));
        self.emit(abi::load_u64("x14", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::subtract_registers("x14", "x14", "x10"));
        self.emit(abi::subtract_immediate("x14", "x14", 1));
        self.emit(abi::add_immediate("x15", "x10", 1));
        self.emit(abi::multiply_registers("x15", "x15", "x16"));
        self.emit(abi::add_immediate("x12", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers("x12", "x12", "x15"));
        self.emit_collection_data_pointer("x20", "x8");
        self.emit_copy_collection_entries(
            "x12",
            "x20",
            "x17",
            "x21",
            "x13",
            "x14",
            "list_remove_suffix",
        )?;
        self.emit(abi::branch(&done));
        self.emit(abi::label(&invalid));
        self.emit_index_out_of_range_return()?;
        self.emit(abi::label(&done));

        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: list_type.to_string(),
            location: result,
            text: format!("removeAt({list_type}, Integer) over {element_type}"),
        })
    }

    pub(super) fn emit_write_list_header_from_registers(
        &mut self,
        layout: &CollectionTypeLayout,
        collection: &str,
        count: &str,
        data_len: &str,
    ) {
        self.emit(abi::move_immediate("x22", "Byte", &layout.kind.to_string()));
        self.emit(abi::store_u8("x22", collection, COLLECTION_OFFSET_KIND));
        self.emit(abi::move_immediate(
            "x22",
            "Byte",
            &layout.key_type_code.to_string(),
        ));
        self.emit(abi::store_u8("x22", collection, COLLECTION_OFFSET_KEY_TYPE));
        self.emit(abi::move_immediate(
            "x22",
            "Byte",
            &layout.value_type_code.to_string(),
        ));
        self.emit(abi::store_u8(
            "x22",
            collection,
            COLLECTION_OFFSET_VALUE_TYPE,
        ));
        self.emit(abi::move_immediate("x22", "Byte", "1"));
        self.emit(abi::store_u8(
            "x22",
            collection,
            COLLECTION_OFFSET_FLAGS_VERSION,
        ));
        self.emit(abi::store_u64(count, collection, COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64(
            count,
            collection,
            COLLECTION_OFFSET_CAPACITY,
        ));
        self.emit(abi::store_u64(
            data_len,
            collection,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::store_u64(
            data_len,
            collection,
            COLLECTION_OFFSET_DATA_CAPACITY,
        ));
    }

    pub(super) fn emit_copy_collection_entries(
        &mut self,
        source_entry: &str,
        source_data: &str,
        dest_entry: &str,
        dest_data: &str,
        dest_data_offset: &str,
        count: &str,
        label_prefix: &str,
    ) -> Result<(), String> {
        let loop_label = self.label(&format!("{label_prefix}_loop"));
        let done = self.label(&format!("{label_prefix}_done"));
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate(count, "0"));
        self.emit(abi::branch_eq(&done));
        self.emit(abi::move_immediate(
            "x22",
            "Byte",
            &COLLECTION_ENTRY_FLAG_USED.to_string(),
        ));
        self.emit(abi::store_u8(
            "x22",
            dest_entry,
            COLLECTION_ENTRY_OFFSET_FLAGS,
        ));
        self.emit(abi::move_immediate("x22", "Integer", "0"));
        self.emit(abi::store_u64(
            "x22",
            dest_entry,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::store_u64(
            "x22",
            dest_entry,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::load_u64(
            "x22",
            source_entry,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            "x23",
            source_entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::store_u64(
            dest_data_offset,
            dest_entry,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::store_u64(
            "x23",
            dest_entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers("x24", source_data, "x22"));
        self.emit(abi::add_registers("x25", dest_data, dest_data_offset));
        self.emit_block_copy_advance("x25", "x24", "x23", "x22", &format!("{label_prefix}_value"));
        self.emit(abi::load_u64(
            "x23",
            dest_entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers(
            dest_data_offset,
            dest_data_offset,
            "x23",
        ));
        self.emit(abi::add_immediate(
            source_entry,
            source_entry,
            COLLECTION_ENTRY_SIZE,
        ));
        self.emit(abi::add_immediate(
            dest_entry,
            dest_entry,
            COLLECTION_ENTRY_SIZE,
        ));
        self.emit(abi::subtract_immediate(count, count, 1));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&done));
        Ok(())
    }

    pub(super) fn lower_map_concat(
        &mut self,
        left_slot: usize,
        right_slot: usize,
        map_type: &str,
    ) -> Result<ValueResult, String> {
        let layout = CollectionTypeLayout::from_type(map_type)
            .ok_or_else(|| format!("native code collection type '{map_type}' is not supported"))?;
        let key_payload_align = collection_payload_alignment_for_code(layout.key_type_code);
        let value_payload_align = collection_payload_alignment_for_code(layout.value_type_code);
        let map_max_align = key_payload_align.max(value_payload_align);
        for register in ["x20", "x21", "x22", "x23", "x24", "x25"] {
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
        self.emit(abi::load_u64("x8", abi::stack_pointer(), left_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), right_slot));
        self.emit(abi::load_u64("x10", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64("x11", "x9", COLLECTION_OFFSET_COUNT));
        self.emit(abi::add_registers("x12", "x10", "x11"));
        self.emit(abi::load_u64("x13", "x8", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit_align_offset_register("x13", map_max_align, "x15");
        self.emit(abi::load_u64("x14", "x9", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::add_registers("x14", "x13", "x14"));
        self.emit(abi::move_immediate(
            "x15",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x16", "x12", "x15"));
        self.emit(abi::add_immediate(
            abi::return_register(),
            "x16",
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(
            abi::return_register(),
            abi::return_register(),
            "x14",
        ));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64("x1", abi::stack_pointer(), result_slot));

        // Header: recompute total count / total data length from the pointer slots
        // (the pre-alloc registers do not survive `arena_alloc`).
        self.emit(abi::load_u64("x8", abi::stack_pointer(), left_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), right_slot));
        self.emit(abi::load_u64("x10", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64("x11", "x9", COLLECTION_OFFSET_COUNT));
        self.emit(abi::add_registers("x12", "x10", "x11"));
        self.emit(abi::load_u64("x13", "x8", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit_align_offset_register("x13", map_max_align, "x15");
        self.emit(abi::load_u64("x14", "x9", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::add_registers("x14", "x13", "x14"));
        self.emit_write_list_header_from_registers(&layout, "x1", "x12", "x14");

        // --- Data region: A verbatim at base, B verbatim at align(dataLen_A). ---
        self.emit(abi::load_u64("x1", abi::stack_pointer(), result_slot));
        self.emit_collection_data_pointer("x17", "x1"); // x17 = dst data base (stable)
        self.emit(abi::move_register("x23", "x17")); // moving copy dst
        self.emit(abi::load_u64("x8", abi::stack_pointer(), left_slot));
        self.emit_collection_data_pointer("x20", "x8"); // A data base
        self.emit(abi::load_u64("x14", "x8", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit_block_copy_advance("x23", "x20", "x14", "x22", "map_concat_dataA");
        self.emit(abi::load_u64("x8", abi::stack_pointer(), left_slot));
        self.emit(abi::load_u64("x13", "x8", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit_align_offset_register("x13", map_max_align, "x22"); // alignedA
        self.emit(abi::add_registers("x23", "x17", "x13")); // B dest = base + alignedA
        self.emit(abi::load_u64("x9", abi::stack_pointer(), right_slot));
        self.emit_collection_data_pointer("x20", "x9"); // B data base
        self.emit(abi::load_u64("x15", "x9", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit_block_copy_advance("x23", "x20", "x15", "x22", "map_concat_dataB");

        // --- Lookup table: A entries verbatim, then B entries shifted. ---
        self.emit(abi::move_immediate(
            "x16",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::load_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::add_immediate("x17", "x1", COLLECTION_HEADER_SIZE)); // dst table cursor
        self.emit(abi::load_u64("x8", abi::stack_pointer(), left_slot));
        self.emit(abi::add_immediate("x20", "x8", COLLECTION_HEADER_SIZE)); // A table cursor
        self.emit(abi::load_u64("x10", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::multiply_registers("x21", "x10", "x16")); // count_A * ENTRY
        self.emit_block_copy_advance("x17", "x20", "x21", "x22", "map_concat_tableA");

        // B entries: keyOffset and valueOffset each += align(dataLen_A).
        self.emit(abi::load_u64("x9", abi::stack_pointer(), right_slot));
        self.emit(abi::add_immediate("x12", "x9", COLLECTION_HEADER_SIZE)); // B table cursor
        self.emit(abi::load_u64("x11", "x9", COLLECTION_OFFSET_COUNT)); // remaining
        self.emit(abi::load_u64("x8", abi::stack_pointer(), left_slot));
        self.emit(abi::load_u64("x14", "x8", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit_align_offset_register("x14", map_max_align, "x22"); // shift = alignedA
        let copy_loop = self.label("map_concat_b_loop");
        let copy_done = self.label("map_concat_b_done");
        self.emit(abi::label(&copy_loop));
        self.emit(abi::compare_immediate("x11", "0"));
        self.emit(abi::branch_eq(&copy_done));
        self.emit(abi::move_immediate(
            "x22",
            "Byte",
            &COLLECTION_ENTRY_FLAG_USED.to_string(),
        ));
        self.emit(abi::store_u8("x22", "x17", COLLECTION_ENTRY_OFFSET_FLAGS));
        self.emit(abi::load_u64("x22", "x12", COLLECTION_ENTRY_OFFSET_KEY_OFFSET));
        self.emit(abi::add_registers("x22", "x22", "x14"));
        self.emit(abi::store_u64("x22", "x17", COLLECTION_ENTRY_OFFSET_KEY_OFFSET));
        self.emit(abi::load_u64("x22", "x12", COLLECTION_ENTRY_OFFSET_KEY_LENGTH));
        self.emit(abi::store_u64("x22", "x17", COLLECTION_ENTRY_OFFSET_KEY_LENGTH));
        self.emit(abi::load_u64(
            "x22",
            "x12",
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::add_registers("x22", "x22", "x14"));
        self.emit(abi::store_u64(
            "x22",
            "x17",
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            "x22",
            "x12",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::store_u64(
            "x22",
            "x17",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_immediate("x17", "x17", COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate("x12", "x12", COLLECTION_ENTRY_SIZE));
        self.emit(abi::subtract_immediate("x11", "x11", 1));
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
        let layout = CollectionTypeLayout::from_type(map_type)
            .ok_or_else(|| format!("native code collection type '{map_type}' is not supported"))?;
        let key_payload_align = collection_payload_alignment_for_code(layout.key_type_code);
        let value_payload_align = collection_payload_alignment_for_code(layout.value_type_code);
        for register in ["x20", "x21", "x22", "x23", "x24", "x25"] {
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

        self.emit(abi::load_u64("x8", abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), key_slot));
        self.emit(abi::load_u64("x10", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::move_immediate("x11", "Integer", "0"));
        self.emit(abi::move_immediate("x14", "Integer", "0"));
        self.emit(abi::move_immediate("x15", "Integer", "0"));
        self.emit(abi::add_immediate("x12", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::label(&scan_loop));
        self.emit(abi::compare_registers("x11", "x10"));
        self.emit(abi::branch_ge(&scan_done));
        self.emit(abi::load_u64(
            "x13",
            "x12",
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::load_u64(
            "x16",
            "x12",
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit_collection_payload_matches_value_branch(
            key_type, "x8", "x13", "x16", "x9", &scan_next, &scan_keep,
        )?;
        self.emit(abi::label(&scan_keep));
        self.emit(abi::add_immediate("x14", "x14", 1));
        // Accumulate the retained data length with the same per-payload
        // alignment the copy phase applies, so the precomputed allocation
        // matches the bytes actually written.
        self.emit_align_offset_register("x15", key_payload_align, "x16");
        self.emit(abi::load_u64(
            "x16",
            "x12",
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::add_registers("x15", "x15", "x16"));
        self.emit_align_offset_register("x15", value_payload_align, "x16");
        self.emit(abi::load_u64(
            "x17",
            "x12",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers("x15", "x15", "x17"));
        self.emit(abi::label(&scan_next));
        self.emit(abi::add_immediate("x12", "x12", COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate("x11", "x11", 1));
        self.emit(abi::branch(&scan_loop));
        self.emit(abi::label(&scan_done));
        self.emit(abi::move_immediate(
            "x16",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x17", "x14", "x16"));
        self.emit(abi::add_immediate(
            abi::return_register(),
            "x17",
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(
            abi::return_register(),
            abi::return_register(),
            "x15",
        ));
        // `arena_alloc` clobbers both x14 and x15 in its block-grow path; persist
        // the retained count and data length so the header write below does not
        // store stale pointers.
        self.emit(abi::store_u64("x14", abi::stack_pointer(), count_slot));
        self.emit(abi::store_u64("x15", abi::stack_pointer(), data_len_slot));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64("x14", abi::stack_pointer(), count_slot));
        self.emit(abi::load_u64("x15", abi::stack_pointer(), data_len_slot));
        self.emit_write_list_header_from_registers(&layout, "x1", "x14", "x15");

        self.emit(abi::load_u64("x8", abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), key_slot));
        self.emit(abi::load_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64("x10", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::move_immediate("x11", "Integer", "0"));
        self.emit(abi::move_immediate("x13", "Integer", "0"));
        self.emit(abi::add_immediate("x12", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::add_immediate("x17", "x1", COLLECTION_HEADER_SIZE));
        self.emit_collection_data_pointer("x20", "x8");
        self.emit(abi::load_u64("x14", "x1", COLLECTION_OFFSET_COUNT));
        self.emit(abi::move_immediate(
            "x16",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x21", "x14", "x16"));
        self.emit(abi::add_registers("x21", "x17", "x21"));
        self.emit(abi::label(&copy_loop));
        self.emit(abi::compare_registers("x11", "x10"));
        self.emit(abi::branch_ge(&copy_done));
        self.emit(abi::load_u64(
            "x14",
            "x12",
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::load_u64(
            "x15",
            "x12",
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit_collection_payload_matches_value_branch(
            key_type, "x8", "x14", "x15", "x9", &copy_next, &copy_keep,
        )?;
        self.emit(abi::label(&copy_keep));
        self.emit_copy_one_map_entry(
            "x12",
            "x20",
            "x17",
            "x21",
            "x13",
            key_payload_align,
            value_payload_align,
        );
        self.emit(abi::label(&copy_next));
        self.emit(abi::add_immediate("x12", "x12", COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate("x11", "x11", 1));
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
        self.emit(abi::move_immediate(
            "x22",
            "Byte",
            &COLLECTION_ENTRY_FLAG_USED.to_string(),
        ));
        self.emit(abi::store_u8(
            "x22",
            dest_entry,
            COLLECTION_ENTRY_OFFSET_FLAGS,
        ));
        // Align the destination cursor to the key payload alignment before
        // recording its offset, matching the packing used when the map was
        // first built. Idempotent when the cursor is already aligned.
        self.emit_align_offset_register(dest_data_offset, key_align, "x22");
        self.emit(abi::load_u64(
            "x22",
            source_entry,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::load_u64(
            "x23",
            source_entry,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::store_u64(
            dest_data_offset,
            dest_entry,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::store_u64(
            "x23",
            dest_entry,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::add_registers("x24", source_data, "x22"));
        self.emit(abi::add_registers("x25", dest_data, dest_data_offset));
        self.emit_block_copy_advance("x25", "x24", "x23", "x22", "map_entry_key_copy");
        self.emit(abi::load_u64(
            "x23",
            dest_entry,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::add_registers(
            dest_data_offset,
            dest_data_offset,
            "x23",
        ));

        // Align the destination cursor to the value payload alignment before
        // recording its offset.
        self.emit_align_offset_register(dest_data_offset, value_align, "x22");
        self.emit(abi::load_u64(
            "x22",
            source_entry,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            "x23",
            source_entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::store_u64(
            dest_data_offset,
            dest_entry,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::store_u64(
            "x23",
            dest_entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers("x24", source_data, "x22"));
        self.emit(abi::add_registers("x25", dest_data, dest_data_offset));
        self.emit_block_copy_advance("x25", "x24", "x23", "x22", "map_entry_value_copy");
        self.emit(abi::load_u64(
            "x23",
            dest_entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers(
            dest_data_offset,
            dest_data_offset,
            "x23",
        ));
        self.emit(abi::add_immediate(
            dest_entry,
            dest_entry,
            COLLECTION_ENTRY_SIZE,
        ));
    }

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

    pub(super) fn lower_map_get(
        &mut self,
        collection_slot: usize,
        key_slot: usize,
        collection_type: &str,
        key_type: &str,
        value_type: &str,
    ) -> Result<ValueResult, String> {
        if key_type == "String" {
            return self.lower_string_key_map_get(
                collection_slot,
                key_slot,
                collection_type,
                value_type,
            );
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

    fn lower_string_key_map_get(
        &mut self,
        collection_slot: usize,
        key_slot: usize,
        collection_type: &str,
        value_type: &str,
    ) -> Result<ValueResult, String> {
        self.reset_temporary_registers();
        let loop_label = self.label("map_get_loop");
        let found = self.label("map_get_found");
        let next = self.label("map_get_next");
        let not_found = self.label("map_get_not_found");
        let done = self.label("map_get_done");

        self.emit(abi::load_u64("x8", abi::stack_pointer(), collection_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), key_slot));
        self.emit(abi::load_u64("x10", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::move_immediate("x11", "Integer", "0"));
        self.emit(abi::add_immediate("x12", "x8", COLLECTION_HEADER_SIZE));

        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_registers("x11", "x10"));
        self.emit(abi::branch_ge(&not_found));
        self.emit(abi::load_u64(
            "x13",
            "x12",
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::load_u64(
            "x14",
            "x12",
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::load_u64("x15", "x9", 0));
        self.emit(abi::compare_registers("x14", "x15"));
        self.emit(abi::branch_ne(&next));
        self.emit_collection_data_pointer("x15", "x8");
        self.emit(abi::add_registers("x15", "x15", "x13"));
        self.emit(abi::add_immediate("x16", "x9", 8));
        self.emit_compare_bytes_branch("x15", "x16", "x14", &found, &next, "map_get_string_key");

        self.emit(abi::label(&found));
        self.emit(abi::load_u64(
            "x13",
            "x12",
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            "x14",
            "x12",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.reset_temporary_registers();
        let result = self.emit_load_collection_payload(value_type, "x8", "x13", "x14")?;
        self.emit(abi::branch(&done));

        self.emit(abi::label(&next));
        self.emit(abi::add_immediate("x12", "x12", COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate("x11", "x11", 1));
        self.emit(abi::branch(&loop_label));

        self.emit(abi::label(&not_found));
        self.emit_not_found_return()?;
        self.emit(abi::label(&done));

        Ok(ValueResult {
            type_: value_type.to_string(),
            location: result,
            text: format!("get({collection_type}, String)"),
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
