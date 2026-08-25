//! Unreachable code elimination — the Opt1 (tree-level) half of the Level-2
//! catalog row (`planning/optimizations.md`): drop statements that control
//! flow can never reach. The CFG half lives in `opt2::uce`; both feed one
//! "Unreachable code elimination" `-v` count.
//!
//! Unlike DCE, there is **no trap gate at all**: unreachable code cannot raise
//! anything because it never executes, so even trap-capable statements are
//! removable once unreachability is proven. What this half proves, purely
//! structurally, is post-terminal position: every statement after an op that
//! *always* transfers control away (`RETURN`, `FAIL`, `EXIT PROGRAM`,
//! `EXIT`/`CONTINUE` loop, an `IF` whose both arms terminate, a `MATCH` with
//! an `ELSE` case whose every case terminates) is unreachable. Constant-branch
//! arms (`IF FALSE THEN …`) are the branch-simplification row's product and
//! wait for it — this half never evaluates conditions.
//!
//! One structural exception: the very last `RETURN` of a function body is kept
//! even when post-terminal, so a function body always ends in its explicit
//! return op and downstream planning sees the shape it expects.

use crate::target::shared::nir::{NirModule, NirOp};

/// Apply the tree-level UCE row to the whole module. Self-guarded on its
/// catalog level (2); the removal count feeds `optimizer::stats`.
pub(crate) fn eliminate(module: &mut NirModule) {
    if !crate::optimizer::level_enabled(2) {
        return;
    }
    let mut removed = 0;
    for function in &mut module.functions {
        truncate_unreachable(&mut function.body, true, &mut removed);
    }
    crate::optimizer::stats::count_unreachable_eliminations(removed);
}

/// Recursively truncate every body after its first always-terminal statement.
/// `keep_final_return`: at a function's top level, a trailing `RETURN` in the
/// dropped range is retained (see module docs). **`TRAP` handlers in the
/// dropped range are always retained**: a trailing handler is the *raise*
/// path's destination — `FAIL e ; TRAP(err) …` transfers INTO the trap, so
/// "post-terminal" does not mean unreachable for it (this deleted a live
/// error handler and un-swallowed `control-flow-behavior`'s error 11 at
/// `-O2` before the exception existed).
fn truncate_unreachable(ops: &mut Vec<NirOp>, keep_final_return: bool, removed: &mut u64) {
    // Children first, so an `If` whose arms only become terminal after their
    // own truncation is still recognized.
    for op in ops.iter_mut() {
        match op {
            NirOp::If {
                then_body,
                else_body,
                ..
            } => {
                truncate_unreachable(then_body, false, removed);
                truncate_unreachable(else_body, false, removed);
            }
            NirOp::Match { cases, .. } => {
                for case in cases {
                    truncate_unreachable(&mut case.body, false, removed);
                }
            }
            NirOp::While { body, .. }
            | NirOp::For { body, .. }
            | NirOp::DoUntil { body, .. }
            | NirOp::ForEach { body, .. }
            | NirOp::Trap { body, .. } => truncate_unreachable(body, false, removed),
            _ => {}
        }
    }
    let Some(first_terminal) = ops.iter().position(always_terminal) else {
        return;
    };
    if first_terminal + 1 >= ops.len() {
        return;
    }
    let suffix: Vec<NirOp> = ops.drain(first_terminal + 1..).collect();
    let last = suffix.len() - 1;
    for (index, op) in suffix.into_iter().enumerate() {
        let keep = matches!(op, NirOp::Trap { .. })
            || (keep_final_return && index == last && matches!(op, NirOp::Return { .. }));
        if keep {
            ops.push(op);
        } else {
            *removed += 1;
        }
    }
}

