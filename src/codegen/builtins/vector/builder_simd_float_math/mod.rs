// These kernels deliberately spell out full-precision fdlibm/Remez constants
// (the `hi` halves of double-double reductions such as `LN2`, `LOG10_E`,
// `INV_PIO2`). They sit near `std::f64::consts::*` but must NOT be swapped for
// them — the std const is a single rounded double, whereas these are paired with
// their `lo` tails to recombine past double precision. Allow the deny-by-default
// correctness lint for the whole module.
#![allow(clippy::approx_constant)]
// Same reasoning, same constants: `excessive_precision` fires on the digits past
// what a lone `f64` can hold, which is exactly the point — the extra digits are
// what the paired `lo` tail recombines against. Trimming them to silence the
// lint would silently degrade `exp`/`log`/`sin`/`cos` accuracy, so the lint is
// allowed and the literals stay verbatim (bug-345-D2).
#![allow(clippy::excessive_precision)]

// --- codegen tier imports (migration) ---
use crate::codegen::builtins::vector::{COS_COEFFS, EXP_COEFFS, LOG_COEFFS, SIN_COEFFS};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::mir;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::target::shared::abi::Reg;
use crate::types::ParameterType;

mod dispatch;
mod exp_log;
mod inverse_trig;
mod primitives;
mod trig;
mod trig_reduction;
// NEON f64 polynomial kernels for the Float transcendentals — plan-01-simd
// Phase 5. Hand-written, identical on every target, no external math library.
// Coefficients come from `simd_kernel_coeffs.rs` (Remez minimax, validated <=1
// ULP against the committed macOS-libm reference vectors in
// `tools/math-kernels/ref/`). The odd tail reuses the vector kernel by
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
/// `exp` overflow/underflow guards (bug-164). Above `ln(DBL_MAX)` the true result
/// overflows to `+Inf`; below `ln(2^-1075)` it rounds to `+0.0`. Beyond these the
/// Cody-Waite reduction and `2^n` scaling produce garbage, so out-of-range lanes
/// are saturated directly. Values match glibc's `o_threshold` / `u_threshold`.
const EXP_OVERFLOW_THRESHOLD: f64 = 709.782_712_893_383_973_096;
const EXP_UNDERFLOW_THRESHOLD: f64 = -745.133_219_101_941_108_42;
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
/// `2/pi` and the fdlibm three-part `pi/2` for the sin/cos Cody-Waite reduction.
/// It is exact while `q*PIO2_1` and `q*PIO2_2` are (`|x| < 2^20 * pi/2`), and its
/// truncated `pi/2` leaves an absolute error of about `3e-37 * |x|`, so it is
/// used only below [`TRIG_MEDIUM_LIMIT`] and only when the reduced angle has not
/// cancelled below [`TRIG_CANCEL_RATIO`]` * |x|`. Every other finite lane takes
/// the exact table reduction `emit_rem_pio2_large_lane` (bug-618).
const INV_PIO2: f64 = 0.636_619_772_367_581_343_1;
const PIO2_1: f64 = 1.570_796_326_734_125_614_17;
const PIO2_2: f64 = 6.077_100_506_303_965_976_60e-11;
const PIO2_2T: f64 = 2.022_266_248_795_950_631_54e-21;
/// `2^20 * pi/2` (bits `0x413921FB54442D18`): the medium reduction's upper bound.
const TRIG_MEDIUM_LIMIT: f64 = 1_647_099.329_165_285_5;
/// `2^-50`: a medium-reduced angle smaller than this fraction of `|x|` has lost
/// enough leading bits to the cancellation that the Cody-Waite constants'
/// `3e-37 * |x|` error could reach its last ULP, so that lane is re-reduced
/// exactly. Kept lanes have a relative reduction error below `3e-37 * 2^50`
/// (about `3.4e-22`).
const TRIG_CANCEL_RATIO: f64 = 8.881_784_197_001_252_3e-16;
/// `pi/2` as a double-double: [`std::f64::consts::FRAC_PI_2`] is the hi half.
const PIO2_DD_LO: f64 = 6.123_233_995_736_766_035_87e-17;
/// Scales of the three 53-bit pieces the exact reduction splits its fraction into.
const TWO_POW_M54: f64 = 5.551_115_123_125_782_702_1e-17;
const TWO_POW_M107: f64 = 6.162_975_822_039_154_7e-33;
const TWO_POW_M160: f64 = 6.842_277_657_836_020_854_4e-49;
/// Leading zero bits in front of `2/pi` in [`TWO_OVER_PI_WORDS`].
const TWO_OVER_PI_PAD: usize = 55;
/// `floor(2/pi * 2^1225)` as 20 big-endian 64-bit words: bit `t` (0 = the MSB of
/// word 0) is bit `t - 54` after the binary point of `2/pi`, so the first word
/// holds 55 zero bits and then the leading 9 bits of `2/pi` (`0x145` =
/// `0b1_0100_0101`, from `2/pi = 0x0.A2F9836E4E44...`). The table reaches bit 1225
/// of `2/pi`; the widest window (`f64::MAX`, words 16-19) ends at its last word.
///
/// Generated offline in pure integer arithmetic (no host floating point):
///
/// ```text
/// def atan_inv(n, W):                  # atan(1/n) * 2^W, Taylor series
///     n2, power, s, k = n*n, (1 << W) // n, 0, 0
///     while power:
///         t = power // (2*k + 1); s = s + t if k % 2 == 0 else s - t
///         power //= n2; k += 1
///     return s
/// pi = 16*atan_inv(5, W) - 4*atan_inv(239, W)          # Machin, pi * 2^W
/// v = (1 << (64*20 - 55 + 1 + W)) // pi                # 2/pi * 2^1225
/// words = [(v >> (64*(19 - i))) & (2**64 - 1) for i in range(20)]
/// ```
///
/// `W = 1400` and `W = 2400` give identical words, and the leading bits match
/// fdlibm's `two_over_pi` table (`0xA2F983, 0x6E4E44, 0x1529FC, ...`).
const TWO_OVER_PI_WORDS: [u64; 20] = [
    0x0000_0000_0000_0145,
    0xF306_DC9C_882A_53F8,
    0x4EAF_A3EA_69BB_81B6,
    0xC52B_3278_8720_83FC,
    0xA2C7_57BD_778A_C36E,
    0x48DC_7484_9BA5_C00C,
    0x925D_D413_A324_39FC,
    0x3BD6_3962_534E_7DD1,
    0x046B_EA5D_7689_09D3,
    0x38E0_4D68_BEFC_8273,
    0x23AC_7306_A673_E939,
    0x08BF_177B_F250_763F,
    0xF12F_FFBC_0B30_1FDE,
    0x5E23_16B4_14DA_3EDA,
    0x6CFD_9E4F_9613_6E9E,
    0x8C7E_CD3C_BFD4_5AEA,
    0x4F75_8FD7_CBE2_F67A,
    0x0E73_EF14_A525_D4D7,
    0xF6BF_623F_1ABA_10AC,
    0x0660_8DF8_F6D7_57E1,
];

