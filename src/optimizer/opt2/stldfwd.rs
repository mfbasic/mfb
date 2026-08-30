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
pub(crate) fn forward(instructions: &mut Vec<CodeInstruction>, model: &dyn RegisterModel) {
    if !crate::optimizer::level_enabled(3) {
        return;
    }
    let models = regalloc::class_models(model);
    let blocks = build_cfg(instructions);
    let overlay = ssa::build(instructions, &blocks, &models);

    let (mut stores, mut loads) = (0, 0);
    let (forwardable, partial) = forwardable_loads(instructions, &blocks, &models, &overlay);
    for candidate in forwardable {
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

    // The Store PRE / Load PRE row, on the same analysis: complete a load
    // that is available on all but one incoming edge, then the load becomes a
    // copy like the fully-available ones above.
    let mut placements: Vec<(usize, CodeInstruction)> = Vec::new();
    for candidate in partial {
        let Some(dst) = instructions[candidate.inst].operand("dst").cloned() else {
            continue;
        };
        // The load already sits at the top of the join, so the register it
        // reads into is the one the copy now reads from.
        let reload = CodeInstruction::new("ldr_u64")
            .field("dst", candidate.holder.clone())
            .field("base", crate::target::shared::abi::stack_pointer())
            .field("offset", candidate.offset.to_string());
        let block = &blocks[candidate.gap];
        let mut point = block.end;
        if point > block.start
            && crate::codegen::engine::regalloc::analysis::is_block_terminator(
                instructions[point - 1].op,
            )
        {
            point -= 1;
        }
        instructions[candidate.inst] = copy_of(dst, candidate.holder.clone());
        placements.push((point, reload));
    }
    if !placements.is_empty() {
        let fired = placements.len() as u64;
        // Later positions first, so an earlier insertion cannot shift a
        // later one's index.
        placements.sort_by(|a, b| b.0.cmp(&a.0));
        for (point, reload) in placements {
            instructions.insert(point.min(instructions.len()), reload);
        }
        crate::optimizer::stats::count_memory_pre(fired);
    }
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

    fn run(stream: &mut Vec<CodeInstruction>, level: u8) {
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

    /// Store PRE / Load PRE: the slot's value is in `%v1` on one edge into
    /// the join and on neither on the other, and the gap predecessor reaches
    /// the join unconditionally — so the load is placed there and the join's
    /// own load becomes a copy. The path that already had the value stops
    /// loading twice.
    #[test]
    fn a_partially_available_load_is_completed_on_the_gap_edge() {
        let mut stream = vec![
            ci("cmp_imm", &[("lhs", "%v9"), ("rhs", "0")]),
            ci("b.eq", &[("target", "other")]),
            ci(
                "ldr_u64",
                &[("dst", "%v1"), ("base", "sp"), ("offset", "8")],
            ),
            ci("b", &[("target", "join")]),
            ci("label", &[("name", "other")]),
            ci("b", &[("target", "join")]),
            ci("label", &[("name", "join")]),
            ci(
                "ldr_u64",
                &[("dst", "%v2"), ("base", "sp"), ("offset", "8")],
            ),
            ci("ret", &[]),
        ];
        run(&mut stream, 3);
        // The join's load becomes a copy — exactly what the fully-available
        // half of this row produces — which copy propagation bypasses and DCE
        // then removes, leaving the load simply relocated into the gap edge.
        assert_eq!(
            stream.iter().map(|i| i.op).collect::<Vec<_>>(),
            vec![
                CodeOp::CmpImm,
                CodeOp::BranchEq,
                CodeOp::LdrU64,
                CodeOp::Branch,
                CodeOp::Label,
                CodeOp::LdrU64,
                CodeOp::Branch,
                CodeOp::Label,
                CodeOp::Mov,
                CodeOp::Ret,
            ],
        );
        assert_eq!(stream[8].get("src").as_deref(), Some("%v1"));
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
