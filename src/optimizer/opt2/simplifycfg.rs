//! CFG simplification (simplifycfg) — a Level-2 Opt2 catalog row
//! (`planning/optimizations.md`): the umbrella cleanup that keeps the control
//! flow the other rows leave behind in canonical shape.
//!
//! The neighbouring rows each remove one *kind* of thing — branch folding
//! decides conditionals, jump threading collapses hop chains, unreachable
//! pruning drops orphaned blocks, block merging fuses straight-line pairs.
//! What none of them does is the small, purely structural tidying that their
//! output creates. This row is exactly that list, and nothing more:
//!
//! - **Redundant conditional branch** — a conditional branch whose taken
//!   target is also its fall-through block goes both ways to the same place,
//!   so it is deleted (flags are unread by falling through). This is the
//!   shape branch folding and threading produce when both edges converge.
//! - **Branch to a returning block** — an unconditional `b` whose target
//!   block is nothing but `ret` becomes the `ret` itself, removing the jump
//!   without duplicating anything (a bare return is the one tail small enough
//!   to always be worth inlining; the tail-duplication row's budget is about
//!   larger bodies).
//! - **Duplicate consecutive labels** — two labels in a row with nothing
//!   between them describe one point in the program; the second's references
//!   are retargeted at the first and the redundant label goes.
//!
//! Every rewrite here deletes or retargets control flow only where the two
//! paths provably reach the same instruction, so nothing about evaluation
//! order, effects, or traps changes. Runs after the other CFG rows so it sees
//! their output, and before block merging so the label it removes can let a
//! merge happen.

use std::collections::HashMap;

use crate::arch::ops::CodeOp;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::regalloc::analysis::{build_cfg, is_block_terminator};
use crate::codegen::engine::types::CodeInstruction;

use super::plans::mark::conditional_terminator;