/// fdlibm `atan` reduction: the four segment thresholds and their breakpoint
/// `atan(c)` values as hi/lo double-doubles (four thresholds partition `|x|`
/// into five segments), and the minimax polynomial `aT` for the reduced argument.
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
pub(crate) const MATH_CONST_POOL_SYMBOL: &str = "_mfb_math_const_pool";

/// The deduplicated constant words (f64/i64 bit patterns) the SIMD kernels
/// broadcast, in a fixed order. Each occupies a 16-byte slot (the value in both
/// `.2d` lanes), so its byte offset is `index * 16`.
pub(crate) fn math_const_pool_words() -> Vec<u64> {
    // Built once and memoized (bug-332 G2): the layout is order-dependent and three
    // things derive from it (broadcast offsets, the data-object hex, mod.rs's size),
    // so all three must see the *same* order — memoizing at this level guarantees
    // that and drops the per-broadcast O(n^2) `contains` dedup rebuild. The
    // `math_const_pool_layout_is_pinned` test enforces the cross-derivation agreement.
    use std::sync::OnceLock;
    static POOL: OnceLock<Vec<u64>> = OnceLock::new();
    POOL.get_or_init(|| {
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
        // bug-618: the trig reduction's routing and exact-reduction constants.
        // Appended after every earlier constant, so no existing offset moves.
        for v in [
            TRIG_MEDIUM_LIMIT,
            TRIG_CANCEL_RATIO,
            f64::INFINITY,
            PIO2_DD_LO,
            TWO_POW_M54,
            TWO_POW_M107,
            TWO_POW_M160,
        ] {
            add(&mut w, v.to_bits());
        }
        // The `2/pi` table last, verbatim (NOT deduplicated): the exact reduction
        // indexes it as `pool + TABLE + 16*j`, so its words must stay contiguous
        // and in order even if one equals a constant above.
        w.extend_from_slice(&TWO_OVER_PI_WORDS);
        w
    })
    .clone()
}

