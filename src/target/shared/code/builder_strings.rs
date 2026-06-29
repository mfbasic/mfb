use super::*;

impl CodeBuilder<'_> {
    pub(super) fn lower_replace(&mut self, args: &[NirValue]) -> Result<ValueResult, String> {
        let value = self.lower_value(&args[0])?;
        if let Some(element_type) = list_element_type(&value.type_) {
            let value_slot = self.allocate_stack_object("replace_list_value", 8);
            self.emit(abi::store_u64(
                &value.location,
                abi::stack_pointer(),
                value_slot,
            ));
            let old = self.lower_value(&args[1])?;
            if old.type_ != element_type {
                return Err(format!(
                    "native list replace old must be {}, got {}",
                    element_type, old.type_
                ));
            }
            let old_slot = self.allocate_stack_object("replace_list_old", 8);
            self.emit(abi::store_u64(
                &old.location,
                abi::stack_pointer(),
                old_slot,
            ));
            let new = self.lower_value(&args[2])?;
            if new.type_ != element_type {
                return Err(format!(
                    "native list replace new must be {}, got {}",
                    element_type, new.type_
                ));
            }
            let new_slot = self.allocate_stack_object("replace_list_new", 8);
            self.emit(abi::store_u64(
                &new.location,
                abi::stack_pointer(),
                new_slot,
            ));
            return self.lower_list_replace(
                value_slot,
                old_slot,
                new_slot,
                &value.type_,
                &element_type,
            );
        }
        if value.type_ != "String" {
            return Err(format!(
                "native string replace value must be String, got {}",
                value.type_
            ));
        }
        let value_slot = self.allocate_stack_object("replace_value", 8);
        self.emit(abi::store_u64(
            &value.location,
            abi::stack_pointer(),
            value_slot,
        ));

        let old = self.lower_value(&args[1])?;
        if old.type_ != "String" {
            return Err(format!(
                "native string replace old must be String, got {}",
                old.type_
            ));
        }
        let old_slot = self.allocate_stack_object("replace_old", 8);
        self.emit(abi::store_u64(
            &old.location,
            abi::stack_pointer(),
            old_slot,
        ));

        let new = self.lower_value(&args[2])?;
        if new.type_ != "String" {
            return Err(format!(
                "native string replace new must be String, got {}",
                new.type_
            ));
        }
        let new_slot = self.allocate_stack_object("replace_new", 8);
        self.emit(abi::store_u64(
            &new.location,
            abi::stack_pointer(),
            new_slot,
        ));

        let result_slot = self.allocate_stack_object("replace_result", 8);
        let output_len_slot = self.allocate_stack_object("replace_output_len", 8);

        let value_ptr = "x8";
        let value_len = "x9";
        let old_ptr = "x10";
        let old_len = "x11";
        let new_ptr = "x12";
        let new_len = "x13";
        let index = "x14";
        let output_len = "x15";
        let last_start = "x16";
        let match_index = "x17";
        let candidate = "x20";
        let old_cursor = "x21";
        let value_byte = "x22";
        let old_byte = "x23";
        let dest = "x24";
        let new_cursor = "x25";
        let new_index = "x26";
        for register in [
            candidate, old_cursor, value_byte, old_byte, dest, new_cursor, new_index,
        ] {
            if abi::is_callee_saved(register)
                && !self.used_callee_saved.iter().any(|saved| saved == register)
            {
                self.used_callee_saved.push(register.to_string());
            }
        }

        let copy_original = self.label("replace_copy_original");
        let first_loop = self.label("replace_first_loop");
        let first_compare = self.label("replace_first_compare");
        let first_match = self.label("replace_first_match");
        let first_next = self.label("replace_first_next");
        let first_done = self.label("replace_first_done");
        let alloc_ok = self.label("replace_alloc_ok");
        let second_loop = self.label("replace_second_loop");
        let second_compare = self.label("replace_second_compare");
        let second_match = self.label("replace_second_match");
        let second_copy_new_loop = self.label("replace_second_copy_new_loop");
        let second_copy_new_done = self.label("replace_second_copy_new_done");
        let second_copy_one = self.label("replace_second_copy_one");
        let second_done = self.label("replace_second_done");
        let done = self.label("replace_done");
        let result = self.allocate_register()?;

        self.emit(abi::load_u64(value_ptr, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(old_ptr, abi::stack_pointer(), old_slot));
        self.emit(abi::load_u64(new_ptr, abi::stack_pointer(), new_slot));
        self.emit(abi::load_u64(value_len, value_ptr, 0));
        self.emit(abi::load_u64(old_len, old_ptr, 0));
        self.emit(abi::load_u64(new_len, new_ptr, 0));
        self.emit(abi::compare_immediate(old_len, "0"));
        self.emit(abi::branch_eq(&copy_original));
        self.emit(abi::compare_registers(old_len, value_len));
        self.emit(abi::branch_hi(&copy_original));
        self.emit(abi::add_immediate(value_ptr, value_ptr, 8));
        self.emit(abi::add_immediate(old_ptr, old_ptr, 8));
        self.emit(abi::add_immediate(new_ptr, new_ptr, 8));
        self.emit(abi::move_immediate(index, "Integer", "0"));
        self.emit(abi::move_register(output_len, value_len));
        self.emit(abi::subtract_registers(last_start, value_len, old_len));

        self.emit(abi::label(&first_loop));
        self.emit(abi::compare_registers(index, last_start));
        self.emit(abi::branch_hi(&first_done));
        self.emit(abi::move_immediate(match_index, "Integer", "0"));
        self.emit(abi::add_registers(candidate, value_ptr, index));
        self.emit(abi::move_register(old_cursor, old_ptr));
        self.emit(abi::label(&first_compare));
        self.emit(abi::compare_registers(match_index, old_len));
        self.emit(abi::branch_eq(&first_match));
        self.emit(abi::load_u8(value_byte, candidate, 0));
        self.emit(abi::load_u8(old_byte, old_cursor, 0));
        self.emit(abi::compare_registers(value_byte, old_byte));
        self.emit(abi::branch_ne(&first_next));
        self.emit(abi::add_immediate(candidate, candidate, 1));
        self.emit(abi::add_immediate(old_cursor, old_cursor, 1));
        self.emit(abi::add_immediate(match_index, match_index, 1));
        self.emit(abi::branch(&first_compare));

        self.emit(abi::label(&first_match));
        self.emit(abi::subtract_registers(output_len, output_len, old_len));
        self.emit(abi::add_registers(output_len, output_len, new_len));
        self.emit(abi::add_registers(index, index, old_len));
        self.emit(abi::branch(&first_loop));

        self.emit(abi::label(&first_next));
        self.emit(abi::add_immediate(index, index, 1));
        self.emit(abi::branch(&first_loop));

        self.emit(abi::label(&first_done));
        self.emit(abi::store_u64(
            output_len,
            abi::stack_pointer(),
            output_len_slot,
        ));
        self.emit(abi::add_immediate(abi::return_register(), output_len, 9));
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
        self.emit(abi::load_u64(
            output_len,
            abi::stack_pointer(),
            output_len_slot,
        ));
        self.emit(abi::store_u64(output_len, "x1", 0));
        self.emit(abi::add_immediate(dest, "x1", 8));
        self.emit(abi::load_u64(value_ptr, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(old_ptr, abi::stack_pointer(), old_slot));
        self.emit(abi::load_u64(new_ptr, abi::stack_pointer(), new_slot));
        self.emit(abi::load_u64(value_len, value_ptr, 0));
        self.emit(abi::load_u64(old_len, old_ptr, 0));
        self.emit(abi::load_u64(new_len, new_ptr, 0));
        self.emit(abi::add_immediate(value_ptr, value_ptr, 8));
        self.emit(abi::add_immediate(old_ptr, old_ptr, 8));
        self.emit(abi::add_immediate(new_ptr, new_ptr, 8));
        self.emit(abi::subtract_registers(last_start, value_len, old_len));
        self.emit(abi::move_immediate(index, "Integer", "0"));

        self.emit(abi::label(&second_loop));
        self.emit(abi::compare_registers(index, value_len));
        self.emit(abi::branch_ge(&second_done));
        self.emit(abi::compare_registers(index, last_start));
        self.emit(abi::branch_hi(&second_copy_one));
        self.emit(abi::move_immediate(match_index, "Integer", "0"));
        self.emit(abi::add_registers(candidate, value_ptr, index));
        self.emit(abi::move_register(old_cursor, old_ptr));
        self.emit(abi::label(&second_compare));
        self.emit(abi::compare_registers(match_index, old_len));
        self.emit(abi::branch_eq(&second_match));
        self.emit(abi::load_u8(value_byte, candidate, 0));
        self.emit(abi::load_u8(old_byte, old_cursor, 0));
        self.emit(abi::compare_registers(value_byte, old_byte));
        self.emit(abi::branch_ne(&second_copy_one));
        self.emit(abi::add_immediate(candidate, candidate, 1));
        self.emit(abi::add_immediate(old_cursor, old_cursor, 1));
        self.emit(abi::add_immediate(match_index, match_index, 1));
        self.emit(abi::branch(&second_compare));

        self.emit(abi::label(&second_match));
        self.emit(abi::move_immediate(new_index, "Integer", "0"));
        self.emit(abi::move_register(new_cursor, new_ptr));
        self.emit(abi::label(&second_copy_new_loop));
        self.emit(abi::compare_registers(new_index, new_len));
        self.emit(abi::branch_eq(&second_copy_new_done));
        self.emit(abi::load_u8(value_byte, new_cursor, 0));
        self.emit(abi::store_u8(value_byte, dest, 0));
        self.emit(abi::add_immediate(new_cursor, new_cursor, 1));
        self.emit(abi::add_immediate(dest, dest, 1));
        self.emit(abi::add_immediate(new_index, new_index, 1));
        self.emit(abi::branch(&second_copy_new_loop));
        self.emit(abi::label(&second_copy_new_done));
        self.emit(abi::add_registers(index, index, old_len));
        self.emit(abi::branch(&second_loop));

        self.emit(abi::label(&second_copy_one));
        self.emit(abi::add_registers(candidate, value_ptr, index));
        self.emit(abi::load_u8(value_byte, candidate, 0));
        self.emit(abi::store_u8(value_byte, dest, 0));
        self.emit(abi::add_immediate(dest, dest, 1));
        self.emit(abi::add_immediate(index, index, 1));
        self.emit(abi::branch(&second_loop));

        self.emit(abi::label(&second_done));
        self.emit(abi::move_immediate(value_byte, "Integer", "0"));
        self.emit(abi::store_u8(value_byte, dest, 0));
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        self.emit(abi::branch(&done));

        self.emit(abi::label(&copy_original));
        // No replacement occurred. The caller owns and frees this result, so it
        // must be a fresh arena block, not a borrow of the input `value` (which
        // may be a caller local or a static constant — freeing either would
        // double-free or fault). Deep-copy the input into the arena.
        let original_ptr = self.allocate_register()?;
        self.emit(abi::load_u64(
            &original_ptr,
            abi::stack_pointer(),
            value_slot,
        ));
        let copied = self.copy_flat_block("String", &original_ptr)?;
        self.emit(abi::move_register(&result, &copied));
        self.emit(abi::label(&done));

        Ok(ValueResult {
            type_: "String".to_string(),
            location: result,
            text: "replace(String, String, String)".to_string(),
        })
    }

    pub(super) fn lower_list_replace(
        &mut self,
        value_slot: usize,
        old_slot: usize,
        new_slot: usize,
        list_type: &str,
        element_type: &str,
    ) -> Result<ValueResult, String> {
        let layout = CollectionTypeLayout::from_type(list_type)
            .ok_or_else(|| format!("native code collection type '{list_type}' is not supported"))?;
        for register in ["x20", "x21", "x22", "x23", "x24", "x25"] {
            self.mark_register_used(register);
        }

        let new_payload = PayloadSlot {
            slot: new_slot,
            type_: element_type.to_string(),
        };
        let new_len_slot = self.emit_payload_length_to_stack(&new_payload, "replace_new_len")?;
        let data_len_slot = self.allocate_stack_object("replace_list_data_len", 8);
        let result_slot = self.allocate_stack_object("replace_list_result", 8);
        let loop_label = self.label("replace_list_length_loop");
        let add_new = self.label("replace_list_length_add_new");
        let add_old = self.label("replace_list_length_add_old");
        let length_next = self.label("replace_list_length_next");
        let length_done = self.label("replace_list_length_done");
        let alloc_ok = self.label("replace_list_alloc_ok");
        let copy_loop = self.label("replace_list_copy_loop");
        let copy_new = self.label("replace_list_copy_new");
        let copy_old = self.label("replace_list_copy_old");
        let copy_new_string_loop = self.label("replace_list_copy_new_string_loop");
        let copy_new_string_done = self.label("replace_list_copy_new_string_done");
        let copy_new_inline_loop = self.label("replace_list_copy_new_inline_loop");
        let copy_new_inline_done = self.label("replace_list_copy_new_inline_done");
        let copy_old_loop = self.label("replace_list_copy_old_loop");
        let copy_done_one = self.label("replace_list_copy_done_one");
        let copy_done = self.label("replace_list_copy_done");

        self.emit(abi::load_u64("x8", abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), old_slot));
        self.emit(abi::load_u64("x11", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::move_immediate("x12", "Integer", "0"));
        self.emit(abi::move_immediate("x15", "Integer", "0"));
        self.emit(abi::add_immediate("x16", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_registers("x12", "x11"));
        self.emit(abi::branch_ge(&length_done));
        self.emit(abi::load_u64(
            "x17",
            "x16",
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            "x20",
            "x16",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit_collection_payload_matches_value_branch(
            element_type,
            "x8",
            "x17",
            "x20",
            "x9",
            &add_new,
            &add_old,
        )?;
        self.emit(abi::label(&add_new));
        self.emit(abi::load_u64("x21", abi::stack_pointer(), new_len_slot));
        self.emit(abi::add_registers("x15", "x15", "x21"));
        self.emit(abi::branch(&length_next));
        self.emit(abi::label(&add_old));
        self.emit(abi::add_registers("x15", "x15", "x20"));
        self.emit(abi::label(&length_next));
        self.emit(abi::add_immediate("x16", "x16", COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate("x12", "x12", 1));
        self.emit(abi::branch(&loop_label));

        self.emit(abi::label(&length_done));
        self.emit(abi::store_u64("x15", abi::stack_pointer(), data_len_slot));
        self.emit(abi::move_immediate(
            "x14",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x16", "x11", "x14"));
        self.emit(abi::add_immediate(
            abi::return_register(),
            "x16",
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
        self.emit(abi::load_u64("x8", abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64("x11", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64("x11", "x1", COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64("x11", "x1", COLLECTION_OFFSET_CAPACITY));
        self.emit(abi::load_u64("x15", abi::stack_pointer(), data_len_slot));
        self.emit(abi::store_u64("x15", "x1", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::store_u64("x15", "x1", COLLECTION_OFFSET_DATA_CAPACITY));

        self.emit(abi::load_u64("x8", abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), old_slot));
        self.emit(abi::load_u64("x10", abi::stack_pointer(), new_slot));
        self.emit(abi::load_u64("x11", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::add_immediate("x16", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::add_immediate("x17", "x1", COLLECTION_HEADER_SIZE));
        self.emit_collection_data_pointer("x20", "x8");
        self.emit(abi::move_immediate(
            "x14",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x21", "x11", "x14"));
        self.emit(abi::add_registers("x21", "x17", "x21"));
        self.emit(abi::move_immediate("x12", "Integer", "0"));
        self.emit(abi::move_immediate("x13", "Integer", "0"));

        self.emit(abi::label(&copy_loop));
        self.emit(abi::compare_registers("x12", "x11"));
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
        self.emit(abi::load_u64(
            "x22",
            "x16",
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            "x23",
            "x16",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::store_u64(
            "x13",
            "x17",
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit_collection_payload_matches_value_branch(
            element_type,
            "x8",
            "x22",
            "x23",
            "x9",
            &copy_new,
            &copy_old,
        )?;

        self.emit(abi::label(&copy_new));
        self.emit(abi::load_u64("x23", abi::stack_pointer(), new_len_slot));
        self.emit(abi::store_u64(
            "x23",
            "x17",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers("x25", "x21", "x13"));
        match element_type {
            "Boolean" | "Byte" => {
                self.emit(abi::load_u64("x24", abi::stack_pointer(), new_slot));
                self.emit(abi::store_u8("x24", "x25", 0));
            }
            "Integer" | "Float" | "Fixed" => {
                self.emit(abi::load_u64("x24", abi::stack_pointer(), new_slot));
                self.emit(abi::store_u64("x24", "x25", 0));
            }
            "String" => {
                self.emit(abi::load_u64("x24", abi::stack_pointer(), new_slot));
                self.emit(abi::add_immediate("x24", "x24", 8));
                self.emit(abi::label(&copy_new_string_loop));
                self.emit(abi::compare_immediate("x23", "0"));
                self.emit(abi::branch_eq(&copy_new_string_done));
                self.emit(abi::load_u8("x22", "x24", 0));
                self.emit(abi::store_u8("x22", "x25", 0));
                self.emit(abi::add_immediate("x24", "x24", 1));
                self.emit(abi::add_immediate("x25", "x25", 1));
                self.emit(abi::subtract_immediate("x23", "x23", 1));
                self.emit(abi::branch(&copy_new_string_loop));
                self.emit(abi::label(&copy_new_string_done));
            }
            other if self.inline_collection_payload_size(other).is_some() => {
                self.emit(abi::load_u64("x24", abi::stack_pointer(), new_slot));
                self.emit(abi::label(&copy_new_inline_loop));
                self.emit(abi::compare_immediate("x23", "0"));
                self.emit(abi::branch_eq(&copy_new_inline_done));
                self.emit(abi::load_u8("x22", "x24", 0));
                self.emit(abi::store_u8("x22", "x25", 0));
                self.emit(abi::add_immediate("x24", "x24", 1));
                self.emit(abi::add_immediate("x25", "x25", 1));
                self.emit(abi::subtract_immediate("x23", "x23", 1));
                self.emit(abi::branch(&copy_new_inline_loop));
                self.emit(abi::label(&copy_new_inline_done));
            }
            _ => {
                self.emit(abi::load_u64("x24", abi::stack_pointer(), new_slot));
                self.emit(abi::store_u64("x24", "x25", 0));
            }
        }
        self.emit(abi::branch(&copy_done_one));

        self.emit(abi::label(&copy_old));
        self.emit(abi::store_u64(
            "x23",
            "x17",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers("x24", "x20", "x22"));
        self.emit(abi::add_registers("x25", "x21", "x13"));
        self.emit(abi::label(&copy_old_loop));
        self.emit(abi::compare_immediate("x23", "0"));
        self.emit(abi::branch_eq(&copy_done_one));
        self.emit(abi::load_u8("x22", "x24", 0));
        self.emit(abi::store_u8("x22", "x25", 0));
        self.emit(abi::add_immediate("x24", "x24", 1));
        self.emit(abi::add_immediate("x25", "x25", 1));
        self.emit(abi::subtract_immediate("x23", "x23", 1));
        self.emit(abi::branch(&copy_old_loop));

        self.emit(abi::label(&copy_done_one));
        self.emit(abi::load_u64(
            "x23",
            "x17",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers("x13", "x13", "x23"));
        self.emit(abi::add_immediate("x16", "x16", COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate("x17", "x17", COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate("x12", "x12", 1));
        self.emit(abi::branch(&copy_loop));
        self.emit(abi::label(&copy_done));

        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: list_type.to_string(),
            location: result,
            text: format!("replace({list_type}, {element_type}, {element_type})"),
        })
    }

    pub(super) fn lower_to_string(&mut self, args: &[NirValue]) -> Result<ValueResult, String> {
        let value = self.lower_value(&args[0])?;
        // Observation boundary: rendering a `Float` to text makes it
        // user-accessible, so a non-finite arithmetic result must trap here
        // rather than print as "inf"/"nan" (plan-17). `toString`/`toText` are
        // the only Float→String path (`print` formats through them), and a
        // non-arithmetic Float argument is already finite by the invariant.
        self.observe_float(&args[0], &value)?;
        let value_slot = self.allocate_stack_object("to_string_value", 8);
        self.emit(abi::store_u64(
            &value.location,
            abi::stack_pointer(),
            value_slot,
        ));

        let precision_slot = self.allocate_stack_object("to_string_precision", 8);
        if let Some(precision) = args.get(1) {
            let precision = self.lower_value(precision)?;
            if precision.type_ != "Byte" {
                return Err(format!(
                    "native toString precision must be Byte, got {}",
                    precision.type_
                ));
            }
            self.emit(abi::store_u64(
                &precision.location,
                abi::stack_pointer(),
                precision_slot,
            ));
        } else {
            self.emit(abi::move_immediate("x8", "Byte", "2"));
            self.emit(abi::store_u64("x8", abi::stack_pointer(), precision_slot));
        }

        self.reset_temporary_registers();
        let value_register = self.allocate_register()?;
        self.emit(abi::load_u64(
            &value_register,
            abi::stack_pointer(),
            value_slot,
        ));

        match value.type_.as_str() {
            "String" => Ok(ValueResult {
                type_: "String".to_string(),
                location: value_register,
                text: format!("toString({})", value.text),
            }),
            "Boolean" => self.lower_boolean_to_string(&value_register),
            "Byte" => self.emit_integer_to_string_value(&value_register, false),
            "Integer" => self.emit_integer_to_string_value(&value_register, true),
            "List OF Byte" => self.emit_byte_list_to_string_value(&value_register),
            "Fixed" => {
                let precision = self.allocate_register()?;
                self.emit(abi::load_u64(
                    &precision,
                    abi::stack_pointer(),
                    precision_slot,
                ));
                self.emit_fixed_to_string_value(&value_register, &precision)
            }
            "Float" => {
                let precision = self.allocate_register()?;
                self.emit(abi::load_u64(
                    &precision,
                    abi::stack_pointer(),
                    precision_slot,
                ));
                self.emit_float_to_string_value(&value_register, &precision)
            }
            other => Err(format!(
                "native toString does not accept argument type '{other}'"
            )),
        }
    }

    pub(super) fn lower_boolean_to_string(
        &mut self,
        value_register: &str,
    ) -> Result<ValueResult, String> {
        let false_label = self.label("bool_string_false");
        let done = self.label("bool_string_done");
        let result = self.allocate_register()?;
        self.emit(abi::compare_immediate(value_register, "0"));
        self.emit(abi::branch_eq(&false_label));
        self.emit_load_string_constant(&result, "TRUE")?;
        self.emit(abi::branch(&done));
        self.emit(abi::label(&false_label));
        self.emit_load_string_constant(&result, "FALSE")?;
        self.emit(abi::label(&done));
        Ok(ValueResult {
            type_: "String".to_string(),
            location: result,
            text: "toString(Boolean)".to_string(),
        })
    }

    pub(super) fn emit_integer_to_string_value(
        &mut self,
        source_register: &str,
        signed: bool,
    ) -> Result<ValueResult, String> {
        let buffer_slot = self.allocate_stack_object("to_string_integer_buffer", 40);
        let length_slot = self.allocate_stack_object("to_string_integer_length", 8);
        let start_slot = self.allocate_stack_object("to_string_integer_start", 8);
        let result_slot = self.allocate_stack_object("to_string_integer_result", 8);

        let value = "x8";
        let negative = "x9";
        let length = "x10";
        let cursor = "x11";
        let divisor = "x12";
        let quotient = "x13";
        let digit = "x14";
        let dst = "x15";
        let done = self.label("int_string_done");
        let nonnegative = self.label("int_string_nonnegative");
        let zero = self.label("int_string_zero");
        let loop_start = self.label("int_string_loop");
        let digits_done = self.label("int_string_digits_done");
        let sign_done = self.label("int_string_sign_done");
        let alloc_ok = self.label("int_string_alloc_ok");
        let copy_loop = self.label("int_string_copy_loop");
        let copy_done = self.label("int_string_copy_done");

        self.emit(abi::move_register(value, source_register));
        self.emit(abi::move_immediate(negative, "Integer", "0"));
        self.emit(abi::move_immediate(length, "Integer", "0"));
        self.emit(abi::compare_immediate(value, "0"));
        self.emit(abi::branch_eq(&zero));
        if signed {
            self.emit(abi::branch_ge(&nonnegative));
            self.emit(abi::subtract_registers(value, "xzr", value));
            self.emit(abi::move_immediate(negative, "Integer", "1"));
            self.emit(abi::label(&nonnegative));
        }
        self.emit(abi::add_immediate(
            cursor,
            abi::stack_pointer(),
            buffer_slot + 39,
        ));
        self.emit(abi::move_immediate(divisor, "Integer", "10"));
        self.emit(abi::label(&loop_start));
        self.emit(abi::compare_immediate(value, "0"));
        self.emit(abi::branch_eq(&digits_done));
        self.emit(abi::unsigned_divide_registers(quotient, value, divisor));
        self.emit(abi::multiply_subtract_registers(
            digit, quotient, divisor, value,
        ));
        self.emit(abi::add_immediate(digit, digit, b'0' as usize));
        self.emit(abi::store_u8(digit, cursor, 0));
        self.emit(abi::subtract_immediate(cursor, cursor, 1));
        self.emit(abi::add_immediate(length, length, 1));
        self.emit(abi::move_register(value, quotient));
        self.emit(abi::branch(&loop_start));

        self.emit(abi::label(&zero));
        self.emit(abi::add_immediate(
            cursor,
            abi::stack_pointer(),
            buffer_slot + 39,
        ));
        self.emit(abi::move_immediate(
            digit,
            "Integer",
            &(b'0' as u64).to_string(),
        ));
        self.emit(abi::store_u8(digit, cursor, 0));
        self.emit(abi::subtract_immediate(cursor, cursor, 1));
        self.emit(abi::move_immediate(length, "Integer", "1"));

        self.emit(abi::label(&digits_done));
        self.emit(abi::compare_immediate(negative, "0"));
        self.emit(abi::branch_eq(&sign_done));
        self.emit(abi::move_immediate(
            digit,
            "Integer",
            &(b'-' as u64).to_string(),
        ));
        self.emit(abi::store_u8(digit, cursor, 0));
        self.emit(abi::subtract_immediate(cursor, cursor, 1));
        self.emit(abi::add_immediate(length, length, 1));
        self.emit(abi::label(&sign_done));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::store_u64(length, abi::stack_pointer(), length_slot));
        self.emit(abi::store_u64(cursor, abi::stack_pointer(), start_slot));

        self.emit(abi::add_immediate(abi::return_register(), length, 9));
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
        self.emit(abi::load_u64(length, abi::stack_pointer(), length_slot));
        self.emit(abi::store_u64(length, "x1", 0));
        self.emit(abi::add_immediate(dst, "x1", 8));
        self.emit(abi::load_u64(cursor, abi::stack_pointer(), start_slot));
        self.emit(abi::label(&copy_loop));
        self.emit(abi::compare_immediate(length, "0"));
        self.emit(abi::branch_eq(&copy_done));
        self.emit(abi::load_u8(digit, cursor, 0));
        self.emit(abi::store_u8(digit, dst, 0));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::add_immediate(dst, dst, 1));
        self.emit(abi::subtract_immediate(length, length, 1));
        self.emit(abi::branch(&copy_loop));
        self.emit(abi::label(&copy_done));
        self.emit(abi::move_immediate(digit, "Integer", "0"));
        self.emit(abi::store_u8(digit, dst, 0));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        self.emit(abi::label(&done));
        Ok(ValueResult {
            type_: "String".to_string(),
            location: result,
            text: "toString(Integer)".to_string(),
        })
    }

    pub(super) fn emit_byte_list_to_string_value(
        &mut self,
        source_register: &str,
    ) -> Result<ValueResult, String> {
        let list_slot = self.allocate_stack_object("to_string_byte_list", 8);
        let length_slot = self.allocate_stack_object("to_string_byte_list_length", 8);
        let data_slot = self.allocate_stack_object("to_string_byte_list_data", 8);
        let result_slot = self.allocate_stack_object("to_string_byte_list_result", 8);

        let list = "x8";
        let length = "x9";
        let index = "x10";
        let offset = "x11";
        let byte = "x12";
        let byte2 = "x13";
        let byte3 = "x14";
        let byte4 = "x15";
        let result = "x16";
        let dst = "x17";

        let validate_loop = self.label("byte_list_string_validate_loop");
        let validate_done = self.label("byte_list_string_validate_done");
        let invalid = self.label("byte_list_string_invalid");
        let ascii = self.label("byte_list_string_ascii");
        let two = self.label("byte_list_string_two");
        let three = self.label("byte_list_string_three");
        let three_e0 = self.label("byte_list_string_three_e0");
        let three_ed = self.label("byte_list_string_three_ed");
        let three_mid = self.label("byte_list_string_three_mid");
        let four = self.label("byte_list_string_four");
        let four_f0 = self.label("byte_list_string_four_f0");
        let four_f4 = self.label("byte_list_string_four_f4");
        let four_mid = self.label("byte_list_string_four_mid");
        let alloc_ok = self.label("byte_list_string_alloc_ok");
        let copy_loop = self.label("byte_list_string_copy_loop");
        let copy_done = self.label("byte_list_string_copy_done");

        self.emit(abi::move_register(list, source_register));
        self.emit(abi::store_u64(list, abi::stack_pointer(), list_slot));
        self.emit(abi::load_u64(length, list, COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64(length, abi::stack_pointer(), length_slot));
        self.emit_collection_data_pointer(offset, list);
        self.emit(abi::store_u64(offset, abi::stack_pointer(), data_slot));
        self.emit(abi::move_immediate(index, "Integer", "0"));

        self.emit(abi::label(&validate_loop));
        self.emit(abi::compare_registers(index, length));
        self.emit(abi::branch_ge(&validate_done));
        self.emit(abi::load_u64(offset, abi::stack_pointer(), data_slot));
        self.emit(abi::add_registers(offset, offset, index));
        self.emit(abi::load_u8(byte, offset, 0));

        self.emit(abi::compare_immediate(byte, "128"));
        self.emit(abi::branch_lo(&ascii));

        self.emit(abi::compare_immediate(byte, "194"));
        self.emit(abi::branch_lo(&invalid));
        self.emit(abi::compare_immediate(byte, "224"));
        self.emit(abi::branch_lo(&two));
        self.emit(abi::compare_immediate(byte, "240"));
        self.emit(abi::branch_lo(&three));
        self.emit(abi::compare_immediate(byte, "245"));
        self.emit(abi::branch_lo(&four));
        self.emit(abi::branch(&invalid));

        self.emit(abi::label(&ascii));
        self.emit(abi::add_immediate(index, index, 1));
        self.emit(abi::branch(&validate_loop));

        self.emit(abi::label(&two));
        self.emit(abi::add_immediate(index, index, 1));
        self.emit(abi::compare_registers(index, length));
        self.emit(abi::branch_ge(&invalid));
        self.emit(abi::load_u64(offset, abi::stack_pointer(), data_slot));
        self.emit(abi::add_registers(offset, offset, index));
        self.emit(abi::load_u8(byte2, offset, 0));
        self.emit(abi::compare_immediate(byte2, "128"));
        self.emit(abi::branch_lo(&invalid));
        self.emit(abi::compare_immediate(byte2, "192"));
        self.emit(abi::branch_ge(&invalid));
        self.emit(abi::add_immediate(index, index, 1));
        self.emit(abi::branch(&validate_loop));

        self.emit(abi::label(&three));
        self.emit(abi::add_immediate(index, index, 2));
        self.emit(abi::compare_registers(index, length));
        self.emit(abi::branch_ge(&invalid));
        self.emit(abi::subtract_immediate(index, index, 2));
        self.emit(abi::load_u64(offset, abi::stack_pointer(), data_slot));
        self.emit(abi::add_registers(offset, offset, index));
        self.emit(abi::add_immediate(offset, offset, 1));
        self.emit(abi::load_u8(byte2, offset, 0));
        self.emit(abi::load_u64(offset, abi::stack_pointer(), data_slot));
        self.emit(abi::add_registers(offset, offset, index));
        self.emit(abi::add_immediate(offset, offset, 2));
        self.emit(abi::load_u8(byte3, offset, 0));
        self.emit(abi::compare_immediate(byte3, "128"));
        self.emit(abi::branch_lo(&invalid));
        self.emit(abi::compare_immediate(byte3, "192"));
        self.emit(abi::branch_ge(&invalid));
        self.emit(abi::compare_immediate(byte, "224"));
        self.emit(abi::branch_eq(&three_e0));
        self.emit(abi::compare_immediate(byte, "237"));
        self.emit(abi::branch_eq(&three_ed));
        self.emit(abi::branch(&three_mid));

        self.emit(abi::label(&three_e0));
        self.emit(abi::compare_immediate(byte2, "160"));
        self.emit(abi::branch_lo(&invalid));
        self.emit(abi::compare_immediate(byte2, "192"));
        self.emit(abi::branch_ge(&invalid));
        self.emit(abi::add_immediate(index, index, 3));
        self.emit(abi::branch(&validate_loop));

        self.emit(abi::label(&three_ed));
        self.emit(abi::compare_immediate(byte2, "128"));
        self.emit(abi::branch_lo(&invalid));
        self.emit(abi::compare_immediate(byte2, "160"));
        self.emit(abi::branch_ge(&invalid));
        self.emit(abi::add_immediate(index, index, 3));
        self.emit(abi::branch(&validate_loop));

        self.emit(abi::label(&three_mid));
        self.emit(abi::compare_immediate(byte2, "128"));
        self.emit(abi::branch_lo(&invalid));
        self.emit(abi::compare_immediate(byte2, "192"));
        self.emit(abi::branch_ge(&invalid));
        self.emit(abi::add_immediate(index, index, 3));
        self.emit(abi::branch(&validate_loop));

        self.emit(abi::label(&four));
        self.emit(abi::add_immediate(index, index, 3));
        self.emit(abi::compare_registers(index, length));
        self.emit(abi::branch_ge(&invalid));
        self.emit(abi::subtract_immediate(index, index, 3));
        self.emit(abi::load_u64(offset, abi::stack_pointer(), data_slot));
        self.emit(abi::add_registers(offset, offset, index));
        self.emit(abi::add_immediate(offset, offset, 1));
        self.emit(abi::load_u8(byte2, offset, 0));
        self.emit(abi::load_u64(offset, abi::stack_pointer(), data_slot));
        self.emit(abi::add_registers(offset, offset, index));
        self.emit(abi::add_immediate(offset, offset, 2));
        self.emit(abi::load_u8(byte3, offset, 0));
        self.emit(abi::load_u64(offset, abi::stack_pointer(), data_slot));
        self.emit(abi::add_registers(offset, offset, index));
        self.emit(abi::add_immediate(offset, offset, 3));
        self.emit(abi::load_u8(byte4, offset, 0));
        for continuation in [byte3, byte4] {
            self.emit(abi::compare_immediate(continuation, "128"));
            self.emit(abi::branch_lo(&invalid));
            self.emit(abi::compare_immediate(continuation, "192"));
            self.emit(abi::branch_ge(&invalid));
        }
        self.emit(abi::compare_immediate(byte, "240"));
        self.emit(abi::branch_eq(&four_f0));
        self.emit(abi::compare_immediate(byte, "244"));
        self.emit(abi::branch_eq(&four_f4));
        self.emit(abi::branch(&four_mid));

        self.emit(abi::label(&four_f0));
        self.emit(abi::compare_immediate(byte2, "144"));
        self.emit(abi::branch_lo(&invalid));
        self.emit(abi::compare_immediate(byte2, "192"));
        self.emit(abi::branch_ge(&invalid));
        self.emit(abi::add_immediate(index, index, 4));
        self.emit(abi::branch(&validate_loop));

        self.emit(abi::label(&four_f4));
        self.emit(abi::compare_immediate(byte2, "128"));
        self.emit(abi::branch_lo(&invalid));
        self.emit(abi::compare_immediate(byte2, "144"));
        self.emit(abi::branch_ge(&invalid));
        self.emit(abi::add_immediate(index, index, 4));
        self.emit(abi::branch(&validate_loop));

        self.emit(abi::label(&four_mid));
        self.emit(abi::compare_immediate(byte2, "128"));
        self.emit(abi::branch_lo(&invalid));
        self.emit(abi::compare_immediate(byte2, "192"));
        self.emit(abi::branch_ge(&invalid));
        self.emit(abi::add_immediate(index, index, 4));
        self.emit(abi::branch(&validate_loop));

        self.emit(abi::label(&invalid));
        self.emit_encoding_error_return()?;

        self.emit(abi::label(&validate_done));
        self.emit(abi::load_u64(length, abi::stack_pointer(), length_slot));
        self.emit(abi::add_immediate(abi::return_register(), length, 9));
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
        self.emit(abi::load_u64(length, abi::stack_pointer(), length_slot));
        self.emit(abi::store_u64(length, "x1", 0));
        self.emit(abi::add_immediate(dst, "x1", 8));
        self.emit(abi::move_immediate(index, "Integer", "0"));
        self.emit(abi::load_u64(list, abi::stack_pointer(), list_slot));

        self.emit(abi::label(&copy_loop));
        self.emit(abi::compare_registers(index, length));
        self.emit(abi::branch_ge(&copy_done));
        self.emit(abi::load_u64(offset, abi::stack_pointer(), data_slot));
        self.emit(abi::add_registers(offset, offset, index));
        self.emit(abi::load_u8(byte, offset, 0));
        self.emit(abi::store_u8(byte, dst, 0));
        self.emit(abi::add_immediate(dst, dst, 1));
        self.emit(abi::add_immediate(index, index, 1));
        self.emit(abi::branch(&copy_loop));

        self.emit(abi::label(&copy_done));
        self.emit(abi::move_immediate(byte, "Integer", "0"));
        self.emit(abi::store_u8(byte, dst, 0));
        self.emit(abi::load_u64(result, abi::stack_pointer(), result_slot));

        Ok(ValueResult {
            type_: "String".to_string(),
            location: result.to_string(),
            text: "toString(List OF Byte)".to_string(),
        })
    }

    pub(super) fn emit_fixed_to_string_value(
        &mut self,
        source_register: &str,
        precision_register: &str,
    ) -> Result<ValueResult, String> {
        let buffer_slot = self.allocate_stack_object("to_string_fixed_buffer", 48);
        let integer_start_slot = self.allocate_stack_object("to_string_fixed_integer_start", 8);
        let integer_len_slot = self.allocate_stack_object("to_string_fixed_integer_len", 8);
        let total_len_slot = self.allocate_stack_object("to_string_fixed_total_len", 8);
        let magnitude_slot = self.allocate_stack_object("to_string_fixed_magnitude", 8);
        let precision_slot = self.allocate_stack_object("to_string_fixed_precision", 8);
        let result_slot = self.allocate_stack_object("to_string_fixed_result", 8);

        let raw = "x8";
        let negative = "x9";
        let int_part = "x10";
        let frac_part = "x11";
        let cursor = "x12";
        let length = "x13";
        let divisor = "x14";
        let quotient = "x15";
        let digit = "x16";
        let precision = "x17";
        let total_len = "x20";
        let dst = "x21";
        let counter = "x22";
        let scale = "x23";
        for register in [total_len, dst, counter, scale] {
            if abi::is_callee_saved(register)
                && !self.used_callee_saved.iter().any(|saved| saved == register)
            {
                self.used_callee_saved.push(register.to_string());
            }
        }

        let nonnegative = self.label("fixed_string_nonnegative");
        let integer_zero = self.label("fixed_string_integer_zero");
        let integer_loop = self.label("fixed_string_integer_loop");
        let integer_done = self.label("fixed_string_integer_done");
        let sign_done = self.label("fixed_string_sign_done");
        let no_fraction = self.label("fixed_string_no_fraction");
        let alloc_ok = self.label("fixed_string_alloc_ok");
        let copy_integer_loop = self.label("fixed_string_copy_integer_loop");
        let copy_integer_done = self.label("fixed_string_copy_integer_done");
        let fraction_loop = self.label("fixed_string_fraction_loop");
        let fraction_done = self.label("fixed_string_fraction_done");

        self.emit(abi::move_register(raw, source_register));
        self.emit(abi::move_register(precision, precision_register));
        self.emit(abi::store_u64(
            precision,
            abi::stack_pointer(),
            precision_slot,
        ));
        self.emit(abi::move_immediate(negative, "Integer", "0"));
        self.emit(abi::compare_immediate(raw, "0"));
        self.emit(abi::branch_ge(&nonnegative));
        self.emit(abi::subtract_registers(raw, "xzr", raw));
        self.emit(abi::move_immediate(negative, "Integer", "1"));
        self.emit(abi::label(&nonnegative));
        self.emit(abi::store_u64(raw, abi::stack_pointer(), magnitude_slot));
        self.emit(abi::shift_right_immediate(int_part, raw, 32));
        self.emit(abi::shift_left_immediate(frac_part, raw, 32));
        self.emit(abi::shift_right_immediate(frac_part, frac_part, 32));
        self.emit(abi::move_immediate(length, "Integer", "0"));
        self.emit(abi::add_immediate(
            cursor,
            abi::stack_pointer(),
            buffer_slot + 47,
        ));
        self.emit(abi::compare_immediate(int_part, "0"));
        self.emit(abi::branch_eq(&integer_zero));
        self.emit(abi::move_immediate(divisor, "Integer", "10"));
        self.emit(abi::label(&integer_loop));
        self.emit(abi::compare_immediate(int_part, "0"));
        self.emit(abi::branch_eq(&integer_done));
        self.emit(abi::unsigned_divide_registers(quotient, int_part, divisor));
        self.emit(abi::multiply_subtract_registers(
            digit, quotient, divisor, int_part,
        ));
        self.emit(abi::add_immediate(digit, digit, b'0' as usize));
        self.emit(abi::store_u8(digit, cursor, 0));
        self.emit(abi::subtract_immediate(cursor, cursor, 1));
        self.emit(abi::add_immediate(length, length, 1));
        self.emit(abi::move_register(int_part, quotient));
        self.emit(abi::branch(&integer_loop));

        self.emit(abi::label(&integer_zero));
        self.emit(abi::move_immediate(
            digit,
            "Integer",
            &(b'0' as u64).to_string(),
        ));
        self.emit(abi::store_u8(digit, cursor, 0));
        self.emit(abi::subtract_immediate(cursor, cursor, 1));
        self.emit(abi::move_immediate(length, "Integer", "1"));

        self.emit(abi::label(&integer_done));
        self.emit(abi::compare_immediate(negative, "0"));
        self.emit(abi::branch_eq(&sign_done));
        self.emit(abi::move_immediate(
            digit,
            "Integer",
            &(b'-' as u64).to_string(),
        ));
        self.emit(abi::store_u8(digit, cursor, 0));
        self.emit(abi::subtract_immediate(cursor, cursor, 1));
        self.emit(abi::add_immediate(length, length, 1));
        self.emit(abi::label(&sign_done));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::store_u64(
            cursor,
            abi::stack_pointer(),
            integer_start_slot,
        ));
        self.emit(abi::store_u64(
            length,
            abi::stack_pointer(),
            integer_len_slot,
        ));
        self.emit(abi::move_register(total_len, length));
        self.emit(abi::compare_immediate(precision, "0"));
        self.emit(abi::branch_eq(&no_fraction));
        self.emit(abi::add_immediate(total_len, total_len, 1));
        self.emit(abi::add_registers(total_len, total_len, precision));
        self.emit(abi::label(&no_fraction));
        self.emit(abi::store_u64(
            total_len,
            abi::stack_pointer(),
            total_len_slot,
        ));

        self.emit(abi::add_immediate(abi::return_register(), total_len, 9));
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
        self.emit(abi::load_u64(
            total_len,
            abi::stack_pointer(),
            total_len_slot,
        ));
        self.emit(abi::store_u64(total_len, "x1", 0));
        self.emit(abi::add_immediate(dst, "x1", 8));
        self.emit(abi::load_u64(
            cursor,
            abi::stack_pointer(),
            integer_start_slot,
        ));
        self.emit(abi::load_u64(
            length,
            abi::stack_pointer(),
            integer_len_slot,
        ));
        self.emit(abi::label(&copy_integer_loop));
        self.emit(abi::compare_immediate(length, "0"));
        self.emit(abi::branch_eq(&copy_integer_done));
        self.emit(abi::load_u8(digit, cursor, 0));
        self.emit(abi::store_u8(digit, dst, 0));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::add_immediate(dst, dst, 1));
        self.emit(abi::subtract_immediate(length, length, 1));
        self.emit(abi::branch(&copy_integer_loop));
        self.emit(abi::label(&copy_integer_done));

        self.emit(abi::load_u64(
            precision,
            abi::stack_pointer(),
            precision_slot,
        ));
        self.emit(abi::compare_immediate(precision, "0"));
        self.emit(abi::branch_eq(&fraction_done));
        self.emit(abi::move_immediate(
            digit,
            "Integer",
            &(b'.' as u64).to_string(),
        ));
        self.emit(abi::store_u8(digit, dst, 0));
        self.emit(abi::add_immediate(dst, dst, 1));
        self.emit(abi::load_u64(raw, abi::stack_pointer(), magnitude_slot));
        self.emit(abi::shift_left_immediate(frac_part, raw, 32));
        self.emit(abi::shift_right_immediate(frac_part, frac_part, 32));
        self.emit(abi::move_immediate(counter, "Integer", "0"));
        self.emit(abi::move_immediate(divisor, "Integer", "10"));
        self.emit(abi::move_immediate(scale, "Integer", "4294967296"));
        self.emit(abi::label(&fraction_loop));
        self.emit(abi::compare_registers(counter, precision));
        self.emit(abi::branch_eq(&fraction_done));
        self.emit(abi::multiply_registers(frac_part, frac_part, divisor));
        self.emit(abi::unsigned_divide_registers(digit, frac_part, scale));
        self.emit(abi::multiply_subtract_registers(
            frac_part, digit, scale, frac_part,
        ));
        self.emit(abi::add_immediate(digit, digit, b'0' as usize));
        self.emit(abi::store_u8(digit, dst, 0));
        self.emit(abi::add_immediate(dst, dst, 1));
        self.emit(abi::add_immediate(counter, counter, 1));
        self.emit(abi::branch(&fraction_loop));
        self.emit(abi::label(&fraction_done));
        self.emit(abi::move_immediate(digit, "Integer", "0"));
        self.emit(abi::store_u8(digit, dst, 0));

        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: "String".to_string(),
            location: result,
            text: "toString(Fixed)".to_string(),
        })
    }

    pub(super) fn emit_float_to_string_value(
        &mut self,
        source_register: &str,
        precision_register: &str,
    ) -> Result<ValueResult, String> {
        let buffer_slot =
            self.allocate_stack_object("to_string_float_buffer", FLOAT_TO_STRING_BUFFER_SIZE);
        let length_slot = self.allocate_stack_object("to_string_float_length", 8);
        let result_slot = self.allocate_stack_object("to_string_float_result", 8);

        let snprintf_symbol = if self.platform_imports.contains_key("_snprintf") {
            "_snprintf"
        } else if self.platform_imports.contains_key("snprintf") {
            "snprintf"
        } else {
            return Err("native toString(Float) requires snprintf import".to_string());
        };
        let format_symbol = self
            .string_symbols
            .get(FLOAT_TO_STRING_FORMAT)
            .ok_or_else(|| "native toString(Float) requires float format string".to_string())?
            .clone();

        let format = "x2";
        let precision = "x3";
        let length = "x20";
        let src = "x22";
        let dst = "x23";
        let byte = "x24";
        for register in [length, src, dst, byte] {
            if abi::is_callee_saved(register)
                && !self.used_callee_saved.iter().any(|saved| saved == register)
            {
                self.used_callee_saved.push(register.to_string());
            }
        }

        let snprintf_ok = self.label("float_string_snprintf_ok");
        let snprintf_invalid = self.label("float_string_snprintf_invalid");
        let alloc_ok = self.label("float_string_alloc_ok");
        let copy_loop = self.label("float_string_copy_loop");
        let copy_done = self.label("float_string_copy_done");

        self.emit(abi::add_immediate("x0", abi::stack_pointer(), buffer_slot));
        self.emit(abi::move_immediate(
            "x1",
            "Integer",
            &FLOAT_TO_STRING_BUFFER_SIZE.to_string(),
        ));
        self.emit(abi::load_page_address(format, &format_symbol));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: format_symbol.clone(),
            kind: "page21".to_string(),
            binding: "data".to_string(),
            library: None,
        });
        self.emit(abi::add_page_offset(format, format, &format_symbol));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: format_symbol,
            kind: "pageoff12".to_string(),
            binding: "data".to_string(),
            library: None,
        });
        self.emit(abi::add_immediate(format, format, 8));
        self.emit(abi::move_register(precision, precision_register));
        self.emit(abi::float_move_d_from_x("d0", source_register));
        if snprintf_symbol == "_snprintf" {
            self.emit(abi::subtract_stack(16));
            self.emit(abi::store_u64(precision, abi::raw_stack_pointer(), 0));
            self.emit(abi::float_move_x_from_d("x4", "d0"));
            self.emit(abi::store_u64("x4", abi::raw_stack_pointer(), 8));
        }
        self.emit_symbol_call(snprintf_symbol);
        if snprintf_symbol == "_snprintf" {
            self.emit(abi::add_stack(16));
        }
        self.emit(abi::move_register(length, abi::return_register()));
        self.emit(abi::compare_immediate(length, "0"));
        self.emit(abi::branch_lt(&snprintf_invalid));
        self.emit(abi::compare_immediate(
            length,
            &FLOAT_TO_STRING_BUFFER_SIZE.to_string(),
        ));
        self.emit(abi::branch_lt(&snprintf_ok));
        self.emit(abi::label(&snprintf_invalid));
        self.emit_invalid_argument_return()?;
        self.emit(abi::label(&snprintf_ok));
        self.emit(abi::store_u64(length, abi::stack_pointer(), length_slot));

        self.emit(abi::add_immediate(abi::return_register(), length, 9));
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
        self.emit(abi::load_u64(length, abi::stack_pointer(), length_slot));
        self.emit(abi::store_u64(length, "x1", 0));
        self.emit(abi::add_immediate(dst, "x1", 8));
        self.emit(abi::add_immediate(src, abi::stack_pointer(), buffer_slot));
        self.emit(abi::label(&copy_loop));
        self.emit(abi::compare_immediate(length, "0"));
        self.emit(abi::branch_eq(&copy_done));
        self.emit(abi::load_u8(byte, src, 0));
        self.emit(abi::store_u8(byte, dst, 0));
        self.emit(abi::add_immediate(src, src, 1));
        self.emit(abi::add_immediate(dst, dst, 1));
        self.emit(abi::subtract_immediate(length, length, 1));
        self.emit(abi::branch(&copy_loop));
        self.emit(abi::label(&copy_done));
        self.emit(abi::move_immediate(byte, "Integer", "0"));
        self.emit(abi::store_u8(byte, dst, 0));

        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: "String".to_string(),
            location: result,
            text: "toString(Float)".to_string(),
        })
    }
}
