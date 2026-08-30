//! Check fusion with existing comparisons — a Level-3 Opt2 catalog row
//! (`planning/optimizations.md`): reuse a comparison the program already made
//! to discharge a check, instead of emitting a second one.
//!
//! MFB's safety checks are compare-and-branch pairs, and a guarded region
//! very often re-asks a question the guard just answered — the bounds guard
//! compares an index against a length, then the access re-compares the same
//! index against the same length; the division guard compares the divisor
//! against zero, then the modulo half compares it again. The *branch* is
//! genuinely needed each time (control has to go somewhere), but the `cmp`
//! that feeds it is not: the flags still hold exactly that comparison.
//!
//! So this row is availability, not folding. A forward walk over the CFG
//! tracks which comparison the condition flags currently reflect, and a `cmp`
//! that would re-establish an already-current one is deleted. The branch that
//! reads the flags is untouched and still decides what it always decided —
//! which is what makes this behavior-preserving even where the sibling
//! check-elision rows have no proof at all: nothing is assumed about the
//! *values*, only about which comparison the flags describe.
//!
//! **What clears the flags.** Only [`flag_preserving`] instructions carry the
//! fact forward — moves of a real register, immediate materialization, memory
//! traffic, labels and branches. Emphatically *not* the pure-ALU whitelist the
//! neighbouring rows use for purity: on x86-64 `and`/`orr`/`eor`/`add`/`sub`
//! all write EFLAGS, so treating them as transparent here would read a
//! comparison off flags some arithmetic had since overwritten. Branches do
//! preserve the fact (branching reads flags, it never sets them), which is
//! what lets a guard's comparison survive into the block it guards. A join
//! keeps the fact only when *every* predecessor arrives with the identical
//! one, and a back edge contributes nothing, so a loop header always starts
//! flagless.

use crate::arch::ops::CodeOp;
use crate::codegen::engine::regalloc::analysis::{
    build_cfg, classify_ref, is_block_terminator, ClassModel, RegRef,
};
use crate::codegen::engine::regalloc::{self};
use crate::codegen::engine::types::CodeInstruction;
use crate::target::shared::regmodel::RegisterModel;

use super::plans::mark::flag_preserving;
use super::plans::ranges::{self, reverse_postorder};
use super::plans::ssa::{self, Ssa, ValueId};

/// One side of a comparison, as the fact identifies it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Side {
    Value(ValueId),
    Literal(i64),
}

/// The comparison the flags currently reflect: `cmp lhs, rhs`, in that order
/// (the order decides every signed condition, so it is part of the identity).
type Fact = (Side, Side);

/// Run the check-fusion row over one function's selected stream, in place.
/// Self-guarded on the row's catalog level (3).
pub(crate) fn fuse(instructions: &mut Vec<CodeInstruction>, model: &dyn RegisterModel) {
    if !crate::optimizer::level_enabled(3) {
        return;
    }
    let models = regalloc::class_models(model);
    let blocks = build_cfg(instructions);
    if blocks.is_empty() {
        return;
    }
    let overlay = ssa::build(instructions, &blocks, &models);

    let comparison = |i: usize| -> Option<Fact> {
        match instructions[i].op {
            CodeOp::Cmp | CodeOp::CmpImm => Some((
                side(instructions, &models, &overlay, i, "lhs")?,
                side(instructions, &models, &overlay, i, "rhs")?,
            )),
            _ => None,
        }
    };

    // Forward availability over the CFG. One reverse-postorder sweep is
    // enough: the only edges it cannot see first are back edges, and those
    // contribute nothing anyway (a loop header must not inherit a fact from
    // its own body, which may have clobbered the flags).
    let mut entry: Vec<Option<Fact>> = vec![None; blocks.len()];
    let mut exit: Vec<Option<Fact>> = vec![None; blocks.len()];
    let mut computed = vec![false; blocks.len()];
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); blocks.len()];
    for (b, block) in blocks.iter().enumerate() {
        for &successor in &block.succ {
            preds[successor].push(b);
        }
    }

    let mut redundant: Vec<usize> = Vec::new();
    for &b in &reverse_postorder(&blocks) {
        // A block whose predecessors do not all agree — or are not all
        // computed yet — starts with no fact.
        let mut current = None;
        if !preds[b].is_empty() && preds[b].iter().all(|&p| computed[p]) {
            let first = exit[preds[b][0]];
            if preds[b].iter().all(|&p| exit[p] == first) {
                current = first;
            }
        }
        entry[b] = current;

        for i in blocks[b].start..blocks[b].end {
            if let Some(fact) = comparison(i) {
                if current == Some(fact) {
                    redundant.push(i);
                } else {
                    current = Some(fact);
                }
                continue;
            }
            // Branching reads the flags; it never writes them.
            if is_block_terminator(instructions[i].op) {
                continue;
            }
            if !flag_preserving(&instructions[i]) {
                current = None;
            }
        }
        exit[b] = current;
        computed[b] = true;
    }

    if redundant.is_empty() {
        return;
    }
    let fired = redundant.len() as u64;
    let mut keep = vec![true; instructions.len()];
    for index in redundant {
        keep[index] = false;
    }
    let mut index = 0;
    instructions.retain(|_| {
        let keep = keep[index];
        index += 1;
        keep
    });
    crate::optimizer::stats::count_check_fusions(fired);
}

