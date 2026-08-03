use super::super::*;

// The utf8proc property record was repacked to 6 live u16 fields (plan-77 U1):
// the 5 never-read fields (decomp_type + the casefold/uppercase/lowercase and
// decomp seqindexes — case mapping and NFD use the flattened u32 tables) were
// dropped, shrinking the record from 24 to 12 bytes. Offsets here must match
// the packing order in `unicode/runtime_tables.rs::encode_le`.
const UNICODE_PROPERTY_SIZE: usize = 12;
const UNICODE_PROPERTY_OFFSET_COMBINING_CLASS: usize = 0;
const UNICODE_PROPERTY_OFFSET_COMB_INDEX: usize = 2;
const UNICODE_PROPERTY_OFFSET_COMB_LENGTH: usize = 4;
const UNICODE_PROPERTY_OFFSET_FLAGS: usize = 6;
const UNICODE_PROPERTY_OFFSET_BOUNDCLASS: usize = 8;
const UNICODE_PROPERTY_OFFSET_INDIC_CONJUNCT_BREAK: usize = 10;
const UNICODE_NFD_ENTRY_SIZE: usize = 16;
const UNICODE_NFD_ENTRY_OFFSET_CODEPOINT: usize = 0;
const UNICODE_NFD_ENTRY_OFFSET_SEQUENCE_OFFSET: usize = 4;
const UNICODE_NFD_ENTRY_OFFSET_SEQUENCE_LENGTH: usize = 8;
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

/// UTF-8 continuation-byte discriminator: `byte & 0xC0 == 0x80` iff `byte` is a
/// trailing continuation byte (`10xxxxxx`). Named so the scalar-boundary walks
/// stop respelling the two masks as bare `"192"`/`"128"` string immediates.
/// These name ONLY the continuation-mask/tag concept — the encoder's `0xC0`/`0x80`
/// lead/continuation *prefixes* and the `< 0x80` ASCII-boundary test are distinct
/// uses of the same numbers and are deliberately left spelled out.
pub(in crate::target::shared::code) const UTF8_CONTINUATION_MASK: &str = "192";
pub(in crate::target::shared::code) const UTF8_CONTINUATION_TAG: &str = "128";

