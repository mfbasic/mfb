//! Branch simplification / folding — the Opt1 (tree-level) half of the
//! Level-2 catalog row (`planning/optimizations.md`): fold a branch whose
//! condition is a compile-time Boolean constant. The CFG half lives in
//! `opt2::branches`; both feed one "Branch simplification / folding" `-v`
//! count.
//!
//! Constant conditions are the constant-folding row's product (`1 < 2` has
//! already become `TRUE` by the time this pass runs), so this pass matches
//! only a literal `Const Boolean` condition:
//!
//! - `IF TRUE/FALSE` is replaced by its taken arm's statements, spliced in
//!   place. The dropped arm is *unreachable* code (the "dead vs unreachable"
//!   definition in the catalog): it never executes, so no trap gate applies
//!   to its contents, and evaluating the constant condition itself has no
//!   observable effect. The taken arm's statements keep their order and
//!   nesting, so `EXIT`/`CONTINUE` inside them still target the same
//!   enclosing loops.
//! - `WHILE FALSE` runs its body zero times: the whole loop is dropped.
//!
//! Deliberately *not* folded: `DO … UNTIL TRUE` (splicing the body would
//! re-target any `EXIT`/`CONTINUE` inside it at the wrong enclosing loop),
//! `WHILE TRUE` (an intended infinite/`EXIT`-terminated loop), constant
//! `MATCH` scrutinees (worth a row of their own), and any non-literal
//! condition. Runs after the local rewrites (which mint the constants) and
//! before tree-UCE/DCE, so a now-terminal spliced arm truncates what follows
//! and stranded bindings get swept.

use crate::target::shared::nir::{NirModule, NirOp, NirValue};
use crate::types::ParameterType;

/// Apply the tree-level branch-simplification row to the whole module.
/// Self-guarded on its catalog level (2); the fold count feeds
/// `optimizer::stats`.
pub(crate) fn simplify(module: &mut NirModule) {
    if !crate::optimizer::level_enabled(2) {
        return;
    }
    let mut folded = 0;
    for function in &mut module.functions {
        simplify_body(&mut function.body, &mut folded);
    }
    crate::optimizer::stats::count_branch_simplifications(folded);
}

/// The literal Boolean a condition holds, when it is one.
fn constant_condition(condition: &NirValue) -> Option<bool> {
    let NirValue::Const { type_, value } = condition else {
        return None;
    };
    if *type_ != ParameterType::Boolean {
        return None;
    }
    match value.as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn simplify_body(ops: &mut Vec<NirOp>, folded: &mut u64) {
    let mut result: Vec<NirOp> = Vec::with_capacity(ops.len());
    for mut op in ops.drain(..) {
        // Children first, so a nested constant branch inside a kept arm is
        // folded before the arm is spliced up a level.
        match &mut op {
            NirOp::If {
                then_body,
                else_body,
                ..
            } => {
                simplify_body(then_body, folded);
                simplify_body(else_body, folded);
            }
            NirOp::Match { cases, .. } => {
                for case in cases {
                    simplify_body(&mut case.body, folded);
                }
            }
            NirOp::While { body, .. }
            | NirOp::For { body, .. }
            | NirOp::DoUntil { body, .. }
            | NirOp::ForEach { body, .. }
            | NirOp::Trap { body, .. } => simplify_body(body, folded),
            _ => {}
        }
        match op {
            NirOp::If {
                condition,
                then_body,
                else_body,
            } if constant_condition(&condition).is_some() => {
                *folded += 1;
                let taken = if constant_condition(&condition).expect("guarded") {
                    then_body
                } else {
                    else_body
                };
                result.extend(taken);
            }
            NirOp::While { ref condition, .. } if constant_condition(condition) == Some(false) => {
                *folded += 1; // zero iterations: the loop vanishes whole
            }
            other => result.push(other),
        }
    }
    *ops = result;
}

#[cfg(test)]
mod tests {
    use super::super::local_rewrites::testutil::*;
    use super::*;
    use crate::optimizer::{with_opt_level, OptLevel};
    use crate::target::shared::nir::NirFunction;
    use std::collections::HashMap;

    fn boolean(value: bool) -> NirValue {
        typed_const(ParameterType::Boolean, if value { "true" } else { "false" })
    }

    fn eval(value: NirValue) -> NirOp {
        NirOp::Eval { value }
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
        with_opt_level(OptLevel(level), || simplify(&mut module));
        module.functions.remove(0).body
    }

    /// `IF TRUE` splices the then-arm (trap-capable statements included — they
    /// were reachable and stay); the else-arm vanishes even though *it* holds
    /// trap-capable code (it is unreachable, no trap gate applies).
    #[test]
    fn constant_true_keeps_then_and_drops_else() {
        let body = run(
            vec![NirOp::If {
                condition: boolean(true),
                then_body: vec![eval(binary("+", local("a"), local("b")))],
                else_body: vec![eval(binary("/", local("x"), int_const("0")))],
            }],
            2,
        );
        assert_eq!(body.len(), 1);
        let NirOp::Eval { value } = &body[0] else {
            panic!("expected the spliced then-arm Eval");
        };
        assert!(matches!(value, NirValue::Binary { op, .. } if op == "+"));
    }

    /// `IF FALSE` keeps the else-arm; an empty else-arm means the whole IF
    /// vanishes.
    #[test]
    fn constant_false_keeps_else() {
        let body = run(
            vec![
                NirOp::If {
                    condition: boolean(false),
                    then_body: vec![eval(local("dead"))],
                    else_body: vec![eval(local("live"))],
                },
                NirOp::If {
                    condition: boolean(false),
                    then_body: vec![eval(local("dead"))],
                    else_body: vec![],
                },
            ],
            2,
        );
        assert_eq!(body.len(), 1, "second IF vanishes entirely");
    }

    /// `WHILE FALSE` runs zero times and is dropped whole; `WHILE TRUE` and
    /// non-constant conditions are untouched.
    #[test]
    fn while_false_drops_and_while_true_stays() {
        let loop_with = |condition: NirValue| NirOp::While {
            kind: crate::ast::LoopKind::While,
            condition,
            body: vec![eval(local("x"))],
        };
        let body = run(
            vec![
                loop_with(boolean(false)),
                loop_with(boolean(true)),
                loop_with(local("c")),
            ],
            2,
        );
        assert_eq!(body.len(), 2, "only WHILE FALSE vanishes");
    }

    /// Nested constant branches fold bottom-up: an inner `IF TRUE` inside a
    /// kept arm splices before the outer arm does.
    #[test]
    fn nested_constant_branches_fold() {
        let body = run(
            vec![NirOp::If {
                condition: boolean(true),
                then_body: vec![NirOp::If {
                    condition: boolean(false),
                    then_body: vec![eval(local("dead"))],
                    else_body: vec![eval(local("kept"))],
                }],
                else_body: vec![],
            }],
            2,
        );
        assert_eq!(body.len(), 1);
        assert!(matches!(&body[0], NirOp::Eval { value: NirValue::Local(name) } if name == "kept"));
    }

    /// The row is off at `-O1`.
    #[test]
    fn level_one_disables_the_row() {
        let body = run(
            vec![NirOp::If {
                condition: boolean(true),
                then_body: vec![eval(local("x"))],
                else_body: vec![],
            }],
            1,
        );
        assert!(matches!(body[0], NirOp::If { .. }));
    }
}
