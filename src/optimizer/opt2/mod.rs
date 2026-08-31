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
//! 2. **The between-selection-and-regalloc MIR seam**, [`optimize_mir`]. The
//!    rows there are listed in pipeline order in that function's own body and
//!    described one apiece in their modules; `optimizer::catalog::rows` is the
//!    authoritative level/stage list. They fall into families: the propagation
//!    and folding rows on the SSA overlay ([`constant_folding`], [`constprop`],
//!    [`sccp`], [`copyprop`], [`knownbits`]); the redundancy rows
//!    ([`lvn`], [`gvn`], [`pre`], and the memory pair in [`stldfwd`]); the
//!    control-flow rows ([`branches`], [`threading`], [`tailduped`], [`uce`],
//!    [`simplifycfg`], [`merge`]); the removal rows ([`dce`], [`adce`],
//!    [`dse`]); the code-motion rows ([`sink`], [`licm`]); and the
//!    range-driven check-elision cluster ([`checks`]) with its flag-availability
//!    sibling ([`flags`]). The [`plans`] module holds their optimization-only
//!    analyses — def-use marking, postdominators/control dependence, the SSA
//!    overlay, stack-slot availability, the known-bits lattice, the integer
//!    range lattice, and the natural-loop finder — all built on the
//!    allocator's own effect model and CFG.

pub(crate) mod adce;
pub(crate) mod branches;
pub(crate) mod checks;
pub(crate) mod constant_folding;
pub(crate) mod constprop;
pub(crate) mod copyprop;
pub(crate) mod dce;
pub(crate) mod dse;
pub(crate) mod flags;
pub(crate) mod gvn;
pub(crate) mod indvars;
pub(crate) mod knownbits;
pub(crate) mod licm;
pub(crate) mod lvn;
pub(crate) mod merge;
pub(crate) mod peephole;
pub(crate) mod plans;
pub(crate) mod pre;
pub(crate) mod rle;
pub(crate) mod sccp;
pub(crate) mod simplifycfg;
pub(crate) mod sink;
pub(crate) mod stldfwd;
pub(crate) mod tailduped;
pub(crate) mod threading;
pub(crate) mod uce;

use crate::codegen::engine::types::CodeInstruction;
use crate::optimizer::OptLevel;
use crate::target::shared::regmodel::RegisterModel;

