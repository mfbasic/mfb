// The fdlibm/Remez constants below are spelled at full precision on purpose: each
// `hi` half is paired with a `lo` tail so the pair recombines past double
// precision. Two deny/warn-by-default lints fire on exactly that property —
// `approx_constant` (some sit near `std::f64::consts::*`, but a std const is a
// single rounded double and is NOT interchangeable) and `excessive_precision`
// (the digits past `f64` are what the `lo` tail consumes). Trimming a digit to
// silence either one would degrade `pow` accuracy: a correctness regression
// dressed as a lint fix. Mirrors the block in `builder_simd_float_math.rs`
// (bug-345-D2).
#![allow(clippy::approx_constant)]
#![allow(clippy::excessive_precision)]
use super::builder_math::FloatInfinityError;
use super::*;

// Hand-written scalar `pow(x, y)` — a port of fdlibm `__ieee754_pow` (public
// domain, Sun Microsystems). Unlike the other Float kernels this is *not* SIMD:
// `pow` is dominated by data-dependent bit manipulation (mantissa-segment
// selection, the integer-exponent / odd-even sign rule, the `2**k` exponent
// split) that does not vectorize cleanly, so it runs one element at a time over
// GPRs + scalar FP. Scalar `math::pow` calls it once; the array overload loops
// it per element. Faithfully rounded (<=1 ULP of the true value, matching macOS
// libm) including negative base with an integer exponent: `(-2)**3 = -8`.
//
// MFBASIC Float values are always finite (overflow raises ErrFloatInf, and there
// is no NaN value), so libm's inf/NaN exception prologue is omitted; the routine
// still *produces* inf on overflow and NaN for a negative base with a
// non-integer exponent, which the caller turns into ErrFloatInf / ErrFloatNan
// via emit_float_result_check. x == 0 / |x| == 1 fall out of the general path via
// natural overflow/underflow, so they need no special branch.
//
// plan-03 Phase 1 — register residency. The fdlibm working set (the ~19 f64
// intermediates and the two inputs) lives entirely in `d`-registers rather than
// stack slots: the kernel makes no `bl`, so it owns every caller-saved FP
// register (`d0`-`d7`, `d16`-`d31`) and never touches the callee-saved `d8`-`d15`
// the surrounding float allocator parks loop-carried values in — exactly the
// register discipline the SIMD kernels (`builder_simd_float_math.rs`) already
// follow. `d0`/`d1`/`d2` are scratch; the remaining 21 caller-saved registers are
// the value homes (see `PowHomes`). This removes every per-op load/store and the
// GPR bounce the old `pld`/`pst` helpers incurred — the arithmetic is unchanged,
// so the result is bit-identical (still <=1 ULP, the `pow.ref` golden is stable).

const BP: [f64; 2] = [1.0, 1.5];
const DP_H: [f64; 2] = [0.0, 5.849_624_872_207_641_601_56e-01];
const DP_L: [f64; 2] = [0.0, 1.350_039_202_129_748_971_28e-08];
const L1: f64 = 5.999_999_999_999_946_487_25e-01;
const L2: f64 = 4.285_714_285_785_501_842_52e-01;
const L3: f64 = 3.333_333_298_183_774_329_18e-01;
const L4: f64 = 2.727_281_238_085_340_064_89e-01;
const L5: f64 = 2.306_607_457_755_613_663_31e-01;
const L6: f64 = 2.069_750_178_003_384_177_84e-01;
const P1: f64 = 1.666_666_666_666_660_190_37e-01;
const P2: f64 = -2.777_777_777_701_559_338_42e-03;
const P3: f64 = 6.613_756_321_437_934_361_17e-05;
const P4: f64 = -1.653_390_220_546_525_153_90e-06;
const P5: f64 = 4.138_136_797_057_238_460_39e-08;
const LG2: f64 = 6.931_471_805_599_452_862_27e-01;
const LG2_H: f64 = 6.931_471_824_645_996_093_75e-01;
const LG2_L: f64 = -1.904_654_299_957_768_045_25e-09;
const CP: f64 = 9.617_966_939_259_755_543_29e-01;
const CP_H: f64 = 9.617_967_009_544_372_558_59e-01;
const CP_L: f64 = -7.028_461_650_952_758_265_16e-09;
const TWO53: f64 = 9_007_199_254_740_992.0;
const HUGE: f64 = 1.0e300;
const TINY: f64 = 1.0e-300;

const HIGH32_MASK: &str = "18446744069414584320"; // 0xFFFFFFFF00000000
const ABS_MASK: &str = "9223372036854775807"; // 0x7FFFFFFFFFFFFFFF
const SIGN_BIT: &str = "9223372036854775808"; // 0x8000000000000000
const TWOM54_BITS: &str = "4363988038922010624"; // 2**-54 = 0x3C90000000000000

/// Register homes for the scalar pow kernel — one home per live f64. The five
/// low homes (`x`/`y` inputs and `ax`/`sh`/`sl`) are the physical caller-saved
/// `d3`–`d7`, disjoint from the `d0`-`d2` scratch; the remaining sixteen are FP
/// virtual registers minted per invocation ([`CodeBuilder::emit_pow_scalar`]),
/// so the allocator places them per-ISA — no fixed high-bank claim and no
/// post-emit register patching. The kernel makes no returning call, so every
/// home is caller-saved-safe exactly as the historical fixed `d16`–`d31` bank
/// was.
struct PowHomes<'a> {
    x: &'a str,
    y: &'a str,
    ax: &'a str,
    sh: &'a str,
    sl: &'a str,
    th: &'a str,
    tl: &'a str,
    rr: &'a str,
    uu: &'a str,
    vv: &'a str,
    ww: &'a str,
    ph: &'a str,
    pl: &'a str,
    zh: &'a str,
    zl: &'a str,
    t1: &'a str,
    t2: &'a str,
    s2: &'a str,
    tmp: &'a str,
    cs: &'a str,
    zz: &'a str,
}

