//! plan-176-U: the numeric kernels — overflow and division checks, integer and Fixed power,
//! Fixed multiply/divide, abs/neg, numeric loads and the Float power. The module doc lives in
//! `engine/operators/builder_numeric/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// Raise `ErrOverflow` when the flags just set report a signed overflow. The
    /// raise is out of line (bug-774), so a passing check is one untaken branch.
    pub(crate) fn emit_overflow_if_flags_set(&mut self) -> Result<(), String> {
        let overflow = self.label("overflow");
        self.emit(abi::branch_vs(&overflow));
        self.emit_cold_block(&overflow, |builder| builder.raise_error_bare("ErrOverflow"))
    }

    pub(crate) fn emit_checked_integer_add(
        &mut self,
        dst: impl Into<abi::Reg>,
        lhs: impl Into<abi::Reg>,
        rhs: impl Into<abi::Reg>,
    ) -> Result<(), String> {
        self.emit(abi::add_registers_set_flags(dst, lhs, rhs));
        self.emit_overflow_if_flags_set()
    }

    pub(crate) fn emit_byte_upper_bound_check(
        &mut self,
        value: impl Into<abi::Reg>,
    ) -> Result<(), String> {
        let overflow_label = self.label("byte_overflow");
        self.emit(abi::compare_immediate(value, "255"));
        self.emit(abi::branch_hi(&overflow_label));
        self.emit_cold_block(&overflow_label, |builder| {
            builder.raise_error_bare("ErrOverflow")
        })
    }

    pub(crate) fn emit_checked_integer_multiply(
        &mut self,
        dst: impl Into<abi::Reg>,
        left: impl Into<abi::Reg>,
        right: impl Into<abi::Reg>,
    ) -> Result<(), String> {
        let dst = dst.into();
        let left = left.into();
        let right = right.into();
        let high = self.allocate_register();
        let sign = self.allocate_register();
        let overflow = self.label("mul_overflow");
        // Compute the signed high half from the original operands *before* the low
        // multiply writes `dst` — the pow loops call this with `dst == left`, so
        // reading `left` after `multiply_registers` would see the low product.
        self.emit(abi::signed_multiply_high_registers(&high, left, right));
        self.emit(abi::multiply_registers(dst, left, right));
        self.emit(abi::arithmetic_shift_right_immediate(&sign, dst, 63));
        self.emit(abi::compare_registers(&high, &sign));
        self.emit(abi::branch_ne(&overflow));
        self.emit_cold_block(&overflow, |builder| builder.raise_error_bare("ErrOverflow"))
    }

    pub(crate) fn emit_nonzero_or_invalid(
        &mut self,
        value: impl Into<abi::Reg>,
    ) -> Result<(), String> {
        let ok_label = self.label("nonzero");
        self.emit(abi::compare_immediate(value, "0"));
        self.emit(abi::branch_ne(&ok_label));
        self.raise_error_bare("ErrInvalidArgument")?;
        self.emit(abi::label(&ok_label));
        Ok(())
    }

    pub(crate) fn emit_integer_division_overflow_check(
        &mut self,
        left: impl Into<abi::Reg>,
        right: impl Into<abi::Reg>,
    ) -> Result<(), String> {
        let min = self.allocate_register();
        let minus_one = self.allocate_register();
        let not_min = self.label("div_not_min");
        let ok = self.label("div_overflow_ok");
        self.emit(abi::move_immediate(&min, "Integer", F64_SIGN_BIT));
        self.emit(abi::compare_registers(left, &min));
        self.emit(abi::branch_ne(&not_min));
        self.emit(abi::move_immediate(
            &minus_one,
            "Integer",
            &u64::MAX.to_string(),
        ));
        self.emit(abi::compare_registers(right, &minus_one));
        self.emit(abi::branch_ne(&ok));
        self.raise_error_bare("ErrOverflow")?;
        self.emit(abi::label(&not_min));
        self.emit(abi::label(&ok));
        Ok(())
    }

    pub(super) fn emit_min_i64_negation_check(
        &mut self,
        value: impl Into<abi::Reg>,
        label_prefix: &str,
    ) -> Result<(), String> {
        let min = self.allocate_register();
        let ok = self.label(&format!("{label_prefix}_not_min"));
        self.emit(abi::move_immediate(&min, "Integer", F64_SIGN_BIT));
        self.emit(abi::compare_registers(value, &min));
        self.emit(abi::branch_ne(&ok));
        self.raise_error_bare("ErrOverflow")?;
        self.emit(abi::label(&ok));
        Ok(())
    }

    pub(crate) fn emit_integer_pow(
        &mut self,
        dst: impl Into<abi::Reg>,
        base: impl Into<abi::Reg>,
        exponent: impl Into<abi::Reg>,
        byte_result: bool,
    ) -> Result<(), String> {
        let dst = dst.into();
        let base = base.into();
        let exponent = exponent.into();
        let loop_label = self.label("pow_loop");
        let done_label = self.label("pow_done");
        let nonnegative = self.label("pow_nonnegative");
        let remaining = self.allocate_register();
        self.emit(abi::compare_immediate(exponent, "0"));
        self.emit(abi::branch_ge(&nonnegative));
        self.raise_error_bare("ErrInvalidArgument")?;
        self.emit(abi::label(&nonnegative));
        self.emit(abi::move_register(&remaining, exponent));
        self.emit(abi::move_immediate(dst, "Integer", "1"));
        // Bounded-base fast path (bug-61): a base in {-1, 0, 1} has bounded powers,
        // so the checked-multiply loop below never overflows and would iterate the
        // full exponent (up to i64::MAX) — an effective hang. Resolve those bases
        // in closed form. Any |base| >= 2 falls through to the loop, which still
        // terminates within ~63 iterations via the multiply overflow trap.
        let bounded_zero = self.label("pow_bounded_zero");
        self.emit(abi::compare_immediate(base, "1"));
        self.emit(abi::branch_gt(&loop_label)); // base > 1: slow path.
        self.emit(abi::compare_immediate(base, &(-1_i64 as u64).to_string()));
        self.emit(abi::branch_lt(&loop_label)); // base < -1: slow path.
                                                // base is now one of {-1, 0, 1}; `dst` currently holds 1.
        self.emit(abi::compare_immediate(base, "0"));
        self.emit(abi::branch_eq(&bounded_zero));
        self.emit(abi::branch_gt(&done_label)); // base == 1: 1^n == 1.
                                                // base == -1: 1 for an even exponent, -1 for an odd exponent.
        let parity = self.allocate_register();
        let one_bit = self.allocate_register();
        self.emit(abi::move_immediate(&one_bit, "Integer", "1"));
        self.emit(abi::and_registers(&parity, exponent, &one_bit));
        self.emit(abi::compare_immediate(&parity, "0"));
        self.emit(abi::branch_eq(&done_label)); // even exponent: (-1)^n == 1.
        self.emit_neg_i64(dst)?; // odd exponent: (-1)^n == -1.
        self.emit(abi::branch(&done_label));
        self.emit(abi::label(&bounded_zero));
        // base == 0: 0^0 == 1 (dst already 1); 0^n == 0 for n > 0.
        self.emit(abi::compare_immediate(exponent, "0"));
        self.emit(abi::branch_eq(&done_label));
        self.emit(abi::move_immediate(dst, "Integer", "0"));
        self.emit(abi::branch(&done_label));
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate(&remaining, "0"));
        self.emit(abi::branch_eq(&done_label));
        self.emit_checked_integer_multiply(dst, dst, base)?;
        self.emit(abi::subtract_immediate(&remaining, &remaining, 1));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&done_label));
        if byte_result {
            self.emit_byte_upper_bound_check(dst)?;
        }
        Ok(())
    }

    pub(crate) fn emit_fixed_multiply(
        &mut self,
        dst: impl Into<abi::Reg>,
        left: impl Into<abi::Reg>,
        right: impl Into<abi::Reg>,
    ) -> Result<(), String> {
        // The working registers are dead once the product lands in `dst`.
        let saved_registers = self.next_register;
        let overflow = self.label("fixed_mul_overflow");
        let ok = self.label("fixed_mul_ok");
        self.emit_fixed_multiply_or_branch(dst, left, right, &overflow);
        self.emit(abi::branch(&ok));
        self.emit(abi::label(&overflow));
        self.raise_error_bare("ErrOverflow")?;
        self.emit(abi::label(&ok));
        self.next_register = saved_registers;
        Ok(())
    }

    /// The truncating Q32.32 product `dst = left * right`, branching to `overflow`
    /// (with `dst` clobbered) when it leaves the `Fixed` range instead of raising.
    /// Falls through with the product in `dst` otherwise. The caller owns the
    /// working registers' lifetime.
    pub(crate) fn emit_fixed_multiply_or_branch(
        &mut self,
        dst: impl Into<abi::Reg>,
        left: impl Into<abi::Reg>,
        right: impl Into<abi::Reg>,
        overflow: &str,
    ) {
        let dst = dst.into();
        let left = left.into();
        let right = right.into();
        let high = self.allocate_register();
        let shifted_high = self.allocate_register();
        let max_high = self.allocate_register();
        let min_high = self.allocate_register();
        // Compute the signed high half from the original operands *before* the low
        // multiply overwrites `dst` — the Fixed `^` loop calls this with
        // `dst == left`, so reading `left` afterwards would see the low product
        // and recombine garbage (bug-74).
        self.emit(abi::signed_multiply_high_registers(&high, left, right));
        self.emit(abi::multiply_registers(dst, left, right));
        self.emit(abi::move_immediate(&max_high, "Integer", "2147483647"));
        self.emit(abi::compare_registers(&high, &max_high));
        self.emit(abi::branch_gt(overflow));
        self.emit(abi::move_immediate(
            &min_high,
            "Integer",
            &(-2147483648_i64 as u64).to_string(),
        ));
        self.emit(abi::compare_registers(&high, &min_high));
        self.emit(abi::branch_lt(overflow));
        self.emit(abi::shift_right_immediate(dst, dst, 32));
        self.emit(abi::shift_left_immediate(&shifted_high, &high, 32));
        self.emit(abi::or_registers(dst, &shifted_high, dst));
    }

    pub(crate) fn emit_fixed_divide(
        &mut self,
        dst: impl Into<abi::Reg>,
        left: impl Into<abi::Reg>,
        right: impl Into<abi::Reg>,
    ) -> Result<(), String> {
        let dst = dst.into();
        let right = right.into();
        // The working registers are dead once the quotient lands in `dst`.
        let saved_registers = self.next_register;
        self.emit_nonzero_or_invalid(right)?;
        let lhs_abs = self.allocate_register();
        let rhs_abs = self.allocate_register();
        let sign = self.allocate_register();
        let integer = self.allocate_register();
        let remainder = self.allocate_register();
        let fraction = self.allocate_register();
        let counter = self.allocate_register();
        let bit = self.allocate_register();
        self.emit(abi::move_register(&lhs_abs, left));
        self.emit(abi::move_register(&rhs_abs, right));
        self.emit(abi::exclusive_or_registers(&sign, &lhs_abs, &rhs_abs));
        self.emit_abs_i64(&lhs_abs)?;
        self.emit_abs_i64(&rhs_abs)?;
        self.emit(abi::unsigned_divide_registers(&integer, &lhs_abs, &rhs_abs));
        self.emit(abi::multiply_subtract_registers(
            &remainder, &integer, &rhs_abs, &lhs_abs,
        ));
        let max_integer = self.allocate_register();
        let overflow = self.label("fixed_div_overflow");
        let integer_ok = self.label("fixed_div_integer_ok");
        // Admit an integer magnitude up to 2^31 (not 2^31 - 1). The exact result
        // -2147483648.0 (raw i64::MIN) has magnitude integer part 2^31 with a zero
        // fraction; rejecting it here wrongly traps its own representable minimum
        // (bug-61). Anything strictly above 2^31 cannot fit even the negative range
        // and would corrupt the `integer << 32` below, so it still traps. The final
        // signed range check (see below) rejects the sub-cases of 2^31 that are not
        // representable (a positive result, or a negative one with a nonzero
        // fraction).
        self.emit(abi::move_immediate(&max_integer, "Integer", "2147483648"));
        self.emit(abi::compare_registers(&integer, &max_integer));
        self.emit(abi::branch_hi(&overflow));
        self.emit(abi::shift_left_immediate(dst, &integer, 32));
        self.emit(abi::move_immediate(&fraction, "Integer", "0"));
        self.emit(abi::move_immediate(&counter, "Integer", "32"));
        self.emit(abi::branch(&integer_ok));
        self.emit(abi::label(&overflow));
        self.raise_error_bare("ErrOverflow")?;
        self.emit(abi::label(&integer_ok));

        self.emit_q32_fraction_bits(&remainder, &fraction, &rhs_abs, &counter, &bit);
        self.emit(abi::or_registers(dst, dst, &fraction));
        // `dst` now holds the unsigned magnitude of the quotient in Q32.32. Apply
        // the sign and range-check the signed result the way `emit_fixed_multiply`
        // does, so the representable minimum -2147483648.0 (raw i64::MIN, magnitude
        // 2^63) is admitted while genuine overflows still trap (bug-61).
        let negative = self.label("fixed_div_negative");
        let negate = self.label("fixed_div_negate");
        let magnitude_overflow = self.label("fixed_div_mag_overflow");
        let quotient_done = self.label("fixed_div_signed");
        self.emit(abi::compare_immediate(&sign, "0"));
        self.emit(abi::branch_lt(&negative));
        // Positive result: the magnitude must fit a signed i64 (top bit clear). A
        // magnitude of 2^63 (exactly +2147483648.0) has the top bit set, so it
        // correctly traps here — only the negative form of that magnitude is
        // representable.
        self.emit(abi::compare_immediate(dst, "0"));
        self.emit(abi::branch_ge(&quotient_done));
        self.emit(abi::branch(&magnitude_overflow));
        self.emit(abi::label(&negative));
        // Negative result = -(magnitude). Representable iff magnitude <= 2^63.
        //  - magnitude < 2^63  (dst >= 0 as signed): negate normally.
        //  - magnitude == 2^63 (dst == i64::MIN): the result is exactly
        //    -2147483648.0, whose raw value is i64::MIN; negating i64::MIN is a
        //    no-op, so keep `dst` unchanged.
        //  - magnitude > 2^63: overflow.
        self.emit(abi::compare_immediate(dst, "0"));
        self.emit(abi::branch_ge(&negate));
        let min_raw = self.allocate_register();
        self.emit(abi::move_immediate(
            &min_raw,
            "Integer",
            &(i64::MIN as u64).to_string(),
        ));
        self.emit(abi::compare_registers(dst, &min_raw));
        self.emit(abi::branch_eq(&quotient_done));
        self.emit(abi::label(&magnitude_overflow));
        self.raise_error_bare("ErrOverflow")?;
        self.emit(abi::label(&negate));
        self.emit_neg_i64(dst)?;
        self.emit(abi::label(&quotient_done));
        self.next_register = saved_registers;
        Ok(())
    }

    /// Restoring long division for the 32 fractional bits of a Q32.32 quotient:
    /// on entry `remainder < divisor` is the remainder of the integer-part
    /// division, `fraction` is 0 and `counter` is 32; on exit `fraction` holds
    /// `floor(remainder * 2^32 / divisor)` and `remainder` what is left of it.
    /// All three are unsigned magnitudes (`divisor <= 2^63`, so the doubled
    /// remainder stays below 2^64). `bit` is a scratch register. Shared by the
    /// truncating `emit_fixed_divide` and the mode-rounded Money/Fixed rescale
    /// (`emit_money_fixed_divide`), which reads the final remainder.
    pub(crate) fn emit_q32_fraction_bits(
        &mut self,
        remainder: impl Into<abi::Reg>,
        fraction: impl Into<abi::Reg>,
        divisor: impl Into<abi::Reg>,
        counter: impl Into<abi::Reg>,
        bit: impl Into<abi::Reg>,
    ) {
        let (remainder, fraction, divisor, counter, bit): (
            abi::Reg,
            abi::Reg,
            abi::Reg,
            abi::Reg,
            abi::Reg,
        ) = (
            remainder.into(),
            fraction.into(),
            divisor.into(),
            counter.into(),
            bit.into(),
        );
        let loop_start = self.label("fixed_div_loop");
        let skip_subtract = self.label("fixed_div_skip_subtract");
        let done = self.label("fixed_div_done");
        self.emit(abi::label(&loop_start));
        self.emit(abi::compare_immediate(counter, "0"));
        self.emit(abi::branch_eq(&done));
        self.emit(abi::shift_left_immediate(remainder, remainder, 1));
        self.emit(abi::shift_left_immediate(fraction, fraction, 1));
        self.emit(abi::compare_registers(remainder, divisor));
        self.emit(abi::branch_lo(&skip_subtract));
        self.emit(abi::subtract_registers(remainder, remainder, divisor));
        self.emit(abi::move_immediate(bit, "Integer", "1"));
        self.emit(abi::or_registers(fraction, fraction, bit));
        self.emit(abi::label(&skip_subtract));
        self.emit(abi::subtract_immediate(counter, counter, 1));
        self.emit(abi::branch(&loop_start));
        self.emit(abi::label(&done));
    }

    /// `Fixed ^ Integer-valued Fixed` for the `^` operator: rejects a negative or
    /// non-whole exponent up front, then runs the bug-61/bug-74 ±1.0 closed form
    /// and the truncate-to-zero multiply loop.
    ///
    /// Shares that fast-path *shape* (and the copied bug-61/bug-74 comments) with
    /// `gen_fixed_math::emit_fixed_pow_general`'s integer branch (bug-332 E2),
    /// but the two are deliberately **not** merged: they accept different domains
    /// (`emit_fixed_pow_general` handles negative exponents via a reciprocal tail
    /// and fractional exponents via `exp(y·ln x)`), and even the shared multiply
    /// loop diverges in its emitted instructions — this path multiplies in place
    /// (`emit_fixed_multiply(dst, dst, base)`) while the general path multiplies
    /// through a separate `product` register and moves it back. No zero-diff
    /// extraction spans both, so they stay separate.
    pub(crate) fn emit_fixed_pow(
        &mut self,
        dst: impl Into<abi::Reg>,
        base: impl Into<abi::Reg>,
        exponent: impl Into<abi::Reg>,
    ) -> Result<(), String> {
        let dst = dst.into();
        let base = base.into();
        let exponent = exponent.into();
        let one_raw = 1_u64 << 32;
        let remaining = self.allocate_register();
        let whole = self.allocate_register();
        let nonnegative = self.label("fixed_pow_nonnegative");
        let loop_label = self.label("fixed_pow_loop");
        let done_label = self.label("fixed_pow_done");
        self.emit(abi::compare_immediate(exponent, "0"));
        self.emit(abi::branch_ge(&nonnegative));
        self.raise_error_bare("ErrInvalidArgument")?;
        self.emit(abi::label(&nonnegative));
        self.emit(abi::arithmetic_shift_right_immediate(&whole, exponent, 32));
        self.emit(abi::shift_left_immediate(&remaining, &whole, 32));
        self.emit(abi::compare_registers(&remaining, exponent));
        let exponent_is_whole = self.label("fixed_pow_whole");
        self.emit(abi::branch_eq(&exponent_is_whole));
        self.raise_error_bare("ErrInvalidArgument")?;
        self.emit(abi::label(&exponent_is_whole));
        self.emit(abi::move_register(&remaining, &whole));
        self.emit(abi::move_immediate(dst, "Fixed", &one_raw.to_string()));
        // Bounded-base fast path (bug-61): |base| == 1.0 has bounded powers, so the
        // loop's only exit (the multiply overflow trap) never fires and it would
        // iterate the full exponent. Resolve ±1.0 in closed form.
        // Compare against ±1.0 through registers, not `compare_immediate`: the raw
        // Fixed constants are `±2^32`, which exceed the x86 CMP imm32 field and
        // fail to encode (bug-74). `dst` already holds `one_raw`.
        let neg_one_raw = -(one_raw as i64) as u64;
        let neg_one_reg = self.allocate_register();
        self.emit(abi::move_immediate(
            &neg_one_reg,
            "Fixed",
            &neg_one_raw.to_string(),
        ));
        self.emit(abi::compare_registers(base, dst));
        self.emit(abi::branch_eq(&done_label)); // 1.0^n == 1.0 (dst already 1.0).
        self.emit(abi::compare_registers(base, &neg_one_reg));
        self.emit(abi::branch_ne(&loop_label)); // |base| != 1.0: enter the loop.
                                                // base == -1.0: 1.0 for an even exponent, -1.0 for an odd exponent.
        let parity = self.allocate_register();
        let one_bit = self.allocate_register();
        self.emit(abi::move_immediate(&one_bit, "Integer", "1"));
        self.emit(abi::and_registers(&parity, &whole, &one_bit));
        self.emit(abi::compare_immediate(&parity, "0"));
        self.emit(abi::branch_eq(&done_label)); // even exponent: (-1.0)^n == 1.0.
        self.emit_neg_i64(dst)?; // odd exponent: (-1.0)^n == -1.0.
        self.emit(abi::branch(&done_label));
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate(&remaining, "0"));
        self.emit(abi::branch_eq(&done_label));
        // A product that truncates to 0 (any |base| < 1.0, or base == 0.0) stays 0
        // for every remaining multiply, so stop now rather than iterate the whole
        // (possibly enormous) exponent (bug-61). This never changes a result.
        self.emit(abi::compare_immediate(dst, "0"));
        self.emit(abi::branch_eq(&done_label));
        self.emit_fixed_multiply(dst, dst, base)?;
        self.emit(abi::subtract_immediate(&remaining, &remaining, 1));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&done_label));
        Ok(())
    }

    pub(crate) fn emit_abs_i64(&mut self, register: impl Into<abi::Reg>) -> Result<(), String> {
        let register = register.into();
        let positive = self.label("abs_positive");
        self.emit(abi::compare_immediate(register, "0"));
        self.emit(abi::branch_ge(&positive));
        self.emit_neg_i64(register)?;
        self.emit(abi::label(&positive));
        Ok(())
    }

    pub(crate) fn emit_neg_i64(&mut self, register: impl Into<abi::Reg>) -> Result<(), String> {
        let register = register.into();
        self.emit(abi::subtract_registers(register, abi::ZERO, register));
        Ok(())
    }

    pub(crate) fn load_numeric_as_double(
        &mut self,
        dst: impl Into<abi::Reg>,
        value: &ValueResult,
    ) -> Result<(), String> {
        let dst = dst.into();
        let fixed_scratch = self.temporary_vreg();
        match &value.type_ {
            // A `d`-native float is already in an FP register: move it `d`-to-`d`
            // (no GPR round-trip). A GP-native float carries its bits in `value
            // .location` and is moved across the `fmov d, x` boundary.
            ParameterType::Float if Self::float_is_dnative(value) => {
                self.emit(abi::float_move_d_from_d(dst, &value.location))
            }
            ParameterType::Float => self.emit(abi::float_move_d_from_x(dst, &value.location)),
            ParameterType::Byte | ParameterType::Integer => {
                self.emit(abi::signed_convert_to_float_d(dst, &value.location))
            }
            ParameterType::Fixed => {
                self.emit(abi::signed_convert_to_float_d(dst, &value.location));
                self.emit_f64_const(abi::FP_SCRATCH[7], &fixed_scratch, 4_294_967_296.0);
                self.emit(abi::float_divide_d(dst, dst, abi::FP_SCRATCH[7]));
            }
            // Money's f64 value is its raw scaled i64 divided by the base-10 scale
            // (the SCALE cancels in `M / M`; used by the ratio, DIV, and
            // `toFloat(Money)` paths) (plan-29-E §4.3).
            ParameterType::Money => {
                self.emit(abi::signed_convert_to_float_d(dst, &value.location));
                self.emit_f64_const(abi::FP_SCRATCH[7], &fixed_scratch, 100_000.0);
                self.emit(abi::float_divide_d(dst, dst, abi::FP_SCRATCH[7]));
            }
            other => {
                return Err(format!(
                    "native Float arithmetic cannot load operand type '{other}'"
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn load_numeric_as_fixed(
        &mut self,
        dst: impl Into<abi::Reg>,
        value: &ValueResult,
    ) -> Result<(), String> {
        let dst = dst.into();
        match &value.type_ {
            ParameterType::Fixed => self.emit(abi::move_register(dst, &value.location)),
            ParameterType::Byte | ParameterType::Integer => {
                self.emit_integer_to_fixed_value(&value.location, dst)?
            }
            ParameterType::Float => {
                // The Float→Fixed conversion reads the f64 bit pattern, so a
                // `d`-native float is materialized into a GPR first (plan-01).
                let bits = self.float_value_as_gpr(value)?;
                self.emit_float_bits_to_fixed_value(&bits, dst)?
            }
            other => {
                return Err(format!(
                    "native Fixed comparison cannot load operand type '{other}'"
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn emit_f64_const(
        &mut self,
        dst: impl Into<abi::Reg>,
        scratch: impl Into<abi::Reg>,
        value: f64,
    ) {
        let scratch = scratch.into();
        self.emit(abi::move_immediate(
            scratch,
            "Integer",
            &value.to_bits().to_string(),
        ));
        self.emit(abi::float_move_d_from_x(dst, scratch));
    }

    pub(crate) fn emit_float_pow(
        &mut self,
        dst: impl Into<abi::Reg>,
        exponent: impl Into<abi::Reg>,
    ) -> Result<(), String> {
        let dst = dst.into();
        let exponent = exponent.into();
        // `dst` holds the base f64; `exponent` the exponent f64. The `^` operator
        // on Float is defined only for a whole, non-negative exponent
        // (ErrFloatDomain otherwise), so keep those two domain guards. The actual
        // power was previously computed by repeated multiplication whose only
        // loop exit was `exponent == 0`: a large whole exponent (e.g. `1.0e18`)
        // ran ~1e18 iterations and effectively hung the program. Compute via the
        // fdlibm scalar-pow kernel — the same one `math::pow` uses (unaffected) —
        // which is O(1) and, for the whole exponents this operator admits, gives
        // the mathematically correct result (bug-135). Overflow to Inf stays an
        // anonymous intermediate trapped at the observation boundary (plan-17),
        // so no inline result check is emitted here.
        let nonnegative = self.label("float_pow_nonnegative");
        let exponent_whole = self.label("float_pow_whole");
        self.emit(abi::float_compare_zero_d(exponent));
        self.emit(abi::branch_ge(&nonnegative));
        self.raise_error_bare("ErrFloatDomain")?;
        self.emit(abi::label(&nonnegative));
        let exponent_int = self.allocate_register();
        let exponent_roundtrip = self.allocate_register();
        let exponent_bits = self.allocate_register();
        // bug-312 K3: the round-trip whole-number test below converts with a
        // SATURATING `fcvtzs`. An exponent whose magnitude exceeds i64::MAX (e.g.
        // 1.0e19) saturates to i64::MAX, whose f64 round-trip is unequal, so it was
        // reported as ErrFloatDomain -- a fractional-exponent rejection -- for an
        // exponent that is in fact whole. `1.0 ^ 1.0e19` is 1.0 and `0.5 ^ 1.0e19`
        // is 0.0; both lost their value, and for base > 1 the correct
        // ErrFloatOverflow was mis-coded as ErrFloatDomain.
        //
        // Every finite f64 at or above 2^52 is already an integer (the mantissa has
        // no fractional bits left), so such an exponent is whole by construction and
        // needs no round-trip. The exponent is known non-negative here, so its sign
        // bit is 0 and `bits >> 52` is exactly the biased exponent; 2^52 is biased
        // 1075. The upper bound excludes 2047 (Inf/NaN), which is NOT a whole number
        // and must keep falling through to the domain rejection.
        let biased = self.allocate_register();
        let not_huge = self.label("float_pow_not_huge");
        self.emit(abi::float_move_x_from_d(&exponent_bits, exponent));
        self.emit(abi::shift_right_immediate(&biased, &exponent_bits, 52));
        self.emit(abi::compare_immediate(&biased, "1075"));
        self.emit(abi::branch_lt(&not_huge));
        self.emit(abi::compare_immediate(&biased, "2047"));
        self.emit(abi::branch_ge(&not_huge));
        self.emit(abi::branch(&exponent_whole));
        self.emit(abi::label(&not_huge));
        self.emit(abi::float_convert_to_signed_x(&exponent_int, exponent));
        self.emit(abi::signed_convert_to_float_d(
            abi::FP_SCRATCH[2],
            &exponent_int,
        ));
        self.emit(abi::float_move_x_from_d(
            &exponent_roundtrip,
            abi::FP_SCRATCH[2],
        ));
        self.emit(abi::float_move_x_from_d(&exponent_bits, exponent));
        self.emit(abi::compare_registers(&exponent_roundtrip, &exponent_bits));
        self.emit(abi::branch_eq(&exponent_whole));
        self.raise_error_bare("ErrFloatDomain")?;
        self.emit(abi::label(&exponent_whole));
        // Spill the base and exponent bit patterns before invoking the kernel,
        // which resets the register file (mirroring the `MOD`/fmod path).
        let base_slot = self.allocate_stack_object("float_pow_base", 8);
        let exp_slot = self.allocate_stack_object("float_pow_exp", 8);
        let base_bits = self.allocate_register();
        self.emit(abi::float_move_x_from_d(&base_bits, dst));
        self.emit(abi::store_u64(&base_bits, abi::stack_pointer(), base_slot));
        self.emit(abi::store_u64(
            &exponent_bits,
            abi::stack_pointer(),
            exp_slot,
        ));
        self.reset_temporary_registers();
        let base_reg = self.allocate_register();
        self.emit(abi::load_u64(&base_reg, abi::stack_pointer(), base_slot));
        let exp_reg = self.allocate_register();
        self.emit(abi::load_u64(&exp_reg, abi::stack_pointer(), exp_slot));
        let result = self.emit_pow_scalar(&base_reg, &exp_reg)?;
        self.emit(abi::float_move_d_from_x(dst, &result));
        Ok(())
    }
}
