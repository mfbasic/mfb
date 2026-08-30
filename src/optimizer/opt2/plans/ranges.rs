//! Integer range lattice over the SSA overlay — the fact base the
//! check-elision cluster of Level-3 Opt2 rows shares
//! (`planning/optimizations.md`): correlated value propagation,
//! overflow-check elimination, division / modulo-check elimination,
//! bounds-check elimination, redundant union-tag / error-tag check
//! elimination, range-check widening / narrowing, and dead error-handler
//! elimination. One engine, so their notions of "what is provably true about
//! this value here" cannot diverge.
//!
//! Two layers, because the two questions are different:
//!
//! 1. **Global ranges** — one signed interval per SSA value, computed by an
//!    optimistic worklist fixpoint with widening. This is flow-insensitive by
//!    construction: an SSA value has one definition, so its interval holds
//!    everywhere the value is live. A loop-carried phi widens to the full
//!    range almost immediately, which is exactly right — nothing about a
//!    counter is knowable from its definition alone.
//! 2. **Per-block refinements** — the interval a value is *additionally*
//!    known to lie in on the paths that reach a given block, taken from the
//!    dominating compare-and-branch edges. This is where a bounds or tag
//!    check becomes provable: `i < n` on the taken edge is what pins `i`'s
//!    upper end inside the loop body, and no amount of global reasoning can
//!    see it.
//!
//! Within a block the refinements are then pushed *forward* through the
//! block's own pure arithmetic, so a fact proven about `i` also constrains
//! `i + 1` and `i * 2`. That derivation is the "range-check widening /
//! narrowing" row: one dominating condition discharging the checks on several
//! derived indices.
//!
//! **Trap discipline is structural.** The transfer table covers only ops
//! whose value behavior is total and target-independent. The checked
//! arithmetic ops `adds`/`subs` are modeled *only* through the same
//! `checked_add`/`checked_sub` the interval arithmetic uses everywhere else:
//! if the true sum of the two input intervals is representable, the wrapped
//! result equals the true result and the interval is exact; if it is not, the
//! op contributes nothing. So the lattice can never claim to know the value
//! of an operation that may raise. Every unmodeled op yields the full range,
//! so mis-modeling can only lose a rewrite, never invent one.

use std::collections::HashMap;

use crate::arch::ops::CodeOp;
use crate::codegen::engine::regalloc::analysis::{
    classify_ref, is_use_field, Block, ClassModel, RegRef,
};
use crate::codegen::engine::types::CodeInstruction;

use super::mark::{conditional_terminator, flag_preserving, removable_op};
use super::ssa::{Ssa, ValueDef, ValueId};

/// A closed signed interval. `Integer` is MFB's 64-bit signed type, so one
/// `i64` pair describes every integer value the lattice reasons about.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Range {
    pub(crate) lo: i64,
    pub(crate) hi: i64,
}

impl Range {
    /// Nothing is known.
    pub(crate) const FULL: Range = Range {
        lo: i64::MIN,
        hi: i64::MAX,
    };

    /// Exactly one value.
    pub(crate) fn exact(value: i64) -> Range {
        Range {
            lo: value,
            hi: value,
        }
    }

    pub(crate) fn is_full(&self) -> bool {
        self.lo == i64::MIN && self.hi == i64::MAX
    }

    /// The single value this range pins down, when it pins one down.
    pub(crate) fn singleton(&self) -> Option<i64> {
        (self.lo == self.hi).then_some(self.lo)
    }

    /// The union hull — what a phi knows from its arguments.
    fn hull(self, other: Range) -> Range {
        Range {
            lo: self.lo.min(other.lo),
            hi: self.hi.max(other.hi),
        }
    }

    /// The intersection — combining two independent facts about one value.
    ///
    /// An *empty* intersection means the block the fact was derived for is
    /// unreachable. Rather than hand a caller an inverted `lo > hi` it could
    /// misread as a proof of anything, the original is kept: a superset is
    /// always sound, and unreachable blocks are the unreachable-code row's
    /// business, not this one's.
    pub(crate) fn meet(self, other: Range) -> Range {
        let (lo, hi) = (self.lo.max(other.lo), self.hi.min(other.hi));
        if lo > hi {
            self
        } else {
            Range { lo, hi }
        }
    }

    /// Whether every value in the range is non-negative — the precondition
    /// for reading an unsigned comparison as a signed one.
    pub(crate) fn non_negative(&self) -> bool {
        self.lo >= 0
    }
}

/// Interval addition. An endpoint that would overflow makes the whole result
/// unknown — never a wrapped bound, which would be a false fact.
fn range_add(a: Range, b: Range) -> Range {
    match (a.lo.checked_add(b.lo), a.hi.checked_add(b.hi)) {
        (Some(lo), Some(hi)) => Range { lo, hi },
        _ => Range::FULL,
    }
}

fn range_sub(a: Range, b: Range) -> Range {
    match (a.lo.checked_sub(b.hi), a.hi.checked_sub(b.lo)) {
        (Some(lo), Some(hi)) => Range { lo, hi },
        _ => Range::FULL,
    }
}

