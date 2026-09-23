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

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::target::shared::abi;
/// Q32.32 scale factor (`2^32`) as a raw `Fixed` of value `1.0`.
const FIXED_ONE: u64 = 1u64 << 32;
/// Half of one Q32.32 unit (`0.5`), used as a round-to-nearest bias.
const FIXED_HALF: u64 = 1u64 << 31;
/// Mask selecting the fractional 32 bits of a Q32.32 value.
const FIXED_FRACTION_MASK: u64 = 0xFFFF_FFFF;
/// Raw Q32.32 value of `64.0`: the magnitude past which [`CodeBuilder::emit_fixed_exp`]
/// decides its result from the SIGN of the argument instead of running the
/// `x / ln2` argument reduction (bug-659). `Fixed` spans just under `[-2^31, 2^31)`,
/// so `exp` overflows above `ln(2^31) = 21.4876` and rounds to zero below
/// `-33*ln2 = -22.8742`; `64` is outside both, so no in-range result moves, and
/// well inside the reduction's wrap point `2^31*ln2 = 1.4885e9`, so the unchecked
/// Q32.32 product downstream can no longer leave range.
const FIXED_EXP_ARGUMENT_BOUND: u64 = 64u64 << 32;
/// `2^63`: the value `1.0` in the unsigned Q1.63 format of the trig series.
const Q63_ONE: u64 = 1u64 << 63;
/// Horner levels of the `sin`/`cos` Taylor series (terms through `r^18`); see
/// [`CodeBuilder::emit_fixed_sin_cos_magnitudes`].
const FIXED_TRIG_TERMS: u64 = 9;
/// `floor(2/pi · 2^64)`, the Q0.64 multiplier that estimates the quadrant count
/// (bug-615). Baked, not computed from host `f64`; pinned by
/// `trig_constants_match_machin_pi`.
const TWO_OVER_PI_Q64: u64 = 0xA2F9_836E_4E44_1529;
/// `round(pi/2 · 2^159)` as big-endian 64-bit words (the top word holds 32 bits):
/// the multi-word pi/2 the trig reduction subtracts `k` times (bug-615). Pinned by
/// `trig_constants_match_machin_pi`.
const PI_OVER_2_Q159: [u64; 3] = [0xC90F_DAA2, 0x2168_C234_C4C6_628B, 0x80DC_1CD1_2902_4E09];
/// Horner levels of the `atan` Taylor series (terms through `u^55`); see
/// [`CodeBuilder::emit_fixed_atan_ratio`].
const FIXED_ATAN_TERMS: u64 = 27;

/// `round(pi/2 · 2^(159−shift))` read out of the baked 160-bit
/// [`PI_OVER_2_Q159`] (bug-615). Defined for `97 <= shift <= 127`, the window
/// where the result is a 64-bit integer. The addend is the top discarded bit, so
/// this is round-half-up on the exact 160-bit value — it never reads a host
/// `f64`, which is what bug-137.1 forbade. Every value below is pinned by
/// `trig_constants_match_machin_pi`.
const fn pi_over_2_scaled(shift: u32) -> u64 {
    (PI_OVER_2_Q159[0] << (128 - shift))
        + (PI_OVER_2_Q159[1] >> (shift - 64))
        + ((PI_OVER_2_Q159[1] >> (shift - 65)) & 1)
}

/// `round(pi · 2^61)`: the Q3.61 half-turn the `atan2` quadrant fold adds.
const PI_Q61: u64 = pi_over_2_scaled(97);
/// `round(pi/2 · 2^61)`: the Q3.61 quarter-turn the `|t| > 1`, `asin` and
/// `acos` folds add.
const PI_OVER_2_Q61: u64 = pi_over_2_scaled(98);
/// `round(pi/4 · 2^61)`: the Q3.61 eighth-turn the `t > 1/2` fold adds.
const PI_OVER_4_Q61: u64 = pi_over_2_scaled(99);
/// `round(pi · 2^32)`: the exact `Fixed` value of `atan2(0, x)` for `x < 0`.
const PI_FIXED: i64 = pi_over_2_scaled(126) as i64;
/// `round(pi/2 · 2^32)`: the exact `Fixed` value of `atan2(y, 0)` for `y > 0`.
const PI_OVER_2_FIXED: i64 = pi_over_2_scaled(127) as i64;

