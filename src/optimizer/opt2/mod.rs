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
//!    occupied by its first row: the Opt2 half of constant folding
//!    ([`constant_folding`], Level 1).

pub(crate) mod constant_folding;
pub(crate) mod peephole;

use crate::codegen::engine::types::CodeInstruction;
use crate::optimizer::OptLevel;

/// The Opt2 seam: MIR-level optimization between instruction selection and
/// register allocation, in place on the selected stream.
///
/// Occupied by the block-local MIR constant folder ([`constant_folding`], the
/// Opt2 half of the Level-1 "Constant folding" row). The pipeline this seam
/// eventually brackets is
///
/// ```text
/// Plan2(CFG + SSA/def-use) -> Opt2 passes -> Out-of-SSA(MIR) -> regalloc
/// ```
///
/// where Plan2 is the persistent CFG + SSA/def-use (promoting the throwaway
/// `build_cfg` in `codegen::engine::regalloc::analysis`) together with its
/// **demand-driven** analyses — SSA promotion (mem2reg), alias analysis,
/// memory-SSA / memory-dependence, range and trap analysis, loop
/// canonicalization, and function-attribute (`no-trap`) inference. None of that
/// is built yet: it arrives with the first Opt2 pass that needs it, because
/// building and destructing SSA with zero consumers is pure risk against the
/// default-level byte-identity gate (plan-100 §5 / Open Decisions).
///
/// Rows still to land here are the CFG/dataflow ones in
/// `planning/optimizations.md` — dead-store elimination, unreachable-block
/// pruning, jump threading, redundant-load elimination, the alias-based
/// broadening of store-to-load forwarding, behavior-preserving check elision.
///
/// The two machine peepholes are deliberately **not** here: they operate on
/// physical registers and so stay at their post-regalloc call sites. `level` is
/// the active dial, threaded for the future rows that will filter on it; each
/// row self-guards on its own catalog level rather than on the seam.
pub(crate) fn optimize_mir(instructions: &mut Vec<CodeInstruction>, level: OptLevel) {
    let _ = level;
    constant_folding::fold_constants(instructions);
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
            optimize_mir(&mut stream, OptLevel(level));
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
        with_opt_level(OptLevel(0), || optimize_mir(&mut off, OptLevel(0)));
        assert_eq!(off[2].op, CodeOp::Add, "-O0 must not fold");

        let mut on = foldable();
        with_opt_level(OptLevel(1), || optimize_mir(&mut on, OptLevel(1)));
        assert_eq!(on[2].op, CodeOp::MovImm, "-O1 must fold through the seam");
        assert_eq!(on[2].get("value").as_deref(), Some("5"));
    }
}
