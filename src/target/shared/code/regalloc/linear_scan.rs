//! Linear-scan register allocator for the integer class (plan-03 Stage B).
//!
//! Colors `allocate_register` virtual registers to physical registers by
//! liveness, reusing a register as soon as its previous occupant dies, and
//! spilling to a stack slot under pressure (removing the legacy "break nested
//! expressions into LETs" hard failure). It is sound by construction:
//!
//! * a virtual register is never colored to a physical that is *busy* (live,
//!   used, or defined) anywhere in the virtual register's live interval — this
//!   covers both the hand-written lowerings' hardcoded physicals and other
//!   colored virtual registers;
//! * a virtual register whose live interval crosses a call is spilled, because no
//!   register survives an internal runtime helper (`_mfb_arena_alloc` clobbers
//!   callee-saved `x20`–`x28`; see `.ai/compiler.md`).
//!
//! Liveness is represented as over-approximating per-virtual-register intervals
//! (sound: wider intervals only forbid more), so coloring is a near-linear scan
//! with binary-search interference checks — fast even on the multi-thousand-
//! instruction generated functions (the regex engine).

use std::collections::HashMap;

use crate::arch::aarch64::regmodel::{RegClass, RegisterModel};

use super::super::types::CodeInstruction;
use super::analysis::{self, physical_busy, ClassModel};

pub(super) struct RunResult {
    pub(super) instructions: Vec<CodeInstruction>,
    pub(super) spill_slot_count: usize,
    pub(super) extra_callee_saved: Vec<String>,
}