impl CodeBuilder<'_> {
    /// Advance `cursor`/`remaining` past the continuation bytes of the scalar whose
    /// lead byte the caller has already consumed, stopping at the next scalar
    /// boundary or when `remaining` hits zero. `mask` must already hold
    /// [`UTF8_CONTINUATION_MASK`]; the caller mints every register and both labels
    /// (and emits `advanced_label` afterward), so this stays byte-identical to the
    /// four hand-written copies in `lower_find`/`lower_mid` it replaced.
    pub(in crate::target::shared::code) fn emit_scalar_skip_continuations(
        &mut self,
        cursor: &str,
        remaining: &str,
        byte: &str,
        mask: &str,
        continue_label: &str,
        advanced_label: &str,
    ) {
        self.emit(abi::label(continue_label));
        self.emit(abi::compare_immediate(remaining, "0"));
        self.emit(abi::branch_eq(advanced_label));
        self.emit(abi::load_u8(byte, cursor, 0));
        self.emit(abi::and_registers(byte, byte, mask));
        self.emit(abi::compare_immediate(byte, UTF8_CONTINUATION_TAG));
        self.emit(abi::branch_ne(advanced_label));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::subtract_immediate(remaining, remaining, 1));
        self.emit(abi::branch(continue_label));
    }

    /// Count the scalars (non-continuation bytes) in the `length`-byte buffer at
    /// `base`, accumulating into `count`. Uses `index`/`addr`/`byte` as scratch and
    /// loads [`UTF8_CONTINUATION_MASK`] into `mask` itself. The caller mints every
    /// register (including `base`, computed from its own source) and all four
    /// labels, so the emitted sequence is byte-identical to the two verbatim copies
    /// in `lower_strings_pad`.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::target::shared::code) fn emit_scalar_count_loop(
        &mut self,
        base: &str,
        index: &str,
        count: &str,
        addr: &str,
        byte: &str,
        mask: &str,
        length: &str,
        loop_label: &str,
        not_cont: &str,
        after: &str,
        done: &str,
    ) {
        self.emit(abi::move_immediate(index, "Integer", "0"));
        self.emit(abi::move_immediate(count, "Integer", "0"));
        self.emit(abi::move_immediate(mask, "Integer", UTF8_CONTINUATION_MASK));
        self.emit(abi::label(loop_label));
        self.emit(abi::compare_registers(index, length));
        self.emit(abi::branch_ge(done));
        self.emit(abi::add_registers(addr, base, index));
        self.emit(abi::load_u8(byte, addr, 0));
        self.emit(abi::and_registers(byte, byte, mask));
        self.emit(abi::compare_immediate(byte, UTF8_CONTINUATION_TAG));
        self.emit(abi::branch_ne(not_cont));
        self.emit(abi::branch(after));
        self.emit(abi::label(not_cont));
        self.emit(abi::add_immediate(count, count, 1));
        self.emit(abi::label(after));
        self.emit(abi::add_immediate(index, index, 1));
        self.emit(abi::branch(loop_label));
        self.emit(abi::label(done));
    }

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
        // Vreg scratch, never a pinned register: on x86-64 several role names
        // collapse to the same physical register (they fall to `rax` via
        // selection's None fallback), so `and %byte,%byte,%mask` became
        // `and rax,rax,rax` — the continuation-byte mask was dropped and the
        // codepoint decoded wrong.
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
        let index = self.temporary_vreg();
        let table = self.temporary_vreg();
        let index = index.as_str();
        let table = table.as_str();
        self.emit(abi::shift_right_immediate(index, codepoint, 8));
        self.emit(abi::shift_left_immediate(index, index, 1));
        self.emit_load_data_address(table, UNICODE_STAGE1_SYMBOL);
        self.emit(abi::add_registers(table, table, index));
        self.emit(abi::load_u16(index, table, 0));
        self.emit(abi::move_immediate(table, "Integer", "255"));
        self.emit(abi::and_registers(table, codepoint, table));
        self.emit(abi::add_registers(index, index, table));
        self.emit(abi::shift_left_immediate(index, index, 1));
        self.emit_load_data_address(table, UNICODE_STAGE2_SYMBOL);
        self.emit(abi::add_registers(table, table, index));
        self.emit(abi::load_u16(index, table, 0));
        self.emit(abi::move_immediate(
            table,
            "Integer",
            &UNICODE_PROPERTY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(index, index, table));
        self.emit_load_data_address(property, UNICODE_PROPERTIES_SYMBOL);
        self.emit(abi::add_registers(property, property, index));
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

    /// The terminal display width (0/1/2 columns) of a codepoint, read from the
    /// `charwidth` field packed into `flags` bits 4–5 (plan-70-A). The runtime
    /// computes `(flags >> 4) & 0b11`; `ambiguous_width` (bit 6) is carried in the
    /// table but not read here (policy: East-Asian Ambiguous = narrow). This is
    /// the per-scalar width primitive `strings::displayWidth` and every renderer
    /// (plan-70-B..F) consume.
    pub(in crate::target::shared::code) fn emit_unicode_property_charwidth(
        &mut self,
        property: &str,
        output: &str,
    ) {
        let mask = self.temporary_vreg();
        let mask = mask.as_str();
        self.emit(abi::load_u16(
            output,
            property,
            UNICODE_PROPERTY_OFFSET_FLAGS,
        ));
        self.emit(abi::shift_right_immediate(output, output, 4));
        self.emit(abi::move_immediate(mask, "Integer", "3"));
        self.emit(abi::and_registers(output, output, mask));
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
        let limit = self.temporary_vreg();
        let limit = limit.as_str();

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
        self.emit(abi::move_immediate(limit, "Integer", "65536"));
        self.emit(abi::compare_registers(codepoint, limit));
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
        let byte = self.temporary_vreg();
        let mask = self.temporary_vreg();
        let byte = byte.as_str();
        let mask = mask.as_str();

        self.emit(abi::compare_immediate(codepoint, "128"));
        self.emit(abi::branch_ge(&two));
        self.emit(abi::store_u8(codepoint, cursor, 0));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::branch(&done));

        self.emit(abi::label(&two));
        self.emit(abi::compare_immediate(codepoint, "2048"));
        self.emit(abi::branch_ge(&three));
        self.emit(abi::shift_right_immediate(byte, codepoint, 6));
        self.emit(abi::move_immediate(mask, "Integer", "192"));
        self.emit(abi::or_registers(byte, byte, mask));
        self.emit(abi::store_u8(byte, cursor, 0));
        self.emit(abi::move_immediate(mask, "Integer", "63"));
        self.emit(abi::and_registers(byte, codepoint, mask));
        self.emit(abi::move_immediate(mask, "Integer", "128"));
        self.emit(abi::or_registers(byte, byte, mask));
        self.emit(abi::store_u8(byte, cursor, 1));
        self.emit(abi::add_immediate(cursor, cursor, 2));
        self.emit(abi::branch(&done));

        self.emit(abi::label(&three));
        self.emit(abi::move_immediate(byte, "Integer", "65536"));
        self.emit(abi::compare_registers(codepoint, byte));
        self.emit(abi::branch_ge(&four));
        self.emit(abi::shift_right_immediate(byte, codepoint, 12));
        self.emit(abi::move_immediate(mask, "Integer", "224"));
        self.emit(abi::or_registers(byte, byte, mask));
        self.emit(abi::store_u8(byte, cursor, 0));
        self.emit(abi::shift_right_immediate(byte, codepoint, 6));
        self.emit(abi::move_immediate(mask, "Integer", "63"));
        self.emit(abi::and_registers(byte, byte, mask));
        self.emit(abi::move_immediate(mask, "Integer", "128"));
        self.emit(abi::or_registers(byte, byte, mask));
        self.emit(abi::store_u8(byte, cursor, 1));
        self.emit(abi::move_immediate(mask, "Integer", "63"));
        self.emit(abi::and_registers(byte, codepoint, mask));
        self.emit(abi::move_immediate(mask, "Integer", "128"));
        self.emit(abi::or_registers(byte, byte, mask));
        self.emit(abi::store_u8(byte, cursor, 2));
        self.emit(abi::add_immediate(cursor, cursor, 3));
        self.emit(abi::branch(&done));

        self.emit(abi::label(&four));
        self.emit(abi::shift_right_immediate(byte, codepoint, 18));
        self.emit(abi::move_immediate(mask, "Integer", "240"));
        self.emit(abi::or_registers(byte, byte, mask));
        self.emit(abi::store_u8(byte, cursor, 0));
        self.emit(abi::shift_right_immediate(byte, codepoint, 12));
        self.emit(abi::move_immediate(mask, "Integer", "63"));
        self.emit(abi::and_registers(byte, byte, mask));
        self.emit(abi::move_immediate(mask, "Integer", "128"));
        self.emit(abi::or_registers(byte, byte, mask));
        self.emit(abi::store_u8(byte, cursor, 1));
        self.emit(abi::shift_right_immediate(byte, codepoint, 6));
        self.emit(abi::move_immediate(mask, "Integer", "63"));
        self.emit(abi::and_registers(byte, byte, mask));
        self.emit(abi::move_immediate(mask, "Integer", "128"));
        self.emit(abi::or_registers(byte, byte, mask));
        self.emit(abi::store_u8(byte, cursor, 2));
        self.emit(abi::move_immediate(mask, "Integer", "63"));
        self.emit(abi::and_registers(byte, codepoint, mask));
        self.emit(abi::move_immediate(mask, "Integer", "128"));
        self.emit(abi::or_registers(byte, byte, mask));
        self.emit(abi::store_u8(byte, cursor, 3));
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
        let lo = self.temporary_vreg();
        let hi = self.temporary_vreg();
        let lo = lo.as_str();
        let hi = hi.as_str();
        // Binary-search scratch as vregs.
        let mid_v = self.temporary_vreg();
        let offset_v = self.temporary_vreg();
        let entry_ptr_v = self.temporary_vreg();
        let field_v = self.temporary_vreg();
        let mid = mid_v.as_str();
        let offset = offset_v.as_str();
        let entry_ptr = entry_ptr_v.as_str();
        let field = field_v.as_str();

        self.emit(abi::move_immediate(lo, "Integer", "0"));
        self.emit(abi::move_immediate(hi, "Integer", &entry_count.to_string()));
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_registers(lo, hi));
        self.emit(abi::branch_ge(&not_found));
        self.emit(abi::add_registers(mid, lo, hi));
        self.emit(abi::shift_right_immediate(mid, mid, 1));
        // `mid * UNICODE_NFD_ENTRY_SIZE`, as a shift. Deriving the shift from the
        // constant rather than hard-coding `4` is what keeps the stride and the
        // record layout above from drifting apart — the spec cites the constant
        // by name (`unicode/01_tables-and-algorithms.md`), so a silent
        // disagreement here would make the spec wrong (bug-326-D2).
        const NFD_ENTRY_SHIFT: u32 = UNICODE_NFD_ENTRY_SIZE.trailing_zeros();
        const _: () = assert!(UNICODE_NFD_ENTRY_SIZE.is_power_of_two());
        self.emit(abi::shift_left_immediate(
            offset,
            mid,
            NFD_ENTRY_SHIFT as u8,
        ));
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
        self.emit(abi::move_register(hi, mid));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&move_left));
        self.emit(abi::add_immediate(lo, mid, 1));
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
        let base = self.temporary_vreg();
        let l_index = self.temporary_vreg();
        let base = base.as_str();
        let l_index = l_index.as_str();
        // Scratch as a vreg. A raw physical-register write here is invisible to
        // the register allocator, so a caller value it had placed in that
        // register is silently clobbered — a layout-sensitive miscompile.
        let v_index = self.temporary_vreg();
        let v_index = v_index.as_str();

        self.emit(abi::move_immediate(base, "Integer", "4352"));
        self.emit(abi::compare_registers(starter, base));
        self.emit(abi::branch_lo(&check_lv_t));
        self.emit(abi::subtract_registers(l_index, starter, base));
        self.emit(abi::compare_immediate(l_index, "19"));
        self.emit(abi::branch_ge(&check_lv_t));

        self.emit(abi::move_immediate(base, "Integer", "4449"));
        self.emit(abi::compare_registers(current, base));
        self.emit(abi::branch_lo(&check_lv_t));
        self.emit(abi::subtract_registers(v_index, current, base));
        self.emit(abi::compare_immediate(v_index, "21"));
        self.emit(abi::branch_ge(&check_lv_t));
        self.emit(abi::move_immediate(base, "Integer", "21"));
        self.emit(abi::multiply_registers(output, l_index, base));
        self.emit(abi::add_registers(output, output, v_index));
        self.emit(abi::move_immediate(base, "Integer", "28"));
        self.emit(abi::multiply_registers(output, output, base));
        self.emit(abi::move_immediate(base, "Integer", "44032"));
        self.emit(abi::add_registers(output, output, base));
        self.emit(abi::branch(found_label));

        self.emit(abi::label(&check_lv_t));
        self.emit(abi::move_immediate(base, "Integer", "44032"));
        self.emit(abi::compare_registers(starter, base));
        self.emit(abi::branch_lo(fallback_label));
        self.emit(abi::subtract_registers(l_index, starter, base));
        self.emit(abi::move_immediate(base, "Integer", "11172"));
        self.emit(abi::compare_registers(l_index, base));
        self.emit(abi::branch_ge(fallback_label));
        self.emit(abi::move_immediate(base, "Integer", "28"));
        self.emit(abi::unsigned_divide_registers(v_index, l_index, base));
        self.emit(abi::multiply_subtract_registers(
            v_index, v_index, base, l_index,
        ));
        self.emit(abi::compare_immediate(v_index, "0"));
        self.emit(abi::branch_ne(fallback_label));
        self.emit(abi::move_immediate(base, "Integer", "4519"));
        self.emit(abi::compare_registers(current, base));
        self.emit(abi::branch_lo(fallback_label));
        self.emit(abi::subtract_registers(v_index, current, base));
        self.emit(abi::compare_immediate(v_index, "0"));
        self.emit(abi::branch_eq(fallback_label));
        self.emit(abi::compare_immediate(v_index, "28"));
        self.emit(abi::branch_ge(fallback_label));
        self.emit(abi::add_registers(output, starter, v_index));
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
        let lptr = self.temporary_vreg();
        let rptr = self.temporary_vreg();
        let remaining = self.temporary_vreg();
        let lbyte = self.temporary_vreg();
        let lptr = lptr.as_str();
        let rptr = rptr.as_str();
        let remaining = remaining.as_str();
        let lbyte = lbyte.as_str();
        // Byte-compare scratch as a vreg. This helper backs `String` equality
        // and every substring predicate, so a raw physical-register write here
        // clobbers a caller value the allocator had placed there under register
        // pressure — a layout-sensitive miscompile that corrupted adjacent
        // string comparisons.
        let rbyte = self.temporary_vreg();
        let rbyte = rbyte.as_str();
        self.emit(abi::move_register(lptr, left_data));
        self.emit(abi::move_register(rptr, right_data));
        self.emit(abi::move_register(remaining, length));
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate(remaining, "0"));
        self.emit(abi::branch_eq(equal_label));
        self.emit(abi::load_u8(lbyte, lptr, 0));
        self.emit(abi::load_u8(rbyte, rptr, 0));
        self.emit(abi::compare_registers(lbyte, rbyte));
        self.emit(abi::branch_ne(not_equal_label));
        self.emit(abi::add_immediate(lptr, lptr, 1));
        self.emit(abi::add_immediate(rptr, rptr, 1));
        self.emit(abi::subtract_immediate(remaining, remaining, 1));
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
        let lead = self.temporary_vreg();
        let lead = lead.as_str();
        // Continuation-byte scratch as a vreg. A raw physical-register write is
        // invisible to the register allocator and clobbers a caller value under
        // register pressure — a layout-sensitive miscompile.
        let cont = self.temporary_vreg();
        let cont = cont.as_str();
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
        self.emit(abi::load_u8(lead, cursor, 0));
        self.emit(abi::compare_immediate(lead, "9"));
        self.emit(abi::branch_lo(&check_c2));
        self.emit(abi::compare_immediate(lead, "13"));
        self.emit(abi::branch_le(&one));
        self.emit(abi::compare_immediate(lead, "32"));
        self.emit(abi::branch_eq(&one));

        self.emit(abi::label(&check_c2));
        self.emit(abi::compare_immediate(lead, "194"));
        self.emit(abi::branch_ne(&check_e1));
        self.emit(abi::compare_immediate(remaining, "2"));
        self.emit(abi::branch_lo(not_whitespace_label));
        self.emit(abi::load_u8(cont, cursor, 1));
        self.emit(abi::compare_immediate(cont, "133"));
        self.emit(abi::branch_eq(&two));
        self.emit(abi::compare_immediate(cont, "160"));
        self.emit(abi::branch_eq(&two));
        self.emit(abi::branch(not_whitespace_label));

        self.emit(abi::label(&check_e1));
        self.emit(abi::compare_immediate(lead, "225"));
        self.emit(abi::branch_ne(&check_e2));
        self.emit(abi::compare_immediate(remaining, "3"));
        self.emit(abi::branch_lo(not_whitespace_label));
        self.emit(abi::load_u8(cont, cursor, 1));
        self.emit(abi::compare_immediate(cont, "154"));
        self.emit(abi::branch_ne(not_whitespace_label));
        self.emit(abi::load_u8(cont, cursor, 2));
        self.emit(abi::compare_immediate(cont, "128"));
        self.emit(abi::branch_eq(&three));
        self.emit(abi::branch(not_whitespace_label));

        self.emit(abi::label(&check_e2));
        self.emit(abi::compare_immediate(lead, "226"));
        self.emit(abi::branch_ne(&check_e3));
        self.emit(abi::compare_immediate(remaining, "3"));
        self.emit(abi::branch_lo(not_whitespace_label));
        self.emit(abi::load_u8(cont, cursor, 1));
        self.emit(abi::compare_immediate(cont, "128"));
        self.emit(abi::branch_eq(&e2_80));
        self.emit(abi::compare_immediate(cont, "129"));
        self.emit(abi::branch_eq(&e2_81));
        self.emit(abi::branch(not_whitespace_label));

        self.emit(abi::label(&e2_80));
        self.emit(abi::load_u8(cont, cursor, 2));
        self.emit(abi::compare_immediate(cont, "128"));
        self.emit(abi::branch_lo(&e2_80_check_a8));
        self.emit(abi::compare_immediate(cont, "138"));
        self.emit(abi::branch_le(&e2_80_range));
        self.emit(abi::label(&e2_80_check_a8));
        self.emit(abi::compare_immediate(cont, "168"));
        self.emit(abi::branch_eq(&three));
        self.emit(abi::branch(&e2_80_check_a9));
        self.emit(abi::label(&e2_80_range));
        self.emit(abi::branch(&three));
        self.emit(abi::label(&e2_80_check_a9));
        self.emit(abi::compare_immediate(cont, "169"));
        self.emit(abi::branch_eq(&three));
        self.emit(abi::label(&e2_80_check_af));
        self.emit(abi::compare_immediate(cont, "175"));
        self.emit(abi::branch_eq(&three));
        self.emit(abi::branch(not_whitespace_label));

        self.emit(abi::label(&e2_81));
        self.emit(abi::load_u8(cont, cursor, 2));
        self.emit(abi::compare_immediate(cont, "159"));
        self.emit(abi::branch_eq(&three));
        self.emit(abi::branch(not_whitespace_label));

        self.emit(abi::label(&check_e3));
        self.emit(abi::compare_immediate(lead, "227"));
        self.emit(abi::branch_ne(not_whitespace_label));
        self.emit(abi::compare_immediate(remaining, "3"));
        self.emit(abi::branch_lo(not_whitespace_label));
        self.emit(abi::load_u8(cont, cursor, 1));
        self.emit(abi::compare_immediate(cont, "128"));
        self.emit(abi::branch_ne(not_whitespace_label));
        self.emit(abi::load_u8(cont, cursor, 2));
        self.emit(abi::compare_immediate(cont, "128"));
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

