//! Opt1 — the NIR half of the optimizer (plan-100).
//!
//! A single seam, [`optimize_nir`], sitting between NIR lowering and Plan1
//! (storage / `StorageType` / symbol assignment). Twelve catalog rows occupy
//! it — [`constant_folding`], [`algebraic`] simplification, and non-loop
//! [`strength`] reduction (Level 1, composed into one scope-tracked walk by
//! [`local_rewrites`]); tree-level [`branches`] simplification, [`uce`], and
//! [`dce`] (Level 2, the last consuming the [`plans`] name-usage census);
//! and the six structured-loop rows (Level 3): [`licm`], [`unswitch`],
//! [`fuse`], [`fission`], [`peel`], and [`rotate`], consuming the
//! [`plans::loops`] fact base (invariance, loop-control capture, the pure
//! statement class).

pub(crate) mod algebraic;
pub(crate) mod branches;
pub(crate) mod constant_folding;
pub(crate) mod dce;
pub(crate) mod fission;
pub(crate) mod fuse;
pub(crate) mod licm;
pub(crate) mod local_rewrites;
pub(crate) mod peel;
pub(crate) mod plans;
pub(crate) mod rotate;
pub(crate) mod strength;
pub(crate) mod uce;
pub(crate) mod unswitch;

use crate::optimizer::OptLevel;
use crate::target::shared::nir::NirModule;

/// The Opt1 seam: whole-module NIR-to-NIR optimization, run once per build on
/// the sole `NirModule` every target consumes.
///
/// Placed *before* Plan1 so a pass here can still change what storage Plan1
/// assigns, and *after* `merge_packages`, so it sees the complete unified
/// function set rather than one compilation unit.
///
/// Landed rows (all Level 1, driven by [`local_rewrites`]): **constant
/// folding** (`1 + 1` → `2`, the non-trapping subset), **algebraic
/// simplification** (`x * 1` → `x`), and **strength reduction (non-loop)**
/// (`x * 2` → `x + x`, `x ^ 2` → `x * x`). Rows still to land here are the
/// remaining structured/high-level ones in `planning/optimizations.md` —
/// constant propagation, inlining and devirtualization, SROA and escape
/// analysis (both of which must precede Plan1's slot decisions), global
/// localization/constification, and loop unrolling (structured
/// `NirOp::For`/`While` nodes make it CFG/SSA-free here, plan-100 §5).
///
/// `level` is the active dial, threaded for future rows; each row self-guards
/// on its own catalog level (via [`crate::optimizer::level_enabled`] in the
/// [`local_rewrites`] driver) rather than on the seam, so one `-ON` lights up
/// rows across every seam at once.
pub(crate) fn optimize_nir(mut module: NirModule, level: OptLevel) -> NirModule {
    let _ = level;
    local_rewrites::apply(&mut module);
    // After the local rewrites: branch simplification (Level 2) folds the
    // constant conditions folding just minted; then the loop rows (Level 3)
    // on the still-structured loops — LICM shrinks bodies, unswitching
    // splits on invariant tests, fusion/fission reshape adjacent phases,
    // peeling splits first iterations, and rotation runs last so every
    // other row saw the head-tested `WHILE` shape. Then unreachable-code
    // truncation (Level 2), so a name whose only readers were unreachable
    // (or in a dropped arm) is provably unused for tree-DCE (Level 2), which
    // sweeps it along with the bindings the rewrites stranded.
    branches::simplify(&mut module);
    licm::hoist(&mut module);
    unswitch::unswitch(&mut module);
    fuse::fuse(&mut module);
    fission::split(&mut module);
    peel::peel(&mut module);
    rotate::rotate(&mut module);
    uce::eliminate(&mut module);
    dce::eliminate(&mut module);
    module
}