/// Allocate one register class over `instructions`. Spill slots are placed at
/// `spill_base_offset + k*8` (pre-prologue `sp`-relative, shifted later by
/// `finalize_frame` like every other stack access). The two physical files never
/// interfere, so the Int and Fp classes are each allocated by a separate call.
pub(super) fn run(
    instructions: &[CodeInstruction],
    model: &dyn RegisterModel,
    class: RegClass,
    class_model: &ClassModel,
    spill_base_offset: usize,
) -> RunResult {
    let live = analysis::analyze(instructions, class_model);
    let n = instructions.len();

    // Per-physical sorted index lists where the physical is busy, for O(log)
    // "is physical p busy anywhere in [s, e]" interference checks. 32 covers
    // both x0–x30 and d0–d31.
    let mut phys_busy_indices: Vec<Vec<usize>> = vec![Vec::new(); 32];
    for (i, &mask) in live.phys_busy_at.iter().enumerate() {
        if mask == 0 {
            continue;
        }
        for p in 0..32u32 {
            if physical_busy(mask, p) {
                phys_busy_indices[p as usize].push(i);
            }
        }
    }
    let phys_busy_in = |p: u32, s: usize, e: usize| -> bool {
        let idx = &phys_busy_indices[p as usize];
        match idx.binary_search(&s) {
            Ok(_) => true,
            Err(pos) => idx.get(pos).is_some_and(|&j| j <= e),
        }
    };
    let crosses_call = |s: usize, e: usize| -> bool {
        match live.call_idx.binary_search(&s) {
            Ok(_) => true,
            Err(pos) => live.call_idx.get(pos).is_some_and(|&j| j <= e),
        }
    };

    // Allocatable physicals as (name, index), in preference order.
    let allocatable: Vec<(&str, u32)> = model
        .allocatable(class)
        .iter()
        .map(|&name| {
            (
                name,
                (class_model.physical_index)(name).expect("allocatable must be a class register"),
            )
        })
        .collect();

    // Virtual registers sorted by interval start for the linear scan.
    let mut vregs: Vec<(u32, usize, usize)> = live
        .vreg_interval
        .iter()
        .map(|(&v, &(s, e))| (v, s, e))
        .collect();
    vregs.sort_by_key(|&(_, s, _)| s);

    // Active intervals: (end, vreg, phys_index), and the mask of physicals they
    // hold. Expired by start order.
    let mut active: Vec<(usize, u32, u32)> = Vec::new();
    let mut active_mask: u64 = 0;
    let mut assignment: HashMap<u32, String> = HashMap::new();
    let mut assigned_index: HashMap<u32, u32> = HashMap::new();
    let mut spilled: Vec<u32> = Vec::new();

    for &(v, s, e) in &vregs {
        // Expire intervals that ended before this one starts.
        active.retain(|&(end, _, pi)| {
            if end < s {
                active_mask &= !(1u64 << pi);
                false
            } else {
                true
            }
        });
        if crosses_call(s, e) {
            spilled.push(v);
            continue;
        }
        let choice = allocatable.iter().find(|&&(_, pi)| {
            (active_mask & (1u64 << pi)) == 0 && !phys_busy_in(pi, s, e)
        });
        match choice {
            Some(&(name, pi)) => {
                assignment.insert(v, name.to_string());
                assigned_index.insert(v, pi);
                active.push((e, v, pi));
                active_mask |= 1u64 << pi;
            }
            None => spilled.push(v),
        }
    }

    // Assign a stack slot to each spilled vreg.
    let mut spill_slot: HashMap<u32, usize> = HashMap::new();
    for &v in &spilled {
        let k = spill_slot.len();
        spill_slot.insert(v, spill_base_offset + k * 8);
    }
    let spill_slot_count = spill_slot.len();

    // Per-instruction physical occupancy after coloring (hardcoded physicals plus
    // colored virtual registers live there), used to pick spill scratch. Built
    // only when there are spills.
    let colored_mask_at = if spilled.is_empty() {
        Vec::new()
    } else {
        let mut masks = live.phys_busy_at.clone();
        for (&v, &(s, e)) in &live.vreg_interval {
            if let Some(&pi) = assigned_index.get(&v) {
                for m in masks.iter_mut().take(e + 1).skip(s) {
                    *m |= 1u64 << pi;
                }
            }
        }
        masks
    };

    // Rewrite the stream.
    let spilled_set: std::collections::HashSet<u32> = spilled.iter().copied().collect();
    let mut out: Vec<CodeInstruction> = Vec::with_capacity(n);
    for (i, instruction) in instructions.iter().enumerate() {
        let eff = analysis::effect(instruction, class_model);
        let used_spilled: Vec<u32> = eff
            .uses
            .iter()
            .filter_map(|name| (class_model.parse_vreg)(name))
            .filter(|v| spilled_set.contains(v))
            .collect();
        let def_spilled: Vec<u32> = eff
            .defs
            .iter()
            .filter_map(|name| (class_model.parse_vreg)(name))
            .filter(|v| spilled_set.contains(v))
            .collect();

        let mut scratch_for: HashMap<u32, String> = HashMap::new();
        if !used_spilled.is_empty() || !def_spilled.is_empty() {
            let mut taken = occupied_at(i, &colored_mask_at, instruction, class_model);
            for &v in used_spilled.iter().chain(def_spilled.iter()) {
                if scratch_for.contains_key(&v) {
                    continue;
                }
                let scratch = allocatable
                    .iter()
                    .find(|&&(_, pi)| (taken & (1u64 << pi)) == 0)
                    .map(|&(name, pi)| (name, pi))
                    .expect("register allocator: no scratch physical free for a spill");
                taken |= 1u64 << scratch.1;
                scratch_for.insert(v, scratch.0.to_string());
            }
        }

        for &v in &used_spilled {
            out.push(model.emit_reload(class, &scratch_for[&v], spill_slot[&v]));
        }
        out.push(substitute(instruction, &assignment, &scratch_for, class_model));
        for &v in &def_spilled {
            out.push(model.emit_spill(class, &scratch_for[&v], spill_slot[&v]));
        }
    }

    let mut extra_callee_saved: Vec<String> = Vec::new();
    for phys in assignment.values() {
        if model.is_callee_saved(phys) && !extra_callee_saved.iter().any(|s| s == phys) {
            extra_callee_saved.push(phys.clone());
        }
    }
    extra_callee_saved.sort();

    RunResult {
        instructions: out,
        spill_slot_count,
        extra_callee_saved,
    }
}

/// The physical-occupancy mask at instruction `i` (colored occupancy plus the
/// instruction's own literal physical operands of this class), for spill-scratch
/// selection.
fn occupied_at(
    i: usize,
    colored_mask_at: &[u64],
    instruction: &CodeInstruction,
    class_model: &ClassModel,
) -> u64 {
    let mut mask = colored_mask_at.get(i).copied().unwrap_or(0);
    for (_field, value) in &instruction.fields {
        if let Some(p) = (class_model.physical_index)(value) {
            mask |= 1u64 << p;
        }
    }
    mask
}

/// Produce a copy of `instruction` with this class's virtual-register operands
/// replaced: colored vregs by their physical, spilled vregs by their
/// per-instruction scratch.
fn substitute(
    instruction: &CodeInstruction,
    assignment: &HashMap<u32, String>,
    scratch_for: &HashMap<u32, String>,
    class_model: &ClassModel,
) -> CodeInstruction {
    let mut copy = CodeInstruction {
        op: instruction.op,
        fields: instruction.fields.clone(),
    };
    for (_field, value) in copy.fields.iter_mut() {
        if let Some(v) = (class_model.parse_vreg)(value) {
            if let Some(phys) = assignment.get(&v) {
                *value = phys.clone();
            } else if let Some(scratch) = scratch_for.get(&v) {
                *value = scratch.clone();
            }
        }
    }
    copy
}
