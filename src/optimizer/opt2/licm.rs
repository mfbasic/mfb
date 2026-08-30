//! Loop-nest invariant code motion — a Level-3 Opt2 catalog row
//! (`planning/optimizations.md`): hoist a loop-invariant computation to the
//! **shallowest** enclosing loop level it is still invariant at, not merely
//! out of the loop it happens to sit in.
//!
//! The Opt1 LICM row already hoists out of a *structured* `FOR`/`WHILE` body,
//! one level, on the NIR. This is the other half of the problem, and it is a
//! genuinely different one:
//!
//! - By the Opt2 seam the loops are the ones the *machine* has, which is a
//!   superset of the ones the source wrote — collection iteration, string
//!   scans and the desugarings all emit loops of their own that no NIR pass
//!   can see. [`super::plans::mirloops`] finds them from the CFG.
//! - Hoisting to the shallowest level is what makes a nest pay. A value
//!   invariant in both an inner and an outer loop, moved only out of the
//!   inner one, still runs once per outer iteration; moved to the outer
//!   preheader it runs once. The row walks the nest outward, re-testing
//!   invariance at each level, so it lands as far out as the facts allow.
//!
//! **What may move.** The pure, flag-free, memory-free whitelist, with a
//! single `dst` that is written exactly once in the whole function and whose
//! operands are all either defined outside the loop or written exactly once
//! and defined outside it. Nothing that can trap, read or write memory, or
//! set flags is ever hoisted — a trapping op moved in front of a loop would
//! raise on a zero-iteration loop that used to complete quietly, which is the
//! one thing a behavior-preserving row may not do.
//!
//! **Where it lands.** Only into a real preheader — a single outside
//! predecessor whose only successor is the header. This module never creates
//! one; a loop entered from two places simply is not hoisted out of, because
//! the alternative (synthesizing a block) would change the CFG the
//! neighbouring rows already reasoned about.

use crate::codegen::engine::regalloc::analysis::{
    build_cfg, classify_ref, effect, is_use_field, Block, ClassModel, RegRef,
};
use crate::codegen::engine::regalloc::{self};
use crate::codegen::engine::types::CodeInstruction;
use crate::target::shared::regmodel::RegisterModel;

use super::lvn::numberable_op;
use super::plans::mirloops::{self, Loop};
use super::plans::ranges::block_of;
use super::plans::ssa::{self, Ssa, ValueDef};

/// Run the loop-nest code-motion row over one function's selected stream, in
/// place. Self-guarded on the row's catalog level (3).
pub(crate) fn hoist(instructions: &mut Vec<CodeInstruction>, model: &dyn RegisterModel) {
    if !crate::optimizer::level_enabled(3) {
        return;
    }
    let models = regalloc::class_models(model);
    let blocks = build_cfg(instructions);
    if blocks.len() < 2 {
        return;
    }
    let overlay = ssa::build(instructions, &blocks, &models);
    let loops = mirloops::find(&blocks, &overlay);
    if loops.is_empty() {
        return;
    }
    let where_of = block_of(&blocks, instructions.len());

    let mut def_count: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
    for instruction in instructions.iter() {
        for reference in effect(instruction, &models.0).defs {
            if let RegRef::VReg(id) = reference {
                *def_count.entry(id).or_insert(0) += 1;
            }
        }
    }

    // For each candidate, the outermost loop it is still invariant in. Walking
    // the nest outward from the innermost containing loop is what makes this
    // the *nest* row rather than one more single-level hoist.
    let mut moves: Vec<(usize, usize)> = Vec::new();
    for i in 0..instructions.len() {
        if !numberable_op(instructions[i].op) {
            continue;
        }
        let Some(RegRef::VReg(destination)) = sole_def(instructions, &models.0, i) else {
            continue;
        };
        if def_count.get(&destination) != Some(&1) {
            continue;
        }
        let block = where_of[i];
        // Innermost first, then progressively shallower: the last level whose
        // preheader accepts the instruction wins.
        let mut containing: Vec<&Loop> = loops.iter().filter(|l| l.contains(block)).collect();
        containing.sort_by_key(|l| std::cmp::Reverse(l.depth));
        let mut landing: Option<usize> = None;
        for enclosing in containing {
            if !invariant_in(instructions, &models.0, &overlay, &where_of, i, enclosing) {
                break;
            }
            let Some(preheader) = enclosing.preheader else {
                // No place to put it at this level; a shallower level may
                // still have one, but the value would then cross this loop's
                // entry without a home. Stop here.
                break;
            };
            landing = Some(preheader);
        }
        if let Some(preheader) = landing {
            if !blocks[preheader].succ.is_empty() {
                moves.push((i, preheader));
            }
        }
    }

    if moves.is_empty() {
        return;
    }
    let fired = moves.len() as u64;
    relocate(instructions, &blocks, moves);
    crate::optimizer::stats::count_loop_nest_hoists(fired);
}

