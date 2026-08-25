//! Loop-invariant code motion (LICM) — a Level-3 catalog row
//! (`planning/optimizations.md`), the structured-loop form: move a `LET`
//! whose initializer provably computes the same value every iteration out in
//! front of the loop, so it runs once instead of per iteration.
//!
//! The trap discipline is the row's own annotation — "hoists only
//! trap-free/proven ops": a hoisted initializer is evaluated once *before*
//! the loop, including when the loop would have run zero times, so it must be
//! in the pure, non-trapping class (`dce::pure_non_trapping` — leaves,
//! comparisons, Boolean connectives; **no arithmetic**, which can raise
//! `ErrOverflow`, and note a bind of a Float *leaf* is fine because the leaf
//! already passed its own §4.1 observation boundary). Invariance is
//! conservative ([`plans::loops::invariant`]): the initializer reads no name
//! the body can redefine (loop variables included) and no global. Moving the
//! bind is scope-safe only when every occurrence of its name lives inside the
//! loop body (whole-function census vs body census — scope-blind, so
//! shadowing anywhere blocks the hoist) and the body defines the name exactly
//! once. Hoisting runs to a fixpoint, so an inner loop's hoisted bind can
//! ride outward through the enclosing loop in a later round.

use crate::target::shared::nir::{NirModule, NirOp};

use super::plans::loops::{invariant, loop_body_defined};
use super::plans::reads::NameUses;
use std::collections::HashSet;

/// Apply the LICM row to the whole module. Self-guarded on its catalog
/// level (3); the hoist count feeds `optimizer::stats`.
pub(crate) fn hoist(module: &mut NirModule) {
    if !crate::optimizer::level_enabled(3) {
        return;
    }
    let mut hoisted = 0;
    for function in &mut module.functions {
        loop {
            let uses = NameUses::census(&function.body);
            let before = hoisted;
            hoist_in_body(&mut function.body, &uses, &mut hoisted);
            if hoisted == before {
                break;
            }
        }
    }
    crate::optimizer::stats::count_licm_hoists(hoisted);
}

fn loop_body_mut(op: &mut NirOp) -> Option<&mut Vec<NirOp>> {
    match op {
        NirOp::While { body, .. }
        | NirOp::For { body, .. }
        | NirOp::DoUntil { body, .. }
        | NirOp::ForEach { body, .. } => Some(body),
        _ => None,
    }
}

fn hoist_in_body(ops: &mut Vec<NirOp>, uses: &NameUses, hoisted: &mut u64) {
    // Children first: an inner loop's hoist lands in this level's loop body,
    // where the next fixpoint round can lift it further out.
    for op in ops.iter_mut() {
        match op {
            NirOp::If {
                then_body,
                else_body,
                ..
            } => {
                hoist_in_body(then_body, uses, hoisted);
                hoist_in_body(else_body, uses, hoisted);
            }
            NirOp::Match { cases, .. } => {
                for case in cases {
                    hoist_in_body(&mut case.body, uses, hoisted);
                }
            }
            NirOp::Trap { body, .. } => hoist_in_body(body, uses, hoisted),
            _ => {
                if let Some(body) = loop_body_mut(op) {
                    hoist_in_body(body, uses, hoisted);
                }
            }
        }
    }
    let mut index = 0;
    while index < ops.len() {
        let Some(defined) = loop_body_defined(&ops[index]) else {
            index += 1;
            continue;
        };
        let body = loop_body_mut(&mut ops[index]).expect("loop_body_defined matched a loop");
        match hoistable_bind(body, uses, &defined) {
            Some(position) => {
                let bind = body.remove(position);
                ops.insert(index, bind);
                *hoisted += 1;
                index += 1; // the loop moved one slot right; re-examine it
            }
            None => index += 1,
        }
    }
}

/// The first top-level `Bind` in `body` eligible to move out, if any.
/// `defined` is [`loop_body_defined`]'s set — the body's definitions plus the
/// loop variable (it includes each bind's own name, which is harmless: an
/// initializer cannot read its own binding).
fn hoistable_bind(
    body: &[NirOp],
    function_uses: &NameUses,
    defined: &HashSet<String>,
) -> Option<usize> {
    let body_uses = NameUses::census(body);
    body.iter().position(|op| {
        let NirOp::Bind {
            name,
            type_,
            value: Some(value),
            ..
        } = op
        else {
            return false;
        };
        // Scalars only: a heap-typed bind carries per-iteration allocation
        // and drop machinery whose timing hoisting would change.
        if !super::dce::scalar_type(type_) {
            return false;
        }
        // Every occurrence of the name lives inside this loop, and the body
        // holds no *other* definition of it.
        if function_uses.count(name) != body_uses.count(name) {
            return false;
        }
        let mut defs = 0;
        count_defs(body, name, &mut defs);
        defs <= 1 && invariant(value, defined)
    })
}