/// Interval multiplication. A product over a box attains its extremes at the
/// corners, so four `checked_mul`s decide it.
fn range_mul(a: Range, b: Range) -> Range {
    let (mut lo, mut hi) = (i64::MAX, i64::MIN);
    for x in [a.lo, a.hi] {
        for y in [b.lo, b.hi] {
            match x.checked_mul(y) {
                Some(product) => {
                    lo = lo.min(product);
                    hi = hi.max(product);
                }
                None => return Range::FULL,
            }
        }
    }
    Range { lo, hi }
}

/// The range facts for one function's stream.
pub(crate) struct Ranges {
    /// One interval per SSA value, valid everywhere the value is live.
    global: Vec<Range>,
    /// Per block, the extra facts that hold on the paths reaching it, sorted
    /// by value id so a lookup is a binary search.
    per_block: Vec<Vec<(ValueId, Range)>>,
    /// Per block, the value ids whose fact was *derived* — pushed forward
    /// through arithmetic from another value's fact rather than read straight
    /// off a dominating compare. This is what the range-check widening /
    /// narrowing row counts.
    derived: Vec<Vec<ValueId>>,
}

impl Ranges {
    /// The interval `value` is known to lie in on the paths reaching `block`.
    pub(crate) fn at(&self, block: usize, value: ValueId) -> Range {
        let global = self.global.get(value).copied().unwrap_or(Range::FULL);
        let Some(facts) = self.per_block.get(block) else {
            return global;
        };
        match facts.binary_search_by_key(&value, |(id, _)| *id) {
            Ok(index) => global.meet(facts[index].1),
            Err(_) => global,
        }
    }

    /// Whether `block`'s fact about `value` was derived through arithmetic
    /// rather than read straight off a dominating comparison.
    pub(crate) fn is_derived(&self, block: usize, value: ValueId) -> bool {
        self.derived
            .get(block)
            .is_some_and(|ids| ids.binary_search(&value).is_ok())
    }
}

/// A block's flag-setting compare, resolved to SSA values (or literals) — the
/// fact a conditional terminator turns into an edge condition.
pub(crate) struct Compare {
    pub(crate) lhs: Operandish,
    pub(crate) rhs: Operandish,
}

/// One side of a compare: a tracked SSA value, a literal, or something this
/// pass does not model.
#[derive(Clone, Copy)]
pub(crate) enum Operandish {
    Value(ValueId),
    Literal(i64),
    Opaque,
}

/// The compare a block's conditional terminator branches on, when this pass
/// models it: the last flag-setting instruction before the terminator must be
/// a `cmp`/`cmp_imm` with nothing flag-touching in between.
///
/// `pub(crate)` because the check-elision rows need to identify the compare
/// itself (to discharge it), not only the fact it produces.
pub(crate) fn block_compare(
    instructions: &[CodeInstruction],
    models: &(ClassModel, ClassModel),
    overlay: &Ssa,
    block: &Block,
) -> Option<Compare> {
    let terminator = block.end - 1;
    if !conditional_terminator(instructions[terminator].op) {
        return None;
    }
    // Walk back to the flag setter. Only instructions that write no flags on
    // *any* backend may sit between it and the branch — `flag_preserving`,
    // not the pure-ALU whitelist, because on x86-64 the pure ALU ops all
    // write EFLAGS. In practice nothing sits between them at all: the MIR
    // layer fuses a setter with the branch that reads it, and selection
    // re-emits the pair adjacently.
    let mut i = terminator;
    while i > block.start {
        i -= 1;
        match instructions[i].op {
            CodeOp::Cmp | CodeOp::CmpImm => {
                return Some(Compare {
                    lhs: side(instructions, models, overlay, i, "lhs"),
                    rhs: side(instructions, models, overlay, i, "rhs"),
                })
            }
            _ if flag_preserving(&instructions[i]) => continue,
            _ => return None,
        }
    }
    None
}

/// Resolve one operand of an instruction to a tracked value or a literal.
fn side(
    instructions: &[CodeInstruction],
    models: &(ClassModel, ClassModel),
    overlay: &Ssa,
    i: usize,
    field: &str,
) -> Operandish {
    let Some(operand) = instructions[i].operand(field) else {
        return Operandish::Opaque;
    };
    match classify_ref(operand, &models.0) {
        Some(RegRef::VReg(id)) => match overlay.value_of_use(i, (false, id)) {
            Some(value) => Operandish::Value(value),
            None => Operandish::Opaque,
        },
        Some(RegRef::Phys(_)) => Operandish::Opaque,
        None => match literal(&operand.rendered()) {
            Some(value) => Operandish::Literal(value),
            None => Operandish::Opaque,
        },
    }
}

/// A literal operand's signed value, in the folder's spelling rules (an
/// immediate is written either as a signed decimal or as its unsigned 64-bit
/// pattern).
pub(crate) fn literal(text: &str) -> Option<i64> {
    text.parse::<i64>()
        .ok()
        .or_else(|| text.parse::<u64>().ok().map(|bits| bits as i64))
}

/// How many rounds a value may be re-evaluated before its interval is widened
/// to the full range. Loop-carried phis are the reason: without widening they
/// would crawl outward one iteration at a time.
const WIDEN_ROUNDS: u32 = 3;

/// The most facts one block carries. Refinement maps are inherited by
/// successors, so an unbounded map would make the walk quadratic on the giant
/// generated functions; the cap keeps it linear and only ever loses a fact.
const MAX_FACTS: usize = 64;