/// Byte offset of [`TWO_OVER_PI_WORDS`]`[0]`'s 16-byte slot in the pool.
fn two_over_pi_table_offset() -> usize {
    (math_const_pool_words().len() - TWO_OVER_PI_WORDS.len()) * 16
}

/// The 16-byte-slot byte offset of `bits` in the pool, or `None` if not pooled.
/// Only the deduplicated constant section is searched — the `2/pi` table is
/// addressed by index, never by value.
fn math_const_pool_offset(bits: u64) -> Option<usize> {
    let words = math_const_pool_words();
    words[..words.len() - TWO_OVER_PI_WORDS.len()]
        .iter()
        .position(|word| *word == bits)
        .map(|index| index * 16)
}

/// Hex (the `raw` data-object `value`) for the pool: each word little-endian,
/// written twice to fill both `.2d` lanes of its 16-byte slot.
pub(crate) fn math_const_pool_data_value() -> String {
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
pub(crate) enum FloatKernel {
    /// `e^x`; `ErrOverflow` when the result exceeds the finite double range.
    Exp,
    /// `ln(x)` / `log10(x)`; `ErrInvalidArgument` on a non-positive lane.
    Log,
    Log10,
    /// `sin(x)` / `cos(x)` / `tan(x)`; `ErrFloatNan` only for a NaN/inf input.
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
    v16: Reg,
    v17: Reg,
    v18: Reg,
    v19: Reg,
    v20: Reg,
    v21: Reg,
    v22: Reg,
    v23: Reg,
    v24: Reg,
    v25: Reg,
    v26: Reg,
    v27: Reg,
    v28: Reg,
    v29: Reg,
    v30: Reg,
    v31: Reg,
    /// The sin/cos/tan reduction's extra registers; `None` for every other kernel.
    trig: Option<TrigRegs>,
}

/// Registers only the trig kernels use (bug-618), minted with their
/// [`KernelRegs`]. `limit`/`cancel`/`inf`/`one` are persistent constants
/// broadcast by `emit_float_kernel_setup`; the other three are written by
/// `emit_sincos_reduce` on every body and read by the polynomial evaluators.
struct TrigRegs {
    /// [`TRIG_MEDIUM_LIMIT`].
    limit: Reg,
    /// [`TRIG_CANCEL_RATIO`].
    cancel: Reg,
    /// `+inf`.
    inf: Reg,
    /// `1.0`.
    one: Reg,
    /// Per-lane all-ones where the lane takes the exact reduction.
    route: Reg,
    /// The low half's first-order effect on `sin(r)`: `r_lo * (1 - r^2/2)`.
    sin_corr: Reg,
    /// The low half's first-order effect on `cos(r)`: `-r * r_lo`.
    cos_corr: Reg,
    /// The untouched argument, saved by `sin`/`tan` before the reduction
    /// overwrites it, so a zero lane can keep its sign (see
    /// [`CodeBuilder::emit_zero_argument_passthrough`]). `cos` never reads it.
    arg: Reg,
}

impl KernelRegs {
    /// The trig registers. Only the sin/cos/tan emitters call this, and their
    /// `KernelRegs` is always minted with `float_kernel_regs(true)`.
    fn trig(&self) -> &TrigRegs {
        self.trig
            .as_ref()
            .expect("sin/cos/tan kernel regs are minted with their TrigRegs")
    }
}

impl FloatKernel {
    fn is_trig(self) -> bool {
        matches!(self, FloatKernel::Sin | FloatKernel::Cos | FloatKernel::Tan)
    }
}

/// A two-array `math::` Float kernel. `atan2` is the sole SIMD binary kernel;
/// array/scalar `pow` runs the per-element scalar fdlibm kernel
/// (`emit_pow_scalar`/`lower_pow_array`, bug-68), not this driver.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum FloatBinaryKernel {
    /// `atan2(y, x)`.
    Atan2,
}

