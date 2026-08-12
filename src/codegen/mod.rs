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
