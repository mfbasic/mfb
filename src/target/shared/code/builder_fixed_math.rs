//! Deterministic compiler-owned `Fixed` (Q32.32) math implementations.
//!
//! `Fixed` is a signed 64-bit value scaled by `2^32`, so the real value of a
//! raw integer `r` is `r / 2^32`. The standard math package requires `Fixed`
//! overloads to be deterministic across targets and to round to the nearest
//! `Fixed`. Routing these through host floating-point/libm violates that
//! contract (libm differs across platforms, and `double` only has a 52-bit
//! mantissa so it loses precision for large `Fixed` values). The routines here
//! operate on the raw Q32.32 integer representation using only integer and
//! deterministic shift/add primitives.

use super::*;

/// Q32.32 scale factor (`2^32`) as a raw `Fixed` of value `1.0`.
const FIXED_ONE: u64 = 1u64 << 32;
/// Half of one Q32.32 unit (`0.5`), used as a round-to-nearest bias.
const FIXED_HALF: u64 = 1u64 << 31;
/// Mask selecting the fractional 32 bits of a Q32.32 value.
const FIXED_FRACTION_MASK: u64 = 0xFFFF_FFFF;
/// Number of CORDIC iterations; ~2^-32 precision, far finer than display.
const CORDIC_ITERATIONS: usize = 31;

