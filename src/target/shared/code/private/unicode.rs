#![allow(dead_code)]

use super::super::*;

const UNICODE_PROPERTY_SIZE: usize = 24;
const UNICODE_PROPERTY_OFFSET_COMBINING_CLASS: usize = 0;
// Offsets 6/8/10 (casefold/uppercase/lowercase seqindex) exist in the on-disk
// record but are never read: case mapping uses the flattened u32 tables. The
// former `UNICODE_PROPERTY_OFFSET_*_SEQINDEX` constants and their emit helpers
// were dead and were removed (bug-70).
const UNICODE_PROPERTY_OFFSET_COMB_INDEX: usize = 12;
const UNICODE_PROPERTY_OFFSET_COMB_LENGTH: usize = 14;
const UNICODE_PROPERTY_OFFSET_FLAGS: usize = 16;
const UNICODE_PROPERTY_OFFSET_BOUNDCLASS: usize = 18;
const UNICODE_PROPERTY_OFFSET_INDIC_CONJUNCT_BREAK: usize = 20;
const UNICODE_NFD_ENTRY_SIZE: usize = 16;
const UNICODE_NFD_ENTRY_OFFSET_CODEPOINT: usize = 0;
const UNICODE_NFD_ENTRY_OFFSET_SEQUENCE_OFFSET: usize = 4;
const UNICODE_NFD_ENTRY_OFFSET_SEQUENCE_LENGTH: usize = 8;
const UNICODE_PROPERTY_FLAG_COMB_IS_SECOND: &str = "1";
const GRAPHEME_BOUNDCLASS_CR: &str = "2";
const GRAPHEME_BOUNDCLASS_LF: &str = "3";
const GRAPHEME_BOUNDCLASS_CONTROL: &str = "4";
const GRAPHEME_BOUNDCLASS_EXTEND: &str = "5";
const GRAPHEME_BOUNDCLASS_L: &str = "6";
const GRAPHEME_BOUNDCLASS_V: &str = "7";
const GRAPHEME_BOUNDCLASS_T: &str = "8";
const GRAPHEME_BOUNDCLASS_LV: &str = "9";
const GRAPHEME_BOUNDCLASS_LVT: &str = "10";
const GRAPHEME_BOUNDCLASS_REGIONAL_INDICATOR: &str = "11";
const GRAPHEME_BOUNDCLASS_SPACINGMARK: &str = "12";
const GRAPHEME_BOUNDCLASS_PREPEND: &str = "13";
const GRAPHEME_BOUNDCLASS_ZWJ: &str = "14";
const GRAPHEME_BOUNDCLASS_EXTENDED_PICTOGRAPHIC: &str = "19";
const GRAPHEME_BOUNDCLASS_E_ZWG: &str = "20";
const INDIC_CONJUNCT_BREAK_LINKER: &str = "1";
const INDIC_CONJUNCT_BREAK_CONSONANT: &str = "2";
const INDIC_CONJUNCT_BREAK_EXTEND: &str = "3";

