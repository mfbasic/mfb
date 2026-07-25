//! Unicode support: the runtime-table generators (`runtime_tables`) and the
//! case/normalization/grapheme backend (`backend`). bug-343 A4: consolidated
//! from the loose top-level `unicode_backend.rs` / `unicode_runtime_tables.rs`.
pub(crate) mod backend;
pub(crate) mod runtime_tables;
