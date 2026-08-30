//! Partial redundancy elimination (PRE) — a Level-3 Opt2 catalog row
//! (`planning/optimizations.md`): an expression that is already computed on
//! *some* paths into a block is made fully redundant by computing it on the
//! remaining one, and the block's own copy then goes.
//!
//! Global value numbering (the neighbouring row) removes a recomputation only
//! when the earlier result is available on *every* path — it needs the leader
//! to dominate. The classic case it must decline is the diamond:
//!
//! ```text
//!     if c then  h = a * b        <- computed here
//!     ...
//!     d = a * b                   <- and again here, though only one arm needs it
//! ```
//!
//! PRE closes it by inserting `a * b` on the arm that lacks it, after which
//! every path arrives with the value already in hand and the join's own
//! multiply is deleted outright.
//!
//! **This row never grows the program.** It fires only when exactly *one*
//! predecessor lacks the expression, so the one instruction inserted is paid
//! for by the one deleted — static size is unchanged, and every path that
//! already had the value now executes one operation fewer. That is a stronger
//! guarantee than textbook PRE offers, and it is deliberate: without a profile
//! there is no way to justify a net-growth insertion, so the row declines
//! rather than guess.
//!
//! **Where it may insert.** Only into a predecessor whose *only* successor is
//! the join. A predecessor that branches would run the inserted computation on
//! every one of its executions while reaching the join on only some of them —
//! the classic critical-edge regression, avoided here by declining instead of
//! splitting the edge.
//!
//! **What may move.** The same pure, flag-free, single-`dst` whitelist value
//! numbering uses, so nothing that can trap, touch memory, or set flags is
//! ever duplicated. Both the leader's destination and the join's must be
//! written exactly once in the whole function, and the leader's operands must
//! be defined in blocks dominating the insertion point — the holder-currency
//! rule the memory and value-numbering rows share, restated for a new
//! position.

use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::regalloc::analysis::{
    build_cfg, classify_ref, effect, is_use_field, Block, ClassModel, RegRef,
};
use crate::codegen::engine::regalloc::{self};
use crate::codegen::engine::types::CodeInstruction;
use crate::target::shared::regmodel::RegisterModel;

use super::lvn::numberable_op;
use super::plans::ranges::block_of;
use super::plans::ssa::{self, Ssa, ValueDef, ValueId};

/// One operand of an expression, as its identity.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
enum Part {
    Value(ValueId),
    Literal(String),
}

/// An expression's identity: the op plus every operand in field order.
type Key = (crate::arch::ops::CodeOp, Vec<Part>);

/// Run the PRE row over one function's selected stream, in place.
/// Self-guarded on the row's catalog level (3).
pub(crate) fn eliminate(instructions: &mut Vec<CodeInstruction>, model: &dyn RegisterModel) {
    if !crate::optimizer::level_enabled(3) {
        return;
    }
    let models = regalloc::class_models(model);
    let blocks = build_cfg(instructions);
    if blocks.len() < 3 {
        return;
    }
    let overlay = ssa::build(instructions, &blocks, &models);
    let where_of = block_of(&blocks, instructions.len());

    let mut def_count: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
    for instruction in instructions.iter() {
        for reference in effect(instruction, &models.0).defs {
            if let RegRef::VReg(id) = reference {
                *def_count.entry(id).or_insert(0) += 1;
            }
        }
    }

    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); blocks.len()];
    for (b, block) in blocks.iter().enumerate() {
        for &successor in &block.succ {
            preds[successor].push(b);
        }
    }

    // Every candidate expression, keyed, with the instruction that computes it.
    let mut leaders: std::collections::HashMap<Key, Vec<usize>> = std::collections::HashMap::new();
    for i in 0..instructions.len() {
        if let Some(key) = expression(instructions, &models.0, &overlay, i) {
            leaders.entry(key).or_default().push(i);
        }
    }

    // (join instruction to delete, its dst vreg, replacement operand, insertion
    // point, the instruction to insert).
    let mut plans: Vec<Plan> = Vec::new();
    let mut claimed: Vec<bool> = vec![false; instructions.len()];

    for i in 0..instructions.len() {
        if claimed[i] {
            continue;
        }
        let block = where_of[i];
        if preds[block].len() < 2 {
            continue;
        }
        let Some(key) = expression(instructions, &models.0, &overlay, i) else {
            continue;
        };
        let Some(RegRef::VReg(join_register)) = sole_def(instructions, &models.0, i) else {
            continue;
        };
        if def_count.get(&join_register) != Some(&1) {
            continue;
        }
        let Some(candidates) = leaders.get(&key) else {
            continue;
        };
        for &leader in candidates {
            if leader == i || claimed[leader] || where_of[leader] == block {
                continue;
            }
            let Some(RegRef::VReg(holder_register)) = sole_def(instructions, &models.0, leader)
            else {
                continue;
            };
            if holder_register == join_register || def_count.get(&holder_register) != Some(&1) {
                continue;
            }
            // Fully available is the value-numbering row's case, not this one's.
            if overlay.dominates(where_of[leader], block) {
                continue;
            }
            let covered = |pred: usize| overlay.dominates(where_of[leader], pred);
            let uncovered: Vec<usize> = preds[block]
                .iter()
                .copied()
                .filter(|&pred| !covered(pred))
                .collect();
            // Exactly one gap, so one insertion pays for one deletion.
            if uncovered.len() != 1 || uncovered.len() == preds[block].len() {
                continue;
            }
            let gap = uncovered[0];
            // Only an edge the predecessor always takes: otherwise the
            // insertion runs more often than the deletion saved.
            if blocks[gap].succ.len() != 1 {
                continue;
            }
            if !operands_reach(instructions, &models.0, &overlay, &where_of, leader, gap) {
                continue;
            }
            let Some(point) = insertion_point(instructions, &blocks[gap]) else {
                continue;
            };
            // A block dominates itself, so `operands_reach` would accept an
            // operand the gap block defines *after* the insertion point.
            if !operands_precede(
                instructions,
                &models.0,
                &overlay,
                leader,
                gap_block_range(&blocks[gap]),
                point,
            ) {
                continue;
            }
            let Some(holder) = instructions[leader].operand("dst").cloned() else {
                continue;
            };
            plans.push(Plan {
                delete: i,
                replace: join_register,
                holder,
                point,
                inserted: clone_of(&instructions[leader]),
            });
            claimed[i] = true;
            claimed[leader] = true;
            break;
        }
    }

    if plans.is_empty() {
        return;
    }
    let fired = plans.len() as u64;
    apply(instructions, &models.0, plans);
    crate::optimizer::stats::count_partial_redundancies(fired);
}

