//! Stack-slot value availability — the fact base the two memory rows share:
//! store-to-load forwarding (`opt2::stldfwd`) and redundant load elimination
//! (`opt2::rle`). Both ask the same question at a load: *is the value in this
//! slot already sitting in a register?* — differing only in where the
//! available value came from (a store, or an earlier load).
//!
//! A forward must-dataflow over the allocator's own CFG, the mirror image of
//! the backward one in `opt2::dse`: state is a map slot → the SSA value (and
//! the register spelling) the slot provably holds, the meet over predecessors
//! is agreement (a slot survives a join only when every predecessor leaves the
//! *same* value in it), and the entry state is empty. Everything not
//! explicitly modeled clears the whole state, so mis-modeling can only lose a
//! rewrite, never invent one:
//!
//! - `str [sp,#off]` of an int vreg makes `off` hold that register's SSA
//!   value; a partially-overlapping slot (`|other - off| < 8`) is untracked
//!   (no byte-granular credit).
//! - `ldr [sp,#off]` **reads** the slot: when it is available the load is a
//!   rewrite candidate; either way `off` afterwards holds the loaded
//!   register's value.
//! - pure ALU, compares, labels, and branch terminators pass through (no
//!   memory effect; the block meet handles their paths).
//! - anything else — other loads/stores, calls, FP ops, unknown ops — clears
//!   everything, because the frame can be reached through a recomputed
//!   address and every such path starts with an op outside the neutral set.
//!
//! Availability proves the *value* is current; whether the holder *register*
//! still holds it at the load is the separate question the consumers settle
//! with the same single-definition rule GVN uses (`def_count == 1`).
//!
//! The same fixpoint answers one more question at no extra cost, for the
//! **Store PRE / Load PRE** row: a load at the top of a join whose slot is
//! available on every incoming edge *but one*. Placing that load at the end of
//! the odd edge's predecessor makes it available on all of them, and the
//! join's own load then becomes a copy — the same copy the fully-available
//! half produces, which copy propagation bypasses and dead-code elimination
//! removes. The net effect is that the load moves off the path that already
//! had the value. The per-predecessor exit states the fixpoint already
//! computed are exactly the evidence that question needs, which is why it is
//! answered here rather than by a second traversal.
//!
//! Shape matters here: each instruction's memory effect is classified **once**
//! (no operand re-parsing per round) and each block's exit state is cached, so
//! the fixpoint costs one transfer per instruction per round rather than one
//! per predecessor edge. The first version re-ran every predecessor's whole
//! block inside the meet and re-parsed offsets each time, which cost minutes
//! on the giant generated functions.

use std::collections::HashMap;

use crate::arch::ops::CodeOp;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::regalloc::analysis::{
    classify_ref, effect, is_block_terminator, Block, ClassModel, RegRef,
};
use crate::codegen::engine::types::CodeInstruction;

use super::mark::removable_op;
use super::ssa::{Ssa, Var};

/// Where an available slot value came from — which row may claim the rewrite.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Origin {
    /// A `str` put it there: forwarding the load is store-to-load forwarding.
    Store,
    /// An earlier `ldr` read it: forwarding is redundant-load elimination.
    Load,
}

/// One available slot value: the SSA value the slot holds, the register
/// holding it, and how it got there.
#[derive(Clone)]
pub(crate) struct Available {
    pub(crate) value: usize,
    pub(crate) holder: Operand,
    pub(crate) holder_var: Var,
    pub(crate) origin: Origin,
}

/// A load this analysis proved redundant: rewrite `instructions[inst]` into a
/// copy of `available.holder`.
pub(crate) struct Forwardable {
    pub(crate) inst: usize,
    pub(crate) available: Available,
}

/// A load whose slot value is already in `holder` on every path into its
/// block **except one** — the Store PRE / Load PRE row's candidate. Placing
/// the same load at the end of `gap` makes it available on that path too,
/// after which the load itself becomes a copy.
pub(crate) struct PartialLoad {
    pub(crate) inst: usize,
    /// The one predecessor block that lacks the value.
    pub(crate) gap: usize,
    /// The register the other predecessors already leave it in.
    pub(crate) holder: Operand,
    /// The slot to read into `holder` on the gap path.
    pub(crate) offset: i64,
}

