//! plan-176-S: fixed-point exp, log and general pow. The constants and the multiply/sqrt kernels
//! live in `builtins/money/gen_fixed_math/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// Lower `exp` for a `Fixed` argument. Computes `2^n * exp(r)` with
    /// `n = round(x / ln2)` and `r = x - n*ln2`, evaluating `exp(r)` by a Taylor
    /// series. Overflow beyond `Fixed` range fails with `ErrOverflow`.
    pub(crate) fn emit_fixed_exp(
        &mut self,
        src: impl Into<Reg>,
    ) -> Result<VirtualRegister, String> {
        let src: Reg = src.into();
        let x_slot = self.allocate_stack_object("fixed_exp_x", 8);
        let result_slot = self.allocate_stack_object("fixed_exp_result", 8);
        self.emit(abi::store_u64(src, abi::stack_pointer(), x_slot));
        self.reset_temporary_registers();
        let x = self.allocate_register();
        self.emit(abi::load_u64(&x, abi::stack_pointer(), x_slot));
        // bug-659: gate the argument BEFORE the reduction below. `x / ln2` is an
        // unchecked Q32.32 multiply, so for |x| > 2^31*ln2 = 1.4885e9 the product
        // leaves `Fixed` range and wraps, and `n` comes out with the WRONG SIGN.
        // `emit_fixed_scale_by_power_of_two` dispatches on that sign — the n >= 0
        // arm doubles with an `ErrOverflow` check, the n < 0 arm halves with no
        // check at all — so a wrapped sign selected the opposite arm and silently
        // swapped overflow with underflow (exp(-2e9) raised, exp(2e9) returned
        // 0.00). Both outcomes are already settled far below the wrap point:
        // `Fixed` tops out just under 2^31, so the result overflows for
        // x > ln(2^31) = 21.4876 and rounds to zero for x < -33*ln2 = -22.8742.
        // Gating at |x| = 64 sits comfortably outside both thresholds, so no
        // in-range result changes, and comfortably inside the wrap point, so the
        // reduction can no longer wrap (|64/ln2| < 93).
        let bound = self.allocate_register();
        let within_upper = self.label("fixed_exp_within_upper");
        let in_range = self.label("fixed_exp_in_range");
        let done = self.label("fixed_exp_done");
        // Compare through a register, never `compare_immediate`: the raw bound is
        // 64*2^32, far past the x86 CMP imm32 field (bug-74).
        self.emit(abi::move_immediate(
            &bound,
            abi::IMMEDIATE_CLASS_FIXED,
            &FIXED_EXP_ARGUMENT_BOUND.to_string(),
        ));
        self.emit(abi::compare_registers(&x, &bound));
        self.emit(abi::branch_le(&within_upper));
        self.raise_error_bare("ErrOverflow")?;
        self.emit(abi::label(&within_upper));
        // Re-materialise the bound rather than negating the register: the raise
        // above is a non-returning tail that the allocator treats as clobbering.
        self.emit(abi::move_immediate(
            &bound,
            abi::IMMEDIATE_CLASS_FIXED,
            &(FIXED_EXP_ARGUMENT_BOUND.wrapping_neg()).to_string(),
        ));
        self.emit(abi::compare_registers(&x, &bound));
        self.emit(abi::branch_ge(&in_range));
        // x < -64: e^x is below half a Q32.32 unit, so it rounds to exactly 0.00.
        self.emit(abi::move_immediate(&bound, abi::IMMEDIATE_CLASS_FIXED, "0"));
        self.emit(abi::store_u64(&bound, abi::stack_pointer(), result_slot));
        self.emit(abi::branch(&done));
        self.emit(abi::label(&in_range));
        // n = round(x / ln2).
        let inv_ln2 = self.allocate_register();
        self.emit_const_i64(&inv_ln2, fixed_inv_ln2());
        let scaled = self.emit_fixed_mul(&x, &inv_ln2)?;
        let n = self.allocate_register();
        self.emit(abi::move_immediate(&n, "Integer", &FIXED_HALF.to_string()));
        self.emit(abi::add_registers(&n, &scaled, &n));
        self.emit(abi::arithmetic_shift_right_immediate(&n, &n, 32));
        // r = x - n*ln2 (mod 2^64).
        let ln2 = self.allocate_register();
        self.emit_const_i64(&ln2, fixed_ln2());
        let nl = self.allocate_register();
        self.emit(abi::multiply_registers(&nl, &n, &ln2));
        let r = self.allocate_register();
        self.emit(abi::subtract_registers(&r, &x, &nl));
        // exp(r) via Taylor series: sum = 1 + r + r^2/2! + ...
        let sum = self.allocate_register();
        let term = self.allocate_register();
        let k = self.allocate_register();
        let counter = self.allocate_register();
        let s0 = self.allocate_register();
        let s1 = self.allocate_register();
        self.emit(abi::move_immediate(
            &sum,
            abi::IMMEDIATE_CLASS_FIXED,
            &FIXED_ONE.to_string(),
        ));
        self.emit(abi::move_immediate(
            &term,
            abi::IMMEDIATE_CLASS_FIXED,
            &FIXED_ONE.to_string(),
        ));
        self.emit(abi::move_immediate(&k, "Integer", "1"));
        self.emit(abi::move_immediate(&counter, "Integer", "18"));
        let series = self.label("fixed_exp_series");
        let series_done = self.label("fixed_exp_series_done");
        self.emit(abi::label(&series));
        self.emit(abi::compare_immediate(&counter, "0"));
        self.emit(abi::branch_eq(&series_done));
        self.emit_fixed_mul_inplace(&term, &term, &r, &s0, &s1);
        self.emit(abi::signed_divide_registers(&term, &term, &k));
        self.emit(abi::add_registers(&sum, &sum, &term));
        self.emit(abi::add_immediate(&k, &k, 1));
        self.emit(abi::subtract_immediate(&counter, &counter, 1));
        self.emit(abi::branch(&series));
        self.emit(abi::label(&series_done));
        // result = sum << n (n >= 0) or sum >> -n (n < 0), with overflow guard.
        self.emit_fixed_scale_by_power_of_two(&sum, &n)?;
        self.emit(abi::store_u64(&sum, abi::stack_pointer(), result_slot));
        self.emit(abi::label(&done));
        self.reset_temporary_registers();
        let result = self.allocate_register();
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(result)
    }

    /// Multiply `value` (a `Fixed`) by `2^n` in place where `n` is a runtime
    /// signed integer, trapping with `ErrOverflow` if the result leaves `Fixed`
    /// range. Used to recombine the exponent in `exp`/`pow`.
    fn emit_fixed_scale_by_power_of_two(
        &mut self,
        value: impl Into<Reg>,
        n: impl Into<Reg>,
    ) -> Result<(), String> {
        let (value, n): (Reg, Reg) = (value.into(), n.into());
        let count = self.allocate_register();
        let limit = self.allocate_register();
        let negative = self.label("fixed_scale_negative");
        let up_loop = self.label("fixed_scale_up");
        let up_done = self.label("fixed_scale_up_done");
        let down_loop = self.label("fixed_scale_down");
        let down_done = self.label("fixed_scale_down_done");
        let no_overflow = self.label("fixed_scale_no_overflow");
        self.emit(abi::compare_immediate(n, "0"));
        self.emit(abi::branch_lt(&negative));
        // n >= 0: double `count` times, checking for overflow before each shift.
        self.emit(abi::move_register(&count, n));
        // limit = i64::MAX / 2; if value > limit a doubling would overflow.
        self.emit(abi::move_immediate(
            &limit,
            "Integer",
            &(i64::MAX as u64 / 2).to_string(),
        ));
        self.emit(abi::label(&up_loop));
        self.emit(abi::compare_immediate(&count, "0"));
        self.emit(abi::branch_eq(&up_done));
        self.emit(abi::compare_registers(value, &limit));
        self.emit(abi::branch_le(&no_overflow));
        self.raise_error_bare("ErrOverflow")?;
        self.emit(abi::label(&no_overflow));
        self.emit(abi::shift_left_immediate(value, value, 1));
        self.emit(abi::subtract_immediate(&count, &count, 1));
        self.emit(abi::branch(&up_loop));
        self.emit(abi::label(&up_done));
        let finish = self.label("fixed_scale_finish");
        self.emit(abi::branch(&finish));
        // n < 0: halve `-n` times (arithmetic shift; value is non-negative).
        self.emit(abi::label(&negative));
        self.emit(abi::subtract_registers(&count, abi::ZERO, n));
        self.emit(abi::label(&down_loop));
        self.emit(abi::compare_immediate(&count, "0"));
        self.emit(abi::branch_eq(&down_done));
        self.emit(abi::arithmetic_shift_right_immediate(value, value, 1));
        self.emit(abi::subtract_immediate(&count, &count, 1));
        self.emit(abi::branch(&down_loop));
        self.emit(abi::label(&down_done));
        self.emit(abi::label(&finish));
        Ok(())
    }

    /// Lower `log`/`log10` for a `Fixed` argument. Non-positive inputs fail with
    /// `ErrInvalidArgument`. Computes `ln(x) = e*ln2 + ln(m)` after normalising
    /// `x = m * 2^e` with `m in [1, 2)`, then scales for base-10.
    pub(crate) fn emit_fixed_log(
        &mut self,
        src: impl Into<Reg>,
        base10: bool,
    ) -> Result<VirtualRegister, String> {
        let src: Reg = src.into();
        let x_slot = self.allocate_stack_object("fixed_log_x", 8);
        self.emit(abi::store_u64(src, abi::stack_pointer(), x_slot));
        self.reset_temporary_registers();
        let x = self.allocate_register();
        self.emit(abi::load_u64(&x, abi::stack_pointer(), x_slot));
        let positive = self.label("fixed_log_positive");
        self.emit(abi::compare_immediate(&x, "0"));
        self.emit(abi::branch_gt(&positive));
        self.raise_error_bare("ErrInvalidArgument")?;
        self.emit(abi::label(&positive));
        // Normalise x into [1, 2): m_raw in [2^32, 2^33), tracking exponent e.
        let m = self.allocate_register();
        let e = self.allocate_register();
        let upper = self.allocate_register();
        let lower = self.allocate_register();
        self.emit(abi::move_register(&m, &x));
        self.emit(abi::move_immediate(&e, "Integer", "0"));
        self.emit(abi::move_immediate(
            &upper,
            "Integer",
            &(FIXED_ONE << 1).to_string(),
        ));
        self.emit(abi::move_immediate(
            &lower,
            "Integer",
            &FIXED_ONE.to_string(),
        ));
        let norm_high = self.label("fixed_log_norm_high");
        let norm_high_done = self.label("fixed_log_norm_high_done");
        self.emit(abi::label(&norm_high));
        self.emit(abi::compare_registers(&m, &upper));
        self.emit(abi::branch_lt(&norm_high_done));
        self.emit(abi::arithmetic_shift_right_immediate(&m, &m, 1));
        self.emit(abi::add_immediate(&e, &e, 1));
        self.emit(abi::branch(&norm_high));
        self.emit(abi::label(&norm_high_done));
        let norm_low = self.label("fixed_log_norm_low");
        let norm_low_done = self.label("fixed_log_norm_low_done");
        self.emit(abi::label(&norm_low));
        self.emit(abi::compare_registers(&m, &lower));
        self.emit(abi::branch_ge(&norm_low_done));
        self.emit(abi::shift_left_immediate(&m, &m, 1));
        self.emit(abi::subtract_immediate(&e, &e, 1));
        self.emit(abi::branch(&norm_low));
        self.emit(abi::label(&norm_low_done));
        // t = (m - 1)/(m + 1); ln(m) = 2*(t + t^3/3 + t^5/5 + ...).
        let numerator = self.allocate_register();
        let denominator = self.allocate_register();
        let one = self.allocate_register();
        self.emit(abi::move_immediate(
            &one,
            abi::IMMEDIATE_CLASS_FIXED,
            &FIXED_ONE.to_string(),
        ));
        self.emit(abi::subtract_registers(&numerator, &m, &one));
        self.emit(abi::add_registers(&denominator, &m, &one));
        // Spill e across the division helper (which resets the register file).
        let e_slot = self.allocate_stack_object("fixed_log_e", 8);
        self.emit(abi::store_u64(&e, abi::stack_pointer(), e_slot));
        let num_slot = self.allocate_stack_object("fixed_log_num", 8);
        let den_slot = self.allocate_stack_object("fixed_log_den", 8);
        self.emit(abi::store_u64(&numerator, abi::stack_pointer(), num_slot));
        self.emit(abi::store_u64(&denominator, abi::stack_pointer(), den_slot));
        self.reset_temporary_registers();
        let num_reg = self.allocate_register();
        let den_reg = self.allocate_register();
        self.emit(abi::load_u64(&num_reg, abi::stack_pointer(), num_slot));
        self.emit(abi::load_u64(&den_reg, abi::stack_pointer(), den_slot));
        let t = self.allocate_register();
        self.emit_fixed_divide(&t, &num_reg, &den_reg)?;
        // Series for ln(m) using t and t^2.
        let t2 = self.allocate_register();
        let saved = self.next_register;
        let ms0 = self.allocate_register();
        let ms1 = self.allocate_register();
        self.emit_fixed_mul_inplace(&t2, &t, &t, &ms0, &ms1);
        self.next_register = saved;
        let sum = self.allocate_register();
        let term = self.allocate_register();
        let k = self.allocate_register();
        let counter = self.allocate_register();
        let s0 = self.allocate_register();
        let s1 = self.allocate_register();
        let scratch = self.allocate_register();
        self.emit(abi::move_register(&sum, &t));
        self.emit(abi::move_register(&term, &t));
        self.emit(abi::move_immediate(&k, "Integer", "3"));
        self.emit(abi::move_immediate(&counter, "Integer", "14"));
        let series = self.label("fixed_log_series");
        let series_done = self.label("fixed_log_series_done");
        self.emit(abi::label(&series));
        self.emit(abi::compare_immediate(&counter, "0"));
        self.emit(abi::branch_eq(&series_done));
        self.emit_fixed_mul_inplace(&term, &term, &t2, &s0, &s1);
        self.emit(abi::signed_divide_registers(&scratch, &term, &k));
        self.emit(abi::add_registers(&sum, &sum, &scratch));
        self.emit(abi::add_immediate(&k, &k, 2));
        self.emit(abi::subtract_immediate(&counter, &counter, 1));
        self.emit(abi::branch(&series));
        self.emit(abi::label(&series_done));
        // ln(m) = 2 * sum.
        self.emit(abi::shift_left_immediate(&sum, &sum, 1));
        // ln(x) = e*ln2 + ln(m).
        let e_reg = self.allocate_register();
        let ln2 = self.allocate_register();
        self.emit(abi::load_u64(&e_reg, abi::stack_pointer(), e_slot));
        self.emit_const_i64(&ln2, fixed_ln2());
        let elog = self.allocate_register();
        self.emit(abi::multiply_registers(&elog, &e_reg, &ln2));
        self.emit(abi::add_registers(&sum, &sum, &elog));
        if base10 {
            let inv_ln10 = self.allocate_register();
            self.emit_const_i64(&inv_ln10, fixed_inv_ln10());
            return self.emit_fixed_mul(&sum, &inv_ln10);
        }
        Ok(sum)
    }

    /// Lower `pow(base, exponent)` for `Fixed` arguments. Whole-number exponents
    /// use exact repeated multiplication (any base sign, reciprocal for negative
    /// exponents); fractional exponents use `exp(exponent * ln(base))`, which
    /// requires `base > 0`. Overflow fails with `ErrOverflow`.
    /// `math::pow(Fixed, Fixed)` — the full-domain Fixed power: an exact repeated
    /// multiply for a whole exponent (with a reciprocal tail for negatives) and
    /// `exp(exponent·ln base)` for a fractional one.
    ///
    /// Its integer branch shares the bug-61/bug-74 ±1.0 closed form and the
    /// truncate-to-zero multiply loop with `builder_numeric::emit_fixed_pow` (the
    /// `^`-operator path), but the two are deliberately not merged (bug-332 E2):
    /// the domains differ (that path rejects negative/fractional exponents this one
    /// accepts) and the multiply loop itself emits different instructions — here it
    /// multiplies through a separate `product` register and moves it back, while
    /// `emit_fixed_pow` multiplies in place. No zero-diff extraction spans both.
    /// bug-931: finish `base^n` for a negative whole `n` whose forward power has
    /// just left the `Fixed` range. `power` holds `base^k` (in range, Q32.32) and
    /// `count` the multiplies still to do, the one that overflowed included; the
    /// result, `1 / base^|n|`, is at most two units, rounded to the nearest unit and
    /// stored at `result_slot`.
    ///
    /// The rest of the power is carried as an unsigned Q36.28 magnitude, which holds
    /// up to 2^36 to within 2^-28 — far finer than the half unit the rounding has to
    /// decide. Past 2^34 the reciprocal is under a quarter unit, so the result is
    /// 0 and the loop stops. The sign is `base`'s for an odd `|n|`.
    fn emit_fixed_pow_tiny_reciprocal(
        &mut self,
        power: impl Into<Reg>,
        base: impl Into<Reg>,
        count: impl Into<Reg>,
        n: impl Into<Reg>,
        result_slot: usize,
    ) -> Result<(), String> {
        let (power, base, count, n): (Reg, Reg, Reg, Reg) =
            (power.into(), base.into(), count.into(), n.into());
        let saved = self.next_register;
        let wide = self.allocate_register();
        let magnitude = self.allocate_register();
        let high = self.allocate_register();
        let low = self.allocate_register();
        let limit = self.allocate_register();
        let looping = self.label("fixed_pow_tiny_loop");
        let done = self.label("fixed_pow_tiny_done");
        let zero = self.label("fixed_pow_tiny_zero");
        let positive = self.label("fixed_pow_tiny_positive");
        let stored = self.label("fixed_pow_tiny_stored");
        // |power| as an unsigned word (`0 - i64::MIN` is 2^63, its magnitude), then
        // Q32.32 -> Q36.28.
        self.emit(abi::move_register(&wide, power));
        self.emit_abs_i64(&wide)?;
        self.emit(abi::shift_right_immediate(&wide, &wide, 4));
        self.emit(abi::move_register(&magnitude, base));
        self.emit_abs_i64(&magnitude)?;
        // A product of 2^62 or more in Q36.28 (a power of at least 2^34) is the
        // high word reaching 2^30.
        self.emit(abi::move_immediate(
            &limit,
            "Integer",
            &((1u64 << 30) - 1).to_string(),
        ));
        self.emit(abi::label(&looping));
        self.emit(abi::compare_immediate(count, "0"));
        self.emit(abi::branch_eq(&done));
        self.emit(abi::unsigned_multiply_high_registers(
            &high, &wide, &magnitude,
        ));
        self.emit(abi::multiply_registers(&low, &wide, &magnitude));
        self.emit(abi::compare_registers(&high, &limit));
        self.emit(abi::branch_hi(&zero));
        self.emit(abi::shift_right_immediate(&low, &low, 32));
        self.emit(abi::shift_left_immediate(&wide, &high, 32));
        self.emit(abi::or_registers(&wide, &wide, &low));
        self.emit(abi::subtract_immediate(count, count, 1));
        self.emit(abi::branch(&looping));
        // 1 / power in Q32.32 is 2^60 / wide; round to nearest by adding half the
        // divisor. `wide` is at least 2^59 here (the power passed 2^31), so the
        // quotient is at most 2 and the dividend stays below 2^62.
        self.emit(abi::label(&done));
        self.emit(abi::shift_right_immediate(&high, &wide, 1));
        self.emit(abi::move_immediate(
            &low,
            "Integer",
            &(1u64 << 60).to_string(),
        ));
        self.emit(abi::add_registers(&low, &low, &high));
        self.emit(abi::unsigned_divide_registers(&high, &low, &wide));
        // Negative exactly when `base` is negative and `|n|` is odd.
        self.emit(abi::compare_immediate(base, "0"));
        self.emit(abi::branch_ge(&positive));
        self.emit(abi::move_register(&low, n));
        self.emit(abi::move_immediate(&limit, "Integer", "1"));
        self.emit(abi::and_registers(&low, &low, &limit));
        self.emit(abi::compare_immediate(&low, "0"));
        self.emit(abi::branch_eq(&positive));
        self.emit_neg_i64(&high)?;
        self.emit(abi::label(&positive));
        self.emit(abi::store_u64(&high, abi::stack_pointer(), result_slot));
        self.emit(abi::branch(&stored));
        self.emit(abi::label(&zero));
        self.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), result_slot));
        self.emit(abi::label(&stored));
        self.next_register = saved;
        Ok(())
    }

    pub(crate) fn emit_fixed_pow_general(
        &mut self,
        base: impl Into<Reg>,
        exponent: impl Into<Reg>,
    ) -> Result<VirtualRegister, String> {
        let (base, exponent): (Reg, Reg) = (base.into(), exponent.into());
        let base_slot = self.allocate_stack_object("fixed_pow_base", 8);
        let exp_slot = self.allocate_stack_object("fixed_pow_exp", 8);
        self.emit(abi::store_u64(base, abi::stack_pointer(), base_slot));
        self.emit(abi::store_u64(exponent, abi::stack_pointer(), exp_slot));
        self.reset_temporary_registers();
        let exp_reg = self.allocate_register();
        self.emit(abi::load_u64(&exp_reg, abi::stack_pointer(), exp_slot));
        // Whole-number exponent? (no fractional Q32.32 bits)
        let frac = self.allocate_register();
        let mask = self.allocate_register();
        self.emit(abi::move_immediate(
            &mask,
            "Integer",
            &FIXED_FRACTION_MASK.to_string(),
        ));
        self.emit(abi::and_registers(&frac, &exp_reg, &mask));
        let fractional = self.label("fixed_pow_fractional");
        self.emit(abi::compare_immediate(&frac, "0"));
        self.emit(abi::branch_ne(&fractional));

        // Integer exponent: exact repeated multiplication.
        let integer_result_slot = self.allocate_stack_object("fixed_pow_int_result", 8);
        let n = self.allocate_register();
        self.emit(abi::arithmetic_shift_right_immediate(&n, &exp_reg, 32));
        let count = self.allocate_register();
        self.emit(abi::move_register(&count, &n));
        self.emit_abs_i64(&count)?;
        let base_reg = self.allocate_register();
        self.emit(abi::load_u64(&base_reg, abi::stack_pointer(), base_slot));
        let result = self.allocate_register();
        let product = self.allocate_register();
        self.emit(abi::move_immediate(
            &result,
            abi::IMMEDIATE_CLASS_FIXED,
            &FIXED_ONE.to_string(),
        ));
        let mul_loop = self.label("fixed_pow_int_loop");
        let mul_done = self.label("fixed_pow_int_done");
        // Bounded-base fast path (bug-61): |base| == 1.0 has bounded powers, so the
        // loop's only exit (the multiply overflow trap) never fires and it would
        // iterate the full |exponent|. Resolve ±1.0 in closed form, then fall
        // through to the shared negative-exponent reciprocal handling below.
        // Compare against ±1.0 through registers, not `compare_immediate`: the raw
        // Fixed constants are `±2^32`, which exceed the x86 CMP imm32 field and
        // fail to encode (bug-74). `result` already holds `FIXED_ONE`.
        let fixed_neg_one = -(FIXED_ONE as i64) as u64;
        let neg_one_reg = self.allocate_register();
        self.emit(abi::move_immediate(
            &neg_one_reg,
            abi::IMMEDIATE_CLASS_FIXED,
            &fixed_neg_one.to_string(),
        ));
        self.emit(abi::compare_registers(&base_reg, &result));
        self.emit(abi::branch_eq(&mul_done)); // 1.0^n == 1.0 (result already 1.0).
        self.emit(abi::compare_registers(&base_reg, &neg_one_reg));
        self.emit(abi::branch_ne(&mul_loop)); // |base| != 1.0: run the loop.
                                              // base == -1.0: 1.0 for an even |exponent|, -1.0 for an odd one.
        let parity = self.allocate_register();
        let one_bit = self.allocate_register();
        self.emit(abi::move_immediate(&one_bit, "Integer", "1"));
        self.emit(abi::and_registers(&parity, &count, &one_bit));
        self.emit(abi::compare_immediate(&parity, "0"));
        self.emit(abi::branch_eq(&mul_done)); // even: result stays 1.0.
        self.emit_neg_i64(&result)?; // odd: result = -1.0.
        self.emit(abi::branch(&mul_done));
        self.emit(abi::label(&mul_loop));
        self.emit(abi::compare_immediate(&count, "0"));
        self.emit(abi::branch_eq(&mul_done));
        // A product that truncates to 0 (any |base| < 1.0, or base == 0.0) stays 0
        // for every remaining multiply, so stop now rather than iterate the whole
        // (possibly enormous) |exponent| (bug-61). This never changes a result.
        self.emit(abi::compare_immediate(&result, "0"));
        self.emit(abi::branch_eq(&mul_done));
        // bug-931: a forward product that leaves the `Fixed` range is an overflow
        // only for a non-negative exponent. For a negative one the result is its
        // reciprocal, which is in range (at most two units), so it is finished in
        // a wider format below instead of raising.
        let forward_out_of_range = self.label("fixed_pow_int_forward_out_of_range");
        let reload = self.label("fixed_pow_int_reload");
        self.emit_fixed_multiply_or_branch(&product, &result, &base_reg, &forward_out_of_range);
        self.emit(abi::move_register(&result, &product));
        self.emit(abi::subtract_immediate(&count, &count, 1));
        self.emit(abi::branch(&mul_loop));
        self.emit(abi::label(&forward_out_of_range));
        let negative_exponent = self.label("fixed_pow_int_tiny");
        self.emit(abi::compare_immediate(&n, "0"));
        self.emit(abi::branch_lt(&negative_exponent));
        self.raise_error_bare("ErrOverflow")?;
        self.emit(abi::label(&negative_exponent));
        self.emit_fixed_pow_tiny_reciprocal(&result, &base_reg, &count, &n, integer_result_slot)?;
        self.emit(abi::branch(&reload));
        self.emit(abi::label(&mul_done));
        // Negative exponent: reciprocal 1 / result.
        let nonneg = self.label("fixed_pow_int_nonneg");
        self.emit(abi::compare_immediate(&n, "0"));
        self.emit(abi::branch_ge(&nonneg));
        self.emit(abi::store_u64(
            &result,
            abi::stack_pointer(),
            integer_result_slot,
        ));
        self.reset_temporary_registers();
        let denom = self.allocate_register();
        let one = self.allocate_register();
        let recip = self.allocate_register();
        self.emit(abi::load_u64(
            &denom,
            abi::stack_pointer(),
            integer_result_slot,
        ));
        // A forward product that underflowed to 0 (|base| < 1 raised to a large
        // magnitude) makes the reciprocal 1.0/0.0. That is a genuine Fixed-range
        // overflow, so trap ErrOverflow rather than letting emit_fixed_divide
        // report ErrInvalidArgument for the zero divisor (bug-137).
        let recip_ok = self.label("fixed_pow_recip_ok");
        self.emit(abi::compare_immediate(&denom, "0"));
        self.emit(abi::branch_ne(&recip_ok));
        self.raise_error_bare("ErrOverflow")?;
        self.emit(abi::label(&recip_ok));
        self.emit(abi::move_immediate(
            &one,
            abi::IMMEDIATE_CLASS_FIXED,
            &FIXED_ONE.to_string(),
        ));
        self.emit_fixed_divide(&recip, &one, &denom)?;
        self.emit(abi::store_u64(
            &recip,
            abi::stack_pointer(),
            integer_result_slot,
        ));
        self.emit(abi::branch(&reload));
        self.emit(abi::label(&nonneg));
        self.emit(abi::store_u64(
            &result,
            abi::stack_pointer(),
            integer_result_slot,
        ));
        self.emit(abi::label(&reload));
        self.reset_temporary_registers();
        let int_result = self.allocate_register();
        self.emit(abi::load_u64(
            &int_result,
            abi::stack_pointer(),
            integer_result_slot,
        ));
        let finish = self.label("fixed_pow_finish");
        // Stash the integer result and branch past the fractional path.
        let result_slot = self.allocate_stack_object("fixed_pow_result", 8);
        self.emit(abi::store_u64(
            &int_result,
            abi::stack_pointer(),
            result_slot,
        ));
        self.emit(abi::branch(&finish));

        // Fractional exponent: exp(exponent * ln(base)), requires base > 0.
        self.emit(abi::label(&fractional));
        self.reset_temporary_registers();
        let base_reg = self.allocate_register();
        self.emit(abi::load_u64(&base_reg, abi::stack_pointer(), base_slot));
        // ln(base) (also enforces base > 0 via ErrInvalidArgument).
        let ln_base = self.emit_fixed_log(&base_reg, false)?;
        let ln_slot = self.allocate_stack_object("fixed_pow_ln", 8);
        self.emit(abi::store_u64(&ln_base, abi::stack_pointer(), ln_slot));
        self.reset_temporary_registers();
        let exp_reg = self.allocate_register();
        let ln_reg = self.allocate_register();
        self.emit(abi::load_u64(&exp_reg, abi::stack_pointer(), exp_slot));
        self.emit(abi::load_u64(&ln_reg, abi::stack_pointer(), ln_slot));
        // bug-659: saturate, never wrap. `|ln(base)|` reaches ~22 across the Fixed
        // domain, so an exponent past ~1e8 drives this product out of Q32.32 range;
        // a wrapped product flips sign, and `emit_fixed_exp` reads its outcome
        // (ErrOverflow vs 0.00) off exactly that sign — so the wrap did not blur a
        // magnitude, it inverted the answer. A saturated `i64::MAX`/`i64::MIN` is
        // past exp's argument gate in the correct direction, which is the true
        // result: `|exponent * ln(base)| > 2^31` cannot land inside Fixed range.
        let product = self.emit_fixed_mul_saturating(&exp_reg, &ln_reg)?;
        let frac_result = self.emit_fixed_exp(&product)?;
        self.emit(abi::store_u64(
            &frac_result,
            abi::stack_pointer(),
            result_slot,
        ));

        self.emit(abi::label(&finish));
        self.reset_temporary_registers();
        let final_result = self.allocate_register();
        self.emit(abi::load_u64(
            &final_result,
            abi::stack_pointer(),
            result_slot,
        ));
        Ok(final_result)
    }
}
