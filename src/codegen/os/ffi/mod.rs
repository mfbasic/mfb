//! `codegen::os::ffi` module wiring.

pub(crate) mod com_call;
pub(crate) mod external_call;
pub(crate) use com_call::*;
pub(crate) use external_call::*;
