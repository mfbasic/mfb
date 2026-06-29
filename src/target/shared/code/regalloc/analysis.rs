//! Instruction-effect model, CFG, and liveness for the liveness-driven
//! allocation strategies (plan-03 Stage B). ISA-neutral: it reads register
//! operands by field role and classifies each value, but names no specific
//! physical register.
//!
//! Performance matters: generated functions (e.g. the regex engine) have
//! thousands of instructions and hundreds of virtual registers, so liveness uses
//! interned register ids and bitsets, and the result is compact — per-virtual-
//! register live *intervals* plus per-instruction physical-occupancy bitsets —
//! so the linear-scan coloring stays near-linear.

use std::collections::HashMap;

use crate::arch::aarch64::ops::CodeOp;

use super::super::types::CodeInstruction;

/// Fields that name a register the instruction *writes*. AArch64 is
/// three-address with no tied operands, so a `dst` field is always a pure
/// definition.
const DEF_FIELDS: &[&str] = &["dst"];

/// Fields that name a register the instruction *reads*.
const USE_FIELDS: &[&str] = &["src", "lhs", "rhs", "minuend", "base", "register", "addend"];

/// Per-register-class hooks the allocator core queries: which operand strings are
/// this class's virtual registers, and which are its physical registers. The Int
/// class matches `%vN` / `x0`–`x30`; the Fp class matches `%fN` / `d0`–`d31`. A
/// register of the *other* class is invisible to a pass (the two physical files
/// never interfere), so cross-class moves (`fmov x, d`) are handled correctly by
/// each pass seeing only its own operands.
#[derive(Clone, Copy)]
pub(super) struct ClassModel {
    pub(super) parse_vreg: fn(&str) -> Option<u32>,
    pub(super) physical_index: fn(&str) -> Option<u32>,
    /// Whether this is the FP class (selects the FP vs integer clobber sets).
    pub(super) is_fp: bool,
}

/// Caller-saved integer registers `x0`–`x17` (clobbered by any call per the PCS).
const CALLER_SAVED_INT: PhysMask = 0x3_ffff;
/// Caller-saved FP registers `d0`–`d7` and `d16`–`d31` (`d8`–`d15` are
/// callee-saved by the PCS; the inlined NEON kernels also avoid `v8`–`v15`).
const CALLER_SAVED_FP: PhysMask = 0xffff_00ff;
/// Every integer register `x0`–`x30` — forbidding all of them forces a spill.
const ALL_INT: PhysMask = 0x7fff_ffff;

/// The set of physical registers (of `is_fp`'s class) a call instruction
/// destroys, so a value live across it must avoid them (plan-03 §4.3). Every case
/// rests on the PCS contract that callee-saved registers (`x19`–`x28`, `d8`–`d15`)
/// survive any call; only the caller-saved set, plus any extra a given callee is
/// known to trample, is clobbered. Modeled per target:
/// - `_mfb_fn_*` / `_mfb_ifn_*` (user/built-in functions, compiled here with a PCS
///   frame that saves the callee-saved registers it uses) and libc clobber only
///   caller-saved registers.
/// - other `_mfb_*` runtime helpers clobber every integer register: the
///   hand-written `_mfb_arena_alloc` uses callee-saved `x20`–`x28` as scratch
///   (saving only `x30`), and other helpers' integer clobber sets are unknown.
///   Their FP clobber still follows the PCS (caller-saved only) — `_mfb_arena_alloc`
///   touches no FP on its fast path and reaches `mmap` (PCS) when it grows.
/// - `blr` is an indirect call to a PCS function; `svc` is a syscall (no FP).
pub(super) fn call_clobber_mask(instruction: &CodeInstruction, is_fp: bool) -> PhysMask {
    match instruction.op {
        CodeOp::Svc => {
            // A syscall preserves callee-saved integer registers and touches no FP.
            if is_fp {
                0
            } else {
                CALLER_SAVED_INT
            }
        }
        CodeOp::BranchLinkRegister => {
            if is_fp {
                CALLER_SAVED_FP
            } else {
                CALLER_SAVED_INT
            }
        }
        CodeOp::BranchLink => {
            let target = instruction.get("target").unwrap_or("");
            if target.starts_with("_mfb_fn_") || target.starts_with("_mfb_ifn_") {
                // A compiled user/built-in function: PCS, preserves callee-saved.
                if is_fp {
                    CALLER_SAVED_FP
                } else {
                    CALLER_SAVED_INT
                }
            } else if target.starts_with("_mfb_") {
                // A runtime helper: every integer register is treated as
                // destroyed (`_mfb_arena_alloc` tramples callee-saved `x20`–`x28`),
                // while FP follows the PCS — caller-saved gone, `d8`–`d15` kept.
                if is_fp {
                    CALLER_SAVED_FP
                } else {
                    ALL_INT
                }
            } else {
                // libc (PCS).
                if is_fp {
                    CALLER_SAVED_FP
                } else {
                    CALLER_SAVED_INT
                }
            }
        }
        _ => 0,
    }
}

