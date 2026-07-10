// These kernels deliberately spell out full-precision fdlibm/Remez constants
// (the `hi` halves of double-double reductions such as `LN2`, `LOG10_E`,
// `INV_PIO2`). They sit near `std::f64::consts::*` but must NOT be swapped for
// them — the std const is a single rounded double, whereas these are paired with
// their `lo` tails to recombine past double precision. Allow the deny-by-default
// correctness lint for the whole module.
#![allow(clippy::approx_constant)]

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
/// `1/ln2`, full f64 — exp reduces `n = floor(x/ln2 + 0.5)` with a reciprocal
/// multiply instead of a divide (the Cody-Waite `n*ln2_hi/lo` subtraction keeps
/// the reduction exact, so this only moves the rounding of `x/ln2` by ≤1 ULP —
/// validated ≤1 ULP by `runtime_ulp.py exp`).
const INV_LN2: f64 = 1.442_695_040_888_963_407_359_924_681_001_89;
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

/// fdlibm `atan` 4-segment reduction: breakpoint `atan(c)` values as hi/lo
/// double-doubles, and the minimax polynomial `aT` for the reduced argument.
/// These reach strict <=1 ULP of macOS libm (public-domain Sun fdlibm constants).
const ATAN_SEG_THRESH: [f64; 4] = [0.4375, 0.6875, 1.1875, 2.4375];
const ATAN_HI: [f64; 4] = [
    4.636_476_090_008_060_935_15e-01,
    7.853_981_633_974_482_789_99e-01,
    9.827_937_232_473_290_540_82e-01,
    1.570_796_326_794_896_558_00e+00,
];
const ATAN_LO: [f64; 4] = [
    2.269_877_745_296_168_709_24e-17,
    3.061_616_997_868_383_017_93e-17,
    1.390_331_103_123_099_845_16e-17,
    6.123_233_995_736_766_035_87e-17,
];
const ATAN_AT: [f64; 11] = [
    3.333_333_333_333_293_180_27e-01,
    -1.999_999_999_987_648_324_76e-01,
    1.428_571_427_250_346_637_11e-01,
    -1.111_111_040_546_235_578_80e-01,
    9.090_887_133_436_506_561_96e-02,
    -7.691_876_205_044_829_994_95e-02,
    6.661_073_137_387_531_206_69e-02,
    -5.833_570_133_790_573_486_45e-02,
    4.976_877_994_615_932_360_17e-02,
    -3.653_157_274_421_691_552_70e-02,
    1.628_582_011_536_578_236_23e-02,
];

// --- Math-kernel constant pool (plan-03 Phase 2 shared infrastructure) ---
//
// The scalar `math::` path runs each SIMD kernel body once per call, so every
// f64/i64 constant it broadcasts was rebuilt from immediates every call: a
// `movz`+`movk`x3+`dup` chain (5 instructions) per constant, dozens per trig
// call. Instead, gather every kernel constant into one read-only data object —
// each value stored duplicated across both `.2d` lanes of a 16-byte slot — load
// the pool's base once per kernel into a pinned scratch GPR, and replace each
// broadcast with a single `ldr q, [base, #off]`. Same bit patterns, so the
// result is bit-identical (the ULP gate and the `.run` goldens are unchanged);
// `broadcast_f64`/`broadcast_i64` fall back to the old inline build for any value
// not in the pool, so accuracy can never depend on the pool's coverage.

/// Internal read-only data symbol holding the kernel constant pool. Emitted by
/// the module assembler iff some function references it (mod.rs).
pub(super) const MATH_CONST_POOL_SYMBOL: &str = "_mfb_math_const_pool";

/// GPR pinned to the pool base for a kernel's lifetime. The register is
/// per-backend (`mir::Backend::math_pool_base`): AArch64 uses `x2` (caller-saved
/// scratch the allocator never assigns, `INT_ALLOCATABLE` is `x8`+, so it holds
/// the base across the whole body; the array paths load it *after* their one
/// `bl`, and the bodies make no further call). x86 uses `r11` instead, because
/// `x2` is a SysV ABI-role register there and would be remapped inconsistently
/// across the quadrant branch. See `math_pool_base` for the full rationale.

/// The deduplicated constant words (f64/i64 bit patterns) the SIMD kernels
/// broadcast, in a fixed order. Each occupies a 16-byte slot (the value in both
/// `.2d` lanes), so its byte offset is `index * 16`.
pub(super) fn math_const_pool_words() -> Vec<u64> {
    fn add(words: &mut Vec<u64>, bits: u64) {
        if !words.contains(&bits) {
            words.push(bits);
        }
    }
    let mut w: Vec<u64> = Vec::new();
    for v in [
        LN2,
        INV_LN2,
        LN2_HI,
        LN2_LO,
        SQRT_HALF,
        LN2_DD_LO,
        LOG10_E,
        LOG10_E_DD_LO,
        INV_PIO2,
        PIO2_1,
        PIO2_2,
        PIO2_2T,
        0.5,
        1.0,
        1.5,
        2.0,
        3.0,
        std::f64::consts::FRAC_PI_2,
        std::f64::consts::PI,
    ] {
        add(&mut w, v.to_bits());
    }
    for v in ATAN_SEG_THRESH {
        add(&mut w, v.to_bits());
    }
    for v in ATAN_HI {
        add(&mut w, v.to_bits());
    }
    for v in ATAN_LO {
        add(&mut w, v.to_bits());
    }
    for v in ATAN_AT {
        add(&mut w, v.to_bits());
    }
    for v in EXP_COEFFS {
        add(&mut w, v.to_bits());
    }
    for v in LOG_COEFFS {
        add(&mut w, v.to_bits());
    }
    for v in SIN_COEFFS {
        add(&mut w, v.to_bits());
    }
    for v in COS_COEFFS {
        add(&mut w, v.to_bits());
    }
    for v in [
        1023_i64,
        -1022,
        2047,
        1022,
        1,
        3,
        i64::MIN,
        i64::MAX,
        1022_i64 << 52,
        0x800F_FFFF_FFFF_FFFF_u64 as i64,
    ] {
        add(&mut w, v as u64);
    }
    w
}

/// The 16-byte-slot byte offset of `bits` in the pool, or `None` if not pooled.
fn math_const_pool_offset(bits: u64) -> Option<usize> {
    math_const_pool_words()
        .iter()
        .position(|word| *word == bits)
        .map(|index| index * 16)
}

/// Hex (the `raw` data-object `value`) for the pool: each word little-endian,
/// written twice to fill both `.2d` lanes of its 16-byte slot.
pub(super) fn math_const_pool_data_value() -> String {
    use std::fmt::Write;
    let mut hex = String::new();
    for word in math_const_pool_words() {
        let le = word.to_le_bytes();
        for _lane in 0..2 {
            for byte in le {
                let _ = write!(hex, "{byte:02x}");
            }
        }
    }
    hex
}

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

/// A float error condition + the per-lane mask register the kernel accumulates
/// it into. Error codes match the scalar `math::` man pages: domain failures →
/// `ErrFloatDomain`, NaN results → `ErrFloatNan`, infinite results → `ErrFloatInf`.
#[derive(Clone, Copy)]
enum FloatError {
    /// Input outside the domain (sqrt<0, log≤0, asin/acos |x|>1). Mask in `v22`.
    Domain,
    /// Result is NaN (trig/exp/pow of NaN/inf input). Mask in `v22`.
    Nan,
    /// Result overflows to infinity (exp/pow). Mask in `v24`.
    Inf,
}

