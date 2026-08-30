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

// --- codegen tier imports (migration) ---
use crate::arch::ops::CodeOp;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::target::shared::regmodel::{RegClass, RegisterModel};

use super::analysis::{self, physical_busy, ClassModel, RegRef};
use crate::optimizer::opt2::plans::mark::removable_op as mark_removable;

pub(crate) struct RunResult {
    pub(crate) instructions: Vec<CodeInstruction>,
    pub(crate) spill_slot_count: usize,
    pub(crate) extra_callee_saved: Vec<String>,
    /// Set when coloring could not represent an instruction (it names more
    /// simultaneously-live registers than the target's allocatable pool holds),
    /// so no valid allocation exists. `allocate` surfaces this as a clear
    /// compile-time failure rather than the raw `.expect` ICE it replaced
    /// (bug-127.2). `None` on success.
    pub(crate) error: Option<String>,
}

/// Allocate one register class over `instructions`. Spill slots are placed at
/// `spill_base_offset + k*8` (pre-prologue `sp`-relative, shifted later by
/// `finalize_frame` like every other stack access). The two physical files never
/// interfere, so the Int and Fp classes are each allocated by a separate call.
pub(crate) fn run(
    instructions: Vec<CodeInstruction>,
    model: &dyn RegisterModel,
    class: RegClass,
    class_model: &ClassModel,
    spill_base_offset: usize,
    slot_bytes: usize,
    reserved: &[&str],
) -> RunResult {
    let n = instructions.len();
    // plan-78-C compute-once: classify every instruction's effect a single time
    // and share it between the liveness pass and the rewrite loop below (the two
    // passes over this stream that previously each recomputed `effect`).
    let effects: Vec<analysis::Effect> = instructions
        .iter()
        .map(|instruction| analysis::effect(instruction, class_model))
        .collect();
    let live = analysis::analyze(&instructions, class_model, &effects);

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
    // The set of this class's physical registers clobbered by any call in the
    // inclusive span `[s, e]` (`idx > e` breaks, so `idx == e` is included) — a
    // value live across those calls must avoid them.
    let call_clobber_in = |s: usize, e: usize| -> u64 {
        let start = live.call_clobber.partition_point(|&(idx, _)| idx < s);
        let mut mask = 0u64;
        for &(idx, m) in &live.call_clobber[start..] {
            if idx > e {
                break;
            }
            mask |= m;
        }
        mask
    };

    // Allocatable physicals as (name, index), in preference order. `reserved`
    // registers are held out of allocation entirely (used neither as a value's
    // home nor as spill scratch / eviction victim), so a physical the codegen
    // pins for a fixed role (e.g. the arena-state register, the syscall
    // number/argument registers) is never handed to the allocator to color.
    let allocatable: Vec<(&'static str, u32)> = model
        .allocatable(class)
        .iter()
        .filter(|&&name| !reserved.contains(&name))
        .map(|&name| {
            (
                name,
                (class_model.physical_index)(name).expect("allocatable must be a class register"),
            )
        })
        .collect();

    // The last index `>= s` (bounded by `e`) at which `p` is still free of
    // hardcoded occupancy and of call clobbers — the largest head range a
    // split could give it. `None` when `p` is busy at `s` itself.
    let first_free_until = |p: u32, s: usize, e: usize| -> Option<usize> {
        let busy = &phys_busy_indices[p as usize];
        let next_busy = match busy.binary_search(&s) {
            Ok(_) => return None,
            Err(pos) => busy.get(pos).copied().unwrap_or(usize::MAX),
        };
        let next_clobber = live
            .call_clobber
            .iter()
            .find(|&&(idx, mask)| idx >= s && (mask & (1u64 << p)) != 0)
            .map(|&(idx, _)| idx)
            .unwrap_or(usize::MAX);
        let limit = next_busy.min(next_clobber);
        if limit <= s {
            return None;
        }
        Some((limit - 1).min(e))
    };

    // Virtual registers sorted by interval start for the linear scan. Tie-break
    // by vreg id so vregs sharing a start are colored in a deterministic order:
    // `vreg_interval` is a HashMap, so a start-only key left tied vregs in
    // per-process-random iteration order, making register/spill selection — and
    // thus the emitted bytes — nondeterministic across builds (bug-87).
    let mut vregs: Vec<(u32, usize, usize)> = live
        .vreg_interval
        .iter()
        .map(|(&v, &(s, e))| (v, s, e))
        .collect();
    vregs.sort_by_key(|&(v, s, _)| (s, v));

    // Coalescing hints (`optimizer` catalog row "Register coalescing", Level
    // 2): for a `mov dst, src` between two vregs of this class where `src`
    // dies at the copy, giving `dst` the same physical turns the copy into
    // `mov xN, xN`, which the sweep at the end of this function deletes. The
    // hint is only ever *tried* — every interference test below still has to
    // pass, so coalescing can lose a register choice but never make an unsafe
    // one.
    let coalescing = crate::optimizer::level_enabled(2);
    let mut copy_hint: analysis::U32Map<u32> = analysis::U32Map::default();
    if coalescing {
        for (i, instruction) in instructions.iter().enumerate() {
            let is_copy = matches!(instruction.op, CodeOp::Mov | CodeOp::FMovDFromD);
            if !is_copy {
                continue;
            }
            let eff = &effects[i];
            if let (Some(RegRef::VReg(dst)), Some(RegRef::VReg(src))) =
                (eff.defs.first(), eff.uses.first())
            {
                // Only when the copy is the source's last use: then the two
                // values never coexist and may share one register.
                if live.vreg_interval.get(src).is_some_and(|&(_, e)| e == i)
                    && live.vreg_interval.get(dst).is_some_and(|&(ds, _)| ds == i)
                {
                    copy_hint.insert(*dst, *src);
                }
            }
        }
    }

    // Active intervals: (end, vreg, phys_index), and the mask of physicals they
    // hold. Expired by start order.
    let mut active: Vec<(usize, u32, u32)> = Vec::new();
    let mut active_mask: u64 = 0;
    // plan-82-B: a colored vreg records its physical `(name, index)` — the static
    // name (no heap box) that the rewrite writes into `Operand::Phys`, and the
    // class index. `assigned_index` keeps the bare `u32` for `colored_mask_sweep`
    // and the operand-mask (both purely numeric).
    let mut assignment: analysis::U32Map<(&'static str, u32)> = analysis::U32Map::default();
    let mut assigned_index: analysis::U32Map<u32> = analysis::U32Map::default();
    let mut spilled: Vec<u32> = Vec::new();
    let mut coalesced = 0u64;
    let splitting = crate::optimizer::level_enabled(2);
    // A split value's tail range: `(first instruction of the tail, physical)`.
    // The head range stays in `assignment`, so every consumer that is not
    // index-aware keeps seeing a single, valid home.
    let mut split_tail: analysis::U32Map<(usize, &'static str, u32)> = analysis::U32Map::default();
    let mut split_ranges = 0u64;

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
        // Registers the calls in this value's interval destroy. The value must
        // avoid them (plan-03 §4.3); e.g. across `_mfb_arena_alloc` an FP value is
        // unrestricted (it touches no FP) while an integer value avoids `x20`–`x28`.
        let clobbered = call_clobber_in(s, e);
        let usable = |pi: u32, s: usize, e: usize, active_mask: u64| -> bool {
            (active_mask & (1u64 << pi)) == 0
                && (call_clobber_in(s, e) & (1u64 << pi)) == 0
                && !phys_busy_in(pi, s, e)
        };
        // Coalescing first: reuse the copy source's register when it is free
        // apart from the source itself, which dies at this very instruction.
        let hinted = copy_hint.get(&v).and_then(|src| {
            let &(name, pi) = assignment.get(src)?;
            let held_only_by_source = active
                .iter()
                .all(|&(end, holder, held)| held != pi || (holder == *src && end <= s));
            (held_only_by_source && (clobbered & (1u64 << pi)) == 0 && !phys_busy_in(pi, s, e))
                .then_some((name, pi, *src))
        });
        if let Some((name, pi, src)) = hinted {
            // The source is dead here, so drop its claim before taking over —
            // otherwise its later expiry would clear a bit this value holds.
            active.retain(|&(_, holder, held)| !(holder == src && held == pi));
            assignment.insert(v, (name, pi));
            assigned_index.insert(v, pi);
            active.push((e, v, pi));
            active_mask |= 1u64 << pi;
            coalesced += 1;
            continue;
        }
        let choice = allocatable
            .iter()
            .find(|&&(_, pi)| usable(pi, s, e, active_mask));
        match choice {
            Some(&(name, pi)) => {
                assignment.insert(v, (name, pi));
                assigned_index.insert(v, pi);
                active.push((e, v, pi));
                active_mask |= 1u64 << pi;
            }
            None => {
                // "Live-range splitting" (Level 2): no single register is free
                // for the whole interval, but two may cover it end to end —
                // one for `[s, m]`, another for `[m + 1, e]`, joined by a copy
                // the rewrite inserts at the boundary. That keeps the value in
                // registers where the unsplit scan would have spilled it to
                // memory for its entire life.
                let split = splitting
                    .then(|| {
                        allocatable.iter().find_map(|&(first_name, first)| {
                            if (active_mask & (1u64 << first)) != 0 {
                                return None;
                            }
                            // How far this register stays free from `s`.
                            let m = first_free_until(first, s, e)?;
                            if m >= e {
                                return None; // would have been a whole-interval fit
                            }
                            let (second_name, second) = allocatable
                                .iter()
                                .find(|&&(_, pi)| pi != first && usable(pi, m + 1, e, active_mask))
                                .copied()?;
                            Some((first_name, first, m, second_name, second))
                        })
                    })
                    .flatten();
                match split {
                    Some((first_name, first, m, second_name, second)) => {
                        assignment.insert(v, (first_name, first));
                        assigned_index.insert(v, first);
                        active.push((m, v, first));
                        active_mask |= 1u64 << first;
                        // The tail range holds its own register to the end.
                        split_tail.insert(v, (m + 1, second_name, second));
                        active.push((e, v, second));
                        active_mask |= 1u64 << second;
                        split_ranges += 1;
                    }
                    None => spilled.push(v),
                }
            }
        }
    }

    // Assign a stack slot to each spilled vreg. With the "Stack slot coloring"
    // row (Level 2) on, slots are *reused* by values whose live intervals do
    // not overlap — a linear scan over the slots exactly like the register
    // scan above, shrinking the frame. Off, every spilled value gets its own
    // slot (the original behavior).
    let slot_coloring = crate::optimizer::level_enabled(2);
    let mut spill_slot: analysis::U32Map<usize> = analysis::U32Map::default();
    // Per slot index, the end of the interval currently occupying it.
    let mut slot_end: Vec<usize> = Vec::new();
    let mut slots_reused = 0u64;
    // Spills are discovered in interval-start order (the scan above), so this
    // walk is already sorted by start.
    for &v in &spilled {
        let (s, e) = live.vreg_interval[&v];
        let reused = slot_coloring
            .then(|| slot_end.iter().position(|&end| end < s))
            .flatten();
        let k = match reused {
            Some(k) => {
                slot_end[k] = e;
                slots_reused += 1;
                k
            }
            None => {
                slot_end.push(e);
                slot_end.len() - 1
            }
        };
        spill_slot.insert(v, spill_base_offset + k * slot_bytes);
    }
    let spill_slot_count = slot_end.len();

    // "Rematerialization" (Level 2): a spilled value whose definition is a
    // self-contained constant materialization is cheaper to recompute at each
    // use than to store and reload. Only `mov_imm` qualifies — it reads no
    // register, so re-executing it anywhere yields the identical bits — and
    // only when the value has exactly one definition, so every use wants that
    // same constant. The spill store is then dead and is skipped too.
    let remat_enabled = crate::optimizer::level_enabled(2);
    let mut remat: analysis::U32Map<CodeInstruction> = analysis::U32Map::default();
    if remat_enabled && !spilled.is_empty() {
        let mut defs_of: analysis::U32Map<usize> = analysis::U32Map::default();
        let mut multi: analysis::U32Set = analysis::U32Set::default();
        for (i, eff) in effects.iter().enumerate() {
            for def in &eff.defs {
                if let RegRef::VReg(v) = def {
                    if defs_of.insert(*v, i).is_some() {
                        multi.insert(*v);
                    }
                }
            }
        }
        for &v in &spilled {
            let Some(&i) = defs_of.get(&v) else { continue };
            if multi.contains(&v) || instructions[i].op != CodeOp::MovImm {
                continue;
            }
            // A `mov_imm` with no register inputs: its value is positional-
            // independent, so it can be re-emitted at any use.
            if !effects[i].uses.is_empty() {
                continue;
            }
            remat.insert(v, instructions[i].clone());
        }
    }
    let remat_count = remat.len() as u64;

    // Per-instruction physical occupancy after coloring (hardcoded physicals plus
    // colored virtual registers live there), used to pick spill scratch. Built
    // only when there are spills.
    let colored_mask_at = if spilled.is_empty() {
        Vec::new()
    } else {
        colored_mask_sweep(&live.phys_busy_at, &live.vreg_interval, &assigned_index)
    };

    // Rewrite the stream. Evict-slot base sits just past the per-value spill
    // slots; the most evictions any single instruction needs sets how many of
    // those slots the frame must reserve.
    let evict_base = spill_base_offset + spill_slot_count * slot_bytes;
    let mut max_evictions = 0usize;
    let spilled_set: analysis::U32Set = spilled.iter().copied().collect();
    // Callee-saved registers commandeered by the *genuinely-free* scratch branch
    // below. Unlike a colored home (recorded from `assignment` later) or an
    // eviction victim (save/restored around its single use), a genuinely-free
    // callee-saved scratch is written by this function and never bracketed, so
    // the frame must save/restore it or the caller's value in it is silently
    // clobbered (bug-54). Collected here and merged into `extra_callee_saved`.
    let mut scratch_callee_saved: Vec<String> = Vec::new();
    // Split boundaries in stream order: at each one the value moves from its
    // head register to its tail register, so the rewrite emits the copy and
    // re-points `assignment` — every later instruction then substitutes the
    // tail register with no per-instruction bookkeeping.
    let mut boundaries: Vec<(usize, u32, &'static str, u32)> = split_tail
        .iter()
        .map(|(&v, &(at, name, pi))| (at, v, name, pi))
        .collect();
    boundaries.sort_by_key(|&(at, v, _, _)| (at, v));
    let mut next_boundary = 0usize;
    // Every physical this allocation hands out as a home, including a split
    // value's head register (which `assignment` stops naming once the boundary
    // re-points it) — the callee-saved sweep below must see all of them.
    let mut every_home: Vec<(&'static str, u32)> = assignment.values().copied().collect();
    every_home.extend(split_tail.values().map(|&(_, name, index)| (name, index)));
    let mut assignment = assignment;
    let mut out: Vec<CodeInstruction> = Vec::with_capacity(n);
    // Set if an instruction cannot be colored (more simultaneously-live registers
    // than the pool holds); surfaced by `allocate` (bug-127.2).
    let mut alloc_error: Option<String> = None;
    // Consume the input stream: each instruction is moved through `substitute`,
    // which rewrites only its this-class vreg operands in place (plan-84 Phase 3),
    // so the `fields` Vec is carried rather than re-cloned once per class pass.
    'rewrite: for (i, instruction) in instructions.into_iter().enumerate() {
        // Cross any split boundary starting here before the instruction is
        // substituted, so this instruction already reads the tail register.
        while next_boundary < boundaries.len() && boundaries[next_boundary].0 == i {
            let (_, v, tail_name, tail_index) = boundaries[next_boundary];
            next_boundary += 1;
            let Some(&(head_name, head_index)) = assignment.get(&v) else {
                continue;
            };
            let dst = Operand::phys(class, tail_index, tail_name);
            let src = Operand::phys(class, head_index, head_name);
            out.push(match class {
                RegClass::Int => crate::target::shared::abi::move_register(dst, src),
                RegClass::Fp => crate::target::shared::abi::float_move_d_from_d(dst, src),
            });
            assignment.insert(v, (tail_name, tail_index));
        }
        // plan-78-C: reuse the effect classified once above (no re-parse).
        let eff = &effects[i];
        let spilled_vreg = |reg: &RegRef| -> Option<u32> {
            match reg {
                RegRef::VReg(v) if spilled_set.contains(v) => Some(*v),
                _ => None,
            }
        };
        let used_spilled: Vec<u32> = eff.uses.iter().filter_map(spilled_vreg).collect();
        let def_spilled: Vec<u32> = eff.defs.iter().filter_map(spilled_vreg).collect();

        // plan-82-B: a spilled vreg's per-instruction scratch physical, as
        // `(name, index)` — the static name for `emit_spill`/`emit_reload` and the
        // `Operand::Phys` the rewrite writes, plus the class index.
        let mut scratch_for: analysis::U32Map<(&'static str, u32)> = analysis::U32Map::default();
        // Registers this instruction actually reads or writes (after coloring) —
        // a spill scratch may never reuse one of these, even by eviction.
        let mut operand_mask = 0u64;
        for reg in eff.defs.iter().chain(eff.uses.iter()) {
            match *reg {
                RegRef::Phys(p) => operand_mask |= 1u64 << p,
                RegRef::VReg(v) => {
                    if let Some(&pi) = assigned_index.get(&v) {
                        operand_mask |= 1u64 << pi;
                    }
                }
            }
        }
        // (victim physical, evict-slot index) for registers commandeered by eviction.
        let mut evictions: Vec<(String, usize)> = Vec::new();
        if !used_spilled.is_empty() || !def_spilled.is_empty() {
            // `occupied` holds a live value (no free scratch there); `reserved`
            // also tracks the operands and scratches in use at this instruction
            // (an eviction victim may not be one of those).
            let mut occupied = occupied_at(i, &colored_mask_at, &instruction, class_model);
            let mut reserved = operand_mask;
            for &v in used_spilled.iter().chain(def_spilled.iter()) {
                if scratch_for.contains_key(&v) {
                    continue;
                }
                if let Some(&(name, pi)) = allocatable
                    .iter()
                    .find(|&&(_, pi)| (occupied & (1u64 << pi)) == 0)
                {
                    // A genuinely free register — no per-use save/restore needed
                    // (nothing live is there to preserve around this one use).
                    // But if it is callee-saved, this function still *writes* it,
                    // and the caller relies on the PCS preserving it, so it must
                    // be added to the frame's save set — exactly like a
                    // callee-saved colored home (bug-54).
                    occupied |= 1u64 << pi;
                    reserved |= 1u64 << pi;
                    if model.is_callee_saved(name)
                        && !scratch_callee_saved.iter().any(|s| s == name)
                    {
                        scratch_callee_saved.push(name.to_string());
                    }
                    scratch_for.insert(v, (name, pi));
                } else {
                    // Every register is live, so commandeer one that this instruction
                    // does not itself use, saving and restoring it around the use.
                    // One exists whenever the pool is at least as large as the
                    // instruction's distinct register-operand count. If it is not
                    // — e.g. a 5-operand `add_carry` all spilled against x86's
                    // 4-register integer pool — no valid allocation exists: distinct
                    // simultaneously-live operands need distinct homes, so scratch
                    // cannot be reused. Surface a hard error via `RunResult` instead
                    // of the raw `.expect` ICE this replaced (bug-127.2); `allocate`
                    // turns it into a clear compile-time failure.
                    let Some(&(name, pi)) = allocatable
                        .iter()
                        .find(|&&(_, pi)| (reserved & (1u64 << pi)) == 0)
                    else {
                        alloc_error = Some(format!(
                            "register allocator: instruction `{}` names more \
                             simultaneously-live registers than the {} allocatable \
                             {class:?}-class registers this target provides",
                            instruction.op.mnemonic(),
                            allocatable.len(),
                        ));
                        break 'rewrite;
                    };
                    reserved |= 1u64 << pi;
                    let slot_index = evictions.len();
                    evictions.push((name.to_string(), slot_index));
                    scratch_for.insert(v, (name, pi));
                }
            }
        }
        max_evictions = max_evictions.max(evictions.len());

        // Save evicted registers, reload used spills, run the instruction, store
        // defined spills, then restore the evicted registers.
        for (victim, slot) in &evictions {
            out.push(model.emit_spill(class, victim, evict_base + slot * slot_bytes));
        }
        for &v in &used_spilled {
            match remat.get(&v) {
                // Recompute instead of reloading: the definition is a constant
                // materialization, re-targeted at this use's scratch register.
                Some(definition) => {
                    let mut copy = definition.clone();
                    for (name, operand) in copy.fields.iter_mut() {
                        if *name == "dst" {
                            *operand = Operand::phys(class, scratch_for[&v].1, scratch_for[&v].0);
                        }
                    }
                    out.push(copy);
                }
                None => out.push(model.emit_reload(class, scratch_for[&v].0, spill_slot[&v])),
            }
        }
        out.push(substitute(
            instruction,
            class,
            &assignment,
            &scratch_for,
            class_model,
        )); // moves `instruction` (plan-84 Phase 3)
        for &v in &def_spilled {
            // A rematerialized value is never reloaded from its slot, so the
            // store is dead.
            if remat.contains_key(&v) {
                continue;
            }
            out.push(model.emit_spill(class, scratch_for[&v].0, spill_slot[&v]));
        }
        for (victim, slot) in evictions.iter().rev() {
            out.push(model.emit_reload(class, victim, evict_base + slot * slot_bytes));
        }
    }
    // "Spill-code optimization" (Level 2): the rewrite above emits a reload
    // before every use of a spilled value and a store after every definition,
    // independently. Two redundancies follow directly from that shape and are
    // provable from the emitted stream alone:
    //
    //   * a reload of a slot into the register that a store *just* wrote to
    //     that same slot, with nothing in between — the value is already
    //     there;
    //   * a second reload of the same slot into the same register with no
    //     intervening store to it and no redefinition of the register.
    //
    // Both delete a load whose result provably already sits in the register,
    // so nothing about values, flags, or memory changes. Only the spill and
    // evict slots this function itself owns are considered, and any
    // instruction that writes the register or the slot — or that this pass
    // does not model — ends the window.
    let mut spill_loads_removed = 0u64;
    if crate::optimizer::level_enabled(2) {
        // (slot offset, register) currently known to hold the same value.
        let mut resident: Vec<(usize, String)> = Vec::new();
        let mut keep = vec![true; out.len()];
        for (index, instruction) in out.iter().enumerate() {
            let slot = instruction
                .get("offset")
                .and_then(|text| text.parse::<usize>().ok())
                .filter(|_| {
                    instruction
                        .operand("base")
                        .is_some_and(|base| base.rendered() == "sp")
                });
            let register = instruction.get(if instruction.op == CodeOp::StrU64 {
                "src"
            } else {
                "dst"
            });
            match (instruction.op, slot, register) {
                (CodeOp::StrU64, Some(slot), Some(register)) => {
                    // The slot now holds this register's value; any other
                    // register's claim on it is stale.
                    resident.retain(|(other, _)| *other != slot);
                    resident.push((slot, register));
                }
                (CodeOp::LdrU64, Some(slot), Some(register)) => {
                    if resident
                        .iter()
                        .any(|(other, held)| *other == slot && *held == register)
                    {
                        keep[index] = false;
                        spill_loads_removed += 1;
                        continue;
                    }
                    // This register now holds the slot; it holds nothing else.
                    resident.retain(|(other, held)| *other != slot && *held != register);
                    resident.push((slot, register));
                }
                _ => {
                    // Only the provably pure, memory-free ops may leave claims
                    // standing, and even then a claim on the register they
                    // write is gone. EVERYTHING else clears the whole set.
                    //
                    // The first version keyed this on "does this instruction
                    // define a register of this class?", which is wrong in the
                    // one case that matters most: a `bl` names no `dst`, so it
                    // looked definition-free and left claims standing across a
                    // call — but a call destroys the caller-saved registers
                    // holding them, so the next reload was deleted and the
                    // callee's garbage was read instead. That segfaulted 600+
                    // fixtures' error paths at `-O2`.
                    let pure = mark_removable(instruction.op)
                        || matches!(
                            instruction.op,
                            CodeOp::Adds | CodeOp::Subs | CodeOp::Cmp | CodeOp::CmpImm
                        );
                    if !pure {
                        resident.clear();
                    } else if let Some(written) = instruction.get("dst") {
                        resident.retain(|(_, held)| *held != written);
                    }
                }
            }
        }
        if spill_loads_removed != 0 {
            let mut index = 0;
            out.retain(|_| {
                let keep = keep[index];
                index += 1;
                keep
            });
        }
    }
    crate::optimizer::stats::count_spill_code_removed(spill_loads_removed);

    // Coalescing's payoff: a copy whose source and destination were colored to
    // the same register is a no-op. Deleting it is unconditionally safe — a
    // register move sets no flags and touches no memory — and it is the only
    // way the hint above turns into fewer instructions. Self-moves the input
    // already contained are removed too; they were equally dead.
    let mut copies_removed = 0u64;
    if coalescing {
        out.retain(|instruction| {
            let self_move = matches!(instruction.op, CodeOp::Mov | CodeOp::FMovDFromD)
                && match (instruction.operand("dst"), instruction.operand("src")) {
                    (Some(dst), Some(src)) => dst.rendered() == src.rendered(),
                    _ => false,
                };
            if self_move {
                copies_removed += 1;
            }
            !self_move
        });
    }
    crate::optimizer::stats::count_registers_coalesced(copies_removed);
    crate::optimizer::stats::count_spill_slots_shared(slots_reused);
    crate::optimizer::stats::count_values_rematerialized(remat_count);
    crate::optimizer::stats::count_live_ranges_split(split_ranges);
    let _ = coalesced;

    let total_slot_count = spill_slot_count + max_evictions;

    let mut extra_callee_saved: Vec<String> = Vec::new();
    for &(phys, _index) in &every_home {
        if model.is_callee_saved(phys) && !extra_callee_saved.iter().any(|s| s == phys) {
            extra_callee_saved.push(phys.to_string());
        }
    }
    // Callee-saved registers commandeered only as genuinely-free reload scratch are
    // never colored homes, so they are absent from `assignment` — merge them in
    // so `finalize_frame` saves/restores them too (bug-54). The same generic
    // `run` colors both the Int and Fp classes, so this covers `x20`–`x28` and
    // `d8`–`d15` alike.
    for phys in &scratch_callee_saved {
        if !extra_callee_saved.iter().any(|s| s == phys) {
            extra_callee_saved.push(phys.clone());
        }
    }
    extra_callee_saved.sort();

    // Invariant (bug-54): every callee-saved register generated code *keeps*
    // written — a colored home or a genuinely-free reload scratch — is in the
    // frame's save set. Eviction victims are excluded: they are bracketed by a
    // save/reload around their single use, so the function does not leave them
    // modified.
    #[cfg(debug_assertions)]
    {
        for phys in every_home
            .iter()
            .map(|(name, _index)| *name)
            .filter(|name| model.is_callee_saved(name))
            .chain(scratch_callee_saved.iter().map(String::as_str))
        {
            debug_assert!(
                extra_callee_saved.iter().any(|s| s == phys),
                "bug-54: callee-saved register {phys} written by generated code \
                 (colored home or reload scratch) is missing from the frame save set",
            );
        }
    }

    RunResult {
        instructions: out,
        spill_slot_count: total_slot_count,
        extra_callee_saved,
        error: alloc_error,
    }
}

/// Per-instruction physical-occupancy mask over the colored virtual registers,
/// built by an endpoint sweep (plan-78-C Phase 1).
///
/// A colored vreg with physical index `pi` and (over-approximated) live interval
/// `[s, e]` contributes bit `pi` to every instruction in `s..=e`. The naive
/// construction ORs that bit across the whole interval for every vreg, which is
/// O(vregs × interval) — on the ~135k-vreg inlined regex body with wide
/// intervals that is the spill-path quadratic. Instead, emit `+pi` at `s` and
/// `-pi` at `e + 1`, then fold across instruction indices maintaining a running
/// mask; the result is **bit-identical** to the naive double loop (the
/// `sweep_equals_naive` property test proves it) but runs in
/// O(instructions + Σ interval endpoints).
///
/// Bits overlap — several vregs can share one physical index across nested
/// intervals — so a per-index reference count is kept, and bit `pi` clears only
/// when the *last* vreg occupying it leaves. Starts from `phys_busy_at` (the
/// hardcoded-physical occupancy), exactly like the naive form.
fn colored_mask_sweep(
    phys_busy_at: &[u64],
    vreg_interval: &analysis::U32Map<(usize, usize)>,
    assigned_index: &analysis::U32Map<u32>,
) -> Vec<u64> {
    let n = phys_busy_at.len();
    let mut masks = phys_busy_at.to_vec();
    // `deltas[i]` = list of (physical index, +1 start / -1 end) events at `i`.
    // Sized `n + 1` so an interval ending at the last instruction can post its
    // `-1` at `e + 1 == n` without bounds trouble (that slot is never masked).
    let mut deltas: Vec<Vec<(u32, i32)>> = vec![Vec::new(); n + 1];
    for (v, &(s, e)) in vreg_interval {
        if let Some(&pi) = assigned_index.get(v) {
            deltas[s].push((pi, 1));
            deltas[e + 1].push((pi, -1));
        }
    }
    // Reference count per physical index; a bit is live in `running` while its
    // count is > 0. Physical indices span 0..=63 (one machine word).
    let mut count = [0i32; 64];
    let mut running = 0u64;
    for i in 0..n {
        for &(pi, delta) in &deltas[i] {
            let slot = &mut count[pi as usize];
            let was_zero = *slot == 0;
            *slot += delta;
            if was_zero && *slot > 0 {
                running |= 1u64 << pi;
            } else if !was_zero && *slot == 0 {
                running &= !(1u64 << pi);
            }
        }
        masks[i] |= running;
    }
    masks
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
        // A typed physical of this class carries its index inline (plan-82-B); any
        // other operand (including a `Phys` of the other class) takes the `Raw`
        // string path, byte-identical to the pre-typing behavior.
        let p = match value {
            Operand::Phys { class, index, .. } if *class == class_model.class => Some(*index),
            _ => (class_model.physical_index)(&value.rendered()),
        };
        if let Some(p) = p {
            mask |= 1u64 << p;
        }
    }
    mask
}

/// Rewrite `instruction`'s this-class virtual-register operands **in place** —
/// colored vregs to their physical, spilled vregs to their per-instruction
/// scratch — and return the moved-through instruction. plan-84 Phase 3: the
/// instruction is consumed by value and only the vreg-bearing operands are
/// mutated, so the whole `fields` Vec is carried, not `clone`d (the two regalloc
/// passes previously re-cloned every instruction's fields Vec).
fn substitute(
    mut instruction: CodeInstruction,
    class: RegClass,
    assignment: &analysis::U32Map<(&'static str, u32)>,
    scratch_for: &analysis::U32Map<(&'static str, u32)>,
    class_model: &ClassModel,
) -> CodeInstruction {
    for (_field, value) in instruction.fields.iter_mut() {
        // Detect this class's virtual register: a typed `VReg` of the matching
        // class yields its id with no string work (plan-82-B); a `Raw("%vN")`
        // (pre-plan-82-C stream) takes the parse fallback. A `VReg`/`Phys` of the
        // other class is not this class's vreg — skip it, as the old parse did.
        let vreg = match value {
            Operand::VReg { class: c, id } if *c == class => Some(*id),
            Operand::VReg { .. } | Operand::Phys { .. } => None,
            _ => (class_model.parse_vreg)(&value.rendered()),
        };
        if let Some(v) = vreg {
            // Write a typed `Operand::Phys` carrying the static name (no heap box)
            // and the class index the encoder reads directly (plan-82-D). Its
            // `rendered()` equals the old physical-name string, so downstream
            // consumers and every dump are byte-identical (plan-82-A round trip).
            if let Some(&(name, index)) = assignment.get(&v) {
                *value = Operand::phys(class, index, name);
            } else if let Some(&(name, index)) = scratch_for.get(&v) {
                *value = Operand::phys(class, index, name);
            }
        }
    }
    instruction
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch::aarch64::regmodel::Aarch64RegisterModel;
    use crate::optimizer::{with_opt_level, OptLevel};
    use crate::target::shared::regmodel::RegClass;

    fn ci(op: &str, fields: &[(&'static str, &str)]) -> CodeInstruction {
        let mut inst = CodeInstruction::new(op);
        for (k, v) in fields {
            inst = inst.field(k, v);
        }
        inst
    }

    /// Spill-code optimization deletes a reload only while the value is still
    /// resident. A call destroys the caller-saved registers holding claims, so
    /// the reload after it MUST survive — keying invalidation on "does this
    /// instruction define a register?" instead (a `bl` names no `dst`) left
    /// stale claims and segfaulted 600+ fixtures' error paths at `-O2`.
    #[test]
    fn a_call_invalidates_resident_spill_claims() {
        // Already-colored stream: store x9 to a slot, call, then reload it.
        let stream = vec![
            ci(
                "str_u64",
                &[("src", "x9"), ("base", "sp"), ("offset", "16")],
            ),
            ci("bl", &[("target", "_mfb_arena_alloc")]),
            ci(
                "ldr_u64",
                &[("dst", "x9"), ("base", "sp"), ("offset", "16")],
            ),
            ci("ret", &[]),
        ];
        let model = Aarch64RegisterModel;
        let class_model = crate::codegen::engine::regalloc::class_models(&model).0;
        let result = with_opt_level(OptLevel(2), || {
            run(stream, &model, RegClass::Int, &class_model, 0, 8, &[])
        });
        assert!(
            result
                .instructions
                .iter()
                .any(|instruction| instruction.op == CodeOp::LdrU64),
            "the reload after a call must survive: the call clobbered x9"
        );
    }

    /// With nothing in between, the reload of a just-stored register is
    /// genuinely redundant and goes.
    #[test]
    fn an_immediate_reload_of_a_just_stored_register_is_removed() {
        let stream = vec![
            ci(
                "str_u64",
                &[("src", "x9"), ("base", "sp"), ("offset", "16")],
            ),
            ci(
                "ldr_u64",
                &[("dst", "x9"), ("base", "sp"), ("offset", "16")],
            ),
            ci("ret", &[]),
        ];
        let model = Aarch64RegisterModel;
        let class_model = crate::codegen::engine::regalloc::class_models(&model).0;
        let result = with_opt_level(OptLevel(2), || {
            run(stream, &model, RegClass::Int, &class_model, 0, 8, &[])
        });
        assert!(
            !result
                .instructions
                .iter()
                .any(|instruction| instruction.op == CodeOp::LdrU64),
            "x9 already holds the slot's value"
        );
    }

    /// An instruction that redefines the holding register ends its claim.
    #[test]
    fn redefining_the_register_keeps_the_reload() {
        let stream = vec![
            ci(
                "str_u64",
                &[("src", "x9"), ("base", "sp"), ("offset", "16")],
            ),
            ci("add_imm", &[("dst", "x9"), ("src", "x9"), ("imm", "1")]),
            ci(
                "ldr_u64",
                &[("dst", "x9"), ("base", "sp"), ("offset", "16")],
            ),
            ci("ret", &[]),
        ];
        let model = Aarch64RegisterModel;
        let class_model = crate::codegen::engine::regalloc::class_models(&model).0;
        let result = with_opt_level(OptLevel(2), || {
            run(stream, &model, RegClass::Int, &class_model, 0, 8, &[])
        });
        assert!(
            result
                .instructions
                .iter()
                .any(|instruction| instruction.op == CodeOp::LdrU64),
            "x9 no longer holds the slot's value"
        );
    }

    /// The naive O(vregs × interval) construction the sweep replaces — the oracle
    /// for the property test: for each colored vreg, OR its physical-index bit
    /// across every instruction in its live interval, starting from the
    /// hardcoded-physical occupancy.
    fn colored_mask_naive(
        phys_busy_at: &[u64],
        vreg_interval: &analysis::U32Map<(usize, usize)>,
        assigned_index: &analysis::U32Map<u32>,
    ) -> Vec<u64> {
        let mut masks = phys_busy_at.to_vec();
        for (v, &(s, e)) in vreg_interval {
            if let Some(&pi) = assigned_index.get(v) {
                for m in masks.iter_mut().take(e + 1).skip(s) {
                    *m |= 1u64 << pi;
                }
            }
        }
        masks
    }

    /// The endpoint sweep must produce a bit-identical mask to the naive double
    /// loop over randomized intervals — dense physical indices (0..16) and many
    /// vregs over a short instruction range force heavy interval overlap, which
    /// exercises the per-index reference count (a bit stays set while any vreg
    /// still occupies it). Deterministic: a fixed-seed LCG, since `rand`/`Date`
    /// are unavailable and non-deterministic here.
    #[test]
    fn sweep_equals_naive_over_randomized_intervals() {
        let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 33) as u32
        };
        for trial in 0..500 {
            let n = 1 + next() as usize % 40;
            let phys_busy_at: Vec<u64> = (0..n).map(|_| (next() as u64) & 0xFF).collect();
            let mut vreg_interval: analysis::U32Map<(usize, usize)> = analysis::U32Map::default();
            let mut assigned_index: analysis::U32Map<u32> = analysis::U32Map::default();
            let vcount = next() as usize % 30;
            for v in 0..vcount as u32 {
                let a = next() as usize % n;
                let b = next() as usize % n;
                let (s, e) = if a <= b { (a, b) } else { (b, a) };
                vreg_interval.insert(v, (s, e));
                // A few vregs are left uncolored (absent from assigned_index), like
                // spilled ones; the rest share a dense 0..16 physical-index space.
                if next() % 8 != 0 {
                    assigned_index.insert(v, next() % 16);
                }
            }
            let expected = colored_mask_naive(&phys_busy_at, &vreg_interval, &assigned_index);
            let got = colored_mask_sweep(&phys_busy_at, &vreg_interval, &assigned_index);
            assert_eq!(got, expected, "trial {trial}: sweep mask != naive mask");
        }
    }

    /// A hand-built case that pins the overlap semantics: two vregs on the same
    /// physical index with nested intervals — the bit must stay set across the
    /// whole union and clear only after the outer one ends.
    #[test]
    fn overlapping_same_index_clears_only_after_last() {
        let phys_busy_at = vec![0u64; 6];
        let mut vreg_interval = analysis::U32Map::default();
        vreg_interval.insert(1u32, (0usize, 4usize)); // outer
        vreg_interval.insert(2u32, (1usize, 2usize)); // nested, same index
        let mut assigned_index = analysis::U32Map::default();
        assigned_index.insert(1u32, 3u32);
        assigned_index.insert(2u32, 3u32);
        let masks = colored_mask_sweep(&phys_busy_at, &vreg_interval, &assigned_index);
        let bit = 1u64 << 3;
        assert_eq!(masks, vec![bit, bit, bit, bit, bit, 0]);
    }
}
