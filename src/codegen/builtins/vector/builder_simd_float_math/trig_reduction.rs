//! plan-176-T: the trig argument reduction — the Cody-Waite path and the large-argument `rem_pio2`. The consts, the math const pool and the kernel enums live in
//! `builtins/vector/builder_simd_float_math/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// Reduce `x` to `r = r_hi + r_lo` in `[-pi/4, pi/4]` and the quadrant, per lane.
    ///
    /// Every lane first runs the fdlibm medium Cody-Waite step, which also keeps
    /// the low half its last subtraction drops. A lane then takes the exact table
    /// reduction ([`Self::emit_rem_pio2_large_lane`]) instead when that step
    /// cannot be trusted to the last ULP: `|x| >= 2^20 * pi/2` (the step is exact
    /// only below it, and past about `1e100` it produces NaN), or the reduced
    /// angle cancelled below `2^-50 * |x|` (the truncated `pi/2` constants' error
    /// is about `3e-37 * |x|`). A NaN or infinite lane is never re-reduced; its
    /// medium result is NaN and raises `ErrFloatNan` as before. The branch to the
    /// exact path is taken only when some lane needs it, so an ordinary call pays
    /// the routing compare and nothing else.
    ///
    /// Leaves `r_hi` in `v2`, `r*r` in `v1`, the quadrant (an integer whose low two
    /// bits matter) in `v5`, and the low half's first-order corrections in
    /// `trig.sin_corr` (`r_lo * (1 - r^2/2)`, added to `sin(r_hi)`) and
    /// `trig.cos_corr` (`-r_hi * r_lo`, added to `cos(r_hi)`). `v0` (the input) is
    /// consumed; `v3`, `v4`, `v6`, `v7` are scratch. Assumes the persistent trig
    /// constants in v16-v21 and `trig`. `scalar` (both lanes hold the same value)
    /// tests and re-reduces lane 0 only.
    pub(super) fn emit_sincos_reduce(&mut self, k: &KernelRegs, scalar: bool) {
        let t = k.trig();
        let v = abi::VEC_SCRATCH;
        self.emit(abi::vector_fmul(v[1], v[0], &k.v16)); // x*invpio2
        self.emit(abi::vector_fadd(v[1], v[1], &k.v17)); // +0.5
        self.emit(abi::vector_frintm(v[1], v[1])); // q = floor(..)
        self.emit(abi::vector_orr(v[2], v[0], v[0])); // r = x
        self.emit(abi::vector_fmls(v[2], v[1], &k.v18)); // r -= q*PIO2_1
        self.emit(abi::vector_fmul(v[3], v[1], &k.v19)); // w = q*PIO2_2
        self.emit(abi::vector_fsub(v[6], v[2], v[3])); // y0 = r - w
        self.emit(abi::vector_fsub(v[7], v[2], v[6])); // r - y0
        self.emit(abi::vector_fsub(v[7], v[7], v[3])); // t = (r-y0) - w
        self.emit(abi::vector_fneg(v[7], v[7])); // -t
        self.emit(abi::vector_fmla(v[7], v[1], &k.v20)); // -t + q*PIO2_2T
        self.emit(abi::vector_fsub(v[2], v[6], v[7])); // r_hi = y0 - (..)
        self.emit(abi::vector_fcvtzs(v[5], v[1])); // q (int)
        self.emit(abi::vector_and(v[5], v[5], &k.v21)); // quad = q & 3

        // r_lo = (y0 - r_hi) - (q*PIO2_2T - t): exactly what that last subtraction
        // rounded away (bug-618 — dropping it cost the medium range its last ULP).
        self.emit(abi::vector_fsub(&t.sin_corr, v[6], v[2]));
        self.emit(abi::vector_fsub(&t.sin_corr, &t.sin_corr, v[7]));
        // Route mask: (limit <= |x| < inf) | (|r_hi| < |x| * 2^-50). Every compare
        // is false on a NaN lane, and the ratio compare is false on an infinite one
        // (|r_hi| is NaN there).
        self.emit(abi::vector_fabs(v[3], v[0]));
        self.emit(abi::vector_fcmge(&t.route, v[3], &t.limit));
        self.emit(abi::vector_fcmgt(v[4], &t.inf, v[3]));
        self.emit(abi::vector_and(&t.route, &t.route, v[4]));
        self.emit(abi::vector_fmul(v[4], v[3], &t.cancel));
        self.emit(abi::vector_fabs(v[7], v[2]));
        self.emit(abi::vector_fcmgt(v[4], v[4], v[7]));
        self.emit(abi::vector_orr(&t.route, &t.route, v[4]));
        let any = self.temporary_vreg();
        self.emit(abi::vector_extract_to_x(&any, &t.route, 0));
        if !scalar {
            let lane1 = self.temporary_vreg();
            self.emit(abi::vector_extract_to_x(&lane1, &t.route, 1));
            self.emit(abi::or_registers(&any, &any, &lane1));
        }
        let reduced = self.label("simd_trig_reduced");
        self.emit(abi::compare_immediate(&any, "0"));
        self.emit(abi::branch_eq(&reduced));
        if scalar {
            let bits = self.temporary_vreg();
            self.emit(abi::vector_extract_to_x(&bits, v[0], 0));
            let (hi, lo, quad) = self.emit_rem_pio2_large_lane(&bits);
            self.emit(abi::vector_dup_from_x(v[2], &hi));
            self.emit(abi::vector_dup_from_x(&t.sin_corr, &lo));
            self.emit(abi::vector_dup_from_x(v[5], &quad));
        } else {
            self.emit_rem_pio2_large_lanes(k);
        }
        self.emit(abi::label(&reduced));
        // First-order effect of r_lo: sin(r_hi + r_lo) = sin(r_hi) + r_lo*cos(r_hi)
        // and cos(r_hi + r_lo) = cos(r_hi) - r_lo*sin(r_hi), with cos(r_hi) taken as
        // 1 - r^2/2 and sin(r_hi) as r_hi. The dropped terms are below 2^-60 of the
        // result, and r_lo is itself at most half an ULP of r_hi.
        self.emit(abi::vector_fmul(v[1], v[2], v[2])); // r2 (the Horner variable)
        self.emit(abi::vector_orr(v[3], &t.one, &t.one));
        self.emit(abi::vector_fmls(v[3], v[1], &k.v17)); // 1 - r2*0.5
        self.emit(abi::vector_fmul(&t.cos_corr, v[2], &t.sin_corr));
        self.emit(abi::vector_fneg(&t.cos_corr, &t.cos_corr)); // -r_hi*r_lo
        self.emit(abi::vector_fmul(&t.sin_corr, &t.sin_corr, v[3])); // r_lo*(1 - r2/2)
    }

    /// The exact reduction for both lanes of an array body: each routed lane runs
    /// [`Self::emit_rem_pio2_large_lane`] (one copy of the code, looped over the two
    /// lanes through a stack block), and the results replace `r_hi` (`v2`), `r_lo`
    /// (`trig.sin_corr`) and the quadrant (`v5`) in the routed lanes only. An
    /// unrouted lane's result words are never written and never selected.
    fn emit_rem_pio2_large_lanes(&mut self, k: &KernelRegs) {
        let t = k.trig();
        let v = abi::VEC_SCRATCH;
        // Block layout, 16 bytes a field, lane 0 then lane 1:
        // [x, route, r_hi, r_lo, quadrant].
        if !self.stack_size.is_multiple_of(16) {
            self.allocate_stack_object("simd_trig_pad", 16 - self.stack_size % 16);
        }
        let block = self.allocate_stack_object("simd_trig_lanes", 80);
        self.emit(abi::vector_store(v[0], abi::stack_pointer(), block));
        self.emit(abi::vector_store(
            &t.route,
            abi::stack_pointer(),
            block + 16,
        ));
        let lane = self.temporary_vreg();
        self.emit(abi::add_immediate(&lane, abi::stack_pointer(), block));
        let remaining = self.temporary_vreg();
        self.emit(abi::move_immediate(&remaining, "Integer", "2"));
        let top = self.label("simd_trig_lane");
        let next = self.label("simd_trig_lane_next");
        self.emit(abi::label(&top));
        let routed = self.temporary_vreg();
        self.emit(abi::load_u64(&routed, &lane, 16));
        self.emit(abi::compare_immediate(&routed, "0"));
        self.emit(abi::branch_eq(&next));
        let bits = self.temporary_vreg();
        self.emit(abi::load_u64(&bits, &lane, 0));
        let (hi, lo, quad) = self.emit_rem_pio2_large_lane(&bits);
        self.emit(abi::store_u64(&hi, &lane, 32));
        self.emit(abi::store_u64(&lo, &lane, 48));
        self.emit(abi::store_u64(&quad, &lane, 64));
        self.emit(abi::label(&next));
        self.emit(abi::add_immediate(&lane, &lane, 8));
        self.emit(abi::subtract_immediate(&remaining, &remaining, 1));
        self.emit(abi::compare_immediate(&remaining, "0"));
        self.emit(abi::branch_ne(&top));
        self.emit(abi::vector_load(v[3], abi::stack_pointer(), block + 32));
        self.emit(abi::vector_bit(v[2], v[3], &t.route));
        self.emit(abi::vector_load(v[3], abi::stack_pointer(), block + 48));
        self.emit(abi::vector_bit(&t.sin_corr, v[3], &t.route));
        self.emit(abi::vector_load(v[3], abi::stack_pointer(), block + 64));
        self.emit(abi::vector_bit(v[5], v[3], &t.route));
    }

    /// Exact reduction of one finite lane (Payne-Hanek with a fixed-width window).
    /// `bits` holds the lane's `f64` bit pattern; returns GPRs holding the bit
    /// patterns of `r_hi` and `r_lo` and the quadrant (low two bits significant).
    ///
    /// Write `|x| = m * 2^e` with `m` the 53-bit integer mantissa. Bits of `2/pi`
    /// worth `2^-i` with `i <= e - 2` add multiples of 4 to `x * 2/pi`, so they
    /// cannot move the quadrant or the fraction; the window starts at `2/pi`'s bit
    /// `e - 1`. With the table's 55-bit pad that bit is table bit
    /// `s = biased_exponent - 1022`, which lies in word `j = s >> 6` at offset
    /// `s & 63`. `P = m * T[j..j+4] mod 2^256` (three 128-bit products and a carry
    /// chain), shifted left by `s & 63`, puts the integer part mod 4 in the top two
    /// bits and at least 190 fraction bits below them. The bits of `2/pi` past the
    /// window contribute less than `m * 2^((s & 63) + 2 - 256) < 2^-138`.
    ///
    /// No finite double lies within `2^-61` of a multiple of `pi/2` in these
    /// units (the closest, `6381956970095103 * 2^797`, has a fraction of about
    /// `2^-61.5`), so rounding the fraction to the nearest integer — the top
    /// fraction bit picks the quadrant and the sign, and a one's complement gives
    /// the magnitude to within `2^-190` — leaves at least 76 significant bits.
    /// Three 53-bit pieces of that magnitude convert to doubles exactly and sum to
    /// a double-double, which the double-double `pi/2` scales to radians; the
    /// result is within about `2^-100` relative of the true reduced angle.
    ///
    /// Only reached for a lane `emit_sincos_reduce` routed, which is finite and
    /// at least `pi/4` in magnitude (a smaller one has quotient 0 and cannot
    /// cancel), so `s` is in `[0, 1024]` and the four words `j..j+3` are in the
    /// table.
    fn emit_rem_pio2_large_lane(
        &mut self,
        bits: &VirtualRegister,
    ) -> (VirtualRegister, VirtualRegister, VirtualRegister) {
        let pool = self.math_pool_base_reg();
        // s = biased exponent - (1077 - pad); window words T[j..j+4].
        let shift = self.temporary_vreg();
        self.emit(abi::shift_left_immediate(&shift, bits, 1));
        self.emit(abi::shift_right_immediate(&shift, &shift, 53));
        self.emit(abi::subtract_immediate(
            &shift,
            &shift,
            1077 - TWO_OVER_PI_PAD,
        ));
        let window = self.temporary_vreg();
        self.emit(abi::shift_right_immediate(&window, &shift, 6));
        self.emit(abi::shift_left_immediate(&window, &window, 4)); // j * 16
        self.emit(abi::add_registers(&window, &window, &pool));
        let base = two_over_pi_table_offset();
        let mut words = Vec::new();
        for i in 0..4 {
            let word = self.temporary_vreg();
            self.emit(abi::load_u64(&word, &window, base + 16 * i));
            words.push(word);
        }
        let (w3, w2, w1, w0) = (&words[0], &words[1], &words[2], &words[3]);
        // m = mantissa | 2^52.
        let mant = self.temporary_vreg();
        self.emit(abi::shift_left_immediate(&mant, bits, 12));
        self.emit(abi::shift_right_immediate(&mant, &mant, 12));
        let unit = self.temporary_vreg();
        self.emit(abi::move_immediate(
            &unit,
            "Integer",
            &(1u64 << 52).to_string(),
        ));
        self.emit(abi::or_registers(&mant, &mant, &unit));
        // P = m * (w3:w2:w1:w0) mod 2^256 = p3:p2:p1:p0.
        let p0 = self.temporary_vreg();
        let h0 = self.temporary_vreg();
        let p1 = self.temporary_vreg();
        let h1 = self.temporary_vreg();
        let p2 = self.temporary_vreg();
        let h2 = self.temporary_vreg();
        let p3 = self.temporary_vreg();
        self.emit(abi::multiply_registers(&p0, &mant, w0));
        self.emit(abi::unsigned_multiply_high_registers(&h0, &mant, w0));
        self.emit(abi::multiply_registers(&p1, &mant, w1));
        self.emit(abi::unsigned_multiply_high_registers(&h1, &mant, w1));
        self.emit(abi::multiply_registers(&p2, &mant, w2));
        self.emit(abi::unsigned_multiply_high_registers(&h2, &mant, w2));
        self.emit(abi::multiply_registers(&p3, &mant, w3));
        let carry1 = self.temporary_vreg();
        let carry2 = self.temporary_vreg();
        self.emit(abi::add_carry(&p1, &carry1, &h0, &p1, abi::ZERO));
        self.emit(abi::add_carry(&p2, &carry2, &h1, &p2, &carry1));
        self.emit(abi::add_carry(&p3, abi::ZERO, &h2, &p3, &carry2));
        // y3:y2:y1 = P << (s & 63). Variable shifts use the amount's low six bits
        // on every backend; `(w >> 1) >> (63 - b)` is `w >> (64 - b)` without the
        // undefined shift by 64 at b = 0 (`!s & 63 == 63 - (s & 63)`).
        let back = self.temporary_vreg();
        self.emit(abi::bitwise_not(&back, &shift));
        let y3 = self.emit_shl_pair(&p3, &p2, &shift, &back);
        let y2 = self.emit_shl_pair(&p2, &p1, &shift, &back);
        let y1 = self.emit_shl_pair(&p1, &p0, &shift, &back);
        // Quadrant = round(integer.fraction) mod 4 = (y3 + 2^61) >> 62.
        let quad = self.temporary_vreg();
        self.emit(abi::move_immediate(
            &quad,
            "Integer",
            &(1u64 << 61).to_string(),
        ));
        self.emit(abi::add_registers(&quad, &quad, &y3));
        self.emit(abi::shift_right_immediate(&quad, &quad, 62));
        // Rounded up (fraction >= 1/2): the reduced angle is negative and its
        // magnitude is the one's complement of the fraction.
        let negative = self.temporary_vreg();
        self.emit(abi::shift_left_immediate(&negative, &y3, 2));
        self.emit(abi::arithmetic_shift_right_immediate(
            &negative, &negative, 63,
        ));
        self.emit(abi::exclusive_or_registers(&y3, &y3, &negative));
        self.emit(abi::exclusive_or_registers(&y2, &y2, &negative));
        self.emit(abi::exclusive_or_registers(&y1, &y1, &negative));
        // Three 53-bit pieces of the magnitude (y3 bit 61 is 2^-1, and is 0 here):
        // y3[60..8] * 2^-54, y3[7..0]:y2[63..19] * 2^-107, y2[18..0]:y1[63..30] * 2^-160.
        let piece_a = self.temporary_vreg();
        self.emit(abi::shift_left_immediate(&piece_a, &y3, 3));
        self.emit(abi::shift_right_immediate(&piece_a, &piece_a, 11));
        let piece_b = self.emit_join_piece(&y3, 56, &y2, 19);
        let piece_c = self.emit_join_piece(&y2, 45, &y1, 30);
        // Double-double u = a + b + c (units of pi/2), then r = u * pi/2.
        let fa = self.emit_scaled_piece(&piece_a, TWO_POW_M54);
        let fb = self.emit_scaled_piece(&piece_b, TWO_POW_M107);
        let fc = self.emit_scaled_piece(&piece_c, TWO_POW_M160);
        let sum = self.temporary_fp_vreg();
        let err = self.temporary_fp_vreg();
        let tmp1 = self.temporary_fp_vreg();
        let tmp2 = self.temporary_fp_vreg();
        self.emit(abi::float_add_d(&sum, &fa, &fb)); // two-sum(a, b)
        self.emit(abi::float_subtract_d(&tmp1, &sum, &fa));
        self.emit(abi::float_subtract_d(&tmp2, &sum, &tmp1));
        self.emit(abi::float_subtract_d(&tmp2, &fa, &tmp2));
        self.emit(abi::float_subtract_d(&tmp1, &fb, &tmp1));
        self.emit(abi::float_add_d(&err, &tmp2, &tmp1));
        self.emit(abi::float_add_d(&err, &err, &fc));
        let u_hi = self.temporary_fp_vreg();
        let u_lo = self.temporary_fp_vreg();
        self.emit(abi::float_add_d(&u_hi, &sum, &err)); // fast two-sum renormalize
        self.emit(abi::float_subtract_d(&u_lo, &u_hi, &sum));
        self.emit(abi::float_subtract_d(&u_lo, &err, &u_lo));
        let pio2_hi: Reg = self.temporary_fp_vreg().into();
        let pio2_lo: Reg = self.temporary_fp_vreg().into();
        self.emit_load_pool_double(&pio2_hi, std::f64::consts::FRAC_PI_2);
        self.emit_load_pool_double(&pio2_lo, PIO2_DD_LO);
        let prod = self.temporary_fp_vreg();
        let prod_err = self.temporary_fp_vreg();
        self.emit(abi::float_multiply_d(&prod, &u_hi, &pio2_hi));
        self.emit(abi::float_negate_d(&prod_err, &prod));
        self.emit(abi::float_multiply_add_d(
            &prod_err, &prod_err, &u_hi, &pio2_hi,
        ));
        self.emit(abi::float_multiply_add_d(
            &prod_err, &prod_err, &u_hi, &pio2_lo,
        ));
        self.emit(abi::float_multiply_add_d(
            &prod_err, &prod_err, &u_lo, &pio2_hi,
        ));
        let r_hi = self.temporary_fp_vreg();
        let r_lo = self.temporary_fp_vreg();
        self.emit(abi::float_add_d(&r_hi, &prod, &prod_err));
        self.emit(abi::float_subtract_d(&r_lo, &r_hi, &prod));
        self.emit(abi::float_subtract_d(&r_lo, &prod_err, &r_lo));
        // Sign: negative reduced angle XOR negative x. For a negative x the
        // quadrant negates too: (q ^ sx) - sx.
        let sign_x = self.temporary_vreg();
        self.emit(abi::arithmetic_shift_right_immediate(&sign_x, bits, 63));
        self.emit(abi::exclusive_or_registers(&negative, &negative, &sign_x));
        self.emit(abi::shift_left_immediate(&negative, &negative, 63)); // sign bit
        let hi = self.temporary_vreg();
        let lo = self.temporary_vreg();
        self.emit(abi::float_move_x_from_d(&hi, &r_hi));
        self.emit(abi::float_move_x_from_d(&lo, &r_lo));
        self.emit(abi::exclusive_or_registers(&hi, &hi, &negative));
        self.emit(abi::exclusive_or_registers(&lo, &lo, &negative));
        self.emit(abi::exclusive_or_registers(&quad, &quad, &sign_x));
        self.emit(abi::subtract_registers(&quad, &quad, &sign_x));
        (hi, lo, quad)
    }

    /// `(high << b) | (low >> (64 - b))` into a fresh register, with `back = !b`.
    fn emit_shl_pair(
        &mut self,
        high: &VirtualRegister,
        low: &VirtualRegister,
        amount: &VirtualRegister,
        back: &VirtualRegister,
    ) -> VirtualRegister {
        let out = self.temporary_vreg();
        let spill = self.temporary_vreg();
        self.emit(abi::shift_left_variable(&out, high, amount));
        self.emit(abi::shift_right_immediate(&spill, low, 1));
        self.emit(abi::shift_right_variable(&spill, &spill, back));
        self.emit(abi::or_registers(&out, &out, &spill));
        out
    }

    /// A 53-bit piece: the low `64 - left` bits of `high` followed by the top
    /// `64 - right` bits of `low` (`(64 - left) + (64 - right) == 53`).
    fn emit_join_piece(
        &mut self,
        high: &VirtualRegister,
        left: u8,
        low: &VirtualRegister,
        right: u8,
    ) -> VirtualRegister {
        let out = self.temporary_vreg();
        let tail = self.temporary_vreg();
        self.emit(abi::shift_left_immediate(&out, high, left));
        self.emit(abi::shift_right_immediate(&out, &out, 11));
        self.emit(abi::shift_right_immediate(&tail, low, right));
        self.emit(abi::or_registers(&out, &out, &tail));
        out
    }

    /// `piece * scale` as a double: the piece has at most 53 bits, so the
    /// conversion is exact, and `scale` is a power of two.
    fn emit_scaled_piece(&mut self, piece: &VirtualRegister, scale: f64) -> Reg {
        let out: Reg = self.temporary_fp_vreg().into();
        let factor: Reg = self.temporary_fp_vreg().into();
        self.emit(abi::signed_convert_to_float_d(&out, piece));
        self.emit_load_pool_double(&factor, scale);
        self.emit(abi::float_multiply_d(&out, &out, &factor));
        out
    }

    /// Load a pooled `f64` constant into a scalar FP register.
    fn emit_load_pool_double(&mut self, dst: impl Into<Reg>, value: f64) {
        let dst: Reg = dst.into();
        let offset = math_const_pool_offset(value.to_bits())
            .expect("the exact trig reduction's constants are pooled");
        let pool = self.math_pool_base_reg();
        self.emit(abi::load_double(dst, &pool, offset));
    }
}
