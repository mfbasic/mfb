//! Target-generic code generation layer (plan-95).
//!
//! Extracted out of `src/target` because it is target-*generic*, not target-
//! specific: the lowering here emits abstract instructions through the `abi::`
//! seam, which each backend resolves per arch/os. Owns the builtin registry
//! (`registry`) and, as functions migrate, their target-generic lowering.

pub(crate) mod builtins;
pub(crate) mod memory;
pub(crate) mod registry;
