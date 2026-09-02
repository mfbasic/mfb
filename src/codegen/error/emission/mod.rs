//! `codegen::error::emission` module wiring.

pub(crate) mod builder_error_emission;
pub(crate) mod native_fail;
pub(crate) mod park_error_helper;
pub(crate) use native_fail::*;
