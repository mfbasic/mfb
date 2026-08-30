//! Recovery-region simplification — a Level-3 Opt1 catalog row
//! (`planning/optimizations.md`): drop a `TRAP` handler that can never receive
//! an error.
//!
//! MFB's error model puts at most one function-level `TRAP` at the bottom of a
//! `FUNC`/`SUB`, and it is "reachable only via `FAIL` (in the body), an
//! auto-propagated failure from a call, or `FAIL`/`PROPAGATE` inside the trap"
//! (`mfb spec language error-model` §8.6 rule 3) — never by falling through.
//! So when the guarded region provably contains none of those, the handler is
//! dead code: it is emitted, it takes space, and nothing can ever branch to it.
//!
//! Proving "cannot raise" is the whole row, and it is deliberately blunt. The
//! region qualifies only if every statement in it, at every depth, evaluates
//! nothing outside [`pure_non_trapping`] — leaves, comparisons, and the
//! Boolean connectives. That excludes, correctly:
//!
//! - every call, because in MFB *every* call is fallible (§8.1);
//! - all arithmetic, which is checked and raises `ErrOverflow`;
//! - `&`, which allocates and can raise `ErrOutOfMemory`;
//! - `FAIL` itself;
//! - a `RES` binding, whose lexical close is a cleanup that can record a
//!   failure of its own (the same test `module_may_record_cleanup_failure`
//!   uses, so the two cannot disagree about which types those are);
//! - a nested `TRAP`, whose handler may `PROPAGATE` or `FAIL` outward.
//!
//! What is left is a body of pure value flow and control flow — rare, but
//! exactly the shape a defensive `TRAP` on an infallible helper produces, and
//! the same shape the front end already flags for the *inline* form as
//! `TYPE_INLINE_TRAP_DEAD_HANDLER`. That warning does not remove anything; this
//! row does.
//!
//! **What this row is not.** The catalog entry also names coalescing nested
//! recovery regions and re-routing to the nearest live handler. Neither exists
//! to do in MFB: at most one function-level `TRAP` is legal per function
//! (§8.6 rule 1), so regions never nest, and an inline `TRAP` is desugared
//! away entirely before NIR — into a `bind $trap_res`, a `resultIsOk` test and
//! an `IF` (verified by dumping the IR of an inline-trap function) — so there
//! is no handler to route to and no region to coalesce.

use crate::target::shared::nir::{NirModule, NirOp};

use super::dce::pure_non_trapping;
use super::plans::shape::{match_guards, nested_bodies, own_values};

/// Apply the row to the whole module. Self-guarded on its catalog level (3).
pub(crate) fn simplify(module: &mut NirModule) {
    if !crate::optimizer::level_enabled(3) {
        return;
    }
    let mut removed = 0u64;
    for function in &mut module.functions {
        if !matches!(function.body.last(), Some(NirOp::Trap { .. })) {
            continue;
        }
        let guarded = &function.body[..function.body.len() - 1];
        if !region_cannot_raise(guarded) {
            continue;
        }
        function.body.pop();
        removed += 1;
    }
    crate::optimizer::stats::count_recovery_regions_simplified(removed);
}

/// Whether nothing in `ops` can enter the error path.
fn region_cannot_raise(ops: &[NirOp]) -> bool {
    ops.iter().all(op_cannot_raise)
}

