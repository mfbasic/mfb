//! Tail duplication — a Level-3 Opt2 catalog row
//! (`planning/optimizations.md`): copy a small join block into each of its
//! predecessors, so the straight-line rows downstream see one longer block
//! instead of a merge they must forget at.
//!
//! Every block-local pass in this seam — the constant folder, branch folding,
//! local value numbering, the availability dataflows' meet — loses its facts
//! at a join, because another path may have left different values in the same
//! registers. Duplicating a *small* join into its predecessors removes the
//! merge for those copies: each predecessor's facts now flow straight through
//! the duplicated tail. It is the classic enabler transform; it never removes
//! anything itself.
//!
//! The transform is exact duplication, so it needs no purity or trap-freedom
//! at all: each path executes the identical instructions in the identical
//! order, just from its own copy. What it does need is that duplication be
//! *legal* and *bounded*:
//!
//! - the join's predecessors must all reach it by an **unconditional** branch
//!   (a fallthrough predecessor would need its own copy left in place, and a
//!   conditional one would need its not-taken edge rewritten);
//! - the join must not be its own predecessor, and must be entered only by
//!   branches this pass can see — its label may be named by nothing else, or
//!   the copies would diverge from a path still jumping to the original;
//! - the join's body must be **terminator-final and self-contained**: a
//!   trailing `ret` or unconditional `b`, no labels inside, and small
//!   (`BODY_CAP`), so the code growth is bounded;
//! - MIR virtual registers are function-wide, and a duplicated definition is
//!   simply a second definition of that register on a different path — which
//!   every analysis in this seam already models (the SSA overlay places a phi;
//!   the multi-def rules in GVN and the memory rows refuse to reuse such a
//!   register). Nothing is renamed, so nothing can be mis-renamed.
//!
//! Once every predecessor carries its own copy, the original join is left
//! unreferenced and the unreachable-block row prunes it.
//!
//! Growth is budgeted per function (see `GROWTH_FLOOR`/`GROWTH_PERCENT`):
//! this is the only row in the seam that *adds* instructions, and on a giant
//! generated function unbudgeted duplication slows every later pass and the
//! register allocator far more than it saves.

use std::collections::HashMap;

use crate::arch::ops::CodeOp;
use crate::codegen::engine::regalloc::analysis::{
    build_cfg, is_block_terminator, is_unconditional_terminator,
};
use crate::codegen::engine::types::CodeInstruction;

/// Join bodies above this size are not worth copying into every predecessor.
const BODY_CAP: usize = 8;

/// Instructions this row may add to one function, as a fraction of that
/// function's length (with a floor so small functions are still served).
/// Duplication is the one row here that *grows* code, and growth costs every
/// later pass and the register allocator: unbudgeted, it took a `-O3` build of
/// `examples/browser/app` from ~1 minute to nearly 11. The budget is reported
/// through the row's `-v` counter — a function that hits it simply duplicates
/// fewer tails, never a silently different program.
const GROWTH_FLOOR: usize = 64;
const GROWTH_PERCENT: usize = 2;

