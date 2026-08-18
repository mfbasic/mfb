//! `codegen::compiler::opt` module wiring.

pub(crate) mod fma_fusion;
pub(crate) use fma_fusion::*;
pub(crate) mod peephole;
pub(crate) use peephole::*;
pub(crate) mod selfmove_probe;
pub(crate) use selfmove_probe::*;
