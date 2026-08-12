//! Target-generic code generation layer (plan-95).
//!
//! Extracted out of `src/target` because it is target-*generic*, not target-
//! specific: the lowering here emits abstract instructions through the `abi::`
//! seam, which each backend resolves per arch/os. Holds the migrated builtin
//! packages (`builtins`) and, as functions migrate, their target-generic lowering.
//! The builtin descriptor registry itself lives in `target::shared::registry`; the
//! per-module migration into this layer is being re-done incrementally (see
//! `planning/todo.md` — the "one `implementations` array per builtin" north star).

pub(crate) mod builtins;
pub(crate) mod memory;
pub(crate) mod os;
// Clean-room north-star registry (planning/todo.md), built in parallel and not yet
// wired into the pipeline — packages migrate onto it one at a time. It is fully
// exercised by its own `#[cfg(test)]` suite (the lint stays live there as a
// tripwire); it is "dead" only in the shipping binary, precisely because no package
// has migrated onto it yet. The allow is scoped to `not(test)` and lifts, item by
// item, as real consumers appear — it is not a blanket suppression.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) mod registry;