/// Build the range facts for one function. `blocks` must be
/// `analysis::build_cfg(instructions)` and `overlay` the SSA built over it.
pub(crate) fn analyze(
    instructions: &[CodeInstruction],
    models: &(ClassModel, ClassModel),
    overlay: &Ssa,
    blocks: &[Block],
) -> Ranges {
    let global = global_ranges(instructions, models, overlay);
    let (per_block, derived) = block_facts(instructions, models, overlay, blocks, &global);
    Ranges {
        global,
        per_block,
        derived,
    }
}

/// Layer 1: one interval per SSA value, by optimistic worklist fixpoint.
fn global_ranges(
    instructions: &[CodeInstruction],
    models: &(ClassModel, ClassModel),
    overlay: &Ssa,
) -> Vec<Range> {
    let count = overlay.values.len();
    // `None` is the optimistic bottom ("no evidence yet"), which is what lets
    // a loop-carried phi start from its initial value instead of the full
    // range. Widening turns the remaining growth into `FULL` in bounded time.
    let mut range: Vec<Option<Range>> = vec![None; count];
    let mut visits: Vec<u32> = vec![0; count];

    // Consumer edges, so a refined value re-evaluates exactly its users — the
    // same dependency graph the known-bits lattice walks.
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); count];
    for (vid, def) in overlay.values.iter().enumerate() {
        match def {
            ValueDef::Inst(i) => {
                for (name, operand) in &instructions[*i].fields {
                    if !is_use_field(name) {
                        continue;
                    }
                    if let Some(RegRef::VReg(id)) = classify_ref(operand, &models.0) {
                        if let Some(input) = overlay.value_of_use(*i, (false, id)) {
                            dependents[input].push(vid);
                        }
                    }
                }
            }
            ValueDef::Phi { args, .. } => {
                for &(_, arg) in args {
                    dependents[arg].push(vid);
                }
            }
            ValueDef::Entry => {}
        }
    }

    let mut work: Vec<usize> = (0..count).collect();
    let mut budget = count.saturating_mul(8) + 1024;
    while let Some(vid) = work.pop() {
        if budget == 0 {
            break;
        }
        budget -= 1;
        let next = match &overlay.values[vid] {
            // A live-in, or a value with no definition on the dominator path:
            // opaque by construction.
            ValueDef::Entry => Range::FULL,
            ValueDef::Inst(i) => match transfer(instructions, models, overlay, &range, *i) {
                Some(computed) => computed,
                // An input has no evidence yet. Evaluating now would pin this
                // value at the full range *permanently* — the join below only
                // ever hulls outward — so defer instead; the dependency edge
                // from that input re-queues this value once it settles.
                None => continue,
            },
            ValueDef::Phi { args, .. } => {
                let mut hull: Option<Range> = None;
                for &(_, arg) in args {
                    let Some(argument) = range[arg] else { continue };
                    hull = Some(match hull {
                        Some(current) => current.hull(argument),
                        None => argument,
                    });
                }
                match hull {
                    Some(hull) => hull,
                    // Every argument is still bottom: stay bottom.
                    None => continue,
                }
            }
        };
        let next = match range[vid] {
            None => next,
            Some(current) if current == next => continue,
            Some(current) => {
                visits[vid] += 1;
                if visits[vid] > WIDEN_ROUNDS {
                    Range::FULL
                } else {
                    current.hull(next)
                }
            }
        };
        if range[vid] == Some(next) {
            continue;
        }
        range[vid] = Some(next);
        for &dependent in &dependents[vid] {
            work.push(dependent);
        }
    }
    range
        .into_iter()
        .map(|value| value.unwrap_or(Range::FULL))
        .collect()
}

/// The interval an instruction's destination holds, from its inputs'.
///
/// `None` means an input is still bottom — no evidence yet. The caller must
/// defer rather than substitute the full range: the fixpoint's join only
/// widens, so one premature "unknown" would pin the value there forever.
fn transfer(
    instructions: &[CodeInstruction],
    models: &(ClassModel, ClassModel),
    overlay: &Ssa,
    range: &[Option<Range>],
    i: usize,
) -> Option<Range> {
    let instruction = &instructions[i];
    let bottom = std::cell::Cell::new(false);
    let operand = |field: &str| -> Range {
        let Some(operand) = instruction.operand(field) else {
            return Range::FULL;
        };
        match classify_ref(operand, &models.0) {
            Some(RegRef::VReg(id)) => match overlay.value_of_use(i, (false, id)) {
                Some(value) => match range[value] {
                    Some(known) => known,
                    None => {
                        bottom.set(true);
                        Range::FULL
                    }
                },
                None => Range::FULL,
            },
            Some(RegRef::Phys(_)) => Range::FULL,
            None => match literal(&operand.rendered()) {
                Some(value) => Range::exact(value),
                None => Range::FULL,
            },
        }
    };
    let computed = apply(instruction, &operand);
    (!bottom.get()).then_some(computed)
}

