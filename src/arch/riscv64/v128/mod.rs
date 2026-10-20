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

mod rvv;
mod scalarize;
pub(crate) use rvv::{lower_v128_run, rvv_lowerable};
pub(crate) use scalarize::scalarize_v128;
// The tests (`v128/tests/`) drive `rvv_arm` directly through their `use super::*;`.
#[cfg(test)]
use rvv::rvv_arm;

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

/// plan-32-C: lower a single `v128` op to the runtime dual path. Scalar-only (no
/// guard) when the function overflowed the vector-register pool (`vregs` is
/// `None`) or the op is not vector-lowered; otherwise a one-op run (per-op prologue
/// preserved — byte-identical to the pre-per-run lowering).
#[cfg(test)]
mod tests;
