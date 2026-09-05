//! Target-generic code generation layer (plan-95).
//!
//! Extracted out of `src/target` because it is target-*generic*, not target-
//! specific: the lowering here emits abstract instructions through the `abi::`
//! seam, which each backend resolves per arch/os. Holds the migrated builtin
//! packages (`builtins`) and, as functions migrate, their target-generic lowering.
//! The builtin registry itself lives in `codegen::registry`.

pub(crate) mod app;
pub(crate) mod builtins;
// Front-end test-desugar metadata (assertion builtins), relocated from
// `src/builtins/testing.rs` (plan-103). Kept separate from the `testing` *package*
// lowering at `codegen::builtins::testing`.
pub(crate) mod builtins_testing;
pub(crate) mod cleanup;
pub(crate) mod collection;
pub(crate) mod compiler;
pub(crate) mod engine;
pub(crate) mod error;
pub(crate) mod io;
pub(crate) mod link;
pub(crate) mod memory;
pub(crate) mod os;
pub(crate) mod resource;
pub(crate) mod runtime;
pub(crate) mod string;
pub(crate) mod term;
// The clean-room builtin registry (planning/todo.md): every builtin package now
// registers itself here and all builtin dispatch flows through it. A few
// descriptor accessors are exercised only by the registry's own `#[cfg(test)]`
// suite (kept for symmetry/future consumers), hence the `not(test)` dead-code
// allow — scoped, not a blanket production suppression.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) mod registry;

/// plan-116-J: for each local bound in `ops`, the locals it structurally **holds**.
///
/// A whole-function pre-pass, in the shape `promotable_vector_locals` established, read
/// by `deactivate_consumed_cleanups`. `LET tag = canvas::Text[…, font := face, …]` gives
/// `tag -> ["face"]`, so that handing `[tag]` to a consuming parameter can find the close
/// obligation that has to be dropped — `face`, whose name appears nowhere in the
/// argument.
///
/// Not filtered to resources here. The filter needs each local's *type*, which the
/// builder has and this pass does not, and `deactivate_consumed_cleanups` applies it when
/// it looks each name up. Carrying non-resources costs a map entry and cannot cause a
/// wrong deactivation.
///
/// **The twin of `ir::verify::link`'s `held_resources`, and structurally identical to
/// it.** They are two lists because `IrValue` and `NirValue` are distinct types; an arm
/// added to one and not the other compiles. If you add a value form that PLACES its
/// operands into the value it builds, add it to both.
pub(crate) fn resource_containment(
    ops: &[crate::target::shared::nir::NirOp],
) -> std::collections::HashMap<String, Vec<String>> {
    use crate::target::shared::nir::NirOp;
    fn walk(
        ops: &[NirOp],
        out: &mut std::collections::HashMap<String, Vec<String>>,
    ) {
        for op in ops {
            match op {
                NirOp::Bind {
                    name,
                    value: Some(value),
                    ..
                }
                | NirOp::Assign { name, value } => {
                    let mut held = Vec::new();
                    crate::codegen::engine::builder::CodeBuilder::collect_consumed_locals(
                        value, &mut held,
                    );
                    held.retain(|held_name| held_name != name);
                    if !held.is_empty() {
                        out.insert(name.clone(), held);
                    }
                }
                NirOp::If {
                    then_body,
                    else_body,
                    ..
                } => {
                    walk(then_body, out);
                    walk(else_body, out);
                }
                NirOp::Match { cases, .. } => {
                    for case in cases {
                        walk(&case.body, out);
                    }
                }
                NirOp::While { body, .. }
                | NirOp::For { body, .. }
                | NirOp::DoUntil { body, .. }
                | NirOp::ForEach { body, .. }
                | NirOp::Trap { body, .. } => walk(body, out),
                _ => {}
            }
        }
    }
    let mut out = std::collections::HashMap::new();
    walk(ops, &mut out);
    out
}