/// The shared op semantics, over any way of resolving an operand's range.
/// Both layers use it: the global fixpoint over lattice values, and the
/// per-block forward derivation over refined facts.
fn apply(instruction: &CodeInstruction, operand: &dyn Fn(&str) -> Range) -> Range {
    match instruction.op {
        CodeOp::MovImm => {
            if instruction.get("type").as_deref()
                != Some(crate::target::shared::abi::IMMEDIATE_CLASS_INTEGER)
            {
                return Range::FULL;
            }
            match instruction.get("value").as_deref().and_then(literal) {
                Some(value) => Range::exact(value),
                None => Range::FULL,
            }
        }
        CodeOp::Mov => operand("src"),
        // `adds`/`subs` are MFB's *checked* add and subtract. Their interval
        // is the true one exactly when the true one is representable — which
        // is what `range_add`/`range_sub` already test, returning the full
        // range otherwise. So a possibly-trapping op is never claimed to have
        // a known value, and no row built on these facts can rewrite one.
        CodeOp::Add | CodeOp::Adds => range_add(operand("lhs"), operand("rhs")),
        CodeOp::Sub | CodeOp::Subs => range_sub(operand("lhs"), operand("rhs")),
        CodeOp::AddImm => range_add(operand("src"), operand("imm")),
        CodeOp::SubImm => range_sub(operand("src"), operand("imm")),
        CodeOp::Mul => range_mul(operand("lhs"), operand("rhs")),
        CodeOp::And => {
            // A non-negative mask clears the sign bit, so the result is in
            // `[0, mask]` whatever the other side holds.
            let (lhs, rhs) = (operand("lhs"), operand("rhs"));
            if let Some(mask) = rhs.singleton().filter(|mask| *mask >= 0) {
                Range { lo: 0, hi: mask }
            } else if let Some(mask) = lhs.singleton().filter(|mask| *mask >= 0) {
                Range { lo: 0, hi: mask }
            } else if lhs.non_negative() {
                Range { lo: 0, hi: lhs.hi }
            } else if rhs.non_negative() {
                Range { lo: 0, hi: rhs.hi }
            } else {
                Range::FULL
            }
        }
        CodeOp::Orr | CodeOp::Eor => {
            let (lhs, rhs) = (operand("lhs"), operand("rhs"));
            if lhs.non_negative() && rhs.non_negative() {
                // Both sign bits are clear, so the result's is too.
                Range {
                    lo: 0,
                    hi: i64::MAX,
                }
            } else {
                Range::FULL
            }
        }
        CodeOp::LsrImm => match shift_amount(instruction) {
            Some(0) => operand("src"),
            // A logical right shift by one or more clears the sign bit.
            Some(amount) => Range {
                lo: 0,
                hi: (u64::MAX >> amount) as i64,
            },
            None => Range::FULL,
        },
        CodeOp::AsrImm => match shift_amount(instruction) {
            Some(amount) => {
                let source = operand("src");
                Range {
                    lo: source.lo >> amount,
                    hi: source.hi >> amount,
                }
            }
            None => Range::FULL,
        },
        CodeOp::LslImm => match shift_amount(instruction) {
            Some(amount) => {
                let source = operand("src");
                if source.lo >= 0 && source.hi <= (i64::MAX >> amount) {
                    Range {
                        lo: source.lo << amount,
                        hi: source.hi << amount,
                    }
                } else {
                    Range::FULL
                }
            }
            None => Range::FULL,
        },
        CodeOp::SDiv => {
            // Only a positive constant divisor: division is then monotone in
            // the dividend, and neither `x / 0` nor `MIN / -1` is in reach.
            let divisor = operand("rhs");
            match divisor.singleton().filter(|d| *d > 0) {
                Some(divisor) => {
                    let dividend = operand("lhs");
                    Range {
                        lo: dividend.lo / divisor,
                        hi: dividend.hi / divisor,
                    }
                }
                None => Range::FULL,
            }
        }
        _ => Range::FULL,
    }
}

/// The ops a per-block fact may be pushed forward through. The pure ALU
/// whitelist, plus the two *checked* ops — `apply` already refuses to claim a
/// value for those unless the true result is representable, so including them
/// costs no soundness and is what lets a fact about a loop counter reach the
/// `i + 1` the next check tests.
fn derivable_op(op: CodeOp) -> bool {
    removable_op(op) || matches!(op, CodeOp::Adds | CodeOp::Subs)
}

/// Whether the sum of two intervals is representable for *every* pair in
/// them — the question the overflow-check row asks. Distinct from
/// `range_add` returning a bounded interval: `[MIN, MAX] + [0, 0]` cannot
/// overflow yet is still the full range.
pub(crate) fn add_cannot_overflow(a: Range, b: Range) -> bool {
    a.lo.checked_add(b.lo).is_some() && a.hi.checked_add(b.hi).is_some()
}

/// The same question for a difference.
pub(crate) fn sub_cannot_overflow(a: Range, b: Range) -> bool {
    a.lo.checked_sub(b.hi).is_some() && a.hi.checked_sub(b.lo).is_some()
}

/// A shift instruction's amount, when it is a literal in `0..64`.
fn shift_amount(instruction: &CodeInstruction) -> Option<u32> {
    instruction
        .get("shift")
        .and_then(|text| text.parse::<u32>().ok())
        .filter(|amount| *amount < 64)
}

