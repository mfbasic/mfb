//! Opt1 — the NIR half of the optimizer (plan-100).
//!
//! A single seam, [`optimize_nir`], sitting between NIR lowering and Plan1
//! (storage / `StorageType` / symbol assignment). Three catalog rows occupy
//! it — [`constant_folding`], [`algebraic`] simplification, and non-loop
//! [`strength`] reduction (all Level 1) — composed into one scope-tracked walk
//! by [`local_rewrites`].

pub(crate) mod algebraic;
pub(crate) mod constant_folding;
pub(crate) mod local_rewrites;
pub(crate) mod strength;

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
    module
}
