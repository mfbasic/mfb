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
        let body = loop_body(op).expect("loop_body_defined matched a loop");
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
        let Some(position) = body.iter().position(|statement| {
            let NirOp::If {
                condition,
                then_body,
                else_body,
            } = statement
            else {
                return false;
            };
            if !invariant(condition, &defined) {
                return false;
            }
            // Splicing an arm into the body flattens the arm's scope into the
            // body's: their own-level declarations must not collide, or the
            // specialized copy declares a name twice.
            scope_bind_names(then_body).is_disjoint(&body_scope)
                && scope_bind_names(else_body).is_disjoint(&body_scope)
        }) else {
            continue;
        };
        let NirOp::If { condition, .. } = &body[position] else {
            unreachable!("position points at an If");
        };
        let condition = condition.clone();
        let then_loop = specialized_copy(op, position, true);
        // NIR locals are function-unique: the second copy re-declares the
        // loop variable and every body bind, so it is freshened wholesale
        // (verified rename; skip the unswitch when it cannot be produced).
        let Some(mut else_side) = freshened_clone(
            std::slice::from_ref(&specialized_copy(op, position, false)),
            census,
            salt,
        ) else {
            continue;
        };
        let else_loop = else_side.remove(0);
        *op = NirOp::If {
            condition,
            then_body: vec![then_loop],
            else_body: vec![else_loop],
        };
        *split += 1;
    }
}

fn loop_body(op: &NirOp) -> Option<&Vec<NirOp>> {
    match op {
        NirOp::While { body, .. }
        | NirOp::For { body, .. }
        | NirOp::DoUntil { body, .. }
        | NirOp::ForEach { body, .. } => Some(body),
        _ => None,
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
mod tests {
    use super::super::local_rewrites::testutil::*;
    use super::*;
    use crate::optimizer::{with_opt_level, OptLevel};
    use crate::target::shared::nir::{NirFunction, NirValue};
    use crate::types::ParameterType;
    use std::collections::HashMap;

    fn run(body: Vec<NirOp>, level: u8) -> Vec<NirOp> {
        let function = NirFunction {
            name: "f".to_string(),
            visibility: "private".to_string(),
            kind: "function".to_string(),
            isolated: false,
            params: vec![],
            returns: ParameterType::Integer,
            body,
            file: "main.mfb".to_string(),
            resource_owners: HashMap::new(),
        };
        let mut module = test_module(vec![function]);
        with_opt_level(OptLevel(level), || unswitch(&mut module));
        module.functions.remove(0).body
    }

    fn assign(name: &str, value: NirValue) -> NirOp {
        NirOp::Assign {
            name: name.to_string(),
            value,
        }
    }

    fn while_with_if(cond: NirValue) -> Vec<NirOp> {
        vec![NirOp::While {
            kind: crate::ast::LoopKind::While,
            condition: local("c"),
            body: vec![
                assign("pre", local("x")),
                NirOp::If {
                    condition: cond,
                    then_body: vec![
                        assign("a", local("x")),
                        NirOp::ExitLoop {
                            kind: crate::ast::LoopKind::While,
                        },
                    ],
                    else_body: vec![assign("b", local("x"))],
                },
                assign("post", local("x")),
            ],
        }]
    }

    /// An invariant IF splits the loop: one guard, two specialized copies —
    /// with the surrounding statements and even an `EXIT` preserved verbatim
    /// inside each same-kind copy.
    #[test]
    fn invariant_if_splits_the_loop() {
        let body = run(while_with_if(binary("<", local("p"), local("q"))), 3);
        let NirOp::If {
            then_body,
            else_body,
            ..
        } = &body[0]
        else {
            panic!("expected the unswitch guard");
        };
        let NirOp::While {
            body: then_loop, ..
        } = &then_body[0]
        else {
            panic!("then arm holds the specialized loop");
        };
        assert_eq!(then_loop.len(), 4, "pre + arm(2 ops) + post");
        assert!(matches!(then_loop[2], NirOp::ExitLoop { .. }));
        let NirOp::While {
            body: else_loop, ..
        } = &else_body[0]
        else {
            panic!("else arm holds the specialized loop");
        };
        assert_eq!(else_loop.len(), 3, "pre + arm(1 op) + post");
    }

    /// A condition reading a name the body assigns is variant: no unswitch.
    /// So is a trap-capable condition (arithmetic can raise ErrOverflow).
    #[test]
    fn variant_or_trapping_conditions_stay() {
        let variant = run(while_with_if(binary("<", local("pre"), local("q"))), 3);
        assert!(matches!(&variant[0], NirOp::While { .. }));

        let trapping = run(while_with_if(binary("+", local("p"), local("q"))), 3);
        assert!(matches!(&trapping[0], NirOp::While { .. }));
    }

    /// The loop *variable* counts as defined-per-iteration even though it is
    /// no body statement: a condition reading it is variant.
    #[test]
    fn loop_variable_conditions_are_variant() {
        let body = run(
            vec![NirOp::For {
                name: "i".to_string(),
                type_: ParameterType::Integer,
                start: int_const("0"),
                end: int_const("9"),
                step: int_const("1"),
                body: vec![NirOp::If {
                    condition: binary("<", local("i"), local("q")),
                    then_body: vec![assign("a", local("x"))],
                    else_body: vec![],
                }],
                loc: Default::default(),
            }],
            3,
        );
        assert!(matches!(&body[0], NirOp::For { .. }), "no unswitch");
    }

    /// The row is off at `-O2` (it is a Level-3 row).
    #[test]
    fn level_two_disables_the_row() {
        let body = run(while_with_if(binary("<", local("p"), local("q"))), 2);
        assert!(matches!(&body[0], NirOp::While { .. }));
    }
}
