//! Unreachable code elimination — the Opt2 (CFG) half of the Level-2 catalog
//! row (`planning/optimizations.md`): prune basic blocks control flow can
//! never reach from the selected pre-regalloc stream. The tree half lives in
//! `opt1::uce`; both feed one "Unreachable code elimination" `-v` count.
//!
//! No trap gate applies: an unreachable block never executes, so even
//! trap-capable instructions in it are removable. Reachability runs over the
//! allocator's own CFG with a deliberately over-approximated root set — block
//! 0 plus **every block whose label name is referenced by any field of any
//! instruction in the function** (not just modeled branch targets), so an op
//! this pass has never heard of that names a label keeps its target alive.
//! Only blocks that are neither fallthrough/branch-reachable from a root nor
//! label-referenced anywhere are pruned, labels included (provably, nothing
//! names them).

use std::collections::HashSet;

use crate::arch::ops::CodeOp;
use crate::codegen::engine::regalloc::analysis::build_cfg;
use crate::codegen::engine::types::CodeInstruction;

/// Run the Opt2 UCE row over one function's selected stream, in place.
/// Self-guarded on the row's catalog level (2).
pub(crate) fn eliminate(instructions: &mut Vec<CodeInstruction>) {
    if !crate::optimizer::level_enabled(2) {
        return;
    }
    let blocks = build_cfg(instructions);
    if blocks.is_empty() {
        return;
    }
    // Every label name referenced by any instruction field, from any op.
    let mut referenced: HashSet<String> = HashSet::new();
    for instruction in instructions.iter() {
        for (name, _) in &instruction.fields {
            if instruction.op == CodeOp::Label && *name == "name" {
                continue; // a label's own name is a definition, not a reference
            }
            if let Some(value) = instruction.get(name) {
                referenced.insert(value);
            }
        }
    }
    // Roots: entry + any block whose label is referenced anywhere.
    let mut reachable = vec![false; blocks.len()];
    let mut queue: Vec<usize> = vec![0];
    for (index, block) in blocks.iter().enumerate() {
        let leader = &instructions[block.start];
        if leader.op == CodeOp::Label {
            if let Some(label) = leader.get("name") {
                if referenced.contains(&label) {
                    queue.push(index);
                }
            }
        }
    }
    while let Some(block) = queue.pop() {
        if reachable[block] {
            continue;
        }
        reachable[block] = true;
        for &successor in &blocks[block].succ {
            if !reachable[successor] {
                queue.push(successor);
            }
        }
    }
    let mut keep = vec![true; instructions.len()];
    for (index, block) in blocks.iter().enumerate() {
        if !reachable[index] {
            for slot in &mut keep[block.start..block.end] {
                *slot = false;
            }
        }
    }
    let removed = keep.iter().filter(|keep| !**keep).count() as u64;
    if removed != 0 {
        let mut index = 0;
        instructions.retain(|_| {
            let keep = keep[index];
            index += 1;
            keep
        });
    }
    crate::optimizer::stats::count_unreachable_eliminations(removed);
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

    /// Code between an unconditional terminator and the next *referenced*
    /// label is unreachable and dies — trap-capable instructions included
    /// (they can never execute).
    #[test]
    fn unreferenced_blocks_after_a_terminator_die() {
        let mut stream = vec![
            ci("b", &[("target", "end")]),
            // Unreachable: nothing references `orphan`, no fallthrough path.
            ci("label", &[("name", "orphan")]),
            ci("adds", &[("dst", "%v1"), ("lhs", "%v2"), ("rhs", "%v3")]),
            ci("b", &[("target", "end")]),
            ci("label", &[("name", "end")]),
            ci("ret", &[]),
        ];
        with_opt_level(OptLevel(2), || eliminate(&mut stream));
        assert_eq!(
            ops(&stream),
            vec![CodeOp::Branch, CodeOp::Label, CodeOp::Ret],
            "the orphan block dies whole"
        );
    }

    /// A label referenced by *any* instruction field keeps its block, even
    /// when no modeled CFG edge reaches it — the over-approximated root set.
    #[test]
    fn any_reference_keeps_a_block() {
        let mut stream = vec![
            ci("b", &[("target", "end")]),
            ci("label", &[("name", "kept")]),
            ci("ret", &[]),
            ci("label", &[("name", "end")]),
            // Reference from an arbitrary (non-branch) op field.
            ci(
                "mov_imm",
                &[("dst", "%v1"), ("type", "Integer"), ("value", "kept")],
            ),
            ci("ret", &[]),
        ];
        let before = ops(&stream);
        with_opt_level(OptLevel(2), || eliminate(&mut stream));
        assert_eq!(ops(&stream), before);
    }

    /// Conditional-branch fallthrough keeps the next block reachable.
    #[test]
    fn fallthrough_blocks_stay() {
        let mut stream = vec![
            ci("b.eq", &[("target", "end")]),
            ci("str_u64", &[("src", "x0"), ("base", "sp"), ("offset", "8")]),
            ci("label", &[("name", "end")]),
            ci("ret", &[]),
        ];
        let before = ops(&stream);
        with_opt_level(OptLevel(2), || eliminate(&mut stream));
        assert_eq!(ops(&stream), before);
    }

    /// The row is off at `-O1`.
    #[test]
    fn level_one_disables_the_row() {
        let mut stream = vec![
            ci("b", &[("target", "end")]),
            ci("label", &[("name", "orphan")]),
            ci("ret", &[]),
            ci("label", &[("name", "end")]),
            ci("ret", &[]),
        ];
        let before = ops(&stream);
        with_opt_level(OptLevel(1), || eliminate(&mut stream));
        assert_eq!(ops(&stream), before);
    }
}