/// Run the CFG-simplification row over one function's selected stream, in
/// place. Self-guarded on the row's catalog level (2).
pub(crate) fn simplify(instructions: &mut Vec<CodeInstruction>) {
    if !crate::optimizer::level_enabled(2) {
        return;
    }
    let mut fired = 0;

    // 1. Retarget references away from a duplicate consecutive label, then
    //    drop it. Done first so the two rewrites below see canonical labels.
    let mut alias: HashMap<String, String> = HashMap::new();
    for window in instructions.windows(2) {
        if window[0].op == CodeOp::Label && window[1].op == CodeOp::Label {
            if let (Some(first), Some(second)) = (window[0].get("name"), window[1].get("name")) {
                // Chase, so a run of three collapses onto the first.
                let canonical = alias.get(&first).cloned().unwrap_or(first);
                alias.insert(second, canonical);
            }
        }
    }
    if !alias.is_empty() {
        for instruction in instructions.iter_mut() {
            if instruction.op == CodeOp::Label {
                continue; // a label's own name is a definition
            }
            for (name, operand) in instruction.fields.iter_mut() {
                if *name != "target" {
                    continue;
                }
                if let Some(canonical) = alias.get(operand.rendered().as_ref()) {
                    *operand = Operand::from(canonical.clone());
                    fired += 1;
                }
            }
        }
        let aliased = alias;
        instructions.retain(|instruction| {
            !(instruction.op == CodeOp::Label
                && instruction
                    .get("name")
                    .is_some_and(|name| aliased.contains_key(&name)))
        });
    }

    // 2. A conditional branch whose target is also its fall-through: both
    //    edges land on the same block, so the branch decides nothing.
    let blocks = build_cfg(instructions);
    let mut drop_indices: Vec<usize> = Vec::new();
    for block in &blocks {
        let terminator = block.end - 1;
        if !conditional_terminator(instructions[terminator].op) || block.succ.len() < 2 {
            continue;
        }
        if block.succ[0] == block.succ[1] {
            drop_indices.push(terminator);
        }
    }

    // 3. A `b` to a block that only returns becomes the return.
    let mut returns: HashMap<String, CodeInstruction> = HashMap::new();
    for block in &blocks {
        let leader = &instructions[block.start];
        if leader.op != CodeOp::Label || block.end != block.start + 2 {
            continue;
        }
        let body = &instructions[block.start + 1];
        if body.op == CodeOp::Ret {
            if let Some(name) = leader.get("name") {
                returns.insert(name, body.clone());
            }
        }
    }

    let mut rewritten: Vec<(usize, CodeInstruction)> = Vec::new();
    if !returns.is_empty() {
        for (i, instruction) in instructions.iter().enumerate() {
            if instruction.op != CodeOp::Branch {
                continue;
            }
            if let Some(ret) = instruction
                .get("target")
                .and_then(|target| returns.get(&target))
            {
                rewritten.push((i, ret.clone()));
            }
        }
    }

    fired += (drop_indices.len() + rewritten.len()) as u64;
    for (i, replacement) in rewritten {
        instructions[i] = replacement;
    }
    if !drop_indices.is_empty() {
        let mut keep = vec![true; instructions.len()];
        for index in drop_indices {
            keep[index] = false;
        }
        let mut index = 0;
        instructions.retain(|_| {
            let keep = keep[index];
            index += 1;
            keep
        });
    }
    // A terminator-free tail can only arise if something above deleted the
    // last instruction of the stream; the block builder tolerates it, but the
    // assertion documents that this row never leaves one.
    debug_assert!(
        instructions
            .last()
            .is_none_or(|last| is_block_terminator(last.op) || last.op == CodeOp::Label),
        "simplifycfg left a stream whose last instruction is not a terminator",
    );
    crate::optimizer::stats::count_cfg_simplifications(fired);
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
        with_opt_level(OptLevel(level), || simplify(stream));
    }

    /// A conditional branch whose target is its own fall-through decides
    /// nothing and goes.
    #[test]
    fn branches_to_the_fallthrough_are_deleted() {
        let mut stream = vec![
            ci("cmp_imm", &[("lhs", "%v1"), ("rhs", "0")]),
            ci("b.eq", &[("target", "next")]),
            ci("label", &[("name", "next")]),
            ci("ret", &[]),
        ];
        run(&mut stream, 2);
        assert_eq!(
            ops(&stream),
            vec![CodeOp::CmpImm, CodeOp::Label, CodeOp::Ret]
        );
    }

    /// A jump to a return-only block becomes the return.
    #[test]
    fn branches_to_a_returning_block_become_returns() {
        let mut stream = vec![
            ci("b", &[("target", "done")]),
            ci("label", &[("name", "other")]),
            ci(
                "str_u64",
                &[("src", "%v1"), ("base", "sp"), ("offset", "8")],
            ),
            ci("label", &[("name", "done")]),
            ci("ret", &[]),
        ];
        run(&mut stream, 2);
        assert_eq!(stream[0].op, CodeOp::Ret, "the jump became the return");
    }

    /// Consecutive labels describe one point: references collapse onto the
    /// first and the duplicate goes. (The target block here does more than
    /// return, so the jump-to-return rewrite cannot also fire and the label
    /// collapse is what the assertion sees.)
    #[test]
    fn duplicate_labels_collapse() {
        let mut stream = vec![
            ci("b", &[("target", "second")]),
            ci("label", &[("name", "first")]),
            ci("label", &[("name", "second")]),
            ci(
                "str_u64",
                &[("src", "%v1"), ("base", "sp"), ("offset", "8")],
            ),
            ci("ret", &[]),
        ];
        run(&mut stream, 2);
        assert_eq!(
            ops(&stream),
            vec![CodeOp::Branch, CodeOp::Label, CodeOp::StrU64, CodeOp::Ret]
        );
        assert_eq!(stream[0].get("target").as_deref(), Some("first"));
        assert_eq!(stream[1].get("name").as_deref(), Some("first"));
    }

    /// The two rewrites compose: collapsing the labels exposes a
    /// return-only target, which then absorbs the jump.
    #[test]
    fn collapsed_labels_expose_a_returning_target() {
        let mut stream = vec![
            ci("b", &[("target", "second")]),
            ci("label", &[("name", "first")]),
            ci("label", &[("name", "second")]),
            ci("ret", &[]),
        ];
        run(&mut stream, 2);
        assert_eq!(ops(&stream), vec![CodeOp::Ret, CodeOp::Label, CodeOp::Ret]);
    }

    /// A genuine two-way branch is untouched.
    #[test]
    fn real_branches_survive() {
        let stream = || {
            vec![
                ci("cmp_imm", &[("lhs", "%v1"), ("rhs", "0")]),
                ci("b.eq", &[("target", "elsewhere")]),
                ci(
                    "str_u64",
                    &[("src", "%v1"), ("base", "sp"), ("offset", "8")],
                ),
                ci("label", &[("name", "elsewhere")]),
                ci("ret", &[]),
            ]
        };
        let mut off = stream();
        run(&mut off, 2);
        assert_eq!(ops(&off), ops(&stream()));
    }

    /// The row is off at `-O1`.
    #[test]
    fn level_one_disables_the_row() {
        let stream = || {
            vec![
                ci("cmp_imm", &[("lhs", "%v1"), ("rhs", "0")]),
                ci("b.eq", &[("target", "next")]),
                ci("label", &[("name", "next")]),
                ci("ret", &[]),
            ]
        };
        let mut off = stream();
        run(&mut off, 1);
        assert_eq!(ops(&off), ops(&stream()));
    }
}
