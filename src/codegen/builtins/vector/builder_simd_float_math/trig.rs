//! plan-176-T: the `sin`/`cos`/`tan` kernels. The consts, the math const pool and the kernel
//! enums live in `builtins/vector/builder_simd_float_math/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// `sin`/`cos` kernel. After reduction, evaluate the polynomials in
    /// double-double (compensated Horner) for `sin_r = r*P_sin(r^2)` (collapsed
    /// into `v24`) and `cos_r = P_cos(r^2)` (collapsed into `v23`), then apply the
    /// quadrant selection/sign. The compensated polynomials make sin/cos strict
    /// <=1 ULP of macOS libm.
    /// `sin(±0)` is `±0` and `tan(±0)` is `±0` — IEEE 754 §6.3 keeps the sign of a
    /// zero argument, and every libm does. The kernel lost it: the reduced angle
    /// of `-0.0` is `-0.0`, `-0.0 * P(r^2)` is `-0.0`, but the compensated sum
    /// that collapses the double-double adds the `+0.0` low half, and
    /// `(-0.0) + (+0.0)` is `+0.0`. Rather than re-plumb the sign through the
    /// compensation, select the argument itself on any lane whose input is zero:
    /// `fcmeq_zero` is true for both zeros and false for everything else
    /// (including NaN), so no other lane can be touched. Costs three instructions
    /// on `sin` and `tan`; `cos(±0)` is `1.0` and needs nothing.
    ///
    /// Pre-existing, found while fixing bug-618 (the RED case is in
    /// `tests/runtime/rt_math_float_trig_large_angles.rs`).
    fn emit_zero_argument_passthrough(&mut self, k: &KernelRegs) {
        let arg = k.trig().arg;
        let mask: Reg = self.temporary_fp_vreg().into();
        self.emit(abi::vector_fcmeq_zero(&mask, &arg));
        self.emit(abi::vector_bsl(&mask, &arg, abi::VEC_SCRATCH[0]));
        self.emit(abi::vector_orr(abi::VEC_SCRATCH[0], &mask, &mask));
    }

    /// Save the argument for [`Self::emit_zero_argument_passthrough`] before the
    /// reduction overwrites `v0`.
    fn emit_save_trig_argument(&mut self, k: &KernelRegs) {
        let arg = k.trig().arg;
        self.emit(abi::vector_orr(
            &arg,
            abi::VEC_SCRATCH[0],
            abi::VEC_SCRATCH[0],
        ));
    }

    pub(super) fn emit_sin_cos_body(&mut self, want_cos: bool, k: &KernelRegs) {
        if !want_cos {
            self.emit_save_trig_argument(k);
        }
        self.emit_sincos_reduce(k, false); // reduced=v2, r2=v1, quad=v5
                                           // cos_r = collapse(P_cos(r2)) → v23.
        self.emit_cos_r_into(&k.v23, k);
        // sin_r = r * collapse(P_sin(r2)) → v24 (carry the lo through the multiply).
        self.emit_sin_r_into(&k.v24, k);
        // Quadrant masks: bit0 (v1) and bit1 (v0) of quad.
        self.emit(abi::vector_shl(
            abi::VEC_SCRATCH[1],
            abi::VEC_SCRATCH[5],
            63,
        ));
        self.emit(abi::vector_sshr(
            abi::VEC_SCRATCH[1],
            abi::VEC_SCRATCH[1],
            63,
        ));
        self.emit(abi::vector_shl(
            abi::VEC_SCRATCH[0],
            abi::VEC_SCRATCH[5],
            62,
        ));
        self.emit(abi::vector_sshr(
            abi::VEC_SCRATCH[0],
            abi::VEC_SCRATCH[0],
            63,
        ));
        if !want_cos {
            // sin: val = bit0 ? cos_r : sin_r; negate if bit1.
            self.emit(abi::vector_bsl(abi::VEC_SCRATCH[1], &k.v23, &k.v24));
            self.emit(abi::vector_fneg(abi::VEC_SCRATCH[3], abi::VEC_SCRATCH[1]));
            self.emit(abi::vector_bsl(
                abi::VEC_SCRATCH[0],
                abi::VEC_SCRATCH[3],
                abi::VEC_SCRATCH[1],
            ));
            self.emit_zero_argument_passthrough(k);
        } else {
            // cos: val = bit0 ? sin_r : cos_r; negate if bit0 XOR bit1.
            self.emit(abi::vector_eor(
                abi::VEC_SCRATCH[4],
                abi::VEC_SCRATCH[1],
                abi::VEC_SCRATCH[0],
            ));
            self.emit(abi::vector_bsl(abi::VEC_SCRATCH[1], &k.v24, &k.v23));
            self.emit(abi::vector_fneg(abi::VEC_SCRATCH[3], abi::VEC_SCRATCH[1]));
            self.emit(abi::vector_bsl(
                abi::VEC_SCRATCH[4],
                abi::VEC_SCRATCH[3],
                abi::VEC_SCRATCH[1],
            ));
            self.emit(abi::vector_orr(
                abi::VEC_SCRATCH[0],
                abi::VEC_SCRATCH[4],
                abi::VEC_SCRATCH[4],
            ));
        }
    }

    /// Scalar-only `sin`/`cos`: branch on the quadrant's bit0 and evaluate only
    /// the single polynomial the result needs. The array body must compute both
    /// `sin_r` and `cos_r` because its two lanes can land in different quadrants;
    /// a scalar call has one lane, so it can branch and skip the unused
    /// double-double Horner. Bit-identical to `emit_sin_cos_body` — same Cody-Waite
    /// reduction, same compensated polynomial, same sign select — for ~half the
    /// polynomial work. (`tan` still needs both halves and keeps the array body.)
    pub(super) fn emit_sin_cos_body_scalar(&mut self, want_cos: bool, k: &KernelRegs) {
        if !want_cos {
            self.emit_save_trig_argument(k);
        }
        self.emit_sincos_reduce(k, true); // reduced=v2, r2=v1, quad=v5
                                          // Negate mask → v25 (the Horner never touches v25/v26): sin negates on
                                          // bit1, cos on bit0^bit1. Matches emit_sin_cos_body's branchless masks.
        self.emit(abi::vector_shl(&k.v26, abi::VEC_SCRATCH[5], 63));
        self.emit(abi::vector_sshr(&k.v26, &k.v26, 63)); // bit0 all-ones
        self.emit(abi::vector_shl(&k.v25, abi::VEC_SCRATCH[5], 62));
        self.emit(abi::vector_sshr(&k.v25, &k.v25, 63)); // bit1 all-ones
        if want_cos {
            self.emit(abi::vector_eor(&k.v25, &k.v26, &k.v25)); // bit0 XOR bit1
        }
        // Branch on bit0. sin: bit0 ? cos_r : sin_r; cos: bit0 ? sin_r : cos_r.
        let bit0 = self.temporary_vreg();
        let one = self.temporary_vreg();
        self.emit(abi::vector_extract_to_x(&bit0, abi::VEC_SCRATCH[5], 0));
        self.emit(abi::move_immediate(&one, "Integer", "1"));
        self.emit(abi::and_registers(&bit0, &bit0, &one));
        let bit0_clear = self.label("simd_sc_bit0_clear");
        let sc_done = self.label("simd_sc_done");
        self.emit(abi::compare_immediate(&bit0, "0"));
        self.emit(abi::branch_eq(&bit0_clear));
        // bit0 set.
        if !want_cos {
            self.emit_cos_r_into(abi::VEC_SCRATCH[0], k);
        } else {
            self.emit_sin_r_into(abi::VEC_SCRATCH[0], k);
        }
        self.emit(abi::branch(&sc_done));
        self.emit(abi::label(&bit0_clear));
        // bit0 clear.
        if !want_cos {
            self.emit_sin_r_into(abi::VEC_SCRATCH[0], k);
        } else {
            self.emit_cos_r_into(abi::VEC_SCRATCH[0], k);
        }
        self.emit(abi::label(&sc_done));
        // Apply the sign: v0 = negmask ? -v0 : v0 (BIT inserts -v0 where set).
        self.emit(abi::vector_fneg(abi::VEC_SCRATCH[3], abi::VEC_SCRATCH[0]));
        self.emit(abi::vector_bit(
            abi::VEC_SCRATCH[0],
            abi::VEC_SCRATCH[3],
            &k.v25,
        ));
        if !want_cos {
            self.emit_zero_argument_passthrough(k);
        }
        self.emit_result_nan_into_mask(k);
    }

    /// `cos_r = collapse(P_cos(r2))` into `dst` (Horner var in `v1`). The exact
    /// instruction sequence emit_sin_cos_body uses, so the result is bit-identical.
    fn emit_cos_r_into(&mut self, dst: impl Into<Reg>, k: &KernelRegs) {
        let dst: Reg = dst.into();
        self.emit_compensated_horner(
            abi::VEC_SCRATCH[3],
            abi::VEC_SCRATCH[4],
            abi::VEC_SCRATCH[1],
            &COS_COEFFS,
            k,
        );
        let cos_corr = &k.trig().cos_corr;
        self.emit(abi::vector_fadd(
            abi::VEC_SCRATCH[4],
            abi::VEC_SCRATCH[4],
            cos_corr,
        )); // lo += -r*r_lo
        self.emit(abi::vector_fadd(
            dst,
            abi::VEC_SCRATCH[3],
            abi::VEC_SCRATCH[4],
        ));
    }

    /// `sin_r = collapse(r * P_sin(r2))` into `dst` (reduced angle in `v2`, Horner
    /// var in `v1`); carries the lo half through the multiply, exactly as
    /// emit_sin_cos_body — bit-identical.
    fn emit_sin_r_into(&mut self, dst: impl Into<Reg>, k: &KernelRegs) {
        let dst: Reg = dst.into();
        self.emit_compensated_horner(
            abi::VEC_SCRATCH[3],
            abi::VEC_SCRATCH[4],
            abi::VEC_SCRATCH[1],
            &SIN_COEFFS,
            k,
        );
        self.emit_twoprod(
            abi::VEC_SCRATCH[6],
            abi::VEC_SCRATCH[7],
            abi::VEC_SCRATCH[2],
            abi::VEC_SCRATCH[3],
        );
        self.emit(abi::vector_fmla(
            abi::VEC_SCRATCH[7],
            abi::VEC_SCRATCH[2],
            abi::VEC_SCRATCH[4],
        )); // pe += r*lo
        let sin_corr = &k.trig().sin_corr;
        self.emit(abi::vector_fadd(
            abi::VEC_SCRATCH[7],
            abi::VEC_SCRATCH[7],
            sin_corr,
        )); // pe += r_lo*(1 - r2/2)
        self.emit(abi::vector_fadd(
            dst,
            abi::VEC_SCRATCH[6],
            abi::VEC_SCRATCH[7],
        ));
    }

    /// `tan(x) = sin(x) / cos(x)`, strict <=1 ULP. `sin_r` and `cos_r` are
    /// computed as double-doubles (compensated Horner), the quadrant
    /// selection/sign is applied to BOTH the hi and lo halves, and the final
    /// quotient is evaluated with a one-step double-double-accurate division
    /// (`q = sh/ch; tan = q + (fma(-q,ch,sh) + (sl - q*cl))/ch`). Carrying the lo
    /// halves through the divide closes the near-pole 2-ULP residual the plain
    /// `fdiv` left. (It shares `emit_sincos_reduce` with sin/cos, so a huge or a
    /// cancelling angle takes the same exact reduction, bug-618.)
    /// tan's shared double-double `sin_r`/`cos_r` computation (bug-332 B2): reduce
    /// the angle, evaluate P_cos(r2) and reduced*P_sin(r2) as compensated
    /// double-doubles, and stash `cos_r` in (v25,v26) and `sin_r` in (v23,v24).
    /// Identical for the branchless (`emit_tan_body`) and scalar-branching
    /// (`emit_tan_body_scalar`) quadrant selects that follow, so extracting it
    /// keeps the two paths bit-identical through the reduction.
    fn emit_tan_sincos_dd(&mut self, k: &KernelRegs, scalar: bool) {
        self.emit_save_trig_argument(k);
        self.emit_sincos_reduce(k, scalar); // reduced=v2, r2=v1 (survives), quad=v5
                                            // cos_r as a double-double (hi,lo) → stash in v25/v26.
        self.emit_compensated_horner(
            abi::VEC_SCRATCH[3],
            abi::VEC_SCRATCH[4],
            abi::VEC_SCRATCH[1],
            &COS_COEFFS,
            k,
        );
        let t = k.trig();
        self.emit(abi::vector_fadd(
            abi::VEC_SCRATCH[4],
            abi::VEC_SCRATCH[4],
            &t.cos_corr,
        )); // lo += -r*r_lo
        self.emit(abi::vector_orr(
            &k.v25,
            abi::VEC_SCRATCH[3],
            abi::VEC_SCRATCH[3],
        )); // cos_hi
        self.emit(abi::vector_orr(
            &k.v26,
            abi::VEC_SCRATCH[4],
            abi::VEC_SCRATCH[4],
        )); // cos_lo
            // sin_r = reduced * P_sin(r2) as a double-double → stash in v23/v24.
        self.emit_compensated_horner(
            abi::VEC_SCRATCH[3],
            abi::VEC_SCRATCH[4],
            abi::VEC_SCRATCH[1],
            &SIN_COEFFS,
            k,
        );
        self.emit_twoprod(
            abi::VEC_SCRATCH[6],
            abi::VEC_SCRATCH[7],
            abi::VEC_SCRATCH[2],
            abi::VEC_SCRATCH[3],
        ); // reduced*sin_hi → (v6,v7)
        self.emit(abi::vector_fmla(
            abi::VEC_SCRATCH[7],
            abi::VEC_SCRATCH[2],
            abi::VEC_SCRATCH[4],
        )); // lo += reduced*sin_lo
        self.emit(abi::vector_fadd(
            abi::VEC_SCRATCH[7],
            abi::VEC_SCRATCH[7],
            &t.sin_corr,
        )); // lo += r_lo*(1 - r2/2)
        self.emit(abi::vector_orr(
            &k.v23,
            abi::VEC_SCRATCH[6],
            abi::VEC_SCRATCH[6],
        )); // sin_hi
        self.emit(abi::vector_orr(
            &k.v24,
            abi::VEC_SCRATCH[7],
            abi::VEC_SCRATCH[7],
        )); // sin_lo
    }

    pub(super) fn emit_tan_body(&mut self, k: &KernelRegs) {
        self.emit_tan_sincos_dd(k, false);
        // Quadrant masks: b0 → v27, b1 → v2.
        self.emit(abi::vector_shl(&k.v27, abi::VEC_SCRATCH[5], 63));
        self.emit(abi::vector_sshr(&k.v27, &k.v27, 63));
        self.emit(abi::vector_shl(
            abi::VEC_SCRATCH[2],
            abi::VEC_SCRATCH[5],
            62,
        ));
        self.emit(abi::vector_sshr(
            abi::VEC_SCRATCH[2],
            abi::VEC_SCRATCH[2],
            63,
        ));
        // sin_full = (b1 ? -1 : 1) * (b0 ? cos_r : sin_r), as a dd → (v28,v29).
        self.emit(abi::vector_orr(abi::VEC_SCRATCH[3], &k.v27, &k.v27));
        self.emit(abi::vector_bsl(abi::VEC_SCRATCH[3], &k.v25, &k.v23)); // b0?cos_hi:sin_hi
        self.emit(abi::vector_orr(abi::VEC_SCRATCH[4], &k.v27, &k.v27));
        self.emit(abi::vector_bsl(abi::VEC_SCRATCH[4], &k.v26, &k.v24)); // b0?cos_lo:sin_lo
        self.emit(abi::vector_fneg(abi::VEC_SCRATCH[6], abi::VEC_SCRATCH[3]));
        self.emit(abi::vector_fneg(abi::VEC_SCRATCH[7], abi::VEC_SCRATCH[4]));
        self.emit(abi::vector_orr(
            &k.v28,
            abi::VEC_SCRATCH[2],
            abi::VEC_SCRATCH[2],
        ));
        self.emit(abi::vector_bsl(
            &k.v28,
            abi::VEC_SCRATCH[6],
            abi::VEC_SCRATCH[3],
        )); // sin_full_hi
        self.emit(abi::vector_orr(
            &k.v29,
            abi::VEC_SCRATCH[2],
            abi::VEC_SCRATCH[2],
        ));
        self.emit(abi::vector_bsl(
            &k.v29,
            abi::VEC_SCRATCH[7],
            abi::VEC_SCRATCH[4],
        )); // sin_full_lo
            // cos_full = ((b0^b1) ? -1 : 1) * (b0 ? sin_r : cos_r), as a dd → (v30,v31).
        self.emit(abi::vector_orr(abi::VEC_SCRATCH[3], &k.v27, &k.v27));
        self.emit(abi::vector_bsl(abi::VEC_SCRATCH[3], &k.v23, &k.v25)); // b0?sin_hi:cos_hi
        self.emit(abi::vector_orr(abi::VEC_SCRATCH[4], &k.v27, &k.v27));
        self.emit(abi::vector_bsl(abi::VEC_SCRATCH[4], &k.v24, &k.v26)); // b0?sin_lo:cos_lo
        self.emit(abi::vector_fneg(abi::VEC_SCRATCH[6], abi::VEC_SCRATCH[3]));
        self.emit(abi::vector_fneg(abi::VEC_SCRATCH[7], abi::VEC_SCRATCH[4]));
        self.emit(abi::vector_eor(
            abi::VEC_SCRATCH[1],
            &k.v27,
            abi::VEC_SCRATCH[2],
        )); // b0^b1
        self.emit(abi::vector_orr(
            &k.v30,
            abi::VEC_SCRATCH[1],
            abi::VEC_SCRATCH[1],
        ));
        self.emit(abi::vector_bsl(
            &k.v30,
            abi::VEC_SCRATCH[6],
            abi::VEC_SCRATCH[3],
        )); // cos_full_hi
        self.emit(abi::vector_orr(
            &k.v31,
            abi::VEC_SCRATCH[1],
            abi::VEC_SCRATCH[1],
        ));
        self.emit(abi::vector_bsl(
            &k.v31,
            abi::VEC_SCRATCH[7],
            abi::VEC_SCRATCH[4],
        )); // cos_full_lo
            // Double-double-accurate divide: sh=v28 sl=v29 ch=v30 cl=v31.
        self.emit_tan_divide(&k.v28, &k.v29, &k.v30, &k.v31, k);
    }

    /// One-step double-double-accurate quotient `tan = sh:sl / ch:cl` into `v0`:
    /// `q = sh/ch; tan = q + (fma(-q,ch,sh) + (sl - q*cl))/ch`. Reads only the
    /// four operand registers; scratch is v0/v3/v4/v6.
    fn emit_tan_divide(
        &mut self,
        sh: impl Into<Reg>,
        sl: impl Into<Reg>,
        ch: impl Into<Reg>,
        cl: impl Into<Reg>,
        k: &KernelRegs,
    ) {
        let (sh, sl, ch, cl): (Reg, Reg, Reg, Reg) = (sh.into(), sl.into(), ch.into(), cl.into());
        self.emit(abi::vector_fdiv(abi::VEC_SCRATCH[0], sh, ch)); // q = sh/ch
        self.emit(abi::vector_fneg(abi::VEC_SCRATCH[3], abi::VEC_SCRATCH[0])); // -q
        self.emit(abi::vector_orr(abi::VEC_SCRATCH[4], sh, sh));
        self.emit(abi::vector_fmla(
            abi::VEC_SCRATCH[4],
            abi::VEC_SCRATCH[3],
            ch,
        )); // sh - q*ch (fma residual)
        self.emit(abi::vector_orr(abi::VEC_SCRATCH[6], sl, sl));
        self.emit(abi::vector_fmls(
            abi::VEC_SCRATCH[6],
            abi::VEC_SCRATCH[0],
            cl,
        )); // sl - q*cl
        self.emit(abi::vector_fadd(
            abi::VEC_SCRATCH[4],
            abi::VEC_SCRATCH[4],
            abi::VEC_SCRATCH[6],
        )); // num
        self.emit(abi::vector_fdiv(
            abi::VEC_SCRATCH[4],
            abi::VEC_SCRATCH[4],
            ch,
        )); // num/ch
        self.emit(abi::vector_fadd(
            abi::VEC_SCRATCH[0],
            abi::VEC_SCRATCH[0],
            abi::VEC_SCRATCH[4],
        )); // tan = q + num/ch
        self.emit_zero_argument_passthrough(k);
    }

    /// Scalar-only `tan`: `tan` has period π, so the quadrant reduces to bit0 —
    /// the bit1 signs negate BOTH the numerator and denominator double-doubles
    /// together and cancel in the quotient (bit-identically). A single scalar
    /// lane can therefore branch on bit0 and pick the (num,den) dd pair directly,
    /// dropping the whole branchless sin_full/cos_full quadrant-selection block
    /// the 2-lane array body needs. The two compensated Horners still run (the
    /// ratio needs both halves); only the selection is removed.
    pub(super) fn emit_tan_body_scalar(&mut self, k: &KernelRegs) {
        self.emit_tan_sincos_dd(k, true);
        // bit0 ? -cos_r/sin_r : sin_r/cos_r (bit1 cancels in the ratio).
        let bit0 = self.temporary_vreg();
        let one = self.temporary_vreg();
        self.emit(abi::vector_extract_to_x(&bit0, abi::VEC_SCRATCH[5], 0));
        self.emit(abi::move_immediate(&one, "Integer", "1"));
        self.emit(abi::and_registers(&bit0, &bit0, &one));
        let bit0_clear = self.label("simd_tan_bit0_clear");
        let tan_done = self.label("simd_tan_done");
        self.emit(abi::compare_immediate(&bit0, "0"));
        self.emit(abi::branch_eq(&bit0_clear));
        // bit0 set: num = cos_r, den = -sin_r.
        self.emit(abi::vector_fneg(&k.v30, &k.v23)); // -sin_hi
        self.emit(abi::vector_fneg(&k.v31, &k.v24)); // -sin_lo
        self.emit_tan_divide(&k.v25, &k.v26, &k.v30, &k.v31, k);
        self.emit(abi::branch(&tan_done));
        self.emit(abi::label(&bit0_clear));
        // bit0 clear: num = sin_r, den = cos_r.
        self.emit_tan_divide(&k.v23, &k.v24, &k.v25, &k.v26, k);
        self.emit(abi::label(&tan_done));
        // tan fails only on a NaN result (NaN/inf input) — same as the array body.
        self.emit_result_nan_into_mask(k);
    }
}
