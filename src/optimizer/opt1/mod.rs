//! Opt1 — the NIR half of the optimizer (plan-100).
//!
//! A single seam, [`optimize_nir`], sitting between NIR lowering and Plan1
//! (storage / `StorageType` / symbol assignment). Identity today; no catalog
//! row occupies it yet.

use crate::optimizer::OptLevel;
use crate::target::shared::nir::NirModule;

/// The Opt1 seam: whole-module NIR-to-NIR optimization, run once per build on
/// the sole `NirModule` every target consumes.
///
/// **Identity today** — the scaffold reserves the position without occupying it.
/// Placed *before* Plan1 so a pass here can still change what storage Plan1
/// assigns, and *after* `merge_packages`, so it sees the complete unified
/// function set rather than one compilation unit.
///
/// Rows that will land here are the structured/high-level ones in
/// `planning/optimizations.md` — constant folding and propagation, algebraic
/// simplification, non-loop strength reduction, inlining and devirtualization,
/// SROA and escape analysis (both of which must precede Plan1's slot
/// decisions), global localization/constification, and loop unrolling. Loop
/// unrolling is the natural first row: loops are still structured
/// `NirOp::For`/`While` nodes here, so it needs no CFG or SSA (plan-100 §5).
///
/// `level` is the active dial, threaded for the future rows that will filter on
/// it; each row self-guards on its own catalog level via
/// [`crate::optimizer::level_enabled`] rather than on the seam, so one `-ON`
/// lights up rows across every seam at once.
pub(crate) fn optimize_nir(module: NirModule, level: OptLevel) -> NirModule {
    let _ = level;
    module
}