/// One instruction's memory effect, classified once up front.
enum MemEffect {
    /// A full-slot `sp` store; `held` is the stored register when trackable.
    Store {
        offset: i64,
        held: Option<(usize, Operand, Var)>,
    },
    /// A full-slot `sp` load; `dst` is the loaded register when trackable.
    Load {
        offset: i64,
        dst: Option<(usize, Operand, Var)>,
    },
    /// Provably memory-free: pure ALU, compares, labels, branch terminators.
    Neutral,
    /// Anything else — may touch the frame: forget every slot.
    Barrier,
}

/// Availability state: slot offset → value, kept as a **sorted vector**
/// rather than a map. A block's live slot set is tiny (single digits), and the
/// fixpoint clones this state per block per round — with a `HashMap` the
/// allocation and hashing dominated the whole `-O3` build.
type State = Vec<(i64, Available)>;

fn slot(state: &State, offset: i64) -> Option<&Available> {
    state
        .binary_search_by_key(&offset, |(other, _)| *other)
        .ok()
        .map(|index| &state[index].1)
}

fn put(state: &mut State, offset: i64, available: Available) {
    match state.binary_search_by_key(&offset, |(other, _)| *other) {
        Ok(index) => state[index].1 = available,
        Err(index) => state.insert(index, (offset, available)),
    }
}

/// Every load whose slot value is already in a register at that point, with
/// the holder's definition count checked (single-def holders only — a
/// multi-def register may hold something else by the time the load runs).
/// `blocks` must be `build_cfg(instructions)`.
pub(crate) fn forwardable_loads(
    instructions: &[CodeInstruction],
    blocks: &[Block],
    models: &(ClassModel, ClassModel),
    overlay: &Ssa,
) -> (Vec<Forwardable>, Vec<PartialLoad>) {
    let nb = blocks.len();
    if nb == 0 {
        return (Vec::new(), Vec::new());
    }
    let effects: Vec<MemEffect> = (0..instructions.len())
        .map(|i| classify(instructions, i, models, overlay))
        .collect();
    // No trackable load: nothing this analysis could ever forward, so skip
    // the fixpoint entirely.
    if !effects
        .iter()
        .any(|effect| matches!(effect, MemEffect::Load { dst: Some(_), .. }))
    {
        return (Vec::new(), Vec::new());
    }

    // Only slots some trackable load actually reads are worth carrying: a
    // store to a slot nothing reloads can never be forwarded, and on the
    // giant generated functions the untracked majority is what made the
    // per-block state (cloned at every visit) expensive.
    let mut loaded: Vec<i64> = effects
        .iter()
        .filter_map(|effect| match effect {
            MemEffect::Load {
                offset,
                dst: Some(_),
            } => Some(*offset),
            _ => None,
        })
        .collect();
    loaded.sort_unstable();
    loaded.dedup();
    let tracked = |offset: i64| loaded.binary_search(&offset).is_ok();

    // Single-definition int vregs: the only registers whose value at a later
    // point is provably still the one this analysis recorded.
    let mut def_count: HashMap<Var, u32> = HashMap::new();
    for instruction in instructions {
        for model in [&models.0, &models.1] {
            for def in effect(instruction, model).defs {
                if let RegRef::VReg(id) = def {
                    *def_count.entry((model.is_fp, id)).or_insert(0) += 1;
                }
            }
        }
    }

    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); nb];
    for (b, block) in blocks.iter().enumerate() {
        for &s in &block.succ {
            preds[s].push(b);
        }
    }

    // Forward fixpoint, worklist-driven in reverse postorder: a block is
    // re-processed only when a predecessor's exit actually changed. (Sweeping
    // every block for a fixed number of rounds instead was the whole cost of
    // this row on the giant generated functions.) States only ever lose slots
    // — the meet is agreement — so this terminates.
    let mut exit_state: Vec<Option<State>> = vec![None; nb];
    let order = reverse_postorder(blocks);
    let mut position = vec![usize::MAX; nb];
    for (index, &b) in order.iter().enumerate() {
        position[b] = index;
    }
    let mut queued = vec![false; nb];
    // A BTreeSet keyed by RPO position drains blocks in dataflow order, which
    // is what keeps the pass to roughly one sweep on acyclic regions.
    let mut work: std::collections::BTreeSet<usize> = order.iter().map(|&b| position[b]).collect();
    for &b in &order {
        queued[b] = true;
    }
    while let Some(&next) = work.iter().next() {
        work.remove(&next);
        let b = order[next];
        queued[b] = false;
        let Some(mut state) = entry_state(b, &preds[b], &exit_state) else {
            continue; // no predecessor facts yet
        };
        for i in blocks[b].start..blocks[b].end {
            transfer(&effects[i], &mut state, &tracked);
        }
        if exit_state[b].as_ref().is_some_and(|old| same(old, &state)) {
            continue;
        }
        exit_state[b] = Some(state);
        for &s in &blocks[b].succ {
            if position[s] != usize::MAX && !queued[s] {
                queued[s] = true;
                work.insert(position[s]);
            }
        }
    }

    // Collection walk: replay each block from its (now stable) entry state.
    let mut found = Vec::new();
    let mut partial = Vec::new();
    for (b, block) in blocks.iter().enumerate() {
        let Some(mut state) = entry_state(b, &preds[b], &exit_state) else {
            continue; // unreachable block: no facts, no rewrites
        };
        for i in block.start..block.end {
            if let MemEffect::Load {
                offset,
                dst: Some(_),
            } = effects[i]
            {
                if let Some(available) = slot(&state, offset) {
                    if def_count.get(&available.holder_var) == Some(&1) {
                        found.push(Forwardable {
                            inst: i,
                            available: available.clone(),
                        });
                    }
                } else if i <= block.start + 1 {
                    // Partially available: every predecessor but one already
                    // leaves the slot's value in the same register. Only a
                    // load at the very top of the block qualifies — further
                    // in, the block's own instructions may have changed the
                    // slot since entry, and the per-predecessor exit states no
                    // longer describe what is true at this point.
                    if let Some(candidate) =
                        partially_available(blocks, &preds[b], &exit_state, &def_count, i, offset)
                    {
                        partial.push(candidate);
                    }
                }
            }
            transfer(&effects[i], &mut state, &tracked);
        }
    }
    (found, partial)
}

