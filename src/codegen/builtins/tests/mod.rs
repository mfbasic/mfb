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
mod app_surface;
mod canvas;
mod collection_compare;
mod collections;
mod corpus;
mod data_layout;
mod diagnostics;
mod entry;
mod fixture_projects;
mod harness;
mod imported_types;
mod inplace;
mod inplace_fields;
mod link;
mod link_struct_widths;
mod math;
mod nir_json;
mod nir_validation;
mod optimizer;
mod optimizer_globals;
mod os;
mod package_format;
mod perf;
mod plan_validation;
mod platform_failure;
mod platform_hooks;
mod registry_bodies;
mod resource_union_default;
mod strings_fold;
mod term_read;
mod thread_send_scalar;
mod threads;
mod validation;
mod vector;
mod vector_promotion;