/// How many times `body` (re)defines `name` — binds, assigns, loop variables,
/// TRAP bindings, at any depth.
fn count_defs(ops: &[NirOp], name: &str, count: &mut u64) {
    for op in ops {
        match op {
            NirOp::Bind { name: n, .. }
            | NirOp::Assign { name: n, .. }
            | NirOp::For { name: n, .. }
            | NirOp::ForEach { name: n, .. }
            | NirOp::Trap { name: n, .. } => {
                if n == name {
                    *count += 1;
                }
            }
            NirOp::StateAssign { resource, .. } => {
                if resource == name {
                    *count += 1;
                }
            }
            _ => {}
        }
        match op {
            NirOp::If {
                then_body,
                else_body,
                ..
            } => {
                count_defs(then_body, name, count);
                count_defs(else_body, name, count);
            }
            NirOp::Match { cases, .. } => {
                for case in cases {
                    count_defs(&case.body, name, count);
                }
            }
            NirOp::While { body, .. }
            | NirOp::For { body, .. }
            | NirOp::DoUntil { body, .. }
            | NirOp::ForEach { body, .. }
            | NirOp::Trap { body, .. } => count_defs(body, name, count),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::local_rewrites::testutil::*;
    use super::*;
    use crate::optimizer::{with_opt_level, OptLevel};
    use crate::target::shared::nir::{NirFunction, NirValue};
    use crate::types::ParameterType;
    use std::collections::HashMap;

    fn bind(name: &str, value: NirValue) -> NirOp {
        NirOp::Bind {
            mutable: false,
            name: name.to_string(),
            type_: ParameterType::Boolean,
            value: Some(value),
        }
    }

    fn while_loop(body: Vec<NirOp>) -> NirOp {
        NirOp::While {
            kind: crate::ast::LoopKind::While,
            condition: local("c"),
            body,
        }
    }

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
        with_opt_level(OptLevel(level), || hoist(&mut module));
        module.functions.remove(0).body
    }

    /// An invariant pure comparison bind moves out; the trap-capable
    /// arithmetic bind and the variant bind stay.
    #[test]
    fn invariant_pure_binds_hoist() {
        let body = run(
            vec![while_loop(vec![
                bind("inv", binary("<", local("a"), local("b"))),
                bind("arith", binary("+", local("a"), local("b"))),
                bind("var", binary("<", local("a"), local("m"))),
                NirOp::Assign {
                    name: "m".to_string(),
                    value: local("inv"),
                },
                NirOp::Eval {
                    value: local("arith"),
                },
                NirOp::Eval {
                    value: local("var"),
                },
            ])],
            3,
        );
        assert!(
            matches!(&body[0], NirOp::Bind { name, .. } if name == "inv"),
            "the invariant comparison hoists in front of the loop"
        );
        let NirOp::While { body: rest, .. } = &body[1] else {
            panic!("loop follows the hoisted bind");
        };
        assert_eq!(rest.len(), 5, "arith (trap-capable) and var (variant) stay");
    }

    /// A name also used outside the loop is not scope-safe to move.
    #[test]
    fn outside_uses_block_the_hoist() {
        let body = run(
            vec![
                while_loop(vec![
                    bind("t", binary("<", local("a"), local("b"))),
                    NirOp::Eval { value: local("t") },
                ]),
                NirOp::Eval { value: local("t") },
            ],
            3,
        );
        assert!(matches!(&body[0], NirOp::While { .. }), "nothing moved");
    }

    /// A `For` body bind depending on the loop variable is variant.
    #[test]
    fn loop_variable_dependence_blocks_the_hoist() {
        let body = run(
            vec![NirOp::For {
                name: "i".to_string(),
                type_: ParameterType::Integer,
                start: int_const("0"),
                end: int_const("9"),
                step: int_const("1"),
                body: vec![
                    bind("t", binary("<", local("i"), local("b"))),
                    NirOp::Eval { value: local("t") },
                ],
                loc: Default::default(),
            }],
            3,
        );
        let NirOp::For { body: inner, .. } = &body[0] else {
            panic!("expected For");
        };
        assert_eq!(inner.len(), 2, "loop-variable read keeps it inside");
    }

    /// The row is off at `-O2` (it is a Level-3 row).
    #[test]
    fn level_two_disables_the_row() {
        let body = run(
            vec![while_loop(vec![
                bind("inv", binary("<", local("a"), local("b"))),
                NirOp::Eval {
                    value: local("inv"),
                },
            ])],
            2,
        );
        assert!(matches!(&body[0], NirOp::While { .. }));
    }
}
