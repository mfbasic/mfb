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
use super::parse_vreg;

/// Fields that name a register the instruction *writes*. AArch64 is
/// three-address with no tied operands, so a `dst` field is always a pure
/// definition.
const DEF_FIELDS: &[&str] = &["dst"];

/// Fields that name a register the instruction *reads*.
const USE_FIELDS: &[&str] = &["src", "lhs", "rhs", "minuend", "base", "register", "addend"];

/// Whether `name` is an integer register the allocator tracks: a virtual
/// register, or a physical `x0`–`x30`. Excludes `x31`/`xzr` (the constant zero),
/// `sp`, and any FP/SIMD (`d*`/`v*`) register (Stage B allocates only the integer
/// class).
pub(super) fn is_tracked_int(name: &str) -> bool {
    if parse_vreg(name).is_some() {
        return true;
    }
    physical_index(name).is_some()
}

/// The physical-register index `0..=30` of a tracked physical integer register
/// (`x0`–`x30`), or `None` for a virtual register, `x31`/`xzr`, `sp`, or an FP
/// register.
pub(super) fn physical_index(name: &str) -> Option<u32> {
    let rest = name.strip_prefix('x')?;
    let n: u32 = rest.parse().ok()?;
    if n <= 30 {
        Some(n)
    } else {
        None
    }
}

/// The integer registers an instruction defines and uses (tracked names only),
/// and whether it is a call/syscall (clobbers caller-saved registers).
pub(super) struct Effect {
    pub(super) defs: Vec<String>,
    pub(super) uses: Vec<String>,
    pub(super) is_call: bool,
}

pub(super) fn effect(instruction: &CodeInstruction) -> Effect {
    let mut defs = Vec::new();
    let mut uses = Vec::new();
    for (name, value) in &instruction.fields {
        if DEF_FIELDS.contains(name) {
            if is_tracked_int(value) {
                defs.push(value.clone());
            }
        } else if USE_FIELDS.contains(name) && is_tracked_int(value) {
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
    /// Sorted indices of call/syscall instructions.
    pub(super) call_idx: Vec<usize>,
}

/// Occupancy bitset over physical registers `x0`–`x30` (31 < 64 bits).
pub(super) type PhysMask = u64;

pub(super) fn physical_busy(bits: PhysMask, index: u32) -> bool {
    bits & (1u64 << index) != 0
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
pub(super) fn analyze(instructions: &[CodeInstruction]) -> Liveness {
    let n = instructions.len();
    let blocks = build_cfg(instructions);
    let nb = blocks.len();

    // Per-instruction physical def/use masks, virtual-register def/use id lists,
    // and call indices. Virtual registers are interned to dense ids.
    let mut phys_def: Vec<PhysMask> = vec![0; n];
    let mut phys_use: Vec<PhysMask> = vec![0; n];
    let mut vdef: Vec<Vec<u32>> = vec![Vec::new(); n];
    let mut vuse: Vec<Vec<u32>> = vec![Vec::new(); n];
    let mut call_idx: Vec<usize> = Vec::new();
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
        let eff = effect(instruction);
        if eff.is_call {
            call_idx.push(i);
        }
        for d in &eff.defs {
            if let Some(p) = physical_index(d) {
                phys_def[i] |= 1u64 << p;
            } else if let Some(v) = parse_vreg(d) {
                vdef[i].push(intern(v, &mut vid_of, &mut vreg_of));
            }
        }
        for u in &eff.uses {
            if let Some(p) = physical_index(u) {
                phys_use[i] |= 1u64 << p;
            } else if let Some(v) = parse_vreg(u) {
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
        call_idx,
    }
}