impl CodeBuilder<'_> {
    /// Lower `floor`/`ceil`/`round` for a `Fixed` argument to an `Integer`
    /// result using raw Q32.32 arithmetic. The rounded integer always fits in
    /// `Integer` range, so no overflow check is required.
    pub(super) fn emit_fixed_rounding_to_integer(
        &mut self,
        function: &str,
        src: &str,
        dst: &str,
    ) -> Result<(), String> {
        match function {
            "floor" => {
                // Arithmetic shift right rounds toward negative infinity.
                self.emit(abi::arithmetic_shift_right_immediate(dst, src, 32));
            }
            "ceil" => {
                let frac = self.allocate_register()?;
                let mask = self.allocate_register()?;
                let done = self.label("fixed_ceil_done");
                self.emit(abi::arithmetic_shift_right_immediate(dst, src, 32));
                self.emit(abi::move_immediate(
                    &mask,
                    "Integer",
                    &FIXED_FRACTION_MASK.to_string(),
                ));
                self.emit(abi::and_registers(&frac, src, &mask));
                self.emit(abi::compare_immediate(&frac, "0"));
                self.emit(abi::branch_eq(&done));
                self.emit(abi::add_immediate(dst, dst, 1));
                self.emit(abi::label(&done));
            }
            "round" => {
                let whole = self.allocate_register()?;
                let frac = self.allocate_register()?;
                let mask = self.allocate_register()?;
                let threshold = self.allocate_register()?;
                let negative = self.label("fixed_round_negative");
                let compare = self.label("fixed_round_compare");
                let done = self.label("fixed_round_done");
                self.emit(abi::arithmetic_shift_right_immediate(&whole, src, 32));
                self.emit(abi::move_immediate(
                    &mask,
                    "Integer",
                    &FIXED_FRACTION_MASK.to_string(),
                ));
                self.emit(abi::and_registers(&frac, src, &mask));
                // Ties round away from zero: for negative inputs the fractional
                // part must strictly exceed 0.5 to round toward zero, so use a
                // threshold of 0.5 (>=) for non-negative and 0.5+1 (>=) for
                // negative values.
                self.emit(abi::compare_immediate(src, "0"));
                self.emit(abi::branch_lt(&negative));
                self.emit(abi::move_immediate(
                    &threshold,
                    "Integer",
                    &FIXED_HALF.to_string(),
                ));
                self.emit(abi::branch(&compare));
                self.emit(abi::label(&negative));
                self.emit(abi::move_immediate(
                    &threshold,
                    "Integer",
                    &(FIXED_HALF + 1).to_string(),
                ));
                self.emit(abi::label(&compare));
                self.emit(abi::move_register(dst, &whole));
                self.emit(abi::compare_registers(&frac, &threshold));
                self.emit(abi::branch_lo(&done));
                self.emit(abi::add_immediate(dst, dst, 1));
                self.emit(abi::label(&done));
            }
            other => {
                return Err(format!("fixed rounding does not support math.{other}"));
            }
        }
        Ok(())
    }

    /// Deterministic Q32.32 square root of a non-negative `Fixed` in `src`,
    /// writing the nearest-`Fixed` result to `dst`.
    ///
    /// The real result is `sqrt(src / 2^32)`, whose raw representation is
    /// `sqrt(src_raw * 2^32)`. This is computed with a digit-by-digit
    /// (restoring) integer square root over the 128-bit radicand
    /// `src_raw << 64` (the extra `<< 32` left-justifies the 96-bit value
    /// `src_raw << 32` to the top of the 128-bit window). The result is at most
    /// 48 bits, so every loop quantity except the radicand shift stays within
    /// 64 bits. The caller guarantees `src >= 0`.
    pub(super) fn emit_fixed_sqrt(&mut self, src: &str) -> Result<String, String> {
        // The algorithm needs a handful of working registers. Spill the input
        // to the stack and reset the temporary register file so the surrounding
        // expression's prior allocations do not exhaust the pool, mirroring the
        // external-call lowering pattern.
        let slot = self.allocate_stack_object("fixed_sqrt_input", 8);
        self.emit(abi::store_u64(src, abi::stack_pointer(), slot));
        self.reset_temporary_registers();
        let input = self.allocate_register()?;
        self.emit(abi::load_u64(&input, abi::stack_pointer(), slot));
        let src = input.as_str();
        let dst = self.allocate_register()?;
        let nhi = self.allocate_register()?;
        let nlo = self.allocate_register()?;
        let res = self.allocate_register()?;
        let rem = self.allocate_register()?;
        let digit = self.allocate_register()?;
        let trial = self.allocate_register()?;
        let counter = self.allocate_register()?;
        let carry = self.allocate_register()?;
        let loop_label = self.label("fixed_sqrt_loop");
        let skip = self.label("fixed_sqrt_skip");
        let done = self.label("fixed_sqrt_done");
        let round_done = self.label("fixed_sqrt_round_done");
        // Radicand = src_raw << 64: high word holds src_raw, low word is zero.
        self.emit(abi::move_register(&nhi, src));
        self.emit(abi::move_immediate(&nlo, "Integer", "0"));
        self.emit(abi::move_immediate(&res, "Integer", "0"));
        self.emit(abi::move_immediate(&rem, "Integer", "0"));
        self.emit(abi::move_immediate(&counter, "Integer", "48"));
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate(&counter, "0"));
        self.emit(abi::branch_eq(&done));
        // digit = top two bits of the 128-bit radicand.
        self.emit(abi::shift_right_immediate(&digit, &nhi, 62));
        // Shift the 128-bit radicand left by two, feeding the next pair.
        self.emit(abi::shift_left_immediate(&nhi, &nhi, 2));
        self.emit(abi::shift_right_immediate(&carry, &nlo, 62));
        self.emit(abi::or_registers(&nhi, &nhi, &carry));
        self.emit(abi::shift_left_immediate(&nlo, &nlo, 2));
        // rem = rem * 4 + digit; res *= 2.
        self.emit(abi::shift_left_immediate(&rem, &rem, 2));
        self.emit(abi::or_registers(&rem, &rem, &digit));
        self.emit(abi::shift_left_immediate(&res, &res, 1));
        // trial = 2 * res + 1.
        self.emit(abi::shift_left_immediate(&trial, &res, 1));
        self.emit(abi::add_immediate(&trial, &trial, 1));
        self.emit(abi::compare_registers(&rem, &trial));
        self.emit(abi::branch_lo(&skip));
        self.emit(abi::subtract_registers(&rem, &rem, &trial));
        self.emit(abi::add_immediate(&res, &res, 1));
        self.emit(abi::label(&skip));
        self.emit(abi::subtract_immediate(&counter, &counter, 1));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&done));
        // Round to nearest: if the leftover remainder exceeds the result, the
        // true root is closer to res + 1.
        self.emit(abi::compare_registers(&rem, &res));
        self.emit(abi::branch_le(&round_done));
        self.emit(abi::add_immediate(&res, &res, 1));
        self.emit(abi::label(&round_done));
        self.emit(abi::move_register(&dst, &res));
        Ok(dst)
    }

    /// Move a signed 64-bit constant into `reg`.
    fn emit_const_i64(&mut self, reg: &str, value: i64) {
        self.emit(abi::move_immediate(
            reg,
            "Integer",
            &(value as u64).to_string(),
        ));
    }

    /// Round-to-nearest Q32.32 multiply `(a * b) / 2^32` into a fresh register.
    /// Intended for internal use where the result is known to stay in range, so
    /// no overflow trap is emitted. Nets a single new register (the result); the
    /// working temporaries are released before returning.
    fn emit_fixed_mul(&mut self, a: &str, b: &str) -> Result<String, String> {
        let result = self.allocate_register()?;
        let saved = self.next_register;
        let s0 = self.allocate_register()?;
        let s1 = self.allocate_register()?;
        self.emit_fixed_mul_inplace(&result, a, b, &s0, &s1);
        self.next_register = saved;
        Ok(result)
    }

    /// Round-to-nearest Q32.32 multiply writing `(a * b) / 2^32` to `dst` using
    /// the two caller-provided scratch registers `s0`/`s1`. Allocation-free, so
    /// it is safe to call inside register-tight runtime loops. `dst` may alias
    /// `a` or `b`; the inputs are fully consumed before `dst` is written.
    fn emit_fixed_mul_inplace(&mut self, dst: &str, a: &str, b: &str, s0: &str, s1: &str) {
        self.emit(abi::multiply_registers(s0, a, b)); // low 64 bits
        self.emit(abi::signed_multiply_high_registers(s1, a, b)); // high 64 bits
                                                                  // Combined middle word = (s1 << 32) | (s0 >>u 32) = bits[95:32].
        self.emit(abi::shift_left_immediate(s1, s1, 32));
        self.emit(abi::shift_right_immediate(dst, s0, 32));
        self.emit(abi::or_registers(dst, dst, s1));
        // Round half up using bit 31 of the low word (the top discarded bit).
        self.emit(abi::shift_right_immediate(s0, s0, 31));
        self.emit(abi::shift_left_immediate(s0, s0, 63));
        self.emit(abi::shift_right_immediate(s0, s0, 63));
        self.emit(abi::add_registers(dst, dst, s0));
    }

    /// Run unrolled CORDIC circular vectoring on `(vx, vy)` accumulating the
    /// rotation angle into `z`. Drives `vy` toward zero so that `z` converges to
    /// `atan(vy0 / vx0)`. Requires `vx > 0`. The registers are updated in place.
    fn emit_cordic_vectoring(&mut self, vx: &str, vy: &str, z: &str) -> Result<(), String> {
        let sx = self.allocate_register()?;
        let sy = self.allocate_register()?;
        let konst = self.allocate_register()?;
        for i in 0..CORDIC_ITERATIONS {
            let negative = self.label("cordic_vec_neg");
            let done = self.label("cordic_vec_done");
            if i == 0 {
                self.emit(abi::move_register(&sx, vx));
                self.emit(abi::move_register(&sy, vy));
            } else {
                self.emit(abi::arithmetic_shift_right_immediate(&sx, vx, i as u8));
                self.emit(abi::arithmetic_shift_right_immediate(&sy, vy, i as u8));
            }
            self.emit_const_i64(&konst, cordic_atan_raw(i));
            self.emit(abi::compare_immediate(vy, "0"));
            self.emit(abi::branch_lt(&negative));
            // vy >= 0: rotate clockwise to reduce vy.
            self.emit(abi::add_registers(vx, vx, &sy));
            self.emit(abi::subtract_registers(vy, vy, &sx));
            self.emit(abi::add_registers(z, z, &konst));
            self.emit(abi::branch(&done));
            self.emit(abi::label(&negative));
            self.emit(abi::subtract_registers(vx, vx, &sy));
            self.emit(abi::add_registers(vy, vy, &sx));
            self.emit(abi::subtract_registers(z, z, &konst));
            self.emit(abi::label(&done));
        }
        Ok(())
    }

    /// Deterministic Q32.32 `atan2(y, x)` returning the angle in radians.
    pub(super) fn emit_fixed_atan2(&mut self, y_src: &str, x_src: &str) -> Result<String, String> {
        let y_slot = self.allocate_stack_object("fixed_atan2_y", 8);
        let x_slot = self.allocate_stack_object("fixed_atan2_x", 8);
        self.emit(abi::store_u64(y_src, abi::stack_pointer(), y_slot));
        self.emit(abi::store_u64(x_src, abi::stack_pointer(), x_slot));
        self.reset_temporary_registers();
        let vy = self.allocate_register()?;
        let vx = self.allocate_register()?;
        let z = self.allocate_register()?;
        let offset = self.allocate_register()?;
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&vy, abi::stack_pointer(), y_slot));
        self.emit(abi::load_u64(&vx, abi::stack_pointer(), x_slot));

        let general = self.label("fixed_atan2_general");
        let finish = self.label("fixed_atan2_finish");
        // y == 0 axis cases give exact results (CORDIC would otherwise leave a
        // tiny non-zero residue). atan2(0, x>=0) = 0; atan2(0, x<0) = pi.
        let y_zero_check = self.label("fixed_atan2_y_nonzero");
        let y_zero_neg_x = self.label("fixed_atan2_y0_negx");
        self.emit(abi::compare_immediate(&vy, "0"));
        self.emit(abi::branch_ne(&y_zero_check));
        self.emit(abi::compare_immediate(&vx, "0"));
        self.emit(abi::branch_lt(&y_zero_neg_x));
        self.emit(abi::move_immediate(&result, "Integer", "0"));
        self.emit(abi::branch(&finish));
        self.emit(abi::label(&y_zero_neg_x));
        self.emit_const_i64(&result, fixed_pi());
        self.emit(abi::branch(&finish));
        self.emit(abi::label(&y_zero_check));
        // x == 0 axis cases.
        let x_zero_pos = self.label("fixed_atan2_x0_pos");
        let x_zero_neg = self.label("fixed_atan2_x0_neg");
        self.emit(abi::compare_immediate(&vx, "0"));
        self.emit(abi::branch_ne(&general));
        self.emit(abi::compare_immediate(&vy, "0"));
        self.emit(abi::branch_gt(&x_zero_pos));
        self.emit(abi::branch_lt(&x_zero_neg));
        self.emit(abi::move_immediate(&result, "Integer", "0"));
        self.emit(abi::branch(&finish));
        self.emit(abi::label(&x_zero_pos));
        self.emit_const_i64(&result, fixed_pi_over_2());
        self.emit(abi::branch(&finish));
        self.emit(abi::label(&x_zero_neg));
        self.emit_const_i64(&result, -fixed_pi_over_2());
        self.emit(abi::branch(&finish));

        self.emit(abi::label(&general));
        let x_positive = self.label("fixed_atan2_x_positive");
        let offset_neg = self.label("fixed_atan2_offset_neg");
        let setup_done = self.label("fixed_atan2_setup_done");
        self.emit(abi::compare_immediate(&vx, "0"));
        self.emit(abi::branch_gt(&x_positive));
        // x < 0: reflect through the origin and add +/- pi.
        self.emit(abi::subtract_registers(&vx, "xzr", &vx));
        self.emit(abi::subtract_registers(&vy, "xzr", &vy));
        self.emit(abi::compare_immediate(&vy, "0"));
        // vy here is already negated; the offset sign depends on the original y.
        // original y >= 0  <=>  negated vy <= 0.
        self.emit(abi::branch_gt(&offset_neg));
        self.emit_const_i64(&offset, fixed_pi());
        self.emit(abi::branch(&setup_done));
        self.emit(abi::label(&offset_neg));
        self.emit_const_i64(&offset, -fixed_pi());
        self.emit(abi::branch(&setup_done));
        self.emit(abi::label(&x_positive));
        self.emit(abi::move_immediate(&offset, "Integer", "0"));
        self.emit(abi::label(&setup_done));

        self.emit(abi::move_immediate(&z, "Integer", "0"));
        self.emit_cordic_vectoring(&vx, &vy, &z)?;
        self.emit(abi::add_registers(&result, &z, &offset));
        self.emit(abi::label(&finish));
        Ok(result)
    }

    /// Run unrolled CORDIC circular rotation, rotating the vector `(cosr, sinr)`
    /// by the angle in `z` (which must lie within the CORDIC convergence range,
    /// roughly `[-pi/4, pi/4]` here). On entry `cosr` holds the inverse gain and
    /// `sinr` is zero; on exit `cosr ~= cos(z0)` and `sinr ~= sin(z0)`.
    fn emit_cordic_rotation(&mut self, cosr: &str, sinr: &str, z: &str) -> Result<(), String> {
        let sx = self.allocate_register()?;
        let sy = self.allocate_register()?;
        let konst = self.allocate_register()?;
        for i in 0..CORDIC_ITERATIONS {
            let negative = self.label("cordic_rot_neg");
            let done = self.label("cordic_rot_done");
            if i == 0 {
                self.emit(abi::move_register(&sx, cosr));
                self.emit(abi::move_register(&sy, sinr));
            } else {
                self.emit(abi::arithmetic_shift_right_immediate(&sx, cosr, i as u8));
                self.emit(abi::arithmetic_shift_right_immediate(&sy, sinr, i as u8));
            }
            self.emit_const_i64(&konst, cordic_atan_raw(i));
            self.emit(abi::compare_immediate(z, "0"));
            self.emit(abi::branch_lt(&negative));
            // z >= 0: rotate by +atan(2^-i).
            self.emit(abi::subtract_registers(cosr, cosr, &sy));
            self.emit(abi::add_registers(sinr, sinr, &sx));
            self.emit(abi::subtract_registers(z, z, &konst));
            self.emit(abi::branch(&done));
            self.emit(abi::label(&negative));
            self.emit(abi::add_registers(cosr, cosr, &sy));
            self.emit(abi::subtract_registers(sinr, sinr, &sx));
            self.emit(abi::add_registers(z, z, &konst));
            self.emit(abi::label(&done));
        }
        Ok(())
    }

    /// Deterministic Q32.32 `sin` and `cos` of `src`. Returns `(sin, cos)`
    /// registers. Reduces the angle to `[-pi/4, pi/4]` and tracks the quadrant.
    fn emit_fixed_sincos(&mut self, src: &str) -> Result<(String, String), String> {
        let slot = self.allocate_stack_object("fixed_sincos_input", 8);
        self.emit(abi::store_u64(src, abi::stack_pointer(), slot));
        self.reset_temporary_registers();
        let theta = self.allocate_register()?;
        self.emit(abi::load_u64(&theta, abi::stack_pointer(), slot));

        // k = round(theta * 2/pi): the number of pi/2 quadrants.
        let two_over_pi = self.allocate_register()?;
        self.emit_const_i64(&two_over_pi, fixed_two_over_pi());
        let scaled = self.emit_fixed_mul(&theta, &two_over_pi)?;
        let k = self.allocate_register()?;
        self.emit(abi::move_immediate(&k, "Integer", &FIXED_HALF.to_string()));
        self.emit(abi::add_registers(&k, &scaled, &k));
        self.emit(abi::arithmetic_shift_right_immediate(&k, &k, 32));
        // r = theta - k * (pi/2), computed modulo 2^64 so a large k*pi/2 that
        // overflows 64 bits still yields the correct small reduced angle.
        let pi_over_2 = self.allocate_register()?;
        self.emit_const_i64(&pi_over_2, fixed_pi_over_2());
        let kq = self.allocate_register()?;
        self.emit(abi::multiply_registers(&kq, &k, &pi_over_2));
        let r = self.allocate_register()?;
        self.emit(abi::subtract_registers(&r, &theta, &kq));

        // CORDIC rotation on the reduced angle.
        let cosr = self.allocate_register()?;
        let sinr = self.allocate_register()?;
        self.emit_const_i64(&cosr, cordic_gain_inverse());
        self.emit(abi::move_immediate(&sinr, "Integer", "0"));
        self.emit_cordic_rotation(&cosr, &sinr, &r)?;

        // Quadrant selection from k mod 4.
        let kmod = self.allocate_register()?;
        let three = self.allocate_register()?;
        self.emit(abi::move_immediate(&three, "Integer", "3"));
        self.emit(abi::and_registers(&kmod, &k, &three));
        let sin_out = self.allocate_register()?;
        let cos_out = self.allocate_register()?;
        let q1 = self.label("fixed_sincos_q1");
        let q2 = self.label("fixed_sincos_q2");
        let q3 = self.label("fixed_sincos_q3");
        let done = self.label("fixed_sincos_done");
        self.emit(abi::compare_immediate(&kmod, "1"));
        self.emit(abi::branch_eq(&q1));
        self.emit(abi::compare_immediate(&kmod, "2"));
        self.emit(abi::branch_eq(&q2));
        self.emit(abi::compare_immediate(&kmod, "3"));
        self.emit(abi::branch_eq(&q3));
        // q0: sin = sinr, cos = cosr.
        self.emit(abi::move_register(&sin_out, &sinr));
        self.emit(abi::move_register(&cos_out, &cosr));
        self.emit(abi::branch(&done));
        // q1: sin = cosr, cos = -sinr.
        self.emit(abi::label(&q1));
        self.emit(abi::move_register(&sin_out, &cosr));
        self.emit(abi::subtract_registers(&cos_out, "xzr", &sinr));
        self.emit(abi::branch(&done));
        // q2: sin = -sinr, cos = -cosr.
        self.emit(abi::label(&q2));
        self.emit(abi::subtract_registers(&sin_out, "xzr", &sinr));
        self.emit(abi::subtract_registers(&cos_out, "xzr", &cosr));
        self.emit(abi::branch(&done));
        // q3: sin = -cosr, cos = sinr.
        self.emit(abi::label(&q3));
        self.emit(abi::subtract_registers(&sin_out, "xzr", &cosr));
        self.emit(abi::move_register(&cos_out, &sinr));
        self.emit(abi::label(&done));
        Ok((sin_out, cos_out))
    }

    /// Lower `sin`/`cos` for a `Fixed` argument.
    pub(super) fn emit_fixed_sin_cos(
        &mut self,
        src: &str,
        want_cos: bool,
    ) -> Result<String, String> {
        let (sin_out, cos_out) = self.emit_fixed_sincos(src)?;
        Ok(if want_cos { cos_out } else { sin_out })
    }

    /// Lower `tan` for a `Fixed` argument as `sin / cos`. Undefined points
    /// (`cos == 0`) fail with `ErrInvalidArgument`.
    pub(super) fn emit_fixed_tan(&mut self, src: &str) -> Result<String, String> {
        let (sin_out, cos_out) = self.emit_fixed_sincos(src)?;
        // Spill across the division helper, which resets the register file.
        let sin_slot = self.allocate_stack_object("fixed_tan_sin", 8);
        let cos_slot = self.allocate_stack_object("fixed_tan_cos", 8);
        self.emit(abi::store_u64(&sin_out, abi::stack_pointer(), sin_slot));
        self.emit(abi::store_u64(&cos_out, abi::stack_pointer(), cos_slot));
        self.reset_temporary_registers();
        let sin_reg = self.allocate_register()?;
        let cos_reg = self.allocate_register()?;
        self.emit(abi::load_u64(&sin_reg, abi::stack_pointer(), sin_slot));
        self.emit(abi::load_u64(&cos_reg, abi::stack_pointer(), cos_slot));
        // `emit_fixed_divide` already fails with ErrInvalidArgument when the
        // divisor (cos) is zero, which matches the spec's undefined-point rule.
        let result = self.allocate_register()?;
        self.emit_fixed_divide(&result, &sin_reg, &cos_reg)?;
        Ok(result)
    }

    /// Lower `asin`/`acos` for a `Fixed` argument. Inputs outside `[-1, 1]` fail
    /// with `ErrInvalidArgument`. Uses `asin(x) = atan2(x, sqrt(1 - x^2))` and
    /// `acos(x) = atan2(sqrt(1 - x^2), x)`.
    pub(super) fn emit_fixed_asin(&mut self, src: &str, is_acos: bool) -> Result<String, String> {
        let x_slot = self.allocate_stack_object("fixed_asin_x", 8);
        self.emit(abi::store_u64(src, abi::stack_pointer(), x_slot));
        self.reset_temporary_registers();
        let x = self.allocate_register()?;
        self.emit(abi::load_u64(&x, abi::stack_pointer(), x_slot));
        // Domain check: |x| <= 1.
        let one = self.allocate_register()?;
        self.emit(abi::move_immediate(&one, "Fixed", &FIXED_ONE.to_string()));
        let in_domain_upper = self.label("fixed_asin_upper_ok");
        let domain_error = self.label("fixed_asin_domain_error");
        let checked = self.label("fixed_asin_checked");
        self.emit(abi::compare_registers(&x, &one));
        self.emit(abi::branch_le(&in_domain_upper));
        self.emit(abi::branch(&domain_error));
        self.emit(abi::label(&in_domain_upper));
        let neg_one = self.allocate_register()?;
        self.emit(abi::subtract_registers(&neg_one, "xzr", &one));
        self.emit(abi::compare_registers(&x, &neg_one));
        self.emit(abi::branch_ge(&checked));
        self.emit(abi::label(&domain_error));
        self.emit_invalid_argument_return()?;
        self.emit(abi::label(&checked));
        // s = sqrt(1 - x^2).
        let x2 = self.emit_fixed_mul(&x, &x)?;
        let one_minus = self.allocate_register()?;
        self.emit(abi::move_immediate(
            &one_minus,
            "Fixed",
            &FIXED_ONE.to_string(),
        ));
        self.emit(abi::subtract_registers(&one_minus, &one_minus, &x2));
        let s = self.emit_fixed_sqrt(&one_minus)?; // resets register file
        let xr = self.allocate_register()?;
        self.emit(abi::load_u64(&xr, abi::stack_pointer(), x_slot));
        if is_acos {
            self.emit_fixed_atan2(&s, &xr)
        } else {
            self.emit_fixed_atan2(&xr, &s)
        }
    }

    /// Lower `exp` for a `Fixed` argument. Computes `2^n * exp(r)` with
    /// `n = round(x / ln2)` and `r = x - n*ln2`, evaluating `exp(r)` by a Taylor
    /// series. Overflow beyond `Fixed` range fails with `ErrOverflow`.
    pub(super) fn emit_fixed_exp(&mut self, src: &str) -> Result<String, String> {
        let x_slot = self.allocate_stack_object("fixed_exp_x", 8);
        self.emit(abi::store_u64(src, abi::stack_pointer(), x_slot));
        self.reset_temporary_registers();
        let x = self.allocate_register()?;
        self.emit(abi::load_u64(&x, abi::stack_pointer(), x_slot));
        // n = round(x / ln2).
        let inv_ln2 = self.allocate_register()?;
        self.emit_const_i64(&inv_ln2, fixed_inv_ln2());
        let scaled = self.emit_fixed_mul(&x, &inv_ln2)?;
        let n = self.allocate_register()?;
        self.emit(abi::move_immediate(&n, "Integer", &FIXED_HALF.to_string()));
        self.emit(abi::add_registers(&n, &scaled, &n));
        self.emit(abi::arithmetic_shift_right_immediate(&n, &n, 32));
        // r = x - n*ln2 (mod 2^64).
        let ln2 = self.allocate_register()?;
        self.emit_const_i64(&ln2, fixed_ln2());
        let nl = self.allocate_register()?;
        self.emit(abi::multiply_registers(&nl, &n, &ln2));
        let r = self.allocate_register()?;
        self.emit(abi::subtract_registers(&r, &x, &nl));
        // exp(r) via Taylor series: sum = 1 + r + r^2/2! + ...
        let sum = self.allocate_register()?;
        let term = self.allocate_register()?;
        let k = self.allocate_register()?;
        let counter = self.allocate_register()?;
        let s0 = self.allocate_register()?;
        let s1 = self.allocate_register()?;
        self.emit(abi::move_immediate(&sum, "Fixed", &FIXED_ONE.to_string()));
        self.emit(abi::move_immediate(&term, "Fixed", &FIXED_ONE.to_string()));
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
        Ok(sum)
    }

    /// Multiply `value` (a `Fixed`) by `2^n` in place where `n` is a runtime
    /// signed integer, trapping with `ErrOverflow` if the result leaves `Fixed`
    /// range. Used to recombine the exponent in `exp`/`pow`.
    fn emit_fixed_scale_by_power_of_two(&mut self, value: &str, n: &str) -> Result<(), String> {
        let count = self.allocate_register()?;
        let limit = self.allocate_register()?;
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
        self.emit_overflow_return()?;
        self.emit(abi::label(&no_overflow));
        self.emit(abi::shift_left_immediate(value, value, 1));
        self.emit(abi::subtract_immediate(&count, &count, 1));
        self.emit(abi::branch(&up_loop));
        self.emit(abi::label(&up_done));
        let finish = self.label("fixed_scale_finish");
        self.emit(abi::branch(&finish));
        // n < 0: halve `-n` times (arithmetic shift; value is non-negative).
        self.emit(abi::label(&negative));
        self.emit(abi::subtract_registers(&count, "xzr", n));
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
    pub(super) fn emit_fixed_log(&mut self, src: &str, base10: bool) -> Result<String, String> {
        let x_slot = self.allocate_stack_object("fixed_log_x", 8);
        self.emit(abi::store_u64(src, abi::stack_pointer(), x_slot));
        self.reset_temporary_registers();
        let x = self.allocate_register()?;
        self.emit(abi::load_u64(&x, abi::stack_pointer(), x_slot));
        let positive = self.label("fixed_log_positive");
        self.emit(abi::compare_immediate(&x, "0"));
        self.emit(abi::branch_gt(&positive));
        self.emit_invalid_argument_return()?;
        self.emit(abi::label(&positive));
        // Normalise x into [1, 2): m_raw in [2^32, 2^33), tracking exponent e.
        let m = self.allocate_register()?;
        let e = self.allocate_register()?;
        let upper = self.allocate_register()?;
        let lower = self.allocate_register()?;
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
        let numerator = self.allocate_register()?;
        let denominator = self.allocate_register()?;
        let one = self.allocate_register()?;
        self.emit(abi::move_immediate(&one, "Fixed", &FIXED_ONE.to_string()));
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
        let num_reg = self.allocate_register()?;
        let den_reg = self.allocate_register()?;
        self.emit(abi::load_u64(&num_reg, abi::stack_pointer(), num_slot));
        self.emit(abi::load_u64(&den_reg, abi::stack_pointer(), den_slot));
        let t = self.allocate_register()?;
        self.emit_fixed_divide(&t, &num_reg, &den_reg)?;
        // Series for ln(m) using t and t^2.
        let t2 = self.allocate_register()?;
        let saved = self.next_register;
        let ms0 = self.allocate_register()?;
        let ms1 = self.allocate_register()?;
        self.emit_fixed_mul_inplace(&t2, &t, &t, &ms0, &ms1);
        self.next_register = saved;
        let sum = self.allocate_register()?;
        let term = self.allocate_register()?;
        let k = self.allocate_register()?;
        let counter = self.allocate_register()?;
        let s0 = self.allocate_register()?;
        let s1 = self.allocate_register()?;
        let scratch = self.allocate_register()?;
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
        let e_reg = self.allocate_register()?;
        let ln2 = self.allocate_register()?;
        self.emit(abi::load_u64(&e_reg, abi::stack_pointer(), e_slot));
        self.emit_const_i64(&ln2, fixed_ln2());
        let elog = self.allocate_register()?;
        self.emit(abi::multiply_registers(&elog, &e_reg, &ln2));
        self.emit(abi::add_registers(&sum, &sum, &elog));
        if base10 {
            let inv_ln10 = self.allocate_register()?;
            self.emit_const_i64(&inv_ln10, fixed_inv_ln10());
            return self.emit_fixed_mul(&sum, &inv_ln10);
        }
        Ok(sum)
    }

    /// Lower `pow(base, exponent)` for `Fixed` arguments. Whole-number exponents
    /// use exact repeated multiplication (any base sign, reciprocal for negative
    /// exponents); fractional exponents use `exp(exponent * ln(base))`, which
    /// requires `base > 0`. Overflow fails with `ErrOverflow`.
    pub(super) fn emit_fixed_pow_general(
        &mut self,
        base: &str,
        exponent: &str,
    ) -> Result<String, String> {
        let base_slot = self.allocate_stack_object("fixed_pow_base", 8);
        let exp_slot = self.allocate_stack_object("fixed_pow_exp", 8);
        self.emit(abi::store_u64(base, abi::stack_pointer(), base_slot));
        self.emit(abi::store_u64(exponent, abi::stack_pointer(), exp_slot));
        self.reset_temporary_registers();
        let exp_reg = self.allocate_register()?;
        self.emit(abi::load_u64(&exp_reg, abi::stack_pointer(), exp_slot));
        // Whole-number exponent? (no fractional Q32.32 bits)
        let frac = self.allocate_register()?;
        let mask = self.allocate_register()?;
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
        let n = self.allocate_register()?;
        self.emit(abi::arithmetic_shift_right_immediate(&n, &exp_reg, 32));
        let count = self.allocate_register()?;
        self.emit(abi::move_register(&count, &n));
        self.emit_abs_i64(&count)?;
        let base_reg = self.allocate_register()?;
        self.emit(abi::load_u64(&base_reg, abi::stack_pointer(), base_slot));
        let result = self.allocate_register()?;
        let product = self.allocate_register()?;
        self.emit(abi::move_immediate(
            &result,
            "Fixed",
            &FIXED_ONE.to_string(),
        ));
        let mul_loop = self.label("fixed_pow_int_loop");
        let mul_done = self.label("fixed_pow_int_done");
        self.emit(abi::label(&mul_loop));
        self.emit(abi::compare_immediate(&count, "0"));
        self.emit(abi::branch_eq(&mul_done));
        self.emit_fixed_multiply(&product, &result, &base_reg)?;
        self.emit(abi::move_register(&result, &product));
        self.emit(abi::subtract_immediate(&count, &count, 1));
        self.emit(abi::branch(&mul_loop));
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
        let denom = self.allocate_register()?;
        let one = self.allocate_register()?;
        let recip = self.allocate_register()?;
        self.emit(abi::load_u64(
            &denom,
            abi::stack_pointer(),
            integer_result_slot,
        ));
        self.emit(abi::move_immediate(&one, "Fixed", &FIXED_ONE.to_string()));
        self.emit_fixed_divide(&recip, &one, &denom)?;
        self.emit(abi::store_u64(
            &recip,
            abi::stack_pointer(),
            integer_result_slot,
        ));
        let reload = self.label("fixed_pow_int_reload");
        self.emit(abi::branch(&reload));
        self.emit(abi::label(&nonneg));
        self.emit(abi::store_u64(
            &result,
            abi::stack_pointer(),
            integer_result_slot,
        ));
        self.emit(abi::label(&reload));
        self.reset_temporary_registers();
        let int_result = self.allocate_register()?;
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
        let base_reg = self.allocate_register()?;
        self.emit(abi::load_u64(&base_reg, abi::stack_pointer(), base_slot));
        // ln(base) (also enforces base > 0 via ErrInvalidArgument).
        let ln_base = self.emit_fixed_log(&base_reg, false)?;
        let ln_slot = self.allocate_stack_object("fixed_pow_ln", 8);
        self.emit(abi::store_u64(&ln_base, abi::stack_pointer(), ln_slot));
        self.reset_temporary_registers();
        let exp_reg = self.allocate_register()?;
        let ln_reg = self.allocate_register()?;
        self.emit(abi::load_u64(&exp_reg, abi::stack_pointer(), exp_slot));
        self.emit(abi::load_u64(&ln_reg, abi::stack_pointer(), ln_slot));
        let product = self.emit_fixed_mul(&exp_reg, &ln_reg)?;
        let frac_result = self.emit_fixed_exp(&product)?;
        self.emit(abi::store_u64(
            &frac_result,
            abi::stack_pointer(),
            result_slot,
        ));

        self.emit(abi::label(&finish));
        self.reset_temporary_registers();
        let final_result = self.allocate_register()?;
        self.emit(abi::load_u64(
            &final_result,
            abi::stack_pointer(),
            result_slot,
        ));
        Ok(final_result)
    }
}

