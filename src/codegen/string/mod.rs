//! `codegen::string` module wiring.

pub(crate) mod format;
pub(crate) mod repr;
pub(crate) mod util;
pub(crate) mod validate;

pub(crate) mod unicode_props;
pub(crate) use unicode_props::*;
