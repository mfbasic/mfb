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

use crate::target::shared::regmodel::{RegClass, RegisterModel};

use super::super::types::CodeInstruction;
use super::super::Operand;
use super::analysis::{self, physical_busy, ClassModel, RegRef};

pub(super) struct RunResult {
    pub(super) instructions: Vec<CodeInstruction>,
    pub(super) spill_slot_count: usize,
    pub(super) extra_callee_saved: Vec<String>,
    /// Set when coloring could not represent an instruction (it names more
    /// simultaneously-live registers than the target's allocatable pool holds),
    /// so no valid allocation exists. `allocate` surfaces this as a clear
    /// compile-time failure rather than the raw `.expect` ICE it replaced
    /// (bug-127.2). `None` on success.
    pub(super) error: Option<String>,
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
    let live = analysis::analyze(instructions, class_model, &effects);

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
        let choice = allocatable.iter().find(|&&(_, pi)| {
            (active_mask & (1u64 << pi)) == 0
                && (clobbered & (1u64 << pi)) == 0
                && !phys_busy_in(pi, s, e)
        });
        match choice {
            Some(&(name, pi)) => {
                assignment.insert(v, (name, pi));
                assigned_index.insert(v, pi);
                active.push((e, v, pi));
                active_mask |= 1u64 << pi;
            }
            None => spilled.push(v),
        }
    }

    // Assign a stack slot to each spilled vreg.
    let mut spill_slot: analysis::U32Map<usize> = analysis::U32Map::default();
    for &v in &spilled {
        let k = spill_slot.len();
        spill_slot.insert(v, spill_base_offset + k * slot_bytes);
    }
    let spill_slot_count = spill_slot.len();

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
    let mut out: Vec<CodeInstruction> = Vec::with_capacity(n);
    // Set if an instruction cannot be colored (more simultaneously-live registers
    // than the pool holds); surfaced by `allocate` (bug-127.2).
    let mut alloc_error: Option<String> = None;
    'rewrite: for (i, instruction) in instructions.iter().enumerate() {
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
            let mut occupied = occupied_at(i, &colored_mask_at, instruction, class_model);
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
            out.push(model.emit_reload(class, scratch_for[&v].0, spill_slot[&v]));
        }
        out.push(substitute(
            instruction,
            class,
            &assignment,
            &scratch_for,
            class_model,
        ));
        for &v in &def_spilled {
            out.push(model.emit_spill(class, scratch_for[&v].0, spill_slot[&v]));
        }
        for (victim, slot) in evictions.iter().rev() {
            out.push(model.emit_reload(class, victim, evict_base + slot * slot_bytes));
        }
    }
    let total_slot_count = spill_slot_count + max_evictions;

    let mut extra_callee_saved: Vec<String> = Vec::new();
    for &(phys, _index) in assignment.values() {
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
        for phys in assignment
            .values()
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

/// Produce a copy of `instruction` with this class's virtual-register operands
/// replaced: colored vregs by their physical, spilled vregs by their
/// per-instruction scratch.
fn substitute(
    instruction: &CodeInstruction,
    class: RegClass,
    assignment: &analysis::U32Map<(&'static str, u32)>,
    scratch_for: &analysis::U32Map<(&'static str, u32)>,
    class_model: &ClassModel,
) -> CodeInstruction {
    let mut copy = CodeInstruction {
        op: instruction.op,
        fields: instruction.fields.clone(),
    };
    for (_field, value) in copy.fields.iter_mut() {
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
    copy
}

#[cfg(test)]
mod tests {
    use super::*;

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
