//! Basic block merging — a Level-2 Opt2 catalog row
//! (`planning/optimizations.md`): fuse single-predecessor/single-successor
//! block pairs back into straight-line code. On a label-and-fallthrough
//! stream the two constituent rewrites are:
//!
//! 1. **Branch-to-next elimination.** A terminator whose `target` is the very
//!    next instruction's label transfers to where fallthrough already goes —
//!    for a conditional branch *both* outcomes land there — so the branch is
//!    deleted outright (flags are unread by fallthrough).
//! 2. **Unreferenced-label removal.** A label is a block leader; when no
//!    instruction field anywhere in the function names it, the boundary it
//!    creates separates a single-pred/single-succ fallthrough pair for no
//!    reason, and dropping it merges the blocks. The reference census is the
//!    same over-approximation `opt2::uce` uses (*any* field of *any*
//!    instruction, not just modeled branch targets), the label table is
//!    per-function at encode (`encode_plan` clears it between functions), so
//!    an in-stream census is complete — and the stream's leading label
//!    (index 0, the function entry) is kept unconditionally as a belt over
//!    those braces.
//!
//! Both rewrites delete only instructions that execute nothing (a label
//! emits no bytes; the branch's transfer equals fallthrough), so the row is
//! behavior-preserving by construction. Runs last in the seam: branch
//! folding, threading, and unreachable-block pruning are what strand the
//! branches-to-next and orphaned labels this pass fuses away.

use std::collections::HashSet;

use crate::arch::ops::CodeOp;
use crate::codegen::engine::regalloc::analysis::is_block_terminator;
use crate::codegen::engine::types::CodeInstruction;

/// Run the block-merging row over one function's selected stream, in place.
/// Self-guarded on the row's catalog level (2).
pub(crate) fn merge_blocks(instructions: &mut Vec<CodeInstruction>) {
    if !crate::optimizer::level_enabled(2) {
        return;
    }
    let mut keep = vec![true; instructions.len()];
    let mut merged = 0u64;

    // 1. Branch-to-next: the terminator's target is the label right after it.
    for (i, window) in instructions.windows(2).enumerate() {
        if !is_block_terminator(window[0].op) || window[1].op != CodeOp::Label {
            continue;
        }
        let (Some(target), Some(label)) = (window[0].get("target"), window[1].get("name")) else {
            continue;
        };
        if target == label {
            keep[i] = false;
            merged += 1;
        }
    }

    // 2. Unreferenced labels, censused over the surviving instructions (a
    // deleted branch-to-next no longer counts as a reference).
    let mut referenced: HashSet<String> = HashSet::new();
    for (i, instruction) in instructions.iter().enumerate() {
        if !keep[i] {
            continue;
        }
        for (name, _) in &instruction.fields {
            if instruction.op == CodeOp::Label && *name == "name" {
                continue; // a label's own name is a definition, not a reference
            }
            if let Some(value) = instruction.get(name) {
                referenced.insert(value);
            }
        }
    }
    for (i, instruction) in instructions.iter().enumerate().skip(1) {
        if instruction.op != CodeOp::Label {
            continue;
        }
        if let Some(label) = instruction.get("name") {
            if !referenced.contains(&label) {
                keep[i] = false;
                merged += 1;
            }
        }
    }

    if merged != 0 {
        let mut index = 0;
        instructions.retain(|_| {
            let keep = keep[index];
            index += 1;
            keep
        });
    }
    crate::optimizer::stats::count_blocks_merged(merged);
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
        with_opt_level(OptLevel(level), || merge_blocks(stream));
    }

    /// `b next; label next` fuses: the branch goes, and with its reference
    /// gone the label goes too — one straight-line block remains.
    #[test]
    fn branch_to_next_and_its_label_fuse_away() {
        let mut stream = vec![
            ci(
                "mov_imm",
                &[("dst", "%v1"), ("type", "Integer"), ("value", "1")],
            ),
            ci("b", &[("target", "next")]),
            ci("label", &[("name", "next")]),
            ci("ret", &[]),
        ];
        run(&mut stream, 2);
        assert_eq!(ops(&stream), vec![CodeOp::MovImm, CodeOp::Ret]);
    }

    /// A *conditional* branch to next is equally a no-op — both outcomes
    /// fall through — and is deleted; its label survives only if something
    /// else references it.
    #[test]
    fn conditional_branch_to_next_is_deleted() {
        let mut stream = vec![
            ci("cmp_imm", &[("lhs", "%v1"), ("rhs", "0")]),
            ci("b.eq", &[("target", "next")]),
            ci("label", &[("name", "next")]),
            ci("ret", &[]),
        ];
        run(&mut stream, 2);
        assert_eq!(ops(&stream), vec![CodeOp::CmpImm, CodeOp::Ret]);
    }

    /// A referenced label keeps its block boundary — even when the reference
    /// is from a field this pass has never heard of.
    #[test]
    fn referenced_labels_stay() {
        let mut stream = vec![
            ci("b.eq", &[("target", "join")]),
            ci("str_u64", &[("src", "x0"), ("base", "sp"), ("offset", "8")]),
            ci("label", &[("name", "join")]),
            ci("label", &[("name", "noted")]),
            ci(
                "mov_imm",
                &[("dst", "%v1"), ("type", "Integer"), ("value", "noted")],
            ),
            ci("ret", &[]),
        ];
        let before = ops(&stream);
        run(&mut stream, 2);
        assert_eq!(ops(&stream), before);
    }

    /// The stream's leading label (the function entry) is never removed,
    /// referenced or not.
    #[test]
    fn the_entry_label_is_kept() {
        let mut stream = vec![
            ci("label", &[("name", "entry")]),
            ci(
                "mov_imm",
                &[("dst", "%v1"), ("type", "Integer"), ("value", "1")],
            ),
            ci("ret", &[]),
        ];
        let before = ops(&stream);
        run(&mut stream, 2);
        assert_eq!(ops(&stream), before);
    }

    /// The row is off at `-O1`.
    #[test]
    fn level_one_disables_the_row() {
        let mut stream = vec![
            ci("b", &[("target", "next")]),
            ci("label", &[("name", "next")]),
            ci("ret", &[]),
        ];
        let before = ops(&stream);
        run(&mut stream, 1);
        assert_eq!(ops(&stream), before);
    }
}
