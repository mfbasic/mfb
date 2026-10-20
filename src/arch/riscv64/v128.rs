//! RV64GC `v128` scalarization (plan-99 §6, Phase 3).
//!
//! RV64GC has no 128-bit register file (its `f0`–`f31` are 64-bit), so the
//! neutral `v128` ops — which the transcendental math kernels and `vector::`
//! carry on 128-bit vector values (physical `v0`–`v31` *or* FP virtual registers
//! `%fN`, neither of which fits a 64-bit rv64 register) — are realized as
//! operations on a **memory slot region** in the *per-thread* arena state
//! (`arena_base + ARENA_V128_SLOTS_OFFSET`), where each distinct `v128` value
//! gets a 16-byte slot and lane `h ∈ {0,1}` lives at `base + slot*16 + h*8`. The
//! region was a process-global (`_mfb_rt_v128_slots`) until bug-122: two OS
//! threads running v128 kernels concurrently corrupted each other's lanes.
//!
//! Slots are assigned per function by [`build_slot_map`] over *every* `v128`
//! value the function uses (compactly, so both `vN` and `%fN` fit); this runs in
//! selection **before** register allocation, so the allocator never tries to put
//! a 128-bit value in one 64-bit register. Each op materializes the slot base
//! into `t2` (`auipc; addi`), loads its operands' two `f64`/`i64` lanes into the
//! reserved scratch (`t0`/`t1` integer, `ft0`/`ft1`/`ft2` FP), computes the two
//! scalar results, and stores them back. Correct and slower — the "scalarize"
//! the plan calls for; native-`D` FMA keeps the ≤1-ULP kernel contract.
//!
//! The per-thread slots are **non-reentrant within a thread**: a `v128`
//! computation must not span a call to another `v128`-using function. The
//! transcendental kernels are inlined straight-line leaf code, so this holds;
//! and because each thread addresses its own region off `s11`, concurrent
//! threads no longer race (bug-122).

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

use crate::arch::ops::CodeOp;
use crate::codegen::engine::mir::MirInstruction;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::types::CodeInstruction;

/// Maximum distinct `v128` values per function. Capped so the largest lane
/// offset (`(SLOT_COUNT-1)*16 + 8`) stays within the 12-bit signed load/store
/// immediate (±2047) — otherwise the encoder would materialize the address into
/// `t0`, clobbering a lane. 126 slots ⇒ max offset 2008.
///
/// bug-381: dropped from 128 to 127 in lockstep with `ARENA_V128_SLOTS_SIZE`,
/// which reclaimed the 128th slot's 16 bytes for the per-thread flag-emulation
/// rhs snapshot (`ARENA_FLAG_RHS_OFFSET`). No function has ever needed 128
/// distinct `v128` values, and the `peak_slots <= SLOT_COUNT` assertion in
/// `select_riscv64` would catch it loudly if one did.
///
/// bug-789: 127 → 126 in lockstep with `ARENA_V128_SLOTS_SIZE`, which gave its
/// first 16 bytes to the per-thread stack-limit word.
pub(crate) const SLOT_COUNT: usize = 126;

const T0: &str = "t0";
const T1: &str = "t1";
const T2: &str = "t2"; // slot base pointer
const FT0: &str = "ft0";
const FT1: &str = "ft1";
const FT2: &str = "ft2";
const ZERO: &str = "zero";

