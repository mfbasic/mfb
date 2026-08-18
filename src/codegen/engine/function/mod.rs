//! `codegen::engine::function` module wiring.

pub(crate) mod entry;
pub(crate) use entry::*;
pub(crate) mod function_lowering;
pub(crate) use function_lowering::*;
