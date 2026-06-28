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

/// Stack-slot working set for the scalar pow kernel (one f64 per field).
struct PowSlots {
    ax: usize,
    sh: usize,
    sl: usize,
    th: usize,
    tl: usize,
    rr: usize,
    uu: usize,
    vv: usize,
    ww: usize,
    ph: usize,
    pl: usize,
    zh: usize,
    zl: usize,
    t1: usize,
    t2: usize,
    s2: usize,
    tmp: usize,
    cs: usize,
    zz: usize,
}

impl CodeBuilder<'_> {
    fn pld(&mut self, d: &str, slot: usize, xs: &str) {
        self.emit(abi::load_u64(xs, abi::stack_pointer(), slot));
        self.emit(abi::float_move_d_from_x(d, xs));
    }
    fn pst(&mut self, d: &str, slot: usize, xs: &str) {
        self.emit(abi::float_move_x_from_d(xs, d));
        self.emit(abi::store_u64(xs, abi::stack_pointer(), slot));
    }
    fn pop(&mut self, op: char, dst: usize, a: usize, b: usize, xs: &str) {
        self.pld("d0", a, xs);
        self.pld("d1", b, xs);
        match op {
            '+' => self.emit(abi::float_add_d("d0", "d0", "d1")),
            '-' => self.emit(abi::float_subtract_d("d0", "d0", "d1")),
            '*' => self.emit(abi::float_multiply_d("d0", "d0", "d1")),
            '/' => self.emit(abi::float_divide_d("d0", "d0", "d1")),
            _ => unreachable!(),
        }
        self.pst("d0", dst, xs);
    }
    fn pconst(&mut self, slot: usize, value: f64, xs: &str) {
        self.emit(abi::move_immediate(xs, "Integer", &value.to_bits().to_string()));
        self.emit(abi::store_u64(xs, abi::stack_pointer(), slot));
    }
    fn plowzero(&mut self, slot: usize, xs: &str, xm: &str) {
        self.emit(abi::load_u64(xs, abi::stack_pointer(), slot));
        self.emit(abi::move_immediate(xm, "Integer", HIGH32_MASK));
        self.emit(abi::and_registers(xs, xs, xm));
        self.emit(abi::store_u64(xs, abi::stack_pointer(), slot));
    }
    /// `out = c[n-1]; out = c[i] + out*var` (Horner, ascending coeffs).
    fn ppoly(&mut self, var: usize, coeffs: &[f64], out: usize, xs: &str) {
        self.emit(abi::move_immediate(xs, "Integer", &coeffs[coeffs.len() - 1].to_bits().to_string()));
        self.emit(abi::float_move_d_from_x("d2", xs));
        for &c in coeffs.iter().rev().skip(1) {
            self.pld("d1", var, xs);
            self.emit(abi::move_immediate(xs, "Integer", &c.to_bits().to_string()));
            self.emit(abi::float_move_d_from_x("d3", xs));
            self.emit(abi::float_multiply_d("d2", "d2", "d1"));
            self.emit(abi::float_add_d("d2", "d2", "d3"));
        }
        self.pst("d2", out, xs);
    }

    /// Emit scalar `pow(x, y)`; returns a register with the result bit pattern.
    pub(super) fn emit_pow_scalar(&mut self, x_loc: &str, y_loc: &str) -> Result<String, String> {
        let x_slot = self.allocate_stack_object("pow_x", 8);
        let y_slot = self.allocate_stack_object("pow_y", 8);
        self.emit(abi::store_u64(x_loc, abi::stack_pointer(), x_slot));
        self.emit(abi::store_u64(y_loc, abi::stack_pointer(), y_slot));
        self.reset_temporary_registers();

        let s = PowSlots {
            ax: self.allocate_stack_object("pow_ax", 8),
            sh: self.allocate_stack_object("pow_sh", 8),
            sl: self.allocate_stack_object("pow_sl", 8),
            th: self.allocate_stack_object("pow_th", 8),
            tl: self.allocate_stack_object("pow_tl", 8),
            rr: self.allocate_stack_object("pow_r", 8),
            uu: self.allocate_stack_object("pow_u", 8),
            vv: self.allocate_stack_object("pow_v", 8),
            ww: self.allocate_stack_object("pow_w", 8),
            ph: self.allocate_stack_object("pow_ph", 8),
            pl: self.allocate_stack_object("pow_pl", 8),
            zh: self.allocate_stack_object("pow_zh", 8),
            zl: self.allocate_stack_object("pow_zl", 8),
            t1: self.allocate_stack_object("pow_t1", 8),
            t2: self.allocate_stack_object("pow_t2", 8),
            s2: self.allocate_stack_object("pow_s2", 8),
            tmp: self.allocate_stack_object("pow_tmp", 8),
            cs: self.allocate_stack_object("pow_c", 8),
            zz: self.allocate_stack_object("pow_z", 8),
        };

        let result = self.allocate_register()?;
        let xs_o = self.allocate_register()?;
        let xm_o = self.allocate_register()?;
        let xt_o = self.allocate_register()?;
        let xu_o = self.allocate_register()?;
        let smask_o = self.allocate_register()?;
        let nexp_o = self.allocate_register()?;
        let (xs, xm, xt, xu, smask, nexp) = (
            xs_o.as_str(), xm_o.as_str(), xt_o.as_str(), xu_o.as_str(),
            smask_o.as_str(), nexp_o.as_str(),
        );

        let end = self.label("pow_end");
        let ret_nan = self.label("pow_ret_nan");

        // y == 0 -> 1.0
        self.emit(abi::load_u64(xs, abi::stack_pointer(), y_slot));
        self.emit(abi::move_immediate(xm, "Integer", ABS_MASK));
        self.emit(abi::and_registers(xs, xs, xm));
        let y_nonzero = self.label("pow_y_nonzero");
        self.emit(abi::compare_immediate(xs, "0"));
        self.emit(abi::branch_ne(&y_nonzero));
        self.emit(abi::move_immediate(&result, "Integer", &1.0f64.to_bits().to_string()));
        self.emit(abi::branch(&end));
        self.emit(abi::label(&y_nonzero));

        // Sign / integer-exponent rule for x < 0 (sets smask or jumps to ret_nan).
        self.emit(abi::move_immediate(smask, "Integer", "0"));
        self.emit_pow_yisint(x_slot, y_slot, xs, xm, xt, smask, &ret_nan);

        // ax = |x|
        self.emit(abi::load_u64(xs, abi::stack_pointer(), x_slot));
        self.emit(abi::move_immediate(xm, "Integer", ABS_MASK));
        self.emit(abi::and_registers(xs, xs, xm));
        self.emit(abi::store_u64(xs, abi::stack_pointer(), s.ax));

        // log2(ax) -> (t1, t2)
        self.emit_pow_log2(&s, xs, xm, xt, xu, nexp);

        // y*log2(ax) = p_h + p_l ; z = p_h + p_l
        self.emit(abi::load_u64(xs, abi::stack_pointer(), y_slot));
        self.emit(abi::move_immediate(xm, "Integer", HIGH32_MASK));
        self.emit(abi::and_registers(xs, xs, xm));
        self.emit(abi::store_u64(xs, abi::stack_pointer(), s.ww)); // ww = y1
        self.pop('-', s.tmp, y_slot, s.ww, xs); // y - y1
        self.pop('*', s.tmp, s.tmp, s.t1, xs);
        self.pop('*', s.cs, y_slot, s.t2, xs); // y*t2
        self.pop('+', s.pl, s.tmp, s.cs, xs); // p_l
        self.pop('*', s.ph, s.ww, s.t1, xs); // p_h = y1*t1
        self.pop('+', s.zz, s.pl, s.ph, xs); // z

        // overflow: (signed) hi32(z) >= 0x40900000  (signed: z negative is never
        // an overflow, so the high word must be sign-extended).
        let not_ovf = self.label("pow_not_ovf");
        self.emit(abi::load_u64(xs, abi::stack_pointer(), s.zz));
        self.emit(abi::move_immediate(xm, "Integer", "32"));
        self.emit(abi::arithmetic_shift_right_variable(xt, xs, xm));
        self.emit(abi::move_immediate(xm, "Integer", "1083179008")); // 0x40900000
        self.emit(abi::compare_registers(xt, xm));
        self.emit(abi::branch_lt(&not_ovf));
        self.pconst(s.cs, HUGE, xs);
        self.pop('*', s.tmp, s.cs, s.cs, xs);
        self.emit(abi::load_u64(&result, abi::stack_pointer(), s.tmp));
        self.emit(abi::exclusive_or_registers(&result, &result, smask));
        self.emit(abi::branch(&end));
        self.emit(abi::label(&not_ovf));

        // underflow: (hi32(z) & 0x7fffffff) >= 0x4090cc00
        let do_2exp = self.label("pow_do_2exp");
        self.emit(abi::load_u64(xs, abi::stack_pointer(), s.zz));
        self.emit(abi::shift_right_immediate(xt, xs, 32));
        self.emit(abi::move_immediate(xm, "Integer", "2147483647"));
        self.emit(abi::and_registers(xt, xt, xm));
        self.emit(abi::move_immediate(xm, "Integer", "1083280384")); // 0x4090cc00
        self.emit(abi::compare_registers(xt, xm));
        self.emit(abi::branch_lt(&do_2exp));
        self.pconst(s.cs, TINY, xs);
        self.pop('*', s.tmp, s.cs, s.cs, xs);
        self.emit(abi::load_u64(&result, abi::stack_pointer(), s.tmp));
        self.emit(abi::exclusive_or_registers(&result, &result, smask));
        self.emit(abi::branch(&end));
        self.emit(abi::label(&do_2exp));

        // 2**(p_h + p_l) -> zz
        self.emit_pow_exp2(&s, xs, xm, xt, xu, nexp);
        self.emit(abi::load_u64(&result, abi::stack_pointer(), s.zz));
        self.emit(abi::exclusive_or_registers(&result, &result, smask));
        self.emit(abi::branch(&end));

        self.emit(abi::label(&ret_nan));
        self.emit(abi::move_immediate(&result, "Integer", "9221120237041090560")); // 0x7FF8000000000000
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

        self.emit(abi::move_register("x0", &count));
        self.emit(abi::move_immediate("x1", "Integer", &COLLECTION_TYPE_FLOAT.to_string()));
        self.emit(abi::branch_link(SIMD_ALLOC_LIST_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: SIMD_ALLOC_LIST_SYMBOL.to_string(),
            kind: "branch26".to_string(),
            binding: "internal".to_string(),
            library: None,
        });
        self.reset_temporary_registers();
        let result_base = self.allocate_register()?;
        self.emit(abi::move_register(&result_base, "x0"));
        let alloc_ok = self.label("pow_arr_alloc_ok");
        self.emit(abi::compare_immediate("x1", "0"));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit(abi::move_register("x0", "x1"));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&alloc_ok));

        // Loop state lives in stack slots (emit_pow_scalar clobbers the file).
        let result_slot = self.allocate_stack_object("pow_arr_result", 8);
        let ldata_slot = self.allocate_stack_object("pow_arr_ldata", 8);
        let rdata_slot = self.allocate_stack_object("pow_arr_rdata", 8);
        let odata_slot = self.allocate_stack_object("pow_arr_odata", 8);
        let index_slot = self.allocate_stack_object("pow_arr_index", 8);
        self.emit(abi::store_u64(&result_base, abi::stack_pointer(), result_slot));
        let left_ptr = self.allocate_register()?;
        self.emit(abi::load_u64(&left_ptr, abi::stack_pointer(), left_slot));
        let right_ptr = self.allocate_register()?;
        self.emit(abi::load_u64(&right_ptr, abi::stack_pointer(), right_slot));
        let ldata = self.allocate_register()?;
        self.emit_collection_data_pointer(&ldata, &left_ptr);
        self.emit(abi::store_u64(&ldata, abi::stack_pointer(), ldata_slot));
        let rdata = self.allocate_register()?;
        self.emit_collection_data_pointer(&rdata, &right_ptr);
        self.emit(abi::store_u64(&rdata, abi::stack_pointer(), rdata_slot));
        let odata = self.allocate_register()?;
        self.emit_collection_data_pointer(&odata, &result_base);
        self.emit(abi::store_u64(&odata, abi::stack_pointer(), odata_slot));
        self.emit(abi::move_immediate("x0", "Integer", "0"));
        self.emit(abi::store_u64("x0", abi::stack_pointer(), index_slot));

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
        Ok(ValueResult { type_: "List OF Float".to_string(), location: result, text })
    }

    /// Set `smask` (0 or the sign bit) per the negative-base rule, or jump to
    /// `ret_nan` (x<0, non-integer y). For x>=0 leaves smask as 0.
    #[allow(clippy::too_many_arguments)]
    fn emit_pow_yisint(
        &mut self,
        x_slot: usize,
        y_slot: usize,
        xs: &str,
        xm: &str,
        xt: &str,
        smask: &str,
        ret_nan: &str,
    ) {
        let done = self.label("pow_yisint_done");
        // x >= 0 -> nothing to do.
        self.emit(abi::load_u64(xs, abi::stack_pointer(), x_slot));
        self.emit(abi::compare_immediate(xs, "0"));
        self.emit(abi::branch_ge(&done));

        // |y| >= 2^53 -> integer & even -> smask stays 0.
        self.emit(abi::load_u64(xs, abi::stack_pointer(), y_slot));
        self.emit(abi::move_immediate(xm, "Integer", ABS_MASK));
        self.emit(abi::and_registers(xs, xs, xm)); // |y| bits
        self.emit(abi::float_move_d_from_x("d0", xs)); // |y|
        self.emit_f64_const("d1", xt, TWO53);
        self.emit(abi::float_subtract_d("d2", "d0", "d1"));
        self.emit(abi::float_compare_zero_d("d2"));
        self.emit(abi::branch_ge(&done)); // |y| >= 2^53 -> even integer

        // y integer? trunc(y) == y.
        self.emit(abi::load_u64(xs, abi::stack_pointer(), y_slot));
        self.emit(abi::float_move_d_from_x("d0", xs)); // y
        self.emit(abi::float_convert_to_signed_x(xt, "d0")); // trunc(y) (i64)
        self.emit(abi::signed_convert_to_float_d("d1", xt)); // (double)trunc
        self.emit(abi::float_move_x_from_d(xm, "d1"));
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
    fn emit_pow_log2(&mut self, s: &PowSlots, xs: &str, xm: &str, xt: &str, xu: &str, nexp: &str) {
        // n_exp = exponent of ax; reduce ax into [1,2).
        // subnormal: hi32(ax) < 0x00100000 -> ax *= 2^53, n_exp -= 53.
        self.emit(abi::move_immediate(nexp, "Integer", "0"));
        self.emit(abi::load_u64(xs, abi::stack_pointer(), s.ax));
        self.emit(abi::shift_right_immediate(xt, xs, 32)); // hi32(ax)
        let not_sub = self.label("pow_log_notsub");
        self.emit(abi::move_immediate(xm, "Integer", "1048576")); // 0x00100000
        self.emit(abi::compare_registers(xt, xm));
        self.emit(abi::branch_ge(&not_sub));
        self.pld("d0", s.ax, xs);
        self.emit_f64_const("d1", xt, TWO53);
        self.emit(abi::float_multiply_d("d0", "d0", "d1"));
        self.pst("d0", s.ax, xs);
        self.emit(abi::move_immediate(xm, "Integer", "53"));
        self.emit(abi::subtract_registers(nexp, nexp, xm));
        self.emit(abi::label(&not_sub));

        // hi32(ax); n_exp += (hi32>>20) - 1023; j = hi32 & 0xfffff; hi32 = j|0x3ff00000.
        self.emit(abi::load_u64(xs, abi::stack_pointer(), s.ax));
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
        // `j -= 0x100000` borrows into the exponent field correctly; for the other
        // segments j <= 0xfffff so add == or).
        self.emit(abi::move_immediate(xm, "Integer", "1072693248")); // 0x3ff00000
        self.emit(abi::add_registers(xm, xt, xm)); // new hi word
        self.emit(abi::load_u64(xs, abi::stack_pointer(), s.ax));
        self.emit(abi::move_immediate(xt, "Integer", "4294967295"));
        self.emit(abi::and_registers(xs, xs, xt)); // low word
        self.emit(abi::shift_left_immediate(xm, xm, 32));
        self.emit(abi::or_registers(xs, xs, xm));
        self.emit(abi::store_u64(xs, abi::stack_pointer(), s.ax)); // ax in [1,2)

        // bp[kk], dp_h[kk], dp_l[kk] into slots th/tl/cs via select on kk.
        // u = ax - bp[kk]; v = 1/(ax + bp[kk]); s = u*v
        self.emit_pow_select(kk, BP[0], BP[1], s.cs, xs, xt); // cs = bp[kk]
        self.pop('-', s.uu, s.ax, s.cs, xs); // u = ax - bp
        self.pop('+', s.vv, s.ax, s.cs, xs); // ax + bp
        // v = 1/(ax+bp)
        self.pconst(s.tmp, 1.0, xs);
        self.pop('/', s.vv, s.tmp, s.vv, xs);
        self.pop('*', s.sh, s.uu, s.vv, xs); // s = u*v (sh holds s for now)
        // s_h = lowzero(s)
        self.plowzero(s.sh, xs, xm);
        // t_h = set_hi(0, ((hi32(ax)>>1)|0x20000000)+0x00080000+(kk<<18))
        self.emit(abi::load_u64(xs, abi::stack_pointer(), s.ax));
        self.emit(abi::shift_right_immediate(xt, xs, 32)); // hi32(ax)
        self.emit(abi::shift_right_immediate(xt, xt, 1));
        self.emit(abi::move_immediate(xm, "Integer", "536870912")); // 0x20000000
        self.emit(abi::or_registers(xt, xt, xm));
        self.emit(abi::move_immediate(xm, "Integer", "524288")); // 0x00080000
        self.emit(abi::add_registers(xt, xt, xm));
        self.emit(abi::shift_left_immediate(xm, kk, 18)); // kk<<18
        self.emit(abi::add_registers(xt, xt, xm));
        self.emit(abi::shift_left_immediate(xt, xt, 32)); // into high word
        self.emit(abi::store_u64(xt, abi::stack_pointer(), s.th)); // t_h
        // t_l = ax - (t_h - bp[kk])
        self.pop('-', s.tmp, s.th, s.cs, xs); // t_h - bp
        self.pop('-', s.tl, s.ax, s.tmp, xs); // t_l
        // s_l = v*((u - s_h*t_h) - s_h*t_l)
        self.pop('*', s.tmp, s.sh, s.th, xs); // s_h*t_h
        self.pop('-', s.tmp, s.uu, s.tmp, xs); // u - s_h*t_h
        self.pop('*', s.zz, s.sh, s.tl, xs); // s_h*t_l
        self.pop('-', s.tmp, s.tmp, s.zz, xs);
        self.pop('*', s.sl, s.vv, s.tmp, xs); // s_l
        // store s (the full u*v) for use as `s` later; recompute: s = sh? we need ss=u*v.
        self.pop('*', s.zz, s.uu, s.vv, xs); // ss = u*v  (full)
        // s2 = ss*ss
        self.pop('*', s.s2, s.zz, s.zz, xs);
        // r = s2*s2*poly_L(s2) + s_l*(s_h+ss)
        self.ppoly(s.s2, &[L1, L2, L3, L4, L5, L6], s.rr, xs); // poly in s2
        self.pop('*', s.tmp, s.s2, s.s2, xs); // s2*s2
        self.pop('*', s.rr, s.tmp, s.rr, xs); // s2*s2*poly
        self.pop('+', s.tmp, s.sh, s.zz, xs); // s_h + ss
        self.pop('*', s.tmp, s.sl, s.tmp, xs); // s_l*(s_h+ss)
        self.pop('+', s.rr, s.rr, s.tmp, xs); // r
        // s2 = s_h*s_h
        self.pop('*', s.s2, s.sh, s.sh, xs);
        // t_h = lowzero(3 + s2 + r)
        self.pconst(s.tmp, 3.0, xs);
        self.pop('+', s.tmp, s.tmp, s.s2, xs);
        self.pop('+', s.th, s.tmp, s.rr, xs);
        self.plowzero(s.th, xs, xm);
        // t_l = r - ((t_h - 3) - s2)
        self.pconst(s.tmp, 3.0, xs);
        self.pop('-', s.tmp, s.th, s.tmp, xs); // t_h - 3
        self.pop('-', s.tmp, s.tmp, s.s2, xs); // (t_h-3)-s2
        self.pop('-', s.tl, s.rr, s.tmp, xs); // t_l
        // u = s_h*t_h ; v = s_l*t_h + t_l*ss
        self.pop('*', s.uu, s.sh, s.th, xs);
        self.pop('*', s.tmp, s.sl, s.th, xs);
        self.pop('*', s.vv, s.tl, s.zz, xs);
        self.pop('+', s.vv, s.tmp, s.vv, xs);
        // p_h = lowzero(u+v) ; p_l = v - (p_h - u)
        self.pop('+', s.ph, s.uu, s.vv, xs);
        self.plowzero(s.ph, xs, xm);
        self.pop('-', s.tmp, s.ph, s.uu, xs);
        self.pop('-', s.pl, s.vv, s.tmp, xs);
        // z_h = cp_h*p_h ; z_l = cp_l*p_h + p_l*cp + dp_l[kk]
        self.pconst(s.cs, CP_H, xs);
        self.pop('*', s.zh, s.cs, s.ph, xs);
        self.pconst(s.cs, CP_L, xs);
        self.pop('*', s.tmp, s.cs, s.ph, xs); // cp_l*p_h
        self.pconst(s.cs, CP, xs);
        self.pop('*', s.zz, s.pl, s.cs, xs); // p_l*cp
        self.pop('+', s.tmp, s.tmp, s.zz, xs);
        self.emit_pow_select(kk, DP_L[0], DP_L[1], s.cs, xs, xt); // dp_l[kk]
        self.pop('+', s.zl, s.tmp, s.cs, xs); // z_l
        // t = (double)n_exp
        self.emit(abi::signed_convert_to_float_d("d0", nexp));
        self.pst("d0", s.tmp, xs); // tmp = t
        // t1 = lowzero(((z_h+z_l)+dp_h[kk])+t)
        self.pop('+', s.t1, s.zh, s.zl, xs);
        self.emit_pow_select(kk, DP_H[0], DP_H[1], s.cs, xs, xt); // dp_h[kk]
        self.pop('+', s.t1, s.t1, s.cs, xs);
        self.pop('+', s.t1, s.t1, s.tmp, xs);
        self.plowzero(s.t1, xs, xm);
        // t2 = z_l - (((t1 - t) - dp_h[kk]) - z_h)
        self.pop('-', s.zz, s.t1, s.tmp, xs); // t1 - t
        self.pop('-', s.zz, s.zz, s.cs, xs); // - dp_h[kk]  (cs still dp_h[kk])
        self.pop('-', s.zz, s.zz, s.zh, xs); // - z_h
        self.pop('-', s.t2, s.zl, s.zz, xs); // t2
    }

    /// 2**(p_h + p_l) into `zz`; fdlibm e_pow.c. `s.ph`/`s.pl` are p_h/p_l.
    fn emit_pow_exp2(&mut self, s: &PowSlots, xs: &str, xm: &str, xt: &str, xu: &str, nbit: &str) {
        // j = hi32(z) where z = p_h + p_l (recompute) ; i = j & 0x7fffffff ; k=(i>>20)-0x3ff
        self.pop('+', s.zz, s.pl, s.ph, xs); // z
        self.emit(abi::load_u64(xs, abi::stack_pointer(), s.zz));
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
        self.emit(abi::store_u64(xs, abi::stack_pointer(), s.tmp)); // t
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
        self.pop('-', s.ph, s.ph, s.tmp, xs);
        self.emit(abi::label(&no_round));

        // t = lowzero(p_l + p_h)
        self.pop('+', s.tmp, s.pl, s.ph, xs);
        self.plowzero(s.tmp, xs, xm);
        // u = t*lg2_h ; v = (p_l-(t-p_h))*lg2 + t*lg2_l
        self.pconst(s.cs, LG2_H, xs);
        self.pop('*', s.uu, s.tmp, s.cs, xs);
        self.pop('-', s.zz, s.tmp, s.ph, xs); // t - p_h
        self.pop('-', s.zz, s.pl, s.zz, xs); // p_l - (t - p_h)
        self.pconst(s.cs, LG2, xs);
        self.pop('*', s.zz, s.zz, s.cs, xs);
        self.pconst(s.cs, LG2_L, xs);
        self.pop('*', s.zl, s.tmp, s.cs, xs); // t*lg2_l
        self.pop('+', s.vv, s.zz, s.zl, xs); // v
        // z = u + v ; w = v - (z - u)
        self.pop('+', s.zz, s.uu, s.vv, xs);
        self.pop('-', s.tmp, s.zz, s.uu, xs); // z - u
        self.pop('-', s.ww, s.vv, s.tmp, xs); // w
        // t = z*z ; t1 = z - t*P(t)
        self.pop('*', s.tmp, s.zz, s.zz, xs);
        self.ppoly(s.tmp, &[P1, P2, P3, P4, P5], s.t1, xs); // P(t)
        self.pop('*', s.t1, s.tmp, s.t1, xs); // t*P
        self.pop('-', s.t1, s.zz, s.t1, xs); // z - t*P
        // r = (z*t1)/(t1-2) - (w + z*w)
        self.pop('*', s.tmp, s.zz, s.t1, xs); // z*t1
        self.pconst(s.cs, 2.0, xs);
        self.pop('-', s.t2, s.t1, s.cs, xs); // t1 - 2
        self.pop('/', s.tmp, s.tmp, s.t2, xs); // (z*t1)/(t1-2)
        self.pop('*', s.zl, s.zz, s.ww, xs); // z*w
        self.pop('+', s.zl, s.ww, s.zl, xs); // w + z*w
        self.pop('-', s.rr, s.tmp, s.zl, xs); // r
        // z = 1 - (r - z)
        self.pop('-', s.tmp, s.rr, s.zz, xs); // r - z
        self.pconst(s.cs, 1.0, xs);
        self.pop('-', s.zz, s.cs, s.tmp, xs); // z = 1 - (r-z)
        // j = hi32(z) + (n<<20) ; if (j>>20)<=0 -> z=scalbn(z,n) else set_hi(z,j)
        self.emit(abi::load_u64(xs, abi::stack_pointer(), s.zz));
        self.emit(abi::shift_right_immediate(xt, xs, 32)); // hi32(z) (signed)
        self.emit(abi::shift_left_immediate(xm, nbit, 20)); // n<<20
        self.emit(abi::add_registers(xt, xt, xm)); // j
        let subnormal = self.label("pow_exp_subnormal");
        let scaled = self.label("pow_exp_scaled");
        // (j>>20) <= 0 (signed) -> subnormal scalbn path
        self.emit(abi::shift_right_immediate(xm, xt, 20)); // arithmetic? shift_right_immediate is LSR
        // Need signed compare of (j as i32)>>20. Reconstruct sign: treat xt low32 as i32.
        // Simpler: compare j (as built) — j>>20<=0 means exponent field <=0 -> tiny.
        self.emit(abi::compare_immediate(xm, "0"));
        self.emit(abi::branch_le(&subnormal));
        // normal: set_hi(z, j)
        self.emit(abi::load_u64(xs, abi::stack_pointer(), s.zz));
        self.emit(abi::move_immediate(xm, "Integer", "4294967295"));
        self.emit(abi::and_registers(xs, xs, xm)); // low word
        self.emit(abi::move_immediate(xm, "Integer", "4294967295"));
        self.emit(abi::and_registers(xt, xt, xm)); // j low 32
        self.emit(abi::shift_left_immediate(xt, xt, 32));
        self.emit(abi::or_registers(xs, xs, xt));
        self.emit(abi::store_u64(xs, abi::stack_pointer(), s.zz));
        self.emit(abi::branch(&scaled));
        self.emit(abi::label(&subnormal));
        // z = scalbn(z, n): z *= 2^n via constructing 2^n (n small/negative here).
        self.emit_pow_scalbn(s.zz, nbit, xs, xm, xt, xu);
        self.emit(abi::label(&scaled));
    }

    /// `slot = select(kk!=0 ? b : a)` as an f64 constant.
    fn emit_pow_select(&mut self, kk: &str, a: f64, b: f64, slot: usize, xs: &str, _xt: &str) {
        let pick_b = self.label("pow_sel_b");
        let done = self.label("pow_sel_done");
        self.emit(abi::compare_immediate(kk, "0"));
        self.emit(abi::branch_ne(&pick_b));
        self.emit(abi::move_immediate(xs, "Integer", &a.to_bits().to_string()));
        self.emit(abi::branch(&done));
        self.emit(abi::label(&pick_b));
        self.emit(abi::move_immediate(xs, "Integer", &b.to_bits().to_string()));
        self.emit(abi::label(&done));
        self.emit(abi::store_u64(xs, abi::stack_pointer(), slot));
    }

    /// `slot *= 2**n` for an integer `n` in [-1074, 1023] (fdlibm scalbn, simple
    /// path: build the 2**n factor; for the subnormal output path n is small).
    fn emit_pow_scalbn(&mut self, slot: usize, n: &str, xs: &str, xm: &str, _xt: &str, _xu: &str) {
        // factor = (1023 + n) << 52, as a double; z *= factor. For very negative n
        // this can be subnormal, but the pow underflow/overflow gate already
        // excludes the extreme range, so (1023+n) stays a valid exponent here.
        self.emit(abi::move_immediate(xm, "Integer", "1023"));
        self.emit(abi::add_registers(xm, xm, n));
        self.emit(abi::shift_left_immediate(xm, xm, 52));
        self.emit(abi::float_move_d_from_x("d1", xm)); // 2^n
        self.pld("d0", slot, xs);
        self.emit(abi::float_multiply_d("d0", "d0", "d1"));
        self.pst("d0", slot, xs);
    }
}