fn ci(mnemonic: &str, fields: &[(&'static str, &str)]) -> CodeInstruction {
    let mut inst = CodeInstruction::new(mnemonic);
    for (k, v) in fields {
        inst = inst.physical_field(k, v);
    }
    inst
}

/// Whether `op` is one of the `v128` ops this backend scalarizes.
pub(crate) fn is_v128(op: CodeOp) -> bool {
    use CodeOp::*;
    matches!(
        op,
        LdrQ | StrQ
            | FAddV
            | FSubV
            | FMulV
            | FDivV
            | FMlaV
            | FMlsV
            | FMinV
            | FMaxV
            | FCmGtV
            | FCmGeV
            | FCmEqV
            | FAbsV
            | FNegV
            | FSqrtV
            | FRintpV
            | FRintmV
            | FRintaV
            | FRintnV
            | FRintzV
            | FCvtzsV
            | FCvtasV
            | ScvtfV
            | FCmGtZeroV
            | FCmGeZeroV
            | FCmEqZeroV
            | FCmLtZeroV
            | FCmLeZeroV
            | AddV
            | SubV
            | CmGtV
            | CmGeV
            | CmEqV
            | SshlV
            | UshlV
            | NegV
            | AbsV
            | AndV
            | OrrV
            | EorV
            | BslV
            | BitV
            | ShlV
            | SshrV
            | UshrV
            | DupVFromX
            | UmovXFromV
    )
}

/// Whether an operand names a 128-bit vector value (a physical `v`/`d`/`q`
/// register or an FP virtual register `%fN`) — i.e. a value that needs a slot.
/// GPR operands (`base`, the source of `dup`, the destination of `umov`,
/// immediates) are *not* vector values and pass through unchanged.
fn is_vector_operand(value: &str) -> bool {
    if let Some(rest) = value.strip_prefix(['v', 'd', 'q']) {
        if let Ok(n) = rest.parse::<u8>() {
            return n <= 31;
        }
    }
    // The `abi::VEC_SCRATCH`/`FP_SCRATCH` token pools (plan-34-D) reach this
    // pass unrealized — the shared SIMD kernels spell the low bank through
    // them consistently, so the slot map keys on the token exactly as it once
    // keyed on the literal (same mention order, same slot indices).
    if let Some(rest) = value
        .strip_prefix("%vscratch")
        .or_else(|| value.strip_prefix("%fscratch"))
    {
        return rest.parse::<u8>().is_ok_and(|n| n <= 7);
    }
    value
        .strip_prefix("%f")
        .is_some_and(|rest| rest.parse::<u32>().is_ok())
}

/// Assign every distinct `v128` value in a neutral MIR stream a slot index via
/// **linear-scan reuse**: a slot is freed for reuse once its value's live range
/// (first mention … last mention) ends. Because the memory-slot model only ever
/// touches a value by name, its last mention is its last use, so reuse after that
/// point is safe. Sequential SIMD kernels barely overlap, so a function with
/// hundreds of distinct `v128` values needs only a few dozen concurrent slots —
/// keeping the region (and every lane offset) small. The peak slot count is
/// `1 + max(map.values())`; a naive one-slot-per-value scheme would blow the
/// 128-slot / 2047-byte-offset budget on kernel-heavy functions (e.g. a program
/// exercising the whole `math` package uses ~140 distinct values but ≤128 live).
pub(crate) fn build_slot_map(instructions: &[MirInstruction]) -> HashMap<String, usize> {
    let (order, first, last) = v128_live_ranges(instructions);
    // Linear scan: assign each value (in start order) the lowest free slot,
    // recycling slots whose range ended strictly before this value's start.
    let mut map: HashMap<String, usize> = HashMap::new();
    let mut free: BinaryHeap<Reverse<usize>> = BinaryHeap::new();
    let mut next_slot = 0usize;
    let mut active: Vec<(usize, String)> = Vec::new(); // (last index, value)
    for value in &order {
        let start = first[value];
        active.sort_by_key(|(end, _)| *end);
        let expired = active.iter().take_while(|(end, _)| *end < start).count();
        for (_, dead) in active.drain(0..expired) {
            free.push(Reverse(map[&dead]));
        }
        let slot = free.pop().map(|Reverse(s)| s).unwrap_or_else(|| {
            let s = next_slot;
            next_slot += 1;
            s
        });
        map.insert(value.clone(), slot);
        active.push((last[value], value.clone()));
    }
    // Invariant: two values sharing a slot must have disjoint live ranges.
    #[cfg(debug_assertions)]
    for (a, &sa) in &map {
        for (b, &sb) in &map {
            if a < b && sa == sb {
                let (fa, la, fb, lb) = (first[a], last[a], first[b], last[b]);
                assert!(
                    la < fb || lb < fa,
                    "rv64 v128 slot {sa} shared by overlapping ranges {a}[{fa},{la}] {b}[{fb},{lb}]"
                );
            }
        }
    }
    map
}

/// plan-32-C: assign every distinct `v128` value a **physical vector register**
/// `v1`–`v30` via the same live-range analysis and linear-scan reuse
/// [`build_slot_map`] uses for memory slots — only the assigned resource differs.
/// `v0` is reserved as the RVV mask register and `v31` as a lowering scratch (the
/// mask bridge / slidedown temp), so the allocatable pool is `v1`–`v30`.
///
/// Returns `None` if the function's peak concurrent `v128` pressure exceeds the
/// 30-register pool: that function then emits the scalar arm only (still one
/// correct binary — the RVV arm is simply never selected for it). The scalar arm
/// is untouched by this, so overflow costs performance, never correctness.
pub(crate) fn build_vreg_map(instructions: &[MirInstruction]) -> Option<HashMap<String, u8>> {
    /// Highest allocatable vector register (`v1`..=`V_REG_MAX`); `v0` (mask) and
    /// `v31` (scratch) are reserved.
    const V_REG_MAX: u8 = 30;
    let (order, first, last) = v128_live_ranges(instructions);
    // Linear scan over the bounded pool: recycle a register once its value's range
    // ends, else claim the next fresh one; bail to scalar-only on exhaustion.
    let mut map: HashMap<String, u8> = HashMap::new();
    let mut free: BinaryHeap<Reverse<u8>> = BinaryHeap::new();
    let mut next_reg: u8 = 1;
    let mut active: Vec<(usize, String)> = Vec::new();
    for value in &order {
        let start = first[value];
        active.sort_by_key(|(end, _)| *end);
        let expired = active.iter().take_while(|(end, _)| *end < start).count();
        for (_, dead) in active.drain(0..expired) {
            free.push(Reverse(map[&dead]));
        }
        let reg = match free.pop() {
            Some(Reverse(r)) => r,
            None if next_reg <= V_REG_MAX => {
                let r = next_reg;
                next_reg += 1;
                r
            }
            None => return None, // pressure exceeds v1..=v30 → scalar-arm-only
        };
        map.insert(value.clone(), reg);
        active.push((last[value], value.clone()));
    }
    Some(map)
}

/// The live-range core shared by [`build_slot_map`] and [`build_vreg_map`]: each
/// distinct `v128` value's `[first, last]` mention range (instruction indices),
/// extended so any range overlapping a loop body spans the whole loop (so a
/// loop-carried value never shares storage with another across the back-edge),
/// with the values returned in ascending-start order for the linear scan.
fn v128_live_ranges(
    instructions: &[MirInstruction],
) -> (Vec<String>, HashMap<String, usize>, HashMap<String, usize>) {
    // Live range [first, last] (instruction index) for each vector value, in
    // first-appearance order.
    let mut first: HashMap<String, usize> = HashMap::new();
    let mut last: HashMap<String, usize> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for (idx, instruction) in instructions.iter().enumerate() {
        let Some(op) = instruction.op.to_code() else {
            continue;
        };
        if !is_v128(op) {
            continue;
        }
        for (_, value) in &instruction.fields {
            if is_vector_operand(&value.render()) {
                let value = value.render();
                first.entry(value.clone()).or_insert_with(|| {
                    order.push(value.clone());
                    idx
                });
                last.insert(value, idx);
            }
        }
    }
    // Loop bodies `[target, branch]` from every backward branch. A value whose
    // range touches a loop body may be live across the back-edge (defined late,
    // read early next iteration), which a linear index range cannot express — so
    // extend any overlapping range to span the whole loop. Iterate to a fixpoint
    // for nested/overlapping loops. Without this, a slot freed inside a loop is
    // reused while a loop-carried value still needs it (silent corruption).
    let mut label_idx: HashMap<String, usize> = HashMap::new();
    for (idx, instruction) in instructions.iter().enumerate() {
        if instruction.op.to_code() == Some(CodeOp::Label) {
            if let Some((_, name)) = instruction.fields.iter().find(|(k, _)| *k == "name") {
                label_idx.insert(name.render(), idx);
            }
        }
    }
    let mut loops: Vec<(usize, usize)> = Vec::new();
    for (idx, instruction) in instructions.iter().enumerate() {
        if let Some((_, target)) = instruction.fields.iter().find(|(k, _)| *k == "target") {
            if let Some(&t) = label_idx.get(&target.render()) {
                if t < idx {
                    loops.push((t, idx));
                }
            }
        }
    }
    loop {
        let mut changed = false;
        for value in &order {
            let (f, l) = (first[value], last[value]);
            let (mut nf, mut nl) = (f, l);
            for &(t, b) in &loops {
                if nf <= b && t <= nl {
                    nf = nf.min(t);
                    nl = nl.max(b);
                }
            }
            if nf != f || nl != l {
                first.insert(value.clone(), nf);
                last.insert(value.clone(), nl);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    // Linear scan requires ascending start order; loop extension may have moved
    // starts earlier, so re-sort before allocating.
    order.sort_by_key(|value| first[value]);

    (order, first, last)
}

fn f(fields: &[(&'static str, Operand)], name: &str) -> String {
    fields
        .iter()
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.render())
        .unwrap_or_default()
}

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

/// plan-32-C: load the process-global `_mfb_rt_has_rvv` flag byte into `dst`
/// (`adrp`/`add_pageoff` the symbol, then a byte load).
fn load_has_rvv_flag(dst: &str) -> Vec<CodeInstruction> {
    let sym = crate::codegen::error::constants::HAS_RVV_GLOBAL_SYMBOL;
    vec![
        ci("adrp", &[("dst", dst), ("symbol", sym)]),
        ci(
            "add_pageoff",
            &[("dst", dst), ("src", dst), ("symbol", sym)],
        ),
        ci("ldr_u8", &[("dst", dst), ("base", dst), ("offset", "0")]),
    ]
}

/// plan-32-C: the native-RVV realization of one `v128` op, reading its operands
/// from and writing its result to the **same** per-thread slot region the scalar
/// arm uses (so the two arms reconcile at the slot). Returns `None` for an op this
/// pass does not yet vector-lower (compares, `BslV`/`BitV`, min/max, the
/// rounding/ties-away conversions, wide shifts) — the caller then emits the scalar
/// arm only, which is correctness-preserving. Only ops whose RVV per-lane result
/// is **bit-identical** to the scalar `f64`/`i64` op are lowered here: the RVV
/// arms run at the default RNE rounding, exactly like the scalar `fadd_d`/… they
/// mirror, and the integer/bitwise/mem ops are exact.
fn rvv_arm(
    op: CodeOp,
    fields: &[(&'static str, Operand)],
    slots: &HashMap<String, usize>,
    vregs: &HashMap<String, u8>,
) -> Option<Vec<CodeInstruction>> {
    use CodeOp::*;
    // The caller emits the shared prologue (slot base into `t2`, then `vsetivli`)
    // once per run; this returns just this op's loads + compute + stores.
    let mut out = Vec::new();

    let vname = |value: &str| -> String {
        format!(
            "v{}",
            vregs
                .get(value)
                .copied()
                .unwrap_or_else(|| panic!("rv64 v128 rvv arm: no vreg for '{value}'"))
        )
    };
    let slot_off = |value: &str| -> String { (slots[value] * 16).to_string() };
    // Load `value`'s 16-byte (2×e64) slot into its assigned vector register.
    let load = |out: &mut Vec<CodeInstruction>, value: &str| {
        out.push(ci(
            "add_imm",
            &[("dst", T0), ("src", T2), ("imm", &slot_off(value))],
        ));
        out.push(ci(
            "rv.vop",
            &[("vop", "vle64.v"), ("dst", &vname(value)), ("base", T0)],
        ));
    };
    let store = |out: &mut Vec<CodeInstruction>, value: &str| {
        out.push(ci(
            "add_imm",
            &[("dst", T0), ("src", T2), ("imm", &slot_off(value))],
        ));
        out.push(ci(
            "rv.vop",
            &[("vop", "vse64.v"), ("src", &vname(value)), ("base", T0)],
        ));
    };

    match op {
        FAddV | FSubV | FMulV | FDivV => {
            let mn = match op {
                FAddV => "vfadd.vv",
                FSubV => "vfsub.vv",
                FMulV => "vfmul.vv",
                _ => "vfdiv.vv",
            };
            let (d, a, b) = (f(fields, "dst"), f(fields, "lhs"), f(fields, "rhs"));
            load(&mut out, &a);
            load(&mut out, &b);
            out.push(ci(
                "rv.vop",
                &[
                    ("vop", mn),
                    ("dst", &vname(&d)),
                    ("lhs", &vname(&a)),
                    ("rhs", &vname(&b)),
                ],
            ));
            store(&mut out, &d);
        }
        // Fused multiply-add/subtract (single rounding), matching the scalar
        // fmadd_d/fnmsub_d: vfmacc = vd += lhs*rhs, vfnmsac = vd -= lhs*rhs.
        FMlaV | FMlsV => {
            let mn = if op == FMlaV {
                "vfmacc.vv"
            } else {
                "vfnmsac.vv"
            };
            let (d, a, b) = (f(fields, "dst"), f(fields, "lhs"), f(fields, "rhs"));
            load(&mut out, &d); // the accumulator lane
            load(&mut out, &a);
            load(&mut out, &b);
            out.push(ci(
                "rv.vop",
                &[
                    ("vop", mn),
                    ("dst", &vname(&d)),
                    ("lhs", &vname(&a)),
                    ("rhs", &vname(&b)),
                ],
            ));
            store(&mut out, &d);
        }
        // abs/neg via sign-injection (bit ops, exact); sqrt at RNE (exact).
        FAbsV | FNegV => {
            let mn = if op == FAbsV {
                "vfsgnjx.vv"
            } else {
                "vfsgnjn.vv"
            };
            let (d, s) = (f(fields, "dst"), f(fields, "src"));
            load(&mut out, &s);
            out.push(ci(
                "rv.vop",
                &[
                    ("vop", mn),
                    ("dst", &vname(&d)),
                    ("lhs", &vname(&s)),
                    ("rhs", &vname(&s)),
                ],
            ));
            store(&mut out, &d);
        }
        FSqrtV => {
            let (d, s) = (f(fields, "dst"), f(fields, "src"));
            load(&mut out, &s);
            out.push(ci(
                "rv.vop",
                &[
                    ("vop", "vfsqrt.v"),
                    ("dst", &vname(&d)),
                    ("src", &vname(&s)),
                ],
            ));
            store(&mut out, &d);
        }
        // f64→i64 toward zero and i64→f64: same RISC-V converters the scalar arm
        // uses (fcvtzs = fcvt.l.d RTZ; scvtf = fcvt.d.l), so bit-identical.
        FCvtzsV => {
            let (d, s) = (f(fields, "dst"), f(fields, "src"));
            load(&mut out, &s);
            out.push(ci(
                "rv.vop",
                &[
                    ("vop", "vfcvt.rtz.x.f.v"),
                    ("dst", &vname(&d)),
                    ("src", &vname(&s)),
                ],
            ));
            store(&mut out, &d);
        }
        ScvtfV => {
            let (d, s) = (f(fields, "dst"), f(fields, "src"));
            load(&mut out, &s);
            out.push(ci(
                "rv.vop",
                &[
                    ("vop", "vfcvt.f.x.v"),
                    ("dst", &vname(&d)),
                    ("src", &vname(&s)),
                ],
            ));
            store(&mut out, &d);
        }
        AddV | SubV | AndV | OrrV | EorV => {
            let mn = match op {
                AddV => "vadd.vv",
                SubV => "vsub.vv",
                AndV => "vand.vv",
                OrrV => "vor.vv",
                _ => "vxor.vv",
            };
            let (d, a, b) = (f(fields, "dst"), f(fields, "lhs"), f(fields, "rhs"));
            load(&mut out, &a);
            load(&mut out, &b);
            out.push(ci(
                "rv.vop",
                &[
                    ("vop", mn),
                    ("dst", &vname(&d)),
                    ("lhs", &vname(&a)),
                    ("rhs", &vname(&b)),
                ],
            ));
            store(&mut out, &d);
        }
        NegV => {
            let (d, s) = (f(fields, "dst"), f(fields, "src"));
            load(&mut out, &s);
            // vrsub.vx vd, vs, x0 = 0 - vs.
            out.push(ci(
                "rv.vop",
                &[
                    ("vop", "vrsub.vx"),
                    ("dst", &vname(&d)),
                    ("lhs", &vname(&s)),
                    ("gpr", ZERO),
                ],
            ));
            store(&mut out, &d);
        }
        // Immediate lane shifts, only for amounts the `.vi` uimm5 can encode
        // (0..=31); the scalar arm's shift-of-64 special case and any amount ≥32
        // fall back to scalar.
        ShlV | SshrV | UshrV => {
            let (d, s, sh) = (f(fields, "dst"), f(fields, "src"), f(fields, "shift"));
            match sh.parse::<u8>() {
                Ok(n) if n < 32 => {}
                _ => return None,
            }
            let mn = match op {
                ShlV => "vsll.vi",
                SshrV => "vsra.vi",
                _ => "vsrl.vi",
            };
            load(&mut out, &s);
            out.push(ci(
                "rv.vop",
                &[
                    ("vop", mn),
                    ("dst", &vname(&d)),
                    ("lhs", &vname(&s)),
                    ("imm", &sh),
                ],
            ));
            store(&mut out, &d);
        }
        DupVFromX => {
            let (d, s) = (f(fields, "dst"), f(fields, "src")); // s is a GPR
            out.push(ci(
                "rv.vop",
                &[("vop", "vmv.v.x"), ("dst", &vname(&d)), ("gpr", &s)],
            ));
            store(&mut out, &d);
        }
        UmovXFromV => {
            let (d, s, idx) = (f(fields, "dst"), f(fields, "src"), f(fields, "index")); // d is a GPR
            load(&mut out, &s);
            if idx == "0" {
                out.push(ci(
                    "rv.vop",
                    &[("vop", "vmv.x.s"), ("dst", &d), ("src", &vname(&s))],
                ));
            } else {
                // Reach lane 1 via the reserved v31 scratch, then extract element 0.
                out.push(ci(
                    "rv.vop",
                    &[
                        ("vop", "vslidedown.vi"),
                        ("dst", "v31"),
                        ("lhs", &vname(&s)),
                        ("imm", "1"),
                    ],
                ));
                out.push(ci(
                    "rv.vop",
                    &[("vop", "vmv.x.s"), ("dst", &d), ("src", "v31")],
                ));
            }
        }
        // 128-bit unit-stride load/store from an arbitrary address (base+offset).
        LdrQ => {
            let (d, base, o) = (f(fields, "dst"), f(fields, "base"), f(fields, "offset"));
            out.push(ci("add_imm", &[("dst", T0), ("src", &base), ("imm", &o)]));
            out.push(ci(
                "rv.vop",
                &[("vop", "vle64.v"), ("dst", &vname(&d)), ("base", T0)],
            ));
            store(&mut out, &d);
        }
        StrQ => {
            let (s, base, o) = (f(fields, "src"), f(fields, "base"), f(fields, "offset"));
            load(&mut out, &s);
            out.push(ci("add_imm", &[("dst", T0), ("src", &base), ("imm", &o)]));
            out.push(ci(
                "rv.vop",
                &[("vop", "vse64.v"), ("src", &vname(&s)), ("base", T0)],
            ));
        }
        // min/max: RVV vfmin/vfmax follow the same RISC-V minimumNumber/-0<+0
        // semantics as the scalar fminnm_d/fmaxnm_d, so they are bit-identical.
        FMinV | FMaxV => {
            let mn = if op == FMinV { "vfmin.vv" } else { "vfmax.vv" };
            let (d, a, b) = (f(fields, "dst"), f(fields, "lhs"), f(fields, "rhs"));
            load(&mut out, &a);
            load(&mut out, &b);
            out.push(ci(
                "rv.vop",
                &[
                    ("vop", mn),
                    ("dst", &vname(&d)),
                    ("lhs", &vname(&a)),
                    ("rhs", &vname(&b)),
                ],
            ));
            store(&mut out, &d);
        }
        // --- the mask bridge (plan-32-C Phase 3) ------------------------------
        // A compare writes a 1-bit-per-lane mask into v0, which is then
        // materialized into the NEON all-ones/all-zeros lane vector the scalar arm
        // produces (`vmv.v.i vd,0; vmerge.vim vd,vd,-1,v0`), stored to the slot.
        // `BslV`/`BitV` are then the same bit algebra as the scalar arm, so the
        // results match by construction.
        FCmGtV | FCmGeV | FCmEqV => {
            let (d, a, b) = (f(fields, "dst"), f(fields, "lhs"), f(fields, "rhs"));
            load(&mut out, &a);
            load(&mut out, &b);
            // gt: rhs<lhs ; ge: rhs<=lhs ; eq: lhs==rhs (all ordered → NaN false).
            let (mn, x, y) = match op {
                FCmGtV => ("vmflt.vv", &b, &a),
                FCmGeV => ("vmfle.vv", &b, &a),
                _ => ("vmfeq.vv", &a, &b),
            };
            out.push(ci(
                "rv.vop",
                &[
                    ("vop", mn),
                    ("dst", "v0"),
                    ("lhs", &vname(x)),
                    ("rhs", &vname(y)),
                ],
            ));
            lanes_from_mask(&mut out, &vname(&d));
            store(&mut out, &d);
        }
        FCmGtZeroV | FCmGeZeroV | FCmEqZeroV | FCmLtZeroV | FCmLeZeroV => {
            let (d, s) = (f(fields, "dst"), f(fields, "src"));
            load(&mut out, &s);
            // v31 = +0.0 (integer 0 bits) to compare against.
            out.push(ci(
                "rv.vop",
                &[("vop", "vmv.v.i"), ("dst", "v31"), ("imm", "0")],
            ));
            let z = "v31".to_string();
            let sr = vname(&s);
            let (mn, x, y) = match op {
                FCmGtZeroV => ("vmflt.vv", &z, &sr), // 0 < a
                FCmGeZeroV => ("vmfle.vv", &z, &sr), // 0 <= a
                FCmEqZeroV => ("vmfeq.vv", &sr, &z), // a == 0
                FCmLtZeroV => ("vmflt.vv", &sr, &z), // a < 0
                _ => ("vmfle.vv", &sr, &z),          // a <= 0
            };
            out.push(ci(
                "rv.vop",
                &[("vop", mn), ("dst", "v0"), ("lhs", x), ("rhs", y)],
            ));
            lanes_from_mask(&mut out, &vname(&d));
            store(&mut out, &d);
        }
        CmGtV | CmGeV | CmEqV => {
            let (d, a, b) = (f(fields, "dst"), f(fields, "lhs"), f(fields, "rhs"));
            load(&mut out, &a);
            load(&mut out, &b);
            // gt: b<a ; ge: b<=a (signed) ; eq: a==b.
            let (mn, x, y) = match op {
                CmGtV => ("vmslt.vv", &b, &a),
                CmGeV => ("vmsle.vv", &b, &a),
                _ => ("vmseq.vv", &a, &b),
            };
            out.push(ci(
                "rv.vop",
                &[
                    ("vop", mn),
                    ("dst", "v0"),
                    ("lhs", &vname(x)),
                    ("rhs", &vname(y)),
                ],
            ));
            lanes_from_mask(&mut out, &vname(&d));
            store(&mut out, &d);
        }
        // bit-select: result = rhs ^ (mask & (lhs ^ rhs)); the mask is in dst.
        BslV => {
            let (d, a, b) = (f(fields, "dst"), f(fields, "lhs"), f(fields, "rhs"));
            load(&mut out, &d); // the incoming lane mask
            load(&mut out, &a);
            load(&mut out, &b);
            out.push(ci(
                "rv.vop",
                &[
                    ("vop", "vxor.vv"),
                    ("dst", "v31"),
                    ("lhs", &vname(&a)),
                    ("rhs", &vname(&b)),
                ],
            )); // a^b
            out.push(ci(
                "rv.vop",
                &[
                    ("vop", "vand.vv"),
                    ("dst", "v31"),
                    ("lhs", "v31"),
                    ("rhs", &vname(&d)),
                ],
            )); // & mask
            out.push(ci(
                "rv.vop",
                &[
                    ("vop", "vxor.vv"),
                    ("dst", &vname(&d)),
                    ("lhs", &vname(&b)),
                    ("rhs", "v31"),
                ],
            )); // rhs ^ ...
            store(&mut out, &d);
        }
        // bit-insert-if-true: result = dst ^ ((dst ^ lhs) & mask); mask in rhs.
        BitV => {
            let (d, a, m) = (f(fields, "dst"), f(fields, "lhs"), f(fields, "rhs"));
            load(&mut out, &d);
            load(&mut out, &a);
            load(&mut out, &m);
            out.push(ci(
                "rv.vop",
                &[
                    ("vop", "vxor.vv"),
                    ("dst", "v31"),
                    ("lhs", &vname(&d)),
                    ("rhs", &vname(&a)),
                ],
            )); // dst^lhs
            out.push(ci(
                "rv.vop",
                &[
                    ("vop", "vand.vv"),
                    ("dst", "v31"),
                    ("lhs", "v31"),
                    ("rhs", &vname(&m)),
                ],
            )); // & mask
            out.push(ci(
                "rv.vop",
                &[
                    ("vop", "vxor.vv"),
                    ("dst", &vname(&d)),
                    ("lhs", &vname(&d)),
                    ("rhs", "v31"),
                ],
            )); // dst ^ ...
            store(&mut out, &d);
        }
        // Everything else (FRint*, FCvtasV, wide integer shifts, AbsV) is left
        // to the scalar arm. `SshlV`/`UshlV` are recognized `is_v128` ops with
        // no `scalarize_v128` arm, so they fail loud there rather than being
        // scalarized (pinned by `unhandled_v128_op_panics`).
        _ => return None,
    }
    Some(out)
}

/// plan-32-C Phase 3: turn the 1-bit-per-lane mask in `v0` into the NEON
/// all-ones/all-zeros lane vector in `vd` (`vmv.v.i vd,0; vmerge.vim vd,vd,-1,v0`),
/// so downstream `BslV`/`BitV`/`AndV` are the same bit algebra as the scalar arm.
/// `imm` 31 is the 5-bit `-1` the merge sign-extends to an all-ones lane.
fn lanes_from_mask(out: &mut Vec<CodeInstruction>, vd: &str) {
    out.push(ci(
        "rv.vop",
        &[("vop", "vmv.v.i"), ("dst", vd), ("imm", "0")],
    ));
    out.push(ci(
        "rv.vop",
        &[
            ("vop", "vmerge.vim"),
            ("dst", vd),
            ("src", vd),
            ("imm", "31"),
        ],
    ));
}

/// The shared RVV-arm prologue: the per-thread slot base into `t2`, then
/// `vsetivli x0, 2, e64, m1, ta, ma`. Emitted once per run (plan-32-D).
fn rvv_prologue(out: &mut Vec<CodeInstruction>) {
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
    out.push(ci(
        "rv.vop",
        &[("vop", "vsetivli"), ("dst", "zero"), ("avl", "2")],
    ));
}

/// Whether `op` (with these `fields`) is realized on the native-RVV arm. A quick
/// predicate mirroring [`rvv_arm`]'s coverage, so the run builder can decide
/// membership without emitting; ops it rejects always scalarize.
pub(crate) fn rvv_lowerable(op: CodeOp, fields: &[(&'static str, Operand)]) -> bool {
    use CodeOp::*;
    match op {
        FAddV | FSubV | FMulV | FDivV | FMlaV | FMlsV | FAbsV | FNegV | FSqrtV | FCvtzsV
        | ScvtfV | AddV | SubV | AndV | OrrV | EorV | NegV | DupVFromX | UmovXFromV | LdrQ
        | StrQ | FMinV | FMaxV | FCmGtV | FCmGeV | FCmEqV | FCmGtZeroV | FCmGeZeroV
        | FCmEqZeroV | FCmLtZeroV | FCmLeZeroV | CmGtV | CmGeV | CmEqV | BslV | BitV => true,
        // Immediate lane shifts only when the `.vi` uimm5 can encode the amount.
        ShlV | SshrV | UshrV => matches!(f(fields, "shift").parse::<u8>(), Ok(n) if n < 32),
        _ => false,
    }
}

/// plan-32-D: within one RVV-arm body, drop a reload of a slot into a register
/// that already holds that slot's value. Each `v128` value owns its slot and its
/// register exclusively across a run (disjoint live ranges never share either),
/// so once a register has been loaded from — or stored to — its slot, a later
/// `add_imm t0,t2,off; vle64 vX,(t0)` for the *same* register+offset is redundant
/// until `vX` is overwritten by a compute. This is the register-residency that
/// turns a per-op memory round-trip into a resident chain; stores are kept (a
/// conservative choice that cannot miss a live-out spill).
fn drop_redundant_reloads(body: Vec<CodeInstruction>) -> Vec<CodeInstruction> {
    // reg → the slot offset it currently, provably, holds.
    let mut holds: HashMap<String, String> = HashMap::new();
    let mut keep = vec![true; body.len()];
    let mut i = 0;
    while i < body.len() {
        let inst = &body[i];
        // A slot access is `add_imm t0,t2,off` immediately followed by a
        // vle64/vse64 through t0.
        let is_slot_addr = inst.op.mnemonic() == "add_imm"
            && inst.get("dst").as_deref() == Some(T0)
            && inst.get("src").as_deref() == Some(T2);
        if is_slot_addr && i + 1 < body.len() {
            let off = inst.get("imm").unwrap_or_default();
            let next = &body[i + 1];
            if next.op.mnemonic() == "rv.vop" && next.get("base").as_deref() == Some(T0) {
                match next.get("vop").as_deref() {
                    Some("vle64.v") => {
                        let reg = next.get("dst").unwrap_or_default();
                        if holds.get(&reg) == Some(&off) {
                            // Register already holds this slot → drop the reload.
                            keep[i] = false;
                            keep[i + 1] = false;
                        } else {
                            holds.insert(reg, off);
                        }
                        i += 2;
                        continue;
                    }
                    Some("vse64.v") => {
                        let reg = next.get("src").unwrap_or_default();
                        holds.insert(reg, off);
                        i += 2;
                        continue;
                    }
                    _ => {}
                }
            }
        }
        // Any rv.vop that writes a vector register invalidates that register's
        // known slot value (it now holds a computed result). `vmv.x.s` writes a
        // GPR, not a vector reg, so it does not invalidate.
        if inst.op.mnemonic() == "rv.vop" && inst.get("vop").as_deref() != Some("vmv.x.s") {
            if let Some(dst) = inst.get("dst") {
                if dst.starts_with('v') {
                    holds.remove(&dst);
                }
            }
        }
        i += 1;
    }
    body.into_iter()
        .zip(keep)
        .filter_map(|(inst, k)| k.then_some(inst))
        .collect()
}

/// plan-32-C/-D: lower a maximal run of consecutive native-RVV-lowerable `v128`
/// ops to one runtime dual path — a single guard on `_mfb_rt_has_rvv`, then either
/// the RVV arm (one `vsetivli`, register-resident chain) or the scalar arm (the
/// unchanged per-op `scalarize_v128`), reconciled at the shared slot region.
/// `seq` makes the two arm labels unique within the function.
pub(crate) fn lower_v128_run(
    run: &[(CodeOp, Vec<(&'static str, Operand)>)],
    slots: &HashMap<String, usize>,
    vregs: &HashMap<String, u8>,
    seq: usize,
) -> Vec<CodeInstruction> {
    // Build the RVV arm: shared prologue + each op's body, then residency-optimize.
    let mut rvv_body = Vec::new();
    rvv_prologue(&mut rvv_body);
    for (op, fields) in run {
        rvv_body.extend(rvv_arm(*op, fields, slots, vregs).expect("run holds only lowerable ops"));
    }
    let rvv_body = drop_redundant_reloads(rvv_body);

    let scalar_label = format!("v128_scalar_{seq}");
    let done_label = format!("v128_done_{seq}");
    let mut out = load_has_rvv_flag(T0);
    // beqz t0, .scalar — the flag is clear on non-V hardware.
    out.push(ci(
        "rv.br",
        &[
            ("lhs", T0),
            ("rhs", ZERO),
            ("cond", "eq"),
            ("target", &scalar_label),
        ],
    ));
    out.extend(rvv_body);
    out.push(ci("b", &[("target", &done_label)]));
    out.push(ci("label", &[("name", &scalar_label)]));
    for (op, fields) in run {
        out.extend(scalarize_v128(*op, fields, slots));
    }
    out.push(ci("label", &[("name", &done_label)]));
    out
}

/// plan-32-C: lower a single `v128` op to the runtime dual path. Scalar-only (no
/// guard) when the function overflowed the vector-register pool (`vregs` is
/// `None`) or the op is not vector-lowered; otherwise a one-op run (per-op prologue
/// preserved — byte-identical to the pre-per-run lowering).
#[cfg(test)]
mod tests;