/// The integer physical-register index `0..=30` (`x0`–`x30`), or `None`.
/// Excludes `x31`/`xzr`, `sp`, and FP registers.
pub(super) fn int_physical_index(name: &str) -> Option<u32> {
    let rest = name.strip_prefix('x')?;
    let n: u32 = rest.parse().ok()?;
    (n <= 30).then_some(n)
}

/// The FP/SIMD physical-register index `0..=31`, or `None`. Matches BOTH the
/// scalar `d0`–`d31` spelling and the vector `v0`–`v31` spelling, because they
/// alias the same physical file (`d_n` ⊂ `v_n`): a NEON kernel's hardcoded `v5`
/// must mark `d5` busy so the allocator never colors an FP value onto it (§4.6).
pub(super) fn fp_physical_index(name: &str) -> Option<u32> {
    let rest = name.strip_prefix('d').or_else(|| name.strip_prefix('v'))?;
    let n: u32 = rest.parse().ok()?;
    (n <= 31).then_some(n)
}

impl ClassModel {
    /// Whether `name` is a register this class tracks (a virtual or physical one).
    pub(super) fn is_tracked(&self, name: &str) -> bool {
        (self.parse_vreg)(name).is_some() || (self.physical_index)(name).is_some()
    }
}

/// The registers (of one class) an instruction defines and uses, plus whether it
/// is a call/syscall (clobbers caller-saved registers).
pub(super) struct Effect {
    pub(super) defs: Vec<String>,
    pub(super) uses: Vec<String>,
    pub(super) is_call: bool,
}

pub(super) fn effect(instruction: &CodeInstruction, model: &ClassModel) -> Effect {
    let mut defs = Vec::new();
    let mut uses = Vec::new();
    for (name, value) in &instruction.fields {
        if DEF_FIELDS.contains(name) {
            if model.is_tracked(value) {
                defs.push(value.clone());
            }
        } else if USE_FIELDS.contains(name) && model.is_tracked(value) {
            uses.push(value.clone());
        }
    }
    let is_call = matches!(
        instruction.op,
        CodeOp::BranchLink | CodeOp::BranchLinkRegister | CodeOp::Svc
    );
    Effect { defs, uses, is_call }
}

/// A basic block: a half-open instruction range `[start, end)` and its
/// successor block indices.
struct Block {
    start: usize,
    end: usize,
    succ: Vec<usize>,
}

fn is_block_terminator(op: CodeOp) -> bool {
    matches!(
        op,
        CodeOp::Branch
            | CodeOp::BranchEq
            | CodeOp::BranchNe
            | CodeOp::BranchGe
            | CodeOp::BranchLt
            | CodeOp::BranchGt
            | CodeOp::BranchLe
            | CodeOp::BranchVc
            | CodeOp::BranchVs
            | CodeOp::BranchHi
            | CodeOp::BranchLo
            | CodeOp::Ret
            | CodeOp::BranchSelf
    )
}

fn is_unconditional_terminator(op: CodeOp) -> bool {
    matches!(op, CodeOp::Branch | CodeOp::Ret | CodeOp::BranchSelf)
}

fn build_cfg(instructions: &[CodeInstruction]) -> Vec<Block> {
    let n = instructions.len();
    if n == 0 {
        return Vec::new();
    }
    let mut is_leader = vec![false; n];
    is_leader[0] = true;
    for (i, instruction) in instructions.iter().enumerate() {
        if instruction.op == CodeOp::Label {
            is_leader[i] = true;
        }
        if is_block_terminator(instruction.op) && i + 1 < n {
            is_leader[i + 1] = true;
        }
    }
    let starts: Vec<usize> = (0..n).filter(|&i| is_leader[i]).collect();
    let mut block_of = vec![0usize; n];
    for (block_index, window) in starts.windows(2).enumerate() {
        for i in window[0]..window[1] {
            block_of[i] = block_index;
        }
    }
    if let Some(&last_start) = starts.last() {
        for i in last_start..n {
            block_of[i] = starts.len() - 1;
        }
    }
    let mut label_block = HashMap::new();
    for (i, instruction) in instructions.iter().enumerate() {
        if instruction.op == CodeOp::Label {
            if let Some(name) = instruction.get("name") {
                label_block.insert(name.to_string(), block_of[i]);
            }
        }
    }
    let mut blocks: Vec<Block> = Vec::with_capacity(starts.len());
    for (block_index, &start) in starts.iter().enumerate() {
        let end = starts.get(block_index + 1).copied().unwrap_or(n);
        let last = &instructions[end - 1];
        let mut succ = Vec::new();
        if is_block_terminator(last.op) {
            if let Some(target) = last.get("target") {
                if let Some(&tb) = label_block.get(target) {
                    succ.push(tb);
                }
            }
            if !is_unconditional_terminator(last.op) && block_index + 1 < starts.len() {
                succ.push(block_index + 1);
            }
        } else if block_index + 1 < starts.len() {
            succ.push(block_index + 1);
        }
        blocks.push(Block { start, end, succ });
    }
    blocks
}

