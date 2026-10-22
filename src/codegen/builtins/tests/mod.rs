//! Per-package codegen contract suites.
//!
//! **Why they live in a directory called `tests` rather than beside the package
//! they test.** `scripts/coverage-common.sh`'s `IGNORE` excludes `/tests/`, and
//! two separate things go wrong when a suite sits inside the denominator:
//!
//! 1. *The suite pays for its own diagnostics.* A codegen assertion is mostly
//!    "find this instruction or explain precisely what is missing", and every
//!    such arm is dead on a green run. Ten of them put the canvas suite 1.7
//!    points under the floor it exists to raise.
//! 2. *Declaring the module perturbs the package's own `mod.rs`.* A
//!    `cargo llvm-cov --bins` profile merges TWO copies of the crate — the test
//!    binary, which runs, and the plain `mfb` binary, which is instrumented and
//!    never executed — and the two inline differently once a `#[cfg(test)]`
//!    module appears. Measured: adding `mod tests_codegen;` to
//!    `builtins/os/mod.rs` moved that file from `100% (131/131)` to
//!    `85.06% (131/154)`, because `register`'s nineteen
//!    `func_*::register(&mut pkg)` lines stopped being inlined away in one copy
//!    and started being reported as uncovered from the other. The function
//!    demonstrably runs — `register_publishes_the_whole_os_surface` calls it and
//!    every member is registered — and `#[inline(never)]` did not change the
//!    number. Parking the suite restored 131/131.
//!
//! So the declaration itself lives in `codegen/engine/tests/mod.rs`, whose own
//! path contains `/tests/` and is therefore excluded too — the `#[cfg(test)] mod`
//! line has to sit somewhere, and there it damages nothing. Putting it in
//! `builtins/mod.rs` instead was measured at 99.51% (610/613) -> 68.93%
//! (610/885) for that file. The suites reach their emitters through ordinary
//! `pub(crate)` paths.

mod abi_inline;
mod abi_inline_element_types;
mod abi_inline_platforms;
mod app_mode_io;
mod app_surface;
mod arena_transfer_shapes;
mod bits_shift_forms;
mod build_replay;
mod builder_emit_forms;
mod builder_refusals;
mod builtin_record_arguments;
mod byte_list_entry_free;
mod canvas;
mod canvas_argument_refusals;
mod canvas_geo_layout_guards;
mod canvas_item_hash_guards;
mod closure_env_forms;
mod collection_compare;
mod collection_element_widths;
mod collection_misuse;
mod collection_native_refusals;
mod collections;
mod content_equal;
mod control_forms;
mod corpus;
mod corpus_rest;
mod data_layout;
mod debug_report;
mod declared_errors_all;
mod declared_errors_sweep;
mod default_union;
mod default_value_shapes;
mod diagnostics;
mod element_update;
mod element_update_forms;
mod entry;
mod exit_cleanup_forms;
mod fast_path_answers;
mod field_access_shapes;
mod field_replace_forms;
mod field_with_forms;
mod fixture_projects;
mod fixture_sweep;
mod fs_declared_errors;
mod general_declared_errors;
mod handover_forms;
mod handover_variant_forms;
mod harness;
mod helper_import_refusals;
mod imported_types;
mod infallible_proof_coverage;
mod inline_trap_type_name;
mod inplace;
mod inplace_compact;
mod inplace_dest_refusals;
mod inplace_fields;
mod inplace_inline_repack;
mod inplace_rewrite;
mod inplace_rewrite_retype;
mod link;
mod link_float_results;
mod link_out_widths;
mod link_struct_widths;
mod link_thunk_refusals;
mod link_value_widths;
mod literal_coercion;
mod lowered;
mod lowering_programs;
mod lowering_shapes;
mod lowering_shapes_fields;
mod lowering_targets;
mod marshal_record;
mod math;
mod math_dispatch;
mod mfb_body_declared_errors;
mod mir_dump;
mod mixed_with_shapes;
mod module_layout_forms;
mod money_operators;
mod multi_file_projects;
mod nir_build;
mod nir_dump;
mod nir_fault;
mod nir_fault_callees;
mod nir_fault_inplace;
mod nir_fault_loops;
mod nir_fault_operators;
mod nir_fault_records;
mod nir_json;
mod nir_retype;
mod nir_validation;
mod numeric_forms;
mod opt_aggcopy;
mod opt_bits_lattice;
mod opt_check_elision;
mod opt_dead_stores;
mod opt_fission;
mod opt_fuse;
mod opt_indvars;
mod opt_knownbits;
mod opt_lencache;
mod opt_local_rewrites;
mod opt_loop_facts;
mod opt_pre;
mod opt_promote_frame;
mod opt_range_facts;
mod opt_recovery;
mod opt_sink_hoist;
mod opt_slot_availability;
mod optimizer;
mod optimizer_globals;
mod os;
mod owned_list_refusals;
mod package_format;
mod parse_refusals;
mod payload_match_refusals;
mod perf;
mod plan_validation;
mod platform_failure;
mod platform_hooks;
mod platforms;
mod predicate_intrinsics;
mod process_overloads;
mod produced_union_close;
mod reduce_accumulators;
mod registry_bodies;
mod resource_union_default;
mod result_field_ownership;
mod return_passthrough_forms;
mod shape_refusals;
mod sort_declines;
mod static_name_prepass;
mod static_type_oracles;
mod stems;
mod strings_fold;
mod tcp_read_helper;
mod term_measure_text;
mod term_read;
mod thread_primitives;
mod thread_send_scalar;
mod threads;
mod transform_inline;
mod trap_recover_forms;
mod typename_enum_fold;
mod validation;
mod vector;
mod vector_promotion;