/// Layer 2: the per-block refinements, and which of them were derived.
fn block_facts(
    instructions: &[CodeInstruction],
    models: &(ClassModel, ClassModel),
    overlay: &Ssa,
    blocks: &[Block],
    global: &[Range],
) -> (Vec<Vec<(ValueId, Range)>>, Vec<Vec<ValueId>>) {
    let count = blocks.len();
    if count == 0 {
        return (Vec::new(), Vec::new());
    }
    let mut facts: Vec<HashMap<ValueId, Range>> = vec![HashMap::new(); count];
    let mut derived: Vec<Vec<ValueId>> = vec![Vec::new(); count];

    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); count];
    for (b, block) in blocks.iter().enumerate() {
        for &successor in &block.succ {
            preds[successor].push(b);
        }
    }
    let order = reverse_postorder(blocks);
    let mut position = vec![usize::MAX; count];
    for (rank, &b) in order.iter().enumerate() {
        position[b] = rank;
    }

    // The edge facts a block hands its taken and fall-through successors,
    // filled as the walk reaches each block.
    let mut taken_facts: Vec<Vec<(ValueId, Range)>> = vec![Vec::new(); count];
    let mut fallthrough_facts: Vec<Vec<(ValueId, Range)>> = vec![Vec::new(); count];

    for (rank, &b) in order.iter().enumerate() {
        // 1. Inherit. A single predecessor hands its facts down directly; a
        //    join keeps only what every predecessor agrees on (the hull of
        //    what each path proves), and only when every predecessor is
        //    already computed — a back edge contributes nothing.
        let mut inherited: HashMap<ValueId, Range> = HashMap::new();
        let ready = !preds[b].is_empty()
            && preds[b]
                .iter()
                .all(|&p| position[p] != usize::MAX && position[p] < rank);
        if ready {
            for (index, &p) in preds[b].iter().enumerate() {
                let is_taken = blocks[p].succ.first() == Some(&b);
                let is_fallthrough = blocks[p].succ.len() > 1 && blocks[p].succ[1] == b;
                // Both edges landing here means the branch decides nothing
                // about which way control came, so it proves nothing either.
                let from_edge: &[(ValueId, Range)] = match (is_taken, is_fallthrough) {
                    (true, true) => &[],
                    (true, false) => &taken_facts[p],
                    (false, true) => &fallthrough_facts[p],
                    (false, false) => &[],
                };
                let mut incoming = facts[p].clone();
                for &(value, range) in from_edge {
                    incoming
                        .entry(value)
                        .and_modify(|current| *current = current.meet(range))
                        .or_insert(range);
                }
                if index == 0 {
                    inherited = incoming;
                } else {
                    inherited.retain(|value, range| match incoming.get(value) {
                        Some(other) => {
                            *range = range.hull(*other);
                            true
                        }
                        None => false,
                    });
                }
            }
        }
        if inherited.len() > MAX_FACTS {
            inherited.clear();
        }

        // 2. Derive forward through the block's own pure arithmetic, so a
        //    fact about `i` also constrains `i + 1` and `i * 2`. This is the
        //    range-check widening / narrowing row's machinery. Every value
        //    refined here is *defined* here, so every one of its uses is
        //    dominated by this block and the fact holds at all of them.
        let block = &blocks[b];
        for i in block.start..block.end {
            if !derivable_op(instructions[i].op) {
                continue;
            }
            let Some(RegRef::VReg(register)) = instructions[i]
                .operand("dst")
                .and_then(|operand| classify_ref(operand, &models.0))
            else {
                continue;
            };
            let Some(defined) = overlay.value_defined_at(i, (false, register)) else {
                continue;
            };
            let mut refines = false;
            for (name, operand) in &instructions[i].fields {
                if !is_use_field(name) {
                    continue;
                }
                if let Some(RegRef::VReg(id)) = classify_ref(operand, &models.0) {
                    if let Some(value) = overlay.value_of_use(i, (false, id)) {
                        if inherited.contains_key(&value) {
                            refines = true;
                        }
                    }
                }
            }
            if !refines {
                continue;
            }
            let computed = {
                let resolve = |field: &str| -> Range {
                    let Some(operand) = instructions[i].operand(field) else {
                        return Range::FULL;
                    };
                    match classify_ref(operand, &models.0) {
                        Some(RegRef::VReg(id)) => match overlay.value_of_use(i, (false, id)) {
                            Some(value) => {
                                let base = global.get(value).copied().unwrap_or(Range::FULL);
                                match inherited.get(&value) {
                                    Some(fact) => base.meet(*fact),
                                    None => base,
                                }
                            }
                            None => Range::FULL,
                        },
                        Some(RegRef::Phys(_)) => Range::FULL,
                        None => match literal(&operand.rendered()) {
                            Some(value) => Range::exact(value),
                            None => Range::FULL,
                        },
                    }
                };
                apply(&instructions[i], &resolve)
            };
            if computed.is_full() {
                continue;
            }
            if computed == global.get(defined).copied().unwrap_or(Range::FULL) {
                continue;
            }
            if inherited.len() < MAX_FACTS {
                inherited.insert(defined, computed);
                derived[b].push(defined);
            }
        }

        // 3. Publish the edge facts this block's terminator proves.
        if let Some(compare) = block_compare(instructions, models, overlay, block) {
            let branch = instructions[block.end - 1].op;
            let lookup = |value: ValueId| -> Range {
                let base = global.get(value).copied().unwrap_or(Range::FULL);
                match inherited.get(&value) {
                    Some(fact) => base.meet(*fact),
                    None => base,
                }
            };
            taken_facts[b] = edge_fact(branch, &compare, true, &lookup);
            fallthrough_facts[b] = edge_fact(branch, &compare, false, &lookup);
        }

        facts[b] = inherited;
        derived[b].sort_unstable();
        derived[b].dedup();
    }

    let per_block = facts
        .into_iter()
        .map(|map| {
            let mut sorted: Vec<(ValueId, Range)> = map.into_iter().collect();
            sorted.sort_unstable_by_key(|(value, _)| *value);
            sorted
        })
        .collect();
    (per_block, derived)
}

