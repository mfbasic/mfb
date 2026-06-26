use super::*;

impl CodeBuilder<'_> {
    /// `collections::get`/`getOr` extract an element as a borrow into the
    /// container's data region for inline composite / nested-collection payloads
    /// (`emit_load_collection_payload`). By value semantics `get` returns an
    /// **owned** value the caller may bind, store, and free, so copy such a
    /// borrow into a standalone arena block (scalars are by-value and `String`
    /// is already materialized fresh, so they pass through). plan-02 Phase 8.
    pub(super) fn materialize_owned_element(
        &mut self,
        result: ValueResult,
    ) -> Result<ValueResult, String> {
        if self.is_freeable_flat_value(&result.type_) && result.type_ != "String" {
            let copied = self.copy_flat_block(&result.type_, &result.location)?;
            return Ok(ValueResult {
                type_: result.type_,
                location: copied,
                text: result.text,
            });
        }
        Ok(result)
    }

    pub(super) fn lower_collection_get(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let collection = self.lower_value(&args[0])?;
        let collection_slot = self.allocate_stack_object("get_collection", 8);
        self.emit(abi::store_u64(
            &collection.location,
            abi::stack_pointer(),
            collection_slot,
        ));

        let key = self.lower_value(&args[1])?;
        let key_slot = self.allocate_stack_object("get_key", 8);
        self.emit(abi::store_u64(
            &key.location,
            abi::stack_pointer(),
            key_slot,
        ));

        if let Some(element_type) = list_element_type(&collection.type_) {
            if key.type_ != "Integer" {
                return Err(format!(
                    "native collection get list index must be Integer, got {}",
                    key.type_
                ));
            }
            let result =
                self.lower_list_get(collection_slot, key_slot, &collection.type_, &element_type)?;
            return self.materialize_owned_element(result);
        }

        if let Some((key_type, value_type)) = map_type_parts(&collection.type_) {
            if key.type_ != key_type {
                return Err(format!(
                    "native collection get map key must be {}, got {}",
                    key_type, key.type_
                ));
            }
            let result = self.lower_map_get(
                collection_slot,
                key_slot,
                &collection.type_,
                &key_type,
                &value_type,
            )?;
            return self.materialize_owned_element(result);
        }

        Err(format!(
            "native collection get does not accept {}",
            collection.type_
        ))
    }

    pub(super) fn lower_collection_contains(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let collection = self.lower_value(&args[0])?;
        let collection_slot = self.allocate_stack_object("contains_collection", 8);
        self.emit(abi::store_u64(
            &collection.location,
            abi::stack_pointer(),
            collection_slot,
        ));

        let item = self.lower_value(&args[1])?;
        let item_slot = self.allocate_stack_object("contains_item", 8);
        self.emit(abi::store_u64(
            &item.location,
            abi::stack_pointer(),
            item_slot,
        ));

        let Some(element_type) = list_element_type(&collection.type_) else {
            return Err(format!(
                "native collection contains does not accept {}",
                collection.type_
            ));
        };
        if item.type_ != element_type {
            return Err(format!(
                "native collection contains item must be {}, got {}",
                element_type, item.type_
            ));
        }

        self.reset_temporary_registers();
        let collection_register = self.allocate_register()?;
        let item_register = self.allocate_register()?;
        let count = self.allocate_register()?;
        let index = self.allocate_register()?;
        let entry = self.allocate_register()?;
        let value_offset = self.allocate_register()?;
        let value_length = self.allocate_register()?;
        let result = self.allocate_register()?;
        let loop_label = self.label("contains_loop");
        let found = self.label("contains_found");
        let next = self.label("contains_next");
        let not_found = self.label("contains_not_found");
        let done = self.label("contains_done");

        self.emit(abi::load_u64(
            &collection_register,
            abi::stack_pointer(),
            collection_slot,
        ));
        self.emit(abi::load_u64(
            &item_register,
            abi::stack_pointer(),
            item_slot,
        ));
        self.emit(abi::load_u64(
            &count,
            &collection_register,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::move_immediate(&index, "Integer", "0"));
        self.emit(abi::add_immediate(
            &entry,
            &collection_register,
            COLLECTION_HEADER_SIZE,
        ));

        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_registers(&index, &count));
        self.emit(abi::branch_ge(&not_found));
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
        self.emit_collection_payload_match_branch(
            &element_type,
            &collection_register,
            &value_offset,
            &value_length,
            &item_register,
            &found,
            &next,
        )?;

        self.emit(abi::label(&found));
        self.emit(abi::move_immediate(&result, "Boolean", "true"));
        self.emit(abi::branch(&done));

        self.emit(abi::label(&next));
        self.emit(abi::add_immediate(&entry, &entry, COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate(&index, &index, 1));
        self.emit(abi::branch(&loop_label));

        self.emit(abi::label(&not_found));
        self.emit(abi::move_immediate(&result, "Boolean", "false"));
        self.emit(abi::label(&done));

        Ok(ValueResult {
            type_: "Boolean".to_string(),
            location: result,
            text: format!("contains({}, {})", collection.type_, element_type),
        })
    }

    pub(super) fn lower_collection_get_or(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let collection = self.lower_value(&args[0])?;
        let collection_slot = self.allocate_stack_object("get_or_collection", 8);
        self.emit(abi::store_u64(
            &collection.location,
            abi::stack_pointer(),
            collection_slot,
        ));

        let key = self.lower_value(&args[1])?;
        let key_slot = self.allocate_stack_object("get_or_key", 8);
        self.emit(abi::store_u64(
            &key.location,
            abi::stack_pointer(),
            key_slot,
        ));

        let default = self.lower_value(&args[2])?;
        let default_slot = self.allocate_stack_object("get_or_default", 8);
        self.emit(abi::store_u64(
            &default.location,
            abi::stack_pointer(),
            default_slot,
        ));

        if let Some(element_type) = list_element_type(&collection.type_) {
            if key.type_ != "Integer" {
                return Err(format!(
                    "native collection getOr list index must be Integer, got {}",
                    key.type_
                ));
            }
            if default.type_ != element_type {
                return Err(format!(
                    "native collection getOr default must be {}, got {}",
                    element_type, default.type_
                ));
            }
            let result = self.lower_list_get_or(
                collection_slot,
                key_slot,
                default_slot,
                &collection.type_,
                &element_type,
            )?;
            return self.materialize_owned_element(result);
        }

        if let Some((key_type, value_type)) = map_type_parts(&collection.type_) {
            if key.type_ != key_type {
                return Err(format!(
                    "native collection getOr map key must be {}, got {}",
                    key_type, key.type_
                ));
            }
            if default.type_ != value_type {
                return Err(format!(
                    "native collection getOr default must be {}, got {}",
                    value_type, default.type_
                ));
            }
            let result = self.lower_map_get_or(
                collection_slot,
                key_slot,
                default_slot,
                &collection.type_,
                &key_type,
                &value_type,
            )?;
            return self.materialize_owned_element(result);
        }

        Err(format!(
            "native collection getOr does not accept {}",
            collection.type_
        ))
    }

    pub(super) fn lower_collection_has_key(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let collection = self.lower_value(&args[0])?;
        let collection_slot = self.allocate_stack_object("has_key_collection", 8);
        self.emit(abi::store_u64(
            &collection.location,
            abi::stack_pointer(),
            collection_slot,
        ));
        let key = self.lower_value(&args[1])?;
        let key_slot = self.allocate_stack_object("has_key_key", 8);
        self.emit(abi::store_u64(
            &key.location,
            abi::stack_pointer(),
            key_slot,
        ));

        let Some((key_type, _)) = map_type_parts(&collection.type_) else {
            return Err(format!(
                "native collection hasKey does not accept {}",
                collection.type_
            ));
        };
        if key.type_ != key_type {
            return Err(format!(
                "native collection hasKey key must be {}, got {}",
                key_type, key.type_
            ));
        }

        self.reset_temporary_registers();
        let result = self.allocate_register()?;
        let loop_label = self.label("has_key_loop");
        let found = self.label("has_key_found");
        let next = self.label("has_key_next");
        let not_found = self.label("has_key_not_found");
        let done = self.label("has_key_done");

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
        self.emit_collection_payload_matches_value_branch(
            &key_type, "x8", "x13", "x14", "x9", &found, &next,
        )?;
        self.emit(abi::label(&found));
        self.emit(abi::move_immediate(&result, "Boolean", "true"));
        self.emit(abi::branch(&done));
        self.emit(abi::label(&next));
        self.emit(abi::add_immediate("x12", "x12", COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate("x11", "x11", 1));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&not_found));
        self.emit(abi::move_immediate(&result, "Boolean", "false"));
        self.emit(abi::label(&done));

        Ok(ValueResult {
            type_: "Boolean".to_string(),
            location: result,
            text: format!("hasKey({}, {})", collection.type_, key_type),
        })
    }

    pub(super) fn lower_collection_keys(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let collection = self.lower_value(&args[0])?;
        let Some((key_type, _)) = map_type_parts(&collection.type_) else {
            return Err(format!(
                "native collection keys does not accept {}",
                collection.type_
            ));
        };
        self.lower_map_projection(&collection, &key_type, true)
    }

    pub(super) fn lower_collection_values_builtin(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let collection = self.lower_value(&args[0])?;
        let Some((_, value_type)) = map_type_parts(&collection.type_) else {
            return Err(format!(
                "native collection values does not accept {}",
                collection.type_
            ));
        };
        self.lower_map_projection(&collection, &value_type, false)
    }

    pub(super) fn lower_map_projection(
        &mut self,
        collection: &ValueResult,
        element_type: &str,
        project_key: bool,
    ) -> Result<ValueResult, String> {
        let collection_slot = self.allocate_stack_object("map_projection_collection", 8);
        self.emit(abi::store_u64(
            &collection.location,
            abi::stack_pointer(),
            collection_slot,
        ));
        let layout = CollectionTypeLayout::from_type(&format!("List OF {element_type}"))
            .ok_or_else(|| {
                format!("native code collection type 'List OF {element_type}' is not supported")
            })?;
        for register in ["x20", "x21", "x22", "x23", "x24", "x25"] {
            self.mark_register_used(register);
        }
        let data_len_slot = self.allocate_stack_object("map_projection_data_len", 8);
        let result_slot = self.allocate_stack_object("map_projection_result", 8);
        let length_loop = self.label("map_projection_length_loop");
        let length_done = self.label("map_projection_length_done");
        let alloc_ok = self.label("map_projection_alloc_ok");
        let copy_loop = self.label("map_projection_copy_loop");
        let copy_bytes = self.label("map_projection_copy_bytes");
        let copy_bytes_done = self.label("map_projection_copy_bytes_done");
        let copy_done = self.label("map_projection_copy_done");
        let offset_field = if project_key {
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET
        } else {
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET
        };
        let length_field = if project_key {
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH
        } else {
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH
        };

        self.emit(abi::load_u64("x8", abi::stack_pointer(), collection_slot));
        self.emit(abi::load_u64("x9", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::move_immediate("x10", "Integer", "0"));
        self.emit(abi::move_immediate("x11", "Integer", "0"));
        self.emit(abi::add_immediate("x12", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::label(&length_loop));
        self.emit(abi::compare_registers("x10", "x9"));
        self.emit(abi::branch_ge(&length_done));
        self.emit(abi::load_u64("x13", "x12", length_field));
        self.emit(abi::add_registers("x11", "x11", "x13"));
        self.emit(abi::add_immediate("x12", "x12", COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate("x10", "x10", 1));
        self.emit(abi::branch(&length_loop));
        self.emit(abi::label(&length_done));
        self.emit(abi::store_u64("x11", abi::stack_pointer(), data_len_slot));
        self.emit(abi::move_immediate(
            "x14",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x15", "x9", "x14"));
        self.emit(abi::add_immediate(
            abi::return_register(),
            "x15",
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(
            abi::return_register(),
            abi::return_register(),
            "x11",
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
        self.emit(abi::move_immediate("x13", "Byte", &layout.kind.to_string()));
        self.emit(abi::store_u8("x13", "x1", COLLECTION_OFFSET_KIND));
        self.emit(abi::move_immediate(
            "x13",
            "Byte",
            &layout.key_type_code.to_string(),
        ));
        self.emit(abi::store_u8("x13", "x1", COLLECTION_OFFSET_KEY_TYPE));
        self.emit(abi::move_immediate(
            "x13",
            "Byte",
            &layout.value_type_code.to_string(),
        ));
        self.emit(abi::store_u8("x13", "x1", COLLECTION_OFFSET_VALUE_TYPE));
        self.emit(abi::move_immediate("x13", "Byte", "1"));
        self.emit(abi::store_u8("x13", "x1", COLLECTION_OFFSET_FLAGS_VERSION));
        self.emit(abi::load_u64("x9", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64("x9", "x1", COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64("x9", "x1", COLLECTION_OFFSET_CAPACITY));
        self.emit(abi::load_u64("x11", abi::stack_pointer(), data_len_slot));
        self.emit(abi::store_u64("x11", "x1", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::store_u64("x11", "x1", COLLECTION_OFFSET_DATA_CAPACITY));

        self.emit(abi::load_u64("x8", abi::stack_pointer(), collection_slot));
        self.emit(abi::load_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64("x9", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::add_immediate("x12", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::add_immediate("x17", "x1", COLLECTION_HEADER_SIZE));
        self.emit_collection_data_pointer("x20", "x8");
        self.emit(abi::move_immediate(
            "x14",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x21", "x9", "x14"));
        self.emit(abi::add_registers("x21", "x17", "x21"));
        self.emit(abi::move_immediate("x10", "Integer", "0"));
        self.emit(abi::move_immediate("x11", "Integer", "0"));
        self.emit(abi::label(&copy_loop));
        self.emit(abi::compare_registers("x10", "x9"));
        self.emit(abi::branch_ge(&copy_done));
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
        self.emit(abi::load_u64("x22", "x12", offset_field));
        self.emit(abi::load_u64("x23", "x12", length_field));
        self.emit(abi::store_u64(
            "x11",
            "x17",
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::store_u64(
            "x23",
            "x17",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers("x24", "x20", "x22"));
        self.emit(abi::add_registers("x25", "x21", "x11"));
        self.emit(abi::label(&copy_bytes));
        self.emit(abi::compare_immediate("x23", "0"));
        self.emit(abi::branch_eq(&copy_bytes_done));
        self.emit(abi::load_u8("x22", "x24", 0));
        self.emit(abi::store_u8("x22", "x25", 0));
        self.emit(abi::add_immediate("x24", "x24", 1));
        self.emit(abi::add_immediate("x25", "x25", 1));
        self.emit(abi::subtract_immediate("x23", "x23", 1));
        self.emit(abi::branch(&copy_bytes));
        self.emit(abi::label(&copy_bytes_done));
        self.emit(abi::load_u64(
            "x23",
            "x17",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers("x11", "x11", "x23"));
        self.emit(abi::add_immediate("x12", "x12", COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate("x17", "x17", COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate("x10", "x10", 1));
        self.emit(abi::branch(&copy_loop));
        self.emit(abi::label(&copy_done));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: format!("List OF {element_type}"),
            location: result,
            text: if project_key {
                format!("keys({})", collection.type_)
            } else {
                format!("values({})", collection.type_)
            },
        })
    }

    pub(super) fn lower_collection_sum(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let collection = self.lower_value(&args[0])?;
        let Some(element_type) = list_element_type(&collection.type_) else {
            return Err(format!(
                "native collection sum does not accept {}",
                collection.type_
            ));
        };
        if !matches!(element_type.as_str(), "Integer" | "Float" | "Fixed") {
            return Err(format!(
                "native collection sum does not accept {}",
                collection.type_
            ));
        }
        let collection_slot = self.allocate_stack_object("sum_collection", 8);
        self.emit(abi::store_u64(
            &collection.location,
            abi::stack_pointer(),
            collection_slot,
        ));
        let loop_label = self.label("sum_loop");
        let done = self.label("sum_done");
        self.emit(abi::load_u64("x8", abi::stack_pointer(), collection_slot));
        self.emit(abi::load_u64("x9", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::move_immediate("x10", "Integer", "0"));
        self.emit(abi::add_immediate("x11", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::move_immediate("x14", &element_type, "0"));
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_registers("x10", "x9"));
        self.emit(abi::branch_ge(&done));
        self.emit(abi::load_u64(
            "x12",
            "x11",
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit_collection_data_pointer("x15", "x8");
        self.emit(abi::add_registers("x15", "x15", "x12"));
        match element_type.as_str() {
            "Integer" => {
                self.emit(abi::load_u64("x16", "x15", 0));
                self.emit_checked_integer_add("x14", "x14", "x16")?;
            }
            "Float" => {
                self.emit(abi::load_u64("x16", "x15", 0));
                self.emit(abi::float_move_d_from_x("d0", "x14"));
                self.emit(abi::float_move_d_from_x("d1", "x16"));
                self.emit(abi::float_add_d("d0", "d0", "d1"));
                self.emit(abi::float_move_x_from_d("x14", "d0"));
            }
            "Fixed" => {
                self.emit(abi::load_u64("x16", "x15", 0));
                self.emit_checked_integer_add("x14", "x14", "x16")?;
            }
            _ => unreachable!(),
        }
        self.emit(abi::add_immediate("x11", "x11", COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate("x10", "x10", 1));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&done));
        let result = self.allocate_register()?;
        self.emit(abi::move_register(&result, "x14"));
        Ok(ValueResult {
            type_: element_type,
            location: result,
            text: format!("sum({})", collection.type_),
        })
    }

    pub(super) fn lower_collection_for_each_call(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let collection = self.lower_value(&args[0])?;
        let Some(element_type) = list_element_type(&collection.type_) else {
            return Err(format!(
                "native collection forEach does not accept {}",
                collection.type_
            ));
        };
        let action = self.lower_value(&args[1])?;
        if !action.type_.starts_with("FUNC(") {
            return Err(format!(
                "native collection forEach action must be a function, got {}",
                action.type_
            ));
        }
        if action.location == "void" {
            return Err(
                "native collection forEach action does not have a callable location".to_string(),
            );
        }
        let action_slot = self.allocate_stack_object("for_each_call_action", 8);
        self.emit(abi::store_u64(
            &action.location,
            abi::stack_pointer(),
            action_slot,
        ));
        let collection_slot = self.allocate_stack_object("for_each_call_collection", 8);
        let cursor_slot = self.allocate_stack_object("for_each_call_cursor", 8);
        let remaining_slot = self.allocate_stack_object("for_each_call_remaining", 8);
        self.emit(abi::store_u64(
            &collection.location,
            abi::stack_pointer(),
            collection_slot,
        ));
        self.emit(abi::load_u64("x8", abi::stack_pointer(), collection_slot));
        self.emit(abi::load_u64("x9", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::add_immediate("x10", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::store_u64("x10", abi::stack_pointer(), cursor_slot));
        self.emit(abi::store_u64("x9", abi::stack_pointer(), remaining_slot));
        let loop_label = self.label("for_each_call_loop");
        let ok_label = self.label("for_each_call_ok");
        let done = self.label("for_each_call_done");
        self.emit(abi::label(&loop_label));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), remaining_slot));
        self.emit(abi::compare_immediate("x9", "0"));
        self.emit(abi::branch_eq(&done));
        self.emit(abi::load_u64("x10", abi::stack_pointer(), cursor_slot));
        self.emit(abi::load_u64(
            "x11",
            "x10",
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            "x12",
            "x10",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::load_u64("x8", abi::stack_pointer(), collection_slot));
        let item = self.emit_load_collection_payload(&element_type, "x8", "x11", "x12")?;
        self.emit(abi::move_register(&abi::argument_register(0)?, &item));
        self.emit(abi::load_u64("x17", abi::stack_pointer(), action_slot));
        self.emit_direct_callable_branch("x17");
        self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
        self.emit(abi::branch_eq(&ok_label));
        self.emit(abi::return_());
        self.emit(abi::label(&ok_label));
        self.emit(abi::load_u64("x10", abi::stack_pointer(), cursor_slot));
        self.emit(abi::add_immediate("x10", "x10", COLLECTION_ENTRY_SIZE));
        self.emit(abi::store_u64("x10", abi::stack_pointer(), cursor_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), remaining_slot));
        self.emit(abi::subtract_immediate("x9", "x9", 1));
        self.emit(abi::store_u64("x9", abi::stack_pointer(), remaining_slot));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&done));
        Ok(ValueResult {
            type_: "Nothing".to_string(),
            location: "void".to_string(),
            text: format!("forEach({}, {})", collection.type_, action.text),
        })
    }

    pub(super) fn lower_collection_transform_call(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let collection = self.lower_value(&args[0])?;
        let Some(element_type) = list_element_type(&collection.type_) else {
            return Err(format!(
                "native collection transform does not accept {}",
                collection.type_
            ));
        };
        let collection_slot = self.allocate_stack_object("transform_collection", 8);
        self.emit(abi::store_u64(
            &collection.location,
            abi::stack_pointer(),
            collection_slot,
        ));
        let action = self.lower_value(&args[1])?;
        let output_type = callable_return_type(&action.type_).ok_or_else(|| {
            format!(
                "native collection transform action must be a function, got {}",
                action.type_
            )
        })?;
        self.require_direct_callable("transform", &action)?;
        let action_slot = self.allocate_stack_object("transform_action", 8);
        self.emit(abi::store_u64(
            &action.location,
            abi::stack_pointer(),
            action_slot,
        ));
        let output_list_type = format!("List OF {output_type}");
        let output = self.lower_empty_collection(&output_list_type)?;
        let output_slot = self.allocate_stack_object("transform_output", 8);
        let cursor_slot = self.allocate_stack_object("transform_cursor", 8);
        let remaining_slot = self.allocate_stack_object("transform_remaining", 8);
        self.emit(abi::store_u64(
            &output.location,
            abi::stack_pointer(),
            output_slot,
        ));
        self.initialize_collection_loop_slots(collection_slot, cursor_slot, remaining_slot);

        let loop_label = self.label("transform_call_loop");
        let ok_label = self.label("transform_call_ok");
        let done = self.label("transform_call_done");
        self.emit(abi::label(&loop_label));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), remaining_slot));
        self.emit(abi::compare_immediate("x9", "0"));
        self.emit(abi::branch_eq(&done));
        let item = self.load_collection_loop_item(collection_slot, cursor_slot, &element_type)?;
        self.emit(abi::move_register(&abi::argument_register(0)?, &item));
        self.emit(abi::load_u64("x17", abi::stack_pointer(), action_slot));
        self.emit_direct_callable_branch("x17");
        self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
        self.emit(abi::branch_eq(&ok_label));
        self.emit(abi::return_());
        self.emit(abi::label(&ok_label));

        let item_slot = self.allocate_stack_object("transform_item", 8);
        self.emit(abi::store_u64(
            RESULT_VALUE_REGISTER,
            abi::stack_pointer(),
            item_slot,
        ));
        // The output accumulator is a private, uniquely-owned buffer, so append
        // each transformed item in place with geometric headroom (plan-01 §4.2)
        // — amortized O(1) instead of the O(n) splice the singleton+insert did.
        self.lower_list_append_in_place(output_slot, item_slot, &output_list_type, &output_type)?;
        self.advance_collection_loop(cursor_slot, remaining_slot, &loop_label);
        self.emit(abi::label(&done));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), output_slot));
        Ok(ValueResult {
            type_: output_list_type,
            location: result,
            text: format!("transform({}, {})", collection.type_, action.text),
        })
    }

    pub(super) fn lower_collection_filter_call(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let collection = self.lower_value(&args[0])?;
        let Some(element_type) = list_element_type(&collection.type_) else {
            return Err(format!(
                "native collection filter does not accept {}",
                collection.type_
            ));
        };
        let collection_slot = self.allocate_stack_object("filter_collection", 8);
        self.emit(abi::store_u64(
            &collection.location,
            abi::stack_pointer(),
            collection_slot,
        ));
        let action = self.lower_value(&args[1])?;
        let output_type = callable_return_type(&action.type_).ok_or_else(|| {
            format!(
                "native collection filter predicate must be a function, got {}",
                action.type_
            )
        })?;
        if output_type != "Boolean" {
            return Err(format!(
                "native collection filter predicate must return Boolean, got {output_type}"
            ));
        }
        self.require_direct_callable("filter", &action)?;
        let action_slot = self.allocate_stack_object("filter_action", 8);
        self.emit(abi::store_u64(
            &action.location,
            abi::stack_pointer(),
            action_slot,
        ));
        let output = self.lower_empty_collection(&collection.type_)?;
        let output_slot = self.allocate_stack_object("filter_output", 8);
        let cursor_slot = self.allocate_stack_object("filter_cursor", 8);
        let remaining_slot = self.allocate_stack_object("filter_remaining", 8);
        let item_slot = self.allocate_stack_object("filter_item", 8);
        self.emit(abi::store_u64(
            &output.location,
            abi::stack_pointer(),
            output_slot,
        ));
        self.initialize_collection_loop_slots(collection_slot, cursor_slot, remaining_slot);

        let loop_label = self.label("filter_call_loop");
        let ok_label = self.label("filter_call_ok");
        let keep_label = self.label("filter_call_keep");
        let skip_label = self.label("filter_call_skip");
        let done = self.label("filter_call_done");
        self.emit(abi::label(&loop_label));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), remaining_slot));
        self.emit(abi::compare_immediate("x9", "0"));
        self.emit(abi::branch_eq(&done));
        let item = self.load_collection_loop_item(collection_slot, cursor_slot, &element_type)?;
        self.emit(abi::store_u64(&item, abi::stack_pointer(), item_slot));
        self.emit(abi::move_register(&abi::argument_register(0)?, &item));
        self.emit(abi::load_u64("x17", abi::stack_pointer(), action_slot));
        self.emit_direct_callable_branch("x17");
        self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
        self.emit(abi::branch_eq(&ok_label));
        self.emit(abi::return_());
        self.emit(abi::label(&ok_label));
        self.emit(abi::compare_immediate(RESULT_VALUE_REGISTER, "0"));
        self.emit(abi::branch_ne(&keep_label));
        self.emit(abi::branch(&skip_label));
        self.emit(abi::label(&keep_label));
        // Private accumulator → append in place with headroom (plan-01 §4.2).
        self.lower_list_append_in_place(output_slot, item_slot, &collection.type_, &element_type)?;
        self.emit(abi::label(&skip_label));
        self.advance_collection_loop(cursor_slot, remaining_slot, &loop_label);
        self.emit(abi::label(&done));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), output_slot));
        Ok(ValueResult {
            type_: collection.type_.clone(),
            location: result,
            text: format!("filter({}, {})", collection.type_, action.text),
        })
    }

    pub(super) fn lower_collection_reduce_call(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let collection = self.lower_value(&args[0])?;
        let Some(element_type) = list_element_type(&collection.type_) else {
            return Err(format!(
                "native collection reduce does not accept {}",
                collection.type_
            ));
        };
        let collection_slot = self.allocate_stack_object("reduce_collection", 8);
        self.emit(abi::store_u64(
            &collection.location,
            abi::stack_pointer(),
            collection_slot,
        ));
        let initial = self.lower_value(&args[1])?;
        let accumulator_slot = self.allocate_stack_object("reduce_accumulator", 8);
        self.emit(abi::store_u64(
            &initial.location,
            abi::stack_pointer(),
            accumulator_slot,
        ));
        let action = self.lower_value(&args[2])?;
        let output_type = callable_return_type(&action.type_).ok_or_else(|| {
            format!(
                "native collection reduce reducer must be a function, got {}",
                action.type_
            )
        })?;
        if output_type != initial.type_ {
            return Err(format!(
                "native collection reduce reducer must return {}, got {output_type}",
                initial.type_
            ));
        }
        self.require_direct_callable("reduce", &action)?;
        let action_slot = self.allocate_stack_object("reduce_action", 8);
        self.emit(abi::store_u64(
            &action.location,
            abi::stack_pointer(),
            action_slot,
        ));
        let cursor_slot = self.allocate_stack_object("reduce_cursor", 8);
        let remaining_slot = self.allocate_stack_object("reduce_remaining", 8);
        self.initialize_collection_loop_slots(collection_slot, cursor_slot, remaining_slot);

        let loop_label = self.label("reduce_call_loop");
        let ok_label = self.label("reduce_call_ok");
        let done = self.label("reduce_call_done");
        self.emit(abi::label(&loop_label));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), remaining_slot));
        self.emit(abi::compare_immediate("x9", "0"));
        self.emit(abi::branch_eq(&done));
        let item = self.load_collection_loop_item(collection_slot, cursor_slot, &element_type)?;
        self.emit(abi::load_u64(
            &abi::argument_register(0)?,
            abi::stack_pointer(),
            accumulator_slot,
        ));
        self.emit(abi::move_register(&abi::argument_register(1)?, &item));
        self.emit(abi::load_u64("x17", abi::stack_pointer(), action_slot));
        self.emit_direct_callable_branch("x17");
        self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
        self.emit(abi::branch_eq(&ok_label));
        self.emit(abi::return_());
        self.emit(abi::label(&ok_label));
        self.emit(abi::store_u64(
            RESULT_VALUE_REGISTER,
            abi::stack_pointer(),
            accumulator_slot,
        ));
        self.advance_collection_loop(cursor_slot, remaining_slot, &loop_label);
        self.emit(abi::label(&done));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(
            &result,
            abi::stack_pointer(),
            accumulator_slot,
        ));
        Ok(ValueResult {
            type_: initial.type_,
            location: result,
            text: format!(
                "reduce({}, {}, {})",
                collection.type_, initial.text, action.text
            ),
        })
    }

    pub(super) fn require_direct_callable(
        &self,
        name: &str,
        action: &ValueResult,
    ) -> Result<(), String> {
        if !action.type_.starts_with("FUNC(") {
            return Err(format!(
                "native collection {name} action must be a function, got {}",
                action.type_
            ));
        }
        if action.location == "void" {
            return Err(format!(
                "native collection {name} action does not have a callable location"
            ));
        }
        Ok(())
    }

    pub(super) fn emit_direct_callable_branch(&mut self, location: &str) {
        let saved_env_slot = self.allocate_stack_object("closure_saved_env", 8);
        let code_register = self
            .allocate_register()
            .expect("closure call needs a scratch register");
        let env_register = self
            .allocate_register()
            .expect("closure call needs a scratch register");
        self.emit(abi::store_u64(
            CLOSURE_ENV_REGISTER,
            abi::stack_pointer(),
            saved_env_slot,
        ));
        self.emit(abi::load_u64(&code_register, location, CLOSURE_OFFSET_CODE));
        self.emit(abi::load_u64(&env_register, location, CLOSURE_OFFSET_ENV));
        self.emit(abi::move_register(CLOSURE_ENV_REGISTER, &env_register));
        self.emit_callable_branch(&code_register);
        self.emit(abi::load_u64(
            CLOSURE_ENV_REGISTER,
            abi::stack_pointer(),
            saved_env_slot,
        ));
    }

    pub(super) fn emit_callable_branch(&mut self, location: &str) {
        if location.starts_with('x') {
            self.emit(abi::branch_link_register(location));
            return;
        }
        self.emit(abi::branch_link(location));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: location.to_string(),
            kind: "branch26".to_string(),
            binding: "internal".to_string(),
            library: None,
        });
    }

    pub(super) fn initialize_collection_loop_slots(
        &mut self,
        collection_slot: usize,
        cursor_slot: usize,
        remaining_slot: usize,
    ) {
        self.emit(abi::load_u64("x8", abi::stack_pointer(), collection_slot));
        self.emit(abi::load_u64("x9", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::add_immediate("x10", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::store_u64("x10", abi::stack_pointer(), cursor_slot));
        self.emit(abi::store_u64("x9", abi::stack_pointer(), remaining_slot));
    }

    pub(super) fn load_collection_loop_item(
        &mut self,
        collection_slot: usize,
        cursor_slot: usize,
        element_type: &str,
    ) -> Result<String, String> {
        self.emit(abi::load_u64("x10", abi::stack_pointer(), cursor_slot));
        self.emit(abi::load_u64(
            "x11",
            "x10",
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            "x12",
            "x10",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::load_u64("x8", abi::stack_pointer(), collection_slot));
        self.emit_load_collection_payload(element_type, "x8", "x11", "x12")
    }

    pub(super) fn advance_collection_loop(
        &mut self,
        cursor_slot: usize,
        remaining_slot: usize,
        loop_label: &str,
    ) {
        self.emit(abi::load_u64("x10", abi::stack_pointer(), cursor_slot));
        self.emit(abi::add_immediate("x10", "x10", COLLECTION_ENTRY_SIZE));
        self.emit(abi::store_u64("x10", abi::stack_pointer(), cursor_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), remaining_slot));
        self.emit(abi::subtract_immediate("x9", "x9", 1));
        self.emit(abi::store_u64("x9", abi::stack_pointer(), remaining_slot));
        self.emit(abi::branch(loop_label));
    }
}
