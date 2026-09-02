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
mod tests {
    use super::super::local_rewrites::testutil::*;
    use super::*;
    use crate::optimizer::{with_opt_level, OptLevel};
    use std::collections::HashMap;

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
            resource_owners: HashMap::new(),
        }
    }

    fn bind(name: &str, type_: ParameterType, value: Option<NirValue>) -> NirOp {
        NirOp::Bind {
            mutable: false,
            name: name.to_string(),
            type_,
            value,
        }
    }

    fn run(body: Vec<NirOp>, level: u8) -> (Vec<NirOp>, u64) {
        let before = count_ops(&body);
        let mut module = test_module(vec![function(body)]);
        with_opt_level(OptLevel(level), || eliminate(&mut module));
        let body = module.functions.remove(0).body;
        let removed = before - count_ops(&body);
        (body, removed)
    }

    /// Recursive op count, so `run` can report how many ops a sweep dropped
    /// without reading the process-global stats.
    fn count_ops(ops: &[NirOp]) -> u64 {
        ops.iter()
            .map(|op| {
                1 + match op {
                    NirOp::If {
                        then_body,
                        else_body,
                        ..
                    } => count_ops(then_body) + count_ops(else_body),
                    NirOp::Match { cases, .. } => {
                        cases.iter().map(|case| count_ops(&case.body)).sum()
                    }
                    NirOp::While { body, .. }
                    | NirOp::For { body, .. }
                    | NirOp::DoUntil { body, .. }
                    | NirOp::ForEach { body, .. }
                    | NirOp::Trap { body, .. } => count_ops(body),
                    _ => 0,
                }
            })
            .sum()
    }

    /// An unused scalar bind chain dies transitively: removing `b` (only user
    /// of `a`) exposes `a` as dead on the next fixpoint sweep.
    #[test]
    fn unused_bind_chains_die_to_a_fixpoint() {
        let (body, removed) = run(
            vec![
                bind("a", ParameterType::Integer, Some(int_const("1"))),
                bind("b", ParameterType::Integer, Some(local("a"))),
                bind("keep", ParameterType::Integer, Some(int_const("2"))),
                NirOp::Return {
                    value: Some(local("keep")),
                },
            ],
            2,
        );
        assert_eq!(removed, 2);
        assert_eq!(body.len(), 2, "only `keep` and the return survive");
    }

    /// The trap gate: an unused bind whose initializer is checked arithmetic
    /// (can raise ErrOverflow) or a Float computation (the bind is the
    /// observation boundary) must stay; so must non-scalar types and anything
    /// read, assigned, or resource-owning.
    #[test]
    fn trapping_effectful_or_used_binds_stay() {
        let trapping = bind(
            "t",
            ParameterType::Integer,
            Some(binary(BinaryOp::Add, local("keep"), local("keep"))),
        );
        let float_boundary = bind(
            "fb",
            ParameterType::Float,
            Some(binary(BinaryOp::Divide, local("g"), local("h"))),
        );
        let non_scalar = bind("s", ParameterType::String, Some(local("other")));
        let assigned = bind("m", ParameterType::Integer, Some(int_const("0")));
        let (body, removed) = run(
            vec![
                trapping,
                float_boundary,
                non_scalar,
                assigned,
                NirOp::Assign {
                    name: "m".to_string(),
                    value: int_const("5"),
                },
                bind("keep", ParameterType::Integer, Some(int_const("2"))),
                NirOp::Return {
                    value: Some(local("keep")),
                },
            ],
            2,
        );
        assert_eq!(removed, 0);
        assert_eq!(body.len(), 7);
    }

    /// Pure comparisons/logic are removable; nested bodies are swept; a pure
    /// `Eval` dies.
    #[test]
    fn pure_comparisons_and_evals_die_in_nested_bodies() {
        let (body, removed) = run(
            vec![
                bind("keep", ParameterType::Integer, Some(int_const("2"))),
                NirOp::If {
                    condition: local("keep"),
                    then_body: vec![
                        bind(
                            "cmp",
                            ParameterType::Boolean,
                            Some(binary(BinaryOp::Less, local("keep"), int_const("3"))),
                        ),
                        NirOp::Eval {
                            value: unary(UnaryOp::Not, local("keep")),
                        },
                        NirOp::Return {
                            value: Some(local("keep")),
                        },
                    ],
                    else_body: vec![],
                },
                NirOp::Return {
                    value: Some(local("keep")),
                },
            ],
            2,
        );
        assert_eq!(removed, 2);
        let NirOp::If { then_body, .. } = &body[1] else {
            panic!("expected If");
        };
        assert_eq!(then_body.len(), 1, "only the return survives in the arm");
    }

    /// A shadowing rebind shares its name with the outer binding, so the
    /// scope-blind census keeps both — conservative by design.
    #[test]
    fn shadowed_names_are_never_removed() {
        let (body, removed) = run(
            vec![
                bind("x", ParameterType::Integer, Some(int_const("1"))),
                NirOp::If {
                    condition: local("c"),
                    then_body: vec![bind("x", ParameterType::Integer, Some(int_const("2")))],
                    else_body: vec![],
                },
            ],
            2,
        );
        assert_eq!(removed, 0);
        assert_eq!(body.len(), 2);
    }

    /// Level gating: the row is off at `-O1`.
    #[test]
    fn level_one_disables_the_row() {
        let (body, removed) = run(
            vec![bind("dead", ParameterType::Integer, Some(int_const("1")))],
            1,
        );
        assert_eq!(removed, 0);
        assert_eq!(body.len(), 1);
    }
}