/// Resolve one operand of a compare to the identity the fact keys on.
fn side(
    instructions: &[CodeInstruction],
    models: &(ClassModel, ClassModel),
    overlay: &Ssa,
    i: usize,
    field: &str,
) -> Option<Side> {
    let operand = instructions[i].operand(field)?;
    match classify_ref(operand, &models.0) {
        Some(RegRef::VReg(id)) => overlay.value_of_use(i, (false, id)).map(Side::Value),
        // A physical register's contents are an ABI effect this pass does not
        // track, so it can never be part of a fact.
        Some(RegRef::Phys(_)) => None,
        None => ranges::literal(&operand.rendered()).map(Side::Literal),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        with_opt_level(OptLevel(level), || fuse(stream, &model));
    }

    /// The guard's comparison is still current inside the block it guards, so
    /// the second `cmp` of the same pair goes and its branch reads the first
    /// one's flags.
    #[test]
    fn a_repeated_comparison_is_dropped() {
        let mut stream = vec![
            ci("cmp", &[("lhs", "%v1"), ("rhs", "%v2")]),
            ci("b.ge", &[("target", "out")]),
            ci("cmp", &[("lhs", "%v1"), ("rhs", "%v2")]),
            ci("b.lt", &[("target", "ok")]),
            ci("label", &[("name", "ok")]),
            ci("ret", &[]),
            ci("label", &[("name", "out")]),
            ci("ret", &[]),
        ];
        run(&mut stream, 3);
        assert_eq!(
            ops(&stream),
            vec![
                CodeOp::Cmp,
                CodeOp::BranchGe,
                CodeOp::BranchLt,
                CodeOp::Label,
                CodeOp::Ret,
                CodeOp::Label,
                CodeOp::Ret,
            ],
        );
    }

    /// A call between the two comparisons destroys the flags, so the second
    /// one is doing real work.
    #[test]
    fn a_call_between_them_keeps_the_second_comparison() {
        let stream = || {
            vec![
                ci("cmp", &[("lhs", "%v1"), ("rhs", "%v2")]),
                ci("b.ge", &[("target", "out")]),
                ci("bl", &[("target", "_helper")]),
                ci("cmp", &[("lhs", "%v1"), ("rhs", "%v2")]),
                ci("b.lt", &[("target", "ok")]),
                ci("label", &[("name", "ok")]),
                ci("ret", &[]),
                ci("label", &[("name", "out")]),
                ci("ret", &[]),
            ]
        };
        let mut off = stream();
        run(&mut off, 3);
        assert_eq!(ops(&off), ops(&stream()));
    }

    /// Operand order is part of the comparison's identity: `cmp a, b` does
    /// not establish `cmp b, a`.
    #[test]
    fn swapped_operands_are_a_different_comparison() {
        let stream = || {
            vec![
                ci("cmp", &[("lhs", "%v1"), ("rhs", "%v2")]),
                ci("b.ge", &[("target", "out")]),
                ci("cmp", &[("lhs", "%v2"), ("rhs", "%v1")]),
                ci("b.lt", &[("target", "ok")]),
                ci("label", &[("name", "ok")]),
                ci("ret", &[]),
                ci("label", &[("name", "out")]),
                ci("ret", &[]),
            ]
        };
        let mut off = stream();
        run(&mut off, 3);
        assert_eq!(ops(&off), ops(&stream()));
    }

    /// A join whose predecessors disagree starts flagless.
    #[test]
    fn a_join_does_not_inherit_a_disputed_fact() {
        let stream = || {
            vec![
                ci("cmp", &[("lhs", "%v1"), ("rhs", "%v2")]),
                ci("b.ge", &[("target", "other")]),
                ci("b", &[("target", "join")]),
                ci("label", &[("name", "other")]),
                ci("bl", &[("target", "_helper")]),
                ci("b", &[("target", "join")]),
                ci("label", &[("name", "join")]),
                ci("cmp", &[("lhs", "%v1"), ("rhs", "%v2")]),
                ci("b.lt", &[("target", "ok")]),
                ci("label", &[("name", "ok")]),
                ci("ret", &[]),
            ]
        };
        let mut off = stream();
        run(&mut off, 3);
        assert_eq!(
            ops(&off),
            ops(&stream()),
            "one arm called out, so the flags are not current at the join"
        );
    }

    /// The row is off below `-O3`.
    #[test]
    fn level_two_disables_the_row() {
        let stream = || {
            vec![
                ci("cmp", &[("lhs", "%v1"), ("rhs", "%v2")]),
                ci("b.ge", &[("target", "out")]),
                ci("cmp", &[("lhs", "%v1"), ("rhs", "%v2")]),
                ci("b.lt", &[("target", "ok")]),
                ci("label", &[("name", "ok")]),
                ci("ret", &[]),
                ci("label", &[("name", "out")]),
                ci("ret", &[]),
            ]
        };
        let mut off = stream();
        run(&mut off, 2);
        assert_eq!(ops(&off), ops(&stream()));
    }
}
