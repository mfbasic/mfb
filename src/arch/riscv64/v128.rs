//! RV64GC `v128` scalarization (plan-99 §6, Phase 3).
//!
//! RV64GC has no 128-bit register file (its `f0`–`f31` are 64-bit), so the
//! neutral `v128` ops — which the transcendental math kernels and `vector::`
//! carry on 128-bit vector values (physical `v0`–`v31` *or* FP virtual registers
//! `%fN`, neither of which fits a 64-bit rv64 register) — are realized as
//! operations on a **memory slot region**: `_mfb_rt_v128_slots`, where each
//! distinct `v128` value gets a 16-byte slot and lane `h ∈ {0,1}` lives at
//! `slots + slot*16 + h*8`.
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
//! The global slots make this **single-threaded / non-reentrant**: a `v128`
//! computation must not span a call to another `v128`-using function. The
//! transcendental kernels are inlined straight-line leaf code, so this holds.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

use crate::arch::aarch64::ops::CodeOp;
use crate::target::shared::code::mir::MirInstruction;
use crate::target::shared::code::CodeInstruction;

/// The global slot region symbol. Sized [`SLOT_COUNT`] × 16 bytes; emitted as a
/// data object by the module lowering when any function references it.
pub(crate) const V128_SLOTS_SYMBOL: &str = "_mfb_rt_v128_slots";

/// Maximum distinct `v128` values per function. Capped so the largest lane
/// offset (`(SLOT_COUNT-1)*16 + 8`) stays within the 12-bit signed load/store
/// immediate (±2047) — otherwise the encoder would materialize the address into
/// `t0`, clobbering a lane. 128 slots ⇒ max offset 2040.
pub(crate) const SLOT_COUNT: usize = 128;

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
        inst = inst.field(k, v);
    }
    inst
}