/// The compact liveness result the coloring consumes.
pub(super) struct Liveness {
    /// Virtual register index -> `[min, max]` instruction indices over which it
    /// is busy. `allocate_register` temporaries are single-def, def-before-use,
    /// and statement-local, so the textual span from first to last occurrence is
    /// a sound, tight live interval (no dataflow needed for virtual registers).
    pub(super) vreg_interval: HashMap<u32, (usize, usize)>,
    /// Per-instruction occupancy of hardcoded physical registers: bit `p` set
    /// means physical `xP` is busy (live, used, or defined) at that instruction.
    /// Physical liveness *does* need dataflow (a value can be live across an
    /// instruction with no operand mentioning it), but over only 31 registers it
    /// fits one machine word, so it is cheap even on huge functions.
    pub(super) phys_busy_at: Vec<PhysMask>,
    /// Call/syscall instructions and the set of this class's physical registers
    /// each one clobbers (`call_clobber_mask`), sorted by instruction index. A
    /// value live across a call must avoid that call's clobbered registers.
    pub(super) call_clobber: Vec<(usize, PhysMask)>,
}

/// Occupancy bitset over physical registers `x0`–`x30` (31 < 64 bits).
pub(super) type PhysMask = u64;

pub(super) fn physical_busy(bits: PhysMask, index: u32) -> bool {
    bits & (1u64 << index) != 0
}

/// Per-instruction **live-out** of the integer physical registers, computed over
/// a fully-colored stream (no virtual registers remain). `live_out[i]` is the set
/// of `x0`–`x30` whose value at the point *after* instruction `i` may still be
/// read before being overwritten. Used by the FP-shuttle peephole to prove a GPR
/// that only carried a float's bit pattern is dead and the shuttle can be dropped.
///
/// A call destroys its caller-saved registers, so they are modeled as definitions
/// (killed) at the call — a value left in one is not live across it.
pub(super) fn integer_live_out(instructions: &[CodeInstruction]) -> Vec<PhysMask> {
    let model = ClassModel {
        parse_vreg: |_| None,
        physical_index: int_physical_index,
        is_fp: false,
    };
    let n = instructions.len();
    let blocks = build_cfg(instructions);
    let nb = blocks.len();

    let mut phys_def: Vec<PhysMask> = vec![0; n];
    let mut phys_use: Vec<PhysMask> = vec![0; n];
    for (i, instruction) in instructions.iter().enumerate() {
        let eff = effect(instruction, &model);
        if eff.is_call {
            phys_def[i] |= call_clobber_mask(instruction, false);
        }
        for d in &eff.defs {
            if let Some(p) = (model.physical_index)(d) {
                phys_def[i] |= 1u64 << p;
            }
        }
        for u in &eff.uses {
            if let Some(p) = (model.physical_index)(u) {
                phys_use[i] |= 1u64 << p;
            }
        }
    }

    let mut phys_in: Vec<PhysMask> = vec![0; nb];
    let mut changed = true;
    while changed {
        changed = false;
        for b in (0..nb).rev() {
            let mut live = 0u64;
            for &s in &blocks[b].succ {
                live |= phys_in[s];
            }
            for i in (blocks[b].start..blocks[b].end).rev() {
                live = (live & !phys_def[i]) | phys_use[i];
            }
            if live != phys_in[b] {
                phys_in[b] = live;
                changed = true;
            }
        }
    }

    let mut live_out: Vec<PhysMask> = vec![0; n];
    for b in 0..nb {
        let mut live = 0u64;
        for &s in &blocks[b].succ {
            live |= phys_in[s];
        }
        for i in (blocks[b].start..blocks[b].end).rev() {
            live_out[i] = live;
            live = (live & !phys_def[i]) | phys_use[i];
        }
    }
    live_out
}