// ---------------------------------------------------------------------------
// Free-function Unicode primitives for the neutral console codegen (plan-70-B/C).
//
// `term_grid.rs`/`term.rs` build their writer/present/stamp loops as free
// functions that push raw `abi::` instructions into a `Vec<CodeInstruction>` and
// their relocations into a `Vec<CodeRelocation>` — they are NOT `CodeBuilder`
// methods, so they cannot call the `emit_unicode_*` helpers above (those go
// through `self.emit`/`self.relocations`). These free mirrors take the current
// function `symbol` (the relocation `from`), an explicit `label_prefix` for
// unique labels, and caller-chosen scratch vregs, so the console width path can
// reuse the exact two-stage property walk without a `CodeBuilder`.

/// Load the address of an internal data `symbol` into `register` via the
/// `adrp`/`add` page pair, recording the two data-address relocations against
/// `from`. Free mirror of `CodeBuilder::emit_load_data_address`.
pub(in crate::target::shared::code) fn emit_load_data_address_free(
    from: &str,
    symbol: &str,
    register: &str,
    instrs: &mut Vec<CodeInstruction>,
    relocs: &mut Vec<CodeRelocation>,
) {
    instrs.push(abi::load_page_address(register, symbol));
    relocs.push(CodeRelocation {
        from: from.to_string(),
        to: symbol.to_string(),
        kind: RelocIntent::DataAddrHi,
        binding: "data".to_string(),
        library: None,
    });
    instrs.push(abi::add_page_offset(register, register, symbol));
    relocs.push(CodeRelocation {
        from: from.to_string(),
        to: symbol.to_string(),
        kind: RelocIntent::DataAddrLo,
        binding: "data".to_string(),
        library: None,
    });
}