/// The relation an edge proves between the two compared values.
#[derive(Clone, Copy)]
pub(crate) enum Relation {
    Eq,
    Ne,
    Ge,
    Gt,
    Le,
    Lt,
}

/// The relation that holds on one edge out of a compare-and-branch, when this
/// pass models the branch. `holds` selects the taken edge (the condition is
/// true) or the fall-through (it is false).
///
/// `b.mi` reads the sign of a *wrapped* difference, and the overflow guards
/// `b.vs`/`b.vc` are trap flow: neither is modeled here, and neither may be.
/// The unsigned family reads as the signed one only when both sides are
/// provably non-negative.
pub(crate) fn relation_on_edge(
    branch: CodeOp,
    holds: bool,
    lhs: Range,
    rhs: Range,
) -> Option<Relation> {
    Some(match (branch, holds) {
        (CodeOp::BranchEq, true) | (CodeOp::BranchNe, false) => Relation::Eq,
        (CodeOp::BranchNe, true) | (CodeOp::BranchEq, false) => Relation::Ne,
        (CodeOp::BranchGe, true) | (CodeOp::BranchLt, false) => Relation::Ge,
        (CodeOp::BranchLt, true) | (CodeOp::BranchGe, false) => Relation::Lt,
        (CodeOp::BranchGt, true) | (CodeOp::BranchLe, false) => Relation::Gt,
        (CodeOp::BranchLe, true) | (CodeOp::BranchGt, false) => Relation::Le,
        (CodeOp::BranchHi, _) | (CodeOp::BranchLs, _) | (CodeOp::BranchLo, _) => {
            if !(lhs.non_negative() && rhs.non_negative()) {
                return None;
            }
            match (branch, holds) {
                (CodeOp::BranchHi, true) => Relation::Gt,
                (CodeOp::BranchHi, false) => Relation::Le,
                (CodeOp::BranchLs, true) => Relation::Le,
                (CodeOp::BranchLs, false) => Relation::Gt,
                (CodeOp::BranchLo, true) => Relation::Lt,
                (CodeOp::BranchLo, false) => Relation::Ge,
                _ => return None,
            }
        }
        _ => return None,
    })
}

/// What one edge out of a compare-and-branch proves about the compared
/// values.
fn edge_fact(
    branch: CodeOp,
    compare: &Compare,
    holds: bool,
    lookup: &dyn Fn(ValueId) -> Range,
) -> Vec<(ValueId, Range)> {
    let resolve = |side: Operandish| -> Option<Range> {
        match side {
            Operandish::Value(value) => Some(lookup(value)),
            Operandish::Literal(value) => Some(Range::exact(value)),
            Operandish::Opaque => None,
        }
    };
    let (Some(lhs), Some(rhs)) = (resolve(compare.lhs), resolve(compare.rhs)) else {
        return Vec::new();
    };
    let Some(relation) = relation_on_edge(branch, holds, lhs, rhs) else {
        return Vec::new();
    };

    let (left, right) = refine(relation, lhs, rhs);
    let mut out = Vec::new();
    if let (Operandish::Value(value), Some(range)) = (compare.lhs, left) {
        out.push((value, range));
    }
    if let (Operandish::Value(value), Some(range)) = (compare.rhs, right) {
        out.push((value, range));
    }
    out
}

/// Tighten each side of a compare from the relation and the other side's
/// range. Every bound arrives through a checked operation, so a fact is
/// dropped rather than wrapped.
fn refine(relation: Relation, lhs: Range, rhs: Range) -> (Option<Range>, Option<Range>) {
    let tighten = |lo: Option<i64>, hi: Option<i64>, base: Range| -> Option<Range> {
        let candidate = Range {
            lo: lo.unwrap_or(i64::MIN),
            hi: hi.unwrap_or(i64::MAX),
        };
        if candidate.is_full() {
            None
        } else {
            Some(base.meet(candidate))
        }
    };
    match relation {
        Relation::Eq => (
            tighten(Some(rhs.lo), Some(rhs.hi), lhs),
            tighten(Some(lhs.lo), Some(lhs.hi), rhs),
        ),
        // `!=` only sharpens a bound when the excluded value sits exactly on
        // one — otherwise what is left is not an interval.
        Relation::Ne => {
            let exclude = |value: Option<i64>, base: Range| -> Option<Range> {
                let value = value?;
                if base.lo == value && base.lo < base.hi {
                    Some(Range {
                        lo: base.lo.checked_add(1)?,
                        hi: base.hi,
                    })
                } else if base.hi == value && base.lo < base.hi {
                    Some(Range {
                        lo: base.lo,
                        hi: base.hi.checked_sub(1)?,
                    })
                } else {
                    None
                }
            };
            (exclude(rhs.singleton(), lhs), exclude(lhs.singleton(), rhs))
        }
        Relation::Ge => (
            tighten(Some(rhs.lo), None, lhs),
            tighten(None, Some(lhs.hi), rhs),
        ),
        Relation::Gt => (
            tighten(rhs.lo.checked_add(1), None, lhs),
            tighten(None, lhs.hi.checked_sub(1), rhs),
        ),
        Relation::Le => (
            tighten(None, Some(rhs.hi), lhs),
            tighten(Some(lhs.lo), None, rhs),
        ),
        Relation::Lt => (
            tighten(None, rhs.hi.checked_sub(1), lhs),
            tighten(lhs.lo.checked_add(1), None, rhs),
        ),
    }
}

