//! plan-176-T: the `exp` and `log`/`log10` kernels. The consts, the math const pool and the
//! kernel enums live in `builtins/vector/builder_simd_float_math/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// `exp` kernel: n=floor(x/ln2+0.5), Cody-Waite r, Horner P(r), scale 2^n.
    pub(super) fn emit_exp_body(&mut self, k: &KernelRegs) {
        // NaN input → ErrFloatNan: chunk_nan = ~fcmeq(x,x); accumulate into v22.
        self.emit(abi::vector_fcmeq(
            abi::VEC_SCRATCH[6],
            abi::VEC_SCRATCH[0],
            abi::VEC_SCRATCH[0],
        )); // non-NaN lanes = all-ones
        self.emit(abi::vector_cmeq(
            abi::VEC_SCRATCH[7],
            abi::VEC_SCRATCH[0],
            abi::VEC_SCRATCH[0],
        )); // all-ones (bitwise self-eq)
        self.emit(abi::vector_eor(
            abi::VEC_SCRATCH[6],
            abi::VEC_SCRATCH[6],
            abi::VEC_SCRATCH[7],
        )); // NaN lanes = all-ones
        self.emit(abi::vector_orr(&k.v22, &k.v22, abi::VEC_SCRATCH[6]));
        // n = floor(x*(1/ln2) + 0.5); r = x - n*ln2 (Cody-Waite); Horner P(r).
        // v16 holds 1/ln2 (reciprocal multiply, not a divide).
        self.emit(abi::vector_fmul(
            abi::VEC_SCRATCH[1],
            abi::VEC_SCRATCH[0],
            &k.v16,
        ));
        self.emit(abi::vector_fadd(
            abi::VEC_SCRATCH[1],
            abi::VEC_SCRATCH[1],
            &k.v17,
        ));
        self.emit(abi::vector_frintm(abi::VEC_SCRATCH[1], abi::VEC_SCRATCH[1]));
        self.emit(abi::vector_orr(
            abi::VEC_SCRATCH[2],
            abi::VEC_SCRATCH[0],
            abi::VEC_SCRATCH[0],
        ));
        self.emit(abi::vector_fmls(
            abi::VEC_SCRATCH[2],
            abi::VEC_SCRATCH[1],
            &k.v18,
        ));
        self.emit(abi::vector_fmls(
            abi::VEC_SCRATCH[2],
            abi::VEC_SCRATCH[1],
            &k.v19,
        ));
        self.emit_horner(abi::VEC_SCRATCH[3], abi::VEC_SCRATCH[2], &EXP_COEFFS);
        self.emit(abi::vector_fcvtzs(abi::VEC_SCRATCH[5], abi::VEC_SCRATCH[1])); // n (int)
                                                                                 // Scale P(r) by 2^n in two steps, 2^n = 2^n1 · 2^n2 with n1 = n>>1 and
                                                                                 // n2 = n-n1, so each biased exponent (n_i+1023) stays inside the normal
                                                                                 // range and both `2^n_i` factors are exact powers of two. Multiplying by
                                                                                 // exact powers of two is itself exact for any in-range result, so this is
                                                                                 // bit-identical to the old single `(n+1023)<<52` shift wherever that shift
                                                                                 // was valid — but it now overflows to +Inf only for a genuinely
                                                                                 // out-of-range result and underflows through the subnormals to 0, instead
                                                                                 // of fabricating Inf at n=1024 (finite results near x≈709.5) and flushing
                                                                                 // every subnormal result to 0 (bug-130).
        self.emit(abi::vector_sshr(
            abi::VEC_SCRATCH[6],
            abi::VEC_SCRATCH[5],
            1,
        )); // n1 = n>>1
        self.emit(abi::vector_sub(
            abi::VEC_SCRATCH[7],
            abi::VEC_SCRATCH[5],
            abi::VEC_SCRATCH[6],
        )); // n2 = n-n1
        self.emit(abi::vector_add(
            abi::VEC_SCRATCH[6],
            abi::VEC_SCRATCH[6],
            &k.v20,
        )); // n1+1023
        self.emit(abi::vector_shl(
            abi::VEC_SCRATCH[6],
            abi::VEC_SCRATCH[6],
            52,
        )); // 2^n1
        self.emit(abi::vector_add(
            abi::VEC_SCRATCH[7],
            abi::VEC_SCRATCH[7],
            &k.v20,
        )); // n2+1023
        self.emit(abi::vector_shl(
            abi::VEC_SCRATCH[7],
            abi::VEC_SCRATCH[7],
            52,
        )); // 2^n2
            // bug-164: capture the overflow/underflow lane masks from the original x
            // (still live in VEC_SCRATCH[0]) before the scaling overwrites it. v25 =
            // overflow threshold, v26 = underflow threshold.
        self.emit(abi::vector_fcmgt(&k.v28, abi::VEC_SCRATCH[0], &k.v25)); // x > overflow
        self.emit(abi::vector_fcmgt(&k.v29, &k.v26, abi::VEC_SCRATCH[0])); // x < underflow
        self.emit(abi::vector_fmul(
            abi::VEC_SCRATCH[0],
            abi::VEC_SCRATCH[3],
            abi::VEC_SCRATCH[6],
        )); // P(r)·2^n1
        self.emit(abi::vector_fmul(
            abi::VEC_SCRATCH[0],
            abi::VEC_SCRATCH[0],
            abi::VEC_SCRATCH[7],
        )); // ·2^n2
            // Saturate out-of-range lanes: overflow -> +Inf (v27), underflow -> +0.0
            // (v21). The +Inf lanes then flow through the finiteness check below and
            // report ErrFloatInf (an honest overflow); the +0.0 lanes stay finite.
        self.emit_vsel(abi::VEC_SCRATCH[0], &k.v28, &k.v27);
        self.emit_vsel(abi::VEC_SCRATCH[0], &k.v29, &k.v21);
        // ErrFloatInf iff the actual result overflowed to +/-Inf: result*0 is NaN
        // only for a non-finite result (NaN inputs were already masked above).
        self.emit(abi::vector_fmul(
            abi::VEC_SCRATCH[6],
            abi::VEC_SCRATCH[0],
            &k.v21,
        )); // result*0
        self.emit(abi::vector_fcmeq(
            abi::VEC_SCRATCH[7],
            abi::VEC_SCRATCH[6],
            abi::VEC_SCRATCH[6],
        )); // finite = all-ones
        self.emit(abi::vector_cmeq(
            abi::VEC_SCRATCH[1],
            abi::VEC_SCRATCH[6],
            abi::VEC_SCRATCH[6],
        )); // all-ones
        self.emit(abi::vector_eor(
            abi::VEC_SCRATCH[7],
            abi::VEC_SCRATCH[7],
            abi::VEC_SCRATCH[1],
        )); // Inf lanes = all-ones
        self.emit(abi::vector_orr(&k.v24, &k.v24, abi::VEC_SCRATCH[7])); // accumulate inf mask
    }

    /// `log`/`log10` kernel: x = 2^k*m (frexp + fold to [1/sqrt2, sqrt2)),
    /// s=(m-1)/(m+1), ln(x) = k*ln2 + s*P(s^2), evaluated in double-double (a
    /// compensated Horner plus two-sum/two-product recombination) so the result
    /// is strict <=1 ULP; `log10` then multiplies by `1/ln10` as a double-double.
    /// Constants: v16 sqrt_half, v17 1.0, v18 0x7ff, v19 1022, v20 mantmask,
    /// v21 1022<<52, v23 1, v24/v25 ln2 hi/lo, v26/v27 1/ln10 hi/lo; v22 error.
    pub(super) fn emit_log_body(&mut self, base10: bool, k: &KernelRegs) {
        // Domain: x <= 0 fails.
        self.emit(abi::vector_fcmle_zero(
            abi::VEC_SCRATCH[1],
            abi::VEC_SCRATCH[0],
        ));
        self.emit(abi::vector_orr(&k.v22, &k.v22, abi::VEC_SCRATCH[1]));
        // Subnormal prologue (bug-134): a subnormal input (biased exponent field
        // 0) carries no implicit leading mantissa bit, so the exponent/mantissa
        // split below would extract a fake k = -1022 and a fake m. Scale such
        // lanes by 2^54 into the normal range — making the split correct — and
        // subtract 54 from the extracted exponent afterward, since
        // log(x) = log(x·2^54) − 54·ln2. The mask is taken from the original
        // bits; v28 is dead until the two-product further down.
        self.emit(abi::vector_ushr(
            abi::VEC_SCRATCH[1],
            abi::VEC_SCRATCH[0],
            52,
        ));
        self.emit(abi::vector_and(
            abi::VEC_SCRATCH[1],
            abi::VEC_SCRATCH[1],
            &k.v18,
        )); // exponent field
        self.emit(abi::vector_eor(
            abi::VEC_SCRATCH[2],
            abi::VEC_SCRATCH[2],
            abi::VEC_SCRATCH[2],
        )); // 0
        self.emit(abi::vector_cmeq(
            &k.v28,
            abi::VEC_SCRATCH[1],
            abi::VEC_SCRATCH[2],
        )); // subnormal (exp == 0)
        self.broadcast_f64(abi::VEC_SCRATCH[2], 18014398509481984.0); // 2^54
        self.emit(abi::vector_fmul(
            abi::VEC_SCRATCH[1],
            abi::VEC_SCRATCH[0],
            abi::VEC_SCRATCH[2],
        )); // x·2^54
        self.emit_vsel(abi::VEC_SCRATCH[0], &k.v28, abi::VEC_SCRATCH[1]); // v0 = subnormal ? x·2^54 : x
                                                                          // k = ((bits>>52) & 0x7ff) - 1022  (integer, v1).
        self.emit(abi::vector_ushr(
            abi::VEC_SCRATCH[1],
            abi::VEC_SCRATCH[0],
            52,
        ));
        self.emit(abi::vector_and(
            abi::VEC_SCRATCH[1],
            abi::VEC_SCRATCH[1],
            &k.v18,
        ));
        self.emit(abi::vector_sub(
            abi::VEC_SCRATCH[1],
            abi::VEC_SCRATCH[1],
            &k.v19,
        ));
        // Undo the 2^54 scale in the exponent for subnormal lanes (k -= 54).
        self.broadcast_i64(abi::VEC_SCRATCH[2], 54);
        self.emit(abi::vector_and(
            abi::VEC_SCRATCH[2],
            abi::VEC_SCRATCH[2],
            &k.v28,
        ));
        self.emit(abi::vector_sub(
            abi::VEC_SCRATCH[1],
            abi::VEC_SCRATCH[1],
            abi::VEC_SCRATCH[2],
        ));
        // m = bits with exponent field replaced by 1022 → m in [0.5, 1) (v6).
        self.emit(abi::vector_and(
            abi::VEC_SCRATCH[6],
            abi::VEC_SCRATCH[0],
            &k.v20,
        ));
        self.emit(abi::vector_orr(
            abi::VEC_SCRATCH[6],
            abi::VEC_SCRATCH[6],
            &k.v21,
        ));
        // if m < 1/sqrt2 { m *= 2; k -= 1 }.
        self.emit(abi::vector_fcmgt(
            abi::VEC_SCRATCH[7],
            &k.v16,
            abi::VEC_SCRATCH[6],
        )); // mask: sqrt_half > m
        self.emit(abi::vector_and(
            abi::VEC_SCRATCH[0],
            abi::VEC_SCRATCH[7],
            &k.v23,
        )); // mask & 1
        self.emit(abi::vector_sub(
            abi::VEC_SCRATCH[1],
            abi::VEC_SCRATCH[1],
            abi::VEC_SCRATCH[0],
        )); // k -= adjust
        self.emit(abi::vector_fadd(
            abi::VEC_SCRATCH[0],
            abi::VEC_SCRATCH[6],
            abi::VEC_SCRATCH[6],
        )); // m*2
        self.emit(abi::vector_bsl(
            abi::VEC_SCRATCH[7],
            abi::VEC_SCRATCH[0],
            abi::VEC_SCRATCH[6],
        )); // v7 = mask?m2:m  (= m)
        self.emit(abi::vector_scvtf(abi::VEC_SCRATCH[3], abi::VEC_SCRATCH[1])); // k -> float (v3)
                                                                                // s = (m-1)/(m+1) (v2); s2 = s*s (v1, the Horner variable).
        self.emit(abi::vector_fsub(
            abi::VEC_SCRATCH[0],
            abi::VEC_SCRATCH[7],
            &k.v17,
        )); // m - 1
        self.emit(abi::vector_fadd(
            abi::VEC_SCRATCH[6],
            abi::VEC_SCRATCH[7],
            &k.v17,
        )); // m + 1
        self.emit(abi::vector_fdiv(
            abi::VEC_SCRATCH[2],
            abi::VEC_SCRATCH[0],
            abi::VEC_SCRATCH[6],
        )); // s
        self.emit(abi::vector_fmul(
            abi::VEC_SCRATCH[1],
            abi::VEC_SCRATCH[2],
            abi::VEC_SCRATCH[2],
        )); // s2
            // P(s2) as a double-double (hi=v4, lo=v5) via compensated Horner.
        self.emit_compensated_horner(
            abi::VEC_SCRATCH[4],
            abi::VEC_SCRATCH[5],
            abi::VEC_SCRATCH[1],
            &LOG_COEFFS,
            k,
        );
        // ln(m) = s * (hi+lo): two-product then fma the lo terms → (v7=lh, v28=le).
        self.emit_twoprod(
            abi::VEC_SCRATCH[7],
            &k.v28,
            abi::VEC_SCRATCH[2],
            abi::VEC_SCRATCH[4],
        );
        self.emit(abi::vector_fmla(
            &k.v28,
            abi::VEC_SCRATCH[2],
            abi::VEC_SCRATCH[5],
        )); // le += s*lo
            // k*ln2 as a double-double → (v29=kh, v30=ke).
        self.emit_twoprod(&k.v29, &k.v30, abi::VEC_SCRATCH[3], &k.v24);
        self.emit(abi::vector_fmla(&k.v30, abi::VEC_SCRATCH[3], &k.v25)); // ke += k*ln2lo
                                                                          // (kh,ke) + (lh,le): two-sum hi, accumulate the lows → hi=v0, lo=v31.
                                                                          // Scratch v4/v5 are dead (Horner outputs consumed).
        self.emit_twosum(
            abi::VEC_SCRATCH[0],
            &k.v31,
            &k.v29,
            abi::VEC_SCRATCH[7],
            abi::VEC_SCRATCH[4],
            abi::VEC_SCRATCH[5],
        );
        self.emit(abi::vector_fadd(&k.v31, &k.v31, &k.v30)); // + ke
        self.emit(abi::vector_fadd(&k.v31, &k.v31, &k.v28)); // + le
        if !base10 {
            self.emit(abi::vector_fadd(
                abi::VEC_SCRATCH[0],
                abi::VEC_SCRATCH[0],
                &k.v31,
            )); // ln(x) = hi + lo
        } else {
            // log10(x) = (hi+lo) * (1/ln10 as hi+lo), compensated.
            self.emit_twoprod(
                abi::VEC_SCRATCH[6],
                abi::VEC_SCRATCH[7],
                abi::VEC_SCRATCH[0],
                &k.v26,
            ); // ph = hi*L10HI
            self.emit(abi::vector_fmla(
                abi::VEC_SCRATCH[7],
                abi::VEC_SCRATCH[0],
                &k.v27,
            )); // pe += hi*L10LO
            self.emit(abi::vector_fmla(abi::VEC_SCRATCH[7], &k.v31, &k.v26)); // pe += lo*L10HI
            self.emit(abi::vector_fadd(
                abi::VEC_SCRATCH[0],
                abi::VEC_SCRATCH[6],
                abi::VEC_SCRATCH[7],
            ));
        }
    }
}
