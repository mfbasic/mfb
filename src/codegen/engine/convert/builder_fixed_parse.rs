// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::target::shared::abi;

/// First fractional decimal digits that decide a Q32.32 rounding. A half quantum,
/// 2^-33, has exactly 33 fractional digits, so the first 33 digits settle the
/// rounding and every digit past them can only confirm it (mirrors
/// `numeric::fixed_conversion_from_decimal`, `DECIDING_DIGITS`).
const DECIDING_DIGITS: usize = 33;
/// The whole part may not exceed this: `2^31` is the magnitude of the minimum
/// `Fixed`, and anything above it overflows the 32-bit integer part.
const WHOLE_LIMIT: &str = "2147483648";

impl CodeBuilder<'_> {
    /// Parse a decimal `String` EXACTLY to its Q32.32 carrier in `dst`, with
    /// integer arithmetic only (no binary64), so `toFixed(String)` stores the
    /// value `numeric::fixed_raw_from_decimal` stores for the same text: the
    /// nearest carrier, ties away from zero, then the range check.
    ///
    /// Grammar (byte for byte the one `_mfb_rt_string_to_float` accepts, so no
    /// program's set of accepted strings changes): `[+-]? (digit | '.')*
    /// ([eE] [+-]? digit+)?` with at least one digit and at most one dot.
    ///
    /// Pass 1 validates the grammar and finds the mantissa digit count `nd`, the
    /// number of digits before the point `pp`, and the exponent `ex` (clamped, as
    /// the float scanner clamps it); the point then sits after digit `pp + ex`.
    /// Pass 2 accumulates the digits left of that point into the whole part
    /// (bounded to `2^31`) and stores the first [`DECIDING_DIGITS`] digits right
    /// of it into a zeroed stack buffer. The fraction is converted to binary by
    /// doubling that decimal buffer 33 times: each carry out of its top digit is
    /// the next fractional bit, so the 33rd is the round bit, and rounding half
    /// away from zero is `(bits + 1) >> 1` on the magnitude. The sign is applied
    /// last and the carrier is range-checked (positive up to `i64::MAX`, negative
    /// down to `i64::MIN`).
    ///
    /// Control leaves through `dst` set (fall through), `invalid_label`
    /// (malformed text) or `overflow_label` (outside the Fixed range).
    pub(crate) fn emit_parse_decimal_string_to_fixed_raw(
        &mut self,
        source_register: impl Into<abi::Reg>,
        dst: impl Into<abi::Reg>,
        invalid_label: &str,
        overflow_label: &str,
    ) -> Result<(), String> {
        let (source_register, dst): (abi::Reg, abi::Reg) = (source_register.into(), dst.into());
        let buffer = self.allocate_stack_object("fixed_parse_digits", DECIDING_DIGITS + 7);
        let length = self.temporary_vreg();
        let cursor = self.temporary_vreg();
        let index = self.temporary_vreg();
        let byte = self.temporary_vreg();
        let digit = self.temporary_vreg();
        let negative = self.temporary_vreg();
        let seen_digit = self.temporary_vreg();
        let dot_seen = self.temporary_vreg();
        let count = self.temporary_vreg(); // mantissa digits seen so far / total
        let point = self.temporary_vreg(); // digits before the point
        let exponent = self.temporary_vreg();
        let exp_negative = self.temporary_vreg();
        let mantissa_end = self.temporary_vreg();
        let whole = self.temporary_vreg();
        let used = self.temporary_vreg(); // buffer digits in use
        let ten = self.temporary_vreg();
        let limit = self.temporary_vreg();
        let scratch = self.temporary_vreg();
        let address = self.temporary_vreg();
        let other = self.temporary_vreg();
        let string = self.temporary_vreg();
        let start = self.temporary_vreg();
        let zero = self.temporary_vreg();

        let sign_minus = self.label("fixed_parse_minus");
        let sign_done = self.label("fixed_parse_sign_done");
        let scan_loop = self.label("fixed_parse_scan");
        let scan_dot = self.label("fixed_parse_scan_dot");
        let scan_next = self.label("fixed_parse_scan_next");
        let mantissa_done = self.label("fixed_parse_mantissa_done");
        let point_set = self.label("fixed_parse_point_set");
        let have_exponent = self.label("fixed_parse_have_exponent");
        let exp_sign_done = self.label("fixed_parse_exp_sign_done");
        let exp_not_plus = self.label("fixed_parse_exp_not_plus");
        let exp_loop = self.label("fixed_parse_exp_loop");
        let exp_next = self.label("fixed_parse_exp_next");
        let exp_done = self.label("fixed_parse_exp_done");
        let exp_positive = self.label("fixed_parse_exp_positive");
        let place_loop = self.label("fixed_parse_place");
        let place_end = self.label("fixed_parse_place_end");
        let sign_skip = self.label("fixed_parse_sign_skip");
        let place_fraction = self.label("fixed_parse_place_fraction");
        let place_next = self.label("fixed_parse_place_next");
        let pad_loop = self.label("fixed_parse_pad");
        let pad_done = self.label("fixed_parse_pad_done");
        let bits_done = self.label("fixed_parse_bits_done");
        let round_loop = self.label("fixed_parse_round");
        let digit_loop = self.label("fixed_parse_double");
        let digit_store = self.label("fixed_parse_double_store");
        let digit_done = self.label("fixed_parse_double_done");
        let negate = self.label("fixed_parse_negate");
        let set_done = self.label("fixed_parse_set_done");

        // ----- Pass 1: grammar, digit count, point position, exponent. -----
        self.emit(abi::move_register(&string, source_register));
        self.emit(abi::load_u64(&length, &string, 0));
        self.emit(abi::compare_immediate(&length, "0"));
        self.emit(abi::branch_eq(invalid_label));
        self.emit(abi::add_immediate(&cursor, &string, 8));
        self.emit(abi::move_immediate(&index, "Integer", "0"));
        self.emit(abi::move_immediate(&negative, "Integer", "0"));
        self.emit(abi::move_immediate(&seen_digit, "Integer", "0"));
        self.emit(abi::move_immediate(&dot_seen, "Integer", "0"));
        self.emit(abi::move_immediate(&count, "Integer", "0"));
        self.emit(abi::move_immediate(&point, "Integer", "0"));
        self.emit(abi::move_immediate(&exponent, "Integer", "0"));
        self.emit(abi::move_immediate(&ten, "Integer", "10"));
        self.emit(abi::load_u8(&byte, &cursor, 0));
        self.emit(abi::compare_immediate(&byte, "45")); // '-'
        self.emit(abi::branch_eq(&sign_minus));
        self.emit(abi::compare_immediate(&byte, "43")); // '+'
        self.emit(abi::branch_ne(&sign_done));
        self.emit(abi::branch(&sign_skip));
        self.emit(abi::label(&sign_minus));
        self.emit(abi::move_immediate(&negative, "Integer", "1"));
        self.emit(abi::label(&sign_skip));
        self.emit(abi::add_immediate(&index, &index, 1));
        self.emit(abi::add_immediate(&cursor, &cursor, 1));
        self.emit(abi::label(&sign_done));
        // The mantissa starts here; pass 2 restarts from this offset.
        self.emit(abi::move_register(&start, &index));

        self.emit(abi::label(&scan_loop));
        self.emit(abi::compare_registers(&index, &length));
        self.emit(abi::branch_ge(&mantissa_done));
        self.emit(abi::load_u8(&byte, &cursor, 0));
        self.emit(abi::compare_immediate(&byte, "46")); // '.'
        self.emit(abi::branch_eq(&scan_dot));
        self.emit(abi::compare_immediate(&byte, "101")); // 'e'
        self.emit(abi::branch_eq(&mantissa_done));
        self.emit(abi::compare_immediate(&byte, "69")); // 'E'
        self.emit(abi::branch_eq(&mantissa_done));
        self.emit(abi::compare_immediate(&byte, "48"));
        self.emit(abi::branch_lo(invalid_label));
        self.emit(abi::compare_immediate(&byte, "57"));
        self.emit(abi::branch_hi(invalid_label));
        self.emit(abi::move_immediate(&seen_digit, "Integer", "1"));
        self.emit(abi::add_immediate(&count, &count, 1));
        self.emit(abi::branch(&scan_next));
        self.emit(abi::label(&scan_dot));
        self.emit(abi::compare_immediate(&dot_seen, "0"));
        self.emit(abi::branch_ne(invalid_label)); // a second '.' is malformed
        self.emit(abi::move_immediate(&dot_seen, "Integer", "1"));
        self.emit(abi::move_register(&point, &count));
        self.emit(abi::label(&scan_next));
        self.emit(abi::add_immediate(&index, &index, 1));
        self.emit(abi::add_immediate(&cursor, &cursor, 1));
        self.emit(abi::branch(&scan_loop));

        self.emit(abi::label(&mantissa_done));
        self.emit(abi::compare_immediate(&seen_digit, "0"));
        self.emit(abi::branch_eq(invalid_label)); // no digits at all
        self.emit(abi::compare_immediate(&dot_seen, "0"));
        self.emit(abi::branch_ne(&point_set));
        self.emit(abi::move_register(&point, &count)); // no '.': the point is at the end
        self.emit(abi::label(&point_set));
        self.emit(abi::move_register(&mantissa_end, &index));
        self.emit(abi::compare_registers(&index, &length));
        self.emit(abi::branch_lt(&have_exponent));
        self.emit(abi::branch(&exp_done));

        // Optional exponent: [eE] [+-]? digit+, clamped like the float scanner.
        self.emit(abi::label(&have_exponent));
        self.emit(abi::add_immediate(&index, &index, 1));
        self.emit(abi::add_immediate(&cursor, &cursor, 1));
        self.emit(abi::compare_registers(&index, &length));
        self.emit(abi::branch_ge(invalid_label)); // 'e' with nothing after it
        self.emit(abi::move_immediate(&exp_negative, "Integer", "0"));
        self.emit(abi::load_u8(&byte, &cursor, 0));
        self.emit(abi::compare_immediate(&byte, "45")); // '-'
        self.emit(abi::branch_ne(&exp_not_plus));
        self.emit(abi::move_immediate(&exp_negative, "Integer", "1"));
        self.emit(abi::branch(&exp_sign_done));
        self.emit(abi::label(&exp_not_plus));
        self.emit(abi::compare_immediate(&byte, "43")); // '+'
        self.emit(abi::branch_ne(&exp_loop));
        self.emit(abi::label(&exp_sign_done));
        self.emit(abi::add_immediate(&index, &index, 1));
        self.emit(abi::add_immediate(&cursor, &cursor, 1));
        self.emit(abi::compare_registers(&index, &length));
        self.emit(abi::branch_ge(invalid_label)); // a sign with no digits
        self.emit(abi::label(&exp_loop));
        self.emit(abi::compare_registers(&index, &length));
        self.emit(abi::branch_ge(&exp_positive));
        self.emit(abi::load_u8(&byte, &cursor, 0));
        self.emit(abi::compare_immediate(&byte, "48"));
        self.emit(abi::branch_lo(invalid_label));
        self.emit(abi::compare_immediate(&byte, "57"));
        self.emit(abi::branch_hi(invalid_label));
        // Past 100000 every finite mantissa already overflows or rounds to zero.
        self.emit(abi::move_immediate(&scratch, "Integer", "100000"));
        self.emit(abi::compare_registers(&exponent, &scratch));
        self.emit(abi::branch_ge(&exp_next));
        self.emit(abi::subtract_immediate(&digit, &byte, 48));
        self.emit(abi::multiply_registers(&exponent, &exponent, &ten));
        self.emit(abi::add_registers(&exponent, &exponent, &digit));
        self.emit(abi::label(&exp_next));
        self.emit(abi::add_immediate(&index, &index, 1));
        self.emit(abi::add_immediate(&cursor, &cursor, 1));
        self.emit(abi::branch(&exp_loop));
        self.emit(abi::label(&exp_positive));
        self.emit(abi::compare_immediate(&exp_negative, "0"));
        self.emit(abi::branch_eq(&exp_done));
        self.emit(abi::move_immediate(&zero, "Integer", "0"));
        self.emit(abi::subtract_registers(&exponent, &zero, &exponent));
        self.emit(abi::label(&exp_done));
        // point = digits before the point, shifted by the exponent.
        self.emit(abi::add_registers(&point, &point, &exponent));

        // ----- Pass 2: place every mantissa digit. -----
        self.emit(abi::move_immediate(&scratch, "Integer", "0"));
        for word in 0..(DECIDING_DIGITS + 7) / 8 {
            self.emit(abi::store_u64(
                &scratch,
                abi::stack_pointer(),
                buffer + word * 8,
            ));
        }
        self.emit(abi::add_immediate(&address, abi::stack_pointer(), buffer));
        self.emit(abi::move_immediate(&whole, "Integer", "0"));
        self.emit(abi::move_immediate(&used, "Integer", "0"));
        self.emit(abi::move_immediate(&limit, "Integer", WHOLE_LIMIT));
        // `count` becomes the running digit index k; the total moves to `other`.
        self.emit(abi::move_register(&other, &count));
        self.emit(abi::move_immediate(&count, "Integer", "0"));
        self.emit(abi::move_register(&index, &start));
        // The byte cursor restarts at the first mantissa byte.
        self.emit(abi::add_immediate(&cursor, &string, 8));
        self.emit(abi::add_registers(&cursor, &cursor, &start));

        self.emit(abi::label(&place_loop));
        self.emit(abi::compare_registers(&index, &mantissa_end));
        self.emit(abi::branch_ge(&place_end));
        self.emit(abi::load_u8(&byte, &cursor, 0));
        self.emit(abi::compare_immediate(&byte, "46")); // '.'
        self.emit(abi::branch_eq(&place_next));
        self.emit(abi::subtract_immediate(&digit, &byte, 48));
        self.emit(abi::compare_registers(&count, &point));
        self.emit(abi::branch_ge(&place_fraction));
        // Left of the point: whole = whole * 10 + digit, bounded.
        self.emit(abi::multiply_registers(&whole, &whole, &ten));
        self.emit(abi::add_registers(&whole, &whole, &digit));
        self.emit(abi::compare_registers(&whole, &limit));
        self.emit(abi::branch_hi(overflow_label));
        self.emit(abi::add_immediate(&count, &count, 1));
        self.emit(abi::branch(&place_next));
        // Right of the point: fraction position f = k - point, kept while < 33.
        self.emit(abi::label(&place_fraction));
        self.emit(abi::subtract_registers(&scratch, &count, &point));
        self.emit(abi::add_immediate(&count, &count, 1));
        self.emit(abi::compare_immediate(
            &scratch,
            &DECIDING_DIGITS.to_string(),
        ));
        self.emit(abi::branch_ge(&place_next));
        self.emit(abi::add_registers(&byte, &address, &scratch));
        self.emit(abi::store_u8(&digit, &byte, 0));
        self.emit(abi::add_immediate(&used, &scratch, 1));
        self.emit(abi::label(&place_next));
        self.emit(abi::add_immediate(&index, &index, 1));
        self.emit(abi::add_immediate(&cursor, &cursor, 1));
        self.emit(abi::branch(&place_loop));

        // Digits missing between the last one and the point are zeros: shift the
        // whole part once per missing place (it is bounded, so this ends within a
        // handful of steps; a zero whole part stays zero).
        self.emit(abi::label(&place_end));
        self.emit(abi::subtract_registers(&scratch, &point, &other));
        self.emit(abi::label(&pad_loop));
        self.emit(abi::compare_immediate(&scratch, "0"));
        self.emit(abi::branch_le(&pad_done));
        self.emit(abi::compare_immediate(&whole, "0"));
        self.emit(abi::branch_eq(&pad_done));
        self.emit(abi::multiply_registers(&whole, &whole, &ten));
        self.emit(abi::compare_registers(&whole, &limit));
        self.emit(abi::branch_hi(overflow_label));
        self.emit(abi::subtract_immediate(&scratch, &scratch, 1));
        self.emit(abi::branch(&pad_loop));
        self.emit(abi::label(&pad_done));

        // ----- Fraction to 33 binary digits by doubling the decimal buffer. -----
        // `exponent` is reused as the accumulated bits, `count` as the round index.
        self.emit(abi::move_immediate(&exponent, "Integer", "0"));
        self.emit(abi::compare_immediate(&used, "0"));
        self.emit(abi::branch_eq(&bits_done));
        self.emit(abi::move_immediate(&count, "Integer", "0"));
        self.emit(abi::label(&round_loop));
        // Double the digits from the last one in use to the first.
        self.emit(abi::move_register(&index, &used));
        self.emit(abi::move_immediate(&digit, "Integer", "0")); // carry
        self.emit(abi::label(&digit_loop));
        self.emit(abi::compare_immediate(&index, "0"));
        self.emit(abi::branch_eq(&digit_done));
        self.emit(abi::subtract_immediate(&index, &index, 1));
        self.emit(abi::add_registers(&cursor, &address, &index));
        self.emit(abi::load_u8(&byte, &cursor, 0));
        self.emit(abi::add_registers(&byte, &byte, &byte));
        self.emit(abi::add_registers(&byte, &byte, &digit));
        self.emit(abi::move_immediate(&digit, "Integer", "0"));
        self.emit(abi::compare_immediate(&byte, "10"));
        self.emit(abi::branch_lo(&digit_store));
        self.emit(abi::subtract_immediate(&byte, &byte, 10));
        self.emit(abi::move_immediate(&digit, "Integer", "1"));
        self.emit(abi::label(&digit_store));
        self.emit(abi::store_u8(&byte, &cursor, 0));
        self.emit(abi::branch(&digit_loop));
        self.emit(abi::label(&digit_done));
        // The carry out of the top digit is the next fractional bit.
        self.emit(abi::add_registers(&exponent, &exponent, &exponent));
        self.emit(abi::add_registers(&exponent, &exponent, &digit));
        self.emit(abi::add_immediate(&count, &count, 1));
        self.emit(abi::compare_immediate(&count, "33"));
        self.emit(abi::branch_lt(&round_loop));
        self.emit(abi::label(&bits_done));
        // exponent = 33 fraction bits; round half away: (bits + 1) >> 1.
        self.emit(abi::add_immediate(&exponent, &exponent, 1));
        self.emit(abi::shift_right_immediate(&exponent, &exponent, 1));

        // ----- Assemble, apply the sign, range-check. -----
        self.emit(abi::shift_left_immediate(&whole, &whole, 32));
        self.emit(abi::add_registers(&whole, &whole, &exponent));
        self.emit(abi::compare_immediate(&negative, "0"));
        self.emit(abi::branch_ne(&negate));
        // Positive: the magnitude must fit i64::MAX.
        self.emit(abi::move_immediate(
            &scratch,
            "Integer",
            "9223372036854775807",
        ));
        self.emit(abi::compare_registers(&whole, &scratch));
        self.emit(abi::branch_hi(overflow_label));
        self.emit(abi::move_register(&dst, &whole));
        self.emit(abi::branch(&set_done));
        self.emit(abi::label(&negate));
        // Negative: the magnitude may reach 2^63 (i64::MIN); `0 - 2^63` wraps to it.
        self.emit(abi::move_immediate(
            &scratch,
            "Integer",
            "9223372036854775807",
        ));
        self.emit(abi::add_immediate(&scratch, &scratch, 1));
        self.emit(abi::compare_registers(&whole, &scratch));
        self.emit(abi::branch_hi(overflow_label));
        self.emit(abi::move_immediate(&zero, "Integer", "0"));
        self.emit(abi::subtract_registers(&dst, &zero, &whole));
        self.emit(abi::label(&set_done));
        Ok(())
    }
}
