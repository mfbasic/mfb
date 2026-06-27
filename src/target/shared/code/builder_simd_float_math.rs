use super::simd_kernel_coeffs::{ATAN_COEFFS, COS_COEFFS, EXP_COEFFS, LOG_COEFFS, SIN_COEFFS};
use super::*;

// NEON f64 polynomial kernels for the Float transcendentals — plan-01-simd
// Phase 5. Hand-written, identical on every target, no external math library.
// Coefficients come from `simd_kernel_coeffs.rs` (Remez minimax, validated <=1
// ULP against the committed macOS-libm reference vectors in
// `tests/_data/math_kernel_ref/`). The odd tail reuses the vector kernel by
// broadcasting the single element into both lanes (Open Decision #6) — no
// separate scalar path, so the tail lane is bit-identical to a body lane.
//
// Each kernel mirrors the reduction the `gen_coeffs.py verify` harness models,
// so it lands within the validated <=1 ULP envelope.

/// `ln2`, full f64 — exp/log scale constant.
const LN2: f64 = 0.693_147_180_559_945_309_417_232_121_458_18;
/// fdlibm two-part `ln2` so `n*ln2` reconstructs past double precision.
const LN2_HI: f64 = 6.931_471_803_691_238_164_90e-01;
const LN2_LO: f64 = 1.908_214_929_270_587_700_02e-10;
/// `1/sqrt(2)` — the log mantissa fold point.
const SQRT_HALF: f64 = 0.707_106_781_186_547_524_4;
/// `ln2` and `1/ln(10)` as true double-doubles (hi = the nearest double, lo = the
/// tail) so `log`/`log10` recombine the reduction to >double precision and reach
/// strict <=1 ULP. `LN2` above is the hi of `ln2`; `LOG10_E` the hi of `1/ln10`.
const LN2_DD_LO: f64 = 2.319_046_813_846_299_6e-17;
const LOG10_E: f64 = 0.434_294_481_903_251_827_6;
const LOG10_E_DD_LO: f64 = 1.098_319_650_216_765_0e-17;
/// `2/pi` and the fdlibm three-part `pi/2` for the sin/cos Cody-Waite reduction
/// (accurate for `|x| < 2^20 * pi/2`; large arguments would need Payne-Hanek).
const INV_PIO2: f64 = 0.636_619_772_367_581_343_1;
const PIO2_1: f64 = 1.570_796_326_734_125_614_17;
const PIO2_2: f64 = 6.077_100_506_303_965_976_60e-11;
const PIO2_2T: f64 = 2.022_266_248_795_950_631_54e-21;

/// A unary `math::` Float array kernel and the error (if any) it raises.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum FloatKernel {
    /// `e^x`; `ErrOverflow` when the result exceeds the finite double range.
    Exp,
    /// `ln(x)` / `log10(x)`; `ErrInvalidArgument` on a non-positive lane.
    Log,
    Log10,
    /// `sin(x)` / `cos(x)` / `tan(x)`; no error (medium-range reduction).
    Sin,
    Cos,
    Tan,
    /// `atan(x)`; no error.
    Atan,
    /// `asin(x)` / `acos(x)`; `ErrInvalidArgument` for `|x| > 1`.
    Asin,
    Acos,
}

impl FloatKernel {
    fn error(self) -> Option<FloatError> {
        match self {
            FloatKernel::Exp => Some(FloatError::Overflow),
            FloatKernel::Log | FloatKernel::Log10 => Some(FloatError::InvalidArgument),
            FloatKernel::Asin | FloatKernel::Acos => Some(FloatError::InvalidArgument),
            FloatKernel::Sin | FloatKernel::Cos | FloatKernel::Tan | FloatKernel::Atan => None,
        }
    }
}

#[derive(Clone, Copy)]
enum FloatError {
    Overflow,
    InvalidArgument,
}

