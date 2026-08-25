//! Loop peeling — a Level-3 catalog row (`planning/optimizations.md`), the
//! structured-loop form: split the first iteration of a small `WHILE` out in
//! front, so the downstream rows can specialize it (constants reaching a
//! first iteration fold; the loop entry test disappears from the hot path).
//!
//! ```text
//! WHILE c: body      IF c THEN body; WHILE c: body END IF
//! ```
//!
//! This is exact duplication, not reordering — over any execution the
//! condition still evaluates n+1 times and the body n times, in the same
//! order, so **no purity or trap-freedom is required of either**: a trapping
//! condition traps at the identical evaluation. Two structural guards apply:
//! the body must not carry `EXIT`/`CONTINUE` that would bind to this loop
//! ([`plans::loops::captures_loop_control`] for the loop's own kind — the
//! peeled copy sits *outside* the loop, where such an op would rebind or
//! fail to lower), and only small bodies are worth the growth. One peel per
//! loop per compile (bottom-up; the embedded copy is not revisited).

use crate::target::shared::nir::{NirModule, NirOp};

use super::plans::loops::{captures_loop_control, freshened_clone, op_count};
use super::plans::reads::NameUses;

/// Bodies above this size are not worth duplicating.
const BODY_CAP: usize = 16;

/// Apply the peeling row to the whole module. Self-guarded on its catalog
/// level (3); the peel count feeds `optimizer::stats`.
pub(crate) fn peel(module: &mut NirModule) {
    if !crate::optimizer::level_enabled(3) {
        return;
    }
    let mut peeled = 0;
    for function in &mut module.functions {
        let census = NameUses::census(&function.body);
        let mut salt = 0;
        peel_in_body(
            &mut function.body,
            &census,
            &function.resource_owners,
            &mut salt,
            &mut peeled,
        );
    }
    crate::optimizer::stats::count_loops_peeled(peeled);
}

type ResourceOwners = std::collections::HashMap<String, crate::ir::resource_escape::ResOwner>;

fn peel_in_body(
    ops: &mut Vec<NirOp>,
    census: &NameUses,
    resource_owners: &ResourceOwners,
    salt: &mut u64,
    peeled: &mut u64,
) {
    for op in ops.iter_mut() {
        match op {
            NirOp::If {
                then_body,
                else_body,
                ..
            } => {
                peel_in_body(then_body, census, resource_owners, salt, peeled);
                peel_in_body(else_body, census, resource_owners, salt, peeled);
            }
            NirOp::Match { cases, .. } => {
                for case in cases {
                    peel_in_body(&mut case.body, census, resource_owners, salt, peeled);
                }
            }
            NirOp::While { body, .. }
            | NirOp::For { body, .. }
            | NirOp::DoUntil { body, .. }
            | NirOp::ForEach { body, .. }
            | NirOp::Trap { body, .. } => peel_in_body(body, census, resource_owners, salt, peeled),
            _ => {}
        }
    }
    for op in ops.iter_mut() {
        let NirOp::While {
            kind,
            condition,
            body,
        } = op
        else {
            continue;
        };
        if op_count(body) > BODY_CAP || captures_loop_control(body, *kind) {
            continue;
        }
        // A body declaring a RES owner cannot be duplicated: the freshened
        // copy's renamed owner would fall outside `resource_owners`, and its
        // close/escape machinery would misfire (measured: an -O3 peel closed
        // `resource-collection-floats-runtime`'s handles early).
        if super::plans::loops::declared_names(body)
            .iter()
            .any(|name| resource_owners.contains_key(name.as_str()))
        {
            continue;
        }
        // NIR locals are function-unique: the peeled copy re-declares every
        // name the body declares, so it is cloned *freshened* (and the peel
        // is skipped if the verified rename cannot be produced).
        let Some(then_body) = freshened_clone(body, census, salt) else {
            continue;
        };
        let mut then_body = then_body;
        let condition = condition.clone();
        let inner = std::mem::replace(
            op,
            NirOp::If {
                condition,
                then_body: Vec::new(),
                else_body: Vec::new(),
            },
        );
        then_body.push(inner);
        let NirOp::If {
            then_body: slot, ..
        } = op
        else {
            unreachable!("just constructed");
        };
        *slot = then_body;
        *peeled += 1;
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
        with_opt_level(OptLevel(level), || peel(&mut module));
        module.functions.remove(0).body
    }

    fn assign(name: &str, value: NirValue) -> NirOp {
        NirOp::Assign {
            name: name.to_string(),
            value,
        }
    }

    fn small_while(body: Vec<NirOp>) -> NirOp {
        NirOp::While {
            kind: crate::ast::LoopKind::While,
            condition: local("c"),
            body,
        }
    }

    /// A small while peels: guard, first-iteration copy, then the loop.
    #[test]
    fn small_while_peels_its_first_iteration() {
        let body = run(vec![small_while(vec![assign("x", local("y"))])], 3);
        let NirOp::If {
            then_body,
            else_body,
            ..
        } = &body[0]
        else {
            panic!("expected the peel guard");
        };
        assert!(else_body.is_empty());
        assert_eq!(then_body.len(), 2, "peeled copy + the loop");
        assert!(matches!(then_body[0], NirOp::Assign { .. }));
        assert!(matches!(then_body[1], NirOp::While { .. }));
    }

    /// Loop control targeting the peeled loop blocks peeling (the copy would
    /// sit outside the loop); control bound to an inner same-kind loop is
    /// shielded and fine.
    #[test]
    fn loop_control_blocks_unless_shielded() {
        let exposed = run(
            vec![small_while(vec![NirOp::ExitLoop {
                kind: crate::ast::LoopKind::While,
            }])],
            3,
        );
        assert!(matches!(&exposed[0], NirOp::While { .. }), "no peel");

        let shielded = run(
            vec![small_while(vec![small_while(vec![NirOp::ExitLoop {
                kind: crate::ast::LoopKind::While,
            }])])],
            3,
        );
        assert!(
            matches!(&shielded[0], NirOp::If { .. }),
            "the inner loop shields its own EXIT, so the outer loop peels"
        );
    }

    /// A body declaring a resource owner is never duplicated — its renamed
    /// copy would orphan the `resource_owners` close machinery (the fixed
    /// early-close bug in `resource-collection-floats-runtime` at `-O3`).
    #[test]
    fn resource_owner_bodies_are_not_peeled() {
        let function = NirFunction {
            name: "f".to_string(),
            visibility: "private".to_string(),
            kind: "function".to_string(),
            isolated: false,
            params: vec![],
            returns: ParameterType::Integer,
            body: vec![small_while(vec![NirOp::Bind {
                mutable: false,
                name: "handle".to_string(),
                type_: ParameterType::Integer,
                value: Some(local("y")),
            }])],
            file: "main.mfb".to_string(),
            resource_owners: HashMap::from([(
                "handle".to_string(),
                crate::ir::resource_escape::ResOwner::Local,
            )]),
        };
        let mut module = test_module(vec![function]);
        with_opt_level(OptLevel(3), || peel(&mut module));
        assert!(
            matches!(&module.functions[0].body[0], NirOp::While { .. }),
            "the RES-owning body must stay unduplicated"
        );
    }

    /// The row is off at `-O2` (it is a Level-3 row).
    #[test]
    fn level_two_disables_the_row() {
        let body = run(vec![small_while(vec![assign("x", local("y"))])], 2);
        assert!(matches!(&body[0], NirOp::While { .. }));
    }
}
