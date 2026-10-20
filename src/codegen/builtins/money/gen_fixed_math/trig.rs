//! plan-176-S: fixed-point trigonometry — atan/atan2, angle reduction, sin/cos, tan and asin. The
//! constants and the multiply/sqrt kernels live in `builtins/money/gen_fixed_math/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// `atan(num/den)` for unsigned 64-bit `num` and `den`, as an unsigned Q3.61
    /// angle in `[0, pi/2]` (bug-615). This is the one kernel behind every
    /// `Fixed` inverse-trig member: `atan`, `atan2`, `asin` and `acos` differ
    /// only in the ratio they hand it and the quadrant they add afterwards.
    ///
    /// Taking a *ratio* rather than a `Fixed` is what buys the accuracy. The
    /// quotient is never rounded to Q32.32 first, so a huge or tiny argument
    /// keeps its full relative precision, and the two range folds are exact
    /// integer rewrites of the ratio:
    ///
    /// * `num > den`: `atan(t) = pi/2 - atan(1/t)`, i.e. swap the two words.
    /// * `2*num > den`: `atan(t) = pi/4 - atan((1-t)/(1+t))`, i.e. replace
    ///   `(num, den)` with `(den - num, den + num)`.
    ///
    /// Both leave `u = num/den <= 1/2`, which the Taylor series
    /// `atan u = u * sum (-u^2)^n/(2n+1)` reaches in [`FIXED_ATAN_TERMS`] + 1
    /// Horner levels: the first omitted term is `u^55/55 < 2^-60`. `u` itself is
    /// an exact unsigned Q0.63 quotient from a 63-step long division, and the
    /// series runs in Q1.63, so the angle carries about 60 bits before it is
    /// rounded once into Q32.32 by the caller. The residue is under `2^-55`
    /// radians, against the several Q32.32 units the 31-step Q32.32 CORDIC
    /// vectoring loop this replaces left behind.
    ///
    /// `den` is first held below `2^62` so `den + num` cannot wrap; that moves
    /// the ratio by at most `8/den <= 2^-59`. `num = 0` gives `0` and `den = 0`
    /// gives `pi/2`, so a zero numerator and the `|x| = 1` pole of `asin` both
    /// fall out without a divide.
    fn emit_fixed_atan_ratio(
        &mut self,
        num_src: impl Into<Reg>,
        den_src: impl Into<Reg>,
    ) -> VirtualRegister {
        let (num_src, den_src): (Reg, Reg) = (num_src.into(), den_src.into());
        let num = self.allocate_register();
        let den = self.allocate_register();
        let angle = self.allocate_register();
        let scratch = self.allocate_register();
        let swapped = self.allocate_register();
        let reduced = self.allocate_register();
        self.emit(abi::move_register(&num, num_src));
        self.emit(abi::move_register(&den, den_src));
        let general = self.label("fixed_atan_general");
        let finish = self.label("fixed_atan_finish");
        // atan(0/den) = 0; this also covers num = den = 0.
        self.emit(abi::compare_immediate(&num, "0"));
        self.emit(abi::branch_ne(&general));
        self.emit(abi::move_immediate(&angle, "Integer", "0"));
        self.emit(abi::branch(&finish));
        self.emit(abi::label(&general));
        // atan(num/0) = pi/2 for num > 0.
        let den_nonzero = self.label("fixed_atan_den_nonzero");
        self.emit(abi::compare_immediate(&den, "0"));
        self.emit(abi::branch_ne(&den_nonzero));
        self.emit_const_u64(&angle, PI_OVER_2_Q61);
        self.emit(abi::branch(&finish));
        self.emit(abi::label(&den_nonzero));
        // |t| > 1: swap, and take pi/2 minus the angle at the end.
        self.emit(abi::move_immediate(&swapped, "Integer", "0"));
        let no_swap = self.label("fixed_atan_no_swap");
        self.emit(abi::compare_registers(&num, &den));
        self.emit(abi::branch_ls(&no_swap));
        self.emit(abi::move_register(&scratch, &num));
        self.emit(abi::move_register(&num, &den));
        self.emit(abi::move_register(&den, &scratch));
        self.emit(abi::move_immediate(&swapped, "Integer", "1"));
        self.emit(abi::label(&no_swap));
        // Keep the larger word below 2^62 so den + num below cannot wrap.
        let no_scale = self.label("fixed_atan_no_scale");
        self.emit_const_u64(&scratch, 1u64 << 62);
        self.emit(abi::compare_registers(&den, &scratch));
        self.emit(abi::branch_lo(&no_scale));
        self.emit(abi::shift_right_immediate(&num, &num, 2));
        self.emit(abi::shift_right_immediate(&den, &den, 2));
        self.emit(abi::label(&no_scale));
        // t > 1/2: replace the ratio by (1-t)/(1+t) and add pi/4 at the end.
        self.emit(abi::move_immediate(&reduced, "Integer", "0"));
        let no_reduce = self.label("fixed_atan_no_reduce");
        self.emit(abi::shift_left_immediate(&scratch, &num, 1));
        self.emit(abi::compare_registers(&scratch, &den));
        self.emit(abi::branch_ls(&no_reduce));
        self.emit(abi::add_registers(&scratch, &den, &num));
        self.emit(abi::subtract_registers(&num, &den, &num));
        self.emit(abi::move_register(&den, &scratch));
        self.emit(abi::move_immediate(&reduced, "Integer", "1"));
        self.emit(abi::label(&no_reduce));
        // u = num/den in unsigned Q0.63. num < den < 2^63, so the remainder
        // stays below den and doubling it never wraps.
        let u = self.allocate_register();
        let remainder = self.allocate_register();
        let counter = self.allocate_register();
        self.emit(abi::move_register(&remainder, &num));
        self.emit(abi::move_immediate(&u, "Integer", "0"));
        self.emit(abi::move_immediate(&counter, "Integer", "63"));
        let divide = self.label("fixed_atan_divide");
        let divide_skip = self.label("fixed_atan_divide_skip");
        let divided = self.label("fixed_atan_divided");
        self.emit(abi::label(&divide));
        self.emit(abi::compare_immediate(&counter, "0"));
        self.emit(abi::branch_eq(&divided));
        self.emit(abi::shift_left_immediate(&remainder, &remainder, 1));
        self.emit(abi::shift_left_immediate(&u, &u, 1));
        self.emit(abi::compare_registers(&remainder, &den));
        self.emit(abi::branch_lo(&divide_skip));
        self.emit(abi::subtract_registers(&remainder, &remainder, &den));
        self.emit(abi::add_immediate(&u, &u, 1));
        self.emit(abi::label(&divide_skip));
        self.emit(abi::subtract_immediate(&counter, &counter, 1));
        self.emit(abi::branch(&divide));
        self.emit(abi::label(&divided));
        // P = 1/(2n+1) - u^2*P for n = FIXED_ATAN_TERMS..0, in unsigned Q1.63
        // against u^2 in unsigned Q0.64. The registers of the long division are
        // free now and carry the series divisor and its level counter.
        let u_squared = self.allocate_register();
        let accumulator = self.allocate_register();
        let one = self.allocate_register();
        self.emit(abi::unsigned_multiply_high_registers(&u_squared, &u, &u));
        self.emit(abi::shift_left_immediate(&u_squared, &u_squared, 2));
        self.emit_const_u64(&one, Q63_ONE);
        self.emit(abi::move_immediate(&accumulator, "Integer", "0"));
        self.emit(abi::move_immediate(
            &counter,
            "Integer",
            &FIXED_ATAN_TERMS.to_string(),
        ));
        let series = self.label("fixed_atan_series");
        let series_done = self.label("fixed_atan_series_done");
        self.emit(abi::label(&series));
        self.emit(abi::add_registers(&remainder, &counter, &counter));
        self.emit(abi::add_immediate(&remainder, &remainder, 1));
        self.emit(abi::unsigned_multiply_high_registers(
            &scratch,
            &u_squared,
            &accumulator,
        ));
        self.emit(abi::unsigned_divide_registers(
            &accumulator,
            &one,
            &remainder,
        ));
        self.emit(abi::subtract_registers(
            &accumulator,
            &accumulator,
            &scratch,
        ));
        self.emit(abi::compare_immediate(&counter, "0"));
        self.emit(abi::branch_eq(&series_done));
        self.emit(abi::subtract_immediate(&counter, &counter, 1));
        self.emit(abi::branch(&series));
        self.emit(abi::label(&series_done));
        // atan u = u*P in Q1.63 (u <= 1/2, so u << 1 is still the Q0.64 form of
        // u), rounded to Q3.61, then the two folds undone.
        self.emit(abi::shift_left_immediate(&u, &u, 1));
        self.emit(abi::unsigned_multiply_high_registers(
            &angle,
            &u,
            &accumulator,
        ));
        self.emit(abi::add_immediate(&angle, &angle, 2));
        self.emit(abi::shift_right_immediate(&angle, &angle, 2));
        let not_reduced = self.label("fixed_atan_not_reduced");
        self.emit(abi::compare_immediate(&reduced, "0"));
        self.emit(abi::branch_eq(&not_reduced));
        self.emit_const_u64(&scratch, PI_OVER_4_Q61);
        self.emit(abi::subtract_registers(&angle, &scratch, &angle));
        self.emit(abi::label(&not_reduced));
        let not_swapped = self.label("fixed_atan_not_swapped");
        self.emit(abi::compare_immediate(&swapped, "0"));
        self.emit(abi::branch_eq(&not_swapped));
        self.emit_const_u64(&scratch, PI_OVER_2_Q61);
        self.emit(abi::subtract_registers(&angle, &scratch, &angle));
        self.emit(abi::label(&not_swapped));
        self.emit(abi::label(&finish));
        angle
    }

    /// Round the unsigned Q3.61 angle `angle` (at most `pi*2^61`) to a raw
    /// Q32.32 `Fixed` and negate it when the 0/1 register `negative` is 1,
    /// writing `dst`. The one rounding step of the whole inverse-trig family.
    fn emit_q61_to_signed_fixed(
        &mut self,
        dst: impl Into<Reg>,
        angle: impl Into<Reg>,
        negative: impl Into<Reg>,
    ) {
        let (dst, angle, negative): (Reg, Reg, Reg) = (dst.into(), angle.into(), negative.into());
        let mask = self.allocate_register();
        self.emit_const_u64(&mask, 1u64 << 28);
        self.emit(abi::add_registers(dst, angle, &mask));
        self.emit(abi::shift_right_immediate(dst, dst, 29));
        self.emit_conditional_negate(dst, negative, &mask);
    }

    /// Deterministic Q32.32 `atan2(y, x)` returning the angle in radians, within
    /// one Q32.32 unit of the true value at every pair of arguments (bug-615).
    ///
    /// The axes are exact constants; elsewhere the magnitudes go to
    /// [`Self::emit_fixed_atan_ratio`] as a ratio and the quadrant is added in
    /// Q3.61 before the single rounding step: `x > 0` keeps the angle, `x < 0`
    /// takes `pi` minus it, and the sign of `y` is applied last. `atan2(0, x<0)`
    /// is `pi` rather than `-pi`, matching the sign rule for `y = 0`.
    pub(crate) fn emit_fixed_atan2(
        &mut self,
        y_src: impl Into<Reg>,
        x_src: impl Into<Reg>,
    ) -> Result<VirtualRegister, String> {
        let (y_src, x_src): (Reg, Reg) = (y_src.into(), x_src.into());
        let y_slot = self.allocate_stack_object("fixed_atan2_y", 8);
        let x_slot = self.allocate_stack_object("fixed_atan2_x", 8);
        self.emit(abi::store_u64(y_src, abi::stack_pointer(), y_slot));
        self.emit(abi::store_u64(x_src, abi::stack_pointer(), x_slot));
        self.reset_temporary_registers();
        let vy = self.allocate_register();
        let vx = self.allocate_register();
        let result = self.allocate_register();
        let scratch = self.allocate_register();
        self.emit(abi::load_u64(&vy, abi::stack_pointer(), y_slot));
        self.emit(abi::load_u64(&vx, abi::stack_pointer(), x_slot));
        let general = self.label("fixed_atan2_general");
        let finish = self.label("fixed_atan2_finish");
        // y == 0: atan2(0, x >= 0) = 0, atan2(0, x < 0) = pi.
        let y_nonzero = self.label("fixed_atan2_y_nonzero");
        let y_zero_done = self.label("fixed_atan2_y_zero_done");
        self.emit(abi::compare_immediate(&vy, "0"));
        self.emit(abi::branch_ne(&y_nonzero));
        self.emit(abi::move_immediate(&result, "Integer", "0"));
        self.emit(abi::compare_immediate(&vx, "0"));
        self.emit(abi::branch_ge(&y_zero_done));
        self.emit_const_i64(&result, PI_FIXED);
        self.emit(abi::label(&y_zero_done));
        self.emit(abi::branch(&finish));
        self.emit(abi::label(&y_nonzero));
        // x == 0: atan2(y, 0) = +/- pi/2.
        let x_zero_done = self.label("fixed_atan2_x_zero_done");
        self.emit(abi::compare_immediate(&vx, "0"));
        self.emit(abi::branch_ne(&general));
        self.emit_const_i64(&result, PI_OVER_2_FIXED);
        self.emit(abi::compare_immediate(&vy, "0"));
        self.emit(abi::branch_gt(&x_zero_done));
        self.emit_const_i64(&result, -PI_OVER_2_FIXED);
        self.emit(abi::label(&x_zero_done));
        self.emit(abi::branch(&finish));
        self.emit(abi::label(&general));
        // |v| without overflow: v ^ (v>>63) - (v>>63) maps raw i64::MIN to 2^63
        // rather than back to itself, which is what broke the old reflection
        // through the origin (bug-128).
        let negative = self.allocate_register();
        self.emit(abi::arithmetic_shift_right_immediate(&scratch, &vy, 63));
        self.emit(abi::exclusive_or_registers(&negative, &vy, &scratch));
        self.emit(abi::subtract_registers(&negative, &negative, &scratch));
        self.emit(abi::arithmetic_shift_right_immediate(&scratch, &vx, 63));
        self.emit(abi::exclusive_or_registers(&result, &vx, &scratch));
        self.emit(abi::subtract_registers(&result, &result, &scratch));
        let angle = self.emit_fixed_atan_ratio(&negative, &result);
        // x < 0 reflects the first-quadrant angle through pi/2 in Q3.61.
        let x_positive = self.label("fixed_atan2_x_positive");
        self.emit(abi::compare_immediate(&vx, "0"));
        self.emit(abi::branch_gt(&x_positive));
        self.emit_const_u64(&scratch, PI_Q61);
        self.emit(abi::subtract_registers(&angle, &scratch, &angle));
        self.emit(abi::label(&x_positive));
        self.emit(abi::shift_right_immediate(&negative, &vy, 63));
        self.emit_q61_to_signed_fixed(&result, &angle, &negative);
        self.emit(abi::label(&finish));
        Ok(result)
    }

    /// Reduce the `Fixed` angle in `src` for the trig kernels (bug-615):
    /// `|x| = k·(pi/2) + r` with `k = round(|x|·2/pi)` and `|r| <= pi/4 + 2^-31`.
    ///
    /// `r` is formed exactly to about `2^-127`: `|x|·2^127` minus
    /// `k·(pi/2)·2^127`, where pi/2 is the 160-bit constant [`PI_OVER_2_Q159`].
    /// The whole subtraction runs modulo `2^128` — the wrapped high bits of
    /// `|x|` and of `k·pi/2` cancel, because the true difference fits — so only
    /// the low 33 bits of `|x|` and the low 160 bits of `k·pi/2` take part. `k`
    /// is below `2^31`, so the product's truncation error is under `2^-128`.
    /// Rounding pi/2 to Q32.32 instead (the old kernel) lost `k·0.26` units and
    /// collapsed `r` to zero at `math::pi2Fixed`.
    fn emit_fixed_trig_reduce(&mut self, src: impl Into<Reg>) -> ReducedAngle {
        let src: Reg = src.into();
        let slot = self.allocate_stack_object("fixed_trig_input", 8);
        self.emit(abi::store_u64(src, abi::stack_pointer(), slot));
        self.reset_temporary_registers();
        let magnitude = self.allocate_register();
        self.emit(abi::load_u64(&magnitude, abi::stack_pointer(), slot));
        // |x| as an unsigned word (raw i64::MIN becomes 2^63), and its sign bit.
        let negative_x = self.allocate_register();
        let scratch = self.allocate_register();
        self.emit(abi::shift_right_immediate(&negative_x, &magnitude, 63));
        self.emit(abi::arithmetic_shift_right_immediate(
            &scratch, &magnitude, 63,
        ));
        self.emit(abi::exclusive_or_registers(
            &magnitude, &magnitude, &scratch,
        ));
        self.emit(abi::subtract_registers(&magnitude, &magnitude, &scratch));
        // k = round(|x|·2/pi): umulh by 2/pi in Q0.64 is |x|·2/pi in Q32.32,
        // within one unit, so a wrong k only ever leaves |r| <= pi/4 + 2^-31.
        let k = self.allocate_register();
        self.emit_const_u64(&scratch, TWO_OVER_PI_Q64);
        self.emit(abi::unsigned_multiply_high_registers(
            &k, &magnitude, &scratch,
        ));
        self.emit_const_u64(&scratch, FIXED_HALF);
        self.emit(abi::add_registers(&k, &k, &scratch));
        self.emit(abi::shift_right_immediate(&k, &k, 32));
        // k·(pi/2)·2^159 modulo 2^160 in words (w2, w1, w0).
        let w0 = self.allocate_register();
        let w1 = self.allocate_register();
        let w2 = self.allocate_register();
        let product = self.allocate_register();
        self.emit_const_u64(&scratch, PI_OVER_2_Q159[2]);
        self.emit(abi::multiply_registers(&w0, &k, &scratch));
        self.emit(abi::unsigned_multiply_high_registers(&w1, &k, &scratch));
        self.emit_const_u64(&scratch, PI_OVER_2_Q159[1]);
        self.emit(abi::multiply_registers(&product, &k, &scratch));
        self.emit(abi::unsigned_multiply_high_registers(&w2, &k, &scratch));
        self.emit(abi::add_registers(&w1, &w1, &product));
        let no_carry = self.label("fixed_trig_reduce_no_carry");
        self.emit(abi::compare_registers(&product, &w1));
        self.emit(abi::branch_ls(&no_carry));
        // umulh(k, P1) < k < 2^31, so this increment cannot wrap.
        self.emit(abi::add_immediate(&w2, &w2, 1));
        self.emit(abi::label(&no_carry));
        self.emit_const_u64(&scratch, PI_OVER_2_Q159[0]);
        self.emit(abi::multiply_registers(&product, &k, &scratch));
        self.emit(abi::add_registers(&w2, &w2, &product));
        // Shift right 32: k·(pi/2)·2^127 modulo 2^128 in (high, low).
        let r_high = self.allocate_register();
        let r_low = self.allocate_register();
        self.emit(abi::shift_right_immediate(&r_low, &w0, 32));
        self.emit(abi::shift_left_immediate(&scratch, &w1, 32));
        self.emit(abi::or_registers(&r_low, &r_low, &scratch));
        self.emit(abi::shift_right_immediate(&r_high, &w1, 32));
        self.emit(abi::shift_left_immediate(&scratch, &w2, 32));
        self.emit(abi::or_registers(&r_high, &r_high, &scratch));
        // r·2^127 = (|x|·2^95 mod 2^128) − that; |x|·2^95 has a zero low word.
        self.emit(abi::shift_left_immediate(&scratch, &magnitude, 31));
        self.emit(abi::subtract_registers(&r_high, &scratch, &r_high));
        let no_borrow = self.label("fixed_trig_reduce_no_borrow");
        self.emit(abi::compare_immediate(&r_low, "0"));
        self.emit(abi::branch_eq(&no_borrow));
        self.emit(abi::subtract_immediate(&r_high, &r_high, 1));
        self.emit(abi::subtract_registers(&r_low, abi::ZERO, &r_low));
        self.emit(abi::label(&no_borrow));
        // |r| < 0.79 < 1, so the 128-bit result is a signed value: take its sign
        // and magnitude.
        let negative_r = self.allocate_register();
        self.emit(abi::shift_right_immediate(&negative_r, &r_high, 63));
        let positive_r = self.label("fixed_trig_reduce_positive_r");
        self.emit(abi::compare_immediate(&negative_r, "0"));
        self.emit(abi::branch_eq(&positive_r));
        // −(high, low) = (~high + (low == 0), −low).
        self.emit(abi::bitwise_not(&r_high, &r_high));
        let low_nonzero = self.label("fixed_trig_reduce_low_nonzero");
        self.emit(abi::compare_immediate(&r_low, "0"));
        self.emit(abi::branch_ne(&low_nonzero));
        self.emit(abi::add_immediate(&r_high, &r_high, 1));
        self.emit(abi::label(&low_nonzero));
        self.emit(abi::subtract_registers(&r_low, abi::ZERO, &r_low));
        self.emit(abi::label(&positive_r));
        let quadrant = self.allocate_register();
        self.emit(abi::move_immediate(&scratch, "Integer", "3"));
        self.emit(abi::and_registers(&quadrant, &k, &scratch));
        ReducedAngle {
            negative_x,
            quadrant,
            negative_r,
            r_high,
            r_low,
        }
    }

    /// `(sin |r|, cos |r|)` in unsigned Q1.63 for the reduced angle
    /// `|r|·2^127 = (r_high, r_low)` (bug-615).
    ///
    /// Horner-form Taylor series in Q1.63 on `|r|` truncated to Q0.64:
    /// `S = 1 − r²/(2n(2n+1))·S` and `C = 1 − r²/((2n−1)2n)·C` for
    /// `n = FIXED_TRIG_TERMS..1`, then `sin = r·S`. With `|r| <= pi/4 + 2^-31`
    /// the first omitted terms (`r^20/21!`, `r^20/20!`) are below `2^-67`, and
    /// the truncating `umulh`/`udiv` steps keep each result within a few units of
    /// `2^-63` — far inside one Q32.32 unit.
    fn emit_fixed_sin_cos_magnitudes(
        &mut self,
        r_high: impl Into<Reg>,
        r_low: impl Into<Reg>,
    ) -> (VirtualRegister, VirtualRegister) {
        let (r_high, r_low): (Reg, Reg) = (r_high.into(), r_low.into());
        let r = self.allocate_register();
        let r_squared = self.allocate_register();
        let scratch = self.allocate_register();
        // |r| in Q0.64 = (|r|·2^127) >> 63.
        self.emit(abi::shift_left_immediate(&r, r_high, 1));
        self.emit(abi::shift_right_immediate(&scratch, r_low, 63));
        self.emit(abi::or_registers(&r, &r, &scratch));
        self.emit(abi::unsigned_multiply_high_registers(&r_squared, &r, &r));
        let one = self.allocate_register();
        let sine = self.allocate_register();
        let cosine = self.allocate_register();
        let n = self.allocate_register();
        let two_n = self.allocate_register();
        let divisor = self.allocate_register();
        self.emit_const_u64(&one, Q63_ONE);
        self.emit(abi::move_register(&sine, &one));
        self.emit(abi::move_register(&cosine, &one));
        self.emit(abi::move_immediate(
            &n,
            "Integer",
            &FIXED_TRIG_TERMS.to_string(),
        ));
        let series = self.label("fixed_trig_series");
        let series_done = self.label("fixed_trig_series_done");
        self.emit(abi::label(&series));
        self.emit(abi::compare_immediate(&n, "0"));
        self.emit(abi::branch_eq(&series_done));
        self.emit(abi::add_registers(&two_n, &n, &n));
        // S = 1 − r²·S / (2n·(2n+1)).
        self.emit(abi::add_immediate(&divisor, &two_n, 1));
        self.emit(abi::multiply_registers(&divisor, &divisor, &two_n));
        self.emit(abi::unsigned_multiply_high_registers(
            &scratch, &r_squared, &sine,
        ));
        self.emit(abi::unsigned_divide_registers(&scratch, &scratch, &divisor));
        self.emit(abi::subtract_registers(&sine, &one, &scratch));
        // C = 1 − r²·C / ((2n−1)·2n).
        self.emit(abi::subtract_immediate(&divisor, &two_n, 1));
        self.emit(abi::multiply_registers(&divisor, &divisor, &two_n));
        self.emit(abi::unsigned_multiply_high_registers(
            &scratch, &r_squared, &cosine,
        ));
        self.emit(abi::unsigned_divide_registers(&scratch, &scratch, &divisor));
        self.emit(abi::subtract_registers(&cosine, &one, &scratch));
        self.emit(abi::subtract_immediate(&n, &n, 1));
        self.emit(abi::branch(&series));
        self.emit(abi::label(&series_done));
        // sin |r| = |r| · S: Q0.64 × Q1.63 → Q1.63.
        self.emit(abi::unsigned_multiply_high_registers(&sine, &r, &sine));
        (sine, cosine)
    }

    /// Round the unsigned Q1.63 magnitude `m` (at most `2^63`) to Q32.32 and
    /// negate it when the 0/1 register `negative` is 1, writing `dst`.
    fn emit_q63_to_signed_fixed(
        &mut self,
        dst: impl Into<Reg>,
        m: impl Into<Reg>,
        negative: impl Into<Reg>,
    ) {
        let (dst, m, negative): (Reg, Reg, Reg) = (dst.into(), m.into(), negative.into());
        let mask = self.allocate_register();
        self.emit_const_u64(&mask, 1u64 << 30);
        self.emit(abi::add_registers(dst, m, &mask));
        self.emit(abi::shift_right_immediate(dst, dst, 31));
        self.emit_conditional_negate(dst, negative, &mask);
    }

    /// `dst = negative ? −dst : dst` for a 0/1 `negative`, branch-free:
    /// `(dst ^ −negative) − (−negative)`. `mask` is scratch.
    fn emit_conditional_negate(
        &mut self,
        dst: impl Into<Reg>,
        negative: impl Into<Reg>,
        mask: impl Into<Reg>,
    ) {
        let (dst, negative, mask): (Reg, Reg, Reg) = (dst.into(), negative.into(), mask.into());
        self.emit(abi::subtract_registers(mask, abi::ZERO, negative));
        self.emit(abi::exclusive_or_registers(dst, dst, mask));
        self.emit(abi::subtract_registers(dst, dst, mask));
    }

    /// Lower `sin`/`cos` for a `Fixed` argument: the nearest `Fixed` to the true
    /// value, within one Q32.32 unit at every argument (bug-615).
    ///
    /// With `|x| = k·(pi/2) + r` from [`Self::emit_fixed_trig_reduce`], `sin |x|`
    /// is `sin r, cos r, −sin r, −cos r` and `cos |x|` is
    /// `cos r, −sin r, −cos r, sin r` for quadrants `k mod 4 = 0..3`. `sin r`
    /// carries the sign of `r`; `sin x` also carries the sign of `x`.
    pub(crate) fn emit_fixed_sin_cos(
        &mut self,
        src: impl Into<Reg>,
        want_cos: bool,
    ) -> Result<VirtualRegister, String> {
        let src: Reg = src.into();
        let angle = self.emit_fixed_trig_reduce(src);
        let (sine, cosine) = self.emit_fixed_sin_cos_magnitudes(&angle.r_high, &angle.r_low);
        let bit = self.allocate_register();
        let uses_sine = self.allocate_register();
        let negative = self.allocate_register();
        let magnitude = self.allocate_register();
        let half_turn = self.allocate_register();
        self.emit(abi::move_immediate(&bit, "Integer", "1"));
        // sin selects `sin r` in the even quadrants, cos in the odd ones.
        self.emit(abi::and_registers(&uses_sine, &angle.quadrant, &bit));
        if !want_cos {
            self.emit(abi::exclusive_or_registers(&uses_sine, &uses_sine, &bit));
        }
        let pick_cosine = self.label("fixed_trig_pick_cosine");
        let picked = self.label("fixed_trig_picked");
        self.emit(abi::compare_immediate(&uses_sine, "0"));
        self.emit(abi::branch_eq(&pick_cosine));
        self.emit(abi::move_register(&magnitude, &sine));
        self.emit(abi::move_register(&negative, &angle.negative_r));
        self.emit(abi::branch(&picked));
        self.emit(abi::label(&pick_cosine));
        self.emit(abi::move_register(&magnitude, &cosine));
        self.emit(abi::move_immediate(&negative, "Integer", "0"));
        self.emit(abi::label(&picked));
        // The half-turn sign: sin is negative in quadrants 2 and 3 (bit 1 of
        // k mod 4), cos in quadrants 1 and 2 (bit 1 of k mod 4 + 1).
        if want_cos {
            self.emit(abi::add_immediate(&half_turn, &angle.quadrant, 1));
            self.emit(abi::shift_right_immediate(&half_turn, &half_turn, 1));
            self.emit(abi::and_registers(&half_turn, &half_turn, &bit));
        } else {
            self.emit(abi::shift_right_immediate(&half_turn, &angle.quadrant, 1));
            self.emit(abi::exclusive_or_registers(
                &negative,
                &negative,
                &angle.negative_x,
            ));
        }
        self.emit(abi::exclusive_or_registers(
            &negative, &negative, &half_turn,
        ));
        let result = self.allocate_register();
        self.emit_q63_to_signed_fixed(&result, &magnitude, &negative);
        Ok(result)
    }

    /// Lower `tan` for a `Fixed` argument: the nearest `Fixed` to the true
    /// tangent, or `ErrOverflow` when the true tangent is outside the `Fixed`
    /// range (bug-615). No `Fixed` argument is a pole — pi/2 is irrational — so
    /// there is no undefined point to reject.
    ///
    /// With `|x| = k·(pi/2) + r`, `tan |x|` is `tan r` for even `k` and
    /// `−cot r` for odd `k`; `tan x` also carries the sign of `x`. Two paths:
    ///
    /// * **Even `k`, or odd `k` with `|r| >= 2^-8`**: the magnitude is
    ///   `sin|r| / cos|r|` (even) or `cos|r| / sin|r|` (odd), at most `2^8`, from
    ///   the Q1.63 series of [`Self::emit_fixed_sin_cos_magnitudes`] and a
    ///   32-bit-fraction long division rounded by its remainder. The `2^-61`
    ///   error of the operands is at most `2^-45` in a quotient of `2^8`.
    /// * **Odd `k` with `|r| < 2^-8`** (near a pole): the Q1.63 operands no
    ///   longer have the relative precision a quotient near `2^31` needs, so the
    ///   magnitude is `cot|r| = 1/|r| − |r|/3 − |r|³/45` (next term below
    ///   `2^-48`). `1/|r|` comes from the exact 128-bit `|r|` by a 73-step long
    ///   division carrying 8 guard bits, so it is good to `2^-31` units even at
    ///   `|r| = 2^-32`. `|r| < 2^-32` means `|tan| > 2^32`: `ErrOverflow`
    ///   without dividing. The overflow decision is made on the rounded
    ///   quotient, which is within half a unit of the truth.
    pub(crate) fn emit_fixed_tan(
        &mut self,
        src: impl Into<Reg>,
    ) -> Result<VirtualRegister, String> {
        let src: Reg = src.into();
        let angle = self.emit_fixed_trig_reduce(src);
        let result = self.allocate_register();
        let negative = self.allocate_register();
        let odd = self.allocate_register();
        let scratch = self.allocate_register();
        // tan x = sign(x) · sign(r) · (odd k ? −1 : 1) · |tan r or cot r|.
        self.emit(abi::move_immediate(&odd, "Integer", "1"));
        self.emit(abi::and_registers(&odd, &odd, &angle.quadrant));
        self.emit(abi::exclusive_or_registers(
            &negative,
            &odd,
            &angle.negative_r,
        ));
        self.emit(abi::exclusive_or_registers(
            &negative,
            &negative,
            &angle.negative_x,
        ));
        let general = self.label("fixed_tan_general");
        let apply_sign = self.label("fixed_tan_apply_sign");
        let overflow = self.label("fixed_tan_overflow");
        self.emit(abi::compare_immediate(&odd, "0"));
        self.emit(abi::branch_eq(&general));
        // |r| >= 2^-8  <=>  high word of |r|·2^127 >= 2^55.
        self.emit_const_u64(&scratch, 1u64 << 55);
        self.emit(abi::compare_registers(&scratch, &angle.r_high));
        self.emit(abi::branch_ls(&general));

        // Near a pole. |r| < 2^-32 (high word < 2^31): |cot r| > 2^32.
        self.emit_const_u64(&scratch, 1u64 << 31);
        self.emit(abi::compare_registers(&angle.r_high, &scratch));
        self.emit(abi::branch_lo(&overflow));
        // (qh, ql) = floor(2^167 / |r|·2^127) = floor(2^40 / |r|): the long
        // division starts from the remainder 2^94 < |r|·2^127, so 73 quotient
        // bits remain; the remainder stays below |r|·2^128 < 2^120.
        let rem_high = self.allocate_register();
        let rem_low = self.allocate_register();
        let quot_high = self.allocate_register();
        let quot_low = self.allocate_register();
        let counter = self.allocate_register();
        self.emit_const_u64(&rem_high, 1u64 << 30);
        self.emit(abi::move_immediate(&rem_low, "Integer", "0"));
        self.emit(abi::move_immediate(&quot_high, "Integer", "0"));
        self.emit(abi::move_immediate(&quot_low, "Integer", "0"));
        self.emit(abi::move_immediate(&counter, "Integer", "73"));
        let divide = self.label("fixed_tan_pole_divide");
        let subtract = self.label("fixed_tan_pole_subtract");
        let no_borrow = self.label("fixed_tan_pole_no_borrow");
        let next_bit = self.label("fixed_tan_pole_next_bit");
        let divided = self.label("fixed_tan_pole_divided");
        self.emit(abi::label(&divide));
        self.emit(abi::compare_immediate(&counter, "0"));
        self.emit(abi::branch_eq(&divided));
        for (high, low) in [(&rem_high, &rem_low), (&quot_high, &quot_low)] {
            self.emit(abi::shift_left_immediate(high, high, 1));
            self.emit(abi::shift_right_immediate(&scratch, low, 63));
            self.emit(abi::or_registers(high, high, &scratch));
            self.emit(abi::shift_left_immediate(low, low, 1));
        }
        // remainder >= |r|·2^127 (128-bit unsigned compare)?
        self.emit(abi::compare_registers(&rem_high, &angle.r_high));
        self.emit(abi::branch_hi(&subtract));
        self.emit(abi::branch_lo(&next_bit));
        self.emit(abi::compare_registers(&rem_low, &angle.r_low));
        self.emit(abi::branch_lo(&next_bit));
        self.emit(abi::label(&subtract));
        self.emit(abi::subtract_registers(&rem_high, &rem_high, &angle.r_high));
        self.emit(abi::compare_registers(&angle.r_low, &rem_low));
        self.emit(abi::branch_ls(&no_borrow));
        self.emit(abi::subtract_immediate(&rem_high, &rem_high, 1));
        self.emit(abi::label(&no_borrow));
        self.emit(abi::subtract_registers(&rem_low, &rem_low, &angle.r_low));
        self.emit(abi::add_immediate(&quot_low, &quot_low, 1));
        self.emit(abi::label(&next_bit));
        self.emit(abi::subtract_immediate(&counter, &counter, 1));
        self.emit(abi::branch(&divide));
        self.emit(abi::label(&divided));
        // correction = (|r|/3 + |r|³/45)·2^40, each term truncated (< 2^-40 off).
        let correction = self.allocate_register();
        let r = self.allocate_register();
        let power = self.allocate_register();
        self.emit(abi::shift_right_immediate(&correction, &angle.r_high, 23));
        self.emit(abi::move_immediate(&scratch, "Integer", "3"));
        self.emit(abi::unsigned_divide_registers(
            &correction,
            &correction,
            &scratch,
        ));
        self.emit(abi::shift_left_immediate(&r, &angle.r_high, 1));
        self.emit(abi::shift_right_immediate(&scratch, &angle.r_low, 63));
        self.emit(abi::or_registers(&r, &r, &scratch));
        self.emit(abi::unsigned_multiply_high_registers(&power, &r, &r));
        self.emit(abi::unsigned_multiply_high_registers(&power, &power, &r));
        self.emit(abi::shift_right_immediate(&power, &power, 24));
        self.emit(abi::move_immediate(&scratch, "Integer", "45"));
        self.emit(abi::unsigned_divide_registers(&power, &power, &scratch));
        self.emit(abi::add_registers(&correction, &correction, &power));
        // (qh, ql) −= correction; then += 2^7 to round away the 8 guard bits.
        let no_borrow = self.label("fixed_tan_pole_correction_no_borrow");
        self.emit(abi::compare_registers(&correction, &quot_low));
        self.emit(abi::branch_ls(&no_borrow));
        self.emit(abi::subtract_immediate(&quot_high, &quot_high, 1));
        self.emit(abi::label(&no_borrow));
        self.emit(abi::subtract_registers(&quot_low, &quot_low, &correction));
        self.emit(abi::add_immediate(&correction, &quot_low, 128));
        let no_carry = self.label("fixed_tan_pole_round_no_carry");
        self.emit(abi::compare_registers(&quot_low, &correction));
        self.emit(abi::branch_ls(&no_carry));
        self.emit(abi::add_immediate(&quot_high, &quot_high, 1));
        self.emit(abi::label(&no_carry));
        // |tan|·2^32 = (qh, ql) >> 8 must fit below 2^63.
        self.emit(abi::shift_right_immediate(&scratch, &quot_high, 7));
        self.emit(abi::compare_immediate(&scratch, "0"));
        self.emit(abi::branch_ne(&overflow));
        self.emit(abi::shift_left_immediate(&result, &quot_high, 56));
        self.emit(abi::shift_right_immediate(&scratch, &correction, 8));
        self.emit(abi::or_registers(&result, &result, &scratch));
        self.emit(abi::branch(&apply_sign));

        // Away from a pole: a rounded 32-bit-fraction quotient of the Q1.63
        // magnitudes, numerator sin (even k) or cos (odd k).
        self.emit(abi::label(&general));
        let (sine, cosine) = self.emit_fixed_sin_cos_magnitudes(&angle.r_high, &angle.r_low);
        let numerator = self.allocate_register();
        let denominator = self.allocate_register();
        let even_quadrant = self.label("fixed_tan_even_quadrant");
        let operands = self.label("fixed_tan_operands");
        self.emit(abi::compare_immediate(&odd, "0"));
        self.emit(abi::branch_eq(&even_quadrant));
        self.emit(abi::move_register(&numerator, &cosine));
        self.emit(abi::move_register(&denominator, &sine));
        self.emit(abi::branch(&operands));
        self.emit(abi::label(&even_quadrant));
        self.emit(abi::move_register(&numerator, &sine));
        self.emit(abi::move_register(&denominator, &cosine));
        self.emit(abi::label(&operands));
        // integer part, then 32 fraction bits; the remainder stays below the
        // denominator (<= 2^63), so doubling it never wraps.
        let remainder = self.allocate_register();
        let counter = self.allocate_register();
        self.emit(abi::unsigned_divide_registers(
            &result,
            &numerator,
            &denominator,
        ));
        self.emit(abi::multiply_subtract_registers(
            &remainder,
            &result,
            &denominator,
            &numerator,
        ));
        self.emit(abi::move_immediate(&counter, "Integer", "32"));
        let fraction = self.label("fixed_tan_fraction");
        let fraction_skip = self.label("fixed_tan_fraction_skip");
        let fraction_done = self.label("fixed_tan_fraction_done");
        self.emit(abi::label(&fraction));
        self.emit(abi::compare_immediate(&counter, "0"));
        self.emit(abi::branch_eq(&fraction_done));
        self.emit(abi::shift_left_immediate(&remainder, &remainder, 1));
        self.emit(abi::shift_left_immediate(&result, &result, 1));
        self.emit(abi::compare_registers(&remainder, &denominator));
        self.emit(abi::branch_lo(&fraction_skip));
        self.emit(abi::subtract_registers(
            &remainder,
            &remainder,
            &denominator,
        ));
        self.emit(abi::add_immediate(&result, &result, 1));
        self.emit(abi::label(&fraction_skip));
        self.emit(abi::subtract_immediate(&counter, &counter, 1));
        self.emit(abi::branch(&fraction));
        self.emit(abi::label(&fraction_done));
        // Round to nearest: 2·remainder >= denominator.
        let rounded = self.label("fixed_tan_rounded");
        self.emit(abi::shift_left_immediate(&remainder, &remainder, 1));
        self.emit(abi::compare_registers(&remainder, &denominator));
        self.emit(abi::branch_lo(&rounded));
        self.emit(abi::add_immediate(&result, &result, 1));
        self.emit(abi::label(&rounded));

        self.emit(abi::label(&apply_sign));
        self.emit_conditional_negate(&result, &negative, &scratch);
        let done = self.label("fixed_tan_done");
        self.emit(abi::branch(&done));
        self.emit(abi::label(&overflow));
        self.raise_error_bare("ErrOverflow")?;
        self.emit(abi::label(&done));
        Ok(result)
    }

    /// Lower `asin`/`acos` for a `Fixed` argument: the nearest `Fixed` to the
    /// true value, within one Q32.32 unit over the whole domain (bug-615).
    /// Inputs outside `[-1, 1]` fail with `ErrInvalidArgument`.
    ///
    /// `asin x = atan(x / sqrt(1 - x^2))`, handed to
    /// [`Self::emit_fixed_atan_ratio`] as an exact integer ratio rather than as
    /// a `Fixed` quotient — that is what survives `|x| -> 1`, where the quotient
    /// runs off to infinity and a Q32.32 numerator would keep only a bit or two.
    /// With `x = r/2^32`, `1 - x^2 = d/2^64` for `d = 2^64 - r^2`, which is
    /// exactly the low word of `-r*r` because `|r| <= 2^32`. The ratio is then
    /// `|r|*2^16 / round(sqrt(d)*2^16)`, and the root is
    /// [`Self::emit_fixed_sqrt`] of `d` read as an unsigned word (its radicand
    /// is `d*2^32`, so its Q32.32 result *is* `sqrt(d)` with 16 fraction bits).
    /// A half-unit error there moves the angle by at most `2^-48` radians,
    /// because `d(atan(r/s))/ds = -r/(s^2 + r^2)` is `-r/2^96` at this scale.
    /// `|x| = 1` gives `d = 0`, and the kernel answers `pi/2` for a zero
    /// denominator with no divide.
    ///
    /// `acos x = pi/2 - asin x`, formed in Q3.61 before the single rounding
    /// step so the two roundings never compound.
    pub(crate) fn emit_fixed_asin(
        &mut self,
        src: impl Into<Reg>,
        is_acos: bool,
    ) -> Result<VirtualRegister, String> {
        let src: Reg = src.into();
        let x_slot = self.allocate_stack_object("fixed_asin_x", 8);
        let root_slot = self.allocate_stack_object("fixed_asin_root", 8);
        self.emit(abi::store_u64(src, abi::stack_pointer(), x_slot));
        self.reset_temporary_registers();
        let x = self.allocate_register();
        self.emit(abi::load_u64(&x, abi::stack_pointer(), x_slot));
        // Domain check: |x| <= 1.
        let one = self.allocate_register();
        self.emit(abi::move_immediate(
            &one,
            abi::IMMEDIATE_CLASS_FIXED,
            &FIXED_ONE.to_string(),
        ));
        let in_domain_upper = self.label("fixed_asin_upper_ok");
        let domain_error = self.label("fixed_asin_domain_error");
        let checked = self.label("fixed_asin_checked");
        self.emit(abi::compare_registers(&x, &one));
        self.emit(abi::branch_le(&in_domain_upper));
        self.emit(abi::branch(&domain_error));
        self.emit(abi::label(&in_domain_upper));
        let neg_one = self.allocate_register();
        self.emit(abi::subtract_registers(&neg_one, abi::ZERO, &one));
        self.emit(abi::compare_registers(&x, &neg_one));
        self.emit(abi::branch_ge(&checked));
        self.emit(abi::label(&domain_error));
        self.raise_error_bare("ErrInvalidArgument")?;
        self.emit(abi::label(&checked));
        // d = 2^64 - r^2 as an unsigned word (exact; |r| = 2^32 gives 0).
        let magnitude = self.allocate_register();
        let d = self.allocate_register();
        self.emit(abi::arithmetic_shift_right_immediate(&d, &x, 63));
        self.emit(abi::exclusive_or_registers(&magnitude, &x, &d));
        self.emit(abi::subtract_registers(&magnitude, &magnitude, &d));
        self.emit(abi::multiply_registers(&d, &magnitude, &magnitude));
        self.emit(abi::subtract_registers(&d, abi::ZERO, &d));
        // root = round(sqrt(d) * 2^16); this resets the register file.
        let root = self.emit_fixed_sqrt(&d)?;
        self.emit(abi::store_u64(&root, abi::stack_pointer(), root_slot));
        self.reset_temporary_registers();
        let xr = self.allocate_register();
        let numerator = self.allocate_register();
        let denominator = self.allocate_register();
        let sign = self.allocate_register();
        self.emit(abi::load_u64(&xr, abi::stack_pointer(), x_slot));
        self.emit(abi::load_u64(&denominator, abi::stack_pointer(), root_slot));
        self.emit(abi::arithmetic_shift_right_immediate(&sign, &xr, 63));
        self.emit(abi::exclusive_or_registers(&numerator, &xr, &sign));
        self.emit(abi::subtract_registers(&numerator, &numerator, &sign));
        self.emit(abi::shift_left_immediate(&numerator, &numerator, 16));
        let angle = self.emit_fixed_atan_ratio(&numerator, &denominator);
        let negative = self.allocate_register();
        if is_acos {
            // acos x = pi/2 - asin x, and asin carries the sign of x.
            let quarter = self.allocate_register();
            let negative_x = self.label("fixed_acos_negative_x");
            let combined = self.label("fixed_acos_combined");
            self.emit_const_u64(&quarter, PI_OVER_2_Q61);
            self.emit(abi::compare_immediate(&xr, "0"));
            self.emit(abi::branch_lt(&negative_x));
            self.emit(abi::subtract_registers(&angle, &quarter, &angle));
            self.emit(abi::branch(&combined));
            self.emit(abi::label(&negative_x));
            self.emit(abi::add_registers(&angle, &quarter, &angle));
            self.emit(abi::label(&combined));
            self.emit(abi::move_immediate(&negative, "Integer", "0"));
        } else {
            self.emit(abi::shift_right_immediate(&negative, &xr, 63));
        }
        let result = self.allocate_register();
        self.emit_q61_to_signed_fixed(&result, &angle, &negative);
        Ok(result)
    }
}
