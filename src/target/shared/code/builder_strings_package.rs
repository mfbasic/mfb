use super::*;

impl CodeBuilder<'_> {
    pub(super) fn lower_strings_package_call(
        &mut self,
        target: &str,
        args: &[NirValue],
    ) -> Result<Option<ValueResult>, String> {
        if let Some(value) = self.static_strings_package_string(target, args)? {
            let register = self.load_string_constant(&value)?;
            return Ok(Some(ValueResult {
                type_: "String".to_string(),
                location: register,
                text: format!("{target}"),
            }));
        }
        if target == "strings.graphemes" && args.len() == 1 {
            if let Some(value) = self.static_string_value(&args[0]) {
                let values = crate::unicode_backend::graphemes(&value)
                    .into_iter()
                    .map(|value| NirValue::Const {
                        type_: "String".to_string(),
                        value,
                    })
                    .collect::<Vec<_>>();
                return Ok(Some(self.lower_list_literal("List OF String", &values)?));
            }
        }
        let result = match target {
            "strings.trim" if args.len() == 1 => self.lower_strings_trim(&args[0], true, true)?,
            "strings.trimStart" if args.len() == 1 => {
                self.lower_strings_trim(&args[0], true, false)?
            }
            "strings.trimEnd" if args.len() == 1 => {
                self.lower_strings_trim(&args[0], false, true)?
            }
            "strings.upper" if args.len() == 1 => {
                self.lower_strings_case_map(&args[0], UnicodeCaseMap::Upper)?
            }
            "strings.lower" if args.len() == 1 => {
                self.lower_strings_case_map(&args[0], UnicodeCaseMap::Lower)?
            }
            "strings.caseFold" if args.len() == 1 => {
                self.lower_strings_case_map(&args[0], UnicodeCaseMap::CaseFold)?
            }
            "strings.normalizeNfc" if args.len() == 1 => {
                self.lower_strings_normalize_nfc(&args[0])?
            }
            "strings.byteLen" if args.len() == 1 => self.lower_strings_byte_len(&args[0])?,
            "strings.startsWith" if args.len() == 2 => {
                self.lower_strings_starts_with(&args[0], &args[1])?
            }
            "strings.endsWith" if args.len() == 2 => {
                self.lower_strings_ends_with(&args[0], &args[1])?
            }
            "strings.contains" if args.len() == 2 => {
                self.lower_strings_contains(&args[0], &args[1])?
            }
            "strings.graphemes" if args.len() == 1 => self.lower_strings_graphemes(&args[0])?,
            "strings.split" if args.len() == 2 => self.lower_strings_split(&args[0], &args[1])?,
            "strings.join" if args.len() == 2 => self.lower_strings_join(&args[0], &args[1])?,
            _ => return Ok(None),
        };
        Ok(Some(result))
    }

    fn static_strings_package_string(
        &self,
        target: &str,
        args: &[NirValue],
    ) -> Result<Option<String>, String> {
        let Some(value) = args.first().and_then(|arg| self.static_string_value(arg)) else {
            return Ok(None);
        };
        let value = match target {
            "strings.upper" if args.len() == 1 => crate::unicode_backend::upper(&value),
            "strings.lower" if args.len() == 1 => crate::unicode_backend::lower(&value),
            "strings.caseFold" if args.len() == 1 => crate::unicode_backend::case_fold(&value),
            "strings.normalizeNfc" if args.len() == 1 => {
                crate::unicode_backend::normalize_nfc(&value)
            }
            _ => return Ok(None),
        };
        Ok(Some(value))
    }

    fn lower_strings_graphemes(&mut self, value: &NirValue) -> Result<ValueResult, String> {
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
        for register in [
            "x20", "x21", "x22", "x23", "x24", "x25", "x26", "x27", "x28",
        ] {
            self.mark_register_used(register);
        }

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

        self.emit(abi::load_u64("x16", abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64("x9", "x16", 0));
        self.emit(abi::compare_immediate("x9", "0"));
        self.emit(abi::branch_eq(&count_empty));
        self.emit(abi::add_immediate("x14", "x16", 8));
        self.emit(abi::move_immediate("x22", "Integer", "1"));
        self.emit_utf8_decode_next("x14", "x10", "x11");
        self.emit_unicode_property_lookup("x10", "x12");
        self.emit_unicode_property_boundclass("x12", "x24");
        self.emit_unicode_property_indic_conjunct_break("x12", "x25");
        self.emit(abi::move_register("x23", "x11"));
        self.emit(abi::label(&count_loop));
        self.emit(abi::compare_registers("x23", "x9"));
        self.emit(abi::branch_ge(&count_done));
        self.emit(abi::add_registers("x15", "x14", "x23"));
        self.emit_utf8_decode_next("x15", "x10", "x11");
        self.emit_unicode_property_lookup("x10", "x12");
        self.emit_unicode_property_boundclass("x12", "x26");
        self.emit_unicode_property_indic_conjunct_break("x12", "x27");
        self.emit_grapheme_break_branch("x24", "x25", "x26", "x27", &count_break, &count_no_break);
        self.emit(abi::label(&count_break));
        self.emit(abi::add_immediate("x22", "x22", 1));
        self.emit(abi::branch(&count_after_break));
        self.emit(abi::label(&count_no_break));
        self.emit(abi::branch(&count_after_break));
        self.emit(abi::label(&count_after_break));
        self.emit_grapheme_state_update("x24", "x25", "x26", "x27");
        self.emit(abi::add_registers("x23", "x23", "x11"));
        self.emit(abi::branch(&count_loop));
        self.emit(abi::label(&count_empty));
        self.emit(abi::move_immediate("x22", "Integer", "0"));
        self.emit(abi::label(&count_done));
        self.emit(abi::store_u64("x22", abi::stack_pointer(), count_slot));

        self.emit(abi::move_immediate(
            "x13",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x13", "x13", "x22"));
        self.emit(abi::add_immediate(
            abi::return_register(),
            "x13",
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(
            abi::return_register(),
            abi::return_register(),
            "x9",
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
        self.emit(abi::load_u64("x11", abi::stack_pointer(), count_slot));
        self.emit(abi::load_u64("x16", abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64("x9", "x16", 0));
        self.emit_write_list_header_from_registers(&layout, "x1", "x11", "x9");

        self.emit(abi::compare_immediate("x9", "0"));
        self.emit(abi::branch_eq(&write_empty));
        self.emit(abi::load_u64("x16", abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64("x9", "x16", 0));
        self.emit(abi::add_immediate("x14", "x16", 8));
        self.emit(abi::load_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::add_immediate("x20", "x1", COLLECTION_HEADER_SIZE));
        self.emit_collection_data_pointer("x21", "x1");
        self.emit(abi::move_immediate("x22", "Integer", "0"));
        self.emit(abi::move_immediate("x24", "Integer", "0"));
        self.emit_utf8_decode_next("x14", "x10", "x11");
        self.emit_unicode_property_lookup("x10", "x12");
        self.emit_unicode_property_boundclass("x12", "x25");
        self.emit_unicode_property_indic_conjunct_break("x12", "x26");
        self.emit(abi::store_u64("x25", abi::stack_pointer(), state_bc_slot));
        self.emit(abi::store_u64("x26", abi::stack_pointer(), state_icb_slot));
        self.emit(abi::move_register("x23", "x11"));
        self.emit(abi::label(&write_loop));
        self.emit(abi::compare_registers("x23", "x9"));
        self.emit(abi::branch_ge(&write_final));
        self.emit(abi::add_registers("x15", "x14", "x23"));
        self.emit_utf8_decode_next("x15", "x10", "x11");
        self.emit_unicode_property_lookup("x10", "x12");
        self.emit_unicode_property_boundclass("x12", "x27");
        self.emit_unicode_property_indic_conjunct_break("x12", "x28");
        self.emit(abi::load_u64("x25", abi::stack_pointer(), state_bc_slot));
        self.emit(abi::load_u64("x26", abi::stack_pointer(), state_icb_slot));
        self.emit_grapheme_break_branch("x25", "x26", "x27", "x28", &write_break, &write_no_break);
        self.emit(abi::label(&write_break));
        self.emit_grapheme_state_update("x25", "x26", "x27", "x28");
        self.emit(abi::store_u64("x25", abi::stack_pointer(), state_bc_slot));
        self.emit(abi::store_u64("x26", abi::stack_pointer(), state_icb_slot));
        self.emit_string_split_write_entry("x20", "x21", "x22", "x24", "x23")?;
        self.emit(abi::move_register("x24", "x23"));
        self.emit(abi::branch(&write_after_break));
        self.emit(abi::label(&write_no_break));
        self.emit_grapheme_state_update("x25", "x26", "x27", "x28");
        self.emit(abi::store_u64("x25", abi::stack_pointer(), state_bc_slot));
        self.emit(abi::store_u64("x26", abi::stack_pointer(), state_icb_slot));
        self.emit(abi::branch(&write_after_break));
        self.emit(abi::label(&write_after_break));
        self.emit(abi::add_registers("x23", "x23", "x11"));
        self.emit(abi::branch(&write_loop));
        self.emit(abi::label(&write_final));
        self.emit_string_split_write_entry("x20", "x21", "x22", "x24", "x9")?;
        self.emit(abi::label(&write_empty));

        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: "List OF String".to_string(),
            location: result,
            text: "strings.graphemes".to_string(),
        })
    }

    fn lower_strings_case_map(
        &mut self,
        value: &NirValue,
        map: UnicodeCaseMap,
    ) -> Result<ValueResult, String> {
        let value = self.lower_value(value)?;
        self.require_string(map.label(), &value)?;
        let value_slot = self.store_string_pointer(map.slot_prefix(), &value.location);
        let length_slot = self.allocate_stack_object("strings_case_map_length", 8);
        let width_slot = self.allocate_stack_object("strings_case_map_width", 8);
        let result_slot = self.allocate_stack_object("strings_case_map_result", 8);
        for register in [
            "x20", "x21", "x22", "x23", "x24", "x25", "x26", "x27", "x28",
        ] {
            self.mark_register_used(register);
        }

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

        self.emit(abi::load_u64("x20", abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64("x21", "x20", 0));
        self.emit(abi::add_immediate("x22", "x20", 8));
        self.emit(abi::move_immediate("x23", "Integer", "0"));
        self.emit(abi::move_immediate("x24", "Integer", "0"));
        self.emit(abi::label(&count_loop));
        self.emit(abi::compare_registers("x23", "x21"));
        self.emit(abi::branch_ge(&count_done));
        self.emit(abi::add_registers("x14", "x22", "x23"));
        self.emit_utf8_decode_next("x14", "x10", "x11");
        self.emit(abi::store_u64("x11", abi::stack_pointer(), width_slot));
        self.emit_case_map_lookup(map, "x10", "x26", "x27");
        self.emit(abi::compare_immediate("x27", "0"));
        self.emit(abi::branch_eq(&count_identity));
        self.emit(abi::branch(&count_sequence));
        self.emit(abi::label(&count_identity));
        self.emit(abi::add_registers("x24", "x24", "x11"));
        self.emit(abi::branch(&count_next));
        self.emit(abi::label(&count_sequence));
        self.emit(abi::label(&count_sequence_loop));
        self.emit(abi::compare_immediate("x27", "0"));
        self.emit(abi::branch_eq(&count_next));
        self.emit(abi::load_u32("x10", "x26", 0));
        self.emit(abi::add_immediate("x26", "x26", 4));
        self.emit_utf8_encoded_width("x10", "x13");
        self.emit(abi::add_registers("x24", "x24", "x13"));
        self.emit(abi::subtract_immediate("x27", "x27", 1));
        self.emit(abi::branch(&count_sequence_loop));
        self.emit(abi::label(&count_next));
        self.emit(abi::load_u64("x11", abi::stack_pointer(), width_slot));
        self.emit(abi::add_registers("x23", "x23", "x11"));
        self.emit(abi::branch(&count_loop));
        self.emit(abi::label(&count_done));
        self.emit(abi::store_u64("x24", abi::stack_pointer(), length_slot));

        self.emit(abi::add_immediate(abi::return_register(), "x24", 9));
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
        self.emit(abi::load_u64("x24", abi::stack_pointer(), length_slot));
        self.emit(abi::store_u64("x24", "x1", 0));

        self.emit(abi::load_u64("x20", abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64("x21", "x20", 0));
        self.emit(abi::add_immediate("x22", "x20", 8));
        self.emit(abi::move_immediate("x23", "Integer", "0"));
        self.emit(abi::load_u64("x28", abi::stack_pointer(), result_slot));
        self.emit(abi::add_immediate("x28", "x28", 8));
        self.emit(abi::label(&write_loop));
        self.emit(abi::compare_registers("x23", "x21"));
        self.emit(abi::branch_ge(&write_done));
        self.emit(abi::add_registers("x14", "x22", "x23"));
        self.emit_utf8_decode_next("x14", "x10", "x11");
        self.emit(abi::store_u64("x11", abi::stack_pointer(), width_slot));
        self.emit_case_map_lookup(map, "x10", "x26", "x27");
        self.emit(abi::compare_immediate("x27", "0"));
        self.emit(abi::branch_eq(&write_identity));
        self.emit(abi::branch(&write_sequence));
        self.emit(abi::label(&write_identity));
        self.emit_utf8_encode_next("x28", "x10");
        self.emit(abi::branch(&write_next));
        self.emit(abi::label(&write_sequence));
        self.emit(abi::label(&write_sequence_loop));
        self.emit(abi::compare_immediate("x27", "0"));
        self.emit(abi::branch_eq(&write_next));
        self.emit(abi::load_u32("x10", "x26", 0));
        self.emit(abi::add_immediate("x26", "x26", 4));
        self.emit_utf8_encode_next("x28", "x10");
        self.emit(abi::subtract_immediate("x27", "x27", 1));
        self.emit(abi::branch(&write_sequence_loop));
        self.emit(abi::label(&write_next));
        self.emit(abi::load_u64("x11", abi::stack_pointer(), width_slot));
        self.emit(abi::add_registers("x23", "x23", "x11"));
        self.emit(abi::branch(&write_loop));
        self.emit(abi::label(&write_done));
        self.emit(abi::move_immediate("x10", "Integer", "0"));
        self.emit(abi::store_u8("x10", "x28", 0));

        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: "String".to_string(),
            location: result,
            text: map.name().to_string(),
        })
    }

    fn lower_strings_normalize_nfc(&mut self, value: &NirValue) -> Result<ValueResult, String> {
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
        for register in [
            "x20", "x21", "x22", "x23", "x24", "x25", "x26", "x27", "x28",
        ] {
            self.mark_register_used(register);
        }

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

        self.emit(abi::load_u64("x20", abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64("x21", "x20", 0));
        self.emit(abi::add_immediate("x22", "x20", 8));
        self.emit(abi::move_immediate("x23", "Integer", "0"));
        self.emit(abi::move_immediate("x24", "Integer", "0"));
        self.emit(abi::label(&count_loop));
        self.emit(abi::compare_registers("x23", "x21"));
        self.emit(abi::branch_ge(&count_done));
        self.emit(abi::add_registers("x14", "x22", "x23"));
        self.emit_utf8_decode_next("x14", "x10", "x11");
        self.emit(abi::store_u64("x11", abi::stack_pointer(), width_slot));
        self.emit_unicode_u32_mapping_lookup(
            "x10",
            UNICODE_NFD_ENTRIES_SYMBOL,
            crate::unicode_runtime_tables::tables().nfd_entries.len(),
            UNICODE_NFD_SEQUENCES_SYMBOL,
            "x26",
            "x27",
        );
        self.emit(abi::compare_immediate("x27", "0"));
        self.emit(abi::branch_eq(&count_identity));
        self.emit(abi::add_registers("x24", "x24", "x27"));
        self.emit(abi::branch(&count_next));
        self.emit(abi::label(&count_identity));
        self.emit(abi::add_immediate("x24", "x24", 1));
        self.emit(abi::label(&count_next));
        self.emit(abi::load_u64("x11", abi::stack_pointer(), width_slot));
        self.emit(abi::add_registers("x23", "x23", "x11"));
        self.emit(abi::branch(&count_loop));
        self.emit(abi::label(&count_done));
        self.emit(abi::store_u64(
            "x24",
            abi::stack_pointer(),
            scalar_count_slot,
        ));

        self.emit(abi::move_immediate("x13", "Integer", "8"));
        self.emit(abi::multiply_registers(
            abi::return_register(),
            "x24",
            "x13",
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
        self.emit(abi::branch_eq(&temp_alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&temp_alloc_ok));
        self.emit(abi::store_u64("x1", abi::stack_pointer(), temp_slot));

        self.emit(abi::load_u64("x20", abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64("x21", "x20", 0));
        self.emit(abi::add_immediate("x22", "x20", 8));
        self.emit(abi::load_u64("x25", abi::stack_pointer(), temp_slot));
        self.emit(abi::move_immediate("x23", "Integer", "0"));
        self.emit(abi::move_immediate("x24", "Integer", "0"));
        self.emit(abi::label(&fill_loop));
        self.emit(abi::compare_registers("x23", "x21"));
        self.emit(abi::branch_ge(&fill_done));
        self.emit(abi::add_registers("x14", "x22", "x23"));
        self.emit_utf8_decode_next("x14", "x10", "x11");
        self.emit(abi::store_u64("x11", abi::stack_pointer(), width_slot));
        self.emit_unicode_u32_mapping_lookup(
            "x10",
            UNICODE_NFD_ENTRIES_SYMBOL,
            crate::unicode_runtime_tables::tables().nfd_entries.len(),
            UNICODE_NFD_SEQUENCES_SYMBOL,
            "x26",
            "x27",
        );
        self.emit(abi::compare_immediate("x27", "0"));
        self.emit(abi::branch_eq(&fill_identity));
        self.emit(abi::label(&fill_sequence_loop));
        self.emit(abi::compare_immediate("x27", "0"));
        self.emit(abi::branch_eq(&fill_next));
        self.emit(abi::load_u32("x10", "x26", 0));
        self.emit(abi::add_immediate("x26", "x26", 4));
        self.emit(abi::branch(&fill_store));
        self.emit(abi::label(&fill_identity));
        self.emit(abi::label(&fill_store));
        self.emit(abi::shift_left_immediate("x12", "x24", 3));
        self.emit(abi::add_registers("x12", "x25", "x12"));
        self.emit(abi::store_u64("x10", "x12", 0));
        self.emit(abi::add_immediate("x24", "x24", 1));
        self.emit(abi::compare_immediate("x27", "0"));
        self.emit(abi::branch_eq(&fill_next));
        self.emit(abi::subtract_immediate("x27", "x27", 1));
        self.emit(abi::branch(&fill_sequence_loop));
        self.emit(abi::label(&fill_next));
        self.emit(abi::load_u64("x11", abi::stack_pointer(), width_slot));
        self.emit(abi::add_registers("x23", "x23", "x11"));
        self.emit(abi::branch(&fill_loop));
        self.emit(abi::label(&fill_done));

        self.emit(abi::load_u64("x25", abi::stack_pointer(), temp_slot));
        self.emit(abi::load_u64(
            "x21",
            abi::stack_pointer(),
            scalar_count_slot,
        ));
        self.emit(abi::move_immediate("x23", "Integer", "0"));
        self.emit(abi::label(&order_loop));
        self.emit(abi::add_immediate("x6", "x23", 1));
        self.emit(abi::compare_registers("x6", "x21"));
        self.emit(abi::branch_ge(&order_done));
        self.emit(abi::shift_left_immediate("x12", "x23", 3));
        self.emit(abi::add_registers("x12", "x25", "x12"));
        self.emit(abi::load_u64("x10", "x12", 0));
        self.emit(abi::load_u64("x11", "x12", 8));
        self.emit_unicode_property_lookup("x10", "x13");
        self.emit_unicode_property_combining_class("x13", "x14");
        self.emit_unicode_property_lookup("x11", "x13");
        self.emit_unicode_property_combining_class("x13", "x15");
        self.emit(abi::compare_immediate("x15", "0"));
        self.emit(abi::branch_eq(&order_no_swap));
        self.emit(abi::compare_registers("x14", "x15"));
        self.emit(abi::branch_hi(&order_swap));
        self.emit(abi::branch(&order_no_swap));
        self.emit(abi::label(&order_swap));
        self.emit(abi::store_u64("x11", "x12", 0));
        self.emit(abi::store_u64("x10", "x12", 8));
        self.emit(abi::compare_immediate("x23", "0"));
        self.emit(abi::branch_gt(&order_decrement));
        self.emit(abi::add_immediate("x23", "x23", 1));
        self.emit(abi::branch(&order_loop));
        self.emit(abi::label(&order_decrement));
        self.emit(abi::subtract_immediate("x23", "x23", 1));
        self.emit(abi::branch(&order_loop));
        self.emit(abi::label(&order_no_swap));
        self.emit(abi::add_immediate("x23", "x23", 1));
        self.emit(abi::branch(&order_loop));
        self.emit(abi::label(&order_done));

        self.emit(abi::load_u64("x25", abi::stack_pointer(), temp_slot));
        self.emit(abi::load_u64(
            "x21",
            abi::stack_pointer(),
            scalar_count_slot,
        ));
        self.emit(abi::move_immediate("x23", "Integer", "0"));
        self.emit(abi::move_immediate("x24", "Integer", "0"));
        self.emit(abi::move_immediate("x26", "Integer", "0"));
        self.emit(abi::move_immediate("x27", "Integer", "0"));
        self.emit(abi::move_immediate("x28", "Integer", "0"));
        self.emit(abi::label(&compose_loop));
        self.emit(abi::compare_registers("x23", "x21"));
        self.emit(abi::branch_ge(&compose_next));
        self.emit(abi::shift_left_immediate("x12", "x23", 3));
        self.emit(abi::add_registers("x12", "x25", "x12"));
        self.emit(abi::load_u64("x10", "x12", 0));
        self.emit_unicode_property_lookup("x10", "x13");
        self.emit_unicode_property_combining_class("x13", "x15");
        self.emit(abi::compare_immediate("x26", "0"));
        self.emit(abi::branch_eq(&compose_no_starter));
        self.emit(abi::compare_immediate("x15", "0"));
        self.emit(abi::branch_eq(&compose_try));
        self.emit(abi::compare_registers("x15", "x28"));
        self.emit(abi::branch_hi(&compose_try));
        self.emit(abi::branch(&compose_write));
        self.emit(abi::label(&compose_try));
        self.emit(abi::shift_left_immediate("x12", "x27", 3));
        self.emit(abi::add_registers("x12", "x25", "x12"));
        self.emit(abi::load_u64("x11", "x12", 0));
        self.emit_hangul_composition_attempt(
            "x11",
            "x10",
            "x14",
            &compose_found_direct,
            &compose_try_tables,
        );
        self.emit(abi::label(&compose_try_tables));
        self.emit_unicode_property_lookup("x11", "x13");
        self.emit_unicode_property_comb_index("x13", "x16");
        self.emit_unicode_property_comb_length("x13", "x17");
        self.emit_unicode_property_lookup("x10", "x13");
        self.emit_unicode_property_flags("x13", "x9");
        self.emit(abi::move_immediate("x6", "Integer", "1023"));
        self.emit(abi::compare_registers("x16", "x6"));
        self.emit(abi::branch_ge(&compose_write));
        self.emit(abi::move_immediate("x6", "Integer", "1"));
        self.emit(abi::and_registers("x9", "x9", "x6"));
        self.emit(abi::compare_immediate("x9", "0"));
        self.emit(abi::branch_eq(&compose_write));
        self.emit_load_data_address("x6", UNICODE_COMBINATIONS_SECOND_SYMBOL);
        self.emit(abi::shift_left_immediate("x7", "x16", 2));
        self.emit(abi::add_registers("x6", "x6", "x7"));
        self.emit_load_data_address("x8", UNICODE_COMBINATIONS_COMBINED_SYMBOL);
        self.emit(abi::add_registers("x8", "x8", "x7"));
        self.emit(abi::label(&compose_scan_loop));
        self.emit(abi::compare_immediate("x17", "0"));
        self.emit(abi::branch_eq(&compose_write));
        self.emit(abi::load_u32("x14", "x6", 0));
        self.emit(abi::compare_registers("x14", "x10"));
        self.emit(abi::branch_eq(&compose_found));
        self.emit(abi::branch_hi(&compose_write));
        self.emit(abi::add_immediate("x6", "x6", 4));
        self.emit(abi::add_immediate("x8", "x8", 4));
        self.emit(abi::subtract_immediate("x17", "x17", 1));
        self.emit(abi::branch(&compose_scan_loop));
        self.emit(abi::label(&compose_found));
        self.emit(abi::load_u32("x14", "x8", 0));
        self.emit(abi::label(&compose_found_direct));
        self.emit(abi::shift_left_immediate("x12", "x27", 3));
        self.emit(abi::add_registers("x12", "x25", "x12"));
        self.emit(abi::store_u64("x14", "x12", 0));
        self.emit(abi::add_immediate("x23", "x23", 1));
        self.emit(abi::branch(&compose_loop));
        self.emit(abi::label(&compose_no_starter));
        self.emit(abi::label(&compose_write));
        self.emit(abi::shift_left_immediate("x12", "x24", 3));
        self.emit(abi::add_registers("x12", "x25", "x12"));
        self.emit(abi::store_u64("x10", "x12", 0));
        self.emit(abi::compare_immediate("x15", "0"));
        self.emit(abi::branch_ne(&compose_nonstarter));
        self.emit(abi::move_immediate("x26", "Integer", "1"));
        self.emit(abi::move_register("x27", "x24"));
        self.emit(abi::move_immediate("x28", "Integer", "0"));
        self.emit(abi::branch(&compose_nonstarter_done));
        self.emit(abi::label(&compose_nonstarter));
        self.emit(abi::compare_registers("x15", "x28"));
        self.emit(abi::branch_hi(&compose_nonstarter_update));
        self.emit(abi::branch(&compose_nonstarter_done));
        self.emit(abi::label(&compose_nonstarter_update));
        self.emit(abi::move_register("x28", "x15"));
        self.emit(abi::label(&compose_nonstarter_done));
        self.emit(abi::add_immediate("x24", "x24", 1));
        self.emit(abi::add_immediate("x23", "x23", 1));
        self.emit(abi::branch(&compose_loop));
        self.emit(abi::label(&compose_next));
        self.emit(abi::store_u64(
            "x24",
            abi::stack_pointer(),
            composed_count_slot,
        ));

        self.emit(abi::move_immediate("x23", "Integer", "0"));
        self.emit(abi::move_immediate("x24", "Integer", "0"));
        self.emit(abi::label(&byte_len_loop));
        self.emit(abi::load_u64(
            "x21",
            abi::stack_pointer(),
            composed_count_slot,
        ));
        self.emit(abi::compare_registers("x23", "x21"));
        self.emit(abi::branch_ge(&byte_len_done));
        self.emit(abi::shift_left_immediate("x12", "x23", 3));
        self.emit(abi::load_u64("x25", abi::stack_pointer(), temp_slot));
        self.emit(abi::add_registers("x12", "x25", "x12"));
        self.emit(abi::load_u64("x10", "x12", 0));
        self.emit_utf8_encoded_width("x10", "x11");
        self.emit(abi::add_registers("x24", "x24", "x11"));
        self.emit(abi::add_immediate("x23", "x23", 1));
        self.emit(abi::branch(&byte_len_loop));
        self.emit(abi::label(&byte_len_done));
        self.emit(abi::store_u64("x24", abi::stack_pointer(), output_len_slot));

        self.emit(abi::add_immediate(abi::return_register(), "x24", 9));
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
        self.emit(abi::branch_eq(&result_alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&result_alloc_ok));
        self.emit(abi::store_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64("x24", abi::stack_pointer(), output_len_slot));
        self.emit(abi::store_u64("x24", "x1", 0));
        self.emit(abi::add_immediate("x28", "x1", 8));
        self.emit(abi::move_immediate("x23", "Integer", "0"));
        self.emit(abi::label(&encode_loop));
        self.emit(abi::load_u64(
            "x21",
            abi::stack_pointer(),
            composed_count_slot,
        ));
        self.emit(abi::compare_registers("x23", "x21"));
        self.emit(abi::branch_ge(&encode_done));
        self.emit(abi::shift_left_immediate("x12", "x23", 3));
        self.emit(abi::load_u64("x25", abi::stack_pointer(), temp_slot));
        self.emit(abi::add_registers("x12", "x25", "x12"));
        self.emit(abi::load_u64("x10", "x12", 0));
        self.emit_utf8_encode_next("x28", "x10");
        self.emit(abi::add_immediate("x23", "x23", 1));
        self.emit(abi::branch(&encode_loop));
        self.emit(abi::label(&encode_done));
        self.emit(abi::move_immediate("x10", "Integer", "0"));
        self.emit(abi::store_u8("x10", "x28", 0));

        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: "String".to_string(),
            location: result,
            text: "strings.normalizeNfc".to_string(),
        })
    }

    fn lower_strings_trim(
        &mut self,
        value: &NirValue,
        trim_start: bool,
        trim_end: bool,
    ) -> Result<ValueResult, String> {
        let value = self.lower_value(value)?;
        self.require_string("strings.trim value", &value)?;
        let value_slot = self.store_string_pointer("strings_trim_value", &value.location);
        let start_slot = self.allocate_stack_object("strings_trim_start", 8);
        let end_slot = self.allocate_stack_object("strings_trim_end", 8);
        let done_start = self.label("strings_trim_start_done");

        self.emit(abi::load_u64("x16", abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64("x9", "x16", 0));
        self.emit(abi::move_immediate("x10", "Integer", "0"));
        self.emit(abi::store_u64("x10", abi::stack_pointer(), start_slot));
        self.emit(abi::store_u64("x9", abi::stack_pointer(), end_slot));

        if trim_start {
            let loop_label = self.label("strings_trim_start_loop");
            let ws_label = self.label("strings_trim_start_ws");
            self.emit(abi::add_immediate("x11", "x16", 8));
            self.emit(abi::move_register("x12", "x9"));
            self.emit(abi::label(&loop_label));
            self.emit(abi::compare_immediate("x12", "0"));
            self.emit(abi::branch_eq(&done_start));
            self.emit_unicode_whitespace_branch("x11", "x12", "x13", &ws_label, &done_start);
            self.emit(abi::label(&ws_label));
            self.emit(abi::load_u64("x14", abi::stack_pointer(), start_slot));
            self.emit(abi::add_registers("x14", "x14", "x13"));
            self.emit(abi::store_u64("x14", abi::stack_pointer(), start_slot));
            self.emit(abi::add_registers("x11", "x11", "x13"));
            self.emit(abi::subtract_registers("x12", "x12", "x13"));
            self.emit(abi::branch(&loop_label));
        }
        self.emit(abi::label(&done_start));

        if trim_end {
            let loop_label = self.label("strings_trim_end_loop");
            let ws_label = self.label("strings_trim_end_ws");
            let not_ws_label = self.label("strings_trim_end_not_ws");
            let done_label = self.label("strings_trim_end_done");
            self.emit(abi::load_u64("x14", abi::stack_pointer(), start_slot));
            self.emit(abi::load_u64("x16", abi::stack_pointer(), value_slot));
            self.emit(abi::load_u64("x9", "x16", 0));
            self.emit(abi::add_immediate("x11", "x16", 8));
            self.emit(abi::add_registers("x11", "x11", "x14"));
            self.emit(abi::subtract_registers("x12", "x9", "x14"));
            self.emit(abi::move_register("x15", "x14"));
            self.emit(abi::store_u64("x14", abi::stack_pointer(), end_slot));
            self.emit(abi::label(&loop_label));
            self.emit(abi::compare_immediate("x12", "0"));
            self.emit(abi::branch_eq(&done_label));
            self.emit_unicode_whitespace_branch("x11", "x12", "x13", &ws_label, &not_ws_label);
            self.emit(abi::label(&ws_label));
            self.emit(abi::add_registers("x11", "x11", "x13"));
            self.emit(abi::add_registers("x15", "x15", "x13"));
            self.emit(abi::subtract_registers("x12", "x12", "x13"));
            self.emit(abi::branch(&loop_label));
            self.emit(abi::label(&not_ws_label));
            self.emit(abi::add_immediate("x11", "x11", 1));
            self.emit(abi::add_immediate("x15", "x15", 1));
            self.emit(abi::subtract_immediate("x12", "x12", 1));
            self.emit(abi::store_u64("x15", abi::stack_pointer(), end_slot));
            self.emit(abi::branch(&loop_label));
            self.emit(abi::label(&done_label));
        }

        self.emit(abi::load_u64("x16", abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64("x10", abi::stack_pointer(), start_slot));
        self.emit(abi::load_u64("x11", abi::stack_pointer(), end_slot));
        self.emit(abi::subtract_registers("x12", "x11", "x10"));
        self.emit(abi::add_immediate("x13", "x16", 8));
        self.emit(abi::add_registers("x13", "x13", "x10"));
        let result = self.emit_materialize_string_from_bytes("x13", "x12")?;
        Ok(ValueResult {
            type_: "String".to_string(),
            location: result,
            text: "strings.trim".to_string(),
        })
    }

    fn lower_strings_byte_len(&mut self, value: &NirValue) -> Result<ValueResult, String> {
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

    fn lower_strings_starts_with(
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

    fn lower_strings_ends_with(
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

    fn lower_strings_contains(
        &mut self,
        value: &NirValue,
        needle: &NirValue,
    ) -> Result<ValueResult, String> {
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

        self.emit(abi::load_u64("x16", abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64("x17", abi::stack_pointer(), needle_slot));
        self.emit(abi::load_u64("x9", "x16", 0));
        self.emit(abi::load_u64("x10", "x17", 0));
        self.emit(abi::compare_immediate("x10", "0"));
        self.emit(abi::branch_eq(&true_label));
        self.emit(abi::compare_registers("x10", "x9"));
        self.emit(abi::branch_hi(&false_label));
        self.emit(abi::add_immediate("x11", "x16", 8));
        self.emit(abi::add_immediate("x12", "x17", 8));
        self.emit(abi::subtract_registers("x13", "x9", "x10"));
        self.emit(abi::move_immediate("x14", "Integer", "0"));
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_registers("x14", "x13"));
        self.emit(abi::branch_hi(&false_label));
        self.emit(abi::add_registers("x15", "x11", "x14"));
        self.emit_string_byte_range_equal_branch("x15", "x12", "x10", &true_label, &no_match_label);
        self.emit(abi::label(&no_match_label));
        self.emit(abi::add_immediate("x14", "x14", 1));
        self.emit(abi::branch(&loop_label));
        self.emit_string_predicate_result(result_slot, &true_label, &false_label, &done_label);
        self.finish_string_predicate_result("strings.contains", result_slot)
    }

    pub(super) fn lower_strings_join(
        &mut self,
        parts: &NirValue,
        delimiter: &NirValue,
    ) -> Result<ValueResult, String> {
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

        self.emit(abi::load_u64("x16", abi::stack_pointer(), parts_slot));
        self.emit(abi::load_u64("x17", abi::stack_pointer(), delimiter_slot));
        self.emit(abi::load_u64("x9", "x16", COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64("x10", "x17", 0));
        self.emit(abi::move_immediate("x11", "Integer", "0"));
        self.emit(abi::move_immediate("x12", "Integer", "0"));
        self.emit(abi::add_immediate("x13", "x16", COLLECTION_HEADER_SIZE));
        self.emit(abi::label(&length_loop));
        self.emit(abi::compare_registers("x12", "x9"));
        self.emit(abi::branch_ge(&length_done));
        self.emit(abi::compare_immediate("x12", "0"));
        self.emit(abi::branch_eq(&length_no_delim));
        self.emit(abi::add_registers("x11", "x11", "x10"));
        self.emit(abi::label(&length_no_delim));
        self.emit(abi::load_u64(
            "x14",
            "x13",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers("x11", "x11", "x14"));
        self.emit(abi::add_immediate("x13", "x13", COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate("x12", "x12", 1));
        self.emit(abi::branch(&length_loop));
        self.emit(abi::label(&length_done));
        self.emit(abi::store_u64("x11", abi::stack_pointer(), output_len_slot));

        self.emit(abi::add_immediate(abi::return_register(), "x11", 9));
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
        self.emit(abi::load_u64("x11", abi::stack_pointer(), output_len_slot));
        self.emit(abi::store_u64("x11", "x1", 0));

        self.emit(abi::load_u64("x16", abi::stack_pointer(), parts_slot));
        self.emit(abi::load_u64("x17", abi::stack_pointer(), delimiter_slot));
        self.emit(abi::load_u64("x9", "x16", COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64("x10", "x17", 0));
        self.emit(abi::add_immediate("x11", "x17", 8));
        self.emit(abi::load_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::add_immediate("x13", "x1", 8));
        self.emit_collection_data_pointer("x14", "x16");
        self.emit(abi::add_immediate("x15", "x16", COLLECTION_HEADER_SIZE));
        self.emit(abi::move_immediate("x12", "Integer", "0"));
        self.emit(abi::label(&copy_loop));
        self.emit(abi::compare_registers("x12", "x9"));
        self.emit(abi::branch_ge(&copy_done));
        self.emit(abi::compare_immediate("x12", "0"));
        self.emit(abi::branch_eq(&copy_no_delim));
        self.emit(abi::move_register("x2", "x11"));
        self.emit(abi::move_register("x3", "x10"));
        self.emit(abi::label(&delim_loop));
        self.emit(abi::compare_immediate("x3", "0"));
        self.emit(abi::branch_eq(&delim_done));
        self.emit(abi::load_u8("x4", "x2", 0));
        self.emit(abi::store_u8("x4", "x13", 0));
        self.emit(abi::add_immediate("x2", "x2", 1));
        self.emit(abi::add_immediate("x13", "x13", 1));
        self.emit(abi::subtract_immediate("x3", "x3", 1));
        self.emit(abi::branch(&delim_loop));
        self.emit(abi::label(&delim_done));
        self.emit(abi::label(&copy_no_delim));
        self.emit(abi::load_u64(
            "x2",
            "x15",
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            "x3",
            "x15",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers("x2", "x14", "x2"));
        self.emit(abi::label(&value_loop));
        self.emit(abi::compare_immediate("x3", "0"));
        self.emit(abi::branch_eq(&value_done));
        self.emit(abi::load_u8("x4", "x2", 0));
        self.emit(abi::store_u8("x4", "x13", 0));
        self.emit(abi::add_immediate("x2", "x2", 1));
        self.emit(abi::add_immediate("x13", "x13", 1));
        self.emit(abi::subtract_immediate("x3", "x3", 1));
        self.emit(abi::branch(&value_loop));
        self.emit(abi::label(&value_done));
        self.emit(abi::add_immediate("x15", "x15", COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate("x12", "x12", 1));
        self.emit(abi::branch(&copy_loop));
        self.emit(abi::label(&copy_done));
        self.emit(abi::move_immediate("x4", "Integer", "0"));
        self.emit(abi::store_u8("x4", "x13", 0));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: "String".to_string(),
            location: result,
            text: "strings.join".to_string(),
        })
    }

    fn lower_strings_split(
        &mut self,
        value: &NirValue,
        delimiter: &NirValue,
    ) -> Result<ValueResult, String> {
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
        for register in [
            "x20", "x21", "x22", "x23", "x24", "x25", "x26", "x27", "x28",
        ] {
            self.mark_register_used(register);
        }

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

        self.emit(abi::load_u64("x16", abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64("x17", abi::stack_pointer(), delimiter_slot));
        self.emit(abi::load_u64("x9", "x16", 0));
        self.emit(abi::load_u64("x10", "x17", 0));
        self.emit(abi::compare_immediate("x10", "0"));
        self.emit(abi::branch_eq(&invalid_delimiter));
        self.emit(abi::move_immediate("x11", "Integer", "1"));
        self.emit(abi::store_u64("x11", abi::stack_pointer(), count_slot));
        self.emit(abi::store_u64("x9", abi::stack_pointer(), data_len_slot));
        self.emit(abi::compare_registers("x10", "x9"));
        self.emit(abi::branch_hi(&length_done));
        self.emit(abi::subtract_registers("x12", "x9", "x10"));
        self.emit(abi::move_immediate("x13", "Integer", "0"));
        self.emit(abi::add_immediate("x14", "x16", 8));
        self.emit(abi::add_immediate("x15", "x17", 8));
        self.emit(abi::label(&length_loop));
        self.emit(abi::compare_registers("x13", "x12"));
        self.emit(abi::branch_hi(&length_done));
        self.emit(abi::move_immediate("x2", "Integer", "0"));
        self.emit(abi::add_registers("x3", "x14", "x13"));
        self.emit(abi::move_register("x4", "x15"));
        self.emit(abi::label(&length_compare));
        self.emit(abi::compare_registers("x2", "x10"));
        self.emit(abi::branch_eq(&length_match));
        self.emit(abi::load_u8("x5", "x3", 0));
        self.emit(abi::load_u8("x6", "x4", 0));
        self.emit(abi::compare_registers("x5", "x6"));
        self.emit(abi::branch_ne(&length_next));
        self.emit(abi::add_immediate("x2", "x2", 1));
        self.emit(abi::add_immediate("x3", "x3", 1));
        self.emit(abi::add_immediate("x4", "x4", 1));
        self.emit(abi::branch(&length_compare));
        self.emit(abi::label(&length_match));
        self.emit(abi::load_u64("x11", abi::stack_pointer(), count_slot));
        self.emit(abi::add_immediate("x11", "x11", 1));
        self.emit(abi::store_u64("x11", abi::stack_pointer(), count_slot));
        self.emit(abi::load_u64("x11", abi::stack_pointer(), data_len_slot));
        self.emit(abi::subtract_registers("x11", "x11", "x10"));
        self.emit(abi::store_u64("x11", abi::stack_pointer(), data_len_slot));
        self.emit(abi::add_registers("x13", "x13", "x10"));
        self.emit(abi::branch(&length_loop));
        self.emit(abi::label(&length_next));
        self.emit(abi::add_immediate("x13", "x13", 1));
        self.emit(abi::branch(&length_loop));
        self.emit(abi::label(&length_done));

        self.emit(abi::load_u64("x11", abi::stack_pointer(), count_slot));
        self.emit(abi::load_u64("x12", abi::stack_pointer(), data_len_slot));
        self.emit(abi::move_immediate(
            "x13",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x13", "x13", "x11"));
        self.emit(abi::add_immediate(
            abi::return_register(),
            "x13",
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(
            abi::return_register(),
            abi::return_register(),
            "x12",
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
        self.emit(abi::load_u64("x11", abi::stack_pointer(), count_slot));
        self.emit(abi::load_u64("x12", abi::stack_pointer(), data_len_slot));
        self.emit_write_list_header_from_registers(&layout, "x1", "x11", "x12");

        self.emit(abi::load_u64("x16", abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64("x17", abi::stack_pointer(), delimiter_slot));
        self.emit(abi::load_u64("x9", "x16", 0));
        self.emit(abi::load_u64("x10", "x17", 0));
        self.emit(abi::add_immediate("x14", "x16", 8));
        self.emit(abi::add_immediate("x15", "x17", 8));
        self.emit(abi::load_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::add_immediate("x20", "x1", COLLECTION_HEADER_SIZE));
        self.emit_collection_data_pointer("x21", "x1");
        self.emit(abi::move_immediate("x22", "Integer", "0"));
        self.emit(abi::move_immediate("x23", "Integer", "0"));
        self.emit(abi::move_immediate("x24", "Integer", "0"));
        self.emit(abi::compare_registers("x10", "x9"));
        self.emit(abi::branch_hi(&write_final));
        self.emit(abi::subtract_registers("x12", "x9", "x10"));
        self.emit(abi::label(&write_loop));
        self.emit(abi::compare_registers("x23", "x12"));
        self.emit(abi::branch_hi(&write_final));
        self.emit(abi::move_immediate("x2", "Integer", "0"));
        self.emit(abi::add_registers("x3", "x14", "x23"));
        self.emit(abi::move_register("x4", "x15"));
        self.emit(abi::label(&write_compare));
        self.emit(abi::compare_registers("x2", "x10"));
        self.emit(abi::branch_eq(&write_match));
        self.emit(abi::load_u8("x5", "x3", 0));
        self.emit(abi::load_u8("x6", "x4", 0));
        self.emit(abi::compare_registers("x5", "x6"));
        self.emit(abi::branch_ne(&write_next));
        self.emit(abi::add_immediate("x2", "x2", 1));
        self.emit(abi::add_immediate("x3", "x3", 1));
        self.emit(abi::add_immediate("x4", "x4", 1));
        self.emit(abi::branch(&write_compare));
        self.emit(abi::label(&write_match));
        self.emit_string_split_write_entry("x20", "x21", "x22", "x24", "x23")?;
        self.emit(abi::add_registers("x23", "x23", "x10"));
        self.emit(abi::move_register("x24", "x23"));
        self.emit(abi::branch(&write_loop));
        self.emit(abi::label(&write_next));
        self.emit(abi::add_immediate("x23", "x23", 1));
        self.emit(abi::branch(&write_loop));
        self.emit(abi::label(&write_final));
        self.emit_string_split_write_entry("x20", "x21", "x22", "x24", "x9")?;
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

    fn emit_string_split_write_entry(
        &mut self,
        entry: &str,
        data: &str,
        data_offset: &str,
        segment_start: &str,
        segment_end: &str,
    ) -> Result<(), String> {
        let copy_segment_loop = self.label("strings_split_copy_segment_loop");
        let copy_segment_done = self.label("strings_split_copy_segment_done");
        self.emit(abi::move_immediate(
            "x25",
            "Byte",
            &COLLECTION_ENTRY_FLAG_USED.to_string(),
        ));
        self.emit(abi::store_u8("x25", entry, COLLECTION_ENTRY_OFFSET_FLAGS));
        self.emit(abi::move_immediate("x25", "Integer", "0"));
        self.emit(abi::store_u64(
            "x25",
            entry,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::store_u64(
            "x25",
            entry,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::store_u64(
            data_offset,
            entry,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::subtract_registers("x25", segment_end, segment_start));
        self.emit(abi::store_u64(
            "x25",
            entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers("x26", data, data_offset));
        self.emit(abi::add_registers("x27", "x14", segment_start));
        self.emit(abi::label(&copy_segment_loop));
        self.emit(abi::compare_immediate("x25", "0"));
        self.emit(abi::branch_eq(&copy_segment_done));
        self.emit(abi::load_u8("x28", "x27", 0));
        self.emit(abi::store_u8("x28", "x26", 0));
        self.emit(abi::add_immediate("x27", "x27", 1));
        self.emit(abi::add_immediate("x26", "x26", 1));
        self.emit(abi::subtract_immediate("x25", "x25", 1));
        self.emit(abi::branch(&copy_segment_loop));
        self.emit(abi::label(&copy_segment_done));
        self.emit(abi::subtract_registers("x25", segment_end, segment_start));
        self.emit(abi::add_registers(data_offset, data_offset, "x25"));
        self.emit(abi::add_immediate(entry, entry, COLLECTION_ENTRY_SIZE));
        Ok(())
    }

    fn lower_string_prefix_predicate(
        &mut self,
        label: &str,
        value_slot: usize,
        part_slot: usize,
        suffix: bool,
    ) -> Result<ValueResult, String> {
        let result_slot = self.allocate_stack_object("strings_prefix_result", 8);
        let true_label = self.label("strings_prefix_true");
        let false_label = self.label("strings_prefix_false");
        let done_label = self.label("strings_prefix_done");
        let no_match_label = self.label("strings_prefix_no_match");

        self.emit(abi::load_u64("x16", abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64("x17", abi::stack_pointer(), part_slot));
        self.emit(abi::load_u64("x9", "x16", 0));
        self.emit(abi::load_u64("x10", "x17", 0));
        self.emit(abi::compare_registers("x10", "x9"));
        self.emit(abi::branch_hi(&false_label));
        self.emit(abi::add_immediate("x11", "x16", 8));
        self.emit(abi::add_immediate("x12", "x17", 8));
        if suffix {
            self.emit(abi::subtract_registers("x13", "x9", "x10"));
            self.emit(abi::add_registers("x11", "x11", "x13"));
        }
        self.emit_string_byte_range_equal_branch("x11", "x12", "x10", &true_label, &no_match_label);
        self.emit(abi::label(&no_match_label));
        self.emit(abi::branch(&false_label));
        self.emit_string_predicate_result(result_slot, &true_label, &false_label, &done_label);
        self.finish_string_predicate_result(label, result_slot)
    }

    fn emit_string_predicate_result(
        &mut self,
        result_slot: usize,
        true_label: &str,
        false_label: &str,
        done_label: &str,
    ) {
        self.emit(abi::label(true_label));
        self.emit(abi::move_immediate("x8", "Boolean", "true"));
        self.emit(abi::store_u64("x8", abi::stack_pointer(), result_slot));
        self.emit(abi::branch(done_label));
        self.emit(abi::label(false_label));
        self.emit(abi::move_immediate("x8", "Boolean", "false"));
        self.emit(abi::store_u64("x8", abi::stack_pointer(), result_slot));
        self.emit(abi::label(done_label));
    }

    fn finish_string_predicate_result(
        &mut self,
        label: &str,
        result_slot: usize,
    ) -> Result<ValueResult, String> {
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: "Boolean".to_string(),
            location: result,
            text: label.to_string(),
        })
    }

    pub(super) fn require_string(&self, label: &str, value: &ValueResult) -> Result<(), String> {
        if value.type_ == "String" {
            Ok(())
        } else {
            Err(format!("{label} must be String, got {}", value.type_))
        }
    }

    pub(super) fn store_string_pointer(&mut self, label: &str, register: &str) -> usize {
        let slot = self.allocate_stack_object(label, 8);
        self.emit(abi::store_u64(register, abi::stack_pointer(), slot));
        slot
    }

    fn emit_case_map_lookup(
        &mut self,
        map: UnicodeCaseMap,
        codepoint: &str,
        sequence_ptr: &str,
        sequence_length: &str,
    ) {
        self.emit_unicode_u32_mapping_lookup(
            codepoint,
            map.entries_symbol(),
            map.entry_count(),
            map.sequences_symbol(),
            sequence_ptr,
            sequence_length,
        );
    }
}

#[derive(Clone, Copy)]
enum UnicodeCaseMap {
    Upper,
    Lower,
    CaseFold,
}

impl UnicodeCaseMap {
    fn name(self) -> &'static str {
        match self {
            UnicodeCaseMap::Upper => "strings.upper",
            UnicodeCaseMap::Lower => "strings.lower",
            UnicodeCaseMap::CaseFold => "strings.caseFold",
        }
    }

    fn label(self) -> &'static str {
        match self {
            UnicodeCaseMap::Upper => "strings.upper value",
            UnicodeCaseMap::Lower => "strings.lower value",
            UnicodeCaseMap::CaseFold => "strings.caseFold value",
        }
    }

    fn slot_prefix(self) -> &'static str {
        match self {
            UnicodeCaseMap::Upper => "strings_upper_value",
            UnicodeCaseMap::Lower => "strings_lower_value",
            UnicodeCaseMap::CaseFold => "strings_case_fold_value",
        }
    }

    fn entries_symbol(self) -> &'static str {
        match self {
            UnicodeCaseMap::Upper => UNICODE_UPPERCASE_ENTRIES_SYMBOL,
            UnicodeCaseMap::Lower => UNICODE_LOWERCASE_ENTRIES_SYMBOL,
            UnicodeCaseMap::CaseFold => UNICODE_CASEFOLD_ENTRIES_SYMBOL,
        }
    }

    fn sequences_symbol(self) -> &'static str {
        match self {
            UnicodeCaseMap::Upper => UNICODE_UPPERCASE_SEQUENCES_SYMBOL,
            UnicodeCaseMap::Lower => UNICODE_LOWERCASE_SEQUENCES_SYMBOL,
            UnicodeCaseMap::CaseFold => UNICODE_CASEFOLD_SEQUENCES_SYMBOL,
        }
    }

    fn entry_count(self) -> usize {
        let tables = crate::unicode_runtime_tables::tables();
        match self {
            UnicodeCaseMap::Upper => tables.uppercase_entries.len(),
            UnicodeCaseMap::Lower => tables.lowercase_entries.len(),
            UnicodeCaseMap::CaseFold => tables.casefold_entries.len(),
        }
    }
}
