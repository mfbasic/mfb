//! Structural accessors over a NIR statement — the two questions every Opt1
//! row that walks the tree by hand has to answer, in one place so they cannot
//! drift apart:
//!
//! - **What values does this statement evaluate in its *own* scope?** — its
//!   initializer, its condition, its scrutinee. Not the values inside a body
//!   it owns, which run a different number of times and in a different scope.
//! - **What statement lists does it own?** — the arms of an `IF`, the body of
//!   a loop, the cases of a `MATCH`, the handler of a `TRAP`.
//!
//! The read-only census rows ride [`crate::target::shared::nir::visit`]
//! instead, which recurses into everything at once. That is the right shape
//! for counting and the wrong one for rewriting: a row that rewrites has to
//! distinguish "here" from "inside", because the two have different rules.
//! These accessors are that distinction, and the `_mut` halves are what makes
//! them usable from a rewriting walk.
//!
//! Adding a `NirOp` variant makes every match here fail to compile, which is
//! the point — a new statement kind must state where its values live rather
//! than silently arriving as "no values, no bodies".

use crate::target::shared::nir::{NirOp, NirValue};

/// The values `op` evaluates in its own scope.
pub(in crate::optimizer::opt1) fn own_values(op: &NirOp) -> Vec<&NirValue> {
    match op {
        NirOp::Bind { value, .. } | NirOp::StoreGlobal { value, .. } => value.iter().collect(),
        NirOp::Return { value } => value.iter().collect(),
        NirOp::Assign { value, .. }
        | NirOp::StateAssign { value, .. }
        | NirOp::Eval { value }
        | NirOp::Fail { error: value }
        | NirOp::ExitProgram { code: value } => vec![value],
        NirOp::If { condition, .. }
        | NirOp::While { condition, .. }
        | NirOp::DoUntil { condition, .. } => vec![condition],
        // A `MATCH` evaluates its scrutinee here; each case's guard runs in
        // the case's own scope, alongside its body.
        NirOp::Match { value, .. } => vec![value],
        NirOp::ForEach { iterable, .. } => vec![iterable],
        NirOp::For {
            start, end, step, ..
        } => vec![start, end, step],
        // A `TRAP` region owns only its handler; the error binding it
        // introduces is not a value evaluated here.
        NirOp::Trap { .. } | NirOp::ExitLoop { .. } | NirOp::ContinueLoop { .. } => Vec::new(),
    }
}

/// The same, for a rewriting walk.
pub(in crate::optimizer::opt1) fn own_values_mut(op: &mut NirOp) -> Vec<&mut NirValue> {
    match op {
        NirOp::Bind { value, .. } | NirOp::StoreGlobal { value, .. } => value.iter_mut().collect(),
        NirOp::Return { value } => value.iter_mut().collect(),
        NirOp::Assign { value, .. }
        | NirOp::StateAssign { value, .. }
        | NirOp::Eval { value }
        | NirOp::Fail { error: value }
        | NirOp::ExitProgram { code: value } => vec![value],
        NirOp::If { condition, .. }
        | NirOp::While { condition, .. }
        | NirOp::DoUntil { condition, .. } => vec![condition],
        NirOp::Match { value, .. } => vec![value],
        NirOp::ForEach { iterable, .. } => vec![iterable],
        NirOp::For {
            start, end, step, ..
        } => vec![start, end, step],
        NirOp::Trap { .. } | NirOp::ExitLoop { .. } | NirOp::ContinueLoop { .. } => Vec::new(),
    }
}

/// The statement lists `op` owns. A `MATCH` case's guard is *not* here: it is
/// a value, and it belongs to the case, so a row that cares about guards has
/// to reach for [`match_guards`].
pub(in crate::optimizer::opt1) fn nested_bodies(op: &NirOp) -> Vec<&Vec<NirOp>> {
    match op {
        NirOp::If {
            then_body,
            else_body,
            ..
        } => vec![then_body, else_body],
        NirOp::While { body, .. }
        | NirOp::For { body, .. }
        | NirOp::DoUntil { body, .. }
        | NirOp::ForEach { body, .. }
        | NirOp::Trap { body, .. } => vec![body],
        NirOp::Match { cases, .. } => cases.iter().map(|case| &case.body).collect(),
        NirOp::Bind { .. }
        | NirOp::StoreGlobal { .. }
        | NirOp::Assign { .. }
        | NirOp::StateAssign { .. }
        | NirOp::Return { .. }
        | NirOp::ExitLoop { .. }
        | NirOp::ContinueLoop { .. }
        | NirOp::ExitProgram { .. }
        | NirOp::Fail { .. }
        | NirOp::Eval { .. } => Vec::new(),
    }
}

/// The same, for a rewriting walk.
pub(in crate::optimizer::opt1) fn nested_bodies_mut(op: &mut NirOp) -> Vec<&mut Vec<NirOp>> {
    match op {
        NirOp::If {
            then_body,
            else_body,
            ..
        } => vec![then_body, else_body],
        NirOp::While { body, .. }
        | NirOp::For { body, .. }
        | NirOp::DoUntil { body, .. }
        | NirOp::ForEach { body, .. }
        | NirOp::Trap { body, .. } => vec![body],
        NirOp::Match { cases, .. } => cases.iter_mut().map(|case| &mut case.body).collect(),
        NirOp::Bind { .. }
        | NirOp::StoreGlobal { .. }
        | NirOp::Assign { .. }
        | NirOp::StateAssign { .. }
        | NirOp::Return { .. }
        | NirOp::ExitLoop { .. }
        | NirOp::ContinueLoop { .. }
        | NirOp::ExitProgram { .. }
        | NirOp::Fail { .. }
        | NirOp::Eval { .. } => Vec::new(),
    }
}

/// A `MATCH`'s per-case guard expressions, which run in the case's scope
/// rather than the statement's.
pub(in crate::optimizer::opt1) fn match_guards(op: &NirOp) -> Vec<&NirValue> {
    match op {
        NirOp::Match { cases, .. } => cases
            .iter()
            .filter_map(|case| case.guard.as_ref())
            .collect(),
        _ => Vec::new(),
    }
}
