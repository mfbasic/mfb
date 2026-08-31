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

pub(crate) mod aggcopy;
pub(crate) mod algebraic;
pub(crate) mod branches;
pub(crate) mod constant_folding;
pub(crate) mod dce;
pub(crate) mod fission;
pub(crate) mod fuse;
pub(crate) mod globals;
pub(crate) mod lencache;
pub(crate) mod licm;
pub(crate) mod local_rewrites;
pub(crate) mod peel;
pub(crate) mod plans;
pub(crate) mod recovery;
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
    // Each row is wrapped in a `-vv` trace span named for its catalog row (see
    // the identical treatment in `opt2::optimize_mir`); the wrapper is inert
    // without `-vv` and changes nothing about what runs.
    use crate::trace::timed;
    timed("local rewrites", || local_rewrites::apply(&mut module));
    // After the local rewrites: branch simplification (Level 2) folds the
    // constant conditions folding just minted; then the loop rows (Level 3)
    // on the still-structured loops — LICM shrinks bodies, unswitching
    // splits on invariant tests, fusion/fission reshape adjacent phases,
    // peeling splits first iterations, and rotation runs last so every
    // other row saw the head-tested `WHILE` shape. Then unreachable-code
    // truncation (Level 2), so a name whose only readers were unreachable
    // (or in a dropped arm) is provably unused for tree-DCE (Level 2), which
    // sweeps it along with the bindings the rewrites stranded.
    timed("branch simplification", || branches::simplify(&mut module));
    // The three global rows (Level 2) run on the whole module before the loop
    // rows: constifying a never-written global turns its reads into literals,
    // which the invariance and folding checks below can then see through.
    timed("globals", || globals::simplify(&mut module));
    // Codepoint `len()` caching (Level 3) runs before the loop rows: a cached
    // count is a plain local, which the invariance test can then hoist.
    timed("len cache", || lencache::cache(&mut module));
    timed("LICM (NIR)", || licm::hoist(&mut module));
    timed("loop unswitching", || unswitch::unswitch(&mut module));
    timed("loop fusion", || fuse::fuse(&mut module));
    timed("loop fission", || fission::split(&mut module));
    timed("loop peeling", || peel::peel(&mut module));
    timed("loop rotation", || rotate::rotate(&mut module));
    // Recovery-region simplification (Level 3) runs after the rewrites above:
    // a body they simplified into pure value flow is one whose TRAP handler
    // is provably unreachable.
    // Aggregate copy propagation (Level 3): a `LET b = a` on a record,
    // String or collection is a whole-block copy; forwarding it strands the
    // binding, which DCE below sweeps.
    timed("aggregate copy propagation", || {
        aggcopy::propagate(&mut module)
    });
    timed("recovery regions", || recovery::simplify(&mut module));
    timed("UCE (NIR)", || uce::eliminate(&mut module));
    timed("DCE (NIR)", || dce::eliminate(&mut module));
    module
}
