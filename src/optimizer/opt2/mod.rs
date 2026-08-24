//! Opt2 — the MIR/machine half of the optimizer (plan-100).
//!
//! Two distinct things live here:
//!
//! 1. **The landed Level-1 passes.** [`fma_fusion::fuse_scalar_fma`] runs on the
//!    target-neutral MIR stream just before register allocation;
//!    [`peephole::forward_stores_to_loads`] and [`peephole::remove_fp_shuttles`]
//!    run *after* register allocation, on physical registers. Each self-guards
//!    with [`crate::optimizer::level_enabled`]`(1)`, so `-O0` turns all three off
//!    and `-O1` (the default) runs them exactly where and as they always have.
//! 2. **The reserved between-selection-and-regalloc MIR seam**, [`optimize_mir`].
//!    Identity today; no catalog row occupies it yet.

pub(crate) mod fma_fusion;
pub(crate) mod peephole;

use crate::codegen::engine::types::CodeInstruction;
use crate::optimizer::OptLevel;

/// The reserved Opt2 seam: MIR-level optimization between instruction selection
/// and register allocation, in place on the selected stream.
///
/// **Identity today** — the scaffold reserves the position without occupying it.
/// The pipeline this seam eventually brackets is
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
/// Rows that will land here are the CFG/dataflow ones in
/// `planning/optimizations.md` — dead-store elimination, unreachable-block
/// pruning, jump threading, redundant-load elimination, the alias-based
/// broadening of store-to-load forwarding, behavior-preserving check elision.
///
/// The two machine peepholes are deliberately **not** here: they operate on
/// physical registers and so stay at their post-regalloc call sites. `level` is
/// the active dial, threaded for the future rows that will filter on it; each
/// row self-guards on its own catalog level rather than on the seam.
pub(crate) fn optimize_mir(instructions: &mut Vec<CodeInstruction>, level: OptLevel) {
    let _ = (instructions, level);
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

    /// `%f2 = %f0 * %f1 ; %f3 = %f2 + %f9` — the canonical stream
    /// `fuse_scalar_fma` collapses into one `fmadd_d` at `-O1`.
    fn fusable() -> Vec<CodeInstruction> {
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
    fn level_zero_disables_fma_fusion() {
        let mut off = fusable();
        with_opt_level(OptLevel(0), || fma_fusion::fuse_scalar_fma(&mut off));
        assert_eq!(shape(&off), shape(&fusable()), "-O0 must not fuse");

        let mut on = fusable();
        with_opt_level(OptLevel(1), || fma_fusion::fuse_scalar_fma(&mut on));
        assert_eq!(on.len(), 1, "-O1 must still fuse");
        assert_eq!(on[0].op, CodeOp::FMaddD);
    }

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

    /// plan-100 §3(b): the reserved seam is an identity at *every* level, so no
    /// row can accidentally fire out of it before one is written.
    #[test]
    fn optimize_mir_is_identity_at_every_level() {
        for level in 0..=6u8 {
            let mut stream = fusable();
            optimize_mir(&mut stream, OptLevel(level));
            assert_eq!(shape(&stream), shape(&fusable()), "level {level}");
        }
    }
}