/// A `Fixed` angle `x` reduced by [`CodeBuilder::emit_fixed_trig_reduce`] to
/// `|x| = k·(pi/2) + r`, `|r| <= pi/4 + 2^-31`. Every field is a register.
struct ReducedAngle {
    /// 1 when `x < 0`, else 0.
    negative_x: VirtualRegister,
    /// `k mod 4`.
    quadrant: VirtualRegister,
    /// 1 when `r < 0`, else 0.
    negative_r: VirtualRegister,
    /// High word of the 128-bit `|r|·2^127`.
    r_high: VirtualRegister,
    /// Low word of the 128-bit `|r|·2^127`.
    r_low: VirtualRegister,
}

impl CodeBuilder<'_> {
    /// Lower `floor`/`ceil`/`round` for a `Fixed` argument to an `Integer`
    /// result using raw Q32.32 arithmetic. The rounded integer always fits in
    /// `Integer` range, so no overflow check is required.
    pub(crate) fn emit_fixed_rounding_to_integer(
        &mut self,
        function: &str,
        src: impl Into<Operand>,
        dst: impl Into<Operand>,
    ) -> Result<(), String> {
        let src = src.into();
        let dst = dst.into();
        match function {
            "floor" => {
                // Arithmetic shift right rounds toward negative infinity.
                self.emit(abi::arithmetic_shift_right_immediate(
                    dst.clone(),
                    src.clone(),
                    32,
                ));
            }
            "ceil" => {
                let frac = self.allocate_register();
                let mask = self.allocate_register();
                let done = self.label("fixed_ceil_done");
                self.emit(abi::arithmetic_shift_right_immediate(
                    dst.clone(),
                    src.clone(),
                    32,
                ));
                self.emit(abi::move_immediate(
                    &mask,
                    "Integer",
                    &FIXED_FRACTION_MASK.to_string(),
                ));
                self.emit(abi::and_registers(&frac, src.clone(), &mask));
                self.emit(abi::compare_immediate(&frac, "0"));
                self.emit(abi::branch_eq(&done));
                self.emit(abi::add_immediate(dst.clone(), dst.clone(), 1));
                self.emit(abi::label(&done));
            }
            "round" => {
                let whole = self.allocate_register();
                let frac = self.allocate_register();
                let mask = self.allocate_register();
                let threshold = self.allocate_register();
                let negative = self.label("fixed_round_negative");
                let compare = self.label("fixed_round_compare");
                let done = self.label("fixed_round_done");
                self.emit(abi::arithmetic_shift_right_immediate(
                    &whole,
                    src.clone(),
                    32,
                ));
                self.emit(abi::move_immediate(
                    &mask,
                    "Integer",
                    &FIXED_FRACTION_MASK.to_string(),
                ));
                self.emit(abi::and_registers(&frac, src.clone(), &mask));
                // Ties round away from zero: for negative inputs the fractional
                // part must strictly exceed 0.5 to round toward zero, so use a
                // threshold of 0.5 (>=) for non-negative and 0.5+1 (>=) for
                // negative values.
                self.emit(abi::compare_immediate(src.clone(), "0"));
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
                self.emit(abi::move_register(dst.clone(), &whole));
                self.emit(abi::compare_registers(&frac, &threshold));
                self.emit(abi::branch_lo(&done));
                self.emit(abi::add_immediate(dst.clone(), dst.clone(), 1));
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
    pub(crate) fn emit_fixed_sqrt(
        &mut self,
        src: impl Into<Operand>,
    ) -> Result<VirtualRegister, String> {
        // The algorithm needs a handful of working registers. Spill the input
        // to the stack and reset the temporary register file so the surrounding
        // expression's prior allocations do not exhaust the pool, mirroring the
        // external-call lowering pattern.
        let slot = self.allocate_stack_object("fixed_sqrt_input", 8);
        self.emit(abi::store_u64(src, abi::stack_pointer(), slot));
        self.reset_temporary_registers();
        let input = self.allocate_register();
        self.emit(abi::load_u64(&input, abi::stack_pointer(), slot));
        let src = &input;
        let dst = self.allocate_register();
        let nhi = self.allocate_register();
        let nlo = self.allocate_register();
        let res = self.allocate_register();
        let rem = self.allocate_register();
        let digit = self.allocate_register();
        let trial = self.allocate_register();
        let counter = self.allocate_register();
        let carry = self.allocate_register();
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
    fn emit_const_i64(&mut self, reg: impl Into<Operand>, value: i64) {
        self.emit(abi::move_immediate(
            reg,
            "Integer",
            &(value as u64).to_string(),
        ));
    }

    /// Move an unsigned 64-bit constant into `reg`.
    fn emit_const_u64(&mut self, reg: impl Into<Operand>, value: u64) {
        self.emit(abi::move_immediate(reg, "Integer", &value.to_string()));
    }

    /// Round-to-nearest Q32.32 multiply `(a * b) / 2^32` into a fresh register.
    /// Intended for internal use where the result is known to stay in range, so
    /// no overflow trap is emitted. Nets a single new register (the result); the
    /// working temporaries are released before returning.
    fn emit_fixed_mul(
        &mut self,
        a: impl Into<Operand>,
        b: impl Into<Operand>,
    ) -> Result<VirtualRegister, String> {
        let result = self.allocate_register();
        let saved = self.next_register;
        let s0 = self.allocate_register();
        let s1 = self.allocate_register();
        self.emit_fixed_mul_inplace(&result, a, b, &s0, &s1);
        self.next_register = saved;
        Ok(result)
    }

    /// Round-to-nearest Q32.32 multiply writing `(a * b) / 2^32` to `dst` using
    /// the two caller-provided scratch registers `s0`/`s1`. Allocation-free, so
    /// it is safe to call inside register-tight runtime loops. `dst` may alias
    /// `a` or `b`; the inputs are fully consumed before `dst` is written.
    fn emit_fixed_mul_inplace(
        &mut self,
        dst: impl Into<Operand>,
        a: impl Into<Operand>,
        b: impl Into<Operand>,
        s0: impl Into<Operand>,
        s1: impl Into<Operand>,
    ) {
        let dst = dst.into();
        let a = a.into();
        let b = b.into();
        let s0 = s0.into();
        let s1 = s1.into();
        self.emit(abi::multiply_registers(s0.clone(), a.clone(), b.clone())); // low 64 bits
        self.emit(abi::signed_multiply_high_registers(
            s1.clone(),
            a.clone(),
            b.clone(),
        )); // high 64 bits
            // Combined middle word = (s1 << 32) | (s0 >>u 32) = bits[95:32].
        self.emit(abi::shift_left_immediate(s1.clone(), s1.clone(), 32));
        self.emit(abi::shift_right_immediate(dst.clone(), s0.clone(), 32));
        self.emit(abi::or_registers(dst.clone(), dst.clone(), s1.clone()));
        // Round half up using bit 31 of the low word (the top discarded bit).
        self.emit(abi::shift_right_immediate(s0.clone(), s0.clone(), 31));
        self.emit(abi::shift_left_immediate(s0.clone(), s0.clone(), 63));
        self.emit(abi::shift_right_immediate(s0.clone(), s0.clone(), 63));
        self.emit(abi::add_registers(dst.clone(), dst.clone(), s0.clone()));
    }

    /// Round-to-nearest Q32.32 multiply that SATURATES rather than wraps: a
    /// product outside `Fixed` range yields `i64::MAX`/`i64::MIN` carrying the
    /// product's true sign (bug-659). In range it is bit-identical to
    /// [`Self::emit_fixed_mul`].
    ///
    /// Use this wherever the product's SIGN, not just its value, steers a later
    /// branch. `emit_fixed_mul` wraps, and a wrapped sign does not merely
    /// misreport a magnitude — it sends the consumer down the opposite arm. That
    /// is how `pow`'s `exponent * ln(base)` turned a genuine overflow into `0.00`
    /// and a genuine underflow into `ErrOverflow`. Saturating keeps the sign, so
    /// `emit_fixed_exp`'s range gate still reads the correct outcome off it.
    ///
    /// `a` and `b` must not be the returned register (it is freshly allocated, so
    /// only a caller re-using the result as an input could violate that).
    fn emit_fixed_mul_saturating(
        &mut self,
        a: impl Into<Operand>,
        b: impl Into<Operand>,
    ) -> Result<VirtualRegister, String> {
        let a = a.into();
        let b = b.into();
        let result = self.allocate_register();
        let saved = self.next_register;
        let lo = self.allocate_register();
        let hi = self.allocate_register();
        let probe = self.allocate_register();
        let in_range = self.label("fixed_mul_sat_in_range");
        let done = self.label("fixed_mul_sat_done");
        // Low and high halves of the 128-bit signed product.
        self.emit(abi::multiply_registers(&lo, a.clone(), b.clone()));
        self.emit(abi::signed_multiply_high_registers(&hi, a, b));
        // The Q32.32 result is bits[95:32] of that product, so it fits a signed
        // 64-bit word exactly when bits[127:95] are all sign bits — that is, when
        // `hi >> 31` (arithmetic; == product >> 95) is 0 or -1. Biasing by one maps
        // that pair to {1, 0}, which a single unsigned `<= 1` test covers.
        self.emit(abi::arithmetic_shift_right_immediate(&probe, &hi, 31));
        self.emit(abi::add_immediate(&probe, &probe, 1));
        self.emit(abi::compare_immediate(&probe, "1"));
        self.emit(abi::branch_ls(&in_range));
        // Out of range: saturate toward the product's true sign. `result` is 0 for
        // a non-negative product and all-ones for a negative one, so XOR with
        // `i64::MAX` gives `i64::MAX` and `i64::MIN` respectively.
        self.emit(abi::arithmetic_shift_right_immediate(&result, &hi, 63));
        self.emit(abi::move_immediate(
            &probe,
            "Integer",
            &(i64::MAX as u64).to_string(),
        ));
        self.emit(abi::exclusive_or_registers(&result, &result, &probe));
        self.emit(abi::branch(&done));
        self.emit(abi::label(&in_range));
        // Combined middle word = (hi << 32) | (lo >>u 32) = bits[95:32].
        self.emit(abi::shift_left_immediate(&hi, &hi, 32));
        self.emit(abi::shift_right_immediate(&result, &lo, 32));
        self.emit(abi::or_registers(&result, &result, &hi));
        // Round half up using bit 31 of the low word (the top discarded bit).
        self.emit(abi::shift_right_immediate(&lo, &lo, 31));
        self.emit(abi::shift_left_immediate(&lo, &lo, 63));
        self.emit(abi::shift_right_immediate(&lo, &lo, 63));
        // That carry is the one way an in-range combine can still wrap: rounding
        // `i64::MAX` up yields `i64::MIN`, flipping the sign this routine exists to
        // preserve. `i64::MAX` is already the saturated answer, so hold it there.
        self.emit(abi::move_immediate(
            &probe,
            "Integer",
            &(i64::MAX as u64).to_string(),
        ));
        self.emit(abi::compare_registers(&result, &probe));
        self.emit(abi::branch_eq(&done));
        self.emit(abi::add_registers(&result, &result, &lo));
        self.emit(abi::label(&done));
        self.next_register = saved;
        Ok(result)
    }

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
        num_src: impl Into<Operand>,
        den_src: impl Into<Operand>,
    ) -> VirtualRegister {
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
        dst: impl Into<Operand>,
        angle: impl Into<Operand>,
        negative: impl Into<Operand>,
    ) {
        let dst = dst.into();
        let mask = self.allocate_register();
        self.emit_const_u64(&mask, 1u64 << 28);
        self.emit(abi::add_registers(dst.clone(), angle, &mask));
        self.emit(abi::shift_right_immediate(dst.clone(), dst.clone(), 29));
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
        y_src: impl Into<Operand>,
        x_src: impl Into<Operand>,
    ) -> Result<VirtualRegister, String> {
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
    fn emit_fixed_trig_reduce(&mut self, src: impl Into<Operand>) -> ReducedAngle {
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
        r_high: impl Into<Operand>,
        r_low: impl Into<Operand>,
    ) -> (VirtualRegister, VirtualRegister) {
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
        dst: impl Into<Operand>,
        m: impl Into<Operand>,
        negative: impl Into<Operand>,
    ) {
        let dst = dst.into();
        let mask = self.allocate_register();
        self.emit_const_u64(&mask, 1u64 << 30);
        self.emit(abi::add_registers(dst.clone(), m, &mask));
        self.emit(abi::shift_right_immediate(dst.clone(), dst.clone(), 31));
        self.emit_conditional_negate(dst, negative, &mask);
    }

    /// `dst = negative ? −dst : dst` for a 0/1 `negative`, branch-free:
    /// `(dst ^ −negative) − (−negative)`. `mask` is scratch.
    fn emit_conditional_negate(
        &mut self,
        dst: impl Into<Operand>,
        negative: impl Into<Operand>,
        mask: impl Into<Operand>,
    ) {
        let dst = dst.into();
        let mask = mask.into();
        self.emit(abi::subtract_registers(mask.clone(), abi::ZERO, negative));
        self.emit(abi::exclusive_or_registers(
            dst.clone(),
            dst.clone(),
            mask.clone(),
        ));
        self.emit(abi::subtract_registers(dst.clone(), dst, mask));
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
        src: impl Into<Operand>,
        want_cos: bool,
    ) -> Result<VirtualRegister, String> {
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
        src: impl Into<Operand>,
    ) -> Result<VirtualRegister, String> {
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
        src: impl Into<Operand>,
        is_acos: bool,
    ) -> Result<VirtualRegister, String> {
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

    /// Lower `exp` for a `Fixed` argument. Computes `2^n * exp(r)` with
    /// `n = round(x / ln2)` and `r = x - n*ln2`, evaluating `exp(r)` by a Taylor
    /// series. Overflow beyond `Fixed` range fails with `ErrOverflow`.
    pub(crate) fn emit_fixed_exp(
        &mut self,
        src: impl Into<Operand>,
    ) -> Result<VirtualRegister, String> {
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
        value: impl Into<Operand>,
        n: impl Into<Operand>,
    ) -> Result<(), String> {
        let value = value.into();
        let n = n.into();
        let count = self.allocate_register();
        let limit = self.allocate_register();
        let negative = self.label("fixed_scale_negative");
        let up_loop = self.label("fixed_scale_up");
        let up_done = self.label("fixed_scale_up_done");
        let down_loop = self.label("fixed_scale_down");
        let down_done = self.label("fixed_scale_down_done");
        let no_overflow = self.label("fixed_scale_no_overflow");
        self.emit(abi::compare_immediate(n.clone(), "0"));
        self.emit(abi::branch_lt(&negative));
        // n >= 0: double `count` times, checking for overflow before each shift.
        self.emit(abi::move_register(&count, n.clone()));
        // limit = i64::MAX / 2; if value > limit a doubling would overflow.
        self.emit(abi::move_immediate(
            &limit,
            "Integer",
            &(i64::MAX as u64 / 2).to_string(),
        ));
        self.emit(abi::label(&up_loop));
        self.emit(abi::compare_immediate(&count, "0"));
        self.emit(abi::branch_eq(&up_done));
        self.emit(abi::compare_registers(value.clone(), &limit));
        self.emit(abi::branch_le(&no_overflow));
        self.raise_error_bare("ErrOverflow")?;
        self.emit(abi::label(&no_overflow));
        self.emit(abi::shift_left_immediate(value.clone(), value.clone(), 1));
        self.emit(abi::subtract_immediate(&count, &count, 1));
        self.emit(abi::branch(&up_loop));
        self.emit(abi::label(&up_done));
        let finish = self.label("fixed_scale_finish");
        self.emit(abi::branch(&finish));
        // n < 0: halve `-n` times (arithmetic shift; value is non-negative).
        self.emit(abi::label(&negative));
        self.emit(abi::subtract_registers(&count, abi::ZERO, n.clone()));
        self.emit(abi::label(&down_loop));
        self.emit(abi::compare_immediate(&count, "0"));
        self.emit(abi::branch_eq(&down_done));
        self.emit(abi::arithmetic_shift_right_immediate(
            value.clone(),
            value.clone(),
            1,
        ));
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
        src: impl Into<Operand>,
        base10: bool,
    ) -> Result<VirtualRegister, String> {
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
    pub(crate) fn emit_fixed_pow_general(
        &mut self,
        base: impl Into<Operand>,
        exponent: impl Into<Operand>,
    ) -> Result<VirtualRegister, String> {
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

/// Raw Q32.32 representation of a real value.
fn fixed_raw(value: f64) -> i64 {
    (value * 4_294_967_296.0).round() as i64
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

#[cfg(test)]
mod tests {
    use super::{
        pi_over_2_scaled, FIXED_ATAN_TERMS, PI_FIXED, PI_OVER_2_FIXED, PI_OVER_2_Q159,
        PI_OVER_2_Q61, PI_OVER_4_Q61, PI_Q61, TWO_OVER_PI_Q64,
    };
    use num_bigint::BigInt;

    /// `atan(1/n)` scaled by `2^bits`.
    fn atan_inverse(n: i64, bits: u32) -> BigInt {
        let n2 = BigInt::from(n * n);
        let mut power = (BigInt::from(1) << bits) / n;
        let mut sum = BigInt::from(0);
        let mut k = 0i64;
        while power != BigInt::from(0) {
            let term = &power / (2 * k + 1);
            if k % 2 == 0 {
                sum += term;
            } else {
                sum -= term;
            }
            power /= &n2;
            k += 1;
        }
        sum
    }

    /// The trig reduction's baked constants are pi from Machin's formula
    /// (`pi = 16·atan(1/5) − 4·atan(1/239)`), never a host `f64` (bug-615).
    #[test]
    fn trig_constants_match_machin_pi() {
        const GUARD: u32 = 320;
        let pi =
            BigInt::from(16) * atan_inverse(5, GUARD) - BigInt::from(4) * atan_inverse(239, GUARD);
        // round(pi/2 · 2^159) = round(pi · 2^158).
        let half = BigInt::from(1) << (GUARD - 158 - 1);
        let pi_over_2 = (&pi + half) >> (GUARD - 158);
        let words = (BigInt::from(PI_OVER_2_Q159[0]) << 128)
            + (BigInt::from(PI_OVER_2_Q159[1]) << 64)
            + BigInt::from(PI_OVER_2_Q159[2]);
        assert_eq!(words, pi_over_2);
        // floor(2/pi · 2^64).
        let two_over_pi = (BigInt::from(2) << (GUARD + 64)) / &pi;
        assert_eq!(two_over_pi, BigInt::from(TWO_OVER_PI_Q64));
        // Every scaled turn the inverse-trig assembly adds is the same 160-bit
        // pi/2 rounded, so no second, disagreeing pi enters the kernel
        // (bug-615-C). round(pi/2 · 2^(159−shift)) = round(pi · 2^(158−shift)).
        for (shift, value) in [
            (97u32, PI_Q61),
            (98, PI_OVER_2_Q61),
            (99, PI_OVER_4_Q61),
            (126, PI_FIXED as u64),
            (127, PI_OVER_2_FIXED as u64),
        ] {
            let half = BigInt::from(1) << (GUARD - (158 - shift) - 1);
            let expected = (&pi + half) >> (GUARD - (158 - shift));
            assert_eq!(
                expected,
                BigInt::from(value),
                "pi_over_2_scaled({shift}) must round the baked pi/2"
            );
            assert_eq!(value, pi_over_2_scaled(shift));
        }
        // The `atan` series must reach 2^-60 over its reduced range u <= 1/2:
        // the first omitted term is u^(2K+3)/(2K+3) <= 2^-(2K+3)/(2K+3).
        let terms = u32::try_from(FIXED_ATAN_TERMS).unwrap();
        assert!(
            BigInt::from(2 * terms + 3) << (2 * terms + 3) > BigInt::from(1) << 60,
            "FIXED_ATAN_TERMS leaves more than 2^-60 of Taylor remainder"
        );
    }
}