impl CodeBuilder<'_> {
    /// Copy a value home into a working `d`-register (a no-op when they coincide).
    fn pld(&mut self, d: &str, home: &str) {
        if d != home {
            self.emit(abi::float_move_d_from_d(d, home));
        }
    }
    /// Copy a working `d`-register back into a value home (no-op when identical).
    fn pst(&mut self, d: &str, home: &str) {
        if d != home {
            self.emit(abi::float_move_d_from_d(home, d));
        }
    }
    /// `dst = a <op> b`, operating register-to-register (homes are live, so no
    /// load/store and no scratch are needed; `dst` may alias `a`/`b`).
    fn pop(&mut self, op: char, dst: &str, a: &str, b: &str) {
        match op {
            '+' => self.emit(abi::float_add_d(dst, a, b)),
            '-' => self.emit(abi::float_subtract_d(dst, a, b)),
            '*' => self.emit(abi::float_multiply_d(dst, a, b)),
            '/' => self.emit(abi::float_divide_d(dst, a, b)),
            _ => unreachable!(),
        }
    }
    /// `home = value` (an f64 constant materialized through GPR `xs`).
    fn pconst(&mut self, home: &str, value: f64, xs: &str) {
        self.emit(abi::move_immediate(
            xs,
            "Integer",
            &value.to_bits().to_string(),
        ));
        self.emit(abi::float_move_d_from_x(home, xs));
    }
    /// `home &= 0xFFFFFFFF00000000` — zero the low 32 bits of the f64 (fdlibm's
    /// `SET_LOW_WORD(x, 0)` head/tail split).
    fn plowzero(&mut self, home: &str, xs: &str, xm: &str) {
        self.emit(abi::float_move_x_from_d(xs, home));
        self.emit(abi::move_immediate(xm, "Integer", HIGH32_MASK));
        self.emit(abi::and_registers(xs, xs, xm));
        self.emit(abi::float_move_d_from_x(home, xs));
    }
    /// `out = c[n-1]; out = c[i] + out*var` (Horner, ascending coeffs). Uses `d0`
    /// (accumulator) and `d1` (coefficient) as scratch; `var`/`out` are homes.
    fn ppoly(&mut self, var: &str, coeffs: &[f64], out: &str, xs: &str) {
        self.emit(abi::move_immediate(
            xs,
            "Integer",
            &coeffs[coeffs.len() - 1].to_bits().to_string(),
        ));
        self.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[0], xs));
        for &c in coeffs.iter().rev().skip(1) {
            self.emit(abi::move_immediate(xs, "Integer", &c.to_bits().to_string()));
            self.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[1], xs));
            // One single-rounded fused step `d0 = d1 + d0*var` in place of the
            // discrete `fmul; fadd` (plan-02 §4 kernel audit): fewer instructions
            // and, like the vector `fmla` Horner steps, one rounding instead of two.
            self.emit(abi::float_multiply_add_d(
                abi::FP_SCRATCH[0],
                abi::FP_SCRATCH[1],
                abi::FP_SCRATCH[0],
                var,
            ));
        }
        self.pst(abi::FP_SCRATCH[0], out);
    }

    /// Emit scalar `pow(x, y)`; returns a register with the result bit pattern.
    pub(super) fn emit_pow_scalar(&mut self, x_loc: &str, y_loc: &str) -> Result<String, String> {
        // The sixteen high value homes are FP vregs minted per invocation (the
        // low five stay on the physical `d3`-`d7` input/scratch bank).
        let th_v = self.temporary_fp_vreg();
        let tl_v = self.temporary_fp_vreg();
        let rr_v = self.temporary_fp_vreg();
        let uu_v = self.temporary_fp_vreg();
        let vv_v = self.temporary_fp_vreg();
        let ww_v = self.temporary_fp_vreg();
        let ph_v = self.temporary_fp_vreg();
        let pl_v = self.temporary_fp_vreg();
        let zh_v = self.temporary_fp_vreg();
        let zl_v = self.temporary_fp_vreg();
        let t1_v = self.temporary_fp_vreg();
        let t2_v = self.temporary_fp_vreg();
        let s2_v = self.temporary_fp_vreg();
        let tmp_v = self.temporary_fp_vreg();
        let cs_v = self.temporary_fp_vreg();
        let zz_v = self.temporary_fp_vreg();
        let s = PowHomes {
            x: abi::FP_SCRATCH[3],
            y: abi::FP_SCRATCH[4],
            ax: abi::FP_SCRATCH[5],
            sh: abi::FP_SCRATCH[6],
            sl: abi::FP_SCRATCH[7],
            th: &th_v,
            tl: &tl_v,
            rr: &rr_v,
            uu: &uu_v,
            vv: &vv_v,
            ww: &ww_v,
            ph: &ph_v,
            pl: &pl_v,
            zh: &zh_v,
            zl: &zl_v,
            t1: &t1_v,
            t2: &t2_v,
            s2: &s2_v,
            tmp: &tmp_v,
            cs: &cs_v,
            zz: &zz_v,
        };
        // Inputs arrive in GPRs; move their bit patterns into the value homes.
        self.emit(abi::float_move_d_from_x(s.x, x_loc));
        self.emit(abi::float_move_d_from_x(s.y, y_loc));
        self.reset_temporary_registers();

        let result = self.allocate_register()?;
        let xs_o = self.allocate_register()?;
        let xm_o = self.allocate_register()?;
        let xt_o = self.allocate_register()?;
        let xu_o = self.allocate_register()?;
        let smask_o = self.allocate_register()?;
        let nexp_o = self.allocate_register()?;
        let (xs, xm, xt, xu, smask, nexp) = (
            xs_o.as_str(),
            xm_o.as_str(),
            xt_o.as_str(),
            xu_o.as_str(),
            smask_o.as_str(),
            nexp_o.as_str(),
        );

        let end = self.label("pow_end");
        let ret_nan = self.label("pow_ret_nan");

        // y == 0 -> 1.0
        self.emit(abi::float_move_x_from_d(xs, s.y));
        self.emit(abi::move_immediate(xm, "Integer", ABS_MASK));
        self.emit(abi::and_registers(xs, xs, xm));
        let y_nonzero = self.label("pow_y_nonzero");
        self.emit(abi::compare_immediate(xs, "0"));
        self.emit(abi::branch_ne(&y_nonzero));
        self.emit(abi::move_immediate(
            &result,
            "Integer",
            &1.0f64.to_bits().to_string(),
        ));
        self.emit(abi::branch(&end));
        self.emit(abi::label(&y_nonzero));

        // |x| == 0 (x is +-0.0): fdlibm's special-value rule, not the general
        // log2/exp2 path. The general path handles +0.0 via natural
        // overflow/underflow, but -0.0 has the sign bit set and the x<0 test in
        // emit_pow_yisint is a *signed* compare of the raw bits, so -0.0 wrongly
        // classified as a negative base and routed pow(-0.0, non-integer) to
        // ret_nan (bug-137.5). Handle both zeros here per e_pow.c: z = |x| = +0;
        // y<0 -> z = 1/z = +inf; then for x == -0.0 and an odd-integer y, negate z
        // (the (-1)**non-int NaN sub-case of fdlibm cannot occur for ix == 0). The
        // caller's emit_float_result_check turns a +-inf result into ErrFloatInf.
        let x_nonzero = self.label("pow_x_nonzero");
        self.emit(abi::float_move_x_from_d(xs, s.x));
        self.emit(abi::move_immediate(xm, "Integer", ABS_MASK));
        self.emit(abi::and_registers(xt, xs, xm)); // |x| bits
        self.emit(abi::compare_immediate(xt, "0"));
        self.emit(abi::branch_ne(&x_nonzero));
        // x is +-0.0. base result = (y < 0 ? +inf : +0.0).
        self.emit(abi::move_immediate(&result, "Integer", "0")); // +0.0
        let zero_ypos = self.label("pow_zero_ypos");
        self.emit(abi::float_move_x_from_d(xm, s.y));
        self.emit(abi::compare_immediate(xm, "0")); // signed: y < 0 ?
        self.emit(abi::branch_ge(&zero_ypos));
        self.emit(abi::move_immediate(
            &result,
            "Integer",
            "9218868437227405312",
        )); // +inf = 0x7FF0000000000000
        self.emit(abi::label(&zero_ypos));
        // Negate the result iff x is -0.0 AND y is an odd integer.
        let zero_ret = self.label("pow_zero_ret");
        self.emit(abi::float_move_x_from_d(xm, s.x));
        self.emit(abi::compare_immediate(xm, "0")); // signed: x < 0 (i.e. -0.0) ?
        self.emit(abi::branch_ge(&zero_ret)); // x == +0.0 -> no sign flip
                                              // |y| >= 2^53 -> even integer -> no flip.
        self.emit(abi::float_move_x_from_d(xs, s.y));
        self.emit(abi::move_immediate(xm, "Integer", ABS_MASK));
        self.emit(abi::and_registers(xs, xs, xm)); // |y| bits
        self.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[0], xs)); // |y|
        self.emit_f64_const(abi::FP_SCRATCH[1], xt, TWO53);
        self.emit(abi::float_subtract_d(
            abi::FP_SCRATCH[2],
            abi::FP_SCRATCH[0],
            abi::FP_SCRATCH[1],
        ));
        self.emit(abi::float_compare_zero_d(abi::FP_SCRATCH[2]));
        self.emit(abi::branch_ge(&zero_ret)); // |y| >= 2^53 -> even
                                              // trunc(y) == y ? (non-integer -> no flip).
        self.emit(abi::float_move_x_from_d(xs, s.y));
        self.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[0], xs)); // y
        self.emit(abi::float_convert_to_signed_x(xt, abi::FP_SCRATCH[0])); // trunc(y)
        self.emit(abi::signed_convert_to_float_d(abi::FP_SCRATCH[1], xt));
        self.emit(abi::float_move_x_from_d(xm, abi::FP_SCRATCH[1]));
        self.emit(abi::compare_registers(xm, xs));
        self.emit(abi::branch_ne(&zero_ret)); // non-integer -> no flip
                                              // odd? trunc & 1.
        self.emit(abi::move_immediate(xm, "Integer", "1"));
        self.emit(abi::and_registers(xt, xt, xm));
        self.emit(abi::compare_immediate(xt, "0"));
        self.emit(abi::branch_eq(&zero_ret)); // even -> no flip
        self.emit(abi::move_immediate(xm, "Integer", SIGN_BIT));
        self.emit(abi::exclusive_or_registers(&result, &result, xm)); // -0.0 ** odd -> negate
        self.emit(abi::label(&zero_ret));
        self.emit(abi::branch(&end));
        self.emit(abi::label(&x_nonzero));

        // Sign / integer-exponent rule for x < 0 (sets smask or jumps to ret_nan).
        self.emit(abi::move_immediate(smask, "Integer", "0"));
        self.emit_pow_yisint(s.x, s.y, xs, xm, xt, smask, &ret_nan);

        // ax = |x|
        self.emit(abi::float_move_x_from_d(xs, s.x));
        self.emit(abi::move_immediate(xm, "Integer", ABS_MASK));
        self.emit(abi::and_registers(xs, xs, xm));
        self.emit(abi::float_move_d_from_x(s.ax, xs));

        // log2(ax) -> (t1, t2)
        self.emit_pow_log2(&s, xs, xm, xt, xu, nexp);

        // y*log2(ax) = p_h + p_l ; z = p_h + p_l
        self.emit(abi::float_move_x_from_d(xs, s.y));
        self.emit(abi::move_immediate(xm, "Integer", HIGH32_MASK));
        self.emit(abi::and_registers(xs, xs, xm));
        self.emit(abi::float_move_d_from_x(s.ww, xs)); // ww = y1
        self.pop('-', s.tmp, s.y, s.ww); // y - y1
        self.pop('*', s.tmp, s.tmp, s.t1);
        self.pop('*', s.cs, s.y, s.t2); // y*t2
        self.pop('+', s.pl, s.tmp, s.cs); // p_l
        self.pop('*', s.ph, s.ww, s.t1); // p_h = y1*t1
        self.pop('+', s.zz, s.pl, s.ph); // z

        // overflow: (signed) hi32(z) >= 0x40900000  (signed: z negative is never
        // an overflow, so the high word must be sign-extended).
        let not_ovf = self.label("pow_not_ovf");
        self.emit(abi::float_move_x_from_d(xs, s.zz));
        self.emit(abi::move_immediate(xm, "Integer", "32"));
        self.emit(abi::arithmetic_shift_right_variable(xt, xs, xm));
        self.emit(abi::move_immediate(xm, "Integer", "1083179008")); // 0x40900000
        self.emit(abi::compare_registers(xt, xm));
        self.emit(abi::branch_lt(&not_ovf));
        self.pconst(s.cs, HUGE, xs);
        self.pop('*', s.tmp, s.cs, s.cs);
        self.emit(abi::float_move_x_from_d(&result, s.tmp));
        self.emit(abi::exclusive_or_registers(&result, &result, smask));
        self.emit(abi::branch(&end));
        self.emit(abi::label(&not_ovf));

        // underflow: (hi32(z) & 0x7fffffff) >= 0x4090cc00
        let do_2exp = self.label("pow_do_2exp");
        self.emit(abi::float_move_x_from_d(xs, s.zz));
        self.emit(abi::shift_right_immediate(xt, xs, 32));
        self.emit(abi::move_immediate(xm, "Integer", "2147483647"));
        self.emit(abi::and_registers(xt, xt, xm));
        self.emit(abi::move_immediate(xm, "Integer", "1083280384")); // 0x4090cc00
        self.emit(abi::compare_registers(xt, xm));
        self.emit(abi::branch_lt(&do_2exp));
        self.pconst(s.cs, TINY, xs);
        self.pop('*', s.tmp, s.cs, s.cs);
        self.emit(abi::float_move_x_from_d(&result, s.tmp));
        self.emit(abi::exclusive_or_registers(&result, &result, smask));
        self.emit(abi::branch(&end));
        self.emit(abi::label(&do_2exp));

        // 2**(p_h + p_l) -> zz
        self.emit_pow_exp2(&s, xs, xm, xt, xu, nexp);
        self.emit(abi::float_move_x_from_d(&result, s.zz));
        self.emit(abi::exclusive_or_registers(&result, &result, smask));
        self.emit(abi::branch(&end));

        self.emit(abi::label(&ret_nan));
        self.emit(abi::move_immediate(
            &result,
            "Integer",
            "9221120237041090560",
        )); // 0x7FF8000000000000
        self.emit(abi::label(&end));
        Ok(result)
    }

    /// `math.pow(base AS Float[], exp AS Float[]) AS Float[]` — per-element scalar
    /// fdlibm pow (the kernel does not vectorize). `ErrInvalidArgument` if the two
    /// lists differ in length; per element, overflow -> ErrFloatInf and negative
    /// base with a non-integer exponent -> ErrFloatNan, matching the scalar man
    /// page and `f(x) == f([x])[0]`.
    pub(super) fn lower_pow_array(
        &mut self,
        left_slot: usize,
        right_slot: usize,
        text: String,
    ) -> Result<ValueResult, String> {
        self.reset_temporary_registers();
        let left_ptr = self.allocate_register()?;
        self.emit(abi::load_u64(&left_ptr, abi::stack_pointer(), left_slot));
        let right_ptr = self.allocate_register()?;
        self.emit(abi::load_u64(&right_ptr, abi::stack_pointer(), right_slot));
        let count = self.allocate_register()?;
        let rcount = self.allocate_register()?;
        self.emit(abi::load_u64(&count, &left_ptr, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(&rcount, &right_ptr, COLLECTION_OFFSET_COUNT));
        let lengths_ok = self.label("pow_arr_len_ok");
        self.emit(abi::compare_registers(&count, &rcount));
        self.emit(abi::branch_eq(&lengths_ok));
        self.emit_invalid_argument_return()?;
        self.emit(abi::label(&lengths_ok));
        let count_slot = self.allocate_stack_object("pow_arr_count", 8);
        self.emit(abi::store_u64(&count, abi::stack_pointer(), count_slot));

        self.emit(abi::move_register(abi::ARG[0], &count));
        self.emit(abi::move_immediate(
            abi::ARG[1],
            "Integer",
            &COLLECTION_TYPE_FLOAT.to_string(),
        ));
        self.emit(abi::branch_link(SIMD_ALLOC_LIST_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: SIMD_ALLOC_LIST_SYMBOL.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
        self.reset_temporary_registers();
        let result_base = self.allocate_register()?;
        self.emit(abi::move_register(&result_base, abi::return_register()));
        let alloc_ok = self.label("pow_arr_alloc_ok");
        self.emit(abi::compare_immediate(abi::RET[1], "0"));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit(abi::move_register(abi::return_register(), abi::RET[1]));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&alloc_ok));

        // Loop state lives in stack slots (emit_pow_scalar clobbers the file).
        let result_slot = self.allocate_stack_object("pow_arr_result", 8);
        let ldata_slot = self.allocate_stack_object("pow_arr_ldata", 8);
        let rdata_slot = self.allocate_stack_object("pow_arr_rdata", 8);
        let odata_slot = self.allocate_stack_object("pow_arr_odata", 8);
        let index_slot = self.allocate_stack_object("pow_arr_index", 8);
        self.emit(abi::store_u64(
            &result_base,
            abi::stack_pointer(),
            result_slot,
        ));
        let left_ptr = self.allocate_register()?;
        self.emit(abi::load_u64(&left_ptr, abi::stack_pointer(), left_slot));
        let right_ptr = self.allocate_register()?;
        self.emit(abi::load_u64(&right_ptr, abi::stack_pointer(), right_slot));
        let ldata = self.allocate_register()?;
        self.emit_collection_data_pointer_for(&ldata, &left_ptr, "Float");
        self.emit(abi::store_u64(&ldata, abi::stack_pointer(), ldata_slot));
        let rdata = self.allocate_register()?;
        self.emit_collection_data_pointer_for(&rdata, &right_ptr, "Float");
        self.emit(abi::store_u64(&rdata, abi::stack_pointer(), rdata_slot));
        let odata = self.allocate_register()?;
        self.emit_collection_data_pointer_for(&odata, &result_base, "Float");
        self.emit(abi::store_u64(&odata, abi::stack_pointer(), odata_slot));
        self.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), index_slot));

        let loop_label = self.label("pow_arr_loop");
        let loop_done = self.label("pow_arr_done");
        self.emit(abi::label(&loop_label));
        self.reset_temporary_registers();
        let index = self.allocate_register()?;
        let cnt = self.allocate_register()?;
        self.emit(abi::load_u64(&index, abi::stack_pointer(), index_slot));
        self.emit(abi::load_u64(&cnt, abi::stack_pointer(), count_slot));
        self.emit(abi::compare_registers(&index, &cnt));
        self.emit(abi::branch_ge(&loop_done));
        // offset = index*8
        let off = self.allocate_register()?;
        self.emit(abi::shift_left_immediate(&off, &index, 3));
        let lbase = self.allocate_register()?;
        self.emit(abi::load_u64(&lbase, abi::stack_pointer(), ldata_slot));
        self.emit(abi::add_registers(&lbase, &lbase, &off));
        let lbits = self.allocate_register()?;
        self.emit(abi::load_u64(&lbits, &lbase, 0));
        let rbase = self.allocate_register()?;
        self.emit(abi::load_u64(&rbase, abi::stack_pointer(), rdata_slot));
        self.emit(abi::add_registers(&rbase, &rbase, &off));
        let rbits = self.allocate_register()?;
        self.emit(abi::load_u64(&rbits, &rbase, 0));
        let res = self.emit_pow_scalar(&lbits, &rbits)?; // resets the register file
        self.emit_float_result_check(&res, FloatInfinityError::Infinity)?;
        // Reload out base + index for the store.
        let res_slot = self.allocate_stack_object("pow_arr_res", 8);
        self.emit(abi::store_u64(&res, abi::stack_pointer(), res_slot));
        self.reset_temporary_registers();
        let index = self.allocate_register()?;
        self.emit(abi::load_u64(&index, abi::stack_pointer(), index_slot));
        let off = self.allocate_register()?;
        self.emit(abi::shift_left_immediate(&off, &index, 3));
        let obase = self.allocate_register()?;
        self.emit(abi::load_u64(&obase, abi::stack_pointer(), odata_slot));
        self.emit(abi::add_registers(&obase, &obase, &off));
        let res = self.allocate_register()?;
        self.emit(abi::load_u64(&res, abi::stack_pointer(), res_slot));
        self.emit(abi::store_u64(&res, &obase, 0));
        self.emit(abi::add_immediate(&index, &index, 1));
        self.emit(abi::store_u64(&index, abi::stack_pointer(), index_slot));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&loop_done));

        self.reset_temporary_registers();
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: "List OF Float".to_string(),
            location: result,
            text,
        })
    }

    #[allow(clippy::too_many_arguments)]
    /// Set `smask` (0 or the sign bit) per the negative-base rule, or jump to
    /// `ret_nan` (x<0, non-integer y). For x>=0 leaves smask as 0. `x`/`y` are the
    /// value homes holding the inputs.
    fn emit_pow_yisint(
        &mut self,
        x: &str,
        y: &str,
        xs: &str,
        xm: &str,
        xt: &str,
        smask: &str,
        ret_nan: &str,
    ) {
        let done = self.label("pow_yisint_done");
        // x >= 0 -> nothing to do.
        self.emit(abi::float_move_x_from_d(xs, x));
        self.emit(abi::compare_immediate(xs, "0"));
        self.emit(abi::branch_ge(&done));

        // |y| >= 2^53 -> integer & even -> smask stays 0.
        self.emit(abi::float_move_x_from_d(xs, y));
        self.emit(abi::move_immediate(xm, "Integer", ABS_MASK));
        self.emit(abi::and_registers(xs, xs, xm)); // |y| bits
        self.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[0], xs)); // |y|
        self.emit_f64_const(abi::FP_SCRATCH[1], xt, TWO53);
        self.emit(abi::float_subtract_d(
            abi::FP_SCRATCH[2],
            abi::FP_SCRATCH[0],
            abi::FP_SCRATCH[1],
        ));
        self.emit(abi::float_compare_zero_d(abi::FP_SCRATCH[2]));
        self.emit(abi::branch_ge(&done)); // |y| >= 2^53 -> even integer

        // y integer? trunc(y) == y.
        self.emit(abi::float_move_x_from_d(xs, y));
        self.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[0], xs)); // y
        self.emit(abi::float_convert_to_signed_x(xt, abi::FP_SCRATCH[0])); // trunc(y) (i64)
        self.emit(abi::signed_convert_to_float_d(abi::FP_SCRATCH[1], xt)); // (double)trunc
        self.emit(abi::float_move_x_from_d(xm, abi::FP_SCRATCH[1]));
        self.emit(abi::compare_registers(xm, xs));
        self.emit(abi::branch_ne(ret_nan)); // non-integer -> NaN
                                            // odd? trunc & 1.
        self.emit(abi::move_immediate(xm, "Integer", "1"));
        self.emit(abi::and_registers(xt, xt, xm));
        self.emit(abi::compare_immediate(xt, "0"));
        self.emit(abi::branch_eq(&done)); // even
        self.emit(abi::move_immediate(smask, "Integer", SIGN_BIT));
        self.emit(abi::label(&done));
    }

    /// log2(ax) computed to extra precision into (t1, t2); fdlibm e_pow.c.
    fn emit_pow_log2(&mut self, s: &PowHomes, xs: &str, xm: &str, xt: &str, xu: &str, nexp: &str) {
        // n_exp = exponent of ax; reduce ax into [1,2).
        // subnormal: hi32(ax) < 0x00100000 -> ax *= 2^53, n_exp -= 53.
        self.emit(abi::move_immediate(nexp, "Integer", "0"));
        self.emit(abi::float_move_x_from_d(xs, s.ax));
        self.emit(abi::shift_right_immediate(xt, xs, 32)); // hi32(ax)
        let not_sub = self.label("pow_log_notsub");
        self.emit(abi::move_immediate(xm, "Integer", "1048576")); // 0x00100000
        self.emit(abi::compare_registers(xt, xm));
        self.emit(abi::branch_ge(&not_sub));
        self.pld(abi::FP_SCRATCH[0], s.ax);
        self.emit_f64_const(abi::FP_SCRATCH[1], xt, TWO53);
        self.emit(abi::float_multiply_d(
            abi::FP_SCRATCH[0],
            abi::FP_SCRATCH[0],
            abi::FP_SCRATCH[1],
        ));
        self.pst(abi::FP_SCRATCH[0], s.ax);
        self.emit(abi::move_immediate(xm, "Integer", "53"));
        self.emit(abi::subtract_registers(nexp, nexp, xm));
        self.emit(abi::label(&not_sub));

        // hi32(ax); n_exp += (hi32>>20) - 1023; j = hi32 & 0xfffff; hi32 = j|0x3ff00000.
        self.emit(abi::float_move_x_from_d(xs, s.ax));
        self.emit(abi::shift_right_immediate(xt, xs, 32)); // hi32
        self.emit(abi::shift_right_immediate(xm, xt, 20));
        self.emit(abi::move_immediate(xu, "Integer", "1023"));
        self.emit(abi::subtract_registers(xm, xm, xu));
        self.emit(abi::add_registers(nexp, nexp, xm)); // n_exp += exp-1023
        self.emit(abi::move_immediate(xm, "Integer", "1048575")); // 0xfffff
        self.emit(abi::and_registers(xt, xt, xm)); // j (xt)

        // kk segment; adjust j / n_exp.
        // kk=0 if j<=0x3988E ; kk=1 if j<0xBB67A ; else kk=0, n_exp+=1, hi-=0x100000.
        // We track kk via a register (0/1) and bp/dp via select.
        let kk = xu;
        let seg1 = self.label("pow_log_seg1");
        let seg_done = self.label("pow_log_segdone");
        self.emit(abi::move_immediate(kk, "Integer", "0"));
        self.emit(abi::move_immediate(xm, "Integer", "235662")); // 0x3988E
        self.emit(abi::compare_registers(xt, xm));
        self.emit(abi::branch_le(&seg_done)); // j<=0x3988E -> kk=0
        self.emit(abi::move_immediate(xm, "Integer", "767610")); // 0xBB67A
        self.emit(abi::compare_registers(xt, xm));
        self.emit(abi::branch_lt(&seg1)); // j<0xBB67A -> kk=1
                                          // else: kk=0, n_exp+=1, j-=0x100000  (j is the masked-low; fdlibm subtracts
                                          // 0x00100000 from the high word, i.e. from (j|0x3ff00000) -> exponent down).
        self.emit(abi::move_immediate(xm, "Integer", "1"));
        self.emit(abi::add_registers(nexp, nexp, xm));
        self.emit(abi::move_immediate(xm, "Integer", "1048576")); // 0x100000
        self.emit(abi::subtract_registers(xt, xt, xm)); // j -= 0x100000 (becomes negative-ish high adj)
        self.emit(abi::branch(&seg_done));
        self.emit(abi::label(&seg1));
        self.emit(abi::move_immediate(kk, "Integer", "1"));
        self.emit(abi::label(&seg_done));

        // ax = set_hi(ax, j + 0x3ff00000)  (ADD, not OR, so the third-segment
        // `j -= 0x100000` carries into the exponent field correctly; for the other
        // segments j <= 0xfffff so add == or).
        self.emit(abi::move_immediate(xm, "Integer", "1072693248")); // 0x3ff00000
        self.emit(abi::add_registers(xm, xt, xm)); // new hi word
        self.emit(abi::float_move_x_from_d(xs, s.ax));
        self.emit(abi::move_immediate(xt, "Integer", "4294967295"));
        self.emit(abi::and_registers(xs, xs, xt)); // low word
        self.emit(abi::shift_left_immediate(xm, xm, 32));
        self.emit(abi::or_registers(xs, xs, xm));
        self.emit(abi::float_move_d_from_x(s.ax, xs)); // ax in [1,2)

        // bp[kk], dp_h[kk], dp_l[kk] into homes cs/th/tl via select on kk.
        // u = ax - bp[kk]; v = 1/(ax + bp[kk]); s = u*v
        self.emit_pow_select(kk, BP[0], BP[1], s.cs, xs, xt); // cs = bp[kk]
        self.pop('-', s.uu, s.ax, s.cs); // u = ax - bp
        self.pop('+', s.vv, s.ax, s.cs); // ax + bp
                                         // v = 1/(ax+bp)
        self.pconst(s.tmp, 1.0, xs);
        self.pop('/', s.vv, s.tmp, s.vv);
        self.pop('*', s.sh, s.uu, s.vv); // s = u*v (sh holds s for now)
                                         // s_h = lowzero(s)
        self.plowzero(s.sh, xs, xm);
        // t_h = set_hi(0, ((hi32(ax)>>1)|0x20000000)+0x00080000+(kk<<18))
        self.emit(abi::float_move_x_from_d(xs, s.ax));
        self.emit(abi::shift_right_immediate(xt, xs, 32)); // hi32(ax)
        self.emit(abi::shift_right_immediate(xt, xt, 1));
        self.emit(abi::move_immediate(xm, "Integer", "536870912")); // 0x20000000
        self.emit(abi::or_registers(xt, xt, xm));
        self.emit(abi::move_immediate(xm, "Integer", "524288")); // 0x00080000
        self.emit(abi::add_registers(xt, xt, xm));
        self.emit(abi::shift_left_immediate(xm, kk, 18)); // kk<<18
        self.emit(abi::add_registers(xt, xt, xm));
        self.emit(abi::shift_left_immediate(xt, xt, 32)); // into high word
        self.emit(abi::float_move_d_from_x(s.th, xt)); // t_h
                                                       // t_l = ax - (t_h - bp[kk])
        self.pop('-', s.tmp, s.th, s.cs); // t_h - bp
        self.pop('-', s.tl, s.ax, s.tmp); // t_l
                                          // s_l = v*((u - s_h*t_h) - s_h*t_l)
        self.pop('*', s.tmp, s.sh, s.th); // s_h*t_h
        self.pop('-', s.tmp, s.uu, s.tmp); // u - s_h*t_h
        self.pop('*', s.zz, s.sh, s.tl); // s_h*t_l
        self.pop('-', s.tmp, s.tmp, s.zz);
        self.pop('*', s.sl, s.vv, s.tmp); // s_l
                                          // store s (the full u*v) for use as `s` later; recompute: s = sh? we need ss=u*v.
        self.pop('*', s.zz, s.uu, s.vv); // ss = u*v  (full)
                                         // s2 = ss*ss
        self.pop('*', s.s2, s.zz, s.zz);
        // r = s2*s2*poly_L(s2) + s_l*(s_h+ss)
        self.ppoly(s.s2, &[L1, L2, L3, L4, L5, L6], s.rr, xs); // poly in s2
        self.pop('*', s.tmp, s.s2, s.s2); // s2*s2
        self.pop('*', s.rr, s.tmp, s.rr); // s2*s2*poly
        self.pop('+', s.tmp, s.sh, s.zz); // s_h + ss
        self.pop('*', s.tmp, s.sl, s.tmp); // s_l*(s_h+ss)
        self.pop('+', s.rr, s.rr, s.tmp); // r
                                          // s2 = s_h*s_h
        self.pop('*', s.s2, s.sh, s.sh);
        // t_h = lowzero(3 + s2 + r)
        self.pconst(s.tmp, 3.0, xs);
        self.pop('+', s.tmp, s.tmp, s.s2);
        self.pop('+', s.th, s.tmp, s.rr);
        self.plowzero(s.th, xs, xm);
        // t_l = r - ((t_h - 3) - s2)
        self.pconst(s.tmp, 3.0, xs);
        self.pop('-', s.tmp, s.th, s.tmp); // t_h - 3
        self.pop('-', s.tmp, s.tmp, s.s2); // (t_h-3)-s2
        self.pop('-', s.tl, s.rr, s.tmp); // t_l
                                          // u = s_h*t_h ; v = s_l*t_h + t_l*ss
        self.pop('*', s.uu, s.sh, s.th);
        self.pop('*', s.tmp, s.sl, s.th);
        self.pop('*', s.vv, s.tl, s.zz);
        self.pop('+', s.vv, s.tmp, s.vv);
        // p_h = lowzero(u+v) ; p_l = v - (p_h - u)
        self.pop('+', s.ph, s.uu, s.vv);
        self.plowzero(s.ph, xs, xm);
        self.pop('-', s.tmp, s.ph, s.uu);
        self.pop('-', s.pl, s.vv, s.tmp);
        // z_h = cp_h*p_h ; z_l = cp_l*p_h + p_l*cp + dp_l[kk]
        self.pconst(s.cs, CP_H, xs);
        self.pop('*', s.zh, s.cs, s.ph);
        self.pconst(s.cs, CP_L, xs);
        self.pop('*', s.tmp, s.cs, s.ph); // cp_l*p_h
        self.pconst(s.cs, CP, xs);
        self.pop('*', s.zz, s.pl, s.cs); // p_l*cp
        self.pop('+', s.tmp, s.tmp, s.zz);
        self.emit_pow_select(kk, DP_L[0], DP_L[1], s.cs, xs, xt); // dp_l[kk]
        self.pop('+', s.zl, s.tmp, s.cs); // z_l
                                          // t = (double)n_exp
        self.emit(abi::signed_convert_to_float_d(abi::FP_SCRATCH[0], nexp));
        self.pst(abi::FP_SCRATCH[0], s.tmp); // tmp = t
                                             // t1 = lowzero(((z_h+z_l)+dp_h[kk])+t)
        self.pop('+', s.t1, s.zh, s.zl);
        self.emit_pow_select(kk, DP_H[0], DP_H[1], s.cs, xs, xt); // dp_h[kk]
        self.pop('+', s.t1, s.t1, s.cs);
        self.pop('+', s.t1, s.t1, s.tmp);
        self.plowzero(s.t1, xs, xm);
        // t2 = z_l - (((t1 - t) - dp_h[kk]) - z_h)
        self.pop('-', s.zz, s.t1, s.tmp); // t1 - t
        self.pop('-', s.zz, s.zz, s.cs); // - dp_h[kk]  (cs still dp_h[kk])
        self.pop('-', s.zz, s.zz, s.zh); // - z_h
        self.pop('-', s.t2, s.zl, s.zz); // t2
    }

    /// 2**(p_h + p_l) into `zz`; fdlibm e_pow.c. `s.ph`/`s.pl` are p_h/p_l.
    fn emit_pow_exp2(&mut self, s: &PowHomes, xs: &str, xm: &str, xt: &str, xu: &str, nbit: &str) {
        // j = hi32(z) where z = p_h + p_l (recompute) ; i = j & 0x7fffffff ; k=(i>>20)-0x3ff
        self.pop('+', s.zz, s.pl, s.ph); // z
        self.emit(abi::float_move_x_from_d(xs, s.zz));
        self.emit(abi::move_immediate(xm, "Integer", "32"));
        self.emit(abi::arithmetic_shift_right_variable(xt, xs, xm)); // j = hi32(z) signed
        self.emit(abi::move_immediate(xm, "Integer", "2147483647"));
        self.emit(abi::and_registers(xu, xt, xm)); // i = j & 0x7fffffff
        self.emit(abi::move_immediate(nbit, "Integer", "0")); // n = 0
                                                              // if i > 0x3fe00000: compute n and p_h -= t
        let no_round = self.label("pow_exp_noround");
        self.emit(abi::move_immediate(xm, "Integer", "1071644672")); // 0x3fe00000
        self.emit(abi::compare_registers(xu, xm));
        self.emit(abi::branch_le(&no_round));
        // k = (i>>20) - 0x3ff
        let kreg = xm;
        self.emit(abi::shift_right_immediate(kreg, xu, 20));
        self.emit(abi::move_immediate(xs, "Integer", "1023"));
        self.emit(abi::subtract_registers(kreg, kreg, xs)); // k
                                                            // n = j + (0x00100000 >> (k+1))
        self.emit(abi::move_immediate(xs, "Integer", "1048576")); // 0x100000
        self.emit(abi::move_immediate(nbit, "Integer", "1"));
        self.emit(abi::add_registers(nbit, kreg, nbit)); // k+1
        self.emit(abi::shift_right_variable(xs, xs, nbit)); // 0x100000>>(k+1)
        self.emit(abi::add_registers(nbit, xt, xs)); // n = j + that  (xt = j)
                                                     // k = ((n & 0x7fffffff) >> 20) - 0x3ff
        self.emit(abi::move_immediate(xs, "Integer", "2147483647"));
        self.emit(abi::and_registers(xs, nbit, xs));
        self.emit(abi::shift_right_immediate(xs, xs, 20));
        self.emit(abi::move_immediate(kreg, "Integer", "1023"));
        self.emit(abi::subtract_registers(kreg, xs, kreg)); // new k
                                                            // t = set_hi(0, n & ~(0x000fffff >> k))
        self.emit(abi::move_immediate(xs, "Integer", "1048575")); // 0xfffff
        self.emit(abi::shift_right_variable(xs, xs, kreg)); // 0xfffff>>k
        self.emit(abi::bitwise_not(xs, xs));
        self.emit(abi::and_registers(xs, nbit, xs)); // n & ~(...)
        self.emit(abi::shift_left_immediate(xs, xs, 32));
        self.emit(abi::float_move_d_from_x(s.tmp, xs)); // t
                                                        // n = ((n & 0xfffff) | 0x100000) >> (20 - k)
        self.emit(abi::move_immediate(xs, "Integer", "1048575"));
        self.emit(abi::and_registers(xs, nbit, xs)); // n & 0xfffff
        self.emit(abi::move_immediate(xu, "Integer", "1048576"));
        self.emit(abi::or_registers(xs, xs, xu)); // | 0x100000
        self.emit(abi::move_immediate(xu, "Integer", "20"));
        self.emit(abi::subtract_registers(xu, xu, kreg)); // 20 - k
        self.emit(abi::shift_right_variable(nbit, xs, xu)); // n
                                                            // if j < 0: n = -n   (xt holds j)
        let n_pos = self.label("pow_exp_npos");
        self.emit(abi::compare_immediate(xt, "0"));
        self.emit(abi::branch_ge(&n_pos));
        self.emit(abi::move_immediate(xs, "Integer", "0"));
        self.emit(abi::subtract_registers(nbit, xs, nbit));
        self.emit(abi::label(&n_pos));
        // p_h -= t
        self.pop('-', s.ph, s.ph, s.tmp);
        self.emit(abi::label(&no_round));

        // t = lowzero(p_l + p_h)
        self.pop('+', s.tmp, s.pl, s.ph);
        self.plowzero(s.tmp, xs, xm);
        // u = t*lg2_h ; v = (p_l-(t-p_h))*lg2 + t*lg2_l
        self.pconst(s.cs, LG2_H, xs);
        self.pop('*', s.uu, s.tmp, s.cs);
        self.pop('-', s.zz, s.tmp, s.ph); // t - p_h
        self.pop('-', s.zz, s.pl, s.zz); // p_l - (t - p_h)
        self.pconst(s.cs, LG2, xs);
        self.pop('*', s.zz, s.zz, s.cs);
        self.pconst(s.cs, LG2_L, xs);
        self.pop('*', s.zl, s.tmp, s.cs); // t*lg2_l
        self.pop('+', s.vv, s.zz, s.zl); // v
                                         // z = u + v ; w = v - (z - u)
        self.pop('+', s.zz, s.uu, s.vv);
        self.pop('-', s.tmp, s.zz, s.uu); // z - u
        self.pop('-', s.ww, s.vv, s.tmp); // w
                                          // t = z*z ; t1 = z - t*P(t)
        self.pop('*', s.tmp, s.zz, s.zz);
        self.ppoly(s.tmp, &[P1, P2, P3, P4, P5], s.t1, xs); // P(t)
        self.pop('*', s.t1, s.tmp, s.t1); // t*P
        self.pop('-', s.t1, s.zz, s.t1); // z - t*P
                                         // r = (z*t1)/(t1-2) - (w + z*w)
        self.pop('*', s.tmp, s.zz, s.t1); // z*t1
        self.pconst(s.cs, 2.0, xs);
        self.pop('-', s.t2, s.t1, s.cs); // t1 - 2
        self.pop('/', s.tmp, s.tmp, s.t2); // (z*t1)/(t1-2)
        self.pop('*', s.zl, s.zz, s.ww); // z*w
        self.pop('+', s.zl, s.ww, s.zl); // w + z*w
        self.pop('-', s.rr, s.tmp, s.zl); // r
                                          // z = 1 - (r - z)
        self.pop('-', s.tmp, s.rr, s.zz); // r - z
        self.pconst(s.cs, 1.0, xs);
        self.pop('-', s.zz, s.cs, s.tmp); // z = 1 - (r-z)
                                          // j = hi32(z) + (n<<20) ; if (j>>20)<=0 -> z=scalbn(z,n) else set_hi(z,j)
        self.emit(abi::float_move_x_from_d(xs, s.zz));
        self.emit(abi::shift_right_immediate(xt, xs, 32)); // hi32(z) (signed)
        self.emit(abi::shift_left_immediate(xm, nbit, 20)); // n<<20
        self.emit(abi::add_registers(xt, xt, xm)); // j
        let subnormal = self.label("pow_exp_subnormal");
        let scaled = self.label("pow_exp_scaled");
        // (j>>20) <= 0 (signed) -> subnormal scalbn path. `j` mirrors fdlibm's
        // signed 32-bit int, so the exponent-field test must be an *arithmetic*
        // shift: for n <= -1023 `j` is negative and a logical LSR would read a
        // huge positive value and wrongly take the normal (set_hi) branch,
        // constructing a sign-bit-set / huge-exponent double (bug-129).
        self.emit(abi::arithmetic_shift_right_immediate(xm, xt, 20)); // ASR: signed j>>20
        self.emit(abi::compare_immediate(xm, "0"));
        self.emit(abi::branch_le(&subnormal));
        // normal: set_hi(z, j)
        self.emit(abi::float_move_x_from_d(xs, s.zz));
        self.emit(abi::move_immediate(xm, "Integer", "4294967295"));
        self.emit(abi::and_registers(xs, xs, xm)); // low word
        self.emit(abi::move_immediate(xm, "Integer", "4294967295"));
        self.emit(abi::and_registers(xt, xt, xm)); // j low 32
        self.emit(abi::shift_left_immediate(xt, xt, 32));
        self.emit(abi::or_registers(xs, xs, xt));
        self.emit(abi::float_move_d_from_x(s.zz, xs));
        self.emit(abi::branch(&scaled));
        self.emit(abi::label(&subnormal));
        // z = scalbn(z, n): z *= 2^n via constructing 2^n (n small/negative here).
        self.emit_pow_scalbn(s.zz, nbit, xs, xm, xt, xu);
        self.emit(abi::label(&scaled));
    }

    /// `home = select(kk!=0 ? b : a)` as an f64 constant.
    fn emit_pow_select(&mut self, kk: &str, a: f64, b: f64, home: &str, xs: &str, _xt: &str) {
        let pick_b = self.label("pow_sel_b");
        let done = self.label("pow_sel_done");
        self.emit(abi::compare_immediate(kk, "0"));
        self.emit(abi::branch_ne(&pick_b));
        self.emit(abi::move_immediate(xs, "Integer", &a.to_bits().to_string()));
        self.emit(abi::branch(&done));
        self.emit(abi::label(&pick_b));
        self.emit(abi::move_immediate(xs, "Integer", &b.to_bits().to_string()));
        self.emit(abi::label(&done));
        self.emit(abi::float_move_d_from_x(home, xs));
    }

    /// `home = scalbn(z, n)` — a faithful port of fdlibm `scalbn` for the pow
    /// subnormal output path. `home` (z) is always a *normal, positive* double
    /// near 1.0 (the exp2 mantissa `1-(r-z)`), so the zero/subnormal/Inf/NaN
    /// *input* cases of the library routine are elided. A subnormal *result* is
    /// produced with the two-step 2**54 compensation (bias the exponent up by 54
    /// so it stays a valid normal field, then multiply by 2**-54 for a correctly
    /// rounded subnormal) instead of the old `(1023+n)<<52` factor, which is the
    /// bit pattern 0 (= +0.0) at n == -1023 and a malformed sign/exponent below
    /// it (bug-129).
    fn emit_pow_scalbn(&mut self, home: &str, n: &str, xs: &str, xm: &str, xt: &str, xu: &str) {
        self.emit(abi::float_move_x_from_d(xs, home)); // xs = bits(z)
        self.emit(abi::shift_right_immediate(xt, xs, 32)); // hx = hi32(z)
                                                           // k = ((hx & 0x7ff00000) >> 20) + n   (biased exponent of z)
        self.emit(abi::move_immediate(xm, "Integer", "2146435072")); // 0x7ff00000
        self.emit(abi::and_registers(xu, xt, xm));
        self.emit(abi::shift_right_immediate(xu, xu, 20));
        self.emit(abi::add_registers(xu, xu, n)); // k (signed)
                                                  // keep = hx & 0x800fffff  (sign bit + high 20 mantissa bits)
        self.emit(abi::move_immediate(xm, "Integer", "2148532223")); // 0x800fffff
        self.emit(abi::and_registers(xt, xt, xm)); // keep

        let sub = self.label("pow_scalbn_sub");
        let underflow = self.label("pow_scalbn_uf");
        let done = self.label("pow_scalbn_done");

        // k <= 0 -> subnormal / underflow region; else a normal result.
        self.emit(abi::compare_immediate(xu, "0"));
        self.emit(abi::branch_le(&sub));
        // normal: set_hi(z, keep | (k<<20))
        self.emit(abi::shift_left_immediate(xm, xu, 20));
        self.emit(abi::or_registers(xm, xt, xm)); // new hi word
        self.emit(abi::move_immediate(xu, "Integer", "4294967295"));
        self.emit(abi::and_registers(xs, xs, xu)); // low32(z)
        self.emit(abi::shift_left_immediate(xm, xm, 32));
        self.emit(abi::or_registers(xs, xs, xm));
        self.emit(abi::float_move_d_from_x(home, xs));
        self.emit(abi::branch(&done));

        self.emit(abi::label(&sub));
        // k += 54; if still <= 0 the result is a genuine underflow -> flush to 0.
        self.emit(abi::move_immediate(xm, "Integer", "54"));
        self.emit(abi::add_registers(xu, xu, xm)); // k += 54
        self.emit(abi::compare_immediate(xu, "0"));
        self.emit(abi::branch_le(&underflow));
        // subnormal: set_hi(z, keep | (k<<20)); z *= 2**-54
        self.emit(abi::shift_left_immediate(xm, xu, 20));
        self.emit(abi::or_registers(xm, xt, xm));
        self.emit(abi::move_immediate(xu, "Integer", "4294967295"));
        self.emit(abi::and_registers(xs, xs, xu));
        self.emit(abi::shift_left_immediate(xm, xm, 32));
        self.emit(abi::or_registers(xs, xs, xm));
        self.emit(abi::float_move_d_from_x(home, xs));
        self.pld(abi::FP_SCRATCH[0], home);
        self.emit(abi::move_immediate(xt, "Integer", TWOM54_BITS)); // 2**-54
        self.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[1], xt));
        self.emit(abi::float_multiply_d(
            abi::FP_SCRATCH[0],
            abi::FP_SCRATCH[0],
            abi::FP_SCRATCH[1],
        ));
        self.pst(abi::FP_SCRATCH[0], home);
        self.emit(abi::branch(&done));

        self.emit(abi::label(&underflow));
        // underflow to 0 (the result sign is applied by the caller's sign mask).
        self.emit(abi::move_immediate(xs, "Integer", "0"));
        self.emit(abi::float_move_d_from_x(home, xs));

        self.emit(abi::label(&done));
    }
}