/// Whether `op` is one of the `v128` ops this backend scalarizes.
pub(crate) fn is_v128(op: CodeOp) -> bool {
    use CodeOp::*;
    matches!(
        op,
        LdrQ | StrQ
            | FAddV | FSubV | FMulV | FDivV | FMlaV | FMlsV | FMinV | FMaxV
            | FCmGtV | FCmGeV | FCmEqV | FAbsV | FNegV | FSqrtV
            | FRintpV | FRintmV | FRintaV | FRintnV | FRintzV
            | FCvtzsV | FCvtasV | ScvtfV
            | FCmGtZeroV | FCmGeZeroV | FCmEqZeroV | FCmLtZeroV | FCmLeZeroV
            | AddV | SubV | CmGtV | CmGeV | CmEqV | SshlV | UshlV | NegV | AbsV
            | AndV | OrrV | EorV | BslV | BitV | ShlV | SshrV | UshrV
            | DupVFromX | UmovXFromV
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
            if is_vector_operand(value) {
                first.entry(value.clone()).or_insert_with(|| {
                    order.push(value.clone());
                    idx
                });
                last.insert(value.clone(), idx);
            }
        }
    }
    // Loop bodies `[target, branch]` from every backward branch. A value whose
    // range touches a loop body may be live across the back-edge (defined late,
    // read early next iteration), which a linear index range cannot express — so
    // extend any overlapping range to span the whole loop. Iterate to a fixpoint
    // for nested/overlapping loops. Without this, a slot freed inside a loop is
    // reused while a loop-carried value still needs it (silent corruption).
    let mut label_idx: HashMap<&str, usize> = HashMap::new();
    for (idx, instruction) in instructions.iter().enumerate() {
        if instruction.op.to_code() == Some(CodeOp::Label) {
            if let Some((_, name)) = instruction.fields.iter().find(|(k, _)| *k == "name") {
                label_idx.insert(name.as_str(), idx);
            }
        }
    }
    let mut loops: Vec<(usize, usize)> = Vec::new();
    for (idx, instruction) in instructions.iter().enumerate() {
        if let Some((_, target)) = instruction.fields.iter().find(|(k, _)| *k == "target") {
            if let Some(&t) = label_idx.get(target.as_str()) {
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

fn f(fields: &[(&'static str, String)], name: &str) -> String {
    fields
        .iter()
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.clone())
        .unwrap_or_default()
}

/// Scalarize one `v128` CodeOp into RV64GC scalar/memory ops (plan-99 §6), using
/// `slots` to place each vector value.
pub(crate) fn scalarize_v128(
    op: CodeOp,
    fields: &[(&'static str, String)],
    slots: &HashMap<String, usize>,
) -> Vec<CodeInstruction> {
    let mut out = Vec::new();
    // Materialize the slot base into t2 (`auipc t2, %hi; addi t2, t2, %lo`).
    out.push(ci("adrp", &[("dst", T2), ("symbol", V128_SLOTS_SYMBOL)]));
    out.push(ci(
        "add_pageoff",
        &[("dst", T2), ("src", T2), ("symbol", V128_SLOTS_SYMBOL)],
    ));

    let off = |name: &str, half: u8| -> String {
        let idx = *slots
            .get(name)
            .unwrap_or_else(|| panic!("rv64 v128: no slot for '{name}'"));
        (idx * 16 + half as usize * 8).to_string()
    };
    let fld = |o: &mut Vec<CodeInstruction>, dst: &str, name: &str, half: u8| {
        o.push(ci("ldr_d", &[("dst", dst), ("base", T2), ("offset", &off(name, half))]));
    };
    let fsd = |o: &mut Vec<CodeInstruction>, src: &str, name: &str, half: u8| {
        o.push(ci("str_d", &[("src", src), ("base", T2), ("offset", &off(name, half))]));
    };
    let ild = |o: &mut Vec<CodeInstruction>, dst: &str, name: &str, half: u8| {
        o.push(ci("ldr_u64", &[("dst", dst), ("base", T2), ("offset", &off(name, half))]));
    };
    let isd = |o: &mut Vec<CodeInstruction>, src: &str, name: &str, half: u8| {
        o.push(ci("str_u64", &[("src", src), ("base", T2), ("offset", &off(name, half))]));
    };

    use CodeOp::*;
    match op {
        // --- 128-bit memory load/store (16 bytes, no lane interpretation) ------
        LdrQ => {
            let (dst, base, o) = (f(fields, "dst"), f(fields, "base"), f(fields, "offset"));
            let o8 = (o.parse::<u64>().unwrap_or(0) + 8).to_string();
            // Value in T1: a large `base` offset makes the encoder use T0 as the
            // address scratch, which would clobber the value if it were in T0.
            out.push(ci("ldr_u64", &[("dst", T1), ("base", &base), ("offset", &o)]));
            isd(&mut out, T1, &dst, 0);
            out.push(ci("ldr_u64", &[("dst", T1), ("base", &base), ("offset", &o8)]));
            isd(&mut out, T1, &dst, 1);
        }
        StrQ => {
            let (src, base, o) = (f(fields, "src"), f(fields, "base"), f(fields, "offset"));
            let o8 = (o.parse::<u64>().unwrap_or(0) + 8).to_string();
            ild(&mut out, T1, &src, 0);
            out.push(ci("str_u64", &[("src", T1), ("base", &base), ("offset", &o)]));
            ild(&mut out, T1, &src, 1);
            out.push(ci("str_u64", &[("src", T1), ("base", &base), ("offset", &o8)]));
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
        // Fused multiply-subtract: dst -= lhs*rhs.
        FMlsV => {
            let (d, a, b) = (f(fields, "dst"), f(fields, "lhs"), f(fields, "rhs"));
            for h in 0..2 {
                fld(&mut out, FT1, &a, h);
                fld(&mut out, FT2, &b, h);
                out.push(ci("fmul_d", &[("dst", FT1), ("lhs", FT1), ("rhs", FT2)]));
                fld(&mut out, FT0, &d, h);
                out.push(ci("fsub_d", &[("dst", FT0), ("lhs", FT0), ("rhs", FT1)]));
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
        // Round to integral f64 by mode: convert f64→i64 (with the mode) then back.
        FRintmV | FRintpV | FRintzV | FRintaV | FRintnV => {
            let cvt = match op {
                FRintmV => "fcvtms_x_from_d", // toward -inf
                FRintpV => "fcvtps_x_from_d", // toward +inf
                FRintzV => "fcvtzs_x_from_d", // toward zero
                _ => "fcvtas_x_from_d",       // nearest ties away (frinta / ~frintn)
            };
            let (d, s) = (f(fields, "dst"), f(fields, "src"));
            for h in 0..2 {
                fld(&mut out, FT0, &s, h);
                out.push(ci(cvt, &[("dst", T0), ("src", FT0)]));
                out.push(ci("scvtf_d_from_x", &[("dst", FT0), ("src", T0)]));
                fsd(&mut out, FT0, &d, h);
            }
        }
        // Lane f64→i64 conversions (result is an i64 in the slot).
        FCvtzsV | FCvtasV => {
            let cvt = if op == FCvtzsV { "fcvtzs_x_from_d" } else { "fcvtas_x_from_d" };
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
                out.push(ci("rv.fcmp", &[("dst", T0), ("lhs", l), ("rhs", r), ("cmp", cmp)]));
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
                out.push(ci("rv.fcmp", &[("dst", T0), ("lhs", l), ("rhs", r), ("cmp", cmp)]));
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
                        out.push(ci("sub_imm", &[("dst", T0), ("src", T0), ("imm", "1")])); // (a<b)?0:-1
                    }
                    _ => {
                        out.push(ci("eor", &[("dst", T0), ("lhs", T0), ("rhs", T1)])); // a^b
                        out.push(ci("rv.sltu", &[("dst", T0), ("lhs", ZERO), ("rhs", T0)])); // !=0
                        out.push(ci("sub_imm", &[("dst", T0), ("src", T0), ("imm", "1")])); // ==0 ? -1 : 0
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
            for h in 0..2 {
                ild(&mut out, T0, &s, h);
                out.push(ci(mn, &[("dst", T0), ("src", T0), ("shift", &sh)]));
                isd(&mut out, T0, &d, h);
            }
        }
        // --- Lane broadcast / extract -----------------------------------------
        DupVFromX => {
            let (d, src) = (f(fields, "dst"), f(fields, "src"));
            out.push(ci("str_u64", &[("src", &src), ("base", T2), ("offset", &off(&d, 0))]));
            out.push(ci("str_u64", &[("src", &src), ("base", T2), ("offset", &off(&d, 1))]));
        }
        UmovXFromV => {
            let (dst, src, idx) = (f(fields, "dst"), f(fields, "src"), f(fields, "index"));
            let half = idx.parse::<u8>().unwrap_or(0);
            out.push(ci("ldr_u64", &[("dst", &dst), ("base", T2), ("offset", &off(&src, half))]));
        }
        other => panic!("rv64 v128: op {} not yet scalarized", other.mnemonic()),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, usize)]) -> HashMap<String, usize> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[test]
    fn detects_vector_operands() {
        assert!(is_vector_operand("v0"));
        assert!(is_vector_operand("d16"));
        assert!(is_vector_operand("%f7"));
        assert!(!is_vector_operand("a0"));
        assert!(!is_vector_operand("%v3"));
        assert!(!is_vector_operand("128"));
    }

    #[test]
    fn fadd_v_scalarizes_to_two_lane_adds() {
        let fields = vec![
            ("dst", "v0".to_string()),
            ("lhs", "%f7".to_string()),
            ("rhs", "v2".to_string()),
        ];
        let slots = map(&[("v0", 0), ("%f7", 1), ("v2", 2)]);
        let out = scalarize_v128(CodeOp::FAddV, &fields, &slots);
        // auipc + addi base, then 2 lanes × (ldr,ldr,fadd,str) = 2 + 8.
        assert_eq!(out.len(), 10);
        assert_eq!(out[0].op, CodeOp::Adrp);
        assert_eq!(out.iter().filter(|i| i.op.mnemonic() == "fadd_d").count(), 2);
        // %f7 (slot 1) low lane reads offset 16.
        assert!(out.iter().any(|i| i.get("offset") == Some("16")));
    }

    fn mir(op: crate::target::shared::code::mir::MirOp, fields: &[(&'static str, &str)]) -> MirInstruction {
        MirInstruction {
            op,
            fields: fields.iter().map(|(k, v)| (*k, v.to_string())).collect(),
        }
    }

    fn peak(map: &HashMap<String, usize>) -> usize {
        map.values().map(|s| s + 1).max().unwrap_or(0)
    }

    #[test]
    fn slots_are_reused_across_disjoint_live_ranges() {
        use crate::target::shared::code::mir::MirOp;
        // Two independent lane-adds in straight-line code: the second op's values
        // recycle the first op's slots (their ranges do not overlap), so six
        // distinct values need only three concurrent slots.
        let inst = vec![
            mir(MirOp::FAddV, &[("dst", "%f0"), ("lhs", "%f1"), ("rhs", "%f2")]),
            mir(MirOp::FAddV, &[("dst", "%f3"), ("lhs", "%f4"), ("rhs", "%f5")]),
        ];
        let slots = build_slot_map(&inst);
        assert_eq!(slots.len(), 6, "all six values are mapped");
        assert_eq!(peak(&slots), 3, "but only three slots are live at once");
    }

    #[test]
    fn loop_carried_values_never_share_a_slot() {
        use crate::target::shared::code::mir::MirOp;
        // The same two ops inside a loop (a backward branch to `top`): a value
        // defined late could be read early on the next iteration, so live ranges
        // are extended across the whole loop and no slot is recycled within it —
        // otherwise a loop-carried value would be silently clobbered.
        let inst = vec![
            mir(MirOp::Label, &[("name", "top")]),
            mir(MirOp::FAddV, &[("dst", "%f0"), ("lhs", "%f1"), ("rhs", "%f2")]),
            mir(MirOp::FAddV, &[("dst", "%f3"), ("lhs", "%f4"), ("rhs", "%f5")]),
            mir(MirOp::BranchEq, &[("lhs", "a0"), ("rhs", "a1"), ("target", "top")]),
        ];
        let slots = build_slot_map(&inst);
        assert_eq!(slots.len(), 6);
        assert_eq!(peak(&slots), 6, "loop extension keeps all six values distinct");
    }

    #[test]
    fn dup_broadcasts_both_lanes() {
        let fields = vec![("dst", "v3".to_string()), ("src", "a0".to_string())];
        let slots = map(&[("v3", 0)]);
        let out = scalarize_v128(CodeOp::DupVFromX, &fields, &slots);
        assert_eq!(out.iter().filter(|i| i.op.mnemonic() == "str_u64").count(), 2);
        // src (a0, a GPR) passes through unslotted.
        assert!(out.iter().any(|i| i.get("src") == Some("a0")));
    }
}