impl CodeBuilder<'_> {
    /// Lower a unary `math::` Float array overload: build a tight result list,
    /// stream the data region two lanes at a time through the kernel, handle the
    /// odd tail by broadcasting the single element, and reduce the per-lane error
    /// mask (`v22`) to one error.
    pub(super) fn lower_simd_float_unary(
        &mut self,
        kernel: FloatKernel,
        input: ValueResult,
        text: String,
    ) -> Result<ValueResult, String> {
        self.reset_temporary_registers();
        let in_ptr = self.allocate_register()?;
        self.emit(abi::move_register(&in_ptr, &input.location));
        let count = self.allocate_register()?;
        self.emit(abi::load_u64(&count, &in_ptr, COLLECTION_OFFSET_COUNT));
        let in_slot = self.allocate_stack_object("simd_fl_in", 8);
        let count_slot = self.allocate_stack_object("simd_fl_count", 8);
        self.emit(abi::store_u64(&in_ptr, abi::stack_pointer(), in_slot));
        self.emit(abi::store_u64(&count, abi::stack_pointer(), count_slot));

        self.emit(abi::move_register("x0", &count));
        self.emit(abi::move_immediate(
            "x1",
            "Integer",
            &COLLECTION_TYPE_FLOAT.to_string(),
        ));
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
        let alloc_ok = self.label("simd_fl_alloc_ok");
        self.emit(abi::compare_immediate("x1", "0"));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit(abi::move_register("x0", "x1"));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&alloc_ok));

        let in_ptr = self.allocate_register()?;
        self.emit(abi::load_u64(&in_ptr, abi::stack_pointer(), in_slot));
        let count = self.allocate_register()?;
        self.emit(abi::load_u64(&count, abi::stack_pointer(), count_slot));
        let in_data = self.allocate_register()?;
        self.emit_collection_data_pointer(&in_data, &in_ptr);
        let out_data = self.allocate_register()?;
        self.emit_collection_data_pointer(&out_data, &result_base);
        let pairs = self.allocate_register()?;
        self.emit(abi::shift_right_immediate(&pairs, &count, 1));

        // v22 = accumulated error mask (valid even when the loop never runs).
        self.emit(abi::vector_eor("v22", "v22", "v22"));
        self.emit_float_kernel_setup(kernel);

        let loop_label = self.label("simd_fl_loop");
        let loop_done = self.label("simd_fl_done");
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate(&pairs, "0"));
        self.emit(abi::branch_eq(&loop_done));
        self.emit(abi::vector_load("v0", &in_data, 0));
        self.emit_float_kernel_body(kernel);
        self.emit(abi::vector_store("v0", &out_data, 0));
        self.emit(abi::add_immediate(&in_data, &in_data, 16));
        self.emit(abi::add_immediate(&out_data, &out_data, 16));
        self.emit(abi::subtract_immediate(&pairs, &pairs, 1));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&loop_done));

        // Scalar tail: broadcast the single element, run the kernel, store lane 0.
        self.emit(abi::move_immediate("x1", "Integer", "1"));
        self.emit(abi::and_registers("x1", &count, "x1"));
        let tail_done = self.label("simd_fl_tail_done");
        self.emit(abi::compare_immediate("x1", "0"));
        self.emit(abi::branch_eq(&tail_done));
        self.emit(abi::load_u64("x0", &in_data, 0));
        self.emit(abi::vector_dup_from_x("v0", "x0"));
        self.emit_float_kernel_body(kernel);
        self.emit(abi::vector_extract_to_x("x0", "v0", 0));
        self.emit(abi::store_u64("x0", &out_data, 0));
        self.emit(abi::label(&tail_done));

        if let Some(err) = kernel.error() {
            self.emit(abi::vector_extract_to_x("x0", "v22", 0));
            self.emit(abi::vector_extract_to_x("x1", "v22", 1));
            self.emit(abi::or_registers("x0", "x0", "x1"));
            let no_err = self.label("simd_fl_no_err");
            self.emit(abi::compare_immediate("x0", "0"));
            self.emit(abi::branch_eq(&no_err));
            match err {
                FloatError::Overflow => self.emit_overflow_return()?,
                FloatError::InvalidArgument => self.emit_invalid_argument_return()?,
            }
            self.emit(abi::label(&no_err));
        }

        Ok(ValueResult {
            type_: "List OF Float".to_string(),
            location: result_base,
            text,
        })
    }

    /// Lower a *scalar* `Float` transcendental onto the array kernel by
    /// broadcasting the single value into both lanes and extracting lane 0 — so
    /// `math::f(x)` and `math::f([x])[0]` are bit-identical ("one deterministic
    /// surface", plan-01-simd §4.7). No per-lane error reduce: `sin`/`cos` never
    /// error, `exp` overflow yields the kernel's saturated `inf` (matching libm),
    /// and the `log`/`log10` domain is checked by the caller before this runs.
    pub(super) fn lower_simd_float_scalar(
        &mut self,
        kernel: FloatKernel,
        value_loc: &str,
        text: String,
    ) -> Result<ValueResult, String> {
        self.emit(abi::vector_dup_from_x("v0", value_loc));
        self.emit(abi::vector_eor("v22", "v22", "v22"));
        self.emit_float_kernel_setup(kernel);
        self.emit_float_kernel_body(kernel);
        let dst = self.allocate_register()?;
        self.emit(abi::vector_extract_to_x(&dst, "v0", 0));
        Ok(ValueResult {
            type_: "Float".to_string(),
            location: dst,
            text,
        })
    }

    /// Broadcast the kernel's persistent constants into v16+ (called once before
    /// the loop; v22 is reserved for the error mask).
    fn emit_float_kernel_setup(&mut self, kernel: FloatKernel) {
        match kernel {
            FloatKernel::Exp => {
                self.broadcast_f64("v16", LN2);
                self.broadcast_f64("v17", 0.5);
                self.broadcast_f64("v18", LN2_HI);
                self.broadcast_f64("v19", LN2_LO);
                self.broadcast_i64("v20", 1023);
                self.emit(abi::vector_eor("v21", "v21", "v21"));
                self.broadcast_i64("v23", -1022);
            }
            FloatKernel::Log | FloatKernel::Log10 => {
                self.broadcast_f64("v16", SQRT_HALF);
                self.broadcast_f64("v17", 1.0);
                self.broadcast_i64("v18", 2047); // 0x7ff exponent mask
                self.broadcast_i64("v19", 1022); // frexp bias / new exponent
                self.broadcast_i64("v20", 0x800F_FFFF_FFFF_FFFF_u64 as i64); // ~exp field
                self.broadcast_i64("v21", 1022_i64 << 52); // exponent=1022 field
                self.broadcast_i64("v23", 1); // integer one (k adjust)
                self.broadcast_f64("v24", LN2); // ln2 hi
                self.broadcast_f64("v25", LN2_DD_LO); // ln2 lo
                if kernel == FloatKernel::Log10 {
                    self.broadcast_f64("v26", LOG10_E); // 1/ln10 hi
                    self.broadcast_f64("v27", LOG10_E_DD_LO); // 1/ln10 lo
                }
            }
            FloatKernel::Sin | FloatKernel::Cos | FloatKernel::Tan => {
                self.broadcast_f64("v16", INV_PIO2);
                self.broadcast_f64("v17", 0.5);
                self.broadcast_f64("v18", PIO2_1);
                self.broadcast_f64("v19", PIO2_2);
                self.broadcast_f64("v20", PIO2_2T);
                self.broadcast_i64("v21", 3); // quadrant mask
            }
            FloatKernel::Atan | FloatKernel::Asin | FloatKernel::Acos => {
                self.broadcast_f64("v16", 1.0);
                self.broadcast_f64("v17", std::f64::consts::FRAC_PI_2);
                self.broadcast_i64("v18", i64::MIN); // sign mask 0x8000..
                self.broadcast_i64("v19", i64::MAX); // abs mask 0x7fff..
            }
        }
    }

    /// Emit the per-chunk kernel body: input lanes in `v0`, result in `v0`, error
    /// lanes OR-accumulated into `v22`. Working scratch is v1-v6.
    fn emit_float_kernel_body(&mut self, kernel: FloatKernel) {
        match kernel {
            FloatKernel::Exp => self.emit_exp_body(),
            FloatKernel::Log | FloatKernel::Log10 => {
                self.emit_log_body(kernel == FloatKernel::Log10)
            }
            FloatKernel::Sin => self.emit_sin_cos_body(false),
            FloatKernel::Cos => self.emit_sin_cos_body(true),
            FloatKernel::Tan => self.emit_tan_body(),
            FloatKernel::Atan => self.emit_atan_core(),
            FloatKernel::Asin => self.emit_asin_acos_body(false),
            FloatKernel::Acos => self.emit_asin_acos_body(true),
        }
    }

    /// `asin(x)` / `acos(x)` via `asin(x) = atan(x / sqrt(1 - x^2))` (NEON `fdiv`
    /// yields ±inf at x=±1, and `atan(inf) = ±pi/2`); `acos = pi/2 - asin`.
    /// `ErrInvalidArgument` for `|x| > 1`. Faithfully rounded (within a few ULP).
    fn emit_asin_acos_body(&mut self, want_acos: bool) {
        // Domain: |x| > 1 fails.
        self.emit(abi::vector_and("v1", "v0", "v19")); // ax
        self.emit(abi::vector_fcmgt("v6", "v1", "v16")); // ax > 1
        self.emit(abi::vector_orr("v22", "v22", "v6"));
        // arg = x / sqrt(1 - x^2).
        self.emit(abi::vector_orr("v7", "v16", "v16")); // 1.0
        self.emit(abi::vector_fmls("v7", "v0", "v0")); // 1 - x*x
        self.emit(abi::vector_fsqrt("v7", "v7"));
        self.emit(abi::vector_fdiv("v0", "v0", "v7")); // arg → v0
        self.emit_atan_core(); // v0 = atan(arg) = asin(x)
        if want_acos {
            self.emit(abi::vector_fsub("v0", "v17", "v0")); // pi/2 - asin
        }
    }

    /// `atan(x)` core (input in `v0`, result in `v0`): for `|x|<=1` evaluate
    /// `ax*P(ax^2)`; for `|x|>1` use `pi/2 - inv*P(inv^2)` with `inv=1/|x|`;
    /// restore the sign. Constants: v16=1.0, v17=pi/2, v18=sign mask, v19=abs
    /// mask. Reused by asin/acos. (Faithfully rounded; strict <=1 ULP needs a
    /// segmented argument reduction.)
    fn emit_atan_core(&mut self) {
        self.emit(abi::vector_and("v1", "v0", "v19")); // ax = |x|
        self.emit(abi::vector_fcmgt("v2", "v1", "v16")); // mask: ax > 1
        self.emit(abi::vector_fdiv("v3", "v16", "v1")); // inv = 1/ax
        self.emit(abi::vector_orr("v4", "v2", "v2"));
        self.emit(abi::vector_bsl("v4", "v3", "v1")); // u = mask ? inv : ax
        self.emit(abi::vector_fmul("v5", "v4", "v4")); // u2
        // Horner P(u2) → v6, using v3 (inv, now dead) as the coeff scratch so v4=u
        // survives.
        self.broadcast_f64("v6", ATAN_COEFFS[ATAN_COEFFS.len() - 1]);
        for i in (0..ATAN_COEFFS.len() - 1).rev() {
            self.broadcast_f64("v3", ATAN_COEFFS[i]);
            self.emit(abi::vector_fmla("v3", "v6", "v5"));
            self.emit(abi::vector_orr("v6", "v3", "v3"));
        }
        self.emit(abi::vector_fmul("v6", "v4", "v6")); // up = u*P
        self.emit(abi::vector_fsub("v7", "v17", "v6")); // pi/2 - up
        self.emit(abi::vector_bsl("v2", "v7", "v6")); // mask ? (pi/2-up) : up
        self.emit(abi::vector_and("v2", "v2", "v19")); // |result|
        self.emit(abi::vector_and("v0", "v0", "v18")); // sign of x
        self.emit(abi::vector_orr("v0", "v2", "v0")); // restore sign
    }

    /// Cody-Waite reduce `x` to `r in [-pi/4, pi/4]` and quadrant `q & 3`. Leaves
    /// the reduced angle in `v2` and the quadrant (int) in `v5`. Working: v1,v3,
    /// v6,v7. Assumes the persistent trig constants in v16-v21.
    fn emit_sincos_reduce(&mut self) {
        self.emit(abi::vector_fmul("v1", "v0", "v16")); // x*invpio2
        self.emit(abi::vector_fadd("v1", "v1", "v17")); // +0.5
        self.emit(abi::vector_frintm("v1", "v1")); // q = floor(..)
        self.emit(abi::vector_orr("v2", "v0", "v0")); // r = x
        self.emit(abi::vector_fmls("v2", "v1", "v18")); // r -= q*PIO2_1
        self.emit(abi::vector_fmul("v3", "v1", "v19")); // w = q*PIO2_2
        self.emit(abi::vector_fsub("v6", "v2", "v3")); // y0 = r - w
        self.emit(abi::vector_fsub("v7", "v2", "v6")); // r - y0
        self.emit(abi::vector_fsub("v7", "v7", "v3")); // t = (r-y0) - w
        self.emit(abi::vector_fneg("v7", "v7")); // -t
        self.emit(abi::vector_fmla("v7", "v1", "v20")); // -t + q*PIO2_2T
        self.emit(abi::vector_fsub("v2", "v6", "v7")); // reduced = y0 - (..)
        self.emit(abi::vector_fcvtzs("v5", "v1")); // q (int)
        self.emit(abi::vector_and("v5", "v5", "v21")); // quad = q & 3
    }

    /// `sin`/`cos` kernel. After reduction, evaluate the polynomials in
    /// double-double (compensated Horner) for `sin_r = r*P_sin(r^2)` (collapsed
    /// into `v24`) and `cos_r = P_cos(r^2)` (collapsed into `v23`), then apply the
    /// quadrant selection/sign. The compensated polynomials make sin/cos strict
    /// <=1 ULP of macOS libm.
    fn emit_sin_cos_body(&mut self, want_cos: bool) {
        self.emit_sincos_reduce(); // reduced=v2, quad=v5
        self.emit(abi::vector_fmul("v1", "v2", "v2")); // r2 (Horner var)
        // cos_r = collapse(P_cos(r2)) → v23.
        self.emit_compensated_horner("v3", "v4", "v1", &COS_COEFFS);
        self.emit(abi::vector_fadd("v23", "v3", "v4"));
        // sin_r = r * collapse(P_sin(r2)) → v24 (carry the lo through the multiply).
        self.emit_compensated_horner("v3", "v4", "v1", &SIN_COEFFS);
        self.emit_twoprod("v6", "v7", "v2", "v3");
        self.emit(abi::vector_fmla("v7", "v2", "v4")); // pe += r*lo
        self.emit(abi::vector_fadd("v24", "v6", "v7"));
        // Quadrant masks: bit0 (v1) and bit1 (v0) of quad.
        self.emit(abi::vector_shl("v1", "v5", 63));
        self.emit(abi::vector_sshr("v1", "v1", 63));
        self.emit(abi::vector_shl("v0", "v5", 62));
        self.emit(abi::vector_sshr("v0", "v0", 63));
        if !want_cos {
            // sin: val = bit0 ? cos_r : sin_r; negate if bit1.
            self.emit(abi::vector_bsl("v1", "v23", "v24"));
            self.emit(abi::vector_fneg("v3", "v1"));
            self.emit(abi::vector_bsl("v0", "v3", "v1"));
        } else {
            // cos: val = bit0 ? sin_r : cos_r; negate if bit0 XOR bit1.
            self.emit(abi::vector_eor("v4", "v1", "v0"));
            self.emit(abi::vector_bsl("v1", "v24", "v23"));
            self.emit(abi::vector_fneg("v3", "v1"));
            self.emit(abi::vector_bsl("v4", "v3", "v1"));
            self.emit(abi::vector_orr("v0", "v4", "v4"));
        }
    }

    /// `tan(x) = sin(x) / cos(x)`: one reduction, compensated sin_r/cos_r, both
    /// quadrant selections, divide. Strict <=1 ULP except a couple of inputs very
    /// near an asymptote, where a double-double range reduction (future work)
    /// would be needed; ~99.8% of the reference within 1 ULP, max 2.
    fn emit_tan_body(&mut self) {
        self.emit_sincos_reduce(); // reduced=v2, quad=v5
        self.emit(abi::vector_fmul("v1", "v2", "v2")); // r2
        self.emit_compensated_horner("v3", "v4", "v1", &COS_COEFFS);
        self.emit(abi::vector_fadd("v23", "v3", "v4")); // cos_r → v23
        self.emit_compensated_horner("v3", "v4", "v1", &SIN_COEFFS);
        self.emit_twoprod("v6", "v7", "v2", "v3");
        self.emit(abi::vector_fmla("v7", "v2", "v4"));
        self.emit(abi::vector_fadd("v24", "v6", "v7")); // sin_r → v24
        // Quadrant masks b0 (v1), b1 (v2).
        self.emit(abi::vector_shl("v1", "v5", 63));
        self.emit(abi::vector_sshr("v1", "v1", 63));
        self.emit(abi::vector_shl("v2", "v5", 62));
        self.emit(abi::vector_sshr("v2", "v2", 63));
        // sin_full = (b1 ? -1 : 1) * (b0 ? cos_r : sin_r) → v0.
        self.emit(abi::vector_orr("v3", "v1", "v1"));
        self.emit(abi::vector_bsl("v3", "v23", "v24"));
        self.emit(abi::vector_fneg("v4", "v3"));
        self.emit(abi::vector_orr("v0", "v2", "v2"));
        self.emit(abi::vector_bsl("v0", "v4", "v3"));
        // cos_full = ((b0^b1) ? -1 : 1) * (b0 ? sin_r : cos_r) → v1.
        self.emit(abi::vector_orr("v3", "v1", "v1"));
        self.emit(abi::vector_bsl("v3", "v24", "v23"));
        self.emit(abi::vector_fneg("v4", "v3"));
        self.emit(abi::vector_eor("v1", "v1", "v2"));
        self.emit(abi::vector_bsl("v1", "v4", "v3"));
        self.emit(abi::vector_fdiv("v0", "v0", "v1")); // tan = sin/cos
    }

    /// `exp` kernel: n=floor(x/ln2+0.5), Cody-Waite r, Horner P(r), scale 2^n.
    fn emit_exp_body(&mut self) {
        self.emit(abi::vector_fdiv("v1", "v0", "v16"));
        self.emit(abi::vector_fadd("v1", "v1", "v17"));
        self.emit(abi::vector_frintm("v1", "v1"));
        self.emit(abi::vector_orr("v2", "v0", "v0"));
        self.emit(abi::vector_fmls("v2", "v1", "v18"));
        self.emit(abi::vector_fmls("v2", "v1", "v19"));
        self.emit_horner("v3", "v2", &EXP_COEFFS);
        self.emit(abi::vector_fcvtzs("v5", "v1"));
        self.emit(abi::vector_cmgt("v6", "v5", "v20"));
        self.emit(abi::vector_orr("v22", "v22", "v6"));
        self.emit(abi::vector_cmgt("v6", "v23", "v5")); // underflow mask
        self.emit(abi::vector_add("v5", "v5", "v20"));
        self.emit(abi::vector_shl("v5", "v5", 52));
        self.emit(abi::vector_fmul("v0", "v3", "v5"));
        self.emit(abi::vector_bsl("v6", "v21", "v0"));
        self.emit(abi::vector_orr("v0", "v6", "v6"));
    }

    /// `log`/`log10` kernel: x = 2^k*m (frexp + fold to [1/sqrt2, sqrt2)),
    /// s=(m-1)/(m+1), ln(x) = k*ln2 + s*P(s^2), evaluated in double-double (a
    /// compensated Horner plus two-sum/two-product recombination) so the result
    /// is strict <=1 ULP; `log10` then multiplies by `1/ln10` as a double-double.
    /// Constants: v16 sqrt_half, v17 1.0, v18 0x7ff, v19 1022, v20 mantmask,
    /// v21 1022<<52, v23 1, v24/v25 ln2 hi/lo, v26/v27 1/ln10 hi/lo; v22 error.
    fn emit_log_body(&mut self, base10: bool) {
        // Domain: x <= 0 fails.
        self.emit(abi::vector_fcmle_zero("v1", "v0"));
        self.emit(abi::vector_orr("v22", "v22", "v1"));
        // k = ((bits>>52) & 0x7ff) - 1022  (integer, v1).
        self.emit(abi::vector_ushr("v1", "v0", 52));
        self.emit(abi::vector_and("v1", "v1", "v18"));
        self.emit(abi::vector_sub("v1", "v1", "v19"));
        // m = bits with exponent field replaced by 1022 → m in [0.5, 1) (v6).
        self.emit(abi::vector_and("v6", "v0", "v20"));
        self.emit(abi::vector_orr("v6", "v6", "v21"));
        // if m < 1/sqrt2 { m *= 2; k -= 1 }.
        self.emit(abi::vector_fcmgt("v7", "v16", "v6")); // mask: sqrt_half > m
        self.emit(abi::vector_and("v0", "v7", "v23")); // mask & 1
        self.emit(abi::vector_sub("v1", "v1", "v0")); // k -= adjust
        self.emit(abi::vector_fadd("v0", "v6", "v6")); // m*2
        self.emit(abi::vector_bsl("v7", "v0", "v6")); // v7 = mask?m2:m  (= m)
        self.emit(abi::vector_scvtf("v3", "v1")); // k -> float (v3)
        // s = (m-1)/(m+1) (v2); s2 = s*s (v1, the Horner variable).
        self.emit(abi::vector_fsub("v0", "v7", "v17")); // m - 1
        self.emit(abi::vector_fadd("v6", "v7", "v17")); // m + 1
        self.emit(abi::vector_fdiv("v2", "v0", "v6")); // s
        self.emit(abi::vector_fmul("v1", "v2", "v2")); // s2
        // P(s2) as a double-double (hi=v4, lo=v5) via compensated Horner.
        self.emit_compensated_horner("v4", "v5", "v1", &LOG_COEFFS);
        // ln(m) = s * (hi+lo): two-product then fma the lo terms → (v7=lh, v28=le).
        self.emit_twoprod("v7", "v28", "v2", "v4");
        self.emit(abi::vector_fmla("v28", "v2", "v5")); // le += s*lo
        // k*ln2 as a double-double → (v29=kh, v30=ke).
        self.emit_twoprod("v29", "v30", "v3", "v24");
        self.emit(abi::vector_fmla("v30", "v3", "v25")); // ke += k*ln2lo
        // (kh,ke) + (lh,le): two-sum hi, accumulate the lows → hi=v0, lo=v31.
        // Scratch v4/v5 are dead (Horner outputs consumed).
        self.emit_twosum("v0", "v31", "v29", "v7", "v4", "v5");
        self.emit(abi::vector_fadd("v31", "v31", "v30")); // + ke
        self.emit(abi::vector_fadd("v31", "v31", "v28")); // + le
        if !base10 {
            self.emit(abi::vector_fadd("v0", "v0", "v31")); // ln(x) = hi + lo
        } else {
            // log10(x) = (hi+lo) * (1/ln10 as hi+lo), compensated.
            self.emit_twoprod("v6", "v7", "v0", "v26"); // ph = hi*L10HI
            self.emit(abi::vector_fmla("v7", "v0", "v27")); // pe += hi*L10LO
            self.emit(abi::vector_fmla("v7", "v31", "v26")); // pe += lo*L10HI
            self.emit(abi::vector_fadd("v0", "v6", "v7"));
        }
    }

    /// Double-double product `a*b → (p, e)` with `p+e == a*b` to ~2x precision:
    /// `p = a*b`, `e = fma(a, b, -p)`. `p`/`e` must be distinct from `a`/`b`.
    fn emit_twoprod(&mut self, p: &str, e: &str, a: &str, b: &str) {
        self.emit(abi::vector_fmul(p, a, b));
        self.emit(abi::vector_fneg(e, p)); // e = -p
        self.emit(abi::vector_fmla(e, a, b)); // e = -p + a*b = fma(a,b,-p)
    }

    /// Knuth two-sum `a+b → (s, e)` with `s+e == a+b` exactly. `t1`/`t2` are
    /// caller-supplied scratch (must be caller-saved vector regs distinct from the
    /// operands/results — `v8`-`v15` are callee-saved and off-limits here).
    fn emit_twosum(&mut self, s: &str, e: &str, a: &str, b: &str, t1: &str, t2: &str) {
        self.emit(abi::vector_fadd(s, a, b));
        self.emit(abi::vector_fsub(t1, s, a)); // t
        self.emit(abi::vector_fsub(t2, s, t1)); // s - t  (~a)
        self.emit(abi::vector_fsub(t2, a, t2)); // a - (s-t)
        self.emit(abi::vector_fsub(t1, b, t1)); // b - t
        self.emit(abi::vector_fadd(e, t2, t1));
    }

    /// Compensated (double-double) Horner of `coeffs` in `var`, leaving the result
    /// as `(hi, lo)`. Each step keeps the running accumulator to ~2x precision.
    /// Uses v6 (coeff broadcast), v7/v28/v29/v30 and v8/v9 (via two-sum) as
    /// scratch — distinct from `hi`/`lo`/`var`.
    fn emit_compensated_horner(&mut self, hi: &str, lo: &str, var: &str, coeffs: &[f64]) {
        self.broadcast_f64(hi, coeffs[coeffs.len() - 1]);
        self.emit(abi::vector_eor(lo, lo, lo));
        for i in (0..coeffs.len() - 1).rev() {
            // (ph, pe) = twoprod(hi, var); pe += lo*var.
            self.emit_twoprod("v7", "v28", hi, var);
            self.emit(abi::vector_fmla("v28", lo, var));
            // (sh, se) = twosum(c, ph). Scratch v0/v31 are free during the Horner.
            self.broadcast_f64("v6", coeffs[i]);
            self.emit_twosum("v29", "v30", "v6", "v7", "v0", "v31");
            // hi = sh; lo = se + pe.
            self.emit(abi::vector_orr(hi, "v29", "v29"));
            self.emit(abi::vector_fadd(lo, "v30", "v28"));
        }
    }

    /// Horner evaluation of `coeffs` in the variable held by `var`, leaving the
    /// result in `acc`. `acc = c[n-1]; acc = c[i] + acc*var` for `i = n-2..=0`.
    /// Uses `v4` as the per-step coefficient broadcast (callers must not hold a
    /// live value there).
    fn emit_horner(&mut self, acc: &str, var: &str, coeffs: &[f64]) {
        self.broadcast_f64(acc, coeffs[coeffs.len() - 1]);
        for i in (0..coeffs.len() - 1).rev() {
            self.broadcast_f64("v4", coeffs[i]);
            self.emit(abi::vector_fmla("v4", acc, var));
            self.emit(abi::vector_orr(acc, "v4", "v4"));
        }
    }

    /// Broadcast an `f64` constant's bit pattern into both `.2d` lanes of `vreg`.
    fn broadcast_f64(&mut self, vreg: &str, value: f64) {
        self.emit(abi::move_immediate("x0", "Integer", &value.to_bits().to_string()));
        self.emit(abi::vector_dup_from_x(vreg, "x0"));
    }

    /// Broadcast a signed `i64` constant into both `.2d` lanes of `vreg`.
    fn broadcast_i64(&mut self, vreg: &str, value: i64) {
        self.emit(abi::move_immediate(
            "x0",
            "Integer",
            &(value as u64).to_string(),
        ));
        self.emit(abi::vector_dup_from_x(vreg, "x0"));
    }

    /// Lower a two-array `math::` Float overload (`atan2`/`pow`). Both lists must
    /// have the same length (`ErrInvalidArgument` otherwise). `left_slot`/
    /// `right_slot` already hold the two list pointers (the caller spilled them).
    pub(super) fn lower_simd_float_binary(
        &mut self,
        kernel: FloatBinaryKernel,
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
        let lengths_ok = self.label("simd_flb_len_ok");
        self.emit(abi::compare_registers(&count, &rcount));
        self.emit(abi::branch_eq(&lengths_ok));
        self.emit_invalid_argument_return()?;
        self.emit(abi::label(&lengths_ok));
        let count_slot = self.allocate_stack_object("simd_flb_count", 8);
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
        let alloc_ok = self.label("simd_flb_alloc_ok");
        self.emit(abi::compare_immediate("x1", "0"));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit(abi::move_register("x0", "x1"));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&alloc_ok));

        let left_ptr = self.allocate_register()?;
        self.emit(abi::load_u64(&left_ptr, abi::stack_pointer(), left_slot));
        let right_ptr = self.allocate_register()?;
        self.emit(abi::load_u64(&right_ptr, abi::stack_pointer(), right_slot));
        let count = self.allocate_register()?;
        self.emit(abi::load_u64(&count, abi::stack_pointer(), count_slot));
        let left_data = self.allocate_register()?;
        self.emit_collection_data_pointer(&left_data, &left_ptr);
        let right_data = self.allocate_register()?;
        self.emit_collection_data_pointer(&right_data, &right_ptr);
        let out_data = self.allocate_register()?;
        self.emit_collection_data_pointer(&out_data, &result_base);
        let pairs = self.allocate_register()?;
        self.emit(abi::shift_right_immediate(&pairs, &count, 1));
        self.emit(abi::vector_eor("v22", "v22", "v22"));
        self.emit_float_binary_setup(kernel);

        let loop_label = self.label("simd_flb_loop");
        let loop_done = self.label("simd_flb_done");
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate(&pairs, "0"));
        self.emit(abi::branch_eq(&loop_done));
        self.emit(abi::vector_load("v0", &left_data, 0));
        self.emit(abi::vector_load("v1", &right_data, 0));
        self.emit_float_binary_body(kernel);
        self.emit(abi::vector_store("v0", &out_data, 0));
        self.emit(abi::add_immediate(&left_data, &left_data, 16));
        self.emit(abi::add_immediate(&right_data, &right_data, 16));
        self.emit(abi::add_immediate(&out_data, &out_data, 16));
        self.emit(abi::subtract_immediate(&pairs, &pairs, 1));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&loop_done));

        // Tail: broadcast both single elements, run the kernel, store lane 0.
        self.emit(abi::move_immediate("x1", "Integer", "1"));
        self.emit(abi::and_registers("x1", &count, "x1"));
        let tail_done = self.label("simd_flb_tail_done");
        self.emit(abi::compare_immediate("x1", "0"));
        self.emit(abi::branch_eq(&tail_done));
        self.emit(abi::load_u64("x0", &left_data, 0));
        self.emit(abi::vector_dup_from_x("v0", "x0"));
        self.emit(abi::load_u64("x0", &right_data, 0));
        self.emit(abi::vector_dup_from_x("v1", "x0"));
        self.emit_float_binary_body(kernel);
        self.emit(abi::vector_extract_to_x("x0", "v0", 0));
        self.emit(abi::store_u64("x0", &out_data, 0));
        self.emit(abi::label(&tail_done));

        Ok(ValueResult {
            type_: "List OF Float".to_string(),
            location: result_base,
            text,
        })
    }

    fn emit_float_binary_setup(&mut self, kernel: FloatBinaryKernel) {
        match kernel {
            FloatBinaryKernel::Atan2 => {
                self.broadcast_f64("v16", 1.0);
                self.broadcast_f64("v17", std::f64::consts::FRAC_PI_2);
                self.broadcast_i64("v18", i64::MIN); // sign mask
                self.broadcast_i64("v19", i64::MAX); // abs mask
                self.broadcast_f64("v23", std::f64::consts::PI);
            }
            // pow re-broadcasts the log then exp constants inside the body.
            FloatBinaryKernel::Pow => {}
        }
    }

    fn emit_float_binary_body(&mut self, kernel: FloatBinaryKernel) {
        match kernel {
            FloatBinaryKernel::Atan2 => {
                // atan2(y=v0, x=v1) = atan(y/x) + (x<0 ? copysign(pi, y) : 0).
                self.emit(abi::vector_fcmlt_zero("v20", "v1")); // x < 0 mask
                self.emit(abi::vector_and("v21", "v0", "v18")); // sign(y)
                self.emit(abi::vector_fdiv("v0", "v0", "v1")); // q = y/x
                self.emit_atan_core(); // v0 = atan(q)
                self.emit(abi::vector_orr("v2", "v23", "v21")); // copysign(pi, y)
                self.emit(abi::vector_and("v2", "v2", "v20")); // & (x<0)
                self.emit(abi::vector_fadd("v0", "v0", "v2"));
            }
            FloatBinaryKernel::Pow => {
                // pow(x=v0, y=v1) = exp(y * log(x)). Re-broadcast each kernel's
                // constants in turn; y is parked in v26 (untouched by log/exp).
                self.emit(abi::vector_orr("v26", "v1", "v1")); // save y
                self.emit_float_kernel_setup(FloatKernel::Log);
                self.emit_log_body(false); // v0 = log(x)
                self.emit(abi::vector_fmul("v0", "v0", "v26")); // y*log(x)
                self.emit_float_kernel_setup(FloatKernel::Exp);
                self.emit_exp_body(); // v0 = exp(y*log(x))
            }
        }
    }
}

/// A two-array `math::` Float kernel.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum FloatBinaryKernel {
    /// `atan2(y, x)`.
    Atan2,
    /// `pow(base, exponent) = exp(exponent * log(base))`.
    Pow,
}
