use super::*;

impl CodeBuilder<'_> {
    pub(super) fn lower_find(&mut self, args: &[NirValue]) -> Result<ValueResult, String> {
        let haystack = self.lower_value(&args[0])?;
        if let Some(element_type) = list_element_type(&haystack.type_) {
            let haystack_slot = self.allocate_stack_object("find_list_haystack", 8);
            self.emit(abi::store_u64(
                &haystack.location,
                abi::stack_pointer(),
                haystack_slot,
            ));
            let needle = self.lower_value(&args[1])?;
            let needle_slot = self.allocate_stack_object("find_list_needle", 8);
            // `d`-native float needle stores via `str d` (plan-01 float-dnative).
            self.store_value_at(&needle, abi::stack_pointer(), needle_slot);
            let start_slot = self.allocate_stack_object("find_list_start", 8);
            if let Some(start) = args.get(2) {
                let start = self.lower_value(start)?;
                if start.type_ != "Integer" {
                    return Err(format!(
                        "native list find start must be Integer, got {}",
                        start.type_
                    ));
                }
                self.emit(abi::store_u64(
                    &start.location,
                    abi::stack_pointer(),
                    start_slot,
                ));
            } else {
                self.emit(abi::move_immediate("x8", "Integer", "0"));
                self.emit(abi::store_u64("x8", abi::stack_pointer(), start_slot));
            }

            if needle.type_ == element_type {
                return self.lower_list_find_item(
                    haystack_slot,
                    needle_slot,
                    start_slot,
                    &haystack.type_,
                    &element_type,
                );
            }
            if needle.type_ == haystack.type_ {
                return self.lower_list_find_sublist(
                    haystack_slot,
                    needle_slot,
                    start_slot,
                    &haystack.type_,
                    &element_type,
                );
            }
            return Err(format!(
                "native list find needle must be {} or {}, got {}",
                element_type, haystack.type_, needle.type_
            ));
        }
        if haystack.type_ != "String" {
            return Err(format!(
                "native string find haystack must be String, got {}",
                haystack.type_
            ));
        }
        let haystack_slot = self.allocate_stack_object("find_haystack", 8);
        self.emit(abi::store_u64(
            &haystack.location,
            abi::stack_pointer(),
            haystack_slot,
        ));

        let needle = self.lower_value(&args[1])?;
        if needle.type_ != "String" {
            return Err(format!(
                "native string find needle must be String, got {}",
                needle.type_
            ));
        }
        let needle_slot = self.allocate_stack_object("find_needle", 8);
        self.emit(abi::store_u64(
            &needle.location,
            abi::stack_pointer(),
            needle_slot,
        ));

        let start_slot = self.allocate_stack_object("find_start", 8);
        if let Some(start) = args.get(2) {
            let start = self.lower_value(start)?;
            if start.type_ != "Integer" {
                return Err(format!(
                    "native string find start must be Integer, got {}",
                    start.type_
                ));
            }
            self.emit(abi::store_u64(
                &start.location,
                abi::stack_pointer(),
                start_slot,
            ));
        } else {
            self.emit(abi::move_immediate("x8", "Integer", "0"));
            self.emit(abi::store_u64("x8", abi::stack_pointer(), start_slot));
        }

        let result_slot = self.allocate_stack_object("find_result", 8);
        let haystack_ptr = "x8";
        let needle_ptr = "x9";
        let haystack_len = "x10";
        let needle_len = "x11";
        let start = "x12";
        let scalar_index = "x13";
        let cursor = "x14";
        let remaining = "x15";
        let byte = "x16";
        let mask = "x17";
        let candidate = "x20";
        let compare_remaining = "x21";
        let needle_cursor = "x22";
        let haystack_byte = "x23";
        let needle_byte = "x24";
        for register in [
            candidate,
            compare_remaining,
            needle_cursor,
            haystack_byte,
            needle_byte,
        ] {
            if abi::is_callee_saved(register)
                && !self.used_callee_saved.iter().any(|saved| saved == register)
            {
                self.used_callee_saved.push(register.to_string());
            }
        }

        let locate_start = self.label("find_locate_start");
        let locate_continue = self.label("find_locate_continue");
        let start_ready = self.label("find_start_ready");
        let search_loop = self.label("find_search_loop");
        let compare_loop = self.label("find_compare_loop");
        let advance_candidate = self.label("find_advance_candidate");
        let skip_continuation = self.label("find_skip_continuation");
        let candidate_ready = self.label("find_candidate_ready");
        let found = self.label("find_found");
        let invalid_start = self.label("find_invalid_start");
        let not_found = self.label("find_not_found");

        self.emit(abi::load_u64(
            haystack_ptr,
            abi::stack_pointer(),
            haystack_slot,
        ));
        self.emit(abi::load_u64(needle_ptr, abi::stack_pointer(), needle_slot));
        self.emit(abi::load_u64(haystack_len, haystack_ptr, 0));
        self.emit(abi::load_u64(needle_len, needle_ptr, 0));
        self.emit(abi::load_u64(start, abi::stack_pointer(), start_slot));
        self.emit(abi::move_immediate(scalar_index, "Integer", "0"));
        self.emit(abi::add_immediate(cursor, haystack_ptr, 8));
        self.emit(abi::move_register(remaining, haystack_len));
        self.emit(abi::move_immediate(mask, "Integer", "192"));

        self.emit(abi::label(&locate_start));
        self.emit(abi::compare_registers(scalar_index, start));
        self.emit(abi::branch_eq(&start_ready));
        self.emit(abi::compare_immediate(remaining, "0"));
        self.emit(abi::branch_eq(&invalid_start));
        self.emit(abi::load_u8(byte, cursor, 0));
        self.emit(abi::and_registers(byte, byte, mask));
        self.emit(abi::compare_immediate(byte, "128"));
        self.emit(abi::branch_eq(&locate_continue));
        self.emit(abi::add_immediate(scalar_index, scalar_index, 1));
        self.emit(abi::label(&locate_continue));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::subtract_immediate(remaining, remaining, 1));
        self.emit(abi::branch(&locate_start));

        self.emit(abi::label(&start_ready));
        self.emit(abi::compare_immediate(needle_len, "0"));
        self.emit(abi::branch_eq(&found));

        self.emit(abi::label(&search_loop));
        self.emit(abi::compare_registers(remaining, needle_len));
        self.emit(abi::branch_lo(&not_found));
        self.emit(abi::move_register(candidate, cursor));
        self.emit(abi::add_immediate(needle_cursor, needle_ptr, 8));
        self.emit(abi::move_register(compare_remaining, needle_len));

        self.emit(abi::label(&compare_loop));
        self.emit(abi::compare_immediate(compare_remaining, "0"));
        self.emit(abi::branch_eq(&found));
        self.emit(abi::load_u8(haystack_byte, candidate, 0));
        self.emit(abi::load_u8(needle_byte, needle_cursor, 0));
        self.emit(abi::compare_registers(haystack_byte, needle_byte));
        self.emit(abi::branch_ne(&advance_candidate));
        self.emit(abi::add_immediate(candidate, candidate, 1));
        self.emit(abi::add_immediate(needle_cursor, needle_cursor, 1));
        self.emit(abi::subtract_immediate(
            compare_remaining,
            compare_remaining,
            1,
        ));
        self.emit(abi::branch(&compare_loop));

        self.emit(abi::label(&advance_candidate));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::subtract_immediate(remaining, remaining, 1));
        self.emit(abi::label(&skip_continuation));
        self.emit(abi::compare_immediate(remaining, "0"));
        self.emit(abi::branch_eq(&candidate_ready));
        self.emit(abi::load_u8(byte, cursor, 0));
        self.emit(abi::and_registers(byte, byte, mask));
        self.emit(abi::compare_immediate(byte, "128"));
        self.emit(abi::branch_ne(&candidate_ready));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::subtract_immediate(remaining, remaining, 1));
        self.emit(abi::branch(&skip_continuation));
        self.emit(abi::label(&candidate_ready));
        self.emit(abi::add_immediate(scalar_index, scalar_index, 1));
        self.emit(abi::branch(&search_loop));

        self.emit(abi::label(&invalid_start));
        self.emit_index_out_of_range_return()?;
        self.emit(abi::label(&not_found));
        self.emit_not_found_return()?;
        self.emit(abi::label(&found));
        self.emit(abi::store_u64(
            scalar_index,
            abi::stack_pointer(),
            result_slot,
        ));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));

        Ok(ValueResult {
            type_: "Integer".to_string(),
            location: result,
            text: "find(String, String)".to_string(),
        })
    }

    pub(super) fn lower_list_find_item(
        &mut self,
        haystack_slot: usize,
        needle_slot: usize,
        start_slot: usize,
        list_type: &str,
        element_type: &str,
    ) -> Result<ValueResult, String> {
        let result_slot = self.allocate_stack_object("find_list_result", 8);
        let valid_start = self.label("list_find_item_valid_start");
        let loop_label = self.label("list_find_item_loop");
        let found = self.label("list_find_item_found");
        let next = self.label("list_find_item_next");
        let invalid_start = self.label("list_find_item_invalid_start");
        let not_found = self.label("list_find_item_not_found");
        let done = self.label("list_find_item_done");

        self.emit(abi::load_u64("x8", abi::stack_pointer(), haystack_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), needle_slot));
        self.emit(abi::load_u64("x10", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64("x11", abi::stack_pointer(), start_slot));
        self.emit(abi::compare_immediate("x11", "0"));
        self.emit(abi::branch_ge(&valid_start));
        self.emit(abi::branch(&invalid_start));
        self.emit(abi::label(&valid_start));
        self.emit(abi::compare_registers("x11", "x10"));
        self.emit(abi::branch_gt(&invalid_start));
        self.emit(abi::move_register("x12", "x11"));
        self.emit(abi::move_immediate(
            "x13",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x14", "x12", "x13"));
        self.emit(abi::add_immediate("x15", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers("x15", "x15", "x14"));

        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_registers("x12", "x10"));
        self.emit(abi::branch_ge(&not_found));
        self.emit(abi::load_u64(
            "x16",
            "x15",
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            "x17",
            "x15",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit_collection_payload_matches_value_branch(
            element_type,
            "x8",
            "x16",
            "x17",
            "x9",
            &found,
            &next,
        )?;

        self.emit(abi::label(&next));
        self.emit(abi::add_immediate("x15", "x15", COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate("x12", "x12", 1));
        self.emit(abi::branch(&loop_label));

        self.emit(abi::label(&invalid_start));
        self.emit_index_out_of_range_return()?;
        self.emit(abi::label(&not_found));
        self.emit_not_found_return()?;
        self.emit(abi::label(&found));
        self.emit(abi::store_u64("x12", abi::stack_pointer(), result_slot));
        self.emit(abi::branch(&done));
        self.emit(abi::label(&done));

        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: "Integer".to_string(),
            location: result,
            text: format!("find({list_type}, {element_type})"),
        })
    }

    pub(super) fn lower_list_find_sublist(
        &mut self,
        haystack_slot: usize,
        needle_slot: usize,
        start_slot: usize,
        list_type: &str,
        element_type: &str,
    ) -> Result<ValueResult, String> {
        for register in ["x20", "x21", "x22", "x23", "x24", "x25"] {
            self.mark_register_used(register);
        }

        let result_slot = self.allocate_stack_object("find_sublist_result", 8);
        let valid_start = self.label("list_find_sublist_valid_start");
        let empty_found = self.label("list_find_sublist_empty_found");
        let outer_loop = self.label("list_find_sublist_outer_loop");
        let compare_loop = self.label("list_find_sublist_compare_loop");
        let compare_next = self.label("list_find_sublist_compare_next");
        let found = self.label("list_find_sublist_found");
        let advance_outer = self.label("list_find_sublist_advance_outer");
        let invalid_start = self.label("list_find_sublist_invalid_start");
        let not_found = self.label("list_find_sublist_not_found");
        let done = self.label("list_find_sublist_done");

        self.emit(abi::load_u64("x8", abi::stack_pointer(), haystack_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), needle_slot));
        self.emit(abi::load_u64("x10", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64("x11", "x9", COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64("x12", abi::stack_pointer(), start_slot));
        self.emit(abi::compare_immediate("x12", "0"));
        self.emit(abi::branch_ge(&valid_start));
        self.emit(abi::branch(&invalid_start));
        self.emit(abi::label(&valid_start));
        self.emit(abi::compare_registers("x12", "x10"));
        self.emit(abi::branch_gt(&invalid_start));
        self.emit(abi::compare_immediate("x11", "0"));
        self.emit(abi::branch_eq(&empty_found));

        self.emit(abi::move_register("x13", "x12"));
        self.emit(abi::label(&outer_loop));
        self.emit(abi::add_registers("x14", "x13", "x11"));
        self.emit(abi::compare_registers("x14", "x10"));
        self.emit(abi::branch_gt(&not_found));
        self.emit(abi::move_immediate("x14", "Integer", "0"));

        self.emit(abi::label(&compare_loop));
        self.emit(abi::compare_registers("x14", "x11"));
        self.emit(abi::branch_eq(&found));
        self.emit(abi::add_registers("x15", "x13", "x14"));
        self.emit(abi::move_immediate(
            "x16",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x15", "x15", "x16"));
        self.emit(abi::add_immediate("x17", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers("x17", "x17", "x15"));
        self.emit(abi::multiply_registers("x20", "x14", "x16"));
        self.emit(abi::add_immediate("x25", "x9", COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers("x20", "x25", "x20"));
        self.emit(abi::load_u64(
            "x21",
            "x17",
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            "x22",
            "x17",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::load_u64(
            "x23",
            "x20",
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            "x24",
            "x20",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit_collection_payloads_match_branch(
            element_type,
            "x8",
            "x21",
            "x22",
            "x9",
            "x23",
            "x24",
            &compare_next,
            &advance_outer,
        )?;

        self.emit(abi::label(&compare_next));
        self.emit(abi::add_immediate("x14", "x14", 1));
        self.emit(abi::branch(&compare_loop));

        self.emit(abi::label(&advance_outer));
        self.emit(abi::add_immediate("x13", "x13", 1));
        self.emit(abi::branch(&outer_loop));

        self.emit(abi::label(&empty_found));
        self.emit(abi::store_u64("x12", abi::stack_pointer(), result_slot));
        self.emit(abi::branch(&done));
        self.emit(abi::label(&invalid_start));
        self.emit_index_out_of_range_return()?;
        self.emit(abi::label(&not_found));
        self.emit_not_found_return()?;
        self.emit(abi::label(&found));
        self.emit(abi::store_u64("x13", abi::stack_pointer(), result_slot));
        self.emit(abi::label(&done));

        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: "Integer".to_string(),
            location: result,
            text: format!("find({list_type}, {list_type}) over {element_type}"),
        })
    }

    pub(super) fn lower_mid(&mut self, args: &[NirValue]) -> Result<ValueResult, String> {
        let value = self.lower_value(&args[0])?;
        if let Some(element_type) = list_element_type(&value.type_) {
            let value_slot = self.allocate_stack_object("mid_list_value", 8);
            self.emit(abi::store_u64(
                &value.location,
                abi::stack_pointer(),
                value_slot,
            ));
            let start = self.lower_value(&args[1])?;
            if start.type_ != "Integer" {
                return Err(format!(
                    "native list mid start must be Integer, got {}",
                    start.type_
                ));
            }
            let start_slot = self.allocate_stack_object("mid_list_start", 8);
            self.emit(abi::store_u64(
                &start.location,
                abi::stack_pointer(),
                start_slot,
            ));
            let count = self.lower_value(&args[2])?;
            if count.type_ != "Integer" {
                return Err(format!(
                    "native list mid count must be Integer, got {}",
                    count.type_
                ));
            }
            let count_slot = self.allocate_stack_object("mid_list_count", 8);
            self.emit(abi::store_u64(
                &count.location,
                abi::stack_pointer(),
                count_slot,
            ));
            return self.lower_list_mid(
                value_slot,
                start_slot,
                count_slot,
                &value.type_,
                &element_type,
            );
        }
        if value.type_ != "String" {
            return Err(format!(
                "native string mid value must be String, got {}",
                value.type_
            ));
        }
        let value_slot = self.allocate_stack_object("mid_value", 8);
        self.emit(abi::store_u64(
            &value.location,
            abi::stack_pointer(),
            value_slot,
        ));

        let start = self.lower_value(&args[1])?;
        if start.type_ != "Integer" {
            return Err(format!(
                "native string mid start must be Integer, got {}",
                start.type_
            ));
        }
        let start_slot = self.allocate_stack_object("mid_start", 8);
        self.emit(abi::store_u64(
            &start.location,
            abi::stack_pointer(),
            start_slot,
        ));

        let count = self.lower_value(&args[2])?;
        if count.type_ != "Integer" {
            return Err(format!(
                "native string mid count must be Integer, got {}",
                count.type_
            ));
        }
        let count_slot = self.allocate_stack_object("mid_count", 8);
        self.emit(abi::store_u64(
            &count.location,
            abi::stack_pointer(),
            count_slot,
        ));

        let result_slot = self.allocate_stack_object("mid_result", 8);
        let start_ptr_slot = self.allocate_stack_object("mid_start_ptr", 8);
        let byte_len_slot = self.allocate_stack_object("mid_byte_len", 8);
        let value_ptr = "x8";
        let string_len = "x9";
        let cursor = "x10";
        let remaining = "x11";
        let scalar_index = "x12";
        let start_index = "x13";
        let count_value = "x14";
        let end_index = "x15";
        let byte = "x16";
        let mask = "x17";
        let start_ptr = "x20";
        let end_ptr = "x21";
        let copy_src = "x22";
        let copy_dst = "x23";
        let copy_remaining = "x24";
        let byte_len = "x25";
        for register in [
            start_ptr,
            end_ptr,
            copy_src,
            copy_dst,
            copy_remaining,
            byte_len,
        ] {
            if abi::is_callee_saved(register)
                && !self.used_callee_saved.iter().any(|saved| saved == register)
            {
                self.used_callee_saved.push(register.to_string());
            }
        }

        let locate_start = self.label("mid_locate_start");
        let locate_start_continue = self.label("mid_locate_start_continue");
        let locate_start_advanced = self.label("mid_locate_start_advanced");
        let start_ready = self.label("mid_start_ready");
        let locate_end = self.label("mid_locate_end");
        let locate_end_continue = self.label("mid_locate_end_continue");
        let locate_end_advanced = self.label("mid_locate_end_advanced");
        let end_ready = self.label("mid_end_ready");
        let alloc_ok = self.label("mid_alloc_ok");
        let copy_loop = self.label("mid_copy_loop");
        let copy_done = self.label("mid_copy_done");
        let invalid_range = self.label("mid_invalid_range");

        self.emit(abi::load_u64(value_ptr, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(start_index, abi::stack_pointer(), start_slot));
        self.emit(abi::load_u64(count_value, abi::stack_pointer(), count_slot));
        self.emit(abi::compare_immediate(start_index, "0"));
        self.emit(abi::branch_lt(&invalid_range));
        self.emit(abi::compare_immediate(count_value, "0"));
        self.emit(abi::branch_lt(&invalid_range));
        self.emit(abi::add_registers(end_index, start_index, count_value));
        self.emit(abi::compare_registers(end_index, start_index));
        self.emit(abi::branch_lo(&invalid_range));
        self.emit(abi::load_u64(string_len, value_ptr, 0));
        self.emit(abi::add_immediate(cursor, value_ptr, 8));
        self.emit(abi::move_register(start_ptr, cursor));
        self.emit(abi::move_register(end_ptr, cursor));
        self.emit(abi::move_register(remaining, string_len));
        self.emit(abi::move_immediate(scalar_index, "Integer", "0"));
        self.emit(abi::move_immediate(mask, "Integer", "192"));

        self.emit(abi::label(&locate_start));
        self.emit(abi::compare_registers(scalar_index, start_index));
        self.emit(abi::branch_eq(&start_ready));
        self.emit(abi::compare_immediate(remaining, "0"));
        self.emit(abi::branch_eq(&invalid_range));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::subtract_immediate(remaining, remaining, 1));
        self.emit(abi::label(&locate_start_continue));
        self.emit(abi::compare_immediate(remaining, "0"));
        self.emit(abi::branch_eq(&locate_start_advanced));
        self.emit(abi::load_u8(byte, cursor, 0));
        self.emit(abi::and_registers(byte, byte, mask));
        self.emit(abi::compare_immediate(byte, "128"));
        self.emit(abi::branch_ne(&locate_start_advanced));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::subtract_immediate(remaining, remaining, 1));
        self.emit(abi::branch(&locate_start_continue));
        self.emit(abi::label(&locate_start_advanced));
        self.emit(abi::add_immediate(scalar_index, scalar_index, 1));
        self.emit(abi::branch(&locate_start));

        self.emit(abi::label(&start_ready));
        self.emit(abi::move_register(start_ptr, cursor));
        self.emit(abi::move_register(end_ptr, cursor));
        self.emit(abi::label(&locate_end));
        self.emit(abi::compare_registers(scalar_index, end_index));
        self.emit(abi::branch_eq(&end_ready));
        self.emit(abi::compare_immediate(remaining, "0"));
        self.emit(abi::branch_eq(&invalid_range));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::subtract_immediate(remaining, remaining, 1));
        self.emit(abi::label(&locate_end_continue));
        self.emit(abi::compare_immediate(remaining, "0"));
        self.emit(abi::branch_eq(&locate_end_advanced));
        self.emit(abi::load_u8(byte, cursor, 0));
        self.emit(abi::and_registers(byte, byte, mask));
        self.emit(abi::compare_immediate(byte, "128"));
        self.emit(abi::branch_ne(&locate_end_advanced));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::subtract_immediate(remaining, remaining, 1));
        self.emit(abi::branch(&locate_end_continue));
        self.emit(abi::label(&locate_end_advanced));
        self.emit(abi::add_immediate(scalar_index, scalar_index, 1));
        self.emit(abi::branch(&locate_end));

        self.emit(abi::label(&end_ready));
        self.emit(abi::move_register(end_ptr, cursor));
        self.emit(abi::subtract_registers(byte_len, end_ptr, start_ptr));
        self.emit(abi::store_u64(
            start_ptr,
            abi::stack_pointer(),
            start_ptr_slot,
        ));
        self.emit(abi::store_u64(
            byte_len,
            abi::stack_pointer(),
            byte_len_slot,
        ));
        self.emit(abi::add_immediate(abi::return_register(), byte_len, 9));
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
        self.emit(abi::load_u64(byte_len, abi::stack_pointer(), byte_len_slot));
        self.emit(abi::store_u64(byte_len, "x1", 0));
        self.emit(abi::load_u64(
            start_ptr,
            abi::stack_pointer(),
            start_ptr_slot,
        ));
        self.emit(abi::move_register(copy_src, start_ptr));
        self.emit(abi::add_immediate(copy_dst, "x1", 8));
        self.emit(abi::move_register(copy_remaining, byte_len));
        self.emit(abi::label(&copy_loop));
        self.emit(abi::compare_immediate(copy_remaining, "0"));
        self.emit(abi::branch_eq(&copy_done));
        self.emit(abi::load_u8(byte, copy_src, 0));
        self.emit(abi::store_u8(byte, copy_dst, 0));
        self.emit(abi::add_immediate(copy_src, copy_src, 1));
        self.emit(abi::add_immediate(copy_dst, copy_dst, 1));
        self.emit(abi::subtract_immediate(copy_remaining, copy_remaining, 1));
        self.emit(abi::branch(&copy_loop));
        self.emit(abi::label(&copy_done));
        self.emit(abi::move_immediate(byte, "Integer", "0"));
        self.emit(abi::store_u8(byte, copy_dst, 0));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        let done = self.label("mid_done");
        self.emit(abi::branch(&done));

        self.emit(abi::label(&invalid_range));
        self.emit_index_out_of_range_return()?;
        self.emit(abi::label(&done));

        Ok(ValueResult {
            type_: "String".to_string(),
            location: result,
            text: "mid(String, Integer, Integer)".to_string(),
        })
    }

    pub(super) fn lower_list_mid(
        &mut self,
        value_slot: usize,
        start_slot: usize,
        count_slot: usize,
        list_type: &str,
        element_type: &str,
    ) -> Result<ValueResult, String> {
        let layout = CollectionTypeLayout::from_type(list_type)
            .ok_or_else(|| format!("native code collection type '{list_type}' is not supported"))?;
        for register in ["x20", "x21", "x22", "x23", "x24", "x25"] {
            self.mark_register_used(register);
        }

        let data_len_slot = self.allocate_stack_object("mid_list_data_len", 8);
        let result_slot = self.allocate_stack_object("mid_list_result", 8);
        let valid_start = self.label("mid_list_valid_start");
        let valid_count = self.label("mid_list_valid_count");
        let range_ok = self.label("mid_list_range_ok");
        let length_loop = self.label("mid_list_length_loop");
        let length_done = self.label("mid_list_length_done");
        let alloc_ok = self.label("mid_list_alloc_ok");
        let copy_loop = self.label("mid_list_copy_loop");
        let copy_entry = self.label("mid_list_copy_entry");
        let copy_bytes = self.label("mid_list_copy_bytes");
        let copy_bytes_done = self.label("mid_list_copy_bytes_done");
        let copy_done = self.label("mid_list_copy_done");
        let invalid_range = self.label("mid_list_invalid_range");
        let done = self.label("mid_list_done");

        self.emit(abi::load_u64("x8", abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), start_slot));
        self.emit(abi::load_u64("x10", abi::stack_pointer(), count_slot));
        self.emit(abi::load_u64("x11", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::compare_immediate("x9", "0"));
        self.emit(abi::branch_ge(&valid_start));
        self.emit(abi::branch(&invalid_range));
        self.emit(abi::label(&valid_start));
        self.emit(abi::compare_immediate("x10", "0"));
        self.emit(abi::branch_ge(&valid_count));
        self.emit(abi::branch(&invalid_range));
        self.emit(abi::label(&valid_count));
        self.emit(abi::compare_registers("x9", "x11"));
        self.emit(abi::branch_gt(&invalid_range));
        self.emit(abi::add_registers("x12", "x9", "x10"));
        self.emit(abi::compare_registers("x12", "x9"));
        self.emit(abi::branch_lt(&invalid_range));
        self.emit(abi::compare_registers("x12", "x11"));
        self.emit(abi::branch_le(&range_ok));
        self.emit(abi::branch(&invalid_range));

        self.emit(abi::label(&range_ok));
        self.emit(abi::move_immediate("x13", "Integer", "0"));
        self.emit(abi::store_u64("x13", abi::stack_pointer(), data_len_slot));
        self.emit(abi::move_immediate(
            "x14",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x15", "x9", "x14"));
        self.emit(abi::add_immediate("x16", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers("x16", "x16", "x15"));

        self.emit(abi::label(&length_loop));
        self.emit(abi::compare_registers("x13", "x10"));
        self.emit(abi::branch_ge(&length_done));
        self.emit(abi::load_u64(
            "x17",
            "x16",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::load_u64("x20", abi::stack_pointer(), data_len_slot));
        self.emit(abi::add_registers("x20", "x20", "x17"));
        self.emit(abi::store_u64("x20", abi::stack_pointer(), data_len_slot));
        self.emit(abi::add_immediate("x16", "x16", COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate("x13", "x13", 1));
        self.emit(abi::branch(&length_loop));

        self.emit(abi::label(&length_done));
        self.emit(abi::move_immediate(
            "x14",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x15", "x10", "x14"));
        self.emit(abi::load_u64("x16", abi::stack_pointer(), data_len_slot));
        self.emit(abi::add_immediate(
            abi::return_register(),
            "x15",
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(
            abi::return_register(),
            abi::return_register(),
            "x16",
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
        self.emit(abi::load_u64("x10", abi::stack_pointer(), count_slot));
        self.emit(abi::store_u64("x10", "x1", COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64("x10", "x1", COLLECTION_OFFSET_CAPACITY));
        self.emit(abi::load_u64("x16", abi::stack_pointer(), data_len_slot));
        self.emit(abi::store_u64("x16", "x1", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::store_u64("x16", "x1", COLLECTION_OFFSET_DATA_CAPACITY));

        self.emit(abi::load_u64("x8", abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), start_slot));
        self.emit(abi::load_u64("x10", abi::stack_pointer(), count_slot));
        self.emit(abi::load_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::move_immediate(
            "x14",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x15", "x9", "x14"));
        self.emit(abi::add_immediate("x16", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers("x16", "x16", "x15"));
        self.emit(abi::add_immediate("x17", "x1", COLLECTION_HEADER_SIZE));
        self.emit_collection_data_pointer("x20", "x8");
        self.emit(abi::multiply_registers("x21", "x10", "x14"));
        self.emit(abi::add_registers("x21", "x17", "x21"));
        self.emit(abi::move_immediate("x12", "Integer", "0"));
        self.emit(abi::move_immediate("x13", "Integer", "0"));

        self.emit(abi::label(&copy_loop));
        self.emit(abi::compare_registers("x12", "x10"));
        self.emit(abi::branch_ge(&copy_done));
        self.emit(abi::label(&copy_entry));
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
        self.emit(abi::store_u64(
            "x23",
            "x17",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers("x24", "x20", "x22"));
        self.emit(abi::add_registers("x25", "x21", "x13"));

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
        self.emit(abi::add_registers("x13", "x13", "x23"));
        self.emit(abi::add_immediate("x16", "x16", COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate("x17", "x17", COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate("x12", "x12", 1));
        self.emit(abi::branch(&copy_loop));

        self.emit(abi::label(&invalid_range));
        self.emit_index_out_of_range_return()?;
        self.emit(abi::label(&copy_done));
        self.emit(abi::branch(&done));
        self.emit(abi::label(&done));

        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: list_type.to_string(),
            location: result,
            text: format!("mid({list_type}, Integer, Integer) over {element_type}"),
        })
    }
}
