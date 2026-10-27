//! Condition-flag liveness (plan-171-B §4.2): for each instruction of a final,
//! selected stream, whether the condition flags are **dead after it** — no path
//! from the point after the instruction reaches a flag *reader* before a
//! definite flag *setter*.
//!
//! The compare-and-branch row (`opt2::cbranch`) asks it one question: after a
//! `cmp r, #0; b.eq L`, do the flags matter on either edge? Only if they do
//! not may the pair become `cbz r, L` (which sets no flags on AArch64) or a
//! bit test (whose x86-64 `test`/`bt` sets different ones). A missed reader is
//! a silently wrong branch, so every rule leans live:
//!
//! * **readers** — every flag-reading `b.<cond>` (`mir::is_flag_reading_branch`)
//!   and the x86-64 float branches `x86.j*` `select_x86` writes. A reader makes
//!   the flags live before it.
//! * **setters** — `cmp`, `cmp_imm`, `adds`, `subs`, `fcmp_d`, `fcmp_zero_d`,
//!   which write every flag a reader can read on every backend (AArch64 NZCV;
//!   x86-64 `cmp`/`add`/`sub`/`ucomisd`/`test` write ZF, SF, CF, OF and PF). A
//!   setter kills: the flags before it are not the ones after it.
//! * **everything else is transparent**, calls and `svc` included, even where
//!   an instruction really does write flags (x86-64 `and`, a macOS syscall's
//!   carry): treating a writer as transparent can only keep flags live longer.
//! * **block exits** — the union over the successors of the allocator's CFG
//!   (`regalloc::analysis::build_cfg`); `ret` and `branch_self` end dead; a
//!   branch whose target is not a label of this function (a tail branch out),
//!   or a stream that ends without a terminator, ends live.
//!
//! It is a pure analysis: nothing here rewrites the stream.

use crate::arch::ops::CodeOp;
use crate::codegen::engine::mir::is_flag_reading_branch;
use crate::codegen::engine::regalloc::analysis::{build_cfg, is_block_terminator};
use crate::codegen::engine::types::CodeInstruction;
use std::collections::HashSet;

/// What one instruction does to flag liveness, walking backwards.
fn step(live: bool, op: CodeOp) -> bool {
    if reads_flags(op) {
        true
    } else if sets_flags(op) {
        false
    } else {
        live
    }
}

/// Whether `op` reads the condition flags.
pub(crate) fn reads_flags(op: CodeOp) -> bool {
    is_flag_reading_branch(op)
        || matches!(
            op,
            CodeOp::X86Jae
                | CodeOp::X86Jp
                | CodeOp::X86Jnp
                | CodeOp::X86Ja
                | CodeOp::X86Jb
                | CodeOp::X86Jbe
                | CodeOp::X86Je
                | CodeOp::X86Jne
        )
}

/// Whether `op` writes every condition flag a reader reads, on every backend.
fn sets_flags(op: CodeOp) -> bool {
    matches!(
        op,
        CodeOp::Cmp
            | CodeOp::CmpImm
            | CodeOp::Adds
            | CodeOp::Subs
            | CodeOp::FCmpD
            | CodeOp::FCmpZeroD
    )
}

/// `dead[i]`: the condition flags are dead at the point after instruction `i`
/// on every path out of it.
pub(crate) fn dead_after(instructions: &[CodeInstruction]) -> Vec<bool> {
    let n = instructions.len();
    let blocks = build_cfg(instructions);
    let labels: HashSet<String> = instructions
        .iter()
        .filter(|instruction| instruction.op == CodeOp::Label)
        .filter_map(|instruction| instruction.get("name"))
        .collect();
    // A block that leaves through a branch to a label outside the function
    // keeps the flags live at its exit (nothing is known about the code there).
    let leaves_function: Vec<bool> = blocks
        .iter()
        .map(|block| {
            let last = &instructions[block.end - 1];
            if !is_block_terminator(last.op) {
                // Falling off the end of the stream: unknown code follows.
                return block.succ.is_empty();
            }
            !matches!(last.op, CodeOp::Ret | CodeOp::BranchSelf)
                && !last
                    .get("target")
                    .is_some_and(|target| labels.contains(&target))
        })
        .collect();
    let exit_live = |index: usize, live_in: &[bool]| {
        leaves_function[index] || blocks[index].succ.iter().any(|&s| live_in[s])
    };
    let mut live_in = vec![false; blocks.len()];
    let mut changed = true;
    while changed {
        changed = false;
        for index in (0..blocks.len()).rev() {
            let mut live = exit_live(index, &live_in);
            for i in (blocks[index].start..blocks[index].end).rev() {
                live = step(live, instructions[i].op);
            }
            if live != live_in[index] {
                live_in[index] = live;
                changed = true;
            }
        }
    }
    let mut dead = vec![false; n];
    for (index, block) in blocks.iter().enumerate() {
        let mut live = exit_live(index, &live_in);
        for i in (block.start..block.end).rev() {
            dead[i] = !live;
            live = step(live, instructions[i].op);
        }
    }
    dead
}

#[cfg(test)]
mod tests;