impl FloatKernel {
    /// The error checks this kernel performs, matching the scalar man pages.
    fn errors(self) -> &'static [FloatError] {
        match self {
            FloatKernel::Exp => &[FloatError::Nan, FloatError::Inf],
            FloatKernel::Log | FloatKernel::Log10 => &[FloatError::Domain],
            FloatKernel::Asin | FloatKernel::Acos => &[FloatError::Domain],
            FloatKernel::Sin | FloatKernel::Cos | FloatKernel::Tan | FloatKernel::Atan => {
                &[FloatError::Nan]
            }
        }
    }
}

/// The float kernels' shared working register file — one FP virtual register
/// per historical `v16`–`v31` home. The kernels use this bank as a
/// *function-global* file: `emit_float_kernel_setup` broadcasts each kernel's
/// persistent constants (and zeroes its error masks) into it once, and the body
/// emitters — possibly emitted several times (chunk loop + odd tail) — read
/// them, so one `KernelRegs` is minted per kernel invocation
/// (`lower_simd_float_*`) and threaded explicitly through the whole emitter
/// call graph. Field names keep the historical register spelling so each
/// kernel's register comments still line up. The physical `v0`–`v7` scratch the
/// bodies also use stays physical (caller-saved working bank, below the
/// allocator's interference radar only in the sense that it is per-instruction
/// tracked physical liveness).
///
/// Register-width contract (same as the historical physical bank): these carry
/// 128-bit vectors, and no value in this file is ever live across a *returning*
/// call — the only `bl`s inside a kernel invocation are dead-end error returns.
/// The historical caller-saved `v16`–`v31` bank imposed the identical
/// constraint, so the conversion adds no new obligation; it removes the fixed
/// physical claim.
struct KernelRegs {
    v16: String,
    v17: String,
    v18: String,
    v19: String,
    v20: String,
    v21: String,
    v22: String,
    v23: String,
    v24: String,
    v25: String,
    v26: String,
    v27: String,
    v28: String,
    v29: String,
    v30: String,
    v31: String,
}