/// Decode the UTF-8 scalar at `ptr` (whose byte length `len` is already known,
/// 1..=4, with lead byte in `lead`) into `codepoint`. Standard mask/shift decode;
/// the caller has already clamped `len` to the remaining bytes, so this never
/// reads past the string. `mask`/`byte` are caller-owned throwaway vregs. Used by
/// the console writer/stamp path (plan-70-B/C) to recover a codepoint for the
/// width lookup after the raw bytes were packed into the cell glyph.
pub(in crate::target::shared::code) fn emit_utf8_codepoint_by_len(
    label_prefix: &str,
    ptr: &str,
    len: &str,
    lead: &str,
    codepoint: &str,
    byte: &str,
    mask: &str,
    instrs: &mut Vec<CodeInstruction>,
) {
    let do2 = format!("{label_prefix}_cp2");
    let do3 = format!("{label_prefix}_cp3");
    let done = format!("{label_prefix}_cpdone");
    // Continuation byte payload mask.
    // len == 1 (default): codepoint = lead.
    instrs.push(abi::move_register(codepoint, lead));
    instrs.push(abi::compare_immediate(len, "2"));
    instrs.push(abi::branch_lo(&done));
    instrs.push(abi::branch_eq(&do2));
    instrs.push(abi::compare_immediate(len, "3"));
    instrs.push(abi::branch_eq(&do3));
    // len == 4: (lead & 0x07)<<18 | (b1&0x3F)<<12 | (b2&0x3F)<<6 | (b3&0x3F)
    instrs.push(abi::move_immediate(mask, "Integer", "7"));
    instrs.push(abi::and_registers(codepoint, lead, mask));
    instrs.push(abi::shift_left_immediate(codepoint, codepoint, 18));
    instrs.push(abi::move_immediate(mask, "Integer", "63"));
    instrs.push(abi::load_u8(byte, ptr, 1));
    instrs.push(abi::and_registers(byte, byte, mask));
    instrs.push(abi::shift_left_immediate(byte, byte, 12));
    instrs.push(abi::or_registers(codepoint, codepoint, byte));
    instrs.push(abi::load_u8(byte, ptr, 2));
    instrs.push(abi::and_registers(byte, byte, mask));
    instrs.push(abi::shift_left_immediate(byte, byte, 6));
    instrs.push(abi::or_registers(codepoint, codepoint, byte));
    instrs.push(abi::load_u8(byte, ptr, 3));
    instrs.push(abi::and_registers(byte, byte, mask));
    instrs.push(abi::or_registers(codepoint, codepoint, byte));
    instrs.push(abi::branch(&done));
    // len == 3: (lead & 0x0F)<<12 | (b1&0x3F)<<6 | (b2&0x3F)
    instrs.push(abi::label(&do3));
    instrs.push(abi::move_immediate(mask, "Integer", "15"));
    instrs.push(abi::and_registers(codepoint, lead, mask));
    instrs.push(abi::shift_left_immediate(codepoint, codepoint, 12));
    instrs.push(abi::move_immediate(mask, "Integer", "63"));
    instrs.push(abi::load_u8(byte, ptr, 1));
    instrs.push(abi::and_registers(byte, byte, mask));
    instrs.push(abi::shift_left_immediate(byte, byte, 6));
    instrs.push(abi::or_registers(codepoint, codepoint, byte));
    instrs.push(abi::load_u8(byte, ptr, 2));
    instrs.push(abi::and_registers(byte, byte, mask));
    instrs.push(abi::or_registers(codepoint, codepoint, byte));
    instrs.push(abi::branch(&done));
    // len == 2: (lead & 0x1F)<<6 | (b1&0x3F)
    instrs.push(abi::label(&do2));
    instrs.push(abi::move_immediate(mask, "Integer", "31"));
    instrs.push(abi::and_registers(codepoint, lead, mask));
    instrs.push(abi::shift_left_immediate(codepoint, codepoint, 6));
    instrs.push(abi::move_immediate(mask, "Integer", "63"));
    instrs.push(abi::load_u8(byte, ptr, 1));
    instrs.push(abi::and_registers(byte, byte, mask));
    instrs.push(abi::or_registers(codepoint, codepoint, byte));
    instrs.push(abi::label(&done));
}

