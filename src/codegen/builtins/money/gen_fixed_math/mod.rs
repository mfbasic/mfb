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
use crate::target::shared::abi::Reg;

mod exp_log;
mod trig;
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
        src: impl Into<Reg>,
        dst: impl Into<Reg>,
    ) -> Result<(), String> {
        let (src, dst): (Reg, Reg) = (src.into(), dst.into());
        match function {
            "floor" => {
                // Arithmetic shift right rounds toward negative infinity.
                self.emit(abi::arithmetic_shift_right_immediate(dst, src, 32));
            }
            "ceil" => {
                let frac = self.allocate_register();
                let mask = self.allocate_register();
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
                let whole = self.allocate_register();
                let frac = self.allocate_register();
                let mask = self.allocate_register();
                let threshold = self.allocate_register();
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
    pub(crate) fn emit_fixed_sqrt(
        &mut self,
        src: impl Into<Reg>,
    ) -> Result<VirtualRegister, String> {
        let src: Reg = src.into();
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
    fn emit_const_i64(&mut self, reg: impl Into<Reg>, value: i64) {
        let reg: Reg = reg.into();
        self.emit(abi::move_immediate(
            reg,
            "Integer",
            &(value as u64).to_string(),
        ));
    }

    /// Move an unsigned 64-bit constant into `reg`.
    fn emit_const_u64(&mut self, reg: impl Into<Reg>, value: u64) {
        let reg: Reg = reg.into();
        self.emit(abi::move_immediate(reg, "Integer", &value.to_string()));
    }

    /// Round-to-nearest Q32.32 multiply `(a * b) / 2^32` into a fresh register.
    /// Intended for internal use where the result is known to stay in range, so
    /// no overflow trap is emitted. Nets a single new register (the result); the
    /// working temporaries are released before returning.
    fn emit_fixed_mul(
        &mut self,
        a: impl Into<Reg>,
        b: impl Into<Reg>,
    ) -> Result<VirtualRegister, String> {
        let (a, b): (Reg, Reg) = (a.into(), b.into());
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
        dst: impl Into<Reg>,
        a: impl Into<Reg>,
        b: impl Into<Reg>,
        s0: impl Into<Reg>,
        s1: impl Into<Reg>,
    ) {
        let (dst, a, b, s0, s1): (Reg, Reg, Reg, Reg, Reg) =
            (dst.into(), a.into(), b.into(), s0.into(), s1.into());
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
        a: impl Into<Reg>,
        b: impl Into<Reg>,
    ) -> Result<VirtualRegister, String> {
        let (a, b): (Reg, Reg) = (a.into(), b.into());
        let result = self.allocate_register();
        let saved = self.next_register;
        let lo = self.allocate_register();
        let hi = self.allocate_register();
        let probe = self.allocate_register();
        let in_range = self.label("fixed_mul_sat_in_range");
        let done = self.label("fixed_mul_sat_done");
        // Low and high halves of the 128-bit signed product.
        self.emit(abi::multiply_registers(&lo, a, b));
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