/// Whether the relation provably holds (or provably fails) for every pair of
/// values in the two ranges. `None` means the ranges overlap either way and
/// the comparison genuinely has to run.
pub(crate) fn decide(relation: Relation, lhs: Range, rhs: Range) -> Option<bool> {
    match relation {
        Relation::Eq => {
            if lhs.hi < rhs.lo || lhs.lo > rhs.hi {
                Some(false)
            } else if lhs.singleton().is_some() && lhs == rhs {
                Some(true)
            } else {
                None
            }
        }
        Relation::Ne => decide(Relation::Eq, lhs, rhs).map(|equal| !equal),
        Relation::Ge => {
            if lhs.lo >= rhs.hi {
                Some(true)
            } else if lhs.hi < rhs.lo {
                Some(false)
            } else {
                None
            }
        }
        Relation::Lt => decide(Relation::Ge, lhs, rhs).map(|ge| !ge),
        Relation::Gt => {
            if lhs.lo > rhs.hi {
                Some(true)
            } else if lhs.hi <= rhs.lo {
                Some(false)
            } else {
                None
            }
        }
        Relation::Le => decide(Relation::Gt, lhs, rhs).map(|gt| !gt),
    }
}

/// Reverse postorder over the CFG from block 0 — the order that lets a
/// forward fact walk see every predecessor of a non-loop block first.
pub(crate) fn reverse_postorder(blocks: &[Block]) -> Vec<usize> {
    if blocks.is_empty() {
        return Vec::new();
    }
    let mut seen = vec![false; blocks.len()];
    let mut order = Vec::with_capacity(blocks.len());
    // Iterative DFS: the giant generated functions overflow a recursive one.
    let mut stack: Vec<(usize, usize)> = vec![(0, 0)];
    seen[0] = true;
    while let Some((block, next)) = stack.pop() {
        if next < blocks[block].succ.len() {
            stack.push((block, next + 1));
            let successor = blocks[block].succ[next];
            if !seen[successor] {
                seen[successor] = true;
                stack.push((successor, 0));
            }
        } else {
            order.push(block);
        }
    }
    order.reverse();
    order
}

