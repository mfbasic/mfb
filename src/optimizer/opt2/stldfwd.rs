//! Store-to-load forwarding — the Level-3 half of the catalog row
//! (`planning/optimizations.md`): replace a load whose slot provably still
//! holds a *stored* value with a copy of the storing register, across the
//! whole function.
//!
//! The Level-1 half already ships as the post-regalloc machine peephole
//! [`super::peephole::forward_stores_to_loads`] — block-local, physical
//! registers, adjacent `str`/`ldr` pairs. This row is the row's own named
//! broadening: it runs on the pre-regalloc stream over the allocator's CFG,
//! consuming the [`super::plans::memory`] availability dataflow, so a store
//! in one block forwards to a load in another whenever *every* path between
//! them leaves the slot untouched. Two guards make it behavior-preserving:
//! availability itself (any unmodeled instruction clears the whole state, so
//! a call, an FP op, or a non-`sp` memory access ends forwarding), and the
//! single-definition rule on the holder register (a multi-def register may no
//! longer hold the recorded value at the load — the same rule GVN uses).
//!
//! The rewrite is `ldr dst, [sp,#off]` → `mov dst, <holder>`: identical bits
//! in the destination, no memory traffic, nothing added or reordered. Copy
//! propagation then bypasses the `mov` and DCE sweeps whatever strands. The
//! full alias-analysis form — arbitrary bases, memory-SSA — remains Plan2
//! infrastructure per the catalog's "Prerequisites are not dial rows" note;
//! this row covers the `sp`-slot traffic the frame layout actually emits.

use crate::codegen::engine::regalloc;
use crate::codegen::engine::regalloc::analysis::build_cfg;
use crate::codegen::engine::types::CodeInstruction;
use crate::target::shared::regmodel::RegisterModel;

use super::lvn::copy_of;
use super::plans::memory::{forwardable_loads, Origin};
use super::plans::ssa;

/// Run **both** memory rows over one function's selected stream, in place:
/// this one and redundant load elimination (`opt2::rle`, whose module carries
/// its own documentation and tests). They ask the identical question of the
/// identical dataflow and differ only in the origin of the available value,
/// so they share one traversal — running the analysis (and its SSA overlay)
/// twice cost ~50s per `-O3` build of the giant generated functions for
/// nothing. Each origin still reports into its own row's counter, so `-v`
/// attributes the rewrites separately. Both rows are Level 3, so the single
/// guard here is each row's guard.
pub(crate) fn forward(instructions: &mut [CodeInstruction], model: &dyn RegisterModel) {
    if !crate::optimizer::level_enabled(3) {
        return;
    }
    let models = regalloc::class_models(model);
    let blocks = build_cfg(instructions);
    let overlay = ssa::build(instructions, &blocks, &models);

    let (mut stores, mut loads) = (0, 0);
    for candidate in forwardable_loads(instructions, &blocks, &models, &overlay) {
        let Some(dst) = instructions[candidate.inst].operand("dst").cloned() else {
            continue;
        };
        instructions[candidate.inst] = copy_of(dst, candidate.available.holder.clone());
        match candidate.available.origin {
            Origin::Store => stores += 1,
            Origin::Load => loads += 1,
        }
    }
    crate::optimizer::stats::count_stores_forwarded(stores);
    crate::optimizer::stats::count_redundant_loads_removed(loads);
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

    fn run(stream: &mut [CodeInstruction], level: u8) {
        with_opt_level(OptLevel(level), || forward(stream, &Aarch64RegisterModel));
    }

    /// The row's own broadening: the store and the load are in *different*
    /// blocks, which the Level-1 block-local peephole cannot connect.
    #[test]
    fn stores_forward_across_blocks() {
        let mut stream = vec![
            ci(
                "mov_imm",
                &[("dst", "%v1"), ("type", "Integer"), ("value", "5")],
            ),
            ci(
                "str_u64",
                &[("src", "%v1"), ("base", "sp"), ("offset", "8")],
            ),
            ci("b", &[("target", "next")]),
            ci("label", &[("name", "next")]),
            ci(
                "ldr_u64",
                &[("dst", "%v2"), ("base", "sp"), ("offset", "8")],
            ),
            ci(
                "str_u64",
                &[("src", "%v2"), ("base", "sp"), ("offset", "16")],
            ),
            ci("ret", &[]),
        ];
        run(&mut stream, 3);
        assert_eq!(stream[4].op, CodeOp::Mov, "the reload became a copy");
        assert_eq!(stream[4].get("src").as_deref(), Some("%v1"));
        assert_eq!(stream[4].get("dst").as_deref(), Some("%v2"));
    }

    /// A call between the store and the load ends availability (it may write
    /// the frame); so does an intervening store to the same slot from a
    /// register that is not single-def.
    #[test]
    fn calls_and_clobbers_stop_forwarding() {
        let mut stream = vec![
            ci(
                "mov_imm",
                &[("dst", "%v1"), ("type", "Integer"), ("value", "5")],
            ),
            ci(
                "str_u64",
                &[("src", "%v1"), ("base", "sp"), ("offset", "8")],
            ),
            ci("bl", &[("target", "_mfb_fn_callee")]),
            ci(
                "ldr_u64",
                &[("dst", "%v2"), ("base", "sp"), ("offset", "8")],
            ),
            ci("ret", &[]),
        ];
        run(&mut stream, 3);
        assert_eq!(stream[3].op, CodeOp::LdrU64, "the call cleared the slot");
    }

    /// Only one path stores the slot: the meet at the join keeps nothing.
    #[test]
    fn one_sided_stores_do_not_survive_a_join() {
        let mut stream = vec![
            ci(
                "mov_imm",
                &[("dst", "%v1"), ("type", "Integer"), ("value", "5")],
            ),
            ci("b.eq", &[("target", "join")]),
            ci(
                "str_u64",
                &[("src", "%v1"), ("base", "sp"), ("offset", "8")],
            ),
            ci("label", &[("name", "join")]),
            ci(
                "ldr_u64",
                &[("dst", "%v2"), ("base", "sp"), ("offset", "8")],
            ),
            ci("ret", &[]),
        ];
        run(&mut stream, 3);
        assert_eq!(stream[4].op, CodeOp::LdrU64, "not available on every path");
    }

    /// The row is off at `-O2` (it is a Level-3 row; the Level-1 machine
    /// peephole is a different row and keeps running).
    #[test]
    fn level_two_disables_the_row() {
        let mut stream = vec![
            ci(
                "mov_imm",
                &[("dst", "%v1"), ("type", "Integer"), ("value", "5")],
            ),
            ci(
                "str_u64",
                &[("src", "%v1"), ("base", "sp"), ("offset", "8")],
            ),
            ci("b", &[("target", "next")]),
            ci("label", &[("name", "next")]),
            ci(
                "ldr_u64",
                &[("dst", "%v2"), ("base", "sp"), ("offset", "8")],
            ),
            ci("ret", &[]),
        ];
        run(&mut stream, 2);
        assert_eq!(stream[4].op, CodeOp::LdrU64);
    }
}
