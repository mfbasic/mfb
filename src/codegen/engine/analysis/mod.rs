//! `codegen::engine::analysis` module wiring.

pub(crate) mod last_use;
pub(crate) mod module_analysis;
pub(crate) use module_analysis::*;