impl FloatBinaryKernel {
    /// The error checks this kernel performs, matching the scalar man pages.
    /// `atan2` only ever fails with a NaN result (NaN/inf input).
    fn errors(self) -> &'static [FloatError] {
        match self {
            FloatBinaryKernel::Atan2 => &[FloatError::Nan],
        }
    }
}

#[cfg(test)]
mod pool_layout_tests {
    use super::*;

    /// bug-332 G2 / Phase 1: the kernel constant pool is order-dependent and
    /// positional — three things derive from `math_const_pool_words()`'s order (the
    /// immediate offsets baked into every broadcast, the data-object hex, and the
    /// declared size in `mod.rs`), and nothing else pinned the layout. This test
    /// enforces that those derivations agree, so memoizing `math_const_pool_words`
    /// (G2) — or any future coefficient edit — cannot silently make the emitted
    /// offsets and the emitted data blob disagree.
    ///
    /// bug-618 split the pool in two: the value-addressed constant section this
    /// test always pinned, and the `2/pi` table appended after it, which the exact
    /// trig reduction addresses BY INDEX (`pool + table + 16*j`) and which is
    /// therefore deliberately neither deduplicated nor reachable through
    /// `math_const_pool_offset`. The constant section keeps every assertion it had;
    /// the table gets the stronger positional one (word `i` sits at
    /// `table_offset + 16*i`, verbatim and in order), since that is what its
    /// indexed load depends on.
    #[test]
    fn math_const_pool_layout_is_pinned() {
        let words = math_const_pool_words();
        assert!(!words.is_empty(), "the constant pool must not be empty");
        let constants = words.len() - TWO_OVER_PI_WORDS.len();
        assert_eq!(
            two_over_pi_table_offset(),
            constants * 16,
            "the 2/pi table must start right after the constant section"
        );

        // The constant section dedups on insertion; a repeat would mean that
        // invariant broke.
        let mut seen = std::collections::HashSet::new();
        for &w in &words[..constants] {
            assert!(seen.insert(w), "duplicate word {w:#x} in the constant pool");
        }

        // Every constant's byte offset is exactly its index * 16, and the reverse
        // lookup agrees — this is what the broadcast immediates depend on.
        for (i, &w) in words[..constants].iter().enumerate() {
            assert_eq!(
                math_const_pool_offset(w),
                Some(i * 16),
                "offset for word[{i}] = {w:#x} disagrees with its position"
            );
        }

        // The 2/pi table is stored verbatim, in order, immediately after them, and
        // no value lookup ever resolves into it (a table word that happens to equal
        // a constant must still resolve to the constant).
        for (i, &w) in TWO_OVER_PI_WORDS.iter().enumerate() {
            assert_eq!(
                words[constants + i],
                w,
                "2/pi table word {i} is not at its slot"
            );
            assert!(
                math_const_pool_offset(w).is_none_or(|offset| offset < constants * 16),
                "value lookup of {w:#x} resolved into the 2/pi table"
            );
        }

        // The emitted data object stores two 8-byte lanes per word: 32 hex chars
        // each. If this and the offsets ever disagree, every transcendental would
        // read the wrong constant.
        assert_eq!(
            math_const_pool_data_value().len(),
            words.len() * 32,
            "data-object hex length must equal words.len() * 32 (2 lanes * 8 bytes * 2 hex chars)"
        );

        // Memoization must not change what a second call returns.
        assert_eq!(
            math_const_pool_words(),
            words,
            "math_const_pool_words() is not stable across calls"
        );
    }
}
