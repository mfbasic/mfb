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
        if target == "strings.toBytes" && args.len() == 1 {
            if let Some(value) = self.static_string_value(&args[0]) {
                let values = value
                    .as_bytes()
                    .iter()
                    .map(|byte| NirValue::Const {
                        type_: "Byte".to_string(),
                        value: byte.to_string(),
                    })
                    .collect::<Vec<_>>();
                return Ok(Some(self.lower_list_literal("List OF Byte", &values)?));
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
            "strings.toBytes" if args.len() == 1 => self.lower_strings_to_bytes(&args[0])?,
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
            "strings.startsWithAny" if args.len() == 2 => {
                self.lower_strings_with_any(&args[0], &args[1], false)?
            }
            "strings.endsWithAny" if args.len() == 2 => {
                self.lower_strings_with_any(&args[0], &args[1], true)?
            }
            "strings.stripPrefix" if args.len() == 2 => {
                self.lower_strings_strip(&args[0], &args[1], false)?
            }
            "strings.stripSuffix" if args.len() == 2 => {
                self.lower_strings_strip(&args[0], &args[1], true)?
            }
            "strings.count" if args.len() == 2 => self.lower_strings_count(&args[0], &args[1])?,
            "strings.left" if args.len() == 2 => {
                self.lower_strings_left_right(&args[0], &args[1], false)?
            }
            "strings.right" if args.len() == 2 => {
                self.lower_strings_left_right(&args[0], &args[1], true)?
            }
            "strings.repeat" if args.len() == 2 => self.lower_strings_repeat(&args[0], &args[1])?,
            "strings.padLeft" if args.len() == 2 || args.len() == 3 => {
                self.lower_strings_pad(args, false)?
            }
            "strings.padRight" if args.len() == 2 || args.len() == 3 => {
                self.lower_strings_pad(args, true)?
            }
            "strings.graphemeAt" if args.len() == 2 => {
                self.lower_strings_grapheme_at(&args[0], &args[1])?
            }
            "strings.graphemesCount" if args.len() == 1 => {
                self.lower_strings_graphemes_count(&args[0])?
            }
            "strings.trimChars" if args.len() == 2 => {
                self.lower_strings_trim_chars(&args[0], &args[1])?
            }
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

    /// Branch to `in_set` if the scalar at [`scalar_ptr`, scalar_ptr+scalar_len)
    /// byte-matches any scalar in the `chars` set string; otherwise branch to
    /// `not_in_set`. The chars string pointer lives in `chars_slot`.
    ///
    /// Uses x2-x8 as scratch (callee-clobbered temporaries) plus the passed
    /// registers; does not touch x9-x19 except the passed scalar registers.
    pub(super) fn emit_chars_set_contains_branch(
        &mut self,
        scalar_ptr: &str,
        scalar_len: &str,
        chars_slot: usize,
        in_set: &str,
        not_in_set: &str,
    ) {
        // Save scalar ptr/len into scratch we control across the inner loop.
        let loop_label = self.label("strings_chars_set_loop");
        let cmp_loop = self.label("strings_chars_set_cmp_loop");
        let next = self.label("strings_chars_set_next");
        // Scratch as vregs: the registers this loop needs are x86-64 ABI
        // argument/return registers, so none of them may be pinned here.
        let chars_ptr_v = self.temporary_vreg();
        let chars_len_v = self.temporary_vreg();
        let cursor_v = self.temporary_vreg();
        let cand_v = self.temporary_vreg();
        let scalar_end_v = self.temporary_vreg();
        let tmp_v = self.temporary_vreg();
        let cbyte_v = self.temporary_vreg();
        let rem_v = self.temporary_vreg();
        let chars_ptr = chars_ptr_v.as_str();
        let chars_len = chars_len_v.as_str();
        let cursor = cursor_v.as_str();
        let cand = cand_v.as_str();
        let scalar_end = scalar_end_v.as_str();
        let tmp = tmp_v.as_str();
        let cbyte = cbyte_v.as_str();
        let rem = rem_v.as_str();
        let cont_mask = self.temporary_vreg();
        let target_byte = self.temporary_vreg();
        // chars_ptr = chars data ptr, chars_len = chars len, cursor = index.
        self.emit(abi::load_u64(cand, abi::stack_pointer(), chars_slot));
        self.emit(abi::load_u64(chars_len, cand, 0));
        self.emit(abi::add_immediate(chars_ptr, cand, 8));
        self.emit(abi::move_immediate(cursor, "Integer", "0"));
        self.emit(abi::move_immediate(&cont_mask, "Integer", "192"));
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_registers(cursor, chars_len));
        self.emit(abi::branch_ge(not_in_set));
        // scalar length: from cursor advance lead + continuation bytes -> scalar_end.
        self.emit(abi::add_immediate(scalar_end, cursor, 1));
        let clen = self.label("strings_chars_set_clen");
        let clen_done = self.label("strings_chars_set_clen_done");
        self.emit(abi::label(&clen));
        self.emit(abi::compare_registers(scalar_end, chars_len));
        self.emit(abi::branch_ge(&clen_done));
        self.emit(abi::add_registers(tmp, chars_ptr, scalar_end));
        self.emit(abi::load_u8(tmp, tmp, 0));
        self.emit(abi::and_registers(tmp, tmp, &cont_mask));
        self.emit(abi::compare_immediate(tmp, "128"));
        self.emit(abi::branch_ne(&clen_done));
        self.emit(abi::add_immediate(scalar_end, scalar_end, 1));
        self.emit(abi::branch(&clen));
        self.emit(abi::label(&clen_done));
        // candidate byte length = scalar_end - cursor. Compare with scalar_len.
        self.emit(abi::subtract_registers(tmp, scalar_end, cursor));
        self.emit(abi::compare_registers(tmp, scalar_len));
        self.emit(abi::branch_ne(&next));
        // byte-compare candidate [chars_ptr+cursor, len) against scalar_ptr.
        self.emit(abi::add_registers(cand, chars_ptr, cursor)); // candidate ptr
        self.emit(abi::move_register(tmp, scalar_ptr)); // target ptr
        self.emit(abi::subtract_registers(rem, scalar_end, cursor)); // remaining bytes
        self.emit(abi::label(&cmp_loop));
        self.emit(abi::compare_immediate(rem, "0"));
        self.emit(abi::branch_eq(in_set));
        self.emit(abi::load_u8(cbyte, cand, 0));
        self.emit(abi::load_u8(&target_byte, tmp, 0));
        self.emit(abi::compare_registers(cbyte, &target_byte));
        self.emit(abi::branch_ne(&next));
        self.emit(abi::add_immediate(cand, cand, 1));
        self.emit(abi::add_immediate(tmp, tmp, 1));
        self.emit(abi::subtract_immediate(rem, rem, 1));
        self.emit(abi::branch(&cmp_loop));
        self.emit(abi::label(&next));
        self.emit(abi::move_register(cursor, scalar_end));
        self.emit(abi::branch(&loop_label));
    }

    pub(super) fn emit_string_split_write_entry(
        &mut self,
        entry: &str,
        data: &str,
        data_offset: &str,
        segment_start: &str,
        segment_end: &str,
        source_data: &str,
    ) -> Result<(), String> {
        let tmp = self.temporary_vreg();
        let dst = self.temporary_vreg();
        let src = self.temporary_vreg();
        let byte = self.temporary_vreg();
        let copy_segment_loop = self.label("strings_split_copy_segment_loop");
        let copy_segment_done = self.label("strings_split_copy_segment_done");
        self.emit(abi::move_immediate(
            &tmp,
            "Byte",
            &COLLECTION_ENTRY_FLAG_USED.to_string(),
        ));
        self.emit(abi::store_u8(&tmp, entry, COLLECTION_ENTRY_OFFSET_FLAGS));
        self.emit(abi::move_immediate(&tmp, "Integer", "0"));
        self.emit(abi::store_u64(
            &tmp,
            entry,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::store_u64(
            &tmp,
            entry,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::store_u64(
            data_offset,
            entry,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::subtract_registers(&tmp, segment_end, segment_start));
        self.emit(abi::store_u64(
            &tmp,
            entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers(&dst, data, data_offset));
        self.emit(abi::add_registers(&src, source_data, segment_start));
        self.emit(abi::label(&copy_segment_loop));
        self.emit(abi::compare_immediate(&tmp, "0"));
        self.emit(abi::branch_eq(&copy_segment_done));
        self.emit(abi::load_u8(&byte, &src, 0));
        self.emit(abi::store_u8(&byte, &dst, 0));
        self.emit(abi::add_immediate(&src, &src, 1));
        self.emit(abi::add_immediate(&dst, &dst, 1));
        self.emit(abi::subtract_immediate(&tmp, &tmp, 1));
        self.emit(abi::branch(&copy_segment_loop));
        self.emit(abi::label(&copy_segment_done));
        self.emit(abi::subtract_registers(&tmp, segment_end, segment_start));
        self.emit(abi::add_registers(data_offset, data_offset, &tmp));
        self.emit(abi::add_immediate(entry, entry, COLLECTION_ENTRY_SIZE));
        Ok(())
    }

    pub(super) fn lower_string_prefix_predicate(
        &mut self,
        label: &str,
        value_slot: usize,
        part_slot: usize,
        suffix: bool,
    ) -> Result<ValueResult, String> {
        let value_ptr = self.temporary_vreg();
        let part_ptr = self.temporary_vreg();
        let value_len = self.temporary_vreg();
        let part_len = self.temporary_vreg();
        let value_data = self.temporary_vreg();
        let part_data = self.temporary_vreg();
        let delta = self.temporary_vreg();
        let result_slot = self.allocate_stack_object("strings_prefix_result", 8);
        let true_label = self.label("strings_prefix_true");
        let false_label = self.label("strings_prefix_false");
        let done_label = self.label("strings_prefix_done");
        let no_match_label = self.label("strings_prefix_no_match");

        self.emit(abi::load_u64(&value_ptr, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(&part_ptr, abi::stack_pointer(), part_slot));
        self.emit(abi::load_u64(&value_len, &value_ptr, 0));
        self.emit(abi::load_u64(&part_len, &part_ptr, 0));
        self.emit(abi::compare_registers(&part_len, &value_len));
        self.emit(abi::branch_hi(&false_label));
        self.emit(abi::add_immediate(&value_data, &value_ptr, 8));
        self.emit(abi::add_immediate(&part_data, &part_ptr, 8));
        if suffix {
            self.emit(abi::subtract_registers(&delta, &value_len, &part_len));
            self.emit(abi::add_registers(&value_data, &value_data, &delta));
        }
        self.emit_string_byte_range_equal_branch(
            &value_data,
            &part_data,
            &part_len,
            &true_label,
            &no_match_label,
        );
        self.emit(abi::label(&no_match_label));
        self.emit(abi::branch(&false_label));
        self.emit_string_predicate_result(result_slot, &true_label, &false_label, &done_label);
        self.finish_string_predicate_result(label, result_slot)
    }

    pub(super) fn emit_string_predicate_result(
        &mut self,
        result_slot: usize,
        true_label: &str,
        false_label: &str,
        done_label: &str,
    ) {
        let flag = self.temporary_vreg();
        self.emit(abi::label(true_label));
        self.emit(abi::move_immediate(&flag, "Boolean", "true"));
        self.emit(abi::store_u64(&flag, abi::stack_pointer(), result_slot));
        self.emit(abi::branch(done_label));
        self.emit(abi::label(false_label));
        self.emit(abi::move_immediate(&flag, "Boolean", "false"));
        self.emit(abi::store_u64(&flag, abi::stack_pointer(), result_slot));
        self.emit(abi::label(done_label));
    }

    pub(super) fn finish_string_predicate_result(
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

    pub(super) fn emit_case_map_lookup(
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
pub(super) enum UnicodeCaseMap {
    Upper,
    Lower,
    CaseFold,
}

impl UnicodeCaseMap {
    pub(super) fn name(self) -> &'static str {
        match self {
            UnicodeCaseMap::Upper => "strings.upper",
            UnicodeCaseMap::Lower => "strings.lower",
            UnicodeCaseMap::CaseFold => "strings.caseFold",
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            UnicodeCaseMap::Upper => "strings.upper value",
            UnicodeCaseMap::Lower => "strings.lower value",
            UnicodeCaseMap::CaseFold => "strings.caseFold value",
        }
    }

    pub(super) fn slot_prefix(self) -> &'static str {
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
