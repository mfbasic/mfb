//! Aggressive DCE (ADCE) — a Level-3 Opt2 catalog row
//! (`planning/optimizations.md`): control-dependence-based "assume dead, prove
//! live" elimination that also removes the dead *control structure* plain DCE
//! leaves behind. This is the row's proof-gated L3 form: only provably pure,
//! trap-free code is ever removed — a trap-capable region keeps its guarding
//! branch alive automatically, because the raise path's call/store
//! instructions are live seeds and live code keeps every branch it is
//! control-dependent on ([`plans::mark`]).
//!
//! Facts come from [`plans::postdom`] over the allocator's own CFG. When a
//! conditional branch survives marking as dead, every path from its block to
//! its immediate postdominator carries only dead-swept instructions, labels,
//! and intra-region unconditional branches (anything live between them would
//! be control-dependent on it, and a block on *all* such paths would itself
//! postdominate the branch) — so simply deleting the branch and falling
//! through executes nothing observable before rejoining, on every input.
//! Functions whose CFG yields no postdominance facts (an infinite loop) are
//! skipped whole.
//!
//! Runs after the plain-DCE row: at `-O3` both rows fire and this one reports
//! only its additional removals.

use crate::codegen::engine::regalloc;
use crate::codegen::engine::regalloc::analysis::build_cfg;
use crate::codegen::engine::types::CodeInstruction;
use crate::target::shared::regmodel::RegisterModel;

use super::plans::{mark, postdom, ssa};

/// Run the ADCE row over one function's selected stream, in place.
/// Self-guarded on the row's catalog level (3).
pub(crate) fn eliminate(instructions: &mut Vec<CodeInstruction>, model: &dyn RegisterModel) {
    if !crate::optimizer::level_enabled(3) {
        return;
    }
    let blocks = build_cfg(instructions);
    let Some(postdom) = postdom::compute(&blocks) else {
        return; // no postdominance facts (e.g. an infinite loop): skip
    };
    let models = regalloc::class_models(model);
    let ssa = ssa::build(instructions, &blocks, &models);
    let marking = mark::mark_live(instructions, &models, Some((&blocks, &postdom)), Some(&ssa));
    let removed = super::dce::sweep(instructions, &marking.live);
    crate::optimizer::stats::count_aggressive_dce(removed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch::aarch64::regmodel::Aarch64RegisterModel;
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
        with_opt_level(OptLevel(level), || eliminate(stream, &Aarch64RegisterModel));
    }

    /// A branch that only skips dead pure code dies with the code it guarded:
    /// the whole `b.eq` + dead arm collapses, leaving the label and the live
    /// tail. Plain DCE (which seeds branches live) removes only the arm body.
    #[test]
    fn dead_guarded_region_collapses_including_the_branch() {
        let mut stream = vec![
            ci("b.eq", &[("target", "join")]),
            ci(
                "mov_imm",
                &[("dst", "%v9"), ("type", "Integer"), ("value", "1")],
            ),
            ci("label", &[("name", "join")]),
            ci("ret", &[]),
        ];
        run(&mut stream, 3);
        assert_eq!(
            ops(&stream),
            vec![CodeOp::Label, CodeOp::Ret],
            "branch and dead arm both removed"
        );
    }

    /// A live instruction in the guarded region keeps the branch: control
    /// dependence marks the `b.eq` live because the store depends on it.
    #[test]
    fn live_guarded_region_keeps_the_branch() {
        let mut stream = vec![
            ci("b.eq", &[("target", "join")]),
            ci("str_u64", &[("src", "x0"), ("base", "sp"), ("offset", "8")]),
            ci("label", &[("name", "join")]),
            ci("ret", &[]),
        ];
        let before = ops(&stream);
        run(&mut stream, 3);
        assert_eq!(ops(&stream), before);
    }

    /// The trap shape: `adds` + `b.vs` guarding an error-raise call. The call
    /// is a live seed, the raise block is control-dependent on the `b.vs`, so
    /// the whole checked-arithmetic skeleton survives even though the value
    /// result is unused.
    #[test]
    fn trap_capable_region_is_never_removed() {
        let mut stream = vec![
            ci("adds", &[("dst", "%v1"), ("lhs", "%v2"), ("rhs", "%v3")]),
            ci("b.vc", &[("target", "ok")]),
            ci("bl", &[("target", "_raise")]),
            ci("label", &[("name", "ok")]),
            ci("ret", &[]),
        ];
        let before = ops(&stream);
        run(&mut stream, 3);
        assert_eq!(ops(&stream), before);
    }

    /// An infinite loop yields no postdominance facts: the function is skipped
    /// untouched rather than transformed on missing information.
    #[test]
    fn infinite_loop_functions_are_skipped() {
        let mut stream = vec![
            ci("label", &[("name", "spin")]),
            ci(
                "mov_imm",
                &[("dst", "%v1"), ("type", "Integer"), ("value", "1")],
            ),
            ci("b", &[("target", "spin")]),
        ];
        let before = ops(&stream);
        run(&mut stream, 3);
        assert_eq!(ops(&stream), before);
    }

    /// The row is off at `-O2`.
    #[test]
    fn level_two_disables_the_row() {
        let mut stream = vec![
            ci("b.eq", &[("target", "join")]),
            ci(
                "mov_imm",
                &[("dst", "%v9"), ("type", "Integer"), ("value", "1")],
            ),
            ci("label", &[("name", "join")]),
            ci("ret", &[]),
        ];
        let before = ops(&stream);
        run(&mut stream, 2);
        assert_eq!(ops(&stream), before);
    }
}