/// Run the tail-duplication row over one function's selected stream, in
/// place. Self-guarded on the row's catalog level (3).
pub(crate) fn duplicate(instructions: &mut Vec<CodeInstruction>) {
    if !crate::optimizer::level_enabled(3) {
        return;
    }
    let blocks = build_cfg(instructions);
    if blocks.is_empty() {
        crate::optimizer::stats::count_tails_duplicated(0);
        return;
    }

    // How often each label is named anywhere in the stream, by any field —
    // the same over-approximation `opt2::uce` uses. A join whose label is
    // referenced by something other than the branches we are about to rewrite
    // must keep its original block, so it is not a candidate.
    // Both maps are built in ONE pass: rescanning the stream per candidate
    // block instead made this row quadratic (blocks x instructions), which on
    // the giant generated functions cost minutes.
    let mut references: HashMap<String, usize> = HashMap::new();
    let mut branches_to: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, instruction) in instructions.iter().enumerate() {
        for (name, operand) in &instruction.fields {
            if instruction.op == CodeOp::Label && *name == "name" {
                continue; // a label's own name is a definition, not a reference
            }
            let value = operand.rendered();
            if let Some(count) = references.get_mut(value.as_ref()) {
                *count += 1;
            } else {
                references.insert(value.clone().into_owned(), 1);
            }
            if instruction.op == CodeOp::Branch && *name == "target" {
                branches_to
                    .entry(value.into_owned())
                    .or_default()
                    .push(index);
            }
        }
    }

    // Candidate joins: label-led, terminator-final, small, and entered only by
    // unconditional branches.
    let mut plan: Vec<(String, Vec<CodeInstruction>)> = Vec::new();
    for block in &blocks {
        let leader = &instructions[block.start];
        if leader.op != CodeOp::Label {
            continue;
        }
        let Some(label) = leader.get("name") else {
            continue;
        };
        let body: &[CodeInstruction] = &instructions[block.start + 1..block.end];
        if body.is_empty() || body.len() > BODY_CAP {
            continue;
        }
        // The tail must end in an UNCONDITIONAL terminator (`ret`/`b`). A
        // conditional branch has a second, implicit successor — its
        // fall-through — and inlining the body at a branch site silently
        // re-points that edge at whatever follows the branch instead of what
        // followed the original tail. (Accepting any terminator here made a
        // Set dedupe wrongly at -O3: pointlen=3 for a 2-element set.)
        if !is_unconditional_terminator(body[body.len() - 1].op) {
            continue;
        }
        if body[..body.len() - 1].iter().any(|instruction| {
            is_block_terminator(instruction.op) || instruction.op == CodeOp::Label
        }) {
            continue;
        }
        // Every branch that names this label must be an unconditional `b`, and
        // the label must be named by nothing else at all.
        let branches = branches_to.get(&label).map(Vec::as_slice).unwrap_or(&[]);
        if branches.len() < 2 || references.get(&label).copied().unwrap_or(0) != branches.len() {
            continue;
        }
        // A self-referencing tail (a loop) must not be unrolled here.
        if branches
            .iter()
            .any(|&index| index >= block.start && index < block.end)
        {
            continue;
        }
        plan.push((label, body.to_vec()));
    }
    if plan.is_empty() {
        crate::optimizer::stats::count_tails_duplicated(0);
        return;
    }

    // Apply within the growth budget: every `b <join>` becomes the join's body
    // inline until the budget is spent, after which the remaining branches are
    // left as they are. The original join block stays and becomes unreferenced
    // — the unreachable-block row prunes it.
    let mut budget = GROWTH_FLOOR.max(instructions.len() * GROWTH_PERCENT / 100);
    let tails: HashMap<String, Vec<CodeInstruction>> = plan.into_iter().collect();
    let mut rebuilt: Vec<CodeInstruction> = Vec::with_capacity(instructions.len());
    let mut duplicated = 0;
    for instruction in std::mem::take(instructions) {
        let tail = (instruction.op == CodeOp::Branch)
            .then(|| instruction.get("target"))
            .flatten()
            .and_then(|target| tails.get(&target))
            // The copy replaces the branch, so it adds `len - 1` instructions.
            .filter(|body| body.len().saturating_sub(1) <= budget);
        match tail {
            Some(body) => {
                budget -= body.len().saturating_sub(1);
                rebuilt.extend(body.iter().cloned());
                duplicated += 1;
            }
            None => rebuilt.push(instruction),
        }
    }
    *instructions = rebuilt;
    crate::optimizer::stats::count_tails_duplicated(duplicated);
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
        with_opt_level(OptLevel(level), || duplicate(stream));
    }

    /// Two predecessors branching to a small returning tail each get their own
    /// copy, so neither path passes through a join any more.
    #[test]
    fn small_returning_tails_duplicate_into_predecessors() {
        let mut stream = vec![
            /* 0 */ ci("b.eq", &[("target", "second")]),
            /* 1 */
            ci(
                "mov_imm",
                &[("dst", "%v1"), ("type", "Integer"), ("value", "1")],
            ),
            /* 2 */ ci("b", &[("target", "tail")]),
            /* 3 */ ci("label", &[("name", "second")]),
            /* 4 */
            ci(
                "mov_imm",
                &[("dst", "%v1"), ("type", "Integer"), ("value", "2")],
            ),
            /* 5 */ ci("b", &[("target", "tail")]),
            /* 6 */ ci("label", &[("name", "tail")]),
            /* 7 */
            ci(
                "str_u64",
                &[("src", "%v1"), ("base", "sp"), ("offset", "8")],
            ),
            /* 8 */ ci("ret", &[]),
        ];
        run(&mut stream, 3);
        // Each `b tail` became `str; ret`; the original tail block stays for
        // the unreachable-block row to prune.
        assert_eq!(
            ops(&stream),
            vec![
                CodeOp::BranchEq,
                CodeOp::MovImm,
                CodeOp::StrU64,
                CodeOp::Ret,
                CodeOp::Label,
                CodeOp::MovImm,
                CodeOp::StrU64,
                CodeOp::Ret,
                CodeOp::Label,
                CodeOp::StrU64,
                CodeOp::Ret,
            ]
        );
    }

    /// A label reached by anything other than the unconditional branches this
    /// pass rewrites — a conditional branch, or a fallthrough predecessor —
    /// is not duplicated.
    #[test]
    fn other_entries_block_duplication() {
        let conditional = || {
            vec![
                ci("b.eq", &[("target", "tail")]),
                ci("b", &[("target", "tail")]),
                ci("label", &[("name", "tail")]),
                ci("ret", &[]),
            ]
        };
        let mut stream = conditional();
        run(&mut stream, 3);
        assert_eq!(ops(&stream), ops(&conditional()), "conditional entry stays");

        // Fallthrough into the label: only one `b` names it, so the two-branch
        // requirement already refuses it.
        let fallthrough = || {
            vec![
                ci(
                    "mov_imm",
                    &[("dst", "%v1"), ("type", "Integer"), ("value", "1")],
                ),
                ci("label", &[("name", "tail")]),
                ci("ret", &[]),
            ]
        };
        let mut stream = fallthrough();
        run(&mut stream, 3);
        assert_eq!(ops(&stream), ops(&fallthrough()));
    }

    /// A tail bigger than the cap, or one that is not terminator-final, is
    /// left alone (bounded growth; a complete tail only).
    #[test]
    fn oversized_or_open_ended_tails_are_left_alone() {
        let big = || {
            let mut stream = vec![
                ci("b", &[("target", "tail")]),
                ci("b", &[("target", "tail")]),
                ci("label", &[("name", "tail")]),
            ];
            for _ in 0..BODY_CAP + 1 {
                stream.push(ci(
                    "mov_imm",
                    &[("dst", "%v1"), ("type", "Integer"), ("value", "1")],
                ));
            }
            stream.push(ci("ret", &[]));
            stream
        };
        let mut stream = big();
        run(&mut stream, 3);
        assert_eq!(ops(&stream), ops(&big()), "over the growth cap");

        // Open-ended: the block falls through to the next label rather than
        // ending in a terminator, so the copy would not be a complete tail.
        let open = || {
            vec![
                ci("b", &[("target", "tail")]),
                ci("b", &[("target", "tail")]),
                ci("label", &[("name", "tail")]),
                ci(
                    "mov_imm",
                    &[("dst", "%v1"), ("type", "Integer"), ("value", "1")],
                ),
                ci("label", &[("name", "after")]),
                ci("ret", &[]),
            ]
        };
        let mut stream = open();
        run(&mut stream, 3);
        assert_eq!(ops(&stream), ops(&open()));
    }

    /// A tail ending in a CONDITIONAL branch is never duplicated: its
    /// fall-through is a second successor, and inlining the body at a branch
    /// site would re-point that edge at whatever follows the branch. (This
    /// miscompiled a Set dedupe at -O3 — pointlen=3 for a 2-element set —
    /// until the check required an unconditional terminator.)
    #[test]
    fn conditionally_terminated_tails_are_never_duplicated() {
        let stream = || {
            vec![
                ci("b", &[("target", "tail")]),
                ci("b", &[("target", "tail")]),
                ci("label", &[("name", "tail")]),
                ci("cmp_imm", &[("lhs", "%v1"), ("rhs", "0")]),
                ci("b.eq", &[("target", "elsewhere")]),
                ci("label", &[("name", "elsewhere")]),
                ci("ret", &[]),
            ]
        };
        let mut off = stream();
        run(&mut off, 3);
        assert_eq!(
            ops(&off),
            ops(&stream()),
            "the fall-through edge must not move"
        );
    }

    /// A tail that branches back to itself is a loop, never a duplication
    /// candidate.
    #[test]
    fn self_looping_tails_are_not_unrolled() {
        let loop_tail = || {
            vec![
                ci("b", &[("target", "tail")]),
                ci("label", &[("name", "tail")]),
                ci(
                    "mov_imm",
                    &[("dst", "%v1"), ("type", "Integer"), ("value", "1")],
                ),
                ci("b", &[("target", "tail")]),
            ]
        };
        let mut stream = loop_tail();
        run(&mut stream, 3);
        assert_eq!(ops(&stream), ops(&loop_tail()));
    }

    /// The row is off at `-O2` (it is a Level-3 row).
    #[test]
    fn level_two_disables_the_row() {
        let stream = || {
            vec![
                ci("b", &[("target", "tail")]),
                ci("b", &[("target", "tail")]),
                ci("label", &[("name", "tail")]),
                ci("ret", &[]),
            ]
        };
        let mut off = stream();
        run(&mut off, 2);
        assert_eq!(ops(&off), ops(&stream()));
    }
}