/// The Store PRE / Load PRE test: all but one predecessor leave the slot's
/// value in the same single-definition register, and the odd one out reaches
/// this block unconditionally (so a load placed at its end runs exactly as
/// often as the edge into this block is taken).
fn partially_available(
    blocks: &[Block],
    preds: &[usize],
    exit_state: &[Option<State>],
    def_count: &HashMap<Var, u32>,
    inst: usize,
    offset: i64,
) -> Option<PartialLoad> {
    if preds.len() < 2 {
        return None;
    }
    let mut leader: Option<Available> = None;
    let mut gap: Option<usize> = None;
    for &p in preds {
        let exit = exit_state[p].as_ref()?;
        match slot(exit, offset) {
            Some(available) => match &leader {
                None => leader = Some(available.clone()),
                Some(current) => {
                    if current.value != available.value
                        || current.holder.rendered() != available.holder.rendered()
                    {
                        return None;
                    }
                }
            },
            None => {
                if gap.is_some() {
                    return None; // more than one gap: not a size-neutral fix
                }
                gap = Some(p);
            }
        }
    }
    let (leader, gap) = (leader?, gap?);
    if def_count.get(&leader.holder_var) != Some(&1) {
        return None;
    }
    // A predecessor that branches would run the inserted load on executions
    // that never reach this block.
    if blocks[gap].succ.len() != 1 {
        return None;
    }
    Some(PartialLoad {
        inst,
        gap,
        holder: leader.holder,
        offset,
    })
}

/// The block's entry state: empty for the entry block, otherwise the meet of
/// its predecessors' exits (agreement on the same SSA value). A predecessor
/// with no facts yet leaves the block undecided.
fn entry_state(b: usize, preds: &[usize], exit_state: &[Option<State>]) -> Option<State> {
    if b == 0 {
        return Some(State::new());
    }
    let mut merged: Option<State> = None;
    for &p in preds {
        let exit = exit_state[p].as_ref()?;
        merged = Some(match merged {
            None => exit.clone(),
            Some(current) => current
                .into_iter()
                .filter(|(offset, available)| {
                    slot(exit, *offset).is_some_and(|other| other.value == available.value)
                })
                .collect(),
        });
    }
    merged
}