/// Run CFG construction and liveness, returning compact per-virtual-register
/// intervals and per-instruction physical occupancy.
///
/// Physical-register liveness uses a single machine word (31 registers).
/// Virtual-register liveness uses sparse interned-id sets — a temporary held
/// across a loop back-edge is live for the whole loop, which a textual span
/// would miss, so real dataflow is required; but the live set at any point is
/// small (statement-local temporaries), so it stays fast even on the
/// multi-thousand-block generated functions.
pub(super) fn analyze(instructions: &[CodeInstruction], model: &ClassModel) -> Liveness {
    let n = instructions.len();
    let blocks = build_cfg(instructions);
    let nb = blocks.len();

    // Per-instruction physical def/use masks, virtual-register def/use id lists,
    // and call indices. Virtual registers are interned to dense ids.
    let mut phys_def: Vec<PhysMask> = vec![0; n];
    let mut phys_use: Vec<PhysMask> = vec![0; n];
    let mut vdef: Vec<Vec<u32>> = vec![Vec::new(); n];
    let mut vuse: Vec<Vec<u32>> = vec![Vec::new(); n];
    let mut call_clobber: Vec<(usize, PhysMask)> = Vec::new();
    // Virtual-register index -> dense id, and the reverse.
    let mut vid_of: HashMap<u32, u32> = HashMap::new();
    let mut vreg_of: Vec<u32> = Vec::new();
    let intern = |v: u32, vid_of: &mut HashMap<u32, u32>, vreg_of: &mut Vec<u32>| -> u32 {
        *vid_of.entry(v).or_insert_with(|| {
            let id = vreg_of.len() as u32;
            vreg_of.push(v);
            id
        })
    };
    for (i, instruction) in instructions.iter().enumerate() {
        let eff = effect(instruction, model);
        if eff.is_call {
            call_clobber.push((i, call_clobber_mask(instruction, model.is_fp)));
        }
        for d in &eff.defs {
            if let Some(p) = (model.physical_index)(d) {
                phys_def[i] |= 1u64 << p;
            } else if let Some(v) = (model.parse_vreg)(d) {
                vdef[i].push(intern(v, &mut vid_of, &mut vreg_of));
            }
        }
        for u in &eff.uses {
            if let Some(p) = (model.physical_index)(u) {
                phys_use[i] |= 1u64 << p;
            } else if let Some(v) = (model.parse_vreg)(u) {
                vuse[i].push(intern(v, &mut vid_of, &mut vreg_of));
            }
        }
    }

    // Physical-register liveness (single-word backward dataflow).
    let mut phys_in: Vec<PhysMask> = vec![0; nb];
    let mut changed = true;
    while changed {
        changed = false;
        for b in (0..nb).rev() {
            let mut live = 0u64;
            for &s in &blocks[b].succ {
                live |= phys_in[s];
            }
            for i in (blocks[b].start..blocks[b].end).rev() {
                live = (live & !phys_def[i]) | phys_use[i];
            }
            if live != phys_in[b] {
                phys_in[b] = live;
                changed = true;
            }
        }
    }
    let mut phys_busy_at: Vec<PhysMask> = vec![0; n];
    for b in 0..nb {
        let mut live = 0u64;
        for &s in &blocks[b].succ {
            live |= phys_in[s];
        }
        for i in (blocks[b].start..blocks[b].end).rev() {
            let live_in_i = (live & !phys_def[i]) | phys_use[i];
            phys_busy_at[i] = live_in_i | phys_def[i];
            live = live_in_i;
        }
    }

    // Virtual-register liveness (sparse backward dataflow over interned ids).
    let mut vin: Vec<std::collections::HashSet<u32>> = vec![std::collections::HashSet::new(); nb];
    let mut changed = true;
    while changed {
        changed = false;
        for b in (0..nb).rev() {
            let mut live: std::collections::HashSet<u32> = std::collections::HashSet::new();
            for &s in &blocks[b].succ {
                for &id in &vin[s] {
                    live.insert(id);
                }
            }
            for i in (blocks[b].start..blocks[b].end).rev() {
                for &d in &vdef[i] {
                    live.remove(&d);
                }
                for &u in &vuse[i] {
                    live.insert(u);
                }
            }
            if live != vin[b] {
                vin[b] = live;
                changed = true;
            }
        }
    }
    // Expand to virtual-register intervals: busy(i) = live-in(i) ∪ def(i).
    let mut vreg_interval: HashMap<u32, (usize, usize)> = HashMap::new();
    for b in 0..nb {
        let mut live: std::collections::HashSet<u32> = std::collections::HashSet::new();
        for &s in &blocks[b].succ {
            for &id in &vin[s] {
                live.insert(id);
            }
        }
        for i in (blocks[b].start..blocks[b].end).rev() {
            for &d in &vdef[i] {
                live.remove(&d);
            }
            for &u in &vuse[i] {
                live.insert(u);
            }
            let mut note = |id: u32| {
                let v = vreg_of[id as usize];
                let entry = vreg_interval.entry(v).or_insert((i, i));
                entry.0 = entry.0.min(i);
                entry.1 = entry.1.max(i);
            };
            for &id in &live {
                note(id);
            }
            for &d in &vdef[i] {
                note(d);
            }
        }
    }

    Liveness {
        vreg_interval,
        phys_busy_at,
        call_clobber,
    }
}