/// Raw Q32.32 representation of a real value.
fn fixed_raw(value: f64) -> i64 {
    (value * 4_294_967_296.0).round() as i64
}

/// `atan(2^-i)` as a raw Q32.32 constant.
fn cordic_atan_raw(i: usize) -> i64 {
    fixed_raw((2f64).powi(-(i as i32)).atan())
}

/// Raw Q32.32 value of `pi`.
fn fixed_pi() -> i64 {
    fixed_raw(std::f64::consts::PI)
}

/// Raw Q32.32 value of `pi / 2`.
fn fixed_pi_over_2() -> i64 {
    fixed_raw(std::f64::consts::FRAC_PI_2)
}

/// Raw Q32.32 value of `2 / pi`.
fn fixed_two_over_pi() -> i64 {
    fixed_raw(std::f64::consts::FRAC_2_PI)
}

/// Raw Q32.32 inverse CORDIC gain `prod 1/sqrt(1 + 2^-2i)` over the iteration
/// count, i.e. the starting `x` for rotation mode so the result is unscaled.
fn cordic_gain_inverse() -> i64 {
    let mut gain = 1.0f64;
    for i in 0..CORDIC_ITERATIONS {
        gain *= (1.0 + (2f64).powi(-2 * i as i32)).sqrt();
    }
    fixed_raw(1.0 / gain)
}

/// Raw Q32.32 value of `ln(2)`.
fn fixed_ln2() -> i64 {
    fixed_raw(std::f64::consts::LN_2)
}

/// Raw Q32.32 value of `1 / ln(2)`.
fn fixed_inv_ln2() -> i64 {
    fixed_raw(1.0 / std::f64::consts::LN_2)
}

/// Raw Q32.32 value of `1 / ln(10)`.
fn fixed_inv_ln10() -> i64 {
    fixed_raw(1.0 / std::f64::consts::LN_10)
}