fn op_cannot_raise(op: &NirOp) -> bool {
    match op {
        // The explicit entry to the error path.
        NirOp::Fail { .. } => false,
        // A nested handler can `PROPAGATE` or `FAIL` outward.
        NirOp::Trap { .. } => false,
        // A binding whose type closes on scope exit has a cleanup that can
        // record a failure.
        NirOp::Bind { type_, .. }
            if crate::codegen::builtins::resource_close_function(type_).is_some() =>
        {
            false
        }
        _ => {
            own_values(op).into_iter().all(pure_non_trapping)
                && match_guards(op).into_iter().all(pure_non_trapping)
                && nested_bodies(op)
                    .into_iter()
                    .all(|body| region_cannot_raise(body))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::local_rewrites::testutil::*;
    use super::*;
    use crate::optimizer::{with_opt_level, OptLevel};
    use crate::target::shared::nir::{NirFunction, NirSourceLoc, NirValue};
    use crate::types::ParameterType;

    fn function(body: Vec<NirOp>) -> NirFunction {
        NirFunction {
            name: "f".to_string(),
            visibility: "private".to_string(),
            kind: "function".to_string(),
            isolated: false,
            params: vec![],
            returns: ParameterType::Integer,
            body,
            file: "main.mfb".to_string(),
            resource_owners: std::collections::HashMap::new(),
        }
    }

    fn trap() -> NirOp {
        NirOp::Trap {
            name: "e".to_string(),
            body: vec![NirOp::Return {
                value: Some(int_const("0")),
            }],
        }
    }

    fn run(module: &mut NirModule, level: u8) {
        with_opt_level(OptLevel(level), || simplify(module));
    }

    /// A body of pure value flow can never enter the error path, so its
    /// handler is unreachable and goes.
    #[test]
    fn a_handler_that_cannot_be_reached_is_removed() {
        let mut module = test_module(vec![function(vec![
            NirOp::Bind {
                mutable: false,
                name: "a".to_string(),
                type_: ParameterType::Integer,
                value: Some(int_const("1")),
            },
            NirOp::Return {
                value: Some(local("a")),
            },
            trap(),
        ])]);
        run(&mut module, 3);
        assert_eq!(module.functions[0].body.len(), 2, "the handler is gone");
    }

    /// Every call in MFB is fallible, so a body containing one keeps its
    /// handler.
    #[test]
    fn a_body_with_a_call_keeps_its_handler() {
        let mut module = test_module(vec![function(vec![
            NirOp::Return {
                value: Some(NirValue::Call {
                    target: "helper".to_string(),
                    args: vec![],
                    loc: NirSourceLoc::default(),
                }),
            },
            trap(),
        ])]);
        run(&mut module, 3);
        assert_eq!(module.functions[0].body.len(), 2, "the handler stays");
    }

    /// Arithmetic is checked and raises `ErrOverflow`, so it keeps the
    /// handler too — this is the case that makes the row's predicate stricter
    /// than "no calls".
    #[test]
    fn arithmetic_keeps_the_handler() {
        let mut module = test_module(vec![function(vec![
            NirOp::Return {
                value: Some(binary("+", int_const("1"), int_const("2"))),
            },
            trap(),
        ])]);
        run(&mut module, 3);
        assert_eq!(module.functions[0].body.len(), 2);
    }

    /// `FAIL` is the explicit entry to the error path.
    #[test]
    fn an_explicit_fail_keeps_the_handler() {
        let mut module = test_module(vec![function(vec![
            NirOp::Fail {
                error: local("boom"),
            },
            trap(),
        ])]);
        run(&mut module, 3);
        assert_eq!(module.functions[0].body.len(), 2);
    }

    /// A raisable statement nested inside a loop counts: the walk is
    /// recursive, not top-level only.
    #[test]
    fn a_nested_raise_keeps_the_handler() {
        let mut module = test_module(vec![function(vec![
            NirOp::While {
                kind: crate::ast::LoopKind::While,
                condition: int_const("1"),
                body: vec![NirOp::Eval {
                    value: NirValue::Call {
                        target: "helper".to_string(),
                        args: vec![],
                        loc: NirSourceLoc::default(),
                    },
                }],
            },
            NirOp::Return {
                value: Some(int_const("0")),
            },
            trap(),
        ])]);
        run(&mut module, 3);
        assert_eq!(module.functions[0].body.len(), 3);
    }

    /// The row is off below `-O3`.
    #[test]
    fn level_two_disables_the_row() {
        let mut module = test_module(vec![function(vec![
            NirOp::Return {
                value: Some(int_const("0")),
            },
            trap(),
        ])]);
        run(&mut module, 2);
        assert_eq!(module.functions[0].body.len(), 2);
    }
}
