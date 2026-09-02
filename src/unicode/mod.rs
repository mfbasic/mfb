//! Unicode support: the runtime-table generators (`runtime_tables`) and the
//! case/normalization/grapheme backend (`backend`). bug-343 A4: consolidated
//! from the loose top-level `unicode_backend.rs` / `unicode_runtime_tables.rs`.
pub(crate) mod backend;
/// plan-118-B: the pinned general-category / Script run tables, read as data
/// instead of compiled as 5,807 arms of generated MFBASIC.
pub(crate) mod range_tables;
pub(crate) mod runtime_tables;
