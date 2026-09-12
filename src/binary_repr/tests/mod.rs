use super::*;

mod builder_tests;
mod cross_package_tests;
mod doc_table_tests;
mod fixtures;
mod gap_tests;
mod mod_error_path_tests;
mod mod_inner_ir_error_tests;
mod mod_tests;
mod native_library_table_tests;
mod package_info_and_validation_tests;
mod reader_gap_tests;
mod reader_tests;
mod resource_table_tests;
mod sections_tests;
// `util_tests` moved to `wire/src/bytes.rs`'s own test module with the
// primitives it covers (plan-126-B) — a test for `mfb_wire` code that ran only
// under the compiler crate would have left the new crate's own gate empty.
mod writer_tests;
mod writer_walker_tests;