/// Classify one instruction's memory effect (done once per stream).
fn classify(
    instructions: &[CodeInstruction],
    i: usize,
    models: &(ClassModel, ClassModel),
    overlay: &Ssa,
) -> MemEffect {
    let instruction = &instructions[i];
    match instruction.op {
        CodeOp::StrU64 if sp_based(instruction) => match numeric_offset(instruction) {
            Some(offset) => {
                let held = instruction.operand("src").and_then(|operand| {
                    match classify_ref(operand, &models.0)? {
                        RegRef::VReg(id) => {
                            let value = overlay.value_of_use(i, (false, id))?;
                            Some((value, operand.clone(), (false, id)))
                        }
                        RegRef::Phys(_) => None,
                    }
                });
                MemEffect::Store { offset, held }
            }
            None => MemEffect::Barrier,
        },
        CodeOp::LdrU64 if sp_based(instruction) => match numeric_offset(instruction) {
            Some(offset) => {
                let dst = instruction.operand("dst").and_then(|operand| {
                    match classify_ref(operand, &models.0)? {
                        RegRef::VReg(id) => {
                            let value = overlay.value_defined_at(i, (false, id))?;
                            Some((value, operand.clone(), (false, id)))
                        }
                        RegRef::Phys(_) => None,
                    }
                });
                MemEffect::Load { offset, dst }
            }
            None => MemEffect::Barrier,
        },
        CodeOp::Label => MemEffect::Neutral,
        op if removable_op(op)
            || is_block_terminator(op)
            || matches!(
                op,
                CodeOp::Adds | CodeOp::Subs | CodeOp::Cmp | CodeOp::CmpImm
            ) =>
        {
            MemEffect::Neutral
        }
        _ => MemEffect::Barrier,
    }
}

/// Advance the state past one instruction (the module docs' transfer rules).
fn transfer(effect: &MemEffect, state: &mut State, tracked: &impl Fn(i64) -> bool) {
    match effect {
        MemEffect::Store { offset, held } => {
            kill_overlapping(state, *offset);
            if let Some((value, holder, holder_var)) = held.as_ref().filter(|_| tracked(*offset)) {
                put(
                    state,
                    *offset,
                    Available {
                        value: *value,
                        holder: holder.clone(),
                        holder_var: *holder_var,
                        origin: Origin::Store,
                    },
                );
            }
        }
        MemEffect::Load { offset, dst } => {
            kill_overlapping(state, *offset);
            // After the load, the slot's value sits in the loaded register —
            // available to the *next* load of the same slot (the RLE case).
            if let Some((value, holder, holder_var)) = dst.as_ref().filter(|_| tracked(*offset)) {
                put(
                    state,
                    *offset,
                    Available {
                        value: *value,
                        holder: holder.clone(),
                        holder_var: *holder_var,
                        origin: Origin::Load,
                    },
                );
            }
        }
        MemEffect::Neutral => {}
        MemEffect::Barrier => state.clear(),
    }
}

/// Drop every slot whose 8 bytes overlap `offset` (the slot itself included —
/// the caller re-inserts it when the new value is trackable).
fn kill_overlapping(state: &mut State, offset: i64) {
    state.retain(|(other, _)| (other - offset).abs() >= 8);
}

fn same(a: &State, b: &State) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b.iter())
            .all(|((oa, va), (ob, vb))| oa == ob && va.value == vb.value)
}

/// Reverse postorder of the CFG from the entry block (unreachable blocks are
/// absent, and so are never processed).
fn reverse_postorder(blocks: &[Block]) -> Vec<usize> {
    let nb = blocks.len();
    let mut order = Vec::with_capacity(nb);
    let mut seen = vec![false; nb];
    let mut stack: Vec<(usize, usize)> = vec![(0, 0)];
    seen[0] = true;
    while let Some(&mut (b, ref mut next)) = stack.last_mut() {
        if *next < blocks[b].succ.len() {
            let s = blocks[b].succ[*next];
            *next += 1;
            if !seen[s] {
                seen[s] = true;
                stack.push((s, 0));
            }
        } else {
            order.push(b);
            stack.pop();
        }
    }
    order.reverse();
    order
}

fn sp_based(instruction: &CodeInstruction) -> bool {
    instruction
        .operand("base")
        .is_some_and(|base| base.rendered() == "sp")
}

fn numeric_offset(instruction: &CodeInstruction) -> Option<i64> {
    instruction.get("offset")?.parse().ok()
}
