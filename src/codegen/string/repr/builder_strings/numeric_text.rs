//! plan-176-P: decimal text for `Fixed`, `Money` and `Float` values. String comparison and
//! ordering live in `string/repr/builder_strings/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// Sign/precision prologue shared verbatim by the Fixed and Money renderers:
    /// stash the raw magnitude and precision, take the absolute value, and record
    /// the sign. The callers mint every register and label (in the same order and
    /// with the same names), so this emit-only helper is byte-identical to the two
    /// copies it replaces. The scale-specific pre-round and int/frac split that
    /// follow stay inline at each site.
    fn emit_decimal_sign_prologue(
        &mut self,
        raw: impl Into<abi::Reg>,
        precision: impl Into<abi::Reg>,
        negative: impl Into<abi::Reg>,
        source_register: impl Into<abi::Reg>,
        precision_register: impl Into<abi::Reg>,
        precision_slot: usize,
        nonnegative: &str,
    ) {
        let raw = raw.into();
        let precision = precision.into();
        let negative = negative.into();
        self.emit(abi::move_register(raw, source_register));
        self.emit(abi::move_register(precision, precision_register));
        self.emit(abi::store_u64(
            precision,
            abi::stack_pointer(),
            precision_slot,
        ));
        self.emit(abi::move_immediate(negative, "Integer", "0"));
        self.emit(abi::compare_immediate(raw, "0"));
        self.emit(abi::branch_ge(nonnegative));
        self.emit(abi::subtract_registers(raw, abi::ZERO, raw));
        self.emit(abi::move_immediate(negative, "Integer", "1"));
        self.emit(abi::label(nonnegative));
    }

    /// Integer-part rendering shared by the Fixed and Money renderers: emit
    /// `int_part`'s decimal digits (or a single `0`) right-to-left into the buffer
    /// tail, prepend the sign, record the integer span, and compute the total
    /// length. Byte-identical; the callers own all registers, slots, and labels.
    /// `int_part`/`frac_part` are derived scale-specifically before this runs.
    #[allow(clippy::too_many_arguments)]
    fn emit_decimal_integer_render(
        &mut self,
        int_part: impl Into<abi::Reg>,
        cursor: impl Into<abi::Reg>,
        length: impl Into<abi::Reg>,
        divisor: impl Into<abi::Reg>,
        quotient: impl Into<abi::Reg>,
        digit: impl Into<abi::Reg>,
        negative: impl Into<abi::Reg>,
        total_len: impl Into<abi::Reg>,
        precision: impl Into<abi::Reg>,
        buffer_slot: usize,
        integer_start_slot: usize,
        integer_len_slot: usize,
        total_len_slot: usize,
        integer_zero: &str,
        integer_loop: &str,
        integer_done: &str,
        sign_done: &str,
        no_fraction: &str,
    ) {
        let int_part = int_part.into();
        let cursor = cursor.into();
        let length = length.into();
        let divisor = divisor.into();
        let quotient = quotient.into();
        let digit = digit.into();
        let total_len = total_len.into();
        let precision = precision.into();
        self.emit(abi::move_immediate(length, "Integer", "0"));
        self.emit(abi::add_immediate(
            cursor,
            abi::stack_pointer(),
            buffer_slot + 47,
        ));
        self.emit(abi::compare_immediate(int_part, "0"));
        self.emit(abi::branch_eq(integer_zero));
        self.emit(abi::move_immediate(divisor, "Integer", "10"));
        self.emit(abi::label(integer_loop));
        self.emit(abi::compare_immediate(int_part, "0"));
        self.emit(abi::branch_eq(integer_done));
        self.emit(abi::unsigned_divide_registers(quotient, int_part, divisor));
        self.emit(abi::multiply_subtract_registers(
            digit, quotient, divisor, int_part,
        ));
        self.emit(abi::add_immediate(digit, digit, b'0' as usize));
        self.emit(abi::store_u8(digit, cursor, 0));
        self.emit(abi::subtract_immediate(cursor, cursor, 1));
        self.emit(abi::add_immediate(length, length, 1));
        self.emit(abi::move_register(int_part, quotient));
        self.emit(abi::branch(integer_loop));

        self.emit(abi::label(integer_zero));
        self.emit(abi::move_immediate(
            digit,
            "Integer",
            &(b'0' as u64).to_string(),
        ));
        self.emit(abi::store_u8(digit, cursor, 0));
        self.emit(abi::subtract_immediate(cursor, cursor, 1));
        self.emit(abi::move_immediate(length, "Integer", "1"));

        self.emit(abi::label(integer_done));
        self.emit(abi::compare_immediate(negative, "0"));
        self.emit(abi::branch_eq(sign_done));
        self.emit(abi::move_immediate(
            digit,
            "Integer",
            &(b'-' as u64).to_string(),
        ));
        self.emit(abi::store_u8(digit, cursor, 0));
        self.emit(abi::subtract_immediate(cursor, cursor, 1));
        self.emit(abi::add_immediate(length, length, 1));
        self.emit(abi::label(sign_done));
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
        self.emit(abi::branch_eq(no_fraction));
        self.emit(abi::add_immediate(total_len, total_len, 1));
        self.emit(abi::add_registers(total_len, total_len, precision));
        self.emit(abi::label(no_fraction));
        self.emit(abi::store_u64(
            total_len,
            abi::stack_pointer(),
            total_len_slot,
        ));
    }

    /// Arena-allocate the String, copy the rendered integer part in, and emit the
    /// decimal point (skipping the fraction entirely when `precision == 0`) —
    /// shared verbatim by the Fixed and Money renderers. Byte-identical; the
    /// scale-specific fractional digit derivation follows inline at each site.
    #[allow(clippy::too_many_arguments)]
    fn emit_decimal_alloc_and_copy_integer(
        &mut self,
        total_len: impl Into<abi::Reg>,
        dst: impl Into<abi::Reg>,
        cursor: impl Into<abi::Reg>,
        length: impl Into<abi::Reg>,
        digit: impl Into<abi::Reg>,
        precision: impl Into<abi::Reg>,
        total_len_slot: usize,
        result_slot: usize,
        integer_start_slot: usize,
        integer_len_slot: usize,
        precision_slot: usize,
        alloc_ok: &str,
        copy_integer_loop: &str,
        copy_integer_done: &str,
        fraction_done: &str,
    ) -> Result<(), String> {
        let total_len = total_len.into();
        let dst = dst.into();
        let cursor = cursor.into();
        let length = length.into();
        let digit = digit.into();
        let precision = precision.into();
        self.emit(abi::add_immediate(abi::return_register(), total_len, 9));
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(alloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(alloc_ok));
        self.emit(abi::store_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            result_slot,
        ));
        self.emit(abi::load_u64(
            total_len,
            abi::stack_pointer(),
            total_len_slot,
        ));
        self.emit(abi::store_u64(total_len, abi::mfb_return(1), 0));
        self.emit(abi::add_immediate(dst, abi::mfb_return(1), 8));
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
        self.emit(abi::label(copy_integer_loop));
        self.emit(abi::compare_immediate(length, "0"));
        self.emit(abi::branch_eq(copy_integer_done));
        self.emit(abi::load_u8(digit, cursor, 0));
        self.emit(abi::store_u8(digit, dst, 0));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::add_immediate(dst, dst, 1));
        self.emit(abi::subtract_immediate(length, length, 1));
        self.emit(abi::branch(copy_integer_loop));
        self.emit(abi::label(copy_integer_done));

        self.emit(abi::load_u64(
            precision,
            abi::stack_pointer(),
            precision_slot,
        ));
        self.emit(abi::compare_immediate(precision, "0"));
        self.emit(abi::branch_eq(fraction_done));
        self.emit(abi::move_immediate(
            digit,
            "Integer",
            &(b'.' as u64).to_string(),
        ));
        self.emit(abi::store_u8(digit, dst, 0));
        self.emit(abi::add_immediate(dst, dst, 1));
        Ok(())
    }

    /// Fractional digit loop and NUL terminator shared by the Fixed and Money
    /// renderers: each digit is `(frac_part * 10) / scale` with the remainder fed
    /// back, for `precision` places. Byte-identical; `frac_part`/`scale` are set
    /// scale-specifically by the caller before this runs.
    #[allow(clippy::too_many_arguments)]
    fn emit_decimal_fraction_render(
        &mut self,
        frac_part: impl Into<abi::Reg>,
        digit: impl Into<abi::Reg>,
        dst: impl Into<abi::Reg>,
        counter: impl Into<abi::Reg>,
        divisor: impl Into<abi::Reg>,
        scale: impl Into<abi::Reg>,
        precision: impl Into<abi::Reg>,
        fraction_loop: &str,
        fraction_done: &str,
    ) {
        let frac_part = frac_part.into();
        let digit = digit.into();
        let dst = dst.into();
        let counter = counter.into();
        let scale = scale.into();
        self.emit(abi::label(fraction_loop));
        self.emit(abi::compare_registers(counter, precision));
        self.emit(abi::branch_eq(fraction_done));
        self.emit(abi::multiply_registers(frac_part, frac_part, divisor));
        self.emit(abi::unsigned_divide_registers(digit, frac_part, scale));
        self.emit(abi::multiply_subtract_registers(
            frac_part, digit, scale, frac_part,
        ));
        self.emit(abi::add_immediate(digit, digit, b'0' as usize));
        self.emit(abi::store_u8(digit, dst, 0));
        self.emit(abi::add_immediate(dst, dst, 1));
        self.emit(abi::add_immediate(counter, counter, 1));
        self.emit(abi::branch(fraction_loop));
        self.emit(abi::label(fraction_done));
        self.emit(abi::move_immediate(digit, "Integer", "0"));
        self.emit(abi::store_u8(digit, dst, 0));
    }

    pub(crate) fn emit_fixed_to_string_value(
        &mut self,
        source_register: impl Into<abi::Reg>,
        precision_register: impl Into<abi::Reg>,
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

        let raw = &scratch8;
        let negative = &scratch9;
        let int_part = &scratch10;
        let frac_part = &scratch11;
        let cursor = &scratch12;
        let length = &scratch13;
        let divisor = &scratch14;
        let quotient = &scratch15;
        let digit = &scratch16;
        let precision = &scratch17;
        let total_len = &scratch20;
        let dst = &scratch21;
        let counter = &scratch22;
        let scale = &scratch23;

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
        let exponent = &exponent_s;

        self.emit_decimal_sign_prologue(
            raw,
            precision,
            negative,
            source_register,
            precision_register,
            precision_slot,
            &nonnegative,
        );
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
        // `raw` is the unsigned magnitude (at most 2^63, the minimum Fixed) and the
        // bias at most 2^31, so the sum fits in 64 unsigned bits and the logical
        // shifts below split it correctly. No magnitude skips the bump: next to the
        // maximum Fixed it carries into an integer part of 2^31 (bug-818).
        self.emit(abi::add_registers(raw, raw, scale));
        self.emit(abi::label(&round_skip));
        self.emit(abi::store_u64(raw, abi::stack_pointer(), magnitude_slot));
        self.emit(abi::shift_right_immediate(int_part, raw, 32));
        self.emit(abi::shift_left_immediate(frac_part, raw, 32));
        self.emit(abi::shift_right_immediate(frac_part, frac_part, 32));
        self.emit_decimal_integer_render(
            int_part,
            cursor,
            length,
            divisor,
            quotient,
            digit,
            negative,
            total_len,
            precision,
            buffer_slot,
            integer_start_slot,
            integer_len_slot,
            total_len_slot,
            &integer_zero,
            &integer_loop,
            &integer_done,
            &sign_done,
            &no_fraction,
        );

        self.emit_decimal_alloc_and_copy_integer(
            total_len,
            dst,
            cursor,
            length,
            digit,
            precision,
            total_len_slot,
            result_slot,
            integer_start_slot,
            integer_len_slot,
            precision_slot,
            &alloc_ok,
            &copy_integer_loop,
            &copy_integer_done,
            &fraction_done,
        )?;
        self.emit(abi::load_u64(raw, abi::stack_pointer(), magnitude_slot));
        self.emit(abi::shift_left_immediate(frac_part, raw, 32));
        self.emit(abi::shift_right_immediate(frac_part, frac_part, 32));
        self.emit(abi::move_immediate(counter, "Integer", "0"));
        self.emit(abi::move_immediate(divisor, "Integer", "10"));
        self.emit(abi::move_immediate(scale, "Integer", "4294967296"));
        self.emit_decimal_fraction_render(
            frac_part,
            digit,
            dst,
            counter,
            divisor,
            scale,
            precision,
            &fraction_loop,
            &fraction_done,
        );

        let result = self.allocate_register();
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        // bug-536 shape B: the rendered decimal is this lowering's own
        // `result_slot` allocation; the argument is a scalar.
        self.mark_fresh_string(Operand::from(result));
        Ok(ValueResult {
            origin: None,
            type_: ParameterType::String,
            location: Operand::from(result),
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
    pub(crate) fn emit_money_to_string_value(
        &mut self,
        source_register: impl Into<abi::Reg>,
        precision_register: impl Into<abi::Reg>,
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

        let raw = &scratch8;
        let negative = &scratch9;
        let int_part = &scratch10;
        let frac_part = &scratch11;
        let cursor = &scratch12;
        let length = &scratch13;
        let divisor = &scratch14;
        let quotient = &scratch15;
        let digit = &scratch16;
        let precision = &scratch17;
        let total_len = &scratch20;
        let dst = &scratch21;
        let counter = &scratch22;
        let scale = &scratch23;
        let exponent = &scratch24;
        let remainder = &scratch25;

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

        self.emit_decimal_sign_prologue(
            raw,
            precision,
            negative,
            source_register,
            precision_register,
            precision_slot,
            &nonnegative,
        );

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
        self.emit_decimal_integer_render(
            int_part,
            cursor,
            length,
            divisor,
            quotient,
            digit,
            negative,
            total_len,
            precision,
            buffer_slot,
            integer_start_slot,
            integer_len_slot,
            total_len_slot,
            &integer_zero,
            &integer_loop,
            &integer_done,
            &sign_done,
            &no_fraction,
        );

        self.emit_decimal_alloc_and_copy_integer(
            total_len,
            dst,
            cursor,
            length,
            digit,
            precision,
            total_len_slot,
            result_slot,
            integer_start_slot,
            integer_len_slot,
            precision_slot,
            &alloc_ok,
            &copy_integer_loop,
            &copy_integer_done,
            &fraction_done,
        )?;
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
        self.emit_decimal_fraction_render(
            frac_part,
            digit,
            dst,
            counter,
            divisor,
            scale,
            precision,
            &fraction_loop,
            &fraction_done,
        );

        let result = self.allocate_register();
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        // bug-536 shape B: the rendered decimal is this lowering's own
        // `result_slot` allocation; the argument is a scalar.
        self.mark_fresh_string(Operand::from(result));
        Ok(ValueResult {
            origin: None,
            type_: ParameterType::String,
            location: Operand::from(result),
            text: "toString(Money)".to_string(),
        })
    }

    /// Render a finite Float to text via the in-tree exact `%.*f` formatter
    /// (`float_format.rs`, `_mfb_rt_float_to_string`) — no libc involvement.
    /// The helper takes the f64 bit pattern in `x0` and the precision in `x1`,
    /// and returns the arena-alloc Result convention (tag in `x0`, String
    /// pointer in `x1`); the only possible failure is allocation.
    pub(crate) fn emit_float_to_string_value(
        &mut self,
        source_register: impl Into<abi::Reg>,
        precision_register: impl Into<abi::Reg>,
    ) -> Result<ValueResult, String> {
        let alloc_ok = self.label("float_string_alloc_ok");
        self.emit(abi::move_register(abi::c_arg(0), source_register));
        self.emit(abi::move_register(abi::c_arg(1), precision_register));
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
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&alloc_ok));
        let result = self.allocate_register();
        self.emit(abi::move_register(&result, abi::mfb_return(1)));
        // bug-536 shape B: `_mfb_rt_float_to_string` ends in an
        // `_mfb_arena_alloc` + copy of its own digit buffer (`float_format.rs`),
        // and its argument is an f64 bit pattern — nothing of the caller's can
        // come back. Fresh.
        self.mark_fresh_string(Operand::from(result));
        Ok(ValueResult {
            origin: None,
            type_: ParameterType::String,
            location: Operand::from(result),
            text: "toString(Float)".to_string(),
        })
    }

    /// Call the scientific-digit helper (`float_format_sci.rs`,
    /// `_mfb_rt_float_to_string_sci`). Same convention as its fixed sibling
    /// above: the f64 bit pattern in `x0`, the arena-alloc Result out, and
    /// allocation the only failure.
    pub(crate) fn emit_sci_parts_call(
        &mut self,
        source_register: impl Into<abi::Reg>,
    ) -> Result<ValueResult, String> {
        let alloc_ok = self.label("sci_parts_alloc_ok");
        self.emit(abi::move_register(abi::c_arg(0), source_register));
        self.emit(abi::branch_link(FLOAT_TO_STRING_SCI_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: FLOAT_TO_STRING_SCI_SYMBOL.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&alloc_ok));
        let result = self.allocate_register();
        self.emit(abi::move_register(&result, abi::mfb_return(1)));
        // bug-536 shape B: `_mfb_rt_float_to_string_sci` ends in an
        // `_mfb_arena_alloc` (`float_format_sci.rs`) and takes an f64 bit
        // pattern, so its String is fresh.
        self.mark_fresh_string(Operand::from(result));
        Ok(ValueResult {
            origin: None,
            type_: ParameterType::String,
            location: Operand::from(result),
            text: "json::sciParts(Float)".to_string(),
        })
    }
}
