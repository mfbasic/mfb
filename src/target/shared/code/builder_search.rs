use super::*;

impl CodeBuilder<'_> {
    pub(super) fn lower_find(&mut self, args: &[NirValue]) -> Result<ValueResult, String> {
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
                self.emit(abi::move_immediate(&scratch8, "Integer", "0"));
                self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), start_slot));
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
            self.emit(abi::move_immediate(&scratch8, "Integer", "0"));
            self.emit(abi::store_u64(&scratch8, abi::stack_pointer(), start_slot));
        }

        let result_slot = self.allocate_stack_object("find_result", 8);
        let haystack_ptr = scratch8.as_str();
        let needle_ptr = scratch9.as_str();
        let haystack_len = scratch10.as_str();
        let needle_len = scratch11.as_str();
        let start = scratch12.as_str();
        let scalar_index = scratch13.as_str();
        let cursor = scratch14.as_str();
        let remaining = scratch15.as_str();
        let byte = scratch16.as_str();
        let mask = scratch17.as_str();
        let candidate = scratch20.as_str();
        let compare_remaining = scratch21.as_str();
        let needle_cursor = scratch22.as_str();
        let haystack_byte = scratch23.as_str();
        let needle_byte = scratch24.as_str();

        let locate_start = self.label("find_locate_start");
        let locate_continue = self.label("find_locate_continue");
        let locate_advanced = self.label("find_locate_advanced");
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
        // Negative start: raise the documented 77050001 immediately instead of
        // relying on loop exhaustion after a full O(n) walk (audit-unicode #6);
        // matches the explicit checks in list-find and mid.
        self.emit(abi::compare_immediate(start, "0"));
        self.emit(abi::branch_lt(&invalid_start));
        self.emit(abi::move_immediate(scalar_index, "Integer", "0"));
        self.emit(abi::add_immediate(cursor, haystack_ptr, 8));
        self.emit(abi::move_register(remaining, haystack_len));
        self.emit(abi::move_immediate(mask, "Integer", "192"));

        // Walk `start` characters forward. Each character is one lead/ASCII byte
        // followed by its continuation bytes; consume the whole character before
        // re-checking `scalar_index == start` so the cursor always lands on a
        // character boundary. The earlier form re-checked equality after seeing a
        // lead byte but before skipping its continuations, so when character
        // `start-1` was multibyte the cursor stopped on a continuation byte and
        // every returned index was inflated by one (bug-133). Mirrors `lower_mid`.
        self.emit(abi::label(&locate_start));
        self.emit(abi::compare_registers(scalar_index, start));
        self.emit(abi::branch_eq(&start_ready));
        self.emit(abi::compare_immediate(remaining, "0"));
        self.emit(abi::branch_eq(&invalid_start));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::subtract_immediate(remaining, remaining, 1));
        self.emit(abi::label(&locate_continue));
        self.emit(abi::compare_immediate(remaining, "0"));
        self.emit(abi::branch_eq(&locate_advanced));
        self.emit(abi::load_u8(byte, cursor, 0));
        self.emit(abi::and_registers(byte, byte, mask));
        self.emit(abi::compare_immediate(byte, "128"));
        self.emit(abi::branch_ne(&locate_advanced));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::subtract_immediate(remaining, remaining, 1));
        self.emit(abi::branch(&locate_continue));
        self.emit(abi::label(&locate_advanced));
        self.emit(abi::add_immediate(scalar_index, scalar_index, 1));
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
        let result_slot = self.allocate_stack_object("find_list_result", 8);
        let valid_start = self.label("list_find_item_valid_start");
        let loop_label = self.label("list_find_item_loop");
        let found = self.label("list_find_item_found");
        let next = self.label("list_find_item_next");
        let invalid_start = self.label("list_find_item_invalid_start");
        let not_found = self.label("list_find_item_not_found");
        let done = self.label("list_find_item_done");

        self.emit(abi::load_u64(
            &scratch8,
            abi::stack_pointer(),
            haystack_slot,
        ));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), needle_slot));
        self.emit(abi::load_u64(
            &scratch10,
            &scratch8,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::load_u64(&scratch11, abi::stack_pointer(), start_slot));
        self.emit(abi::compare_immediate(&scratch11, "0"));
        self.emit(abi::branch_ge(&valid_start));
        self.emit(abi::branch(&invalid_start));
        self.emit(abi::label(&valid_start));
        self.emit(abi::compare_registers(&scratch11, &scratch10));
        self.emit(abi::branch_gt(&invalid_start));
        // kind 2: scratch15 is a byte OFFSET into the data region, seeded at
        // `start * payload`, and the span is derivable from it (plan-57-D).
        let find_payload = kind2_payload_size(element_type);
        self.emit(abi::move_register(&scratch12, &scratch11));
        self.emit(abi::move_immediate(
            &scratch13,
            "Integer",
            &find_payload.unwrap_or(COLLECTION_ENTRY_SIZE).to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch14, &scratch12, &scratch13));
        if find_payload.is_some() {
            self.emit(abi::move_register(&scratch15, &scratch14));
        } else {
            self.emit(abi::add_immediate(
                &scratch15,
                &scratch8,
                COLLECTION_HEADER_SIZE,
            ));
            self.emit(abi::add_registers(&scratch15, &scratch15, &scratch14));
        }

        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_registers(&scratch12, &scratch10));
        self.emit(abi::branch_ge(&not_found));
        if let Some(payload) = find_payload {
            self.emit(abi::move_register(&scratch16, &scratch15));
            self.emit(abi::move_immediate(
                &scratch17,
                "Integer",
                &payload.to_string(),
            ));
        } else {
            self.emit(abi::load_u64(
                &scratch16,
                &scratch15,
                COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
            ));
            self.emit(abi::load_u64(
                &scratch17,
                &scratch15,
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ));
        }
        self.emit_collection_payload_matches_value_branch(
            element_type,
            &scratch8,
            &scratch16,
            &scratch17,
            &scratch9,
            &found,
            &next,
        )?;

        self.emit(abi::label(&next));
        self.emit(abi::add_immediate(
            &scratch15,
            &scratch15,
            find_payload.unwrap_or(COLLECTION_ENTRY_SIZE),
        ));
        self.emit(abi::add_immediate(&scratch12, &scratch12, 1));
        self.emit(abi::branch(&loop_label));

        self.emit(abi::label(&invalid_start));
        self.emit_index_out_of_range_return()?;
        self.emit(abi::label(&not_found));
        self.emit_not_found_return()?;
        self.emit(abi::label(&found));
        self.emit(abi::store_u64(
            &scratch12,
            abi::stack_pointer(),
            result_slot,
        ));
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

        self.emit(abi::load_u64(
            &scratch8,
            abi::stack_pointer(),
            haystack_slot,
        ));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), needle_slot));
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
        self.emit(abi::load_u64(&scratch12, abi::stack_pointer(), start_slot));
        self.emit(abi::compare_immediate(&scratch12, "0"));
        self.emit(abi::branch_ge(&valid_start));
        self.emit(abi::branch(&invalid_start));
        self.emit(abi::label(&valid_start));
        self.emit(abi::compare_registers(&scratch12, &scratch10));
        self.emit(abi::branch_gt(&invalid_start));
        self.emit(abi::compare_immediate(&scratch11, "0"));
        self.emit(abi::branch_eq(&empty_found));

        self.emit(abi::move_register(&scratch13, &scratch12));
        self.emit(abi::label(&outer_loop));
        self.emit(abi::add_registers(&scratch14, &scratch13, &scratch11));
        self.emit(abi::compare_registers(&scratch14, &scratch10));
        self.emit(abi::branch_gt(&not_found));
        self.emit(abi::move_immediate(&scratch14, "Integer", "0"));

        self.emit(abi::label(&compare_loop));
        self.emit(abi::compare_registers(&scratch14, &scratch11));
        self.emit(abi::branch_eq(&found));
        self.emit(abi::add_registers(&scratch15, &scratch13, &scratch14));
        self.emit(abi::move_immediate(
            &scratch16,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch15, &scratch15, &scratch16));
        self.emit(abi::add_immediate(
            &scratch17,
            &scratch8,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(&scratch17, &scratch17, &scratch15));
        self.emit(abi::multiply_registers(&scratch20, &scratch14, &scratch16));
        self.emit(abi::add_immediate(
            &scratch25,
            &scratch9,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(&scratch20, &scratch25, &scratch20));
        self.emit(abi::load_u64(
            &scratch21,
            &scratch17,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            &scratch22,
            &scratch17,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::load_u64(
            &scratch23,
            &scratch20,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            &scratch24,
            &scratch20,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit_collection_payloads_match_branch(
            element_type,
            &scratch8,
            &scratch21,
            &scratch22,
            &scratch9,
            &scratch23,
            &scratch24,
            &compare_next,
            &advance_outer,
        )?;

        self.emit(abi::label(&compare_next));
        self.emit(abi::add_immediate(&scratch14, &scratch14, 1));
        self.emit(abi::branch(&compare_loop));

        self.emit(abi::label(&advance_outer));
        self.emit(abi::add_immediate(&scratch13, &scratch13, 1));
        self.emit(abi::branch(&outer_loop));

        self.emit(abi::label(&empty_found));
        self.emit(abi::store_u64(
            &scratch12,
            abi::stack_pointer(),
            result_slot,
        ));
        self.emit(abi::branch(&done));
        self.emit(abi::label(&invalid_start));
        self.emit_index_out_of_range_return()?;
        self.emit(abi::label(&not_found));
        self.emit_not_found_return()?;
        self.emit(abi::label(&found));
        self.emit(abi::store_u64(
            &scratch13,
            abi::stack_pointer(),
            result_slot,
        ));
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
        let value_ptr = scratch8.as_str();
        let string_len = scratch9.as_str();
        let cursor = scratch10.as_str();
        let remaining = scratch11.as_str();
        let scalar_index = scratch12.as_str();
        let start_index = scratch13.as_str();
        let count_value = scratch14.as_str();
        let end_index = scratch15.as_str();
        let byte = scratch16.as_str();
        let mask = scratch17.as_str();
        let start_ptr = scratch20.as_str();
        let end_ptr = scratch21.as_str();
        let copy_src = scratch22.as_str();
        let copy_dst = scratch23.as_str();
        let copy_remaining = scratch24.as_str();
        let byte_len = scratch25.as_str();

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
        self.emit(abi::load_u64(byte_len, abi::stack_pointer(), byte_len_slot));
        self.emit(abi::store_u64(byte_len, abi::RET[1], 0));
        self.emit(abi::load_u64(
            start_ptr,
            abi::stack_pointer(),
            start_ptr_slot,
        ));
        self.emit(abi::move_register(copy_src, start_ptr));
        self.emit(abi::add_immediate(copy_dst, abi::RET[1], 8));
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

        // Zero-stride for kind 2 throughout: `mid` neither reads nor writes a
        // lookup table for a fixed-width list (plan-57-D).
        let mid_payload = kind2_payload_size(element_type);
        let data_len_slot = self.allocate_stack_object("mid_list_data_len", 8);
        let disordered_slot = self.allocate_stack_object("mid_list_disordered", 8);
        let result_slot = self.allocate_stack_object("mid_list_result", 8);
        let mid_unordered = self.label("mid_list_unordered");
        let valid_start = self.label("mid_list_valid_start");
        let valid_count = self.label("mid_list_valid_count");
        let range_ok = self.label("mid_list_range_ok");
        let length_loop = self.label("mid_list_length_loop");
        let length_done = self.label("mid_list_length_done");
        let alloc_ok = self.label("mid_list_alloc_ok");
        let copy_done = self.label("mid_list_copy_done");
        let invalid_range = self.label("mid_list_invalid_range");
        let done = self.label("mid_list_done");

        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), start_slot));
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), count_slot));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch8,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::compare_immediate(&scratch9, "0"));
        self.emit(abi::branch_ge(&valid_start));
        self.emit(abi::branch(&invalid_range));
        self.emit(abi::label(&valid_start));
        self.emit(abi::compare_immediate(&scratch10, "0"));
        self.emit(abi::branch_ge(&valid_count));
        self.emit(abi::branch(&invalid_range));
        self.emit(abi::label(&valid_count));
        self.emit(abi::compare_registers(&scratch9, &scratch11));
        self.emit(abi::branch_gt(&invalid_range));
        self.emit(abi::add_registers(&scratch12, &scratch9, &scratch10));
        self.emit(abi::compare_registers(&scratch12, &scratch9));
        self.emit(abi::branch_lt(&invalid_range));
        self.emit(abi::compare_registers(&scratch12, &scratch11));
        self.emit(abi::branch_le(&range_ok));
        self.emit(abi::branch(&invalid_range));

        self.emit(abi::label(&range_ok));
        self.emit(abi::move_immediate(&scratch13, "Integer", "0"));
        self.emit(abi::store_u64(
            &scratch13,
            abi::stack_pointer(),
            data_len_slot,
        ));
        self.emit(abi::move_immediate(
            &scratch14,
            "Integer",
            &mid_payload.map_or(COLLECTION_ENTRY_SIZE, |_| 0).to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch15, &scratch9, &scratch14));
        self.emit(abi::add_immediate(
            &scratch16,
            &scratch8,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(&scratch16, &scratch16, &scratch15)); // src entry[start]

        // kind 2 needs neither the length loop nor the probe: `dataLength` is
        // `count * payloadSize` and the slice is contiguous and in order by
        // construction (plan-57-D). `scratch16` above is meaningless for it, and
        // is not read on this path.
        if let Some(payload) = mid_payload {
            self.emit(abi::move_immediate(&scratch14, "Integer", "0"));
            self.emit(abi::store_u64(
                &scratch14,
                abi::stack_pointer(),
                disordered_slot,
            ));
            self.emit(abi::move_immediate(
                &scratch14,
                "Integer",
                &payload.to_string(),
            ));
            self.emit(abi::multiply_registers(&scratch13, &scratch10, &scratch14));
            self.emit(abi::store_u64(
                &scratch13,
                abi::stack_pointer(),
                data_len_slot,
            ));
            self.emit(abi::branch(&length_done));
        }

        // Order/tightness probe folded into the length loop: `expected` (scratch11)
        // tracks where the next payload should begin if the slice is a contiguous
        // ordered span; any entry whose valueOffset differs sets `disordered_slot`.
        // A sorted `fs::listDirectory` result permutes entry records without moving
        // the data, so its slice is out of order and takes the per-entry fallback.
        let mid_ordered_ok = self.label("mid_list_ordered_ok");
        self.emit(abi::move_immediate(&scratch14, "Integer", "0"));
        self.emit(abi::store_u64(
            &scratch14,
            abi::stack_pointer(),
            disordered_slot,
        ));
        self.emit(abi::load_u64(
            &scratch11,
            &scratch16,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        )); // expected = entry[start].valueOffset

        self.emit(abi::label(&length_loop));
        self.emit(abi::compare_registers(&scratch13, &scratch10));
        self.emit(abi::branch_ge(&length_done));
        self.emit(abi::load_u64(
            &scratch17,
            &scratch16,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::load_u64(
            &scratch9,
            &scratch16,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        )); // vo
        self.emit(abi::compare_registers(&scratch9, &scratch11));
        self.emit(abi::branch_eq(&mid_ordered_ok));
        self.emit(abi::move_immediate(&scratch14, "Integer", "1"));
        self.emit(abi::store_u64(
            &scratch14,
            abi::stack_pointer(),
            disordered_slot,
        ));
        self.emit(abi::label(&mid_ordered_ok));
        self.emit(abi::add_registers(&scratch11, &scratch9, &scratch17)); // expected = vo + vl
        self.emit(abi::load_u64(
            &scratch20,
            abi::stack_pointer(),
            data_len_slot,
        ));
        self.emit(abi::add_registers(&scratch20, &scratch20, &scratch17));
        self.emit(abi::store_u64(
            &scratch20,
            abi::stack_pointer(),
            data_len_slot,
        ));
        self.emit(abi::add_immediate(
            &scratch16,
            &scratch16,
            COLLECTION_ENTRY_SIZE,
        ));
        self.emit(abi::add_immediate(&scratch13, &scratch13, 1));
        self.emit(abi::branch(&length_loop));

        self.emit(abi::label(&length_done));
        self.emit(abi::move_immediate(
            &scratch14,
            "Integer",
            &mid_payload.map_or(COLLECTION_ENTRY_SIZE, |_| 0).to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch15, &scratch10, &scratch14));
        self.emit(abi::load_u64(
            &scratch16,
            abi::stack_pointer(),
            data_len_slot,
        ));
        self.emit(abi::add_immediate(
            abi::return_register(),
            &scratch15,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(
            abi::return_register(),
            abi::return_register(),
            &scratch16,
        ));
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
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), count_slot));
        self.emit(abi::store_u64(
            &scratch10,
            abi::RET[1],
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::store_u64(
            &scratch10,
            abi::RET[1],
            COLLECTION_OFFSET_CAPACITY,
        ));
        self.emit(abi::load_u64(
            &scratch16,
            abi::stack_pointer(),
            data_len_slot,
        ));
        self.emit(abi::store_u64(
            &scratch16,
            abi::RET[1],
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::store_u64(
            &scratch16,
            abi::RET[1],
            COLLECTION_OFFSET_DATA_CAPACITY,
        ));

        self.emit(abi::load_u64(&scratch8, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), start_slot));
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), count_slot));
        self.emit(abi::load_u64(
            abi::RET[1],
            abi::stack_pointer(),
            result_slot,
        ));
        self.emit(abi::move_immediate(
            &scratch14,
            "Integer",
            &mid_payload.map_or(COLLECTION_ENTRY_SIZE, |_| 0).to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch15, &scratch9, &scratch14));
        self.emit(abi::add_immediate(
            &scratch16,
            &scratch8,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(&scratch16, &scratch16, &scratch15));
        self.emit(abi::add_immediate(
            &scratch17,
            abi::RET[1],
            COLLECTION_HEADER_SIZE,
        ));
        self.emit_collection_data_pointer_for(&scratch20, &scratch8, element_type);
        self.emit(abi::multiply_registers(&scratch21, &scratch10, &scratch14));
        self.emit(abi::add_registers(&scratch21, &scratch17, &scratch21));
        // Fast path — an in-order, gap-free slice (the probe found no disorder) is
        // a single contiguous data span: copy `dataLen` bytes from
        // `src.data + srcBaseOffset` (the first slice entry's valueOffset) with one
        // block copy, and copy the entry span verbatim with each valueOffset shifted
        // down by `srcBaseOffset` so the slice repacks from 0 (plan-25-B). A slice
        // that failed the probe (a permuted / gappy source) takes the per-entry
        // re-pack fallback, which reads each entry's own valueOffset.
        self.emit(abi::load_u64(
            &scratch13,
            abi::stack_pointer(),
            disordered_slot,
        ));
        self.emit(abi::compare_immediate(&scratch13, "0"));
        self.emit(abi::branch_ne(&mid_unordered));
        if let Some(payload) = mid_payload {
            // kind 2: srcBaseOffset is `start * payloadSize`. Reload `start` from
            // its slot — `_mfb_arena_alloc` above destroys every caller-saved
            // register, so nothing held across it can be trusted.
            self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), start_slot));
            self.emit(abi::move_immediate(
                &scratch13,
                "Integer",
                &payload.to_string(),
            ));
            self.emit(abi::multiply_registers(&scratch13, &scratch9, &scratch13));
        } else {
            self.emit(abi::load_u64(
                &scratch13,
                &scratch16,
                COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
            )); // srcBaseOffset
        }
        self.emit(abi::add_registers(&scratch20, &scratch20, &scratch13)); // src.data + srcBaseOffset
        self.emit(abi::load_u64(
            &scratch14,
            abi::stack_pointer(),
            data_len_slot,
        ));
        self.emit_block_copy_advance(
            &scratch21,
            &scratch20,
            &scratch14,
            &scratch22,
            "mid_list_data",
        );
        if mid_payload.is_none() {
            self.emit_bulk_copy_entries_shift(
                &scratch16,
                &scratch17,
                &scratch10,
                Some((&scratch13, true)),
                "mid_list_entries",
            );
        }
        self.emit(abi::branch(&copy_done));

        // Fallback: per-entry re-pack for an out-of-order / gappy source (scratch20
        // still holds the un-advanced src data base).
        self.emit(abi::label(&mid_unordered));
        self.emit(abi::move_immediate(&scratch13, "Integer", "0"));
        self.emit_copy_collection_entries(
            &scratch16,
            &scratch20,
            &scratch17,
            &scratch21,
            &scratch13,
            &scratch10,
            "mid_list_copy",
        )?;
        self.emit(abi::branch(&copy_done));

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
