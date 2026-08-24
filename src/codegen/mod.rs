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
