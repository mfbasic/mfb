//! Opt2 — the MIR/machine half of the optimizer (plan-100).
//!
//! Two distinct things live here:
//!
//! 1. **The landed Level-1 passes.** [`peephole::forward_stores_to_loads`] and
//!    [`peephole::remove_fp_shuttles`] run *after* register allocation, on
//!    physical registers. Each self-guards with
//!    [`crate::optimizer::level_enabled`]`(1)`, so `-O0` turns both off and `-O1`
//!    (the default) runs them exactly where and as they always have. Both are
//!    strictly **behavior-preserving** -- forwarding never removes or reorders an
//!    instruction, and the shuttle fold fires only on a GPR proven dead by
//!    integer liveness -- which is what makes `-O0` a pure "less optimized, same
//!    answers" path.
//!
//!    `fuse_scalar_fma` deliberately does **not** live here: FMA contraction
//!    rounds once instead of twice and so changes float results, making it
//!    mandatory lowering rather than a dial row. It stays in
//!    `crate::codegen::compiler::opt::fma_fusion` (plan-100 Corrections).
//! 2. **The between-selection-and-regalloc MIR seam**, [`optimize_mir`],
//!    occupied by twelve rows: the Opt2 halves of constant folding
//!    ([`constant_folding`], Level 1), constant propagation ([`constprop`],
//!    Level 2) and copy propagation ([`copyprop`], Level 2) on the SSA
//!    overlay, local and global value numbering ([`lvn`]/[`gvn`], Level 3 —
//!    the latter also the CSE row), branch folding ([`branches`], Level 2),
//!    jump threading ([`threading`], Level 3), dead-code elimination
//!    ([`dce`], Level 2), unreachable code elimination ([`uce`], Level 2),
//!    dead-store elimination ([`dse`], Level 2), aggressive DCE ([`adce`],
//!    Level 3), and basic block merging ([`merge`], Level 2). The [`plans`]
//!    module holds their optimization-only analyses (def-use marking,
//!    postdominators/control dependence, the SSA overlay), built on the
//!    allocator's own effect model and CFG.

pub(crate) mod adce;
pub(crate) mod branches;
pub(crate) mod constant_folding;
pub(crate) mod constprop;
pub(crate) mod copyprop;
pub(crate) mod dce;
pub(crate) mod dse;
pub(crate) mod gvn;
pub(crate) mod lvn;
pub(crate) mod merge;
pub(crate) mod peephole;
pub(crate) mod plans;
pub(crate) mod threading;
pub(crate) mod uce;

use crate::codegen::engine::types::CodeInstruction;
use crate::optimizer::OptLevel;
use crate::target::shared::regmodel::RegisterModel;