/// Compute the address of `codepoint`'s property record into `prop_out` via the
/// two-stage trie. A codepoint `>= 0x110000` yields the default (index-0) record,
/// matching `property_for_codepoint`'s guard and avoiding an OOB stage-1 read.
/// `scratch` is a caller-owned throwaway vreg. The console cluster walk
/// (plan-70-B Phase 2) reads boundclass (offset 18), indic-conjunct-break (offset
/// 20), and charwidth (`(flags@16 >> 4) & 3`) straight from `prop_out`.
pub(in crate::target::shared::code) fn emit_unicode_property_ptr_free(
    from: &str,
    label_prefix: &str,
    codepoint: &str,
    prop_out: &str,
    scratch: &str,
    instrs: &mut Vec<CodeInstruction>,
    relocs: &mut Vec<CodeRelocation>,
) {
    let in_range = format!("{label_prefix}_pp_in");
    let done = format!("{label_prefix}_pp_done");
    instrs.push(abi::move_immediate(scratch, "Integer", "1114112"));
    instrs.push(abi::compare_registers(codepoint, scratch));
    instrs.push(abi::branch_lo(&in_range));
    // Out of range → the index-0 (default) record.
    emit_load_data_address_free(from, UNICODE_PROPERTIES_SYMBOL, prop_out, instrs, relocs);
    instrs.push(abi::branch(&done));
    instrs.push(abi::label(&in_range));
    // stage1[cp >> 8]
    instrs.push(abi::shift_right_immediate(prop_out, codepoint, 8));
    instrs.push(abi::shift_left_immediate(prop_out, prop_out, 1));
    emit_load_data_address_free(from, UNICODE_STAGE1_SYMBOL, scratch, instrs, relocs);
    instrs.push(abi::add_registers(scratch, scratch, prop_out));
    instrs.push(abi::load_u16(prop_out, scratch, 0));
    // stage2[stage1 + (cp & 0xff)]
    instrs.push(abi::move_immediate(scratch, "Integer", "255"));
    instrs.push(abi::and_registers(scratch, codepoint, scratch));
    instrs.push(abi::add_registers(prop_out, prop_out, scratch));
    instrs.push(abi::shift_left_immediate(prop_out, prop_out, 1));
    emit_load_data_address_free(from, UNICODE_STAGE2_SYMBOL, scratch, instrs, relocs);
    instrs.push(abi::add_registers(scratch, scratch, prop_out));
    instrs.push(abi::load_u16(prop_out, scratch, 0));
    // properties + stage2 * PROPERTY_SIZE
    instrs.push(abi::move_immediate(
        scratch,
        "Integer",
        &UNICODE_PROPERTY_SIZE.to_string(),
    ));
    instrs.push(abi::multiply_registers(prop_out, prop_out, scratch));
    emit_load_data_address_free(from, UNICODE_PROPERTIES_SYMBOL, scratch, instrs, relocs);
    instrs.push(abi::add_registers(prop_out, scratch, prop_out));
    instrs.push(abi::label(&done));
}