/// One decided rewrite.
struct Plan {
    /// The join's recomputation, to be deleted.
    delete: usize,
    /// Its destination vreg — every use is repointed at the holder.
    replace: u32,
    /// The leader's destination, which the uses now read.
    holder: Operand,
    /// Where in the gap predecessor the computation goes.
    point: usize,
    /// The computation to place there.
    inserted: CodeInstruction,
}

/// The expression an instruction computes, when this row models it.
fn expression(
    instructions: &[CodeInstruction],
    model: &ClassModel,
    overlay: &Ssa,
    i: usize,
) -> Option<Key> {
    let instruction = &instructions[i];
    if !numberable_op(instruction.op) {
        return None;
    }
    let mut parts = Vec::new();
    for (name, operand) in &instruction.fields {
        if !is_use_field(name) {
            continue;
        }
        match classify_ref(operand, model) {
            Some(RegRef::VReg(id)) => {
                parts.push(Part::Value(overlay.value_of_use(i, (false, id))?))
            }
            // A physical register's contents are an ABI effect, not a value
            // this row can identify.
            Some(RegRef::Phys(_)) => return None,
            None => parts.push(Part::Literal(operand.rendered().into_owned())),
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some((instruction.op, parts))
}

/// The instruction's single register definition, when it has exactly one.
fn sole_def(instructions: &[CodeInstruction], model: &ClassModel, i: usize) -> Option<RegRef> {
    let effect = effect(&instructions[i], model);
    if effect.defs.len() != 1 || effect.is_call {
        return None;
    }
    Some(effect.defs[0])
}

/// Whether every value the leader reads is defined somewhere that dominates
/// `target`, so the copy placed there computes the same thing.
fn operands_reach(
    instructions: &[CodeInstruction],
    model: &ClassModel,
    overlay: &Ssa,
    where_of: &[usize],
    leader: usize,
    target: usize,
) -> bool {
    for (name, operand) in &instructions[leader].fields {
        if !is_use_field(name) {
            continue;
        }
        let Some(RegRef::VReg(id)) = classify_ref(operand, model) else {
            continue;
        };
        let Some(value) = overlay.value_of_use(leader, (false, id)) else {
            return false;
        };
        match &overlay.values[value] {
            // A live-in is available everywhere.
            ValueDef::Entry => {}
            ValueDef::Inst(at) => {
                if !overlay.dominates(where_of[*at], target) {
                    return false;
                }
            }
            ValueDef::Phi { block, .. } => {
                if !overlay.dominates(*block, target) {
                    return false;
                }
            }
        }
    }
    true
}

/// The half-open instruction range a block covers.
fn gap_block_range(block: &Block) -> (usize, usize) {
    (block.start, block.end)
}

/// Whether every value the leader reads that is defined *inside* the gap block
/// is defined before the insertion point.
fn operands_precede(
    instructions: &[CodeInstruction],
    model: &ClassModel,
    overlay: &Ssa,
    leader: usize,
    range: (usize, usize),
    point: usize,
) -> bool {
    for (name, operand) in &instructions[leader].fields {
        if !is_use_field(name) {
            continue;
        }
        let Some(RegRef::VReg(id)) = classify_ref(operand, model) else {
            continue;
        };
        let Some(value) = overlay.value_of_use(leader, (false, id)) else {
            return false;
        };
        if let ValueDef::Inst(at) = &overlay.values[value] {
            if *at >= range.0 && *at < range.1 && *at >= point {
                return false;
            }
        }
    }
    true
}

/// Where a computation may be placed at the end of a block: before the
/// terminator, and before any flag-setting run leading up to it, so an
/// inserted operation can never land between a comparison and the branch — or
/// the block-crossing flag fact — that reads it.
///
/// The walk back stops at the first instruction that is not itself a flag
/// setter. Going further would risk stepping over a definition the inserted
/// computation reads, and buys nothing: splitting a setter/branch pair is the
/// only hazard being avoided.
fn insertion_point(instructions: &[CodeInstruction], block: &Block) -> Option<usize> {
    let floor = if instructions[block.start].op == crate::arch::ops::CodeOp::Label {
        block.start + 1
    } else {
        block.start
    };
    let mut point = block.end;
    if point > floor
        && crate::codegen::engine::regalloc::analysis::is_block_terminator(
            instructions[point - 1].op,
        )
    {
        point -= 1;
    }
    while point > floor && is_flag_setter(instructions[point - 1].op) {
        point -= 1;
    }
    (point >= floor).then_some(point)
}

/// The ops whose whole purpose is to leave a condition in the flags for the
/// branch that follows. Nothing may be inserted between one and its branch.
fn is_flag_setter(op: crate::arch::ops::CodeOp) -> bool {
    use crate::arch::ops::CodeOp as Op;
    matches!(
        op,
        Op::Cmp | Op::CmpImm | Op::Adds | Op::Subs | Op::FCmpD | Op::FCmpZeroD
    )
}

/// A fresh instruction with the same op and fields — `CodeInstruction` is not
/// `Clone`, so rebuild it from its parts (the same shape `lower_to_mir` uses).
fn clone_of(instruction: &CodeInstruction) -> CodeInstruction {
    CodeInstruction {
        op: instruction.op,
        fields: instruction.fields.clone(),
        source: instruction.source,
    }
}

/// Delete each join recomputation, repoint its uses at the holder, and place
/// each inserted computation.
fn apply(instructions: &mut Vec<CodeInstruction>, model: &ClassModel, plans: Vec<Plan>) {
    for plan in &plans {
        for (i, instruction) in instructions.iter_mut().enumerate() {
            if i == plan.delete {
                continue;
            }
            for (name, operand) in instruction.fields.iter_mut() {
                if !is_use_field(name) {
                    continue;
                }
                if matches!(classify_ref(operand, model), Some(RegRef::VReg(id)) if id == plan.replace)
                {
                    *operand = plan.holder.clone();
                }
            }
        }
    }

    let mut deletions = vec![false; instructions.len()];
    let mut arrivals: Vec<(usize, CodeInstruction)> = Vec::new();
    for plan in plans {
        deletions[plan.delete] = true;
        arrivals.push((plan.point, plan.inserted));
    }
    // Later positions first, so an earlier insertion cannot shift a later one.
    arrivals.sort_by(|a, b| b.0.cmp(&a.0));

    let mut index = 0;
    instructions.retain(|_| {
        let keep = !deletions[index];
        index += 1;
        keep
    });
    // The deletions above shifted every later index; recompute each arrival's
    // position by how many deletions preceded it.
    let mut shift: Vec<usize> = Vec::with_capacity(deletions.len() + 1);
    let mut removed = 0;
    for &deleted in &deletions {
        shift.push(removed);
        if deleted {
            removed += 1;
        }
    }
    shift.push(removed);
    for (point, instruction) in arrivals {
        let at = point - shift[point.min(shift.len() - 1)];
        instructions.insert(at.min(instructions.len()), instruction);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch::ops::CodeOp;
    use crate::optimizer::{with_opt_level, OptLevel};

    fn ci(op: &str, fields: &[(&'static str, &str)]) -> CodeInstruction {
        let mut inst = CodeInstruction::new(op);
        for (k, v) in fields {
            inst = inst.field(k, v);
        }
        inst
    }

    fn ops(instructions: &[CodeInstruction]) -> Vec<CodeOp> {
        instructions.iter().map(|inst| inst.op).collect()
    }

    fn run(stream: &mut Vec<CodeInstruction>, level: u8) {
        let model = crate::arch::aarch64::regmodel::Aarch64RegisterModel;
        with_opt_level(OptLevel(level), || eliminate(stream, &model));
    }

    /// The textbook diamond: one arm already computes `a * b`, the other does
    /// not, and the join recomputes it. The gap arm gets the computation and
    /// the join's copy goes — same instruction count, one multiply fewer on
    /// the arm that had it.
    #[test]
    fn a_partially_available_expression_is_completed() {
        let mut stream = vec![
            ci("cmp_imm", &[("lhs", "%v9"), ("rhs", "0")]),
            ci("b.eq", &[("target", "arm")]),
            // gap arm: no multiply
            ci("b", &[("target", "join")]),
            ci("label", &[("name", "arm")]),
            ci("mul", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
            ci("b", &[("target", "join")]),
            ci("label", &[("name", "join")]),
            ci("mul", &[("dst", "%v4"), ("lhs", "%v1"), ("rhs", "%v2")]),
            ci("mov", &[("dst", "%v5"), ("src", "%v4")]),
            ci("ret", &[]),
        ];
        let before = stream.len();
        run(&mut stream, 3);
        assert_eq!(
            stream.len(),
            before,
            "PRE never changes the instruction count"
        );
        assert_eq!(
            ops(&stream),
            vec![
                CodeOp::CmpImm,
                CodeOp::BranchEq,
                CodeOp::Mul,
                CodeOp::Branch,
                CodeOp::Label,
                CodeOp::Mul,
                CodeOp::Branch,
                CodeOp::Label,
                CodeOp::Mov,
                CodeOp::Ret,
            ],
        );
        // The surviving use reads the leader's register.
        let mov = stream.iter().find(|i| i.op == CodeOp::Mov).expect("mov");
        assert_eq!(mov.get("src").as_deref(), Some("%v3"));
    }

    /// A predecessor that branches would run the insertion more often than
    /// the join runs, so the row declines.
    #[test]
    fn a_branching_predecessor_is_not_an_insertion_point() {
        let stream = || {
            vec![
                ci("cmp_imm", &[("lhs", "%v9"), ("rhs", "0")]),
                ci("b.eq", &[("target", "arm")]),
                ci("cmp_imm", &[("lhs", "%v8"), ("rhs", "0")]),
                ci("b.eq", &[("target", "join")]),
                ci("ret", &[]),
                ci("label", &[("name", "arm")]),
                ci("mul", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
                ci("b", &[("target", "join")]),
                ci("label", &[("name", "join")]),
                ci("mul", &[("dst", "%v4"), ("lhs", "%v1"), ("rhs", "%v2")]),
                ci("mov", &[("dst", "%v5"), ("src", "%v4")]),
                ci("ret", &[]),
            ]
        };
        let mut off = stream();
        run(&mut off, 3);
        assert_eq!(ops(&off), ops(&stream()));
    }

    /// A leader that dominates the join is fully redundant — the value
    /// numbering row's case, left alone here.
    #[test]
    fn a_fully_available_expression_is_left_to_value_numbering() {
        let stream = || {
            vec![
                ci("mul", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
                ci("cmp_imm", &[("lhs", "%v9"), ("rhs", "0")]),
                ci("b.eq", &[("target", "arm")]),
                ci("b", &[("target", "join")]),
                ci("label", &[("name", "arm")]),
                ci("b", &[("target", "join")]),
                ci("label", &[("name", "join")]),
                ci("mul", &[("dst", "%v4"), ("lhs", "%v1"), ("rhs", "%v2")]),
                ci("mov", &[("dst", "%v5"), ("src", "%v4")]),
                ci("ret", &[]),
            ]
        };
        let mut off = stream();
        run(&mut off, 3);
        assert_eq!(ops(&off), ops(&stream()));
    }

    /// The row is off below `-O3`.
    #[test]
    fn level_two_disables_the_row() {
        let stream = || {
            vec![
                ci("cmp_imm", &[("lhs", "%v9"), ("rhs", "0")]),
                ci("b.eq", &[("target", "arm")]),
                ci("b", &[("target", "join")]),
                ci("label", &[("name", "arm")]),
                ci("mul", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
                ci("b", &[("target", "join")]),
                ci("label", &[("name", "join")]),
                ci("mul", &[("dst", "%v4"), ("lhs", "%v1"), ("rhs", "%v2")]),
                ci("mov", &[("dst", "%v5"), ("src", "%v4")]),
                ci("ret", &[]),
            ]
        };
        let mut off = stream();
        run(&mut off, 2);
        assert_eq!(ops(&off), ops(&stream()));
    }
}