/// Whether the statement always transfers control away (never falls through).
fn always_terminal(op: &NirOp) -> bool {
    match op {
        NirOp::Return { .. }
        | NirOp::Fail { .. }
        | NirOp::ExitProgram { .. }
        | NirOp::ExitLoop { .. }
        | NirOp::ContinueLoop { .. } => true,
        NirOp::If {
            then_body,
            else_body,
            ..
        } => {
            // Both arms must exist and terminate; an empty arm falls through.
            then_body.last().is_some_and(always_terminal)
                && else_body.last().is_some_and(always_terminal)
        }
        NirOp::Match { cases, .. } => {
            // Conservative exhaustiveness: require an explicit ELSE case (the
            // type checker guarantees exhaustiveness, but this half does not
            // depend on that guarantee reaching NIR intact).
            let has_else = cases.iter().any(|case| {
                matches!(
                    case.pattern,
                    crate::target::shared::nir::NirMatchPattern::Else
                )
            });
            has_else
                && cases
                    .iter()
                    .all(|case| case.body.last().is_some_and(always_terminal))
        }
        // Loops may run zero (or many) iterations and TRAP resumes after its
        // body — none of them guarantees a transfer away.
        NirOp::While { .. }
        | NirOp::For { .. }
        | NirOp::DoUntil { .. }
        | NirOp::ForEach { .. }
        | NirOp::Trap { .. }
        | NirOp::Bind { .. }
        | NirOp::StoreGlobal { .. }
        | NirOp::Assign { .. }
        | NirOp::StateAssign { .. }
        | NirOp::Eval { .. } => false,
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

    fn ret(value: crate::target::shared::nir::NirValue) -> NirOp {
        NirOp::Return { value: Some(value) }
    }

    fn eval(value: crate::target::shared::nir::NirValue) -> NirOp {
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
        with_opt_level(OptLevel(level), || eliminate(&mut module));
        module.functions.remove(0).body
    }

    /// A `TRAP` handler after a `FAIL` is the raise path's destination — it
    /// must survive truncation (the fixed error-11 un-swallowing bug), while
    /// ordinary statements around it still die.
    #[test]
    fn trailing_trap_handlers_survive_truncation() {
        let body = run(
            vec![
                NirOp::Fail { error: local("e") },
                eval(local("dead")),
                NirOp::Trap {
                    name: "err".to_string(),
                    body: vec![ret(int_const("1"))],
                },
                eval(local("also_dead")),
            ],
            2,
        );
        assert_eq!(body.len(), 2, "Fail + kept Trap; the Evals die");
        assert!(matches!(body[1], NirOp::Trap { .. }));
    }

    /// Statements after a RETURN are unreachable and die — even trap-capable
    /// ones (they never execute, so there is nothing to preserve).
    #[test]
    fn post_return_statements_die_including_trapping_ones() {
        let body = run(
            vec![
                ret(local("x")),
                eval(binary("+", local("a"), local("b"))),
                eval(local("y")),
            ],
            2,
        );
        assert_eq!(body.len(), 1);
    }

    /// An IF whose both arms return is terminal, so the code after it dies —
    /// but the function's own trailing RETURN is kept.
    #[test]
    fn both_arms_terminal_truncates_but_keeps_final_return() {
        let both_return = NirOp::If {
            condition: local("c"),
            then_body: vec![ret(int_const("1"))],
            else_body: vec![ret(int_const("2"))],
        };
        let body = run(
            vec![both_return, eval(local("dead")), ret(int_const("0"))],
            2,
        );
        assert_eq!(body.len(), 2, "If + retained final Return");
        assert!(matches!(body[1], NirOp::Return { .. }));
    }

    /// An IF with an empty (or non-terminal) arm falls through: nothing after
    /// it is unreachable.
    #[test]
    fn fallthrough_arms_are_not_terminal() {
        let half = NirOp::If {
            condition: local("c"),
            then_body: vec![ret(int_const("1"))],
            else_body: vec![],
        };
        let body = run(vec![half, eval(local("live")), ret(int_const("0"))], 2);
        assert_eq!(body.len(), 3);
    }

    /// Truncation recurses into nested bodies, and the row is off at `-O1`.
    #[test]
    fn nested_bodies_truncate_and_level_one_disables() {
        let nested = || {
            vec![
                NirOp::While {
                    kind: crate::ast::LoopKind::While,
                    condition: local("c"),
                    body: vec![
                        NirOp::ExitLoop {
                            kind: crate::ast::LoopKind::While,
                        },
                        eval(local("dead")),
                    ],
                },
                ret(int_const("0")),
            ]
        };
        let body = run(nested(), 2);
        let NirOp::While {
            body: loop_body, ..
        } = &body[0]
        else {
            panic!("expected While");
        };
        assert_eq!(loop_body.len(), 1, "post-EXIT statement dies");

        let body = run(nested(), 1);
        let NirOp::While {
            body: loop_body, ..
        } = &body[0]
        else {
            panic!("expected While");
        };
        assert_eq!(loop_body.len(), 2, "-O1 must not truncate");
    }
}
