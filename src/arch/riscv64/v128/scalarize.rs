//! plan-176-O: scalarizing a V128 run into 64-bit lane ops. The slot/vreg maps and the shared helpers live in
//! `riscv64/v128/mod.rs`.

use super::*;

/// Scalarize one `v128` CodeOp into RV64GC scalar/memory ops (plan-99 §6), using
/// `slots` to place each vector value.
pub(crate) fn scalarize_v128(
    op: CodeOp,
    fields: &[(&'static str, Operand)],
    slots: &HashMap<String, usize>,
) -> Vec<CodeInstruction> {
    let mut out = Vec::new();
    // Materialize the per-thread slot base into t2: `addi t2, s11,
    // ARENA_V128_SLOTS_OFFSET`. The slots live in the per-thread arena state
    // (addressed off the pinned arena base `s11`), so each OS thread gets its own
    // region — the old process-global `_mfb_rt_v128_slots` was corrupted by two
    // worker threads running v128 kernels concurrently (bug-122). t0 is free at
    // this point (lanes are loaded after), so the immediate materialization the
    // encoder would use for an out-of-range offset is harmless here.
    out.push(ci(
        "add_imm",
        &[
            ("dst", T2),
            ("src", crate::arch::riscv64::regmodel::ARENA_BASE_REGISTER),
            (
                "imm",
                &crate::codegen::error::constants::ARENA_V128_SLOTS_OFFSET.to_string(),
            ),
        ],
    ));

    let off = |name: &str, half: u8| -> String {
        let idx = *slots
            .get(name)
            .unwrap_or_else(|| panic!("rv64 v128: no slot for '{name}'"));
        (idx * 16 + half as usize * 8).to_string()
    };
    let fld = |o: &mut Vec<CodeInstruction>, dst: &str, name: &str, half: u8| {
        o.push(ci(
            "ldr_d",
            &[("dst", dst), ("base", T2), ("offset", &off(name, half))],
        ));
    };
    let fsd = |o: &mut Vec<CodeInstruction>, src: &str, name: &str, half: u8| {
        o.push(ci(
            "str_d",
            &[("src", src), ("base", T2), ("offset", &off(name, half))],
        ));
    };
    let ild = |o: &mut Vec<CodeInstruction>, dst: &str, name: &str, half: u8| {
        o.push(ci(
            "ldr_u64",
            &[("dst", dst), ("base", T2), ("offset", &off(name, half))],
        ));
    };
    let isd = |o: &mut Vec<CodeInstruction>, src: &str, name: &str, half: u8| {
        o.push(ci(
            "str_u64",
            &[("src", src), ("base", T2), ("offset", &off(name, half))],
        ));
    };

    // bug-284 C5: the high lane sits 8 bytes past the low lane, so its offset has
    // to be derived from the same value the low-lane op uses. `unwrap_or(0) + 8`
    // instead produced the literal offset 8 for any non-u64 spelling (negative,
    // symbolic, empty) while the low-lane op forwarded the raw string -- the two
    // lanes then addressed inconsistent locations. That only ever failed loudly
    // because the sibling op happened to hand the same string to
    // `operand::immediate()`, which rejects it; correctness must not rest on that
    // coincidence, so this arm fails on its own terms.
    fn high_lane_offset(offset: &str, op: &str) -> String {
        match offset.parse::<u64>() {
            Ok(value) => (value + 8).to_string(),
            Err(_) => panic!("rv64 v128: {op} needs a numeric offset, got {offset:?}"),
        }
    }

    use CodeOp::*;
    match op {
        // --- 128-bit memory load/store (16 bytes, no lane interpretation) ------
        LdrQ => {
            let (dst, base, o) = (f(fields, "dst"), f(fields, "base"), f(fields, "offset"));
            let o8 = high_lane_offset(&o, "ldr_q");
            // Value in T1: a large `base` offset makes the encoder use T0 as the
            // address scratch, which would clobber the value if it were in T0.
            out.push(ci(
                "ldr_u64",
                &[("dst", T1), ("base", &base), ("offset", &o)],
            ));
            isd(&mut out, T1, &dst, 0);
            out.push(ci(
                "ldr_u64",
                &[("dst", T1), ("base", &base), ("offset", &o8)],
            ));
            isd(&mut out, T1, &dst, 1);
        }
        StrQ => {
            let (src, base, o) = (f(fields, "src"), f(fields, "base"), f(fields, "offset"));
            let o8 = high_lane_offset(&o, "str_q");
            ild(&mut out, T1, &src, 0);
            out.push(ci(
                "str_u64",
                &[("src", T1), ("base", &base), ("offset", &o)],
            ));
            ild(&mut out, T1, &src, 1);
            out.push(ci(
                "str_u64",
                &[("src", T1), ("base", &base), ("offset", &o8)],
            ));
        }
        // --- FP three-same `.2d` ----------------------------------------------
        FAddV | FSubV | FMulV | FDivV => {
            let mn = match op {
                FAddV => "fadd_d",
                FSubV => "fsub_d",
                FMulV => "fmul_d",
                _ => "fdiv_d",
            };
            let (d, a, b) = (f(fields, "dst"), f(fields, "lhs"), f(fields, "rhs"));
            for h in 0..2 {
                fld(&mut out, FT0, &a, h);
                fld(&mut out, FT1, &b, h);
                out.push(ci(mn, &[("dst", FT0), ("lhs", FT0), ("rhs", FT1)]));
                fsd(&mut out, FT0, &d, h);
            }
        }
        // Lane min/max with IEEE number semantics (a finite operand wins over a
        // NaN). `fminnm_d`/`fmaxnm_d` match both the aarch64 vector body (`fmin_v`/
        // `fmax_v`, equal on the finite inputs guaranteed here) and the scalar
        // `math::min/max(Float)` tail (`float_min_d`/`float_max_d`). Without these
        // arms `math::min/max/clamp(List OF Float)` ICE'd on riscv (bug-121).
        FMinV | FMaxV => {
            let mn = if op == FMinV { "fminnm_d" } else { "fmaxnm_d" };
            let (d, a, b) = (f(fields, "dst"), f(fields, "lhs"), f(fields, "rhs"));
            for h in 0..2 {
                fld(&mut out, FT0, &a, h);
                fld(&mut out, FT1, &b, h);
                out.push(ci(mn, &[("dst", FT0), ("lhs", FT0), ("rhs", FT1)]));
                fsd(&mut out, FT0, &d, h);
            }
        }
        // Fused multiply-add: dst += lhs*rhs (single rounding).
        FMlaV => {
            let (d, a, b) = (f(fields, "dst"), f(fields, "lhs"), f(fields, "rhs"));
            for h in 0..2 {
                fld(&mut out, FT0, &d, h);
                fld(&mut out, FT1, &a, h);
                fld(&mut out, FT2, &b, h);
                out.push(ci(
                    "fmadd_d",
                    &[("dst", FT0), ("addend", FT0), ("lhs", FT1), ("rhs", FT2)],
                ));
                fsd(&mut out, FT0, &d, h);
            }
        }
        // Fused multiply-subtract: dst -= lhs*rhs (single rounding, per the op's
        // contract in mir.rs). `fnmsub_d` = NMSUB = addend − lhs*rhs, i.e.
        // d − a*b in one rounding — the scalarized fmul+fsub used two (bug-158).
        FMlsV => {
            let (d, a, b) = (f(fields, "dst"), f(fields, "lhs"), f(fields, "rhs"));
            for h in 0..2 {
                fld(&mut out, FT0, &d, h);
                fld(&mut out, FT1, &a, h);
                fld(&mut out, FT2, &b, h);
                out.push(ci(
                    "fnmsub_d",
                    &[("dst", FT0), ("addend", FT0), ("lhs", FT1), ("rhs", FT2)],
                ));
                fsd(&mut out, FT0, &d, h);
            }
        }
        // --- FP two-reg-misc `.2d` --------------------------------------------
        FAbsV | FNegV | FSqrtV => {
            let mn = match op {
                FAbsV => "fabs_d",
                FNegV => "fneg_d",
                _ => "fsqrt_d",
            };
            let (d, s) = (f(fields, "dst"), f(fields, "src"));
            for h in 0..2 {
                fld(&mut out, FT0, &s, h);
                out.push(ci(mn, &[("dst", FT0), ("src", FT0)]));
                fsd(&mut out, FT0, &d, h);
            }
        }
        // Round each lane to an integral f64 by rounding mode. The naive
        // f64→i64→f64 round-trip has two defects (bug-126.1): (1) a lane with
        // |x| ≥ 2^63 saturates the i64 (and a non-finite lane traps/produces
        // garbage), corrupting values that are *already* integral; and (2) there
        // is no nearest-ties-to-EVEN i64 converter, so `FRintnV` (frintn) must not
        // reuse the ties-AWAY `fcvtas` (RMM). Both are fixed here, branchlessly
        // (the per-function slot model has only t0/t1 integer scratch and no local
        // label generator, so a mask-select is both simpler and deterministic):
        //
        //   * Only lanes with |x| < 2^52 have a fractional part; a lane with
        //     |x| ≥ 2^52 (which includes every ±Inf/NaN, whose |bits| exceed
        //     2^52's) is already integral and is kept verbatim. `mask` is all-ones
        //     for the convert lanes, and the result is selected bitwise
        //     `(converted & mask) | (x & ~mask)` — so the i64 round-trip, which
        //     only runs where |x| < 2^52 < 2^63, can never saturate.
        //   * `FRintnV` rounds ties-to-even via the 2^52 magic-number add/sub
        //     (`fadd_d`/`fsub_d` are hard-wired to RNE) with x's sign restored,
        //     matching AArch64 `frintn` (including `frintn(-0.3) = -0.0`).
        //
        // The four directed modes keep the mode-specific `fcvt`; a zero *result*
        // from them carries `+0.0` (the pre-existing sign-of-zero behavior of the
        // i64 round-trip) rather than AArch64's signed zero — unchanged here.
        FRintmV | FRintpV | FRintzV | FRintaV | FRintnV => {
            const TWO52_BITS: &str = "4841369599423283200"; // bits(2^52) = 0x4330000000000000
            let (d, s) = (f(fields, "dst"), f(fields, "src"));
            for h in 0..2 {
                fld(&mut out, FT0, &s, h); // ft0 = x (preserved through the select)
                                           // converted (ft1) = round(x) in the requested mode.
                if op == FRintnV {
                    // Nearest-ties-even via the 2^52 magic number, then restore
                    // x's sign so ±0 and small negatives round to the right zero.
                    out.push(ci("fabs_d", &[("dst", FT2), ("src", FT0)])); // |x|
                    out.push(ci("mov_imm", &[("dst", T0), ("value", TWO52_BITS)]));
                    out.push(ci("fmov_d_from_x", &[("dst", FT1), ("src", T0)])); // ft1 = 2^52
                    out.push(ci("fadd_d", &[("dst", FT2), ("lhs", FT2), ("rhs", FT1)])); // |x|+2^52
                    out.push(ci("fsub_d", &[("dst", FT2), ("lhs", FT2), ("rhs", FT1)])); // round(|x|)
                    out.push(ci("fmov_x_from_d", &[("dst", T0), ("src", FT2)])); // round bits (sign 0)
                    out.push(ci("fmov_x_from_d", &[("dst", T1), ("src", FT0)])); // x bits
                    out.push(ci("lsr_imm", &[("dst", T1), ("src", T1), ("shift", "63")])); // signbit
                    out.push(ci("lsl_imm", &[("dst", T1), ("src", T1), ("shift", "63")])); // sign mask
                    out.push(ci("orr", &[("dst", T0), ("lhs", T0), ("rhs", T1)])); // apply sign
                    out.push(ci("fmov_d_from_x", &[("dst", FT1), ("src", T0)]));
                // ft1 = converted
                } else {
                    let cvt = match op {
                        FRintmV => "fcvtms_x_from_d", // toward -inf
                        FRintpV => "fcvtps_x_from_d", // toward +inf
                        FRintzV => "fcvtzs_x_from_d", // toward zero
                        _ => "fcvtas_x_from_d",       // FRintaV: nearest ties away
                    };
                    out.push(ci(cvt, &[("dst", T0), ("src", FT0)]));
                    out.push(ci("scvtf_d_from_x", &[("dst", FT1), ("src", T0)]));
                    // ft1 = converted
                }
                // mask (t1) = all-ones iff |x| < 2^52 (a lane with a fractional
                // part), else 0. Unsigned bit compare: |bits| of any finite
                // |x| ≥ 2^52 — and of every ±Inf/NaN — is ≥ bits(2^52).
                out.push(ci("fmov_x_from_d", &[("dst", T0), ("src", FT0)])); // x bits
                out.push(ci("lsl_imm", &[("dst", T1), ("src", T0), ("shift", "1")])); // drop sign bit
                out.push(ci("lsr_imm", &[("dst", T1), ("src", T1), ("shift", "1")])); // t1 = |bits|
                out.push(ci("mov_imm", &[("dst", T0), ("value", TWO52_BITS)]));
                out.push(ci("rv.sltu", &[("dst", T1), ("lhs", T1), ("rhs", T0)])); // |bits| < 2^52 ?
                out.push(ci("sub", &[("dst", T1), ("lhs", ZERO), ("rhs", T1)])); // 0/1 → 0/all-ones
                                                                                 // result (ft0) = (converted & mask) | (x & ~mask).
                out.push(ci("fmov_x_from_d", &[("dst", T0), ("src", FT1)])); // converted bits
                out.push(ci("and", &[("dst", T0), ("lhs", T0), ("rhs", T1)])); // converted & mask
                out.push(ci("fmov_d_from_x", &[("dst", FT1), ("src", T0)])); // park in ft1
                out.push(ci("fmov_x_from_d", &[("dst", T0), ("src", FT0)])); // x bits
                out.push(ci("mvn", &[("dst", T1), ("src", T1)])); // ~mask
                out.push(ci("and", &[("dst", T0), ("lhs", T0), ("rhs", T1)])); // x & ~mask
                out.push(ci("fmov_x_from_d", &[("dst", T1), ("src", FT1)])); // (converted & mask) bits
                out.push(ci("orr", &[("dst", T0), ("lhs", T0), ("rhs", T1)])); // combine
                out.push(ci("fmov_d_from_x", &[("dst", FT0), ("src", T0)])); // ft0 = result
                fsd(&mut out, FT0, &d, h);
            }
        }
        // Lane f64→i64 conversions (result is an i64 in the slot).
        FCvtzsV | FCvtasV => {
            let cvt = if op == FCvtzsV {
                "fcvtzs_x_from_d"
            } else {
                "fcvtas_x_from_d"
            };
            let (d, s) = (f(fields, "dst"), f(fields, "src"));
            for h in 0..2 {
                fld(&mut out, FT0, &s, h);
                out.push(ci(cvt, &[("dst", T0), ("src", FT0)]));
                isd(&mut out, T0, &d, h);
            }
        }
        ScvtfV => {
            let (d, s) = (f(fields, "dst"), f(fields, "src"));
            for h in 0..2 {
                ild(&mut out, T0, &s, h);
                out.push(ci("scvtf_d_from_x", &[("dst", FT0), ("src", T0)]));
                fsd(&mut out, FT0, &d, h);
            }
        }
        // --- FP lane compares → all-ones/all-zeros mask -----------------------
        FCmGtV | FCmGeV | FCmEqV => {
            let (d, a, b) = (f(fields, "dst"), f(fields, "lhs"), f(fields, "rhs"));
            for h in 0..2 {
                fld(&mut out, FT0, &a, h);
                fld(&mut out, FT1, &b, h);
                // gt: b<a ; ge: b<=a ; eq: a==b (all ordered).
                let (l, r, cmp) = match op {
                    FCmGtV => (FT1, FT0, "lt"),
                    FCmGeV => (FT1, FT0, "le"),
                    _ => (FT0, FT1, "eq"),
                };
                out.push(ci(
                    "rv.fcmp",
                    &[("dst", T0), ("lhs", l), ("rhs", r), ("cmp", cmp)],
                ));
                out.push(ci("sub", &[("dst", T0), ("lhs", ZERO), ("rhs", T0)])); // mask = -bool
                isd(&mut out, T0, &d, h);
            }
        }
        // FP compare-against-zero → mask.
        FCmGtZeroV | FCmGeZeroV | FCmEqZeroV | FCmLtZeroV | FCmLeZeroV => {
            let (d, s) = (f(fields, "dst"), f(fields, "src"));
            for h in 0..2 {
                out.push(ci("fmov_d_from_x", &[("dst", FT1), ("src", ZERO)])); // ft1 = +0.0
                fld(&mut out, FT0, &s, h);
                let (l, r, cmp) = match op {
                    FCmGtZeroV => (FT1, FT0, "lt"), // 0 < a
                    FCmGeZeroV => (FT1, FT0, "le"), // 0 <= a
                    FCmEqZeroV => (FT0, FT1, "eq"), // a == 0
                    FCmLtZeroV => (FT0, FT1, "lt"), // a < 0
                    _ => (FT0, FT1, "le"),          // a <= 0
                };
                out.push(ci(
                    "rv.fcmp",
                    &[("dst", T0), ("lhs", l), ("rhs", r), ("cmp", cmp)],
                ));
                out.push(ci("sub", &[("dst", T0), ("lhs", ZERO), ("rhs", T0)]));
                isd(&mut out, T0, &d, h);
            }
        }
        // --- Integer three-same `.2d` -----------------------------------------
        AddV | SubV => {
            let mn = if op == AddV { "add" } else { "sub" };
            let (d, a, b) = (f(fields, "dst"), f(fields, "lhs"), f(fields, "rhs"));
            for h in 0..2 {
                ild(&mut out, T0, &a, h);
                ild(&mut out, T1, &b, h);
                out.push(ci(mn, &[("dst", T0), ("lhs", T0), ("rhs", T1)]));
                isd(&mut out, T0, &d, h);
            }
        }
        // Integer lane compares → mask.
        CmGtV | CmGeV | CmEqV => {
            let (d, a, b) = (f(fields, "dst"), f(fields, "lhs"), f(fields, "rhs"));
            for h in 0..2 {
                ild(&mut out, T0, &a, h);
                ild(&mut out, T1, &b, h);
                match op {
                    CmGtV => {
                        out.push(ci("rv.slt", &[("dst", T0), ("lhs", T1), ("rhs", T0)])); // b<a
                        out.push(ci("sub", &[("dst", T0), ("lhs", ZERO), ("rhs", T0)]));
                    }
                    CmGeV => {
                        out.push(ci("rv.slt", &[("dst", T0), ("lhs", T0), ("rhs", T1)])); // a<b
                        out.push(ci("sub_imm", &[("dst", T0), ("src", T0), ("imm", "1")]));
                        // (a<b)?0:-1
                    }
                    _ => {
                        out.push(ci("eor", &[("dst", T0), ("lhs", T0), ("rhs", T1)])); // a^b
                        out.push(ci("rv.sltu", &[("dst", T0), ("lhs", ZERO), ("rhs", T0)])); // !=0
                        out.push(ci("sub_imm", &[("dst", T0), ("src", T0), ("imm", "1")]));
                        // ==0 ? -1 : 0
                    }
                }
                isd(&mut out, T0, &d, h);
            }
        }
        NegV => {
            let (d, s) = (f(fields, "dst"), f(fields, "src"));
            for h in 0..2 {
                ild(&mut out, T0, &s, h);
                out.push(ci("sub", &[("dst", T0), ("lhs", ZERO), ("rhs", T0)]));
                isd(&mut out, T0, &d, h);
            }
        }
        // Integer lane absolute value: abs(x) = (x ^ (x>>a63)) - (x>>a63), where
        // `x>>a63` (arithmetic) is 0 for non-negative x and all-ones for negative.
        // Emitted for `math::abs(List OF Integer/Fixed)`; without this arm those
        // ICE'd on riscv (bug-121).
        AbsV => {
            let (d, s) = (f(fields, "dst"), f(fields, "src"));
            for h in 0..2 {
                ild(&mut out, T0, &s, h);
                out.push(ci("asr_imm", &[("dst", T1), ("src", T0), ("shift", "63")]));
                out.push(ci("eor", &[("dst", T0), ("lhs", T0), ("rhs", T1)]));
                out.push(ci("sub", &[("dst", T0), ("lhs", T0), ("rhs", T1)]));
                isd(&mut out, T0, &d, h);
            }
        }
        // --- Bitwise `.16b` ----------------------------------------------------
        AndV | OrrV | EorV => {
            let mn = match op {
                AndV => "and",
                OrrV => "orr",
                _ => "eor",
            };
            let (d, a, b) = (f(fields, "dst"), f(fields, "lhs"), f(fields, "rhs"));
            for h in 0..2 {
                ild(&mut out, T0, &a, h);
                ild(&mut out, T1, &b, h);
                out.push(ci(mn, &[("dst", T0), ("lhs", T0), ("rhs", T1)]));
                isd(&mut out, T0, &d, h);
            }
        }
        // bit-select: result = b ^ (mask & (a ^ b)); mask in dst.
        BslV => {
            let (d, a, b) = (f(fields, "dst"), f(fields, "lhs"), f(fields, "rhs"));
            for h in 0..2 {
                ild(&mut out, T0, &a, h);
                ild(&mut out, T1, &b, h);
                out.push(ci("eor", &[("dst", T0), ("lhs", T0), ("rhs", T1)])); // a^b
                ild(&mut out, T1, &d, h); // mask (in dst)
                out.push(ci("and", &[("dst", T0), ("lhs", T0), ("rhs", T1)])); // mask&(a^b)
                ild(&mut out, T1, &b, h); // b
                out.push(ci("eor", &[("dst", T0), ("lhs", T0), ("rhs", T1)])); // b ^ ...
                isd(&mut out, T0, &d, h);
            }
        }
        // bit-insert-if-true: result = dst ^ ((dst ^ lhs) & mask); mask in rhs.
        BitV => {
            let (d, a, m) = (f(fields, "dst"), f(fields, "lhs"), f(fields, "rhs"));
            for h in 0..2 {
                ild(&mut out, T0, &d, h);
                ild(&mut out, T1, &a, h);
                out.push(ci("eor", &[("dst", T0), ("lhs", T0), ("rhs", T1)])); // dst^lhs
                ild(&mut out, T1, &m, h); // mask
                out.push(ci("and", &[("dst", T0), ("lhs", T0), ("rhs", T1)])); // &mask
                ild(&mut out, T1, &d, h); // dst
                out.push(ci("eor", &[("dst", T0), ("lhs", T0), ("rhs", T1)])); // dst ^ ...
                isd(&mut out, T0, &d, h);
            }
        }
        // --- Shifted-immediate `.2d` ------------------------------------------
        ShlV | SshrV | UshrV => {
            let mn = match op {
                ShlV => "lsl_imm",
                SshrV => "asr_imm",
                _ => "lsr_imm",
            };
            let (d, s, sh) = (f(fields, "dst"), f(fields, "src"), f(fields, "shift"));
            // AArch64 allows a `.2d` shift of exactly 64 (`sshr`/`ushr` take
            // 1..=64): arithmetic sign-fills the lane, logical zeroes it. RISC-V
            // masks `shamt` to 6 bits, so forwarding 64 to a scalar shift is both
            // an encoder error and the wrong result — special-case it (bug-16).
            for h in 0..2 {
                if sh == "64" {
                    match op {
                        SshrV => {
                            ild(&mut out, T0, &s, h);
                            // srai by 63 broadcasts the sign bit across the lane.
                            out.push(ci("asr_imm", &[("dst", T0), ("src", T0), ("shift", "63")]));
                            isd(&mut out, T0, &d, h);
                        }
                        // Logical/left shift out every bit: the lane becomes zero.
                        _ => isd(&mut out, ZERO, &d, h),
                    }
                    continue;
                }
                ild(&mut out, T0, &s, h);
                out.push(ci(mn, &[("dst", T0), ("src", T0), ("shift", &sh)]));
                isd(&mut out, T0, &d, h);
            }
        }
        // --- Lane broadcast / extract -----------------------------------------
        DupVFromX => {
            let (d, src) = (f(fields, "dst"), f(fields, "src"));
            out.push(ci(
                "str_u64",
                &[("src", &src), ("base", T2), ("offset", &off(&d, 0))],
            ));
            out.push(ci(
                "str_u64",
                &[("src", &src), ("base", T2), ("offset", &off(&d, 1))],
            ));
        }
        UmovXFromV => {
            let (dst, src, idx) = (f(fields, "dst"), f(fields, "src"), f(fields, "index"));
            // bug-284 C4: `unwrap_or(0)` mapped a malformed index to lane 0, and
            // nothing bounded it -- `index = 2` computes `slot * 16 + 16`, reading
            // the low lane of the *adjacent slot*, i.e. an unrelated value's data.
            // AArch64 rejects this loudly ("umov .d lane index out of range"), so a
            // builder bug caught there miscompiled silently here.
            let half = match idx.parse::<u8>() {
                Ok(half) if half <= 1 => half,
                _ => panic!("rv64 v128: umov .d lane index out of range: {idx:?}"),
            };
            out.push(ci(
                "ldr_u64",
                &[("dst", &dst), ("base", T2), ("offset", &off(&src, half))],
            ));
        }
        other => panic!("rv64 v128: op {} not yet scalarized", other.mnemonic()),
    }
    out
}
