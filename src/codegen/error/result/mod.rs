//! `codegen::error::result` module wiring.

pub(crate) mod error_result;
pub(crate) use error_result::*;
pub(crate) mod unboxed_trap;
pub(crate) use unboxed_trap::UnboxedPayloadSource;