/// The Opt2 seam: MIR-level optimization between instruction selection and
/// register allocation, in place on the selected stream.
///
/// The row order is the body below, each row self-guarded on its own catalog
/// level and each documented in its own module. They consume the
/// optimization-only analyses in [`plans`], which reuse the allocator's effect
/// model and CFG rather than restating them.
///
/// Plan2's SSA is an **overlay**: the stream keeps its `%vN` registers and the
/// values live only in the analysis, so no out-of-SSA lowering runs before
/// regalloc. Its demand-driven analyses arrive with the first row that needs
/// them (plan-100 §5) — the SSA overlay itself, stack-slot availability, the
/// known-bits lattice, the integer range lattice with dominating-predicate
/// refinement, and the natural-loop finder have all landed that way. What is
/// still missing is a general alias analysis: every memory row here is
/// confined to `sp` slots because nothing yet distinguishes two heap
/// addresses.
///
/// Rows still to land here are the remaining CFG/dataflow ones in
/// `planning/optimizations.md` — the alias-based broadening of store-to-load
/// forwarding and of dead-store elimination beyond `sp` slots, and the
/// predication family (if-conversion, select formation, memcpy/memset idioms),
/// which needs a neutral select op and per-backend lowering rather than
/// optimizer work.
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
    // Pipeline order, each row self-guarded on its own catalog level. Folding
    // (L1) strands dead feeders; constant propagation (L2) folds the
    // cross-block constants the block-local folder cannot see; SCCP (L3)
    // re-runs the constant question optimistically with reachability,
    // deciding branches the pessimistic pass could not; the memory rows (L3)
    // turn reloads into register copies and induction-variable simplification
    // (L3) merges duplicate loop counters, both feeding the numbering below;
    // tail duplication (L3) removes merges so the block-local rows keep their
    // facts; the known-bits rows (L2) turn masks and extensions into copies;
    // local and global value numbering (L3) rewrite recomputes into copies and
    // copy propagation (L2) bypasses them; PRE (L3) completes the
    // partially-available expressions numbering had to decline; loop-nest code
    // motion (L3) lifts what is left in a loop body; branch folding (L2) turns
    // known compares into unconditional flow; the check-elision cluster (L3)
    // decides the guards constants alone cannot and orphans their raise paths;
    // check fusion (L3) drops a comparison the flags already hold; sinking
    // (L3) moves work into the branch that uses it; jump threading (L3)
    // collapses hop chains; dead-store elimination (L2/L3) removes and sinks
    // stores; DCE (L2) sweeps what everything above stranded; ADCE (L3)
    // removes the dead control structure plain DCE keeps; unreachable-block
    // pruning (L2) drops what the folded branches orphaned; CFG simplification
    // (L2) tidies the leftovers and block merging (L2) fuses them back into
    // straight-line blocks.
    //
    // Every row is wrapped in a `-vv` trace span named for its catalog row, so
    // the profile attributes MIR time per pass rather than to the seam as a
    // whole. The wrapper is inert without `-vv` (`crate::trace::timed` calls the
    // body directly), and — like every other row here — self-guarding stays
    // inside each pass, so wrapping changes nothing about what runs.
    use crate::trace::timed;
    timed("constant folding", || {
        constant_folding::fold_constants(instructions)
    });
    timed("constant propagation", || {
        constprop::eliminate(instructions, model)
    });
    timed("SCCP", || sccp::eliminate(instructions, model));
    // One traversal serves both memory rows (see `stldfwd::forward`).
    timed("store-to-load + redundant load", || {
        stldfwd::forward(instructions, model)
    });
    timed("induction variables", || {
        indvars::simplify(instructions, model)
    });
    // Tail duplication (L3) runs before the block-local rows below: removing a
    // merge is what lets them keep their facts through the duplicated tail.
    timed("tail duplication", || tailduped::duplicate(instructions));
    // The known-bits rows (L2) run before value numbering: a mask or
    // extension they turn into a copy is one fewer expression to number.
    timed("known bits", || knownbits::simplify(instructions, model));
    timed("local value numbering", || {
        lvn::eliminate(instructions, model)
    });
    timed("global value numbering", || {
        gvn::eliminate(instructions, model)
    });
    timed("copy propagation", || {
        copyprop::eliminate(instructions, model)
    });
    // PRE (L3) picks up where global value numbering had to stop: the
    // expressions available on only some paths into a join. It runs before the
    // control-flow rows below so the joins it reasons about are the ones the
    // stream actually still has.
    timed("PRE", || pre::eliminate(instructions, model));
    // Loop-nest code motion (L3) runs on the redundancy-free stream: what is
    // left in a loop body by now is what genuinely recomputes each iteration.
    timed("LICM (MIR)", || licm::hoist(instructions, model));
    timed("branch folding", || branches::fold_branches(instructions));
    // The range-driven check-elision rows (L3) run after the constant-based
    // branch folding above and before threading/UCE below: what they decide is
    // exactly the guards constants alone cannot, and the raise paths they
    // orphan are what the unreachable-block sweep then removes.
    timed("check elision", || checks::eliminate(instructions, model));
    // Check fusion (L3) runs right after them, on what survives: a comparison
    // the flags already hold is deleted, its branch left to read the earlier
    // one's flags.
    timed("check fusion", || flags::fuse(instructions, model));
    // Sinking (L3) runs once the control flow above has settled: the branches
    // it moves work into are the ones that survive folding and threading.
    timed("sinking", || sink::sink(instructions, model));
    timed("jump threading", || threading::thread_jumps(instructions));
    timed("dead-store elimination", || {
        dse::eliminate(instructions, model)
    });
    timed("DCE (MIR)", || dce::eliminate(instructions, model));
    timed("ADCE", || adce::eliminate(instructions, model));
    timed("UCE (MIR)", || uce::eliminate(instructions));
    // CFG simplification (L2) tidies what the control-flow rows above leave —
    // no-op conditionals, jumps to returns, duplicate labels — and runs just
    // before merging so a label it removes can let a merge happen.
    timed("CFG simplification", || simplifycfg::simplify(instructions));
    timed("block merging", || merge::merge_blocks(instructions));
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