/// The Opt2 seam: MIR-level optimization between instruction selection and
/// register allocation, in place on the selected stream.
///
/// Occupied by the block-local MIR constant folder ([`constant_folding`],
/// L1), the SSA-overlay propagation rows ([`constprop`] and [`copyprop`],
/// L2), branch folding ([`branches`], L2), jump threading ([`threading`],
/// L3), the precise-DCE sweep ([`dce`], L2), dead-store elimination
/// ([`dse`], L2), control-dependence ADCE ([`adce`], L3),
/// unreachable-block pruning ([`uce`], L2), and basic block merging
/// ([`merge`], L2) — consuming the optimization-only
/// analyses in [`plans`] (the SSA overlay, def-use marking,
/// postdominators/control dependence), which reuse the allocator's effect
/// model and CFG. Plan2's SSA is an **overlay**: the stream keeps its `%vN`
/// registers and the values live only in the analysis, so no out-of-SSA
/// lowering runs before regalloc. Its remaining demand-driven analyses —
/// alias analysis, memory-SSA / memory-dependence, range and trap analysis,
/// loop canonicalization, function-attribute (`no-trap`) inference — each
/// arrive with the first Opt2 pass that needs them (plan-100 §5).
///
/// Rows still to land here are the remaining CFG/dataflow ones in
/// `planning/optimizations.md` — redundant-load elimination, the alias-based
/// broadening of store-to-load forwarding and of DSE beyond `sp` slots,
/// behavior-preserving check elision.
///
/// The two machine peepholes are deliberately **not** here: they operate on
/// physical registers and so stay at their post-regalloc call sites. `level` is
/// the active dial, threaded for the future rows that will filter on it; each
/// row self-guards on its own catalog level rather than on the seam.
pub(crate) fn optimize_mir(
    instructions: &mut Vec<CodeInstruction>,
    model: &dyn RegisterModel,
    level: OptLevel,
) {
    let _ = level;
    // Pipeline order, each row self-guarded on its own catalog level: folding
    // (L1) strands dead feeders; constant propagation (L2) folds the
    // cross-block constants the block-local folder cannot see; local and
    // global value numbering (L3) rewrite recomputes into copies; copy
    // propagation (L2) bypasses register copies — the minted ones included —
    // stranding them; branch
    // folding (L2) turns known compares into unconditional flow (creating
    // statically-dead blocks); jump threading (L3) collapses jump-to-jump
    // chains; dead-store elimination (L2) strands more; DCE (L2) sweeps them
    // all; ADCE (L3) removes the dead control structure plain DCE keeps;
    // unreachable-block pruning (L2) drops what the folded branches and
    // threaded jumps orphaned; block merging (L2) runs last, fusing the
    // branch-to-next hops and orphaned labels back into straight-line blocks.
    constant_folding::fold_constants(instructions);
    constprop::eliminate(instructions, model);
    lvn::eliminate(instructions, model);
    gvn::eliminate(instructions, model);
    copyprop::eliminate(instructions, model);
    branches::fold_branches(instructions);
    threading::thread_jumps(instructions);
    dse::eliminate(instructions);
    dce::eliminate(instructions, model);
    adce::eliminate(instructions, model);
    uce::eliminate(instructions);
    merge::merge_blocks(instructions);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch::ops::CodeOp;
    use crate::optimizer::with_opt_level;

    fn ci(op: &str, fields: &[(&'static str, &str)]) -> CodeInstruction {
        let mut inst = CodeInstruction::new(op);
        for (k, v) in fields {
            inst = inst.field(k, v);
        }
        inst
    }

    /// `(op, [(field, rendered)])` for every instruction — enough to prove a
    /// stream came through a pass untouched.
    fn shape(instructions: &[CodeInstruction]) -> Vec<(CodeOp, Vec<(&'static str, String)>)> {
        instructions
            .iter()
            .map(|inst| {
                (
                    inst.op,
                    inst.fields
                        .iter()
                        .map(|(name, _)| (*name, inst.get(name).unwrap_or_default()))
                        .collect(),
                )
            })
            .collect()
    }

    /// A stream with no dial row in it -- used to prove the reserved seam is an
    /// identity, and (in `fma_fusion`'s own tests) that contraction still fires
    /// regardless of level.
    fn plain() -> Vec<CodeInstruction> {
        vec![
            ci("fmul_d", &[("dst", "%f2"), ("lhs", "%f0"), ("rhs", "%f1")]),
            ci("fadd_d", &[("dst", "%f3"), ("lhs", "%f2"), ("rhs", "%f9")]),
        ]
    }

    /// `str x10, [sp, #8] ; ldr x8, [sp, #8]` — the reload
    /// `forward_stores_to_loads` rewrites into `mov x8, x10` at `-O1`.
    fn forwardable() -> Vec<CodeInstruction> {
        vec![
            ci(
                "str_u64",
                &[("src", "x10"), ("base", "sp"), ("offset", "8")],
            ),
            ci("ldr_u64", &[("dst", "x8"), ("base", "sp"), ("offset", "8")]),
        ]
    }

    /// `fmov x8, d11 ; str x8, [sp, #1120]` with `x8` dead after — the shuttle
    /// pair `remove_fp_shuttles` folds into a single `str d11` at `-O1`.
    fn shuttle() -> Vec<CodeInstruction> {
        vec![
            ci("fmov_x_from_d", &[("dst", "x8"), ("src", "d11")]),
            ci(
                "str_u64",
                &[("src", "x8"), ("base", "sp"), ("offset", "1120")],
            ),
            ci("ret", &[]),
        ]
    }

    /// plan-100 §3(a): at `-O0` every Level-1 pass is a no-op on a stream it
    /// *does* rewrite at `-O1`. The two halves share one input, so a pass that
    /// silently stopped firing would fail the `-O1` half rather than passing
    /// both vacuously.
    #[test]
    fn level_zero_disables_store_to_load_forwarding() {
        let mut off = forwardable();
        with_opt_level(OptLevel(0), || {
            peephole::forward_stores_to_loads(&mut off, false)
        });
        assert_eq!(shape(&off), shape(&forwardable()), "-O0 must not forward");

        let mut on = forwardable();
        with_opt_level(OptLevel(1), || {
            peephole::forward_stores_to_loads(&mut on, false)
        });
        assert_ne!(
            shape(&on),
            shape(&forwardable()),
            "-O1 must still forward the reload"
        );
    }

    #[test]
    fn level_zero_disables_fp_shuttle_removal() {
        // AArch64 spellings (`x8`, `d11`), so the AArch64 model supplies the
        // caller-saved set the underlying liveness reads (bug-350).
        let model = &crate::arch::aarch64::regmodel::Aarch64RegisterModel;

        let mut off = shuttle();
        with_opt_level(OptLevel(0), || {
            peephole::remove_fp_shuttles(&mut off, model)
        });
        assert_eq!(shape(&off), shape(&shuttle()), "-O0 must not fold");

        let mut on = shuttle();
        with_opt_level(OptLevel(1), || peephole::remove_fp_shuttles(&mut on, model));
        assert_eq!(on.len(), 2, "-O1 must still fold the shuttle pair");
        assert_eq!(on[0].op, CodeOp::StrD);
        assert_eq!(on[0].get("src").as_deref(), Some("d11"));
    }

    /// The seam is occupied (constant folding), but on a stream with no
    /// foldable constants it must still be an identity at every level — no row
    /// may fire outside its own pattern.
    #[test]
    fn optimize_mir_is_identity_on_a_constant_free_stream() {
        for level in 0..=6u8 {
            let mut stream = plain();
            optimize_mir(
                &mut stream,
                &crate::arch::aarch64::regmodel::Aarch64RegisterModel,
                OptLevel(level),
            );
            assert_eq!(shape(&stream), shape(&plain()), "level {level}");
        }
    }

    /// The seam's first row: a known-constant ALU chain folds at `-O1` and is
    /// left alone at `-O0`. (The row's own unit tests cover the fold rules;
    /// this pins the seam wiring.)
    #[test]
    fn optimize_mir_runs_the_constant_folder() {
        let foldable = || {
            vec![
                ci(
                    "mov_imm",
                    &[("dst", "%1"), ("type", "Integer"), ("value", "2")],
                ),
                ci(
                    "mov_imm",
                    &[("dst", "%2"), ("type", "Integer"), ("value", "3")],
                ),
                ci("add", &[("dst", "%3"), ("lhs", "%1"), ("rhs", "%2")]),
            ]
        };
        let mut off = foldable();
        with_opt_level(OptLevel(0), || {
            optimize_mir(
                &mut off,
                &crate::arch::aarch64::regmodel::Aarch64RegisterModel,
                OptLevel(0),
            )
        });
        assert_eq!(off[2].op, CodeOp::Add, "-O0 must not fold");

        let mut on = foldable();
        with_opt_level(OptLevel(1), || {
            optimize_mir(
                &mut on,
                &crate::arch::aarch64::regmodel::Aarch64RegisterModel,
                OptLevel(1),
            )
        });
        assert_eq!(on[2].op, CodeOp::MovImm, "-O1 must fold through the seam");
        assert_eq!(on[2].get("value").as_deref(), Some("5"));
    }
}
