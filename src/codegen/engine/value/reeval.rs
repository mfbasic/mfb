//! The shared "is this a re-evaluable place" predicate for native lowerings that
//! evaluate an operand more than once (vector-op lane inlining, the collections
//! `groupBy` / `sortBy` fast paths).

use crate::target::shared::nir::*;

/// Whether `value` is cheap and side-effect-free to evaluate more than once (a
/// binding read, a local-slot address, or a field read of one — a `MemberAccess`
/// chain rooted at any of those, recursing through nested fields). A
/// call-rooted or arithmetic operand is not — it would be recomputed once per
/// lane, or (for a fast-path builtin argument) re-evaluate a side effect on
/// every native lowering site — so those fall back to the FUNC/`.mfb` path.
///
/// plan-149's place forwarding (`optimizer/opt1/placefwd.rs`) rewrites
/// `LET base = rec.xs` + `f(base, …)` into `f(rec.xs, …)`, turning a `Local`
/// argument into a `MemberAccess` one; a gate that only accepted `Local` lost the
/// forwarded case (bug-760 A) even though a place read is exactly as safe as the
/// local it replaced.
pub(crate) fn is_reevaluation_safe(value: &NirValue) -> bool {
    match value {
        NirValue::Local(_) | NirValue::Global { .. } | NirValue::Const { .. } => true,
        NirValue::LocalRef { .. } => true,
        NirValue::MemberAccess { target, .. } => is_reevaluation_safe(target),
        _ => false,
    }
}
