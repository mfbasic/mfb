// The utf8proc property record was repacked to 6 live u16 fields (plan-77 U1):
// the 5 never-read fields (decomp_type + the casefold/uppercase/lowercase and
// decomp seqindexes — case mapping and NFD use the flattened u32 tables) were
// dropped, shrinking the record from 24 to 12 bytes. Offsets here must match
// the packing order in `unicode/runtime_tables.rs::encode_le`.
// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;

mod free_mirrors;
mod grapheme;
mod properties;
use free_mirrors::*;
pub(crate) use free_mirrors::{
    emit_grapheme_break_branch_free, emit_grapheme_state_update_free, emit_load_data_address_free,
    emit_read_boundclass_icb_charwidth_free, emit_seed_indic_conjunct_state_free,
    emit_unicode_property_ptr_free, emit_utf8_codepoint_by_len,
};
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
pub(crate) const UTF8_CONTINUATION_MASK: &str = "192";
pub(crate) const UTF8_CONTINUATION_TAG: &str = "128";

impl CodeBuilder<'_> {
    /// Advance `cursor`/`remaining` past the continuation bytes of the scalar whose
    /// lead byte the caller has already consumed, stopping at the next scalar
    /// boundary or when `remaining` hits zero. `mask` must already hold
    /// [`UTF8_CONTINUATION_MASK`]; the caller mints every register and both labels
    /// (and emits `advanced_label` afterward), so this stays byte-identical to the
    /// four hand-written copies in `lower_find`/`lower_mid` it replaced.
    pub(crate) fn emit_scalar_skip_continuations(
        &mut self,
        cursor: impl Into<abi::Reg>,
        remaining: impl Into<abi::Reg>,
        byte: impl Into<abi::Reg>,
        mask: impl Into<abi::Reg>,
        continue_label: &str,
        advanced_label: &str,
    ) {
        let cursor: abi::Reg = cursor.into();
        let remaining: abi::Reg = remaining.into();
        let byte: abi::Reg = byte.into();
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
    pub(crate) fn emit_scalar_count_loop(
        &mut self,
        base: impl Into<abi::Reg>,
        index: impl Into<abi::Reg>,
        count: impl Into<abi::Reg>,
        addr: impl Into<abi::Reg>,
        byte: impl Into<abi::Reg>,
        mask: impl Into<abi::Reg>,
        length: impl Into<abi::Reg>,
        loop_label: &str,
        not_cont: &str,
        after: &str,
        done: &str,
    ) {
        let index: abi::Reg = index.into();
        let count: abi::Reg = count.into();
        let addr: abi::Reg = addr.into();
        let byte: abi::Reg = byte.into();
        let mask: abi::Reg = mask.into();
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

    pub(crate) fn emit_load_data_address(&mut self, register: impl Into<abi::Reg>, symbol: &str) {
        let register: abi::Reg = register.into();
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
    pub(crate) fn emit_utf8_decode_next(
        &mut self,
        cursor: impl Into<abi::Reg>,
        codepoint: impl Into<abi::Reg>,
        width: impl Into<abi::Reg>,
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
        let byte = &byte;
        let byte2 = &byte2;
        let byte3 = &byte3;
        let masked = &masked;
        let mask = &mask;
        let cursor: abi::Reg = cursor.into();
        let codepoint: abi::Reg = codepoint.into();
        let width: abi::Reg = width.into();

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

    pub(crate) fn emit_utf8_encoded_width(
        &mut self,
        codepoint: impl Into<abi::Reg>,
        width: impl Into<abi::Reg>,
    ) {
        let two = self.label("utf8_width_two");
        let three = self.label("utf8_width_three");
        let four = self.label("utf8_width_four");
        let done = self.label("utf8_width_done");
        let limit = self.temporary_vreg();
        let limit = &limit;
        let codepoint: abi::Reg = codepoint.into();
        let width: abi::Reg = width.into();

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

    pub(crate) fn emit_utf8_encode_next(
        &mut self,
        cursor: impl Into<abi::Reg>,
        codepoint: impl Into<abi::Reg>,
    ) {
        let two = self.label("utf8_encode_two");
        let three = self.label("utf8_encode_three");
        let four = self.label("utf8_encode_four");
        let done = self.label("utf8_encode_done");
        let byte = self.temporary_vreg();
        let mask = self.temporary_vreg();
        let byte = &byte;
        let mask = &mask;
        let cursor: abi::Reg = cursor.into();
        let codepoint: abi::Reg = codepoint.into();

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

    pub(crate) fn emit_string_byte_range_equal_branch(
        &mut self,
        left_data: impl Into<abi::Reg>,
        right_data: impl Into<abi::Reg>,
        length: impl Into<abi::Reg>,
        equal_label: &str,
        not_equal_label: &str,
    ) {
        let loop_label = self.label("string_bytes_equal_loop");
        let lptr = self.temporary_vreg();
        let rptr = self.temporary_vreg();
        let remaining = self.temporary_vreg();
        let lbyte = self.temporary_vreg();
        let lptr = &lptr;
        let rptr = &rptr;
        let remaining = &remaining;
        let lbyte = &lbyte;
        // Byte-compare scratch as a vreg. This helper backs `String` equality
        // and every substring predicate, so a raw physical-register write here
        // clobbers a caller value the allocator had placed there under register
        // pressure — a layout-sensitive miscompile that corrupted adjacent
        // string comparisons.
        let rbyte = self.temporary_vreg();
        let rbyte = &rbyte;
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

    pub(crate) fn emit_unicode_whitespace_branch(
        &mut self,
        cursor: impl Into<abi::Reg>,
        remaining: impl Into<abi::Reg>,
        width: impl Into<abi::Reg>,
        whitespace_label: &str,
        not_whitespace_label: &str,
    ) {
        let lead = self.temporary_vreg();
        let lead = &lead;
        // Continuation-byte scratch as a vreg. A raw physical-register write is
        // invisible to the register allocator and clobbers a caller value under
        // register pressure — a layout-sensitive miscompile.
        let cont = self.temporary_vreg();
        let cont = &cont;
        let cursor: abi::Reg = cursor.into();
        let remaining: abi::Reg = remaining.into();
        let width: abi::Reg = width.into();
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