impl CodeBuilder<'_> {
    pub(in crate::target::shared::code) fn emit_load_data_address(
        &mut self,
        register: &str,
        symbol: &str,
    ) {
        self.emit(abi::load_page_address(register, symbol));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: symbol.to_string(),
            kind: RelocIntent::DataAddrHi,
            binding: "data".to_string(),
            library: None,
        });
        self.emit(abi::add_page_offset(register, register, symbol));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: symbol.to_string(),
            kind: RelocIntent::DataAddrLo,
            binding: "data".to_string(),
            library: None,
        });
    }

    /// Decode the UTF-8 scalar at `cursor` into `codepoint`/`width`.
    ///
    /// Self-defending (audit-unicode #3): every `String` is valid UTF-8 by the
    /// ingress invariant, but this decoder no longer trusts it. Continuation
    /// bytes must be `0x80..=0xBF`, surrogates (`ED A0..`), overlongs
    /// (`C0`/`C1`, `E0 80..9F`, `F0 80..8F`) and codepoints above U+10FFFF
    /// (`F4 90..`, `F5..FF`) are rejected; any malformed sequence decodes as
    /// U+FFFD with width 1 (byte-wise resync). Each continuation byte is
    /// validated before the next is read, so a truncated tail stops at the
    /// string's NUL terminator instead of reading past the allocation, and the
    /// produced codepoint is always `<= 0x10FFFF` and never a surrogate — the
    /// two-stage property-table walk downstream is in-bounds by construction.
    /// On a valid `String` the substitution never fires, so valid strings
    /// decode exactly as before.
    pub(in crate::target::shared::code) fn emit_utf8_decode_next(
        &mut self,
        cursor: &str,
        codepoint: &str,
        width: &str,
    ) {
        let check_two = self.label("utf8_decode_check_two");
        let check_three = self.label("utf8_decode_check_three");
        let four = self.label("utf8_decode_four");
        let three_not_e0 = self.label("utf8_decode_three_not_e0");
        let three_not_ed = self.label("utf8_decode_three_not_ed");
        let four_not_f0 = self.label("utf8_decode_four_not_f0");
        let four_not_f4 = self.label("utf8_decode_four_not_f4");
        let invalid = self.label("utf8_decode_invalid");
        let done = self.label("utf8_decode_done");
        // Vreg scratch (was physical `x6`/`x7`): on x86 the ABI-argument names
        // `x4`-`x7` collapse together (both fall to `rax` via selection's None
        // fallback), so `and %byte,%byte,%mask` became `and rax,rax,rax` — the
        // continuation-byte mask was dropped and the codepoint decoded wrong.
        let byte = self.temporary_vreg();
        let byte2 = self.temporary_vreg();
        let byte3 = self.temporary_vreg();
        let masked = self.temporary_vreg();
        let mask = self.temporary_vreg();
        let byte = byte.as_str();
        let byte2 = byte2.as_str();
        let byte3 = byte3.as_str();
        let masked = masked.as_str();
        let mask = mask.as_str();

        self.emit(abi::load_u8(codepoint, cursor, 0));
        self.emit(abi::compare_immediate(codepoint, "128"));
        self.emit(abi::branch_ge(&check_two));
        self.emit(abi::move_immediate(width, "Integer", "1"));
        self.emit(abi::branch(&done));

        self.emit(abi::label(&check_two));
        // 0x80..0xC1: stray continuation byte or overlong two-byte lead.
        self.emit(abi::compare_immediate(codepoint, "194"));
        self.emit(abi::branch_lt(&invalid));
        self.emit(abi::compare_immediate(codepoint, "224"));
        self.emit(abi::branch_ge(&check_three));
        self.emit(abi::load_u8(byte, cursor, 1));
        self.emit(abi::move_immediate(mask, "Integer", "192"));
        self.emit(abi::and_registers(masked, byte, mask));
        self.emit(abi::compare_immediate(masked, "128"));
        self.emit(abi::branch_ne(&invalid));
        self.emit(abi::move_immediate(width, "Integer", "2"));
        self.emit(abi::move_immediate(masked, "Integer", "31"));
        self.emit(abi::and_registers(codepoint, codepoint, masked));
        self.emit(abi::shift_left_immediate(codepoint, codepoint, 6));
        self.emit(abi::move_immediate(mask, "Integer", "63"));
        self.emit(abi::and_registers(byte, byte, mask));
        self.emit(abi::or_registers(codepoint, codepoint, byte));
        self.emit(abi::branch(&done));

        self.emit(abi::label(&check_three));
        self.emit(abi::compare_immediate(codepoint, "240"));
        self.emit(abi::branch_ge(&four));
        self.emit(abi::load_u8(byte, cursor, 1));
        self.emit(abi::move_immediate(mask, "Integer", "192"));
        self.emit(abi::and_registers(masked, byte, mask));
        self.emit(abi::compare_immediate(masked, "128"));
        self.emit(abi::branch_ne(&invalid));
        // E0: second byte must be 0xA0..0xBF (reject overlongs).
        self.emit(abi::compare_immediate(codepoint, "224"));
        self.emit(abi::branch_ne(&three_not_e0));
        self.emit(abi::compare_immediate(byte, "160"));
        self.emit(abi::branch_lt(&invalid));
        self.emit(abi::label(&three_not_e0));
        // ED: second byte must be 0x80..0x9F (reject surrogates D800..DFFF).
        self.emit(abi::compare_immediate(codepoint, "237"));
        self.emit(abi::branch_ne(&three_not_ed));
        self.emit(abi::compare_immediate(byte, "160"));
        self.emit(abi::branch_ge(&invalid));
        self.emit(abi::label(&three_not_ed));
        self.emit(abi::load_u8(byte2, cursor, 2));
        self.emit(abi::and_registers(masked, byte2, mask));
        self.emit(abi::compare_immediate(masked, "128"));
        self.emit(abi::branch_ne(&invalid));
        self.emit(abi::move_immediate(width, "Integer", "3"));
        self.emit(abi::move_immediate(masked, "Integer", "15"));
        self.emit(abi::and_registers(codepoint, codepoint, masked));
        self.emit(abi::shift_left_immediate(codepoint, codepoint, 12));
        self.emit(abi::move_immediate(mask, "Integer", "63"));
        self.emit(abi::and_registers(byte, byte, mask));
        self.emit(abi::shift_left_immediate(byte, byte, 6));
        self.emit(abi::or_registers(codepoint, codepoint, byte));
        self.emit(abi::and_registers(byte2, byte2, mask));
        self.emit(abi::or_registers(codepoint, codepoint, byte2));
        self.emit(abi::branch(&done));

        self.emit(abi::label(&four));
        // 0xF5..0xFF: leads beyond U+10FFFF.
        self.emit(abi::compare_immediate(codepoint, "245"));
        self.emit(abi::branch_ge(&invalid));
        self.emit(abi::load_u8(byte, cursor, 1));
        self.emit(abi::move_immediate(mask, "Integer", "192"));
        self.emit(abi::and_registers(masked, byte, mask));
        self.emit(abi::compare_immediate(masked, "128"));
        self.emit(abi::branch_ne(&invalid));
        // F0: second byte must be 0x90..0xBF (reject overlongs).
        self.emit(abi::compare_immediate(codepoint, "240"));
        self.emit(abi::branch_ne(&four_not_f0));
        self.emit(abi::compare_immediate(byte, "144"));
        self.emit(abi::branch_lt(&invalid));
        self.emit(abi::label(&four_not_f0));
        // F4: second byte must be 0x80..0x8F (reject > U+10FFFF).
        self.emit(abi::compare_immediate(codepoint, "244"));
        self.emit(abi::branch_ne(&four_not_f4));
        self.emit(abi::compare_immediate(byte, "144"));
        self.emit(abi::branch_ge(&invalid));
        self.emit(abi::label(&four_not_f4));
        self.emit(abi::load_u8(byte2, cursor, 2));
        self.emit(abi::and_registers(masked, byte2, mask));
        self.emit(abi::compare_immediate(masked, "128"));
        self.emit(abi::branch_ne(&invalid));
        self.emit(abi::load_u8(byte3, cursor, 3));
        self.emit(abi::and_registers(masked, byte3, mask));
        self.emit(abi::compare_immediate(masked, "128"));
        self.emit(abi::branch_ne(&invalid));
        self.emit(abi::move_immediate(width, "Integer", "4"));
        self.emit(abi::move_immediate(masked, "Integer", "7"));
        self.emit(abi::and_registers(codepoint, codepoint, masked));
        self.emit(abi::shift_left_immediate(codepoint, codepoint, 18));
        self.emit(abi::move_immediate(mask, "Integer", "63"));
        self.emit(abi::and_registers(byte, byte, mask));
        self.emit(abi::shift_left_immediate(byte, byte, 12));
        self.emit(abi::or_registers(codepoint, codepoint, byte));
        self.emit(abi::and_registers(byte2, byte2, mask));
        self.emit(abi::shift_left_immediate(byte2, byte2, 6));
        self.emit(abi::or_registers(codepoint, codepoint, byte2));
        self.emit(abi::and_registers(byte3, byte3, mask));
        self.emit(abi::or_registers(codepoint, codepoint, byte3));
        self.emit(abi::branch(&done));

        self.emit(abi::label(&invalid));
        // Substitute U+FFFD and resync one byte; unreachable on a valid String.
        self.emit(abi::move_immediate(codepoint, "Integer", "65533"));
        self.emit(abi::move_immediate(width, "Integer", "1"));
        self.emit(abi::label(&done));
    }

    pub(in crate::target::shared::code) fn emit_unicode_property_lookup(
        &mut self,
        codepoint: &str,
        property: &str,
    ) {
        let x6 = self.temporary_vreg();
        let x7 = self.temporary_vreg();
        let x6 = x6.as_str();
        let x7 = x7.as_str();
        self.emit(abi::shift_right_immediate(x6, codepoint, 8));
        self.emit(abi::shift_left_immediate(x6, x6, 1));
        self.emit_load_data_address(x7, UNICODE_STAGE1_SYMBOL);
        self.emit(abi::add_registers(x7, x7, x6));
        self.emit(abi::load_u16(x6, x7, 0));
        self.emit(abi::move_immediate(x7, "Integer", "255"));
        self.emit(abi::and_registers(x7, codepoint, x7));
        self.emit(abi::add_registers(x6, x6, x7));
        self.emit(abi::shift_left_immediate(x6, x6, 1));
        self.emit_load_data_address(x7, UNICODE_STAGE2_SYMBOL);
        self.emit(abi::add_registers(x7, x7, x6));
        self.emit(abi::load_u16(x6, x7, 0));
        self.emit(abi::move_immediate(
            x7,
            "Integer",
            &UNICODE_PROPERTY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(x6, x6, x7));
        self.emit_load_data_address(property, UNICODE_PROPERTIES_SYMBOL);
        self.emit(abi::add_registers(property, property, x6));
    }

    pub(in crate::target::shared::code) fn emit_unicode_property_boundclass(
        &mut self,
        property: &str,
        output: &str,
    ) {
        self.emit(abi::load_u16(
            output,
            property,
            UNICODE_PROPERTY_OFFSET_BOUNDCLASS,
        ));
    }

    pub(in crate::target::shared::code) fn emit_unicode_property_u16(
        &mut self,
        property: &str,
        output: &str,
        offset: usize,
    ) {
        self.emit(abi::load_u16(output, property, offset));
    }

    pub(in crate::target::shared::code) fn emit_unicode_property_combining_class(
        &mut self,
        property: &str,
        output: &str,
    ) {
        self.emit_unicode_property_u16(property, output, UNICODE_PROPERTY_OFFSET_COMBINING_CLASS);
    }

    pub(in crate::target::shared::code) fn emit_unicode_property_comb_index(
        &mut self,
        property: &str,
        output: &str,
    ) {
        self.emit_unicode_property_u16(property, output, UNICODE_PROPERTY_OFFSET_COMB_INDEX);
    }

    pub(in crate::target::shared::code) fn emit_unicode_property_comb_length(
        &mut self,
        property: &str,
        output: &str,
    ) {
        self.emit_unicode_property_u16(property, output, UNICODE_PROPERTY_OFFSET_COMB_LENGTH);
    }

    pub(in crate::target::shared::code) fn emit_unicode_property_flags(
        &mut self,
        property: &str,
        output: &str,
    ) {
        self.emit_unicode_property_u16(property, output, UNICODE_PROPERTY_OFFSET_FLAGS);
    }

    pub(in crate::target::shared::code) fn emit_unicode_property_indic_conjunct_break(
        &mut self,
        property: &str,
        output: &str,
    ) {
        self.emit(abi::load_u16(
            output,
            property,
            UNICODE_PROPERTY_OFFSET_INDIC_CONJUNCT_BREAK,
        ));
    }

    pub(in crate::target::shared::code) fn emit_utf8_encoded_width(
        &mut self,
        codepoint: &str,
        width: &str,
    ) {
        let two = self.label("utf8_width_two");
        let three = self.label("utf8_width_three");
        let four = self.label("utf8_width_four");
        let done = self.label("utf8_width_done");
        let x6 = self.temporary_vreg();
        let x6 = x6.as_str();

        self.emit(abi::compare_immediate(codepoint, "128"));
        self.emit(abi::branch_ge(&two));
        self.emit(abi::move_immediate(width, "Integer", "1"));
        self.emit(abi::branch(&done));
        self.emit(abi::label(&two));
        self.emit(abi::compare_immediate(codepoint, "2048"));
        self.emit(abi::branch_ge(&three));
        self.emit(abi::move_immediate(width, "Integer", "2"));
        self.emit(abi::branch(&done));
        self.emit(abi::label(&three));
        self.emit(abi::move_immediate(x6, "Integer", "65536"));
        self.emit(abi::compare_registers(codepoint, x6));
        self.emit(abi::branch_ge(&four));
        self.emit(abi::move_immediate(width, "Integer", "3"));
        self.emit(abi::branch(&done));
        self.emit(abi::label(&four));
        self.emit(abi::move_immediate(width, "Integer", "4"));
        self.emit(abi::label(&done));
    }

    pub(in crate::target::shared::code) fn emit_utf8_encode_next(
        &mut self,
        cursor: &str,
        codepoint: &str,
    ) {
        let two = self.label("utf8_encode_two");
        let three = self.label("utf8_encode_three");
        let four = self.label("utf8_encode_four");
        let done = self.label("utf8_encode_done");
        let x6 = self.temporary_vreg();
        let x7 = self.temporary_vreg();
        let x6 = x6.as_str();
        let x7 = x7.as_str();

        self.emit(abi::compare_immediate(codepoint, "128"));
        self.emit(abi::branch_ge(&two));
        self.emit(abi::store_u8(codepoint, cursor, 0));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::branch(&done));

        self.emit(abi::label(&two));
        self.emit(abi::compare_immediate(codepoint, "2048"));
        self.emit(abi::branch_ge(&three));
        self.emit(abi::shift_right_immediate(x6, codepoint, 6));
        self.emit(abi::move_immediate(x7, "Integer", "192"));
        self.emit(abi::or_registers(x6, x6, x7));
        self.emit(abi::store_u8(x6, cursor, 0));
        self.emit(abi::move_immediate(x7, "Integer", "63"));
        self.emit(abi::and_registers(x6, codepoint, x7));
        self.emit(abi::move_immediate(x7, "Integer", "128"));
        self.emit(abi::or_registers(x6, x6, x7));
        self.emit(abi::store_u8(x6, cursor, 1));
        self.emit(abi::add_immediate(cursor, cursor, 2));
        self.emit(abi::branch(&done));

        self.emit(abi::label(&three));
        self.emit(abi::move_immediate(x6, "Integer", "65536"));
        self.emit(abi::compare_registers(codepoint, x6));
        self.emit(abi::branch_ge(&four));
        self.emit(abi::shift_right_immediate(x6, codepoint, 12));
        self.emit(abi::move_immediate(x7, "Integer", "224"));
        self.emit(abi::or_registers(x6, x6, x7));
        self.emit(abi::store_u8(x6, cursor, 0));
        self.emit(abi::shift_right_immediate(x6, codepoint, 6));
        self.emit(abi::move_immediate(x7, "Integer", "63"));
        self.emit(abi::and_registers(x6, x6, x7));
        self.emit(abi::move_immediate(x7, "Integer", "128"));
        self.emit(abi::or_registers(x6, x6, x7));
        self.emit(abi::store_u8(x6, cursor, 1));
        self.emit(abi::move_immediate(x7, "Integer", "63"));
        self.emit(abi::and_registers(x6, codepoint, x7));
        self.emit(abi::move_immediate(x7, "Integer", "128"));
        self.emit(abi::or_registers(x6, x6, x7));
        self.emit(abi::store_u8(x6, cursor, 2));
        self.emit(abi::add_immediate(cursor, cursor, 3));
        self.emit(abi::branch(&done));

        self.emit(abi::label(&four));
        self.emit(abi::shift_right_immediate(x6, codepoint, 18));
        self.emit(abi::move_immediate(x7, "Integer", "240"));
        self.emit(abi::or_registers(x6, x6, x7));
        self.emit(abi::store_u8(x6, cursor, 0));
        self.emit(abi::shift_right_immediate(x6, codepoint, 12));
        self.emit(abi::move_immediate(x7, "Integer", "63"));
        self.emit(abi::and_registers(x6, x6, x7));
        self.emit(abi::move_immediate(x7, "Integer", "128"));
        self.emit(abi::or_registers(x6, x6, x7));
        self.emit(abi::store_u8(x6, cursor, 1));
        self.emit(abi::shift_right_immediate(x6, codepoint, 6));
        self.emit(abi::move_immediate(x7, "Integer", "63"));
        self.emit(abi::and_registers(x6, x6, x7));
        self.emit(abi::move_immediate(x7, "Integer", "128"));
        self.emit(abi::or_registers(x6, x6, x7));
        self.emit(abi::store_u8(x6, cursor, 2));
        self.emit(abi::move_immediate(x7, "Integer", "63"));
        self.emit(abi::and_registers(x6, codepoint, x7));
        self.emit(abi::move_immediate(x7, "Integer", "128"));
        self.emit(abi::or_registers(x6, x6, x7));
        self.emit(abi::store_u8(x6, cursor, 3));
        self.emit(abi::add_immediate(cursor, cursor, 4));
        self.emit(abi::label(&done));
    }

    pub(in crate::target::shared::code) fn emit_unicode_u32_mapping_lookup(
        &mut self,
        codepoint: &str,
        entries_symbol: &str,
        entry_count: usize,
        sequences_symbol: &str,
        sequence_ptr: &str,
        sequence_length: &str,
    ) {
        let loop_label = self.label("unicode_mapping_lookup_loop");
        let move_left = self.label("unicode_mapping_lookup_left");
        let found = self.label("unicode_mapping_lookup_found");
        let not_found = self.label("unicode_mapping_lookup_not_found");
        let done = self.label("unicode_mapping_lookup_done");
        let x6 = self.temporary_vreg();
        let x7 = self.temporary_vreg();
        let x6 = x6.as_str();
        let x7 = x7.as_str();
        // Binary-search scratch as vregs (was hand-pinned x8/x9/x13/x14).
        let mid_v = self.temporary_vreg();
        let offset_v = self.temporary_vreg();
        let entry_ptr_v = self.temporary_vreg();
        let field_v = self.temporary_vreg();
        let mid = mid_v.as_str();
        let offset = offset_v.as_str();
        let entry_ptr = entry_ptr_v.as_str();
        let field = field_v.as_str();

        self.emit(abi::move_immediate(x6, "Integer", "0"));
        self.emit(abi::move_immediate(x7, "Integer", &entry_count.to_string()));
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_registers(x6, x7));
        self.emit(abi::branch_ge(&not_found));
        self.emit(abi::add_registers(mid, x6, x7));
        self.emit(abi::shift_right_immediate(mid, mid, 1));
        self.emit(abi::shift_left_immediate(offset, mid, 4));
        self.emit_load_data_address(entry_ptr, entries_symbol);
        self.emit(abi::add_registers(entry_ptr, entry_ptr, offset));
        self.emit(abi::load_u32(
            field,
            entry_ptr,
            UNICODE_NFD_ENTRY_OFFSET_CODEPOINT,
        ));
        self.emit(abi::compare_registers(field, codepoint));
        self.emit(abi::branch_eq(&found));
        self.emit(abi::branch_lo(&move_left));
        self.emit(abi::move_register(x7, mid));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&move_left));
        self.emit(abi::add_immediate(x6, mid, 1));
        self.emit(abi::branch(&loop_label));

        self.emit(abi::label(&found));
        self.emit(abi::load_u32(
            field,
            entry_ptr,
            UNICODE_NFD_ENTRY_OFFSET_SEQUENCE_OFFSET,
        ));
        self.emit(abi::load_u32(
            sequence_length,
            entry_ptr,
            UNICODE_NFD_ENTRY_OFFSET_SEQUENCE_LENGTH,
        ));
        self.emit(abi::shift_left_immediate(field, field, 2));
        self.emit_load_data_address(sequence_ptr, sequences_symbol);
        self.emit(abi::add_registers(sequence_ptr, sequence_ptr, field));
        self.emit(abi::branch(&done));

        self.emit(abi::label(&not_found));
        self.emit(abi::move_immediate(sequence_length, "Integer", "0"));
        self.emit(abi::label(&done));
    }

    pub(in crate::target::shared::code) fn emit_hangul_composition_attempt(
        &mut self,
        starter: &str,
        current: &str,
        output: &str,
        found_label: &str,
        fallback_label: &str,
    ) {
        let check_lv_t = self.label("hangul_compose_check_lv_t");
        let x6 = self.temporary_vreg();
        let x7 = self.temporary_vreg();
        let x6 = x6.as_str();
        let x7 = x7.as_str();
        // Scratch as a vreg (was hand-pinned physical `x8`): a raw `x8` write is
        // invisible to the register allocator, so a caller value it placed in
        // `x8` was silently clobbered — a layout-sensitive miscompile.
        let x8 = self.temporary_vreg();
        let x8 = x8.as_str();

        self.emit(abi::move_immediate(x6, "Integer", "4352"));
        self.emit(abi::compare_registers(starter, x6));
        self.emit(abi::branch_lo(&check_lv_t));
        self.emit(abi::subtract_registers(x7, starter, x6));
        self.emit(abi::compare_immediate(x7, "19"));
        self.emit(abi::branch_ge(&check_lv_t));

        self.emit(abi::move_immediate(x6, "Integer", "4449"));
        self.emit(abi::compare_registers(current, x6));
        self.emit(abi::branch_lo(&check_lv_t));
        self.emit(abi::subtract_registers(x8, current, x6));
        self.emit(abi::compare_immediate(x8, "21"));
        self.emit(abi::branch_ge(&check_lv_t));
        self.emit(abi::move_immediate(x6, "Integer", "21"));
        self.emit(abi::multiply_registers(output, x7, x6));
        self.emit(abi::add_registers(output, output, x8));
        self.emit(abi::move_immediate(x6, "Integer", "28"));
        self.emit(abi::multiply_registers(output, output, x6));
        self.emit(abi::move_immediate(x6, "Integer", "44032"));
        self.emit(abi::add_registers(output, output, x6));
        self.emit(abi::branch(found_label));

        self.emit(abi::label(&check_lv_t));
        self.emit(abi::move_immediate(x6, "Integer", "44032"));
        self.emit(abi::compare_registers(starter, x6));
        self.emit(abi::branch_lo(fallback_label));
        self.emit(abi::subtract_registers(x7, starter, x6));
        self.emit(abi::move_immediate(x6, "Integer", "11172"));
        self.emit(abi::compare_registers(x7, x6));
        self.emit(abi::branch_ge(fallback_label));
        self.emit(abi::move_immediate(x6, "Integer", "28"));
        self.emit(abi::unsigned_divide_registers(x8, x7, x6));
        self.emit(abi::multiply_subtract_registers(x8, x8, x6, x7));
        self.emit(abi::compare_immediate(x8, "0"));
        self.emit(abi::branch_ne(fallback_label));
        self.emit(abi::move_immediate(x6, "Integer", "4519"));
        self.emit(abi::compare_registers(current, x6));
        self.emit(abi::branch_lo(fallback_label));
        self.emit(abi::subtract_registers(x8, current, x6));
        self.emit(abi::compare_immediate(x8, "0"));
        self.emit(abi::branch_eq(fallback_label));
        self.emit(abi::compare_immediate(x8, "28"));
        self.emit(abi::branch_ge(fallback_label));
        self.emit(abi::add_registers(output, starter, x8));
        self.emit(abi::branch(found_label));
    }

    pub(in crate::target::shared::code) fn emit_grapheme_break_branch(
        &mut self,
        state_bc: &str,
        state_icb: &str,
        current_bc: &str,
        current_icb: &str,
        break_label: &str,
        no_break_label: &str,
    ) {
        let no_break = self.label("grapheme_simple_no_break");
        let maybe_break = self.label("grapheme_maybe_break");
        let gb3_not_cr = self.label("grapheme_gb3_not_cr");
        let gb4_not_control = self.label("grapheme_gb4_not_control");
        let gb5_not_control = self.label("grapheme_gb5_not_control");
        let gb6_check = self.label("grapheme_gb6_check");
        let gb7_check = self.label("grapheme_gb7_check");
        let gb7_no = self.label("grapheme_gb7_no");
        let gb8_check = self.label("grapheme_gb8_check");
        let gb8_no = self.label("grapheme_gb8_no");
        let gb9_check = self.label("grapheme_gb9_check");
        let gb11_check = self.label("grapheme_gb11_check");
        let gb1213_check = self.label("grapheme_gb1213_check");
        let gb9c_break = self.label("grapheme_gb9c_break");

        self.emit(abi::compare_immediate(state_bc, GRAPHEME_BOUNDCLASS_CR));
        self.emit(abi::branch_ne(&gb3_not_cr));
        self.emit(abi::compare_immediate(current_bc, GRAPHEME_BOUNDCLASS_LF));
        self.emit(abi::branch_eq(&no_break));
        self.emit(abi::label(&gb3_not_cr));

        self.emit(abi::compare_immediate(state_bc, GRAPHEME_BOUNDCLASS_CR));
        self.emit(abi::branch_lo(&gb4_not_control));
        self.emit(abi::compare_immediate(
            state_bc,
            GRAPHEME_BOUNDCLASS_CONTROL,
        ));
        self.emit(abi::branch_le(&maybe_break));
        self.emit(abi::label(&gb4_not_control));

        self.emit(abi::compare_immediate(current_bc, GRAPHEME_BOUNDCLASS_CR));
        self.emit(abi::branch_lo(&gb5_not_control));
        self.emit(abi::compare_immediate(
            current_bc,
            GRAPHEME_BOUNDCLASS_CONTROL,
        ));
        self.emit(abi::branch_le(&maybe_break));
        self.emit(abi::label(&gb5_not_control));

        self.emit(abi::compare_immediate(state_bc, GRAPHEME_BOUNDCLASS_L));
        self.emit(abi::branch_ne(&gb6_check));
        self.emit(abi::compare_immediate(current_bc, GRAPHEME_BOUNDCLASS_L));
        self.emit(abi::branch_eq(&no_break));
        self.emit(abi::compare_immediate(current_bc, GRAPHEME_BOUNDCLASS_V));
        self.emit(abi::branch_eq(&no_break));
        self.emit(abi::compare_immediate(current_bc, GRAPHEME_BOUNDCLASS_LV));
        self.emit(abi::branch_eq(&no_break));
        self.emit(abi::compare_immediate(current_bc, GRAPHEME_BOUNDCLASS_LVT));
        self.emit(abi::branch_eq(&no_break));
        self.emit(abi::label(&gb6_check));

        self.emit(abi::compare_immediate(state_bc, GRAPHEME_BOUNDCLASS_LV));
        self.emit(abi::branch_eq(&gb7_check));
        self.emit(abi::compare_immediate(state_bc, GRAPHEME_BOUNDCLASS_V));
        self.emit(abi::branch_ne(&gb7_no));
        self.emit(abi::label(&gb7_check));
        self.emit(abi::compare_immediate(current_bc, GRAPHEME_BOUNDCLASS_V));
        self.emit(abi::branch_eq(&no_break));
        self.emit(abi::compare_immediate(current_bc, GRAPHEME_BOUNDCLASS_T));
        self.emit(abi::branch_eq(&no_break));
        self.emit(abi::label(&gb7_no));

        self.emit(abi::compare_immediate(state_bc, GRAPHEME_BOUNDCLASS_LVT));
        self.emit(abi::branch_eq(&gb8_check));
        self.emit(abi::compare_immediate(state_bc, GRAPHEME_BOUNDCLASS_T));
        self.emit(abi::branch_ne(&gb8_no));
        self.emit(abi::label(&gb8_check));
        self.emit(abi::compare_immediate(current_bc, GRAPHEME_BOUNDCLASS_T));
        self.emit(abi::branch_eq(&no_break));
        self.emit(abi::label(&gb8_no));

        self.emit(abi::label(&gb9_check));
        self.emit(abi::compare_immediate(
            current_bc,
            GRAPHEME_BOUNDCLASS_EXTEND,
        ));
        self.emit(abi::branch_eq(&no_break));
        self.emit(abi::compare_immediate(current_bc, GRAPHEME_BOUNDCLASS_ZWJ));
        self.emit(abi::branch_eq(&no_break));
        self.emit(abi::compare_immediate(
            current_bc,
            GRAPHEME_BOUNDCLASS_SPACINGMARK,
        ));
        self.emit(abi::branch_eq(&no_break));
        self.emit(abi::compare_immediate(
            state_bc,
            GRAPHEME_BOUNDCLASS_PREPEND,
        ));
        self.emit(abi::branch_eq(&no_break));

        self.emit(abi::label(&gb11_check));
        self.emit(abi::compare_immediate(state_bc, GRAPHEME_BOUNDCLASS_E_ZWG));
        self.emit(abi::branch_ne(&gb1213_check));
        self.emit(abi::compare_immediate(
            current_bc,
            GRAPHEME_BOUNDCLASS_EXTENDED_PICTOGRAPHIC,
        ));
        self.emit(abi::branch_eq(&no_break));

        self.emit(abi::label(&gb1213_check));
        self.emit(abi::compare_immediate(
            state_bc,
            GRAPHEME_BOUNDCLASS_REGIONAL_INDICATOR,
        ));
        self.emit(abi::branch_ne(&maybe_break));
        self.emit(abi::compare_immediate(
            current_bc,
            GRAPHEME_BOUNDCLASS_REGIONAL_INDICATOR,
        ));
        self.emit(abi::branch_eq(&no_break));

        self.emit(abi::label(&maybe_break));
        self.emit(abi::compare_immediate(
            state_icb,
            INDIC_CONJUNCT_BREAK_LINKER,
        ));
        self.emit(abi::branch_ne(&gb9c_break));
        self.emit(abi::compare_immediate(
            current_icb,
            INDIC_CONJUNCT_BREAK_CONSONANT,
        ));
        self.emit(abi::branch_eq(&no_break));
        self.emit(abi::label(&gb9c_break));
        self.emit(abi::branch(break_label));

        self.emit(abi::label(&no_break));
        self.emit(abi::branch(no_break_label));
    }

    pub(in crate::target::shared::code) fn emit_grapheme_state_update(
        &mut self,
        state_bc: &str,
        state_icb: &str,
        current_bc: &str,
        current_icb: &str,
    ) {
        let icb_consonant = self.label("grapheme_icb_consonant");
        let icb_existing_consonant = self.label("grapheme_icb_existing_consonant");
        let icb_existing_extend = self.label("grapheme_icb_existing_extend");
        let icb_linker = self.label("grapheme_icb_linker");
        let icb_linker_extend = self.label("grapheme_icb_linker_extend");
        let icb_done = self.label("grapheme_icb_done");
        let bc_ri_check = self.label("grapheme_bc_ri_check");
        let bc_extpic_check = self.label("grapheme_bc_extpic_check");
        let bc_extpic_extend = self.label("grapheme_bc_extpic_extend");
        let bc_extpic_zwj = self.label("grapheme_bc_extpic_zwj");
        let bc_set_current = self.label("grapheme_bc_set_current");
        let bc_done = self.label("grapheme_bc_done");

        self.emit(abi::compare_immediate(
            current_icb,
            INDIC_CONJUNCT_BREAK_CONSONANT,
        ));
        self.emit(abi::branch_eq(&icb_consonant));
        self.emit(abi::compare_immediate(
            state_icb,
            INDIC_CONJUNCT_BREAK_CONSONANT,
        ));
        self.emit(abi::branch_eq(&icb_existing_consonant));
        self.emit(abi::compare_immediate(
            state_icb,
            INDIC_CONJUNCT_BREAK_EXTEND,
        ));
        self.emit(abi::branch_eq(&icb_existing_extend));
        self.emit(abi::compare_immediate(
            state_icb,
            INDIC_CONJUNCT_BREAK_LINKER,
        ));
        self.emit(abi::branch_eq(&icb_linker));
        self.emit(abi::branch(&icb_done));
        self.emit(abi::label(&icb_consonant));
        self.emit(abi::move_register(state_icb, current_icb));
        self.emit(abi::branch(&icb_done));
        self.emit(abi::label(&icb_existing_consonant));
        self.emit(abi::move_register(state_icb, current_icb));
        self.emit(abi::branch(&icb_done));
        self.emit(abi::label(&icb_existing_extend));
        self.emit(abi::move_register(state_icb, current_icb));
        self.emit(abi::branch(&icb_done));
        self.emit(abi::label(&icb_linker));
        self.emit(abi::compare_immediate(
            current_icb,
            INDIC_CONJUNCT_BREAK_EXTEND,
        ));
        self.emit(abi::branch_eq(&icb_linker_extend));
        self.emit(abi::move_register(state_icb, current_icb));
        self.emit(abi::branch(&icb_done));
        self.emit(abi::label(&icb_linker_extend));
        self.emit(abi::move_immediate(
            state_icb,
            "Integer",
            INDIC_CONJUNCT_BREAK_LINKER,
        ));
        self.emit(abi::label(&icb_done));

        self.emit(abi::compare_registers(state_bc, current_bc));
        self.emit(abi::branch_ne(&bc_extpic_check));
        self.emit(abi::compare_immediate(
            current_bc,
            GRAPHEME_BOUNDCLASS_REGIONAL_INDICATOR,
        ));
        self.emit(abi::branch_eq(&bc_ri_check));
        self.emit(abi::label(&bc_extpic_check));
        self.emit(abi::compare_immediate(
            state_bc,
            GRAPHEME_BOUNDCLASS_EXTENDED_PICTOGRAPHIC,
        ));
        self.emit(abi::branch_ne(&bc_set_current));
        self.emit(abi::compare_immediate(
            current_bc,
            GRAPHEME_BOUNDCLASS_EXTEND,
        ));
        self.emit(abi::branch_eq(&bc_extpic_extend));
        self.emit(abi::compare_immediate(current_bc, GRAPHEME_BOUNDCLASS_ZWJ));
        self.emit(abi::branch_eq(&bc_extpic_zwj));
        self.emit(abi::branch(&bc_set_current));
        self.emit(abi::label(&bc_ri_check));
        self.emit(abi::move_immediate(state_bc, "Integer", "1"));
        self.emit(abi::branch(&bc_done));
        self.emit(abi::label(&bc_extpic_extend));
        self.emit(abi::move_immediate(
            state_bc,
            "Integer",
            GRAPHEME_BOUNDCLASS_EXTENDED_PICTOGRAPHIC,
        ));
        self.emit(abi::branch(&bc_done));
        self.emit(abi::label(&bc_extpic_zwj));
        self.emit(abi::move_immediate(
            state_bc,
            "Integer",
            GRAPHEME_BOUNDCLASS_E_ZWG,
        ));
        self.emit(abi::branch(&bc_done));
        self.emit(abi::label(&bc_set_current));
        self.emit(abi::move_register(state_bc, current_bc));
        self.emit(abi::label(&bc_done));
    }

    pub(in crate::target::shared::code) fn emit_string_byte_range_equal_branch(
        &mut self,
        left_data: &str,
        right_data: &str,
        length: &str,
        equal_label: &str,
        not_equal_label: &str,
    ) {
        let loop_label = self.label("string_bytes_equal_loop");
        let x4 = self.temporary_vreg();
        let x5 = self.temporary_vreg();
        let x6 = self.temporary_vreg();
        let x7 = self.temporary_vreg();
        let x4 = x4.as_str();
        let x5 = x5.as_str();
        let x6 = x6.as_str();
        let x7 = x7.as_str();
        // Byte-compare scratch as a vreg (was hand-pinned physical `x8`). This
        // helper backs `String` equality and every substring predicate, so a
        // raw `x8` here clobbered a caller value the allocator had placed in
        // `x8` under register pressure — a layout-sensitive miscompile that
        // corrupted adjacent string comparisons.
        let x8 = self.temporary_vreg();
        let x8 = x8.as_str();
        self.emit(abi::move_register(x4, left_data));
        self.emit(abi::move_register(x5, right_data));
        self.emit(abi::move_register(x6, length));
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate(x6, "0"));
        self.emit(abi::branch_eq(equal_label));
        self.emit(abi::load_u8(x7, x4, 0));
        self.emit(abi::load_u8(x8, x5, 0));
        self.emit(abi::compare_registers(x7, x8));
        self.emit(abi::branch_ne(not_equal_label));
        self.emit(abi::add_immediate(x4, x4, 1));
        self.emit(abi::add_immediate(x5, x5, 1));
        self.emit(abi::subtract_immediate(x6, x6, 1));
        self.emit(abi::branch(&loop_label));
    }

    pub(in crate::target::shared::code) fn emit_unicode_whitespace_branch(
        &mut self,
        cursor: &str,
        remaining: &str,
        width: &str,
        whitespace_label: &str,
        not_whitespace_label: &str,
    ) {
        let x7 = self.temporary_vreg();
        let x7 = x7.as_str();
        // Continuation-byte scratch as a vreg (was hand-pinned physical `x8`): a
        // raw `x8` write is invisible to the register allocator and clobbered a
        // caller value under register pressure — a layout-sensitive miscompile.
        let x8 = self.temporary_vreg();
        let x8 = x8.as_str();
        let check_c2 = self.label("unicode_ws_check_c2");
        let check_e1 = self.label("unicode_ws_check_e1");
        let check_e2 = self.label("unicode_ws_check_e2");
        let check_e3 = self.label("unicode_ws_check_e3");
        let one = self.label("unicode_ws_one");
        let two = self.label("unicode_ws_two");
        let three = self.label("unicode_ws_three");
        let e2_80 = self.label("unicode_ws_e2_80");
        let e2_81 = self.label("unicode_ws_e2_81");
        let e2_80_range = self.label("unicode_ws_e2_80_range");
        let e2_80_check_a8 = self.label("unicode_ws_e2_80_check_a8");
        let e2_80_check_a9 = self.label("unicode_ws_e2_80_check_a9");
        let e2_80_check_af = self.label("unicode_ws_e2_80_check_af");

        self.emit(abi::compare_immediate(remaining, "0"));
        self.emit(abi::branch_eq(not_whitespace_label));
        self.emit(abi::load_u8(x7, cursor, 0));
        self.emit(abi::compare_immediate(x7, "9"));
        self.emit(abi::branch_lo(&check_c2));
        self.emit(abi::compare_immediate(x7, "13"));
        self.emit(abi::branch_le(&one));
        self.emit(abi::compare_immediate(x7, "32"));
        self.emit(abi::branch_eq(&one));

        self.emit(abi::label(&check_c2));
        self.emit(abi::compare_immediate(x7, "194"));
        self.emit(abi::branch_ne(&check_e1));
        self.emit(abi::compare_immediate(remaining, "2"));
        self.emit(abi::branch_lo(not_whitespace_label));
        self.emit(abi::load_u8(x8, cursor, 1));
        self.emit(abi::compare_immediate(x8, "133"));
        self.emit(abi::branch_eq(&two));
        self.emit(abi::compare_immediate(x8, "160"));
        self.emit(abi::branch_eq(&two));
        self.emit(abi::branch(not_whitespace_label));

        self.emit(abi::label(&check_e1));
        self.emit(abi::compare_immediate(x7, "225"));
        self.emit(abi::branch_ne(&check_e2));
        self.emit(abi::compare_immediate(remaining, "3"));
        self.emit(abi::branch_lo(not_whitespace_label));
        self.emit(abi::load_u8(x8, cursor, 1));
        self.emit(abi::compare_immediate(x8, "154"));
        self.emit(abi::branch_ne(not_whitespace_label));
        self.emit(abi::load_u8(x8, cursor, 2));
        self.emit(abi::compare_immediate(x8, "128"));
        self.emit(abi::branch_eq(&three));
        self.emit(abi::branch(not_whitespace_label));

        self.emit(abi::label(&check_e2));
        self.emit(abi::compare_immediate(x7, "226"));
        self.emit(abi::branch_ne(&check_e3));
        self.emit(abi::compare_immediate(remaining, "3"));
        self.emit(abi::branch_lo(not_whitespace_label));
        self.emit(abi::load_u8(x8, cursor, 1));
        self.emit(abi::compare_immediate(x8, "128"));
        self.emit(abi::branch_eq(&e2_80));
        self.emit(abi::compare_immediate(x8, "129"));
        self.emit(abi::branch_eq(&e2_81));
        self.emit(abi::branch(not_whitespace_label));

        self.emit(abi::label(&e2_80));
        self.emit(abi::load_u8(x8, cursor, 2));
        self.emit(abi::compare_immediate(x8, "128"));
        self.emit(abi::branch_lo(&e2_80_check_a8));
        self.emit(abi::compare_immediate(x8, "138"));
        self.emit(abi::branch_le(&e2_80_range));
        self.emit(abi::label(&e2_80_check_a8));
        self.emit(abi::compare_immediate(x8, "168"));
        self.emit(abi::branch_eq(&three));
        self.emit(abi::branch(&e2_80_check_a9));
        self.emit(abi::label(&e2_80_range));
        self.emit(abi::branch(&three));
        self.emit(abi::label(&e2_80_check_a9));
        self.emit(abi::compare_immediate(x8, "169"));
        self.emit(abi::branch_eq(&three));
        self.emit(abi::label(&e2_80_check_af));
        self.emit(abi::compare_immediate(x8, "175"));
        self.emit(abi::branch_eq(&three));
        self.emit(abi::branch(not_whitespace_label));

        self.emit(abi::label(&e2_81));
        self.emit(abi::load_u8(x8, cursor, 2));
        self.emit(abi::compare_immediate(x8, "159"));
        self.emit(abi::branch_eq(&three));
        self.emit(abi::branch(not_whitespace_label));

        self.emit(abi::label(&check_e3));
        self.emit(abi::compare_immediate(x7, "227"));
        self.emit(abi::branch_ne(not_whitespace_label));
        self.emit(abi::compare_immediate(remaining, "3"));
        self.emit(abi::branch_lo(not_whitespace_label));
        self.emit(abi::load_u8(x8, cursor, 1));
        self.emit(abi::compare_immediate(x8, "128"));
        self.emit(abi::branch_ne(not_whitespace_label));
        self.emit(abi::load_u8(x8, cursor, 2));
        self.emit(abi::compare_immediate(x8, "128"));
        self.emit(abi::branch_eq(&three));
        self.emit(abi::branch(not_whitespace_label));

        self.emit(abi::label(&one));
        self.emit(abi::move_immediate(width, "Integer", "1"));
        self.emit(abi::branch(whitespace_label));
        self.emit(abi::label(&two));
        self.emit(abi::move_immediate(width, "Integer", "2"));
        self.emit(abi::branch(whitespace_label));
        self.emit(abi::label(&three));
        self.emit(abi::move_immediate(width, "Integer", "3"));
        self.emit(abi::branch(whitespace_label));
    }
}