/// The instruction's single register definition, when it has exactly one.
fn sole_def(instructions: &[CodeInstruction], model: &ClassModel, i: usize) -> Option<RegRef> {
    let effect = effect(&instructions[i], model);
    if effect.defs.len() != 1 || effect.is_call {
        return None;
    }
    Some(effect.defs[0])
}

/// Whether every value the instruction reads is defined outside `enclosing`
/// (and written exactly once, so no path inside the loop can change it).
fn invariant_in(
    instructions: &[CodeInstruction],
    model: &ClassModel,
    overlay: &Ssa,
    where_of: &[usize],
    i: usize,
    enclosing: &Loop,
) -> bool {
    for (name, operand) in &instructions[i].fields {
        if !is_use_field(name) {
            continue;
        }
        let Some(reference) = classify_ref(operand, model) else {
            // A literal.
            continue;
        };
        let RegRef::VReg(id) = reference else {
            // A physical register is an ABI effect with no tracked lifetime.
            return false;
        };
        let Some(value) = overlay.value_of_use(i, (false, id)) else {
            return false;
        };
        match &overlay.values[value] {
            // A live-in is defined before every loop.
            ValueDef::Entry => {}
            ValueDef::Inst(at) => {
                if enclosing.contains(where_of[*at]) {
                    return false;
                }
            }
            // A join inside the loop is loop-carried by definition.
            ValueDef::Phi { block, .. } => {
                if enclosing.contains(*block) {
                    return false;
                }
            }
        }
    }
    true
}

