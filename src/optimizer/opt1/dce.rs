//! Dead-code elimination — the Opt1 (tree-level) half of the Level-2 catalog
//! row (`planning/optimizations.md`): remove unused bindings and value-less
//! evaluations from structured NIR. The precise Opt2 half lives in
//! `opt2::dce`; both feed one "Dead-code elimination (DCE)" `-v` count.
//!
//! Trap discipline (the same rule as every dial row): dead code may only be
//! removed when it is **provably trap-free and effect-free**, so removal can
//! never erase an observable raise. Concretely a `Bind` is removed only when
//! all of:
//!
//! - its name occurs nowhere else in the body ([`plans::reads`] — scope-blind
//!   and therefore shadow-safe), and it is not a resource owner
//!   (`resource_owners` names have close effects);
//! - its declared type is a plain scalar (Integer/Byte/Float/Fixed/Money/
//!   Boolean/Nothing) — no allocation, no ownership machinery, no drop;
//! - its initializer (if any) is in the pure, non-trapping expression class:
//!   value leaves (`Const`/`Local`/`Global`/`Capture`/`FunctionRef`),
//!   comparisons and `AND`/`OR`/`XOR`/`NOT` over that class (§11 comparisons
//!   never trap — a Float compare is not an observation boundary), and nothing
//!   else. Arithmetic stays: an unused `x + y` can still raise `ErrOverflow`,
//!   and an unused Float bind is itself the observation boundary that traps a
//!   non-finite (§4.1) — a bind of a bare Float *leaf* is removable exactly
//!   because the leaf already passed its own boundary.
//!
//! A bare `Eval` of a pure, non-trapping value is removed under the same rule.
//! Removal iterates to a fixpoint: deleting `LET b = a` can make `a`'s own
//! binding unused.

use crate::target::shared::nir::{NirFunction, NirModule, NirOp, NirValue};
use crate::types::ParameterType;

use super::plans::reads::NameUses;
use crate::operators::{BinaryOp, UnaryOp};

/// Apply the tree-level DCE row to the whole module. Self-guarded on its
/// catalog level (2); the removal count feeds `optimizer::stats`.
pub(crate) fn eliminate(module: &mut NirModule) {
    if !crate::optimizer::level_enabled(2) {
        return;
    }
    let mut removed = 0;
    for function in &mut module.functions {
        removed += eliminate_in_function(function);
    }
    crate::optimizer::stats::count_dead_code_eliminations(removed);
}

fn eliminate_in_function(function: &mut NirFunction) -> u64 {
    let mut removed = 0;
    loop {
        let uses = NameUses::census(&function.body);
        let resource_owners = &function.resource_owners;
        let before = removed;
        remove_dead_ops(&mut function.body, &uses, resource_owners, &mut removed);
        if removed == before {
            return removed;
        }
    }
}

/// One sweep over a body (recursing into nested bodies), dropping dead ops
/// against the given whole-function census.
fn remove_dead_ops(
    ops: &mut Vec<NirOp>,
    uses: &NameUses,
    resource_owners: &std::collections::HashMap<String, crate::ir::resource_escape::ResOwner>,
    removed: &mut u64,
) {
    ops.retain_mut(|op| {
        match op {
            NirOp::Bind {
                name, type_, value, ..
            } => {
                let dead = !uses.used_besides_bind(name)
                    && !resource_owners.contains_key(name.as_str())
                    && scalar_type(type_)
                    && value.as_ref().is_none_or(pure_non_trapping);
                if dead {
                    *removed += 1;
                    return false;
                }
            }
            NirOp::Eval { value } => {
                if pure_non_trapping(value) {
                    *removed += 1;
                    return false;
                }
            }
            NirOp::If {
                then_body,
                else_body,
                ..
            } => {
                remove_dead_ops(then_body, uses, resource_owners, removed);
                remove_dead_ops(else_body, uses, resource_owners, removed);
            }
            NirOp::Match { cases, .. } => {
                for case in cases {
                    remove_dead_ops(&mut case.body, uses, resource_owners, removed);
                }
            }
            NirOp::While { body, .. }
            | NirOp::For { body, .. }
            | NirOp::DoUntil { body, .. }
            | NirOp::ForEach { body, .. }
            | NirOp::Trap { body, .. } => {
                remove_dead_ops(body, uses, resource_owners, removed);
            }
            NirOp::StoreGlobal { .. }
            | NirOp::Assign { .. }
            | NirOp::StateAssign { .. }
            | NirOp::Return { .. }
            | NirOp::ExitLoop { .. }
            | NirOp::ContinueLoop { .. }
            | NirOp::ExitProgram { .. }
            | NirOp::Fail { .. } => {}
        }
        true
    });
}

/// Types whose bindings carry no allocation, ownership, or drop machinery —
/// removing an unused one is a pure register/slot saving. pub(in opt1): the
/// loop rows share the class (a hoisted/reordered bind must be equally inert).
pub(in crate::optimizer::opt1) fn scalar_type(type_: &ParameterType) -> bool {
    matches!(
        type_,
        ParameterType::Integer
            | ParameterType::Byte
            | ParameterType::Float
            | ParameterType::Fixed
            | ParameterType::Money
            | ParameterType::Boolean
            | ParameterType::Nothing
    )
}

/// The provably pure, non-trapping expression class this row may erase — and
/// the loop rows may move or re-evaluate (pub(in opt1)): evaluating it more
/// or fewer times, or elsewhere, is unobservable by construction.
pub(in crate::optimizer::opt1) fn pure_non_trapping(value: &NirValue) -> bool {
    match value {
        NirValue::Const { .. }
        | NirValue::Local(_)
        | NirValue::Global { .. }
        | NirValue::Capture { .. }
        | NirValue::FunctionRef { .. } => true,
        NirValue::Binary {
            op, left, right, ..
        } => {
            // Comparisons never trap (§4.11; a Float compare is not an
            // observation boundary), and the Boolean connectives are pure.
            // Arithmetic and `&` (allocates) are NOT in the class.
            (op.is_comparison() || matches!(op, BinaryOp::And | BinaryOp::Or | BinaryOp::Xor))
                && pure_non_trapping(left)
                && pure_non_trapping(right)
        }
        NirValue::Unary { op, operand, .. } => *op == UnaryOp::Not && pure_non_trapping(operand),
        _ => false,
    }
}

#[cfg(test)]
mod tests;