impl CodeBuilder<'_> {
    /// Mint the float kernels' working register file (see [`KernelRegs`]).
    fn float_kernel_regs(&mut self) -> KernelRegs {
        KernelRegs {
            v16: self.temporary_fp_vreg(),
            v17: self.temporary_fp_vreg(),
            v18: self.temporary_fp_vreg(),
            v19: self.temporary_fp_vreg(),
            v20: self.temporary_fp_vreg(),
            v21: self.temporary_fp_vreg(),
            v22: self.temporary_fp_vreg(),
            v23: self.temporary_fp_vreg(),
            v24: self.temporary_fp_vreg(),
            v25: self.temporary_fp_vreg(),
            v26: self.temporary_fp_vreg(),
            v27: self.temporary_fp_vreg(),
            v28: self.temporary_fp_vreg(),
            v29: self.temporary_fp_vreg(),
            v30: self.temporary_fp_vreg(),
            v31: self.temporary_fp_vreg(),
        }
    }

    /// Lower a unary `math::` Float array overload: build a tight result list,
    /// stream the data region two lanes at a time through the kernel, handle the
    /// odd tail by broadcasting the single element, and reduce the per-lane error
    /// mask (`k.v22`) to one error.
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
            kind: RelocIntent::Call,
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
        let k = &self.float_kernel_regs();
        self.emit(abi::vector_eor(&k.v22, &k.v22, &k.v22));
        self.emit_float_kernel_setup(kernel, k);

        let loop_label = self.label("simd_fl_loop");
        let loop_done = self.label("simd_fl_done");
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate(&pairs, "0"));
        self.emit(abi::branch_eq(&loop_done));
        self.emit(abi::vector_load("v0", &in_data, 0));
        self.emit_float_kernel_body(kernel, k);
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
        self.emit_float_kernel_body(kernel, k);
        self.emit(abi::vector_extract_to_x("x0", "v0", 0));
        self.emit(abi::store_u64("x0", &out_data, 0));
        self.emit(abi::label(&tail_done));

        for err in kernel.errors() {
            self.emit_float_error_reduce(*err, k)?;
        }

        Ok(ValueResult {
            type_: "List OF Float".to_string(),
            location: result_base,
            text,
        })
    }

    /// Reduce one per-lane error mask to a single GPR and raise the matching
    /// float error if any lane is set (the result list is discarded). Mask
    /// registers: `Domain`/`Nan` → `v22`, `Inf` → `v24`.
    fn emit_float_error_reduce(&mut self, err: FloatError, k: &KernelRegs) -> Result<(), String> {
        let mask = match err {
            FloatError::Domain | FloatError::Nan => &k.v22,
            FloatError::Inf => &k.v24,
        };
        self.emit(abi::vector_extract_to_x("x0", mask, 0));
        self.emit(abi::vector_extract_to_x("x1", mask, 1));
        self.emit(abi::or_registers("x0", "x0", "x1"));
        let no_err = self.label("simd_fl_no_err");
        self.emit(abi::compare_immediate("x0", "0"));
        self.emit(abi::branch_eq(&no_err));
        match err {
            FloatError::Domain => self.emit_float_domain_return()?,
            FloatError::Nan => self.emit_float_nan_return()?,
            FloatError::Inf => self.emit_float_inf_return()?,
        }
        self.emit(abi::label(&no_err));
        Ok(())
    }

    /// Accumulate a result-is-NaN mask (lane where `v0 != v0`) into `v22`, for the
    /// kernels whose only failure is a NaN result (sin/cos/tan/atan/atan2). Uses
    /// v1/v2 as scratch (free at the end of those bodies).
    fn emit_result_nan_into_mask(&mut self, k: &KernelRegs) {
        self.emit(abi::vector_fcmeq("v1", "v0", "v0")); // non-NaN = all-ones
        self.emit(abi::vector_cmeq("v2", "v0", "v0")); // all-ones (bitwise self-eq)
        self.emit(abi::vector_eor("v1", "v1", "v2")); // NaN lanes = all-ones
        self.emit(abi::vector_orr(&k.v22, &k.v22, "v1"));
    }

    /// Lower a *scalar* `Float` transcendental onto the array kernel by
    /// broadcasting the single value into both lanes and extracting lane 0 — so
    /// `math::f(x)` and `math::f([x])[0]` are bit-identical ("one deterministic
    /// surface", plan-01-simd §4.7). The same per-lane error checks run, so the
    /// scalar error codes match the array (and the man pages): `ErrFloatDomain`
    /// for `log`/`log10` non-positive input, `ErrFloatNan`/`ErrFloatInf` for
    /// `exp`/`sin`/`cos`.
    pub(super) fn lower_simd_float_scalar(
        &mut self,
        kernel: FloatKernel,
        value_loc: &str,
        text: String,
    ) -> Result<ValueResult, String> {
        self.emit(abi::vector_dup_from_x("v0", value_loc));
        let k = &self.float_kernel_regs();
        self.emit(abi::vector_eor(&k.v22, &k.v22, &k.v22));
        self.emit_float_kernel_setup(kernel, k);
        // A single scalar lane can branch on the quadrant and run just one of the
        // sin/cos polynomials (the array body computes both because its lanes can
        // disagree). Bit-identical, ~half the work; other kernels share the body.
        match kernel {
            FloatKernel::Sin => self.emit_sin_cos_body_scalar(false, k),
            FloatKernel::Cos => self.emit_sin_cos_body_scalar(true, k),
            FloatKernel::Tan => self.emit_tan_body_scalar(k),
            _ => self.emit_float_kernel_body(kernel, k),
        }
        for err in kernel.errors() {
            self.emit_float_error_reduce(*err, k)?;
        }
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
                self.broadcast_i64(&k.v23, -1022);
                self.emit(abi::vector_eor(&k.v24, &k.v24, &k.v24)); // overflow (inf) mask
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
                self.emit_log_body(kernel == FloatKernel::Log10, false, k)
            }
            FloatKernel::Sin => self.emit_sin_cos_body(false, k),
            FloatKernel::Cos => self.emit_sin_cos_body(true, k),
            FloatKernel::Tan => self.emit_tan_body(k),
            FloatKernel::Atan => self.emit_atan_core(k),
            FloatKernel::Asin => self.emit_asin_acos_body(false, k),
            FloatKernel::Acos => self.emit_asin_acos_body(true, k),
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

    /// `asin(x)` via `asin(x) = atan(x / sqrt(1 - x^2))` and `acos(x)` via the
    /// half-angle identity `acos(x) = 2*atan( sqrt( (1-x)/(1+x) ) )`. NEON `fdiv`
    /// yields ±inf at the domain edge (`x=±1` for asin, `x=-1` for acos), which
    /// `atan` maps to the correct ±pi/2, so the endpoints come out exact. `acos`
    /// uses the half-angle form rather than `pi/2 - asin` to avoid catastrophic
    /// cancellation as `x → +1` (where `acos → 0`); `1±x` is exact for `|x| <= 1`
    /// (Sterbenz), so both stay ≤1 ULP. `ErrInvalidArgument` for `|x| > 1`.
    fn emit_asin_acos_body(&mut self, want_acos: bool, k: &KernelRegs) {
        // Domain: |x| > 1 fails.
        self.emit(abi::vector_and("v1", "v0", &k.v19)); // ax
        self.emit(abi::vector_fcmgt("v6", "v1", &k.v16)); // ax > 1
        self.emit(abi::vector_orr(&k.v22, &k.v22, "v6"));
        if !want_acos {
            // asin(x) = atan(x / sqrt(1 - x^2)).
            self.emit(abi::vector_orr("v7", &k.v16, &k.v16)); // 1.0
            self.emit(abi::vector_fmls("v7", "v0", "v0")); // 1 - x*x
            self.emit(abi::vector_fsqrt("v7", "v7"));
            self.emit(abi::vector_fdiv("v0", "v0", "v7")); // arg → v0
            self.emit_atan_core(k); // v0 = atan(arg) = asin(x)
        } else {
            // acos(x) = 2*atan( sqrt( (1-x)/(1+x) ) ).
            self.emit(abi::vector_fsub("v6", &k.v16, "v0")); // 1 - x
            self.emit(abi::vector_fadd("v7", &k.v16, "v0")); // 1 + x
            self.emit(abi::vector_fdiv("v0", "v6", "v7")); // (1-x)/(1+x)
            self.emit(abi::vector_fsqrt("v0", "v0")); // sqrt(...) >= 0
            self.emit_atan_core(k); // v0 = atan(sqrt(...))
            self.emit(abi::vector_fadd("v0", "v0", "v0")); // 2*atan(...)
        }
    }

    /// `atan(x)` core (input in `v0`, result in `v0`): for `|x|<=1` evaluate
    /// `ax*P(ax^2)`; for `|x|>1` use `pi/2 - inv*P(inv^2)` with `inv=1/|x|`;
    /// restore the sign. Constants: v16=1.0, v17=pi/2, v18=sign mask, v19=abs
    /// mask. Reused by asin/acos. (Faithfully rounded; strict <=1 ULP needs a
    /// segmented argument reduction.)
    /// `acc[lane] = mask ? val : acc[lane]` accumulator select. `BIT acc, val,
    /// mask` inserts `val`'s bits into `acc` where `mask` is set — one instruction,
    /// leaving `mask` and `val` untouched (both are reused across atan's segments),
    /// bit-identical to the old `orr;bsl;orr` triple.
    fn emit_vsel(&mut self, acc: &str, mask: &str, val: &str) {
        self.emit(abi::vector_bit(acc, val, mask));
    }

    /// fdlibm 4-segment `atan(x)` (input/result in `v0`): reduce `|x|` into a tiny
    /// argument via one of 5 segments (breakpoints 7/16, 11/16, 19/16, 39/16),
    /// evaluate the minimax `aT` polynomial in the fdlibm `s1`/`s2` split, and
    /// recombine `atan(c) - ((reduced*P - atan_lo) - reduced)` with the segment's
    /// double-double `atan(c)`; restore the sign. Strict <=1 ULP. Persistent
    /// inputs v18=sign mask, v19=abs mask; sign parked in v25; segment masks in
    /// v28-v31; offset in v26/v27. (Reused by asin/acos/atan2.)
    fn emit_atan_core(&mut self, k: &KernelRegs) {
        self.emit(abi::vector_and("v1", "v0", &k.v19)); // ax = |x|
        self.emit(abi::vector_and(&k.v25, "v0", &k.v18)); // sign(x)
                                                          // Cumulative segment masks (ax >= threshold).
        self.broadcast_f64("v4", ATAN_SEG_THRESH[0]);
        self.emit(abi::vector_fcmge(&k.v28, "v1", "v4"));
        self.broadcast_f64("v4", ATAN_SEG_THRESH[1]);
        self.emit(abi::vector_fcmge(&k.v29, "v1", "v4"));
        self.broadcast_f64("v4", ATAN_SEG_THRESH[2]);
        self.emit(abi::vector_fcmge(&k.v30, "v1", "v4"));
        self.broadcast_f64("v4", ATAN_SEG_THRESH[3]);
        self.emit(abi::vector_fcmge(&k.v31, "v1", "v4"));
        // reduced = ax (default, segment -1); off = 0.
        self.emit(abi::vector_orr("v2", "v1", "v1"));
        self.emit(abi::vector_eor(&k.v26, &k.v26, &k.v26));
        self.emit(abi::vector_eor(&k.v27, &k.v27, &k.v27));
        // Segment 0: reduced=(2ax-1)/(2+ax), off=atan(0.5).
        self.broadcast_f64("v4", 2.0);
        self.emit(abi::vector_fadd("v5", "v1", "v1")); // 2ax
        self.broadcast_f64("v6", 1.0);
        self.emit(abi::vector_fsub("v5", "v5", "v6")); // 2ax-1
        self.emit(abi::vector_fadd("v7", "v1", "v4")); // 2+ax
        self.emit(abi::vector_fdiv("v3", "v5", "v7"));
        self.emit_vsel("v2", &k.v28, "v3");
        self.broadcast_f64("v3", ATAN_HI[0]);
        self.emit_vsel(&k.v26, &k.v28, "v3");
        self.broadcast_f64("v3", ATAN_LO[0]);
        self.emit_vsel(&k.v27, &k.v28, "v3");
        // Segment 1: reduced=(ax-1)/(ax+1), off=atan(1)=pi/4.
        self.broadcast_f64("v6", 1.0);
        self.emit(abi::vector_fsub("v5", "v1", "v6"));
        self.emit(abi::vector_fadd("v7", "v1", "v6"));
        self.emit(abi::vector_fdiv("v3", "v5", "v7"));
        self.emit_vsel("v2", &k.v29, "v3");
        self.broadcast_f64("v3", ATAN_HI[1]);
        self.emit_vsel(&k.v26, &k.v29, "v3");
        self.broadcast_f64("v3", ATAN_LO[1]);
        self.emit_vsel(&k.v27, &k.v29, "v3");
        // Segment 2: reduced=(ax-1.5)/(1+1.5ax), off=atan(1.5).
        self.broadcast_f64("v4", 1.5);
        self.emit(abi::vector_fmul("v7", "v1", "v4")); // 1.5ax
        self.broadcast_f64("v6", 1.0);
        self.emit(abi::vector_fadd("v7", "v7", "v6")); // 1+1.5ax
        self.emit(abi::vector_fsub("v5", "v1", "v4")); // ax-1.5
        self.emit(abi::vector_fdiv("v3", "v5", "v7"));
        self.emit_vsel("v2", &k.v30, "v3");
        self.broadcast_f64("v3", ATAN_HI[2]);
        self.emit_vsel(&k.v26, &k.v30, "v3");
        self.broadcast_f64("v3", ATAN_LO[2]);
        self.emit_vsel(&k.v27, &k.v30, "v3");
        // Segment 3: reduced=-1/ax, off=atan(inf)=pi/2.
        self.broadcast_f64("v6", 1.0);
        self.emit(abi::vector_fdiv("v3", "v6", "v1"));
        self.emit(abi::vector_fneg("v3", "v3"));
        self.emit_vsel("v2", &k.v31, "v3");
        self.broadcast_f64("v3", ATAN_HI[3]);
        self.emit_vsel(&k.v26, &k.v31, "v3");
        self.broadcast_f64("v3", ATAN_LO[3]);
        self.emit_vsel(&k.v27, &k.v31, "v3");
        // Polynomial: z=reduced^2, w=z^2; s1=z*odd, s2=w*even (fdlibm split).
        self.emit(abi::vector_fmul("v5", "v2", "v2")); // z
        self.emit(abi::vector_fmul("v6", "v5", "v5")); // w
        self.broadcast_f64("v3", ATAN_AT[10]);
        for &i in &[8usize, 6, 4, 2] {
            self.broadcast_f64("v4", ATAN_AT[i]);
            self.emit(abi::vector_fmla("v4", "v3", "v6"));
            self.emit(abi::vector_orr("v3", "v4", "v4"));
        }
        self.broadcast_f64("v4", ATAN_AT[0]);
        self.emit(abi::vector_fmla("v4", "v3", "v6"));
        self.emit(abi::vector_fmul("v3", "v5", "v4")); // s1 = z*(...)
        self.broadcast_f64("v7", ATAN_AT[9]);
        for &i in &[7usize, 5, 3, 1] {
            self.broadcast_f64("v4", ATAN_AT[i]);
            self.emit(abi::vector_fmla("v4", "v7", "v6"));
            self.emit(abi::vector_orr("v7", "v4", "v4"));
        }
        self.emit(abi::vector_fmul("v7", "v6", "v7")); // s2 = w*(...)
        self.emit(abi::vector_fadd("v3", "v3", "v7")); // P = s1+s2
        self.emit(abi::vector_fmul("v3", "v2", "v3")); // t = reduced*P
                                                       // result = off_hi - ((t - off_lo) - reduced).
        self.emit(abi::vector_fsub("v4", "v3", &k.v27));
        self.emit(abi::vector_fsub("v4", "v4", "v2"));
        self.emit(abi::vector_fsub("v0", &k.v26, "v4"));
        // Restore the sign (atan(|x|) >= 0).
        self.emit(abi::vector_orr("v0", "v0", &k.v25));
    }

    /// Cody-Waite reduce `x` to `r in [-pi/4, pi/4]` and quadrant `q & 3`. Leaves
    /// the reduced angle in `v2` and the quadrant (int) in `v5`. Working: v1,v3,
    /// v6,v7. Assumes the persistent trig constants in v16-v21.
    fn emit_sincos_reduce(&mut self, k: &KernelRegs) {
        self.emit(abi::vector_fmul("v1", "v0", &k.v16)); // x*invpio2
        self.emit(abi::vector_fadd("v1", "v1", &k.v17)); // +0.5
        self.emit(abi::vector_frintm("v1", "v1")); // q = floor(..)
        self.emit(abi::vector_orr("v2", "v0", "v0")); // r = x
        self.emit(abi::vector_fmls("v2", "v1", &k.v18)); // r -= q*PIO2_1
        self.emit(abi::vector_fmul("v3", "v1", &k.v19)); // w = q*PIO2_2
        self.emit(abi::vector_fsub("v6", "v2", "v3")); // y0 = r - w
        self.emit(abi::vector_fsub("v7", "v2", "v6")); // r - y0
        self.emit(abi::vector_fsub("v7", "v7", "v3")); // t = (r-y0) - w
        self.emit(abi::vector_fneg("v7", "v7")); // -t
        self.emit(abi::vector_fmla("v7", "v1", &k.v20)); // -t + q*PIO2_2T
        self.emit(abi::vector_fsub("v2", "v6", "v7")); // reduced = y0 - (..)
        self.emit(abi::vector_fcvtzs("v5", "v1")); // q (int)
        self.emit(abi::vector_and("v5", "v5", &k.v21)); // quad = q & 3
    }

    /// `sin`/`cos` kernel. After reduction, evaluate the polynomials in
    /// double-double (compensated Horner) for `sin_r = r*P_sin(r^2)` (collapsed
    /// into `v24`) and `cos_r = P_cos(r^2)` (collapsed into `v23`), then apply the
    /// quadrant selection/sign. The compensated polynomials make sin/cos strict
    /// <=1 ULP of macOS libm.
    fn emit_sin_cos_body(&mut self, want_cos: bool, k: &KernelRegs) {
        self.emit_sincos_reduce(k); // reduced=v2, quad=v5
        self.emit(abi::vector_fmul("v1", "v2", "v2")); // r2 (Horner var)
                                                       // cos_r = collapse(P_cos(r2)) → v23.
        self.emit_compensated_horner("v3", "v4", "v1", &COS_COEFFS, k);
        self.emit(abi::vector_fadd(&k.v23, "v3", "v4"));
        // sin_r = r * collapse(P_sin(r2)) → v24 (carry the lo through the multiply).
        self.emit_compensated_horner("v3", "v4", "v1", &SIN_COEFFS, k);
        self.emit_twoprod("v6", "v7", "v2", "v3");
        self.emit(abi::vector_fmla("v7", "v2", "v4")); // pe += r*lo
        self.emit(abi::vector_fadd(&k.v24, "v6", "v7"));
        // Quadrant masks: bit0 (v1) and bit1 (v0) of quad.
        self.emit(abi::vector_shl("v1", "v5", 63));
        self.emit(abi::vector_sshr("v1", "v1", 63));
        self.emit(abi::vector_shl("v0", "v5", 62));
        self.emit(abi::vector_sshr("v0", "v0", 63));
        if !want_cos {
            // sin: val = bit0 ? cos_r : sin_r; negate if bit1.
            self.emit(abi::vector_bsl("v1", &k.v23, &k.v24));
            self.emit(abi::vector_fneg("v3", "v1"));
            self.emit(abi::vector_bsl("v0", "v3", "v1"));
        } else {
            // cos: val = bit0 ? sin_r : cos_r; negate if bit0 XOR bit1.
            self.emit(abi::vector_eor("v4", "v1", "v0"));
            self.emit(abi::vector_bsl("v1", &k.v24, &k.v23));
            self.emit(abi::vector_fneg("v3", "v1"));
            self.emit(abi::vector_bsl("v4", "v3", "v1"));
            self.emit(abi::vector_orr("v0", "v4", "v4"));
        }
    }

    /// Scalar-only `sin`/`cos`: branch on the quadrant's bit0 and evaluate only
    /// the single polynomial the result needs. The array body must compute both
    /// `sin_r` and `cos_r` because its two lanes can land in different quadrants;
    /// a scalar call has one lane, so it can branch and skip the unused
    /// double-double Horner. Bit-identical to `emit_sin_cos_body` — same Cody-Waite
    /// reduction, same compensated polynomial, same sign select — for ~half the
    /// polynomial work. (`tan` still needs both halves and keeps the array body.)
    fn emit_sin_cos_body_scalar(&mut self, want_cos: bool, k: &KernelRegs) {
        self.emit_sincos_reduce(k); // reduced=v2, quad=v5
        self.emit(abi::vector_fmul("v1", "v2", "v2")); // r2 (Horner var)
                                                       // Negate mask → v25 (the Horner never touches v25/v26): sin negates on
                                                       // bit1, cos on bit0^bit1. Matches emit_sin_cos_body's branchless masks.
        self.emit(abi::vector_shl(&k.v26, "v5", 63));
        self.emit(abi::vector_sshr(&k.v26, &k.v26, 63)); // bit0 all-ones
        self.emit(abi::vector_shl(&k.v25, "v5", 62));
        self.emit(abi::vector_sshr(&k.v25, &k.v25, 63)); // bit1 all-ones
        if want_cos {
            self.emit(abi::vector_eor(&k.v25, &k.v26, &k.v25)); // bit0 XOR bit1
        }
        // Branch on bit0. sin: bit0 ? cos_r : sin_r; cos: bit0 ? sin_r : cos_r.
        self.emit(abi::vector_extract_to_x("x0", "v5", 0));
        self.emit(abi::move_immediate("x1", "Integer", "1"));
        self.emit(abi::and_registers("x0", "x0", "x1"));
        let bit0_clear = self.label("simd_sc_bit0_clear");
        let sc_done = self.label("simd_sc_done");
        self.emit(abi::compare_immediate("x0", "0"));
        self.emit(abi::branch_eq(&bit0_clear));
        // bit0 set.
        if !want_cos {
            self.emit_cos_r_into("v0", k);
        } else {
            self.emit_sin_r_into("v0", k);
        }
        self.emit(abi::branch(&sc_done));
        self.emit(abi::label(&bit0_clear));
        // bit0 clear.
        if !want_cos {
            self.emit_sin_r_into("v0", k);
        } else {
            self.emit_cos_r_into("v0", k);
        }
        self.emit(abi::label(&sc_done));
        // Apply the sign: v0 = negmask ? -v0 : v0 (BIT inserts -v0 where set).
        self.emit(abi::vector_fneg("v3", "v0"));
        self.emit(abi::vector_bit("v0", "v3", &k.v25));
        self.emit_result_nan_into_mask(k);
    }

    /// `cos_r = collapse(P_cos(r2))` into `dst` (Horner var in `v1`). The exact
    /// instruction sequence emit_sin_cos_body uses, so the result is bit-identical.
    fn emit_cos_r_into(&mut self, dst: &str, k: &KernelRegs) {
        self.emit_compensated_horner("v3", "v4", "v1", &COS_COEFFS, k);
        self.emit(abi::vector_fadd(dst, "v3", "v4"));
    }

    /// `sin_r = collapse(r * P_sin(r2))` into `dst` (reduced angle in `v2`, Horner
    /// var in `v1`); carries the lo half through the multiply, exactly as
    /// emit_sin_cos_body — bit-identical.
    fn emit_sin_r_into(&mut self, dst: &str, k: &KernelRegs) {
        self.emit_compensated_horner("v3", "v4", "v1", &SIN_COEFFS, k);
        self.emit_twoprod("v6", "v7", "v2", "v3");
        self.emit(abi::vector_fmla("v7", "v2", "v4")); // pe += r*lo
        self.emit(abi::vector_fadd(dst, "v6", "v7"));
    }

    /// `tan(x) = sin(x) / cos(x)`, strict <=1 ULP. `sin_r` and `cos_r` are
    /// computed as double-doubles (compensated Horner), the quadrant
    /// selection/sign is applied to BOTH the hi and lo halves, and the final
    /// quotient is evaluated with a one-step double-double-accurate division
    /// (`q = sh/ch; tan = q + (fma(-q,ch,sh) + (sl - q*cl))/ch`). Carrying the lo
    /// halves through the divide closes the near-pole 2-ULP residual the plain
    /// `fdiv` left. (Medium-range Cody-Waite reduction, like sin/cos; huge
    /// arguments would need Payne-Hanek, out of scope.)
    fn emit_tan_body(&mut self, k: &KernelRegs) {
        self.emit_sincos_reduce(k); // reduced=v2, quad=v5
        self.emit(abi::vector_fmul("v1", "v2", "v2")); // r2 (Horner var, survives)
                                                       // cos_r as a double-double (hi,lo) → stash in v25/v26.
        self.emit_compensated_horner("v3", "v4", "v1", &COS_COEFFS, k);
        self.emit(abi::vector_orr(&k.v25, "v3", "v3")); // cos_hi
        self.emit(abi::vector_orr(&k.v26, "v4", "v4")); // cos_lo
                                                        // sin_r = reduced * P_sin(r2) as a double-double → stash in v23/v24.
        self.emit_compensated_horner("v3", "v4", "v1", &SIN_COEFFS, k);
        self.emit_twoprod("v6", "v7", "v2", "v3"); // reduced*sin_hi → (v6,v7)
        self.emit(abi::vector_fmla("v7", "v2", "v4")); // lo += reduced*sin_lo
        self.emit(abi::vector_orr(&k.v23, "v6", "v6")); // sin_hi
        self.emit(abi::vector_orr(&k.v24, "v7", "v7")); // sin_lo
                                                        // Quadrant masks: b0 → v27, b1 → v2.
        self.emit(abi::vector_shl(&k.v27, "v5", 63));
        self.emit(abi::vector_sshr(&k.v27, &k.v27, 63));
        self.emit(abi::vector_shl("v2", "v5", 62));
        self.emit(abi::vector_sshr("v2", "v2", 63));
        // sin_full = (b1 ? -1 : 1) * (b0 ? cos_r : sin_r), as a dd → (v28,v29).
        self.emit(abi::vector_orr("v3", &k.v27, &k.v27));
        self.emit(abi::vector_bsl("v3", &k.v25, &k.v23)); // b0?cos_hi:sin_hi
        self.emit(abi::vector_orr("v4", &k.v27, &k.v27));
        self.emit(abi::vector_bsl("v4", &k.v26, &k.v24)); // b0?cos_lo:sin_lo
        self.emit(abi::vector_fneg("v6", "v3"));
        self.emit(abi::vector_fneg("v7", "v4"));
        self.emit(abi::vector_orr(&k.v28, "v2", "v2"));
        self.emit(abi::vector_bsl(&k.v28, "v6", "v3")); // sin_full_hi
        self.emit(abi::vector_orr(&k.v29, "v2", "v2"));
        self.emit(abi::vector_bsl(&k.v29, "v7", "v4")); // sin_full_lo
                                                        // cos_full = ((b0^b1) ? -1 : 1) * (b0 ? sin_r : cos_r), as a dd → (v30,v31).
        self.emit(abi::vector_orr("v3", &k.v27, &k.v27));
        self.emit(abi::vector_bsl("v3", &k.v23, &k.v25)); // b0?sin_hi:cos_hi
        self.emit(abi::vector_orr("v4", &k.v27, &k.v27));
        self.emit(abi::vector_bsl("v4", &k.v24, &k.v26)); // b0?sin_lo:cos_lo
        self.emit(abi::vector_fneg("v6", "v3"));
        self.emit(abi::vector_fneg("v7", "v4"));
        self.emit(abi::vector_eor("v1", &k.v27, "v2")); // b0^b1
        self.emit(abi::vector_orr(&k.v30, "v1", "v1"));
        self.emit(abi::vector_bsl(&k.v30, "v6", "v3")); // cos_full_hi
        self.emit(abi::vector_orr(&k.v31, "v1", "v1"));
        self.emit(abi::vector_bsl(&k.v31, "v7", "v4")); // cos_full_lo
                                                        // Double-double-accurate divide: sh=v28 sl=v29 ch=v30 cl=v31.
        self.emit_tan_divide(&k.v28, &k.v29, &k.v30, &k.v31);
    }

    /// One-step double-double-accurate quotient `tan = sh:sl / ch:cl` into `v0`:
    /// `q = sh/ch; tan = q + (fma(-q,ch,sh) + (sl - q*cl))/ch`. Reads only the
    /// four operand registers; scratch is v0/v3/v4/v6.
    fn emit_tan_divide(&mut self, sh: &str, sl: &str, ch: &str, cl: &str) {
        self.emit(abi::vector_fdiv("v0", sh, ch)); // q = sh/ch
        self.emit(abi::vector_fneg("v3", "v0")); // -q
        self.emit(abi::vector_orr("v4", sh, sh));
        self.emit(abi::vector_fmla("v4", "v3", ch)); // sh - q*ch (fma residual)
        self.emit(abi::vector_orr("v6", sl, sl));
        self.emit(abi::vector_fmls("v6", "v0", cl)); // sl - q*cl
        self.emit(abi::vector_fadd("v4", "v4", "v6")); // num
        self.emit(abi::vector_fdiv("v4", "v4", ch)); // num/ch
        self.emit(abi::vector_fadd("v0", "v0", "v4")); // tan = q + num/ch
    }

    /// Scalar-only `tan`: `tan` has period π, so the quadrant reduces to bit0 —
    /// the bit1 signs negate BOTH the numerator and denominator double-doubles
    /// together and cancel in the quotient (bit-identically). A single scalar
    /// lane can therefore branch on bit0 and pick the (num,den) dd pair directly,
    /// dropping the whole branchless sin_full/cos_full quadrant-selection block
    /// the 2-lane array body needs. The two compensated Horners still run (the
    /// ratio needs both halves); only the selection is removed.
    fn emit_tan_body_scalar(&mut self, k: &KernelRegs) {
        self.emit_sincos_reduce(k); // reduced=v2, quad=v5
        self.emit(abi::vector_fmul("v1", "v2", "v2")); // r2 (Horner var, survives)
                                                       // cos_r as a double-double (hi,lo) → v25/v26.
        self.emit_compensated_horner("v3", "v4", "v1", &COS_COEFFS, k);
        self.emit(abi::vector_orr(&k.v25, "v3", "v3")); // cos_hi
        self.emit(abi::vector_orr(&k.v26, "v4", "v4")); // cos_lo
                                                        // sin_r = reduced * P_sin(r2) as a double-double → v23/v24.
        self.emit_compensated_horner("v3", "v4", "v1", &SIN_COEFFS, k);
        self.emit_twoprod("v6", "v7", "v2", "v3"); // reduced*sin_hi → (v6,v7)
        self.emit(abi::vector_fmla("v7", "v2", "v4")); // lo += reduced*sin_lo
        self.emit(abi::vector_orr(&k.v23, "v6", "v6")); // sin_hi
        self.emit(abi::vector_orr(&k.v24, "v7", "v7")); // sin_lo
                                                        // bit0 ? -cos_r/sin_r : sin_r/cos_r (bit1 cancels in the ratio).
        self.emit(abi::vector_extract_to_x("x0", "v5", 0));
        self.emit(abi::move_immediate("x1", "Integer", "1"));
        self.emit(abi::and_registers("x0", "x0", "x1"));
        let bit0_clear = self.label("simd_tan_bit0_clear");
        let tan_done = self.label("simd_tan_done");
        self.emit(abi::compare_immediate("x0", "0"));
        self.emit(abi::branch_eq(&bit0_clear));
        // bit0 set: num = cos_r, den = -sin_r.
        self.emit(abi::vector_fneg(&k.v30, &k.v23)); // -sin_hi
        self.emit(abi::vector_fneg(&k.v31, &k.v24)); // -sin_lo
        self.emit_tan_divide(&k.v25, &k.v26, &k.v30, &k.v31);
        self.emit(abi::branch(&tan_done));
        self.emit(abi::label(&bit0_clear));
        // bit0 clear: num = sin_r, den = cos_r.
        self.emit_tan_divide(&k.v23, &k.v24, &k.v25, &k.v26);
        self.emit(abi::label(&tan_done));
        // tan fails only on a NaN result (NaN/inf input) — same as the array body.
        self.emit_result_nan_into_mask(k);
    }

    /// `exp` kernel: n=floor(x/ln2+0.5), Cody-Waite r, Horner P(r), scale 2^n.
    fn emit_exp_body(&mut self, k: &KernelRegs) {
        self.emit_exp_body_lo(None, k);
    }

    /// exp kernel. `lo` (if given) is a double-double low correction added to the
    /// reduced argument `r` — used by `pow` to evaluate `exp(y·log x)` to extra
    /// precision.
    fn emit_exp_body_lo(&mut self, lo: Option<&str>, k: &KernelRegs) {
        // NaN input → ErrFloatNan: chunk_nan = ~fcmeq(x,x); accumulate into v22.
        self.emit(abi::vector_fcmeq("v6", "v0", "v0")); // non-NaN lanes = all-ones
        self.emit(abi::vector_cmeq("v7", "v0", "v0")); // all-ones (bitwise self-eq)
        self.emit(abi::vector_eor("v6", "v6", "v7")); // NaN lanes = all-ones
        self.emit(abi::vector_orr(&k.v22, &k.v22, "v6"));
        // n = floor(x*(1/ln2) + 0.5); r = x - n*ln2 (Cody-Waite); Horner P(r).
        // v16 holds 1/ln2 (reciprocal multiply, not a divide).
        self.emit(abi::vector_fmul("v1", "v0", &k.v16));
        self.emit(abi::vector_fadd("v1", "v1", &k.v17));
        self.emit(abi::vector_frintm("v1", "v1"));
        self.emit(abi::vector_orr("v2", "v0", "v0"));
        self.emit(abi::vector_fmls("v2", "v1", &k.v18));
        self.emit(abi::vector_fmls("v2", "v1", &k.v19));
        if let Some(lo) = lo {
            self.emit(abi::vector_fadd("v2", "v2", lo)); // r += dd low part
        }
        self.emit_horner("v3", "v2", &EXP_COEFFS);
        self.emit(abi::vector_fcvtzs("v5", "v1"));
        // Overflow (result past finite range) → ErrFloatInf: n > 1023.
        self.emit(abi::vector_cmgt("v6", "v5", &k.v20));
        self.emit(abi::vector_orr(&k.v24, &k.v24, "v6")); // accumulate inf mask
        self.emit(abi::vector_cmgt("v6", &k.v23, "v5")); // underflow mask
        self.emit(abi::vector_add("v5", "v5", &k.v20));
        self.emit(abi::vector_shl("v5", "v5", 52));
        self.emit(abi::vector_fmul("v0", "v3", "v5"));
        self.emit(abi::vector_bsl("v6", &k.v21, "v0")); // flush underflow to 0
        self.emit(abi::vector_orr("v0", "v6", "v6"));
    }

    /// `log`/`log10` kernel: x = 2^k*m (frexp + fold to [1/sqrt2, sqrt2)),
    /// s=(m-1)/(m+1), ln(x) = k*ln2 + s*P(s^2), evaluated in double-double (a
    /// compensated Horner plus two-sum/two-product recombination) so the result
    /// is strict <=1 ULP; `log10` then multiplies by `1/ln10` as a double-double.
    /// Constants: v16 sqrt_half, v17 1.0, v18 0x7ff, v19 1022, v20 mantmask,
    /// v21 1022<<52, v23 1, v24/v25 ln2 hi/lo, v26/v27 1/ln10 hi/lo; v22 error.
    /// `log`/`log10` kernel. When `keep_lo` (non-base10 only), leaves the result
    /// as a double-double `hi=v0`, `lo=v31` instead of collapsing — for `pow`.
    fn emit_log_body(&mut self, base10: bool, keep_lo: bool, k: &KernelRegs) {
        // Domain: x <= 0 fails.
        self.emit(abi::vector_fcmle_zero("v1", "v0"));
        self.emit(abi::vector_orr(&k.v22, &k.v22, "v1"));
        // k = ((bits>>52) & 0x7ff) - 1022  (integer, v1).
        self.emit(abi::vector_ushr("v1", "v0", 52));
        self.emit(abi::vector_and("v1", "v1", &k.v18));
        self.emit(abi::vector_sub("v1", "v1", &k.v19));
        // m = bits with exponent field replaced by 1022 → m in [0.5, 1) (v6).
        self.emit(abi::vector_and("v6", "v0", &k.v20));
        self.emit(abi::vector_orr("v6", "v6", &k.v21));
        // if m < 1/sqrt2 { m *= 2; k -= 1 }.
        self.emit(abi::vector_fcmgt("v7", &k.v16, "v6")); // mask: sqrt_half > m
        self.emit(abi::vector_and("v0", "v7", &k.v23)); // mask & 1
        self.emit(abi::vector_sub("v1", "v1", "v0")); // k -= adjust
        self.emit(abi::vector_fadd("v0", "v6", "v6")); // m*2
        self.emit(abi::vector_bsl("v7", "v0", "v6")); // v7 = mask?m2:m  (= m)
        self.emit(abi::vector_scvtf("v3", "v1")); // k -> float (v3)
                                                  // s = (m-1)/(m+1) (v2); s2 = s*s (v1, the Horner variable).
        self.emit(abi::vector_fsub("v0", "v7", &k.v17)); // m - 1
        self.emit(abi::vector_fadd("v6", "v7", &k.v17)); // m + 1
        self.emit(abi::vector_fdiv("v2", "v0", "v6")); // s
        self.emit(abi::vector_fmul("v1", "v2", "v2")); // s2
                                                       // P(s2) as a double-double (hi=v4, lo=v5) via compensated Horner.
        self.emit_compensated_horner("v4", "v5", "v1", &LOG_COEFFS, k);
        // ln(m) = s * (hi+lo): two-product then fma the lo terms → (v7=lh, v28=le).
        self.emit_twoprod("v7", &k.v28, "v2", "v4");
        self.emit(abi::vector_fmla(&k.v28, "v2", "v5")); // le += s*lo
                                                         // k*ln2 as a double-double → (v29=kh, v30=ke).
        self.emit_twoprod(&k.v29, &k.v30, "v3", &k.v24);
        self.emit(abi::vector_fmla(&k.v30, "v3", &k.v25)); // ke += k*ln2lo
                                                           // (kh,ke) + (lh,le): two-sum hi, accumulate the lows → hi=v0, lo=v31.
                                                           // Scratch v4/v5 are dead (Horner outputs consumed).
        self.emit_twosum("v0", &k.v31, &k.v29, "v7", "v4", "v5");
        self.emit(abi::vector_fadd(&k.v31, &k.v31, &k.v30)); // + ke
        self.emit(abi::vector_fadd(&k.v31, &k.v31, &k.v28)); // + le
        if !base10 {
            if !keep_lo {
                self.emit(abi::vector_fadd("v0", "v0", &k.v31)); // ln(x) = hi + lo
            }
            // keep_lo: leave hi=v0, lo=v31 for pow's double-double y*log(x).
        } else {
            // log10(x) = (hi+lo) * (1/ln10 as hi+lo), compensated.
            self.emit_twoprod("v6", "v7", "v0", &k.v26); // ph = hi*L10HI
            self.emit(abi::vector_fmla("v7", "v0", &k.v27)); // pe += hi*L10LO
            self.emit(abi::vector_fmla("v7", &k.v31, &k.v26)); // pe += lo*L10HI
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
    fn emit_compensated_horner(
        &mut self,
        hi: &str,
        lo: &str,
        var: &str,
        coeffs: &[f64],
        k: &KernelRegs,
    ) {
        self.broadcast_f64(hi, coeffs[coeffs.len() - 1]);
        self.emit(abi::vector_eor(lo, lo, lo));
        for i in (0..coeffs.len() - 1).rev() {
            // (ph, pe) = twoprod(hi, var); pe += lo*var.
            self.emit_twoprod("v7", &k.v28, hi, var);
            self.emit(abi::vector_fmla(&k.v28, lo, var));
            // (sh, se) = twosum(c, ph). Scratch v0/v31 are free during the Horner.
            self.broadcast_f64("v6", coeffs[i]);
            self.emit_twosum(&k.v29, &k.v30, "v6", "v7", "v0", &k.v31);
            // hi = sh; lo = se + pe.
            self.emit(abi::vector_orr(hi, &k.v29, &k.v29));
            self.emit(abi::vector_fadd(lo, &k.v30, &k.v28));
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

    /// Load the constant pool base address into the kernel's pool-base register
    /// before any broadcast). adrp+add to the read-only pool data symbol.
    /// The register the SIMD math kernels address the constant pool through. If
    /// the backend pins a physical (AArch64 `x2`), return it; otherwise (x86) a
    /// per-kernel virtual register the allocator colors, minted once per function
    /// and reused for every coefficient load and the re-materialized base.
    fn math_pool_base_reg(&mut self) -> String {
        if let Some(phys) = mir::active_backend().register_model().math_pool_base() {
            return phys.to_string();
        }
        if let Some((symbol, reg)) = &self.math_pool_base_vreg {
            if *symbol == self.current_symbol {
                return reg.clone();
            }
        }
        // LinearScan's `allocate_register` is infallible (it never overflows a
        // pool — it spills); the physical-pinning backends never reach here.
        let reg = self
            .allocate_register()
            .expect("math pool-base virtual register allocation");
        self.math_pool_base_vreg = Some((self.current_symbol.clone(), reg.clone()));
        reg
    }

    fn emit_load_math_pool_base(&mut self) {
        let pool_base = self.math_pool_base_reg();
        self.emit(abi::load_page_address(&pool_base, MATH_CONST_POOL_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: MATH_CONST_POOL_SYMBOL.to_string(),
            kind: RelocIntent::DataAddrHi,
            binding: "data".to_string(),
            library: None,
        });
        self.emit(abi::add_page_offset(
            &pool_base,
            &pool_base,
            MATH_CONST_POOL_SYMBOL,
        ));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: MATH_CONST_POOL_SYMBOL.to_string(),
            kind: RelocIntent::DataAddrLo,
            binding: "data".to_string(),
            library: None,
        });
    }

    /// Broadcast an `f64` constant's bit pattern into both `.2d` lanes of `vreg`.
    /// Pooled constants load with one `ldr q` from the pool base; anything else
    /// falls back to building the immediate inline (so the pool's coverage never
    /// affects correctness).
    fn broadcast_f64(&mut self, vreg: &str, value: f64) {
        if let Some(offset) = math_const_pool_offset(value.to_bits()) {
            let pool_base = self.math_pool_base_reg();
            self.emit(abi::vector_load(vreg, &pool_base, offset));
        } else {
            self.emit(abi::move_immediate(
                "x0",
                "Integer",
                &value.to_bits().to_string(),
            ));
            self.emit(abi::vector_dup_from_x(vreg, "x0"));
        }
    }

    /// Broadcast a signed `i64` constant into both `.2d` lanes of `vreg`.
    fn broadcast_i64(&mut self, vreg: &str, value: i64) {
        if let Some(offset) = math_const_pool_offset(value as u64) {
            let pool_base = self.math_pool_base_reg();
            self.emit(abi::vector_load(vreg, &pool_base, offset));
        } else {
            self.emit(abi::move_immediate(
                "x0",
                "Integer",
                &(value as u64).to_string(),
            ));
            self.emit(abi::vector_dup_from_x(vreg, "x0"));
        }
    }

    /// Lower a two-array `math::` Float overload. `atan2` is the only kernel that
    /// reaches this driver — array `pow` is diverted to `lower_pow_array`
    /// (`builder_math.rs`) because it needs the per-element fdlibm kernel — so the
    /// error reduce below only has to cover `atan2`'s single `Nan` failure. Both
    /// lists must have the same length (`ErrInvalidArgument` otherwise).
    /// `left_slot`/`right_slot` already hold the two list pointers (the caller
    /// spilled them).
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
        self.emit(abi::move_immediate(
            "x1",
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
        let k = &self.float_kernel_regs();
        self.emit(abi::vector_eor(&k.v22, &k.v22, &k.v22));
        self.emit_float_binary_setup(kernel, k);

        let loop_label = self.label("simd_flb_loop");
        let loop_done = self.label("simd_flb_done");
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate(&pairs, "0"));
        self.emit(abi::branch_eq(&loop_done));
        self.emit(abi::vector_load("v0", &left_data, 0));
        self.emit(abi::vector_load("v1", &right_data, 0));
        self.emit_float_binary_body(kernel, k);
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
        self.emit_float_binary_body(kernel, k);
        self.emit(abi::vector_extract_to_x("x0", "v0", 0));
        self.emit(abi::store_u64("x0", &out_data, 0));
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
        debug_assert!(
            matches!(kernel, FloatBinaryKernel::Atan2),
            "lower_simd_float_binary is atan2-only; wiring an Inf-raising kernel \
             (e.g. Pow) requires hoisting the v24 zero out of the loop body"
        );
        for err in kernel.errors() {
            self.emit_float_error_reduce(*err, k)?;
        }

        Ok(ValueResult {
            type_: "List OF Float".to_string(),
            location: result_base,
            text,
        })
    }

    /// Lower a *scalar* binary `Float` overload (`atan2`/`pow`) onto the array
    /// binary kernel by broadcasting both operands into both `.2d` lanes and
    /// extracting lane 0 — the binary analog of `lower_simd_float_scalar`, so
    /// `math::f(y, x)` and `math::f([y], [x])[0]` are bit-identical. The same
    /// per-lane error checks run, so the scalar error codes match the array.
    pub(super) fn lower_simd_float_binary_scalar(
        &mut self,
        kernel: FloatBinaryKernel,
        left_loc: &str,
        right_loc: &str,
        text: String,
    ) -> Result<ValueResult, String> {
        self.emit(abi::vector_dup_from_x("v0", left_loc));
        self.emit(abi::vector_dup_from_x("v1", right_loc));
        let k = &self.float_kernel_regs();
        self.emit(abi::vector_eor(&k.v22, &k.v22, &k.v22));
        self.emit(abi::vector_eor(&k.v24, &k.v24, &k.v24)); // inf/overflow mask
        self.emit_float_binary_setup(kernel, k);
        self.emit_float_binary_body(kernel, k);
        for err in kernel.errors() {
            self.emit_float_error_reduce(*err, k)?;
        }
        let dst = self.allocate_register()?;
        self.emit(abi::vector_extract_to_x(&dst, "v0", 0));
        Ok(ValueResult {
            type_: "Float".to_string(),
            location: dst,
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
            // pow re-broadcasts the log then exp constants inside the body.
            FloatBinaryKernel::Pow => {}
        }
    }

    fn emit_float_binary_body(&mut self, kernel: FloatBinaryKernel, k: &KernelRegs) {
        match kernel {
            FloatBinaryKernel::Atan2 => {
                // atan2(y=v0, x=v1) = atan(y/x) + (x<0 ? copysign(pi, y) : 0).
                self.emit(abi::vector_fcmlt_zero(&k.v20, "v1")); // x < 0 mask
                self.emit(abi::vector_and(&k.v21, "v0", &k.v18)); // sign(y)
                self.emit(abi::vector_fdiv("v0", "v0", "v1")); // q = y/x
                self.emit_atan_core(k); // v0 = atan(q)
                self.emit(abi::vector_orr("v2", &k.v23, &k.v21)); // copysign(pi, y)
                self.emit(abi::vector_and("v2", "v2", &k.v20)); // & (x<0)
                self.emit(abi::vector_fadd("v0", "v0", "v2"));
                self.emit_result_nan_into_mask(k);
            }
            FloatBinaryKernel::Pow => {
                // pow(x=v0, y=v1) = exp(y * log(x)). Re-broadcast each kernel's
                // constants in turn; y is parked in v26 (untouched by log/exp).
                // log_body sets v22 for a non-positive base (no real result), and
                // the result-NaN check below catches NaN/overflow inputs — both
                // surface as ErrFloatNan, matching the scalar pow man page.
                self.emit(abi::vector_orr(&k.v26, "v1", "v1")); // save y
                self.emit_float_kernel_setup(FloatKernel::Log, k);
                self.emit_log_body(false, true, k); // log(x) as dd: hi=v0, lo=v31
                                                    // y*log(x) as a double-double (v0 = hi, v27 = lo).
                self.emit_twoprod("v2", "v3", &k.v26, "v0"); // y*log_hi
                self.emit(abi::vector_fmla("v3", &k.v26, &k.v31)); // + y*log_lo
                self.emit(abi::vector_orr("v0", "v2", "v2"));
                self.emit(abi::vector_orr(&k.v27, "v3", "v3"));
                self.emit_float_kernel_setup(FloatKernel::Exp, k);
                self.emit_exp_body_lo(Some(&k.v27), k); // exp((y*log x) as dd)
                self.emit_result_nan_into_mask(k);
            }
        }
    }
}

/// A two-array `math::` Float kernel.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum FloatBinaryKernel {
    /// `atan2(y, x)`.
    Atan2,
    /// `pow(base, exponent)`.
    Pow,
}

impl FloatBinaryKernel {
    /// The error checks this kernel performs, matching the scalar man pages.
    /// `atan2` only ever fails with a NaN result (NaN/inf input); `pow` (Phase 4)
    /// also raises `ErrFloatInf` on overflow (its `exp` core sets the `v24` mask).
    fn errors(self) -> &'static [FloatError] {
        match self {
            FloatBinaryKernel::Atan2 => &[FloatError::Nan],
            FloatBinaryKernel::Pow => &[FloatError::Nan, FloatError::Inf],
        }
    }
}
