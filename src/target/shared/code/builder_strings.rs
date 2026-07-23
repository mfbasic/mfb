use super::*;

impl CodeBuilder<'_> {
    pub(super) fn lower_replace(&mut self, args: &[NirValue]) -> Result<ValueResult, String> {
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
        let scratch21 = self.temporary_vreg();
        let scratch22 = self.temporary_vreg();
        let scratch23 = self.temporary_vreg();
        let scratch24 = self.temporary_vreg();
        let scratch25 = self.temporary_vreg();
        let scratch26 = self.temporary_vreg();
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

        let value_ptr = scratch8.as_str();
        let value_len = scratch9.as_str();
        let old_ptr = scratch10.as_str();
        let old_len = scratch11.as_str();
        let new_ptr = scratch12.as_str();
        let new_len = scratch13.as_str();
        let index = scratch14.as_str();
        let output_len = scratch15.as_str();
        let last_start = scratch16.as_str();
        let match_index = scratch17.as_str();
        let candidate = scratch20.as_str();
        let old_cursor = scratch21.as_str();
        let value_byte = scratch22.as_str();
        let old_byte = scratch23.as_str();
        let dest = scratch24.as_str();
        let new_cursor = scratch25.as_str();
        let new_index = scratch26.as_str();

        let copy_original = self.label("replace_copy_original");
        let first_loop = self.label("replace_first_loop");
        let first_compare = self.label("replace_first_compare");
        let first_match = self.label("replace_first_match");
        let first_next = self.label("replace_first_next");
        let first_done = self.label("replace_first_done");
        let alloc_ok = self.label("replace_alloc_ok");
        let overflow = self.label("replace_overflow");
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
        // output_len += new_len - old_len. old_len <= value_len at a match, so the
        // subtract never underflows; the add is the growth term — trap a 64-bit
        // wrap so the second pass cannot write past the (undersized) allocation.
        self.emit(abi::subtract_registers(output_len, output_len, old_len));
        self.emit_checked_size_add(output_len, output_len, new_len, &overflow);
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
        // allocate output_len + 9 (block header), trapping the header add's wrap.
        self.emit_checked_size_add_immediate(abi::return_register(), output_len, 9, &overflow);
        self.emit(abi::move_immediate(abi::ARG[1], "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        // A size wrap reports the same 77010001 an impossible allocation would
        // (x0 does not hold an error code before the call, so the register-based
        // return above cannot be shared). The checked-size helper deposits the
        // partially-computed size into the return register before branching here,
        // so `emit_allocation_error_return` would surface that size as the error
        // code (bug-60 detection, bug-352 code fix).
        self.emit(abi::label(&overflow));
        self.emit_error_code_return(ERR_OUT_OF_MEMORY_CODE, ERR_ALLOCATION_MESSAGE)?;

        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(
            abi::RET[1],
            abi::stack_pointer(),
            result_slot,
        ));
        self.emit(abi::load_u64(
            output_len,
            abi::stack_pointer(),
            output_len_slot,
        ));
        self.emit(abi::store_u64(output_len, abi::RET[1], 0));
        self.emit(abi::add_immediate(dest, abi::RET[1], 8));
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
        // must be a fresh arena block, not an alias of the input `value` (which
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
        // Entry stride for this element type: zero builds the result entry-free
        // and makes every cursor below stride the data region (plan-57-D).
        let rep_stride = list_entry_stride(element_type);
        let rep_payload = kind2_payload_size(element_type);
        let layout = CollectionTypeLayout::from_type(list_type)
            .ok_or_else(|| format!("native code collection type '{list_type}' is not supported"))?;
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
        let scratch21 = self.temporary_vreg();
        let scratch22 = self.temporary_vreg();
        let scratch23 = self.temporary_vreg();
        let scratch24 = self.temporary_vreg();
        let scratch25 = self.temporary_vreg();

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
        let overflow = self.label("replace_list_overflow");
        let copy_loop = self.label("replace_list_copy_loop");
        let copy_new = self.label("replace_list_copy_new");
        let copy_old = self.label("replace_list_copy_old");
        let copy_done_one = self.label("replace_list_copy_done_one");
        let copy_done = self.label("replace_list_copy_done");

        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), old_slot));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch8,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::move_immediate(&scratch12, "Integer", "0"));
        self.emit(abi::move_immediate(&scratch15, "Integer", "0"));
        if rep_payload.is_some() {
            // kind 2: a byte OFFSET into the data region, not an entry pointer.
            self.emit(abi::move_immediate(&scratch16, "Integer", "0"));
        } else {
            self.emit(abi::add_immediate(
                &scratch16,
                &scratch8,
                COLLECTION_HEADER_SIZE,
            ));
        }
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_registers(&scratch12, &scratch11));
        self.emit(abi::branch_ge(&length_done));
        if let Some(payload) = rep_payload {
            self.emit(abi::move_register(&scratch17, &scratch16));
            self.emit(abi::move_immediate(
                &scratch20,
                "Integer",
                &payload.to_string(),
            ));
        } else {
            self.emit(abi::load_u64(
                &scratch17,
                &scratch16,
                COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
            ));
            self.emit(abi::load_u64(
                &scratch20,
                &scratch16,
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ));
        }
        self.emit_collection_payload_matches_value_branch(
            element_type,
            element_type,
            &scratch8,
            &scratch17,
            &scratch20,
            &scratch9,
            &add_new,
            &add_old,
        )?;
        self.emit(abi::label(&add_new));
        self.emit(abi::load_u64(
            &scratch21,
            abi::stack_pointer(),
            new_len_slot,
        ));
        self.emit_checked_size_add(&scratch15, &scratch15, &scratch21, &overflow);
        self.emit(abi::branch(&length_next));
        self.emit(abi::label(&add_old));
        self.emit_checked_size_add(&scratch15, &scratch15, &scratch20, &overflow);
        self.emit(abi::label(&length_next));
        self.emit(abi::add_immediate(
            &scratch16,
            &scratch16,
            rep_payload.unwrap_or(COLLECTION_ENTRY_SIZE),
        ));
        self.emit(abi::add_immediate(&scratch12, &scratch12, 1));
        self.emit(abi::branch(&loop_label));

        self.emit(abi::label(&length_done));
        self.emit(abi::store_u64(
            &scratch15,
            abi::stack_pointer(),
            data_len_slot,
        ));
        self.emit(abi::move_immediate(
            &scratch14,
            "Integer",
            &rep_stride.to_string(),
        ));
        // size = count*ENTRY_SIZE + HEADER + data_len, trapping any 64-bit wrap so
        // the copy pass cannot overrun the (undersized) allocation (bug-60).
        self.emit_checked_size_multiply(&scratch16, &scratch11, &scratch14, &overflow);
        self.emit_checked_size_add_immediate(
            abi::return_register(),
            &scratch16,
            COLLECTION_HEADER_SIZE,
            &overflow,
        );
        self.emit_checked_size_add(
            abi::return_register(),
            abi::return_register(),
            &scratch15,
            &overflow,
        );
        self.emit(abi::move_immediate(abi::ARG[1], "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        // A size wrap reports the same 77010001 an impossible allocation would
        // (x0 does not hold an error code before the call, so the register-based
        // return above cannot be shared). The checked-size helper deposits the
        // partially-computed size into the return register before branching here,
        // so `emit_allocation_error_return` would surface that size as the error
        // code (bug-60 detection, bug-352 code fix).
        self.emit(abi::label(&overflow));
        self.emit_error_code_return(ERR_OUT_OF_MEMORY_CODE, ERR_ALLOCATION_MESSAGE)?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(
            abi::RET[1],
            abi::stack_pointer(),
            result_slot,
        ));

        self.emit(abi::move_immediate(
            &scratch13,
            "Byte",
            &layout.kind.to_string(),
        ));
        self.emit(abi::store_u8(
            &scratch13,
            abi::RET[1],
            COLLECTION_OFFSET_KIND,
        ));
        self.emit(abi::move_immediate(
            &scratch13,
            "Byte",
            &layout.key_type_code.to_string(),
        ));
        self.emit(abi::store_u8(
            &scratch13,
            abi::RET[1],
            COLLECTION_OFFSET_KEY_TYPE,
        ));
        self.emit(abi::move_immediate(
            &scratch13,
            "Byte",
            &layout.value_type_code.to_string(),
        ));
        self.emit(abi::store_u8(
            &scratch13,
            abi::RET[1],
            COLLECTION_OFFSET_VALUE_TYPE,
        ));
        self.emit(abi::move_immediate(&scratch13, "Byte", "1"));
        self.emit(abi::store_u8(
            &scratch13,
            abi::RET[1],
            COLLECTION_OFFSET_FLAGS_VERSION,
        ));
        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch8,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::store_u64(
            &scratch11,
            abi::RET[1],
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::store_u64(
            &scratch11,
            abi::RET[1],
            COLLECTION_OFFSET_CAPACITY,
        ));
        self.emit(abi::load_u64(
            &scratch15,
            abi::stack_pointer(),
            data_len_slot,
        ));
        self.emit(abi::store_u64(
            &scratch15,
            abi::RET[1],
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::store_u64(
            &scratch15,
            abi::RET[1],
            COLLECTION_OFFSET_DATA_CAPACITY,
        ));

        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), old_slot));
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), new_slot));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch8,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::load_u64(
            abi::RET[1],
            abi::stack_pointer(),
            result_slot,
        ));
        if rep_payload.is_some() {
            // kind 2: the copy pass re-seeds the SOURCE cursor as a byte offset,
            // exactly as the length pass did.
            self.emit(abi::move_immediate(&scratch16, "Integer", "0"));
        } else {
            self.emit(abi::add_immediate(
                &scratch16,
                &scratch8,
                COLLECTION_HEADER_SIZE,
            ));
        }
        self.emit(abi::add_immediate(
            &scratch17,
            abi::RET[1],
            COLLECTION_HEADER_SIZE,
        ));
        self.emit_collection_data_pointer_for(&scratch20, &scratch8, element_type);
        self.emit(abi::move_immediate(
            &scratch14,
            "Integer",
            &rep_stride.to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch21, &scratch11, &scratch14));
        self.emit(abi::add_registers(&scratch21, &scratch17, &scratch21));
        self.emit(abi::move_immediate(&scratch12, "Integer", "0"));
        self.emit(abi::move_immediate(&scratch13, "Integer", "0"));

        self.emit(abi::label(&copy_loop));
        self.emit(abi::compare_registers(&scratch12, &scratch11));
        self.emit(abi::branch_ge(&copy_done));
        self.emit(abi::move_immediate(
            &scratch22,
            "Byte",
            &COLLECTION_ENTRY_FLAG_USED.to_string(),
        ));
        if rep_stride != 0 {
            self.emit(abi::store_u8(
                &scratch22,
                &scratch17,
                COLLECTION_ENTRY_OFFSET_FLAGS,
            ));
        }
        self.emit(abi::move_immediate(&scratch22, "Integer", "0"));
        if rep_stride != 0 {
            self.emit(abi::store_u64(
                &scratch22,
                &scratch17,
                COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
            ));
        }
        if rep_stride != 0 {
            self.emit(abi::store_u64(
                &scratch22,
                &scratch17,
                COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
            ));
        }
        if let Some(payload) = rep_payload {
            // kind 2: scratch16 is already the source byte offset.
            self.emit(abi::move_register(&scratch22, &scratch16));
            self.emit(abi::move_immediate(
                &scratch23,
                "Integer",
                &payload.to_string(),
            ));
        } else {
            self.emit(abi::load_u64(
                &scratch22,
                &scratch16,
                COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
            ));
            self.emit(abi::load_u64(
                &scratch23,
                &scratch16,
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ));
        }
        if rep_stride != 0 {
            self.emit(abi::store_u64(
                &scratch13,
                &scratch17,
                COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
            ));
        }
        self.emit_collection_payload_matches_value_branch(
            element_type,
            element_type,
            &scratch8,
            &scratch22,
            &scratch23,
            &scratch9,
            &copy_new,
            &copy_old,
        )?;

        self.emit(abi::label(&copy_new));
        self.emit(abi::load_u64(
            &scratch23,
            abi::stack_pointer(),
            new_len_slot,
        ));
        if rep_stride != 0 {
            self.emit(abi::store_u64(
                &scratch23,
                &scratch17,
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ));
        }
        self.emit(abi::add_registers(&scratch25, &scratch21, &scratch13));
        match element_type {
            "Boolean" | "Byte" => {
                self.emit(abi::load_u64(&scratch24, abi::stack_pointer(), new_slot));
                self.emit(abi::store_u8(&scratch24, &scratch25, 0));
            }
            "Integer" | "Float" | "Fixed" => {
                self.emit(abi::load_u64(&scratch24, abi::stack_pointer(), new_slot));
                self.emit(abi::store_u64(&scratch24, &scratch25, 0));
            }
            "String" => {
                self.emit(abi::load_u64(&scratch24, abi::stack_pointer(), new_slot));
                self.emit(abi::add_immediate(&scratch24, &scratch24, 8));
                self.emit_block_copy_advance(
                    &scratch25,
                    &scratch24,
                    &scratch23,
                    &scratch22,
                    "replace_list_copy_new_string",
                );
            }
            other if self.inline_collection_payload_size(other).is_some() => {
                self.emit(abi::load_u64(&scratch24, abi::stack_pointer(), new_slot));
                self.emit_block_copy_advance(
                    &scratch25,
                    &scratch24,
                    &scratch23,
                    &scratch22,
                    "replace_list_copy_new_inline",
                );
            }
            _ => {
                self.emit(abi::load_u64(&scratch24, abi::stack_pointer(), new_slot));
                self.emit(abi::store_u64(&scratch24, &scratch25, 0));
            }
        }
        self.emit(abi::branch(&copy_done_one));

        self.emit(abi::label(&copy_old));
        if rep_stride != 0 {
            self.emit(abi::store_u64(
                &scratch23,
                &scratch17,
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ));
        }
        self.emit(abi::add_registers(&scratch24, &scratch20, &scratch22));
        self.emit(abi::add_registers(&scratch25, &scratch21, &scratch13));
        self.emit_block_copy_advance(
            &scratch25,
            &scratch24,
            &scratch23,
            &scratch22,
            "replace_list_copy_old",
        );

        self.emit(abi::label(&copy_done_one));
        if let Some(payload) = rep_payload {
            // No destination entry to read the written length back from; for a
            // fixed-width element it is the constant payload size.
            self.emit(abi::move_immediate(
                &scratch23,
                "Integer",
                &payload.to_string(),
            ));
        } else {
            self.emit(abi::load_u64(
                &scratch23,
                &scratch17,
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ));
        }
        self.emit(abi::add_registers(&scratch13, &scratch13, &scratch23));
        self.emit(abi::add_immediate(
            &scratch16,
            &scratch16,
            rep_payload.unwrap_or(COLLECTION_ENTRY_SIZE),
        ));
        self.emit(abi::add_immediate(&scratch17, &scratch17, rep_stride));
        self.emit(abi::add_immediate(&scratch12, &scratch12, 1));
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
        let scratch8 = self.temporary_vreg();
        let value = self.lower_value(&args[0])?;
        // Observation boundary: rendering a `Float` to text makes it
        // user-accessible, so a non-finite arithmetic result must trap here
        // rather than print as "inf"/"nan" (plan-17). `toString`/`toText` are
        // the only Float→String path (`print` formats through them), and a
        // non-arithmetic Float argument is already finite by the invariant.
        self.observe_float(&args[0], &value)?;
        // The value is spilled through an integer slot and reloaded into a GPR
        // for formatting, so a `d`-native float is materialized into a GPR first
        // (plan-01 float-dnative). Identity for every GP-native value.
        let value = self.materialize_float(value)?;
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
            self.emit(abi::move_immediate(&scratch8, "Byte", "2"));
            self.emit(abi::store_u64(
                &scratch8,
                abi::stack_pointer(),
                precision_slot,
            ));
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
            "Scalar" => self.emit_scalar_to_string_value(&value_register),
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
            "Money" => {
                let precision = self.allocate_register()?;
                self.emit(abi::load_u64(
                    &precision,
                    abi::stack_pointer(),
                    precision_slot,
                ));
                self.emit_money_to_string_value(&value_register, &precision)
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

        // Virtual registers (not physical `x8`–`x15`): the allocator places them
        // in safe, distinct per-ISA registers. On x86 the physical names collided
        // (`x8`/`x9` both → `rax`) and the `div`/`msub` pair needs the dividend to
        // survive the `div` (which clobbers `rax`/`rdx`) — vregs, never colored to
        // `rax`/`rdx`, satisfy both.
        let value_s = self.allocate_register()?;
        let negative_s = self.allocate_register()?;
        let length_s = self.allocate_register()?;
        let cursor_s = self.allocate_register()?;
        let divisor_s = self.allocate_register()?;
        let quotient_s = self.allocate_register()?;
        let digit_s = self.allocate_register()?;
        let dst_s = self.allocate_register()?;
        let value = value_s.as_str();
        let negative = negative_s.as_str();
        let length = length_s.as_str();
        let cursor = cursor_s.as_str();
        let divisor = divisor_s.as_str();
        let quotient = quotient_s.as_str();
        let digit = digit_s.as_str();
        let dst = dst_s.as_str();
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
            self.emit(abi::subtract_registers(value, abi::ZERO, value));
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
        self.emit(abi::load_u64(length, abi::stack_pointer(), length_slot));
        self.emit(abi::store_u64(length, abi::RET[1], 0));
        self.emit(abi::add_immediate(dst, abi::RET[1], 8));
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
        let list_slot = self.allocate_stack_object("to_string_byte_list", 8);
        let length_slot = self.allocate_stack_object("to_string_byte_list_length", 8);
        let data_slot = self.allocate_stack_object("to_string_byte_list_data", 8);
        let result_slot = self.allocate_stack_object("to_string_byte_list_result", 8);

        let list = scratch8.as_str();
        let length = scratch9.as_str();
        let index = scratch10.as_str();
        let offset = scratch11.as_str();
        let byte = scratch12.as_str();
        let byte2 = scratch13.as_str();
        let byte3 = scratch14.as_str();
        let byte4 = scratch15.as_str();
        let result = scratch16.as_str();
        let dst = scratch17.as_str();

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
        self.emit_collection_data_pointer_for(offset, list, "Byte");
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
        self.emit(abi::load_u64(length, abi::stack_pointer(), length_slot));
        self.emit(abi::store_u64(length, abi::RET[1], 0));
        self.emit(abi::add_immediate(dst, abi::RET[1], 8));
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
        let scratch21 = self.temporary_vreg();
        let scratch22 = self.temporary_vreg();
        let scratch23 = self.temporary_vreg();
        let buffer_slot = self.allocate_stack_object("to_string_fixed_buffer", 48);
        let integer_start_slot = self.allocate_stack_object("to_string_fixed_integer_start", 8);
        let integer_len_slot = self.allocate_stack_object("to_string_fixed_integer_len", 8);
        let total_len_slot = self.allocate_stack_object("to_string_fixed_total_len", 8);
        let magnitude_slot = self.allocate_stack_object("to_string_fixed_magnitude", 8);
        let precision_slot = self.allocate_stack_object("to_string_fixed_precision", 8);
        let result_slot = self.allocate_stack_object("to_string_fixed_result", 8);

        let raw = scratch8.as_str();
        let negative = scratch9.as_str();
        let int_part = scratch10.as_str();
        let frac_part = scratch11.as_str();
        let cursor = scratch12.as_str();
        let length = scratch13.as_str();
        let divisor = scratch14.as_str();
        let quotient = scratch15.as_str();
        let digit = scratch16.as_str();
        let precision = scratch17.as_str();
        let total_len = scratch20.as_str();
        let dst = scratch21.as_str();
        let counter = scratch22.as_str();
        let scale = scratch23.as_str();

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
        // bug-312 K1 presentation pre-round.
        let round_skip = self.label("fixed_string_round_skip");
        let round_pow_loop = self.label("fixed_string_round_pow_loop");
        let round_pow_done = self.label("fixed_string_round_pow_done");
        let exponent_s = self.temporary_vreg();
        let exponent = exponent_s.as_str();

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
        self.emit(abi::subtract_registers(raw, abi::ZERO, raw));
        self.emit(abi::move_immediate(negative, "Integer", "1"));
        self.emit(abi::label(&nonnegative));
        // bug-312 K1: presentation pre-round, half-away-from-zero, mirroring the
        // Money renderer below. The fraction loop truncates -- it emits
        // `(frac*10)>>32` per digit and stops -- so `toString(toFixed("0.666"), 2b)`
        // gave "0.66" where the Float overload gives "0.67", and
        // `toString(toFixed("0.99"), 1b)` gave "0.9" instead of "1.0". The man page
        // documents all three fixed-precision overloads identically, so they must
        // not disagree.
        //
        // Rounding the RAW magnitude before it is split into int/frac is what makes
        // a carry work: `0.99` at one place carries into the integer part, and the
        // split below then sees the carried value. Adjusting digits after they were
        // emitted could not do that. `raw` is already the magnitude here (the sign
        // was stripped above), so half-away is a plain half-up add.
        //
        // half ULP at `precision` places = 2^32 * 0.5 * 10^-p = 2^31 / 10^p.
        // For p >= 10 the Q32.32 fraction (~9.6 decimal digits) is exhausted and the
        // remaining places render as trailing zeros, so no rounding is needed --
        // which also keeps 10^p inside 64 bits.
        self.emit(abi::compare_immediate(precision, "10"));
        self.emit(abi::branch_ge(&round_skip));
        self.emit(abi::move_register(exponent, precision));
        self.emit(abi::move_immediate(divisor, "Integer", "1"));
        self.emit(abi::move_immediate(scale, "Integer", "10"));
        self.emit(abi::label(&round_pow_loop));
        self.emit(abi::compare_immediate(exponent, "0"));
        self.emit(abi::branch_eq(&round_pow_done));
        self.emit(abi::multiply_registers(divisor, divisor, scale));
        self.emit(abi::subtract_immediate(exponent, exponent, 1));
        self.emit(abi::branch(&round_pow_loop));
        self.emit(abi::label(&round_pow_done));
        // half = ceil(2^31 / 10^p).
        //
        // The ceiling matters. A truncating divide makes the bias strictly LESS
        // than half a ULP, so a value sitting exactly on the boundary rounds DOWN:
        // 2^31/100 is 21474836.48, and with 21474836 the exactly-representable
        // `0.125` rendered at two places gave "0.12" instead of "0.13". Rounding
        // the bias up puts the boundary case on the away-from-zero side, which is
        // the documented intent.
        self.emit(abi::move_immediate(scale, "Integer", "2147483648"));
        self.emit(abi::add_registers(scale, scale, divisor));
        self.emit(abi::subtract_immediate(scale, scale, 1));
        self.emit(abi::unsigned_divide_registers(scale, scale, divisor));
        // Skip the bump when it would overflow i64 — only reachable at the very top
        // of the Fixed range, where truncating is the safe answer.
        self.emit(abi::move_immediate(
            quotient,
            "Integer",
            "9223372036854775807",
        ));
        self.emit(abi::subtract_registers(quotient, quotient, scale));
        self.emit(abi::compare_registers(raw, quotient));
        self.emit(abi::branch_gt(&round_skip));
        self.emit(abi::add_registers(raw, raw, scale));
        self.emit(abi::label(&round_skip));
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
        self.emit(abi::load_u64(
            total_len,
            abi::stack_pointer(),
            total_len_slot,
        ));
        self.emit(abi::store_u64(total_len, abi::RET[1], 0));
        self.emit(abi::add_immediate(dst, abi::RET[1], 8));
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

    /// `toString(Money [, precision])` (plan-29-G §4.1). Base-10 decimal
    /// formatting of the scaled raw i64: `intpart = raw / 100000`, five fractional
    /// digits from `|raw| % 100000`, rendered to `precision` places (default 2).
    /// Presentation rounding is a **fixed** half-away-from-zero rule when
    /// `precision < 5`, independent of the global rounding mode — so `toString` is
    /// a pure function of `(raw, precision)`. Structurally mirrors
    /// `emit_fixed_to_string_value` with the scale changed from `2^32` to `100000`
    /// plus the half-away pre-round.
    pub(super) fn emit_money_to_string_value(
        &mut self,
        source_register: &str,
        precision_register: &str,
    ) -> Result<ValueResult, String> {
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
        let scratch21 = self.temporary_vreg();
        let scratch22 = self.temporary_vreg();
        let scratch23 = self.temporary_vreg();
        let scratch24 = self.temporary_vreg();
        let scratch25 = self.temporary_vreg();
        let buffer_slot = self.allocate_stack_object("to_string_money_buffer", 48);
        let integer_start_slot = self.allocate_stack_object("to_string_money_integer_start", 8);
        let integer_len_slot = self.allocate_stack_object("to_string_money_integer_len", 8);
        let total_len_slot = self.allocate_stack_object("to_string_money_total_len", 8);
        let magnitude_slot = self.allocate_stack_object("to_string_money_magnitude", 8);
        let precision_slot = self.allocate_stack_object("to_string_money_precision", 8);
        let result_slot = self.allocate_stack_object("to_string_money_result", 8);

        let raw = scratch8.as_str();
        let negative = scratch9.as_str();
        let int_part = scratch10.as_str();
        let frac_part = scratch11.as_str();
        let cursor = scratch12.as_str();
        let length = scratch13.as_str();
        let divisor = scratch14.as_str();
        let quotient = scratch15.as_str();
        let digit = scratch16.as_str();
        let precision = scratch17.as_str();
        let total_len = scratch20.as_str();
        let dst = scratch21.as_str();
        let counter = scratch22.as_str();
        let scale = scratch23.as_str();
        let exponent = scratch24.as_str();
        let remainder = scratch25.as_str();

        let nonnegative = self.label("money_string_nonnegative");
        let round_skip = self.label("money_string_round_skip");
        let round_pow_loop = self.label("money_string_round_pow_loop");
        let round_pow_done = self.label("money_string_round_pow_done");
        let round_no_bump = self.label("money_string_round_no_bump");
        let integer_zero = self.label("money_string_integer_zero");
        let integer_loop = self.label("money_string_integer_loop");
        let integer_done = self.label("money_string_integer_done");
        let sign_done = self.label("money_string_sign_done");
        let no_fraction = self.label("money_string_no_fraction");
        let alloc_ok = self.label("money_string_alloc_ok");
        let copy_integer_loop = self.label("money_string_copy_integer_loop");
        let copy_integer_done = self.label("money_string_copy_integer_done");
        let fraction_loop = self.label("money_string_fraction_loop");
        let fraction_done = self.label("money_string_fraction_done");

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
        self.emit(abi::subtract_registers(raw, abi::ZERO, raw));
        self.emit(abi::move_immediate(negative, "Integer", "1"));
        self.emit(abi::label(&nonnegative));

        // Presentation pre-round (half-away-from-zero) when precision < 5: replace
        // `raw` (the magnitude) with the value rounded to `precision` places but
        // still expressed at the 5-place scale, so the truncating render below
        // emits exactly the rounded digits. `precision >= 5` needs no rounding
        // (the extra places render as trailing zeros).
        self.emit(abi::compare_immediate(precision, "5"));
        self.emit(abi::branch_ge(&round_skip));
        // divisor = 10^(5 - precision), built by a bounded (<5) multiply loop.
        self.emit(abi::move_immediate(exponent, "Integer", "5"));
        self.emit(abi::subtract_registers(exponent, exponent, precision));
        self.emit(abi::move_immediate(divisor, "Integer", "1"));
        self.emit(abi::move_immediate(scale, "Integer", "10"));
        self.emit(abi::label(&round_pow_loop));
        self.emit(abi::compare_immediate(exponent, "0"));
        self.emit(abi::branch_eq(&round_pow_done));
        self.emit(abi::multiply_registers(divisor, divisor, scale));
        self.emit(abi::subtract_immediate(exponent, exponent, 1));
        self.emit(abi::branch(&round_pow_loop));
        self.emit(abi::label(&round_pow_done));
        // q = raw / divisor, r = raw - q*divisor; bump q when 2*r >= divisor.
        self.emit(abi::unsigned_divide_registers(quotient, raw, divisor));
        self.emit(abi::multiply_subtract_registers(
            remainder, quotient, divisor, raw,
        ));
        self.emit(abi::add_registers(remainder, remainder, remainder)); // 2*r (no overflow: r<divisor<=1e5)
        self.emit(abi::compare_registers(remainder, divisor));
        self.emit(abi::branch_lt(&round_no_bump));
        self.emit(abi::add_immediate(quotient, quotient, 1));
        self.emit(abi::label(&round_no_bump));
        self.emit(abi::multiply_registers(raw, quotient, divisor));
        self.emit(abi::label(&round_skip));

        self.emit(abi::store_u64(raw, abi::stack_pointer(), magnitude_slot));
        // int_part = raw / 100000; frac_part = raw % 100000.
        self.emit(abi::move_immediate(scale, "Integer", "100000"));
        self.emit(abi::unsigned_divide_registers(int_part, raw, scale));
        self.emit(abi::multiply_subtract_registers(
            frac_part, int_part, scale, raw,
        ));
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
        self.emit(abi::load_u64(
            total_len,
            abi::stack_pointer(),
            total_len_slot,
        ));
        self.emit(abi::store_u64(total_len, abi::RET[1], 0));
        self.emit(abi::add_immediate(dst, abi::RET[1], 8));
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
        // frac_part = raw % 100000 (from the possibly-rounded magnitude); each
        // rendered digit is `(frac_part * 10) / 100000`, exhausting to zeros past
        // the 5th place so `precision > 5` pads with `0`.
        self.emit(abi::load_u64(raw, abi::stack_pointer(), magnitude_slot));
        self.emit(abi::move_immediate(scale, "Integer", "100000"));
        self.emit(abi::unsigned_divide_registers(int_part, raw, scale));
        self.emit(abi::multiply_subtract_registers(
            frac_part, int_part, scale, raw,
        ));
        self.emit(abi::move_immediate(counter, "Integer", "0"));
        self.emit(abi::move_immediate(divisor, "Integer", "10"));
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
            text: "toString(Money)".to_string(),
        })
    }

    /// Render a finite Float to text via the in-tree exact `%.*f` formatter
    /// (`float_format.rs`, `_mfb_rt_float_to_string`) — no libc involvement.
    /// The helper takes the f64 bit pattern in `x0` and the precision in `x1`,
    /// and returns the arena-alloc Result convention (tag in `x0`, String
    /// pointer in `x1`); the only possible failure is allocation.
    pub(super) fn emit_float_to_string_value(
        &mut self,
        source_register: &str,
        precision_register: &str,
    ) -> Result<ValueResult, String> {
        let alloc_ok = self.label("float_string_alloc_ok");
        self.emit(abi::move_register(abi::ARG[0], source_register));
        self.emit(abi::move_register(abi::ARG[1], precision_register));
        self.emit(abi::branch_link(FLOAT_TO_STRING_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: FLOAT_TO_STRING_SYMBOL.to_string(),
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
        let result = self.allocate_register()?;
        self.emit(abi::move_register(&result, abi::RET[1]));
        Ok(ValueResult {
            type_: "String".to_string(),
            location: result,
            text: "toString(Float)".to_string(),
        })
    }

    pub(super) fn lower_string_comparison_binary(
        &mut self,
        op: &str,
        left: &ValueResult,
        right: &ValueResult,
    ) -> Result<ValueResult, String> {
        match op {
            "=" | "<>" => {}
            "<" | ">" | "<=" | ">=" => {
                return self.lower_string_ordering_binary(op, left, right);
            }
            other => {
                return Err(format!(
                    "native code does not lower string comparison operator '{other}'"
                ));
            }
        }

        let left_len = self.temporary_vreg();
        let right_len = self.temporary_vreg();
        let left_ptr = self.temporary_vreg();
        let right_ptr = self.temporary_vreg();
        let left_byte = self.temporary_vreg();
        let right_byte = self.temporary_vreg();
        let result = self.allocate_register()?;
        let loop_label = self.label("cmp_string_loop");
        let equal_label = self.label("cmp_string_equal");
        let not_equal_label = self.label("cmp_string_not_equal");
        let done_label = self.label("cmp_string_done");

        self.emit(abi::load_u64(&left_len, &left.location, 0));
        self.emit(abi::load_u64(&right_len, &right.location, 0));
        self.emit(abi::compare_registers(&left_len, &right_len));
        self.emit(abi::branch_ne(&not_equal_label));
        self.emit(abi::add_immediate(&left_ptr, &left.location, 8));
        self.emit(abi::add_immediate(&right_ptr, &right.location, 8));
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate(&left_len, "0"));
        self.emit(abi::branch_eq(&equal_label));
        self.emit(abi::load_u8(&left_byte, &left_ptr, 0));
        self.emit(abi::load_u8(&right_byte, &right_ptr, 0));
        self.emit(abi::compare_registers(&left_byte, &right_byte));
        self.emit(abi::branch_ne(&not_equal_label));
        self.emit(abi::add_immediate(&left_ptr, &left_ptr, 1));
        self.emit(abi::add_immediate(&right_ptr, &right_ptr, 1));
        self.emit(abi::subtract_immediate(&left_len, &left_len, 1));
        self.emit(abi::branch(&loop_label));

        self.emit(abi::label(&equal_label));
        self.emit(abi::move_immediate(
            &result,
            "Boolean",
            if op == "=" { "true" } else { "false" },
        ));
        self.emit(abi::branch(&done_label));

        self.emit(abi::label(&not_equal_label));
        self.emit(abi::move_immediate(
            &result,
            "Boolean",
            if op == "=" { "false" } else { "true" },
        ));
        self.emit(abi::label(&done_label));

        Ok(ValueResult {
            type_: "Boolean".to_string(),
            location: result,
            text: format!("({} {op} {})", left.text, right.text),
        })
    }

    /// Lowers `<`, `>`, `<=`, `>=` for two `String` operands. The order is
    /// lexicographic by Unicode scalar value (§2.2): UTF-8 byte-wise comparison
    /// is identical to code-point order, so we compare the bytes of the common
    /// prefix and, if equal, the shorter string compares less. Bytes and lengths
    /// are compared unsigned. The result is target independent.
    pub(super) fn lower_string_ordering_binary(
        &mut self,
        op: &str,
        left: &ValueResult,
        right: &ValueResult,
    ) -> Result<ValueResult, String> {
        // Boolean outcome for each of the three orderings, per operator.
        let less_value = if matches!(op, "<" | "<=") {
            "true"
        } else {
            "false"
        };
        let greater_value = if matches!(op, ">" | ">=") {
            "true"
        } else {
            "false"
        };
        let equal_value = if matches!(op, "<=" | ">=") {
            "true"
        } else {
            "false"
        };

        let left_len = self.temporary_vreg();
        let right_len = self.temporary_vreg();
        let min_len = self.temporary_vreg();
        let left_ptr = self.temporary_vreg();
        let right_ptr = self.temporary_vreg();
        let left_byte = self.temporary_vreg();
        let result = self.allocate_register()?;
        let min_done_label = self.label("cmp_string_ord_min");
        let loop_label = self.label("cmp_string_ord_loop");
        let prefix_label = self.label("cmp_string_ord_prefix");
        let less_label = self.label("cmp_string_ord_less");
        let greater_label = self.label("cmp_string_ord_greater");
        let done_label = self.label("cmp_string_ord_done");

        // left_len = len(left), right_len = len(right); min_len = min of the two.
        self.emit(abi::load_u64(&left_len, &left.location, 0));
        self.emit(abi::load_u64(&right_len, &right.location, 0));
        self.emit(abi::move_register(&min_len, &left_len));
        self.emit(abi::compare_registers(&left_len, &right_len));
        self.emit(abi::branch_lo(&min_done_label));
        self.emit(abi::move_register(&min_len, &right_len));
        self.emit(abi::label(&min_done_label));

        // left_ptr/right_ptr point at the first data byte of each string.
        self.emit(abi::add_immediate(&left_ptr, &left.location, 8));
        self.emit(abi::add_immediate(&right_ptr, &right.location, 8));

        // Compare the common prefix byte by byte (right_len is free to reuse as a temp).
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate(&min_len, "0"));
        self.emit(abi::branch_eq(&prefix_label));
        self.emit(abi::load_u8(&left_byte, &left_ptr, 0));
        self.emit(abi::load_u8(&right_len, &right_ptr, 0));
        self.emit(abi::compare_registers(&left_byte, &right_len));
        self.emit(abi::branch_lo(&less_label));
        self.emit(abi::branch_hi(&greater_label));
        self.emit(abi::add_immediate(&left_ptr, &left_ptr, 1));
        self.emit(abi::add_immediate(&right_ptr, &right_ptr, 1));
        self.emit(abi::subtract_immediate(&min_len, &min_len, 1));
        self.emit(abi::branch(&loop_label));

        // Common prefix equal: the shorter string compares less.
        self.emit(abi::label(&prefix_label));
        self.emit(abi::load_u64(&left_len, &left.location, 0));
        self.emit(abi::load_u64(&right_len, &right.location, 0));
        self.emit(abi::compare_registers(&left_len, &right_len));
        self.emit(abi::branch_lo(&less_label));
        self.emit(abi::branch_hi(&greater_label));
        // Equal strings.
        self.emit(abi::move_immediate(&result, "Boolean", equal_value));
        self.emit(abi::branch(&done_label));

        self.emit(abi::label(&less_label));
        self.emit(abi::move_immediate(&result, "Boolean", less_value));
        self.emit(abi::branch(&done_label));

        self.emit(abi::label(&greater_label));
        self.emit(abi::move_immediate(&result, "Boolean", greater_value));
        self.emit(abi::label(&done_label));

        Ok(ValueResult {
            type_: "Boolean".to_string(),
            location: result,
            text: format!("({} {op} {})", left.text, right.text),
        })
    }
}