/// Read a property record's boundclass (offset 18), indic-conjunct-break (offset
/// 20), and charwidth (`(flags@16 >> 4) & 3`) from `prop` into the three outputs.
/// `scratch` is a caller-owned throwaway vreg. plan-70-B Phase 2 cluster walk.
pub(in crate::target::shared::code) fn emit_read_boundclass_icb_charwidth_free(
    prop: &str,
    bc_out: &str,
    icb_out: &str,
    width_out: &str,
    scratch: &str,
    instrs: &mut Vec<CodeInstruction>,
) {
    instrs.push(abi::load_u16(
        bc_out,
        prop,
        UNICODE_PROPERTY_OFFSET_BOUNDCLASS,
    ));
    instrs.push(abi::load_u16(
        icb_out,
        prop,
        UNICODE_PROPERTY_OFFSET_INDIC_CONJUNCT_BREAK,
    ));
    instrs.push(abi::load_u16(
        width_out,
        prop,
        UNICODE_PROPERTY_OFFSET_FLAGS,
    ));
    instrs.push(abi::shift_right_immediate(width_out, width_out, 4));
    instrs.push(abi::move_immediate(scratch, "Integer", "3"));
    instrs.push(abi::and_registers(width_out, width_out, scratch));
}

/// Free mirror of `CodeBuilder::emit_grapheme_break_branch` (UAX #29 GB rules):
/// branch to `break_label` if there is a cluster boundary between the previous
/// scalar (`state_bc`/`state_icb`) and the current one (`current_bc`/`current_icb`),
/// else to `no_break_label`. Uses only caller-passed registers + labels derived
/// from `label_prefix`; no scratch. plan-70-B Phase 2.
pub(in crate::target::shared::code) fn emit_grapheme_break_branch_free(
    label_prefix: &str,
    state_bc: &str,
    state_icb: &str,
    current_bc: &str,
    current_icb: &str,
    break_label: &str,
    no_break_label: &str,
    instrs: &mut Vec<CodeInstruction>,
) {
    let no_break = format!("{label_prefix}_gnb");
    let maybe_break = format!("{label_prefix}_gmb");
    let gb3_not_cr = format!("{label_prefix}_g3");
    let gb4_not_control = format!("{label_prefix}_g4");
    let gb5_not_control = format!("{label_prefix}_g5");
    let gb6_check = format!("{label_prefix}_g6");
    let gb7_check = format!("{label_prefix}_g7");
    let gb7_no = format!("{label_prefix}_g7n");
    let gb8_check = format!("{label_prefix}_g8");
    let gb8_no = format!("{label_prefix}_g8n");
    let gb9_check = format!("{label_prefix}_g9");
    let gb11_check = format!("{label_prefix}_g11");
    let gb1213_check = format!("{label_prefix}_g1213");
    let gb9c_break = format!("{label_prefix}_g9c");

    // GB3: CR × LF.
    instrs.push(abi::compare_immediate(state_bc, GRAPHEME_BOUNDCLASS_CR));
    instrs.push(abi::branch_ne(&gb3_not_cr));
    instrs.push(abi::compare_immediate(current_bc, GRAPHEME_BOUNDCLASS_LF));
    instrs.push(abi::branch_eq(&no_break));
    instrs.push(abi::label(&gb3_not_cr));
    // GB4: (CR|LF|Control) ÷.
    instrs.push(abi::compare_immediate(state_bc, GRAPHEME_BOUNDCLASS_CR));
    instrs.push(abi::branch_lo(&gb4_not_control));
    instrs.push(abi::compare_immediate(
        state_bc,
        GRAPHEME_BOUNDCLASS_CONTROL,
    ));
    instrs.push(abi::branch_le(&maybe_break));
    instrs.push(abi::label(&gb4_not_control));
    // GB5: ÷ (CR|LF|Control).
    instrs.push(abi::compare_immediate(current_bc, GRAPHEME_BOUNDCLASS_CR));
    instrs.push(abi::branch_lo(&gb5_not_control));
    instrs.push(abi::compare_immediate(
        current_bc,
        GRAPHEME_BOUNDCLASS_CONTROL,
    ));
    instrs.push(abi::branch_le(&maybe_break));
    instrs.push(abi::label(&gb5_not_control));
    // GB6: L × (L|V|LV|LVT).
    instrs.push(abi::compare_immediate(state_bc, GRAPHEME_BOUNDCLASS_L));
    instrs.push(abi::branch_ne(&gb6_check));
    instrs.push(abi::compare_immediate(current_bc, GRAPHEME_BOUNDCLASS_L));
    instrs.push(abi::branch_eq(&no_break));
    instrs.push(abi::compare_immediate(current_bc, GRAPHEME_BOUNDCLASS_V));
    instrs.push(abi::branch_eq(&no_break));
    instrs.push(abi::compare_immediate(current_bc, GRAPHEME_BOUNDCLASS_LV));
    instrs.push(abi::branch_eq(&no_break));
    instrs.push(abi::compare_immediate(current_bc, GRAPHEME_BOUNDCLASS_LVT));
    instrs.push(abi::branch_eq(&no_break));
    instrs.push(abi::label(&gb6_check));
    // GB7: (LV|V) × (V|T).
    instrs.push(abi::compare_immediate(state_bc, GRAPHEME_BOUNDCLASS_LV));
    instrs.push(abi::branch_eq(&gb7_check));
    instrs.push(abi::compare_immediate(state_bc, GRAPHEME_BOUNDCLASS_V));
    instrs.push(abi::branch_ne(&gb7_no));
    instrs.push(abi::label(&gb7_check));
    instrs.push(abi::compare_immediate(current_bc, GRAPHEME_BOUNDCLASS_V));
    instrs.push(abi::branch_eq(&no_break));
    instrs.push(abi::compare_immediate(current_bc, GRAPHEME_BOUNDCLASS_T));
    instrs.push(abi::branch_eq(&no_break));
    instrs.push(abi::label(&gb7_no));
    // GB8: (LVT|T) × T.
    instrs.push(abi::compare_immediate(state_bc, GRAPHEME_BOUNDCLASS_LVT));
    instrs.push(abi::branch_eq(&gb8_check));
    instrs.push(abi::compare_immediate(state_bc, GRAPHEME_BOUNDCLASS_T));
    instrs.push(abi::branch_ne(&gb8_no));
    instrs.push(abi::label(&gb8_check));
    instrs.push(abi::compare_immediate(current_bc, GRAPHEME_BOUNDCLASS_T));
    instrs.push(abi::branch_eq(&no_break));
    instrs.push(abi::label(&gb8_no));
    // GB9/GB9a/GB9b: × (Extend|ZWJ|SpacingMark), Prepend ×.
    instrs.push(abi::label(&gb9_check));
    instrs.push(abi::compare_immediate(
        current_bc,
        GRAPHEME_BOUNDCLASS_EXTEND,
    ));
    instrs.push(abi::branch_eq(&no_break));
    instrs.push(abi::compare_immediate(current_bc, GRAPHEME_BOUNDCLASS_ZWJ));
    instrs.push(abi::branch_eq(&no_break));
    instrs.push(abi::compare_immediate(
        current_bc,
        GRAPHEME_BOUNDCLASS_SPACINGMARK,
    ));
    instrs.push(abi::branch_eq(&no_break));
    instrs.push(abi::compare_immediate(
        state_bc,
        GRAPHEME_BOUNDCLASS_PREPEND,
    ));
    instrs.push(abi::branch_eq(&no_break));
    // GB11: ExtPict ZWJ × ExtPict (state E_ZWG).
    instrs.push(abi::label(&gb11_check));
    instrs.push(abi::compare_immediate(state_bc, GRAPHEME_BOUNDCLASS_E_ZWG));
    instrs.push(abi::branch_ne(&gb1213_check));
    instrs.push(abi::compare_immediate(
        current_bc,
        GRAPHEME_BOUNDCLASS_EXTENDED_PICTOGRAPHIC,
    ));
    instrs.push(abi::branch_eq(&no_break));
    // GB12/GB13: RI × RI (paired).
    instrs.push(abi::label(&gb1213_check));
    instrs.push(abi::compare_immediate(
        state_bc,
        GRAPHEME_BOUNDCLASS_REGIONAL_INDICATOR,
    ));
    instrs.push(abi::branch_ne(&maybe_break));
    instrs.push(abi::compare_immediate(
        current_bc,
        GRAPHEME_BOUNDCLASS_REGIONAL_INDICATOR,
    ));
    instrs.push(abi::branch_eq(&no_break));
    // GB9c: Consonant [Extend] Linker × Consonant.
    instrs.push(abi::label(&maybe_break));
    instrs.push(abi::compare_immediate(
        state_icb,
        INDIC_CONJUNCT_BREAK_LINKER,
    ));
    instrs.push(abi::branch_ne(&gb9c_break));
    instrs.push(abi::compare_immediate(
        current_icb,
        INDIC_CONJUNCT_BREAK_CONSONANT,
    ));
    instrs.push(abi::branch_eq(&no_break));
    instrs.push(abi::label(&gb9c_break));
    instrs.push(abi::branch(break_label));
    instrs.push(abi::label(&no_break));
    instrs.push(abi::branch(no_break_label));
}