/// Move each instruction to the end of its landing block, before the
/// terminator and before any flag-setting run leading up to it.
fn relocate(instructions: &mut Vec<CodeInstruction>, blocks: &[Block], moves: Vec<(usize, usize)>) {
    let point_of = |block: usize| -> usize {
        let block = &blocks[block];
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
        point
    };

    let mut removed = vec![false; instructions.len()];
    let mut arrivals: Vec<(usize, Vec<usize>)> = Vec::new();
    for (source, destination) in moves {
        let point = point_of(destination);
        // Never move an instruction to a point after itself: that would be a
        // sink, not a hoist, and the ordering below assumes forward motion is
        // impossible here.
        if point > source {
            continue;
        }
        removed[source] = true;
        match arrivals.iter_mut().find(|(at, _)| *at == point) {
            Some((_, list)) => list.push(source),
            None => arrivals.push((point, vec![source])),
        }
    }

    let taken = std::mem::take(instructions);
    let mut carried: Vec<Option<CodeInstruction>> = taken.into_iter().map(Some).collect();
    let mut rebuilt: Vec<CodeInstruction> = Vec::with_capacity(carried.len());
    for index in 0..carried.len() {
        if let Some((_, sources)) = arrivals.iter().find(|(at, _)| *at == index) {
            for &source in sources {
                if let Some(instruction) = carried[source].take() {
                    rebuilt.push(instruction);
                }
            }
        }
        if removed[index] {
            continue;
        }
        if let Some(instruction) = carried[index].take() {
            rebuilt.push(instruction);
        }
    }
    *instructions = rebuilt;
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
        with_opt_level(OptLevel(level), || hoist(stream, &model));
    }

    /// An invariant multiply inside a loop moves to the preheader.
    #[test]
    fn an_invariant_computation_leaves_the_loop() {
        let mut stream = vec![
            ci("mov", &[("dst", "%v1"), ("src", "%v8")]),
            ci("mov", &[("dst", "%v2"), ("src", "%v9")]),
            ci("b", &[("target", "head")]),
            ci("label", &[("name", "head")]),
            ci("mul", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
            ci("cmp_imm", &[("lhs", "%v3"), ("rhs", "10")]),
            ci("b.lt", &[("target", "head")]),
            ci("ret", &[]),
        ];
        run(&mut stream, 3);
        assert_eq!(
            ops(&stream),
            vec![
                CodeOp::Mov,
                CodeOp::Mov,
                CodeOp::Mul,
                CodeOp::Branch,
                CodeOp::Label,
                CodeOp::CmpImm,
                CodeOp::BranchLt,
                CodeOp::Ret,
            ],
        );
    }

    /// A computation reading a value the loop itself redefines is not
    /// invariant and stays.
    #[test]
    fn a_loop_carried_computation_stays() {
        let stream = || {
            vec![
                ci("mov", &[("dst", "%v1"), ("src", "%v8")]),
                ci("b", &[("target", "head")]),
                ci("label", &[("name", "head")]),
                ci("add_imm", &[("dst", "%v1"), ("src", "%v1"), ("imm", "1")]),
                ci("mul", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v1")]),
                ci("cmp_imm", &[("lhs", "%v3"), ("rhs", "10")]),
                ci("b.lt", &[("target", "head")]),
                ci("ret", &[]),
            ]
        };
        let mut off = stream();
        run(&mut off, 3);
        assert_eq!(ops(&off), ops(&stream()));
    }

    /// A value invariant in both loops of a nest lands in the *outer*
    /// preheader, not merely outside the inner loop — the row's whole point.
    #[test]
    fn a_nest_invariant_lands_at_the_outer_level() {
        let mut stream = vec![
            ci("mov", &[("dst", "%v1"), ("src", "%v8")]),
            ci("mov", &[("dst", "%v2"), ("src", "%v9")]),
            ci("b", &[("target", "outer")]),
            ci("label", &[("name", "outer")]),
            ci("b", &[("target", "inner")]),
            ci("label", &[("name", "inner")]),
            ci("mul", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
            ci("cmp_imm", &[("lhs", "%v7"), ("rhs", "10")]),
            ci("b.lt", &[("target", "inner")]),
            ci("cmp_imm", &[("lhs", "%v6"), ("rhs", "10")]),
            ci("b.lt", &[("target", "outer")]),
            ci("ret", &[]),
        ];
        run(&mut stream, 3);
        // The multiply is now in the block that precedes the *outer* header.
        let position = stream
            .iter()
            .position(|i| i.op == CodeOp::Mul)
            .expect("the multiply survives");
        let outer_label = stream
            .iter()
            .position(|i| i.op == CodeOp::Label && i.get("name").as_deref() == Some("outer"))
            .expect("the outer header survives");
        assert!(
            position < outer_label,
            "hoisted past the outer header, not just the inner one"
        );
    }

    /// A loop with two entries has no preheader, so nothing is hoisted and no
    /// block is invented.
    #[test]
    fn a_loop_without_a_preheader_is_left_alone() {
        let stream = || {
            vec![
                ci("cmp_imm", &[("lhs", "%v9"), ("rhs", "0")]),
                ci("b.eq", &[("target", "head")]),
                ci("b", &[("target", "head")]),
                ci("label", &[("name", "head")]),
                ci("mul", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
                ci("cmp_imm", &[("lhs", "%v3"), ("rhs", "10")]),
                ci("b.lt", &[("target", "head")]),
                ci("ret", &[]),
            ]
        };
        let mut off = stream();
        run(&mut off, 3);
        assert_eq!(ops(&off), ops(&stream()));
    }

    /// A checked add is not in the whitelist: hoisting it would move a trap
    /// in front of a loop that might never run.
    #[test]
    fn a_trapping_operation_is_never_hoisted() {
        let stream = || {
            vec![
                ci("mov", &[("dst", "%v1"), ("src", "%v8")]),
                ci("mov", &[("dst", "%v2"), ("src", "%v9")]),
                ci("b", &[("target", "head")]),
                ci("label", &[("name", "head")]),
                ci("adds", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
                ci("b.vc", &[("target", "ok")]),
                ci("bl", &[("target", "_mfb_make_error_result")]),
                ci("label", &[("name", "ok")]),
                ci("cmp_imm", &[("lhs", "%v3"), ("rhs", "10")]),
                ci("b.lt", &[("target", "head")]),
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
                ci("mov", &[("dst", "%v1"), ("src", "%v8")]),
                ci("mov", &[("dst", "%v2"), ("src", "%v9")]),
                ci("b", &[("target", "head")]),
                ci("label", &[("name", "head")]),
                ci("mul", &[("dst", "%v3"), ("lhs", "%v1"), ("rhs", "%v2")]),
                ci("cmp_imm", &[("lhs", "%v3"), ("rhs", "10")]),
                ci("b.lt", &[("target", "head")]),
                ci("ret", &[]),
            ]
        };
        let mut off = stream();
        run(&mut off, 2);
        assert_eq!(ops(&off), ops(&stream()));
    }
}
