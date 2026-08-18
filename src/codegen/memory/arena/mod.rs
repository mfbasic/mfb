//! `codegen::memory::arena` module wiring.

pub(crate) mod arena;
pub(crate) use arena::*;
pub(crate) mod builder_arena_transfer;
pub(crate) mod native_arena;
pub(crate) use native_arena::*;
