//! plan-176-T: the inverse-trig kernels — `asin`/`acos` and the `atan` core. The consts, the math const pool and the kernel enums live in
//! `builtins/vector/builder_simd_float_math/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// `asin(x)` via `asin(x) = atan(x / sqrt(1 - x^2))` and `acos(x)` via the
    /// half-angle identity `acos(x) = 2*atan( sqrt( (1-x)/(1+x) ) )`. NEON `fdiv`
    /// yields ±inf at the domain edge (`x=±1` for asin, `x=-1` for acos), which
    /// `atan` maps to the correct ±pi/2, so the endpoints come out exact. `acos`
    /// uses the half-angle form rather than `pi/2 - asin` to avoid catastrophic
    /// cancellation as `x → +1` (where `acos → 0`); `1±x` is exact for `|x| <= 1`
    /// (Sterbenz), so both stay ≤1 ULP. `ErrInvalidArgument` for `|x| > 1`.
    pub(super) fn emit_asin_acos_body(&mut self, want_acos: bool, scalar: bool, k: &KernelRegs) {
        // Domain: |x| > 1 fails.
        self.emit(abi::vector_and(
            abi::VEC_SCRATCH[1],
            abi::VEC_SCRATCH[0],
            &k.v19,
        )); // ax
        self.emit(abi::vector_fcmgt(
            abi::VEC_SCRATCH[6],
            abi::VEC_SCRATCH[1],
            &k.v16,
        )); // ax > 1
        self.emit(abi::vector_orr(&k.v22, &k.v22, abi::VEC_SCRATCH[6]));
        if !want_acos {
            // asin(x) = atan(x / sqrt(1 - x^2)).
            self.emit(abi::vector_orr(abi::VEC_SCRATCH[7], &k.v16, &k.v16)); // 1.0
            self.emit(abi::vector_fmls(
                abi::VEC_SCRATCH[7],
                abi::VEC_SCRATCH[0],
                abi::VEC_SCRATCH[0],
            )); // 1 - x*x
            self.emit(abi::vector_fsqrt(abi::VEC_SCRATCH[7], abi::VEC_SCRATCH[7]));
            self.emit(abi::vector_fdiv(
                abi::VEC_SCRATCH[0],
                abi::VEC_SCRATCH[0],
                abi::VEC_SCRATCH[7],
            )); // arg → v0
            if scalar {
                self.emit_atan_core_scalar(k);
            } else {
                self.emit_atan_core(k);
            } // v0 = atan(arg) = asin(x)
        } else {
            // acos(x) = 2*atan( sqrt( (1-x)/(1+x) ) ).
            self.emit(abi::vector_fsub(
                abi::VEC_SCRATCH[6],
                &k.v16,
                abi::VEC_SCRATCH[0],
            )); // 1 - x
            self.emit(abi::vector_fadd(
                abi::VEC_SCRATCH[7],
                &k.v16,
                abi::VEC_SCRATCH[0],
            )); // 1 + x
            self.emit(abi::vector_fdiv(
                abi::VEC_SCRATCH[0],
                abi::VEC_SCRATCH[6],
                abi::VEC_SCRATCH[7],
            )); // (1-x)/(1+x)
            self.emit(abi::vector_fsqrt(abi::VEC_SCRATCH[0], abi::VEC_SCRATCH[0])); // sqrt(...) >= 0
            if scalar {
                self.emit_atan_core_scalar(k);
            } else {
                self.emit_atan_core(k);
            } // v0 = atan(sqrt(...))
            self.emit(abi::vector_fadd(
                abi::VEC_SCRATCH[0],
                abi::VEC_SCRATCH[0],
                abi::VEC_SCRATCH[0],
            )); // 2*atan(...)
        }
    }

    /// fdlibm 5-segment `atan(x)` (input/result in `v0`): reduce `|x|` into a tiny
    /// argument via one of 5 segments (breakpoints 7/16, 11/16, 19/16, 39/16),
    /// evaluate the minimax `aT` polynomial in the fdlibm `s1`/`s2` split, and
    /// recombine `atan(c) - ((reduced*P - atan_lo) - reduced)` with the segment's
    /// double-double `atan(c)`; restore the sign. Strict <=1 ULP. Persistent
    /// inputs v18=sign mask, v19=abs mask; sign parked in v25; segment masks in
    /// v28-v31; offset in v26/v27. (Reused by asin/acos/atan2.)
    ///
    /// The scalar-branching twin `emit_atan_core_scalar` is deliberately kept
    /// separate (bug-332 C2): it computes only the one taken segment and selects
    /// via compare-and-branch rather than cumulative NEON masks, so the two are
    /// genuinely different instruction streams. Their only shared part — the
    /// polynomial recombine — is already factored into `emit_atan_poly_recombine`,
    /// which both call, so a coefficient change still lands in one place.
    pub(super) fn emit_atan_core(&mut self, k: &KernelRegs) {
        self.emit(abi::vector_and(
            abi::VEC_SCRATCH[1],
            abi::VEC_SCRATCH[0],
            &k.v19,
        )); // ax = |x|
        self.emit(abi::vector_and(&k.v25, abi::VEC_SCRATCH[0], &k.v18)); // sign(x)
                                                                         // Cumulative segment masks (ax >= threshold).
        self.broadcast_f64(abi::VEC_SCRATCH[4], ATAN_SEG_THRESH[0]);
        self.emit(abi::vector_fcmge(
            &k.v28,
            abi::VEC_SCRATCH[1],
            abi::VEC_SCRATCH[4],
        ));
        self.broadcast_f64(abi::VEC_SCRATCH[4], ATAN_SEG_THRESH[1]);
        self.emit(abi::vector_fcmge(
            &k.v29,
            abi::VEC_SCRATCH[1],
            abi::VEC_SCRATCH[4],
        ));
        self.broadcast_f64(abi::VEC_SCRATCH[4], ATAN_SEG_THRESH[2]);
        self.emit(abi::vector_fcmge(
            &k.v30,
            abi::VEC_SCRATCH[1],
            abi::VEC_SCRATCH[4],
        ));
        self.broadcast_f64(abi::VEC_SCRATCH[4], ATAN_SEG_THRESH[3]);
        self.emit(abi::vector_fcmge(
            &k.v31,
            abi::VEC_SCRATCH[1],
            abi::VEC_SCRATCH[4],
        ));
        // reduced = ax (default, segment -1); off = 0.
        self.emit(abi::vector_orr(
            abi::VEC_SCRATCH[2],
            abi::VEC_SCRATCH[1],
            abi::VEC_SCRATCH[1],
        ));
        self.emit(abi::vector_eor(&k.v26, &k.v26, &k.v26));
        self.emit(abi::vector_eor(&k.v27, &k.v27, &k.v27));
        // Segment 0: reduced=(2ax-1)/(2+ax), off=atan(0.5).
        self.broadcast_f64(abi::VEC_SCRATCH[4], 2.0);
        self.emit(abi::vector_fadd(
            abi::VEC_SCRATCH[5],
            abi::VEC_SCRATCH[1],
            abi::VEC_SCRATCH[1],
        )); // 2ax
        self.broadcast_f64(abi::VEC_SCRATCH[6], 1.0);
        self.emit(abi::vector_fsub(
            abi::VEC_SCRATCH[5],
            abi::VEC_SCRATCH[5],
            abi::VEC_SCRATCH[6],
        )); // 2ax-1
        self.emit(abi::vector_fadd(
            abi::VEC_SCRATCH[7],
            abi::VEC_SCRATCH[1],
            abi::VEC_SCRATCH[4],
        )); // 2+ax
        self.emit(abi::vector_fdiv(
            abi::VEC_SCRATCH[3],
            abi::VEC_SCRATCH[5],
            abi::VEC_SCRATCH[7],
        ));
        self.emit_vsel(abi::VEC_SCRATCH[2], &k.v28, abi::VEC_SCRATCH[3]);
        self.broadcast_f64(abi::VEC_SCRATCH[3], ATAN_HI[0]);
        self.emit_vsel(&k.v26, &k.v28, abi::VEC_SCRATCH[3]);
        self.broadcast_f64(abi::VEC_SCRATCH[3], ATAN_LO[0]);
        self.emit_vsel(&k.v27, &k.v28, abi::VEC_SCRATCH[3]);
        // Segment 1: reduced=(ax-1)/(ax+1), off=atan(1)=pi/4.
        self.broadcast_f64(abi::VEC_SCRATCH[6], 1.0);
        self.emit(abi::vector_fsub(
            abi::VEC_SCRATCH[5],
            abi::VEC_SCRATCH[1],
            abi::VEC_SCRATCH[6],
        ));
        self.emit(abi::vector_fadd(
            abi::VEC_SCRATCH[7],
            abi::VEC_SCRATCH[1],
            abi::VEC_SCRATCH[6],
        ));
        self.emit(abi::vector_fdiv(
            abi::VEC_SCRATCH[3],
            abi::VEC_SCRATCH[5],
            abi::VEC_SCRATCH[7],
        ));
        self.emit_vsel(abi::VEC_SCRATCH[2], &k.v29, abi::VEC_SCRATCH[3]);
        self.broadcast_f64(abi::VEC_SCRATCH[3], ATAN_HI[1]);
        self.emit_vsel(&k.v26, &k.v29, abi::VEC_SCRATCH[3]);
        self.broadcast_f64(abi::VEC_SCRATCH[3], ATAN_LO[1]);
        self.emit_vsel(&k.v27, &k.v29, abi::VEC_SCRATCH[3]);
        // Segment 2: reduced=(ax-1.5)/(1+1.5ax), off=atan(1.5).
        self.broadcast_f64(abi::VEC_SCRATCH[4], 1.5);
        self.emit(abi::vector_fmul(
            abi::VEC_SCRATCH[7],
            abi::VEC_SCRATCH[1],
            abi::VEC_SCRATCH[4],
        )); // 1.5ax
        self.broadcast_f64(abi::VEC_SCRATCH[6], 1.0);
        self.emit(abi::vector_fadd(
            abi::VEC_SCRATCH[7],
            abi::VEC_SCRATCH[7],
            abi::VEC_SCRATCH[6],
        )); // 1+1.5ax
        self.emit(abi::vector_fsub(
            abi::VEC_SCRATCH[5],
            abi::VEC_SCRATCH[1],
            abi::VEC_SCRATCH[4],
        )); // ax-1.5
        self.emit(abi::vector_fdiv(
            abi::VEC_SCRATCH[3],
            abi::VEC_SCRATCH[5],
            abi::VEC_SCRATCH[7],
        ));
        self.emit_vsel(abi::VEC_SCRATCH[2], &k.v30, abi::VEC_SCRATCH[3]);
        self.broadcast_f64(abi::VEC_SCRATCH[3], ATAN_HI[2]);
        self.emit_vsel(&k.v26, &k.v30, abi::VEC_SCRATCH[3]);
        self.broadcast_f64(abi::VEC_SCRATCH[3], ATAN_LO[2]);
        self.emit_vsel(&k.v27, &k.v30, abi::VEC_SCRATCH[3]);
        // Segment 3: reduced=-1/ax, off=atan(inf)=pi/2.
        self.broadcast_f64(abi::VEC_SCRATCH[6], 1.0);
        self.emit(abi::vector_fdiv(
            abi::VEC_SCRATCH[3],
            abi::VEC_SCRATCH[6],
            abi::VEC_SCRATCH[1],
        ));
        self.emit(abi::vector_fneg(abi::VEC_SCRATCH[3], abi::VEC_SCRATCH[3]));
        self.emit_vsel(abi::VEC_SCRATCH[2], &k.v31, abi::VEC_SCRATCH[3]);
        self.broadcast_f64(abi::VEC_SCRATCH[3], ATAN_HI[3]);
        self.emit_vsel(&k.v26, &k.v31, abi::VEC_SCRATCH[3]);
        self.broadcast_f64(abi::VEC_SCRATCH[3], ATAN_LO[3]);
        self.emit_vsel(&k.v27, &k.v31, abi::VEC_SCRATCH[3]);
        self.emit_atan_poly_recombine(k);
    }

    /// Shared tail of `atan` (plan-39 B4): the fdlibm `aT` polynomial in the
    /// `s1`/`s2` split, the double-double recombine with the segment's `atan(c)`
    /// (`v26`/`v27`), and the sign restore (`v25`). Inputs: reduced angle in `v2`,
    /// off hi/lo in `v26`/`v27`, sign in `v25`; result in `v0`. Factored out so the
    /// branchless (`emit_atan_core`) and scalar-branching (`emit_atan_core_scalar`)
    /// segment selects run the *identical* polynomial — bit-for-bit.
    fn emit_atan_poly_recombine(&mut self, k: &KernelRegs) {
        // Polynomial: z=reduced^2, w=z^2; s1=z*odd, s2=w*even (fdlibm split).
        self.emit(abi::vector_fmul(
            abi::VEC_SCRATCH[5],
            abi::VEC_SCRATCH[2],
            abi::VEC_SCRATCH[2],
        )); // z
        self.emit(abi::vector_fmul(
            abi::VEC_SCRATCH[6],
            abi::VEC_SCRATCH[5],
            abi::VEC_SCRATCH[5],
        )); // w
        self.broadcast_f64(abi::VEC_SCRATCH[3], ATAN_AT[10]);
        for &i in &[8usize, 6, 4, 2] {
            self.broadcast_f64(abi::VEC_SCRATCH[4], ATAN_AT[i]);
            self.emit(abi::vector_fmla(
                abi::VEC_SCRATCH[4],
                abi::VEC_SCRATCH[3],
                abi::VEC_SCRATCH[6],
            ));
            self.emit(abi::vector_orr(
                abi::VEC_SCRATCH[3],
                abi::VEC_SCRATCH[4],
                abi::VEC_SCRATCH[4],
            ));
        }
        self.broadcast_f64(abi::VEC_SCRATCH[4], ATAN_AT[0]);
        self.emit(abi::vector_fmla(
            abi::VEC_SCRATCH[4],
            abi::VEC_SCRATCH[3],
            abi::VEC_SCRATCH[6],
        ));
        self.emit(abi::vector_fmul(
            abi::VEC_SCRATCH[3],
            abi::VEC_SCRATCH[5],
            abi::VEC_SCRATCH[4],
        )); // s1 = z*(...)
        self.broadcast_f64(abi::VEC_SCRATCH[7], ATAN_AT[9]);
        for &i in &[7usize, 5, 3, 1] {
            self.broadcast_f64(abi::VEC_SCRATCH[4], ATAN_AT[i]);
            self.emit(abi::vector_fmla(
                abi::VEC_SCRATCH[4],
                abi::VEC_SCRATCH[7],
                abi::VEC_SCRATCH[6],
            ));
            self.emit(abi::vector_orr(
                abi::VEC_SCRATCH[7],
                abi::VEC_SCRATCH[4],
                abi::VEC_SCRATCH[4],
            ));
        }
        self.emit(abi::vector_fmul(
            abi::VEC_SCRATCH[7],
            abi::VEC_SCRATCH[6],
            abi::VEC_SCRATCH[7],
        )); // s2 = w*(...)
        self.emit(abi::vector_fadd(
            abi::VEC_SCRATCH[3],
            abi::VEC_SCRATCH[3],
            abi::VEC_SCRATCH[7],
        )); // P = s1+s2
        self.emit(abi::vector_fmul(
            abi::VEC_SCRATCH[3],
            abi::VEC_SCRATCH[2],
            abi::VEC_SCRATCH[3],
        )); // t = reduced*P
            // result = off_hi - ((t - off_lo) - reduced).
        self.emit(abi::vector_fsub(
            abi::VEC_SCRATCH[4],
            abi::VEC_SCRATCH[3],
            &k.v27,
        ));
        self.emit(abi::vector_fsub(
            abi::VEC_SCRATCH[4],
            abi::VEC_SCRATCH[4],
            abi::VEC_SCRATCH[2],
        ));
        self.emit(abi::vector_fsub(
            abi::VEC_SCRATCH[0],
            &k.v26,
            abi::VEC_SCRATCH[4],
        ));
        // Restore the sign (atan(|x|) >= 0).
        self.emit(abi::vector_orr(
            abi::VEC_SCRATCH[0],
            abi::VEC_SCRATCH[0],
            &k.v25,
        ));
    }

    /// plan-39 B4: scalar-lane `atan` core. Both lanes hold the same value, so the
    /// segment is the same for both — branch to the single segment's reduction
    /// instead of computing all five and masking. Each branch writes the *exact*
    /// `reduced`/`off_hi`/`off_lo` the branchless `emit_atan_core` selects for that
    /// segment (same ops, same constants), then runs the shared
    /// `emit_atan_poly_recombine`, so the result is bit-identical. Saves ~4 of the 5
    /// segment reductions (each a NEON `fdiv`).
    pub(super) fn emit_atan_core_scalar(&mut self, k: &KernelRegs) {
        self.emit(abi::vector_and(
            abi::VEC_SCRATCH[1],
            abi::VEC_SCRATCH[0],
            &k.v19,
        )); // ax = |x|
        self.emit(abi::vector_and(&k.v25, abi::VEC_SCRATCH[0], &k.v18)); // sign(x)
        let poly = self.label("atan_scalar_poly");
        // Try segments high threshold first (cumulative masks: highest wins).
        let seg_labels = [
            self.label("atan_scalar_seg0"),
            self.label("atan_scalar_seg1"),
            self.label("atan_scalar_seg2"),
            self.label("atan_scalar_seg3"),
        ];
        // ax >= THRESH[3] -> seg3, else >=[2] -> seg2, else >=[1] -> seg1, else
        // >=[0] -> seg0, else default. Emit the comparisons top-down as branches so
        // the highest threshold met wins (matching the branchless cumulative masks).
        for thresh in [3usize, 2, 1, 0] {
            self.broadcast_f64(abi::VEC_SCRATCH[4], ATAN_SEG_THRESH[thresh]);
            self.emit(abi::vector_fcmge(
                abi::VEC_SCRATCH[5],
                abi::VEC_SCRATCH[1],
                abi::VEC_SCRATCH[4],
            ));
            let bit = self.temporary_vreg();
            self.emit(abi::vector_extract_to_x(&bit, abi::VEC_SCRATCH[5], 0));
            self.emit(abi::compare_immediate(&bit, "0"));
            self.emit(abi::branch_ne(&seg_labels[thresh]));
        }
        // Default segment (ax < THRESH[0]): reduced=ax, off=0.
        self.emit(abi::vector_orr(
            abi::VEC_SCRATCH[2],
            abi::VEC_SCRATCH[1],
            abi::VEC_SCRATCH[1],
        ));
        self.emit(abi::vector_eor(&k.v26, &k.v26, &k.v26));
        self.emit(abi::vector_eor(&k.v27, &k.v27, &k.v27));
        self.emit(abi::branch(&poly));
        // Segment 0: reduced=(2ax-1)/(2+ax), off=atan(0.5).
        self.emit(abi::label(&seg_labels[0]));
        self.broadcast_f64(abi::VEC_SCRATCH[4], 2.0);
        self.emit(abi::vector_fadd(
            abi::VEC_SCRATCH[5],
            abi::VEC_SCRATCH[1],
            abi::VEC_SCRATCH[1],
        ));
        self.broadcast_f64(abi::VEC_SCRATCH[6], 1.0);
        self.emit(abi::vector_fsub(
            abi::VEC_SCRATCH[5],
            abi::VEC_SCRATCH[5],
            abi::VEC_SCRATCH[6],
        ));
        self.emit(abi::vector_fadd(
            abi::VEC_SCRATCH[7],
            abi::VEC_SCRATCH[1],
            abi::VEC_SCRATCH[4],
        ));
        self.emit(abi::vector_fdiv(
            abi::VEC_SCRATCH[2],
            abi::VEC_SCRATCH[5],
            abi::VEC_SCRATCH[7],
        ));
        self.broadcast_f64(&k.v26, ATAN_HI[0]);
        self.broadcast_f64(&k.v27, ATAN_LO[0]);
        self.emit(abi::branch(&poly));
        // Segment 1: reduced=(ax-1)/(ax+1), off=atan(1).
        self.emit(abi::label(&seg_labels[1]));
        self.broadcast_f64(abi::VEC_SCRATCH[6], 1.0);
        self.emit(abi::vector_fsub(
            abi::VEC_SCRATCH[5],
            abi::VEC_SCRATCH[1],
            abi::VEC_SCRATCH[6],
        ));
        self.emit(abi::vector_fadd(
            abi::VEC_SCRATCH[7],
            abi::VEC_SCRATCH[1],
            abi::VEC_SCRATCH[6],
        ));
        self.emit(abi::vector_fdiv(
            abi::VEC_SCRATCH[2],
            abi::VEC_SCRATCH[5],
            abi::VEC_SCRATCH[7],
        ));
        self.broadcast_f64(&k.v26, ATAN_HI[1]);
        self.broadcast_f64(&k.v27, ATAN_LO[1]);
        self.emit(abi::branch(&poly));
        // Segment 2: reduced=(ax-1.5)/(1+1.5ax), off=atan(1.5).
        self.emit(abi::label(&seg_labels[2]));
        self.broadcast_f64(abi::VEC_SCRATCH[4], 1.5);
        self.emit(abi::vector_fmul(
            abi::VEC_SCRATCH[7],
            abi::VEC_SCRATCH[1],
            abi::VEC_SCRATCH[4],
        ));
        self.broadcast_f64(abi::VEC_SCRATCH[6], 1.0);
        self.emit(abi::vector_fadd(
            abi::VEC_SCRATCH[7],
            abi::VEC_SCRATCH[7],
            abi::VEC_SCRATCH[6],
        ));
        self.emit(abi::vector_fsub(
            abi::VEC_SCRATCH[5],
            abi::VEC_SCRATCH[1],
            abi::VEC_SCRATCH[4],
        ));
        self.emit(abi::vector_fdiv(
            abi::VEC_SCRATCH[2],
            abi::VEC_SCRATCH[5],
            abi::VEC_SCRATCH[7],
        ));
        self.broadcast_f64(&k.v26, ATAN_HI[2]);
        self.broadcast_f64(&k.v27, ATAN_LO[2]);
        self.emit(abi::branch(&poly));
        // Segment 3: reduced=-1/ax, off=atan(inf).
        self.emit(abi::label(&seg_labels[3]));
        self.broadcast_f64(abi::VEC_SCRATCH[6], 1.0);
        self.emit(abi::vector_fdiv(
            abi::VEC_SCRATCH[2],
            abi::VEC_SCRATCH[6],
            abi::VEC_SCRATCH[1],
        ));
        self.emit(abi::vector_fneg(abi::VEC_SCRATCH[2], abi::VEC_SCRATCH[2]));
        self.broadcast_f64(&k.v26, ATAN_HI[3]);
        self.broadcast_f64(&k.v27, ATAN_LO[3]);
        // fall through to poly.
        self.emit(abi::label(&poly));
        self.emit_atan_poly_recombine(k);
    }
}
