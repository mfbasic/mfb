use super::*;

use super::builder_strings_package::UnicodeCaseMap;

impl CodeBuilder<'_> {
    pub(super) fn lower_strings_graphemes(&mut self, value: &NirValue) -> Result<ValueResult, String> {
        let scratch16 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        let scratch22 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch24 = self.temporary_vreg();
        let scratch25 = self.temporary_vreg();
        let scratch23 = self.temporary_vreg();
        let scratch15 = self.temporary_vreg();
        let scratch26 = self.temporary_vreg();
        let scratch27 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let scratch20 = self.temporary_vreg();
        let scratch21 = self.temporary_vreg();
        let scratch28 = self.temporary_vreg();
        let value = self.lower_value(value)?;
        self.require_string("strings.graphemes value", &value)?;
        let value_slot = self.store_string_pointer("strings_graphemes_value", &value.location);
        let count_slot = self.allocate_stack_object("strings_graphemes_count", 8);
        let state_bc_slot = self.allocate_stack_object("strings_graphemes_state_bc", 8);
        let state_icb_slot = self.allocate_stack_object("strings_graphemes_state_icb", 8);
        let result_slot = self.allocate_stack_object("strings_graphemes_result", 8);
        let layout = CollectionTypeLayout::from_type("List OF String").ok_or_else(|| {
            "native strings.graphemes cannot resolve List OF String layout".to_string()
        })?;

        let count_empty = self.label("strings_graphemes_count_empty");
        let count_loop = self.label("strings_graphemes_count_loop");
        let count_break = self.label("strings_graphemes_count_break");
        let count_no_break = self.label("strings_graphemes_count_no_break");
        let count_after_break = self.label("strings_graphemes_count_after_break");
        let count_done = self.label("strings_graphemes_count_done");
        let alloc_ok = self.label("strings_graphemes_alloc_ok");
        let write_empty = self.label("strings_graphemes_write_empty");
        let write_loop = self.label("strings_graphemes_write_loop");
        let write_break = self.label("strings_graphemes_write_break");
        let write_no_break = self.label("strings_graphemes_write_no_break");
        let write_after_break = self.label("strings_graphemes_write_after_break");
        let write_final = self.label("strings_graphemes_write_final");

        self.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(&scratch9, &scratch16, 0));
        self.emit(abi::compare_immediate(&scratch9, "0"));
        self.emit(abi::branch_eq(&count_empty));
        self.emit(abi::add_immediate(&scratch14, &scratch16, 8));
        self.emit(abi::move_immediate(&scratch22, "Integer", "1"));
        self.emit_utf8_decode_next(&scratch14, &scratch10, &scratch11);
        self.emit_unicode_property_lookup(&scratch10, &scratch12);
        self.emit_unicode_property_boundclass(&scratch12, &scratch24);
        self.emit_unicode_property_indic_conjunct_break(&scratch12, &scratch25);
        self.emit(abi::move_register(&scratch23, &scratch11));
        self.emit(abi::label(&count_loop));
        self.emit(abi::compare_registers(&scratch23, &scratch9));
        self.emit(abi::branch_ge(&count_done));
        self.emit(abi::add_registers(&scratch15, &scratch14, &scratch23));
        self.emit_utf8_decode_next(&scratch15, &scratch10, &scratch11);
        self.emit_unicode_property_lookup(&scratch10, &scratch12);
        self.emit_unicode_property_boundclass(&scratch12, &scratch26);
        self.emit_unicode_property_indic_conjunct_break(&scratch12, &scratch27);
        self.emit_grapheme_break_branch(&scratch24, &scratch25, &scratch26, &scratch27, &count_break, &count_no_break);
        self.emit(abi::label(&count_break));
        self.emit(abi::add_immediate(&scratch22, &scratch22, 1));
        self.emit(abi::branch(&count_after_break));
        self.emit(abi::label(&count_no_break));
        self.emit(abi::branch(&count_after_break));
        self.emit(abi::label(&count_after_break));
        self.emit_grapheme_state_update(&scratch24, &scratch25, &scratch26, &scratch27);
        self.emit(abi::add_registers(&scratch23, &scratch23, &scratch11));
        self.emit(abi::branch(&count_loop));
        self.emit(abi::label(&count_empty));
        self.emit(abi::move_immediate(&scratch22, "Integer", "0"));
        self.emit(abi::label(&count_done));
        self.emit(abi::store_u64(&scratch22, abi::stack_pointer(), count_slot));

        self.emit(abi::move_immediate(
            &scratch13,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch13, &scratch13, &scratch22));
        self.emit(abi::add_immediate(
            abi::return_register(),
            &scratch13,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(
            abi::return_register(),
            abi::return_register(),
            &scratch9,
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
        self.emit(abi::load_u64(&scratch11, abi::stack_pointer(), count_slot));
        self.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(&scratch9, &scratch16, 0));
        self.emit_write_list_header_from_registers(&layout, "x1", &scratch11, &scratch9);

        self.emit(abi::compare_immediate(&scratch9, "0"));
        self.emit(abi::branch_eq(&write_empty));
        self.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(&scratch9, &scratch16, 0));
        self.emit(abi::add_immediate(&scratch14, &scratch16, 8));
        self.emit(abi::load_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::add_immediate(&scratch20, "x1", COLLECTION_HEADER_SIZE));
        self.emit_collection_data_pointer(&scratch21, "x1");
        self.emit(abi::move_immediate(&scratch22, "Integer", "0"));
        self.emit(abi::move_immediate(&scratch24, "Integer", "0"));
        self.emit_utf8_decode_next(&scratch14, &scratch10, &scratch11);
        self.emit_unicode_property_lookup(&scratch10, &scratch12);
        self.emit_unicode_property_boundclass(&scratch12, &scratch25);
        self.emit_unicode_property_indic_conjunct_break(&scratch12, &scratch26);
        self.emit(abi::store_u64(&scratch25, abi::stack_pointer(), state_bc_slot));
        self.emit(abi::store_u64(&scratch26, abi::stack_pointer(), state_icb_slot));
        self.emit(abi::move_register(&scratch23, &scratch11));
        self.emit(abi::label(&write_loop));
        self.emit(abi::compare_registers(&scratch23, &scratch9));
        self.emit(abi::branch_ge(&write_final));
        self.emit(abi::add_registers(&scratch15, &scratch14, &scratch23));
        self.emit_utf8_decode_next(&scratch15, &scratch10, &scratch11);
        self.emit_unicode_property_lookup(&scratch10, &scratch12);
        self.emit_unicode_property_boundclass(&scratch12, &scratch27);
        self.emit_unicode_property_indic_conjunct_break(&scratch12, &scratch28);
        self.emit(abi::load_u64(&scratch25, abi::stack_pointer(), state_bc_slot));
        self.emit(abi::load_u64(&scratch26, abi::stack_pointer(), state_icb_slot));
        self.emit_grapheme_break_branch(&scratch25, &scratch26, &scratch27, &scratch28, &write_break, &write_no_break);
        self.emit(abi::label(&write_break));
        self.emit_grapheme_state_update(&scratch25, &scratch26, &scratch27, &scratch28);
        self.emit(abi::store_u64(&scratch25, abi::stack_pointer(), state_bc_slot));
        self.emit(abi::store_u64(&scratch26, abi::stack_pointer(), state_icb_slot));
        self.emit_string_split_write_entry(&scratch20, &scratch21, &scratch22, &scratch24, &scratch23, &scratch14)?;
        self.emit(abi::move_register(&scratch24, &scratch23));
        self.emit(abi::branch(&write_after_break));
        self.emit(abi::label(&write_no_break));
        self.emit_grapheme_state_update(&scratch25, &scratch26, &scratch27, &scratch28);
        self.emit(abi::store_u64(&scratch25, abi::stack_pointer(), state_bc_slot));
        self.emit(abi::store_u64(&scratch26, abi::stack_pointer(), state_icb_slot));
        self.emit(abi::branch(&write_after_break));
        self.emit(abi::label(&write_after_break));
        self.emit(abi::add_registers(&scratch23, &scratch23, &scratch11));
        self.emit(abi::branch(&write_loop));
        self.emit(abi::label(&write_final));
        self.emit_string_split_write_entry(&scratch20, &scratch21, &scratch22, &scratch24, &scratch9, &scratch14)?;
        self.emit(abi::label(&write_empty));

        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: "List OF String".to_string(),
            location: result,
            text: "strings.graphemes".to_string(),
        })
    }

    /// `strings.toBytes(value)` — the raw UTF-8 bytes backing `value`, as a
    /// `List OF Byte` (one element per byte). The inverse of
    /// `toString(List OF Byte)`. Builds the collection element-by-element so the
    /// per-element entry table and packed payload match the standard layout.
    pub(super) fn lower_strings_to_bytes(&mut self, value: &NirValue) -> Result<ValueResult, String> {
        let scratch16 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let scratch20 = self.temporary_vreg();
        let scratch21 = self.temporary_vreg();
        let scratch22 = self.temporary_vreg();
        let scratch23 = self.temporary_vreg();
        let scratch25 = self.temporary_vreg();
        let scratch24 = self.temporary_vreg();
        let scratch26 = self.temporary_vreg();
        let scratch27 = self.temporary_vreg();
        let scratch28 = self.temporary_vreg();
        let value = self.lower_value(value)?;
        self.require_string("strings.toBytes value", &value)?;
        let value_slot = self.store_string_pointer("strings_to_bytes_value", &value.location);
        let count_slot = self.allocate_stack_object("strings_to_bytes_count", 8);
        let result_slot = self.allocate_stack_object("strings_to_bytes_result", 8);
        let layout = CollectionTypeLayout::from_type("List OF Byte")
            .ok_or_else(|| "native strings.toBytes cannot resolve List OF Byte layout".to_string())?;

        let alloc_ok = self.label("strings_to_bytes_alloc_ok");
        let write_loop = self.label("strings_to_bytes_write_loop");
        let write_done = self.label("strings_to_bytes_write_done");

        // count = byteLen( [strptr + 0] ); spill across the allocation call.
        self.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(&scratch9, &scratch16, 0));
        self.emit(abi::store_u64(&scratch9, abi::stack_pointer(), count_slot));

        // alloc size = HEADER + count * (ENTRY_SIZE) + count (one payload byte each).
        self.emit(abi::move_immediate(
            &scratch13,
            "Integer",
            &(COLLECTION_ENTRY_SIZE + 1).to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch13, &scratch9, &scratch13));
        self.emit(abi::add_immediate(
            abi::return_register(),
            &scratch13,
            COLLECTION_HEADER_SIZE,
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
        self.emit(abi::compare_immediate(abi::return_register(), RESULT_OK_TAG));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&alloc_ok));
        // x1 holds the new collection pointer.
        self.emit(abi::store_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::move_register(&scratch20, "x1"));
        self.emit(abi::load_u64(&scratch9, abi::stack_pointer(), count_slot));
        // Header: count == capacity == dataLength == dataCapacity == count.
        self.emit_write_list_header_from_registers(&layout, &scratch20, &scratch9, &scratch9);

        // payload base = collection + HEADER + capacity * ENTRY_SIZE.
        self.emit(abi::move_immediate(
            &scratch13,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch13, &scratch9, &scratch13));
        self.emit(abi::add_immediate(&scratch21, &scratch20, COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers(&scratch21, &scratch21, &scratch13));

        // x22 = string data pointer (strptr + 8); x23 = i (0).
        self.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
        self.emit(abi::add_immediate(&scratch22, &scratch16, 8));
        self.emit(abi::move_immediate(&scratch23, "Integer", "0"));

        self.emit(abi::label(&write_loop));
        self.emit(abi::compare_registers(&scratch23, &scratch9));
        self.emit(abi::branch_ge(&write_done));
        // entry_addr (x24) = collection + HEADER + i * ENTRY_SIZE.
        self.emit(abi::move_immediate(
            &scratch25,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch25, &scratch23, &scratch25));
        self.emit(abi::add_immediate(&scratch24, &scratch20, COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers(&scratch24, &scratch24, &scratch25));
        // flags = USED; key offset/length = 0.
        self.emit(abi::move_immediate(
            &scratch26,
            "Byte",
            &COLLECTION_ENTRY_FLAG_USED.to_string(),
        ));
        self.emit(abi::store_u8(&scratch26, &scratch24, COLLECTION_ENTRY_OFFSET_FLAGS));
        self.emit(abi::store_u64("x31", &scratch24, COLLECTION_ENTRY_OFFSET_KEY_OFFSET));
        self.emit(abi::store_u64("x31", &scratch24, COLLECTION_ENTRY_OFFSET_KEY_LENGTH));
        // value offset = i; value length = 1.
        self.emit(abi::store_u64(&scratch23, &scratch24, COLLECTION_ENTRY_OFFSET_VALUE_OFFSET));
        self.emit(abi::move_immediate(&scratch26, "Integer", "1"));
        self.emit(abi::store_u64(&scratch26, &scratch24, COLLECTION_ENTRY_OFFSET_VALUE_LENGTH));
        // payload[i] = string byte[i].
        self.emit(abi::add_registers(&scratch27, &scratch22, &scratch23));
        self.emit(abi::load_u8(&scratch26, &scratch27, 0));
        self.emit(abi::add_registers(&scratch28, &scratch21, &scratch23));
        self.emit(abi::store_u8(&scratch26, &scratch28, 0));
        self.emit(abi::add_immediate(&scratch23, &scratch23, 1));
        self.emit(abi::branch(&write_loop));
        self.emit(abi::label(&write_done));

        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: "List OF Byte".to_string(),
            location: result,
            text: "strings.toBytes".to_string(),
        })
    }

    pub(super) fn lower_strings_case_map(
        &mut self,
        value: &NirValue,
        map: UnicodeCaseMap,
    ) -> Result<ValueResult, String> {
        let scratch20 = self.temporary_vreg();
        let scratch21 = self.temporary_vreg();
        let scratch22 = self.temporary_vreg();
        let scratch23 = self.temporary_vreg();
        let scratch24 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch26 = self.temporary_vreg();
        let scratch27 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let scratch28 = self.temporary_vreg();
        let value = self.lower_value(value)?;
        self.require_string(map.label(), &value)?;
        let value_slot = self.store_string_pointer(map.slot_prefix(), &value.location);
        let length_slot = self.allocate_stack_object("strings_case_map_length", 8);
        let width_slot = self.allocate_stack_object("strings_case_map_width", 8);
        let result_slot = self.allocate_stack_object("strings_case_map_result", 8);

        let count_loop = self.label("strings_case_map_count_loop");
        let count_identity = self.label("strings_case_map_count_identity");
        let count_sequence = self.label("strings_case_map_count_sequence");
        let count_sequence_loop = self.label("strings_case_map_count_sequence_loop");
        let count_next = self.label("strings_case_map_count_next");
        let count_done = self.label("strings_case_map_count_done");
        let alloc_ok = self.label("strings_case_map_alloc_ok");
        let write_loop = self.label("strings_case_map_write_loop");
        let write_identity = self.label("strings_case_map_write_identity");
        let write_sequence = self.label("strings_case_map_write_sequence");
        let write_sequence_loop = self.label("strings_case_map_write_sequence_loop");
        let write_next = self.label("strings_case_map_write_next");
        let write_done = self.label("strings_case_map_write_done");

        self.emit(abi::load_u64(&scratch20, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(&scratch21, &scratch20, 0));
        self.emit(abi::add_immediate(&scratch22, &scratch20, 8));
        self.emit(abi::move_immediate(&scratch23, "Integer", "0"));
        self.emit(abi::move_immediate(&scratch24, "Integer", "0"));
        self.emit(abi::label(&count_loop));
        self.emit(abi::compare_registers(&scratch23, &scratch21));
        self.emit(abi::branch_ge(&count_done));
        self.emit(abi::add_registers(&scratch14, &scratch22, &scratch23));
        self.emit_utf8_decode_next(&scratch14, &scratch10, &scratch11);
        self.emit(abi::store_u64(&scratch11, abi::stack_pointer(), width_slot));
        self.emit_case_map_lookup(map, &scratch10, &scratch26, &scratch27);
        self.emit(abi::compare_immediate(&scratch27, "0"));
        self.emit(abi::branch_eq(&count_identity));
        self.emit(abi::branch(&count_sequence));
        self.emit(abi::label(&count_identity));
        self.emit(abi::add_registers(&scratch24, &scratch24, &scratch11));
        self.emit(abi::branch(&count_next));
        self.emit(abi::label(&count_sequence));
        self.emit(abi::label(&count_sequence_loop));
        self.emit(abi::compare_immediate(&scratch27, "0"));
        self.emit(abi::branch_eq(&count_next));
        self.emit(abi::load_u32(&scratch10, &scratch26, 0));
        self.emit(abi::add_immediate(&scratch26, &scratch26, 4));
        self.emit_utf8_encoded_width(&scratch10, &scratch13);
        self.emit(abi::add_registers(&scratch24, &scratch24, &scratch13));
        self.emit(abi::subtract_immediate(&scratch27, &scratch27, 1));
        self.emit(abi::branch(&count_sequence_loop));
        self.emit(abi::label(&count_next));
        self.emit(abi::load_u64(&scratch11, abi::stack_pointer(), width_slot));
        self.emit(abi::add_registers(&scratch23, &scratch23, &scratch11));
        self.emit(abi::branch(&count_loop));
        self.emit(abi::label(&count_done));
        self.emit(abi::store_u64(&scratch24, abi::stack_pointer(), length_slot));

        self.emit(abi::add_immediate(abi::return_register(), &scratch24, 9));
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
        self.emit(abi::load_u64(&scratch24, abi::stack_pointer(), length_slot));
        self.emit(abi::store_u64(&scratch24, "x1", 0));

        self.emit(abi::load_u64(&scratch20, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(&scratch21, &scratch20, 0));
        self.emit(abi::add_immediate(&scratch22, &scratch20, 8));
        self.emit(abi::move_immediate(&scratch23, "Integer", "0"));
        self.emit(abi::load_u64(&scratch28, abi::stack_pointer(), result_slot));
        self.emit(abi::add_immediate(&scratch28, &scratch28, 8));
        self.emit(abi::label(&write_loop));
        self.emit(abi::compare_registers(&scratch23, &scratch21));
        self.emit(abi::branch_ge(&write_done));
        self.emit(abi::add_registers(&scratch14, &scratch22, &scratch23));
        self.emit_utf8_decode_next(&scratch14, &scratch10, &scratch11);
        self.emit(abi::store_u64(&scratch11, abi::stack_pointer(), width_slot));
        self.emit_case_map_lookup(map, &scratch10, &scratch26, &scratch27);
        self.emit(abi::compare_immediate(&scratch27, "0"));
        self.emit(abi::branch_eq(&write_identity));
        self.emit(abi::branch(&write_sequence));
        self.emit(abi::label(&write_identity));
        self.emit_utf8_encode_next(&scratch28, &scratch10);
        self.emit(abi::branch(&write_next));
        self.emit(abi::label(&write_sequence));
        self.emit(abi::label(&write_sequence_loop));
        self.emit(abi::compare_immediate(&scratch27, "0"));
        self.emit(abi::branch_eq(&write_next));
        self.emit(abi::load_u32(&scratch10, &scratch26, 0));
        self.emit(abi::add_immediate(&scratch26, &scratch26, 4));
        self.emit_utf8_encode_next(&scratch28, &scratch10);
        self.emit(abi::subtract_immediate(&scratch27, &scratch27, 1));
        self.emit(abi::branch(&write_sequence_loop));
        self.emit(abi::label(&write_next));
        self.emit(abi::load_u64(&scratch11, abi::stack_pointer(), width_slot));
        self.emit(abi::add_registers(&scratch23, &scratch23, &scratch11));
        self.emit(abi::branch(&write_loop));
        self.emit(abi::label(&write_done));
        self.emit(abi::move_immediate(&scratch10, "Integer", "0"));
        self.emit(abi::store_u8(&scratch10, &scratch28, 0));

        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: "String".to_string(),
            location: result,
            text: map.name().to_string(),
        })
    }

    pub(super) fn lower_strings_normalize_nfc(&mut self, value: &NirValue) -> Result<ValueResult, String> {
        let scratch20 = self.temporary_vreg();
        let scratch21 = self.temporary_vreg();
        let scratch22 = self.temporary_vreg();
        let scratch23 = self.temporary_vreg();
        let scratch24 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch26 = self.temporary_vreg();
        let scratch27 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let scratch25 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch15 = self.temporary_vreg();
        let scratch28 = self.temporary_vreg();
        let scratch16 = self.temporary_vreg();
        let scratch17 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch8 = self.temporary_vreg();
        let value = self.lower_value(value)?;
        self.require_string("strings.normalizeNfc value", &value)?;
        let value_slot = self.store_string_pointer("strings_normalize_nfc_value", &value.location);
        let scalar_count_slot = self.allocate_stack_object("strings_normalize_nfc_scalar_count", 8);
        let temp_slot = self.allocate_stack_object("strings_normalize_nfc_temp", 8);
        let composed_count_slot =
            self.allocate_stack_object("strings_normalize_nfc_composed_count", 8);
        let output_len_slot = self.allocate_stack_object("strings_normalize_nfc_output_len", 8);
        let width_slot = self.allocate_stack_object("strings_normalize_nfc_width", 8);
        let result_slot = self.allocate_stack_object("strings_normalize_nfc_result", 8);

        let count_loop = self.label("strings_nfc_count_loop");
        let count_identity = self.label("strings_nfc_count_identity");
        let count_next = self.label("strings_nfc_count_next");
        let count_done = self.label("strings_nfc_count_done");
        let temp_alloc_ok = self.label("strings_nfc_temp_alloc_ok");
        let fill_loop = self.label("strings_nfc_fill_loop");
        let fill_identity = self.label("strings_nfc_fill_identity");
        let fill_sequence_loop = self.label("strings_nfc_fill_sequence_loop");
        let fill_store = self.label("strings_nfc_fill_store");
        let fill_next = self.label("strings_nfc_fill_next");
        let fill_done = self.label("strings_nfc_fill_done");
        let order_loop = self.label("strings_nfc_order_loop");
        let order_done = self.label("strings_nfc_order_done");
        let order_no_swap = self.label("strings_nfc_order_no_swap");
        let order_swap = self.label("strings_nfc_order_swap");
        let order_decrement = self.label("strings_nfc_order_decrement");
        let compose_loop = self.label("strings_nfc_compose_loop");
        let compose_write = self.label("strings_nfc_compose_write");
        let compose_try = self.label("strings_nfc_compose_try");
        let compose_try_tables = self.label("strings_nfc_compose_try_tables");
        let compose_scan_loop = self.label("strings_nfc_compose_scan_loop");
        let compose_found = self.label("strings_nfc_compose_found");
        let compose_found_direct = self.label("strings_nfc_compose_found_direct");
        let compose_next = self.label("strings_nfc_compose_next");
        let compose_no_starter = self.label("strings_nfc_compose_no_starter");
        let compose_nonstarter = self.label("strings_nfc_compose_nonstarter");
        let compose_nonstarter_update = self.label("strings_nfc_compose_nonstarter_update");
        let compose_nonstarter_done = self.label("strings_nfc_compose_nonstarter_done");
        let byte_len_loop = self.label("strings_nfc_byte_len_loop");
        let byte_len_done = self.label("strings_nfc_byte_len_done");
        let result_alloc_ok = self.label("strings_nfc_result_alloc_ok");
        let encode_loop = self.label("strings_nfc_encode_loop");
        let encode_done = self.label("strings_nfc_encode_done");

        self.emit(abi::load_u64(&scratch20, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(&scratch21, &scratch20, 0));
        self.emit(abi::add_immediate(&scratch22, &scratch20, 8));
        self.emit(abi::move_immediate(&scratch23, "Integer", "0"));
        self.emit(abi::move_immediate(&scratch24, "Integer", "0"));
        self.emit(abi::label(&count_loop));
        self.emit(abi::compare_registers(&scratch23, &scratch21));
        self.emit(abi::branch_ge(&count_done));
        self.emit(abi::add_registers(&scratch14, &scratch22, &scratch23));
        self.emit_utf8_decode_next(&scratch14, &scratch10, &scratch11);
        self.emit(abi::store_u64(&scratch11, abi::stack_pointer(), width_slot));
        self.emit_unicode_u32_mapping_lookup(
            &scratch10,
            UNICODE_NFD_ENTRIES_SYMBOL,
            crate::unicode_runtime_tables::tables().nfd_entries.len(),
            UNICODE_NFD_SEQUENCES_SYMBOL,
            &scratch26,
            &scratch27,
        );
        self.emit(abi::compare_immediate(&scratch27, "0"));
        self.emit(abi::branch_eq(&count_identity));
        self.emit(abi::add_registers(&scratch24, &scratch24, &scratch27));
        self.emit(abi::branch(&count_next));
        self.emit(abi::label(&count_identity));
        self.emit(abi::add_immediate(&scratch24, &scratch24, 1));
        self.emit(abi::label(&count_next));
        self.emit(abi::load_u64(&scratch11, abi::stack_pointer(), width_slot));
        self.emit(abi::add_registers(&scratch23, &scratch23, &scratch11));
        self.emit(abi::branch(&count_loop));
        self.emit(abi::label(&count_done));
        self.emit(abi::store_u64(
            &scratch24,
            abi::stack_pointer(),
            scalar_count_slot,
        ));

        self.emit(abi::move_immediate(&scratch13, "Integer", "8"));
        self.emit(abi::multiply_registers(
            abi::return_register(),
            &scratch24,
            &scratch13,
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
        self.emit(abi::branch_eq(&temp_alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&temp_alloc_ok));
        self.emit(abi::store_u64("x1", abi::stack_pointer(), temp_slot));

        self.emit(abi::load_u64(&scratch20, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(&scratch21, &scratch20, 0));
        self.emit(abi::add_immediate(&scratch22, &scratch20, 8));
        self.emit(abi::load_u64(&scratch25, abi::stack_pointer(), temp_slot));
        self.emit(abi::move_immediate(&scratch23, "Integer", "0"));
        self.emit(abi::move_immediate(&scratch24, "Integer", "0"));
        self.emit(abi::label(&fill_loop));
        self.emit(abi::compare_registers(&scratch23, &scratch21));
        self.emit(abi::branch_ge(&fill_done));
        self.emit(abi::add_registers(&scratch14, &scratch22, &scratch23));
        self.emit_utf8_decode_next(&scratch14, &scratch10, &scratch11);
        self.emit(abi::store_u64(&scratch11, abi::stack_pointer(), width_slot));
        self.emit_unicode_u32_mapping_lookup(
            &scratch10,
            UNICODE_NFD_ENTRIES_SYMBOL,
            crate::unicode_runtime_tables::tables().nfd_entries.len(),
            UNICODE_NFD_SEQUENCES_SYMBOL,
            &scratch26,
            &scratch27,
        );
        self.emit(abi::compare_immediate(&scratch27, "0"));
        self.emit(abi::branch_eq(&fill_identity));
        self.emit(abi::label(&fill_sequence_loop));
        self.emit(abi::compare_immediate(&scratch27, "0"));
        self.emit(abi::branch_eq(&fill_next));
        self.emit(abi::load_u32(&scratch10, &scratch26, 0));
        self.emit(abi::add_immediate(&scratch26, &scratch26, 4));
        self.emit(abi::branch(&fill_store));
        self.emit(abi::label(&fill_identity));
        self.emit(abi::label(&fill_store));
        self.emit(abi::shift_left_immediate(&scratch12, &scratch24, 3));
        self.emit(abi::add_registers(&scratch12, &scratch25, &scratch12));
        self.emit(abi::store_u64(&scratch10, &scratch12, 0));
        self.emit(abi::add_immediate(&scratch24, &scratch24, 1));
        self.emit(abi::compare_immediate(&scratch27, "0"));
        self.emit(abi::branch_eq(&fill_next));
        self.emit(abi::subtract_immediate(&scratch27, &scratch27, 1));
        self.emit(abi::branch(&fill_sequence_loop));
        self.emit(abi::label(&fill_next));
        self.emit(abi::load_u64(&scratch11, abi::stack_pointer(), width_slot));
        self.emit(abi::add_registers(&scratch23, &scratch23, &scratch11));
        self.emit(abi::branch(&fill_loop));
        self.emit(abi::label(&fill_done));

        self.emit(abi::load_u64(&scratch25, abi::stack_pointer(), temp_slot));
        self.emit(abi::load_u64(
            &scratch21,
            abi::stack_pointer(),
            scalar_count_slot,
        ));
        self.emit(abi::move_immediate(&scratch23, "Integer", "0"));
        self.emit(abi::label(&order_loop));
        // x12 (dead at the loop head; redefined below) — not x6: ABI registers
        // stay physical through vregify, and the x86 remap role-colors them
        // (x6/x7 both collapsed onto rax, corrupting the scan pointers).
        self.emit(abi::add_immediate(&scratch12, &scratch23, 1));
        self.emit(abi::compare_registers(&scratch12, &scratch21));
        self.emit(abi::branch_ge(&order_done));
        self.emit(abi::shift_left_immediate(&scratch12, &scratch23, 3));
        self.emit(abi::add_registers(&scratch12, &scratch25, &scratch12));
        self.emit(abi::load_u64(&scratch10, &scratch12, 0));
        self.emit(abi::load_u64(&scratch11, &scratch12, 8));
        self.emit_unicode_property_lookup(&scratch10, &scratch13);
        self.emit_unicode_property_combining_class(&scratch13, &scratch14);
        self.emit_unicode_property_lookup(&scratch11, &scratch13);
        self.emit_unicode_property_combining_class(&scratch13, &scratch15);
        self.emit(abi::compare_immediate(&scratch15, "0"));
        self.emit(abi::branch_eq(&order_no_swap));
        self.emit(abi::compare_registers(&scratch14, &scratch15));
        self.emit(abi::branch_hi(&order_swap));
        self.emit(abi::branch(&order_no_swap));
        self.emit(abi::label(&order_swap));
        self.emit(abi::store_u64(&scratch11, &scratch12, 0));
        self.emit(abi::store_u64(&scratch10, &scratch12, 8));
        self.emit(abi::compare_immediate(&scratch23, "0"));
        self.emit(abi::branch_gt(&order_decrement));
        self.emit(abi::add_immediate(&scratch23, &scratch23, 1));
        self.emit(abi::branch(&order_loop));
        self.emit(abi::label(&order_decrement));
        self.emit(abi::subtract_immediate(&scratch23, &scratch23, 1));
        self.emit(abi::branch(&order_loop));
        self.emit(abi::label(&order_no_swap));
        self.emit(abi::add_immediate(&scratch23, &scratch23, 1));
        self.emit(abi::branch(&order_loop));
        self.emit(abi::label(&order_done));

        self.emit(abi::load_u64(&scratch25, abi::stack_pointer(), temp_slot));
        self.emit(abi::load_u64(
            &scratch21,
            abi::stack_pointer(),
            scalar_count_slot,
        ));
        self.emit(abi::move_immediate(&scratch23, "Integer", "0"));
        self.emit(abi::move_immediate(&scratch24, "Integer", "0"));
        self.emit(abi::move_immediate(&scratch26, "Integer", "0"));
        self.emit(abi::move_immediate(&scratch27, "Integer", "0"));
        self.emit(abi::move_immediate(&scratch28, "Integer", "0"));
        self.emit(abi::label(&compose_loop));
        self.emit(abi::compare_registers(&scratch23, &scratch21));
        self.emit(abi::branch_ge(&compose_next));
        self.emit(abi::shift_left_immediate(&scratch12, &scratch23, 3));
        self.emit(abi::add_registers(&scratch12, &scratch25, &scratch12));
        self.emit(abi::load_u64(&scratch10, &scratch12, 0));
        self.emit_unicode_property_lookup(&scratch10, &scratch13);
        self.emit_unicode_property_combining_class(&scratch13, &scratch15);
        self.emit(abi::compare_immediate(&scratch26, "0"));
        self.emit(abi::branch_eq(&compose_no_starter));
        self.emit(abi::compare_immediate(&scratch15, "0"));
        self.emit(abi::branch_eq(&compose_try));
        self.emit(abi::compare_registers(&scratch15, &scratch28));
        self.emit(abi::branch_hi(&compose_try));
        self.emit(abi::branch(&compose_write));
        self.emit(abi::label(&compose_try));
        self.emit(abi::shift_left_immediate(&scratch12, &scratch27, 3));
        self.emit(abi::add_registers(&scratch12, &scratch25, &scratch12));
        self.emit(abi::load_u64(&scratch11, &scratch12, 0));
        self.emit_hangul_composition_attempt(
            &scratch11,
            &scratch10,
            &scratch14,
            &compose_found_direct,
            &compose_try_tables,
        );
        self.emit(abi::label(&compose_try_tables));
        self.emit_unicode_property_lookup(&scratch11, &scratch13);
        self.emit_unicode_property_comb_index(&scratch13, &scratch16);
        self.emit_unicode_property_comb_length(&scratch13, &scratch17);
        self.emit_unicode_property_lookup(&scratch10, &scratch13);
        self.emit_unicode_property_flags(&scratch13, &scratch9);
        // x13/x9 are dead here (both consumed by the property extraction just
        // above); use them — not x6/x7: ABI registers stay physical through
        // vregify and the x86 remap role-colors them (x6 and x7 both collapsed
        // onto rax, so the scan pointer lost its table base).
        self.emit(abi::move_immediate(&scratch13, "Integer", "1023"));
        self.emit(abi::compare_registers(&scratch16, &scratch13));
        self.emit(abi::branch_ge(&compose_write));
        self.emit(abi::move_immediate(&scratch13, "Integer", "1"));
        self.emit(abi::and_registers(&scratch9, &scratch9, &scratch13));
        self.emit(abi::compare_immediate(&scratch9, "0"));
        self.emit(abi::branch_eq(&compose_write));
        self.emit_load_data_address(&scratch13, UNICODE_COMBINATIONS_SECOND_SYMBOL);
        self.emit(abi::shift_left_immediate(&scratch9, &scratch16, 2));
        self.emit(abi::add_registers(&scratch13, &scratch13, &scratch9));
        self.emit_load_data_address(&scratch8, UNICODE_COMBINATIONS_COMBINED_SYMBOL);
        self.emit(abi::add_registers(&scratch8, &scratch8, &scratch9));
        self.emit(abi::label(&compose_scan_loop));
        self.emit(abi::compare_immediate(&scratch17, "0"));
        self.emit(abi::branch_eq(&compose_write));
        self.emit(abi::load_u32(&scratch14, &scratch13, 0));
        self.emit(abi::compare_registers(&scratch14, &scratch10));
        self.emit(abi::branch_eq(&compose_found));
        self.emit(abi::branch_hi(&compose_write));
        self.emit(abi::add_immediate(&scratch13, &scratch13, 4));
        self.emit(abi::add_immediate(&scratch8, &scratch8, 4));
        self.emit(abi::subtract_immediate(&scratch17, &scratch17, 1));
        self.emit(abi::branch(&compose_scan_loop));
        self.emit(abi::label(&compose_found));
        self.emit(abi::load_u32(&scratch14, &scratch8, 0));
        self.emit(abi::label(&compose_found_direct));
        self.emit(abi::shift_left_immediate(&scratch12, &scratch27, 3));
        self.emit(abi::add_registers(&scratch12, &scratch25, &scratch12));
        self.emit(abi::store_u64(&scratch14, &scratch12, 0));
        self.emit(abi::add_immediate(&scratch23, &scratch23, 1));
        self.emit(abi::branch(&compose_loop));
        self.emit(abi::label(&compose_no_starter));
        self.emit(abi::label(&compose_write));
        self.emit(abi::shift_left_immediate(&scratch12, &scratch24, 3));
        self.emit(abi::add_registers(&scratch12, &scratch25, &scratch12));
        self.emit(abi::store_u64(&scratch10, &scratch12, 0));
        self.emit(abi::compare_immediate(&scratch15, "0"));
        self.emit(abi::branch_ne(&compose_nonstarter));
        self.emit(abi::move_immediate(&scratch26, "Integer", "1"));
        self.emit(abi::move_register(&scratch27, &scratch24));
        self.emit(abi::move_immediate(&scratch28, "Integer", "0"));
        self.emit(abi::branch(&compose_nonstarter_done));
        self.emit(abi::label(&compose_nonstarter));
        self.emit(abi::compare_registers(&scratch15, &scratch28));
        self.emit(abi::branch_hi(&compose_nonstarter_update));
        self.emit(abi::branch(&compose_nonstarter_done));
        self.emit(abi::label(&compose_nonstarter_update));
        self.emit(abi::move_register(&scratch28, &scratch15));
        self.emit(abi::label(&compose_nonstarter_done));
        self.emit(abi::add_immediate(&scratch24, &scratch24, 1));
        self.emit(abi::add_immediate(&scratch23, &scratch23, 1));
        self.emit(abi::branch(&compose_loop));
        self.emit(abi::label(&compose_next));
        self.emit(abi::store_u64(
            &scratch24,
            abi::stack_pointer(),
            composed_count_slot,
        ));

        self.emit(abi::move_immediate(&scratch23, "Integer", "0"));
        self.emit(abi::move_immediate(&scratch24, "Integer", "0"));
        self.emit(abi::label(&byte_len_loop));
        self.emit(abi::load_u64(
            &scratch21,
            abi::stack_pointer(),
            composed_count_slot,
        ));
        self.emit(abi::compare_registers(&scratch23, &scratch21));
        self.emit(abi::branch_ge(&byte_len_done));
        self.emit(abi::shift_left_immediate(&scratch12, &scratch23, 3));
        self.emit(abi::load_u64(&scratch25, abi::stack_pointer(), temp_slot));
        self.emit(abi::add_registers(&scratch12, &scratch25, &scratch12));
        self.emit(abi::load_u64(&scratch10, &scratch12, 0));
        self.emit_utf8_encoded_width(&scratch10, &scratch11);
        self.emit(abi::add_registers(&scratch24, &scratch24, &scratch11));
        self.emit(abi::add_immediate(&scratch23, &scratch23, 1));
        self.emit(abi::branch(&byte_len_loop));
        self.emit(abi::label(&byte_len_done));
        self.emit(abi::store_u64(&scratch24, abi::stack_pointer(), output_len_slot));

        self.emit(abi::add_immediate(abi::return_register(), &scratch24, 9));
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
        self.emit(abi::branch_eq(&result_alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&result_alloc_ok));
        self.emit(abi::store_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64(&scratch24, abi::stack_pointer(), output_len_slot));
        self.emit(abi::store_u64(&scratch24, "x1", 0));
        self.emit(abi::add_immediate(&scratch28, "x1", 8));
        self.emit(abi::move_immediate(&scratch23, "Integer", "0"));
        self.emit(abi::label(&encode_loop));
        self.emit(abi::load_u64(
            &scratch21,
            abi::stack_pointer(),
            composed_count_slot,
        ));
        self.emit(abi::compare_registers(&scratch23, &scratch21));
        self.emit(abi::branch_ge(&encode_done));
        self.emit(abi::shift_left_immediate(&scratch12, &scratch23, 3));
        self.emit(abi::load_u64(&scratch25, abi::stack_pointer(), temp_slot));
        self.emit(abi::add_registers(&scratch12, &scratch25, &scratch12));
        self.emit(abi::load_u64(&scratch10, &scratch12, 0));
        self.emit_utf8_encode_next(&scratch28, &scratch10);
        self.emit(abi::add_immediate(&scratch23, &scratch23, 1));
        self.emit(abi::branch(&encode_loop));
        self.emit(abi::label(&encode_done));
        self.emit(abi::move_immediate(&scratch10, "Integer", "0"));
        self.emit(abi::store_u8(&scratch10, &scratch28, 0));

        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: "String".to_string(),
            location: result,
            text: "strings.normalizeNfc".to_string(),
        })
    }

    pub(super) fn lower_strings_trim(
        &mut self,
        value: &NirValue,
        trim_start: bool,
        trim_end: bool,
    ) -> Result<ValueResult, String> {
        let scratch16 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        let scratch15 = self.temporary_vreg();
        let value = self.lower_value(value)?;
        self.require_string("strings.trim value", &value)?;
        let value_slot = self.store_string_pointer("strings_trim_value", &value.location);
        let start_slot = self.allocate_stack_object("strings_trim_start", 8);
        let end_slot = self.allocate_stack_object("strings_trim_end", 8);
        let done_start = self.label("strings_trim_start_done");

        self.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(&scratch9, &scratch16, 0));
        self.emit(abi::move_immediate(&scratch10, "Integer", "0"));
        self.emit(abi::store_u64(&scratch10, abi::stack_pointer(), start_slot));
        self.emit(abi::store_u64(&scratch9, abi::stack_pointer(), end_slot));

        if trim_start {
            let loop_label = self.label("strings_trim_start_loop");
            let ws_label = self.label("strings_trim_start_ws");
            self.emit(abi::add_immediate(&scratch11, &scratch16, 8));
            self.emit(abi::move_register(&scratch12, &scratch9));
            self.emit(abi::label(&loop_label));
            self.emit(abi::compare_immediate(&scratch12, "0"));
            self.emit(abi::branch_eq(&done_start));
            self.emit_unicode_whitespace_branch(&scratch11, &scratch12, &scratch13, &ws_label, &done_start);
            self.emit(abi::label(&ws_label));
            self.emit(abi::load_u64(&scratch14, abi::stack_pointer(), start_slot));
            self.emit(abi::add_registers(&scratch14, &scratch14, &scratch13));
            self.emit(abi::store_u64(&scratch14, abi::stack_pointer(), start_slot));
            self.emit(abi::add_registers(&scratch11, &scratch11, &scratch13));
            self.emit(abi::subtract_registers(&scratch12, &scratch12, &scratch13));
            self.emit(abi::branch(&loop_label));
        }
        self.emit(abi::label(&done_start));

        if trim_end {
            let loop_label = self.label("strings_trim_end_loop");
            let ws_label = self.label("strings_trim_end_ws");
            let not_ws_label = self.label("strings_trim_end_not_ws");
            let done_label = self.label("strings_trim_end_done");
            self.emit(abi::load_u64(&scratch14, abi::stack_pointer(), start_slot));
            self.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
            self.emit(abi::load_u64(&scratch9, &scratch16, 0));
            self.emit(abi::add_immediate(&scratch11, &scratch16, 8));
            self.emit(abi::add_registers(&scratch11, &scratch11, &scratch14));
            self.emit(abi::subtract_registers(&scratch12, &scratch9, &scratch14));
            self.emit(abi::move_register(&scratch15, &scratch14));
            self.emit(abi::store_u64(&scratch14, abi::stack_pointer(), end_slot));
            self.emit(abi::label(&loop_label));
            self.emit(abi::compare_immediate(&scratch12, "0"));
            self.emit(abi::branch_eq(&done_label));
            self.emit_unicode_whitespace_branch(&scratch11, &scratch12, &scratch13, &ws_label, &not_ws_label);
            self.emit(abi::label(&ws_label));
            self.emit(abi::add_registers(&scratch11, &scratch11, &scratch13));
            self.emit(abi::add_registers(&scratch15, &scratch15, &scratch13));
            self.emit(abi::subtract_registers(&scratch12, &scratch12, &scratch13));
            self.emit(abi::branch(&loop_label));
            self.emit(abi::label(&not_ws_label));
            self.emit(abi::add_immediate(&scratch11, &scratch11, 1));
            self.emit(abi::add_immediate(&scratch15, &scratch15, 1));
            self.emit(abi::subtract_immediate(&scratch12, &scratch12, 1));
            self.emit(abi::store_u64(&scratch15, abi::stack_pointer(), end_slot));
            self.emit(abi::branch(&loop_label));
            self.emit(abi::label(&done_label));
        }

        self.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), start_slot));
        self.emit(abi::load_u64(&scratch11, abi::stack_pointer(), end_slot));
        self.emit(abi::subtract_registers(&scratch12, &scratch11, &scratch10));
        self.emit(abi::add_immediate(&scratch13, &scratch16, 8));
        self.emit(abi::add_registers(&scratch13, &scratch13, &scratch10));
        let result = self.emit_materialize_string_from_bytes(&scratch13, &scratch12)?;
        Ok(ValueResult {
            type_: "String".to_string(),
            location: result,
            text: "strings.trim".to_string(),
        })
    }

    pub(super) fn lower_strings_byte_len(&mut self, value: &NirValue) -> Result<ValueResult, String> {
        let value = self.lower_value(value)?;
        self.require_string("strings.byteLen value", &value)?;
        let register = self.allocate_register()?;
        self.emit(abi::load_u64(&register, &value.location, 0));
        Ok(ValueResult {
            type_: "Integer".to_string(),
            location: register,
            text: format!("strings.byteLen({})", value.text),
        })
    }

    pub(super) fn lower_strings_starts_with(
        &mut self,
        value: &NirValue,
        prefix: &NirValue,
    ) -> Result<ValueResult, String> {
        let value = self.lower_value(value)?;
        self.require_string("strings.startsWith value", &value)?;
        let value_slot = self.store_string_pointer("strings_starts_with_value", &value.location);
        let prefix = self.lower_value(prefix)?;
        self.require_string("strings.startsWith prefix", &prefix)?;
        let prefix_slot = self.store_string_pointer("strings_starts_with_prefix", &prefix.location);
        self.lower_string_prefix_predicate("strings.startsWith", value_slot, prefix_slot, false)
    }

    pub(super) fn lower_strings_ends_with(
        &mut self,
        value: &NirValue,
        suffix: &NirValue,
    ) -> Result<ValueResult, String> {
        let value = self.lower_value(value)?;
        self.require_string("strings.endsWith value", &value)?;
        let value_slot = self.store_string_pointer("strings_ends_with_value", &value.location);
        let suffix = self.lower_value(suffix)?;
        self.require_string("strings.endsWith suffix", &suffix)?;
        let suffix_slot = self.store_string_pointer("strings_ends_with_suffix", &suffix.location);
        self.lower_string_prefix_predicate("strings.endsWith", value_slot, suffix_slot, true)
    }

    pub(super) fn lower_strings_contains(
        &mut self,
        value: &NirValue,
        needle: &NirValue,
    ) -> Result<ValueResult, String> {
        let scratch16 = self.temporary_vreg();
        let scratch17 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        let scratch15 = self.temporary_vreg();
        let value = self.lower_value(value)?;
        self.require_string("strings.contains value", &value)?;
        let value_slot = self.store_string_pointer("strings_contains_value", &value.location);
        let needle = self.lower_value(needle)?;
        self.require_string("strings.contains needle", &needle)?;
        let needle_slot = self.store_string_pointer("strings_contains_needle", &needle.location);

        let result_slot = self.allocate_stack_object("strings_contains_result", 8);
        let true_label = self.label("strings_contains_true");
        let false_label = self.label("strings_contains_false");
        let done_label = self.label("strings_contains_done");
        let loop_label = self.label("strings_contains_loop");
        let no_match_label = self.label("strings_contains_no_match");

        self.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(&scratch17, abi::stack_pointer(), needle_slot));
        self.emit(abi::load_u64(&scratch9, &scratch16, 0));
        self.emit(abi::load_u64(&scratch10, &scratch17, 0));
        self.emit(abi::compare_immediate(&scratch10, "0"));
        self.emit(abi::branch_eq(&true_label));
        self.emit(abi::compare_registers(&scratch10, &scratch9));
        self.emit(abi::branch_hi(&false_label));
        self.emit(abi::add_immediate(&scratch11, &scratch16, 8));
        self.emit(abi::add_immediate(&scratch12, &scratch17, 8));
        self.emit(abi::subtract_registers(&scratch13, &scratch9, &scratch10));
        self.emit(abi::move_immediate(&scratch14, "Integer", "0"));
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_registers(&scratch14, &scratch13));
        self.emit(abi::branch_hi(&false_label));
        self.emit(abi::add_registers(&scratch15, &scratch11, &scratch14));
        self.emit_string_byte_range_equal_branch(&scratch15, &scratch12, &scratch10, &true_label, &no_match_label);
        self.emit(abi::label(&no_match_label));
        self.emit(abi::add_immediate(&scratch14, &scratch14, 1));
        self.emit(abi::branch(&loop_label));
        self.emit_string_predicate_result(result_slot, &true_label, &false_label, &done_label);
        self.finish_string_predicate_result("strings.contains", result_slot)
    }

    pub(super) fn lower_strings_join(
        &mut self,
        parts: &NirValue,
        delimiter: &NirValue,
    ) -> Result<ValueResult, String> {
        let scratch16 = self.temporary_vreg();
        let scratch17 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        let scratch15 = self.temporary_vreg();
        let parts = self.lower_value(parts)?;
        if list_element_type(&parts.type_).as_deref() != Some("String") {
            return Err(format!(
                "strings.join parts must be List OF String, got {}",
                parts.type_
            ));
        }
        let parts_slot = self.store_string_pointer("strings_join_parts", &parts.location);
        let delimiter = self.lower_value(delimiter)?;
        self.require_string("strings.join delimiter", &delimiter)?;
        let delimiter_slot =
            self.store_string_pointer("strings_join_delimiter", &delimiter.location);
        let output_len_slot = self.allocate_stack_object("strings_join_output_len", 8);
        let result_slot = self.allocate_stack_object("strings_join_result", 8);
        let length_loop = self.label("strings_join_length_loop");
        let length_no_delim = self.label("strings_join_length_no_delim");
        let length_done = self.label("strings_join_length_done");
        let alloc_ok = self.label("strings_join_alloc_ok");
        let copy_loop = self.label("strings_join_copy_loop");
        let copy_no_delim = self.label("strings_join_copy_no_delim");
        let delim_loop = self.label("strings_join_delim_loop");
        let delim_done = self.label("strings_join_delim_done");
        let value_loop = self.label("strings_join_value_loop");
        let value_done = self.label("strings_join_value_done");
        let copy_done = self.label("strings_join_copy_done");

        // Copy-loop scratch as vregs (was out-of-pool x2/x3/x4, which fall back to
        // rax via the Ret-role default on x86 and collide). x9-x17 stay (vregify
        // pool). AArch64 unaffected.
        let cursor_v = self.temporary_vreg();
        let remaining_v = self.temporary_vreg();
        let byte_v = self.temporary_vreg();
        let cursor = cursor_v.as_str();
        let remaining = remaining_v.as_str();
        let byte = byte_v.as_str();

        self.emit(abi::load_u64(&scratch16, abi::stack_pointer(), parts_slot));
        self.emit(abi::load_u64(&scratch17, abi::stack_pointer(), delimiter_slot));
        self.emit(abi::load_u64(&scratch9, &scratch16, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(&scratch10, &scratch17, 0));
        self.emit(abi::move_immediate(&scratch11, "Integer", "0"));
        self.emit(abi::move_immediate(&scratch12, "Integer", "0"));
        self.emit(abi::add_immediate(&scratch13, &scratch16, COLLECTION_HEADER_SIZE));
        self.emit(abi::label(&length_loop));
        self.emit(abi::compare_registers(&scratch12, &scratch9));
        self.emit(abi::branch_ge(&length_done));
        self.emit(abi::compare_immediate(&scratch12, "0"));
        self.emit(abi::branch_eq(&length_no_delim));
        self.emit(abi::add_registers(&scratch11, &scratch11, &scratch10));
        self.emit(abi::label(&length_no_delim));
        self.emit(abi::load_u64(
            &scratch14,
            &scratch13,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers(&scratch11, &scratch11, &scratch14));
        self.emit(abi::add_immediate(&scratch13, &scratch13, COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate(&scratch12, &scratch12, 1));
        self.emit(abi::branch(&length_loop));
        self.emit(abi::label(&length_done));
        self.emit(abi::store_u64(&scratch11, abi::stack_pointer(), output_len_slot));

        self.emit(abi::add_immediate(abi::return_register(), &scratch11, 9));
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
        self.emit(abi::load_u64(&scratch11, abi::stack_pointer(), output_len_slot));
        self.emit(abi::store_u64(&scratch11, "x1", 0));

        self.emit(abi::load_u64(&scratch16, abi::stack_pointer(), parts_slot));
        self.emit(abi::load_u64(&scratch17, abi::stack_pointer(), delimiter_slot));
        self.emit(abi::load_u64(&scratch9, &scratch16, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(&scratch10, &scratch17, 0));
        self.emit(abi::add_immediate(&scratch11, &scratch17, 8));
        // Carry the result pointer in a vreg, not physical x1 (a reload with no
        // call context maps unreliably on x86; the concat/split pattern).
        let out_ptr_v = self.temporary_vreg();
        let out_ptr = out_ptr_v.as_str();
        self.emit(abi::load_u64(out_ptr, abi::stack_pointer(), result_slot));
        self.emit(abi::add_immediate(&scratch13, out_ptr, 8));
        self.emit_collection_data_pointer(&scratch14, &scratch16);
        self.emit(abi::add_immediate(&scratch15, &scratch16, COLLECTION_HEADER_SIZE));
        self.emit(abi::move_immediate(&scratch12, "Integer", "0"));
        self.emit(abi::label(&copy_loop));
        self.emit(abi::compare_registers(&scratch12, &scratch9));
        self.emit(abi::branch_ge(&copy_done));
        self.emit(abi::compare_immediate(&scratch12, "0"));
        self.emit(abi::branch_eq(&copy_no_delim));
        self.emit(abi::move_register(cursor, &scratch11));
        self.emit(abi::move_register(remaining, &scratch10));
        self.emit(abi::label(&delim_loop));
        self.emit(abi::compare_immediate(remaining, "0"));
        self.emit(abi::branch_eq(&delim_done));
        self.emit(abi::load_u8(byte, cursor, 0));
        self.emit(abi::store_u8(byte, &scratch13, 0));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::add_immediate(&scratch13, &scratch13, 1));
        self.emit(abi::subtract_immediate(remaining, remaining, 1));
        self.emit(abi::branch(&delim_loop));
        self.emit(abi::label(&delim_done));
        self.emit(abi::label(&copy_no_delim));
        self.emit(abi::load_u64(
            cursor,
            &scratch15,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            remaining,
            &scratch15,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers(cursor, &scratch14, cursor));
        self.emit(abi::label(&value_loop));
        self.emit(abi::compare_immediate(remaining, "0"));
        self.emit(abi::branch_eq(&value_done));
        self.emit(abi::load_u8(byte, cursor, 0));
        self.emit(abi::store_u8(byte, &scratch13, 0));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::add_immediate(&scratch13, &scratch13, 1));
        self.emit(abi::subtract_immediate(remaining, remaining, 1));
        self.emit(abi::branch(&value_loop));
        self.emit(abi::label(&value_done));
        self.emit(abi::add_immediate(&scratch15, &scratch15, COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate(&scratch12, &scratch12, 1));
        self.emit(abi::branch(&copy_loop));
        self.emit(abi::label(&copy_done));
        self.emit(abi::move_immediate(byte, "Integer", "0"));
        self.emit(abi::store_u8(byte, &scratch13, 0));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: "String".to_string(),
            location: result,
            text: "strings.join".to_string(),
        })
    }

    pub(super) fn lower_strings_split(
        &mut self,
        value: &NirValue,
        delimiter: &NirValue,
    ) -> Result<ValueResult, String> {
        let scratch16 = self.temporary_vreg();
        let scratch17 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        let scratch15 = self.temporary_vreg();
        let scratch20 = self.temporary_vreg();
        let scratch21 = self.temporary_vreg();
        let scratch22 = self.temporary_vreg();
        let scratch23 = self.temporary_vreg();
        let scratch24 = self.temporary_vreg();
        let value = self.lower_value(value)?;
        self.require_string("strings.split value", &value)?;
        let value_slot = self.store_string_pointer("strings_split_value", &value.location);
        let delimiter = self.lower_value(delimiter)?;
        self.require_string("strings.split delimiter", &delimiter)?;
        let delimiter_slot =
            self.store_string_pointer("strings_split_delimiter", &delimiter.location);
        let count_slot = self.allocate_stack_object("strings_split_count", 8);
        let data_len_slot = self.allocate_stack_object("strings_split_data_len", 8);
        let result_slot = self.allocate_stack_object("strings_split_result", 8);
        let layout = CollectionTypeLayout::from_type("List OF String").ok_or_else(|| {
            "native strings.split cannot resolve List OF String layout".to_string()
        })?;

        let invalid_delimiter = self.label("strings_split_invalid_delimiter");
        let length_loop = self.label("strings_split_length_loop");
        let length_compare = self.label("strings_split_length_compare");
        let length_match = self.label("strings_split_length_match");
        let length_next = self.label("strings_split_length_next");
        let length_done = self.label("strings_split_length_done");
        let alloc_ok = self.label("strings_split_alloc_ok");
        let write_loop = self.label("strings_split_write_loop");
        let write_compare = self.label("strings_split_write_compare");
        let write_match = self.label("strings_split_write_match");
        let write_next = self.label("strings_split_write_next");
        let write_final = self.label("strings_split_write_final");
        let write_done = self.label("strings_split_write_done");
        let done = self.label("strings_split_done");

        // Inner delimiter-scan scratch as vregs (was out-of-pool x2-x6, which
        // collide with x86 ABI argument registers). x9-x17/x20-x28 remain (the
        // vregify pass colors those under LinearScan).
        let scan_i_v = self.temporary_vreg();
        let scan_ptr_v = self.temporary_vreg();
        let delim_ptr_v = self.temporary_vreg();
        let sbyte_v = self.temporary_vreg();
        let dbyte_v = self.temporary_vreg();
        let scan_i = scan_i_v.as_str();
        let scan_ptr = scan_ptr_v.as_str();
        let delim_ptr = delim_ptr_v.as_str();
        let sbyte = sbyte_v.as_str();
        let dbyte = dbyte_v.as_str();

        self.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(&scratch17, abi::stack_pointer(), delimiter_slot));
        self.emit(abi::load_u64(&scratch9, &scratch16, 0));
        self.emit(abi::load_u64(&scratch10, &scratch17, 0));
        self.emit(abi::compare_immediate(&scratch10, "0"));
        self.emit(abi::branch_eq(&invalid_delimiter));
        self.emit(abi::move_immediate(&scratch11, "Integer", "1"));
        self.emit(abi::store_u64(&scratch11, abi::stack_pointer(), count_slot));
        self.emit(abi::store_u64(&scratch9, abi::stack_pointer(), data_len_slot));
        self.emit(abi::compare_registers(&scratch10, &scratch9));
        self.emit(abi::branch_hi(&length_done));
        self.emit(abi::subtract_registers(&scratch12, &scratch9, &scratch10));
        self.emit(abi::move_immediate(&scratch13, "Integer", "0"));
        self.emit(abi::add_immediate(&scratch14, &scratch16, 8));
        self.emit(abi::add_immediate(&scratch15, &scratch17, 8));
        self.emit(abi::label(&length_loop));
        self.emit(abi::compare_registers(&scratch13, &scratch12));
        self.emit(abi::branch_hi(&length_done));
        self.emit(abi::move_immediate(scan_i, "Integer", "0"));
        self.emit(abi::add_registers(scan_ptr, &scratch14, &scratch13));
        self.emit(abi::move_register(delim_ptr, &scratch15));
        self.emit(abi::label(&length_compare));
        self.emit(abi::compare_registers(scan_i, &scratch10));
        self.emit(abi::branch_eq(&length_match));
        self.emit(abi::load_u8(sbyte, scan_ptr, 0));
        self.emit(abi::load_u8(dbyte, delim_ptr, 0));
        self.emit(abi::compare_registers(sbyte, dbyte));
        self.emit(abi::branch_ne(&length_next));
        self.emit(abi::add_immediate(scan_i, scan_i, 1));
        self.emit(abi::add_immediate(scan_ptr, scan_ptr, 1));
        self.emit(abi::add_immediate(delim_ptr, delim_ptr, 1));
        self.emit(abi::branch(&length_compare));
        self.emit(abi::label(&length_match));
        self.emit(abi::load_u64(&scratch11, abi::stack_pointer(), count_slot));
        self.emit(abi::add_immediate(&scratch11, &scratch11, 1));
        self.emit(abi::store_u64(&scratch11, abi::stack_pointer(), count_slot));
        self.emit(abi::load_u64(&scratch11, abi::stack_pointer(), data_len_slot));
        self.emit(abi::subtract_registers(&scratch11, &scratch11, &scratch10));
        self.emit(abi::store_u64(&scratch11, abi::stack_pointer(), data_len_slot));
        self.emit(abi::add_registers(&scratch13, &scratch13, &scratch10));
        self.emit(abi::branch(&length_loop));
        self.emit(abi::label(&length_next));
        self.emit(abi::add_immediate(&scratch13, &scratch13, 1));
        self.emit(abi::branch(&length_loop));
        self.emit(abi::label(&length_done));

        self.emit(abi::load_u64(&scratch11, abi::stack_pointer(), count_slot));
        self.emit(abi::load_u64(&scratch12, abi::stack_pointer(), data_len_slot));
        self.emit(abi::move_immediate(
            &scratch13,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch13, &scratch13, &scratch11));
        self.emit(abi::add_immediate(
            abi::return_register(),
            &scratch13,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(
            abi::return_register(),
            abi::return_register(),
            &scratch12,
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
        self.emit(abi::load_u64(&scratch11, abi::stack_pointer(), count_slot));
        self.emit(abi::load_u64(&scratch12, abi::stack_pointer(), data_len_slot));
        self.emit_write_list_header_from_registers(&layout, "x1", &scratch11, &scratch12);

        self.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(&scratch17, abi::stack_pointer(), delimiter_slot));
        self.emit(abi::load_u64(&scratch9, &scratch16, 0));
        self.emit(abi::load_u64(&scratch10, &scratch17, 0));
        self.emit(abi::add_immediate(&scratch14, &scratch16, 8));
        self.emit(abi::add_immediate(&scratch15, &scratch17, 8));
        // Carry the list pointer in a vreg, not physical x1 (a reload with no
        // call context maps unreliably on x86; the concat/repeat pattern).
        let list_ptr_v = self.temporary_vreg();
        let list_ptr = list_ptr_v.as_str();
        self.emit(abi::load_u64(list_ptr, abi::stack_pointer(), result_slot));
        self.emit(abi::add_immediate(&scratch20, list_ptr, COLLECTION_HEADER_SIZE));
        self.emit_collection_data_pointer(&scratch21, list_ptr);
        self.emit(abi::move_immediate(&scratch22, "Integer", "0"));
        self.emit(abi::move_immediate(&scratch23, "Integer", "0"));
        self.emit(abi::move_immediate(&scratch24, "Integer", "0"));
        self.emit(abi::compare_registers(&scratch10, &scratch9));
        self.emit(abi::branch_hi(&write_final));
        self.emit(abi::subtract_registers(&scratch12, &scratch9, &scratch10));
        self.emit(abi::label(&write_loop));
        self.emit(abi::compare_registers(&scratch23, &scratch12));
        self.emit(abi::branch_hi(&write_final));
        self.emit(abi::move_immediate(scan_i, "Integer", "0"));
        self.emit(abi::add_registers(scan_ptr, &scratch14, &scratch23));
        self.emit(abi::move_register(delim_ptr, &scratch15));
        self.emit(abi::label(&write_compare));
        self.emit(abi::compare_registers(scan_i, &scratch10));
        self.emit(abi::branch_eq(&write_match));
        self.emit(abi::load_u8(sbyte, scan_ptr, 0));
        self.emit(abi::load_u8(dbyte, delim_ptr, 0));
        self.emit(abi::compare_registers(sbyte, dbyte));
        self.emit(abi::branch_ne(&write_next));
        self.emit(abi::add_immediate(scan_i, scan_i, 1));
        self.emit(abi::add_immediate(scan_ptr, scan_ptr, 1));
        self.emit(abi::add_immediate(delim_ptr, delim_ptr, 1));
        self.emit(abi::branch(&write_compare));
        self.emit(abi::label(&write_match));
        self.emit_string_split_write_entry(&scratch20, &scratch21, &scratch22, &scratch24, &scratch23, &scratch14)?;
        self.emit(abi::add_registers(&scratch23, &scratch23, &scratch10));
        self.emit(abi::move_register(&scratch24, &scratch23));
        self.emit(abi::branch(&write_loop));
        self.emit(abi::label(&write_next));
        self.emit(abi::add_immediate(&scratch23, &scratch23, 1));
        self.emit(abi::branch(&write_loop));
        self.emit(abi::label(&write_final));
        self.emit_string_split_write_entry(&scratch20, &scratch21, &scratch22, &scratch24, &scratch9, &scratch14)?;
        self.emit(abi::label(&write_done));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        self.emit(abi::branch(&done));
        self.emit(abi::label(&invalid_delimiter));
        self.emit_invalid_argument_return()?;
        self.emit(abi::label(&done));

        Ok(ValueResult {
            type_: "List OF String".to_string(),
            location: result,
            text: "strings.split".to_string(),
        })
    }

    /// startsWithAny / endsWithAny: TRUE if `value` begins (or ends, when
    /// `suffix`) with ANY string in the `List OF String` argument. Empty list ->
    /// FALSE. Total (never errors).
    pub(super) fn lower_strings_with_any(
        &mut self,
        value: &NirValue,
        parts: &NirValue,
        suffix: bool,
    ) -> Result<ValueResult, String> {
        let scratch16 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch17 = self.temporary_vreg();
        let scratch23 = self.temporary_vreg();
        let scratch22 = self.temporary_vreg();
        let scratch21 = self.temporary_vreg();
        let scratch20 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch15 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let value = self.lower_value(value)?;
        self.require_string("strings.withAny value", &value)?;
        let value_slot = self.store_string_pointer("strings_with_any_value", &value.location);
        let parts = self.lower_value(parts)?;
        if list_element_type(&parts.type_).as_deref() != Some("String") {
            return Err(format!(
                "strings.startsWithAny/endsWithAny parts must be List OF String, got {}",
                parts.type_
            ));
        }
        let parts_slot = self.store_string_pointer("strings_with_any_parts", &parts.location);
        let result_slot = self.allocate_stack_object("strings_with_any_result", 8);

        let true_label = self.label("strings_with_any_true");
        let false_label = self.label("strings_with_any_false");
        let done_label = self.label("strings_with_any_done");
        let outer_loop = self.label("strings_with_any_loop");
        let outer_next = self.label("strings_with_any_next");
        let no_match = self.label("strings_with_any_no_match");

        // x16 = value ptr, x9 = value len, x11 = value data ptr.
        self.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(&scratch9, &scratch16, 0));
        self.emit(abi::add_immediate(&scratch11, &scratch16, 8));
        // x17 = list ptr, x19 = count, x22 = entry ptr, x21 = data ptr.
        self.emit(abi::load_u64(&scratch17, abi::stack_pointer(), parts_slot));
        self.emit(abi::load_u64(&scratch23, &scratch17, COLLECTION_OFFSET_COUNT));
        self.emit(abi::add_immediate(&scratch22, &scratch17, COLLECTION_HEADER_SIZE));
        self.emit_collection_data_pointer(&scratch21, &scratch17);
        self.emit(abi::move_immediate(&scratch20, "Integer", "0"));

        self.emit(abi::label(&outer_loop));
        self.emit(abi::compare_registers(&scratch20, &scratch23));
        self.emit(abi::branch_ge(&false_label));
        // x10 = element length, x12 = element bytes pointer.
        self.emit(abi::load_u64(
            &scratch10,
            &scratch22,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::load_u64(
            &scratch12,
            &scratch22,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::add_registers(&scratch12, &scratch21, &scratch12));
        // element longer than value -> no match.
        self.emit(abi::compare_registers(&scratch10, &scratch9));
        self.emit(abi::branch_hi(&outer_next));
        // x15 = compare start in value (offset by len-elementLen for suffix).
        self.emit(abi::move_register(&scratch15, &scratch11));
        if suffix {
            self.emit(abi::subtract_registers(&scratch13, &scratch9, &scratch10));
            self.emit(abi::add_registers(&scratch15, &scratch15, &scratch13));
        }
        self.emit_string_byte_range_equal_branch(&scratch15, &scratch12, &scratch10, &true_label, &no_match);
        self.emit(abi::label(&no_match));
        self.emit(abi::label(&outer_next));
        self.emit(abi::add_immediate(&scratch22, &scratch22, COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate(&scratch20, &scratch20, 1));
        self.emit(abi::branch(&outer_loop));

        self.emit_string_predicate_result(result_slot, &true_label, &false_label, &done_label);
        let label = if suffix {
            "strings.endsWithAny"
        } else {
            "strings.startsWithAny"
        };
        self.finish_string_predicate_result(label, result_slot)
    }

    /// stripPrefix / stripSuffix: if `value` starts (or ends, when `suffix`) with
    /// `part`, return `value` with ONE leading (trailing) `part` removed; else
    /// return `value` unchanged. Total. Empty `part` -> value unchanged.
    pub(super) fn lower_strings_strip(
        &mut self,
        value: &NirValue,
        part: &NirValue,
        suffix: bool,
    ) -> Result<ValueResult, String> {
        let scratch16 = self.temporary_vreg();
        let scratch17 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let value = self.lower_value(value)?;
        self.require_string("strings.strip value", &value)?;
        let value_slot = self.store_string_pointer("strings_strip_value", &value.location);
        let part = self.lower_value(part)?;
        self.require_string("strings.strip part", &part)?;
        let part_slot = self.store_string_pointer("strings_strip_part", &part.location);
        let ptr_slot = self.allocate_stack_object("strings_strip_ptr", 8);
        let len_slot = self.allocate_stack_object("strings_strip_len", 8);

        let matched = self.label("strings_strip_matched");
        let unchanged = self.label("strings_strip_unchanged");
        let no_match = self.label("strings_strip_no_match");
        let build = self.label("strings_strip_build");

        // x16 = value ptr, x9 = value len, x17 = part ptr, x10 = part len.
        self.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(&scratch17, abi::stack_pointer(), part_slot));
        self.emit(abi::load_u64(&scratch9, &scratch16, 0));
        self.emit(abi::load_u64(&scratch10, &scratch17, 0));
        // part empty or longer than value -> unchanged.
        self.emit(abi::compare_immediate(&scratch10, "0"));
        self.emit(abi::branch_eq(&unchanged));
        self.emit(abi::compare_registers(&scratch10, &scratch9));
        self.emit(abi::branch_hi(&unchanged));
        self.emit(abi::add_immediate(&scratch11, &scratch16, 8));
        self.emit(abi::add_immediate(&scratch12, &scratch17, 8));
        if suffix {
            self.emit(abi::subtract_registers(&scratch13, &scratch9, &scratch10));
            self.emit(abi::add_registers(&scratch11, &scratch11, &scratch13));
        }
        self.emit_string_byte_range_equal_branch(&scratch11, &scratch12, &scratch10, &matched, &no_match);
        self.emit(abi::label(&no_match));
        self.emit(abi::branch(&unchanged));

        // matched: result = value with one part removed. Compute ptr/len into slots.
        self.emit(abi::label(&matched));
        self.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(&scratch17, abi::stack_pointer(), part_slot));
        self.emit(abi::load_u64(&scratch9, &scratch16, 0));
        self.emit(abi::load_u64(&scratch10, &scratch17, 0));
        self.emit(abi::subtract_registers(&scratch12, &scratch9, &scratch10));
        self.emit(abi::add_immediate(&scratch13, &scratch16, 8));
        if !suffix {
            // strip from front: advance data pointer past the prefix.
            self.emit(abi::add_registers(&scratch13, &scratch13, &scratch10));
        }
        self.emit(abi::store_u64(&scratch13, abi::stack_pointer(), ptr_slot));
        self.emit(abi::store_u64(&scratch12, abi::stack_pointer(), len_slot));
        self.emit(abi::branch(&build));

        // unchanged: result = whole value (ptr = value+8, len = value len).
        self.emit(abi::label(&unchanged));
        self.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(&scratch9, &scratch16, 0));
        self.emit(abi::add_immediate(&scratch13, &scratch16, 8));
        self.emit(abi::store_u64(&scratch13, abi::stack_pointer(), ptr_slot));
        self.emit(abi::store_u64(&scratch9, abi::stack_pointer(), len_slot));

        self.emit(abi::label(&build));
        self.emit(abi::load_u64(&scratch13, abi::stack_pointer(), ptr_slot));
        self.emit(abi::load_u64(&scratch12, abi::stack_pointer(), len_slot));
        let result = self.emit_materialize_string_from_bytes(&scratch13, &scratch12)?;
        let label = if suffix {
            "strings.stripSuffix"
        } else {
            "strings.stripPrefix"
        };
        Ok(ValueResult {
            type_: "String".to_string(),
            location: result,
            text: label.to_string(),
        })
    }

    /// count: number of NON-overlapping occurrences of `needle` in `value`. Empty
    /// needle -> error 77050002.
    pub(super) fn lower_strings_count(
        &mut self,
        value: &NirValue,
        needle: &NirValue,
    ) -> Result<ValueResult, String> {
        let scratch16 = self.temporary_vreg();
        let scratch17 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch23 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let scratch15 = self.temporary_vreg();
        let value = self.lower_value(value)?;
        self.require_string("strings.count value", &value)?;
        let value_slot = self.store_string_pointer("strings_count_value", &value.location);
        let needle = self.lower_value(needle)?;
        self.require_string("strings.count needle", &needle)?;
        let needle_slot = self.store_string_pointer("strings_count_needle", &needle.location);
        let count_slot = self.allocate_stack_object("strings_count_result", 8);

        let invalid = self.label("strings_count_invalid");
        let loop_label = self.label("strings_count_loop");
        let match_label = self.label("strings_count_match");
        let no_match = self.label("strings_count_no_match");
        let done = self.label("strings_count_done");

        self.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(&scratch17, abi::stack_pointer(), needle_slot));
        self.emit(abi::load_u64(&scratch9, &scratch16, 0));
        self.emit(abi::load_u64(&scratch10, &scratch17, 0));
        self.emit(abi::compare_immediate(&scratch10, "0"));
        self.emit(abi::branch_eq(&invalid));
        // x11 = value data, x12 = needle data, x14 = cursor index, x19 = count.
        self.emit(abi::add_immediate(&scratch11, &scratch16, 8));
        self.emit(abi::add_immediate(&scratch12, &scratch17, 8));
        self.emit(abi::move_immediate(&scratch23, "Integer", "0"));
        self.emit(abi::move_immediate(&scratch14, "Integer", "0"));
        self.emit(abi::label(&loop_label));
        // need x14 + needleLen <= valueLen, i.e. cursor <= valueLen - needleLen.
        self.emit(abi::subtract_registers(&scratch13, &scratch9, &scratch10));
        self.emit(abi::compare_registers(&scratch14, &scratch13));
        self.emit(abi::branch_hi(&done));
        self.emit(abi::add_registers(&scratch15, &scratch11, &scratch14));
        self.emit_string_byte_range_equal_branch(&scratch15, &scratch12, &scratch10, &match_label, &no_match);
        self.emit(abi::label(&match_label));
        self.emit(abi::add_immediate(&scratch23, &scratch23, 1));
        // non-overlapping: advance past the whole needle.
        self.emit(abi::add_registers(&scratch14, &scratch14, &scratch10));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&no_match));
        self.emit(abi::add_immediate(&scratch14, &scratch14, 1));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&done));
        self.emit(abi::store_u64(&scratch23, abi::stack_pointer(), count_slot));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), count_slot));
        let after = self.label("strings_count_after");
        self.emit(abi::branch(&after));
        self.emit(abi::label(&invalid));
        self.emit_invalid_argument_return()?;
        self.emit(abi::label(&after));
        Ok(ValueResult {
            type_: "Integer".to_string(),
            location: result,
            text: "strings.count".to_string(),
        })
    }

    /// left / right: first (last) `count` Unicode scalars. count<0 -> error
    /// 77050002. count>=scalarLen -> whole string. count==0 -> "".
    pub(super) fn lower_strings_left_right(
        &mut self,
        value: &NirValue,
        count: &NirValue,
        right: bool,
    ) -> Result<ValueResult, String> {
        let scratch16 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch17 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        let scratch15 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let value = self.lower_value(value)?;
        self.require_string("strings.left/right value", &value)?;
        let value_slot = self.store_string_pointer("strings_lr_value", &value.location);
        let count = self.lower_value(count)?;
        if count.type_ != "Integer" {
            return Err(format!(
                "strings.left/right count must be Integer, got {}",
                count.type_
            ));
        }
        let count_slot = self.store_string_pointer("strings_lr_count", &count.location);
        let ptr_slot = self.allocate_stack_object("strings_lr_ptr", 8);
        let len_slot = self.allocate_stack_object("strings_lr_len", 8);

        let invalid = self.label("strings_lr_invalid");
        let build = self.label("strings_lr_build");

        self.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), count_slot));
        self.emit(abi::compare_immediate(&scratch10, "0"));
        self.emit(abi::branch_lt(&invalid));
        self.emit(abi::load_u64(&scratch9, &scratch16, 0));
        self.emit(abi::add_immediate(&scratch11, &scratch16, 8));
        // mask = 192, cont byte test == 128.
        self.emit(abi::move_immediate(&scratch17, "Integer", "192"));

        if !right {
            // Walk forward `count` scalars from the start, tracking byte cursor.
            let walk = self.label("strings_left_walk");
            let cont = self.label("strings_left_cont");
            let cont_done = self.label("strings_left_cont_done");
            let walk_done = self.label("strings_left_walk_done");
            // x12 = scalars taken, x14 = byte cursor.
            self.emit(abi::move_immediate(&scratch12, "Integer", "0"));
            self.emit(abi::move_immediate(&scratch14, "Integer", "0"));
            self.emit(abi::label(&walk));
            self.emit(abi::compare_registers(&scratch12, &scratch10));
            self.emit(abi::branch_ge(&walk_done));
            self.emit(abi::compare_registers(&scratch14, &scratch9));
            self.emit(abi::branch_ge(&walk_done));
            // advance one byte (lead), then skip continuation bytes.
            self.emit(abi::add_immediate(&scratch14, &scratch14, 1));
            self.emit(abi::label(&cont));
            self.emit(abi::compare_registers(&scratch14, &scratch9));
            self.emit(abi::branch_ge(&cont_done));
            self.emit(abi::add_registers(&scratch15, &scratch11, &scratch14));
            self.emit(abi::load_u8(&scratch13, &scratch15, 0));
            self.emit(abi::and_registers(&scratch13, &scratch13, &scratch17));
            self.emit(abi::compare_immediate(&scratch13, "128"));
            self.emit(abi::branch_ne(&cont_done));
            self.emit(abi::add_immediate(&scratch14, &scratch14, 1));
            self.emit(abi::branch(&cont));
            self.emit(abi::label(&cont_done));
            self.emit(abi::add_immediate(&scratch12, &scratch12, 1));
            self.emit(abi::branch(&walk));
            self.emit(abi::label(&walk_done));
            // ptr = value+8, len = byte cursor.
            self.emit(abi::store_u64(&scratch11, abi::stack_pointer(), ptr_slot));
            self.emit(abi::store_u64(&scratch14, abi::stack_pointer(), len_slot));
        } else {
            // Walk backward `count` scalars from the end (count non-continuation
            // bytes scanning from the end).
            let walk = self.label("strings_right_walk");
            let walk_done = self.label("strings_right_walk_done");
            let skip = self.label("strings_right_skip");
            let counted = self.label("strings_right_counted");
            // x12 = scalars taken, x14 = byte cursor (one-past current), start at len.
            self.emit(abi::move_immediate(&scratch12, "Integer", "0"));
            self.emit(abi::move_register(&scratch14, &scratch9));
            self.emit(abi::label(&walk));
            self.emit(abi::compare_registers(&scratch12, &scratch10));
            self.emit(abi::branch_ge(&walk_done));
            self.emit(abi::compare_immediate(&scratch14, "0"));
            self.emit(abi::branch_eq(&walk_done));
            // step back over the scalar: at least one byte, plus any continuation bytes.
            self.emit(abi::label(&skip));
            self.emit(abi::subtract_immediate(&scratch14, &scratch14, 1));
            // at index 0 we are necessarily at a scalar boundary.
            self.emit(abi::compare_immediate(&scratch14, "0"));
            self.emit(abi::branch_eq(&counted));
            self.emit(abi::add_registers(&scratch15, &scratch11, &scratch14));
            self.emit(abi::load_u8(&scratch13, &scratch15, 0));
            self.emit(abi::and_registers(&scratch13, &scratch13, &scratch17));
            self.emit(abi::compare_immediate(&scratch13, "128"));
            self.emit(abi::branch_eq(&skip));
            self.emit(abi::label(&counted));
            self.emit(abi::add_immediate(&scratch12, &scratch12, 1));
            self.emit(abi::branch(&walk));
            self.emit(abi::label(&walk_done));
            // ptr = value+8+cursor, len = valueLen - cursor.
            self.emit(abi::add_registers(&scratch13, &scratch11, &scratch14));
            self.emit(abi::subtract_registers(&scratch12, &scratch9, &scratch14));
            self.emit(abi::store_u64(&scratch13, abi::stack_pointer(), ptr_slot));
            self.emit(abi::store_u64(&scratch12, abi::stack_pointer(), len_slot));
        }

        self.emit(abi::branch(&build));
        self.emit(abi::label(&invalid));
        self.emit_invalid_argument_return()?;
        self.emit(abi::label(&build));
        self.emit(abi::load_u64(&scratch13, abi::stack_pointer(), ptr_slot));
        self.emit(abi::load_u64(&scratch12, abi::stack_pointer(), len_slot));
        let result = self.emit_materialize_string_from_bytes(&scratch13, &scratch12)?;
        let label = if right {
            "strings.right"
        } else {
            "strings.left"
        };
        Ok(ValueResult {
            type_: "String".to_string(),
            location: result,
            text: label.to_string(),
        })
    }

    /// repeat: `value` concatenated `times` times. times==0 -> "". times<0 ->
    /// error 77050002.
    pub(super) fn lower_strings_repeat(
        &mut self,
        value: &NirValue,
        times: &NirValue,
    ) -> Result<ValueResult, String> {
        let value = self.lower_value(value)?;
        self.require_string("strings.repeat value", &value)?;
        let value_slot = self.store_string_pointer("strings_repeat_value", &value.location);
        let times = self.lower_value(times)?;
        if times.type_ != "Integer" {
            return Err(format!(
                "strings.repeat times must be Integer, got {}",
                times.type_
            ));
        }
        let times_slot = self.store_string_pointer("strings_repeat_times", &times.location);
        let total_slot = self.allocate_stack_object("strings_repeat_total", 8);
        let result_slot = self.allocate_stack_object("strings_repeat_result", 8);

        let invalid = self.label("strings_repeat_invalid");
        let alloc_ok = self.label("strings_repeat_alloc_ok");
        let outer = self.label("strings_repeat_outer");
        let inner = self.label("strings_repeat_inner");
        let inner_done = self.label("strings_repeat_inner_done");
        let outer_done = self.label("strings_repeat_outer_done");

        // Scratch as vregs (was hand-pinned x9/x10/x11/x13/x14/x16 + out-of-pool
        // x2/x3/x4). x1 stays physical only as the arena_alloc ABI arg/result;
        // the allocation pointer is then carried in a neutral vreg across the
        // copy loops, since a held physical result register is fragile on ISAs
        // whose result/argument registers differ (x86-64).
        let val_ptr_v = self.temporary_vreg();
        let times_rem_v = self.temporary_vreg();
        let len_v = self.temporary_vreg();
        let total_v = self.temporary_vreg();
        let dst_v = self.temporary_vreg();
        let src_base_v = self.temporary_vreg();
        let inner_src_v = self.temporary_vreg();
        let inner_cnt_v = self.temporary_vreg();
        let byte_v = self.temporary_vreg();
        let val_ptr = val_ptr_v.as_str();
        let times_rem = times_rem_v.as_str();
        let len = len_v.as_str();
        let total = total_v.as_str();
        let dst = dst_v.as_str();
        let src_base = src_base_v.as_str();
        let inner_src = inner_src_v.as_str();
        let inner_cnt = inner_cnt_v.as_str();
        let byte = byte_v.as_str();

        self.emit(abi::load_u64(val_ptr, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(times_rem, abi::stack_pointer(), times_slot));
        self.emit(abi::compare_immediate(times_rem, "0"));
        self.emit(abi::branch_lt(&invalid));
        self.emit(abi::load_u64(len, val_ptr, 0));
        // total = len * times.
        self.emit(abi::multiply_registers(total, len, times_rem));
        self.emit(abi::store_u64(total, abi::stack_pointer(), total_slot));
        // allocate total + 9.
        self.emit(abi::add_immediate(abi::return_register(), total, 9));
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
        // Capture the allocation result while x1 is unambiguously the call result.
        let result_ptr = self.allocate_register()?;
        self.emit(abi::move_register(&result_ptr, "x1"));
        self.emit(abi::store_u64(&result_ptr, abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64(total, abi::stack_pointer(), total_slot));
        self.emit(abi::store_u64(total, &result_ptr, 0));

        // Copy loop: times_rem outer counter, dst cursor, src_base, len.
        self.emit(abi::load_u64(val_ptr, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(times_rem, abi::stack_pointer(), times_slot));
        self.emit(abi::load_u64(len, val_ptr, 0));
        self.emit(abi::add_immediate(src_base, val_ptr, 8));
        self.emit(abi::add_immediate(dst, &result_ptr, 8));
        self.emit(abi::label(&outer));
        self.emit(abi::compare_immediate(times_rem, "0"));
        self.emit(abi::branch_eq(&outer_done));
        // inner: copy len bytes from src_base to dst.
        self.emit(abi::move_register(inner_src, src_base));
        self.emit(abi::move_register(inner_cnt, len));
        self.emit(abi::label(&inner));
        self.emit(abi::compare_immediate(inner_cnt, "0"));
        self.emit(abi::branch_eq(&inner_done));
        self.emit(abi::load_u8(byte, inner_src, 0));
        self.emit(abi::store_u8(byte, dst, 0));
        self.emit(abi::add_immediate(inner_src, inner_src, 1));
        self.emit(abi::add_immediate(dst, dst, 1));
        self.emit(abi::subtract_immediate(inner_cnt, inner_cnt, 1));
        self.emit(abi::branch(&inner));
        self.emit(abi::label(&inner_done));
        self.emit(abi::subtract_immediate(times_rem, times_rem, 1));
        self.emit(abi::branch(&outer));
        self.emit(abi::label(&outer_done));
        self.emit(abi::move_immediate(byte, "Integer", "0"));
        self.emit(abi::store_u8(byte, dst, 0));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        let after = self.label("strings_repeat_after");
        self.emit(abi::branch(&after));
        self.emit(abi::label(&invalid));
        self.emit_invalid_argument_return()?;
        self.emit(abi::label(&after));
        Ok(ValueResult {
            type_: "String".to_string(),
            location: result,
            text: "strings.repeat".to_string(),
        })
    }

    /// padLeft / padRight: pad `value` with `padChar` to total scalar `width`.
    /// width<0 -> error 77050002. padChar must be exactly one scalar else error
    /// 77050002. When 2 args, padChar defaults to a single space.
    pub(super) fn lower_strings_pad(&mut self, args: &[NirValue], right: bool) -> Result<ValueResult, String> {
        let scratch9 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch17 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        let scratch16 = self.temporary_vreg();
        let scratch15 = self.temporary_vreg();
        let value = self.lower_value(&args[0])?;
        self.require_string("strings.pad value", &value)?;
        let value_slot = self.store_string_pointer("strings_pad_value", &value.location);
        let width = self.lower_value(&args[1])?;
        if width.type_ != "Integer" {
            return Err(format!(
                "strings.pad width must be Integer, got {}",
                width.type_
            ));
        }
        let width_slot = self.store_string_pointer("strings_pad_width", &width.location);
        let pad_slot = if args.len() == 3 {
            let pad = self.lower_value(&args[2])?;
            self.require_string("strings.pad padChar", &pad)?;
            self.store_string_pointer("strings_pad_char", &pad.location)
        } else {
            // Default padChar is a single space " ". Materialize a one-byte String
            // (0x20) so the downstream code path is uniform.
            let space_slot = self.allocate_stack_object("strings_pad_space_byte", 8);
            self.emit(abi::move_immediate(&scratch9, "Byte", "32"));
            self.emit(abi::store_u8(&scratch9, abi::stack_pointer(), space_slot));
            self.emit(abi::add_immediate(&scratch13, abi::stack_pointer(), space_slot));
            self.emit(abi::move_immediate(&scratch12, "Integer", "1"));
            let space = self.emit_materialize_string_from_bytes(&scratch13, &scratch12)?;
            self.store_string_pointer("strings_pad_char", &space)
        };
        // Number of pad chars to prepend/append.
        let pad_count_slot = self.allocate_stack_object("strings_pad_count", 8);
        // Byte length of one padChar.
        let pad_len_slot = self.allocate_stack_object("strings_pad_char_len", 8);
        let total_slot = self.allocate_stack_object("strings_pad_total", 8);
        let result_slot = self.allocate_stack_object("strings_pad_result", 8);

        let invalid = self.label("strings_pad_invalid");
        let alloc_ok = self.label("strings_pad_alloc_ok");

        // Validate width >= 0.
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), width_slot));
        self.emit(abi::compare_immediate(&scratch10, "0"));
        self.emit(abi::branch_lt(&invalid));

        // Validate padChar is exactly one scalar (len>0 and scalar count == 1).
        self.emit(abi::load_u64(&scratch17, abi::stack_pointer(), pad_slot));
        self.emit(abi::load_u64(&scratch9, &scratch17, 0));
        self.emit(abi::compare_immediate(&scratch9, "0"));
        self.emit(abi::branch_eq(&invalid));
        self.emit(abi::store_u64(&scratch9, abi::stack_pointer(), pad_len_slot));
        {
            let loop_label = self.label("strings_pad_scalars_loop");
            let not_cont = self.label("strings_pad_scalars_not_cont");
            let after = self.label("strings_pad_scalars_after");
            let done = self.label("strings_pad_scalars_done");
            self.emit(abi::add_immediate(&scratch11, &scratch17, 8));
            self.emit(abi::move_immediate(&scratch12, "Integer", "0")); // byte index
            self.emit(abi::move_immediate(&scratch14, "Integer", "0")); // scalar count
            self.emit(abi::move_immediate(&scratch16, "Integer", "192"));
            self.emit(abi::label(&loop_label));
            self.emit(abi::compare_registers(&scratch12, &scratch9));
            self.emit(abi::branch_ge(&done));
            self.emit(abi::add_registers(&scratch15, &scratch11, &scratch12));
            self.emit(abi::load_u8(&scratch13, &scratch15, 0));
            self.emit(abi::and_registers(&scratch13, &scratch13, &scratch16));
            self.emit(abi::compare_immediate(&scratch13, "128"));
            self.emit(abi::branch_ne(&not_cont));
            self.emit(abi::branch(&after));
            self.emit(abi::label(&not_cont));
            self.emit(abi::add_immediate(&scratch14, &scratch14, 1));
            self.emit(abi::label(&after));
            self.emit(abi::add_immediate(&scratch12, &scratch12, 1));
            self.emit(abi::branch(&loop_label));
            self.emit(abi::label(&done));
            self.emit(abi::compare_immediate(&scratch14, "1"));
            self.emit(abi::branch_ne(&invalid));
        }

        // Count scalars in value into x14.
        self.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(&scratch9, &scratch16, 0));
        {
            let loop_label = self.label("strings_pad_value_loop");
            let not_cont = self.label("strings_pad_value_not_cont");
            let after = self.label("strings_pad_value_after");
            let done = self.label("strings_pad_value_done");
            self.emit(abi::add_immediate(&scratch11, &scratch16, 8));
            self.emit(abi::move_immediate(&scratch12, "Integer", "0")); // byte index
            self.emit(abi::move_immediate(&scratch14, "Integer", "0")); // scalar count
            self.emit(abi::move_immediate(&scratch17, "Integer", "192"));
            self.emit(abi::label(&loop_label));
            self.emit(abi::compare_registers(&scratch12, &scratch9));
            self.emit(abi::branch_ge(&done));
            self.emit(abi::add_registers(&scratch15, &scratch11, &scratch12));
            self.emit(abi::load_u8(&scratch13, &scratch15, 0));
            self.emit(abi::and_registers(&scratch13, &scratch13, &scratch17));
            self.emit(abi::compare_immediate(&scratch13, "128"));
            self.emit(abi::branch_ne(&not_cont));
            self.emit(abi::branch(&after));
            self.emit(abi::label(&not_cont));
            self.emit(abi::add_immediate(&scratch14, &scratch14, 1));
            self.emit(abi::label(&after));
            self.emit(abi::add_immediate(&scratch12, &scratch12, 1));
            self.emit(abi::branch(&loop_label));
            self.emit(abi::label(&done));
        }
        // pad_count = max(0, width - scalarLen).
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), width_slot));
        {
            let no_pad = self.label("strings_pad_no_pad");
            let have_pad = self.label("strings_pad_have_pad");
            self.emit(abi::compare_registers(&scratch10, &scratch14));
            self.emit(abi::branch_le(&no_pad));
            self.emit(abi::subtract_registers(&scratch10, &scratch10, &scratch14));
            self.emit(abi::branch(&have_pad));
            self.emit(abi::label(&no_pad));
            self.emit(abi::move_immediate(&scratch10, "Integer", "0"));
            self.emit(abi::label(&have_pad));
        }
        self.emit(abi::store_u64(&scratch10, abi::stack_pointer(), pad_count_slot));

        // total = valueLen + pad_count * padLen.
        self.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(&scratch9, &scratch16, 0));
        self.emit(abi::load_u64(&scratch11, abi::stack_pointer(), pad_len_slot));
        self.emit(abi::multiply_registers(&scratch12, &scratch10, &scratch11));
        self.emit(abi::add_registers(&scratch11, &scratch9, &scratch12));
        self.emit(abi::store_u64(&scratch11, abi::stack_pointer(), total_slot));

        // allocate total + 9.
        self.emit(abi::add_immediate(abi::return_register(), &scratch11, 9));
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
        self.emit(abi::load_u64(&scratch11, abi::stack_pointer(), total_slot));
        self.emit(abi::store_u64(&scratch11, "x1", 0));

        // Write the output. x13 = dst cursor. Carry the result ptr in a vreg,
        // not physical x1 (concat/split pattern). Copy-loop scratch as vregs too
        // (was out-of-pool x2/x3/x4).
        let out_ptr_v = self.temporary_vreg();
        let out_ptr = out_ptr_v.as_str();
        let pad_src_v = self.temporary_vreg();
        let pad_cnt_v = self.temporary_vreg();
        let byte_v = self.temporary_vreg();
        let pad_src = pad_src_v.as_str();
        let pad_cnt = pad_cnt_v.as_str();
        let byte = byte_v.as_str();
        self.emit(abi::load_u64(out_ptr, abi::stack_pointer(), result_slot));
        self.emit(abi::add_immediate(&scratch13, out_ptr, 8));

        let copy_value = |b: &mut Self| {
            // copy value bytes (x14 base, x9 len) to x13.
            b.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
            b.emit(abi::load_u64(&scratch9, &scratch16, 0));
            b.emit(abi::add_immediate(&scratch14, &scratch16, 8));
            let loop_label = b.label("strings_pad_copy_value_loop");
            let done = b.label("strings_pad_copy_value_done");
            b.emit(abi::label(&loop_label));
            b.emit(abi::compare_immediate(&scratch9, "0"));
            b.emit(abi::branch_eq(&done));
            b.emit(abi::load_u8(byte, &scratch14, 0));
            b.emit(abi::store_u8(byte, &scratch13, 0));
            b.emit(abi::add_immediate(&scratch14, &scratch14, 1));
            b.emit(abi::add_immediate(&scratch13, &scratch13, 1));
            b.emit(abi::subtract_immediate(&scratch9, &scratch9, 1));
            b.emit(abi::branch(&loop_label));
            b.emit(abi::label(&done));
        };
        let copy_pads = |b: &mut Self, tag: &str| {
            // write pad_count copies of padChar (x14 base, x11 len) to x13.
            b.emit(abi::load_u64(&scratch10, abi::stack_pointer(), pad_count_slot));
            b.emit(abi::load_u64(&scratch17, abi::stack_pointer(), pad_slot));
            b.emit(abi::add_immediate(&scratch14, &scratch17, 8));
            b.emit(abi::load_u64(&scratch11, abi::stack_pointer(), pad_len_slot));
            let outer = b.label(&format!("strings_pad_{tag}_outer"));
            let outer_done = b.label(&format!("strings_pad_{tag}_outer_done"));
            let inner = b.label(&format!("strings_pad_{tag}_inner"));
            let inner_done = b.label(&format!("strings_pad_{tag}_inner_done"));
            b.emit(abi::label(&outer));
            b.emit(abi::compare_immediate(&scratch10, "0"));
            b.emit(abi::branch_eq(&outer_done));
            b.emit(abi::move_register(pad_src, &scratch14));
            b.emit(abi::move_register(pad_cnt, &scratch11));
            b.emit(abi::label(&inner));
            b.emit(abi::compare_immediate(pad_cnt, "0"));
            b.emit(abi::branch_eq(&inner_done));
            b.emit(abi::load_u8(byte, pad_src, 0));
            b.emit(abi::store_u8(byte, &scratch13, 0));
            b.emit(abi::add_immediate(pad_src, pad_src, 1));
            b.emit(abi::add_immediate(&scratch13, &scratch13, 1));
            b.emit(abi::subtract_immediate(pad_cnt, pad_cnt, 1));
            b.emit(abi::branch(&inner));
            b.emit(abi::label(&inner_done));
            b.emit(abi::subtract_immediate(&scratch10, &scratch10, 1));
            b.emit(abi::branch(&outer));
            b.emit(abi::label(&outer_done));
        };

        if right {
            copy_value(self);
            copy_pads(self, "right");
        } else {
            copy_pads(self, "left");
            copy_value(self);
        }
        self.emit(abi::move_immediate(byte, "Integer", "0"));
        self.emit(abi::store_u8(byte, &scratch13, 0));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        let after = self.label("strings_pad_after");
        self.emit(abi::branch(&after));
        self.emit(abi::label(&invalid));
        self.emit_invalid_argument_return()?;
        self.emit(abi::label(&after));
        let label = if right {
            "strings.padRight"
        } else {
            "strings.padLeft"
        };
        Ok(ValueResult {
            type_: "String".to_string(),
            location: result,
            text: label.to_string(),
        })
    }

    /// graphemesCount: number of extended grapheme clusters in `value`.
    pub(super) fn lower_strings_graphemes_count(&mut self, value: &NirValue) -> Result<ValueResult, String> {
        let scratch16 = self.temporary_vreg();
        let list = self.lower_strings_graphemes(value)?;
        let list_slot = self.store_string_pointer("strings_graphemes_count_list", &list.location);
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&scratch16, abi::stack_pointer(), list_slot));
        self.emit(abi::load_u64(&result, &scratch16, COLLECTION_OFFSET_COUNT));
        Ok(ValueResult {
            type_: "Integer".to_string(),
            location: result,
            text: "strings.graphemesCount".to_string(),
        })
    }

    /// graphemeAt: the extended grapheme cluster at zero-based grapheme `index`.
    /// index<0 or index>=graphemeCount -> error 77050001.
    pub(super) fn lower_strings_grapheme_at(
        &mut self,
        value: &NirValue,
        index: &NirValue,
    ) -> Result<ValueResult, String> {
        let scratch16 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        let scratch15 = self.temporary_vreg();
        let index = self.lower_value(index)?;
        if index.type_ != "Integer" {
            return Err(format!(
                "strings.graphemeAt index must be Integer, got {}",
                index.type_
            ));
        }
        let index_slot = self.store_string_pointer("strings_grapheme_at_index", &index.location);
        let list = self.lower_strings_graphemes(value)?;
        let list_slot = self.store_string_pointer("strings_grapheme_at_list", &list.location);
        let ptr_slot = self.allocate_stack_object("strings_grapheme_at_ptr", 8);
        let len_slot = self.allocate_stack_object("strings_grapheme_at_len", 8);

        let invalid = self.label("strings_grapheme_at_invalid");
        let ok = self.label("strings_grapheme_at_ok");

        self.emit(abi::load_u64(&scratch16, abi::stack_pointer(), list_slot));
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), index_slot));
        self.emit(abi::load_u64(&scratch9, &scratch16, COLLECTION_OFFSET_COUNT));
        self.emit(abi::compare_immediate(&scratch10, "0"));
        self.emit(abi::branch_lt(&invalid));
        self.emit(abi::compare_registers(&scratch10, &scratch9));
        self.emit(abi::branch_ge(&invalid));
        // entry = header + index * ENTRY_SIZE.
        self.emit(abi::move_immediate(
            &scratch11,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(&scratch11, &scratch11, &scratch10));
        self.emit(abi::add_immediate(&scratch12, &scratch16, COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers(&scratch12, &scratch12, &scratch11));
        // x13 = value offset, x14 = value length.
        self.emit(abi::load_u64(
            &scratch13,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            &scratch14,
            &scratch12,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit_collection_data_pointer(&scratch15, &scratch16);
        self.emit(abi::add_registers(&scratch15, &scratch15, &scratch13));
        self.emit(abi::store_u64(&scratch15, abi::stack_pointer(), ptr_slot));
        self.emit(abi::store_u64(&scratch14, abi::stack_pointer(), len_slot));
        self.emit(abi::branch(&ok));
        self.emit(abi::label(&invalid));
        self.emit_index_out_of_range_return()?;
        self.emit(abi::label(&ok));
        self.emit(abi::load_u64(&scratch15, abi::stack_pointer(), ptr_slot));
        self.emit(abi::load_u64(&scratch14, abi::stack_pointer(), len_slot));
        let result = self.emit_materialize_string_from_bytes(&scratch15, &scratch14)?;
        Ok(ValueResult {
            type_: "String".to_string(),
            location: result,
            text: "strings.graphemeAt".to_string(),
        })
    }

    /// trimChars: remove leading/trailing SCALARS of `value` that appear in the
    /// set `chars`. chars=="" -> value unchanged. Scalar-based.
    pub(super) fn lower_strings_trim_chars(
        &mut self,
        value: &NirValue,
        chars: &NirValue,
    ) -> Result<ValueResult, String> {
        let scratch16 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch17 = self.temporary_vreg();
        let scratch15 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let scratch23 = self.temporary_vreg();
        let value = self.lower_value(value)?;
        self.require_string("strings.trimChars value", &value)?;
        let value_slot = self.store_string_pointer("strings_trim_chars_value", &value.location);
        let chars = self.lower_value(chars)?;
        self.require_string("strings.trimChars chars", &chars)?;
        let chars_slot = self.store_string_pointer("strings_trim_chars_chars", &chars.location);
        let start_slot = self.allocate_stack_object("strings_trim_chars_start", 8);
        let end_slot = self.allocate_stack_object("strings_trim_chars_end", 8);

        // start = 0, end = valueLen.
        self.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(&scratch9, &scratch16, 0));
        self.emit(abi::move_immediate(&scratch10, "Integer", "0"));
        self.emit(abi::store_u64(&scratch10, abi::stack_pointer(), start_slot));
        self.emit(abi::store_u64(&scratch9, abi::stack_pointer(), end_slot));

        // Leading trim: while start < end, take scalar [start, scalarEnd); if it is
        // in the chars set, set start = scalarEnd, else stop.
        {
            let loop_label = self.label("strings_trim_chars_lead_loop");
            let done = self.label("strings_trim_chars_lead_done");
            self.emit(abi::label(&loop_label));
            self.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
            self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), start_slot));
            self.emit(abi::load_u64(&scratch11, abi::stack_pointer(), end_slot));
            self.emit(abi::compare_registers(&scratch10, &scratch11));
            self.emit(abi::branch_ge(&done));
            // scalar bytes: [x10, x12) where x12 = scalarEnd (advance one lead +
            // continuation bytes).
            self.emit(abi::add_immediate(&scratch14, &scratch16, 8));
            self.emit(abi::add_registers(&scratch14, &scratch14, &scratch10)); // scalar start ptr
            self.emit(abi::move_register(&scratch12, &scratch10));
            self.emit(abi::add_immediate(&scratch12, &scratch12, 1));
            self.emit(abi::move_immediate(&scratch17, "Integer", "192"));
            let cont = self.label("strings_trim_chars_lead_cont");
            let cont_done = self.label("strings_trim_chars_lead_cont_done");
            self.emit(abi::label(&cont));
            self.emit(abi::compare_registers(&scratch12, &scratch11));
            self.emit(abi::branch_ge(&cont_done));
            self.emit(abi::add_immediate(&scratch15, &scratch16, 8));
            self.emit(abi::add_registers(&scratch15, &scratch15, &scratch12));
            self.emit(abi::load_u8(&scratch13, &scratch15, 0));
            self.emit(abi::and_registers(&scratch13, &scratch13, &scratch17));
            self.emit(abi::compare_immediate(&scratch13, "128"));
            self.emit(abi::branch_ne(&cont_done));
            self.emit(abi::add_immediate(&scratch12, &scratch12, 1));
            self.emit(abi::branch(&cont));
            self.emit(abi::label(&cont_done));
            // scalar byte length = x12 - x10, ptr = x14.
            self.emit(abi::subtract_registers(&scratch23, &scratch12, &scratch10));
            let in_set = self.label("strings_trim_chars_lead_in_set");
            let not_in_set = self.label("strings_trim_chars_lead_not_in_set");
            self.emit_chars_set_contains_branch(&scratch14, &scratch23, chars_slot, &in_set, &not_in_set);
            self.emit(abi::label(&not_in_set));
            self.emit(abi::branch(&done));
            self.emit(abi::label(&in_set));
            self.emit(abi::store_u64(&scratch12, abi::stack_pointer(), start_slot));
            self.emit(abi::branch(&loop_label));
            self.emit(abi::label(&done));
        }

        // Trailing trim: while end > start, take the last scalar [scalarStart, end);
        // if in set, end = scalarStart, else stop.
        {
            let loop_label = self.label("strings_trim_chars_trail_loop");
            let done = self.label("strings_trim_chars_trail_done");
            self.emit(abi::label(&loop_label));
            self.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
            self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), start_slot));
            self.emit(abi::load_u64(&scratch11, abi::stack_pointer(), end_slot));
            self.emit(abi::compare_registers(&scratch11, &scratch10));
            self.emit(abi::branch_le(&done));
            // find scalar start: step back from end over continuation bytes.
            self.emit(abi::move_register(&scratch12, &scratch11));
            self.emit(abi::move_immediate(&scratch17, "Integer", "192"));
            let back = self.label("strings_trim_chars_trail_back");
            let back_done = self.label("strings_trim_chars_trail_back_done");
            self.emit(abi::label(&back));
            self.emit(abi::subtract_immediate(&scratch12, &scratch12, 1));
            self.emit(abi::compare_registers(&scratch12, &scratch10));
            self.emit(abi::branch_le(&back_done));
            self.emit(abi::add_immediate(&scratch15, &scratch16, 8));
            self.emit(abi::add_registers(&scratch15, &scratch15, &scratch12));
            self.emit(abi::load_u8(&scratch13, &scratch15, 0));
            self.emit(abi::and_registers(&scratch13, &scratch13, &scratch17));
            self.emit(abi::compare_immediate(&scratch13, "128"));
            self.emit(abi::branch_eq(&back));
            self.emit(abi::label(&back_done));
            // scalar = [x12, x11), ptr = value+8+x12, len = x11 - x12.
            self.emit(abi::add_immediate(&scratch14, &scratch16, 8));
            self.emit(abi::add_registers(&scratch14, &scratch14, &scratch12));
            self.emit(abi::subtract_registers(&scratch23, &scratch11, &scratch12));
            let in_set = self.label("strings_trim_chars_trail_in_set");
            let not_in_set = self.label("strings_trim_chars_trail_not_in_set");
            self.emit_chars_set_contains_branch(&scratch14, &scratch23, chars_slot, &in_set, &not_in_set);
            self.emit(abi::label(&not_in_set));
            self.emit(abi::branch(&done));
            self.emit(abi::label(&in_set));
            self.emit(abi::store_u64(&scratch12, abi::stack_pointer(), end_slot));
            self.emit(abi::branch(&loop_label));
            self.emit(abi::label(&done));
        }

        // Build result from [start, end).
        self.emit(abi::load_u64(&scratch16, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(&scratch10, abi::stack_pointer(), start_slot));
        self.emit(abi::load_u64(&scratch11, abi::stack_pointer(), end_slot));
        self.emit(abi::subtract_registers(&scratch12, &scratch11, &scratch10));
        self.emit(abi::add_immediate(&scratch13, &scratch16, 8));
        self.emit(abi::add_registers(&scratch13, &scratch13, &scratch10));
        let result = self.emit_materialize_string_from_bytes(&scratch13, &scratch12)?;
        Ok(ValueResult {
            type_: "String".to_string(),
            location: result,
            text: "strings.trimChars".to_string(),
        })
    }
}
