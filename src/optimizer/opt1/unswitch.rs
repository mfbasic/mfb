//! Loop unswitching — a Level-3 catalog row (`planning/optimizations.md`),
//! the structured-loop form: a loop whose body tests a loop-invariant
//! condition every iteration is split into one test selecting between two
//! specialized loop copies, each with the `IF` replaced by its taken arm.
//!
//! ```text
//! WHILE c            IF inv THEN
//!   pre                WHILE c: pre; A; post
//!   IF inv: A ELSE B ELSE
//!   post               WHILE c: pre; B; post
//! ```
//!
//! Soundness: the condition must be pure, non-trapping, and invariant
//! ([`plans::loops::invariant`]) — it is now evaluated exactly once, before
//! the loop, instead of once per iteration (including *once instead of zero
//! times* for a zero-trip loop, which is why trap-freedom is mandatory).
//! Everything else is verbatim duplication: the arms stay inside a loop of
//! the same kind and nesting depth, so `EXIT`/`CONTINUE` inside them bind
//! exactly as before, and a `FOR`/`FOR EACH` header expression — though
//! duplicated syntactically — still evaluates once at runtime (only one
//! branch runs). Growth is bounded: one unswitch per loop per compile
//! (bottom-up, copies are not revisited) and only for bodies under the size
//! cap.

use crate::target::shared::nir::{NirModule, NirOp};

use super::plans::loops::{
    freshened_clone, invariant, loop_body_defined, op_count, scope_bind_names,
};
use super::plans::reads::NameUses;
#[cfg(test)]
use crate::operators::BinaryOp;

/// Bodies above this size are not worth doubling.
const BODY_CAP: usize = 64;

/// Apply the unswitching row to the whole module. Self-guarded on its
/// catalog level (3); the split count feeds `optimizer::stats`.
pub(crate) fn unswitch(module: &mut NirModule) {
    if !crate::optimizer::level_enabled(3) {
        return;
    }
    let mut split = 0;
    for function in &mut module.functions {
        let census = NameUses::census(&function.body);
        let mut salt = 0;
        unswitch_in_body(
            &mut function.body,
            &census,
            &function.resource_owners,
            &mut salt,
            &mut split,
        );
    }
    crate::optimizer::stats::count_loop_unswitches(split);
}

type ResourceOwners = std::collections::HashMap<String, crate::ir::resource_escape::ResOwner>;

fn loop_body_mut(op: &mut NirOp) -> Option<&mut Vec<NirOp>> {
    match op {
        NirOp::While { body, .. }
        | NirOp::For { body, .. }
        | NirOp::DoUntil { body, .. }
        | NirOp::ForEach { body, .. } => Some(body),
        _ => None,
    }
}

fn unswitch_in_body(
    ops: &mut Vec<NirOp>,
    census: &NameUses,
    resource_owners: &ResourceOwners,
    salt: &mut u64,
    split: &mut u64,
) {
    for op in ops.iter_mut() {
        match op {
            NirOp::If {
                then_body,
                else_body,
                ..
            } => {
                unswitch_in_body(then_body, census, resource_owners, salt, split);
                unswitch_in_body(else_body, census, resource_owners, salt, split);
            }
            NirOp::Match { cases, .. } => {
                for case in cases {
                    unswitch_in_body(&mut case.body, census, resource_owners, salt, split);
                }
            }
            NirOp::Trap { body, .. } => {
                unswitch_in_body(body, census, resource_owners, salt, split)
            }
            _ => {
                if let Some(body) = loop_body_mut(op) {
                    unswitch_in_body(body, census, resource_owners, salt, split);
                }
            }
        }
    }
    for op in ops.iter_mut() {
        let Some(defined) = loop_body_defined(op) else {
            continue;
        };
        let body: &Vec<NirOp> = loop_body_mut(op).expect("loop_body_defined matched a loop");
        if op_count(body) > BODY_CAP {
            continue;
        }
        // A body declaring a RES owner cannot be duplicated: the freshened
        // copy's renamed owner would fall outside `resource_owners` and its
        // close/escape machinery would misfire (see `opt1::peel`).
        if super::plans::loops::declared_names(body)
            .iter()
            .any(|name| resource_owners.contains_key(name.as_str()))
        {
            continue;
        }
        let body_scope = scope_bind_names(body);
        let Some((position, condition)) =
            body.iter().enumerate().find_map(|(position, statement)| {
                let NirOp::If {
                    condition,
                    then_body,
                    else_body,
                } = statement
                else {
                    return None;
                };
                // Splicing an arm into the body flattens the arm's scope into
                // the body's: their own-level declarations must not collide, or
                // the specialized copy declares a name twice.
                (invariant(condition, &defined)
                    && scope_bind_names(then_body).is_disjoint(&body_scope)
                    && scope_bind_names(else_body).is_disjoint(&body_scope))
                .then(|| (position, condition.clone()))
            })
        else {
            continue;
        };
        let then_loop = specialized_copy(op, position, true);
        // NIR locals are function-unique: the second copy re-declares the
        // loop variable and every body bind, so it is freshened wholesale.
        let else_loop = freshened_clone(
            std::slice::from_ref(&specialized_copy(op, position, false)),
            census,
            salt,
        )
        .remove(0);
        *op = NirOp::If {
            condition,
            then_body: vec![then_loop],
            else_body: vec![else_loop],
        };
        *split += 1;
    }
}

/// A copy of the loop with the `IF` at `position` replaced by one arm.
fn specialized_copy(loop_op: &NirOp, position: usize, take_then: bool) -> NirOp {
    let mut clone = loop_op.clone();
    let body = loop_body_mut(&mut clone).expect("caller verified a loop");
    let NirOp::If {
        then_body,
        else_body,
        ..
    } = body.remove(position)
    else {
        unreachable!("position points at an If");
    };
    let arm = if take_then { then_body } else { else_body };
    body.splice(position..position, arm);
    clone
}

#[cfg(test)]
mod tests;