/// A block index per instruction — the lookup every consuming row needs to
/// ask a per-block question about an instruction.
pub(crate) fn block_of(blocks: &[Block], length: usize) -> Vec<usize> {
    let mut map = vec![0usize; length];
    for (b, block) in blocks.iter().enumerate() {
        map[block.start..block.end.min(length)].fill(b);
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::engine::regalloc::analysis::build_cfg;
    use crate::codegen::engine::regalloc::class_models;

    fn ci(op: &str, fields: &[(&'static str, &str)]) -> CodeInstruction {
        let mut inst = CodeInstruction::new(op);
        for (k, v) in fields {
            inst = inst.field(k, v);
        }
        inst
    }

    fn mov_imm(dst: &str, value: &str) -> CodeInstruction {
        ci(
            "mov_imm",
            &[("dst", dst), ("type", "Integer"), ("value", value)],
        )
    }

    /// Build the facts for a stream, with the AArch64 model (the spellings
    /// the fixtures use).
    fn facts_for(stream: &[CodeInstruction]) -> (Ranges, Vec<Block>, Ssa) {
        let model = crate::arch::aarch64::regmodel::Aarch64RegisterModel;
        let models = class_models(&model);
        let blocks = build_cfg(stream);
        let overlay = super::super::ssa::build(stream, &blocks, &models);
        let ranges = analyze(stream, &models, &overlay, &blocks);
        (ranges, blocks, overlay)
    }

    /// A literal's interval is exact, and it propagates through pure
    /// arithmetic.
    #[test]
    fn constants_propagate_through_arithmetic() {
        let stream = vec![
            mov_imm("%v1", "10"),
            ci("add_imm", &[("dst", "%v2"), ("src", "%v1"), ("imm", "5")]),
            ci("ret", &[]),
        ];
        let (ranges, _, overlay) = facts_for(&stream);
        let value = overlay.value_defined_at(1, (false, 2)).expect("defined");
        assert_eq!(ranges.at(0, value), Range::exact(15));
    }

    /// A checked add whose inputs cannot sum past the 64-bit range gets the
    /// exact interval; one that can gets nothing.
    #[test]
    fn checked_add_is_modeled_only_when_representable() {
        let safe = vec![
            mov_imm("%v1", "1"),
            ci("adds", &[("dst", "%v2"), ("lhs", "%v1"), ("rhs", "%v1")]),
            ci("ret", &[]),
        ];
        let (ranges, _, overlay) = facts_for(&safe);
        let value = overlay.value_defined_at(1, (false, 2)).expect("defined");
        assert_eq!(ranges.at(0, value), Range::exact(2));

        let unsafe_stream = vec![
            mov_imm("%v1", &i64::MAX.to_string()),
            ci("adds", &[("dst", "%v2"), ("lhs", "%v1"), ("rhs", "%v1")]),
            ci("ret", &[]),
        ];
        let (ranges, _, overlay) = facts_for(&unsafe_stream);
        let value = overlay.value_defined_at(1, (false, 2)).expect("defined");
        assert!(
            ranges.at(0, value).is_full(),
            "a sum that can overflow is never claimed to be known"
        );
    }

    /// The dominating condition of an `IF` refines the compared value inside
    /// the guarded block — the fact every check-elision row is built on.
    #[test]
    fn a_dominating_compare_refines_the_guarded_block() {
        let stream = vec![
            ci("cmp_imm", &[("lhs", "%v1"), ("rhs", "10")]),
            ci("b.ge", &[("target", "big")]),
            // fall-through: %v1 < 10
            ci("mov", &[("dst", "%v2"), ("src", "%v1")]),
            ci("ret", &[]),
            ci("label", &[("name", "big")]),
            ci("ret", &[]),
        ];
        let (ranges, blocks, overlay) = facts_for(&stream);
        let value = overlay.value_of_use(0, (false, 1)).expect("used");
        let block = block_of(&blocks, stream.len())[2];
        assert_eq!(
            ranges.at(block, value).hi,
            9,
            "the fall-through edge proves %v1 <= 9"
        );
    }

    /// The refinement is pushed forward through the block's own arithmetic:
    /// proving `i <= 9` also proves `i + 1 <= 10`. This is the range-check
    /// widening / narrowing row.
    #[test]
    fn refinements_are_derived_through_arithmetic() {
        let stream = vec![
            ci("cmp_imm", &[("lhs", "%v1"), ("rhs", "10")]),
            ci("b.ge", &[("target", "big")]),
            ci("add_imm", &[("dst", "%v2"), ("src", "%v1"), ("imm", "1")]),
            ci("ret", &[]),
            ci("label", &[("name", "big")]),
            ci("ret", &[]),
        ];
        let (ranges, blocks, overlay) = facts_for(&stream);
        let derived_value = overlay.value_defined_at(2, (false, 2)).expect("defined");
        let block = block_of(&blocks, stream.len())[2];
        assert_eq!(ranges.at(block, derived_value).hi, 10);
        assert!(
            ranges.is_derived(block, derived_value),
            "the fact came through arithmetic, not off the compare"
        );
    }

    /// A join keeps only what both arms prove.
    #[test]
    fn a_join_keeps_only_the_common_fact() {
        let stream = vec![
            ci("cmp_imm", &[("lhs", "%v1"), ("rhs", "0")]),
            ci("b.lt", &[("target", "negative")]),
            ci("b", &[("target", "join")]),
            ci("label", &[("name", "negative")]),
            ci("b", &[("target", "join")]),
            ci("label", &[("name", "join")]),
            ci("mov", &[("dst", "%v2"), ("src", "%v1")]),
            ci("ret", &[]),
        ];
        let (ranges, blocks, overlay) = facts_for(&stream);
        let value = overlay.value_of_use(0, (false, 1)).expect("used");
        let join = block_of(&blocks, stream.len())[6];
        assert!(
            ranges.at(join, value).is_full(),
            "one arm proves >= 0 and the other < 0: together, nothing"
        );
    }

    /// `decide` answers only when the ranges settle the question.
    #[test]
    fn decide_is_all_or_nothing() {
        let low = Range { lo: 0, hi: 5 };
        let high = Range { lo: 10, hi: 20 };
        assert_eq!(decide(Relation::Lt, low, high), Some(true));
        assert_eq!(decide(Relation::Ge, low, high), Some(false));
        assert_eq!(decide(Relation::Eq, low, high), Some(false));
        assert_eq!(decide(Relation::Lt, low, low), None);
    }

    /// The overflow guards are outside the model entirely: no edge out of a
    /// `b.vs`/`b.vc` ever produces a fact.
    #[test]
    fn overflow_guards_prove_nothing() {
        assert!(relation_on_edge(CodeOp::BranchVs, true, Range::FULL, Range::FULL).is_none());
        assert!(relation_on_edge(CodeOp::BranchVc, false, Range::FULL, Range::FULL).is_none());
        assert!(relation_on_edge(CodeOp::BranchMi, true, Range::FULL, Range::FULL).is_none());
    }

    /// An unsigned comparison reads as a signed one only when both sides are
    /// provably non-negative.
    #[test]
    fn unsigned_compares_need_non_negative_operands() {
        let unknown = Range::FULL;
        let positive = Range { lo: 0, hi: 100 };
        assert!(relation_on_edge(CodeOp::BranchLo, true, unknown, positive).is_none());
        assert!(relation_on_edge(CodeOp::BranchLo, true, positive, positive).is_some());
    }
}
