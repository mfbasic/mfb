//! Loop rotation — a Level-3 catalog row (`planning/optimizations.md`), the
//! structured-loop form: convert a head-tested `WHILE` into the guarded
//! bottom-tested shape, saving one branch per iteration in the lowered code
//! (the while lowering re-tests at the head and jumps back; the do-until
//! lowering falls through its test).
//!
//! ```text
//! WHILE c: body      IF c THEN DO body UNTIL NOT c END IF
//! ```
//!
//! Evaluation parity is exact — the condition still evaluates n+1 times
//! (guard once, bottom test n times) and the body n times, in the same
//! order — so **no purity or trap-freedom is required**; a trapping
//! condition traps at the identical evaluation, and the `DO..UNTIL`
//! condition evaluates in the outer scope exactly like a `WHILE` condition
//! (`ir/lower`'s scoping). Two structural guards: the body must not carry
//! `EXIT`/`CONTINUE` bound to this loop's kind (they would no longer find
//! it — the rotated loop pushes `Do` on the loop stack), and must not carry
//! unshielded `Do`-kind loop control (the new `DO..UNTIL` would *capture*
//! an op meant for an enclosing `DO` loop). Runs last among the loop rows,
//! so the other `WHILE`-shaped rewrites see the original form.

use crate::ast::LoopKind;
use crate::target::shared::nir::{NirModule, NirOp, NirValue};

use super::plans::loops::captures_loop_control;

/// Apply the rotation row to the whole module. Self-guarded on its catalog
/// level (3); the rotation count feeds `optimizer::stats`.
pub(crate) fn rotate(module: &mut NirModule) {
    if !crate::optimizer::level_enabled(3) {
        return;
    }
    let mut rotated = 0;
    for function in &mut module.functions {
        rotate_in_body(&mut function.body, &mut rotated);
    }
    crate::optimizer::stats::count_loops_rotated(rotated);
}

fn rotate_in_body(ops: &mut Vec<NirOp>, rotated: &mut u64) {
    for op in ops.iter_mut() {
        match op {
            NirOp::If {
                then_body,
                else_body,
                ..
            } => {
                rotate_in_body(then_body, rotated);
                rotate_in_body(else_body, rotated);
            }
            NirOp::Match { cases, .. } => {
                for case in cases {
                    rotate_in_body(&mut case.body, rotated);
                }
            }
            NirOp::While { body, .. }
            | NirOp::For { body, .. }
            | NirOp::DoUntil { body, .. }
            | NirOp::ForEach { body, .. }
            | NirOp::Trap { body, .. } => rotate_in_body(body, rotated),
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
        if captures_loop_control(body, *kind) || captures_loop_control(body, LoopKind::Do) {
            continue;
        }
        let guard = condition.clone();
        let negated = NirValue::Unary {
            op: "NOT".to_string(),
            operand: Box::new(condition.clone()),
            loc: Default::default(),
        };
        let rotated_loop = NirOp::DoUntil {
            body: std::mem::take(body),
            condition: negated,
        };
        *op = NirOp::If {
            condition: guard,
            then_body: vec![rotated_loop],
            else_body: Vec::new(),
        };
        *rotated += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::super::local_rewrites::testutil::*;
    use super::*;
    use crate::optimizer::{with_opt_level, OptLevel};
    use crate::target::shared::nir::NirFunction;
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
        with_opt_level(OptLevel(level), || rotate(&mut module));
        module.functions.remove(0).body
    }

    fn assign(name: &str) -> NirOp {
        NirOp::Assign {
            name: name.to_string(),
            value: local("y"),
        }
    }

    fn while_loop(body: Vec<NirOp>) -> NirOp {
        NirOp::While {
            kind: crate::ast::LoopKind::While,
            condition: local("c"),
            body,
        }
    }

    /// A plain while rotates into the guarded bottom-tested form with the
    /// condition negated at the bottom.
    #[test]
    fn while_rotates_to_guarded_do_until() {
        let body = run(vec![while_loop(vec![assign("x")])], 3);
        let NirOp::If {
            condition,
            then_body,
            else_body,
        } = &body[0]
        else {
            panic!("expected the rotation guard");
        };
        assert!(matches!(condition, NirValue::Local(name) if name == "c"));
        assert!(else_body.is_empty());
        let NirOp::DoUntil {
            condition: until, ..
        } = &then_body[0]
        else {
            panic!("expected the rotated DO..UNTIL");
        };
        assert!(matches!(until, NirValue::Unary { op, .. } if op == "NOT"));
    }

    /// Loop control bound to the while (its EXIT would dangle) or to an
    /// enclosing DO (the new DO..UNTIL would capture it) blocks rotation.
    #[test]
    fn loop_control_hazards_block_rotation() {
        let own = run(
            vec![while_loop(vec![NirOp::ExitLoop {
                kind: crate::ast::LoopKind::While,
            }])],
            3,
        );
        assert!(matches!(&own[0], NirOp::While { .. }));

        let capture = run(
            vec![while_loop(vec![NirOp::ContinueLoop {
                kind: crate::ast::LoopKind::Do,
            }])],
            3,
        );
        assert!(matches!(&capture[0], NirOp::While { .. }));
    }

    /// The row is off at `-O2` (it is a Level-3 row).
    #[test]
    fn level_two_disables_the_row() {
        let body = run(vec![while_loop(vec![assign("x")])], 2);
        assert!(matches!(&body[0], NirOp::While { .. }));
    }
}
