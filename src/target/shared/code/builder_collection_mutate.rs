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
        // Observation boundary: a `Float` appended element must be finite
        // (plan-17).
        self.observe_float(&args[1], &item)?;
        // A `d`-native float item is materialized into a GPR before being
        // spilled into the collection payload (plan-01 float-dnative).
        let item = self.materialize_float(item)?;
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
        // Observation boundary: a `Float` prepended element must be finite
        // (plan-17).
        self.observe_float(&args[1], &item)?;
        if item.type_ == list.type_ {
            return Err("native collection prepend expects a single item, not a list".to_string());
        }
        // Materialize a `d`-native float before the payload spill (plan-01).
        let item = self.materialize_float(item)?;
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
        // Observation boundary: a `Float` inserted element must be finite
        // (plan-17).
        self.observe_float(&args[2], &item)?;
        if item.type_ == list.type_ {
            return Err("native collection insert expects a single item, not a list".to_string());
        }
        // Materialize a `d`-native float before the payload spill (plan-01).
        let item = self.materialize_float(item)?;
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
            // Observation boundary: a `Float` replacement element must be finite
            // (plan-17).
            self.observe_float(&args[2], &item)?;
            if item.type_ != element_type {
                return Err(format!(
                    "native collection set list item must be {}, got {}",
                    element_type, item.type_
                ));
            }
            // Materialize a `d`-native float before the payload spill (plan-01).
            let item = self.materialize_float(item)?;
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
            // Observation boundary: a `Float` map key must be finite (plan-17).
            self.observe_float(&args[1], &key)?;
            if key.type_ != key_type {
                return Err(format!(
                    "native collection set map key must be {}, got {}",
                    key_type, key.type_
                ));
            }
            let key = self.materialize_float(key)?;
            let key_slot = self.allocate_stack_object("set_map_key", 8);
            self.emit(abi::store_u64(
                &key.location,
                abi::stack_pointer(),
                key_slot,
            ));
            let value = self.lower_value(&args[2])?;
            // Observation boundary: a `Float` map value must be finite (plan-17).
            self.observe_float(&args[2], &value)?;
            if value.type_ != value_type {
                return Err(format!(
                    "native collection set map value must be {}, got {}",
                    value_type, value.type_
                ));
            }
            let value = self.materialize_float(value)?;
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
        // `d`-native float key stores via `str d` (plan-01 float-dnative).
        self.store_value_at(&key, abi::stack_pointer(), key_slot);
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
            kind: RelocIntent::Call,
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
        self.emit(abi::store_u64(
            "x22",
            "x17",
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::store_u64(
            "x22",
            "x17",
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
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
    pub(super) fn lower_list_append_in_place(
        &mut self,
        buffer_slot: usize,
        item_slot: usize,
        list_type: &str,
        element_type: &str,
    ) -> Result<ValueResult, String> {
        let layout = CollectionTypeLayout::from_type(list_type)
            .ok_or_else(|| format!("native code collection type '{list_type}' is not supported"))?;
        for register in ["x20", "x21", "x22", "x23", "x24", "x25"] {
            self.mark_register_used(register);
        }
        let item = PayloadSlot {
            slot: item_slot,
            type_: element_type.to_string(),
        };
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

        // Room check: count < capacity AND dataLength + need <= dataCapacity.
        self.emit(abi::load_u64("x8", abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64("x9", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64("x10", "x8", COLLECTION_OFFSET_CAPACITY));
        self.emit(abi::compare_registers("x9", "x10"));
        self.emit(abi::branch_ge(&realloc));
        self.emit(abi::load_u64("x11", "x8", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::load_u64("x12", abi::stack_pointer(), need_slot));
        self.emit(abi::add_registers("x11", "x11", "x12"));
        self.emit(abi::load_u64("x13", "x8", COLLECTION_OFFSET_DATA_CAPACITY));
        self.emit(abi::compare_registers("x11", "x13"));
        self.emit(abi::branch_hi(&realloc));
        self.emit(abi::branch(&write));

        // --- Grow: allocate a headroom buffer; copy entries + data verbatim. ---
        self.emit(abi::label(&realloc));
        // newCapacity = step(capacity).
        self.emit(abi::load_u64("x8", abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64("x10", "x8", COLLECTION_OFFSET_CAPACITY));
        self.emit_geometric_step(
            "x10",
            "x14",
            "x15",
            COLLECTION_GROW_LOOKUP_INIT,
            COLLECTION_GROW_LOOKUP_TAPER,
            "append_grow_cap",
        );
        self.emit(abi::store_u64("x14", abi::stack_pointer(), new_cap_slot));
        // newDataCapacity = max(step(dataCapacity), dataLength + need).
        self.emit(abi::load_u64("x8", abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64("x10", "x8", COLLECTION_OFFSET_DATA_CAPACITY));
        self.emit_geometric_step(
            "x10",
            "x14",
            "x15",
            COLLECTION_GROW_DATA_INIT,
            COLLECTION_GROW_DATA_TAPER,
            "append_grow_dcap",
        );
        self.emit(abi::load_u64("x8", abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64("x11", "x8", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::load_u64("x12", abi::stack_pointer(), need_slot));
        self.emit(abi::add_registers("x11", "x11", "x12")); // required
        self.emit(abi::compare_registers("x14", "x11"));
        self.emit(abi::branch_hi(&dcap_keep));
        self.emit(abi::branch_eq(&dcap_keep));
        self.emit(abi::move_register("x14", "x11")); // step < required → use required
        self.emit(abi::label(&dcap_keep));
        self.emit(abi::store_u64("x14", abi::stack_pointer(), new_dcap_slot));

        // alloc size = HEADER + newCapacity * ENTRY + newDataCapacity.
        self.emit(abi::load_u64("x14", abi::stack_pointer(), new_cap_slot));
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
        self.emit(abi::load_u64("x15", abi::stack_pointer(), new_dcap_slot));
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
            kind: RelocIntent::Call,
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
        self.emit(abi::store_u64("x1", abi::stack_pointer(), new_buf_slot));

        // Header: old count / old dataLength, new capacity / data capacity.
        self.emit(abi::load_u64("x8", abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64("x9", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64("x11", "x8", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::load_u64("x14", abi::stack_pointer(), new_cap_slot));
        self.emit(abi::load_u64("x15", abi::stack_pointer(), new_dcap_slot));
        self.emit(abi::load_u64("x1", abi::stack_pointer(), new_buf_slot));
        self.emit_write_collection_header_full(&layout, "x1", "x9", "x14", "x11", "x15");

        // Copy the data region verbatim (dataLength bytes), capacity-based base.
        self.emit(abi::load_u64("x1", abi::stack_pointer(), new_buf_slot));
        self.emit_collection_data_pointer("x17", "x1");
        self.emit(abi::load_u64("x8", abi::stack_pointer(), buffer_slot));
        self.emit_collection_data_pointer("x20", "x8");
        self.emit(abi::load_u64("x14", "x8", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit_block_copy_advance("x17", "x20", "x14", "x22", "append_grow_data");

        // Copy the live lookup entries verbatim (count * ENTRY bytes).
        self.emit(abi::load_u64("x1", abi::stack_pointer(), new_buf_slot));
        self.emit(abi::add_immediate("x17", "x1", COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64("x8", abi::stack_pointer(), buffer_slot));
        self.emit(abi::add_immediate("x20", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64("x9", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::move_immediate(
            "x16",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x21", "x9", "x16"));
        self.emit_block_copy_advance("x17", "x20", "x21", "x22", "append_grow_entries");

        // Install the grown buffer; fall through to write the new element.
        self.emit(abi::load_u64("x1", abi::stack_pointer(), new_buf_slot));
        self.emit(abi::store_u64("x1", abi::stack_pointer(), buffer_slot));
        self.emit(abi::branch(&write));

        // --- Write the new element into slot[count], payload at dataLength. ---
        self.emit(abi::label(&write));
        self.emit(abi::load_u64("x8", abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64("x9", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64("x11", "x8", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::add_immediate("x12", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::move_immediate(
            "x16",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x13", "x9", "x16"));
        self.emit(abi::add_registers("x12", "x12", "x13")); // entry addr
        self.emit(abi::move_immediate(
            "x13",
            "Byte",
            &COLLECTION_ENTRY_FLAG_USED.to_string(),
        ));
        self.emit(abi::store_u8("x13", "x12", COLLECTION_ENTRY_OFFSET_FLAGS));
        self.emit(abi::move_immediate("x13", "Integer", "0"));
        self.emit(abi::store_u64(
            "x13",
            "x12",
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::store_u64(
            "x13",
            "x12",
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::store_u64(
            "x11",
            "x12",
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        )); // valueOffset = dataLength
        self.emit(abi::load_u64("x13", abi::stack_pointer(), need_slot));
        self.emit(abi::store_u64(
            "x13",
            "x12",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        )); // valueLength = need
            // Copy the payload bytes to data base + dataLength.
        self.emit(abi::store_u64(
            "x11",
            abi::stack_pointer(),
            data_offset_slot,
        ));
        self.emit_copy_payload_to_collection(buffer_slot, need_slot, &item, data_offset_slot)?;
        // Bump count and dataLength.
        self.emit(abi::load_u64("x8", abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64("x9", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::add_immediate("x9", "x9", 1));
        self.emit(abi::store_u64("x9", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64("x9", "x8", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::load_u64("x13", abi::stack_pointer(), need_slot));
        self.emit(abi::add_registers("x9", "x9", "x13"));
        self.emit(abi::store_u64("x9", "x8", COLLECTION_OFFSET_DATA_LENGTH));

        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), buffer_slot));
        Ok(ValueResult {
            type_: list_type.to_string(),
            location: result,
            text: format!("append in place {list_type} over {element_type}"),
        })
    }

    /// Prepend a single element (held in `item_slot`) to the front of the list
    /// whose pointer lives in `buffer_slot`, **mutating `buffer_slot` in place**
    /// (plan-02 §3). Ensures room exactly like `lower_list_append_in_place`
    /// (geometric grow only when full), then shifts the live lookup entries right
    /// by one (so the new entry takes index 0) and appends the element's payload to
    /// the spare data tail — entry offsets are independent of position, so no data
    /// move is needed. Still O(n) per call (the entry shift), but it drops the
    /// per-call allocation + double copy of the value-semantic insert. The caller
    /// guarantees unique ownership and not an active `FOR EACH` iterable.
    pub(super) fn lower_list_prepend_in_place(
        &mut self,
        buffer_slot: usize,
        item_slot: usize,
        list_type: &str,
        element_type: &str,
    ) -> Result<ValueResult, String> {
        let layout = CollectionTypeLayout::from_type(list_type)
            .ok_or_else(|| format!("native code collection type '{list_type}' is not supported"))?;
        for register in ["x20", "x21", "x22", "x23", "x24", "x25"] {
            self.mark_register_used(register);
        }
        let item = PayloadSlot {
            slot: item_slot,
            type_: element_type.to_string(),
        };
        let need_slot = self.emit_payload_length_to_stack(&item, "prepend_inplace_need")?;
        let data_offset_slot = self.allocate_stack_object("prepend_inplace_doff", 8);
        let new_cap_slot = self.allocate_stack_object("prepend_inplace_newcap", 8);
        let new_dcap_slot = self.allocate_stack_object("prepend_inplace_newdcap", 8);
        let new_buf_slot = self.allocate_stack_object("prepend_inplace_newbuf", 8);

        let realloc = self.label("prepend_inplace_realloc");
        let write = self.label("prepend_inplace_write");
        let alloc_ok = self.label("prepend_inplace_alloc_ok");
        let dcap_keep = self.label("prepend_inplace_dcap_keep");
        let shift_loop = self.label("prepend_inplace_shift_loop");
        let shift_done = self.label("prepend_inplace_shift_done");

        // Room check: count < capacity AND dataLength + need <= dataCapacity.
        self.emit(abi::load_u64("x8", abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64("x9", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64("x10", "x8", COLLECTION_OFFSET_CAPACITY));
        self.emit(abi::compare_registers("x9", "x10"));
        self.emit(abi::branch_ge(&realloc));
        self.emit(abi::load_u64("x11", "x8", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::load_u64("x12", abi::stack_pointer(), need_slot));
        self.emit(abi::add_registers("x11", "x11", "x12"));
        self.emit(abi::load_u64("x13", "x8", COLLECTION_OFFSET_DATA_CAPACITY));
        self.emit(abi::compare_registers("x11", "x13"));
        self.emit(abi::branch_hi(&realloc));
        self.emit(abi::branch(&write));

        // --- Grow: geometric headroom; copy entries + data verbatim. ---
        self.emit(abi::label(&realloc));
        self.emit(abi::load_u64("x8", abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64("x10", "x8", COLLECTION_OFFSET_CAPACITY));
        self.emit_geometric_step(
            "x10",
            "x14",
            "x15",
            COLLECTION_GROW_LOOKUP_INIT,
            COLLECTION_GROW_LOOKUP_TAPER,
            "prepend_grow_cap",
        );
        self.emit(abi::store_u64("x14", abi::stack_pointer(), new_cap_slot));
        self.emit(abi::load_u64("x8", abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64("x10", "x8", COLLECTION_OFFSET_DATA_CAPACITY));
        self.emit_geometric_step(
            "x10",
            "x14",
            "x15",
            COLLECTION_GROW_DATA_INIT,
            COLLECTION_GROW_DATA_TAPER,
            "prepend_grow_dcap",
        );
        self.emit(abi::load_u64("x8", abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64("x11", "x8", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::load_u64("x12", abi::stack_pointer(), need_slot));
        self.emit(abi::add_registers("x11", "x11", "x12"));
        self.emit(abi::compare_registers("x14", "x11"));
        self.emit(abi::branch_hi(&dcap_keep));
        self.emit(abi::branch_eq(&dcap_keep));
        self.emit(abi::move_register("x14", "x11"));
        self.emit(abi::label(&dcap_keep));
        self.emit(abi::store_u64("x14", abi::stack_pointer(), new_dcap_slot));
        // alloc = HEADER + newCapacity * ENTRY + newDataCapacity.
        self.emit(abi::load_u64("x14", abi::stack_pointer(), new_cap_slot));
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
        self.emit(abi::load_u64("x15", abi::stack_pointer(), new_dcap_slot));
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
            kind: RelocIntent::Call,
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
        self.emit(abi::store_u64("x1", abi::stack_pointer(), new_buf_slot));
        // Header: old count / old dataLength, new capacity / data capacity.
        self.emit(abi::load_u64("x8", abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64("x9", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64("x11", "x8", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::load_u64("x14", abi::stack_pointer(), new_cap_slot));
        self.emit(abi::load_u64("x15", abi::stack_pointer(), new_dcap_slot));
        self.emit(abi::load_u64("x1", abi::stack_pointer(), new_buf_slot));
        self.emit_write_collection_header_full(&layout, "x1", "x9", "x14", "x11", "x15");
        // Copy the data region verbatim (dataLength bytes), capacity-based base.
        self.emit(abi::load_u64("x1", abi::stack_pointer(), new_buf_slot));
        self.emit_collection_data_pointer("x17", "x1");
        self.emit(abi::load_u64("x8", abi::stack_pointer(), buffer_slot));
        self.emit_collection_data_pointer("x20", "x8");
        self.emit(abi::load_u64("x14", "x8", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit_block_copy_advance("x17", "x20", "x14", "x22", "prepend_grow_data");
        // Copy the live lookup entries verbatim (count * ENTRY bytes).
        self.emit(abi::load_u64("x1", abi::stack_pointer(), new_buf_slot));
        self.emit(abi::add_immediate("x17", "x1", COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64("x8", abi::stack_pointer(), buffer_slot));
        self.emit(abi::add_immediate("x20", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64("x9", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::move_immediate(
            "x16",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x21", "x9", "x16"));
        self.emit_block_copy_advance("x17", "x20", "x21", "x22", "prepend_grow_entries");
        self.emit(abi::load_u64("x1", abi::stack_pointer(), new_buf_slot));
        self.emit(abi::store_u64("x1", abi::stack_pointer(), buffer_slot));
        self.emit(abi::branch(&write));

        // --- Write: shift entries right by one, new entry at slot[0]. ---
        self.emit(abi::label(&write));
        // Shift lookup entries [0..count) → [1..count+1), backward to avoid overlap.
        self.emit(abi::load_u64("x8", abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64("x9", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::subtract_immediate("x10", "x9", 1)); // i = count - 1
        self.emit(abi::label(&shift_loop));
        self.emit(abi::compare_immediate("x10", "0"));
        self.emit(abi::branch_lt(&shift_done));
        // src = buffer + HEADER + i*ENTRY ; dst = src + ENTRY (x8 = buffer, live).
        self.emit(abi::move_immediate(
            "x16",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x11", "x10", "x16"));
        self.emit(abi::add_immediate("x12", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers("x11", "x12", "x11")); // src = entry[i]
        self.emit(abi::add_immediate("x12", "x11", COLLECTION_ENTRY_SIZE)); // dst = entry[i+1]
        for offset in [0usize, 8, 16, 24, 32] {
            self.emit(abi::load_u64("x13", "x11", offset));
            self.emit(abi::store_u64("x13", "x12", offset));
        }
        self.emit(abi::subtract_immediate("x10", "x10", 1));
        self.emit(abi::branch(&shift_loop));
        self.emit(abi::label(&shift_done));
        // New entry at slot[0]: payload at dataLength, valueOffset = dataLength.
        self.emit(abi::load_u64("x8", abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64("x11", "x8", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::add_immediate("x12", "x8", COLLECTION_HEADER_SIZE)); // entry[0]
        self.emit(abi::move_immediate(
            "x13",
            "Byte",
            &COLLECTION_ENTRY_FLAG_USED.to_string(),
        ));
        self.emit(abi::store_u8("x13", "x12", COLLECTION_ENTRY_OFFSET_FLAGS));
        self.emit(abi::move_immediate("x13", "Integer", "0"));
        self.emit(abi::store_u64(
            "x13",
            "x12",
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::store_u64(
            "x13",
            "x12",
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::store_u64(
            "x11",
            "x12",
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64("x13", abi::stack_pointer(), need_slot));
        self.emit(abi::store_u64(
            "x13",
            "x12",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        // Copy the payload bytes to data base + dataLength.
        self.emit(abi::store_u64(
            "x11",
            abi::stack_pointer(),
            data_offset_slot,
        ));
        self.emit_copy_payload_to_collection(buffer_slot, need_slot, &item, data_offset_slot)?;
        // Bump count and dataLength.
        self.emit(abi::load_u64("x8", abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64("x9", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::add_immediate("x9", "x9", 1));
        self.emit(abi::store_u64("x9", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64("x9", "x8", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::load_u64("x13", abi::stack_pointer(), need_slot));
        self.emit(abi::add_registers("x9", "x9", "x13"));
        self.emit(abi::store_u64("x9", "x8", COLLECTION_OFFSET_DATA_LENGTH));

        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), buffer_slot));
        Ok(ValueResult {
            type_: list_type.to_string(),
            location: result,
            text: format!("prepend in place {list_type} over {element_type}"),
        })
    }

    /// Replace the element at `index_slot` of the list whose buffer pointer lives
    /// in `buffer_slot`, **mutating `buffer_slot` in place** when the replacement
    /// payload fits the target slot (plan-02 §4.1). The common cases — fixed-width
    /// elements, and records/strings whose new payload is the same size or shorter
    /// (`need <= oldLen`) — overwrite the value bytes at the entry's `valueOffset`
    /// and patch `valueLength`: no allocation, no copy. A longer payload
    /// (`need > oldLen`) falls back to the rebuild path (remove + insert), which
    /// repoints `buffer_slot` at a fresh tight buffer. An out-of-range index raises
    /// the same `index out of range` error as the rebuild path. The caller
    /// guarantees the buffer is uniquely owned and not an active `FOR EACH`
    /// iterable. Returns the (possibly new) buffer pointer.
    pub(super) fn lower_list_set_in_place(
        &mut self,
        buffer_slot: usize,
        index_slot: usize,
        item_slot: usize,
        list_type: &str,
        element_type: &str,
    ) -> Result<ValueResult, String> {
        if CollectionTypeLayout::from_type(list_type).is_none() {
            return Err(format!(
                "native code collection type '{list_type}' is not supported"
            ));
        }
        for register in ["x20", "x21", "x22", "x23", "x24", "x25"] {
            self.mark_register_used(register);
        }
        let item = PayloadSlot {
            slot: item_slot,
            type_: element_type.to_string(),
        };
        // Byte size of the replacement payload.
        let need_slot = self.emit_payload_length_to_stack(&item, "set_inplace_need")?;
        let voffset_slot = self.allocate_stack_object("set_inplace_voff", 8);

        let valid = self.label("set_inplace_valid");
        let invalid = self.label("set_inplace_invalid");
        let rebuild = self.label("set_inplace_rebuild");
        let done = self.label("set_inplace_done");

        // Bounds check: 0 <= index < count.
        self.emit(abi::load_u64("x8", abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64("x10", abi::stack_pointer(), index_slot));
        self.emit(abi::load_u64("x11", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::compare_immediate("x10", "0"));
        self.emit(abi::branch_ge(&valid));
        self.emit(abi::branch(&invalid));
        self.emit(abi::label(&valid));
        self.emit(abi::compare_registers("x10", "x11"));
        self.emit(abi::branch_ge(&invalid));

        // entry = buffer + HEADER + index * ENTRY; read valueOffset / valueLength.
        self.emit(abi::move_immediate(
            "x16",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x17", "x10", "x16"));
        self.emit(abi::add_immediate("x12", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers("x12", "x12", "x17"));
        self.emit(abi::load_u64(
            "x13",
            "x12",
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::store_u64("x13", abi::stack_pointer(), voffset_slot));
        self.emit(abi::load_u64(
            "x9",
            "x12",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        )); // oldLen
        self.emit(abi::load_u64("x14", abi::stack_pointer(), need_slot)); // need
                                                                          // need > oldLen (unsigned) → rebuild; else overwrite in place.
        self.emit(abi::compare_registers("x14", "x9"));
        self.emit(abi::branch_hi(&rebuild));

        // --- Overwrite: write the payload at valueOffset, patch valueLength. ---
        self.emit_copy_payload_to_collection(buffer_slot, need_slot, &item, voffset_slot)?;
        self.emit(abi::load_u64("x8", abi::stack_pointer(), buffer_slot));
        self.emit(abi::load_u64("x10", abi::stack_pointer(), index_slot));
        self.emit(abi::move_immediate(
            "x16",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x17", "x10", "x16"));
        self.emit(abi::add_immediate("x12", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers("x12", "x12", "x17"));
        self.emit(abi::load_u64("x14", abi::stack_pointer(), need_slot));
        self.emit(abi::store_u64(
            "x14",
            "x12",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::branch(&done));

        // --- Rebuild (payload grew): remove + insert a fresh singleton list. ---
        self.emit(abi::label(&rebuild));
        let singleton = self.lower_collection_values(
            list_type,
            vec![CollectionValueSlot {
                key: None,
                value: PayloadSlot {
                    slot: item_slot,
                    type_: element_type.to_string(),
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
        self.emit(abi::branch(&done));

        self.emit(abi::label(&invalid));
        self.emit_index_out_of_range_return()?;
        self.emit(abi::label(&done));

        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), buffer_slot));
        Ok(ValueResult {
            type_: list_type.to_string(),
            location: result,
            text: format!("set in place {list_type} over {element_type}"),
        })
    }

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
        let layout = CollectionTypeLayout::from_type(map_type)
            .ok_or_else(|| format!("native code collection type '{map_type}' is not supported"))?;
        for register in ["x20", "x21", "x22", "x23", "x24", "x25"] {
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
            let entry_slot =
                self.emit_map_probe(map_slot, key_slot, key_type, &not_found)?;
            // emit_map_probe already stored the entry address; record the index too
            // (x0 held it before the entry-address math, so recompute it from the
            // entry address: index = (entry - map - HEADER) / ENTRY).
            self.emit(abi::load_u64("x9", abi::stack_pointer(), entry_slot));
            self.emit(abi::load_u64("x10", abi::stack_pointer(), map_slot));
            self.emit(abi::subtract_registers("x9", "x9", "x10"));
            self.emit(abi::subtract_immediate("x9", "x9", COLLECTION_HEADER_SIZE));
            self.emit(abi::move_immediate(
                "x16",
                "Integer",
                &COLLECTION_ENTRY_SIZE.to_string(),
            ));
            self.emit(abi::unsigned_divide_registers("x9", "x9", "x16"));
            self.emit(abi::store_u64("x9", abi::stack_pointer(), found_index_slot));
            self.emit(abi::load_u64("x9", abi::stack_pointer(), entry_slot));
            self.emit(abi::store_u64("x9", abi::stack_pointer(), found_entry_slot));
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
            self.emit(abi::add_immediate(&entry, &collection, COLLECTION_HEADER_SIZE));
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
            self.emit(abi::label(&next));
            self.emit(abi::add_immediate(&entry, &entry, COLLECTION_ENTRY_SIZE));
            self.emit(abi::add_immediate(&index, &index, 1));
            self.emit(abi::branch(&loop_label));
            self.emit(abi::label(&found));
            self.emit(abi::store_u64(&entry, abi::stack_pointer(), found_entry_slot));
            self.emit(abi::store_u64(&index, abi::stack_pointer(), found_index_slot));
            self.emit(abi::branch(&found_handle));
        }

        // --- Found handling (shared): overwrite the value when it fits, else
        // append-and-repoint. Slot-based so it serves both the probe and scan. ---
        self.emit(abi::label(&found_handle));
        self.emit(abi::load_u64("x8", abi::stack_pointer(), found_entry_slot));
        self.emit(abi::load_u64(
            "x9",
            "x8",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        )); // oldValLen
        self.emit(abi::load_u64("x14", abi::stack_pointer(), val_len_slot)); // newValLen
        self.emit(abi::compare_registers("x14", "x9"));
        self.emit(abi::branch_hi(&value_grow)); // newLen > oldLen → append + repoint
        self.emit(abi::load_u64("x8", abi::stack_pointer(), found_entry_slot));
        self.emit(abi::load_u64(
            "x13",
            "x8",
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::store_u64("x13", abi::stack_pointer(), voff_slot));
        self.emit_copy_payload_to_collection(map_slot, val_len_slot, &value_payload, voff_slot)?;
        self.emit(abi::load_u64("x8", abi::stack_pointer(), found_entry_slot));
        self.emit(abi::load_u64("x14", abi::stack_pointer(), val_len_slot));
        self.emit(abi::store_u64(
            "x14",
            "x8",
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
        self.emit(abi::load_u64("x8", abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64("x9", "x8", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::store_u64("x9", abi::stack_pointer(), new_data_len_slot));
        self.emit_align_offset_slot(new_data_len_slot, value_align);
        self.emit(abi::load_u64("x8", abi::stack_pointer(), new_data_len_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), val_len_slot));
        self.emit(abi::add_registers("x8", "x8", "x9"));
        self.emit(abi::store_u64("x8", abi::stack_pointer(), new_data_len_slot));
        // Room: newDataLength <= dataCapacity?
        self.emit(abi::load_u64("x8", abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64("x11", "x8", COLLECTION_OFFSET_DATA_CAPACITY));
        self.emit(abi::load_u64("x12", abi::stack_pointer(), new_data_len_slot));
        self.emit(abi::compare_registers("x12", "x11"));
        self.emit(abi::branch_hi(&vgrow));
        self.emit(abi::branch(&vwrite));

        // Grow the data region only (capacity unchanged); copy entries + data
        // verbatim against the capacity-based base, then repoint.
        self.emit(abi::label(&vgrow));
        self.emit(abi::load_u64("x8", abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64("x10", "x8", COLLECTION_OFFSET_DATA_CAPACITY));
        self.emit_geometric_step(
            "x10",
            "x14",
            "x15",
            COLLECTION_GROW_DATA_INIT,
            COLLECTION_GROW_DATA_TAPER,
            "mapset_vgrow_dcap",
        );
        self.emit(abi::load_u64("x11", abi::stack_pointer(), new_data_len_slot));
        self.emit(abi::compare_registers("x14", "x11"));
        self.emit(abi::branch_hi(&vdcap_keep));
        self.emit(abi::branch_eq(&vdcap_keep));
        self.emit(abi::move_register("x14", "x11"));
        self.emit(abi::label(&vdcap_keep));
        self.emit(abi::store_u64("x14", abi::stack_pointer(), new_dcap_slot));
        // alloc = HEADER + capacity * ENTRY + newDataCapacity (capacity unchanged).
        self.emit(abi::load_u64("x8", abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64("x14", "x8", COLLECTION_OFFSET_CAPACITY));
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
        self.emit(abi::load_u64("x15", abi::stack_pointer(), new_dcap_slot));
        self.emit(abi::add_registers(
            abi::return_register(),
            abi::return_register(),
            "x15",
        ));
        // Reserve the map hash bucket region (x14 = capacity, unchanged on vgrow).
        self.emit_reserve_map_buckets(true, "x14", abi::return_register(), "x16");
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&valloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&valloc_ok));
        self.emit(abi::store_u64("x1", abi::stack_pointer(), new_buf_slot));
        // Header: old count / old dataLength, same capacity, new data capacity.
        self.emit(abi::load_u64("x8", abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64("x9", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64("x14", "x8", COLLECTION_OFFSET_CAPACITY));
        self.emit(abi::load_u64("x11", "x8", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::load_u64("x15", abi::stack_pointer(), new_dcap_slot));
        self.emit(abi::load_u64("x1", abi::stack_pointer(), new_buf_slot));
        self.emit_write_collection_header_full(&layout, "x1", "x9", "x14", "x11", "x15");
        // Copy the data region verbatim (dataLength bytes), capacity-based base.
        self.emit(abi::load_u64("x1", abi::stack_pointer(), new_buf_slot));
        self.emit_collection_data_pointer("x17", "x1");
        self.emit(abi::load_u64("x8", abi::stack_pointer(), map_slot));
        self.emit_collection_data_pointer("x20", "x8");
        self.emit(abi::load_u64("x14", "x8", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit_block_copy_advance("x17", "x20", "x14", "x22", "mapset_vgrow_data");
        // Copy the lookup entries verbatim (count * ENTRY bytes).
        self.emit(abi::load_u64("x1", abi::stack_pointer(), new_buf_slot));
        self.emit(abi::add_immediate("x17", "x1", COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64("x8", abi::stack_pointer(), map_slot));
        self.emit(abi::add_immediate("x20", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64("x9", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::move_immediate(
            "x16",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x21", "x9", "x16"));
        self.emit_block_copy_advance("x17", "x20", "x21", "x22", "mapset_vgrow_entries");
        self.emit(abi::load_u64("x1", abi::stack_pointer(), new_buf_slot));
        self.emit(abi::store_u64("x1", abi::stack_pointer(), map_slot));
        self.emit(abi::branch(&vwrite));

        // Write the new value at the aligned data tail; repoint the found entry.
        self.emit(abi::label(&vwrite));
        self.emit(abi::load_u64("x8", abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64("x9", "x8", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::store_u64("x9", abi::stack_pointer(), data_offset_slot));
        self.emit_align_offset_slot(data_offset_slot, value_align);
        // entryAddr = map + HEADER + foundIndex * ENTRY (the buffer may have moved).
        self.emit(abi::load_u64("x8", abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), found_index_slot));
        self.emit(abi::move_immediate(
            "x16",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x13", "x9", "x16"));
        self.emit(abi::add_immediate("x12", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers("x12", "x12", "x13"));
        self.emit(abi::store_u64("x12", abi::stack_pointer(), entry_addr_slot));
        // valueOffset = aligned data offset, valueLength = newValLen.
        self.emit(abi::load_u64("x13", abi::stack_pointer(), data_offset_slot));
        self.emit(abi::store_u64(
            "x13",
            "x12",
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64("x13", abi::stack_pointer(), val_len_slot));
        self.emit(abi::store_u64(
            "x13",
            "x12",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit_copy_payload_to_collection(
            map_slot,
            val_len_slot,
            &value_payload,
            data_offset_slot,
        )?;
        // dataLength = final data offset (includes the alignment pad + new value).
        self.emit(abi::load_u64("x8", abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), data_offset_slot));
        self.emit(abi::store_u64("x9", "x8", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::branch(&done));

        // --- Not found: compute the would-be new dataLength after the insert. ---
        self.emit(abi::label(&not_found));
        self.emit(abi::load_u64("x8", abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64("x9", "x8", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::store_u64("x9", abi::stack_pointer(), new_data_len_slot));
        self.emit_align_offset_slot(new_data_len_slot, key_align);
        self.emit(abi::load_u64("x8", abi::stack_pointer(), new_data_len_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), key_len_slot));
        self.emit(abi::add_registers("x8", "x8", "x9"));
        self.emit(abi::store_u64("x8", abi::stack_pointer(), new_data_len_slot));
        self.emit_align_offset_slot(new_data_len_slot, value_align);
        self.emit(abi::load_u64("x8", abi::stack_pointer(), new_data_len_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), val_len_slot));
        self.emit(abi::add_registers("x8", "x8", "x9"));
        self.emit(abi::store_u64("x8", abi::stack_pointer(), new_data_len_slot));
        // Room check: count < capacity AND newDataLength <= dataCapacity.
        self.emit(abi::load_u64("x8", abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64("x9", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64("x10", "x8", COLLECTION_OFFSET_CAPACITY));
        self.emit(abi::compare_registers("x9", "x10"));
        self.emit(abi::branch_ge(&grow));
        self.emit(abi::load_u64("x11", "x8", COLLECTION_OFFSET_DATA_CAPACITY));
        self.emit(abi::load_u64("x12", abi::stack_pointer(), new_data_len_slot));
        self.emit(abi::compare_registers("x12", "x11"));
        self.emit(abi::branch_hi(&grow));
        self.emit(abi::branch(&write));

        // --- Grow: geometric capacity + data, copy entries/data verbatim. ---
        self.emit(abi::label(&grow));
        self.emit(abi::load_u64("x8", abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64("x10", "x8", COLLECTION_OFFSET_CAPACITY));
        self.emit_geometric_step(
            "x10",
            "x14",
            "x15",
            COLLECTION_GROW_LOOKUP_INIT,
            COLLECTION_GROW_LOOKUP_TAPER,
            "mapset_grow_cap",
        );
        self.emit(abi::store_u64("x14", abi::stack_pointer(), new_cap_slot));
        self.emit(abi::load_u64("x8", abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64("x10", "x8", COLLECTION_OFFSET_DATA_CAPACITY));
        self.emit_geometric_step(
            "x10",
            "x14",
            "x15",
            COLLECTION_GROW_DATA_INIT,
            COLLECTION_GROW_DATA_TAPER,
            "mapset_grow_dcap",
        );
        // newDataCapacity = max(step(dataCapacity), newDataLength).
        self.emit(abi::load_u64("x11", abi::stack_pointer(), new_data_len_slot));
        self.emit(abi::compare_registers("x14", "x11"));
        self.emit(abi::branch_hi(&dcap_keep));
        self.emit(abi::branch_eq(&dcap_keep));
        self.emit(abi::move_register("x14", "x11"));
        self.emit(abi::label(&dcap_keep));
        self.emit(abi::store_u64("x14", abi::stack_pointer(), new_dcap_slot));
        // alloc = HEADER + newCapacity * ENTRY + newDataCapacity.
        self.emit(abi::load_u64("x14", abi::stack_pointer(), new_cap_slot));
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
        self.emit(abi::load_u64("x15", abi::stack_pointer(), new_dcap_slot));
        self.emit(abi::add_registers(
            abi::return_register(),
            abi::return_register(),
            "x15",
        ));
        // Reserve the map hash bucket region (x14 = new capacity).
        self.emit_reserve_map_buckets(true, "x14", abi::return_register(), "x16");
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: RelocIntent::Call,
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
        self.emit(abi::store_u64("x1", abi::stack_pointer(), new_buf_slot));
        // Header: old count / old dataLength, new capacity / data capacity.
        self.emit(abi::load_u64("x8", abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64("x9", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64("x11", "x8", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::load_u64("x14", abi::stack_pointer(), new_cap_slot));
        self.emit(abi::load_u64("x15", abi::stack_pointer(), new_dcap_slot));
        self.emit(abi::load_u64("x1", abi::stack_pointer(), new_buf_slot));
        self.emit_write_collection_header_full(&layout, "x1", "x9", "x14", "x11", "x15");
        // Copy the data region verbatim (dataLength bytes), capacity-based base.
        self.emit(abi::load_u64("x1", abi::stack_pointer(), new_buf_slot));
        self.emit_collection_data_pointer("x17", "x1");
        self.emit(abi::load_u64("x8", abi::stack_pointer(), map_slot));
        self.emit_collection_data_pointer("x20", "x8");
        self.emit(abi::load_u64("x14", "x8", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit_block_copy_advance("x17", "x20", "x14", "x22", "mapset_grow_data");
        // Copy the live lookup entries verbatim (count * ENTRY bytes).
        self.emit(abi::load_u64("x1", abi::stack_pointer(), new_buf_slot));
        self.emit(abi::add_immediate("x17", "x1", COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64("x8", abi::stack_pointer(), map_slot));
        self.emit(abi::add_immediate("x20", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64("x9", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::move_immediate(
            "x16",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x21", "x9", "x16"));
        self.emit_block_copy_advance("x17", "x20", "x21", "x22", "mapset_grow_entries");
        self.emit(abi::load_u64("x1", abi::stack_pointer(), new_buf_slot));
        self.emit(abi::store_u64("x1", abi::stack_pointer(), map_slot));
        self.emit(abi::branch(&write));

        // --- Write the new entry into slot[count], key+value aligned in data. ---
        self.emit(abi::label(&write));
        self.emit(abi::load_u64("x8", abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64("x9", "x8", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::store_u64("x9", abi::stack_pointer(), data_offset_slot));
        // entryAddr = map + HEADER + count * ENTRY.
        self.emit(abi::load_u64("x9", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::move_immediate(
            "x16",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x13", "x9", "x16"));
        self.emit(abi::add_immediate("x12", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers("x12", "x12", "x13"));
        self.emit(abi::store_u64("x12", abi::stack_pointer(), entry_addr_slot));
        self.emit(abi::move_immediate(
            "x13",
            "Byte",
            &COLLECTION_ENTRY_FLAG_USED.to_string(),
        ));
        self.emit(abi::store_u8("x13", "x12", COLLECTION_ENTRY_OFFSET_FLAGS));
        // Key: align, record keyOffset/keyLength, copy bytes.
        self.emit_align_offset_slot(data_offset_slot, key_align);
        self.emit(abi::load_u64("x12", abi::stack_pointer(), entry_addr_slot));
        self.emit(abi::load_u64("x13", abi::stack_pointer(), data_offset_slot));
        self.emit(abi::store_u64(
            "x13",
            "x12",
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::load_u64("x13", abi::stack_pointer(), key_len_slot));
        self.emit(abi::store_u64(
            "x13",
            "x12",
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit_copy_payload_to_collection(map_slot, key_len_slot, &key_payload, data_offset_slot)?;
        // Value: align, record valueOffset/valueLength, copy bytes.
        self.emit_align_offset_slot(data_offset_slot, value_align);
        self.emit(abi::load_u64("x12", abi::stack_pointer(), entry_addr_slot));
        self.emit(abi::load_u64("x13", abi::stack_pointer(), data_offset_slot));
        self.emit(abi::store_u64(
            "x13",
            "x12",
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64("x13", abi::stack_pointer(), val_len_slot));
        self.emit(abi::store_u64(
            "x13",
            "x12",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit_copy_payload_to_collection(
            map_slot,
            val_len_slot,
            &value_payload,
            data_offset_slot,
        )?;
        // Header: count++, dataLength = final data offset.
        self.emit(abi::load_u64("x8", abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64("x9", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::add_immediate("x9", "x9", 1));
        self.emit(abi::store_u64("x9", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), data_offset_slot));
        self.emit(abi::store_u64("x9", "x8", COLLECTION_OFFSET_DATA_LENGTH));
        // Keep the hash index current: if the buckets are already built (a prior
        // probe), insert the new entry incrementally so a build-via-`set` loop stays
        // O(n). The grow path reset the ready flag (the bucket region moved), so it
        // falls through here and is rebuilt lazily on the next probe. The 2*capacity
        // load factor guarantees a free slot for a spare-slot insert.
        let skip_put = self.label("mapset_skip_put");
        self.emit(abi::load_u8("x9", "x8", COLLECTION_OFFSET_BUCKETS_READY));
        self.emit(abi::compare_immediate("x9", "0"));
        self.emit(abi::branch_eq(&skip_put));
        self.emit(abi::load_u64("x0", abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64("x1", "x0", COLLECTION_OFFSET_COUNT));
        self.emit(abi::subtract_immediate("x1", "x1", 1)); // new entry index
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
            kind: RelocIntent::Call,
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

    /// Write a tight collection header: `capacity == count`, `dataCapacity ==
    /// dataLength` (no headroom). Used by the splice/remove paths that produce a
    /// fresh exact-sized buffer (plan-01 §4.3 — snapshots stay tight).
    pub(super) fn emit_write_list_header_from_registers(
        &mut self,
        layout: &CollectionTypeLayout,
        collection: &str,
        count: &str,
        data_len: &str,
    ) {
        self.emit_write_collection_header_full(
            layout, collection, count, count, data_len, data_len,
        );
    }

    /// Write a collection header with `capacity`/`dataCapacity` distinct from the
    /// live `count`/`dataLength` — the headroom form used by the append grow path
    /// (plan-01 §4.2). `capacity >= count` and `dataCapacity >= dataLength` must
    /// hold; the data region base is computed from `capacity`, so the writer and
    /// every reader agree via `emit_collection_data_pointer`. Uses x22 scratch.
    pub(super) fn emit_write_collection_header_full(
        &mut self,
        layout: &CollectionTypeLayout,
        collection: &str,
        count: &str,
        capacity: &str,
        data_len: &str,
        data_cap: &str,
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
        // Mark the map hash index not-ready (built lazily on first probe); a no-op
        // field for lists. Fresh, grown, and copied collections all reset it here.
        self.emit(abi::move_immediate("x22", "Byte", "0"));
        self.emit(abi::store_u8(
            "x22",
            collection,
            COLLECTION_OFFSET_BUCKETS_READY,
        ));
        self.emit(abi::store_u64(count, collection, COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64(
            capacity,
            collection,
            COLLECTION_OFFSET_CAPACITY,
        ));
        self.emit(abi::store_u64(
            data_len,
            collection,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::store_u64(
            data_cap,
            collection,
            COLLECTION_OFFSET_DATA_CAPACITY,
        ));
    }

    /// Emit `out_reg = geometric_step(value_reg)` per the plan-01 §5 growth shape:
    /// `0 -> init`; `value < threshold -> value * 2`; otherwise `value * 1.5`
    /// (taper the multiplier once large). All-integer, rounds down on the ×1.5
    /// (still strictly larger than `value`). `value_reg`, `out_reg`, and `scratch`
    /// must be three distinct registers; `value_reg` is preserved.
    pub(super) fn emit_geometric_step(
        &mut self,
        value_reg: &str,
        out_reg: &str,
        scratch: &str,
        init: usize,
        threshold: usize,
        prefix: &str,
    ) {
        let small = self.label(&format!("{prefix}_double"));
        let init_label = self.label(&format!("{prefix}_init"));
        let after = self.label(&format!("{prefix}_after"));
        self.emit(abi::compare_immediate(value_reg, "0"));
        self.emit(abi::branch_eq(&init_label));
        self.emit(abi::move_immediate(
            scratch,
            "Integer",
            &threshold.to_string(),
        ));
        self.emit(abi::compare_registers(value_reg, scratch));
        self.emit(abi::branch_lo(&small));
        // ×1.5: out = value + value/2.
        self.emit(abi::shift_right_immediate(out_reg, value_reg, 1));
        self.emit(abi::add_registers(out_reg, value_reg, out_reg));
        self.emit(abi::branch(&after));
        self.emit(abi::label(&small));
        self.emit(abi::shift_left_immediate(out_reg, value_reg, 1)); // ×2
        self.emit(abi::branch(&after));
        self.emit(abi::label(&init_label));
        self.emit(abi::move_immediate(out_reg, "Integer", &init.to_string()));
        self.emit(abi::label(&after));
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
        // Reserve the map hash bucket region (x12 = total count = capacity).
        self.emit_reserve_map_buckets(true, "x12", abi::return_register(), "x15");
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: RelocIntent::Call,
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
        self.emit(abi::load_u64(
            "x22",
            "x12",
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::add_registers("x22", "x22", "x14"));
        self.emit(abi::store_u64(
            "x22",
            "x17",
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::load_u64(
            "x22",
            "x12",
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::store_u64(
            "x22",
            "x17",
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
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
        // Reserve the map hash bucket region (x14 = remaining count = capacity).
        self.emit_reserve_map_buckets(true, "x14", abi::return_register(), "x16");
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: RelocIntent::Call,
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
}
