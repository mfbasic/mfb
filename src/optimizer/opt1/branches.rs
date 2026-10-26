//! Branch simplification / folding — the Opt1 (tree-level) half of the
//! Level-1 catalog row (`planning/optimizations.md`): fold a branch whose
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
//! An `IF` arm is also a cleanup scope, and splicing it moves that scope's
//! exit to the end of the enclosing body. So an arm that declares, at its own
//! level, a binding with a scope-exit effect — a `RES` binding (the function's
//! `resource_owners`) or a value whose type can carry a resource (a `List OF
//! RES`, a record or union, a closure) — is left unfolded: its close must run
//! where the arm ends, before the statements that follow it (a buffered write
//! handle flushes on close, and the code after the block can read that file).
//!
//! Deliberately *not* folded: `DO … UNTIL TRUE` (splicing the body would
//! re-target any `EXIT`/`CONTINUE` inside it at the wrong enclosing loop),
//! `WHILE TRUE` (an intended infinite/`EXIT`-terminated loop), constant
//! `MATCH` scrutinees (worth a row of their own), and any non-literal
//! condition. Runs after the local rewrites (which mint the constants) and
//! before tree-UCE/DCE, so a now-terminal spliced arm truncates what follows
//! and stranded bindings get swept.

use crate::ir::resource_escape::ResOwner;
#[cfg(test)]
use crate::operators::BinaryOp;
use crate::target::shared::nir::{NirModule, NirOp, NirValue};
use crate::types::ParameterType;
use std::collections::HashMap;

/// Apply the tree-level branch-simplification row to the whole module.
/// Self-guarded on its catalog level (1); the fold count feeds
/// `optimizer::stats`.
pub(crate) fn simplify(module: &mut NirModule) {
    if !crate::optimizer::level_enabled(1) {
        return;
    }
    let mut folded = 0;
    for function in &mut module.functions {
        simplify_body(&mut function.body, &function.resource_owners, &mut folded);
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

/// Whether a value of `type_` provably holds no resource at any depth, so its
/// binding has no observable scope-exit effect. Structural on purpose: a
/// nominal (record, union, resource), a `RES`/`STATE` marker, a thread handle,
/// a closure or an unresolved type answers `false` — this pass has no type
/// model, and a record or union can carry a resource field
/// (`codegen::resource::walk::type_carries_resources`).
fn provably_resource_free(type_: &ParameterType) -> bool {
    match type_ {
        ParameterType::AttributeString
        | ParameterType::Boolean
        | ParameterType::Byte
        | ParameterType::Integer
        | ParameterType::Fixed
        | ParameterType::Float
        | ParameterType::Money
        | ParameterType::Nothing
        | ParameterType::String
        | ParameterType::C(_) => true,
        ParameterType::ListOf(element)
        | ParameterType::SetOf(element)
        | ParameterType::ResultOf(element) => provably_resource_free(element),
        ParameterType::MapOf(key, value) | ParameterType::MapEntryOf(key, value) => {
            provably_resource_free(key) && provably_resource_free(value)
        }
        _ => false,
    }
}

/// Whether splicing `arm` into its parent would move a scope-exit cleanup:
/// it declares, at its own level, a `RES` binding or a binding whose type can
/// carry a resource. Nested bodies keep their own scopes and are not checked.
fn arm_owns_scope_cleanup(arm: &[NirOp], resource_owners: &HashMap<String, ResOwner>) -> bool {
    arm.iter().any(|op| match op {
        NirOp::Bind { name, type_, .. } => {
            resource_owners.contains_key(name.as_str()) || !provably_resource_free(type_)
        }
        _ => false,
    })
}

fn simplify_body(
    ops: &mut Vec<NirOp>,
    resource_owners: &HashMap<String, ResOwner>,
    folded: &mut u64,
) {
    // Splicing a taken arm flattens its scope into this one: its own-level
    // declarations must not collide with this body's, or the fold would
    // declare a name twice (`plans::loops::scope_bind_names`).
    let body_scope = super::plans::loops::scope_bind_names(ops);
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
                simplify_body(then_body, resource_owners, folded);
                simplify_body(else_body, resource_owners, folded);
            }
            NirOp::Match { cases, .. } => {
                for case in cases {
                    simplify_body(&mut case.body, resource_owners, folded);
                }
            }
            NirOp::While { body, .. }
            | NirOp::For { body, .. }
            | NirOp::DoUntil { body, .. }
            | NirOp::ForEach { body, .. }
            | NirOp::Trap { body, .. } => simplify_body(body, resource_owners, folded),
            _ => {}
        }
        match op {
            NirOp::If {
                condition,
                then_body,
                else_body,
            } if constant_condition(&condition).is_some_and(|taken| {
                let arm = if taken { &then_body } else { &else_body };
                super::plans::loops::scope_bind_names(arm).is_disjoint(&body_scope)
                    && !arm_owns_scope_cleanup(arm, resource_owners)
            }) =>
            {
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
mod tests;
