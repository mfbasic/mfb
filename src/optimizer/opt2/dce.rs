//! Dead-code elimination — the Opt2 half of the Level-2 catalog row
//! (`planning/optimizations.md`): remove pure instructions whose results are
//! provably unused from the selected pre-regalloc stream. The tree-level half
//! lives in `opt1::dce`; both feed one "Dead-code elimination (DCE)" `-v`
//! count.
//!
//! [`plans::mark`] does the work with control flow untouched (conditional
//! branches seeded live): what remains dead is exactly the whitelisted pure
//! ALU with unused virtual destinations — for example the `mov_imm` feeders
//! the Opt2 constant folder strands. Trap preservation is structural: checked
//! arithmetic lowers to flag-setting ops outside the whitelist, so it and its
//! error paths are always live seeds. (The catalog row's "precise SSA-based"
//! form waits on Plan2 SSA; this def-use marking keeps every definition of a
//! used vreg, which is the sound non-SSA approximation.)

use crate::codegen::engine::regalloc;
use crate::codegen::engine::types::CodeInstruction;
use crate::target::shared::regmodel::RegisterModel;

use super::plans::mark;

/// Run the Opt2 DCE row over one function's selected stream, in place.
/// Self-guarded on the row's catalog level (2).
pub(crate) fn eliminate(instructions: &mut Vec<CodeInstruction>, model: &dyn RegisterModel) {
    if !crate::optimizer::level_enabled(2) {
        return;
    }
    let models = regalloc::class_models(model);
    let marking = mark::mark_live(instructions, &models, None);
    let removed = sweep(instructions, &marking.live);
    crate::optimizer::stats::count_dead_code_eliminations(removed);
}

/// Drop every non-live instruction, returning how many went.
pub(super) fn sweep(instructions: &mut Vec<CodeInstruction>, live: &[bool]) -> u64 {
    let before = instructions.len();
    let mut index = 0;
    instructions.retain(|_| {
        let keep = live[index];
        index += 1;
        keep
    });
    (before - instructions.len()) as u64
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

    /// Dead pure chains die transitively (the folder's stranded `mov_imm`
    /// feeders); used values, physical destinations, and unmodeled ops stay.
    #[test]
    fn dead_chains_die_and_effects_stay() {
        let mut stream = vec![
            // Dead chain: %1 feeds %2 feeds nothing live.
            ci(
                "mov_imm",
                &[("dst", "%v1"), ("type", "Integer"), ("value", "2")],
            ),
            ci("add_imm", &[("dst", "%v2"), ("src", "%v1"), ("imm", "1")]),
            // Live: feeds the store below.
            ci(
                "mov_imm",
                &[("dst", "%v3"), ("type", "Integer"), ("value", "7")],
            ),
            // Effects: a store (unmodeled) and a physical-dst move.
            ci(
                "str_u64",
                &[("src", "%v3"), ("base", "sp"), ("offset", "8")],
            ),
            ci(
                "mov_imm",
                &[("dst", "x0"), ("type", "Integer"), ("value", "0")],
            ),
            ci("ret", &[]),
        ];
        with_opt_level(OptLevel(2), || {
            eliminate(&mut stream, &Aarch64RegisterModel)
        });
        assert_eq!(
            ops(&stream),
            vec![CodeOp::MovImm, CodeOp::StrU64, CodeOp::MovImm, CodeOp::Ret],
            "the %v1/%v2 chain dies; %v3, the store, the x0 def, and ret stay"
        );
        assert_eq!(stream[0].get("dst").as_deref(), Some("%v3"));
    }

    /// Flag-setters are never candidates — checked arithmetic and its error
    /// paths survive even when the value result is unused.
    #[test]
    fn flag_setting_checked_arithmetic_stays() {
        let mut stream = vec![
            ci("adds", &[("dst", "%v1"), ("lhs", "%v2"), ("rhs", "%v3")]),
            ci("b.vc", &[("target", "ok")]),
            ci("bl", &[("target", "_raise")]),
            ci("label", &[("name", "ok")]),
            ci("ret", &[]),
        ];
        let before = ops(&stream);
        with_opt_level(OptLevel(2), || {
            eliminate(&mut stream, &Aarch64RegisterModel)
        });
        assert_eq!(ops(&stream), before);
    }

    /// The row is off at `-O1`.
    #[test]
    fn level_one_disables_the_row() {
        let mut stream = vec![ci(
            "mov_imm",
            &[("dst", "%v1"), ("type", "Integer"), ("value", "2")],
        )];
        with_opt_level(OptLevel(1), || {
            eliminate(&mut stream, &Aarch64RegisterModel)
        });
        assert_eq!(stream.len(), 1);
    }
}
