use super::simd_kernel_coeffs::{COS_COEFFS, EXP_COEFFS, LOG_COEFFS, SIN_COEFFS};
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
/// `log10(e) = 1/ln(10)`.
const LOG10_E: f64 = 0.434_294_481_903_251_827_6;
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
}

impl FloatKernel {
    fn error(self) -> Option<FloatError> {
        match self {
            FloatKernel::Exp => Some(FloatError::Overflow),
            FloatKernel::Log | FloatKernel::Log10 => Some(FloatError::InvalidArgument),
            FloatKernel::Sin | FloatKernel::Cos | FloatKernel::Tan => None,
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
                self.broadcast_f64("v16", LN2);
                self.broadcast_f64("v17", SQRT_HALF);
                self.broadcast_f64("v18", 1.0);
                self.broadcast_i64("v19", 2047); // 0x7ff exponent mask
                self.broadcast_i64("v20", 1022); // frexp bias / new exponent
                self.broadcast_i64("v21", 0x800F_FFFF_FFFF_FFFF_u64 as i64); // ~exp field
                self.broadcast_i64("v24", 1022_i64 << 52); // exponent=1022 field
                self.broadcast_i64("v25", 1); // integer one (k adjust)
                if kernel == FloatKernel::Log10 {
                    self.broadcast_f64("v26", LOG10_E);
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
        }
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

    /// `sin`/`cos` kernel. After reduction, evaluate `sin_r = r*P_sin(r^2)` and
    /// `cos_r = P_cos(r^2)`, then apply the quadrant selection/sign.
    fn emit_sin_cos_body(&mut self, want_cos: bool) {
        self.emit_sincos_reduce(); // reduced=v2, quad=v5
        self.emit(abi::vector_fmul("v3", "v2", "v2")); // r2
        self.emit_horner("v6", "v3", &SIN_COEFFS); // P_sin in v6
        self.emit(abi::vector_fmul("v6", "v2", "v6")); // sin_r = r*P_sin
        self.emit_horner("v7", "v3", &COS_COEFFS); // cos_r in v7
        // Quadrant masks: bit0 and bit1 of quad.
        self.emit(abi::vector_shl("v1", "v5", 63));
        self.emit(abi::vector_sshr("v1", "v1", 63)); // mask_b0 (all-ones if bit0)
        self.emit(abi::vector_shl("v0", "v5", 62));
        self.emit(abi::vector_sshr("v0", "v0", 63)); // mask_b1
        if !want_cos {
            // sin: val = bit0 ? cos_r : sin_r; negate if bit1.
            self.emit(abi::vector_bsl("v1", "v7", "v6")); // v1 = val
            self.emit(abi::vector_fneg("v3", "v1"));
            self.emit(abi::vector_bsl("v0", "v3", "v1")); // v0 = bit1 ? -val : val
        } else {
            // cos: val = bit0 ? sin_r : cos_r; negate if bit0 XOR bit1.
            self.emit(abi::vector_eor("v4", "v1", "v0")); // negmask = b0 ^ b1
            self.emit(abi::vector_bsl("v1", "v6", "v7")); // v1 = val
            self.emit(abi::vector_fneg("v3", "v1"));
            self.emit(abi::vector_bsl("v4", "v3", "v1")); // v4 = negmask ? -val : val
            self.emit(abi::vector_orr("v0", "v4", "v4"));
        }
    }

    /// `tan(x) = sin(x) / cos(x)`: one reduction, both quadrant selections, divide.
    fn emit_tan_body(&mut self) {
        self.emit_sincos_reduce(); // reduced=v2, quad=v5
        self.emit(abi::vector_fmul("v3", "v2", "v2")); // r2
        self.emit_horner("v6", "v3", &SIN_COEFFS);
        self.emit(abi::vector_fmul("v6", "v2", "v6")); // sin_r (v6)
        self.emit_horner("v7", "v3", &COS_COEFFS); // cos_r (v7)
        // Quadrant masks b0 (v1), b1 (v2).
        self.emit(abi::vector_shl("v1", "v5", 63));
        self.emit(abi::vector_sshr("v1", "v1", 63));
        self.emit(abi::vector_shl("v2", "v5", 62));
        self.emit(abi::vector_sshr("v2", "v2", 63));
        // sin_full = (b1 ? -1 : 1) * (b0 ? cos_r : sin_r)  → v0.
        self.emit(abi::vector_orr("v3", "v1", "v1"));
        self.emit(abi::vector_bsl("v3", "v7", "v6")); // val_s = b0 ? cos_r : sin_r
        self.emit(abi::vector_fneg("v4", "v3"));
        self.emit(abi::vector_orr("v0", "v2", "v2"));
        self.emit(abi::vector_bsl("v0", "v4", "v3")); // sin_full
        // cos_full = ((b0^b1) ? -1 : 1) * (b0 ? sin_r : cos_r)  → v1.
        self.emit(abi::vector_orr("v3", "v1", "v1"));
        self.emit(abi::vector_bsl("v3", "v6", "v7")); // val_c = b0 ? sin_r : cos_r
        self.emit(abi::vector_fneg("v4", "v3"));
        self.emit(abi::vector_eor("v1", "v1", "v2")); // negmask = b0 ^ b1
        self.emit(abi::vector_bsl("v1", "v4", "v3")); // cos_full
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
    /// s=(m-1)/(m+1), ln(x) = k*ln2 + s*P(s^2).
    fn emit_log_body(&mut self, base10: bool) {
        // Domain: x <= 0 fails.
        self.emit(abi::vector_fcmle_zero("v1", "v0"));
        self.emit(abi::vector_orr("v22", "v22", "v1"));
        // k = ((bits>>52) & 0x7ff) - 1022.
        self.emit(abi::vector_ushr("v1", "v0", 52));
        self.emit(abi::vector_and("v1", "v1", "v19"));
        self.emit(abi::vector_sub("v2", "v1", "v20")); // k (int)
        // m = bits with exponent field replaced by 1022 → m in [0.5, 1).
        self.emit(abi::vector_and("v3", "v0", "v21"));
        self.emit(abi::vector_orr("v3", "v3", "v24")); // m (float)
        // if m < 1/sqrt2 { m *= 2; k -= 1 }.
        self.emit(abi::vector_fcmgt("v4", "v17", "v3")); // mask: sqrt_half > m
        self.emit(abi::vector_and("v5", "v4", "v25")); // mask & 1
        self.emit(abi::vector_sub("v2", "v2", "v5")); // k -= adjust
        self.emit(abi::vector_fadd("v5", "v3", "v3")); // m*2
        self.emit(abi::vector_bsl("v4", "v5", "v3")); // v4 = mask?m2:m
        // s = (m-1)/(m+1); s2 = s*s.
        self.emit(abi::vector_fsub("v5", "v4", "v18")); // m - 1
        self.emit(abi::vector_fadd("v6", "v4", "v18")); // m + 1
        self.emit(abi::vector_fdiv("v1", "v5", "v6")); // s  (v1)
        self.emit(abi::vector_fmul("v5", "v1", "v1")); // s2 (v5)
        // P(s2) via Horner into v3.
        self.emit_horner("v3", "v5", &LOG_COEFFS);
        // poly = s * P(s2); result = poly + k_f*ln2.
        self.emit(abi::vector_fmul("v3", "v1", "v3")); // poly
        self.emit(abi::vector_scvtf("v2", "v2")); // k -> float
        self.emit(abi::vector_fmla("v3", "v2", "v16")); // poly + k_f*ln2
        if base10 {
            self.emit(abi::vector_fmul("v3", "v3", "v26"));
        }
        self.emit(abi::vector_orr("v0", "v3", "v3"));
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
}
