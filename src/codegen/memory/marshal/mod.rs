//! `codegen::memory::marshal` module wiring.

pub(crate) mod byte_list;
pub(crate) mod construct_helpers;
pub(crate) use byte_list::*;
pub(crate) mod record;
pub(crate) use record::*;
pub(crate) mod write_payload;
pub(crate) use write_payload::*;