/// Free mirror of `CodeBuilder::emit_grapheme_state_update`: advance the running
/// boundclass/indic-conjunct-break state (`state_bc`/`state_icb`) after consuming
/// the current scalar (`current_bc`/`current_icb`). plan-70-B Phase 2.
pub(in crate::target::shared::code) fn emit_grapheme_state_update_free(
    label_prefix: &str,
    state_bc: &str,
    state_icb: &str,
    current_bc: &str,
    current_icb: &str,
    instrs: &mut Vec<CodeInstruction>,
) {
    let icb_consonant = format!("{label_prefix}_uc");
    let icb_existing_consonant = format!("{label_prefix}_uec");
    let icb_existing_extend = format!("{label_prefix}_uee");
    let icb_linker = format!("{label_prefix}_ul");
    let icb_linker_extend = format!("{label_prefix}_ule");
    let icb_done = format!("{label_prefix}_ud");
    let bc_ri_check = format!("{label_prefix}_uri");
    let bc_extpic_check = format!("{label_prefix}_uep");
    let bc_extpic_extend = format!("{label_prefix}_uepe");
    let bc_extpic_zwj = format!("{label_prefix}_uepz");
    let bc_set_current = format!("{label_prefix}_usc");
    let bc_done = format!("{label_prefix}_ubd");

    instrs.push(abi::compare_immediate(
        current_icb,
        INDIC_CONJUNCT_BREAK_CONSONANT,
    ));
    instrs.push(abi::branch_eq(&icb_consonant));
    instrs.push(abi::compare_immediate(
        state_icb,
        INDIC_CONJUNCT_BREAK_CONSONANT,
    ));
    instrs.push(abi::branch_eq(&icb_existing_consonant));
    instrs.push(abi::compare_immediate(
        state_icb,
        INDIC_CONJUNCT_BREAK_EXTEND,
    ));
    instrs.push(abi::branch_eq(&icb_existing_extend));
    instrs.push(abi::compare_immediate(
        state_icb,
        INDIC_CONJUNCT_BREAK_LINKER,
    ));
    instrs.push(abi::branch_eq(&icb_linker));
    instrs.push(abi::branch(&icb_done));
    instrs.push(abi::label(&icb_consonant));
    instrs.push(abi::move_register(state_icb, current_icb));
    instrs.push(abi::branch(&icb_done));
    instrs.push(abi::label(&icb_existing_consonant));
    instrs.push(abi::move_register(state_icb, current_icb));
    instrs.push(abi::branch(&icb_done));
    instrs.push(abi::label(&icb_existing_extend));
    instrs.push(abi::move_register(state_icb, current_icb));
    instrs.push(abi::branch(&icb_done));
    instrs.push(abi::label(&icb_linker));
    instrs.push(abi::compare_immediate(
        current_icb,
        INDIC_CONJUNCT_BREAK_EXTEND,
    ));
    instrs.push(abi::branch_eq(&icb_linker_extend));
    instrs.push(abi::move_register(state_icb, current_icb));
    instrs.push(abi::branch(&icb_done));
    instrs.push(abi::label(&icb_linker_extend));
    instrs.push(abi::move_immediate(
        state_icb,
        "Integer",
        INDIC_CONJUNCT_BREAK_LINKER,
    ));
    instrs.push(abi::label(&icb_done));

    instrs.push(abi::compare_registers(state_bc, current_bc));
    instrs.push(abi::branch_ne(&bc_extpic_check));
    instrs.push(abi::compare_immediate(
        current_bc,
        GRAPHEME_BOUNDCLASS_REGIONAL_INDICATOR,
    ));
    instrs.push(abi::branch_eq(&bc_ri_check));
    instrs.push(abi::label(&bc_extpic_check));
    instrs.push(abi::compare_immediate(
        state_bc,
        GRAPHEME_BOUNDCLASS_EXTENDED_PICTOGRAPHIC,
    ));
    instrs.push(abi::branch_ne(&bc_set_current));
    instrs.push(abi::compare_immediate(
        current_bc,
        GRAPHEME_BOUNDCLASS_EXTEND,
    ));
    instrs.push(abi::branch_eq(&bc_extpic_extend));
    instrs.push(abi::compare_immediate(current_bc, GRAPHEME_BOUNDCLASS_ZWJ));
    instrs.push(abi::branch_eq(&bc_extpic_zwj));
    instrs.push(abi::branch(&bc_set_current));
    instrs.push(abi::label(&bc_ri_check));
    instrs.push(abi::move_immediate(state_bc, "Integer", "1"));
    instrs.push(abi::branch(&bc_done));
    instrs.push(abi::label(&bc_extpic_extend));
    instrs.push(abi::move_immediate(
        state_bc,
        "Integer",
        GRAPHEME_BOUNDCLASS_EXTENDED_PICTOGRAPHIC,
    ));
    instrs.push(abi::branch(&bc_done));
    instrs.push(abi::label(&bc_extpic_zwj));
    instrs.push(abi::move_immediate(
        state_bc,
        "Integer",
        GRAPHEME_BOUNDCLASS_E_ZWG,
    ));
    instrs.push(abi::branch(&bc_done));
    instrs.push(abi::label(&bc_set_current));
    instrs.push(abi::move_register(state_bc, current_bc));
    instrs.push(abi::label(&bc_done));
}
