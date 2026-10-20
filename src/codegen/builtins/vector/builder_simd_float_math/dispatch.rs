//! plan-176-T: the kernel dispatchers — unary, scalar and binary entry points, kernel setup/body selection and the
//! error reduction. The consts, the math const pool and the kernel enums live in
//! `builtins/vector/builder_simd_float_math/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// Mint the float kernels' working register file (see [`KernelRegs`]); `trig`
    /// adds the sin/cos/tan reduction registers.
    fn float_kernel_regs(&mut self, trig: bool) -> KernelRegs {
        KernelRegs {
            v16: self.temporary_fp_vreg().into(),
            v17: self.temporary_fp_vreg().into(),
            v18: self.temporary_fp_vreg().into(),
            v19: self.temporary_fp_vreg().into(),
            v20: self.temporary_fp_vreg().into(),
            v21: self.temporary_fp_vreg().into(),
            v22: self.temporary_fp_vreg().into(),
            v23: self.temporary_fp_vreg().into(),
            v24: self.temporary_fp_vreg().into(),
            v25: self.temporary_fp_vreg().into(),
            v26: self.temporary_fp_vreg().into(),
            v27: self.temporary_fp_vreg().into(),
            v28: self.temporary_fp_vreg().into(),
            v29: self.temporary_fp_vreg().into(),
            v30: self.temporary_fp_vreg().into(),
            v31: self.temporary_fp_vreg().into(),
            trig: trig.then(|| TrigRegs {
                limit: self.temporary_fp_vreg().into(),
                cancel: self.temporary_fp_vreg().into(),
                inf: self.temporary_fp_vreg().into(),
                one: self.temporary_fp_vreg().into(),
                route: self.temporary_fp_vreg().into(),
                sin_corr: self.temporary_fp_vreg().into(),
                cos_corr: self.temporary_fp_vreg().into(),
                arg: self.temporary_fp_vreg().into(),
            }),
        }
    }

    /// Lower a unary `math::` Float array overload: build a tight result list,
    /// stream the data region two lanes at a time through the kernel, handle the
    /// odd tail by broadcasting the single element, and reduce the per-lane error
    /// mask (`k.v22`) to one error.
    pub(crate) fn lower_simd_float_unary(
        &mut self,
        kernel: FloatKernel,
        input: ValueResult,
        text: String,
    ) -> Result<ValueResult, String> {
        self.reset_temporary_registers();
        let in_ptr = self.allocate_register();
        self.emit(abi::move_register(&in_ptr, &input.location));
        let count = self.allocate_register();
        self.emit(abi::load_u64(&count, &in_ptr, COLLECTION_OFFSET_COUNT));
        let in_slot = self.allocate_stack_object("simd_fl_in", 8);
        let count_slot = self.allocate_stack_object("simd_fl_count", 8);
        self.emit(abi::store_u64(&in_ptr, abi::stack_pointer(), in_slot));
        self.emit(abi::store_u64(&count, abi::stack_pointer(), count_slot));

        let result_base =
            self.emit_alloc_result_list(&count, &COLLECTION_TYPE_FLOAT.to_string(), "simd_fl")?;

        let in_ptr = self.allocate_register();
        self.emit(abi::load_u64(&in_ptr, abi::stack_pointer(), in_slot));
        let count = self.allocate_register();
        self.emit(abi::load_u64(&count, abi::stack_pointer(), count_slot));
        let in_data = self.allocate_register();
        self.emit_collection_data_pointer_for(&in_data, &in_ptr, &ParameterType::Float);
        let out_data = self.allocate_register();
        self.emit_collection_data_pointer_for(&out_data, &result_base, &ParameterType::Float);
        let pairs = self.allocate_register();
        self.emit(abi::shift_right_immediate(&pairs, &count, 1));

        // v22 = accumulated error mask (valid even when the loop never runs).
        let k = &self.float_kernel_regs(kernel.is_trig());
        self.emit(abi::vector_eor(&k.v22, &k.v22, &k.v22));
        self.emit_float_kernel_setup(kernel, k);

        let loop_label = self.label("simd_fl_loop");
        let loop_done = self.label("simd_fl_done");
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate(&pairs, "0"));
        self.emit(abi::branch_eq(&loop_done));
        self.emit(abi::vector_load(abi::VEC_SCRATCH[0], &in_data, 0));
        self.emit_float_kernel_body(kernel, k);
        self.emit(abi::vector_store(abi::VEC_SCRATCH[0], &out_data, 0));
        self.emit(abi::add_immediate(&in_data, &in_data, 16));
        self.emit(abi::add_immediate(&out_data, &out_data, 16));
        self.emit(abi::subtract_immediate(&pairs, &pairs, 1));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&loop_done));

        // Scalar tail: broadcast the single element, run the kernel, store lane 0.
        let tail_bit = self.temporary_vreg();
        self.emit(abi::move_immediate(&tail_bit, "Integer", "1"));
        self.emit(abi::and_registers(&tail_bit, &count, &tail_bit));
        let tail_done = self.label("simd_fl_tail_done");
        self.emit(abi::compare_immediate(&tail_bit, "0"));
        self.emit(abi::branch_eq(&tail_done));
        let elem = self.temporary_vreg();
        self.emit(abi::load_u64(&elem, &in_data, 0));
        self.emit(abi::vector_dup_from_x(abi::VEC_SCRATCH[0], &elem));
        self.emit_float_kernel_body(kernel, k);
        let res_lane = self.temporary_vreg();
        self.emit(abi::vector_extract_to_x(&res_lane, abi::VEC_SCRATCH[0], 0));
        self.emit(abi::store_u64(&res_lane, &out_data, 0));
        self.emit(abi::label(&tail_done));

        for err in kernel.errors() {
            self.emit_float_error_reduce(*err, k, false)?;
        }

        Ok(ValueResult {
            origin: None,
            type_: ParameterType::ListOf(Box::new(ParameterType::Float)),
            location: Operand::from(result_base),
            text,
        })
    }

    /// Reduce one per-lane error mask to a single GPR and raise the matching
    /// float error if any lane is set (the result list is discarded). Mask
    /// registers: `Domain`/`Nan` → `v22`, `Inf` → `v24`.
    fn emit_float_error_reduce(
        &mut self,
        err: FloatError,
        k: &KernelRegs,
        scalar: bool,
    ) -> Result<(), String> {
        let mask = match err {
            FloatError::Domain | FloatError::Nan => &k.v22,
            FloatError::Inf => &k.v24,
        };
        let lane0 = self.temporary_vreg();
        self.emit(abi::vector_extract_to_x(&lane0, mask, 0));
        // plan-39 B3: a scalar call broadcast the same value into both lanes, so the
        // mask lanes are identical — lane 0 alone is the whole answer. The array
        // path must OR both lanes (they can disagree).
        if !scalar {
            let lane1 = self.temporary_vreg();
            self.emit(abi::vector_extract_to_x(&lane1, mask, 1));
            self.emit(abi::or_registers(&lane0, &lane0, &lane1));
        }
        let no_err = self.label("simd_fl_no_err");
        self.emit(abi::compare_immediate(&lane0, "0"));
        self.emit(abi::branch_eq(&no_err));
        match err {
            FloatError::Domain => self.raise_error_bare("ErrFloatDomain")?,
            FloatError::Nan => self.raise_error_bare("ErrFloatNaN")?,
            FloatError::Inf => self.raise_error_bare("ErrFloatInf")?,
        }
        self.emit(abi::label(&no_err));
        Ok(())
    }

    /// Accumulate a result-is-NaN mask (lane where `v0 != v0`) into `v22`, for the
    /// kernels whose only failure is a NaN result (sin/cos/tan/atan/atan2). Uses
    /// v1/v2 as scratch (free at the end of those bodies).
    pub(super) fn emit_result_nan_into_mask(&mut self, k: &KernelRegs) {
        self.emit(abi::vector_fcmeq(
            abi::VEC_SCRATCH[1],
            abi::VEC_SCRATCH[0],
            abi::VEC_SCRATCH[0],
        )); // non-NaN = all-ones
        self.emit(abi::vector_cmeq(
            abi::VEC_SCRATCH[2],
            abi::VEC_SCRATCH[0],
            abi::VEC_SCRATCH[0],
        )); // all-ones (bitwise self-eq)
        self.emit(abi::vector_eor(
            abi::VEC_SCRATCH[1],
            abi::VEC_SCRATCH[1],
            abi::VEC_SCRATCH[2],
        )); // NaN lanes = all-ones
        self.emit(abi::vector_orr(&k.v22, &k.v22, abi::VEC_SCRATCH[1]));
    }

    /// Lower a *scalar* `Float` transcendental onto the array kernel by
    /// broadcasting the single value into both lanes and extracting lane 0 — so
    /// `math::f(x)` and `math::f([x])[0]` are bit-identical ("one deterministic
    /// surface", plan-01-simd §4.7). The same per-lane error checks run, so the
    /// scalar error codes match the array (and the man pages): `ErrFloatDomain`
    /// for `log`/`log10` non-positive input, `ErrFloatNan`/`ErrFloatInf` for
    /// `exp`/`sin`/`cos`.
    pub(crate) fn lower_simd_float_scalar(
        &mut self,
        kernel: FloatKernel,
        value_loc: impl Into<Reg>,
        text: String,
    ) -> Result<ValueResult, String> {
        let value_loc: Reg = value_loc.into();
        self.emit(abi::vector_dup_from_x(abi::VEC_SCRATCH[0], value_loc));
        let k = &self.float_kernel_regs(kernel.is_trig());
        self.emit(abi::vector_eor(&k.v22, &k.v22, &k.v22));
        self.emit_float_kernel_setup(kernel, k);
        // A single scalar lane can branch on the quadrant and run just one of the
        // sin/cos polynomials (the array body computes both because its lanes can
        // disagree). Bit-identical, ~half the work; other kernels share the body.
        match kernel {
            FloatKernel::Sin => self.emit_sin_cos_body_scalar(false, k),
            FloatKernel::Cos => self.emit_sin_cos_body_scalar(true, k),
            FloatKernel::Tan => self.emit_tan_body_scalar(k),
            // plan-39 B4: single-lane invtrig branches to one atan segment.
            FloatKernel::Atan => {
                self.emit_atan_core_scalar(k);
                self.emit_result_nan_into_mask(k);
            }
            FloatKernel::Asin => self.emit_asin_acos_body(false, true, k),
            FloatKernel::Acos => self.emit_asin_acos_body(true, true, k),
            _ => self.emit_float_kernel_body(kernel, k),
        }
        for err in kernel.errors() {
            self.emit_float_error_reduce(*err, k, true)?;
        }
        let dst = self.allocate_register();
        self.emit(abi::vector_extract_to_x(&dst, abi::VEC_SCRATCH[0], 0));
        Ok(ValueResult {
            origin: None,
            type_: ParameterType::Float,
            location: Operand::from(dst),
            text,
        })
    }

    /// Broadcast the kernel's persistent constants into v16+ (called once before
    /// the loop; v22 is reserved for the error mask).
    fn emit_float_kernel_setup(&mut self, kernel: FloatKernel, k: &KernelRegs) {
        self.emit_load_math_pool_base();
        match kernel {
            FloatKernel::Exp => {
                self.broadcast_f64(&k.v16, INV_LN2);
                self.broadcast_f64(&k.v17, 0.5);
                self.broadcast_f64(&k.v18, LN2_HI);
                self.broadcast_f64(&k.v19, LN2_LO);
                self.broadcast_i64(&k.v20, 1023);
                self.emit(abi::vector_eor(&k.v21, &k.v21, &k.v21));
                self.emit(abi::vector_eor(&k.v24, &k.v24, &k.v24)); // overflow (inf) mask
                                                                    // bug-164 large-argument clamp: the Cody-Waite reduction and the
                                                                    // 2^n biased-exponent scaling both break down once |x| leaves the
                                                                    // representable exp range (the reduced r loses all precision and the
                                                                    // exponent field wraps). v25/v26 hold the exact overflow/underflow
                                                                    // thresholds and v27 holds +Inf so `emit_exp_body` can saturate
                                                                    // out-of-range lanes to +Inf / +0.0 before the finiteness check.
                self.broadcast_f64(&k.v25, EXP_OVERFLOW_THRESHOLD);
                self.broadcast_f64(&k.v26, EXP_UNDERFLOW_THRESHOLD);
                self.broadcast_f64(&k.v27, f64::INFINITY);
            }
            FloatKernel::Log | FloatKernel::Log10 => {
                self.broadcast_f64(&k.v16, SQRT_HALF);
                self.broadcast_f64(&k.v17, 1.0);
                self.broadcast_i64(&k.v18, 2047); // 0x7ff exponent mask
                self.broadcast_i64(&k.v19, 1022); // frexp bias / new exponent
                self.broadcast_i64(&k.v20, 0x800F_FFFF_FFFF_FFFF_u64 as i64); // ~exp field
                self.broadcast_i64(&k.v21, 1022_i64 << 52); // exponent=1022 field
                self.broadcast_i64(&k.v23, 1); // integer one (k adjust)
                self.broadcast_f64(&k.v24, LN2); // ln2 hi
                self.broadcast_f64(&k.v25, LN2_DD_LO); // ln2 lo
                if kernel == FloatKernel::Log10 {
                    self.broadcast_f64(&k.v26, LOG10_E); // 1/ln10 hi
                    self.broadcast_f64(&k.v27, LOG10_E_DD_LO); // 1/ln10 lo
                }
            }
            FloatKernel::Sin | FloatKernel::Cos | FloatKernel::Tan => {
                self.broadcast_f64(&k.v16, INV_PIO2);
                self.broadcast_f64(&k.v17, 0.5);
                self.broadcast_f64(&k.v18, PIO2_1);
                self.broadcast_f64(&k.v19, PIO2_2);
                self.broadcast_f64(&k.v20, PIO2_2T);
                self.broadcast_i64(&k.v21, 3); // quadrant mask
                let t = k.trig();
                self.broadcast_f64(&t.limit, TRIG_MEDIUM_LIMIT);
                self.broadcast_f64(&t.cancel, TRIG_CANCEL_RATIO);
                self.broadcast_f64(&t.inf, f64::INFINITY);
                self.broadcast_f64(&t.one, 1.0);
            }
            FloatKernel::Atan | FloatKernel::Asin | FloatKernel::Acos => {
                self.broadcast_f64(&k.v16, 1.0);
                self.broadcast_f64(&k.v17, std::f64::consts::FRAC_PI_2);
                self.broadcast_i64(&k.v18, i64::MIN); // sign mask 0x8000..
                self.broadcast_i64(&k.v19, i64::MAX); // abs mask 0x7fff..
            }
        }
    }

    /// Emit the per-chunk kernel body: input lanes in `v0`, result in `v0`, error
    /// lanes OR-accumulated into `v22`. Working scratch is v1-v6.
    fn emit_float_kernel_body(&mut self, kernel: FloatKernel, k: &KernelRegs) {
        match kernel {
            FloatKernel::Exp => self.emit_exp_body(k),
            FloatKernel::Log | FloatKernel::Log10 => {
                self.emit_log_body(kernel == FloatKernel::Log10, k)
            }
            FloatKernel::Sin => self.emit_sin_cos_body(false, k),
            FloatKernel::Cos => self.emit_sin_cos_body(true, k),
            FloatKernel::Tan => self.emit_tan_body(k),
            FloatKernel::Atan => self.emit_atan_core(k),
            FloatKernel::Asin => self.emit_asin_acos_body(false, false, k),
            FloatKernel::Acos => self.emit_asin_acos_body(true, false, k),
        }
        // sin/cos/tan/atan fail only with ErrFloatNan (a NaN result, which only
        // happens for NaN/inf input); accumulate the result-NaN mask here so the
        // reduce can raise it. (exp's NaN is detected from its input in-body;
        // asin/acos/log/log10 set their domain mask in-body.)
        if matches!(
            kernel,
            FloatKernel::Sin | FloatKernel::Cos | FloatKernel::Tan | FloatKernel::Atan
        ) {
            self.emit_result_nan_into_mask(k);
        }
    }

    /// Lower a two-array `math::` Float overload. `atan2` is the only kernel that
    /// reaches this driver — array `pow` is diverted to `lower_pow_array`
    /// (`gen_math/`) because it needs the per-element fdlibm kernel — so the
    /// error reduce below only has to cover `atan2`'s single `Nan` failure. Both
    /// lists must have the same length (`ErrInvalidArgument` otherwise).
    /// `left_slot`/`right_slot` already hold the two list pointers (the caller
    /// spilled them).
    pub(crate) fn lower_simd_float_binary(
        &mut self,
        kernel: FloatBinaryKernel,
        left_slot: usize,
        right_slot: usize,
        text: String,
    ) -> Result<ValueResult, String> {
        self.reset_temporary_registers();
        let left_ptr = self.allocate_register();
        self.emit(abi::load_u64(&left_ptr, abi::stack_pointer(), left_slot));
        let right_ptr = self.allocate_register();
        self.emit(abi::load_u64(&right_ptr, abi::stack_pointer(), right_slot));
        let count = self.allocate_register();
        let rcount = self.allocate_register();
        self.emit(abi::load_u64(&count, &left_ptr, COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64(&rcount, &right_ptr, COLLECTION_OFFSET_COUNT));
        let lengths_ok = self.label("simd_flb_len_ok");
        self.emit(abi::compare_registers(&count, &rcount));
        self.emit(abi::branch_eq(&lengths_ok));
        self.raise_error_bare("ErrInvalidArgument")?;
        self.emit(abi::label(&lengths_ok));
        let count_slot = self.allocate_stack_object("simd_flb_count", 8);
        self.emit(abi::store_u64(&count, abi::stack_pointer(), count_slot));

        let result_base =
            self.emit_alloc_result_list(&count, &COLLECTION_TYPE_FLOAT.to_string(), "simd_flb")?;

        let left_ptr = self.allocate_register();
        self.emit(abi::load_u64(&left_ptr, abi::stack_pointer(), left_slot));
        let right_ptr = self.allocate_register();
        self.emit(abi::load_u64(&right_ptr, abi::stack_pointer(), right_slot));
        let count = self.allocate_register();
        self.emit(abi::load_u64(&count, abi::stack_pointer(), count_slot));
        let left_data = self.allocate_register();
        self.emit_collection_data_pointer_for(&left_data, &left_ptr, &ParameterType::Float);
        let right_data = self.allocate_register();
        self.emit_collection_data_pointer_for(&right_data, &right_ptr, &ParameterType::Float);
        let out_data = self.allocate_register();
        self.emit_collection_data_pointer_for(&out_data, &result_base, &ParameterType::Float);
        let pairs = self.allocate_register();
        self.emit(abi::shift_right_immediate(&pairs, &count, 1));
        let k = &self.float_kernel_regs(false);
        self.emit(abi::vector_eor(&k.v22, &k.v22, &k.v22));
        self.emit_float_binary_setup(kernel, k);

        let loop_label = self.label("simd_flb_loop");
        let loop_done = self.label("simd_flb_done");
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate(&pairs, "0"));
        self.emit(abi::branch_eq(&loop_done));
        self.emit(abi::vector_load(abi::VEC_SCRATCH[0], &left_data, 0));
        self.emit(abi::vector_load(abi::VEC_SCRATCH[1], &right_data, 0));
        self.emit_float_binary_body(kernel, k, false);
        self.emit(abi::vector_store(abi::VEC_SCRATCH[0], &out_data, 0));
        self.emit(abi::add_immediate(&left_data, &left_data, 16));
        self.emit(abi::add_immediate(&right_data, &right_data, 16));
        self.emit(abi::add_immediate(&out_data, &out_data, 16));
        self.emit(abi::subtract_immediate(&pairs, &pairs, 1));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&loop_done));

        // Tail: broadcast both single elements, run the kernel, store lane 0.
        let tail_bit = self.temporary_vreg();
        self.emit(abi::move_immediate(&tail_bit, "Integer", "1"));
        self.emit(abi::and_registers(&tail_bit, &count, &tail_bit));
        let tail_done = self.label("simd_flb_tail_done");
        self.emit(abi::compare_immediate(&tail_bit, "0"));
        self.emit(abi::branch_eq(&tail_done));
        let lane = self.temporary_vreg();
        self.emit(abi::load_u64(&lane, &left_data, 0));
        self.emit(abi::vector_dup_from_x(abi::VEC_SCRATCH[0], &lane));
        self.emit(abi::load_u64(&lane, &right_data, 0));
        self.emit(abi::vector_dup_from_x(abi::VEC_SCRATCH[1], &lane));
        self.emit_float_binary_body(kernel, k, false);
        let res_lane = self.temporary_vreg();
        self.emit(abi::vector_extract_to_x(&res_lane, abi::VEC_SCRATCH[0], 0));
        self.emit(abi::store_u64(&res_lane, &out_data, 0));
        self.emit(abi::label(&tail_done));

        // `atan2` is the only kernel routed here (array `pow` is diverted); it
        // fails only with `ErrFloatNan` on a NaN result, matching the scalar man
        // page. Reduce every error the kernel declares (mirroring the scalar
        // sibling `lower_simd_float_binary_scalar`) rather than hardcoding `Nan`,
        // so a future kernel wired here reduces its full error set. Any kernel
        // that raises `Inf` (e.g. `Pow`) also needs its `v24` mask zeroed before
        // the loop and kept out of the per-iteration body — assert atan2-only
        // until that is wired, since the length-mismatch `ErrInvalidArgument` is
        // raised above.
        // Enforce the atan2-only invariant in release too (bug-235): a future
        // Inf-raising binary kernel (e.g. Pow) reuses `v24` as a per-iteration
        // scratch here, so it needs its `v24` Inf mask zeroed before the loop and
        // kept out of the body first. Fail the build loudly rather than silently
        // reducing a stale/never-zeroed mask (spurious or missed ErrFloatInf).
        if !matches!(kernel, FloatBinaryKernel::Atan2) {
            return Err(
                "lower_simd_float_binary is atan2-only; wiring another binary kernel requires \
                 hoisting the v24 Inf-mask zero out of emit_float_binary_body first"
                    .to_string(),
            );
        }
        for err in kernel.errors() {
            self.emit_float_error_reduce(*err, k, false)?;
        }

        Ok(ValueResult {
            origin: None,
            type_: ParameterType::ListOf(Box::new(ParameterType::Float)),
            location: Operand::from(result_base),
            text,
        })
    }

    /// Lower a *scalar* binary `Float` overload (`atan2`/`pow`) onto the array
    /// binary kernel by broadcasting both operands into both `.2d` lanes and
    /// extracting lane 0 — the binary analog of `lower_simd_float_scalar`, so
    /// `math::f(y, x)` and `math::f([y], [x])[0]` are bit-identical. The same
    /// per-lane error checks run, so the scalar error codes match the array.
    pub(crate) fn lower_simd_float_binary_scalar(
        &mut self,
        kernel: FloatBinaryKernel,
        left_loc: impl Into<Reg>,
        right_loc: impl Into<Reg>,
        text: String,
    ) -> Result<ValueResult, String> {
        let (left_loc, right_loc): (Reg, Reg) = (left_loc.into(), right_loc.into());
        self.emit(abi::vector_dup_from_x(abi::VEC_SCRATCH[0], left_loc));
        self.emit(abi::vector_dup_from_x(abi::VEC_SCRATCH[1], right_loc));
        let k = &self.float_kernel_regs(false);
        self.emit(abi::vector_eor(&k.v22, &k.v22, &k.v22));
        self.emit(abi::vector_eor(&k.v24, &k.v24, &k.v24)); // inf/overflow mask
        self.emit_float_binary_setup(kernel, k);
        self.emit_float_binary_body(kernel, k, true);
        for err in kernel.errors() {
            self.emit_float_error_reduce(*err, k, true)?;
        }
        let dst = self.allocate_register();
        self.emit(abi::vector_extract_to_x(&dst, abi::VEC_SCRATCH[0], 0));
        Ok(ValueResult {
            origin: None,
            type_: ParameterType::Float,
            location: Operand::from(dst),
            text,
        })
    }

    fn emit_float_binary_setup(&mut self, kernel: FloatBinaryKernel, k: &KernelRegs) {
        self.emit_load_math_pool_base();
        match kernel {
            FloatBinaryKernel::Atan2 => {
                self.broadcast_f64(&k.v16, 1.0);
                self.broadcast_f64(&k.v17, std::f64::consts::FRAC_PI_2);
                self.broadcast_i64(&k.v18, i64::MIN); // sign mask
                self.broadcast_i64(&k.v19, i64::MAX); // abs mask
                self.broadcast_f64(&k.v23, std::f64::consts::PI);
            }
        }
    }

    fn emit_float_binary_body(&mut self, kernel: FloatBinaryKernel, k: &KernelRegs, scalar: bool) {
        match kernel {
            FloatBinaryKernel::Atan2 => {
                // atan2(y=v0, x=v1) = atan(y/x) + (signbit(x) ? copysign(pi, y) : 0).
                // The mask is bit 63 of x smeared across the lane, not an ordered
                // `x < 0` compare: that is false for -0.0, which dropped the +pi
                // atan2(y, -0.0) = copysign(pi/2, y) needs (bug-932).
                // The origin (0,0) drives q = 0/0 = NaN, which the result-NaN
                // check would raise as ErrFloatNan — but the man page (and the
                // Fixed sibling) define atan2(0,0) = 0. Capture the origin lanes
                // now, while both operands are live, in v24 (untouched by
                // emit_atan_core), and force those result lanes to +0.0 before the
                // NaN check runs (bug-131).
                self.emit(abi::vector_fcmeq_zero(&k.v24, abi::VEC_SCRATCH[0])); // y == 0
                self.emit(abi::vector_fcmeq_zero(
                    abi::VEC_SCRATCH[2],
                    abi::VEC_SCRATCH[1],
                )); // x == 0
                self.emit(abi::vector_and(&k.v24, &k.v24, abi::VEC_SCRATCH[2])); // origin mask
                self.emit(abi::vector_sshr(&k.v20, abi::VEC_SCRATCH[1], 63)); // signbit(x) mask
                self.emit(abi::vector_and(&k.v21, abi::VEC_SCRATCH[0], &k.v18)); // sign(y)
                self.emit(abi::vector_fdiv(
                    abi::VEC_SCRATCH[0],
                    abi::VEC_SCRATCH[0],
                    abi::VEC_SCRATCH[1],
                )); // q = y/x
                    // plan-39 B4: a single-lane scalar call branches to one atan
                    // segment (bit-identical to the branchless core).
                if scalar {
                    self.emit_atan_core_scalar(k); // v0 = atan(q)
                } else {
                    self.emit_atan_core(k); // v0 = atan(q)
                }
                self.emit(abi::vector_orr(abi::VEC_SCRATCH[2], &k.v23, &k.v21)); // copysign(pi, y)
                self.emit(abi::vector_and(
                    abi::VEC_SCRATCH[2],
                    abi::VEC_SCRATCH[2],
                    &k.v20,
                )); // & signbit(x)
                self.emit(abi::vector_fadd(
                    abi::VEC_SCRATCH[0],
                    abi::VEC_SCRATCH[0],
                    abi::VEC_SCRATCH[2],
                ));
                // Origin lanes → +0.0 (select 0 where the origin mask is set).
                self.emit(abi::vector_eor(
                    abi::VEC_SCRATCH[2],
                    abi::VEC_SCRATCH[2],
                    abi::VEC_SCRATCH[2],
                ));
                self.emit_vsel(abi::VEC_SCRATCH[0], &k.v24, abi::VEC_SCRATCH[2]);
                self.emit_result_nan_into_mask(k);
            }
        }
    }
}
